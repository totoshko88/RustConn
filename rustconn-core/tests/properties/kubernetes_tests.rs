//! Property-based tests for Kubernetes protocol
//!
//! Tests KubernetesConfig creation, serialization round-trip,
//! protocol validation, and command building.

use proptest::prelude::*;
use rustconn_core::models::{Connection, KubernetesConfig, ProtocolConfig};
use rustconn_core::protocol::{KubernetesProtocol, Protocol};
use std::path::PathBuf;

// ============================================================================
// Strategies
// ============================================================================

fn arb_pod_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{2,20}"
}

fn arb_namespace() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("default".to_string()),
        Just("kube-system".to_string()),
        "[a-z][a-z0-9-]{2,15}".prop_map(|s| s),
    ]
}

fn arb_context() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("minikube".to_string()),
        Just("docker-desktop".to_string()),
        "[a-z][a-z0-9-]{2,20}".prop_map(|s| s),
    ]
}

fn arb_shell() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("/bin/sh".to_string()),
        Just("/bin/bash".to_string()),
        Just("/bin/ash".to_string()),
        Just("/bin/zsh".to_string()),
    ]
}

fn arb_busybox_image() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("busybox:latest".to_string()),
        Just("alpine:latest".to_string()),
        Just("alpine:3.19".to_string()),
        Just("ubuntu:22.04".to_string()),
    ]
}

fn arb_custom_args() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z0-9-]{1,10}", 0..3)
}

/// Strategy for exec-mode KubernetesConfig (pod required)
fn arb_k8s_exec_config() -> impl Strategy<Value = KubernetesConfig> {
    (
        prop::option::of(arb_context()),
        prop::option::of(arb_namespace()),
        arb_pod_name(),
        prop::option::of("[a-z][a-z0-9-]{2,10}".prop_map(|s| s)),
        arb_shell(),
        arb_custom_args(),
    )
        .prop_map(|(context, namespace, pod, container, shell, custom_args)| {
            KubernetesConfig {
                kubeconfig: None,
                context,
                namespace,
                pod: Some(pod),
                container,
                shell,
                use_busybox: false,
                busybox_image: "busybox:latest".to_string(),
                custom_args,
            }
        })
}

/// Strategy for busybox-mode KubernetesConfig
fn arb_k8s_busybox_config() -> impl Strategy<Value = KubernetesConfig> {
    (
        prop::option::of(arb_context()),
        prop::option::of(arb_namespace()),
        arb_shell(),
        arb_busybox_image(),
        arb_custom_args(),
    )
        .prop_map(
            |(context, namespace, shell, busybox_image, custom_args)| KubernetesConfig {
                kubeconfig: None,
                context,
                namespace,
                pod: None,
                container: None,
                shell,
                use_busybox: true,
                busybox_image,
                custom_args,
            },
        )
}

/// Strategy for any valid KubernetesConfig
fn arb_k8s_config() -> impl Strategy<Value = KubernetesConfig> {
    prop_oneof![arb_k8s_exec_config(), arb_k8s_busybox_config(),]
}

fn arb_k8s_connection() -> impl Strategy<Value = Connection> {
    (arb_k8s_config(), "[a-zA-Z][a-zA-Z0-9 _-]{0,20}").prop_map(|(config, name)| {
        let mut conn = Connection::new_kubernetes(name);
        conn.protocol_config = ProtocolConfig::Kubernetes(config);
        conn
    })
}

// ============================================================================
// Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Default KubernetesConfig has /bin/sh shell and busybox:latest image
    #[test]
    fn prop_default_k8s_config_values(_dummy in 0u32..1) {
        let config = KubernetesConfig::default();
        prop_assert_eq!(&config.shell, "/bin/sh");
        prop_assert_eq!(&config.busybox_image, "busybox:latest");
        prop_assert!(!config.use_busybox);
        prop_assert!(config.pod.is_none());
        prop_assert!(config.container.is_none());
        prop_assert!(config.kubeconfig.is_none());
        prop_assert!(config.context.is_none());
        prop_assert!(config.namespace.is_none());
        prop_assert!(config.custom_args.is_empty());
    }

    /// KubernetesConfig serialization round-trip preserves all fields
    #[test]
    fn prop_k8s_config_serde_roundtrip(config in arb_k8s_config()) {
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: KubernetesConfig = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&config, &deserialized);
    }

    /// Kubernetes connection round-trip preserves protocol config
    #[test]
    fn prop_k8s_connection_serde_roundtrip(conn in arb_k8s_connection()) {
        let json = serde_json::to_string(&conn).unwrap();
        let deserialized: Connection = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(conn.protocol, deserialized.protocol);
        prop_assert_eq!(conn.name, deserialized.name);
        if let (
            ProtocolConfig::Kubernetes(orig),
            ProtocolConfig::Kubernetes(deser),
        ) = (&conn.protocol_config, &deserialized.protocol_config)
        {
            prop_assert_eq!(orig, deser);
        } else {
            prop_assert!(false, "Expected Kubernetes protocol config");
        }
    }

    /// KubernetesProtocol validates exec connections with pod name
    #[test]
    fn prop_k8s_exec_validation_passes(config in arb_k8s_exec_config()) {
        let protocol = KubernetesProtocol::new();
        let mut conn = Connection::new_kubernetes("test".to_string());
        conn.protocol_config = ProtocolConfig::Kubernetes(config);
        prop_assert!(protocol.validate_connection(&conn).is_ok());
    }

    /// KubernetesProtocol validates busybox connections without pod name
    #[test]
    fn prop_k8s_busybox_validation_passes(config in arb_k8s_busybox_config()) {
        let protocol = KubernetesProtocol::new();
        let mut conn = Connection::new_kubernetes("test".to_string());
        conn.protocol_config = ProtocolConfig::Kubernetes(config);
        prop_assert!(protocol.validate_connection(&conn).is_ok());
    }

    /// KubernetesProtocol rejects exec mode without pod name
    #[test]
    fn prop_k8s_exec_rejects_missing_pod(
        shell in arb_shell()
    ) {
        let config = KubernetesConfig {
            pod: None,
            use_busybox: false,
            shell,
            ..KubernetesConfig::default()
        };
        let mut conn = Connection::new_kubernetes("test".to_string());
        conn.protocol_config = ProtocolConfig::Kubernetes(config);
        let protocol = KubernetesProtocol::new();
        prop_assert!(protocol.validate_connection(&conn).is_err());
    }

    /// KubernetesProtocol rejects empty shell
    #[test]
    fn prop_k8s_rejects_empty_shell(pod in arb_pod_name()) {
        let config = KubernetesConfig {
            pod: Some(pod),
            shell: String::new(),
            ..KubernetesConfig::default()
        };
        let mut conn = Connection::new_kubernetes("test".to_string());
        conn.protocol_config = ProtocolConfig::Kubernetes(config);
        let protocol = KubernetesProtocol::new();
        prop_assert!(protocol.validate_connection(&conn).is_err());
    }

    /// KubernetesProtocol rejects busybox mode with empty image
    #[test]
    fn prop_k8s_rejects_empty_busybox_image(
        shell in arb_shell()
    ) {
        let config = KubernetesConfig {
            use_busybox: true,
            busybox_image: String::new(),
            shell,
            ..KubernetesConfig::default()
        };
        let mut conn = Connection::new_kubernetes("test".to_string());
        conn.protocol_config = ProtocolConfig::Kubernetes(config);
        let protocol = KubernetesProtocol::new();
        prop_assert!(protocol.validate_connection(&conn).is_err());
    }

    /// build_command produces kubectl as first element for exec mode
    #[test]
    fn prop_k8s_exec_build_command_starts_with_kubectl(conn in arb_k8s_exec_config()
        .prop_map(|config| {
            let mut c = Connection::new_kubernetes("test".to_string());
            c.protocol_config = ProtocolConfig::Kubernetes(config);
            c
        })
    ) {
        let protocol = KubernetesProtocol::new();
        let cmd = protocol.build_command(&conn);
        let cmd = cmd.expect("Exec config should produce a command");
        prop_assert_eq!(&cmd[0], "kubectl");
        prop_assert!(cmd.contains(&"exec".to_string()));
        prop_assert!(cmd.contains(&"-it".to_string()));
        prop_assert!(cmd.contains(&"--".to_string()));
    }

    /// build_command produces kubectl run for busybox mode
    #[test]
    fn prop_k8s_busybox_build_command_uses_run(conn in arb_k8s_busybox_config()
        .prop_map(|config| {
            let mut c = Connection::new_kubernetes("test".to_string());
            c.protocol_config = ProtocolConfig::Kubernetes(config);
            c
        })
    ) {
        let protocol = KubernetesProtocol::new();
        let cmd = protocol.build_command(&conn);
        let cmd = cmd.expect("Busybox config should produce a command");
        prop_assert_eq!(&cmd[0], "kubectl");
        prop_assert!(cmd.contains(&"run".to_string()));
        prop_assert!(cmd.contains(&"--rm".to_string()));
        prop_assert!(cmd.contains(&"--image".to_string()));
    }

    /// build_command includes kubeconfig when set
    #[test]
    fn prop_k8s_build_command_includes_kubeconfig(
        pod in arb_pod_name(),
        path in "/tmp/[a-z]{3,10}/kubeconfig"
    ) {
        let config = KubernetesConfig {
            kubeconfig: Some(PathBuf::from(&path)),
            pod: Some(pod),
            ..KubernetesConfig::default()
        };
        let mut conn = Connection::new_kubernetes("test".to_string());
        conn.protocol_config = ProtocolConfig::Kubernetes(config);
        let protocol = KubernetesProtocol::new();
        let cmd = protocol.build_command(&conn).unwrap();
        prop_assert!(cmd.contains(&"--kubeconfig".to_string()));
        prop_assert!(cmd.contains(&path));
    }

    /// build_command shell is always the last element
    #[test]
    fn prop_k8s_build_command_shell_is_last(conn in arb_k8s_connection()) {
        let protocol = KubernetesProtocol::new();
        if let Some(cmd) = protocol.build_command(&conn)
            && let ProtocolConfig::Kubernetes(ref cfg) = conn.protocol_config {
                prop_assert_eq!(cmd.last().unwrap(), &cfg.shell);
            }
    }
}

// ============================================================================
// Non-proptest unit tests
// ============================================================================

#[test]
fn test_k8s_protocol_metadata() {
    let protocol = KubernetesProtocol::new();
    assert_eq!(protocol.protocol_id(), "kubernetes");
    assert_eq!(protocol.display_name(), "Kubernetes");
    assert_eq!(protocol.default_port(), 0);
}

#[test]
fn test_k8s_capabilities_are_terminal() {
    let protocol = KubernetesProtocol::new();
    let caps = protocol.capabilities();
    assert!(caps.terminal_based);
    assert!(caps.embedded);
    assert!(caps.split_view);
    assert!(!caps.file_transfer);
    assert!(!caps.audio);
}

#[test]
fn test_new_kubernetes_connection_has_correct_protocol() {
    let conn = Connection::new_kubernetes("Test K8s".to_string());
    assert_eq!(
        conn.protocol,
        rustconn_core::models::ProtocolType::Kubernetes
    );
    assert_eq!(conn.port, 0);
}
