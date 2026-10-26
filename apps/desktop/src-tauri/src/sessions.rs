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
//! *   **Only what happened.** A session holds the prompt/answer pairs the history replays, because
//!     that is what makes reopening one mean something: the next prompt is answered from the same
//!     exchange. It also holds the rows the transcript drew while the run was up — the tool calls,
//!     the edits and their hunks, the notices and the refusals — because those are not a rendering
//!     of the exchange but a record of what the run *did*, and a reopened conversation that showed
//!     only the words would claim the model answered without touching anything. The reasoning stream
//!     is still deliberately never written back (Bonsai-demo#183), and neither is a picture's bytes
//!     twice over: a filed row names the turn's image by its place in `turns`, which already holds it.
//! *   **Nothing invented.** The workspace name is the folder the run worked in — the project picked in
//!     Settings, or the root the host resolved with no pick — the title is the user's own first
//!     sentence cut short, and the model is the label the run printed.
//!     The rows are the window's own, handed over at the end of a turn rather than reconstructed here.
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

/// Where a deleted conversation waits instead of vanishing: a subdirectory of the conversation
/// directory, so it lives in the store the app already owns and is measured as part of it on the
/// Storage page, and so no id can name a path that reaches outside it.
const TRASH: &str = "trash";

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
    /// The transcript as the window drew it, in order: turn rows interleaved with the run's steps.
    ///
    /// Held as the JSON the window sent rather than a Rust shape of this module's inventing, because
    /// the row model belongs to the transcript and its i18n keys to the catalogs — a typed copy here
    /// would be a second definition of `Step` free to drift from the first. A turn row's `image` is
    /// stripped by the sender and re-joined from [`Session::turns`] on the way back, so a filed
    /// picture is stored once.
    ///
    /// Additive, so `SESSION_SCHEMA_VERSION` stays where it is: a build from before this field
    /// ignores it and reopens the conversation as the exchange alone, which is what it did anyway.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<serde_json::Value>,
    /// Whether the row holds its place at the head of the list whatever else happened.
    ///
    /// Additive like `rows`, and for the same reason: a build from before this field reads the file
    /// fine and simply sorts the conversation where it always did. Nothing else about it changes —
    /// a pinned conversation still files turns, still goes to the Trash when deleted, and stays
    /// pinned when it does, so unpinning is not something a delete can do for you.
    ///
    /// Left out of the document entirely when it is `false`, so the files this build writes for an
    /// ordinary conversation are byte for byte what the build before the field wrote.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// The OpenCode session this conversation continues in, when its turns are answered by the
    /// OpenCode app on this Mac. Stored so reopening the conversation tomorrow continues the same
    /// exchange instead of asking a fresh session what the window still shows. Additive for the same
    /// reason `rows` is: an older build ignores what it does not know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opencode_session: Option<String>,
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
    /// Carried so the row can draw the pin it is filed with. The list is already sorted pinned-first,
    /// so a row that does not show it would be pinned by a rule the sidebar hides.
    pub pinned: bool,
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
            pinned: session.pinned,
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
            rows: Vec::new(),
            pinned: false,
            opencode_session: None,
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

    /// Replace the filed transcript with the rows the window is holding.
    ///
    /// A whole-list write rather than an append, because the window is the only thing that knows the
    /// order its rows came in and what it folded away: appending would need this module to guess
    /// where the last filing stopped, and a guess about a transcript is how rows end up duplicated
    /// under an answer.
    pub fn set_rows(&mut self, rows: Vec<serde_json::Value>) {
        self.rows = rows;
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

/// What `skip_serializing_if` wants for a flag whose absence means the same as `false`.
fn is_false(value: &bool) -> bool {
    !*value
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

/// The Trash beside the filed conversations, holding the ones the sidebar stopped listing.
fn trash_dir(dir: &Path) -> PathBuf {
    dir.join(TRASH)
}

/// The folder a conversation belongs to, named the way the status card names it: the last component
/// of the folder a run works in. That is the project folder picked in Settings when there is one, and
/// otherwise the root the host resolved — the checkout's directory in a development build, the data
/// directory an installed app mirrored its catalog into. Sessions already filed keep the name they
/// were filed under; picking another folder groups the new work under it rather than rewriting them.
pub fn workspace_label(app: &AppHandle, settings: &freecode_config::Settings) -> String {
    crate::paths::project_root(Some(app), settings)
        .ok()
        .or_else(|| crate::paths::workspace_root(Some(app)).ok())
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

/// Change what the window asks to change, and write the whole conversation back.
///
/// Read-modify-write rather than a text patch of the file, because the file *is* the record and a
/// partial rewrite of it can leave a conversation that reads as nothing. What it costs is a re-encode
/// of a few dozen kilobytes, which is what one click on a row is worth.
///
/// Deliberately not touching `updated_at_ms`: that field is what the list sorts by and what the row
/// prints, so renaming a conversation from last month would otherwise move it to the top of the list
/// and claim it happened now.
fn revise(dir: &Path, id: &str, change: impl FnOnce(&mut Session)) -> Result<Summary, String> {
    let mut session = read_in(dir, id)?;
    change(&mut session);
    write_in(dir, &session)?;
    Ok(session.summary())
}

/// Name a conversation what the user calls it rather than the first sentence it began with.
///
/// The name sticks because nothing recomputes it: `Session::new` titles a conversation once and every
/// later filing keeps the title the file already held. Not clipped to [`TITLE_MAX`] the way a
/// generated one is — that limit exists so a pasted stack trace does not become a label nobody chose,
/// and a name someone typed is a name they chose at exactly the length they chose it.
pub fn rename_in(dir: &Path, id: &str, title: &str) -> Result<Summary, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("a conversation needs a name to show on its row".into());
    }
    let title = title.to_string();
    revise(dir, id, move |session| session.title = title)
}

pub fn rename(id: &str, title: &str) -> Result<Summary, String> {
    let dir = dir().ok_or("this platform gives the app no conversation directory")?;
    rename_in(&dir, id, title)
}

/// Hold a conversation at the head of the list, or let it fall back into the newest-first order.
pub fn set_pinned_in(dir: &Path, id: &str, pinned: bool) -> Result<Summary, String> {
    revise(dir, id, move |session| session.pinned = pinned)
}

pub fn set_pinned(id: &str, pinned: bool) -> Result<Summary, String> {
    let dir = dir().ok_or("this platform gives the app no conversation directory")?;
    set_pinned_in(&dir, id, pinned)
}

/// The date a filing's wall-clock milliseconds fall on, in UTC, as `YYYY-MM-DD`.
///
/// The civil-from-days conversion is the standard one — the epoch shifted to a 1 March year so the
/// leap day lands last — because this crate carries no calendar dependency, and an exported history
/// with no date on it is a history that cannot be told apart from the next one.
fn civil_from_epoch(ms: u64) -> (i64, u32, u32) {
    let days = (ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

impl Session {
    /// The conversation as a markdown document: its exchange in order, with the run's steps named
    /// under the answer they belong to.
    ///
    /// The turns are what an export has to carry, because they are what the engine was told and what
    /// it said back. The rows are listed as the one-line shape the transcript drew them in — the tool
    /// a call ran and the path it touched — rather than re-rendered here, because a second copy of
    /// the row model would be a second definition of it, and a picture's bytes stay out for the same
    /// reason they stay out of a filed row: the turn already holds them.
    pub fn markdown(&self) -> String {
        let (year, month, day) = civil_from_epoch(self.updated_at_ms);
        let mut out = String::new();
        out.push_str(&format!("# {}\n\n", self.title));
        out.push_str(&format!("- Workspace: {}\n", self.workspace));
        out.push_str(&format!("- Model: {}\n", self.model));
        out.push_str(&format!("- Last turn: {year:04}-{month:02}-{day:02}\n"));
        out.push_str(&format!("- Turns: {}\n", self.turns.len()));
        if self.pinned {
            out.push_str("- Pinned\n");
        }
        for turn in &self.turns {
            let who = match turn.role {
                Role::User => "You",
                Role::Assistant => "FreeCode",
            };
            out.push_str(&format!("\n## {who}\n\n{}\n", turn.text));
            if turn.image.is_some() {
                out.push_str("\n_(an image was attached to this prompt)_\n");
            }
        }
        if !self.rows.is_empty() {
            out.push_str("\n## What the runs did\n\n");
            for row in &self.rows {
                for line in step_lines(row, 0) {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
        out
    }
}

/// One transcript row as lines of the export: a step's kind, the path or command it names, and what
/// its diff left behind.
///
/// A turn row is the exchange and is already written above, so it is skipped here. A notice carries
/// only a catalog code and its parameters, which are the interface's sentence rather than anything the
/// run did, so it goes out as the code. The rows nest one deep — a batch header over the calls it
/// folded — and `depth` is what indents the children under it.
fn step_lines(row: &serde_json::Value, depth: usize) -> Vec<String> {
    let Some(kind) = row.get("kind").and_then(serde_json::Value::as_str) else {
        return Vec::new();
    };
    if kind == "turn" {
        return Vec::new();
    }
    let pad = "  ".repeat(depth);
    let named = ["subject", "detail"]
        .iter()
        .find_map(|key| row.get(*key).and_then(serde_json::Value::as_str));
    let status = row.get("status").and_then(serde_json::Value::as_str);
    let mut line = format!("{pad}- `{kind}`");
    if let Some(named) = named {
        line.push_str(&format!(" {named}"));
    }
    if let Some(status) = status.filter(|s| *s != "ok") {
        line.push_str(&format!(" ({status})"));
    }
    if let Some(diff) = row.get("diff") {
        let added = diff.get("added").and_then(serde_json::Value::as_u64);
        let removed = diff.get("removed").and_then(serde_json::Value::as_u64);
        if let (Some(added), Some(removed)) = (added, removed) {
            line.push_str(&format!(" (+{added}/-{removed})"));
        }
    }
    let mut lines = vec![line];
    if let Some(children) = row.get("children").and_then(serde_json::Value::as_array) {
        for child in children {
            lines.extend(step_lines(child, depth + 1));
        }
    }
    lines
}

/// Move one conversation's file into the Trash.
///
/// A file that is not there answers `Ok` rather than an error: another window can have deleted the
/// same row a moment before, and the state the user asked for — that conversation is off the list —
/// holds either way. An error there would be a message about a bookkeeping race the row cannot show.
///
/// The id is checked by the same rule a read is, so this moves inside the session directory and
/// nowhere else, whatever the window sends. A file this build cannot parse is still moved: the
/// conversation it holds is unreadable here, and "throw this away" is the one instruction that needs
/// no reading.
///
/// It is a move rather than an unlink because the file is the only copy of the exchange, and the
/// press that throws it away is a glyph that appears on hover with no undo behind it. The Trash is
/// that undo. The name is kept, and a second copy of a conversation the Trash already holds is
/// numbered rather than written over — replacing the older copy is the loss this exists to prevent.
pub fn delete_in(dir: &Path, id: &str) -> Result<(), String> {
    let id = safe_id(id)?;
    let from = dir.join(format!("{id}.json"));
    if !from.exists() {
        return Ok(());
    }
    let to = trash_dir(dir);
    std::fs::create_dir_all(&to).map_err(|e| format!("creating {} failed: {e}", to.display()))?;
    let mut target = to.join(format!("{id}.json"));
    let mut taken = 1;
    while target.exists() {
        target = to.join(format!("{id}-{taken}.json"));
        taken += 1;
    }
    std::fs::rename(&from, &target).map_err(|e| format!("deleting {id} failed: {e}"))
}

/// One conversation waiting in the Trash: the fields a row is named by, the size its file takes on
/// the machine, and the name that file goes by — which is not always the conversation's id, since a
/// second deletion of one is numbered.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trashed {
    #[serde(flatten)]
    pub summary: Summary,
    pub name: String,
    pub bytes: u64,
}

/// Everything in the Trash, newest first.
///
/// A file that does not parse is skipped by the same rule the live list uses — one bad file must not
/// empty the section — and is left where it is rather than removed, because this build cannot tell
/// what it was.
pub fn list_trashed_in(dir: &Path) -> Vec<Trashed> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(trash_dir(dir)) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(session) = parse(&bytes) else {
            continue;
        };
        found.push(Trashed {
            summary: session.summary(),
            name: name.to_string(),
            bytes: entry.metadata().map(|meta| meta.len()).unwrap_or_default(),
        });
    }
    found.sort_by_key(|held| std::cmp::Reverse(held.summary.updated_at_ms));
    found
}

/// Put a trashed conversation back on the list, under the id it was filed with.
///
/// The file is read before it moves, so a name the window invents cannot land a conversation in the
/// list that was never filed, and a conversation that is already back is refused rather than
/// duplicated — two rows for one exchange is the same file answering twice.
pub fn restore_in(dir: &Path, name: &str) -> Result<Summary, String> {
    let name = safe_id(name)?;
    let from = trash_dir(dir).join(format!("{name}.json"));
    let bytes = match std::fs::read(&from) {
        Ok(bytes) => bytes,
        // The second press lands here: the first moved the file out, so there is nothing left to put
        // back. That is the state the press asked for, and it says so rather than quoting an errno.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("{name} is not in the Trash"))
        }
        Err(e) => return Err(format!("reading {} failed: {e}", from.display())),
    };
    let session = parse(&bytes)
        .map_err(|why| format!("{} is not a readable conversation: {why}", from.display()))?;
    let to = dir.join(format!("{}.json", session.id));
    if to.exists() {
        return Err(format!("{} is already back on the list", session.title));
    }
    std::fs::rename(&from, &to).map_err(|e| format!("restoring {name} failed: {e}"))?;
    Ok(session.summary())
}

/// Remove one trashed conversation's file for good. This is the only thing in this module that
/// unlinks a conversation, and the only way the bytes a delete moved out of the list leave the
/// machine.
pub fn purge_in(dir: &Path, name: &str) -> Result<(), String> {
    let name = safe_id(name)?;
    match std::fs::remove_file(trash_dir(dir).join(format!("{name}.json"))) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("deleting {name} failed: {e}")),
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
    // Pinned first, then newest. The id breaks a tie so that two conversations filed in the same
    // millisecond keep one order across two windows, rather than the order the directory was walked.
    found.sort_by(|a, b| {
        (!a.pinned, std::cmp::Reverse(&a.updated_at_ms), &a.id).cmp(&(
            !b.pinned,
            std::cmp::Reverse(&b.updated_at_ms),
            &b.id,
        ))
    });
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
        let mut session = Session::new("freecode", "Ternary Bonsai 1.7B (PQ2_0)", prompt);
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
        assert_eq!(read.workspace, "freecode");
    }

    #[test]
    fn a_filed_transcript_comes_back_with_the_run_that_made_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut conversation = session("make me a page");
        // The window's own row shapes, held as the JSON it sent rather than reinterpreted here.
        conversation.set_rows(vec![
            serde_json::json!({ "id": "t0", "kind": "turn", "role": "user", "text": "make me a page" }),
            serde_json::json!({ "id": "s0", "kind": "createFile", "subject": "index.html" }),
            serde_json::json!({ "id": "t1", "kind": "turn", "role": "assistant", "text": "an answer" }),
        ]);
        write_in(dir.path(), &conversation).unwrap();

        let read = read_in(dir.path(), &conversation.id).unwrap();
        assert_eq!(read.rows.len(), 3);
        assert_eq!(read.rows[1]["kind"], "createFile");
        assert_eq!(read.rows[1]["subject"], "index.html");
    }

    #[test]
    fn a_conversation_with_no_rows_is_the_file_this_field_never_existed_in() {
        // `skip_serializing_if` is what lets an older build read a newer file and lets a newer build
        // leave a text-only conversation exactly as small as it was. A `rows: []` written into every
        // file would be noise in all of them and a shape change in the one that matters.
        let conversation = session("just a question");
        let filed = serde_json::to_value(&conversation).unwrap();
        assert!(
            filed.get("rows").is_none(),
            "an empty transcript was written: {filed}"
        );
    }

    #[test]
    fn a_file_written_before_the_transcript_field_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let conversation = session("asked in an older build");
        write_in(dir.path(), &conversation).unwrap();
        let path = dir.path().join(format!("{}.json", conversation.id));
        let filed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // This is the file a build from before the field wrote, and the one this build reads back:
        // no `rows` key at all, which `serde(default)` turns into the exchange alone.
        assert!(filed.get("rows").is_none(), "{filed}");

        let read = read_in(dir.path(), &conversation.id).unwrap();
        assert!(read.rows.is_empty());
        assert_eq!(read.turns.len(), 2);
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
    fn a_deleted_conversation_leaves_the_list_but_not_the_machine() {
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
        // And the one that went is off the list rather than off the disk: the press that throws a
        // conversation away is two clicks, and the file is the only copy of the exchange.
        let trashed = list_trashed_in(dir.path());
        assert_eq!(trashed.len(), 1);
        assert_eq!(trashed[0].summary.id, gone.id);
        assert_eq!(trashed[0].name, gone.id);
        assert!(trashed[0].bytes > 0);
        // The Trash is inside the conversation directory, so the Storage page's walk of that one
        // directory counts the bytes still held without learning about a fourth place.
        assert!(dir
            .path()
            .join(TRASH)
            .join(format!("{}.json", gone.id))
            .exists());
    }

    #[test]
    fn a_trashed_conversation_comes_back_whole_and_its_slot_is_freed() {
        let dir = tempfile::tempdir().unwrap();
        let gone = session("the one that was thrown away");
        write_in(dir.path(), &gone).unwrap();
        delete_in(dir.path(), &gone.id).unwrap();

        let back = restore_in(dir.path(), &gone.id).unwrap();

        assert_eq!(back.id, gone.id);
        assert_eq!(back.turn_count, gone.turns.len());
        assert_eq!(read_in(dir.path(), &gone.id).unwrap().turns, gone.turns);
        assert!(list_trashed_in(dir.path()).is_empty());
        // The second press has nothing left to move — the first took the file out — and says that
        // rather than quoting the errno it found. The row on the list is still the one it put back.
        let err = restore_in(dir.path(), &gone.id).unwrap_err();
        assert!(err.contains("is not in the Trash"), "{err}");
        assert_eq!(list_in(dir.path()).len(), 1);
    }

    #[test]
    fn a_trashed_copy_is_not_restored_onto_a_conversation_that_is_already_back() {
        let dir = tempfile::tempdir().unwrap();
        let gone = session("answered in two windows");
        write_in(dir.path(), &gone).unwrap();
        delete_in(dir.path(), &gone.id).unwrap();
        // The older copy is in the bin and the live file exists again: the second window filed a
        // turn into it. Moving the copy back would replace what that run wrote.
        write_in(dir.path(), &gone).unwrap();

        let err = restore_in(dir.path(), &gone.id).unwrap_err();

        assert!(err.contains("already back"), "{err}");
        assert!(dir
            .path()
            .join(TRASH)
            .join(format!("{}.json", gone.id))
            .exists());
        assert_eq!(read_in(dir.path(), &gone.id).unwrap().turns, gone.turns);
    }

    #[test]
    fn deleting_a_conversation_the_trash_already_holds_keeps_both_copies() {
        let dir = tempfile::tempdir().unwrap();
        let gone = session("deleted twice");
        write_in(dir.path(), &gone).unwrap();
        delete_in(dir.path(), &gone.id).unwrap();
        // The state that numbers a second copy is not the same row deleted twice — a restore frees
        // the name — but two windows over one store: this one files a turn into a conversation the
        // other one had already thrown away, so the id is on the list again while its older copy is
        // still in the bin.
        write_in(dir.path(), &gone).unwrap();
        delete_in(dir.path(), &gone.id).unwrap();

        let trashed = list_trashed_in(dir.path());
        assert_eq!(trashed.len(), 2, "{trashed:?}");
        // The older copy is untouched: overwriting it is the loss the numbering exists to prevent.
        assert!(trashed.iter().any(|held| held.name == gone.id));
        assert!(trashed.iter().any(|held| held.name != gone.id));
        for held in &trashed {
            assert_eq!(held.summary.id, gone.id);
        }
    }

    #[test]
    fn purging_is_the_press_that_actually_loses_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let gone = session("purged");
        write_in(dir.path(), &gone).unwrap();
        delete_in(dir.path(), &gone.id).unwrap();

        purge_in(dir.path(), &gone.id).unwrap();

        assert!(list_trashed_in(dir.path()).is_empty());
        assert!(!dir.path().join(format!("{}.json", gone.id)).exists());
        // A name already purged answers `Ok` for the same reason a delete of a missing file does.
        purge_in(dir.path(), &gone.id).unwrap();
    }

    #[test]
    fn a_trash_name_carrying_a_path_reaches_nothing_outside_the_session_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("settings.json");
        std::fs::write(&outside, b"secret").unwrap();

        for name in ["../../settings", "/etc/passwd", "..", "a/b", ""] {
            assert!(restore_in(dir.path(), name).is_err(), "{name}");
            assert!(purge_in(dir.path(), name).is_err(), "{name}");
        }
        assert!(outside.exists());
    }

    #[test]
    fn the_trash_is_never_listed_as_a_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let gone = session("in the bin");
        write_in(dir.path(), &gone).unwrap();
        delete_in(dir.path(), &gone.id).unwrap();

        // The directory is a sibling of the JSON files the list walks, so the list must not read it
        // as a conversation — and the one that is filed still is.
        assert_eq!(list_in(dir.path()).len(), 0);
        write_in(dir.path(), &session("still here")).unwrap();
        assert_eq!(list_in(dir.path()).len(), 1);
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
        let mut only = Session::new("freecode", "m", "what did you say?");
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

    #[test]
    fn renaming_replaces_the_name_and_leaves_the_rest_of_the_record_alone() {
        let dir = tempfile::tempdir().unwrap();
        let first = session("why does the loop stop early");
        let (id, turns, filed_at, workspace) = (
            first.id.clone(),
            first.turns.len(),
            first.updated_at_ms,
            first.workspace.clone(),
        );
        write_in(dir.path(), &first).unwrap();

        let after = rename_in(dir.path(), &id, "Round budget").unwrap();

        assert_eq!(after.title, "Round budget");
        let read = read_in(dir.path(), &id).unwrap();
        assert_eq!(read.title, "Round budget");
        // The rename is a name, not a new conversation: the exchange, the folder and the moment it
        // last happened all stay where the row said they were.
        assert_eq!(read.turns.len(), turns);
        assert_eq!(read.workspace, workspace);
        assert_eq!(read.updated_at_ms, filed_at);
    }

    #[test]
    fn a_name_of_nothing_is_refused_rather_than_a_row_with_no_label() {
        let dir = tempfile::tempdir().unwrap();
        let first = session("kept");
        write_in(dir.path(), &first).unwrap();
        assert!(rename_in(dir.path(), &first.id, "   ").is_err());
        assert_eq!(read_in(dir.path(), &first.id).unwrap().title, first.title);
    }

    #[test]
    fn a_pinned_conversation_leads_the_list_however_new_the_rest_are() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = session("the old one");
        old.updated_at_ms = 1_000;
        let mut mid = session("the pinned one");
        mid.updated_at_ms = 2_000;
        mid.pinned = true;
        let mut fresh = session("the newest one");
        fresh.updated_at_ms = 3_000;
        for session in [&old, &mid, &fresh] {
            write_in(dir.path(), session).unwrap();
        }

        let titles: Vec<_> = list_in(dir.path()).into_iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            vec!["the pinned one", "the newest one", "the old one"]
        );

        // Unpinning lets it fall back into the order rather than leaving it stranded at the top.
        set_pinned_in(dir.path(), &mid.id, false).unwrap();
        let titles: Vec<_> = list_in(dir.path()).into_iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            vec!["the newest one", "the pinned one", "the old one"]
        );
    }

    #[test]
    fn pinning_does_not_claim_a_conversation_happened_now() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = session("kept");
        first.updated_at_ms = 1_000;
        write_in(dir.path(), &first).unwrap();

        let after = set_pinned_in(dir.path(), &first.id, true).unwrap();
        assert_eq!(after.updated_at_ms, 1_000);
        assert!(after.pinned);
    }

    #[test]
    fn a_file_from_before_the_pin_still_reads_and_starts_unpinned() {
        let dir = tempfile::tempdir().unwrap();
        let first = session("kept");
        write_in(dir.path(), &first).unwrap();
        // The bytes a build from before the field wrote, which an unpinned conversation still is: no
        // `pinned` key anywhere in the document.
        let path = dir.path().join(format!("{}.json", first.id));
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("pinned"));
        assert!(!read_in(dir.path(), &first.id).unwrap().pinned);

        // The key appears the moment it means something, and a build that has never heard of it
        // ignores an unknown field rather than failing to read the file.
        set_pinned_in(dir.path(), &first.id, true).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"pinned\": true"));
    }

    #[test]
    fn an_export_carries_the_exchange_in_order_and_the_steps_under_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = session("what pins the engine?");
        first.set_rows(vec![
            serde_json::json!({ "id": "t1", "kind": "turn", "role": "user", "text": "what pins the engine?" }),
            serde_json::json!({
                "id": "b1",
                "kind": "toolBatch",
                "children": [
                    { "id": "s1", "kind": "readFile", "subject": "engine.rs", "status": "ok" },
                    { "id": "s2", "kind": "editFile", "subject": "pin.rs", "status": "error", "diff": { "added": 7, "removed": 2 } }
                ]
            }),
        ]);
        write_in(dir.path(), &first).unwrap();

        let body = read_in(dir.path(), &first.id).unwrap().markdown();

        assert!(body.starts_with("# what pins the engine?\n"));
        // Both sides, in the order they happened, under the names the window uses for them.
        let asked = body.find("## You").unwrap();
        let answered = body.find("## FreeCode").unwrap();
        assert!(asked < answered);
        assert!(body.contains("an answer"));
        // A turn row is the exchange and is written above once, not again as a step.
        assert_eq!(body.matches("## You").count(), 1);
        // A batch's children are listed under it, and the one that failed says so with its counts.
        assert!(body.contains("- `readFile` engine.rs"));
        assert!(body.contains("  - `editFile` pin.rs (error) (+7/-2)"));
    }

    #[test]
    fn a_filing_moment_becomes_the_utc_day_it_falls_on() {
        // The epoch itself, and a date on either side of a leap day, so the 1 March era shift and the
        // century rule are both answered rather than assumed.
        assert_eq!(civil_from_epoch(0), (1970, 1, 1));
        assert_eq!(civil_from_epoch(1_735_689_600_000), (2025, 1, 1));
        assert_eq!(civil_from_epoch(1_709_164_800_000), (2024, 2, 29));
        assert_eq!(civil_from_epoch(1_704_067_200_000), (2024, 1, 1));
    }
}
