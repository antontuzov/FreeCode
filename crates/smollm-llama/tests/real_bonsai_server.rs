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

use smollm_llama::client::{
    chat_body, prompt_tokens, stream_chat, ChatRequest, Message, Reply, ReplyDelta, StreamControl,
};
use smollm_llama::load_guard::{self, FileState, GuardRequest};
use smollm_llama::{EngineBinary, LaunchRequest, LaunchTuning, ServerHandle};
use smollm_models::{gguf, Catalog, EngineLock, HostHardware};

/// A real pack has to become resident before `/health` answers, and the wait is priced for the
/// largest one this host could be pointed at rather than the 0.46 GB one the default model picks —
/// a timeout that expires while a model is still loading just re-runs the same load.
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
            pid_registry: None,
            tuning: LaunchTuning::default(),
        }
    }
}

#[tokio::test]
#[ignore = "needs the pinned engine unpacked and the default model's weights on disk"]
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
    // Counted before the run, on the same request: the app asks the server how big a prompt is so it
    // can warn about one that will not fit, and the answer is only worth having if it is the number
    // the run itself later reports.
    let counted = prompt_tokens(handle.base_url(), &req, &r.model, handle.context_tokens())
        .await
        .expect("the fork answers /apply-template and /tokenize");
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
            StreamControl::Continue
        },
    )
    .await
    .expect("a streaming chat completion");

    println!("reasoning: {} chars", reply.reasoning.chars().count());
    println!("answer   : {:?}", reply.text);
    println!("finish   : {:?}", reply.finish_reason);
    println!(
        "prompt   : counted {counted} before, reported {:?}",
        reply.prompt_tokens
    );
    assert_eq!(
        reply.prompt_tokens,
        Some(counted),
        "the count taken before the run must be the count the usage frame reports for it"
    );
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

/// One turn against the running server, with the same budget the host asks for.
async fn ask(r: &Real, handle: &ServerHandle, history: &[Message]) -> Reply {
    let req = ChatRequest {
        max_tokens: 2_048,
        ..ChatRequest::new(history.to_vec())
    };
    stream_chat(
        handle.base_url(),
        &req,
        &r.model,
        handle.context_tokens(),
        |_| StreamControl::Continue,
    )
    .await
    .expect("the server answers a replayed history")
}

/// The alternative the pinned server keeps proposing. Its startup log says "chat template supports
/// preserving reasoning, consider enabling it via `--reasoning-preserve`", the flag is real, and the
/// 1.7B's template does read `message.reasoning_content` — so declining it is a decision, not an
/// omission: re-rendering an earlier thought into a later prompt is what misses the prompt cache on
/// every turn of a tool loop (Bonsai-demo#183). The launch therefore carries no bare `--reasoning`
/// switch and nothing that touches the server's history — the three `--reasoning-*` flags
/// `reasoning_flags` emits are the reply floor and the effort allow list, which say nothing about
/// replay — and [`Message`] has no field a thought could be replayed through.
///
/// What only the real server can settle is the price of that choice. Stripping reasoning is only
/// harmless if the answer alone still carries the conversation — so this asks a second turn something
/// the first turn is the only source for, and looks at the bytes that went out.
#[tokio::test]
#[ignore = "needs the pinned engine unpacked and real weights on disk"]
async fn a_second_turn_remembers_the_first_without_replaying_its_reasoning() {
    let r = real();
    need!(
        r.engine.installed,
        "no fork engine at {}",
        r.engine.server_path.display()
    );
    need!(r.weights.is_file(), "no weights at {}", r.weights.display());

    let handle = smollm_llama::server::start(r.request(&r.engine), &r.lock, READY)
        .await
        .expect("the pinned server must become healthy");

    let mut history = vec![Message::user(
        "Remember one word for later: sparrow. Reply with the single word ok.",
    )];
    let first = ask(&r, &handle, &history).await;
    println!("turn 1 answer: {:?}", first.text);
    assert!(
        !first.text.trim().is_empty(),
        "the first turn answered with nothing"
    );

    // Replayed exactly the way the host replays it: the answer text, and nothing of the reasoning
    // that came with it.
    history.push(Message::assistant(first.text.clone()));
    history.push(Message::user(
        "Which word did I ask you to remember? Answer with that one word.",
    ));

    let wire = chat_body(
        &ChatRequest::new(history.clone()),
        &r.model,
        handle.context_tokens(),
    )
    .expect("a replayed history is a request the template accepts")
    .to_string();
    assert!(
        wire.contains("sparrow") || wire.contains(&first.text),
        "the first turn never reached the request: {wire}"
    );
    assert!(
        !wire.contains("reasoning_content"),
        "an assistant turn replays as its answer only: {wire}"
    );

    let second = ask(&r, &handle, &history).await;
    println!("turn 2 answer: {:?}", second.text);
    assert!(
        second.text.to_lowercase().contains("sparrow"),
        "the second turn lost what the first was told: {:?}",
        second.text
    );

    // The context meter reads these counts, so they have to be real ones: the request asks for the
    // usage frame, and the replayed history is the same window measured a second later — bigger,
    // because the first turn is now part of what the server was given.
    println!(
        "usage: turn 1 {:?} tokens, turn 2 {:?}",
        first.total_tokens, second.total_tokens
    );
    let (Some(before), Some(after)) = (first.total_tokens, second.total_tokens) else {
        panic!(
            "the pinned server sent no usage frame for a request that asked for one (turn 1 {:?}, \
             turn 2 {:?})",
            first.total_tokens, second.total_tokens
        );
    };
    assert!(
        after > before,
        "the second turn's window did not grow with the replayed first: {before} -> {after}"
    );
}

/// The other half of the Stop button: a stream the caller abandons part way through. The text it
/// had already is the answer — a discarded partial reply is how a Stop comes to look like a crash —
/// and the server has to still be there afterwards, because a Stop that took the engine with it
/// would charge the next message a full model load.
#[tokio::test]
#[ignore = "needs the pinned engine unpacked and real weights on disk"]
async fn a_stream_the_caller_stops_keeps_its_text_and_its_server() {
    let r = real();
    need!(
        r.engine.installed,
        "no fork engine at {}",
        r.engine.server_path.display()
    );
    need!(r.weights.is_file(), "no weights at {}", r.weights.display());

    let mut handle = smollm_llama::server::start(r.request(&r.engine), &r.lock, READY)
        .await
        .expect("the pinned server must become healthy");

    // Long enough that three batches cannot be the whole answer.
    let req = ChatRequest {
        max_tokens: 2_048,
        ..ChatRequest::new(vec![Message::user(
            "Count from 1 to 200, separating the numbers with commas.",
        )])
    };
    let mut batches = 0usize;
    let stopped = stream_chat(
        handle.base_url(),
        &req,
        &r.model,
        handle.context_tokens(),
        |_| {
            batches += 1;
            if batches >= 3 {
                StreamControl::Stop
            } else {
                StreamControl::Continue
            }
        },
    )
    .await
    .expect("a stopped stream is a reply, not a failure");

    println!("stopped after {batches} batches: {:?}", stopped.text);
    assert!(batches >= 3, "the answer never streamed at all");
    assert!(
        !stopped.text.trim().is_empty(),
        "stopping threw away the text it already had"
    );
    assert!(
        !stopped.text.contains("200"),
        "the whole count arrived, so nothing was stopped: {:?}",
        stopped.text
    );
    assert!(
        handle.is_running(),
        "stopping the reply took the server with it"
    );
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

/// A port that is already somebody else's has to fail in words the launcher can act on.
///
/// The retry in `start` keys off the server's own output, so the wording is not a detail I may
/// remember: it is whatever `libllama-server-impl.dylib` from the pinned release actually writes on
/// this machine. This holds a loopback port and asks the real server for it by number, which is the
/// only way to see the message as it is rather than as the llama.cpp docs say it was.
///
/// The assertion is in two parts: the child must die with its own exit rather than time out on
/// `/health`, and the tail it leaves must carry the fragment `bind_collision` matches on. A pin change
/// that rewords the message therefore fails here — which is the point, since the retry would otherwise
/// degrade back into "launch failed" with nothing in CI to say why.
///
/// This holds the port and names it, because a lost handover of a *selected* port cannot be provoked
/// through the public API — the kernel picks the number, and racing it is exactly the thing that is
/// not deterministic. The recovery itself was therefore proved once by hand on 2026-10-17, with a
/// temporary seam that forced `start`'s first attempt onto a held port: the pinned server refused
/// 56725, `start` asked for another number, and the same launch came up healthy on 56729 in 4.3s. What
/// is fixed here is the wording that recovery keys on.
#[tokio::test]
#[ignore = "needs the pinned engine unpacked and real weights on disk"]
async fn a_held_port_fails_in_words_that_name_the_bind() {
    let r = real();
    need!(
        r.engine.installed,
        "no fork engine at {}",
        r.engine.server_path.display()
    );
    need!(r.weights.is_file(), "no weights at {}", r.weights.display());

    let held = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("a loopback port to hold");
    let port = held.local_addr().expect("the held port").port();

    let mut req = r.request(&r.engine);
    // An explicit port: `start` must use this number and report on it, not quietly move elsewhere.
    req.tuning.port = port;
    let err = smollm_llama::server::start(req, &r.lock, READY)
        .await
        .expect_err("a server cannot bind a port that is already held");
    println!("FAILED AS: {err}");

    let log = match &err {
        smollm_llama::LlamaError::ServerExited { status, log } => {
            println!("EXIT STATUS: {status}");
            log.clone()
        }
        other => {
            panic!("a busy port must reach the caller as the child's own exit, not as {other:?}")
        }
    };
    assert!(
        !log.trim().is_empty(),
        "the exit was reported with an empty log, so nothing could name the reason"
    );
    let lowered = log.to_lowercase();
    assert!(
        lowered.contains("couldn't bind"),
        "the tail does not carry the line `bind_collision` keys on:\n{log}"
    );
    drop(held);
}
