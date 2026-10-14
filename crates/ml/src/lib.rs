//! FilmCraft's shared on-device ML runtime layer (fork AI-PLAN AI1.5), used by the neural voices
//! (`filmcraft-tts`), and meant for voice isolation (AUDIO-PLAN A2) and Magic Mask (GRADE-PLAN G3):
//!
//! - [`simd`]: which vector instructions this CPU offers at run time (for logs and evaluations;
//!   the fork builds for plain x86-64 since AI1.2, so nothing may assume AVX2 at compile time);
//! - [`job`]: run a model step under `catch_unwind` with a cancel token;
//! - `ops` (feature `candle`): candle f32 operations whose candle kernels are scalar without
//!   compile-time AVX, rewritten on candle's matmul, which `gemm` dispatches at run time.
//!
//! The quantized language model has its own kernels in `filmcraft-llm` (`quant`, `kquant`).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod job;
#[cfg(feature = "candle")]
pub mod ops;
pub mod simd;
