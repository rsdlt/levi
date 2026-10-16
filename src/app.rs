//! Application state and key handling.

use std::collections::{HashMap, VecDeque};

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::git::{Commit, DiffLine, FileChange};

/// Diffs kept in memory; older ones are refetched on demand.
const DIFF_CACHE_SIZE: usize = 128;

/// (commit sha, file path)
pub type DiffKey = (String, String);
pub type DiffResult = Result<Vec<DiffLine>, String>;

pub enum Msg {
    Input(Event),
    Commits(Vec<Commit>),
    LogDone(Result<(), String>),
    Diff(DiffKey, DiffResult),
}

pub struct DiffRequest {
    pub sha: String,
    pub file: FileChange,
}

impl DiffRequest {
    pub fn key(&self) -> DiffKey {
        (self.sha.clone(), self.file.path.clone())
    }
}

pub enum DiffView<'a> {
    Empty,
    Loading,
    Ready(&'a [DiffLine]),
    Failed(&'a str),
}

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

pub struct App {
    pub commits: Vec<Commit>,
    pub loading: bool,
    pub log_error: Option<String>,
    pub query: String,
    pub focus: Focus,
    pub commit_idx: usize,
    pub file_idx: usize,
    pub diff_scroll: usize,
    /// Rows visible in the diff pane, updated on every draw.
    pub diff_height: usize,
    pub exit: Option<Exit>,
    diffs: DiffCache,
    /// Last diff handed out by `take_diff_request`.
    requested: Option<DiffKey>,
}

impl App {
    pub fn new() -> App {
        App {
            commits: Vec::new(),
            loading: true,
            log_error: None,
            query: String::new(),
            focus: Focus::Commits,
            commit_idx: 0,
            file_idx: 0,
            diff_scroll: 0,
            diff_height: 0,
            exit: None,
            diffs: DiffCache::default(),
            requested: None,
        }
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => self.handle_key(key),
            Msg::Input(_) => {}
            Msg::Commits(batch) => self.commits.extend(batch),
            Msg::LogDone(result) => {
                self.loading = false;
                self.log_error = result.err();
            }
            Msg::Diff(key, result) => self.diffs.insert(key, result),
        }
    }

    /// The diff the selection needs, if it isn't cached and wasn't already asked for.
    pub fn take_diff_request(&mut self) -> Option<DiffRequest> {
        let commit = self.commit()?;
        let file = self.file()?;
        let req = DiffRequest {
            sha: commit.sha.clone(),
            file: file.clone(),
        };
        let key = req.key();
        if self.requested.as_ref() == Some(&key) {
            return None;
        }
        self.requested = Some(key.clone());
        if self.diffs.touch(&key) {
            return None;
        }
        Some(req)
    }

    pub fn commit(&self) -> Option<&Commit> {
        self.commits.get(self.commit_idx)
    }

    pub fn file(&self) -> Option<&FileChange> {
        self.commit()?.files.get(self.file_idx)
    }

    pub fn diff(&self) -> DiffView<'_> {
        let (Some(commit), Some(file)) = (self.commit(), self.file()) else {
            return DiffView::Empty;
        };
        match self.diffs.get(&(commit.sha.clone(), file.path.clone())) {
            None => DiffView::Loading,
            Some(Ok(lines)) => DiffView::Ready(lines),
            Some(Err(err)) => DiffView::Failed(err),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
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
                    self.diff_scroll = 0;
                }
            }
            Focus::Files => {
                let len = self.commit().map_or(0, |c| c.files.len());
                let idx = step(self.file_idx, delta, len);
                if idx != self.file_idx {
                    self.file_idx = idx;
                    self.diff_scroll = 0;
                }
            }
            Focus::Diff => self.scroll_diff(delta),
        }
    }

    fn scroll_diff(&mut self, delta: isize) {
        let lines = match self.diff() {
            DiffView::Ready(lines) => lines.len(),
            _ => 0,
        };
        self.diff_scroll = step(self.diff_scroll, delta, lines);
    }
}

/// Moves `idx` by `delta` within `0..len`, clamping at the ends.
fn step(idx: usize, delta: isize, len: usize) -> usize {
    idx.saturating_add_signed(delta).min(len.saturating_sub(1))
}

/// Small LRU: recency order in a deque, which is fine at this size.
#[derive(Default)]
struct DiffCache {
    map: HashMap<DiffKey, DiffResult>,
    order: VecDeque<DiffKey>,
}

impl DiffCache {
    fn get(&self, key: &DiffKey) -> Option<&DiffResult> {
        self.map.get(key)
    }

    /// Marks `key` as recently used. Returns whether it's cached.
    fn touch(&mut self, key: &DiffKey) -> bool {
        let Some(pos) = self.order.iter().position(|k| k == key) else {
            return false;
        };
        let key = self.order.remove(pos).expect("position is in bounds");
        self.order.push_back(key);
        true
    }

    fn insert(&mut self, key: DiffKey, value: DiffResult) {
        if self.map.insert(key.clone(), value).is_some() {
            self.touch(&key);
            return;
        }
        self.order.push_back(key);
        if self.order.len() > DIFF_CACHE_SIZE {
            let oldest = self.order.pop_front().expect("cache is non-empty");
            self.map.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: usize) -> DiffKey {
        (n.to_string(), "f".into())
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let mut cache = DiffCache::default();
        for n in 0..DIFF_CACHE_SIZE {
            cache.insert(key(n), Ok(vec![]));
        }
        assert!(cache.touch(&key(0)));
        cache.insert(key(DIFF_CACHE_SIZE), Ok(vec![]));
        assert!(cache.get(&key(0)).is_some(), "recently touched survives");
        assert!(cache.get(&key(1)).is_none(), "oldest is evicted");
        assert_eq!(cache.map.len(), DIFF_CACHE_SIZE);
    }
}
