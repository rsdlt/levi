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
    let status = match &app.log_error {
        Some(err) => Line::styled(format!(" {err} "), Color::Red),
        None if app.loading => Line::from(format!(" {} commits… ", app.commits.len())),
        None => Line::from(format!(" {} commits ", app.commits.len())),
    };
    let block = Block::bordered()
        .title(" levi ".bold())
        .title_bottom(status.right_aligned())
        .border_style(Style::new().dark_gray());
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn draw_commits(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .commits
        .iter()
        .map(|c| {
            ListItem::new(Line::from(vec![
                Span::styled(&c.short, Style::new().fg(Color::Yellow)),
                Span::raw(" "),
                Span::raw(&c.subject),
            ]))
        })
        .collect();
    let focused = app.focus == Focus::Commits;
    let list = List::new(items)
        .block(pane("commits", focused))
        .highlight_style(highlight(focused));
    let mut state = ListState::default().with_selected(Some(app.commit_idx));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_files(frame: &mut Frame, app: &App, area: Rect) {
    let files = app.commit().map_or(&[][..], |c| &c.files[..]);
    let items: Vec<ListItem> = files
        .iter()
        .map(|f| {
            let mut spans = vec![
                Span::styled(f.status.letter().to_string(), status_style(f.status)),
                Span::raw(" "),
            ];
            if let Some(old) = &f.old_path {
                spans.push(Span::styled(old, Style::new().dim()));
                spans.push(Span::styled(" → ", Style::new().dim()));
            }
            spans.push(Span::raw(&f.path));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let focused = app.focus == Focus::Files;
    let list = List::new(items)
        .block(pane("files", focused))
        .highlight_style(highlight(focused));
    let mut state = ListState::default().with_selected(Some(app.file_idx));
    frame.render_stateful_widget(list, area, &mut state);
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
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<String> = (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        rows.join("\n")
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
}
