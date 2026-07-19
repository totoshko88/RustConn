//! Adaptive toolbar overflow for embedded protocol viewers.
//!
//! Embedded RDP/VNC/SPICE widgets build a horizontal `embedded-toolbar`
//! [`gtk4::Box`]. In a narrow split panel — or a small/narrow application window
//! — that toolbar clips. [`ToolbarOverflow`] watches the viewer's drawing-area
//! width and, below a documented breakpoint, folds the *secondary* actions into
//! a "⋯" overflow [`gtk4::MenuButton`] popover while the *primary* actions
//! (Fit resolution, Ctrl+Alt+Del) stay directly reachable.
//!
//! The existing button widgets are **reparented** between the toolbar and the
//! popover — never rebuilt — so every signal handler stays bound and every
//! action remains reachable at any width (R12.4). This behaviour is a property
//! of the widget itself, so it works identically in a split panel and in a
//! shrunk single-tab window.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, DrawingArea, MenuButton, Orientation, Popover, Widget};

use crate::i18n::i18n;

/// Collapse breakpoint for the RDP toolbar, in drawing-area pixels.
///
/// RDP carries six secondary actions (Copy, Paste, Autotype, Scripts, Quick
/// actions, Save Files) on top of the two primary ones, so the assembled
/// toolbar is the widest; below this width the secondary set is folded away.
// ponytail: eyeballed from the natural button widths at the default font; retune
// if the RDP toolbar gains/loses buttons or a theme changes button metrics.
pub const RDP_OVERFLOW_THRESHOLD_PX: i32 = 560;

/// Collapse breakpoint for the SPICE and VNC toolbars, in drawing-area pixels.
///
/// These carry only Copy + Paste as secondary actions, so they clip much later
/// than RDP and need a smaller breakpoint.
// ponytail: eyeballed; see `RDP_OVERFLOW_THRESHOLD_PX`.
pub const SPICE_VNC_OVERFLOW_THRESHOLD_PX: i32 = 360;

/// Width margin above the collapse breakpoint required before expanding again.
///
/// Two thresholds — collapse below `threshold`, expand at/above
/// `threshold + margin` — stop the overflow button flapping when a resize drag
/// settles right on the breakpoint.
const OVERFLOW_HYSTERESIS_PX: i32 = 48;

/// Adaptive overflow controller for an `embedded-toolbar` box.
///
/// Construct with [`ToolbarOverflow::new`] once the toolbar is fully assembled,
/// then wire the width watch with [`ToolbarOverflow::attach`]. The returned
/// `Rc` does not need to be stored: [`attach`](Self::attach) moves a clone into
/// the resize closure, which lives as long as the monitored drawing area.
pub struct ToolbarOverflow {
    /// The "⋯" button appended to the toolbar; hidden while everything fits.
    overflow_button: MenuButton,
    /// Vertical box inside the overflow popover holding the collapsed actions.
    overflow_box: GtkBox,
    /// The toolbar the secondary actions live in when expanded.
    toolbar: GtkBox,
    /// Secondary actions paired with the sibling they sit *after* when expanded
    /// (captured from the assembled toolbar so [`expand`](Self::expand) restores
    /// the original order). `None` means "first child".
    secondary: Vec<(Widget, Option<Widget>)>,
    /// Collapse breakpoint in drawing-area pixels.
    threshold_px: i32,
    /// Whether the secondary actions currently live in the overflow popover.
    collapsed: Cell<bool>,
}

impl ToolbarOverflow {
    /// Appends a hidden overflow button to `toolbar` and records the secondary actions.
    ///
    /// `secondary` is the ordered list of actions to fold into the popover when
    /// the toolbar is narrower than `threshold_px`; pass them in their toolbar
    /// order. Primary actions are simply left out of `secondary`. An empty
    /// `secondary` list makes the controller a no-op (the overflow button never
    /// appears).
    #[must_use]
    pub fn new(toolbar: &GtkBox, secondary: Vec<Widget>, threshold_px: i32) -> Rc<Self> {
        let overflow_box = GtkBox::new(Orientation::Vertical, 4);
        overflow_box.set_margin_start(6);
        overflow_box.set_margin_end(6);
        overflow_box.set_margin_top(6);
        overflow_box.set_margin_bottom(6);

        let popover = Popover::new();
        popover.set_child(Some(&overflow_box));

        let overflow_button = MenuButton::new();
        overflow_button.set_icon_name("view-more-symbolic");
        overflow_button.add_css_class("flat");
        overflow_button.set_tooltip_text(Some(&i18n("More actions")));
        overflow_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("More actions"))]);
        overflow_button.set_popover(Some(&popover));
        overflow_button.set_visible(false);
        toolbar.append(&overflow_button);

        // Capture each secondary widget's anchor (its preceding sibling) from the
        // fully-assembled toolbar. Because expand() processes the list in order,
        // an anchor that is itself a secondary widget is already back in place by
        // the time it is needed, so the original layout is restored exactly.
        let secondary = secondary
            .into_iter()
            .map(|w| {
                let anchor = w.prev_sibling();
                (w, anchor)
            })
            .collect();

        Rc::new(Self {
            overflow_button,
            overflow_box,
            toolbar: toolbar.clone(),
            secondary,
            threshold_px,
            collapsed: Cell::new(false),
        })
    }

    /// Wires the width watch to `resize_source` (the viewer's drawing area).
    ///
    /// The drawing area fills the panel/window, so its width is a reliable proxy
    /// for the available toolbar width. A clone of `self` is moved into the
    /// resize closure, keeping the controller alive for the widget's lifetime.
    pub fn attach(self: &Rc<Self>, resize_source: &DrawingArea) {
        let this = Rc::clone(self);
        resize_source.connect_resize(move |_, width, _| {
            this.update(width);
        });
    }

    /// Collapses or expands the secondary actions for the current `width`.
    fn update(&self, width: i32) {
        if self.secondary.is_empty() {
            return;
        }
        if self.collapsed.get() {
            if width >= self.threshold_px + OVERFLOW_HYSTERESIS_PX {
                self.expand();
            }
        } else if width < self.threshold_px {
            self.collapse();
        }
    }

    /// Moves the secondary actions into the popover and reveals the overflow button.
    fn collapse(&self) {
        for (widget, _) in &self.secondary {
            self.toolbar.remove(widget);
            self.overflow_box.append(widget);
        }
        self.overflow_button.set_visible(true);
        self.collapsed.set(true);
    }

    /// Moves the secondary actions back into the toolbar and hides the overflow button.
    fn expand(&self) {
        for (widget, anchor) in &self.secondary {
            self.overflow_box.remove(widget);
            self.toolbar.insert_child_after(widget, anchor.as_ref());
        }
        self.overflow_button.set_visible(false);
        self.collapsed.set(false);
    }
}
