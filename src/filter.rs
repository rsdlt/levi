//! fzf-style matching over commits and the files they touched.

use std::thread;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::git::Commit;

/// Below this many commits per thread, spawning costs more than it saves.
const PAR_CHUNK_MIN: usize = 4096;

/// Shortest query treated as a sha prefix. Shorter hex strings are too common.
const MIN_SHA_PREFIX: usize = 4;

/// A commit that survived the filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Index into the full commit list.
    pub commit: usize,
    pub score: u32,
    /// Indices of matching files, or `None` when the commit itself matched
    /// (then every file is shown).
    pub files: Option<Vec<usize>>,
}

impl Hit {
    pub fn all(commit: usize) -> Hit {
        Hit {
            commit,
            score: 0,
            files: None,
        }
    }
}

pub struct Filter {
    query: String,
    pattern: Pattern,
    text: Matcher,
    paths: Matcher,
    buf: Vec<char>,
    header: String,
}

impl Filter {
    pub fn new() -> Filter {
        Filter {
            query: String::new(),
            pattern: Pattern::default(),
            text: Matcher::new(Config::DEFAULT),
            paths: Matcher::new(Config::DEFAULT.match_paths()),
            buf: Vec::new(),
            header: String::new(),
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.atoms.is_empty()
    }

    /// Replaces the query. Returns true if every match of the new query is
    /// guaranteed to be a match of the old one, so callers may refilter only
    /// the previous hits.
    pub fn set_query(&mut self, query: &str) -> bool {
        // Appending only narrows: longer atoms and extra atoms both match less.
        // Negations (`!`), anchors (`$`) and escapes can widen or reinterpret
        // the query, so start over whenever they're involved.
        const SPECIAL: [char; 3] = ['!', '$', '\\'];
        let narrows = !self.query.is_empty()
            && query
                .strip_prefix(self.query.as_str())
                .is_some_and(|added| !added.contains(SPECIAL))
            && !self.query.contains(SPECIAL);
        self.query = query.to_owned();
        self.pattern
            .reparse(query, CaseMatching::Smart, Normalization::Smart);
        narrows
    }

    /// Matches a commit by sha prefix, subject, author or reflog action, or any
    /// file it touched.
    pub fn matches(&mut self, idx: usize, commit: &Commit) -> Option<Hit> {
        if self.is_empty() {
            return Some(Hit::all(idx));
        }
        let query = self.query.trim();
        if query.len() >= MIN_SHA_PREFIX
            && query.bytes().all(|b| b.is_ascii_hexdigit())
            && commit.sha.starts_with(&query.to_ascii_lowercase())
        {
            return Some(Hit {
                commit: idx,
                score: u32::MAX,
                files: None,
            });
        }

        // One haystack for the header, so a negation like `!wip` rejects the
        // commit when either field contains it.
        self.header.clear();
        self.header.push_str(&commit.subject);
        self.header.push('\t');
        self.header.push_str(&commit.author);
        if let Some(reflog) = &commit.reflog {
            self.header.push('\t');
            self.header.push_str(&reflog.action);
        }
        let header = score(&self.pattern, &mut self.text, &mut self.buf, &self.header);
        let mut best = header;
        let mut files = Vec::new();
        for (i, file) in commit.files.iter().enumerate() {
            let mut s = score(&self.pattern, &mut self.paths, &mut self.buf, &file.path);
            if let Some(old) = &file.old_path {
                s = s.max(score(&self.pattern, &mut self.paths, &mut self.buf, old));
            }
            if s.is_some() {
                files.push(i);
                best = best.max(s);
            }
        }
        Some(Hit {
            commit: idx,
            score: best?,
            files: header.is_none().then_some(files),
        })
    }

    /// Char positions in `text` matched by the query, for highlighting.
    pub fn highlights(&mut self, text: &str, is_path: bool) -> Vec<usize> {
        if self.is_empty() {
            return Vec::new();
        }
        let matcher = if is_path {
            &mut self.paths
        } else {
            &mut self.text
        };
        let mut indices = Vec::new();
        self.pattern
            .indices(Utf32Str::new(text, &mut self.buf), matcher, &mut indices);
        indices.sort_unstable();
        indices.dedup();
        indices.into_iter().map(|i| i as usize).collect()
    }
}

fn score(pattern: &Pattern, matcher: &mut Matcher, buf: &mut Vec<char>, text: &str) -> Option<u32> {
    pattern.score(Utf32Str::new(text, buf), matcher)
}

/// Matches `candidates` (indices into `commits`) against `query` across all
/// cores. Hits come back in candidate order.
pub fn par_matches(query: &str, commits: &[Commit], candidates: &[usize]) -> Vec<Hit> {
    let threads = thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = candidates.len().div_ceil(threads).max(PAR_CHUNK_MIN);
    let run = |chunk: &[usize]| {
        // Matchers hold scratch buffers, so each thread gets its own.
        let mut filter = Filter::new();
        filter.set_query(query);
        chunk
            .iter()
            .filter_map(|&i| filter.matches(i, &commits[i]))
            .collect::<Vec<_>>()
    };
    thread::scope(|s| {
        let handles: Vec<_> = candidates
            .chunks(chunk)
            .map(|c| s.spawn(move || run(c)))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("filter thread panicked"))
            .collect()
    })
}

/// Best first; equal scores keep log order (newest first).
pub fn sort(hits: &mut [Hit]) {
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.commit.cmp(&b.commit)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{FileChange, Status};

    fn commit(sha: &str, subject: &str, files: &[&str]) -> Commit {
        Commit {
            sha: sha.into(),
            short: sha[..7].into(),
            author: "Levi Ackerman".into(),
            date: "now".into(),
            subject: subject.into(),
            reflog: None,
            files: files
                .iter()
                .map(|p| FileChange {
                    status: Status::Modified,
                    path: (*p).into(),
                    old_path: None,
                })
                .collect(),
        }
    }

    fn hit(filter: &mut Filter, query: &str, c: &Commit) -> Option<Hit> {
        filter.set_query(query);
        filter.matches(0, c)
    }

    #[test]
    fn empty_query_matches_everything() {
        let c = commit("abcdef0123", "anything", &["a.rs"]);
        assert_eq!(hit(&mut Filter::new(), "", &c), Some(Hit::all(0)));
        assert_eq!(hit(&mut Filter::new(), "   ", &c), Some(Hit::all(0)));
    }

    #[test]
    fn subject_match_shows_all_files() {
        let c = commit("abcdef0123", "fix titan parser", &["src/a.rs", "src/b.rs"]);
        let h = hit(&mut Filter::new(), "titan", &c).unwrap();
        assert_eq!(h.files, None);
    }

    #[test]
    fn author_matches() {
        let c = commit("abcdef0123", "whatever", &["x"]);
        assert!(hit(&mut Filter::new(), "ackerman", &c).is_some());
    }

    #[test]
    fn file_match_shows_only_matching_files() {
        let c = commit(
            "abcdef0123",
            "refactor",
            &["src/parse.rs", "README.md", "src/ui.rs"],
        );
        let h = hit(&mut Filter::new(), "parse.rs", &c).unwrap();
        assert_eq!(h.files, Some(vec![0]));
        assert!(hit(&mut Filter::new(), "zzz", &c).is_none());
    }

    #[test]
    fn renamed_files_match_old_path() {
        let mut c = commit("abcdef0123", "move", &["new/place.rs"]);
        c.files[0].old_path = Some("old/spot.rs".into());
        assert_eq!(
            hit(&mut Filter::new(), "spot", &c).unwrap().files,
            Some(vec![0])
        );
    }

    #[test]
    fn sha_prefix_but_not_fuzzy_sha() {
        let c = commit("deadbeef00", "zzz", &["x"]);
        assert_eq!(
            hit(&mut Filter::new(), "DEADbe", &c).unwrap().score,
            u32::MAX
        );
        // Too short to be a sha, and not fuzzy-matched against the hash.
        assert!(hit(&mut Filter::new(), "dea", &c).is_none());
        assert!(hit(&mut Filter::new(), "dbf0", &c).is_none());
    }

    #[test]
    fn fzf_syntax() {
        let c = commit("abcdef0123", "add wall maria", &["walls.rs"]);
        let mut f = Filter::new();
        assert!(hit(&mut f, "'wall ^add", &c).is_some());
        assert!(
            hit(&mut f, "!maria", &c).is_some(),
            "file still matches the negation"
        );
        assert!(hit(&mut f, "!wall", &c).is_none());
        assert!(hit(&mut f, "Wall", &c).is_none(), "smart case");
    }

    #[test]
    fn narrowing() {
        let mut f = Filter::new();
        assert!(!f.set_query("a"));
        assert!(f.set_query("ab"));
        assert!(!f.set_query("a"), "deleting widens");
        assert!(!f.set_query("!a"));
        assert!(!f.set_query("!ab"), "longer negation widens");
        assert!(!f.set_query("x$"));
        assert!(!f.set_query("x$y"));
        assert!(!f.set_query("x"));
        assert!(f.set_query("x y"), "extra atom narrows");
        assert!(!f.set_query("x"));
        assert!(!f.set_query("x !y"), "added negation widens");
    }

    #[test]
    fn highlights_are_char_positions() {
        let mut f = Filter::new();
        f.set_query("tn");
        assert_eq!(f.highlights("titan", false), vec![0, 4]);
        f.set_query("");
        assert!(f.highlights("titan", false).is_empty());
    }

    #[test]
    fn sort_by_score_then_log_order() {
        let mut hits = vec![
            Hit {
                commit: 2,
                score: 5,
                files: None,
            },
            Hit {
                commit: 0,
                score: 1,
                files: None,
            },
            Hit {
                commit: 1,
                score: 5,
                files: None,
            },
        ];
        sort(&mut hits);
        let order: Vec<_> = hits.iter().map(|h| h.commit).collect();
        assert_eq!(order, [1, 2, 0]);
    }

    #[test]
    fn parallel_matches_sequential() {
        let commits: Vec<_> = (0..10_000)
            .map(|n| {
                commit(
                    "abcdef0123",
                    &format!("change {n}"),
                    &[&format!("src/m{}.rs", n % 7)],
                )
            })
            .collect();
        let candidates: Vec<_> = (0..commits.len()).rev().collect();
        let mut f = Filter::new();
        f.set_query("3");
        let sequential: Vec<_> = candidates
            .iter()
            .filter_map(|&i| f.matches(i, &commits[i]))
            .collect();
        assert!(sequential.iter().any(|h| h.files.is_none()), "header hits");
        assert!(
            sequential.iter().any(|h| h.files.is_some()),
            "file-only hits"
        );
        assert_eq!(par_matches("3", &commits, &candidates), sequential);
    }
}
