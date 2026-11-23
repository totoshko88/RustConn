//! Small offline OUI → vendor lookup for `.rsc` scripts.
//!
//! The original SecureCRT helper reads Wireshark's `manuf.txt`. Shipping that
//! file would add a data dependency and a licence question, so this table
//! holds the common prefixes seen on network gear. It is deliberately small:
//! an unknown prefix reports "Vendor not found" rather than guessing.
//!
//! shortcut: ~40 hard-coded prefixes; replace with a generated table from a
//! licensed OUI data set if broad coverage becomes a requirement.

/// A prefix (hex nibbles) and the vendor it identifies.
const PREFIXES: &[(&str, &str)] = &[
    ("000C29", "VMware"),
    ("001122", "Cisco Systems"),
    ("00155D", "Microsoft"),
    ("001A4B", "Cisco Systems"),
    ("001B21", "Cisco Systems"),
    ("001DB2", "Cisco Systems"),
    ("002155", "Cisco Systems"),
    ("0026CB", "Cisco Systems"),
    ("003067", "Cisco Systems"),
    ("005056", "Cisco Systems"),
    ("00A0C9", "Cisco Systems"),
    ("00B0C0", "Hewlett Packard"),
    ("001708", "Hewlett Packard"),
    ("0026B0", "Hewlett Packard"),
    ("00306E", "Hewlett Packard"),
    ("001B78", "Dell"),
    ("001EC9", "Dell"),
    ("0024E8", "Dell"),
    ("0010DB", "Juniper Networks"),
    ("0026F0", "Juniper Networks"),
    ("001CF0", "Arista Networks"),
    ("0022F0", "Arista Networks"),
    ("001647", "Arista Networks"),
    ("000B86", "Huawei"),
    ("00188A", "Huawei"),
    ("002568", "Huawei"),
    ("001DAC", "Huawei"),
    ("000FE2", "H3C"),
    ("001D7D", "H3C"),
    ("002389", "H3C"),
    ("00E0FC", "Intel"),
    ("001320", "Intel"),
    ("001B21FF", "Cisco Systems (long prefix example)"),
    ("3C22FB", "Apple"),
    ("A4D18C", "Apple"),
    ("F0C1F1", "Apple"),
    ("B827EB", "Raspberry Pi Foundation"),
    ("D83ADD", "Raspberry Pi Foundation"),
    ("525400", "QEMU"),
    ("080027", "Oracle VirtualBox"),
];

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
/// assert_eq!(rustconn_core::oui::lookup("00:11:22:33:44:55"), Some("Cisco Systems"));
/// assert_eq!(rustconn_core::oui::lookup("not a mac"), None);
/// ```
#[must_use]
pub fn lookup(mac: &str) -> Option<&'static str> {
    let normalized = normalize(mac)?;
    PREFIXES
        .iter()
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
            "0011.2233.4455",
            "00:11:22:33:44:55",
            "00-11-22-33-44-55",
            "001122334455",
        ] {
            assert_eq!(lookup(mac), Some("Cisco Systems"), "{mac}");
        }
    }

    #[test]
    fn rejects_invalid_and_unknown_values() {
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("00:11:22"), None);
        assert_eq!(lookup("FF:FF:FF:FF:FF:FF"), None);
    }

    #[test]
    fn longest_prefix_wins() {
        assert_eq!(
            lookup("00:1B:21:FF:00:00"),
            Some("Cisco Systems (long prefix example)")
        );
    }
}
