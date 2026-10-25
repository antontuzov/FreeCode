//! The before-image of every file a run writes, and the control that puts them back.
//!
//! A write tool is the only thing in this app that changes the user's work, and the review panel can
//! show what it cost but not undo it. This module holds the other half: before a write call runs, the
//! host reads each file it is about to change; after the call lands, the new text's fingerprint is
//! recorded beside it. A revert walks those records backwards, checking at every step that the file
//! still holds exactly what the run left there, so an edit made since then is reported rather than
//! overwritten.
//!
//! The journal belongs to the live conversation: [`crate::chat::ChatState`] clears it on New task and
//! when a filed conversation is opened, because those are different conversations whose files may
//! stand in different states. It is bounded by entry count and by held bytes, and the oldest entries
//! are dropped first — an undo older than that is refused rather than guessed at.

use std::collections::VecDeque;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use freecode_tools::{write_targets, Edit, SandboxPolicy};

/// The tools refuse to read — and so to edit — a file past this size, so a before-image can never be
/// larger than it.
const MAX_BEFORE_BYTES: u64 = 1024 * 1024;
/// How many writes one conversation can undo. A run that edits more than this still works; the
/// oldest writes stop being revertible, and the panel says so instead of restoring a guess.
const MAX_ENTRIES: usize = 64;
/// What the held before-images may cost together. The entry cap alone would allow 64 MB of text.
const MAX_HELD_BYTES: usize = 8 * 1024 * 1024;

/// One write, as a revert needs to see it: the file, the text it replaced, and a fingerprint of what
/// the write left there.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Workspace-relative, spelled as the edit row spells it.
    pub path: String,
    /// The workspace the call wrote in, so a revert to a folder the user has since changed is
    /// refused rather than sent through a path that now means something else.
    pub root: PathBuf,
    /// `None` when the call created the file, which makes the revert a removal.
    pub before: Option<String>,
    pub after: u64,
}

/// A file a write call is about to change, read before the call lands.
#[derive(Debug, Clone, PartialEq)]
pub struct Captured {
    label: String,
    before: Option<String>,
}

/// The answer for one requested path.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reverted {
    pub path: String,
    pub ok: bool,
    /// The file did not exist before the run, so the revert removed it.
    pub removed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl Reverted {
    fn refused(path: &str, why: impl Into<String>) -> Self {
        Self {
            path: path.to_string(),
            ok: false,
            removed: false,
            why: Some(why.into()),
        }
    }
}

/// Read every file a write call will change, before it changes them.
///
/// A tool with no writes answers nothing, and neither does a target the sandbox refuses: a file the
/// call could not touch is not one the review may offer to revert. A file that cannot be read as text
/// is left out for the same reason — an undo built on half of a file's bytes is a corrupted file.
pub fn capture(tool: &str, arguments: &Value, policy: &SandboxPolicy) -> Vec<Captured> {
    let mut out: Vec<Captured> = Vec::new();
    for abs in write_targets(tool, arguments, policy) {
        let label = policy
            .display_path(&abs)
            .replace(std::path::MAIN_SEPARATOR, "/");
        if out.iter().any(|held| held.label == label) {
            continue;
        }
        let before = match fs::metadata(&abs) {
            Ok(meta) if meta.is_file() && meta.len() <= MAX_BEFORE_BYTES => {
                match fs::read_to_string(&abs) {
                    Ok(text) => Some(text),
                    // Not text this build can edit, so the executor will refuse it too.
                    Err(_) => continue,
                }
            }
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => continue,
        };
        out.push(Captured { label, before });
    }
    out
}

/// Turn the captured files and the call's own report of what it changed into journal entries.
///
/// Only a path both halves name is recorded: a target the executor decided not to write has no edit,
/// and an edit with no captured before-image cannot be reverted safely.
pub fn entries(captured: &[Captured], edits: &[Edit], root: &Path) -> Vec<Entry> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    edits
        .iter()
        .filter_map(|edit| {
            let held = captured.iter().find(|held| held.label == edit.path)?;
            let after = fs::read_to_string(root.join(&edit.path)).ok()?;
            Some(Entry {
                path: edit.path.clone(),
                root: root.clone(),
                before: held.before.clone(),
                after: fingerprint(&after),
            })
        })
        .collect()
}

/// The live conversation's write history, newest last.
#[derive(Debug, Default)]
pub struct Journal {
    entries: VecDeque<Entry>,
    held: usize,
}

impl Journal {
    /// Add one call's writes. The oldest entries go first once either bound is passed, because the
    /// recent past is what a revert is asked for; a dropped entry makes its path unrevertible and the
    /// panel says that rather than restoring a state from before other changes it no longer holds.
    pub fn record(&mut self, entries: Vec<Entry>) {
        for entry in entries {
            self.held += cost(&entry);
            self.entries.push_back(entry);
        }
        while self.entries.len() > MAX_ENTRIES {
            self.evict();
        }
        while self.held > MAX_HELD_BYTES && self.entries.len() > 1 {
            self.evict();
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.held = 0;
    }

    /// Only the tests ask: the panel learns whether there is anything to revert from the rows it is
    /// showing, which are the run's own.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Put the named files back to the state before the run first wrote them.
    ///
    /// Every step is checked before anything is written: the file has to hold what the write left,
    /// and the previous entry's `after` has to match what the step before it would restore. A path
    /// that fails any check is left exactly as it stands and answered with the reason, so a revert of
    /// five files never leaves one of them half-restored.
    pub fn revert(&mut self, paths: &[String], root: &Path) -> Vec<Reverted> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for path in paths {
            if seen.contains(&path.as_str()) {
                continue;
            }
            seen.push(path.as_str());
            out.push(self.revert_one(path, &root));
        }
        out
    }

    fn revert_one(&mut self, path: &str, root: &Path) -> Reverted {
        if !addressable(path) {
            return Reverted::refused(
                path,
                "that is not a path inside the project, so nothing was put back",
            );
        }
        let count = self.entries.iter().filter(|e| e.path == path).count();
        if count == 0 {
            return Reverted::refused(
                path,
                "this window is not holding the state before that file was written. Only the session \
                 that wrote it holds it.",
            );
        }
        if self
            .entries
            .iter()
            .any(|e| e.path == path && e.root != root)
        {
            return Reverted::refused(
                path,
                "the project folder has changed since that file was written, so nothing was put back",
            );
        }

        let abs = root.join(path);
        let mut current = match fs::read_to_string(&abs) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Reverted::refused(
                    path,
                    format!("{} could not be read back: {e}", abs.display()),
                )
            }
        };
        for entry in self.entries.iter().rev().filter(|e| e.path == path) {
            let Some(text) = current.as_deref() else {
                return Reverted::refused(
                    path,
                    "the file is no longer on disk, so nothing was put back",
                );
            };
            if fingerprint(text) != entry.after {
                return Reverted::refused(
                    path,
                    "the file has changed since the run wrote it, so nothing was put back",
                );
            }
            current = entry.before.clone();
        }

        let removed = match current {
            Some(text) => {
                if let Err(e) = fs::write(&abs, text) {
                    return Reverted::refused(
                        path,
                        format!("writing the earlier text back failed: {e}"),
                    );
                }
                false
            }
            None => {
                match fs::remove_file(&abs) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Reverted::refused(
                            path,
                            format!("removing the file the run created failed: {e}"),
                        )
                    }
                }
                remove_empty_parents(abs.parent(), root);
                true
            }
        };

        self.entries.retain(|entry| entry.path != path);
        self.held = self.entries.iter().map(cost).sum();
        Reverted {
            path: path.to_string(),
            ok: true,
            removed,
            why: None,
        }
    }

    fn evict(&mut self) {
        if let Some(entry) = self.entries.pop_front() {
            self.held = self.held.saturating_sub(cost(&entry));
        }
    }
}

fn cost(entry: &Entry) -> usize {
    entry.before.as_ref().map_or(0, String::len)
}

/// A path the journal may address: relative, with no hop that could leave the root it is joined to.
/// The executors only ever report such paths, and this is the second lock on the door anyway.
fn addressable(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// FNV-1a. A fingerprint rather than a stored copy of the new text: the journal only has to tell
/// "still what the run wrote" from "changed since", and a second copy of every edit doubles what the
/// conversation holds for a comparison one number can make.
fn fingerprint(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Take back the directories a create made, as far as they are now empty. A folder that holds
/// anything else — a file the user put there, or one the run wrote afterwards — is left alone, and
/// the root itself is never a candidate.
fn remove_empty_parents(mut dir: Option<&Path>, root: &Path) {
    while let Some(here) = dir {
        if here == root || !here.starts_with(root) || fs::remove_dir(here).is_err() {
            break;
        }
        dir = here.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The half the panel's press depends on: what `capture` held before a call, what the real
    /// executor reported after it, and the file the journal then puts back. Built against the
    /// executor rather than a hand-written entry because the two halves meet on the edit row's path
    /// spelling — a journal that never agreed with the call's own report would revert nothing.
    #[test]
    fn a_real_write_lands_in_the_journal_and_comes_back_out_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("notes.txt");
        fs::write(&file, "alpha\nbeta\n").unwrap();
        let policy = SandboxPolicy {
            workspace_root: root.clone(),
            ..SandboxPolicy::default()
        };
        let arguments = json!({
            "path": "notes.txt",
            "old_string": "beta",
            "new_string": "beta edited",
        });

        let captured = capture("edit_file", &arguments, &policy);
        let done = freecode_tools::execute("edit_file", &arguments, &policy).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "alpha\nbeta edited\n",
            "the executor did not write"
        );

        let mut journal = Journal::default();
        journal.record(entries(&captured, &done.edits, &root));
        let out = journal.revert(&["notes.txt".into()], &root);

        assert!(out[0].ok, "{:?}", out[0]);
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha\nbeta\n");
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn edit(path: &str, before: Option<&str>, after: &str, root: &Path) -> Entry {
        Entry {
            path: path.to_string(),
            root: root.canonicalize().unwrap(),
            before: before.map(str::to_string),
            after: fingerprint(after),
        }
    }

    #[test]
    fn reverting_an_edit_restores_the_text_it_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("src/lib.rs");
        write(&file, "one\ntwo\n");
        let mut journal = Journal::default();
        journal.record(vec![edit("src/lib.rs", Some("one\n"), "one\ntwo\n", root)]);

        let out = journal.revert(&["src/lib.rs".into()], root);

        assert!(out[0].ok, "{:?}", out[0]);
        assert!(!out[0].removed);
        assert_eq!(fs::read_to_string(&file).unwrap(), "one\n");
        assert!(journal.is_empty());
    }

    #[test]
    fn reverting_a_create_removes_the_file_and_the_folders_it_made() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("my-app/pages/index.html");
        write(&file, "<p>hello</p>\n");
        let mut journal = Journal::default();
        journal.record(vec![edit(
            "my-app/pages/index.html",
            None,
            "<p>hello</p>\n",
            root,
        )]);

        let out = journal.revert(&["my-app/pages/index.html".into()], root);

        assert!(out[0].ok && out[0].removed, "{:?}", out[0]);
        assert!(!file.exists());
        assert!(!root.join("my-app").exists(), "the create's folders stayed");
        assert!(root.exists(), "the workspace itself was removed");
    }

    #[test]
    fn a_file_changed_since_the_write_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("notes.md");
        write(&file, "mine\n");
        let mut journal = Journal::default();
        journal.record(vec![edit(
            "notes.md",
            Some("old\n"),
            "the run's text\n",
            root,
        )]);

        let out = journal.revert(&["notes.md".into()], root);

        assert!(!out[0].ok);
        assert!(
            out[0].why.as_deref().unwrap_or("").contains("changed"),
            "{:?}",
            out[0]
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "mine\n");
        assert!(!journal.is_empty(), "a refused revert keeps the record");
    }

    #[test]
    fn two_writes_to_one_file_revert_to_the_text_before_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("a.txt");
        write(&file, "three\n");
        let mut journal = Journal::default();
        journal.record(vec![
            edit("a.txt", Some("one\n"), "two\n", root),
            edit("a.txt", Some("two\n"), "three\n", root),
        ]);

        let out = journal.revert(&["a.txt".into()], root);

        assert!(out[0].ok, "{:?}", out[0]);
        assert_eq!(fs::read_to_string(&file).unwrap(), "one\n");
    }

    #[test]
    fn a_create_then_an_edit_reverts_to_a_file_that_is_not_there() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("new.txt");
        write(&file, "grown\n");
        let mut journal = Journal::default();
        journal.record(vec![
            edit("new.txt", None, "born\n", root),
            edit("new.txt", Some("born\n"), "grown\n", root),
        ]);

        let out = journal.revert(&["new.txt".into()], root);

        assert!(out[0].ok && out[0].removed, "{:?}", out[0]);
        assert!(!file.exists());
    }

    #[test]
    fn a_path_the_journal_never_saw_is_refused_rather_than_guessed_at() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.txt"), "x\n");
        let mut journal = Journal::default();

        let out = journal.revert(&["a.txt".into()], root);

        assert!(!out[0].ok);
        assert!(
            out[0].why.as_deref().unwrap_or("").contains("not holding"),
            "{:?}",
            out[0]
        );
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "x\n");
    }

    #[test]
    fn a_path_with_a_hop_out_of_the_workspace_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let outside = dir.path().parent().unwrap().join("outside.txt");
        write(&outside, "keep\n");
        let mut journal = Journal::default();
        journal.record(vec![edit("a.txt", Some("keep\n"), "changed\n", root)]);

        let out = journal.revert(&["../outside.txt".into()], root);

        assert!(!out[0].ok);
        assert!(
            out[0].why.as_deref().unwrap_or("").contains("not a path"),
            "{:?}",
            out[0]
        );
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep\n");
        fs::remove_file(&outside).unwrap();
    }

    #[test]
    fn a_record_from_another_project_is_refused() {
        let here = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let file = here.path().join("a.txt");
        write(&file, "changed\n");
        let mut journal = Journal::default();
        journal.record(vec![edit(
            "a.txt",
            Some("keep\n"),
            "changed\n",
            elsewhere.path(),
        )]);

        let out = journal.revert(&["a.txt".into()], here.path());

        assert!(!out[0].ok);
        assert!(
            out[0]
                .why
                .as_deref()
                .unwrap_or("")
                .contains("project folder"),
            "{:?}",
            out[0]
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "changed\n");
    }

    #[test]
    fn a_missing_file_is_refused_instead_of_recreated_from_a_chain() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut journal = Journal::default();
        journal.record(vec![edit("gone.txt", Some("old\n"), "run\n", root)]);

        let out = journal.revert(&["gone.txt".into()], root);

        assert!(!out[0].ok);
        assert!(
            out[0]
                .why
                .as_deref()
                .unwrap_or("")
                .contains("no longer on disk"),
            "{:?}",
            out[0]
        );
        assert!(!root.join("gone.txt").exists());
    }

    #[test]
    fn the_oldest_records_are_dropped_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut journal = Journal::default();
        let batch: Vec<Entry> = (0..=MAX_ENTRIES)
            .map(|n| edit(&format!("f{n}.txt"), Some("x"), "y", root))
            .collect();
        journal.record(batch);

        assert_eq!(journal.entries.len(), MAX_ENTRIES);
        assert!(!journal.entries.iter().any(|e| e.path == "f0.txt"));
    }

    #[test]
    fn a_non_empty_folder_a_create_used_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let created = root.join("app/kept.txt");
        write(&created, "run\n");
        write(&root.join("app/user.txt"), "mine\n");
        let mut journal = Journal::default();
        journal.record(vec![edit("app/kept.txt", None, "run\n", root)]);

        journal.revert(&["app/kept.txt".into()], root);

        assert!(!created.exists());
        assert_eq!(
            fs::read_to_string(root.join("app/user.txt")).unwrap(),
            "mine\n"
        );
    }
}
