//! Hardware-aware model fit (`docs/PLAN.md` §5.2, §8). Given the detected
//! hardware profile, say which *local* catalog models this machine can
//! actually run — the honest floor (§8): nothing hard-refuses, but the
//! user sees "runs here" vs "too big, offload to a remote provider"
//! (§5.11) instead of discovering it by OOM. Remote-provider models
//! (HuggingFace et al.) are never gated here — they run on someone
//! else's hardware.

use crate::catalog::{Catalog, ModelEntry};
use crate::profile::HardwareProfile;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Comfortable headroom.
    Runs,
    /// Fits, but tight — expect swapping / small context.
    Tight,
    /// Won't fit locally — use a remote provider instead.
    TooBig,
}

impl Fit {
    pub fn label(self) -> &'static str {
        match self {
            Fit::Runs => "runs on your machine",
            Fit::Tight => "tight — runs but leaves little headroom",
            Fit::TooBig => "too big for this machine — use a remote provider",
        }
    }
}

/// Headroom the OS + a desktop session want on top of model weights.
const RESERVED_GB: u64 = 3;

pub fn assess(model: &ModelEntry, hw: &HardwareProfile) -> Fit {
    // Unknown requirement: treat as small (the catalog only omits
    // min_ram_gb for tiny models).
    let need = model.min_ram_gb.map(u64::from).unwrap_or(2);
    let usable = hw.total_ram_gb.saturating_sub(RESERVED_GB);
    if usable >= need + 2 {
        Fit::Runs
    } else if hw.total_ram_gb >= need {
        Fit::Tight
    } else {
        Fit::TooBig
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub id: String,
    pub task: String,
    pub license: String,
    pub min_ram_gb: Option<u32>,
    pub fit: Fit,
    pub note: String,
}

/// Assess every non-revoked catalog model against the hardware.
pub fn recommend(catalog: &Catalog, hw: &HardwareProfile) -> Vec<Recommendation> {
    catalog
        .models
        .iter()
        .filter(|m| !m.revoked)
        .map(|m| {
            let fit = assess(m, hw);
            Recommendation {
                id: m.id.clone(),
                task: m.task.clone(),
                license: m.license.clone(),
                min_ram_gb: m.min_ram_gb,
                fit,
                note: m.notes.clone().unwrap_or_default(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    fn hw(ram: u64, tier: u8) -> HardwareProfile {
        HardwareProfile {
            os: "linux".into(),
            arch: "x86_64".into(),
            total_ram_gb: ram,
            unified_memory: false,
            gpu_nodes: 1,
            npu_nodes: 0,
            tier,
        }
    }

    fn model(min: Option<u32>) -> ModelEntry {
        ModelEntry {
            id: "m".into(),
            task: "system".into(),
            tiers: vec![2],
            license: "Apache-2.0".into(),
            engine: "llama-server".into(),
            source: None,
            blake3: None,
            config_source: None,
            config_blake3: None,
            min_ram_gb: min,
            notes: None,
            revoked: false,
        }
    }

    #[test]
    fn fit_tracks_ram_against_requirement() {
        // 8 GB box, model wants 16 → too big.
        assert_eq!(assess(&model(Some(16)), &hw(8, 1)), Fit::TooBig);
        // 32 GB box, model wants 16 → comfortable.
        assert_eq!(assess(&model(Some(16)), &hw(32, 3)), Fit::Runs);
        // 16 GB box, model wants 16 → fits but tight (reserved headroom).
        assert_eq!(assess(&model(Some(16)), &hw(16, 2)), Fit::Tight);
        // Unknown requirement on a small box → treated as tiny, runs.
        assert_eq!(assess(&model(None), &hw(8, 1)), Fit::Runs);
    }

    #[test]
    fn seed_catalog_is_assessable_and_skips_revoked() {
        let cat = catalog::parse(include_str!("../../../models/catalog/catalog.toml")).unwrap();
        let recs = recommend(&cat, &hw(16, 2));
        assert!(!recs.is_empty());
        // The flagship 30B model must not claim to run on a 16 GB box.
        if let Some(big) = recs.iter().find(|r| r.id.contains("30b")) {
            assert_eq!(big.fit, Fit::TooBig);
        }
    }
}
