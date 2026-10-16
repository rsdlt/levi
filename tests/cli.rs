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

    let out = levi(dir.path(), &["-V"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("levi {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_argument() {
    let dir = TempDir::new().unwrap();
    let out = levi(dir.path(), &["--all"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("unexpected argument '--all'"));
}
