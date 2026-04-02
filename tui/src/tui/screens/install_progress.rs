use std::sync::mpsc;
use std::thread;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use tracing_subscriber::layer::SubscriberExt;

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::tui::theme;
use crate::tui::tracing_layer::ChannelLayer;

enum InstallState {
    Running,
    Succeeded,
    Failed(String),
}

struct LogEntry {
    text: String,
    level: i32, // 0=trace, 1=debug, 2=info, 3=warn, 4=error
}

const LEVEL_NAMES: &[&str] = &["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

pub struct InstallProgress {
    log_entries: Vec<LogEntry>,
    scroll: usize,
    state: InstallState,
    rx: mpsc::Receiver<(String, i32)>,
    min_level: i32, // minimum level to display (default 2=info)
}

impl InstallProgress {
    pub fn start(config: GlobalConfig) -> Self {
        let (tx, rx) = mpsc::channel();

        let tx_clone = tx.clone();
        thread::spawn(move || {
            let channel_layer = ChannelLayer::new(tx_clone.clone());
            // Capture all levels including trace (command output)
            let filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("trace"));
            let file_appender = tracing_appender::rolling::never("/tmp", "archinstall-zfs.log");
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(file_appender)
                .with_ansi(false)
                .with_target(true);
            let subscriber = tracing_subscriber::registry()
                .with(filter)
                .with(file_layer)
                .with(channel_layer);
            let _guard = tracing::subscriber::set_default(subscriber);

            let runner = archinstall_zfs_core::system::cmd::RealRunner;
            let result = crate::app::run_install(&runner, &config);
            match result {
                Ok(()) => {
                    let _ = tx_clone.send(("[INFO ] Installation complete!".to_string(), 2));
                }
                Err(e) => {
                    let _ = tx_clone.send((format!("[ERROR] {e}"), 4));
                }
            }
        });

        Self {
            log_entries: vec![LogEntry {
                text: "[INFO ] Starting installation...".to_string(),
                level: 2,
            }],
            scroll: 0,
            state: InstallState::Running,
            rx,
            min_level: 2, // show info+ by default
        }
    }

    fn filtered_lines(&self) -> Vec<&LogEntry> {
        self.log_entries
            .iter()
            .filter(|e| e.level >= self.min_level)
            .collect()
    }

    pub fn tick(&mut self) {
        while let Ok((text, level)) = self.rx.try_recv() {
            if text.contains("[INFO ] Installation complete!") {
                self.state = InstallState::Succeeded;
            } else if text.starts_with("[ERROR]") {
                let err = text.strip_prefix("[ERROR] ").unwrap_or(&text).to_string();
                self.state = InstallState::Failed(err);
            }
            self.log_entries.push(LogEntry { text, level });
            // Auto-scroll to bottom if viewing filtered list
            let filtered_count = self.filtered_lines().len();
            self.scroll = filtered_count.saturating_sub(1);
        }
    }

    pub fn is_done(&self) -> bool {
        !matches!(self.state, InstallState::Running)
    }

    pub fn handle_event(&mut self, ev: Event) -> bool {
        if let Event::Key(key) = ev {
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    if self.is_done() {
                        return true;
                    }
                }
                (KeyCode::Enter, _) if self.is_done() => return true,
                (KeyCode::Up | KeyCode::Char('k'), _) => {
                    self.scroll = self.scroll.saturating_sub(1);
                }
                (KeyCode::Down | KeyCode::Char('j'), _) => {
                    let max = self.filtered_lines().len().saturating_sub(1);
                    self.scroll = (self.scroll + 1).min(max);
                }
                (KeyCode::Home, _) => self.scroll = 0,
                (KeyCode::End, _) => {
                    self.scroll = self.filtered_lines().len().saturating_sub(1);
                }
                // Toggle log level with 'l'
                (KeyCode::Char('l'), _) => {
                    self.min_level = match self.min_level {
                        0 => 2, // trace -> info
                        2 => 1, // info -> debug
                        1 => 0, // debug -> trace
                        _ => 2,
                    };
                    // Re-clamp scroll
                    let max = self.filtered_lines().len().saturating_sub(1);
                    self.scroll = self.scroll.min(max);
                }
                _ => {}
            }
        }
        false
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(area);

        // Title
        let (title_text, title_style) = match &self.state {
            InstallState::Running => (" Installing... ", theme::TITLE_STYLE),
            InstallState::Succeeded => (" Installation Complete ", theme::SUCCESS_STYLE),
            InstallState::Failed(_) => (" Installation Failed ", theme::ERROR_STYLE),
        };
        let title = Paragraph::new(Line::from(vec![Span::styled(title_text, title_style)]))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .style(theme::BORDER_STYLE),
            );
        frame.render_widget(title, chunks[0]);

        // Log area
        let level_name = LEVEL_NAMES.get(self.min_level as usize).unwrap_or(&"?");
        let log_block = Block::default()
            .title(format!(" Log [{level_name}+] "))
            .title_style(theme::HEADER_STYLE)
            .borders(Borders::ALL)
            .style(theme::BORDER_STYLE);
        let inner = log_block.inner(chunks[1]);
        frame.render_widget(log_block, chunks[1]);

        let filtered = self.filtered_lines();
        let visible_height = inner.height as usize;
        let total = filtered.len();

        let start = if self.scroll + visible_height > total {
            total.saturating_sub(visible_height)
        } else {
            self.scroll
        };

        for (i, entry) in filtered.iter().skip(start).take(visible_height).enumerate() {
            let y = inner.y + i as u16;
            let style = match entry.level {
                4 => theme::ERROR_STYLE,
                3 => theme::ERROR_STYLE,
                2 => {
                    if entry.text.contains("Phase ") {
                        theme::HEADER_STYLE
                    } else if entry.text.contains("complete") {
                        theme::SUCCESS_STYLE
                    } else {
                        theme::NORMAL_STYLE
                    }
                }
                1 => theme::DIMMED_STYLE,
                _ => theme::DIMMED_STYLE, // trace
            };
            let line_area = ratatui::layout::Rect::new(inner.x, y, inner.width, 1);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(entry.text.as_str(), style))),
                line_area,
            );
        }

        // Scrollbar
        if total > visible_height {
            let mut scrollbar_state = ScrollbarState::new(total).position(start);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                chunks[1],
                &mut scrollbar_state,
            );
        }

        // Footer
        let footer_text = if self.is_done() {
            format!(" Enter/q: exit | l: log level ({level_name}+) ")
        } else {
            format!(" j/k: scroll | l: log level ({level_name}+) ")
        };
        let footer = Paragraph::new(Line::from(vec![Span::styled(
            footer_text,
            theme::DIMMED_STYLE,
        )]))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .style(theme::BORDER_STYLE),
        );
        frame.render_widget(footer, chunks[2]);
    }
}
