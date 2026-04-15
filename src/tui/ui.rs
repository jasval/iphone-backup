use super::App;
use chrono::{DateTime, Utc};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Min(0),    // main content
            Constraint::Length(1), // footer
        ])
        .split(area);

    render_title(f, app, outer[0]);
    render_main(f, app, outer[1]);
    render_footer(f, app, outer[2]);
}

// ── Title bar ─────────────────────────────────────────────────────────────────

fn render_title(f: &mut Frame, app: &App, area: Rect) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    let storage = if app.storage_ok {
        Span::styled("  ● storage", Style::default().fg(Color::Green))
    } else {
        Span::styled("  ○ storage not found", Style::default().fg(Color::DarkGray))
    };

    let running = if app.backup_running {
        let s = SPINNER[app.spinner_frame % SPINNER.len()];
        Span::styled(
            format!("  {} backing up...", s),
            Style::default().fg(Color::Yellow),
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
        Span::styled(
            format!("  {now} "),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    f.render_widget(
        Paragraph::new(title).style(Style::default().bg(Color::Black)),
        area,
    );
}

// ── Main two-column area ──────────────────────────────────────────────────────

fn render_main(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    render_devices(f, app, cols[0]);
    render_logs(f, app, cols[1]);
}

// ── Devices pane ──────────────────────────────────────────────────────────────

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

// ── Logs pane ─────────────────────────────────────────────────────────────────

fn render_logs(f: &mut Frame, app: &App, area: Rect) {
    let hint = if app.auto_scroll {
        " ↓ live"
    } else {
        " [G] jump to end"
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
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

    let lines: Vec<Line> = visible
        .iter()
        .map(|l| {
            let color = if l.contains("✓") || l.contains("Done") || l.contains("complete") {
                Color::Green
            } else if l.contains("ERROR") || l.contains("✗") || l.contains("failed") {
                Color::Red
            } else if l.contains("Backing up") || l.contains("Discovering") {
                Color::Cyan
            } else {
                Color::Gray
            };
            Line::from(Span::styled(l.as_str(), Style::default().fg(color)))
        })
        .collect();

    f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

// ── Footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let text = if let Some(msg) = &app.flash {
        Span::styled(format!(" {}", msg), Style::default().fg(Color::Yellow))
    } else if app.backup_running {
        Span::styled(
            " Backup running...  [q] quit",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Span::styled(
            " [r] backup  [↑↓] select  [PgUp/PgDn] scroll  [G] log end  [q] quit",
            Style::default().fg(Color::DarkGray),
        )
    };
    f.render_widget(Paragraph::new(Line::from(vec![text])), area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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
