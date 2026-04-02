use ratatui::style::{Color, Modifier, Style};

pub const TITLE_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const SELECTED_STYLE: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
pub const NORMAL_STYLE: Style = Style::new().fg(Color::White);
pub const DIMMED_STYLE: Style = Style::new().fg(Color::DarkGray);
pub const ERROR_STYLE: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
pub const SUCCESS_STYLE: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const BORDER_STYLE: Style = Style::new().fg(Color::DarkGray);
pub const HEADER_STYLE: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const VALUE_STYLE: Style = Style::new().fg(Color::Green);
pub const UNSET_STYLE: Style = Style::new().fg(Color::DarkGray);
