//! Property tests for RDP backend selection

use proptest::prelude::*;
use rustconn_core::rdp_client::{RdpBackend, RdpBackendSelector};

proptest! {
    /// Property: All backends have non-empty command names
    #[test]
    fn all_backends_have_command_names(
        backend_idx in 0usize..5,
    ) {
        let backends = [
            RdpBackend::IronRdp,
            RdpBackend::WlFreeRdp,
            RdpBackend::XFreeRdp3,
            RdpBackend::XFreeRdp,
            RdpBackend::FreeRdp,
        ];
        let backend = backends[backend_idx];
        prop_assert!(!backend.command_name().is_empty());
    }

    /// Property: All backends have non-empty display names
    #[test]
    fn all_backends_have_display_names(
        backend_idx in 0usize..5,
    ) {
        let backends = [
            RdpBackend::IronRdp,
            RdpBackend::WlFreeRdp,
            RdpBackend::XFreeRdp3,
            RdpBackend::XFreeRdp,
            RdpBackend::FreeRdp,
        ];
        let backend = backends[backend_idx];
        prop_assert!(!backend.display_name().is_empty());
    }

    /// Property: Display trait matches display_name
    #[test]
    fn display_matches_display_name(
        backend_idx in 0usize..5,
    ) {
        let backends = [
            RdpBackend::IronRdp,
            RdpBackend::WlFreeRdp,
            RdpBackend::XFreeRdp3,
            RdpBackend::XFreeRdp,
            RdpBackend::FreeRdp,
        ];
        let backend = backends[backend_idx];
        prop_assert_eq!(format!("{backend}"), backend.display_name());
    }

    /// Property: Only IronRdp is native
    #[test]
    fn only_ironrdp_is_native(
        backend_idx in 0usize..5,
    ) {
        let backends = [
            RdpBackend::IronRdp,
            RdpBackend::WlFreeRdp,
            RdpBackend::XFreeRdp3,
            RdpBackend::XFreeRdp,
            RdpBackend::FreeRdp,
        ];
        let backend = backends[backend_idx];
        let is_native = backend.is_native();
        let is_ironrdp = matches!(backend, RdpBackend::IronRdp);
        prop_assert_eq!(is_native, is_ironrdp);
    }

    /// Property: Embedded support only for IronRdp and WlFreeRdp
    #[test]
    fn embedded_support_correct(
        backend_idx in 0usize..5,
    ) {
        let backends = [
            RdpBackend::IronRdp,
            RdpBackend::WlFreeRdp,
            RdpBackend::XFreeRdp3,
            RdpBackend::XFreeRdp,
            RdpBackend::FreeRdp,
        ];
        let backend = backends[backend_idx];
        let supports = backend.supports_embedded();
        let expected = matches!(backend, RdpBackend::IronRdp | RdpBackend::WlFreeRdp);
        prop_assert_eq!(supports, expected);
    }
}

#[test]
fn test_selector_with_ironrdp_true() {
    let mut selector = RdpBackendSelector::with_ironrdp_available(true);
    let results = selector.detect_all();

    // IronRDP should be available
    let ironrdp_result = results.iter().find(|r| r.backend == RdpBackend::IronRdp);
    assert!(ironrdp_result.is_some());
    assert!(ironrdp_result.unwrap().available);
}

#[test]
fn test_selector_with_ironrdp_false() {
    let mut selector = RdpBackendSelector::with_ironrdp_available(false);
    let results = selector.detect_all();

    // IronRDP should not be available
    let ironrdp_result = results.iter().find(|r| r.backend == RdpBackend::IronRdp);
    assert!(ironrdp_result.is_some());
    assert!(!ironrdp_result.unwrap().available);
}

#[test]
fn test_selector_cache_behavior() {
    let mut selector = RdpBackendSelector::with_ironrdp_available(true);

    // First call populates cache
    let results1 = selector.detect_all();
    let count1 = results1.len();

    // Second call uses cache
    let results2 = selector.detect_all();
    let count2 = results2.len();

    assert_eq!(count1, count2);
    assert_eq!(count1, 5); // All 5 backends checked

    // Clear cache
    selector.clear_cache();

    // Third call repopulates cache
    let results3 = selector.detect_all();
    assert_eq!(results3.len(), 5);
}

#[test]
fn test_embedded_selection_priority() {
    let mut selector = RdpBackendSelector::with_ironrdp_available(true);

    // With IronRDP available, it should be selected for embedded
    if let Some(backend) = selector.select_embedded() {
        assert!(backend.supports_embedded());
        // IronRDP should be preferred
        assert_eq!(backend, RdpBackend::IronRdp);
    }
}

#[test]
fn test_backend_command_names_unique() {
    let backends = [
        RdpBackend::IronRdp,
        RdpBackend::WlFreeRdp,
        RdpBackend::XFreeRdp3,
        RdpBackend::XFreeRdp,
        RdpBackend::FreeRdp,
    ];

    let mut names: Vec<&str> = backends.iter().map(|b| b.command_name()).collect();
    let original_len = names.len();
    names.sort();
    names.dedup();

    assert_eq!(names.len(), original_len, "Command names should be unique");
}

#[test]
fn test_backend_equality() {
    assert_eq!(RdpBackend::IronRdp, RdpBackend::IronRdp);
    assert_ne!(RdpBackend::IronRdp, RdpBackend::WlFreeRdp);
    assert_ne!(RdpBackend::XFreeRdp, RdpBackend::XFreeRdp3);
}

#[test]
fn test_backend_clone() {
    let backend = RdpBackend::WlFreeRdp;
    let cloned = backend;
    assert_eq!(backend, cloned);
}

#[test]
fn test_selector_default() {
    let mut selector = RdpBackendSelector::default();
    // Default selector should work and detect backends
    let results = selector.detect_all();
    assert!(!results.is_empty());
}
