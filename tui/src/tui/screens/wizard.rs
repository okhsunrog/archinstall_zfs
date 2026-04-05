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
    step_cursors: [usize; 7],
    step_scrolls: [usize; 7],
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
            MenuKind::RadioOption {
                group_key, index, ..
            } => {
                pickers::apply_select(&mut self.config, group_key, *index, terminal)?;
            }
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
                    let current = self.config.locale.as_deref().unwrap_or("");
                    if let Some(loc) = pickers::pick_locale(terminal, current)? {
                        self.config.locale = Some(loc);
                    }
                }
                "keyboard" => {
                    if let Some(km) = pickers::pick_keyboard(terminal, &self.config.keyboard_layout)?
                    {
                        self.config.keyboard_layout = km;
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
                    pickers::pick_profile(&mut self.config, terminal)?;
                }
                "display_manager" => {
                    let eff_dm = self.config.display_manager_override.clone().or_else(|| {
                        self.config
                            .profile
                            .as_deref()
                            .and_then(archinstall_zfs_core::profile::get_profile)
                            .and_then(|p| p.display_manager().map(str::to_string))
                    });
                    if let Some(result) =
                        pickers::pick_display_manager(terminal, eff_dm.as_deref())?
                    {
                        self.config.display_manager_override = result;
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
            MenuKind::SectionHeader | MenuKind::RadioHeader => {}
        }
        Ok(Action::Continue)
    }

    // ── Render ───────────────────────────────────────────

    pub fn render(&self, frame: &mut Frame) {
        frame.render_widget(Block::default().style(theme::BG_STYLE), frame.area());

        let area = frame.area();
        let items = self.items();

        // Vertical: title | body | footer
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_title(frame, v_chunks[0]);

        // Body: sidebar | separator | content
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(18),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(v_chunks[1]);

        self.render_sidebar(frame, h_chunks[0]);

        // Vertical separator line
        let sep = Block::default()
            .borders(Borders::LEFT)
            .border_type(BorderType::Plain)
            .border_style(theme::BORDER_STYLE);
        frame.render_widget(sep, h_chunks[1]);

        self.render_content(frame, h_chunks[2], &items);
        self.render_footer(frame, v_chunks[2]);
    }

    fn render_title(&self, frame: &mut Frame, area: Rect) {
        let step_idx = self.current_step.index();

        let title = Paragraph::new(Line::from(vec![
            Span::styled(" archinstall", theme::TITLE_STYLE),
            Span::styled("-zfs", theme::ACCENT_STYLE),
            Span::raw("  "),
            Span::styled(
                format!("step {} of {}", step_idx + 1, StepId::ALL.len()),
                theme::DIMMED_STYLE,
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_type(BorderType::Plain)
                .border_style(theme::BORDER_STYLE),
        );
        frame.render_widget(title, area);
    }

    fn render_sidebar(&self, frame: &mut Frame, area: Rect) {
        // Fill sidebar background
        frame.render_widget(Block::default().style(theme::SIDEBAR_BG), area);

        let inner = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );

        for (i, step) in StepId::ALL.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }
            let y = inner.y + i as u16;
            let line_area = Rect::new(inner.x, y, inner.width, 1);

            let is_current = *step == self.current_step;
            let is_visited = i <= self.max_visited;

            let (icon, style) = if is_current {
                ("\u{25b8}", theme::SIDEBAR_CURRENT) // ▸
            } else if is_visited {
                ("\u{2713}", theme::SIDEBAR_DONE) // ✓
            } else {
                ("\u{00b7}", theme::SIDEBAR_PENDING) // ·
            };

            // Full-line background for current step
            if is_current {
                frame.render_widget(Block::default().style(theme::SIDEBAR_CURRENT), line_area);
            }

            let line = Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(format!("{} ", step.label()), style),
            ]);
            frame.render_widget(Paragraph::new(line), line_area);
        }
    }

    fn render_content(&self, frame: &mut Frame, area: Rect, items: &[MenuItem]) {
        let step = self.current_step;

        // Step title inside content area
        let title_area = Rect::new(area.x, area.y, area.width, 2);
        let title_line = Line::from(vec![
            Span::styled("  ", theme::NORMAL_STYLE),
            Span::styled(step.label(), theme::SECTION_STYLE),
        ]);
        frame.render_widget(Paragraph::new(title_line), title_area);

        let content_area = Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        );

        let visible_height = content_area.height as usize;
        let total_items = items.len();
        let cursor = self.cursor();

        // Scrolling
        let mut scroll = self.scroll();
        if cursor >= scroll + visible_height {
            scroll = cursor - visible_height + 1;
        }
        if cursor < scroll {
            scroll = cursor;
        }

        for (vi, item) in items.iter().enumerate().skip(scroll).take(visible_height) {
            let y = content_area.y + (vi - scroll) as u16;
            let line_area = Rect::new(content_area.x, y, content_area.width, 1);
            let is_selected = vi == cursor;

            // Full-line highlight for selected item
            if is_selected && item.is_selectable() {
                frame.render_widget(Block::default().style(theme::HOVER_BG), line_area);
            }

            match &item.kind {
                MenuKind::SectionHeader => {
                    self.render_section_header(frame, line_area, item);
                }
                MenuKind::RadioHeader => {
                    let line = Line::from(vec![
                        Span::styled("  ", theme::NORMAL_STYLE),
                        Span::styled(item.label, theme::LABEL_STYLE),
                    ]);
                    frame.render_widget(Paragraph::new(line), line_area);
                }
                MenuKind::RadioOption { selected, .. } => {
                    let (icon, icon_style) = if *selected {
                        ("\u{25cf}", theme::RADIO_SELECTED) // ●
                    } else {
                        ("\u{25cb}", theme::RADIO_UNSELECTED) // ○
                    };
                    let label_style = if is_selected {
                        theme::SELECTED_STYLE
                    } else if *selected {
                        theme::VALUE_STYLE
                    } else {
                        theme::NORMAL_STYLE
                    };
                    let line = Line::from(vec![
                        Span::styled("    ", theme::NORMAL_STYLE),
                        Span::styled(
                            format!("{icon} "),
                            if is_selected {
                                theme::SELECTED_STYLE
                            } else {
                                icon_style
                            },
                        ),
                        Span::styled(item.label, label_style),
                    ]);
                    frame.render_widget(Paragraph::new(line), line_area);
                }
                MenuKind::Toggle => {
                    let is_on = item.value == "Enabled";
                    let (indicator, ind_style) = if is_on {
                        ("[ON] ", theme::TOGGLE_ON)
                    } else {
                        ("[OFF]", theme::TOGGLE_OFF)
                    };
                    let label_style = if is_selected {
                        theme::SELECTED_STYLE
                    } else {
                        theme::NORMAL_STYLE
                    };
                    let line = Line::from(vec![
                        Span::styled("  ", theme::NORMAL_STYLE),
                        Span::styled(
                            indicator,
                            if is_selected {
                                theme::SELECTED_STYLE
                            } else {
                                ind_style
                            },
                        ),
                        Span::styled(format!(" {}", item.label), label_style),
                    ]);
                    frame.render_widget(Paragraph::new(line), line_area);
                }
                MenuKind::Action => {
                    let style = if is_selected {
                        theme::SELECTED_STYLE
                    } else {
                        theme::ACTION_STYLE
                    };
                    let line = Line::from(vec![
                        Span::styled("  ", theme::NORMAL_STYLE),
                        Span::styled(format!("\u{25b8} {}", item.label), style),
                    ]);
                    frame.render_widget(Paragraph::new(line), line_area);
                }
                MenuKind::Password => {
                    self.render_kv_item(frame, line_area, item, is_selected, "\u{1f512} ");
                }
                MenuKind::Text => {
                    self.render_kv_item(frame, line_area, item, is_selected, "\u{270e} ");
                }
                MenuKind::Custom => {
                    self.render_kv_item(frame, line_area, item, is_selected, "");
                }
                MenuKind::Select { .. } => {
                    self.render_kv_item(frame, line_area, item, is_selected, "\u{25bc} ");
                }
            }
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

    /// Render a key-value item: `  icon label          value`
    fn render_kv_item(
        &self,
        frame: &mut Frame,
        area: Rect,
        item: &MenuItem,
        is_selected: bool,
        icon: &str,
    ) {
        let is_unset =
            item.value.contains("Not set") || item.value == "None" || item.value.is_empty();

        let label_style = if is_selected {
            theme::SELECTED_STYLE
        } else {
            theme::NORMAL_STYLE
        };

        let value_style = if is_selected {
            theme::SELECTED_VALUE_STYLE
        } else if is_unset {
            theme::UNSET_STYLE
        } else {
            theme::VALUE_STYLE
        };

        let icon_style = if is_selected {
            theme::SELECTED_STYLE
        } else {
            theme::DIMMED_STYLE
        };

        // Calculate padding between label and value
        let icon_width = icon.chars().count();
        let label_width = item.label.chars().count();
        let value_width = item.value.chars().count();
        let available =
            (area.width as usize).saturating_sub(2 + icon_width + label_width + value_width + 2);
        let padding = " ".repeat(available);

        let line = Line::from(vec![
            Span::styled("  ", theme::NORMAL_STYLE),
            Span::styled(icon, icon_style),
            Span::styled(item.label, label_style),
            Span::styled(padding, label_style),
            Span::styled(format!("{} ", &item.value), value_style),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_section_header(&self, frame: &mut Frame, area: Rect, item: &MenuItem) {
        if item.label.is_empty() && item.value.is_empty() {
            // Empty separator
            let sep = Paragraph::new(Line::from(Span::styled(
                "\u{2500}".repeat(area.width as usize),
                theme::BORDER_STYLE,
            )));
            frame.render_widget(sep, area);
        } else if !item.label.is_empty() && item.value.is_empty() {
            // Section header
            let line = Line::from(vec![
                Span::styled(" \u{2500}\u{2500} ", theme::BORDER_STYLE),
                Span::styled(item.label, theme::SECTION_STYLE),
                Span::styled(
                    format!(
                        " {}",
                        "\u{2500}"
                            .repeat((area.width as usize).saturating_sub(item.label.len() + 5))
                    ),
                    theme::BORDER_STYLE,
                ),
            ]);
            frame.render_widget(Paragraph::new(line), area);
        } else if item.label.is_empty() && !item.value.is_empty() {
            // Error/warning line
            let line = Line::from(vec![
                Span::styled("  \u{26a0} ", theme::WARN_STYLE),
                Span::styled(&item.value, theme::WARN_STYLE),
            ]);
            frame.render_widget(Paragraph::new(line), area);
        } else {
            // Summary line (review step): label + value
            let is_unset =
                item.value.contains("Not set") || item.value == "None" || item.value.is_empty();
            let value_style = if is_unset {
                theme::UNSET_STYLE
            } else {
                theme::VALUE_STYLE
            };
            let label_width = item.label.chars().count();
            let value_width = item.value.chars().count();
            let available = (area.width as usize).saturating_sub(4 + label_width + value_width + 2);
            let padding = " ".repeat(available);

            let line = Line::from(vec![
                Span::styled("    ", theme::NORMAL_STYLE),
                Span::styled(item.label, theme::LABEL_STYLE),
                Span::styled(padding, theme::NORMAL_STYLE),
                Span::styled(format!("{} ", &item.value), value_style),
            ]);
            frame.render_widget(Paragraph::new(line), area);
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let step = self.current_step;
        let mut spans = vec![];

        spans.extend([
            Span::styled(" \u{2191}\u{2193}", theme::ACCENT_STYLE),
            Span::styled(" nav ", theme::DIMMED_STYLE),
            Span::styled("\u{23ce}", theme::ACCENT_STYLE),
            Span::styled(" select ", theme::DIMMED_STYLE),
        ]);

        if step != StepId::Welcome {
            spans.extend([
                Span::styled("S-Tab", theme::ACCENT_STYLE),
                Span::styled(" back ", theme::DIMMED_STYLE),
            ]);
        }
        if step != StepId::Review {
            spans.extend([
                Span::styled("Tab", theme::ACCENT_STYLE),
                Span::styled(" next ", theme::DIMMED_STYLE),
            ]);
        }

        spans.extend([
            Span::styled("1-7", theme::ACCENT_STYLE),
            Span::styled(" jump ", theme::DIMMED_STYLE),
            Span::styled("q", theme::ACCENT_STYLE),
            Span::styled(" quit", theme::DIMMED_STYLE),
        ]);

        let footer = Paragraph::new(Line::from(spans)).alignment(Alignment::Center);
        frame.render_widget(footer, area);
    }
}
