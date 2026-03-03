// r[impl installer.tui.rust]
// r[impl installer.tui.disk-detection]
// r[impl installer.tui.variant-selection]
// r[impl installer.tui.tpm-toggle]
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
    pub firstboot: Option<FirstbootConfig>,
    pub boot_device: Option<PathBuf>,
    pub write_progress: Option<ProgressSnapshot>,
    pub confirm_input: String,
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
    ) -> Self {
        Self {
            screen: Screen::Welcome,
            selected_disk_index: default_disk_index.unwrap_or(0),
            devices,
            variant,
            disable_tpm,
            firstboot,
            boot_device,
            write_progress: None,
            confirm_input: String::new(),
        }
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
            Screen::VariantSelection => Screen::Confirmation,
            Screen::TpmToggle => Screen::Confirmation,
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
            Screen::Confirmation => {
                if self.variant == Variant::Metal {
                    Screen::TpmToggle
                } else {
                    Screen::VariantSelection
                }
            }
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
        AppState::new(devices, Variant::Metal, false, None, None, None)
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
        assert_eq!(state.screen, Screen::Confirmation);
    }

    // r[verify installer.tui.tpm-toggle]
    #[test]
    fn go_back_through_metal_flow() {
        let mut state = make_state();
        state.variant = Variant::Metal;
        state.screen = Screen::Confirmation;

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
}
