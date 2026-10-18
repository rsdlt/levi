//! Opening a file at a line: in the Neovim levi runs inside, when it does, or
//! else in `$VISUAL`/`$EDITOR`, which takes over the terminal until it quits.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::app::OpenRequest;
use crate::git;

/// The Neovim whose terminal levi runs in, if any: it sets `$NVIM` there.
pub fn host_nvim() -> Option<String> {
    std::env::var("NVIM").ok().filter(|s| !s.is_empty())
}

/// Opens `file` at `line` in a new tab of the Neovim at `server`, unmodifiable
/// if `read_only`. levi keeps running in its own tab.
pub fn open_in_nvim(server: &str, file: &Path, line: usize, read_only: bool) -> Result<()> {
    let file = file.to_str().context("the path isn't valid UTF-8")?;
    let lock = if read_only {
        " .. ' | setlocal nomodifiable'"
    } else {
        ""
    };
    let expr = format!(
        "execute('tabedit +{line} ' .. fnameescape({}){lock})",
        vim_string(file)
    );
    let out = Command::new("nvim")
        .args(["--server", server, "--remote-expr", &expr])
        .stdin(Stdio::null())
        .output()
        .context("couldn't run nvim")?;
    if !out.status.success() {
        bail!("nvim: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Writes the file `req` asks for, as git has it in history, to a read-only
/// temporary copy: the commit's version, or its parent's for a file the commit
/// deletes. The copy keeps the file's name, so editors still recognize the
/// language, under a directory named for the commit.
pub fn past_copy(root: &Path, req: &OpenRequest) -> Result<PathBuf> {
    // The full sha: a short one could collide between repositories.
    let at = if req.deleted { "-parent" } else { "" };
    let dir = std::env::temp_dir().join(format!("levi-{}{at}", req.sha));
    let copy = dir.join(&req.path);
    // The same file from the same commit is the same content.
    if copy.is_file() {
        return Ok(copy);
    }
    let content = git::file_at(root, &req.sha, &req.path, req.deleted).with_context(|| {
        format!(
            "{} isn't in the working tree, and git couldn't show it",
            req.path
        )
    })?;
    if let Some(parent) = copy.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&copy, content)?;
    let mut perms = std::fs::metadata(&copy)?.permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&copy, perms)?;
    Ok(copy)
}

/// Deletes a copy from `past_copy`, and the directories that only held it,
/// quietly: it's only a temporary file.
pub fn remove_copy(copy: &Path) {
    if let Ok(meta) = std::fs::metadata(copy) {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(copy, perms);
    }
    let _ = std::fs::remove_file(copy);
    let temp = std::env::temp_dir();
    // `remove_dir` only removes empty directories, so this stops at the
    // first one still holding something, and never goes past the temp dir.
    for dir in copy.ancestors().skip(1) {
        if !dir.starts_with(&temp) || dir == temp || std::fs::remove_dir(dir).is_err() {
            break;
        }
    }
}

/// A Vim single-quoted string, in which only `'` needs escaping (doubled).
fn vim_string(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The editor to run: `$VISUAL`, then `$EDITOR`, then `vi`.
pub fn editor() -> String {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "vi".to_owned())
}

/// The command that opens `file` at `line` in `editor`, run in `dir`. Like git,
/// it goes through `sh`, so `$EDITOR` may carry arguments (`code --wait`).
pub fn command(editor: &str, dir: &Path, file: &str, line: usize) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut words = editor.split_whitespace();
        let mut cmd = Command::new(words.next().unwrap_or("notepad"));
        cmd.args(words);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!("{editor} \"$@\"")).arg("sh");
        cmd
    };
    cmd.args(args(editor, file, line)).current_dir(dir);
    cmd
}

/// Points the editor at the terminal. levi's own stdout is often a pipe, as in
/// `git show $(levi)`, where the editor would draw into the sha's place; it
/// draws where levi does, on stderr, and reads keys from the terminal too.
pub fn on_terminal(cmd: &mut Command) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        use std::os::fd::AsFd;
        cmd.stdout(io::stderr().as_fd().try_clone_to_owned()?);
        if !io::stdin().is_terminal() {
            cmd.stdin(std::fs::File::open("/dev/tty")?);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        cmd.stdout(io::stderr().as_handle().try_clone_to_owned()?);
    }
    Ok(())
}

/// How `editor` takes a line: `+12 file` for vi, Neovim, Emacs, nano, micro,
/// Kakoune and most others; `file:12` for a few.
fn args(editor: &str, file: &str, line: usize) -> Vec<String> {
    let program = editor.split_whitespace().next().unwrap_or_default();
    let name = Path::new(program).file_name().and_then(|n| n.to_str());
    match name.unwrap_or_default() {
        "hx" | "helix" | "zed" | "subl" => vec![format!("{file}:{line}")],
        "code" | "codium" | "cursor" => vec!["--goto".into(), format!("{file}:{line}")],
        _ => vec![format!("+{line}"), file.to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_get_the_line_their_way() {
        assert_eq!(args("nvim", "src/a.rs", 12), ["+12", "src/a.rs"]);
        assert_eq!(args("/usr/bin/vim -p", "a.rs", 3), ["+3", "a.rs"]);
        assert_eq!(args("hx", "a.rs", 3), ["a.rs:3"]);
        assert_eq!(args("code --wait", "a.rs", 3), ["--goto", "a.rs:3"]);
    }

    #[test]
    fn past_copies_come_from_history_read_only() {
        let repo = crate::testutil::Repo::new();
        repo.write("src/ghost.rs", "one\ntwo\n");
        repo.commit("add ghost");
        repo.git(&["rm", "-q", "src/ghost.rs"]);
        repo.commit("remove ghost");
        let log = repo.log();
        let (removed, added) = (&log[0], &log[1]);
        let req = |commit: &crate::git::Commit, deleted| OpenRequest {
            sha: commit.sha.clone(),
            short: commit.short.clone(),
            path: "src/ghost.rs".into(),
            line: 2,
            deleted,
        };

        // As of the commit that added it...
        let copy = past_copy(repo.path(), &req(added, false)).unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "one\ntwo\n");
        assert!(copy.ends_with("src/ghost.rs"), "keeps its name: {copy:?}");
        assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());
        remove_copy(&copy);
        let dir = copy.parent().unwrap().parent().unwrap();
        assert!(
            dir.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("levi-")
        );
        assert!(!dir.exists(), "its directories go too");

        // ...and, from the commit that deleted it, as it was just before.
        let copy = past_copy(repo.path(), &req(removed, true)).unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "one\ntwo\n");
        remove_copy(&copy);

        // The commit that deleted it has no such file to show.
        let err = past_copy(repo.path(), &req(removed, false)).unwrap_err();
        assert!(format!("{err:#}").starts_with("src/ghost.rs isn't in the working tree"));
    }

    #[test]
    fn vim_strings_double_their_quotes() {
        assert_eq!(vim_string("src/a.rs"), "'src/a.rs'");
        assert_eq!(vim_string("it's.rs"), "'it''s.rs'");
    }

    #[test]
    fn the_command_goes_through_sh_with_the_editors_own_arguments() {
        if cfg!(windows) {
            return;
        }
        let cmd = command("code --wait", Path::new("/repo"), "a.rs", 3);
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args, ["-c", "code --wait \"$@\"", "sh", "--goto", "a.rs:3"]);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/repo")));
    }
}
