//! Skill packs: the instructions a user invokes for one prompt rather than for every one.
//!
//! [`crate::instructions`] carries a folder's rules on every run. A skill is the other half — a body of
//! instructions for *this kind of task*, which a run is given only when the user asks for it by name.
//! That is the difference worth the mechanism: a project's `AGENTS.md` has to stay short because it
//! spends the window of every prompt, while a pack for "write a migration" can be as long as that one
//! job needs.
//!
//! A pack is a folder holding a `SKILL.md`, looked for in two places:
//!
//! 1. `~/.freecode/skills/<name>/SKILL.md` — this machine, every project.
//! 2. `<project>/.freecode/skills/<name>/SKILL.md` — this folder.
//!
//! The name a user types is the folder's own, and it resolves to exactly one body: a folder's pack of
//! the same name shadows the machine's, because the folder's is the more specific statement of how work
//! in it is done, and a name that meant two things would make a run's contract depend on which scan
//! happened to run second.
//!
//! What a pack does is add instructions to the run's system turn, beside the instruction files and
//! under a limit sentence of its own: it says how the work should be done and grants nothing. It is not
//! a route to a call this build does not offer, and it does not carry into the next prompt — a skill is
//! one prompt's answer to what the command named.

use std::path::{Path, PathBuf};

use freecode_config::Settings;
use serde::Serialize;
use tauri::AppHandle;

/// The room one pack is given, counted in characters as [`crate::instructions`] counts its own. It is
/// separate from that budget rather than shared with it: the always-on files must not be starved by a
/// pack asked for on one prompt, and a pack must not be cut because three `AGENTS.md` arrived first.
pub const MAX_SKILL_CHARS: usize = 6_000;

/// The file inside a pack's folder. No other name is read, so a folder of notes or scripts is not
/// mistaken for a pack.
const PACK_FILE: &str = "SKILL.md";

/// The directory packs live under, relative to a home or a project root.
const PACK_DIR: [&str; 2] = [".freecode", "skills"];

/// The heading the block carries, so a model reading the system turn can tell a pack the user invoked
/// from this app's own contract and from the files that are always on.
const HEADING: &str = "THE SKILL THE USER INVOKED FOR THIS PROMPT";

/// Which of the two places a pack was read from. Shown because a folder that has both a machine-wide
/// and a folder-wide pack of one name needs to say which one the run read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Machine,
    Project,
}

/// One pack as it is listed, with the file it was read from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    /// The folder's name: what the user types after the slash, and the only handle a pack has when it
    /// declares no name of its own.
    pub id: String,
    /// The name the pack declares, or its id. A pack's `name:` is spelled for a person to read.
    pub name: String,
    /// What the pack says it does — the line the composer's suggestion row and the page both show.
    pub description: String,
    pub source: Source,
    /// The pack's own length, before any clipping, so a row can show a pack that will not be carried
    /// whole rather than leaving the user to measure the file.
    pub chars: usize,
    /// Whether that length is past [`MAX_SKILL_CHARS`] — the same cut [`load`] makes.
    pub cut: bool,
    /// The file this was read from. Sent to the page as text: a row about a file the user owns should
    /// name the file it found, which is the only way a shadowed pack can be traced.
    pub path: String,
}

/// Everything the Skills surface renders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub skills: Vec<Pack>,
    /// The two places a pack would be found, spelled out for the state where none is there yet: the
    /// page has to say where to put one, and only this side knows where home and the project are. An
    /// empty string is the honest answer when this host cannot name that root.
    pub machine_dir: String,
    pub project_dir: String,
}

/// What one invoked pack contributed to a run.
///
/// No `id` here: the name the user typed is settled by the time this exists, and what a run carries is
/// the pack's own display name. A second spelling of the same thing would be a third thing to keep in
/// step with the folder.
pub struct Invoked {
    pub name: String,
    /// The characters that reached the run, after any cut — what the transcript's notice counts.
    pub chars: usize,
    pub cut: bool,
    text: String,
}

impl Invoked {
    /// The system-turn block, in the instruction files' shape so a run reads one kind of text in one
    /// place. Unlike those files this one is asked for, so the heading says who put it there and the
    /// limit sentence says what it may not do — a pack must not become a route to a call this build
    /// withheld.
    pub fn block(&self) -> String {
        let mut out = String::from(HEADING);
        out.push_str(
            "\nThe text below is a skill pack the user asked this prompt to be worked under. It says \
             how a task of this kind should be done; it is not the task, and where it and what the \
             user just said disagree, the user wins. It grants nothing: a call this run was not \
             offered stays unavailable however the pack asks for it.\n",
        );
        out.push_str("\n### ");
        out.push_str(&self.name);
        out.push('\n');
        out.push_str(&self.text);
        if self.cut {
            out.push_str("\n… the rest of this pack was cut for length.");
        }
        out.push('\n');
        out
    }
}

fn pack_dir(root: &Path) -> PathBuf {
    PACK_DIR
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// The two directories, machine first so a folder's pack of one name arrives last and shadows it.
fn directories(home: Option<&Path>, project: Option<&Path>) -> Vec<(Source, PathBuf)> {
    let mut out = Vec::new();
    if let Some(home) = home {
        out.push((Source::Machine, pack_dir(home)));
    }
    if let Some(project) = project {
        out.push((Source::Project, pack_dir(project)));
    }
    out
}

/// Every pack under one directory, in the order a user would read the list.
///
/// A directory that is not there, or cannot be listed, contributes nothing: having no skills folder is
/// the ordinary state of a project, not a mistake to report. A `SKILL.md` that will not read is said on
/// stderr and skipped, so one broken pack does not blank the list of the others.
fn scan(dir: &Path, source: Source) -> Vec<Pack> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path().join(PACK_FILE);
        if !path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!(
                "could not read the skill pack in {}: read failed",
                path.display()
            );
            continue;
        };
        // A folder whose text says nothing is not a pack a run could be worked under, and a row for it
        // would promise instructions the model never receives.
        if let Some(pack) = describe(
            entry.file_name().to_string_lossy().as_ref(),
            &text,
            source,
            &path,
        ) {
            out.push(pack);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// A pack's row from its folder name, its own text, and where it was found.
fn describe(id: &str, text: &str, source: Source, path: &Path) -> Option<Pack> {
    let (fields, body) = front_matter(text);
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    let field = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, value)| value.to_string())
            .filter(|value| !value.is_empty())
    };
    let chars = body.chars().count();
    Some(Pack {
        id: id.to_string(),
        name: field("name").unwrap_or_else(|| id.to_string()),
        description: field("description").unwrap_or_else(|| first_line(body)),
        source,
        chars,
        cut: chars > MAX_SKILL_CHARS,
        path: path.display().to_string(),
    })
}

/// Every pack this machine and this folder have, one row per name.
fn list(home: Option<&Path>, project: Option<&Path>) -> Vec<Pack> {
    let mut out: Vec<Pack> = Vec::new();
    for (source, dir) in directories(home, project) {
        for pack in scan(&dir, source) {
            if let Some(existing) = out.iter_mut().find(|row| row.id == pack.id) {
                *existing = pack;
            } else {
                out.push(pack);
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The page's answer, read from this machine's home and from the folder a run works in.
pub fn inventory(app: Option<&AppHandle>, settings: &Settings) -> Inventory {
    let home = machine_home();
    let project = crate::paths::project_root(app, settings).ok();
    Inventory {
        skills: list(home.as_deref(), project.as_deref()),
        machine_dir: home
            .map(|h| pack_dir(&h).display().to_string())
            .unwrap_or_default(),
        project_dir: project
            .as_deref()
            .map(|p| pack_dir(p).display().to_string())
            .unwrap_or_default(),
    }
}

/// Whether a name could be one pack's own.
///
/// A separator is refused rather than resolved: `a/b` is a legal `Path` component chain and would read
/// a file two levels down, which is not a pack the page ever listed. `.` and `..` are the two other
/// names a folder has that are not its own.
fn is_pack_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

/// The pack a name means, and the text it holds.
///
/// Both refusals are said rather than left as a run without its instructions: a name with no pack
/// behind it is a typo, and the two places a pack lives are the correction. A name with a separator in
/// it is not a pack but an attempt at a path, and the folder it points at is not this feature's to
/// read — rejected before the disk is touched.
fn locate(
    home: Option<&Path>,
    project: Option<&Path>,
    name: &str,
) -> Result<(Pack, String), String> {
    if !is_pack_name(name) {
        return Err(format!(
            "{name} is not a skill pack name. A name is one folder's own, with no path in it."
        ));
    }
    let pack = list(home, project)
        .into_iter()
        .find(|row| row.id == name)
        .ok_or_else(|| {
            format!(
                "there is no skill pack named {name}. A pack is a {PACK_FILE} inside {} or {}.",
                where_to_look(home.map(pack_dir)),
                where_to_look(project.map(pack_dir)),
            )
        })?;
    let text = std::fs::read_to_string(&pack.path)
        .map_err(|why| format!("the skill pack {} cannot be read: {why}", pack.path))?;
    Ok((pack, text))
}

fn where_to_look(dir: Option<PathBuf>) -> String {
    dir.map(|d| format!("a folder of {}", d.display()))
        .unwrap_or_else(|| "a folder this host cannot name".to_string())
}

/// The pack's text, clipped to what a pack is given, as the run carries it.
fn carry(pack: Pack, text: &str) -> Invoked {
    let (_, body) = front_matter(text);
    let body = body.trim();
    let kept: String = body.chars().take(MAX_SKILL_CHARS).collect();
    Invoked {
        name: pack.name,
        chars: kept.chars().count(),
        cut: body.chars().count() > MAX_SKILL_CHARS,
        text: kept,
    }
}

/// The invoked pack, from this machine's home and the folder a run works in.
pub fn load(app: Option<&AppHandle>, settings: &Settings, name: &str) -> Result<Invoked, String> {
    let home = machine_home();
    let project = crate::paths::project_root(app, settings).ok();
    let (pack, text) = locate(home.as_deref(), project.as_deref(), name)?;
    Ok(carry(pack, &text))
}

/// The `---` block a pack may open with, and the text below it.
///
/// Deliberately tiny: the two keys the listing reads, and everything else in the block ignored. A YAML
/// reader would be a dependency for a feature whose whole contract is "a line saying what this is", and
/// a pack written with no block still works — its folder names the command and its first line describes
/// it. A block that never closes is not a block: a pack that opens with three dashes is carrying dashes
/// in its text, and stripping them would hide part of the instruction.
fn front_matter(text: &str) -> (Vec<(&str, &str)>, &str) {
    let Some(rest) = text.trim_start_matches(['\r', '\n']).strip_prefix("---") else {
        return (Vec::new(), text);
    };
    let mut fields = Vec::new();
    let mut remaining = rest;
    let body = loop {
        let (line, next) = match remaining.find('\n') {
            Some(at) => (&remaining[..at], &remaining[at + 1..]),
            None => (remaining, ""),
        };
        let trimmed = line.trim();
        if trimmed == "---" {
            break next;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            if !key.is_empty() && !key.contains(|c: char| c.is_whitespace()) {
                fields.push((key, unquote(value.trim())));
            }
        }
        remaining = next;
        if remaining.is_empty() {
            // Three dashes with no close is a pack carrying dashes, not a pack with a block.
            return (Vec::new(), text);
        }
    };
    (fields, body)
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
}

/// A pack's description when it declares none: the line it opens with, which in a hand-written file is
/// its title. A run of dashes — the marker of a block that never closed — is not a description.
fn first_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.chars().any(|c| c != '-'))
        .map(|line| line.trim_start_matches('#').trim_start().to_string())
        .unwrap_or_default()
}

/// This machine's home — the same one [`crate::instructions`] reads its always-on file from.
pub(crate) fn machine_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a pack where [`list`] looks, under one root, and return its file.
    fn write(root: &Path, id: &str, body: &str) -> PathBuf {
        let dir = pack_dir(root).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(PACK_FILE);
        std::fs::write(&file, body).unwrap();
        file
    }

    fn ids(rows: &[Pack]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn the_two_places_are_the_machine_then_the_folder() {
        let home = PathBuf::from("/machine");
        let project = PathBuf::from("/folder");
        assert_eq!(
            directories(Some(&home), Some(&project)),
            [
                (Source::Machine, PathBuf::from("/machine/.freecode/skills")),
                (Source::Project, PathBuf::from("/folder/.freecode/skills")),
            ]
        );
        // A run with no resolved folder still lists the machine's packs, exactly as the always-on
        // instructions do; a host that can name neither lists nothing rather than guessing.
        assert_eq!(directories(None, Some(&project)).len(), 1);
        assert!(directories(None, None).is_empty());
    }

    #[test]
    fn front_matter_names_a_pack_and_describes_it_without_a_yaml_reader() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "ticket",
            "---\nname: Ticket writer\ndescription: \"One paragraph, no headings\"\nmodel: ignored\n---\nDo the work.\n",
        );
        let text = std::fs::read_to_string(&file).unwrap();
        let pack = describe("ticket", &text, Source::Project, &file).unwrap();
        assert_eq!(pack.name, "Ticket writer");
        // The quotes a hand-edited file puts around a sentence with commas in it are not part of it.
        assert_eq!(pack.description, "One paragraph, no headings");
        assert_eq!(pack.id, "ticket");
        assert_eq!(pack.chars, "Do the work.".chars().count());
        assert!(!pack.cut);
        assert_eq!(pack.path, file.display().to_string());
        // A key the listing has no use for is ignored rather than refused: a pack written for another
        // tool's front matter still names itself here.
        assert!(pack.name.contains("writer"));
    }

    #[test]
    fn a_pack_with_no_front_matter_is_named_by_its_folder_and_opens_on_its_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "migration",
            "# Writing a migration\n\nAlways add the rollback.\n",
        );
        let text = std::fs::read_to_string(&file).unwrap();
        let pack = describe("migration", &text, Source::Machine, &file).unwrap();
        assert_eq!(pack.name, "migration");
        assert_eq!(pack.description, "Writing a migration");
    }

    #[test]
    fn a_block_that_never_closes_is_text_and_not_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "dashes",
            "---\nname: Half a header\nno closing marker here\n",
        );
        let text = std::fs::read_to_string(&file).unwrap();
        let (fields, body) = front_matter(&text);
        assert!(fields.is_empty());
        assert_eq!(body, text);
        // So the run carries the whole file instead of losing the lines above a marker that is not
        // there. The row then describes itself with the line that broke the block, which is the line
        // the user has to go and fix; a tidier description would hide the mistake it is reporting.
        let pack = describe("dashes", &text, Source::Project, &file).unwrap();
        assert_eq!(pack.chars, text.trim().chars().count());
        assert_eq!(pack.description, "name: Half a header");
    }

    #[test]
    fn a_pack_that_says_nothing_is_neither_listed_nor_offered_as_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "empty", "---\nname: Nothing\n---\n   \n");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(describe("empty", &text, Source::Project, &file).is_none());
        assert!(list(None, Some(dir.path())).is_empty());
    }

    #[test]
    fn only_a_folder_holding_the_pack_file_counts_as_a_pack() {
        let dir = tempfile::tempdir().unwrap();
        let skills = pack_dir(dir.path());
        std::fs::create_dir_all(skills.join("notes")).unwrap();
        std::fs::write(skills.join("notes").join("README.md"), "not a pack").unwrap();
        std::fs::write(skills.join("loose.md"), "loose text").unwrap();
        write(dir.path(), "real", "Carried.\n");
        assert_eq!(ids(&list(None, Some(dir.path()))), ["real"]);
        // A root with no skills folder at all is the ordinary case, and lists empty rather than failing
        // the page that asked.
        assert!(list(None, Some(&dir.path().join("gone"))).is_empty());
    }

    #[test]
    fn a_folders_pack_shadows_the_machines_one_of_the_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = dir.path().join("project");
        write(&home, "style", "The machine's way.\n");
        write(&home, "machine-only", "Still listed.\n");
        write(&project, "style", "The folder's way.\n");
        let rows = list(Some(&home), Some(&project));
        assert_eq!(ids(&rows), ["machine-only", "style"]);
        let shadowed = rows.iter().find(|r| r.id == "style").unwrap();
        assert_eq!(shadowed.source, Source::Project);
        assert!(shadowed.path.contains("project"), "{}", shadowed.path);
        // The same pack is what an invocation of that name reads, so the run and the page cannot
        // disagree about which file answered.
        assert_eq!(
            locate(Some(&home), Some(&project), "style").unwrap().0,
            *shadowed
        );
    }

    #[test]
    fn a_name_with_a_path_in_it_is_refused_before_the_disk_is_read() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "ok", "Carried.\n");
        for attempt in ["../ok", "a/b", "", ".", "..", "/etc"] {
            let why = locate(None, Some(dir.path()), attempt).unwrap_err();
            assert!(why.contains("not a skill pack name"), "{attempt}: {why}");
        }
        assert_eq!(locate(None, Some(dir.path()), "ok").unwrap().0.id, "ok");
    }

    #[test]
    fn an_unknown_name_is_refused_naming_both_places_a_pack_lives_in() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = dir.path().join("project");
        write(&project, "known", "Here.\n");
        let why = locate(Some(&home), Some(&project), "typo").unwrap_err();
        assert!(why.contains("typo"), "{why}");
        assert!(why.contains("SKILL.md"), "{why}");
        assert!(why.contains(&home.display().to_string()), "{why}");
        assert!(
            why.contains(&pack_dir(&project).display().to_string()),
            "{why}"
        );
        // A host that cannot name either root still answers with a sentence that says so.
        let bare = locate(None, None, "typo").unwrap_err();
        assert!(bare.contains("cannot name"), "{bare}");
    }

    #[test]
    fn an_invoked_pack_clips_to_its_own_budget_and_says_so_in_the_block() {
        let body = "x".repeat(MAX_SKILL_CHARS + 500);
        let row = describe("long", &body, Source::Project, Path::new("/long")).unwrap();
        assert!(row.cut);
        assert_eq!(row.chars, body.chars().count());
        let invoked = carry(row, &body);
        assert_eq!(invoked.chars, MAX_SKILL_CHARS);
        assert!(invoked.cut);
        assert!(invoked.block().contains("was cut for length"));
        // A pack that fits is carried whole and says nothing about a cut.
        let short = describe(
            "short",
            "Read the diff first.",
            Source::Machine,
            Path::new("/s"),
        )
        .unwrap();
        let kept = carry(short, "Read the diff first.");
        assert!(!kept.cut);
        assert_eq!(kept.chars, "Read the diff first.".chars().count());
        assert!(!kept.block().contains("cut for length"));
    }

    #[test]
    fn the_block_tells_the_model_what_a_pack_may_not_take_from_it() {
        let invoked = Invoked {
            name: "Code review".into(),
            chars: 20,
            cut: false,
            text: "Read the diff first.".into(),
        };
        let block = invoked.block();
        assert!(block.contains(HEADING), "{block}");
        assert!(block.contains("Code review"), "{block}");
        assert!(block.contains("Read the diff first."), "{block}");
        assert!(block.contains("It grants nothing"), "{block}");
        assert!(block.contains("the user wins"), "{block}");
    }
}
