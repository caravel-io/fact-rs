use crate::filesystem::slurp;

use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, Debug)]
pub struct MemoryType {
    pub total_bytes: u64,
}

pub struct MemoryComponent;

impl MemoryComponent {
    pub fn new() -> Self {
        Self
    }
}
impl Collector for MemoryComponent {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let contents = slurp(Path::new("/proc/meminfo")).context("failed to read file")?;
        let meminfo = parse_meminfo(&contents);
        let facts = build_memory_facts(meminfo);
        let j = to_value(facts).context("serializing to json value")?;
        Ok(j)
    }
}

fn parse_meminfo(contents: &str) -> HashMap<String, u64> {
    contents
        .lines()
        .filter_map(|line| {
            let (label, rest) = line.split_once(':')?;
            let mut parts = rest.split_whitespace();
            let value = parts.next()?.parse::<u64>().ok()?;
            let multiplier = match parts.next() {
                Some("kB") => 1024,
                Some("mB") => 1_000_000,
                Some("MB") => 1_000_000,
                Some("B") | None => 1,
                _ => 1,
            };
            Some((label.to_string(), value * multiplier))
        })
        .collect()
}

fn build_memory_facts(meminfo: HashMap<String, u64>) -> HashMap<String, MemoryType> {
    let mut sections: HashMap<String, MemoryType> = HashMap::new();

    for (key, total_bytes) in meminfo.iter() {
        match key.as_str() {
            "MemTotal" => sections.insert(
                "real".to_string(),
                MemoryType {
                    total_bytes: *total_bytes,
                },
            ),
            "SwapTotal" => sections.insert(
                "swap".to_string(),
                MemoryType {
                    total_bytes: *total_bytes,
                },
            ),
            _ => continue,
        };
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_parse_meminfo() {
        let meminfo_content = "MemTotal:       16384 kB\nSwapTotal:      8192 kB\n";
        let meminfo = parse_meminfo(meminfo_content);
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
        let memory_facts = build_memory_facts(meminfo);
        assert_eq!(memory_facts.get("real").unwrap().total_bytes, 16777216);
        assert_eq!(memory_facts.get("swap").unwrap().total_bytes, 8388608);
    }
}
