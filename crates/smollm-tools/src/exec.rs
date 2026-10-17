//! The workspace executors: four calls that read it and one that changes a file in it.
//!
//! Every one of them takes `(arguments, policy)` and goes through [`authorize`] first, so a call
//! cannot reach the disk without the sandbox having resolved its path, and the loop has no way to skip
//! that check. Shells and network calls are deliberately absent — they have no executor in this build,
//! which is why a run that carries tools offers
//! [`EXECUTABLE`](crate::registry::EXECUTABLE) and refuses anything else out loud instead of
//! pretending to try it.
//!
//! The one write here is `edit_file`, and it is the reason the sandbox's `Access::Write` exists: a
//! path outside the workspace root, or onto a protected name, is refused by [`authorize`] before this
//! file opens anything. What is left to the executor is the part a sandbox cannot decide — whether the
//! text the model quoted is actually in the file, and actually only once.
//!
//! Three budgets, and all three are about the *context window* rather than about safety:
//!   * [`MAX_RESULT_BYTES`] caps what goes back into the prompt. A tool result is prefilled on every
//!     later round of the run and replayed for every later turn of the conversation, so one 5 MB file
//!     read into a 32k-token window costs the rest of the task;
//!   * [`MAX_FILE_BYTES`] skips a file too big to be source, and [`MAX_WALK_ENTRIES`] bounds the walk;
//!   * [`SKIP_DIRS`] keeps build and vendor trees out of it, because descending into `target/` is the
//!     slowest possible way to find nothing.
//!
//! The sandbox is what refuses a path. These only decide how much of an allowed answer to send, and
//! they say what was left out rather than cutting it quietly.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use crate::{authorize, registry::is_executable, Access, SandboxPolicy, ToolsError};

/// Most result text one call may hand back, in bytes — about 3k tokens, against the smallest context
/// window this app starts a server with.
pub const MAX_RESULT_BYTES: usize = 12 * 1024;

/// A file past this size is not source the model should be reading, and reading it is the slowest way
/// to answer nothing. It is also what bounds one read's allocation.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Files one walk may collect before it stops and says so.
const MAX_WALK_ENTRIES: usize = 5_000;

/// How many entries or paths one answer may list.
const MAX_LISTED: usize = 200;

/// Matching lines `grep_search` reports before it stops.
const MAX_MATCHES: usize = 100;

/// The depth a walk descends. Past it the workspace is a build artefact, not a source tree.
const MAX_DEPTH: usize = 12;

/// Lines of the file shown around a change, either side. `grep_search` takes its context from the
/// call; an edit's hunk has no argument to ask with, and two is what makes a changed line
/// identifiable in the file it came from without reprinting the file.
const CTX_LINES: usize = 2;

/// Trees no read enumerates. A budget, not a security rule — the sandbox's denied names are what keep
/// `.env` and `.ssh` out of an answer.
const SKIP_DIRS: &[&str] = &[
    ".hg",
    ".svn",
    ".next",
    ".cache",
    ".venv",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
];

/// What one call changed on disk. Only `edit_file` produces one, and the counts are the replacement's
/// own arithmetic rather than a diff engine's: the lines it took out and the lines it put back, with
/// the lines it only passed over cancelled from both sides. For a call that swaps one block of text
/// for another that is exactly what a diff reports for the hunk, not an approximation of one.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    /// Workspace-relative, in forward slashes — the same spelling `glob` and `grep_search` answer
    /// with, so the model can hand it straight back to `read_file`.
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// How many copies of `old_string` were replaced. More than one only when the call asked for
    /// `replace_all`, which is the difference between "fixed a line" and "renamed a symbol".
    pub occurrences: u32,
}

/// What one call produced: a clause for the transcript row and the text the model is handed.
#[derive(Debug, Clone, PartialEq)]
pub struct Executed {
    /// The one line under the row's label — "307 of 5000 lines, capped here", "37 matches in 12 files".
    pub summary: String,
    /// The tool result, as the model sees it. Never longer than [`MAX_RESULT_BYTES`] plus the sentence
    /// that says what was left out.
    pub content: String,
    /// The file this call changed, for the run's diff. `None` for every read, which is what keeps a
    /// "+0 −0" pill off a run that touched nothing.
    pub edit: Option<Edit>,
}

/// Run one call. The single entry point: authorization, the executable gate, then the work.
///
/// Takes the tool id and its parsed arguments rather than a [`crate::ToolCall`], because the wire's
/// own call shape lives in `smollm-llama` and this crate has to stay testable without it.
pub fn execute(
    tool_id: &str,
    arguments: &Value,
    policy: &SandboxPolicy,
) -> Result<Executed, ToolsError> {
    let tool = authorize(tool_id, arguments, policy)?;
    if !is_executable(tool.id) {
        return Err(ToolsError::NoExecutor(tool.id.to_string()));
    }
    match tool.id {
        "read_file" => read_file(tool.id, arguments, policy),
        "list_dir" => list_dir(tool.id, arguments, policy),
        "glob" => find_files(tool.id, arguments, policy),
        "grep_search" => search_content(tool.id, arguments, policy),
        "edit_file" => edit_file(tool.id, arguments, policy),
        // Unreachable while [`registry::EXECUTABLE`](crate::registry::EXECUTABLE) and this match move
        // together, and a refusal rather than a panic if they ever do not.
        other => Err(ToolsError::NoExecutor(other.to_string())),
    }
}

/// Read a text file, optionally a line range of one.
///
/// The whole allowed file is held while the range is picked out of it, which is what makes the answer
/// able to state the file's real line count and to continue from any line the cap stopped it on.
fn read_file(
    tool: &str,
    arguments: &Value,
    policy: &SandboxPolicy,
) -> Result<Executed, ToolsError> {
    let label = required(tool, arguments, "path")?.to_string();
    let path = resolve(policy, &label, Access::Read)?;
    let text = read_text(tool, &path)?.ok_or_else(|| ToolsError::InvalidArguments {
        tool: tool.to_string(),
        reason: format!("{label} is not text this tool can read"),
    })?;
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return Ok(Executed {
            summary: "empty".to_string(),
            content: format!("{label} is empty."),
            edit: None,
        });
    }

    let from = number_arg(arguments, "start_line").unwrap_or(1).max(1);
    if from > total {
        return Ok(Executed {
            summary: format!("{total} lines"),
            content: format!(
                "Nothing to show: {label} ends at line {total} and the range asked for starts past line \
                 {total}."
            ),
            edit: None,
        });
    }
    let to = number_arg(arguments, "end_line")
        .unwrap_or(total)
        .min(total);
    if to < from {
        return Ok(Executed {
            summary: format!("nothing between {from} and {to}"),
            content: format!(
                "Nothing to show: the range ends at line {to}, before it starts at line {from}."
            ),
            edit: None,
        });
    }

    // Cut on a whole line, and name the line it stopped on: `start_line` on the next call continues
    // exactly where this one ran out.
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0usize;
    let mut next = None;
    for (i, line) in lines[from - 1..to].iter().enumerate() {
        let cost = line.len() + 1;
        if used + cost > MAX_RESULT_BYTES {
            next = Some(from + i);
            break;
        }
        used += cost;
        kept.push(line);
    }
    let last = from + kept.len() - 1;

    if kept.is_empty() {
        return Ok(Executed {
            summary: format!("line {from} alone passes the output cap"),
            content: format!(
                "Line {from} of {label} is longer than the {} bytes this build sends back, so nothing of \
                 it is shown. Search for the words instead of reading the line.",
                MAX_RESULT_BYTES
            ),
            edit: None,
        });
    }
    let whole = from == 1 && last >= total;
    let Some(stopped) = next else {
        return Ok(Executed {
            summary: if whole {
                format!("{total} lines")
            } else {
                format!("lines {from}–{last} of {total}")
            },
            content: kept.join("\n"),
            edit: None,
        });
    };
    Ok(Executed {
        summary: format!("lines {from}–{last} of {total}, capped here"),
        content: format!(
            "{}\n…output capped at {} bytes: line {stopped} of {total} and everything after it are not \
             shown. Ask again with start_line={stopped} to continue.",
            kept.join("\n"),
            MAX_RESULT_BYTES
        ),
        edit: None,
    })
}

/// List a directory, one entry per line, directories with a trailing `/`.
///
/// What the sandbox protects and what this file skips are counted rather than dropped silently: a
/// listing that hides a secret with nothing said about it teaches the model that the workspace is
/// smaller than it is.
fn list_dir(tool: &str, arguments: &Value, policy: &SandboxPolicy) -> Result<Executed, ToolsError> {
    let label = required(tool, arguments, "path")?.to_string();
    let path = resolve(policy, &label, Access::Read)?;
    let show_hidden = flag_arg(arguments, "include_hidden");

    let mut entries = Vec::new();
    let mut held_back = 0usize;
    let dir = fs::read_dir(&path).map_err(|e| io_error(tool, &label, &e))?;
    for entry in dir {
        let entry = entry.map_err(|e| io_error(tool, &label, &e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if (!show_hidden && name.starts_with('.'))
            || SKIP_DIRS.contains(&name.as_str())
            || policy.is_protected(&entry.path())
        {
            held_back += 1;
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let size = if is_dir {
            0
        } else {
            entry.metadata().map(|m| m.len()).unwrap_or_default()
        };
        entries.push((name, is_dir, size));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let listed = entries.len().min(MAX_LISTED);
    let mut lines: Vec<String> = entries[..listed]
        .iter()
        .map(|(name, is_dir, size)| {
            if *is_dir {
                format!("{name}/")
            } else {
                format!("{name} ({})", human_bytes(*size))
            }
        })
        .collect();
    if listed < entries.len() {
        lines.push(format!(
            "…{} more entries are not listed.",
            entries.len() - listed
        ));
    }
    if held_back > 0 {
        lines.push(format!(
            "…{held_back} entries are held back (hidden, protected, or build trees)."
        ));
    }
    Ok(Executed {
        summary: format!("{listed} of {} shown", entries.len() + held_back),
        content: if lines.is_empty() {
            format!("{label} holds nothing that may be listed.")
        } else {
            lines.join("\n")
        },
        edit: None,
    })
}

/// Match file paths against a glob, under `path` or the whole workspace.
fn find_files(
    tool: &str,
    arguments: &Value,
    policy: &SandboxPolicy,
) -> Result<Executed, ToolsError> {
    let pattern = required(tool, arguments, "pattern")?.to_string();
    let glob = compile(tool, "pattern", &pattern, glob::Pattern::new)?;
    let root = optional_root(arguments, policy)?;
    // A pattern that names a separator has to match the whole relative path and respect its `/`s; one
    // that does not is a file name, and finds the file wherever it lives in the tree.
    let literal_separator = pattern.contains('/');
    let opts = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: literal_separator,
        require_literal_leading_dot: false,
    };

    let mut skipped = 0usize;
    let files = walk(policy, &root, &mut skipped);
    let mut found: Vec<String> = Vec::new();
    for file in &files {
        let rel = relative(policy, file);
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if glob.matches_with(&rel, opts) || (!literal_separator && glob.matches_with(&name, opts)) {
            found.push(rel);
        }
    }

    let count = found.len();
    let mut content = found[..count.min(MAX_LISTED)].join("\n");
    if count > MAX_LISTED {
        content.push_str(&format!(
            "\n…{} matches are not listed.",
            count - MAX_LISTED
        ));
    }
    Ok(Executed {
        summary: if count == 1 {
            format!("1 file matches {pattern}")
        } else {
            format!("{count} files match {pattern}")
        },
        content: if content.is_empty() {
            format!("No file in the workspace matches {pattern}.")
        } else {
            with_skips(content, skipped)
        },
        edit: None,
    })
}

/// Search file contents by regular expression, with the file, the line number and the line.
fn search_content(
    tool: &str,
    arguments: &Value,
    policy: &SandboxPolicy,
) -> Result<Executed, ToolsError> {
    let pattern = required(tool, arguments, "pattern")?.to_string();
    let re = regex::RegexBuilder::new(&pattern)
        .case_insensitive(flag_arg(arguments, "ignore_case"))
        .build()
        .map_err(|e| ToolsError::InvalidArguments {
            tool: tool.to_string(),
            reason: format!("\"{pattern}\" is not a valid regular expression: {e}"),
        })?;
    let root = optional_root(arguments, policy)?;
    let filter = match text_arg(arguments, "glob") {
        Some(raw) => Some(compile(tool, "glob", raw, glob::Pattern::new)?),
        None => None,
    };
    let context = number_arg(arguments, "context_lines").unwrap_or(0).min(20);

    let mut skipped = 0usize;
    let files = walk(policy, &root, &mut skipped);
    let mut lines: Vec<String> = Vec::new();
    let mut matches = 0usize;
    let mut hit_files = 0usize;
    let mut capped = false;

    for file in &files {
        let rel = relative(policy, file);
        if filter.as_ref().is_some_and(|g| !g.matches(&rel)) {
            continue;
        }
        // A file this read cannot open — a permission, a socket, a bad UTF-8 boundary — is simply not
        // in the answer, rather than the reason a search of the whole workspace fails.
        let Ok(Some(text)) = read_text(tool, file) else {
            continue;
        };
        let owned: Vec<&str> = text.lines().collect();
        // The matching lines first, and only then how they are printed: neighbouring hits have to
        // share their context instead of repeating it, so a file with three close matches reads as one
        // block the way `grep -C` gives it.
        let hits: Vec<usize> = owned
            .iter()
            .enumerate()
            .filter(|(_, line)| re.is_match(line))
            .map(|(i, _)| i)
            .collect();
        if hits.is_empty() {
            continue;
        }
        hit_files += 1;
        let room = MAX_MATCHES - matches;
        let shown = hits.len().min(room);
        matches += shown;
        push_block(&mut lines, &rel, &owned, &hits[..shown], context);
        if shown < hits.len() {
            capped = true;
            break;
        }
    }

    let mut content = lines.join("\n");
    if capped {
        content.push_str(&format!(
            "\n…stopped at {MAX_MATCHES} matching lines. Narrow the pattern, the glob, or the path."
        ));
    }
    Ok(Executed {
        summary: match matches {
            0 => format!("nothing matches {pattern}"),
            1 => format!(
                "1 match in {hit_files} file{}",
                if hit_files == 1 { "" } else { "s" }
            ),
            n => format!("{n} matches in {hit_files} files"),
        },
        content: if content.is_empty() {
            format!("No line in the workspace matches {pattern}.")
        } else {
            with_skips(content, skipped)
        },
        edit: None,
    })
}

/// Replace an exact string in one workspace file, and report what it cost in lines.
///
/// Three things refuse the write before anything touches the disk: the file has to be text this tool
/// can read, the text named has to be present, and it has to be present once unless the call asked for
/// every copy. An ambiguous match is the failure a model makes most often and a person wants least,
/// so the call names which of the three it hit rather than writing its guess.
///
/// The sandbox is not rechecked here. [`crate::authorize`] resolves every path argument against the
/// policy before an executor runs, which is where a write outside the workspace, into `.git`, or into
/// a protected name is refused — out loud, in the sentence the row and the model both get.
fn edit_file(
    tool: &str,
    arguments: &Value,
    policy: &SandboxPolicy,
) -> Result<Executed, ToolsError> {
    let label = required(tool, arguments, "path")?.to_string();
    let path = resolve(policy, &label, Access::Write)?;
    let old = required(tool, arguments, "old_string")?;
    let new = required(tool, arguments, "new_string")?;
    let refuse = |reason: String| ToolsError::InvalidArguments {
        tool: tool.to_string(),
        reason,
    };
    if old.is_empty() {
        return Err(refuse(
            "\"old_string\" is empty, which every position in the file matches.".to_string(),
        ));
    }
    let Some(text) = read_text(tool, &path)? else {
        return Err(refuse(format!(
            "{label} is not text this tool can read, so it was not edited."
        )));
    };

    let found = text.matches(old).count();
    let every = flag_arg(arguments, "replace_all");
    if found == 0 {
        return Err(refuse(format!(
            "{label} holds nothing matching \"old_string\", so nothing was written. Read the file and \
             copy the line as it stands."
        )));
    }
    if found > 1 && !every {
        return Err(refuse(format!(
            "{label} holds {found} copies of \"old_string\" and the call named one, so nothing was \
             written. Include enough of the lines around it to match once, or set replace_all."
        )));
    }

    let times = if every { found } else { 1 };
    let edited = if every {
        text.replace(old, new)
    } else {
        text.replacen(old, new, 1)
    };
    fs::write(&path, &edited).map_err(|e| io_error(tool, &label, &e))?;

    let (removed, added) = changed_lines(old, new);
    let edit = Edit {
        path: relative(policy, &path),
        added: (added * times) as u32,
        removed: (removed * times) as u32,
        occurrences: times as u32,
    };
    let summary = if times > 1 {
        format!("{times} copies replaced")
    } else {
        "1 copy replaced".to_string()
    };
    Ok(Executed {
        content: format!(
            "Edited {} (+{} −{}): {summary}\n{}",
            edit.path,
            edit.added,
            edit.removed,
            hunk(&label, &text, old, new)
        ),
        summary,
        edit: Some(edit),
    })
}

/// How many lines at each end of a replacement the replacement shares with what it replaced. The two
/// are what makes "wrap this block in an `if`" read as two added lines rather than as the whole block
/// rewritten: a line the call carried over unchanged is not one it changed.
fn shared_edges(before: &[&str], after: &[&str]) -> (usize, usize) {
    let mut head = 0;
    while head < before.len() && head < after.len() && before[head] == after[head] {
        head += 1;
    }
    let mut tail = 0;
    while head + tail < before.len()
        && head + tail < after.len()
        && before[before.len() - 1 - tail] == after[after.len() - 1 - tail]
    {
        tail += 1;
    }
    (head, tail)
}

/// The lines a replacement takes out and puts back, as `(removed, added)`, with the shared edges
/// cancelled from both sides.
fn changed_lines(old: &str, new: &str) -> (usize, usize) {
    let before: Vec<&str> = old.lines().collect();
    let after: Vec<&str> = new.lines().collect();
    let (head, tail) = shared_edges(&before, &after);
    (before.len() - head - tail, after.len() - head - tail)
}

/// The change as the model sees it: the lines it took out and put back, with two of the file on either
/// side, in the `+`/`−` shape a diff uses — and the same shared edges [`changed_lines`] cancels printed
/// as context, so a hunk and the counts standing over it cannot disagree.
///
/// Only the first change gets a hunk: a `replace_all` over forty copies would spend the whole result
/// budget saying the same forty lines forty times. A hunk past the budget is cut, and says how much of
/// itself the cut left out.
fn hunk(label: &str, text: &str, old: &str, new: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = text
        .find(old)
        .map_or(0, |offset| text[..offset].lines().count());
    let before: Vec<&str> = old.lines().collect();
    let after: Vec<&str> = new.lines().collect();
    let (head, tail) = shared_edges(&before, &after);
    let end = start + before.len();

    let mut body: Vec<String> = vec![format!("{label}, at line {}:", start + 1)];
    for i in start.saturating_sub(CTX_LINES)..(start + head) {
        body.push(format!(" {}", lines.get(i).copied().unwrap_or("")));
    }
    for line in &before[head..before.len() - tail] {
        body.push(format!("-{line}"));
    }
    for line in &after[head..after.len() - tail] {
        body.push(format!("+{line}"));
    }
    // The unchanged lines inside the block, printed as context, then the file's own lines after it.
    let changed_end = end.min(lines.len());
    for line in &lines[(end - tail).max(start + head).min(changed_end)..changed_end] {
        body.push(format!(" {line}"));
    }
    for line in &lines[changed_end..(end + CTX_LINES).min(lines.len())] {
        body.push(format!(" {line}"));
    }

    let mut kept: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut held = 0usize;
    for line in body {
        let cost = line.len() + 1;
        if used + cost > MAX_RESULT_BYTES {
            held += 1;
            continue;
        }
        used += cost;
        kept.push(line);
    }
    if held > 0 {
        kept.push(format!("…{held} lines of the change are not shown."));
    }
    kept.join("\n")
}

/// One file's matches, printed with their context merged between neighbours: a hit's line carries
/// `path:NN: text`, the lines around it carry `path:NN| text`, and two hits whose windows touch print
/// as one block rather than repeating the lines they share.
fn push_block(out: &mut Vec<String>, rel: &str, lines: &[&str], hits: &[usize], context: usize) {
    let last = lines.len().saturating_sub(1);
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for &hit in hits {
        let from = hit.saturating_sub(context);
        let to = (hit + context).min(last);
        match groups.last_mut() {
            // Touching or overlapping windows become one group.
            Some(prev) if from <= prev.1.saturating_add(1) => prev.1 = prev.1.max(to),
            _ => groups.push((from, to)),
        }
    }
    for (g, &(from, to)) in groups.iter().enumerate() {
        if g > 0 {
            out.push(format!("{rel}- …"));
        }
        for (i, line) in lines.iter().enumerate().take(to + 1).skip(from) {
            let mark = if hits.binary_search(&i).is_ok() {
                ':'
            } else {
                '|'
            };
            out.push(format!("{rel}:{}{mark} {}", i + 1, line.trim_end()));
        }
    }
}

/// Every readable file under `root`, in walk order, with the trees and sizes a read could not answer
/// from counted in `skipped`.
///
/// Symlinks are not followed, so a link out of the workspace cannot make the walk leave it — the same
/// rule [`SandboxPolicy::resolve_path`] applies to a path the model names. `root` itself has already
/// been through that check by the time this runs, so anything under it only has to be tested against
/// the protected names.
fn walk(policy: &SandboxPolicy, root: &Path, skipped: &mut usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
        return out;
    }
    let mut iter = WalkDir::new(root)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .into_iter();
    while let Some(entry) = iter.next() {
        let Ok(entry) = entry else {
            *skipped += 1;
            continue;
        };
        if entry.file_type().is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if SKIP_DIRS.contains(&name.as_str()) || policy.is_protected(entry.path()) {
                iter.skip_current_dir();
            }
            continue;
        }
        if !entry.file_type().is_file()
            || policy.is_protected(entry.path())
            || entry
                .metadata()
                .map(|m| m.len() > MAX_FILE_BYTES)
                .unwrap_or(true)
        {
            *skipped += 1;
            continue;
        }
        if out.len() >= MAX_WALK_ENTRIES {
            *skipped += 1;
            break;
        }
        out.push(entry.path().to_path_buf());
    }
    out
}

/// A file as text, whole: the walk already refused anything past [`MAX_FILE_BYTES`], so this read has
/// a bounded allocation and the answer gets an exact line count out of it. `None` when the bytes are
/// not text.
fn read_text(tool: &str, path: &Path) -> Result<Option<String>, ToolsError> {
    let shown = path.display().to_string();
    let meta = fs::metadata(path).map_err(|e| io_error(tool, &shown, &e))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(ToolsError::InvalidArguments {
            tool: tool.to_string(),
            reason: format!(
                "{shown} is {}, past the {} this build reads",
                human_bytes(meta.len()),
                human_bytes(MAX_FILE_BYTES)
            ),
        });
    }
    let mut file = File::open(path).map_err(|e| io_error(tool, &shown, &e))?;
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut buf)
        .map_err(|e| io_error(tool, &shown, &e))?;
    Ok(String::from_utf8(buf).ok())
}

/// The one sentence both walkers owe: what they left out of the tree and why.
fn with_skips(content: String, skipped: usize) -> String {
    if skipped == 0 {
        return content;
    }
    format!(
        "{content}\n({skipped} paths were deeper than {MAX_DEPTH} levels, larger than {}, protected, \
         or unreadable.)",
        human_bytes(MAX_FILE_BYTES)
    )
}

fn required<'a>(tool: &str, arguments: &'a Value, key: &str) -> Result<&'a str, ToolsError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolsError::InvalidArguments {
            tool: tool.to_string(),
            reason: format!("\"{key}\" is required and must be a string"),
        })
}

fn resolve(policy: &SandboxPolicy, raw: &str, access: Access) -> Result<PathBuf, ToolsError> {
    Ok(policy.resolve_path(Path::new(raw), access)?)
}

/// A directory to walk: the `path` argument, or the whole workspace when it is left out.
fn optional_root(arguments: &Value, policy: &SandboxPolicy) -> Result<PathBuf, ToolsError> {
    resolve(
        policy,
        text_arg(arguments, "path").unwrap_or("."),
        Access::Read,
    )
}

fn compile(
    tool: &str,
    key: &str,
    raw: &str,
    build: fn(&str) -> Result<glob::Pattern, glob::PatternError>,
) -> Result<glob::Pattern, ToolsError> {
    build(raw).map_err(|e| ToolsError::InvalidArguments {
        tool: tool.to_string(),
        reason: format!("\"{raw}\" is not a valid glob in \"{key}\": {e}"),
    })
}

/// The workspace-relative path, in forward slashes: what the model passed to `glob` and what it can
/// pass straight back to `read_file`.
fn relative(policy: &SandboxPolicy, path: &Path) -> String {
    policy
        .display_path(path)
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn text_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn number_arg(arguments: &Value, key: &str) -> Option<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .map(|n| n as usize)
}

fn flag_arg(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool) == Some(true)
}

fn io_error(tool: &str, path: &str, error: &std::io::Error) -> ToolsError {
    ToolsError::Io {
        tool: tool.to_string(),
        path: path.to_string(),
        reason: error.to_string(),
    }
}

/// A size in the unit a person compares, because a raw byte count in a transcript row is a number
/// that means nothing next to another.
fn human_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < KB * KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{:.1} MB", bytes as f64 / (KB * KB) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn policy(root: &Path) -> SandboxPolicy {
        SandboxPolicy {
            workspace_root: root.to_path_buf(),
            ..Default::default()
        }
    }

    /// A workspace with one secret the sandbox protects, one vendor tree, and two readable files.
    fn fixture() -> (tempfile::TempDir, SandboxPolicy) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.path().join("src/big.rs"), "x").unwrap();
        std::fs::write(dir.path().join(".env"), "one TOKEN=secret\n").unwrap();
        std::fs::write(dir.path().join("node_modules/pkg/index.js"), "one\n").unwrap();
        let p = policy(dir.path());
        (dir, p)
    }

    fn ran(id: &str, args: &Value, p: &SandboxPolicy) -> Result<Executed, ToolsError> {
        execute(id, args, p)
    }

    /// A file of `lines` readable lines, each labelled with its 1-based number and padded to `width`.
    fn long_file(dir: &Path, lines: usize, width: usize) -> PathBuf {
        let path = dir.join("long.txt");
        let mut f = File::create(&path).unwrap();
        for i in 1..=lines {
            let mut line = format!("line {i} of the long file,");
            while line.len() < width {
                line.push_str(" padded out");
            }
            writeln!(f, "{line}").unwrap();
        }
        path
    }

    /// A workspace with nothing in it the sandbox protects, so an exact answer can be asserted.
    fn plain() -> (tempfile::TempDir, SandboxPolicy) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "one\ntwo\nthree\n").unwrap();
        let p = policy(dir.path());
        (dir, p)
    }

    #[test]
    fn a_read_answers_the_whole_file_when_no_range_is_asked_for() {
        let (_d, p) = fixture();
        let got =
            ran("read_file", &json!({ "path": "src/lib.rs" }), &p).expect("a workspace read runs");
        assert_eq!(got.content, "one\ntwo\nthree");
        assert_eq!(got.summary, "3 lines");
    }

    #[test]
    fn a_range_answers_that_range_and_says_how_much_of_the_file_it_was() {
        let (_d, p) = fixture();
        let got = ran(
            "read_file",
            &json!({ "path": "src/lib.rs", "start_line": 2, "end_line": 3 }),
            &p,
        )
        .unwrap();
        assert_eq!(got.content, "two\nthree");
        assert_eq!(got.summary, "lines 2–3 of 3");
        // A range that starts past the end is an answer, not a failure: the model needs to learn the
        // file is shorter than it thought.
        let past = ran(
            "read_file",
            &json!({ "path": "src/lib.rs", "start_line": 99 }),
            &p,
        )
        .unwrap();
        assert!(past.content.contains("past line 3"), "{}", past.content);
        // And an empty range says so rather than showing line `from`.
        let backwards = ran(
            "read_file",
            &json!({ "path": "src/lib.rs", "start_line": 3, "end_line": 1 }),
            &p,
        )
        .unwrap();
        assert!(
            backwards.content.contains("before it starts"),
            "{}",
            backwards.content
        );
    }

    #[test]
    fn a_long_file_is_cut_on_a_line_and_says_where_to_ask_from() {
        let dir = tempfile::tempdir().unwrap();
        long_file(dir.path(), 5_000, 55);
        let p = policy(dir.path());
        let got = ran("read_file", &json!({ "path": "long.txt" }), &p).unwrap();
        assert!(
            got.content.len() <= MAX_RESULT_BYTES + 200,
            "{} bytes of answer",
            got.content.len()
        );
        assert!(got.summary.contains("capped"), "{}", got.summary);
        assert!(
            got.content.contains("Ask again with start_line="),
            "a cut is only useful if it names where to continue: {}",
            got.summary
        );
        // And the count it quotes is the file's real length, not the number of lines it happened to
        // hold in the answer.
        assert!(got.summary.contains("of 5000"), "{}", got.summary);
        assert!(!got.content.ends_with('\n'), "the cut stays on a line");
    }

    #[test]
    fn a_range_past_the_cap_still_answers_the_tail_of_a_long_file() {
        let dir = tempfile::tempdir().unwrap();
        long_file(dir.path(), 2_000, 45);
        let p = policy(dir.path());
        let got = ran(
            "read_file",
            &json!({ "path": "long.txt", "start_line": 1_900 }),
            &p,
        )
        .unwrap();
        assert!(got.content.starts_with("line 1900"), "{}", got.content);
        assert!(got.summary.contains("of 2000"), "{}", got.summary);
    }

    /// The line number a capped answer told the model to ask for next.
    fn resume_line(content: &str) -> u64 {
        content
            .rsplit("start_line=")
            .next()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn continuing_where_the_last_answer_stopped_leaves_no_line_out() {
        let dir = tempfile::tempdir().unwrap();
        long_file(dir.path(), 2_000, 55);
        let p = policy(dir.path());
        let first = ran("read_file", &json!({ "path": "long.txt" }), &p).unwrap();
        let stopped = resume_line(&first.content);
        let last_shown = first
            .content
            .split("\n…output capped")
            .next()
            .unwrap()
            .lines()
            .last()
            .unwrap()
            .to_string();
        assert!(
            last_shown.starts_with(&format!("line {} of the long file,", stopped - 1)),
            "{last_shown}"
        );
        let second = ran(
            "read_file",
            &json!({ "path": "long.txt", "start_line": stopped }),
            &p,
        )
        .unwrap();
        assert!(
            second.content.starts_with(&format!("line {stopped} of")),
            "{}",
            second.content
        );
        assert!(second.summary.contains("of 2000"), "{}", second.summary);
    }

    #[test]
    fn a_listing_holds_back_the_secret_and_the_vendor_tree_and_says_so() {
        let (_d, p) = fixture();
        let got = ran("list_dir", &json!({ "path": "." }), &p).unwrap();
        assert!(got.content.contains("src/"), "{}", got.content);
        assert!(!got.content.contains(".env"), "protected: {}", got.content);
        assert!(
            !got.content.contains("node_modules"),
            "vendor: {}",
            got.content
        );
        assert!(
            got.content.contains("held back"),
            "the gap is named: {}",
            got.content
        );
        // Inside a real source directory the files are listed, with their sizes.
        let inside = ran("list_dir", &json!({ "path": "src" }), &p).unwrap();
        assert!(inside.content.contains("lib.rs ("), "{}", inside.content);
        assert!(inside.content.contains("big.rs ("), "{}", inside.content);
    }

    #[test]
    fn a_glob_answers_workspace_relative_paths_the_model_can_read_back() {
        let (_d, p) = fixture();
        let got = ran("glob", &json!({ "pattern": "src/*.rs" }), &p).unwrap();
        assert!(got.content.contains("src/big.rs"), "{}", got.content);
        assert!(got.content.contains("src/lib.rs"), "{}", got.content);
        assert!(!got.content.contains("node_modules"), "{}", got.content);
        // Every path it prints is one `read_file` will accept.
        for line in got.content.lines().filter(|l| !l.starts_with('(')) {
            assert!(
                ran("read_file", &json!({ "path": line }), &p).is_ok(),
                "{line} is not readable back"
            );
        }
        // A bare name pattern finds the file wherever it is, and a separator-bound one does not cross
        // directories.
        let bare = ran("glob", &json!({ "pattern": "*.rs" }), &p).unwrap();
        assert!(bare.content.contains("src/lib.rs"), "{}", bare.content);
        let wide = ran("glob", &json!({ "pattern": "src/*" }), &p).unwrap();
        assert_eq!(
            wide.content.lines().filter(|l| l.contains('/')).count(),
            2,
            "`src/*` reached past one directory: {}",
            wide.content
        );
    }

    #[test]
    fn a_search_answers_file_line_number_and_line() {
        let (_d, p) = plain();
        let got = ran("grep_search", &json!({ "pattern": "^two" }), &p).unwrap();
        assert_eq!(got.content, "src/lib.rs:2: two");
        assert_eq!(got.summary, "1 match in 1 file");
        // Nothing found is an answer too, and says what it looked for.
        let none = ran("grep_search", &json!({ "pattern": "zzzz" }), &p).unwrap();
        assert!(none.content.starts_with("No line"), "{}", none.content);
        assert!(none.summary.contains("nothing matches"), "{}", none.summary);
    }

    #[test]
    fn a_search_never_reads_the_secret_or_the_vendor_tree() {
        let (_d, p) = fixture();
        // `one` is in `.env`, in `node_modules` and in `src/lib.rs`: only the last may appear.
        let got = ran("grep_search", &json!({ "pattern": "one|TOKEN" }), &p).unwrap();
        assert!(!got.content.contains("TOKEN"), "protected: {}", got.content);
        assert!(
            !got.content.contains("node_modules"),
            "vendor tree searched: {}",
            got.content
        );
        assert!(got.content.contains("src/lib.rs:1: one"), "{}", got.content);
    }

    #[test]
    fn a_search_finds_a_match_past_what_one_read_of_a_file_returns() {
        let dir = tempfile::tempdir().unwrap();
        // Line 4_000 is far past MAX_RESULT_BYTES: a search that only read the head of each file would
        // answer "no line matches" about a file it never looked at.
        long_file(dir.path(), 4_000, 55);
        std::fs::write(dir.path().join("needle.txt"), "top\nneedle here\nbottom\n").unwrap();
        let mut contents = std::fs::read_to_string(dir.path().join("long.txt")).unwrap();
        contents.push_str("the needle is at the end of the long file\n");
        std::fs::write(dir.path().join("long.txt"), contents).unwrap();
        let got = ran(
            "grep_search",
            &json!({ "pattern": "needle", "glob": "*.txt" }),
            &policy(dir.path()),
        )
        .unwrap();
        assert!(
            got.content.contains("long.txt:4001: the needle"),
            "{}",
            got.content
        );
    }

    #[test]
    fn context_lines_come_off_the_file_around_the_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "1\n2\nhit\n4\n5\n").unwrap();
        let got = ran(
            "grep_search",
            &json!({ "pattern": "hit", "context_lines": 1 }),
            &policy(dir.path()),
        )
        .unwrap();
        assert!(got.content.contains("a.txt:2| 2"), "{}", got.content);
        assert!(got.content.contains("a.txt:3: hit"), "{}", got.content);
        assert!(got.content.contains("a.txt:4| 4"), "{}", got.content);
    }

    #[test]
    fn a_search_respects_its_glob_and_its_case_flag() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/a.md"), "Needed here\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "NEEDED there\n").unwrap();
        let p = policy(dir.path());
        let only_md = ran(
            "grep_search",
            &json!({ "pattern": "NEEDED", "glob": "docs/*", "ignore_case": true }),
            &p,
        )
        .unwrap();
        assert_eq!(only_md.content, "docs/a.md:1: Needed here");
        let strict = ran("grep_search", &json!({ "pattern": "Needed" }), &p).unwrap();
        assert!(
            !strict.content.contains("b.rs"),
            "case folded without the flag"
        );
    }

    #[test]
    fn a_call_outside_the_workspace_is_refused_before_anything_is_read() {
        let (_d, p) = fixture();
        let err = ran("read_file", &json!({ "path": "../outside.txt" }), &p)
            .expect_err("a read that escapes must be refused");
        assert!(matches!(err, ToolsError::Sandbox(_)), "{err:?}");
        let walk = ran("grep_search", &json!({ "pattern": "x", "path": ".." }), &p)
            .expect_err("a walk that starts outside must be refused");
        assert!(matches!(walk, ToolsError::Sandbox(_)), "{walk:?}");
        // And naming the protected file itself is refused, not answered with a quiet "nothing found".
        let err = ran("read_file", &json!({ "path": ".env" }), &p).expect_err("secret");
        assert!(matches!(err, ToolsError::Sandbox(_)), "{err:?}");
    }

    #[test]
    fn a_declared_tool_without_an_executor_is_refused_rather_than_run() {
        let (_d, p) = fixture();
        // `git_status` is a read the registry declares and nothing implements: offering it would be a
        // promise this file cannot keep.
        let err = ran("git_status", &json!({}), &p).expect_err("no executor exists");
        assert!(matches!(err, ToolsError::NoExecutor(_)), "{err:?}");
        assert!(matches!(
            ran(
                "write_file",
                &json!({ "path": "src/x", "contents": "y" }),
                &p
            ),
            Err(ToolsError::NoExecutor(_))
        ));
    }

    #[test]
    fn a_missing_or_uncompilable_argument_says_which_one() {
        let (_d, p) = fixture();
        let err = ran("read_file", &json!({}), &p).expect_err("path is required");
        assert!(err.to_string().contains("\"path\""), "{err}");
        let err = ran("glob", &json!({ "path": 3 }), &p).expect_err("pattern is required");
        assert!(err.to_string().contains("\"pattern\""), "{err}");
        // A pattern the matcher cannot compile is the model's mistake, and it has to be able to fix it.
        let err =
            ran("grep_search", &json!({ "pattern": "(unbalanced" }), &p).expect_err("bad regex");
        assert!(err.to_string().contains("regular expression"), "{err}");
        let err = ran("glob", &json!({ "pattern": "**[a" }), &p).expect_err("bad glob");
        assert!(err.to_string().contains("not a valid glob"), "{err}");
    }

    #[test]
    fn a_binary_file_is_not_answered_with_replacement_characters() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bin.dat"), [0u8, 159, 146, 150]).unwrap();
        let err = ran(
            "read_file",
            &json!({ "path": "bin.dat" }),
            &policy(dir.path()),
        )
        .expect_err("not text");
        assert!(err.to_string().contains("not text"), "{err}");
    }

    #[test]
    fn a_file_past_the_size_a_read_answers_is_named_and_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.bin");
        let f = File::create(&path).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        let err = ran(
            "read_file",
            &json!({ "path": "huge.bin" }),
            &policy(dir.path()),
        )
        .expect_err("too big to read");
        assert!(err.to_string().contains("MB"), "{err}");
    }

    #[test]
    fn an_empty_file_is_answered_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("nothing.txt"), "").unwrap();
        let got = ran(
            "read_file",
            &json!({ "path": "nothing.txt" }),
            &policy(dir.path()),
        )
        .unwrap();
        assert!(got.content.contains("is empty"), "{}", got.content);
    }

    #[test]
    fn an_absolute_path_inside_the_root_is_the_same_answer_as_a_relative_one() {
        let (_d, p) = fixture();
        let abs = p.workspace_root.join("src/lib.rs");
        let got = ran("read_file", &json!({ "path": abs }), &p).unwrap();
        assert_eq!(got.content, "one\ntwo\nthree");
        // And a listing that walked an absolute root prints relative paths, so no answer carries the
        // home directory into the prompt.
        let walk = ran(
            "glob",
            &json!({ "pattern": "*.rs", "path": abs.parent().unwrap() }),
            &p,
        )
        .unwrap();
        assert!(!walk.content.contains("private/var"), "{}", walk.content);
    }

    #[test]
    fn sizes_are_printed_in_the_unit_a_person_compares() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    /// The bytes on disk after a call, which is the only evidence that counts for a write.
    fn on_disk(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    #[test]
    fn an_edit_writes_the_file_and_counts_the_lines_it_cost() {
        let (d, p) = plain();
        let got = ran(
            "edit_file",
            &json!({ "path": "src/lib.rs", "old_string": "two", "new_string": "TWO\nand a half" }),
            &p,
        )
        .expect("an edit of a workspace file runs");
        assert_eq!(
            on_disk(d.path(), "src/lib.rs"),
            "one\nTWO\nand a half\nthree\n"
        );
        let edit = got.edit.expect("a write reports what it wrote");
        assert_eq!((edit.added, edit.removed, edit.occurrences), (2, 1, 1));
        // The path is the spelling a follow-up `read_file` takes back.
        assert_eq!(edit.path, "src/lib.rs");
        assert_eq!(got.summary, "1 copy replaced");
    }

    #[test]
    fn a_replacement_that_carries_a_line_over_costs_only_the_line_it_added() {
        let (d, p) = plain();
        let got = ran(
            "edit_file",
            &json!({ "path": "src/lib.rs", "old_string": "two", "new_string": "two\nfour" }),
            &p,
        )
        .expect("an edit runs");
        // The line the call kept is not a line it rewrote, so the edit is one line rather than two.
        let edit = got.edit.unwrap();
        assert_eq!((edit.added, edit.removed), (1, 0));
        assert_eq!(on_disk(d.path(), "src/lib.rs"), "one\ntwo\nfour\nthree\n");
    }

    #[test]
    fn wrapping_a_block_costs_its_own_lines_rather_than_the_whole_block() {
        let (d, p) = plain();
        // The file's first and last lines are shared and cancel out; what is left is the edit a diff
        // would print — one line out, three in — rather than "the whole block was rewritten".
        let got = ran(
            "edit_file",
            &json!({
                "path": "src/lib.rs",
                "old_string": "one\ntwo\nthree",
                "new_string": "one\nif ready {\n  two\n}\nthree",
            }),
            &p,
        )
        .expect("a multi-line edit runs");
        let edit = got.edit.unwrap();
        assert_eq!((edit.added, edit.removed), (3, 1));
        assert_eq!(
            on_disk(d.path(), "src/lib.rs"),
            "one\nif ready {\n  two\n}\nthree\n"
        );
    }

    #[test]
    fn an_edit_that_matches_nothing_writes_nothing_and_says_so() {
        let (d, p) = plain();
        let why = ran(
            "edit_file",
            &json!({ "path": "src/lib.rs", "old_string": "four", "new_string": "4" }),
            &p,
        )
        .expect_err("a match the file does not hold cannot be applied");
        assert!(why.to_string().contains("holds nothing matching"), "{why}");
        assert_eq!(on_disk(d.path(), "src/lib.rs"), "one\ntwo\nthree\n");
    }

    #[test]
    fn an_ambiguous_edit_is_refused_rather_than_applied_to_the_wrong_copy() {
        let (d, p) = plain();
        std::fs::write(d.path().join("src/lib.rs"), "two\ntwo\n").unwrap();
        let why = ran(
            "edit_file",
            &json!({ "path": "src/lib.rs", "old_string": "two", "new_string": "ONE" }),
            &p,
        )
        .expect_err("two copies of the text is not one match");
        let reason = why.to_string();
        assert!(reason.contains("2 copies"), "{reason}");
        // The sentence names the way out, because the model is the reader that can take it.
        assert!(reason.contains("replace_all"), "{reason}");
        assert_eq!(on_disk(d.path(), "src/lib.rs"), "two\ntwo\n");
    }

    #[test]
    fn replace_all_changes_every_copy_and_counts_the_lines_once_each() {
        let (d, p) = plain();
        std::fs::write(d.path().join("src/lib.rs"), "old\nold\nold\n").unwrap();
        let got = ran(
            "edit_file",
            &json!({
                "path": "src/lib.rs",
                "old_string": "old",
                "new_string": "new",
                "replace_all": true,
            }),
            &p,
        )
        .expect("a call that asks for every copy gets it");
        assert_eq!(on_disk(d.path(), "src/lib.rs"), "new\nnew\nnew\n");
        let edit = got.edit.unwrap();
        assert_eq!((edit.added, edit.removed, edit.occurrences), (3, 3, 3));
        assert_eq!(got.summary, "3 copies replaced");
        // One hunk rather than three: the copies are the same change said three times.
        assert_eq!(got.content.matches("at line").count(), 1, "{}", got.content);
    }

    #[test]
    fn a_hunk_shows_the_changed_line_between_the_lines_around_it() {
        let (_d, p) = plain();
        let got = ran(
            "edit_file",
            &json!({ "path": "src/lib.rs", "old_string": "two", "new_string": "TWO" }),
            &p,
        )
        .expect("an edit runs");
        assert_eq!(
            got.content,
            "Edited src/lib.rs (+1 −1): 1 copy replaced\nsrc/lib.rs, at line 2:\n one\n-two\n+TWO\n three"
        );
    }

    #[test]
    fn an_edit_outside_the_workspace_is_refused_before_it_can_touch_a_file() {
        let (d, p) = plain();
        let outside = d.path().parent().unwrap().join("escaped.rs");
        let _ = std::fs::remove_file(&outside);
        let why = ran(
            "edit_file",
            &json!({
                "path": outside,
                "old_string": "x",
                "new_string": "y",
            }),
            &p,
        )
        .expect_err("the workspace root is the boundary of a write");
        assert!(matches!(why, ToolsError::Sandbox(_)), "{why}");
        assert!(!outside.exists(), "the refusal wrote the file anyway");
    }

    #[test]
    fn a_protected_name_is_refused_for_a_write_as_it_is_for_a_read() {
        let (d, p) = fixture();
        let why = ran(
            "edit_file",
            &json!({ "path": ".env", "old_string": "TOKEN", "new_string": "token" }),
            &p,
        )
        .expect_err("a secret the sandbox hides cannot be edited through it");
        assert!(matches!(why, ToolsError::Sandbox(_)), "{why}");
        assert_eq!(on_disk(d.path(), ".env"), "one TOKEN=secret\n");
    }

    #[test]
    fn an_empty_old_string_is_refused_rather_than_matching_every_position() {
        let (_d, p) = plain();
        let why = ran(
            "edit_file",
            &json!({ "path": "src/lib.rs", "old_string": "", "new_string": "x" }),
            &p,
        )
        .expect_err("an empty match is not an edit");
        assert!(why.to_string().contains("empty"), "{why}");
    }
}
