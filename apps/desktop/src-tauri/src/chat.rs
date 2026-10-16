//! The live engine: one supervised fork server for this workspace, and the chat path that streams a
//! real answer from it into the transcript.
//!
//! This is the phase's proof that the whole local chain works on the machine it will ship for: the
//! same `status::Assembly` the status card renders hands the guard's verdict, the pinned binary and
//! the weight file to `smollm_llama::server::start`, and the reply comes back from that server over
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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use smollm_config::{ReasoningEffort, Settings};
use smollm_core::events::{AgentEvent, NoticeCode, RunReport};
use smollm_llama::{
    prompt_tokens, start as start_server, ChatRequest, LaunchRequest, LaunchTuning, Message, Reply,
    ReplyDelta, ServerHandle, StreamControl,
};
use smollm_models::ModelEntry;

use crate::{appearance, sessions, status};
use sessions::{Role, Session};

/// Event name the transcript subscribes to for the duration of the window.
pub const EVENT: &str = "agent-event";

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

/// A launched server plus the settings it was launched under.
struct Running {
    handle: ServerHandle,
    turn: Turn,
    /// What this server was started with, kept so a death can be compared against a *change*: the
    /// restart budget is a budget per configuration, not per app run. See [`MAX_RESTARTS`].
    tuning: LaunchTuning,
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
            effort_swap,
        }
    }
}

/// One supervised server that was found dead: what it said last, and the configuration it died under.
struct Crash {
    log: String,
    tuning: LaunchTuning,
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
    /// launcher actually uses (context size, GPU offload, thinking mode, flash attention) re-arms the
    /// budget. That is the documented fix for the common cause, so the bound never traps a user who is
    /// working the problem — it only stops the host from re-running a launch that just failed.
    fn begin(&mut self, tuning: &LaunchTuning) -> Result<Launch, String> {
        if self.launching {
            return Err("the engine is already starting".into());
        }
        if self.crash.as_ref().is_some_and(|c| &c.tuning != tuning) {
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
    /// Set by [`chat_stop`] and read by the run in flight at the next batch of deltas. A plain
    /// flag rather than a dropped future because the answer streamed so far has to survive: the
    /// request ends at a chunk boundary and returns what it had.
    stop_requested: Arc<AtomicBool>,
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
    smollm_llama::kill_spawned_servers();
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

/// The user's setting translated into the launcher's vocabulary.
///
/// `offload_all_layers` off is the CPU-decode path (`-ngl 0`), and the context field is documented
/// in the Settings surface as "tokens the server reserves", so it goes to `-c` as given rather than
/// being second-guessed by the RAM tier.
fn tuning(settings: &Settings) -> LaunchTuning {
    LaunchTuning {
        context_tokens: Some(settings.sampling.budget.clamped().context_tokens),
        layers_override: (!settings.local_model.offload_all_layers).then_some(0),
        flash_attention: settings.local_model.flash_attention,
        dedicated_server_slots: settings.local_model.dedicated_server_slots,
        thinking: !settings.sampling.non_thinking_mode,
        // A projector cannot be attached to a running slot, so the lazy path fetches one and
        // relaunches rather than passing it here on faith. See `vision_projector` in the launcher.
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

/// Start the supervised server, or report the guard's refusal to allow it.
///
/// The refusal is emitted as `EngineBlocked` rather than only returned: the transcript has to show
/// the blocker with its install action, which is the shape the acceptance criteria name.
async fn launch(app: &AppHandle) -> Result<Running, String> {
    let (root, settings) = crate::root_and_settings(app)?;
    let blocking_root = root.clone();
    let probe_settings = settings.clone();
    let assembly = tauri::async_runtime::spawn_blocking(move || {
        status::assemble(&blocking_root, &probe_settings)
    })
    .await
    .map_err(|e| format!("engine resolution failed: {e}"))??;

    if let Some(blocker) = assembly.report.first_blocker() {
        let info = smollm_core::blocked_info(blocker, &assembly.lock, &assembly.model.repo_id);
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

    let tuning = tuning(&settings);
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

/// The live server's snapshot, if the child is still alive.
///
/// A server that exited since it answered `/health` is released here, which reaps the child and frees
/// its port, and the death is parked in the slot with its log: whether another one gets started is
/// [`Slot::begin`]'s decision, since that is the place which knows both the budget and the
/// configuration the next launch would use.
fn live_turn(app: &AppHandle) -> Option<Turn> {
    let state = app.state::<ChatState>();
    let mut slot = lock(&state.slot);
    let running = slot.running.as_mut()?;
    if running.handle.is_running() {
        return Some(running.turn.clone());
    }
    let crash = Crash {
        log: running.handle.log_tail(),
        tuning: running.tuning.clone(),
    };
    slot.running = None;
    slot.note_death(crash);
    None
}

async fn ensure_running(app: &AppHandle) -> Result<Turn, String> {
    if let Some(turn) = live_turn(app) {
        return Ok(turn);
    }
    let settings = appearance::load_settings()?;
    let kind = {
        let state = app.state::<ChatState>();
        let mut slot = lock(&state.slot);
        slot.begin(&tuning(&settings))?
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
    let outcome = launch(app).await;
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

async fn stream_turn(
    app: &AppHandle,
    turn: &Turn,
    request: &ChatRequest,
    stop_requested: &AtomicBool,
    stopped: &AtomicBool,
) -> Result<Reply, String> {
    let sink = app.clone();
    let on_delta = move |deltas: &[ReplyDelta]| {
        for delta in deltas {
            let event = match delta {
                ReplyDelta::Thought(text) => AgentEvent::ThoughtDelta { text: text.clone() },
                ReplyDelta::Answer(text) => AgentEvent::AnswerDelta { text: text.clone() },
                // Nothing asked for a tool call: the chat path sends no `tools`, so this is the model
                // emitting one anyway. Said out loud rather than dropped, because the answer text is
                // truncated at that point and silence would read as a stalled server.
                ReplyDelta::ToolCall { name, .. } => {
                    let name = name.clone().unwrap_or_default();
                    AgentEvent::notice(NoticeCode::ToolCallWithoutTools, &[("name", name.as_str())])
                }
            };
            emit(&sink, event);
        }
        if !stop_requested.load(Ordering::Relaxed) {
            return StreamControl::Continue;
        }
        // Recorded here instead of reading the request flag afterwards: a click that lands after the
        // last chunk would otherwise label a complete answer as one that was stopped.
        stopped.store(true, Ordering::Relaxed);
        StreamControl::Stop
    };
    smollm_llama::stream_chat(
        &turn.view.base_url,
        request,
        &turn.model,
        turn.view.context_tokens,
        on_delta,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Start the engine on request. Explicit, so it re-arms the restart budget — a press is the user
/// asking for another try at something the host cannot see changed.
#[tauri::command]
pub async fn start_engine(app: AppHandle) -> Result<EngineView, String> {
    {
        let state = app.state::<ChatState>();
        lock(&state.slot).re_arm();
    }
    Ok(ensure_running(&app).await?.view)
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
    // alive — the release and the restart stay with [`live_turn`], which is where a turn needs them.
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

/// What the run cost, from the two frames the stream answers with beside the usage one. `None`
/// unless both landed: token counts without the server's own clock give rates nobody measured, and
/// the inspector then has nothing to report rather than something invented.
fn run_report(reply: &Reply, elapsed: Duration) -> Option<RunReport> {
    let timings = reply.timings?;
    Some(RunReport {
        generated_tokens: timings.generated_tokens,
        prompt_tokens: reply.prompt_tokens?,
        cached_tokens: timings.cached_tokens,
        generated_per_second: timings.generated_per_second,
        prompt_per_second: timings.prompt_per_second,
        // The host's wall clock rather than the server's sum of its two phases: this is the wait the
        // user sat through, round trip, queue and every delta over the IPC boundary included.
        elapsed_ms: elapsed.as_millis() as u64,
    })
}

/// Send one prompt to the local engine and stream the answer as `agent-event`s. The return value is
/// the finished answer, so a listener that missed a delta can still close the turn.
#[tauri::command]
pub async fn chat_send(app: AppHandle, text: String) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("there is nothing to send".into());
    }
    let turn = ensure_running(&app).await?;
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
    let run_id = format!("run-{}", state.runs.fetch_add(1, Ordering::Relaxed) + 1);
    let stopped = AtomicBool::new(false);
    let model_label = format!("{} ({})", turn.model.name, turn.view.packing);
    emit(
        &app,
        AgentEvent::RunStarted {
            run_id: run_id.clone(),
            model_label: model_label.clone(),
            // The answer is coming from the supervised loopback server, so the only thing that can
            // make this run not-local is a configured remote provider.
            local: status::local_badge(true, settings.privacy.allow_remote_providers),
        },
    );
    lock(&state.history).push(Message::user(&text));
    // Filed before the request goes out rather than after the answer lands. A run that dies mid-stream
    // leaves the question on screen and nowhere else, and the one copy of what was asked that survives
    // the window is the copy in the conversation store.
    file_turn(
        &app,
        &state,
        &sessions::workspace_label(&app),
        &model_label,
        Role::User,
        &text,
    );
    // Read before the request goes out and acted on after it lands: the replay this turn is about to
    // do is already too big for the window, but what the run did with that is only known once the
    // server reports its own count.
    let outgrew = outgrew_window(*lock(&state.last_fill), turn.view.context_tokens);
    let mut request = ChatRequest::new(conversation(&state, &settings.sampling.system_prompt));
    request.max_tokens = turn.max_tokens;
    request.reasoning_effort = Some(turn.effort.clone());

    // Asked of the server, because the size that decides whether the prompt fits is the size the
    // server counts it at — and asked before the run, because the notice above can only reach a
    // conversation that has already filled the window once. A prompt that is oversized on its own
    // first turn has no previous run to be inferred from. A measurement that fails is silent: the
    // run still goes out, since a server that will not count a prompt may well still answer one.
    let warned_before = match prompt_tokens(
        &turn.view.base_url,
        &request,
        &turn.model,
        turn.view.context_tokens,
    )
    .await
    {
        Ok(sent) if leaves_no_room(sent, turn.view.context_tokens) => {
            let tokens = sent.to_string();
            let window = turn.view.context_tokens.to_string();
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
        // No logger in this crate, and nothing to do about it either way: the run goes out and the
        // server's own answer is the remaining evidence of how big the prompt was.
        Err(_) => false,
    };

    let started = Instant::now();
    let outcome = stream_turn(&app, &turn, &request, &state.stop_requested, &stopped).await;
    lock(&state.slot).streaming = false;

    match outcome {
        Ok(reply) => {
            if !reply.text.is_empty() {
                lock(&state.history).push(Message::assistant(reply.text.clone()));
                file_turn(
                    &app,
                    &state,
                    &sessions::workspace_label(&app),
                    &model_label,
                    Role::Assistant,
                    &reply.text,
                );
            }
            if let Some(total) = reply.total_tokens {
                *lock(&state.last_fill) = Some((turn.view.context_tokens, total));
            }
            // The same news twice in one run's transcript reads as two problems. The count before the
            // run is the better-measured of the two sentences, so it is the one that stands.
            if outgrew && !warned_before {
                emit(&app, AgentEvent::notice(NoticeCode::ContextTrimmed, &[]));
            }
            if reply.ran_out_of_budget() {
                let max_tokens = turn.max_tokens.to_string();
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
                    context_used: context_percent(reply.total_tokens, turn.view.context_tokens),
                    total_diff: None,
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
) {
    let session = {
        let mut cell = lock(&state.session);
        let session = cell.get_or_insert_with(|| Session::new(workspace, model, text));
        session.model = model.to_string();
        session.push(role, text);
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
    *lock(&state.session) = Some(session.clone());
    Ok(session)
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
            Role::User => Message::user(&turn.text),
            Role::Assistant => Message::assistant(&turn.text),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_new_task_forgets_the_conversation_and_keeps_the_server() {
        let state = ChatState::default();
        lock(&state.history).push(Message::user("first question"));
        lock(&state.history).push(Message::assistant("first answer"));
        *lock(&state.session) = Some(Session::new("smollm-studio", "model", "first question"));
        forget_conversation(&state).unwrap();
        // The next request is built from this vector, so an empty one is a model that remembers
        // nothing — which is the entire behaviour the control claims.
        assert!(lock(&state.history).is_empty());
        // And the next prompt opens its own file rather than appending to the one just left, or the
        // two conversations would be written over each other.
        assert!(lock(&state.session).is_none());
    }

    #[test]
    fn a_reopened_conversation_goes_back_into_the_history_in_the_order_it_was_said() {
        let mut session = Session::new("smollm-studio", "model", "first question");
        session.push(Role::User, "first question");
        session.push(Role::Assistant, "first answer");
        session.push(Role::User, "and then?");

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
        assert_eq!(tuning(&settings).layers_override, None);
        assert!(tuning(&settings).thinking);
        assert_eq!(tuning(&settings).context_tokens, Some(32_768));
    }

    #[test]
    fn turning_the_gpu_switch_off_selects_cpu_decode() {
        // The setting is the app's CPU fallback: `-ngl 0` on the same Metal binary, which is what the
        // demo's `BONSAI_NGL=0` does.
        let mut settings = Settings::default();
        settings.local_model.offload_all_layers = false;
        assert_eq!(tuning(&settings).layers_override, Some(0));
    }

    #[test]
    fn instruct_mode_switches_the_server_to_the_instruct_sampling_profile() {
        let mut settings = Settings::default();
        settings.sampling.non_thinking_mode = true;
        assert!(!tuning(&settings).thinking);
    }

    #[test]
    fn the_effort_goes_out_as_the_pickers_word() {
        // `low` is the template's problem to downgrade, not this host's.
        assert_eq!(effort_word(ReasoningEffort::Low), "low");
        assert_eq!(effort_word(ReasoningEffort::None), "none");
    }

    fn seeded(id: &str) -> ModelEntry {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        smollm_models::Catalog::load_default(&root)
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

    fn death(tuning: &LaunchTuning) -> Crash {
        Crash {
            log: DEAD.to_string(),
            tuning: tuning.clone(),
        }
    }

    /// The whole budget, driven on a `Slot` directly: a real `Running` needs a real server.
    /// `launching` is released first because the real `ensure_running` clears it as soon as a launch
    /// resolves, and a death is only ever noticed between two resolved launches.
    fn died_under(slot: &mut Slot, tuning: &LaunchTuning) -> Result<Launch, String> {
        slot.launching = false;
        slot.note_death(death(tuning));
        slot.begin(tuning)
    }

    #[test]
    fn a_server_that_dies_is_restarted_and_each_restart_is_counted() {
        let tuning = tuning(&Settings::default());
        let mut slot = Slot::default();
        assert_eq!(slot.begin(&tuning), Ok(Launch::First));

        for attempt in 1..=MAX_RESTARTS {
            assert_eq!(
                died_under(&mut slot, &tuning),
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
        let tuning = tuning(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&tuning).unwrap();
        for _ in 1..=MAX_RESTARTS {
            died_under(&mut slot, &tuning).expect("inside the budget");
        }
        let message = died_under(&mut slot, &tuning).expect_err("past the budget it is refused");
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
        let was_tried = tuning(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&was_tried).unwrap();
        for _ in 1..=MAX_RESTARTS {
            died_under(&mut slot, &was_tried).unwrap();
        }
        died_under(&mut slot, &was_tried).expect_err("budget spent");

        let mut lowered = was_tried.clone();
        lowered.context_tokens = Some(4_096);
        assert_eq!(
            slot.begin(&lowered),
            Ok(Launch::First),
            "a configuration that has never died has its own budget"
        );
    }

    #[test]
    fn an_answer_that_finished_pays_for_the_deaths_before_it() {
        let tuning = tuning(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&tuning).unwrap();
        died_under(&mut slot, &tuning).unwrap();
        slot.launching = false;
        slot.re_arm();
        assert_eq!(slot.begin(&tuning), Ok(Launch::First));
    }

    #[test]
    fn one_launch_can_be_in_flight_at_a_time() {
        let tuning = tuning(&Settings::default());
        let mut slot = Slot::default();
        slot.begin(&tuning).expect("the first start is allowed");
        assert_eq!(
            slot.begin(&tuning),
            Err("the engine is already starting".into())
        );
    }
}
