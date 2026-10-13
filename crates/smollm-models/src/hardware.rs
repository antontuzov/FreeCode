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
    /// Vulkan is excluded because PQ2_0 silently decodes on the CPU there (llama.cpp#238, fixed in
    /// source but not in a binary release), and SYCL is excluded because it cannot load either
    /// ternary type at all (#235).
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

    /// Backends the app ships an engine for. SYCL is deliberately absent: it cannot load either
    /// ternary type (llama.cpp#235), so Intel GPUs are served by the Vulkan build instead.
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
    AmdRdna2,
    /// Strix Halo iGPU: PQ2_0 output is garbled on Windows HIP (#223).
    AmdGfx1151,
    AmdOther,
    /// Intel Arc: Vulkan only (SYCL cannot load ternary), and PTQ1_0 can hang after ~1900 tokens
    /// (#192).
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
    /// A binary release is being used rather than a from-source build, so fixes merged to `prism`
    /// but not yet shipped are unavailable.
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
                host.arch = run("rocminfo", &[])
                    .and_then(|o| {
                        o.lines()
                            .find(|l| l.trim_start().starts_with("gfx"))
                            .map(|l| l.trim().to_string())
                    })
                    .map(|g| GpuArch::from_gpu_name(&g))
                    .unwrap_or(GpuArch::Unknown);
                host.gpu_name = run("rocminfo", &[]).and_then(|o| {
                    o.lines()
                        .find_map(|l| l.strip_prefix("  Name:"))
                        .map(str::trim)
                        .map(str::to_string)
                });
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
        host
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
