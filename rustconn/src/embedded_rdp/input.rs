//! Input handler setup for the embedded RDP widget
//!
//! Contains keyboard, mouse, and scroll event handlers with coordinate
//! transformation between widget space and RDP framebuffer space.

use gtk4::gdk;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use gtk4::{
    EventControllerKey, EventControllerMotion, EventControllerScroll, EventControllerScrollFlags,
    GestureClick,
};
use std::cell::RefCell;
use std::rc::Rc;

use super::types::{RdpCommand, RdpConnectionState};

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

impl super::EmbeddedRdpWidget {
    /// Sets up keyboard and mouse input handlers with coordinate transformation
    #[cfg(feature = "rdp-embedded")]
    pub(super) fn setup_input_handlers(&self) {
        use rustconn_core::rdp_client::{
            keycode_to_scancode, keyval_to_scancode, keyval_to_unicode,
        };

        // Keyboard input handler
        let key_controller = EventControllerKey::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();

        key_controller.connect_key_pressed(move |_controller, keyval, keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                if using_ironrdp {
                    // Use hardware keycode for layout-independent scancode mapping.
                    // This fixes keyboard layout issues (e.g. German QWERTZ) where
                    // keyval-based mapping produces wrong characters (#15).
                    // Fallback chain: keycode → keyval → Unicode
                    let hw_keycode = keycode;
                    let gdk_keyval = keyval.into_glib();

                    if let Some(scancode) = keycode_to_scancode(hw_keycode) {
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::KeyEvent {
                                scancode: scancode.code,
                                pressed: true,
                                extended: scancode.extended,
                            });
                        }
                    } else if let Some(scancode) = keyval_to_scancode(gdk_keyval) {
                        // Fallback to keyval for keys not in keycode table
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::KeyEvent {
                                scancode: scancode.code,
                                pressed: true,
                                extended: scancode.extended,
                            });
                        }
                    } else if let Some(ch) = keyval_to_unicode(gdk_keyval) {
                        // Non-Latin characters (Cyrillic, etc.) — send as Unicode event
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::UnicodeEvent {
                                character: ch,
                                pressed: true,
                            });
                        }
                    } else {
                        tracing::warn!(
                            "[IronRDP] Unknown key: keycode={}, keyval=0x{:X}",
                            hw_keycode,
                            gdk_keyval
                        );
                    }
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: true,
                    });
                }
            }

            gdk::glib::Propagation::Proceed
        });

        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();

        key_controller.connect_key_released(move |_controller, keyval, keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                if using_ironrdp {
                    let hw_keycode = keycode;
                    let gdk_keyval = keyval.into_glib();

                    if let Some(scancode) = keycode_to_scancode(hw_keycode) {
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::KeyEvent {
                                scancode: scancode.code,
                                pressed: false,
                                extended: scancode.extended,
                            });
                        }
                    } else if let Some(scancode) = keyval_to_scancode(gdk_keyval) {
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::KeyEvent {
                                scancode: scancode.code,
                                pressed: false,
                                extended: scancode.extended,
                            });
                        }
                    } else if let Some(ch) = keyval_to_unicode(gdk_keyval)
                        && let Some(ref tx) = *ironrdp_tx.borrow()
                    {
                        let _ = tx.send(RdpClientCommand::UnicodeEvent {
                            character: ch,
                            pressed: false,
                        });
                    }
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: false,
                    });
                }
            }
        });

        self.drawing_area.add_controller(key_controller);

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
