//! The `llama-server` command line: what the pinned fork must be started with for these weights.
//!
//! Every flag here is transcribed from `PrismML-Eng/Bonsai-demo`, not chosen. The authority is
//! `scripts/start_llama_server.sh` — the *server* launcher — whose 27B branch runs
//! ```text
//! exec "$BIN" -m "$MODEL" --host "$HOST" --port "$PORT" -ngl "$NGL" -fa on -c "$_ctx" \
//!     $SAMPLING --jinja ${MMPROJ:+--mmproj "$MMPROJ"} ... "$@"
//! ```
//! with `SAMPLING="--temp 1.0 --top-p 0.95 --top-k 20 --min-p 0.05"` for `BONSAI_FAMILY=bonsai2`.
//! The `-ngl` and `-c` values come from `scripts/common.sh` (`bonsai_llama_ngl`,
//! `bonsai_ctx_default`). Where the catalog and the demo disagree the demo wins, which is why
//! `--log-disable` is not emitted: `run_llama.sh` uses it for the one-shot CLI, but the server
//! launcher deliberately keeps logging on, and a supervisor that cannot read stderr cannot explain a
//! load failure.
//!
//! Assembly is pure — no process is spawned — so CI can assert the exact argument vector for any
//! host/backend/packing combination without an engine or weights present.

use std::path::{Path, PathBuf};

use smollm_models::catalog::{CatalogDefaults, ModelEntry, ProfileValues};
use smollm_models::hardware::{Backend, HostHardware};

use crate::engine::EngineBinary;
use crate::LlamaError;

/// `common.sh bonsai_image_max_tokens()`: Metal / Vulkan / CPU cap images at 1024 vision tokens for
/// latency; CUDA and ROCm run uncapped.
const IMAGE_TOKEN_CAP: u32 = 1024;
/// KNOWN_ISSUES.md, "A single conversation keeps re-processing its prompt on the server": for
/// single-user use start the server with `-np 1 --cache-ram 24576`.
const PROMPT_CACHE_MIB: u32 = 24576;
/// llama.cpp's own defaults for the two penalties the GGUF does not carry (KNOWN_ISSUES.md,
/// "`presence_penalty` and `repetition_penalty` are not in the GGUF").
const DEFAULT_PRESENCE_PENALTY: f32 = 0.0;
const DEFAULT_REPETITION_PENALTY: f32 = 1.0;

/// Everything the caller decides that the demo would read from an env var.
///
/// This crate deliberately does not depend on `smollm-config`: `smollm-core` builds this from the
/// user's `LocalModel` and `GenerationBudget` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchTuning {
    /// The demo binds `127.0.0.1` unless `BONSAI_HOST` says otherwise; the app has no LAN mode.
    pub host: String,
    pub port: u16,
    /// `Some` is the user's explicit context size (`BONSAI_CTX`); `None` derives it from RAM.
    pub context_tokens: Option<u32>,
    /// `BONSAI_NGL` equivalent: `Some(0)` forces CPU decode even on a GPU build.
    pub layers_override: Option<u32>,
    pub flash_attention: bool,
    /// One slot plus a prompt cache big enough for a long tool loop (Bonsai-demo #183).
    pub dedicated_server_slots: bool,
    /// Thinking-mode sampling (temp 1.0 / top-p 0.95 / top-k 20 / min-p 0.05) or instruct-mode.
    pub thinking: bool,
    /// Vision projector to pair with the model, when one is installed.
    pub vision_projector: Option<PathBuf>,
    /// `BONSAI_MMPROJ_CPU=1` → `--no-mmproj-offload`, freeing ~0.9 GiB of unified memory.
    pub vision_on_cpu: bool,
    /// `BONSAI_KV4=1` equivalent: KV cache type for both K and V.
    pub kv_cache: Option<String>,
    /// Extra server arguments, appended last exactly as the demo appends `"$@"`.
    pub extra_args: Vec<String>,
}

impl Default for LaunchTuning {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 0,
            context_tokens: None,
            layers_override: None,
            flash_attention: true,
            dedicated_server_slots: true,
            thinking: true,
            vision_projector: None,
            vision_on_cpu: false,
            kv_cache: None,
            extra_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchRequest<'a> {
    /// Workspace root holding `engines/` and `models/`.
    pub root: &'a Path,
    pub engine: &'a EngineBinary,
    pub model: &'a ModelEntry,
    pub defaults: &'a CatalogDefaults,
    /// Absolute path of the chosen pack's GGUF.
    pub model_path: PathBuf,
    pub hardware: &'a HostHardware,
    pub tuning: LaunchTuning,
    /// File mirroring the pid of every server this host spawns, so the next instance can reap what a
    /// killed one stranded. See [`crate::server::reap_orphaned_servers`]. `None` keeps the registry
    /// in memory, which is enough for a caller that cannot be killed.
    pub pid_registry: Option<PathBuf>,
}

/// A fully assembled invocation. `args` contains nothing this function guessed: each value is
/// traceable to the demo script or the catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The backend whose kernels these flags were chosen for.
    pub backend: Backend,
    pub layers_offloaded: u32,
    /// `-c`. The streaming client clamps `max_tokens` against it, because llama.cpp rejects a
    /// request whose generation budget does not fit the loaded context.
    pub context_tokens: u32,
    pub base_url: String,
    /// The whole command line, as launched. It reaches the host's `EngineView` and the real-server
    /// test's output; no part of the window renders it, whatever the field used to claim.
    pub display: String,
}

impl LaunchPlan {
    pub fn assemble(req: &LaunchRequest) -> Result<Self, LlamaError> {
        // The load-guard refuses stock llama.cpp in `check()`; the launcher enforces it again,
        // because a plan is the only thing a launch can be built from and there is no bypass.
        req.engine.assert_not_stock_engine(req.root)?;
        if !req.model_path.is_file() {
            return Err(LlamaError::WeightsNotInstalled {
                path: req.model_path.clone(),
            });
        }

        let backend = req.engine.target.backend;
        let ngl = req.tuning.layers_override.unwrap_or_else(|| {
            layers_to_offload(&req.engine.target.os, &req.engine.target.arch, backend)
        });
        let derived = req
            .tuning
            .context_tokens
            .unwrap_or_else(|| context_from_ram(req.hardware.total_ram_bytes, req.model));
        // `bonsai_ctx_default` treats 0 as unset so `-c 0` is never emitted downstream. llama.cpp
        // also rejects a request whose reply does not fit the loaded context, so the tier is floored
        // at the catalog's minimum generation budget and capped at the model's own window.
        let context_tokens = derived
            .max(req.model.agent_constraints.min_generation_budget_tokens)
            .min(req.model.context_length);

        let model_file = req.model_path.display().to_string();
        let port = req.tuning.port.to_string();
        let ngl_string = ngl.to_string();
        let ctx_string = context_tokens.to_string();
        let mut args: Vec<String> = vec![
            "-m".into(),
            model_file,
            "--host".into(),
            req.tuning.host.clone(),
            "--port".into(),
            port,
            // `llama-server --help` records the default as `--cors-origins ORIGINS … (default: *)`,
            // and the pinned build says so out loud on startup: "CORS is set to allow all origins
            // ('*') and no API key is set / this can be a security risk (cross-origin attacks)".
            // A browser page the user visits could otherwise read replies from this server. The
            // app never uses the browser to reach it — the agent core calls it from Rust, where CORS
            // does not apply — so narrowing this to the documented 'localhost' special value costs
            // nothing and closes the hole. The demo launcher leaves the default because it *is* a
            // browser page served by this very port.
            "--cors-origins".into(),
            "localhost".into(),
            "-ngl".into(),
            ngl_string,
        ];
        if req.tuning.flash_attention {
            // The demo writes the explicit `-fa on`, not bare `-fa`.
            args.extend(["-fa".into(), "on".into()]);
        }
        args.extend(["-c".into(), ctx_string]);
        args.extend(sampling_flags(if req.tuning.thinking {
            &req.model.sampling.thinking
        } else {
            &req.model.sampling.instruct
        }));

        // Native OpenAI-style tool calling. Without it the server cannot return the `tool_calls`
        // field the agent loop depends on; the demo's 27B branch always sets it.
        args.push("--jinja".into());

        if let Some(mmproj) = &req.tuning.vision_projector {
            args.extend(["--mmproj".into(), mmproj.display().to_string()]);
            if req.tuning.vision_on_cpu {
                args.push("--no-mmproj-offload".into());
            }
            if !matches!(backend, Backend::Cuda | Backend::Rocm | Backend::Hip) {
                args.extend(["--image-max-tokens".into(), IMAGE_TOKEN_CAP.to_string()]);
            }
        }

        if let Some(kind) = &req.tuning.kv_cache {
            let constraints = &req.model.agent_constraints;
            // KNOWN_ISSUES.md: `q5_0` KV is several times slower at every depth (llama.cpp#191), and
            // the catalog names the three types worth using.
            if constraints.kv_cache_forbidden_types.contains(kind)
                || !constraints.kv_cache_types.iter().any(|k| k == kind)
            {
                return Err(LlamaError::KvCacheRejected {
                    kind: kind.clone(),
                    allowed: constraints.kv_cache_types.clone(),
                });
            }
            args.extend([
                "--cache-type-k".into(),
                kind.clone(),
                "--cache-type-v".into(),
                kind.clone(),
            ]);
        }

        if req.tuning.dedicated_server_slots {
            args.extend([
                "-np".into(),
                "1".into(),
                "--cache-ram".into(),
                PROMPT_CACHE_MIB.to_string(),
            ]);
        }

        for flag in &req.defaults.server_flags_extra {
            args.push(flag.clone());
        }
        for flag in &req.tuning.extra_args {
            args.push(flag.clone());
        }

        Ok(Self {
            program: req.engine.server_path.clone(),
            backend,
            layers_offloaded: ngl,
            context_tokens,
            base_url: format!("http://{}:{}", req.tuning.host, req.tuning.port),
            display: format!("{} {}", req.engine.server_path.display(), args.join(" ")),
            args,
        })
    }

    /// The demo exports `LD_LIBRARY_PATH=<binary dir>:$LD_LIBRARY_PATH` on every launch ("needed on
    /// Linux CUDA, harmless elsewhere") because the assets ship their shared libraries beside the
    /// binary.
    pub fn library_path(&self) -> Option<&Path> {
        self.program.parent()
    }

    /// The value passed for a flag, for assertions and for the status card.
    pub fn arg(&self, flag: &str) -> Option<&str> {
        self.args
            .iter()
            .position(|a| a == flag)
            .and_then(|i| self.args.get(i + 1))
            .map(String::as_str)
    }

    /// Where this plan actually decodes, which is not always what the binary was built for.
    ///
    /// `-ngl 0` leaves every layer on the CPU whatever backend carries the kernels, so naming the
    /// machine's accelerator on a run that switched it off is a claim the command line contradicts.
    pub fn decode_backend(&self) -> Backend {
        if self.layers_offloaded == 0 {
            Backend::Cpu
        } else {
            self.backend
        }
    }
}

/// The demo's four sampling flags plus the two the GGUF cannot carry. The penalties are only passed
/// where they differ from llama.cpp's own defaults, so a thinking-mode command line stays
/// byte-identical to the demo's.
fn sampling_flags(profile: &ProfileValues) -> Vec<String> {
    let mut flags = vec![
        "--temp".into(),
        decimal(profile.temperature),
        "--top-p".into(),
        decimal(profile.top_p),
        "--top-k".into(),
        profile.top_k.to_string(),
        "--min-p".into(),
        decimal(profile.min_p),
    ];
    if profile.presence_penalty != DEFAULT_PRESENCE_PENALTY {
        flags.extend([
            "--presence-penalty".into(),
            decimal(profile.presence_penalty),
        ]);
    }
    if profile.repetition_penalty != DEFAULT_REPETITION_PENALTY {
        flags.extend([
            "--repetition-penalty".into(),
            decimal(profile.repetition_penalty),
        ]);
    }
    flags
}

/// `common.sh bonsai_llama_ngl()`: 99 for CUDA / ROCm-HIP / Vulkan / Apple Silicon Metal, 0 for an
/// Intel Mac ("no Metal") and for CPU-only hosts.
pub fn layers_to_offload(os: &str, arch: &str, backend: Backend) -> u32 {
    if os == "darwin" && arch == "x86_64" {
        return 0;
    }
    backend.layers_to_offload()
}

/// `common.sh bonsai_ctx_default()`, the RAM tiers in GiB: ≤11→8192, ≤23→16384, ≤35→32768,
/// ≤71→65536, above that 131072 for a model documented past 64k and 65536 for the rest ("older
/// sizes are documented up to 65536").
pub fn context_from_ram(total_ram_bytes: u64, model: &ModelEntry) -> u32 {
    let gib = total_ram_bytes / 1_073_741_824;
    if gib <= 11 {
        8_192
    } else if gib <= 23 {
        16_384
    } else if gib <= 35 {
        32_768
    } else if gib <= 71 {
        65_536
    } else if model.context_length > 65_536 {
        131_072
    } else {
        65_536
    }
}

/// llama.cpp parses these as floats, and Rust's `Display` for `1.0` is `"1"`. The demo writes
/// `1.0`; matching it keeps a captured command line comparable with the script's own output.
fn decimal(v: f32) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smollm_models::catalog::Pack;
    use smollm_models::lock::EngineLock;
    use smollm_models::Catalog;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Owns every value a `LaunchRequest` borrows, so a test can assemble against it.
    struct Fixture {
        tmp: tempfile::TempDir,
        engine: EngineBinary,
        model: ModelEntry,
        defaults: CatalogDefaults,
        model_path: PathBuf,
        hardware: HostHardware,
    }

    impl Fixture {
        fn request(&self, tuning: LaunchTuning) -> LaunchRequest<'_> {
            LaunchRequest {
                root: self.tmp.path(),
                engine: &self.engine,
                model: &self.model,
                defaults: &self.defaults,
                model_path: self.model_path.clone(),
                hardware: &self.hardware,
                pid_registry: None,
                tuning,
            }
        }

        fn plan(&self, tuning: LaunchTuning) -> LaunchPlan {
            LaunchPlan::assemble(&self.request(tuning)).expect("assembly must succeed")
        }
    }

    /// A fork engine laid down in a tempdir under the real pinned lock, with a fake weights file at
    /// the path the resolver would use.
    fn fixture(backend: Backend, ram_gib: u64) -> Fixture {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let catalog = Catalog::load_default(&repo_root()).expect("catalog must load");
        let lock = EngineLock::load_default(&repo_root()).expect("engine lock must parse");
        let model = catalog.models.first().expect("one seeded model").clone();
        let target = lock
            .targets
            .iter()
            .find(|t| t.backend == backend)
            .cloned()
            .unwrap_or_else(|| panic!("no pinned target for {}", backend.label()));
        let dir = lock.install_dir(&crate::engines_root(root), &target);
        std::fs::create_dir_all(&dir).expect("engine dir");
        std::fs::write(dir.join(lock.server_binary()), b"the fork build").expect("server file");
        let engine = EngineBinary::from_target(root, &lock, &target).expect("resolve");
        assert!(engine.installed, "{:?}", engine.server_path);

        let pack = model
            .packs
            .iter()
            .find(|p: &&Pack| p.key == "PTQ1_0")
            .cloned()
            .expect("PTQ1_0 seeded");
        let models = crate::models_root(root);
        std::fs::create_dir_all(&models).expect("models dir");
        let model_path = models.join(&pack.file);
        std::fs::write(&model_path, b"gguf").expect("weights file");

        let hardware = HostHardware {
            total_ram_bytes: ram_gib * 1_073_741_824,
            ..Default::default()
        };
        Fixture {
            tmp,
            engine,
            model,
            defaults: catalog.defaults,
            model_path,
            hardware,
        }
    }

    #[test]
    fn the_thinking_mode_flags_match_start_llama_server_sh_verbatim() {
        let f = fixture(Backend::Metal, 16);
        let plan = f.plan(LaunchTuning::default());
        let tail = plan.args.join(" ");
        assert!(
            tail.contains("-fa on -c 16384 --temp 1.0 --top-p 0.95 --top-k 20 --min-p 0.05"),
            "{tail}"
        );
        assert!(tail.contains("--jinja"), "{tail}");
        assert_eq!(
            plan.arg("-m"),
            Some(f.model_path.display().to_string().as_str())
        );
        assert_eq!(plan.arg("-ngl"), Some("99"));
        assert_eq!(plan.arg("--host"), Some("127.0.0.1"));
        assert_eq!(plan.backend, Backend::Metal);
        assert_eq!(plan.context_tokens, 16_384);
        assert!(plan
            .display
            .starts_with(&plan.program.display().to_string()));
    }

    /// The pinned build prints this on startup unless asked otherwise:
    /// "CORS is set to allow all origins ('*') and no API key is set / this can be a security risk
    /// (cross-origin attacks)". A page loaded in the user's browser could read whatever the local
    /// model generates, so the plan closes that with the flag the build's own `--help` documents.
    #[test]
    fn the_server_does_not_offer_the_local_model_to_any_browser_origin() {
        let f = fixture(Backend::Metal, 16);
        let plan = f.plan(LaunchTuning::default());
        assert_eq!(plan.arg("--cors-origins"), Some("localhost"));
        assert_eq!(plan.arg("--host"), Some("127.0.0.1"));
    }

    #[test]
    fn the_catalog_penalties_are_passed_when_the_gguf_cannot_carry_them() {
        let f = fixture(Backend::Metal, 16);
        let thinking = f.plan(LaunchTuning::default());
        assert_eq!(thinking.arg("--presence-penalty"), None);
        assert_eq!(thinking.arg("--repetition-penalty"), None);

        let instruct = f.plan(LaunchTuning {
            thinking: false,
            ..Default::default()
        });
        // Model card, instruct mode: presence 1.5, and KNOWN_ISSUES.md records the GGUF does not
        // carry it, so the launcher has to.
        assert_eq!(instruct.arg("--presence-penalty"), Some("1.5"));
        assert_eq!(instruct.arg("--temp"), Some("0.7"));
        assert_eq!(instruct.arg("--min-p"), Some("0.0"));
    }

    #[test]
    fn context_uses_the_demo_ram_tiers_and_never_emits_minus_c_zero() {
        for (gib, ctx) in [
            (8_u64, "8192"),
            (16, "16384"),
            (24, "32768"),
            (48, "65536"),
            (128, "131072"),
        ] {
            let f = fixture(Backend::Metal, gib);
            assert_eq!(
                f.plan(LaunchTuning::default()).arg("-c"),
                Some(ctx),
                "{gib} GiB"
            );
        }
        // Zero RAM is "auto" downstream and still yields a real number.
        let f = fixture(Backend::Metal, 0);
        assert_eq!(f.plan(LaunchTuning::default()).arg("-c"), Some("8192"));
    }

    #[test]
    fn the_context_always_leaves_room_for_the_minimum_generation_budget() {
        let f = fixture(Backend::Metal, 16);
        let small = f.plan(LaunchTuning {
            context_tokens: Some(1024),
            ..Default::default()
        });
        assert_eq!(
            small.context_tokens, f.model.agent_constraints.min_generation_budget_tokens,
            "an explicit -c smaller than the minimum reply budget must not be launched"
        );
        let huge = f.plan(LaunchTuning {
            context_tokens: Some(1_000_000),
            ..Default::default()
        });
        assert_eq!(huge.context_tokens, f.model.context_length);
    }

    #[test]
    fn an_intel_mac_offloads_no_layers_however_its_backend_is_labelled() {
        assert_eq!(layers_to_offload("darwin", "x86_64", Backend::Cpu), 0);
        assert_eq!(layers_to_offload("darwin", "x86_64", Backend::Metal), 0);
        assert_eq!(layers_to_offload("darwin", "aarch64", Backend::Metal), 99);
        assert_eq!(layers_to_offload("linux", "x86_64", Backend::Cuda), 99);
        assert_eq!(layers_to_offload("linux", "x86_64", Backend::Cpu), 0);
    }

    #[test]
    fn a_cpu_build_runs_with_no_offload() {
        let f = fixture(Backend::Cpu, 16);
        let plan = f.plan(LaunchTuning::default());
        assert_eq!(plan.arg("-ngl"), Some("0"));
        assert_eq!(plan.layers_offloaded, 0);
        assert_eq!(plan.backend, Backend::Cpu);
        // BONSAI_NGL's override is the same knob the demo exposes.
        let forced = f.plan(LaunchTuning {
            layers_override: Some(99),
            ..Default::default()
        });
        assert_eq!(forced.arg("-ngl"), Some("99"));
    }

    /// A Metal build told to offload nothing decodes on the CPU, and the run has to be reported that
    /// way: the switch that causes it is a user-facing setting, and a status that keeps saying
    /// "Metal" after it is thrown tells the user the machine is doing work it is not doing.
    #[test]
    fn a_metal_build_told_to_offload_nothing_decodes_on_the_cpu() {
        let f = fixture(Backend::Metal, 16);
        let metal = f.plan(LaunchTuning::default());
        assert_eq!(metal.decode_backend(), Backend::Metal);
        let cpu = f.plan(LaunchTuning {
            layers_override: Some(0),
            ..Default::default()
        });
        assert_eq!(cpu.arg("-ngl"), Some("0"));
        assert_eq!(cpu.backend, Backend::Metal);
        assert_eq!(cpu.decode_backend(), Backend::Cpu);
    }

    #[test]
    fn one_dedicated_slot_with_a_prompt_cache_is_the_documented_single_user_launch() {
        let f = fixture(Backend::Metal, 16);
        let plan = f.plan(LaunchTuning::default());
        assert_eq!(plan.arg("-np"), Some("1"));
        assert_eq!(plan.arg("--cache-ram"), Some("24576"));
        let shared = f.plan(LaunchTuning {
            dedicated_server_slots: false,
            ..Default::default()
        });
        assert_eq!(shared.arg("-np"), None);
        assert_eq!(shared.arg("--cache-ram"), None);
    }

    #[test]
    fn the_vision_projector_is_paired_with_the_documented_image_token_cap() {
        let f = fixture(Backend::Metal, 16);
        let proj = f.tmp.path().join("mmproj.gguf");
        std::fs::write(&proj, b"projector").expect("mmproj");
        let plan = f.plan(LaunchTuning {
            vision_projector: Some(proj.clone()),
            ..Default::default()
        });
        assert_eq!(
            plan.arg("--mmproj"),
            Some(proj.display().to_string().as_str())
        );
        assert_eq!(plan.arg("--image-max-tokens"), Some("1024"));
        let on_cpu = f.plan(LaunchTuning {
            vision_projector: Some(proj.clone()),
            vision_on_cpu: true,
            ..Default::default()
        });
        // A valueless flag has no argument to read back, so it is asserted by presence below, not by
        // `arg()` — which would return the *next* flag's name.
        assert!(
            on_cpu.args.iter().any(|a| a == "--no-mmproj-offload"),
            "{:?}",
            on_cpu.args
        );

        let c = fixture(Backend::Cuda, 32);
        let uncapped = c.plan(LaunchTuning {
            vision_projector: Some(proj),
            ..Default::default()
        });
        assert_eq!(uncapped.arg("--image-max-tokens"), None);
    }

    #[test]
    fn a_forbidden_kv_cache_type_is_refused_with_the_types_the_card_lists() {
        let f = fixture(Backend::Metal, 16);
        for kind in ["q5_0", "q6_0"] {
            let err = LaunchPlan::assemble(&f.request(LaunchTuning {
                kv_cache: Some(kind.into()),
                ..Default::default()
            }))
            .expect_err("only the catalog's three cache types may be launched");
            match err {
                LlamaError::KvCacheRejected { allowed, .. } => {
                    assert_eq!(allowed, vec!["f16", "q8_0", "q4_0"]);
                }
                other => panic!("{other:?}"),
            }
        }
        let plan = f.plan(LaunchTuning {
            kv_cache: Some("q4_0".into()),
            ..Default::default()
        });
        assert_eq!(plan.arg("--cache-type-k"), Some("q4_0"));
        assert_eq!(plan.arg("--cache-type-v"), Some("q4_0"));
    }

    #[test]
    fn no_reasoning_flag_is_emitted_because_auto_is_the_only_safe_default() {
        let f = fixture(Backend::Metal, 16);
        let plan = f.plan(LaunchTuning::default());
        // KNOWN_ISSUES.md: `--reasoning on` overrides a client's `reasoning_effort: "none"`, and
        // `auto` is already llama.cpp's default — so the launcher sends neither.
        //
        // The same assertion is what keeps `--reasoning-preserve` out, which is the one the pinned
        // build proposes on startup. Preserving a trace in the server's history is precisely the
        // re-rendering Bonsai-demo#183 costs a prompt-cache miss per turn, so reasoning stays out of
        // the replayed history and `real_bonsai_server` proves a second turn still has the first.
        assert!(
            !plan.args.iter().any(|a| a.starts_with("--reasoning")),
            "{:?}",
            plan.args
        );
        assert_eq!(
            f.model.sampling.reasoning.server_reasoning_flag, "auto",
            "the catalog's stated default must be the flag-less launch"
        );
    }

    #[test]
    fn logging_stays_on_for_a_supervised_server() {
        let f = fixture(Backend::Metal, 16);
        let plan = f.plan(LaunchTuning::default());
        assert!(
            !plan.args.iter().any(|a| a == "--log-disable"),
            "{:?}",
            plan.args
        );
        assert!(
            f.defaults
                .server_flags_extra
                .iter()
                .all(|f| f != "--log-disable"),
            "the catalog must not re-add a flag that blinds the supervisor: {:?}",
            f.defaults.server_flags_extra
        );
    }

    #[test]
    fn caller_args_come_last_exactly_as_the_demo_appends_positionals() {
        let f = fixture(Backend::Metal, 16);
        let plan = f.plan(LaunchTuning {
            extra_args: vec!["--slots".into(), "--metrics".into()],
            ..Default::default()
        });
        let n = plan.args.len();
        assert_eq!(&plan.args[n - 2..], ["--slots", "--metrics"]);
        assert_eq!(plan.arg("--port"), Some("0"));
        assert_eq!(plan.base_url, "http://127.0.0.1:0");
    }

    #[test]
    fn a_plan_for_a_binary_outside_engines_is_refused_before_any_exec() {
        let mut f = fixture(Backend::Metal, 16);
        f.engine.server_path = PathBuf::from("/opt/homebrew/bin/llama-server");
        let err = LaunchPlan::assemble(&f.request(LaunchTuning::default()))
            .expect_err("a PATH binary is never the fork");
        assert!(
            matches!(err, LlamaError::StockEngineRejected { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn a_plan_needs_weights_that_are_actually_on_disk() {
        let f = fixture(Backend::Metal, 16);
        let missing = f.model_path.parent().unwrap().join("absent.gguf");
        let err = LaunchPlan::assemble(&LaunchRequest {
            model_path: missing,
            ..f.request(LaunchTuning::default())
        })
        .expect_err("no weights, no launch");
        assert!(
            matches!(err, LlamaError::WeightsNotInstalled { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn every_pinned_target_produces_a_launchable_argument_vector() {
        let catalog = Catalog::load_default(&repo_root()).expect("catalog must load");
        let lock = EngineLock::load_default(&repo_root()).expect("engine lock must parse");
        let model = catalog.models.first().expect("one seeded model").clone();
        let tmp = tempfile::tempdir().expect("tempdir");
        let weights = tmp.path().join("Ternary-Bonsai-2-27B-PTQ1_0.gguf");
        std::fs::write(&weights, b"gguf").expect("weights file");
        let hardware = HostHardware::default();
        for target in &lock.targets {
            let engine = EngineBinary::from_target(&repo_root(), &lock, target).expect("resolve");
            let plan = LaunchPlan::assemble(&LaunchRequest {
                root: &repo_root(),
                engine: &engine,
                model: &model,
                defaults: &catalog.defaults,
                model_path: weights.clone(),
                hardware: &hardware,
                pid_registry: None,
                tuning: LaunchTuning::default(),
            })
            .unwrap_or_else(|e| panic!("{}: {e}", target.key));
            for flag in ["-m", "-c", "--host", "--port", "-ngl", "--temp", "--jinja"] {
                assert!(
                    plan.args.iter().any(|a| a == flag),
                    "{}: no {flag} in {:?}",
                    target.key,
                    plan.args
                );
            }
            assert!(
                !plan.args.iter().any(String::is_empty),
                "{}: an empty argument would shift every later flag",
                target.key
            );
            assert!(plan.library_path().is_some(), "{}", target.key);
        }
    }

    #[test]
    fn decimals_keep_the_form_the_demo_prints() {
        assert_eq!(decimal(1.0), "1.0");
        assert_eq!(decimal(0.95), "0.95");
        assert_eq!(decimal(0.0), "0.0");
        assert_eq!(decimal(0.05), "0.05");
        assert_eq!(decimal(1.5), "1.5");
    }
}
