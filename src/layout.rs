//! Where the commits, files and diff panes go, and the seams between them
//! that a mouse drag moves.

use ratatui::layout::{Position, Rect};

/// Below this many columns, three side-by-side panes leave the diff too
/// narrow to read, so the lists go above it instead.
pub const STACK_BELOW: u16 = 120;

/// No drag makes a pane narrower or shorter than this.
const MIN_WIDTH: u16 = 12;
const MIN_HEIGHT: u16 = 5;

/// Where the seams sit, as fractions of the body, so a terminal resize keeps
/// the proportions. Each layout keeps its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Splits {
    /// Side by side: where the commits end and where the files end, across.
    wide: [f32; 2],
    /// Stacked: where the lists end, down; where the commits end, across.
    stacked: [f32; 2],
}

const WIDE: [f32; 2] = [0.25, 0.45];
const STACKED: [f32; 2] = [0.35, 0.55];

impl Default for Splits {
    fn default() -> Splits {
        Splits {
            wide: WIDE,
            stacked: STACKED,
        }
    }
}

/// A place where two panes meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seam {
    /// Side by side: commits | files.
    CommitsFiles,
    /// Side by side: files | diff.
    FilesDiff,
    /// Stacked: the lists above the diff.
    Lists,
    /// Stacked: commits | files, above the diff.
    StackedCommitsFiles,
}

/// A seam being dragged. `grab` is how far the seam's first cell, the start
/// of the pane after it, is from where the press landed, so the seam doesn't
/// jump when the drag starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resize {
    pub seam: Seam,
    grab: u16,
}

fn stacked(body: Rect) -> bool {
    body.width < STACK_BELOW
}

/// `fraction` of `len` cells, kept between `lo` and `hi` when they leave room.
fn cut(len: u16, fraction: f32, lo: u16, hi: u16) -> u16 {
    let at = ((f32::from(len) * fraction).round() as u16).min(len);
    if lo > hi { at } else { at.clamp(lo, hi) }
}

impl Splits {
    /// The commits, files and diff panes.
    pub fn areas(&self, body: Rect) -> [Rect; 3] {
        let Rect {
            x,
            y,
            width,
            height,
        } = body;
        if stacked(body) {
            let lists = cut(
                height,
                self.stacked[0],
                MIN_HEIGHT,
                height.saturating_sub(MIN_HEIGHT),
            );
            let commits = cut(
                width,
                self.stacked[1],
                MIN_WIDTH,
                width.saturating_sub(MIN_WIDTH),
            );
            [
                Rect::new(x, y, commits, lists),
                Rect::new(x + commits, y, width - commits, lists),
                Rect::new(x, y + lists, width, height - lists),
            ]
        } else {
            let last = width.saturating_sub(MIN_WIDTH);
            let commits = cut(
                width,
                self.wide[0],
                MIN_WIDTH,
                last.saturating_sub(MIN_WIDTH),
            );
            let files = cut(width, self.wide[1], commits + MIN_WIDTH, last).max(commits);
            [
                Rect::new(x, y, commits, height),
                Rect::new(x + commits, y, files - commits, height),
                Rect::new(x + files, y, width - files, height),
            ]
        }
    }

    /// The seam under the pointer, if any: the two border cells where two of
    /// `panes` (as `areas` laid them out) meet.
    pub fn seam_at(body: Rect, panes: [Rect; 3], at: Position) -> Option<Resize> {
        let [_, files, diff] = panes;
        let across = |pane: Rect, seam| {
            let rows = pane.y..pane.bottom();
            (rows.contains(&at.y) && (at.x == pane.x || at.x + 1 == pane.x)).then(|| Resize {
                seam,
                grab: pane.x - at.x,
            })
        };
        if stacked(body) {
            let down = (at.y == diff.y || at.y + 1 == diff.y).then(|| Resize {
                seam: Seam::Lists,
                grab: diff.y - at.y,
            });
            across(files, Seam::StackedCommitsFiles).or(down)
        } else {
            across(files, Seam::CommitsFiles).or(across(diff, Seam::FilesDiff))
        }
        .filter(|_| body.contains(at))
    }

    /// Moves the dragged seam to the pointer, short of the floors.
    pub fn drag(&mut self, body: Rect, panes: [Rect; 3], resize: Resize, at: Position) {
        let [commits, files, _] = panes;
        let column = (at.x + resize.grab).saturating_sub(body.x);
        let row = (at.y + resize.grab).saturating_sub(body.y);
        let fraction = |cell: u16, from: u16, to: u16, len: u16| {
            if from > to {
                return None;
            }
            Some(f32::from(cell.clamp(from, to)) / f32::from(len))
        };
        let (slot, value) = match resize.seam {
            Seam::CommitsFiles => {
                let end = files.right() - body.x - MIN_WIDTH;
                (
                    &mut self.wide[0],
                    fraction(column, MIN_WIDTH, end, body.width),
                )
            }
            Seam::FilesDiff => {
                let start = commits.right() - body.x + MIN_WIDTH;
                let end = body.width - MIN_WIDTH;
                (&mut self.wide[1], fraction(column, start, end, body.width))
            }
            Seam::Lists => {
                let end = body.height.saturating_sub(MIN_HEIGHT);
                (
                    &mut self.stacked[0],
                    fraction(row, MIN_HEIGHT, end, body.height),
                )
            }
            Seam::StackedCommitsFiles => {
                let end = body.width.saturating_sub(MIN_WIDTH);
                (
                    &mut self.stacked[1],
                    fraction(column, MIN_WIDTH, end, body.width),
                )
            }
        };
        if let Some(value) = value {
            *slot = value;
        }
    }

    /// Puts the layout `body` shows back to its defaults.
    pub fn reset(&mut self, body: Rect) {
        if stacked(body) {
            self.stacked = STACKED;
        } else {
            self.wide = WIDE;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE_BODY: Rect = Rect::new(0, 3, 200, 40);
    const NARROW_BODY: Rect = Rect::new(0, 3, 100, 40);

    fn widths(panes: [Rect; 3]) -> [u16; 3] {
        panes.map(|p| p.width)
    }

    #[test]
    fn the_defaults_match_the_plan() {
        let splits = Splits::default();
        assert_eq!(widths(splits.areas(WIDE_BODY)), [50, 40, 110]);
        let [commits, files, diff] = splits.areas(NARROW_BODY);
        assert_eq!((commits.width, files.width, commits.height), (55, 45, 14));
        assert_eq!((diff.y, diff.width, diff.height), (17, 100, 26));
    }

    #[test]
    fn a_seam_is_the_two_borders_where_panes_meet() {
        let panes = Splits::default().areas(WIDE_BODY);
        let at = |x, y| Splits::seam_at(WIDE_BODY, panes, Position::new(x, y));
        assert_eq!(at(49, 10).map(|r| r.seam), Some(Seam::CommitsFiles));
        assert_eq!(at(50, 10).map(|r| r.seam), Some(Seam::CommitsFiles));
        assert_eq!(at(89, 10).map(|r| r.seam), Some(Seam::FilesDiff));
        assert_eq!(at(90, 42).map(|r| r.seam), Some(Seam::FilesDiff));
        for (x, y) in [(48, 10), (51, 10), (0, 10), (199, 10), (50, 2), (50, 43)] {
            assert_eq!(at(x, y), None, "({x}, {y})");
        }

        let panes = Splits::default().areas(NARROW_BODY);
        let at = |x, y| Splits::seam_at(NARROW_BODY, panes, Position::new(x, y));
        assert_eq!(at(55, 10).map(|r| r.seam), Some(Seam::StackedCommitsFiles));
        assert_eq!(at(55, 20), None, "below the lists, the diff is whole");
        assert_eq!(at(30, 16).map(|r| r.seam), Some(Seam::Lists));
        assert_eq!(at(30, 17).map(|r| r.seam), Some(Seam::Lists));
    }

    fn drag(splits: &mut Splits, body: Rect, from: (u16, u16), to: (u16, u16)) {
        let panes = splits.areas(body);
        let resize = Splits::seam_at(body, panes, Position::new(from.0, from.1)).unwrap();
        splits.drag(body, panes, resize, Position::new(to.0, to.1));
    }

    #[test]
    fn dragging_moves_only_the_panes_beside_the_seam() {
        let mut splits = Splits::default();
        // Grabbed by the commits' right border, which lands where it's let go.
        drag(&mut splits, WIDE_BODY, (49, 10), (69, 10));
        assert_eq!(widths(splits.areas(WIDE_BODY)), [70, 20, 110]);
        // Grabbed by the diff's left border.
        drag(&mut splits, WIDE_BODY, (90, 10), (120, 10));
        assert_eq!(widths(splits.areas(WIDE_BODY)), [70, 50, 80]);

        drag(&mut splits, NARROW_BODY, (30, 17), (30, 30));
        let [commits, files, diff] = splits.areas(NARROW_BODY);
        assert_eq!(
            (commits.height, files.height, diff.y, diff.height),
            (27, 27, 30, 13)
        );
        drag(&mut splits, NARROW_BODY, (54, 10), (19, 10));
        assert_eq!(widths(splits.areas(NARROW_BODY)), [20, 80, 100]);
        // The wide layout kept its own.
        assert_eq!(widths(splits.areas(WIDE_BODY)), [70, 50, 80]);
    }

    #[test]
    fn seams_stop_at_the_floors() {
        let mut splits = Splits::default();
        drag(&mut splits, WIDE_BODY, (50, 10), (0, 10));
        assert_eq!(widths(splits.areas(WIDE_BODY)), [12, 78, 110]);
        drag(&mut splits, WIDE_BODY, (12, 10), (199, 10));
        assert_eq!(widths(splits.areas(WIDE_BODY)), [78, 12, 110]);
        drag(&mut splits, WIDE_BODY, (90, 10), (199, 10));
        assert_eq!(widths(splits.areas(WIDE_BODY)), [78, 110, 12]);

        drag(&mut splits, NARROW_BODY, (30, 17), (30, 0));
        assert_eq!(splits.areas(NARROW_BODY)[0].height, 5);
        drag(&mut splits, NARROW_BODY, (30, 8), (30, 99));
        assert_eq!(splits.areas(NARROW_BODY)[2].height, 5);
    }

    #[test]
    fn proportions_survive_a_terminal_resize() {
        let mut splits = Splits::default();
        drag(&mut splits, WIDE_BODY, (50, 10), (70, 10));
        assert_eq!(widths(splits.areas(WIDE_BODY)), [70, 20, 110]);
        let double = Rect::new(0, 3, 400, 40);
        assert_eq!(widths(splits.areas(double)), [140, 40, 220]);
        // A body too small for the floors still gets laid out.
        let tiny = Rect::new(0, 3, 20, 4);
        let [commits, files, diff] = splits.areas(tiny);
        assert_eq!(
            (commits.width + files.width, commits.height + diff.height),
            (20, 4)
        );
    }

    #[test]
    fn reset_puts_back_only_the_layout_shown() {
        let mut splits = Splits::default();
        drag(&mut splits, WIDE_BODY, (50, 10), (70, 10));
        drag(&mut splits, NARROW_BODY, (30, 17), (30, 30));
        splits.reset(WIDE_BODY);
        assert_eq!(widths(splits.areas(WIDE_BODY)), [50, 40, 110]);
        assert_ne!(splits, Splits::default());
        splits.reset(NARROW_BODY);
        assert_eq!(splits, Splits::default());
    }
}
