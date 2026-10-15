//! FilmCraft's optional, offline, small language model (L2).
//!
//! It turns the grader's structured facts into one or two plain-language sentences for a beginner
//! (GRADE-PLAN.md item 11). Everything here is optional and local:
//!
//! - [`models`]: the catalogue of candidate instruct models (Apache-2.0, quantized GGUF), each
//!   file pinned to a Hugging Face revision with its SHA-256 and size. Weights are **never
//!   bundled or committed**; with feature `download` they are fetched after the user opts in,
//!   through `filmcraft-speech`'s verified downloader.
//! - [`gguf`]: a bounds-checked walk over a GGUF header that runs before any tensor is read,
//!   so a damaged or hostile file is an error, never a huge allocation or a panic.
//! - [`Model`] (feature `infer`): CPU inference with our own forward pass (pure Rust, run-time SIMD dispatch): load, ChatML prompt,
//!   greedy or sampled decoding that stops at end-of-turn, `max_tokens` or a deadline, under
//!   `catch_unwind`. Without the feature, [`Model::load`] returns [`LlmError::Unavailable`], so
//!   callers need no `cfg`.
//! - [`explain_prompt`]: the fixed, grounded prompt for "explain this grade".
//!
//! The caller must still check the output against the facts (numbers, directions, length) and
//! fall back to the template sentence on any mismatch, error or timeout.

#[cfg(feature = "infer")]
pub mod forward;
pub mod gguf;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "infer")]
pub mod kquant;
pub mod models;
mod prompt;
#[cfg(feature = "infer")]
pub mod quant;
#[cfg(feature = "infer")]
pub mod sample;
#[cfg(feature = "infer")]
pub mod weights;

#[cfg(feature = "infer")]
mod infer;
#[cfg(not(feature = "infer"))]
mod stub;

use std::time::Duration;

#[cfg(feature = "infer")]
pub use infer::Model;
pub use models::{DEFAULT_MODEL, Family, ModelFile, ModelInfo};
pub use prompt::{chat_prompt, compact_facts, explain_prompt, sanitize};
#[cfg(not(feature = "infer"))]
pub use stub::Model;

/// The most tokens a prompt plus its answer may use. The candidates support far more, but short
/// contexts keep memory and time small, and explanations are short.
pub const MAX_CONTEXT: usize = 2048;

/// Whether this build can run a model (feature `infer`).
pub const AVAILABLE: bool = cfg!(feature = "infer");

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LlmError {
    #[error("the language model is not available in this build ({0})")]
    Unavailable(String),
    #[error("model file `{0}` is missing")]
    NotInstalled(String),
    #[error("model file `{file}` is {size} bytes, more than the expected {max}")]
    TooLarge { file: String, size: u64, max: u64 },
    #[error("model file `{file}` is damaged: {reason}")]
    BadModel { file: String, reason: String },
    #[error("the prompt is {tokens} tokens; at most {max} fit")]
    PromptTooLong { tokens: usize, max: usize },
    #[error("inference: {0}")]
    Inference(String),
    #[error("the language model stopped unexpectedly: {0}")]
    Panicked(String),
    #[error("download: {0}")]
    Download(String),
    #[error("stopped")]
    Cancelled,
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        LlmError::Io(e.to_string())
    }
}

/// Generation settings.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GenOptions {
    /// Most new tokens (also capped by [`MAX_CONTEXT`] minus the prompt).
    pub max_tokens: usize,
    /// Wall-clock budget for the whole call (prompt + answer). When it runs out, generation stops
    /// and [`Generation::stop`] is [`StopReason::Deadline`].
    pub deadline: Duration,
    /// 0 = greedy (deterministic, the default); otherwise softmax temperature.
    pub temperature: f64,
    /// Penalty for repeating one of the last 64 tokens (1.0 = none).
    pub repeat_penalty: f32,
    /// Seed for sampling (unused when greedy).
    pub seed: u64,
}

impl Default for GenOptions {
    fn default() -> Self {
        Self { max_tokens: 96, deadline: Duration::from_secs(10), temperature: 0.0, repeat_penalty: 1.1, seed: 299_792_458 }
    }
}

/// Why generation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum StopReason {
    /// The model ended its turn.
    EndOfTurn,
    /// `max_tokens` (or the context) was reached.
    MaxTokens,
    /// The deadline passed; the text may end mid-sentence.
    Deadline,
    /// The caller's streaming callback asked to stop ([`Model::generate_streaming`]).
    Cancelled,
}

/// The result of [`Model::generate_detailed`].
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Generation {
    pub text: String,
    pub prompt_tokens: usize,
    pub new_tokens: usize,
    pub stop: StopReason,
    /// Whole call (tokenizing, prefill and decoding).
    pub elapsed: Duration,
    /// Tokenizing plus the prompt's forward pass (time to the first answer token's logits); the
    /// rest of `elapsed` is decoding `new_tokens` tokens.
    pub prefill: Duration,
}

/// Download the missing files of `m` into `models_dir/<id>/`, each checked against its SHA-256
/// and size (feature `download`; the user must have opted in first).
#[cfg(feature = "download")]
pub fn download(models_dir: &std::path::Path, m: &ModelInfo, progress: filmcraft_speech::models::DownloadProgress) -> Result<(), LlmError> {
    use filmcraft_speech::SpeechError;
    if m.is_local() {
        return Err(LlmError::Download(format!(
            "{} has no download source yet (a locally built model): copy its files into {}",
            m.id,
            models::model_dir(models_dir, m).display()
        )));
    }
    filmcraft_speech::models::download_files(&models::model_dir(models_dir, m), m.files, progress).map_err(|e| match e {
        SpeechError::Cancelled => LlmError::Cancelled,
        SpeechError::Io(s) => LlmError::Io(s),
        other => LlmError::Download(other.to_string()),
    })
}

/// The message of a caught panic.
#[cfg_attr(not(feature = "infer"), allow(dead_code))]
pub(crate) fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_string()
    }
}
