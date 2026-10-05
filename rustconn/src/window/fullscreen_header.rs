//! The main window's header bar and tab bar while the window is fullscreen
//! (issue #354).
//!
//! Fullscreen hides the window's chrome — the header bar and, since the
//! follow-up on the same issue, the tab bar under it — so an embedded RDP or VNC
//! session is not cut by fixed strips of application controls. Both rows live
//! in one [`gtk4::Revealer`], the first top bar of the main
//! [`adw::ToolbarView`]; entering fullscreen moves the tab bar into that
//! revealer under the header, and leaving it puts the tab bar back above the
//! tab view. The two rows therefore always appear and disappear together, as
//! one block, which is what GNOME Web does with its fullscreen tab bar.
//!
//! The banners stacked under the chrome in the same toolbar view stay where they
//! are, because two of them must never be missed — the group broadcast banner
//! (#329: keystrokes are reaching tabs the user cannot see) and the hardware-key
//! touch cue (#350: without it an unlock looks hung). That is why this hides its
//! own revealer rather than calling `set_reveal_top_bars(false)`, which takes
//! every top bar with it.
//!
//! The chrome comes back without leaving fullscreen in three ways:
//!
//! - the pointer touching the top edge of the screen, the pattern GNOME apps use
//!   for a fullscreen header; it slides away again a moment after the pointer
//!   moves below it, unless it is in use (see [`State::in_use`]);
//! - F10, the primary-menu key, which would otherwise have no visible button to
//!   open. It is left alone while keyboard passthrough has cleared the menu
//!   button's `primary` flag, so the key still reaches the remote session;
//! - switching tabs, which shows it briefly so a keyboard switch is not made
//!   blind.
//!
//! While fullscreen the content extends under the top bars, so revealing the
//! chrome overlays the session instead of shrinking it. An embedded RDP session
//! resizes the remote desktop on every allocation change, and chrome that
//! reflowed the content would trigger one on every hover. For the same reason
//! the tab bar is moved in and out of the chrome only when fullscreen changes —
//! that is one resize, which entering fullscreen causes anyway.
//!
//! Touch has no hover, so on a touch screen without a keyboard the top-edge
//! reveal does not fire and F11 remains the way out of fullscreen.
//!
//! The sidebar has a header bar of its own, which this revealer does not hold,
//! so entering fullscreen hides the sidebar too and leaving it brings the
//! sidebar back only if it was shown before. F9 still reveals it meanwhile.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk4::{gdk, glib};
use libadwaita as adw;

use crate::embedded_toolbar_overflow::contains_active_menu;
use crate::terminal::TerminalNotebook;

/// How close to the top edge, in logical pixels, the pointer has to come to
/// reveal the chrome. Small on purpose: the top rows of a remote desktop are
/// real targets (its own title bars, a browser's tab strip), and a wide hot zone
/// would cover them with the chrome every time the user aimed there.
const REVEAL_EDGE_PX: f64 = 2.0;

/// How long the chrome waits, once the pointer has moved below it, before it
/// slides away. With two rows it is easy to overshoot the bottom one by a few
/// pixels on the way to a tab; this lets the pointer come back without the
/// target vanishing under it.
const HIDE_DELAY: Duration = Duration::from_millis(300);

/// How long the chrome stays up after a tab switch with the pointer elsewhere —
/// long enough to read which tab is now selected.
const TAB_SWITCH_LINGER: Duration = Duration::from_millis(1500);

/// The slide duration, in milliseconds. Matches the floating session toolbar,
/// so the window's two kinds of revealed controls move at the same speed.
const SLIDE_MS: u32 = 150;

/// The block of window chrome that fullscreen hides: the header bar, and in
/// fullscreen the tab bar under it.
pub(super) struct Chrome {
    revealer: gtk4::Revealer,
    rows: gtk4::Box,
}

impl Chrome {
    /// Wraps `header_bar` in the revealer that is added to the toolbar view as
    /// its first top bar.
    ///
    /// Wrapping does not change the header's look: libadwaita styles a header
    /// bar or a tab bar anywhere inside a toolbar view's top bar, not only as its
    /// direct child.
    pub(super) fn new(header_bar: &adw::HeaderBar) -> Self {
        let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        rows.append(header_bar);

        // Slide rather than crossfade: a crossfading revealer keeps its full
        // height while hidden, which would leave an empty raised strip — and an
        // input-swallowing one — across the top of the session. The slide moves
        // only the top bars; the content extends under them in fullscreen, so
        // its allocation does not change during the animation.
        let revealer = gtk4::Revealer::new();
        revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(SLIDE_MS);
        revealer.set_reveal_child(true);
        revealer.set_child(Some(&rows));

        Self { revealer, rows }
    }

    /// The widget to add as the toolbar view's top bar.
    pub(super) fn widget(&self) -> &gtk4::Revealer {
        &self.revealer
    }
}

/// Wires the fullscreen chrome behaviour onto the main window.
///
/// Reacts to the window's `fullscreened` property rather than to the menu
/// action, so every path into fullscreen is covered — `win.toggle-fullscreen`,
/// F11 and a window-manager fullscreen alike. The current state is applied once
/// here as well, in case the window is already fullscreen.
pub(super) fn install(
    window: &adw::ApplicationWindow,
    toolbar_view: &adw::ToolbarView,
    split_view: &adw::OverlaySplitView,
    chrome: Chrome,
    menu_button: &gtk4::MenuButton,
    notebook: &TerminalNotebook,
) {
    // Observes drags over the window without taking part in them; see
    // `State::in_use`.
    let drop_motion = gtk4::DropControllerMotion::new();
    window.add_controller(drop_motion.clone());

    let state = Rc::new(State {
        window: window.downgrade(),
        toolbar_view: toolbar_view.clone(),
        split_view: split_view.downgrade(),
        sidebar_before_fullscreen: Cell::new(None),
        revealer: chrome.revealer,
        rows: chrome.rows,
        tab_bar: notebook.tab_bar().clone(),
        tab_bar_home: notebook.widget().clone(),
        drop_motion,
        // Starts "far below the top bars", so chrome opened with F10 before the
        // pointer has moved hides when its menu closes instead of staying up
        // indefinitely.
        last_pointer_y: Cell::new(f64::INFINITY),
        tab_menu_open: Cell::new(false),
        hide_timer: RefCell::new(None),
    });

    state.apply_fullscreen(window.is_fullscreen());
    {
        let state = Rc::clone(&state);
        window.connect_fullscreened_notify(move |win| {
            state.apply_fullscreen(win.is_fullscreen());
        });
    }

    // Capture phase: the session widgets underneath (VTE, the RDP/VNC drawing
    // area) consume pointer events in the bubble phase. A motion controller
    // never claims anything, so watching here does not take events from them.
    let motion = gtk4::EventControllerMotion::new();
    motion.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let state = Rc::clone(&state);
        motion.connect_motion(move |_, _x, y| {
            state.last_pointer_y.set(y);
            if !state.is_fullscreen() {
                return;
            }
            if y <= REVEAL_EDGE_PX {
                state.show(true);
            } else if state.pointer_below() {
                if state.revealer.reveals_child() {
                    state.schedule_hide_if_idle();
                }
            } else {
                state.cancel_hide();
            }
        });
    }
    {
        // Leaving the window — onto a monitor above, say — sends no further
        // motion, so the last height would stay at the top edge and keep the
        // chrome up. Count the pointer as below it instead, like a fresh start.
        // An open menu is a surface of its own, so moving into it is a leave
        // too; `in_use` keeps the chrome up meanwhile, and `enter` restores
        // the real height when the pointer comes back.
        let state = Rc::clone(&state);
        motion.connect_leave(move |_| {
            state.last_pointer_y.set(f64::INFINITY);
            if state.is_fullscreen() && state.revealer.reveals_child() {
                state.schedule_hide_if_idle();
            }
        });
    }
    {
        let state = Rc::clone(&state);
        motion.connect_enter(move |_, _x, y| state.last_pointer_y.set(y));
    }
    window.add_controller(motion);

    // F10 with the chrome hidden: show it at once and let the key carry on to
    // GTK's own primary-menu binding, which only finds a menu button that is
    // mapped. A sliding reveal maps its child on the first animation frame, too
    // late for the binding that runs right after this handler.
    let keys = gtk4::EventControllerKey::new();
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let state = Rc::clone(&state);
        let menu_button = menu_button.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let unmodified = (modifiers & gtk4::accelerator_get_default_mod_mask()).is_empty();
            if key == gdk::Key::F10
                && unmodified
                && menu_button.is_primary()
                && state.is_fullscreen()
            {
                state.show(false);
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);

    // Chrome opened for its menu goes away again when the menu closes, unless
    // the pointer is still up in the top bars.
    {
        let state = Rc::clone(&state);
        menu_button.connect_active_notify(move |button| {
            if !button.is_active() {
                state.hide_soon_if_pointer_below();
            }
        });
    }

    // The tab context menu is the tab view's own popover, not a menu button,
    // so `contains_active_menu` cannot see it. The view announces it instead:
    // `setup-menu` carries the page when the menu opens and `None` once it has
    // closed.
    {
        let state = Rc::clone(&state);
        notebook.tab_view().connect_setup_menu(move |_, page| {
            state.tab_menu_open.set(page.is_some());
            if page.is_none() {
                state.hide_soon_if_pointer_below();
            }
        });
    }

    // A tab switch shows the chrome for a moment. Switching from the keyboard
    // in fullscreen would otherwise give no sign of which tab is now selected.
    {
        let state = Rc::clone(&state);
        notebook.tab_view().connect_selected_page_notify(move |_| {
            if state.is_fullscreen() {
                state.show(true);
                state.schedule_hide(TAB_SWITCH_LINGER);
            }
        });
    }
}

/// Everything the fullscreen chrome handlers share.
struct State {
    window: glib::WeakRef<adw::ApplicationWindow>,
    toolbar_view: adw::ToolbarView,
    /// The window's split view; weak, it is the window's own descendant.
    split_view: glib::WeakRef<adw::OverlaySplitView>,
    /// Whether the sidebar was shown when fullscreen began; `None` outside
    /// fullscreen.
    sidebar_before_fullscreen: Cell<Option<bool>>,
    revealer: gtk4::Revealer,
    /// The revealer's child: the header bar, and in fullscreen the tab bar.
    rows: gtk4::Box,
    tab_bar: adw::TabBar,
    /// Where the tab bar lives outside fullscreen — the notebook's container,
    /// as its first child, above the tab view.
    tab_bar_home: gtk4::Box,
    drop_motion: gtk4::DropControllerMotion,
    /// Last pointer height over the window.
    last_pointer_y: Cell<f64>,
    tab_menu_open: Cell<bool>,
    hide_timer: RefCell<Option<glib::SourceId>>,
}

impl State {
    fn is_fullscreen(&self) -> bool {
        self.window.upgrade().is_some_and(|w| w.is_fullscreen())
    }

    /// Puts the chrome and the content layout into their fullscreen or
    /// windowed shape.
    ///
    /// The top-bar style has to change with the layout. Under the default `Flat`
    /// style a toolbar view's top bars have no background of their own — they
    /// rely on the content starting below them. Once the content extends under
    /// them, flat chrome is drawn straight over the session, which shows
    /// through it. `Raised` gives the top bars the opaque header-bar background
    /// and a shadow, so revealed chrome covers what is beneath it.
    ///
    /// Both directions switch without animation. Leaving fullscreen in
    /// particular must not slide: the content stops extending under the top
    /// bars at the same moment, so a sliding header would resize the session on
    /// every frame of the animation.
    fn apply_fullscreen(&self, fullscreen: bool) {
        self.cancel_hide();
        if fullscreen {
            self.toolbar_view
                .set_top_bar_style(adw::ToolbarStyle::Raised);
            self.toolbar_view.set_extend_content_to_top_edge(true);
            self.dock_tab_bar(true);
            self.set_revealed(false, false);
            if let Some(split_view) = self.split_view.upgrade() {
                self.sidebar_before_fullscreen
                    .set(Some(split_view.shows_sidebar()));
                split_view.set_show_sidebar(false);
            }
        } else {
            self.set_revealed(true, false);
            self.dock_tab_bar(false);
            if self.sidebar_before_fullscreen.take() == Some(true)
                && let Some(split_view) = self.split_view.upgrade()
            {
                split_view.set_show_sidebar(true);
            }
            self.toolbar_view.set_extend_content_to_top_edge(false);
            self.toolbar_view.set_top_bar_style(adw::ToolbarStyle::Flat);
        }
    }

    /// Moves the tab bar under the header (`true`) or back above the tab view.
    ///
    /// The tab bar stays bound to its tab view through `AdwTabBar:view`, and the
    /// `tab` action group the context menu uses is inserted on the tab bar
    /// itself, so both travel with it.
    fn dock_tab_bar(&self, in_chrome: bool) {
        let (from, to) = if in_chrome {
            (&self.tab_bar_home, &self.rows)
        } else {
            (&self.rows, &self.tab_bar_home)
        };
        if self.tab_bar.parent().as_ref() != Some(from.upcast_ref::<gtk4::Widget>()) {
            return;
        }
        from.remove(&self.tab_bar);
        if in_chrome {
            to.append(&self.tab_bar);
        } else {
            to.prepend(&self.tab_bar);
        }
    }

    /// Shows the chrome, sliding unless `animate` is false, and drops any
    /// pending hide.
    fn show(&self, animate: bool) {
        self.cancel_hide();
        self.set_revealed(true, animate);
    }

    fn set_revealed(&self, revealed: bool, animate: bool) {
        if animate {
            self.revealer.set_reveal_child(revealed);
        } else {
            // A zero duration makes the revealer jump straight to its target.
            let duration = self.revealer.transition_duration();
            self.revealer.set_transition_duration(0);
            self.revealer.set_reveal_child(revealed);
            self.revealer.set_transition_duration(duration);
        }
    }

    /// Whether the last pointer position is below everything stacked at the top
    /// of the window — the chrome while it is shown, plus any revealed banner.
    fn pointer_below(&self) -> bool {
        self.last_pointer_y.get() > f64::from(self.toolbar_view.top_bar_height())
    }

    /// Whether the chrome must stay up regardless of where the pointer is:
    ///
    /// - one of its menus is open — the header's menu buttons, or the tab
    ///   context menu;
    /// - a drag is in progress over the window, such as a tab being dragged out
    ///   of the tab bar into a split pane. Hiding the tab bar under a drag that
    ///   started there would take its source away mid-gesture.
    ///
    /// Keyboard focus is deliberately not a reason: GTK hands focus back to the
    /// menu button when its popover closes, so a focus rule would keep the
    /// chrome pinned after every menu use until the user clicked somewhere else.
    fn in_use(&self) -> bool {
        self.tab_menu_open.get()
            || self.drop_motion.contains_pointer()
            || contains_active_menu(self.rows.upcast_ref::<gtk4::Widget>())
    }

    /// Hides the chrome after [`HIDE_DELAY`] when a menu has just closed and the
    /// pointer is not up in the top bars.
    fn hide_soon_if_pointer_below(self: &Rc<Self>) {
        if self.is_fullscreen() && self.pointer_below() {
            self.schedule_hide_if_idle();
        }
    }

    /// Starts the [`HIDE_DELAY`] countdown unless one is already running — a
    /// longer [`TAB_SWITCH_LINGER`] is not cut short by pointer motion.
    fn schedule_hide_if_idle(self: &Rc<Self>) {
        if self.hide_timer.borrow().is_none() {
            self.schedule_hide(HIDE_DELAY);
        }
    }

    /// Hides the chrome after `delay`, if by then the pointer is below it and
    /// nothing is using it. A hide refused for those reasons is not retried
    /// here; the next pointer motion or menu close schedules a new one.
    fn schedule_hide(self: &Rc<Self>, delay: Duration) {
        self.cancel_hide();
        let state: Weak<Self> = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(delay, move || {
            let Some(state) = state.upgrade() else {
                return;
            };
            // The source has fired and is gone; forget it before anything can
            // try to remove it.
            state.hide_timer.borrow_mut().take();
            if state.is_fullscreen() && state.pointer_below() && !state.in_use() {
                state.set_revealed(false, true);
            }
        });
        *self.hide_timer.borrow_mut() = Some(source);
    }

    fn cancel_hide(&self) {
        if let Some(source) = self.hide_timer.borrow_mut().take() {
            source.remove();
        }
    }
}
