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
/// `engine-install` event and the answer is the final state. This is the app's only outbound request
/// on the local-model path, and it happens because the user pressed Install.
#[tauri::command]
async fn install_engine(app: AppHandle) -> Result<engine::InstallOutcome, String> {
    engine::install_engine(&app).await
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // The supervised server and the conversation it is answering from. One window, one engine:
        // the state is here so a second `start_engine` cannot begin a second 6 GB load.
        .manage(chat::ChatState::default())
        .invoke_handler(tauri::generate_handler![
            model_status,
            install_engine,
            load_appearance,
            save_appearance,
            load_settings,
            save_settings,
            chat::start_engine,
            chat::stop_engine,
            chat::engine_status,
            chat::chat_send
        ])
        .run(tauri::generate_context!())
        .expect("error while running SmoLLM Studio");
}
