//! Application state and key handling.

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::git::{self, Commit, FileChange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Commits,
    Files,
    Diff,
}

impl Focus {
    fn next(self) -> Focus {
        match self {
            Focus::Commits => Focus::Files,
            Focus::Files => Focus::Diff,
            Focus::Diff => Focus::Commits,
        }
    }

    fn prev(self) -> Focus {
        match self {
            Focus::Commits => Focus::Diff,
            Focus::Files => Focus::Commits,
            Focus::Diff => Focus::Files,
        }
    }
}

pub enum Exit {
    Quit,
    /// Print this sha to stdout.
    Pick(String),
}

pub struct Diff {
    key: (String, String),
    pub text: Result<String, String>,
}

pub struct App {
    root: PathBuf,
    pub commits: Vec<Commit>,
    pub query: String,
    pub focus: Focus,
    pub commit_idx: usize,
    pub file_idx: usize,
    pub diff: Option<Diff>,
    pub diff_scroll: usize,
    /// Rows visible in the diff pane, updated on every draw.
    pub diff_height: usize,
    pub exit: Option<Exit>,
}

impl App {
    pub fn new(root: PathBuf, commits: Vec<Commit>) -> App {
        let mut app = App {
            root,
            commits,
            query: String::new(),
            focus: Focus::Commits,
            commit_idx: 0,
            file_idx: 0,
            diff: None,
            diff_scroll: 0,
            diff_height: 0,
            exit: None,
        };
        app.load_diff();
        app
    }

    pub fn commit(&self) -> Option<&Commit> {
        self.commits.get(self.commit_idx)
    }

    pub fn file(&self) -> Option<&FileChange> {
        self.commit()?.files.get(self.file_idx)
    }

    pub fn diff_lines(&self) -> usize {
        match &self.diff {
            Some(Diff { text: Ok(text), .. }) => text.lines().count(),
            _ => 0,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half_page = (self.diff_height / 2).max(1) as isize;
        let page = self.diff_height.max(1) as isize;
        match key.code {
            KeyCode::Char('c') if ctrl => self.exit = Some(Exit::Quit),
            KeyCode::Char('n') if ctrl => self.move_selection(1),
            KeyCode::Char('p') if ctrl => self.move_selection(-1),
            KeyCode::Char('d') if ctrl => self.scroll_diff(half_page),
            KeyCode::Char('u') if ctrl => self.scroll_diff(-half_page),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::PageDown => self.scroll_diff(page),
            KeyCode::PageUp => self.scroll_diff(-page),
            KeyCode::Tab | KeyCode::Right => self.focus = self.focus.next(),
            KeyCode::BackTab | KeyCode::Left => self.focus = self.focus.prev(),
            KeyCode::Enter => {
                if let Some(commit) = self.commit() {
                    self.exit = Some(Exit::Pick(commit.sha.clone()));
                }
            }
            KeyCode::Esc if self.query.is_empty() => self.exit = Some(Exit::Quit),
            KeyCode::Esc => self.query.clear(),
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Char(c) if !ctrl => self.query.push(c),
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            Focus::Commits => {
                let idx = step(self.commit_idx, delta, self.commits.len());
                if idx != self.commit_idx {
                    self.commit_idx = idx;
                    self.file_idx = 0;
                    self.load_diff();
                }
            }
            Focus::Files => {
                let len = self.commit().map_or(0, |c| c.files.len());
                let idx = step(self.file_idx, delta, len);
                if idx != self.file_idx {
                    self.file_idx = idx;
                    self.load_diff();
                }
            }
            Focus::Diff => self.scroll_diff(delta),
        }
    }

    fn scroll_diff(&mut self, delta: isize) {
        let max = self.diff_lines().saturating_sub(1);
        self.diff_scroll = self.diff_scroll.saturating_add_signed(delta).min(max);
    }

    /// Fetches the diff for the selected file, unless it's already loaded.
    fn load_diff(&mut self) {
        let Some(commit) = self.commit() else {
            self.diff = None;
            return;
        };
        let Some(file) = self.file() else {
            self.diff = None;
            return;
        };
        let key = (commit.sha.clone(), file.path.clone());
        if self.diff.as_ref().is_some_and(|d| d.key == key) {
            return;
        }
        let text = git::diff(&self.root, &commit.sha, file).map_err(|e| e.to_string());
        self.diff = Some(Diff { key, text });
        self.diff_scroll = 0;
    }
}

/// Moves `idx` by `delta` within `0..len`, clamping at the ends.
fn step(idx: usize, delta: isize, len: usize) -> usize {
    idx.saturating_add_signed(delta).min(len.saturating_sub(1))
}
