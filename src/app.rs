//! Application state and key handling.

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::text::Span;

use crate::filter::{self, Filter, Hit};
use crate::git::{self, Commit, DiffLine, FileChange, LineKind, Status};
use crate::layout::{Resize, Seam, Splits};
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
/// What Enter and Ctrl-Y say on the row of uncommitted changes.
const NO_SHA: &str = "uncommitted changes have no sha";
/// Two presses on a seam within this long put its layout back to the defaults.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

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
    /// What `Source` shows changed under levi.
    Moved(Source, Change),
    /// The commits a refresh found on top, from the load with this generation.
    Newer(u64, Result<Vec<Commit>, String>),
    /// The uncommitted changes, at first and whenever they change.
    Working(Vec<FileChange>),
}

/// How what levi shows changed since the watcher last looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// New commits on top, as after a commit or a pull: these revisions cover
    /// just them.
    Grew(Vec<String>),
    /// Anything else, like an amend, a rebase or a new reflog entry.
    Rewritten,
}

/// Search the diffs of `targets` for `pattern`. A higher generation supersedes the rest.
pub struct CodeRequest {
    pub pattern: String,
    pub targets: Vec<CodeTarget>,
    pub generation: u64,
}

/// Search the uncommitted changes for `pattern`, for the `/c` search with
/// this generation.
pub struct WorkingCodeRequest {
    pub pattern: String,
    pub target: CodeTarget,
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
    /// Search only these commits next time, the ones a refresh added; the
    /// rest were searched already.
    only: Option<HashSet<String>>,
    /// The uncommitted changes need searching: the search started, or they
    /// changed since.
    working_due: bool,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRequest {
    pub source: Source,
    pub generation: u64,
    /// Only the commits these revisions cover, to go on top of the list.
    pub newer: Option<Vec<String>>,
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

/// Ctrl-O's file: where it is in the working tree, and which commit to take
/// it from if it isn't there anymore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    pub sha: String,
    pub short: String,
    pub path: String,
    pub line: usize,
    /// The commit deletes the file, so its last version is the parent's.
    pub deleted: bool,
}

/// A place a hunk jump can land: the scroll that shows it, its line, and for a
/// code match, the columns it spans.
#[derive(Clone, Copy)]
struct Stop {
    scroll: usize,
    line: usize,
    hit: Option<(usize, usize)>,
}

/// Which end of a diff a hunk jump lands on when it crosses into it.
#[derive(Clone, Copy)]
enum Edge {
    First,
    Last,
}

/// A mouse drag in one pane: whole rows of a list, or text of the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub pane: Focus,
    /// Where the drag started.
    pub anchor: Spot,
    /// Where it is now.
    pub head: Spot,
}

/// A place in a pane's content, not on screen, so scrolling doesn't move it:
/// a list item and the screen column (which only tells a drag from a click),
/// or a diff line and display column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub row: usize,
    pub col: usize,
}

impl Selection {
    /// Its two ends in reading order.
    pub fn span(&self) -> (Spot, Spot) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    /// A click selects nothing; only moving away from where it started does.
    pub fn dragged(&self) -> bool {
        self.anchor != self.head
    }

    /// The selected rows, once the mouse has moved.
    pub fn rows(&self, pane: Focus) -> Option<RangeInclusive<usize>> {
        let (start, end) = self.span();
        (self.pane == pane && self.dragged()).then_some(start.row..=end.row)
    }
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
    /// Print this to stdout: the sha, or with `--pick-context` a reference.
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
    /// Enter prints a reference to what's on screen instead of the sha.
    pub pick_context: bool,
    /// Soft backgrounds for changed lines, when the terminal allows them.
    pub tints: Option<crate::tint::Tints>,
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
    /// The area the panes share, updated on every draw.
    pub body: Rect,
    /// Where the seams between panes sit, moved by dragging them.
    pub splits: Splits,
    /// The seam being dragged.
    resizing: Option<Resize>,
    /// The last press on a seam, to tell a double-click.
    seam_press: Option<(Seam, Instant)>,
    /// Where to land in a diff that a hunk jump crossed into, once it loads.
    pending_jump: Option<Edge>,
    /// What the mouse is dragging over. It stays shown after the drag, until
    /// the next key or click.
    pub selection: Option<Selection>,
    /// Text waiting to go to the clipboard (see `take_clipboard`).
    clipboard: Option<String>,
    /// A file and line waiting to open in an editor (see `take_open`).
    open: Option<OpenRequest>,
    /// A one-off message for the status line, like what was just copied.
    /// The next key or click clears it.
    pub notice: Option<String>,
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
    /// A refresh is loading the commits on top.
    refreshing: bool,
    /// Where to go back to once a reload brings the selected commit again.
    reselect: Option<Reselect>,
    /// The watcher should look now (see `take_check`).
    check: bool,
    /// The uncommitted changes as last reported; `None` until the first
    /// report. Shown as a row on top of the log while there are any.
    working: Option<Vec<FileChange>>,
    /// Counts the reports, to tell the row's states apart (see `git::WORKING`).
    working_version: u64,
    /// The row's sha before its last change, whose diff stays shown while the
    /// new one loads.
    working_before: Option<String>,
}

/// The selected commit, file and diff scroll, kept across a reload.
struct Reselect {
    sha: String,
    file_idx: usize,
    diff_scroll: usize,
    diff_column: usize,
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
            pick_context: false,
            tints: None,
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
            body: Rect::default(),
            splits: Splits::default(),
            resizing: None,
            seam_press: None,
            pending_jump: None,
            selection: None,
            clipboard: None,
            open: None,
            notice: None,
            exit: None,
            diffs: DiffCache::default(),
            requested: None,
            applied: String::new(),
            generation: 0,
            log_request: Some(LogRequest {
                source: Source::Log,
                generation: 0,
                newer: None,
            }),
            by_sha: HashMap::new(),
            code_generation: 0,
            code_cancel: false,
            jumped: None,
            refreshing: false,
            reselect: None,
            check: false,
            working: None,
            working_version: 0,
            working_before: None,
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
                self.notice = None;
                self.reselect = None;
                self.handle_key(key);
                // Any key ends a selection, once the key had a chance to use it
                // (Enter with --pick-context names the selected lines).
                self.selection = None;
            }
            Msg::Input(Event::Mouse(mouse)) => {
                self.settle();
                self.reselect = None;
                self.handle_mouse(mouse);
            }
            // Back from another tab or window, where a commit may have happened.
            Msg::Input(Event::FocusGained) => self.check = true,
            Msg::Input(_) => {}
            // Leftovers from a load that was superseded.
            Msg::Commits(generation, _)
            | Msg::LogDone(generation, _)
            | Msg::Newer(generation, _)
                if generation != self.generation => {}
            Msg::Commits(_, batch) => {
                self.settle();
                self.add_commits(batch);
            }
            Msg::LogDone(_, result) => {
                self.loading = false;
                self.log_error = result.err();
            }
            Msg::Moved(source, change) => {
                self.settle();
                self.moved(source, change);
            }
            Msg::Newer(_, result) => {
                self.refreshing = false;
                match result {
                    Ok(commits) => self.prepend_commits(commits),
                    // Whatever went wrong, a full load sorts it out.
                    Err(_) => self.reload(),
                }
            }
            Msg::Diff(key, result) => self.diffs.insert(key, result),
            Msg::Working(files) => {
                self.settle();
                self.set_working(files);
            }
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
            newer: None,
        });
        self.refreshing = false;
        self.commits.clear();
        self.view.clear();
        self.by_sha.clear();
        self.loading = true;
        self.log_error = None;
        if source == Source::Log
            && let Some(row) = self.working_row()
        {
            self.by_sha.insert(row.sha.clone(), vec![0]);
            self.commits.push(row);
            self.admit_row();
        }
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
                only: None,
                working_due: false,
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
        // The uncommitted changes go on their own (see `take_working_code_request`).
        let front = self
            .commits
            .first()
            .is_some_and(|c| git::is_working(&c.sha));
        code.working_due = front;
        let all: Vec<usize> = match code.only.take() {
            Some(only) => (0..self.commits.len())
                .filter(|&i| only.contains(&self.commits[i].sha))
                .collect(),
            None => (front as usize..self.commits.len()).collect(),
        };
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

    /// The search of the uncommitted changes, once a code search started and
    /// whenever they change during it, over the files its fuzzy part matched.
    pub fn take_working_code_request(&mut self) -> Option<WorkingCodeRequest> {
        let code = self.code.as_mut().filter(|c| c.working_due)?;
        code.working_due = false;
        let scope = code.scope.as_deref()?;
        let row = self.commits.first().filter(|c| git::is_working(&c.sha))?;
        let hits = filter::par_matches(scope, &self.commits[..1], &[0]);
        let target = code_targets(&self.commits[..1], hits).pop();
        let Some(target) = target else {
            // The fuzzy part doesn't match it, so it's not shown either way.
            let none = (row.sha.clone(), Vec::new());
            let generation = code.generation;
            self.add_code_hits(generation, vec![none]);
            return None;
        };
        Some(WorkingCodeRequest {
            pattern: code.pattern.clone(),
            target,
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
            // The uncommitted changes' latest result replaces the one before,
            // unless they changed again since.
            if git::is_working(&sha) {
                if self.commits.first().is_none_or(|c| c.sha != sha) {
                    continue;
                }
                code.hits.insert(sha, paths);
                self.view.retain(|h| h.commit != 0);
                if let Some(hit) = self.filter.matches(0, &self.commits[0]) {
                    self.view.extend(code.admit(&self.commits, hit));
                }
                continue;
            }
            let positions = self.by_sha.get(&sha).cloned().unwrap_or_default();
            // A search that started over after a refresh finds some again.
            if code.hits.insert(sha, paths).is_some() {
                continue;
            }
            for i in positions {
                if let Some(hit) = self.filter.matches(i, &self.commits[i]) {
                    self.view.extend(code.admit(&self.commits, hit));
                }
            }
        }
        self.resort(selected);
        self.try_reselect();
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

    /// Where the diff can jump to: each hunk, or each code match while `/c` is
    /// on, in order. Empty until the diff is loaded.
    fn stops(&self) -> Vec<Stop> {
        let DiffView::Ready(lines) = self.diff() else {
            return Vec::new();
        };
        let pattern = self.code.as_ref().map(|c| c.pattern.as_str());
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| match pattern {
                Some(pattern) => {
                    let hit = code_hit(l, pattern)?;
                    let scroll = i.saturating_sub(JUMP_CONTEXT);
                    Some(Stop {
                        scroll,
                        line: i,
                        hit: Some(hit),
                    })
                }
                None => (l.kind == LineKind::Hunk).then_some(Stop {
                    scroll: i,
                    line: i,
                    hit: None,
                }),
            })
            .collect()
    }

    fn go_to(&mut self, stop: Stop) {
        match stop.hit {
            Some((before, through)) => self.show_code_hit(stop.line, before, through),
            None => self.diff_scroll = stop.scroll,
        }
    }

    /// Scrolls the diff to the next (`forward`) or previous hunk, or code match
    /// while `/c` is on. Past a file's last one it goes on to the next file's
    /// first, and back from the first to the previous file's last, like `]c`
    /// in vim; at the commit's first and last it stays put.
    fn jump_in_diff(&mut self, forward: bool) {
        let stops = self.stops();
        let current = self.diff_scroll;
        let stop = if forward {
            stops.into_iter().find(|s| s.scroll > current)
        } else {
            stops.into_iter().rfind(|s| s.scroll < current)
        };
        if let Some(stop) = stop {
            return self.go_to(stop);
        }
        let (step, edge) = if forward {
            (1, Edge::First)
        } else {
            (-1, Edge::Last)
        };
        let before = self.file_idx;
        self.move_in(Focus::Files, step);
        if self.file_idx != before {
            // The other file's diff may still be loading.
            self.pending_jump = Some(edge);
            self.apply_pending_jump();
        }
    }

    /// Finishes a jump into another file once its diff is in.
    fn apply_pending_jump(&mut self) {
        let Some(edge) = self.pending_jump else {
            return;
        };
        if matches!(self.diff(), DiffView::Loading) {
            return;
        }
        self.pending_jump = None;
        let stops = self.stops();
        let stop = match edge {
            Edge::First => stops.first(),
            Edge::Last => stops.last(),
        };
        if let Some(&stop) = stop {
            self.go_to(stop);
        }
    }

    fn copy(&mut self, text: String, notice: String) {
        self.clipboard = Some(text);
        self.notice = Some(notice);
    }

    /// Text to put on the clipboard, if something was just copied. The main
    /// loop does it, since it owns the terminal.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// What `--pick-context` prints on Enter: the commit, and when a file is
    /// selected, its path and the lines in view (or under the drag selection),
    /// on one line, for a person or an agent to follow:
    /// `commit d3a93ab ("Stack the lists"), src/ui.rs lines 27-40`.
    fn context(&self) -> Option<String> {
        let commit = self.commit()?;
        let text = commit
            .reflog
            .as_ref()
            .map_or(&commit.subject, |r| &r.action);
        let mut out = if git::is_working(&commit.sha) {
            "the uncommitted changes".to_owned()
        } else {
            format!("commit {} (\"{text}\")", commit.short)
        };
        let Some(file) = self.file() else {
            return Some(out);
        };
        out.push_str(&format!(", {}", file.path));
        let DiffView::Ready(lines) = self.diff() else {
            return Some(out);
        };
        let deleted = file.status == Status::Deleted;
        let rows = match self.selection.and_then(|s| s.rows(Focus::Diff)) {
            Some(rows) => rows,
            None => self.diff_scroll..=self.diff_scroll + self.diff_height.max(1) - 1,
        };
        if let Some((first, last)) = lines_in(lines, rows, deleted) {
            if first == last {
                out.push_str(&format!(" line {first}"));
            } else {
                out.push_str(&format!(" lines {first}-{last}"));
            }
        }
        Some(out)
    }

    /// A file and line to open in an editor, if Ctrl-O asked for one. The main
    /// loop does it, since it owns the terminal.
    pub fn take_open(&mut self) -> Option<OpenRequest> {
        self.open.take()
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
        self.try_reselect();
    }

    /// Whether the watcher should look now. Asks once.
    pub fn take_check(&mut self) -> bool {
        mem::take(&mut self.check)
    }

    /// Follows a change the watcher saw: loads just the new commits on top
    /// when the history only grew, and everything again otherwise.
    fn moved(&mut self, source: Source, change: Change) {
        if source != self.source {
            return;
        }
        match change {
            // Partial loads don't stack: a change during any load reloads.
            Change::Grew(revs) if !self.loading && !self.refreshing => {
                self.generation += 1;
                self.log_request = Some(LogRequest {
                    source,
                    generation: self.generation,
                    newer: Some(revs),
                });
                self.refreshing = true;
            }
            _ => self.reload(),
        }
    }

    /// Loads everything again, coming back to the selected commit, file and
    /// scroll once the commit is in, and running `/c` again.
    fn reload(&mut self) {
        if self.reselect.is_none() {
            self.reselect = self.commit().map(|c| Reselect {
                sha: c.sha.clone(),
                file_idx: self.file_idx,
                diff_scroll: self.diff_scroll,
                diff_column: self.diff_column,
            });
        }
        self.switch_source(self.source);
        if let Some(code) = &self.code {
            let pattern = code.pattern.clone();
            self.restart_code(Some(pattern));
        }
        self.commit_idx = 0;
        self.file_idx = 0;
        self.reset_diff_scroll();
        self.notice = Some(match self.source {
            Source::Log => "history changed, reloaded".into(),
            Source::Reflog => "reflog updated".into(),
        });
    }

    /// Goes back to the commit selected before a reload, once it's in view.
    fn try_reselect(&mut self) {
        let Some(want) = &self.reselect else {
            return;
        };
        let Some(&i) = self.by_sha.get(&want.sha).and_then(|is| is.first()) else {
            return;
        };
        let Some(idx) = self.view.iter().position(|h| h.commit == i) else {
            return;
        };
        let want = self.reselect.take().expect("checked above");
        self.commit_idx = idx;
        self.file_idx = want.file_idx;
        self.diff_scroll = want.diff_scroll;
        self.diff_column = want.diff_column;
    }

    /// Puts the commits a refresh found on top of the list, in log order,
    /// keeping the selection on the same commit. `/c` searches just them.
    fn prepend_commits(&mut self, fresh: Vec<Commit>) {
        // One may have come in with the full load too, if it landed just then.
        let fresh: Vec<Commit> = fresh
            .into_iter()
            .filter(|c| !self.by_sha.contains_key(&c.sha))
            .collect();
        let n = fresh.len();
        if n == 0 {
            return;
        }
        // Below the row of uncommitted changes, if there is one.
        let at = self.front();
        let selected = self.hit().map(|h| {
            if h.commit >= at {
                h.commit + n
            } else {
                h.commit
            }
        });
        let rest = self.commits.split_off(at);
        self.commits.extend(fresh);
        self.commits.extend(rest);
        self.shift(at, n as isize);
        for i in at..at + n {
            self.by_sha
                .entry(self.commits[i].sha.clone())
                .or_default()
                .push(i);
            // Under /c, the new commits show once their diffs match.
            if self.code.is_none()
                && let Some(hit) = self.filter.matches(i, &self.commits[i])
            {
                self.view.push(hit);
            }
        }
        // Equal scores keep log order, so with no query this puts them on top.
        self.resort(selected);
        // Row numbers in the list moved.
        if self.selection.is_some_and(|s| s.pane == Focus::Commits) {
            self.selection = None;
        }
        if let Some(code) = &mut self.code
            && code.scope.is_some()
        {
            self.code_generation += 1;
            code.generation = self.code_generation;
            // A finished search only needs the new commits; one still running
            // starts over on its whole scope, keeping what it found.
            if code.finished {
                code.only = Some(
                    self.commits[at..at + n]
                        .iter()
                        .map(|c| c.sha.clone())
                        .collect(),
                );
            }
            code.scope = None;
            code.finished = false;
            code.progress = (0, 0);
            let now = Instant::now();
            code.typed_at = now.checked_sub(CODE_DEBOUNCE).unwrap_or(now);
            self.code_cancel = true;
        }
        let plural = if n == 1 { "" } else { "s" };
        self.notice = Some(format!("{n} new commit{plural}"));
    }

    /// Whether the first commit is the row of uncommitted changes.
    pub fn has_working_row(&self) -> bool {
        self.commits
            .first()
            .is_some_and(|c| git::is_working(&c.sha))
    }

    /// Where the commits proper start: after the row of uncommitted changes.
    fn front(&self) -> usize {
        self.has_working_row() as usize
    }

    /// Whether the row of uncommitted changes is selected.
    pub fn on_working_row(&self) -> bool {
        self.commit().is_some_and(|c| git::is_working(&c.sha))
    }

    /// The row of uncommitted changes, as a commit the lists can hold, while
    /// there are any.
    fn working_row(&self) -> Option<Commit> {
        let files = self.working.as_ref().filter(|f| !f.is_empty())?;
        Some(Commit {
            sha: format!("{}{}", git::WORKING, self.working_version),
            short: "working".into(),
            author: String::new(),
            date: String::new(),
            subject: "uncommitted changes".into(),
            reflog: None,
            files: files.clone(),
        })
    }

    /// Shows the row of uncommitted changes, at index 0, if the query lets it.
    fn admit_row(&mut self) {
        if let Some(hit) = self.filter.matches(0, &self.commits[0]) {
            match &self.code {
                Some(code) => self.view.extend(code.admit(&self.commits, hit)),
                None => self.view.push(hit),
            }
        }
    }

    /// Moves the commits from index `from` on by `delta` places, in every
    /// index that points at them, for an insertion or removal there.
    fn shift(&mut self, from: usize, delta: isize) {
        let moved = |i: &mut usize| {
            if *i >= from {
                *i = i.saturating_add_signed(delta);
            }
        };
        self.by_sha.values_mut().flatten().for_each(moved);
        self.view.iter_mut().for_each(|h| moved(&mut h.commit));
    }

    /// Follows the uncommitted changes the watcher reported: the row on top
    /// appears, changes or goes. A changed row keeps its selection, file and
    /// scroll, so an agent's edits can be watched as they land; the first
    /// report selects it, if nothing else was.
    fn set_working(&mut self, files: Vec<FileChange>) {
        let first = self.working.is_none();
        let listed = self.working.as_ref() != Some(&files);
        self.working = Some(files);
        self.working_version += 1;
        if self.source != Source::Log {
            return;
        }
        let had = self.has_working_row();
        let selected = self.hit().map(|h| h.commit);
        let on_row = had && selected == Some(0);
        let mut before = None;
        if had {
            let old = self.commits.remove(0);
            self.by_sha.remove(&old.sha);
            self.view.retain(|h| h.commit != 0);
            self.shift(1, -1);
            let paths = self.code.as_mut().and_then(|c| c.hits.remove(&old.sha));
            before = Some((old.sha, paths));
        }
        if let Some(row) = self.working_row() {
            self.shift(0, 1);
            self.by_sha.insert(row.sha.clone(), vec![0]);
            if let Some(code) = &mut self.code {
                // What the search last found stays until it's searched again.
                if let Some((_, Some(paths))) = &before {
                    code.hits.insert(row.sha.clone(), paths.clone());
                }
                code.working_due |= code.scope.is_some();
            }
            if let Some(jumped) = &mut self.jumped
                && before.as_ref().is_some_and(|(sha, _)| *sha == jumped.0)
            {
                jumped.0 = row.sha.clone();
            }
            self.commits.insert(0, row);
            self.admit_row();
        }
        self.working_before = before.map(|(sha, _)| sha);
        let has = self.has_working_row();
        let target = match selected {
            _ if first && has && self.commit_idx == 0 => Some(0),
            Some(0) if had => has.then_some(0),
            Some(i) => Some(i - had as usize + has as usize),
            None => None,
        };
        self.resort(target);
        if self.selection.is_some_and(|s| s.pane == Focus::Commits) {
            self.selection = None;
        }
        // Another commit is selected now: the row, at first, or the top one,
        // once the row is gone.
        if (target == Some(0) && has) != on_row {
            self.commit_idx = self.commit_idx.min(self.view.len().saturating_sub(1));
            self.file_idx = 0;
            self.reset_diff_scroll();
        }
        if !first && has && listed {
            self.notice = Some("working tree changed".into());
        }
    }

    /// The diff the selection needs, if it isn't cached and wasn't already asked for.
    pub fn take_diff_request(&mut self) -> Option<DiffRequest> {
        self.settle();
        self.jump_to_code();
        self.apply_pending_jump();
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
        let file = self.file();
        let cached = self.diffs.get(&diff_key(&commit.sha, file)).or_else(|| {
            // The uncommitted changes changed: the last diff stays until the new one is in.
            let before = self.working_before.as_ref()?;
            git::is_working(&commit.sha)
                .then(|| self.diffs.get(&diff_key(before, file)))
                .flatten()
        });
        match cached {
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
            KeyCode::Char('y') if ctrl && self.on_working_row() => {
                self.notice = Some(NO_SHA.into())
            }
            KeyCode::Char('y') if ctrl => {
                if let Some(commit) = self.commit() {
                    let notice = format!("copied {}", commit.short);
                    self.copy(commit.sha.clone(), notice);
                }
            }
            KeyCode::Char('o') if ctrl => match (self.commit(), self.file()) {
                (Some(commit), Some(file)) => {
                    // A file this commit deletes only has lines on the old side.
                    let deleted = file.status == Status::Deleted;
                    let line = match self.diff() {
                        DiffView::Ready(lines) => line_at(lines, self.diff_scroll, deleted),
                        _ => None,
                    };
                    self.open = Some(OpenRequest {
                        sha: commit.sha.clone(),
                        short: commit.short.clone(),
                        path: file.path.clone(),
                        line: line.unwrap_or(1),
                        deleted,
                    });
                }
                _ => self.notice = Some("no file to open".into()),
            },
            KeyCode::Enter if !self.pick_context && self.on_working_row() => {
                self.notice = Some(NO_SHA.into());
            }
            KeyCode::Enter => {
                let picked = if self.pick_context {
                    self.context()
                } else {
                    self.commit().map(|c| c.sha.clone())
                };
                if let Some(picked) = picked {
                    self.exit = Some(Exit::Pick(picked));
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
        // A drag carries on wherever the pointer goes, even off its pane.
        match (mouse.kind, self.resizing) {
            (MouseEventKind::Drag(MouseButton::Left), Some(resize)) => {
                return self.splits.drag(self.body, self.panes, resize, at);
            }
            (MouseEventKind::Up(MouseButton::Left), Some(_)) => {
                self.resizing = None;
                return;
            }
            (MouseEventKind::Drag(MouseButton::Left), None) => return self.drag(mouse),
            (MouseEventKind::Up(MouseButton::Left), None) => return self.release(),
            (MouseEventKind::Down(MouseButton::Left), _) => {
                if let Some(resize) = Splits::seam_at(self.body, self.panes, at) {
                    return self.press_seam(resize);
                }
            }
            _ => {}
        }
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
                self.notice = None;
                self.selection = None;
                // Clicks count only between the pane's borders.
                let Some(inner) = inside(self.panes[i]).filter(|r| r.contains(at)) else {
                    return;
                };
                let row = (mouse.row - inner.y) as usize;
                let (current, offset, len) = match pane {
                    Focus::Commits => (self.commit_idx, self.commit_offset, self.view.len()),
                    Focus::Files => (self.file_idx, self.file_offset, self.files().len()),
                    Focus::Diff => {
                        let col = self.diff_column + (mouse.column - inner.x) as usize;
                        let spot = Spot {
                            row: self.diff_scroll + row,
                            col,
                        };
                        self.selection = Some(Selection {
                            pane,
                            anchor: spot,
                            head: spot,
                        });
                        return;
                    }
                };
                let target = offset + row;
                if target < len {
                    self.move_in(pane, target as isize - current as isize);
                    let spot = Spot {
                        row: target,
                        col: mouse.column as usize,
                    };
                    self.selection = Some(Selection {
                        pane,
                        anchor: spot,
                        head: spot,
                    });
                }
            }
            _ => {}
        }
    }

    /// Starts dragging a seam, or on a second press soon after the first,
    /// puts the layout back to its defaults.
    fn press_seam(&mut self, resize: Resize) {
        let now = Instant::now();
        let double = self
            .seam_press
            .is_some_and(|(seam, at)| seam == resize.seam && now - at < DOUBLE_CLICK);
        if double {
            self.splits.reset(self.body);
            self.seam_press = None;
        } else {
            self.resizing = Some(resize);
            self.seam_press = Some((resize.seam, now));
        }
    }

    /// Moves the end of the selection to the pointer, kept inside the pane.
    /// Past the diff's top or bottom edge, the diff scrolls along.
    fn drag(&mut self, mouse: MouseEvent) {
        let Some(mut sel) = self.selection else {
            return;
        };
        // `panes` is in `Focus` order.
        let i = sel.pane as usize;
        let Some(inner) = inside(self.panes[i]) else {
            return;
        };
        let (top, bottom) = (inner.y, inner.bottom() - 1);
        let row = (mouse.row.clamp(top, bottom) - top) as usize;
        let col = (mouse.column.clamp(inner.x, inner.right() - 1) - inner.x) as usize;
        sel.head = match sel.pane {
            Focus::Diff => {
                if mouse.row < top {
                    self.scroll_diff(-1);
                } else if mouse.row > bottom {
                    self.scroll_diff(1);
                }
                Spot {
                    row: self.diff_scroll + row,
                    col: self.diff_column + col,
                }
            }
            list => {
                let (offset, len) = match list {
                    Focus::Commits => (self.commit_offset, self.view.len()),
                    _ => (self.file_offset, self.files().len()),
                };
                if len == 0 {
                    return;
                }
                Spot {
                    row: (offset + row).min(len - 1),
                    col: mouse.column as usize,
                }
            }
        };
        self.selection = Some(sel);
    }

    /// The end of a drag copies what it covered; a click selects nothing.
    fn release(&mut self) {
        let Some(sel) = self.selection else {
            return;
        };
        if !sel.dragged() {
            self.selection = None;
            return;
        }
        if let Some((text, notice)) = self.selected_text(&sel) {
            self.copy(text, notice);
        }
    }

    /// What a selection copies, and how to say so. Lists copy whole values,
    /// not their shortened rows: full paths, and commits as `git log
    /// --oneline` would print them. The diff copies exactly what's selected.
    fn selected_text(&self, sel: &Selection) -> Option<(String, String)> {
        let (start, end) = sel.span();
        let rows = start.row..=end.row;
        let lines: Vec<String> = match sel.pane {
            Focus::Commits => self
                .view
                .get(rows)?
                .iter()
                .map(|hit| {
                    let c = &self.commits[hit.commit];
                    let text = c.reflog.as_ref().map_or(&c.subject, |r| &r.action);
                    format!("{} {text}", c.short)
                })
                .collect(),
            Focus::Files => self
                .files()
                .get(rows)?
                .iter()
                .map(|f| f.path.clone())
                .collect(),
            Focus::Diff => {
                let DiffView::Ready(diff) = self.diff() else {
                    return None;
                };
                let last = end.row.min(diff.len().checked_sub(1)?);
                (start.row..=last)
                    .map(|l| {
                        let from = if l == start.row { start.col } else { 0 };
                        let to = if l == end.row {
                            end.col + 1
                        } else {
                            usize::MAX
                        };
                        slice_columns(&diff[l].text, from, to).to_owned()
                    })
                    .collect()
            }
        };
        if lines.is_empty() || lines.iter().all(String::is_empty) {
            return None;
        }
        let noun = match sel.pane {
            Focus::Commits => "commit",
            Focus::Files => "path",
            Focus::Diff => "line",
        };
        let plural = if lines.len() == 1 { "" } else { "s" };
        let notice = format!("copied {} {noun}{plural}", lines.len());
        Some((lines.join("\n"), notice))
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

    /// For a newly selected commit or file, whose diff starts over.
    fn reset_diff_scroll(&mut self) {
        self.diff_scroll = 0;
        self.diff_column = 0;
        self.selection = None;
        self.pending_jump = None;
    }
}

/// The line of the new file (or the old one, for `old`) that diff line `top`
/// shows, or the first one after it that does: what an editor should open at
/// when the diff is scrolled to `top`. A line only on the other side maps to
/// where it would be. None with no hunks at all.
pub fn line_at(lines: &[DiffLine], top: usize, old: bool) -> Option<usize> {
    // `@@ -a,b +c,d @@`: the old side starts at line a, the new one at c.
    let marker = if old { " -" } else { " +" };
    let this_side = if old {
        LineKind::Removed
    } else {
        LineKind::Added
    };
    let mut next = None;
    for (i, l) in lines.iter().enumerate() {
        match l.kind {
            LineKind::Hunk => {
                next = l.text.split_once(marker).and_then(|(_, rest)| {
                    let digits = rest.split(|c: char| !c.is_ascii_digit()).next()?;
                    digits.parse::<usize>().ok()
                });
            }
            LineKind::Context | LineKind::Added | LineKind::Removed if i >= top => return next,
            k if k == LineKind::Context || k == this_side => next = next.map(|n| n + 1),
            _ => {}
        }
    }
    next.map(|n| n.saturating_sub(1).max(1))
}

/// The first and last file line that diff rows `rows` show, on the new side
/// (or the old one, for `old`). None when those rows hold no code.
pub fn lines_in(
    lines: &[DiffLine],
    rows: RangeInclusive<usize>,
    old: bool,
) -> Option<(usize, usize)> {
    let this_side = if old {
        LineKind::Removed
    } else {
        LineKind::Added
    };
    let mut next: Option<usize> = None;
    let mut found: Option<(usize, usize)> = None;
    for (i, l) in lines.iter().enumerate() {
        if i > *rows.end() {
            break;
        }
        match l.kind {
            LineKind::Hunk => {
                let marker = if old { " -" } else { " +" };
                next = l.text.split_once(marker).and_then(|(_, rest)| {
                    let digits = rest.split(|c: char| !c.is_ascii_digit()).next()?;
                    digits.parse::<usize>().ok()
                });
            }
            k if k == LineKind::Context || k == this_side => {
                if let Some(n) = next {
                    if rows.contains(&i) {
                        found = Some(found.map_or((n, n), |(first, _)| (first, n)));
                    }
                    next = Some(n + 1);
                }
            }
            _ => {}
        }
    }
    found
}

/// A pane's area between its borders, if there's any.
fn inside(pane: Rect) -> Option<Rect> {
    (pane.width > 2 && pane.height > 2).then(|| Rect {
        x: pane.x + 1,
        y: pane.y + 1,
        width: pane.width - 2,
        height: pane.height - 2,
    })
}

/// The part of `text` whose chars start at display columns `from..to`.
pub fn slice_columns(text: &str, from: usize, to: usize) -> &str {
    let (mut start, mut end) = (None, text.len());
    let mut col = 0;
    for (i, c) in text.char_indices() {
        if col >= to {
            end = i;
            break;
        }
        if start.is_none() && col >= from {
            start = Some(i);
        }
        col += columns(&text[i..i + c.len_utf8()]);
    }
    let start = start.unwrap_or(end);
    &text[start.min(end)..end]
}

/// Terminal columns `text` takes up.
pub fn columns(text: &str) -> usize {
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
