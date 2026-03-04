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

        // r[impl installer.tui.hostname]
        Screen::Hostname => match key.code {
            KeyCode::Esc => state.go_back(),
            KeyCode::Enter => state.advance(),
            KeyCode::Backspace => {
                state.hostname_input.pop();
            }
            KeyCode::Char(c) => {
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
        Screen::SshKeys => match key.code {
            KeyCode::Esc => state.go_back(),
            KeyCode::Tab => state.advance(),
            KeyCode::Enter => {
                state.ssh_keys_input.push('\n');
            }
            KeyCode::Backspace => {
                state.ssh_keys_input.pop();
            }
            KeyCode::Char(c) => {
                state.ssh_keys_input.push(c);
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
            reboot();
            return KeyAction::Quit;
        }

        Screen::Error(_) => {
            return KeyAction::Quit;
        }
    }

    KeyAction::Continue
}

pub fn run_tui(mut state: AppState, image_path: &Path) -> Result<()> {
    terminal::enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut state, image_path);

    terminal::disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show).ok();

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    image_path: &Path,
) -> Result<()> {
    let (worker_tx, worker_rx) = mpsc::channel::<WorkerMessage>();

    loop {
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
        match handle_key(key, &mut state) {
            KeyAction::Quit | KeyAction::StartWrite => break,
            KeyAction::Continue => {}
        }
    }
    state
}

fn start_write_worker(image_path: &Path, state: &AppState, tx: &mpsc::Sender<WorkerMessage>) {
    let source = image_path.to_path_buf();
    let target = match state.selected_disk() {
        Some(d) => d.path.clone(),
        None => {
            let _ = tx.send(WorkerMessage::WriteError("no disk selected".into()));
            return;
        }
    };
    let tx = tx.clone();

    thread::spawn(move || {
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
            // SshKeys: skip -> Confirmation
            press(KeyCode::Tab),
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
    }

    // r[verify installer.dryrun.script.headless]
    #[test]
    fn scripted_cloud_skips_tpm() {
        let state = make_state();
        let events = vec![
            press(KeyCode::Enter),
            // DiskSelection -> VariantSelection
            press(KeyCode::Enter),
            // Toggle to Cloud
            press(KeyCode::Down),
            // VariantSelection -> Hostname (skips TpmToggle)
            press(KeyCode::Enter),
            // Hostname: advance
            press(KeyCode::Enter),
            // Tailscale: advance
            press(KeyCode::Enter),
            // SshKeys: advance
            press(KeyCode::Tab),
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
            tailscale_authkey: None,
            ssh_authorized_keys: vec![],
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
            // Walk to Confirmation
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Enter),
            press(KeyCode::Tab),
            // Now on Confirmation — go back
            press(KeyCode::Esc),
        ];

        let final_state = run_tui_scripted(state, events);
        assert_eq!(final_state.screen, Screen::SshKeys);
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
}
