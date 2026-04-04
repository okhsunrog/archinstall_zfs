pub mod desktop;
pub mod disk;
pub mod review;
pub mod system;
pub mod users;
pub mod welcome;
pub mod zfs;

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
    /// Select from a list of options
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
    pub fn is_selectable(&self) -> bool {
        !matches!(self.kind, MenuKind::SectionHeader)
    }
}
