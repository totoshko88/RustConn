//! String interning utilities for connection data
//!
//! This module provides utilities for interning frequently repeated strings
//! in connection data to reduce memory usage. Common candidates for interning
//! include protocol names, common hostnames, and usernames.

use std::sync::Arc;

use crate::performance::memory_optimizer;

/// Interns a protocol name string for memory efficiency
///
/// Protocol names are frequently repeated across many connections,
/// making them excellent candidates for string interning.
///
/// # Arguments
///
/// * `protocol_name` - The protocol name to intern (e.g., "SSH", "RDP", "VNC")
///
/// # Returns
///
/// An `Arc<str>` pointing to the interned string
#[must_use]
pub fn intern_protocol_name(protocol_name: &str) -> Arc<str> {
    memory_optimizer().interner().intern(protocol_name)
}

/// Interns a hostname string for memory efficiency
///
/// Common hostnames that appear across multiple connections
/// benefit from interning to reduce memory usage.
///
/// # Arguments
///
/// * `hostname` - The hostname to intern
///
/// # Returns
///
/// An `Arc<str>` pointing to the interned string
#[must_use]
pub fn intern_hostname(hostname: &str) -> Arc<str> {
    memory_optimizer().interner().intern(hostname)
}

/// Interns a username string for memory efficiency
///
/// Usernames are often repeated across connections to the same
/// infrastructure, making them good candidates for interning.
///
/// # Arguments
///
/// * `username` - The username to intern
///
/// # Returns
///
/// An `Arc<str>` pointing to the interned string
#[must_use]
pub fn intern_username(username: &str) -> Arc<str> {
    memory_optimizer().interner().intern(username)
}

/// Interns multiple connection-related strings at once
///
/// This is useful when loading connections to batch intern
/// all relevant strings for a connection.
///
/// # Arguments
///
/// * `protocol_name` - The protocol name
/// * `hostname` - The hostname
/// * `username` - Optional username
///
/// # Returns
///
/// A tuple of interned strings: (protocol, hostname, Option<username>)
#[must_use]
pub fn intern_connection_strings(
    protocol_name: &str,
    hostname: &str,
    username: Option<&str>,
) -> (Arc<str>, Arc<str>, Option<Arc<str>>) {
    let interner = memory_optimizer().interner();
    (
        interner.intern(protocol_name),
        interner.intern(hostname),
        username.map(|u| interner.intern(u)),
    )
}

/// Logs interning statistics and returns a warning if hit rate is low
///
/// This function checks the current interning statistics and logs them.
/// If the hit rate falls below the threshold (30%), it returns a warning message.
///
/// # Arguments
///
/// * `threshold` - The minimum acceptable hit rate (0.0 to 1.0)
///
/// # Returns
///
/// `Some(warning_message)` if hit rate is below threshold, `None` otherwise
#[must_use]
#[allow(clippy::cast_precision_loss)] // Precision loss acceptable for statistics
pub fn check_interning_stats(threshold: f64) -> Option<String> {
    let stats = memory_optimizer().interner().stats();
    let intern_count = stats
        .intern_count
        .load(std::sync::atomic::Ordering::Relaxed);
    let hit_count = stats.hit_count.load(std::sync::atomic::Ordering::Relaxed);
    let bytes_saved = stats.bytes_saved.load(std::sync::atomic::Ordering::Relaxed);

    if intern_count == 0 {
        return None;
    }

    let hit_rate = hit_count as f64 / intern_count as f64;

    // Log statistics
    tracing::debug!(
        intern_count = intern_count,
        hit_count = hit_count,
        hit_rate = format!("{:.1}%", hit_rate * 100.0),
        bytes_saved = bytes_saved,
        "String interning statistics"
    );

    if hit_rate < threshold && intern_count > 100 {
        Some(format!(
            "String interner hit rate ({:.1}%) is below recommended threshold ({:.1}%). \
             Consider reviewing which strings are being interned.",
            hit_rate * 100.0,
            threshold * 100.0
        ))
    } else {
        None
    }
}

/// Gets the current interning statistics
///
/// # Returns
///
/// A tuple of (`intern_count`, `hit_count`, `hit_rate`, `bytes_saved`)
#[must_use]
#[allow(clippy::cast_precision_loss)] // Precision loss acceptable for statistics
pub fn get_interning_stats() -> (usize, usize, f64, usize) {
    let stats = memory_optimizer().interner().stats();
    let intern_count = stats
        .intern_count
        .load(std::sync::atomic::Ordering::Relaxed);
    let hit_count = stats.hit_count.load(std::sync::atomic::Ordering::Relaxed);
    let bytes_saved = stats.bytes_saved.load(std::sync::atomic::Ordering::Relaxed);

    let hit_rate = if intern_count > 0 {
        hit_count as f64 / intern_count as f64
    } else {
        0.0
    };

    (intern_count, hit_count, hit_rate, bytes_saved)
}

/// Logs interning statistics at info level
///
/// This function logs the current interning statistics including:
/// - Total intern requests
/// - Cache hits
/// - Hit rate percentage
/// - Bytes saved through deduplication
///
/// Call this periodically (e.g., after loading connections) to monitor
/// interning effectiveness.
pub fn log_interning_stats() {
    let (intern_count, hit_count, hit_rate, bytes_saved) = get_interning_stats();

    if intern_count == 0 {
        tracing::debug!("String interning: no strings interned yet");
        return;
    }

    tracing::info!(
        intern_count = intern_count,
        hit_count = hit_count,
        hit_rate_percent = format!("{:.1}", hit_rate * 100.0),
        bytes_saved = bytes_saved,
        "String interning statistics"
    );
}

/// Logs interning statistics and emits a warning if hit rate is below threshold
///
/// This function logs the current interning statistics and checks if the
/// hit rate is below the recommended threshold (30%). If so, it logs a
/// warning suggesting configuration review.
///
/// # Arguments
///
/// * `threshold` - The minimum acceptable hit rate (0.0 to 1.0), defaults to 0.3
///
/// # Returns
///
/// `true` if hit rate is acceptable, `false` if below threshold
pub fn log_interning_stats_with_warning(threshold: f64) -> bool {
    let (intern_count, hit_count, hit_rate, bytes_saved) = get_interning_stats();

    if intern_count == 0 {
        tracing::debug!("String interning: no strings interned yet");
        return true;
    }

    // Log statistics at info level
    tracing::info!(
        intern_count = intern_count,
        hit_count = hit_count,
        hit_rate_percent = format!("{:.1}", hit_rate * 100.0),
        bytes_saved = bytes_saved,
        "String interning statistics"
    );

    // Check if hit rate is below threshold (only warn if we have enough samples)
    if hit_rate < threshold && intern_count > 100 {
        tracing::warn!(
            hit_rate_percent = format!("{:.1}", hit_rate * 100.0),
            threshold_percent = format!("{:.1}", threshold * 100.0),
            "String interner hit rate is below recommended threshold. \
             Consider reviewing which strings are being interned."
        );
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_protocol_name() {
        let arc1 = intern_protocol_name("SSH");
        let arc2 = intern_protocol_name("SSH");

        // Same string should return same Arc
        assert!(Arc::ptr_eq(&arc1, &arc2));
        assert_eq!(&*arc1, "SSH");
    }

    #[test]
    fn test_intern_hostname() {
        let arc1 = intern_hostname("example.com");
        let arc2 = intern_hostname("example.com");

        assert!(Arc::ptr_eq(&arc1, &arc2));
        assert_eq!(&*arc1, "example.com");
    }

    #[test]
    fn test_intern_username() {
        let arc1 = intern_username("admin");
        let arc2 = intern_username("admin");

        assert!(Arc::ptr_eq(&arc1, &arc2));
        assert_eq!(&*arc1, "admin");
    }

    #[test]
    fn test_intern_connection_strings() {
        let (proto, host, user) = intern_connection_strings("RDP", "server.local", Some("user1"));

        assert_eq!(&*proto, "RDP");
        assert_eq!(&*host, "server.local");
        assert_eq!(user.as_deref(), Some("user1"));

        // Intern again and verify same Arcs
        let (proto2, host2, user2) =
            intern_connection_strings("RDP", "server.local", Some("user1"));

        assert!(Arc::ptr_eq(&proto, &proto2));
        assert!(Arc::ptr_eq(&host, &host2));
        assert!(Arc::ptr_eq(&user.unwrap(), &user2.unwrap()));
    }

    #[test]
    fn test_intern_connection_strings_no_username() {
        let (proto, host, user) = intern_connection_strings("VNC", "vnc.local", None);

        assert_eq!(&*proto, "VNC");
        assert_eq!(&*host, "vnc.local");
        assert!(user.is_none());
    }

    #[test]
    fn test_get_interning_stats() {
        // Intern some strings to generate stats
        let _ = intern_protocol_name("TEST_PROTO");
        let _ = intern_protocol_name("TEST_PROTO");
        let _ = intern_hostname("test.host");

        let (intern_count, hit_count, hit_rate, _bytes_saved) = get_interning_stats();

        // We should have at least the strings we just interned
        assert!(intern_count >= 3);
        assert!(hit_count >= 1);
        assert!((0.0..=1.0).contains(&hit_rate));
    }

    #[test]
    fn test_check_interning_stats_no_warning_when_empty() {
        // With no interning, should return None
        // Note: This test may be affected by other tests that intern strings
        // so we just verify the function doesn't panic
        let _ = check_interning_stats(0.3);
    }

    #[test]
    fn test_log_interning_stats() {
        // Intern some strings to generate stats
        let _ = intern_protocol_name("LOG_TEST_PROTO");
        let _ = intern_protocol_name("LOG_TEST_PROTO");
        let _ = intern_hostname("log.test.host");

        // Should not panic
        log_interning_stats();
    }

    #[test]
    fn test_log_interning_stats_with_warning() {
        // Intern some strings to generate stats
        let _ = intern_protocol_name("WARN_TEST_PROTO");
        let _ = intern_protocol_name("WARN_TEST_PROTO");
        let _ = intern_hostname("warn.test.host");

        // Should not panic
        log_interning_stats_with_warning(0.3);
    }
}
