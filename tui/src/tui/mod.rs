pub mod screens;
pub mod theme;
pub mod tracing_layer;
pub mod widgets;

use std::time::Duration;

use color_eyre::eyre::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, EventStream};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::prelude::CrosstermBackend;

use archinstall_zfs_core::config::types::GlobalConfig;

use self::screens::install_progress::InstallProgress;
use self::screens::wizard::Wizard;

pub async fn run_tui(config: GlobalConfig, _dry_run: bool) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, config).await;
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

async fn run_app(terminal: &mut DefaultTerminal, config: GlobalConfig) -> Result<()> {
    let mut wizard = Wizard::new(config);
    let mut events = EventStream::new();

    loop {
        terminal.draw(|frame| wizard.render(frame))?;

        match tokio::time::timeout(Duration::from_millis(50), events.next()).await {
            Ok(Some(Ok(ev))) => match wizard.handle_event(ev, terminal).await? {
                Action::Continue => {}
                Action::Install => {
                    let config = wizard.into_config();
                    run_install_screen(terminal, config).await?;
                    return Ok(());
                }
                Action::Quit => return Ok(()),
            },
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => return Ok(()), // stream ended
            Err(_) => {}               // timeout, continue loop
        }
    }
}

async fn run_install_screen(terminal: &mut DefaultTerminal, config: GlobalConfig) -> Result<()> {
    let mut progress = InstallProgress::start(config);
    let mut events = EventStream::new();

    loop {
        progress.tick();

        terminal.draw(|frame| progress.render(frame))?;

        match tokio::time::timeout(Duration::from_millis(50), events.next()).await {
            Ok(Some(Ok(ev))) => {
                if progress.handle_event(ev) {
                    return Ok(());
                }
            }
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => return Ok(()),
            Err(_) => {} // timeout
        }
    }
}
