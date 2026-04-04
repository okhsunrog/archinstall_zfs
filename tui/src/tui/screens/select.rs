use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph,
};

use crate::tui::theme;

pub struct SelectResult {
    pub selected: Option<usize>,
}

pub struct MultiSelectResult {
    /// Indices of checked items. `None` means the user cancelled.
    pub selected: Option<Vec<usize>>,
}

/// Show a modal checklist and block until the user confirms or cancels.
///
/// `initially_checked` contains the indices that should start pre-checked.
/// Returns `selected: None` on cancel — callers treat this as "nothing chosen",
/// not as an error.
pub fn run_multiselect(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
    items: &[&str],
    initially_checked: &[usize],
) -> color_eyre::eyre::Result<MultiSelectResult> {
    let mut list_state = ListState::default().with_selected(Some(0));
    let mut checked: Vec<bool> = (0..items.len())
        .map(|i| initially_checked.contains(&i))
        .collect();

    loop {
        terminal.draw(|frame| render_multiselect(frame, title, items, &list_state, &checked))?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            let ev = crossterm::event::read()?;
            if let Event::Key(key) = ev {
                let selected = list_state.selected().unwrap_or(0);
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _)
                    | (KeyCode::Char('q'), _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(MultiSelectResult { selected: None });
                    }
                    (KeyCode::Enter, _) => {
                        let indices = checked
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| **c)
                            .map(|(i, _)| i)
                            .collect();
                        return Ok(MultiSelectResult {
                            selected: Some(indices),
                        });
                    }
                    (KeyCode::Up | KeyCode::Char('k'), _) => {
                        let i = if selected == 0 {
                            items.len() - 1
                        } else {
                            selected - 1
                        };
                        list_state.select(Some(i));
                    }
                    (KeyCode::Down | KeyCode::Char('j'), _) => {
                        let i = if selected >= items.len() - 1 {
                            0
                        } else {
                            selected + 1
                        };
                        list_state.select(Some(i));
                    }
                    (KeyCode::Char(' '), _) => {
                        if selected < checked.len() {
                            checked[selected] = !checked[selected];
                        }
                    }
                    (KeyCode::Char('a'), KeyModifiers::NONE) => {
                        checked.iter_mut().for_each(|c| *c = true);
                    }
                    (KeyCode::Char('A'), _) => {
                        checked.iter_mut().for_each(|c| *c = false);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn render_multiselect(
    frame: &mut Frame,
    title: &str,
    items: &[&str],
    list_state: &ListState,
    checked: &[bool],
) {
    let area = frame.area();
    let bg = Paragraph::new("").style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(bg, area);

    let popup_width = (items.iter().map(|s| s.len() + 8).max().unwrap_or(30) + 4).min(72) as u16;
    let popup_height = (items.len() as u16 + 4).min(area.height.saturating_sub(4));
    let popup = super::centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(format!(" {title} "))
        .title_style(theme::HEADER_STYLE)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::BORDER_STYLE)
        .style(theme::BG_STYLE);

    let cursor = list_state.selected().unwrap_or(0);
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (box_str, box_style) = if checked[i] {
                ("[✓] ", theme::VALUE_STYLE)
            } else {
                ("[ ] ", theme::DIMMED_STYLE)
            };
            let label_style = if i == cursor {
                theme::SELECTED_STYLE
            } else {
                theme::NORMAL_STYLE
            };
            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(box_str, box_style),
                Span::styled(*s, label_style),
            ]))
        })
        .collect();

    let list = List::new(list_items)
        .block(block)
        .highlight_style(theme::SELECTED_STYLE)
        .highlight_spacing(HighlightSpacing::Always);

    let mut state = *list_state;
    frame.render_stateful_widget(list, popup, &mut state);

    let footer_area = Rect::new(popup.x, popup.y + popup.height, popup.width, 1);
    if footer_area.y < area.height {
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(" Space", theme::ACCENT_STYLE),
            Span::styled(" toggle  ", theme::DIMMED_STYLE),
            Span::styled("a", theme::ACCENT_STYLE),
            Span::styled(" all  ", theme::DIMMED_STYLE),
            Span::styled("A", theme::ACCENT_STYLE),
            Span::styled(" none  ", theme::DIMMED_STYLE),
            Span::styled("Enter", theme::ACCENT_STYLE),
            Span::styled(" confirm  ", theme::DIMMED_STYLE),
            Span::styled("Esc", theme::ACCENT_STYLE),
            Span::styled(" cancel ", theme::DIMMED_STYLE),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(footer, footer_area);
    }
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
    let popup = super::centered_rect(popup_width, popup_height, area);

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
