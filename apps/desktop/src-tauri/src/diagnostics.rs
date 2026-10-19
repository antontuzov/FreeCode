//! `Diagnostics`: what this machine and this build actually are, in one report.
//!
//! The status card answers "can I run it now". This answers the question a bug report asks
//! afterwards — which chip, how much memory, which pin, which version of the app — and none of those
//! are preferences, so the page holds no controls. The figures come from the same `Assembly` the
//! status card reads rather than a second probe, because two probes of one machine is how a page ends
//! up contradicting the card above it.

use std::path::Path;

use serde::Serialize;

use smollm_config::Settings;

use crate::status;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The version the running bundle carries. In a dev run this is the Cargo version, which is the
    /// honest answer to "which build is this" either way.
    pub app_version: String,
    /// Operating system and CPU architecture, as the compiler's own target states them. Not the OS
    /// release: this build has no cross-platform probe for that, and a row that reads empty on
    /// Windows would be worse than no row.
    pub system: String,
    /// The inference backend the engine was resolved for — Metal, CUDA, CPU only.
    pub backend: String,
    /// What the GPU probe named, when it reached one.
    pub chip: Option<String>,
    /// The memory the packing recommendation was sized against. On Apple Silicon this is also the
    /// GPU's memory, which is why there is no separate figure to show.
    pub total_ram_bytes: u64,
    pub gpu_ram_bytes: Option<u64>,
    /// The vector instruction set the x86 probe settled on. `None` off x86, where the question does
    /// not apply — the pack recommender only asks it of parts that have those names.
    pub simd: Option<String>,
    /// The app's data root: the directory holding `engines/` and, unless the Model page redirects
    /// it, `models/`.
    pub root: String,
    pub engine_tag: String,
    pub engine_installed: bool,
    pub model: String,
    pub repo_id: String,
    pub packing: String,
    pub size_gb: f32,
    /// The notices the guard matched against this machine, in full. The status card folds the
    /// unchecked ones behind a disclosure; a page whose whole job is reporting has nothing to fold.
    pub warnings: Vec<String>,
    pub unchecked_warnings: Vec<String>,
    /// The refusal that currently stops a run, with the technical line the card keeps collapsed.
    pub blocker: Option<status::BlockedView>,
}

impl Report {
    /// Built from a full assembly, so the packing, pin and notices here are the ones the run itself
    /// would be governed by.
    pub fn build(root: &Path, settings: &Settings, app_version: &str) -> Result<Self, String> {
        let assembly = status::assemble(root, settings)?;
        let view = assembly.view(settings);
        let host = &assembly.host;
        Ok(Report {
            app_version: app_version.to_string(),
            system: system(),
            backend: host.backend.label().to_string(),
            chip: host.gpu_name.clone(),
            total_ram_bytes: host.total_ram_bytes,
            gpu_ram_bytes: host.gpu_ram_bytes,
            simd: simd(host.avx512),
            root: root.display().to_string(),
            engine_tag: assembly.lock.pinned_tag.clone(),
            engine_installed: assembly.engine_installed(),
            model: view.display_name,
            repo_id: view.repo_id,
            packing: view.packing,
            size_gb: view.size_gb,
            warnings: view.warnings,
            unchecked_warnings: view.unchecked_warnings,
            blocker: view.blocked,
        })
    }
}

/// The spelling a user reads for the OS the target const names.
fn os_label(os: &str) -> &str {
    match os {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn system() -> String {
    format!(
        "{} · {}",
        os_label(std::env::consts::OS),
        std::env::consts::ARCH
    )
}

/// `AVX-512` changes which pack the recommender picks on an x86 host, so it is a fact worth stating
/// there and noise anywhere else.
fn simd(avx512: bool) -> Option<String> {
    let x86 = matches!(std::env::consts::ARCH, "x86_64" | "x86");
    x86.then(|| {
        if avx512 {
            "AVX-512".to_string()
        } else {
            "AVX2".to_string()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    /// A checkout with the pins and nothing fetched: the state a fresh install reports from, and the
    /// one that carries a blocker.
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
    fn the_version_the_bundle_carries_is_the_version_the_page_shows() {
        let root = clean_checkout();
        let report = Report::build(root.path(), &Settings::default(), "1.2.3").unwrap();
        assert_eq!(report.app_version, "1.2.3");
    }

    #[test]
    fn the_system_row_names_this_machine_without_inventing_a_probe() {
        assert_eq!(os_label("macos"), "macOS");
        assert_eq!(os_label("windows"), "Windows");
        assert_eq!(os_label("linux"), "Linux");

        let report =
            Report::build(clean_checkout().path(), &Settings::default(), "0.0.0-test").unwrap();
        assert!(
            report.system.ends_with(std::env::consts::ARCH),
            "{}",
            report.system
        );
    }

    #[test]
    fn the_vector_row_appears_only_where_the_question_is_asked() {
        let report =
            Report::build(clean_checkout().path(), &Settings::default(), "0.0.0-test").unwrap();
        let x86 = matches!(std::env::consts::ARCH, "x86_64" | "x86");
        assert_eq!(report.simd.is_some(), x86, "{:?}", report.simd);
    }

    #[test]
    fn a_fresh_tree_reports_the_refusal_that_stops_the_run() {
        let root = clean_checkout();
        let report = Report::build(root.path(), &Settings::default(), "0.0.0-test").unwrap();
        let blocker = report.blocker.expect("nothing is fetched yet");
        assert_eq!(
            blocker.info.code,
            smollm_llama::load_guard::BlockerCode::EngineNotInstalled
        );
        assert!(!report.engine_installed);
        assert_eq!(report.root, root.path().display().to_string());
    }

    #[test]
    fn the_packing_and_pin_are_the_ones_the_run_is_governed_by() {
        // The real checkout, so the pins and the catalog are the shipped ones.
        let report = Report::build(&repo_root(), &Settings::default(), "0.0.0-test").unwrap();
        let status = crate::status::build(&repo_root(), &Settings::default()).unwrap();
        assert_eq!(report.packing, status.packing);
        assert_eq!(report.engine_tag, status.engine_tag);
        assert_eq!(report.model, status.display_name);
        assert_eq!(report.warnings, status.warnings);
        assert_eq!(report.unchecked_warnings, status.unchecked_warnings);
        assert!(
            report.total_ram_bytes > 0,
            "the memory probe answered nothing"
        );
    }

    #[test]
    fn a_catalog_that_does_not_know_the_setting_is_an_error_the_page_shows() {
        let mut settings = Settings::default();
        settings.local_model.catalog_id = "not-in-the-catalog".into();
        let err = Report::build(&repo_root(), &settings, "0.0.0-test").unwrap_err();
        assert!(err.contains("not-in-the-catalog"), "{err}");
    }
}
