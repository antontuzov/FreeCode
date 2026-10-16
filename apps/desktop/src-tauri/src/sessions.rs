//! The conversations this app has already had, filed where the app may write.
//!
//! Until now the history lived in [`crate::chat::ChatState`] and died with the window: the sidebar's
//! Workspaces group had nothing to list because nothing remembered that a conversation had happened.
//! This module is that memory — one JSON file per conversation, under the directory the app owns
//! beside `settings.json`, which is the same place the server pid registry lives and for the same
//! reason: it has to outlive a crash and it has to be writable by a packaged app.
//!
//! Two rules keep it honest.
//!
//! *   **Only what the model was actually told.** A session holds the prompt/answer pairs the
//!     history replays, because that is what makes reopening one mean something: the next prompt is
//!     answered from the same exchange. A run's notices, refusals and timings are not replayed and so
//!     are not filed, and the reasoning stream is deliberately never written back (Bonsai-demo#183).
//! *   **Nothing invented.** The workspace name is the folder the host resolved its root from, the
//!     title is the user's own first sentence cut short, and the model is the label the run printed.
//!     A field with no real source is a field this module does not have.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Bumped when a stored file's shape changes in a way this build cannot read. A file that asks for a
/// newer version is skipped rather than half-parsed, so an app downgrade shows an empty list instead
/// of a corrupt one.
pub const SESSION_SCHEMA_VERSION: u32 = 1;

/// The directory's name under the app's data dir.
const DIR: &str = "sessions";

/// How much of the first prompt a conversation is called by. Two words' more than the sidebar's
/// column shows, so the real limit is the row's own truncation; this one exists so a pasted stack
/// trace does not become a filename.
const TITLE_MAX: usize = 60;

/// The suffix that says the title is a prefix of what was asked rather than the whole of it.
const ELLIPSIS: char = '…';

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One exchange's side. `text` is exactly what went into or came out of the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub schema_version: u32,
    pub id: String,
    /// The folder the conversation happened in, named by the root the host resolved. This is the
    /// heading the sidebar groups under.
    pub workspace: String,
    /// The first prompt, shortened. The user's own words rather than a generated summary, because a
    /// summary costs a run and this build will not spend one to label a row.
    pub title: String,
    /// The model that answered the newest turn, as the run announced it. Rewritten every turn rather
    /// than fixed at the first, because the useful answer to "which model was this" is the one that
    /// produced what is on screen now — and a conversation reopened after a model switch continues
    /// with the model now selected, whether or not that is the one that began it.
    pub model: String,
    /// Wall-clock milliseconds of the last turn filed, which is what the list sorts by.
    pub updated_at_ms: u64,
    pub turns: Vec<Turn>,
}

/// What the sidebar needs, without every answer's text crossing the IPC boundary to get it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub workspace: String,
    pub title: String,
    pub model: String,
    pub updated_at_ms: u64,
    pub turn_count: usize,
}

impl From<&Session> for Summary {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            workspace: session.workspace.clone(),
            title: session.title.clone(),
            model: session.model.clone(),
            updated_at_ms: session.updated_at_ms,
            turn_count: session.turns.len(),
        }
    }
}

impl Session {
    /// A conversation opened by `prompt`, which names it.
    pub fn new(workspace: &str, model: &str, prompt: &str) -> Self {
        Self {
            schema_version: SESSION_SCHEMA_VERSION,
            id: next_id(),
            workspace: workspace.to_string(),
            title: title_of(prompt),
            model: model.to_string(),
            updated_at_ms: now_ms(),
            turns: Vec::new(),
        }
    }

    pub fn push(&mut self, role: Role, text: &str) {
        self.turns.push(Turn {
            role,
            text: text.to_string(),
        });
        self.updated_at_ms = now_ms();
    }

    fn summary(&self) -> Summary {
        Summary::from(self)
    }

    fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.json", self.id))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// `pid` keeps two windows of the same checkout from minting the same id in the same millisecond;
/// the sequence separates the ones that happen inside it.
fn next_id() -> String {
    let n = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    format!("{}-{}-{}", now_ms(), std::process::id(), n)
}

/// The name a conversation is filed under: its first sentence, on one line, shortened.
///
/// Whitespace is collapsed because the prompt arrives from a text area, and a title that kept the
/// newlines would either break the sidebar's row or put a paragraph in a filename.
pub fn title_of(prompt: &str) -> String {
    let collapsed = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    if chars.len() <= TITLE_MAX {
        return chars.into_iter().collect();
    }
    let mut title: String = chars[..TITLE_MAX].iter().collect();
    // Cut at the last word boundary the prefix contains, so a shortened title ends on a word rather
    // than on half of one. No space in the prefix means one unbroken token, which gets the raw cut.
    if let Some(space) = title.rfind(' ') {
        if space > TITLE_MAX / 2 {
            title.truncate(space);
        }
    }
    title.push(ELLIPSIS);
    title
}

/// The directory this app files conversations in: beside `settings.json`, created on the first write
/// rather than at startup so a run that never asks a question leaves nothing behind.
pub fn dir() -> Option<PathBuf> {
    crate::paths::app_data_dir().map(|data| data.join(DIR))
}

/// The folder a conversation belongs to, named the way the status card names it: the last component
/// of the root the host resolved. A checkout gives the repository's directory; an installed app gives
/// the data directory its catalog was mirrored into.
pub fn workspace_label(app: &AppHandle) -> String {
    crate::paths::workspace_root(Some(app))
        .ok()
        .or_else(crate::paths::app_data_dir)
        .and_then(|root| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

/// Write the whole conversation, replacing the file it was. The turns are appended in memory and the
/// file rewritten, rather than the file appended to: a JSON array has no append, and a torn write is
/// then a file this build can fail to read rather than a half-line the next read inherits.
pub fn write_in(dir: &Path, session: &Session) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {} failed: {e}", dir.display()))?;
    let path = session.path_in(dir);
    let json = serde_json::to_string_pretty(session)
        .map_err(|e| format!("encoding {} failed: {e}", path.display()))?;
    std::fs::write(&path, json).map_err(|e| format!("writing {} failed: {e}", path.display()))
}

pub fn write(session: &Session) -> Result<(), String> {
    match dir() {
        Some(dir) => write_in(&dir, session),
        None => Err("this platform gives the app no directory to file conversations in".into()),
    }
}

/// An id becomes a filename here, and only in this shape, because it arrives from the window. A path
/// separator or a `..` in it would read outside the session directory, and the ids this module mints
/// are always digits with dashes.
fn safe_id(id: &str) -> Result<&str, String> {
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(id)
    } else {
        Err(format!(
            "{id} is not a conversation this app could have filed"
        ))
    }
}

pub fn read_in(dir: &Path, id: &str) -> Result<Session, String> {
    let id = safe_id(id)?;
    let path = dir.join(format!("{id}.json"));
    let bytes =
        std::fs::read(&path).map_err(|e| format!("reading {} failed: {e}", path.display()))?;
    parse(&bytes).map_err(|why| format!("{} is not a readable conversation: {why}", path.display()))
}

pub fn read(id: &str) -> Result<Session, String> {
    let dir = dir().ok_or("this platform gives the app no conversation directory")?;
    read_in(&dir, id)
}

fn parse(bytes: &[u8]) -> Result<Session, String> {
    let session: Session = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if session.schema_version > SESSION_SCHEMA_VERSION {
        return Err(format!(
            "it was filed by a newer version (schema {})",
            session.schema_version
        ));
    }
    Ok(session)
}

/// Every filed conversation, newest first. A file that does not parse is skipped rather than
/// fatal: one unreadable conversation must not empty the sidebar, and it is left on disk untouched,
/// because deleting what this build cannot read would turn a bad file into lost work.
pub fn list_in(dir: &Path) -> Vec<Summary> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Ok(session) = parse(&bytes) {
            found.push(session.summary());
        }
    }
    found.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
    found
}

pub fn list() -> Vec<Summary> {
    dir().map(|dir| list_in(&dir)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(prompt: &str) -> Session {
        let mut session = Session::new("smollm-studio", "Ternary Bonsai 1.7B (PQ2_0)", prompt);
        session.push(Role::User, prompt);
        session.push(Role::Assistant, "an answer");
        session
    }

    #[test]
    fn a_filed_conversation_comes_back_with_both_sides_of_every_turn() {
        let dir = tempfile::tempdir().unwrap();
        let first = session("what pins the engine?");
        write_in(dir.path(), &first).unwrap();
        let read = read_in(dir.path(), &first.id).unwrap();
        assert_eq!(read.turns, first.turns);
        assert_eq!(read.title, "what pins the engine?");
        assert_eq!(read.workspace, "smollm-studio");
    }

    #[test]
    fn rewriting_a_conversation_replaces_it_instead_of_leaving_a_second_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = session("first");
        write_in(dir.path(), &first).unwrap();
        first.push(Role::User, "and then");
        write_in(dir.path(), &first).unwrap();

        assert_eq!(list_in(dir.path()).len(), 1);
        assert_eq!(read_in(dir.path(), &first.id).unwrap().turns.len(), 3);
    }

    #[test]
    fn the_list_leads_with_the_conversation_that_spoke_last() {
        let dir = tempfile::tempdir().unwrap();
        let older = session("asked first");
        write_in(dir.path(), &older).unwrap();
        // `now_ms` would answer the same value twice in a millisecond, which is not what is being
        // tested here; the sort is what is.
        let mut newer = session("asked second");
        newer.updated_at_ms = older.updated_at_ms + 1000;
        write_in(dir.path(), &newer).unwrap();

        let listed = list_in(dir.path());
        assert_eq!(listed[0].id, newer.id);
        assert_eq!(listed[1].id, older.id);
        assert_eq!(listed[0].turn_count, 2);
    }

    #[test]
    fn a_title_is_the_first_sentence_on_one_line_and_short() {
        let long = "Line one\n\tand two   \n".to_string()
            + &"word ".repeat(40)
            + "the part nobody will read in a sidebar row";
        let title = title_of(&long);
        assert!(title.ends_with(ELLIPSIS), "{title}");
        assert!(title.chars().count() <= TITLE_MAX, "{title}");
        assert!(!title.contains('\n') && !title.contains("  "), "{title}");
        assert!(
            title.ends_with(&format!("word{ELLIPSIS}")),
            "{title} ends mid-word"
        );
    }

    #[test]
    fn a_short_ask_is_a_title_untouched() {
        assert_eq!(title_of("  continue  "), "continue");
    }

    #[test]
    fn an_unreadable_file_is_skipped_and_left_alone_for_the_next_version_to_read() {
        let dir = tempfile::tempdir().unwrap();
        let good = session("kept");
        write_in(dir.path(), &good).unwrap();
        std::fs::write(dir.path().join("broken.json"), b"{ not json").unwrap();

        let listed = list_in(dir.path());
        assert_eq!(
            listed.len(),
            1,
            "one unreadable file must not empty the list"
        );
        assert!(dir.path().join("broken.json").exists());
    }

    #[test]
    fn a_conversation_from_a_newer_build_is_not_half_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut future = session("from tomorrow");
        future.schema_version = SESSION_SCHEMA_VERSION + 1;
        write_in(dir.path(), &future).unwrap();

        assert!(list_in(dir.path()).is_empty());
        assert!(read_in(dir.path(), &future.id).is_err());
    }

    #[test]
    fn an_id_carrying_a_path_reads_nothing_outside_the_session_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("settings.json");
        std::fs::write(&outside, b"secret").unwrap();

        for id in ["../../settings", "/etc/passwd", "..", "a/b", ""] {
            let err = read_in(dir.path(), id).unwrap_err();
            assert!(err.contains("not a conversation"), "{id}: {err}");
        }
        assert!(outside.exists());
    }

    #[test]
    fn two_conversations_opened_in_the_same_millisecond_get_different_files() {
        let first = Session::new("w", "m", "one");
        let second = Session::new("w", "m", "two");
        assert_ne!(first.id, second.id);
    }
}
