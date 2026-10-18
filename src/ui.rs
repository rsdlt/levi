//! Rendering: search bar on top, commits | files | diff below, or on narrow
//! terminals commits | files above the diff.

use std::ops::Range;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::app::{App, DiffView, Focus, columns};
use crate::git::{DiffLine, LineKind, Status};
use crate::query::{Source, find_code};

const ACCENT: Color = Color::Cyan;

/// The bottom row's key hints, most useful first. Those that don't fit the
/// terminal's width are dropped from the end.
const HINTS: [(&str, &str); 9] = [
    ("↑↓", "move"),
    ("enter", "pick"),
    ("/c", "search code"),
    ("/r", "reflog"),
    ("tab", "pane"),
    ("esc", "clear/quit"),
    ("pgup/pgdn", "scroll"),
    ("S-↑↓", "hunk"),
    ("S-←→", "sideways"),
];

/// Below this many columns, three side-by-side panes leave the diff too
/// narrow to read, so the lists go above it instead.
const STACK_BELOW: u16 = 120;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [search, body, hints] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [commits, files, diff] = if body.width < STACK_BELOW {
        let [lists, diff] =
            Layout::vertical([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(body);
        let [commits, files] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .areas(lists);
        [commits, files, diff]
    } else {
        Layout::horizontal([
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(55),
        ])
        .areas(body)
    };
    app.panes = [commits, files, diff];

    draw_search(frame, app, search);
    draw_commits(frame, app, commits);
    draw_files(frame, app, files);
    draw_diff(frame, app, diff);
    draw_hints(frame, hints);
    if app.no_color {
        strip_colors(frame.buffer_mut());
    }
}

/// Drops every color. What a background color marked, like the selection or a
/// code match, is shown in reverse video instead, so it still stands out.
fn strip_colors(buf: &mut Buffer) {
    for cell in &mut buf.content {
        if cell.bg != Color::Reset {
            cell.modifier.insert(Modifier::REVERSED);
        }
        cell.fg = Color::Reset;
        cell.bg = Color::Reset;
    }
}

fn draw_hints(frame: &mut Frame, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    let mut width = 1;
    for (key, action) in HINTS {
        let hint = [
            Span::styled(key, Style::new().fg(ACCENT).bold()),
            Span::raw(" "),
            Span::styled(action, Style::new().dim()),
            Span::raw("  "),
        ];
        // The trailing gap may run past the edge; the hint itself must not.
        let needed: usize = hint[..3].iter().map(Span::width).sum();
        if width + needed > area.width as usize {
            break;
        }
        width += needed + hint[3].width();
        spans.extend(hint);
    }
    frame.render_widget(Line::from(spans), area);
}

fn pane(title: &str, focused: bool) -> Block<'_> {
    let block = Block::bordered().title(format!(" {title} "));
    if focused {
        block
            .border_style(Style::new().fg(ACCENT))
            .title_style(Modifier::BOLD)
    } else {
        block.border_style(Style::new().dark_gray())
    }
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::DIM)
    }
}

fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let line = Line::from(vec![
        Span::styled("> ", Style::new().fg(ACCENT).bold()),
        Span::raw(&app.query),
    ]);
    // The terminal's own cursor, after the query and inside the border, so it
    // takes the user's cursor style and input methods know where typing goes.
    if area.width > 2 && area.height > 2 {
        let x = area.x + 1 + line.width() as u16;
        frame.set_cursor_position((x.min(area.right() - 2), area.y + 1));
    }
    let count = if app.filter.is_empty() && app.code.is_none() {
        app.commits.len().to_string()
    } else {
        format!("{}/{}", app.view.len(), app.commits.len())
    };
    let noun = match (app.source, &app.rev) {
        (Source::Log, Some(rev)) => &format!("commits in {rev}"),
        (Source::Log, None) => "commits",
        (Source::Reflog, _) => "reflog entries",
    };
    let mut status = match &app.log_error {
        Some(err) => Line::styled(format!(" {err} "), Color::Red),
        None if app.loading => Line::from(format!(" {count} {noun}… ")),
        None => Line::from(format!(" {count} {noun} ")),
    };
    match &app.code {
        Some(code) if code.error.is_some() => status.push_span(Span::styled(
            format!(
                "· code search failed: {} ",
                code.error.as_deref().unwrap_or_default()
            ),
            Color::Red,
        )),
        Some(code) if !code.finished => {
            let (done, total) = code.progress;
            let percent = (done * 100).checked_div(total).unwrap_or(0);
            status.push_span(Span::styled(
                format!("· searching code {percent}% "),
                Style::new().fg(ACCENT),
            ));
        }
        _ => {}
    }
    let mut title = vec![Span::raw(" "), "levi".bold()];
    if app.source == Source::Reflog {
        title.push(Span::styled(" · reflog", Style::new().fg(ACCENT)));
    }
    if let Some(code) = &app.code {
        title.push(Span::styled(
            format!(" · code: {}", code.pattern),
            Style::new().fg(ACCENT),
        ));
    }
    title.push(Span::raw(" "));
    let block = Block::bordered()
        .title(Line::from(title))
        .title_bottom(status.right_aligned())
        .border_style(Style::new().dark_gray());
    frame.render_widget(Paragraph::new(line).block(block), area);
}

/// Scrolls `offset` just enough to keep `selected` within `height` rows.
fn follow(offset: &mut usize, selected: usize, height: usize) {
    // With no room at all, the selection still has to be at or past the offset.
    if selected < *offset || height == 0 {
        *offset = selected;
    } else if selected >= *offset + height {
        *offset = selected + 1 - height;
    }
}

/// Renders rows `offset..` of a list whose items were built for that window only,
/// so cost doesn't grow with the length of the log.
fn draw_window(
    frame: &mut Frame,
    area: Rect,
    block: Block,
    items: Vec<ListItem>,
    row: usize,
    focused: bool,
) {
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused));
    let mut state = ListState::default().with_selected(Some(row));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_commits(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Commits;
    let title = match app.source {
        Source::Log => "commits",
        Source::Reflog => "reflog",
    };
    let block = pane(title, focused);
    let height = block.inner(area).height as usize;
    follow(&mut app.commit_offset, app.commit_idx, height);

    let App {
        view,
        commits,
        filter,
        ..
    } = app;
    let mut items: Vec<ListItem> = view
        .iter()
        .skip(app.commit_offset)
        .take(height)
        .map(|hit| {
            let c = &commits[hit.commit];
            let sha_style = if hit.score == u32::MAX {
                matched(Style::new().fg(Color::Yellow))
            } else {
                Style::new().fg(Color::Yellow)
            };
            let mut spans = vec![Span::styled(c.short.as_str(), sha_style), Span::raw(" ")];
            // In the reflog, what happened says more than the commit's subject.
            let text = c.reflog.as_ref().map_or(&c.subject, |r| &r.action);
            spans.extend(highlighted(
                text,
                &filter.highlights(text, false),
                Style::new(),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();
    if items.is_empty() && !app.query.trim().is_empty() {
        let searching = app.loading || app.code.as_ref().is_some_and(|c| !c.finished);
        let note = if searching {
            "searching…"
        } else {
            "no matches"
        };
        items.push(ListItem::new(Line::styled(note, Style::new().dim())));
    }
    let row = app.commit_idx - app.commit_offset;
    draw_window(frame, area, block, items, row, focused);
}

fn draw_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Files;
    let block = pane("files", focused);
    let inner = block.inner(area);
    let height = inner.height as usize;
    follow(&mut app.file_offset, app.file_idx, height);
    // After the status letter and its space.
    let width = (inner.width as usize).saturating_sub(2);

    let App {
        view,
        commits,
        filter,
        ..
    } = app;
    let Some(hit) = view.get(app.commit_idx) else {
        frame.render_widget(block, area);
        return;
    };
    let files = &commits[hit.commit].files;
    let shown: Vec<usize> = hit
        .files
        .clone()
        .unwrap_or_else(|| (0..files.len()).collect());
    let items: Vec<ListItem> = shown
        .iter()
        .skip(app.file_offset)
        .take(height)
        .map(|&i| {
            let f = &files[i];
            let mut spans = vec![
                Span::styled(f.status.letter().to_string(), status_style(f.status)),
                Span::raw(" "),
            ];
            let new = (f.path.as_str(), filter.highlights(&f.path, true));
            let parts = match &f.old_path {
                Some(old) => renamed((old, filter.highlights(old, true)), new, width),
                None => vec![Part::new(new.0, new.1, Style::new())],
            };
            spans.extend(fitted(parts, width));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let row = app.file_idx - app.file_offset;
    draw_window(frame, area, block, items, row, focused);
}

/// A piece of a files-pane row: text, the chars of it to highlight, its style.
struct Part<'a> {
    text: &'a str,
    marks: Vec<usize>,
    style: Style,
}

impl<'a> Part<'a> {
    fn new(text: &'a str, marks: Vec<usize>, style: Style) -> Self {
        Part { text, marks, style }
    }

    fn plain(text: &'a str) -> Self {
        Part::new(text, Vec::new(), Style::new().dim())
    }
}

/// A rename the way `git diff --stat` writes it, the shared directories once:
/// `src/{old.rs → new.rs}` or `src/{a → b}/mod.rs`. Each path comes with its
/// highlighted chars. When even the part that changed can't fit `width`, it's
/// just the new path, so a cut never lands inside the rename.
fn renamed<'a>(
    old: (&'a str, Vec<usize>),
    new: (&'a str, Vec<usize>),
    width: usize,
) -> Vec<Part<'a>> {
    let ((old, old_marks), (new, new_marks)) = (old, new);
    // Byte lengths of the shared leading directories and trailing ones, each
    // ending or starting at a `/`, while both sides keep something between.
    let head = common_dirs(old.bytes(), new.bytes());
    let tail = common_dirs(old[head..].bytes().rev(), new[head..].bytes().rev());
    let (old_end, new_end) = (old.len() - tail, new.len() - tail);
    // Highlights are char positions; these take the ones inside a byte range.
    let within = |path: &str, marks: &[usize], range: Range<usize>| -> Vec<usize> {
        let from = path[..range.start].chars().count();
        let to = path[..range.end].chars().count();
        marks
            .iter()
            .filter(|&&m| (from..to).contains(&m))
            .map(|&m| m - from)
            .collect()
    };
    let bright = Style::new();
    // The changed part with what follows it: `{old → new}/rest`, or all of
    // `old → new` when nothing is shared up front.
    let changed = columns(&old[head..]) + columns(" → ") + columns(&new[head..]);
    let braces = if head > 0 || tail > 0 { 2 } else { 0 };
    let whole = columns(&new[..head]) + changed + braces;
    if whole > width && changed + braces + columns("…") > width {
        return vec![Part::new(new, new_marks, bright)];
    }
    if braces == 0 {
        return vec![
            Part::new(old, old_marks, bright.dim()),
            Part::plain(" → "),
            Part::new(new, new_marks, bright),
        ];
    }
    vec![
        Part::new(&new[..head], within(new, &new_marks, 0..head), bright),
        Part::plain("{"),
        Part::new(
            &old[head..old_end],
            within(old, &old_marks, head..old_end),
            bright.dim(),
        ),
        Part::plain(" → "),
        Part::new(
            &new[head..new_end],
            within(new, &new_marks, head..new_end),
            bright,
        ),
        Part::plain("}"),
        Part::new(
            &new[new_end..],
            within(new, &new_marks, new_end..new.len()),
            bright,
        ),
    ]
}

/// How many leading bytes `a` and `b` share, cut back to just after a `/`
/// (or, read in reverse, to a `/`), leaving both with something past it.
fn common_dirs(a: impl Iterator<Item = u8>, b: impl Iterator<Item = u8>) -> usize {
    let (a, b): (Vec<u8>, Vec<u8>) = (a.collect(), b.collect());
    let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let shared = shared
        .min(a.len().saturating_sub(1))
        .min(b.len().saturating_sub(1));
    a[..shared]
        .iter()
        .rposition(|&c| c == b'/')
        .map_or(0, |i| i + 1)
}

/// The parts in at most `width` columns, highlighted. Too long a row loses its
/// start, at a `/` when it can, so its end, the file name, always shows:
/// `…/components/Button.tsx`.
fn fitted<'a>(parts: Vec<Part<'a>>, width: usize) -> Vec<Span<'a>> {
    let whole: String = parts.iter().map(|p| p.text).collect();
    let mut skip = cut(&whole, width);
    let mut spans = Vec::new();
    if skip > 0 {
        spans.push(Span::styled("…", Style::new().dim()));
    }
    for part in parts {
        if skip >= part.text.len() {
            skip -= part.text.len();
            continue;
        }
        // Highlight positions count chars, so drop and shift past the cut ones.
        let dropped = part.text[..skip].chars().count();
        let marks: Vec<usize> = part
            .marks
            .iter()
            .filter_map(|&m| m.checked_sub(dropped))
            .collect();
        spans.extend(highlighted(&part.text[skip..], &marks, part.style));
        skip = 0;
    }
    spans
}

/// The byte where a shortened `path` starts so that `…` and the rest fit in
/// `width` columns: at a `/` when the rest fits from one, so whole directories
/// go first. 0 when the path fits as it is.
fn cut(path: &str, width: usize) -> usize {
    if columns(path) <= width {
        return 0;
    }
    let room = width.saturating_sub(1);
    // The earliest start (longest tail) that fits, and the earliest at a `/`.
    let (mut fits, mut at_slash) = (path.len(), None);
    let mut used = 0;
    for (i, c) in path.char_indices().rev() {
        used += columns(&path[i..i + c.len_utf8()]);
        if used > room {
            break;
        }
        fits = i;
        if c == '/' {
            at_slash = Some(i);
        }
    }
    at_slash.unwrap_or(fits)
}

fn matched(style: Style) -> Style {
    style
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Splits `text` into spans, marking the chars at `positions` (sorted).
fn highlighted<'a>(text: &'a str, positions: &[usize], base: Style) -> Vec<Span<'a>> {
    if positions.is_empty() {
        return vec![Span::styled(text, base)];
    }
    let mut spans = Vec::new();
    let mut start = 0;
    let mut current = false;
    let mut next = positions.iter().peekable();
    for (n, (byte, _)) in text.char_indices().enumerate() {
        let is_match = next.next_if(|&&p| p == n).is_some();
        if is_match != current && byte > start {
            let style = if current { matched(base) } else { base };
            spans.push(Span::styled(&text[start..byte], style));
            start = byte;
        }
        current = is_match;
    }
    let style = if current { matched(base) } else { base };
    spans.push(Span::styled(&text[start..], style));
    spans
}

fn status_style(status: Status) -> Style {
    let color = match status {
        Status::Added => Color::Green,
        Status::Deleted => Color::Red,
        Status::Modified => Color::Yellow,
        Status::Renamed | Status::Copied => Color::Blue,
        _ => Color::Magenta,
    };
    Style::new().fg(color).bold()
}

fn draw_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let mut title = app.file().map_or("diff".to_owned(), |f| f.path.clone());
    if app.diff_column > 0 {
        // Says the view is shifted, and by how much.
        title.push_str(&format!(" · col {}", app.diff_column + 1));
    }
    let block = pane(&title, app.focus == Focus::Diff);
    let inner = block.inner(area);
    app.diff_height = inner.height as usize;
    app.diff_width = inner.width as usize;

    let pattern = app.code.as_ref().map(|c| c.pattern.as_str());
    let mut lines: Vec<Line> = match app.diff() {
        DiffView::Ready(lines) => lines
            .iter()
            .skip(app.diff_scroll)
            .take(app.diff_height)
            .map(|l| diff_line(l, pattern))
            .collect(),
        DiffView::Failed(err) => vec![Line::styled(err, Color::Red)],
        DiffView::Loading => vec![Line::styled("loading…", Style::new().dim())],
        DiffView::Empty => vec![],
    };
    if app.commit().is_some() && app.file().is_none() && lines.len() < app.diff_height {
        lines.push(Line::default());
        lines.push(Line::styled("no file changes", Style::new().dim()));
    }
    let column = app.diff_column.min(u16::MAX as usize) as u16;
    frame.render_widget(Paragraph::new(lines).block(block).scroll((0, column)), area);
}

/// A diff line with its changed words in reverse video, like `diff-highlight`,
/// and every occurrence of the code-search text marked on top.
fn diff_line<'a>(line: &'a DiffLine, pattern: Option<&str>) -> Line<'a> {
    let text = line.text.as_str();
    let base = line_style(line.kind);
    let found = pattern.map_or_else(Vec::new, |p| find_code(text, p));
    if line.emph.is_empty() && found.is_empty() {
        return Line::styled(text, base);
    }
    let mut cuts: Vec<usize> = line
        .emph
        .iter()
        .chain(&found)
        .flat_map(|r| [r.start, r.end])
        .chain([0, text.len()])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let within = |ranges: &[Range<usize>], at: usize| ranges.iter().any(|r| r.contains(&at));
    let spans: Vec<Span> = cuts
        .windows(2)
        .map(|w| {
            let style = if within(&found, w[0]) {
                base.patch(code_match())
            } else if within(&line.emph, w[0]) {
                base.add_modifier(Modifier::REVERSED)
            } else {
                base
            };
            Span::styled(&text[w[0]..w[1]], style)
        })
        .collect();
    Line::from(spans)
}

fn code_match() -> Style {
    Style::new()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn line_style(kind: LineKind) -> Style {
    match kind {
        LineKind::Meta => Style::new().fg(Color::Yellow),
        LineKind::Message => Style::new(),
        LineKind::Header => Style::new().dim().bold(),
        LineKind::Hunk => Style::new().fg(ACCENT),
        LineKind::Added => Style::new().fg(Color::Green),
        LineKind::Removed => Style::new().fg(Color::Red),
        LineKind::Context => Style::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Exit, Focus, Msg};
    use crate::git;
    use crate::testutil::Repo;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };

    fn render(app: &mut App) -> String {
        render_at(app, 120, 24)
    }

    fn render_at(app: &mut App, width: u16, height: u16) -> String {
        app.settle();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<String> = (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        rows.join("\n")
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn press(app: &mut App, code: KeyCode) {
        let key = KeyEvent::new(code, KeyModifiers::NONE);
        app.update(Msg::Input(Event::Key(key)));
    }

    /// Plays the diff worker's part synchronously.
    fn serve_diffs(app: &mut App, repo: &Repo) {
        while let Some(req) = app.take_diff_request() {
            let result = git::diff(repo.path(), &req.sha, req.file.as_ref())
                .map(|text| git::parse_diff(&text))
                .map_err(|e| e.to_string());
            app.update(Msg::Diff(req.key(), result));
        }
    }

    #[test]
    fn three_panes_and_navigation() {
        let repo = Repo::new();
        repo.write("tatakae.rs", "fn main() {}\n");
        repo.commit("first");
        repo.write("tatakae.rs", "fn main() { attack(); }\n");
        repo.write("wall.md", "maria\n");
        repo.commit("second");

        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        let screen = render(&mut app);
        assert!(screen.contains("2 commits…"));
        assert!(screen.contains("loading…"));

        app.update(Msg::LogDone(0, Ok(())));
        serve_diffs(&mut app, &repo);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("2 commits "));
        assert!(screen.contains("second"));
        assert!(screen.contains("first"));
        assert!(screen.contains("M tatakae.rs"));
        assert!(screen.contains("A wall.md"));
        assert!(screen.contains("+fn main() { attack(); }"));

        // Files pane: move to wall.md, diff follows.
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);
        serve_diffs(&mut app, &repo);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("+maria"));

        // Back to tatakae.rs: served from cache, no new request.
        press(&mut app, KeyCode::Up);
        assert!(app.take_diff_request().is_none());
        assert!(render(&mut app).contains("+fn main() { attack(); }"));

        // Back to commits, go to the older one: file selection resets.
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Down);
        serve_diffs(&mut app, &repo);
        assert!(render(&mut app).contains("+fn main() {}"));
        assert_eq!(app.file_idx, 0);

        // Typing goes to the query; Esc clears it, then quits.
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.query, "x");
        press(&mut app, KeyCode::Esc);
        assert!(app.query.is_empty() && app.exit.is_none());

        press(&mut app, KeyCode::Enter);
        let sha = app.commit().unwrap().sha.clone();
        assert!(matches!(app.exit, Some(Exit::Pick(ref s)) if *s == sha));
    }

    #[test]
    fn diff_starts_with_the_commit_message() {
        let repo = Repo::new();
        repo.write("a.rs", "let x = foo(1, a);\n");
        repo.commit("first");
        repo.write("a.rs", "let x = foo(2, a);\n");
        repo.commit("give it a two\n\nOne was never enough.");
        repo.commit("an empty one");
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        app.update(Msg::LogDone(0, Ok(())));

        // A commit without files still shows its message.
        serve_diffs(&mut app, &repo);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("Author: Levi <levi@scouts>"));
        assert!(screen.contains("    an empty one"));
        assert!(screen.contains("no file changes"));

        press(&mut app, KeyCode::Down);
        serve_diffs(&mut app, &repo);
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<String> = (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        let row = |text: &str| rows.iter().position(|r| r.contains(text)).unwrap();
        assert!(row("    give it a two") < row("    One was never enough."));
        assert!(row("One was never enough.") < row("+let x = foo(2, a);"));

        // In the diff lines, only the changed word is in reverse video.
        let reversed: String = (0..buf.area.height)
            .filter(|&y| rows[y as usize].contains("let x = foo("))
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .filter(|&p| buf[p].modifier.contains(Modifier::REVERSED))
            .map(|p| buf[p].symbol().to_owned())
            .collect();
        assert_eq!(reversed, "12");
    }

    fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
        app.update(Msg::Input(Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })));
    }

    #[test]
    fn mouse_clicks_and_scrolls() {
        let repo = titans();
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        app.update(Msg::LogDone(0, Ok(())));
        let screen = render(&mut app);
        let rows: Vec<&str> = screen.lines().collect();
        let row_of = |text: &str| rows.iter().position(|r| r.contains(text)).unwrap() as u16;
        let click = MouseEventKind::Down(MouseButton::Left);

        // Click the oldest commit: selected, and the commits pane has focus.
        mouse(&mut app, click, 2, row_of("initial import"));
        assert_eq!(app.commit().unwrap().subject, "initial import");
        assert_eq!(app.focus, Focus::Commits);

        // Click its second file: the files pane gets focus.
        render(&mut app);
        let files_x = app.panes[1].x + 2;
        // First row inside the border.
        let top = app.panes[1].y + 1;
        mouse(&mut app, click, files_x, top + 1);
        assert_eq!(app.focus, Focus::Files);
        assert_eq!(app.file().unwrap().path, "src/parse.rs");
        mouse(&mut app, click, files_x, top);
        assert_eq!(app.file().unwrap().path, "README.md");

        // The border and empty rows below the list select nothing.
        mouse(&mut app, click, files_x, top - 1);
        mouse(&mut app, click, files_x, top + 2);
        assert_eq!(app.file().unwrap().path, "README.md");

        // The wheel moves the list under the pointer, not the focused one.
        mouse(&mut app, MouseEventKind::ScrollUp, 2, 10);
        assert_eq!(app.commit().unwrap().subject, "tweak things");
        assert_eq!(app.focus, Focus::Files);

        // Over the diff, it scrolls a few lines at a time.
        serve_diffs(&mut app, &repo);
        render(&mut app);
        let diff_x = app.panes[2].x + 2;
        mouse(&mut app, MouseEventKind::ScrollDown, diff_x, 10);
        assert_eq!(app.diff_scroll, 3);
        mouse(&mut app, MouseEventKind::ScrollUp, diff_x, 10);
        assert_eq!(app.diff_scroll, 0);

        // Clicking the diff focuses it; the search bar ignores clicks.
        mouse(&mut app, click, diff_x, 10);
        assert_eq!(app.focus, Focus::Diff);
        mouse(&mut app, click, 2, 1);
        assert_eq!(app.focus, Focus::Diff);
    }

    #[test]
    fn tiny_terminals_do_not_crash() {
        let repo = titans();
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        press(&mut app, KeyCode::Down);
        for (w, h) in [(0, 0), (10, 3), (100, 5)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
        }
    }

    #[test]
    fn narrow_terminals_stack_the_lists_above_the_diff() {
        let repo = titans();
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        render(&mut app);
        let [commits, files, diff] = app.panes;
        assert!(
            commits.y == diff.y && diff.x == files.right(),
            "side by side when wide"
        );

        serve_diffs(&mut app, &repo);
        let screen = render_at(&mut app, 90, 30);
        println!("{screen}");
        let [commits, files, diff] = app.panes;
        assert_eq!((commits.y, files.y), (3, 3));
        assert_eq!(files.x, commits.right());
        assert_eq!((diff.x, diff.y, diff.width), (0, commits.bottom(), 90));
        assert!(screen.contains("Author: Levi"), "the diff is drawn there");
        assert_eq!(app.diff_width, 88);
    }

    #[test]
    fn long_lines_scroll_sideways() {
        let repo = Repo::new();
        let long = format!("let start = 1; {}// the TAIL end", "x".repeat(120));
        repo.write("wide.rs", &format!("{long}\n"));
        repo.write("narrow.rs", "short\n");
        repo.commit("wide");
        let mut app = loaded(&repo);
        render(&mut app);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);
        serve_diffs(&mut app, &repo);
        assert_eq!(app.file().unwrap().path, "wide.rs");
        let screen = render(&mut app);
        assert!(screen.contains("+let start") && !screen.contains("TAIL"));

        let shift = |app: &mut App, code| {
            let key = KeyEvent::new(code, KeyModifiers::SHIFT);
            app.update(Msg::Input(Event::Key(key)));
        };
        shift(&mut app, KeyCode::Right);
        let screen = render(&mut app);
        assert!(
            screen.contains("wide.rs · col 9"),
            "the title says it's shifted"
        );
        assert!(!screen.contains("+let start"));

        // It stops once the end of the widest line is in view.
        for _ in 0..50 {
            shift(&mut app, KeyCode::Right);
        }
        let screen = render(&mut app);
        assert!(
            screen.contains("the TAIL end│"),
            "flush with the right edge"
        );
        assert_eq!(app.diff_column, long.len() + 1 - app.diff_width);
        assert_eq!(app.focus, Focus::Files, "Shift-arrows don't switch panes");

        // The wheel scrolls sideways over the diff, and so does Shift+wheel.
        let x = app.panes[2].x + 2;
        let wheel = |app: &mut App, kind, modifiers| {
            app.update(Msg::Input(Event::Mouse(MouseEvent {
                kind,
                column: x,
                row: 10,
                modifiers,
            })));
        };
        let end = app.diff_column;
        wheel(&mut app, MouseEventKind::ScrollLeft, KeyModifiers::NONE);
        assert_eq!(app.diff_column, end - 8);
        wheel(&mut app, MouseEventKind::ScrollDown, KeyModifiers::SHIFT);
        assert_eq!(app.diff_column, end);
        assert_eq!(app.diff_scroll, 0, "Shift+wheel doesn't scroll down");

        // Another file starts at the left edge again.
        press(&mut app, KeyCode::Up);
        assert_eq!(app.diff_column, 0);
        shift(&mut app, KeyCode::Left);
        assert_eq!(app.diff_column, 0);
    }

    #[test]
    fn shift_arrows_jump_between_hunks_and_matches() {
        let repo = Repo::new();
        let lines: Vec<String> = (1..=60).map(|n| format!("line {n}")).collect();
        repo.write("a.rs", &(lines.join("\n") + "\n"));
        repo.commit("plain");
        let mut changed = lines.clone();
        for n in [9, 29, 49] {
            changed[n] = format!("attack {n}");
        }
        changed[30] = "attack again".into();
        repo.write("a.rs", &(changed.join("\n") + "\n"));
        repo.commit("three hunks");
        let mut app = loaded(&repo);
        serve_diffs(&mut app, &repo);
        render(&mut app);
        let DiffView::Ready(diff) = app.diff() else {
            panic!("no diff");
        };
        let at = |kind: LineKind| -> Vec<usize> {
            (0..diff.len()).filter(|&i| diff[i].kind == kind).collect()
        };
        let hunks = at(LineKind::Hunk);
        let added = at(LineKind::Added);
        assert_eq!((hunks.len(), added.len()), (3, 4));

        let shift = |app: &mut App, code| {
            let key = KeyEvent::new(code, KeyModifiers::SHIFT);
            app.update(Msg::Input(Event::Key(key)));
        };
        // From the commit pane: hunk by hunk, staying put past the last one.
        for &hunk in &hunks {
            shift(&mut app, KeyCode::Down);
            assert_eq!(app.diff_scroll, hunk);
        }
        shift(&mut app, KeyCode::Down);
        assert_eq!(app.diff_scroll, hunks[2]);
        assert_eq!(app.focus, Focus::Commits, "focus stays put");
        assert!(render(&mut app).contains("@@ -47,7"), "the hunk is on top");
        shift(&mut app, KeyCode::Up);
        assert_eq!(app.diff_scroll, hunks[1]);
        shift(&mut app, KeyCode::Up);
        shift(&mut app, KeyCode::Up);
        shift(&mut app, KeyCode::Up);
        assert_eq!(app.diff_scroll, hunks[0], "and before the first");

        // With `/c` on, the same keys step through the matching lines, each with
        // a little context above it.
        type_str(&mut app, "/c attack");
        serve_code(&mut app, &repo);
        serve_diffs(&mut app, &repo);
        render(&mut app);
        assert_eq!(app.diff_scroll, added[0] - 3, "the first match");
        for &line in &added[1..] {
            shift(&mut app, KeyCode::Down);
            assert_eq!(app.diff_scroll, line - 3);
        }
        shift(&mut app, KeyCode::Up);
        assert_eq!(app.diff_scroll, added[2] - 3);
    }

    #[test]
    fn code_search_scrolls_to_a_match_past_the_edge() {
        let repo = Repo::new();
        let long = format!("let start = 1; {}// the TAIL end", "x".repeat(120));
        repo.write("wide.rs", &format!("{long}\n"));
        repo.commit("wide");
        let mut app = loaded(&repo);
        render(&mut app);
        type_str(&mut app, "/c TAIL");
        serve_code(&mut app, &repo);
        serve_diffs(&mut app, &repo);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(app.diff_column > 0);
        assert!(screen.contains("the TAIL end"), "the match is in view");
    }

    #[test]
    fn the_cursor_follows_the_query() {
        let mut app = App::new();
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        type_str(&mut app, "tatakae");
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        // Past the border, "> " and the 7 letters.
        assert_eq!(terminal.get_cursor_position().unwrap(), (10, 1).into());

        // A query wider than the bar keeps it inside the border.
        type_str(&mut app, &"x".repeat(50));
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(terminal.get_cursor_position().unwrap(), (38, 1).into());
    }

    #[test]
    fn no_color_keeps_the_marks_without_colors() {
        let repo = battlefield();
        let mut app = loaded(&repo);
        app.no_color = true;
        type_str(&mut app, "/c attack");
        serve_code(&mut app, &repo);
        serve_diffs(&mut app, &repo);
        render(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        assert!(
            buf.content
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
        // The selected commit and the code match are in reverse video.
        let reversed = |text: &str| {
            let y = (0..buf.area.height)
                .find(|&y| {
                    let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                    row.contains(text)
                })
                .unwrap();
            (0..buf.area.width).any(|x| buf[(x, y)].modifier.contains(Modifier::REVERSED))
        };
        assert!(reversed("expedition"));
        assert!(reversed("+attack titan"));
    }

    #[test]
    fn key_hints_fit_the_width() {
        let mut app = App::new();
        let screen = render_at(&mut app, 100, 24);
        let bottom = screen.lines().last().unwrap();
        assert!(bottom.starts_with(" ↑↓ move  enter pick  /c search code  /r reflog  tab pane"));
        assert!(bottom.contains("pgup/pgdn scroll"));
        assert!(
            !bottom.contains("sideways"),
            "the last one doesn't fit 100 columns"
        );

        // Narrower: the least useful go, and no hint is cut in half.
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        let bottom: String = (0..40).map(|x| buf[(x, 9)].symbol()).collect();
        assert_eq!(bottom.trim_end(), " ↑↓ move  enter pick  /c search code");
    }

    #[test]
    fn status_names_the_revision() {
        let repo = titans();
        let mut app = App::new();
        app.rev = Some("HEAD~2..".into());
        app.update(Msg::Commits(0, repo.log()[..2].to_vec()));
        app.update(Msg::LogDone(0, Ok(())));
        assert!(render(&mut app).contains(" 2 commits in HEAD~2.. "));

        // The reflog isn't narrowed by it, so it isn't named there.
        type_str(&mut app, "/r");
        assert!(!render(&mut app).contains("HEAD~2.."));
    }

    #[test]
    fn log_error_shows_in_status() {
        let mut app = App::new();
        app.update(Msg::LogDone(0, Err("git log: boom".into())));
        assert!(render(&mut app).contains("git log: boom"));
    }

    fn titans() -> Repo {
        let repo = Repo::new();
        repo.write("src/parse.rs", "1\n");
        repo.write("README.md", "levi\n");
        repo.commit("initial import");
        repo.write("src/parse.rs", "2\n");
        repo.write("src/ui.rs", "ui\n");
        repo.commit("tweak things");
        repo.write("README.md", "levi ackerman\n");
        repo.commit("docs: humanity's strongest");
        repo
    }

    #[test]
    fn search_by_file_narrows_both_columns() {
        let repo = titans();
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        app.update(Msg::LogDone(0, Ok(())));

        type_str(&mut app, "parse.rs");
        serve_diffs(&mut app, &repo);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("2/3 commits"));
        assert!(screen.contains("tweak things"));
        assert!(screen.contains("initial import"));
        assert!(!screen.contains("humanity"));
        // Only the matching file is listed, and its diff is shown.
        assert!(screen.contains("M src/parse.rs"));
        assert!(!screen.contains("src/ui.rs"));
        assert!(!screen.contains("README"));
        assert!(screen.contains("+2"));

        // Navigation acts on the filtered view.
        press(&mut app, KeyCode::Down);
        assert_eq!(app.commit().unwrap().subject, "initial import");
        assert_eq!(app.files().len(), 1);

        // A subject match lists every file of the commit.
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "tweak");
        let screen = render(&mut app);
        assert!(screen.contains("1/3 commits"));
        assert!(screen.contains("src/parse.rs") && screen.contains("src/ui.rs"));

        type_str(&mut app, "zzz");
        assert!(render(&mut app).contains("no matches"));
        assert!(app.take_diff_request().is_none());
    }

    #[test]
    fn keys_after_typing_see_the_new_view() {
        let repo = titans();
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        // Typed and moved in one burst, before any frame settled the query.
        type_str(&mut app, "parse");
        press(&mut app, KeyCode::Down);
        assert_eq!(app.commit().unwrap().subject, "initial import");
    }

    #[test]
    fn ctrl_w_deletes_the_last_word() {
        let mut app = App::new();
        let ctrl_w = |app: &mut App| {
            let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
            app.update(Msg::Input(Event::Key(key)));
        };
        type_str(&mut app, "parse.rs /c let x  ");
        ctrl_w(&mut app);
        assert_eq!(app.query, "parse.rs /c let ");
        ctrl_w(&mut app);
        ctrl_w(&mut app);
        assert_eq!(app.query, "parse.rs ");
        ctrl_w(&mut app);
        ctrl_w(&mut app);
        assert_eq!(app.query, "", "and nothing breaks on an empty query");
    }

    #[test]
    fn streaming_keeps_selection_under_a_query() {
        let repo = titans();
        let mut log = repo.log();
        let older = log.split_off(1);
        let mut app = App::new();
        app.update(Msg::Commits(0, log));
        type_str(&mut app, "s");
        app.settle();
        let selected = app.commit().unwrap().sha.clone();
        app.update(Msg::Commits(0, older));
        assert_eq!(app.commit().unwrap().sha, selected);
        assert_eq!(app.view.len(), 3);
    }

    #[test]
    fn highlighted_splits_on_char_boundaries() {
        let spans = highlighted("ñandú", &[1, 2, 4], Style::new());
        let parts: Vec<_> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(parts, ["ñ", "an", "d", "ú"]);
        assert!(spans[1].style.add_modifier.contains(Modifier::UNDERLINED));
        assert!(!spans[2].style.add_modifier.contains(Modifier::UNDERLINED));
    }

    /// What `fitted` shows, as text.
    fn shown(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn fit(path: &str, width: usize) -> String {
        shown(&fitted(vec![Part::new(path, vec![], Style::new())], width))
    }

    fn rename(old: &str, new: &str) -> String {
        shown(&fitted(renamed((old, vec![]), (new, vec![]), 100), 100))
    }

    #[test]
    fn long_paths_keep_their_end() {
        let deep = "app/frontend/src/components/frontend/Button.tsx";
        assert_eq!(fit(deep, 47), deep, "what fits is left alone");
        // Whole directories go first, so the rest starts at a `/`...
        assert_eq!(
            fit(deep, 46),
            "…/frontend/src/components/frontend/Button.tsx"
        );
        assert_eq!(fit(deep, 32), "…/components/frontend/Button.tsx");
        // ...then the start of the name, once even that doesn't fit.
        assert_eq!(fit(deep, 8), "…ton.tsx");
        assert_eq!(fit(deep, 0), "…");
        // Wide chars count by the columns they take.
        assert_eq!(fit("dir/日本語.rs", 8), "…本語.rs");

        // Highlights move with the text that's left.
        let part = Part::new("src/a/Button.tsx", vec![0, 6, 7], Style::new());
        let spans = fitted(vec![part], 12);
        let parts: Vec<_> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(parts, ["…", "/", "Bu", "tton.tsx"]);
        assert!(spans[2].style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn renames_write_shared_directories_once() {
        assert_eq!(
            rename("app/fe/Button.tsx", "app/fe/Knob.tsx"),
            "app/fe/{Button.tsx → Knob.tsx}"
        );
        assert_eq!(rename("src/a/mod.rs", "src/b/mod.rs"), "src/{a → b}/mod.rs");
        assert_eq!(rename("a/x.rs", "a/b/x.rs"), "a/{x.rs → b/x.rs}");
        assert_eq!(rename("x.rs", "y.rs"), "x.rs → y.rs");
        assert_eq!(rename("é/x.rs", "é/ý.rs"), "é/{x.rs → ý.rs}");

        // Narrower, the shared directories go first; once the change itself
        // doesn't fit, only the new path is left, its end showing.
        let (old, new) = ("app/fe/Button.tsx", "app/fe/PrimaryButton.tsx");
        let at = |width| shown(&fitted(renamed((old, vec![]), (new, vec![]), width), width));
        assert_eq!(at(34), "…/{Button.tsx → PrimaryButton.tsx}");
        assert_eq!(at(33), "…{Button.tsx → PrimaryButton.tsx}");
        assert_eq!(at(32), "app/fe/PrimaryButton.tsx");
        assert_eq!(at(20), "…/PrimaryButton.tsx");
        assert_eq!(at(12), "…yButton.tsx");

        // Each side keeps its own highlights: here "src/a/mod.rs" matched on
        // its "a" (char 4) and "src/b/mod.rs" on its "b" and "m" (4 and 6).
        let spans = fitted(
            renamed(("src/a/mod.rs", vec![4]), ("src/b/mod.rs", vec![4, 6]), 100),
            100,
        );
        let lit: Vec<_> = spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(lit, ["a", "b", "m"]);
    }

    #[test]
    fn files_pane_shows_the_end_of_long_paths() {
        let repo = Repo::new();
        let deep = "app/frontend/src/components/frontend/Button.tsx";
        repo.write(deep, "export {}\n");
        repo.commit("button");
        repo.git(&["mv", deep, "app/frontend/src/components/frontend/Knob.tsx"]);
        repo.commit("rename");
        let mut app = loaded(&repo);
        serve_diffs(&mut app, &repo);
        let screen = render(&mut app);
        let files = app.panes[1];
        let row = |screen: &str| -> String {
            let line = screen.lines().nth(files.y as usize + 1).unwrap();
            line.chars()
                .skip(files.x as usize + 1)
                .take(files.width as usize - 2)
                .collect()
        };
        // Too narrow for the rename itself, so the new path shows, its end first.
        assert_eq!(row(&screen).trim_end(), "R …/frontend/Knob.tsx");
        // The diff's title has the whole path.
        assert!(screen.contains("app/frontend/src/components/frontend/Knob.tsx"));

        press(&mut app, KeyCode::Down);
        serve_diffs(&mut app, &repo);
        let added = row(&render(&mut app));
        assert_eq!(added.trim_end(), "A …/Button.tsx");
    }

    #[test]
    fn long_lists_scroll_with_the_selection() {
        let repo = Repo::new();
        for n in 0..30 {
            repo.write("f", &format!("{n}\n"));
            repo.commit(&format!("commit number {n}"));
        }
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        for _ in 0..20 {
            press(&mut app, KeyCode::Down);
        }
        let screen = render(&mut app);
        assert!(screen.contains("commit number 9"), "selected row visible");
        assert!(!screen.contains("commit number 29"), "top scrolled away");
        // 24 rows: 3 for search, 2 borders, 1 for key hints, 18 list rows.
        assert_eq!(app.commit_offset, 20 + 1 - 18);
    }

    #[test]
    fn slash_r_switches_to_the_reflog() {
        let repo = titans();
        repo.git(&["checkout", "-q", "-b", "expedition"]);
        repo.git(&["checkout", "-q", "main"]);
        let mut app = App::new();
        let first = app.take_log_request().unwrap();
        assert_eq!((first.source, first.generation), (Source::Log, 0));
        app.update(Msg::Commits(0, repo.log()));
        app.update(Msg::LogDone(0, Ok(())));

        type_str(&mut app, "/r");
        let req = app.take_log_request().unwrap();
        assert_eq!((req.source, req.generation), (Source::Reflog, 1));
        assert!(app.commits.is_empty() && app.loading);
        assert!(app.take_log_request().is_none(), "asked once");

        // A late batch from the old log is ignored.
        app.update(Msg::Commits(0, repo.log()));
        assert!(app.commits.is_empty());

        let entries = git::log(repo.path(), Source::Reflog)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        app.update(Msg::Commits(1, entries));
        app.update(Msg::LogDone(1, Ok(())));
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("levi · reflog"));
        assert!(
            screen.contains("┌ reflog ─"),
            "pane title follows the source"
        );
        assert!(screen.contains(" checkout: movin"));
        assert!(screen.contains("reflog entries"));
        let action = &app.commit().unwrap().reflog.as_ref().unwrap().action;
        assert_eq!(action, "checkout: moving from expedition to main");

        // The rest of the query filters entries, including by action.
        type_str(&mut app, " expedition");
        let screen = render(&mut app);
        assert!(screen.contains("2/"), "{screen}");
        assert!(app.take_log_request().is_none(), "filtering doesn't reload");

        // Clearing the query goes back to the log.
        press(&mut app, KeyCode::Esc);
        let req = app.take_log_request().unwrap();
        assert_eq!((req.source, req.generation), (Source::Log, 2));
    }

    /// Plays the code searcher's part synchronously, as if typing paused long enough.
    fn serve_code(app: &mut App, repo: &Repo) {
        let later = std::time::Instant::now() + crate::app::CODE_DEBOUNCE * 2;
        let req = app.take_code_request(later).expect("a code search is due");
        let hits = crate::worker::search_now(repo.path(), &req);
        app.update(Msg::CodeHits(req.generation, hits));
        app.update(Msg::CodeDone(req.generation, Ok(())));
    }

    fn shas(req: &crate::app::CodeRequest) -> Vec<String> {
        req.targets.iter().map(|t| t.sha.clone()).collect()
    }

    fn battlefield() -> Repo {
        let repo = Repo::new();
        let lines: Vec<String> = (1..=40).map(|n| format!("line {n}")).collect();
        repo.write("a.rs", &(lines.join("\n") + "\n"));
        repo.write("README.md", "levi\n");
        repo.commit("walls");
        let mut changed = lines.clone();
        changed[29] = "attack titan".into();
        repo.write("a.rs", &(changed.join("\n") + "\n"));
        repo.write("wall.md", "attack\n");
        repo.commit("expedition");
        repo.write("README.md", "levi ackerman\n");
        repo.commit("docs");
        repo
    }

    fn loaded(repo: &Repo) -> App {
        let mut app = App::new();
        app.update(Msg::Commits(0, repo.log()));
        app.update(Msg::LogDone(0, Ok(())));
        app
    }

    #[test]
    fn slash_c_searches_the_diffs() {
        let repo = battlefield();
        let mut app = loaded(&repo);

        type_str(&mut app, "/c attack");
        // Typing just paused: nothing yet.
        assert!(app.take_code_request(std::time::Instant::now()).is_none());
        assert!(app.deadline().is_some(), "the loop must wake up for it");
        assert!(render(&mut app).contains("searching…"));

        serve_code(&mut app, &repo);
        serve_diffs(&mut app, &repo);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("levi · code: attack"));
        assert!(screen.contains("1/3 commits"));
        assert!(screen.contains("expedition"));
        assert!(!screen.contains("docs") && !screen.contains("walls"));
        assert!(screen.contains("M a.rs") && screen.contains("A wall.md"));

        // The diff jumped down to the match, which is highlighted.
        assert!(app.diff_scroll > 0, "scrolled to the match");
        assert!(screen.contains("+attack titan"));
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        let marked = (0..100)
            .flat_map(|x| (0..20).map(move |y| (x, y)))
            .filter(|&(x, y)| buf[(x, y)].bg == Color::Yellow)
            .count();
        assert_eq!(marked, "attack".len());
    }

    #[test]
    fn slash_c_combines_with_fuzzy() {
        let repo = battlefield();
        let mut app = loaded(&repo);
        type_str(&mut app, "wall /c attack");
        serve_code(&mut app, &repo);
        assert_eq!(app.view.len(), 1);
        let files: Vec<_> = app.files().iter().map(|f| f.path.clone()).collect();
        assert_eq!(files, ["wall.md"], "both filters apply to files");

        // README changed in `walls` and `docs`, but neither diff says `attack`.
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "README /c attack");
        serve_code(&mut app, &repo);
        assert!(app.view.is_empty());
    }

    #[test]
    fn slash_c_searches_only_what_the_fuzzy_part_matches() {
        let repo = battlefield();
        let mut app = loaded(&repo);
        let later = || std::time::Instant::now() + crate::app::CODE_DEBOUNCE * 2;
        let subjects = |app: &App, shas: &[String]| -> Vec<String> {
            let mut s: Vec<_> = shas
                .iter()
                .map(|sha| {
                    app.commits
                        .iter()
                        .find(|c| &c.sha == sha)
                        .unwrap()
                        .subject
                        .clone()
                })
                .collect();
            s.sort();
            s
        };

        type_str(&mut app, "README /c attack");
        let req = app.take_code_request(later()).unwrap();
        assert_eq!(
            subjects(&app, &shas(&req)),
            ["docs", "walls"],
            "only README commits"
        );
        app.take_code_cancel();

        // Narrowing the fuzzy part keeps the results: no new search.
        let hits = crate::worker::search_now(repo.path(), &req);
        app.update(Msg::CodeHits(req.generation, hits));
        app.update(Msg::CodeDone(req.generation, Ok(())));
        app.query = "README.md /c attack".into();
        assert!(app.take_code_request(later()).is_none());
        assert!(!app.take_code_cancel());

        // Widening it may need commits that weren't searched: search again.
        app.query = "e /c attack".into();
        let req = app.take_code_request(later()).unwrap();
        assert_eq!(subjects(&app, &shas(&req)), ["docs", "expedition", "walls"]);

        // With no fuzzy part, everything is searched, and any fuzzy text added
        // later reuses that.
        app.query = "/c attack".into();
        let req = app.take_code_request(later()).unwrap();
        assert_eq!(req.targets.len(), 3);
        app.query = "wall /c attack".into();
        assert!(app.take_code_request(later()).is_none());
    }

    #[test]
    fn slash_c_waits_for_fuzzy_typing_to_pause() {
        let repo = battlefield();
        let mut app = loaded(&repo);
        type_str(&mut app, "/c attack");
        app.settle();
        let first_due = app.deadline().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        // Typing the fuzzy part before the search started pushes it back.
        app.query = "w /c attack".into();
        app.settle();
        assert!(app.deadline().unwrap() > first_due);
        let req = app
            .take_code_request(std::time::Instant::now() + crate::app::CODE_DEBOUNCE * 2)
            .unwrap();
        assert_eq!(req.targets.len(), 2, "scoped to commits matching `w`");
    }

    #[test]
    fn slash_c_restarts_and_ignores_stale_results() {
        let repo = battlefield();
        let mut app = loaded(&repo);
        type_str(&mut app, "/c attack");
        let later = std::time::Instant::now() + crate::app::CODE_DEBOUNCE * 2;
        let old = app.take_code_request(later).unwrap();
        assert!(app.take_code_cancel(), "a new search cancels whatever ran");

        type_str(&mut app, "er");
        app.settle();
        assert!(
            app.take_code_cancel(),
            "editing the pattern cancels the old search"
        );
        // The old search's results arrive late and are ignored.
        let hits = crate::worker::search_now(repo.path(), &old);
        app.update(Msg::CodeHits(old.generation, hits));
        assert!(app.view.is_empty());

        serve_code(&mut app, &repo);
        assert!(app.view.is_empty(), "nothing says `attacker`");
        assert!(render(&mut app).contains("no matches"));

        // Removing `/c` brings everything back.
        for _ in 0.."/c attacker".len() {
            press(&mut app, KeyCode::Backspace);
        }
        app.settle();
        assert!(app.code.is_none());
        assert_eq!(app.view.len(), 3);
        assert!(app.take_code_cancel());
    }

    #[test]
    fn slash_c_waits_for_the_log() {
        let repo = battlefield();
        let mut app = App::new();
        type_str(&mut app, "/c attack");
        let later = std::time::Instant::now() + crate::app::CODE_DEBOUNCE * 2;
        app.update(Msg::Commits(0, repo.log()));
        assert!(
            app.take_code_request(later).is_none(),
            "the list isn't complete yet"
        );
        app.update(Msg::LogDone(0, Ok(())));
        assert_eq!(app.take_code_request(later).unwrap().targets.len(), 3);
    }

    #[test]
    fn slash_r_slash_c_searches_each_reflog_commit_once() {
        let repo = battlefield();
        repo.git(&["checkout", "-q", "-b", "scouts"]);
        repo.git(&["checkout", "-q", "main"]);
        let mut app = App::new();
        type_str(&mut app, "/r /c attack");
        let req = app.take_log_request().unwrap();
        assert_eq!(req.source, Source::Reflog);
        let entries: Vec<_> = git::log(repo.path(), Source::Reflog)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let entry_count = entries.len();
        app.update(Msg::Commits(req.generation, entries));
        app.update(Msg::LogDone(req.generation, Ok(())));
        let later = std::time::Instant::now() + crate::app::CODE_DEBOUNCE * 2;
        let code = app.take_code_request(later).unwrap();
        assert_eq!(
            code.targets.len(),
            3,
            "{entry_count} entries, 3 distinct commits"
        );
    }

    #[test]
    fn slash_c_diffs_only_the_files_the_fuzzy_part_matched() {
        let repo = battlefield();
        let mut app = loaded(&repo);
        type_str(&mut app, "wall /c attack");
        let req = app
            .take_code_request(std::time::Instant::now() + crate::app::CODE_DEBOUNCE * 2)
            .unwrap();
        let by_subject = |subject: &str| {
            let sha = &app
                .commits
                .iter()
                .find(|c| c.subject == subject)
                .unwrap()
                .sha;
            req.targets
                .iter()
                .find(|t| &t.sha == sha)
                .unwrap()
                .paths
                .clone()
        };
        // `walls` matched by its subject: every file counts.
        assert_eq!(by_subject("walls"), None);
        // `expedition` matched through wall.md only: a.rs needn't be diffed.
        assert_eq!(by_subject("expedition"), Some(vec!["wall.md".into()]));
    }

    #[test]
    fn slash_c_scoped_to_a_renamed_file_keeps_the_rename() {
        let repo = Repo::new();
        repo.write("old_name.rs", "let needle = 1;\nother\nlines\nhere\n");
        repo.commit("add");
        repo.git(&["mv", "old_name.rs", "new_name.rs"]);
        repo.commit("rename only");
        let mut app = loaded(&repo);
        // A pure rename adds or removes no line, so it must not match, even when
        // only the new name is in the query.
        type_str(&mut app, "new_name /c needle");
        serve_code(&mut app, &repo);
        assert!(
            app.view
                .iter()
                .all(|h| app.commits[h.commit].subject != "rename only")
        );
    }
}
