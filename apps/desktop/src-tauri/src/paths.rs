//! Locating the data files the host reads: `catalog/models.toml` and `engines/lock.toml`.
//!
//! An installed app carries them as bundled resources; a checkout reads them from the repo root.
//! `SMOLLM_STUDIO_ROOT` overrides both, which is what the native-verification job points at a
//! scratch tree. When it is set it is honoured strictly: a wrong override must fail loudly rather
//! than silently reading the checkout's catalog and reporting that tree as verified.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use tauri::{AppHandle, Manager};

pub const ROOT_ENV: &str = "SMOLLM_STUDIO_ROOT";

/// The directory holding `catalog/` and `engines/`.
pub fn workspace_root(app: Option<&AppHandle>) -> Result<PathBuf, String> {
    root_from(std::env::var_os(ROOT_ENV), app)
}

fn root_from(override_dir: Option<OsString>, app: Option<&AppHandle>) -> Result<PathBuf, String> {
    if let Some(dir) = override_dir {
        let root = absolute(PathBuf::from(dir));
        return match missing_contract_files(&root) {
            Ok(()) => Ok(root),
            Err(why) => Err(format!(
                "{ROOT_ENV}={} is not a usable root: {why}",
                root.display()
            )),
        };
    }

    let candidates = candidates(app);
    candidates
        .iter()
        .find(|root| missing_contract_files(root).is_ok())
        .cloned()
        .ok_or_else(|| {
            format!(
                "no {} + {} found in any of: {}",
                smollm_models::CATALOG_REL,
                smollm_models::ENGINE_LOCK_REL,
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn candidates(app: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(app) = app {
        if let Ok(dir) = app.path().resolve("", tauri::path::BaseDirectory::Resource) {
            // Tauri unpacks resources that lived above the crate directory under `_up_/`, so an
            // installed app keeps them in a mirror of the repo layout rather than at the root.
            out.push(dir.join("_up_"));
            out.push(dir);
        }
    }
    // `apps/desktop/src-tauri` -> repo root, valid in a dev checkout.
    out.push(absolute(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."),
    ));
    out
}

/// `..` in a path the user is being asked to inspect is noise they cannot verify, and it hides the
/// fact that two different roots were found. `std::path::absolute` only prepends the cwd, so the
/// `..` components are collapsed here — lexically, without resolving symlinks.
fn absolute(path: PathBuf) -> PathBuf {
    let joined = std::path::absolute(&path).unwrap_or(path);
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && out.as_os_str().is_empty() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn missing_contract_files(root: &Path) -> Result<(), String> {
    let mut missing = Vec::new();
    for rel in [smollm_models::CATALOG_REL, smollm_models::ENGINE_LOCK_REL] {
        if !root.join(rel).is_file() {
            missing.push(rel);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing {}", missing.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(dir: &Path, files: &[&str]) {
        for rel in files {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
    }

    #[test]
    fn an_explicit_override_wins_over_the_checkout() {
        let dir = tempfile::tempdir().unwrap();
        seeded(
            dir.path(),
            &[smollm_models::CATALOG_REL, smollm_models::ENGINE_LOCK_REL],
        );
        let found = root_from(Some(dir.path().to_path_buf().into_os_string()), None).unwrap();
        assert_eq!(found, dir.path());
    }

    #[test]
    fn a_partial_override_is_refused_instead_of_falling_back() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path(), &[smollm_models::CATALOG_REL]);
        let err = root_from(Some(dir.path().to_path_buf().into_os_string()), None).unwrap_err();
        assert!(err.contains("engines/lock.toml"), "{err}");
    }

    #[test]
    fn the_checkout_root_is_shown_without_dot_dot_components() {
        let root = absolute(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."));
        assert!(
            !root.to_string_lossy().contains(".."),
            "{} still has a hop the user cannot read",
            root.display()
        );
        assert!(root.join(smollm_models::CATALOG_REL).is_file());
    }

    #[test]
    fn a_checkout_resolves_without_any_override() {
        let root = root_from(None, None).expect("the repo root holds the contract files");
        assert!(root.join(smollm_models::CATALOG_REL).is_file());
        assert!(root.join(smollm_models::ENGINE_LOCK_REL).is_file());
    }
}
