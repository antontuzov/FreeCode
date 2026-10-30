//! Locating the directories the host works in.
//!
//! A checkout keeps working in the repository it runs from; an installed bundle has no such tree on
//! the user's machine and is not writable in place, so it works in the directory it owns beside
//! `settings.json`. `FREECODE_ROOT` overrides both, which is what a scratch harness points at its own
//! tree.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use tauri::{AppHandle, Manager};

pub const ROOT_ENV: &str = "FREECODE_ROOT";
/// The directory this app wrote under its previous name. Read once by `migrate_legacy_app_dir`;
/// nothing else in the build refers to it.
const LEGACY_APP_DIR: &str = "smollm-studio";

/// The directory the app owns beside `settings.json`. `None` on a platform that gives the app no such
/// directory.
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

/// The directory this host works in: the override, else the checkout it is running from, else the
/// directory the app owns.
pub fn workspace_root(app: Option<&AppHandle>) -> Result<PathBuf, String> {
    let writes = app_data_dir().or_else(|| {
        app.and_then(|app| app.path().app_data_dir().ok())
            .map(absolute)
    });
    root_from(std::env::var_os(ROOT_ENV), writes)
}

/// The checkout this binary was built from, when it still exists on this machine. A released bundle
/// is built on another machine, so that path is absent here and the app falls through to its own
/// directory.
fn checkout_root() -> PathBuf {
    absolute(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn root_from(override_dir: Option<OsString>, writes: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(dir) = override_dir {
        return Ok(absolute(PathBuf::from(dir)));
    }
    let checkout = checkout_root();
    if checkout.is_dir() {
        return Ok(checkout);
    }
    writes.ok_or_else(|| "this platform gives the app no data directory to work in".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_override_wins_over_every_other_root() {
        let dir = tempfile::tempdir().unwrap();
        let found = root_from(Some(dir.path().to_path_buf().into_os_string()), None).unwrap();
        assert_eq!(found, dir.path());
    }

    #[test]
    fn a_checkout_works_in_the_tree_it_was_built_from() {
        let root = root_from(None, None).expect("a test run is a checkout");
        assert_eq!(root, checkout_root());
        assert!(
            !root.to_string_lossy().contains(".."),
            "{} still has a hop the user cannot read",
            root.display()
        );
        assert!(root.is_dir());
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
        assert!(root.is_dir());
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
