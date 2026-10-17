//! Application state and key handling.

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::text::Span;

use crate::filter::{self, Filter, Hit};
use crate::git::{Commit, DiffLine, FileChange, LineKind};
use crate::query::{self, Source, find_code};

/// Diffs kept in memory; older ones are refetched on demand.
const DIFF_CACHE_SIZE: usize = 128;
/// A code search starts once typing pauses this long, not on every keystroke.
pub const CODE_DEBOUNCE: Duration = Duration::from_millis(300);
/// Lines kept above a code match when the diff jumps to it.
const JUMP_CONTEXT: usize = 3;
/// Diff lines one notch of the mouse wheel scrolls.
const WHEEL_LINES: isize = 3;
/// Columns one sideways scroll of the diff moves, by key or wheel.
const SIDE_STEP: isize = 8;

/// (commit sha, file path), or an empty path for a commit without files.
pub type DiffKey = (String, String);
pub type DiffResult = Result<Vec<DiffLine>, String>;

pub enum Msg {
    Input(Event),
    /// A batch from the log load with this generation.
    Commits(u64, Vec<Commit>),
    LogDone(u64, Result<(), String>),
    Diff(DiffKey, DiffResult),
    /// Commits from code search `u64` whose diffs matched, with the matching paths.
    CodeHits(u64, Vec<(String, Vec<String>)>),
    /// Chunks searched so far, out of how many.
    CodeProgress(u64, usize, usize),
    CodeDone(u64, Result<(), String>),
}

/// Search the diffs of `targets` for `pattern`. A higher generation supersedes the rest.
pub struct CodeRequest {
    pub pattern: String,
    pub targets: Vec<CodeTarget>,
    pub generation: u64,
}

/// A commit to search, and which of its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeTarget {
    pub sha: String,
    /// The files the fuzzy query matched; `None` when the commit itself matched,
    /// so every file counts.
    pub paths: Option<Vec<String>>,
}

/// A `/c` search: what was asked and what has come back so far.
pub struct CodeSearch {
    pub pattern: String,
    generation: u64,
    /// sha → paths whose diffs matched.
    hits: HashMap<String, Vec<String>>,
    /// Chunks searched, out of how many.
    pub progress: (usize, usize),
    pub finished: bool,
    pub error: Option<String>,
    /// The fuzzy query whose matches were searched, once the search started.
    /// Results stay valid for any query that only narrows it.
    scope: Option<String>,
    typed_at: Instant,
}

impl CodeSearch {
    /// Narrows a fuzzy hit to the files whose diffs matched; `None` if none did.
    fn admit(&self, commits: &[Commit], mut hit: Hit) -> Option<Hit> {
        let commit = &commits[hit.commit];
        let paths = self.hits.get(&commit.sha)?;
        let matched = |i: &usize| {
            let f = &commit.files[*i];
            paths
                .iter()
                .any(|p| *p == f.path || Some(p) == f.old_path.as_ref())
        };
        let files: Vec<usize> = match &hit.files {
            None => (0..commit.files.len()).filter(matched).collect(),
            Some(files) => files.iter().copied().filter(matched).collect(),
        };
        if files.is_empty() {
            return None;
        }
        hit.files = Some(files);
        Some(hit)
    }
}

/// Load `source` into the commits column. A higher generation supersedes the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRequest {
    pub source: Source,
    pub generation: u64,
}

pub struct DiffRequest {
    pub sha: String,
    /// `None` for a commit without files: only its message is shown.
    pub file: Option<FileChange>,
}

impl DiffRequest {
    pub fn key(&self) -> DiffKey {
        diff_key(&self.sha, self.file.as_ref())
    }
}

fn diff_key(sha: &str, file: Option<&FileChange>) -> DiffKey {
    (
        sha.to_owned(),
        file.map_or_else(String::new, |f| f.path.clone()),
    )
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
    /// The revision levi was started with, if not the current branch. The
    /// reflog ignores it.
    pub rev: Option<String>,
    /// Draw without colors, for `NO_COLOR` (https://no-color.org).
    pub no_color: bool,
    /// The running or finished `/c` search, if the query has one.
    pub code: Option<CodeSearch>,
    pub focus: Focus,
    /// Index into `view`.
    pub commit_idx: usize,
    /// Index into `files()`.
    pub file_idx: usize,
    /// First visible row of each list, maintained by the renderer.
    pub commit_offset: usize,
    pub file_offset: usize,
    pub diff_scroll: usize,
    /// First visible column of the diff, for lines wider than the pane.
    pub diff_column: usize,
    /// Rows visible in the diff pane, updated on every draw.
    pub diff_height: usize,
    /// Columns visible in the diff pane, updated on every draw.
    pub diff_width: usize,
    /// Where the commits, files and diff panes are on screen, updated on every
    /// draw, so clicks can find their target.
    pub panes: [Rect; 3],
    pub exit: Option<Exit>,
    diffs: DiffCache,
    /// Last diff handed out by `take_diff_request`.
    requested: Option<DiffKey>,
    /// The query the view reflects.
    applied: String,
    generation: u64,
    log_request: Option<LogRequest>,
    /// Where each sha sits in `commits` (the reflog can repeat one).
    by_sha: HashMap<String, Vec<usize>>,
    code_generation: u64,
    /// Running searches must stop.
    code_cancel: bool,
    /// The diff that was last scrolled to its first code match.
    jumped: Option<DiffKey>,
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
            rev: None,
            no_color: false,
            code: None,
            focus: Focus::Commits,
            commit_idx: 0,
            file_idx: 0,
            commit_offset: 0,
            file_offset: 0,
            diff_scroll: 0,
            diff_column: 0,
            diff_height: 0,
            diff_width: 0,
            panes: [Rect::default(); 3],
            exit: None,
            diffs: DiffCache::default(),
            requested: None,
            applied: String::new(),
            generation: 0,
            log_request: Some(LogRequest {
                source: Source::Log,
                generation: 0,
            }),
            by_sha: HashMap::new(),
            code_generation: 0,
            code_cancel: false,
            jumped: None,
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
            Msg::Input(Event::Mouse(mouse)) => {
                self.settle();
                self.handle_mouse(mouse);
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
            Msg::CodeHits(generation, hits) => {
                self.settle();
                self.add_code_hits(generation, hits);
            }
            Msg::CodeProgress(generation, done, total) => {
                if let Some(code) = self.code.as_mut().filter(|c| c.generation == generation) {
                    code.progress = (done, total);
                }
            }
            Msg::CodeDone(generation, result) => {
                if let Some(code) = self.code.as_mut().filter(|c| c.generation == generation) {
                    code.finished = true;
                    code.error = result.err();
                }
            }
        }
    }

    /// Applies a changed query to the view. Selection jumps to the best match.
    pub fn settle(&mut self) {
        if self.query == self.applied {
            return;
        }
        self.applied = self.query.clone();
        let parsed = query::parse(&self.query);
        let switched = parsed.source != self.source;
        if switched {
            self.switch_source(parsed.source);
        }
        let mut code_changed =
            switched || parsed.code.as_deref() != self.code.as_ref().map(|c| c.pattern.as_str());
        if code_changed {
            self.restart_code(parsed.code);
        } else if let Some(code) = &mut self.code
            && parsed.fuzzy != self.filter.query()
        {
            match &code.scope {
                // The search covered everything the new fuzzy query can match.
                Some(scope) if filter::narrows(scope, &parsed.fuzzy) => {}
                // It might miss some: search the new scope instead.
                Some(_) => {
                    let pattern = code.pattern.clone();
                    self.restart_code(Some(pattern));
                    code_changed = true;
                }
                // Not started yet: it'll use the new scope once typing pauses.
                None => code.typed_at = Instant::now(),
            }
        }
        if !code_changed && parsed.fuzzy == self.filter.query() {
            return;
        }
        // A new code search starts from nothing, so previous hits can't be reused.
        let narrows = self.filter.set_query(&parsed.fuzzy) && !code_changed;
        let candidates: Vec<usize> = if narrows {
            self.view.iter().map(|h| h.commit).collect()
        } else {
            (0..self.commits.len()).collect()
        };
        self.view = filter::par_matches(&parsed.fuzzy, &self.commits, &candidates);
        if let Some(code) = &self.code {
            let hits = mem::take(&mut self.view);
            self.view = hits
                .into_iter()
                .filter_map(|h| code.admit(&self.commits, h))
                .collect();
        }
        filter::sort(&mut self.view);
        self.commit_idx = 0;
        self.file_idx = 0;
        self.reset_diff_scroll();
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
        self.by_sha.clear();
        self.loading = true;
        self.log_error = None;
    }

    /// Drops the current code search and prepares `pattern`, if any. It's handed
    /// out by `take_code_request` once typing pauses and the log is loaded.
    fn restart_code(&mut self, pattern: Option<String>) {
        self.code_cancel = true;
        self.jumped = None;
        self.code = pattern.map(|pattern| {
            self.code_generation += 1;
            CodeSearch {
                pattern,
                generation: self.code_generation,
                hits: HashMap::new(),
                progress: (0, 0),
                finished: false,
                error: None,
                scope: None,
                typed_at: Instant::now(),
            }
        });
    }

    /// Whether running code searches must stop. Asks once.
    pub fn take_code_cancel(&mut self) -> bool {
        mem::take(&mut self.code_cancel)
    }

    /// The code search to start: after typing paused for `CODE_DEBOUNCE` and the
    /// log is complete, over the loaded commits that pass the fuzzy filter.
    pub fn take_code_request(&mut self, now: Instant) -> Option<CodeRequest> {
        self.settle();
        if self.loading {
            return None;
        }
        let code = self.code.as_mut()?;
        if code.scope.is_some() || now < code.typed_at + CODE_DEBOUNCE {
            return None;
        }
        let fuzzy = self.filter.query().to_owned();
        let all: Vec<usize> = (0..self.commits.len()).collect();
        let targets = code_targets(
            &self.commits,
            filter::par_matches(&fuzzy, &self.commits, &all),
        );
        code.scope = Some(fuzzy);
        Some(CodeRequest {
            pattern: code.pattern.clone(),
            targets,
            generation: code.generation,
        })
    }

    /// When the main loop must wake up even without input: a code search
    /// waiting out its debounce.
    pub fn deadline(&self) -> Option<Instant> {
        let code = self.code.as_ref()?;
        (code.scope.is_none() && !self.loading).then(|| code.typed_at + CODE_DEBOUNCE)
    }

    /// Records code-search matches and shows the commits that also pass the
    /// fuzzy filter.
    fn add_code_hits(&mut self, generation: u64, hits: Vec<(String, Vec<String>)>) {
        let Some(code) = self.code.as_mut().filter(|c| c.generation == generation) else {
            return;
        };
        let selected = self.view.get(self.commit_idx).map(|h| h.commit);
        for (sha, paths) in hits {
            let positions = self.by_sha.get(&sha).cloned().unwrap_or_default();
            code.hits.insert(sha, paths);
            for i in positions {
                if let Some(hit) = self.filter.matches(i, &self.commits[i]) {
                    self.view.extend(code.admit(&self.commits, hit));
                }
            }
        }
        self.resort(selected);
    }

    /// Sorts the view and keeps `selected` (an index into `commits`) selected.
    fn resort(&mut self, selected: Option<usize>) {
        filter::sort(&mut self.view);
        if let Some(selected) = selected {
            self.commit_idx = self
                .view
                .iter()
                .position(|h| h.commit == selected)
                .unwrap_or(0);
        }
    }

    /// Scrolls a freshly shown diff to the first changed line with the code match.
    fn jump_to_code(&mut self) {
        let (Some(code), Some(commit), Some(file)) = (&self.code, self.commit(), self.file())
        else {
            return;
        };
        let key = (commit.sha.clone(), file.path.clone());
        if self.jumped.as_ref() == Some(&key) {
            return;
        }
        let Some(Ok(lines)) = self.diffs.get(&key) else {
            return;
        };
        let first = lines
            .iter()
            .enumerate()
            .find_map(|(i, l)| Some((i, code_hit(l, &code.pattern)?)));
        if let Some((i, (before, through))) = first {
            self.show_code_hit(i, before, through);
        }
        self.jumped = Some(key);
    }

    /// Scrolls to diff line `i`, which has a code match taking up columns
    /// `before..through`. A match past the right edge is scrolled into view too.
    fn show_code_hit(&mut self, i: usize, before: usize, through: usize) {
        self.diff_scroll = i.saturating_sub(JUMP_CONTEXT);
        self.diff_column = if self.diff_width > 0 && through > self.diff_width {
            before.saturating_sub(SIDE_STEP as usize)
        } else {
            0
        };
    }

    /// Scrolls the diff to the next (`forward`) or previous hunk, or code match
    /// while `/c` is on. Past the last one it stays put.
    fn jump_in_diff(&mut self, forward: bool) {
        let DiffView::Ready(lines) = self.diff() else {
            return;
        };
        let pattern = self.code.as_ref().map(|c| c.pattern.as_str());
        // Each stop with the scroll position that shows it.
        let mut stops = lines.iter().enumerate().filter_map(|(i, l)| match pattern {
            Some(pattern) => {
                let hit = code_hit(l, pattern)?;
                Some((i.saturating_sub(JUMP_CONTEXT), i, Some(hit)))
            }
            None => (l.kind == LineKind::Hunk).then_some((i, i, None)),
        });
        let current = self.diff_scroll;
        let stop = if forward {
            stops.find(|&(scroll, ..)| scroll > current)
        } else {
            stops.rfind(|&(scroll, ..)| scroll < current)
        };
        match stop {
            Some((_, i, Some((before, through)))) => self.show_code_hit(i, before, through),
            Some((scroll, ..)) => self.diff_scroll = scroll,
            None => {}
        }
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
            self.by_sha
                .entry(self.commits[i].sha.clone())
                .or_default()
                .push(i);
            if let Some(hit) = self.filter.matches(i, &self.commits[i]) {
                match &self.code {
                    Some(code) => self.view.extend(code.admit(&self.commits, hit)),
                    None => self.view.push(hit),
                }
            }
        }
        // With no query the view is already in log order.
        if !self.filter.is_empty() {
            self.resort(selected);
        }
    }

    /// The diff the selection needs, if it isn't cached and wasn't already asked for.
    pub fn take_diff_request(&mut self) -> Option<DiffRequest> {
        self.settle();
        self.jump_to_code();
        let commit = self.commit()?;
        let req = DiffRequest {
            sha: commit.sha.clone(),
            file: self.file().cloned(),
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
        let Some(commit) = self.commit() else {
            return DiffView::Empty;
        };
        match self.diffs.get(&diff_key(&commit.sha, self.file())) {
            None => DiffView::Loading,
            Some(Ok(lines)) => DiffView::Ready(lines),
            Some(Err(err)) => DiffView::Failed(err),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let half_page = (self.diff_height / 2).max(1) as isize;
        let page = self.diff_height.max(1) as isize;
        match key.code {
            KeyCode::Char('c') if ctrl => self.exit = Some(Exit::Quit),
            KeyCode::Char('n') if ctrl => self.move_selection(1),
            KeyCode::Char('p') if ctrl => self.move_selection(-1),
            KeyCode::Char('d') if ctrl => self.scroll_diff(half_page),
            KeyCode::Char('u') if ctrl => self.scroll_diff(-half_page),
            KeyCode::Down if shift => self.jump_in_diff(true),
            KeyCode::Up if shift => self.jump_in_diff(false),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::PageDown => self.scroll_diff(page),
            KeyCode::PageUp => self.scroll_diff(-page),
            KeyCode::Right if shift => self.scroll_diff_sideways(SIDE_STEP),
            KeyCode::Left if shift => self.scroll_diff_sideways(-SIDE_STEP),
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
            KeyCode::Char('w') if ctrl => {
                // Like a shell: the last word and the spaces after it.
                let kept = self
                    .query
                    .trim_end()
                    .trim_end_matches(|c: char| !c.is_whitespace());
                self.query.truncate(kept.len());
            }
            KeyCode::Char(c) if !ctrl => self.query.push(c),
            _ => {}
        }
    }

    /// Clicks select a row and focus its pane; the wheel acts on the pane
    /// under the pointer without moving focus.
    fn handle_mouse(&mut self, mouse: MouseEvent) {
        let at = Position::new(mouse.column, mouse.row);
        let Some(i) = self.panes.iter().position(|p| p.contains(at)) else {
            return;
        };
        let pane = [Focus::Commits, Focus::Files, Focus::Diff][i];
        let diff = pane == Focus::Diff;
        // Some terminals send Shift+wheel as a vertical scroll with Shift held.
        let sideways = mouse.modifiers.contains(KeyModifiers::SHIFT);
        match mouse.kind {
            MouseEventKind::ScrollRight if diff => self.scroll_diff_sideways(SIDE_STEP),
            MouseEventKind::ScrollLeft if diff => self.scroll_diff_sideways(-SIDE_STEP),
            MouseEventKind::ScrollDown if diff && sideways => self.scroll_diff_sideways(SIDE_STEP),
            MouseEventKind::ScrollUp if diff && sideways => self.scroll_diff_sideways(-SIDE_STEP),
            MouseEventKind::ScrollDown if diff => self.scroll_diff(WHEEL_LINES),
            MouseEventKind::ScrollUp if diff => self.scroll_diff(-WHEEL_LINES),
            MouseEventKind::ScrollDown => self.move_in(pane, 1),
            MouseEventKind::ScrollUp => self.move_in(pane, -1),
            MouseEventKind::Down(MouseButton::Left) => {
                self.focus = pane;
                let (current, offset, len) = match pane {
                    Focus::Commits => (self.commit_idx, self.commit_offset, self.view.len()),
                    Focus::Files => (self.file_idx, self.file_offset, self.files().len()),
                    Focus::Diff => return,
                };
                // Rows sit between the pane's top and bottom borders.
                let row = mouse.row - self.panes[i].y;
                if !(1..self.panes[i].height.saturating_sub(1)).contains(&row) {
                    return;
                }
                let target = offset + row as usize - 1;
                if target < len {
                    self.move_in(pane, target as isize - current as isize);
                }
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        self.move_in(self.focus, delta);
    }

    fn move_in(&mut self, pane: Focus, delta: isize) {
        match pane {
            Focus::Commits => {
                let idx = step(self.commit_idx, delta, self.view.len());
                if idx != self.commit_idx {
                    self.commit_idx = idx;
                    self.file_idx = 0;
                    self.reset_diff_scroll();
                }
            }
            Focus::Files => {
                let len = self.files().len();
                let idx = step(self.file_idx, delta, len);
                if idx != self.file_idx {
                    self.file_idx = idx;
                    self.reset_diff_scroll();
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

    /// Moves the diff sideways, no further than it takes to show the end of
    /// its widest line.
    fn scroll_diff_sideways(&mut self, delta: isize) {
        let widest = match self.diff() {
            DiffView::Ready(lines) => lines.iter().map(|l| columns(&l.text)).max(),
            _ => None,
        };
        let last = widest.unwrap_or(0).saturating_sub(self.diff_width);
        self.diff_column = step(self.diff_column, delta, last + 1);
    }

    fn reset_diff_scroll(&mut self) {
        self.diff_scroll = 0;
        self.diff_column = 0;
    }
}

/// Terminal columns `text` takes up.
fn columns(text: &str) -> usize {
    Span::raw(text).width()
}

/// Where the first code match on a changed line starts and ends, in columns.
fn code_hit(line: &DiffLine, pattern: &str) -> Option<(usize, usize)> {
    if !matches!(line.kind, LineKind::Added | LineKind::Removed) {
        return None;
    }
    let found = find_code(&line.text[1..], pattern).into_iter().next()?;
    // Back to `line.text` offsets, past the leading `+`/`-`.
    let text = &line.text;
    Some((
        columns(&text[..found.start + 1]),
        columns(&text[..found.end + 1]),
    ))
}

/// Keys that only change the query (see `handle_key`).
fn is_query_edit(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('w') => true,
        KeyCode::Char(_) => !key.modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Backspace | KeyCode::Esc => true,
        _ => false,
    }
}

/// What a code search must look at for these fuzzy hits: each commit once, and
/// only the files the fuzzy query matched, since no other file can show up.
pub fn code_targets(commits: &[Commit], hits: Vec<Hit>) -> Vec<CodeTarget> {
    let mut seen = HashSet::new();
    hits.into_iter()
        .filter(|h| seen.insert(commits[h.commit].sha.as_str()))
        .map(|h| {
            let commit = &commits[h.commit];
            // Renames keep both names so git still pairs them.
            let paths = h.files.map(|files| {
                files
                    .iter()
                    .flat_map(|&i| {
                        let f = &commit.files[i];
                        std::iter::once(f.path.clone()).chain(f.old_path.clone())
                    })
                    .collect()
            });
            CodeTarget {
                sha: commit.sha.clone(),
                paths,
            }
        })
        .collect()
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

    /// `LEVI_BENCH_REPO=... cargo test --release code_search_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn code_search_speed() {
        let Ok(path) = std::env::var("LEVI_BENCH_REPO") else {
            return;
        };
        let root = std::path::PathBuf::from(path);
        let commits: Vec<Commit> = crate::git::log(&root, Source::Log)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let all: Vec<usize> = (0..commits.len()).collect();
        let search = |pattern: &str, targets: Vec<CodeTarget>| {
            let (tx, rx) = std::sync::mpsc::channel();
            let t = Instant::now();
            crate::worker::CodeSearcher::new(root.clone(), tx).start(CodeRequest {
                pattern: pattern.into(),
                targets,
                generation: 1,
            });
            let (mut hits, mut first) = (0, None);
            for msg in rx {
                match msg {
                    Msg::CodeHits(_, h) => {
                        hits += h.len();
                        first.get_or_insert(t.elapsed());
                    }
                    Msg::CodeDone(_, r) => {
                        r.unwrap();
                        break;
                    }
                    _ => {}
                }
            }
            (hits, first, t.elapsed())
        };
        for fuzzy in ["", "parse", "parse.c"] {
            let scoped = code_targets(&commits, filter::par_matches(fuzzy, &commits, &all));
            let n = scoped.len();
            let (hits, _, all) = search("git_parse_ssize_t", scoped);
            println!(
                "{fuzzy:>8} /c git_parse_ssize_t: {n:>5} commits searched, {hits} hits, {all:.2?}"
            );
        }
        let everything = code_targets(&commits, filter::par_matches("", &commits, &all));
        for pattern in [
            "xstrdup_or_null",
            "git_parse_ssize_t",
            "ODM_gear_nonexistent",
        ] {
            let (hits, first, all) = search(pattern, everything.clone());
            println!("{pattern:>22}: {hits:>5} commits, first {first:.2?}, all {all:.2?}");
        }
    }
}
