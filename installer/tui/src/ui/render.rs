use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};

use crate::disk::BlockDevice;
use crate::writer::format_eta;

use super::{AppState, Screen};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(frame, chunks[0], state);

    match &state.screen {
        Screen::Welcome => render_welcome(frame, chunks[1], state),
        Screen::DiskSelection => render_disk_selection(frame, chunks[1], state),
        Screen::VariantSelection => render_variant_selection(frame, chunks[1], state),
        Screen::TpmToggle => render_tpm_toggle(frame, chunks[1], state),
        Screen::Hostname => render_hostname(frame, chunks[1], state),
        Screen::Tailscale => render_tailscale(frame, chunks[1], state),
        Screen::SshKeys => render_ssh_keys(frame, chunks[1], state),
        Screen::Password => render_password(frame, chunks[1], state),
        Screen::Confirmation => render_confirmation(frame, chunks[1], state),
        Screen::Writing => render_writing(frame, chunks[1], state),
        Screen::FirstbootApply => render_firstboot(frame, chunks[1]),
        Screen::Done => render_done(frame, chunks[1]),
        Screen::Error(msg) => render_error(frame, chunks[1], msg),
    }

    render_footer(frame, chunks[2], state);
}

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    let step = match &state.screen {
        Screen::Welcome => "Welcome",
        Screen::DiskSelection => "1/8 Select Target Disk",
        Screen::VariantSelection => "2/8 Select Variant",
        Screen::TpmToggle => "2/8 TPM Configuration",
        Screen::Hostname => "3/8 Hostname",
        Screen::Tailscale => "4/8 Tailscale",
        Screen::SshKeys => "5/8 SSH Keys",
        Screen::Password => "6/8 Password",
        Screen::Confirmation => "7/8 Confirm",
        Screen::Writing => "8/8 Writing Image",
        Screen::FirstbootApply => "8/8 Applying Configuration",
        Screen::Done => "Complete",
        Screen::Error(_) => "Error",
    };
    let title = if state.build_info.is_empty() {
        format!(" BES Installer -- {step} ")
    } else {
        format!(" BES Installer -- {step} | {} ", state.build_info)
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(block, area);
}

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let hints = match &state.screen {
        Screen::Welcome => "Enter: start | q: quit",
        Screen::DiskSelection => "Up/Down: select | Enter: next | Esc: back | q: quit",
        Screen::VariantSelection => "Up/Down: select | Enter: next | Esc: back | q: quit",
        Screen::TpmToggle => "Space: toggle | Enter: next | Esc: back | q: quit",
        Screen::Hostname => "Enter: next | Esc: back",
        Screen::Tailscale => "Enter: next | Esc: back",
        Screen::SshKeys => "Tab: next | Enter: new line | Esc: back",
        Screen::Password => {
            if state.password_confirming {
                "Enter: confirm | Esc: back to password"
            } else {
                "Enter/Tab: next | Esc: back"
            }
        }
        Screen::Confirmation => "Type 'yes' to confirm | Esc: back | q: quit",
        Screen::Writing => "Please wait...",
        Screen::FirstbootApply => "Please wait...",
        Screen::Done => "Press any key to reboot",
        Screen::Error(_) => "Press any key to exit",
    };
    let paragraph = Paragraph::new(hints).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.welcome]
fn render_welcome(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut description = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Tamanu Linux",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Tamanu Linux is BES's preferred system layout for Linux deployments."),
        Line::from("  It is based on Ubuntu Server. If you're not installing a Tamanu or"),
        Line::from("  other BES system, you may want to use the normal Ubuntu Server ISO."),
        Line::from(""),
        Line::from("  Available variants:"),
        Line::from(Span::styled(
            "    metal  — Full-disk encryption, hardware-locked when a TPM is available",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "    cloud  — For cloud or on-prem VMs where disk encryption is not needed",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from("  For support, contact BES at:"),
        Line::from(Span::styled(
            "    https://bes.au",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),
        )),
    ];

    if !state.build_info.is_empty() {
        description.push(Line::from(""));
        description.push(Line::from(Span::styled(
            format!("  {}", state.build_info),
            Style::default().fg(Color::DarkGray),
        )));
    }

    description.push(Line::from(""));
    description.push(Line::from("  Press Enter to begin."));

    let paragraph = Paragraph::new(description).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.disk-detection]
fn render_disk_selection(frame: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let is_boot = state.boot_device.as_ref().is_some_and(|bd| *bd == dev.path);
            let boot_marker = if is_boot { " (boot)" } else { "" };
            let line = format!(
                "{} {} {} [{}]{}",
                dev.path.display(),
                dev.size_display(),
                model_or_unknown(dev),
                dev.transport,
                boot_marker,
            );
            let style = if i == state.selected_disk_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if is_boot {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let block = Block::default()
        .title(" Target Disk ")
        .borders(Borders::ALL);

    let list = List::new(items).block(block).highlight_symbol(">> ");
    frame.render_widget(list, area);
}

// r[impl installer.tui.variant-selection]
fn render_variant_selection(frame: &mut Frame, area: Rect, state: &AppState) {
    let variants = [
        ("metal", "Full-disk encryption (LUKS2) with TPM auto-unlock"),
        ("cloud", "No encryption, for cloud/VM deployments"),
    ];

    let items: Vec<ListItem> = variants
        .iter()
        .map(|(name, desc)| {
            let is_selected = (*name == "metal" && state.variant == crate::config::Variant::Metal)
                || (*name == "cloud" && state.variant == crate::config::Variant::Cloud);
            let marker = if is_selected { "[x]" } else { "[ ]" };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("{marker} {name}: {desc}")).style(style)
        })
        .collect();

    let block = Block::default()
        .title(" Image Variant ")
        .borders(Borders::ALL);

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

// r[impl installer.tui.tpm-toggle]
// r[impl image.tpm.disableable]
fn render_tpm_toggle(frame: &mut Frame, area: Rect, state: &AppState) {
    let status = if state.disable_tpm {
        "DISABLED"
    } else {
        "ENABLED"
    };
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  TPM auto-enrollment: "),
            Span::styled(
                status,
                if state.disable_tpm {
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                },
            ),
        ]),
        Line::from(""),
        Line::from("  When enabled, the system will automatically enroll the LUKS key"),
        Line::from("  in the TPM2 on first boot, allowing unattended disk unlock."),
        Line::from(""),
        Line::from("  Press Space to toggle, Enter to continue."),
    ];

    let block = Block::default()
        .title(" TPM Configuration ")
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.hostname+2]
// r[impl installer.tui.hostname+2]
fn render_hostname(frame: &mut Frame, area: Rect, state: &AppState) {
    let hint = if state.hostname_required() {
        "  A hostname is required for the metal variant."
    } else {
        "  Leave empty to skip (default: ubuntu, overridden by DHCP/cloud-init)."
    };

    let mut lines = vec![
        Line::from(""),
        Line::from("  Enter the hostname for this system."),
        Line::from(hint),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Hostname: "),
            Span::styled(
                &state.hostname_input,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    if state.hostname_required() && state.hostname_input.trim().is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Hostname cannot be empty for the metal variant.",
            Style::default().fg(Color::Red),
        )));
    }

    let block = Block::default().title(" Hostname ").borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.tailscale]
fn render_tailscale(frame: &mut Frame, area: Rect, state: &AppState) {
    let lines = vec![
        Line::from(""),
        Line::from("  Enter a Tailscale auth key for automatic enrollment."),
        Line::from("  Leave empty to skip Tailscale configuration."),
        Line::from(""),
        Line::from(Span::styled(
            "  The key will be used on first boot to run 'tailscale up --auth-key --ssh'",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  and will be deleted after use.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Auth key: "),
            Span::styled(
                &state.tailscale_input,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let block = Block::default().title(" Tailscale ").borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn mask(input: &str) -> String {
    "*".repeat(input.len())
}

// r[impl installer.tui.ssh-keys]
fn render_ssh_keys(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::vertical([Constraint::Length(5), Constraint::Min(0)]).split(area);

    let intro = vec![
        Line::from(""),
        Line::from("  Paste SSH public keys for the 'ubuntu' user (one per line)."),
        Line::from("  Leave empty to skip. Press Tab when done."),
        Line::from(""),
    ];
    let intro_paragraph = Paragraph::new(intro);
    frame.render_widget(intro_paragraph, chunks[0]);

    let key_lines: Vec<Line> = if state.ssh_keys_input.is_empty() {
        vec![Line::from(Span::styled(
            "_",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        let mut lines: Vec<Line> = state
            .ssh_keys_input
            .lines()
            .map(|l| {
                Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(Color::Yellow),
                ))
            })
            .collect();
        // Show cursor on the last line
        if state.ssh_keys_input.ends_with('\n') {
            lines.push(Line::from(Span::styled(
                "_",
                Style::default().fg(Color::DarkGray),
            )));
        } else if let Some(last) = lines.last_mut() {
            last.spans
                .push(Span::styled("_", Style::default().fg(Color::DarkGray)));
        }
        lines
    };

    let block = Block::default()
        .title(" SSH Authorized Keys ")
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(key_lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[1]);
}

// r[impl installer.tui.confirmation]
// r[impl installer.tui.password]
fn render_password(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = vec![
        Line::from(""),
        Line::from("  Set a password for the 'ubuntu' user."),
        Line::from("  Leave both fields empty to keep the default password (expired)."),
        Line::from(""),
    ];

    let password_label = if state.password_confirming {
        Span::raw("  Password: ")
    } else {
        Span::styled(
            "  Password: ",
            Style::default().add_modifier(Modifier::BOLD),
        )
    };

    let masked = mask(&state.password_input);
    let mut password_line = vec![
        password_label,
        Span::styled(
            masked,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if !state.password_confirming {
        password_line.push(Span::styled("_", Style::default().fg(Color::DarkGray)));
    }
    lines.push(Line::from(password_line));

    let confirm_label = if state.password_confirming {
        Span::styled(
            "  Confirm:  ",
            Style::default().add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  Confirm:  ")
    };

    let confirm_masked = mask(&state.password_confirm_input);
    let mut confirm_line = vec![
        confirm_label,
        Span::styled(
            confirm_masked,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if state.password_confirming {
        confirm_line.push(Span::styled("_", Style::default().fg(Color::DarkGray)));
    }
    lines.push(Line::from(confirm_line));

    if state.password_mismatch {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Passwords do not match.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }

    let block = Block::default().title(" Password ").borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_confirmation(frame: &mut Frame, area: Rect, state: &AppState) {
    let disk = state.selected_disk();
    let disk_desc = disk
        .map(|d| {
            format!(
                "{} ({}, {})",
                d.path.display(),
                model_or_unknown(d),
                d.size_display()
            )
        })
        .unwrap_or_else(|| "(none)".into());

    let tpm_status = if state.variant == crate::config::Variant::Metal {
        if state.disable_tpm {
            "disabled"
        } else {
            "enabled"
        }
    } else {
        "n/a"
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Target disk:  "),
            Span::styled(&disk_desc, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw("  Variant:      "),
            Span::styled(
                state.variant.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  TPM enroll:   "),
            Span::styled(tpm_status, Style::default().add_modifier(Modifier::BOLD)),
        ]),
    ];

    if let Some(fb) = state.firstboot_config() {
        if let Some(ref h) = fb.hostname {
            lines.push(Line::from(vec![
                Span::raw("  Hostname:     "),
                Span::styled(h.to_string(), Style::default().add_modifier(Modifier::BOLD)),
            ]));
        }
        if fb.tailscale_authkey.is_some() {
            lines.push(Line::from(vec![
                Span::raw("  Tailscale:    "),
                Span::styled(
                    "auth key provided",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        if !fb.ssh_authorized_keys.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("  SSH keys:     "),
                Span::styled(
                    format!("{} key(s)", fb.ssh_authorized_keys.len()),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        if fb.password.is_some() || fb.password_hash.is_some() {
            lines.push(Line::from(vec![
                Span::raw("  Password:     "),
                Span::styled(
                    "custom password set",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  WARNING: ALL DATA ON THE TARGET DISK WILL BE DESTROYED",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(format!(
        "  Type '{}' to proceed: {}",
        state.confirmation_text(),
        state.confirm_input
    )));

    let block = Block::default()
        .title(" Confirm Installation ")
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.progress]
fn render_writing(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(area);

    if let Some(ref progress) = state.write_progress {
        let fraction = progress
            .total_bytes
            .map(|t| {
                if t == 0 {
                    0.0
                } else {
                    (progress.bytes_written as f64 / t as f64).min(1.0)
                }
            })
            .unwrap_or(0.0);

        let eta_str = progress.eta.map(format_eta).unwrap_or_default();
        let label = format!(
            "{:.1} MiB written | {:.1} MiB/s | ETA: {}",
            progress.bytes_written as f64 / (1024.0 * 1024.0),
            progress.throughput_mbps,
            if eta_str.is_empty() { "..." } else { &eta_str },
        );

        let info_lines = vec![
            Line::from(""),
            Line::from("  Writing image to disk..."),
            Line::from(""),
            Line::from(format!("  {label}")),
        ];
        let info = Paragraph::new(info_lines);
        frame.render_widget(info, chunks[0]);

        let gauge = Gauge::default()
            .block(Block::default().title(" Progress ").borders(Borders::ALL))
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(fraction);
        frame.render_widget(gauge, chunks[1]);
    } else {
        let paragraph = Paragraph::new("  Preparing to write...");
        frame.render_widget(paragraph, area);
    }
}

fn render_firstboot(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from("  Applying first-boot configuration..."),
        Line::from(""),
        Line::from("  Mounting target filesystem and writing settings."),
    ];
    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_done(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Installation complete!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Remove the installation media and press any key to reboot."),
    ];
    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn render_error(frame: &mut Frame, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Installation failed!",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("  Error: {msg}")),
        Line::from(""),
        Line::from("  Press any key to exit."),
    ];
    let block = Block::default().title(" Error ").borders(Borders::ALL);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn model_or_unknown(dev: &BlockDevice) -> &str {
    if dev.model.is_empty() {
        "unknown"
    } else {
        &dev.model
    }
}
