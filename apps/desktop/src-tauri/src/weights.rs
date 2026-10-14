//! The only path in the app that writes to `models/`.
//!
//! Weights come down through the same discipline as the engine archive — resume plus a streaming
//! sha256 from [`smollm_models::download`], guarded by the size and digest the catalog published —
//! and they land through a `…gguf.part` sibling whose name no pack resolves to. A transfer killed
//! halfway therefore still reads as "not downloaded", which is the honest verdict, rather than as a
//! model file that failed verification.
//!
//! Nothing here chooses *which* packing to fetch. The host runs the same `status::assemble` proof
//! chain the status strip rendered, takes the pack that chain selected, and writes it into the
//! directory the chain reads; those two facts are the whole contract. Fetch the pack the
//! recommendation did not choose, or write it somewhere the probe does not look, and the load guard's
//! verdict never changes — the user pays 463 MB for a button that still refuses.
//!
//! The token is an environment variable *name* read from settings. The downloader reads its value at
//! call time; this module never stores, logs, or echoes it.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use smollm_llama::load_guard::FileState;
use smollm_models::download::{self, Progress};
use smollm_models::DEFAULT_HEADROOM_BYTES;

use crate::status;

/// Event name the Model Manager listens to while a download runs.
pub const PROGRESS_EVENT: &str = "weights-download";

/// What the UI is told while a download runs, in bytes rather than a percentage — the total is the
/// catalog's `size_bytes` and the user can check both against the published file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "phase")]
pub enum Phase {
    #[serde(rename_all = "camelCase")]
    Downloading {
        transferred_bytes: u64,
        total_bytes: u64,
    },
    Done {
        downloaded: Downloaded,
    },
}

/// The pack now on disk, and where the launcher will read it from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Downloaded {
    pub catalog_id: String,
    pub packing: String,
    pub file: String,
    pub sha256: String,
    pub path: PathBuf,
    pub size_bytes: u64,
}

pub async fn download_weights(app: &AppHandle) -> Result<Downloaded, String> {
    let (root, settings) = crate::root_and_settings(app)?;
    // Directory scans and `sysctl`, so off the event loop — and so the pack fetched here is provably
    // the pack the refusal on screen named.
    let blocking_root = root.clone();
    let blocking_settings = settings.clone();
    let assembly = tauri::async_runtime::spawn_blocking(move || {
        status::assemble(&blocking_root, &blocking_settings)
    })
    .await
    .map_err(|e| format!("weight resolution failed: {e}"))??;

    let weights = status::weights_dir(&root, &settings);
    let pack = &assembly.pack;
    let plan = assembly
        .model
        .plan_for(pack, &weights, settings.local_model.hf_token_env.clone());

    // Refuse at zero bytes rather than at ninety percent with a full disk. `available_disk_bytes` is
    // the same probe that chose this packing, so the number the UI quoted and the one checked here
    // cannot disagree.
    download::disk_preflight(
        assembly.host.available_disk_bytes,
        &plan,
        DEFAULT_HEADROOM_BYTES,
    )
    .map_err(|e| e.to_string())?;

    // A fresh checkout has no `models/` at all, and the downloader streams into `<dest>.part` before
    // it ever creates a directory.
    std::fs::create_dir_all(&weights).map_err(|e| {
        format!(
            "cannot create the weights directory {}: {e}",
            weights.display()
        )
    })?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("smollm-studio/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("cannot build the download client: {e}"))?;
    let emitter = app.clone();
    let reporter = app.clone();
    let on_progress = move |p: Progress| {
        let _ = emitter.emit(
            PROGRESS_EVENT,
            &Phase::Downloading {
                transferred_bytes: p.transferred_bytes,
                total_bytes: p.total_bytes,
            },
        );
    };
    // `download` resumes an existing part, restarts one the server refused to range, and only renames
    // after the streaming sha256 matched the catalog — so an `Ok` here is a verified file.
    download::download(&client, &plan, on_progress)
        .await
        .map_err(|e| e.to_string())?;

    // Success is the status probe's own reading of the path, not the transfer returning: the guard
    // checks `<weights dir>/<catalog file name>`, so a file that landed anywhere else would leave the
    // very refusal the button was pressed to clear.
    let dest = plan.dest.clone();
    let size_bytes = plan.expected_size_bytes;
    let sha256 = plan.expected_sha256.clone();
    let recheck = tauri::async_runtime::spawn_blocking(move || {
        status::file_state(&dest, size_bytes, &sha256)
    })
    .await
    .map_err(|e| format!("weight verification failed: {e}"))?;
    if recheck != FileState::Verified {
        return Err(format!(
            "downloaded {} to {} but the status probe reads it as {}",
            pack.file,
            plan.dest.display(),
            describe(&recheck)
        ));
    }

    let downloaded = Downloaded {
        catalog_id: assembly.model.id.clone(),
        packing: pack.key.clone(),
        file: pack.file.clone(),
        sha256: pack.sha256.clone(),
        path: plan.dest.clone(),
        size_bytes: pack.size_bytes,
    };
    let _ = reporter.emit(
        PROGRESS_EVENT,
        &Phase::Done {
            downloaded: downloaded.clone(),
        },
    );
    Ok(downloaded)
}

/// The recheck's own words, for an error the user has to act on.
fn describe(state: &FileState) -> String {
    match state {
        FileState::Absent => "not downloaded".into(),
        FileState::Mismatch { status } => format!("a mismatch ({status})"),
        FileState::Unverifiable => "unverifiable, with no catalog sha256".into(),
        FileState::Verified => "verified".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The event payload shape is what the UI's progress bar reads, and `ipc.ts` mirrors it by name.
    /// A rename on either side silently strands the bar at zero.
    #[test]
    fn progress_and_result_serialize_to_the_fields_the_ui_reads() {
        assert_eq!(
            serde_json::to_value(Phase::Downloading {
                transferred_bytes: 400,
                total_bytes: 1_000
            })
            .unwrap(),
            serde_json::json!({
                "phase": "downloading",
                "transferredBytes": 400,
                "totalBytes": 1_000
            })
        );
        let done = serde_json::to_value(Phase::Done {
            downloaded: Downloaded {
                catalog_id: "ternary-bonsai-1-7b".into(),
                packing: "PQ2_0".into(),
                file: "Ternary-Bonsai-1.7B-PQ2_0.gguf".into(),
                sha256: "de68".into(),
                path: PathBuf::from("/models/Ternary-Bonsai-1.7B-PQ2_0.gguf"),
                size_bytes: 463_290_464,
            },
        })
        .unwrap();
        assert_eq!(done["phase"], "done");
        assert_eq!(done["downloaded"]["packing"], "PQ2_0");
        assert_eq!(
            done["downloaded"]["path"],
            "/models/Ternary-Bonsai-1.7B-PQ2_0.gguf"
        );
    }

    /// A `.part` beside the pack is invisible to the probe, which is what makes an interrupted
    /// transfer report "not downloaded" instead of a corrupted model.
    #[test]
    fn an_interrupted_transfer_leaves_nothing_the_probe_mistakes_for_a_pack() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Ternary-Bonsai-1.7B-PQ2_0.gguf");
        std::fs::write(&file, "partial").unwrap();
        // Size is the whole check on the status path, so a truncated file is a mismatch, never
        // verified — and clearing it is the download's job, not the probe's.
        assert!(matches!(
            status::file_state(&file, 463_290_464, "not-empty"),
            FileState::Mismatch { .. }
        ));
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            status::file_state(&file, 463_290_464, "not-empty"),
            FileState::Absent
        );

        let part = download::DownloadPlan {
            url: String::new(),
            dest: file.clone(),
            expected_size_bytes: 0,
            expected_sha256: String::new(),
            token_env: None,
        }
        .part_path();
        assert_ne!(part, file, "the scratch name must not be the pack's name");
    }
}
