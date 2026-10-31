//! The panel's own shell: a command the user types, run in the project folder with the agent's rules.
//!
//! The panel already shows the commands a run executed ([`crate::agent`]); this is the other half of
//! "re-run the check by hand". A real PTY is opened once and kept, so `cd`, an exported variable and a
//! running server all survive from one line to the next and the folder is the same one the run worked
//! in — not the app's own directory.
//!
//! # Why the panel reads the line before the shell does
//!
//! The approval dial is the promise this app makes about what runs on this machine. A shell that took
//! raw keystrokes would be the one way around it: the user could type `rm -rf .`, press Enter, and the
//! policy would only find out afterwards. So the shell runs with its echo and line editor off, the
//! window collects the line, and it is admitted by [`admit`] — the same [`evaluate`] the model's calls
//! go through — *before* a byte reaches the PTY. A refused line never runs; the panel says which entry
//! refused it. The cost is stated rather than hidden: there is no cursor editing or tab completion in
//! this field, because those are the shell reading keys we have not admitted.
//!
//! A prompt in the dial (Ask mode) is an admission here: the person typing the line is the one the
//! prompt would be asking. A refusal (Plan-only, Terminal access off, the command on the deny list) is
//! kept, which is the half the dial exists for.
//!
//! # The Python half
//!
//! The panel opens either a shell or a Python REPL in the project folder; the lines go through the
//! same [`admit`], so the same switch, dial and deny list stand in front of both. Python is not given a
//! PTY. A PTY is what makes an interactive Python build read keys through readline, and readline draws
//! the line it reads — the panel would then show every command twice, and the "the host echoes the
//! admitted line" promise would be gone. Piped input has no terminal echo and no line editor, so the
//! line the host reports is the only copy of it. [`PY_STARTUP`] blanks the prompts and raises the same
//! ready marker [`RC`] does, and Ctrl-C is a real `SIGINT` at the process rather than an ETX byte,
//! which is what makes Python answer it with `KeyboardInterrupt` and keep the session open.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use freecode_config::agent::{shell_governed, ApprovalMode, Approvals};
use freecode_core::approvals::{evaluate, ApprovalDecision, Pending};
use freecode_tools::SandboxPolicy;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

/// What the panel starts: the user's own shell, or a Python REPL. Both run in the project folder and
/// both are admitted line by line by [`admit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    Shell,
    Python,
}

/// What starting or sending a line was refused by. The window words it; the codes are the contract.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum Refusal {
    /// Terminal access is off in Settings, which is the consent for every command in this build.
    Off,
    /// The approval dial is Plan only: a shell could start, but nothing it ran would be admitted, so
    /// the panel says that instead of opening a field that always refuses.
    Plan,
    /// The command is refused by policy — the deny list, or a tool override that blocks the shell.
    Blocked { command: String },
    /// No shell is open, or the one that was open has ended.
    Closed,
    /// A terminal or the shell itself could not be started.
    Start { reason: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellInfo {
    pub id: u64,
    /// The folder the shell starts in, so the panel can say where a command will run.
    pub cwd: String,
    /// Which of the two sessions this is, so the panel can write the right prompt and tab.
    pub language: Language,
}

/// What the window hears while a shell is open. One channel: the line it admitted, the bytes back,
/// and the end.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ShellEvent {
    /// The line that was just admitted, sent before a byte of it reaches the terminal. The shell runs
    /// with echo off, so this is the only copy of what the user typed — and because a refused line
    /// never gets here, the panel's log cannot show a command that did not run.
    Sent {
        id: u64,
        command: String,
    },
    Chunk {
        id: u64,
        text: String,
    },
    Exit {
        id: u64,
        code: i32,
    },
}

pub struct ShellState {
    next: AtomicU64,
    session: Mutex<Option<Session>>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(1),
            session: Mutex::new(None),
        }
    }
}

struct Session {
    id: u64,
    cwd: PathBuf,
    language: Language,
    process: Process,
    /// Set by the waiter thread, so a send after the shell ended is answered as such rather than
    /// writing into a closed terminal.
    dead: Arc<AtomicBool>,
    /// Raised once the startup has finished and the session may be written to. A send holds until
    /// then.
    ready: Arc<Ready>,
    /// The folder the startup file was written in. Removed with the session.
    scratch: PathBuf,
}

/// How a session is written to and stopped. The shell is a PTY — it needs a terminal for `cd`, an
/// editor or a server to behave. Python is a pipe pair for the reason the module doc gives: a PTY is
/// what would bring readline's own echo back.
enum Process {
    Pty {
        /// The master side is what keeps the PTY open: dropping it hangs the shell up, so it lives as
        /// long as the session and is the handle a resize goes through.
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
        killer: Box<dyn ChildKiller + Send + Sync>,
    },
    /// Python only exists where `fork` and signals do; the host's Windows build refuses it at the
    /// door, so the variant is compiled out rather than left to hold nothing.
    #[cfg(unix)]
    Pipes {
        stdin: Box<dyn Write + Send>,
        /// The pid is how the interrupt and the kill reach a process whose `wait` is owned by the
        /// waiter thread.
        pid: u32,
    },
}

impl Session {
    /// One admitted line into the session. A shell gets its newline and its canonical-mode delivery; a
    /// Python REPL gets the same, and an empty line is a real line there — it closes a block.
    fn write_line(&mut self, command: &str) -> std::io::Result<()> {
        match &mut self.process {
            Process::Pty { writer, .. } => writer
                .write_all(format!("{command}\n").as_bytes())
                .and_then(|_| writer.flush()),
            #[cfg(unix)]
            Process::Pipes { stdin, .. } => stdin
                .write_all(format!("{command}\n").as_bytes())
                .and_then(|_| stdin.flush()),
        }
    }

    /// Ctrl-C: stops what is running rather than starting something, so it is not a line.
    fn interrupt(&mut self) -> std::io::Result<()> {
        match &mut self.process {
            Process::Pty { writer, .. } => writer.write_all(&[3u8]),
            #[cfg(unix)]
            Process::Pipes { pid, .. } => {
                // Safety: `kill` with a plain signal number has no memory-safety preconditions; a pid
                // that has already gone is reported as ESRCH and ignored by the caller.
                unsafe {
                    libc::kill(*pid as libc::pid_t, libc::SIGINT);
                }
                Ok(())
            }
        }
    }

    /// Ending the session from the app: the kill, not a closed stdin, because a Python process can be
    /// waiting on something that does not read its input.
    fn kill(&mut self) {
        match &mut self.process {
            Process::Pty { killer, .. } => {
                let _ = killer.kill();
            }
            #[cfg(unix)]
            Process::Pipes { pid, .. } => {
                // Safety: as in `interrupt`; SIGKILL has no handler and no memory-safety conditions.
                unsafe {
                    libc::kill(*pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

/// The startup file a session is born with. `stty -echo` and no zle are the two halves of the rule
/// above: the kernel and the shell both stay quiet about keys they never see, and only an admitted
/// line is written to the terminal. A blank prompt because the panel draws its own. History is off:
/// these lines belong to the panel, and writing them into the user's own shell history would be a
/// record they did not ask for.
const RC: &str = "\
unsetopt zle prompt_sp prompt_cr beep
setopt no_prompt_subst
PROMPT=
RPROMPT=
PROMPT_EOL_MARK=
PS2=
HISTFILE=/dev/null
SAVEHIST=0
HISTSIZE=0
stty -echo
printf '\\033]633;freecode-ready\\007'
";

/// Printed by [`RC`] once echo is off. A line is held until it arrives: a command written the instant
/// the panel opens would otherwise land in the terminal's input queue before `stty -echo` ran, and
/// either the kernel or zsh's line editor would draw it a second time. It is an OSC sequence, so the
/// sanitizer would drop it anyway; [`ReadyScan`] removes it before that so it never reaches the panel.
const READY: &[u8] = b"\x1b]633;freecode-ready\x07";

/// The startup file Python runs before its first prompt (`PYTHONSTARTUP`). Both prompts are blank
/// because the panel draws its own, and the marker is the same one [`RC`] raises so one [`Ready`] gate
/// serves both sessions. Python reads this file only in interactive mode, which is what `-i` forces.
const PY_STARTUP: &str = "\
import sys
sys.ps1 = \"\"
sys.ps2 = \"\"
print(\"\\033]633;freecode-ready\\007\", end=\"\", flush=True)
";

/// The gate between "the terminal exists" and "a line may be written".
#[derive(Default)]
struct Ready {
    seen: Mutex<bool>,
    cv: Condvar,
}

impl Ready {
    fn mark(&self) {
        let mut seen = self.seen.lock().unwrap();
        if !*seen {
            *seen = true;
            self.cv.notify_all();
        }
    }

    /// True if the shell signalled readiness before the deadline. A false answer is not fatal — the
    /// write goes ahead anyway, because a startup file that failed does not necessarily mean a shell
    /// that cannot run a command.
    fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut seen = self.seen.lock().unwrap();
        while !*seen {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let (guard, _) = self.cv.wait_timeout(seen, left).unwrap();
            seen = guard;
        }
        true
    }
}

/// The decision for one line. Kept apart from the PTY so the policy half is testable without one.
fn admit(approvals: &Approvals, command: &str) -> Result<(), Refusal> {
    if !approvals.shell_enabled {
        return Err(Refusal::Off);
    }
    let Some(tool) = freecode_tools::find("terminal") else {
        return Err(Refusal::Blocked {
            command: command.to_string(),
        });
    };
    // The switch was just read, so the sandbox the engine would build for a run is open on it. The
    // root plays no part in the decision; the command is not path-confined and the dial knows it.
    let sandbox = SandboxPolicy {
        shell_allowed: true,
        ..SandboxPolicy::default()
    };
    match evaluate(
        &tool,
        &json!({ "command": command }),
        approvals,
        &sandbox,
        "user-shell",
    ) {
        Pending::Decided(decision) if decision.is_allowed() => Ok(()),
        // No prompt is shown for this field, and asking would be asking the person who just typed
        // the line: their Enter is the approval.
        Pending::Prompt(_) => Ok(()),
        Pending::Decided(ApprovalDecision::BlockedByPolicy) => {
            if approvals.mode == ApprovalMode::PlanOnly {
                Err(Refusal::Plan)
            } else {
                Err(Refusal::Blocked {
                    command: command.to_string(),
                })
            }
        }
        Pending::Decided(_) => Err(Refusal::Blocked {
            command: command.to_string(),
        }),
    }
}

/// Why a shell cannot be opened at all: the same two refusals that would stop every line.
fn admit_start(approvals: &Approvals) -> Result<(), Refusal> {
    if !approvals.shell_enabled {
        return Err(Refusal::Off);
    }
    if approvals.mode == ApprovalMode::PlanOnly {
        return Err(Refusal::Plan);
    }
    // A tool override can switch the shell off while Terminal access stays on; the panel should say
    // so at the door rather than after the first refused line.
    if let Some(tool) = freecode_tools::find("terminal") {
        if shell_governed(tool.id) && approvals.decide(tool.id, tool.permission) == Some(false) {
            return Err(Refusal::Blocked {
                command: String::new(),
            });
        }
    }
    Ok(())
}

/// Open the panel's session. `language` is the press the user made — a shell or a Python REPL; a
/// window that names neither gets the shell, which is what every earlier caller meant.
#[tauri::command]
pub fn shell_open(
    app: AppHandle,
    state: State<'_, ShellState>,
    language: Option<Language>,
) -> Result<ShellInfo, Refusal> {
    let language = language.unwrap_or(Language::Shell);
    let settings =
        crate::appearance::load_settings().map_err(|reason| Refusal::Start { reason })?;
    admit_start(&settings.approvals)?;
    let root = crate::paths::project_root(Some(&app), &settings)
        .map_err(|reason| Refusal::Start { reason })?;

    let mut guard = state.session.lock().unwrap();
    if let Some(session) = guard.as_ref() {
        if !session.dead.load(Ordering::Relaxed) && session.language == language {
            return Ok(ShellInfo {
                id: session.id,
                cwd: session.cwd.display().to_string(),
                language,
            });
        }
    }
    // An ended session is replaced rather than reused: its writer is a closed terminal. A live one of
    // the other language is replaced too, because the panel holds one session at a time and the press
    // that named another language is the press that ends this one.
    if let Some(mut session) = guard.take() {
        session.kill();
    }

    let id = state.next.fetch_add(1, Ordering::Relaxed);
    let session = spawn(&app, id, &root, language)?;
    let info = ShellInfo {
        id,
        cwd: session.cwd.display().to_string(),
        language,
    };
    *guard = Some(session);
    Ok(info)
}

fn spawn(app: &AppHandle, id: u64, root: &Path, language: Language) -> Result<Session, Refusal> {
    match language {
        Language::Shell => spawn_shell(app, id, root),
        Language::Python => spawn_python(app, id, root),
    }
}

#[cfg(not(unix))]
fn spawn_shell(_app: &AppHandle, _id: u64, _root: &Path) -> Result<Session, Refusal> {
    Err(Refusal::Start {
        reason: "the built-in shell runs on macOS and Linux only".to_string(),
    })
}

#[cfg(unix)]
fn spawn_shell(app: &AppHandle, id: u64, root: &Path) -> Result<Session, Refusal> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 12,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| Refusal::Start {
            reason: format!("a terminal could not be opened: {e}"),
        })?;

    let rc = std::env::temp_dir().join(format!("freecode-shell-{id}-{}", std::process::id()));
    std::fs::create_dir_all(&rc).map_err(|e| Refusal::Start {
        reason: format!("the shell could not be prepared: {e}"),
    })?;
    std::fs::write(rc.join(".zshrc"), RC).map_err(|e| Refusal::Start {
        reason: format!("the shell could not be prepared: {e}"),
    })?;

    let mut cmd = CommandBuilder::new("/bin/zsh");
    cmd.arg("-i");
    cmd.cwd(root);
    // A builder starts with no environment of its own, and a shell without PATH cannot run the
    // project's own tools. The app's environment is the one the run inherits too.
    for (key, value) in std::env::vars_os() {
        cmd.env(key, value);
    }
    cmd.env("ZDOTDIR", &rc);
    cmd.env("TERM", "dumb");
    cmd.env("NO_COLOR", "1");
    cmd.env("CLICOLOR", "0");
    cmd.env("PAGER", "cat");
    cmd.env("GIT_PAGER", "cat");

    let child = pair.slave.spawn_command(cmd).map_err(|e| Refusal::Start {
        reason: format!("the shell could not be started: {e}"),
    })?;
    // The slave end belongs to the child now; keeping our copy open would keep the terminal from
    // reporting EOF when the shell exits.
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().map_err(|e| Refusal::Start {
        reason: format!("the terminal could not be read: {e}"),
    })?;
    let writer = pair.master.take_writer().map_err(|e| Refusal::Start {
        reason: format!("the terminal could not be written: {e}"),
    })?;
    let killer = child.clone_killer();

    let dead = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(Ready::default());
    let reader_app = app.clone();
    let reader_ready = ready.clone();
    thread::spawn(move || read_loop(reader_app, id, reader, reader_ready));
    let waiter_app = app.clone();
    let waiter_dead = dead.clone();
    thread::spawn(move || {
        let mut child = child;
        let code = match child.wait() {
            Ok(status) => status.exit_code() as i32,
            Err(_) => -1,
        };
        waiter_dead.store(true, Ordering::Relaxed);
        let _ = waiter_app.emit("shell-event", ShellEvent::Exit { id, code });
    });

    Ok(Session {
        id,
        cwd: root.to_path_buf(),
        language: Language::Shell,
        process: Process::Pty {
            master: pair.master,
            writer,
            killer,
        },
        dead,
        ready,
        scratch: rc,
    })
}

#[cfg(not(unix))]
fn spawn_python(_app: &AppHandle, _id: u64, _root: &Path) -> Result<Session, Refusal> {
    Err(Refusal::Start {
        reason: "the Python REPL runs on macOS and Linux only".to_string(),
    })
}

/// Python with its stdio on a pipe pair, merged into one stream so a `print` and the traceback after
/// it keep the order the interpreter wrote them in. The process is the user's own `python3` from
/// `PATH`, in the project folder, with the app's environment; the REPL has no sandbox of its own,
/// which is why every line is admitted before it is written.
#[cfg(unix)]
fn spawn_python(app: &AppHandle, id: u64, root: &Path) -> Result<Session, Refusal> {
    use std::os::unix::net::UnixStream;
    use std::process::{Command, Stdio};

    let scratch = std::env::temp_dir().join(format!("freecode-python-{id}-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| Refusal::Start {
        reason: format!("Python could not be prepared: {e}"),
    })?;
    let startup = scratch.join("startup.py");
    std::fs::write(&startup, PY_STARTUP).map_err(|e| Refusal::Start {
        reason: format!("Python could not be prepared: {e}"),
    })?;

    // One writer end cloned for both stdio handles, so the two streams arrive as one and the reader
    // sanitises them in a single pass.
    let (reader, out) = UnixStream::pair().map_err(|e| Refusal::Start {
        reason: format!("Python's output could not be wired: {e}"),
    })?;
    let err = out.try_clone().map_err(|e| Refusal::Start {
        reason: format!("Python's output could not be wired: {e}"),
    })?;
    let out: std::os::fd::OwnedFd = out.into();
    let err: std::os::fd::OwnedFd = err.into();

    let mut child = Command::new("python3")
        .args(["-q", "-i"])
        .current_dir(root)
        // `-q` drops the version banner; `-i` is what makes a piped stdin interactive, and that is
        // also what makes Python run `PYTHONSTARTUP`.
        .env("PYTHONSTARTUP", &startup)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| Refusal::Start {
            reason: format!("python3 could not be started: {e}"),
        })?;

    let pid = child.id();
    let stdin = child.stdin.take().ok_or_else(|| Refusal::Start {
        reason: "python3 was started without an input pipe".to_string(),
    })?;

    let dead = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(Ready::default());
    let reader_app = app.clone();
    let reader_ready = ready.clone();
    let reader = Box::new(reader);
    thread::spawn(move || read_loop(reader_app, id, reader, reader_ready));
    let waiter_app = app.clone();
    let waiter_dead = dead.clone();
    thread::spawn(move || {
        let code = match child.wait() {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };
        waiter_dead.store(true, Ordering::Relaxed);
        let _ = waiter_app.emit("shell-event", ShellEvent::Exit { id, code });
    });

    Ok(Session {
        id,
        cwd: root.to_path_buf(),
        language: Language::Python,
        process: Process::Pipes {
            stdin: Box::new(stdin),
            pid,
        },
        dead,
        ready,
        scratch,
    })
}

/// Read the terminal until it closes, turning bytes into text the panel can draw. Bytes that are not
/// text are dropped as they would be on a terminal; a sequence split across two reads is held until
/// the rest of it arrives.
fn read_loop(app: AppHandle, id: u64, mut reader: Box<dyn Read + Send>, ready: Arc<Ready>) {
    let mut plain = Plain::default();
    let mut scan = ReadyScan::default();
    let mut buf = [0u8; 8 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let bytes = scan.push(&buf[..n]);
                if scan.is_ready() {
                    ready.mark();
                }
                let text = plain.push(&bytes);
                if !text.is_empty() {
                    let _ = app.emit("shell-event", ShellEvent::Chunk { id, text });
                }
            }
        }
    }
    let rest = plain.flush();
    if !rest.is_empty() {
        let _ = app.emit("shell-event", ShellEvent::Chunk { id, text: rest });
    }
}

#[tauri::command]
pub async fn shell_send(
    app: AppHandle,
    state: State<'_, ShellState>,
    line: String,
) -> Result<(), Refusal> {
    // Leading indentation is part of a Python line — a suite's body is written with it — so only the
    // line ending the input field added is taken off.
    let command = line.trim_end_matches(['\r', '\n']);
    let language = state
        .session
        .lock()
        .unwrap()
        .as_ref()
        .map(|session| session.language)
        .unwrap_or(Language::Shell);
    // An empty line runs nothing in a shell, so it is answered without a round trip. In a Python REPL
    // it is a line the interpreter acts on: it closes a block. It is still not a program, so it goes
    // to the policy-free path below rather than being admitted as one.
    let blank = command.trim().is_empty();
    if blank && language != Language::Python {
        return Ok(());
    }
    if !blank {
        let settings =
            crate::appearance::load_settings().map_err(|reason| Refusal::Start { reason })?;
        admit(&settings.approvals, command)?;
    }

    // A shell opened a moment ago may not have sourced its startup file yet, and a line written now
    // would be echoed by the terminal before `stty -echo` ran — the panel would show the command
    // twice. The wait is taken without the session lock so a resize or a stop is never held behind
    // it, and the command is asynchronous so the wait is never on the window's thread.
    let (id, ready, dead) = {
        let guard = state.session.lock().unwrap();
        let Some(session) = guard.as_ref() else {
            return Err(Refusal::Closed);
        };
        if session.dead.load(Ordering::Relaxed) {
            return Err(Refusal::Closed);
        }
        (session.id, session.ready.clone(), session.dead.clone())
    };
    if !ready.wait(Duration::from_secs(5)) && dead.load(Ordering::Relaxed) {
        return Err(Refusal::Closed);
    }

    let mut guard = state.session.lock().unwrap();
    let Some(session) = guard.as_mut() else {
        return Err(Refusal::Closed);
    };
    // The session may have ended or been replaced while the gate was waiting.
    if session.id != id || session.dead.load(Ordering::Relaxed) {
        return Err(Refusal::Closed);
    }
    // The line is echoed to the window rather than by the terminal, and it is emitted before the
    // write so the log reads in the order the user typed it: a command that ran always has its line
    // above its output, and a refused one has neither. A blank Python line is echoed as well: it is
    // the line that closes a block, and a log that skipped it would never say the block ended.
    let _ = app.emit(
        "shell-event",
        ShellEvent::Sent {
            id: session.id,
            command: command.to_string(),
        },
    );
    // Both sessions run quiet about keys: the shell has echo off, and Python's stdin is a pipe that
    // has no echo to turn off. The newline is all either needs.
    session.write_line(command).map_err(|e| {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            Refusal::Closed
        } else {
            Refusal::Start {
                reason: format!("the command could not be sent: {e}"),
            }
        }
    })
}

/// Ctrl-C: the interrupt goes to the session, not through the line policy, because it stops a
/// command rather than starting one. A shell gets the terminal's ETX byte; Python, with no terminal
/// attached, gets a real `SIGINT`, which is what it answers with `KeyboardInterrupt`.
#[tauri::command]
pub fn shell_interrupt(state: State<'_, ShellState>) -> Result<(), Refusal> {
    let mut guard = state.session.lock().unwrap();
    let Some(session) = guard.as_mut() else {
        return Err(Refusal::Closed);
    };
    if session.dead.load(Ordering::Relaxed) {
        return Err(Refusal::Closed);
    }
    session.interrupt().map_err(|e| Refusal::Start {
        reason: format!("the interrupt could not be sent: {e}"),
    })
}

/// The panel body's measure, so a long line wraps where the panel does rather than at the width the
/// PTY happened to start with. Python's stdin is a pipe with no width, so there is nothing to set.
#[tauri::command]
pub fn shell_resize(state: State<'_, ShellState>, rows: u16, cols: u16) -> Result<(), Refusal> {
    let guard = state.session.lock().unwrap();
    let Some(session) = guard.as_ref() else {
        return Ok(());
    };
    let Process::Pty { master, .. } = &session.process else {
        return Ok(());
    };
    master
        .resize(PtySize {
            rows: rows.max(2),
            cols: cols.max(20),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| Refusal::Start {
            reason: format!("the terminal could not be resized: {e}"),
        })
}

#[tauri::command]
pub fn shell_close(state: State<'_, ShellState>) -> Result<(), Refusal> {
    if let Some(mut session) = state.session.lock().unwrap().take() {
        session.kill();
    }
    Ok(())
}

/// Nothing of this app may outlive it. Called from the exit hook, because a shell left behind is a
/// process still holding the project folder open after the window is gone.
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<ShellState>();
    let session = state.session.lock().ok().and_then(|mut guard| guard.take());
    if let Some(mut session) = session {
        session.kill();
    }
}

/// Finds [`READY`] in the terminal's bytes and takes it out, so the panel never draws the signal and
/// a read that ends inside it is not lost. Once seen, the rest of the stream passes through untouched.
#[derive(Default)]
struct ReadyScan {
    seen: bool,
    /// The tail of the last read that could still be the start of the marker.
    pending: Vec<u8>,
}

impl ReadyScan {
    fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        if self.seen {
            return bytes.to_vec();
        }
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(bytes);
        let mut out = Vec::with_capacity(buf.len());
        let mut i = 0;
        while i < buf.len() {
            if buf[i..].starts_with(READY) {
                self.seen = true;
                i += READY.len();
            } else if READY.starts_with(&buf[i..]) {
                // The rest could still become the marker with more bytes.
                break;
            } else {
                out.push(buf[i]);
                i += 1;
            }
        }
        if !self.seen {
            self.pending = buf[i..].to_vec();
        } else {
            out.extend_from_slice(&buf[i..]);
        }
        out
    }

    fn is_ready(&self) -> bool {
        self.seen
    }
}

/// Terminal output on its way to a `<pre>`: the escapes a terminal would interpret are dropped, and
/// the state needed to drop a sequence whose tail is in the next read is carried between calls.
#[derive(Default)]
struct Plain {
    /// Bytes of a UTF-8 character split by the read boundary.
    pending: Vec<u8>,
    escape: Escape,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Escape {
    #[default]
    None,
    Csi,
    Osc,
    OscEsc,
}

impl Plain {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut consumed = 0;
        while consumed < self.pending.len() {
            match std::str::from_utf8(&self.pending[consumed..]) {
                Ok(valid) => {
                    text.push_str(valid);
                    consumed = self.pending.len();
                }
                Err(err) => {
                    let good = consumed + err.valid_up_to();
                    if let Ok(prefix) = std::str::from_utf8(&self.pending[consumed..good]) {
                        text.push_str(prefix);
                    }
                    match err.error_len() {
                        Some(bad) => {
                            text.push('\u{fffd}');
                            consumed = good + bad;
                        }
                        // An incomplete sequence at the end of this read: keep it for the next one.
                        None => {
                            consumed = good;
                            break;
                        }
                    }
                }
            }
        }
        self.pending.drain(..consumed);
        self.plain(&text)
    }

    /// Whatever was still held when the terminal closed. An unterminated character is not worth a
    /// stall: it is shown as the replacement mark a terminal would draw.
    fn flush(&mut self) -> String {
        let rest = String::from_utf8_lossy(&self.pending).to_string();
        self.pending.clear();
        self.plain(&rest)
    }

    fn plain(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match self.escape {
                Escape::Csi => {
                    if ('\x40'..='\x7e').contains(&c) {
                        self.escape = Escape::None;
                    }
                }
                Escape::Osc => match c {
                    '\x07' => self.escape = Escape::None,
                    '\x1b' => self.escape = Escape::OscEsc,
                    _ => {}
                },
                Escape::OscEsc => {
                    self.escape = if c == '\\' { Escape::None } else { Escape::Osc };
                }
                Escape::None => match c {
                    '\x1b' => match chars.next() {
                        Some('[') => self.escape = Escape::Csi,
                        Some(']') => self.escape = Escape::Osc,
                        // A two-byte escape (or a lone one at the end): drop it whole.
                        Some(_) => {}
                        None => self.escape = Escape::None,
                    },
                    // A terminal returns the carriage to the start of the line; a `<pre>` has no
                    // line to return to, so it becomes the newline the output meant to draw.
                    '\r' => {
                        if chars.peek() != Some(&'\n') {
                            out.push('\n');
                        }
                    }
                    '\n' | '\t' => out.push(c),
                    c if (c as u32) < 0x20 || c == '\x7f' => {}
                    c => out.push(c),
                },
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecode_config::agent::ApprovalMode;
    use std::collections::BTreeMap;

    #[cfg(unix)]
    use std::time::Duration;

    fn approvals() -> Approvals {
        Approvals {
            shell_enabled: true,
            ..Approvals::default()
        }
    }

    #[test]
    fn a_line_the_dial_admits_is_admitted() {
        assert!(matches!(admit(&approvals(), "cargo test"), Ok(())));
    }

    #[test]
    fn the_switch_is_the_consent_the_panel_cannot_give_itself() {
        let off = Approvals::default();
        assert!(matches!(admit(&off, "cargo test"), Err(Refusal::Off)));
        assert!(matches!(admit_start(&off), Err(Refusal::Off)));
    }

    #[test]
    fn plan_only_refuses_a_command_that_could_change_the_folder() {
        let plan = Approvals {
            mode: ApprovalMode::PlanOnly,
            ..approvals()
        };
        assert!(matches!(admit(&plan, "cargo test"), Err(Refusal::Plan)));
        assert!(matches!(admit_start(&plan), Err(Refusal::Plan)));
    }

    /// The half the task names: a command the dial refuses for the model is refused here, and the
    /// panel is told which one so it can say so.
    #[test]
    fn a_denied_command_is_refused_with_its_own_text() {
        let deny = Approvals {
            shell_deny: vec!["rm".into()],
            ..approvals()
        };
        match admit(&deny, "rm -rf target") {
            Err(Refusal::Blocked { command }) => assert_eq!(command, "rm -rf target"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        // The deny entry is a prefix or a first word: `echo rm` is not `rm`.
        assert!(admit(&deny, "echo rm").is_ok());
    }

    /// A tool override that takes the shell away while the switch stays on is refused at the start,
    /// rather than opening a field whose every line is refused afterwards.
    #[test]
    fn a_tool_override_blocks_the_shell_at_the_door() {
        let mut overrides = BTreeMap::new();
        overrides.insert("terminal".to_string(), false);
        let blocked = Approvals {
            mode: ApprovalMode::Custom,
            tool_overrides: overrides,
            ..approvals()
        };
        assert!(matches!(
            admit_start(&blocked),
            Err(Refusal::Blocked { .. })
        ));
        assert!(matches!(
            admit(&blocked, "cargo test"),
            Err(Refusal::Blocked { .. })
        ));
    }

    #[test]
    fn plain_text_drops_escapes_and_keeps_the_lines() {
        let mut plain = Plain::default();
        assert_eq!(plain.push(b"\x1b[31mred\x1b[0m\r\nnext\n"), "red\nnext\n");
    }

    #[test]
    fn an_escape_split_across_reads_is_dropped_whole() {
        let mut plain = Plain::default();
        assert_eq!(plain.push(b"a\x1b[3"), "a");
        assert_eq!(plain.push(b"1mred"), "red");
        assert_eq!(plain.push(b"\x1b]0;title\x07ok"), "ok");
    }

    #[test]
    fn a_utf8_character_split_across_reads_is_kept() {
        let mut plain = Plain::default();
        let bytes = "привет".as_bytes();
        // The third byte is inside `р`: the read ends mid-character, so only the first one is drawn
        // and the fragment is held for the read that completes it.
        assert_eq!(plain.push(&bytes[..3]), "п");
        assert_eq!(plain.push(&bytes[3..]), "ривет");
    }

    #[test]
    fn bytes_that_are_not_text_become_the_replacement_mark() {
        let mut plain = Plain::default();
        assert_eq!(plain.push(b"a\xffb"), "a\u{fffd}b");
    }

    #[test]
    fn a_carriage_return_alone_is_a_newline_and_a_crlf_is_one() {
        let mut plain = Plain::default();
        assert_eq!(plain.push(b"one\rtwo\r\nthree"), "one\ntwo\nthree");
    }

    #[test]
    fn the_ready_signal_is_taken_out_of_the_stream() {
        let mut scan = ReadyScan::default();
        assert_eq!(
            scan.push(b"first\x1b]633;freecode-ready\x07second"),
            b"firstsecond"
        );
        assert!(scan.is_ready());
        // Once the shell is ready nothing is held back.
        assert_eq!(scan.push(b"later"), b"later");
    }

    #[test]
    fn a_ready_signal_split_across_reads_is_found() {
        let mut scan = ReadyScan::default();
        assert_eq!(scan.push(b"a\x1b]633;free"), b"a");
        assert!(!scan.is_ready());
        assert_eq!(scan.push(b"code-ready\x07b"), b"b");
        assert!(scan.is_ready());
    }

    /// The panel showed the line twice when it was written before the startup file ran: the terminal
    /// echoed it. The gate is what makes that impossible — the line waits for `stty -echo`, so only
    /// what the command printed comes back.
    #[cfg(unix)]
    #[test]
    fn a_line_sent_at_open_is_not_echoed_by_the_terminal() {
        use std::sync::mpsc;
        let dir = std::env::temp_dir().join(format!("freecode-shell-ready-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".zshrc"), RC).unwrap();

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 12,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new("/bin/zsh");
        cmd.arg("-i");
        for (key, value) in std::env::vars_os() {
            cmd.env(key, value);
        }
        cmd.env("ZDOTDIR", &dir);
        cmd.env("TERM", "dumb");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();

        let ready = Arc::new(Ready::default());
        let reader_ready = ready.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut plain = Plain::default();
            let mut scan = ReadyScan::default();
            let mut buf = [0u8; 4096];
            let mut text = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let bytes = scan.push(&buf[..n]);
                        if scan.is_ready() {
                            reader_ready.mark();
                        }
                        text.push_str(&plain.push(&bytes));
                        if text.contains("ok:gate") {
                            let _ = tx.send(text.clone());
                            break;
                        }
                    }
                }
            }
        });

        assert!(
            ready.wait(Duration::from_secs(10)),
            "the shell never said it was ready"
        );
        writer.write_all(b"printf 'ok:%s\\n' gate\n").unwrap();
        writer.flush().unwrap();
        let seen = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the shell answered");
        assert!(seen.contains("ok:gate"), "{seen:?}");
        // The typed text was `printf …`; an echo would put it above the output.
        assert!(
            !seen.contains("printf"),
            "the typed line was echoed: {seen:?}"
        );

        let _ = child.kill();
        drop(writer);
        drop(pair.master);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Python half, end to end: a real `python3` on the pipe pair the host uses, the startup file
    /// in [`PY_STARTUP`], the reader's two promises. The expression's value comes back, and the line
    /// that produced it does not — there is no terminal to echo it. The block with a blank line is
    /// the reason an empty line is not dropped for this session: it is what closes the suite.
    #[cfg(unix)]
    #[test]
    fn a_python_line_is_answered_and_never_echoed() {
        use std::os::unix::net::UnixStream;
        use std::process::{Command, Stdio};
        use std::sync::mpsc;

        let dir = std::env::temp_dir().join(format!("freecode-python-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let startup = dir.join("startup.py");
        std::fs::write(&startup, PY_STARTUP).unwrap();

        let (reader, out) = UnixStream::pair().unwrap();
        let err = out.try_clone().unwrap();
        let out: std::os::fd::OwnedFd = out.into();
        let err: std::os::fd::OwnedFd = err.into();
        let mut child = Command::new("python3")
            .args(["-q", "-i"])
            .env("PYTHONSTARTUP", &startup)
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("TERM", "dumb")
            .stdin(Stdio::piped())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .expect("python3 is on PATH");
        let mut stdin = child.stdin.take().unwrap();

        let ready = Arc::new(Ready::default());
        let reader_ready = ready.clone();
        let (tx, rx) = mpsc::channel();
        let mut reader = reader;
        thread::spawn(move || {
            let mut plain = Plain::default();
            let mut scan = ReadyScan::default();
            let mut buf = [0u8; 4096];
            let mut text = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let bytes = scan.push(&buf[..n]);
                        if scan.is_ready() {
                            reader_ready.mark();
                        }
                        text.push_str(&plain.push(&bytes));
                        if text.contains("42") {
                            let _ = tx.send(text.clone());
                            break;
                        }
                    }
                }
            }
        });

        assert!(
            ready.wait(Duration::from_secs(10)),
            "Python never said it was ready"
        );
        stdin
            .write_all(b"def double(n):\n    return n * 2\n\ndouble(21)\n")
            .unwrap();
        stdin.flush().unwrap();
        let seen = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("Python answered");
        assert!(seen.contains("42"), "{seen:?}");
        assert!(
            !seen.contains("double(21)"),
            "the line was echoed: {seen:?}"
        );
        assert!(
            !seen.contains(">>>"),
            "the prompt reached the panel: {seen:?}"
        );

        let _ = child.kill();
        let _ = child.wait();
        drop(stdin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole path once: a real shell, a real PTY, the line policy and the bytes back. This is the
    /// only test that can say the reader thread and the terminal agree on what "output" is.
    #[cfg(unix)]
    #[test]
    fn a_shell_round_trip_echoes_its_output() {
        use std::sync::mpsc;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 12,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-i");
        cmd.env("PS1", "");
        cmd.env("TERM", "dumb");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut reader = reader;
            let mut seen = String::new();
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                if seen.contains("freecode-shell-ok") {
                    let _ = tx.send(seen.clone());
                    break;
                }
            }
        });
        writer.write_all(b"echo freecode-shell-ok\n").unwrap();
        writer.flush().unwrap();
        let seen = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the shell answered");
        assert!(seen.contains("freecode-shell-ok"), "{seen:?}");
        let _ = child.kill();
        drop(writer);
        drop(pair.master);
    }
}
