//! Overlay-based colored highlight rendering for VTE terminals.
//!
//! VTE's `match_add_regex()` only shows underlines on hover — it does not
//! support custom foreground/background colors.  This module draws colored
//! rectangles and underlines on a transparent `gtk4::DrawingArea` layered
//! on top of the terminal via `gtk4::Overlay`.
//!
//! A background rule tints the whole cell behind the match; a foreground rule
//! draws a thick underline in its colour. The overlay paints on a transparent
//! layer above VTE and cannot recolour VTE's own glyphs, so the rule editors
//! call the foreground colour the *underline* colour: recolouring the text
//! itself takes an output filter such as `chromaterm`, which rewrites the stream
//! before VTE sees it (issue #343).
//!
//! ## Architecture
//!
//! [`HighlightOverlay::attach`] creates the `DrawingArea`, puts it on the
//! `gtk4::Overlay` that hosts the terminal, and repaints it whenever VTE's text,
//! cursor or cell size changes, the view scrolls or is resized, or the display
//! scale changes. On each paint it reads the visible text via
//! `terminal.text_range_format()`, runs [`CompiledHighlightRules::find_matches`]
//! per line, and draws colored rectangles (background) and underlines
//! (foreground) using Cairo, clipped to the character grid.
//!
//! ## Lifecycle (issue #343)
//!
//! A session's terminal moves — into a split pane, into a detached window and
//! back — and every move wraps it in a different `gtk4::Overlay`. The drawing
//! layer follows it: whenever the terminal is mapped, the layer re-homes itself
//! on the overlay that hosts the terminal now. Pinned to the overlay it was
//! first attached to, it used to vanish after a detach and never appear in a
//! split pane.
//!
//! The value owns everything it adds. Dropping it takes the layer off its
//! overlay, disconnects the signal handlers it put on the terminal and on the
//! terminal's scroll adjustment, and unregisters the hover regexes it was
//! handed, so replacing a session's rules — on reconnect, or after Settings
//! change — no longer stacks a second layer, a second set of handlers and a
//! second set of regexes on the first.
//!
//! ## Coordinate system (issues #154, #343)
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
//! VTE is actually painting at any given moment — as long as VTE has dropped
//! no scrollback yet (see Limitations).
//!
//! That value counts rows but need not be a whole number: touchpad scrolling
//! and a drag on the scrollbar leave the view between two rows, and VTE keeps
//! it there. It then draws buffer row `r` with its top edge
//! `r * char_height - round(value * char_height)` pixels below the top of its
//! grid (`row_to_pixel()` in `vte.cc`, the same in 0.80.5 and 0.84), so the top
//! row is cut part-way and one more row shows at the bottom. [`viewport_rows`]
//! repeats that arithmetic. Truncating the value to whole rows, as the layer
//! used to, put every highlight in such a view up to a row below its text and
//! left the bottom row bare (issue #343).
//!
//! Repaints do not wait for `contents-changed` when the view moves. VTE queues
//! that signal behind its own update cycle when the view scrolls, and emits
//! none when a resize moves the rows (see `vte_contract_tests` in
//! `terminal/mod.rs`), which left the highlights on the old rows until the next
//! output. The layer repaints on the adjustment's own `value-changed` and
//! `changed` signals instead, and when the display scale changes.
//!
//! ## Cell geometry (issue #343)
//!
//! Cell size comes from VTE's own `char_width()`/`char_height()`, not from
//! dividing the DrawingArea by the row/column count (which spread the slack over
//! every column and drifted). The grid starts at the top-left of VTE's *content*
//! box, which [`grid_origin`] maps into the DrawingArea's coordinates. That
//! absorbs the scrollbar beside the terminal and VTE's own CSS padding (1px by
//! default), both of which an origin derived from the DrawingArea or from VTE's
//! border box got wrong. Measured against VTE 0.84 by rendering a full block to
//! a texture: the block's first pixel is exactly at the content-box origin. The
//! Flatpak bundles VTE 0.80.5, which was not measured; its source places the
//! grid the same way, at the start of both axes (`xalign` and `yalign` default
//! to start), with any spare pixels left at the right and bottom.
//! Byte offsets are turned into columns with [`byte_offset_to_column`], which
//! counts a wide (CJK) glyph as two cells, a combining mark as zero and a tab up
//! to the next tab stop.
//!
//! ## Limitations
//!
//! - [`byte_offset_to_column`] approximates Unicode width over the common CJK,
//!   kana, Hangul, fullwidth and emoji ranges; a rarer wide block, or a
//!   multi-scalar emoji sequence (ZWJ / regional-indicator pairs) counted per
//!   scalar, can still place a highlight a cell off. Tabs assume VTE's default
//!   stop every eight columns.
//! - In a scrolled-back view VTE also paints into the spare pixels below its
//!   last whole row (`yfill`, on by default) when the terminal's height is not
//!   a whole number of rows. The layer is clipped to whole rows, so a highlight
//!   in that sliver of less than a row is cut short.
//! - When VTE scrolls by itself (mouse wheel, touchpad over the terminal) it
//!   moves its own view first and hands the new position to the adjustment
//!   from its update cycle, so the layer can trail the text for a frame until
//!   `value-changed` arrives. The adjustment is the only public read of the
//!   scroll position.
//! - VTE's adjustment counts from the oldest row it still keeps
//!   (`notify_scroll_value_changed()` in `widget.cc`, 0.76 and 0.80.5), while
//!   `text_range_format()` takes absolute rows. The two agree until VTE drops
//!   scrollback — the scrollback limit is reached, or `clear` sends `ESC [3J` —
//!   after which the layer reads rows as many lines too early as were dropped.
//!   No public VTE API reports that offset. Read from the source, not yet
//!   reproduced.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Adjustment, DrawingArea, Overlay, glib, graphene};
use rustconn_core::highlight::{CompiledHighlightRules, byte_offset_to_column, viewport_rows};
use uuid::Uuid;
use vte4::Terminal;
use vte4::prelude::*;

/// How many levels above the terminal to look for the overlay that hosts it.
///
/// A tab and a split pane both wrap the terminal as
/// `Overlay > Box(terminal, scrollbar)`, so the overlay is two levels up. One
/// level of slack keeps an extra wrapper working without letting the search
/// climb to an unrelated overlay near the top of the window.
const HOST_OVERLAY_SEARCH_DEPTH: usize = 3;

/// A transparent drawing layer that renders colored highlight matches
/// on top of a VTE terminal.
///
/// Owns what it adds to the terminal and its scroll adjustment, and removes
/// all of it when dropped — see the module's lifecycle notes.
pub struct HighlightOverlay {
    drawing_area: DrawingArea,
    terminal: glib::WeakRef<Terminal>,
    /// Handlers connected on the terminal, disconnected on drop.
    terminal_handlers: Vec<glib::SignalHandlerId>,
    /// The terminal's vertical adjustment, shared with the scrollbar beside it;
    /// `None` when VTE has none.
    adjustment: Option<glib::WeakRef<Adjustment>>,
    /// Handlers connected on `adjustment`, disconnected on drop.
    adjustment_handlers: Vec<glib::SignalHandlerId>,
    /// Hover regexes registered on the terminal for these rules, removed on drop.
    match_tags: Vec<i32>,
}

impl HighlightOverlay {
    /// Creates a session's highlight layer and attaches it above `terminal`.
    ///
    /// `rules` is the shared compiled-rules map for all sessions; the layer
    /// draws the entry for `session_id`. `match_tags` are the hover regexes the
    /// caller registered on `terminal` for the same rules. The layer takes them
    /// over and unregisters them when it is dropped.
    pub fn attach(
        terminal: &Terminal,
        rules: Rc<RefCell<HashMap<Uuid, CompiledHighlightRules>>>,
        session_id: Uuid,
        match_tags: Vec<i32>,
    ) -> Self {
        // Presentation: a decoration over the terminal, whose own text a
        // screen reader already reads; `accessible-role` is construct-only.
        let drawing_area = DrawingArea::builder()
            .accessible_role(gtk4::AccessibleRole::Presentation)
            .build();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        // Let mouse events pass through to the terminal underneath
        drawing_area.set_can_target(false);

        // Weak: the terminal must not be kept alive by its own decoration.
        let term_weak = terminal.downgrade();
        drawing_area.set_draw_func(move |da, cr, _width, _height| {
            // Clear to fully transparent
            cr.set_operator(gtk4::cairo::Operator::Clear);
            if cr.paint().is_err() {
                return;
            }
            cr.set_operator(gtk4::cairo::Operator::Over);

            let Some(terminal) = term_weak.upgrade() else {
                return;
            };
            let rules_map = rules.borrow();
            let Some(compiled) = rules_map.get(&session_id) else {
                return;
            };
            draw_matches(da, cr, &terminal, compiled);
        });

        let schedule_redraw = redraw_scheduler(&drawing_area);
        let mut terminal_handlers = Vec::with_capacity(5);

        // Repaint when the text changes. `cursor-moved` too, because
        // `contents-changed` alone does not fire reliably for every escape
        // sequence (e.g. `\033[2J`, erase display), while the cursor home that
        // `clear` always sends does move the cursor (issue #154).
        let redraw = Rc::clone(&schedule_redraw);
        terminal_handlers.push(terminal.connect_contents_changed(move |_| redraw()));
        let redraw = Rc::clone(&schedule_redraw);
        terminal_handlers.push(terminal.connect_cursor_moved(move |_| redraw()));
        // A font change or zoom resizes the cells, and VTE only reports
        // `contents-changed` for it when the row or column count changes too.
        let redraw = Rc::clone(&schedule_redraw);
        terminal_handlers.push(terminal.connect_char_size_changed(move |_, _, _| redraw()));
        // A move to a display with another scale factor redraws the terminal
        // at the new scale; the layer is redrawn with it.
        let redraw = next_frame_redraw(&drawing_area);
        terminal_handlers.push(terminal.connect_scale_factor_notify(move |_| redraw()));

        // Scrolling and resizing move the rows under the layer without changing
        // their text, so repaint whenever the adjustment's position or range
        // changes (see the module's coordinate-system notes).
        let adjustment = terminal.vadjustment();
        let mut adjustment_handlers = Vec::with_capacity(2);
        if let Some(adjustment) = &adjustment {
            let redraw = next_frame_redraw(&drawing_area);
            adjustment_handlers.push(adjustment.connect_value_changed(move |_| redraw()));
            let redraw = next_frame_redraw(&drawing_area);
            adjustment_handlers.push(adjustment.connect_changed(move |_| redraw()));
        }

        // Follow the terminal when it is shown somewhere new. Deferred to idle so
        // the widget tree is not rearranged in the middle of a `map` emission.
        let da_weak = drawing_area.downgrade();
        terminal_handlers.push(terminal.connect_map(move |terminal| {
            let da_weak = da_weak.clone();
            let term_weak = terminal.downgrade();
            glib::idle_add_local_once(move || {
                if let (Some(da), Some(terminal)) = (da_weak.upgrade(), term_weak.upgrade()) {
                    attach_to_host_overlay(&da, &terminal);
                }
            });
        }));

        attach_to_host_overlay(&drawing_area, terminal);

        Self {
            drawing_area,
            terminal: terminal.downgrade(),
            terminal_handlers,
            adjustment: adjustment.map(|adjustment| adjustment.downgrade()),
            adjustment_handlers,
            match_tags,
        }
    }
}

impl Drop for HighlightOverlay {
    fn drop(&mut self) {
        detach_from_parent(&self.drawing_area);
        if let Some(terminal) = self.terminal.upgrade() {
            for handler in self.terminal_handlers.drain(..) {
                terminal.disconnect(handler);
            }
            // Removing a tag VTE no longer knows (the search dialog clears all
            // matches) is a no-op.
            for tag in self.match_tags.drain(..) {
                terminal.match_remove(tag);
            }
        }
        // Checked on its own: the scrollbar beside the terminal holds the
        // adjustment too, so it can outlive the terminal.
        if let Some(adjustment) = &self.adjustment
            && let Some(adjustment) = adjustment.upgrade()
        {
            for handler in self.adjustment_handlers.drain(..) {
                adjustment.disconnect(handler);
            }
        }
    }
}

/// Returns a callback that repaints the layer once, on the next idle.
///
/// `idle_add_local_once` runs after VTE finishes processing the current input
/// batch but before the next frame is composited, and rapid signals within one
/// main-loop iteration share a single pending flag, so a burst of output costs
/// one `queue_draw()`. The callback holds the layer weakly.
fn redraw_scheduler(drawing_area: &DrawingArea) -> Rc<dyn Fn()> {
    let pending = Rc::new(Cell::new(false));
    let da_weak = drawing_area.downgrade();
    Rc::new(move || {
        if pending.replace(true) {
            return; // Already scheduled
        }
        let pending = Rc::clone(&pending);
        let da_weak = da_weak.clone();
        glib::idle_add_local_once(move || {
            pending.set(false);
            if let Some(da) = da_weak.upgrade() {
                da.queue_draw();
            }
        });
    })
}

/// Returns a callback that queues a repaint of the layer for the next frame.
///
/// For the signals that move the grid under the layer rather than change its
/// text: scrolling, a resize and a change of display scale. GTK already folds
/// repeated `queue_draw()` calls into one draw per frame, so these need no idle
/// hop of their own, and the layer is queued the moment the view moves. The
/// callback holds the layer weakly, and `use<>` keeps it from borrowing
/// `drawing_area`, so it can go to a `'static` signal handler.
///
/// `pub(super)` because the failed-command marker overlay
/// ([`super::failed_marker`]) reuses the same terminal-overlay plumbing.
pub(super) fn next_frame_redraw(drawing_area: &DrawingArea) -> impl Fn() + use<> {
    let da_weak = drawing_area.downgrade();
    move || {
        if let Some(da) = da_weak.upgrade() {
            da.queue_draw();
        }
    }
}

/// Finds the `gtk4::Overlay` that hosts `terminal` right now, if any.
fn host_overlay(terminal: &Terminal) -> Option<Overlay> {
    let mut ancestor = terminal.parent();
    for _ in 0..HOST_OVERLAY_SEARCH_DEPTH {
        match ancestor?.downcast::<Overlay>() {
            Ok(overlay) => return Some(overlay),
            Err(widget) => ancestor = widget.parent(),
        }
    }
    None
}

/// Puts the drawing layer on the overlay that hosts `terminal` now.
///
/// A no-op when it is already there. Otherwise the layer leaves its old overlay
/// and joins the new one directly above the terminal — below any controls that
/// overlay also carries, such as a split pane's corner buttons, so a highlight
/// never paints over them.
///
/// `pub(super)` so [`super::failed_marker`] can share the same attachment logic.
pub(super) fn attach_to_host_overlay(drawing_area: &DrawingArea, terminal: &Terminal) {
    let Some(overlay) = host_overlay(terminal) else {
        return;
    };
    if drawing_area.parent().as_ref() == Some(overlay.upcast_ref::<gtk4::Widget>()) {
        return;
    }
    detach_from_parent(drawing_area);
    overlay.add_overlay(drawing_area);
    if let Some(main_child) = overlay.child() {
        drawing_area.insert_after(&overlay, Some(&main_child));
    }
    drawing_area.queue_draw();
}

/// Takes the drawing layer off whatever it is attached to.
fn detach_from_parent(drawing_area: &DrawingArea) {
    let Some(parent) = drawing_area.parent() else {
        return;
    };
    match parent.downcast::<Overlay>() {
        Ok(overlay) => overlay.remove_overlay(drawing_area),
        Err(_) => drawing_area.unparent(),
    }
}

/// Top-left corner of VTE's character grid, in the DrawingArea's coordinates.
///
/// A widget's own coordinates start at its content box, so this is the point
/// `(0, 0)` of the terminal mapped into the DrawingArea. `None` until both are
/// in the same, allocated widget tree — the frame is skipped rather than guessed.
pub(super) fn grid_origin(terminal: &Terminal, drawing_area: &DrawingArea) -> Option<(f64, f64)> {
    let origin = terminal.compute_point(drawing_area, &graphene::Point::new(0.0, 0.0))?;
    Some((f64::from(origin.x()), f64::from(origin.y())))
}

/// Paints the matches of `compiled` for the rows VTE is showing right now.
fn draw_matches(
    da: &DrawingArea,
    cr: &gtk4::cairo::Context,
    terminal: &Terminal,
    compiled: &CompiledHighlightRules,
) {
    let row_count = terminal.row_count();
    let col_count = terminal.column_count();
    if row_count <= 0 || col_count <= 0 {
        return;
    }

    // VTE quantises each cell to an integer `char_width` × `char_height`, so
    // dividing the DrawingArea by the row/column count spreads the unused slack
    // across every column and the error accumulates along the line and down the
    // screen. The true cell size removes that drift.
    let cell_w = terminal.char_width() as f64;
    let cell_h = terminal.char_height() as f64;
    if cell_w <= 0.0 || cell_h <= 0.0 {
        return;
    }

    let Some((origin_x, origin_y)) = grid_origin(terminal, da) else {
        return;
    };

    // Anchor the read range to the rows VTE is painting, down to the pixel.
    //
    // VTE addresses the entire scrollback + visible area in a single coordinate
    // system, so the viewport starts at the adjustment's value rather than at
    // row 0 (issue #154). Between two rows that value is fractional, and VTE
    // then draws the top row part-way above the grid and one more row below it
    // (issue #343). See the module docs.
    let scroll_value = terminal.vadjustment().map_or(0.0, |adj| adj.value());
    let viewport = viewport_rows(scroll_value, cell_h, row_count);
    tracing::trace!(
        scroll_value,
        cell_w,
        cell_h,
        origin_x,
        origin_y,
        first_row = viewport.first_row,
        y_offset = viewport.y_offset,
        rows = viewport.rows,
        "highlight layer geometry"
    );

    // Paint inside the character grid only, so the top row's part above it and
    // the bottom row's part below it land neither on VTE's padding nor on
    // anything else the overlay hosts, such as the scrollbar.
    // ponytail: whole rows only — a highlight in the spare pixels VTE paints
    // below them when scrolled back is cut short (module docs, Limitations);
    // clip to `terminal.height()` and draw one row more if that ever matters.
    let grid_width = col_count as f64 * cell_w;
    let grid_height = row_count as f64 * cell_h;
    if cr.save().is_err() {
        return;
    }
    cr.rectangle(origin_x, origin_y, grid_width, grid_height);
    cr.clip();

    // ponytail: re-runs the rule regex over every visible row on each repaint
    // (already coalesced to 1/frame). Fine for a ~24-50 row viewport with short
    // lines; if profiling ever shows this hot (huge terminals + many rules),
    // cache matches keyed by (row text, rules version) and skip unchanged rows.
    'rows: for visible_row in 0..viewport.rows {
        let buffer_row = viewport.first_row.saturating_add(visible_row);
        let (line_opt, _) =
            terminal.text_range_format(vte4::Format::Text, buffer_row, 0, buffer_row, col_count);
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

        let y = (visible_row as f64).mul_add(cell_h, origin_y - viewport.y_offset);

        for m in &matches {
            // Byte offsets to terminal columns: a wide (CJK) glyph is two cells,
            // a combining mark none, a tab runs to the next stop.
            let col_start = byte_offset_to_column(line, m.start);
            let col_end = byte_offset_to_column(line, m.end);
            let x = (col_start as f64).mul_add(cell_w, origin_x);
            let w = (col_end - col_start) as f64 * cell_w;

            // Background rule: tint the whole cell behind the match.
            if let Some((r, g, b)) = m.background_rgb {
                cr.set_source_rgba(r, g, b, 0.35);
                cr.rectangle(x, y, w, cell_h);
                if cr.fill().is_err() {
                    break 'rows;
                }
            }

            // Foreground rule: a thick underline in the rule's colour. It cannot
            // recolour VTE's glyphs (see the module docs), and unlike the
            // full-cell wash 0.22.6 used it does not read as a background tint.
            // Inset from the row's bottom edge so it is not clipped.
            if let Some((r, g, b)) = m.foreground_rgb {
                cr.set_source_rgba(r, g, b, 0.95);
                cr.set_line_width(3.0);
                let underline_y = y + cell_h - 2.0;
                cr.move_to(x, underline_y);
                cr.line_to(x + w, underline_y);
                if cr.stroke().is_err() {
                    break 'rows;
                }
            }
        }
    }

    // Lifts the clip. After a failed fill or stroke the context stays in its
    // error state and this does nothing, which is fine: GTK discards the
    // context once the draw function returns.
    let _ = cr.restore();
}
