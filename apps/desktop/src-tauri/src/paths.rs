//! Locating the data files the host reads: `catalog/models.toml` and `engines/lock.toml`.
//!
//! An installed app carries them as bundled resources; a checkout reads them from the repo root.
//! `FREECODE_ROOT` overrides both, which is what the native-verification job points at a
//! scratch tree. When it is set it is honoured strictly: a wrong override must fail loudly rather
//! than silently reading the checkout's catalog and reporting that tree as verified.
//!
//! Reading and writing are not the same directory once the app is packaged. The engine unpack and
//! the weight download both land under the root, and the code signature covers everything inside a
//! `.app` — so a bundle that grew by 463 MB is a bundle that will not launch, and in
//! `/Applications` it is not writable at all, so the download fails halfway. There the contract
//! files are read from the bundle and mirrored into the directory the app owns, which becomes the
//! root. A checkout keeps a single root, so every path verified in development is unchanged.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use tauri::{AppHandle, Manager};

pub const ROOT_ENV: &str = "FREECODE_ROOT";
/// The directory this app wrote under its previous name. Read once by `migrate_legacy_app_dir`;
/// nothing else in the build refers to it.
const LEGACY_APP_DIR: &str = "smollm-studio";
/// The file mirroring the pids of the supervised servers, one line per launch.
const SERVER_PIDS: &str = "servers.pid";
const CONTRACT: [&str; 2] = [
    freecode_models::CATALOG_REL,
    freecode_models::ENGINE_LOCK_REL,
];

/// The directory the app owns beside `settings.json`, and the only place a packaged app may unpack
/// its engine or download weights. `None` on a platform that gives the app no such directory.
pub(crate) fn app_data_dir() -> Option<PathBuf> {
    freecode_config::Settings::default_path()
        .and_then(|settings| settings.parent().map(|dir| absolute(dir.to_path_buf())))
}

/// Move the data directory this app wrote under its previous name to the one it uses now.
///
/// Settings and the filed conversations live inside it, so without the move a renamed app would open
/// onto an empty history. It runs once, from `setup`, before any command can read the directory; a
/// directory already under the new name is never touched, and a missing old one is not a failure.
pub(crate) fn migrate_legacy_app_dir() {
    let Some(new) = app_data_dir() else { return };
    let Some(container) = new.parent() else {
        return;
    };
    let old = container.join(LEGACY_APP_DIR);
    if let Err(why) = migrate_dir(&old, &new) {
        eprintln!(
            "could not move {} to {}: {why}",
            old.display(),
            new.display()
        );
    }
}

fn migrate_dir(old: &Path, new: &Path) -> std::io::Result<()> {
    if new.exists() || !old.exists() {
        return Ok(());
    }
    std::fs::rename(old, new)
}

/// Where the server pid registry lives: beside `settings.json`, in the directory the app already
/// owns and which outlives a crash. `None` when the platform has no such directory, which leaves the
/// registry in memory only.
pub(crate) fn server_pid_registry() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(SERVER_PIDS))
}

/// The directory holding `catalog/`, `engines/` and `models/` — the one the host writes in.
pub fn workspace_root(app: Option<&AppHandle>) -> Result<PathBuf, String> {
    root_from(std::env::var_os(ROOT_ENV), candidates(app), app_data_dir())
}

/// The folder a run's file calls are confined to: the one the user picked, or the root this host
/// resolved when no folder was picked. A picked folder that has since moved is an error, not a cue to
/// fall back — a run that quietly worked in the app's own directory instead of the project would
/// report changes the user cannot find in the folder they chose.
pub fn project_root(
    app: Option<&AppHandle>,
    settings: &freecode_config::Settings,
) -> Result<PathBuf, String> {
    let chosen = settings.project.root.trim();
    if chosen.is_empty() {
        return workspace_root(app);
    }
    let path = PathBuf::from(chosen);
    // A dialog hands back an absolute path. A hand-edited one is resolved against the process's
    // working directory otherwise, which is the checkout in a dev run and `/` in a bundle.
    if !path.is_absolute() {
        return Err(format!(
            "the project folder {chosen} is not an absolute path. Choose it again from Settings."
        ));
    }
    let root = path.canonicalize().map_err(|why| {
        format!(
            "the project folder {} cannot be opened: {why}. Choose another one from Settings.",
            path.display()
        )
    })?;
    if !root.is_dir() {
        return Err(format!(
            "the project folder {} is a file, not a folder. Choose another one from Settings.",
            root.display()
        ));
    }
    Ok(root)
}

/// A place the contract files might live, and whether the app is allowed to write there.
struct Candidate {
    root: PathBuf,
    /// Inside this app's own bundle, which the signature covers.
    bundled: bool,
}

fn root_from(
    override_dir: Option<OsString>,
    found: Vec<Candidate>,
    writes: Option<PathBuf>,
) -> Result<PathBuf, String> {
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

    for candidate in &found {
        // A candidate that lacks a contract file is not the root; the search has not ended yet.
        if missing_contract_files(&candidate.root).is_err() {
            continue;
        }
        if !candidate.bundled {
            return Ok(candidate.root.clone());
        }
        let writes = writes.clone().ok_or_else(|| {
            format!(
                "{} is this app's own bundle, so unpacking its engine and weights there would break \
                 the signature, and this platform gives the app no writable directory to use instead",
                candidate.root.display()
            )
        })?;
        mirror_contract(&candidate.root, &writes).map_err(|why| {
            format!(
                "cannot use {} as the app's data root: {why}",
                writes.display()
            )
        })?;
        return Ok(writes);
    }

    Err(format!(
        "no {} + {} found in any of: {}",
        freecode_models::CATALOG_REL,
        freecode_models::ENGINE_LOCK_REL,
        found
            .iter()
            .map(|c| c.root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn candidates(app: Option<&AppHandle>) -> Vec<Candidate> {
    let mut out = Vec::new();
    if let Some(app) = app {
        if let Ok(dir) = app.path().resolve("", tauri::path::BaseDirectory::Resource) {
            // Tauri unpacks resources that lived above the crate directory under `_up_/`, so an
            // installed app keeps them in a mirror of the repo layout rather than at the root.
            out.push(Candidate {
                root: dir.join("_up_"),
                bundled: true,
            });
            out.push(Candidate {
                root: dir,
                bundled: true,
            });
        }
    }
    // `apps/desktop/src-tauri` -> repo root, valid in a dev checkout.
    out.push(Candidate {
        root: absolute(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")),
        bundled: false,
    });
    out
}

/// Copy the contract files into the directory the app writes in, which then reads as any other root
/// would. Compared by content rather than copied only when absent, because an app upgrade ships a
/// new catalog and a mirror that merely filled gaps would serve the previous one forever.
fn mirror_contract(from: &Path, to: &Path) -> Result<(), String> {
    for rel in CONTRACT {
        let bytes = std::fs::read(from.join(rel))
            .map_err(|e| format!("reading bundled {rel} failed: {e}"))?;
        let target = to.join(rel);
        if std::fs::read(&target).is_ok_and(|current| current == bytes) {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {} failed: {e}", parent.display()))?;
        }
        std::fs::write(&target, bytes)
            .map_err(|e| format!("writing {} failed: {e}", target.display()))?;
    }
    Ok(())
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
    for rel in CONTRACT {
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
            std::fs::write(&path, "x").unwrap();
        }
    }

    fn bundled(root: &Path) -> Candidate {
        Candidate {
            root: root.to_path_buf(),
            bundled: true,
        }
    }

    fn checkout(root: &Path) -> Candidate {
        Candidate {
            root: root.to_path_buf(),
            bundled: false,
        }
    }

    fn override_to(dir: &Path) -> Option<OsString> {
        Some(dir.to_path_buf().into_os_string())
    }

    #[test]
    fn an_explicit_override_wins_over_the_checkout() {
        let dir = tempfile::tempdir().unwrap();
        seeded(
            dir.path(),
            &[
                freecode_models::CATALOG_REL,
                freecode_models::ENGINE_LOCK_REL,
            ],
        );
        let found = root_from(override_to(dir.path()), vec![], None).unwrap();
        assert_eq!(found, dir.path());
    }

    #[test]
    fn a_partial_override_is_refused_instead_of_falling_back() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path(), &[freecode_models::CATALOG_REL]);
        let err = root_from(override_to(dir.path()), vec![], None).unwrap_err();
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
        assert!(root.join(freecode_models::CATALOG_REL).is_file());
    }

    #[test]
    fn a_checkout_resolves_without_any_override() {
        let root = root_from(None, candidates(None), None)
            .expect("the repo root holds the contract files");
        assert!(root.join(freecode_models::CATALOG_REL).is_file());
        assert!(root.join(freecode_models::ENGINE_LOCK_REL).is_file());
    }

    #[test]
    fn a_picked_folder_becomes_the_project_root() {
        let chosen = tempfile::tempdir().unwrap();
        let mut settings = freecode_config::Settings::default();
        settings.project.root = chosen.path().display().to_string();
        let root = project_root(None, &settings).unwrap();
        assert_eq!(root, chosen.path().canonicalize().unwrap());
    }

    #[test]
    fn no_pick_keeps_the_root_the_host_resolved() {
        let root = project_root(None, &freecode_config::Settings::default())
            .expect("the checkout resolves without a pick");
        assert!(root.join(freecode_models::CATALOG_REL).is_file());
    }

    /// A chosen folder that has moved must not quietly send the run to the app's own directory: the
    /// sentence names the folder and the surface that can replace it.
    #[test]
    fn a_vanished_pick_is_refused_instead_of_falling_back() {
        let chosen = tempfile::tempdir().unwrap();
        let path = chosen.path().join("gone");
        let mut settings = freecode_config::Settings::default();
        settings.project.root = path.display().to_string();
        let why = project_root(None, &settings).unwrap_err();
        assert!(why.contains(&path.display().to_string()), "{why}");
        assert!(why.contains("Settings"), "{why}");
    }

    #[test]
    fn a_relative_pick_is_refused_because_a_bundle_starts_elsewhere() {
        let mut settings = freecode_config::Settings::default();
        settings.project.root = "my-project".into();
        let why = project_root(None, &settings).unwrap_err();
        assert!(why.contains("absolute"), "{why}");
    }

    #[test]
    fn a_file_is_not_a_folder_the_run_can_work_in() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut settings = freecode_config::Settings::default();
        settings.project.root = file.path().display().to_string();
        let why = project_root(None, &settings).unwrap_err();
        assert!(why.contains("not a folder"), "{why}");
    }

    #[test]
    fn a_writable_root_is_used_as_is_and_nothing_is_mirrored() {
        let tree = tempfile::tempdir().unwrap();
        seeded(tree.path(), &CONTRACT);
        let data = tempfile::tempdir().unwrap();
        let root = root_from(
            None,
            vec![checkout(tree.path())],
            Some(data.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(root, tree.path());
        // A checkout writes where it reads, so the app's data dir stays untouched.
        assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0);
    }

    #[test]
    fn an_installed_app_reads_its_bundle_but_writes_in_its_data_dir() {
        let bundle = tempfile::tempdir().unwrap();
        seeded(bundle.path(), &CONTRACT);
        let data = tempfile::tempdir().unwrap();
        let root = root_from(
            None,
            vec![bundled(bundle.path())],
            Some(data.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(root, data.path());
        for rel in CONTRACT {
            let copied = std::fs::read(root.join(rel)).unwrap();
            assert_eq!(
                copied,
                std::fs::read(bundle.path().join(rel)).unwrap(),
                "{rel}"
            );
        }
        // The bundle still holds exactly the two files it arrived with: an engine unpacked or a
        // weight downloaded there would invalidate the signature that lets the app launch.
        assert_eq!(std::fs::read_dir(bundle.path()).unwrap().count(), 2);
        assert!(!bundle.path().join("models").exists());
    }

    #[test]
    fn a_newer_bundled_catalog_replaces_the_mirror_instead_of_being_ignored() {
        let bundle = tempfile::tempdir().unwrap();
        seeded(bundle.path(), &CONTRACT);
        let data = tempfile::tempdir().unwrap();
        // A previous version's mirror, already in place.
        std::fs::create_dir_all(data.path().join("catalog")).unwrap();
        std::fs::write(data.path().join(freecode_models::CATALOG_REL), "version 1").unwrap();

        let upgrade = data.path().join(freecode_models::CATALOG_REL);
        std::fs::write(
            bundle.path().join(freecode_models::CATALOG_REL),
            "version 2",
        )
        .unwrap();
        root_from(
            None,
            vec![bundled(bundle.path())],
            Some(data.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&upgrade).unwrap(), "version 2");
    }

    #[test]
    fn a_bundle_with_nowhere_to_write_is_refused_rather_than_half_installing() {
        let bundle = tempfile::tempdir().unwrap();
        seeded(bundle.path(), &CONTRACT);
        let err = root_from(None, vec![bundled(bundle.path())], None).unwrap_err();
        assert!(err.contains("signature"), "{err}");
    }

    #[test]
    fn a_tree_without_contract_files_is_still_reported_by_every_place_searched() {
        let empty = tempfile::tempdir().unwrap();
        let err = root_from(None, vec![checkout(empty.path())], None).unwrap_err();
        assert!(err.contains(freecode_models::CATALOG_REL), "{err}");
        assert!(err.contains(&empty.path().display().to_string()), "{err}");
    }

    /// The rename must not look like a first launch: the settings and the conversations written
    /// under the previous name arrive with the directory.
    #[test]
    fn the_previous_names_directory_moves_with_its_contents() {
        let container = tempfile::tempdir().unwrap();
        let old = container.path().join("smollm-studio");
        let new = container.path().join("freecode");
        std::fs::create_dir_all(old.join("sessions")).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();

        migrate_dir(&old, &new).unwrap();

        assert!(!old.exists());
        assert!(new.join("settings.json").is_file());
        assert!(new.join("sessions").is_dir());
    }

    /// A directory already under the new name is the truth; the old one is left where it is rather
    /// than merged into it behind the user's back.
    #[test]
    fn a_new_directory_is_never_replaced_by_the_old_one() {
        let container = tempfile::tempdir().unwrap();
        let old = container.path().join("smollm-studio");
        let new = container.path().join("freecode");
        std::fs::create_dir_all(old.join("sessions")).unwrap();
        std::fs::create_dir_all(&new).unwrap();

        migrate_dir(&old, &new).unwrap();

        assert!(old.join("sessions").is_dir());
        assert!(new.is_dir());
    }

    #[test]
    fn a_machine_that_never_ran_the_old_name_has_nothing_to_move() {
        let container = tempfile::tempdir().unwrap();
        let old = container.path().join("smollm-studio");
        let new = container.path().join("freecode");

        migrate_dir(&old, &new).unwrap();

        assert!(
            !new.exists(),
            "a migration must not invent a data directory"
        );
    }
}
