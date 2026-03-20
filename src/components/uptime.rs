use std::path::Path;

use crate::filesystem::slurp;

use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;

#[derive(Serialize, Debug)]
pub struct UptimeFacts {
    pub seconds: u64,
}

pub struct UptimeComponent;

impl UptimeComponent {
    pub fn new() -> Self {
        Self
    }
}

impl Collector for UptimeComponent {
    fn name(&self) -> &'static str {
        "uptime"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let uptime_line = get_uptime_line()?;
        let uptime_int = parse_uptime(&uptime_line)?;
        let uf = UptimeFacts {
            seconds: uptime_int,
        };
        Ok(to_value(uf)?)
    }
}

fn get_uptime_line() -> Result<String> {
    slurp(Path::new("/proc/uptime")).context("failed to read uptime")
}

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
