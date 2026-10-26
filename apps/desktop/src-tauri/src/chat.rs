//! The live engine: one supervised fork server for this workspace, and the chat path that streams a
//! real answer from it into the transcript.
//!
//! This is the phase's proof that the whole local chain works on the machine it will ship for: the
//! same `status::Assembly` the status card renders hands the guard's verdict, the pinned binary and
//! the weight file to `freecode_llama::server::start`, and the reply comes back from that server over
//! the OpenAI wire — no mock, no second implementation. A refusal therefore also arrives here as the
//! guard's own blocker with its install action, which is what the acceptance criteria ask to see.
//!
//! The rules that shape the state:
//!   * **one server at a time.** It is started with `-np 1` so a long run keeps its prompt cache
//!     (Bonsai-demo#183), so a second launch would only fail on the port and a second request would
//!     only queue. `launching` and `streaming` reserve those two slots.
//!   * **a death is a fact, not a retry.** The server is started again when it exits between runs,
//!     a bounded number of times per launch configuration ([`MAX_RESTARTS`]), and the reason it
//!     stopped is announced rather than swallowed by a relaunch that looks like a slow answer.
//!   * **the server never outlives this process.** [`release_engine`] runs on the app-exit path and
//!     on SIGTERM/SIGINT/SIGHUP: a host that dies quietly would leave a bound port and a weight file
//!     in RAM owned by nobody.
//!   * **no lock is held across an `.await`.** A multi-GB load takes seconds and a reply takes
//!     minutes; the window must still repaint. Callers take a snapshot and drop the guard.
//!   * **reasoning is displayed, never replayed.** Only the answer text goes back into the history
//!     the next request is built from (`AgentConstraints::do_not_echo_reasoning_into_history`).
//!   * **a picture needs a server that was started for one.** `--mmproj` is a launcher argument, so a
//!     run that carries an image is answered by a server holding the projector the model's card names,
//!     and reaching that is a deliberate relaunch with its cost announced — see [`Needs`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use freecode_config::{
    ApprovalMode, ModelSource, PackingChoice, ReasoningEffort, Settings, DEFAULT_SYSTEM_PROMPT,
};
use freecode_core::events::{AgentEvent, NoticeCode, RunReport, ToolOutcome};
use freecode_core::transcript::{DiffStat, StepStatus, TranscriptStep};
use freecode_llama::{
    prompt_tokens, start as start_server, ChatRequest, LaunchRequest, LaunchTuning, Message,
    RemoteAccess, Reply, ReplyDelta, ServerHandle, StreamControl, IMAGE_TOKEN_CAP,
};
use freecode_models::{gguf, IntegrityStatus, ModelEntry};
use freecode_tools::exec::EditHunk;
use freecode_tools::{Access, Edit, SandboxPolicy, Summary, SummaryKey};

use crate::agent::{self, Ran};
use crate::free_models::FreeModel;
use crate::{appearance, free_models, opencode, sessions, status, undo};
use sessions::{Role, Session};

/// Event name the transcript subscribes to for the duration of the window.
pub const EVENT: &str = "agent-event";

/// How many rounds of tool calls one answer may take before the loop stops asking.
///
/// This bound is a backstop against a model that never answers, not a description of a task. Four
/// was a description: list, find, read, search — which ran out in the middle of an ordinary change
/// (read the test, edit, run it, fix, run it again). The window is the first guard, because every
/// round replays the conversation plus everything the round before read and the prompt is measured
/// before each follow-up; this is the second, set high enough that a working loop ends by answering.
const MAX_TOOL_ROUNDS: usize = 24;

/// How many times a subagent may ask the engine before it has to answer. Six is a few reads and an
/// answer: a delegated "find where X is implemented" has to list, grep, read two or three files and
/// report. Two was one read and an answer, which left no room to follow a reference it found.
const SUBAGENT_ROUNDS: usize = 6;

/// How much of one read a subagent may bring into its own context. The parent's window is not the
/// subagent's, but both are the same 32k slot: four uncut files could spend it in one round.
const SUBAGENT_TOOL_CHARS: usize = 12_000;

/// How much of a subagent's answer comes back to the parent. The report is folded into the parent's
/// history as one tool result, and a transcript pasted whole would cost the delegation its point.
const SUBAGENT_RESULT_CHARS: usize = 6_000;

/// The one system turn a subagent is given. It names the read-only offer in the model's own terms,
/// because a 1.7B model will otherwise ask for a write the loop does not offer and spend a round.
const SUBAGENT_SYSTEM: &str = "\
You are a subagent inside a local coding assistant. You answer one delegated question about this \
workspace. You can read it with read_file, list_dir, glob and grep_search, and read its repository \
with git_status, git_diff and git_log; you cannot change, create or run anything, and you cannot \
reach the internet. Use the tools you need, then answer in plain prose: what you found, and which \
files say so. Keep the answer short; the assistant that delegated the question may act on it.";

/// The contract a plan-mode run is given instead of the shipped coding one, which opens by telling
/// the model to make the change. Plan mode reads the workspace and answers with the change to make;
/// a contract that says "make it" under a read-only offer spends the run on refused write calls.
///
/// The ending is as load-bearing as the beginning: a 1.7B model asked for a plan will happily start
/// writing the implementation into the answer, and a plan the user cannot approve or reject without
/// reading code is not the thing this mode promises.
const PLAN_SYSTEM: &str = "\
You are planning inside a local coding assistant, in plan mode. Your job is to decide what should \
change and write the plan, not the change. You can explore the workspace with read_file, list_dir, \
glob and grep_search, and read its repository with git_status, git_diff and git_log; you cannot \
write, create, run or reach the internet, and asking for those tools will be refused. Read enough \
to be sure — the files the change touches and the tests around them — then answer with a plan: a \
short paragraph on what you found, then a numbered list of steps, each naming the file it changes \
and the change it makes there. End with the files the plan touches and what you could not verify. \
If nothing needs to change, say so. Do not paste the implementation; the user asks for it once the \
plan reads right.";

/// Sized for the largest pack the catalog offers, not the default one: a cold first load pages 5.95 GB
/// of weights in from disk, and the observed warm load of that file on an M1 was under 11 s, so this is
/// slack for the cold case, not a guess that loading is slow. The 463 MB pack the default entry selects
/// answers `/health` in under a second here.
const READY_TIMEOUT: Duration = Duration::from_secs(180);

/// How many times a server that died after answering `/health` is started again for one launch
/// configuration. The bound exists because the usual reason for that death is one an identical
/// relaunch fixes exactly as well as the first attempt — a context window larger than the memory
/// behind it — and each attempt costs the window up to [`READY_TIMEOUT`] seconds of apparent freeze.
/// Three is a generous reading of "the machine was briefly busy"; past it the host stops and quotes
/// the server's own last lines, because the cause is something the user can see and the host cannot.
const MAX_RESTARTS: u32 = 3;

/// How many lines of a dead server's output a notice quotes.
const NOTICE_LINES: usize = 12;

/// The part of a crashed server's tail that can explain anything.
///
/// `llama-server` prints a timing block every few tokens, so a tail taken while it was decoding is
/// heartbeat from end to end: the notice then fills the transcript without saying why the server
/// stopped, which is the one job it has. Slot progress lines are dropped and what is left is capped,
/// with the count of the lines that were set aside said out loud rather than the tail silently cut.
///
/// A server whose whole tail is heartbeat gets some of it anyway, because the alternative is a notice
/// that quotes nothing at all.
fn excerpt(log: &str) -> String {
    let mut lines = log
        .lines()
        .filter(|line| !(line.contains("print_timing") || line.contains("get_available")))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines = log.lines().collect();
    }
    let hidden = log.lines().count() - lines.len() + lines.len().saturating_sub(NOTICE_LINES);
    let mut text = lines[lines.len().saturating_sub(NOTICE_LINES)..].join("\n");
    if hidden > 0 {
        text.push_str(&format!(
            "\n({hidden} more lines of server output are not quoted.)"
        ));
    }
    text
}

/// What a running server is, as the UI reports it. The command line is included because the whole
/// point of Phase 2 is that these exact flags — not a paraphrase of them — reach the binary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineView {
    pub base_url: String,
    pub backend: String,
    pub engine_tag: String,
    pub packing: String,
    pub model_file: String,
    pub context_tokens: u32,
    pub layers_offloaded: u32,
    pub command: String,
}

/// Everything one turn needs, snapshotted so the request can be built and streamed without touching
/// the state (and the child process) while it runs.
#[derive(Clone)]
struct Turn {
    view: EngineView,
    model: ModelEntry,
    /// The picker's word, after this model's effort contract has had the chance to override it:
    /// `client::wire_effort` still applies the chat template's vocabulary and reports the `low`
    /// downgrade, so this host does not pre-apply the same rule twice.
    effort: String,
    max_tokens: u32,
}

/// A turn answered by a remote provider instead of the supervised local server.
///
/// The key is held for the life of the run and goes on its requests only. It is never filed with the
/// conversation, written to settings, printed in a notice, or returned over IPC — the settings page
/// can ask whether one is saved, never read it back.
#[derive(Clone)]
struct RemoteTurn {
    base_url: String,
    model: String,
    context_tokens: u32,
    max_tokens: u32,
    key: String,
}

/// The gateway's advertised window for a free-model turn. The run's configured budget still caps the
/// answer well below it — this number only decides when a replayed conversation has outgrown the
/// window the provider will accept.
const FREE_CONTEXT_TOKENS: u32 = 32_768;

/// A turn answered by one of the free models behind the single gateway key.
///
/// `model` is the picked entry, not necessarily the one that answers: the registry's attempt order
/// moves a pre-token refusal to the next routable model and says so in the transcript. The key is
/// the same OpenRouter credential the remote-provider row would use, kept under the free tier's own
/// service name, and lives here and on the requests it makes — never filed or logged.
#[derive(Clone)]
struct FreeTurn {
    model: &'static FreeModel,
    key: String,
    max_tokens: u32,
}

/// A turn answered by the OpenCode app on this Mac.
///
/// This is the free tier's other route: the same registry rows, answered by the runtime their models
/// are reserved for, with no gateway key. `session` is the service session the conversation continues
/// in; it is opened on the first turn and remembered on the filed conversation, so a reopened
/// conversation picks the exchange up rather than starting over. `fresh` says this turn opened that
/// session, which is when the system prompt has to be carried to it; `replay` is true for the one
/// turn that had to open a fresh service session under a conversation the app already holds: its
/// prompt then carries the exchange the service has not seen.
///
/// `directory` is the folder the session works in — the project root the rest of this app sandboxes to.
/// It is what makes the route's change visible: the service writes where its session says it works, so
/// binding the folder is what puts a file somewhere Review can show, the inspector can list, and the
/// desktop can open. Writes there are then put to this app, which decides them by the same rule its own
/// loop applies — see [`opencode::project_permissions`]. `None` is a machine whose project folder could
/// not be read: the session opens with nothing it can write, rather than writing where nobody can see.
#[derive(Clone)]
struct OpenCodeTurn {
    service: opencode::Service,
    model: &'static FreeModel,
    max_tokens: u32,
    session: Option<String>,
    fresh: bool,
    replay: bool,
    directory: Option<PathBuf>,
}

/// Where one turn is answered.
///
/// There is deliberately no fallback between the three: a remote selection whose consent switch is
/// off, whose address or pick is incomplete, or whose key is missing is a refusal, not a quiet return
/// to the local engine. Falling back would send a prompt somewhere the user did not choose, or pay a
/// model load they did not ask for, and the refusal names the setting to change instead. The only
/// fallback that exists is inside the free tier itself, where the registry's order is the whole point
/// of the pick.
#[derive(Clone)]
enum Answer {
    // Boxed because the local turn carries the resolved model entry and the server view, and an enum
    // is as large as its widest variant: a provider turn would otherwise copy that whole struct on
    // every move, for a run that never reads it.
    Local(Box<Turn>),
    Remote(RemoteTurn),
    Free(FreeTurn),
    /// The same free registry, answered by the OpenCode app this Mac already runs. No key, and no
    /// bytes leave for anywhere but the local service.
    OpenCode(OpenCodeTurn),
}

impl Answer {
    /// Whether the answer is produced on this machine: by the supervised server, or by the OpenCode
    /// app's own loopback service. Either way no prompt leaves for the internet from here.
    fn is_local(&self) -> bool {
        matches!(self, Self::Local(_) | Self::OpenCode(_))
    }

    fn label(&self) -> String {
        match self {
            Self::Local(turn) => format!("{} ({})", turn.model.name, turn.view.packing),
            Self::Remote(turn) => format!("{} (remote)", turn.model),
            Self::Free(turn) => format!("{} (free)", turn.model.label),
            Self::OpenCode(turn) => format!("{} (FreeCode)", turn.model.label),
        }
    }

    fn window_tokens(&self) -> u32 {
        match self {
            Self::Local(turn) => turn.view.context_tokens,
            Self::Remote(turn) => turn.context_tokens,
            Self::Free(_) | Self::OpenCode(_) => FREE_CONTEXT_TOKENS,
        }
    }

    fn max_tokens(&self) -> u32 {
        match self {
            Self::Local(turn) => turn.max_tokens,
            Self::Remote(turn) => turn.max_tokens,
            Self::Free(turn) => turn.max_tokens,
            Self::OpenCode(turn) => turn.max_tokens,
        }
    }

    fn effort(&self) -> Option<&str> {
        match self {
            Self::Local(turn) => Some(&turn.effort),
            // The local template's effort vocabulary was measured. A remote endpoint's is not, so
            // the field stays off rather than sending a word the provider may reject. The local
            // OpenCode service takes no effort switch at all.
            Self::Remote(_) | Self::Free(_) | Self::OpenCode(_) => None,
        }
    }
}

fn remote_access(turn: &RemoteTurn) -> RemoteAccess {
    RemoteAccess {
        base_url: turn.base_url.clone(),
        model: turn.model.clone(),
        key: turn.key.clone(),
    }
}

/// A launched server plus the settings it was launched under.
struct Running {
    handle: ServerHandle,
    turn: Turn,
    /// What this server was started with, kept so a death can be compared against a *change*: the
    /// restart budget is a budget per configuration, not per app run. See [`MAX_RESTARTS`].
    tuning: LaunchTuning,
    /// The weights this server actually loaded, which is what a later request has to match before it
    /// may reuse the answer it would otherwise get from another model.
    weights: ModelKey,
    /// Set when the model's effort contract replaced the saved choice, so the launch can announce it
    /// rather than the override looking like a silent default.
    effort_swap: Option<EffortSwap>,
}

impl Running {
    fn new(
        handle: ServerHandle,
        assembly: &status::Assembly,
        settings: &Settings,
        tuning: LaunchTuning,
    ) -> Self {
        let view = EngineView {
            base_url: handle.base_url().to_string(),
            backend: handle.plan.decode_backend().label().to_string(),
            engine_tag: assembly.lock.pinned_tag.clone(),
            packing: assembly.pack.key.clone(),
            model_file: assembly.file.display().to_string(),
            context_tokens: handle.context_tokens(),
            layers_offloaded: handle.plan.layers_offloaded,
            command: handle.plan.display.clone(),
        };
        let (effort, effort_swap) = resolve_effort(&assembly.model, settings.sampling.effort);
        Self {
            handle,
            turn: Turn {
                view,
                model: assembly.model.clone(),
                effort,
                max_tokens: settings.sampling.budget.clamped().max_tokens,
            },
            tuning,
            weights: model_key(settings),
            effort_swap,
        }
    }
}

/// Which weights a request asks for, in the user's own words rather than as a resolved path.
///
/// This is the half of "the same configuration" [`LaunchTuning`] does not carry: two requests can want
/// the same context size, backend and thinking mode and still need different servers, because one of
/// them names another model. A running server holds its weights in RAM and cannot be handed another
/// file, so the pair decides whether the live one may answer at all.
#[derive(Clone, Debug, PartialEq)]
struct ModelKey {
    catalog_id: String,
    packing: PackingChoice,
}

fn model_key(settings: &Settings) -> ModelKey {
    ModelKey {
        catalog_id: settings.local_model.catalog_id.clone(),
        packing: settings.local_model.packing.clone(),
    }
}

/// One supervised server that was found dead: what it said last, and the configuration it died under.
struct Crash {
    log: String,
    tuning: LaunchTuning,
    weights: ModelKey,
}

/// What the host is about to do about the engine not running.
#[derive(Debug, PartialEq)]
enum Launch {
    /// Nothing has died: an ordinary first start.
    First,
    /// The `attempt`th restart after a death, carrying that death's log so it can be announced.
    Restart { attempt: u32, log: String },
}

#[derive(Default)]
struct Slot {
    running: Option<Running>,
    /// A launch is in flight off-lock, so a second click cannot start a second 6 GB server.
    launching: bool,
    /// A reply is streaming.
    streaming: bool,
    /// Deaths that no completed answer has paid for, and the most recent one. See [`MAX_RESTARTS`].
    deaths: u32,
    crash: Option<Crash>,
}

impl Slot {
    /// Decide whether a start may proceed, and reserve the slot if it may.
    ///
    /// A death under a *different* configuration is not the same failure, so changing what the
    /// launcher actually uses (context size, GPU offload, thinking mode, flash attention) — or which
    /// weights it points at — re-arms the budget. That is the documented fix for the common cause, so
    /// the bound never traps a user who is working the problem — it only stops the host from
    /// re-running a launch that just failed.
    fn begin(&mut self, tuning: &LaunchTuning, weights: &ModelKey) -> Result<Launch, String> {
        if self.launching {
            return Err("the engine is already starting".into());
        }
        if self
            .crash
            .as_ref()
            .is_some_and(|c| &c.tuning != tuning || &c.weights != weights)
        {
            self.deaths = 0;
            self.crash = None;
        }
        let launch = match &self.crash {
            None => Launch::First,
            Some(crash) if self.deaths > MAX_RESTARTS => return Err(self.refused(crash)),
            Some(crash) => Launch::Restart {
                attempt: self.deaths,
                log: crash.log.clone(),
            },
        };
        self.launching = true;
        Ok(launch)
    }

    /// Record a death observed between runs, so the next start is a restart with a log to quote.
    fn note_death(&mut self, crash: Crash) {
        self.deaths += 1;
        self.crash = Some(crash);
    }

    /// Hand the whole restart budget back.
    ///
    /// Two things earn that: an answer that finished, which proves the current configuration works,
    /// and an explicit Start or Stop, which is the user asking for another try at something the host
    /// cannot see changed.
    fn re_arm(&mut self) {
        self.deaths = 0;
        self.crash = None;
    }

    /// The refusal a spent budget produces.
    fn refused(&self, crash: &Crash) -> String {
        format!(
            "The engine exited {} times in a row after {} automatic restarts, so it is not being \
             started again. Its last lines:\n{}\n\nA server that stops after it became healthy \
             usually cannot fit what it was asked to fit: lower Context window in Settings, or turn \
             off Offload every layer to the GPU. Any change to those settings is tried again.",
            self.deaths,
            self.deaths - 1,
            excerpt(&crash.log)
        )
    }
}

/// Managed state: the supervised server and the conversation the next request is built from.
#[derive(Default)]
pub struct ChatState {
    slot: Mutex<Slot>,
    history: Mutex<Vec<Message>>,
    /// The same history as it is filed on disk, or `None` between New task and the next prompt.
    /// Kept beside the history rather than rebuilt from it because the file carries what the history
    /// cannot: the conversation's name, its workspace, and the model that answered.
    session: Mutex<Option<Session>>,
    /// The last run's fill as the server counted it, paired with the window it was counted against:
    /// `(window, tokens)`. Only [`outgrew_window`] reads it, and a pairing is what keeps a Settings
    /// change that doubles the slot from inheriting the old one's verdict.
    last_fill: Mutex<Option<(u32, u64)>>,
    runs: AtomicU64,
    /// What this run changed on disk, keyed by the workspace-relative path: the line counts every
    /// accepted edit cost. Keyed rather than summed because a file edited three times is one file
    /// changed and every one of those lines still counts — which is the one thing [`DiffStat::merge`]
    /// cannot say, since it takes the larger of two files-changed counts rather than the union.
    staged: Mutex<BTreeMap<String, (u32, u32)>>,
    /// The text each write replaced, for the review panel's revert. Belongs to this conversation, so
    /// New task and opening a filed one both clear it: their files are not at the states held here.
    undo: Mutex<undo::Journal>,
    /// The last "this route sends no effort field" announcement, as `(model label, chosen word)`.
    /// Keyed rather than a plain flag because changing the model or the dial is a new fact to
    /// report, while repeating the same sentence beside every prompt is only noise.
    effort_noted: Mutex<Option<(String, String)>>,
    /// Set by [`chat_stop`] and read by the run in flight at the next batch of deltas. A plain
    /// flag rather than a dropped future because the answer streamed so far has to survive: the
    /// request ends at a chunk boundary and returns what it had.
    stop_requested: Arc<AtomicBool>,
}

impl ChatState {
    /// Open the run's ledger. Called before the request goes out, so a run that changes nothing
    /// reports nothing rather than quoting the last run's numbers.
    pub fn begin_run(&self) {
        lock(&self.staged).clear();
    }

    /// Charge one accepted edit to the ledger and report the run's total so far, which is what the
    /// pill above the composer shows while the run is still going.
    pub fn stage(&self, path: &str, added: u32, removed: u32) -> DiffStat {
        {
            let mut held = lock(&self.staged);
            let entry = held.entry(path.to_string()).or_insert((0, 0));
            entry.0 += added;
            entry.1 += removed;
        }
        self.staged_diff().expect("the ledger has a row now")
    }

    /// Hold what one call wrote, so the review can put it back.
    pub fn remember(&self, entries: Vec<undo::Entry>) {
        lock(&self.undo).record(entries);
    }

    /// The run's diff, or `None` while it has changed nothing — the difference between a pill that
    /// says "+12 −4" and one that says "+0 −0" about a run that only read files.
    pub fn staged_diff(&self) -> Option<DiffStat> {
        let held = lock(&self.staged);
        if held.is_empty() {
            return None;
        }
        Some(DiffStat {
            lines_added: held.values().map(|(added, _)| added).sum(),
            lines_removed: held.values().map(|(_, removed)| removed).sum(),
            files_changed: held.len() as u32,
        })
    }
}

/// Stop and release every server this process owns.
///
/// The kill is `ServerHandle`'s synchronous `Drop`, so it lands even when the caller is about to exit
/// the process and even once the async runtime is gone. Both app-exit paths need it: a `llama-server`
/// that outlives its host keeps the port bound and the weights resident, and nothing can reach it
/// again. SIGKILL is the right signal here — the server has no state to flush, only memory to return.
///
/// The registry flush is the second half: while a server is loading it is a child with no handle yet
/// (`start` builds the handle only once `/health` answers), so a signal that arrives mid-load would
/// otherwise orphan exactly the process the slot cannot name.
pub fn release_engine(state: &ChatState) -> bool {
    let had_one = {
        let mut slot = lock(&state.slot);
        slot.streaming = false;
        slot.launching = false;
        slot.running.take().is_some()
    };
    freecode_llama::kill_spawned_servers();
    had_one
}

/// A poisoned lock still holds a valid `Slot` — the only code that can panic while one is held
/// assigns these fields — so the guard is reclaimed rather than turned into a user-facing failure.
fn lock<T>(cell: &Mutex<T>) -> MutexGuard<'_, T> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

fn emit(app: &AppHandle, event: AgentEvent) {
    let _ = app.emit(EVENT, event);
}

/// What the next request needs from the engine beyond the text it replays.
///
/// A tag rather than a `bool` because the value at the call site is the whole decision: `--mmproj`
/// cannot be handed to a server that is already up, so asking for a picture is asking for a different
/// process, and that is worth naming where it is passed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    TextOnly,
    Projector,
}

impl Needs {
    fn from_request(carries_image: bool) -> Self {
        if carries_image {
            Needs::Projector
        } else {
            Needs::TextOnly
        }
    }
}

/// Where the projector a picture-run needs comes from, for the model the guard selected.
#[derive(Debug)]
enum Projector {
    /// The model's card publishes no projector at all. No file on this machine would make it read a
    /// picture, which is a fact about the model rather than about the download.
    Unsupported,
    /// The card names one and this machine does not have it, or has a file of the wrong length —
    /// which is what an interrupted transfer leaves, and the download is the fix for both.
    Absent { file: String },
    /// The card names it and it is here at the published size.
    Present { file: String, path: PathBuf },
}

/// Find the projector the model's card recommends, the same size-only probe the packs get.
///
/// The catalog's `recommended` pick is the one that goes out, because a projector has to match the
/// language pack under it and the card is what says which one does. A torn file is `Absent` rather
/// than a second sentence: the status card's download re-fetches it and says so on the way.
fn projector(weights: &Path, model: &ModelEntry) -> Projector {
    let Some(proj) = model.recommended_vision() else {
        return Projector::Unsupported;
    };
    let path = weights.join(&proj.file);
    let present = matches!(
        gguf::check_size(&path, proj.size_bytes),
        Ok(IntegrityStatus::Ok)
    );
    if present {
        Projector::Present {
            file: proj.file.clone(),
            path,
        }
    } else {
        Projector::Absent {
            file: proj.file.clone(),
        }
    }
}

/// The user's setting translated into the launcher's vocabulary.
///
/// `offload_all_layers` off is the CPU-decode path (`-ngl 0`), and the context field is documented
/// in the Settings surface as "tokens the server reserves", so it goes to `-c` as given rather than
/// being second-guessed by the RAM tier.
///
/// `vision_projector` is `None` unless a request asked for one and [`projector`] found it on disk: a
/// projector is resident memory the text path never uses, so it is loaded for the runs that read
/// pictures rather than for every server the app starts.
fn tuning(settings: &Settings, vision_projector: Option<PathBuf>) -> LaunchTuning {
    LaunchTuning {
        context_tokens: Some(settings.sampling.budget.clamped().context_tokens),
        layers_override: (!settings.local_model.offload_all_layers).then_some(0),
        flash_attention: settings.local_model.flash_attention,
        dedicated_server_slots: settings.local_model.dedicated_server_slots,
        thinking: !settings.sampling.non_thinking_mode,
        vision_projector,
        ..LaunchTuning::default()
    }
}

fn effort_word(effort: ReasoningEffort) -> &'static str {
    match effort {
        // Never reaches a request: `resolve_effort` answers `auto` from the catalog first, so the
        // word only names a choice the user actually made.
        ReasoningEffort::Auto => "auto",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::Xhigh => "xhigh",
        ReasoningEffort::None => "none",
    }
}

/// The saved choice a run has to report as unsent, if it made one this route cannot keep.
///
/// `auto` is not a promise: it asks for whatever the model does by itself, which is exactly what a
/// route with no effort field does. The other four words are choices, and a route that sends none of
/// them would otherwise look like the dial had taken effect.
fn unheard_effort(stored: ReasoningEffort, sends_field: bool) -> Option<&'static str> {
    if sends_field {
        return None;
    }
    (stored != ReasoningEffort::Auto).then_some(effort_word(stored))
}

/// A saved reasoning effort the selected model does not accept, as the four facts a notice about it
/// needs. The words are the locale catalog's; these are what its sentence interpolates, so the
/// substitution can be explained in the language the interface is being read in.
#[derive(Debug, PartialEq)]
struct EffortSwap {
    model: String,
    /// The model's accepted efforts, quoted and comma-joined the way the sentence lists them.
    accepted: String,
    /// The picker's word that was not among them.
    chosen: String,
    /// The word the run went out with instead.
    sent: String,
}

/// The effort this model will take, plus the substitution it had to make.
///
/// `client::wire_effort` rejects an effort the loaded model's template does not accept, so a choice
/// saved while another model was selected would turn every request into an error. The catalog's
/// `default_effort` stands in, and the substitution is reported rather than applied quietly.
///
/// `auto` takes that default with no report: nothing is being overridden, so announcing it would be
/// the app describing its own default to the user on every launch. A saved word is different — the
/// user picked it, and the notice is what tells them the model they now have cannot take it.
///
/// `low` is left alone even when unsupported: `wire_effort` maps it onto the catalog's measured
/// synonym, and the Settings control already labels that downgrade.
fn resolve_effort(model: &ModelEntry, stored: ReasoningEffort) -> (String, Option<EffortSwap>) {
    let r = &model.sampling.reasoning;
    if stored == ReasoningEffort::Auto {
        return (r.default_effort.clone(), None);
    }
    let word = effort_word(stored).to_string();
    let translatable = word == "low" && !r.low_behaves_like.is_empty();
    if r.supported_efforts.contains(&word) || translatable {
        return (word, None);
    }
    let accepted = r
        .supported_efforts
        .iter()
        .map(|e| format!("\"{e}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let default = &r.default_effort;
    (
        default.clone(),
        Some(EffortSwap {
            model: model.name.clone(),
            accepted,
            chosen: word,
            sent: default.clone(),
        }),
    )
}

/// Resolve the projector a picture-run has to have, announcing what stands in its way.
///
/// Answers the file name beside the path because the notice that fires when it is missing names the
/// file the user is being asked to fetch. Only a run that carries a picture pays this: the assembly
/// re-read here is the guard's directory scan and host probe, and the text path already gets the
/// server the slot holds without it.
///
/// An `Err` is a refusal the transcript has already been told about in its own language — the same
/// split `launch` uses for a guard blocker, where the string only has to identify the problem to
/// whoever reads the log.
async fn projector_for(app: &AppHandle) -> Result<(String, PathBuf), String> {
    let (root, settings) = crate::root_and_settings(app)?;
    let blocking_root = root.clone();
    let probe_settings = settings.clone();
    let assembly = tauri::async_runtime::spawn_blocking(move || {
        status::assemble(&blocking_root, &probe_settings)
    })
    .await
    .map_err(|e| format!("engine resolution failed: {e}"))??;
    let weights = status::weights_dir(&root, &settings);
    let model = assembly.model.name.clone();
    match projector(&weights, &assembly.model) {
        Projector::Unsupported => {
            emit(
                app,
                AgentEvent::notice(NoticeCode::ModelTakesNoImages, &[("model", model.as_str())]),
            );
            Err(format!("{model} publishes no projector file"))
        }
        Projector::Absent { file } => {
            emit(
                app,
                AgentEvent::notice(
                    NoticeCode::ProjectorAbsent,
                    &[("model", model.as_str()), ("file", file.as_str())],
                ),
            );
            Err(format!(
                "{model}'s projector {file} is not in {}",
                weights.display()
            ))
        }
        Projector::Present { file, path } => Ok((file, path)),
    }
}

/// Start the supervised server, or report the guard's refusal to allow it.
///
/// The refusal is emitted as `EngineBlocked` rather than only returned: the transcript has to show
/// the blocker with its install action, which is the shape the acceptance criteria name.
async fn launch(app: &AppHandle, vision_projector: Option<PathBuf>) -> Result<Running, String> {
    let (root, settings) = crate::root_and_settings(app)?;
    let blocking_root = root.clone();
    let probe_settings = settings.clone();
    let assembly = tauri::async_runtime::spawn_blocking(move || {
        status::assemble(&blocking_root, &probe_settings)
    })
    .await
    .map_err(|e| format!("engine resolution failed: {e}"))??;

    if let Some(blocker) = assembly.report.first_blocker() {
        let info = freecode_core::blocked_info(blocker, &assembly.lock, &assembly.model.repo_id);
        emit(app, AgentEvent::EngineBlocked { info: info.clone() });
        // The refusal the user reads is the transcript row above, in their language. This string only
        // has to identify the problem to whoever reads the log, so it names the code and quotes the
        // evidence rather than carrying a second copy of a sentence the catalogs own.
        return Err(format!(
            "engine refused: {} — {}",
            info.code.as_str(),
            info.technical
        ));
    }
    let engine = assembly
        .engine
        .as_ref()
        .ok_or("the load guard allowed a run with no resolved engine")?;

    let tuning = tuning(&settings, vision_projector);
    let handle = start_server(
        LaunchRequest {
            root: &root,
            engine,
            model: &assembly.model,
            defaults: &assembly.defaults,
            model_path: assembly.file.clone(),
            hardware: &assembly.host,
            tuning: tuning.clone(),
            pid_registry: crate::paths::server_pid_registry(),
        },
        &assembly.lock,
        READY_TIMEOUT,
    )
    .await
    .map_err(|e| e.to_string())?;

    let running = Running::new(handle, &assembly, &settings, tuning);
    if let Some(swap) = &running.effort_swap {
        emit(
            app,
            AgentEvent::notice(
                NoticeCode::EffortNotSupported,
                &[
                    ("model", swap.model.as_str()),
                    ("accepted", swap.accepted.as_str()),
                    ("chosen", swap.chosen.as_str()),
                    ("sent", swap.sent.as_str()),
                ],
            ),
        );
    }
    Ok(running)
}

/// What the slot holds that is still a child process, judged against the configuration this run needs.
enum Live {
    /// No server, or one that exited since it answered `/health` — that case is reaped here, its death
    /// parked for [`Slot::begin`], because a relaunch has to know what it is replacing.
    Nothing,
    /// A server that is alive and answers this run as it stands. Boxed because a `Turn` carries the
    /// whole request, and an enum sized by it would make every one of these returns a 1.3 KB copy.
    Reuse(Box<Turn>),
    /// A server that is alive but was started without what this run needs, so it is being replaced on
    /// purpose. Nothing died: the handle comes out so the caller can shut it down with no lock held
    /// and the restart budget stays where it was. Boxed for the same reason as [`Live::Reuse`].
    Replace(Box<ServerHandle>),
}

/// The live server, if the child is still alive — and only a reuse if it can answer at all.
///
/// A server that exited since it answered `/health` is released here, which reaps the child and frees
/// its port, and the death is parked in the slot with its log: whether another one gets started is
/// [`Slot::begin`]'s decision, since that is the place which knows both the budget and the
/// configuration the next launch would use.
fn live(app: &AppHandle, wanted: &LaunchTuning, weights: &ModelKey) -> Live {
    let state = app.state::<ChatState>();
    let mut slot = lock(&state.slot);
    let Some(running) = slot.running.as_mut() else {
        return Live::Nothing;
    };
    if !running.handle.is_running() {
        let crash = Crash {
            log: running.handle.log_tail(),
            tuning: running.tuning.clone(),
            weights: running.weights.clone(),
        };
        slot.running = None;
        slot.note_death(crash);
        return Live::Nothing;
    }
    // Both halves, because the tuning says nothing about which weights are resident: a server started
    // for one model answers a request for another with the first model's words, which reads as the
    // switch having worked.
    if &running.tuning == wanted && &running.weights == weights {
        return Live::Reuse(Box::new(running.turn.clone()));
    }
    let running = slot
        .running
        .take()
        .expect("a live server was just borrowed");
    Live::Replace(Box::new(running.handle))
}

async fn ensure_running(app: &AppHandle, needs: Needs) -> Result<Turn, String> {
    let settings = appearance::load_settings()?;
    // Resolved before the slot is read, because whether this run can reuse the live server is exactly
    // the question the projector answers, and a model that cannot take the picture is refused here
    // rather than after a launch that would have to refuse it anyway.
    let (vision_file, vision_projector) = match needs {
        Needs::TextOnly => (None, None),
        Needs::Projector => {
            let (file, path) = projector_for(app).await?;
            (Some(file), Some(path))
        }
    };
    let wanted = tuning(&settings, vision_projector.clone());
    let weights = model_key(&settings);
    match live(app, &wanted, &weights) {
        Live::Reuse(turn) => return Ok(*turn),
        Live::Replace(handle) => {
            // The wait is the picture's cost, and it is the reload rather than the answer that takes the
            // seconds, so it is named before the shutdown instead of looking like a slow first reply. A
            // run that needs no projector is replacing the live server for some other reason — the
            // settings moved under it — and that relaunch is the guard's to describe, so this stays
            // quiet rather than sending a notice whose file name would be blank.
            if let Some(file) = &vision_file {
                emit(
                    app,
                    AgentEvent::notice(NoticeCode::ProjectorLoading, &[("file", file.as_str())]),
                );
            }
            handle.shutdown().await;
        }
        Live::Nothing => {}
    }
    let kind = {
        let state = app.state::<ChatState>();
        let mut slot = lock(&state.slot);
        slot.begin(&wanted, &weights)?
    };
    // Announced, not swallowed: a relaunch that said nothing would just look like a slow first
    // answer, and the reason the last one died is the only clue to what to change.
    if let Launch::Restart { attempt, log } = kind {
        let attempt = attempt.to_string();
        let max = MAX_RESTARTS.to_string();
        let log = excerpt(&log);
        emit(
            app,
            AgentEvent::notice(
                NoticeCode::EngineRestarted,
                &[
                    ("attempt", attempt.as_str()),
                    ("max", max.as_str()),
                    ("log", log.as_str()),
                ],
            ),
        );
    }
    let outcome = launch(app, vision_projector).await;
    let state = app.state::<ChatState>();
    let mut slot = lock(&state.slot);
    slot.launching = false;
    let running = outcome?;
    let turn = running.turn.clone();
    // Replacing rather than adding: the displaced handle's `Drop` kills its child, so two servers can
    // never end up owned by one slot.
    slot.running = Some(running);
    Ok(turn)
}

/// Resolve where this run is answered before anything is launched.
///
/// The free pick is checked first because it is the most recent choice the picker recorded, then the
/// remote source: that order is what makes either selection mean what it says. A missing key refuses
/// at the chosen step, and the remote-provider consent switch refuses only the remote source; the
/// local engine is never started as a substitute.
async fn ensure_answer(app: &AppHandle, needs: Needs) -> Result<Answer, String> {
    let settings = appearance::load_settings()?;
    if settings.free_model.is_some() {
        return free_answer(app, &settings).await;
    }
    if settings.remote_provider.source == ModelSource::Remote {
        return remote_turn(&settings).await.map(Answer::Remote);
    }
    ensure_running(app, needs)
        .await
        .map(|turn| Answer::Local(Box::new(turn)))
}

/// Everything a provider turn needs except the credential: consent, address, model and budgets.
///
/// Split from [`remote_turn`] so the refusals can be tested without reading or writing a Keychain
/// item. Every refusal names the place to change, and none of them falls back to the local engine.
fn remote_plan(settings: &Settings) -> Result<(String, String, u32, u32), String> {
    let provider = &settings.remote_provider;
    if !settings.privacy.allow_remote_providers {
        return Err(
            "Remote providers are off in Settings → Privacy, so this prompt was not sent anywhere. \
             Turn that switch on to use the address saved under Settings → Model → Remote provider."
                .into(),
        );
    }
    if !provider.configured() {
        return Err(
            "the remote provider has no base URL or model id; set both under Settings → Model → \
             Remote provider"
                .into(),
        );
    }
    let base_url = crate::provider::normalize_base_url(&provider.base_url)?;
    if provider.context_tokens == 0 {
        return Err(
            "the remote provider's context window is 0; set the window the provider reports".into(),
        );
    }
    // The run's configured budget, capped by half the declared window for the same reason the local
    // clamp uses half: the prompt and the answer share one slot.
    let max_tokens = settings
        .sampling
        .budget
        .clamped()
        .max_tokens
        .min(provider.context_tokens / 2)
        .max(1);
    Ok((
        base_url,
        provider.model.trim().to_string(),
        provider.context_tokens,
        max_tokens,
    ))
}

/// The provider turn, admitted only after consent, address and credential all check out.
///
/// The key is read from the Keychain on the blocking pool and lives in this struct and the request
/// it makes; nothing here logs or echoes it.
async fn remote_turn(settings: &Settings) -> Result<RemoteTurn, String> {
    let (base_url, model, context_tokens, max_tokens) = remote_plan(settings)?;
    let key = tauri::async_runtime::spawn_blocking(crate::provider::load_key)
        .await
        .map_err(|e| format!("reading the saved key stopped: {e}"))??;
    Ok(RemoteTurn {
        base_url,
        model,
        context_tokens,
        max_tokens,
        key,
    })
}

/// Everything a free turn needs except the credential: the picked registry entry and the budget.
///
/// Split from the two transports below like [`remote_plan`] is from [`remote_turn`], for the same
/// reason: the refusals are testable without a Keychain or a running service. Every refusal names the
/// place to change, and none of them falls back to the local engine.
///
/// A free pick does not consult the remote-provider consent switch: that switch governs the address
/// the user types under Settings → Model → Remote provider, and a free model is not that address.
fn free_choice(settings: &Settings) -> Result<(&'static FreeModel, u32), String> {
    let Some(id) = settings.free_model.as_deref() else {
        return Err(
            "no free model is selected; pick one in Settings → Model → Model picker".into(),
        );
    };
    let Some(model) = free_models::find(id) else {
        return Err(format!(
            "the selected free model is not in the registry: {id}"
        ));
    };
    // The run's configured budget, capped by half the gateway's window for the same reason the
    // remote clamp uses half: the prompt and the answer share one slot.
    let max_tokens = settings
        .sampling
        .budget
        .clamped()
        .max_tokens
        .clamp(1, FREE_CONTEXT_TOKENS / 2);
    Ok((model, max_tokens))
}

/// The gateway route's admission: the shared checks, plus an upstream id the relay can address.
fn free_plan(settings: &Settings) -> Result<(&'static FreeModel, u32), String> {
    let (model, max_tokens) = free_choice(settings)?;
    if model.upstream.is_none() {
        return Err(format!(
            "{} is a promotion listed here and not routable from this app; pick another free model",
            model.label
        ));
    }
    Ok((model, max_tokens))
}

/// The free pick, answered by whichever route is live.
///
/// The OpenCode app on this Mac comes first: its service is the runtime the registry's models are
/// reserved for, it needs no gateway key, and it is the only route that carries the promotion rows.
/// The gateway is the fallback, and only for a pick it actually lists — a promotion with no upstream
/// id is refused with the reason rather than sent to a model that is not there.
async fn free_answer(app: &AppHandle, settings: &Settings) -> Result<Answer, String> {
    let (model, max_tokens) = free_choice(settings)?;
    if let Some(service) = opencode::ready().await {
        // Resolved here rather than per turn, because a session keeps the folder it opened with: what
        // the run is asked to change is decided once, and the review's paths are read against it.
        // A folder that cannot be read — deleted, or never picked and the default gone — leaves the
        // session with nothing to write, which the route says on the row rather than as a refusal: a
        // question still deserves its answer, and a plain chat needs no folder at all.
        let directory = crate::paths::project_root(Some(app), settings).ok();
        return Ok(Answer::OpenCode(OpenCodeTurn {
            service,
            model,
            max_tokens,
            session: None,
            fresh: true,
            replay: false,
            directory,
        }));
    }
    // A promotion with no upstream id is served only by the service above, so its absence is the
    // reason to give: "not routable" would send the user to a picker whose other promotions the same
    // missing service would refuse next. This is the first-run state of a build that defaults onto
    // a promotion, so it says what to start rather than what the registry thinks of the row.
    if model.upstream.is_none() {
        return Err(format!(
            "{} is served by the free-model service on this Mac, which is not answering; start it, \
             or pick a free model the gateway carries",
            model.label
        ));
    }
    let (model, max_tokens) = free_plan(settings)?;
    // Read from the Keychain on the blocking pool and lives in this struct and the requests it
    // makes; nothing here logs or echoes it.
    let key = tauri::async_runtime::spawn_blocking(free_models::load_key)
        .await
        .map_err(|e| format!("reading the saved key stopped: {e}"))??;
    Ok(Answer::Free(FreeTurn {
        model,
        key,
        max_tokens,
    }))
}

/// Point an OpenCode turn at the service session this conversation continues in, opening one when
/// there is none to continue.
///
/// The id is remembered on the filed conversation, so reopening it tomorrow asks the same service
/// session what the window still shows. A remembered session the service has dropped — it was
/// restarted, or OpenCode forgot it — opens a new one, and the turn is marked `replay` so the prompt
/// carries the exchange the service has not seen. That replay is also why a conversation that was
/// filed before this route existed still continues instead of starting blank.
async fn open_service_session(state: &ChatState, answer: Answer) -> Result<Answer, String> {
    let mut turn = match answer {
        Answer::OpenCode(turn) => turn,
        other => return Ok(other),
    };
    let held = lock(&state.session)
        .as_ref()
        .and_then(|s| s.opencode_session.clone());
    if let Some(id) = held {
        // A service that cannot be asked is not evidence the session is gone; the prompt below
        // would fail on the same connection, so only a clear "no" from a live service reopens it.
        if opencode::session_lives(&turn.service, &id)
            .await
            .unwrap_or(false)
        {
            // The picker may have moved since this conversation last ran; the session answers with
            // whatever model it was last told, so it is told again.
            opencode::switch_model(&turn.service, &id, turn.model.id).await?;
            turn.session = Some(id);
            turn.fresh = false;
            return Ok(Answer::OpenCode(turn));
        }
    }
    // The prompt this turn carries is already in the history beside the turns the service has not
    // seen; more than one message means there is an exchange to replay.
    turn.replay = lock(&state.history).len() > 1;
    let id =
        opencode::create_session(&turn.service, turn.model.id, turn.directory.as_deref()).await?;
    turn.session = Some(id.clone());
    let filed = {
        let mut cell = lock(&state.session);
        cell.as_mut().map(|session| {
            session.opencode_session = Some(id);
            session.clone()
        })
    };
    // A conversation that will not take the id is one the next turn replays; that is the honest
    // fallback and not a reason to fail a turn the service has already opened.
    if let Some(session) = filed {
        let _ = sessions::write(&session);
    }
    Ok(Answer::OpenCode(turn))
}

/// The request for one turn: the system prompt as it is configured now, then the stored history.
fn conversation(state: &ChatState, system_prompt: &str) -> Vec<Message> {
    let history = lock(&state.history);
    let mut messages = Vec::with_capacity(history.len() + 1);
    if !system_prompt.trim().is_empty() {
        messages.push(Message::system(system_prompt));
    }
    messages.extend(history.iter().cloned());
    messages
}

/// The user's side of a turn, with its picture still in it.
///
/// One place builds this because three readers have to agree on what was asked: the request that goes
/// out now, the copy filed beside it, and the history a reopened conversation is rebuilt from.
fn prompt(text: &str, image: Option<String>) -> Message {
    match image {
        Some(url) => Message::user_with_image(text, url),
        None => Message::user(text),
    }
}

/// The tokens the pictures in a request cost beyond what the server just counted.
///
/// [`prompt_tokens`] asks the server to template the messages and tokenize the string it gets back,
/// and a picture is in that string only as the placeholder the template writes for it — the vision
/// tokens the projector expands it into never pass through `/tokenize`. So the pre-check charges the
/// launcher's own cap, once per picture, because the replay sends all of them again. Where the
/// launcher leaves the cap off (a CUDA or ROCm decode, see [`IMAGE_TOKEN_CAP`]) this is the cheap
/// reading of the cost, which is the direction a warning that arrives late is safer than one that
/// arrives never.
fn picture_tokens(request: &ChatRequest) -> u64 {
    request
        .messages
        .iter()
        .map(|m| m.images.len() as u64 * IMAGE_TOKEN_CAP as u64)
        .sum()
}

/// The two halves of an interruption, which always travel together: the flag a Stop press sets for
/// whichever run is live, and the flag that run sets to record that it saw the press. One round that
/// could halt a stream without recording the halt would leave the transcript holding an answer and no
/// way to say it was cut short.
struct Stops<'a> {
    requested: &'a AtomicBool,
    stopped: &'a AtomicBool,
}

/// Stream one round of an answer.
///
/// `offered` is whether this run put tools on the request. A call arriving without any is the model
/// emitting one anyway and has to be said out loud, because the answer text is truncated at that
/// point and silence reads as a stalled server. With tools offered the same delta is simply the call
/// the loop is about to run, and [`agent::run_batch`] gives it a row — two announcements for one event
/// would put the same tool in the transcript twice.
///
/// The free arm is the one place fallback exists: a refusal that arrives before the first delta moves
/// the request to the next routable model in the registry's order, announced through `on_fallback`
/// so the transcript can say which model was asked and which one answered. Once a delta has landed
/// the answer belongs to that model — a provider that dies mid-sentence is reported, not quietly
/// restarted as a different model's words.
///
/// `gate` and `on_write` are read by the OpenCode arm alone, which runs its tool loop inside the
/// service rather than through [`agent`]: the service asks before it writes, this app answers, and what
/// became of the write comes back as a row. No other route asks — every call it makes already goes
/// through [`agent`]'s sandbox and gets its row there.
async fn stream_round(
    answer: &Answer,
    request: &ChatRequest,
    stop: &AtomicBool,
    mut on_delta: impl FnMut(&[ReplyDelta]) -> StreamControl,
    mut on_fallback: impl FnMut(&'static str, &'static str),
    gate: impl FnMut(&opencode::Request) -> opencode::Verdict,
    on_write: impl FnMut(&opencode::Write),
) -> Result<Reply, String> {
    match answer {
        Answer::Local(turn) => freecode_llama::stream_chat(
            &turn.view.base_url,
            request,
            &turn.model,
            turn.view.context_tokens,
            on_delta,
        )
        .await
        .map_err(|e| e.to_string()),
        Answer::Remote(turn) => {
            freecode_llama::stream_remote_chat(&remote_access(turn), request, on_delta)
                .await
                .map_err(|e| e.to_string())
        }
        Answer::Free(turn) => {
            // The order the registry picked, which [`free_models::attempt_order`] already filters to
            // routable entries — a promotion can never be reached from here.
            let order = free_models::attempt_order(turn.model.id);
            let mut last_error: Option<String> = None;
            for (index, model) in order.iter().enumerate() {
                let access = RemoteAccess {
                    base_url: free_models::GATEWAY_BASE.to_string(),
                    model: model.upstream.unwrap_or_default().to_string(),
                    key: turn.key.clone(),
                };
                let mut landed = false;
                let result = {
                    let deltas = &mut on_delta;
                    freecode_llama::stream_remote_chat(&access, request, |ds| {
                        if !ds.is_empty() {
                            landed = true;
                        }
                        deltas(ds)
                    })
                    .await
                };
                match result {
                    Ok(reply) => return Ok(reply),
                    Err(error) if !landed => {
                        if let Some(next) = order.get(index + 1) {
                            on_fallback(model.label, next.label);
                        }
                        last_error = Some(error.to_string());
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            Err(last_error
                .unwrap_or_else(|| "no routable free model is listed for this pick".to_string()))
        }
        Answer::OpenCode(turn) => {
            let Some(session) = turn.session.as_deref() else {
                return Err("this free turn has no free-model session to continue".into());
            };
            let prompt = opencode_prompt(request, turn)?;
            opencode::stream_turn(
                &turn.service,
                session,
                &prompt,
                stop,
                on_delta,
                gate,
                on_write,
            )
            .await
        }
    }
}

/// The text one OpenCode prompt carries.
///
/// The service keeps a conversation of its own, so an ordinary turn sends only what was just asked:
/// repeating the history would say everything twice. The exception is a session that had to be
/// opened under a conversation this app already holds — the system prompt is sent to every fresh
/// session, and the turns before this one go with it so the service answers as the same
/// conversation.
///
/// Pictures are refused rather than dropped: the service's prompt is text, and an answer produced
/// without the picture the user attached would be a guess presented as one.
fn opencode_prompt(request: &ChatRequest, turn: &OpenCodeTurn) -> Result<String, String> {
    let Some(last) = request.messages.last() else {
        return Ok(String::new());
    };
    if !last.images.is_empty() {
        return Err(
            "the free-model service takes text prompts only, so the picture on this prompt was not \
             sent. Remove the picture, or pick a model that answers through the gateway key."
                .into(),
        );
    }
    if !turn.fresh {
        return Ok(last.content.clone());
    }
    let mut prompt = String::new();
    if let Some(system) = request
        .messages
        .iter()
        .find(|message| message.role == freecode_llama::Role::System)
    {
        if !system.content.trim().is_empty() {
            prompt.push_str("Instructions for this conversation:\n");
            prompt.push_str(&system.content);
            prompt.push_str("\n\n");
        }
    }
    if turn.replay {
        prompt.push_str("The conversation so far, for context:\n\n");
        for message in &request.messages[..request.messages.len() - 1] {
            if message.role == freecode_llama::Role::System || message.content.trim().is_empty() {
                continue;
            }
            let who = match message.role {
                freecode_llama::Role::Assistant => "Assistant",
                freecode_llama::Role::Tool => "Tool",
                _ => "User",
            };
            prompt.push_str(who);
            prompt.push_str(":\n");
            prompt.push_str(&message.content);
            prompt.push_str("\n\n");
        }
        prompt.push_str("Now answer the last message:\n\n");
    }
    prompt.push_str(&last.content);
    Ok(prompt)
}

/// What this app does with a write the OpenCode service asks about: the answer it sends back, and the
/// row it opens for it.
///
/// The service runs its own calls, so these rows cannot come from a call list the way [`agent`]'s do —
/// they come from the asking. One request opens one row and that call's own report closes it; measured
/// against the live service, the id a request names as its source is the id `session.tool.success` and
/// `session.tool.failed` carry, which is what pairs the two halves without guessing. The map in between
/// is what stops a run that wrote four files from closing the first row four times.
struct ServiceWrites {
    /// The folder the run's writes are checked against, or the sentence saying why there is none to
    /// check them against: a project folder this app cannot read is one no write can be confined to.
    policy: Result<SandboxPolicy, String>,
    /// Whether a run of this shape may write at all — a chat is offered no tools and plan mode reads
    /// and proposes — and the words that tell the model which of the two it hit.
    may_write: bool,
    refusal: &'static str,
    open: BTreeMap<String, TranscriptStep>,
}

impl ServiceWrites {
    fn new(policy: Result<SandboxPolicy, String>, may_write: bool, refusal: &'static str) -> Self {
        Self {
            policy,
            may_write,
            refusal,
            open: BTreeMap::new(),
        }
    }

    fn policy(&self) -> Option<&SandboxPolicy> {
        self.policy.as_ref().ok()
    }

    /// A path the run changed, spelled the way the row and the review name it: inside the project as a
    /// relative path, which is what lets two writes naming one file land on one ledger line.
    fn shown(&self, raw: &str) -> String {
        match self.policy() {
            Some(policy) => policy.display_path(&self.target(raw)),
            None => raw.to_string(),
        }
    }

    /// Where the service's own spelling of a path lands on this machine.
    ///
    /// Measured: a file inside the session's folder is named relative to it (`page.html`), and one
    /// outside it arrives absolute. The absolute form is what the sandbox exists for — no form of the
    /// service's ruleset both keeps the write tool and confines it to a folder, so `ask` hands the
    /// decision here and this is the check that holds a free-model run inside the project.
    fn target(&self, raw: &str) -> PathBuf {
        let path = Path::new(raw);
        match (path.is_absolute(), self.policy()) {
            (true, _) => path.to_path_buf(),
            (false, Some(policy)) => policy.workspace_root.join(path),
            (false, None) => path.to_path_buf(),
        }
    }

    /// The paths one request would touch: the ones it was asked about, or, when it names none, the ones
    /// the patch it carries describes.
    fn targets(&self, request: &opencode::Request) -> Vec<PathBuf> {
        let names: Vec<&str> = if request.resources.is_empty() {
            request.changes.iter().map(|c| c.path.as_str()).collect()
        } else {
            request.resources.iter().map(String::as_str).collect()
        };
        names.iter().map(|raw| self.target(raw)).collect()
    }

    /// Answer one ask, in the words the service hands back to the model, and open its row.
    ///
    /// Two things can refuse: the run's own shape, read from the settings it started under rather than
    /// from the session, which outlives the dial moving; and the sandbox, which bounds every path the
    /// call names rather than the first one, because a write that touches two files lands outside on
    /// either of them.
    fn ask(&mut self, app: &AppHandle, request: &opencode::Request) -> opencode::Verdict {
        let verdict = if !self.may_write {
            opencode::Verdict::Refuse(self.refusal.to_string())
        } else {
            match &self.policy {
                Err(why) => opencode::Verdict::Refuse(why.clone()),
                Ok(policy) => self
                    .targets(request)
                    .iter()
                    .find_map(|path| policy.resolve_path(path, Access::Write).err())
                    .map(|violation| opencode::Verdict::Refuse(violation.to_string()))
                    .unwrap_or(opencode::Verdict::Allow),
            }
        };
        // Nothing can land without a folder to land it in, so there is no row to open and no write to
        // report: the answer goes back to the model, and the model says what it could not do.
        if self.policy.is_err() {
            return verdict;
        }
        let row = self.row_for(request);
        agent::emit(app, AgentEvent::ToolStarted { step: row.clone() });
        self.open.insert(request.id.clone(), row);
        verdict
    }

    /// The row one write gets: the same shape a local run's write row has, built by the same function,
    /// so Review, the inspector's changed files and the page's open control cannot tell the routes apart.
    fn row_for(&self, request: &opencode::Request) -> TranscriptStep {
        // Measured: the service's write tool answers for a new file and a changed one alike, and its
        // action says `edit` for both — so the row's kind, which is what makes a page an artifact the
        // window can open, comes from the status of the files it listed.
        let created =
            !request.changes.is_empty() && request.changes.iter().all(|change| change.is_created());
        let subject = request
            .changes
            .first()
            .map(|change| self.shown(&change.path))
            .unwrap_or_default();
        agent::started_row(
            if created { "write_file" } else { "edit_file" },
            &json!({ "path": subject }),
        )
    }

    /// Close the row a write ended, and move the pill when the bytes landed.
    ///
    /// Only an applied write carries a diff, which is what keeps Review honest on this route: a row for
    /// a change that did not happen would be counted by the panel's file list and offered by the open
    /// control, exactly like one that did.
    fn tell(&mut self, app: &AppHandle, write: &opencode::Write) {
        let Some(mut row) = self.open.remove(&write.request.id) else {
            return;
        };
        let mut staged = None;
        let outcome = match &write.fate {
            opencode::Fate::Refused(why) => {
                row.finish(StepStatus::Blocked);
                ToolOutcome::blocked(why)
            }
            opencode::Fate::Failed => {
                row.finish(StepStatus::Failed);
                ToolOutcome::blocked("the free-model service reported that this write did not land")
            }
            opencode::Fate::Applied => {
                row.finish(StepStatus::Done);
                let edits = self.edits(&write.request.changes);
                match agent::written(&edits) {
                    // The call said it ran and named no file, so there is no change to draw: the row
                    // says what the service called it, and the pill stays where it was.
                    None => ToolOutcome::ok(Summary::verbatim(format!(
                        "{} applied",
                        write.request.action
                    ))),
                    Some(done) => {
                        row.diff = Some(done.diff);
                        row.detail = done.detail;
                        row.hunk = done.hunk.clone();
                        for file in &done.files {
                            staged = Some(app.state::<ChatState>().stage(
                                &file.path,
                                file.added,
                                file.removed,
                            ));
                        }
                        // The row lists its files only when one hunk cannot describe the call, which is
                        // the rule the local loop works to.
                        if row.hunk.is_none() {
                            row.files = done.files;
                        }
                        ToolOutcome::ok(summed(&write.request.changes, &edits))
                    }
                }
            }
        };
        agent::emit(app, AgentEvent::ToolFinished { step: row, outcome });
        // The pill moves once the row is closed, as it does in the local loop: a count of a change the
        // transcript has not finished reporting is a number ahead of the rows under it.
        if let Some(diff) = staged {
            agent::emit(app, AgentEvent::DiffStaged { diff });
        }
    }

    /// The service's patch, as the app's write rows are built from one.
    ///
    /// The counts and the diff lines are the service's rather than recomputed here, so the review shows
    /// what the service wrote it changed — and so a row from this route goes through the same rule as a
    /// local one: one block in one file keeps the chip's `hunk`, anything wider is listed per file.
    fn edits(&self, changes: &[opencode::Change]) -> Vec<Edit> {
        changes
            .iter()
            .map(|change| Edit {
                path: self.shown(&change.path),
                added: change.added,
                removed: change.removed,
                occurrences: change.hunks.len() as u32,
                hunks: change
                    .hunks
                    .iter()
                    .map(|hunk| EditHunk {
                        at: hunk.at,
                        lines: hunk.lines.clone(),
                    })
                    .collect(),
            })
            .collect()
    }
}

/// The line under the row's label, in the same sentence the app's own three writes answer with: the
/// catalog already words a created file, an applied hunk, several hunks and a patch over several files,
/// and a free-model write is one of those four things.
fn summed(changes: &[opencode::Change], edits: &[Edit]) -> Summary {
    let hunks: usize = edits.iter().map(|edit| edit.hunks.len()).sum();
    if edits.len() == 1 && changes.iter().all(|change| change.is_created()) {
        return Summary::said(SummaryKey::FileCreated, json!({}), "1 file created");
    }
    if edits.len() > 1 {
        return Summary::said(
            SummaryKey::FilesPatched,
            json!({ "count": edits.len() }),
            format!("{} files patched", edits.len()),
        );
    }
    if hunks == 1 {
        return Summary::said(SummaryKey::HunkApplied, json!({}), "1 hunk applied");
    }
    Summary::said(
        SummaryKey::HunksApplied,
        json!({ "count": hunks }),
        format!("{hunks} hunks applied"),
    )
}

async fn stream_turn(
    app: &AppHandle,
    answer: &Answer,
    request: &ChatRequest,
    offered: bool,
    stops: &Stops<'_>,
) -> Result<Reply, String> {
    let sink = app.clone();
    let fallback_sink = sink.clone();
    let on_delta = move |deltas: &[ReplyDelta]| {
        for delta in deltas {
            let event = match delta {
                ReplyDelta::Thought(text) => AgentEvent::ThoughtDelta { text: text.clone() },
                ReplyDelta::Answer(text) => AgentEvent::AnswerDelta { text: text.clone() },
                ReplyDelta::ToolCall { name, .. } => {
                    if offered {
                        // The row for this call arrives with the round's batch, a moment from now.
                        continue;
                    }
                    let name = name.clone().unwrap_or_default();
                    AgentEvent::notice(NoticeCode::ToolCallWithoutTools, &[("name", name.as_str())])
                }
            };
            emit(&sink, event);
        }
        if !stops.requested.load(Ordering::Relaxed) {
            return StreamControl::Continue;
        }
        // Recorded here instead of reading the request flag afterwards: a click that lands after the
        // last chunk would otherwise label a complete answer as one that was stopped.
        stops.stopped.store(true, Ordering::Relaxed);
        StreamControl::Stop
    };
    let on_fallback = move |from: &'static str, to: &'static str| {
        emit(
            &fallback_sink,
            AgentEvent::notice(NoticeCode::FreeModelFallback, &[("from", from), ("to", to)]),
        );
    };
    // The service's route is the one route whose tool loop is not this app's, so its sandbox, its rows
    // and its ledger are built here. The other three never ask, and their answer is a sandbox this app
    // resolved anyway — so the folder is read only for a run the service may answer.
    let settings = appearance::load_settings()?;
    let plan = planning(&settings, offered);
    let writer: Arc<Mutex<Option<ServiceWrites>>> = Arc::new(Mutex::new(match answer {
        Answer::OpenCode(_) => Some(ServiceWrites::new(
            agent::policy_for(app, &settings),
            offered && !plan,
            if plan { PLAN_REFUSAL } else { CHAT_REFUSAL },
        )),
        _ => None,
    }));
    // A run whose writer is absent is one whose route never asks; the answer is only reached by a
    // service that asked while this app was not watching for it, and nothing that cannot be confined
    // to a folder gets let through.
    let gate_cell = Arc::clone(&writer);
    let gate = move |request: &opencode::Request| match &mut *lock(&gate_cell) {
        Some(writes) => writes.ask(app, request),
        None => opencode::Verdict::Refuse(UNCONFINED.to_string()),
    };
    let tell_cell = Arc::clone(&writer);
    let on_write = move |write: &opencode::Write| {
        if let Some(writes) = &mut *lock(&tell_cell) {
            writes.tell(app, write);
        }
    };
    stream_round(
        answer,
        request,
        stops.requested,
        on_delta,
        on_fallback,
        gate,
        on_write,
    )
    .await
}

/// What a service write is refused for when this app has no folder to confine it to: no row, no change,
/// and the model told in the same sentence every time.
const UNCONFINED: &str =
    "the project folder this run works in cannot be read, so no file may be changed";

/// The two refusals a run's own shape earns, before its paths are ever looked at. Both are said to the
/// model, which answers with what it would have done instead — which is the whole of plan mode, and the
/// whole of a chat sent with the control off.
const PLAN_REFUSAL: &str = "Plan mode is on: this run reads and proposes, and changes no file.";
const CHAT_REFUSAL: &str =
    "This run was sent as a chat, with no tools offered, so it changes no file.";

/// The third shape: a run that is not the user's, so its rows belong to nobody's transcript.
const SUBAGENT_REFUSAL: &str = "A subagent reads and reports; it is not allowed to change a file.";

/// Whether this run is the dial's Plan-only mode: it was offered tools and may read, but the answer
/// is the change to make rather than the change itself. Coding off is a plain chat either way — there
/// is no workspace to explore, so a planning contract would only make the answer odd.
fn planning(settings: &Settings, tools: bool) -> bool {
    tools && settings.approvals.mode == ApprovalMode::PlanOnly
}

/// The request for one round: the history as the host holds it now, with the run's own budget and the
/// tools the composer asked for.
///
/// Rebuilt per round rather than extended, because the only thing a round adds is in the history — and
/// the system prompt is read fresh, so a Settings change made mid-run is in the next request.
fn request_for(
    state: &ChatState,
    settings: &Settings,
    answer: &Answer,
    tools: bool,
) -> ChatRequest {
    let plan = planning(settings, tools);
    let system = if plan {
        plan_system(&settings.sampling.system_prompt)
    } else {
        settings.sampling.system_prompt.clone()
    };
    let mut request = ChatRequest::new(conversation(state, &system));
    request.max_tokens = answer.max_tokens();
    request.reasoning_effort = answer.effort().map(str::to_string);
    if tools {
        request.tools = tool_offer(settings);
    }
    request
}

/// The calls a tool-offering run may ask for: every executable one the switches allow, or only the
/// read-only seven while the dial says Plan only.
///
/// The read-only offer is the same list a subagent gets, and for the same reason: a tool offered and
/// then refused costs the model a round, and the approval engine would refuse every write in this
/// mode anyway. The planning contract can then name exactly what exists.
fn tool_offer(settings: &Settings) -> Vec<Value> {
    if settings.approvals.mode == ApprovalMode::PlanOnly {
        return freecode_tools::read_only_specs();
    }
    // Only the calls this build has runners for — the four reads, the three workspace writes, the
    // two web calls when the Privacy switch is on, the shell when Terminal access is on, and
    // delegation when Subagents is on: a schema the host cannot run, or one that would leave the
    // machine or start a second run against the user's stated wish, is a promise the model would
    // be asked to break.
    freecode_tools::executable_specs(
        settings.privacy.allow_remote_providers,
        settings.approvals.shell_enabled,
        settings.approvals.subagents_enabled,
    )
}

/// Whether the answer stopped to ask for a tool rather than finishing in prose.
fn wants_tools(reply: &Reply) -> bool {
    !reply.tool_calls.is_empty()
}

/// Keep at most `max` characters, saying that the rest was cut. Character-based rather than byte-based
/// so a Cyrillic answer is clipped at the length the constant names, not a third of it.
fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push_str("\n… (cut)");
    clipped
}

/// The one system turn a subagent is given: the user's own instructions, then the subagent's bounds.
///
/// The shipped coding contract is not repeated here: it tells the model to make the change, which is
/// the one thing a read-only subagent cannot do. A prompt the user wrote still leads, because that is
/// their instruction about how work should be done; the offer below then says what this run may do.
fn subagent_system(parent: &str) -> String {
    let parent = parent.trim();
    if parent.is_empty() || parent == DEFAULT_SYSTEM_PROMPT {
        SUBAGENT_SYSTEM.to_string()
    } else {
        format!("{parent}\n\n{SUBAGENT_SYSTEM}")
    }
}

/// The one system turn a plan-mode run is given: the same bargain as [`subagent_system`], with
/// [`PLAN_SYSTEM`] in place of the subagent's bounds.
///
/// The shipped contract is replaced rather than prefixed, because its first instruction is the act
/// this mode forbids. A prompt the user wrote still leads: it is their instruction about how work
/// should be done, and the plan contract only says what this run may do about it.
fn plan_system(parent: &str) -> String {
    let parent = parent.trim();
    if parent.is_empty() || parent == DEFAULT_SYSTEM_PROMPT {
        PLAN_SYSTEM.to_string()
    } else {
        format!("{parent}\n\n{PLAN_SYSTEM}")
    }
}

/// What one subagent run produced, for the row and the parent's tool result.
struct Delegated {
    text: String,
    rounds: usize,
    reads: usize,
}

/// Ask the engine a delegated question, let it read, and collect its answer.
///
/// This is the bounded second run the `spawn_subagent` switch pays for: its own history — the parent's
/// conversation is not replayed into it — its own read-only offer, and [`SUBAGENT_ROUNDS`] requests.
/// Its tool calls are run against the same sandbox and approval policy as the parent's, so a read the
/// user's dial would refuse is refused here too. Nothing is emitted to the transcript from this loop:
/// the one row the user sees is the delegation, and the report is what the parent gets back.
async fn delegate(
    answer: &Answer,
    settings: &Settings,
    policy: &SandboxPolicy,
    task: &str,
    stops: &Stops<'_>,
) -> Result<Delegated, String> {
    let mut history = vec![
        Message::system(subagent_system(&settings.sampling.system_prompt)),
        Message::user(task),
    ];
    let mut request = ChatRequest::new(history.clone());
    request.max_tokens = answer.max_tokens();
    request.reasoning_effort = answer.effort().map(str::to_string);
    request.tools = freecode_tools::read_only_specs();

    // Only a local server can count a prompt before it answers: `/apply-template` and `/tokenize`
    // are the fork's endpoints, not the OpenAI contract. A remote provider reports the oversize
    // refusal itself, which then names its own limit instead of a guessed one.
    if let Answer::Local(turn) = answer {
        if let Ok(sent) = prompt_tokens(
            &turn.view.base_url,
            &request,
            &turn.model,
            turn.view.context_tokens,
        )
        .await
        {
            if leaves_no_room(sent, answer.window_tokens()) {
                return Err(
                    "the delegated question does not leave room in the model's window for an answer"
                        .into(),
                );
            }
        }
    }

    let mut rounds = 0usize;
    let mut reads = 0usize;
    loop {
        if rounds == SUBAGENT_ROUNDS || stops.stopped.load(Ordering::Relaxed) {
            return Ok(Delegated {
                text: "The subagent was stopped before it answered.".into(),
                rounds,
                reads,
            });
        }
        rounds += 1;
        let reply = stream_round(
            answer,
            &request,
            stops.requested,
            |_| {
                if stops.requested.load(Ordering::Relaxed) {
                    stops.stopped.store(true, Ordering::Relaxed);
                    StreamControl::Stop
                } else {
                    StreamControl::Continue
                }
            },
            // A subagent's fallback moves are the parent run's news; the parent's own stream is
            // what announces them, and the subagent's window is not a transcript.
            |_, _| {},
            // A subagent is offered the read-only seven, so a write on this route is the service's own
            // tool reaching past the offer — and this loop emits no rows, so a change it made would be
            // one nothing reported. Refusing it is the answer that keeps the bargain the parent made.
            |_| opencode::Verdict::Refuse(SUBAGENT_REFUSAL.to_string()),
            |_| {},
        )
        .await
        .map_err(|e| format!("the delegated request failed: {e}"))?;

        if reply.tool_calls.is_empty() {
            let text = if reply.text.trim().is_empty() {
                if reply.ran_out_of_budget() {
                    "The subagent ran out of its answer budget without saying anything.".to_string()
                } else {
                    "The subagent finished without an answer.".to_string()
                }
            } else {
                reply.text.trim().to_string()
            };
            return Ok(Delegated {
                text,
                rounds,
                reads,
            });
        }
        if rounds == SUBAGENT_ROUNDS {
            // No request left to answer with. Whatever prose came beside the calls is the report, and
            // the row says the rounds were spent rather than pretending the subagent concluded.
            let text = if reply.text.trim().is_empty() {
                "The subagent spent all its rounds asking for tools and did not answer.".to_string()
            } else {
                reply.text.trim().to_string()
            };
            return Ok(Delegated {
                text,
                rounds,
                reads,
            });
        }

        let mut results = Vec::with_capacity(reply.tool_calls.len());
        for call in &reply.tool_calls {
            let arguments: Value =
                serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
            let name = call.name.as_str();
            let result = match freecode_tools::find(name) {
                Some(tool) if freecode_tools::is_executable(name) && tool.is_read_only() => {
                    match agent::cleared(name, &arguments, &call.id, settings, policy) {
                        Ok(()) => match freecode_tools::execute(name, &arguments, policy) {
                            Ok(done) => {
                                reads += 1;
                                clip_chars(&done.content, SUBAGENT_TOOL_CHARS)
                            }
                            Err(why) => format!("{name} did not run: {why}"),
                        },
                        Err(why) => format!("{name} did not run: {why}"),
                    }
                }
                _ => format!(
                    "{name} is not available to a subagent. It can use read_file, list_dir, glob, \
                     grep_search, git_status, git_diff and git_log."
                ),
            };
            results.push((call.id.clone(), result));
        }

        history.push(Message {
            tool_calls: reply.tool_calls.clone(),
            ..Message::assistant(reply.text.clone())
        });
        for (id, result) in results {
            history.push(Message::tool_result(&id, &result));
        }
        request = ChatRequest::new(history.clone());
        request.max_tokens = answer.max_tokens();
        request.reasoning_effort = answer.effort().map(str::to_string);
        request.tools = freecode_tools::read_only_specs();
        if let Answer::Local(turn) = answer {
            if let Ok(sent) = prompt_tokens(
                &turn.view.base_url,
                &request,
                &turn.model,
                turn.view.context_tokens,
            )
            .await
            {
                if leaves_no_room(sent, answer.window_tokens()) {
                    return Err(
                        "the subagent's reading did not leave room in the model's window for an answer"
                            .into(),
                    );
                }
            }
        }
    }
}

/// Close a delegation that never started, as a row that says why.
fn refuse_delegation(
    app: &AppHandle,
    call_id: &str,
    mut row: TranscriptStep,
    reason: String,
) -> Ran {
    row.finish(StepStatus::Blocked);
    row.detail = reason.clone();
    let outcome = ToolOutcome::blocked(&reason);
    agent::emit(app, AgentEvent::ToolFinished { step: row, outcome });
    Ran {
        call_id: call_id.to_string(),
        result: format!("spawn_subagent did not run: {reason}"),
    }
}

/// Run one `spawn_subagent` call: the switch, the dial, one bounded second run, and the answer folded
/// into the parent's history as one tool result.
async fn run_delegation(
    app: &AppHandle,
    answer: &Answer,
    settings: &Settings,
    policy: &SandboxPolicy,
    call: &freecode_llama::ToolCall,
    stops: &Stops<'_>,
    allowed: bool,
) -> Ran {
    let arguments: Value = serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
    let task = arguments
        .get("task")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let row = agent::started_row("spawn_subagent", &arguments);
    agent::emit(app, AgentEvent::ToolStarted { step: row.clone() });
    if task.is_empty() {
        return refuse_delegation(
            app,
            &call.id,
            row,
            "the task is empty; a subagent needs a self-contained question".into(),
        );
    }
    if !settings.approvals.subagents_enabled {
        return refuse_delegation(
            app,
            &call.id,
            row,
            "Subagents is off in Settings, so no second model run may start".into(),
        );
    }
    if !allowed {
        return refuse_delegation(
            app,
            &call.id,
            row,
            "this run has already used its one delegation".into(),
        );
    }
    if let Err(reason) = agent::cleared("spawn_subagent", &arguments, &call.id, settings, policy) {
        return refuse_delegation(app, &call.id, row, reason);
    }

    let mut row = row;
    match delegate(answer, settings, policy, &task, stops).await {
        Ok(done) => {
            row.finish(StepStatus::Done);
            let summary = Summary::said(
                SummaryKey::SubagentReport,
                json!({ "rounds": done.rounds, "reads": done.reads }),
                format!("{} rounds · {} reads", done.rounds, done.reads),
            );
            row.detail = summary.text.clone();
            agent::emit(
                app,
                AgentEvent::ToolFinished {
                    step: row,
                    outcome: ToolOutcome::ok(summary),
                },
            );
            Ran {
                call_id: call.id.clone(),
                result: format!(
                    "A read-only subagent answered after {} round(s) and {} read(s):\n{}\n\nIt could \
                     not change any file; make any edit yourself.",
                    done.rounds,
                    done.reads,
                    clip_chars(&done.text, SUBAGENT_RESULT_CHARS)
                ),
            }
        }
        Err(reason) => refuse_delegation(app, &call.id, row, reason),
    }
}

/// Ask, run what the answer calls for, and ask again — until the model answers in prose.
///
/// Each round replays the whole conversation with the previous round's calls and their results inside
/// it, so the window grows by a tool result per round: the prompt is measured again before each
/// follow-up, and one that no longer fits stops the loop rather than letting the server drop the
/// beginning of the task in silence. [`MAX_TOOL_ROUNDS`] is the second guard, for a model that keeps
/// calling and never answers.
///
/// The final `Reply` is the one that came in prose. A round that was refused a follow-up leaves the
/// last text the model wrote standing as the answer, which is what the user sees either way — and the
/// notice above it says why it stopped.
async fn run_rounds(
    app: &AppHandle,
    state: &ChatState,
    answer: &Answer,
    settings: &Settings,
    mut request: ChatRequest,
    tools: bool,
    stops: &Stops<'_>,
) -> Result<Reply, String> {
    let mut reply = stream_turn(app, answer, &request, tools, stops).await?;
    // The service's route needs nothing from this loop: it runs its calls itself, and the rows for
    // them are already emitted while the answer streams — and its `Reply` carries no call list, so a
    // second round here would only re-ask the same question. The sandbox below belongs to this app's
    // own calls, which is why it is resolved after this exit rather than before it.
    if !tools || matches!(answer, Answer::OpenCode(_)) {
        // The service denies its model the shell, and this app's runners are not its tools, so a
        // change it made would otherwise go out unmeasured. Verification here reads the same ledger
        // the route's writes were staged in, which is why it does not need the loop below.
        if tools && !stops.stopped.load(Ordering::Relaxed) {
            if let Ok(policy) = agent::policy_for(app, settings) {
                agent::verify_run(app, settings, &policy, false).await;
            }
        }
        return Ok(reply);
    }
    // Built here rather than by the caller: a chat with the control off does not need a workspace at
    // all, and a machine with no coding project open must keep answering plain questions. Asked once,
    // because the root does not move inside a run.
    let policy = agent::policy_for(app, settings)?;
    let mut rounds = 0usize;
    // One delegation per run. The switch is the consent to pay for a second model run; the budget is
    // what keeps a model that likes delegating from spending the whole window on its subagents.
    let mut delegations = 0usize;
    // Whether this run already measured itself. A model that ran its own tests has that result in
    // the transcript, and the host must not make the user wait through the same suite twice.
    let mut checked = false;
    while wants_tools(&reply) && !stops.stopped.load(Ordering::Relaxed) {
        if rounds == MAX_TOOL_ROUNDS {
            let cap = MAX_TOOL_ROUNDS.to_string();
            emit(
                app,
                AgentEvent::notice(NoticeCode::ToolLoopCapped, &[("rounds", cap.as_str())]),
            );
            break;
        }
        rounds += 1;
        let calls = reply.tool_calls.clone();
        // In the order the model asked, because the replay pairs each result with its `call_id` and
        // the template rejects a batch whose answers are shuffled. An ordinary call goes on the
        // blocking pool — a walk of the workspace is exactly the kind of thing the event loop must not
        // wait on — while a delegation is itself async and streams its own request.
        let mut ran = Vec::with_capacity(calls.len());
        for call in &calls {
            if freecode_tools::is_delegated(&call.name) {
                let allowed = delegations == 0;
                if allowed {
                    delegations += 1;
                }
                ran.push(
                    run_delegation(app, answer, settings, &policy, call, stops, allowed).await,
                );
                continue;
            }
            let (host, batch_settings, batch_policy, one) =
                (app.clone(), settings.clone(), policy.clone(), call.clone());
            let one = tauri::async_runtime::spawn_blocking(move || {
                agent::run_one(&host, &one, &batch_settings, &batch_policy)
            })
            .await
            .map_err(|e| format!("the tool round stopped: {e}"))?;
            ran.push(one);
        }
        agent::close_batch(app, &calls);
        // A batch that wrote starts the measurement over: a check from an earlier round said nothing
        // about a file this round changed after it.
        checked = if calls.iter().any(|call| {
            matches!(
                call.name.as_str(),
                "edit_file" | "write_file" | "apply_patch"
            )
        }) {
            ran_a_check(&calls)
        } else {
            checked || ran_a_check(&calls)
        };

        // The replay the fork's template needs: the assistant's own call list, then one result per
        // `call_id`. Both go in even for a call that was refused, because a call with no answer at all
        // is what makes the next prompt untemplateable.
        {
            let mut history = lock(&state.history);
            history.push(Message {
                tool_calls: calls,
                ..Message::assistant(reply.text.clone())
            });
            for one in &ran {
                history.push(Message::tool_result(&one.call_id, &one.result));
            }
        }

        // A Stop press during the batch — most likely inside a subagent's own stream — has nothing
        // left to ask for. The rows are closed; the run ends here.
        if stops.stopped.load(Ordering::Relaxed) {
            break;
        }
        request = request_for(state, settings, answer, tools);
        // Measured again for the same reason the first prompt is: a conversation that filled the window
        // during this run cannot be replayed, and the server would answer from its tail without saying
        // it dropped the task. Only the local server can be asked to count it; a remote provider keeps
        // its own limit and reports it on the request.
        if let Answer::Local(turn) = answer {
            if let Ok(sent) = prompt_tokens(
                &turn.view.base_url,
                &request,
                &turn.model,
                turn.view.context_tokens,
            )
            .await
            {
                if leaves_no_room(sent + picture_tokens(&request), answer.window_tokens()) {
                    let tokens = sent.to_string();
                    let window = answer.window_tokens().to_string();
                    emit(
                        app,
                        AgentEvent::notice(
                            NoticeCode::PromptTooLong,
                            &[("tokens", tokens.as_str()), ("window", window.as_str())],
                        ),
                    );
                    break;
                }
            }
        }
        reply = stream_turn(app, answer, &request, tools, stops).await?;
    }
    // The same measurement for this route: the model's own check is the better one when it ran, and
    // a Stop press means the work the checks would judge is the work the user just cut off.
    if !stops.stopped.load(Ordering::Relaxed) {
        agent::verify_run(app, settings, &policy, checked).await;
    }
    Ok(reply)
}

/// Whether a round ran one of the project's own checks. Only the three runners count: a `terminal`
/// call may be a read, and treating `ls` as a measurement would skip the check the edit still needs.
fn ran_a_check(calls: &[freecode_llama::ToolCall]) -> bool {
    calls
        .iter()
        .any(|call| matches!(call.name.as_str(), "run_tests" | "run_build" | "run_linter"))
}

/// Start the engine on request. Explicit, so it re-arms the restart budget — a press is the user
/// asking for another try at something the host cannot see changed.
#[tauri::command]
pub async fn start_engine(app: AppHandle) -> Result<EngineView, String> {
    {
        let state = app.state::<ChatState>();
        lock(&state.slot).re_arm();
    }
    Ok(ensure_running(&app, Needs::TextOnly).await?.view)
}

/// Release the engine and its port, so the weights stop occupying memory until something asks for
/// them again, and with it the restart budget: the same reasoning as [`start_engine`].
///
/// Refused while an answer is streaming. Interrupting a reply is [`chat_stop`], which keeps the
/// server; pulling the socket out from under a run in flight is a different and more expensive act,
/// and the control that offers it is disabled until the answer lands.
#[tauri::command]
pub async fn stop_engine(app: AppHandle) -> Result<(), String> {
    let running = take_running(&app.state::<ChatState>())?;
    running.handle.shutdown().await;
    Ok(())
}

/// The server the slot owns, if one may be released right now.
fn take_running(state: &ChatState) -> Result<Running, String> {
    let mut slot = lock(&state.slot);
    if slot.streaming {
        return Err("an answer is still streaming".into());
    }
    slot.re_arm();
    slot.running
        .take()
        .ok_or_else(|| "the local engine is not running".into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub running: bool,
    pub engine: Option<EngineView>,
}

#[tauri::command]
pub fn engine_status(state: tauri::State<'_, ChatState>) -> EngineStatus {
    // Asked of the child, not of the slot. A server that died since it answered `/health` is not
    // running, and a status card that goes on naming its port is how a dead engine gets believed
    // alive — the release and the restart stay with [`live`], which is where a turn needs them.
    let mut slot = lock(&state.slot);
    let mut view = None;
    if let Some(running) = slot.running.as_mut() {
        if running.handle.is_running() {
            view = Some(running.turn.view.clone());
        }
    }
    EngineStatus {
        running: view.is_some(),
        engine: view,
    }
}

/// How full the window has to be before the next turn is certain not to fit.
///
/// A run that fills the slot to within this margin leaves no room for the one message the next turn
/// adds on top of the replay, so the server has to drop something from the front to answer at all.
/// The margin is generous on purpose: it is the distance the fork itself kept from the ceiling when
/// it trimmed a 1090-token prompt into a 512-token slot (493 used), and a smaller one would let a
/// conversation that genuinely still fits be reported as a trimmed one.
const FULL_BY: u64 = 64;

/// Whether the conversation this run replayed was already too big for the window it was answered in.
///
/// The fork has no flag for a trim: it drops the front of the prompt, reports the size it kept, and
/// logs `truncated = 0` while it does it. What makes the drop provable anyway is that the host's
/// history only grows — every turn replays the whole of the last one and adds to it — so once a run
/// has measured the window as full, the turn after it cannot have fit.
fn outgrew_window(last: Option<(u32, u64)>, window: u32) -> bool {
    last.is_some_and(|(last_window, last_total)| {
        // A different window is a different server: the measurement that filled the old one says
        // nothing about the room in this one.
        last_window == window && last_total + FULL_BY >= window as u64
    })
}

/// Whether a prompt this size leaves the server anywhere to answer.
///
/// [`outgrew_window`] can only speak for a conversation that has already filled the window once, so
/// it says nothing about the first turn of a task whose prompt is oversized on its own. This is the
/// same margin applied to the prompt before it goes out, which is the case the after-the-fact notice
/// structurally cannot reach.
fn leaves_no_room(prompt_tokens: u64, window: u32) -> bool {
    prompt_tokens + FULL_BY > window as u64
}

/// How full the window is after a run, as a percent of what the server was started with.
///
/// The server's own count is the only one that means anything: the host never sees the conversation
/// after the chat template expands it. A stream that reported no usage answers `None`, which is what
/// keeps the meter from showing a number nobody measured.
fn context_percent(total_tokens: Option<u64>, window: u32) -> Option<f32> {
    let tokens = total_tokens?;
    if window == 0 {
        return None;
    }
    Some(((tokens as f64 * 100.0) / window as f64).clamp(0.0, 100.0) as f32)
}

/// What the run cost. The wait is the host's own and always stands; the five numbers the engine
/// measured are there only when its final frame arrived, which a stream the user stopped never does.
/// Absent ones stay absent rather than being divided out of a clock the host did not have.
///
/// With Coding on, these are the *last* round's counts — the rounds that ran tools are separate
/// streams, each with its own usage — while the wait is the whole run's. The rates stay the server's
/// own, measured over their own tokens, so the two halves never disagree by arithmetic.
fn run_report(reply: &Reply, elapsed: Duration) -> RunReport {
    let timings = reply.timings;
    RunReport {
        generated_tokens: timings.map(|t| t.generated_tokens),
        prompt_tokens: reply.prompt_tokens,
        cached_tokens: timings.map(|t| t.cached_tokens),
        generated_per_second: timings.map(|t| t.generated_per_second),
        prompt_per_second: timings.map(|t| t.prompt_per_second),
        // The host's wall clock rather than the server's sum of its two phases: this is the wait the
        // user sat through, round trip, queue and every delta over the IPC boundary included.
        elapsed_ms: elapsed.as_millis() as u64,
    }
}

/// The largest `data:` URL a picture crosses the boundary as.
///
/// The composer rescales before sending, so this is not a size the app aims at — it is the point past
/// which the string is not a snapshot of a picture. The cost of getting it wrong is not one request:
/// the URL is replayed into every later turn of the conversation, over the wire and into the filed
/// copy, for as long as that conversation stays open.
const IMAGE_MAX_CHARS: usize = 12 * 1024 * 1024;

/// Whether a string is the base64 picture the launcher could hand to a projector.
fn is_picture(url: &str) -> bool {
    url.starts_with("data:image/") && url.contains(";base64,") && url.len() <= IMAGE_MAX_CHARS
}

/// Send one prompt to the local engine and stream the answer as `agent-event`s. The return value is
/// the finished answer, so a listener that missed a delta can still close the turn.
///
/// `image` is the attached picture as a `data:` URL, and a prompt may be a picture with no text under
/// it: "what is this?" is the reason the control exists. The picture goes into the history the way it
/// went out, so every later turn of this conversation still has it in front of the model.
///
/// `tools` asks for the agent loop. The composer always sends it true — every prompt here is an agent
/// prompt, and what a run may do is set by the approval dial, Terminal access and Subagents. False,
/// this is the chat it has always been: one request, one streamed answer, and a tool call the model
/// emits anyway becomes a notice. On, the calls this build can run are offered and [`run_rounds`]
/// answers what the model asks for — unless the approval dial says Plan only, in which case the run
/// explores with the read-only calls and answers with a plan instead of changing the workspace.
#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    text: String,
    image: Option<String>,
    tools: bool,
) -> Result<String, String> {
    let text = text.trim().to_string();
    let image = image.filter(|url| !url.is_empty());
    if let Some(url) = &image {
        if !is_picture(url) {
            return Err("the attached file is not a picture this engine can be given".into());
        }
    }
    if text.is_empty() && image.is_none() {
        return Err("there is nothing to send".into());
    }
    // A picture already in the history needs the same projector as one arriving now: the replay below
    // puts it back on the wire whether or not this prompt carries it.
    let carries_picture = image.is_some() || {
        let state = app.state::<ChatState>();
        let held = lock(&state.history);
        held.iter().any(|m| !m.images.is_empty())
    };
    let answer = ensure_answer(&app, Needs::from_request(carries_picture)).await?;
    // The OpenCode route runs its own tool loop inside the service, which asks this app before any
    // write of it lands — so the run has rows, a diff and a Review panel exactly like a local one, and
    // the tools stay offered. What it does not get is this app's plan contract on the model's own
    // tools: the service carries its own, and the sandbox is applied at the ask rather than at a call.
    let state = app.state::<ChatState>();
    {
        let mut slot = lock(&state.slot);
        if slot.streaming {
            return Err("a reply is already streaming".into());
        }
        slot.streaming = true;
        // This run's own stop signal, not the last one's.
        state.stop_requested.store(false, Ordering::Relaxed);
    }

    let settings = appearance::load_settings()?;
    // Read from the settings this run will use, not from the page, so the announcement and the
    // request cannot disagree about what the dial said.
    let plan = planning(&settings, tools);
    let run_id = format!("run-{}", state.runs.fetch_add(1, Ordering::Relaxed) + 1);
    let stopped = AtomicBool::new(false);
    // This run's own tally, opened before its first request: the pill reads it, and a run that
    // changes nothing has to be able to say nothing.
    state.begin_run();
    let model_label = answer.label();
    emit(
        &app,
        AgentEvent::RunStarted {
            run_id: run_id.clone(),
            model_label: model_label.clone(),
            // The source was resolved above, so this is a fact rather than a possibility: a local
            // turn talks only to this machine — the supervised loopback server or the OpenCode
            // app's own service — and a remote one only to the address the user configured.
            local: answer.is_local(),
            // Only the supervised server has to be paged into memory, so only a run it answers may
            // tell the user the local engine is starting.
            engine: matches!(answer, Answer::Local(_)),
            // Zero says this run has no rounds to spend, which is what the step pill reads as "do not
            // draw me" — a plain chat does not have a budget, and a pill saying `Step 0 / 0` over it
            // would be a counter of something that cannot happen. The service's route is the other one
            // with nothing to count: its rounds are spent inside the service, which closes no batch
            // this app sees, so a budget here would be a number that never moves.
            max_rounds: if tools && !matches!(answer, Answer::OpenCode(_)) {
                MAX_TOOL_ROUNDS
            } else {
                0
            },
            // What the transcript needs to say the answer is a plan rather than a change the run
            // decided not to make — the mode is a setting, and the dial may move before this run ends.
            plan,
        },
    );
    // The choice the dial held was made for a route that takes no effort field — the free gateway,
    // a remote provider or the FreeCode service. Told here rather than by the bolt: the sentence
    // names the model the answer actually came from, and only the host resolved that — the free
    // pick's transport is chosen at run time.
    if let Some(chosen) = unheard_effort(settings.sampling.effort, answer.effort().is_some()) {
        let mut noted = lock(&state.effort_noted);
        let key = (model_label.clone(), chosen.to_string());
        if noted.as_ref() != Some(&key) {
            *noted = Some(key);
            emit(
                &app,
                AgentEvent::notice(
                    NoticeCode::EffortNotSent,
                    &[("model", model_label.as_str()), ("chosen", chosen)],
                ),
            );
        }
    }
    lock(&state.history).push(prompt(&text, image.clone()));
    // Filed before the request goes out rather than after the answer lands. A run that dies mid-stream
    // leaves the question on screen and nowhere else, and the one copy of what was asked that survives
    // the window is the copy in the conversation store.
    file_turn(
        &app,
        &state,
        &sessions::workspace_label(&app, &settings),
        &model_label,
        Role::User,
        &text,
        image.clone(),
    );
    // After the user turn is filed, because the conversation this run continues is the one that file
    // just opened or appended to: a fresh service session is remembered on it, and a conversation
    // with no service session yet is the one whose history the prompt has to replay.
    let answer = match open_service_session(&state, answer).await {
        Ok(answer) => answer,
        Err(message) => {
            // The prompt never went anywhere, so the history must not hold it as asked: the retry
            // the message invites would otherwise replay the same question twice.
            lock(&state.history).pop();
            lock(&state.slot).streaming = false;
            return Err(message);
        }
    };
    // Read before the request goes out and acted on after it lands: the replay this turn is about to
    // do is already too big for the window, but what the run did with that is only known once the
    // server reports its own count.
    let outgrew = outgrew_window(*lock(&state.last_fill), answer.window_tokens());
    let request = request_for(&state, &settings, &answer, tools);

    // Asked of the server, because the size that decides whether the prompt fits is the size the
    // server counts it at — and asked before the run, because the notice above can only reach a
    // conversation that has already filled the window once. A prompt that is oversized on its own
    // first turn has no previous run to be inferred from. A measurement that fails is silent: the
    // run still goes out, since a server that will not count a prompt may well still answer one.
    // Only the local fork has the counting endpoints; a remote provider answers with its own limit.
    let warned_before = match &answer {
        Answer::Local(turn) => {
            match prompt_tokens(
                &turn.view.base_url,
                &request,
                &turn.model,
                turn.view.context_tokens,
            )
            .await
            {
                // Counted with the pictures charged on top, because the server's tokenize pass never
                // sees them; see [`picture_tokens`].
                Ok(sent)
                    if leaves_no_room(sent + picture_tokens(&request), answer.window_tokens()) =>
                {
                    let tokens = sent.to_string();
                    let window = answer.window_tokens().to_string();
                    emit(
                        &app,
                        AgentEvent::notice(
                            NoticeCode::PromptTooLong,
                            &[("tokens", tokens.as_str()), ("window", window.as_str())],
                        ),
                    );
                    true
                }
                Ok(_) => false,
                // No logger in this crate, and nothing to do about it either way: the run goes out and
                // the server's own answer is the remaining evidence of how big the prompt was.
                Err(_) => false,
            }
        }
        Answer::Remote(_) | Answer::Free(_) | Answer::OpenCode(_) => false,
    };

    let started = Instant::now();
    let stops = Stops {
        requested: &state.stop_requested,
        stopped: &stopped,
    };
    let outcome = run_rounds(&app, &state, &answer, &settings, request, tools, &stops).await;
    lock(&state.slot).streaming = false;

    match outcome {
        Ok(reply) => {
            if !reply.text.is_empty() {
                lock(&state.history).push(Message::assistant(reply.text.clone()));
                file_turn(
                    &app,
                    &state,
                    &sessions::workspace_label(&app, &settings),
                    &model_label,
                    Role::Assistant,
                    &reply.text,
                    None,
                );
            }
            if let Some(total) = reply.total_tokens {
                *lock(&state.last_fill) = Some((answer.window_tokens(), total));
            }
            // The same news twice in one run's transcript reads as two problems. The count before the
            // run is the better-measured of the two sentences, so it is the one that stands.
            if outgrew && !warned_before {
                emit(&app, AgentEvent::notice(NoticeCode::ContextTrimmed, &[]));
            }
            if reply.ran_out_of_budget() {
                let max_tokens = answer.max_tokens().to_string();
                emit(
                    &app,
                    AgentEvent::notice(
                        NoticeCode::BudgetExhausted,
                        &[("maxTokens", max_tokens.as_str())],
                    ),
                );
            }
            emit(
                &app,
                AgentEvent::RunFinished {
                    run_id,
                    // The transcript's "stopped" row is what tells the user the answer they see is
                    // the one they cut short, so the flag has to come from the run itself.
                    stopped: stopped.load(Ordering::Relaxed),
                    context_used: context_percent(reply.total_tokens, answer.window_tokens()),
                    // What the run's accepted edits cost, or nothing at all for a run that only read
                    // the workspace. `None` is the honest answer there, not a zero.
                    total_diff: state.staged_diff(),
                    run: run_report(&reply, started.elapsed()),
                },
            );
            // A complete answer is the evidence that the configuration in use works, whatever killed
            // the server before it: the budget starts over.
            lock(&state.slot).re_arm();
            Ok(reply.text)
        }
        Err(message) => {
            emit(
                &app,
                AgentEvent::Failed {
                    message: message.clone(),
                },
            );
            Err(message)
        }
    }
}

/// Stop the reply in flight, and keep the server.
///
/// This is the composer's Stop. What it must not do is release the engine: interrupting one answer
/// is not a request to pay another model load for the next one, and the half-finished text the run
/// already streamed stays on screen either way.
#[tauri::command]
pub async fn chat_stop(app: AppHandle) -> Result<(), String> {
    request_stop(&app.state::<ChatState>());
    Ok(())
}

/// Raise this run's stop flag, if there is a run to stop.
///
/// Nothing in flight is a success, not a refusal: the control is only drawn while an answer is
/// arriving, so the one way to land there is a second click that lost the race to the last chunk —
/// and a double-click is not something to put an error line on screen for.
fn request_stop(state: &ChatState) -> bool {
    if !lock(&state.slot).streaming {
        return false;
    }
    // Set here and read by the run at its next batch of deltas; a click that lands after the last
    // chunk must not label a complete answer as one that was stopped.
    state.stop_requested.store(true, Ordering::Relaxed);
    true
}

/// Start a new task: forget the conversation, keep the server running.
///
/// The history is the whole of what a second turn remembers the first by, so dropping it is all the
/// control can mean. The loaded weights are not part of that, and someone starting a fresh question
/// has no reason to pay another load for it.
#[tauri::command]
pub async fn new_task(app: AppHandle) -> Result<(), String> {
    forget_conversation(&app.state::<ChatState>())
}

fn forget_conversation(state: &ChatState) -> Result<(), String> {
    {
        let slot = lock(&state.slot);
        // `chat_send` appends the prompt before the stream opens, so clearing mid-reply would drop a
        // prompt whose answer is still arriving and leave the next turn replaying half an exchange.
        if slot.streaming {
            return Err("a reply is still streaming; stop it before starting a new task".into());
        }
    }
    lock(&state.history).clear();
    // The before-images belong to the conversation that was just thrown away, and its files may
    // already have moved on. A revert is offered only for the run the window is showing.
    lock(&state.undo).clear();
    // The measurement was of the conversation this just threw away: the next request replays
    // nothing, so there is no overflow left to announce.
    *lock(&state.last_fill) = None;
    // The filed copy stays where it is — that is the point of having filed it — and the next prompt
    // opens a new one. This is what makes the sidebar's row for a conversation the user started and
    // abandoned a true statement rather than a promise to keep writing to it.
    *lock(&state.session) = None;
    Ok(())
}

/// Append one side of the exchange to the conversation store, opening the file if this is its first.
///
/// A conversation that will not land is announced on the transcript rather than swallowed. The
/// sidebar's row is a promise that the exchange is still there tomorrow, and a run that quietly
/// failed to keep it would break that promise at the one moment nobody is looking.
fn file_turn(
    app: &AppHandle,
    state: &ChatState,
    workspace: &str,
    model: &str,
    role: Role,
    text: &str,
    image: Option<String>,
) {
    let session = {
        let mut cell = lock(&state.session);
        let session = cell.get_or_insert_with(|| Session::new(workspace, model, text));
        session.model = model.to_string();
        session.push(role, text, image);
        session.clone()
    };
    if let Err(why) = sessions::write(&session) {
        emit(
            app,
            AgentEvent::notice(NoticeCode::NotFiled, &[("why", why.as_str())]),
        );
    }
}

/// Every conversation this app has filed, newest first. What the sidebar's Workspaces group lists.
#[tauri::command]
pub async fn list_sessions() -> Result<Vec<sessions::Summary>, String> {
    Ok(sessions::list())
}

/// Name a conversation what the user calls it rather than the first sentence it began with.
///
/// Both copies change, because the window's is the one the next turn writes back: [`file_turn`]
/// appends to the session it already holds rather than reading the file again, so a rename that only
/// reached the disk would be overwritten by the next answer with the name the row just threw away.
/// Not refused mid-reply the way opening one is — nothing on screen changes, so the streaming answer
/// still belongs to the conversation it is answering.
#[tauri::command]
pub async fn rename_session(
    app: AppHandle,
    id: String,
    title: String,
) -> Result<sessions::Summary, String> {
    let summary = sessions::rename(&id, &title)?;
    let state = app.state::<ChatState>();
    {
        let mut cell = lock(&state.session);
        if let Some(session) = cell.as_mut().filter(|session| session.id == id) {
            session.title = summary.title.clone();
        }
    }
    Ok(summary)
}

/// Hold a conversation at the head of the sidebar's list, or let it fall back into the order.
///
/// The same two copies for the same reason: the pin lives in the file, and the window holds the file.
#[tauri::command]
pub async fn set_session_pinned(
    app: AppHandle,
    id: String,
    pinned: bool,
) -> Result<sessions::Summary, String> {
    let summary = sessions::set_pinned(&id, pinned)?;
    let state = app.state::<ChatState>();
    {
        let mut cell = lock(&state.session);
        if let Some(session) = cell.as_mut().filter(|session| session.id == id) {
            session.pinned = pinned;
        }
    }
    Ok(summary)
}

/// A conversation's name as a filename: the title with everything a path could read as structure
/// taken out, and the id behind it so two conversations that share a short name still ask for
/// different files.
///
/// The title is the user's own first sentence, so it can be `../../etc/passwd` or contain a
/// separator, and this string is handed to the save dialog as the name it pre-fills. Only characters
/// that cannot mean anything to a path survive, which is also what keeps a pasted stack trace from
/// becoming one.
fn suggested_stem(session: &Session) -> String {
    let cleaned: String = session
        .title
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .chars()
        .take(48)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return session.id.clone();
    }
    format!("{cleaned}-{id}", id = session.id)
}

/// Write one filed conversation out as a markdown document, to a path the user picks.
///
/// Reads the file rather than the window's copy, because the file is the record: what has been filed
/// is what comes out, and a run still in flight has not filed its answer yet. Nothing is written when
/// the chooser is cancelled, and the answer is the path that was — `None` for a cancel, which is a
/// decision rather than a failure.
#[tauri::command]
pub async fn export_session(app: AppHandle, id: String) -> Result<Option<String>, String> {
    let session = sessions::read(&id)?;
    let body = session.markdown();
    let name = format!("{}.md", suggested_stem(&session));
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_file_name(name)
            .add_filter("Markdown", &["md"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| format!("the save chooser stopped: {e}"))?;
    let Some(path) = picked else { return Ok(None) };
    let path = path
        .into_path()
        .map_err(|_| "the chosen name is not a path".to_string())?;
    std::fs::write(&path, body).map_err(|e| format!("writing {} failed: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

/// The filed conversations that say what the query says. What the palette lists.
///
/// The search is the host's rather than the window's because the words it looks for are in the
/// turns, and the turns deliberately do not cross the boundary to fill a sidebar row.
#[tauri::command]
pub async fn search_sessions(query: String) -> Result<Vec<sessions::Hit>, String> {
    Ok(sessions::search(&query))
}

/// Reopen a filed conversation: its turns go back on screen *and* into the history the next request
/// is built from, so asking it another question continues that exchange instead of starting a new one
/// that happens to look like it.
#[tauri::command]
pub async fn open_session(app: AppHandle, id: String) -> Result<Session, String> {
    let state = app.state::<ChatState>();
    {
        let slot = lock(&state.slot);
        // Same reason New task refuses: the reply in flight belongs to the conversation on screen, and
        // replacing the history under it would have the next turn answer out of two conversations.
        if slot.streaming {
            return Err(
                "a reply is still streaming; stop it before opening another conversation".into(),
            );
        }
    }
    let session = sessions::read(&id)?;
    // Whichever conversation the window was holding, its measurement is now about a different one.
    *lock(&state.history) = history_of(&session);
    *lock(&state.last_fill) = None;
    // The reopened conversation's writes predate this window, and the states it would restore are
    // not the ones its files hold now. Revert belongs to the conversation that wrote them here.
    lock(&state.undo).clear();
    *lock(&state.session) = Some(session.clone());
    Ok(session)
}

/// Put files a run wrote back to what they held before it first touched them.
///
/// The before-images live in this conversation's journal, not in the filed transcript: an undo after
/// a restart would need snapshots on disk, and this build would rather refuse than restore a state
/// it cannot prove. Each path is checked before it is written — a file the user has edited since the
/// run wrote it is reported, not overwritten — and the answer is per path so one refusal does not
/// hide the files that did go back.
#[tauri::command]
pub async fn revert_changes(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<Vec<undo::Reverted>, String> {
    {
        let state = app.state::<ChatState>();
        if lock(&state.slot).streaming {
            return Err(
                "a reply is still streaming; wait for it to finish before putting files back"
                    .into(),
            );
        }
    }
    let settings = appearance::load_settings()?;
    let root = crate::paths::project_root(Some(&app), &settings)?;
    let state = app.state::<ChatState>();
    let mut journal = lock(&state.undo);
    Ok(journal.revert(&paths, &root))
}

/// File the transcript the window is holding over the conversation it belongs to.
///
/// The rows are the window's, not the host's: a run's events arrive here as they stream and the
/// transcript store is what folds them into steps, batches and hunks, so the only faithful copy of
/// what was on screen is the one the window hands back. Without it a reopened conversation shows the
/// exchange and nothing of the work — the edit rows, the pages read, the diff the review panel drew
/// from them — which reads as an answer that arrived from nowhere.
///
/// A whole-list write, and refused mid-reply: the answer the stream is still producing has not been
/// filed, and rows filed above it would land under the wrong question.
#[tauri::command]
pub async fn file_rows(app: AppHandle, rows: Vec<serde_json::Value>) -> Result<(), String> {
    let state = app.state::<ChatState>();
    {
        let slot = lock(&state.slot);
        if slot.streaming {
            return Err(
                "a reply is still streaming; its rows cannot be filed until it closes".into(),
            );
        }
    }
    let session = {
        let mut cell = lock(&state.session);
        let session = cell
            .as_mut()
            .ok_or("there is no conversation on screen to file these rows under")?;
        session.set_rows(rows);
        session.clone()
    };
    sessions::write(&session).map_err(|why| why.to_string())
}

/// Move one filed conversation to the Trash, and take it off the screen too when the row deleted is
/// the one already open there.
///
/// The file is the only copy of the exchange — the transcript on screen is a rendering of it, and the
/// engine's history holds it until the next prompt replaces it — so when the row deleted is the one on
/// screen the history goes with the file. A window that kept answering out of a conversation the
/// sidebar no longer lists would be continuing an exchange it just claimed never happened. The bytes
/// stay: [`sessions::delete_in`] moves them, and Settings → Storage is what can put them back.
fn delete_filed(dir: &Path, state: &ChatState, id: &str) -> Result<(), String> {
    let on_screen = lock(&state.session).as_ref().is_some_and(|s| s.id == id);
    if on_screen {
        {
            let slot = lock(&state.slot);
            // `chat_send` files the answer into this conversation when the stream closes, so deleting it
            // mid-reply would leave a row that came back the moment the reply landed. New task refuses
            // for the same reason; this row has less to show for getting it wrong.
            if slot.streaming {
                return Err(
                    "a reply is still streaming; stop it before deleting this conversation".into(),
                );
            }
        }
    }
    sessions::delete_in(dir, id)?;
    if on_screen {
        lock(&state.history).clear();
        *lock(&state.last_fill) = None;
        *lock(&state.session) = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_session(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<ChatState>();
    let dir = sessions::dir().ok_or("this platform gives the app no conversation directory")?;
    delete_filed(&dir, &state, &id)
}

/// The conversations the sidebar stopped listing and the machine still holds. What Settings →
/// Storage lists under the Trash, next to the figure for the bytes they take.
#[tauri::command]
pub async fn list_trashed() -> Result<Vec<sessions::Trashed>, String> {
    Ok(sessions::dir()
        .map(|dir| sessions::list_trashed_in(&dir))
        .unwrap_or_default())
}

/// Put one back on the list. The row appears in the sidebar because the file is where the sidebar
/// reads, and nothing here opens it — the press restores a conversation, it does not resume one.
#[tauri::command]
pub async fn restore_trashed(name: String) -> Result<sessions::Summary, String> {
    let dir = sessions::dir().ok_or("this platform gives the app no conversation directory")?;
    sessions::restore_in(&dir, &name)
}

/// The press that means it: take the file out of the machine. This is the only path in the app that
/// unlinks a conversation, and the sidebar's own delete does not reach it.
#[tauri::command]
pub async fn purge_trashed(name: String) -> Result<(), String> {
    let dir = sessions::dir().ok_or("this platform gives the app no conversation directory")?;
    sessions::purge_in(&dir, &name)
}

/// The history a filed conversation would be answered from, in the order it was said.
///
/// Reopening has to rebuild what the run built the first time. The transcript on screen and the
/// memory behind the next prompt are the same exchange or neither is the conversation the user
/// believes they continued, and a row that showed turns the engine no longer holds would let the
/// next answer come from a different one.
fn history_of(session: &Session) -> Vec<Message> {
    session
        .turns
        .iter()
        .map(|turn| match turn.role {
            // The same builder the live turn used, so reopening a conversation replays the picture it
            // was answered from rather than the words alone — and the engine is then asked for the
            // projector it needs by the same check any other turn goes through.
            Role::User => prompt(&turn.text, turn.image.clone()),
            Role::Assistant => Message::assistant(&turn.text),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecode_llama::Timings;

    #[test]
    fn a_stop_names_the_reply_in_flight_and_nothing_else() {
        let state = ChatState::default();
        // Idle: nothing is stopped, and the flag stays down, so a click that arrives after the
        // answer finished cannot be mistaken for one that cut the answer short.
        assert!(!request_stop(&state));
        assert!(!state.stop_requested.load(Ordering::Relaxed));

        lock(&state.slot).streaming = true;
        assert!(request_stop(&state));
        assert!(state.stop_requested.load(Ordering::Relaxed));
    }

    #[test]
    fn a_saved_effort_is_reported_when_the_route_sends_no_field() {
        // A route with no effort field cannot keep a choice, so the run says which word went
        // nowhere rather than letting the dial look answered.
        assert_eq!(
            unheard_effort(ReasoningEffort::Medium, false),
            Some("medium")
        );
        assert_eq!(unheard_effort(ReasoningEffort::None, false), Some("none"));
        // Auto is the model's own choice, which is exactly what such a route does: nothing is lost.
        assert_eq!(unheard_effort(ReasoningEffort::Auto, false), None);
        // The local engine takes the field, so every choice is answered — including auto.
        assert_eq!(unheard_effort(ReasoningEffort::Xhigh, true), None);
        assert_eq!(unheard_effort(ReasoningEffort::Auto, true), None);
    }

    #[test]
    fn a_round_that_ran_a_command_has_already_measured_itself() {
        let call = |name: &str| freecode_llama::ToolCall {
            id: name.into(),
            name: name.into(),
            arguments: "{}".into(),
        };
        // The three runners are the acts the host would otherwise repeat.
        for name in ["run_tests", "run_build", "run_linter"] {
            assert!(ran_a_check(&[call(name)]), "{name}");
        }
        // A terminal may have been a read, and reading and editing measure nothing themselves, so
        // none of them may stand in for the check the edit still needs.
        assert!(!ran_a_check(&[call("terminal")]));
        assert!(!ran_a_check(&[call("read_file"), call("edit_file")]));
        assert!(!ran_a_check(&[]));
    }

    #[test]
    fn a_subagent_is_not_told_to_make_the_change_it_cannot_make() {
        // The shipped contract opens with "make the change"; a read-only run must not inherit it.
        let shipped = subagent_system(DEFAULT_SYSTEM_PROMPT);
        assert_eq!(shipped, SUBAGENT_SYSTEM);
        assert!(!shipped.contains("make the change"));
        // A user's own words still lead, because that is how they asked for work to be done.
        let mine = subagent_system("Answer like a pirate.");
        assert!(mine.starts_with("Answer like a pirate."));
        assert!(mine.ends_with(SUBAGENT_SYSTEM));
    }

    #[test]
    fn a_plan_run_proposes_and_is_offered_only_the_reads() {
        let mut settings = Settings::default();
        settings.approvals.mode = ApprovalMode::PlanOnly;
        assert!(planning(&settings, true));
        // Coding off stays a plain chat: there is no workspace to explore, and a planning contract
        // would only make a typed question's answer odd.
        assert!(!planning(&settings, false));

        // The shipped contract tells the model to make the change, which is exactly the act this
        // mode forbids; the planning one replaces it rather than standing beside it.
        assert_eq!(plan_system(DEFAULT_SYSTEM_PROMPT), PLAN_SYSTEM);
        assert!(!PLAN_SYSTEM.contains("make the change"));
        // A prompt the user wrote still leads, and the plan contract follows it.
        let mine = plan_system("Answer in short bullets.");
        assert!(mine.starts_with("Answer in short bullets."));
        assert!(mine.ends_with(PLAN_SYSTEM));

        let offer = tool_offer(&settings);
        let names: Vec<&str> = offer
            .iter()
            .map(|spec| spec["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "read_file",
                "list_dir",
                "glob",
                "grep_search",
                "todo_write",
                "git_status",
                "git_diff",
                "git_log"
            ],
            "the plan-mode run gets the reads and the plan list, and no write"
        );
    }

    #[test]
    fn an_ordinary_coding_run_keeps_the_coding_contract_and_its_writes() {
        let settings = Settings::default();
        assert_eq!(settings.approvals.mode, ApprovalMode::AutoApprove);
        assert!(!planning(&settings, true));
        let offer = tool_offer(&settings);
        let names: Vec<&str> = offer
            .iter()
            .map(|spec| spec["function"]["name"].as_str().unwrap())
            .collect();
        for id in ["write_file", "edit_file", "apply_patch"] {
            assert!(names.contains(&id), "{id} is missing: {names:?}");
        }
    }

    #[test]
    fn a_run_that_writes_nothing_leaves_nothing_to_review() {
        let state = ChatState::default();
        assert_eq!(state.staged_diff(), None);
        state.begin_run();
        // A read-only run and a run that never happened both end with no pill, rather than a "+0 −0"
        // that looks like a change nobody can find.
        assert_eq!(state.staged_diff(), None);
    }

    #[test]
    fn one_file_edited_twice_is_one_file_and_every_line_it_cost() {
        let state = ChatState::default();
        state.begin_run();
        state.stage("src/lib.rs", 4, 1);
        let total = state.stage("src/lib.rs", 2, 0);
        assert_eq!(
            total,
            DiffStat {
                lines_added: 6,
                lines_removed: 1,
                files_changed: 1
            }
        );
        // A second file is the only thing that moves the count, which is what `merge` could not say.
        let total = state.stage("src/exec.rs", 7, 2);
        assert_eq!(total.files_changed, 2);
        assert_eq!((total.lines_added, total.lines_removed), (13, 3));
        // And the next run opens a fresh ledger, so a pill never quotes the run before it.
        state.begin_run();
        assert_eq!(state.staged_diff(), None);
    }

    #[test]
    fn a_new_task_forgets_the_conversation_and_keeps_the_server() {
        let state = ChatState::default();
        lock(&state.history).push(Message::user("first question"));
        lock(&state.history).push(Message::assistant("first answer"));
        *lock(&state.session) = Some(Session::new("freecode", "model", "first question"));
        forget_conversation(&state).unwrap();
        // The next request is built from this vector, so an empty one is a model that remembers
        // nothing — which is the entire behaviour the control claims.
        assert!(lock(&state.history).is_empty());
        // And the next prompt opens its own file rather than appending to the one just left, or the
        // two conversations would be written over each other.
        assert!(lock(&state.session).is_none());
    }

    #[test]
    fn a_conversation_named_by_a_path_cannot_ask_for_one() {
        // The title is the user's own first sentence, and it pre-fills the save dialog, so a title
        // that reads as a path must not become one. What survives is the words with the structure
        // taken out, and the id behind them so two conversations sharing a name still differ.
        let mut session = Session::new("freecode", "model", "../../../../etc/passwd");
        let id = session.id.clone();
        assert_eq!(suggested_stem(&session), format!("etcpasswd-{id}"));

        session.title = String::new();
        assert_eq!(suggested_stem(&session), id);

        session.title = "Fix the loop".into();
        assert_eq!(suggested_stem(&session), format!("Fix the loop-{id}"));

        // A title is a suggestion to a dialog, not a document, so it is capped rather than kept whole.
        session.title = "x".repeat(400);
        assert_eq!(suggested_stem(&session).chars().count(), 48 + 1 + id.len());
    }

    /// A conversation filed in a scratch directory, so the host's own copy of it can be deleted here
    /// without reaching for the one this machine keeps.
    fn filed(dir: &Path, prompt: &str) -> Session {
        let mut session = Session::new("freecode", "model", prompt);
        session.push(Role::User, prompt, None);
        session.push(Role::Assistant, "an answer", None);
        sessions::write_in(dir, &session).unwrap();
        session
    }

    #[test]
    fn deleting_the_conversation_on_screen_takes_its_file_and_the_history_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = ChatState::default();
        let open = filed(dir.path(), "first question");
        *lock(&state.session) = Some(open.clone());
        *lock(&state.history) = history_of(&open);
        assert!(!lock(&state.history).is_empty());

        delete_filed(dir.path(), &state, &open.id).unwrap();

        assert!(!dir.path().join(format!("{}.json", open.id)).exists());
        // Both copies go, not just the one the sidebar shows: a transcript that still held the exchange
        // would let the next prompt continue a conversation the row says never happened.
        assert!(lock(&state.history).is_empty());
        assert!(lock(&state.session).is_none());
    }

    #[test]
    fn deleting_a_row_that_is_not_the_one_on_screen_leaves_the_answer_in_flight_alone() {
        let dir = tempfile::tempdir().unwrap();
        let state = ChatState::default();
        let open = filed(dir.path(), "the one being answered");
        let other = filed(dir.path(), "an old conversation");
        *lock(&state.session) = Some(open.clone());
        *lock(&state.history) = history_of(&open);

        delete_filed(dir.path(), &state, &other.id).unwrap();

        assert!(!dir.path().join(format!("{}.json", other.id)).exists());
        assert!(dir.path().join(format!("{}.json", open.id)).exists());
        assert_eq!(lock(&state.history).len(), 2);
        assert_eq!(lock(&state.session).as_ref().unwrap().id, open.id);
    }

    #[test]
    fn a_reply_streaming_into_a_conversation_refuses_to_have_it_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let state = ChatState::default();
        let open = filed(dir.path(), "first question");
        *lock(&state.session) = Some(open.clone());
        lock(&state.slot).streaming = true;

        assert!(delete_filed(dir.path(), &state, &open.id).is_err());
        // Refused means untouched: the reply that is arriving files itself into this very conversation,
        // so a delete that had gone through would have been undone by the answer.
        assert!(dir.path().join(format!("{}.json", open.id)).exists());
        assert!(lock(&state.session).is_some());
    }

    #[test]
    fn a_reopened_conversation_goes_back_into_the_history_in_the_order_it_was_said() {
        let mut session = Session::new("freecode", "model", "first question");
        session.push(Role::User, "first question", None);
        session.push(Role::Assistant, "first answer", None);
        session.push(Role::User, "and then?", None);

        // The same vector the run built the first time, so the next prompt continues this exchange
        // rather than answering a transcript the engine never held.
        assert_eq!(
            history_of(&session),
            vec![
                Message::user("first question"),
                Message::assistant("first answer"),
                Message::user("and then?"),
            ]
        );
    }

    #[test]
    fn a_new_task_refuses_while_a_reply_is_streaming() {
        let state = ChatState::default();
        lock(&state.history).push(Message::user("asked"));
        lock(&state.slot).streaming = true;
        let err = forget_conversation(&state).unwrap_err();
        assert!(err.contains("streaming"), "{err}");
        assert_eq!(
            lock(&state.history).len(),
            1,
            "a refused clear left the history alone"
        );
    }

    /// The card's release is not the composer's Stop: it may not pull the socket out from under a
    /// reply that is still arriving.
    #[test]
    fn releasing_the_engine_waits_for_the_answer_in_flight() {
        let state = ChatState::default();
        let idle = take_running(&state).err().unwrap();
        assert!(idle.contains("not running"), "{idle}");

        lock(&state.slot).streaming = true;
        let busy = take_running(&state).err().unwrap();
        assert!(busy.contains("streaming"), "{busy}");
    }

    #[test]
    fn the_window_is_measured_in_the_tokens_the_server_counted() {
        // A quarter of a 32k window, and the same count read against the window it was asked for.
        assert_eq!(context_percent(Some(8_192), 32_768), Some(25.0));
        assert_eq!(context_percent(Some(31), 100), Some(31.0));
        // A reply that filled the window says so at the top of the scale, not past it.
        assert_eq!(context_percent(Some(40_000), 32_768), Some(100.0));
        // Nothing counted, and nothing to count against: the meter stays unknown.
        assert_eq!(context_percent(None, 32_768), None);
        assert_eq!(context_percent(Some(31), 0), None);
    }

    /// The reply a Stop press leaves behind: `stream_chat` breaks out of the loop before the fork's
    /// closing frame, so neither the usage nor the timings reaches the host. The run still cost a
    /// wait, and the inspector names it instead of saying nothing was timed at all.
    #[test]
    fn a_run_cut_short_still_reports_the_wait_it_cost() {
        let cut_short = Reply {
            text: "hal".into(),
            ..Default::default()
        };
        let report = run_report(&cut_short, Duration::from_millis(12_625));
        assert_eq!(report.elapsed_ms, 12_625);
        // None of these are a rate anybody measured, and none of them are a zero claiming to be one.
        assert_eq!(
            report,
            RunReport {
                generated_tokens: None,
                prompt_tokens: None,
                cached_tokens: None,
                generated_per_second: None,
                prompt_per_second: None,
                elapsed_ms: 12_625,
            }
        );
    }

    /// The same shape on a run the engine did time: the counts and rates are the server's, the wait
    /// is the host's, and the longer number is the one the user felt.
    #[test]
    fn a_timed_run_reports_the_servers_counts_and_ours_for_the_wait() {
        let timed = Reply {
            prompt_tokens: Some(3162),
            timings: Some(Timings {
                prompt_per_second: 285.17,
                generated_per_second: 34.8,
                generated_tokens: 38,
                cached_tokens: 2900,
            }),
            ..Default::default()
        };
        assert_eq!(
            run_report(&timed, Duration::from_millis(9_000)),
            RunReport {
                generated_tokens: Some(38),
                prompt_tokens: Some(3162),
                cached_tokens: Some(2900),
                generated_per_second: Some(34.8),
                prompt_per_second: Some(285.17),
                elapsed_ms: 9_000,
            }
        );
    }

    #[test]
    fn a_prompt_that_reaches_the_ceiling_is_measured_before_the_run() {
        // The margin is the distance the fork kept from the ceiling when it trimmed a 1090-token
        // prompt into a 512-token slot, so a prompt with room left is not reported as a full one.
        assert!(!leaves_no_room(1, 32_768));
        assert!(!leaves_no_room(1_026, 1_090));
        assert!(leaves_no_room(1_027, 1_090));
        assert!(leaves_no_room(1_090, 1_090));
    }

    #[test]
    fn leaving_the_gpu_switch_on_offloads_every_layer() {
        let settings = Settings::default();
        assert_eq!(tuning(&settings, None).layers_override, None);
        assert!(tuning(&settings, None).thinking);
        assert_eq!(tuning(&settings, None).context_tokens, Some(32_768));
    }

    #[test]
    fn turning_the_gpu_switch_off_selects_cpu_decode() {
        // The setting is the app's CPU fallback: `-ngl 0` on the same Metal binary, which is what the
        // demo's `BONSAI_NGL=0` does.
        let mut settings = Settings::default();
        settings.local_model.offload_all_layers = false;
        assert_eq!(tuning(&settings, None).layers_override, Some(0));
    }

    #[test]
    fn instruct_mode_switches_the_server_to_the_instruct_sampling_profile() {
        let mut settings = Settings::default();
        settings.sampling.non_thinking_mode = true;
        assert!(!tuning(&settings, None).thinking);
    }

    #[test]
    fn the_effort_goes_out_as_the_pickers_word() {
        // `low` is the template's problem to downgrade, not this host's.
        assert_eq!(effort_word(ReasoningEffort::Low), "low");
        assert_eq!(effort_word(ReasoningEffort::None), "none");
    }

    fn seeded(id: &str) -> ModelEntry {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        freecode_models::Catalog::load_default(&root)
            .expect("catalog loads")
            .model(id)
            .expect("seeded entry")
            .clone()
    }

    #[test]
    fn a_model_without_a_thinking_branch_replaces_the_saved_effort_and_says_so() {
        let m = seeded("ternary-bonsai-1-7b");
        let (word, swap) = resolve_effort(&m, ReasoningEffort::Xhigh);
        assert_eq!(word, "none");
        let swap = swap.expect("the substitution must be announced, not silent");
        // The four facts a notice about the substitution interpolates, not a sentence: what the user
        // reads is the catalog's, in whichever language the window is set to.
        assert_eq!(swap.chosen, "xhigh");
        assert_eq!(swap.sent, "none");
        assert_eq!(swap.model, m.name);
        assert!(
            swap.accepted.contains("\"none\"") && !swap.accepted.contains("xhigh"),
            "{}",
            swap.accepted
        );
        // `low` stays un-translated here because this model has no measured synonym for it;
        // `client::wire_effort` is the one that rejects it.
        assert_eq!(
            resolve_effort(&m, ReasoningEffort::None),
            ("none".into(), None)
        );

        // The 27B keeps every choice the template can take, including the documented `low` synonym.
        let big = seeded("ternary-bonsai-2-27b");
        assert_eq!(
            resolve_effort(&big, ReasoningEffort::Low),
            ("low".into(), None)
        );
        assert_eq!(
            resolve_effort(&big, ReasoningEffort::Xhigh),
            ("xhigh".into(), None)
        );
    }

    /// `auto` is what a fresh install ships, so it must not cost the user a notice on every launch,
    /// while the very same model still reports a substitution for an effort someone did pick.
    #[test]
    fn auto_takes_the_models_published_default_without_announcing_it() {
        assert_eq!(
            resolve_effort(&seeded("ternary-bonsai-1-7b"), ReasoningEffort::Auto),
            ("none".into(), None)
        );
        assert_eq!(
            resolve_effort(&seeded("ternary-bonsai-2-27b"), ReasoningEffort::Auto),
            ("xhigh".into(), None)
        );
        assert!(
            resolve_effort(&seeded("ternary-bonsai-1-7b"), ReasoningEffort::Xhigh)
                .1
                .is_some()
        );
    }

    #[test]
    fn a_poisoned_lock_still_answers_with_its_state() {
        let state = ChatState::default();
        {
            let slot = lock(&state.slot);
            assert!(slot.running.is_none());
            drop(slot);
        }
        assert!(!lock(&state.slot).launching);
    }

    /// The heartbeat `llama-server` prints every few tokens, which is all a tail holds when the server
    /// was decoding at the time it died.
    fn heartbeat(n: usize) -> String {
        (0..n)
            .map(|i| {
                format!("0.{i:03} slot print_timing: id 0 | task 0 | n_gen = {i} | tg = 30.4 t/s")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The notice has one job — say why the server stopped — and 120 lines of timing block bury both
    /// that sentence and the setting the message tells the user to change.
    #[test]
    fn a_notice_quotes_what_a_dead_server_could_have_said_not_its_heartbeat() {
        let log = format!(
            "0.001 srv  update_slots: all slots are idle\n0.050 load_model: loading model 'Bonsai.gguf'\n{}\n0.351 slot get_available: id 0 | task -1 | selected slot by LRU",
            heartbeat(40)
        );
        let quoted = excerpt(&log);
        assert!(quoted.contains("loading model"), "{quoted}");
        assert!(quoted.contains("all slots are idle"), "{quoted}");
        assert!(
            !quoted.contains("print_timing") && !quoted.contains("get_available"),
            "the slot chatter is not the clue: {quoted}"
        );
        assert!(
            quoted.contains("(41 more lines of server output are not quoted.)"),
            "cutting the tail is said out loud: {quoted}"
        );
    }

    #[test]
    fn a_short_log_is_quoted_whole_and_an_all_chatter_tail_is_quoted_anyway() {
        assert_eq!(excerpt(DEAD), DEAD, "nothing to trim");
        let only = heartbeat(3);
        assert_eq!(
            excerpt(&only),
            only,
            "a tail that is all heartbeat still says when it stopped"
        );
    }

    const DEAD: &str = "llama_init_from_gguf: unable to load the model";

    fn death(tuning: &LaunchTuning, weights: &ModelKey) -> Crash {
        Crash {
            log: DEAD.to_string(),
            tuning: tuning.clone(),
            weights: weights.clone(),
        }
    }

    /// The whole budget, driven on a `Slot` directly: a real `Running` needs a real server.
    /// `launching` is released first because the real `ensure_running` clears it as soon as a launch
    /// resolves, and a death is only ever noticed between two resolved launches.
    fn died_under(
        slot: &mut Slot,
        tuning: &LaunchTuning,
        weights: &ModelKey,
    ) -> Result<Launch, String> {
        slot.launching = false;
        slot.note_death(death(tuning, weights));
        slot.begin(tuning, weights)
    }

    #[test]
    fn a_server_that_dies_is_restarted_and_each_restart_is_counted() {
        let tuning = tuning(&Settings::default(), None);
        let weights = model_key(&Settings::default());
        let mut slot = Slot::default();
        assert_eq!(slot.begin(&tuning, &weights), Ok(Launch::First));

        for attempt in 1..=MAX_RESTARTS {
            assert_eq!(
                died_under(&mut slot, &tuning, &weights),
                Ok(Launch::Restart {
                    attempt,
                    log: DEAD.to_string()
                }),
                "death {attempt} of the budget is a restart, not a refusal"
            );
        }
    }

    #[test]
    fn a_budget_spent_on_the_same_configuration_is_a_refusal_that_quotes_the_server() {
        let tuning = tuning(&Settings::default(), None);
        let weights = model_key(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&tuning, &weights).unwrap();
        for _ in 1..=MAX_RESTARTS {
            died_under(&mut slot, &tuning, &weights).expect("inside the budget");
        }
        let message =
            died_under(&mut slot, &tuning, &weights).expect_err("past the budget it is refused");
        assert!(message.contains("not being started again"), "{message}");
        // The count the user sees is deaths, and the restarts that were spent on them.
        assert!(
            message.contains(&format!(
                "{} times in a row after {} automatic restarts",
                MAX_RESTARTS + 1,
                MAX_RESTARTS
            )),
            "{message}"
        );
        assert!(message.contains(DEAD), "the reason must be in the message");
        assert!(
            !slot.launching,
            "a refusal must not reserve the launch slot"
        );
    }

    #[test]
    fn changing_the_launch_configuration_is_a_new_attempt_not_a_spent_budget() {
        // The bound exists to stop the host retrying the same doomed launch, so the setting that
        // changes what is launched has to re-arm it: that is the documented fix for the common cause.
        let was_tried = tuning(&Settings::default(), None);
        let weights = model_key(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&was_tried, &weights).unwrap();
        for _ in 1..=MAX_RESTARTS {
            died_under(&mut slot, &was_tried, &weights).unwrap();
        }
        died_under(&mut slot, &was_tried, &weights).expect_err("budget spent");

        let mut lowered = was_tried.clone();
        lowered.context_tokens = Some(4_096);
        assert_eq!(
            slot.begin(&lowered, &weights),
            Ok(Launch::First),
            "a configuration that has never died has its own budget"
        );
    }

    #[test]
    fn another_model_under_the_same_launch_settings_is_a_new_attempt_too() {
        // Every field the launcher passes can be identical between two models — same context, same
        // offload, same sampling — and still be a different file on disk, which is the thing most
        // likely to have failed. So the weights are part of the configuration the budget counts.
        let tuning = tuning(&Settings::default(), None);
        let first = model_key(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&tuning, &first).unwrap();
        for _ in 1..=MAX_RESTARTS {
            died_under(&mut slot, &tuning, &first).unwrap();
        }
        died_under(&mut slot, &tuning, &first).expect_err("budget spent");

        let other = ModelKey {
            catalog_id: "ternary-bonsai-2-4b".into(),
            packing: PackingChoice::Auto,
        };
        assert_eq!(
            slot.begin(&tuning, &other),
            Ok(Launch::First),
            "a model that has never died has its own budget"
        );
    }

    #[test]
    fn an_answer_that_finished_pays_for_the_deaths_before_it() {
        let tuning = tuning(&Settings::default(), None);
        let weights = model_key(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&tuning, &weights).unwrap();
        died_under(&mut slot, &tuning, &weights).unwrap();
        slot.launching = false;
        slot.re_arm();
        assert_eq!(slot.begin(&tuning, &weights), Ok(Launch::First));
    }

    #[test]
    fn one_launch_can_be_in_flight_at_a_time() {
        let tuning = tuning(&Settings::default(), None);
        let weights = model_key(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&tuning, &weights)
            .expect("the first start is allowed");
        assert_eq!(
            slot.begin(&tuning, &weights),
            Err("the engine is already starting".into())
        );
    }

    /// A file the renderer cannot hand to a projector is refused before a server is started for it,
    /// because the alternative is a run that spends ten seconds loading weights to fail at the wire.
    #[test]
    fn only_an_encoded_picture_is_accepted() {
        assert!(is_picture("data:image/jpeg;base64,/9j/4AAQSkZJRg=="));
        // A `data:` URL of any other kind has no path to an answer: the engine reads images.
        assert!(!is_picture("data:application/pdf;base64,JVBERi0="));
        // And a raw path or a http URL is not bytes at all — the renderer shrinks and encodes first.
        assert!(!is_picture("/Users/me/photo.jpg"));
        assert!(!is_picture("https://example.com/photo.jpg"));
        assert!(!is_picture("data:image/png"));
        // Past the ceiling the request body is the problem, not the picture, and the sender's own
        // limit is what says so.
        assert!(!is_picture(&format!(
            "data:image/jpeg;base64,{}",
            "A".repeat(IMAGE_MAX_CHARS)
        )));
    }

    /// The pre-check asks the server to tokenize the templated prompt, where a picture is only the
    /// placeholder the template writes — so the vision tokens have to be charged here or a picture-heavy
    /// conversation is announced as fitting the window it is about to overflow.
    #[test]
    fn each_attached_picture_is_charged_the_engines_own_image_cap() {
        let mut request = ChatRequest::new(vec![
            prompt("what is in this?", Some("data:image/jpeg;base64,AA".into())),
            Message::assistant("a chart"),
            prompt("and in this one?", Some("data:image/jpeg;base64,BB".into())),
        ]);
        assert_eq!(picture_tokens(&request), 2 * IMAGE_TOKEN_CAP as u64);
        request.messages[0].images.clear();
        request.messages[2].images.clear();
        assert_eq!(picture_tokens(&request), 0, "a text-only run pays nothing");
    }

    /// A reopened conversation replays from the filed turns, so a picture that is not filed is a
    /// prompt about a photo the model can no longer see.
    #[test]
    fn a_filed_picture_comes_back_into_the_history_with_its_prompt() {
        let mut session = Session::new("freecode", "model", "what is in this?");
        session.push(
            Role::User,
            "what is in this?",
            Some("data:image/jpeg;base64,/9j/".into()),
        );
        session.push(Role::Assistant, "a chart", None);
        let history = history_of(&session);
        assert_eq!(history[0].images.len(), 1, "the picture replays");
        assert!(history[1].images.is_empty(), "the answer carries none");
        // And a picture anywhere in the history is enough to require the projector, whether or not the
        // prompt arriving now has one.
        assert!(matches!(Needs::from_request(true), Needs::Projector));
        assert!(matches!(Needs::from_request(false), Needs::TextOnly));
    }

    /// The default model publishes no projector, which is the reachability gap this host refuses on
    /// rather than launching a server that would answer a picture with a guess.
    #[test]
    fn the_projector_a_model_publishes_is_what_decides_whether_pictures_are_possible() {
        let dir = tempfile::tempdir().unwrap();
        let weights = dir.path();
        assert!(matches!(
            projector(weights, &seeded("ternary-bonsai-1-7b")),
            Projector::Unsupported
        ));

        let four = seeded("ternary-bonsai-2-4b");
        let file = four
            .recommended_vision()
            .expect("the 4B publishes a projector")
            .file
            .clone();
        assert!(matches!(
            projector(weights, &four),
            Projector::Absent { .. }
        ));
        // Present only at the published length: a torn file is the download's to fix, and starting a
        // server on one would be a crash the status card already knows how to clear.
        std::fs::write(weights.join(&file), "torn").unwrap();
        assert!(matches!(
            projector(weights, &four),
            Projector::Absent { .. }
        ));
        // The published length of this pack is 366 MB, and a probe that is size-only cannot tell that
        // from any other length — so the file is stood in for at a length of this test's choosing
        // rather than written out at the cost of a download.
        let mut at_length = four.clone();
        let small = 4_096u64;
        for proj in at_length.vision.iter_mut() {
            proj.size_bytes = small;
        }
        std::fs::write(weights.join(&file), vec![0u8; small as usize]).unwrap();
        match projector(weights, &at_length) {
            Projector::Present { file: named, path } => {
                assert_eq!(named, file);
                assert_eq!(path, weights.join(&file));
            }
            other => panic!("expected the file at its published size to be present: {other:?}"),
        }
        // And the same file no longer at that length reads as absent again: the probe measures, it
        // does not remember.
        std::fs::write(weights.join(&file), "torn").unwrap();
        assert!(matches!(
            projector(weights, &at_length),
            Projector::Absent { .. }
        ));
    }

    /// The subagent's answer is folded into the parent's history, which grows the window by one tool
    /// result; the cap is counted in characters so a Cyrillic or CJK answer is cut at the length the
    /// constant names rather than at a third of it. The marker is the model's cue that something is
    /// missing, so it stays even for a text that overruns by one character.
    #[test]
    fn a_subagent_answer_is_clipped_by_characters_not_bytes() {
        let short = "нашёл точку входа в main.rs";
        assert_eq!(clip_chars(short, 40), short);
        let long = "я".repeat(30);
        let clipped = clip_chars(&long, 10);
        assert!(clipped.starts_with(&"я".repeat(10)));
        assert!(clipped.ends_with("\n… (cut)"));
        assert_eq!(clipped.chars().count(), 10 + "\n… (cut)".chars().count());
        // A non-ASCII text just past the cap is cut and marked, not passed through whole.
        assert!(clip_chars(&"я".repeat(11), 10).ends_with("\n… (cut)"));
    }

    /// Remote is a choice, not a preference: every missing precondition is a refusal that names the
    /// setting, because the alternative — quietly asking the local engine — would send the prompt
    /// somewhere the user did not select and spend a model load they did not ask for.
    #[test]
    fn a_provider_turn_is_refused_before_it_could_fall_back() {
        let mut settings = Settings::default();
        settings.remote_provider.base_url = "https://api.example.com/v1".into();
        settings.remote_provider.model = "model-id".into();

        let refused = remote_plan(&settings).unwrap_err();
        assert!(refused.contains("Settings → Privacy"), "{refused}");

        settings.privacy.allow_remote_providers = true;
        settings.remote_provider.model.clear();
        let refused = remote_plan(&settings).unwrap_err();
        assert!(refused.contains("Settings → Model"), "{refused}");

        settings.remote_provider.model = "model-id".into();
        settings.remote_provider.base_url = "http://api.example.com/v1".into();
        let refused = remote_plan(&settings).unwrap_err();
        assert!(refused.contains("HTTPS"), "{refused}");

        settings.remote_provider.base_url = "http://127.0.0.1:8080/v1".into();
        let (base, model, window, _) = remote_plan(&settings).expect("loopback provider");
        assert_eq!(base, "http://127.0.0.1:8080/v1");
        assert_eq!(model, "model-id");
        assert_eq!(window, 32_768);

        settings.remote_provider.context_tokens = 0;
        let refused = remote_plan(&settings).unwrap_err();
        assert!(refused.contains("context window is 0"), "{refused}");
    }

    /// The provider declares its own window and the app cannot measure it the way it measures its
    /// server's, so the answer budget is capped by half of what the provider says it accepts — and by
    /// the configured budget, whichever is smaller.
    #[test]
    fn a_provider_reply_budget_is_capped_by_half_the_declared_window() {
        let mut settings = Settings::default();
        settings.privacy.allow_remote_providers = true;
        settings.remote_provider.base_url = "https://api.example.com/v1".into();
        settings.remote_provider.model = "model-id".into();

        settings.remote_provider.context_tokens = 10_000;
        assert_eq!(remote_plan(&settings).unwrap().3, 5_000);

        settings.remote_provider.context_tokens = 2_000_000;
        assert_eq!(remote_plan(&settings).unwrap().3, 16_384);

        settings.remote_provider.context_tokens = 3;
        assert_eq!(remote_plan(&settings).unwrap().3, 1);
    }

    /// The free pick is a choice like the remote one: a missing pick, an unknown id and a promotion
    /// all refuse with the reason, because a quiet return to the local engine would send the prompt
    /// somewhere the user did not select. The remote-provider consent switch is not one of those
    /// reasons: it governs the user's own address, and a picked free model must send without it.
    #[test]
    fn a_free_turn_refuses_before_it_could_fall_back() {
        let mut settings = Settings {
            free_model: Some("glm-5.3-flash-free".into()),
            ..Settings::default()
        };
        assert!(!settings.privacy.allow_remote_providers);

        let (model, _) = free_plan(&settings).expect("consent does not gate a free pick");
        assert_eq!(model.id, "glm-5.3-flash-free");

        settings.free_model = None;
        let refused = free_plan(&settings).unwrap_err();
        assert!(refused.contains("no free model is selected"), "{refused}");

        settings.free_model = Some("not-a-model".into());
        let refused = free_plan(&settings).unwrap_err();
        assert!(refused.contains("not-a-model"), "{refused}");

        // A promotion is listed but not routable: the refusal says so instead of inventing an
        // address for a model the gateway does not carry.
        settings.free_model = Some("big-pickle".into());
        let refused = free_plan(&settings).unwrap_err();
        assert!(refused.contains("not routable"), "{refused}");

        settings.free_model = Some("glm-5.3-flash-free".into());
        let (model, max_tokens) = free_plan(&settings).expect("routable pick");
        assert_eq!(model.id, "glm-5.3-flash-free");
        assert!(
            (1..=FREE_CONTEXT_TOKENS / 2).contains(&max_tokens),
            "{max_tokens}"
        );
    }

    /// The free tier's only fallback is inside the tier: a pre-token refusal moves down the
    /// registry's attempt order, which for a routable pick starts at the pick and never rewinds to
    /// an earlier one. The loop itself is exercised by the registry's order, not the wire.
    #[test]
    fn a_free_turn_falls_forward_only() {
        let order = free_models::attempt_order("glm-5.3-flash-free");
        let ids: Vec<&str> = order.iter().map(|m| m.id).collect();
        assert_eq!(ids.first(), Some(&"glm-5.3-flash-free"));
        // Every entry the loop can reach is routable — a promotion is never on the wire.
        assert!(order.iter().all(|m| m.upstream.is_some()));
        // An unknown id has nowhere to go, which the stream reports rather than guessing.
        assert!(free_models::attempt_order("not-a-model").is_empty());
    }

    /// The subagent is the same assistant with narrower hands, not a stranger: it gets the user's own
    /// instructions first, then the bounds that say what it cannot do. An empty parent prompt is the
    /// one case where the bounds are the whole system turn.
    #[test]
    fn a_subagent_inherits_the_users_instructions_before_its_own_bounds() {
        let inherited = subagent_system("You are terse.\n");
        assert!(inherited.starts_with("You are terse.\n\n"));
        assert!(inherited.ends_with(SUBAGENT_SYSTEM));
        assert!(!inherited.contains("You are terse.  \n"));
        assert_eq!(subagent_system("   \n"), SUBAGENT_SYSTEM);
        // The bounds are the promise the switch makes: read-only, no network, and an answer rather
        // than an edit. A change to this sentence is a change to what the user consented to.
        for bound in ["cannot change", "cannot reach the internet", "read_file"] {
            assert!(SUBAGENT_SYSTEM.contains(bound), "{bound}");
        }
    }
}
