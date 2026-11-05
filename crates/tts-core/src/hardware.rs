//! Hardware detection and compatibility checks.
//!
//! The specification requires a compatibility check before a model is
//! installed or loaded. In particular the desktop application running on
//! macOS must not assume that Fish Audio Drama 3 can run there.

use std::process::Stdio;

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessesToUpdate, System};

/// Description of the machine the application runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    /// `macos`, `linux` or `windows`.
    pub platform: String,
    /// `aarch64`, `x86_64`, ...
    pub architecture: String,
    /// True on Apple Silicon.
    pub apple_silicon: bool,
    /// True when an NVIDIA GPU with a usable driver was detected.
    pub cuda_available: bool,
    /// Name of the detected NVIDIA GPU, when known.
    pub nvidia_gpu: Option<String>,
    /// Total physical memory in bytes.
    pub total_ram_bytes: u64,
    /// Memory that is free right now, in bytes.
    pub available_ram_bytes: u64,
}

impl HardwareProfile {
    /// Detects the current machine.
    pub fn detect() -> Self {
        let platform = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        };
        let architecture = std::env::consts::ARCH.to_string();
        let apple_silicon = cfg!(target_os = "macos") && architecture == "aarch64";

        let (total_ram_bytes, available_ram_bytes) = {
            let mut system = System::new();
            system.refresh_memory();
            // macOS does not report the "available" memory, only the free
            // pages; the larger of the two is the honest answer.
            let available = system.available_memory().max(system.free_memory());
            (system.total_memory(), available)
        };

        let (cuda_available, nvidia_gpu) = detect_nvidia();

        Self {
            platform: platform.to_string(),
            architecture,
            apple_silicon,
            cuda_available,
            nvidia_gpu,
            total_ram_bytes,
            available_ram_bytes,
        }
    }

    /// Builds a profile by hand; used by tests and by the CLI.
    pub fn new(
        platform: impl Into<String>,
        architecture: impl Into<String>,
        apple_silicon: bool,
        cuda_available: bool,
        total_ram_bytes: u64,
    ) -> Self {
        Self {
            platform: platform.into(),
            architecture: architecture.into(),
            apple_silicon,
            cuda_available,
            nvidia_gpu: None,
            total_ram_bytes,
            // A profile that is built by hand knows no free memory.
            available_ram_bytes: 0,
        }
    }
}

/// Detects NVIDIA GPUs by asking `nvidia-smi`.
fn detect_nvidia() -> (bool, Option<String>) {
    let output = std::process::Command::new("nvidia-smi")
        .arg("--query-gpu=name")
        .arg("--format=csv,noheader")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let name = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            (true, name)
        }
        _ => (false, None),
    }
}

/// Reads the memory a process occupies right now.
///
/// The value is the resident memory of the process, not an estimate. It is
/// `None` when the process does not exist any more.
pub fn process_memory(pid: u32) -> Option<u64> {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(|process| process.memory())
}
