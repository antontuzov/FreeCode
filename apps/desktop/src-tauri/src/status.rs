//! `ModelStatus`: one round trip that answers "can this machine run the model right now, and if
//! not, what is the one thing I must do?".
//!
//! The assembly order is the contract's proof chain: catalog and lock files are read, the hardware
//! is probed, a packing is chosen from that probe, the fork engine is resolved for it, and only
//! then does `freecode_llama::load_guard` hand down the verdict. Nothing here decides policy — the
//! guard owns refusals. `Assembly` is the single place that chain runs, so the UI projection and
//! the native CI probe can never disagree about what this machine does.

use std::path::{Path, PathBuf};

use serde::Serialize;

use freecode_config::{ModelSource, PackingChoice, Settings};
use freecode_llama::load_guard::{self, Blocker, FileState, GuardReport, GuardRequest};
use freecode_llama::{models_root, EngineBinary};
use freecode_models::{
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
            .filter(|b| !self.pending_fetch(b))
            .map(|b| format!("{}: {}", b.code().as_str(), b.technical()))
            .collect()
    }

    /// The refusals that are simply work the app has not done yet, and which its own fetch buttons
    /// clear: the pinned engine is not unpacked, or the chosen pack is not on disk.
    fn pending_fetch(&self, blocker: &Blocker) -> bool {
        matches!(blocker, Blocker::EngineNotInstalled { .. })
            || (self.weights_absent
                && matches!(blocker, Blocker::IntegrityFailed { file, .. } if *file == self.pack.file))
    }

    pub fn view(&self, settings: &Settings) -> ModelStatusView {
        // A caveat whose condition this build cannot evaluate is not a claim about this machine.
        // It travels in its own field so the UI can fold it behind a disclosure, rather than
        // printing another platform's bug report as if it were a warning about this one.
        let mut warnings: Vec<String> = self
            .host
            .backend_override
            .iter()
            .map(|o| o.reason.clone())
            .chain(self.recommendation.warnings.iter().cloned())
            .chain(
                self.report
                    .warnings
                    .iter()
                    .filter(|w| w.code != load_guard::WarningCode::CaveatUnevaluated)
                    .map(|w| w.message.clone()),
            )
            .collect();
        warnings.sort();
        warnings.dedup();

        let mut unchecked_warnings: Vec<String> = self
            .report
            .warnings
            .iter()
            .filter(|w| w.code == load_guard::WarningCode::CaveatUnevaluated)
            .map(|w| w.message.clone())
            .collect();
        unchecked_warnings.sort();
        unchecked_warnings.dedup();

        let blocked = self.report.first_blocker().map(|b| BlockedView {
            info: freecode_core::blocked_info(b, &self.lock, &self.model.repo_id),
            // Two refusals are things this app clears itself by fetching what it pinned. Every
            // other one is a fact about the tree — a stock binary, a stale pin, a file whose bytes
            // disagree with the catalog — and the honest answer there is the URL, not a button
            // that would overwrite a file the user put there.
            installable: matches!(b, Blocker::EngineNotInstalled { .. }),
            downloadable: self.weights_absent && matches!(b, Blocker::IntegrityFailed { .. }),
        });

        let remote_selected = settings.remote_provider.source == ModelSource::Remote;
        let remote = remote_selected.then(|| RemoteView {
            base_url: settings.remote_provider.base_url.clone(),
            model: settings.remote_provider.model.clone(),
            context_tokens: settings.remote_provider.context_tokens,
            allowed: settings.privacy.allow_remote_providers,
            configured: settings.remote_provider.configured(),
        });
        let free = settings
            .free_model
            .as_deref()
            // The free tier is the built-in route, so a document with no pick still names the model
            // the next prompt will reach — the card and the footer would otherwise describe weights
            // this build no longer runs. A selected remote provider is the one source that stands
            // in front of it.
            .or_else(|| (!remote_selected).then_some(freecode_config::DEFAULT_FREE_MODEL_ID))
            .and_then(crate::free_models::find)
            .map(|model| FreeView {
                id: model.id.to_string(),
                label: model.label.to_string(),
                provider: model.provider.to_string(),
            });

        ModelStatusView {
            catalog_id: self.model.id.clone(),
            display_name: self.model.name.clone(),
            repo_id: self.model.repo_id.clone(),
            backend: self.host.backend.label().to_string(),
            engine_tag: self.lock.pinned_tag.clone(),
            engine_installed: self.engine_installed(),
            weights_present: self.weights_present(),
            local: local_badge(self.report.is_allowed(), remote_selected, free.is_some()),
            remote,
            free,
            // Filled by the command after the probe: the status build is synchronous and this one
            // fact is a live process rather than a document on disk.
            opencode: OpenCodeView::default(),
            packing: self.pack.key.clone(),
            size_gb: self.pack.size_gb,
            warnings,
            unchecked_warnings,
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
    let catalog = freecode_models::Catalog::load_default(root).map_err(|e| e.to_string())?;
    let lock = EngineLock::load_default(root).map_err(|e| e.to_string())?;
    let model = find_model(&catalog, &settings.local_model.catalog_id)?;

    let weights = weights_dir(root, settings);
    let host = HostHardware::detect(&weights.display().to_string());
    let installed = installed_packs(model, &weights);
    let recommendation = recommend(
        model,
        &host,
        override_key(&settings.local_model.packing),
        DEFAULT_HEADROOM_BYTES,
        &installed,
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

fn find_model<'a>(
    catalog: &'a freecode_models::Catalog,
    id: &str,
) -> Result<&'a ModelEntry, String> {
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

/// Weight files already in the directory, by name. Size is the whole check: hashing 5.95 GB on
/// every status round trip would take longer than loading the model, and the download path verifies
/// the sha256 once when a transfer finishes.
fn installed_packs(model: &ModelEntry, weights: &Path) -> Vec<String> {
    model
        .packs
        .iter()
        .filter(|p| {
            matches!(
                gguf::check_size(&weights.join(&p.file), p.size_bytes),
                Ok(IntegrityStatus::Ok)
            )
        })
        .map(|p| p.file.clone())
        .collect()
}

/// Where the weights live: the per-workspace override, else `<root>/models`, the sibling of the
/// `engines/` directory the resolver unpacks into.
pub(crate) fn weights_dir(root: &Path, settings: &Settings) -> PathBuf {
    match &settings.local_model.model_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => models_root(root),
    }
}

/// Size-checked here; the sha256 is verified once, when the download finishes.
pub(crate) fn file_state(
    file: &Path,
    expected_size_bytes: u64,
    expected_sha256: &str,
) -> FileState {
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

/// The "Local" badge reflects reality: generation is local only if the guard permits this machine to
/// run it, and only while the local source is the one selected. Consent alone does not take the badge
/// away — a local turn never falls back to the provider, so with Local selected it is still local —
/// but choosing the provider or a free model does. `chat` uses the same fact for a live run by asking
/// where the run was actually resolved, so the transcript and the status card cannot disagree.
pub(crate) fn local_badge(guard_allows: bool, remote_selected: bool, free_selected: bool) -> bool {
    guard_allows && !remote_selected && !free_selected
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

/// The refusal the status strip renders. The `info` fields are the same ones the transcript's refusal
/// row gets, flattened rather than copied, so the two surfaces cannot word — or code — the same
/// refusal differently. What is added here is which of the two the app can clear by itself.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedView {
    #[serde(flatten)]
    pub info: freecode_core::BlockedInfo,
    /// True when the app can clear this itself by fetching the pinned engine; false when the fix is
    /// a file or a pin change and the URL is the honest answer.
    pub installable: bool,
    /// True when the app can clear this itself by downloading the chosen pack into the weights
    /// directory, which is only the case when that pack has never been fetched.
    pub downloadable: bool,
}

/// The provider the card shows when Remote is the selected source.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteView {
    pub base_url: String,
    pub model: String,
    pub context_tokens: u32,
    /// Whether the user's consent switch is on. Off means a remote turn is refused, not rerouted.
    pub allowed: bool,
    /// Whether the address and model are both present. A turn also needs consent and a saved key.
    pub configured: bool,
}

/// The free model the card shows when the picker's free tier is the selected source.
///
/// No secret is here, and no list of providers is: the registry is the host's, and a row is answered
/// by the local service or not at all — which the page reads from [`OpenCodeView`], not from here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeView {
    pub id: String,
    pub label: String,
    pub provider: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeView {
    /// Whether the OpenCode app's local service answers on this machine.
    pub available: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusView {
    pub catalog_id: String,
    pub display_name: String,
    pub repo_id: String,
    /// The backend the run uses. Normally what the probe found; different when `warnings` carries a
    /// refusal that swapped it, which is then what the row is honestly reporting.
    pub backend: String,
    pub engine_tag: String,
    pub engine_installed: bool,
    pub weights_present: bool,
    pub local: bool,
    /// The provider this turn goes to instead of the local engine, when Remote is the selected
    /// source. No secret is here: the Settings page asks the Keychain separately whether a key is
    /// saved, and this status poll never reads one.
    pub remote: Option<RemoteView>,
    /// The free model this turn goes to instead of the local engine, when the picker's free tier is
    /// the selected source. Same no-secret rule as `remote`.
    pub free: Option<FreeView>,
    /// The OpenCode app on this Mac, when it is running. It is the only thing that can answer a free
    /// pick, so the card has to be able to say whether the route the user is about to take exists.
    #[serde(default)]
    pub opencode: OpenCodeView,
    pub packing: String,
    pub size_gb: f32,
    pub warnings: Vec<String>,
    /// Caveats whose condition this build could not evaluate, so they may or may not apply here.
    pub unchecked_warnings: Vec<String>,
    pub packs: Vec<PackView>,
    pub blocked: Option<BlockedView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecode_models::hardware::{Backend, GpuArch};
    use freecode_models::Catalog;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    /// A checkout carrying the pinned configuration and nothing fetched: no engine, no weights.
    /// Built in a tempdir because the machine running these tests may already have installed the
    /// engine, and a "clean checkout" that reads the repository's own `engines/` would pass or fail
    /// according to what the last run downloaded.
    fn clean_checkout() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let real = repo_root();
        for rel in ["engines/lock.toml", "catalog/models.toml"] {
            let dest = tmp.path().join(rel);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(real.join(rel), &dest).unwrap_or_else(|e| panic!("{rel}: {e}"));
        }
        tmp
    }

    #[test]
    fn a_clean_checkout_reports_the_missing_engine_as_the_first_blocker() {
        // Nothing unpacked, nothing downloaded: the refusal shown first must be the one carrying an
        // install action, and it must point at the fork, not a system llama.cpp.
        let root = clean_checkout();
        let status = build(root.path(), &Settings::default()).expect("status builds");
        let blocked = status.blocked.expect("a fresh tree blocks the run");
        assert_eq!(
            blocked.info.code,
            load_guard::BlockerCode::EngineNotInstalled
        );
        assert!(
            blocked.installable,
            "the first blocker on a fresh tree is the one Install fixes"
        );
        assert!(
            blocked.info.cta_url.contains("PrismML-Eng"),
            "{}",
            blocked.info.cta_url
        );
        assert!(!status.local);
        assert!(!status.engine_installed);
        assert!(!status.weights_present);
    }

    #[test]
    fn a_fresh_tree_is_pending_install_not_a_contract_failure() {
        let root = clean_checkout();
        let assembly = assemble(root.path(), &Settings::default()).unwrap();
        assert!(
            assembly.contract_failures().is_empty(),
            "{:?}",
            assembly.contract_failures()
        );
        assert!(!assembly.report.is_allowed());
    }

    #[test]
    fn a_pack_that_was_never_fetched_offers_the_in_app_download() {
        let root = clean_checkout();
        let mut assembly = assemble(root.path(), &Settings::default()).unwrap();
        // The state a machine reaches once the engine has been installed and the pack has not: the
        // one blocker the view must hand to the Download button.
        assembly.report.blockers = vec![Blocker::IntegrityFailed {
            file: assembly.pack.file.clone(),
            status: "not downloaded".into(),
        }];
        let blocked = assembly.view(&Settings::default()).blocked.unwrap();
        assert!(blocked.downloadable);
        assert!(
            !blocked.installable,
            "the engine is on disk in this state, so Install is not the fix"
        );

        // The same refusal for a file that IS on disk with the wrong bytes is not a fetch the app
        // gets to decide about: clearing it would overwrite something the user put there, so the URL
        // stays the answer.
        assembly.weights_absent = false;
        let blocked = assembly.view(&Settings::default()).blocked.unwrap();
        assert!(!blocked.downloadable);
    }

    #[test]
    fn caveats_the_host_cannot_evaluate_stay_out_of_the_matched_list() {
        // The shipped catalog carries caveats about other operating systems and GPU models. They
        // must reach the UI folded, never as a warning that reads like it is about this machine.
        let status = build(&repo_root(), &Settings::default()).expect("status builds");
        assert!(
            status
                .warnings
                .iter()
                .all(|w| !w.contains("condition: backend ==")),
            "a cross-platform caveat leaked into the matched warnings: {:?}",
            status.warnings
        );
        assert!(
            !status.unchecked_warnings.is_empty(),
            "the catalog's unverifiable caveats must still be surfaced somewhere"
        );
    }

    /// A backend the app refused is a fact about the run, not a detail of the probe: the card's
    /// backend row now names something other than what the machine installed. Assembled on this
    /// machine and then re-stamped as an RDNA2 host, because no Apple hardware produces that state.
    #[test]
    fn a_refused_backend_reaches_the_card_as_a_warning() {
        let root = clean_checkout();
        let mut assembly = assemble(root.path(), &Settings::default()).unwrap();
        let mut host = HostHardware {
            backend: Backend::Hip,
            arch: GpuArch::AmdRdna2,
            ..assembly.host.clone()
        };
        host.refuse_hostile_backend();
        assembly.host = host;

        let view = assembly.view(&Settings::default());
        assert_eq!(view.backend, "Vulkan");
        assert!(
            view.warnings.iter().any(|w| w.contains("#264")),
            "{:?}",
            view.warnings
        );
    }

    #[test]
    fn every_catalog_packing_is_offered_to_the_ui_with_a_reason() {
        let status = build(&repo_root(), &Settings::default()).unwrap();
        let catalog = Catalog::load_default(&repo_root()).unwrap();
        // The entry the status was built for, not whichever happens to be first: with two models in
        // the catalog, `models[0]` would silently stop describing what the view shows.
        let model = catalog.model(&status.catalog_id).unwrap();
        assert_eq!(status.packs.len(), model.packs.len());
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
        // Forced to a packing the probe would not have chosen, named off the entry the settings
        // select rather than hard-coded: the catalog has two models now, and a pack key that belongs
        // to the other one is a test that quietly stops testing anything.
        let catalog = Catalog::load_default(&repo_root()).unwrap();
        let model = catalog
            .model(&Settings::default().local_model.catalog_id)
            .expect("the default id is in the catalog");
        let auto = assemble(&repo_root(), &Settings::default()).unwrap();
        let forced = model
            .packs
            .iter()
            .find(|p| p.key != auto.pack.key)
            .expect("an entry has more than one packing");

        let mut settings = Settings::default();
        settings.local_model.packing = PackingChoice::Forced(forced.key.clone());
        let assembly = assemble(&repo_root(), &settings).unwrap();
        assert_eq!(assembly.pack.key, forced.key);
        assert_eq!(assembly.view(&settings).packing, forced.key);
    }

    #[test]
    fn the_local_badge_needs_a_permitted_run_and_the_local_source() {
        assert!(local_badge(true, false, false));
        assert!(
            !local_badge(true, true, false),
            "selecting the remote provider takes the badge off"
        );
        assert!(
            !local_badge(true, false, true),
            "selecting a free model takes the badge off too"
        );
        assert!(
            !local_badge(false, false, false),
            "a refused load is not local, it is not running"
        );
    }

    #[test]
    fn selecting_the_remote_provider_moves_the_card_off_the_local_engine() {
        let mut settings = Settings::default();
        settings.remote_provider.source = ModelSource::Remote;
        settings.remote_provider.base_url = "https://api.example.com/v1".into();
        settings.remote_provider.model = "gpt-test".into();

        let assembly = assemble(&repo_root(), &settings).unwrap();
        let view = assembly.view(&settings);
        assert!(!view.local);
        let remote = view.remote.expect("a remote view");
        assert!(remote.configured);
        assert!(
            !remote.allowed,
            "the consent switch starts off and is reported honestly"
        );
        assert_eq!(remote.model, "gpt-test");
        assert_eq!(remote.context_tokens, 32_768);

        let local = Settings::default();
        assert!(assembly.view(&local).remote.is_none());
    }
}
