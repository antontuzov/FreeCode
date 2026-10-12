//! SmoLLM Studio's Tauri host.
//!
//! The crate's only jobs are to expose the Rust side to React and to keep that surface honest: the
//! UI never computes a packing choice, an engine path or a refusal — it asks, and this host answers
//! from `smollm-models` / `smollm-llama` / `smollm-config`. Commands are `async` so the blocking
//! hardware probe and header parse run on the async runtime instead of the event-loop thread.

mod appearance;
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

fn root_and_settings(app: &AppHandle) -> Result<(PathBuf, Settings), String> {
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

/// Persists the shell's five controls and echoes what the host now believes, so the UI never keeps
/// a stale `resolvedMode`. `resolvedMode` is host-computed and is not read from the payload.
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            model_status,
            load_appearance,
            save_appearance
        ])
        .run(tauri::generate_context!())
        .expect("error while running SmoLLM Studio");
}
