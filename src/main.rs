mod app;
mod clipboard;
mod filter;
mod git;
mod query;
mod ui;
mod words;
mod worker;

#[cfg(test)]
mod testutil;

use std::io::{self, IsTerminal, Stderr, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use app::{App, Exit};

type Tui = Terminal<CrosstermBackend<Stderr>>;

const HELP: &str = "\
levi — commits, their files and diffs, with fuzzy search. Small, swift, precise.

Usage: levi [revision]                     (run inside a git repository)
       levi --filter <query> [revision]   print what the query finds, no TUI

Shows the current branch, or the commits of any revision git understands:
  levi main..HEAD     what your branch adds to main
  levi v1.0..         everything since a tag
  levi other-branch   another branch, without checking it out

Type to filter commits by subject, author, sha prefix or touched files.
fzf syntax: 'exact  ^prefix  suffix$  !negate  (space-separated terms all match)

Commands (whole words in the search bar):
  /r                  search the reflog instead of the log: /r checkout
  /c text             keep commits whose diffs add or remove `text` (the rest of
                      the line, literal, smart case): parse.rs /c let x =

Keys:
  ↑/↓  C-p/C-n        move in the focused pane
  Tab/S-Tab  ←/→      switch pane
  PgUp/PgDn  C-u/C-d  scroll the diff
  S-↑/S-↓             jump to the previous/next hunk or /c match, across files
  S-←/S-→             scroll the diff sideways
  Enter               print the selected sha and quit:  git show $(levi)
  C-y                 copy the selected commit's sha
  C-w                 delete the last word of the query
  Esc                 clear the query, or quit
  C-c                 quit
  click               select a commit or file
  wheel               move a list, or scroll the diff
  drag                copy text from the diff, or whole rows from a list

Exits 130 when quit without picking a commit, like fzf. Set NO_COLOR for no colors.

--filter (or -f) runs the same search without the TUI, like fzf --filter, for
scripts and coding agents. It prints `<sha> <subject>` per match, best first,
and exits 1 when nothing matches:
  levi --filter 'parse.rs /c let x ='   which commits changed that line there
";

/// What the command line asks for.
enum Cli {
    Help,
    Version,
    Browse(Option<String>),
    Filter(String, Option<String>),
}

/// At most one revision, and `--filter <query>` anywhere around it.
fn parse_args(args: &[String]) -> Result<Cli, String> {
    match args {
        [flag] if flag == "-h" || flag == "--help" => return Ok(Cli::Help),
        // levi has no verbose mode, so -v is free to mean version too.
        [flag] if flag == "-V" || flag == "-v" || flag == "--version" => return Ok(Cli::Version),
        _ => {}
    }
    let (mut rev, mut query) = (None, None);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(q) = arg.strip_prefix("--filter=") {
            query = Some(q.to_owned());
        } else if arg == "--filter" || arg == "-f" {
            query = Some(args.next().ok_or(format!("'{arg}' needs a query"))?.clone());
        } else if rev.is_none() && !arg.starts_with('-') {
            rev = Some(arg.clone());
        } else {
            // Anything else starting with `-` would be a git option, not a revision.
            return Err(format!("unexpected argument '{arg}'"));
        }
    }
    Ok(match query {
        Some(query) => Cli::Filter(query, rev),
        None => Cli::Browse(rev),
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rev = match parse_args(&args) {
        Ok(Cli::Help) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Cli::Version) => {
            println!("levi {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Cli::Browse(rev)) => rev,
        Ok(Cli::Filter(query, rev)) => {
            return match print_matches(query, rev) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("levi: {e:#}");
                    ExitCode::from(2)
                }
            };
        }
        Err(e) => {
            eprintln!("levi: {e} (see --help)");
            return ExitCode::from(2);
        }
    };
    match try_main(rev) {
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

/// `--filter`: runs the query the way the search bar would, without a terminal,
/// and prints each matching commit, best first. Returns whether anything did.
fn print_matches(query: String, rev: Option<String>) -> Result<bool> {
    let root = git::preflight(&std::env::current_dir()?, rev.as_deref())?;
    let mut app = App::new();
    app.query = query;
    let (tx, rx) = mpsc::channel();
    let logs = worker::LogLoader::new(
        root.clone(),
        rev.unwrap_or_else(|| "HEAD".into()),
        tx.clone(),
    );
    let code = worker::CodeSearcher::new(root, tx);
    loop {
        if let Some(req) = app.take_log_request() {
            logs.start(req);
        }
        // Nobody is typing, so a code search needn't wait for a pause.
        if let Some(req) = app.take_code_request(Instant::now() + app::CODE_DEBOUNCE) {
            code.start(req);
        }
        let searching = app.code.as_ref().is_some_and(|c| !c.finished);
        if !app.loading && !searching {
            break;
        }
        app.update(rx.recv()?);
    }
    if let Some(err) = app.log_error.take() {
        bail!(err);
    }
    if let Some(err) = app.code.as_mut().and_then(|c| c.error.take()) {
        bail!("code search failed: {err}");
    }

    let mut out = io::BufWriter::new(io::stdout().lock());
    for hit in &app.view {
        let c = &app.commits[hit.commit];
        let text = c.reflog.as_ref().map_or(&c.subject, |r| &r.action);
        // A closed pipe (`| head`) just means nobody wants the rest.
        if writeln!(out, "{} {text}", c.sha).is_err() {
            break;
        }
    }
    let _ = out.flush();
    Ok(!app.view.is_empty())
}

fn try_main(rev: Option<String>) -> Result<Exit> {
    let root = git::preflight(&std::env::current_dir()?, rev.as_deref())?;
    let mut app = App::new();
    app.rev = rev;
    app.no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());

    if !io::stderr().is_terminal() {
        bail!("needs a terminal (it draws on stderr, which isn't one)");
    }
    let mut terminal = init().context("needs a terminal")?;
    let result = run(&mut terminal, &mut app, root);
    restore();

    result?;
    Ok(app.exit.unwrap_or(Exit::Quit))
}

fn run(terminal: &mut Tui, app: &mut App, root: PathBuf) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    worker::spawn_input(tx.clone());
    let rev = app.rev.clone().unwrap_or_else(|| "HEAD".into());
    let logs = worker::LogLoader::new(root.clone(), rev, tx.clone());
    let code = worker::CodeSearcher::new(root.clone(), tx.clone());
    let diffs = worker::spawn_diff(root, tx);

    while app.exit.is_none() {
        if let Some(req) = app.take_log_request() {
            logs.start(req);
        }
        if app.take_code_cancel() {
            code.cancel();
        }
        if let Some(req) = app.take_code_request(Instant::now()) {
            code.start(req);
        }
        if let Some(req) = app.take_diff_request() {
            diffs.send(req)?;
        }
        if let Some(text) = app.take_clipboard() {
            clipboard::copy(&text);
        }
        terminal.draw(|frame| ui::draw(frame, app))?;
        // Sleep until something happens, or until a pending code search is due.
        let msg = match app.deadline() {
            Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => bail!("event channel closed"),
            },
            None => Some(rx.recv()?),
        };
        if let Some(msg) = msg {
            app.update(msg);
        }
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
    execute!(io::stderr(), EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stderr()))?)
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stderr(), DisableMouseCapture, LeaveAlternateScreen);
}
