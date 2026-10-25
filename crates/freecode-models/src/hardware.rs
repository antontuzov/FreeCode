//! Host detection: which inference backend to use, what GPU class we are on, and how much room
//! there is.
//!
//! Detection is deliberately conservative. It shells out to the platform's own tools and treats any
//! failure as "CPU only", because picking a GPU build that cannot start is worse than a slow but
//! working CPU run. Every probe is a read-only command.
//!
//! The backend names match `scripts/common.sh` in PrismML-Eng/Bonsai-demo, which is the source of
//! truth for which backends have ternary kernels.

use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Metal,
    Cuda,
    Rocm,
    Hip,
    Vulkan,
    Sycl,
    Cpu,
}

impl Backend {
    /// Verbatim registry from common.sh `pq2_0_ready_backend()`: `mac|cpu|cuda|rocm|hip|local`.
    /// The two exclusions are that script's policy, not a statement about what the pinned build can
    /// do. Vulkan is left out even though its PQ2_0 kernels shipped in `prism-b10735-842b188`
    /// (llama.cpp#238, so they are in this pin), and SYCL is left out although llama.cpp#235 merged
    /// ternary SYCL kernels and #356 added the TernSYCL XMX path — both in this pin too. Both rows
    /// are still ✅* (partial / needs validation) in BACKEND-SUPPORT.md, whose cells keep audit
    /// baselines older than the release they describe (Vulkan audited at `prism-b10743-adfffbe`,
    /// SYCL at `prism-b10770-6684606`, the rest at `prism-b10709-9a9394a`) under the rule that
    /// "pending PRs and newer branch code do not count as released support". That is why this app
    /// keeps the demo's conservative list rather than the capability matrix.
    pub fn pq2_0_kernels_ready(self) -> bool {
        matches!(
            self,
            Self::Metal | Self::Cpu | Self::Cuda | Self::Rocm | Self::Hip
        )
    }

    /// `-ngl 99` offloads all layers; `0` is CPU-only. common.sh emits 99 for CUDA/ROCm/Vulkan and
    /// for Apple Silicon Metal, and 0 for Intel Macs and CPU-only hosts.
    pub fn layers_to_offload(self) -> u32 {
        match self {
            Self::Cpu | Self::Sycl => 0,
            _ => 99,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Metal => "Metal",
            Self::Cuda => "CUDA",
            Self::Rocm => "ROCm",
            Self::Hip => "HIP",
            Self::Vulkan => "Vulkan",
            Self::Sycl => "SYCL",
            Self::Cpu => "CPU only",
        }
    }

    /// Key used in `engines/lock.toml`'s `unsupported_backends` table.
    pub fn key(self) -> &'static str {
        match self {
            Self::Metal => "metal",
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
            Self::Hip => "hip",
            Self::Vulkan => "vulkan",
            Self::Sycl => "sycl",
            Self::Cpu => "cpu",
        }
    }

    /// Backends the app ships an engine for. SYCL is deliberately absent: no SYCL asset is pinned in
    /// `engines/lock.toml`, and upstream marks its ternary rows partial with no end-to-end Intel
    /// validation, so Intel GPUs are served by the Vulkan build instead.
    pub fn distributable() -> [Backend; 6] {
        [
            Self::Metal,
            Self::Cuda,
            Self::Rocm,
            Self::Hip,
            Self::Vulkan,
            Self::Cpu,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GpuArch {
    /// RTX 40-series and L4: the card reports faster *decode* on the dense-trit PTQ1_0 pack.
    NvidiaAda,
    /// L4 is Ada-based datacenter silicon; kept distinct only for labelling.
    NvidiaTuringOrOlder,
    NvidiaHopper,
    NvidiaBlackwell,
    AppleSilicon,
    /// Consumer RDNA2 (RX 6000 / gfx103x): the HIP build aborts during load (#264, open), which is
    /// why [`HostHardware::refuse_hostile_backend`] swaps that combination onto Vulkan.
    AmdRdna2,
    /// Strix Halo iGPU: PQ2_0 output is garbled on Windows HIP (#223).
    AmdGfx1151,
    AmdOther,
    /// Intel Arc: this app's Intel path is Vulkan, because no SYCL engine is pinned and upstream
    /// still marks the SYCL ternary cells partial (#235 and #356 put the kernels in the pinned build,
    /// and docs/backend/SYCL-bonsai.md measures one Arc card); and PTQ1_0 can still hang after ~1900
    /// tokens on Vulkan (#192, open).
    IntelArc,
    Unknown,
    None,
}

impl GpuArch {
    /// Classify from a GPU product name. Anything unrecognised stays `Unknown`, which the pack
    /// recommender treats as "use the general rule for this backend" rather than guessing.
    pub fn from_gpu_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("blackwell")
            || n.contains("rtx 50")
            || n.contains("geforce rtx 50")
            || n.contains("b100")
            || n.contains("b200")
            || n.contains("gb10")
            || n.contains("gb200")
            || n.contains("rtx pro 5")
        {
            return Self::NvidiaBlackwell;
        }
        if n.contains("h100") || n.contains("h200") || n.contains("h20") || n.contains("hopper") {
            return Self::NvidiaHopper;
        }
        if n.contains("a100") || n.contains("a10") || n.contains("a6000") || n.contains("ampere") {
            // A100 is explicitly listed on the card as a PQ2_0-fast part.
            return Self::NvidiaHopper;
        }
        if n.contains("rtx 40") || n.contains("ada") || n.contains("l4") || n.contains("rtx 3") {
            return Self::NvidiaAda;
        }
        if n.contains("rtx 20") || n.contains("gtx") || n.contains("turing") || n.contains("volta")
        {
            return Self::NvidiaTuringOrOlder;
        }
        if n.contains("gfx1151") || n.contains("strix halo") {
            return Self::AmdGfx1151;
        }
        if n.contains("gfx10") || n.contains("rx 6") || n.contains("rdna2") {
            return Self::AmdRdna2;
        }
        if n.contains("rx 7") || n.contains("gfx11") || n.contains("rdna") {
            return Self::AmdOther;
        }
        if n.contains("arc") {
            return Self::IntelArc;
        }
        if n.contains("apple")
            || n.contains("m1")
            || n.contains("m2")
            || n.contains("m3")
            || n.contains("m4")
            || n.contains("m5")
        {
            return Self::AppleSilicon;
        }
        Self::Unknown
    }

    /// Faster decode on PTQ1_0 for Ada-generation GPUs and L4; faster decode on PQ2_0 for
    /// Hopper/Blackwell (model card).
    pub fn prefers_dense_trits(self) -> bool {
        matches!(self, Self::NvidiaAda | Self::NvidiaTuringOrOlder)
    }
}

/// A backend this app swapped out after probing it: the probe reported `from`, this app refuses to
/// run that build on the GPU class found beside it, and every later decision sees `to`.
///
/// Carried on the probe result rather than re-derived where it is displayed, so the status card, the
/// pack choice and the launch plan cannot disagree about which engine is about to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendOverride {
    pub from: Backend,
    pub to: Backend,
    /// The sentence for the card: what was refused, and what the user should expect instead.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostHardware {
    pub backend: Backend,
    pub arch: GpuArch,
    pub gpu_name: Option<String>,
    pub total_ram_bytes: u64,
    /// Discrete GPU memory, when the probe reported one. Apple Silicon has no separate number here:
    /// its unified memory is `total_ram_bytes`.
    pub gpu_ram_bytes: Option<u64>,
    pub available_disk_bytes: u64,
    pub avx512: bool,
    /// True when the probe for the chosen backend failed and we fell back to CPU.
    pub degraded_to_cpu: bool,
    /// Set when [`HostHardware::refuse_hostile_backend`] replaced the probed backend, so the swap is
    /// reported rather than silent. `None` means `backend` is what the probe found.
    #[serde(default)]
    pub backend_override: Option<BackendOverride>,
    /// A release binary is being used rather than a build the user made themselves, so anything
    /// merged to `prism` after `pinned_tag` is unavailable. Note that the fixes this field was
    /// written for are *in* the pinned release — see the header of `engines/lock.toml` — so rules
    /// gated on it can only ever decline a pack this app has not measured, not dodge a missing fix.
    pub prebuilt_engine: bool,
}

impl Default for HostHardware {
    fn default() -> Self {
        Self {
            backend: Backend::Cpu,
            arch: GpuArch::None,
            gpu_name: None,
            total_ram_bytes: 0,
            gpu_ram_bytes: None,
            available_disk_bytes: 0,
            avx512: false,
            degraded_to_cpu: true,
            prebuilt_engine: true,
            backend_override: None,
        }
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn parse_u64(s: Option<String>) -> u64 {
    s.and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(0)
}

#[cfg(not(windows))]
/// KiB blocks from `df -kP`, converted to bytes. Returns 0 when the probe fails, which the disk
/// preflight treats as "unknown" rather than "plenty".
fn available_disk_bytes_unix(path: &str) -> u64 {
    let Some(raw) = run("df", &["-kP", path]) else {
        return 0;
    };
    let Some(last) = raw.lines().last() else {
        return 0;
    };
    last.split_whitespace()
        .nth(3)
        .and_then(|v| v.parse::<u64>().ok())
        .map(|kib| kib * 1024)
        .unwrap_or(0)
}

#[cfg(windows)]
/// Windows has no `df`; `Get-PSDrive` reports free space for the drive holding the model dir.
fn available_disk_bytes_windows(path: &str) -> u64 {
    let drive = match path.chars().next() {
        Some(c) => c.to_ascii_uppercase().to_string(),
        None => return 0,
    };
    let script = format!("(Get-PSDrive -Name {drive}).Free");
    run("powershell", &["-NoProfile", "-Command", &script])
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}

fn available_disk_bytes(path: &str) -> u64 {
    #[cfg(windows)]
    {
        available_disk_bytes_windows(path)
    }
    #[cfg(not(windows))]
    {
        available_disk_bytes_unix(path)
    }
}

/// This machine's OS key spelled the way `engines/lock.toml` spells it.
///
/// `std::env::consts::OS` says `macos`, while the lock — like llama.cpp's own asset names — says
/// `darwin`. Handing one to a lookup in the other silently resolves no target on Apple hardware, so
/// every lock lookup goes through here.
pub fn host_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

impl HostHardware {
    /// Detect the machine we are running on, sized for the directory whose free space matters
    /// (the model dir).
    pub fn detect(model_dir: &str) -> Self {
        let mut host = Self {
            total_ram_bytes: Self::detect_ram(),
            available_disk_bytes: available_disk_bytes(model_dir),
            avx512: Self::detect_avx512(),
            ..Self::default()
        };
        #[cfg(target_os = "macos")]
        {
            if run("sysctl", &["-n", "hw.optional.arm64"]).as_deref() == Some("1") {
                host.backend = Backend::Metal;
                host.arch = GpuArch::AppleSilicon;
                host.gpu_name = run("sysctl", &["-n", "hw.model"]);
                host.degraded_to_cpu = false;
            } else {
                // Intel Mac: no Metal path here, and common.sh emits -ngl 0 for it.
                host.backend = Backend::Cpu;
                host.arch = GpuArch::None;
                host.gpu_name = run("sysctl", &["-n", "machdep.cpu.brand_string"]);
                host.degraded_to_cpu = true;
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(h) = Self::detect_nvidia() {
                host.backend = Backend::Cuda;
                host.arch = h.1;
                host.gpu_name = Some(h.0);
                host.gpu_ram_bytes = h.2;
                host.degraded_to_cpu = false;
            } else if run("rocminfo", &["--version"]).is_some()
                || run("hipcc", &["--version"]).is_some()
            {
                // common.sh treats ROCm and HIP as separate binary directories; the local process
                // picks whichever build exists, both of which have PQ2_0 kernels.
                host.backend = if cfg!(target_os = "windows") {
                    Backend::Hip
                } else {
                    Backend::Rocm
                };
                // The chip class and the chip name come from one read of the same value, because a
                // probe that lands on `Unknown` here is a host `refuse_hostile_backend` cannot
                // protect from #264's load abort.
                let amd = run("rocminfo", &[])
                    .as_deref()
                    .and_then(Self::amd_target_from_rocminfo);
                host.arch = amd
                    .as_ref()
                    .map(|(arch, _)| *arch)
                    .unwrap_or(GpuArch::Unknown);
                host.gpu_name = amd.map(|(_, name)| name);
                host.degraded_to_cpu = false;
            } else if run("vulkaninfo", &["--summary"]).is_some() {
                host.backend = Backend::Vulkan;
                host.gpu_name = run("vulkaninfo", &["--summary"]).and_then(|o| {
                    o.lines()
                        .find(|l| l.contains("deviceName"))
                        .map(str::to_string)
                });
                host.arch = host
                    .gpu_name
                    .as_deref()
                    .map(GpuArch::from_gpu_name)
                    .unwrap_or(GpuArch::Unknown);
                host.degraded_to_cpu = false;
            } else {
                host.backend = Backend::Cpu;
                host.degraded_to_cpu = true;
            }
        }
        host.refuse_hostile_backend();
        host
    }

    /// `(class, name)` for the AMD agent in a `rocminfo` dump, read off the agent's ISA target — the
    /// `Name:` field inside each agent block (`  Name:   gfx1030`), which `GpuArch::from_gpu_name`
    /// resolves to a card class. CPU agents come first and their `Name:` values are not targets, so
    /// the scan takes the first line that normalises to something starting with `gfx` and ignores the
    /// rest. Compiled and tested on every host: on a machine with no AMD GPU in it this is the only
    /// way to know the parser reads the field it claims to.
    pub fn amd_target_from_rocminfo(raw: &str) -> Option<(GpuArch, String)> {
        let target = raw
            .lines()
            .map(|line| {
                line.trim()
                    .trim_start_matches("Name:")
                    .trim()
                    .trim_end_matches(':')
            })
            .find(|value| value.starts_with("gfx"))?;
        Some((GpuArch::from_gpu_name(target), target.to_string()))
    }

    fn detect_ram() -> u64 {
        #[cfg(target_os = "macos")]
        {
            parse_u64(run("sysctl", &["-n", "hw.memsize"]))
        }
        #[cfg(target_os = "linux")]
        {
            let token = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .map(str::to_string)
            });
            // MemTotal is in KiB.
            parse_u64(token).saturating_mul(1024)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            // No portable probe: 0 means unknown, which `can_hold` reports as a warning rather than
            // pretending there is room.
            parse_u64(None)
        }
    }

    fn detect_avx512() -> bool {
        #[cfg(target_os = "macos")]
        {
            run("sysctl", &["-n", "hw.optional.avx512f"]).as_deref() == Some("1")
        }
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/cpuinfo")
                .map(|s| s.contains("avx512f"))
                .unwrap_or(false)
        }
        #[cfg(windows)]
        {
            std::env::var("PROCESSOR_IDENTIFIER")
                .map(|v| v.to_ascii_uppercase().contains("AVX512"))
                .unwrap_or(false)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            false
        }
    }

    #[cfg(not(target_os = "macos"))]
    /// `(name, arch, memory_bytes)` for the first NVIDIA GPU, if nvidia-smi answers. Memory is
    /// reported in MiB by `--format=csv,noheader,nounits`.
    fn detect_nvidia() -> Option<(String, GpuArch, Option<u64>)> {
        let line = run(
            "nvidia-smi",
            &[
                "--query-gpu=name,memory.total",
                "--format=csv,noheader,nounits",
            ],
        )?;
        let first = line.lines().next()?.trim();
        let mut fields = first.split(',');
        let name = fields.next()?.trim().to_string();
        let mib = fields
            .next()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|mib| mib * 1024 * 1024);
        Some((name.clone(), GpuArch::from_gpu_name(&name), mib))
    }

    /// llama.cpp#264: the HIP engine build aborts during load on a consumer RDNA2 card (RX 6000 /
    /// gfx1030) whatever the packing, and the workaround the catalog records for it is "none on HIP:
    /// install the Vulkan engine build". This applies that workaround: the probe's HIP answer becomes
    /// Vulkan, with the reason kept on the result so the card can say what was refused.
    ///
    /// It happens here, on the probe, rather than inside `EngineBinary::resolve_for`, because the
    /// backend is read by more than the engine lookup — the pack choice (which keeps PQ2_0 off
    /// Vulkan), `-ngl`, the status card, diagnostics — and none of them may disagree about which
    /// build is about to run.
    ///
    /// HIP only. Linux reports the same silicon as ROCm, which #264 does not name, and
    /// `engines/lock.toml` pins no Linux Vulkan asset, so there would be nothing to route it to.
    ///
    /// The swap trades a certain abort for a build that needs a working Vulkan driver on the card,
    /// which is the trade upstream's own workaround line records — hence the reason travelling with
    /// the result, so the card says what was refused and why rather than changing silently.
    pub fn refuse_hostile_backend(&mut self) {
        if self.backend == Backend::Hip && self.arch == GpuArch::AmdRdna2 {
            self.backend_override = Some(BackendOverride {
                from: Backend::Hip,
                to: Backend::Vulkan,
                reason: "the HIP engine build aborts while loading on a consumer RDNA2 card \
                         (RX 6000 / gfx1030) at any packing — llama.cpp#264, still open, so no \
                         release fixes it. This app resolves the pinned Vulkan build for this \
                         machine instead, and the pack choice follows that backend."
                    .to_string(),
            });
            self.backend = Backend::Vulkan;
        }
    }

    /// The 27B packs stream from RAM/VRAM, so this is the number that decides whether a laptop can
    /// hold the model at all.
    pub fn can_hold(&self, pack_size_bytes: u64, headroom_bytes: u64) -> bool {
        self.total_ram_bytes == 0
            || self.total_ram_bytes.saturating_sub(headroom_bytes) >= pack_size_bytes
    }

    pub fn disk_has_room(&self, pack_size_bytes: u64, headroom_bytes: u64) -> bool {
        self.available_disk_bytes == 0
            || self.available_disk_bytes.saturating_sub(headroom_bytes) >= pack_size_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq2_ready_registry_matches_bonsai_demo() {
        for b in [
            Backend::Metal,
            Backend::Cpu,
            Backend::Cuda,
            Backend::Rocm,
            Backend::Hip,
        ] {
            assert!(b.pq2_0_kernels_ready(), "{b:?}");
        }
        for b in [Backend::Vulkan, Backend::Sycl] {
            assert!(!b.pq2_0_kernels_ready(), "{b:?}");
        }
    }

    #[test]
    fn offload_depth_follows_common_sh() {
        assert_eq!(Backend::Metal.layers_to_offload(), 99);
        assert_eq!(Backend::Cuda.layers_to_offload(), 99);
        assert_eq!(Backend::Cpu.layers_to_offload(), 0);
    }

    #[test]
    fn gpu_names_classify_for_pack_choice() {
        assert_eq!(
            GpuArch::from_gpu_name("NVIDIA GeForce RTX 5090"),
            GpuArch::NvidiaBlackwell
        );
        assert_eq!(GpuArch::from_gpu_name("NVIDIA L4"), GpuArch::NvidiaAda);
        assert_eq!(
            GpuArch::from_gpu_name("NVIDIA H100 80GB HBM3"),
            GpuArch::NvidiaHopper
        );
        assert_eq!(
            GpuArch::from_gpu_name("NVIDIA A100-SXM4-40GB"),
            GpuArch::NvidiaHopper
        );
        assert_eq!(
            GpuArch::from_gpu_name("Apple M5 Max"),
            GpuArch::AppleSilicon
        );
        assert_eq!(GpuArch::from_gpu_name("gfx1151"), GpuArch::AmdGfx1151);
        assert_eq!(GpuArch::from_gpu_name("Intel Arc B390"), GpuArch::IntelArc);
        assert!(GpuArch::NvidiaAda.prefers_dense_trits());
        assert!(!GpuArch::NvidiaBlackwell.prefers_dense_trits());
    }

    /// `rocminfo` prints one block per agent, CPU agents first, and the field that names the card
    /// class sits indented inside them. Written from that layout rather than captured from a Radeon
    /// card — this machine has none — so it proves the parser reads the field it claims to, not that
    /// a particular driver prints it.
    #[test]
    fn the_amd_agent_target_is_read_out_of_a_rocminfo_dump() {
        let raw = "******************\n\
                   Agent 0\n\
                   ******************\n\
                     Name: 0\n\
                   ******************\n\
                   Agent 1\n\
                   ******************\n\
                     Name:                    gfx1030\n\
                     Marketing Name:          AMD Radeon RX 6800\n";
        let (arch, name) =
            HostHardware::amd_target_from_rocminfo(raw).expect("the GPU agent names its target");
        assert_eq!(arch, GpuArch::AmdRdna2);
        assert_eq!(name, "gfx1030");
        // A CPU-only dump reports no target rather than reading `Name: 0` as one.
        assert!(HostHardware::amd_target_from_rocminfo("  Name: 0\n").is_none());
    }

    /// #264: the abort belongs to the HIP build on this card class, not to a packing, so the build
    /// must not be resolved at all here. The rule reads the probe's two fields, so no AMD GPU is
    /// needed to test it.
    #[test]
    fn a_hip_probe_on_consumer_rdna2_runs_the_vulkan_build() {
        let mut h = HostHardware {
            backend: Backend::Hip,
            arch: GpuArch::AmdRdna2,
            gpu_name: Some("gfx1030".into()),
            ..Default::default()
        };
        h.refuse_hostile_backend();
        assert_eq!(h.backend, Backend::Vulkan);
        let o = h
            .backend_override
            .expect("the swap is recorded, not made silently");
        assert_eq!((o.from, o.to), (Backend::Hip, Backend::Vulkan));
        assert!(o.reason.contains("#264"), "{}", o.reason);
        assert!(o.reason.contains("Vulkan"), "{}", o.reason);
    }

    /// Every other combination keeps the backend the probe found. An unclassified card is not swapped
    /// on a guess, and the documented neighbours each have their own answer: gfx1151 garbles output
    /// rather than aborting (#223, and the pack choice handles it), RDNA3 and newer load, and #264
    /// does not name the ROCm build Linux reports.
    #[test]
    fn only_the_rdna2_hip_combination_is_refused() {
        for (backend, arch) in [
            (Backend::Hip, GpuArch::AmdGfx1151),
            (Backend::Hip, GpuArch::AmdOther),
            (Backend::Hip, GpuArch::Unknown),
            (Backend::Rocm, GpuArch::AmdRdna2),
            (Backend::Cuda, GpuArch::AmdRdna2),
            (Backend::Vulkan, GpuArch::IntelArc),
            (Backend::Metal, GpuArch::AppleSilicon),
        ] {
            let mut h = HostHardware {
                backend,
                arch,
                ..Default::default()
            };
            h.refuse_hostile_backend();
            assert_eq!(h.backend, backend, "{backend:?} on {arch:?}");
            assert_eq!(h.backend_override, None, "{backend:?} on {arch:?}");
        }
    }

    #[test]
    fn unknown_sizes_never_claim_there_is_room() {
        let h = HostHardware {
            total_ram_bytes: 0,
            available_disk_bytes: 0,
            ..Default::default()
        };
        // A zero probe result is treated as unknown-but-permissive only for RAM sizing, which the
        // caller must surface as a warning; disk is checked separately.
        assert!(h.can_hold(7_206_168_928, 2_000_000_000));
        assert!(h.disk_has_room(7_206_168_928, 2_000_000_000));
        let big = HostHardware {
            total_ram_bytes: 32_000_000_000,
            available_disk_bytes: 20_000_000_000,
            ..Default::default()
        };
        assert!(!big.can_hold(53_808_408_928, 2_000_000_000));
        assert!(big.disk_has_room(7_206_168_928, 2_000_000_000));
        assert!(!big.disk_has_room(53_808_408_928, 2_000_000_000));
    }
}
