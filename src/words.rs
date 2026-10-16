//! Word-level highlighting, like git's `diff-highlight`: when removed lines are
//! followed by added ones, each pair is compared word by word and only the
//! words that changed are marked.

use std::ops::Range;

use crate::git::{DiffLine, LineKind};

type Ranges = Vec<Range<usize>>;

/// Word-by-word comparison is quadratic in the tokens a pair differs in, so
/// one diff gets this many steps of it. Past that, or for a single pair that
/// would cost more, the whole span between the shared ends is marked instead.
const BUDGET: usize = 1 << 20;

/// Fills `emph` for paired removed/added lines: the first removed line of a
/// block with the first added one after it, and so on.
pub fn mark(lines: &mut [DiffLine]) {
    let mut budget = BUDGET;
    let mut i = 0;
    while i < lines.len() {
        let removed = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let added = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        if added == removed {
            i += 1;
            continue;
        }
        for k in 0..(added - removed).min(i - added) {
            // Skip the leading `-`/`+`.
            let pair = changed(
                &lines[removed + k].text[1..],
                &lines[added + k].text[1..],
                &mut budget,
            );
            if let Some((old, new)) = pair {
                lines[removed + k].emph = shift(old);
                lines[added + k].emph = shift(new);
            }
        }
    }
}

fn shift(ranges: Ranges) -> Ranges {
    ranges.into_iter().map(|r| r.start + 1..r.end + 1).collect()
}

/// The changed byte ranges of `old` and `new`, or `None` when the lines have
/// too little in common for word marks to help (most of the line would be lit).
fn changed(old: &str, new: &str, budget: &mut usize) -> Option<(Ranges, Ranges)> {
    let a = tokens(old);
    let b = tokens(new);
    let (a_kept, b_kept) = common(old, &a, new, &b, budget);

    let weight = |text: &str, toks: &[Range<usize>], keep: Option<&[bool]>| -> usize {
        toks.iter()
            .enumerate()
            .filter(|&(i, t)| keep.is_none_or(|k| k[i]) && !text[t.clone()].trim().is_empty())
            .map(|(_, t)| t.len())
            .sum()
    };
    let same = weight(old, &a, Some(&a_kept));
    let total = weight(old, &a, None).max(weight(new, &b, None));
    if same * 2 < total {
        return None;
    }
    Some((spans(&a, &a_kept), spans(&b, &b_kept)))
}

/// Which tokens of each side belong to their longest common subsequence, or
/// just to their shared ends once `budget` can't pay for more.
fn common(
    old: &str,
    a: &[Range<usize>],
    new: &str,
    b: &[Range<usize>],
    budget: &mut usize,
) -> (Vec<bool>, Vec<bool>) {
    let eq = |i: usize, j: usize| old[a[i].clone()] == new[b[j].clone()];
    let mut a_kept = vec![true; a.len()];
    let mut b_kept = vec![true; b.len()];

    // Shared ends are kept as they are; only the middle needs the full comparison.
    let mut lo = 0;
    while lo < a.len() && lo < b.len() && eq(lo, lo) {
        lo += 1;
    }
    let (mut a_hi, mut b_hi) = (a.len(), b.len());
    while a_hi > lo && b_hi > lo && eq(a_hi - 1, b_hi - 1) {
        a_hi -= 1;
        b_hi -= 1;
    }
    let (n, m) = (a_hi - lo, b_hi - lo);
    let cost = (n + 1) * (m + 1);
    if cost > *budget {
        a_kept[lo..a_hi].fill(false);
        b_kept[lo..b_hi].fill(false);
        return (a_kept, b_kept);
    }
    *budget -= cost;

    // lcs[i][j]: length of the longest common subsequence of the middles from i and j on.
    let mut lcs = vec![0u32; cost];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if eq(lo + i, lo + j) {
                lcs[at(i + 1, j + 1)] + 1
            } else {
                lcs[at(i + 1, j)].max(lcs[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && eq(lo + i, lo + j) {
            i += 1;
            j += 1;
        } else if j == m || (i < n && lcs[at(i + 1, j)] >= lcs[at(i, j + 1)]) {
            a_kept[lo + i] = false;
            i += 1;
        } else {
            b_kept[lo + j] = false;
            j += 1;
        }
    }
    (a_kept, b_kept)
}

/// Merges runs of changed tokens into byte ranges.
fn spans(toks: &[Range<usize>], kept: &[bool]) -> Ranges {
    let mut out = Ranges::new();
    for (t, _) in toks.iter().zip(kept).filter(|&(_, &k)| !k) {
        match out.last_mut() {
            Some(last) if last.end == t.start => last.end = t.end,
            _ => out.push(t.clone()),
        }
    }
    out
}

#[derive(PartialEq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// Words, runs of whitespace, and single punctuation characters.
fn tokens(text: &str) -> Ranges {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let kind = class(c);
        let mut end = start + c.len_utf8();
        if kind != Class::Other {
            while let Some((at, next)) = chars.next_if(|&(_, next)| class(next) == kind) {
                end = at + next.len_utf8();
            }
        }
        out.push(start..end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(diff: &[&str]) -> Vec<DiffLine> {
        diff.iter()
            .map(|text| DiffLine {
                kind: match text.as_bytes()[0] {
                    b'-' => LineKind::Removed,
                    b'+' => LineKind::Added,
                    _ => LineKind::Context,
                },
                text: text.to_string(),
                emph: Vec::new(),
            })
            .collect()
    }

    /// The marked parts of each line.
    fn marked(diff: &[&str]) -> Vec<Vec<String>> {
        let mut lines = lines(diff);
        mark(&mut lines);
        lines
            .iter()
            .map(|l| {
                l.emph
                    .iter()
                    .map(|r| l.text[r.clone()].to_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn marks_only_the_changed_words() {
        assert_eq!(
            marked(&["-    let x = foo(1, a);", "+    let x = foo(2, a);"]),
            [vec!["1"], vec!["2"]]
        );
        assert_eq!(
            marked(&["-a.b(c).d", "+a.e(c).d.f"]),
            [vec!["b"], vec!["e", ".f"]]
        );
        assert_eq!(
            marked(&["-let titan = 1;", "+let titän = 1;"]),
            [vec!["titan"], vec!["titän"]]
        );
    }

    #[test]
    fn pairs_lines_in_order_within_a_block() {
        let got = marked(&[
            " ctx",
            "-let one = 1;",
            "-let two = 2;",
            "+let one = 10;",
            "+let two = 20;",
            "+let three = 3;",
            " ctx",
        ]);
        assert_eq!(
            got,
            [
                vec![],
                vec!["1"],
                vec!["2"],
                vec!["10"],
                vec!["20"],
                vec![],
                vec![]
            ]
        );
    }

    #[test]
    fn leaves_unrelated_lines_alone() {
        assert_eq!(
            marked(&["-foo(a, b)", "+bar(c, d)"]),
            [Vec::<String>::new(), vec![]]
        );
        // Added lines with nothing removed before them have nothing to compare to.
        assert_eq!(
            marked(&["+new", " ctx", "-old"]),
            [Vec::<String>::new(), vec![], vec![]]
        );
    }

    #[test]
    fn past_the_budget_marks_between_the_shared_ends() {
        let (old, new) = ("let x = foo(1, a);", "let x = foo(2, b);");
        let mut budget = 0;
        let (a, b) = changed(old, new, &mut budget).unwrap();
        assert_eq!((&old[a[0].clone()], &new[b[0].clone()]), ("1, a", "2, b"));
        let mut budget = BUDGET;
        let (a, _) = changed(old, new, &mut budget).unwrap();
        assert_eq!(a, [12..13, 15..16], "word by word while it lasts");
        assert!(budget < BUDGET);
    }
}
