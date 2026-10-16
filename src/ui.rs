//! Rendering: search bar on top, commits | files | diff below.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::app::{App, DiffView, Focus};
use crate::git::{LineKind, Status};

const ACCENT: Color = Color::Cyan;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [search, body] = Layout::vertical([Constraint::Length(3), Constraint::Min(0)])
        .areas(frame.area());
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
        block.border_style(Style::new().fg(ACCENT)).title_style(Modifier::BOLD)
    } else {
        block.border_style(Style::new().dark_gray())
    }
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
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
    let count = if app.query.trim().is_empty() {
        app.commits.len().to_string()
    } else {
        format!("{}/{}", app.view.len(), app.commits.len())
    };
    let status = match &app.log_error {
        Some(err) => Line::styled(format!(" {err} "), Color::Red),
        None if app.loading => Line::from(format!(" {count} commits… ")),
        None => Line::from(format!(" {count} commits ")),
    };
    let block = Block::bordered()
        .title(" levi ".bold())
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
fn draw_window(frame: &mut Frame, area: Rect, block: Block, items: Vec<ListItem>, row: usize, focused: bool) {
    let list = List::new(items).block(block).highlight_style(highlight(focused));
    let mut state = ListState::default().with_selected(Some(row));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_commits(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Commits;
    let block = pane("commits", focused);
    let height = block.inner(area).height as usize;
    follow(&mut app.commit_offset, app.commit_idx, height);

    let App { view, commits, filter, .. } = app;
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
            spans.extend(highlighted(&c.subject, &filter.highlights(&c.subject, false), Style::new()));
            ListItem::new(Line::from(spans))
        })
        .collect();
    if items.is_empty() && !app.query.trim().is_empty() && !app.loading {
        items.push(ListItem::new(Line::styled("no matches", Style::new().dim())));
    }
    let row = app.commit_idx - app.commit_offset;
    draw_window(frame, area, block, items, row, focused);
}

fn draw_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Files;
    let block = pane("files", focused);
    let height = block.inner(area).height as usize;
    follow(&mut app.file_offset, app.file_idx, height);

    let App { view, commits, filter, .. } = app;
    let Some(hit) = view.get(app.commit_idx) else {
        frame.render_widget(block, area);
        return;
    };
    let files = &commits[hit.commit].files;
    let shown: Vec<usize> = hit.files.clone().unwrap_or_else(|| (0..files.len()).collect());
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
                spans.extend(highlighted(old, &filter.highlights(old, true), Style::new().dim()));
                spans.push(Span::styled(" → ", Style::new().dim()));
            }
            spans.extend(highlighted(&f.path, &filter.highlights(&f.path, true), Style::new()));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let row = app.file_idx - app.file_offset;
    draw_window(frame, area, block, items, row, focused);
}

fn matched(style: Style) -> Style {
    style.fg(Color::Magenta).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
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

    let lines: Vec<Line> = match app.diff() {
        DiffView::Ready(lines) => lines
            .iter()
            .skip(app.diff_scroll)
            .take(app.diff_height)
            .map(|l| Line::styled(l.text.as_str(), line_style(l.kind)))
            .collect(),
        DiffView::Failed(err) => vec![Line::styled(err, Color::Red)],
        DiffView::Loading => vec![Line::styled("loading…", Style::new().dim())],
        DiffView::Empty if app.commit().is_some() => {
            vec![Line::styled("no file changes", Style::new().dim())]
        }
        DiffView::Empty => vec![],
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn line_style(kind: LineKind) -> Style {
    match kind {
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
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
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
            let result = git::diff(repo.path(), &req.sha, &req.file)
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
        app.update(Msg::Commits(repo.log()));
        let screen = render(&mut app);
        assert!(screen.contains("2 commits…"));
        assert!(screen.contains("loading…"));

        app.update(Msg::LogDone(Ok(())));
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
    fn log_error_shows_in_status() {
        let mut app = App::new();
        app.update(Msg::LogDone(Err("git log: boom".into())));
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
        app.update(Msg::Commits(repo.log()));
        app.update(Msg::LogDone(Ok(())));

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
        app.update(Msg::Commits(repo.log()));
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
        app.update(Msg::Commits(log));
        type_str(&mut app, "s");
        app.settle();
        let selected = app.commit().unwrap().sha.clone();
        app.update(Msg::Commits(older));
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
        app.update(Msg::Commits(repo.log()));
        for _ in 0..20 {
            press(&mut app, KeyCode::Down);
        }
        let screen = render(&mut app);
        assert!(screen.contains("commit number 9"), "selected row visible");
        assert!(!screen.contains("commit number 29"), "top scrolled away");
        // 14 rows: 3 for search, 2 borders, 9 list rows.
        assert_eq!(app.commit_offset, 20 + 1 - 9);
    }
}
