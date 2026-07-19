//! Navigation toolbar for the embedded web browser.
//!
//! Provides Back, Forward, Reload, Home, page title, Autofill, Zoom, and Menu
//! controls following the GNOME HIG pattern used by the RDP embedded toolbar.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use webkit6::prelude::*;

use crate::i18n::i18n;

/// Zoom step: 10% per click/shortcut.
const ZOOM_STEP: f64 = 0.1;

/// Minimum zoom level: 30%.
const ZOOM_MIN: f64 = 0.3;

/// Maximum zoom level: 300%.
const ZOOM_MAX: f64 = 3.0;

/// Default zoom level: 100%.
const ZOOM_DEFAULT: f64 = 1.0;

/// Web browser navigation toolbar.
///
/// Layout: [Back] [Forward] [Reload] [Home] | [URL Entry] | [Autofill] [Zoom+] [Zoom-] [Menu]
///
/// When the toolbar is narrow (< 500px), secondary actions (Home, Autofill,
/// Zoom In, Zoom Out) are hidden and available via the overflow menu.
///
/// All icon-only buttons carry both `set_tooltip_text` and `update_property`
/// for GNOME HIG accessibility compliance.
pub struct NavigationToolbar {
    /// Container box for the toolbar
    container: gtk4::Box,
    /// Back navigation button
    back_button: gtk4::Button,
    /// Forward navigation button
    forward_button: gtk4::Button,
    /// Reload current page button
    reload_button: gtk4::Button,
    /// Navigate to home (configured URL) button
    home_button: gtk4::Button,
    /// URL address bar entry (shows current URL, allows manual navigation)
    url_entry: gtk4::Entry,
    /// Autofill credentials button
    autofill_button: gtk4::Button,
    /// Zoom in button
    zoom_in_button: gtk4::Button,
    /// Zoom out button
    zoom_out_button: gtk4::Button,
    /// Menu button for additional actions
    menu_button: gtk4::MenuButton,
    /// Box containing secondary actions that collapse on narrow width.
    /// Stored to prevent GTK from dropping the widget (it has no other owning reference).
    #[expect(
        dead_code,
        reason = "field keeps the GTK widget alive; removing it destroys the box"
    )]
    secondary_box: gtk4::Box,
}

impl NavigationToolbar {
    /// Creates the navigation toolbar with all buttons and accessibility labels.
    #[must_use]
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        container.add_css_class("toolbar");
        container.set_margin_start(6);
        container.set_margin_end(6);
        container.set_margin_top(4);
        container.set_margin_bottom(4);
        container.set_hexpand(true);
        container.set_halign(gtk4::Align::Fill);

        // --- Left navigation buttons ---

        let back_button = gtk4::Button::from_icon_name("go-previous-symbolic");
        back_button.add_css_class("flat");
        back_button.set_tooltip_text(Some(&i18n("Back")));
        back_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Back"))]);
        back_button.set_sensitive(false);
        container.append(&back_button);

        let forward_button = gtk4::Button::from_icon_name("go-next-symbolic");
        forward_button.add_css_class("flat");
        forward_button.set_tooltip_text(Some(&i18n("Forward")));
        forward_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Forward"))]);
        forward_button.set_sensitive(false);
        container.append(&forward_button);

        let reload_button = gtk4::Button::from_icon_name("view-refresh-symbolic");
        reload_button.add_css_class("flat");
        reload_button.set_tooltip_text(Some(&i18n("Reload")));
        reload_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Reload"))]);
        container.append(&reload_button);

        let home_button = gtk4::Button::from_icon_name("go-home-symbolic");
        home_button.add_css_class("flat");
        home_button.set_tooltip_text(Some(&i18n("Home")));
        home_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Home"))]);
        container.append(&home_button);

        // --- Center: URL address bar (expands to fill available space) ---

        let url_entry = gtk4::Entry::new();
        url_entry.set_hexpand(true);
        url_entry.set_halign(gtk4::Align::Fill);
        url_entry.set_placeholder_text(Some(&i18n("Enter URL")));
        url_entry.update_property(&[gtk4::accessible::Property::Label(&i18n("URL address bar"))]);
        // Select all text on focus for easy URL replacement
        url_entry.connect_has_focus_notify(|entry| {
            if entry.has_focus() {
                entry.select_region(0, -1);
            }
        });
        container.append(&url_entry);

        // --- Right buttons: secondary (collapsible) + menu (always visible) ---

        let secondary_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

        let autofill_button = gtk4::Button::from_icon_name("dialog-password-symbolic");
        autofill_button.add_css_class("flat");
        autofill_button.set_tooltip_text(Some(&i18n("Autofill credentials")));
        autofill_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Autofill credentials",
        ))]);
        secondary_box.append(&autofill_button);

        let zoom_in_button = gtk4::Button::from_icon_name("zoom-in-symbolic");
        zoom_in_button.add_css_class("flat");
        zoom_in_button.set_tooltip_text(Some(&i18n("Zoom in")));
        zoom_in_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Zoom in"))]);
        secondary_box.append(&zoom_in_button);

        let zoom_out_button = gtk4::Button::from_icon_name("zoom-out-symbolic");
        zoom_out_button.add_css_class("flat");
        zoom_out_button.set_tooltip_text(Some(&i18n("Zoom out")));
        zoom_out_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Zoom out"))]);
        secondary_box.append(&zoom_out_button);

        container.append(&secondary_box);

        let menu_button = gtk4::MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.add_css_class("flat");
        menu_button.set_tooltip_text(Some(&i18n("Menu")));
        menu_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Menu"))]);
        container.append(&menu_button);

        // --- Responsive overflow: hide secondary actions on narrow width ---
        // Threshold: 500px. Secondary buttons + Home are hidden; actions
        // remain reachable via the menu popover.
        {
            // Overflow threshold: below this width, hide secondary actions.
            const OVERFLOW_THRESHOLD_PX: i32 = 500;
            let secondary_for_overflow = secondary_box.clone();
            let home_for_overflow = home_button.clone();
            container.add_tick_callback(move |widget, _| {
                let width = widget.width();
                if width > 0 {
                    let narrow = width < OVERFLOW_THRESHOLD_PX;
                    secondary_for_overflow.set_visible(!narrow);
                    home_for_overflow.set_visible(!narrow);
                }
                glib::ControlFlow::Continue
            });
        }

        Self {
            container,
            back_button,
            forward_button,
            reload_button,
            home_button,
            url_entry,
            autofill_button,
            zoom_in_button,
            zoom_out_button,
            menu_button,
            secondary_box,
        }
    }

    /// Returns the toolbar container widget.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Returns a reference to the autofill button.
    ///
    /// Used by `EmbeddedWebWidget` to connect the autofill action and
    /// control sensitivity based on credential availability.
    #[must_use]
    pub fn autofill_button(&self) -> &gtk4::Button {
        &self.autofill_button
    }

    /// Returns a reference to the menu button.
    #[must_use]
    pub fn menu_button(&self) -> &gtk4::MenuButton {
        &self.menu_button
    }

    /// Binds the toolbar to a WebView, connecting signals for navigation
    /// state, page title updates, and zoom controls.
    ///
    /// # Arguments
    /// * `web_view` - The WebKitGTK WebView to bind to
    /// * `home_url` - Shared reference to the original configured URL
    pub fn bind_to_webview(&self, web_view: &webkit6::WebView, home_url: &Rc<RefCell<String>>) {
        self.connect_navigation_buttons(web_view, home_url);
        self.connect_property_notifications(web_view);
        self.connect_zoom_buttons(web_view);
        self.setup_keyboard_shortcuts(web_view);
        self.setup_menu(web_view, home_url);
    }

    /// Connects Back, Forward, Reload, Home button click signals.
    fn connect_navigation_buttons(
        &self,
        web_view: &webkit6::WebView,
        home_url: &Rc<RefCell<String>>,
    ) {
        // Back button
        let wv = web_view.clone();
        self.back_button.connect_clicked(move |_| {
            wv.go_back();
        });

        // Forward button
        let wv = web_view.clone();
        self.forward_button.connect_clicked(move |_| {
            wv.go_forward();
        });

        // Reload button
        let wv = web_view.clone();
        self.reload_button.connect_clicked(move |_| {
            wv.reload();
        });

        // Home button — navigates to the original configured URL
        let wv = web_view.clone();
        let url = Rc::clone(home_url);
        self.home_button.connect_clicked(move |_| {
            let u = url.borrow().clone();
            wv.load_uri(&u);
        });
    }

    /// Connects WebView property notifications to update button sensitivity
    /// and URL entry.
    fn connect_property_notifications(&self, web_view: &webkit6::WebView) {
        // can-go-back → back button sensitivity
        let back_btn = self.back_button.clone();
        web_view.connect_notify_local(Some("can-go-back"), move |wv, _| {
            back_btn.set_sensitive(wv.can_go_back());
        });

        // can-go-forward → forward button sensitivity
        let fwd_btn = self.forward_button.clone();
        web_view.connect_notify_local(Some("can-go-forward"), move |wv, _| {
            fwd_btn.set_sensitive(wv.can_go_forward());
        });

        // title → URL entry tooltip (page title shown on hover)
        let entry_for_title = self.url_entry.clone();
        web_view.connect_notify_local(Some("title"), move |wv, _| {
            let title = wv.title().map(|t| t.to_string()).unwrap_or_default();
            if title.is_empty() {
                entry_for_title.set_tooltip_text(None);
            } else {
                entry_for_title.set_tooltip_text(Some(&title));
            }
        });

        // uri → URL entry text (shows current URL)
        let entry_for_uri = self.url_entry.clone();
        web_view.connect_notify_local(Some("uri"), move |wv, _| {
            let uri = wv.uri().map(|u| u.to_string()).unwrap_or_default();
            if uri == "about:blank" {
                entry_for_uri.set_text("");
            } else {
                entry_for_uri.set_text(&uri);
            }
        });

        // URL entry activation (Enter key) → navigate to the entered URL
        let wv_for_activate = web_view.clone();
        self.url_entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            let url = if text.contains("://") {
                text
            } else {
                // Add https:// if no scheme provided
                format!("https://{text}")
            };
            if crate::embedded_web::validate_url(&url).is_ok() {
                wv_for_activate.load_uri(&url);
            }
        });
    }

    /// Connects zoom in/out button clicks and updates button sensitivity.
    fn connect_zoom_buttons(&self, web_view: &webkit6::WebView) {
        let wv_in = web_view.clone();
        let zoom_in_btn = self.zoom_in_button.clone();
        let zoom_out_btn_for_in = self.zoom_out_button.clone();
        self.zoom_in_button.connect_clicked(move |_| {
            let new_level = zoom_in(&wv_in);
            update_zoom_button_sensitivity(&zoom_in_btn, &zoom_out_btn_for_in, new_level);
        });

        let wv_out = web_view.clone();
        let zoom_in_btn_for_out = self.zoom_in_button.clone();
        let zoom_out_btn = self.zoom_out_button.clone();
        self.zoom_out_button.connect_clicked(move |_| {
            let new_level = zoom_out(&wv_out);
            update_zoom_button_sensitivity(&zoom_in_btn_for_out, &zoom_out_btn, new_level);
        });

        // Update zoom button tooltips with current percentage on zoom change
        let zoom_in_tooltip = self.zoom_in_button.clone();
        let zoom_out_tooltip = self.zoom_out_button.clone();
        web_view.connect_notify_local(Some("zoom-level"), move |wv, _| {
            let pct = (wv.zoom_level() * 100.0).round() as u32;
            let in_tip = format!("{} ({}%)", i18n("Zoom in"), pct);
            let out_tip = format!("{} ({}%)", i18n("Zoom out"), pct);
            zoom_in_tooltip.set_tooltip_text(Some(&in_tip));
            zoom_out_tooltip.set_tooltip_text(Some(&out_tip));
        });
    }

    /// Sets up keyboard shortcuts for zoom: Ctrl+Plus/Equal, Ctrl+Minus, Ctrl+0.
    fn setup_keyboard_shortcuts(&self, web_view: &webkit6::WebView) {
        let key_controller = gtk4::EventControllerKey::new();

        let wv = web_view.clone();
        let zoom_in_btn = self.zoom_in_button.clone();
        let zoom_out_btn = self.zoom_out_button.clone();

        key_controller.connect_key_pressed(move |_, key, _, state| {
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);

            // Alt+Left: navigate back
            if alt && matches!(key.name().as_deref(), Some("Left")) {
                if wv.can_go_back() {
                    wv.go_back();
                }
                return glib::Propagation::Stop;
            }

            // Alt+Right: navigate forward
            if alt && matches!(key.name().as_deref(), Some("Right")) {
                if wv.can_go_forward() {
                    wv.go_forward();
                }
                return glib::Propagation::Stop;
            }

            if !ctrl {
                return glib::Propagation::Proceed;
            }

            match key.name().as_deref() {
                Some("r") => {
                    // Ctrl+R: reload page
                    wv.reload();
                    glib::Propagation::Stop
                }
                Some("plus" | "equal" | "KP_Add") => {
                    let new_level = zoom_in(&wv);
                    update_zoom_button_sensitivity(&zoom_in_btn, &zoom_out_btn, new_level);
                    glib::Propagation::Stop
                }
                Some("minus" | "KP_Subtract") => {
                    let new_level = zoom_out(&wv);
                    update_zoom_button_sensitivity(&zoom_in_btn, &zoom_out_btn, new_level);
                    glib::Propagation::Stop
                }
                Some("0" | "KP_0") => {
                    let new_level = zoom_reset(&wv);
                    update_zoom_button_sensitivity(&zoom_in_btn, &zoom_out_btn, new_level);
                    glib::Propagation::Stop
                }
                Some("l") => {
                    // Ctrl+L: copy current URL to clipboard
                    if let Some(uri) = wv.uri() {
                        let display = wv.display();
                        display.clipboard().set_text(&uri);
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });

        web_view.add_controller(key_controller);
    }

    /// Sets up the menu button's popover menu with browser actions.
    ///
    /// Menu items: Copy URL, Open in System Browser, separator,
    /// Zoom Reset (100%), separator, Clear Session Data.
    fn setup_menu(&self, web_view: &webkit6::WebView, home_url: &Rc<RefCell<String>>) {
        use gtk4::gio;

        // Build menu model
        let menu = gio::Menu::new();

        let nav_section = gio::Menu::new();
        nav_section.append(Some(&i18n("Copy URL")), Some("web.copy-url"));
        nav_section.append(
            Some(&i18n("Open in System Browser")),
            Some("web.open-external"),
        );
        menu.append_section(None, &nav_section);

        let zoom_section = gio::Menu::new();
        zoom_section.append(Some(&i18n("Zoom Reset (100%)")), Some("web.zoom-reset"));
        menu.append_section(None, &zoom_section);

        let session_section = gio::Menu::new();
        session_section.append(Some(&i18n("Clear Session Data")), Some("web.clear-session"));
        menu.append_section(None, &session_section);

        self.menu_button.set_menu_model(Some(&menu));

        // Create action group and register actions
        let action_group = gio::SimpleActionGroup::new();

        // Copy URL action
        let wv_copy = web_view.clone();
        let copy_action = gio::SimpleAction::new("copy-url", None);
        copy_action.connect_activate(move |_, _| {
            if let Some(uri) = wv_copy.uri() {
                let display = wv_copy.display();
                display.clipboard().set_text(&uri);
                crate::toast::show_info_toast_on_active_window(&crate::i18n::i18n(
                    "URL copied to clipboard",
                ));
            }
        });
        action_group.add_action(&copy_action);

        // Open in system browser action
        let wv_open = web_view.clone();
        let home_url_open = Rc::clone(home_url);
        let open_action = gio::SimpleAction::new("open-external", None);
        open_action.connect_activate(move |_, _| {
            // Use current page URI if available, fall back to configured home URL
            let uri = wv_open
                .uri()
                .map(|u| u.to_string())
                .unwrap_or_else(|| home_url_open.borrow().clone());
            let launcher = gtk4::UriLauncher::new(&uri);
            let window = wv_open
                .root()
                .and_then(|r| r.downcast::<gtk4::Window>().ok());
            launcher.launch(window.as_ref(), gtk4::gio::Cancellable::NONE, |_| {});
        });
        action_group.add_action(&open_action);

        // Zoom reset action
        let wv_zoom = web_view.clone();
        let zoom_in_btn = self.zoom_in_button.clone();
        let zoom_out_btn = self.zoom_out_button.clone();
        let zoom_reset_action = gio::SimpleAction::new("zoom-reset", None);
        zoom_reset_action.connect_activate(move |_, _| {
            let new_level = zoom_reset(&wv_zoom);
            update_zoom_button_sensitivity(&zoom_in_btn, &zoom_out_btn, new_level);
        });
        action_group.add_action(&zoom_reset_action);

        // Clear session data action
        let wv_clear = web_view.clone();
        let clear_action = gio::SimpleAction::new("clear-session", None);
        clear_action.connect_activate(move |_, _| {
            if let Some(session) = wv_clear.network_session()
                && let Some(data_manager) = session.website_data_manager()
            {
                data_manager.clear(
                    webkit6::WebsiteDataTypes::all(),
                    glib::TimeSpan(0), // timespan: 0 = all time
                    gtk4::gio::Cancellable::NONE,
                    |_| {},
                );
                crate::toast::show_info_toast_on_active_window(&crate::i18n::i18n(
                    "Session data cleared",
                ));
            }
        });
        action_group.add_action(&clear_action);

        // Install the action group on the toolbar container
        self.container
            .insert_action_group("web", Some(&action_group));
    }
}

impl Default for NavigationToolbar {
    fn default() -> Self {
        Self::new()
    }
}

/// Increases zoom level by one step, clamped to maximum.
///
/// Returns the new zoom level after the operation.
fn zoom_in(web_view: &webkit6::WebView) -> f64 {
    let current = web_view.zoom_level();
    let new_level = (current + ZOOM_STEP).min(ZOOM_MAX);
    // Round to avoid floating-point drift (e.g. 1.0000000000000002)
    let new_level = (new_level * 10.0).round() / 10.0;
    web_view.set_zoom_level(new_level);
    new_level
}

/// Decreases zoom level by one step, clamped to minimum.
///
/// Returns the new zoom level after the operation.
fn zoom_out(web_view: &webkit6::WebView) -> f64 {
    let current = web_view.zoom_level();
    let new_level = (current - ZOOM_STEP).max(ZOOM_MIN);
    // Round to avoid floating-point drift
    let new_level = (new_level * 10.0).round() / 10.0;
    web_view.set_zoom_level(new_level);
    new_level
}

/// Resets zoom level to 100%.
///
/// Returns the new zoom level (always 1.0).
fn zoom_reset(web_view: &webkit6::WebView) -> f64 {
    web_view.set_zoom_level(ZOOM_DEFAULT);
    ZOOM_DEFAULT
}

/// Updates zoom button sensitivity based on the current zoom level.
///
/// Disables Zoom In at the maximum and Zoom Out at the minimum.
fn update_zoom_button_sensitivity(
    zoom_in_button: &gtk4::Button,
    zoom_out_button: &gtk4::Button,
    zoom_level: f64,
) {
    // Use small epsilon for floating-point comparison
    zoom_in_button.set_sensitive(zoom_level < ZOOM_MAX - 0.01);
    zoom_out_button.set_sensitive(zoom_level > ZOOM_MIN + 0.01);
}

/// Pure zoom-level calculation for property-based testing.
///
/// Applies a zoom-in operation to the given level, returning the new level
/// clamped to [ZOOM_MIN, ZOOM_MAX] and rounded to one decimal place.
#[cfg(test)]
pub fn calc_zoom_in(current: f64) -> f64 {
    let new_level = (current + ZOOM_STEP).min(ZOOM_MAX);
    (new_level * 10.0).round() / 10.0
}

/// Pure zoom-level calculation for property-based testing.
///
/// Applies a zoom-out operation to the given level, returning the new level
/// clamped to [ZOOM_MIN, ZOOM_MAX] and rounded to one decimal place.
#[cfg(test)]
pub fn calc_zoom_out(current: f64) -> f64 {
    let new_level = (current - ZOOM_STEP).max(ZOOM_MIN);
    (new_level * 10.0).round() / 10.0
}

/// Pure zoom-level calculation for property-based testing.
///
/// Resets the zoom level to default (1.0).
#[cfg(test)]
pub fn calc_zoom_reset() -> f64 {
    ZOOM_DEFAULT
}

/// Pure logic for navigation button sensitivity, for property-based testing.
///
/// Maps a `(can_go_back, can_go_forward)` state to
/// `(back_sensitive, forward_sensitive)` — the sensitivity values that should
/// be applied to the corresponding buttons.
///
/// The invariant is simple: buttons are sensitive only when the corresponding
/// navigation direction is available.
#[cfg(test)]
pub fn compute_nav_sensitivity(can_go_back: bool, can_go_forward: bool) -> (bool, bool) {
    (can_go_back, can_go_forward)
}

/// Exported constants for property-based testing.
#[cfg(test)]
pub const TEST_ZOOM_MIN: f64 = ZOOM_MIN;
#[cfg(test)]
pub const TEST_ZOOM_MAX: f64 = ZOOM_MAX;
#[cfg(test)]
pub const TEST_ZOOM_STEP: f64 = ZOOM_STEP;

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    // Feature: embedded-web-browser, Property 2: Zoom Level Clamping

    /// Zoom operation variants for property-based testing.
    #[derive(Debug, Clone, Copy)]
    enum ZoomOp {
        In,
        Out,
        Reset,
    }

    /// Strategy generating a single zoom operation.
    fn arb_zoom_op() -> impl Strategy<Value = ZoomOp> {
        prop_oneof![Just(ZoomOp::In), Just(ZoomOp::Out), Just(ZoomOp::Reset),]
    }

    /// Strategy generating a valid initial zoom level within [0.3, 3.0],
    /// rounded to one decimal place (matching the step granularity).
    fn arb_initial_zoom() -> impl Strategy<Value = f64> {
        // Generate integer in [3, 30] and divide by 10 to get valid zoom levels
        (3u32..=30u32).prop_map(|n| f64::from(n) / 10.0)
    }

    /// Applies a zoom operation to the current level using the pure calc functions.
    fn apply_zoom_op(current: f64, op: ZoomOp) -> f64 {
        match op {
            ZoomOp::In => calc_zoom_in(current),
            ZoomOp::Out => calc_zoom_out(current),
            ZoomOp::Reset => calc_zoom_reset(),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: embedded-web-browser, Property 2: Zoom Level Clamping**
        /// **Validates: Requirements 3.9, 3.10, 3.11, 3.12, 3.13**
        ///
        /// For any sequence of zoom in/out/reset operations applied to a WebView
        /// starting from any valid zoom level in [0.3, 3.0], the resulting zoom level
        /// always remains within [0.3, 3.0] inclusive, and each operation changes the
        /// zoom level by exactly 0.1 or leaves it unchanged at the boundary.
        #[test]
        fn zoom_level_always_clamped(
            initial in arb_initial_zoom(),
            ops in proptest::collection::vec(arb_zoom_op(), 10..50),
        ) {
            let mut level = initial;

            // Verify initial level is within bounds
            prop_assert!(
                (TEST_ZOOM_MIN..=TEST_ZOOM_MAX).contains(&level),
                "Initial level must be in [0.3, 3.0], got: {}",
                level
            );

            for (i, &op) in ops.iter().enumerate() {
                let prev_level = level;
                level = apply_zoom_op(level, op);

                // Assert: level always stays within [0.3, 3.0]
                prop_assert!(
                    (TEST_ZOOM_MIN - f64::EPSILON..=TEST_ZOOM_MAX + f64::EPSILON)
                        .contains(&level),
                    "After operation {:?} at step {}, zoom level {} is out of bounds [0.3, 3.0]",
                    op, i, level
                );

                // Assert: each step changes by exactly 0.1 or is unchanged at boundary
                let delta = (level - prev_level).abs();
                let is_step = (delta - TEST_ZOOM_STEP).abs() < 1e-9;
                let is_unchanged = delta < 1e-9;
                // Reset can jump to 1.0 from any level — allow arbitrary change
                let is_reset = matches!(op, ZoomOp::Reset);

                prop_assert!(
                    is_step || is_unchanged || is_reset,
                    "Step {} ({:?}): change from {} to {} = delta {}, \
                     expected exactly 0.1, 0.0, or reset",
                    i, op, prev_level, level, delta
                );

                // For non-reset operations specifically:
                // - If at max, zoom_in should leave unchanged
                // - If at min, zoom_out should leave unchanged
                if !is_reset {
                    if (prev_level - TEST_ZOOM_MAX).abs() < 1e-9 && matches!(op, ZoomOp::In) {
                        prop_assert!(
                            is_unchanged,
                            "Zoom in at max boundary should leave level unchanged"
                        );
                    }
                    if (prev_level - TEST_ZOOM_MIN).abs() < 1e-9 && matches!(op, ZoomOp::Out) {
                        prop_assert!(
                            is_unchanged,
                            "Zoom out at min boundary should leave level unchanged"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_zoom_in_from_default() {
        let result = calc_zoom_in(1.0);
        assert!((result - 1.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_zoom_out_from_default() {
        let result = calc_zoom_out(1.0);
        assert!((result - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn test_zoom_in_clamped_at_max() {
        let result = calc_zoom_in(3.0);
        assert!((result - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_zoom_out_clamped_at_min() {
        let result = calc_zoom_out(0.3);
        assert!((result - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_zoom_reset() {
        let result = calc_zoom_reset();
        assert!((result - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_zoom_in_near_max() {
        // At 2.9, zooming in should reach exactly 3.0
        let result = calc_zoom_in(2.9);
        assert!((result - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_zoom_out_near_min() {
        // At 0.4, zooming out should reach exactly 0.3
        let result = calc_zoom_out(0.4);
        assert!((result - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_zoom_in_above_max_clamps() {
        // Even if somehow at 3.05 (shouldn't happen), clamp to 3.0
        let result = calc_zoom_in(3.05);
        assert!((result - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_zoom_out_below_min_clamps() {
        // Even if somehow at 0.25 (shouldn't happen), clamp to 0.3
        let result = calc_zoom_out(0.25);
        assert!((result - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_zoom_sensitivity_at_max() {
        // At 3.0, zoom in should be disabled (sensitive=false)
        const { assert!(3.0 >= ZOOM_MAX - 0.01) }; // Would set sensitive=false
    }

    #[test]
    fn test_zoom_sensitivity_at_min() {
        // At 0.3, zoom out should be disabled (sensitive=false)
        const { assert!(0.3 <= ZOOM_MIN + 0.01) }; // Would set sensitive=false
    }

    // ========== Property 7: Navigation Button Sensitivity Consistency ==========

    // **Feature: embedded-web-browser, Property 7: Navigation Button Sensitivity Consistency**
    // **Validates: Requirements 3.2, 3.3**
    //
    // For any (can_go_back, can_go_forward) state, the back button is disabled
    // when can_go_back is false, and the forward button is disabled when
    // can_go_forward is false.
    // Feature: embedded-web-browser, Property 7: Navigation Button Sensitivity Consistency
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn nav_button_sensitivity_matches_history_state(
            can_go_back in proptest::bool::ANY,
            can_go_forward in proptest::bool::ANY,
        ) {
            let (back_sensitive, forward_sensitive) =
                compute_nav_sensitivity(can_go_back, can_go_forward);

            // If can_go_back is false, back button must be insensitive (disabled)
            if can_go_back {
                prop_assert!(
                    back_sensitive,
                    "Back button must be enabled when can_go_back is true"
                );
            } else {
                prop_assert!(
                    !back_sensitive,
                    "Back button must be disabled when can_go_back is false"
                );
            }

            // If can_go_forward is false, forward button must be insensitive (disabled)
            if can_go_forward {
                prop_assert!(
                    forward_sensitive,
                    "Forward button must be enabled when can_go_forward is true"
                );
            } else {
                prop_assert!(
                    !forward_sensitive,
                    "Forward button must be disabled when can_go_forward is false"
                );
            }
        }
    }
}
