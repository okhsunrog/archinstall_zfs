pub mod screens;
pub mod theme;
pub mod tracing_layer;
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

use self::screens::install_progress::InstallProgress;
use self::screens::main_menu::MainMenu;

pub fn run_tui(config: GlobalConfig, _dry_run: bool) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, config);
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

/// What the main menu wants the app to do next.
pub enum Action {
    Continue,
    Install,
    Quit,
}

fn run_app(terminal: &mut DefaultTerminal, config: GlobalConfig) -> Result<()> {
    let mut menu = MainMenu::new(config);

    loop {
        terminal.draw(|frame| menu.render(frame))?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            let ev = crossterm::event::read()?;
            match menu.handle_event(ev, terminal)? {
                Action::Continue => {}
                Action::Install => {
                    let config = menu.into_config();
                    run_install_screen(terminal, config)?;
                    return Ok(());
                }
                Action::Quit => return Ok(()),
            }
        }
    }
}

fn run_install_screen(terminal: &mut DefaultTerminal, config: GlobalConfig) -> Result<()> {
    let mut progress = InstallProgress::start(config);

    loop {
        progress.tick();

        terminal.draw(|frame| progress.render(frame))?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            let ev = crossterm::event::read()?;
            if progress.handle_event(ev) {
                return Ok(());
            }
        }
    }
}
