//! [`Model`] when the crate is built without feature `infer` (e.g. wasm): same API, always
//! unavailable, so callers need no `cfg`.

use std::path::Path;

use crate::{GenOptions, Generation, LlmError, ModelInfo};

/// A loaded model. Without feature `infer` it can never be constructed.
#[derive(Debug)]
pub struct Model {
    _never: std::convert::Infallible,
}

impl Model {
    pub fn load(_dir: &Path, info: &ModelInfo) -> Result<Model, LlmError> {
        Err(LlmError::Unavailable(format!("built without feature `infer`; cannot load {}", info.id)))
    }

    /// Re-hash the files of `info` in `dir` against the catalogue (works without `infer`).
    pub fn verify(dir: &Path, info: &ModelInfo) -> Result<(), LlmError> {
        crate::models::verify(dir, info)
    }

    pub fn info(&self) -> &ModelInfo {
        match self._never {}
    }

    pub fn generate(&mut self, _system: &str, _user: &str, _opts: GenOptions) -> Result<String, LlmError> {
        match self._never {}
    }

    pub fn prompt_tokens(&self, _system: &str, _user: &str) -> Result<usize, LlmError> {
        match self._never {}
    }

    pub fn generate_detailed(&mut self, _system: &str, _user: &str, _opts: GenOptions) -> Result<Generation, LlmError> {
        match self._never {}
    }
}
