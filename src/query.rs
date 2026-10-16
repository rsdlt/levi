//! Splits what's typed in the search bar into the fuzzy filter and slash commands.
//!
//! - `/r` anywhere switches the commits column to the reflog.
//! - `/c` starts a code search: everything after it, spaces included, is the
//!   text to look for in the diffs. Everything before it stays fuzzy.
//!
//! Commands are whole words, so `src/config` or `/cache` stay fuzzy text.

use std::ops::Range;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Log,
    Reflog,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub fuzzy: String,
    pub source: Source,
    /// Text the diffs must contain, if a code search was asked for.
    pub code: Option<String>,
}

pub fn parse(raw: &str) -> Query {
    let (before, code) = match find_word(raw, "/c") {
        Some(at) => {
            let pattern = raw[at + 2..].trim_start();
            (
                &raw[..at],
                (!pattern.is_empty()).then(|| pattern.to_owned()),
            )
        }
        None => (raw, None),
    };

    let mut source = Source::Log;
    let mut fuzzy = String::new();
    let mut rest = before;
    while let Some(at) = find_word(rest, "/r") {
        source = Source::Reflog;
        fuzzy.push_str(&rest[..at]);
        rest = &rest[at + 2..];
    }
    fuzzy.push_str(rest);

    Query {
        fuzzy: fuzzy.split_whitespace().collect::<Vec<_>>().join(" "),
        source,
        code,
    }
}

/// Byte offset of `word` standing alone: at the start or after whitespace, and
/// at the end or before whitespace.
fn find_word(text: &str, word: &str) -> Option<usize> {
    text.match_indices(word).map(|(at, _)| at).find(|&at| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        before.is_none_or(char::is_whitespace) && after.is_none_or(char::is_whitespace)
    })
}

/// Where code-search `pattern` occurs in `line`, as byte ranges, matching the way
/// the search itself does: literally, ignoring case unless the pattern has an
/// uppercase letter. (Case folding is ASCII-only, close enough for highlighting.)
pub fn find_code(line: &str, pattern: &str) -> Vec<Range<usize>> {
    let (hay, needle) = (line.as_bytes(), pattern.as_bytes());
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    let ignore_case = !pattern.chars().any(char::is_uppercase);
    let mut found = Vec::new();
    let mut at = 0;
    while at + needle.len() <= hay.len() {
        let window = &hay[at..at + needle.len()];
        let hit = if ignore_case {
            window.eq_ignore_ascii_case(needle)
        } else {
            window == needle
        };
        if hit {
            found.push(at..at + needle.len());
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(fuzzy: &str, source: Source, code: Option<&str>) -> Query {
        Query {
            fuzzy: fuzzy.into(),
            source,
            code: code.map(Into::into),
        }
    }

    #[test]
    fn plain_query_is_all_fuzzy() {
        assert_eq!(parse("parse.rs fix"), q("parse.rs fix", Source::Log, None));
        assert_eq!(parse(""), Query::default());
    }

    #[test]
    fn reflog_anywhere() {
        assert_eq!(parse("/r"), q("", Source::Reflog, None));
        assert_eq!(parse("/r parse"), q("parse", Source::Reflog, None));
        assert_eq!(parse("parse /r"), q("parse", Source::Reflog, None));
        assert_eq!(parse("a /r b"), q("a b", Source::Reflog, None));
    }

    #[test]
    fn code_takes_the_rest_of_the_line() {
        assert_eq!(
            parse("parse.rs /c let x = 1"),
            q("parse.rs", Source::Log, Some("let x = 1"))
        );
        assert_eq!(parse("/c  fn main"), q("", Source::Log, Some("fn main")));
        // `/r` after `/c` is part of the code, not a command.
        assert_eq!(parse("/c a /r b"), q("", Source::Log, Some("a /r b")));
        assert_eq!(parse("/r /c foo"), q("", Source::Reflog, Some("foo")));
    }

    #[test]
    fn empty_code_search_is_no_code_search() {
        assert_eq!(parse("/c"), q("", Source::Log, None));
        assert_eq!(parse("x /c   "), q("x", Source::Log, None));
    }

    #[test]
    fn commands_are_whole_words() {
        assert_eq!(parse("src/config"), q("src/config", Source::Log, None));
        assert_eq!(parse("/cache /rust"), q("/cache /rust", Source::Log, None));
        assert_eq!(parse("a/r b/c"), q("a/r b/c", Source::Log, None));
    }

    #[test]
    fn find_code_is_literal_with_smart_case() {
        assert_eq!(find_code("let x = Foo(foo)", "foo"), vec![8..11, 12..15]);
        assert_eq!(find_code("let x = Foo(foo)", "Foo"), vec![8..11]);
        assert_eq!(find_code("a.b axb", "a.b"), vec![0..3]);
        assert_eq!(find_code("ñandú ñ", "ñ"), vec![0..2, 8..10]);
        assert!(find_code("short", "longer text").is_empty());
    }
}
