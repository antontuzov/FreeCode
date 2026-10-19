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

/// How many conversations one search hands the window. A store this size is scanned in full anyway,
/// but a result list longer than the palette's panel is not a result list.
const HIT_MAX: usize = 12;

/// How many characters of the exchange the palette shows around a match — enough to tell the two
/// conversations that both mention the same model apart, short enough to stay on two lines.
const EXCERPT_PAD: usize = 48;

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
    /// A prompt's attached picture, as the `data:` URL the engine was handed. Filed because the rule
    /// above is the rule: it is part of what the model was told, and a reopened conversation that
    /// replayed the question without it would answer a different question than the one on screen.
    /// `skip_serializing_if` keeps every text-only line — which is all of them until a picture is
    /// attached — exactly the file it was before the field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
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

/// A conversation the search found, and the line it found it in.
///
/// The summary travels flattened into the same object so the palette's row has the title, the
/// workspace and the model to draw without a second call — the fields it lists are the ones it shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    #[serde(flatten)]
    pub summary: Summary,
    /// Which side of the exchange said the words the query matched. The palette names it, because a
    /// conversation you matched by what you asked and one you matched by what came back are two
    /// different things to find.
    pub role: Role,
    /// The matched line, cut to around the match and collapsed onto the kind of one line a row holds.
    pub excerpt: String,
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

    pub fn push(&mut self, role: Role, text: &str, image: Option<String>) {
        self.turns.push(Turn {
            role,
            text: text.to_string(),
            image,
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

/// Remove one conversation's file.
///
/// A file that is not there answers `Ok` rather than an error: another window can have deleted the
/// same row a moment before, and the state the user asked for — that conversation is gone — holds
/// either way. An error there would be a message about a bookkeeping race the row cannot show.
///
/// The id is checked by the same rule a read is, so this deletes inside the session directory and
/// nowhere else, whatever the window sends. A file this build cannot parse is still deleted: the
/// conversation it holds is unreadable here, and "throw this away" is the one instruction that needs
/// no reading.
pub fn delete_in(dir: &Path, id: &str) -> Result<(), String> {
    let id = safe_id(id)?;
    match std::fs::remove_file(dir.join(format!("{id}.json"))) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("deleting {id} failed: {e}")),
    }
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
fn sessions_in(dir: &Path) -> Vec<Session> {
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
            found.push(session);
        }
    }
    found.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
    found
}

pub fn list_in(dir: &Path) -> Vec<Summary> {
    sessions_in(dir).iter().map(Session::summary).collect()
}

pub fn list() -> Vec<Summary> {
    dir().map(|dir| list_in(&dir)).unwrap_or_default()
}

/// The conversations that say what the query says, best match first.
///
/// Every conversation file is read, because the turns are what is searched and the list hands out
/// summaries on purpose. That is a directory of a few dozen small JSON files, so a scan costs less
/// than the palette waiting on one would; a store that outgrows it gets an index, not a slower scan.
pub fn search_in(dir: &Path, query: &str) -> Vec<Hit> {
    let needle = query.trim();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Hit> = sessions_in(dir)
        .iter()
        .filter_map(|session| hit_of(session, needle))
        .collect();
    // `hit_of` already says which side matched, and the sort is stable, so the conversations whose
    // own questions matched lead — newest first among them — with the ones only an answer mentioned
    // behind. Both groups were already newest-first, so this is the only reordering.
    hits.sort_by_key(|hit| hit.role == Role::Assistant);
    hits.truncate(HIT_MAX);
    hits
}

pub fn search(query: &str) -> Vec<Hit> {
    dir().map(|dir| search_in(&dir, query)).unwrap_or_default()
}

/// The one turn a conversation is matched by: the first that asked the query, or, if only the
/// answer said it, the first that did.
///
/// Questions are searched before answers rather than the whole exchange in order, because the row a
/// palette offers for a phrase in the prompt is the conversation that phrase names, and one found
/// in a paragraph of answer text is a conversation that merely happens to contain it.
fn hit_of(session: &Session, query: &str) -> Option<Hit> {
    for role in [Role::User, Role::Assistant] {
        for turn in session.turns.iter().filter(|turn| turn.role == role) {
            let Some(excerpt) = excerpt_of(&turn.text, query) else {
                continue;
            };
            return Some(Hit {
                summary: session.summary(),
                role,
                excerpt,
            });
        }
    }
    None
}

/// The line around the first case-insensitive occurrence of `query`, or `None` when the text has no
/// occurrence. Ellipses mark the ends that the cut dropped.
///
/// Whitespace is collapsed before the search so an excerpt is one line — the text arrives from a
/// transcript with paragraphs, newlines and the runs of spaces markdown leaves behind — and the
/// search is done on that collapsed copy so the cut and the match are measured in the same string.
/// Matching is per character with a case fold rather than `to_lowercase` on both halves, because a
/// lowered copy's byte offsets do not line up with the original text the excerpt is quoted from.
fn excerpt_of(text: &str, query: &str) -> Option<String> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    let needle: Vec<char> = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(fold)
        .collect();
    if needle.is_empty() || chars.len() < needle.len() {
        return None;
    }
    let folded: Vec<char> = chars.iter().map(|c| fold(*c)).collect();
    let at = folded.windows(needle.len()).position(|w| w == needle)?;
    let from = at.saturating_sub(EXCERPT_PAD);
    let to = (at + needle.len() + EXCERPT_PAD).min(chars.len());
    let mut out = String::new();
    if from > 0 {
        out.push(ELLIPSIS);
    }
    out.extend(chars[from..to].iter());
    if to < chars.len() {
        out.push(ELLIPSIS);
    }
    Some(out)
}

/// A character as the search sees it. A character whose lowercase is more than one character keeps
/// its first, which means both sides of the comparison are measured one input character at a time and
/// the position found is still a position in the text that gets quoted.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(prompt: &str) -> Session {
        let mut session = Session::new("smollm-studio", "Ternary Bonsai 1.7B (PQ2_0)", prompt);
        session.push(Role::User, prompt, None);
        session.push(Role::Assistant, "an answer", None);
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
        first.push(Role::User, "and then", None);
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
    fn a_deleted_conversation_leaves_no_row_and_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let kept = session("kept");
        let gone = session("deleted");
        write_in(dir.path(), &kept).unwrap();
        write_in(dir.path(), &gone).unwrap();

        delete_in(dir.path(), &gone.id).unwrap();

        assert_eq!(list_in(dir.path()).len(), 1);
        assert_eq!(list_in(dir.path())[0].id, kept.id);
        assert!(!dir.path().join(format!("{}.json", gone.id)).exists());
        assert!(read_in(dir.path(), &gone.id).is_err());
        // The one that stays is not merely listed; its turns are still the ones that were filed.
        assert_eq!(read_in(dir.path(), &kept.id).unwrap().turns, kept.turns);
    }

    #[test]
    fn a_row_that_another_window_already_deleted_is_the_state_this_one_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing was ever filed under this id, and the sidebar's row is gone all the same once the
        // window drops it locally — an error here would be a message about a race nobody can see.
        delete_in(dir.path(), "1700000000000-1-1").unwrap();
        assert!(list_in(dir.path()).is_empty());
    }

    #[test]
    fn an_id_carrying_a_path_deletes_nothing_outside_the_session_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("settings.json");
        std::fs::write(&outside, b"secret").unwrap();

        for id in ["../../settings", "/etc/passwd", "..", "a/b", ""] {
            let err = delete_in(dir.path(), id).unwrap_err();
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
    #[test]
    fn a_search_finds_the_conversation_whose_question_said_it() {
        let dir = tempfile::tempdir().unwrap();
        let asked = session("what pins the engine version?");
        let unrelated = session("tell me about mint");
        write_in(dir.path(), &asked).unwrap();
        write_in(dir.path(), &unrelated).unwrap();

        let hits = search_in(dir.path(), "pins the engine");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].summary.id, asked.id);
        assert_eq!(hits[0].role, Role::User);
        assert!(
            hits[0].excerpt.contains("pins the engine"),
            "{}",
            hits[0].excerpt
        );
    }

    #[test]
    fn a_query_is_read_whatever_case_it_and_the_file_were_in() {
        let dir = tempfile::tempdir().unwrap();
        write_in(dir.path(), &session("Which LINE-numbers setting ANSWERS?")).unwrap();

        assert_eq!(
            search_in(dir.path(), "line-numbers setting answers").len(),
            1
        );
        assert_eq!(search_in(dir.path(), "WHICH line-numbers").len(), 1);
    }

    #[test]
    fn a_match_only_in_the_answer_still_found_is_named_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut only = Session::new("smollm-studio", "m", "what did you say?");
        only.push(Role::User, "what did you say?", None);
        only.push(Role::Assistant, "the fork commit pins the engine", None);
        write_in(dir.path(), &only).unwrap();

        let hits = search_in(dir.path(), "pins the engine");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, Role::Assistant);
    }

    #[test]
    fn a_question_leads_an_answer_when_both_matched_and_both_are_newest_of_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let mut mentioned = session("tell me about mint");
        // Filed later, so the newest-first scan would put it first if the match's side did not count.
        mentioned.updated_at_ms += 1000;
        mentioned.push(
            Role::Assistant,
            "the engine pin you asked about is in the catalog",
            None,
        );
        let mut asked = session("where is the engine pin recorded?");
        asked.push(Role::Assistant, "in the catalog", None);
        write_in(dir.path(), &mentioned).unwrap();
        write_in(dir.path(), &asked).unwrap();

        let hits = search_in(dir.path(), "engine pin");
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].summary.id, asked.id,
            "the match in a question leads"
        );
        assert_eq!(hits[1].summary.id, mentioned.id);
    }

    #[test]
    fn an_excerpt_is_one_line_from_around_the_match_and_the_cut_is_shown_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut long = session("explain the download");
        long.turns[1].text = format!(
            "the pack is fetched once,   from the catalog\n\nand {} until the needle sits near the \
             middle of an answer that keeps going far past what the row has room to show",
            "padding ".repeat(40)
        );
        write_in(dir.path(), &long).unwrap();

        let hit = &search_in(dir.path(), "the needle sits")[0];
        assert_eq!(hit.role, Role::Assistant);
        assert!(hit.excerpt.starts_with(ELLIPSIS), "{}", hit.excerpt);
        assert!(hit.excerpt.ends_with(ELLIPSIS), "{}", hit.excerpt);
        assert!(
            !hit.excerpt.contains('\n') && !hit.excerpt.contains("  "),
            "{}",
            hit.excerpt
        );
        assert!(
            hit.excerpt.chars().count() <= 2 * EXCERPT_PAD + 2 + "the needle sits".len(),
            "{}",
            hit.excerpt
        );
    }

    #[test]
    fn a_blank_or_absent_query_finds_nothing_instead_of_everything() {
        let dir = tempfile::tempdir().unwrap();
        write_in(dir.path(), &session("kept")).unwrap();
        assert!(search_in(dir.path(), "   ").is_empty());
        assert!(search_in(dir.path(), "no such words").is_empty());
    }

    #[test]
    fn a_search_reads_the_same_files_the_sidebar_lists_and_ignores_the_broken_one() {
        let dir = tempfile::tempdir().unwrap();
        write_in(dir.path(), &session("the catalog pin")).unwrap();
        std::fs::write(dir.path().join("broken.json"), b"{ not json").unwrap();

        assert_eq!(list_in(dir.path()).len(), 1);
        assert_eq!(search_in(dir.path(), "catalog pin").len(), 1);
    }
}
