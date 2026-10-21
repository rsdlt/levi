//! What a running levi has on screen, for `levi --focus` to print: a note
//! naming the commit, file and lines, the same reference `--pick-context`
//! prints on Enter. It lives in the temporary directory, one per repository,
//! never in the repository itself.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the note for the repository at `root` goes. The name hashes the path,
/// so worktrees of one repository each get their own.
fn path(root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
    std::env::temp_dir().join(format!("levi-focus-{:016x}", fnv(&root)))
}

/// FNV-1a: a stable hash, unlike std's, so levis of different builds agree.
fn fnv(path: &Path) -> u64 {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
}

/// Replaces the note with `reference`, signed with this process's id. The
/// last levi to write wins, which is the one being used.
pub fn write(root: &Path, reference: &str) {
    let path = path(root);
    let tmp = path.with_extension(std::process::id().to_string());
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    // A note that can't be written only costs `--focus` its answer.
    let written = options.open(&tmp).and_then(|mut f| {
        writeln!(f, "{}", std::process::id())?;
        writeln!(f, "{reference}")
    });
    // A rename, so `--focus` never reads half a note.
    if written.is_err() || fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// Removes the note, if it's still this levi's.
pub fn remove(root: &Path) {
    let path = path(root);
    if fs::read_to_string(&path).is_ok_and(|note| owner(&note) == Some(std::process::id())) {
        let _ = fs::remove_file(path);
    }
}

/// The reference a running levi left for `root`. A note whose levi is gone
/// (killed, or its terminal closed) doesn't count.
pub fn read(root: &Path) -> Option<String> {
    let note = fs::read_to_string(path(root)).ok()?;
    if !alive(owner(&note)?) {
        return None;
    }
    let reference = note.lines().nth(1)?;
    Some(reference.to_owned())
}

fn owner(note: &str) -> Option<u32> {
    note.lines().next()?.parse().ok()
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // Signal 0 only asks whether the process exists. EPERM: it does, as
    // someone else's.
    let found = unsafe { libc::kill(pid, 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_reads_back_until_removed() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(read(dir.path()), None);
        write(
            dir.path(),
            "commit d3a93ab (\"Stack\"), src/ui.rs lines 27-40",
        );
        assert_eq!(
            read(dir.path()).as_deref(),
            Some("commit d3a93ab (\"Stack\"), src/ui.rs lines 27-40")
        );
        write(dir.path(), "commit 9f8e7d6 (\"Resize\")");
        assert_eq!(
            read(dir.path()).as_deref(),
            Some("commit 9f8e7d6 (\"Resize\")")
        );
        remove(dir.path());
        assert_eq!(read(dir.path()), None);
    }

    #[test]
    fn each_repository_has_its_own_note() {
        let (a, b) = (
            tempfile::TempDir::new().unwrap(),
            tempfile::TempDir::new().unwrap(),
        );
        write(a.path(), "commit aaaaaaa (\"A\")");
        assert_eq!(read(b.path()), None);
        remove(a.path());
    }

    #[cfg(unix)]
    #[test]
    fn another_levis_note_stays_until_it_is_gone() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = path(dir.path());
        // A levi that's still running: this test's parent, say.
        let parent = std::os::unix::process::parent_id();
        fs::write(&path, format!("{parent}\ncommit 1234567 (\"Other\")\n")).unwrap();
        remove(dir.path());
        assert_eq!(
            read(dir.path()).as_deref(),
            Some("commit 1234567 (\"Other\")")
        );
        // A levi that's gone: no process has an id this high.
        fs::write(&path, "4000000000\ncommit 1234567 (\"Other\")\n").unwrap();
        assert_eq!(read(dir.path()), None);
        fs::remove_file(path).unwrap();
    }
}
