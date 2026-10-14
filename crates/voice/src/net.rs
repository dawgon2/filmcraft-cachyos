//! The DeepFilterNet3 network, one 10 ms frame at a time. Plain-Rust f32 kernels on preallocated
//! buffers (no allocation per frame), with each layer keeping the few past frames it convolves
//! over, so the same code serves live playback and offline rendering. The weights come from the
//! checked checkpoint ([`crate::weights::Weights`]); the layer structure follows the published
//! model (`df/deepfilternet3.py`, `df/modules.py`, MIT / Apache-2.0):
//!
//! - encoder: ERB features `[1 × 32]` → convs → `e0 [64 × 32]`, `e1 [64 × 16]`, `e2`, `e3 [64 × 8]`;
//!   complex features `[2 × 96]` → `c0 [64 × 96]`, `c1 [64 × 48]` → grouped linear; summed into a
//!   512-wide embedding → squeezed GRU (1 layer) → `emb`, local SNR.
//! - ERB decoder: squeezed GRU (2 layers) → `[64 × 8]` → transposed convs with encoder skips →
//!   32 band gains (sigmoid).
//! - DF decoder: squeezed GRU (2 layers) + grouped-linear skip → 96 bins × 5 complex taps (tanh),
//!   plus a 5-frame convolution over `c0`.
//!
//! Convolutions are causal in time (the features are already shifted by the model's lookahead,
//! see [`crate::denoise`]); "same" padding in frequency.

use std::sync::Arc;

use crate::ModelError;
use crate::dsp::C32;
use crate::weights::Weights;

fn err(e: impl std::fmt::Display) -> ModelError {
    ModelError(e.to_string())
}

const CH: usize = 64;
const NB_ERB: usize = 32;
const NB_DF: usize = 96;
const DF_ORDER: usize = 5;
const EMB: usize = 512;
const HIDDEN: usize = 256;
const LSNR_MIN: f32 = -15.0;
const LSNR_MAX: f32 = 35.0;

/// `Σ a·b` with eight partial sums, so the compiler can vectorise it.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (a8, ar) = a.as_chunks::<8>();
    let (b8, br) = b.as_chunks::<8>();
    for (x, y) in a8.iter().zip(b8) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let tail: f32 = ar.iter().zip(br).map(|(x, y)| x * y).sum();
    acc.iter().sum::<f32>() + tail
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[derive(Clone, Copy)]
enum Act {
    None,
    Relu,
    Sigmoid,
}

impl Act {
    fn apply(self, x: f32) -> f32 {
        match self {
            Act::None => x,
            Act::Relu => x.max(0.0),
            Act::Sigmoid => sigmoid(x),
        }
    }
}

fn vec_of(w: &Weights, name: &str, len: usize) -> Result<Arc<[f32]>, ModelError> {
    let v = w.values(name)?;
    if v.len() != len {
        return Err(err(format!("`{name}` has {} values, expected {len}", v.len())));
    }
    Ok(v)
}

/// A grouped 2-D convolution over (time, frequency), causal in time, stride only in frequency.
struct Conv {
    in_ch: usize,
    out_ch: usize,
    kt: usize,
    kf: usize,
    fstride: usize,
    groups: usize,
    f_in: usize,
    f_out: usize,
    /// `[out][in / groups][kt][kf]`.
    w: Arc<[f32]>,
    /// The `kt − 1` previous input frames, oldest first, each `[in][f_in]`.
    hist: Vec<f32>,
}

impl Conv {
    fn new(w: &Weights, name: &str, in_ch: usize, out_ch: usize, kt: usize, kf: usize, fstride: usize, groups: usize, f_in: usize) -> Result<Conv, ModelError> {
        let ig = in_ch / groups;
        let w = vec_of(w, name, out_ch * ig * kt * kf)?;
        let pad = kf / 2;
        let f_out = (f_in + 2 * pad - kf) / fstride + 1;
        Ok(Conv { in_ch, out_ch, kt, kf, fstride, groups, f_in, f_out, w, hist: vec![0.0; (kt - 1) * in_ch * f_in] })
    }

    fn reset(&mut self) {
        self.hist.fill(0.0);
    }

    /// `x`: `[in][f_in]` (this frame) → `y`: `[out][f_out]`.
    fn run(&mut self, x: &[f32], y: &mut [f32]) {
        let (ig, og) = (self.in_ch / self.groups, self.out_ch / self.groups);
        let (kt, kf, f_in, f_out) = (self.kt, self.kf, self.f_in, self.f_out);
        let frame_len = self.in_ch * f_in;
        let pad = kf / 2;
        for o in 0..self.out_ch {
            let g = o / og;
            for fo in 0..f_out {
                let mut acc = 0.0;
                for i in 0..ig {
                    let ci = g * ig + i;
                    for t in 0..kt {
                        let frame = if t + 1 == kt { x } else { &self.hist[t * frame_len..(t + 1) * frame_len] };
                        let row = &frame[ci * f_in..(ci + 1) * f_in];
                        let wb = ((o * ig + i) * kt + t) * kf;
                        for k in 0..kf {
                            let fi = (fo * self.fstride + k).wrapping_sub(pad);
                            if fi < f_in {
                                acc += self.w[wb + k] * row[fi];
                            }
                        }
                    }
                }
                y[o * f_out + fo] = acc;
            }
        }
        if kt > 1 {
            self.hist.rotate_left(frame_len);
            let n = self.hist.len();
            self.hist[n - frame_len..].copy_from_slice(&x[..frame_len]);
        }
    }
}

/// A 1×1 convolution `[out][in]`.
struct Pointwise {
    in_ch: usize,
    out_ch: usize,
    w: Arc<[f32]>,
}

impl Pointwise {
    fn new(w: &Weights, name: &str, in_ch: usize, out_ch: usize) -> Result<Pointwise, ModelError> {
        Ok(Pointwise { in_ch, out_ch, w: vec_of(w, name, out_ch * in_ch)? })
    }

    /// `x`: `[in][f]` → `y`: `[out][f]`.
    fn run(&self, x: &[f32], f: usize, y: &mut [f32]) {
        y[..self.out_ch * f].fill(0.0);
        for o in 0..self.out_ch {
            let yo = &mut y[o * f..(o + 1) * f];
            for i in 0..self.in_ch {
                let wv = self.w[o * self.in_ch + i];
                for (yv, xv) in yo.iter_mut().zip(&x[i * f..(i + 1) * f]) {
                    *yv += wv * xv;
                }
            }
        }
    }
}

/// BatchNorm (inference) + activation, per channel, in place on `[ch][f]`.
struct NormAct {
    scale: Vec<f32>,
    shift: Vec<f32>,
    act: Act,
}

impl NormAct {
    fn new(w: &Weights, prefix: &str, act: Act) -> Result<NormAct, ModelError> {
        let (scale, shift) = w.batch_norm(prefix)?;
        Ok(NormAct { scale, shift, act })
    }

    fn run(&self, x: &mut [f32], f: usize) {
        for (c, (s, b)) in self.scale.iter().zip(&self.shift).enumerate() {
            for v in &mut x[c * f..(c + 1) * f] {
                *v = self.act.apply(*v * s + b);
            }
        }
    }
}

/// `Conv2dNormAct`: conv, optional 1×1 (separable), BatchNorm, activation.
struct ConvBlock {
    conv: Conv,
    pw: Option<Pointwise>,
    norm: NormAct,
    tmp: Vec<f32>,
}

impl ConvBlock {
    fn new(
        w: &Weights,
        p: &str,
        idx: (usize, Option<usize>, usize),
        in_ch: usize,
        out_ch: usize,
        k: (usize, usize),
        fstride: usize,
        groups: usize,
        f_in: usize,
        act: Act,
    ) -> Result<ConvBlock, ModelError> {
        let conv = Conv::new(w, &format!("{p}.{}.weight", idx.0), in_ch, out_ch, k.0, k.1, fstride, groups, f_in)?;
        let pw = match idx.1 {
            Some(i) => Some(Pointwise::new(w, &format!("{p}.{i}.weight"), out_ch, out_ch)?),
            None => None,
        };
        let norm = NormAct::new(w, &format!("{p}.{}", idx.2), act)?;
        let tmp = vec![0.0; out_ch * conv.f_out];
        Ok(ConvBlock { conv, pw, norm, tmp })
    }

    fn f_out(&self) -> usize {
        self.conv.f_out
    }

    fn run(&mut self, x: &[f32], y: &mut [f32]) {
        let f = self.conv.f_out;
        match &self.pw {
            Some(pw) => {
                self.conv.run(x, &mut self.tmp);
                pw.run(&self.tmp, f, y);
            }
            None => self.conv.run(x, y),
        }
        self.norm.run(y, f);
    }
}

/// `ConvTranspose2dNormAct` as used here: depthwise (1 × 3) transposed conv with frequency stride
/// 2 (`f → 2f`), 1×1, BatchNorm, ReLU.
struct UpBlock {
    ch: usize,
    f_in: usize,
    /// `[ch][3]`.
    w: Arc<[f32]>,
    pw: Pointwise,
    norm: NormAct,
    tmp: Vec<f32>,
}

impl UpBlock {
    fn new(w: &Weights, p: &str, ch: usize, f_in: usize) -> Result<UpBlock, ModelError> {
        Ok(UpBlock {
            ch,
            f_in,
            w: vec_of(w, &format!("{p}.0.weight"), ch * 3)?,
            pw: Pointwise::new(w, &format!("{p}.1.weight"), ch, ch)?,
            norm: NormAct::new(w, &format!("{p}.2"), Act::Relu)?,
            tmp: vec![0.0; ch * f_in * 2],
        })
    }

    fn run(&mut self, x: &[f32], y: &mut [f32]) {
        let (f_in, f_out) = (self.f_in, self.f_in * 2);
        self.tmp.fill(0.0);
        for c in 0..self.ch {
            for i in 0..f_in {
                let xv = x[c * f_in + i];
                for k in 0..3 {
                    // o = i·stride − padding + k, padding 1, output padding 1.
                    let o = (2 * i + k).wrapping_sub(1);
                    if o < f_out {
                        self.tmp[c * f_out + o] += xv * self.w[c * 3 + k];
                    }
                }
            }
        }
        self.pw.run(&self.tmp, f_out, y);
        self.norm.run(y, f_out);
    }
}

/// `GroupedLinearEinsum`: `y[g·H/G + h] = Σ_i x[g·I/G + i] · w[g][i][h]`, optional activation.
struct GroupedLinear {
    groups: usize,
    ig: usize,
    hg: usize,
    w: Arc<[f32]>,
    act: Act,
}

impl GroupedLinear {
    fn new(w: &Weights, name: &str, input: usize, hidden: usize, groups: usize, act: Act) -> Result<GroupedLinear, ModelError> {
        let (ig, hg) = (input / groups, hidden / groups);
        Ok(GroupedLinear { groups, ig, hg, w: vec_of(w, name, groups * ig * hg)?, act })
    }

    fn run(&self, x: &[f32], y: &mut [f32]) {
        let (ig, hg) = (self.ig, self.hg);
        y[..self.groups * hg].fill(0.0);
        for g in 0..self.groups {
            let yg = &mut y[g * hg..(g + 1) * hg];
            for i in 0..ig {
                let xv = x[g * ig + i];
                let row = &self.w[(g * ig + i) * hg..(g * ig + i + 1) * hg];
                for (yv, wv) in yg.iter_mut().zip(row) {
                    *yv += xv * wv;
                }
            }
            for v in yg.iter_mut() {
                *v = self.act.apply(*v);
            }
        }
    }
}

/// One PyTorch GRU layer (gate order r, z, n), hidden 256.
struct GruLayer {
    w_ih: Arc<[f32]>,
    w_hh: Arc<[f32]>,
    b_ih: Arc<[f32]>,
    b_hh: Arc<[f32]>,
    h: Vec<f32>,
    gi: Vec<f32>,
    gh: Vec<f32>,
}

impl GruLayer {
    fn new(w: &Weights, p: &str, l: usize) -> Result<GruLayer, ModelError> {
        Ok(GruLayer {
            w_ih: vec_of(w, &format!("{p}.weight_ih_l{l}"), 3 * HIDDEN * HIDDEN)?,
            w_hh: vec_of(w, &format!("{p}.weight_hh_l{l}"), 3 * HIDDEN * HIDDEN)?,
            b_ih: vec_of(w, &format!("{p}.bias_ih_l{l}"), 3 * HIDDEN)?,
            b_hh: vec_of(w, &format!("{p}.bias_hh_l{l}"), 3 * HIDDEN)?,
            h: vec![0.0; HIDDEN],
            gi: vec![0.0; 3 * HIDDEN],
            gh: vec![0.0; 3 * HIDDEN],
        })
    }

    /// `x` (256) → the new hidden state, also left in `self.h`.
    fn step(&mut self, x: &[f32]) {
        for r in 0..3 * HIDDEN {
            self.gi[r] = dot(&self.w_ih[r * HIDDEN..(r + 1) * HIDDEN], x) + self.b_ih[r];
            self.gh[r] = dot(&self.w_hh[r * HIDDEN..(r + 1) * HIDDEN], &self.h) + self.b_hh[r];
        }
        for j in 0..HIDDEN {
            let r = sigmoid(self.gi[j] + self.gh[j]);
            let z = sigmoid(self.gi[HIDDEN + j] + self.gh[HIDDEN + j]);
            let n = (self.gi[2 * HIDDEN + j] + r * self.gh[2 * HIDDEN + j]).tanh();
            let h = (1.0 - z) * n + z * self.h[j];
            // A non-finite state would never recover: start that unit over instead.
            self.h[j] = if h.is_finite() { h } else { 0.0 };
        }
    }
}

/// `SqueezedGRU_S`: grouped linear in (ReLU) → GRU layers → optional grouped linear out (ReLU).
struct SqueezedGru {
    lin_in: GroupedLinear,
    layers: Vec<GruLayer>,
    lin_out: Option<GroupedLinear>,
    x: Vec<f32>,
}

impl SqueezedGru {
    fn new(w: &Weights, p: &str, layers: usize, in_groups: usize, out: bool) -> Result<SqueezedGru, ModelError> {
        let lin_in = GroupedLinear::new(w, &format!("{p}.linear_in.0.weight"), EMB, HIDDEN, in_groups, Act::Relu)?;
        let layers = (0..layers).map(|l| GruLayer::new(w, &format!("{p}.gru"), l)).collect::<Result<Vec<_>, _>>()?;
        let lin_out = if out { Some(GroupedLinear::new(w, &format!("{p}.linear_out.0.weight"), HIDDEN, EMB, 16, Act::Relu)?) } else { None };
        Ok(SqueezedGru { lin_in, layers, lin_out, x: vec![0.0; HIDDEN] })
    }

    fn reset(&mut self) {
        for l in &mut self.layers {
            l.h.fill(0.0);
        }
    }

    /// `input` (512) → `y` (512 with `lin_out`, else 256).
    fn run(&mut self, input: &[f32], y: &mut [f32]) {
        self.lin_in.run(input, &mut self.x);
        for l in &mut self.layers {
            l.step(&self.x);
            self.x.copy_from_slice(&l.h);
        }
        match &self.lin_out {
            Some(o) => o.run(&self.x, y),
            None => y[..HIDDEN].copy_from_slice(&self.x),
        }
    }
}

/// What the network says about one frame.
pub struct NetOut<'a> {
    /// Local SNR estimate, dB (−15…35).
    pub lsnr: f32,
    /// 32 ERB band gains (0…1).
    pub gains: &'a [f32],
    /// Deep-filter taps, `[bin][order]` for the first 96 bins.
    pub coefs: &'a [C32],
}

/// The whole network with its streaming state.
pub struct Net {
    erb_conv0: ConvBlock,
    erb_conv: [ConvBlock; 3],
    df_conv0: ConvBlock,
    df_conv1: ConvBlock,
    df_fc_emb: GroupedLinear,
    enc_gru: SqueezedGru,
    lsnr_w: Arc<[f32]>,
    lsnr_b: f32,
    erb_gru: SqueezedGru,
    convp: [ConvBlock; 4],
    convt3: ConvBlock,
    convt2: UpBlock,
    convt1: UpBlock,
    conv0_out: ConvBlock,
    df_gru: SqueezedGru,
    df_skip: GroupedLinear,
    df_convp: ConvBlock,
    df_out: GroupedLinear,
    // Buffers.
    spec_in: Vec<f32>,
    e: [Vec<f32>; 4],
    c0: Vec<f32>,
    c1: Vec<f32>,
    c1t: Vec<f32>,
    cemb: Vec<f32>,
    emb_in: Vec<f32>,
    emb: Vec<f32>,
    dec: Vec<f32>,
    d: Vec<f32>,
    p: Vec<f32>,
    skip: Vec<f32>,
    gains: Vec<f32>,
    dfh: Vec<f32>,
    dfo: Vec<f32>,
    c0p: Vec<f32>,
    coefs: Vec<C32>,
}

impl Net {
    pub fn new(w: &Weights) -> Result<Net, ModelError> {
        let relu = Act::Relu;
        let erb_conv0 = ConvBlock::new(w, "enc.erb_conv0", (1, None, 2), 1, CH, (3, 3), 1, 1, NB_ERB, relu)?;
        let erb_conv = [
            ConvBlock::new(w, "enc.erb_conv1", (0, Some(1), 2), CH, CH, (1, 3), 2, CH, 32, relu)?,
            ConvBlock::new(w, "enc.erb_conv2", (0, Some(1), 2), CH, CH, (1, 3), 2, CH, 16, relu)?,
            ConvBlock::new(w, "enc.erb_conv3", (0, Some(1), 2), CH, CH, (1, 3), 1, CH, 8, relu)?,
        ];
        let df_conv0 = ConvBlock::new(w, "enc.df_conv0", (1, Some(2), 3), 2, CH, (3, 3), 1, 2, NB_DF, relu)?;
        let df_conv1 = ConvBlock::new(w, "enc.df_conv1", (0, Some(1), 2), CH, CH, (1, 3), 2, CH, NB_DF, relu)?;
        let df_fc_emb = GroupedLinear::new(w, "enc.df_fc_emb.0.weight", CH * NB_DF / 2, EMB, 32, relu)?;
        let enc_gru = SqueezedGru::new(w, "enc.emb_gru", 1, 16, true)?;
        let lsnr_w = vec_of(w, "enc.lsnr_fc.0.weight", EMB)?;
        let lsnr_b = vec_of(w, "enc.lsnr_fc.0.bias", 1)?.first().copied().unwrap_or(0.0);
        let erb_gru = SqueezedGru::new(w, "erb_dec.emb_gru", 2, 16, true)?;
        let convp = [
            ConvBlock::new(w, "erb_dec.conv0p", (0, None, 1), CH, CH, (1, 1), 1, CH, 32, relu)?,
            ConvBlock::new(w, "erb_dec.conv1p", (0, None, 1), CH, CH, (1, 1), 1, CH, 16, relu)?,
            ConvBlock::new(w, "erb_dec.conv2p", (0, None, 1), CH, CH, (1, 1), 1, CH, 8, relu)?,
            ConvBlock::new(w, "erb_dec.conv3p", (0, None, 1), CH, CH, (1, 1), 1, CH, 8, relu)?,
        ];
        let convt3 = ConvBlock::new(w, "erb_dec.convt3", (0, Some(1), 2), CH, CH, (1, 3), 1, CH, 8, relu)?;
        let convt2 = UpBlock::new(w, "erb_dec.convt2", CH, 8)?;
        let convt1 = UpBlock::new(w, "erb_dec.convt1", CH, 16)?;
        let conv0_out = ConvBlock::new(w, "erb_dec.conv0_out", (0, None, 1), CH, 1, (1, 3), 1, 1, 32, Act::Sigmoid)?;
        let df_gru = SqueezedGru::new(w, "df_dec.df_gru", 2, 8, false)?;
        let df_skip = GroupedLinear::new(w, "df_dec.df_skip.weight", EMB, HIDDEN, 16, Act::None)?;
        let df_convp = ConvBlock::new(w, "df_dec.df_convp", (1, Some(2), 3), CH, 2 * DF_ORDER, (5, 1), 1, 2, NB_DF, relu)?;
        // `df_out` is `Sequential(GroupedLinear, Tanh)`; the tanh is applied in `step`.
        let df_out = GroupedLinear::new(w, "df_dec.df_out.0.weight", HIDDEN, NB_DF * 2 * DF_ORDER, 16, Act::None)?;
        if df_conv1.f_out() != NB_DF / 2 || erb_conv[2].f_out() != 8 {
            return Err(err("unexpected layer sizes"));
        }
        Ok(Net {
            erb_conv0,
            erb_conv,
            df_conv0,
            df_conv1,
            df_fc_emb,
            enc_gru,
            lsnr_w,
            lsnr_b,
            erb_gru,
            convp,
            convt3,
            convt2,
            convt1,
            conv0_out,
            df_gru,
            df_skip,
            df_convp,
            df_out,
            spec_in: vec![0.0; 2 * NB_DF],
            e: [vec![0.0; CH * 32], vec![0.0; CH * 16], vec![0.0; CH * 8], vec![0.0; CH * 8]],
            c0: vec![0.0; CH * NB_DF],
            c1: vec![0.0; CH * NB_DF / 2],
            c1t: vec![0.0; CH * NB_DF / 2],
            cemb: vec![0.0; EMB],
            emb_in: vec![0.0; EMB],
            emb: vec![0.0; EMB],
            dec: vec![0.0; EMB],
            d: vec![0.0; CH * 32],
            p: vec![0.0; CH * 32],
            skip: vec![0.0; HIDDEN],
            gains: vec![0.0; NB_ERB],
            dfh: vec![0.0; HIDDEN],
            dfo: vec![0.0; NB_DF * 2 * DF_ORDER],
            c0p: vec![0.0; 2 * DF_ORDER * NB_DF],
            coefs: vec![C32::ZERO; NB_DF * DF_ORDER],
        })
    }

    /// Forget all past frames (a new clip, a seek).
    pub fn reset(&mut self) {
        self.erb_conv0.conv.reset();
        self.df_conv0.conv.reset();
        self.df_convp.conv.reset();
        self.enc_gru.reset();
        self.erb_gru.reset();
        self.df_gru.reset();
    }

    /// One frame: normalised ERB features (32) and complex features (96 bins).
    pub fn step(&mut self, feat_erb: &[f32], feat_spec: &[C32]) -> Result<NetOut<'_>, ModelError> {
        if feat_erb.len() != NB_ERB || feat_spec.len() != NB_DF {
            return Err(err("wrong feature sizes"));
        }
        // Encoder.
        self.erb_conv0.run(feat_erb, &mut self.e[0]);
        {
            let [e0, e1, e2, e3] = &mut self.e;
            self.erb_conv[0].run(e0, e1);
            self.erb_conv[1].run(e1, e2);
            self.erb_conv[2].run(e2, e3);
        }
        for (f, c) in feat_spec.iter().enumerate() {
            self.spec_in[f] = c.re;
            self.spec_in[NB_DF + f] = c.im;
        }
        self.df_conv0.run(&self.spec_in, &mut self.c0);
        self.df_conv1.run(&self.c0, &mut self.c1);
        // [C][F] → [F][C] flattening (PyTorch `permute(0, 2, 3, 1).flatten(2)`).
        let half = NB_DF / 2;
        for c in 0..CH {
            for f in 0..half {
                self.c1t[f * CH + c] = self.c1[c * half + f];
            }
        }
        self.df_fc_emb.run(&self.c1t, &mut self.cemb);
        for c in 0..CH {
            for f in 0..8 {
                self.emb_in[f * CH + c] = self.e[3][c * 8 + f] + self.cemb[f * CH + c];
            }
        }
        self.enc_gru.run(&self.emb_in, &mut self.emb);
        let lsnr = sigmoid(dot(&self.lsnr_w, &self.emb) + self.lsnr_b) * (LSNR_MAX - LSNR_MIN) + LSNR_MIN;

        // ERB decoder.
        self.erb_gru.run(&self.emb, &mut self.dec);
        // [F/8][C] → [C][F/8].
        for c in 0..CH {
            for f in 0..8 {
                self.d[c * 8 + f] = self.dec[f * CH + c];
            }
        }
        self.convp[3].run(&self.e[3], &mut self.p[..CH * 8]);
        add(&mut self.p[..CH * 8], &self.d[..CH * 8]);
        self.convt3.run(&self.p[..CH * 8], &mut self.d[..CH * 8]);
        self.convp[2].run(&self.e[2], &mut self.p[..CH * 8]);
        add(&mut self.p[..CH * 8], &self.d[..CH * 8]);
        self.convt2.run(&self.p[..CH * 8], &mut self.d[..CH * 16]);
        self.convp[1].run(&self.e[1], &mut self.p[..CH * 16]);
        add(&mut self.p[..CH * 16], &self.d[..CH * 16]);
        self.convt1.run(&self.p[..CH * 16], &mut self.d[..CH * 32]);
        self.convp[0].run(&self.e[0], &mut self.p);
        add(&mut self.p, &self.d);
        self.conv0_out.run(&self.p, &mut self.gains);

        // DF decoder.
        self.df_gru.run(&self.emb, &mut self.dfh);
        self.df_skip.run(&self.emb, &mut self.skip);
        add(&mut self.dfh, &self.skip);
        self.df_out.run(&self.dfh, &mut self.dfo);
        self.df_convp.run(&self.c0, &mut self.c0p);
        // coefs[f][2o + ri] = tanh(df_out)[f·10 + 2o + ri] + c0p[2o + ri][f].
        for f in 0..NB_DF {
            for o in 0..DF_ORDER {
                let k = f * 2 * DF_ORDER + 2 * o;
                let re = self.dfo[k].tanh() + self.c0p[(2 * o) * NB_DF + f];
                let im = self.dfo[k + 1].tanh() + self.c0p[(2 * o + 1) * NB_DF + f];
                self.coefs[f * DF_ORDER + o] = C32 { re, im };
            }
        }
        Ok(NetOut { lsnr, gains: &self.gains, coefs: &self.coefs })
    }
}

fn add(a: &mut [f32], b: &[f32]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += y;
    }
}
