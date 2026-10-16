//! Thin layer over the `git` CLI: repo discovery, streaming log, per-file diffs.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};

use crate::query::Source;
use crate::words;

/// Separates commit records in the log stream. Can't appear in a NUL-delimited field
/// produced by `-z`, and practically never appears in commit subjects.
const RECORD_SEP: u8 = 0x1e;
/// Reflog fields (`%gd`, `%gs`) come out empty outside reflog mode, so one parser serves both.
const LOG_FORMAT: &str = "--format=%x1e%H%x00%h%x00%an%x00%ar%x00%s%x00%gd%x00%gs";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    /// Set when the commit comes from walking the reflog.
    pub reflog: Option<Reflog>,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reflog {
    /// `HEAD@{3}`
    pub selector: String,
    /// `checkout: moving from main to fix`
    pub action: String,
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

/// Fails with a readable message unless levi can run in `dir`, on `rev` if one
/// was given. Returns the directory git commands should run from: the work
/// tree's top level (log and diff paths are relative to it), or `dir` itself in
/// a bare repository.
pub fn preflight(dir: &Path, rev: Option<&str>) -> Result<PathBuf> {
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

    let Some(rev) = rev else {
        let head = output(git(&root).args(["rev-parse", "--verify", "--quiet", "HEAD"]))?;
        if !head.status.success() {
            bail!("no commits yet");
        }
        return Ok(root);
    };
    // `--` makes git read `rev` as a revision only, never as a path.
    let first =
        output(git(&root).args(["rev-list", "--max-count=1", "--end-of-options", rev, "--"]))?;
    if !first.status.success() {
        bail!("bad revision '{rev}'");
    }
    if first.stdout.is_empty() {
        bail!("no commits in '{rev}'");
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

/// Starts `git log` for the current branch (or walks HEAD's reflog) and returns a
/// lazy stream of commits. Merges are diffed against their first parent.
pub fn log(root: &Path, source: Source) -> Result<LogStream> {
    let mut cmd = log_command(root);
    if source == Source::Reflog {
        cmd.arg("--walk-reflogs");
    }
    stream(cmd, &[])
}

/// The same records as `log`, for exactly `shas`, in that order. Lets callers
/// split history into chunks and load them in parallel.
pub fn log_commits(root: &Path, shas: &[String]) -> Result<LogStream> {
    let mut cmd = log_command(root);
    cmd.args(["--no-walk=unsorted", "--stdin"]);
    stream(cmd, shas)
}

/// Commit ids of `rev` (a branch, a range like `main..HEAD`, …) in `git log`
/// order, as git lists them. Kills git if dropped early.
pub fn rev_list(root: &Path, rev: &str) -> Result<RevList> {
    let mut child = git(root)
        .args(["rev-list", "--end-of-options", rev, "--"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    let stdout = child.stdout.take().expect("stdout is piped");
    Ok(RevList {
        lines: BufReader::new(stdout).lines(),
        child,
    })
}

pub struct RevList {
    lines: std::io::Lines<BufReader<ChildStdout>>,
    child: Child,
}

impl Iterator for RevList {
    type Item = Result<String>;

    fn next(&mut self) -> Option<Result<String>> {
        match self.lines.next() {
            Some(line) => Some(line.map_err(Into::into)),
            // EOF: surface a non-zero exit as a final error.
            None => match self.child.wait() {
                Ok(status) if status.success() => None,
                Ok(_) => {
                    let mut stderr = String::new();
                    if let Some(mut pipe) = self.child.stderr.take() {
                        let _ = pipe.read_to_string(&mut stderr);
                    }
                    Some(Err(anyhow!("git rev-list: {}", stderr.trim())))
                }
                Err(e) => Some(Err(e.into())),
            },
        }
    }
}

impl Drop for RevList {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn log_command(root: &Path) -> Command {
    let mut cmd = git(root);
    cmd.args([
        "log",
        "--no-color",
        "--no-show-signature",
        "--diff-merges=first-parent",
        "--name-status",
        "-z",
        LOG_FORMAT,
    ]);
    cmd
}

/// Spawns `cmd`, feeds it `input` one line each, and streams the records it prints.
fn stream(mut cmd: Command, input: &[String]) -> Result<LogStream> {
    let mut child = cmd
        .stdin(if input.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    if let Some(mut stdin) = child.stdin.take() {
        // `--stdin` reads everything before printing anything, so this can't deadlock.
        for line in input {
            writeln!(stdin, "{line}")?;
        }
    }
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

/// Parses one record: `sha\0short\0author\0date\0subject\0selector\0action\0`, optionally followed by
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
        reflog: None,
        files: Vec::new(),
    };
    let selector = field("reflog selector")?;
    let action = field("reflog action")?;
    if !selector.is_empty() {
        commit.reflog = Some(Reflog {
            selector: selector.to_owned(),
            action: action.to_owned(),
        });
    }

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

/// Commits whose diffs add or remove a line containing `pattern`, out of `shas`,
/// each with the paths where that happens. Matching is literal; it ignores case
/// unless the pattern has an uppercase letter. With `paths`, only those files are
/// diffed. Kills git if dropped early.
pub fn code_search(
    root: &Path,
    pattern: &str,
    shas: &[String],
    paths: Option<&[String]>,
) -> Result<CodeStream> {
    let mut cmd = git(root);
    cmd.args([
        "log",
        "--no-color",
        "--no-show-signature",
        "--no-walk=unsorted",
        "--stdin",
        "--diff-merges=first-parent",
        "--name-only",
        "-z",
        "--format=%x1e%H%x00",
        "-G",
        &literal_regex(pattern),
    ]);
    if !pattern.chars().any(char::is_uppercase) {
        cmd.arg("--regexp-ignore-case");
    }
    if let Some(paths) = paths {
        cmd.arg("--").args(paths);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to run git")?;
    // A chunk of shas fits in the pipe buffer, so writing it all up front can't block.
    let mut stdin = child.stdin.take().expect("stdin is piped");
    for sha in shas {
        writeln!(stdin, "{sha}")?;
    }
    drop(stdin);
    let stdout = child.stdout.take().expect("stdout is piped");
    Ok(CodeStream {
        reader: BufReader::new(stdout),
        buf: Vec::new(),
        child: Arc::new(Mutex::new(child)),
    })
}

/// Escapes POSIX extended regex syntax (what `-G` speaks) so `text` matches literally.
fn literal_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if r".[]()*+?{}|^$\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `(sha, paths)` for each matching commit, as git finds them.
pub struct CodeStream {
    reader: BufReader<ChildStdout>,
    buf: Vec<u8>,
    child: Arc<Mutex<Child>>,
}

impl CodeStream {
    /// The git process, so another thread can kill it while this one is
    /// blocked waiting for output.
    pub fn process(&self) -> Arc<Mutex<Child>> {
        self.child.clone()
    }
}

impl Iterator for CodeStream {
    type Item = (String, Vec<String>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.buf.clear();
            if self.reader.read_until(RECORD_SEP, &mut self.buf).ok()? == 0 {
                return None;
            }
            if self.buf.last() == Some(&RECORD_SEP) {
                self.buf.pop();
            }
            let text = String::from_utf8_lossy(&self.buf);
            let mut tokens = text
                .split('\0')
                .map(|t| t.trim_start_matches('\n'))
                .filter(|t| !t.is_empty());
            // The stream starts with a separator, so the first chunk is empty.
            let Some(sha) = tokens.next() else { continue };
            return Some((sha.to_owned(), tokens.map(str::to_owned).collect()));
        }
    }
}

impl Drop for CodeStream {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// What `git show` prints for one file of a commit: the header and message, a
/// NUL, then the file's unified diff (merges: against the first parent). With no
/// file, just the message.
pub fn diff(root: &Path, sha: &str, file: Option<&FileChange>) -> Result<String> {
    let mut cmd = git(root);
    cmd.args([
        "show",
        "--no-color",
        "--no-ext-diff",
        "--no-show-signature",
        "--diff-merges=first-parent",
        // Laid out like `git show`; the date follows the user's `log.date`.
        // The NUL must come after `%w` is reset, or the wrapping drops it.
        "--format=commit %H%nAuthor: %an <%ae>%nDate:   %ad%n%n%w(0,4,4)%B%w(0,0,0)%x00",
    ]);
    match file {
        Some(file) => {
            cmd.args(["--patch", sha, "--"]);
            if let Some(old) = &file.old_path {
                cmd.arg(old);
            }
            cmd.arg(&file.path);
        }
        None => {
            cmd.args(["--no-patch", sha]);
        }
    }
    Ok(String::from_utf8_lossy(&run(&mut cmd)?).into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// `commit`, `Author:` and `Date:` above the message.
    Meta,
    /// The commit message, indented like `git show` does.
    Message,
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
    /// Byte ranges of `text` that changed within the line (see `words`).
    pub emph: Vec<Range<usize>>,
}

/// Classifies each line of `diff`'s output, expanding tabs for display, and
/// marks the words that changed in edited lines.
pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    let (message, patch) = text.split_once('\0').unwrap_or(("", text));
    let line = |kind, raw: &str| DiffLine {
        kind,
        text: raw.replace('\t', "    ").replace('\r', ""),
        emph: Vec::new(),
    };
    // The header ends at the first blank line; the message follows.
    let mut in_message = false;
    let mut lines: Vec<DiffLine> = message
        .trim_end()
        .lines()
        .map(|raw| {
            in_message |= raw.trim().is_empty();
            let kind = if in_message {
                LineKind::Message
            } else {
                LineKind::Meta
            };
            line(kind, raw.trim_end())
        })
        .collect();
    let patch = patch.trim_start_matches('\n');
    if !lines.is_empty() && !patch.is_empty() {
        lines.push(line(LineKind::Message, ""));
    }

    // `---`/`+++` are headers only before the first hunk, so track position.
    let mut in_hunk = false;
    lines.extend(patch.lines().map(|raw| {
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
        line(kind, raw)
    }));
    words::mark(&mut lines);
    lines
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

        let d = diff(repo.path(), &c.sha, Some(&c.files[0])).unwrap();
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

        let d = diff(repo.path(), &commits[0].sha, Some(file)).unwrap();
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

        let d = diff(repo.path(), &merge.sha, Some(&merge.files[0])).unwrap();
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
        let d = diff(repo.path(), &c.sha, Some(&c.files[0])).unwrap();
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
        let err = preflight(dir.path(), None).unwrap_err().to_string();
        assert_eq!(err, "not inside a git repository");

        let repo = Repo::new();
        let err = preflight(repo.path(), None).unwrap_err().to_string();
        assert_eq!(err, "no commits yet");

        repo.write("a", "a\n");
        repo.commit("a");
        let root = preflight(&repo.path().join(".git"), None)
            .unwrap_err()
            .to_string();
        assert!(
            root.contains("work tree"),
            "inside .git is neither bare nor a work tree: {root}"
        );
        let root = preflight(repo.path(), None).unwrap();
        assert_eq!(
            root.canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn preflight_checks_the_revision() {
        let repo = Repo::new();
        repo.write("a.txt", "a\n");
        repo.commit("one");
        repo.git(&["branch", "base"]);
        repo.write("a.txt", "b\n");
        repo.commit("two");

        assert!(preflight(repo.path(), Some("base..HEAD")).is_ok());
        assert!(preflight(repo.path(), Some("base")).is_ok());
        let err = |rev| preflight(repo.path(), Some(rev)).unwrap_err().to_string();
        assert_eq!(err("bsae..HEAD"), "bad revision 'bsae..HEAD'");
        assert_eq!(err("HEAD..base"), "no commits in 'HEAD..base'");
        // A file name is not a revision, even though git log would take it as a path.
        assert_eq!(err("a.txt"), "bad revision 'a.txt'");
    }

    #[test]
    fn rev_list_follows_the_revision() {
        let repo = Repo::new();
        for n in ["one", "two", "three"] {
            repo.write("f", n);
            repo.commit(n);
        }
        let shas = |rev| -> Vec<String> {
            rev_list(repo.path(), rev)
                .unwrap()
                .collect::<Result<_>>()
                .unwrap()
        };
        let all = shas("HEAD");
        assert_eq!(all.len(), 3);
        assert_eq!(shas("HEAD~2.."), all[..2]);
        assert_eq!(shas("HEAD~1"), all[1..]);
    }

    #[test]
    fn bare_repository() {
        let repo = Repo::new();
        repo.write("a.txt", "a\n");
        repo.commit("in a bare repo");
        let bare = TempDir::new().unwrap();
        let bare_path = bare.path().join("r.git");
        repo.git(&["clone", "-q", "--bare", ".", bare_path.to_str().unwrap()]);

        let root = preflight(&bare_path, None).unwrap();
        let commits: Vec<_> = log(&root, Source::Log)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(commits[0].subject, "in a bare repo");
        let d = diff(&root, &commits[0].sha, Some(&commits[0].files[0])).unwrap();
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
        let raw = b"\x1esha1\0s1\0Levi\0now\0copy\0\0\0\nC75\0src.rs\0dst.rs\0M\0x\0\x1esha2\0s2\0Erwin\0then\0root\0HEAD@{1}\0commit (initial): root\0";
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
        assert_eq!(commits[0].reflog, None);
        assert_eq!(commits[1].author, "Erwin");
        assert!(commits[1].files.is_empty());
        assert_eq!(
            commits[1].reflog,
            Some(Reflog {
                selector: "HEAD@{1}".into(),
                action: "commit (initial): root".into(),
            })
        );
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

    #[test]
    fn parse_diff_puts_the_message_first() {
        let text = "commit abc\nAuthor: Levi <l@s>\n\n    subject\n\n    body\n\0\ndiff --git a/x b/x\n@@ -1 +1 @@\n-a = 1\n+a = 2\n";
        let lines = parse_diff(text);
        let kinds: Vec<_> = lines.iter().map(|l| l.kind).collect();
        use LineKind::*;
        assert_eq!(
            kinds,
            [
                Meta, Meta, Message, Message, Message, Message, Message, Header, Hunk, Removed,
                Added
            ]
        );
        assert_eq!(lines[3].text, "    subject");
        assert_eq!(lines[6].text, "", "a blank line before the diff");
        let marked: Vec<_> = lines[10]
            .emph
            .iter()
            .map(|r| &lines[10].text[r.clone()])
            .collect();
        assert_eq!(marked, ["2"], "changed words are marked");

        let repo = Repo::new();
        repo.write("a", "a\n");
        repo.commit("first\n\nwith a body");
        let c = &repo.log()[0];
        let lines = parse_diff(&diff(repo.path(), &c.sha, None).unwrap());
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts[0], format!("commit {}", c.sha));
        assert_eq!(texts[1], "Author: Levi <levi@scouts>");
        assert_eq!(texts[3..], ["", "    first", "", "    with a body"]);
    }

    #[test]
    fn reflog_walks_head_movements() {
        let repo = Repo::new();
        repo.write("a", "a\n");
        repo.commit("first");
        repo.git(&["checkout", "-q", "-b", "side"]);
        repo.write("b", "b\n");
        repo.commit("on side");
        repo.git(&["checkout", "-q", "main"]);
        // Deleting the branch leaves its commit reachable only through the reflog.
        repo.git(&["branch", "-q", "-D", "side"]);

        let branch: Vec<_> = log(repo.path(), Source::Log)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(branch.len(), 1);
        assert!(branch[0].reflog.is_none());

        let reflog: Vec<_> = log(repo.path(), Source::Reflog)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        let actions: Vec<_> = reflog
            .iter()
            .map(|c| c.reflog.as_ref().unwrap().action.as_str())
            .collect();
        assert_eq!(actions[0], "checkout: moving from side to main");
        assert!(actions.contains(&"commit: on side"));
        assert_eq!(reflog[0].reflog.as_ref().unwrap().selector, "HEAD@{0}");
        let lost = reflog.iter().find(|c| c.subject == "on side").unwrap();
        assert_eq!(lost.files, vec![change(Status::Added, "b")]);
    }

    fn code_hits(repo: &Repo, pattern: &str) -> Vec<(String, Vec<String>)> {
        let shas: Vec<_> = repo.log().into_iter().map(|c| c.sha).collect();
        code_search(repo.path(), pattern, &shas, None)
            .unwrap()
            .collect()
    }

    #[test]
    fn code_search_is_literal_and_smart_case() {
        let repo = Repo::new();
        repo.write("a.rs", "fn main() {\n    let x = foo(1);\n}\n");
        repo.write("b.txt", "a.b x[0] c\\d {} Upper ^$|+?*\n");
        repo.commit("one");
        repo.write("a.rs", "fn main() {\n    let x = bar(2);\n}\n");
        repo.write("c.txt", "other\n");
        repo.commit("two");
        let log = repo.log();
        let (two, one) = (&log[0].sha, &log[1].sha);

        // Removing a line counts too, so `foo(` hits both commits, newest first.
        let hits = code_hits(&repo, "foo(");
        assert_eq!(
            hits,
            vec![
                (two.clone(), vec!["a.rs".into()]),
                (one.clone(), vec!["a.rs".into()])
            ]
        );
        assert_eq!(
            code_hits(&repo, "bar(2)"),
            vec![(two.clone(), vec!["a.rs".into()])]
        );
        assert_eq!(
            code_hits(&repo, "let x = b").len(),
            1,
            "spaces are part of the text"
        );

        for literal in ["a.b", "x[0]", "c\\d", "{}", "^$|+?*"] {
            assert_eq!(
                code_hits(&repo, literal),
                vec![(one.clone(), vec!["b.txt".into()])],
                "{literal}"
            );
        }
        assert!(code_hits(&repo, "a.c").is_empty(), "`.` isn't a wildcard");

        assert_eq!(code_hits(&repo, "upper").len(), 1, "lowercase ignores case");
        assert!(code_hits(&repo, "UPPER").is_empty(), "uppercase is exact");
        assert!(code_hits(&repo, "nowhere").is_empty());
    }

    #[test]
    fn code_search_only_looks_at_given_commits() {
        let repo = Repo::new();
        repo.write("a", "needle\n");
        repo.commit("one");
        repo.write("a", "needle again\n");
        repo.commit("two");
        let log = repo.log();
        let hits: Vec<_> = code_search(repo.path(), "needle", &[log[1].sha.clone()], None)
            .unwrap()
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, log[1].sha);
    }

    #[test]
    fn code_search_can_diff_only_some_paths() {
        let repo = Repo::new();
        repo.write("a.rs", "needle\n");
        repo.write("b.rs", "needle\n");
        repo.commit("both");
        let shas = vec![repo.log()[0].sha.clone()];
        let only_b: Vec<_> = code_search(repo.path(), "needle", &shas, Some(&["b.rs".into()]))
            .unwrap()
            .collect();
        assert_eq!(only_b, vec![(shas[0].clone(), vec!["b.rs".into()])]);
        let neither: Vec<_> = code_search(repo.path(), "needle", &shas, Some(&["c.rs".into()]))
            .unwrap()
            .collect();
        assert!(neither.is_empty());
    }
}
