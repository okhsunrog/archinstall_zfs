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
use tracing_subscriber::util::SubscriberInitExt;

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::tui::theme;
use crate::tui::tracing_layer::ChannelLayer;

enum InstallState {
    Running,
    Succeeded,
    Failed(String),
}

pub struct InstallProgress {
    log_lines: Vec<String>,
    scroll: usize,
    state: InstallState,
    rx: mpsc::Receiver<String>,
}

impl InstallProgress {
    /// Start installation in a background thread, return the progress screen.
    /// Installs a tracing layer that captures all log events from core.
    pub fn start(config: GlobalConfig) -> Self {
        let (tx, rx) = mpsc::channel();

        // Add a channel-based tracing layer so all tracing::info!() etc.
        // from the core crate flow into our log display.
        let channel_layer = ChannelLayer::new(tx.clone());

        // We need to set a new global subscriber that includes our layer.
        // Since the file logger was set up in main(), we rebuild the stack
        // with both the file layer and our channel layer.
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        let file_appender = tracing_appender::rolling::never("/tmp", "archinstall-zfs.log");
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(file_appender)
            .with_ansi(false)
            .with_target(true);

        // Replace the global subscriber with one that has both layers
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(channel_layer);

        // Use try_init because a subscriber was already set in main().
        // If it fails (already set), the channel layer won't work — fall back
        // to a thread-local approach.
        let _guard = tracing::subscriber::set_default(subscriber);

        // Spawn the install thread. It uses the thread-local subscriber
        // we just set, but since set_default is scoped to this thread,
        // we need to pass the subscriber to the install thread instead.
        let tx_clone = tx.clone();
        thread::spawn(move || {
            // Set up a subscriber for this thread that includes the channel layer
            let channel_layer = ChannelLayer::new(tx_clone.clone());
            let filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
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
                    let _ = tx_clone.send("[OK] Installation complete!".to_string());
                }
                Err(e) => {
                    let _ = tx_clone.send(format!("[ERROR] {e}"));
                }
            }
        });

        Self {
            log_lines: vec!["Starting installation...".to_string()],
            scroll: 0,
            state: InstallState::Running,
            rx,
        }
    }

    pub fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            if msg.starts_with("[OK]") {
                self.state = InstallState::Succeeded;
            } else if msg.starts_with("[ERROR]") {
                let err = msg.strip_prefix("[ERROR] ").unwrap_or(&msg).to_string();
                self.state = InstallState::Failed(err);
            }
            self.log_lines.push(msg);
            // Auto-scroll to bottom
            self.scroll = self.log_lines.len().saturating_sub(1);
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
                    self.scroll = (self.scroll + 1).min(self.log_lines.len().saturating_sub(1));
                }
                (KeyCode::Home, _) => self.scroll = 0,
                (KeyCode::End, _) => {
                    self.scroll = self.log_lines.len().saturating_sub(1);
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
        let log_block = Block::default()
            .title(" Log ")
            .title_style(theme::HEADER_STYLE)
            .borders(Borders::ALL)
            .style(theme::BORDER_STYLE);
        let inner = log_block.inner(chunks[1]);
        frame.render_widget(log_block, chunks[1]);

        let visible_height = inner.height as usize;
        let total = self.log_lines.len();

        let start = if self.scroll + visible_height > total {
            total.saturating_sub(visible_height)
        } else {
            self.scroll
        };

        for (i, line) in self
            .log_lines
            .iter()
            .skip(start)
            .take(visible_height)
            .enumerate()
        {
            let y = inner.y + i as u16;
            let style = if line.starts_with("[OK]") {
                theme::SUCCESS_STYLE
            } else if line.starts_with("[ERROR]") || line.starts_with("[WARN]") {
                theme::ERROR_STYLE
            } else if line.starts_with("Phase ") {
                theme::HEADER_STYLE
            } else {
                theme::NORMAL_STYLE
            };
            let line_area = ratatui::layout::Rect::new(inner.x, y, inner.width, 1);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(line.as_str(), style))),
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
            " Press Enter or q to exit "
        } else {
            " j/k: scroll | Installation in progress... "
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
