//! The vision tower's sizes from a Qwen3-VL `config.json` (AI-PLAN AI4 item 2), checked so a
//! damaged or hostile file can't ask for impossible shapes before any weight is read.

use crate::LlmError;

/// Largest value accepted for any size (the 4B model's largest is 4096).
const MAX_DIM: u64 = 1 << 16;
/// Most encoder blocks accepted.
const MAX_DEPTH: u64 = 256;

/// The vision encoder's shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionConfig {
    pub depth: usize,
    pub hidden: usize,
    pub heads: usize,
    pub intermediate: usize,
    pub patch: usize,
    pub temporal_patch: usize,
    pub merge: usize,
    /// Width of the merged tokens handed to the language model.
    pub out_hidden: usize,
    /// Entries in the learned position table (a square grid).
    pub positions: usize,
    /// Blocks whose output also feeds the language model (DeepStack).
    pub deepstack: Vec<usize>,
}

impl VisionConfig {
    /// Width of one attention head.
    pub fn head_dim(&self) -> usize {
        self.hidden / self.heads
    }
}

/// Parses `vision_config` of a Qwen3-VL `config.json`.
pub fn parse(json: &[u8], file: &str) -> Result<VisionConfig, LlmError> {
    let bad = |reason: String| LlmError::BadModel { file: file.to_string(), reason };
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| bad(format!("not JSON: {e}")))?;
    let vc = v.get("vision_config").ok_or_else(|| bad("no vision_config".into()))?;
    let num = |k: &str, max: u64| -> Result<usize, LlmError> {
        let n = vc.get(k).and_then(|x| x.as_u64()).ok_or_else(|| bad(format!("vision_config.{k} missing")))?;
        if n == 0 || n > max {
            return Err(bad(format!("vision_config.{k} = {n} is out of range")));
        }
        usize::try_from(n).map_err(|_| bad(format!("vision_config.{k} too large")))
    };
    let c = VisionConfig {
        depth: num("depth", MAX_DEPTH)?,
        hidden: num("hidden_size", MAX_DIM)?,
        heads: num("num_heads", MAX_DIM)?,
        intermediate: num("intermediate_size", MAX_DIM)?,
        patch: num("patch_size", 1024)?,
        temporal_patch: num("temporal_patch_size", 16)?,
        merge: num("spatial_merge_size", 16)?,
        out_hidden: num("out_hidden_size", MAX_DIM)?,
        positions: num("num_position_embeddings", MAX_DIM * MAX_DIM)?,
        deepstack: Vec::new(),
    };
    if !c.hidden.is_multiple_of(c.heads) {
        return Err(bad(format!("hidden_size {} is not a multiple of num_heads {}", c.hidden, c.heads)));
    }
    let side = c.positions.isqrt();
    if side * side != c.positions {
        return Err(bad(format!("num_position_embeddings {} is not a square grid", c.positions)));
    }
    if vc.get("hidden_act").and_then(|a| a.as_str()) != Some("gelu_pytorch_tanh") {
        return Err(bad("hidden_act is not gelu_pytorch_tanh (the only activation implemented)".into()));
    }
    let deepstack = match vc.get("deepstack_visual_indexes") {
        None => Vec::new(),
        Some(d) => d
            .as_array()
            .ok_or_else(|| bad("deepstack_visual_indexes is not a list".into()))?
            .iter()
            .map(|x| x.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < c.depth))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| bad("deepstack_visual_indexes must be block numbers below depth".into()))?,
    };
    Ok(VisionConfig { deepstack, ..c })
}

#[cfg(test)]
mod tests {
    use super::*;

    const QWEN3_VL_4B: &str = r#"{"architectures":["Qwen3VLForConditionalGeneration"],"vision_config":{"deepstack_visual_indexes":[5,11,17],"depth":24,"hidden_act":"gelu_pytorch_tanh","hidden_size":1024,"in_channels":3,"intermediate_size":4096,"num_heads":16,"num_position_embeddings":2304,"out_hidden_size":2560,"patch_size":16,"spatial_merge_size":2,"temporal_patch_size":2}}"#;

    #[test]
    fn reads_the_4b_vision_tower() {
        let c = parse(QWEN3_VL_4B.as_bytes(), "config.json").unwrap();
        assert_eq!((c.depth, c.hidden, c.heads, c.head_dim(), c.intermediate), (24, 1024, 16, 64, 4096));
        assert_eq!((c.patch, c.temporal_patch, c.merge, c.out_hidden, c.positions), (16, 2, 2, 2560, 2304));
        assert_eq!(c.deepstack, vec![5, 11, 17]);
        // the processor's constants agree with the model's own config
        assert_eq!((c.patch, c.merge, c.temporal_patch), (crate::vision_prep::PATCH, crate::vision_prep::MERGE, crate::vision_prep::TEMPORAL));
    }

    #[test]
    fn hostile_configs_are_errors() {
        let with = |from: &str, to: &str| QWEN3_VL_4B.replace(from, to);
        for (json, why) in [
            (with("\"num_heads\":16", "\"num_heads\":0"), "zero heads"),
            (with("\"num_heads\":16", "\"num_heads\":7"), "hidden not divisible"),
            (with("\"depth\":24", "\"depth\":100000"), "absurd depth"),
            (with("\"num_position_embeddings\":2304", "\"num_position_embeddings\":2000"), "not square"),
            (with("[5,11,17]", "[5,11,99]"), "deepstack past depth"),
            (with("gelu_pytorch_tanh", "silu"), "other activation"),
            (with("\"patch_size\":16", "\"patch_size\":-16"), "negative"),
            (r#"{"text_config":{}}"#.to_string(), "no vision config"),
            ("not json".to_string(), "not JSON"),
        ] {
            assert!(parse(json.as_bytes(), "c").is_err(), "{why}");
        }
    }

    /// The downloaded model's config, when present.
    #[test]
    #[ignore]
    fn qwen3vl_config_parses() {
        let Some(dir) = std::env::var_os("FILMCRAFT_QWEN3VL_DIR").map(std::path::PathBuf::from) else { return };
        let c = parse(&std::fs::read(dir.join("config.json")).unwrap(), "config.json").unwrap();
        assert_eq!(c, parse(QWEN3_VL_4B.as_bytes(), "c").unwrap());
    }
}
