mod app;
mod filter;
mod git;
mod ui;
mod worker;

#[cfg(test)]
mod testutil;

use std::io::{self, Stderr};
use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use app::{App, Exit};

type Tui = Terminal<CrosstermBackend<Stderr>>;

fn main() -> Result<()> {
    let root = git::repo_root(&std::env::current_dir()?)?;
    let mut app = App::new();

    let mut terminal = init()?;
    let result = run(&mut terminal, &mut app, root);
    restore();

    result?;
    if let Some(Exit::Pick(sha)) = app.exit {
        println!("{sha}");
    }
    Ok(())
}

fn run(terminal: &mut Tui, app: &mut App, root: PathBuf) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    worker::spawn_input(tx.clone());
    worker::spawn_log(root.clone(), tx.clone());
    let diffs = worker::spawn_diff(root, tx);

    while app.exit.is_none() {
        if let Some(req) = app.take_diff_request() {
            diffs.send(req)?;
        }
        terminal.draw(|frame| ui::draw(frame, app))?;
        app.update(rx.recv()?);
        // Apply everything that queued up during the draw before drawing again.
        while let Ok(msg) = rx.try_recv() {
            app.update(msg);
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
