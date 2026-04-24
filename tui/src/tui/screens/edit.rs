use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::theme;

pub struct EditResult {
    pub value: Option<String>,
}

/// Show a modal text input and block until the user confirms or cancels.
/// If `mask` is true, input is displayed as asterisks and a zxcvbn strength
/// bar is rendered below the field.
pub fn run_edit(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
    initial: &str,
    mask: bool,
) -> color_eyre::eyre::Result<EditResult> {
    let mut value = initial.to_string();
    // cursor is a *char* index (not byte offset)
    let mut cursor = value.chars().count();

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
                        let byte_pos = char_to_byte(&value, cursor);
                        value.insert(byte_pos, c);
                        cursor += 1;
                    }
                    (KeyCode::Backspace, _) if cursor > 0 => {
                        cursor -= 1;
                        let byte_pos = char_to_byte(&value, cursor);
                        value.remove(byte_pos);
                    }
                    (KeyCode::Delete, _) => {
                        let char_count = value.chars().count();
                        if cursor < char_count {
                            let byte_pos = char_to_byte(&value, cursor);
                            value.remove(byte_pos);
                        }
                    }
                    (KeyCode::Left, _) => {
                        cursor = cursor.saturating_sub(1);
                    }
                    (KeyCode::Right, _) => {
                        cursor = (cursor + 1).min(value.chars().count());
                    }
                    (KeyCode::Home, _) => cursor = 0,
                    (KeyCode::End, _) => cursor = value.chars().count(),
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

/// Convert a char index to a byte offset in `s`.
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte, _)| byte)
        .unwrap_or(s.len())
}

fn render_edit(frame: &mut Frame, title: &str, value: &str, cursor: usize, mask: bool) {
    let area = frame.area();

    // Dim background
    let bg = ratatui::widgets::Paragraph::new("")
        .style(ratatui::style::Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(bg, area);

    // Password mode: taller popup to fit the strength indicator (2 extra rows).
    let popup_width = 54u16.min(area.width - 4);
    let popup_height = if mask { 7u16 } else { 5u16 };
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
    // cursor is a char index — convert to byte offset for splitting
    let display: String = if mask {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };

    let byte_cursor = char_to_byte(&display, cursor);
    let (before, after) = display.split_at(byte_cursor);
    let cursor_char = after.chars().next().unwrap_or(' ');
    let rest = if after.len() > cursor_char.len_utf8() {
        &after[cursor_char.len_utf8()..]
    } else {
        ""
    };

    let input_line = Line::from(vec![
        Span::styled(before, theme::NORMAL_STYLE),
        Span::styled(
            cursor_char.to_string(),
            theme::NORMAL_STYLE.add_modifier(Modifier::REVERSED),
        ),
        Span::styled(rest, theme::NORMAL_STYLE),
    ]);

    // Layout: gap | input | [strength bar] | [feedback] | gap
    let constraints = if mask {
        vec![
            Constraint::Length(1), // top gap
            Constraint::Length(1), // input
            Constraint::Length(1), // strength bar
            Constraint::Length(1), // feedback text
            Constraint::Fill(1),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ]
    };

    let areas = Layout::vertical(constraints).split(inner);

    let input_area = Rect::new(
        areas[1].x + 1,
        areas[1].y,
        areas[1].width.saturating_sub(2),
        1,
    );
    frame.render_widget(Paragraph::new(input_line), input_area);

    if mask {
        let (bar_line, feedback_line) = strength_widgets(value, areas[2].width as usize);
        frame.render_widget(Paragraph::new(bar_line), areas[2]);
        frame.render_widget(Paragraph::new(feedback_line), areas[3]);
    }

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

/// Build the strength bar and feedback line for `password`.
/// Returns `(bar_line, feedback_line)`.
fn strength_widgets(password: &str, width: usize) -> (Line<'static>, Line<'static>) {
    if password.is_empty() {
        let empty = Line::from(Span::styled(" Type a password…", theme::DIMMED_STYLE));
        return (empty.clone(), Line::default());
    }

    let entropy = zxcvbn::zxcvbn(password, &[]);
    let score = entropy.score(); // Score: 0-4

    let (label, bar_color) = match u8::from(score) {
        0 => ("Very weak", Color::Red),
        1 => ("Weak    ", Color::LightRed),
        2 => ("Fair    ", Color::Yellow),
        3 => ("Strong  ", Color::LightGreen),
        _ => ("Very strong", Color::Green),
    };

    // Bar: 4 filled + 4 empty blocks scaled to available width.
    let bar_width = (width.saturating_sub(label.len() + 4)).min(20);
    let filled = ((u8::from(score) as usize + 1) * bar_width) / 5;
    let empty = bar_width - filled;
    let bar_str = "█".repeat(filled) + &"░".repeat(empty);

    let crack_time = entropy
        .crack_times()
        .online_no_throttling_10_per_second()
        .to_string();

    let bar_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(bar_str, Style::default().fg(bar_color)),
        Span::raw("  "),
        Span::styled(
            label,
            Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
        ),
    ]);

    // Feedback: show the first suggestion if available, else crack time.
    let feedback_text = entropy
        .feedback()
        .and_then(|f| f.suggestions().first().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("~{crack_time} to crack"));

    let feedback_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(feedback_text, theme::DIMMED_STYLE),
    ]);

    (bar_line, feedback_line)
}

use super::centered_rect;
