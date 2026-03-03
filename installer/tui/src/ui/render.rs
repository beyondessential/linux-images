use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};
use tui_big_text::{BigText, PixelSize};

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
        Screen::Welcome => render_welcome(frame, chunks[1]),
        Screen::DiskSelection => render_disk_selection(frame, chunks[1], state),
        Screen::VariantSelection => render_variant_selection(frame, chunks[1], state),
        Screen::TpmToggle => render_tpm_toggle(frame, chunks[1], state),
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
        Screen::DiskSelection => "1/4 Select Target Disk",
        Screen::VariantSelection => "2/4 Select Variant",
        Screen::TpmToggle => "2/4 TPM Configuration",
        Screen::Confirmation => "3/4 Confirm",
        Screen::Writing => "4/4 Writing Image",
        Screen::FirstbootApply => "4/4 Applying Configuration",
        Screen::Done => "Complete",
        Screen::Error(_) => "Error",
    };
    let block = Block::default()
        .title(format!(" BES Installer -- {step} "))
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
fn render_welcome(frame: &mut Frame, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area);

    let big_text = BigText::builder()
        .pixel_size(PixelSize::Quadrant)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .lines(vec!["BES Installer".into()])
        .alignment(Alignment::Center)
        .build();
    frame.render_widget(big_text, chunks[0]);

    let description = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  BES Linux Images — Disk Installer",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  This installer writes a pre-built BES Linux disk image to the"),
        Line::from("  target disk you select. The image contains a fully configured"),
        Line::from("  Ubuntu Server system with BES's preferred disk and system layout."),
        Line::from(""),
        Line::from("  Available variants:"),
        Line::from(Span::styled(
            "    metal  — Full-disk encryption (LUKS2) with optional TPM auto-unlock",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "    cloud  — No encryption, for cloud/VM deployments",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  WARNING: the selected disk will be completely overwritten.",
            Style::default().fg(Color::Red),
        )),
        Line::from(""),
        Line::from("  For support, contact BES at:"),
        Line::from(Span::styled(
            "    https://bearcove.eu",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from("  Press Enter to begin."),
    ];

    let paragraph = Paragraph::new(description).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[2]);
}

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

    if let Some(ref fb) = state.firstboot {
        if let Some(ref h) = fb.hostname {
            lines.push(Line::from(format!("  Hostname:     {h}")));
        }
        if fb.tailscale_authkey.is_some() {
            lines.push(Line::from("  Tailscale:    auth key provided"));
        }
        if !fb.ssh_authorized_keys.is_empty() {
            lines.push(Line::from(format!(
                "  SSH keys:     {}",
                fb.ssh_authorized_keys.len()
            )));
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
