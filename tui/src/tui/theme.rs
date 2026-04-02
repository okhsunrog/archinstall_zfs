use ratatui::style::{Color, Modifier, Style};

// ── Catppuccin Mocha palette ────────────────────────
const BASE: Color = Color::Rgb(30, 30, 46);
const SURFACE0: Color = Color::Rgb(49, 50, 68);
const SURFACE1: Color = Color::Rgb(69, 71, 90);
const OVERLAY0: Color = Color::Rgb(108, 112, 134);
const TEXT: Color = Color::Rgb(205, 214, 244);
const SUBTEXT0: Color = Color::Rgb(166, 173, 200);
const BLUE: Color = Color::Rgb(137, 180, 250);
const GREEN: Color = Color::Rgb(166, 227, 161);
const RED: Color = Color::Rgb(243, 139, 168);
const YELLOW: Color = Color::Rgb(249, 226, 175);
const MAUVE: Color = Color::Rgb(203, 166, 247);
const TEAL: Color = Color::Rgb(148, 226, 213);
const PEACH: Color = Color::Rgb(250, 179, 135);
const LAVENDER: Color = Color::Rgb(180, 190, 254);

// ── Semantic styles ─────────────────────────────────
pub const TITLE_STYLE: Style = Style::new().fg(MAUVE).add_modifier(Modifier::BOLD);
pub const SELECTED_STYLE: Style = Style::new().fg(BASE).bg(BLUE);
pub const NORMAL_STYLE: Style = Style::new().fg(TEXT);
pub const DIMMED_STYLE: Style = Style::new().fg(OVERLAY0);
pub const ERROR_STYLE: Style = Style::new().fg(RED).add_modifier(Modifier::BOLD);
pub const SUCCESS_STYLE: Style = Style::new().fg(GREEN).add_modifier(Modifier::BOLD);
pub const BORDER_STYLE: Style = Style::new().fg(SURFACE1);
pub const HEADER_STYLE: Style = Style::new().fg(LAVENDER).add_modifier(Modifier::BOLD);
pub const VALUE_STYLE: Style = Style::new().fg(GREEN);
pub const UNSET_STYLE: Style = Style::new().fg(OVERLAY0);
pub const SECTION_STYLE: Style = Style::new().fg(MAUVE).add_modifier(Modifier::BOLD);
pub const KEY_STYLE: Style = Style::new().fg(SUBTEXT0);
pub const ACCENT_STYLE: Style = Style::new().fg(TEAL);
pub const WARN_STYLE: Style = Style::new().fg(YELLOW);
pub const ACTION_STYLE: Style = Style::new().fg(PEACH).add_modifier(Modifier::BOLD);
pub const BG_STYLE: Style = Style::new().bg(BASE).fg(TEXT);

// ── Status icons ────────────────────────────────────
pub const ICON_SET: &str = "\u{25cf}";    // ● filled circle (set)
pub const ICON_UNSET: &str = "\u{25cb}";  // ○ empty circle (unset)
pub const ICON_ARROW: &str = "\u{25b8}";  // ▸ arrow (selected)
pub const ICON_CHECK: &str = "\u{2713}";  // ✓ check
pub const ICON_WARN: &str = "\u{26a0}";   // ⚠ warning
