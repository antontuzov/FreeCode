//! The Storage page: what this app has put on the machine, and what the machine has left.
//!
//! A storage screen in a cloud app counts what is synced. The local answer to the same row is the
//! three directories this host writes into and nothing else: the weights the downloader placed, the
//! engine the installer unpacked, and the conversations the sidebar filed. Each figure is walked from
//! the files themselves rather than quoted from the catalog, because a pack that was half-overwritten
//! by hand and a pack that matches its published size are different facts, and only the first one is
//! interesting to the user.
//!
//! Free space is read on the volume a download would land in — the weights directory, the same one
//! [`crate::weights`]’s preflight measures before it starts a fetch — so the row cannot promise room
//! the downloader then refuses for. Nothing here is cached or estimated: a directory that does not
//! exist yet reads as zero bytes, which is true and is what the page shows.

use std::path::Path;

use serde::Serialize;
use smollm_config::Settings;
use smollm_models::EngineLock;

/// How far below a reported directory the walk goes. The app's own trees are two levels deep —
/// `engines/<tag>/<backend>/llama-server` is the deepest — so a directory nested past this was not
/// written here, and counting it would report someone else's tree as this app's.
const MAX_DEPTH: usize = 4;

/// How many directories one measurement may enter. The walk always terminates — depth is bounded and
/// no link is followed — so this is a time bound rather than a safety one: a weights directory
/// pointed at a home folder holds more files than a window should wait for, and a figure that is
/// short by a tree is better than a page that never arrives.
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
    /// Whether the directory is there at all. The page says "not downloaded yet" rather than "0 MB"
    /// for a directory that was never created, because the second reads like a deleted one.
    pub present: bool,
}

/// Everything the page renders, in one read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub weights: Area,
    pub engine: Area,
    pub sessions: Area,
    /// The three above, summed — the figure the section header leads with, so the page states what
    /// this app owns rather than making the user add three rows.
    pub total_bytes: u64,
    /// Bytes the volume behind the weights directory can still allocate for an unprivileged process.
    pub free_bytes: u64,
    /// The directory the free figure was read on, which is where the next pack would be written.
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
    /// Read from the same root and settings every other host command resolves, so the page cannot
    /// report a directory the engine would not load from.
    pub fn build(root: &Path, settings: &Settings) -> Result<Self, String> {
        // The pinned tag is the directory the installer unpacked into. A lock that will not parse is
        // not a report with a blank engine row — the page would then be showing two true figures and
        // one unknown one without saying which.
        let lock = EngineLock::load_default(root).map_err(|e| e.to_string())?;
        let wdir = crate::status::weights_dir(root, settings);
        let weights = Area::measure(&wdir);
        let engine = Area::measure(&smollm_llama::tag_dir(root, &lock));
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
        let free_bytes = available_disk(&wdir, root)?;
        let total_bytes = weights.bytes + engine.bytes + sessions.bytes;
        Ok(Report {
            weights,
            engine,
            sessions,
            total_bytes,
            free_bytes,
            root: wdir.display().to_string(),
        })
    }
}

/// Free space on the volume a download would fill.
///
/// This is the probe [`crate::weights`] refuses a download with, deliberately: a storage page that
/// read the volume a second way could promise room the same app then declines to use. The probe
/// reports zero when it fails, which is treated as unknown rather than as an empty disk — and a
/// weights directory that does not exist yet answers zero for a real reason, so the root on the same
/// volume is tried before the page gives up.
fn available_disk(weights: &Path, root: &Path) -> Result<u64, String> {
    let probe = |dir: &Path| {
        smollm_models::HostHardware::detect(&dir.display().to_string()).available_disk_bytes
    };
    let bytes = probe(weights);
    if bytes > 0 {
        return Ok(bytes);
    }
    let bytes = probe(root);
    if bytes > 0 {
        Ok(bytes)
    } else {
        Err(format!(
            "neither {} nor {} reported free space",
            weights.display(),
            root.display()
        ))
    }
}

/// Bytes of the regular files under `dir`, and whether `dir` exists.
///
/// Symlinks are counted as their own size and never descended into: this walk runs on a directory the
/// user may have pointed somewhere else, and following a link out of it would report another tree as
/// this app's storage — or loop forever on one that points back.
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

    fn settings_with(dir: &Path) -> Settings {
        let mut settings = Settings::default();
        settings.local_model.model_dir = Some(dir.display().to_string());
        settings
    }

    /// The checkout's own lock, copied into a scratch tree. Invented TOML would pin a tag this build
    /// would never unpack, and the engine row's whole claim is that it names the directory the
    /// installer wrote to.
    fn real_lock(tree: &Path) -> EngineLock {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let to = tree.join(smollm_models::ENGINE_LOCK_REL);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(from.join(smollm_models::ENGINE_LOCK_REL), &to).unwrap();
        EngineLock::load(&to).unwrap()
    }

    /// The figure is the logical size of the files, added across the depths this app actually writes
    /// to — `engines/<tag>/<backend>/llama-server` is two levels below the reported directory.
    #[test]
    fn a_directories_bytes_are_its_regular_files_across_the_depths_the_app_writes() {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "a.gguf", 1_000);
        file(dir.path(), "nested/b.gguf", 250);
        file(dir.path(), "nested/deeper/c.gguf", 7);
        assert_eq!(usage(dir.path()), (1_257, true));
    }

    #[test]
    fn a_directory_that_was_never_created_reads_absent_rather_than_empty() {
        let missing = PathBuf::from(tempfile::tempdir().unwrap().path()).join("models");
        assert_eq!(usage(&missing), (0, false));
    }

    /// A weight directory pointed at the home folder would otherwise report the machine as this
    /// app's storage, and a link back into its own tree would never stop being walked.
    #[test]
    #[cfg(unix)]
    fn a_symlink_is_counted_as_a_link_and_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        file(outside.path(), "huge.bin", 100_000);
        file(dir.path(), "real.gguf", 10);
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
        file(dir.path(), "a.gguf", 4);
        link(dir.path(), &dir.path().join("self"));
        let (bytes, _) = usage(dir.path());
        assert!(bytes > 0);
    }

    /// The three areas are the only directories this host writes into, so the figure the header prints
    /// is the sum of the rows under it — and the free space is read on the volume a download would
    /// need room on.
    #[test]
    fn the_report_adds_the_rows_it_shows() {
        let tree = tempfile::tempdir().unwrap();
        let weights = tempfile::tempdir().unwrap();
        file(weights.path(), "pack.gguf", 512);
        real_lock(tree.path());
        let report = Report::build(tree.path(), &settings_with(weights.path())).unwrap();

        assert_eq!(report.weights.bytes, 512);
        assert!(report.weights.present);
        // Nothing has been unpacked into this scratch tree, and the page has to say that rather than
        // show an engine directory of 0 bytes as if a download had emptied it.
        assert!(!report.engine.present);
        assert_eq!(report.engine.bytes, 0);
        // The sessions row is this machine's real store, so the claim tested here is the one the
        // header makes: that it is the sum of the rows beneath it, whatever those rows hold.
        assert_eq!(
            report.total_bytes,
            report.weights.bytes + report.engine.bytes + report.sessions.bytes
        );
        assert!(
            report.free_bytes > 0,
            "the volume reads no free space at all"
        );
        assert_eq!(report.root, weights.path().display().to_string());
    }

    /// The engine row measures the pinned tag's directory, because that is the one the installer
    /// unpacks into and the one a re-pin leaves behind.
    #[test]
    fn the_engine_row_measures_the_pinned_tag_and_not_its_siblings() {
        let tree = tempfile::tempdir().unwrap();
        let lock = real_lock(tree.path());
        let dir = smollm_llama::tag_dir(tree.path(), &lock);
        file(&dir, "macos-arm64/llama-server", 900);
        file(
            &smollm_llama::engines_root(tree.path()).join("an-older-tag"),
            "left.gguf",
            4_000,
        );
        let weights = tempfile::tempdir().unwrap();
        let report = Report::build(tree.path(), &settings_with(weights.path())).unwrap();

        assert_eq!(report.engine.bytes, 900);
        assert!(report.engine.dir.ends_with(&lock.pinned_tag));
    }

    /// The fresh-install state: nothing has been written yet, and the page still names the volume the
    /// first download would fill instead of failing on a directory that is not there.
    #[test]
    fn free_space_is_still_read_when_the_weights_directory_is_absent() {
        let tree = tempfile::tempdir().unwrap();
        real_lock(tree.path());
        let report =
            Report::build(tree.path(), &settings_with(&tree.path().join("models"))).unwrap();
        assert!(!report.weights.present);
        assert!(report.free_bytes > 0);
    }

    /// A tree whose lock this build cannot read has no pinned tag, so there is no engine directory to
    /// name. The command says so rather than reporting the two figures it does know as the whole page.
    #[test]
    fn a_lock_that_will_not_parse_is_a_failure_the_page_reports() {
        let tree = tempfile::tempdir().unwrap();
        file(tree.path(), smollm_models::ENGINE_LOCK_REL, 20);
        let err = Report::build(tree.path(), &Settings::default()).unwrap_err();
        assert!(err.contains("lock"), "{err}");
    }
}
