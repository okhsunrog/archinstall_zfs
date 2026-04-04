use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Gauge, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use tracing_subscriber::layer::SubscriberExt;

use archinstall_zfs_core::config::types::GlobalConfig;
use archinstall_zfs_core::system::async_download::{DownloadProgress, PackageState};

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
    rx: mpsc::UnboundedReceiver<(String, i32)>,
    download_rx: watch::Receiver<DownloadProgress>,
    cancel: CancellationToken,
    min_level: i32,
}

impl InstallProgress {
    pub fn start(config: GlobalConfig) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();

        // Create download progress channel — sender goes to download engine, receiver stays here
        let (download_tx, download_rx) = watch::channel(DownloadProgress {
            packages: vec![],
            total_bytes: 0,
            downloaded_bytes: 0,
            active_downloads: 0,
            completed: 0,
            failed: 0,
        });
        let download_tx = Arc::new(download_tx);

        let tx_clone = tx.clone();
        let cancel_clone = cancel.clone();
        let download_tx_clone = download_tx.clone();
        tokio::spawn(async move {
            let channel_layer = ChannelLayer::new(tx_clone.clone());
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

            let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
                Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
            let result =
                crate::app::run_install(runner, config, cancel_clone, Some(download_tx_clone))
                    .await;

            match result {
                Ok(()) => {
                    let _ = tx.send(("[INFO ] Installation complete!".to_string(), 2));
                }
                Err(e) => {
                    let _ = tx.send((format!("[ERROR] {e}"), 4));
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
            download_rx,
            cancel,
            min_level: 2,
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
            let filtered_count = self.filtered_lines().len();
            self.scroll = filtered_count.saturating_sub(1);
        }
    }

    pub fn is_done(&self) -> bool {
        !matches!(self.state, InstallState::Running)
    }

    fn is_downloading(&self) -> bool {
        let progress = self.download_rx.borrow();
        progress.total_bytes > 0
            && (progress.active_downloads > 0
                || (progress.completed + progress.failed < progress.packages.len()
                    && !progress.packages.is_empty()))
    }

    pub fn handle_event(&mut self, ev: Event) -> bool {
        if let Event::Key(key) = ev {
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    if self.is_done() {
                        return true;
                    }
                }
                (KeyCode::Esc, _) => {
                    if !self.is_done() {
                        tracing::warn!("cancellation requested by user");
                        self.cancel.cancel();
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
                (KeyCode::Char('l'), _) => {
                    self.min_level = match self.min_level {
                        0 => 2,
                        2 => 1,
                        1 => 0,
                        _ => 2,
                    };
                    let max = self.filtered_lines().len().saturating_sub(1);
                    self.scroll = self.scroll.min(max);
                }
                _ => {}
            }
        }
        false
    }

    pub fn render(&self, frame: &mut Frame) {
        use ratatui::widgets::BorderType;

        frame.render_widget(Block::default().style(theme::BG_STYLE), frame.area());

        let area = frame.area();
        let is_downloading = self.is_downloading();

        // Layout: title | log | [download progress] | footer
        let constraints = if is_downloading {
            vec![
                Constraint::Length(3), // title
                Constraint::Min(5),    // log
                Constraint::Length(8), // download progress
                Constraint::Length(1), // footer
            ]
        } else {
            vec![
                Constraint::Length(3), // title
                Constraint::Min(0),    // log
                Constraint::Length(1), // footer
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        // Title
        let (title_text, title_style) = match &self.state {
            InstallState::Running => (" Installing... ".to_string(), theme::TITLE_STYLE),
            InstallState::Succeeded => (
                " \u{2713} Installation Complete ".to_string(),
                theme::SUCCESS_STYLE,
            ),
            InstallState::Failed(err) => (format!(" \u{26a0} Failed: {err} "), theme::ERROR_STYLE),
        };
        let title = Paragraph::new(Line::from(vec![Span::styled(&title_text, title_style)]))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_type(BorderType::Rounded)
                    .border_style(theme::BORDER_STYLE),
            );
        frame.render_widget(title, chunks[0]);

        // Log area
        let level_name = LEVEL_NAMES.get(self.min_level as usize).unwrap_or(&"?");
        let log_block = Block::default()
            .title(format!(" Log [{level_name}+] "))
            .title_style(theme::HEADER_STYLE)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::BORDER_STYLE);
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
                3 => theme::WARN_STYLE,
                2 => {
                    if entry.text.contains("Phase ") {
                        theme::SECTION_STYLE
                    } else if entry.text.contains("complete") || entry.text.contains("Complete") {
                        theme::SUCCESS_STYLE
                    } else {
                        theme::NORMAL_STYLE
                    }
                }
                1 => theme::DIMMED_STYLE,
                _ => theme::DIMMED_STYLE,
            };
            let line_area = Rect::new(inner.x, y, inner.width, 1);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(entry.text.as_str(), style))),
                line_area,
            );
        }

        if total > visible_height {
            let mut scrollbar_state = ScrollbarState::new(total).position(start);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                chunks[1],
                &mut scrollbar_state,
            );
        }

        // Download progress panel (only shown during active downloads)
        if is_downloading {
            let dl_chunk = chunks[2];
            self.render_download_progress(frame, dl_chunk);
        }

        // Footer
        let footer_chunk = if is_downloading { chunks[3] } else { chunks[2] };
        let footer = if self.is_done() {
            Line::from(vec![
                Span::styled(" Enter/q", theme::ACCENT_STYLE),
                Span::styled(" exit  ", theme::DIMMED_STYLE),
                Span::styled("l", theme::ACCENT_STYLE),
                Span::styled(format!(" log level ({level_name}+) "), theme::DIMMED_STYLE),
            ])
        } else {
            Line::from(vec![
                Span::styled(" j/k", theme::ACCENT_STYLE),
                Span::styled(" scroll  ", theme::DIMMED_STYLE),
                Span::styled("Esc", theme::ACCENT_STYLE),
                Span::styled(" cancel  ", theme::DIMMED_STYLE),
                Span::styled("l", theme::ACCENT_STYLE),
                Span::styled(format!(" log level ({level_name}+) "), theme::DIMMED_STYLE),
            ])
        };
        frame.render_widget(
            Paragraph::new(footer).alignment(Alignment::Center),
            footer_chunk,
        );
    }

    fn render_download_progress(&self, frame: &mut Frame, area: Rect) {
        use ratatui::widgets::BorderType;

        let progress = self.download_rx.borrow();

        // Overall progress bar
        let pct = if progress.total_bytes > 0 {
            (progress.downloaded_bytes as f64 / progress.total_bytes as f64 * 100.0) as u16
        } else {
            0
        };

        let speed = progress.total_speed_bps();
        let speed_str = format_speed(speed);
        let eta_str = progress
            .eta()
            .map(format_duration)
            .unwrap_or_else(|| "--:--".to_string());

        let title = format!(
            " Downloads {}/{} | {} | ETA {} ",
            progress.completed,
            progress.packages.len(),
            speed_str,
            eta_str,
        );

        let dl_block = Block::default()
            .title(title)
            .title_style(theme::HEADER_STYLE)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::BORDER_STYLE);
        let inner = dl_block.inner(area);
        frame.render_widget(dl_block, area);

        if inner.height == 0 {
            return;
        }

        // Overall gauge on first line
        let gauge_area = Rect::new(inner.x, inner.y, inner.width, 1);
        let overall_label = format!(
            "{} / {} ({}%)",
            format_bytes(progress.downloaded_bytes),
            format_bytes(progress.total_bytes),
            pct
        );
        let gauge = Gauge::default()
            .gauge_style(theme::ACCENT_STYLE)
            .ratio(progress.downloaded_bytes as f64 / progress.total_bytes.max(1) as f64)
            .label(overall_label);
        frame.render_widget(gauge, gauge_area);

        // Per-package status lines (active downloads)
        let mut y = inner.y + 1;
        for pkg in &progress.packages {
            if y >= inner.y + inner.height {
                break;
            }
            match pkg {
                PackageState::Downloading {
                    filename,
                    downloaded,
                    total,
                    speed_bps,
                    ..
                } => {
                    let name = truncate_filename(filename, 30);
                    let pkg_pct = if *total > 0 {
                        (*downloaded as f64 / *total as f64 * 100.0) as u64
                    } else {
                        0
                    };
                    let line = format!("  {} {}% {}", name, pkg_pct, format_speed(*speed_bps),);
                    let line_area = Rect::new(inner.x, y, inner.width, 1);
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(line, theme::NORMAL_STYLE))),
                        line_area,
                    );
                    y += 1;
                }
                PackageState::Verifying { filename } => {
                    let name = truncate_filename(filename, 30);
                    let line = format!("  {} verifying SHA256...", name);
                    let line_area = Rect::new(inner.x, y, inner.width, 1);
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(line, theme::DIMMED_STYLE))),
                        line_area,
                    );
                    y += 1;
                }
                _ => {}
            }
        }
    }
}

fn format_speed(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("{:.1} MB/s", bps as f64 / 1_000_000.0)
    } else if bps >= 1_000 {
        format!("{:.0} KB/s", bps as f64 / 1_000.0)
    } else if bps > 0 {
        format!("{bps} B/s")
    } else {
        "-- B/s".to_string()
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

fn truncate_filename(name: &str, max: usize) -> String {
    if name.len() <= max {
        format!("{:width$}", name, width = max)
    } else {
        format!("{}...", &name[..max - 3])
    }
}
