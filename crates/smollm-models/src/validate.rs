//! Catalog assertions, run by the `verify-hf-gguf-catalog-entry` CI job.
//!
//! The rule the job exists to enforce is narrow but existential: a ternary pack served by stock
//! llama.cpp either fails to load (PTQ1_0/PQ2_0 are unknown types) or, worse, loads and produces
//! garbage. That is a silent, expensive failure — a user downloads 7 GB and gets noise. So these
//! checks are data-driven assertions over `catalog/models.toml`, and the build fails if any entry
//! would be served by an upstream binary.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, ModelEntry};
use crate::hardware::Backend;

/// A prebuilt engine asset available for one platform/backend, from `engines/lock.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableTarget {
    /// `darwin` / `windows` / `linux`
    pub os: String,
    /// `aarch64` / `x86_64`
    pub arch: String,
    pub backend: crate::hardware::Backend,
    pub asset: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub rule: String,
    pub subject: String,
    pub detail: String,
}

impl Violation {
    fn new(rule: &str, subject: impl Into<String>, detail: String) -> Self {
        Self {
            rule: rule.into(),
            subject: subject.into(),
            detail,
        }
    }
}

/// Parse the `b<digits>` build number out of a fork release tag like `prism-b10772-e8fd209`.
pub fn build_number(tag: &str) -> Option<u32> {
    tag.split('-').find_map(|part| {
        let digits = part.strip_prefix('b')?;
        (!digits.is_empty() && digits.chars().all(char::is_numeric))
            .then(|| digits.parse().ok())
            .flatten()
    })
}

fn is_hex_sha(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn validate_catalog(catalog: &Catalog, targets: &[AvailableTarget]) -> Vec<Violation> {
    let mut out = Vec::new();

    if catalog.models.is_empty() {
        out.push(Violation::new(
            "catalog-not-empty",
            "catalog",
            "no [[model]] entries parsed".to_string(),
        ));
        return out;
    }

    let mut ids = HashSet::new();
    for model in &catalog.models {
        if !ids.insert(model.id.clone()) {
            out.push(Violation::new(
                "unique-model-id",
                &model.id,
                "duplicate catalog entry id".to_string(),
            ));
        }
        validate_model(model, targets, &mut out);
    }
    out
}

fn validate_model(model: &ModelEntry, targets: &[AvailableTarget], out: &mut Vec<Violation>) {
    let ternary = model.weight_format.is_ternary();

    // 1. Every ternary pack is needs-fork, and needs_fork is stated explicitly rather than implied.
    for pack in &model.packs {
        if pack.is_ternary_quant() && !model.engine.needs_fork {
            out.push(Violation::new(
                "ternary-requires-fork",
                format!("{}:{}", model.id, pack.key),
                "PTQ1_0/PQ2_0 are unknown quant types to stock llama.cpp, but engine.needs_fork is false".into(),
            ));
        }
        if pack.needs_fork_engine(&model.weight_format) && !model.engine.needs_fork {
            out.push(Violation::new(
                "rotated-basis-requires-fork",
                format!("{}:{}", model.id, pack.key),
                "the pack depends on a runtime activation transform, so stock cannot run it".into(),
            ));
        }
        if !model.engine.needs_fork && ternary {
            out.push(Violation::new(
                "ternary-requires-fork",
                &model.id,
                "weight_format.scheme is ternary but engine.needs_fork is false".into(),
            ));
        }
        if !is_hex_sha(&pack.sha256) {
            out.push(Violation::new(
                "pack-sha256",
                format!("{}:{}", model.id, pack.key),
                format!("sha256 must be 64 hex chars, got {:?}", pack.sha256),
            ));
        }
        if pack.size_bytes == 0 {
            out.push(Violation::new(
                "pack-size",
                format!("{}:{}", model.id, pack.key),
                "size_bytes must be non-zero".into(),
            ));
        }
        // The UI shows both, so a typo that desynchronises them is a user-visible lie.
        let derived_gb = pack.size_bytes as f64 / 1e9;
        if (derived_gb - pack.size_gb as f64).abs() > 0.05 {
            out.push(Violation::new(
                "pack-size-vs-gb",
                format!("{}:{}", model.id, pack.key),
                format!(
                    "size_bytes {derived_gb:.3} GB disagrees with size_gb {}",
                    pack.size_gb
                ),
            ));
        }
        if pack.bits_per_weight <= 0.0 || pack.bits_per_weight > 16.0 {
            out.push(Violation::new(
                "pack-bpw-sane",
                format!("{}:{}", model.id, pack.key),
                format!("bits_per_weight {}", pack.bits_per_weight),
            ));
        }
    }

    // 2. Filenames must be unique across packs and vision projectors.
    let mut files = HashSet::new();
    for file in model
        .packs
        .iter()
        .map(|p| p.file.as_str())
        .chain(model.vision.iter().map(|v| v.file.as_str()))
    {
        if !files.insert(file) {
            out.push(Violation::new(
                "unique-pack-file",
                &model.id,
                format!("duplicate file name {file}"),
            ));
        }
    }

    // 3. needs_fork is only meaningful if a fork binary can actually be resolved.
    if model.engine.needs_fork {
        if model.engine.fork_repo.is_empty() {
            out.push(Violation::new(
                "fork-repo",
                &model.id,
                "needs_fork but no fork_repo".into(),
            ));
        }
        match build_number(&model.engine.min_release_tag) {
            None => out.push(Violation::new(
                "fork-min-release-parse",
                &model.id,
                format!(
                    "cannot read a build number from tag {:?}",
                    model.engine.min_release_tag
                ),
            )),
            Some(n) => {
                // b10687 shipped no binaries at all; b10709 is the practical floor for the
                // platforms we distribute.
                if n < 10_709 {
                    out.push(Violation::new(
                        "fork-min-release-floor",
                        &model.id,
                        format!("min_release_tag {n} predates b10709, which is the first release with macOS/Linux/Windows/Android binaries"),
                    ));
                }
            }
        }
        // The rotation-metadata requirement exists for exports whose weights were folded through a
        // Hadamard basis (Bonsai 2): a file quantized by anyone else loads silently and emits
        // nonsense, and metadata is the only proof of provenance. A ternary model stored in an
        // unrotated basis — the earlier Bonsai generation — simply has no `prism.*` keys, so
        // requiring them would reject the real file. Its provenance is instead the quant type id
        // that only the fork's loader knows.
        let rotated = model.weight_format.is_rotated();
        if rotated {
            if model.engine.required_metadata_prefixes.is_empty() {
                out.push(Violation::new(
                    "rotation-metadata-required",
                    &model.id,
                    "no required_metadata_prefixes: a ternary file quantized outside Prism loads silently and outputs garbage (llama.cpp#242)".into(),
                ));
            }
            if model.weight_format.metadata_key_prefix.is_empty() {
                out.push(Violation::new(
                    "rotation-metadata-required",
                    &model.id,
                    "weight_format.metadata_key_prefix is empty".into(),
                ));
            }
        } else if !model.engine.required_metadata_prefixes.is_empty() {
            // Guard and catalog must agree, or the app rejects a pack it should run.
            out.push(Violation::new(
                "rotation-metadata-required",
                &model.id,
                format!(
                    "the format declares rotation {:?}, so required_metadata_prefixes {:?} can never be satisfied by a real export",
                    model.weight_format.rotation, model.engine.required_metadata_prefixes
                ),
            ));
        }

        // 4. Every runtime pack's preferred backend must have a resolvable asset on at least one OS.
        for pack in model.packs.iter().filter(|p| p.is_runtime_pack()) {
            for backend in &pack.preferred_for_backends {
                if !targets
                    .iter()
                    .any(|t| t.backend == *backend && t.recommended)
                {
                    out.push(Violation::new(
                        "fork-target-resolvable",
                        format!("{}:{}", model.id, pack.key),
                        format!("preferred backend {backend:?} has no recommended asset in engines/lock.toml"),
                    ));
                }
            }
        }
        if let Some(backend) = Backend::distributable()
            .into_iter()
            .find(|b| !targets.iter().any(|t| t.backend == *b))
        {
            out.push(Violation::new(
                "fork-target-resolvable",
                &model.id,
                format!("backend {backend:?} is a backend we claim to ship, but engines/lock.toml has no asset for it"),
            ));
        }
    }

    // 5. Reasoning-effort contract: `high` must never be sendable, `low` must be documented as
    // behaving like the default, and the default must be supported. The `low` mapping is a Bonsai 2
    // observation, so it is only required of a model that can actually think.
    let r = &model.sampling.reasoning;
    let thinks = r.supported_efforts.iter().any(|e| e != "none");
    if r.supported_efforts.contains(&"high".to_string()) {
        out.push(Violation::new(
            "effort-high-never",
            &model.id,
            "the chat template returns HTTP 500 for reasoning_effort=high".into(),
        ));
    }
    if !r.supported_efforts.contains(&r.default_effort) {
        out.push(Violation::new(
            "effort-default-supported",
            &model.id,
            format!(
                "default_effort {:?} is not in supported_efforts",
                r.default_effort
            ),
        ));
    }
    if thinks && r.unsupported_efforts.contains(&"low".to_string()) && r.low_behaves_like != "xhigh"
    {
        out.push(Violation::new(
            "effort-low-mapping",
            &model.id,
            format!(
                "low is unsupported and must declare low_behaves_like = xhigh, got {:?}",
                r.low_behaves_like
            ),
        ));
    }
    if !thinks && r.default_effort != "none" {
        out.push(Violation::new(
            "effort-none-only",
            &model.id,
            format!(
                "the template has no thinking branch, so default_effort must be \"none\", got {:?}",
                r.default_effort
            ),
        ));
    }
    if !r.never_send.contains(&"high".to_string()) {
        out.push(Violation::new(
            "effort-high-never",
            &model.id,
            "reasoning.never_send must list \"high\"".into(),
        ));
    }
    if r.server_reasoning_flag != "auto" {
        out.push(Violation::new(
            "reasoning-auto",
            &model.id,
            "--reasoning on overrides a client's reasoning_effort=none, so the server must stay at auto".into(),
        ));
    }

    // 6. Sampling contract: llama.cpp seeds its sampler defaults from the GGUF, so for every value
    // the launcher has an opinion about, the catalog must say whether the file carries it or the
    // flag has to be passed explicitly. A value in neither list is a gap where an engine default
    // would silently decide the answer, and engine defaults differ between builds.
    const SAMPLED: [&str; 6] = [
        "temperature",
        "top_p",
        "top_k",
        "min_p",
        "presence_penalty",
        "repetition_penalty",
    ];
    let carried = |key: &str| {
        r.carried_in_gguf.iter().any(|c| c == key) || r.not_carried_in_gguf.iter().any(|c| c == key)
    };
    for key in SAMPLED {
        if !carried(key) {
            out.push(Violation::new(
                "sampling-carriage",
                &model.id,
                format!("{key} is neither declared in carried_in_gguf nor in not_carried_in_gguf"),
            ));
        }
    }
    // Bonsai 2's GGUF carries no min_p key (KNOWN_ISSUES.md "Sampling and metadata"), so its card
    // value has to be stated by the launcher; a file that does carry min_p speaks for itself.
    if !r.carried_in_gguf.iter().any(|c| c == "min_p") && model.sampling.thinking.min_p <= 0.0 {
        out.push(Violation::new(
            "thinking-min-p",
            &model.id,
            "min_p is absent from the GGUF, so thinking min_p must be stated (Bonsai 2: 0.05)"
                .into(),
        ));
    }

    // 7. Generation budget: a cap small enough to end the run mid-thought yields an empty reply,
    // the single most common report against the thinking models. A model that cannot think has no
    // trace to truncate, so only the thinking case carries the 4k floor.
    let c = &model.agent_constraints;
    let budget_floor = if thinks { 4_096 } else { 256 };
    if c.min_generation_budget_tokens < budget_floor {
        out.push(Violation::new(
            "thinking-budget-floor",
            &model.id,
            format!(
                "min_generation_budget_tokens {} is below the {} token floor",
                c.min_generation_budget_tokens, budget_floor
            ),
        ));
    }
    if c.recommended_generation_budget_tokens > model.context_length {
        out.push(Violation::new(
            "budget-fits-context",
            &model.id,
            format!(
                "recommended generation {} exceeds context_length {}",
                c.recommended_generation_budget_tokens, model.context_length
            ),
        ));
    }
    if c.kv_cache_forbidden_types
        .iter()
        .any(|f| c.kv_cache_types.contains(f))
    {
        out.push(Violation::new(
            "kv-cache-types",
            &model.id,
            "a KV cache type is both allowed and forbidden".into(),
        ));
    }

    // 8. Attribution must survive into the shipped product.
    if model.license.is_empty()
        || model.contact.is_empty()
        || !model
            .citation
            .contains(&model.homepage.replace("https://", ""))
    {
        out.push(Violation::new(
            "attribution",
            &model.id,
            "license, contact and a citation naming the upstream home are all required".into(),
        ));
    }

    // 9. Benchmark sanity: a ternary pack that claims to be a runtime pack must be measured, not
    // guessed. A percentage outside 50..100 means a typo like 8.2 for 82.
    if let Some(b) = &model.benchmarks.bonsai {
        if !(50.0..=100.0).contains(&b.percent_of_fp16) {
            out.push(Violation::new(
                "benchmark-sane",
                &model.id,
                format!(
                    "bonsai percent_of_fp16 {} is implausible",
                    b.percent_of_fp16
                ),
            ));
        }
        if b.avg > model.benchmarks.fp16_avg {
            out.push(Violation::new(
                "benchmark-below-fp16",
                &model.id,
                format!("a quantized pack cannot score above its FP16 reference on the same suite ({} > {})", b.avg, model.benchmarks.fp16_avg),
            ));
        }
    } else if model.packs.iter().any(|p| p.is_runtime_pack()) {
        out.push(Violation::new(
            "benchmark-measured",
            &model.id,
            "the model offers runtime packs but [model.benchmarks] records no score for the ternary pack".into(),
        ));
    }

    // 10. There must be something to auto-pick between.
    if model.downloadable_packs().len() < 2 {
        out.push(Violation::new(
            "two-runtime-packs",
            &model.id,
            "the packing picker needs at least two downloadable packs with per-backend guidance"
                .into(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn catalog() -> Catalog {
        Catalog::from_toml_str(include_str!("../../../catalog/models.toml"))
            .expect("seeded catalog parses")
    }

    fn targets() -> Vec<AvailableTarget> {
        crate::lock::EngineLock::load_default(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        )
        .expect("engine lock loads")
        .available_targets()
    }

    #[test]
    fn the_seeded_catalog_passes_every_rule() {
        let violations = validate_catalog(&catalog(), &targets());
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn dropping_needs_fork_is_caught() {
        let mut c = catalog();
        c.models[0].engine.needs_fork = false;
        let rules: Vec<String> = validate_catalog(&c, &targets())
            .into_iter()
            .map(|v| v.rule)
            .collect();
        assert!(
            rules.contains(&"ternary-requires-fork".to_string()),
            "{rules:?}"
        );
        assert!(
            rules.contains(&"rotated-basis-requires-fork".to_string()),
            "{rules:?}"
        );
    }

    #[test]
    fn a_fork_pin_older_than_the_requirement_is_caught() {
        let mut c = catalog();
        // A catalog demanding a newer fork than the lock pins fails the lock check...
        c.models[0].engine.min_release_tag = "prism-b10999-abcdefg".into();
        let lock = crate::lock::EngineLock::load_default(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        )
        .unwrap();
        let err = lock.satisfies(&c.models[0]).unwrap_err();
        assert!(err.contains("older than the catalog requirement"), "{err}");
        // ...and a floor below the first release that shipped binaries fails the catalog check.
        let mut c = catalog();
        c.models[0].engine.min_release_tag = "prism-b10687-abcdefg".into();
        let rules: Vec<String> = validate_catalog(&c, &targets())
            .into_iter()
            .map(|v| v.rule)
            .collect();
        assert!(
            rules.contains(&"fork-min-release-floor".to_string()),
            "{rules:?}"
        );
    }

    #[test]
    fn a_missing_sha_or_a_desynchronised_size_is_caught() {
        let mut c = catalog();
        c.models[0].packs[0].sha256 = "deadbeef".into();
        c.models[0].packs[1].size_gb = 1.0;
        let rules: Vec<String> = validate_catalog(&c, &targets())
            .into_iter()
            .map(|v| v.rule)
            .collect();
        assert!(rules.contains(&"pack-sha256".to_string()), "{rules:?}");
        assert!(rules.contains(&"pack-size-vs-gb".to_string()), "{rules:?}");
    }

    #[test]
    fn reasoning_high_is_caught_but_its_absence_is_the_pass() {
        let mut c = catalog();
        c.models[0]
            .sampling
            .reasoning
            .supported_efforts
            .push("high".into());
        c.models[0].sampling.reasoning.never_send.clear();
        let rules: Vec<String> = validate_catalog(&c, &targets())
            .into_iter()
            .map(|v| v.rule)
            .collect();
        assert!(
            rules.contains(&"effort-high-never".to_string()),
            "{rules:?}"
        );
    }

    #[test]
    fn a_generation_cap_small_enough_to_cut_off_thinking_is_caught() {
        let mut c = catalog();
        c.models[0].agent_constraints.min_generation_budget_tokens = 256;
        let rules: Vec<String> = validate_catalog(&c, &targets())
            .into_iter()
            .map(|v| v.rule)
            .collect();
        assert!(
            rules.contains(&"thinking-budget-floor".to_string()),
            "{rules:?}"
        );
    }

    #[test]
    fn build_numbers_parse_from_real_tags() {
        assert_eq!(build_number("prism-b10772-e8fd209"), Some(10_772));
        assert_eq!(build_number("prism-b10709-9a9394a"), Some(10_709));
        assert_eq!(build_number("nightly-2026-10-01"), None);
    }
}
