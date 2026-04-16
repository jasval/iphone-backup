use super::cat;
use super::{App, RestoreFlow, Tab};
use chrono::{DateTime, Utc};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Frame,
};

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ── Top-level layout ──────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let use_header = area.height >= 18;
    let header_h = if use_header { 7 } else { 1 };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h), // header (cat + status) or compact title
            Constraint::Length(1),        // tab bar
            Constraint::Min(0),           // content
            Constraint::Length(1),        // footer
        ])
        .split(area);

    if use_header {
        render_header(f, app, outer[0]);
    } else {
        render_title(f, app, outer[0]);
    }
    render_tabs(f, app, outer[1]);
    match app.tab {
        Tab::Dashboard => render_dashboard(f, app, outer[2]),
        Tab::Restore => render_restore(f, app, outer[2]),
        Tab::Services => render_services(f, app, outer[2]),
    }
    render_footer(f, app, outer[3]);
}

// ── Title bar ─────────────────────────────────────────────────────────────────

fn render_title(f: &mut Frame, app: &App, area: Rect) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let storage = if app.storage_ok {
        Span::styled("  ● storage", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            "  ○ storage not found",
            Style::default().fg(Color::DarkGray),
        )
    };
    let running = if app.backup_running || app.active_job.is_some() {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        let label = if let Some(job) = &app.active_job {
            let progress = app.backup_progress.as_deref().unwrap_or("running");
            format!("  {} {} since {} — {}", s, job.job_id, job.start_time, progress)
        } else {
            match &app.backup_progress {
                Some(p) => format!("  {} backing up: {}", s, p),
                None => format!("  {} backing up...", s),
            }
        };
        let color = if app.active_job_is_daemon {
            Color::Magenta
        } else {
            Color::Yellow
        };
        Span::styled(label, Style::default().fg(color))
    } else if app.pairing_running {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        Span::styled(
            format!("  {} pairing...", s),
            Style::default().fg(Color::Cyan),
        )
    } else if app.update_running {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        Span::styled(
            format!("  {} updating...", s),
            Style::default().fg(Color::Magenta),
        )
    } else {
        Span::raw("")
    };
    let title = Line::from(vec![
        Span::styled(
            " iphone-backup",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        storage,
        running,
        Span::styled(format!("  {now} "), Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(
        Paragraph::new(title).style(Style::default().bg(Color::Black)),
        area,
    );
}

// ── Header (cat + status info) ────────────────────────────────────────────────

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(area);

    render_cat(f, app, cols[0]);
    render_status_info(f, app, cols[1]);
}

fn render_cat(f: &mut Frame, app: &App, area: Rect) {
    let state = cat::cat_state(app);
    let frame = cat::current_frame(state, app.spinner_frame);
    let (label, color) = cat::status_label(state);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(label, Style::default().fg(color)));

    let lines: Vec<Line> = frame.iter().map(|&s| Line::from(s)).collect();

    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .alignment(Alignment::Center),
        area,
    );
}

fn render_status_info(f: &mut Frame, app: &App, area: Rect) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    let storage = if app.storage_ok {
        Line::from(Span::styled(
            "  ● storage",
            Style::default().fg(Color::Green),
        ))
    } else {
        Line::from(Span::styled(
            "  ○ storage not found",
            Style::default().fg(Color::DarkGray),
        ))
    };

    let running = if app.backup_running || app.active_job.is_some() {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        let label = if let Some(job) = &app.active_job {
            let progress = app.backup_progress.as_deref().unwrap_or("running");
            format!(
                "  {} {} since {} — {}",
                s, job.job_id, job.start_time, progress
            )
        } else {
            match &app.backup_progress {
                Some(p) => format!("  {} backing up: {}", s, p),
                None => format!("  {} backing up...", s),
            }
        };
        let color = if app.active_job_is_daemon {
            Color::Magenta
        } else {
            Color::Yellow
        };
        Line::from(Span::styled(label, Style::default().fg(color)))
    } else if app.pairing_running {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        Line::from(Span::styled(
            format!("  {} pairing...", s),
            Style::default().fg(Color::Cyan),
        ))
    } else if app.update_running {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        Line::from(Span::styled(
            format!("  {} updating...", s),
            Style::default().fg(Color::Magenta),
        ))
    } else {
        Line::from(Span::styled(
            "  idle",
            Style::default().fg(Color::DarkGray),
        ))
    };

    let timestamp = Line::from(Span::styled(
        format!("  {}", now),
        Style::default().fg(Color::DarkGray),
    ));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " iphone-backup ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    if let Some(pct) = app.backup_progress_pct {
        // Show a progress gauge at the bottom of the status info block.
        let inner = block.inner(area);
        f.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

        let lines = vec![Line::raw(""), storage, running, timestamp];
        f.render_widget(Paragraph::new(Text::from(lines)), rows[0]);

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
            .percent(pct.min(100))
            .label(format!("  {}%", pct.min(100)));
        f.render_widget(gauge, rows[1]);
    } else {
        let lines = vec![Line::raw(""), storage, running, timestamp, Line::raw("")];
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }
}

// ── Tab bar ───────────────────────────────────────────────────────────────────

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let tabs: &[(&str, Tab)] = &[
        ("[1] Dashboard", Tab::Dashboard),
        ("[2] Restore", Tab::Restore),
        ("[3] Services", Tab::Services),
    ];
    let spans: Vec<Span> = tabs
        .iter()
        .flat_map(|(label, t)| {
            let style = if *t == app.tab {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            vec![Span::styled(format!(" {} ", label), style), Span::raw("  ")]
        })
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Black)),
        area,
    );
}

// ── Footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let flash_style = Style::default().fg(Color::Yellow);
    let hint_style = Style::default().fg(Color::DarkGray);

    let text = match &app.tab {
        Tab::Dashboard => {
            if let Some(msg) = &app.flash {
                Span::styled(format!(" {}", msg), flash_style)
            } else if app.backup_running || app.active_job.is_some() {
                let source = if app.active_job_is_daemon { " (daemon)" } else { "" };
                Span::styled(
                    format!(" Running{source}...  [X] cancel  [Tab] switch tab  [q] quit"),
                    hint_style,
                )
            } else if app.pairing_running {
                Span::styled(" Running...  [Tab] switch tab  [q] quit", hint_style)
            } else {
                Span::styled(
                    " [r] backup  [p] pair  [↑↓] select  [PgUp/PgDn] scroll  [Tab] tab  [q] quit",
                    hint_style,
                )
            }
        }
        Tab::Restore => match &app.restore_flow {
            RestoreFlow::SelectBackup => Span::styled(
                " [↑↓] select  [Enter] restore  [D] delete  [R] refresh  [Tab] tab  [q] quit",
                hint_style,
            ),
            RestoreFlow::SelectDevice { .. } => Span::styled(
                " [↑↓] select device  [Enter] next  [Esc] back  [R] refresh  [Tab] tab",
                hint_style,
            ),
            RestoreFlow::Confirm { .. } => {
                Span::styled(" [Enter] start restore  [Esc] back", hint_style)
            }
            RestoreFlow::ConfirmDelete { .. } => {
                Span::styled(" [Enter] confirm delete  [Esc] cancel", hint_style)
            }
            RestoreFlow::Running => {
                Span::styled(" Restore running...  [PgUp/PgDn] scroll", hint_style)
            }
            RestoreFlow::Done(_) => Span::styled(" [Esc] back to backup list", hint_style),
        },
        Tab::Services => {
            if app.editing_path {
                Span::styled(" Type path  [Enter] confirm  [Esc] cancel", hint_style)
            } else if app.editing_schedule {
                Span::styled(" Type HH:MM  [Enter] confirm  [Esc] cancel", hint_style)
            } else if let Some(msg) = &app.services_flash {
                Span::styled(format!(" {}", msg), flash_style)
            } else {
                Span::styled(
                    " [i] install  [l] load  [u] unload  [s] start  [p] pair  [e] path  [c] schedule  [U] update  [Tab] tab  [q] quit",
                    hint_style,
                )
            }
        }
    };
    f.render_widget(Paragraph::new(Line::from(vec![text])), area);
}

// ── Dashboard ─────────────────────────────────────────────────────────────────

fn render_dashboard(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);
    render_devices(f, app, cols[0]);
    render_logs(f, app, cols[1]);
}

fn render_devices(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled("Devices", Style::default().fg(Color::Cyan)));

    if app.devices.is_empty() {
        let msg = if !app.storage_ok {
            "Backup path not accessible.\n\nSet backup_path in:\n  iphone-backup config\n\nDefault: ~/Backups/iOS"
        } else {
            "No backups yet.\n\nPress [r] to run a backup."
        };
        f.render_widget(
            Paragraph::new(msg)
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let selected = i == app.selected;
            let (sym, status_color) = match d.status.as_str() {
                "success" => ("✓", Color::Green),
                "failed" => ("✗", Color::Red),
                _ => ("?", Color::DarkGray),
            };
            let name_style = if selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let arrow = if selected { "▶ " } else { "  " };
            let ios = d.ios.as_deref().unwrap_or("?");
            let size = d.size.as_deref().unwrap_or("?");
            let age = time_ago(&d.last_run);
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled(arrow, Style::default().fg(Color::Cyan)),
                    Span::styled(d.name.replace('_', " "), name_style),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled(
                        format!("iOS {}  ·  {}", ios, size),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled(format!("{:<16}", age), Style::default().fg(Color::Gray)),
                    Span::styled(
                        format!("{} {}", sym, d.status),
                        Style::default().fg(status_color),
                    ),
                ]),
                Line::raw(""),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn render_logs(f: &mut Frame, app: &App, area: Rect) {
    let hint = if app.auto_scroll {
        " ↓ live"
    } else {
        " [G] jump to end"
    };
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        format!("Logs{}", hint),
        Style::default().fg(Color::Cyan),
    ));

    let inner_h = area.height.saturating_sub(2) as usize;
    let total = app.logs.len();
    let bottom = app.log_scroll.min(total.saturating_sub(1));
    let top = bottom.saturating_sub(inner_h.saturating_sub(1));
    let visible = if total > 0 {
        &app.logs[top..=bottom.min(total - 1)]
    } else {
        &[][..]
    };

    let lines: Vec<Line> = visible.iter().map(|l| log_line_color(l)).collect();
    f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

// ── Restore ───────────────────────────────────────────────────────────────────

fn render_restore(f: &mut Frame, app: &App, area: Rect) {
    match &app.restore_flow {
        RestoreFlow::SelectBackup => render_restore_select_backup(f, app, area),
        RestoreFlow::SelectDevice { .. } => render_restore_select_device(f, app, area),
        RestoreFlow::Confirm {
            backup_idx,
            device_idx,
        } => render_restore_confirm(f, app, area, *backup_idx, *device_idx),
        RestoreFlow::ConfirmDelete { backup_idx } => {
            render_restore_confirm_delete(f, app, area, *backup_idx)
        }
        RestoreFlow::Running => render_restore_running(f, app, area),
        RestoreFlow::Done(msg) => render_restore_done(f, app, area, msg),
    }
}

fn render_restore_select_backup(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "Restore — Select Backup",
        Style::default().fg(Color::Cyan),
    ));

    if app.restore_loading {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        f.render_widget(
            Paragraph::new(format!("{} Scanning backups and devices...", s))
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    if app.backups.is_empty() {
        f.render_widget(
            Paragraph::new("No backups found.\n\nRun a backup first from the Dashboard tab, then press [R] to refresh.")
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .backups
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let sel = i == app.restore_selected_backup;
            let arrow = if sel { "▶ " } else { "  " };
            let name_style = if sel {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let age = time_ago(&b.last_run);
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled(arrow, Style::default().fg(Color::Cyan)),
                    Span::styled(b.name.replace('_', " "), name_style),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled(
                        format!("{}  ·  {}", b.size, age),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::raw(""),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.restore_selected_backup));
    f.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn render_restore_select_device(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "Restore — Select Device",
        Style::default().fg(Color::Cyan),
    ));

    if app.connected_devices.is_empty() {
        f.render_widget(
            Paragraph::new("No devices connected via USB or Wi-Fi.\n\nPlug in or enable Wi-Fi sync, then press [R] to refresh.")
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .connected_devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let sel = i == app.restore_selected_device;
            let arrow = if sel { "▶ " } else { "  " };
            let name_style = if sel {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let ios = d.ios.as_deref().unwrap_or("?");
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled(arrow, Style::default().fg(Color::Cyan)),
                    Span::styled(&d.name, name_style),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled(
                        format!("iOS {}  ·  {}", ios, &d.udid[..8.min(d.udid.len())]),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::raw(""),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.restore_selected_device));
    f.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn render_restore_confirm(f: &mut Frame, app: &App, area: Rect, bidx: usize, didx: usize) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "Restore — Confirm",
        Style::default().fg(Color::Yellow),
    ));

    let backup_name = app
        .backups
        .get(bidx)
        .map(|b| b.name.replace('_', " "))
        .unwrap_or_default();
    let backup_size = app
        .backups
        .get(bidx)
        .map(|b| b.size.clone())
        .unwrap_or_default();
    let device_name = app
        .connected_devices
        .get(didx)
        .map(|d| d.name.clone())
        .unwrap_or_default();

    let text = format!(
        "WARNING: This will OVERWRITE all data on the device.\n\n\
         Backup : {backup_name}  ({backup_size})\n\
         Device : {device_name}\n\n\
         Press [Enter] to start restore, or [Esc] to go back."
    );
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(Color::White)),
        area,
    );
}

fn render_restore_confirm_delete(f: &mut Frame, app: &App, area: Rect, bidx: usize) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "Delete Backup — Confirm",
        Style::default().fg(Color::Red),
    ));

    let backup_name = app
        .backups
        .get(bidx)
        .map(|b| b.name.replace('_', " "))
        .unwrap_or_default();
    let backup_size = app
        .backups
        .get(bidx)
        .map(|b| b.size.clone())
        .unwrap_or_default();

    let text = format!(
        "WARNING: This will PERMANENTLY delete the backup.\n\n\
         Backup : {backup_name}  ({backup_size})\n\n\
         Press [Enter] to confirm deletion, or [Esc] to cancel."
    );
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(Color::White)),
        area,
    );
}

fn render_restore_running(f: &mut Frame, app: &App, area: Rect) {
    let s = SPINNER[app.spinner_frame % SPINNER.len()];
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        format!("Restore {} running...", s),
        Style::default().fg(Color::Yellow),
    ));

    let inner_h = area.height.saturating_sub(2) as usize;
    let total = app.restore_logs.len();
    let bottom = app.restore_log_scroll.min(total.saturating_sub(1));
    let top = bottom.saturating_sub(inner_h.saturating_sub(1));
    let visible = if total > 0 {
        &app.restore_logs[top..=bottom.min(total - 1)]
    } else {
        &[][..]
    };

    let lines: Vec<Line> = visible.iter().map(|l| log_line_color(l)).collect();
    f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

fn render_restore_done(f: &mut Frame, _app: &App, area: Rect, msg: &str) {
    let color = if msg.contains('✓') {
        Color::Green
    } else {
        Color::Red
    };
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "Restore — Complete",
        Style::default().fg(color),
    ));
    let text = format!("{}\n\nPress [Esc] to return to the backup list.", msg);
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(color)),
        area,
    );
}

// ── Services ──────────────────────────────────────────────────────────────────

fn render_services(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    render_launchd_status(f, app, cols[0]);
    render_connected_devices(f, app, cols[1]);
    render_path_editor(f, app, rows[1]);
}

fn render_launchd_status(f: &mut Frame, app: &App, area: Rect) {
    let st = &app.launchd_status;
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "launchd Service",
        Style::default().fg(Color::Cyan),
    ));

    let installed_sym = if st.installed {
        ("✓", Color::Green)
    } else {
        ("✗", Color::Red)
    };
    let loaded_sym = if st.loaded {
        ("✓", Color::Green)
    } else {
        ("✗", Color::Red)
    };

    let schedule_str = if app.editing_schedule {
        format!("{}█", app.schedule_input)
    } else {
        format!(
            "daily at {:02}:{:02}  [c] change",
            app.config.schedule_hour, app.config.schedule_minute
        )
    };
    let schedule_color = if app.editing_schedule {
        Color::Yellow
    } else {
        Color::Gray
    };

    let home = dirs::home_dir().unwrap_or_default();
    let plist_display = st
        .plist_path
        .to_string_lossy()
        .replace(home.to_str().unwrap_or(""), "~");

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::raw("  Installed  "),
            Span::styled(
                installed_sym.0,
                Style::default()
                    .fg(installed_sym.1)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Loaded     "),
            Span::styled(
                loaded_sym.0,
                Style::default()
                    .fg(loaded_sym.1)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Schedule   ", Style::default().fg(Color::DarkGray)),
            Span::styled(schedule_str, Style::default().fg(schedule_color)),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Plist      ", Style::default().fg(Color::DarkGray)),
            Span::styled(plist_display, Style::default().fg(Color::DarkGray)),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Log        ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "/tmp/iphone-backup-launchd.log",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::raw(""),
        Line::raw(""),
        Line::from(Span::styled(
            "  [i] install+load  [l] load  [u] unload  [s] start now  [U] update",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

fn render_path_editor(f: &mut Frame, app: &App, area: Rect) {
    let backup_path = app.config.backup_path().to_string_lossy().into_owned();
    let home = dirs::home_dir().unwrap_or_default();
    let display_path = backup_path.replace(home.to_str().unwrap_or(""), "~");

    let accessible = app.storage_ok;
    let (status_sym, status_color) = if accessible {
        ("● mounted", Color::Green)
    } else {
        ("○ not accessible", Color::Red)
    };

    if app.editing_path {
        let input = format!(" Backup path: {}█", app.path_input);
        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            "Edit Backup Path  [Enter] confirm  [Esc] cancel",
            Style::default().fg(Color::Yellow),
        ));
        f.render_widget(
            Paragraph::new(input)
                .block(block)
                .style(Style::default().fg(Color::White)),
            area,
        );
    } else {
        let line = Line::from(vec![
            Span::styled("  Backup path  ", Style::default().fg(Color::DarkGray)),
            Span::styled(&display_path, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled(status_sym, Style::default().fg(status_color)),
            Span::styled("    [e] edit", Style::default().fg(Color::DarkGray)),
        ]);
        let block = Block::default().borders(Borders::ALL);
        f.render_widget(Paragraph::new(line).block(block), area);
    }
}

fn render_connected_devices(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        "Connected Devices",
        Style::default().fg(Color::Cyan),
    ));

    if app.connected_devices.is_empty() {
        f.render_widget(
            Paragraph::new("\n  No devices found.\n\n  Connect via USB or ensure\n  Wi-Fi sync is enabled.\n\n  Press [R] to refresh.\n  Press [p] to pair.")
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .connected_devices
        .iter()
        .map(|d| {
            let ios = d.ios.as_deref().unwrap_or("?");
            let model = d.model.as_deref().unwrap_or("?");
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        &d.name,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{} · iOS {}", model, ios),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(&d.udid, Style::default().fg(Color::DarkGray)),
                ]),
                Line::raw(""),
            ]))
        })
        .collect();

    f.render_widget(List::new(items).block(block), area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn log_line_color(l: &str) -> Line<'_> {
    let color = if l.contains('✓')
        || l.contains("Done")
        || l.contains("complete")
        || l.contains("success")
    {
        Color::Green
    } else if l.contains("ERROR") || l.contains('✗') || l.contains("failed") || l.contains("error")
    {
        Color::Red
    } else if l.contains("Backing up")
        || l.contains("Discovering")
        || l.contains("Pairing")
        || l.contains("restore")
    {
        Color::Cyan
    } else {
        Color::Gray
    };
    Line::from(Span::styled(l, Style::default().fg(color)))
}

fn time_ago(iso: &str) -> String {
    let dt = DateTime::parse_from_rfc3339(iso)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let diff = Utc::now().signed_duration_since(dt);
    if diff.num_minutes() < 2 {
        "just now".into()
    } else if diff.num_hours() < 1 {
        format!("{}m ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else {
        format!("{}d ago", diff.num_days())
    }
}
