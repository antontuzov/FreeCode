//! `ModelStatus`: one round trip that answers "can this machine run the model right now, and if
//! not, what is the one thing I must do?".
//!
//! The assembly order is the contract's proof chain: catalog and lock files are read, the hardware
//! is probed, a packing is chosen from that probe, the fork engine is resolved for it, and only
//! then does `smollm_llama::load_guard` hand down the verdict. Nothing here decides policy — the
//! guard owns refusals. `Assembly` is the single place that chain runs, so the UI projection and
//! the native CI probe can never disagree about what this machine does.

use std::path::{Path, PathBuf};

use serde::Serialize;

use smollm_config::{PackingChoice, Settings};
use smollm_llama::load_guard::{self, Blocker, FileState, GuardReport, GuardRequest};
use smollm_llama::{models_root, EngineBinary};
use smollm_models::{
    gguf, recommend, EngineLock, HostHardware, IntegrityStatus, ModelEntry, Pack, Recommendation,
    DEFAULT_HEADROOM_BYTES,
};

/// Everything the guard needed, kept so typed verdicts survive into the CI probe.
pub struct Assembly {
    pub lock: EngineLock,
    pub model: ModelEntry,
    pub pack: Pack,
    /// Absolute path of the weight file this packing expects.
    pub file: PathBuf,
    pub host: HostHardware,
    pub recommendation: Recommendation,
    /// `None` when no lock target exists for this host at all (an unsupported backend).
    pub engine: Option<EngineBinary>,
    pub report: GuardReport,
    pub weights_absent: bool,
}

impl Assembly {
    pub fn engine_installed(&self) -> bool {
        self.engine.as_ref().is_some_and(|e| e.installed)
    }

    pub fn weights_present(&self) -> bool {
        !self.weights_absent
    }

    /// Blockers that mean the tree or the settings are wrong, as opposed to the two a fresh install
    /// legitimately carries: the engine is not unpacked yet and the weights are not downloaded yet.
    /// `tauri-desktop-native-verification` fails on these even when it is still tolerating the
    /// missing engine.
    pub fn contract_failures(&self) -> Vec<String> {
        self.report
            .blockers
            .iter()
            .filter(|b| !self.is_pending_install(b))
            .map(|b| format!("{}: {}", b.title(), b.detail()))
            .collect()
    }

    fn is_pending_install(&self, blocker: &Blocker) -> bool {
        match blocker {
            Blocker::EngineNotInstalled { .. } => true,
            Blocker::IntegrityFailed { file, .. } => self.weights_absent && *file == self.pack.file,
            _ => false,
        }
    }

    pub fn view(&self, settings: &Settings) -> ModelStatusView {
        let mut warnings: Vec<String> = self
            .recommendation
            .warnings
            .iter()
            .cloned()
            .chain(self.report.warnings.iter().map(|w| w.message.clone()))
            .collect();
        warnings.sort();
        warnings.dedup();

        let blocked = self.report.first_blocker().map(|b| {
            let cta = b.cta(&self.lock);
            BlockedView {
                title: b.title().to_string(),
                detail: b.detail(),
                cta_label: cta.label.to_string(),
                cta_url: cta.url,
            }
        });

        ModelStatusView {
            catalog_id: self.model.id.clone(),
            display_name: self.model.name.clone(),
            repo_id: self.model.repo_id.clone(),
            backend: self.host.backend.label().to_string(),
            engine_tag: self.lock.pinned_tag.clone(),
            engine_installed: self.engine_installed(),
            weights_present: self.weights_present(),
            local: local_badge(
                self.report.is_allowed(),
                settings.privacy.allow_remote_providers,
            ),
            packing: self.pack.key.clone(),
            size_gb: self.pack.size_gb,
            warnings,
            packs: self
                .recommendation
                .reports
                .iter()
                .map(|r| PackView {
                    key: r.key.clone(),
                    file: r.file.clone(),
                    bits_per_weight: r.bits_per_weight,
                    size_gb: r.size_gb,
                    selected: r.selected,
                    viable: r.viable,
                    reason: r.reason.clone(),
                })
                .collect(),
            blocked,
        }
    }
}

/// Run the whole proof chain for this machine.
pub fn assemble(root: &Path, settings: &Settings) -> Result<Assembly, String> {
    let catalog = smollm_models::Catalog::load_default(root).map_err(|e| e.to_string())?;
    let lock = EngineLock::load_default(root).map_err(|e| e.to_string())?;
    let model = find_model(&catalog, &settings.local_model.catalog_id)?;

    let weights = weights_dir(root, settings);
    let host = HostHardware::detect(&weights.display().to_string());
    let recommendation = recommend(
        model,
        &host,
        override_key(&settings.local_model.packing),
        DEFAULT_HEADROOM_BYTES,
    );
    let pack = model
        .packs
        .iter()
        .find(|p| p.key == recommendation.selected_key)
        .ok_or_else(|| {
            format!(
                "catalog chose unknown packing {:?}",
                recommendation.selected_key
            )
        })?
        .clone();

    let file = weights.join(&pack.file);
    let state = file_state(&file, pack.size_bytes, &pack.sha256);
    let weights_absent = matches!(state, FileState::Absent);
    // A missing or truncated file has no trustworthy header, so the guard is told `None` rather
    // than a partially parsed one.
    let header = matches!(state, FileState::Verified)
        .then(|| gguf::read_header(&file).ok())
        .flatten();
    let engine = EngineBinary::resolve(root, &lock, &host).ok();

    let report = load_guard::check(&GuardRequest {
        root,
        lock: &lock,
        model,
        pack: &pack,
        file: &file,
        state,
        header: header.as_ref(),
        engine: engine.as_ref(),
        host: Some(&host),
    });

    Ok(Assembly {
        lock,
        model: model.clone(),
        pack,
        file,
        host,
        recommendation,
        engine,
        report,
        weights_absent,
    })
}

pub fn build(root: &Path, settings: &Settings) -> Result<ModelStatusView, String> {
    let assembly = assemble(root, settings)?;
    Ok(assembly.view(settings))
}

fn find_model<'a>(catalog: &'a smollm_models::Catalog, id: &str) -> Result<&'a ModelEntry, String> {
    catalog
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("catalog/models.toml has no model with id {id:?}"))
}

fn override_key(packing: &PackingChoice) -> Option<&str> {
    match packing {
        PackingChoice::Auto => None,
        PackingChoice::Forced(key) => Some(key),
    }
}

/// Where the weights live: the per-workspace override, else `<root>/models`, the sibling of the
/// `engines/` directory the resolver unpacks into.
fn weights_dir(root: &Path, settings: &Settings) -> PathBuf {
    match &settings.local_model.model_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => models_root(root),
    }
}

/// Size-checked here; the sha256 is verified once, when the download finishes.
fn file_state(file: &Path, expected_size_bytes: u64, expected_sha256: &str) -> FileState {
    if expected_sha256.is_empty() {
        return FileState::Unverifiable;
    }
    match gguf::check_size(file, expected_size_bytes) {
        Ok(IntegrityStatus::Ok) => FileState::Verified,
        Ok(IntegrityStatus::Missing) => FileState::Absent,
        Ok(status) => FileState::Mismatch {
            status: status.describe(),
        },
        Err(e) => FileState::Mismatch {
            status: e.to_string(),
        },
    }
}

/// The "Local" badge reflects reality: generation is local only if the guard actually permits this
/// machine to run it, and only while no remote provider is enabled.
fn local_badge(guard_allows: bool, allow_remote_providers: bool) -> bool {
    guard_allows && !allow_remote_providers
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackView {
    pub key: String,
    pub file: String,
    pub bits_per_weight: f32,
    pub size_gb: f32,
    pub selected: bool,
    pub viable: bool,
    pub reason: String,
}

/// The refusal the status strip renders: title, why, and the single action that clears it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedView {
    pub title: String,
    pub detail: String,
    pub cta_label: String,
    pub cta_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusView {
    pub catalog_id: String,
    pub display_name: String,
    pub repo_id: String,
    pub backend: String,
    pub engine_tag: String,
    pub engine_installed: bool,
    pub weights_present: bool,
    pub local: bool,
    pub packing: String,
    pub size_gb: f32,
    pub warnings: Vec<String>,
    pub packs: Vec<PackView>,
    pub blocked: Option<BlockedView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use smollm_models::Catalog;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    #[test]
    fn a_clean_checkout_reports_the_missing_engine_as_the_first_blocker() {
        // Nothing unpacked, nothing downloaded: the refusal shown first must be the one carrying an
        // install action, and it must point at the fork, not a system llama.cpp.
        let status = build(&repo_root(), &Settings::default()).expect("status builds");
        let blocked = status.blocked.expect("a fresh tree blocks the run");
        assert_eq!(blocked.title, "PrismML engine not installed");
        assert!(
            blocked.cta_url.contains("PrismML-Eng"),
            "{}",
            blocked.cta_url
        );
        assert!(!status.local);
        assert!(!status.engine_installed);
        assert!(!status.weights_present);
    }

    #[test]
    fn a_fresh_tree_is_pending_install_not_a_contract_failure() {
        let assembly = assemble(&repo_root(), &Settings::default()).unwrap();
        assert!(
            assembly.contract_failures().is_empty(),
            "{:?}",
            assembly.contract_failures()
        );
        assert!(!assembly.report.is_allowed());
    }

    #[test]
    fn every_catalog_packing_is_offered_to_the_ui_with_a_reason() {
        let status = build(&repo_root(), &Settings::default()).unwrap();
        let catalog = Catalog::load_default(&repo_root()).unwrap();
        assert_eq!(status.packs.len(), catalog.models[0].packs.len());
        assert!(status.packs.iter().all(|p| !p.reason.is_empty()));
        assert_eq!(status.packs.iter().filter(|p| p.selected).count(), 1);
    }

    #[test]
    fn an_unknown_catalog_id_is_a_clear_error_not_a_panic() {
        let mut settings = Settings::default();
        settings.local_model.catalog_id = "not-in-the-catalog".into();
        let err = build(&repo_root(), &settings).unwrap_err();
        assert!(err.contains("not-in-the-catalog"), "{err}");
    }

    #[test]
    fn forcing_a_packing_overrides_the_probe() {
        let mut settings = Settings::default();
        settings.local_model.packing = PackingChoice::Forced("F16".into());
        let status = build(&repo_root(), &settings).expect("a forced pack still builds a status");
        assert_eq!(status.packing, "F16");
    }

    #[test]
    fn a_forced_packing_reuses_the_same_assembly_the_ui_reads() {
        let mut settings = Settings::default();
        settings.local_model.packing = PackingChoice::Forced("PTQ1_0".into());
        let assembly = assemble(&repo_root(), &settings).unwrap();
        assert_eq!(assembly.pack.key, "PTQ1_0");
        assert_eq!(assembly.view(&settings).packing, "PTQ1_0");
    }

    #[test]
    fn the_local_badge_needs_both_a_permitted_run_and_no_remote_provider() {
        assert!(local_badge(true, false));
        assert!(
            !local_badge(true, true),
            "a remote provider takes the badge off"
        );
        assert!(
            !local_badge(false, false),
            "a refused load is not local, it is not running"
        );
    }
}
