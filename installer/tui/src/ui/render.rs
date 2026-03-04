use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};

use crate::disk::BlockDevice;
use crate::net::CheckPhase;
use crate::writer::format_eta;

use super::{AppState, NetPane, Screen};

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
        Screen::NetworkCheck => render_network_check(frame, chunks[1], state),
        Screen::NetworkResults => render_network_results(frame, chunks[1], state),
        Screen::DiskSelection => render_disk_selection(frame, chunks[1], state),
        Screen::VariantSelection => render_variant_selection(frame, chunks[1], state),
        Screen::TpmToggle => render_tpm_toggle(frame, chunks[1], state),
        Screen::Hostname => render_hostname(frame, chunks[1], state),
        Screen::Tailscale => render_tailscale(frame, chunks[1], state),
        Screen::SshKeys => render_ssh_keys(frame, chunks[1], state),
        Screen::Password => render_password(frame, chunks[1], state),
        Screen::Timezone => render_timezone(frame, chunks[1], state),
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
        Screen::NetworkCheck => "Network Check",
        Screen::DiskSelection => "1/8 Select Target Disk",
        Screen::VariantSelection => "2/8 Select Variant",
        Screen::TpmToggle => "2/8 TPM Configuration",
        Screen::Hostname => "3/8 Hostname",
        Screen::Tailscale => "4/8 Tailscale",
        Screen::SshKeys => "5/8 SSH Keys",
        Screen::Password => "6/8 Password",
        Screen::Timezone => "7/8 Timezone",
        Screen::NetworkResults => "Network Results",
        Screen::Confirmation => "8/8 Confirm",
        Screen::Writing => "Writing Image",
        Screen::FirstbootApply => "Applying Configuration",
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

/// Build the connectivity check list items.
fn connectivity_items(state: &AppState) -> Vec<ListItem<'_>> {
    let mut items: Vec<ListItem> = state
        .net_check_results
        .iter()
        .enumerate()
        .map(|(i, result)| match result {
            Some(r) => {
                let (icon, color) = if r.passed {
                    ("PASS", Color::Green)
                } else {
                    ("FAIL", Color::Red)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  [{icon}] "), Style::default().fg(color)),
                    Span::raw(&r.label),
                    Span::styled(
                        format!("  ({})", r.detail),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            }
            None => {
                let label = if i < state.net_check_total - 1 {
                    crate::net::default_endpoints()
                        .get(i)
                        .map(|e| e.label.as_str())
                        .unwrap_or("?")
                        .to_string()
                } else {
                    "pool.ntp.org:123 (NTP/UDP)".to_string()
                };
                ListItem::new(Line::from(vec![
                    Span::styled("  [ .. ] ", Style::default().fg(Color::DarkGray)),
                    Span::raw(label),
                ]))
            }
        })
        .collect();

    items.push(ListItem::new(Line::from("")));
    items
}

/// Build the tailscale netcheck output lines.
fn tailscale_lines(state: &AppState) -> Vec<Line<'_>> {
    match &state.netcheck_result {
        Some(result) => {
            let color = if result.success {
                Color::White
            } else {
                Color::Red
            };
            result
                .output
                .lines()
                .map(|l| Line::from(Span::styled(format!("  {l}"), Style::default().fg(color))))
                .collect()
        }
        None => {
            if state.netcheck_phase == CheckPhase::Running {
                vec![Line::from(Span::styled(
                    "  Waiting for tailscale netcheck results...",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                vec![Line::from(Span::styled(
                    "  Tailscale netcheck has not run.",
                    Style::default().fg(Color::DarkGray),
                ))]
            }
        }
    }
}

/// Render the two-pane accordion used by both NetworkCheck and NetworkResults.
///
/// The `intro_text` line is shown at the top. One pane is expanded with
/// scroll support; the other is collapsed to a single title bar.
fn render_net_accordion(frame: &mut Frame, area: Rect, state: &AppState, intro_text: &str) {
    let net_status = match state.net_check_phase {
        CheckPhase::NotStarted => "Not started",
        CheckPhase::Running => "Running...",
        CheckPhase::Done => "Done",
    };
    let ts_status = match state.netcheck_phase {
        CheckPhase::NotStarted => "Not started",
        CheckPhase::Running => "Running...",
        CheckPhase::Done => "Done",
    };

    let intro = vec![
        Line::from(""),
        Line::from(format!("  {intro_text}")),
        Line::from(Span::styled(
            format!(
                "  Connectivity: {} ({}/{})  |  Tailscale: {}",
                net_status,
                state.net_checks_done(),
                state.net_check_total,
                ts_status,
            ),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];

    // Collapsed pane: just a bordered title bar (height 3).
    // Expanded pane: takes remaining space.
    let (conn_constraint, ts_constraint) = match state.net_pane {
        NetPane::Connectivity => (Constraint::Min(0), Constraint::Length(3)),
        NetPane::Tailscale => (Constraint::Length(3), Constraint::Min(0)),
    };

    let chunks =
        Layout::vertical([Constraint::Length(5), conn_constraint, ts_constraint]).split(area);

    let intro_paragraph = Paragraph::new(intro);
    frame.render_widget(intro_paragraph, chunks[0]);

    // --- Connectivity pane ---
    let conn_active = state.net_pane == NetPane::Connectivity;
    let conn_border_style = if conn_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let conn_title = if conn_active {
        let total = state.net_check_line_count();
        let visible = chunks[1].height.saturating_sub(2) as usize;
        if total > visible {
            format!(" Connectivity (line {}/{}) ", state.net_scroll + 1, total,)
        } else {
            " Connectivity ".to_string()
        }
    } else {
        " Connectivity [Tab to expand] ".to_string()
    };
    let conn_block = Block::default()
        .title(conn_title)
        .borders(Borders::ALL)
        .border_style(conn_border_style);

    if conn_active {
        let items = connectivity_items(state);
        let list = List::new(items).block(conn_block);
        frame.render_stateful_widget(
            list,
            chunks[1],
            &mut ratatui::widgets::ListState::default().with_offset(state.net_scroll as usize),
        );
    } else {
        frame.render_widget(conn_block, chunks[1]);
    }

    // --- Tailscale pane ---
    let ts_active = state.net_pane == NetPane::Tailscale;
    let ts_border_style = if ts_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let ts_title = if ts_active {
        let total = state.netcheck_line_count();
        let visible = chunks[2].height.saturating_sub(2) as usize;
        if total > visible {
            format!(
                " Tailscale Netcheck (line {}/{}) ",
                state.net_scroll + 1,
                total,
            )
        } else {
            " Tailscale Netcheck ".to_string()
        }
    } else {
        " Tailscale Netcheck [Tab to expand] ".to_string()
    };
    let ts_block = Block::default()
        .title(ts_title)
        .borders(Borders::ALL)
        .border_style(ts_border_style);

    if ts_active {
        let lines = tailscale_lines(state);
        let paragraph = Paragraph::new(lines)
            .block(ts_block)
            .scroll((state.net_scroll, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, chunks[2]);
    } else {
        frame.render_widget(ts_block, chunks[2]);
    }
}

// r[impl installer.tui.network-check+2]
fn render_network_check(frame: &mut Frame, area: Rect, state: &AppState) {
    render_net_accordion(
        frame,
        area,
        state,
        "Checking network connectivity to endpoints needed for deployment.",
    );
}

// r[impl installer.tui.network-check+2]
fn render_network_results(frame: &mut Frame, area: Rect, state: &AppState) {
    render_net_accordion(
        frame,
        area,
        state,
        "Network check results (ran in the background during installation).",
    );
}

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let hints = match &state.screen {
        Screen::Welcome => "Enter: start | n: network check | q: quit",
        Screen::NetworkCheck => {
            "Tab: switch pane | Up/Down: scroll | r: re-run | Esc: back | q: quit"
        }
        Screen::DiskSelection => "Up/Down: select | Enter: next | Esc: back | q: quit",
        Screen::VariantSelection => "Up/Down: select | Enter: next | Esc: back | q: quit",
        Screen::TpmToggle => "Space: toggle | Enter: next | Esc: back | q: quit",
        Screen::Hostname => "Enter: next | Esc: back",
        Screen::Tailscale => "Enter: next | Esc: back",
        Screen::SshKeys => {
            if state.ssh_github_focus {
                "Enter: fetch keys | Tab: next screen | Esc: back to keys"
            } else {
                "Tab: GitHub lookup | Enter: new line | Esc: back"
            }
        }
        Screen::Password => {
            if state.password_confirming {
                "Enter: confirm | Esc: back to password"
            } else {
                "Enter/Tab: next | Esc: back"
            }
        }
        Screen::Timezone => "Type to search | Up/Down: navigate | Enter: select | Esc: back",
        Screen::NetworkResults => {
            "Tab: switch pane | Up/Down: scroll | Enter: next | r: re-run | Esc: back | q: quit"
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

// r[impl installer.tui.welcome+3]
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
    description.push(Line::from(
        "  Press Enter to begin, or 'n' for a network check.",
    ));

    let paragraph = Paragraph::new(description).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.disk-detection+3]
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
fn render_hostname(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_metal = state.variant == crate::config::Variant::Metal;
    let dhcp_active = state.hostname_from_dhcp;

    let hint = if dhcp_active {
        "  The system will get its hostname from DHCP."
    } else if is_metal {
        "  A hostname is required for the metal variant."
    } else {
        "  Leave empty to skip (default: ubuntu, overridden by DHCP/cloud-init)."
    };

    let hostname_style = if dhcp_active {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };

    let mut lines = vec![
        Line::from(""),
        Line::from("  Enter the hostname for this system."),
        Line::from(hint),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Hostname: "),
            Span::styled(&state.hostname_input, hostname_style),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    if is_metal {
        lines.push(Line::from(""));
        let toggle_marker = if dhcp_active { "x" } else { " " };
        lines.push(Line::from(vec![
            Span::raw(format!("  [{toggle_marker}] ")),
            Span::styled(
                "Use DHCP hostname (no static hostname)",
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            "      Tab to switch focus, Space to toggle",
            Style::default().fg(Color::DarkGray),
        )));
        if dhcp_active {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  The static hostname will be empty (shown as n/a by hostnamectl).",
                Style::default().fg(Color::Cyan),
            )));
        }
    }

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
    let github_height: u16 = if state.ssh_github_focus { 5 } else { 2 };
    let chunks = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(github_height),
    ])
    .split(area);

    let focus_hint = if state.ssh_github_focus {
        "GitHub username input active. Tab to switch back to keys."
    } else {
        "Paste SSH public keys (one per line). Tab to switch to GitHub lookup."
    };
    let intro = vec![
        Line::from(""),
        Line::from(format!("  {focus_hint}")),
        Line::from("  Leave empty to skip."),
        Line::from(""),
    ];
    let intro_paragraph = Paragraph::new(intro);
    frame.render_widget(intro_paragraph, chunks[0]);

    let keys_border_style = if state.ssh_github_focus {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    let key_lines: Vec<Line> = if state.ssh_keys_input.is_empty() {
        vec![Line::from(Span::styled(
            if state.ssh_github_focus { " " } else { "_" },
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
        if !state.ssh_github_focus {
            if state.ssh_keys_input.ends_with('\n') {
                lines.push(Line::from(Span::styled(
                    "_",
                    Style::default().fg(Color::DarkGray),
                )));
            } else if let Some(last) = lines.last_mut() {
                last.spans
                    .push(Span::styled("_", Style::default().fg(Color::DarkGray)));
            }
        }
        lines
    };

    let block = Block::default()
        .title(" SSH Authorized Keys ")
        .borders(Borders::ALL)
        .border_style(keys_border_style);

    let paragraph = Paragraph::new(key_lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[1]);

    // r[impl installer.tui.ssh-keys.github]
    let github_border_style = if state.ssh_github_focus {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut github_lines: Vec<Line> = vec![Line::from(vec![
        Span::raw("  GitHub user: "),
        Span::styled(
            &state.ssh_github_input,
            if state.ssh_github_focus {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
        if state.ssh_github_focus {
            Span::styled("_", Style::default().fg(Color::DarkGray))
        } else {
            Span::raw("")
        },
    ])];

    if state.ssh_github_fetching {
        github_lines.push(Line::from(Span::styled(
            "  Fetching keys...",
            Style::default().fg(Color::DarkGray),
        )));
    } else if let Some(ref err) = state.ssh_github_error {
        github_lines.push(Line::from(Span::styled(
            format!("  Error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    let github_block = Block::default()
        .title(" Import from GitHub ")
        .borders(Borders::ALL)
        .border_style(github_border_style);

    let github_paragraph = Paragraph::new(github_lines).block(github_block);
    frame.render_widget(github_paragraph, chunks[2]);
}

// r[impl installer.tui.confirmation+2]
// r[impl installer.tui.password]
// r[impl installer.tui.timezone]
fn render_timezone(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::vertical([Constraint::Length(5), Constraint::Min(0)]).split(area);

    let intro = vec![
        Line::from(""),
        Line::from("  Select the system timezone."),
        Line::from(vec![
            Span::raw("  Search: "),
            Span::styled(
                &state.timezone_search,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
            Span::raw(format!(
                "  ({} match{})",
                state.timezone_filtered.len(),
                if state.timezone_filtered.len() == 1 {
                    ""
                } else {
                    "es"
                }
            )),
        ]),
        Line::from(""),
    ];
    let intro_paragraph = Paragraph::new(intro);
    frame.render_widget(intro_paragraph, chunks[0]);

    let visible_height = chunks[1].height.saturating_sub(2) as usize;
    let cursor = state.timezone_cursor;
    let scroll_offset = if visible_height == 0 {
        0
    } else if cursor >= visible_height {
        cursor - visible_height + 1
    } else {
        0
    };

    let items: Vec<ListItem> = state
        .timezone_filtered
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height.max(1))
        .map(|(i, &tz_idx)| {
            let tz = &state.available_timezones[tz_idx];
            let style = if i == cursor {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!("  {tz}")).style(style)
        })
        .collect();

    let block = Block::default()
        .title(format!(" Timezone [{}] ", state.timezone_selected))
        .borders(Borders::ALL);

    let list = List::new(items).block(block);
    frame.render_widget(list, chunks[1]);
}

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
        Line::from(vec![
            Span::raw("  Timezone:     "),
            Span::styled(
                state.effective_timezone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
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
