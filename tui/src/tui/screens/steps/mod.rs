pub mod desktop;
pub mod disk;
pub mod review;
pub mod system;
pub mod users;
pub mod welcome;
pub mod zfs;

use archinstall_zfs_core::config::choices::Choice;
use archinstall_zfs_core::config::edit::ChoiceSetting;
use archinstall_zfs_core::config::types::GlobalConfig;

// ── Shared types for all wizard steps ──────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepId {
    Welcome = 0,
    Disk = 1,
    Zfs = 2,
    System = 3,
    Users = 4,
    Desktop = 5,
    Review = 6,
}

impl StepId {
    pub const ALL: [StepId; 7] = [
        StepId::Welcome,
        StepId::Disk,
        StepId::Zfs,
        StepId::System,
        StepId::Users,
        StepId::Desktop,
        StepId::Review,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StepId::Welcome => "Welcome",
            StepId::Disk => "Disk",
            StepId::Zfs => "ZFS",
            StepId::System => "System",
            StepId::Users => "Users",
            StepId::Desktop => "Desktop",
            StepId::Review => "Review",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<StepId> {
        StepId::ALL.get(i).copied()
    }

    pub fn next(self) -> Option<StepId> {
        StepId::from_index(self.index() + 1)
    }

    pub fn prev(self) -> Option<StepId> {
        if self.index() == 0 {
            None
        } else {
            StepId::from_index(self.index() - 1)
        }
    }
}

#[derive(Clone)]
pub enum MenuKind {
    /// Section header with label, or empty separator (not selectable)
    SectionHeader,
    /// Inline radio group header (not selectable, groups RadioOption items below)
    RadioHeader,
    /// Inline radio option (selectable, carries group key + index)
    RadioOption {
        group_key: &'static str,
        index: usize,
        selected: bool,
    },
    /// Select from a list via popup (for long lists: locale, timezone, kernel)
    #[allow(dead_code)]
    Select {
        options: Vec<&'static str>,
        current: usize,
    },
    /// Free-form text input
    Text,
    /// Masked text input (password)
    Password,
    /// Boolean toggle
    Toggle,
    /// Custom handler (disk, timezone, locale, profile — shows value)
    Custom,
    /// Action button (install, quit — no value shown)
    Action,
}

#[derive(Clone)]
pub struct MenuItem {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
    pub kind: MenuKind,
}

impl MenuItem {
    pub fn new(
        key: &'static str,
        label: &'static str,
        value: impl Into<String>,
        kind: MenuKind,
    ) -> Self {
        Self {
            key,
            label,
            value: value.into(),
            kind,
        }
    }

    /// Row edited through a custom handler; shows `value`.
    pub fn custom(key: &'static str, label: &'static str, value: impl Into<String>) -> Self {
        Self::new(key, label, value, MenuKind::Custom)
    }

    /// Free-form text input row.
    pub fn text(key: &'static str, label: &'static str, value: impl Into<String>) -> Self {
        Self::new(key, label, value, MenuKind::Text)
    }

    /// Boolean toggle row, rendered as `Enabled` / `Disabled`.
    pub fn toggle(key: &'static str, label: &'static str, enabled: bool) -> Self {
        let value = if enabled { "Enabled" } else { "Disabled" };
        Self::new(key, label, value, MenuKind::Toggle)
    }

    /// Masked input row that only reveals whether a value is present.
    pub fn secret(key: &'static str, label: &'static str, is_set: bool) -> Self {
        let value = if is_set { "Set" } else { "Not set" };
        Self::new(key, label, value, MenuKind::Password)
    }

    /// Non-selectable header, separator or read-only summary line.
    pub fn header(key: &'static str, label: &'static str, value: impl Into<String>) -> Self {
        Self::new(key, label, value, MenuKind::SectionHeader)
    }

    /// Action button without a value.
    pub fn action(key: &'static str, label: &'static str) -> Self {
        Self::new(key, label, String::new(), MenuKind::Action)
    }

    pub fn is_selectable(&self) -> bool {
        !matches!(self.kind, MenuKind::SectionHeader | MenuKind::RadioHeader)
    }
}

/// Menu rows for one wizard step.
pub fn items_for(step: StepId, config: &GlobalConfig) -> Vec<MenuItem> {
    match step {
        StepId::Welcome => welcome::items(config),
        StepId::Disk => disk::items(config),
        StepId::Zfs => zfs::items(config),
        StepId::System => system::items(config),
        StepId::Users => users::items(config),
        StepId::Desktop => desktop::items(config),
        StepId::Review => review::items(config),
    }
}

/// Helper: emit a radio group header + options as a flat list of MenuItems.
/// Build a radio group from a [`Choice`] enum, so the order, the labels and
/// the selected index all come from one table rather than being spelled out
/// here and inverted again in `pickers::apply_select`.
pub fn choice_group<T: Choice>(
    setting: ChoiceSetting,
    label: &'static str,
    current: T,
) -> Vec<MenuItem> {
    radio_group(setting.as_str(), label, &T::labels(), current.index())
}

pub fn radio_group(
    key: &'static str,
    label: &'static str,
    options: &[&'static str],
    current: usize,
) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::new(
        key,
        label,
        String::new(),
        MenuKind::RadioHeader,
    )];
    for (i, &opt) in options.iter().enumerate() {
        items.push(MenuItem::new(
            key,
            opt,
            String::new(),
            MenuKind::RadioOption {
                group_key: key,
                index: i,
                selected: i == current,
            },
        ));
    }
    items
}
