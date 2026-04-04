pub mod edit;
pub mod install_progress;
pub mod pickers;
pub mod select;
pub mod steps;
pub mod wifi;
pub mod wizard;

use ratatui::layout::{Constraint, Layout, Rect};

/// Center a rectangle of given dimensions within `area`.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let [_, v, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, h, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .areas(v);
    h
}
