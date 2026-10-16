mod app;
mod git;
mod ui;

#[cfg(test)]
mod testutil;

use std::io::{self, Stderr};

use anyhow::Result;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use app::{App, Exit};

type Tui = Terminal<CrosstermBackend<Stderr>>;

fn main() -> Result<()> {
    let root = git::repo_root(&std::env::current_dir()?)?;
    let commits = git::log(&root)?.collect::<Result<Vec<_>>>()?;
    let mut app = App::new(root, commits);

    let mut terminal = init()?;
    let result = run(&mut terminal, &mut app);
    restore();

    result?;
    if let Some(Exit::Pick(sha)) = app.exit {
        println!("{sha}");
    }
    Ok(())
}

fn run(terminal: &mut Tui, app: &mut App) -> Result<()> {
    while app.exit.is_none() {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
    }
    Ok(())
}

/// Draws on stderr so stdout stays free for the picked sha: `git checkout $(levi)`.
fn init() -> Result<Tui> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    enable_raw_mode()?;
    execute!(io::stderr(), EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stderr()))?)
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stderr(), LeaveAlternateScreen);
}
