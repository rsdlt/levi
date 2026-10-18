//! Syntax colors for the code in a diff, in the terminal's own 16 colors, so
//! they follow whatever theme the terminal has. One fixed mapping from grammar
//! scopes to colors: no themes, no settings.

use std::ops::Range;
use std::str::FromStr;
use std::sync::OnceLock;
use std::thread;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, Theme, ThemeItem};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::git::{DiffLine, LineKind};

/// Byte ranges of a line and the ANSI color (0–15) of each.
pub type Colors = Vec<(Range<usize>, u8)>;

/// Bytes of code one diff may color (about 30,000 lines, a second's work).
/// Past it, the rest keeps the plain green/red look.
const BUDGET: usize = 1 << 20;

/// Lines colored between checks for whether the diff is still wanted.
const CHECK_EVERY: usize = 32;

/// Lines longer than this (minified code, data) end coloring for that side of
/// the hunk: grammars get slow on them, and they're rarely read anyway.
const LONGEST: usize = 1000;

/// Scopes to ANSI colors, tried most specific first by syntect.
const RULES: &[(&str, u8)] = &[
    ("comment, punctuation.definition.comment", 8),
    (
        "string, constant.character, punctuation.definition.string",
        2,
    ),
    ("keyword - keyword.operator, storage, variable.language", 5),
    (
        "entity.name.function, support.function, variable.function, entity.name.tag, markup.heading",
        4,
    ),
    (
        "constant.numeric, constant.language, support.constant, entity.name.type, \
         entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, \
         support.type, support.class, entity.other.attribute-name",
        3,
    ),
];

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_nonewlines)
}

fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| Theme {
        scopes: RULES
            .iter()
            .map(|&(scopes, color)| ThemeItem {
                scope: ScopeSelectors::from_str(scopes).expect("valid scope selectors"),
                style: StyleModifier {
                    // An ANSI index travels in `r`, marked by a zero alpha;
                    // syntect's own default (opaque black) means "no color".
                    foreground: Some(Color {
                        r: color,
                        g: 0,
                        b: 0,
                        a: 0,
                    }),
                    background: None,
                    font_style: None,
                },
            })
            .collect(),
        ..Theme::default()
    })
}

/// Loads the grammars in the background, so the first diff needn't wait.
pub fn warm_up() {
    thread::spawn(|| {
        syntaxes();
        theme();
    });
}

/// The grammar for `path`, by extension or, for names like `Makefile`, by name.
fn find(path: &str) -> Option<&'static SyntaxReference> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let set = syntaxes();
    let ext = name.rsplit_once('.').map(|(_, ext)| ext);
    ext.and_then(|ext| set.find_syntax_by_extension(ext))
        .or_else(|| set.find_syntax_by_extension(name))
        // Plain text has a grammar too, but nothing to color.
        .filter(|s| s.name != "Plain Text")
}

/// Colors the code lines of one file's diff. Each hunk's old side (context and
/// removed lines) and new side (context and added lines) are parsed apart, in
/// order, the way each version of the file reads; context lines take the new
/// side's colors. A hunk that starts inside a multi-line comment or string is
/// colored wrong until it ends: the price of not reading whole files.
///
/// Every few lines it asks `wanted` whether to go on, and stops if not: the
/// diff on screen may have changed. Returns whether anything got colored.
pub fn mark(lines: &mut [DiffLine], path: &str, mut wanted: impl FnMut() -> bool) -> bool {
    let Some(syntax) = find(path) else {
        return false;
    };
    let (set, theme) = (syntaxes(), theme());
    let fresh = || Some(HighlightLines::new(syntax, theme));
    let (mut old, mut new) = (None, None);
    let mut budget = BUDGET;
    for (i, line) in lines.iter_mut().enumerate() {
        if i % CHECK_EVERY == CHECK_EVERY - 1 && !wanted() {
            return false;
        }
        if line.kind == LineKind::Hunk {
            (old, new) = (fresh(), fresh());
            continue;
        }
        if !matches!(
            line.kind,
            LineKind::Context | LineKind::Added | LineKind::Removed
        ) {
            continue;
        }
        let code = &line.text[1..];
        if code.len() > budget {
            return true;
        }
        budget -= code.len();
        let too_long = code.len() > LONGEST;
        let color = |side: &mut Option<HighlightLines>| -> Option<Colors> {
            if too_long {
                *side = None;
            }
            let ranges = side.as_mut()?.highlight_line(code, set).ok()?;
            let mut at = 1;
            let mut out = Colors::new();
            for (style, piece) in ranges {
                let fg = style.foreground;
                let end = at + piece.len();
                if fg.a == 0 && !piece.is_empty() {
                    // Neighbors of one color make one range.
                    match out.last_mut() {
                        Some((r, c)) if r.end == at && *c == fg.r => r.end = end,
                        _ => out.push((at..end, fg.r)),
                    }
                }
                at += piece.len();
            }
            Some(out)
        };
        line.syntax = match line.kind {
            LineKind::Added => color(&mut new),
            LineKind::Removed => color(&mut old),
            _ => {
                // Keep the old side in step; the new side's colors are shown.
                color(&mut old);
                color(&mut new)
            }
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(lines: &[(LineKind, &str)]) -> Vec<DiffLine> {
        lines
            .iter()
            .map(|&(kind, text)| DiffLine {
                kind,
                text: text.into(),
                emph: Vec::new(),
                syntax: None,
            })
            .collect()
    }

    /// The colored pieces of a line, as (text, ANSI color).
    fn pieces(line: &DiffLine) -> Vec<(&str, u8)> {
        line.syntax
            .as_ref()
            .unwrap()
            .iter()
            .map(|(r, c)| (&line.text[r.clone()], *c))
            .collect()
    }

    #[test]
    fn colors_code_by_language() {
        let mut lines = diff(&[
            (LineKind::Header, "diff --git a/x.rs b/x.rs"),
            (LineKind::Hunk, "@@ -1,2 +1,2 @@"),
            (LineKind::Context, " // the wall"),
            (LineKind::Removed, "-let name = \"levi\";"),
            (LineKind::Added, "+fn attack() {}"),
        ]);
        mark(&mut lines, "src/x.rs", || true);
        assert_eq!(lines[0].syntax, None, "headers aren't code");
        assert_eq!(pieces(&lines[2]), [("// the wall", 8)]);
        let removed = pieces(&lines[3]);
        assert!(removed.contains(&("let", 5)), "{removed:?}");
        assert!(removed.contains(&("\"levi\"", 2)) || removed.contains(&("levi", 2)));
        let added = pieces(&lines[4]);
        assert!(
            added.contains(&("fn", 5)) && added.contains(&("attack", 4)),
            "{added:?}"
        );
    }

    #[test]
    fn each_side_reads_as_its_own_file() {
        // The old side opens a block comment that the new side never had: the
        // added line after it must not be colored as a comment.
        let mut lines = diff(&[
            (LineKind::Hunk, "@@ -1,2 +1,2 @@"),
            (LineKind::Removed, "-/* old"),
            (LineKind::Added, "+let x = 1;"),
        ]);
        mark(&mut lines, "a.js", || true);
        assert!(pieces(&lines[1]).iter().all(|&(_, c)| c == 8));
        assert!(pieces(&lines[2]).iter().all(|&(_, c)| c != 8));
    }

    #[test]
    fn unknown_files_and_huge_lines_stay_plain() {
        let mut lines = diff(&[(LineKind::Hunk, "@@ -1 +1 @@"), (LineKind::Added, "+hello")]);
        mark(&mut lines, "notes.unknownext", || true);
        assert_eq!(lines[1].syntax, None);
        mark(&mut lines, "LICENSE", || true);
        assert_eq!(lines[1].syntax, None, "plain text has nothing to color");

        let long = format!("+let x = \"{}\";", "a".repeat(LONGEST));
        let mut lines = diff(&[
            (LineKind::Hunk, "@@ -1 +1,2 @@"),
            (LineKind::Added, &long),
            (LineKind::Added, "+let y = 2;"),
        ]);
        mark(&mut lines, "a.rs", || true);
        assert_eq!((&lines[1].syntax, &lines[2].syntax), (&None, &None));
    }

    /// `cargo test --release -- --ignored --nocapture syntax::tests::speed`
    #[test]
    #[ignore]
    fn speed() {
        use std::time::Instant;
        let t = Instant::now();
        syntaxes();
        println!("load grammars: {:?}", t.elapsed());
        // levi's own code, as added lines, up to the budget.
        let code: String = ["src/app.rs", "src/ui.rs", "src/git.rs", "src/worker.rs"]
            .iter()
            .map(|f| std::fs::read_to_string(f).unwrap())
            .collect();
        let mut text = vec![(LineKind::Hunk, "@@ -0,0 +1 @@".to_owned())];
        let mut bytes = 0;
        for line in code.lines().cycle() {
            bytes += line.len();
            if bytes > BUDGET {
                break;
            }
            text.push((LineKind::Added, format!("+{line}")));
        }
        for n in [500, text.len()] {
            let mut lines: Vec<DiffLine> = text[..n]
                .iter()
                .map(|(kind, t)| DiffLine {
                    kind: *kind,
                    text: t.clone(),
                    emph: Vec::new(),
                    syntax: None,
                })
                .collect();
            let t = Instant::now();
            mark(&mut lines, "x.rs", || true);
            println!("{n} lines: {:?}", t.elapsed());
        }
    }

    #[test]
    fn makefiles_are_found_by_name() {
        assert!(find("build/Makefile").is_some());
        assert!(find("src/main.rs").is_some());
        assert!(find("README").is_none());
    }
}
