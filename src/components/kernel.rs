use crate::Collector;
use anyhow::Result;
#[cfg(any(target_os = "windows", test))]
use anyhow::Context;

#[cfg(any(target_os = "windows", test))]
fn parse_kernel_facts_windows(s: &str) -> Result<KernelFacts> {
    let mut lines = s.lines();
    let ostype = lines.next().context("missing ostype")?.trim().to_string();
    let arch = lines.next().context("missing arch")?.trim().to_string();
    let release = lines.next().context("missing release")?.trim().to_string();
    let version = parse_version(&release);
    let majorversion = parse_majorversion(&release);
    Ok(KernelFacts { ostype, arch, release, version, majorversion })
}
use serde::Serialize;
use serde_json::to_value;

#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[derive(Serialize, Debug)]
pub struct KernelFacts {
    pub ostype: String,
    pub arch: String,
    pub release: String,
    pub version: String,
    pub majorversion: String,
}

pub struct KernelComponent {
    #[cfg(target_os = "linux")]
    fsroot: PathBuf,
}

impl KernelComponent {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            fsroot: PathBuf::from("/"),
        }
    }

    #[cfg(all(target_os = "linux", test))]
    pub fn with_root(fsroot: PathBuf) -> Self {
        Self { fsroot }
    }
}

impl Collector for KernelComponent {
    fn name(&self) -> &'static str {
        "kernel"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let kf = self.get_kernel_info()?;
        Ok(to_value(kf)?)
    }
}

// --- Linux ---

#[cfg(target_os = "linux")]
impl KernelComponent {
    fn get_kernel_info(&self) -> Result<KernelFacts> {
        use crate::filesystem::slurp;
        let fsroot = &self.fsroot;
        let ostype = slurp(fsroot.join("proc/sys/kernel/ostype"))?;
        let arch = slurp(fsroot.join("proc/sys/kernel/arch"))?;
        let release = slurp(fsroot.join("proc/sys/kernel/osrelease"))?;
        let version = parse_version(&release);
        let majorversion = parse_majorversion(&release);
        Ok(KernelFacts { ostype, arch, release, majorversion, version })
    }
}

// --- macOS ---

#[cfg(target_os = "macos")]
impl KernelComponent {
    fn get_kernel_info(&self) -> Result<KernelFacts> {
        use crate::filesystem::sysctl_n;
        let ostype = sysctl_n("kern.ostype")?;
        let arch = sysctl_n("hw.machine")?;
        let release = sysctl_n("kern.osrelease")?;
        let version = parse_version(&release);
        let majorversion = parse_majorversion(&release);
        Ok(KernelFacts { ostype, arch, release, version, majorversion })
    }
}

// --- Windows ---

#[cfg(target_os = "windows")]
impl KernelComponent {
    fn get_kernel_info(&self) -> Result<KernelFacts> {
        use crate::filesystem::run_powershell;
        let script = concat!(
            "Write-Output 'Windows';",
            "Write-Output $env:PROCESSOR_ARCHITECTURE;",
            "Write-Output (Get-CimInstance Win32_OperatingSystem).Version",
        );
        parse_kernel_facts_windows(&run_powershell(script)?)
    }
}

// --- Fallback ---

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl KernelComponent {
    fn get_kernel_info(&self) -> Result<KernelFacts> {
        anyhow::bail!("kernel not implemented on this platform")
    }
}

// --- Shared parse logic ---

fn parse_version(release: &str) -> String {
    release
        .split_once('-')
        .map(|(v, _)| v)
        .unwrap_or(release)
        .to_string()
}

fn parse_majorversion(release: &str) -> String {
    release.splitn(3, '.').take(2).collect::<Vec<_>>().join(".")
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kernel_facts_windows() {
        let input = "Windows\nAMD64\n10.0.22631\n";
        let facts = parse_kernel_facts_windows(input).unwrap();
        assert_eq!(facts.ostype, "Windows");
        assert_eq!(facts.arch, "AMD64");
        assert_eq!(facts.release, "10.0.22631");
        assert_eq!(facts.version, "10.0.22631");
        assert_eq!(facts.majorversion, "10.0");
    }

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("5.15.0-48-generic"), "5.15.0");
        assert_eq!(parse_version("5.15.0-48"), "5.15.0");
        assert_eq!(parse_version("5.15.0"), "5.15.0");
        assert_eq!(parse_version("5.15"), "5.15");
        assert_eq!(parse_version("5"), "5");
    }

    #[test]
    fn test_parse_majorversion() {
        assert_eq!(parse_majorversion("5.15.0-48-generic"), "5.15");
        assert_eq!(parse_majorversion("5.15.0-48"), "5.15");
        assert_eq!(parse_majorversion("5.15.0"), "5.15");
        assert_eq!(parse_majorversion("5.15"), "5.15");
        assert_eq!(parse_majorversion("5"), "5");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_kernel_component_common() {
        use std::fs;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("proc/sys/kernel")).unwrap();
        fs::write(root.join("proc/sys/kernel/ostype"), "Linux").unwrap();
        fs::write(root.join("proc/sys/kernel/arch"), "x86_64").unwrap();
        fs::write(root.join("proc/sys/kernel/osrelease"), "5.15.0-48-generic").unwrap();

        let kc = KernelComponent::with_root(root.to_path_buf());
        let facts = kc.collect().unwrap();

        assert_eq!(facts["ostype"], "Linux");
        assert_eq!(facts["arch"], "x86_64");
        assert_eq!(facts["release"], "5.15.0-48-generic");
        assert_eq!(facts["version"], "5.15.0");
        assert_eq!(facts["majorversion"], "5.15");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_kernel_component_weird() {
        use std::fs;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("proc/sys/kernel")).unwrap();
        fs::write(root.join("proc/sys/kernel/ostype"), "Linux").unwrap();
        fs::write(root.join("proc/sys/kernel/arch"), "x86_64").unwrap();
        fs::write(root.join("proc/sys/kernel/osrelease"), "5.15.0-48").unwrap();

        let kc = KernelComponent::with_root(root.to_path_buf());
        let facts = kc.collect().unwrap();

        assert_eq!(facts["ostype"], "Linux");
        assert_eq!(facts["arch"], "x86_64");
        assert_eq!(facts["release"], "5.15.0-48");
        assert_eq!(facts["version"], "5.15.0");
        assert_eq!(facts["majorversion"], "5.15");
    }
}
