//! Offline OUI → vendor lookup for `.rsc` scripts.
//!
//! The original SecureCRT helper reads Wireshark's `manuf.txt`, which is
//! GPLv2, so the table is generated from the IEEE MA-L public listing
//! instead — factual registry data with no licence question. The table is
//! the full listing: "Vendor not found" means IEEE has no such 24-bit OUI.
//!
//! shortcut: MA-L only; the 28/36-bit MA-M/MA-S/IAB blocks need mask
//! matching, add them to `oui_data.rs` if lookups start missing them.

use crate::oui_data::MA_L;

/// Normalizes a MAC to twelve uppercase hex digits.
///
/// Accepts the three formats the SecureCRT helper recognises:
/// `0123.4567.89ab`, `01:23:45:67:89:ab` and `0123456789ab`.
fn normalize(mac: &str) -> Option<String> {
    let hex: String = mac.chars().filter(char::is_ascii_hexdigit).collect();
    (hex.len() == 12).then_some(hex.to_ascii_uppercase())
}

/// Returns the vendor for `mac`, preferring the longest matching prefix.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     rustconn_core::oui::lookup("00:21:55:33:44:55"),
///     Some("Cisco Systems, Inc")
/// );
/// assert_eq!(rustconn_core::oui::lookup("not a mac"), None);
/// ```
#[must_use]
pub fn lookup(mac: &str) -> Option<&'static str> {
    let normalized = normalize(mac)?;
    MA_L.iter()
        .filter(|(prefix, _)| normalized.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, vendor)| *vendor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_three_common_mac_formats() {
        for mac in [
            "0021.5533.4455",
            "00:21:55:33:44:55",
            "00-21-55-33-44-55",
            "002155334455",
        ] {
            assert_eq!(lookup(mac), Some("Cisco Systems, Inc"), "{mac}");
        }
    }

    #[test]
    fn rejects_invalid_and_unknown_values() {
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("00:11:22"), None);
        assert_eq!(lookup("FF:FF:FF:FF:FF:FF"), None);
    }

    #[test]
    fn reports_the_official_registrant_and_skips_local_addresses() {
        // VirtualBox's OUI belongs to its real registrant, not a product name.
        assert_eq!(lookup("08:00:27:00:00:00"), Some("PCS Systemtechnik GmbH"));
        // Locally administered addresses carry no OUI.
        assert_eq!(lookup("02:00:5E:00:00:01"), None);
    }
}
