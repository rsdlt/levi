//! Rendering: search bar on top, commits | files | diff below.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::app::{App, DiffView, Focus};
use crate::git::{DiffLine, LineKind, Status};
use crate::query::{Source, find_code};

const ACCENT: Color = Color::Cyan;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [search, body] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(frame.area());
    let [commits, files, diff] = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Percentage(20),
        Constraint::Percentage(55),
    ])
    .areas(body);

    draw_search(frame, app, search);
    draw_commits(frame, app, commits);
    draw_files(frame, app, files);
    draw_diff(frame, app, diff);
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
        Span::styled("█", Style::new().fg(ACCENT)),
    ]);
    let count = if app.filter.is_empty() && app.code.is_none() {
        app.commits.len().to_string()
    } else {
        format!("{}/{}", app.view.len(), app.commits.len())
    };
    let noun = match app.source {
        Source::Log => "commits",
        Source::Reflog => "reflog entries",
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
    if selected < *offset {
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
    let height = block.inner(area).height as usize;
    follow(&mut app.file_offset, app.file_idx, height);

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
            if let Some(old) = &f.old_path {
                spans.extend(highlighted(
                    old,
                    &filter.highlights(old, true),
                    Style::new().dim(),
                ));
                spans.push(Span::styled(" → ", Style::new().dim()));
            }
            spans.extend(highlighted(
                &f.path,
                &filter.highlights(&f.path, true),
                Style::new(),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let row = app.file_idx - app.file_offset;
    draw_window(frame, area, block, items, row, focused);
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
    let title = app.file().map_or("diff".to_owned(), |f| f.path.clone());
    let block = pane(&title, app.focus == Focus::Diff);
    app.diff_height = block.inner(area).height as usize;

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
    frame.render_widget(Paragraph::new(lines).block(block), area);
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
    use crate::app::{Exit, Msg};
    use crate::git;
    use crate::testutil::Repo;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn render(app: &mut App) -> String {
        app.settle();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
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
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
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
        // 24 rows: 3 for search, 2 borders, 19 list rows.
        assert_eq!(app.commit_offset, 20 + 1 - 19);
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
