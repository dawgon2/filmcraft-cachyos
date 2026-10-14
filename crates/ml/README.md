# filmcraft-ml

The shared on-device ML runtime layer (fork AI-PLAN AI1.5). Pure Rust, CPU, no `unsafe`.

- `simd::level()`: the vector instruction set this CPU offers at run time (`avx512`, `avx2`,
  `sse4.1`, `sse2`, `neon`, `portable`). The fork builds for plain x86-64 (AI1.2), so model code
  must never rely on compile-time AVX.
- `job::guarded(&cancel, |cancel| …)`: runs model work under `catch_unwind`; a panic becomes
  `JobError::Panicked`, a `Cancel` token stops it between steps (`JobError::Cancelled`).
- `ops` (feature `candle`): candle f32 operations whose candle kernels are scalar without
  compile-time AVX, rewritten on candle's matmul (dispatched at run time by `gemm`):
  `conv_transpose1d` (groups 1) and `mean_last`. Used by the Kokoro voices (`filmcraft-tts`);
  meant for voice isolation (AUDIO-PLAN A2) and Magic Mask (GRADE-PLAN G3).

Before porting a new candle model, time its ops with and without the old flag:

```sh
cargo run --release -p filmcraft-speech --features whisper --example candle_ops_bench
CARGO_TARGET_DIR=target/agent-flag RUSTFLAGS="-C target-cpu=x86-64-v3" cargo run --release -p filmcraft-speech --features whisper --example candle_ops_bench
```

and add any op that is much slower without it here. The quantized language model keeps its own
kernels in `filmcraft-llm`.
