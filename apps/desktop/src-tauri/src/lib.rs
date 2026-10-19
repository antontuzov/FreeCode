//! SmoLLM Studio's Tauri host.
//!
//! The crate's only jobs are to expose the Rust side to React and to keep that surface honest: the
//! UI never computes a packing choice, an engine path or a refusal — it asks, and this host answers
//! from `smollm-models` / `smollm-llama` / `smollm-config`. Commands are `async` so the blocking
//! hardware probe and header parse run on the async runtime instead of the event-loop thread.

mod agent;
mod appearance;
mod brand;
mod chat;
mod clipboard;
mod engine;
mod extensions;
mod knowledge;
mod paths;
mod sessions;
mod status;
pub mod verify_native;
mod weights;

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Theme};

use appearance::AppearanceView;
use knowledge::Center;
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

/// The catalog as a page: every model and packing the pins name, and which of those files this
/// machine holds. The Knowledge Center's only source.
#[tauri::command]
async fn knowledge_center(app: AppHandle) -> Result<Center, String> {
    let (root, settings) = root_and_settings(&app)?;
    tauri::async_runtime::spawn_blocking(move || Center::build(&root, &settings))
        .await
        .map_err(|e| format!("catalog listing failed: {e}"))?
}

/// The Extensions page: the tool registry as the agent is handed it, with the current approval dial
/// and sandbox applied to every row by the same engine that decides a call mid-run.
#[tauri::command]
async fn extensions_inventory(app: AppHandle) -> Result<extensions::Inventory, String> {
    let (root, settings) = root_and_settings(&app)?;
    tauri::async_runtime::spawn_blocking(move || extensions::Inventory::build(&root, &settings))
        .await
        .map_err(|e| format!("tool inventory failed: {e}"))
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
    wear_icon(&app, settings.appearance.app_icon);
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
async fn save_settings(app: AppHandle, mut settings: Settings) -> Result<Settings, String> {
    settings.schema_version = smollm_config::SETTINGS_SCHEMA_VERSION;
    appearance::save_settings(&settings)?;
    wear_icon(&app, settings.appearance.app_icon);
    Ok(settings)
}

/// Move the app's face to the tile the document names. The pin is host state because the Dock is
/// host-drawn, so a save from either settings surface has to hand it over and repaint: without the
/// second half the window shows the duck the user picked while its own tile stays the one it launched
/// with. Clearing the pin hands this launch's roll back rather than choosing a new one.
fn wear_icon(app: &AppHandle, pinned: Option<u8>) {
    brand::prefer(pinned.map(usize::from));
    brand::set_dock_tile(app);
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
    // Read before the builder below asks for the mascot's number: a user who pinned a tile should
    // open the app wearing it, on the Dock and in the window both, from the first frame. Nothing
    // here is fatal — a document that will not parse leaves the pin unset, which is the roll that
    // shipped before there was a picker, and the same read reports the failure later.
    let pinned = appearance::load_settings()
        .ok()
        .and_then(|settings| settings.appearance.app_icon)
        .map(usize::from);
    brand::prefer(pinned);

    let app = tauri::Builder::default()
        // The supervised server and the conversation it is answering from. One window, one engine:
        // the state is here so a second `start_engine` cannot begin a second 6 GB load.
        .manage(chat::ChatState::default())
        // The one way this app leaves the machine for the user's browser: refusal links and the links
        // in an answer. The UI cannot reach it directly — the capability scopes it to `http` and
        // `https`, so a `file://` or a `javascript:` address in a model's own text never launches.
        .plugin(tauri_plugin_opener::init())
        // The duck this launch wears, handed to the page before any of its own scripts run, so the
        // window and the Dock tile are the same picture from the first frame rather than a moment
        // apart. See `brand`.
        .append_invoke_initialization_script(brand::init_script())
        .invoke_handler(tauri::generate_handler![
            model_status,
            // What the catalog pins and what this machine has of it, for the Knowledge Center.
            knowledge_center,
            // What the agent can reach and what this build runs, for the Extensions page.
            extensions_inventory,
            install_engine,
            download_weights,
            load_appearance,
            save_appearance,
            load_settings,
            save_settings,
            // The one write the UI may make to the machine it runs on: the text of a code fence, onto
            // the pasteboard. See `clipboard`.
            clipboard::copy_text,
            chat::start_engine,
            chat::stop_engine,
            chat::engine_status,
            chat::chat_send,
            chat::chat_stop,
            chat::new_task,
            // The conversations the app has filed, and the two controls the sidebar's rows have: reopen
            // one, or throw one away. The palette searches all of them by what was said in them, which
            // is why the search reads the files rather than the list.
            chat::list_sessions,
            chat::search_sessions,
            chat::open_session,
            chat::delete_session
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

    app.run(|app_handle, event| match event {
        // The mascot goes on the Dock here rather than in `setup`, because in a dev build Tauri puts
        // the built icon there itself when the event loop reports Ready — after the setup hook and
        // before this callback. Whoever answers Ready last is the one whose face you see.
        tauri::RunEvent::Ready => brand::set_dock_tile(app_handle),
        tauri::RunEvent::Exit => release_engine(app_handle),
        _ => {}
    });
}
