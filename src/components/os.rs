use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;

#[derive(Serialize, Debug)]
pub struct OSFacts {
    pub pretty_name: Option<String>,
    pub name: Option<String>,
    pub version_id: Option<String>,
    pub version: Option<String>,
    pub codename: Option<String>,
    pub id: Option<String>,
}

#[derive(Default)]
pub struct OSComponent;

impl OSComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Collector for OSComponent {
    fn name(&self) -> &'static str {
        "os"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let facts = get_os_facts()?;
        Ok(to_value(facts)?)
    }
}

// --- Linux ---

#[cfg(target_os = "linux")]
fn get_os_facts() -> Result<OSFacts> {
    use crate::filesystem::slurp;
    use std::path::Path;
    let lines = slurp(Path::new("/etc/os-release"))?
        .lines()
        .map(|s| s.to_string())
        .collect();
    parse_into_facts(lines)
}

#[cfg(any(target_os = "linux", test))]
fn parse_into_facts(lines: Vec<String>) -> Result<OSFacts> {
    let mut pretty_name = None;
    let mut name = None;
    let mut version_id = None;
    let mut version = None;
    let mut codename = None;
    let mut id = None;

    for line in lines {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "PRETTY_NAME" => pretty_name = Some(v.trim_matches('"').to_string()),
            "NAME" => name = Some(v.trim_matches('"').to_string()),
            "VERSION_ID" => version_id = Some(v.trim_matches('"').to_string()),
            "VERSION" => version = Some(v.trim_matches('"').to_string()),
            "VERSION_CODENAME" => codename = Some(v.trim_matches('"').to_string()),
            "ID" => id = Some(v.trim_matches('"').to_string()),
            _ => continue,
        }
    }

    Ok(OSFacts { pretty_name, name, version_id, version, codename, id })
}

// --- macOS ---

#[cfg(target_os = "macos")]
fn get_os_facts() -> Result<OSFacts> {
    let output = std::process::Command::new("sw_vers")
        .output()
        .context("failed to run sw_vers")?;
    parse_sw_vers(&String::from_utf8(output.stdout).context("sw_vers output is not valid UTF-8")?)
}

#[cfg(any(target_os = "macos", test))]
fn parse_sw_vers(s: &str) -> Result<OSFacts> {
    let mut product_name = None;
    let mut product_version = None;

    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "ProductName" => product_name = Some(v.trim().to_string()),
            "ProductVersion" => product_version = Some(v.trim().to_string()),
            _ => continue,
        }
    }

    let name = product_name;
    let version = product_version;
    let pretty_name = match (&name, &version) {
        (Some(n), Some(v)) => Some(format!("{n} {v}")),
        _ => None,
    };

    let codename = version.as_deref().and_then(macos_codename);

    Ok(OSFacts {
        pretty_name,
        version_id: version.clone(),
        version,
        name,
        id: Some("macos".to_string()),
        codename,
    })
}

#[cfg(any(target_os = "macos", test))]
fn macos_codename(version: &str) -> Option<String> {
    static DATA: &str = include_str!("../../data/macos_versions.txt");
    let mut parts = version.split('.');
    let major = parts.next()?;
    let key = if major == "10" {
        let minor = parts.next()?;
        format!("10.{minor}")
    } else {
        major.to_string()
    };
    DATA.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .find_map(|l| {
            let (k, v) = l.split_once(' ')?;
            (k == key).then(|| v.to_string())
        })
}

// --- Windows ---

#[cfg(target_os = "windows")]
fn get_os_facts() -> Result<OSFacts> {
    use crate::filesystem::run_powershell;
    let script = concat!(
        "$os = Get-CimInstance Win32_OperatingSystem;",
        "Write-Output $os.Caption;",
        "Write-Output $os.Version;",
        "Write-Output $os.BuildNumber",
    );
    parse_os_facts_windows(&run_powershell(script)?)
}

#[cfg(any(target_os = "windows", test))]
fn parse_os_facts_windows(s: &str) -> Result<OSFacts> {
    let mut lines = s.lines();
    let pretty_name = lines.next().context("missing Caption")?.trim().to_string();
    let version = lines.next().context("missing Version")?.trim().to_string();
    let version_id = lines.next().context("missing BuildNumber")?.trim().to_string();
    let codename = windows_codename(&version_id);

    Ok(OSFacts {
        pretty_name: Some(pretty_name),
        name: Some("Windows".to_string()),
        version: Some(version),
        version_id: Some(version_id),
        id: Some("windows".to_string()),
        codename,
    })
}

#[cfg(any(target_os = "windows", test))]
fn windows_codename(build: &str) -> Option<String> {
    static DATA: &str = include_str!("../../data/windows_versions.txt");
    DATA.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .find_map(|l| {
            let (k, v) = l.split_once(' ')?;
            (k == build).then(|| v.to_string())
        })
}

// --- Fallback ---

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_os_facts() -> Result<OSFacts> {
    anyhow::bail!("os not implemented on this platform")
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    fn to_lines(s: &str) -> Vec<String> {
        s.lines().map(|l| l.to_string()).collect()
    }

    #[test]
    fn test_parse_into_facts() {
        let input = to_lines(
            r#"PRETTY_NAME="Ubuntu 22.04.3 LTS"
NAME="Ubuntu"
VERSION_ID="22.04"
VERSION="22.04.3 LTS (Jammy Jellyfish)"
VERSION_CODENAME=jammy
ID=ubuntu"#,
        );
        let facts = parse_into_facts(input).unwrap();
        assert_eq!(facts.pretty_name.as_deref(), Some("Ubuntu 22.04.3 LTS"));
        assert_eq!(facts.name.as_deref(), Some("Ubuntu"));
        assert_eq!(facts.version_id.as_deref(), Some("22.04"));
        assert_eq!(facts.codename.as_deref(), Some("jammy"));
        assert_eq!(facts.id.as_deref(), Some("ubuntu"));
    }

    #[test]
    fn test_parse_into_facts_unknown_keys_ignored() {
        let input = to_lines("NAME=\"Ubuntu\"\nUNKNOWN_KEY=somevalue\nID=ubuntu");
        let facts = parse_into_facts(input).unwrap();
        assert_eq!(facts.name.as_deref(), Some("Ubuntu"));
        assert_eq!(facts.id.as_deref(), Some("ubuntu"));
    }

    #[test]
    fn test_parse_into_facts_missing_fields_are_none() {
        let input = to_lines("ID=ubuntu\n");
        let facts = parse_into_facts(input).unwrap();
        assert_eq!(facts.id.as_deref(), Some("ubuntu"));
        assert!(facts.pretty_name.is_none());
        assert!(facts.name.is_none());
        assert!(facts.version_id.is_none());
        assert!(facts.codename.is_none());
    }

    #[test]
    fn test_parse_into_facts_rhel() {
        let input = to_lines(
            r#"NAME="Red Hat Enterprise Linux"
VERSION="8.10 (Ootpa)"
ID="rhel"
VERSION_ID="8.10"
PRETTY_NAME="Red Hat Enterprise Linux 8.10 (Ootpa)"
VERSION_CODENAME=ootpa"#,
        );
        let facts = parse_into_facts(input).unwrap();
        assert_eq!(facts.pretty_name.as_deref(), Some("Red Hat Enterprise Linux 8.10 (Ootpa)"));
        assert_eq!(facts.name.as_deref(), Some("Red Hat Enterprise Linux"));
        assert_eq!(facts.version_id.as_deref(), Some("8.10"));
        assert_eq!(facts.version.as_deref(), Some("8.10 (Ootpa)"));
        assert_eq!(facts.codename.as_deref(), Some("ootpa"));
        assert_eq!(facts.id.as_deref(), Some("rhel"));
    }

    #[test]
    fn test_parse_into_facts_suse() {
        let input = to_lines(
            r#"NAME="SLES"
VERSION="15-SP5"
VERSION_ID="15.5"
PRETTY_NAME="SUSE Linux Enterprise Server 15 SP5"
ID="sles"#,
        );
        let facts = parse_into_facts(input).unwrap();
        assert_eq!(facts.pretty_name.as_deref(), Some("SUSE Linux Enterprise Server 15 SP5"));
        assert_eq!(facts.name.as_deref(), Some("SLES"));
        assert_eq!(facts.version_id.as_deref(), Some("15.5"));
        assert_eq!(facts.version.as_deref(), Some("15-SP5"));
        assert_eq!(facts.id.as_deref(), Some("sles"));
        assert!(facts.codename.is_none());
    }

    #[test]
    fn test_parse_sw_vers() {
        let input = "ProductName:\t\tmacOS\nProductVersion:\t\t15.4.1\nBuildVersion:\t\t24E263\n";
        let facts = parse_sw_vers(input).unwrap();
        assert_eq!(facts.name.as_deref(), Some("macOS"));
        assert_eq!(facts.version.as_deref(), Some("15.4.1"));
        assert_eq!(facts.version_id.as_deref(), Some("15.4.1"));
        assert_eq!(facts.pretty_name.as_deref(), Some("macOS 15.4.1"));
        assert_eq!(facts.id.as_deref(), Some("macos"));
        assert_eq!(facts.codename.as_deref(), Some("Sequoia"));
    }

    #[test]
    fn test_macos_codename() {
        assert_eq!(macos_codename("15.4.1").as_deref(), Some("Sequoia"));
        assert_eq!(macos_codename("26.0").as_deref(), Some("Tahoe"));
        assert_eq!(macos_codename("14.7").as_deref(), Some("Sonoma"));
        assert_eq!(macos_codename("13.0").as_deref(), Some("Ventura"));
        assert_eq!(macos_codename("12.6.1").as_deref(), Some("Monterey"));
        assert_eq!(macos_codename("11.0").as_deref(), Some("Big Sur"));
        assert_eq!(macos_codename("10.15.7").as_deref(), Some("Catalina"));
        assert_eq!(macos_codename("10.9.5").as_deref(), Some("Mavericks"));
        assert_eq!(macos_codename("99.0"), None);
    }

    #[test]
    fn test_parse_os_facts_windows() {
        let input = "Microsoft Windows 11 Pro\n10.0.22631\n22631\n";
        let facts = parse_os_facts_windows(input).unwrap();
        assert_eq!(facts.pretty_name.as_deref(), Some("Microsoft Windows 11 Pro"));
        assert_eq!(facts.name.as_deref(), Some("Windows"));
        assert_eq!(facts.version.as_deref(), Some("10.0.22631"));
        assert_eq!(facts.version_id.as_deref(), Some("22631"));
        assert_eq!(facts.id.as_deref(), Some("windows"));
        assert_eq!(facts.codename.as_deref(), Some("23H2"));
    }

    #[test]
    fn test_windows_codename() {
        assert_eq!(windows_codename("22631").as_deref(), Some("23H2"));
        assert_eq!(windows_codename("26100").as_deref(), Some("24H2"));
        assert_eq!(windows_codename("22621").as_deref(), Some("22H2"));
        assert_eq!(windows_codename("22000").as_deref(), Some("21H2"));
        assert_eq!(windows_codename("19045").as_deref(), Some("22H2"));
        assert_eq!(windows_codename("19041").as_deref(), Some("2004"));
        assert_eq!(windows_codename("99999"), None);
    }
}
