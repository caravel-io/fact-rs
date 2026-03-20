use crate::Collector;
use crate::filesystem::slurp;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::to_value;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

// These are the structs used to deserialize from JSON
#[derive(Debug, Deserialize)]
struct IPDevice {
    ifname: String,
    mtu: u32,
    operstate: String,
    link_type: String,
    address: String,
    addr_info: Vec<AddrInfo>,
}

#[derive(Debug, Deserialize)]
struct AddrInfo {
    family: String,
    local: String,
    prefixlen: u32,
    scope: String,
}
// end JSON fields

// These are the structs used to format it how I want them to serialize
#[derive(Debug, Serialize)]
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
// end JSON serialize

pub struct NetworkComponent;
impl NetworkComponent {
    pub fn new() -> Self {
        Self
    }
}

impl Collector for NetworkComponent {
    fn name(&self) -> &'static str {
        "network"
    }

    fn collect(&self) -> Result<serde_json::Value> {
        let hostname = get_hostname()?;
        let domain = get_domain()?;
        let fqdn = build_fqdn(&hostname, &domain);

        let ip_devices_output = get_all_ip_devices_output()?;
        let system_devices = parse_ip_devices_output(&ip_devices_output)?;

        // ip is ordered by ifindex, primary should be first, skipping loopback
        let mut interfaces: HashMap<String, Interface> = HashMap::new();

        // primary device, will be filled out later
        let mut primary_ifname = None;
        let mut primary_ip = None;
        let mut primary_ip6 = None;
        let mut primary_mac = None;
        let mut primary_mtu = None;

        let mut primary_done = false;
        for device in system_devices {
            // properties to be filled out by iterating through the device infos
            let mut ip = None;
            let mut prefix = None;
            let mut ip6 = None;
            let mut prefix6 = None;

            for addr_info in &device.addr_info {
                if addr_info.scope == "link" {
                    // let's not care about link-local addresses for now :)
                    continue;
                }
                if addr_info.family == "inet" {
                    ip = Some(addr_info.local.clone());
                    prefix = Some(addr_info.prefixlen);
                }
                if addr_info.family == "inet6" {
                    ip6 = Some(addr_info.local.clone());
                    prefix6 = Some(addr_info.prefixlen);
                }
            }

            // find the first occurrence of the "ether" device type
            // that will be our primary
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
                    name: device.ifname.clone(),
                    operational_state: device.operstate.clone(),
                    mtu: Some(device.mtu),
                    mac: Some(device.address.clone()),
                    link_type: device.link_type.clone(),
                    ip: ip,
                    prefix: prefix,
                    ip6: ip6,
                    prefix6: prefix6,
                },
            );
        }
        let facts = NetworkFacts {
            hostname,
            domain,
            fqdn,
            primary: primary_ifname,
            ip: primary_ip,
            ip6: primary_ip6,
            mac: primary_mac,
            mtu: primary_mtu,
            interfaces,
        };
        let j = to_value(facts).context("serializing to json value")?;
        Ok(j)
    }
}

fn get_hostname() -> Result<String> {
    slurp(Path::new("/proc/sys/kernel/hostname")).context("failed to read hostname")
}

fn get_domain() -> Result<Option<String>> {
    let domain =
        slurp(Path::new("/proc/sys/kernel/domainname")).context("failed to read domainname")?;
    Ok(parse_domain(&domain))
}

fn parse_domain(s: &str) -> Option<String> {
    if s.is_empty() || s == "(none)" {
        return None;
    }
    Some(s.to_string())
}

fn build_fqdn(hostname: &str, domain: &Option<String>) -> Option<String> {
    return match domain {
        None => None,
        Some(d) => Some(format!("{}.{}", hostname, d)),
    };
}

fn parse_ip_devices_output(output: &str) -> Result<Vec<IPDevice>> {
    let devices: Vec<IPDevice> = serde_json::from_str(&output)?;
    Ok(devices)
}

fn get_all_ip_devices_output() -> Result<String> {
    let output = Command::new("ip")
        .arg("-j")
        .arg("addr")
        .arg("show")
        .output()
        .with_context(|| format!("running ip -j addr show"))?
        .stdout;
    let output = String::from_utf8(output)?;
    Ok(output.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_IP_OUTPUT: &str = r#"[
      {
        "ifindex": 1,
        "ifname": "lo",
        "flags": ["LOOPBACK", "UP", "LOWER_UP"],
        "mtu": 65536,
        "qdisc": "noqueue",
        "operstate": "UNKNOWN",
        "group": "default",
        "txqlen": 1000,
        "link_type": "loopback",
        "address": "00:00:00:00:00:00",
        "broadcast": "00:00:00:00:00:00",
        "addr_info": [
          {"family": "inet", "local": "127.0.0.1", "prefixlen": 8, "scope": "host",
           "label": "lo", "valid_life_time": 4294967295, "preferred_life_time": 4294967295},
          {"family": "inet6", "local": "::1", "prefixlen": 128, "scope": "host",
           "noprefixroute": true, "valid_life_time": 4294967295, "preferred_life_time": 4294967295}
        ]
      },
      {
        "ifindex": 2,
        "ifname": "enp0s1",
        "flags": ["BROADCAST", "MULTICAST", "UP", "LOWER_UP"],
        "mtu": 1500,
        "qdisc": "fq_codel",
        "operstate": "UP",
        "group": "default",
        "txqlen": 1000,
        "link_type": "ether",
        "address": "56:a5:fa:dc:80:45",
        "broadcast": "ff:ff:ff:ff:ff:ff",
        "addr_info": [
          {"family": "inet", "local": "192.168.64.8", "prefixlen": 24, "scope": "global",
           "label": "enp0s1", "valid_life_time": 3132, "preferred_life_time": 3132},
          {"family": "inet6", "local": "fd08:b294:739c:b65:54a5:faff:fedc:8045", "prefixlen": 64,
           "scope": "global", "valid_life_time": 2591980, "preferred_life_time": 604780},
          {"family": "inet6", "local": "fe80::54a5:faff:fedc:8045", "prefixlen": 64,
           "scope": "link", "valid_life_time": 4294967295, "preferred_life_time": 4294967295}
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
        assert_eq!(lo.operstate, "UNKNOWN");
        assert_eq!(lo.link_type, "loopback");
        assert_eq!(lo.addr_info.len(), 2);

        let eth = &devices[1];
        assert_eq!(eth.ifname, "enp0s1");
        assert_eq!(eth.mtu, 1500);
        assert_eq!(eth.operstate, "UP");
        assert_eq!(eth.link_type, "ether");
        assert_eq!(eth.address, "56:a5:fa:dc:80:45");
        assert_eq!(eth.addr_info.len(), 3);
        assert_eq!(eth.addr_info[0].local, "192.168.64.8");
        assert_eq!(eth.addr_info[0].prefixlen, 24);
    }

    const SAMPLE_IP_OUTPUT_NO_IPV6: &str = r#"[
      {
        "ifindex": 1,
        "ifname": "lo",
        "flags": ["LOOPBACK", "UP", "LOWER_UP"],
        "mtu": 65536,
        "qdisc": "noqueue",
        "operstate": "UNKNOWN",
        "group": "default",
        "txqlen": 1000,
        "link_type": "loopback",
        "address": "00:00:00:00:00:00",
        "broadcast": "00:00:00:00:00:00",
        "addr_info": [
          {"family": "inet", "local": "127.0.0.1", "prefixlen": 8, "scope": "host",
           "label": "lo", "valid_life_time": 4294967295, "preferred_life_time": 4294967295}
        ]
      },
      {
        "ifindex": 2,
        "ifname": "enp0s1",
        "flags": ["BROADCAST", "MULTICAST", "UP", "LOWER_UP"],
        "mtu": 1500,
        "qdisc": "fq_codel",
        "operstate": "UP",
        "group": "default",
        "txqlen": 1000,
        "link_type": "ether",
        "address": "56:a5:fa:dc:80:45",
        "broadcast": "ff:ff:ff:ff:ff:ff",
        "addr_info": [
          {"family": "inet", "local": "192.168.64.8", "prefixlen": 24, "scope": "global",
           "label": "enp0s1", "valid_life_time": 3132, "preferred_life_time": 3132}
        ]
      }
    ]"#;

    #[test]
    fn test_parse_ip_devices_output_no_ipv6() {
        let devices = parse_ip_devices_output(SAMPLE_IP_OUTPUT_NO_IPV6).unwrap();
        assert_eq!(devices.len(), 2);

        let lo = &devices[0];
        assert_eq!(lo.addr_info.len(), 1);
        assert_eq!(lo.addr_info[0].family, "inet");

        let eth = &devices[1];
        assert_eq!(eth.addr_info.len(), 1);
        assert_eq!(eth.addr_info[0].family, "inet");
        assert_eq!(eth.addr_info[0].local, "192.168.64.8");
    }

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
        let fqdn = build_fqdn("web01", &Some("example.com".to_string()));
        assert_eq!(fqdn, Some("web01.example.com".to_string()));
    }

    #[test]
    fn test_build_fqdn_without_domain() {
        let fqdn = build_fqdn("web01", &None);
        assert_eq!(fqdn, None);
    }
}
