//! SmoLLM Studio's Tauri host.
//!
//! The crate's only jobs are to expose the Rust side to React and to keep that surface honest: the
//! UI never computes a packing choice, an engine path or a refusal — it asks, and this host answers
//! from `smollm-models` / `smollm-llama` / `smollm-config`. Commands are `async` so the blocking
//! hardware probe and header parse run on the async runtime instead of the event-loop thread.

mod appearance;
mod chat;
mod engine;
mod paths;
mod status;
pub mod verify_native;
mod weights;

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Theme};

use appearance::AppearanceView;
use smollm_config::Settings;
use status::ModelStatusView;

/// `System` asks the webview; a fixed Light/Dark is honoured even if the OS disagrees.
fn os_prefers_light(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|window| window.theme().ok())
        .is_some_and(|theme| matches!(theme, Theme::Light))
}

pub(crate) fn root_and_settings(app: &AppHandle) -> Result<(PathBuf, Settings), String> {
    let root = paths::workspace_root(Some(app))?;
    Ok((root, appearance::load_settings()?))
}

#[tauri::command]
async fn model_status(app: AppHandle) -> Result<ModelStatusView, String> {
    let (root, settings) = root_and_settings(&app)?;
    // Multi-GB directories and `sysctl` probes belong off the event loop.
    tauri::async_runtime::spawn_blocking(move || status::build(&root, &settings))
        .await
        .map_err(|e| format!("model status probe failed: {e}"))?
}

#[tauri::command]
async fn load_appearance(app: AppHandle) -> Result<AppearanceView, String> {
    let settings = appearance::load_settings()?;
    Ok(appearance::view(
        &settings.appearance,
        os_prefers_light(&app),
    ))
}

/// Fetch the pinned fork engine and unpack it into `engines/`. Progress arrives on the
/// `engine-install` event and the answer is the final state. One of the two fetches this app makes
/// on the local-model path, and it only happens because the user pressed Install.
#[tauri::command]
async fn install_engine(app: AppHandle) -> Result<engine::InstallOutcome, String> {
    engine::install_engine(&app).await
}

/// Download the packing the status strip chose into `models/`, resuming whatever is already in
/// flight. Progress arrives on the `weights-download` event; the answer is the file that landed.
#[tauri::command]
async fn download_weights(app: AppHandle) -> Result<weights::Downloaded, String> {
    weights::download_weights(&app).await
}

/// Persists the shell's controls and echoes what the host now believes, so the UI never keeps a
/// stale `resolvedMode`. `resolvedMode` is host-computed and is not read from the payload.
#[tauri::command]
async fn save_appearance(
    app: AppHandle,
    appearance: AppearanceView,
) -> Result<AppearanceView, String> {
    let mut settings = crate::appearance::load_settings()?;
    crate::appearance::apply(&appearance, &mut settings.appearance);
    crate::appearance::save_settings(&settings)?;
    Ok(crate::appearance::view(
        &settings.appearance,
        os_prefers_light(&app),
    ))
}

/// The whole settings document, for the Settings surface. Deserialising on the way out is the
/// validation: a payload the schema rejects never reaches the file.
#[tauri::command]
async fn load_settings() -> Result<Settings, String> {
    appearance::load_settings()
}

/// The Settings surface edits the loaded document in place and hands it back, so a field it does
/// not render still round-trips untouched. The host answers with what it stored.
#[tauri::command]
async fn save_settings(mut settings: Settings) -> Result<Settings, String> {
    settings.schema_version = smollm_config::SETTINGS_SCHEMA_VERSION;
    appearance::save_settings(&settings)?;
    Ok(settings)
}

/// The supervised server is a child process, and nothing on macOS makes a child die with its parent,
/// so releasing it is this process's job on every way out of the run.
fn release_engine(app: &AppHandle) {
    let state = app.state::<chat::ChatState>();
    chat::release_engine(&state);
}

/// The other half of [`release_engine`], for the way out that runs none of this process's code.
///
/// A `SIGKILL` — what a force quit, a crash and `tauri dev`'s restart send — stops the host where it
/// stands, and its server goes on holding gigabytes of weights under a pid nothing remembers. Each
/// launch records that pid beside the host's own, so the next run can tell a stranded server from one
/// a live window is still watching. Observed 2026-10-14: three dev restarts left three servers behind.
fn reap_leftover_servers() {
    let Some(path) = paths::server_pid_registry() else {
        return;
    };
    smollm_llama::reap_orphaned_servers(&path);
}

/// SIGTERM, SIGINT and SIGHUP never reach [`RunEvent::Exit`]: their default action stops the process
/// where it stands, which is how a supervised server is orphaned (observed 2026-10-13 — `kill -TERM`
/// to the host left `llama-server` holding its port and its weights). Registering a listener takes
/// those signals over, so the server dies first and the process then exits with the status the signal
/// would have produced anyway.
///
/// `std::process::exit` rather than `AppHandle::exit`: a graceful teardown is precisely the thing that
/// might not return, and the one promise this path makes is that the child does not outlive the host.
#[cfg(unix)]
fn watch_signals(app: AppHandle) {
    use tokio::signal::unix::{signal, SignalKind};
    for kind in [
        SignalKind::terminate(),
        SignalKind::interrupt(),
        SignalKind::hangup(),
    ] {
        let app = app.clone();
        let status = 128 + kind.as_raw_value();
        // Registration needs the reactor, so it happens inside the task rather than in `setup`.
        tauri::async_runtime::spawn(async move {
            let Ok(mut stream) = signal(kind) else { return };
            if stream.recv().await.is_some() {
                release_engine(&app);
                std::process::exit(status);
            }
        });
    }
}

/// On Windows the equivalent is a Job Object; there is no signal to catch. The app-exit hook covers
/// the paths that exist there.
#[cfg(not(unix))]
fn watch_signals(_: AppHandle) {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // The supervised server and the conversation it is answering from. One window, one engine:
        // the state is here so a second `start_engine` cannot begin a second 6 GB load.
        .manage(chat::ChatState::default())
        .invoke_handler(tauri::generate_handler![
            model_status,
            install_engine,
            download_weights,
            load_appearance,
            save_appearance,
            load_settings,
            save_settings,
            chat::start_engine,
            chat::stop_engine,
            chat::engine_status,
            chat::chat_send,
            chat::new_task
        ])
        .setup(|app| {
            // Before this process spawns a server of its own, so a leftover's port is free to be
            // chosen again and its memory is back before the first load.
            reap_leftover_servers();
            watch_signals(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building SmoLLM Studio");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            release_engine(app_handle);
        }
    });
}
