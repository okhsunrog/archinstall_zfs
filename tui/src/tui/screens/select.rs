use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph,
};

use crate::tui::theme;

pub struct SelectResult {
    pub selected: Option<usize>,
}

/// Show a modal select list and block until the user picks an item or cancels.
pub fn run_select(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
    items: &[&str],
    initial: usize,
) -> color_eyre::eyre::Result<SelectResult> {
    let mut state = ListState::default().with_selected(Some(initial));

    loop {
        terminal.draw(|frame| {
            render_select(frame, title, items, &mut state);
        })?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            let ev = crossterm::event::read()?;
            if let Event::Key(key) = ev {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _)
                    | (KeyCode::Char('q'), _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(SelectResult { selected: None });
                    }
                    (KeyCode::Enter, _) => {
                        return Ok(SelectResult {
                            selected: state.selected(),
                        });
                    }
                    (KeyCode::Up | KeyCode::Char('k'), _) => {
                        let i = state.selected().unwrap_or(0);
                        state.select(Some(if i == 0 { items.len() - 1 } else { i - 1 }));
                    }
                    (KeyCode::Down | KeyCode::Char('j'), _) => {
                        let i = state.selected().unwrap_or(0);
                        state.select(Some(if i >= items.len() - 1 { 0 } else { i + 1 }));
                    }
                    _ => {}
                }
            }
        }
    }
}

fn render_select(frame: &mut Frame, title: &str, items: &[&str], state: &mut ListState) {
    use ratatui::widgets::BorderType;

    // Dim background
    let area = frame.area();
    let bg = Paragraph::new("").style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(bg, area);

    // Center popup
    let popup_width = (items.iter().map(|s| s.len()).max().unwrap_or(20) + 8).min(70) as u16;
    let popup_height = (items.len() as u16 + 4).min(area.height - 4);
    let popup = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(format!(" {title} "))
        .title_style(theme::HEADER_STYLE)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::BORDER_STYLE)
        .style(theme::BG_STYLE);

    let list_items: Vec<ListItem> = items
        .iter()
        .map(|s| ListItem::new(Line::from(format!("  {s}"))))
        .collect();

    let list = List::new(list_items)
        .block(block)
        .highlight_style(theme::SELECTED_STYLE)
        .highlight_symbol(" \u{25b8} ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(list, popup, state);

    // Footer
    let footer_area = Rect::new(popup.x, popup.y + popup.height, popup.width, 1);
    if footer_area.y < area.height {
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(" Enter", theme::ACCENT_STYLE),
            Span::styled(" select  ", theme::DIMMED_STYLE),
            Span::styled("Esc", theme::ACCENT_STYLE),
            Span::styled(" cancel ", theme::DIMMED_STYLE),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(footer, footer_area);
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let [_, v_center, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, h_center, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .areas(v_center);
    h_center
}
