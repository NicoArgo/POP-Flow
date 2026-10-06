// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow: sizing of the alt-tab thumbnail grid, kept free of widgets so it
//! can be unit-tested across window counts, screen sizes and scale factors.
//!
//! # Rules
//!
//! All lengths are logical pixels (what the layer surface lays out in).
//!
//! 1. **One size for every cell.** Every thumbnail box is the base
//!    [`BASE_W`]×[`BASE_H`] times a single scale, so the box keeps its
//!    aspect ratio (≈1.69:1) at every size; the captured image is drawn
//!    `Contain` inside it, so the window's own aspect ratio is never stretched
//!    (an ultrawide window letterboxes, a portrait one pillarboxes).
//! 2. **Never giant.** The scale is capped at 1.0 — 264×156, about what
//!    Windows shows — so one or two windows on a 3440×1440 screen stay
//!    thumbnail-sized instead of filling the screen. On HiDPI the cap also
//!    stays within the capture resolution ([`CAPTURE_MAX_DIM`] physical px on
//!    the longest side), so the image is never upscaled: at scale 2.0 the
//!    biggest box is 256×151.
//! 3. **Biggest that fits.** Otherwise every column count is tried and the
//!    one giving the largest thumbnails that fit `avail_w`×`avail_h` wins.
//!    Ties (several layouts already at the cap) go to the fewest rows, then
//!    the fewest columns — 4 windows on 1080p sit in one row, not a column of
//!    4; 8 windows make a 4×2 block rather than a 7+1 ragged pair of rows.
//! 4. **Never unreadable.** Thumbnails don't shrink below [`MIN_THUMB_W`]
//!    (120×70). Captions and paddings are fixed chrome and never shrink.
//! 5. **Never off screen.** If even the minimum size can't fit every window,
//!    the grid keeps the minimum size, uses as many columns as fit, and shows
//!    only as many rows as fit ([`GridLayout::visible_rows`]) plus a one-line
//!    "N more" hint ([`HINT_H`]). The rows on screen page so the focused cell
//!    is always visible ([`GridLayout::first_visible_row`]); cells keep their
//!    fixed order, only the window onto them moves. Only on screens too small
//!    for a single minimum-size cell (not a real setup) does the size drop
//!    below the minimum, to that one cell.

/// Base (maximum, at scale 1.0) thumbnail box, in logical px.
pub const BASE_W: f32 = 264.0;
pub const BASE_H: f32 = 156.0;
/// Smallest thumbnail width before the grid starts paging rows instead of
/// shrinking further. 120 px wide (≈70 tall) still shows a recognizable window.
pub const MIN_THUMB_W: f32 = 120.0;
/// Caption line reserved under every thumbnail.
pub const LABEL_H: f32 = 20.0;
/// Space between thumbnail and caption inside a cell.
pub const LABEL_GAP: f32 = 6.0;
/// Button padding around every cell (each side).
pub const CELL_PAD: f32 = 6.0;
/// Height reserved for the "N more" line when rows are paged.
pub const HINT_H: f32 = LABEL_H + 8.0;
/// Longest side, in physical px, of the captured thumbnail buffers.
pub const CAPTURE_MAX_DIM: f32 = crate::wayland::THUMB_MAX_DIM as f32;

/// Horizontal / vertical space the grid itself may use on an output of
/// `screen_w`×`screen_h` logical px whose top `top_margin` px are covered (by
/// a panel): the surrounding container's padding, border and top offset are
/// taken out here so the grid never pushes the overlay past the screen.
pub fn available_area(screen_w: f32, screen_h: f32, top_margin: f32) -> (f32, f32) {
    (
        (screen_w - 64.0).max(0.0),
        (screen_h - top_margin - 112.0).max(0.0),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridInput {
    /// Number of cells (windows).
    pub count: usize,
    /// Space the grid may occupy, logical px (see [`available_area`]).
    pub avail_w: f32,
    pub avail_h: f32,
    /// Output scale factor (1.0, 1.25, 2.0, ...).
    pub scale_factor: f32,
    /// Gap between cells, both directions.
    pub spacing: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLayout {
    pub cols: usize,
    /// Total rows needed for every cell.
    pub rows: usize,
    /// Rows actually shown; `< rows` only when paging (rule 5).
    pub visible_rows: usize,
    /// Thumbnail box, logical px (whole pixels).
    pub thumb_w: f32,
    pub thumb_h: f32,
}

impl GridLayout {
    /// Outer width of one cell (thumbnail + padding).
    pub fn cell_w(&self) -> f32 {
        self.thumb_w + 2.0 * CELL_PAD
    }

    /// Outer height of one cell (thumbnail + caption + padding).
    pub fn cell_h(&self) -> f32 {
        self.thumb_h + LABEL_GAP + LABEL_H + 2.0 * CELL_PAD
    }

    /// Whether some rows are paged out (rule 5).
    pub fn paged(&self) -> bool {
        self.visible_rows < self.rows
    }

    /// Total size the grid occupies on screen, including the paging hint.
    pub fn size(&self, spacing: f32) -> (f32, f32) {
        let (c, r) = (self.cols as f32, self.visible_rows as f32);
        let w = c * self.cell_w() + (c - 1.0).max(0.0) * spacing;
        let mut h = r * self.cell_h() + (r - 1.0).max(0.0) * spacing;
        if self.paged() {
            h += HINT_H;
        }
        (w, h)
    }

    /// First row on screen when the cell `focused` is highlighted: rows page in
    /// blocks of `visible_rows`, the last page aligned to the bottom so it is
    /// always full. Stateless, so the same focus always shows the same rows.
    pub fn first_visible_row(&self, focused: usize) -> usize {
        if !self.paged() || self.visible_rows == 0 {
            return 0;
        }
        let row = focused / self.cols.max(1);
        let page_start = (row / self.visible_rows) * self.visible_rows;
        page_start.min(self.rows - self.visible_rows)
    }
}

/// Largest scale at which a `cols`×`rows` grid fits the input's area.
fn fit_scale(input: &GridInput, cols: usize, rows: usize) -> f32 {
    let (c, r) = (cols as f32, rows as f32);
    let fixed_w = c * 2.0 * CELL_PAD + (c - 1.0) * input.spacing;
    let fixed_h = r * (LABEL_GAP + LABEL_H + 2.0 * CELL_PAD) + (r - 1.0) * input.spacing;
    let s_w = (input.avail_w - fixed_w) / (c * BASE_W);
    let s_h = (input.avail_h - fixed_h) / (r * BASE_H);
    s_w.min(s_h)
}

/// How many cells of thumbnail scale `s` fit in `avail` along one axis.
fn fit_count(avail: f32, thumb: f32, fixed: f32, spacing: f32) -> usize {
    ((avail + spacing) / (thumb + fixed + spacing))
        .floor()
        .max(0.0) as usize
}

/// Size the alt-tab grid. See the [module docs](self) for the rules.
pub fn layout(input: GridInput) -> GridLayout {
    let n = input.count;
    let sf = if input.scale_factor > 0.0 {
        input.scale_factor
    } else {
        1.0
    };
    let max_scale = (CAPTURE_MAX_DIM / (BASE_W * sf)).min(1.0);
    let min_scale = (MIN_THUMB_W / BASE_W).min(max_scale);
    let sized = |cols: usize, rows: usize, visible_rows: usize, s: f32| GridLayout {
        cols,
        rows,
        visible_rows,
        thumb_w: (BASE_W * s + 1e-3).floor().max(1.0),
        thumb_h: (BASE_H * s + 1e-3).floor().max(1.0),
    };

    if n == 0 {
        return sized(1, 0, 0, max_scale);
    }

    // Rule 3: biggest thumbnails; ties -> fewest rows, then fewest columns.
    let mut best: Option<(f32, usize, usize)> = None;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let s = fit_scale(&input, cols, rows).min(max_scale);
        let better = match best {
            None => true,
            Some((bs, bc, br)) => {
                if (s - bs).abs() > 1e-4 {
                    s > bs
                } else {
                    (rows, cols) < (br, bc)
                }
            }
        };
        if better {
            best = Some((s, cols, rows));
        }
    }
    let (s, cols, rows) = best.expect("n > 0");
    if s >= min_scale {
        return sized(cols, rows, rows, s);
    }

    // Rule 5: pin the size at the minimum and page rows.
    let s = min_scale;
    let fixed_w = 2.0 * CELL_PAD;
    let fixed_h = LABEL_GAP + LABEL_H + 2.0 * CELL_PAD;
    let cols = fit_count(input.avail_w, BASE_W * s, fixed_w, input.spacing).clamp(1, n);
    let rows = n.div_ceil(cols);
    let visible =
        fit_count(input.avail_h - HINT_H, BASE_H * s, fixed_h, input.spacing).clamp(1, rows);
    let mut l = sized(cols, rows, visible, s);
    // Degenerate screens: not even one minimum cell fits. Shrink that one cell
    // to the space there is rather than overflow.
    let (w, h) = l.size(input.spacing);
    if w > input.avail_w + 0.5 || h > input.avail_h + 0.5 {
        let hint = if l.paged() { HINT_H } else { 0.0 };
        let s_w = (input.avail_w - fixed_w * cols as f32 - input.spacing * (cols as f32 - 1.0))
            / (cols as f32 * BASE_W);
        let r = visible as f32;
        let s_h = (input.avail_h - hint - fixed_h * r - input.spacing * (r - 1.0)) / (r * BASE_H);
        l = sized(cols, rows, visible, s_w.min(s_h).max(0.0));
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACING: f32 = 8.0;
    /// (name, physical w, physical h)
    const SCREENS: [(&str, f32, f32); 4] = [
        ("1366x768", 1366.0, 768.0),
        ("1920x1080", 1920.0, 1080.0),
        ("2560x1440", 2560.0, 1440.0),
        ("3440x1440", 3440.0, 1440.0),
    ];
    const SCALES: [f32; 3] = [1.0, 1.25, 2.0];
    const COUNTS: [usize; 8] = [1, 2, 3, 5, 8, 12, 20, 40];

    fn grid(n: usize, phys_w: f32, phys_h: f32, sf: f32) -> (GridInput, GridLayout) {
        let (aw, ah) = available_area(phys_w / sf, phys_h / sf, 0.0);
        let input = GridInput {
            count: n,
            avail_w: aw,
            avail_h: ah,
            scale_factor: sf,
            spacing: SPACING,
        };
        (input, layout(input))
    }

    #[test]
    fn matrix_invariants() {
        for (name, pw, ph) in SCREENS {
            for sf in SCALES {
                for n in COUNTS {
                    let (input, l) = grid(n, pw, ph, sf);
                    let ctx = format!("{name}@{sf} n={n}: {l:?}");
                    // Every window has a cell.
                    assert!(l.cols * l.rows >= n, "{ctx}");
                    assert!(l.cols * (l.rows - 1) < n, "no empty trailing row: {ctx}");
                    // Never off screen.
                    let (w, h) = l.size(SPACING);
                    assert!(
                        w <= input.avail_w + 0.5,
                        "width {w} > {}: {ctx}",
                        input.avail_w
                    );
                    assert!(
                        h <= input.avail_h + 0.5,
                        "height {h} > {}: {ctx}",
                        input.avail_h
                    );
                    // Never giant, never upscaled past the capture.
                    assert!(l.thumb_w <= BASE_W && l.thumb_h <= BASE_H, "{ctx}");
                    assert!(l.thumb_w * sf <= CAPTURE_MAX_DIM + 0.5, "{ctx}");
                    // Never unreadable.
                    assert!(l.thumb_w >= MIN_THUMB_W - 1.0, "{ctx}");
                    // Aspect ratio of the box matches the base box.
                    let ar = l.thumb_w / l.thumb_h;
                    assert!((ar - BASE_W / BASE_H).abs() < 0.03, "aspect {ar}: {ctx}");
                    // Paging only when the minimum size can't hold everything.
                    if l.paged() {
                        assert!(l.thumb_w <= MIN_THUMB_W, "paged above minimum: {ctx}");
                    }
                }
            }
        }
    }

    #[test]
    fn few_windows_stay_thumbnail_sized_in_a_row() {
        // 1–5 windows at 1.0 on anything 1080p+ are full size in one row.
        for (name, pw, ph) in &SCREENS[1..] {
            for n in [1, 2, 3, 5] {
                let (_, l) = grid(n, *pw, *ph, 1.0);
                assert_eq!((l.cols, l.rows), (n, 1), "{name} n={n}");
                assert_eq!((l.thumb_w, l.thumb_h), (264.0, 156.0), "{name} n={n}");
            }
        }
        // One window on an ultrawide doesn't grow past the base size.
        let (_, l) = grid(1, 3440.0, 1440.0, 1.0);
        assert_eq!((l.thumb_w, l.thumb_h), (BASE_W, BASE_H));
    }

    #[test]
    fn ties_prefer_fewer_rows_then_fewer_columns() {
        // 8 windows on 1080p: full size needs two rows; 4×2, not 7+1 or 1×8.
        let (_, l) = grid(8, 1920.0, 1080.0, 1.0);
        assert_eq!((l.cols, l.rows, l.thumb_w), (4, 2, 264.0));
        // 12 on 1080p: 6×2 at full size.
        let (_, l) = grid(12, 1920.0, 1080.0, 1.0);
        assert_eq!((l.cols, l.rows, l.thumb_w), (6, 2, 264.0));
    }

    #[test]
    fn hidpi_caps_at_capture_resolution() {
        // 512 px buffers at 2.0 → at most 256 logical px wide.
        let (_, l) = grid(1, 2560.0, 1440.0, 2.0);
        assert_eq!((l.thumb_w, l.thumb_h), (256.0, 151.0));
        // 1.25 doesn't hit the capture cap.
        let (_, l) = grid(1, 2560.0, 1440.0, 1.25);
        assert_eq!(l.thumb_w, 264.0);
    }

    #[test]
    fn many_windows_shrink_then_page() {
        // 40 windows on a 1366×768 laptop still fit, shrunk.
        let (_, l) = grid(40, 1366.0, 768.0, 1.0);
        assert!(!l.paged(), "{l:?}");
        assert!(l.thumb_w < BASE_W && l.thumb_w >= MIN_THUMB_W, "{l:?}");
        // 40 windows on 1366×768 at 1.25 can't: minimum size, rows paged.
        let (_, l) = grid(40, 1366.0, 768.0, 1.25);
        assert!(l.paged(), "{l:?}");
        assert_eq!(l.thumb_w, MIN_THUMB_W);
        assert!(l.visible_rows * l.cols < 40);
    }

    #[test]
    fn paging_keeps_focus_visible() {
        let (_, l) = grid(40, 1366.0, 768.0, 2.0);
        assert!(l.paged(), "{l:?}");
        for focused in 0..40 {
            let first = l.first_visible_row(focused);
            let row = focused / l.cols;
            assert!(
                row >= first && row < first + l.visible_rows,
                "focused={focused} {l:?}"
            );
            assert!(first + l.visible_rows <= l.rows);
        }
        // Not paged → always from the top.
        let (_, l) = grid(5, 1920.0, 1080.0, 1.0);
        assert_eq!(l.first_visible_row(4), 0);
    }

    #[test]
    fn empty_and_degenerate() {
        let l = layout(GridInput {
            count: 0,
            avail_w: 1000.0,
            avail_h: 1000.0,
            scale_factor: 1.0,
            spacing: 8.0,
        });
        assert_eq!((l.cols, l.rows, l.visible_rows), (1, 0, 0));
        // Smaller than one minimum cell: shrink to fit rather than overflow.
        let input = GridInput {
            count: 3,
            avail_w: 100.0,
            avail_h: 90.0,
            scale_factor: 1.0,
            spacing: 8.0,
        };
        let l = layout(input);
        let (w, h) = l.size(8.0);
        assert!(w <= 100.5 && h <= 90.5, "{l:?}");
    }

    /// Prints the whole matrix: `cargo test --release grid::tests::print -- --nocapture --ignored`.
    #[test]
    #[ignore]
    fn print_matrix() {
        for (name, pw, ph) in SCREENS {
            for sf in SCALES {
                for n in COUNTS {
                    let (_, l) = grid(n, pw, ph, sf);
                    println!(
                        "{name:>9} @{sf:<4} n={n:>2}: {}x{} (visible {}) thumb {}x{}",
                        l.cols, l.rows, l.visible_rows, l.thumb_w, l.thumb_h
                    );
                }
            }
        }
    }
}
