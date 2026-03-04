use std::io;
use std::net::UdpSocket;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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

// r[impl installer.tui.network-check+3]

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
    ]
}

const CHECK_TIMEOUT_SECS: u64 = 5;

/// Check a single HTTP(S) endpoint using curl.
///
/// When `expect_200` is true, the check passes only if the HTTP status code
/// is 200. Otherwise any HTTP response (even 403) counts as a pass.
fn check_http(url: &str, expect_200: bool) -> (bool, String) {
    let result = Command::new("curl")
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

// r[impl installer.tui.tailscale-netcheck+2]

/// Run `tailscale netcheck` and return the output.
pub fn spawn_tailscale_netcheck() -> mpsc::Receiver<NetcheckResult> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = Command::new("tailscale").args(["netcheck"]).output();

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

// r[impl installer.tui.ssh-keys.github]

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
        let result = Command::new("curl")
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
    fn default_endpoints_has_expected_count() {
        let eps = default_endpoints();
        assert_eq!(eps.len(), 5);
    }

    #[test]
    fn total_check_count_includes_ntp() {
        let eps = default_endpoints();
        assert_eq!(total_check_count(&eps), 6);
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

    // r[verify installer.tui.tailscale-netcheck+2]
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
}
