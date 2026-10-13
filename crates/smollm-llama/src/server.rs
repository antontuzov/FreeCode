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
//! cannot leave a 6 GB server holding the weights.

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

    pub fn is_running(&mut self) -> bool {
        self.exited.is_none() && self.child.try_wait().ok().flatten().is_none()
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

    /// Reap a child that exited on its own, so `is_running` and the error paths agree.
    fn observe_exit(&mut self) -> Option<String> {
        if let Some(status) = &self.exited {
            return Some(status.clone());
        }
        let status = self.child.try_wait().ok().flatten()?;
        let text = format!("{status}");
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
