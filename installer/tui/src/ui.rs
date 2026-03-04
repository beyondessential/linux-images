use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::config::{FirstbootConfig, Variant};
use crate::disk::BlockDevice;
use crate::net::{self, CheckPhase, CheckResult, GithubKeysResult, NetcheckResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetPane {
    Connectivity,
    Tailscale,
}
use crate::writer::WriteProgress;

mod render;
mod run;

pub use run::{run_tui, run_tui_scripted};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    NetworkCheck,
    DiskSelection,
    VariantSelection,
    TpmToggle,
    Hostname,
    Login,
    LoginTailscale,
    LoginSshKeys,
    LoginGithub,
    Timezone,
    NetworkResults,
    Confirmation,
    Writing,
    FirstbootApply,
    Done,
    Error(String),
}

pub struct AppState {
    pub screen: Screen,
    pub devices: Vec<BlockDevice>,
    pub selected_disk_index: usize,
    pub variant: Variant,
    pub disable_tpm: bool,
    pub boot_device: Option<PathBuf>,
    pub write_progress: Option<ProgressSnapshot>,
    pub confirm_input: String,
    pub build_info: String,

    pub hostname_input: String,
    pub hostname_from_dhcp: bool,
    pub hostname_from_template: bool,
    pub tailscale_input: String,
    pub ssh_keys: Vec<String>,
    pub ssh_key_cursor: usize,
    pub password_input: String,
    pub password_confirm_input: String,
    pub password_confirming: bool,
    pub password_mismatch: bool,
    /// Pre-hashed password from config file (takes precedence over plaintext).
    pub config_password_hash: Option<String>,

    // r[impl installer.tui.timezone]
    pub available_timezones: Vec<String>,
    pub timezone_search: String,
    pub timezone_selected: String,
    pub timezone_filtered: Vec<usize>,
    pub timezone_cursor: usize,

    // r[impl installer.tui.network-check+4]
    pub net_check_phase: CheckPhase,
    pub net_check_results: Vec<Option<CheckResult>>,
    pub net_check_rx: Option<mpsc::Receiver<CheckResult>>,
    pub net_check_total: usize,
    pub net_checks_started: bool,

    // r[impl installer.tui.tailscale-netcheck+2]
    pub netcheck_phase: CheckPhase,
    pub netcheck_result: Option<NetcheckResult>,
    pub netcheck_rx: Option<mpsc::Receiver<NetcheckResult>>,

    pub net_pane: NetPane,
    pub net_scroll: u16,

    // r[impl installer.tui.ssh-keys.github+4]
    pub ssh_github_input: String,
    pub ssh_github_fetching: bool,
    pub ssh_github_error: Option<String>,
    pub ssh_github_rx: Option<mpsc::Receiver<GithubKeysResult>>,
}

// r[impl installer.tui.progress]
#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub bytes_written: u64,
    pub total_bytes: Option<u64>,
    pub throughput_mbps: f64,
    pub eta: Option<Duration>,
}

impl From<&WriteProgress> for ProgressSnapshot {
    fn from(p: &WriteProgress) -> Self {
        ProgressSnapshot {
            bytes_written: p.bytes_written,
            total_bytes: p.total_bytes,
            throughput_mbps: p.throughput_mbps(),
            eta: p.eta(),
        }
    }
}

impl AppState {
    #[expect(
        clippy::too_many_arguments,
        reason = "constructor collecting all initial state fields"
    )]
    pub fn new(
        devices: Vec<BlockDevice>,
        variant: Variant,
        disable_tpm: bool,
        firstboot: Option<FirstbootConfig>,
        boot_device: Option<PathBuf>,
        default_disk_index: Option<usize>,
        build_info: String,
        available_timezones: Vec<String>,
    ) -> Self {
        let endpoints = net::default_endpoints();
        let net_check_total = net::total_check_count(&endpoints);
        let (
            hostname_input,
            hostname_from_dhcp,
            hostname_from_template,
            tailscale_input,
            ssh_keys,
            config_password_hash,
            timezone_from_config,
        ) = match firstboot {
            Some(ref fb) => {
                let keys: Vec<String> = fb
                    .ssh_authorized_keys
                    .iter()
                    .filter(|k| !k.trim().is_empty())
                    .cloned()
                    .collect();
                let keys = if keys.is_empty() {
                    vec![String::new()]
                } else {
                    keys
                };
                (
                    fb.hostname.clone().unwrap_or_default(),
                    fb.hostname_from_dhcp,
                    fb.hostname_template.is_some(),
                    fb.tailscale_authkey.clone().unwrap_or_default(),
                    keys,
                    fb.password_hash.clone(),
                    fb.timezone.clone(),
                )
            }
            None => (
                String::new(),
                false,
                false,
                String::new(),
                vec![String::new()],
                None,
                None,
            ),
        };

        let timezone_selected = timezone_from_config.unwrap_or_else(|| "UTC".to_string());
        let timezone_filtered: Vec<usize> = (0..available_timezones.len()).collect();
        let timezone_cursor = available_timezones
            .iter()
            .position(|z| z == &timezone_selected)
            .unwrap_or(0);

        let mut state = Self {
            screen: Screen::Welcome,
            selected_disk_index: default_disk_index.unwrap_or(0),
            devices,
            variant,
            disable_tpm,
            boot_device,
            write_progress: None,
            confirm_input: String::new(),
            build_info,
            hostname_input,
            hostname_from_dhcp,
            hostname_from_template,
            tailscale_input,
            ssh_keys,
            ssh_key_cursor: 0,
            password_input: String::new(),
            password_confirm_input: String::new(),
            password_confirming: false,
            password_mismatch: false,
            config_password_hash,
            available_timezones,
            timezone_search: String::new(),
            timezone_selected,
            timezone_filtered,
            timezone_cursor,
            net_check_phase: CheckPhase::NotStarted,
            net_check_results: vec![None; net_check_total],
            net_check_rx: None,
            net_check_total,
            net_checks_started: false,
            netcheck_phase: CheckPhase::NotStarted,
            netcheck_result: None,
            netcheck_rx: None,
            net_pane: NetPane::Connectivity,
            net_scroll: 0,
            ssh_github_input: String::new(),
            ssh_github_fetching: false,
            ssh_github_error: None,
            ssh_github_rx: None,
        };
        state.ensure_trailing_blank();
        state
    }

    // r[impl installer.tui.hostname+2]
    // r[impl installer.tui.tailscale+3]
    // r[impl installer.tui.ssh-keys+4]
    // r[impl installer.tui.password+3]
    // r[impl installer.tui.timezone]
    /// Build a `FirstbootConfig` from the current interactive input fields.
    /// Returns `None` if all fields are empty (nothing to configure).
    pub fn firstboot_config(&self) -> Option<FirstbootConfig> {
        let hostname = if !self.hostname_from_dhcp && !self.hostname_input.trim().is_empty() {
            Some(self.hostname_input.trim().to_string())
        } else {
            None
        };

        let tailscale_authkey = if self.tailscale_input.trim().is_empty() {
            None
        } else {
            Some(self.tailscale_input.trim().to_string())
        };

        let ssh_authorized_keys: Vec<String> = self
            .ssh_keys
            .iter()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect();

        let password = if self.password_input.is_empty() {
            None
        } else {
            Some(self.password_input.clone())
        };

        let password_hash = self.config_password_hash.clone();

        let timezone = if self.timezone_selected == "UTC" {
            None
        } else {
            Some(self.timezone_selected.clone())
        };

        if hostname.is_none()
            && !self.hostname_from_dhcp
            && tailscale_authkey.is_none()
            && ssh_authorized_keys.is_empty()
            && password.is_none()
            && password_hash.is_none()
            && timezone.is_none()
        {
            return None;
        }

        Some(FirstbootConfig {
            hostname,
            hostname_from_dhcp: self.hostname_from_dhcp,
            hostname_template: None,
            tailscale_authkey,
            ssh_authorized_keys,
            password,
            password_hash,
            timezone,
        })
    }

    pub fn selected_disk(&self) -> Option<&BlockDevice> {
        self.devices.get(self.selected_disk_index)
    }

    // r[impl installer.tui.disk-detection+3]
    pub fn select_next_disk(&mut self) {
        if !self.devices.is_empty() {
            self.selected_disk_index = (self.selected_disk_index + 1) % self.devices.len();
        }
    }

    pub fn select_prev_disk(&mut self) {
        if !self.devices.is_empty() {
            self.selected_disk_index = self
                .selected_disk_index
                .checked_sub(1)
                .unwrap_or(self.devices.len() - 1);
        }
    }

    // r[impl installer.tui.variant-selection]
    pub fn toggle_variant(&mut self) {
        self.variant = match self.variant {
            Variant::Metal => Variant::Cloud,
            Variant::Cloud => Variant::Metal,
        };
    }

    // r[impl installer.tui.network-check+4]
    /// Start (or restart) all network connectivity checks and tailscale netcheck.
    pub fn start_net_checks(&mut self) {
        let endpoints = net::default_endpoints();
        self.net_check_total = net::total_check_count(&endpoints);
        self.net_check_results = vec![None; self.net_check_total];
        self.net_check_phase = CheckPhase::Running;
        self.net_check_rx = Some(net::spawn_checks(&endpoints));
        self.net_checks_started = true;
        self.start_tailscale_netcheck();
    }

    /// Start background checks if they haven't been started yet.
    pub fn ensure_net_checks_started(&mut self) {
        if !self.net_checks_started {
            self.start_net_checks();
        }
    }

    /// Poll for completed network check results. Returns true if any new
    /// results were received.
    pub fn poll_net_checks(&mut self) -> bool {
        let rx = match self.net_check_rx.as_ref() {
            Some(rx) => rx,
            None => return false,
        };
        let mut received = false;
        while let Ok(result) = rx.try_recv() {
            let idx = result.index;
            if idx < self.net_check_results.len() {
                self.net_check_results[idx] = Some(result);
            }
            received = true;
        }
        if self.net_check_results.iter().all(|r| r.is_some()) {
            self.net_check_phase = CheckPhase::Done;
        }
        received
    }

    // r[impl installer.tui.tailscale-netcheck+2]
    /// Start (or restart) the tailscale netcheck.
    pub fn start_tailscale_netcheck(&mut self) {
        self.netcheck_phase = CheckPhase::Running;
        self.netcheck_result = None;
        if self.net_pane == NetPane::Tailscale {
            self.net_scroll = 0;
        }
        self.netcheck_rx = Some(net::spawn_tailscale_netcheck());
    }

    /// Poll for tailscale netcheck result. Returns true if the result arrived.
    pub fn poll_tailscale_netcheck(&mut self) -> bool {
        let rx = match self.netcheck_rx.as_ref() {
            Some(rx) => rx,
            None => return false,
        };
        match rx.try_recv() {
            Ok(result) => {
                self.netcheck_result = Some(result);
                self.netcheck_phase = CheckPhase::Done;
                if self.net_pane == NetPane::Tailscale {
                    self.net_scroll = 0;
                }
                true
            }
            Err(_) => false,
        }
    }

    /// Number of lines in the connectivity check results.
    pub fn net_check_line_count(&self) -> usize {
        // Each result row + blank + note line
        self.net_check_total + 2
    }

    /// Number of lines in the tailscale netcheck output.
    pub fn netcheck_line_count(&self) -> usize {
        match &self.netcheck_result {
            Some(result) => result.output.lines().count(),
            None => 1,
        }
    }

    /// Line count for the currently active network pane.
    fn active_pane_line_count(&self) -> usize {
        match self.net_pane {
            NetPane::Connectivity => self.net_check_line_count(),
            NetPane::Tailscale => self.netcheck_line_count(),
        }
    }

    /// Scroll the active network pane down by one line.
    pub fn scroll_net_down(&mut self) {
        let max = self.active_pane_line_count().saturating_sub(1) as u16;
        if self.net_scroll < max {
            self.net_scroll += 1;
        }
    }

    /// Scroll the active network pane up by one line.
    pub fn scroll_net_up(&mut self) {
        self.net_scroll = self.net_scroll.saturating_sub(1);
    }

    /// Switch the active network pane and reset scroll.
    pub fn toggle_net_pane(&mut self) {
        self.net_pane = match self.net_pane {
            NetPane::Connectivity => NetPane::Tailscale,
            NetPane::Tailscale => NetPane::Connectivity,
        };
        self.net_scroll = 0;
    }

    // r[impl installer.tui.ssh-keys.github+4]
    /// Start fetching SSH keys for the current GitHub username.
    pub fn start_github_key_fetch(&mut self) {
        if self.ssh_github_input.trim().is_empty() {
            self.ssh_github_error = Some("username is empty".into());
            return;
        }
        self.ssh_github_fetching = true;
        self.ssh_github_error = None;
        self.ssh_github_rx = Some(net::spawn_github_key_fetch(&self.ssh_github_input));
    }

    /// Poll for GitHub key fetch result. Returns true if the result arrived.
    pub fn poll_github_keys(&mut self) -> bool {
        let rx = match self.ssh_github_rx.as_ref() {
            Some(rx) => rx,
            None => return false,
        };
        match rx.try_recv() {
            Ok(result) => {
                self.ssh_github_fetching = false;
                if result.success {
                    let first_new = self.ssh_keys.len();
                    for key in &result.keys {
                        self.ssh_keys.push(key.clone());
                    }
                    self.ssh_github_error = None;
                    self.ensure_trailing_blank();
                    self.ssh_key_cursor = first_new;
                    self.screen = Screen::LoginSshKeys;
                } else {
                    self.ssh_github_error = result.error;
                }
                true
            }
            Err(_) => false,
        }
    }

    // r[impl installer.tui.tpm-toggle]
    // r[impl installer.tui.password+3]
    // r[impl installer.tui.timezone]
    pub fn advance(&mut self) {
        self.screen = match &self.screen {
            Screen::Welcome => {
                // r[impl installer.tui.network-check+4]
                self.ensure_net_checks_started();
                Screen::DiskSelection
            }
            Screen::NetworkCheck => return,
            Screen::DiskSelection => Screen::VariantSelection,
            Screen::VariantSelection if self.variant == Variant::Metal => Screen::TpmToggle,
            Screen::VariantSelection => Screen::Hostname,
            Screen::TpmToggle => Screen::Hostname,
            Screen::Hostname => Screen::Login,
            Screen::Login => Screen::Timezone,
            Screen::LoginTailscale | Screen::LoginSshKeys | Screen::LoginGithub => return,
            Screen::Timezone => Screen::NetworkResults,
            Screen::NetworkResults => Screen::Confirmation,
            Screen::Confirmation => Screen::Writing,
            Screen::Writing => Screen::FirstbootApply,
            Screen::FirstbootApply => Screen::Done,
            Screen::Done | Screen::Error(_) => return,
        };
    }

    pub fn go_back(&mut self) {
        self.screen = match &self.screen {
            Screen::NetworkCheck => Screen::Welcome,
            Screen::DiskSelection => Screen::Welcome,
            Screen::VariantSelection => Screen::DiskSelection,
            Screen::TpmToggle => Screen::VariantSelection,
            Screen::Hostname => {
                if self.variant == Variant::Metal {
                    Screen::TpmToggle
                } else {
                    Screen::VariantSelection
                }
            }
            Screen::Login => Screen::Hostname,
            Screen::LoginTailscale | Screen::LoginSshKeys | Screen::LoginGithub => Screen::Login,
            Screen::Timezone => Screen::Login,
            Screen::NetworkResults => Screen::Timezone,
            Screen::Confirmation => Screen::NetworkResults,
            _ => return,
        };
    }

    /// Enter the dedicated network check screen from the welcome screen.
    pub fn open_network_check(&mut self) {
        self.ensure_net_checks_started();
        self.screen = Screen::NetworkCheck;
    }

    pub fn confirmation_text(&self) -> &str {
        "yes"
    }

    // r[impl installer.tui.confirmation+3]
    pub fn is_confirmed(&self) -> bool {
        self.confirm_input
            .trim()
            .eq_ignore_ascii_case(self.confirmation_text())
    }

    // r[impl installer.tui.hostname+2]
    pub fn hostname_required(&self) -> bool {
        self.variant == Variant::Metal && !self.hostname_from_dhcp
    }

    // r[impl installer.tui.network-check+4]
    /// Whether github.com is reachable per background network checks.
    pub fn github_reachable(&self) -> bool {
        self.net_check_results
            .iter()
            .any(|r| matches!(r, Some(r) if r.label == "github.com" && r.passed))
    }

    // r[impl installer.tui.ssh-keys+4]

    /// Recognized SSH public key type prefixes.
    const SSH_KEY_PREFIXES: &[&str] = &[
        "ssh-rsa",
        "ssh-ed25519",
        "ssh-dss",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com",
        "sk-ecdsa-sha2-nistp256@openssh.com",
    ];

    /// Check whether a string looks like a valid SSH public key.
    pub fn is_valid_ssh_key(key: &str) -> bool {
        let trimmed = key.trim();
        Self::SSH_KEY_PREFIXES.iter().any(|prefix| {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                rest.starts_with(' ')
                    && rest.trim().len() > prefix.len() - prefix.len()
                    && rest[1..].contains(|c: char| !c.is_whitespace())
            } else {
                false
            }
        })
    }

    /// Filter ssh_keys: remove empty and invalid entries. Ensure at least one
    /// empty entry remains.
    pub fn filter_ssh_keys(&mut self) {
        self.ssh_keys
            .retain(|k| !k.trim().is_empty() && Self::is_valid_ssh_key(k));
        if self.ssh_keys.is_empty() {
            self.ssh_keys.push(String::new());
        }
        if self.ssh_key_cursor >= self.ssh_keys.len() {
            self.ssh_key_cursor = 0;
        }
    }

    /// Ensure the ssh_keys list always has a trailing blank entry.
    /// If the last entry is non-empty, a new blank entry is appended.
    /// Does not move the cursor.
    pub fn ensure_trailing_blank(&mut self) {
        if self.ssh_keys.is_empty() || !self.ssh_keys.last().unwrap().trim().is_empty() {
            self.ssh_keys.push(String::new());
        }
    }

    /// Build a summary line for a collapsed SSH key field.
    pub fn ssh_key_summary(key: &str) -> String {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return "(empty)".into();
        }
        let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
        match parts.len() {
            1 => {
                if parts[0].len() > 40 {
                    format!("{}...", &parts[0][..37])
                } else {
                    parts[0].to_string()
                }
            }
            2 => {
                let key_type = parts[0];
                let key_data = parts[1];
                let truncated = if key_data.len() > 20 {
                    format!("{}...{}", &key_data[..8], &key_data[key_data.len() - 8..])
                } else {
                    key_data.to_string()
                };
                format!("{key_type} {truncated}")
            }
            _ => {
                let key_type = parts[0];
                let key_data = parts[1];
                let comment = parts[2];
                let truncated = if key_data.len() > 20 {
                    format!("{}...{}", &key_data[..8], &key_data[key_data.len() - 8..])
                } else {
                    key_data.to_string()
                };
                format!("{key_type} {truncated} {comment}")
            }
        }
    }

    // r[impl installer.tui.password+3]
    pub fn password_matches(&self) -> bool {
        self.password_input == self.password_confirm_input
    }

    /// Whether a password has been provided (either typed interactively
    /// or via the config file as a hash).
    #[cfg(test)]
    pub fn has_password(&self) -> bool {
        !self.password_input.is_empty() || self.config_password_hash.is_some()
    }

    // r[impl installer.tui.timezone]
    /// Re-filter the timezone list based on the current search string.
    pub fn update_timezone_filter(&mut self) {
        let query = self.timezone_search.to_lowercase();
        self.timezone_filtered = self
            .available_timezones
            .iter()
            .enumerate()
            .filter(|(_, tz)| {
                if query.is_empty() {
                    return true;
                }
                tz.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        if self.timezone_cursor >= self.timezone_filtered.len() {
            self.timezone_cursor = 0;
        }
    }

    /// Return the currently highlighted timezone name, or the selected one.
    pub fn timezone_highlighted(&self) -> &str {
        if let Some(&idx) = self.timezone_filtered.get(self.timezone_cursor) {
            &self.available_timezones[idx]
        } else {
            &self.timezone_selected
        }
    }

    /// The effective timezone for the install plan (always has a value).
    pub fn effective_timezone(&self) -> &str {
        &self.timezone_selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::net::CheckPhase;

    fn test_timezones() -> Vec<String> {
        vec![
            "America/New_York".into(),
            "Europe/London".into(),
            "Pacific/Auckland".into(),
            "UTC".into(),
        ]
    }

    fn make_state() -> AppState {
        use crate::disk::TransportType;
        let devices = vec![
            BlockDevice {
                path: PathBuf::from("/dev/sda"),
                size_bytes: 500_000_000_000,
                model: "Test SSD".into(),
                transport: TransportType::Nvme,
                removable: false,
            },
            BlockDevice {
                path: PathBuf::from("/dev/sdb"),
                size_bytes: 1_000_000_000_000,
                model: "Test HDD".into(),
                transport: TransportType::Sata,
                removable: false,
            },
        ];
        AppState::new(
            devices,
            Variant::Metal,
            false,
            None,
            None,
            None,
            String::new(),
            test_timezones(),
        )
    }

    // r[verify installer.tui.welcome+3]
    #[test]
    fn initial_state() {
        let state = make_state();
        assert_eq!(state.screen, Screen::Welcome);
        assert_eq!(state.selected_disk_index, 0);
        assert_eq!(state.variant, Variant::Metal);
        assert!(!state.disable_tpm);
        assert_eq!(state.net_check_phase, CheckPhase::NotStarted);
        assert_eq!(state.netcheck_phase, CheckPhase::NotStarted);
        assert!(!state.net_checks_started);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn welcome_advances_to_disk_selection() {
        let mut state = make_state();
        state.advance();
        assert_eq!(state.screen, Screen::DiskSelection);
        assert!(state.net_checks_started);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn open_network_check_from_welcome() {
        let mut state = make_state();
        state.open_network_check();
        assert_eq!(state.screen, Screen::NetworkCheck);
        assert!(state.net_checks_started);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn network_check_goes_back_to_welcome() {
        let mut state = make_state();
        state.screen = Screen::NetworkCheck;
        state.go_back();
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn network_check_advance_is_noop() {
        let mut state = make_state();
        state.screen = Screen::NetworkCheck;
        state.advance();
        assert_eq!(state.screen, Screen::NetworkCheck);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn disk_selection_goes_back_to_welcome() {
        let mut state = make_state();
        state.screen = Screen::DiskSelection;
        state.go_back();
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn timezone_advances_to_network_results() {
        let mut state = make_state();
        state.screen = Screen::Timezone;
        state.timezone_selected = "UTC".into();
        state.advance();
        assert_eq!(state.screen, Screen::NetworkResults);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn network_results_advances_to_confirmation() {
        let mut state = make_state();
        state.screen = Screen::NetworkResults;
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn network_results_goes_back_to_timezone() {
        let mut state = make_state();
        state.screen = Screen::NetworkResults;
        state.go_back();
        assert_eq!(state.screen, Screen::Timezone);
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn confirmation_goes_back_to_network_results() {
        let mut state = make_state();
        state.screen = Screen::Confirmation;
        state.go_back();
        assert_eq!(state.screen, Screen::NetworkResults);
    }

    // r[verify installer.tui.disk-detection+3]
    #[test]
    fn disk_navigation_wraps() {
        let mut state = make_state();
        assert_eq!(state.selected_disk_index, 0);
        state.select_prev_disk();
        assert_eq!(state.selected_disk_index, 1);
        state.select_next_disk();
        assert_eq!(state.selected_disk_index, 0);
    }

    // r[verify installer.tui.variant-selection]
    #[test]
    fn variant_toggle() {
        let mut state = make_state();
        assert_eq!(state.variant, Variant::Metal);
        state.toggle_variant();
        assert_eq!(state.variant, Variant::Cloud);
        state.toggle_variant();
        assert_eq!(state.variant, Variant::Metal);
    }

    // r[verify installer.tui.tpm-toggle]
    // r[verify installer.tui.password+3]
    #[test]
    fn advance_metal_flow() {
        let mut state = make_state();
        state.variant = Variant::Metal;

        assert_eq!(state.screen, Screen::Welcome);
        state.advance();
        assert_eq!(state.screen, Screen::DiskSelection);
        state.advance();
        assert_eq!(state.screen, Screen::VariantSelection);
        state.advance();
        assert_eq!(state.screen, Screen::TpmToggle);
        state.advance();
        assert_eq!(state.screen, Screen::Hostname);
        state.advance();
        assert_eq!(state.screen, Screen::Login);
        state.advance();
        assert_eq!(state.screen, Screen::Timezone);
        state.advance();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.variant-selection]
    // r[verify installer.tui.password+3]
    // r[verify installer.tui.timezone]
    #[test]
    fn advance_cloud_skips_tpm() {
        let mut state = make_state();
        state.variant = Variant::Cloud;

        assert_eq!(state.screen, Screen::Welcome);
        state.advance();
        assert_eq!(state.screen, Screen::DiskSelection);
        state.advance();
        assert_eq!(state.screen, Screen::VariantSelection);
        state.advance();
        assert_eq!(state.screen, Screen::Hostname);
        state.advance();
        assert_eq!(state.screen, Screen::Login);
        state.advance();
        assert_eq!(state.screen, Screen::Timezone);
        state.advance();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.tpm-toggle]
    // r[verify installer.tui.password+3]
    // r[verify installer.tui.timezone]
    #[test]
    fn go_back_through_metal_flow() {
        let mut state = make_state();
        state.variant = Variant::Metal;
        state.screen = Screen::Confirmation;

        state.go_back();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.go_back();
        assert_eq!(state.screen, Screen::Timezone);
        state.go_back();
        assert_eq!(state.screen, Screen::Login);
        state.go_back();
        assert_eq!(state.screen, Screen::Hostname);
        state.go_back();
        assert_eq!(state.screen, Screen::TpmToggle);
        state.go_back();
        assert_eq!(state.screen, Screen::VariantSelection);
        state.go_back();
        assert_eq!(state.screen, Screen::DiskSelection);
        state.go_back();
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.variant-selection]
    // r[verify installer.tui.password+3]
    // r[verify installer.tui.timezone]
    #[test]
    fn go_back_cloud_skips_tpm() {
        let mut state = make_state();
        state.variant = Variant::Cloud;
        state.screen = Screen::Confirmation;

        state.go_back();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.go_back();
        assert_eq!(state.screen, Screen::Timezone);
        state.go_back();
        assert_eq!(state.screen, Screen::Login);
        state.go_back();
        assert_eq!(state.screen, Screen::Hostname);
        state.go_back();
        assert_eq!(state.screen, Screen::VariantSelection);
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn login_sub_screens_go_back_to_login() {
        let mut state = make_state();
        state.screen = Screen::LoginTailscale;
        state.go_back();
        assert_eq!(state.screen, Screen::Login);

        state.screen = Screen::LoginSshKeys;
        state.go_back();
        assert_eq!(state.screen, Screen::Login);

        state.screen = Screen::LoginGithub;
        state.go_back();
        assert_eq!(state.screen, Screen::Login);
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn login_sub_screens_do_not_advance() {
        let mut state = make_state();
        state.screen = Screen::LoginTailscale;
        state.advance();
        assert_eq!(state.screen, Screen::LoginTailscale);

        state.screen = Screen::LoginSshKeys;
        state.advance();
        assert_eq!(state.screen, Screen::LoginSshKeys);

        state.screen = Screen::LoginGithub;
        state.advance();
        assert_eq!(state.screen, Screen::LoginGithub);
    }

    // r[verify installer.tui.confirmation+3]
    #[test]
    fn confirmation_requires_explicit_yes() {
        let mut state = make_state();
        assert!(!state.is_confirmed());
        state.confirm_input = "no".into();
        assert!(!state.is_confirmed());
        state.confirm_input = "yes".into();
        assert!(state.is_confirmed());
        state.confirm_input = " YES ".into();
        assert!(state.is_confirmed());
    }

    // r[verify installer.tui.confirmation+3]
    #[test]
    fn done_and_error_do_not_advance() {
        let mut state = make_state();
        state.screen = Screen::Done;
        state.advance();
        assert_eq!(state.screen, Screen::Done);

        state.screen = Screen::Error("test".into());
        state.advance();
        assert_eq!(state.screen, Screen::Error("test".into()));
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn hostname_prefilled_from_config() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let fb = FirstbootConfig {
            hostname: Some("myhost".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            test_timezones(),
        );
        assert_eq!(state.hostname_input, "myhost");
        assert_eq!(state.tailscale_input, "");
        assert_eq!(state.ssh_keys, vec![String::new()]);
    }

    // r[verify installer.tui.tailscale+3]
    #[test]
    fn tailscale_prefilled_from_config() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let fb = FirstbootConfig {
            tailscale_authkey: Some("tskey-auth-xxx".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            test_timezones(),
        );
        assert_eq!(state.tailscale_input, "tskey-auth-xxx");
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn ssh_keys_prefilled_from_config() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let fb = FirstbootConfig {
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA key1".into(), "ssh-rsa BBBB key2".into()],
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            test_timezones(),
        );
        assert_eq!(
            state.ssh_keys,
            vec!["ssh-ed25519 AAAA key1", "ssh-rsa BBBB key2", ""]
        );
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn firstboot_config_from_inputs() {
        let mut state = make_state();
        assert!(state.firstboot_config().is_none());

        state.hostname_input = "server-01".into();
        let fb = state.firstboot_config().unwrap();
        assert_eq!(fb.hostname.as_deref(), Some("server-01"));
        assert!(fb.tailscale_authkey.is_none());
        assert!(fb.ssh_authorized_keys.is_empty());
        assert!(fb.password.is_none());
        assert!(fb.password_hash.is_none());
    }

    // r[verify installer.tui.tailscale+3]
    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn firstboot_config_all_fields() {
        let mut state = make_state();
        state.hostname_input = "host".into();
        state.tailscale_input = "tskey-auth-123".into();
        state.ssh_keys = vec!["ssh-ed25519 AAAA".into(), "ssh-rsa BBBB".into()];
        state.password_input = "s3cret".into();

        let fb = state.firstboot_config().unwrap();
        assert_eq!(fb.hostname.as_deref(), Some("host"));
        assert_eq!(fb.tailscale_authkey.as_deref(), Some("tskey-auth-123"));
        assert_eq!(fb.ssh_authorized_keys.len(), 2);
        assert_eq!(fb.password.as_deref(), Some("s3cret"));
        assert!(fb.password_hash.is_none());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn firstboot_config_empty_strings_are_none() {
        let mut state = make_state();
        state.hostname_input = "   ".into();
        state.tailscale_input = "  ".into();
        state.ssh_keys = vec![String::new(), "  ".into()];
        assert!(state.firstboot_config().is_none());
    }

    // r[verify installer.tui.password+3]
    #[test]
    fn password_match_logic() {
        let mut state = make_state();
        assert!(state.password_matches());

        state.password_input = "secret".into();
        state.password_confirm_input = "secret".into();
        assert!(state.password_matches());

        state.password_confirm_input = "wrong".into();
        assert!(!state.password_matches());
    }

    // r[verify installer.tui.password+3]
    #[test]
    fn has_password_from_input() {
        let mut state = make_state();
        assert!(!state.has_password());

        state.password_input = "secret".into();
        assert!(state.has_password());
    }

    // r[verify installer.tui.password+3]
    #[test]
    fn has_password_from_config_hash() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let fb = FirstbootConfig {
            password_hash: Some("$6$rounds=4096$salt$hash".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            test_timezones(),
        );
        assert!(state.has_password());
        let fb = state.firstboot_config().unwrap();
        assert!(fb.password.is_none());
        assert_eq!(
            fb.password_hash.as_deref(),
            Some("$6$rounds=4096$salt$hash")
        );
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn hostname_required_for_metal() {
        let mut state = make_state();
        state.variant = Variant::Metal;
        assert!(state.hostname_required());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn hostname_not_required_for_cloud() {
        let mut state = make_state();
        state.variant = Variant::Cloud;
        assert!(!state.hostname_required());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn hostname_not_required_for_metal_with_dhcp() {
        let mut state = make_state();
        state.variant = Variant::Metal;
        state.hostname_from_dhcp = true;
        assert!(!state.hostname_required());
    }

    #[test]
    fn firstboot_config_with_dhcp_hostname() {
        let mut state = make_state();
        state.hostname_from_dhcp = true;
        let fb = state.firstboot_config().unwrap();
        assert!(fb.hostname_from_dhcp);
        assert!(fb.hostname.is_none());
    }

    #[test]
    fn firstboot_config_dhcp_overrides_hostname_input() {
        let mut state = make_state();
        state.hostname_from_dhcp = true;
        state.hostname_input = "should-be-ignored".into();
        let fb = state.firstboot_config().unwrap();
        assert!(fb.hostname_from_dhcp);
        assert!(fb.hostname.is_none());
    }

    #[test]
    fn hostname_from_template_flag_from_config() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let fb = FirstbootConfig {
            hostname: Some("resolved-name".into()),
            hostname_template: Some("srv-{hex:6}".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            Variant::Metal,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            test_timezones(),
        );
        assert!(state.hostname_from_template);
        assert_eq!(state.hostname_input, "resolved-name");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn timezone_defaults_to_utc() {
        let state = make_state();
        assert_eq!(state.timezone_selected, "UTC");
        assert_eq!(state.effective_timezone(), "UTC");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn timezone_prefilled_from_config() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let fb = FirstbootConfig {
            timezone: Some("Pacific/Auckland".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            Variant::Metal,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            test_timezones(),
        );
        assert_eq!(state.timezone_selected, "Pacific/Auckland");
        assert_eq!(state.effective_timezone(), "Pacific/Auckland");
        assert_eq!(state.timezone_cursor, 2);
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn timezone_filter_narrows_list() {
        let mut state = make_state();
        state.timezone_search = "auck".into();
        state.update_timezone_filter();
        assert_eq!(state.timezone_filtered.len(), 1);
        assert_eq!(state.timezone_highlighted(), "Pacific/Auckland");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn timezone_filter_empty_shows_all() {
        let mut state = make_state();
        state.timezone_search = String::new();
        state.update_timezone_filter();
        assert_eq!(state.timezone_filtered.len(), 4);
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn timezone_filter_no_match() {
        let mut state = make_state();
        state.timezone_search = "zzz".into();
        state.update_timezone_filter();
        assert_eq!(state.timezone_filtered.len(), 0);
    }

    // r[verify installer.tui.ssh-keys.github+4]
    #[test]
    fn github_key_fetch_appends_to_ssh_keys() {
        let mut state = make_state();
        state.screen = Screen::LoginGithub;
        state.ssh_keys = vec!["ssh-ed25519 existing-key".into()];

        let (tx, rx) = std::sync::mpsc::channel();
        state.ssh_github_fetching = true;
        state.ssh_github_rx = Some(rx);
        tx.send(crate::net::GithubKeysResult {
            success: true,
            keys: vec!["ssh-rsa fetched-key".into()],
            error: None,
        })
        .unwrap();

        assert!(state.poll_github_keys());
        assert!(!state.ssh_github_fetching);
        assert!(state.ssh_keys.contains(&"ssh-rsa fetched-key".to_string()));
        assert!(
            state
                .ssh_keys
                .contains(&"ssh-ed25519 existing-key".to_string())
        );
        assert_eq!(
            state.screen,
            Screen::LoginSshKeys,
            "should navigate to SSH Keys sub-screen after successful fetch"
        );
        assert_eq!(
            state.ssh_key_cursor, 1,
            "cursor should point to the first imported key"
        );
    }

    #[test]
    fn github_key_fetch_error_sets_message() {
        let mut state = make_state();
        state.screen = Screen::LoginGithub;

        let (tx, rx) = std::sync::mpsc::channel();
        state.ssh_github_fetching = true;
        state.ssh_github_rx = Some(rx);
        tx.send(crate::net::GithubKeysResult {
            success: false,
            keys: vec![],
            error: Some("user not found".into()),
        })
        .unwrap();

        assert!(state.poll_github_keys());
        assert!(!state.ssh_github_fetching);
        assert_eq!(state.ssh_github_error.as_deref(), Some("user not found"));
        assert_eq!(
            state.screen,
            Screen::LoginGithub,
            "should stay on GitHub screen when fetch fails"
        );
    }

    #[test]
    fn github_key_fetch_empty_username_sets_error() {
        let mut state = make_state();
        state.ssh_github_input = "  ".into();
        state.start_github_key_fetch();
        assert!(!state.ssh_github_fetching);
        assert!(state.ssh_github_error.is_some());
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn filter_ssh_keys_removes_invalid() {
        let mut state = make_state();
        state.ssh_keys = vec![
            "ssh-ed25519 AAAA key1".into(),
            "not-a-key".into(),
            String::new(),
            "ssh-rsa BBBB key2".into(),
        ];
        state.filter_ssh_keys();
        assert_eq!(state.ssh_keys.len(), 2);
        assert_eq!(state.ssh_keys[0], "ssh-ed25519 AAAA key1");
        assert_eq!(state.ssh_keys[1], "ssh-rsa BBBB key2");
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn filter_ssh_keys_ensures_one_empty() {
        let mut state = make_state();
        state.ssh_keys = vec!["invalid".into(), String::new()];
        state.filter_ssh_keys();
        assert_eq!(state.ssh_keys, vec![String::new()]);
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn ensure_trailing_blank_appends_when_last_nonempty() {
        let mut state = make_state();
        state.ssh_keys = vec!["ssh-ed25519 AAAA key1".into()];
        state.ensure_trailing_blank();
        assert_eq!(state.ssh_keys, vec!["ssh-ed25519 AAAA key1", ""]);
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn ensure_trailing_blank_noop_when_last_empty() {
        let mut state = make_state();
        state.ssh_keys = vec!["ssh-ed25519 AAAA key1".into(), String::new()];
        state.ensure_trailing_blank();
        assert_eq!(state.ssh_keys, vec!["ssh-ed25519 AAAA key1", ""]);
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn ensure_trailing_blank_on_empty_vec() {
        let mut state = make_state();
        state.ssh_keys = vec![];
        state.ensure_trailing_blank();
        assert_eq!(state.ssh_keys, vec![String::new()]);
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn constructor_has_trailing_blank_with_prefilled_keys() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let fb = FirstbootConfig {
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA key1".into(), "ssh-rsa BBBB key2".into()],
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            test_timezones(),
        );
        assert_eq!(
            state.ssh_keys,
            vec!["ssh-ed25519 AAAA key1", "ssh-rsa BBBB key2", ""]
        );
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn ssh_key_summary_with_comment() {
        let summary =
            AppState::ssh_key_summary("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAbcdef me@host");
        assert!(summary.starts_with("ssh-ed25519 "));
        assert!(summary.contains("me@host"));
    }

    // r[verify installer.tui.ssh-keys+4]
    #[test]
    fn ssh_key_summary_empty() {
        assert_eq!(AppState::ssh_key_summary(""), "(empty)");
    }

    // r[verify installer.tui.network-check+4]
    #[test]
    fn github_reachable_when_check_passes() {
        let mut state = make_state();
        assert!(!state.github_reachable());
        // Simulate github.com check passing
        let idx = state.net_check_results.len() - 2; // github.com is second-to-last (before NTP)
        state.net_check_results[idx] = Some(crate::net::CheckResult {
            index: idx,
            label: "github.com".into(),
            passed: true,
            detail: "HTTP 301".into(),
        });
        assert!(state.github_reachable());
    }

    #[test]
    fn firstboot_config_includes_non_utc_timezone() {
        let mut state = make_state();
        state.hostname_input = "host".into();
        state.timezone_selected = "Europe/London".into();
        let fb = state.firstboot_config().unwrap();
        assert_eq!(fb.timezone.as_deref(), Some("Europe/London"));
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn firstboot_config_utc_timezone_is_none() {
        let mut state = make_state();
        state.hostname_input = "host".into();
        state.timezone_selected = "UTC".into();
        let fb = state.firstboot_config().unwrap();
        assert!(fb.timezone.is_none());
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn firstboot_config_timezone_alone_triggers_some() {
        let mut state = make_state();
        state.timezone_selected = "Asia/Tokyo".into();
        let fb = state.firstboot_config();
        assert!(fb.is_some());
        assert_eq!(fb.unwrap().timezone.as_deref(), Some("Asia/Tokyo"));
    }
}
