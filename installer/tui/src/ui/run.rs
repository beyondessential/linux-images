use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::Variant;
use crate::firstboot;
use crate::writer;

use super::render::render;
use super::{AppState, ProgressSnapshot, Screen};

enum WorkerMessage {
    Progress(ProgressSnapshot),
    WriteDone,
    WriteError(String),
    FirstbootDone,
    FirstbootError(String),
}

/// Result of processing a single key event against the current TUI state.
enum KeyAction {
    Continue,
    Quit,
    Reboot,
    StartWrite,
}

/// Process a single key event, updating state and returning what the event
/// loop should do next.
fn handle_key(key: KeyEvent, state: &mut AppState) -> KeyAction {
    if key.kind != KeyEventKind::Press {
        return KeyAction::Continue;
    }

    match &state.screen {
        Screen::Welcome => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Char('n') => state.open_network_check(),
            KeyCode::Enter => state.advance(),
            _ => {}
        },

        // r[impl installer.tui.network-check+2]
        Screen::NetworkCheck => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Esc => state.go_back(),
            KeyCode::Char('r') => state.start_net_checks(),
            KeyCode::Tab => state.toggle_net_pane(),
            KeyCode::Up | KeyCode::Char('k') => state.scroll_net_up(),
            KeyCode::Down | KeyCode::Char('j') => state.scroll_net_down(),
            _ => {}
        },

        // r[impl installer.tui.network-check+2]
        Screen::NetworkResults => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Esc => state.go_back(),
            KeyCode::Char('r') => state.start_net_checks(),
            KeyCode::Tab => state.toggle_net_pane(),
            KeyCode::Up | KeyCode::Char('k') => state.scroll_net_up(),
            KeyCode::Down | KeyCode::Char('j') => state.scroll_net_down(),
            KeyCode::Enter => state.advance(),
            _ => {}
        },

        Screen::DiskSelection => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Esc => state.go_back(),
            KeyCode::Up | KeyCode::Char('k') => state.select_prev_disk(),
            KeyCode::Down | KeyCode::Char('j') => state.select_next_disk(),
            KeyCode::Enter => {
                if state.selected_disk().is_some() {
                    state.advance();
                }
            }
            _ => {}
        },

        Screen::VariantSelection => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Esc => state.go_back(),
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
                state.toggle_variant();
            }
            KeyCode::Enter => state.advance(),
            _ => {}
        },

        Screen::TpmToggle => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Esc => state.go_back(),
            KeyCode::Char(' ') => state.disable_tpm = !state.disable_tpm,
            KeyCode::Enter => state.advance(),
            _ => {}
        },

        // r[impl installer.tui.hostname+2]
        Screen::Hostname => match key.code {
            KeyCode::Esc => state.go_back(),
            KeyCode::Enter => {
                if state.hostname_required() && state.hostname_input.trim().is_empty() {
                    // Block advance — render will show the hint
                } else {
                    state.advance();
                }
            }
            KeyCode::Tab if state.variant == Variant::Metal => {
                state.hostname_from_dhcp = !state.hostname_from_dhcp;
            }
            KeyCode::Char(' ') if state.variant == Variant::Metal => {
                state.hostname_from_dhcp = !state.hostname_from_dhcp;
            }
            KeyCode::Backspace if !state.hostname_from_dhcp => {
                state.hostname_input.pop();
            }
            KeyCode::Char(c) if !state.hostname_from_dhcp => {
                state.hostname_input.push(c);
            }
            _ => {}
        },

        // r[impl installer.tui.tailscale]
        Screen::Tailscale => match key.code {
            KeyCode::Esc => state.go_back(),
            KeyCode::Enter => state.advance(),
            KeyCode::Backspace => {
                state.tailscale_input.pop();
            }
            KeyCode::Char(c) => {
                state.tailscale_input.push(c);
            }
            _ => {}
        },

        // r[impl installer.tui.ssh-keys]
        // r[impl installer.tui.ssh-keys.github]
        Screen::SshKeys => match key.code {
            KeyCode::Esc => {
                if state.ssh_github_focus {
                    state.ssh_github_focus = false;
                } else {
                    state.go_back();
                }
            }
            KeyCode::Tab => {
                if state.ssh_github_focus {
                    state.ssh_github_focus = false;
                    state.advance();
                } else {
                    state.ssh_github_focus = true;
                }
            }
            KeyCode::Enter => {
                if state.ssh_github_focus {
                    if !state.ssh_github_fetching {
                        state.start_github_key_fetch();
                    }
                } else {
                    state.ssh_keys_input.push('\n');
                }
            }
            KeyCode::Backspace => {
                if state.ssh_github_focus {
                    state.ssh_github_input.pop();
                    state.ssh_github_error = None;
                } else {
                    state.ssh_keys_input.pop();
                }
            }
            KeyCode::Char(c) => {
                if state.ssh_github_focus {
                    state.ssh_github_input.push(c);
                    state.ssh_github_error = None;
                } else {
                    state.ssh_keys_input.push(c);
                }
            }
            _ => {}
        },

        // r[impl installer.tui.password]
        Screen::Password => match key.code {
            KeyCode::Esc => {
                if state.password_confirming {
                    state.password_confirming = false;
                    state.password_mismatch = false;
                } else {
                    state.go_back();
                }
            }
            KeyCode::Enter | KeyCode::Tab => {
                if !state.password_confirming {
                    state.password_confirming = true;
                    state.password_mismatch = false;
                } else if state.password_input.is_empty() && state.password_confirm_input.is_empty()
                {
                    state.password_confirming = false;
                    state.password_mismatch = false;
                    state.advance();
                } else if state.password_matches() {
                    state.password_mismatch = false;
                    state.advance();
                } else {
                    state.password_mismatch = true;
                }
            }
            KeyCode::Backspace => {
                if state.password_confirming {
                    state.password_confirm_input.pop();
                } else {
                    state.password_input.pop();
                }
            }
            KeyCode::Char(c) => {
                if state.password_confirming {
                    state.password_confirm_input.push(c);
                } else {
                    state.password_input.push(c);
                }
            }
            _ => {}
        },

        // r[impl installer.tui.timezone]
        Screen::Timezone => match key.code {
            KeyCode::Esc => state.go_back(),
            KeyCode::Enter => {
                state.timezone_selected = state.timezone_highlighted().to_string();
                state.advance();
            }
            KeyCode::Up => {
                if state.timezone_cursor > 0 {
                    state.timezone_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if state.timezone_cursor + 1 < state.timezone_filtered.len() {
                    state.timezone_cursor += 1;
                }
            }
            KeyCode::Backspace => {
                state.timezone_search.pop();
                state.update_timezone_filter();
            }
            KeyCode::Char(c) => {
                state.timezone_search.push(c);
                state.update_timezone_filter();
            }
            _ => {}
        },

        Screen::Confirmation => match key.code {
            KeyCode::Char('q') if state.confirm_input.is_empty() => return KeyAction::Quit,
            KeyCode::Esc => {
                state.confirm_input.clear();
                state.go_back();
            }
            KeyCode::Backspace => {
                state.confirm_input.pop();
            }
            KeyCode::Enter => {
                if state.is_confirmed() {
                    state.advance();
                    return KeyAction::StartWrite;
                }
            }
            KeyCode::Char(c) => {
                state.confirm_input.push(c);
            }
            _ => {}
        },

        Screen::Writing | Screen::FirstbootApply => {}

        Screen::Done => {
            return KeyAction::Reboot;
        }

        Screen::Error(_) => {
            return KeyAction::Quit;
        }
    }

    KeyAction::Continue
}

pub fn run_tui(mut state: AppState, image_path: &Path, no_reboot: bool) -> Result<()> {
    terminal::enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut state, image_path, no_reboot);

    terminal::disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show).ok();

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    image_path: &Path,
    no_reboot: bool,
) -> Result<()> {
    let (worker_tx, worker_rx) = mpsc::channel::<WorkerMessage>();

    loop {
        // Poll async results for network screens and GitHub key fetch
        state.poll_net_checks();
        state.poll_tailscale_netcheck();
        state.poll_github_keys();

        terminal.draw(|f| render(f, state))?;

        while let Ok(msg) = worker_rx.try_recv() {
            match msg {
                WorkerMessage::Progress(snap) => {
                    state.write_progress = Some(snap);
                }
                WorkerMessage::WriteDone => {
                    state.screen = Screen::FirstbootApply;
                    terminal.draw(|f| render(f, state))?;
                    start_firstboot_worker(state, &worker_tx);
                }
                WorkerMessage::WriteError(e) => {
                    state.screen = Screen::Error(e);
                }
                WorkerMessage::FirstbootDone => {
                    state.screen = Screen::Done;
                }
                WorkerMessage::FirstbootError(e) => {
                    state.screen = Screen::Error(e);
                }
            }
        }

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        match handle_key(key, state) {
            KeyAction::Continue => {}
            KeyAction::Quit => break,
            // r[impl installer.no-reboot]
            KeyAction::Reboot => {
                if !no_reboot {
                    reboot();
                }
                break;
            }
            KeyAction::StartWrite => {
                start_write_worker(image_path, state, &worker_tx);
            }
        }
    }

    Ok(())
}

// r[impl installer.dryrun.script]
// r[impl installer.dryrun.script.headless]
/// Run the TUI state machine driven by a pre-recorded sequence of key events,
/// without initialising a real terminal. Returns the final `AppState` so the
/// caller can inspect decisions or produce an install plan.
pub fn run_tui_scripted(mut state: AppState, events: Vec<KeyEvent>) -> AppState {
    for key in events {
        // Poll async results between key events
        state.poll_net_checks();
        state.poll_tailscale_netcheck();
        state.poll_github_keys();

        match handle_key(key, &mut state) {
            KeyAction::Quit | KeyAction::StartWrite | KeyAction::Reboot => break,
            KeyAction::Continue => {}
        }
    }
    state
}

fn start_write_worker(image_path: &Path, state: &AppState, tx: &mpsc::Sender<WorkerMessage>) {
    let source = image_path.to_path_buf();
    let (target, disk_size) = match state.selected_disk() {
        Some(d) => (d.path.clone(), d.size_bytes),
        None => {
            let _ = tx.send(WorkerMessage::WriteError("no disk selected".into()));
            return;
        }
    };
    let tx = tx.clone();

    thread::spawn(move || {
        // r[impl installer.write.disk-size-check]
        if let Err(e) = writer::image_uncompressed_size(&source)
            .and_then(|image_size| writer::check_disk_size(image_size, disk_size))
        {
            let _ = tx.send(WorkerMessage::WriteError(format!("{e:#}")));
            return;
        }

        let result = writer::write_image(&source, &target, &mut |progress| {
            let _ = tx.send(WorkerMessage::Progress(progress.into()));
        });

        match result {
            Ok(()) => {
                if let Err(e) = writer::reread_partition_table(&target) {
                    tracing::warn!("partition table re-read failed: {e}");
                }
                if let Err(e) = writer::verify_partition_table(&target) {
                    tracing::warn!("partition table verification failed: {e}");
                }
                if let Err(e) = writer::expand_partitions(&target) {
                    tracing::warn!("partition expansion failed: {e}");
                }
                let _ = tx.send(WorkerMessage::WriteDone);
            }
            Err(e) => {
                let _ = tx.send(WorkerMessage::WriteError(format!("{e:#}")));
            }
        }
    });
}

fn start_firstboot_worker(state: &AppState, tx: &mpsc::Sender<WorkerMessage>) {
    let target_disk = match state.selected_disk() {
        Some(d) => d.path.clone(),
        None => {
            let _ = tx.send(WorkerMessage::FirstbootError("no disk selected".into()));
            return;
        }
    };
    let variant = state.variant;
    let disable_tpm = state.disable_tpm;
    let firstboot_config = state.firstboot_config();
    let tx = tx.clone();

    thread::spawn(move || {
        let result = run_firstboot(
            &target_disk,
            variant,
            disable_tpm,
            firstboot_config.as_ref(),
        );
        match result {
            Ok(()) => {
                let _ = tx.send(WorkerMessage::FirstbootDone);
            }
            Err(e) => {
                let _ = tx.send(WorkerMessage::FirstbootError(format!("{e:#}")));
            }
        }
    });
}

fn run_firstboot(
    target_disk: &Path,
    variant: Variant,
    disable_tpm: bool,
    config: Option<&crate::config::FirstbootConfig>,
) -> Result<()> {
    let has_work = config.is_some() || (variant == Variant::Metal && disable_tpm);
    if !has_work {
        return Ok(());
    }

    let mounted = firstboot::mount_target(target_disk, variant)?;

    if let Some(fb) = config {
        firstboot::apply_firstboot(&mounted, fb)?;
    }

    if variant == Variant::Metal && disable_tpm {
        firstboot::apply_tpm_disable(&mounted)?;
    }

    firstboot::unmount_target(mounted)?;
    Ok(())
}

fn reboot() {
    tracing::info!("rebooting system");
    let _ = std::process::Command::new("reboot").status();
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::*;
    use crate::config::{FirstbootConfig, Variant};
    use crate::disk::{BlockDevice, TransportType};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn make_state() -> AppState {
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
            vec![
                "America/New_York".into(),
                "Europe/London".into(),
                "Pacific/Auckland".into(),
                "UTC".into(),
            ],
        )
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_walk_through_metal_flow() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection
            press(KeyCode::Enter),
            // VariantSelection (Metal) -> TpmToggle
            press(KeyCode::Enter),
            // TpmToggle: toggle disable, then advance -> Hostname
            press(KeyCode::Char(' ')),
            press(KeyCode::Enter),
            // Hostname: type "myhost" then advance -> Tailscale
            press(KeyCode::Char('m')),
            press(KeyCode::Char('y')),
            press(KeyCode::Char('h')),
            press(KeyCode::Char('o')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('t')),
            press(KeyCode::Enter),
            // Tailscale: skip -> SshKeys
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub focus, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip (empty) — Enter moves to confirm field, Enter again advances
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: accept default (UTC) -> NetworkResults
            press(KeyCode::Enter),
            // NetworkResults -> Confirmation
            press(KeyCode::Enter),
            // Confirmation: type "yes" and confirm
            press(KeyCode::Char('y')),
            press(KeyCode::Char('e')),
            press(KeyCode::Char('s')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Writing);
        assert_eq!(final_state.variant, Variant::Metal);
        assert!(final_state.disable_tpm);
        assert_eq!(final_state.hostname_input, "myhost");
        assert_eq!(final_state.selected_disk_index, 0);
        assert!(final_state.is_confirmed());
        assert_eq!(final_state.timezone_selected, "UTC");
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_cloud_skips_tpm() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection
            press(KeyCode::Enter),
            // VariantSelection: toggle to Cloud, then advance (skip TpmToggle)
            press(KeyCode::Down),
            press(KeyCode::Enter),
            // Hostname: skip (cloud) -> Tailscale
            press(KeyCode::Enter),
            // Tailscale: skip -> SshKeys
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub focus, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: accept default -> NetworkResults
            press(KeyCode::Enter),
            // NetworkResults -> Confirmation
            press(KeyCode::Enter),
            // Confirmation: type "yes" and confirm
            press(KeyCode::Char('y')),
            press(KeyCode::Char('e')),
            press(KeyCode::Char('s')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Writing);
        assert_eq!(final_state.variant, Variant::Cloud);
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_quit_on_welcome() {
        let state = make_state();
        let events = vec![press(KeyCode::Char('q'))];
        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Welcome);
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_empty_events_keeps_initial_state() {
        let state = make_state();
        let final_state = run_tui_scripted(state, vec![]);
        assert_eq!(final_state.screen, Screen::Welcome);
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_disk_navigation() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // Navigate down to second disk
            press(KeyCode::Down),
            // Accept
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.selected_disk_index, 1);
        assert_eq!(final_state.screen, Screen::VariantSelection);
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_prefilled_hostname_with_backspace() {
        let fb = FirstbootConfig {
            hostname: Some("old-host".into()),
            ..Default::default()
        };
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        let state = AppState::new(
            devices,
            Variant::Cloud,
            false,
            Some(fb),
            None,
            None,
            String::new(),
            vec![
                "America/New_York".into(),
                "Europe/London".into(),
                "Pacific/Auckland".into(),
                "UTC".into(),
            ],
        );

        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: erase "old-host" with 8 backspaces
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            // Type new hostname
            press(KeyCode::Char('n')),
            press(KeyCode::Char('e')),
            press(KeyCode::Char('w')),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.hostname_input, "new");
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_go_back_from_confirmation() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: accept default -> NetworkResults
            press(KeyCode::Enter),
            // NetworkResults -> Confirmation
            press(KeyCode::Enter),
            // Confirmation: go back -> NetworkResults
            press(KeyCode::Esc),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
    }

    // r[verify installer.tui.password]
    #[test]
    fn scripted_password_entry_matching() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: type "abc", tab/enter to confirm field, type "abc", enter
            press(KeyCode::Char('a')),
            press(KeyCode::Char('b')),
            press(KeyCode::Char('c')),
            press(KeyCode::Enter),
            press(KeyCode::Char('a')),
            press(KeyCode::Char('b')),
            press(KeyCode::Char('c')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Timezone);
        assert_eq!(final_state.password_input, "abc");
        assert_eq!(final_state.password_confirm_input, "abc");
        assert!(!final_state.password_mismatch);
    }

    // r[verify installer.tui.password]
    #[test]
    fn scripted_password_mismatch_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" (required for metal) then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Type password
            press(KeyCode::Char('a')),
            press(KeyCode::Char('b')),
            // Tab to confirm field
            press(KeyCode::Tab),
            // Type different confirmation
            press(KeyCode::Char('x')),
            press(KeyCode::Char('y')),
            // Enter — should NOT advance due to mismatch
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Password);
        assert!(final_state.password_mismatch);
    }

    // r[verify installer.tui.password]
    #[test]
    fn scripted_password_esc_from_confirm_returns_to_first_field() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" (required for metal) then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Type password
            press(KeyCode::Char('a')),
            // Tab to confirm
            press(KeyCode::Tab),
            // Esc goes back to first field (not previous screen)
            press(KeyCode::Esc),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Password);
        assert!(!final_state.password_confirming);
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_metal_empty_hostname_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: press Enter with empty input — should NOT advance
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Hostname);
        assert_eq!(final_state.variant, Variant::Metal);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_metal_hostname_typed_allows_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type a name then advance
            press(KeyCode::Char('s')),
            press(KeyCode::Char('r')),
            press(KeyCode::Char('v')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Tailscale);
        assert_eq!(final_state.hostname_input, "srv");
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_cloud_empty_hostname_allows_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection
            press(KeyCode::Enter),
            // Toggle to Cloud
            press(KeyCode::Down),
            // VariantSelection -> Hostname (skips TpmToggle)
            press(KeyCode::Enter),
            // Hostname: press Enter with empty input — should advance for cloud
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Tailscale);
        assert_eq!(final_state.variant, Variant::Cloud);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_metal_dhcp_toggle_allows_advance_with_empty_hostname() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: toggle DHCP on via Tab, then advance with empty input
            press(KeyCode::Tab),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Tailscale);
        assert!(final_state.hostname_from_dhcp);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_metal_dhcp_toggle_via_space() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: toggle DHCP on via Space, then advance
            press(KeyCode::Char(' ')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Tailscale);
        assert!(final_state.hostname_from_dhcp);
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_metal_dhcp_toggle_on_off_requires_hostname() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: toggle DHCP on, then off again
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Now DHCP is off and hostname is empty — Enter should NOT advance
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Hostname);
        assert!(!final_state.hostname_from_dhcp);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_metal_dhcp_on_ignores_typing() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: toggle DHCP on, then try to type
            press(KeyCode::Tab),
            press(KeyCode::Char('a')),
            press(KeyCode::Char('b')),
            press(KeyCode::Char('c')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Tailscale);
        assert!(final_state.hostname_from_dhcp);
        // Typing should have been ignored while DHCP toggle is on
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+2]
    #[test]
    fn scripted_cloud_tab_does_not_toggle_dhcp() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection
            press(KeyCode::Enter),
            // Toggle to Cloud
            press(KeyCode::Down),
            // VariantSelection -> Hostname (skips TpmToggle)
            press(KeyCode::Enter),
            // Hostname: Tab should NOT toggle DHCP for cloud (it's not metal)
            press(KeyCode::Tab),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Tailscale);
        assert!(!final_state.hostname_from_dhcp);
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn handle_key_ignores_release_events() {
        let mut state = make_state();
        let release = KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Release,
            state: KeyEventState::empty(),
        };
        let action = handle_key(release, &mut state);
        assert!(matches!(action, KeyAction::Continue));
        assert_eq!(state.screen, Screen::Welcome);
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_accept_default_utc() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: accept default (UTC) -> NetworkResults
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "UTC");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_navigate_down_and_select() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone list (sorted): America/New_York=0, Europe/London=1,
            // Pacific/Auckland=2, UTC=3. Cursor starts at UTC (index 3).
            // Up once moves to Pacific/Auckland (index 2).
            press(KeyCode::Up),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "Pacific/Auckland");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_search_filters_list() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: type "auckland" to filter, then select first match
            press(KeyCode::Char('a')),
            press(KeyCode::Char('u')),
            press(KeyCode::Char('c')),
            press(KeyCode::Char('k')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "Pacific/Auckland");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_search_backspace_widens_filter() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: type "zzz" (matches nothing), then backspace all, then select
            press(KeyCode::Char('z')),
            press(KeyCode::Char('z')),
            press(KeyCode::Char('z')),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            // Filter is now empty again — all timezones visible, cursor at 0
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        // After clearing the filter, cursor resets to 0 which is the first sorted entry
        assert_eq!(final_state.timezone_selected, "America/New_York");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_esc_goes_back_to_password() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: press Esc to go back
            press(KeyCode::Esc),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Password);
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_down_does_not_go_past_end() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: press Down many times (more than list length), then select
            // List has 4 entries: America/New_York, Europe/London, Pacific/Auckland, UTC
            // Cursor starts at UTC (index 3). Down should not go past 3.
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        // Should still be UTC (last in sorted list, can't go past it)
        assert_eq!(final_state.timezone_selected, "UTC");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_up_does_not_go_before_start() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: press Up many times (more than cursor position), then select
            // Cursor starts at UTC (index 3 in sorted list).
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "America/New_York");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_search_then_navigate() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> VariantSelection -> TpmToggle -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Tailscale: skip
            press(KeyCode::Enter),
            // SshKeys: Tab -> GitHub, Tab -> advance to Password
            press(KeyCode::Tab),
            press(KeyCode::Tab),
            // Password: skip
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Timezone: type "o" — matches America/New_York and Europe/London
            press(KeyCode::Char('o')),
            // Navigate down to second match and select
            press(KeyCode::Down),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        // "o" matches: America/New_York (index 0 in filtered), Europe/London (index 1)
        // Down moves cursor to 1 -> Europe/London
        assert_eq!(final_state.timezone_selected, "Europe/London");
    }
}
