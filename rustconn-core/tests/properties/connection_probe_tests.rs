//! Property tests for `Connection::bypasses_direct_probe()` and
//! `Connection::should_pre_connect_check()`.

use proptest::prelude::*;
use rustconn_core::config::ConnectionSettings;
use rustconn_core::models::{Connection, ProtocolConfig};

/// Strategy: generate a random SSH connection with optional jump_host_id and proxy_command
fn arb_ssh_connection() -> impl Strategy<Value = Connection> {
    (
        any::<bool>(), // has jump_host_id
        any::<bool>(), // has proxy_command
        any::<bool>(), // skip_port_check
    )
        .prop_map(|(has_jump, has_proxy_cmd, skip)| {
            let mut conn = Connection::new_ssh("test".to_string(), "example.com".to_string(), 22);
            if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
                if has_jump {
                    cfg.jump_host_id = Some(uuid::Uuid::new_v4());
                }
                if has_proxy_cmd {
                    cfg.proxy_command = Some("nc %h %p".to_string());
                }
            }
            conn.skip_port_check = skip;
            conn
        })
}

/// Strategy: generate a random RDP connection with optional jump_host_id and gateway
fn arb_rdp_connection() -> impl Strategy<Value = Connection> {
    (
        any::<bool>(), // has jump_host_id
        any::<bool>(), // has gateway
        any::<bool>(), // skip_port_check
    )
        .prop_map(|(has_jump, has_gateway, skip)| {
            let mut conn =
                Connection::new_rdp("test-rdp".to_string(), "rdp.example.com".to_string(), 3389);
            if let ProtocolConfig::Rdp(ref mut cfg) = conn.protocol_config {
                if has_jump {
                    cfg.jump_host_id = Some(uuid::Uuid::new_v4());
                }
                if has_gateway {
                    cfg.gateway = Some(rustconn_core::models::RdpGateway {
                        hostname: "gw.example.com".to_string(),
                        port: 443,
                        username: None,
                    });
                }
            }
            conn.skip_port_check = skip;
            conn
        })
}

proptest! {
    /// Property: SSH with jump_host_id always bypasses direct probe
    #[test]
    fn ssh_jump_host_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.jump_host_id = Some(uuid::Uuid::new_v4());
        }
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: SSH with proxy_command always bypasses direct probe
    #[test]
    fn ssh_proxy_command_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.proxy_command = Some("nc %h %p".to_string());
        }
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: plain SSH (no jump, no proxy) does NOT bypass probe
    #[test]
    fn plain_ssh_does_not_bypass(skip in any::<bool>()) {
        let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
        conn.skip_port_check = skip;
        prop_assert!(!conn.bypasses_direct_probe());
    }

    /// Property: RDP with gateway always bypasses direct probe
    #[test]
    fn rdp_gateway_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new_rdp("t".into(), "h".into(), 3389);
        if let ProtocolConfig::Rdp(ref mut cfg) = conn.protocol_config {
            cfg.gateway = Some(rustconn_core::models::RdpGateway {
                hostname: "gw".to_string(),
                port: 443,
                username: None,
            });
        }
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: ZeroTrust always bypasses direct probe
    #[test]
    fn zerotrust_always_bypasses(_dummy in 0..1) {
        let conn = Connection::new(
            "zt".to_string(),
            String::new(),
            0,
            ProtocolConfig::ZeroTrust(Default::default()),
        );
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: Web always bypasses direct probe
    #[test]
    fn web_always_bypasses(_dummy in 0..1) {
        let conn = Connection::new(
            "web".to_string(),
            "https://example.com".to_string(),
            443,
            ProtocolConfig::Web(Default::default()),
        );
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: should_pre_connect_check is false when global setting is disabled
    #[test]
    fn global_disabled_means_no_check(conn in arb_ssh_connection()) {
        let settings = ConnectionSettings {
            pre_connect_port_check: false,
            ..Default::default()
        };
        prop_assert!(!conn.should_pre_connect_check(&settings));
    }

    /// Property: should_pre_connect_check is false when skip_port_check is true
    #[test]
    fn skip_port_check_means_no_check(has_jump in any::<bool>()) {
        let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
        if has_jump
            && let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
                cfg.jump_host_id = Some(uuid::Uuid::new_v4());
            }
        conn.skip_port_check = true;
        let settings = ConnectionSettings::default();
        prop_assert!(!conn.should_pre_connect_check(&settings));
    }

    /// Property: should_pre_connect_check is false when connection bypasses probe
    #[test]
    fn bypass_means_no_check(conn in arb_rdp_connection()) {
        let settings = ConnectionSettings::default();
        if conn.bypasses_direct_probe() {
            prop_assert!(!conn.should_pre_connect_check(&settings));
        }
    }

    /// Property: plain connection with global enabled and skip=false → check is true
    #[test]
    fn plain_connection_checks(_dummy in 0..1) {
        let conn = Connection::new_ssh("t".into(), "h".into(), 22);
        let settings = ConnectionSettings::default();
        prop_assert!(conn.should_pre_connect_check(&settings));
    }

    /// Property: VNC with jump_host_id always bypasses direct probe
    #[test]
    fn vnc_jump_host_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new_vnc("t".into(), "h".into(), 5900);
        if let ProtocolConfig::Vnc(ref mut cfg) = conn.protocol_config {
            cfg.jump_host_id = Some(uuid::Uuid::new_v4());
        }
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: plain VNC (no jump) does NOT bypass probe
    #[test]
    fn plain_vnc_does_not_bypass(skip in any::<bool>()) {
        let mut conn = Connection::new_vnc("t".into(), "h".into(), 5900);
        conn.skip_port_check = skip;
        prop_assert!(!conn.bypasses_direct_probe());
    }

    /// Property: SPICE with jump_host_id always bypasses direct probe
    #[test]
    fn spice_jump_host_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new_spice("t".into(), "h".into(), 5900);
        if let ProtocolConfig::Spice(ref mut cfg) = conn.protocol_config {
            cfg.jump_host_id = Some(uuid::Uuid::new_v4());
        }
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: SPICE with proxy always bypasses direct probe
    #[test]
    fn spice_proxy_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new_spice("t".into(), "h".into(), 5900);
        if let ProtocolConfig::Spice(ref mut cfg) = conn.protocol_config {
            cfg.proxy = Some("spice-proxy.example.com:3128".to_string());
        }
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: plain SPICE (no jump, no proxy) does NOT bypass probe
    #[test]
    fn plain_spice_does_not_bypass(skip in any::<bool>()) {
        let mut conn = Connection::new_spice("t".into(), "h".into(), 5900);
        conn.skip_port_check = skip;
        prop_assert!(!conn.bypasses_direct_probe());
    }

    /// Property: RDP with jump_host_id always bypasses direct probe
    #[test]
    fn rdp_jump_host_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new_rdp("t".into(), "h".into(), 3389);
        if let ProtocolConfig::Rdp(ref mut cfg) = conn.protocol_config {
            cfg.jump_host_id = Some(uuid::Uuid::new_v4());
        }
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: SFTP shares SSH bypass logic (jump_host_id)
    #[test]
    fn sftp_jump_host_bypasses_probe(skip in any::<bool>()) {
        let mut conn = Connection::new(
            "sftp".to_string(),
            "h".to_string(),
            22,
            ProtocolConfig::Sftp(rustconn_core::models::SshConfig {
                jump_host_id: Some(uuid::Uuid::new_v4()),
                ..Default::default()
            }),
        );
        conn.skip_port_check = skip;
        prop_assert!(conn.bypasses_direct_probe());
    }

    /// Property: bypasses_direct_probe implies should_pre_connect_check is false
    /// (regardless of settings/skip_port_check)
    #[test]
    fn bypass_always_prevents_check(
        has_jump in any::<bool>(),
        has_proxy in any::<bool>(),
        skip in any::<bool>(),
        global_enabled in any::<bool>(),
    ) {
        let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            if has_jump {
                cfg.jump_host_id = Some(uuid::Uuid::new_v4());
            }
            if has_proxy {
                cfg.proxy_command = Some("nc %h %p".to_string());
            }
        }
        conn.skip_port_check = skip;
        let settings = ConnectionSettings {
            pre_connect_port_check: global_enabled,
            ..Default::default()
        };
        if conn.bypasses_direct_probe() {
            prop_assert!(!conn.should_pre_connect_check(&settings));
        }
    }
}
