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
    pub const WIN: u16 = 0x5B;
    /// R key
    pub const R: u16 = 0x13;
    /// I key
    pub const I: u16 = 0x17;
    /// Enter key
    pub const ENTER: u16 = 0x1C;
    /// Escape key
    pub const ESC: u16 = 0x01;
    /// Left Ctrl key
    pub const CTRL: u16 = 0x1D;
    /// Left Shift key
    pub const SHIFT: u16 = 0x2A;
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
}

/// All available quick actions
pub static QUICK_ACTIONS: &[QuickAction] = &[
    QuickAction {
        id: "task-manager",
        label: "Task Manager",
        tooltip: "Open Windows Task Manager (Ctrl+Shift+Esc)",
        icon: "utilities-system-monitor-symbolic",
    },
    QuickAction {
        id: "settings",
        label: "Settings",
        tooltip: "Open Windows Settings (Win+I)",
        icon: "emblem-system-symbolic",
    },
    QuickAction {
        id: "powershell",
        label: "PowerShell",
        tooltip: "Open PowerShell via Run dialog",
        icon: "utilities-terminal-symbolic",
    },
    QuickAction {
        id: "cmd",
        label: "CMD",
        tooltip: "Open Command Prompt via Run dialog",
        icon: "utilities-terminal-symbolic",
    },
    QuickAction {
        id: "event-viewer",
        label: "Event Viewer",
        tooltip: "Open Windows Event Viewer",
        icon: "document-open-recent-symbolic",
    },
    QuickAction {
        id: "services",
        label: "Services",
        tooltip: "Open Windows Services console",
        icon: "application-x-executable-symbolic",
    },
    QuickAction {
        id: "disk-management",
        label: "Disk Management",
        tooltip: "Open Windows Disk Management console",
        icon: "drive-harddisk-symbolic",
    },
    QuickAction {
        id: "resource-monitor",
        label: "Resource Monitor",
        tooltip: "Open Windows Resource Monitor (CPU, memory, disk, network)",
        icon: "org.gnome.SystemMonitor-symbolic",
    },
    QuickAction {
        id: "computer-management",
        label: "Computer Management",
        tooltip: "Open Computer Management (disks, services, users, event log)",
        icon: "computer-symbolic",
    },
];

/// Builds the key sequence for a given quick action ID.
///
/// Returns `None` if the action ID is unknown.
#[must_use]
pub fn build_key_sequence(action_id: &str) -> Option<Vec<(u16, bool, bool)>> {
    match action_id {
        "task-manager" => Some(build_ctrl_shift_esc()),
        "settings" => Some(build_win_i()),
        "powershell" => Some(build_run_command("powershell")),
        "cmd" => Some(build_run_command("cmd")),
        "event-viewer" => Some(build_run_command("eventvwr.msc")),
        "services" => Some(build_run_command("services.msc")),
        "disk-management" => Some(build_run_command("diskmgmt.msc")),
        "resource-monitor" => Some(build_run_command("resmon")),
        "computer-management" => Some(build_run_command("compmgmt.msc")),
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

/// Win+R → type command → Enter
///
/// Inserts a pause marker (scancode 0, not pressed, not extended) between
/// Win+R release and the first typed character. The command processor
/// interprets the 30ms inter-key delay, but the Run dialog needs ~200ms
/// to appear. The GUI layer should handle this by splitting the sequence
/// or the command processor delay is sufficient for most servers.
fn build_run_command(command: &str) -> Vec<(u16, bool, bool)> {
    let mut keys = Vec::with_capacity(4 + command.len() * 2 + 2);

    // Win+R to open Run dialog
    keys.push((scancodes::WIN, true, true));
    keys.push((scancodes::R, true, false));
    keys.push((scancodes::R, false, false));
    keys.push((scancodes::WIN, false, true));

    // Pause: repeat a harmless release to give the Run dialog time to open.
    // The 30ms × 8 = ~240ms delay is enough for the dialog to appear.
    for _ in 0..8 {
        keys.push((0, false, false));
    }

    // Type the command using Unicode events — we encode these as scancode=0
    // with the character packed into a special marker. The command processor
    // will detect scancode=0 + extended=true as a Unicode character request.
    // Actually, we use the existing UnicodeEvent path instead.
    // For simplicity, we encode each character as its scancode equivalent
    // where possible, but for arbitrary commands we need Unicode input.
    //
    // Since SendKeySequence only supports scancodes, we'll use a different
    // approach: encode characters via their keyboard scancodes for ASCII.
    for ch in command.chars() {
        if let Some((sc, needs_shift)) = char_to_scancode(ch) {
            if needs_shift {
                keys.push((0x2A, true, false)); // Shift down
            }
            keys.push((sc, true, false));
            keys.push((sc, false, false));
            if needs_shift {
                keys.push((0x2A, false, false)); // Shift up
            }
        }
    }

    // Enter to execute
    keys.push((scancodes::ENTER, true, false));
    keys.push((scancodes::ENTER, false, false));

    keys
}

/// Maps an ASCII character to its keyboard scancode and shift state.
///
/// Returns `(scancode, needs_shift)`. Only covers characters needed for
/// Windows admin commands (lowercase letters, digits, dot, backslash).
const fn char_to_scancode(ch: char) -> Option<(u16, bool)> {
    // US keyboard layout scancodes for lowercase letters
    match ch {
        'a' => Some((0x1E, false)),
        'b' => Some((0x30, false)),
        'c' => Some((0x2E, false)),
        'd' => Some((0x20, false)),
        'e' => Some((0x12, false)),
        'f' => Some((0x21, false)),
        'g' => Some((0x22, false)),
        'h' => Some((0x23, false)),
        'i' => Some((0x17, false)),
        'j' => Some((0x24, false)),
        'k' => Some((0x25, false)),
        'l' => Some((0x26, false)),
        'm' => Some((0x32, false)),
        'n' => Some((0x31, false)),
        'o' => Some((0x18, false)),
        'p' => Some((0x19, false)),
        'q' => Some((0x10, false)),
        'r' => Some((0x13, false)),
        's' => Some((0x1F, false)),
        't' => Some((0x14, false)),
        'u' => Some((0x16, false)),
        'v' => Some((0x2F, false)),
        'w' => Some((0x11, false)),
        'x' => Some((0x2D, false)),
        'y' => Some((0x15, false)),
        'z' => Some((0x2C, false)),
        '.' => Some((0x34, false)),
        '\\' => Some((0x2B, false)),
        '/' => Some((0x35, false)),
        '-' => Some((0x0C, false)),
        '_' => Some((0x0C, true)), // Shift + -
        ' ' => Some((0x39, false)),
        '0' => Some((0x0B, false)),
        '1' => Some((0x02, false)),
        '2' => Some((0x03, false)),
        '3' => Some((0x04, false)),
        '4' => Some((0x05, false)),
        '5' => Some((0x06, false)),
        '6' => Some((0x07, false)),
        '7' => Some((0x08, false)),
        '8' => Some((0x09, false)),
        '9' => Some((0x0A, false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_quick_actions_have_sequences() {
        for action in QUICK_ACTIONS {
            assert!(
                build_key_sequence(action.id).is_some(),
                "Missing key sequence for action '{}'",
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
    fn run_command_ends_with_enter() {
        let keys = build_run_command("cmd");
        let last_two: Vec<_> = keys.iter().rev().take(2).collect();
        // Last event should be Enter release
        assert_eq!(last_two[0].0, scancodes::ENTER);
        assert!(!last_two[0].1); // released
        // Second to last should be Enter press
        assert_eq!(last_two[1].0, scancodes::ENTER);
        assert!(last_two[1].1); // pressed
    }

    #[test]
    fn char_to_scancode_covers_admin_commands() {
        // All characters used in our admin commands must be mappable
        for cmd in [
            "powershell",
            "cmd",
            "eventvwr.msc",
            "services.msc",
            "diskmgmt.msc",
            "resmon",
            "compmgmt.msc",
        ] {
            for ch in cmd.chars() {
                assert!(
                    char_to_scancode(ch).is_some(),
                    "Unmapped character '{}' in command '{}'",
                    ch,
                    cmd
                );
            }
        }
    }
}
