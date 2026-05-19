use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;
use std::collections::HashMap;

#[derive(Serialize, Debug)]
pub struct MemoryType {
    pub total_bytes: u64,
}

#[derive(Default)]
pub struct MemoryComponent;

impl MemoryComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Collector for MemoryComponent {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let facts = get_memory_facts()?;
        Ok(to_value(facts)?)
    }
}

// --- Linux ---

#[cfg(target_os = "linux")]
fn get_memory_facts() -> Result<HashMap<String, MemoryType>> {
    use crate::filesystem::slurp;
    use std::path::Path;
    let contents = slurp(Path::new("/proc/meminfo"))?;
    let meminfo = parse_meminfo(&contents);
    Ok(build_memory_facts(meminfo))
}

#[cfg(any(target_os = "linux", test))]
fn parse_meminfo(contents: &str) -> HashMap<String, u64> {
    contents
        .lines()
        .filter_map(|line| {
            let (label, rest) = line.split_once(':')?;
            let mut parts = rest.split_whitespace();
            let value = parts.next()?.parse::<u64>().ok()?;
            let multiplier = match parts.next() {
                Some("kB") => 1024,
                Some("mB") | Some("MB") => 1_000_000,
                Some("B") | None => 1,
                _ => 1,
            };
            Some((label.to_string(), value * multiplier))
        })
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn build_memory_facts(meminfo: HashMap<String, u64>) -> HashMap<String, MemoryType> {
    let mut sections: HashMap<String, MemoryType> = HashMap::new();
    for (key, total_bytes) in meminfo.iter() {
        match key.as_str() {
            "MemTotal" => sections.insert("real".to_string(), MemoryType { total_bytes: *total_bytes }),
            "SwapTotal" => sections.insert("swap".to_string(), MemoryType { total_bytes: *total_bytes }),
            _ => continue,
        };
    }
    sections
}

// --- macOS ---

#[cfg(target_os = "macos")]
fn get_memory_facts() -> Result<HashMap<String, MemoryType>> {
    use crate::filesystem::sysctl_n;
    let real_bytes = sysctl_n("hw.memsize")?.parse::<u64>().context("parsing hw.memsize")?;
    let swap_bytes = parse_vm_swapusage(&sysctl_n("vm.swapusage")?)?;
    let mut facts = HashMap::new();
    facts.insert("real".to_string(), MemoryType { total_bytes: real_bytes });
    facts.insert("swap".to_string(), MemoryType { total_bytes: swap_bytes });
    Ok(facts)
}

#[cfg(any(target_os = "macos", test))]
fn parse_vm_swapusage(s: &str) -> Result<u64> {
    // "total = 2048.00M  used = 0.00M  free = 2048.00M  (encrypted)"
    let value_str = s
        .split_whitespace()
        .skip_while(|&t| t != "total")
        .nth(2)
        .ok_or_else(|| anyhow::anyhow!("could not find total in vm.swapusage output"))?;

    let (num_str, unit) = value_str.split_at(value_str.len() - 1);
    let num: f64 = num_str.parse().context("parsing swap total value")?;
    let multiplier = match unit {
        "B" => 1.0,
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        other => anyhow::bail!("unknown unit '{}' in vm.swapusage", other),
    };
    Ok((num * multiplier) as u64)
}

// --- Windows ---

#[cfg(target_os = "windows")]
fn get_memory_facts() -> Result<HashMap<String, MemoryType>> {
    use crate::filesystem::run_powershell;
    let script = concat!(
        "$cs = Get-CimInstance Win32_ComputerSystem;",
        "$os = Get-CimInstance Win32_OperatingSystem;",
        "Write-Output $cs.TotalPhysicalMemory;",
        "Write-Output ($os.TotalPageFile * 1KB)",
    );
    parse_memory_facts_windows(&run_powershell(script)?)
}

#[cfg(any(target_os = "windows", test))]
fn parse_memory_facts_windows(s: &str) -> Result<HashMap<String, MemoryType>> {
    let mut lines = s.lines();
    let real_bytes = lines.next().context("missing TotalPhysicalMemory")?.trim().parse::<u64>().context("parsing TotalPhysicalMemory")?;
    let swap_bytes = lines.next().context("missing TotalPageFile")?.trim().parse::<u64>().context("parsing TotalPageFile")?;
    let mut facts = HashMap::new();
    facts.insert("real".to_string(), MemoryType { total_bytes: real_bytes });
    facts.insert("swap".to_string(), MemoryType { total_bytes: swap_bytes });
    Ok(facts)
}

// --- Fallback ---

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_memory_facts() -> Result<HashMap<String, MemoryType>> {
    anyhow::bail!("memory not implemented on this platform")
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_meminfo() {
        let content = "MemTotal:       16384 kB\nSwapTotal:      8192 kB\n";
        let meminfo = parse_meminfo(content);
        assert_eq!(meminfo.get("MemTotal"), Some(&16777216));
        assert_eq!(meminfo.get("SwapTotal"), Some(&8388608));
    }

    #[test]
    fn test_parse_meminfo_units() {
        let content = "HugePages_Total: 1024 B\nDirectMap2M:       2 MB\nSomeValue:         3 mB\n";
        let meminfo = parse_meminfo(content);
        assert_eq!(meminfo.get("HugePages_Total"), Some(&1024));
        assert_eq!(meminfo.get("DirectMap2M"), Some(&2_000_000));
        assert_eq!(meminfo.get("SomeValue"), Some(&3_000_000));
    }

    #[test]
    fn test_build_memory_facts() {
        let mut meminfo = HashMap::new();
        meminfo.insert("MemTotal".to_string(), 16777216);
        meminfo.insert("SwapTotal".to_string(), 8388608);
        let facts = build_memory_facts(meminfo);
        assert_eq!(facts.get("real").unwrap().total_bytes, 16777216);
        assert_eq!(facts.get("swap").unwrap().total_bytes, 8388608);
    }

    #[test]
    fn test_parse_vm_swapusage_megabytes() {
        let s = "total = 2048.00M  used = 0.00M  free = 2048.00M  (encrypted)";
        assert_eq!(parse_vm_swapusage(s).unwrap(), 2048 * 1024 * 1024);
    }

    #[test]
    fn test_parse_vm_swapusage_gigabytes() {
        let s = "total = 2.00G  used = 0.00G  free = 2.00G  (encrypted)";
        assert_eq!(parse_vm_swapusage(s).unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_vm_swapusage_no_swap() {
        let s = "total = 0.00B  used = 0.00B  free = 0.00B";
        assert_eq!(parse_vm_swapusage(s).unwrap(), 0);
    }

    #[test]
    fn test_parse_vm_swapusage_kilobytes() {
        let s = "total = 1024.00K  used = 0.00K  free = 1024.00K";
        assert_eq!(parse_vm_swapusage(s).unwrap(), 1024 * 1024);
    }

    #[test]
    fn test_parse_memory_facts_windows() {
        let s = "17179869184\n4294967296\n";
        let facts = parse_memory_facts_windows(s).unwrap();
        assert_eq!(facts.get("real").unwrap().total_bytes, 17179869184);
        assert_eq!(facts.get("swap").unwrap().total_bytes, 4294967296);
    }
}
