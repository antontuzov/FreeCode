//! Catalog types mirroring `catalog/models.toml`.
//!
//! The catalog is data, not code: adding another ternary/low-bit HF repo must not require a Rust
//! change. Everything the engine launcher and the Model Manager UI need about a pack is recorded
//! here — true bits/weight, on-disk size, sha256, per-backend recommendation, context, license,
//! citation and the needs-fork flag.
//!
//! Field names match the TOML keys exactly (snake_case), so a catalog edit round-trips without
//! rename annotations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hardware::Backend;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub updated: String,
    #[serde(default)]
    pub defaults: CatalogDefaults,
    #[serde(rename = "model")]
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CatalogDefaults {
    #[serde(default)]
    pub server_flags_extra: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    /// Hugging Face repo id, e.g. `prism-ml/Ternary-Bonsai-2-27B-gguf`.
    pub repo_id: String,
    pub base_model: String,
    pub architecture: String,
    pub license: String,
    pub license_url: String,
    pub context_length: u32,
    pub homepage: String,
    pub contact: String,
    pub citation: String,
    pub paper_url: String,
    pub demo_repo: String,
    pub weight_format: WeightFormat,
    pub engine: EngineRequirement,
    pub packs: Vec<Pack>,
    #[serde(default)]
    pub vision: Vec<VisionPack>,
    pub sampling: SamplingContract,
    #[serde(default)]
    pub agent_constraints: AgentConstraints,
    pub benchmarks: Benchmarks,
    pub throughput: Throughput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightFormat {
    /// e.g. `ternary-g128`. Any value starting with `ternary` makes every pack in the model
    /// needs-fork.
    pub scheme: String,
    pub alphabet: Vec<i8>,
    pub scale_group_size: u32,
    pub scale_dtype: String,
    pub effective_bits_per_weight: f32,
    pub rotation: String,
    pub rotation_block: u32,
    pub rotation_applied: String,
    pub coverage: Vec<String>,
    /// GGUF metadata prefix proving the checkpoint carries the folded rotation.
    pub metadata_key_prefix: String,
}

impl WeightFormat {
    pub fn is_ternary(&self) -> bool {
        self.scheme.starts_with("ternary")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineRequirement {
    pub needs_fork: bool,
    pub fork_repo: String,
    pub min_release_tag: String,
    /// Metadata keys that must be present in the GGUF before we will run it. Without
    /// `prism.hadamard.*` a ternary file loads silently and emits nonsense (llama.cpp#242).
    #[serde(default)]
    pub required_metadata_prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pack {
    pub key: String,
    pub file: String,
    pub quant_type: String,
    pub encoding: String,
    pub bits_per_weight: f32,
    pub size_bytes: u64,
    pub size_gb: f32,
    pub sha256: String,
    #[serde(default)]
    pub marketing_ratio_vs_fp16: Option<f32>,
    #[serde(default)]
    pub preferred_for_backends: Vec<Backend>,
    #[serde(default)]
    pub fallback_for_backends: Vec<Backend>,
    pub notes: String,
    #[serde(default)]
    pub caveats: Vec<Caveat>,
}

impl Pack {
    /// The two ternary quant types exist only in the PrismML fork.
    pub fn is_ternary_quant(&self) -> bool {
        matches!(self.quant_type.as_str(), "PTQ1_0" | "PQ2_0")
    }

    /// Rotated-basis packs — including the F16 reference — are unusable on stock llama.cpp:
    /// PTQ1_0/PQ2_0 are rejected as unknown types, and F16 loads but garbles because the activation
    /// transform depends on metadata only the fork applies.
    pub fn needs_fork_engine(&self, wf: &WeightFormat) -> bool {
        self.is_ternary_quant() || wf.is_ternary() || wf.rotation_applied.contains("runtime")
    }

    pub fn is_runtime_pack(&self) -> bool {
        !self.preferred_for_backends.is_empty() || self.is_ternary_quant()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Moderate,
    Severe,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caveat {
    pub condition: String,
    pub symptom: String,
    pub workaround: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionPack {
    pub key: String,
    pub file: String,
    pub quant_type: String,
    pub size_bytes: u64,
    pub size_gb: f32,
    pub sha256: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SamplingContract {
    pub system_prompt: String,
    pub thinking: ProfileValues,
    pub instruct: ProfileValues,
    pub reasoning: ReasoningValues,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileValues {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub presence_penalty: f32,
    pub repetition_penalty: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningValues {
    pub default_effort: String,
    pub supported_efforts: Vec<String>,
    pub unsupported_efforts: Vec<String>,
    pub low_behaves_like: String,
    /// Effort strings that must never reach the wire (the template 500s on `high`).
    pub never_send: Vec<String>,
    pub carried_in_gguf: Vec<String>,
    pub not_carried_in_gguf: Vec<String>,
    pub budget_transport: String,
    /// The server must stay at `--reasoning auto`: `--reasoning on` overrides a client's
    /// `reasoning_effort: "none"`.
    pub server_reasoning_flag: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentConstraints {
    pub one_system_message_first: bool,
    pub empty_tool_arguments_as_object: bool,
    pub do_not_echo_reasoning_into_history: bool,
    pub min_generation_budget_tokens: u32,
    pub recommended_generation_budget_tokens: u32,
    pub recommended_context: u32,
    pub kv_cache_types: Vec<String>,
    pub kv_cache_forbidden_types: Vec<String>,
}

impl Default for AgentConstraints {
    fn default() -> Self {
        Self {
            one_system_message_first: true,
            empty_tool_arguments_as_object: true,
            do_not_echo_reasoning_into_history: true,
            min_generation_budget_tokens: 4_096,
            recommended_generation_budget_tokens: 16_384,
            recommended_context: 65_536,
            kv_cache_types: vec!["f16".into(), "q8_0".into(), "q4_0".into()],
            kv_cache_forbidden_types: vec!["q5_0".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Benchmarks {
    pub fp16_avg: f32,
    pub ud_q4_k_xl: QuantScore,
    pub iq2_xxs: QuantScore,
    pub bonsai: QuantScore,
    pub by_skill: BTreeMap<String, f32>,
    pub intelligence_density: Density,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuantScore {
    pub bits_per_weight: f32,
    pub size_gb: f32,
    pub avg: f32,
    pub percent_of_fp16: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Density {
    pub score_per_gb: f32,
    pub vs_densest_conventional: f32,
    pub vs_fp16: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Throughput {
    pub rtx_5090_tps: u32,
    pub l4_tps: u32,
    pub m5_max_tps: u32,
    pub m5_pro_tps: u32,
    /// M5 Pro draws ~27.5 W on the GPU and ~34.1 W combined, against 300-455 W board power on the
    /// NVIDIA cards. Keys: gpu / cpu_plus_gpu.
    pub m5_pro_power_w: BTreeMap<String, f32>,
    pub nvidia_board_power_w: BTreeMap<String, u32>,
}

impl Catalog {
    pub fn from_toml_str(raw: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(raw)
    }

    pub fn load(path: &std::path::Path) -> Result<Self, crate::ModelsError> {
        let raw =
            std::fs::read_to_string(path).map_err(|source| crate::ModelsError::ReadCatalog {
                path: path.to_path_buf(),
                source,
            })?;
        Self::from_toml_str(&raw).map_err(|source| crate::ModelsError::ParseCatalog {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }
}

impl ModelEntry {
    pub fn pack(&self, key: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.key == key || p.file == key)
    }

    pub fn recommended_vision(&self) -> Option<&VisionPack> {
        self.vision
            .iter()
            .find(|v| v.recommended)
            .or_else(|| self.vision.first())
    }

    /// Packs the Model Manager offers for download. The F16 reference is excluded: at 53.8 GB it is
    /// a benchmark baseline, not a runtime pack on a laptop.
    pub fn downloadable_packs(&self) -> Vec<&Pack> {
        self.packs.iter().filter(|p| p.is_runtime_pack()).collect()
    }

    /// Resolve a download URL on the HF resolve endpoint.
    pub fn url_for(&self, file: &str) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{file}",
            self.repo_id
        )
    }
}
