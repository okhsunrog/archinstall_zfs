use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::theme;

pub struct EditResult {
    pub value: Option<String>,
}

/// Show a modal text input and block until the user confirms or cancels.
/// If `mask` is true, input is displayed as asterisks (for passwords).
pub fn run_edit(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
    initial: &str,
    mask: bool,
) -> color_eyre::eyre::Result<EditResult> {
    let mut value = initial.to_string();
    let mut cursor = value.len();

    loop {
        terminal.draw(|frame| {
            render_edit(frame, title, &value, cursor, mask);
        })?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            let ev = crossterm::event::read()?;
            if let Event::Key(key) = ev {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(EditResult { value: None });
                    }
                    (KeyCode::Enter, _) => {
                        return Ok(EditResult { value: Some(value) });
                    }
                    (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                        value.insert(cursor, c);
                        cursor += 1;
                    }
                    (KeyCode::Backspace, _) => {
                        if cursor > 0 {
                            cursor -= 1;
                            value.remove(cursor);
                        }
                    }
                    (KeyCode::Delete, _) => {
                        if cursor < value.len() {
                            value.remove(cursor);
                        }
                    }
                    (KeyCode::Left, _) => {
                        cursor = cursor.saturating_sub(1);
                    }
                    (KeyCode::Right, _) => {
                        cursor = (cursor + 1).min(value.len());
                    }
                    (KeyCode::Home, _) => cursor = 0,
                    (KeyCode::End, _) => cursor = value.len(),
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                        value.clear();
                        cursor = 0;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn render_edit(frame: &mut Frame, title: &str, value: &str, cursor: usize, mask: bool) {
    let area = frame.area();

    // Dim background
    let bg = ratatui::widgets::Paragraph::new("")
        .style(ratatui::style::Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(bg, area);

    // Center popup
    let popup_width = 50u16.min(area.width - 4);
    let popup_height = 5u16;
    let popup = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(format!(" {title} "))
        .title_style(theme::HEADER_STYLE)
        .borders(Borders::ALL)
        .border_style(theme::BORDER_STYLE);

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Input line with cursor
    let display: String = if mask {
        "*".repeat(value.len())
    } else {
        value.to_string()
    };

    // Split at cursor position for visual cursor
    let (before, after) = display.split_at(cursor.min(display.len()));
    let cursor_char = after.chars().next().unwrap_or(' ');
    let rest = if after.len() > 1 {
        &after[cursor_char.len_utf8()..]
    } else {
        ""
    };

    let line = Line::from(vec![
        Span::styled(before, theme::NORMAL_STYLE),
        Span::styled(
            cursor_char.to_string(),
            theme::NORMAL_STYLE.add_modifier(Modifier::REVERSED),
        ),
        Span::styled(rest, theme::NORMAL_STYLE),
    ]);

    // Position input in the center of the popup
    let [_, input_area, _] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);

    let input_area = Rect::new(
        input_area.x + 1,
        input_area.y,
        input_area.width.saturating_sub(2),
        1,
    );
    frame.render_widget(Paragraph::new(line), input_area);

    // Footer hint
    let footer_area = Rect::new(popup.x, popup.y + popup.height, popup.width, 1);
    if footer_area.y < area.height {
        let footer = Paragraph::new(Line::from(vec![Span::styled(
            " Enter: confirm | Esc: cancel | Ctrl+U: clear ",
            theme::DIMMED_STYLE,
        )]))
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(footer, footer_area);
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
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
