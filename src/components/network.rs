use crate::Collector;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::to_value;
use std::collections::HashMap;

#[derive(Serialize, Debug)]
pub struct NetworkFacts {
    pub hostname: String,
    pub domain: Option<String>,
    pub fqdn: Option<String>,
    pub primary: Option<String>,
    pub ip: Option<String>,
    pub ip6: Option<String>,
    pub mac: Option<String>,
    pub mtu: Option<u32>,
    pub interfaces: HashMap<String, Interface>,
}

#[derive(Serialize, Debug)]
pub struct Interface {
    pub name: String,
    pub ip: Option<String>,
    pub prefix: Option<u32>,
    pub ip6: Option<String>,
    pub prefix6: Option<u32>,
    pub mtu: Option<u32>,
    pub mac: Option<String>,
    pub operational_state: String,
    pub link_type: String,
}

#[derive(Default)]
pub struct NetworkComponent;

impl NetworkComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Collector for NetworkComponent {
    fn name(&self) -> &'static str {
        "network"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let facts = get_network_facts()?;
        Ok(to_value(facts)?)
    }
}

// --- Shared pure functions ---

fn parse_domain(s: &str) -> Option<String> {
    if s.is_empty() || s == "(none)" {
        return None;
    }
    Some(s.to_string())
}

fn build_fqdn(hostname: &str, domain: &Option<String>) -> Option<String> {
    domain.as_ref().map(|d| format!("{hostname}.{d}"))
}

// --- Linux ---

#[cfg(any(target_os = "linux", test))]
use serde::Deserialize;

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Deserialize)]
struct IPDevice {
    ifname: String,
    mtu: u32,
    operstate: String,
    link_type: String,
    address: String,
    addr_info: Vec<AddrInfo>,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Deserialize)]
struct AddrInfo {
    family: String,
    local: String,
    prefixlen: u32,
    scope: String,
}

#[cfg(any(target_os = "linux", test))]
fn extract_addrs(
    addr_infos: &[AddrInfo],
) -> (Option<String>, Option<u32>, Option<String>, Option<u32>) {
    let mut ip = None;
    let mut prefix = None;
    let mut ip6 = None;
    let mut prefix6 = None;
    for a in addr_infos {
        if a.scope == "link" {
            continue;
        }
        if a.family == "inet" {
            ip = Some(a.local.clone());
            prefix = Some(a.prefixlen);
        } else if a.family == "inet6" {
            ip6 = Some(a.local.clone());
            prefix6 = Some(a.prefixlen);
        }
    }
    (ip, prefix, ip6, prefix6)
}

#[cfg(target_os = "linux")]
fn get_network_facts() -> Result<NetworkFacts> {
    use crate::filesystem::slurp;
    use std::path::Path;

    let hostname = slurp(Path::new("/proc/sys/kernel/hostname"))?;
    let domain_str = slurp(Path::new("/proc/sys/kernel/domainname"))?;
    let domain = parse_domain(&domain_str);
    let fqdn = build_fqdn(&hostname, &domain);

    let ip_devices_output = get_all_ip_devices_output()?;
    let system_devices = parse_ip_devices_output(&ip_devices_output)?;

    let mut interfaces: HashMap<String, Interface> = HashMap::new();
    let mut primary_ifname = None;
    let mut primary_ip = None;
    let mut primary_ip6 = None;
    let mut primary_mac = None;
    let mut primary_mtu = None;
    let mut primary_done = false;

    for device in system_devices {
        let (ip, prefix, ip6, prefix6) = extract_addrs(&device.addr_info);

        if !primary_done && device.link_type == "ether" {
            primary_ifname = Some(device.ifname.clone());
            primary_ip = ip.clone();
            primary_ip6 = ip6.clone();
            primary_mac = Some(device.address.clone());
            primary_mtu = Some(device.mtu);
            primary_done = true;
        }

        interfaces.insert(
            device.ifname.clone(),
            Interface {
                name: device.ifname,
                operational_state: device.operstate,
                mtu: Some(device.mtu),
                mac: Some(device.address),
                link_type: device.link_type,
                ip,
                prefix,
                ip6,
                prefix6,
            },
        );
    }

    Ok(NetworkFacts {
        hostname,
        domain,
        fqdn,
        primary: primary_ifname,
        ip: primary_ip,
        ip6: primary_ip6,
        mac: primary_mac,
        mtu: primary_mtu,
        interfaces,
    })
}

#[cfg(target_os = "linux")]
fn get_all_ip_devices_output() -> Result<String> {
    use std::process::Command;
    let output = Command::new("ip")
        .arg("-j")
        .arg("addr")
        .arg("show")
        .output()
        .context("failed to run ip -j addr show")?
        .stdout;
    Ok(String::from_utf8(output)
        .context("ip addr output is not valid UTF-8")?
        .trim_end()
        .to_string())
}

#[cfg(any(target_os = "linux", test))]
fn parse_ip_devices_output(output: &str) -> Result<Vec<IPDevice>> {
    Ok(serde_json::from_str(output)?)
}

// --- macOS ---

#[cfg(any(target_os = "macos", test))]
struct IfconfigEntry {
    name: String,
    mtu: Option<u32>,
    mac: Option<String>,
    ip: Option<String>,
    prefix: Option<u32>,
    ip6: Option<String>,
    prefix6: Option<u32>,
    link_type: String,
    operational_state: String,
}

#[cfg(any(target_os = "macos", test))]
fn parse_resolv_conf(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line
            .strip_prefix("domain ")
            .or_else(|| line.strip_prefix("search "))
        {
            let domain = rest.split_whitespace().next()?;
            return parse_domain(domain);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn parse_domain_from_resolv_conf() -> Option<String> {
    let content = std::fs::read_to_string("/etc/resolv.conf").ok()?;
    parse_resolv_conf(&content)
}

#[cfg(target_os = "macos")]
fn get_network_facts() -> Result<NetworkFacts> {
    use std::process::Command;

    let hostname = String::from_utf8(
        Command::new("hostname")
            .output()
            .context("failed to run hostname")?
            .stdout,
    )
    .context("hostname output is not valid UTF-8")?
    .trim()
    .to_string();

    let domain = parse_domain_from_resolv_conf();
    let fqdn = build_fqdn(&hostname, &domain);

    let ifconfig_output = String::from_utf8(
        Command::new("ifconfig")
            .output()
            .context("failed to run ifconfig")?
            .stdout,
    )
    .context("ifconfig output is not valid UTF-8")?;

    let entries = parse_ifconfig_output(&ifconfig_output);
    let mut interfaces: HashMap<String, Interface> = HashMap::new();
    let mut primary_ifname = None;
    let mut primary_ip = None;
    let mut primary_ip6 = None;
    let mut primary_mac = None;
    let mut primary_mtu = None;
    let mut primary_done = false;

    for entry in entries {
        if !primary_done && entry.link_type == "ether" && entry.ip.is_some() {
            primary_ifname = Some(entry.name.clone());
            primary_ip = entry.ip.clone();
            primary_ip6 = entry.ip6.clone();
            primary_mac = entry.mac.clone();
            primary_mtu = entry.mtu;
            primary_done = true;
        }
        interfaces.insert(
            entry.name.clone(),
            Interface {
                name: entry.name,
                ip: entry.ip,
                prefix: entry.prefix,
                ip6: entry.ip6,
                prefix6: entry.prefix6,
                mtu: entry.mtu,
                mac: entry.mac,
                operational_state: entry.operational_state,
                link_type: entry.link_type,
            },
        );
    }

    Ok(NetworkFacts {
        hostname,
        domain,
        fqdn,
        primary: primary_ifname,
        ip: primary_ip,
        ip6: primary_ip6,
        mac: primary_mac,
        mtu: primary_mtu,
        interfaces,
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_ifconfig_output(s: &str) -> Vec<IfconfigEntry> {
    // Each interface starts on a non-indented line: "en0: flags=... mtu 1500"
    // Detail lines are tab-indented.
    let mut entries: Vec<IfconfigEntry> = Vec::new();
    let mut current: Option<IfconfigEntry> = None;

    for line in s.lines() {
        if !line.starts_with('\t') && !line.starts_with(' ') {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            let Some((name, rest)) = line.split_once(':') else {
                continue;
            };
            let mtu = rest
                .split_whitespace()
                .skip_while(|&t| t != "mtu")
                .nth(1)
                .and_then(|v| v.parse::<u32>().ok());
            // Determine initial link type and operational state from the flags string
            let link_type = if rest.contains("LOOPBACK") {
                "loopback"
            } else {
                "other"
            }
            .to_string();
            let operational_state = if rest.contains("UP") { "UP" } else { "DOWN" }.to_string();
            current = Some(IfconfigEntry {
                name: name.to_string(),
                mtu,
                mac: None,
                ip: None,
                prefix: None,
                ip6: None,
                prefix6: None,
                link_type,
                operational_state,
            });
        } else if let Some(ref mut entry) = current {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("ether ") {
                entry.mac = rest.split_whitespace().next().map(str::to_string);
                entry.link_type = "ether".to_string();
            } else if let Some(rest) = line.strip_prefix("inet6 ") {
                // Skip link-local (fe80::) and only take the first global address
                let addr = rest.split_whitespace().next().unwrap_or("");
                let addr = addr.split('%').next().unwrap_or(addr);
                if !addr.starts_with("fe80") && entry.ip6.is_none() {
                    entry.ip6 = Some(addr.to_string());
                    entry.prefix6 = rest
                        .split_whitespace()
                        .skip_while(|&t| t != "prefixlen")
                        .nth(1)
                        .and_then(|v| v.parse::<u32>().ok());
                }
            } else if let Some(rest) = line.strip_prefix("inet ") {
                let mut parts = rest.split_whitespace();
                entry.ip = parts.next().map(str::to_string);
                // "netmask 0xffffff00" follows
                entry.prefix = parts
                    .skip_while(|&t| t != "netmask")
                    .nth(1)
                    .and_then(netmask_to_prefix);
            } else if let Some(rest) = line.strip_prefix("status: ") {
                entry.operational_state = if rest.trim() == "active" {
                    "UP"
                } else {
                    "DOWN"
                }
                .to_string();
            }
        }
    }

    if let Some(entry) = current {
        entries.push(entry);
    }

    entries
}

#[cfg(any(target_os = "macos", test))]
fn netmask_to_prefix(mask: &str) -> Option<u32> {
    let n = u32::from_str_radix(mask.trim_start_matches("0x"), 16).ok()?;
    Some(n.count_ones())
}

// --- Windows ---

#[cfg(target_os = "windows")]
fn get_network_facts() -> Result<NetworkFacts> {
    use crate::filesystem::run_powershell;
    let script = concat!(
        "Write-Output (hostname);",
        "Write-Output (Get-CimInstance Win32_ComputerSystem).Domain;",
        "Get-NetAdapter | ForEach-Object {",
        "  $a = $_;",
        "  $i4 = Get-NetIPAddress -InterfaceIndex $a.ifIndex -AddressFamily IPv4",
        "    -ErrorAction SilentlyContinue | Where-Object { $_.PrefixOrigin -ne 'WellKnown' }",
        "    | Select-Object -First 1;",
        "  $i6 = Get-NetIPAddress -InterfaceIndex $a.ifIndex -AddressFamily IPv6",
        "    -ErrorAction SilentlyContinue | Where-Object { $_.IPAddress -notlike 'fe80*' }",
        "    | Select-Object -First 1;",
        "  $lt = if ($a.PhysicalMediaType -eq '802.3') { 'ether' } else { 'other' };",
        "  $st = if ($a.Status -eq 'Up') { 'UP' } else { 'DOWN' };",
        "  Write-Output \"$($a.Name)`t$($a.MacAddress)`t$(if($i4){$i4.IPAddress})`t",
        "    $(if($i4){$i4.PrefixLength})`t$(if($i6){$i6.IPAddress})`t",
        "    $(if($i6){$i6.PrefixLength})`t$($a.MtuSize)`t$st`t$lt\"",
        "}",
    );
    parse_network_output_windows(&run_powershell(script)?)
}

#[cfg(any(target_os = "windows", test))]
fn parse_network_output_windows(s: &str) -> Result<NetworkFacts> {
    let mut lines = s.lines();
    let hostname = lines.next().context("missing hostname")?.trim().to_string();
    let domain = parse_domain(lines.next().context("missing domain")?.trim());
    let fqdn = build_fqdn(&hostname, &domain);

    let mut interfaces: HashMap<String, Interface> = HashMap::new();
    let mut primary_ifname = None;
    let mut primary_ip = None;
    let mut primary_ip6 = None;
    let mut primary_mac = None;
    let mut primary_mtu = None;
    let mut primary_done = false;

    for line in lines {
        let mut parts = line.splitn(9, '\t');
        let name = parts.next().unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        let mac = opt_str(parts.next().unwrap_or("").trim());
        let ip = opt_str(parts.next().unwrap_or("").trim());
        let prefix = parts.next().unwrap_or("").trim().parse::<u32>().ok();
        let ip6 = opt_str(parts.next().unwrap_or("").trim());
        let prefix6 = parts.next().unwrap_or("").trim().parse::<u32>().ok();
        let mtu = parts.next().unwrap_or("").trim().parse::<u32>().ok();
        let operational_state = parts.next().unwrap_or("").trim().to_string();
        let link_type = parts.next().unwrap_or("").trim().to_string();

        if !primary_done && link_type == "ether" {
            primary_ifname = Some(name.clone());
            primary_ip = ip.clone();
            primary_ip6 = ip6.clone();
            primary_mac = mac.clone();
            primary_mtu = mtu;
            primary_done = true;
        }

        interfaces.insert(
            name.clone(),
            Interface {
                name,
                ip,
                prefix,
                ip6,
                prefix6,
                mtu,
                mac,
                operational_state,
                link_type,
            },
        );
    }

    Ok(NetworkFacts {
        hostname,
        domain,
        fqdn,
        primary: primary_ifname,
        ip: primary_ip,
        ip6: primary_ip6,
        mac: primary_mac,
        mtu: primary_mtu,
        interfaces,
    })
}

#[cfg(any(target_os = "windows", test))]
fn opt_str(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

// --- Fallback ---

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_network_facts() -> Result<NetworkFacts> {
    anyhow::bail!("network not implemented on this platform")
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    // parse_domain and build_fqdn are pure — test on all platforms

    #[test]
    fn test_parse_domain_none() {
        assert!(parse_domain("(none)").is_none());
    }

    #[test]
    fn test_parse_domain_empty() {
        assert!(parse_domain("").is_none());
    }

    #[test]
    fn test_parse_domain_valid() {
        assert_eq!(parse_domain("example.com"), Some("example.com".to_string()));
    }

    #[test]
    fn test_build_fqdn_with_domain() {
        assert_eq!(
            build_fqdn("web01", &Some("example.com".to_string())),
            Some("web01.example.com".to_string())
        );
    }

    #[test]
    fn test_build_fqdn_without_domain() {
        assert_eq!(build_fqdn("web01", &None), None);
    }

    // Linux: ip -j addr show JSON parsing

    const SAMPLE_IP_OUTPUT: &str = r#"[
      {
        "ifindex": 1, "ifname": "lo", "flags": ["LOOPBACK","UP","LOWER_UP"],
        "mtu": 65536, "qdisc": "noqueue", "operstate": "UNKNOWN",
        "group": "default", "txqlen": 1000, "link_type": "loopback",
        "address": "00:00:00:00:00:00", "broadcast": "00:00:00:00:00:00",
        "addr_info": [
          {"family":"inet","local":"127.0.0.1","prefixlen":8,"scope":"host",
           "label":"lo","valid_life_time":4294967295,"preferred_life_time":4294967295},
          {"family":"inet6","local":"::1","prefixlen":128,"scope":"host",
           "noprefixroute":true,"valid_life_time":4294967295,"preferred_life_time":4294967295}
        ]
      },
      {
        "ifindex": 2, "ifname": "enp0s1", "flags": ["BROADCAST","MULTICAST","UP","LOWER_UP"],
        "mtu": 1500, "qdisc": "fq_codel", "operstate": "UP",
        "group": "default", "txqlen": 1000, "link_type": "ether",
        "address": "56:a5:fa:dc:80:45", "broadcast": "ff:ff:ff:ff:ff:ff",
        "addr_info": [
          {"family":"inet","local":"192.168.64.8","prefixlen":24,"scope":"global",
           "label":"enp0s1","valid_life_time":3132,"preferred_life_time":3132},
          {"family":"inet6","local":"fd08:b294:739c:b65:54a5:faff:fedc:8045","prefixlen":64,
           "scope":"global","valid_life_time":2591980,"preferred_life_time":604780},
          {"family":"inet6","local":"fe80::54a5:faff:fedc:8045","prefixlen":64,
           "scope":"link","valid_life_time":4294967295,"preferred_life_time":4294967295}
        ]
      }
    ]"#;

    #[test]
    fn test_parse_ip_devices_output() {
        let devices = parse_ip_devices_output(SAMPLE_IP_OUTPUT).unwrap();
        assert_eq!(devices.len(), 2);

        let lo = &devices[0];
        assert_eq!(lo.ifname, "lo");
        assert_eq!(lo.mtu, 65536);
        assert_eq!(lo.link_type, "loopback");

        let eth = &devices[1];
        assert_eq!(eth.ifname, "enp0s1");
        assert_eq!(eth.mtu, 1500);
        assert_eq!(eth.operstate, "UP");
        assert_eq!(eth.address, "56:a5:fa:dc:80:45");
        assert_eq!(eth.addr_info.len(), 3);
        assert_eq!(eth.addr_info[0].local, "192.168.64.8");
        assert_eq!(eth.addr_info[0].prefixlen, 24);
    }

    #[test]
    fn test_extract_addrs() {
        let devices = parse_ip_devices_output(SAMPLE_IP_OUTPUT).unwrap();
        let eth = &devices[1];
        let (ip, prefix, ip6, prefix6) = extract_addrs(&eth.addr_info);
        assert_eq!(ip.as_deref(), Some("192.168.64.8"));
        assert_eq!(prefix, Some(24));
        assert_eq!(
            ip6.as_deref(),
            Some("fd08:b294:739c:b65:54a5:faff:fedc:8045")
        );
        assert_eq!(prefix6, Some(64));

        // loopback scope=host addresses should be included (only scope=link is skipped)
        let lo = &devices[0];
        let (lo_ip, _, lo_ip6, _) = extract_addrs(&lo.addr_info);
        assert_eq!(lo_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(lo_ip6.as_deref(), Some("::1"));
    }

    // macOS: ifconfig text parsing

    #[test]
    fn test_netmask_to_prefix() {
        assert_eq!(netmask_to_prefix("0xffffff00"), Some(24));
        assert_eq!(netmask_to_prefix("0xffff0000"), Some(16));
        assert_eq!(netmask_to_prefix("0xff000000"), Some(8));
        assert_eq!(netmask_to_prefix("0xffffffff"), Some(32));
    }

    #[test]
    fn test_parse_ifconfig_output() {
        let input = "\
lo0: flags=8049<LOOPBACK,UP,LOWER_UP> mtu 16384
\tinet 127.0.0.1 netmask 0xff000000
\tinet6 ::1 prefixlen 128
\tinet6 fe80::1%lo0 prefixlen 64 scopeid 0x1
en0: flags=8863<UP,BROADCAST,MULTICAST> mtu 1500
\tether 70:88:6b:8a:1b:3c
\tinet 192.168.1.5 netmask 0xffffff00 broadcast 192.168.1.255
\tinet6 fe80::7288:6bff:fe8a:1b3c%en0 prefixlen 64 scopeid 0x6
\tinet6 2001:db8::1 prefixlen 64
\tstatus: active
";
        let entries = parse_ifconfig_output(input);
        assert_eq!(entries.len(), 2);

        let lo = &entries[0];
        assert_eq!(lo.name, "lo0");
        assert_eq!(lo.link_type, "loopback");
        assert_eq!(lo.mtu, Some(16384));
        assert_eq!(lo.ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(lo.prefix, Some(8));
        assert_eq!(lo.ip6.as_deref(), Some("::1"));
        assert_eq!(lo.prefix6, Some(128));

        let en = &entries[1];
        assert_eq!(en.name, "en0");
        assert_eq!(en.link_type, "ether");
        assert_eq!(en.mac.as_deref(), Some("70:88:6b:8a:1b:3c"));
        assert_eq!(en.ip.as_deref(), Some("192.168.1.5"));
        assert_eq!(en.prefix, Some(24));
        // fe80 should be skipped; 2001:db8::1 is the global address
        assert_eq!(en.ip6.as_deref(), Some("2001:db8::1"));
        assert_eq!(en.prefix6, Some(64));
        assert_eq!(en.operational_state, "UP");
    }

    // Windows: tab-separated output parsing

    #[test]
    fn test_parse_resolv_conf_domain() {
        let input = "nameserver 8.8.8.8\ndomain example.com\n";
        assert_eq!(parse_resolv_conf(input), Some("example.com".to_string()));
    }

    #[test]
    fn test_parse_resolv_conf_search_takes_first() {
        let input = "nameserver 8.8.8.8\nsearch example.com other.com\n";
        assert_eq!(parse_resolv_conf(input), Some("example.com".to_string()));
    }

    #[test]
    fn test_parse_resolv_conf_no_domain() {
        let input = "nameserver 8.8.8.8\n";
        assert_eq!(parse_resolv_conf(input), None);
    }

    #[test]
    fn test_parse_network_output_windows() {
        let input = "myhost\nexample.com\nEthernet\t00-11-22-33-44-55\t10.0.0.5\t24\t2001:db8::1\t64\t1500\tUP\tether\n";
        let facts = parse_network_output_windows(input).unwrap();
        assert_eq!(facts.hostname, "myhost");
        assert_eq!(facts.domain.as_deref(), Some("example.com"));
        assert_eq!(facts.fqdn.as_deref(), Some("myhost.example.com"));
        assert_eq!(facts.primary.as_deref(), Some("Ethernet"));
        assert_eq!(facts.ip.as_deref(), Some("10.0.0.5"));
        assert_eq!(facts.mtu, Some(1500));
        assert_eq!(facts.interfaces.len(), 1);
    }
}
