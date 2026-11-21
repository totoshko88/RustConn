//! Keyboard passthrough, shortcut inhibition and focus passthrough.
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

/// The `AdwTabView` shortcut set for a keyboard-passthrough state.
///
/// Passthrough disables every one of them so the chord reaches the session;
/// leaving it restores the full default set, which is what the TabView is built
/// with, so toggling twice is a no-op.
const fn tab_view_shortcuts_for_passthrough(enabled: bool) -> adw::TabViewShortcuts {
    if enabled {
        adw::TabViewShortcuts::NONE
    } else {
        adw::TabViewShortcuts::ALL_SHORTCUTS
    }
}

/// Whether the desktop's shortcuts should go to `viewer` while it has focus.
///
/// `hook` is the connection's own choice; `None` follows `auto_capture`, the
/// application's "Auto-capture shortcuts in remote desktop" setting.
/// Keyboard passthrough, which sends them in every case, is checked by the
/// callers.
fn viewer_takes_shortcuts(
    viewer: &gtk4::Widget,
    hook: Option<RdpKeyboardHook>,
    auto_capture: bool,
) -> bool {
    hook_takes_shortcuts(hook, auto_capture, || {
        viewer
            .root()
            .and_downcast::<gtk4::Window>()
            .is_some_and(|window| window.is_fullscreen())
    })
}

/// The decision behind [`viewer_takes_shortcuts`], with the window's full-screen
/// state asked for only when the hook depends on it.
fn hook_takes_shortcuts(
    hook: Option<RdpKeyboardHook>,
    auto_capture: bool,
    fullscreen: impl FnOnce() -> bool,
) -> bool {
    match hook {
        None => auto_capture,
        Some(RdpKeyboardHook::Local) => false,
        Some(RdpKeyboardHook::Remote) => true,
        Some(RdpKeyboardHook::FullscreenOnly) => fullscreen(),
    }
}

/// Inhibits or restores the compositor's shortcuts for the window `widget` is
/// in, so a detached session window asks for its own surface.
///
/// On Wayland this is `zwp_keyboard_shortcuts_inhibit_v1`, on X11 a keyboard
/// grab; GTK picks. It only has effect while that window has keyboard focus.
fn set_system_shortcuts_inhibited(widget: &gtk4::Widget, inhibit: bool) {
    let Some(toplevel) = widget
        .native()
        .and_then(|native| native.surface())
        .and_downcast::<gtk4::gdk::Toplevel>()
    else {
        return;
    };
    tracing::debug!(inhibit, "keyboard passthrough: system shortcuts");
    if inhibit {
        toplevel.inhibit_system_shortcuts(None::<&gtk4::gdk::Event>);
    } else {
        toplevel.restore_system_shortcuts();
    }
}

impl TerminalNotebook {
    /// Turns the TabView's built-in tab shortcuts off for keyboard passthrough.
    ///
    /// `AdwTabView` carries its own shortcut controller (Ctrl+Tab,
    /// Ctrl+Shift+Tab, Ctrl+Page Up/Down, Ctrl+Home/End,
    /// Ctrl+Shift+Page Up/Down/Home/End, Alt+0…9) with window-wide scope. It is not a `GApplication`
    /// accelerator, so clearing the accelerator table in passthrough mode does
    /// not reach it, and because it runs before the session widget's key
    /// controller it switched tabs instead of forwarding the chord (issue #356).
    ///
    /// Also hands the desktop's own shortcuts (Alt+Tab, Super, Super+number)
    /// to an embedded RDP or VNC viewer while one has focus; see
    /// [`Self::attach_shortcut_inhibit`].
    pub fn set_keyboard_passthrough(&self, enabled: bool) {
        let focused = self
            .focused_viewer
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade);
        let viewer_takes = focused.as_ref().is_some_and(|viewer| {
            viewer_takes_shortcuts(
                viewer,
                self.focused_hook.get(),
                self.auto_capture_shortcuts.get(),
            )
        });
        self.tab_view
            .set_shortcuts(tab_view_shortcuts_for_passthrough(enabled || viewer_takes));
        self.keyboard_passthrough.set(enabled);

        if let Some(viewer) = focused {
            set_system_shortcuts_inhibited(&viewer, enabled || viewer_takes);
        } else if !enabled {
            // The viewer may have gone away while it held the shortcuts.
            set_system_shortcuts_inhibited(self.container.upcast_ref(), false);
        }
    }

    /// Enables or disables automatic system shortcut capture for embedded
    /// remote desktop viewers (RDP and VNC) on focus.
    pub fn set_auto_capture_shortcuts(&self, enabled: bool) {
        self.auto_capture_shortcuts.set(enabled);
        if let Some(viewer) = self
            .focused_viewer
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
        {
            let should_inhibit = self.keyboard_passthrough.get()
                || viewer_takes_shortcuts(&viewer, self.focused_hook.get(), enabled);
            set_system_shortcuts_inhibited(&viewer, should_inhibit);
            if should_inhibit {
                self.tab_view.set_shortcuts(adw::TabViewShortcuts::NONE);
            } else {
                self.tab_view
                    .set_shortcuts(adw::TabViewShortcuts::ALL_SHORTCUTS);
            }
        }
    }

    /// Whether automatic system shortcut capture for remote desktop viewers is on.
    #[must_use]
    pub fn auto_capture_shortcuts(&self) -> bool {
        self.auto_capture_shortcuts.get()
    }

    /// Asks the compositor for the desktop shortcuts while `widget` (an
    /// embedded RDP or VNC viewer) has focus and passthrough or auto-capture
    /// is on, and gives them back when focus leaves (issue #356).
    ///
    /// `hook` is an RDP connection's own choice of where Windows key
    /// combinations go (`RdpConfig::keyboard_hook`), read on every focus change;
    /// `None`, or a cell holding `None`, follows auto-capture. "Only in full
    /// screen" also follows the window going in and out of full screen while
    /// the viewer has focus.
    ///
    /// This is the Wayland keyboard-shortcuts-inhibit request that an external
    /// FreeRDP window already makes. GNOME asks the user once whether to allow
    /// it, and Super+Esc always gives the shortcuts back. Terminals are left
    /// out on purpose: in an SSH tab Alt+Tab is expected to switch windows.
    pub(super) fn attach_shortcut_inhibit<W: IsA<gtk4::Widget>>(
        &self,
        widget: &W,
        hook: Option<Rc<std::cell::Cell<Option<RdpKeyboardHook>>>>,
    ) {
        let focus_ctrl = gtk4::EventControllerFocus::new();
        // The full-screen watch of a focused "Only in full screen" viewer.
        let fullscreen_watch: Rc<
            RefCell<Option<(glib::WeakRef<gtk4::Window>, glib::SignalHandlerId)>>,
        > = Rc::new(RefCell::new(None));
        let passthrough = self.keyboard_passthrough.clone();
        let auto_capture = self.auto_capture_shortcuts.clone();
        let focused = self.focused_viewer.clone();
        let focused_hook = self.focused_hook.clone();
        let tab_view = self.tab_view.clone();
        let watch = fullscreen_watch.clone();
        let viewer = widget.upcast_ref::<gtk4::Widget>().downgrade();
        focus_ctrl.connect_enter(move |_| {
            let Some(viewer) = viewer.upgrade() else {
                return;
            };
            *focused.borrow_mut() = Some(viewer.downgrade());
            let viewer_hook = hook.as_ref().and_then(|cell| cell.get());
            focused_hook.set(viewer_hook);
            if passthrough.get() || viewer_takes_shortcuts(&viewer, viewer_hook, auto_capture.get())
            {
                set_system_shortcuts_inhibited(&viewer, true);
                tab_view.set_shortcuts(adw::TabViewShortcuts::NONE);
            }
            if viewer_hook == Some(RdpKeyboardHook::FullscreenOnly)
                && let Some(window) = viewer.root().and_downcast::<gtk4::Window>()
            {
                let passthrough = passthrough.clone();
                let tab_view = tab_view.clone();
                let viewer = viewer.downgrade();
                let handler = window.connect_fullscreened_notify(move |window| {
                    let Some(viewer) = viewer.upgrade() else {
                        return;
                    };
                    let take = passthrough.get() || window.is_fullscreen();
                    set_system_shortcuts_inhibited(&viewer, take);
                    tab_view.set_shortcuts(tab_view_shortcuts_for_passthrough(take));
                });
                if let Some((old_window, old_handler)) =
                    watch.borrow_mut().replace((window.downgrade(), handler))
                    && let Some(old_window) = old_window.upgrade()
                {
                    old_window.disconnect(old_handler);
                }
            }
        });
        let passthrough = self.keyboard_passthrough.clone();
        let focused = self.focused_viewer.clone();
        let focused_hook = self.focused_hook.clone();
        let tab_view = self.tab_view.clone();
        let viewer = widget.upcast_ref::<gtk4::Widget>().downgrade();
        focus_ctrl.connect_leave(move |_| {
            *focused.borrow_mut() = None;
            focused_hook.set(None);
            if let Some((window, handler)) = fullscreen_watch.borrow_mut().take()
                && let Some(window) = window.upgrade()
            {
                window.disconnect(handler);
            }
            if let Some(viewer) = viewer.upgrade()
                && !passthrough.get()
            {
                set_system_shortcuts_inhibited(&viewer, false);
            }
            if !passthrough.get() {
                tab_view.set_shortcuts(adw::TabViewShortcuts::ALL_SHORTCUTS);
            }
        });
        widget.add_controller(focus_ctrl);
    }

    /// Attaches a focus controller that drives the `on_terminal_focus` callback
    /// (`true` on enter, `false` on leave).
    ///
    /// Used for the VTE terminal and the embedded RDP/VNC/SPICE viewers so the
    /// single-Ctrl accelerators are suspended while any of them has focus,
    /// keeping the behavior identical across protocols (issue #197).
    /// `EventControllerFocus` reports focus for the widget and its descendants,
    /// so attaching to the top-level viewer widget fires when any child gains
    /// focus.
    pub(super) fn attach_focus_passthrough<W: IsA<gtk4::Widget>>(&self, widget: &W) {
        let focus_ctrl = gtk4::EventControllerFocus::new();
        let on_focus_enter = self.on_terminal_focus.clone();
        focus_ctrl.connect_enter(move |_| {
            if let Some(cb) = on_focus_enter.borrow().as_ref() {
                cb(true);
            }
        });
        let on_focus_leave = self.on_terminal_focus.clone();
        focus_ctrl.connect_leave(move |_| {
            if let Some(cb) = on_focus_leave.borrow().as_ref() {
                cb(false);
            }
        });
        widget.add_controller(focus_ctrl);
    }
}

#[cfg(test)]
mod passthrough_shortcut_tests {
    use libadwaita as adw;
    use rustconn_core::models::RdpKeyboardHook;

    use super::{hook_takes_shortcuts, tab_view_shortcuts_for_passthrough};

    /// A connection's own choice wins over the app's auto-capture setting, and
    /// "only in full screen" follows the window.
    #[test]
    fn keyboard_hook_decides_where_shortcuts_go() {
        for auto in [true, false] {
            assert_eq!(hook_takes_shortcuts(None, auto, || unreachable!()), auto);
            assert!(!hook_takes_shortcuts(
                Some(RdpKeyboardHook::Local),
                auto,
                || true
            ));
            assert!(hook_takes_shortcuts(
                Some(RdpKeyboardHook::Remote),
                auto,
                || false
            ));
            for fullscreen in [true, false] {
                assert_eq!(
                    hook_takes_shortcuts(Some(RdpKeyboardHook::FullscreenOnly), auto, || {
                        fullscreen
                    }),
                    fullscreen
                );
            }
        }
    }

    /// Issue #356: Ctrl+Shift+Tab and friends switched tabs in passthrough.
    #[test]
    fn passthrough_disables_every_tab_view_shortcut() {
        assert_eq!(
            tab_view_shortcuts_for_passthrough(true),
            adw::TabViewShortcuts::NONE
        );
    }

    /// Leaving passthrough must give back exactly the set `adw::TabView::new()`
    /// starts with — Ctrl+Tab, Ctrl+Page Up/Down and Alt+digit keep working.
    #[test]
    fn leaving_passthrough_restores_the_default_set() {
        assert_eq!(
            tab_view_shortcuts_for_passthrough(false),
            adw::TabViewShortcuts::ALL_SHORTCUTS
        );
    }
}
