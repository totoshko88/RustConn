//! Marks the prompt of a command that exited non-zero.
//!
//! The OSC 133 [`super::mark_registry::MarkRegistry`] records, per session, which
//! prompt rows ran a command that failed (see the shell-integration-osc133 spec).
//! This layer turns those rows into a visible cue: a short red bar in the left
//! margin of each failed prompt row that is currently on screen, drawn on a
//! transparent [`gtk4::DrawingArea`] over the terminal — the same mechanism
//! [`super::highlight_overlay`] uses, reusing its terminal-overlay plumbing
//! (`grid_origin`, `attach_to_host_overlay`, `next_frame_redraw`).
//!
//! It is deliberately minimal and non-blocking: a margin bar,
//! not a modal, not a full-row wash. A row-background tint was the stated
//! fallback if a margin bar proved unworkable; the margin bar works, so that is
//! what ships. The marker is purely a function of the registry, so it stays
//! empty — and invisible — on a session without shell integration and on a build
//! below `vte-0-78`, where the registry is never populated.
//!
//! Like the highlight layer, this owns what it adds to the terminal and removes
//! it on drop.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{DrawingArea, glib};
use rustconn_core::highlight::viewport_rows;
use vte4::Terminal;
use vte4::prelude::*;

use super::highlight_overlay::{attach_to_host_overlay, grid_origin, next_frame_redraw};
use super::mark_registry::MarkRegistry;

/// Width of the failure bar, in pixels, drawn in the row's left margin.
const BAR_WIDTH: f64 = 3.0;

/// Red used for the failure bar. A fixed colour rather than a theme lookup: the
/// cue must read the same against any terminal palette, and it is drawn at full
/// opacity over VTE's own left padding, not over glyphs.
const BAR_RGBA: (f64, f64, f64, f64) = (0.86, 0.20, 0.18, 0.95);

/// A transparent layer that paints a failure bar for each visible failed prompt.
///
/// Holds the terminal and adjustment weakly and disconnects every handler on
/// drop, so it never keeps the session alive.
pub(super) struct FailedMarkerOverlay {
    drawing_area: DrawingArea,
    terminal: glib::WeakRef<Terminal>,
    terminal_handlers: Vec<glib::SignalHandlerId>,
    adjustment: Option<glib::WeakRef<gtk4::Adjustment>>,
    adjustment_handlers: Vec<glib::SignalHandlerId>,
}

impl FailedMarkerOverlay {
    /// Creates the failure-marker layer for `terminal`, reading `registry`.
    pub(super) fn attach(terminal: &Terminal, registry: Rc<RefCell<MarkRegistry>>) -> Self {
        // Presentation: a decoration over the terminal, whose own text a
        // screen reader already reads; `accessible-role` is construct-only.
        let drawing_area = DrawingArea::builder()
            .accessible_role(gtk4::AccessibleRole::Presentation)
            .build();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        // Mouse events pass through to the terminal underneath.
        drawing_area.set_can_target(false);

        let term_weak = terminal.downgrade();
        let draw_registry = Rc::clone(&registry);
        drawing_area.set_draw_func(move |da, cr, _w, _h| {
            cr.set_operator(gtk4::cairo::Operator::Clear);
            if cr.paint().is_err() {
                return;
            }
            cr.set_operator(gtk4::cairo::Operator::Over);

            let Some(terminal) = term_weak.upgrade() else {
                return;
            };
            draw_failed(da, cr, &terminal, &draw_registry.borrow());
        });

        let mut terminal_handlers = Vec::with_capacity(4);
        // Repaint on the same signals the highlight layer tracks: text changes,
        // cursor moves (escape sequences that do not fire contents-changed), and
        // a cell-size change from a font/zoom change.
        let redraw = next_frame_redraw(&drawing_area);
        terminal_handlers.push(terminal.connect_contents_changed(move |_| redraw()));
        let redraw = next_frame_redraw(&drawing_area);
        terminal_handlers.push(terminal.connect_cursor_moved(move |_| redraw()));
        let redraw = next_frame_redraw(&drawing_area);
        terminal_handlers.push(terminal.connect_char_size_changed(move |_, _, _| redraw()));
        let redraw = next_frame_redraw(&drawing_area);
        terminal_handlers.push(terminal.connect_scale_factor_notify(move |_| redraw()));

        // Scrolling moves failed rows under the layer without changing text.
        let adjustment = terminal.vadjustment();
        let mut adjustment_handlers = Vec::with_capacity(2);
        if let Some(adjustment) = &adjustment {
            let redraw = next_frame_redraw(&drawing_area);
            adjustment_handlers.push(adjustment.connect_value_changed(move |_| redraw()));
            // `changed` fires when the adjustment's bounds move — including when
            // VTE trims scrollback and `lower()` (the oldest live row) rises.
            // Trim marks below it here so the registry never grows unbounded on
            // a long session that never triggers prompt navigation, and so a
            // scrolled-out row is never a stale marker. (Navigation also trims,
            // but that path only runs when the user jumps between prompts.)
            let redraw = next_frame_redraw(&drawing_area);
            let trim_registry = Rc::clone(&registry);
            adjustment_handlers.push(adjustment.connect_changed(move |adj| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "buffer rows fit i64; lower() is a small non-negative row index"
                )]
                let oldest = adj.lower() as i64;
                trim_registry.borrow_mut().trim_below(oldest);
                redraw();
            }));
        }

        // Follow the terminal when it is shown somewhere new (detach, split).
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
        }
    }
}

impl Drop for FailedMarkerOverlay {
    fn drop(&mut self) {
        if let Some(terminal) = self.terminal.upgrade() {
            for handler in self.terminal_handlers.drain(..) {
                terminal.disconnect(handler);
            }
        }
        if let Some(adjustment) = self.adjustment.as_ref().and_then(glib::WeakRef::upgrade) {
            for handler in self.adjustment_handlers.drain(..) {
                adjustment.disconnect(handler);
            }
        }
        if let Some(parent) = self.drawing_area.parent()
            && let Some(overlay) = parent.downcast_ref::<gtk4::Overlay>()
        {
            overlay.remove_overlay(&self.drawing_area);
        }
    }
}

/// Paints a left-margin bar for every failed prompt row VTE is showing now.
fn draw_failed(
    da: &DrawingArea,
    cr: &gtk4::cairo::Context,
    terminal: &Terminal,
    registry: &MarkRegistry,
) {
    if registry.is_empty() {
        return;
    }
    let row_count = terminal.row_count();
    let cell_h = terminal.char_height() as f64;
    if row_count <= 0 || cell_h <= 0.0 {
        return;
    }
    let Some((origin_x, origin_y)) = grid_origin(terminal, da) else {
        return;
    };

    let scroll_value = terminal.vadjustment().map_or(0.0, |adj| adj.value());
    let viewport = viewport_rows(scroll_value, cell_h, row_count);

    let first = viewport.first_row;
    let last = viewport.first_row.saturating_add(viewport.rows);

    cr.set_source_rgba(BAR_RGBA.0, BAR_RGBA.1, BAR_RGBA.2, BAR_RGBA.3);

    for buffer_row in registry.failed_rows() {
        if buffer_row < first || buffer_row >= last {
            continue; // not on screen
        }
        let visible_row = buffer_row - viewport.first_row;
        let y = (visible_row as f64).mul_add(cell_h, origin_y - viewport.y_offset);
        cr.rectangle(origin_x, y, BAR_WIDTH, cell_h);
        if cr.fill().is_err() {
            return;
        }
    }
}
