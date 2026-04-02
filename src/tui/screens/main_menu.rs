use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::config::types::GlobalConfig;
use crate::tui::theme;

struct MenuItem {
    label: &'static str,
    value: String,
}

fn config_items(config: &GlobalConfig) -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: "Storage wizard",
            value: config
                .installation_mode
                .map(|m| m.to_string())
                .unwrap_or_else(|| "Not configured".to_string()),
        },
        MenuItem {
            label: "Pool name",
            value: config
                .pool_name
                .clone()
                .unwrap_or_else(|| "Not set".to_string()),
        },
        MenuItem {
            label: "Dataset prefix",
            value: config.dataset_prefix.clone(),
        },
        MenuItem {
            label: "Encryption",
            value: config.zfs_encryption_mode.to_string(),
        },
        MenuItem {
            label: "Compression",
            value: config.compression.to_string(),
        },
        MenuItem {
            label: "Swap",
            value: config.swap_mode.to_string(),
        },
        MenuItem {
            label: "Init system",
            value: config.init_system.to_string(),
        },
        MenuItem {
            label: "Kernels",
            value: config.effective_kernels().join(", "),
        },
        MenuItem {
            label: "Hostname",
            value: config
                .hostname
                .clone()
                .unwrap_or_else(|| "Not set".to_string()),
        },
        MenuItem {
            label: "Locale",
            value: config
                .locale
                .clone()
                .unwrap_or_else(|| "Not set".to_string()),
        },
        MenuItem {
            label: "Timezone",
            value: config
                .timezone
                .clone()
                .unwrap_or_else(|| "Not set".to_string()),
        },
        MenuItem {
            label: "Profile",
            value: config
                .profile
                .clone()
                .unwrap_or_else(|| "Not set".to_string()),
        },
        MenuItem {
            label: "Audio",
            value: config
                .audio
                .map(|a| a.to_string())
                .unwrap_or_else(|| "None".to_string()),
        },
        MenuItem {
            label: "Additional packages",
            value: if config.additional_packages.is_empty() {
                "None".to_string()
            } else {
                format!("{} packages", config.additional_packages.len())
            },
        },
        MenuItem {
            label: "AUR packages",
            value: if config.aur_packages.is_empty() {
                "None".to_string()
            } else {
                format!("{} packages", config.aur_packages.len())
            },
        },
        MenuItem {
            label: "zrepl",
            value: if config.zrepl_enabled {
                "Enabled".to_string()
            } else {
                "Disabled".to_string()
            },
        },
    ]
}

pub fn render(frame: &mut Frame, config: &GlobalConfig) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    // Title
    let title = Paragraph::new(Line::from(vec![Span::styled(
        " archinstall-zfs ",
        theme::TITLE_STYLE,
    )]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .style(theme::BORDER_STYLE),
    );
    frame.render_widget(title, chunks[0]);

    // Menu items
    let items = config_items(config);
    let menu_block = Block::default()
        .title(" Configuration ")
        .title_style(theme::HEADER_STYLE)
        .borders(Borders::ALL)
        .style(theme::BORDER_STYLE);

    let inner = menu_block.inner(chunks[1]);
    frame.render_widget(menu_block, chunks[1]);

    let item_height = 1u16;
    for (i, item) in items.iter().enumerate() {
        let y = inner.y + i as u16 * item_height;
        if y >= inner.y + inner.height {
            break;
        }

        let line_area = Rect::new(inner.x, y, inner.width, 1);
        let style = if item.value.contains("Not") || item.value == "None" {
            theme::UNSET_STYLE
        } else {
            theme::VALUE_STYLE
        };

        let line = Line::from(vec![
            Span::styled(format!("  {:<22}", item.label), theme::NORMAL_STYLE),
            Span::styled(&item.value, style),
        ]);
        frame.render_widget(Paragraph::new(line), line_area);
    }

    // Footer
    let footer = Paragraph::new(Line::from(vec![Span::styled(
        " q: quit | Enter: select | i: install ",
        theme::DIMMED_STYLE,
    )]))
    .alignment(Alignment::Center);
    frame.render_widget(footer, chunks[2]);
}
