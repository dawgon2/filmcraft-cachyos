//! Hardware-accelerated decoding: the bitstream side of the decoder, with the picture decoding
//! done by an [`Accelerator`] (VA-API, Vulkan Video, NVDEC… in `filmcraft-platform`).
//!
//! [`AccelDecoder`] does everything the software [`crate::Decoder`] does except reconstructing
//! samples: parameter sets, slice headers, RASL skipping, picture order counts (8.3.1), reference
//! picture sets and missing references (8.3.2–8.3.3), reference picture lists (8.3.4) and the
//! DPB with output / bumping (C.5.2), with the same code (`crate::dpb`), so pictures come out in
//! the same order with the same pts as from the software decoder. Each complete picture goes to
//! the accelerator once, as an [`AccelPicture`]: its parameter sets, the reference pictures in
//! the DPB (with the surfaces the accelerator gave them), and every slice segment as the original
//! NAL unit bytes plus its parsed header and reference lists.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::dpb::{Dpb, Marking, Output, OutputMeta, PocState, RefPicSet, build_ref_lists};
use crate::error::{Error, Result, ensure, invalid};
use crate::params::{Pps, Sps, Vps};
use crate::slice::{NalHeader, SliceHeader, nal_type};
use filmcraft_bitstream::{annexb_nals, length_prefixed_nals, unescape_rbsp};

/// Decodes pictures into surfaces of its own (GPU memory, typically).
pub trait Accelerator {
    /// A decoded (or being-decoded) picture. Cloning shares it: the DPB, reference lists and
    /// the caller's output all hold the same surface.
    type Surface: Clone;

    /// A surface for the next picture (`sps` gives its size, bit depth and chroma format).
    fn new_surface(&mut self, sps: &Sps) -> Result<Self::Surface>;

    /// A stand-in for a reference picture missing from the stream (8.3.3; after a seek into an
    /// open GOP or a damaged stream). Its content only needs to be valid memory.
    fn missing_reference(&mut self, sps: &Sps, poc: i32) -> Result<Self::Surface> {
        let _ = poc;
        self.new_surface(sps)
    }

    /// Decode one picture into `pic.target`.
    fn decode(&mut self, pic: &AccelPicture<'_, Self::Surface>) -> Result<()>;
}

/// How a reference picture is used by the current picture (8.3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefUse {
    /// In RefPicSetStCurrBefore.
    StCurrBefore,
    /// In RefPicSetStCurrAfter.
    StCurrAfter,
    /// In RefPicSetLtCurr.
    LtCurr,
    /// Kept for later pictures, not used by this one.
    Foll,
}

/// A reference picture in the DPB while the current picture decodes.
pub struct AccelRef<'a, S> {
    pub surface: &'a S,
    pub poc: i32,
    pub long_term: bool,
    pub usage: RefUse,
}

/// One slice segment of the current picture.
pub struct AccelSlice<'a> {
    /// The NAL unit as in the stream (2-byte header, emulation prevention bytes included).
    pub nal: &'a [u8],
    pub header: &'a SliceHeader,
    /// Bytes from the NAL unit header to slice_segment_data(), counted without emulation
    /// prevention bytes (header included).
    pub data_byte_offset: usize,
    /// Emulation prevention bytes inside the slice segment header.
    pub header_emulation_bytes: usize,
    /// RefPicList0 / RefPicList1 as indices into [`AccelPicture::refs`].
    pub ref_lists: [Vec<usize>; 2],
}

/// A complete picture for the accelerator.
pub struct AccelPicture<'a, S> {
    pub target: &'a S,
    pub poc: i32,
    pub nal: NalHeader,
    pub vps: Option<&'a Vps>,
    pub sps: &'a Sps,
    pub pps: &'a Pps,
    /// Every reference picture in the DPB (short- and long-term, used by this picture or kept
    /// for later ones), at most 15 for a conforming stream.
    pub refs: Vec<AccelRef<'a, S>>,
    pub slices: Vec<AccelSlice<'a>>,
    /// An IRAP picture with NoRaslOutputFlag = 1.
    pub irap_no_rasl: bool,
}

/// A picture leaving the decoder, in output order.
pub struct AccelOutput<S> {
    pub surface: S,
    pub poc: i32,
    pub meta: Arc<OutputMeta>,
}

struct PendingSlice {
    nal: Vec<u8>,
    sh: SliceHeader,
    data_byte_offset: usize,
    header_emulation_bytes: usize,
    lists: [Vec<i32>; 2],
}

struct Pending<S> {
    surface: S,
    sps: Arc<Sps>,
    pps: Arc<Pps>,
    first_sh: SliceHeader,
    last_sh: SliceHeader,
    rps: RefPicSet<S>,
    poc: i32,
    output: bool,
    meta: Arc<OutputMeta>,
    irap_no_rasl: bool,
    slices: Vec<PendingSlice>,
}

/// The bitstream half of an HEVC decoder driving an [`Accelerator`] (see the module docs).
pub struct AccelDecoder<A: Accelerator> {
    pub accel: A,
    vpss: Vec<Option<Arc<Vps>>>,
    spss: Vec<Option<Arc<Sps>>>,
    ppss: Vec<Option<Arc<Pps>>>,
    nal_length_size: Option<usize>,
    dpb: Dpb<A::Surface>,
    poc_state: PocState,
    pending: Option<Pending<A::Surface>>,
    active_sps: Option<Arc<Sps>>,
    first_picture: bool,
    after_eos: bool,
    skip_rasl: bool,
    out: VecDeque<AccelOutput<A::Surface>>,
    /// Pictures decoded, and RASL pictures skipped.
    pub pictures: u64,
    pub skipped_rasl: u64,
}

impl<A: Accelerator> AccelDecoder<A> {
    pub fn new(accel: A) -> Self {
        Self {
            accel,
            vpss: vec![None; 16],
            spss: vec![None; 16],
            ppss: vec![None; 64],
            nal_length_size: None,
            dpb: Dpb::default(),
            poc_state: PocState::default(),
            pending: None,
            active_sps: None,
            first_picture: true,
            after_eos: false,
            skip_rasl: false,
            out: VecDeque::new(),
            pictures: 0,
            skipped_rasl: 0,
        }
    }

    /// Configure from an `hvcC` record (parameter sets and the NAL length size).
    pub fn configure_hvcc(&mut self, hvcc: &[u8]) -> Result<()> {
        let (len, nals) = crate::decoder::hvcc_parameter_sets(hvcc)?;
        self.nal_length_size = Some(len);
        for n in nals {
            self.handle_nal(n, 0)?;
        }
        Ok(())
    }

    /// Feed parameter-set NAL units directly (e.g. from a sample entry), Annex B framing off.
    pub fn add_parameter_set(&mut self, nal: &[u8]) -> Result<()> {
        self.handle_nal(nal, 0)
    }

    /// Set the NAL length size of length-prefixed input (None: Annex B).
    pub fn set_nal_length_size(&mut self, n: Option<usize>) {
        self.nal_length_size = n;
    }

    /// The active SPS (after the first picture).
    pub fn active_sps(&self) -> Option<&Arc<Sps>> {
        self.active_sps.as_ref()
    }

    /// Decode one access unit; returns the pictures that became ready for output, in order.
    pub fn decode(&mut self, data: &[u8], pts: i64) -> Result<Vec<AccelOutput<A::Surface>>> {
        let nals = match self.nal_length_size {
            Some(n) => length_prefixed_nals(data, n)?,
            None => annexb_nals(data),
        };
        for nal in nals {
            self.handle_nal(nal, pts)?;
        }
        self.submit_pending()?;
        Ok(self.out.drain(..).collect())
    }

    /// Output every remaining picture (end of stream); the next picture starts a new sequence.
    pub fn flush(&mut self) -> Result<Vec<AccelOutput<A::Surface>>> {
        let r = self.submit_pending();
        let mut outs = Vec::new();
        self.dpb.flush(&mut outs);
        self.queue(outs);
        self.first_picture = true;
        r?;
        Ok(self.out.drain(..).collect())
    }

    /// Forget every picture (seek): nothing is output, the next picture must be a random access point.
    pub fn reset(&mut self) {
        self.pending = None;
        self.dpb.clear();
        self.out.clear();
        self.first_picture = true;
        self.after_eos = false;
        self.skip_rasl = false;
        self.poc_state = PocState::default();
    }

    fn queue(&mut self, outs: Vec<Output<A::Surface>>) {
        self.out.extend(outs.into_iter().map(|o| AccelOutput { surface: o.frame, poc: o.poc, meta: o.meta }));
    }

    fn handle_nal(&mut self, nal: &[u8], pts: i64) -> Result<()> {
        if nal.len() < 2 {
            return Ok(());
        }
        let hdr = NalHeader::parse(nal)?;
        if hdr.layer_id != 0 {
            return Ok(());
        }
        let payload = nal.get(2..).unwrap_or(&[]);
        match hdr.nal_type {
            nal_type::VPS => {
                let v = Vps::parse(&unescape_rbsp(payload))?;
                if let Some(slot) = self.vpss.get_mut(v.id as usize) {
                    *slot = Some(Arc::new(v));
                }
            }
            nal_type::SPS => {
                let s = Sps::parse(&unescape_rbsp(payload))?;
                if let Some(slot) = self.spss.get_mut(s.id as usize) {
                    *slot = Some(Arc::new(s));
                }
            }
            nal_type::PPS => {
                let p = Pps::parse(&unescape_rbsp(payload))?;
                if let Some(slot) = self.ppss.get_mut(p.id as usize) {
                    *slot = Some(Arc::new(p));
                }
            }
            nal_type::EOS | nal_type::EOB => {
                self.submit_pending()?;
                self.after_eos = true;
            }
            0..=9 | 16..=21 => self.handle_slice(nal, hdr, pts)?,
            _ => {}
        }
        Ok(())
    }

    fn handle_slice(&mut self, nal: &[u8], hdr: NalHeader, pts: i64) -> Result<()> {
        let payload = nal.get(2..).unwrap_or(&[]);
        let rbsp = unescape_rbsp(payload);
        let first = rbsp.first().is_some_and(|b| b & 0x80 != 0);
        if first {
            self.submit_pending()?;
        }
        if hdr.is_rasl() && (self.skip_rasl || self.first_picture) {
            if first {
                self.skipped_rasl += 1;
            }
            return Ok(());
        }
        let (ppss, spss) = (&self.ppss, &self.spss);
        let prev = self.pending.as_ref().map(|p| &p.last_sh);
        let (sh, pps, sps) = SliceHeader::parse(
            &rbsp,
            hdr,
            |id| {
                let pps = ppss.get(id as usize).and_then(Option::as_ref).ok_or_else(|| Error::MissingParameterSet(format!("PPS {id}")))?;
                let sps = spss.get(pps.sps_id as usize).and_then(Option::as_ref).ok_or_else(|| Error::MissingParameterSet(format!("SPS {}", pps.sps_id)))?;
                Ok((pps.clone(), sps.clone()))
            },
            prev,
        )?;
        sps.check_supported()?;
        pps.check_supported()?;
        if first {
            self.start_picture(&sh, &sps, &pps, pts)?;
        }
        let Some(pending) = self.pending.as_mut() else { return invalid("slice segment without the first slice of its picture") };
        ensure!(Arc::ptr_eq(&pending.pps, &pps), "PPS changed within a picture");
        let refs = build_ref_lists(&sh, &pending.rps)?;
        let lists = [refs[0].iter().map(|r| r.poc).collect(), refs[1].iter().map(|r| r.poc).collect()];
        let (_, header_emulation_bytes) = escaped_position(payload, sh.data_offset);
        let data_byte_offset = 2 + sh.data_offset;
        if !sh.dependent {
            pending.last_sh = sh.clone();
        }
        pending.slices.push(PendingSlice { nal: nal.to_vec(), sh, data_byte_offset, header_emulation_bytes, lists });
        Ok(())
    }

    fn start_picture(&mut self, sh: &SliceHeader, sps: &Arc<Sps>, pps: &Arc<Pps>, pts: i64) -> Result<()> {
        ensure!(sh.first_slice_segment_in_pic && !sh.dependent, "invalid first slice segment");
        let hdr = sh.nal;
        let changed = self.active_sps.as_ref().is_none_or(|a| {
            a.width != sps.width
                || a.height != sps.height
                || a.bit_depth_luma != sps.bit_depth_luma
                || a.bit_depth_chroma != sps.bit_depth_chroma
                || a.chroma_format_idc != sps.chroma_format_idc
                || a.log2_ctb != sps.log2_ctb
        });
        let mut outs = Vec::new();
        if changed && self.active_sps.is_some() {
            self.dpb.flush(&mut outs);
        }
        self.active_sps = Some(sps.clone());
        self.dpb.max_dec_pic_buffering = sps.max_dec_pic_buffering as usize;
        self.dpb.max_num_reorder = sps.max_num_reorder as usize;
        self.dpb.max_latency = if sps.max_latency_increase_plus1 != 0 { sps.max_num_reorder + sps.max_latency_increase_plus1 - 1 } else { 0 };
        let irap_no_rasl = hdr.is_irap() && (hdr.is_idr() || hdr.is_bla() || self.first_picture || self.after_eos);
        if hdr.is_irap() {
            self.skip_rasl = irap_no_rasl;
        }
        let max_lsb = sps.max_poc_lsb();
        let poc = self.poc_state.compute(sh, max_lsb, irap_no_rasl);
        let accel = &mut self.accel;
        let mut missing = |p: i32| accel.missing_reference(sps, p);
        let rps = self.dpb.apply_rps(sh, poc, max_lsb, irap_no_rasl, &mut missing)?;
        if irap_no_rasl && !self.first_picture {
            if sh.no_output_of_prior_pics && hdr.is_idr() {
                self.dpb.clear();
            } else {
                self.dpb.flush(&mut outs);
            }
        } else {
            self.dpb.bump_before_decode(&mut outs);
        }
        self.queue(outs);
        let surface = self.accel.new_surface(sps)?;
        let output = sh.pic_output && !(hdr.is_rasl() && self.skip_rasl);
        let meta = Arc::new(crate::decoder::output_meta(sps, pts, hdr.is_irap(), false));
        self.pending = Some(Pending {
            surface,
            sps: sps.clone(),
            pps: pps.clone(),
            first_sh: sh.clone(),
            last_sh: sh.clone(),
            rps,
            poc,
            output,
            meta,
            irap_no_rasl,
            slices: Vec::new(),
        });
        self.first_picture = false;
        self.after_eos = false;
        Ok(())
    }

    /// Decode the pending picture on the accelerator and insert it into the DPB.
    fn submit_pending(&mut self) -> Result<()> {
        let Some(p) = self.pending.take() else { return Ok(()) };
        // every reference picture: the DPB's (marked by the RPS), then generated missing ones
        let mut refs: Vec<AccelRef<'_, A::Surface>> = Vec::new();
        let usage = |poc: i32| -> RefUse {
            if p.rps.st_curr_before.iter().any(|r| r.poc == poc) {
                RefUse::StCurrBefore
            } else if p.rps.st_curr_after.iter().any(|r| r.poc == poc) {
                RefUse::StCurrAfter
            } else if p.rps.lt_curr.iter().any(|r| r.poc == poc) {
                RefUse::LtCurr
            } else {
                RefUse::Foll
            }
        };
        for e in self.dpb.entries.iter().filter(|e| e.marking != Marking::Unused) {
            refs.push(AccelRef { surface: &e.frame, poc: e.poc, long_term: e.marking == Marking::Long, usage: usage(e.poc) });
        }
        for r in p.rps.st_curr_before.iter().chain(&p.rps.st_curr_after).chain(&p.rps.lt_curr) {
            if !refs.iter().any(|x| x.poc == r.poc) {
                refs.push(AccelRef { surface: &r.frame, poc: r.poc, long_term: r.long_term, usage: usage(r.poc) });
            }
        }
        let mut slices = Vec::with_capacity(p.slices.len());
        for s in &p.slices {
            let mut lists: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
            for (l, list) in s.lists.iter().enumerate() {
                for poc in list {
                    let i = refs.iter().position(|r| r.poc == *poc).ok_or_else(|| Error::Invalid("reference picture not in the DPB".into()))?;
                    lists[l].push(i);
                }
            }
            slices.push(AccelSlice {
                nal: &s.nal,
                header: &s.sh,
                data_byte_offset: s.data_byte_offset,
                header_emulation_bytes: s.header_emulation_bytes,
                ref_lists: lists,
            });
        }
        let vps = self.vpss.get(p.sps.vps_id as usize).and_then(Option::as_deref);
        let pic =
            AccelPicture { target: &p.surface, poc: p.poc, nal: p.first_sh.nal, vps, sps: &p.sps, pps: &p.pps, refs, slices, irap_no_rasl: p.irap_no_rasl };
        let result = if pic.slices.is_empty() { Ok(()) } else { self.accel.decode(&pic) };
        drop(pic);
        self.pictures += 1;
        self.poc_state.update(&p.first_sh, p.poc);
        // a failed picture still enters the DPB (as the software decoder keeps a damaged one), so
        // later pictures find their reference; the error reaches the caller
        let mut outs = Vec::new();
        self.dpb.insert(p.surface, p.poc, p.output, p.meta, &mut outs);
        self.queue(outs);
        result
    }
}

/// Map a byte offset in the RBSP (emulation prevention bytes removed) of a NAL unit payload to the
/// offset in the escaped payload, and count the emulation prevention bytes before it.
fn escaped_position(escaped: &[u8], rbsp_offset: usize) -> (usize, usize) {
    let (mut zeros, mut rbsp, mut epb) = (0usize, 0usize, 0usize);
    for (i, &b) in escaped.iter().enumerate() {
        if rbsp >= rbsp_offset {
            return (i, epb);
        }
        if zeros >= 2 && b == 3 {
            epb += 1;
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        rbsp += 1;
    }
    (escaped.len(), epb)
}

#[cfg(test)]
mod tests {
    use super::escaped_position;

    #[test]
    fn rbsp_offsets_map_past_emulation_prevention_bytes() {
        // 00 00 03 01 | 00 00 03 00 | ff : RBSP 00 00 01 00 00 00 ff
        let esc = [0u8, 0, 3, 1, 0, 0, 3, 0, 0xff];
        assert_eq!(escaped_position(&esc, 0), (0, 0));
        assert_eq!(escaped_position(&esc, 2), (2, 0));
        assert_eq!(escaped_position(&esc, 3), (4, 1));
        assert_eq!(escaped_position(&esc, 6), (8, 2));
        assert_eq!(escaped_position(&esc, 7), (9, 2));
        assert_eq!(escaped_position(&esc, 99), (9, 2));
        assert_eq!(escaped_position(&[], 5), (0, 0));
    }
}
