//! Starting, proving and stopping a fork `llama-server` — the only path from a [`LaunchPlan`] to a
//! running process.
//!
//! Three proofs happen before a request is ever sent, in this order:
//!   1. the plan itself (`LaunchPlan::assemble`) refuses a binary outside `engines/` and weights that
//!      are not on disk;
//!   2. `llama-server --version` must report the commit `engines/lock.toml` pins
//!      ([`crate::engine::banner_matches_pin`]) — a from-source build of the wrong commit would
//!      otherwise reintroduce the AVX-512 load crash (#245) or the Vulkan silent-CPU-fallback (#238);
//!   3. `/health` must answer 200 before the caller is handed a URL, and if the child dies while
//!      waiting, its own log is attached to the error. That log is the difference between
//!      "server exited" and "server exited: llama_init_from_gguf: unable to load ... PTQ1_0".
//!
//! The handle owns the child and kills it on drop, so a cancelled task, a panic, or a closed window
//! cannot leave a 6 GB server holding the weights. What a handle cannot cover is a signal that stops
//! the host process where it stands: during a load the child runs for seconds before any handle
//! exists, because the handle is only built once `/health` is green. [`kill_spawned_servers`] is the
//! registry for that window, so the host's shutdown hook has something to reach for on its way out.
//! And what *that* cannot cover is a `SIGKILL`, which runs no hook at all — for it the registry is
//! mirrored on disk as each server appears, and the next run calls [`reap_orphaned_servers`] to
//! finish what the last one started.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

use crate::engine::{banner_matches_pin, parse_version, EngineVersion};
use crate::launch::{LaunchPlan, LaunchRequest};
use crate::LlamaError;

use smollm_models::lock::EngineLock;

/// How many trailing lines of server output an error may quote. Enough for a load failure's stack
/// trace, small enough to send to a UI without a cap problem.
const LOG_TAIL_LINES: usize = 120;
/// The demo scripts poll `/health` themselves before pointing a browser at the server.
const HEALTH_POLL: Duration = Duration::from_millis(250);
/// A `--version` probe that has not answered in this long is a broken binary, not a slow one.
const VERSION_PROBE: Duration = Duration::from_secs(10);

/// The pids of the servers this process has spawned and not yet killed.
static SPAWNED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn track(pid: u32) {
    if let Ok(mut pids) = SPAWNED.lock() {
        pids.push(pid);
    }
}

fn untrack(pid: u32) {
    if let Ok(mut pids) = SPAWNED.lock() {
        pids.retain(|p| *p != pid);
    }
}

/// SIGKILL every server this process spawned and has not released, and empty the registry.
///
/// The point is the process-exit path, so this deliberately does nothing that can fail or block: one
/// `kill(2)` per pid, no runtime, no `wait()`. A killed child whose host then exits is reaped by the
/// OS, which is the whole requirement — the weights stop occupying memory and the port is free.
///
/// Killing by pid is safe here because a pid is only in this list while its `Child` is unreaped, and
/// the number cannot be handed to a new process until it is. `ServerHandle::kill` removes its own pid
/// as it kills, so a normal shutdown never leaves a tracked-but-dead entry behind.
///
/// Returns how many processes were signalled, so a caller can tell whether it had anything to kill.
pub fn kill_spawned_servers() -> usize {
    let Ok(mut pids) = SPAWNED.lock() else {
        return 0;
    };
    let count = pids.len();
    for pid in pids.drain(..) {
        #[cfg(unix)]
        // Safety: `kill(2)` is a plain syscall on a pid this process spawned; a pid that is already
        // gone is an `ESRCH`, which the return value we ignore here reports as nothing worth acting on.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
        #[cfg(not(unix))]
        let _ = pid;
    }
    count
}

/// One line of the on-disk mirror of [`SPAWNED`]: `host_pid server_pid program`.
///
/// The file exists for the one death no in-process hook can cover. A `SIGKILL`, an abort and a dev
/// restart all stop the host where it stands — and the server it was tracking goes on holding
/// gigabytes of weights under a pid nothing remembers (observed 2026-10-14: three `tauri dev`
/// restarts left three servers behind, ~4 GB each, with only their next instance to notice).
fn record_spawn(registry: Option<&Path>, server_pid: u32, program: &Path) {
    let Some(path) = registry else { return };
    let line = format!(
        "{} {} {}\n",
        std::process::id(),
        server_pid,
        program.display()
    );
    // Append, never rewrite: another host's lines must survive, and a torn final line is one the
    // reader below skips. A registry that cannot be written costs a leftover server, not a run that
    // could otherwise have worked.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = file.write_all(line.as_bytes());
    }
}

/// Kill the servers that a host which is no longer running recorded, and rewrite the file to hold
/// only the lines still worth having. Returns how many processes were signalled.
///
/// A line is acted on only when three things hold: its host pid is gone, its server pid is still
/// up, and that pid's executable is the program the line names. The last test is what makes killing
/// by a number read out of a file safe — pids are handed out again eventually, and the wrong target
/// would be a process the user started themselves.
#[cfg(unix)]
pub fn reap_orphaned_servers(registry: &Path) -> usize {
    let Ok(raw) = std::fs::read_to_string(registry) else {
        return 0;
    };
    let mut killed = 0;
    let mut kept: Vec<String> = Vec::new();
    for line in raw.lines() {
        let Some((host, server, program)) = parse_entry(line) else {
            continue;
        };
        // Its host is up: the server belongs to a run that is still watching it, this one included.
        if host == std::process::id() || pid_alive(host) {
            kept.push(line.to_string());
            continue;
        }
        if !pid_alive(server) {
            continue;
        }
        if executable_of(server).is_some_and(|found| found == program) {
            // Safety: `kill(2)` on a pid this file only ever holds for a server this app spawned,
            // after its name has been checked above.
            unsafe { libc::kill(server as libc::pid_t, libc::SIGKILL) };
            killed += 1;
        } else {
            // The number has been reused for something else. Leaving the line is the conservative
            // reading: it costs one more look next start, where acting on it could stop a stranger.
            kept.push(line.to_string());
        }
    }
    if kept.len() != raw.lines().count() {
        let mut rewritten = kept.join("\n");
        if !rewritten.is_empty() {
            rewritten.push('\n');
        }
        let _ = std::fs::write(registry, rewritten);
    }
    killed
}

/// Windows has no signal to catch and no pid to check cheaply; there the child is released by the
/// app-exit hook, and a Job Object is the way to cover a kill that runs no code.
#[cfg(not(unix))]
pub fn reap_orphaned_servers(_: &Path) -> usize {
    0
}

/// `host_pid server_pid program`, with the program allowed to contain spaces because an installed
/// app's resources usually do.
fn parse_entry(line: &str) -> Option<(u32, u32, String)> {
    let mut fields = line.splitn(3, ' ');
    let host = fields.next()?.parse().ok()?;
    let server = fields.next()?.parse().ok()?;
    let program = fields.next()?.trim_end().to_string();
    (!program.is_empty()).then_some((host, server, program))
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // Safety: `kill(2)` with signal 0 asks whether the pid exists and sends nothing. `EPERM` answers
    // "it exists, but is not ours", which is still a reason to leave it alone.
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// The executable a pid is running, as the kernel reports it. `None` when it is not running at all
/// or is not ours to ask about.
#[cfg(unix)]
fn executable_of(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let path = text.trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// The server's trailing output, kept so any later failure can quote the reason.
#[derive(Debug, Default)]
struct LogTail {
    lines: VecDeque<String>,
}

impl LogTail {
    fn push(&mut self, line: String) {
        if self.lines.len() == LOG_TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn text(&self) -> String {
        if self.lines.is_empty() {
            return "(the server produced no output)".to_string();
        }
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// A live `llama-server`, killed when this is dropped.
#[derive(Debug)]
pub struct ServerHandle {
    pub plan: LaunchPlan,
    child: Child,
    logs: Arc<Mutex<LogTail>>,
    /// Set once the child is observed to have exited, with its status line.
    exited: Option<String>,
}

impl ServerHandle {
    pub fn base_url(&self) -> &str {
        &self.plan.base_url
    }

    pub fn context_tokens(&self) -> u32 {
        self.plan.context_tokens
    }

    /// Trailing server output. Surfaced in the diagnostics view and in every launch error.
    pub fn log_tail(&self) -> String {
        self.logs.lock().map(|l| l.text()).unwrap_or_default()
    }

    /// Whether the child is still there to answer a request.
    pub fn is_running(&mut self) -> bool {
        self.observe_exit().is_none()
    }

    /// Ask the server to stop and reap it. Idempotent: `Drop` does the same thing without the wait.
    pub async fn shutdown(mut self) {
        self.kill();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
    }

    fn kill(&mut self) {
        if self.exited.is_none() {
            let _ = self.child.start_kill();
        }
        // The pid leaves the registry on the way out, so `kill_spawned_servers` only ever sees a
        // server nobody owns any more.
        if let Some(pid) = self.child.id() {
            untrack(pid);
        }
    }

    async fn wait_until_ready(&mut self, timeout: Duration) -> Result<(), LlamaError> {
        let url = self.plan.base_url.clone();
        let deadline = Instant::now() + timeout;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(LlamaError::Http)?;
        loop {
            if let Some(status) = self.observe_exit() {
                return Err(LlamaError::ServerExited {
                    status,
                    log: self.log_tail(),
                });
            }
            let answer = match http.get(format!("{url}/health")).send().await {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(r) => r.status().to_string(),
                Err(e) => e.to_string(),
            };
            if Instant::now() >= deadline {
                return Err(LlamaError::ServerUnhealthy {
                    url,
                    after_secs: timeout.as_secs().max(1),
                    status: answer,
                    log: self.log_tail(),
                });
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
    }

    /// Reap a child that exited on its own, so `is_running` and the error paths agree. Answers `None`
    /// only while the child is positively still running.
    ///
    /// A status that cannot be read is an exit too, because the two mistakes are not equally
    /// expensive: a false "running" points every later turn at a port nobody owns, which the user can
    /// only escape by quitting the app, while a false "exited" costs one restart that the host
    /// announces. On Unix a status becomes unreadable exactly when the pid stops being ours.
    fn observe_exit(&mut self) -> Option<String> {
        if let Some(status) = &self.exited {
            return Some(status.clone());
        }
        let text = match self.child.try_wait() {
            Ok(None) => return None,
            Ok(Some(status)) => format!("{status}"),
            Err(e) => format!("its exit status could not be read ({e})"),
        };
        self.exited = Some(text.clone());
        Some(text)
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Run `llama-server --version` and prove it is the pinned commit. Returns the parsed banner so the
/// caller can show the build number it actually started.
pub async fn verify_version(
    program: &Path,
    lock: &EngineLock,
) -> Result<EngineVersion, LlamaError> {
    let out = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| LlamaError::Spawn {
            program: program.to_path_buf(),
            source,
        })?;
    // llama.cpp writes its version banner on stderr, not stdout.
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let banner = if stderr.trim().is_empty() {
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        stderr
    };
    let version = parse_version(&banner);
    if !out.status.success() || !banner_matches_pin(&banner, lock) {
        return Err(LlamaError::VersionMismatch {
            program: program.display().to_string(),
            expected: lock.pinned_commit.clone(),
            found: version.commit.unwrap_or_default(),
            banner: banner.trim().to_string(),
        });
    }
    Ok(version)
}

/// Ask the kernel for a port on loopback and release it, so llama-server can bind it a moment later.
///
/// This is the standard ephemeral-port handover: between the drop and the child's `bind()` another
/// process could claim the number. For a single-user desktop app that window is microseconds, and a
/// collision surfaces as `ServerExited` carrying the server's own "bind: Address already in use".
pub fn pick_free_port() -> Result<u16, LlamaError> {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map(|l| l.local_addr().map(|a| a.port()).unwrap_or(0))
        .map_err(|source| LlamaError::Spawn {
            program: PathBuf::from("port selection"),
            source,
        })
}

/// Assemble the plan, prove the binary, spawn it, and return only once `/health` is green.
///
/// `req.tuning.port == 0` selects a free loopback port; an explicit port is used as given.
pub async fn start(
    mut req: LaunchRequest<'_>,
    lock: &EngineLock,
    ready_timeout: Duration,
) -> Result<ServerHandle, LlamaError> {
    if req.tuning.port == 0 {
        req.tuning.port = pick_free_port()?;
    }
    let plan = LaunchPlan::assemble(&req)?;
    let version = tokio::time::timeout(VERSION_PROBE, verify_version(&plan.program, lock))
        .await
        .map_err(|_| LlamaError::VersionProbeStalled {
            program: plan.program.clone(),
            after_secs: VERSION_PROBE.as_secs(),
        })??;
    tracing::info!(
        build = ?version.build,
        commit = ?version.commit,
        ctx = plan.context_tokens,
        ngl = plan.layers_offloaded,
        backend = plan.backend.label(),
        "launching the pinned fork engine"
    );

    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(req.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // run_llama.sh: `export LD_LIBRARY_PATH="$BIN_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"` — the
    // Linux CUDA assets ship their shared libraries beside the binary and will not load without it.
    if let Some(dir) = plan.library_path() {
        let mut path = OsString::from(dir);
        if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
            if !existing.is_empty() {
                path.push(":");
                path.push(existing);
            }
        }
        command.env("LD_LIBRARY_PATH", path);
    }
    let mut child = command.spawn().map_err(|source| LlamaError::Spawn {
        program: plan.program.clone(),
        source,
    })?;
    // Tracked from the instant it exists: the handle below only comes into being once `/health` is
    // green, and a load is the longest window in which the host can be told to die.
    if let Some(pid) = child.id() {
        track(pid);
        record_spawn(req.pid_registry.as_deref(), pid, &plan.program);
    }

    let logs = Arc::new(Mutex::new(LogTail::default()));
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, Arc::clone(&logs));
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, Arc::clone(&logs));
    }

    let mut handle = ServerHandle {
        plan,
        child,
        logs,
        exited: None,
    };
    if let Err(e) = handle.wait_until_ready(ready_timeout).await {
        handle.kill();
        return Err(e);
    }
    Ok(handle)
}

/// Drain one of the child's pipes into the shared tail.
///
/// Detached on purpose: the reader finishes by itself when the killed child's pipe closes, so nothing
/// is left behind and there is nothing to await on the way out.
fn pump<R>(pipe: R, logs: Arc<Mutex<LogTail>>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(pipe).lines();
        // EOF and a read error both end the task, which is what happens when the child is killed.
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "llama-server", "{line}");
            if let Ok(mut tail) = logs.lock() {
                tail.push(line);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process to stand in for a server: the same shape the reaper sees, without the weights.
    #[cfg(unix)]
    fn spawn_placeholder() -> std::process::Child {
        std::process::Command::new("/bin/sleep")
            .arg("120")
            .spawn()
            .expect("a placeholder for a server still holding its weights")
    }

    /// A pid that is not running: a child that finished and was reaped, so its number is released.
    #[cfg(unix)]
    fn dead_host_pid() -> u32 {
        let mut gone = std::process::Command::new("/usr/bin/true")
            .spawn()
            .expect("a host that will not outlive this line");
        let pid = gone.id();
        gone.wait().expect("the host is reaped");
        pid
    }

    #[test]
    fn the_log_tail_keeps_the_last_lines_and_says_when_there_are_none() {
        let mut tail = LogTail::default();
        assert_eq!(tail.text(), "(the server produced no output)");
        for i in 0..(LOG_TAIL_LINES + 40) {
            tail.push(format!("line {i}"));
        }
        let kept: Vec<String> = tail.lines.iter().cloned().collect();
        assert_eq!(kept.len(), LOG_TAIL_LINES);
        assert_eq!(kept.first().unwrap(), &format!("line {}", 40));
        assert!(tail
            .text()
            .ends_with(&format!("line {}", LOG_TAIL_LINES + 39)));
    }

    /// The port handover: a port we obtained and released must be bindable by the next caller.
    #[tokio::test]
    async fn a_selected_port_is_free_to_bind_immediately_after() {
        let port = pick_free_port().expect("a free loopback port");
        assert!(port > 0);
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .unwrap_or_else(|e| panic!("port {port} should be bindable again: {e}"));
    }

    /// The window the registry exists for: a spawned server that has no `ServerHandle` yet, because
    /// the handle is only built once `/health` answers. Flushing the registry has to be enough to
    /// stop it, with no runtime and no owner involved.
    #[cfg(unix)]
    #[test]
    fn a_tracked_child_dies_when_the_registry_is_flushed() {
        use std::os::unix::process::ExitStatusExt;

        let mut child = std::process::Command::new("/bin/sleep")
            .arg("120")
            .spawn()
            .expect("a placeholder for a server still paging weights in");
        let others = SPAWNED.lock().map(|pids| pids.len()).unwrap_or(0);
        track(child.id());

        assert_eq!(kill_spawned_servers(), others + 1);
        let status = child.wait().expect("the placeholder is reaped");
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "it must have been killed, not left to finish"
        );
        assert_eq!(
            kill_spawned_servers(),
            0,
            "a flush empties the registry, so nothing is signalled twice"
        );
    }

    /// A launch records who spawned it and what it is, which is everything the next run needs to
    /// tell a stranded server from one a live window owns.
    #[test]
    fn a_recorded_spawn_names_its_host_and_its_program() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("servers.pid");
        // The path the app installs under routinely contains spaces, so the program is the field
        // that absorbs them rather than being split by them.
        record_spawn(Some(&path), 4242, Path::new("/Users/a b/llama-server"));
        record_spawn(Some(&path), 4243, Path::new("/bin/sleep"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("registry"),
            format!(
                "{} 4242 /Users/a b/llama-server\n{} 4243 /bin/sleep\n",
                std::process::id(),
                std::process::id()
            )
        );
        // No file named is no file written: a caller that cannot be killed keeps the registry in
        // memory, which is what it asked for.
        record_spawn(None, 1, Path::new("/bin/sleep"));
        assert_eq!(parse_entry("nope"), None);
    }

    /// The death the in-memory registry cannot cover: the host is gone, so nothing is left that knows
    /// the server exists. This is the case the on-disk mirror is for.
    #[cfg(unix)]
    #[test]
    fn a_server_whose_host_is_gone_is_reaped_by_the_next_run() {
        use std::os::unix::process::ExitStatusExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("servers.pid");
        let mut stranded = spawn_placeholder();
        let line = format!("{} {} /bin/sleep\n", dead_host_pid(), stranded.id());
        std::fs::write(&path, &line).expect("registry");

        assert_eq!(reap_orphaned_servers(&path), 1);
        assert_eq!(
            stranded.wait().expect("reaped").signal(),
            Some(libc::SIGKILL),
            "the leftover server must actually stop"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("registry"),
            "",
            "a reaped line is not worth carrying to the next start"
        );
    }

    /// Two of the three reasons a line is *not* acted on. A host that is still up is watching its own
    /// server — a second window starting must not kill the first one's engine — and a pid whose
    /// program is not the one recorded has been handed out again since, so killing it would stop a
    /// process the user started themselves.
    #[cfg(unix)]
    #[test]
    fn a_line_is_only_acted_on_when_its_host_is_gone_and_its_program_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("servers.pid");

        let mut watched = spawn_placeholder();
        std::fs::write(
            &path,
            format!("{} {} /bin/sleep\n", std::process::id(), watched.id()),
        )
        .expect("registry");
        assert_eq!(reap_orphaned_servers(&path), 0, "its host is alive");
        assert!(watched.try_wait().expect("polled").is_none());

        let mut reused = spawn_placeholder();
        std::fs::write(
            &path,
            format!("{} {} /bin/cat\n", dead_host_pid(), reused.id()),
        )
        .expect("registry");
        assert_eq!(
            reap_orphaned_servers(&path),
            0,
            "the recorded program is not what holds the pid"
        );
        assert!(reused.try_wait().expect("polled").is_none());
        reused.kill().expect("cleaned up");
        watched.kill().expect("cleaned up");
    }

    /// The mechanism `live_turn` depends on: a server killed by something other than the handle —
    /// `pkill`, a crash, the OOM killer — must stop reading as running, or the next turn is pointed at
    /// a dead port instead of a restart.
    ///
    /// Built the way `start` builds it, because the pipes and the readers are part of the claim: a
    /// child whose stdout is being drained is the child a real server is.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_killed_from_outside_is_reported_dead() {
        use smollm_models::hardware::Backend;

        let mut child = Command::new("/bin/sleep")
            .arg("120")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("a placeholder for a running server");
        let pid = child.id().expect("a child that has not been reaped");
        let logs = Arc::new(Mutex::new(LogTail::default()));
        if let Some(stdout) = child.stdout.take() {
            pump(stdout, Arc::clone(&logs));
        }
        if let Some(stderr) = child.stderr.take() {
            pump(stderr, Arc::clone(&logs));
        }
        let mut handle = ServerHandle {
            plan: LaunchPlan {
                program: "/bin/sleep".into(),
                args: Vec::new(),
                backend: Backend::Cpu,
                layers_offloaded: 0,
                context_tokens: 512,
                base_url: "http://127.0.0.1:1".into(),
                display: "sleep 120".into(),
            },
            child,
            logs,
            exited: None,
        };
        assert!(handle.is_running(), "a live child is running");

        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
        for _ in 0..40 {
            if !handle.is_running() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the SIGKILL was never observed, so the slot would keep using a dead server");
    }

    #[tokio::test]
    async fn the_version_probe_reports_the_pinned_commit_and_rejects_any_other() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lock = EngineLock::load_default(&root).expect("lock parses");
        // The fake banner is what a stock build prints: the fork's build number with upstream's
        // commit, which `banner_matches_pin` must refuse.
        let fake = "version: 10772 (0a1b2c3)\nbuilt with AppleClang 17 for arm64".to_string();
        assert!(!banner_matches_pin(&fake, &lock));
        assert!(banner_matches_pin(
            &format!("version: 10772 ({})", lock.pinned_commit),
            &lock
        ));
    }
}
