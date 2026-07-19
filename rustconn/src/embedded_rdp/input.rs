//! Input handler setup for the embedded RDP widget
//!
//! Contains keyboard, mouse, and scroll event handlers with coordinate
//! transformation between widget space and RDP framebuffer space.

use std::cell::RefCell;
#[cfg(feature = "rdp-embedded")]
use std::collections::HashMap;
use std::rc::Rc;

#[cfg(feature = "rdp-embedded")]
use gtk4::EventControllerFocus;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use gtk4::{
    EventControllerKey, EventControllerMotion, EventControllerScroll, EventControllerScrollFlags,
    GestureClick, gdk,
};
#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

use super::types::{RdpCommand, RdpConnectionState};

/// Sends a key event via IronRDP using the fallback chain: keycode → keyval → Unicode.
///
/// This fixes keyboard layout issues (e.g. German QWERTZ) where keyval-based
/// mapping produces wrong characters (#15).
#[cfg(feature = "rdp-embedded")]
fn send_ironrdp_key(
    keycode: u32,
    keyval: gdk::Key,
    pressed: bool,
    ironrdp_tx: &Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
) {
    use rustconn_core::rdp_client::{keycode_to_scancode, keyval_to_scancode, keyval_to_unicode};

    let gdk_keyval = keyval.into_glib();

    if let Some(scancode) = keycode_to_scancode(keycode) {
        if let Some(ref tx) = *ironrdp_tx.borrow() {
            let _ = tx.send(RdpClientCommand::KeyEvent {
                scancode: scancode.code,
                pressed,
                extended: scancode.extended,
            });
        }
    } else if let Some(scancode) = keyval_to_scancode(gdk_keyval) {
        if let Some(ref tx) = *ironrdp_tx.borrow() {
            let _ = tx.send(RdpClientCommand::KeyEvent {
                scancode: scancode.code,
                pressed,
                extended: scancode.extended,
            });
        }
    } else if let Some(ch) = keyval_to_unicode(gdk_keyval) {
        if let Some(ref tx) = *ironrdp_tx.borrow() {
            let _ = tx.send(RdpClientCommand::UnicodeEvent {
                character: ch,
                pressed,
            });
        }
    } else if pressed {
        tracing::warn!(
            keycode,
            keyval = format_args!("0x{:X}", gdk_keyval),
            "[IronRDP] Unknown key"
        );
    }
}

/// Releases every key currently tracked as pressed and clears the tracking set.
///
/// Invoked when the RDP widget loses keyboard focus. The compositor can grab a
/// key chord (e.g. GNOME `Super`+digit to switch workspace) before its release
/// reaches us, leaving the modifier stuck down in the remote session until a
/// reconnect (#193). Sending a synthetic release on focus-out clears that state.
#[cfg(feature = "rdp-embedded")]
fn release_all_keys(
    pressed_keys: &Rc<RefCell<HashMap<u32, gdk::Key>>>,
    using_ironrdp: bool,
    freerdp_thread: &Rc<RefCell<Option<super::FreeRdpThread>>>,
    ironrdp_tx: &Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
) {
    let keys: Vec<(u32, gdk::Key)> = pressed_keys.borrow_mut().drain().collect();
    for (keycode, keyval) in keys {
        if using_ironrdp {
            send_ironrdp_key(keycode, keyval, false, ironrdp_tx);
        } else if let Some(ref thread) = *freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::KeyEvent {
                keyval: keyval.into_glib(),
                pressed: false,
            });
        }
    }
}

impl super::EmbeddedRdpWidget {
    /// Sets up keyboard and mouse input handlers with coordinate transformation
    #[cfg(feature = "rdp-embedded")]
    pub(super) fn setup_input_handlers(&self) {
        // Keyboard input handler
        let key_controller = EventControllerKey::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();

        // Tracks keys currently held down (keycode -> keyval) so we can release
        // them if the widget loses focus mid-press (#193). Shared between the
        // press/release handlers and the focus controller below.
        let pressed_keys: Rc<RefCell<HashMap<u32, gdk::Key>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let pressed_keys_pressed = pressed_keys.clone();

        key_controller.connect_key_pressed(move |_controller, keyval, keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                pressed_keys_pressed.borrow_mut().insert(keycode, keyval);
                if using_ironrdp {
                    send_ironrdp_key(keycode, keyval, true, &ironrdp_tx);
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: true,
                    });
                }
                // Stop propagation so GTK doesn't also handle the key
                // (e.g. arrow keys moving widget focus instead of going to RDP)
                gdk::glib::Propagation::Stop
            } else {
                gdk::glib::Propagation::Proceed
            }
        });

        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let pressed_keys_released = pressed_keys.clone();

        key_controller.connect_key_released(move |_controller, keyval, keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            pressed_keys_released.borrow_mut().remove(&keycode);

            if embedded && current_state == RdpConnectionState::Connected {
                if using_ironrdp {
                    send_ironrdp_key(keycode, keyval, false, &ironrdp_tx);
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: false,
                    });
                }
            }
        });

        self.drawing_area.add_controller(key_controller);

        // Release any held keys when the widget loses keyboard focus, so a
        // modifier grabbed by the compositor (GNOME Super+digit workspace
        // switch) does not stay stuck down in the remote session (#193).
        let focus_controller = EventControllerFocus::new();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let pressed_keys_focus = pressed_keys.clone();

        focus_controller.connect_leave(move |_controller| {
            release_all_keys(
                &pressed_keys_focus,
                *is_ironrdp.borrow(),
                &freerdp_thread,
                &ironrdp_tx,
            );
        });

        self.drawing_area.add_controller(focus_controller);

        // Track current button state for motion events
        let button_state = Rc::new(RefCell::new(0u8));

        // Mouse motion handler with coordinate transformation
        let motion_controller = EventControllerMotion::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let button_state_motion = button_state.clone();
        let width_motion = self.width.clone();
        let height_motion = self.height.clone();
        let rdp_width_motion = self.rdp_width.clone();
        let rdp_height_motion = self.rdp_height.clone();

        motion_controller.connect_motion(move |_controller, x, y| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                let widget_w = f64::from(*width_motion.borrow());
                let widget_h = f64::from(*height_motion.borrow());
                let rdp_w = f64::from(*rdp_width_motion.borrow());
                let rdp_h = f64::from(*rdp_height_motion.borrow());

                let (rdp_x, rdp_y) = crate::embedded_rdp::ui::transform_widget_to_rdp(
                    x, y, widget_w, widget_h, rdp_w, rdp_h,
                );
                let buttons = *button_state_motion.borrow();

                if using_ironrdp {
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let _ = tx.send(RdpClientCommand::PointerEvent {
                            x: crate::utils::coord_to_u16(rdp_x),
                            y: crate::utils::coord_to_u16(rdp_y),
                            buttons,
                        });
                    }
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::MouseEvent {
                        x: crate::utils::coord_to_i32(rdp_x),
                        y: crate::utils::coord_to_i32(rdp_y),
                        button: u32::from(buttons),
                        pressed: false,
                    });
                }
            }
        });

        self.drawing_area.add_controller(motion_controller);

        // Mouse click handler with coordinate transformation
        let click_controller = GestureClick::new();
        click_controller.set_button(0);
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let button_state_press = button_state.clone();
        let width_press = self.width.clone();
        let height_press = self.height.clone();
        let rdp_width_press = self.rdp_width.clone();
        let rdp_height_press = self.rdp_height.clone();
        let drawing_area_press = self.drawing_area.clone();

        click_controller.connect_pressed(move |gesture, _n_press, x, y| {
            // Grab focus on click to receive keyboard events
            drawing_area_press.grab_focus();

            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                let button = gesture.current_button();

                let widget_w = f64::from(*width_press.borrow());
                let widget_h = f64::from(*height_press.borrow());
                let rdp_w = f64::from(*rdp_width_press.borrow());
                let rdp_h = f64::from(*rdp_height_press.borrow());

                let (rdp_x, rdp_y) = crate::embedded_rdp::ui::transform_widget_to_rdp(
                    x, y, widget_w, widget_h, rdp_w, rdp_h,
                );

                // Convert GTK button to RDP button mask
                let button_bit = crate::embedded_rdp::ui::gtk_button_to_rdp_mask(button);
                let buttons = *button_state_press.borrow() | button_bit;
                *button_state_press.borrow_mut() = buttons;

                if using_ironrdp {
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let rdp_button = crate::embedded_rdp::ui::gtk_button_to_rdp_button(button);
                        let _ = tx.send(RdpClientCommand::MouseButtonPress {
                            x: crate::utils::coord_to_u16(rdp_x),
                            y: crate::utils::coord_to_u16(rdp_y),
                            button: rdp_button,
                        });
                    }
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::MouseEvent {
                        x: crate::utils::coord_to_i32(rdp_x),
                        y: crate::utils::coord_to_i32(rdp_y),
                        button,
                        pressed: true,
                    });
                }
            }
        });

        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let button_state_release = button_state.clone();
        let width_release = self.width.clone();
        let height_release = self.height.clone();
        let rdp_width_release = self.rdp_width.clone();
        let rdp_height_release = self.rdp_height.clone();

        click_controller.connect_released(move |gesture, _n_press, x, y| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                let button = gesture.current_button();

                let widget_w = f64::from(*width_release.borrow());
                let widget_h = f64::from(*height_release.borrow());
                let rdp_w = f64::from(*rdp_width_release.borrow());
                let rdp_h = f64::from(*rdp_height_release.borrow());

                let (rdp_x, rdp_y) = crate::embedded_rdp::ui::transform_widget_to_rdp(
                    x, y, widget_w, widget_h, rdp_w, rdp_h,
                );

                let button_bit = crate::embedded_rdp::ui::gtk_button_to_rdp_mask(button);
                let buttons = *button_state_release.borrow() & !button_bit;
                *button_state_release.borrow_mut() = buttons;

                if using_ironrdp {
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let rdp_button = crate::embedded_rdp::ui::gtk_button_to_rdp_button(button);
                        let _ = tx.send(RdpClientCommand::MouseButtonRelease {
                            x: crate::utils::coord_to_u16(rdp_x),
                            y: crate::utils::coord_to_u16(rdp_y),
                            button: rdp_button,
                        });
                    }
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::MouseEvent {
                        x: crate::utils::coord_to_i32(rdp_x),
                        y: crate::utils::coord_to_i32(rdp_y),
                        button,
                        pressed: false,
                    });
                }
            }
        });

        self.drawing_area.add_controller(click_controller);

        // Mouse scroll handler for wheel events
        let scroll_controller = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();

        scroll_controller.connect_scroll(move |_controller, _dx, dy| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded
                && current_state == RdpConnectionState::Connected
                && using_ironrdp
                && let Some(ref tx) = *ironrdp_tx.borrow()
            {
                let wheel_delta = (-dy * 120.0) as i16;
                if wheel_delta != 0 {
                    let _ = tx.send(RdpClientCommand::WheelEvent {
                        horizontal: 0,
                        vertical: wheel_delta,
                    });
                }
            }

            gdk::glib::Propagation::Proceed
        });

        self.drawing_area.add_controller(scroll_controller);
    }

    /// Sets up keyboard and mouse input handlers (fallback when rdp-embedded is disabled)
    #[cfg(not(feature = "rdp-embedded"))]
    pub(super) fn setup_input_handlers(&self) {
        // Simplified handlers for FreeRDP-only mode
        let key_controller = EventControllerKey::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let freerdp_thread = self.freerdp_thread.clone();

        key_controller.connect_key_pressed(move |_controller, keyval, _keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: true,
                    });
                }
            }

            gdk::glib::Propagation::Proceed
        });

        self.drawing_area.add_controller(key_controller);
    }
}
