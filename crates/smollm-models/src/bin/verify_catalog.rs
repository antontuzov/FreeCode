//! `verify-hf-gguf-catalog-entry`
//!
//! CI job that fails the build when a catalog entry could be served by stock llama.cpp. It checks:
//!   1. every ternary / PTQ1_0 / PQ2_0 (and rotated-basis F16) entry declares `needs_fork = true`
//!      and has a resolvable fork asset for each backend it claims to prefer;
//!   2. the pinned engine in `engines/lock.toml` is at or above the catalog's `min_release_tag`;
//!   3. the resolution path stays inside `engines/` — a system `llama-server` on `PATH` is never an
//!      acceptable answer, so the guard cannot be satisfied by an upstream binary;
//!   4. the numbers we ship are still what Hugging Face publishes (`--remote`), and any pack already
//!      on disk hashes to the recorded sha256 and matches the basis the entry declares
//!      (`--model-dir`).
//!
//! Usage:
//!   cargo run -p smollm-models --bin verify-hf-gguf-catalog-entry -- [root] [--remote] [--model-dir PATH]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use smollm_models::{Backend, Catalog, EngineLock, ModelEntry};

/// The HF tree API for one catalog entry. Built from the entry's own repo, because two entries can
/// come from two different repos and checking a 1.7B pack against the 27B's listing would report
/// every one of its files as unpublished.
fn tree_url(model: &ModelEntry) -> String {
    format!(
        "https://huggingface.co/api/models/{}/tree/main?recursive=true",
        model.repo_id
    )
}

#[derive(Debug, Default)]
struct Args {
    root: PathBuf,
    remote: bool,
    model_dir: Option<PathBuf>,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut a = Args {
        root: PathBuf::from("."),
        ..Default::default()
    };
    let mut iter = argv.into_iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--remote" => a.remote = true,
            "--model-dir" => {
                let v = iter.next().ok_or("--model-dir needs a path")?;
                a.model_dir = Some(PathBuf::from(v));
            }
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            other => a.root = PathBuf::from(other),
        }
    }
    if !a.root.join("catalog/models.toml").is_file() {
        return Err(format!(
            "no catalog/models.toml under {} — run it from the repo root",
            a.root.display()
        ));
    }
    Ok(a)
}

fn report(failures: &mut Vec<String>, ok: bool, label: &str, detail: String) {
    if ok {
        println!("  PASS  {label}");
    } else {
        println!("  FAIL  {label}: {detail}");
        failures.push(format!("{label}: {detail}"));
    }
}

fn check_entry(
    model: &ModelEntry,
    lock: &EngineLock,
    targets: &[smollm_models::AvailableTarget],
    failures: &mut Vec<String>,
) {
    let violations = smollm_models::validate::validate_catalog(
        &Catalog {
            schema_version: 1,
            updated: model.id.clone(),
            defaults: Default::default(),
            models: vec![model.clone()],
        },
        targets,
    );
    report(
        failures,
        violations.is_empty(),
        &format!("{} catalog invariants", model.id),
        violations
            .iter()
            .map(|v| format!("[{}] {}/{}", v.rule, v.subject, v.detail))
            .collect::<Vec<_>>()
            .join(" | "),
    );

    report(
        failures,
        !model.packs.is_empty()
            && model
                .packs
                .iter()
                .any(|p| p.is_ternary_quant() && model.engine.needs_fork),
        &format!("{} ternary packs are needs_fork", model.id),
        "a PTQ1_0/PQ2_0 pack exists and engine.needs_fork is true".into(),
    );

    match lock.satisfies(model) {
        Ok(()) => report(
            failures,
            true,
            &format!("{} pinned engine satisfies {}", model.id, lock.pinned_tag),
            String::new(),
        ),
        Err(detail) => report(
            failures,
            false,
            &format!("{} pinned engine", model.id),
            detail,
        ),
    }

    // The engine must resolve under engines/, never from PATH.
    let engines_only = lock.targets.iter().all(|t| {
        !t.asset.contains('/') && lock.download_url_template.contains("PrismML-Eng/llama.cpp")
    });
    report(
        failures,
        engines_only,
        "engine assets resolve from the fork release, not a system binary",
        "an asset path escaped engines/, or the URL template is not the PrismML fork".into(),
    );

    // Every platform we advertise a build for must resolve to exactly one asset.
    for (os, arch) in [
        ("darwin", "aarch64"),
        ("windows", "x86_64"),
        ("linux", "x86_64"),
    ] {
        for backend in [Backend::Metal, Backend::Cuda, Backend::Vulkan, Backend::Cpu] {
            let has = lock.target_for(os, arch, backend).is_some();
            let expected = matches!(
                (os, backend),
                ("darwin", Backend::Metal)
                    | ("darwin", Backend::Cpu)
                    | ("windows", Backend::Cpu)
                    | ("windows", Backend::Cuda)
                    | ("windows", Backend::Vulkan)
                    | ("linux", Backend::Cpu)
            );
            if has != expected && expected {
                report(
                    failures,
                    false,
                    &format!("engine target {os}/{arch}/{backend:?}"),
                    "the app can select this backend but no asset is pinned for it".into(),
                );
            }
        }
    }
}

async fn check_remote(model: &ModelEntry) -> Vec<String> {
    let mut out = Vec::new();
    let client = reqwest::Client::builder()
        .user_agent("smollm-studio-verify")
        .build()
        .unwrap_or_default();
    let resp = match client.get(tree_url(model)).send().await {
        Ok(r) => r,
        Err(e) => return vec![format!("could not reach the HF tree API: {e}")],
    };
    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return vec![format!("HF tree API returned non-JSON: {e}")],
    };
    let entries = json.as_array().cloned().unwrap_or_default();
    for pack in model
        .packs
        .iter()
        .map(|p| (&p.file, p.size_bytes, p.sha256.as_str()))
    {
        match entries
            .iter()
            .find(|e| e["path"].as_str() == Some(pack.0.as_str()))
        {
            None => out.push(format!("{} is no longer published upstream", pack.0)),
            Some(e) => {
                let size = e["size"].as_u64().unwrap_or(0);
                if size != pack.1 {
                    out.push(format!(
                        "{} upstream size {size} != catalog {}",
                        pack.0, pack.1
                    ));
                }
                let oid = e["lfs"]["oid"].as_str().unwrap_or_default();
                if !oid.is_empty() && !oid.eq_ignore_ascii_case(pack.2) {
                    out.push(format!(
                        "{} upstream sha256 {oid} != catalog {}",
                        pack.0, pack.2
                    ));
                }
            }
        }
    }
    for v in model.vision.iter() {
        if !entries
            .iter()
            .any(|e| e["path"].as_str() == Some(v.file.as_str()))
        {
            out.push(format!("{} is no longer published upstream", v.file));
        }
    }
    out
}

fn check_local_files(model: &ModelEntry, dir: &Path, failures: &mut Vec<String>) {
    for pack in model.packs.iter().filter(|p| p.is_runtime_pack()) {
        let path = dir.join(&pack.file);
        if !path.is_file() {
            continue;
        }
        match smollm_models::gguf::verify(&path, pack.size_bytes, &pack.sha256) {
            Ok(status) => report(
                failures,
                status.is_ok(),
                &format!("{} on disk", pack.key),
                status.describe(),
            ),
            Err(e) => report(
                failures,
                false,
                &format!("{} on disk", pack.key),
                e.to_string(),
            ),
        }
        // The rotation metadata is what separates a real Bonsai file from a ternary-quantized
        // arbitrary model, which loads fine and outputs nonsense. It only exists for an export whose
        // weights were folded through a rotation basis: the unrotated ternary generation has no
        // `prism.*` key at all, and demanding one there would reject the genuine file. Its
        // provenance is instead the quant type id, which only the fork's loader knows.
        match smollm_models::gguf::read_header(&path) {
            Ok(header) => {
                let arch_ok = header
                    .architecture
                    .as_deref()
                    .map(|a| a == model.architecture)
                    .unwrap_or(false);
                if model.weight_format.is_rotated() {
                    report(
                        failures,
                        header.has_rotation_metadata(),
                        &format!("{} carries prism.hadamard.* metadata", pack.key),
                        format!("rotation keys: {:?}", header.rotation_keys),
                    );
                } else {
                    report(
                        failures,
                        header.rotation_keys.is_empty(),
                        &format!("{} is in the plain basis, as declared", pack.key),
                        format!(
                            "the entry declares rotation {:?} but the file carries {}",
                            model.weight_format.rotation,
                            header.rotation_keys.len()
                        ),
                    );
                }
                if !arch_ok {
                    println!(
                        "  WARN  {} declares architecture {:?}, catalog says {}",
                        pack.key, header.architecture, model.architecture
                    );
                }
                if let Some(file_ctx) = header.context_length {
                    report(
                        failures,
                        file_ctx == model.context_length,
                        &format!("{} context length matches the entry", pack.key),
                        format!(
                            "the file says {file_ctx}, the catalog says {}",
                            model.context_length
                        ),
                    );
                }
            }
            Err(e) => report(
                failures,
                false,
                &format!("{} header", pack.key),
                e.to_string(),
            ),
        }
    }
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().collect()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("verify-hf-gguf-catalog-entry: {e}");
            return ExitCode::from(2);
        }
    };

    let catalog = match Catalog::load_default(&args.root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("verify-hf-gguf-catalog-entry: {e}");
            return ExitCode::from(2);
        }
    };
    let lock = match EngineLock::load_default(&args.root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("verify-hf-gguf-catalog-entry: {e}");
            return ExitCode::from(2);
        }
    };

    let targets = lock.available_targets();
    let mut failures = Vec::new();
    println!(
        "verifying {} catalog entry(ies) against engine {} ({} target assets)",
        catalog.models.len(),
        lock.pinned_tag,
        targets.len()
    );
    for model in &catalog.models {
        check_entry(model, &lock, &targets, &mut failures);
        if let Some(dir) = &args.model_dir {
            check_local_files(model, dir, &mut failures);
        }
    }

    if args.remote {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("verify-hf-gguf-catalog-entry: cannot start the async runtime: {e}");
                return ExitCode::from(2);
            }
        };
        for model in &catalog.models {
            let drift = rt.block_on(check_remote(model));
            report(
                &mut failures,
                drift.is_empty(),
                &format!("{} matches upstream", model.id),
                drift.join(" | "),
            );
        }
    }

    if failures.is_empty() {
        println!("verify-hf-gguf-catalog-entry: all checks passed");
        return ExitCode::SUCCESS;
    }
    eprintln!(
        "verify-hf-gguf-catalog-entry: {} check(s) failed",
        failures.len()
    );
    ExitCode::FAILURE
}
