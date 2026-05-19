use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;
use std::collections::HashMap;

#[derive(Serialize, Debug, Clone)]
pub struct MountPoint {
    pub location: String,
    pub device: String,
    pub filesystem: String,
    pub options: Vec<String>,
}

#[derive(Default)]
pub struct MountComponent;

impl MountComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Collector for MountComponent {
    fn name(&self) -> &'static str {
        "mounts"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let facts = get_mount_facts()?;
        Ok(to_value(facts)?)
    }
}

// --- Shared ---

fn build_mount_facts(mounts: Vec<MountPoint>) -> HashMap<String, MountPoint> {
    mounts.into_iter().map(|m| (m.location.clone(), m)).collect()
}

// --- Linux ---

#[cfg(target_os = "linux")]
fn get_mount_facts() -> Result<HashMap<String, MountPoint>> {
    use crate::filesystem::slurp;
    use std::path::Path;
    let contents = slurp(Path::new("/proc/mounts"))?;
    Ok(build_mount_facts(parse_mounts(&contents)))
}

#[cfg(any(target_os = "linux", test))]
fn parse_mounts(contents: &str) -> Vec<MountPoint> {
    contents
        .lines()
        .filter_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<&str>>();
            if parts.len() < 4 {
                return None;
            }
            Some(MountPoint {
                location: parts[1].to_string(),
                device: parts[0].to_string(),
                filesystem: parts[2].to_string(),
                options: parts[3].split(',').map(str::to_string).collect(),
            })
        })
        .collect()
}

// --- macOS ---

#[cfg(target_os = "macos")]
fn get_mount_facts() -> Result<HashMap<String, MountPoint>> {
    let output = std::process::Command::new("mount")
        .output()
        .context("failed to run mount")?;
    Ok(build_mount_facts(parse_mount_output_macos(
        &String::from_utf8(output.stdout).context("mount output is not valid UTF-8")?,
    )))
}

#[cfg(any(target_os = "macos", test))]
fn parse_mount_output_macos(s: &str) -> Vec<MountPoint> {
    // Format: "{device} on {mountpoint} ({fstype}, {option}, ...)"
    s.lines()
        .filter_map(|line| {
            let (device, rest) = line.split_once(" on ")?;
            let (mountpoint, opts_str) = rest.split_once(" (")?;
            let opts_str = opts_str.trim_end_matches(')');
            let mut opts = opts_str.split(", ");
            let filesystem = opts.next()?.to_string();
            let options = opts.map(str::to_string).collect();
            Some(MountPoint {
                location: mountpoint.to_string(),
                device: device.to_string(),
                filesystem,
                options,
            })
        })
        .collect()
}

// --- Windows ---

#[cfg(target_os = "windows")]
fn get_mount_facts() -> Result<HashMap<String, MountPoint>> {
    use crate::filesystem::run_powershell;
    let script = concat!(
        "Get-CimInstance Win32_LogicalDisk | ForEach-Object {",
        "  $t = switch ($_.DriveType) {",
        "    2 { 'removable' } 3 { 'local' } 4 { 'network' } 5 { 'cd-rom' } default { 'unknown' }",
        "  };",
        "  \"$($_.DeviceID)`t$($_.FileSystem)`t$t\"",
        "}",
    );
    Ok(build_mount_facts(parse_mounts_windows(&run_powershell(script)?)))
}

#[cfg(any(target_os = "windows", test))]
fn parse_mounts_windows(s: &str) -> Vec<MountPoint> {
    // Format per line: "{DeviceID}\t{FileSystem}\t{DriveType}"
    s.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let device_id = parts.next()?.trim().to_string();
            let filesystem = parts.next()?.trim().to_string();
            let drive_type = parts.next()?.trim().to_string();
            Some(MountPoint {
                location: format!("{}\\", device_id),
                device: device_id,
                filesystem,
                options: vec![drive_type],
            })
        })
        .collect()
}

// --- Fallback ---

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_mount_facts() -> Result<HashMap<String, MountPoint>> {
    anyhow::bail!("mounts not implemented on this platform")
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mounts() {
        let contents = "/dev/sda1 / xfs rw,relatime 0 0\ntmpfs /tmp tmpfs rw,nosuid,nodev 0 0\n";
        let mounts = parse_mounts(contents);
        assert_eq!(mounts.len(), 2);

        let root = mounts.iter().find(|m| m.location == "/").unwrap();
        assert_eq!(root.device, "/dev/sda1");
        assert_eq!(root.filesystem, "xfs");
        assert_eq!(root.options, vec!["rw", "relatime"]);

        let tmp = mounts.iter().find(|m| m.location == "/tmp").unwrap();
        assert_eq!(tmp.device, "tmpfs");
        assert_eq!(tmp.filesystem, "tmpfs");
        assert_eq!(tmp.options, vec!["rw", "nosuid", "nodev"]);
    }

    #[test]
    fn test_parse_mounts_skips_short_lines() {
        let contents = "/dev/sda1 /\nvalid /mnt xfs rw 0 0\n";
        let mounts = parse_mounts(contents);
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].location, "/mnt");
    }

    #[test]
    fn test_build_mount_facts() {
        let mounts = vec![
            MountPoint {
                location: "/".to_string(),
                device: "/dev/sda1".to_string(),
                filesystem: "xfs".to_string(),
                options: vec!["rw".to_string()],
            },
            MountPoint {
                location: "/tmp".to_string(),
                device: "tmpfs".to_string(),
                filesystem: "tmpfs".to_string(),
                options: vec!["rw".to_string()],
            },
        ];
        let facts = build_mount_facts(mounts);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts["/"].device, "/dev/sda1");
        assert_eq!(facts["/tmp"].filesystem, "tmpfs");
    }

    #[test]
    fn test_parse_mount_output_macos() {
        let input = "/dev/disk3s1s1 on / (apfs, sealed, local, read-only, journaled)\n\
                     devfs on /dev (devfs, local, nobrowse)\n";
        let mounts = parse_mount_output_macos(input);
        assert_eq!(mounts.len(), 2);

        let root = mounts.iter().find(|m| m.location == "/").unwrap();
        assert_eq!(root.device, "/dev/disk3s1s1");
        assert_eq!(root.filesystem, "apfs");
        assert_eq!(root.options, vec!["sealed", "local", "read-only", "journaled"]);

        let dev = mounts.iter().find(|m| m.location == "/dev").unwrap();
        assert_eq!(dev.device, "devfs");
        assert_eq!(dev.filesystem, "devfs");
        assert_eq!(dev.options, vec!["local", "nobrowse"]);
    }

    #[test]
    fn test_parse_mounts_windows() {
        let input = "C:\tNTFS\tlocal\nD:\tNTFS\tlocal\nZ:\tNTFS\tnetwork\n";
        let mounts = parse_mounts_windows(input);
        assert_eq!(mounts.len(), 3);

        let c = mounts.iter().find(|m| m.device == "C:").unwrap();
        assert_eq!(c.location, "C:\\");
        assert_eq!(c.filesystem, "NTFS");
        assert_eq!(c.options, vec!["local"]);

        let z = mounts.iter().find(|m| m.device == "Z:").unwrap();
        assert_eq!(z.options, vec!["network"]);
    }
}
