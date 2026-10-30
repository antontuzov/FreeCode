//! The Storage page: what this app has put on the machine.
//!
//! This build downloads no model and unpacks no engine — every answer comes from the free route — so
//! the only directory it owns is the one holding the conversations the sidebar filed. The figure is
//! walked from the files themselves rather than quoted, because a directory that was half-written and
//! one that is intact are different facts, and only the first one is interesting to the user.

use std::path::Path;

use serde::Serialize;

/// How far below a reported directory the walk goes. The app's own tree is shallow, so a directory
/// nested past this was not written here, and counting it would report someone else's tree as this
/// app's.
const MAX_DEPTH: usize = 4;

/// How many directories one measurement may enter. The walk always terminates — depth is bounded and
/// no link is followed — so this is a time bound rather than a safety one.
const DIRECTORY_CAP: usize = 2_000;

/// One directory the page reports.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    /// The directory as the host resolved it, so the figure can be checked against the files there.
    /// Bytes are the logical size of each file, which is what `ls -l` prints; `du` reports blocks and
    /// will read larger, so the two are the same files measured two ways rather than a disagreement.
    pub dir: String,
    pub bytes: u64,
    /// Whether the directory is there at all. The page says "not filed yet" rather than "0 MB" for a
    /// directory that was never created, because the second reads like a deleted one.
    pub present: bool,
}

/// Everything the page renders, in one read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub sessions: Area,
    /// The sessions row, repeated as the section header's leading figure.
    pub total_bytes: u64,
    /// The app's data root, which is what the page names beneath the figure.
    pub root: String,
}

impl Area {
    fn measure(dir: &Path) -> Self {
        let (bytes, present) = usage(dir);
        Area {
            dir: dir.display().to_string(),
            bytes,
            present,
        }
    }
}

impl Report {
    pub fn build(root: &Path) -> Self {
        // Sessions live beside `settings.json`, not under the root: a packaged app writes its data
        // where the bundle's signature does not reach. A platform that gives the app no such
        // directory has no filed conversations to count either, so the row reads absent.
        let sessions = match crate::sessions::dir() {
            Some(dir) => Area::measure(&dir),
            None => Area {
                dir: String::new(),
                bytes: 0,
                present: false,
            },
        };
        Report {
            total_bytes: sessions.bytes,
            sessions,
            root: root.display().to_string(),
        }
    }
}

/// Bytes of the regular files under `dir`, and whether `dir` exists.
///
/// Symlinks are counted as their own size and never descended into: following a link out of the app's
/// directory would report another tree as this app's storage — or loop forever on one that points
/// back.
fn usage(dir: &Path) -> (u64, bool) {
    let mut entered = 0;
    (walk(dir, 0, &mut entered), dir.is_dir())
}

fn walk(dir: &Path, depth: usize, entered: &mut usize) -> u64 {
    if depth > MAX_DEPTH || *entered >= DIRECTORY_CAP {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    *entered += 1;
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let path = entry.path();
        // `read_dir`'s metadata follows the link, so the file type is checked on the link itself.
        let Ok(link) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if link.file_type().is_symlink() {
            total += link.len();
        } else if meta.is_dir() {
            total += walk(&path, depth + 1, entered);
        } else if meta.is_file() {
            total += meta.len();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(dir: &Path, rel: &str, bytes: usize) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![b'x'; bytes]).unwrap();
    }

    #[cfg(unix)]
    fn link(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    /// The figure is the logical size of the files, added across the depths the app writes to.
    #[test]
    fn a_directories_bytes_are_its_regular_files_across_the_depths_the_app_writes() {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "a.json", 1_000);
        file(dir.path(), "nested/b.json", 250);
        file(dir.path(), "nested/deeper/c.json", 7);
        assert_eq!(usage(dir.path()), (1_257, true));
    }

    #[test]
    fn a_directory_that_was_never_created_reads_absent_rather_than_empty() {
        let missing = PathBuf::from(tempfile::tempdir().unwrap().path()).join("sessions");
        assert_eq!(usage(&missing), (0, false));
    }

    /// A link back into its own tree must not be followed, or the walk would never stop.
    #[test]
    #[cfg(unix)]
    fn a_symlink_is_counted_as_a_link_and_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        file(outside.path(), "huge.bin", 100_000);
        file(dir.path(), "real.json", 10);
        link(outside.path(), &dir.path().join("escape"));

        let escape = std::fs::symlink_metadata(dir.path().join("escape")).unwrap();
        assert_eq!(
            usage(dir.path()).0,
            10 + escape.len(),
            "the tree outside the directory was counted as this app's"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_link_back_into_its_own_directory_terminates() {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "a.json", 4);
        link(dir.path(), &dir.path().join("self"));
        let (bytes, _) = usage(dir.path());
        assert!(bytes > 0);
    }

    /// The header's figure is the row beneath it, whatever the machine's store holds.
    #[test]
    fn the_report_adds_the_rows_it_shows() {
        let root = tempfile::tempdir().unwrap();
        let report = Report::build(root.path());
        assert_eq!(report.total_bytes, report.sessions.bytes);
        assert_eq!(report.root, root.path().display().to_string());
    }
}
