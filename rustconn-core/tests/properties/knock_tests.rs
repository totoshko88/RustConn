//! Property-based tests for KnockSequence parsing and serialization
//!
//! Validates that knock sequences survive parse → display → parse roundtrip
//! and that SPA config serializes correctly.

use proptest::prelude::*;
use rustconn_core::connection::knock::{
    Knock, KnockProtocol, KnockSequence, SpaAllowIp, SpaConfig,
};

// ========== Generators ==========

fn arb_knock_protocol() -> impl Strategy<Value = KnockProtocol> {
    prop_oneof![Just(KnockProtocol::Tcp), Just(KnockProtocol::Udp),]
}

fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

fn arb_knock() -> impl Strategy<Value = Knock> {
    (arb_port(), arb_knock_protocol()).prop_map(|(port, protocol)| Knock { port, protocol })
}

fn arb_knock_sequence() -> impl Strategy<Value = KnockSequence> {
    prop::collection::vec(arb_knock(), 1..10).prop_map(|knocks| KnockSequence {
        knocks,
        delay_ms: 200,
        settle_ms: 500,
    })
}

fn arb_spa_allow_ip() -> impl Strategy<Value = SpaAllowIp> {
    prop_oneof![
        Just(SpaAllowIp::SourceIp),
        Just(SpaAllowIp::ResolvePublic),
        "[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}".prop_map(|ip| SpaAllowIp::Explicit(ip)),
    ]
}

fn arb_spa_access() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("tcp/22".to_string()),
        Just("tcp/22,tcp/443".to_string()),
        Just("udp/53".to_string()),
        "(tcp|udp)/[0-9]{1,5}".prop_map(|s| s),
    ]
}

fn arb_spa_config() -> impl Strategy<Value = SpaConfig> {
    (
        prop::option::of("[a-zA-Z0-9]{8,32}".prop_map(|s| s)),
        prop::option::of("[a-zA-Z0-9]{8,32}".prop_map(|s| s)),
        arb_spa_access(),
        1u16..=65535u16,
        arb_spa_allow_ip(),
    )
        .prop_map(
            |(rij_key, hmac_key, access, dest_port, allow_ip)| SpaConfig {
                rijndael_key_ref: rij_key,
                hmac_key_ref: hmac_key,
                rijndael_key_in_vault: false,
                hmac_key_in_vault: false,
                access,
                dest_port,
                allow_ip,
            },
        )
}

// ========== Tests ==========

proptest! {
    #[test]
    fn knock_sequence_display_parse_roundtrip(seq in arb_knock_sequence()) {
        let displayed = seq.display();
        let parsed = KnockSequence::parse(&displayed).expect("parse should succeed");
        prop_assert_eq!(seq.knocks.len(), parsed.knocks.len());
        for (orig, rest) in seq.knocks.iter().zip(parsed.knocks.iter()) {
            prop_assert_eq!(orig.port, rest.port);
            prop_assert_eq!(orig.protocol, rest.protocol);
        }
    }

    #[test]
    fn knock_sequence_toml_roundtrip(seq in arb_knock_sequence()) {
        let toml_str = toml::to_string(&seq).expect("serialize");
        let restored: KnockSequence = toml::from_str(&toml_str).expect("deserialize");
        prop_assert_eq!(seq.knocks.len(), restored.knocks.len());
        for (orig, rest) in seq.knocks.iter().zip(restored.knocks.iter()) {
            prop_assert_eq!(orig.port, rest.port);
            prop_assert_eq!(orig.protocol, rest.protocol);
        }
        prop_assert_eq!(seq.delay_ms, restored.delay_ms);
        prop_assert_eq!(seq.settle_ms, restored.settle_ms);
    }

    #[test]
    fn knock_from_tcp_ports_all_tcp(ports in prop::collection::vec(arb_port(), 1..8)) {
        let seq = KnockSequence::from_tcp_ports(&ports);
        prop_assert_eq!(seq.knocks.len(), ports.len());
        for (knock, &port) in seq.knocks.iter().zip(ports.iter()) {
            prop_assert_eq!(knock.port, port);
            prop_assert_eq!(knock.protocol, KnockProtocol::Tcp);
        }
    }

    #[test]
    fn spa_config_toml_roundtrip(cfg in arb_spa_config()) {
        let toml_str = toml::to_string(&cfg).expect("serialize");
        let restored: SpaConfig = toml::from_str(&toml_str).expect("deserialize");
        prop_assert_eq!(&cfg.rijndael_key_ref, &restored.rijndael_key_ref);
        prop_assert_eq!(&cfg.hmac_key_ref, &restored.hmac_key_ref);
        prop_assert_eq!(&cfg.access, &restored.access);
        prop_assert_eq!(cfg.dest_port, restored.dest_port);
        prop_assert_eq!(cfg.allow_ip, restored.allow_ip);
    }

    #[test]
    fn knock_parse_rejects_empty(input in "\\s*") {
        let result = KnockSequence::parse(&input);
        prop_assert!(result.is_err());
    }

    #[test]
    fn knock_parse_rejects_invalid_port(port in 65536u32..100_000u32) {
        let input = format!("{port}/tcp");
        let result = KnockSequence::parse(&input);
        prop_assert!(result.is_err());
    }
}
