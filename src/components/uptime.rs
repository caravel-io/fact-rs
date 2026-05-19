use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;

#[derive(Serialize, Debug)]
pub struct UptimeFacts {
    pub seconds: u64,
}

#[derive(Default)]
pub struct UptimeComponent;

impl UptimeComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Collector for UptimeComponent {
    fn name(&self) -> &'static str {
        "uptime"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let seconds = get_uptime_seconds()?;
        Ok(to_value(UptimeFacts { seconds })?)
    }
}

#[cfg(target_os = "linux")]
fn get_uptime_seconds() -> Result<u64> {
    use crate::filesystem::slurp;
    use std::path::Path;
    let line = slurp(Path::new("/proc/uptime"))?;
    parse_uptime(&line)
}

#[cfg(target_os = "macos")]
fn get_uptime_seconds() -> Result<u64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = crate::filesystem::sysctl_n("kern.boottime")?;
    let boot_sec = parse_boottime(&s)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before unix epoch")?
        .as_secs();
    Ok(now.saturating_sub(boot_sec))
}

#[cfg(any(target_os = "macos", test))]
fn parse_boottime(s: &str) -> Result<u64> {
    // "{ sec = 1747602826, usec = 0 } Mon May 19 ..."
    s.split_whitespace()
        .skip_while(|&t| t != "sec")
        .nth(2)
        .ok_or_else(|| anyhow::anyhow!("malformed kern.boottime output"))?
        .trim_end_matches(',')
        .parse::<u64>()
        .context("failed to parse sec from kern.boottime")
}

#[cfg(target_os = "windows")]
fn get_uptime_seconds() -> Result<u64> {
    let s = crate::filesystem::run_powershell(
        "[uint64](New-TimeSpan -Start (gcim Win32_OperatingSystem).LastBootUpTime).TotalSeconds",
    )?;
    s.trim().parse::<u64>().context("failed to parse uptime from powershell output")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_uptime_seconds() -> Result<u64> {
    anyhow::bail!("uptime not implemented on this platform")
}

#[cfg(any(target_os = "linux", test))]
fn parse_uptime(line: &str) -> Result<u64> {
    line.split_whitespace()
        .next()
        .context("uptime is empty")?
        .split('.')
        .next()
        .context("could not parse uptime")?
        .parse::<u64>()
        .context("uptime was not a number")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_boottime() {
        let s = "{ sec = 1747602826, usec = 0 } Mon May 19 05:33:46 2025\n";
        assert_eq!(parse_boottime(s).unwrap(), 1747602826);
    }

    #[test]
    fn test_parse_boottime_with_usec() {
        let s = "{ sec = 1716000000, usec = 123456 } Fri May 17 00:00:00 2024\n";
        assert_eq!(parse_boottime(s).unwrap(), 1716000000);
    }

    #[test]
    fn test_parse_boottime_malformed() {
        assert!(parse_boottime("no sec field here").is_err());
    }

    #[test]
    fn test_parse_uptime() {
        assert_eq!(parse_uptime("358391.42 1234567.89").unwrap(), 358391);
    }

    #[test]
    fn test_parse_uptime_no_fraction() {
        assert_eq!(parse_uptime("358391 1234567.89").unwrap(), 358391);
    }

    #[test]
    fn test_parse_uptime_empty() {
        assert!(parse_uptime("").is_err());
    }
}
