pub mod event;
pub mod screens;
pub mod theme;
pub mod widgets;

use color_eyre::eyre::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::DefaultTerminal;
use ratatui::prelude::CrosstermBackend;

use archinstall_zfs_core::config::types::GlobalConfig;

pub fn run_tui(config: GlobalConfig, dry_run: bool) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, config, dry_run);
    restore_terminal()?;
    result
}

fn setup_terminal() -> Result<DefaultTerminal> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = ratatui::Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

fn run_app(terminal: &mut DefaultTerminal, config: GlobalConfig, _dry_run: bool) -> Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};

    let config = config;

    loop {
        terminal.draw(|frame| {
            screens::main_menu::render(frame, &config);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {
                        // TODO: dispatch to screens
                    }
                }
            }
        }
    }
}
