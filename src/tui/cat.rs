use ratatui::style::Color;

use super::App;

// ── Cat state ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CatState {
    Idle,
    Backup,
    Pairing,
    Restore,
}

/// Derive the cat's current state from the app's running flags.
pub fn cat_state(app: &App) -> CatState {
    if app.backup_running || app.active_job.is_some() || app.update_running {
        CatState::Backup
    } else if app.restore_running {
        CatState::Restore
    } else if app.pairing_running {
        CatState::Pairing
    } else {
        CatState::Idle
    }
}

// ── Frame data ───────────────────────────────────────────────────────────────
//
// Each frame is 5 lines of ASCII art.  Every line within a state is exactly
// 14 characters wide so the centered Paragraph renders stably across frames.

type CatFrame = [&'static str; 5];

// — Idle: sitting peacefully, occasional blink + tail sway ————————————————————

const IDLE_OPEN: CatFrame = [
    r"   /\_/\      ",
    r"  ( o.o )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_) ~  ",
];

const IDLE_BLINK: CatFrame = [
    r"   /\_/\      ",
    r"  ( -.- )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_) ~  ",
];

const IDLE_TAIL_L: CatFrame = [
    r"   /\_/\      ",
    r"  ( o.o )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r" ~(_|  |_)    ",
];

// 12 frames ≈ 2.4 s at 5 FPS — blink once, tail sway once per cycle
const IDLE_FRAMES: &[&CatFrame] = &[
    &IDLE_OPEN, &IDLE_OPEN, &IDLE_OPEN, &IDLE_OPEN,
    &IDLE_BLINK, &IDLE_OPEN, &IDLE_OPEN, &IDLE_OPEN,
    &IDLE_TAIL_L, &IDLE_TAIL_L, &IDLE_OPEN, &IDLE_OPEN,
];

// — Backup: working hard, effort sparkle falls down the right side ————————————

const BACKUP_1: CatFrame = [
    r"   /\_/\    * ",
    r"  ( >_< )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const BACKUP_2: CatFrame = [
    r"   /\_/\      ",
    r"  ( >.< )   * ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const BACKUP_3: CatFrame = [
    r"   /\_/\      ",
    r"  ( >_< )     ",
    r"   > ^ <    * ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const BACKUP_4: CatFrame = [
    r"   /\_/\      ",
    r"  ( >.< )     ",
    r"   > ^ <      ",
    r"   /|  |\   * ",
    r"  (_|  |_)    ",
];

const BACKUP_5: CatFrame = [
    r"   /\_/\      ",
    r"  ( >_< )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)  * ",
];

// 5 frames = 1.0 s cycle
const BACKUP_FRAMES: &[&CatFrame] = &[
    &BACKUP_1, &BACKUP_2, &BACKUP_3, &BACKUP_4, &BACKUP_5,
];

// — Pairing: curious cat, signal dots pulse near ears —————————————————————————

const PAIR_1: CatFrame = [
    r"   /\_/\  .   ",
    r"  ( o.O )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const PAIR_2: CatFrame = [
    r"   /\_/\  ..  ",
    r"  ( O.o )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const PAIR_3: CatFrame = [
    r"   /\_/\  ... ",
    r"  ( o.O )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const PAIR_4: CatFrame = [
    r"   /\_/\  ..  ",
    r"  ( O.o )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const PAIR_5: CatFrame = [
    r"   /\_/\  .   ",
    r"  ( o.O )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

// 5 frames = 1.0 s cycle — signal strength pulses
const PAIRING_FRAMES: &[&CatFrame] = &[
    &PAIR_1, &PAIR_2, &PAIR_3, &PAIR_4, &PAIR_5,
];

// — Restore: happy cat, data packet (+) rises from bottom ————————————————————

const REST_1: CatFrame = [
    r"   /\_/\      ",
    r"  ( ^.^ )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)  + ",
];

const REST_2: CatFrame = [
    r"   /\_/\      ",
    r"  ( ^.^ )     ",
    r"   > ^ <      ",
    r"   /|  |\   + ",
    r"  (_|  |_)    ",
];

const REST_3: CatFrame = [
    r"   /\_/\      ",
    r"  ( ^.^ )     ",
    r"   > ^ <    + ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const REST_4: CatFrame = [
    r"   /\_/\      ",
    r"  ( ^.^ )   + ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

const REST_5: CatFrame = [
    r"   /\_/\    + ",
    r"  ( ^_^ )     ",
    r"   > ^ <      ",
    r"   /|  |\     ",
    r"  (_|  |_)    ",
];

// 5 frames = 1.0 s cycle — + floats up, cat smiles on receipt
const RESTORE_FRAMES: &[&CatFrame] = &[
    &REST_1, &REST_2, &REST_3, &REST_4, &REST_5,
];

// ── Public accessors ─────────────────────────────────────────────────────────

/// Return the current animation frame for the given state and tick.
pub fn current_frame(state: CatState, tick: usize) -> &'static CatFrame {
    let frames: &[&CatFrame] = match state {
        CatState::Idle => IDLE_FRAMES,
        CatState::Backup => BACKUP_FRAMES,
        CatState::Pairing => PAIRING_FRAMES,
        CatState::Restore => RESTORE_FRAMES,
    };
    frames[tick % frames.len()]
}

/// Short label + colour for the cat block title.
pub fn status_label(state: CatState) -> (&'static str, Color) {
    match state {
        CatState::Idle => ("zzz", Color::DarkGray),
        CatState::Backup => ("working!", Color::Yellow),
        CatState::Pairing => ("pairing...", Color::Cyan),
        CatState::Restore => ("restoring!", Color::Green),
    }
}
