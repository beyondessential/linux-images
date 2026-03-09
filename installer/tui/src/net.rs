use std::fs;
use std::io;
use std::net::UdpSocket;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::config::NetworkMode;
use crate::paths;
use crate::ui::StaticNetConfig;

/// An endpoint to check during the network connectivity screen.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub label: String,
    pub url: String,
    pub expect_200: bool,
}

/// Result of checking a single endpoint.
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub index: usize,
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

/// Status of the overall network check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckPhase {
    NotStarted,
    Running,
    Done,
}

/// Result of the tailscale netcheck command.
#[derive(Debug, Clone)]
pub struct NetcheckResult {
    pub success: bool,
    pub output: String,
}

/// Result of a GitHub SSH key lookup.
#[derive(Debug, Clone)]
pub struct GithubKeysResult {
    pub success: bool,
    pub keys: Vec<String>,
    pub error: Option<String>,
}

/// A detected network interface for the dropdown in the network config screen.
#[derive(Debug, Clone)]
pub struct NetInterface {
    pub name: String,
    #[expect(dead_code, reason = "stored for future interface dropdown display")]
    pub mac: String,
    #[expect(dead_code, reason = "stored for future interface dropdown display")]
    pub state: String,
}

/// Connectivity status for the ISO pane status indicator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetConnectivityStatus {
    Unknown,
    Connected(String),
    NoConnectivity,
    Configuring,
}

/// JSON shape returned by `ip -j link show`.
#[derive(Debug, Deserialize)]
struct IpLinkEntry {
    ifname: String,
    address: Option<String>,
    operstate: Option<String>,
    link_type: Option<String>,
}

/// Detect physical network interfaces by running `ip -j link show` and
/// filtering out virtual/loopback interfaces.
pub fn detect_interfaces() -> Vec<NetInterface> {
    let output = match Command::new(paths::IP)
        .args(["-j", "link", "show"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => {
            tracing::warn!(
                "ip link show failed: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            return Vec::new();
        }
        Err(e) => {
            tracing::warn!("failed to run ip: {e}");
            return Vec::new();
        }
    };

    let entries: Vec<IpLinkEntry> = match serde_json::from_slice(&output) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("failed to parse ip -j link show output: {e}");
            return Vec::new();
        }
    };

    entries
        .into_iter()
        .filter(|e| {
            let name = e.ifname.as_str();
            // Filter out loopback, virtual, and container interfaces
            !matches!(name, "lo")
                && !name.starts_with("docker")
                && !name.starts_with("veth")
                && !name.starts_with("br-")
                && !name.starts_with("tailscale")
                && e.link_type.as_deref() != Some("loopback")
        })
        .map(|e| NetInterface {
            name: e.ifname,
            mac: e.address.unwrap_or_default(),
            state: e.operstate.unwrap_or_else(|| "UNKNOWN".into()),
        })
        .collect()
}

const INSTALLER_NETPLAN_PATH: &str = "/etc/netplan/90-installer.yaml";

/// Apply a netplan configuration for the live ISO environment.
///
/// Writes `/etc/netplan/90-installer.yaml` (or removes it for DHCP) and
/// runs `netplan apply`. Returns `Ok(())` on success.
// r[impl installer.tui.network-config+13]
pub fn apply_netplan(mode: NetworkMode, static_cfg: &StaticNetConfig) -> Result<(), String> {
    match mode {
        NetworkMode::Dhcp => {
            remove_installer_netplan();
            run_netplan_apply()?;
        }
        NetworkMode::StaticIp => {
            let yaml = generate_iso_static_netplan(static_cfg);
            write_installer_netplan(&yaml)?;
            run_netplan_apply()?;
        }
        NetworkMode::Ipv6Slaac => {
            let yaml = generate_iso_ipv6_slaac_netplan(static_cfg);
            write_installer_netplan(&yaml)?;
            run_netplan_apply()?;
        }
        NetworkMode::Offline => {
            let yaml = generate_iso_offline_netplan(static_cfg);
            write_installer_netplan(&yaml)?;
            run_netplan_apply()?;
        }
    }
    Ok(())
}

/// Remove the installer-written netplan file (used when reverting to DHCP).
pub fn remove_installer_netplan() {
    let path = std::path::Path::new(INSTALLER_NETPLAN_PATH);
    if path.exists()
        && let Err(e) = fs::remove_file(path)
    {
        tracing::warn!("failed to remove {INSTALLER_NETPLAN_PATH}: {e}");
    }
}

fn write_installer_netplan(yaml: &str) -> Result<(), String> {
    fs::write(INSTALLER_NETPLAN_PATH, yaml)
        .map_err(|e| format!("writing {INSTALLER_NETPLAN_PATH}: {e}"))?;
    fs::set_permissions(INSTALLER_NETPLAN_PATH, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("setting permissions on {INSTALLER_NETPLAN_PATH}: {e}"))?;
    Ok(())
}

fn run_netplan_apply() -> Result<(), String> {
    let output = Command::new(paths::NETPLAN)
        .arg("apply")
        .output()
        .map_err(|e| format!("running netplan apply: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("netplan apply failed: {stderr}"));
    }
    Ok(())
}

fn generate_iso_static_netplan(cfg: &StaticNetConfig) -> String {
    let iface = if cfg.interface.is_empty() {
        "en*"
    } else {
        &cfg.interface
    };
    let id = if cfg.interface.is_empty() {
        "all-en"
    } else {
        &cfg.interface
    };

    let mut yaml = format!(
        "network:\n\
         \x20 version: 2\n\
         \x20 ethernets:\n\
         \x20   {id}:\n\
         \x20     match:\n\
         \x20       name: \"{iface}\"\n\
         \x20     addresses:\n\
         \x20       - {ip}\n\
         \x20     routes:\n\
         \x20       - to: default\n\
         \x20         via: {gw}\n",
        ip = cfg.ip_cidr,
        gw = cfg.gateway,
    );

    if !cfg.dns.is_empty() {
        let servers: Vec<&str> = cfg.dns.split(',').map(|s| s.trim()).collect();
        yaml.push_str("      nameservers:\n        addresses:\n");
        for server in &servers {
            yaml.push_str(&format!("          - {server}\n"));
        }
        if !cfg.search_domain.is_empty() {
            yaml.push_str(&format!(
                "        search:\n          - {}\n",
                cfg.search_domain
            ));
        }
    }

    yaml
}

fn generate_iso_ipv6_slaac_netplan(cfg: &StaticNetConfig) -> String {
    let iface = if cfg.interface.is_empty() {
        "en*"
    } else {
        &cfg.interface
    };
    let id = if cfg.interface.is_empty() {
        "all-en"
    } else {
        &cfg.interface
    };

    format!(
        "network:\n\
         \x20 version: 2\n\
         \x20 ethernets:\n\
         \x20   {id}:\n\
         \x20     match:\n\
         \x20       name: \"{iface}\"\n\
         \x20     dhcp4: false\n\
         \x20     accept-ra: true\n"
    )
}

fn generate_iso_offline_netplan(cfg: &StaticNetConfig) -> String {
    let iface = if cfg.interface.is_empty() {
        "en*"
    } else {
        &cfg.interface
    };
    let id = if cfg.interface.is_empty() {
        "all-en"
    } else {
        &cfg.interface
    };

    format!(
        "network:\n\
         \x20 version: 2\n\
         \x20 ethernets:\n\
         \x20   {id}:\n\
         \x20     match:\n\
         \x20       name: \"{iface}\"\n\
         \x20     dhcp4: false\n\
         \x20     dhcp6: false\n\
         \x20     optional: true\n"
    )
}

/// Probe basic connectivity by checking for a default route.
///
/// Returns a `NetConnectivityStatus` reflecting the current state.
pub fn probe_connectivity() -> NetConnectivityStatus {
    let output = match Command::new(paths::IP)
        .args(["-j", "route", "show", "default"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return NetConnectivityStatus::NoConnectivity,
    };

    let text = String::from_utf8_lossy(&output);
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return NetConnectivityStatus::NoConnectivity;
    }

    // Try to extract device and source from the JSON output
    #[derive(Deserialize)]
    struct Route {
        dev: Option<String>,
        prefsrc: Option<String>,
    }
    if let Ok(routes) = serde_json::from_str::<Vec<Route>>(trimmed)
        && let Some(r) = routes.first()
    {
        let dev = r.dev.as_deref().unwrap_or("?");
        let src = r.prefsrc.as_deref().unwrap_or("?");
        return NetConnectivityStatus::Connected(format!("{src} on {dev}"));
    }

    NetConnectivityStatus::Connected("default route present".into())
}

// r[impl installer.tui.network-check+6]

/// The list of endpoints to check, matching the spec.
pub fn default_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            label: "ghcr.io".into(),
            url: "https://ghcr.io/".into(),
            expect_200: true,
        },
        Endpoint {
            label: "meta.tamanu.app".into(),
            url: "https://meta.tamanu.app/".into(),
            expect_200: true,
        },
        Endpoint {
            label: "tools.ops.tamanu.io".into(),
            url: "https://tools.ops.tamanu.io/".into(),
            expect_200: false,
        },
        Endpoint {
            label: "clients.ops.tamanu.io".into(),
            url: "https://clients.ops.tamanu.io/".into(),
            expect_200: false,
        },
        Endpoint {
            label: "servers.ops.tamanu.io".into(),
            url: "https://servers.ops.tamanu.io/".into(),
            expect_200: false,
        },
        Endpoint {
            label: "github.com".into(),
            url: "https://github.com/".into(),
            expect_200: false,
        },
    ]
}

const CHECK_TIMEOUT_SECS: u64 = 5;

/// Check a single HTTP(S) endpoint using curl.
///
/// When `expect_200` is true, the check passes only if the HTTP status code
/// is 200. Otherwise any HTTP response (even 403) counts as a pass.
fn check_http(url: &str, expect_200: bool) -> (bool, String) {
    let result = Command::new(paths::CURL)
        .args([
            "--silent",
            "--output",
            "/dev/null",
            "--write-out",
            "%{http_code}",
            "--max-time",
            &CHECK_TIMEOUT_SECS.to_string(),
            "--location",
            url,
        ])
        .output();

    match result {
        Ok(output) => {
            let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if code == "000" || code.is_empty() {
                (false, "connection failed".into())
            } else if expect_200 {
                let passed = code == "200";
                let detail = if passed {
                    "HTTP 200".into()
                } else {
                    format!("HTTP {code} (expected 200)")
                };
                (passed, detail)
            } else {
                (true, format!("HTTP {code}"))
            }
        }
        Err(e) => (false, format!("curl failed: {e}")),
    }
}

/// Check NTP connectivity by sending a minimal SNTP packet to pool.ntp.org
/// over UDP port 123 and waiting for any response.
fn check_ntp() -> (bool, String) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => return (false, format!("bind failed: {e}")),
    };

    if let Err(e) = socket.set_read_timeout(Some(Duration::from_secs(CHECK_TIMEOUT_SECS))) {
        return (false, format!("set timeout: {e}"));
    }

    if let Err(e) = socket.connect("pool.ntp.org:123") {
        return (false, format!("connect failed: {e}"));
    }

    // Minimal SNTPv4 client request: LI=0, VN=4, Mode=3 (client)
    let mut packet = [0u8; 48];
    packet[0] = 0x23; // LI=0, VN=4, Mode=3
    if let Err(e) = socket.send(&packet) {
        return (false, format!("send failed: {e}"));
    }

    let mut buf = [0u8; 48];
    match socket.recv(&mut buf) {
        Ok(_) => (true, "NTP response received".into()),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
            (false, "timeout waiting for NTP response".into())
        }
        Err(e) => (false, format!("recv failed: {e}")),
    }
}

/// Spawn all network checks in parallel and send results back over a channel.
/// Returns a receiver that will yield one `CheckResult` per endpoint plus one
/// for NTP, in arbitrary order.
pub fn spawn_checks(endpoints: &[Endpoint]) -> mpsc::Receiver<CheckResult> {
    let (tx, rx) = mpsc::channel();

    for (i, ep) in endpoints.iter().enumerate() {
        let tx = tx.clone();
        let url = ep.url.clone();
        let label = ep.label.clone();
        let expect_200 = ep.expect_200;
        thread::spawn(move || {
            let (passed, detail) = check_http(&url, expect_200);
            let _ = tx.send(CheckResult {
                index: i,
                label,
                passed,
                detail,
            });
        });
    }

    let ntp_index = endpoints.len();
    let tx_ntp = tx;
    thread::spawn(move || {
        let (passed, detail) = check_ntp();
        let _ = tx_ntp.send(CheckResult {
            index: ntp_index,
            label: "pool.ntp.org:123 (NTP/UDP)".into(),
            passed,
            detail,
        });
    });

    rx
}

/// Total number of checks (HTTP endpoints + NTP).
pub fn total_check_count(endpoints: &[Endpoint]) -> usize {
    endpoints.len() + 1
}

// r[impl installer.tui.tailscale-netcheck+3]

/// Run `tailscale netcheck` and return the output.
pub fn spawn_tailscale_netcheck() -> mpsc::Receiver<NetcheckResult> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = Command::new(paths::TAILSCALE).args(["netcheck"]).output();

        let netcheck = match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let combined = if stderr.is_empty() {
                    stdout
                } else if stdout.is_empty() {
                    stderr
                } else {
                    format!("{stdout}\n{stderr}")
                };
                NetcheckResult {
                    success: output.status.success(),
                    output: combined,
                }
            }
            Err(e) => NetcheckResult {
                success: false,
                output: format!(
                    "Failed to run 'tailscale netcheck': {e}\n\nIs tailscale installed on this ISO?"
                ),
            },
        };

        let _ = tx.send(netcheck);
    });

    rx
}

// r[impl installer.tui.ssh-keys.github+4]

/// Fetch SSH public keys for a GitHub user.
pub fn spawn_github_key_fetch(username: &str) -> mpsc::Receiver<GithubKeysResult> {
    let (tx, rx) = mpsc::channel();
    let username = username.trim().to_string();

    thread::spawn(move || {
        if username.is_empty() {
            let _ = tx.send(GithubKeysResult {
                success: false,
                keys: vec![],
                error: Some("username is empty".into()),
            });
            return;
        }

        let url = format!("https://github.com/{username}.keys");
        let result = Command::new(paths::CURL)
            .args([
                "--silent",
                "--fail",
                "--max-time",
                &CHECK_TIMEOUT_SECS.to_string(),
                "--location",
                &url,
            ])
            .output();

        let github_result = match result {
            Ok(output) if output.status.success() => {
                let body = String::from_utf8_lossy(&output.stdout).to_string();
                let keys: Vec<String> = body
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                if keys.is_empty() {
                    GithubKeysResult {
                        success: false,
                        keys: vec![],
                        error: Some(format!("no SSH keys found for GitHub user '{username}'")),
                    }
                } else {
                    GithubKeysResult {
                        success: true,
                        keys,
                        error: None,
                    }
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let code = output
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or("?".into());
                GithubKeysResult {
                    success: false,
                    keys: vec![],
                    error: Some(format!(
                        "failed to fetch keys for '{username}' (curl exit {code}){}",
                        if stderr.is_empty() {
                            String::new()
                        } else {
                            format!(": {stderr}")
                        }
                    )),
                }
            }
            Err(e) => GithubKeysResult {
                success: false,
                keys: vec![],
                error: Some(format!("curl failed: {e}")),
            },
        };

        let _ = tx.send(github_result);
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_interface_fields_accessible() {
        let iface = NetInterface {
            name: "enp0s3".into(),
            mac: "08:00:27:aa:bb:cc".into(),
            state: "UP".into(),
        };
        assert_eq!(iface.name, "enp0s3");
        assert_eq!(iface.mac, "08:00:27:aa:bb:cc");
        assert_eq!(iface.state, "UP");
    }

    #[test]
    fn net_connectivity_status_variants() {
        assert_eq!(
            NetConnectivityStatus::Unknown,
            NetConnectivityStatus::Unknown
        );
        assert_ne!(
            NetConnectivityStatus::Connected("test".into()),
            NetConnectivityStatus::NoConnectivity
        );
        assert_eq!(
            NetConnectivityStatus::Configuring,
            NetConnectivityStatus::Configuring
        );
    }

    #[test]
    fn default_endpoints_has_expected_count() {
        let eps = default_endpoints();
        assert_eq!(eps.len(), 6);
    }

    #[test]
    fn total_check_count_includes_ntp() {
        let eps = default_endpoints();
        assert_eq!(total_check_count(&eps), 7);
    }

    #[test]
    fn endpoint_labels_are_unique() {
        let eps = default_endpoints();
        let labels: Vec<&str> = eps.iter().map(|e| e.label.as_str()).collect();
        let mut deduped = labels.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(labels.len(), deduped.len());
    }

    #[test]
    fn ghcr_expects_200() {
        let eps = default_endpoints();
        let ghcr = eps.iter().find(|e| e.label == "ghcr.io").unwrap();
        assert!(ghcr.expect_200);
    }

    #[test]
    fn ops_endpoints_accept_any_status() {
        let eps = default_endpoints();
        for ep in &eps {
            if ep.label.contains("ops.tamanu.io") {
                assert!(!ep.expect_200, "{} should accept any HTTP status", ep.label);
            }
        }
    }

    // r[verify installer.tui.tailscale-netcheck+3]
    #[test]
    fn tailscale_netcheck_returns_result_with_output() {
        let rx = spawn_tailscale_netcheck();
        let result = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("tailscale netcheck should complete within timeout");
        assert!(
            !result.output.is_empty(),
            "netcheck output should not be empty regardless of success or failure"
        );
    }

    #[test]
    fn generate_iso_static_netplan_full() {
        let cfg = StaticNetConfig {
            interface: "enp0s3".into(),
            ip_cidr: "192.168.1.10/24".into(),
            gateway: "192.168.1.1".into(),
            dns: "8.8.8.8, 1.1.1.1".into(),
            search_domain: "example.com".into(),
        };
        let yaml = generate_iso_static_netplan(&cfg);
        assert!(yaml.contains("enp0s3:"));
        assert!(yaml.contains("name: \"enp0s3\""));
        assert!(yaml.contains("- 192.168.1.10/24"));
        assert!(yaml.contains("via: 192.168.1.1"));
        assert!(yaml.contains("- 8.8.8.8"));
        assert!(yaml.contains("- 1.1.1.1"));
        assert!(yaml.contains("- example.com"));
    }

    #[test]
    fn generate_iso_static_netplan_no_interface() {
        let cfg = StaticNetConfig {
            interface: String::new(),
            ip_cidr: "10.0.0.5/16".into(),
            gateway: "10.0.0.1".into(),
            dns: String::new(),
            search_domain: String::new(),
        };
        let yaml = generate_iso_static_netplan(&cfg);
        assert!(yaml.contains("all-en:"));
        assert!(yaml.contains("name: \"en*\""));
        assert!(yaml.contains("- 10.0.0.5/16"));
        assert!(!yaml.contains("nameservers"));
    }

    #[test]
    fn generate_iso_ipv6_slaac_netplan_contents() {
        let cfg = StaticNetConfig {
            interface: "eth0".into(),
            ..Default::default()
        };
        let yaml = generate_iso_ipv6_slaac_netplan(&cfg);
        assert!(yaml.contains("eth0:"));
        assert!(yaml.contains("dhcp4: false"));
        assert!(yaml.contains("accept-ra: true"));
    }

    #[test]
    fn generate_iso_offline_netplan_contents() {
        let cfg = StaticNetConfig {
            interface: "enp0s3".into(),
            ..Default::default()
        };
        let yaml = generate_iso_offline_netplan(&cfg);
        assert!(yaml.contains("enp0s3:"));
        assert!(yaml.contains("dhcp4: false"));
        assert!(yaml.contains("dhcp6: false"));
        assert!(yaml.contains("optional: true"));
    }

    #[test]
    fn generate_iso_offline_netplan_no_interface() {
        let cfg = StaticNetConfig::default();
        let yaml = generate_iso_offline_netplan(&cfg);
        assert!(yaml.contains("all-en:"));
        assert!(yaml.contains("name: \"en*\""));
    }
}
