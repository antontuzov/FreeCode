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
//! The projector the same model's card names is fetched by the same press, because a model that reads
//! pictures is only usable as one, and a pack that arrives without the file under it leaves the
//! composer's picture button with nothing to send to. Every file in the list is first checked against
//! this machine: a pack already at the published size is not fetched twice, which is the difference
//! between a button that clears a refusal and one that re-pays it.
//!
//! The token is an environment variable *name* read from settings. The downloader reads its value at
//! call time; this module never stores, logs, or echoes it.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use smollm_llama::load_guard::FileState;
use smollm_models::catalog::{ModelEntry, Pack};
use smollm_models::download::{self, DownloadPlan, Progress};
use smollm_models::{gguf, IntegrityStatus, DEFAULT_HEADROOM_BYTES};

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
    /// The projector that came down with it, by name, or `None` when the model publishes none or this
    /// machine already had it. A picture cannot be answered without this file, so a result that
    /// fetched it says so rather than leaving the user to work out which of the two the button did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projector: Option<String>,
}

/// One file this press is on the hook for, and the catalog's word for how big it is.
struct Fetch {
    plan: DownloadPlan,
    /// The name to report it under once it lands.
    file: String,
    projector: bool,
}

/// The pack the proof chain selected, then the projector the same model's card names — minus anything
/// this machine already holds at the published size. The probe is the app's own `check_size`, the one
/// the status card and the Knowledge Center read, so the press can never promise a file a surface
/// above it already calls present, and a second press after an interrupted one re-fetches only the
/// half that is actually missing.
fn to_fetch(
    model: &ModelEntry,
    pack: &Pack,
    weights: &Path,
    token_env: Option<String>,
) -> Vec<Fetch> {
    let mut wanted = vec![(
        pack.file.clone(),
        model.plan_for(pack, weights, token_env.clone()),
        false,
    )];
    if let Some(proj) = model.recommended_vision() {
        wanted.push((
            proj.file.clone(),
            model.plan_for_vision(proj, weights, token_env),
            true,
        ));
    }
    wanted
        .into_iter()
        .filter(|(_, plan, _)| {
            !matches!(
                gguf::check_size(&plan.dest, plan.expected_size_bytes),
                Ok(IntegrityStatus::Ok)
            )
        })
        .map(|(file, plan, projector)| Fetch {
            plan,
            file,
            projector,
        })
        .collect()
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
    let token_env = settings.local_model.hf_token_env.clone();
    // A directory scan is blocking work too, so the whole fetch list — the pack, its projector, and
    // which of the two this machine already holds — is decided off the event loop.
    let missing = {
        let (model, pack, weights, token_env) = (
            assembly.model.clone(),
            assembly.pack.clone(),
            weights.clone(),
            token_env.clone(),
        );
        tauri::async_runtime::spawn_blocking(move || to_fetch(&model, &pack, &weights, token_env))
            .await
            .map_err(|e| format!("weight scan failed: {e}"))?
    };
    let pack = &assembly.pack;

    // Refuse at zero bytes rather than at ninety percent with a full disk. `available_disk_bytes` is
    // the same probe that chose this packing, so the number the UI quoted and the one checked here
    // cannot disagree. Every file is measured against the space that will still be free once the
    // ones before it land, so a pack that fits cannot be followed by a projector that does not.
    let mut committed: u64 = 0;
    for fetch in &missing {
        download::disk_preflight(
            assembly.host.available_disk_bytes.saturating_sub(committed),
            &fetch.plan,
            DEFAULT_HEADROOM_BYTES,
        )
        .map_err(|e| e.to_string())?;
        committed += fetch.plan.expected_size_bytes;
    }

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
    // One bar for the whole press, so a projector arriving after the pack does not restart a progress
    // report the user is watching: the bytes already on disk and the bytes still to come are added
    // into one total the catalog published.
    let total_bytes: u64 = missing.iter().map(|f| f.plan.expected_size_bytes).sum();
    let mut before = 0u64;
    for fetch in &missing {
        let emitter = emitter.clone();
        let base = before;
        let on_progress = move |p: Progress| {
            let _ = emitter.emit(
                PROGRESS_EVENT,
                &Phase::Downloading {
                    transferred_bytes: base + p.transferred_bytes,
                    total_bytes,
                },
            );
        };
        // `download` resumes an existing part, restarts one the server refused to range, and only
        // renames after the streaming sha256 matched the catalog — so an `Ok` here is a verified file.
        download::download(&client, &fetch.plan, on_progress)
            .await
            .map_err(|e| e.to_string())?;
        before += fetch.plan.expected_size_bytes;
    }

    // Success is the status probe's own reading of the path, not the transfer returning: the guard
    // checks `<weights dir>/<catalog file name>`, so a file that landed anywhere else would leave the
    // very refusal the button was pressed to clear.
    let pack_plan = assembly.model.plan_for(pack, &weights, token_env);
    let dest = pack_plan.dest.clone();
    let size_bytes = pack_plan.expected_size_bytes;
    let sha256 = pack_plan.expected_sha256.clone();
    let recheck = tauri::async_runtime::spawn_blocking(move || {
        status::file_state(&dest, size_bytes, &sha256)
    })
    .await
    .map_err(|e| format!("weight verification failed: {e}"))?;
    if recheck != FileState::Verified {
        return Err(format!(
            "downloaded {} to {} but the status probe reads it as {}",
            pack.file,
            pack_plan.dest.display(),
            describe(&recheck)
        ));
    }

    let projector = missing.iter().find(|f| f.projector).map(|f| f.file.clone());
    let downloaded = Downloaded {
        catalog_id: assembly.model.id.clone(),
        packing: pack.key.clone(),
        file: pack.file.clone(),
        sha256: pack.sha256.clone(),
        path: pack_plan.dest.clone(),
        size_bytes: pack.size_bytes,
        projector,
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

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

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
                projector: None,
            },
        })
        .unwrap();
        assert_eq!(done["phase"], "done");
        assert_eq!(done["downloaded"]["packing"], "PQ2_0");
        assert_eq!(
            done["downloaded"]["path"],
            "/models/Ternary-Bonsai-1.7B-PQ2_0.gguf"
        );
        // A model that publishes none reports none, rather than a null the UI has to guess about.
        assert!(done["downloaded"].get("projector").is_none());
    }

    /// A model that publishes a projector owes it to the same press that fetches its pack, and one this
    /// machine already has at the published size is not fetched again — the button clears a refusal, it
    /// does not re-pay a transfer.
    #[test]
    fn the_projector_lands_with_its_pack_and_what_is_already_here_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let weights = dir.path().to_path_buf();
        let catalog = smollm_models::Catalog::load_default(&repo_root())
            .expect("the shipped catalog loads for this test");
        let mut four = catalog
            .model("ternary-bonsai-2-4b")
            .expect("the vision model is in the catalog")
            .clone();
        // The published lengths are 1.15 GB and 366 MB, and the probe that decides what to fetch is
        // size-only — so they are stood in for at lengths of this test's choosing rather than written
        // out at the cost of the download each one describes.
        const PACK_BYTES: u64 = 8_192;
        const PROJ_BYTES: u64 = 4_096;
        for pack in four.packs.iter_mut() {
            pack.size_bytes = PACK_BYTES;
        }
        for proj in four.vision.iter_mut() {
            proj.size_bytes = PROJ_BYTES;
        }
        let pack = four
            .packs
            .iter()
            .find(|p| p.key == "PQ2_0")
            .expect("the 4B publishes a 2-bit pack")
            .clone();
        let proj = four
            .recommended_vision()
            .expect("the 4B publishes a projector")
            .clone();

        let hold = |name: &str, size: u64| {
            std::fs::write(weights.join(name), vec![0u8; size as usize]).unwrap();
        };
        // The press's own list, so this tests the code that runs rather than a copy of it.
        let wanted = || -> Vec<String> {
            to_fetch(&four, &pack, &weights, None)
                .iter()
                .map(|f| f.file.clone())
                .collect()
        };

        assert_eq!(
            wanted(),
            vec![pack.file.clone(), proj.file.clone()],
            "a fresh 4B fetches both halves of vision"
        );
        hold(&pack.file, PACK_BYTES);
        assert_eq!(
            wanted(),
            vec![proj.file.clone()],
            "the pack already here is not fetched twice"
        );
        hold(&proj.file, PROJ_BYTES);
        assert!(
            wanted().is_empty(),
            "nothing left to fetch, so nothing is fetched"
        );
        // A file of the wrong length is not "here" by the app's own probe, so an interrupted
        // transfer is re-fetched rather than announced as a model.
        std::fs::write(weights.join(&proj.file), "torn").unwrap();
        assert_eq!(
            wanted(),
            vec![proj.file.clone()],
            "a torn projector is fetched again, and only it"
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
