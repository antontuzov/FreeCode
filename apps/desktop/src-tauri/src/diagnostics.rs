//! `Diagnostics`: what this machine and this build actually are, in one report.
//!
//! The status card answers "which route will answer". This answers the question a bug report asks
//! afterwards — which system, which version of the app, which directory it writes into — and none of
//! those are preferences, so the page holds no controls beyond the update check.

use std::path::Path;

use serde::Serialize;

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
    /// The app's data root: the directory holding its settings and filed conversations.
    pub root: String,
}

impl Report {
    pub fn build(root: &Path, app_version: &str) -> Self {
        Report {
            app_version: app_version.to_string(),
            system: system(),
            root: root.display().to_string(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_the_bundle_carries_is_the_version_the_page_shows() {
        let report = Report::build(Path::new("/tmp/freecode"), "1.2.3");
        assert_eq!(report.app_version, "1.2.3");
        assert_eq!(report.root, "/tmp/freecode");
    }

    #[test]
    fn the_system_row_names_this_machine_without_inventing_a_probe() {
        assert_eq!(os_label("macos"), "macOS");
        assert_eq!(os_label("windows"), "Windows");
        assert_eq!(os_label("linux"), "Linux");

        let report = Report::build(Path::new("/tmp/freecode"), "0.0.0-test");
        assert!(
            report.system.ends_with(std::env::consts::ARCH),
            "{}",
            report.system
        );
    }
}
