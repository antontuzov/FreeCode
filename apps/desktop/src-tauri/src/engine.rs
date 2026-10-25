//! The only path in the app that writes to `engines/`, and the only one that talks to GitHub.
//!
//! The status strip used to print a URL and ask the user to unpack it by hand. It does not have to:
//! `freecode_llama::install` resolves the pinned asset from `engines/lock.toml`, checks the digest the
//! lock recorded, and unpacks into exactly the directory `EngineBinary` looks in. This module is the
//! thin host around that — one command, one progress event.
//!
//! Progress travels as an event rather than a return value because a 340 MB CUDA download is
//! measured in minutes; the UI re-reads `model_status` once the command returns, so a missed event
//! can leave a stale progress bar but never a stale verdict.

use std::path::PathBuf;

use tauri::{AppHandle, Emitter};

use freecode_llama::install::{self, Installed, Phase};
use freecode_llama::EngineBinary;

use crate::{appearance, paths, status};

/// Event name the Model Manager listens to while an install runs.
pub const PROGRESS_EVENT: &str = "engine-install";

/// The engine that ended up on disk, and where the launcher must point.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub tag: String,
    pub target_key: String,
    pub asset: String,
    pub sha256: String,
    pub server_path: PathBuf,
}

impl From<Installed> for InstallOutcome {
    fn from(installed: Installed) -> Self {
        Self {
            tag: installed.tag,
            target_key: installed.target_key,
            asset: installed.asset,
            sha256: installed.sha256,
            server_path: installed.server_path,
        }
    }
}

pub async fn install_engine(app: &AppHandle) -> Result<InstallOutcome, String> {
    let root = paths::workspace_root(Some(app))?;
    let settings = appearance::load_settings()?;
    // The same assembly the status strip rendered, so the install cannot target a different engine
    // than the one the refusal named. Blocking (directory scans, `sysctl`) so off the event loop.
    let blocking_root = root.clone();
    let assembly =
        tauri::async_runtime::spawn_blocking(move || status::assemble(&blocking_root, &settings))
            .await
            .map_err(|e| format!("engine resolution failed: {e}"))??;
    let lock = assembly.lock;
    let engine = assembly
        .engine
        .ok_or("no PrismML fork build is pinned for this machine's backend")?;

    let client = reqwest::Client::builder()
        // GitHub's release CDN rejects a request with no User-Agent.
        .user_agent(concat!("freecode/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("cannot build the download client: {e}"))?;
    let emitter = app.clone();
    let on_phase = move |phase: Phase| {
        let _ = emitter.emit(PROGRESS_EVENT, &phase);
    };
    let installed = install::install(&lock, &root, &engine, &client, &on_phase)
        .await
        .map_err(|e| e.to_string())?;

    // Success is the resolver's own probe agreeing, not the unpack returning Ok: if the tree landed
    // somewhere `EngineBinary` cannot see it, the next status refresh would still say "not
    // installed" and the user would have paid 340 MB for nothing.
    let recheck = EngineBinary::from_target(&root, &lock, &engine.target)
        .map_err(|e| format!("engine installed but cannot be resolved: {e}"))?;
    if !recheck.installed {
        return Err(format!(
            "installed {} but {} is not where the engine resolver looks",
            installed.asset,
            recheck.server_path.display()
        ));
    }
    Ok(InstallOutcome::from(installed))
}
