//! Thin layer over the `git` CLI: repo discovery, streaming log, per-file diffs.

use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Output, Stdio};

use anyhow::{Context, Result, anyhow, bail};

/// Separates commit records in the log stream. Can't appear in a NUL-delimited field
/// produced by `-z`, and practically never appears in commit subjects.
const RECORD_SEP: u8 = 0x1e;
const LOG_FORMAT: &str = "--format=%x1e%H%x00%h%x00%an%x00%ar%x00%s";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub status: Status,
    pub path: String,
    /// Source path for renames and copies.
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Other(char),
}

impl Status {
    fn parse(token: &str) -> Status {
        match token.chars().next().unwrap_or('?') {
            'A' => Status::Added,
            'M' => Status::Modified,
            'D' => Status::Deleted,
            'R' => Status::Renamed,
            'C' => Status::Copied,
            'T' => Status::TypeChanged,
            c => Status::Other(c),
        }
    }

    pub fn letter(self) -> char {
        match self {
            Status::Added => 'A',
            Status::Modified => 'M',
            Status::Deleted => 'D',
            Status::Renamed => 'R',
            Status::Copied => 'C',
            Status::TypeChanged => 'T',
            Status::Other(c) => c,
        }
    }

    fn has_source(self) -> bool {
        matches!(self, Status::Renamed | Status::Copied)
    }
}

/// A `git` invocation with flags that keep output machine-stable regardless of user config.
fn git(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .arg("--literal-pathspecs")
        .stdin(Stdio::null());
    cmd
}

fn run(cmd: &mut Command) -> Result<Vec<u8>> {
    let out = output(cmd)?;
    if !out.status.success() {
        bail!("git: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

fn output(cmd: &mut Command) -> Result<Output> {
    cmd.output().map_err(|e| match e.kind() {
        ErrorKind::NotFound => anyhow!("git not found in PATH"),
        _ => anyhow!("failed to run git: {e}"),
    })
}

/// `--diff-merges` landed in git 2.31.
const MIN_GIT: (u32, u32) = (2, 31);

/// Fails with a readable message unless levi can run in `dir`. Returns the
/// directory git commands should run from: the work tree's top level (log and
/// diff paths are relative to it), or `dir` itself in a bare repository.
pub fn preflight(dir: &Path) -> Result<PathBuf> {
    let version = run(git(dir).arg("version"))?;
    let version = String::from_utf8_lossy(&version);
    if parse_version(&version).is_some_and(|v| v < MIN_GIT) {
        bail!(
            "needs git {}.{} or newer, found {}",
            MIN_GIT.0,
            MIN_GIT.1,
            version.trim()
        );
    }

    let out = output(git(dir).args(["rev-parse", "--is-bare-repository"]))?;
    if !out.status.success() {
        bail!("not inside a git repository");
    }
    let root = if out.stdout.starts_with(b"true") {
        dir.to_owned()
    } else {
        repo_root(dir)?
    };

    let head = output(git(&root).args(["rev-parse", "--verify", "--quiet", "HEAD"]))?;
    if !head.status.success() {
        bail!("no commits yet");
    }
    Ok(root)
}

/// Top-level directory of the work tree containing `dir`.
pub fn repo_root(dir: &Path) -> Result<PathBuf> {
    let mut cmd = git(dir);
    let out = run(cmd.args(["rev-parse", "--show-toplevel"]))?;
    Ok(PathBuf::from(String::from_utf8_lossy(&out).trim_end()))
}

/// `git version 2.39.3 (Apple Git-146)` → (2, 39)
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split_whitespace().nth(2)?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Starts `git log` for the current branch and returns a lazy stream of commits.
/// Merges are diffed against their first parent.
pub fn log(root: &Path) -> Result<LogStream> {
    let mut child = git(root)
        .args([
            "log",
            "--no-color",
            "--no-show-signature",
            "--diff-merges=first-parent",
            "--name-status",
            "-z",
            LOG_FORMAT,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    let stdout = child.stdout.take().expect("stdout is piped");
    Ok(LogStream {
        records: Records::new(BufReader::new(stdout)),
        child: Some(child),
    })
}

/// Commits from a running `git log`. Kills the process if dropped early.
pub struct LogStream {
    records: Records<BufReader<ChildStdout>>,
    child: Option<Child>,
}

impl Iterator for LogStream {
    type Item = Result<Commit>;

    fn next(&mut self) -> Option<Result<Commit>> {
        if let Some(item) = self.records.next() {
            return Some(item);
        }
        // EOF: surface a non-zero exit as a final error.
        let mut child = self.child.take()?;
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        match child.wait() {
            Ok(status) if status.success() => None,
            Ok(_) => Some(Err(anyhow::anyhow!("git log: {}", stderr.trim()))),
            Err(e) => Some(Err(e.into())),
        }
    }
}

impl Drop for LogStream {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Splits a `git log -z --name-status` byte stream into commits.
pub struct Records<R> {
    reader: R,
    buf: Vec<u8>,
    started: bool,
}

impl<R: BufRead> Records<R> {
    pub fn new(reader: R) -> Self {
        Records {
            reader,
            buf: Vec::new(),
            started: false,
        }
    }
}

impl<R: BufRead> Iterator for Records<R> {
    type Item = Result<Commit>;

    fn next(&mut self) -> Option<Result<Commit>> {
        loop {
            self.buf.clear();
            match self.reader.read_until(RECORD_SEP, &mut self.buf) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => return Some(Err(e.into())),
            }
            if self.buf.last() == Some(&RECORD_SEP) {
                self.buf.pop();
            }
            // The stream starts with a separator, so the first chunk is empty.
            if !self.started {
                self.started = true;
                if self.buf.is_empty() {
                    continue;
                }
            }
            return Some(parse_record(&self.buf));
        }
    }
}

/// Parses one record: `sha\0short\0author\0date\0subject\0` optionally followed by
/// `\nSTATUS\0path\0[path\0]...`.
fn parse_record(record: &[u8]) -> Result<Commit> {
    let text = String::from_utf8_lossy(record);
    let mut tokens = text.split('\0');
    let mut field = |name| {
        tokens
            .next()
            .with_context(|| format!("log record missing {name}"))
    };
    let mut commit = Commit {
        sha: field("sha")?.to_owned(),
        short: field("short sha")?.to_owned(),
        author: field("author")?.to_owned(),
        date: field("date")?.to_owned(),
        subject: field("subject")?.to_owned(),
        files: Vec::new(),
    };

    let mut tokens = tokens
        .map(|t| t.trim_start_matches('\n'))
        .filter(|t| !t.is_empty());
    while let Some(token) = tokens.next() {
        let status = Status::parse(token);
        let first = tokens.next().context("log record: status without path")?;
        let change = if status.has_source() {
            let path = tokens.next().context("log record: rename without target")?;
            FileChange {
                status,
                path: path.to_owned(),
                old_path: Some(first.to_owned()),
            }
        } else {
            FileChange {
                status,
                path: first.to_owned(),
                old_path: None,
            }
        };
        commit.files.push(change);
    }
    Ok(commit)
}

/// Unified diff of one file in one commit (merges: against the first parent).
pub fn diff(root: &Path, sha: &str, file: &FileChange) -> Result<String> {
    let mut cmd = git(root);
    cmd.args([
        "show",
        "--no-color",
        "--no-ext-diff",
        "--no-show-signature",
        "--diff-merges=first-parent",
        "--patch",
        "--format=",
        sha,
        "--",
    ]);
    if let Some(old) = &file.old_path {
        cmd.arg(old);
    }
    cmd.arg(&file.path);
    Ok(String::from_utf8_lossy(&run(&mut cmd)?).into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// `diff --git`, `index`, `---`/`+++`, mode and rename lines.
    Header,
    Hunk,
    Added,
    Removed,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// Classifies each line of a unified diff, expanding tabs for display.
pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    // `---`/`+++` are headers only before the first hunk, so track position.
    let mut in_hunk = false;
    text.lines()
        .map(|raw| {
            let kind = if raw.starts_with("diff --git") {
                in_hunk = false;
                LineKind::Header
            } else if raw.starts_with("@@") {
                in_hunk = true;
                LineKind::Hunk
            } else if !in_hunk {
                LineKind::Header
            } else if raw.starts_with('+') {
                LineKind::Added
            } else if raw.starts_with('-') {
                LineKind::Removed
            } else {
                LineKind::Context
            };
            let text = raw.replace('\t', "    ").replace('\r', "");
            DiffLine { kind, text }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Repo;
    use std::fs;
    use tempfile::TempDir;

    fn change(status: Status, path: &str) -> FileChange {
        FileChange {
            status,
            path: path.into(),
            old_path: None,
        }
    }

    #[test]
    fn root_commit() {
        let repo = Repo::new();
        repo.write("a.txt", "a\n");
        repo.write("dir/b.txt", "b\n");
        repo.commit("first blood");

        let commits = repo.log();
        assert_eq!(commits.len(), 1);
        let c = &commits[0];
        assert_eq!(c.sha.len(), 40);
        assert!(c.sha.starts_with(&c.short));
        assert_eq!(c.author, "Levi");
        assert_eq!(c.subject, "first blood");
        assert_eq!(
            c.files,
            vec![
                change(Status::Added, "a.txt"),
                change(Status::Added, "dir/b.txt")
            ]
        );

        let d = diff(repo.path(), &c.sha, &c.files[0]).unwrap();
        assert!(d.contains("new file mode"));
        assert!(d.contains("+a"));
        assert!(!d.contains("dir/b.txt"));
    }

    #[test]
    fn newest_first_and_statuses() {
        let repo = Repo::new();
        repo.write("keep.txt", "1\n");
        repo.write("gone.txt", "x\n");
        repo.commit("one");
        repo.write("keep.txt", "2\n");
        fs::remove_file(repo.path().join("gone.txt")).unwrap();
        repo.commit("two");

        let commits = repo.log();
        let subjects: Vec<_> = commits.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, ["two", "one"]);
        assert_eq!(
            commits[0].files,
            vec![
                change(Status::Deleted, "gone.txt"),
                change(Status::Modified, "keep.txt")
            ]
        );
    }

    #[test]
    fn rename() {
        let repo = Repo::new();
        repo.write("old.txt", "line one\nline two\nline three\nline four\n");
        repo.commit("add");
        repo.git(&["mv", "old.txt", "new.txt"]);
        repo.commit("move");

        let commits = repo.log();
        let file = &commits[0].files[0];
        assert_eq!(file.status, Status::Renamed);
        assert_eq!(file.path, "new.txt");
        assert_eq!(file.old_path.as_deref(), Some("old.txt"));

        let d = diff(repo.path(), &commits[0].sha, file).unwrap();
        assert!(d.contains("rename from old.txt"), "{d}");
        assert!(d.contains("rename to new.txt"), "{d}");
    }

    #[test]
    fn empty_commit_has_no_files() {
        let repo = Repo::new();
        repo.write("a", "a\n");
        repo.commit("a");
        repo.commit("nothing here");

        let commits = repo.log();
        assert_eq!(commits[0].subject, "nothing here");
        assert!(commits[0].files.is_empty());
        assert_eq!(commits[1].files.len(), 1);
    }

    #[test]
    fn merge_diffs_against_first_parent() {
        let repo = Repo::new();
        repo.write("base.txt", "base\n");
        repo.commit("base");
        repo.git(&["checkout", "-q", "-b", "feature"]);
        repo.write("feature.txt", "feature\n");
        repo.commit("feature work");
        repo.git(&["checkout", "-q", "main"]);
        repo.write("main.txt", "main\n");
        repo.commit("main work");
        repo.git(&["merge", "-q", "--no-edit", "feature"]);

        let commits = repo.log();
        // Full history of the branch, not just first-parent commits.
        assert_eq!(commits.len(), 4);
        let merge = &commits[0];
        assert!(merge.subject.starts_with("Merge branch 'feature'"));
        assert_eq!(merge.files, vec![change(Status::Added, "feature.txt")]);

        let d = diff(repo.path(), &merge.sha, &merge.files[0]).unwrap();
        assert!(d.contains("+feature"), "{d}");
    }

    #[test]
    fn awkward_paths() {
        let repo = Repo::new();
        repo.write("with space.txt", "s\n");
        repo.write("glob*.txt", "g\n");
        repo.write("globby.txt", "nope\n");
        repo.write("ünïcødé.txt", "u\n");
        repo.commit("weird names ✨");

        let c = &repo.log()[0];
        assert_eq!(c.subject, "weird names ✨");
        let paths: Vec<_> = c.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["glob*.txt", "globby.txt", "with space.txt", "ünïcødé.txt"]
        );

        // `*` must be taken literally, not as a glob matching globby.txt.
        let d = diff(repo.path(), &c.sha, &c.files[0]).unwrap();
        assert!(d.contains("+g"));
        assert!(!d.contains("nope"), "{d}");
    }

    #[test]
    fn repo_root_from_subdir() {
        let repo = Repo::new();
        repo.write("deep/er/file", "x\n");
        let root = repo_root(&repo.path().join("deep/er")).unwrap();
        assert_eq!(
            root.canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn preflight_errors() {
        let dir = TempDir::new().unwrap();
        let err = preflight(dir.path()).unwrap_err().to_string();
        assert_eq!(err, "not inside a git repository");

        let repo = Repo::new();
        let err = preflight(repo.path()).unwrap_err().to_string();
        assert_eq!(err, "no commits yet");

        repo.write("a", "a\n");
        repo.commit("a");
        let root = preflight(&repo.path().join(".git"))
            .unwrap_err()
            .to_string();
        assert!(
            root.contains("work tree"),
            "inside .git is neither bare nor a work tree: {root}"
        );
        let root = preflight(repo.path()).unwrap();
        assert_eq!(
            root.canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn bare_repository() {
        let repo = Repo::new();
        repo.write("a.txt", "a\n");
        repo.commit("in a bare repo");
        let bare = TempDir::new().unwrap();
        let bare_path = bare.path().join("r.git");
        repo.git(&["clone", "-q", "--bare", ".", bare_path.to_str().unwrap()]);

        let root = preflight(&bare_path).unwrap();
        let commits: Vec<_> = log(&root).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(commits[0].subject, "in a bare repo");
        let d = diff(&root, &commits[0].sha, &commits[0].files[0]).unwrap();
        assert!(d.contains("+a"));
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("git version 2.56.0\n"), Some((2, 56)));
        assert_eq!(
            parse_version("git version 2.39.3 (Apple Git-146)"),
            Some((2, 39))
        );
        assert_eq!(parse_version("git version 2.45.2.windows.1"), Some((2, 45)));
        assert_eq!(parse_version("nonsense"), None);
        assert!(Some((2, 30)) < Some(MIN_GIT));
    }

    #[test]
    fn parses_copy_records() {
        let raw = b"\x1esha1\0s1\0Levi\0now\0copy\0\nC75\0src.rs\0dst.rs\0M\0x\0\x1esha2\0s2\0Erwin\0then\0root\0";
        let commits: Vec<_> = Records::new(&raw[..]).collect::<Result<_>>().unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(
            commits[0].files,
            vec![
                FileChange {
                    status: Status::Copied,
                    path: "dst.rs".into(),
                    old_path: Some("src.rs".into()),
                },
                change(Status::Modified, "x"),
            ]
        );
        assert_eq!(commits[1].author, "Erwin");
        assert!(commits[1].files.is_empty());
    }

    #[test]
    fn parse_diff_tracks_hunks() {
        let text = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n ctx\tx\r\n";
        let kinds: Vec<_> = parse_diff(text).iter().map(|l| l.kind).collect();
        use LineKind::*;
        assert_eq!(
            kinds,
            [Header, Header, Header, Hunk, Removed, Added, Context]
        );
        assert_eq!(parse_diff(text)[6].text, " ctx    x");
    }
}
