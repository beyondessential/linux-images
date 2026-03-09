use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::config::{DiskEncryption, InstallConfig, NetworkMode};
use crate::disk::BlockDevice;
use crate::net::{
    self, CheckPhase, CheckResult, GithubKeysResult, NetConnectivityStatus, NetInterface,
    NetcheckResult,
};
use crate::writer::{self, PartitionManifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetPane {
    Connectivity,
    Tailscale,
}

/// Network mode for the target pane in the TUI.
/// `CopyCurrent` is TUI-only; it resolves to the effective ISO config
/// before being written to the target. It is never serialised to the
/// config file.
// r[impl installer.tui.network-config+13]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetNetworkMode {
    CopyCurrent,
    Dhcp,
    StaticIp,
    Ipv6Slaac,
    Offline,
}

/// Which pane of the network config accordion is expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetConfigPane {
    Iso,
    Target,
}

/// Which field in the network config screen currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetConfigFocus {
    // ISO pane fields
    IsoMode,
    IsoInterface,
    IsoIp,
    IsoGateway,
    IsoDns,
    IsoDomain,
    // Target pane fields
    TargetMode,
    TargetInterface,
    TargetIp,
    TargetGateway,
    TargetDns,
    TargetDomain,
}

impl NetConfigFocus {
    /// Which pane this focus field belongs to.
    pub fn pane(self) -> NetConfigPane {
        match self {
            Self::IsoMode
            | Self::IsoInterface
            | Self::IsoIp
            | Self::IsoGateway
            | Self::IsoDns
            | Self::IsoDomain => NetConfigPane::Iso,
            Self::TargetMode
            | Self::TargetInterface
            | Self::TargetIp
            | Self::TargetGateway
            | Self::TargetDns
            | Self::TargetDomain => NetConfigPane::Target,
        }
    }

    /// Whether this focus is a text input field (as opposed to a radio/dropdown).
    pub fn is_text_input(self) -> bool {
        matches!(
            self,
            Self::IsoIp
                | Self::IsoGateway
                | Self::IsoDns
                | Self::IsoDomain
                | Self::TargetIp
                | Self::TargetGateway
                | Self::TargetDns
                | Self::TargetDomain
        )
    }

    /// Whether this focus is an interface dropdown.
    pub fn is_interface_dropdown(self) -> bool {
        matches!(self, Self::IsoInterface | Self::TargetInterface)
    }

    /// Whether this focus is the mode radio selector.
    pub fn is_mode_selector(self) -> bool {
        matches!(self, Self::IsoMode | Self::TargetMode)
    }
}

/// Static IP configuration fields for a pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticNetConfig {
    pub interface: String,
    pub ip_cidr: String,
    pub gateway: String,
    pub dns: String,
    pub search_domain: String,
}

impl StaticNetConfig {
    /// If `ip_cidr` has no `/xx` suffix, append `/24`.
    pub fn auto_suffix_cidr(&mut self) {
        let trimmed = self.ip_cidr.trim();
        if !trimmed.is_empty() && !trimmed.contains('/') {
            self.ip_cidr = format!("{trimmed}/24");
        }
    }
}
use crate::writer::WriteProgress;

/// State of the upfront dm-verity integrity check that runs on the welcome screen.
// r[impl iso.verity.check+6]
// r[impl installer.tui.welcome+8]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerityCheckState {
    /// Verity is not active (fallback/dev build); no check needed.
    NotNeeded,
    /// Check is running; progress is tracked via `verity_progress`.
    Running,
    /// Check completed successfully.
    Passed,
    /// Check failed with an error message.
    Failed(String),
}

mod render;
mod run;

pub use run::{run_tui, run_tui_scripted};

#[derive(Debug, Clone, PartialEq, Eq)]
// r[impl installer.tui.disk-encryption+2]
pub enum Screen {
    Welcome,
    NetworkConfig,
    NetworkCheck,
    DiskSelection,
    DiskEncryption,
    Hostname,
    HostnameInput,
    Login,
    LoginTailscale,
    LoginSshKeys,
    LoginGithub,
    Timezone,
    NetworkResults,
    Confirmation,
    Installing,
    Done,
    Error(String),
}

pub struct AppState {
    pub screen: Screen,
    pub devices: Vec<BlockDevice>,
    pub selected_disk_index: usize,
    pub disk_encryption: DiskEncryption,
    pub tpm_present: bool,
    pub boot_device: Option<PathBuf>,
    pub write_progress: Option<ProgressSnapshot>,
    pub confirm_input: String,
    pub build_info: String,
    pub recovery_passphrase: Option<String>,

    // r[impl iso.verity.check+6]
    // r[impl installer.tui.welcome+8]
    pub verity_check: VerityCheckState,
    pub verity_progress: Option<ProgressSnapshot>,
    pub verity_rx: Option<mpsc::Receiver<VerityMessage>>,

    pub hostname_input: String,
    pub hostname_from_dhcp: bool,
    pub hostname_from_template: bool,
    pub hostname_error: Option<String>,
    pub tailscale_input: String,
    pub ssh_keys: Vec<String>,
    pub ssh_key_cursor: usize,
    pub password_input: String,
    pub password_confirm_input: String,
    pub password_confirming: bool,
    pub password_mismatch: bool,
    pub password_empty: bool,
    /// Pre-hashed password from config file (takes precedence over plaintext).
    pub config_password_hash: Option<String>,

    // r[impl installer.tui.timezone]
    pub available_timezones: Vec<String>,
    pub timezone_search: String,
    pub timezone_selected: String,
    pub timezone_filtered: Vec<usize>,
    pub timezone_cursor: usize,

    // r[impl installer.tui.network-check+6]
    pub net_check_phase: CheckPhase,
    pub net_check_results: Vec<Option<CheckResult>>,
    pub net_check_rx: Option<mpsc::Receiver<CheckResult>>,
    pub net_check_total: usize,
    pub net_checks_started: bool,

    // r[impl installer.tui.tailscale-netcheck+3]
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

    // r[impl installer.tui.network-config+13]
    pub iso_network_mode: NetworkMode,
    pub iso_static_config: StaticNetConfig,
    pub target_network_mode: TargetNetworkMode,
    pub target_static_config: StaticNetConfig,
    pub detected_interfaces: Vec<NetInterface>,
    pub iso_net_status: NetConnectivityStatus,
    pub net_config_pane: NetConfigPane,
    pub net_config_focus: NetConfigFocus,
    pub net_apply_debounce: Option<Instant>,
    pub target_pane_touched: bool,
    /// Whether the config file specified a concrete `network-mode` (suppresses
    /// the "Copy current config" option in the target pane).
    pub config_has_network_mode: bool,
    /// Whether the offline target warning dialog is currently shown.
    pub offline_target_warning: bool,
}

/// Message sent from the background verity integrity check thread.
pub enum VerityMessage {
    Progress(ProgressSnapshot),
    Done,
    Error(String),
}

// r[impl installer.tui.progress+4]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPhase {
    Writing,
    Expanding,
    RandomizingUuids,
    EncryptionSetup,
    RebuildingBootConfig,
    VerifyingPartitions,
    ApplyingConfig,
}

impl InstallPhase {
    /// The fixed fraction of the overall progress bar at which this phase
    /// *starts*. Writing occupies 0..90%, post-write steps share the last 10%.
    pub fn bar_start(self) -> f64 {
        match self {
            Self::Writing => 0.0,
            Self::Expanding => 0.90,
            Self::RandomizingUuids => 0.92,
            Self::EncryptionSetup => 0.93,
            Self::RebuildingBootConfig => 0.94,
            Self::VerifyingPartitions => 0.96,
            Self::ApplyingConfig => 0.97,
        }
    }

    /// The fixed fraction at which this phase *ends* (== next phase's start,
    /// or 1.0 for the last phase).
    pub fn bar_end(self) -> f64 {
        match self {
            Self::Writing => 0.90,
            Self::Expanding => 0.92,
            Self::RandomizingUuids => 0.93,
            Self::EncryptionSetup => 0.94,
            Self::RebuildingBootConfig => 0.96,
            Self::VerifyingPartitions => 0.97,
            Self::ApplyingConfig => 1.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Writing => "Writing partitions...",
            Self::Expanding => "Expanding root filesystem...",
            Self::RandomizingUuids => "Randomizing filesystem UUIDs...",
            Self::EncryptionSetup => "Setting up encryption...",
            Self::RebuildingBootConfig => "Rebuilding boot config...",
            Self::VerifyingPartitions => "Verifying partition table...",
            Self::ApplyingConfig => "Applying configuration...",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub bytes_written: u64,
    pub total_bytes: Option<u64>,
    pub throughput_mbps: f64,
    pub eta: Option<Duration>,
    pub phase: InstallPhase,
}

impl ProgressSnapshot {
    /// Overall progress fraction (0.0..1.0) scaled to the full install bar.
    /// During the Writing phase, the write's byte fraction is mapped into
    /// 0..90%. Post-write phases jump to their fixed start fraction.
    pub fn overall_fraction(&self) -> f64 {
        match self.phase {
            InstallPhase::Writing => {
                let write_frac = self
                    .total_bytes
                    .map(|t| {
                        if t == 0 {
                            0.0
                        } else {
                            self.bytes_written as f64 / t as f64
                        }
                    })
                    .unwrap_or(0.0)
                    .min(1.0);
                write_frac * InstallPhase::Writing.bar_end()
            }
            phase => phase.bar_start(),
        }
    }
}

impl From<&WriteProgress> for ProgressSnapshot {
    fn from(p: &WriteProgress) -> Self {
        ProgressSnapshot {
            bytes_written: p.bytes_written,
            total_bytes: p.total_bytes,
            throughput_mbps: p.throughput_mbps(),
            eta: p.eta(),
            phase: InstallPhase::Writing,
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
        disk_encryption: DiskEncryption,
        tpm_present: bool,
        install_config: &InstallConfig,
        boot_device: Option<PathBuf>,
        default_disk_index: Option<usize>,
        build_info: String,
        available_timezones: Vec<String>,
        verity_active: bool,
    ) -> Self {
        let endpoints = net::default_endpoints();
        let net_check_total = net::total_check_count(&endpoints);
        let keys: Vec<String> = install_config
            .ssh_authorized_keys
            .iter()
            .filter(|k| !k.trim().is_empty())
            .cloned()
            .collect();
        let ssh_keys = if keys.is_empty() {
            vec![String::new()]
        } else {
            keys
        };
        let has_static_hostname = install_config
            .hostname
            .as_ref()
            .is_some_and(|h| !h.trim().is_empty())
            || install_config.hostname_template.is_some();
        let hostname_from_dhcp = !has_static_hostname;
        let hostname_input = install_config.hostname.clone().unwrap_or_default();
        let hostname_from_template = install_config.hostname_template.is_some();
        let tailscale_input = install_config.tailscale_authkey.clone().unwrap_or_default();
        let config_password_hash = install_config.password_hash.clone();
        let timezone_from_config = install_config.timezone.clone();

        let timezone_selected = timezone_from_config.unwrap_or_else(|| "UTC".to_string());
        let timezone_filtered: Vec<usize> = (0..available_timezones.len()).collect();
        let timezone_cursor = available_timezones
            .iter()
            .position(|z| z == &timezone_selected)
            .unwrap_or(0);

        // r[impl installer.tui.network-config+13]
        let iso_network_mode = install_config.iso_network_mode.unwrap_or(NetworkMode::Dhcp);
        let iso_static_config = StaticNetConfig {
            interface: install_config
                .iso_network_interface
                .clone()
                .unwrap_or_default(),
            ip_cidr: install_config.iso_network_ip.clone().unwrap_or_default(),
            gateway: install_config
                .iso_network_gateway
                .clone()
                .unwrap_or_default(),
            dns: install_config.iso_network_dns.clone().unwrap_or_default(),
            search_domain: install_config
                .iso_network_domain
                .clone()
                .unwrap_or_default(),
        };

        let config_has_network_mode = install_config.network_mode.is_some();
        let target_network_mode = if let Some(mode) = install_config.network_mode {
            match mode {
                NetworkMode::Dhcp => TargetNetworkMode::Dhcp,
                NetworkMode::StaticIp => TargetNetworkMode::StaticIp,
                NetworkMode::Ipv6Slaac => TargetNetworkMode::Ipv6Slaac,
                NetworkMode::Offline => TargetNetworkMode::Offline,
            }
        } else {
            TargetNetworkMode::CopyCurrent
        };
        let target_static_config = StaticNetConfig {
            interface: install_config.network_interface.clone().unwrap_or_default(),
            ip_cidr: install_config.network_ip.clone().unwrap_or_default(),
            gateway: install_config.network_gateway.clone().unwrap_or_default(),
            dns: install_config.network_dns.clone().unwrap_or_default(),
            search_domain: install_config.network_domain.clone().unwrap_or_default(),
        };

        let mut state = Self {
            screen: Screen::Welcome,
            selected_disk_index: default_disk_index.unwrap_or(0),
            devices,
            disk_encryption,
            tpm_present,
            boot_device,
            write_progress: None,
            confirm_input: String::new(),
            build_info,
            hostname_input,
            hostname_from_dhcp,
            hostname_from_template,
            hostname_error: None,
            tailscale_input,
            ssh_keys,
            ssh_key_cursor: 0,
            password_input: String::new(),
            password_confirm_input: String::new(),
            password_confirming: false,
            password_mismatch: false,
            password_empty: false,
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
            recovery_passphrase: None,
            verity_check: if verity_active {
                VerityCheckState::Running
            } else {
                VerityCheckState::NotNeeded
            },
            verity_progress: None,
            verity_rx: None,
            iso_network_mode,
            iso_static_config,
            target_network_mode,
            target_static_config,
            detected_interfaces: Vec::new(),
            iso_net_status: NetConnectivityStatus::Unknown,
            net_config_pane: NetConfigPane::Iso,
            net_config_focus: NetConfigFocus::IsoMode,
            net_apply_debounce: None,
            target_pane_touched: false,
            config_has_network_mode,
            offline_target_warning: false,
        };
        state.ensure_trailing_blank();
        state
    }

    // r[impl iso.verity.check+6]
    // r[impl installer.tui.welcome+8]
    /// Spawn the background integrity check thread. Must be called once after
    /// construction when `verity_active` is true.
    pub fn start_verity_check(&mut self, manifest: &PartitionManifest, images_dir: &Path) {
        if self.verity_check != VerityCheckState::Running {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.verity_rx = Some(rx);

        let manifest = manifest.clone();
        let images_dir = images_dir.to_path_buf();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let image_files = writer::image_file_sizes(&manifest, &images_dir)?;
                writer::integrity_check(&images_dir, &image_files, &mut |progress| {
                    let _ = tx.send(VerityMessage::Progress(progress.into()));
                })?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    let _ = tx.send(VerityMessage::Done);
                }
                Err(e) => {
                    let _ = tx.send(VerityMessage::Error(format!("{e:#}")));
                }
            }
        });
    }

    /// Poll for verity integrity check progress. Returns true if state changed.
    pub fn poll_verity_check(&mut self) -> bool {
        let rx = match self.verity_rx.as_ref() {
            Some(rx) => rx,
            None => return false,
        };
        let mut changed = false;
        while let Ok(msg) = rx.try_recv() {
            changed = true;
            match msg {
                VerityMessage::Progress(snap) => {
                    self.verity_progress = Some(snap);
                }
                VerityMessage::Done => {
                    self.verity_check = VerityCheckState::Passed;
                }
                VerityMessage::Error(e) => {
                    self.verity_check = VerityCheckState::Failed(e);
                }
            }
        }
        changed
    }

    /// Whether the user is allowed to advance past the welcome screen.
    pub fn verity_check_allows_advance(&self) -> bool {
        matches!(
            self.verity_check,
            VerityCheckState::NotNeeded | VerityCheckState::Passed
        )
    }

    // r[impl installer.tui.disk-encryption+2]
    // r[impl installer.tui.hostname+6]
    // r[impl installer.tui.tailscale+3]
    // r[impl installer.tui.ssh-keys+5]
    // r[impl installer.tui.password+4]
    // r[impl installer.tui.timezone]
    // r[impl installer.finalise.network+4]
    /// Build an `InstallConfig` from the current interactive input fields.
    ///
    /// Network configuration is always included (defaulting to DHCP).
    /// The effective target network mode is resolved here (i.e. `CopyCurrent`
    /// is replaced by the corresponding ISO mode).
    pub fn install_config_fields(&self) -> Option<InstallConfig> {
        let hostname = if self.hostname_required() && !self.hostname_input.trim().is_empty() {
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

        let effective_mode = self.effective_target_network_mode();
        let static_cfg = self.effective_target_static_config();

        let network_mode = Some(effective_mode);
        let (network_interface, network_ip, network_gateway, network_dns, network_domain) =
            match effective_mode {
                NetworkMode::StaticIp => {
                    let iface = if static_cfg.interface.is_empty() {
                        None
                    } else {
                        Some(static_cfg.interface.clone())
                    };
                    let ip = if static_cfg.ip_cidr.is_empty() {
                        None
                    } else {
                        Some(static_cfg.ip_cidr.clone())
                    };
                    let gw = if static_cfg.gateway.is_empty() {
                        None
                    } else {
                        Some(static_cfg.gateway.clone())
                    };
                    let dns = if static_cfg.dns.is_empty() {
                        None
                    } else {
                        Some(static_cfg.dns.clone())
                    };
                    let domain = if static_cfg.search_domain.is_empty() {
                        None
                    } else {
                        Some(static_cfg.search_domain.clone())
                    };
                    (iface, ip, gw, dns, domain)
                }
                NetworkMode::Ipv6Slaac => {
                    let iface = if static_cfg.interface.is_empty() {
                        None
                    } else {
                        Some(static_cfg.interface.clone())
                    };
                    (iface, None, None, None, None)
                }
                _ => (None, None, None, None, None),
            };

        Some(InstallConfig {
            hostname,
            hostname_from_dhcp: self.hostname_from_dhcp,
            hostname_template: None,
            tailscale_authkey,
            ssh_authorized_keys,
            password,
            password_hash,
            timezone,
            network_mode,
            network_interface,
            network_ip,
            network_gateway,
            network_dns,
            network_domain,
            ..Default::default()
        })
    }

    pub fn selected_disk(&self) -> Option<&BlockDevice> {
        self.devices.get(self.selected_disk_index)
    }

    // r[impl installer.tui.disk-detection+4]
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

    // r[impl installer.tui.disk-encryption+2]
    pub fn cycle_disk_encryption(&mut self) {
        if self.tpm_present {
            self.disk_encryption = match self.disk_encryption {
                DiskEncryption::Keyfile => DiskEncryption::None,
                DiskEncryption::None => DiskEncryption::Tpm,
                DiskEncryption::Tpm => DiskEncryption::Keyfile,
            };
        } else {
            self.disk_encryption = match self.disk_encryption {
                DiskEncryption::Keyfile => DiskEncryption::None,
                _ => DiskEncryption::Keyfile,
            };
        }
    }

    pub fn cycle_disk_encryption_reverse(&mut self) {
        if self.tpm_present {
            self.disk_encryption = match self.disk_encryption {
                DiskEncryption::Keyfile => DiskEncryption::Tpm,
                DiskEncryption::Tpm => DiskEncryption::None,
                DiskEncryption::None => DiskEncryption::Keyfile,
            };
        } else {
            self.disk_encryption = match self.disk_encryption {
                DiskEncryption::Keyfile => DiskEncryption::None,
                _ => DiskEncryption::Keyfile,
            };
        }
    }

    // r[impl installer.tui.network-check+6]
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

    // r[impl installer.tui.tailscale-netcheck+3]
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
                    while self.ssh_keys.last().is_some_and(|k| k.trim().is_empty()) {
                        self.ssh_keys.pop();
                    }
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

    // r[impl installer.tui.disk-encryption+2]
    // r[impl installer.tui.hostname+6]
    // r[impl installer.tui.password+4]
    // r[impl installer.tui.timezone]
    pub fn advance(&mut self) {
        self.screen = match &self.screen {
            // r[impl installer.tui.welcome+8]
            Screen::Welcome => {
                if !self.verity_check_allows_advance() {
                    return;
                }
                // r[impl installer.tui.network-config+13]
                Screen::NetworkConfig
            }
            // r[impl installer.tui.network-config+13]
            Screen::NetworkConfig => {
                if !self.try_advance_from_network_config() {
                    return;
                }
                self.ensure_net_checks_started();
                Screen::DiskSelection
            }
            Screen::NetworkCheck => return,
            Screen::DiskSelection => Screen::DiskEncryption,
            Screen::DiskEncryption => Screen::Hostname,
            Screen::Hostname => {
                if self.hostname_from_dhcp {
                    Screen::Login
                } else {
                    Screen::HostnameInput
                }
            }
            Screen::HostnameInput => Screen::Login,
            Screen::Login => Screen::Timezone,
            Screen::LoginTailscale | Screen::LoginSshKeys | Screen::LoginGithub => return,
            Screen::Timezone => Screen::NetworkResults,
            // r[impl installer.tui.confirmation+8]
            // r[impl installer.encryption.recovery-passphrase+3]
            Screen::NetworkResults => {
                if self.disk_encryption.is_encrypted() && self.recovery_passphrase.is_none() {
                    self.recovery_passphrase =
                        Some(crate::encryption::generate_recovery_passphrase());
                }
                Screen::Confirmation
            }
            // r[impl installer.tui.progress+4]
            Screen::Confirmation => Screen::Installing,
            Screen::Installing => return,
            Screen::Done | Screen::Error(_) => return,
        };
    }

    pub fn go_back(&mut self) {
        self.screen = match &self.screen {
            Screen::NetworkConfig => Screen::Welcome,
            Screen::NetworkCheck => Screen::NetworkConfig,
            Screen::DiskSelection => Screen::NetworkConfig,
            Screen::DiskEncryption => Screen::DiskSelection,
            Screen::Hostname => Screen::DiskEncryption,
            Screen::HostnameInput => Screen::Hostname,
            Screen::Login => {
                if self.hostname_from_dhcp {
                    Screen::Hostname
                } else {
                    Screen::HostnameInput
                }
            }
            Screen::LoginTailscale | Screen::LoginSshKeys | Screen::LoginGithub => Screen::Login,
            Screen::Timezone => Screen::Login,
            Screen::NetworkResults => Screen::Timezone,
            Screen::Confirmation => Screen::NetworkResults,
            // No going back from installing/done — those are post-write
            _ => return,
        };
    }

    /// Enter the dedicated network check screen from the network config screen.
    pub fn open_network_check(&mut self) {
        self.ensure_net_checks_started();
        self.screen = Screen::NetworkCheck;
    }

    // ---- Network config screen helpers ----

    /// Visible fields in the ISO pane for the current mode.
    fn iso_visible_fields(&self) -> Vec<NetConfigFocus> {
        let mut fields = vec![NetConfigFocus::IsoMode];
        if self.iso_network_mode == NetworkMode::StaticIp {
            fields.extend([
                NetConfigFocus::IsoInterface,
                NetConfigFocus::IsoIp,
                NetConfigFocus::IsoGateway,
                NetConfigFocus::IsoDns,
                NetConfigFocus::IsoDomain,
            ]);
        }
        fields
    }

    /// Visible fields in the target pane for the current mode.
    fn target_visible_fields(&self) -> Vec<NetConfigFocus> {
        let mut fields = vec![NetConfigFocus::TargetMode];
        if matches!(self.target_network_mode, TargetNetworkMode::StaticIp) {
            fields.extend([
                NetConfigFocus::TargetInterface,
                NetConfigFocus::TargetIp,
                NetConfigFocus::TargetGateway,
                NetConfigFocus::TargetDns,
                NetConfigFocus::TargetDomain,
            ]);
        }
        fields
    }

    /// Move focus forward (Tab). When at the end of a pane, switch to the
    /// other pane.
    pub fn tab_focus_forward(&mut self) {
        let fields = match self.net_config_pane {
            NetConfigPane::Iso => self.iso_visible_fields(),
            NetConfigPane::Target => self.target_visible_fields(),
        };
        if let Some(pos) = fields.iter().position(|f| *f == self.net_config_focus) {
            if pos + 1 < fields.len() {
                self.apply_cidr_auto_suffix();
                self.net_config_focus = fields[pos + 1];
            } else {
                self.apply_cidr_auto_suffix();
                self.switch_pane_forward();
            }
        } else {
            self.apply_cidr_auto_suffix();
            self.switch_pane_forward();
        }
    }

    /// Move focus backward (Shift+Tab). When at the start of a pane, switch
    /// to the other pane.
    pub fn tab_focus_backward(&mut self) {
        let fields = match self.net_config_pane {
            NetConfigPane::Iso => self.iso_visible_fields(),
            NetConfigPane::Target => self.target_visible_fields(),
        };
        if let Some(pos) = fields.iter().position(|f| *f == self.net_config_focus) {
            if pos > 0 {
                self.apply_cidr_auto_suffix();
                self.net_config_focus = fields[pos - 1];
            } else {
                self.apply_cidr_auto_suffix();
                self.switch_pane_backward();
            }
        } else {
            self.apply_cidr_auto_suffix();
            self.switch_pane_backward();
        }
    }

    fn switch_pane_forward(&mut self) {
        match self.net_config_pane {
            NetConfigPane::Iso => {
                self.net_config_pane = NetConfigPane::Target;
                let fields = self.target_visible_fields();
                self.net_config_focus = fields[0];
            }
            NetConfigPane::Target => {
                self.net_config_pane = NetConfigPane::Iso;
                let fields = self.iso_visible_fields();
                self.net_config_focus = fields[0];
            }
        }
    }

    fn switch_pane_backward(&mut self) {
        match self.net_config_pane {
            NetConfigPane::Iso => {
                self.net_config_pane = NetConfigPane::Target;
                let fields = self.target_visible_fields();
                self.net_config_focus = *fields.last().unwrap_or(&NetConfigFocus::TargetMode);
            }
            NetConfigPane::Target => {
                self.net_config_pane = NetConfigPane::Iso;
                let fields = self.iso_visible_fields();
                self.net_config_focus = *fields.last().unwrap_or(&NetConfigFocus::IsoMode);
            }
        }
    }

    /// Apply CIDR auto-suffix when leaving an IP field.
    fn apply_cidr_auto_suffix(&mut self) {
        match self.net_config_focus {
            NetConfigFocus::IsoIp => self.iso_static_config.auto_suffix_cidr(),
            NetConfigFocus::TargetIp => self.target_static_config.auto_suffix_cidr(),
            _ => {}
        }
    }

    /// Cycle the ISO network mode forward.
    pub fn cycle_iso_mode_forward(&mut self) {
        self.iso_network_mode = match self.iso_network_mode {
            NetworkMode::Dhcp => NetworkMode::StaticIp,
            NetworkMode::StaticIp => NetworkMode::Ipv6Slaac,
            NetworkMode::Ipv6Slaac => NetworkMode::Offline,
            NetworkMode::Offline => NetworkMode::Dhcp,
        };
        self.on_iso_mode_changed();
    }

    /// Cycle the ISO network mode backward.
    pub fn cycle_iso_mode_backward(&mut self) {
        self.iso_network_mode = match self.iso_network_mode {
            NetworkMode::Dhcp => NetworkMode::Offline,
            NetworkMode::StaticIp => NetworkMode::Dhcp,
            NetworkMode::Ipv6Slaac => NetworkMode::StaticIp,
            NetworkMode::Offline => NetworkMode::Ipv6Slaac,
        };
        self.on_iso_mode_changed();
    }

    fn on_iso_mode_changed(&mut self) {
        self.iso_net_status = net::NetConnectivityStatus::Unknown;
        self.schedule_iso_apply();
        // If the target pane is still on CopyCurrent and ISO goes offline,
        // auto-switch target to DHCP to avoid copying an offline config.
        if self.iso_network_mode == NetworkMode::Offline
            && self.target_network_mode == TargetNetworkMode::CopyCurrent
            && !self.target_pane_touched
        {
            self.target_network_mode = TargetNetworkMode::Dhcp;
        }
    }

    /// Cycle the target network mode forward.
    pub fn cycle_target_mode_forward(&mut self) {
        self.target_pane_touched = true;
        self.target_network_mode = if self.config_has_network_mode {
            match self.target_network_mode {
                TargetNetworkMode::Dhcp => TargetNetworkMode::StaticIp,
                TargetNetworkMode::StaticIp => TargetNetworkMode::Ipv6Slaac,
                TargetNetworkMode::Ipv6Slaac => TargetNetworkMode::Offline,
                TargetNetworkMode::Offline => TargetNetworkMode::Dhcp,
                TargetNetworkMode::CopyCurrent => TargetNetworkMode::Dhcp,
            }
        } else {
            match self.target_network_mode {
                TargetNetworkMode::CopyCurrent => TargetNetworkMode::Dhcp,
                TargetNetworkMode::Dhcp => TargetNetworkMode::StaticIp,
                TargetNetworkMode::StaticIp => TargetNetworkMode::Ipv6Slaac,
                TargetNetworkMode::Ipv6Slaac => TargetNetworkMode::Offline,
                TargetNetworkMode::Offline => TargetNetworkMode::CopyCurrent,
            }
        };
    }

    /// Cycle the target network mode backward.
    pub fn cycle_target_mode_backward(&mut self) {
        self.target_pane_touched = true;
        self.target_network_mode = if self.config_has_network_mode {
            match self.target_network_mode {
                TargetNetworkMode::Dhcp => TargetNetworkMode::Offline,
                TargetNetworkMode::StaticIp => TargetNetworkMode::Dhcp,
                TargetNetworkMode::Ipv6Slaac => TargetNetworkMode::StaticIp,
                TargetNetworkMode::Offline => TargetNetworkMode::Ipv6Slaac,
                TargetNetworkMode::CopyCurrent => TargetNetworkMode::Offline,
            }
        } else {
            match self.target_network_mode {
                TargetNetworkMode::CopyCurrent => TargetNetworkMode::Offline,
                TargetNetworkMode::Dhcp => TargetNetworkMode::CopyCurrent,
                TargetNetworkMode::StaticIp => TargetNetworkMode::Dhcp,
                TargetNetworkMode::Ipv6Slaac => TargetNetworkMode::StaticIp,
                TargetNetworkMode::Offline => TargetNetworkMode::Ipv6Slaac,
            }
        };
    }

    /// Cycle the interface selection for the focused dropdown.
    pub fn cycle_interface(&mut self, reverse: bool) {
        let current = match self.net_config_focus {
            NetConfigFocus::IsoInterface => &mut self.iso_static_config.interface,
            NetConfigFocus::TargetInterface => &mut self.target_static_config.interface,
            _ => return,
        };
        if self.detected_interfaces.is_empty() {
            return;
        }
        let names: Vec<&str> = self
            .detected_interfaces
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        let pos = names.iter().position(|n| *n == current.as_str());
        let new_pos = if reverse {
            match pos {
                Some(0) | None => names.len() - 1,
                Some(p) => p - 1,
            }
        } else {
            match pos {
                Some(p) if p + 1 < names.len() => p + 1,
                _ => 0,
            }
        };
        *current = names[new_pos].to_string();
        if self.net_config_focus == NetConfigFocus::IsoInterface {
            self.schedule_iso_apply();
        }
    }

    /// Push a character into the currently focused text field.
    pub fn net_config_push_char(&mut self, c: char) {
        match self.net_config_focus {
            NetConfigFocus::IsoIp => self.iso_static_config.ip_cidr.push(c),
            NetConfigFocus::IsoGateway => self.iso_static_config.gateway.push(c),
            NetConfigFocus::IsoDns => self.iso_static_config.dns.push(c),
            NetConfigFocus::IsoDomain => self.iso_static_config.search_domain.push(c),
            NetConfigFocus::TargetIp => self.target_static_config.ip_cidr.push(c),
            NetConfigFocus::TargetGateway => self.target_static_config.gateway.push(c),
            NetConfigFocus::TargetDns => self.target_static_config.dns.push(c),
            NetConfigFocus::TargetDomain => self.target_static_config.search_domain.push(c),
            _ => return,
        }
        if self.net_config_focus.pane() == NetConfigPane::Iso {
            self.schedule_iso_apply();
        }
    }

    /// Delete the last character from the currently focused text field.
    pub fn net_config_backspace(&mut self) {
        match self.net_config_focus {
            NetConfigFocus::IsoIp => {
                self.iso_static_config.ip_cidr.pop();
            }
            NetConfigFocus::IsoGateway => {
                self.iso_static_config.gateway.pop();
            }
            NetConfigFocus::IsoDns => {
                self.iso_static_config.dns.pop();
            }
            NetConfigFocus::IsoDomain => {
                self.iso_static_config.search_domain.pop();
            }
            NetConfigFocus::TargetIp => {
                self.target_static_config.ip_cidr.pop();
            }
            NetConfigFocus::TargetGateway => {
                self.target_static_config.gateway.pop();
            }
            NetConfigFocus::TargetDns => {
                self.target_static_config.dns.pop();
            }
            NetConfigFocus::TargetDomain => {
                self.target_static_config.search_domain.pop();
            }
            _ => return,
        }
        if self.net_config_focus.pane() == NetConfigPane::Iso {
            self.schedule_iso_apply();
        }
    }

    /// Mark that the ISO netplan needs to be re-applied after the debounce period.
    fn schedule_iso_apply(&mut self) {
        self.net_apply_debounce = Some(Instant::now());
    }

    /// Check if the debounce timer has expired and return true if we should
    /// apply the ISO netplan now. Clears the timer when it fires.
    pub fn poll_iso_apply_debounce(&mut self) -> bool {
        if let Some(instant) = self.net_apply_debounce
            && instant.elapsed() >= Duration::from_millis(500)
        {
            self.net_apply_debounce = None;
            return true;
        }
        false
    }

    /// Actually apply the ISO netplan configuration. Called from the event
    /// loop after the debounce fires.
    pub fn apply_iso_netplan(&mut self) {
        self.iso_net_status = net::NetConnectivityStatus::Configuring;
        match net::apply_netplan(self.iso_network_mode, &self.iso_static_config) {
            Ok(()) => {
                self.iso_net_status = net::probe_connectivity();
                self.start_net_checks();
                self.start_tailscale_netcheck();
            }
            Err(e) => {
                tracing::warn!("netplan apply failed: {e}");
                self.iso_net_status = net::NetConnectivityStatus::NoConnectivity;
            }
        }
    }

    /// Populate `detected_interfaces` from the system. Called once when
    /// entering the NetworkConfig screen.
    pub fn detect_network_interfaces(&mut self) {
        self.detected_interfaces = net::detect_interfaces();
    }

    /// Try to advance from the NetworkConfig screen. Returns false if
    /// blocked by the offline warning dialog.
    pub fn try_advance_from_network_config(&mut self) -> bool {
        if self.target_network_mode == TargetNetworkMode::Offline && !self.offline_target_warning {
            self.offline_target_warning = true;
            return false;
        }
        self.offline_target_warning = false;
        true
    }

    /// Build a human-readable summary of the target network configuration
    /// for the confirmation screen and install plan.
    pub fn network_summary(&self) -> String {
        let effective_mode = match self.target_network_mode {
            TargetNetworkMode::CopyCurrent => match self.iso_network_mode {
                NetworkMode::Dhcp => TargetNetworkMode::Dhcp,
                NetworkMode::StaticIp => TargetNetworkMode::StaticIp,
                NetworkMode::Ipv6Slaac => TargetNetworkMode::Ipv6Slaac,
                NetworkMode::Offline => TargetNetworkMode::Offline,
            },
            other => other,
        };
        let static_cfg = match self.target_network_mode {
            TargetNetworkMode::CopyCurrent => &self.iso_static_config,
            _ => &self.target_static_config,
        };
        match effective_mode {
            TargetNetworkMode::CopyCurrent => unreachable!(),
            TargetNetworkMode::Dhcp => "DHCP (all Ethernet interfaces)".to_string(),
            TargetNetworkMode::StaticIp => {
                let iface = if static_cfg.interface.is_empty() {
                    "en*"
                } else {
                    &static_cfg.interface
                };
                let mut s = format!(
                    "Static IP: {} via {} on {}",
                    static_cfg.ip_cidr, static_cfg.gateway, iface
                );
                if !static_cfg.dns.is_empty() {
                    s.push_str(&format!("\n                DNS: {}", static_cfg.dns));
                }
                s
            }
            TargetNetworkMode::Ipv6Slaac => "IPv6 SLAAC only".to_string(),
            TargetNetworkMode::Offline => "Offline (no network configuration)".to_string(),
        }
    }

    /// Resolve the effective target `NetworkMode` (resolving `CopyCurrent`).
    pub fn effective_target_network_mode(&self) -> NetworkMode {
        match self.target_network_mode {
            TargetNetworkMode::CopyCurrent => self.iso_network_mode,
            TargetNetworkMode::Dhcp => NetworkMode::Dhcp,
            TargetNetworkMode::StaticIp => NetworkMode::StaticIp,
            TargetNetworkMode::Ipv6Slaac => NetworkMode::Ipv6Slaac,
            TargetNetworkMode::Offline => NetworkMode::Offline,
        }
    }

    /// Get the effective target static config (resolving `CopyCurrent`).
    pub fn effective_target_static_config(&self) -> &StaticNetConfig {
        match self.target_network_mode {
            TargetNetworkMode::CopyCurrent => &self.iso_static_config,
            _ => &self.target_static_config,
        }
    }

    pub fn confirmation_text(&self) -> &str {
        "yes"
    }

    // r[impl installer.tui.confirmation+8]
    pub fn is_confirmed(&self) -> bool {
        self.confirm_input
            .trim()
            .eq_ignore_ascii_case(self.confirmation_text())
    }

    // r[impl installer.tui.hostname+6]
    pub fn hostname_required(&self) -> bool {
        !self.hostname_from_dhcp
    }

    // r[impl installer.tui.network-check+6]
    /// Whether github.com is reachable per background network checks.
    pub fn github_reachable(&self) -> bool {
        self.net_check_results
            .iter()
            .any(|r| matches!(r, Some(r) if r.label == "github.com" && r.passed))
    }

    // r[impl installer.tui.ssh-keys+5]

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

    // r[impl installer.tui.password+4]
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
        make_state_with_tpm(true)
    }

    fn make_state_with_tpm(tpm_present: bool) -> AppState {
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
        let default_encryption = DiskEncryption::Keyfile;
        AppState::new(
            devices,
            default_encryption,
            tpm_present,
            &InstallConfig::default(),
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        )
    }

    // r[verify installer.tui.welcome+8]
    #[test]
    fn initial_state() {
        let state = make_state();
        assert_eq!(state.screen, Screen::Welcome);
        assert_eq!(state.selected_disk_index, 0);
        assert_eq!(state.disk_encryption, DiskEncryption::Keyfile);
        assert!(state.tpm_present);
        assert_eq!(state.net_check_phase, CheckPhase::NotStarted);
        assert_eq!(state.netcheck_phase, CheckPhase::NotStarted);
        assert!(!state.net_checks_started);
        assert_eq!(state.verity_check, VerityCheckState::NotNeeded);
    }

    // r[verify installer.tui.welcome+8]
    // r[verify iso.verity.check+6]
    #[test]
    fn verity_running_blocks_advance() {
        let mut state = make_state();
        state.verity_check = VerityCheckState::Running;
        assert!(!state.verity_check_allows_advance());
        state.advance();
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.welcome+8]
    // r[verify iso.verity.check+6]
    #[test]
    fn verity_passed_allows_advance() {
        let mut state = make_state();
        state.verity_check = VerityCheckState::Passed;
        assert!(state.verity_check_allows_advance());
        state.advance();
        assert_eq!(state.screen, Screen::NetworkConfig);
    }

    // r[verify installer.tui.welcome+8]
    // r[verify iso.verity.check+6]
    #[test]
    fn verity_not_needed_allows_advance() {
        let mut state = make_state();
        assert_eq!(state.verity_check, VerityCheckState::NotNeeded);
        assert!(state.verity_check_allows_advance());
        state.advance();
        assert_eq!(state.screen, Screen::NetworkConfig);
    }

    // r[verify iso.verity.check+6]
    #[test]
    fn verity_failed_does_not_allow_advance() {
        let state = AppState {
            verity_check: VerityCheckState::Failed("corrupt".into()),
            ..make_state()
        };
        assert!(!state.verity_check_allows_advance());
    }

    // r[verify iso.verity.check+6]
    #[test]
    fn poll_verity_progress_updates_state() {
        let mut state = make_state();
        state.verity_check = VerityCheckState::Running;
        let (tx, rx) = std::sync::mpsc::channel();
        state.verity_rx = Some(rx);

        tx.send(VerityMessage::Progress(ProgressSnapshot {
            bytes_written: 500,
            total_bytes: Some(1000),
            throughput_mbps: 10.0,
            eta: None,
            phase: InstallPhase::Writing,
        }))
        .unwrap();

        assert!(state.poll_verity_check());
        assert_eq!(state.verity_progress.as_ref().unwrap().bytes_written, 500);
        assert_eq!(state.verity_check, VerityCheckState::Running);
    }

    // r[verify iso.verity.check+6]
    #[test]
    fn poll_verity_done_transitions_to_passed() {
        let mut state = make_state();
        state.verity_check = VerityCheckState::Running;
        let (tx, rx) = std::sync::mpsc::channel();
        state.verity_rx = Some(rx);

        tx.send(VerityMessage::Done).unwrap();

        assert!(state.poll_verity_check());
        assert_eq!(state.verity_check, VerityCheckState::Passed);
    }

    // r[verify iso.verity.check+6]
    #[test]
    fn poll_verity_error_transitions_to_failed() {
        let mut state = make_state();
        state.verity_check = VerityCheckState::Running;
        let (tx, rx) = std::sync::mpsc::channel();
        state.verity_rx = Some(rx);

        tx.send(VerityMessage::Error("bad media".into())).unwrap();

        assert!(state.poll_verity_check());
        assert_eq!(
            state.verity_check,
            VerityCheckState::Failed("bad media".into())
        );
    }

    // r[verify installer.tui.welcome+8]
    // r[verify iso.verity.check+6]
    #[test]
    fn verity_running_still_allows_network_check() {
        let mut state = make_state();
        state.verity_check = VerityCheckState::Running;
        state.open_network_check();
        assert_eq!(state.screen, Screen::NetworkCheck);
    }

    // r[verify installer.tui.network-check+6]
    #[test]
    fn welcome_advances_to_network_config() {
        let mut state = make_state();
        state.verity_check = VerityCheckState::NotNeeded;
        state.advance();
        assert_eq!(state.screen, Screen::NetworkConfig);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_config_advances_to_disk_selection() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.advance();
        assert_eq!(state.screen, Screen::DiskSelection);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_config_goes_back_to_welcome() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.go_back();
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn open_network_check_from_network_config() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.open_network_check();
        assert_eq!(state.screen, Screen::NetworkCheck);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_check_goes_back_to_network_config() {
        let mut state = make_state();
        state.screen = Screen::NetworkCheck;
        state.go_back();
        assert_eq!(state.screen, Screen::NetworkConfig);
    }

    // r[verify installer.tui.network-check+6]
    #[test]
    fn network_check_advance_is_noop() {
        let mut state = make_state();
        state.screen = Screen::NetworkCheck;
        state.advance();
        assert_eq!(state.screen, Screen::NetworkCheck);
    }

    // r[verify installer.tui.network-check+6]
    #[test]
    fn disk_selection_goes_back_to_network_config() {
        let mut state = make_state();
        state.screen = Screen::DiskSelection;
        state.go_back();
        assert_eq!(state.screen, Screen::NetworkConfig);
    }

    // r[verify installer.tui.network-check+6]
    #[test]
    fn timezone_advances_to_network_results() {
        let mut state = make_state();
        state.screen = Screen::Timezone;
        state.timezone_selected = "UTC".into();
        state.advance();
        assert_eq!(state.screen, Screen::NetworkResults);
    }

    // r[verify installer.tui.network-check+6]
    #[test]
    fn network_results_advances_to_confirmation() {
        let mut state = make_state();
        state.screen = Screen::NetworkResults;
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.network-check+6]
    #[test]
    fn network_results_goes_back_to_timezone() {
        let mut state = make_state();
        state.screen = Screen::NetworkResults;
        state.go_back();
        assert_eq!(state.screen, Screen::Timezone);
    }

    // r[verify installer.tui.network-check+6]
    #[test]
    fn confirmation_goes_back_to_network_results() {
        let mut state = make_state();
        state.screen = Screen::Confirmation;
        state.go_back();
        assert_eq!(state.screen, Screen::NetworkResults);
    }

    // r[verify installer.tui.disk-detection+4]
    #[test]
    fn disk_navigation_wraps() {
        let mut state = make_state();
        assert_eq!(state.selected_disk_index, 0);
        state.select_prev_disk();
        assert_eq!(state.selected_disk_index, 1);
        state.select_next_disk();
        assert_eq!(state.selected_disk_index, 0);
    }

    // r[verify installer.tui.disk-encryption+2]
    #[test]
    fn disk_encryption_cycle_with_tpm() {
        let mut state = make_state();
        assert_eq!(state.disk_encryption, DiskEncryption::Keyfile);
        state.cycle_disk_encryption();
        assert_eq!(state.disk_encryption, DiskEncryption::None);
        state.cycle_disk_encryption();
        assert_eq!(state.disk_encryption, DiskEncryption::Tpm);
        state.cycle_disk_encryption();
        assert_eq!(state.disk_encryption, DiskEncryption::Keyfile);
    }

    // r[verify installer.tui.disk-encryption+2]
    #[test]
    fn disk_encryption_cycle_without_tpm() {
        let mut state = make_state_with_tpm(false);
        assert_eq!(state.disk_encryption, DiskEncryption::Keyfile);
        state.cycle_disk_encryption();
        assert_eq!(state.disk_encryption, DiskEncryption::None);
        state.cycle_disk_encryption();
        assert_eq!(state.disk_encryption, DiskEncryption::Keyfile);
    }

    // r[verify installer.tui.disk-encryption+2]
    // r[verify installer.tui.password+4]
    #[test]
    fn advance_encrypted_flow() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::Tpm;

        assert_eq!(state.screen, Screen::Welcome);
        state.advance();
        assert_eq!(state.screen, Screen::NetworkConfig);
        state.advance();
        assert_eq!(state.screen, Screen::DiskSelection);
        state.advance();
        assert_eq!(state.screen, Screen::DiskEncryption);
        state.advance();
        assert_eq!(state.screen, Screen::Hostname);
        // Network-assigned (DHCP) is the default regardless of encryption,
        // so advance skips HostnameInput and goes to Login directly.
        state.advance();
        assert_eq!(state.screen, Screen::Login);
        state.advance();
        assert_eq!(state.screen, Screen::Timezone);
        state.advance();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.disk-encryption+2]
    // r[verify installer.tui.password+4]
    // r[verify installer.tui.timezone]
    #[test]
    fn advance_none_encryption_flow() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::None;
        state.hostname_from_dhcp = true;

        assert_eq!(state.screen, Screen::Welcome);
        state.advance();
        assert_eq!(state.screen, Screen::NetworkConfig);
        state.advance();
        assert_eq!(state.screen, Screen::DiskSelection);
        state.advance();
        assert_eq!(state.screen, Screen::DiskEncryption);
        state.advance();
        assert_eq!(state.screen, Screen::Hostname);
        // None encryption defaults to hostname_from_dhcp = true (network-assigned),
        // so advance skips HostnameInput and goes to Login directly.
        state.advance();
        assert_eq!(state.screen, Screen::Login);
        state.advance();
        assert_eq!(state.screen, Screen::Timezone);
        state.advance();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    #[test]
    fn advance_none_static_goes_to_hostname_input() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::None;
        state.hostname_from_dhcp = false;

        state.screen = Screen::Hostname;
        state.advance();
        assert_eq!(state.screen, Screen::HostnameInput);
    }

    #[test]
    fn advance_encrypted_dhcp_skips_hostname_input() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::Tpm;
        state.hostname_from_dhcp = true;

        state.screen = Screen::Hostname;
        state.advance();
        assert_eq!(state.screen, Screen::Login);
    }

    // r[verify installer.tui.disk-encryption+2]
    // r[verify installer.tui.password+4]
    // r[verify installer.tui.timezone]
    #[test]
    fn go_back_through_encrypted_flow() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::Tpm;
        state.screen = Screen::Confirmation;

        // hostname_from_dhcp is true (always the default when no static
        // hostname is configured), so Login goes back to Hostname selector
        // (not HostnameInput).
        state.go_back();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.go_back();
        assert_eq!(state.screen, Screen::Timezone);
        state.go_back();
        assert_eq!(state.screen, Screen::Login);
        state.go_back();
        assert_eq!(state.screen, Screen::Hostname);
        state.go_back();
        assert_eq!(state.screen, Screen::DiskEncryption);
        state.go_back();
        assert_eq!(state.screen, Screen::DiskSelection);
        state.go_back();
        assert_eq!(state.screen, Screen::NetworkConfig);
        state.go_back();
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.disk-encryption+2]
    // r[verify installer.tui.password+4]
    // r[verify installer.tui.timezone]
    #[test]
    fn go_back_none_encryption_flow() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::None;
        state.hostname_from_dhcp = true;
        state.screen = Screen::Confirmation;

        state.go_back();
        assert_eq!(state.screen, Screen::NetworkResults);
        state.go_back();
        assert_eq!(state.screen, Screen::Timezone);
        state.go_back();
        assert_eq!(state.screen, Screen::Login);
        // None encryption: hostname_from_dhcp is true by default, so Login goes back to Hostname selector
        state.go_back();
        assert_eq!(state.screen, Screen::Hostname);
        state.go_back();
        assert_eq!(state.screen, Screen::DiskEncryption);
    }

    #[test]
    fn go_back_from_login_with_dhcp_goes_to_hostname_selector() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::Tpm;
        state.hostname_from_dhcp = true;
        state.screen = Screen::Login;

        state.go_back();
        assert_eq!(state.screen, Screen::Hostname);
    }

    // r[verify installer.tui.ssh-keys+5]
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

    // r[verify installer.tui.ssh-keys+5]
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

    // r[verify installer.tui.confirmation+8]
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

    // r[verify installer.tui.confirmation+8]
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

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_config_prefilled_from_config() {
        use crate::config::NetworkMode as NM;
        let config = InstallConfig {
            network_mode: Some(NM::StaticIp),
            network_interface: Some("enp0s3".into()),
            network_ip: Some("192.168.1.10/24".into()),
            network_gateway: Some("192.168.1.1".into()),
            network_dns: Some("8.8.8.8".into()),
            network_domain: Some("example.com".into()),
            iso_network_mode: Some(NM::StaticIp),
            iso_network_interface: Some("eth0".into()),
            iso_network_ip: Some("10.0.0.5/24".into()),
            iso_network_gateway: Some("10.0.0.1".into()),
            iso_network_dns: Some("1.1.1.1".into()),
            iso_network_domain: Some("test.local".into()),
            ..Default::default()
        };
        let state = AppState::new(
            vec![],
            DiskEncryption::None,
            false,
            &config,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert_eq!(state.iso_network_mode, NM::StaticIp);
        assert_eq!(state.iso_static_config.interface, "eth0");
        assert_eq!(state.iso_static_config.ip_cidr, "10.0.0.5/24");
        assert_eq!(state.iso_static_config.gateway, "10.0.0.1");
        assert_eq!(state.iso_static_config.dns, "1.1.1.1");
        assert_eq!(state.iso_static_config.search_domain, "test.local");
        assert_eq!(state.target_network_mode, TargetNetworkMode::StaticIp);
        assert_eq!(state.target_static_config.interface, "enp0s3");
        assert_eq!(state.target_static_config.ip_cidr, "192.168.1.10/24");
        assert_eq!(state.target_static_config.gateway, "192.168.1.1");
        assert!(state.config_has_network_mode);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_config_defaults_to_copy_current() {
        let config = InstallConfig::default();
        let state = AppState::new(
            vec![],
            DiskEncryption::None,
            false,
            &config,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert_eq!(state.iso_network_mode, NetworkMode::Dhcp);
        assert_eq!(state.target_network_mode, TargetNetworkMode::CopyCurrent);
        assert!(!state.config_has_network_mode);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_summary_dhcp() {
        let state = make_state();
        // Default: CopyCurrent with ISO = DHCP => effective DHCP
        assert_eq!(state.network_summary(), "DHCP (all Ethernet interfaces)");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_summary_static() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::StaticIp;
        state.target_static_config = StaticNetConfig {
            interface: "enp0s3".into(),
            ip_cidr: "192.168.1.10/24".into(),
            gateway: "192.168.1.1".into(),
            dns: "8.8.8.8".into(),
            search_domain: String::new(),
        };
        let summary = state.network_summary();
        assert!(summary.contains("Static IP: 192.168.1.10/24 via 192.168.1.1 on enp0s3"));
        assert!(summary.contains("DNS: 8.8.8.8"));
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_summary_offline() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::Offline;
        assert_eq!(
            state.network_summary(),
            "Offline (no network configuration)"
        );
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_summary_ipv6_slaac() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::Ipv6Slaac;
        assert_eq!(state.network_summary(), "IPv6 SLAAC only");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn network_summary_copy_current_static() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::CopyCurrent;
        state.iso_network_mode = NetworkMode::StaticIp;
        state.iso_static_config = StaticNetConfig {
            interface: "eth0".into(),
            ip_cidr: "10.0.0.5/24".into(),
            gateway: "10.0.0.1".into(),
            dns: String::new(),
            search_domain: String::new(),
        };
        let summary = state.network_summary();
        assert!(summary.contains("Static IP: 10.0.0.5/24 via 10.0.0.1 on eth0"));
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn effective_target_mode_resolves_copy_current() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::CopyCurrent;
        state.iso_network_mode = NetworkMode::Ipv6Slaac;
        assert_eq!(
            state.effective_target_network_mode(),
            NetworkMode::Ipv6Slaac
        );
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn hostname_prefilled_defaults_to_static() {
        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let cfg = InstallConfig {
            hostname: Some("myhost".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::Tpm,
            true,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert!(!state.hostname_from_dhcp);
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn default_config_always_defaults_to_dhcp() {
        // With no static hostname configured, hostname_from_dhcp is always true
        // regardless of encryption type.
        let state = make_state();
        assert!(state.hostname_from_dhcp);

        use crate::disk::TransportType;
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let none_enc = AppState::new(
            devices,
            DiskEncryption::None,
            false,
            &InstallConfig::default(),
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert!(none_enc.hostname_from_dhcp);
    }

    // r[verify installer.tui.hostname+6]
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
        let cfg = InstallConfig {
            hostname: Some("myhost".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::Tpm,
            true,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
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
        let cfg = InstallConfig {
            tailscale_authkey: Some("tskey-auth-xxx".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::Tpm,
            true,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert_eq!(state.tailscale_input, "tskey-auth-xxx");
    }

    // r[verify installer.tui.ssh-keys+5]
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
        let cfg = InstallConfig {
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA key1".into(), "ssh-rsa BBBB key2".into()],
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::Tpm,
            true,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert_eq!(
            state.ssh_keys,
            vec!["ssh-ed25519 AAAA key1", "ssh-rsa BBBB key2", ""]
        );
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn install_config_from_inputs() {
        let mut state = make_state();
        // Network config is always included (defaults to DHCP), so
        // install_config_fields always returns Some.
        state.hostname_from_dhcp = false;
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.network_mode, Some(NetworkMode::Dhcp));
        assert!(cfg.hostname.is_none());

        state.hostname_from_dhcp = false;
        state.hostname_input = "server-01".into();
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.hostname.as_deref(), Some("server-01"));
        assert!(cfg.tailscale_authkey.is_none());
        assert!(cfg.ssh_authorized_keys.is_empty());
        assert!(cfg.password.is_none());
        assert!(cfg.password_hash.is_none());
        assert_eq!(cfg.network_mode, Some(NetworkMode::Dhcp));
    }

    // r[verify installer.tui.tailscale+3]
    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn install_config_all_fields() {
        let mut state = make_state();
        state.hostname_from_dhcp = false;
        state.hostname_input = "host".into();
        state.tailscale_input = "tskey-auth-123".into();
        state.ssh_keys = vec!["ssh-ed25519 AAAA".into(), "ssh-rsa BBBB".into()];
        state.password_input = "s3cret".into();

        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.hostname.as_deref(), Some("host"));
        assert_eq!(cfg.tailscale_authkey.as_deref(), Some("tskey-auth-123"));
        assert_eq!(cfg.ssh_authorized_keys.len(), 2);
        assert_eq!(cfg.password.as_deref(), Some("s3cret"));
        assert!(cfg.password_hash.is_none());
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn install_config_empty_strings_are_none() {
        let mut state = make_state();
        state.hostname_input = "   ".into();
        state.hostname_from_dhcp = false;
        state.tailscale_input = "  ".into();
        // Network config is always present (defaults to DHCP), so we
        // always get Some. Verify the non-network fields are None/empty.
        let cfg = state.install_config_fields().unwrap();
        assert!(cfg.hostname.is_none());
        assert!(!cfg.hostname_from_dhcp);
        assert!(cfg.tailscale_authkey.is_none());
        assert!(cfg.ssh_authorized_keys.is_empty());
        assert!(cfg.password.is_none());
        assert!(cfg.password_hash.is_none());
        assert_eq!(cfg.network_mode, Some(NetworkMode::Dhcp));
    }

    // r[verify installer.tui.password+4]
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

    // r[verify installer.tui.password+4]
    #[test]
    fn has_password_from_input() {
        let mut state = make_state();
        assert!(!state.has_password());

        state.password_input = "secret".into();
        assert!(state.has_password());
    }

    // r[verify installer.tui.password+4]
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
        let cfg = InstallConfig {
            password_hash: Some("$6$rounds=4096$salt$hash".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::Tpm,
            true,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert!(state.has_password());
        let cfg = state.install_config_fields().unwrap();
        assert!(cfg.password.is_none());
        assert_eq!(
            cfg.password_hash.as_deref(),
            Some("$6$rounds=4096$salt$hash")
        );
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn hostname_required_for_encrypted() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::Tpm;
        // hostname_from_dhcp is true by default (encryption no longer matters),
        // so hostname is not required unless the user toggles to Static.
        assert!(!state.hostname_required());
        state.hostname_from_dhcp = false;
        assert!(state.hostname_required());
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn hostname_required_for_none_static() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::None;
        state.hostname_from_dhcp = false;
        assert!(state.hostname_required());
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn hostname_not_required_for_none_dhcp() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::None;
        state.hostname_from_dhcp = true;
        assert!(!state.hostname_required());
    }

    // r[verify installer.tui.hostname+6]
    #[test]
    fn hostname_not_required_for_encrypted_with_dhcp() {
        let mut state = make_state();
        state.disk_encryption = DiskEncryption::Tpm;
        state.hostname_from_dhcp = true;
        assert!(!state.hostname_required());
    }

    #[test]
    fn install_config_with_dhcp_hostname() {
        let mut state = make_state();
        state.hostname_from_dhcp = true;
        let cfg = state.install_config_fields().unwrap();
        assert!(cfg.hostname_from_dhcp);
        assert!(cfg.hostname.is_none());
    }

    #[test]
    fn install_config_dhcp_overrides_hostname_input() {
        let mut state = make_state();
        state.hostname_from_dhcp = true;
        state.hostname_input = "should-be-ignored".into();
        let cfg = state.install_config_fields().unwrap();
        assert!(cfg.hostname_from_dhcp);
        assert!(cfg.hostname.is_none());
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
        let cfg = InstallConfig {
            hostname: Some("resolved-name".into()),
            hostname_template: Some("srv-{hex:6}".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::Tpm,
            true,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
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
        let cfg = InstallConfig {
            timezone: Some("Pacific/Auckland".into()),
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::None,
            false,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
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
        // Realistic state: one existing key followed by the trailing blank
        state.ssh_keys = vec!["ssh-ed25519 existing-key".into(), String::new()];

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
        assert_eq!(
            state.ssh_keys,
            vec!["ssh-ed25519 existing-key", "ssh-rsa fetched-key", ""],
            "trailing blank should be stripped before import, then re-added at the end"
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

    // r[verify installer.tui.ssh-keys.github+4]
    #[test]
    fn github_key_fetch_from_empty_state_has_no_leading_blank() {
        let mut state = make_state();
        state.screen = Screen::LoginGithub;
        // Default state: just the trailing blank
        state.ssh_keys = vec![String::new()];

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
        assert_eq!(
            state.ssh_keys,
            vec!["ssh-rsa fetched-key", ""],
            "empty leading field must be removed, only trailing blank remains"
        );
        assert_eq!(state.ssh_key_cursor, 0);
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

    // r[verify installer.tui.ssh-keys+5]
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

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn filter_ssh_keys_ensures_one_empty() {
        let mut state = make_state();
        state.ssh_keys = vec!["invalid".into(), String::new()];
        state.filter_ssh_keys();
        assert_eq!(state.ssh_keys, vec![String::new()]);
    }

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn ensure_trailing_blank_appends_when_last_nonempty() {
        let mut state = make_state();
        state.ssh_keys = vec!["ssh-ed25519 AAAA key1".into()];
        state.ensure_trailing_blank();
        assert_eq!(state.ssh_keys, vec!["ssh-ed25519 AAAA key1", ""]);
    }

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn ensure_trailing_blank_noop_when_last_empty() {
        let mut state = make_state();
        state.ssh_keys = vec!["ssh-ed25519 AAAA key1".into(), String::new()];
        state.ensure_trailing_blank();
        assert_eq!(state.ssh_keys, vec!["ssh-ed25519 AAAA key1", ""]);
    }

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn ensure_trailing_blank_on_empty_vec() {
        let mut state = make_state();
        state.ssh_keys = vec![];
        state.ensure_trailing_blank();
        assert_eq!(state.ssh_keys, vec![String::new()]);
    }

    // r[verify installer.tui.ssh-keys+5]
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
        let cfg = InstallConfig {
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA key1".into(), "ssh-rsa BBBB key2".into()],
            ..Default::default()
        };
        let state = AppState::new(
            devices,
            DiskEncryption::Tpm,
            true,
            &cfg,
            None,
            None,
            String::new(),
            test_timezones(),
            false,
        );
        assert_eq!(
            state.ssh_keys,
            vec!["ssh-ed25519 AAAA key1", "ssh-rsa BBBB key2", ""]
        );
    }

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn ssh_key_summary_with_comment() {
        let summary =
            AppState::ssh_key_summary("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAbcdef me@host");
        assert!(summary.starts_with("ssh-ed25519 "));
        assert!(summary.contains("me@host"));
    }

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn ssh_key_summary_empty() {
        assert_eq!(AppState::ssh_key_summary(""), "(empty)");
    }

    // r[verify installer.tui.network-check+6]
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
    fn install_config_includes_non_utc_timezone() {
        let mut state = make_state();
        state.hostname_input = "host".into();
        state.timezone_selected = "Europe/London".into();
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.timezone.as_deref(), Some("Europe/London"));
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn install_config_utc_timezone_is_none() {
        let mut state = make_state();
        state.hostname_input = "host".into();
        state.timezone_selected = "UTC".into();
        let cfg = state.install_config_fields().unwrap();
        assert!(cfg.timezone.is_none());
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn install_config_timezone_alone_triggers_some() {
        let mut state = make_state();
        state.timezone_selected = "Asia/Tokyo".into();
        let cfg = state.install_config_fields();
        assert!(cfg.is_some());
        assert_eq!(cfg.unwrap().timezone.as_deref(), Some("Asia/Tokyo"));
    }

    // r[verify installer.tui.progress+4]
    #[test]
    fn confirmation_advances_to_installing() {
        let mut state = make_state();
        state.screen = Screen::Confirmation;
        state.confirm_input = "yes".into();
        state.advance();
        assert_eq!(state.screen, Screen::Installing);
    }

    // r[verify installer.tui.progress+4]
    #[test]
    fn installing_advance_is_noop() {
        let mut state = make_state();
        state.screen = Screen::Installing;
        state.advance();
        assert_eq!(state.screen, Screen::Installing);
    }

    // r[verify installer.tui.progress+4]
    #[test]
    fn installing_no_go_back() {
        let mut state = make_state();
        state.screen = Screen::Installing;
        state.go_back();
        assert_eq!(state.screen, Screen::Installing);
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn install_config_fields_static_network() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::StaticIp;
        state.target_static_config = StaticNetConfig {
            interface: "enp0s3".into(),
            ip_cidr: "192.168.1.10/24".into(),
            gateway: "192.168.1.1".into(),
            dns: "8.8.8.8, 1.1.1.1".into(),
            search_domain: "example.com".into(),
        };
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.network_mode, Some(NetworkMode::StaticIp));
        assert_eq!(cfg.network_interface.as_deref(), Some("enp0s3"));
        assert_eq!(cfg.network_ip.as_deref(), Some("192.168.1.10/24"));
        assert_eq!(cfg.network_gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(cfg.network_dns.as_deref(), Some("8.8.8.8, 1.1.1.1"));
        assert_eq!(cfg.network_domain.as_deref(), Some("example.com"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn install_config_fields_static_no_interface() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::StaticIp;
        state.target_static_config = StaticNetConfig {
            interface: String::new(),
            ip_cidr: "10.0.0.5/16".into(),
            gateway: "10.0.0.1".into(),
            dns: String::new(),
            search_domain: String::new(),
        };
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.network_mode, Some(NetworkMode::StaticIp));
        assert!(cfg.network_interface.is_none());
        assert_eq!(cfg.network_ip.as_deref(), Some("10.0.0.5/16"));
        assert_eq!(cfg.network_gateway.as_deref(), Some("10.0.0.1"));
        assert!(cfg.network_dns.is_none());
        assert!(cfg.network_domain.is_none());
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn install_config_fields_ipv6_slaac() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::Ipv6Slaac;
        state.target_static_config = StaticNetConfig {
            interface: "eth0".into(),
            ..Default::default()
        };
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.network_mode, Some(NetworkMode::Ipv6Slaac));
        assert_eq!(cfg.network_interface.as_deref(), Some("eth0"));
        assert!(cfg.network_ip.is_none());
        assert!(cfg.network_gateway.is_none());
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn install_config_fields_offline() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::Offline;
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.network_mode, Some(NetworkMode::Offline));
        assert!(cfg.network_interface.is_none());
        assert!(cfg.network_ip.is_none());
        assert!(cfg.network_gateway.is_none());
        assert!(cfg.network_dns.is_none());
        assert!(cfg.network_domain.is_none());
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn install_config_fields_copy_current_resolves_to_iso_mode() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::CopyCurrent;
        state.iso_network_mode = NetworkMode::StaticIp;
        state.iso_static_config = StaticNetConfig {
            interface: "enp0s3".into(),
            ip_cidr: "172.16.0.10/24".into(),
            gateway: "172.16.0.1".into(),
            dns: "1.1.1.1".into(),
            search_domain: "local.lan".into(),
        };
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.network_mode, Some(NetworkMode::StaticIp));
        assert_eq!(cfg.network_interface.as_deref(), Some("enp0s3"));
        assert_eq!(cfg.network_ip.as_deref(), Some("172.16.0.10/24"));
        assert_eq!(cfg.network_gateway.as_deref(), Some("172.16.0.1"));
        assert_eq!(cfg.network_dns.as_deref(), Some("1.1.1.1"));
        assert_eq!(cfg.network_domain.as_deref(), Some("local.lan"));
    }

    // r[verify installer.finalise.network+4]
    #[test]
    fn install_config_fields_copy_current_dhcp() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::CopyCurrent;
        state.iso_network_mode = NetworkMode::Dhcp;
        let cfg = state.install_config_fields().unwrap();
        assert_eq!(cfg.network_mode, Some(NetworkMode::Dhcp));
        assert!(cfg.network_interface.is_none());
        assert!(cfg.network_ip.is_none());
    }

    // --- Tab navigation tests ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn tab_forward_iso_mode_to_target_mode_when_dhcp() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.net_config_pane = NetConfigPane::Iso;
        state.net_config_focus = NetConfigFocus::IsoMode;
        state.iso_network_mode = NetworkMode::Dhcp;

        // ISO pane has only IsoMode when DHCP, so Tab switches to target
        state.tab_focus_forward();
        assert_eq!(state.net_config_pane, NetConfigPane::Target);
        assert_eq!(state.net_config_focus, NetConfigFocus::TargetMode);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn tab_forward_through_iso_static_fields() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.net_config_pane = NetConfigPane::Iso;
        state.net_config_focus = NetConfigFocus::IsoMode;
        state.iso_network_mode = NetworkMode::StaticIp;

        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoInterface);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoIp);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoGateway);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoDns);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoDomain);

        // One more Tab wraps to target pane
        state.tab_focus_forward();
        assert_eq!(state.net_config_pane, NetConfigPane::Target);
        assert_eq!(state.net_config_focus, NetConfigFocus::TargetMode);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn shift_tab_backward_from_target_to_iso() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.net_config_pane = NetConfigPane::Target;
        state.net_config_focus = NetConfigFocus::TargetMode;
        state.iso_network_mode = NetworkMode::Dhcp;

        // Shift+Tab from first target field goes to last ISO field (IsoMode for DHCP)
        state.tab_focus_backward();
        assert_eq!(state.net_config_pane, NetConfigPane::Iso);
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoMode);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn shift_tab_backward_from_target_to_iso_static_last() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.net_config_pane = NetConfigPane::Target;
        state.net_config_focus = NetConfigFocus::TargetMode;
        state.iso_network_mode = NetworkMode::StaticIp;

        state.tab_focus_backward();
        assert_eq!(state.net_config_pane, NetConfigPane::Iso);
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoDomain);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn tab_forward_target_static_fields() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.net_config_pane = NetConfigPane::Target;
        state.net_config_focus = NetConfigFocus::TargetMode;
        state.target_network_mode = TargetNetworkMode::StaticIp;

        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::TargetInterface);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::TargetIp);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::TargetGateway);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::TargetDns);
        state.tab_focus_forward();
        assert_eq!(state.net_config_focus, NetConfigFocus::TargetDomain);

        // Wraps back to ISO
        state.tab_focus_forward();
        assert_eq!(state.net_config_pane, NetConfigPane::Iso);
    }

    // --- Mode cycling tests ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_iso_mode_forward() {
        let mut state = make_state();
        state.iso_network_mode = NetworkMode::Dhcp;
        state.cycle_iso_mode_forward();
        assert_eq!(state.iso_network_mode, NetworkMode::StaticIp);
        state.cycle_iso_mode_forward();
        assert_eq!(state.iso_network_mode, NetworkMode::Ipv6Slaac);
        state.cycle_iso_mode_forward();
        assert_eq!(state.iso_network_mode, NetworkMode::Offline);
        state.cycle_iso_mode_forward();
        assert_eq!(state.iso_network_mode, NetworkMode::Dhcp);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_iso_mode_backward() {
        let mut state = make_state();
        state.iso_network_mode = NetworkMode::Dhcp;
        state.cycle_iso_mode_backward();
        assert_eq!(state.iso_network_mode, NetworkMode::Offline);
        state.cycle_iso_mode_backward();
        assert_eq!(state.iso_network_mode, NetworkMode::Ipv6Slaac);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_target_mode_with_copy_current() {
        let mut state = make_state();
        state.config_has_network_mode = false;
        state.target_network_mode = TargetNetworkMode::CopyCurrent;

        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::Dhcp);
        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::StaticIp);
        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::Ipv6Slaac);
        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::Offline);
        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::CopyCurrent);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_target_mode_without_copy_current() {
        let mut state = make_state();
        state.config_has_network_mode = true;
        state.target_network_mode = TargetNetworkMode::Dhcp;

        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::StaticIp);
        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::Ipv6Slaac);
        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::Offline);
        state.cycle_target_mode_forward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::Dhcp);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_target_mode_marks_touched() {
        let mut state = make_state();
        assert!(!state.target_pane_touched);
        state.cycle_target_mode_forward();
        assert!(state.target_pane_touched);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_target_mode_backward_with_copy_current() {
        let mut state = make_state();
        state.config_has_network_mode = false;
        state.target_network_mode = TargetNetworkMode::CopyCurrent;
        state.cycle_target_mode_backward();
        assert_eq!(state.target_network_mode, TargetNetworkMode::Offline);
    }

    // --- ISO mode changes auto-switch target ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn iso_offline_auto_switches_target_from_copy_current_to_dhcp() {
        let mut state = make_state();
        state.config_has_network_mode = false;
        state.target_network_mode = TargetNetworkMode::CopyCurrent;
        state.target_pane_touched = false;
        state.iso_network_mode = NetworkMode::Ipv6Slaac;

        // Cycle to Offline
        state.cycle_iso_mode_forward();
        assert_eq!(state.iso_network_mode, NetworkMode::Offline);
        assert_eq!(
            state.target_network_mode,
            TargetNetworkMode::Dhcp,
            "target should auto-switch to DHCP when ISO goes offline and target is CopyCurrent"
        );
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn iso_offline_does_not_switch_touched_target() {
        let mut state = make_state();
        state.config_has_network_mode = false;
        state.target_network_mode = TargetNetworkMode::CopyCurrent;
        state.target_pane_touched = true;
        state.iso_network_mode = NetworkMode::Ipv6Slaac;

        state.cycle_iso_mode_forward();
        assert_eq!(state.iso_network_mode, NetworkMode::Offline);
        assert_eq!(
            state.target_network_mode,
            TargetNetworkMode::CopyCurrent,
            "touched target should not be auto-switched"
        );
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn iso_offline_does_not_switch_concrete_target() {
        let mut state = make_state();
        state.target_network_mode = TargetNetworkMode::StaticIp;
        state.target_pane_touched = false;
        state.iso_network_mode = NetworkMode::Ipv6Slaac;

        state.cycle_iso_mode_forward();
        assert_eq!(state.iso_network_mode, NetworkMode::Offline);
        assert_eq!(
            state.target_network_mode,
            TargetNetworkMode::StaticIp,
            "concrete target mode should not be auto-switched"
        );
    }

    // --- CIDR auto-suffix tests ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cidr_auto_suffix_appends_slash_24() {
        let mut cfg = StaticNetConfig {
            ip_cidr: "192.168.1.10".into(),
            ..Default::default()
        };
        cfg.auto_suffix_cidr();
        assert_eq!(cfg.ip_cidr, "192.168.1.10/24");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cidr_auto_suffix_preserves_existing() {
        let mut cfg = StaticNetConfig {
            ip_cidr: "10.0.0.5/16".into(),
            ..Default::default()
        };
        cfg.auto_suffix_cidr();
        assert_eq!(cfg.ip_cidr, "10.0.0.5/16");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cidr_auto_suffix_empty_is_noop() {
        let mut cfg = StaticNetConfig::default();
        cfg.auto_suffix_cidr();
        assert_eq!(cfg.ip_cidr, "");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cidr_auto_suffix_trims_whitespace() {
        let mut cfg = StaticNetConfig {
            ip_cidr: "  192.168.1.10  ".into(),
            ..Default::default()
        };
        cfg.auto_suffix_cidr();
        assert_eq!(cfg.ip_cidr, "192.168.1.10/24");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn tab_away_from_ip_field_applies_cidr_suffix() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.net_config_pane = NetConfigPane::Iso;
        state.net_config_focus = NetConfigFocus::IsoIp;
        state.iso_network_mode = NetworkMode::StaticIp;
        state.iso_static_config.ip_cidr = "10.0.0.1".into();

        state.tab_focus_forward();
        assert_eq!(state.iso_static_config.ip_cidr, "10.0.0.1/24");
        assert_eq!(state.net_config_focus, NetConfigFocus::IsoGateway);
    }

    // --- Offline warning dialog tests ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn offline_target_shows_warning_on_advance() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.target_network_mode = TargetNetworkMode::Offline;
        state.offline_target_warning = false;

        // First advance should trigger the warning, not proceed
        let result = state.try_advance_from_network_config();
        assert!(!result);
        assert!(state.offline_target_warning);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn offline_warning_second_advance_proceeds() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.target_network_mode = TargetNetworkMode::Offline;
        state.offline_target_warning = true;

        // When warning is already showing, advance proceeds
        let result = state.try_advance_from_network_config();
        assert!(result);
        assert!(!state.offline_target_warning);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn non_offline_target_advances_without_warning() {
        let mut state = make_state();
        state.screen = Screen::NetworkConfig;
        state.target_network_mode = TargetNetworkMode::Dhcp;

        let result = state.try_advance_from_network_config();
        assert!(result);
        assert!(!state.offline_target_warning);
    }

    // --- Interface cycling tests ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_interface_forward() {
        use crate::net::NetInterface;

        let mut state = make_state();
        state.detected_interfaces = vec![
            NetInterface {
                name: "enp0s3".into(),
                mac: "aa:bb:cc:dd:ee:01".into(),
                state: "UP".into(),
            },
            NetInterface {
                name: "enp0s8".into(),
                mac: "aa:bb:cc:dd:ee:02".into(),
                state: "UP".into(),
            },
        ];
        state.net_config_focus = NetConfigFocus::IsoInterface;
        state.iso_static_config.interface = "enp0s3".into();

        state.cycle_interface(false);
        assert_eq!(state.iso_static_config.interface, "enp0s8");

        state.cycle_interface(false);
        assert_eq!(state.iso_static_config.interface, "enp0s3");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_interface_backward() {
        use crate::net::NetInterface;

        let mut state = make_state();
        state.detected_interfaces = vec![
            NetInterface {
                name: "enp0s3".into(),
                mac: "aa:bb:cc:dd:ee:01".into(),
                state: "UP".into(),
            },
            NetInterface {
                name: "enp0s8".into(),
                mac: "aa:bb:cc:dd:ee:02".into(),
                state: "UP".into(),
            },
        ];
        state.net_config_focus = NetConfigFocus::IsoInterface;
        state.iso_static_config.interface = "enp0s3".into();

        state.cycle_interface(true);
        assert_eq!(state.iso_static_config.interface, "enp0s8");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn cycle_interface_empty_list_is_noop() {
        let mut state = make_state();
        state.detected_interfaces = vec![];
        state.net_config_focus = NetConfigFocus::IsoInterface;
        state.iso_static_config.interface = "something".into();

        state.cycle_interface(false);
        assert_eq!(state.iso_static_config.interface, "something");
    }

    // --- Text input tests ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_push_char_iso_ip() {
        let mut state = make_state();
        state.net_config_focus = NetConfigFocus::IsoIp;
        state.iso_static_config.ip_cidr = "192.168".into();

        state.net_config_push_char('.');
        state.net_config_push_char('1');
        assert_eq!(state.iso_static_config.ip_cidr, "192.168.1");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_backspace_iso_gateway() {
        let mut state = make_state();
        state.net_config_focus = NetConfigFocus::IsoGateway;
        state.iso_static_config.gateway = "10.0.0.1".into();

        state.net_config_backspace();
        assert_eq!(state.iso_static_config.gateway, "10.0.0.");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_push_char_target_dns() {
        let mut state = make_state();
        state.net_config_focus = NetConfigFocus::TargetDns;
        state.target_static_config.dns = "8.8.8".into();

        state.net_config_push_char('.');
        state.net_config_push_char('8');
        assert_eq!(state.target_static_config.dns, "8.8.8.8");
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_push_char_on_mode_selector_is_noop() {
        let mut state = make_state();
        state.net_config_focus = NetConfigFocus::IsoMode;
        state.iso_static_config.ip_cidr = "before".into();

        state.net_config_push_char('x');
        assert_eq!(
            state.iso_static_config.ip_cidr, "before",
            "typing on mode selector should not modify any field"
        );
    }

    // --- Focus helper tests ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_focus_pane() {
        assert_eq!(NetConfigFocus::IsoMode.pane(), NetConfigPane::Iso);
        assert_eq!(NetConfigFocus::IsoInterface.pane(), NetConfigPane::Iso);
        assert_eq!(NetConfigFocus::IsoDomain.pane(), NetConfigPane::Iso);
        assert_eq!(NetConfigFocus::TargetMode.pane(), NetConfigPane::Target);
        assert_eq!(NetConfigFocus::TargetIp.pane(), NetConfigPane::Target);
        assert_eq!(NetConfigFocus::TargetDomain.pane(), NetConfigPane::Target);
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_focus_is_text_input() {
        assert!(!NetConfigFocus::IsoMode.is_text_input());
        assert!(!NetConfigFocus::IsoInterface.is_text_input());
        assert!(NetConfigFocus::IsoIp.is_text_input());
        assert!(NetConfigFocus::IsoGateway.is_text_input());
        assert!(NetConfigFocus::IsoDns.is_text_input());
        assert!(NetConfigFocus::IsoDomain.is_text_input());
        assert!(!NetConfigFocus::TargetMode.is_text_input());
        assert!(!NetConfigFocus::TargetInterface.is_text_input());
        assert!(NetConfigFocus::TargetIp.is_text_input());
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_focus_is_mode_selector() {
        assert!(NetConfigFocus::IsoMode.is_mode_selector());
        assert!(NetConfigFocus::TargetMode.is_mode_selector());
        assert!(!NetConfigFocus::IsoIp.is_mode_selector());
        assert!(!NetConfigFocus::TargetInterface.is_mode_selector());
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn net_config_focus_is_interface_dropdown() {
        assert!(NetConfigFocus::IsoInterface.is_interface_dropdown());
        assert!(NetConfigFocus::TargetInterface.is_interface_dropdown());
        assert!(!NetConfigFocus::IsoMode.is_interface_dropdown());
        assert!(!NetConfigFocus::IsoIp.is_interface_dropdown());
    }

    // --- Debounce schedule test ---

    // r[verify installer.tui.network-config+13]
    #[test]
    fn schedule_iso_apply_sets_debounce() {
        let mut state = make_state();
        assert!(state.net_apply_debounce.is_none());
        state.schedule_iso_apply();
        assert!(state.net_apply_debounce.is_some());
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn poll_iso_apply_debounce_not_ready_immediately() {
        let mut state = make_state();
        state.schedule_iso_apply();
        // Debounce is 500ms, should not fire immediately
        assert!(!state.poll_iso_apply_debounce());
        assert!(state.net_apply_debounce.is_some());
    }

    // r[verify installer.tui.network-config+13]
    #[test]
    fn poll_iso_apply_debounce_fires_after_elapsed() {
        use std::time::{Duration, Instant};

        let mut state = make_state();
        // Set debounce to a time far enough in the past
        state.net_apply_debounce = Some(Instant::now() - Duration::from_secs(1));
        assert!(state.poll_iso_apply_debounce());
        assert!(
            state.net_apply_debounce.is_none(),
            "debounce should be cleared after firing"
        );
    }
}
