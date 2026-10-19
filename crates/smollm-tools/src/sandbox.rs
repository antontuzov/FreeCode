//! The workspace sandbox: the only sanctioned way a tool gets a filesystem path.
//!
//! Every side effect in the agent goes through here, so the rules are short and mechanical:
//!   * a path is resolved *lexically* first (no `..`), then canonicalized as far as the filesystem
//!     allows, so a symlink cannot point out of the workspace;
//!   * writes must land inside `workspace_root`, always;
//!   * reads outside it need `read_outside_workspace`, and even then never cross a denied pattern —
//!     `.env`, key material and signing identities are off-limits to the model, matching what
//!     `.gitignore` already refuses to commit;
//!   * network tools additionally require an explicitly configured remote provider, so the
//!     "Local" badge stays true.
//!
//! Violations carry the reason verbatim so the transcript can show *why* a step was blocked rather
//! than a generic error.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Patterns the model may never read, whatever the sandbox mode. Mirrors the repository's own
/// `.gitignore` secret list.
const DENIED_NAMES: &[&str] = &[
    ".env",
    ".netrc",
    "id_rsa",
    "id_ed25519",
    "credentials",
    ".npmrc",
    ".p12",
    ".cer",
    ".mobileprovision",
    ".keychain",
];

/// Path segments that are never allowed to appear in a model-supplied path.
const DENIED_DIRS: &[&str] = &[".git", ".ssh", ".aws", "node_modules/.bin"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SandboxPolicy {
    pub workspace_root: PathBuf,
    /// Read-only tools may follow absolute paths outside the workspace (e.g. a file the user
    /// explicitly opened). Writes never do.
    pub read_outside_workspace: bool,
    /// True only when the user configured a remote provider; gates the network tool class.
    pub network_allowed: bool,
    /// Extra denied names on top of [`DENIED_NAMES`].
    pub extra_denied: Vec<String>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            read_outside_workspace: false,
            network_allowed: false,
            extra_denied: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase", tag = "type", content = "details")]
pub enum SandboxViolation {
    #[error("{path} resolves outside the workspace ({root})")]
    OutsideWorkspace { path: String, root: String },
    #[error("{path} is outside the workspace, and reading outside it is not allowed")]
    ReadOutsideNotAllowed { path: String },
    #[error("{path} touches a protected location ({hit})")]
    DeniedPath { path: String, hit: String },
    #[error("{path} traverses the protected directory ({hit})")]
    DeniedDirectory { path: String, hit: String },
    #[error("{path} could not be resolved: {reason}")]
    Unresolvable { path: String, reason: String },
    #[error("network access is switched off, so nothing may leave this machine")]
    NetworkDisabled,
}

impl SandboxViolation {
    /// Stable machine code for the transcript row and logs; the display string is for humans.
    pub fn code(&self) -> &'static str {
        match self {
            SandboxViolation::OutsideWorkspace { .. } => "outside-workspace",
            SandboxViolation::ReadOutsideNotAllowed { .. } => "read-outside-not-allowed",
            SandboxViolation::DeniedPath { .. } => "denied-path",
            SandboxViolation::DeniedDirectory { .. } => "denied-directory",
            SandboxViolation::Unresolvable { .. } => "unresolvable",
            SandboxViolation::NetworkDisabled => "network-disabled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

impl SandboxPolicy {
    /// Strip `.` and `..` lexically without touching the filesystem, so a `../../etc/passwd` from
    /// the model is neutralised before it can be resolved against anything.
    pub fn normalize(&self, raw: &Path) -> PathBuf {
        let base = if raw.is_absolute() {
            PathBuf::new()
        } else {
            self.workspace_root.clone()
        };
        let mut out = base;
        for c in raw.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => {
                    // A leading `..` on a relative path escapes the workspace; drop it and let the
                    // containment check below reject the result if it mattered.
                    out.pop();
                }
                other => out.push(other),
            }
        }
        out
    }

    /// Canonicalize as far as possible. Paths that do not exist yet (a `write_file` target) fall
    /// back to the nearest existing ancestor, so the containment check still sees the real location.
    fn resolve(&self, candidate: &Path) -> PathBuf {
        if let Ok(real) = candidate.canonicalize() {
            return real;
        }
        let mut trail = candidate.to_path_buf();
        let mut tail = Vec::new();
        loop {
            if let Ok(real) = trail.canonicalize() {
                let mut out = real;
                for seg in tail.iter().rev() {
                    out.push(seg);
                }
                return out;
            }
            match trail.file_name() {
                Some(name) => {
                    tail.push(name.to_os_string());
                    if !trail.pop() {
                        return candidate.to_path_buf();
                    }
                }
                None => return candidate.to_path_buf(),
            }
        }
    }

    fn is_denied(&self, path: &Path) -> Option<String> {
        let lowered = path.to_string_lossy().to_ascii_lowercase();
        for name in DENIED_NAMES
            .iter()
            .copied()
            .chain(self.extra_denied.iter().map(String::as_str))
        {
            let n = name.to_ascii_lowercase();
            if lowered.ends_with(&n) || lowered.split('/').any(|seg| seg == n) {
                return Some(name.to_string());
            }
        }
        for dir in DENIED_DIRS {
            if lowered.contains(&format!("/{dir}/")) || lowered.ends_with(&format!("/{dir}")) {
                return Some((*dir).to_string());
            }
        }
        // A `.env.local`-style variant is still an env file.
        if lowered
            .rsplit('/')
            .next()
            .is_some_and(|leaf| leaf.starts_with(".env"))
        {
            return Some(".env*".into());
        }
        None
    }

    /// The protected-location rules for one already-resolved path, as the violation they amount to.
    fn violation_at(&self, resolved: &Path, shown: &str) -> Option<SandboxViolation> {
        if let Some(hit) = self.is_denied(resolved) {
            return Some(SandboxViolation::DeniedPath {
                path: shown.to_string(),
                hit,
            });
        }
        DENIED_DIRS.iter().find_map(|dir| {
            contains_dir(resolved, Path::new(dir)).then(|| SandboxViolation::DeniedDirectory {
                path: shown.to_string(),
                hit: (*dir).to_string(),
            })
        })
    }

    /// Whether `path` names something a call may never touch, without resolving it.
    ///
    /// The directory walkers need this: they are already inside the workspace, so containment is
    /// settled, but a walk must not *enumerate* a `.env` or a `.ssh` either — a listing is an answer
    /// the same way the file's contents are.
    pub fn is_protected(&self, path: &Path) -> bool {
        self.violation_at(path, &path.display().to_string())
            .is_some()
    }

    /// The one entry point tools use: resolve `raw` for `access` or return the reason it is
    /// refused.
    pub fn resolve_path(&self, raw: &Path, access: Access) -> Result<PathBuf, SandboxViolation> {
        let candidate = self.normalize(raw);
        let resolved = self.resolve(&candidate);
        let shown = resolved.display().to_string();

        if let Some(violation) = self.violation_at(&resolved, &shown) {
            return Err(violation);
        }

        let root = self.resolve(&self.workspace_root);
        let inside = resolved.starts_with(&root);
        match access {
            Access::Write if !inside => Err(SandboxViolation::OutsideWorkspace {
                path: shown,
                root: root.display().to_string(),
            }),
            Access::Read if !inside && !self.read_outside_workspace => {
                Err(SandboxViolation::ReadOutsideNotAllowed { path: shown })
            }
            _ => Ok(resolved),
        }
    }

    /// Gate for the `Network` tool class.
    pub fn check_network(&self) -> Result<(), SandboxViolation> {
        if self.network_allowed {
            Ok(())
        } else {
            Err(SandboxViolation::NetworkDisabled)
        }
    }

    /// A workspace-relative display path for the transcript, never an absolute one.
    pub fn display_path(&self, path: &Path) -> String {
        let root = self.resolve(&self.workspace_root);
        let resolved = self.resolve(path);
        resolved
            .strip_prefix(&root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }
}

/// True when `path` passes through `prefix` as a run of complete directory components, at any depth
/// — `Path::starts_with` only anchors at the start, and a canonicalized path is already absolute.
fn contains_dir(path: &Path, prefix: &Path) -> bool {
    let segs: Vec<_> = path.components().collect();
    let pre: Vec<_> = prefix.components().collect();
    if pre.is_empty() || segs.len() < pre.len() {
        return false;
    }
    (0..=segs.len() - pre.len()).any(|i| segs[i..i + pre.len()].iter().eq(pre.iter()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(root: &Path) -> SandboxPolicy {
        SandboxPolicy {
            workspace_root: root.to_path_buf(),
            ..Default::default()
        }
    }

    #[test]
    fn traversal_is_neutralised_before_it_can_escape() {
        let dir = tempfile::tempdir().unwrap();
        let p = policy(dir.path());
        let escaped = p.normalize(Path::new("../../../etc/passwd"));
        assert!(
            !escaped.display().to_string().contains(".."),
            "{}",
            escaped.display()
        );
        let err = p.resolve_path(Path::new("../../secrets.txt"), Access::Write);
        assert!(err.is_err(), "writes outside the workspace must be refused");
    }

    #[test]
    fn writes_outside_the_workspace_are_refused_with_both_paths() {
        let dir = tempfile::tempdir().unwrap();
        let p = policy(dir.path());
        let outside = dir.path().join("..").join("elsewhere.txt");
        let err = p
            .resolve_path(&outside, Access::Write)
            .expect_err("must refuse");
        match err {
            SandboxViolation::OutsideWorkspace { path, root } => {
                assert!(path.contains("elsewhere.txt"), "{path}");
                assert!(!root.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn writes_inside_the_workspace_succeed_even_when_the_file_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let p = policy(dir.path());
        let target = dir.path().join("src").join("new.rs");
        let resolved = p
            .resolve_path(&target, Access::Write)
            .expect("a new file inside the workspace is allowed");
        assert!(resolved.display().to_string().ends_with("new.rs"));
    }

    #[test]
    fn reading_outside_is_gated_by_an_explicit_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("..").join("outside.txt");
        let strict = policy(dir.path());
        assert!(
            matches!(
                strict.resolve_path(&outside, Access::Read),
                Err(SandboxViolation::ReadOutsideNotAllowed { .. })
            ),
            "reads outside must be refused by default"
        );
        let lenient = SandboxPolicy {
            read_outside_workspace: true,
            ..strict.clone()
        };
        // Allowed by the containment rule, but see the next test: protected paths stay blocked.
        let r = lenient.resolve_path(&outside, Access::Read);
        assert!(r.is_ok() || matches!(r, Err(SandboxViolation::DeniedPath { .. })));
    }

    #[test]
    fn secrets_and_signing_identities_are_never_readable() {
        let dir = tempfile::tempdir().unwrap();
        let p = SandboxPolicy {
            read_outside_workspace: true,
            ..policy(dir.path())
        };
        for f in [
            ".env",
            "config/.env.local",
            "deploy/app.p12",
            "deploy/app.cer",
            "deploy/app.mobileprovision",
        ] {
            let path = dir.path().join(f);
            let err = p
                .resolve_path(&path, Access::Read)
                .expect_err(&format!("{f} must be protected"));
            assert!(
                matches!(err, SandboxViolation::DeniedPath { .. }),
                "{f}: {err:?}"
            );
        }
    }

    #[test]
    fn git_and_ssh_directories_are_not_traversable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        let p = policy(dir.path());
        let err = p
            .resolve_path(&dir.path().join(".git").join("config"), Access::Read)
            .expect_err("the git directory is not the model's to read");
        assert!(
            matches!(
                err,
                SandboxViolation::DeniedDirectory { .. } | SandboxViolation::DeniedPath { .. }
            ),
            "{err:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_cannot_point_out_of_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(outside.path().join("secret.txt"), &link).unwrap();
        let p = policy(dir.path());
        assert!(
            p.resolve_path(&link, Access::Write).is_err(),
            "canonicalization must defeat the symlink"
        );
    }

    #[test]
    fn a_walk_can_ask_the_protected_question_without_resolving_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = policy(dir.path());
        // The predicate the directory walkers need: an enumeration is an answer, so a `.env` or a
        // `.git` inside the workspace is held back the same way reading it would be.
        for f in [
            ".env",
            ".git/config",
            "node_modules/.bin/tool",
            "secrets/id_ed25519",
        ] {
            assert!(p.is_protected(&dir.path().join(f)), "{f} would be listed");
        }
        for f in ["src/lib.rs", "Cargo.toml", "README.md"] {
            assert!(
                !p.is_protected(&dir.path().join(f)),
                "{f} would be held back"
            );
        }
    }

    #[test]
    fn network_is_off_until_a_provider_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let p = policy(dir.path());
        assert!(matches!(
            p.check_network(),
            Err(SandboxViolation::NetworkDisabled)
        ));
        let on = SandboxPolicy {
            network_allowed: true,
            ..p
        };
        assert!(on.check_network().is_ok());
    }

    #[test]
    fn transcript_paths_are_workspace_relative() {
        let dir = tempfile::tempdir().unwrap();
        let p = policy(dir.path());
        let shown = p.display_path(&dir.path().join("crates").join("lib.rs"));
        assert_eq!(shown, format!("crates{}lib.rs", std::path::MAIN_SEPARATOR));
    }
}
