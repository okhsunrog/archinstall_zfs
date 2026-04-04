use ratatui::style::{Color, Modifier, Style};

// ── Catppuccin Mocha palette ────────────────────────
pub const BASE: Color = Color::Rgb(30, 30, 46);
pub const MANTLE: Color = Color::Rgb(24, 24, 37);
pub const SURFACE0: Color = Color::Rgb(49, 50, 68);
pub const SURFACE1: Color = Color::Rgb(69, 71, 90);
pub const OVERLAY0: Color = Color::Rgb(108, 112, 134);
pub const TEXT: Color = Color::Rgb(205, 214, 244);
pub const SUBTEXT0: Color = Color::Rgb(166, 173, 200);
pub const BLUE: Color = Color::Rgb(137, 180, 250);
pub const GREEN: Color = Color::Rgb(166, 227, 161);
pub const RED: Color = Color::Rgb(243, 139, 168);
pub const YELLOW: Color = Color::Rgb(249, 226, 175);
pub const MAUVE: Color = Color::Rgb(203, 166, 247);
pub const TEAL: Color = Color::Rgb(148, 226, 213);
pub const PEACH: Color = Color::Rgb(250, 179, 135);
pub const LAVENDER: Color = Color::Rgb(180, 190, 254);

// ── Background styles ──────────────────────────────
pub const BG_STYLE: Style = Style::new().bg(BASE).fg(TEXT);
pub const SIDEBAR_BG: Style = Style::new().bg(MANTLE).fg(TEXT);

// ── Text styles ────────────────────────────────────
pub const TITLE_STYLE: Style = Style::new().fg(BLUE).add_modifier(Modifier::BOLD);
pub const NORMAL_STYLE: Style = Style::new().fg(TEXT);
pub const DIMMED_STYLE: Style = Style::new().fg(OVERLAY0);
pub const LABEL_STYLE: Style = Style::new().fg(SUBTEXT0);

// ── Interactive element styles ─────────────────────
pub const SELECTED_STYLE: Style = Style::new().fg(BASE).bg(BLUE);
pub const SELECTED_VALUE_STYLE: Style = Style::new().fg(BASE).bg(BLUE);
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
pub const BORDER_STYLE: Style = Style::new().fg(SURFACE1);
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
