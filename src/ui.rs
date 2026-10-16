//! Rendering: search bar on top, commits | files | diff below.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::app::{App, Diff, Focus};
use crate::git::Status;

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
    let count = format!(" {} commits ", app.commits.len());
    let block = Block::bordered()
        .title(" levi ".bold())
        .title_bottom(Line::from(count).right_aligned())
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

    let lines: Vec<Line> = match &app.diff {
        Some(Diff { text: Ok(text), .. }) => colorize(text, app.diff_scroll, app.diff_height),
        Some(Diff { text: Err(err), .. }) => vec![Line::styled(err.as_str(), Color::Red)],
        None if app.commit().is_some() => vec![Line::styled("no file changes", Style::new().dim())],
        None => vec![],
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Styles the visible window of a unified diff.
fn colorize(text: &str, skip: usize, take: usize) -> Vec<Line<'static>> {
    // Lines before the first hunk are headers; whether a line is header or
    // content depends on what came before, so scan from the top.
    let mut in_hunk = false;
    let mut out = Vec::with_capacity(take);
    for (i, raw) in text.lines().enumerate() {
        if raw.starts_with("diff --git") {
            in_hunk = false;
        } else if raw.starts_with("@@") {
            in_hunk = true;
        }
        if i < skip {
            continue;
        }
        if out.len() == take {
            break;
        }
        let style = if raw.starts_with("@@") {
            Style::new().fg(ACCENT)
        } else if !in_hunk {
            Style::new().dim().bold()
        } else if raw.starts_with('+') {
            Style::new().fg(Color::Green)
        } else if raw.starts_with('-') {
            Style::new().fg(Color::Red)
        } else {
            Style::new()
        };
        let line = raw.replace('\t', "    ").replace('\r', "");
        out.push(Line::styled(line, style));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Exit;
    use crate::testutil::Repo;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn three_panes_and_navigation() {
        let repo = Repo::new();
        repo.write("tatakae.rs", "fn main() {}\n");
        repo.commit("first");
        repo.write("tatakae.rs", "fn main() { attack(); }\n");
        repo.write("wall.md", "maria\n");
        repo.commit("second");

        let mut app = App::new(repo.path().to_owned(), repo.log());
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("second"));
        assert!(screen.contains("first"));
        assert!(screen.contains("M tatakae.rs"));
        assert!(screen.contains("A wall.md"));
        assert!(screen.contains("+fn main() { attack(); }"));

        // Files pane: move to wall.md, diff follows.
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("+maria"));

        // Back to commits, go to the older one: file selection resets.
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Down);
        let screen = render(&mut app);
        assert!(screen.contains("+fn main() {}"));
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
    fn colorize_scrolls_and_tracks_hunks() {
        let text = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
        let lines = colorize(text, 4, 10);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].style.fg, Some(Color::Red));
        assert_eq!(lines[1].style.fg, Some(Color::Green));
        // `---`/`+++` are headers, not removals/additions.
        assert_eq!(colorize(text, 1, 1)[0].style.fg, None);
    }
}
