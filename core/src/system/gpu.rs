use std::process::Command;

use serde::{Deserialize, Serialize};

/// A detected PCI graphics device.
#[derive(Debug, Clone)]
pub struct GpuDevice {
    pub name: String,
}

impl GpuDevice {
    pub fn is_nvidia(&self) -> bool {
        let n = self.name.to_lowercase();
        n.contains("nvidia")
    }

    pub fn is_amd(&self) -> bool {
        let n = self.name.to_lowercase();
        n.contains("amd") || n.contains("ati") || n.contains("radeon")
    }

    pub fn is_intel(&self) -> bool {
        let n = self.name.to_lowercase();
        n.contains("intel")
    }
}

/// Detect graphics devices by parsing `lspci` output.
/// Returns an empty Vec if `lspci` is unavailable.
pub fn detect_gpus() -> Vec<GpuDevice> {
    let output = Command::new("lspci").output();
    let Ok(out) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter(|l| l.contains(" VGA ") || l.contains(" 3D ") || l.contains(" Display "))
        .filter_map(|l| l.split_once(": ").map(|(_, name)| name.trim().to_string()))
        .map(|name| GpuDevice { name })
        .collect()
}

/// Suggest the most appropriate `GfxDriver` given detected GPUs.
/// Prefers open-source drivers. NVIDIA requires manual selection between
/// open-kernel and nouveau so we default to the open kernel module.
pub fn suggested_driver(gpus: &[GpuDevice]) -> Option<GfxDriver> {
    if gpus.is_empty() {
        return None;
    }

    let has_nvidia = gpus.iter().any(|g| g.is_nvidia());
    let has_amd = gpus.iter().any(|g| g.is_amd());
    let has_intel = gpus.iter().any(|g| g.is_intel());

    match (has_nvidia, has_amd, has_intel) {
        // Multi-vendor or all-open fallback
        (true, true, _) | (true, _, true) | (false, true, true) => Some(GfxDriver::AllOpenSource),
        (true, false, false) => Some(GfxDriver::NvidiaOpen),
        (false, true, false) => Some(GfxDriver::Amd),
        (false, false, true) => Some(GfxDriver::Intel),
        (false, false, false) => Some(GfxDriver::AllOpenSource),
    }
}

/// Graphics driver selection. Maps to the package sets that will be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GfxDriver {
    /// All open-source drivers (AMD + Intel + nouveau).
    AllOpenSource,
    /// AMD / ATI open-source (mesa + AMDGPU + Vulkan).
    Amd,
    /// Intel open-source (mesa + VA-API + Vulkan).
    Intel,
    /// NVIDIA open kernel module (Turing and newer, replaces proprietary).
    NvidiaOpen,
    /// NVIDIA nouveau open-source driver.
    NvidiaNouveau,
    /// VirtualBox / generic VM open-source driver.
    Vm,
}

impl std::fmt::Display for GfxDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllOpenSource => write!(f, "All open-source"),
            Self::Amd => write!(f, "AMD / ATI (open-source)"),
            Self::Intel => write!(f, "Intel (open-source)"),
            Self::NvidiaOpen => {
                write!(f, "NVIDIA (open kernel module, Turing+)")
            }
            Self::NvidiaNouveau => write!(f, "NVIDIA (nouveau open-source)"),
            Self::Vm => write!(f, "VM / VirtualBox (open-source)"),
        }
    }
}

impl GfxDriver {
    /// Return the packages that should be installed for this driver.
    pub fn packages(&self) -> &'static [&'static str] {
        match self {
            Self::AllOpenSource => &[
                "mesa",
                "xf86-video-amdgpu",
                "xf86-video-ati",
                "xf86-video-nouveau",
                "libva-intel-driver",
                "intel-media-driver",
                "vulkan-radeon",
                "vulkan-intel",
                "vulkan-nouveau",
            ],
            Self::Amd => &[
                "mesa",
                "xf86-video-amdgpu",
                "xf86-video-ati",
                "vulkan-radeon",
            ],
            Self::Intel => &[
                "mesa",
                "libva-intel-driver",
                "intel-media-driver",
                "vulkan-intel",
            ],
            Self::NvidiaOpen => &["nvidia-open-dkms", "dkms", "libva-nvidia-driver"],
            Self::NvidiaNouveau => &["mesa", "xf86-video-nouveau", "vulkan-nouveau"],
            Self::Vm => &["mesa"],
        }
    }

    pub fn is_nvidia(&self) -> bool {
        matches!(self, Self::NvidiaOpen | Self::NvidiaNouveau)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gfx_driver_packages_nonempty() {
        for driver in [
            GfxDriver::AllOpenSource,
            GfxDriver::Amd,
            GfxDriver::Intel,
            GfxDriver::NvidiaOpen,
            GfxDriver::NvidiaNouveau,
            GfxDriver::Vm,
        ] {
            assert!(!driver.packages().is_empty(), "{driver} has no packages");
        }
    }

    #[test]
    fn test_suggested_driver_nvidia() {
        let gpus = vec![GpuDevice {
            name: "NVIDIA GeForce RTX 3080".into(),
        }];
        assert_eq!(suggested_driver(&gpus), Some(GfxDriver::NvidiaOpen));
    }

    #[test]
    fn test_suggested_driver_amd() {
        let gpus = vec![GpuDevice {
            name: "AMD Radeon RX 6800".into(),
        }];
        assert_eq!(suggested_driver(&gpus), Some(GfxDriver::Amd));
    }

    #[test]
    fn test_suggested_driver_intel() {
        let gpus = vec![GpuDevice {
            name: "Intel UHD Graphics 620".into(),
        }];
        assert_eq!(suggested_driver(&gpus), Some(GfxDriver::Intel));
    }

    #[test]
    fn test_suggested_driver_hybrid() {
        let gpus = vec![
            GpuDevice {
                name: "Intel UHD Graphics 620".into(),
            },
            GpuDevice {
                name: "NVIDIA GeForce GTX 1650".into(),
            },
        ];
        assert_eq!(suggested_driver(&gpus), Some(GfxDriver::AllOpenSource));
    }

    #[test]
    fn test_suggested_driver_empty() {
        assert_eq!(suggested_driver(&[]), None);
    }

    #[test]
    fn test_serde_roundtrip() {
        for driver in [
            GfxDriver::AllOpenSource,
            GfxDriver::Amd,
            GfxDriver::Intel,
            GfxDriver::NvidiaOpen,
            GfxDriver::NvidiaNouveau,
            GfxDriver::Vm,
        ] {
            let json = serde_json::to_string(&driver).unwrap();
            let back: GfxDriver = serde_json::from_str(&json).unwrap();
            assert_eq!(driver, back);
        }
    }
}
