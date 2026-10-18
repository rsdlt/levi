//! Opening a file at a line: in the Neovim levi runs inside, when it does, or
//! else in `$VISUAL`/`$EDITOR`, which takes over the terminal until it quits.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// The Neovim whose terminal levi runs in, if any: it sets `$NVIM` there.
pub fn host_nvim() -> Option<String> {
    std::env::var("NVIM").ok().filter(|s| !s.is_empty())
}

/// Opens `file` at `line` in a new tab of the Neovim at `server`. levi keeps
/// running in its own tab.
pub fn open_in_nvim(server: &str, file: &Path, line: usize) -> Result<()> {
    let file = file.to_str().context("the path isn't valid UTF-8")?;
    let expr = format!(
        "execute('tabedit +{line} ' .. fnameescape({}))",
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
