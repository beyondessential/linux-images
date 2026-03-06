use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Padding, Paragraph, Wrap};

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
        Screen::DiskEncryption => render_disk_encryption(frame, chunks[1], state),
        Screen::Hostname => render_hostname(frame, chunks[1], state),
        Screen::HostnameInput => render_hostname_input(frame, chunks[1], state),
        Screen::Login => render_login(frame, chunks[1], state),
        Screen::LoginTailscale => render_login_tailscale(frame, chunks[1], state),
        Screen::LoginSshKeys => render_login_ssh_keys(frame, chunks[1], state),
        Screen::LoginGithub => render_login_github(frame, chunks[1], state),
        Screen::Timezone => render_timezone(frame, chunks[1], state),
        Screen::Confirmation => render_confirmation(frame, chunks[1], state),
        Screen::Installing => render_installing(frame, chunks[1], state),
        Screen::Done => render_done(frame, chunks[1], state),
        Screen::Error(msg) => render_error(frame, chunks[1], msg),
    }

    render_footer(frame, chunks[2], state);
}

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    let step = match &state.screen {
        Screen::Welcome => "Welcome",
        Screen::NetworkCheck => "Network Check",
        Screen::DiskSelection => "1/6 Select Target Disk",
        Screen::DiskEncryption => "2/6 Disk Encryption",
        Screen::Hostname => "3/6 Hostname",
        Screen::HostnameInput => "3/6 Hostname",
        Screen::Login => "4/6 Login",
        Screen::LoginTailscale => "4/6 Login > Tailscale",
        Screen::LoginSshKeys => "4/6 Login > SSH Keys",
        Screen::LoginGithub => "4/6 Login > GitHub",
        Screen::Timezone => "5/6 Timezone",
        Screen::NetworkResults => "Network Results",
        Screen::Confirmation => "6/6 Confirm",
        Screen::Installing => "Installing",
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
fn connectivity_status(state: &AppState) -> String {
    match state.net_check_phase {
        CheckPhase::NotStarted => "Not started".into(),
        CheckPhase::Running => "Running...".into(),
        CheckPhase::Done => {
            let passed = state
                .net_check_results
                .iter()
                .filter(|r| matches!(r, Some(r) if r.passed))
                .count();
            let total = state.net_check_total;
            if passed == total {
                "All passed".into()
            } else {
                format!("{passed}/{total} passed")
            }
        }
    }
}

fn tailscale_status(state: &AppState) -> &'static str {
    match state.netcheck_phase {
        CheckPhase::NotStarted => "Not started",
        CheckPhase::Running => "Running...",
        CheckPhase::Done => match &state.netcheck_result {
            Some(r) if r.success => "OK",
            Some(_) => "Failed",
            None => "Done",
        },
    }
}

// r[impl installer.tui.network-check+4]
fn render_net_accordion(frame: &mut Frame, area: Rect, state: &AppState, intro_text: &str) {
    let intro = vec![
        Line::from(""),
        Line::from(format!("  {intro_text}")),
        Line::from(""),
    ];

    // Collapsed pane: just a bordered title bar (height 3).
    // Expanded pane: takes remaining space.
    let (conn_constraint, ts_constraint) = match state.net_pane {
        NetPane::Connectivity => (Constraint::Min(0), Constraint::Length(3)),
        NetPane::Tailscale => (Constraint::Length(3), Constraint::Min(0)),
    };

    let chunks =
        Layout::vertical([Constraint::Length(4), conn_constraint, ts_constraint]).split(area);

    let intro_paragraph = Paragraph::new(intro);
    frame.render_widget(intro_paragraph, chunks[0]);

    // --- Connectivity pane ---
    let conn_active = state.net_pane == NetPane::Connectivity;
    let conn_status = connectivity_status(state);
    let conn_title = if conn_active {
        let total = state.net_check_line_count();
        let visible = chunks[1].height.saturating_sub(2) as usize;
        if total > visible {
            format!(
                " Connectivity -- {conn_status} (line {}/{}) ",
                state.net_scroll + 1,
                total,
            )
        } else {
            format!(" Connectivity -- {conn_status} ")
        }
    } else {
        format!(" Connectivity -- {conn_status} [Tab to expand] ")
    };
    let conn_block = Block::default().title(conn_title).borders(Borders::ALL);

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
    let ts_status = tailscale_status(state);
    let ts_title = if ts_active {
        let total = state.netcheck_line_count();
        let visible = chunks[2].height.saturating_sub(2) as usize;
        if total > visible {
            format!(
                " Tailscale Netcheck -- {ts_status} (line {}/{}) ",
                state.net_scroll + 1,
                total,
            )
        } else {
            format!(" Tailscale Netcheck -- {ts_status} ")
        }
    } else {
        format!(" Tailscale Netcheck -- {ts_status} [Tab to expand] ")
    };
    let ts_block = Block::default().title(ts_title).borders(Borders::ALL);

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

// r[impl installer.tui.network-check+4]
fn render_network_check(frame: &mut Frame, area: Rect, state: &AppState) {
    render_net_accordion(
        frame,
        area,
        state,
        "Checking network connectivity to endpoints needed for deployment.",
    );
}

// r[impl installer.tui.network-check+4]
fn render_network_results(frame: &mut Frame, area: Rect, state: &AppState) {
    render_net_accordion(frame, area, state, "Network check results");
}

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let hints: String = match &state.screen {
        Screen::Welcome => "Enter: start | n: network check | q: quit".into(),
        Screen::NetworkCheck => {
            "Tab: switch pane | Up/Down: scroll | r: re-run | Esc: back | q: quit".into()
        }
        Screen::DiskSelection => "Up/Down: select | Enter: next | Esc: back | q: quit".into(),
        Screen::DiskEncryption => "Up/Down: select | Enter: next | Esc: back | q: quit".into(),
        Screen::Hostname => "Up/Down: select | Enter: next | Esc: back".into(),
        Screen::HostnameInput => "Enter: next | Esc: back".into(),
        Screen::Login => {
            let mut h = String::from("Alt+t: tailscale | Alt+s: ssh keys");
            if state.github_reachable() {
                h.push_str(" | Alt+g: github");
            }
            if state.password_confirming {
                h.push_str(" | Enter: confirm | Esc: back to password");
            } else {
                h.push_str(" | Enter: next | Esc: back");
            }
            h
        }
        Screen::LoginTailscale => "Enter: done | Esc: back".into(),
        Screen::LoginSshKeys => "Tab: next | Shift+Tab: prev | Enter: done | Esc: back".into(),
        Screen::LoginGithub => "Enter: fetch keys | Esc: back".into(),
        Screen::Timezone => "Type to search | Up/Down: navigate | Enter: select | Esc: back".into(),
        Screen::NetworkResults => {
            "Tab: switch pane | Up/Down: scroll | Enter: next | r: re-run | Esc: back | q: quit"
                .into()
        }
        Screen::Confirmation => "Type 'yes' to confirm | Esc: back | q: quit".into(),
        Screen::Installing => "Please wait...".into(),
        Screen::Done => "Press Enter to reboot".into(),
        Screen::Error(_) => "Press any key to reboot".into(),
    };
    let paragraph = Paragraph::new(hints);
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.welcome+3]
// r[impl installer.tui.ascii-rendering]
fn render_welcome(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut description = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Tamanu Linux",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(
            String::from("Tamanu Linux is BES's preferred system layout for Linux deployments.")
                + " It is based on Ubuntu Server. If you're not installing a Tamanu or"
                + " other BES system, you may want to use the normal Ubuntu Server ISO.",
        ),
        Line::from(""),
        Line::from(
            String::from("If you want to automate installs, and this is booting from a USB,")
                + " plug the USB drive into a computer and open the BESCONF volume."
                + " Within, you will find a bes-install.toml text file that you can use"
                + " to configure an automated install. You can also image disks directly"
                + " using our disk images, which may be more suitable for bulk installs.",
        ),
        Line::from(""),
        Span::styled("For support, contact BES at: ", Style::default())
            + Span::styled(
                "https://bes.au",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        Line::from(""),
        Line::from("Sources for this installer, and other images, are available at:"),
        Line::from(Span::styled(
            "https://github.com/beyondessential/linux-images",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),
        )),
    ];

    if !state.build_info.is_empty() {
        description.push(Line::from(""));
        description.push(Line::from(Span::styled(
            state.build_info.to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    }

    description.push(Line::from(""));
    description.push(Line::from(
        "Press Enter to begin, or 'n' for a network check.",
    ));

    frame.render_widget(
        Paragraph::new(description)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .padding(Padding::uniform(1))
                    .border_style(Style::default().fg(Color::White)),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
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

// r[impl installer.tui.disk-encryption+2]
fn render_disk_encryption(frame: &mut Frame, area: Rect, state: &AppState) {
    use crate::config::DiskEncryption;

    let options: Vec<(DiskEncryption, &str)> = if state.tpm_present {
        vec![
            (
                DiskEncryption::Tpm,
                "Full-disk encryption, bound to hardware",
            ),
            (
                DiskEncryption::Keyfile,
                "Full-disk encryption, not bound to hardware",
            ),
            (DiskEncryption::None, "No encryption"),
        ]
    } else {
        vec![
            (
                DiskEncryption::Keyfile,
                "Full-disk encryption, not bound to hardware",
            ),
            (DiskEncryption::None, "No encryption"),
        ]
    };

    let items: Vec<ListItem> = options
        .iter()
        .map(|(enc, label)| {
            let is_selected = *enc == state.disk_encryption;
            let marker = if is_selected { ">" } else { " " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("  {marker} {label}")).style(style)
        })
        .collect();

    let explanation = match state.disk_encryption {
        DiskEncryption::Tpm => vec![
            Line::from(""),
            Line::from("  The disk's encryption key will be sealed to this machine's TPM"),
            Line::from("  using PCR 1 (hardware identity: motherboard, CPU, and RAM"),
            Line::from("  model/serials). The system will boot unattended as long as the"),
            Line::from("  hardware stays the same. If you move the disk to different"),
            Line::from("  hardware, you will need the recovery passphrase. Changing the"),
            Line::from("  CPU or RAM may also require the recovery passphrase."),
        ],
        DiskEncryption::Keyfile => vec![
            Line::from(""),
            Line::from("  A keyfile will be stored on the boot partition. The system will"),
            Line::from("  boot unattended on any hardware. If the boot partition is lost,"),
            Line::from("  you will need the recovery passphrase."),
        ],
        DiskEncryption::None => vec![
            Line::from(""),
            Line::from("  The root partition will not be encrypted."),
        ],
    };

    let block = Block::default()
        .title(" Disk Encryption ")
        .borders(Borders::ALL);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([Constraint::Length(options.len() as u16), Constraint::Min(0)])
        .split(inner);

    let list = List::new(items);
    frame.render_widget(list, chunks[0]);

    let paragraph = Paragraph::new(explanation);
    frame.render_widget(paragraph, chunks[1]);
}

// r[impl installer.tui.hostname+5]
fn render_hostname(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_encrypted = state.disk_encryption.is_encrypted();

    let network_label = if is_encrypted {
        "Network-assigned (DHCP)"
    } else {
        "Network-assigned (DHCP / cloud-init)"
    };

    let options = ["Static hostname", network_label];
    let selected_index = if state.hostname_from_dhcp { 1 } else { 0 };

    let mut lines = vec![
        Line::from(""),
        Line::from("  How should this system get its hostname?"),
        Line::from(""),
    ];

    for (i, label) in options.iter().enumerate() {
        let is_sel = i == selected_index;
        let marker = if is_sel { ">" } else { " " };
        let style = if is_sel {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("  {marker} {label}"),
            style,
        )));
    }

    let block = Block::default().title(" Hostname ").borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.hostname+5]
fn render_hostname_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = vec![
        Line::from(""),
        Line::from("  Enter the hostname for this system."),
    ];

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  Hostname: "),
        Span::styled(
            &state.hostname_input,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("_", Style::default().fg(Color::DarkGray)),
    ]));

    if let Some(ref err) = state.hostname_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    let block = Block::default().title(" Hostname ").borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn mask(input: &str) -> String {
    "*".repeat(input.len())
}

// r[impl installer.tui.password+4]
// r[impl installer.tui.tailscale+3]
// r[impl installer.tui.ssh-keys+5]
fn render_login(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = vec![
        Line::from(""),
        Line::from("  Set a password for the 'ubuntu' user."),
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
    } else if state.password_empty {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Password is required.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("  Actions:"));

    let ts_indicator = if !state.tailscale_input.trim().is_empty() {
        Span::styled(" *", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };
    lines.push(Line::from(vec![
        Span::raw("    Alt+t  Tailscale auth key"),
        ts_indicator,
    ]));

    let ssh_indicator = if state.ssh_keys.iter().any(|k| !k.trim().is_empty()) {
        Span::styled(" *", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };
    lines.push(Line::from(vec![
        Span::raw("    Alt+s  SSH authorized keys"),
        ssh_indicator,
    ]));

    if state.github_reachable() {
        lines.push(Line::from("    Alt+g  Import SSH keys from GitHub"));
    }

    let block = Block::default().title(" Login ").borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.tailscale+3]
fn render_login_tailscale(frame: &mut Frame, area: Rect, state: &AppState) {
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

    let block = Block::default()
        .title(" Login > Tailscale ")
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.ssh-keys+5]
fn render_login_ssh_keys(frame: &mut Frame, area: Rect, state: &AppState) {
    let intro_lines = vec![
        Line::from(""),
        Line::from(
            "  SSH authorized keys. Tab/Shift+Tab to navigate. Type in the blank field to add a key.",
        ),
        Line::from("  Leave empty to skip."),
        Line::from(""),
    ];

    let chunks = Layout::vertical([Constraint::Length(5), Constraint::Min(0)]).split(area);

    let intro_paragraph = Paragraph::new(intro_lines);
    frame.render_widget(intro_paragraph, chunks[0]);

    let mut key_lines: Vec<Line> = Vec::new();
    for (i, key) in state.ssh_keys.iter().enumerate() {
        let is_selected = i == state.ssh_key_cursor;
        let is_empty = key.trim().is_empty();
        let is_valid = is_empty || AppState::is_valid_ssh_key(key);
        if is_selected {
            let text_color = if is_empty || is_valid {
                Color::Yellow
            } else {
                Color::Red
            };
            key_lines.push(Line::from(vec![
                Span::styled(
                    format!("  > {}: ", i + 1),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    key.as_str(),
                    Style::default().fg(text_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled("_", Style::default().fg(Color::DarkGray)),
            ]));
        } else {
            let summary = AppState::ssh_key_summary(key);
            let style = if is_empty {
                Style::default().fg(Color::DarkGray)
            } else if is_valid {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Red)
            };
            key_lines.push(Line::from(vec![
                Span::raw(format!("    {}: ", i + 1)),
                Span::styled(summary, style),
            ]));
        }
    }

    let block = Block::default()
        .title(" SSH Authorized Keys ")
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(key_lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[1]);
}

// r[impl installer.tui.ssh-keys.github+4]
fn render_login_github(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = vec![
        Line::from(""),
        Line::from("  Import SSH keys from a GitHub account."),
        Line::from("  Enter a GitHub username and press Enter to fetch their public keys."),
        Line::from(""),
        Line::from(vec![
            Span::raw("  GitHub user: "),
            Span::styled(
                &state.ssh_github_input,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    if state.ssh_github_fetching {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Fetching keys...",
            Style::default().fg(Color::DarkGray),
        )));
    } else if let Some(ref err) = state.ssh_github_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  Error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    let block = Block::default()
        .title(" Login > GitHub ")
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

// r[impl installer.tui.confirmation+7]
// r[impl installer.tui.password+4]
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

// r[impl installer.tui.confirmation+7]
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

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Target disk:      "),
            Span::styled(&disk_desc, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw("  Disk encryption:  "),
            Span::styled(
                state.disk_encryption.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Timezone:     "),
            Span::styled(
                state.effective_timezone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    if let Some(cfg) = state.install_config_fields() {
        if let Some(ref h) = cfg.hostname {
            lines.push(Line::from(vec![
                Span::raw("  Hostname:     "),
                Span::styled(h.to_string(), Style::default().add_modifier(Modifier::BOLD)),
            ]));
        }
        if cfg.tailscale_authkey.is_some() {
            lines.push(Line::from(vec![
                Span::raw("  Tailscale:    "),
                Span::styled(
                    "auth key provided",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        if !cfg.ssh_authorized_keys.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("  SSH keys:     "),
                Span::styled(
                    format!("{} key(s)", cfg.ssh_authorized_keys.len()),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        if cfg.password.is_some() || cfg.password_hash.is_some() {
            lines.push(Line::from(vec![
                Span::raw("  Password:     "),
                Span::styled(
                    "custom password set",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    // r[impl installer.encryption.recovery-passphrase+3]
    if let Some(ref passphrase) = state.recovery_passphrase {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Recovery Passphrase",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("    {passphrase}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(
            "  Write this down and store it in a safe place BEFORE proceeding.",
        ));
        lines.push(Line::from(
            "  You will need it if the primary unlock mechanism fails.",
        ));
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

// r[impl installer.tui.progress+3]
fn render_installing(frame: &mut Frame, area: Rect, state: &AppState) {
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
            Line::from("  Installing to disk..."),
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
        let paragraph = Paragraph::new("  Preparing to install...");
        frame.render_widget(paragraph, area);
    }
}

// r[impl installer.tui.progress+3]
fn render_done(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Installation complete!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // r[impl installer.encryption.recovery-passphrase+3]
    if let Some(ref passphrase) = state.recovery_passphrase {
        lines.push(Line::from(Span::styled(
            "  Recovery Passphrase",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(
            "  Write down this recovery passphrase and store it in a safe place.",
        ));
        lines.push(Line::from(
            "  You will need it if the primary unlock mechanism fails.",
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("    {passphrase}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(
        "  Remove the installation media and press Enter to reboot.",
    ));

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
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
        Line::from("  Press any key to reboot."),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::config::{DiskEncryption, InstallConfig};
    use crate::disk::TransportType;

    fn make_test_state() -> AppState {
        let devices = vec![BlockDevice {
            path: PathBuf::from("/dev/sda"),
            size_bytes: 500_000_000_000,
            model: "Test SSD".into(),
            transport: TransportType::Nvme,
            removable: false,
        }];
        AppState::new(
            devices,
            DiskEncryption::None,
            false,
            &InstallConfig::default(),
            None,
            None,
            String::new(),
            vec!["UTC".into(), "America/New_York".into()],
        )
    }

    fn is_ratatui_border_char(ch: char) -> bool {
        // Box-drawing characters used by ratatui's Borders widget and Gauge.
        // The Linux console supports these via the DEC Special Graphics set,
        // so they do not render as replacement blocks. The spec only forbids
        // non-ASCII *text* (em dashes, curly quotes, ellipsis, etc.).
        matches!(
            ch,
            '─' | '│'
                | '┌'
                | '┐'
                | '└'
                | '┘'
                | '┤'
                | '├'
                | '┬'
                | '┴'
                | '┼'
                | '╔'
                | '╗'
                | '╚'
                | '╝'
                | '║'
                | '═'
                | '╠'
                | '╣'
                | '╦'
                | '╩'
                | '╬'
                | '╭'
                | '╮'
                | '╯'
                | '╰'
                | '▕'
                | '█'
                | '░'
                | '▒'
                | '▓'
                | '▏'
                | '▎'
                | '▍'
                | '▌'
                | '▋'
                | '▊'
                | '▉'
        )
    }

    fn assert_buffer_ascii(terminal: &Terminal<TestBackend>, screen_name: &str) {
        let buf = terminal.backend().buffer();
        for (i, cell) in buf.content().iter().enumerate() {
            let symbol = cell.symbol();
            for ch in symbol.chars() {
                assert!(
                    ch.is_ascii() || is_ratatui_border_char(ch),
                    "non-ASCII character U+{:04X} ({:?}) found in {screen_name} screen at buffer index {i}",
                    ch as u32,
                    ch,
                );
            }
        }
    }

    fn render_screen(state: &AppState) -> Terminal<TestBackend> {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, state)).unwrap();
        terminal
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn welcome_screen_ascii_only() {
        let state = make_test_state();
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Welcome");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn disk_selection_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::DiskSelection;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "DiskSelection");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn disk_encryption_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::DiskEncryption;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "DiskEncryption");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn hostname_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Hostname;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Hostname");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn hostname_input_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::HostnameInput;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "HostnameInput");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn login_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Login;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Login");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn login_tailscale_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::LoginTailscale;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "LoginTailscale");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn login_ssh_keys_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::LoginSshKeys;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "LoginSshKeys");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn login_github_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::LoginGithub;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "LoginGithub");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn timezone_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Timezone;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Timezone");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn confirmation_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Confirmation;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Confirmation");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn installing_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Installing;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Installing");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn done_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Done;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Done");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn done_screen_with_recovery_passphrase_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Done;
        state.recovery_passphrase = Some("test-recovery-phrase".into());
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Done(with recovery passphrase)");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn error_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Error("disk write failed: I/O error".into());
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Error");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn network_check_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::NetworkCheck;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "NetworkCheck");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn network_results_screen_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::NetworkResults;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "NetworkResults");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn tpm_encryption_explanation_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::DiskEncryption;
        state.tpm_present = true;
        state.disk_encryption = DiskEncryption::Tpm;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "DiskEncryption(Tpm)");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn keyfile_encryption_explanation_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::DiskEncryption;
        state.disk_encryption = DiskEncryption::Keyfile;
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "DiskEncryption(Keyfile)");
    }

    // r[verify installer.tui.ascii-rendering]
    #[test]
    fn confirmation_with_recovery_passphrase_ascii_only() {
        let mut state = make_test_state();
        state.screen = Screen::Confirmation;
        state.disk_encryption = DiskEncryption::Tpm;
        state.recovery_passphrase = Some("alpha-bravo-charlie-delta".into());
        let terminal = render_screen(&state);
        assert_buffer_ascii(&terminal, "Confirmation(with recovery passphrase)");
    }
}
