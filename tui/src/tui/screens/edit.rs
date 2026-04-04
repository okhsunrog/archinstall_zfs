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
    let display: String = if mask {
        "•".repeat(value.len())
    } else {
        value.to_string()
    };

    let (before, after) = display.split_at(cursor.min(display.len()));
    let cursor_char = after.chars().next().unwrap_or(' ');
    let rest = if after.len() > 1 {
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
