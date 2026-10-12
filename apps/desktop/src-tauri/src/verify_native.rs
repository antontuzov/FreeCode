//! Headless verification of the native plumbing, so CI can test the engine/catalog/guard wiring
//! without a window server.
//!
//! This is the same `status::Assembly` the React shell reads over IPC — that identity is the whole
//! point. A pass means: the contract files parse, the pin satisfies the catalog, the guard raises
//! nothing beyond a pending install, and settings round-trip. `SMOLLM_REQUIRE_ENGINE=1` additionally
//! fails while the fork binary is still missing, which is how Phase 2 turns this from "the wiring is
//! sound" into "the model actually runs".

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use smollm_config::Settings;
use smollm_models::{Catalog, EngineLock};

use crate::paths;
use crate::status::{self, Assembly};

pub const REQUIRE_ENGINE_ENV: &str = "SMOLLM_REQUIRE_ENGINE";

#[derive(Debug, Default)]
pub struct Probe {
    pub root: PathBuf,
    /// What was established, in order. Printed whichever way the run goes.
    pub notes: Vec<String>,
    pub failures: Vec<String>,
}

impl Probe {
    pub fn ok(&self) -> bool {
        self.failures.is_empty()
    }

    fn say(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    fn fail(&mut self, failure: impl Into<String>) {
        self.failures.push(failure.into());
    }
}

pub fn require_engine_from_env() -> bool {
    std::env::var(REQUIRE_ENGINE_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn probe(root: &Path, require_engine: bool) -> Probe {
    let mut probe = Probe {
        root: root.to_path_buf(),
        ..Probe::default()
    };

    let catalog = match Catalog::load_default(root) {
        Ok(catalog) => catalog,
        Err(e) => {
            probe.fail(e.to_string());
            return probe;
        }
    };
    let lock = match EngineLock::load_default(root) {
        Ok(lock) => lock,
        Err(e) => {
            probe.fail(e.to_string());
            return probe;
        }
    };
    probe.say(format!(
        "engine pin {} from {}, floor {}",
        lock.pinned_tag, lock.repo, lock.min_allowed_tag
    ));

    for model in &catalog.models {
        if let Err(why) = lock.satisfies(model) {
            probe.fail(format!("model {}: {why}", model.id));
        }
    }

    let settings = Settings::default();
    match status::assemble(root, &settings) {
        Err(e) => probe.fail(e),
        Ok(assembly) => describe(&mut probe, &assembly, root, require_engine),
    }

    probe_settings(&mut probe);
    probe
}

fn describe(probe: &mut Probe, assembly: &Assembly, root: &Path, require_engine: bool) {
    probe.say(format!(
        "backend {}, packing {} at {:.2} bpw / {:.2} GB on disk",
        assembly.host.backend.label(),
        assembly.pack.key,
        assembly.pack.bits_per_weight,
        assembly.pack.size_gb
    ));

    for failure in assembly.contract_failures() {
        probe.fail(failure);
    }

    if !assembly.engine_installed() {
        let where_it_belongs = smollm_llama::engines_root(root).join(&assembly.lock.pinned_tag);
        let message = format!(
            "fork engine {} is not unpacked under {}",
            assembly.lock.pinned_tag,
            where_it_belongs.display()
        );
        if require_engine {
            probe.fail(message);
        } else {
            probe.say(format!("{message} — pending install"));
        }
    }
    if assembly.weights_absent {
        probe.say(format!(
            "{} is not downloaded — pending install",
            assembly.file.display()
        ));
    }
    for warning in &assembly.report.warnings {
        probe.say(format!("warning: {}", warning.message));
    }
}

/// Write and read a settings document, without touching the user's real one, then check that the
/// stored document — if there is one — actually parses.
fn probe_settings(probe: &mut Probe) {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "smollm-studio-verify-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let file = dir.join("settings.json");
    let mut want = Settings::default();
    want.appearance.ui_font_size_px = 15;

    let round_trip = (|| -> Result<(), String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        want.save(&file).map_err(|e| e.to_string())?;
        let back = Settings::load(&file).map_err(|e| e.to_string())?;
        if back != want {
            return Err(
                "did not round-trip: the loaded document differs from the one written".into(),
            );
        }
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
    })();
    match round_trip {
        Ok(()) => probe.say("settings round-trip through disk"),
        Err(e) => probe.fail(format!("settings round-trip: {e}")),
    }

    match Settings::default_path() {
        Some(path) => {
            if let Err(e) = Settings::load(&path) {
                probe.fail(format!("stored settings at {}: {e}", path.display()));
            }
        }
        None => probe.fail("no writable config directory on this platform"),
    }
}

pub fn run() -> i32 {
    let root = match paths::workspace_root(None) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("verify-native: {e}");
            return 1;
        }
    };
    let probe = probe(&root, require_engine_from_env());

    println!("verify-native root {}", probe.root.display());
    for note in &probe.notes {
        println!("  {note}");
    }
    for failure in &probe.failures {
        eprintln!("  FAIL {failure}");
    }
    if probe.ok() {
        println!("verify-native: OK");
        0
    } else {
        eprintln!("verify-native: {} failure(s)", probe.failures.len());
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Replace `pinned_tag` in the scratch lock file with a build below the catalog's floor.
    fn demote_pin(root: &Path) {
        let path = root.join(smollm_models::ENGINE_LOCK_REL);
        let raw = std::fs::read_to_string(&path).unwrap();
        let patched = raw
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("pinned_tag") {
                    "pinned_tag = \"prism-b10600-0000000\""
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(patched.as_bytes()).unwrap();
    }

    /// A root holding only the two contract files: no engine, no weights.
    fn scratch_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        for rel in [smollm_models::CATALOG_REL, smollm_models::ENGINE_LOCK_REL] {
            let dest = dir.path().join(rel);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(source.join(rel), &dest).unwrap();
        }
        dir
    }

    #[test]
    fn a_pending_install_passes_until_the_engine_is_required() {
        let dir = scratch_root();
        let root = dir.path();
        let lenient = probe(root, false);
        assert!(lenient.ok(), "{:?}", lenient.failures);
        assert!(
            lenient
                .notes
                .iter()
                .any(|n| n.contains("engine pin") && n.contains("prism-b")),
            "{:?}",
            lenient.notes
        );
        assert!(
            lenient.notes.iter().any(|n| n.ends_with("pending install")),
            "both the engine and the weights should be reported as pending: {:?}",
            lenient.notes
        );

        let strict = probe(root, true);
        assert!(!strict.ok());
        assert!(
            strict.failures.iter().any(|f| f.contains("not unpacked")),
            "{:?}",
            strict.failures
        );
    }

    #[test]
    fn the_repo_root_itself_is_not_a_contract_failure() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let lenient = probe(&root, false);
        assert!(lenient.ok(), "{:?}", lenient.failures);
    }

    #[test]
    fn a_pin_the_catalog_cannot_run_on_fails_the_probe() {
        let dir = scratch_root();
        let root = dir.path();
        demote_pin(root);
        let probe = probe(root, false);
        assert!(!probe.ok());
        assert!(
            probe
                .failures
                .iter()
                .any(|f| f.contains("prism-b10600") && f.contains("older than")),
            "{:?}",
            probe.failures
        );
    }

    #[test]
    fn a_broken_catalog_fails_immediately_with_its_own_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("catalog")).unwrap();
        std::fs::write(
            dir.path().join("catalog/models.toml"),
            b"[[models]] = broken",
        )
        .unwrap();
        let probe = probe(dir.path(), false);
        assert!(!probe.ok());
        assert!(
            probe.failures[0].contains("catalog"),
            "{}",
            probe.failures[0]
        );
    }
}
