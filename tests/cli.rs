//! Runs the real binary for everything that happens before the TUI starts.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn levi(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_levi"))
        .current_dir(dir)
        .args(args)
        // Never let git wander up into a repo that happens to contain the temp dir.
        .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
        .output()
        .unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn outside_a_repository() {
    let dir = TempDir::new().unwrap();
    let out = levi(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "levi: not inside a git repository\n");
    assert!(out.stdout.is_empty());
}

#[test]
fn repository_without_commits() {
    let dir = TempDir::new().unwrap();
    Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(dir.path())
        .status()
        .unwrap();
    let out = levi(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "levi: no commits yet\n");
}

#[test]
fn git_missing() {
    let dir = TempDir::new().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_levi"))
        .current_dir(dir.path())
        .env("PATH", dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "levi: git not found in PATH\n");
}

#[test]
fn help_and_version() {
    let dir = TempDir::new().unwrap();
    let out = levi(dir.path(), &["--help"]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.starts_with("levi — "));
    assert!(help.contains("Enter"));

    for flag in ["-V", "-v", "--version"] {
        let out = levi(dir.path(), &[flag]);
        assert!(out.status.success(), "{flag}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("levi {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn unknown_argument() {
    let dir = TempDir::new().unwrap();
    let out = levi(dir.path(), &["--all"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("unexpected argument '--all'"));
}

fn repo_with_two_commits() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "one"]);
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "two"]);
    dir
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Levi", "-c", "user.email=levi@scouts"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

/// stdout's lines, each without its 40-char sha, after checking it's there.
fn subjects(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| {
            let (sha, subject) = line.split_once(' ').unwrap();
            assert!(sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()));
            subject.to_owned()
        })
        .collect()
}

#[test]
fn filter_prints_what_the_search_finds() {
    // No terminal here, which --filter doesn't need.
    let dir = repo_with_two_commits();
    let out = levi(dir.path(), &["--filter", "two"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(subjects(&out), ["two"]);
    assert_eq!(subjects(&levi(dir.path(), &["-f", ""])), ["two", "one"]);
    assert_eq!(
        subjects(&levi(dir.path(), &["--filter=", "HEAD~1"])),
        ["one"]
    );

    // /c searches the diffs, as in the search bar.
    std::fs::write(dir.path().join("wall.rs"), "let titan = 1;\n").unwrap();
    git(dir.path(), &["add", "wall.rs"]);
    git(dir.path(), &["commit", "-q", "-m", "three"]);
    assert_eq!(subjects(&levi(dir.path(), &["-f", "/c titan"])), ["three"]);
    assert_eq!(
        subjects(&levi(dir.path(), &["-f", "wall /c titan"])),
        ["three"]
    );

    // Nothing found: exit 1, like grep; errors exit 2.
    let out = levi(dir.path(), &["-f", "/c colossal"]);
    assert_eq!((out.status.code(), out.stdout.len()), (Some(1), 0));
    let out = levi(dir.path(), &["--filter"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("'--filter' needs a query"));
}

#[test]
fn bad_revision() {
    let dir = repo_with_two_commits();
    let out = levi(dir.path(), &["mian..HEAD"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "levi: bad revision 'mian..HEAD'\n");

    let out = levi(dir.path(), &["HEAD..HEAD~1"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "levi: no commits in 'HEAD..HEAD~1'\n");
}

#[test]
fn without_a_terminal() {
    // `levi` runs here with stderr piped, like in a script or `2>file`.
    let dir = repo_with_two_commits();
    let out = levi(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "levi: needs a terminal (it draws on stderr, which isn't one)\n"
    );
}

#[test]
fn one_revision_at_most() {
    let dir = repo_with_two_commits();
    let out = levi(dir.path(), &["main", "src/"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("unexpected argument 'src/'"));
}
