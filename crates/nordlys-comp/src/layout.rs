//! Where the stage's windows go on a screen. Pure arithmetic in logical
//! pixels, so it can be tested without a display.

use nordlys_config::Layout;

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Shrink by `by` on every side, never below 1×1.
    pub fn inset(self, by: i32) -> Self {
        Self {
            x: self.x + by,
            y: self.y + by,
            w: (self.w - 2 * by).max(1),
            h: (self.h - 2 * by).max(1),
        }
    }

    /// A `w`×`h` rectangle centred in this one, clipped to it.
    pub fn centred(self, w: i32, h: i32) -> Self {
        let (w, h) = (w.min(self.w).max(1), h.min(self.h).max(1));
        Self {
            x: self.x + (self.w - w) / 2,
            y: self.y + (self.h - h) / 2,
            w,
            h,
        }
    }
}

/// Space between windows, and between windows and the edge.
pub const GAP: i32 = 12;

/// On an ultrawide screen a lone window doesn't stretch edge to edge: it
/// takes this share of the width, in the centre.
const ULTRAWIDE_SOLO: f32 = 0.62;

/// The main and (in two-up) side window's frames inside `area`, which is
/// the output minus whatever the bar and rails reserve.
pub fn arrange(area: Rect, layout: Layout, split: f32, two_up: bool) -> (Rect, Option<Rect>) {
    let inner = area.inset(GAP);
    let layout = layout.for_width(area.w.max(0) as u32);
    if !two_up {
        let main = match layout {
            Layout::Ultrawide => {
                inner.centred((inner.w as f32 * ULTRAWIDE_SOLO).round() as i32, inner.h)
            }
            _ => inner,
        };
        return (main, None);
    }
    let split = split.clamp(0.2, 0.8);
    let main_w = ((inner.w - GAP) as f32 * split).round() as i32;
    let side_w = inner.w - GAP - main_w;
    let main = Rect::new(inner.x, inner.y, main_w.max(1), inner.h);
    let side = Rect::new(inner.x + main_w + GAP, inner.y, side_w.max(1), inner.h);
    (main, Some(side))
}

/// A dialog's frame: its own size, centred over the stage.
pub fn float(area: Rect, w: i32, h: i32) -> Rect {
    let w = if w > 0 { w } else { area.w / 2 };
    let h = if h > 0 { h } else { area.h / 2 };
    area.inset(GAP).centred(w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Rect = Rect {
        x: 0,
        y: 0,
        w: 1280,
        h: 800,
    };

    #[test]
    fn a_lone_window_fills_the_stage_inside_the_gap() {
        let (main, side) = arrange(LAPTOP, Layout::Auto, 0.6, false);
        assert_eq!(main, Rect::new(12, 12, 1256, 776));
        assert_eq!(side, None);
    }

    #[test]
    fn two_up_splits_by_the_configured_share() {
        let (main, side) = arrange(LAPTOP, Layout::Auto, 0.6, true);
        let side = side.unwrap();
        assert_eq!(main.w, 746); // (1256 - 12) * 0.6
        assert_eq!(side.x, main.x + main.w + GAP);
        assert_eq!(side.x + side.w, LAPTOP.w - GAP);
        assert_eq!(main.h, side.h);
    }

    #[test]
    fn a_silly_split_is_clamped() {
        let (main, _) = arrange(LAPTOP, Layout::Auto, 5.0, true);
        assert!(main.w < LAPTOP.w - 100);
    }

    #[test]
    fn ultrawide_keeps_a_lone_window_in_the_centre() {
        let wide = Rect::new(0, 0, 3440, 1440);
        let (main, _) = arrange(wide, Layout::Auto, 0.6, false);
        assert!(main.w < 2200);
        assert_eq!(main.x - GAP, wide.w - GAP - (main.x + main.w));
    }

    #[test]
    fn reserved_edges_move_the_stage() {
        // A 40 px bar on top and 88 px rails on both sides.
        let area = Rect::new(88, 40, 1280 - 176, 760);
        let (main, _) = arrange(area, Layout::Peek, 0.6, false);
        assert_eq!((main.x, main.y), (100, 52));
        assert_eq!(main.x + main.w, 1280 - 88 - GAP);
    }

    #[test]
    fn dialogs_float_centred() {
        let f = float(LAPTOP, 400, 300);
        assert_eq!(f, Rect::new(440, 250, 400, 300));
        assert_eq!(float(LAPTOP, 0, 0).w, 640);
    }
}
