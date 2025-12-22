//! Floating controls component for remote sessions
//!
//! This module provides the `FloatingControls` struct that displays
//! overlay control buttons for remote desktop sessions (VNC, RDP, SPICE).
//!
//! # Requirements Coverage
//!
//! - Requirement 5.1: Overlay floating control buttons using `GtkOverlay`
//! - Requirement 5.2: Show controls with fade-in animation on hover
//! - Requirement 5.6: Auto-hide controls after configurable timeout

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Orientation, Revealer, RevealerTransitionType, Widget};
use std::cell::RefCell;
use std::rc::Rc;

/// Default auto-hide timeout in milliseconds
const DEFAULT_AUTO_HIDE_TIMEOUT_MS: u32 = 3000;

/// Callback type for button actions
type ActionCallback = Box<dyn Fn() + 'static>;

/// Floating control bar for remote sessions
///
/// Provides overlay controls for session management including:
/// - Disconnect button to terminate the session
/// - Fullscreen toggle button
/// - Settings button for session-specific options
///
/// The controls automatically hide after a configurable timeout
/// and show on mouse hover.
///
/// # Example
///
/// ```ignore
/// use rustconn::floating_controls::FloatingControls;
///
/// let controls = FloatingControls::new();
///
/// // Connect callbacks
/// controls.connect_disconnect(|| {
///     println!("Disconnect clicked");
/// });
///
/// controls.connect_fullscreen(|| {
///     println!("Fullscreen toggled");
/// });
///
/// // Add to overlay
/// overlay.add_overlay(controls.widget());
/// ```
pub struct FloatingControls {
    /// Main container box
    container: GtkBox,
    /// Revealer for show/hide animation
    revealer: Revealer,
    /// Disconnect button
    disconnect_btn: Button,
    /// Fullscreen toggle button
    fullscreen_btn: Button,
    /// Settings button
    settings_btn: Button,
    /// Auto-hide timeout in milliseconds
    auto_hide_timeout_ms: Rc<RefCell<u32>>,
    /// Current auto-hide timer source ID
    auto_hide_source: Rc<RefCell<Option<gtk4::glib::SourceId>>>,
    /// Whether fullscreen is currently active
    fullscreen_active: Rc<RefCell<bool>>,
    /// Disconnect callback
    disconnect_callback: Rc<RefCell<Option<ActionCallback>>>,
    /// Fullscreen callback
    fullscreen_callback: Rc<RefCell<Option<ActionCallback>>>,
    /// Settings callback
    settings_callback: Rc<RefCell<Option<ActionCallback>>>,
}

impl FloatingControls {
    /// Creates a new floating controls widget
    ///
    /// The controls are created in a hidden state and positioned
    /// at the top-center of the overlay.
    #[must_use]
    pub fn new() -> Self {
        // Create the revealer for animation
        let revealer = Revealer::new();
        revealer.set_transition_type(RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(200);
        revealer.set_reveal_child(false);

        // Create the button container
        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.add_css_class("floating-controls");
        button_box.set_halign(Align::Center);
        button_box.set_margin_start(12);
        button_box.set_margin_end(12);
        button_box.set_margin_top(8);
        button_box.set_margin_bottom(8);

        // Create disconnect button
        let disconnect_btn = Button::from_icon_name("window-close-symbolic");
        disconnect_btn.set_tooltip_text(Some("Disconnect"));
        disconnect_btn.add_css_class("floating-control-button");
        disconnect_btn.add_css_class("destructive-action");

        // Create fullscreen button
        let fullscreen_btn = Button::from_icon_name("view-fullscreen-symbolic");
        fullscreen_btn.set_tooltip_text(Some("Toggle Fullscreen"));
        fullscreen_btn.add_css_class("floating-control-button");

        // Create settings button
        let settings_btn = Button::from_icon_name("emblem-system-symbolic");
        settings_btn.set_tooltip_text(Some("Session Settings"));
        settings_btn.add_css_class("floating-control-button");

        // Add buttons to container
        button_box.append(&disconnect_btn);
        button_box.append(&fullscreen_btn);
        button_box.append(&settings_btn);

        // Set up revealer
        revealer.set_child(Some(&button_box));

        // Create outer container for positioning
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_halign(Align::Center);
        container.set_valign(Align::Start);
        container.append(&revealer);

        let controls = Self {
            container,
            revealer,
            disconnect_btn,
            fullscreen_btn,
            settings_btn,
            auto_hide_timeout_ms: Rc::new(RefCell::new(DEFAULT_AUTO_HIDE_TIMEOUT_MS)),
            auto_hide_source: Rc::new(RefCell::new(None)),
            fullscreen_active: Rc::new(RefCell::new(false)),
            disconnect_callback: Rc::new(RefCell::new(None)),
            fullscreen_callback: Rc::new(RefCell::new(None)),
            settings_callback: Rc::new(RefCell::new(None)),
        };

        // Connect button signals
        controls.setup_button_signals();

        // Set up hover detection for auto-show/hide
        controls.setup_hover_detection();

        controls
    }

    /// Sets up button click signal handlers
    fn setup_button_signals(&self) {
        // Disconnect button
        let callback = self.disconnect_callback.clone();
        self.disconnect_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *callback.borrow() {
                cb();
            }
        });

        // Fullscreen button
        let callback = self.fullscreen_callback.clone();
        let fullscreen_active = self.fullscreen_active.clone();
        let fullscreen_btn = self.fullscreen_btn.clone();
        self.fullscreen_btn.connect_clicked(move |_| {
            // Toggle fullscreen state
            let is_active = *fullscreen_active.borrow();
            *fullscreen_active.borrow_mut() = !is_active;

            // Update button icon based on new state (after toggle)
            if is_active {
                // Was active, now inactive - show fullscreen icon
                fullscreen_btn.set_icon_name("view-fullscreen-symbolic");
                fullscreen_btn.set_tooltip_text(Some("Toggle Fullscreen"));
            } else {
                // Was inactive, now active - show restore icon
                fullscreen_btn.set_icon_name("view-restore-symbolic");
                fullscreen_btn.set_tooltip_text(Some("Exit Fullscreen"));
            }

            if let Some(ref cb) = *callback.borrow() {
                cb();
            }
        });

        // Settings button
        let callback = self.settings_callback.clone();
        self.settings_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *callback.borrow() {
                cb();
            }
        });
    }

    /// Sets up hover detection for auto-show/hide behavior
    fn setup_hover_detection(&self) {
        let revealer = self.revealer.clone();
        let auto_hide_source = self.auto_hide_source.clone();
        let auto_hide_timeout_ms = self.auto_hide_timeout_ms.clone();

        // Create motion controller for hover detection
        let motion_controller = gtk4::EventControllerMotion::new();

        // Show on enter
        let revealer_enter = revealer.clone();
        let auto_hide_source_enter = auto_hide_source.clone();
        motion_controller.connect_enter(move |_, _, _| {
            // Cancel any pending hide timer
            if let Some(source_id) = auto_hide_source_enter.borrow_mut().take() {
                let () = source_id.remove(); // Ignore error if source already removed
            }
            revealer_enter.set_reveal_child(true);
        });

        // Start hide timer on leave (use the original clones, not redundant ones)
        motion_controller.connect_leave(move |_| {
            // Cancel any existing timer
            if let Some(source_id) = auto_hide_source.borrow_mut().take() {
                let () = source_id.remove(); // Ignore error if source already removed
            }

            let timeout_ms = *auto_hide_timeout_ms.borrow();
            if timeout_ms > 0 {
                let revealer_timeout = revealer.clone();
                let source_id = gtk4::glib::timeout_add_local_once(
                    std::time::Duration::from_millis(u64::from(timeout_ms)),
                    move || {
                        revealer_timeout.set_reveal_child(false);
                    },
                );
                *auto_hide_source.borrow_mut() = Some(source_id);
            }
        });

        self.container.add_controller(motion_controller);
    }

    /// Shows the controls with animation
    pub fn show(&self) {
        // Cancel any pending hide timer
        if let Some(source_id) = self.auto_hide_source.borrow_mut().take() {
            let () = source_id.remove(); // Ignore error if source already removed
        }
        self.revealer.set_reveal_child(true);
    }

    /// Hides the controls with animation
    pub fn hide(&self) {
        // Cancel any pending hide timer
        if let Some(source_id) = self.auto_hide_source.borrow_mut().take() {
            let () = source_id.remove(); // Ignore error if source already removed
        }
        self.revealer.set_reveal_child(false);
    }

    /// Returns whether the controls are currently visible
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.revealer.reveals_child()
    }

    /// Sets the auto-hide timeout in milliseconds
    ///
    /// Set to 0 to disable auto-hide.
    pub fn set_auto_hide_timeout(&self, ms: u32) {
        *self.auto_hide_timeout_ms.borrow_mut() = ms;
    }

    /// Returns the current auto-hide timeout in milliseconds
    #[must_use]
    pub fn auto_hide_timeout(&self) -> u32 {
        *self.auto_hide_timeout_ms.borrow()
    }

    /// Connects a callback for the disconnect button
    ///
    /// The callback is invoked when the user clicks the disconnect button.
    pub fn connect_disconnect<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.disconnect_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for the fullscreen button
    ///
    /// The callback is invoked when the user clicks the fullscreen toggle button.
    pub fn connect_fullscreen<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.fullscreen_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for the settings button
    ///
    /// The callback is invoked when the user clicks the settings button.
    pub fn connect_settings<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.settings_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Updates the fullscreen button to reflect the current state
    ///
    /// # Arguments
    ///
    /// * `active` - Whether fullscreen mode is currently active
    pub fn set_fullscreen_active(&self, active: bool) {
        *self.fullscreen_active.borrow_mut() = active;
        if active {
            self.fullscreen_btn.set_icon_name("view-restore-symbolic");
            self.fullscreen_btn
                .set_tooltip_text(Some("Exit Fullscreen"));
        } else {
            self.fullscreen_btn
                .set_icon_name("view-fullscreen-symbolic");
            self.fullscreen_btn
                .set_tooltip_text(Some("Toggle Fullscreen"));
        }
    }

    /// Returns whether fullscreen mode is currently active
    #[must_use]
    pub fn is_fullscreen_active(&self) -> bool {
        *self.fullscreen_active.borrow()
    }

    /// Returns the widget for adding to an overlay
    #[must_use]
    pub fn widget(&self) -> &Widget {
        self.container.upcast_ref()
    }

    /// Returns a reference to the disconnect button
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // GTK objects can't be const
    pub fn disconnect_button(&self) -> &Button {
        &self.disconnect_btn
    }

    /// Returns a reference to the fullscreen button
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // GTK objects can't be const
    pub fn fullscreen_button(&self) -> &Button {
        &self.fullscreen_btn
    }

    /// Returns a reference to the settings button
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // GTK objects can't be const
    pub fn settings_button(&self) -> &Button {
        &self.settings_btn
    }
}

impl Default for FloatingControls {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FloatingControls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FloatingControls")
            .field("visible", &self.is_visible())
            .field("fullscreen_active", &self.is_fullscreen_active())
            .field("auto_hide_timeout_ms", &self.auto_hide_timeout())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_auto_hide_timeout() {
        // Just verify the constant is reasonable
        assert_eq!(DEFAULT_AUTO_HIDE_TIMEOUT_MS, 3000);
    }
}

// ============================================================================
// Property-Based Tests for Overlay Controls
// ============================================================================

/// Module containing property-based tests for floating controls.
///
/// These tests validate Property 8 from the design document:
/// "Session overlay contains required controls"
///
/// Note: These tests require GTK initialization and are run separately
/// from the core library tests. They are marked with #[ignore] by default
/// and can be run with `cargo test -- --ignored --test-threads=1` when a display is available.
#[cfg(test)]
mod property_tests {
    use super::*;

    /// **Feature: native-protocol-embedding, Property 8: Session overlay contains required controls**
    /// **Validates: Requirements 5.1**
    ///
    /// This test validates that for any `FloatingControls` instance, the overlay SHALL contain:
    /// - A disconnect button with correct icon and styling
    /// - A fullscreen button with correct icon and styling
    /// - A settings button with correct icon and styling
    ///
    /// All sub-properties are tested in a single test function to ensure GTK
    /// initialization happens on the same thread as all widget operations.
    #[test]
    #[ignore = "Requires GTK display - run with: cargo test -p rustconn -- --ignored --test-threads=1"]
    fn prop_session_overlay_contains_required_controls() {
        // Check if we have a display available
        if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
            eprintln!("Skipping test: No display available");
            return;
        }

        // Try to initialize GTK
        if gtk4::init().is_err() {
            eprintln!("Skipping test: GTK initialization failed");
            return;
        }

        // ====================================================================
        // Test 1: FloatingControls::new() creates all required buttons
        // ====================================================================
        {
            let controls = FloatingControls::new();

            // Property: disconnect button exists and has correct icon
            let disconnect_btn = controls.disconnect_button();
            assert!(
                disconnect_btn.icon_name().is_some(),
                "Disconnect button must have an icon"
            );
            assert_eq!(
                disconnect_btn.icon_name().as_deref(),
                Some("window-close-symbolic"),
                "Disconnect button must have window-close-symbolic icon"
            );
            assert!(
                disconnect_btn.has_css_class("destructive-action"),
                "Disconnect button must have destructive-action CSS class"
            );

            // Property: fullscreen button exists and has correct icon
            let fullscreen_btn = controls.fullscreen_button();
            assert!(
                fullscreen_btn.icon_name().is_some(),
                "Fullscreen button must have an icon"
            );
            assert_eq!(
                fullscreen_btn.icon_name().as_deref(),
                Some("view-fullscreen-symbolic"),
                "Fullscreen button must have view-fullscreen-symbolic icon"
            );

            // Property: settings button exists and has correct icon
            let settings_btn = controls.settings_button();
            assert!(
                settings_btn.icon_name().is_some(),
                "Settings button must have an icon"
            );
            assert_eq!(
                settings_btn.icon_name().as_deref(),
                Some("emblem-system-symbolic"),
                "Settings button must have emblem-system-symbolic icon"
            );

            // Property: all buttons have the floating-control-button CSS class
            assert!(
                disconnect_btn.has_css_class("floating-control-button"),
                "Disconnect button must have floating-control-button CSS class"
            );
            assert!(
                fullscreen_btn.has_css_class("floating-control-button"),
                "Fullscreen button must have floating-control-button CSS class"
            );
            assert!(
                settings_btn.has_css_class("floating-control-button"),
                "Settings button must have floating-control-button CSS class"
            );

            // Property: all buttons have tooltips
            assert!(
                disconnect_btn.tooltip_text().is_some(),
                "Disconnect button must have a tooltip"
            );
            assert!(
                fullscreen_btn.tooltip_text().is_some(),
                "Fullscreen button must have a tooltip"
            );
            assert!(
                settings_btn.tooltip_text().is_some(),
                "Settings button must have a tooltip"
            );
        }

        // ====================================================================
        // Test 2: Default trait produces same result
        // ====================================================================
        {
            let controls: FloatingControls = FloatingControls::default();

            // Verify all three buttons exist with icons
            assert!(
                controls.disconnect_button().icon_name().is_some(),
                "Default FloatingControls must have disconnect button with icon"
            );
            assert!(
                controls.fullscreen_button().icon_name().is_some(),
                "Default FloatingControls must have fullscreen button with icon"
            );
            assert!(
                controls.settings_button().icon_name().is_some(),
                "Default FloatingControls must have settings button with icon"
            );
        }

        // ====================================================================
        // Test 3: Widget is valid GTK widget
        // ====================================================================
        {
            let controls = FloatingControls::new();
            let widget = controls.widget();

            // Property: widget can be cast to expected type (Box)
            assert!(
                widget.is::<GtkBox>(),
                "FloatingControls widget must be a GtkBox"
            );
        }

        // ====================================================================
        // Test 4: Multiple instances all have required controls
        // ====================================================================
        {
            // Create multiple instances to verify the property holds universally
            for i in 0..5 {
                let controls = FloatingControls::new();

                assert!(
                    controls.disconnect_button().icon_name().is_some(),
                    "Instance {i}: Disconnect button must have an icon"
                );
                assert!(
                    controls.fullscreen_button().icon_name().is_some(),
                    "Instance {i}: Fullscreen button must have an icon"
                );
                assert!(
                    controls.settings_button().icon_name().is_some(),
                    "Instance {i}: Settings button must have an icon"
                );
            }
        }
    }

    /// **Feature: native-protocol-embedding, Property 7: Fullscreen state toggle is idempotent pair**
    /// **Validates: Requirements 5.4**
    ///
    /// This test validates that for any session, toggling fullscreen twice SHALL return
    /// to the original state. This is an idempotent pair property where:
    /// - toggle(toggle(state)) == state
    ///
    /// The property is tested for both initial states (active and inactive).
    #[test]
    #[ignore = "Requires GTK display - run with: cargo test -p rustconn -- --ignored --test-threads=1"]
    fn prop_fullscreen_toggle_is_idempotent_pair() {
        // Check if we have a display available
        if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
            eprintln!("Skipping test: No display available");
            return;
        }

        // Try to initialize GTK
        if gtk4::init().is_err() {
            eprintln!("Skipping test: GTK initialization failed");
            return;
        }

        // ====================================================================
        // Test 1: Starting from inactive (false), toggle twice returns to inactive
        // ====================================================================
        {
            let controls = FloatingControls::new();

            // Initial state should be inactive (false)
            let initial_state = controls.is_fullscreen_active();
            assert!(
                !initial_state,
                "Initial fullscreen state should be inactive (false)"
            );

            // First toggle: false -> true
            controls.set_fullscreen_active(true);
            let after_first_toggle = controls.is_fullscreen_active();
            assert!(
                after_first_toggle,
                "After first toggle, fullscreen should be active (true)"
            );

            // Verify icon changed to restore
            assert_eq!(
                controls.fullscreen_button().icon_name().as_deref(),
                Some("view-restore-symbolic"),
                "Fullscreen button should show restore icon when active"
            );

            // Second toggle: true -> false
            controls.set_fullscreen_active(false);
            let after_second_toggle = controls.is_fullscreen_active();
            assert_eq!(
                after_second_toggle, initial_state,
                "After toggling twice, state should return to initial state"
            );

            // Verify icon changed back to fullscreen
            assert_eq!(
                controls.fullscreen_button().icon_name().as_deref(),
                Some("view-fullscreen-symbolic"),
                "Fullscreen button should show fullscreen icon when inactive"
            );
        }

        // ====================================================================
        // Test 2: Starting from active (true), toggle twice returns to active
        // ====================================================================
        {
            let controls = FloatingControls::new();

            // Set initial state to active
            controls.set_fullscreen_active(true);
            let initial_state = controls.is_fullscreen_active();
            assert!(
                initial_state,
                "Initial fullscreen state should be active (true) after setting"
            );

            // First toggle: true -> false
            controls.set_fullscreen_active(false);
            let after_first_toggle = controls.is_fullscreen_active();
            assert!(
                !after_first_toggle,
                "After first toggle, fullscreen should be inactive (false)"
            );

            // Second toggle: false -> true
            controls.set_fullscreen_active(true);
            let after_second_toggle = controls.is_fullscreen_active();
            assert_eq!(
                after_second_toggle, initial_state,
                "After toggling twice, state should return to initial state"
            );
        }

        // ====================================================================
        // Test 3: Property holds for multiple toggle cycles
        // ====================================================================
        {
            let controls = FloatingControls::new();

            // Test multiple toggle cycles
            for cycle in 0..10 {
                let state_before = controls.is_fullscreen_active();

                // Toggle twice
                controls.set_fullscreen_active(!state_before);
                controls.set_fullscreen_active(state_before);

                let state_after = controls.is_fullscreen_active();
                assert_eq!(
                    state_after, state_before,
                    "Cycle {cycle}: Double toggle should preserve state"
                );
            }
        }

        // ====================================================================
        // Test 4: Icon state is consistent with fullscreen state
        // ====================================================================
        {
            let controls = FloatingControls::new();

            // Test all state transitions maintain icon consistency
            let test_states = [false, true, false, true, true, false];

            for &target_state in &test_states {
                controls.set_fullscreen_active(target_state);

                let expected_icon = if target_state {
                    "view-restore-symbolic"
                } else {
                    "view-fullscreen-symbolic"
                };

                assert_eq!(
                    controls.fullscreen_button().icon_name().as_deref(),
                    Some(expected_icon),
                    "Icon should be '{expected_icon}' when fullscreen_active is {target_state}"
                );

                assert_eq!(
                    controls.is_fullscreen_active(),
                    target_state,
                    "is_fullscreen_active() should return {target_state}"
                );
            }
        }

        // ====================================================================
        // Test 5: Multiple instances maintain independent state
        // ====================================================================
        {
            let controls1 = FloatingControls::new();
            let controls2 = FloatingControls::new();

            // Set different states
            controls1.set_fullscreen_active(true);
            controls2.set_fullscreen_active(false);

            // Verify independence
            assert!(
                controls1.is_fullscreen_active(),
                "Controls1 should be active"
            );
            assert!(
                !controls2.is_fullscreen_active(),
                "Controls2 should be inactive"
            );

            // Toggle controls1 twice
            controls1.set_fullscreen_active(false);
            controls1.set_fullscreen_active(true);

            // Controls2 should be unaffected
            assert!(
                !controls2.is_fullscreen_active(),
                "Controls2 should still be inactive after toggling controls1"
            );
        }
    }
}
