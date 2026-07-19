//! Windows admin quick actions for RDP sessions
//!
//! Predefined key sequences that launch common Windows administration tools
//! via the remote RDP session. Each action is a sequence of `(scancode,
//! pressed, extended)` tuples that simulate keyboard input.
//!
//! # Approach
//!
//! Most tools are launched via Win+R (Run dialog) followed by typing the
//! command name and pressing Enter. This is reliable across all Windows
//! versions and does not require elevated privileges for the Run dialog
//! itself.

/// Scancode constants for readability
mod scancodes {
    /// Left Windows key (extended)
    pub(super) const WIN: u16 = 0x5B;
    /// R key
    pub(super) const R: u16 = 0x13;
    /// I key
    pub(super) const I: u16 = 0x17;
    /// Enter key
    pub(super) const ENTER: u16 = 0x1C;
    /// Escape key
    pub(super) const ESC: u16 = 0x01;
    /// Left Ctrl key
    pub(super) const CTRL: u16 = 0x1D;
    /// Left Shift key
    pub(super) const SHIFT: u16 = 0x2A;
}

/// A predefined Windows admin quick action
#[derive(Debug, Clone)]
pub struct QuickAction {
    /// Unique identifier
    pub id: &'static str,
    /// Display name (English, will be wrapped with gettext on GUI side)
    pub label: &'static str,
    /// Tooltip description
    pub tooltip: &'static str,
    /// Icon name (symbolic, GNOME icon theme)
    pub icon: &'static str,
    /// Group index for visual separation (items with same group are together)
    pub group: u8,
}

/// All available quick actions
///
/// Group 0: Quick shortcuts (hotkey-based, no Run dialog)
/// Group 1: Admin consoles (Win+R based, alphabetically sorted)
pub static QUICK_ACTIONS: &[QuickAction] = &[
    // Group 0: Quick shortcuts
    QuickAction {
        id: "settings",
        label: "Settings",
        tooltip: "Open Windows Settings (Win+I)",
        icon: "emblem-system-symbolic",
        group: 0,
    },
    QuickAction {
        id: "task-manager",
        label: "Task Manager",
        tooltip: "Open Windows Task Manager (Ctrl+Shift+Esc)",
        icon: "utilities-system-monitor-symbolic",
        group: 0,
    },
    // Group 1: Admin consoles (alphabetical)
    QuickAction {
        id: "computer-management",
        label: "Computer Management",
        tooltip: "Open Computer Management (disks, services, users, event log)",
        icon: "computer-symbolic",
        group: 1,
    },
    QuickAction {
        id: "device-manager",
        label: "Device Manager",
        tooltip: "Open Windows Device Manager",
        icon: "preferences-desktop-peripherals-symbolic",
        group: 1,
    },
    QuickAction {
        id: "disk-management",
        label: "Disk Management",
        tooltip: "Open Windows Disk Management console",
        icon: "drive-harddisk-symbolic",
        group: 1,
    },
    QuickAction {
        id: "event-viewer",
        label: "Event Viewer",
        tooltip: "Open Windows Event Viewer",
        icon: "document-open-recent-symbolic",
        group: 1,
    },
    QuickAction {
        id: "registry-editor",
        label: "Registry Editor",
        tooltip: "Open Windows Registry Editor",
        icon: "preferences-other-symbolic",
        group: 1,
    },
    QuickAction {
        id: "resource-monitor",
        label: "Resource Monitor",
        tooltip: "Open Windows Resource Monitor (CPU, memory, disk, network)",
        icon: "org.gnome.SystemMonitor-symbolic",
        group: 1,
    },
    QuickAction {
        id: "server-manager",
        label: "Server Manager",
        tooltip: "Open Windows Server Manager",
        icon: "network-server-symbolic",
        group: 1,
    },
    QuickAction {
        id: "services",
        label: "Services",
        tooltip: "Open Windows Services console",
        icon: "application-x-executable-symbolic",
        group: 1,
    },
];

/// Builds the hotkey scancode sequence for actions that do not use the Run dialog.
///
/// Returns `None` for Run-dialog actions (use [`run_command_for`] together with
/// [`build_open_run_dialog`] and Unicode autotype instead) and for unknown IDs.
///
/// These two actions are pure modifier hotkeys (Ctrl+Shift+Esc, Win+I) which
/// Windows resolves by virtual-key, so they are unaffected by the remote
/// keyboard layout.
#[must_use]
pub fn build_hotkey_sequence(action_id: &str) -> Option<Vec<(u16, bool, bool)>> {
    match action_id {
        "task-manager" => Some(build_ctrl_shift_esc()),
        "settings" => Some(build_win_i()),
        _ => None,
    }
}

/// Returns the Run-dialog command string for an action ID.
///
/// Returns `None` for hotkey actions (see [`build_hotkey_sequence`]) and unknown
/// IDs. The caller opens the Run dialog with [`build_open_run_dialog`], types
/// the returned string via Unicode autotype (layout-independent), then presses
/// Enter with [`build_enter_sequence`].
#[must_use]
pub fn run_command_for(action_id: &str) -> Option<&'static str> {
    match action_id {
        "registry-editor" => Some("regedit"),
        "device-manager" => Some("devmgmt.msc"),
        "event-viewer" => Some("eventvwr.msc"),
        "services" => Some("services.msc"),
        "disk-management" => Some("diskmgmt.msc"),
        "resource-monitor" => Some("resmon"),
        "computer-management" => Some("compmgmt.msc"),
        "server-manager" => Some("servermanager"),
        _ => None,
    }
}

/// Ctrl+Shift+Esc → Task Manager
fn build_ctrl_shift_esc() -> Vec<(u16, bool, bool)> {
    vec![
        (scancodes::CTRL, true, false),
        (scancodes::SHIFT, true, false),
        (scancodes::ESC, true, false),
        (scancodes::ESC, false, false),
        (scancodes::SHIFT, false, false),
        (scancodes::CTRL, false, false),
    ]
}

/// Win+I → Settings
fn build_win_i() -> Vec<(u16, bool, bool)> {
    vec![
        (scancodes::WIN, true, true),
        (scancodes::I, true, false),
        (scancodes::I, false, false),
        (scancodes::WIN, false, true),
    ]
}

/// Builds the Win+R sequence that opens the Run dialog.
///
/// Layout-independent: `Win` and `R` are resolved by Windows as a registered
/// system hotkey (virtual-key based), and the `R` physical key sits in the same
/// position on QWERTY, AZERTY and QWERTZ. The command text itself must NOT be
/// typed via scancodes — it is sent separately as Unicode autotype events so it
/// is correct regardless of the remote keyboard layout (see issue #184).
#[must_use]
pub fn build_open_run_dialog() -> Vec<(u16, bool, bool)> {
    vec![
        (scancodes::WIN, true, true),
        (scancodes::R, true, false),
        (scancodes::R, false, false),
        (scancodes::WIN, false, true),
    ]
}

/// Builds the Enter key sequence that executes the typed command or pasted content.
#[must_use]
pub fn build_enter_sequence() -> Vec<(u16, bool, bool)> {
    vec![
        (scancodes::ENTER, true, false),
        (scancodes::ENTER, false, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_quick_actions_have_sequences() {
        for action in QUICK_ACTIONS {
            let has_hotkey = build_hotkey_sequence(action.id).is_some();
            let has_run = run_command_for(action.id).is_some();
            assert!(
                has_hotkey ^ has_run,
                "Action '{}' must be exactly one of hotkey or run-command",
                action.id
            );
        }
    }

    #[test]
    fn ctrl_shift_esc_sequence_is_balanced() {
        let keys = build_ctrl_shift_esc();
        // Every press must have a matching release
        let presses = keys.iter().filter(|(_, pressed, _)| *pressed).count();
        let releases = keys.iter().filter(|(_, pressed, _)| !*pressed).count();
        assert_eq!(presses, releases, "Unbalanced key presses/releases");
    }

    #[test]
    fn win_i_sequence_is_balanced() {
        let keys = build_win_i();
        let presses = keys.iter().filter(|(_, pressed, _)| *pressed).count();
        let releases = keys.iter().filter(|(_, pressed, _)| !*pressed).count();
        assert_eq!(presses, releases);
    }

    #[test]
    fn open_run_dialog_is_balanced_and_layout_safe() {
        let keys = build_open_run_dialog();
        let presses = keys.iter().filter(|(_, pressed, _)| *pressed).count();
        let releases = keys.iter().filter(|(_, pressed, _)| !*pressed).count();
        assert_eq!(presses, releases, "Unbalanced Win+R press/release");
        // Only Win and R scancodes — the command itself is typed via Unicode.
        for (sc, _, _) in &keys {
            assert!(
                *sc == scancodes::WIN || *sc == scancodes::R,
                "Unexpected scancode {sc:#x} in Run-dialog opener"
            );
        }
    }

    #[test]
    fn run_commands_are_ascii_and_nonempty() {
        // Sanity: every run action maps to a plausible command. Unicode autotype
        // handles any characters, but admin commands should stay simple ASCII.
        for action in QUICK_ACTIONS {
            if let Some(cmd) = run_command_for(action.id) {
                assert!(!cmd.is_empty(), "Empty command for '{}'", action.id);
                assert!(
                    cmd.is_ascii(),
                    "Non-ASCII command '{cmd}' for '{}'",
                    action.id
                );
            }
        }
    }
}
