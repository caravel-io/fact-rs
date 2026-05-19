use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;

#[derive(Serialize, Debug)]
pub struct CPUFacts {
    pub count: u32,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub model: Vec<String>,
    pub architecture: String,
}

#[derive(Default)]
pub struct CPUComponent;

impl CPUComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Collector for CPUComponent {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let cf = get_cpu_info()?;
        Ok(to_value(cf)?)
    }
}

// --- Linux ---

#[cfg(target_os = "linux")]
fn get_cpu_info() -> Result<CPUFacts> {
    use crate::filesystem::slurp;
    use std::path::Path;

    let contents = slurp(Path::new("/proc/cpuinfo")).context("failed to read cpuinfo")?;
    let cpu_count = get_cpu_count(&contents);
    let phys_core_count = get_physical_core_count(&contents, cpu_count);
    let log_core_count = get_logical_core_count(&contents);
    let arch = get_architecture(&contents);
    let model = get_cpu_model(&contents);
    Ok(CPUFacts {
        count: cpu_count,
        physical_cores: phys_core_count,
        logical_cores: log_core_count,
        architecture: arch,
        model,
    })
}

#[cfg(any(target_os = "linux", test))]
fn get_cpu_count(contents: &str) -> u32 {
    use std::collections::HashSet;
    let ids: HashSet<&str> = contents
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.trim() == "physical id").then(|| v.trim())
        })
        .collect();

    if ids.is_empty() { 1 } else { ids.len() as u32 }
}

#[cfg(any(target_os = "linux", test))]
fn get_physical_core_count(contents: &str, cpu_count: u32) -> u32 {
    // "cpu cores" reports cores per socket on x86
    let cores_per_socket = contents.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == "cpu cores").then(|| v.trim().parse::<u32>().ok())?
    });

    match cores_per_socket {
        Some(cores) => cores * cpu_count,
        // ARM doesn't have "cpu cores" so logical == physical
        None => get_logical_core_count(contents),
    }
}

#[cfg(any(target_os = "linux", test))]
fn get_logical_core_count(contents: &str) -> u32 {
    contents
        .lines()
        .filter(|line| line.trim_start().starts_with("processor"))
        .filter(|line| line.split_once(':').is_some())
        .count() as u32
}

#[cfg(any(target_os = "linux", test))]
fn get_cpu_model(contents: &str) -> Vec<String> {
    use crate::filesystem::slurp;
    use std::collections::HashSet;
    use std::path::Path;

    // x86: deduplicated "model name" values (multi-socket systems may have different CPUs)
    let models: HashSet<String> = contents
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.trim() == "model name").then(|| v.trim().to_string())
        })
        .collect();

    if !models.is_empty() {
        let mut v: Vec<String> = models.into_iter().collect();
        v.sort();
        return v;
    }

    // ARM fallback: /proc/device-tree/model (e.g. "Apple M1", "Raspberry Pi 4 Model B")
    // Device-tree strings are null-terminated, so trim \0
    if let Ok(model) = slurp(Path::new("/proc/device-tree/model")) {
        let model = model.trim_matches('\0').trim().to_string();
        if !model.is_empty() {
            return vec![model];
        }
    }

    // Last resort: construct a generic string from CPU implementer and part
    let implementer = contents.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == "CPU implementer").then(|| v.trim().to_string())
    });
    let part = contents.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == "CPU part").then(|| v.trim().to_string())
    });

    match (implementer, part) {
        (Some(imp), Some(prt)) => vec![format!("ARM (implementer={}, part={})", imp, prt)],
        _ => vec![],
    }
}

#[cfg(any(target_os = "linux", test))]
fn get_architecture(contents: &str) -> String {
    // probably need to do better here...
    for line in contents.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "flags" => {
                let flags: Vec<&str> = v.split_whitespace().collect();
                return if flags.contains(&"lm") {
                    "x86_64".to_string()
                } else {
                    "x86".to_string()
                };
            }
            "CPU architecture" => {
                return match v.trim() {
                    "8" => "aarch64".to_string(),
                    "7" => "armv7l".to_string(),
                    other => format!("arm_v{}", other),
                };
            }
            _ => continue,
        }
    }
    "unknown".to_string()
}

// --- macOS ---

#[cfg(target_os = "macos")]
fn get_cpu_info() -> Result<CPUFacts> {
    use crate::filesystem::sysctl_n;
    let count = sysctl_n("hw.packages")?.parse::<u32>().context("parsing hw.packages")?;
    let physical_cores = sysctl_n("hw.physicalcpu")?.parse::<u32>().context("parsing hw.physicalcpu")?;
    let logical_cores = sysctl_n("hw.logicalcpu")?.parse::<u32>().context("parsing hw.logicalcpu")?;
    let model_str = sysctl_n("machdep.cpu.brand_string")?;
    let model = if model_str.is_empty() { vec![] } else { vec![model_str] };
    let architecture = sysctl_n("hw.machine")?;
    Ok(CPUFacts { count, physical_cores, logical_cores, model, architecture })
}

// --- Windows ---

#[cfg(target_os = "windows")]
fn get_cpu_info() -> Result<CPUFacts> {
    use crate::filesystem::run_powershell;
    let script = concat!(
        "$p = @(Get-CimInstance Win32_Processor);",
        "Write-Output $p.Count;",
        "Write-Output (($p | Measure-Object -Property NumberOfCores -Sum).Sum);",
        "Write-Output (($p | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum);",
        "Write-Output (($p | Select-Object -ExpandProperty Name -Unique) -join ',');",
        "Write-Output $env:PROCESSOR_ARCHITECTURE",
    );
    parse_cpu_info_windows(&run_powershell(script)?)
}

#[cfg(any(target_os = "windows", test))]
fn parse_cpu_info_windows(s: &str) -> Result<CPUFacts> {
    let mut lines = s.lines();
    let count = lines.next().context("missing cpu count")?.trim().parse::<u32>().context("parsing count")?;
    let physical_cores = lines.next().context("missing physical cores")?.trim().parse::<u32>().context("parsing physical_cores")?;
    let logical_cores = lines.next().context("missing logical cores")?.trim().parse::<u32>().context("parsing logical_cores")?;
    let model_str = lines.next().context("missing model")?.trim().to_string();
    let model = if model_str.is_empty() {
        vec![]
    } else {
        model_str.split(',').map(|s| s.trim().to_string()).collect()
    };
    let architecture = lines.next().context("missing architecture")?.trim().to_string();

    Ok(CPUFacts {
        count,
        physical_cores,
        logical_cores,
        model,
        architecture,
    })
}

// --- Fallback ---

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_cpu_info() -> Result<CPUFacts> {
    anyhow::bail!("cpu not implemented on this platform")
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    const X86_64_SINGLE: &str = "\
processor	: 0
model name	: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz
physical id	: 0
cpu cores	: 6
flags		: fpu vme lm
processor	: 1
model name	: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz
physical id	: 0
cpu cores	: 6
flags		: fpu vme lm";

    const X86_64_MULTI_SOCKET: &str = "\
processor	: 0
model name	: Intel(R) Xeon(R) CPU E5-2680 v4
physical id	: 0
cpu cores	: 4
flags		: fpu lm
processor	: 1
model name	: Intel(R) Xeon(R) CPU E5-2680 v4
physical id	: 1
cpu cores	: 4
flags		: fpu lm";

    const X86_32: &str = "\
processor	: 0
model name	: Intel(R) Pentium(R) 4
physical id	: 0
cpu cores	: 1
flags		: fpu vme pse";

    const AARCH64: &str = "\
processor	: 0
CPU architecture: 8
CPU implementer	: 0x41
CPU part	: 0xd0b
processor	: 1
CPU architecture: 8
CPU implementer	: 0x41
CPU part	: 0xd0b";

    const ARMV7: &str = "\
processor	: 0
CPU architecture: 7
CPU implementer	: 0x41
CPU part	: 0xc09";

    #[test]
    fn test_get_architecture_x86_64() {
        assert_eq!(get_architecture(X86_64_SINGLE), "x86_64");
    }

    #[test]
    fn test_get_architecture_x86_32() {
        assert_eq!(get_architecture(X86_32), "x86");
    }

    #[test]
    fn test_get_architecture_aarch64() {
        assert_eq!(get_architecture(AARCH64), "aarch64");
    }

    #[test]
    fn test_get_architecture_armv7() {
        assert_eq!(get_architecture(ARMV7), "armv7l");
    }

    #[test]
    fn test_get_architecture_unknown() {
        assert_eq!(
            get_architecture("processor\t: 0\nvendor_id\t: GenuineIntel"),
            "unknown"
        );
    }

    #[test]
    fn test_get_cpu_count_single_socket() {
        assert_eq!(get_cpu_count(X86_64_SINGLE), 1);
    }

    #[test]
    fn test_get_cpu_count_multi_socket() {
        assert_eq!(get_cpu_count(X86_64_MULTI_SOCKET), 2);
    }

    #[test]
    fn test_get_cpu_count_arm_no_physical_id() {
        // ARM doesn't report physical id — should default to 1
        assert_eq!(get_cpu_count(AARCH64), 1);
    }

    #[test]
    fn test_get_logical_core_count() {
        assert_eq!(get_logical_core_count(X86_64_SINGLE), 2);
        assert_eq!(get_logical_core_count(X86_64_MULTI_SOCKET), 2);
        assert_eq!(get_logical_core_count(AARCH64), 2);
        assert_eq!(get_logical_core_count(ARMV7), 1);
    }

    #[test]
    fn test_get_physical_core_count_x86_single_socket() {
        // 6 cores per socket * 1 socket
        assert_eq!(get_physical_core_count(X86_64_SINGLE, 1), 6);
    }

    #[test]
    fn test_get_physical_core_count_x86_multi_socket() {
        // 4 cores per socket * 2 sockets
        assert_eq!(get_physical_core_count(X86_64_MULTI_SOCKET, 2), 8);
    }

    #[test]
    fn test_get_physical_core_count_arm_falls_back_to_logical() {
        // ARM has no "cpu cores" field — physical == logical
        assert_eq!(
            get_physical_core_count(AARCH64, 1),
            get_logical_core_count(AARCH64)
        );
    }

    #[test]
    fn test_get_cpu_model_x86() {
        let model = get_cpu_model(X86_64_SINGLE);
        assert_eq!(model, vec!["Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz"]);
    }

    #[test]
    fn test_get_cpu_model_deduplicates() {
        // Both processors in X86_64_SINGLE have the same model name
        let model = get_cpu_model(X86_64_SINGLE);
        assert_eq!(model.len(), 1);
    }

    #[test]
    fn test_get_cpu_model_multi_socket_different_cpus() {
        let contents = "\
processor	: 0
model name	: Intel(R) Xeon(R) E5-2680
physical id	: 0
processor	: 1
model name	: Intel(R) Xeon(R) E5-2690
physical id	: 1";
        let mut model = get_cpu_model(contents);
        model.sort();
        assert_eq!(model.len(), 2);
        assert!(model.contains(&"Intel(R) Xeon(R) E5-2680".to_string()));
        assert!(model.contains(&"Intel(R) Xeon(R) E5-2690".to_string()));
    }

    #[test]
    fn test_get_cpu_model_arm_implementer_fallback() {
        // No model name, no device-tree (not present in test env) — falls back to implementer/part
        let model = get_cpu_model(AARCH64);
        assert_eq!(model, vec!["ARM (implementer=0x41, part=0xd0b)"]);
    }

    #[test]
    fn test_get_cpu_model_empty_when_nothing_known() {
        assert_eq!(get_cpu_model("processor\t: 0"), vec![] as Vec<String>);
    }

    #[test]
    fn test_parse_cpu_info_windows_single_socket() {
        let s = "1\n6\n12\nIntel(R) Core(TM) i7-9750H CPU @ 2.60GHz\nAMD64\n";
        let facts = parse_cpu_info_windows(s).unwrap();
        assert_eq!(facts.count, 1);
        assert_eq!(facts.physical_cores, 6);
        assert_eq!(facts.logical_cores, 12);
        assert_eq!(facts.model, vec!["Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz"]);
        assert_eq!(facts.architecture, "AMD64");
    }

    #[test]
    fn test_parse_cpu_info_windows_multi_socket() {
        let s = "2\n8\n16\nIntel(R) Xeon(R) E5-2680,Intel(R) Xeon(R) E5-2690\nAMD64\n";
        let facts = parse_cpu_info_windows(s).unwrap();
        assert_eq!(facts.count, 2);
        assert_eq!(facts.physical_cores, 8);
        assert_eq!(facts.model.len(), 2);
    }
}
