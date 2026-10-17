//! Qwen3-VL vision encoder (AI-PLAN AI4 item 2): the parity oracle. `qwen3vl_oracle/` is a
//! test-only copy of candle's encoder; FilmCraft's own encoder is checked against it block by
//! block once written. Runs only with `FILMCRAFT_QWEN3VL_DIR` set to the downloaded model folder
//! (weights are never downloaded by tests); skips otherwise.

mod qwen3vl_oracle;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3_vl::config::VisionConfig as CandleVision;
use filmcraft_llm::vision::VisionEncoder;
use filmcraft_llm::{safetensors, vision_config, vision_prep};
use qwen3vl_oracle::vision::Qwen3VLVisionModel;

/// Every `model.visual.*` tensor of the model as f32 candle tensors.
fn vision_tensors(dir: &Path) -> HashMap<String, Tensor> {
    let index = safetensors::parse_index(&std::fs::read(dir.join("model.safetensors.index.json")).unwrap(), "index").unwrap();
    let mut headers = BTreeMap::new();
    let mut out = HashMap::new();
    for (name, file) in index.iter().filter(|(k, _)| k.starts_with("model.visual.")) {
        if !headers.contains_key(file) {
            headers.insert(file.clone(), safetensors::read_header(&dir.join(file)).unwrap());
        }
        let info = &headers[file].tensors[name];
        let data = safetensors::read_f32(&dir.join(file), info, 1 << 26).unwrap();
        let shape: Vec<usize> = info.shape.iter().map(|&d| d as usize).collect();
        out.insert(name.clone(), Tensor::from_vec(data, shape, &Device::Cpu).unwrap());
    }
    out
}

#[test]
fn candle_oracle_encodes_a_picture() {
    let Some(dir) = std::env::var_os("FILMCRAFT_QWEN3VL_DIR").map(std::path::PathBuf::from) else { return };
    let json = std::fs::read(dir.join("config.json")).unwrap();
    let ours = vision_config::parse(&json, "config.json").unwrap();
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    let cfg: CandleVision = serde_json::from_value(value["vision_config"].clone()).unwrap();
    // our config parser reads what candle reads
    assert_eq!(
        (ours.depth, ours.hidden, ours.heads, ours.intermediate, ours.patch, ours.temporal_patch, ours.merge, ours.out_hidden, ours.positions, &ours.deepstack),
        (
            cfg.depth,
            cfg.hidden_size,
            cfg.num_heads,
            cfg.intermediate_size,
            cfg.patch_size,
            cfg.temporal_patch_size,
            cfg.spatial_merge_size,
            cfg.out_hidden_size,
            cfg.num_position_embeddings,
            &cfg.deepstack_visual_indexes
        )
    );

    let vb = VarBuilder::from_tensors(vision_tensors(&dir), DType::F32, &Device::Cpu);
    let model = Qwen3VLVisionModel::new(&cfg, vb.pp("model").pp("visual")).unwrap();

    // a 64 × 64 gradient: 4 × 4 patches → 2 × 2 merged tokens
    let (w, h) = (64usize, 64usize);
    let rgb: Vec<f32> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            [x, y, 1.0 - x * y]
        })
        .collect();
    let ((gh, gw), patches) = vision_prep::patches(&rgb, w, h).unwrap();
    let n = gh * gw;
    let xs = Tensor::from_vec(patches, (n, vision_prep::PATCH_VALUES), &Device::Cpu).unwrap();
    let grid = Tensor::from_vec(vec![1u32, gh as u32, gw as u32], (1, 3), &Device::Cpu).unwrap();
    let (tokens, deepstack) = model.forward(&xs, &grid).unwrap();

    let merged = n / (ours.merge * ours.merge);
    assert_eq!(tokens.dims(), &[merged, ours.out_hidden]);
    assert_eq!(deepstack.len(), ours.deepstack.len());
    for t in std::iter::once(&tokens).chain(&deepstack) {
        assert_eq!(t.dims(), &[merged, ours.out_hidden]);
        let v = t.flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert!(v.iter().all(|x| x.is_finite()));
    }
    let sum = tokens.sum_all().unwrap().to_scalar::<f32>().unwrap();

    // FilmCraft's own encoder against the oracle. Allowed gap: candle's merger uses the tanh GELU,
    // the reference model (and ours) the exact one. With the tanh GELU swapped in, every output
    // matched to 3e-6 (2026-10-16), so the gap is that activation alone.
    let ours_enc = VisionEncoder::load(&dir, &ours).unwrap();
    let ((_, _), patches) = vision_prep::patches(&rgb, w, h).unwrap();
    let mine = ours_enc.encode(&patches, (gh, gw)).unwrap();
    assert_eq!(mine.count, merged);
    let rel = |a: &[f32], t: &Tensor| {
        let b = t.flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(a.len(), b.len());
        let scale = b.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
        a.iter().zip(&b).fold(0f32, |m, (x, y)| m.max((x - y).abs())) / scale
    };
    let gaps: Vec<f32> = std::iter::once(rel(&mine.tokens, &tokens)).chain(mine.deepstack.iter().zip(&deepstack).map(|(a, t)| rel(a, t))).collect();
    eprintln!("ours vs oracle, max |diff| / max |oracle|: tokens then taps {gaps:?}");
    assert!(gaps.iter().all(|g| *g < 2e-3), "{gaps:?}");
    eprintln!("oracle: {merged} tokens × {}, sum {sum}, {} DeepStack taps", ours.out_hidden, deepstack.len());

    // a real-size picture through the whole path, timed (synthetic gradient)
    if std::env::var_os("FILMCRAFT_QWEN3VL_TIME").is_some() {
        let (w, h) = (1024usize, 768usize);
        let rgb: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                [x, y, 1.0 - x * y]
            })
            .collect();
        let t = std::time::Instant::now();
        let out = ours_enc.encode_image(&rgb, w, h).unwrap();
        eprintln!("{w} × {h}: {} tokens in {:.1} s (CPU)", out.count, t.elapsed().as_secs_f32());
        assert!(out.tokens.iter().all(|v| v.is_finite()));
    }
}
