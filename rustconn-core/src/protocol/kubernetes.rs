//! Kubernetes protocol handler (kubectl exec)

use crate::error::ProtocolError;
use crate::models::{Connection, KubernetesConfig, ProtocolConfig};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// Kubernetes protocol handler
///
/// Implements the Protocol trait for Kubernetes pod shell connections.
/// Sessions are spawned via VTE terminal using `kubectl exec -it`.
pub struct KubernetesProtocol;

impl KubernetesProtocol {
    /// Creates a new Kubernetes protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts Kubernetes config from a connection
    fn get_k8s_config(connection: &Connection) -> ProtocolResult<&KubernetesConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Kubernetes(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not a Kubernetes connection".to_string(),
            )),
        }
    }
}

impl Default for KubernetesProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for KubernetesProtocol {
    fn protocol_id(&self) -> &'static str {
        "kubernetes"
    }

    fn display_name(&self) -> &'static str {
        "Kubernetes"
    }

    fn default_port(&self) -> u16 {
        0
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let config = Self::get_k8s_config(connection)?;

        // For exec mode, pod name is required
        if !config.use_busybox && config.pod.is_none() {
            return Err(ProtocolError::InvalidConfig(
                "Pod name is required for kubectl exec".to_string(),
            ));
        }

        // Shell must not be empty
        if config.shell.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Shell cannot be empty".to_string(),
            ));
        }

        // Busybox image must not be empty when busybox mode is on
        if config.use_busybox && config.busybox_image.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Busybox image cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::terminal()
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let config = Self::get_k8s_config(connection).ok()?;

        if config.use_busybox {
            Self::build_busybox_command(config)
        } else {
            Self::build_exec_command(config)
        }
    }
}

impl KubernetesProtocol {
    /// Builds `kubectl exec -it <pod> -- <shell>` command
    fn build_exec_command(config: &KubernetesConfig) -> Option<Vec<String>> {
        let mut cmd = vec!["kubectl".to_string()];

        Self::append_global_args(&mut cmd, config);

        cmd.push("exec".to_string());
        cmd.push("-it".to_string());

        // Pod name (required for exec)
        cmd.push(config.pod.as_ref()?.clone());

        // Container
        if let Some(ref container) = config.container {
            if !container.is_empty() {
                cmd.push("-c".to_string());
                cmd.push(container.clone());
            }
        }

        // Custom args
        for arg in &config.custom_args {
            cmd.push(arg.clone());
        }

        // Shell command after --
        cmd.push("--".to_string());
        cmd.push(config.shell.clone());

        Some(cmd)
    }

    /// Builds `kubectl run` command for temporary busybox pod
    fn build_busybox_command(config: &KubernetesConfig) -> Option<Vec<String>> {
        let mut cmd = vec!["kubectl".to_string()];

        Self::append_global_args(&mut cmd, config);

        cmd.push("run".to_string());
        cmd.push("-it".to_string());
        cmd.push("--rm".to_string());
        cmd.push("--restart=Never".to_string());

        // Generate a unique pod name
        cmd.push("rustconn-busybox".to_string());

        cmd.push("--image".to_string());
        cmd.push(config.busybox_image.clone());

        // Custom args
        for arg in &config.custom_args {
            cmd.push(arg.clone());
        }

        // Shell command after --
        cmd.push("--".to_string());
        cmd.push(config.shell.clone());

        Some(cmd)
    }

    /// Appends kubeconfig, context, and namespace args
    fn append_global_args(cmd: &mut Vec<String>, config: &KubernetesConfig) {
        if let Some(ref kubeconfig) = config.kubeconfig {
            cmd.push("--kubeconfig".to_string());
            cmd.push(kubeconfig.display().to_string());
        }

        if let Some(ref context) = config.context {
            if !context.is_empty() {
                cmd.push("--context".to_string());
                cmd.push(context.clone());
            }
        }

        if let Some(ref namespace) = config.namespace {
            if !namespace.is_empty() {
                cmd.push("--namespace".to_string());
                cmd.push(namespace.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProtocolConfig;

    fn create_k8s_connection(config: KubernetesConfig) -> Connection {
        Connection::new(
            "Test K8s".to_string(),
            String::new(),
            0,
            ProtocolConfig::Kubernetes(config),
        )
    }

    #[test]
    fn test_kubernetes_protocol_metadata() {
        let protocol = KubernetesProtocol::new();
        assert_eq!(protocol.protocol_id(), "kubernetes");
        assert_eq!(protocol.display_name(), "Kubernetes");
        assert_eq!(protocol.default_port(), 0);
    }

    #[test]
    fn test_validate_valid_exec_connection() {
        let protocol = KubernetesProtocol::new();
        let config = KubernetesConfig {
            pod: Some("my-pod".to_string()),
            ..Default::default()
        };
        let conn = create_k8s_connection(config);
        assert!(protocol.validate_connection(&conn).is_ok());
    }

    #[test]
    fn test_validate_missing_pod() {
        let protocol = KubernetesProtocol::new();
        let conn = create_k8s_connection(KubernetesConfig::default());
        assert!(protocol.validate_connection(&conn).is_err());
    }

    #[test]
    fn test_validate_busybox_no_pod_required() {
        let protocol = KubernetesProtocol::new();
        let config = KubernetesConfig {
            use_busybox: true,
            ..Default::default()
        };
        let conn = create_k8s_connection(config);
        assert!(protocol.validate_connection(&conn).is_ok());
    }

    #[test]
    fn test_build_exec_command() {
        let protocol = KubernetesProtocol::new();
        let config = KubernetesConfig {
            namespace: Some("production".to_string()),
            pod: Some("web-abc123".to_string()),
            container: Some("app".to_string()),
            ..Default::default()
        };
        let conn = create_k8s_connection(config);
        let cmd = protocol.build_command(&conn).unwrap();
        assert_eq!(cmd[0], "kubectl");
        assert!(cmd.contains(&"--namespace".to_string()));
        assert!(cmd.contains(&"production".to_string()));
        assert!(cmd.contains(&"exec".to_string()));
        assert!(cmd.contains(&"-it".to_string()));
        assert!(cmd.contains(&"web-abc123".to_string()));
        assert!(cmd.contains(&"-c".to_string()));
        assert!(cmd.contains(&"app".to_string()));
        assert!(cmd.contains(&"--".to_string()));
        assert!(cmd.contains(&"/bin/sh".to_string()));
    }

    #[test]
    fn test_build_busybox_command() {
        let protocol = KubernetesProtocol::new();
        let config = KubernetesConfig {
            use_busybox: true,
            busybox_image: "alpine:latest".to_string(),
            shell: "/bin/ash".to_string(),
            ..Default::default()
        };
        let conn = create_k8s_connection(config);
        let cmd = protocol.build_command(&conn).unwrap();
        assert_eq!(cmd[0], "kubectl");
        assert!(cmd.contains(&"run".to_string()));
        assert!(cmd.contains(&"--rm".to_string()));
        assert!(cmd.contains(&"--image".to_string()));
        assert!(cmd.contains(&"alpine:latest".to_string()));
        assert!(cmd.contains(&"/bin/ash".to_string()));
    }

    #[test]
    fn test_build_command_with_kubeconfig() {
        let protocol = KubernetesProtocol::new();
        let config = KubernetesConfig {
            kubeconfig: Some("/home/user/.kube/staging".into()),
            context: Some("staging-ctx".to_string()),
            pod: Some("my-pod".to_string()),
            ..Default::default()
        };
        let conn = create_k8s_connection(config);
        let cmd = protocol.build_command(&conn).unwrap();
        assert!(cmd.contains(&"--kubeconfig".to_string()));
        assert!(cmd.contains(&"/home/user/.kube/staging".to_string()));
        assert!(cmd.contains(&"--context".to_string()));
        assert!(cmd.contains(&"staging-ctx".to_string()));
    }
}
