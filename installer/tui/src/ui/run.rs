use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::validate_hostname;
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

        // r[impl installer.tui.network-check+4]
        Screen::NetworkCheck => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Esc => state.go_back(),
            KeyCode::Char('r') => state.start_net_checks(),
            KeyCode::Tab => state.toggle_net_pane(),
            KeyCode::Up | KeyCode::Char('k') => state.scroll_net_up(),
            KeyCode::Down | KeyCode::Char('j') => state.scroll_net_down(),
            _ => {}
        },

        // r[impl installer.tui.network-check+4]
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

        // r[impl installer.tui.disk-encryption+2]
        Screen::DiskEncryption => match key.code {
            KeyCode::Char('q') => return KeyAction::Quit,
            KeyCode::Esc => state.go_back(),
            KeyCode::Down | KeyCode::Char('j') => {
                state.cycle_disk_encryption();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.cycle_disk_encryption_reverse();
            }
            KeyCode::Enter => state.advance(),
            _ => {}
        },

        // r[impl installer.tui.hostname+5]
        Screen::Hostname => match key.code {
            KeyCode::Esc => state.go_back(),
            KeyCode::Up | KeyCode::Down => {
                state.hostname_from_dhcp = !state.hostname_from_dhcp;
            }
            KeyCode::Enter => state.advance(),
            _ => {}
        },

        // r[impl installer.tui.hostname+5]
        Screen::HostnameInput => match key.code {
            KeyCode::Esc => {
                state.hostname_error = None;
                state.go_back();
            }
            KeyCode::Enter => {
                let trimmed = state.hostname_input.trim();
                if trimmed.is_empty() {
                    state.hostname_error = Some("Hostname cannot be empty.".into());
                } else {
                    match validate_hostname(trimmed) {
                        Ok(()) => {
                            state.hostname_error = None;
                            state.advance();
                        }
                        Err(e) => {
                            state.hostname_error = Some(e);
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                state.hostname_input.pop();
                let trimmed = state.hostname_input.trim();
                if trimmed.is_empty() {
                    state.hostname_error = None;
                } else {
                    state.hostname_error = validate_hostname(trimmed).err();
                }
            }
            KeyCode::Char(c) => {
                state.hostname_input.push(c);
                let trimmed = state.hostname_input.trim();
                if trimmed.is_empty() {
                    state.hostname_error = None;
                } else {
                    state.hostname_error = validate_hostname(trimmed).err();
                }
            }
            _ => {}
        },

        // r[impl installer.tui.password+4]
        // r[impl installer.tui.tailscale+3]
        // r[impl installer.tui.ssh-keys+5]
        // r[impl installer.tui.ssh-keys.github+4]
        Screen::Login => match key.code {
            KeyCode::Esc => {
                if state.password_confirming {
                    state.password_confirming = false;
                    state.password_mismatch = false;
                    state.password_empty = false;
                } else {
                    state.go_back();
                }
            }
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::ALT) => {
                state.screen = Screen::LoginTailscale;
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::ALT) => {
                state.screen = Screen::LoginSshKeys;
            }
            KeyCode::Char('g')
                if key.modifiers.contains(KeyModifiers::ALT) && state.github_reachable() =>
            {
                state.screen = Screen::LoginGithub;
            }
            KeyCode::Enter | KeyCode::Tab => {
                if !state.password_confirming {
                    state.password_confirming = true;
                    state.password_mismatch = false;
                    state.password_empty = false;
                } else if state.password_input.is_empty() && state.password_confirm_input.is_empty()
                {
                    state.password_empty = true;
                    state.password_mismatch = false;
                } else if state.password_matches() {
                    state.password_mismatch = false;
                    state.password_empty = false;
                    state.advance();
                } else {
                    state.password_mismatch = true;
                    state.password_empty = false;
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
                state.password_empty = false;
                if state.password_confirming {
                    state.password_confirm_input.push(c);
                } else {
                    state.password_input.push(c);
                }
            }
            _ => {}
        },

        // r[impl installer.tui.tailscale+3]
        Screen::LoginTailscale => match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                state.screen = Screen::Login;
            }
            KeyCode::Backspace => {
                state.tailscale_input.pop();
            }
            KeyCode::Char(c) => {
                state.tailscale_input.push(c);
            }
            _ => {}
        },

        // r[impl installer.tui.ssh-keys+5]
        Screen::LoginSshKeys => match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                state.filter_ssh_keys();
                state.screen = Screen::Login;
            }
            KeyCode::Tab => {
                let len = state.ssh_keys.len();
                state.ssh_key_cursor = (state.ssh_key_cursor + 1) % len;
            }
            KeyCode::BackTab => {
                if state.ssh_key_cursor == 0 {
                    state.ssh_key_cursor = state.ssh_keys.len() - 1;
                } else {
                    state.ssh_key_cursor -= 1;
                }
            }
            KeyCode::Backspace => {
                state.ssh_keys[state.ssh_key_cursor].pop();
            }
            KeyCode::Char(c) => {
                state.ssh_keys[state.ssh_key_cursor].push(c);
                state.ensure_trailing_blank();
            }
            _ => {}
        },

        // r[impl installer.tui.ssh-keys.github+4]
        Screen::LoginGithub => match key.code {
            KeyCode::Esc => {
                state.screen = Screen::Login;
            }
            KeyCode::Enter => {
                if !state.ssh_github_fetching {
                    state.start_github_key_fetch();
                }
            }
            KeyCode::Backspace => {
                state.ssh_github_input.pop();
                state.ssh_github_error = None;
            }
            KeyCode::Char(c) => {
                state.ssh_github_input.push(c);
                state.ssh_github_error = None;
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
    terminal.clear()?;

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
    let disk_encryption = state.disk_encryption;
    let firstboot_config = state.firstboot_config();
    let tx = tx.clone();

    thread::spawn(move || {
        let result = run_firstboot(&target_disk, disk_encryption, firstboot_config.as_ref());
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
    disk_encryption: crate::config::DiskEncryption,
    config: Option<&crate::config::FirstbootConfig>,
) -> Result<()> {
    if config.is_none() {
        return Ok(());
    }

    let mounted = firstboot::mount_target(target_disk, disk_encryption)?;

    if let Some(fb) = config {
        firstboot::apply_firstboot(&mounted, fb)?;
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
    use crate::config::{DiskEncryption, FirstbootConfig};
    use crate::disk::{BlockDevice, TransportType};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn alt(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::ALT,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    /// Generate key events to type password "pw", confirm it, and advance.
    fn type_password() -> Vec<KeyEvent> {
        vec![
            press(KeyCode::Char('p')),
            press(KeyCode::Char('w')),
            press(KeyCode::Enter),
            press(KeyCode::Char('p')),
            press(KeyCode::Char('w')),
            press(KeyCode::Enter),
        ]
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
            DiskEncryption::Tpm,
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
    fn scripted_walk_through_encrypted_flow() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> DiskEncryptionScreen (default: Tpm)
            press(KeyCode::Enter),
            // DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            // Hostname selector: Static is default for encrypted, Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "myhost" then advance -> Login
            press(KeyCode::Char('m')),
            press(KeyCode::Char('y')),
            press(KeyCode::Char('h')),
            press(KeyCode::Char('o')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('t')),
            press(KeyCode::Enter),
            // Login: type password "pw" + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: accept default (UTC) -> NetworkResults
            press(KeyCode::Enter),
            // NetworkResults -> Confirmation
            press(KeyCode::Enter),
            // Confirmation: type "yes" and confirm
            press(KeyCode::Char('y')),
            press(KeyCode::Char('e')),
            press(KeyCode::Char('s')),
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Writing);
        assert_eq!(final_state.disk_encryption, DiskEncryption::Tpm);
        assert_eq!(final_state.hostname_input, "myhost");
        assert_eq!(final_state.selected_disk_index, 0);
        assert!(final_state.is_confirmed());
        assert_eq!(final_state.timezone_selected, "UTC");
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_none_encryption_flow() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> DiskEncryptionScreen (default: Tpm)
            press(KeyCode::Enter),
            // Cycle: Tpm -> Keyfile -> None
            press(KeyCode::Down),
            press(KeyCode::Down),
            // DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            // Hostname selector: network-assigned is default for none encryption,
            // Enter -> Login (skip HostnameInput)
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: accept default -> NetworkResults
            press(KeyCode::Enter),
            // NetworkResults -> Confirmation
            press(KeyCode::Enter),
            // Confirmation: type "yes" and confirm
            press(KeyCode::Char('y')),
            press(KeyCode::Char('e')),
            press(KeyCode::Char('s')),
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Writing);
        assert_eq!(final_state.disk_encryption, DiskEncryption::None);
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
            // Accept -> DiskEncryptionScreen
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.selected_disk_index, 1);
        assert_eq!(final_state.screen, Screen::DiskEncryption);
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
            DiskEncryption::None,
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
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: config has hostname so Static is selected
            // Enter -> HostnameInput (prefilled with "old-host")
            press(KeyCode::Enter),
            // HostnameInput: erase "old-host" with 8 backspaces
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
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: accept default -> NetworkResults
            press(KeyCode::Enter),
            // NetworkResults -> Confirmation
            press(KeyCode::Enter),
            // Confirmation: go back -> NetworkResults
            press(KeyCode::Esc),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
    }

    // r[verify installer.tui.password+4]
    #[test]
    fn scripted_password_entry_matching() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type "abc", tab/enter to confirm field, type "abc", enter
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

    // r[verify installer.tui.password+4]
    #[test]
    fn scripted_password_mismatch_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" (required for encrypted) then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password
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
        assert_eq!(final_state.screen, Screen::Login);
        assert!(final_state.password_mismatch);
    }

    // r[verify installer.tui.password+4]
    #[test]
    fn scripted_empty_password_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" (required for encrypted) then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: Enter moves to confirm, then Enter again with empty fields
            press(KeyCode::Enter),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert!(final_state.password_empty);
        assert!(final_state.password_confirming);
    }

    // r[verify installer.tui.password+4]
    #[test]
    fn scripted_password_esc_from_confirm_returns_to_first_field() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" (required for encrypted) then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password
            press(KeyCode::Char('a')),
            // Tab to confirm
            press(KeyCode::Tab),
            // Esc goes back to first field (not previous screen)
            press(KeyCode::Esc),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert!(!final_state.password_confirming);
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_encrypted_empty_hostname_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static is default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: press Enter with empty input — should NOT advance
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::HostnameInput);
        assert_eq!(final_state.disk_encryption, DiskEncryption::Tpm);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_encrypted_hostname_typed_allows_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static is default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type a name then advance
            press(KeyCode::Char('s')),
            press(KeyCode::Char('r')),
            press(KeyCode::Char('v')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert_eq!(final_state.hostname_input, "srv");
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_none_encryption_network_assigned_default_advances_to_login() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> DiskEncryptionScreen (default: Tpm)
            press(KeyCode::Enter),
            // Cycle: Tpm -> Keyfile -> None
            press(KeyCode::Down),
            press(KeyCode::Down),
            // DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            // Hostname selector: network-assigned is default for none encryption,
            // Enter -> Login (skip HostnameInput)
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert_eq!(final_state.disk_encryption, DiskEncryption::None);
        assert!(final_state.hostname_from_dhcp);
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_none_encryption_static_empty_hostname_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> DiskEncryptionScreen
            press(KeyCode::Enter),
            // Cycle: Tpm -> Keyfile -> None
            press(KeyCode::Down),
            press(KeyCode::Down),
            // DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            // Hostname selector: network-assigned is default for none,
            // Up to select Static -> Enter -> HostnameInput
            press(KeyCode::Up),
            press(KeyCode::Enter),
            // HostnameInput: press Enter with empty input — should NOT advance
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::HostnameInput);
        assert_eq!(final_state.disk_encryption, DiskEncryption::None);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_encrypted_dhcp_selected_advances_to_login() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Down to select DHCP, then Enter -> Login
            press(KeyCode::Down),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert!(final_state.hostname_from_dhcp);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_encrypted_dhcp_then_static_requires_hostname() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Down (DHCP) -> Up (Static) -> Enter -> HostnameInput
            press(KeyCode::Down),
            press(KeyCode::Up),
            press(KeyCode::Enter),
            // HostnameInput: empty -> Enter should NOT advance
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::HostnameInput);
        assert!(!final_state.hostname_from_dhcp);
        assert!(final_state.hostname_input.is_empty());
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_encrypted_dhcp_skips_text_input() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Down to select DHCP, then Enter -> Login directly
            press(KeyCode::Down),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert!(final_state.hostname_from_dhcp);
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_hostname_input_esc_returns_to_selector() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: Esc -> back to Hostname selector
            press(KeyCode::Esc),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Hostname);
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_none_selector_navigation() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection
            press(KeyCode::Enter),
            // DiskSelection -> DiskEncryptionScreen
            press(KeyCode::Enter),
            // Cycle: Tpm -> Keyfile -> None
            press(KeyCode::Down),
            press(KeyCode::Down),
            // DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            // Hostname selector: network-assigned is default for none,
            // Up toggles to Static, Down toggles back to network-assigned,
            // Enter -> Login (skip HostnameInput)
            press(KeyCode::Up),
            press(KeyCode::Down),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert!(final_state.hostname_from_dhcp);
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_invalid_hostname_chars_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type invalid hostname then try to advance
            press(KeyCode::Char('!')),
            press(KeyCode::Char('?')),
            press(KeyCode::Char('$')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::HostnameInput);
        assert!(final_state.hostname_error.is_some());
        assert!(
            final_state
                .hostname_error
                .as_ref()
                .unwrap()
                .contains("letters, digits, and hyphens")
        );
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_leading_hyphen_hostname_blocks_advance() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type hostname starting with hyphen
            press(KeyCode::Char('-')),
            press(KeyCode::Char('b')),
            press(KeyCode::Char('a')),
            press(KeyCode::Char('d')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::HostnameInput);
        assert!(final_state.hostname_error.is_some());
        assert!(
            final_state
                .hostname_error
                .as_ref()
                .unwrap()
                .contains("hyphen")
        );
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_valid_hostname_advances() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type valid hostname
            press(KeyCode::Char('m')),
            press(KeyCode::Char('y')),
            press(KeyCode::Char('-')),
            press(KeyCode::Char('h')),
            press(KeyCode::Char('o')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('t')),
            press(KeyCode::Char('0')),
            press(KeyCode::Char('1')),
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert_eq!(final_state.hostname_input, "my-host01");
        assert!(final_state.hostname_error.is_none());
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_hostname_error_cleared_on_typing() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type a leading hyphen (error set live), then delete it
            // and type a valid char — error should be cleared
            press(KeyCode::Char('-')),
            press(KeyCode::Backspace),
            press(KeyCode::Char('a')),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::HostnameInput);
        assert!(final_state.hostname_error.is_none());
    }

    // r[verify installer.tui.hostname+5]
    #[test]
    fn scripted_hostname_error_shown_live_on_keystroke() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname selector
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type an invalid character — error should appear without Enter
            press(KeyCode::Char('!')),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::HostnameInput);
        assert!(final_state.hostname_error.is_some());
        assert!(
            final_state
                .hostname_error
                .as_ref()
                .unwrap()
                .contains("letters, digits, and hyphens")
        );
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
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: accept default (UTC) -> NetworkResults
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "UTC");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_navigate_down_and_select() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone list (sorted): America/New_York=0, Europe/London=1,
            // Pacific/Auckland=2, UTC=3. Cursor starts at UTC (index 3).
            // Up once moves to Pacific/Auckland (index 2).
            press(KeyCode::Up),
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "Pacific/Auckland");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_search_filters_list() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: type "auckland" to filter, then select first match
            press(KeyCode::Char('a')),
            press(KeyCode::Char('u')),
            press(KeyCode::Char('c')),
            press(KeyCode::Char('k')),
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "Pacific/Auckland");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_search_backspace_widens_filter() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: type "zzz" (matches nothing), then backspace all, then select
            press(KeyCode::Char('z')),
            press(KeyCode::Char('z')),
            press(KeyCode::Char('z')),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            press(KeyCode::Backspace),
            // Filter is now empty again — all timezones visible, cursor at 0
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        // After clearing the filter, cursor resets to 0 which is the first sorted entry
        assert_eq!(final_state.timezone_selected, "America/New_York");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_esc_goes_back_to_login() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: press Esc to go back
            press(KeyCode::Esc),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_down_does_not_go_past_end() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: press Down many times (more than list length), then select
            // List has 4 entries: America/New_York, Europe/London, Pacific/Auckland, UTC
            // Cursor starts at UTC (index 3). Down should not go past 3.
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        // Should still be UTC (last in sorted list, can't go past it)
        assert_eq!(final_state.timezone_selected, "UTC");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_up_does_not_go_before_start() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: press Up many times (more than cursor position), then select
            // Cursor starts at UTC (index 3 in sorted list).
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Up),
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        assert_eq!(final_state.timezone_selected, "America/New_York");
    }

    // r[verify installer.tui.timezone]
    #[test]
    fn scripted_timezone_search_then_navigate() {
        let state = make_state();
        let mut events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: type password + confirm + advance
        ];
        events.extend(type_password());
        events.extend([
            // Timezone: type "o" — matches America/New_York and Europe/London
            press(KeyCode::Char('o')),
            // Navigate down to second match and select
            press(KeyCode::Down),
            press(KeyCode::Enter),
        ]);

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::NetworkResults);
        // "o" matches: America/New_York (index 0 in filtered), Europe/London (index 1)
        // Down moves cursor to 1 -> Europe/London
        assert_eq!(final_state.timezone_selected, "Europe/London");
    }

    // r[verify installer.tui.tailscale+3]
    #[test]
    fn scripted_login_tailscale_sub_screen() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: press Alt+t to enter tailscale sub-screen
            alt('t'),
            // Type auth key
            press(KeyCode::Char('t')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('k')),
            // Return to Login via Enter
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert_eq!(final_state.tailscale_input, "tsk");
    }

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn scripted_login_ssh_keys_sub_screen() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: press Alt+s to enter ssh keys sub-screen
            alt('s'),
            // Type a key
            press(KeyCode::Char('s')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('h')),
            press(KeyCode::Char('-')),
            press(KeyCode::Char('e')),
            press(KeyCode::Char('d')),
            press(KeyCode::Char('2')),
            press(KeyCode::Char('5')),
            press(KeyCode::Char('5')),
            press(KeyCode::Char('1')),
            press(KeyCode::Char('9')),
            press(KeyCode::Char(' ')),
            press(KeyCode::Char('A')),
            press(KeyCode::Char('A')),
            press(KeyCode::Char('A')),
            press(KeyCode::Char('A')),
            // Return to Login via Esc (filters keys)
            press(KeyCode::Esc),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert_eq!(final_state.ssh_keys, vec!["ssh-ed25519 AAAA"]);
    }

    // r[verify installer.tui.ssh-keys+5]
    #[test]
    fn scripted_login_ssh_keys_tab_cycles_and_trailing_blank() {
        let state = make_state();
        let events = vec![
            // Welcome -> DiskSelection -> DiskEncryptionScreen -> Hostname
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            // Hostname selector: Static default -> Enter -> HostnameInput
            press(KeyCode::Enter),
            // HostnameInput: type "h" then advance
            press(KeyCode::Char('h')),
            press(KeyCode::Enter),
            // Login: press Alt+s to enter ssh keys sub-screen
            alt('s'),
            // Type first key
            press(KeyCode::Char('s')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('h')),
            press(KeyCode::Char('-')),
            press(KeyCode::Char('r')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('a')),
            press(KeyCode::Char(' ')),
            press(KeyCode::Char('B')),
            press(KeyCode::Char('B')),
            press(KeyCode::Char('B')),
            press(KeyCode::Char('B')),
            // Tab cycles to next field (the auto-appended trailing blank)
            press(KeyCode::Tab),
            // Type second key in the trailing blank field
            press(KeyCode::Char('s')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('h')),
            press(KeyCode::Char('-')),
            press(KeyCode::Char('d')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char('s')),
            press(KeyCode::Char(' ')),
            press(KeyCode::Char('C')),
            press(KeyCode::Char('C')),
            // Return to Login
            press(KeyCode::Enter),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::Login);
        assert_eq!(final_state.ssh_keys, vec!["ssh-rsa BBBB", "ssh-dss CC"]);
    }
}
