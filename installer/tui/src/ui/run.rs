use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
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

        // Drain any messages from background workers
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

        if key.kind != KeyEventKind::Press {
            continue;
        }

        match &state.screen {
            Screen::Welcome => match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Enter => state.advance(),
                _ => {}
            },

            Screen::DiskSelection => match key.code {
                KeyCode::Char('q') => break,
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
                KeyCode::Char('q') => break,
                KeyCode::Esc => state.go_back(),
                KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
                    state.toggle_variant();
                }
                KeyCode::Enter => state.advance(),
                _ => {}
            },

            Screen::TpmToggle => match key.code {
                KeyCode::Char('q') => break,
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
                // Tab advances to the next screen (Enter adds a newline)
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
                KeyCode::Char('q') if state.confirm_input.is_empty() => break,
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
                        start_write_worker(image_path, state, &worker_tx);
                    }
                }
                KeyCode::Char(c) => {
                    state.confirm_input.push(c);
                }
                _ => {}
            },

            Screen::Writing | Screen::FirstbootApply => {
                // No input during writes
            }

            Screen::Done => {
                reboot();
                break;
            }

            Screen::Error(_) => {
                break;
            }
        }
    }

    Ok(())
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
