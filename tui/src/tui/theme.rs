use ratatui::style::{Color, Modifier, Style};

// ── ANSI colors (work in all terminals including linux tty) ──
const BASE: Color = Color::Black;
const MANTLE: Color = Color::Black;
const SURFACE0: Color = Color::DarkGray;
const OVERLAY0: Color = Color::DarkGray;
const TEXT: Color = Color::White;
const SUBTEXT0: Color = Color::Gray;
const BLUE: Color = Color::LightBlue;
const GREEN: Color = Color::LightGreen;
const RED: Color = Color::LightRed;
const YELLOW: Color = Color::LightYellow;
const MAUVE: Color = Color::LightMagenta;
const TEAL: Color = Color::LightCyan;
const PEACH: Color = Color::Yellow;
const LAVENDER: Color = Color::LightMagenta;

// ── Background styles ──────────────────────────────
pub const BG_STYLE: Style = Style::new().bg(BASE).fg(TEXT);
pub const SIDEBAR_BG: Style = Style::new().bg(MANTLE).fg(TEXT);

// ── Text styles ────────────────────────────────────
pub const TITLE_STYLE: Style = Style::new().fg(BLUE).add_modifier(Modifier::BOLD);
pub const NORMAL_STYLE: Style = Style::new().fg(TEXT);
pub const DIMMED_STYLE: Style = Style::new().fg(OVERLAY0);
pub const LABEL_STYLE: Style = Style::new().fg(SUBTEXT0);

// ── Interactive element styles ─────────────────────
pub const SELECTED_STYLE: Style = Style::new().fg(Color::Black).bg(BLUE);
pub const SELECTED_VALUE_STYLE: Style = Style::new().fg(Color::Black).bg(BLUE);
pub const HOVER_BG: Style = Style::new().bg(SURFACE0);

// ── Status styles ──────────────────────────────────
pub const VALUE_STYLE: Style = Style::new().fg(GREEN);
pub const UNSET_STYLE: Style = Style::new().fg(OVERLAY0);
pub const ERROR_STYLE: Style = Style::new().fg(RED).add_modifier(Modifier::BOLD);
pub const SUCCESS_STYLE: Style = Style::new().fg(GREEN).add_modifier(Modifier::BOLD);
pub const WARN_STYLE: Style = Style::new().fg(YELLOW);
pub const SECTION_STYLE: Style = Style::new().fg(MAUVE).add_modifier(Modifier::BOLD);
pub const ACTION_STYLE: Style = Style::new().fg(PEACH).add_modifier(Modifier::BOLD);
pub const ACCENT_STYLE: Style = Style::new().fg(TEAL);

// ── Border/header styles ───────────────────────────
pub const BORDER_STYLE: Style = Style::new().fg(SURFACE0);
pub const HEADER_STYLE: Style = Style::new().fg(LAVENDER).add_modifier(Modifier::BOLD);

// ── Sidebar styles ─────────────────────────────────
pub const SIDEBAR_CURRENT: Style = Style::new()
    .fg(BLUE)
    .bg(SURFACE0)
    .add_modifier(Modifier::BOLD);
pub const SIDEBAR_DONE: Style = Style::new().fg(GREEN).bg(MANTLE);
pub const SIDEBAR_PENDING: Style = Style::new().fg(OVERLAY0).bg(MANTLE);

// ── Element type indicators ────────────────────────
pub const TOGGLE_ON: Style = Style::new().fg(GREEN).add_modifier(Modifier::BOLD);
pub const TOGGLE_OFF: Style = Style::new().fg(OVERLAY0);
pub const RADIO_SELECTED: Style = Style::new().fg(GREEN);
pub const RADIO_UNSELECTED: Style = Style::new().fg(OVERLAY0);
