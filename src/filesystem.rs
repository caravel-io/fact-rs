use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Reads a file into a string
///
/// # Example:
///
/// let contents = slurp(Path::new("/proc/sys/kernel/hostname"));
/// assert_eq(contents, "myhostname");
///
pub fn slurp(path: impl AsRef<Path>) -> Result<String> {
    std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.as_ref().display()))
        .map(|s| s.trim().to_string())
}

#[cfg(target_os = "macos")]
pub fn sysctl_n(key: &str) -> Result<String> {
    let output = std::process::Command::new("sysctl")
        .arg("-n")
        .arg(key)
        .output()
        .with_context(|| format!("failed to run sysctl -n {key}"))?;
    Ok(String::from_utf8(output.stdout)
        .context("sysctl output is not valid UTF-8")?
        .trim()
        .to_string())
}

#[cfg(target_os = "windows")]
pub fn run_powershell(script: &str) -> Result<String> {
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .context("failed to run powershell")?;
    String::from_utf8(output.stdout).context("powershell output is not valid UTF-8")
}

#[allow(dead_code)]
pub fn get_dirs_in_path(path: PathBuf) -> Result<Vec<String>> {
    let mut dirs = Vec::new();
    let entries = std::fs::read_dir(&path)
        .with_context(|| format!("listing contents of {}", path.display()))?;
    for entry in entries {
        if let Ok(entry) = entry {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        dirs.push(name.to_string());
                    }
                }
            }
        }
    }
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::Write};
    use tempfile::tempdir;

    #[test]
    fn test_slurp() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let file1path = root.join("test_slurp1");
        let mut file1 = File::create(file1path.clone()).unwrap();
        let _ = file1.write_all(b"testcontent1");
        let content = slurp(file1path.clone()).unwrap();
        assert_eq!(content, "testcontent1");

        let file2path = root.join("test_slurp2");
        let mut file2 = File::create(file2path.clone()).unwrap();
        let _ = file2.write_all(b"1\\^%(*!#_)FA");
        let content = slurp(file2path.clone()).unwrap();
        assert_eq!(content, "1\\^%(*!#_)FA");
    }
}
