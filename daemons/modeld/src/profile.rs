//! Hardware profiler (`docs/PLAN.md` §5.2, tiers per §8).
//!
//! v0 heuristics: RAM (authoritative), GPU/NPU presence from device
//! nodes on Linux; Apple unified memory recognized on macOS. VRAM
//! probing (Vulkan enumeration) refines the tier in a later pass —
//! catalog consumers treat the tier as a recommendation, never a gate
//! (nothing hard-refuses, §8 "honest floor").

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HardwareProfile {
    pub os: String,
    pub arch: String,
    pub total_ram_gb: u64,
    /// True when RAM is shared CPU/GPU (Apple silicon, Strix Halo-class).
    pub unified_memory: bool,
    /// Render nodes (/dev/dri/renderD*) on Linux; assumed 1 on macOS.
    pub gpu_nodes: usize,
    /// NPU accel nodes (/dev/accel*) — bonus lane, never critical path.
    pub npu_nodes: usize,
    /// PLAN §8 tier 0–4.
    pub tier: u8,
}

pub fn profile() -> HardwareProfile {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let total_ram_gb = total_ram_bytes() >> 30;
    let unified_memory = os == "macos" || is_unified_apu();
    let (gpu_nodes, npu_nodes) = device_nodes();
    let tier = tier_of(total_ram_gb, unified_memory, gpu_nodes);
    HardwareProfile {
        os,
        arch,
        total_ram_gb,
        unified_memory,
        gpu_nodes,
        npu_nodes,
        tier,
    }
}

fn total_ram_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo")
            && let Some(kb) = meminfo
                .lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse::<u64>().ok())
        {
            return kb * 1024;
        }
        0
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

/// Strix Halo-class detection lands with real Vulkan probing; device-tree
/// heuristics are not worth their false positives yet.
fn is_unified_apu() -> bool {
    false
}

fn device_nodes() -> (usize, usize) {
    #[cfg(target_os = "linux")]
    {
        let count = |dir: &str, prefix: &str| {
            std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                        .count()
                })
                .unwrap_or(0)
        };
        (count("/dev/dri", "renderD"), count("/dev", "accel"))
    }
    #[cfg(target_os = "macos")]
    {
        (1, 0) // Metal GPU is always present on Apple silicon/Intel Macs.
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        (0, 0)
    }
}

/// PLAN §8: 0 = 4–8 GB floor · 1 = 8–16 iGPU · 2 = 16–32 (+dGPU)
/// reference · 3 = big dGPU · 4 = unified 64 GB+ flagship.
fn tier_of(ram_gb: u64, unified: bool, gpu_nodes: usize) -> u8 {
    if unified && ram_gb >= 64 {
        return 4;
    }
    match ram_gb {
        0..8 => 0,
        8..16 => 1,
        16..32 => {
            if gpu_nodes > 0 {
                2
            } else {
                1
            }
        }
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_match_plan_section_8() {
        assert_eq!(tier_of(4, false, 0), 0);
        assert_eq!(tier_of(8, false, 0), 1);
        assert_eq!(tier_of(16, false, 1), 2);
        assert_eq!(tier_of(16, false, 0), 1, "no GPU => stay tier 1");
        assert_eq!(tier_of(48, false, 1), 3);
        assert_eq!(tier_of(64, true, 1), 4, "unified 64GB+ is the flagship");
        assert_eq!(tier_of(128, true, 1), 4);
    }

    #[test]
    fn local_profile_is_sane() {
        let p = profile();
        assert!(p.total_ram_gb > 0, "RAM detection failed: {p:?}");
        assert!(p.tier <= 4);
    }
}
