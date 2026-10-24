//! Handing the user the file a run just wrote, from the panel that names it.
//!
//! The transcript already opens web addresses, and that path is scoped to `http` and `https` on
//! purpose: widening the opener capability to `file://` would give every link a model can print —
//! including one inside text the model did not write — a way to ask the desktop to launch a local
//! file. So this is a host command instead, and the only thing it accepts is a path a run of the loop
//! already named, which the review panel is showing.
//!
//! It opens rather than reveals, because the point is the page. A run that was asked for a web app has
//! handed the user an `index.html`, and "selected in a file manager" is not the same as seen.

use std::path::{Path, PathBuf};

use smollm_tools::{Access, SandboxPolicy};
use tauri::AppHandle;

/// The workspace's own policy, built here rather than borrowed from the agent's.
///
/// The root is the project folder the run was confined to, so a file the panel names is one this
/// command can actually reach. Launching a file is not a read though: the desktop hands it to
/// whatever program claims that kind, so this command keeps the narrower rule and confines the path
/// to that folder whatever the rest of the app is configured to allow. Shared with `preview`, whose
/// scheme serves the same folder — one sandbox for both doors into the workspace.
pub(crate) fn policy(app: &AppHandle) -> Result<SandboxPolicy, String> {
    let settings = crate::appearance::load_settings()?;
    Ok(SandboxPolicy {
        workspace_root: crate::paths::project_root(Some(app), &settings)?,
        ..SandboxPolicy::default()
    })
}

/// The file behind a path the run named, or the reason there is no such file to open.
pub(crate) fn confined(policy: &SandboxPolicy, raw: &str) -> Result<PathBuf, String> {
    let path = policy
        .resolve_path(Path::new(raw), Access::Read)
        .map_err(|why| why.to_string())?;
    // A change the panel describes can be undone between the run and the press, and a directory or a
    // dangling name is something the desktop would answer with an error window of its own.
    if !path.is_file() {
        return Err(format!(
            "{} is not a file that is there any more, so there is nothing to open.",
            path.display()
        ));
    }
    Ok(path)
}

/// The desktop's own "give this to its default handler" program.
fn opener(path: &Path) -> std::process::Command {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    let mut command = std::process::Command::new(program);
    command.arg(path);
    command
}

/// Hand the file over, and report only what this side can honestly see.
fn launch(path: &Path) -> Result<(), String> {
    let shown = path.display().to_string();
    let status = opener(path)
        .status()
        .map_err(|why| format!("nothing was asked of the desktop for {shown}: {why}"))?;
    // `explorer` exits nonzero even when it opened the file, so on that platform the status answers
    // no question. Everywhere else a refusal is a refusal.
    if !status.success() && !cfg!(windows) {
        return Err(format!("the desktop would not open {shown}."));
    }
    Ok(())
}

/// Open one of the files this run changed with the desktop's default handler.
///
/// Answers with the reason nothing opened, because a control that pressed and did nothing is the
/// same lie as a status card claiming a server is running.
#[tauri::command]
pub async fn open_file(app: AppHandle, path: String) -> Result<(), String> {
    let policy = policy(&app)?;
    let file = confined(&policy, &path)?;
    launch(&file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace with one file in it and one outside, which is the whole difference this module is
    /// about: the run's paths are relative, and the argument arrives from a webview.
    fn fixture() -> (tempfile::TempDir, SandboxPolicy) {
        let root = tempfile::tempdir().unwrap();
        let inside = root.path().join("my-app");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join("index.html"), b"<html></html>").unwrap();
        std::fs::write(root.path().join("elsewhere.txt"), b"outside the task").unwrap();
        let policy = SandboxPolicy {
            workspace_root: root.path().to_path_buf(),
            ..SandboxPolicy::default()
        };
        (root, policy)
    }

    #[test]
    fn a_file_the_run_named_resolves_to_the_one_inside_the_workspace() {
        let (root, policy) = fixture();
        let got = confined(&policy, "my-app/index.html").unwrap();
        assert_eq!(
            got,
            std::fs::canonicalize(root.path().join("my-app/index.html")).unwrap()
        );
    }

    /// The argument is a string the webview chose. `..` is how it would leave, and the desktop is
    /// where leaving matters: an open can run whatever the OS thinks a `.command` is.
    #[test]
    fn a_path_that_walks_out_of_the_workspace_is_refused() {
        let (_root, policy) = fixture();
        let why = confined(&policy, "../elsewhere.txt").unwrap_err();
        assert!(
            why.contains("outside") || why.contains("workspace"),
            "the refusal should say why: {why}"
        );
        assert!(
            !Path::new(&why).exists(),
            "a refusal does not hand over a path"
        );
    }

    #[test]
    fn an_absolute_path_outside_is_refused_too() {
        let (_root, policy) = fixture();
        let why = confined(&policy, "/etc/passwd").unwrap_err();
        assert!(why.contains("outside") || why.contains("denied"), "{why}");
    }

    /// A secret the loop may never read is not opened by the panel either, even though the user could
    /// reach it in a file manager: the command inherits the sandbox's list rather than its own.
    #[test]
    fn a_protected_name_stays_protected_when_it_is_only_opened() {
        let (root, policy) = fixture();
        std::fs::write(root.path().join(".env"), b"TOKEN=1").unwrap();
        let why = confined(&policy, ".env").unwrap_err();
        assert!(why.contains("protected") || why.contains("denied"), "{why}");
    }

    #[test]
    fn a_name_that_is_no_longer_a_file_is_refused_by_name() {
        let (root, policy) = fixture();
        std::fs::remove_file(root.path().join("my-app/index.html")).unwrap();
        let why = confined(&policy, "my-app/index.html").unwrap_err();
        assert!(why.contains("my-app/index.html"), "{why}");
        assert!(why.contains("nothing to open"), "{why}");
    }

    /// A directory is not a file, and `open` on one would show the user a Finder window they did not
    /// ask for while the panel claimed the file had opened.
    #[test]
    fn a_directory_is_not_a_file_to_open() {
        let (_root, policy) = fixture();
        assert!(confined(&policy, "my-app").is_err());
    }

    #[test]
    fn the_opener_is_handed_one_path_and_never_a_shell() {
        let path = Path::new("/somewhere/my-app/index.html");
        let command = opener(path);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, vec![std::ffi::OsStr::new(path)]);
        #[cfg(target_os = "macos")]
        assert_eq!(command.get_program(), "open");
    }
}
