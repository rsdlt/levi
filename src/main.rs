mod app;
mod filter;
mod git;
mod ui;
mod worker;

#[cfg(test)]
mod testutil;

use std::io::{self, Stderr};
use std::path::PathBuf;
use std::process::ExitCode;
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

const HELP: &str = "\
levi — commits, their files and diffs, with fuzzy search. Small, swift, precise.

Usage: levi    (run inside a git repository)

Type to filter commits by subject, author, sha prefix or touched files.
fzf syntax: 'exact  ^prefix  suffix$  !negate  (space-separated terms all match)

Keys:
  ↑/↓  C-p/C-n        move in the focused pane
  Tab/S-Tab  ←/→      switch pane
  PgUp/PgDn  C-u/C-d  scroll the diff
  Enter               print the selected sha and quit:  git show $(levi)
  Esc                 clear the query, or quit
  C-c                 quit

Exits 130 when quit without picking a commit, like fzf.
";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("-h" | "--help") => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("levi {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(arg) => {
            eprintln!("levi: unexpected argument '{arg}' (see --help)");
            return ExitCode::from(2);
        }
    }
    match try_main() {
        Ok(Exit::Pick(sha)) => {
            println!("{sha}");
            ExitCode::SUCCESS
        }
        Ok(Exit::Quit) => ExitCode::from(130),
        Err(e) => {
            eprintln!("levi: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<Exit> {
    let root = git::preflight(&std::env::current_dir()?)?;
    let mut app = App::new();

    let mut terminal = init()?;
    let result = run(&mut terminal, &mut app, root);
    restore();

    result?;
    Ok(app.exit.unwrap_or(Exit::Quit))
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
