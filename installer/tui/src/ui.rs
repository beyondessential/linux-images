// r[impl installer.tui.rust]
// r[impl installer.tui.disk-detection]
// r[impl installer.tui.variant-selection]
// r[impl installer.tui.tpm-toggle]
// r[impl installer.tui.hostname]
// r[impl installer.tui.tailscale]
// r[impl installer.tui.ssh-keys]
// r[impl installer.tui.confirmation]
// r[impl installer.tui.progress]

use std::path::PathBuf;
use std::time::Duration;

use crate::config::{FirstbootConfig, Variant};
use crate::disk::BlockDevice;
use crate::writer::WriteProgress;

mod render;
mod run;

pub use run::run_tui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    DiskSelection,
    VariantSelection,
    TpmToggle,
    Hostname,
    Tailscale,
    SshKeys,
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
    pub tailscale_input: String,
    pub ssh_keys_input: String,
}

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
    pub fn new(
        devices: Vec<BlockDevice>,
        variant: Variant,
        disable_tpm: bool,
        firstboot: Option<FirstbootConfig>,
        boot_device: Option<PathBuf>,
        default_disk_index: Option<usize>,
        build_info: String,
    ) -> Self {
        let (hostname_input, tailscale_input, ssh_keys_input) = match firstboot {
            Some(ref fb) => (
                fb.hostname.clone().unwrap_or_default(),
                fb.tailscale_authkey.clone().unwrap_or_default(),
                fb.ssh_authorized_keys.join("\n"),
            ),
            None => (String::new(), String::new(), String::new()),
        };

        Self {
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
            tailscale_input,
            ssh_keys_input,
        }
    }

    /// Build a `FirstbootConfig` from the current interactive input fields.
    /// Returns `None` if all fields are empty (nothing to configure).
    pub fn firstboot_config(&self) -> Option<FirstbootConfig> {
        let hostname = if self.hostname_input.trim().is_empty() {
            None
        } else {
            Some(self.hostname_input.trim().to_string())
        };

        let tailscale_authkey = if self.tailscale_input.trim().is_empty() {
            None
        } else {
            Some(self.tailscale_input.trim().to_string())
        };

        let ssh_authorized_keys: Vec<String> = self
            .ssh_keys_input
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        if hostname.is_none() && tailscale_authkey.is_none() && ssh_authorized_keys.is_empty() {
            return None;
        }

        Some(FirstbootConfig {
            hostname,
            tailscale_authkey,
            ssh_authorized_keys,
        })
    }

    pub fn selected_disk(&self) -> Option<&BlockDevice> {
        self.devices.get(self.selected_disk_index)
    }

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

    pub fn toggle_variant(&mut self) {
        self.variant = match self.variant {
            Variant::Metal => Variant::Cloud,
            Variant::Cloud => Variant::Metal,
        };
    }

    pub fn advance(&mut self) {
        self.screen = match &self.screen {
            Screen::Welcome => Screen::DiskSelection,
            Screen::DiskSelection => Screen::VariantSelection,
            Screen::VariantSelection if self.variant == Variant::Metal => Screen::TpmToggle,
            Screen::VariantSelection => Screen::Hostname,
            Screen::TpmToggle => Screen::Hostname,
            Screen::Hostname => Screen::Tailscale,
            Screen::Tailscale => Screen::SshKeys,
            Screen::SshKeys => Screen::Confirmation,
            Screen::Confirmation => Screen::Writing,
            Screen::Writing => Screen::FirstbootApply,
            Screen::FirstbootApply => Screen::Done,
            Screen::Done | Screen::Error(_) => return,
        };
    }

    pub fn go_back(&mut self) {
        self.screen = match &self.screen {
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
            Screen::Tailscale => Screen::Hostname,
            Screen::SshKeys => Screen::Tailscale,
            Screen::Confirmation => Screen::SshKeys,
            _ => return,
        };
    }

    pub fn confirmation_text(&self) -> &str {
        "yes"
    }

    pub fn is_confirmed(&self) -> bool {
        self.confirm_input
            .trim()
            .eq_ignore_ascii_case(self.confirmation_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        )
    }

    // r[verify installer.tui.welcome]
    #[test]
    fn initial_state() {
        let state = make_state();
        assert_eq!(state.screen, Screen::Welcome);
        assert_eq!(state.selected_disk_index, 0);
        assert_eq!(state.variant, Variant::Metal);
        assert!(!state.disable_tpm);
    }

    // r[verify installer.tui.welcome]
    #[test]
    fn welcome_advances_to_disk_selection() {
        let mut state = make_state();
        assert_eq!(state.screen, Screen::Welcome);
        state.advance();
        assert_eq!(state.screen, Screen::DiskSelection);
    }

    // r[verify installer.tui.welcome]
    #[test]
    fn disk_selection_goes_back_to_welcome() {
        let mut state = make_state();
        state.screen = Screen::DiskSelection;
        state.go_back();
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.disk-detection]
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
        assert_eq!(state.screen, Screen::Tailscale);
        state.advance();
        assert_eq!(state.screen, Screen::SshKeys);
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.variant-selection]
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
        assert_eq!(state.screen, Screen::Tailscale);
        state.advance();
        assert_eq!(state.screen, Screen::SshKeys);
        state.advance();
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.tpm-toggle]
    #[test]
    fn go_back_through_metal_flow() {
        let mut state = make_state();
        state.variant = Variant::Metal;
        state.screen = Screen::Confirmation;

        state.go_back();
        assert_eq!(state.screen, Screen::SshKeys);
        state.go_back();
        assert_eq!(state.screen, Screen::Tailscale);
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
    #[test]
    fn go_back_cloud_skips_tpm() {
        let mut state = make_state();
        state.variant = Variant::Cloud;
        state.screen = Screen::Confirmation;

        state.go_back();
        assert_eq!(state.screen, Screen::SshKeys);
        state.go_back();
        assert_eq!(state.screen, Screen::Tailscale);
        state.go_back();
        assert_eq!(state.screen, Screen::Hostname);
        state.go_back();
        assert_eq!(state.screen, Screen::VariantSelection);
    }

    // r[verify installer.tui.confirmation]
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

    // r[verify installer.tui.confirmation]
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

    // r[verify installer.tui.hostname]
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
            tailscale_authkey: None,
            ssh_authorized_keys: vec![],
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
        );
        assert_eq!(state.hostname_input, "myhost");
        assert_eq!(state.tailscale_input, "");
        assert_eq!(state.ssh_keys_input, "");
    }

    // r[verify installer.tui.tailscale]
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
            hostname: None,
            tailscale_authkey: Some("tskey-auth-xxx".into()),
            ssh_authorized_keys: vec![],
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
        );
        assert_eq!(state.tailscale_input, "tskey-auth-xxx");
    }

    // r[verify installer.tui.ssh-keys]
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
            hostname: None,
            tailscale_authkey: None,
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA key1".into(), "ssh-rsa BBBB key2".into()],
        };
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
        );
        assert_eq!(
            state.ssh_keys_input,
            "ssh-ed25519 AAAA key1\nssh-rsa BBBB key2"
        );
    }

    // r[verify installer.tui.hostname]
    #[test]
    fn firstboot_config_from_inputs() {
        let mut state = make_state();
        assert!(state.firstboot_config().is_none());

        state.hostname_input = "server-01".into();
        let fb = state.firstboot_config().unwrap();
        assert_eq!(fb.hostname.as_deref(), Some("server-01"));
        assert!(fb.tailscale_authkey.is_none());
        assert!(fb.ssh_authorized_keys.is_empty());
    }

    // r[verify installer.tui.tailscale]
    // r[verify installer.tui.ssh-keys]
    #[test]
    fn firstboot_config_all_fields() {
        let mut state = make_state();
        state.hostname_input = "host".into();
        state.tailscale_input = "tskey-auth-123".into();
        state.ssh_keys_input = "ssh-ed25519 AAAA\nssh-rsa BBBB\n".into();

        let fb = state.firstboot_config().unwrap();
        assert_eq!(fb.hostname.as_deref(), Some("host"));
        assert_eq!(fb.tailscale_authkey.as_deref(), Some("tskey-auth-123"));
        assert_eq!(fb.ssh_authorized_keys.len(), 2);
    }

    // r[verify installer.tui.hostname]
    #[test]
    fn firstboot_config_empty_strings_are_none() {
        let mut state = make_state();
        state.hostname_input = "   ".into();
        state.tailscale_input = "  ".into();
        state.ssh_keys_input = "\n\n".into();
        assert!(state.firstboot_config().is_none());
    }
}
