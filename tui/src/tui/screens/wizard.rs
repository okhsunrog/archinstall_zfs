use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::tui::Action;
use crate::tui::theme;

use super::edit::run_edit;
use super::pickers;
use super::select::run_select;
use super::steps::{MenuItem, MenuKind, StepId};

// ── Wizard state ────────────────────────────────────

pub struct Wizard {
    config: GlobalConfig,
    current_step: StepId,
    /// Per-step cursor positions
    step_cursors: [usize; 7],
    /// Per-step scroll offsets
    step_scrolls: [usize; 7],
    /// Highest step index visited (for jump-back gating)
    max_visited: usize,
}

impl Wizard {
    pub fn new(config: GlobalConfig) -> Self {
        Self {
            config,
            current_step: StepId::Welcome,
            step_cursors: [0; 7],
            step_scrolls: [0; 7],
            max_visited: 0,
        }
    }

    pub fn into_config(self) -> GlobalConfig {
        self.config
    }

    fn items(&self) -> Vec<MenuItem> {
        match self.current_step {
            StepId::Welcome => super::steps::welcome::items(&self.config),
            StepId::Disk => super::steps::disk::items(&self.config),
            StepId::Zfs => super::steps::zfs::items(&self.config),
            StepId::System => super::steps::system::items(&self.config),
            StepId::Users => super::steps::users::items(&self.config),
            StepId::Desktop => super::steps::desktop::items(&self.config),
            StepId::Review => super::steps::review::items(&self.config),
        }
    }

    fn selectable_indices(&self) -> Vec<usize> {
        self.items()
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_selectable())
            .map(|(i, _)| i)
            .collect()
    }

    fn cursor(&self) -> usize {
        self.step_cursors[self.current_step.index()]
    }

    fn set_cursor(&mut self, val: usize) {
        self.step_cursors[self.current_step.index()] = val;
    }

    fn scroll(&self) -> usize {
        self.step_scrolls[self.current_step.index()]
    }

    fn move_up(&mut self) {
        let indices = self.selectable_indices();
        if indices.is_empty() {
            return;
        }
        let cursor = self.cursor();
        if let Some(pos) = indices.iter().position(|&i| i == cursor) {
            let new_pos = if pos == 0 { indices.len() - 1 } else { pos - 1 };
            self.set_cursor(indices[new_pos]);
        } else if let Some(&last) = indices.last() {
            self.set_cursor(last);
        }
    }

    fn move_down(&mut self) {
        let indices = self.selectable_indices();
        if indices.is_empty() {
            return;
        }
        let cursor = self.cursor();
        if let Some(pos) = indices.iter().position(|&i| i == cursor) {
            let new_pos = if pos >= indices.len() - 1 { 0 } else { pos + 1 };
            self.set_cursor(indices[new_pos]);
        } else if let Some(&first) = indices.first() {
            self.set_cursor(first);
        }
    }

    fn go_to_step(&mut self, step: StepId) {
        self.current_step = step;
        if step.index() > self.max_visited {
            self.max_visited = step.index();
        }
        // Ensure cursor is on a selectable item
        let indices = self.selectable_indices();
        let cursor = self.cursor();
        if !indices.contains(&cursor)
            && let Some(&first) = indices.first()
        {
            self.set_cursor(first);
        }
    }

    fn next_step(&mut self) {
        if let Some(next) = self.current_step.next() {
            self.go_to_step(next);
        }
    }

    fn prev_step(&mut self) {
        if let Some(prev) = self.current_step.prev() {
            self.go_to_step(prev);
        }
    }

    // ── Event handling ──────────────────────────────────

    pub async fn handle_event(
        &mut self,
        ev: Event,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Action> {
        if let Event::Key(key) = ev {
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    return Ok(Action::Quit);
                }
                (KeyCode::Up | KeyCode::Char('k'), _) => self.move_up(),
                (KeyCode::Down | KeyCode::Char('j'), _) => self.move_down(),
                (KeyCode::Enter | KeyCode::Right, _) => {
                    return self.activate_item(terminal).await;
                }
                (KeyCode::Tab, _) | (KeyCode::Char('n'), _) => self.next_step(),
                (KeyCode::BackTab, _) | (KeyCode::Char('p'), _) => self.prev_step(),
                (KeyCode::Home, _) => {
                    let indices = self.selectable_indices();
                    if let Some(&first) = indices.first() {
                        self.set_cursor(first);
                    }
                }
                (KeyCode::End, _) => {
                    let indices = self.selectable_indices();
                    if let Some(&last) = indices.last() {
                        self.set_cursor(last);
                    }
                }
                // Number keys 1-7 to jump to visited steps
                (KeyCode::Char(c @ '1'..='7'), _) => {
                    let target = (c as usize) - ('1' as usize);
                    if target <= self.max_visited
                        && let Some(step) = StepId::from_index(target)
                    {
                        self.go_to_step(step);
                    }
                }
                _ => {}
            }
        }
        Ok(Action::Continue)
    }

    async fn activate_item(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Action> {
        let items = self.items();
        let cursor = self.cursor();
        let Some(item) = items.get(cursor) else {
            return Ok(Action::Continue);
        };
        let key = item.key;

        match &item.kind {
            MenuKind::Action => match key {
                "install" => {
                    let errors = self.config.validate_for_install();
                    if !errors.is_empty() {
                        let msg = errors.join("\n");
                        let lines: Vec<&str> = msg.lines().collect();
                        let _ = run_select(terminal, "Validation errors", &lines, 0);
                        return Ok(Action::Continue);
                    }
                    return Ok(Action::Install);
                }
                "save" => {
                    let result = run_edit(terminal, "Save config to file", "config.json", false)?;
                    if let Some(path) = result.value
                        && !path.is_empty()
                    {
                        match self.config.save_to_file(std::path::Path::new(&path)) {
                            Ok(()) => {
                                let _ =
                                    run_select(terminal, &format!("Saved to {path}"), &["OK"], 0);
                            }
                            Err(e) => {
                                let msg = format!("Save failed: {e}");
                                let _ = run_select(terminal, &msg, &["OK"], 0);
                            }
                        }
                    }
                }
                "quit" => return Ok(Action::Quit),
                _ => {}
            },
            MenuKind::Custom => match key {
                "timezone" => {
                    if let Some(tz) = pickers::pick_timezone(terminal)? {
                        self.config.timezone = Some(tz);
                    }
                }
                "locale" => {
                    if let Some(loc) = pickers::pick_locale(terminal)? {
                        self.config.locale = Some(loc);
                    }
                }
                "disk_by_id" => {
                    if let Some(disk) = pickers::pick_disk(terminal)? {
                        self.config.disk_by_id = Some(disk);
                    }
                }
                "efi_partition" => {
                    if let Some(part) = pickers::pick_partition(terminal, "EFI partition")? {
                        self.config.efi_partition_by_id = Some(part);
                    }
                }
                "zfs_partition" => {
                    if let Some(part) = pickers::pick_partition(terminal, "ZFS partition")? {
                        self.config.zfs_partition_by_id = Some(part);
                    }
                }
                "swap_partition" => {
                    if let Some(part) = pickers::pick_partition(terminal, "Swap partition")? {
                        self.config.swap_partition_by_id = Some(part);
                    }
                }
                "kernel" => {
                    if let Some(kernels) = pickers::pick_kernel(&self.config, terminal).await? {
                        self.config.kernels = Some(kernels);
                    }
                }
                "profile" => {
                    if let Some(profile) = pickers::pick_profile(terminal)? {
                        self.config.profile = if profile.is_empty() {
                            None
                        } else {
                            Some(profile)
                        };
                    }
                }
                "gpu_driver" => {
                    if let Some(driver) = pickers::pick_gpu_driver(terminal)? {
                        self.config.gfx_driver = driver;
                    }
                }
                "users" => {
                    pickers::manage_users(&mut self.config, terminal)?;
                }
                "parallel_downloads" => {
                    if let Some(n) = pickers::pick_parallel_downloads(&self.config, terminal)? {
                        self.config.parallel_downloads = n;
                    }
                }
                "pool_name" => {
                    pickers::pick_existing_pool(&mut self.config, terminal)?;
                }
                _ => {}
            },
            MenuKind::Toggle => {
                pickers::apply_toggle(&mut self.config, key);
            }
            MenuKind::Select { options, current } => {
                let label = item.label;
                let result = run_select(terminal, label, options, *current)?;
                if let Some(idx) = result.selected {
                    pickers::apply_select(&mut self.config, key, idx, terminal)?;
                }
            }
            MenuKind::Text => {
                let current = &item.value;
                let initial = if current == "Not set" || current == "None" {
                    ""
                } else {
                    current
                };
                let label = item.label;
                let result = run_edit(terminal, label, initial, false)?;
                if let Some(val) = result.value {
                    pickers::apply_text(&mut self.config, key, &val);
                }
            }
            MenuKind::Password => {
                let label = item.label;
                let result = run_edit(terminal, label, "", true)?;
                if let Some(val) = result.value
                    && !val.is_empty()
                {
                    pickers::apply_text(&mut self.config, key, &val);
                }
            }
            MenuKind::SectionHeader => {}
        }
        Ok(Action::Continue)
    }

    // ── Render ───────────────────────────────────────────

    pub fn render(&self, frame: &mut Frame) {
        frame.render_widget(Block::default().style(theme::BG_STYLE), frame.area());

        let area = frame.area();
        let items = self.items();
        let step_idx = self.current_step.index();

        // Vertical: title | body | footer
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        // Title bar
        let title = Paragraph::new(Line::from(vec![
            Span::styled(" archinstall", theme::TITLE_STYLE),
            Span::styled("-zfs ", theme::ACCENT_STYLE),
            Span::raw("  "),
            Span::styled(
                format!("{} / {}", step_idx + 1, StepId::ALL.len()),
                theme::DIMMED_STYLE,
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_type(BorderType::Rounded)
                .border_style(theme::BORDER_STYLE),
        );
        frame.render_widget(title, v_chunks[0]);

        // Body: sidebar | content
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(14), Constraint::Min(0)])
            .split(v_chunks[1]);

        self.render_sidebar(frame, h_chunks[0]);
        self.render_content(frame, h_chunks[1], &items);

        // Footer
        self.render_footer(frame, v_chunks[2]);
    }

    fn render_sidebar(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_type(BorderType::Rounded)
            .border_style(theme::BORDER_STYLE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        for (i, step) in StepId::ALL.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }
            let y = inner.y + i as u16;
            let line_area = Rect::new(inner.x, y, inner.width, 1);

            let is_current = *step == self.current_step;
            let is_visited = i <= self.max_visited;

            let (icon, style) = if is_current {
                (theme::ICON_ARROW, theme::SIDEBAR_ACTIVE_STYLE)
            } else if is_visited {
                (theme::ICON_SET, theme::SIDEBAR_DONE_STYLE)
            } else {
                (theme::ICON_UNSET, theme::SIDEBAR_PENDING_STYLE)
            };

            let line = Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(step.label(), style),
            ]);
            frame.render_widget(Paragraph::new(line), line_area);
        }
    }

    fn render_content(&self, frame: &mut Frame, area: Rect, items: &[MenuItem]) {
        let step = self.current_step;

        // Content block with step title
        let block = Block::default()
            .title(format!(" {} ", step.label()))
            .title_style(theme::HEADER_STYLE)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::BORDER_STYLE);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let visible_height = inner.height as usize;
        let total_items = items.len();
        let cursor = self.cursor();

        // Adjust scroll
        let mut scroll = self.scroll();
        if cursor >= scroll + visible_height {
            scroll = cursor - visible_height + 1;
        }
        if cursor < scroll {
            scroll = cursor;
        }
        // Can't mutate self here during render, but we store it for next frame
        // (scroll is adjusted in handle_event via set_scroll)

        for (vi, item) in items.iter().enumerate().skip(scroll).take(visible_height) {
            let y = inner.y + (vi - scroll) as u16;
            let line_area = Rect::new(inner.x, y, inner.width, 1);

            if matches!(item.kind, MenuKind::SectionHeader) {
                // Review step: render summary items or section headers
                if item.label.is_empty() && item.value.is_empty() {
                    // Empty separator
                    let sep = Paragraph::new(Line::from(Span::styled(
                        "\u{2500}".repeat(inner.width as usize),
                        theme::BORDER_STYLE,
                    )));
                    frame.render_widget(sep, line_area);
                } else if !item.label.is_empty() && item.value.is_empty() {
                    // Section header
                    let label = format!(" {} ", item.label);
                    let pad_total = (inner.width as usize)
                        .saturating_sub(label.len())
                        .saturating_sub(4);
                    let pad_left = pad_total / 2;
                    let pad_right = pad_total - pad_left;
                    let line = Line::from(vec![
                        Span::styled(
                            format!("  {}\u{2500}", "\u{2500}".repeat(pad_left)),
                            theme::BORDER_STYLE,
                        ),
                        Span::styled(label, theme::SECTION_STYLE),
                        Span::styled(
                            format!("\u{2500}{}", "\u{2500}".repeat(pad_right)),
                            theme::BORDER_STYLE,
                        ),
                    ]);
                    frame.render_widget(Paragraph::new(line), line_area);
                } else if item.label.is_empty() && !item.value.is_empty() {
                    // Error line in review
                    let line = Line::from(vec![
                        Span::styled("   \u{26a0} ", theme::WARN_STYLE),
                        Span::styled(&item.value, theme::WARN_STYLE),
                    ]);
                    frame.render_widget(Paragraph::new(line), line_area);
                } else {
                    // Summary line in review (label + value, read-only)
                    let label_text = format!("{:<20}", item.label);
                    let value_text = &item.value;
                    let is_unset = value_text.contains("Not set")
                        || value_text == "None"
                        || value_text.is_empty();
                    let icon = if is_unset {
                        theme::ICON_UNSET
                    } else {
                        theme::ICON_SET
                    };
                    let icon_style = if is_unset {
                        theme::UNSET_STYLE
                    } else {
                        theme::VALUE_STYLE
                    };
                    let value_style = if is_unset {
                        theme::UNSET_STYLE
                    } else {
                        theme::VALUE_STYLE
                    };
                    let dots_len = (inner.width as usize)
                        .saturating_sub(3 + 1 + 20 + 2 + value_text.len() + 1);
                    let dots = ".".repeat(dots_len);
                    let line = Line::from(vec![
                        Span::raw("   "),
                        Span::styled(format!("{icon} "), icon_style),
                        Span::styled(label_text, theme::NORMAL_STYLE),
                        Span::styled(dots, theme::DIMMED_STYLE),
                        Span::styled(format!(" {value_text} "), value_style),
                    ]);
                    frame.render_widget(Paragraph::new(line), line_area);
                }
                continue;
            }

            let is_selected = vi == cursor;
            let is_action = matches!(item.kind, MenuKind::Action);
            let is_unset =
                item.value.contains("Not set") || item.value == "None" || item.value.is_empty();

            let icon = if is_action {
                ""
            } else if is_unset {
                theme::ICON_UNSET
            } else {
                theme::ICON_SET
            };

            let icon_style = if is_selected {
                theme::SELECTED_STYLE
            } else if is_unset {
                theme::UNSET_STYLE
            } else {
                theme::VALUE_STYLE
            };

            let label_style = if is_selected {
                theme::SELECTED_STYLE
            } else {
                theme::NORMAL_STYLE
            };

            let value_style = if is_selected {
                theme::SELECTED_STYLE
            } else if is_action {
                theme::ACTION_STYLE
            } else if is_unset {
                theme::UNSET_STYLE
            } else {
                theme::VALUE_STYLE
            };

            let arrow = if is_selected {
                format!(" {} ", theme::ICON_ARROW)
            } else {
                "   ".to_string()
            };

            let line = if is_action {
                Line::from(vec![
                    Span::styled(&arrow, label_style),
                    Span::styled(item.label, value_style),
                ])
            } else {
                let label_text = format!("{:<20}", item.label);
                let value_text = &item.value;
                let dots_len =
                    (inner.width as usize).saturating_sub(3 + 1 + 20 + 2 + value_text.len() + 1);
                let dots = ".".repeat(dots_len);

                Line::from(vec![
                    Span::styled(&arrow, label_style),
                    Span::styled(format!("{icon} "), icon_style),
                    Span::styled(label_text, label_style),
                    Span::styled(dots, theme::DIMMED_STYLE),
                    Span::styled(format!(" {value_text} "), value_style),
                ])
            };

            frame.render_widget(Paragraph::new(line), line_area);
        }

        // Scrollbar
        if total_items > visible_height {
            let mut scrollbar_state = ScrollbarState::new(total_items).position(scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area,
                &mut scrollbar_state,
            );
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let step = self.current_step;
        let mut spans = vec![];

        // Navigation hints
        spans.extend([
            Span::styled(" j/k", theme::ACCENT_STYLE),
            Span::styled(" navigate  ", theme::DIMMED_STYLE),
            Span::styled("Enter", theme::ACCENT_STYLE),
            Span::styled(" edit  ", theme::DIMMED_STYLE),
        ]);

        if step != StepId::Welcome {
            spans.extend([
                Span::styled("Shift+Tab", theme::ACCENT_STYLE),
                Span::styled(" back  ", theme::DIMMED_STYLE),
            ]);
        }
        if step != StepId::Review {
            spans.extend([
                Span::styled("Tab", theme::ACCENT_STYLE),
                Span::styled(" next  ", theme::DIMMED_STYLE),
            ]);
        }

        spans.extend([
            Span::styled("1-7", theme::ACCENT_STYLE),
            Span::styled(" jump  ", theme::DIMMED_STYLE),
            Span::styled("q", theme::ACCENT_STYLE),
            Span::styled(" quit ", theme::DIMMED_STYLE),
        ]);

        let footer = Paragraph::new(Line::from(spans)).alignment(Alignment::Center);
        frame.render_widget(footer, area);
    }
}
