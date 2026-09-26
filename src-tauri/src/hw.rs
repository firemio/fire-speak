//! Accelerator detection for the local whisper engine (v0.9).
//!
//! Probed once per process (the Windows probe spawns PowerShell for WMI, about
//! a second) and cached. `setup::sync_server_build` (run at startup) probes on
//! a blocking thread first, so the settings screen normally never waits.

use serde::Serialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Hardware {
    /// NVIDIA driver present (nvcuda.dll) — the CUDA build can run.
    pub nvidia: bool,
    /// An AMD Radeon GPU worth offloading to: a discrete RX/Pro card or a
    /// named APU graphics (780M, 890M, 8060S …). The unnamed 2-CU "AMD
    /// Radeon(TM) Graphics" of desktop Ryzen CPUs does not count — it can be
    /// slower than the CPU.
    pub amd_gpu: bool,
    /// The Vulkan loader is installed (vulkan-1.dll / libvulkan.so.1).
    pub vulkan: bool,
    /// An AMD XDNA NPU with its driver ("NPU Compute Accelerator Device").
    pub npu: bool,
    /// Display names of the GPUs, for the settings screen.
    pub gpus: Vec<String>,
}

static HW: OnceLock<Hardware> = OnceLock::new();

pub fn get() -> &'static Hardware {
    HW.get_or_init(probe)
}

#[cfg(windows)]
fn probe() -> Hardware {
    use std::os::windows::process::CommandExt;
    use std::path::Path;

    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let sys32 = Path::new(&root).join("System32");
    let mut hw = Hardware {
        nvidia: sys32.join("nvcuda.dll").exists(),
        vulkan: sys32.join("vulkan-1.dll").exists(),
        ..Default::default()
    };

    // Same device names Lemonade keys on: Win32_VideoController for GPUs and
    // the PnP entity the AMD NPU driver registers. Driver-provided names are
    // not localized.
    let script = "$ErrorActionPreference='SilentlyContinue';\
        $g=@(Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name });\
        $n=@(Get-CimInstance Win32_PnPEntity -Filter \"Name LIKE '%NPU Compute Accelerator%'\").Count;\
        ConvertTo-Json -Compress @{gpus=$g;npu=$n}";
    if let Some(stdout) = run_with_timeout(
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(0x0800_0000), // CREATE_NO_WINDOW
        std::time::Duration::from_secs(8),
    ) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&stdout) {
            hw.gpus = match &v["gpus"] {
                serde_json::Value::Array(a) => {
                    a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()
                }
                serde_json::Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
            hw.npu = v["npu"].as_u64().unwrap_or(0) > 0;
        }
    }
    hw.amd_gpu = hw.gpus.iter().any(|n| is_amd_gpu_name(n));
    hw
}

/// Run `cmd` and return its stdout, or None if it fails or is still running
/// after `timeout` (then it is killed). A hung WMI query must not block the
/// settings screen, which reads the cached probe. The output is a few hundred
/// bytes, far below the pipe buffer, so reading after exit cannot deadlock.
#[cfg(windows)]
fn run_with_timeout(cmd: &mut std::process::Command, timeout: std::time::Duration) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    Some(out)
}

#[cfg(target_os = "linux")]
fn probe() -> Hardware {
    // Only the Vulkan loader is probed: sysfs does not tell a large Radeon
    // from a 2-CU desktop iGPU, so "auto" never picks Vulkan on Linux and the
    // user selects it explicitly (untested on real Linux hardware).
    Hardware {
        vulkan: [
            "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
            "/usr/lib64/libvulkan.so.1",
            "/usr/lib/libvulkan.so.1",
        ]
        .iter()
        .any(|p| std::path::Path::new(p).exists()),
        ..Default::default()
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn probe() -> Hardware {
    Hardware::default()
}

fn is_amd_gpu_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("radeon") && n.chars().any(|c| c.is_ascii_digit())
}

/// Accelerators the app can download a whisper-server build for on this
/// platform/architecture, in display order.
pub fn supported_accels() -> &'static [&'static str] {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        &["auto", "cuda", "vulkan", "npu", "cpu"]
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        &["auto", "vulkan", "cpu"]
    }
    #[cfg(not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        &["auto", "cpu"]
    }
}

/// Resolve the configured accelerator to a concrete build:
/// "cuda" | "vulkan" | "npu" | "cpu".
///
/// "auto": CPU when the legacy `gpu` switch is off; otherwise CUDA on an
/// NVIDIA driver, Vulkan on an AMD Radeon with the Vulkan loader, else CPU.
/// The NPU is never chosen automatically — on Strix Halo the Radeon iGPU runs
/// the whole model and is faster than the NPU, which only takes the encoder.
pub fn resolve_accel(configured: &str, gpu_switch: bool, hw: &Hardware) -> &'static str {
    let supported = supported_accels();
    match configured {
        "cuda" | "vulkan" | "npu" | "cpu" if supported.contains(&configured) => match configured {
            "cuda" => "cuda",
            "vulkan" => "vulkan",
            "npu" => "npu",
            _ => "cpu",
        },
        _ => {
            if !gpu_switch {
                "cpu"
            } else if hw.nvidia && supported.contains(&"cuda") {
                "cuda"
            } else if hw.amd_gpu && hw.vulkan && supported.contains(&"vulkan") {
                "vulkan"
            } else {
                "cpu"
            }
        }
    }
}

/// Whether an installed server build can serve the wanted accelerator.
/// CPU is served by the CUDA and Vulkan builds too (with `--no-gpu`), so
/// switching to CPU never forces a download; the NPU build is not reused.
pub fn build_satisfies(installed: &str, wanted: &str) -> bool {
    installed == wanted || (wanted == "cpu" && matches!(installed, "cuda" | "vulkan"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(nvidia: bool, amd_gpu: bool, vulkan: bool, npu: bool) -> Hardware {
        Hardware { nvidia, amd_gpu, vulkan, npu, gpus: Vec::new() }
    }

    #[test]
    fn amd_names() {
        assert!(is_amd_gpu_name("AMD Radeon(TM) 8060S Graphics"));
        assert!(is_amd_gpu_name("AMD Radeon RX 7900 XTX"));
        assert!(is_amd_gpu_name("AMD Radeon 780M Graphics"));
        assert!(!is_amd_gpu_name("AMD Radeon(TM) Graphics"));
        assert!(!is_amd_gpu_name("NVIDIA GeForce RTX 4070 Ti SUPER"));
        assert!(!is_amd_gpu_name("Intel(R) Arc(TM) Graphics"));
    }

    #[test]
    fn build_reuse() {
        assert!(build_satisfies("cuda", "cuda"));
        assert!(build_satisfies("vulkan", "cpu"));
        assert!(build_satisfies("cuda", "cpu"));
        assert!(!build_satisfies("npu", "cpu"));
        assert!(!build_satisfies("cpu", "vulkan"));
        assert!(!build_satisfies("vulkan", "npu"));
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn auto_resolution() {
        assert_eq!(resolve_accel("auto", true, &hw(true, false, true, false)), "cuda");
        // Strix Halo: Radeon iGPU + NPU -> Vulkan, never the NPU implicitly
        assert_eq!(resolve_accel("auto", true, &hw(false, true, true, true)), "vulkan");
        assert_eq!(resolve_accel("auto", true, &hw(false, true, false, true)), "cpu");
        assert_eq!(resolve_accel("auto", false, &hw(true, true, true, true)), "cpu");
        assert_eq!(resolve_accel("auto", true, &hw(false, false, true, false)), "cpu");
        assert_eq!(resolve_accel("npu", true, &hw(false, true, true, true)), "npu");
        assert_eq!(resolve_accel("bogus", true, &hw(false, true, true, false)), "vulkan");
    }
}

#[cfg(test)]
mod probe_tests {
    /// Manual check on a real machine: `cargo test probe_this_machine -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_this_machine() {
        let hw = super::get();
        println!("{hw:?} -> auto = {}", super::resolve_accel("auto", true, hw));
    }
}
