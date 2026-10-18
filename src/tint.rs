//! Soft backgrounds for added and removed lines, mixed from the terminal's own
//! background so they suit any theme: a faint green or red on dark, a pale one
//! on light. They need the background's exact color, which the terminal is
//! asked for, and truecolor to show it; without either, there are none.

use ratatui::style::Color;

/// How much green or red goes into a changed line's background, and into the
/// background of the words that changed in it.
const LINE: f32 = 0.13;
const WORDS: f32 = 0.32;

/// The hues mixed in: a calm green and red, not the terminal's loud ones.
const GREEN: Rgb = (46, 160, 67);
const RED: Rgb = (218, 54, 51);

type Rgb = (u8, u8, u8);

/// The backgrounds for changed lines and for the changed words in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tints {
    pub added: Color,
    pub removed: Color,
    pub added_words: Color,
    pub removed_words: Color,
}

impl Tints {
    pub fn over(bg: (u8, u8, u8)) -> Tints {
        let color = |hue, amount| {
            let (r, g, b) = mix(bg, hue, amount);
            Color::Rgb(r, g, b)
        };
        Tints {
            added: color(GREEN, LINE),
            removed: color(RED, LINE),
            added_words: color(GREEN, WORDS),
            removed_words: color(RED, WORDS),
        }
    }
}

/// `amount` of `hue` over `bg`.
fn mix(bg: Rgb, hue: Rgb, amount: f32) -> Rgb {
    let channel =
        |b: u8, h: u8| (f32::from(b) + (f32::from(h) - f32::from(b)) * amount).round() as u8;
    (
        channel(bg.0, hue.0),
        channel(bg.1, hue.1),
        channel(bg.2, hue.2),
    )
}

/// Tints for this terminal, if it shows truecolor and says what its
/// background is. Call it with raw mode on and before anything else reads
/// input, since the answer comes back as input.
pub fn detect() -> Option<Tints> {
    let truecolor = std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit");
    if !truecolor {
        return None;
    }
    query::background().map(Tints::over)
}

/// The color in an OSC 11 reply, `ESC ] 11 ; rgb:RRRR/GGGG/BBBB` ended by BEL
/// or `ESC \`, with 1 to 4 hex digits per channel.
fn parse_reply(reply: &[u8]) -> Option<Rgb> {
    let text = std::str::from_utf8(reply).ok()?;
    let start = text.find("\x1b]11;rgb:")? + "\x1b]11;rgb:".len();
    let rest = &text[start..];
    let end = rest.find(['\x07', '\x1b'])?;
    let mut parts = rest[..end].split('/').map(|hex| {
        let max = 16u32.checked_pow(u32::try_from(hex.len()).ok()?)? - 1;
        let value = u32::from_str_radix(hex, 16).ok()?;
        (!hex.is_empty() && hex.len() <= 4).then(|| (value * 255 / max) as u8)
    });
    let rgb = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(rgb)
}

#[cfg(unix)]
mod query {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    use super::{Rgb, parse_reply};

    /// The longest levi waits for an answer.
    const PATIENCE: Duration = Duration::from_millis(100);

    /// Asks for the background (OSC 11), then for the device attributes (DA1),
    /// which every terminal answers: once that answer is in, whatever was going
    /// to answer the first question has. The questions go out on stderr, which
    /// is the terminal levi draws on, never on stdout, which in `$(levi)` is
    /// the sha's pipe.
    pub fn background() -> Option<Rgb> {
        let mut tty = File::open("/dev/tty").ok()?;
        let mut err = std::io::stderr();
        err.write_all(b"\x1b]11;?\x1b\\\x1b[c").ok()?;
        err.flush().ok()?;

        let deadline = Instant::now() + PATIENCE;
        let mut reply = Vec::new();
        let mut buf = [0u8; 256];
        while !answered(&reply) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !readable(&tty, left) {
                break;
            }
            match tty.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => reply.extend_from_slice(&buf[..n]),
            }
        }
        parse_reply(&reply)
    }

    /// Whether the device attributes, the last answer asked for, are in:
    /// `ESC [ ? … c`.
    fn answered(reply: &[u8]) -> bool {
        reply
            .windows(3)
            .position(|w| w == b"\x1b[?")
            .is_some_and(|at| reply[at..].contains(&b'c'))
    }

    /// Waits up to `wait` for something to read. `select`, not `poll`: macOS's
    /// `poll` doesn't support terminals and claims they're readable at once,
    /// so the read would block until a key came and take it.
    fn readable(tty: &File, wait: Duration) -> bool {
        let fd = tty.as_raw_fd();
        let mut timeout = libc::timeval {
            tv_sec: wait.as_secs() as libc::time_t,
            tv_usec: wait.subsec_micros() as libc::suseconds_t,
        };
        // SAFETY: an fd_set zeroed and filled by its own macros, for a file
        // that stays open during the call, which is below FD_SETSIZE.
        unsafe {
            if fd >= libc::FD_SETSIZE as i32 {
                return false;
            }
            let mut set: libc::fd_set = std::mem::zeroed();
            libc::FD_ZERO(&mut set);
            libc::FD_SET(fd, &mut set);
            let ready = libc::select(
                fd + 1,
                &mut set,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut timeout,
            );
            ready > 0 && libc::FD_ISSET(fd, &set)
        }
    }
}

#[cfg(not(unix))]
mod query {
    pub fn background() -> Option<super::Rgb> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_background_from_the_reply() {
        let st = b"\x1b]11;rgb:1a1a/1b1b/2626\x1b\\\x1b[?62;22c";
        assert_eq!(parse_reply(st), Some((0x1a, 0x1b, 0x26)));
        let bel = b"\x1b]11;rgb:ffff/ffff/ffff\x07";
        assert_eq!(parse_reply(bel), Some((255, 255, 255)));
        // Two digits per channel, and keys typed meanwhile in front.
        assert_eq!(
            parse_reply(b"jk\x1b]11;rgb:ef/f1/f5\x07"),
            Some((0xef, 0xf1, 0xf5))
        );
        // No answer but the device attributes: a terminal that doesn't say.
        assert_eq!(parse_reply(b"\x1b[?1;2c"), None);
        assert_eq!(parse_reply(b"\x1b]11;rgb:zz/00/00\x07"), None);
    }

    #[test]
    fn tints_lean_toward_the_background() {
        let dark = Tints::over((0x1a, 0x1b, 0x26));
        let light = Tints::over((0xef, 0xf1, 0xf5));
        let rgb = |c| match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("not truecolor"),
        };
        // On dark, a dark green; on light, a pale one: green leads either way.
        let (r, g, b) = rgb(dark.added);
        assert!(g > r && g > b && g < 0x50, "{:?}", (r, g, b));
        let (r, g, b) = rgb(light.added);
        assert!(g > r && g > b && r > 0xc0, "{:?}", (r, g, b));
        let (r, g, b) = rgb(dark.removed);
        assert!(r > g && r > b && r < 0x50, "{:?}", (r, g, b));
        // Changed words stand out more than their line.
        assert!(rgb(dark.added_words).1 > rgb(dark.added).1);
        assert!(rgb(light.removed_words).1 < rgb(light.removed).1);
    }
}
