//! Dialog utility functions for parsing and formatting connection data
//!
//! These functions handle the conversion between dialog field formats and
//! structured data types used in Connection configurations.

use std::collections::HashMap;

/// Parses a comma-separated list of key=value pairs into a `HashMap`.
///
/// Format: "Key1=Value1, Key2=Value2, ..."
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::parse_custom_options;
///
/// let options = parse_custom_options("ForwardAgent=yes, StrictHostKeyChecking=no");
/// assert_eq!(options.get("ForwardAgent"), Some(&"yes".to_string()));
/// assert_eq!(options.get("StrictHostKeyChecking"), Some(&"no".to_string()));
/// ```
#[must_use]
pub fn parse_custom_options(text: &str) -> HashMap<String, String> {
    let mut options = HashMap::new();
    if text.trim().is_empty() {
        return options;
    }

    for part in text.split(',') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if !key.is_empty() {
                options.insert(key, value);
            }
        }
    }
    options
}

/// Formats a `HashMap` of options into a comma-separated key=value string.
///
/// This is the inverse of `parse_custom_options`.
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::format_custom_options;
/// use std::collections::HashMap;
///
/// let mut options = HashMap::new();
/// options.insert("ForwardAgent".to_string(), "yes".to_string());
/// let formatted = format_custom_options(&options);
/// assert!(formatted.contains("ForwardAgent=yes"));
/// ```
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn format_custom_options(options: &HashMap<String, String>) -> String {
    let mut pairs: Vec<String> = options.iter().map(|(k, v)| format!("{k}={v}")).collect();
    pairs.sort(); // Sort for deterministic output
    pairs.join(", ")
}

/// Parses a space-separated string into a vector of arguments.
///
/// Note: This is a simple parser that doesn't handle quoted strings.
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::parse_args;
///
/// let args = parse_args("/fullscreen /sound:sys:alsa");
/// assert_eq!(args, vec!["/fullscreen", "/sound:sys:alsa"]);
/// ```
#[must_use]
pub fn parse_args(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split_whitespace()
        .map(std::string::ToString::to_string)
        .collect()
}

/// Formats a vector of arguments into a space-separated string.
///
/// This is the inverse of `parse_args`.
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::format_args;
///
/// let args = vec!["/fullscreen".to_string(), "/sound:sys:alsa".to_string()];
/// let formatted = format_args(&args);
/// assert_eq!(formatted, "/fullscreen /sound:sys:alsa");
/// ```
#[must_use]
pub fn format_args(args: &[String]) -> String {
    args.join(" ")
}

/// Validates a connection name.
///
/// # Errors
///
/// Returns `Err` with a message if the name is empty or whitespace-only.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Connection name is required".to_string());
    }
    Ok(())
}

/// Validates a host address.
///
/// # Errors
///
/// Returns `Err` with a message if the host is empty or contains spaces.
pub fn validate_host(host: &str) -> Result<(), String> {
    if host.trim().is_empty() {
        return Err("Host is required".to_string());
    }
    let host_str = host.trim();
    if host_str.contains(' ') {
        return Err("Host cannot contain spaces".to_string());
    }
    Ok(())
}

/// Validates a port number.
///
/// # Errors
///
/// Returns `Err` with a message if the port is zero.
pub fn validate_port(port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("Port must be greater than 0".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_custom_options_empty() {
        let options = parse_custom_options("");
        assert!(options.is_empty());
    }

    #[test]
    fn test_parse_custom_options_single() {
        let options = parse_custom_options("Key=Value");
        assert_eq!(options.len(), 1);
        assert_eq!(options.get("Key"), Some(&"Value".to_string()));
    }

    #[test]
    fn test_parse_custom_options_multiple() {
        let options = parse_custom_options("Key1=Value1, Key2=Value2");
        assert_eq!(options.len(), 2);
        assert_eq!(options.get("Key1"), Some(&"Value1".to_string()));
        assert_eq!(options.get("Key2"), Some(&"Value2".to_string()));
    }

    #[test]
    fn test_parse_args_empty() {
        let args = parse_args("");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_single() {
        let args = parse_args("/fullscreen");
        assert_eq!(args, vec!["/fullscreen"]);
    }

    #[test]
    fn test_parse_args_multiple() {
        let args = parse_args("/fullscreen /sound:sys:alsa");
        assert_eq!(args, vec!["/fullscreen", "/sound:sys:alsa"]);
    }
}
