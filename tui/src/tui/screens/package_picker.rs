use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph,
};

use crate::tui::theme;

use archinstall_zfs_core::packages::PackageInfo;

/// Result of the package picker: updated lists of repo and AUR packages.
pub struct PackagePickerResult {
    pub repo_packages: Vec<String>,
    pub aur_packages: Vec<String>,
}

enum Focus {
    Search,
    Results,
}

/// Interactive package search and selection.
/// Searches official repos first (via alpm), with option to search AUR.
pub async fn run_package_picker(
    terminal: &mut ratatui::DefaultTerminal,
    initial_repo: &[String],
    initial_aur: &[String],
) -> color_eyre::eyre::Result<Option<PackagePickerResult>> {
    let mut search_text = String::new();
    let mut results: Vec<PackageInfo> = Vec::new();
    let mut list_state = ListState::default();
    let mut focus = Focus::Search;
    let mut searching_aur = false;
    let mut status_msg = String::new();

    // Selected packages (repo and AUR tracked separately)
    let mut selected_repo: Vec<String> = initial_repo.to_vec();
    let mut selected_aur: Vec<String> = initial_aur.to_vec();

    loop {
        let all_selected: Vec<(&str, &str)> = selected_repo
            .iter()
            .map(|s| (s.as_str(), "repo"))
            .chain(selected_aur.iter().map(|s| (s.as_str(), "aur")))
            .collect();

        terminal.draw(|frame| {
            render_picker(
                frame,
                &search_text,
                &results,
                &mut list_state,
                &all_selected,
                &focus,
                searching_aur,
                &status_msg,
            );
        })?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            let ev = crossterm::event::read()?;
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match (&focus, key.code, key.modifiers) {
                    // Global: Esc to cancel, Ctrl+C to cancel
                    (_, KeyCode::Esc, _) | (_, KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(None);
                    }

                    // Search mode
                    (Focus::Search, KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                        search_text.push(c);
                        status_msg.clear();
                        // Search repo
                        results = do_repo_search(&search_text).await;
                        searching_aur = false;
                        list_state.select(if results.is_empty() { None } else { Some(0) });
                    }
                    (Focus::Search, KeyCode::Backspace, _) => {
                        search_text.pop();
                        status_msg.clear();
                        if search_text.is_empty() {
                            results.clear();
                            list_state.select(None);
                        } else {
                            results = do_repo_search(&search_text).await;
                            searching_aur = false;
                            list_state.select(if results.is_empty() { None } else { Some(0) });
                        }
                    }
                    // Tab: search AUR instead
                    (Focus::Search, KeyCode::Tab, _) if !search_text.is_empty() => {
                        status_msg = "Searching AUR...".to_string();
                        // Render the status before blocking on HTTP
                        terminal.draw(|frame| {
                            render_picker(
                                frame,
                                &search_text,
                                &results,
                                &mut list_state,
                                &all_selected,
                                &focus,
                                true,
                                &status_msg,
                            );
                        })?;
                        match archinstall_zfs_core::packages::search_aur(&search_text, 20).await {
                            Ok(aur_results) => {
                                results = aur_results;
                                searching_aur = true;
                                status_msg.clear();
                            }
                            Err(e) => {
                                status_msg = format!("AUR error: {e}");
                                results.clear();
                            }
                        }
                        list_state.select(if results.is_empty() { None } else { Some(0) });
                    }
                    // Down arrow: move to results
                    (Focus::Search, KeyCode::Down, _) if !results.is_empty() => {
                        focus = Focus::Results;
                        if list_state.selected().is_none() {
                            list_state.select(Some(0));
                        }
                    }
                    // Enter in search: add typed text directly as package
                    (Focus::Search, KeyCode::Enter, _) if !search_text.is_empty() => {
                        // If there are results and top one matches, add it
                        if let Some(0) = list_state.selected()
                            && let Some(pkg) = results.first()
                        {
                            add_package(pkg, &mut selected_repo, &mut selected_aur);
                            search_text.clear();
                            results.clear();
                            list_state.select(None);
                            continue;
                        }
                    }
                    // Ctrl+D: done
                    (_, KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                        return Ok(Some(PackagePickerResult {
                            repo_packages: selected_repo,
                            aur_packages: selected_aur,
                        }));
                    }

                    // Results mode
                    (Focus::Results, KeyCode::Up, _) => {
                        let i = list_state.selected().unwrap_or(0);
                        if i == 0 {
                            focus = Focus::Search;
                        } else {
                            list_state.select(Some(i - 1));
                        }
                    }
                    (Focus::Results, KeyCode::Down, _) => {
                        let i = list_state.selected().unwrap_or(0);
                        if i < results.len().saturating_sub(1) {
                            list_state.select(Some(i + 1));
                        }
                    }
                    (Focus::Results, KeyCode::Enter, _) => {
                        if let Some(idx) = list_state.selected()
                            && let Some(pkg) = results.get(idx)
                        {
                            add_package(pkg, &mut selected_repo, &mut selected_aur);
                            search_text.clear();
                            results.clear();
                            list_state.select(None);
                            focus = Focus::Search;
                        }
                    }

                    // Delete selected packages with Ctrl+X
                    (_, KeyCode::Char('x'), KeyModifiers::CONTROL)
                        if !selected_repo.is_empty() || !selected_aur.is_empty() =>
                    {
                        // Remove last added
                        if !selected_aur.is_empty() {
                            selected_aur.pop();
                        } else {
                            selected_repo.pop();
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}

fn add_package(pkg: &PackageInfo, repo: &mut Vec<String>, aur: &mut Vec<String>) {
    let name = &pkg.name;
    // Don't add duplicates
    if repo.iter().any(|s| s == name) || aur.iter().any(|s| s == name) {
        return;
    }
    if pkg.repo == "aur" {
        aur.push(name.clone());
    } else {
        repo.push(name.clone());
    }
}

async fn do_repo_search(query: &str) -> Vec<PackageInfo> {
    archinstall_zfs_core::packages::search_repo(query, 20)
        .await
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn render_picker(
    frame: &mut Frame,
    search: &str,
    results: &[PackageInfo],
    state: &mut ListState,
    selected: &[(&str, &str)],
    focus: &Focus,
    searching_aur: bool,
    status: &str,
) {
    let area = frame.area();
    let bg = Paragraph::new("").style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(bg, area);

    // Full-width popup
    let popup_width = area.width.saturating_sub(4).min(80);
    let popup_height = area.height.saturating_sub(4);
    let popup = super::centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup);

    // Layout: title(1) + search(3) + results(flexible) + selected(variable) + footer(1)
    let selected_height = if selected.is_empty() {
        0
    } else {
        (selected.len() as u16 + 2).min(6)
    };

    let chunks = Layout::vertical([
        Constraint::Length(3),               // search input
        Constraint::Min(3),                  // results
        Constraint::Length(selected_height), // selected packages
    ])
    .split(popup);

    // Search input
    let search_border_style = match focus {
        Focus::Search => theme::ACCENT_STYLE,
        Focus::Results => theme::BORDER_STYLE,
    };
    let source_label = if searching_aur { " AUR " } else { " Packages " };
    let search_block = Block::default()
        .title(source_label)
        .title_style(theme::HEADER_STYLE)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(search_border_style)
        .style(theme::BG_STYLE);

    let search_display = if search.is_empty() {
        Span::styled(" type to search...", theme::DIMMED_STYLE)
    } else {
        Span::styled(format!(" {search}\u{258f}"), theme::ACCENT_STYLE)
    };
    let search_widget = Paragraph::new(Line::from(vec![search_display])).block(search_block);
    frame.render_widget(search_widget, chunks[0]);

    // Results list
    let results_block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(theme::BORDER_STYLE)
        .style(theme::BG_STYLE);

    if results.is_empty() && !search.is_empty() {
        let msg = if !status.is_empty() {
            status.to_string()
        } else {
            "No results. Press Tab to search AUR.".to_string()
        };
        let empty = Paragraph::new(Line::from(Span::styled(
            format!("  {msg}"),
            theme::DIMMED_STYLE,
        )))
        .block(results_block);
        frame.render_widget(empty, chunks[1]);
    } else {
        let list_items: Vec<ListItem> = results
            .iter()
            .map(|pkg| {
                let badge = format!("[{}]", pkg.repo);
                let desc = if pkg.description.len() > 40 {
                    format!("{}...", &pkg.description[..40])
                } else {
                    pkg.description.clone()
                };
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(&pkg.name, theme::NORMAL_STYLE),
                    Span::styled(format!(" {badge} "), theme::DIMMED_STYLE),
                    Span::styled(desc, theme::DIMMED_STYLE),
                ]))
            })
            .collect();

        let list = List::new(list_items)
            .block(results_block)
            .highlight_style(theme::SELECTED_STYLE)
            .highlight_symbol(" \u{25b8} ")
            .highlight_spacing(HighlightSpacing::Always);

        frame.render_stateful_widget(list, chunks[1], state);
    }

    // Selected packages
    if !selected.is_empty() {
        let sel_block = Block::default()
            .title(" Selected ")
            .title_style(theme::HEADER_STYLE)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::BORDER_STYLE)
            .style(theme::BG_STYLE);

        let sel_text: Vec<Span> = selected
            .iter()
            .flat_map(|(name, source)| {
                vec![
                    Span::styled(*name, theme::VALUE_STYLE),
                    Span::styled(format!("[{source}] "), theme::DIMMED_STYLE),
                ]
            })
            .collect();

        let sel_paragraph = Paragraph::new(Line::from(sel_text))
            .block(sel_block)
            .wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(sel_paragraph, chunks[2]);
    }

    // Footer
    let footer_area = Rect::new(popup.x, popup.y + popup.height, popup.width, 1);
    if footer_area.y < area.height {
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(" Enter", theme::ACCENT_STYLE),
            Span::styled(" add  ", theme::DIMMED_STYLE),
            Span::styled("Tab", theme::ACCENT_STYLE),
            Span::styled(" AUR  ", theme::DIMMED_STYLE),
            Span::styled("Ctrl+X", theme::ACCENT_STYLE),
            Span::styled(" remove  ", theme::DIMMED_STYLE),
            Span::styled("Ctrl+D", theme::ACCENT_STYLE),
            Span::styled(" done  ", theme::DIMMED_STYLE),
            Span::styled("Esc", theme::ACCENT_STYLE),
            Span::styled(" cancel", theme::DIMMED_STYLE),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(footer, footer_area);
    }
}
