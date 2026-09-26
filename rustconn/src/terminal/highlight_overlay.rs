//! Overlay-based colored highlight rendering for VTE terminals.
//!
//! VTE's `match_add_regex()` only shows underlines on hover — it does not
//! support custom foreground/background colors.  This module draws colored
//! rectangles and underlines on a transparent `gtk4::DrawingArea` layered
//! on top of the terminal via `gtk4::Overlay`.
//!
//! A background rule tints the whole cell behind the match; a foreground rule
//! draws a thick coloured underline under the match. The overlay cannot
//! recolour VTE's own glyphs, so a text colour is *indicated* by the underline
//! rather than applied literally — and, unlike the translucent full-cell wash
//! used in 0.22.6, an underline does not read as a background tint, which is
//! what a user setting a red *text* colour saw instead of coloured text
//! (issue #343).
//!
//! ## Architecture
//!
//! 1. [`HighlightOverlay::new`] creates a `DrawingArea` and attaches it as
//!    an overlay on the provided `gtk4::Overlay` widget.
//! 2. [`HighlightOverlay::connect`] wires VTE's `contents-changed` signal
//!    so the overlay repaints whenever terminal output changes.
//! 3. On each paint the overlay reads the visible text via
//!    `terminal.text_range_format()`, runs [`CompiledHighlightRules::find_matches`]
//!    per line, and draws colored rectangles (background) and underlines
//!    (foreground) using Cairo.
//!
//! ## Coordinate system (issue #154)
//!
//! VTE uses a single buffer-coordinate system that spans the full scrollback
//! plus the visible viewport.  `text_range_format(0, 0, row_count, col_count)`
//! reads the **first** `row_count` rows of the entire buffer — this is only
//! the visible viewport when the scrollback is empty.  After `clear` (which
//! pushes the previous screen into scrollback before erasing the visible
//! area), rows `0..row_count` become the oldest scrollback lines that still
//! contain the original colored text, while the visible viewport now lives
//! at `[vadjustment.value() .. vadjustment.value() + row_count)`.
//!
//! The fix: anchor the read range to the current viewport top
//! (`vadjustment.value()`), so highlights are computed for the lines that
//! VTE is actually painting at any given moment.
//!
//! ## Cell geometry (issue #343)
//!
//! Cell size comes from VTE's own `char_width()`/`char_height()`, not from
//! dividing the DrawingArea by the row/column count (which spread the slack over
//! every column and drifted). The grid origin comes from
//! `terminal.compute_bounds(drawing_area)`: the DrawingArea overlays a box that
//! holds VTE *and* a scrollbar, so it is wider than VTE, and VTE anchors its grid
//! top-left rather than centring it — computing the origin from VTE's own bounds
//! sidesteps both. Byte offsets are turned into columns with
//! [`byte_offset_to_column`], which counts a wide (CJK) glyph as two cells and a
//! combining mark as zero.
//!
//! ## Limitations
//!
//! - [`byte_offset_to_column`] approximates Unicode width over the common CJK,
//!   kana, Hangul, fullwidth and emoji ranges; a rarer wide block, or a
//!   multi-scalar emoji sequence (ZWJ / regional-indicator pairs) counted per
//!   scalar, can still place a highlight a cell off.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{DrawingArea, Overlay};
use rustconn_core::highlight::{CompiledHighlightRules, byte_offset_to_column};
use uuid::Uuid;
use vte4::Terminal;
use vte4::prelude::*;

/// A transparent drawing layer that renders colored highlight matches
/// on top of a VTE terminal.
pub struct HighlightOverlay {
    drawing_area: DrawingArea,
}

impl HighlightOverlay {
    /// Creates a new highlight overlay and attaches it to the given `Overlay` widget.
    ///
    /// The `DrawingArea` is set to transparent (pass-through for mouse events)
    /// so it does not interfere with VTE's own input handling.
    pub fn new(overlay: &Overlay, terminal: &Terminal) -> Self {
        let drawing_area = DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        // Let mouse events pass through to the terminal underneath
        drawing_area.set_can_target(false);

        overlay.add_overlay(&drawing_area);

        // Initial empty draw function — replaced by `connect()`
        let term_weak = terminal.downgrade();
        drawing_area.set_draw_func(move |_da, cr, _w, _h| {
            cr.set_operator(gtk4::cairo::Operator::Clear);
            let _ = cr.paint();
            cr.set_operator(gtk4::cairo::Operator::Over);
            let _ = term_weak.upgrade();
        });

        Self { drawing_area }
    }

    /// Wires the overlay to repaint on every `contents-changed` signal from VTE.
    ///
    /// `rules` is the shared compiled highlight rules map for all sessions.
    pub fn connect(
        &self,
        terminal: &Terminal,
        rules: Rc<RefCell<HashMap<Uuid, CompiledHighlightRules>>>,
        session_id: Uuid,
    ) {
        let da = self.drawing_area.clone();
        let term_for_draw = terminal.clone();
        let rules_for_draw = rules;

        self.drawing_area
            .set_draw_func(move |da, cr, _width, _height| {
                // Clear to fully transparent
                cr.set_operator(gtk4::cairo::Operator::Clear);
                if cr.paint().is_err() {
                    return;
                }
                cr.set_operator(gtk4::cairo::Operator::Over);

                let rules_map = rules_for_draw.borrow();
                let Some(compiled) = rules_map.get(&session_id) else {
                    return;
                };

                let row_count = term_for_draw.row_count();
                let col_count = term_for_draw.column_count();
                if row_count <= 0 || col_count <= 0 {
                    return;
                }

                // Use VTE's real cell size, not the overlay divided by the grid.
                //
                // VTE quantises each cell to an integer `char_width` × `char_height`,
                // so dividing the DrawingArea by the row/column count spreads the
                // unused slack across every column and the error accumulates along
                // the line and down the screen — the mispositioned highlight in
                // issue #343. The true cell size removes that drift.
                let cell_w = term_for_draw.char_width() as f64;
                let cell_h = term_for_draw.char_height() as f64;
                if cell_w <= 0.0 || cell_h <= 0.0 {
                    return;
                }

                // Anchor the grid to VTE's real position inside the overlay.
                //
                // The DrawingArea overlays `terminal_row`, a horizontal box that
                // holds the VTE terminal *and* (by default) a vertical scrollbar,
                // so the DrawingArea is wider than VTE. Deriving the origin from
                // the DrawingArea — e.g. `(overlay_width - grid) / 2` — folds half
                // the scrollbar column into every x, and assumes VTE centres its
                // grid when it actually anchors top-left (`Align::Start`), leaving
                // the slack on the right/bottom edge. Both push highlights off the
                // text (issue #343).
                //
                // `compute_bounds` gives VTE's own rectangle in DrawingArea
                // coordinates, so its origin already accounts for the scrollbar,
                // the box layout and any container padding. Under the default
                // top-left alignment the grid starts at that origin. If VTE has
                // not been allocated yet the call returns None and we skip the
                // frame rather than guess.
                let Some(bounds) = term_for_draw.compute_bounds(da) else {
                    return;
                };
                let pad_x = f64::from(bounds.x());
                let pad_y = f64::from(bounds.y());

                // Anchor the read range to the current viewport top.
                //
                // VTE addresses the entire scrollback + visible area in a
                // single coordinate system.  Reading rows 0..row_count
                // returns the first lines of the scrollback (which still
                // contain the original colored text after `clear`), not
                // the visible viewport.  See module-level docs for details
                // on issue #154.
                let viewport_top = term_for_draw
                    .vadjustment()
                    .map_or(0_i64, |adj| adj.value() as i64);

                // ponytail: re-runs the rule regex over every visible row on
                // each repaint (already coalesced to 1/frame). Fine for a
                // ~24-50 row viewport with short lines; if profiling ever shows
                // this hot (huge terminals + many rules), cache matches keyed by
                // (row text, rules version) and skip unchanged rows.
                for visible_row in 0..row_count {
                    let buffer_row = viewport_top.saturating_add(visible_row);
                    let (line_opt, _) = term_for_draw.text_range_format(
                        vte4::Format::Text,
                        buffer_row,
                        0,
                        buffer_row,
                        col_count,
                    );
                    let Some(line_gstr) = line_opt else {
                        continue;
                    };
                    let line = line_gstr.as_str();
                    if line.is_empty() {
                        continue;
                    }

                    let matches = compiled.find_matches(line);
                    if matches.is_empty() {
                        continue;
                    }

                    let y = (visible_row as f64).mul_add(cell_h, pad_y);

                    for m in &matches {
                        // Convert byte offsets to terminal columns, counting a
                        // wide (CJK) glyph as two cells and a combining mark as
                        // zero — a plain chars().count() drew a match after a wide
                        // character half a cell off (issue #343). The offset is
                        // then anchored past VTE's own padding.
                        let col_start = byte_offset_to_column(line, m.start);
                        let col_end = byte_offset_to_column(line, m.end);
                        let x = (col_start as f64).mul_add(cell_w, pad_x);
                        let w = (col_end - col_start) as f64 * cell_w;

                        // Background rule: tint the whole cell behind the match.
                        // This is the one case that legitimately fills the cell.
                        if let Some((r, g, b)) = m.background_rgb {
                            cr.set_source_rgba(r, g, b, 0.35);
                            cr.rectangle(x, y, w, cell_h);
                            if cr.fill().is_err() {
                                return;
                            }
                        }

                        // Foreground (text) rule: draw a thick coloured underline,
                        // not a full-cell wash.
                        //
                        // The overlay paints on a transparent layer above VTE and
                        // cannot recolour VTE's glyphs, so a text colour can only
                        // be *indicated*, not applied literally. 0.22.6 indicated
                        // it with a translucent full-cell fill, but that reads as a
                        // background tint — the user in issue #343 set a red text
                        // colour and saw a red background. A bold underline in the
                        // chosen colour marks the run as coloured without masquerading
                        // as a background fill, and stays visually distinct from a
                        // background rule (which fills) so the two rule kinds no
                        // longer look the same. The underline sits on the cell's
                        // baseline edge and is inset by one pixel so it is not
                        // clipped at the row boundary.
                        if let Some((r, g, b)) = m.foreground_rgb {
                            cr.set_source_rgba(r, g, b, 0.95);
                            cr.set_line_width(3.0);
                            let underline_y = y + cell_h - 2.0;
                            cr.move_to(x, underline_y);
                            cr.line_to(x + w, underline_y);
                            if cr.stroke().is_err() {
                                return;
                            }
                        }
                    }
                }
            });

        // Redraw on contents-changed and cursor-moved using idle callback.
        //
        // Both signals are needed because `contents-changed` alone does not
        // fire reliably for all escape sequences (e.g. `\033[2J` erase
        // display).  The `cursor-moved` signal fires on `\033[H` (cursor
        // home) which is always part of `clear`, ensuring the overlay
        // repaints even when `contents-changed` is not emitted (issue #154).
        //
        // `idle_add_local_once` schedules the redraw in the same main-loop
        // iteration — after VTE finishes processing the current input batch
        // but before the next frame is composited.  Coalescing is still
        // effective: rapid signals within one iteration share a single
        // pending flag, so only one `queue_draw()` fires per frame.
        let redraw_pending = Rc::new(std::cell::Cell::new(false));

        let da_weak_contents = da.downgrade();
        let redraw_pending_contents = redraw_pending.clone();
        terminal.connect_contents_changed(move |_| {
            if redraw_pending_contents.get() {
                return; // Already scheduled
            }
            redraw_pending_contents.set(true);
            let da_weak_idle = da_weak_contents.clone();
            let pending = redraw_pending_contents.clone();
            gtk4::glib::idle_add_local_once(move || {
                pending.set(false);
                if let Some(da_ref) = da_weak_idle.upgrade() {
                    da_ref.queue_draw();
                }
            });
        });

        let da_weak_cursor = da.downgrade();
        let redraw_pending_cursor = redraw_pending;
        terminal.connect_cursor_moved(move |_| {
            if redraw_pending_cursor.get() {
                return; // Already scheduled
            }
            redraw_pending_cursor.set(true);
            let da_weak_idle = da_weak_cursor.clone();
            let pending = redraw_pending_cursor.clone();
            gtk4::glib::idle_add_local_once(move || {
                pending.set(false);
                if let Some(da_ref) = da_weak_idle.upgrade() {
                    da_ref.queue_draw();
                }
            });
        });
    }

    /// Returns the underlying `DrawingArea` widget.
    #[must_use]
    #[expect(
        dead_code,
        reason = "kept alive for GTK widget lifecycle / future API exposure"
    )]
    pub fn drawing_area(&self) -> &DrawingArea {
        &self.drawing_area
    }

    /// Triggers a manual redraw of the overlay.
    #[expect(
        dead_code,
        reason = "kept alive for GTK widget lifecycle / future API exposure"
    )]
    pub fn queue_redraw(&self) {
        self.drawing_area.queue_draw();
    }
}
