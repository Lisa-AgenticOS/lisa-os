//! Model catalog parsing. The catalog is signed *data, not code*
//! (`docs/PLAN.md` §5.2): a TOML index describing models, licenses, and
//! hardware requirements. Signature verification (TUF-style) lands in M1.

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("catalog parse error: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub catalog_version: u32,
    #[serde(default, rename = "model")]
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ModelEntry {
    /// Store ref name, e.g. `qwen3-8b-instruct-q4`.
    pub id: String,
    /// Task slot from PLAN §7: system, vision, embeddings, reranker, asr,
    /// tts, wake-word, code, image-gen.
    pub task: String,
    /// Hardware tiers (PLAN §8) this entry is recommended for.
    pub tiers: Vec<u8>,
    pub license: String,
    /// Inference engine: llama-server, whisper-cpp, sd-cpp, onnx, piper.
    pub engine: String,
    /// Download URL — placeholder until pinned in M1; never invented.
    #[serde(default)]
    pub source: Option<String>,
    /// Pinned blake3 of the exact artifact — populated when `source` is.
    #[serde(default)]
    pub blake3: Option<String>,
    /// A second artifact the engine cannot run without. Piper voices are
    /// two files — the `.onnx` weights and an `.onnx.json` carrying the
    /// phoneme map, sample rate and speaker table — and the weights alone
    /// are an unusable download. Pinned in the same way and for the same
    /// reason as `source`/`blake3`: fetching one and not the other would
    /// install a model that cannot speak, which is a worse outcome than
    /// failing to install it.
    #[serde(default)]
    pub config_source: Option<String>,
    #[serde(default)]
    pub config_blake3: Option<String>,
    #[serde(default)]
    pub min_ram_gb: Option<u32>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Revocation flag honored on catalog refresh (PLAN §5.10).
    #[serde(default)]
    pub revoked: bool,
}

pub fn parse(toml_str: &str) -> Result<Catalog, CatalogError> {
    Ok(toml::from_str(toml_str)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed catalog in-repo must always parse and honor the
    /// license-review policy fields.
    #[test]
    fn seed_catalog_parses() {
        let seed = include_str!("../../../models/catalog/catalog.toml");
        let catalog = parse(seed).unwrap();
        assert_eq!(catalog.catalog_version, 1);
        assert!(!catalog.models.is_empty());
        for m in &catalog.models {
            assert!(!m.id.is_empty());
            assert!(
                !m.license.is_empty(),
                "{}: license review is mandatory",
                m.id
            );
            assert!(!m.tiers.is_empty(), "{}: at least one tier", m.id);
            // A pinned source requires a pinned hash, and vice versa.
            assert_eq!(
                m.source.is_some(),
                m.blake3.is_some(),
                "{}: source and blake3 must be pinned together",
                m.id
            );
            // Same rule for the companion artifact, and one more: a
            // config cannot be pinned for a model that has no source of
            // its own, which would describe half a download.
            assert_eq!(
                m.config_source.is_some(),
                m.config_blake3.is_some(),
                "{}: config_source and config_blake3 must be pinned together",
                m.id
            );
            assert!(
                m.config_source.is_none() || m.source.is_some(),
                "{}: pins a config but no model artifact",
                m.id
            );
        }
    }

    /// A piper voice is weights plus an `.onnx.json`; the weights alone
    /// do not synthesize. The catalog must therefore be able to carry
    /// both, and the shipped TTS entry must actually do so — this is the
    /// field that makes `lisa models get` fetch a usable voice rather
    /// than an unusable file.
    #[test]
    fn the_tts_voice_pins_its_config_not_only_its_weights() {
        let seed = include_str!("../../../models/catalog/catalog.toml");
        let catalog = parse(seed).unwrap();
        let tts: Vec<_> = catalog.models.iter().filter(|m| m.task == "tts").collect();
        assert!(!tts.is_empty(), "the catalog must offer a voice");
        let pinned: Vec<_> = tts.iter().filter(|m| m.source.is_some()).collect();
        assert!(
            !pinned.is_empty(),
            "every tts entry is unpinned — nothing can be installed"
        );
        for m in pinned {
            assert!(
                m.config_source.is_some(),
                "{}: a piper voice without its .onnx.json cannot speak",
                m.id
            );
        }
    }

    /// Licences are reviewed, not assumed. Two of the obvious English
    /// piper voices cannot ship in an image — Blizzard 2013 (lessac)
    /// requires a signed per-organisation form, and RyanSpeech is
    /// CC BY-NC-SA. Neither may reappear as a default by inertia.
    #[test]
    fn no_shipped_model_carries_a_licence_we_cannot_redistribute() {
        let seed = include_str!("../../../models/catalog/catalog.toml");
        let catalog = parse(seed).unwrap();
        for m in catalog.models.iter().filter(|m| m.source.is_some()) {
            let l = m.license.to_ascii_uppercase();
            assert!(
                !l.contains("-NC") && !l.contains("NONCOMMERCIAL"),
                "{}: non-commercial licence cannot ship in the image ({})",
                m.id,
                m.license
            );
            let src = m.source.as_deref().unwrap_or_default();
            assert!(
                !src.contains("lessac"),
                "{}: the Blizzard 2013 voice needs a signed licence form",
                m.id
            );
        }
    }
}
