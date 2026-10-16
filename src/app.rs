//! Application state and key handling.

use std::collections::{HashMap, VecDeque};

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::filter::{self, Filter, Hit};
use crate::git::{Commit, DiffLine, FileChange};
use crate::query::{self, Source};

/// Diffs kept in memory; older ones are refetched on demand.
const DIFF_CACHE_SIZE: usize = 128;

/// (commit sha, file path)
pub type DiffKey = (String, String);
pub type DiffResult = Result<Vec<DiffLine>, String>;

pub enum Msg {
    Input(Event),
    /// A batch from the log load with this generation.
    Commits(u64, Vec<Commit>),
    LogDone(u64, Result<(), String>),
    Diff(DiffKey, DiffResult),
}

/// Load `source` into the commits column. A higher generation supersedes the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRequest {
    pub source: Source,
    pub generation: u64,
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
    /// Everything loaded so far, in log order.
    pub commits: Vec<Commit>,
    /// What the commits pane shows: commits matching the query, best first.
    pub view: Vec<Hit>,
    pub filter: Filter,
    pub loading: bool,
    pub log_error: Option<String>,
    /// What's typed in the search bar.
    pub query: String,
    /// Where the commits column comes from, as of the last `settle`.
    pub source: Source,
    pub focus: Focus,
    /// Index into `view`.
    pub commit_idx: usize,
    /// Index into `files()`.
    pub file_idx: usize,
    /// First visible row of each list, maintained by the renderer.
    pub commit_offset: usize,
    pub file_offset: usize,
    pub diff_scroll: usize,
    /// Rows visible in the diff pane, updated on every draw.
    pub diff_height: usize,
    pub exit: Option<Exit>,
    diffs: DiffCache,
    /// Last diff handed out by `take_diff_request`.
    requested: Option<DiffKey>,
    /// The query the view reflects.
    applied: String,
    generation: u64,
    log_request: Option<LogRequest>,
}

impl App {
    pub fn new() -> App {
        App {
            commits: Vec::new(),
            view: Vec::new(),
            filter: Filter::new(),
            loading: true,
            log_error: None,
            query: String::new(),
            source: Source::Log,
            focus: Focus::Commits,
            commit_idx: 0,
            file_idx: 0,
            commit_offset: 0,
            file_offset: 0,
            diff_scroll: 0,
            diff_height: 0,
            exit: None,
            diffs: DiffCache::default(),
            requested: None,
            applied: String::new(),
            generation: 0,
            log_request: Some(LogRequest {
                source: Source::Log,
                generation: 0,
            }),
        }
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                // Typing only edits the query; filtering waits for `settle`, so a
                // burst of keystrokes costs one pass. Anything else needs the
                // up-to-date view first.
                if !is_query_edit(&key) {
                    self.settle();
                }
                self.handle_key(key);
            }
            Msg::Input(_) => {}
            // Leftovers from a load that was superseded.
            Msg::Commits(generation, _) | Msg::LogDone(generation, _)
                if generation != self.generation => {}
            Msg::Commits(_, batch) => {
                self.settle();
                self.add_commits(batch);
            }
            Msg::LogDone(_, result) => {
                self.loading = false;
                self.log_error = result.err();
            }
            Msg::Diff(key, result) => self.diffs.insert(key, result),
        }
    }

    /// Applies a changed query to the view. Selection jumps to the best match.
    pub fn settle(&mut self) {
        if self.query == self.applied {
            return;
        }
        self.applied = self.query.clone();
        let parsed = query::parse(&self.query);
        if parsed.source != self.source {
            self.switch_source(parsed.source);
        } else if parsed.fuzzy == self.filter.query() {
            return;
        }
        let narrows = self.filter.set_query(&parsed.fuzzy);
        let candidates: Vec<usize> = if narrows {
            self.view.iter().map(|h| h.commit).collect()
        } else {
            (0..self.commits.len()).collect()
        };
        self.view = filter::par_matches(&parsed.fuzzy, &self.commits, &candidates);
        filter::sort(&mut self.view);
        self.commit_idx = 0;
        self.file_idx = 0;
        self.diff_scroll = 0;
    }

    /// Drops the loaded commits and asks for `source` instead.
    fn switch_source(&mut self, source: Source) {
        self.source = source;
        self.generation += 1;
        self.log_request = Some(LogRequest {
            source,
            generation: self.generation,
        });
        self.commits.clear();
        self.view.clear();
        self.loading = true;
        self.log_error = None;
    }

    /// The commit list to load, when the source changed.
    pub fn take_log_request(&mut self) -> Option<LogRequest> {
        self.settle();
        self.log_request.take()
    }

    /// Appends newly loaded commits, keeping the selection on the same commit.
    fn add_commits(&mut self, batch: Vec<Commit>) {
        let selected = self.hit().map(|h| h.commit);
        let start = self.commits.len();
        self.commits.extend(batch);
        for i in start..self.commits.len() {
            if let Some(hit) = self.filter.matches(i, &self.commits[i]) {
                self.view.push(hit);
            }
        }
        // With no query the view is already in log order.
        if !self.filter.is_empty() {
            filter::sort(&mut self.view);
            if let Some(selected) = selected {
                self.commit_idx = self
                    .view
                    .iter()
                    .position(|h| h.commit == selected)
                    .unwrap_or(0);
            }
        }
    }

    /// The diff the selection needs, if it isn't cached and wasn't already asked for.
    pub fn take_diff_request(&mut self) -> Option<DiffRequest> {
        self.settle();
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

    pub fn hit(&self) -> Option<&Hit> {
        self.view.get(self.commit_idx)
    }

    pub fn commit(&self) -> Option<&Commit> {
        self.hit().map(|h| &self.commits[h.commit])
    }

    /// Files of the selected commit that match the query.
    pub fn files(&self) -> Vec<&FileChange> {
        let Some(hit) = self.hit() else {
            return Vec::new();
        };
        let files = &self.commits[hit.commit].files;
        match &hit.files {
            None => files.iter().collect(),
            Some(matching) => matching.iter().map(|&i| &files[i]).collect(),
        }
    }

    pub fn file(&self) -> Option<&FileChange> {
        self.files().get(self.file_idx).copied()
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
                let idx = step(self.commit_idx, delta, self.view.len());
                if idx != self.commit_idx {
                    self.commit_idx = idx;
                    self.file_idx = 0;
                    self.diff_scroll = 0;
                }
            }
            Focus::Files => {
                let len = self.files().len();
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

/// Keys that only change the query (see `handle_key`).
fn is_query_edit(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(_) => !key.modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Backspace | KeyCode::Esc => true,
        _ => false,
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

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    /// `LEVI_BENCH_REPO=/path/to/big/repo cargo test --release bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn filter_speed() {
        let Ok(path) = std::env::var("LEVI_BENCH_REPO") else {
            return;
        };
        let t = Instant::now();
        let commits: Vec<Commit> = crate::git::log(path.as_ref(), crate::query::Source::Log)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let files: usize = commits.iter().map(|c| c.files.len()).sum();
        println!(
            "load: {} commits, {files} files in {:?}",
            commits.len(),
            t.elapsed()
        );

        let mut app = App::new();
        app.update(Msg::Commits(0, commits));
        for query in [
            "e",
            "pa",
            "par",
            "pars",
            "parse",
            "parse.c",
            "Documentation/",
            "!t",
            "zzzzz",
            "",
            "junio",
        ] {
            app.query = query.into();
            let t = Instant::now();
            app.settle();
            println!(
                "{query:>16}: {:>6} hits in {:?}",
                app.view.len(),
                t.elapsed()
            );
        }
    }
}
