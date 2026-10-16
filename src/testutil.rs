//! Throwaway git repositories for tests.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

use crate::git::{Commit, log};
use crate::query::Source;

pub struct Repo {
    dir: TempDir,
}

impl Repo {
    pub fn new() -> Repo {
        let repo = Repo {
            dir: TempDir::new().unwrap(),
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(self.path())
            .args(["-c", "user.name=Levi", "-c", "user.email=levi@scouts"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    pub fn write(&self, path: &str, content: &str) {
        let full = self.path().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    pub fn commit(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
    }

    pub fn log(&self) -> Vec<Commit> {
        log(self.path(), Source::Log)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap()
    }
}
