//! Property/unit tests for `Connection::uses_external_viewer`.
//!
//! **Feature: external-session-tracking**
//! Covers Correctness Properties 1–3 from the design document plus the
//! exhaustive VNC/RDP `window_mode` × `client_mode` decision matrix.
//!
//! These tests exercise the GUI-free predicate in `rustconn-core` that decides
//! whether a connection's display is fully delegated to an external viewer
//! process (and therefore gets no embedded notebook tab).

use proptest::prelude::*;
use rustconn_core::models::{
    Connection, ProtocolConfig, RdpClientMode, RdpConfig, VncClientMode, VncConfig, WindowMode,
};

// ========== Builders ==========

/// Builds a VNC connection with the given window and client modes.
fn vnc_conn(window_mode: WindowMode, client_mode: VncClientMode) -> Connection {
    let mut conn = Connection::new(
        "vnc".to_string(),
        "host.example".to_string(),
        5900,
        ProtocolConfig::Vnc(VncConfig {
            client_mode,
            ..VncConfig::default()
        }),
    );
    conn.window_mode = window_mode;
    conn
}

/// Builds an RDP connection with the given window and client modes.
fn rdp_conn(window_mode: WindowMode, client_mode: RdpClientMode) -> Connection {
    let mut conn = Connection::new(
        "rdp".to_string(),
        "host.example".to_string(),
        3389,
        ProtocolConfig::Rdp(RdpConfig {
            client_mode,
            ..RdpConfig::default()
        }),
    );
    conn.window_mode = window_mode;
    conn
}

/// Builds a SPICE connection with the given window mode.
fn spice_conn(window_mode: WindowMode) -> Connection {
    let mut conn = Connection::new_spice("spice".to_string(), "host.example".to_string(), 5900);
    conn.window_mode = window_mode;
    conn
}

/// Builds every non-graphical protocol connection with the given window mode.
///
/// Covers SSH, Telnet, Serial, Kubernetes, Mosh, `ZeroTrust`, and Web — none of
/// which use an external viewer regardless of settings.
fn non_graphical_conns(window_mode: WindowMode) -> Vec<Connection> {
    use rustconn_core::models::{WebConfig, ZeroTrustConfig};

    let mut conns = vec![
        Connection::new_ssh("ssh".to_string(), "host.example".to_string(), 22),
        Connection::new_telnet("telnet".to_string(), "host.example".to_string(), 23),
        Connection::new_serial("serial".to_string(), "/dev/ttyUSB0".to_string()),
        Connection::new_kubernetes("k8s".to_string()),
        Connection::new_mosh("mosh".to_string(), "host.example".to_string(), 22),
        Connection::new(
            "zt".to_string(),
            String::new(),
            0,
            ProtocolConfig::ZeroTrust(ZeroTrustConfig::default()),
        ),
        Connection::new(
            "web".to_string(),
            "https://example.com".to_string(),
            443,
            ProtocolConfig::Web(WebConfig::default()),
        ),
    ];
    for conn in &mut conns {
        conn.window_mode = window_mode;
    }
    conns
}

// ========== Strategies ==========

fn arb_window_mode() -> impl Strategy<Value = WindowMode> {
    prop_oneof![
        Just(WindowMode::Embedded),
        Just(WindowMode::External),
        Just(WindowMode::Fullscreen),
    ]
}

fn arb_vnc_client_mode() -> impl Strategy<Value = VncClientMode> {
    prop_oneof![Just(VncClientMode::Embedded), Just(VncClientMode::External)]
}

fn arb_rdp_client_mode() -> impl Strategy<Value = RdpClientMode> {
    prop_oneof![Just(RdpClientMode::Embedded), Just(RdpClientMode::External)]
}

/// Generates connections spanning every protocol with arbitrary window and
/// client modes, so the predicate is exercised across its whole input space.
fn arb_connection() -> impl Strategy<Value = Connection> {
    prop_oneof![
        (arb_window_mode(), arb_vnc_client_mode()).prop_map(|(w, c)| vnc_conn(w, c)),
        (arb_window_mode(), arb_rdp_client_mode()).prop_map(|(w, c)| rdp_conn(w, c)),
        arb_window_mode().prop_map(spice_conn),
        // One representative non-graphical protocol per generated case; the
        // exhaustive non-graphical sweep lives in `property_3_*` below.
        (arb_window_mode(), 0usize..7usize).prop_map(|(w, idx)| {
            let mut conns = non_graphical_conns(w);
            conns.swap_remove(idx)
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// **Feature: external-session-tracking, Property 1: Predicate determinism**
    ///
    /// For any connection, `uses_external_viewer()` returns the same value on
    /// every call — no hidden state, no I/O.
    #[test]
    fn property_1_determinism(conn in arb_connection()) {
        let first = conn.uses_external_viewer();
        for _ in 0..8 {
            prop_assert_eq!(
                conn.uses_external_viewer(),
                first,
                "predicate must be deterministic across repeated calls"
            );
        }
    }

    /// **Feature: external-session-tracking, Property 1: Predicate determinism**
    ///
    /// Referential transparency: two connections built from the same
    /// `(protocol, window_mode, client_mode)` inputs yield the same decision.
    #[test]
    fn property_1_referential_transparency(
        window_mode in arb_window_mode(),
        client_mode in arb_vnc_client_mode(),
    ) {
        let a = vnc_conn(window_mode, client_mode);
        let b = vnc_conn(window_mode, client_mode);
        prop_assert_eq!(
            a.uses_external_viewer(),
            b.uses_external_viewer(),
            "same inputs must produce the same result"
        );
    }

    /// **Feature: external-session-tracking, Property 2: SPICE is always external**
    ///
    /// For any SPICE connection, the predicate is `true` regardless of
    /// `window_mode` (SPICE has no client_mode; the embedded client was removed).
    #[test]
    fn property_2_spice_always_external(window_mode in arb_window_mode()) {
        let conn = spice_conn(window_mode);
        prop_assert!(
            conn.uses_external_viewer(),
            "SPICE must always use an external viewer"
        );
    }

    /// **Feature: external-session-tracking, Property 3: Non-graphical is never external**
    ///
    /// For any SSH/Telnet/Serial/Kubernetes/Mosh/ZeroTrust/Web connection, the
    /// predicate is `false` for every `window_mode`.
    #[test]
    fn property_3_non_graphical_never_external(window_mode in arb_window_mode()) {
        for conn in non_graphical_conns(window_mode) {
            prop_assert!(
                !conn.uses_external_viewer(),
                "non-graphical protocol {:?} must never be external",
                conn.protocol
            );
        }
    }

    /// **Feature: external-session-tracking, Property 2/3 combined via matrix**
    ///
    /// For VNC, external iff `window_mode == External` OR
    /// `client_mode == External`, across the full generated input space.
    #[test]
    fn vnc_matrix_matches_spec(
        window_mode in arb_window_mode(),
        client_mode in arb_vnc_client_mode(),
    ) {
        let conn = vnc_conn(window_mode, client_mode);
        let expected =
            window_mode == WindowMode::External || client_mode == VncClientMode::External;
        prop_assert_eq!(conn.uses_external_viewer(), expected);
    }

    /// **Feature: external-session-tracking, Property 2/3 combined via matrix**
    ///
    /// For RDP, external iff `window_mode == External` OR
    /// `client_mode == External`, across the full generated input space.
    #[test]
    fn rdp_matrix_matches_spec(
        window_mode in arb_window_mode(),
        client_mode in arb_rdp_client_mode(),
    ) {
        let conn = rdp_conn(window_mode, client_mode);
        let expected =
            window_mode == WindowMode::External || client_mode == RdpClientMode::External;
        prop_assert_eq!(conn.uses_external_viewer(), expected);
    }
}

// ========== Exhaustive matrix unit tests ==========

/// Exhaustive VNC `window_mode` × `client_mode` matrix (Property 2/3 boundary).
#[test]
fn vnc_full_matrix_is_exhaustive() {
    for &window_mode in WindowMode::all() {
        for &client_mode in VncClientMode::all() {
            let conn = vnc_conn(window_mode, client_mode);
            let expected =
                window_mode == WindowMode::External || client_mode == VncClientMode::External;
            assert_eq!(
                conn.uses_external_viewer(),
                expected,
                "VNC window_mode={window_mode:?} client_mode={client_mode:?}"
            );
        }
    }
}

/// Exhaustive RDP `window_mode` × `client_mode` matrix.
#[test]
fn rdp_full_matrix_is_exhaustive() {
    for &window_mode in WindowMode::all() {
        for &client_mode in RdpClientMode::all() {
            let conn = rdp_conn(window_mode, client_mode);
            let expected =
                window_mode == WindowMode::External || client_mode == RdpClientMode::External;
            assert_eq!(
                conn.uses_external_viewer(),
                expected,
                "RDP window_mode={window_mode:?} client_mode={client_mode:?}"
            );
        }
    }
}

/// Embedded VNC/RDP with an embedded client and non-External window is *not*
/// external — the common default case (#209 embedded path stays a tab).
#[test]
fn embedded_defaults_are_not_external() {
    assert!(!vnc_conn(WindowMode::Embedded, VncClientMode::Embedded).uses_external_viewer());
    assert!(!vnc_conn(WindowMode::Fullscreen, VncClientMode::Embedded).uses_external_viewer());
    assert!(!rdp_conn(WindowMode::Embedded, RdpClientMode::Embedded).uses_external_viewer());
    assert!(!rdp_conn(WindowMode::Fullscreen, RdpClientMode::Embedded).uses_external_viewer());
}

/// SPICE is external in every window mode (exhaustive).
#[test]
fn spice_external_in_every_window_mode() {
    for &window_mode in WindowMode::all() {
        assert!(
            spice_conn(window_mode).uses_external_viewer(),
            "SPICE must be external for window_mode={window_mode:?}"
        );
    }
}
