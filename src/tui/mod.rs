pub mod ui;

use crate::device::Device;
use crate::launchd::LaunchdStatus;
use crate::restore::BackupEntry;
use crate::status::{DeviceStatus, Summary};
use crate::{backup, config::Config, device, launchd, restore, status, update};
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io,
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

// ── Tab ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Tab {
    Dashboard,
    Restore,
    Services,
}

impl Tab {
    pub fn next(&self) -> Self {
        match self {
            Tab::Dashboard => Tab::Restore,
            Tab::Restore => Tab::Services,
            Tab::Services => Tab::Dashboard,
        }
    }
}

// ── Restore flow ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum RestoreFlow {
    SelectBackup,
    SelectDevice {
        backup_idx: usize,
    },
    Confirm {
        backup_idx: usize,
        device_idx: usize,
    },
    Running,
    Done(String),
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct App {
    // ── shared ────────────────────────────────────────────────────────────────
    pub config: Config,
    pub tab: Tab,
    pub should_quit: bool,
    pub flash: Option<String>,
    pub spinner_frame: usize,

    // ── log channel (shared by backup + pairing + restore) ───────────────────
    pub log_rx: mpsc::Receiver<String>,
    pub log_tx: mpsc::Sender<String>,

    // ── Dashboard tab ─────────────────────────────────────────────────────────
    pub devices: Vec<DeviceStatus>,
    pub summary: Option<Summary>,
    pub logs: Vec<String>,
    pub log_scroll: usize,
    pub auto_scroll: bool,
    pub selected: usize,
    pub backup_running: bool,
    backup_thread: Option<JoinHandle<()>>,
    pub storage_ok: bool,
    last_refresh: Instant,

    // ── Restore tab ───────────────────────────────────────────────────────────
    pub restore_flow: RestoreFlow,
    pub backups: Vec<BackupEntry>,
    pub connected_devices: Vec<Device>,
    pub restore_selected_backup: usize,
    pub restore_selected_device: usize,
    pub restore_running: bool,
    restore_thread: Option<JoinHandle<bool>>,
    pub restore_logs: Vec<String>,
    pub restore_log_scroll: usize,

    // ── Services tab ──────────────────────────────────────────────────────────
    pub launchd_status: LaunchdStatus,
    pub services_flash: Option<String>,
    pub pairing_running: bool,
    pairing_thread: Option<JoinHandle<()>>,

    // ── Path editing ──────────────────────────────────────────────────────────
    pub editing_path: bool,
    pub path_input: String,

    // ── Schedule editing ──────────────────────────────────────────────────────
    pub editing_schedule: bool,
    pub schedule_input: String,

    // ── Update ────────────────────────────────────────────────────────────────
    pub update_running: bool,
    update_thread: Option<JoinHandle<bool>>,
}

impl App {
    fn new(config: Config, tx: mpsc::Sender<String>, rx: mpsc::Receiver<String>) -> Self {
        let launchd_status = launchd::status();
        Self {
            config,
            tab: Tab::Dashboard,
            should_quit: false,
            flash: None,
            spinner_frame: 0,

            log_rx: rx,
            log_tx: tx,

            devices: vec![],
            summary: None,
            logs: vec![],
            log_scroll: 0,
            auto_scroll: true,
            selected: 0,
            backup_running: false,
            backup_thread: None,
            storage_ok: false,
            last_refresh: Instant::now(),

            restore_flow: RestoreFlow::SelectBackup,
            backups: vec![],
            connected_devices: vec![],
            restore_selected_backup: 0,
            restore_selected_device: 0,
            restore_running: false,
            restore_thread: None,
            restore_logs: vec![],
            restore_log_scroll: 0,

            launchd_status,
            services_flash: None,
            pairing_running: false,
            pairing_thread: None,

            editing_path: false,
            path_input: String::new(),

            editing_schedule: false,
            schedule_input: String::new(),

            update_running: false,
            update_thread: None,
        }
    }

    pub fn refresh(&mut self) {
        let status_dir = self.config.status_dir();
        self.storage_ok = std::fs::read_dir(self.config.backup_path()).is_ok();
        self.devices = status::load_devices(&status_dir);
        self.summary = status::load_summary(&status_dir);
        self.last_refresh = Instant::now();
        if !self.devices.is_empty() {
            self.selected = self.selected.min(self.devices.len() - 1);
        }
    }

    pub fn reload_logs(&mut self) {
        self.logs = status::tail_log(&self.config.log_path(), 500);
        if self.auto_scroll {
            self.log_scroll = self.logs.len().saturating_sub(1);
        }
    }

    fn trigger_backup(&mut self) {
        if self.backup_running {
            return;
        }
        if !self.storage_ok {
            self.flash = Some(format!(
                "Backup path '{}' is not accessible — check it's mounted.",
                self.config.backup_path().display()
            ));
            return;
        }
        self.backup_running = true;
        self.auto_scroll = true;
        self.flash = Some("Backup started...".into());
        let tx = self.log_tx.clone();
        let backup_path = self.config.backup_path();
        self.backup_thread = Some(std::thread::spawn(move || {
            let _ = backup::run(&backup_path, tx);
        }));
    }

    fn trigger_pair(&mut self) {
        if self.pairing_running || self.backup_running {
            return;
        }
        self.pairing_running = true;
        self.auto_scroll = true;
        self.flash = Some("Pairing...".into());
        let tx = self.log_tx.clone();
        self.pairing_thread = Some(std::thread::spawn(move || {
            device::pair(None, &tx);
        }));
    }

    fn refresh_restore_tab(&mut self) {
        self.backups = restore::list_backups(&self.config.backup_path());
        self.connected_devices = device::list_connected();
        if self.restore_selected_backup >= self.backups.len() {
            self.restore_selected_backup = self.backups.len().saturating_sub(1);
        }
        if self.restore_selected_device >= self.connected_devices.len() {
            self.restore_selected_device = self.connected_devices.len().saturating_sub(1);
        }
    }

    fn refresh_services_tab(&mut self) {
        self.launchd_status = launchd::status();
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn run(config: Config) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut app = App::new(config, tx, rx);
    app.refresh();
    app.reload_logs();

    let result = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

// ── Event loop ────────────────────────────────────────────────────────────────

fn event_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(app, key.code);
                }
            }
        }

        app.spinner_frame = app.spinner_frame.wrapping_add(1);

        // Drain shared log channel
        let mut got_new = false;
        while let Ok(line) = app.log_rx.try_recv() {
            // route to restore_logs when restore is running, else dashboard logs
            if app.restore_running {
                app.restore_logs.push(line.clone());
                if app.restore_logs.len() > 500 {
                    app.restore_logs.drain(..50);
                }
                app.restore_log_scroll = app.restore_logs.len().saturating_sub(1);
            } else {
                app.logs.push(line.clone());
                if app.logs.len() > 500 {
                    app.logs.drain(..50);
                }
                got_new = true;
            }
        }
        if got_new && app.auto_scroll {
            app.log_scroll = app.logs.len().saturating_sub(1);
        }

        // Backup thread completion
        if app
            .backup_thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(false)
        {
            app.backup_running = false;
            app.backup_thread = None;
            app.auto_scroll = false;
            app.refresh();
            app.reload_logs();
            app.flash = Some("Backup complete.".into());
        }

        // Pairing thread completion
        if app
            .pairing_thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(false)
        {
            app.pairing_running = false;
            app.pairing_thread = None;
            app.flash = Some("Pairing finished — check log for result.".into());
            app.services_flash = Some("Pairing complete.".into());
            app.refresh_services_tab();
        }

        // Update thread completion
        if app
            .update_thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(false)
        {
            let ok = app
                .update_thread
                .take()
                .and_then(|t| t.join().ok())
                .unwrap_or(false);
            app.update_running = false;
            let msg = if ok {
                "✓ Update complete. Restart to use the new version.".to_string()
            } else {
                "Update finished — check log for details.".to_string()
            };
            app.flash = Some(msg.clone());
            app.services_flash = Some(msg);
        }

        // Restore thread completion
        if app
            .restore_thread
            .as_ref()
            .map(|t| t.is_finished())
            .unwrap_or(false)
        {
            let ok = app
                .restore_thread
                .take()
                .and_then(|t| t.join().ok())
                .unwrap_or(false);
            app.restore_running = false;
            let msg = if ok {
                "✓ Restore complete.".to_string()
            } else {
                "✗ Restore failed — check the log for details.".to_string()
            };
            app.restore_flow = RestoreFlow::Done(msg);
        }

        // Periodic refresh (every 30 s)
        if app.last_refresh.elapsed() > Duration::from_secs(30) {
            app.refresh();
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_key(app: &mut App, code: KeyCode) {
    // Editing modes intercept all keys
    if app.editing_path {
        handle_path_edit_key(app, code);
        return;
    }
    if app.editing_schedule {
        handle_schedule_edit_key(app, code);
        return;
    }

    // Global keys
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            // Esc inside restore wizard navigates back, not quit
            if app.tab == Tab::Restore {
                handle_restore_key(app, code);
                return;
            }
            app.should_quit = true;
            return;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            app.tab = app.tab.next();
            app.flash = None;
            // Lazy-load data when switching tabs
            match &app.tab {
                Tab::Restore => app.refresh_restore_tab(),
                Tab::Services => app.refresh_services_tab(),
                _ => {}
            }
            return;
        }
        KeyCode::Char('1') => {
            app.tab = Tab::Dashboard;
            app.flash = None;
            return;
        }
        KeyCode::Char('2') => {
            app.tab = Tab::Restore;
            app.flash = None;
            app.refresh_restore_tab();
            return;
        }
        KeyCode::Char('3') => {
            app.tab = Tab::Services;
            app.flash = None;
            app.refresh_services_tab();
            return;
        }
        _ => {}
    }

    match app.tab {
        Tab::Dashboard => handle_dashboard_key(app, code),
        Tab::Restore => handle_restore_key(app, code),
        Tab::Services => handle_services_key(app, code),
    }
}

fn handle_dashboard_key(app: &mut App, code: KeyCode) {
    app.flash = None;
    match code {
        KeyCode::Char('r') => app.trigger_backup(),
        KeyCode::Char('p') => app.trigger_pair(),
        KeyCode::Up | KeyCode::Char('k') => {
            if app.selected > 0 {
                app.selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.selected + 1 < app.devices.len() {
                app.selected += 1;
            }
        }
        KeyCode::PageUp => {
            app.log_scroll = app.log_scroll.saturating_sub(10);
            app.auto_scroll = false;
        }
        KeyCode::PageDown => {
            let max = app.logs.len().saturating_sub(1);
            app.log_scroll = (app.log_scroll + 10).min(max);
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.log_scroll = app.logs.len().saturating_sub(1);
            app.auto_scroll = true;
        }
        _ => {}
    }
}

fn handle_restore_key(app: &mut App, code: KeyCode) {
    match &app.restore_flow.clone() {
        RestoreFlow::SelectBackup => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if app.restore_selected_backup > 0 {
                    app.restore_selected_backup -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.restore_selected_backup + 1 < app.backups.len() {
                    app.restore_selected_backup += 1;
                }
            }
            KeyCode::Enter => {
                if !app.backups.is_empty() {
                    let idx = app.restore_selected_backup;
                    app.restore_flow = RestoreFlow::SelectDevice { backup_idx: idx };
                }
            }
            KeyCode::Char('R') => app.refresh_restore_tab(),
            _ => {}
        },
        RestoreFlow::SelectDevice { backup_idx } => {
            let bidx = *backup_idx;
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if app.restore_selected_device > 0 {
                        app.restore_selected_device -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.restore_selected_device + 1 < app.connected_devices.len() {
                        app.restore_selected_device += 1;
                    }
                }
                KeyCode::Enter => {
                    if !app.connected_devices.is_empty() {
                        let didx = app.restore_selected_device;
                        app.restore_flow = RestoreFlow::Confirm {
                            backup_idx: bidx,
                            device_idx: didx,
                        };
                    }
                }
                KeyCode::Esc => app.restore_flow = RestoreFlow::SelectBackup,
                KeyCode::Char('R') => app.refresh_restore_tab(),
                _ => {}
            }
        }
        RestoreFlow::Confirm {
            backup_idx,
            device_idx,
        } => {
            let bidx = *backup_idx;
            let didx = *device_idx;
            match code {
                KeyCode::Enter => {
                    if let (Some(backup), Some(dev)) =
                        (app.backups.get(bidx), app.connected_devices.get(didx))
                    {
                        app.restore_logs.clear();
                        app.restore_log_scroll = 0;
                        app.restore_running = true;
                        app.restore_flow = RestoreFlow::Running;
                        let tx = app.log_tx.clone();
                        let udid = dev.udid.clone();
                        let path = backup.path.clone();
                        app.restore_thread = Some(restore::run(&udid, &path, tx));
                    }
                }
                KeyCode::Esc => {
                    app.restore_flow = RestoreFlow::SelectDevice { backup_idx: bidx };
                }
                _ => {}
            }
        }
        RestoreFlow::Running => {
            // No keys during restore except scroll
            match code {
                KeyCode::PageUp => {
                    app.restore_log_scroll = app.restore_log_scroll.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    let max = app.restore_logs.len().saturating_sub(1);
                    app.restore_log_scroll = (app.restore_log_scroll + 10).min(max);
                }
                _ => {}
            }
        }
        RestoreFlow::Done(_) => {
            if matches!(code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                app.restore_flow = RestoreFlow::SelectBackup;
                app.restore_logs.clear();
            }
        }
    }
}

fn handle_services_key(app: &mut App, code: KeyCode) {
    app.services_flash = None;
    match code {
        KeyCode::Char('i') => {
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/iphone-backup"));
            match launchd::install(&exe, app.config.schedule_hour, app.config.schedule_minute) {
                Ok(()) => {
                    app.services_flash = Some("launchd agent installed and loaded.".into());
                }
                Err(e) => {
                    app.services_flash = Some(format!("Install failed: {e}"));
                }
            }
            app.refresh_services_tab();
        }
        KeyCode::Char('l') => {
            match launchd::load() {
                Ok(()) => app.services_flash = Some("Agent loaded.".into()),
                Err(e) => app.services_flash = Some(format!("Load failed: {e}")),
            }
            app.refresh_services_tab();
        }
        KeyCode::Char('u') => {
            match launchd::unload() {
                Ok(()) => app.services_flash = Some("Agent unloaded.".into()),
                Err(e) => app.services_flash = Some(format!("Unload failed: {e}")),
            }
            app.refresh_services_tab();
        }
        KeyCode::Char('s') => match launchd::start() {
            Ok(()) => app.services_flash = Some("Backup triggered via launchd.".into()),
            Err(e) => app.services_flash = Some(format!("Start failed: {e}")),
        },
        KeyCode::Char('p') => {
            // Pair connected devices
            app.tab = Tab::Dashboard;
            app.trigger_pair();
        }
        KeyCode::Char('e') => {
            app.path_input = app.config.backup_path().to_string_lossy().into_owned();
            app.editing_path = true;
            app.services_flash = None;
        }
        KeyCode::Char('c') => {
            // Edit schedule
            app.schedule_input = format!(
                "{:02}:{:02}",
                app.config.schedule_hour, app.config.schedule_minute
            );
            app.editing_schedule = true;
            app.services_flash = None;
        }
        KeyCode::Char('U') => {
            // Run update
            if !app.update_running {
                app.update_running = true;
                app.auto_scroll = true;
                app.services_flash = Some("Update started — check Dashboard log...".into());
                // Route output to the shared log channel (visible in Dashboard)
                let tx = app.log_tx.clone();
                app.update_thread = Some(update::run(tx));
                // Switch to dashboard so the user can see the streaming output
                app.tab = Tab::Dashboard;
            }
        }
        KeyCode::Char('R') => app.refresh_services_tab(),
        _ => {}
    }
}

fn handle_schedule_edit_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.editing_schedule = false;
            app.schedule_input.clear();
        }
        KeyCode::Enter => {
            let input = app.schedule_input.trim().to_string();
            // Accept HH:MM or H:MM
            let parts: Vec<&str> = input.splitn(2, ':').collect();
            let parsed = if parts.len() == 2 {
                let h = parts[0].parse::<u8>().ok().filter(|&h| h < 24);
                let m = parts[1].parse::<u8>().ok().filter(|&m| m < 60);
                h.zip(m)
            } else {
                None
            };
            match parsed {
                Some((hour, minute)) => {
                    app.config.schedule_hour = hour;
                    app.config.schedule_minute = minute;
                    match app.config.save() {
                        Ok(()) => {
                            // If the agent is already installed, rewrite and reload the plist.
                            if app.launchd_status.installed {
                                match launchd::set_schedule(hour, minute) {
                                    Ok(()) => {
                                        app.services_flash = Some(format!(
                                            "Schedule updated to {:02}:{:02} and agent reloaded.",
                                            hour, minute
                                        ));
                                    }
                                    Err(e) => {
                                        app.services_flash =
                                            Some(format!("Schedule saved but reload failed: {e}"));
                                    }
                                }
                            } else {
                                app.services_flash = Some(format!(
                                    "Schedule saved as {:02}:{:02} (agent not yet installed).",
                                    hour, minute
                                ));
                            }
                            app.refresh_services_tab();
                        }
                        Err(e) => {
                            app.services_flash = Some(format!("Failed to save config: {e}"));
                        }
                    }
                }
                None => {
                    app.services_flash =
                        Some(format!("Invalid time '{}' — use HH:MM (e.g. 02:00)", input));
                }
            }
            app.editing_schedule = false;
            app.schedule_input.clear();
        }
        KeyCode::Backspace => {
            app.schedule_input.pop();
        }
        KeyCode::Char(c) => {
            // Only allow digits and colon, max 5 chars (HH:MM)
            if (c.is_ascii_digit() || c == ':') && app.schedule_input.len() < 5 {
                app.schedule_input.push(c);
            }
        }
        _ => {}
    }
}

fn handle_path_edit_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.editing_path = false;
            app.path_input.clear();
        }
        KeyCode::Enter => {
            let new_path = app.path_input.trim().to_string();
            if new_path.is_empty() {
                app.services_flash = Some("Path cannot be empty.".into());
                app.editing_path = false;
                app.path_input.clear();
                return;
            }
            let expanded = if new_path.starts_with('~') {
                let home = dirs::home_dir().unwrap_or_default();
                home.join(new_path.trim_start_matches("~/"))
            } else {
                std::path::PathBuf::from(&new_path)
            };
            let expanded_str = expanded.to_string_lossy();
            if !expanded.is_absolute() {
                app.services_flash =
                    Some(format!("Path must be absolute (got '{}').", expanded_str));
            } else if expanded_str.contains("..") {
                app.services_flash = Some("Path must not contain '..' components.".into());
            } else {
                match std::fs::create_dir_all(&expanded) {
                    Ok(()) => {
                        app.config.backup_path = expanded.to_string_lossy().into_owned();
                        match app.config.save() {
                            Ok(()) => {
                                app.services_flash =
                                    Some(format!("Backup path updated to {}", expanded.display()));
                                app.refresh();
                            }
                            Err(e) => {
                                app.services_flash = Some(format!("Failed to save config: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        app.services_flash =
                            Some(format!("Cannot create '{}': {e}", expanded.display()));
                    }
                }
            }
            app.editing_path = false;
            app.path_input.clear();
        }
        KeyCode::Backspace => {
            app.path_input.pop();
        }
        KeyCode::Char(c) => {
            app.path_input.push(c);
        }
        _ => {}
    }
}
