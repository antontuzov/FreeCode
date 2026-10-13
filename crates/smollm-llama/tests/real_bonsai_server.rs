//! End-to-end proof against the real thing: the weights Hugging Face published, the engine the
//! installer pinned, the flags the demo scripts specify, and tokens out of the model the app ships
//! with by default.
//!
//! Ignored, because it needs both artefacts on disk — the weight pack and an unpacked
//! `llama-server` — and it loads the model for real. Run it with:
//!
//! ```text
//! cargo test -p smollm-llama --test real_bonsai_server -- --ignored --nocapture
//! ```
//!
//! Everything it asserts is observable: the pinned binary's own version banner, the server's health
//! endpoint, the text it generates, and whether the port is released again when the handle drops.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use smollm_llama::client::{stream_chat, ChatRequest, Message, ReplyDelta};
use smollm_llama::load_guard::{self, FileState, GuardRequest};
use smollm_llama::{EngineBinary, LaunchRequest, LaunchTuning};
use smollm_models::{gguf, Catalog, EngineLock, HostHardware};

/// Loading the largest pack the catalog offers (5.9 GB of ternary weights into an M1's unified
/// memory) is not instantaneous, and the server answers `/health` only once the model is resident.
const READY: Duration = Duration::from_secs(420);

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Skip-with-a-message rather than turn the run red: an ignored probe no machine can satisfy must
/// say what is missing.
macro_rules! need {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            println!("SKIP: {}", format_args!($($arg)+));
            return;
        }
    };
}

/// The real assembly: catalog, lock, host probe, resolved engine, chosen pack, weights on disk.
struct Real {
    /// Owned, because a `LaunchRequest` borrows it; `&root()` in a `let` would dangle at the end of
    /// the statement.
    root: PathBuf,
    catalog: Catalog,
    lock: EngineLock,
    host: HostHardware,
    model: smollm_models::ModelEntry,
    pack: smollm_models::Pack,
    weights: PathBuf,
    engine: EngineBinary,
}

fn real() -> Real {
    let root = root();
    let catalog = Catalog::load_default(&root).expect("catalog");
    let lock = EngineLock::load_default(&root).expect("engine lock");
    // The model the settings pick by default, and the packing this host's probe recommends for it:
    // this probe is the proof of the shipping path, so pinning one named entry here would let the
    // app move on while the test kept exercising a model nobody launches.
    let model = catalog
        .model(smollm_config::DEFAULT_MODEL_ID)
        .expect("the default id is in the catalog")
        .clone();
    let host = HostHardware::detect(&smollm_llama::models_root(&root).display().to_string());
    let installed: Vec<String> = model
        .packs
        .iter()
        .map(|p| p.file.clone())
        .filter(|f| smollm_llama::models_root(&root).join(f).is_file())
        .collect();
    let recommendation = smollm_models::recommend(
        &model,
        &host,
        None,
        smollm_models::DEFAULT_HEADROOM_BYTES,
        &installed,
    );
    let pack = recommendation
        .selected_pack(&model)
        .cloned()
        .expect("the recommendation names a catalog packing");
    println!(
        "probing {} / {} — {}",
        model.id, pack.key, recommendation.reason
    );
    let weights = smollm_llama::models_root(&root).join(&pack.file);
    let engine = EngineBinary::resolve(&root, &lock, &host)
        .expect("the pinned fork engine must resolve on this host");
    Real {
        root,
        catalog,
        lock,
        host,
        model,
        pack,
        weights,
        engine,
    }
}

impl Real {
    fn request<'a>(&'a self, engine: &'a EngineBinary) -> LaunchRequest<'a> {
        LaunchRequest {
            root: &self.root,
            engine,
            model: &self.model,
            defaults: &self.catalog.defaults,
            model_path: self.weights.clone(),
            hardware: &self.host,
            tuning: LaunchTuning::default(),
        }
    }
}

#[tokio::test]
#[ignore = "needs the pinned engine unpacked and ~6 GB of real weights on disk"]
async fn the_pinned_fork_server_loads_the_real_weights_and_streams_a_real_answer() {
    let r = real();
    need!(
        r.engine.installed,
        "no fork engine at {}",
        r.engine.server_path.display()
    );
    need!(r.weights.is_file(), "no weights at {}", r.weights.display());

    // 1. Integrity and the load-guard, against the file as it actually is.
    let integrity = gguf::verify(&r.weights, r.pack.size_bytes, &r.pack.sha256).expect("hash");
    need!(
        integrity.is_ok(),
        "weights failed integrity: {}",
        integrity.describe()
    );
    let header = gguf::read_header(&r.weights).expect("header");
    let report = load_guard::check(&GuardRequest {
        root: &r.root,
        lock: &r.lock,
        model: &r.model,
        pack: &r.pack,
        file: &r.weights,
        state: FileState::Verified,
        header: Some(&header),
        engine: Some(&r.engine),
        host: Some(&r.host),
    });
    assert!(
        report.is_allowed(),
        "the guard must allow the real Prism export on the fork engine; blockers: {:?} warnings: {:?}",
        report.blockers,
        report.warnings
    );
    // Proof of provenance depends on the basis the export was written in: a rotated file is only
    // identifiable by its `prism.hadamard.*` keys, while the unrotated ternary generation has no
    // `prism.*` key at all and is identified by its quant type instead.
    if r.model.weight_format.is_rotated() {
        assert!(
            header.has_rotation_metadata(),
            "a rotated export must carry the prism.hadamard.* keys"
        );
    } else {
        assert!(
            header.rotation_keys.is_empty(),
            "the entry declares an unrotated basis, so the file must not carry prism.* keys: {:?}",
            header.rotation_keys
        );
    }
    assert_eq!(
        header.context_length,
        Some(r.model.context_length),
        "the window the launcher caps `-c` at must be the one the file declares"
    );
    println!(
        "guard allowed; caveats surfaced as warnings: {}",
        report.warnings.len()
    );

    // 2. Launch with the flags assembled from the demo scripts for this host.
    let mut handle = smollm_llama::server::start(r.request(&r.engine), &r.lock, READY)
        .await
        .expect("the pinned server must become healthy");

    println!("launched: {}", handle.plan.display);
    println!(
        "backend {} · -ngl {} · -c {}",
        handle.plan.backend.label(),
        handle.plan.layers_offloaded,
        handle.plan.context_tokens
    );
    println!("server log tail:\n{}", handle.log_tail());
    assert_eq!(handle.plan.backend, r.host.backend);
    assert!(
        handle.plan.args.iter().any(|a| a == "--jinja"),
        "tool calling needs --jinja: {:?}",
        handle.plan.args
    );
    assert!(handle.is_running());

    // 3. A real streamed answer. Short prompt, generous budget: the card's most-common report is an
    //    empty reply when the budget runs out mid-thought.
    let req = ChatRequest {
        max_tokens: 4_096,
        ..ChatRequest::new(vec![Message::user(
            "Reply with exactly this sentence and nothing else: the engine is running.",
        )])
    };
    let mut thoughts = 0usize;
    let mut answers = 0usize;
    let reply = stream_chat(
        handle.base_url(),
        &req,
        &r.model,
        handle.context_tokens(),
        |deltas| {
            for d in deltas {
                match d {
                    ReplyDelta::Thought(_) => thoughts += 1,
                    ReplyDelta::Answer(_) => answers += 1,
                    ReplyDelta::ToolCall { .. } => {}
                }
            }
        },
    )
    .await
    .expect("a streaming chat completion");

    println!("reasoning: {} chars", reply.reasoning.chars().count());
    println!("answer   : {:?}", reply.text);
    println!("finish   : {:?}", reply.finish_reason);
    assert!(
        !reply.text.trim().is_empty(),
        "the model answered with nothing"
    );
    assert!(answers > 0, "the answer never streamed");
    let thinks = r
        .model
        .sampling
        .reasoning
        .supported_efforts
        .iter()
        .any(|e| e != "none");
    if thinks {
        assert!(
            thoughts > 0,
            "a thinking model produced no reasoning_content at all"
        );
    } else {
        // A template with no thinking conditional emits the empty `think` block, so a non-empty
        // reasoning trace here would mean the server was started with reasoning forced on — the one
        // flag the catalog says must stay at `auto`.
        assert_eq!(
            thoughts, 0,
            "this model's chat template has no thinking branch"
        );
    }
    assert!(
        !reply.ran_out_of_budget(),
        "4096 tokens were not enough for a one-sentence reply"
    );

    // 4. Kill-on-drop: the handle goes, the port comes back.
    let port: u16 = handle
        .plan
        .arg("--port")
        .expect("the plan always carries --port")
        .parse()
        .expect("a numeric port");
    drop(handle);
    tokio::time::sleep(Duration::from_secs(2)).await;
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap_or_else(|e| {
        panic!("port {port} is still held — the server outlived its handle: {e}")
    });
}

#[tokio::test]
#[ignore = "needs the pinned engine unpacked"]
async fn the_pinned_binary_reports_the_commit_the_lock_records() {
    let r = real();
    need!(r.engine.installed, "no fork engine installed");
    let version = smollm_llama::server::verify_version(&r.engine.server_path, &r.lock)
        .await
        .expect("`--version` must match the pinned commit");
    println!(
        "engine build {:?} commit {:?}",
        version.build, version.commit
    );
    // `verify_version` already refused the binary unless the commit matched, so what is asserted
    // here is that the two independent pin fields in the lock — the tag name and the commit — both
    // agree with what the machine reports. The lock records GitHub's 7-character abbreviation while
    // the banner prints a longer sha, so the commit is compared by prefix, not verbatim.
    let commit = version.commit.expect("the banner carries a commit");
    assert!(
        commit.starts_with(r.lock.pinned_commit.as_str())
            || r.lock.pinned_commit.starts_with(&commit),
        "{commit} does not extend the pin {}",
        r.lock.pinned_commit
    );
    assert_eq!(
        version.build,
        smollm_models::validate::build_number(&r.lock.pinned_tag),
        "the banner build must equal the build encoded in the pinned tag"
    );
}

#[tokio::test]
#[ignore = "needs the pinned engine unpacked"]
async fn a_stock_binary_is_refused_before_the_server_starts() {
    let r = real();
    need!(r.engine.installed, "no fork engine installed");

    // The supervision path must refuse a binary that is not inside `engines/`, before anything is
    // exec'd. Homebrew's llama.cpp is the suspect on a machine that has one; on a clean machine the
    // same assertion runs against the pinned binary copied outside `engines/`. Location is the whole
    // rule, so a build that is the *right* one in the *wrong* place is still refused — which is what
    // a stray `cp` or a `brew install llama.cpp` symlink produces.
    let homebrew = [
        PathBuf::from("/opt/homebrew/bin/llama-server"),
        PathBuf::from("/usr/local/bin/llama-server"),
    ]
    .into_iter()
    .find(|p| p.is_file());

    let stray = match homebrew {
        Some(path) => path,
        None => {
            let dir = std::env::temp_dir().join("smollm-stock-refusal");
            std::fs::create_dir_all(&dir).expect("temp dir");
            let path = dir.join("llama-server");
            std::fs::copy(&r.engine.server_path, &path).expect("copy the pinned binary");
            let mut perms = std::fs::metadata(&path).expect("copied file").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("chmod");
            path
        }
    };

    let mut engine = r.engine.clone();
    engine.server_path = stray.clone();
    let err = smollm_llama::server::start(r.request(&engine), &r.lock, READY)
        .await
        .expect_err("a binary outside engines/ must never be launched");
    assert!(
        matches!(err, smollm_llama::LlamaError::StockEngineRejected { .. }),
        "{err}"
    );
    println!("refused {}", stray.display());
}
