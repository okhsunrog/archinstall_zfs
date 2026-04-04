use ratatui::style::{Color, Modifier, Style};

// ── ANSI 16 colors — the only palette Linux tty supports ──

// ── Background styles ──────────────────────────────
pub const BG_STYLE: Style = Style::new().bg(Color::Black).fg(Color::White);
pub const SIDEBAR_BG: Style = Style::new().bg(Color::Black).fg(Color::White);

// ── Text styles ────────────────────────────────────
pub const TITLE_STYLE: Style = Style::new()
    .fg(Color::LightCyan)
    .add_modifier(Modifier::BOLD);
pub const NORMAL_STYLE: Style = Style::new().fg(Color::White);
pub const DIMMED_STYLE: Style = Style::new().fg(Color::DarkGray);
pub const LABEL_STYLE: Style = Style::new().fg(Color::Gray);

// ── Interactive element styles ─────────────────────
pub const SELECTED_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::LightCyan)
    .add_modifier(Modifier::BOLD);
pub const SELECTED_VALUE_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::LightCyan)
    .add_modifier(Modifier::BOLD);
pub const HOVER_BG: Style = Style::new().bg(Color::DarkGray);

// ── Status styles ──────────────────────────────────
pub const VALUE_STYLE: Style = Style::new().fg(Color::LightGreen);
pub const UNSET_STYLE: Style = Style::new().fg(Color::DarkGray);
pub const ERROR_STYLE: Style = Style::new()
    .fg(Color::LightRed)
    .add_modifier(Modifier::BOLD);
pub const SUCCESS_STYLE: Style = Style::new()
    .fg(Color::LightGreen)
    .add_modifier(Modifier::BOLD);
pub const WARN_STYLE: Style = Style::new().fg(Color::LightYellow);
pub const SECTION_STYLE: Style = Style::new()
    .fg(Color::LightMagenta)
    .add_modifier(Modifier::BOLD);
pub const ACTION_STYLE: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const ACCENT_STYLE: Style = Style::new().fg(Color::Cyan);

// ── Border/header styles ───────────────────────────
pub const BORDER_STYLE: Style = Style::new().fg(Color::DarkGray);
pub const HEADER_STYLE: Style = Style::new()
    .fg(Color::LightMagenta)
    .add_modifier(Modifier::BOLD);

// ── Sidebar styles ─────────────────────────────────
pub const SIDEBAR_CURRENT: Style = Style::new()
    .fg(Color::LightCyan)
    .bg(Color::DarkGray)
    .add_modifier(Modifier::BOLD);
pub const SIDEBAR_DONE: Style = Style::new().fg(Color::Green);
pub const SIDEBAR_PENDING: Style = Style::new().fg(Color::DarkGray);

// ── Element type indicators ────────────────────────
pub const TOGGLE_ON: Style = Style::new()
    .fg(Color::LightGreen)
    .add_modifier(Modifier::BOLD);
pub const TOGGLE_OFF: Style = Style::new().fg(Color::DarkGray);
pub const RADIO_SELECTED: Style = Style::new().fg(Color::LightGreen);
pub const RADIO_UNSELECTED: Style = Style::new().fg(Color::DarkGray);
