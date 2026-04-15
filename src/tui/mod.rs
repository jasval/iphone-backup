pub mod ui;

use crate::{backup, config::Config, status};
use crate::status::{DeviceStatus, Summary};
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
    time::{Duration, Instant},
};

pub struct App {
    pub config: Config,
    pub devices: Vec<DeviceStatus>,
    pub summary: Option<Summary>,
    pub logs: Vec<String>,
    /// Index of the bottom-most visible log line
    pub log_scroll: usize,
    pub auto_scroll: bool,
    pub selected: usize,
    pub backup_running: bool,
    backup_thread: Option<std::thread::JoinHandle<()>>,
    pub log_rx: mpsc::Receiver<String>,
    log_tx: mpsc::Sender<String>,
    last_refresh: Instant,
    pub spinner_frame: usize,
    pub should_quit: bool,
    pub storage_ok: bool,
    pub flash: Option<String>,
}

impl App {
    fn new(config: Config, tx: mpsc::Sender<String>, rx: mpsc::Receiver<String>) -> Self {
        Self {
            config,
            devices: vec![],
            summary: None,
            logs: vec![],
            log_scroll: 0,
            auto_scroll: true,
            selected: 0,
            backup_running: false,
            backup_thread: None,
            log_rx: rx,
            log_tx: tx,
            last_refresh: Instant::now(),
            spinner_frame: 0,
            should_quit: false,
            storage_ok: false,
            flash: None,
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
        self.backup_running = true;
        self.auto_scroll = true;
        self.flash = Some("Backup started...".into());
        let tx = self.log_tx.clone();
        let backup_path = self.config.backup_path();
        self.backup_thread = Some(std::thread::spawn(move || {
            let _ = backup::run(&backup_path, tx);
        }));
    }
}

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

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
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

        // Drain log lines from the backup thread
        let mut got_new = false;
        while let Ok(line) = app.log_rx.try_recv() {
            app.logs.push(line);
            if app.logs.len() > 500 {
                app.logs.drain(..50);
            }
            got_new = true;
        }
        if got_new && app.auto_scroll {
            app.log_scroll = app.logs.len().saturating_sub(1);
        }

        // Check if the backup thread has finished
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

        // Periodic status refresh (every 30 s)
        if app.last_refresh.elapsed() > Duration::from_secs(30) {
            app.refresh();
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) {
    app.flash = None;
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('r') => app.trigger_backup(),
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
