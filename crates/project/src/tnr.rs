//! Clip-level temporal noise reduction (fork 6.7b, owner question Q9 (c); design
//! `docs/fork/DESIGN-6.7b.md`): applied to a media / subclip item's picture after the input
//! transform and before its grade and effects, like Resolve's per-clip temporal NR. Same four
//! controls as the v1 grade-node `tnr.*` keys, without the prefix.

use std::collections::BTreeMap;

use crate::grade::{NodeParam, NodeParamKind};
use crate::keyframe::{Param, ParamValue};
use filmcraft_time::Tick;
use serde::{Deserialize, Serialize};

fn on() -> bool {
    true
}

fn is_on(v: &bool) -> bool {
    *v
}

/// A clip's temporal NR settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalNr {
    /// Off keeps the values (the Inspector's switch); default on.
    #[serde(default = "on", skip_serializing_if = "is_on")]
    pub enabled: bool,
    /// Keyed by [`TNR_PARAMS`] (`luma`, `chroma`, `frames`, `motion`); a missing key is the
    /// identity; unknown keys are kept (round trip) and ignored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Param>,
}

impl Default for TemporalNr {
    fn default() -> Self {
        Self { enabled: true, params: BTreeMap::new() }
    }
}

/// The schema: the same four controls as the v1 node keys, without the `tnr.` prefix.
pub const TNR_PARAMS: [NodeParam; 4] = [
    NodeParam { key: "luma", label: "Temporal NR luma", kind: NodeParamKind::Float { identity: 0.0, min: 0.0, max: 1.0 } },
    NodeParam { key: "chroma", label: "Temporal NR chroma", kind: NodeParamKind::Float { identity: 0.0, min: 0.0, max: 1.0 } },
    NodeParam { key: "frames", label: "Temporal NR frames", kind: NodeParamKind::Float { identity: 1.0, min: 1.0, max: 2.0 } },
    NodeParam { key: "motion", label: "Temporal NR motion", kind: NodeParamKind::Float { identity: 0.5, min: 0.0, max: 1.0 } },
];

/// The schema entry for `key`.
pub fn tnr_param(key: &str) -> Option<&'static NodeParam> {
    TNR_PARAMS.iter().find(|p| p.key == key)
}

impl TemporalNr {
    /// A parameter at clip time `t`, clamped to its schema range; non-finite, missing or
    /// mistyped values give the identity (0 for an unknown key). Ignores `enabled`.
    pub fn float_at(&self, key: &str, t: Tick) -> f32 {
        let Some(NodeParamKind::Float { identity, min, max }) = tnr_param(key).map(|p| p.kind) else {
            return 0.0;
        };
        match self.params.get(key).map(|p| p.value_at(t)) {
            Some(ParamValue::Float(v)) if v.is_finite() => (v as f32).clamp(min, max),
            _ => identity,
        }
    }

    /// True when nothing runs at `t`: switched off, or both strengths 0.
    pub fn is_identity_at(&self, t: Tick) -> bool {
        !self.enabled || (self.float_at("luma", t) <= 0.0 && self.float_at("chroma", t) <= 0.0)
    }

    /// True when nothing ever runs: switched off, or both strengths 0 in their static values
    /// and at every keyframe.
    pub fn is_identity_ever(&self) -> bool {
        if !self.enabled {
            return true;
        }
        ["luma", "chroma"].iter().all(|k| {
            let Some(p) = self.params.get(*k) else {
                return true;
            };
            let zero = |v: &ParamValue| !matches!(v, ParamValue::Float(x) if x.is_finite() && *x > 0.0);
            zero(&p.value) && p.keyframes.iter().all(|kf| zero(&kf.value))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_nr_round_trips_and_skips_when_none() {
        // an identity setting writes only what it holds
        let t = TemporalNr::default();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "{}");
        let back: TemporalNr = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        // values, the switch and unknown keys round-trip
        let mut t = TemporalNr { enabled: false, params: BTreeMap::new() };
        t.params.insert("luma".into(), Param::new(ParamValue::Float(0.4)));
        t.params.insert("future".into(), Param::new(ParamValue::Float(3.0)));
        let json = serde_json::to_string(&t).unwrap();
        let back: TemporalNr = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert!(json.contains("future"));
        assert_eq!(back.float_at("future", Tick::ZERO), 0.0);
    }

    #[test]
    fn temporal_nr_float_at_clamps_hostile_values() {
        let mut t = TemporalNr::default();
        for (k, v) in [("luma", 7.0), ("chroma", -1.0), ("frames", 99.0), ("motion", f64::NAN)] {
            t.params.insert(k.into(), Param::new(ParamValue::Float(v)));
        }
        let read = |t: &TemporalNr| ["luma", "chroma", "frames", "motion"].map(|k| t.float_at(k, Tick::ZERO));
        assert_eq!(read(&t), [1.0, 0.0, 2.0, 0.5]);
        t.params.insert("luma".into(), Param::new(ParamValue::Float(f64::INFINITY)));
        t.params.insert("frames".into(), Param::new(ParamValue::Text("2".into())));
        t.params.insert("chroma".into(), Param::new(ParamValue::Float(f64::NEG_INFINITY)));
        assert_eq!(read(&t), [0.0, 0.0, 1.0, 0.5]);
        assert!(t.is_identity_at(Tick::ZERO));
        assert!(t.is_identity_ever());
        assert_eq!(TemporalNr::default().float_at("motion", Tick::ZERO), 0.5);
    }

    #[test]
    fn identity_follows_the_switch_and_keyframes() {
        let mut t = TemporalNr::default();
        assert!(t.is_identity_at(Tick::ZERO) && t.is_identity_ever());
        let mut p = Param::new(ParamValue::Float(0.0));
        p.keyframes.push(crate::keyframe::Keyframe::new(Tick::from_seconds_f64(2.0), ParamValue::Float(0.5)));
        t.params.insert("chroma".into(), p);
        assert!(!t.is_identity_ever());
        t.enabled = false;
        assert!(t.is_identity_ever());
        assert!(t.is_identity_at(Tick::from_seconds_f64(2.0)));
    }
}
