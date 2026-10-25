//! `KnowledgeCenter`: the pinned model catalog as a page, with this machine's share of it marked.
//!
//! The host reads it rather than the window because the catalog is a TOML file beside the app and
//! `onDisk` is a stat of a multi-gigabyte file — neither belongs in the boundary the UI crosses.
//!
//! What this surface says is deliberately different from the status strip's. The strip answers "can
//! this machine run the model, and what is the one thing to do about it"; that verdict needs the
//! hardware probe, the packing choice and the guard. This one lists what the cards claim and which
//! of those files are here, so a caveat below is quoted from the model card, not measured on this
//! machine — which is why they are labelled as the card's and never styled as a warning.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use freecode_config::Settings;
use freecode_llama::EngineBinary;
use freecode_models::catalog::{Caveat, Severity};
use freecode_models::{gguf, Catalog, EngineLock, HostHardware, IntegrityStatus, ModelEntry};

use crate::status::weights_dir;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Center {
    /// The `updated` field of the catalog file, so a page of claims carries the date they were read.
    pub catalog_updated: String,
    /// Where packs are looked for: the per-workspace override or `<root>/models`.
    pub weights_dir: String,
    pub engine_tag: String,
    pub engine_installed: bool,
    /// Catalog order, which is the order the file's authors put the models in.
    pub models: Vec<ModelCard>,
}

impl Center {
    pub fn build(root: &Path, settings: &Settings) -> Result<Self, String> {
        let catalog = Catalog::load_default(root).map_err(|e| e.to_string())?;
        let lock = EngineLock::load_default(root).map_err(|e| e.to_string())?;
        let weights = weights_dir(root, settings);
        let host = HostHardware::detect(&weights.display().to_string());
        // `None` is an answer, not a failure: an unsupported backend has no build to install, and
        // the page says the engine is not installed rather than inventing a path for it.
        let engine = EngineBinary::resolve(root, &lock, &host).ok();

        Ok(Center {
            catalog_updated: catalog.updated.clone(),
            weights_dir: weights.display().to_string(),
            engine_tag: lock.pinned_tag.clone(),
            engine_installed: engine.as_ref().is_some_and(|e| e.installed),
            models: catalog
                .models
                .iter()
                .map(|model| ModelCard::of(model, &weights))
                .collect(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCard {
    pub id: String,
    pub name: String,
    pub repo_id: String,
    /// The card the numbers were read from. Derived from `repo_id`, because every source line in the
    /// catalog file names that URL as the source of truth.
    pub repo_url: String,
    pub license: String,
    pub base_model: String,
    pub architecture: String,
    pub context_length: u32,
    pub packs: Vec<PackRow>,
    /// The card's projector files, which is the half of "this model takes images" that a reader
    /// cannot infer from the language packs. Empty for a model that publishes none.
    pub vision: Vec<VisionRow>,
    /// The card's own caveats, gathered across its packings and deduped: a pack-level note that
    /// repeats on three quantisations is one known issue, not three.
    pub caveats: Vec<CaveatRow>,
    /// Bytes of this model sitting in [`Center::weights_dir`], counted from the packs marked
    /// [`PackRow::on_disk`] — the catalog's sizes rather than a `du`, so the number means "what the
    /// pin says is here" and cannot disagree with the pack rows.
    /// Projectors are deliberately not in this total: a 0.37 GB mmproj with no language pack under it
    /// is not a model this machine can run, and the count that feeds "1 of 2 packs here" would stop
    /// meaning what it says.
    pub held_bytes: u64,
}

impl ModelCard {
    fn of(model: &ModelEntry, weights: &Path) -> Self {
        let packs: Vec<PackRow> = model
            .packs
            .iter()
            .map(|pack| PackRow {
                key: pack.key.clone(),
                file: pack.file.clone(),
                quant_type: pack.quant_type.clone(),
                bits_per_weight: pack.bits_per_weight,
                size_gb: pack.size_gb,
                size_bytes: pack.size_bytes,
                on_disk: on_disk(weights, &pack.file, pack.size_bytes),
                notes: pack.notes.clone(),
            })
            .collect();

        let vision: Vec<VisionRow> = model
            .vision
            .iter()
            .map(|proj| VisionRow {
                key: proj.key.clone(),
                file: proj.file.clone(),
                quant_type: proj.quant_type.clone(),
                size_gb: proj.size_gb,
                size_bytes: proj.size_bytes,
                recommended: proj.recommended,
                on_disk: on_disk(weights, &proj.file, proj.size_bytes),
            })
            .collect();

        let held_bytes = packs
            .iter()
            .filter(|p| p.on_disk)
            .map(|p| p.size_bytes)
            .sum();

        Self {
            id: model.id.clone(),
            name: model.name.clone(),
            repo_id: model.repo_id.clone(),
            repo_url: format!("https://huggingface.co/{}", model.repo_id),
            license: model.license.clone(),
            base_model: model.base_model.clone(),
            architecture: model.architecture.clone(),
            context_length: model.context_length,
            caveats: caveat_rows(model),
            packs,
            vision,
            held_bytes,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackRow {
    pub key: String,
    pub file: String,
    pub quant_type: String,
    pub bits_per_weight: f32,
    pub size_gb: f32,
    pub size_bytes: u64,
    pub on_disk: bool,
    pub notes: String,
}

/// A projector the card publishes for images. It carries no bits-per-weight because it is not a
/// quantisation of the language model — it is a separate network that maps picture embeddings into
/// the model's embedding space, and the catalog says so by keeping it off [`PackRow`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionRow {
    pub key: String,
    pub file: String,
    pub quant_type: String,
    pub size_gb: f32,
    pub size_bytes: u64,
    /// The card's own pick for this model, which is a statement about the pack rather than about
    /// this machine — so it is a label on the row, not the row's state.
    pub recommended: bool,
    pub on_disk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaveatRow {
    pub condition: String,
    pub symptom: String,
    pub workaround: String,
    /// `moderate` | `severe` | `fatal`, as the catalog's own enum serialises them.
    pub severity: String,
}

fn severity(caveat: &Caveat) -> String {
    match caveat.severity {
        Severity::Moderate => "moderate",
        Severity::Severe => "severe",
        Severity::Fatal => "fatal",
    }
    .to_string()
}

fn caveat_rows(model: &ModelEntry) -> Vec<CaveatRow> {
    let mut seen = BTreeSet::new();
    model
        .packs
        .iter()
        .flat_map(|pack| pack.caveats.iter())
        .map(|caveat| CaveatRow {
            condition: caveat.condition.clone(),
            symptom: caveat.symptom.clone(),
            workaround: caveat.workaround.clone(),
            severity: severity(caveat),
        })
        // A `BTreeSet` rather than `Vec::contains`: the dedup is on the whole claim, and the set says
        // so in one place instead of comparing four fields by hand.
        .filter(|row| seen.insert(row.clone()))
        .collect()
}

/// Size only, exactly as the status path counts it: hashing a 6 GB pack to answer a question about a
/// list would take longer than loading the model, and the download verifies the sha256 once when a
/// transfer finishes.
fn on_disk(weights: &Path, file: &str, expected_size_bytes: u64) -> bool {
    matches!(
        gguf::check_size(&weights.join(file), expected_size_bytes),
        Ok(IntegrityStatus::Ok)
    )
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::PathBuf;

    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    /// A checkout with the pinned catalog and nothing fetched anywhere: the weights directory points
    /// at an empty tempdir, so no pack can be on disk by accident of what this machine downloaded.
    fn nothing_fetched() -> (tempfile::TempDir, Settings) {
        let tmp = tempfile::tempdir().unwrap();
        let real = repo_root();
        for rel in ["engines/lock.toml", "catalog/models.toml"] {
            let dest = tmp.path().join(rel);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(real.join(rel), &dest).unwrap_or_else(|e| panic!("{rel}: {e}"));
        }
        let weights = tmp.path().join("empty-weights");
        std::fs::create_dir(&weights).unwrap();
        let mut settings = Settings::default();
        settings.local_model.model_dir = Some(weights.display().to_string());
        (tmp, settings)
    }

    #[test]
    fn every_model_and_every_pack_in_the_catalog_is_on_the_page() {
        let root = repo_root();
        let center = Center::build(&root, &Settings::default()).unwrap();
        let catalog = Catalog::load_default(&root).unwrap();

        assert_eq!(center.catalog_updated, catalog.updated);
        assert_eq!(center.models.len(), catalog.models.len());
        for (card, entry) in center.models.iter().zip(&catalog.models) {
            assert_eq!(card.id, entry.id);
            assert_eq!(card.packs.len(), entry.packs.len());
            // A model's projector files are as much the catalog's as its quantisations: a page that
            // dropped them would hide the only half of a vision model that makes it one.
            assert_eq!(card.vision.len(), entry.vision.len());
            assert_eq!(
                card.vision.iter().filter(|v| v.recommended).count(),
                entry.vision.iter().filter(|v| v.recommended).count(),
            );
            assert_eq!(
                card.repo_url,
                format!("https://huggingface.co/{}", entry.repo_id)
            );
            assert_eq!(card.context_length, entry.context_length);
        }
    }

    #[test]
    fn a_pack_that_was_never_downloaded_says_so_and_holds_no_bytes() {
        let (root, settings) = nothing_fetched();
        let center = Center::build(root.path(), &settings).unwrap();
        assert!(
            center
                .models
                .iter()
                .all(|m| m.packs.iter().all(|p| !p.on_disk)),
            "an empty weights directory cannot hold a pack"
        );
        assert!(
            center
                .models
                .iter()
                .all(|m| m.vision.iter().all(|v| !v.on_disk)),
            "or a projector"
        );
        assert!(center.models.iter().all(|m| m.held_bytes == 0));
    }

    /// The 4B is the model the app points at for images, so its page is the one a reader arrives on
    /// asking "which file adds vision, and which of those should I fetch". A projector the card marks
    /// as not its own pick has to say so on the row: the two differ in whether a concurrent image
    /// run crashes the server, which is the whole reason the card publishes both.
    #[test]
    fn the_vision_models_page_names_its_projectors_and_the_picks() {
        let (root, settings) = nothing_fetched();
        let center = Center::build(root.path(), &settings).unwrap();
        let card = center
            .models
            .iter()
            .find(|m| m.id == "ternary-bonsai-2-4b")
            .expect("the catalog ships the 4B");

        assert_eq!(card.vision.len(), 2);
        assert!(card.vision.iter().any(|v| v.recommended));
        assert_eq!(
            card.vision
                .iter()
                .find(|v| v.recommended)
                .unwrap()
                .quant_type
                .as_str(),
            "Q8_0",
            "the card's pick is the Q8_0 projector, because the BF16 one can crash on concurrent images"
        );
        assert!(card.vision.iter().all(|v| !v.file.is_empty()));

        // A model that publishes no projector gets an empty list, not a fabricated row.
        let plain = center
            .models
            .iter()
            .find(|m| m.id == "ternary-bonsai-1-7b")
            .expect("the catalog ships the 1.7B");
        assert!(
            plain.vision.is_empty(),
            "the 1.7B card publishes no projector"
        );
    }

    #[test]
    fn a_pack_sitting_in_the_weights_directory_is_counted() {
        let (root, mut settings) = nothing_fetched();
        let catalog = Catalog::load_default(root.path()).unwrap();
        // The smallest pack in the file, with the entry that owns it: pack keys repeat across models
        // (both publish an F16 reference), so a pack has to be traced back to its own model rather
        // than looked up by key. The check is on size, so a sparse file of exactly that many bytes
        // proves the read without spending the gigabytes on disk.
        let (entry, pack) = catalog
            .models
            .iter()
            .flat_map(|m| m.packs.iter().map(move |p| (m, p)))
            .min_by_key(|(_, p)| p.size_bytes)
            .unwrap();
        let weights = root.path().join("held-weights");
        std::fs::create_dir(&weights).unwrap();
        settings.local_model.model_dir = Some(weights.display().to_string());
        let file = File::create(weights.join(&pack.file)).unwrap();
        file.set_len(pack.size_bytes).unwrap();

        settings.local_model.catalog_id = entry.id.clone();
        let center = Center::build(root.path(), &settings).unwrap();
        let card = center
            .models
            .iter()
            .find(|m| m.id == settings.local_model.catalog_id)
            .unwrap();
        let row = card.packs.iter().find(|p| p.file == pack.file).unwrap();
        assert!(row.on_disk, "{} should read as present", pack.file);
        assert_eq!(card.held_bytes, pack.size_bytes);

        // The other model is untouched by a file that is not its own.
        let other = center.models.iter().find(|m| m.id != card.id).unwrap();
        assert_eq!(other.held_bytes, 0);
    }

    #[test]
    fn a_caveat_the_catalog_repeats_across_packs_is_listed_once() {
        let root = repo_root();
        let center = Center::build(&root, &Settings::default()).unwrap();
        for card in &center.models {
            let mut deduped = card.caveats.clone();
            deduped.dedup();
            assert_eq!(
                deduped.len(),
                card.caveats.len(),
                "{} repeats a caveat",
                card.id
            );
            assert!(
                card.caveats.iter().all(|c| !c.symptom.is_empty()),
                "a known issue with no symptom is not a known issue"
            );
        }
        assert!(
            center.models.iter().any(|m| !m.caveats.is_empty()),
            "the shipped catalog records known issues; a page that shows none is dropping them"
        );
    }

    #[test]
    fn severity_is_the_catalogs_own_word() {
        let root = repo_root();
        let center = Center::build(&root, &Settings::default()).unwrap();
        let words = ["moderate", "severe", "fatal"];
        for card in &center.models {
            for caveat in &card.caveats {
                assert!(
                    words.contains(&caveat.severity.as_str()),
                    "{}",
                    caveat.severity
                );
            }
        }
    }

    #[test]
    fn the_engine_row_names_the_pin_the_lock_carries() {
        let root = repo_root();
        let lock = EngineLock::load_default(&root).unwrap();
        let center = Center::build(&root, &Settings::default()).unwrap();
        assert_eq!(center.engine_tag, lock.pinned_tag);
    }
}
