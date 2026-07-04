//! Overlay-based colored highlight rendering for VTE terminals.
//!
//! VTE's `match_add_regex()` only shows underlines on hover — it does not
//! support custom foreground/background colors.  This module draws colored
//! rectangles and underlines on a transparent `gtk4::DrawingArea` layered
//! on top of the terminal via `gtk4::Overlay`.
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
//! ## Limitations
//!
//! - Wide characters (CJK) occupy 2 terminal columns but `chars().count()`
//!   treats them as 1 character, so highlight positions may be slightly off
//!   for lines containing wide characters.

use gtk4::prelude::*;
use gtk4::{DrawingArea, Overlay};
use rustconn_core::highlight::CompiledHighlightRules;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
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
            .set_draw_func(move |_da, cr, width, height| {
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

                // Compute cell dimensions from the terminal's visible area
                let cell_w = f64::from(width) / col_count as f64;
                let cell_h = f64::from(height) / row_count as f64;
                if cell_w <= 0.0 || cell_h <= 0.0 {
                    return;
                }

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

                    let y = visible_row as f64 * cell_h;

                    for m in &matches {
                        // Convert byte offsets to column positions. col_end is
                        // computed as a delta from col_start so we scan each
                        // line slice once instead of twice from the start.
                        let col_start = line[..m.start].chars().count();
                        let col_end = col_start + line[m.start..m.end].chars().count();
                        let x = col_start as f64 * cell_w;
                        let w = (col_end - col_start) as f64 * cell_w;

                        // Draw background highlight rectangle (colour pre-parsed)
                        if let Some((r, g, b)) = m.background_rgb {
                            cr.set_source_rgba(r, g, b, 0.35);
                            cr.rectangle(x, y, w, cell_h);
                            if cr.fill().is_err() {
                                return;
                            }
                        }

                        // Draw foreground colored underline (2px thick)
                        if let Some((r, g, b)) = m.foreground_rgb {
                            cr.set_source_rgba(r, g, b, 0.9);
                            cr.set_line_width(2.0);
                            cr.move_to(x, y + cell_h - 1.0);
                            cr.line_to(x + w, y + cell_h - 1.0);
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
