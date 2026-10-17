//! Qwen3-VL as one object (AI-PLAN AI4 item 2): FilmCraft's own picture encoder
//! ([`crate::vision`]) and text model ([`crate::forward`], a GGUF converted by `llm_quantize`)
//! answering a question about one picture. The text model runs on a discrete GPU when there is one
//! (feature `gpu`, the same policy as the explainer: `FILMCRAFT_LLM_GPU`), else on the CPU; a GPU
//! failure answers again on the CPU. Greedy decoding (the answers are short factual descriptions). The model folder holds the Hugging Face release (`config.json`,
//! `tokenizer.json`, the safetensors the encoder reads) and the converted text GGUF.

use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tokenizers::Tokenizer;

use crate::LlmError;
use crate::forward::{Config, Forward, MmPicture, MmToken};
use crate::mrope::{self, Span};
use crate::vision::VisionEncoder;
use crate::vision_config;

/// Most new tokens one answer may have.
pub const MAX_NEW_TOKENS: usize = 1024;
/// Context the text model is loaded with (picture tokens + question + answer).
const CONTEXT: usize = 8192;
/// Largest question accepted, in characters.
const MAX_QUESTION_CHARS: usize = 4000;

fn bad(file: &str, reason: impl Into<String>) -> LlmError {
    LlmError::BadModel { file: file.to_string(), reason: reason.into() }
}

/// A loaded picture-and-text model.
pub struct VisionLanguage {
    encoder: VisionEncoder,
    text: Forward,
    /// The text model on the GPU (None: CPU).
    #[cfg(feature = "gpu")]
    gpu: Option<crate::gpu_forward::GpuForward>,
    /// The picture encoder on the GPU (None: CPU).
    #[cfg(feature = "gpu")]
    gpu_vision: Option<crate::gpu_vision::GpuVisionEncoder>,
    tok: Tokenizer,
    sections: [usize; 3],
    vision_start: u32,
    vision_end: u32,
    stops: Vec<u32>,
}

/// One training example ([`VisionLanguage::training_inputs`]).
pub struct TrainingSample {
    /// Token ids; `-1` where a picture row goes (in order: the k-th `-1` is picture row k).
    pub ids: Vec<i64>,
    /// M-RoPE positions (temporal, height, width) per id.
    pub positions: Vec<[u32; 3]>,
    /// Index of the first answer token in `ids` (the loss is on `ids[answer_start..]`).
    pub answer_start: usize,
    /// The picture rows and DeepStack taps (added after the first text layers).
    pub picture: crate::vision::VisionOutput,
}

impl VisionLanguage {
    /// Loads the model in `dir` with the text weights from `gguf` (a file name inside `dir`).
    pub fn load(dir: &Path, gguf: &str) -> Result<Self, LlmError> {
        if gguf.contains(['/', '\\']) || !gguf.ends_with(".gguf") {
            return Err(bad(gguf, "not a GGUF file name in the model folder"));
        }
        let config = std::fs::read(dir.join("config.json")).map_err(|_| LlmError::NotInstalled("config.json".into()))?;
        let vcfg = vision_config::parse(&config, "config.json")?;
        let sections = mrope_sections(&config)?;
        let encoder = VisionEncoder::load(dir, &vcfg)?;
        let bytes = std::fs::read(dir.join(gguf)).map_err(|_| LlmError::NotInstalled(gguf.into()))?;
        let mut c = Cursor::new(&bytes[..]);
        let s = crate::gguf::check(&mut c, bytes.len() as u64)?;
        let cfg = Config::from_gguf(&s, "qwen3", CONTEXT)?;
        if cfg.head_dim / 2 != sections.iter().sum::<usize>() || cfg.embd != vcfg.out_hidden {
            return Err(bad(gguf, "text model doesn't fit the picture encoder"));
        }
        let text = Forward::load(&mut c, &s, cfg)?;
        let tok = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| bad("tokenizer.json", e.to_string()))?;
        let id = |t: &str| tok.token_to_id(t).ok_or_else(|| bad("tokenizer.json", format!("no {t} token")));
        let (vision_start, vision_end) = (id("<|vision_start|>")?, id("<|vision_end|>")?);
        let stops = ["<|im_end|>", "<|endoftext|>"].iter().filter_map(|t| tok.token_to_id(t)).collect();
        #[cfg(feature = "gpu")]
        let gpu = crate::infer::load_gpu(&text);
        #[cfg(feature = "gpu")]
        let gpu_vision = load_gpu_vision(&encoder);
        Ok(Self {
            encoder,
            text,
            #[cfg(feature = "gpu")]
            gpu,
            #[cfg(feature = "gpu")]
            gpu_vision,
            tok,
            sections,
            vision_start,
            vision_end,
            stops,
        })
    }

    /// Drops the model without waiting on the GPU: its GPU half is dropped on a detached thread
    /// (a hung device's queue can block the dropping thread with no timeout, or panic).
    #[cfg_attr(not(feature = "gpu"), allow(unused_mut))]
    pub fn drop_detached(mut self) {
        #[cfg(feature = "gpu")]
        if let Some(g) = self.gpu.take() {
            crate::infer::drop_detached(g);
        }
        #[cfg(feature = "gpu")]
        if let Some(g) = self.gpu_vision.take() {
            drop_off_thread(g);
        }
        drop(self);
    }

    /// Whether the text model runs on the GPU.
    pub fn on_gpu(&self) -> bool {
        #[cfg(feature = "gpu")]
        return self.gpu.is_some();
        #[cfg(not(feature = "gpu"))]
        false
    }

    /// Whether the picture encoder runs on the GPU.
    pub fn encoder_on_gpu(&self) -> bool {
        #[cfg(feature = "gpu")]
        return self.gpu_vision.is_some();
        #[cfg(not(feature = "gpu"))]
        false
    }

    /// Encodes the picture on the GPU when there is one (a GPU failure drops it and encodes on
    /// the CPU), else on the CPU.
    fn encode(&mut self, rgb: &[f32], w: usize, h: usize, max_pixels: u64) -> Result<crate::vision::VisionOutput, LlmError> {
        #[cfg(feature = "gpu")]
        if let Some(g) = self.gpu_vision.as_ref() {
            match g.encode_image_within(rgb, w, h, max_pixels) {
                Ok(out) => return Ok(out),
                Err(e) => {
                    log::warn!("picture model: the GPU encoder failed ({e}); encoding on the CPU");
                    if let Some(g) = self.gpu_vision.take() {
                        drop_off_thread(g);
                    }
                }
            }
        }
        self.encoder.encode_image_within(rgb, w, h, max_pixels)
    }

    /// Forget the conversation on whichever device runs it.
    fn reset(&mut self) {
        #[cfg(feature = "gpu")]
        if let Some(g) = self.gpu.as_mut() {
            g.reset();
            return;
        }
        self.text.reset();
    }

    /// One `forward_multimodal` step on the GPU (prompts in `prefill_step` slices, so a slow
    /// device isn't given seconds of work in one submission) or the CPU. `Ok(None)`: the GPU
    /// failed and was dropped; the caller starts the answer again (now on the CPU).
    fn step(&mut self, toks: &[MmToken], pos: &[[u32; 3]], picture: Option<MmPicture<'_>>) -> Result<Option<Vec<f32>>, LlmError> {
        #[cfg(feature = "gpu")]
        if let Some(g) = self.gpu.as_mut() {
            let n = g.prefill_step().max(1);
            let mut out = Err(LlmError::Inference("empty step".into()));
            for (t, p) in toks.chunks(n).zip(pos.chunks(n)) {
                // a slice without picture tokens takes no picture
                let pic = picture.filter(|_| t.iter().any(|x| matches!(x, MmToken::Picture(_))));
                out = g.forward_multimodal(t, p, self.sections, pic);
                if out.is_err() {
                    break;
                }
            }
            return match out {
                Ok(l) => Ok(Some(l)),
                Err(e) if g.device_failed() => {
                    log::warn!("picture model: the GPU failed ({e}); answering on the CPU");
                    if let Some(g) = self.gpu.take() {
                        crate::infer::drop_detached(g);
                    }
                    Ok(None)
                }
                Err(e) => Err(e),
            };
        }
        self.text.forward_multimodal(toks, pos, self.sections, picture).map(Some)
    }

    fn tokens(&self, s: &str) -> Result<Vec<u32>, LlmError> {
        Ok(self.tok.encode(s, false).map_err(|e| LlmError::Inference(e.to_string()))?.get_ids().to_vec())
    }

    /// The prompt [`VisionLanguage::ask`] sends for `question` about `pic`: the tokens, their
    /// M-RoPE positions and the next free position (where the answer starts).
    fn prompt(&self, pic: &crate::vision::VisionOutput, question: &str) -> Result<(Vec<MmToken>, Vec<[u32; 3]>, u32), LlmError> {
        let question: String = crate::sanitize(&question.chars().take(MAX_QUESTION_CHARS).collect::<String>());
        let mut before = self.tokens("<|im_start|>user\n")?;
        before.push(self.vision_start);
        let mut after = vec![self.vision_end];
        after.extend(self.tokens(&format!("{question}<|im_end|>\n<|im_start|>assistant\n"))?);
        let mut toks: Vec<MmToken> = before.iter().map(|&t| MmToken::Text(t)).collect();
        toks.extend((0..pic.count).map(MmToken::Picture));
        toks.extend(after.iter().map(|&t| MmToken::Text(t)));
        let (pos, next) = mrope::positions(&[Span::Text(before.len()), Span::Picture { rows: pic.rows, cols: pic.cols }, Span::Text(after.len())])
            .ok_or_else(|| LlmError::Inference("prompt too long".into()))?;
        Ok((toks, pos, next))
    }

    /// AI4 item 3 (offline tuning): exactly what [`VisionLanguage::ask`] would give the text model
    /// for `question` about the picture, followed by `answer` and the end-of-turn token, for a
    /// trainer to learn from. Not used by the app.
    pub fn training_inputs(&mut self, rgb: &[f32], w: usize, h: usize, max_pixels: u64, question: &str, answer: &str) -> Result<TrainingSample, LlmError> {
        let max_pixels = max_pixels.min(CONTEXT as u64 / 2 * 1024);
        let picture = self.encode(rgb, w, h, max_pixels)?;
        let (toks, mut positions, mut next) = self.prompt(&picture, question)?;
        let mut ids: Vec<i64> = toks
            .iter()
            .map(|t| match t {
                MmToken::Text(t) => i64::from(*t),
                MmToken::Picture(_) => -1,
            })
            .collect();
        let answer_start = ids.len();
        let mut tail = self.tokens(&crate::sanitize(answer.trim()))?;
        tail.extend(self.tokens("<|im_end|>")?);
        for t in tail {
            ids.push(i64::from(t));
            positions.push([next, next, next]);
            next = next.checked_add(1).ok_or_else(|| LlmError::Inference("position".into()))?;
        }
        if ids.len() > CONTEXT {
            return Err(LlmError::PromptTooLong { tokens: ids.len(), max: CONTEXT });
        }
        Ok(TrainingSample { ids, positions, answer_start, picture })
    }

    /// Answers `question` about an RGB picture (`w * h * 3` values in 0..=1, row by row),
    /// resized to at most `max_pixels`; at most `max_new` tokens; `cancel` stops it between
    /// tokens.
    pub fn ask(
        &mut self,
        rgb: &[f32],
        w: usize,
        h: usize,
        max_pixels: u64,
        question: &str,
        max_new: usize,
        cancel: Option<&AtomicBool>,
    ) -> Result<String, LlmError> {
        // at most half the context in picture tokens (one per 32 × 32 pixels), so a large
        // `max_pixels` can't spend minutes encoding a picture the prompt then can't hold
        let max_pixels = max_pixels.min(CONTEXT as u64 / 2 * 1024);
        let pic = self.encode(rgb, w, h, max_pixels)?;
        let (toks, pos, next) = self.prompt(&pic, question)?;
        let room = CONTEXT.saturating_sub(toks.len());
        if room == 0 {
            return Err(LlmError::PromptTooLong { tokens: toks.len(), max: CONTEXT });
        }
        // twice at most: a GPU that fails is dropped and the answer starts again on the CPU
        'device: for _ in 0..2 {
            let mut next = next;
            self.reset();
            let Some(mut logits) = self.step(&toks, &pos, Some(MmPicture { tokens: &pic.tokens, deepstack: &pic.deepstack }))? else { continue 'device };
            let mut out = Vec::new();
            for _ in 0..max_new.min(MAX_NEW_TOKENS).min(room) {
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return Err(LlmError::Cancelled);
                }
                let t = argmax(&logits).ok_or_else(|| LlmError::Inference("no logits".into()))?;
                if self.stops.contains(&t) {
                    break;
                }
                out.push(t);
                let Some(l) = self.step(&[MmToken::Text(t)], &[[next, next, next]], None)? else { continue 'device };
                logits = l;
                next = next.checked_add(1).ok_or_else(|| LlmError::Inference("position".into()))?;
            }
            return self.tok.decode(&out, true).map_err(|e| LlmError::Inference(e.to_string()));
        }
        Err(LlmError::Inference("the picture model failed on the GPU and the CPU".into()))
    }
}

/// The picture encoder on a discrete GPU, or None (CPU): the same policy as the text model
/// (`FILMCRAFT_LLM_GPU=0` never, `=1` integrated GPUs too; they encoded no faster than the CPU).
#[cfg(feature = "gpu")]
fn load_gpu_vision(cpu: &VisionEncoder) -> Option<crate::gpu_vision::GpuVisionEncoder> {
    let v = std::env::var("FILMCRAFT_LLM_GPU").ok().map(|s| s.trim().to_ascii_lowercase());
    if matches!(v.as_deref(), Some("0" | "off" | "false" | "cpu")) {
        return None;
    }
    let any = matches!(v.as_deref(), Some("1" | "on" | "true" | "gpu"));
    match crate::gpu_vision::GpuVisionEncoder::new(cpu) {
        Ok(g) if g.is_discrete() || any => Some(g),
        Ok(g) => {
            log::info!("picture encoder: on the CPU ({} is an integrated GPU)", g.adapter());
            drop_off_thread(g);
            None
        }
        Err(e) => {
            log::info!("picture encoder: on the CPU ({e})");
            None
        }
    }
}

/// Drops a GPU object on a detached thread (a hung device can block or panic its drop); leaked
/// when no thread can be spawned.
#[cfg(feature = "gpu")]
fn drop_off_thread<T: Send + 'static>(g: T) {
    let g = std::mem::ManuallyDrop::new(g);
    let spawned = std::thread::Builder::new().name("llm-gpu-drop".into()).spawn(move || {
        let g = std::mem::ManuallyDrop::into_inner(g);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(g)));
    });
    if let Err(e) = spawned {
        log::warn!("picture model: leaking a failed GPU object ({e})");
    }
}

fn argmax(v: &[f32]) -> Option<u32> {
    let (i, _) = v.iter().enumerate().filter(|(_, x)| x.is_finite()).max_by(|a, b| a.1.total_cmp(b.1))?;
    u32::try_from(i).ok()
}

/// `text_config.rope_scaling.mrope_section` of a Qwen3-VL `config.json`: three section sizes.
pub fn mrope_sections(config: &[u8]) -> Result<[usize; 3], LlmError> {
    let v: serde_json::Value = serde_json::from_slice(config).map_err(|e| bad("config.json", e.to_string()))?;
    let s = v.pointer("/text_config/rope_scaling/mrope_section").and_then(|s| s.as_array()).ok_or_else(|| bad("config.json", "no mrope_section"))?;
    let n: Vec<usize> = s.iter().filter_map(|x| x.as_u64().and_then(|x| usize::try_from(x).ok())).filter(|x| *x <= 512).collect();
    match n.as_slice() {
        [t, h, w] if n.len() == s.len() => Ok([*t, *h, *w]),
        _ => Err(bad("config.json", "mrope_section is not three sizes")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mrope_sections_are_read_and_checked() {
        assert_eq!(mrope_sections(br#"{"text_config":{"rope_scaling":{"mrope_section":[24,20,20]}}}"#).unwrap(), [24, 20, 20]);
        for bad in [
            &br#"{}"#[..],
            br#"{"text_config":{"rope_scaling":{"mrope_section":[24,20]}}}"#,
            br#"{"text_config":{"rope_scaling":{"mrope_section":[24,20,-1]}}}"#,
            br#"{"text_config":{"rope_scaling":{"mrope_section":[1,2,99999]}}}"#,
            b"not json",
        ] {
            assert!(mrope_sections(bad).is_err());
        }
    }

    #[test]
    fn argmax_skips_nan_and_handles_empty() {
        assert_eq!(argmax(&[0.1, f32::NAN, 0.5, 0.2]), Some(2));
        assert_eq!(argmax(&[]), None);
    }

    #[test]
    fn a_missing_model_or_bad_file_name_is_an_error() {
        let dir = std::env::temp_dir().join("filmcraft-vl-missing");
        assert!(matches!(VisionLanguage::load(&dir, "../x.gguf"), Err(LlmError::BadModel { .. })));
        assert!(VisionLanguage::load(&dir, "text.gguf").is_err());
    }
}
