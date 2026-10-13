//! Hardware-aware packing choice.
//!
//! The two shipped packings are not "small vs big" versions of the same thing; they trade differently
//! by silicon, and two of the combinations fail in specific, documented ways. The rules below encode
//! the model card's measurements plus the `KNOWN_ISSUES.md` workarounds, and every rule cites the
//! line it came from. `catalog/models.toml` carries the same guidance as data (`preferred_for_backends`,
//! `caveats`) so the Model Manager UI can show the reasoning without duplicating it in code.

use serde::{Deserialize, Serialize};

use crate::catalog::{Caveat, ModelEntry, Pack};
use crate::hardware::{Backend, GpuArch, HostHardware};

/// Extra room beyond the weights: KV cache at the chosen context, the compute buffers, and the OS.
/// The card's own note that "the default 32K context doesn't fit on 12 GB GPUs — the demo scripts
/// size context from system RAM, not GPU memory" is why the check is pack + headroom, not pack
/// alone.
pub const DEFAULT_HEADROOM_BYTES: u64 = 2 << 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackReport {
    pub key: String,
    pub file: String,
    pub bits_per_weight: f32,
    pub size_bytes: u64,
    pub size_gb: f32,
    /// Why this pack is or is not the pick, in one user-facing sentence.
    pub reason: String,
    pub selected: bool,
    /// False when the host provably cannot hold it (RAM/disk), or the backend cannot run it.
    pub viable: bool,
    pub warnings: Vec<String>,
    pub caveats: Vec<Caveat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub model_id: String,
    pub selected_key: String,
    pub reason: String,
    pub backend: Backend,
    pub arch: GpuArch,
    pub reports: Vec<PackReport>,
    /// Set when the only way to satisfy the host is a from-source build of the `prism` branch,
    /// because the needed fix is merged but not shipped in any binary release.
    pub requires_source_build: bool,
    pub warnings: Vec<String>,
    /// True when the choice came from a user override rather than detection.
    pub forced: bool,
}

impl Recommendation {
    pub fn selected_pack<'a>(&self, model: &'a ModelEntry) -> Option<&'a Pack> {
        model.pack(&self.selected_key)
    }
}

struct Score {
    value: i32,
    reasons: Vec<String>,
    warnings: Vec<String>,
    viable: bool,
    source_build: bool,
}

/// A pack already sitting in the weights directory beats the hardware preference. The scoring below
/// answers "what should this machine download"; on a machine that already holds a verified pack the
/// honest answer to "what do I run" is the one that is here, not a second download.
const INSTALLED_BONUS: i32 = 50;

fn score_pack(pack: &Pack, host: &HostHardware, headroom: u64, installed: bool) -> Score {
    let mut s = Score {
        value: 0,
        reasons: Vec::new(),
        warnings: Vec::new(),
        viable: true,
        source_build: false,
    };
    let is_pq2 = pack.key == "PQ2_0";
    let is_ptq1 = pack.key == "PTQ1_0";

    // Rule: the fork's PQ2_0 kernels exist for mac/cpu/cuda/rocm/hip only
    // (common.sh pq2_0_ready_backend). Vulkan is excluded because PQ2_0 silently decodes on the CPU
    // at under 2 tok/s in the current binary release (llama.cpp#238).
    if host.backend.pq2_0_kernels_ready() {
        if is_pq2 {
            s.value += 30;
            s.reasons.push(
                "PQ2_0 has kernels for this backend and processes prompts faster everywhere".into(),
            );
        }
        if is_ptq1 {
            s.value += 10;
            s.reasons
                .push("PTQ1_0 runs here and is the smallest pack".into());
        }
    } else if host.backend == Backend::Vulkan {
        if is_ptq1 {
            s.value += 40;
            s.reasons
                .push("Vulkan must use the dense-trit PTQ1_0 pack".into());
        }
        if is_pq2 {
            s.value -= 120;
            s.warnings.push(
                "PQ2_0 on Vulkan silently runs on the CPU at under 2 tok/s (llama.cpp#238): the GPU log is misleading, watch the CPU model buffer size. PTQ1_0 was selected instead."
                    .into(),
            );
            s.source_build = true;
        }
    }

    // Rule: SYCL cannot load either ternary type at all ("unsupport data type", #235, fix still in
    // review) — nothing is viable on it.
    if host.backend == Backend::Sycl && pack.is_ternary_quant() {
        s.viable = false;
        s.value -= 1000;
        s.warnings
            .push("The SYCL build cannot load PQ2_0 or PTQ1_0 (llama.cpp#235). Use the Vulkan build instead.".into());
    }

    // Rule: model card throughput table — PTQ1_0 decodes faster on Ada-generation GPUs and L4,
    // PQ2_0 decodes faster on H100/A100/Blackwell.
    if host.backend == Backend::Cuda {
        match host.arch {
            GpuArch::NvidiaAda | GpuArch::NvidiaTuringOrOlder if is_ptq1 => {
                // Decode speed dominates an agent loop (many short turns), and this is the
                // measurement the card reports for exactly these cards.
                s.value += 40;
                s.reasons.push(
                    "Faster decode than PQ2_0 on Ada-generation GPUs and L4 (model card)".into(),
                );
            }
            GpuArch::NvidiaHopper | GpuArch::NvidiaBlackwell if is_pq2 => {
                s.value += 12;
                s.reasons
                    .push("Faster decode than PTQ1_0 on Hopper/Blackwell (model card)".into());
            }
            _ => {}
        }
    }

    // Rule: Apple Silicon is where PQ2_0 was measured (M5 Max ~47 tok/s, M5 Pro ~28 tok/s).
    if host.backend == Backend::Metal && is_pq2 {
        s.value += 8;
        s.reasons
            .push("PQ2_0 is the pack benchmarked on Apple Silicon".into());
    }

    // Rule: PQ2_0 segfaults while loading on AVX-512 CPUs (Zen 4/5 and some server/virtualised
    // parts), even with all layers offloaded (#245). Merged to the prism branch, not in a binary
    // release, so a prebuilt engine on such a host is exposed to it.
    if is_pq2
        && host.avx512
        && host.prebuilt_engine
        && matches!(
            host.backend,
            Backend::Cpu | Backend::Cuda | Backend::Hip | Backend::Rocm
        )
    {
        s.value -= 200;
        s.warnings.push(
            "PQ2_0 segfaults while loading on AVX-512 CPUs with prebuilt binaries (llama.cpp#245). PTQ1_0 was selected; build the prism branch from source to use PQ2_0 here."
                .into(),
        );
        s.source_build = true;
    }

    // Rule: PQ2_0 output is garbled on Windows HIP with gfx1151 (Strix Halo) (#223); the workarounds
    // are -ngl 0 or Vulkan with PTQ1_0.
    if is_pq2 && host.backend == Backend::Hip && host.arch == GpuArch::AmdGfx1151 {
        s.value -= 300;
        s.warnings.push(
            "PQ2_0 garbles output on gfx1151 HIP (llama.cpp#223): use PTQ1_0, or -ngl 0.".into(),
        );
    }

    // Rule: consumer x86 CPUs without the AVX2/AVX-VNNI kernels (#206, fixed in source) decode
    // slowly on PQ2_0 in prebuilt binaries.
    if is_pq2
        && host.backend == Backend::Cpu
        && host.prebuilt_engine
        && host.arch == GpuArch::None
        && std::env::consts::OS == "linux"
    {
        s.warnings
            .push("Prebuilt CPU builds miss the fast PQ2_0 kernels on consumer x86 (llama.cpp#206); expect low throughput.".into());
    }

    // Rule: memory-tight hosts take the smaller pack. This is the "1-bit / ~9x smaller" case.
    if !host.can_hold(pack.size_bytes, headroom) {
        s.viable = false;
        s.value -= 500;
        s.warnings.push(format!(
            "needs {} GB plus headroom, which this host does not have",
            format_gb(pack.size_bytes)
        ));
    } else if is_ptq1 {
        s.value += 4;
    }

    if !pack.is_runtime_pack() {
        s.value -= 400;
        s.reasons
            .push("reference precision, not offered as a runtime pack".into());
    }

    if installed {
        s.value += INSTALLED_BONUS;
        s.reasons
            .push("already downloaded here, so no second pack is needed".into());
    }

    s
}

fn format_gb(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / (1024f64 * 1024f64 * 1024f64))
}

/// Pick a pack for this machine, or honour a user override. `override_key` is the packing picker's
/// persisted choice (`PackingChoice::Forced`). `installed` lists the weight file names already on
/// disk with the size the catalog claims.
pub fn recommend(
    model: &ModelEntry,
    host: &HostHardware,
    override_key: Option<&str>,
    headroom: u64,
    installed: &[String],
) -> Recommendation {
    let mut scores: Vec<(&Pack, Score)> = Vec::new();
    for pack in &model.packs {
        scores.push((
            pack,
            score_pack(pack, host, headroom, installed.contains(&pack.file)),
        ));
    }

    let mut forced = false;
    let chosen_idx = if let Some(key) = override_key {
        let idx = scores
            .iter()
            .position(|(p, _)| p.key == key || p.file == key);
        if idx.is_some() {
            forced = true;
        }
        idx
    } else {
        None
    }
    .unwrap_or_else(|| {
        // Viable beats unviable; then score; then the smaller pack, so a tie never lands on F16.
        scores
            .iter()
            .enumerate()
            .max_by_key(|(_, (pack, sc))| (sc.viable, sc.value, std::cmp::Reverse(pack.size_bytes)))
            .map(|(i, _)| i)
            .unwrap_or(0)
    });

    let reports: Vec<PackReport> = scores
        .iter()
        .enumerate()
        .map(|(i, (pack, sc))| {
            let mut reasons = sc.reasons.clone();
            if reasons.is_empty() {
                reasons.push(pack.notes.clone());
            }
            PackReport {
                key: pack.key.clone(),
                file: pack.file.clone(),
                bits_per_weight: pack.bits_per_weight,
                size_bytes: pack.size_bytes,
                size_gb: pack.size_gb,
                reason: reasons.join("; "),
                selected: i == chosen_idx,
                viable: sc.viable,
                warnings: sc.warnings.clone(),
                caveats: pack.caveats.clone(),
            }
        })
        .collect();

    // Every documented penalty on this host is worth surfacing, including the ones that steered
    // the choice *away* from a pack the user might otherwise force. A penalty about a pack that is
    // not the one running has to name it, or a 7 GB pick inherits F16's "does not fit".
    let mut warnings: Vec<String> = Vec::new();
    for (i, (pack, sc)) in scores.iter().enumerate() {
        for w in &sc.warnings {
            let text = if i == chosen_idx || w.starts_with(&pack.key) {
                w.clone()
            } else {
                format!("{}: {}", pack.key, w)
            };
            if !warnings.contains(&text) {
                warnings.push(text);
            }
        }
    }
    let requires_source_build = scores.iter().any(|(_, sc)| sc.source_build);
    let chosen = &scores[chosen_idx].0;
    let reason = if forced {
        format!(
            "user override to {}; detection recommended another pack",
            chosen.key
        )
    } else {
        scores[chosen_idx].1.reasons.join("; ")
    };

    if forced && !scores[chosen_idx].1.viable {
        warnings.push(format!(
            "{} was forced but this host may not run it: {}",
            chosen.key,
            scores[chosen_idx].1.warnings.join(" ")
        ));
    }

    Recommendation {
        model_id: model.id.clone(),
        selected_key: chosen.key.clone(),
        reason,
        backend: host.backend,
        arch: host.arch,
        reports,
        requires_source_build,
        warnings,
        forced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn catalog() -> Catalog {
        let raw = include_str!("../../../catalog/models.toml");
        Catalog::from_toml_str(raw).expect("seeded catalog parses")
    }

    fn host(backend: Backend, arch: GpuArch) -> HostHardware {
        HostHardware {
            backend,
            arch,
            gpu_name: None,
            total_ram_bytes: 34_359_738_368, // 32 GiB
            gpu_ram_bytes: Some(34_359_738_368),
            available_disk_bytes: 200 << 30,
            avx512: false,
            degraded_to_cpu: backend == Backend::Cpu,
            prebuilt_engine: true,
        }
    }

    #[test]
    fn catalog_loads_both_packs_with_verified_sizes() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let pq2 = m.pack("PQ2_0").unwrap();
        let ptq1 = m.pack("PTQ1_0").unwrap();
        assert_eq!(pq2.size_bytes, 7_206_168_928);
        assert_eq!(ptq1.size_bytes, 5_946_648_928);
        assert_eq!(m.context_length, 262_144);
        assert!(m.pack("F16").unwrap().needs_fork_engine(&m.weight_format));
        assert_eq!(m.recommended_vision().unwrap().quant_type, "Q8_0");
    }

    #[test]
    fn apple_silicon_gets_the_pack_measured_on_it() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let rec = recommend(
            &m,
            &host(Backend::Metal, GpuArch::AppleSilicon),
            None,
            DEFAULT_HEADROOM_BYTES,
            &[],
        );
        assert_eq!(rec.selected_key, "PQ2_0");
        assert!(!rec.forced);
    }

    #[test]
    fn a_pack_that_is_already_downloaded_beats_the_hardware_preference() {
        // Detection says what to fetch; this machine already holds the 1-bit pack, and launching it
        // beats downloading 1.3 GB more to satisfy a preference list.
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let h = host(Backend::Metal, GpuArch::AppleSilicon);
        let rec = recommend(
            &m,
            &h,
            None,
            DEFAULT_HEADROOM_BYTES,
            &["Ternary-Bonsai-2-27B-PTQ1_0.gguf".into()],
        );
        assert_eq!(rec.selected_key, "PTQ1_0", "{}", rec.reason);
        assert!(rec.reason.contains("already downloaded"), "{}", rec.reason);
        // With nothing on disk the card's ordering still stands.
        assert_eq!(
            recommend(&m, &h, None, DEFAULT_HEADROOM_BYTES, &[]).selected_key,
            "PQ2_0"
        );
    }

    #[test]
    fn a_warning_about_another_pack_names_it() {
        // F16 does not fit a laptop, but the pack that runs does: printing its "no headroom" note
        // under a 5.95 GB pick tells the user the machine is too small when it is not.
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let rec = recommend(
            &m,
            &host(Backend::Metal, GpuArch::AppleSilicon),
            None,
            DEFAULT_HEADROOM_BYTES,
            &["Ternary-Bonsai-2-27B-PTQ1_0.gguf".into()],
        );
        assert_eq!(rec.selected_key, "PTQ1_0");
        assert!(
            rec.warnings
                .iter()
                .any(|w| w.starts_with("F16: needs 50.11 GB")),
            "{:?}",
            rec.warnings
        );
        assert!(
            !rec.warnings.iter().any(|w| w.starts_with("needs")),
            "the chosen pack inherited another pack's warning: {:?}",
            rec.warnings
        );
    }

    #[test]
    fn ada_and_l4_prefer_dense_trits_and_hopper_takes_pq2() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let ada = recommend(
            &m,
            &host(Backend::Cuda, GpuArch::NvidiaAda),
            None,
            DEFAULT_HEADROOM_BYTES,
            &[],
        );
        assert_eq!(ada.selected_key, "PTQ1_0", "{}", ada.reason);
        let hopper = recommend(
            &m,
            &host(Backend::Cuda, GpuArch::NvidiaHopper),
            None,
            DEFAULT_HEADROOM_BYTES,
            &[],
        );
        assert_eq!(hopper.selected_key, "PQ2_0", "{}", hopper.reason);
    }

    #[test]
    fn vulkan_never_gets_pq2() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let rec = recommend(
            &m,
            &host(Backend::Vulkan, GpuArch::IntelArc),
            None,
            DEFAULT_HEADROOM_BYTES,
            &[],
        );
        assert_eq!(rec.selected_key, "PTQ1_0");
        assert!(rec.requires_source_build);
        assert!(rec.warnings.iter().any(|w| w.contains("#238")));
    }

    #[test]
    fn avx512_host_with_prebuilt_engine_avoids_the_load_crash() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let mut h = host(Backend::Cpu, GpuArch::None);
        h.avx512 = true;
        // Even with the crashing pack already downloaded: a documented segfault outranks "it is
        // already here", which is the whole reason the bonus is a score and not a short-circuit.
        let rec = recommend(
            &m,
            &h,
            None,
            DEFAULT_HEADROOM_BYTES,
            &["Ternary-Bonsai-2-27B-PQ2_0.gguf".into()],
        );
        assert_eq!(rec.selected_key, "PTQ1_0");
        assert!(rec.warnings.iter().any(|w| w.contains("#245")));
    }

    #[test]
    fn a_16gb_laptop_still_fits_because_the_ternary_packs_are_under_8gb() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let mut h = host(Backend::Metal, GpuArch::AppleSilicon);
        h.total_ram_bytes = 16 << 30;
        let rec = recommend(&m, &h, None, DEFAULT_HEADROOM_BYTES, &[]);
        let pq2 = rec.reports.iter().find(|r| r.key == "PQ2_0").unwrap();
        assert!(pq2.viable, "PQ2_0 should fit in 16 GiB unified memory");
        let f16 = rec.reports.iter().find(|r| r.key == "F16").unwrap();
        assert!(
            !f16.viable,
            "the 53.8 GB F16 reference must be marked unviable"
        );
    }

    #[test]
    fn user_override_is_honoured_and_labelled() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let rec = recommend(
            &m,
            &host(Backend::Metal, GpuArch::AppleSilicon),
            Some("PTQ1_0"),
            DEFAULT_HEADROOM_BYTES,
            &[],
        );
        assert_eq!(rec.selected_key, "PTQ1_0");
        assert!(rec.forced);
        assert!(rec.reason.contains("user override"));
    }

    #[test]
    fn sycl_hosts_find_no_ternary_pack_viable() {
        let m = catalog().model("ternary-bonsai-2-27b").unwrap().clone();
        let rec = recommend(
            &m,
            &host(Backend::Sycl, GpuArch::IntelArc),
            None,
            DEFAULT_HEADROOM_BYTES,
            &[],
        );
        for r in rec
            .reports
            .iter()
            .filter(|r| r.key == "PQ2_0" || r.key == "PTQ1_0")
        {
            assert!(!r.viable, "{}", r.key);
        }
    }
}
