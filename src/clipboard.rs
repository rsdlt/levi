//! Copying to the system clipboard, two ways at once: through the terminal with
//! OSC 52, which also works over SSH, and through the platform's own tool, for
//! terminals without OSC 52 such as macOS Terminal.app.

use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::thread;

/// Copies `text`. Neither way reports back, so this can't fail; at worst the
/// clipboard is left as it was.
pub fn copy(text: &str) {
    // stderr is the terminal levi draws on.
    let mut terminal = io::stderr();
    let _ = write!(terminal, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = terminal.flush();

    let text = text.to_owned();
    thread::spawn(move || {
        for tool in tools() {
            if pipe(tool, &text) {
                break;
            }
        }
    });
}

/// Clipboard commands to try, in order, until one works.
fn tools() -> &'static [&'static [&'static str]] {
    if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else if cfg!(windows) {
        // Windows Terminal understands OSC 52.
        &[]
    } else {
        &[
            &["wl-copy"],
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
        ]
    }
}

/// Runs `cmd` with `text` on its stdin; true if it succeeded. Its output is
/// discarded so nothing lands on levi's screen.
fn pipe(cmd: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // Dropping stdin closes it, so the tool sees the end of the text.
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|status| status.success()) && wrote
}

/// Standard base64 with padding, which OSC 52 expects.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        // The test vectors from RFC 4648.
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (text, encoded) in cases {
            assert_eq!(base64(text.as_bytes()), encoded, "{text:?}");
        }
        assert_eq!(base64("ñ→".as_bytes()), "w7HihpI=");
    }
}
