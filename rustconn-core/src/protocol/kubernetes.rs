//! Kubernetes protocol handler (kubectl exec)

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, KubernetesConfig, ProtocolConfig};

/// Kubernetes protocol handler
///
/// Implements the Protocol trait for Kubernetes pod shell connections.
/// Sessions are spawned via VTE terminal using `kubectl exec -it`.
#[derive(Debug)]
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

/// The contexts a kubeconfig file defines, plus which one is current.
///
/// Populated by [`read_kubeconfig_contexts`] so the connection editor can offer
/// the contexts to pick from instead of making the user type one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KubeconfigContexts {
    /// Context names in file order, deduplicated.
    pub names: Vec<String>,
    /// The `current-context` value, if the file sets one.
    pub current: Option<String>,
}

/// Just the fields of a kubeconfig this reader cares about. A kubeconfig has
/// much more (clusters, users, auth), all of which `serde_yaml` ignores here.
#[derive(serde::Deserialize)]
struct KubeconfigDoc {
    #[serde(default)]
    contexts: Vec<KubeconfigContextEntry>,
    #[serde(rename = "current-context", default)]
    current_context: Option<String>,
}

#[derive(serde::Deserialize)]
struct KubeconfigContextEntry {
    #[serde(default)]
    name: String,
}

/// Resolves the kubeconfig path to read: the given one, else `$KUBECONFIG`'s
/// first entry, else `~/.kube/config`.
///
/// `$KUBECONFIG` may list several files separated by the platform path
/// separator; kubectl merges them, but for offering contexts in the editor the
/// first existing entry is enough, so only the first is read.
#[must_use]
pub fn resolve_kubeconfig_path(explicit: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    if let Some(env) = std::env::var_os("KUBECONFIG")
        && let Some(first) = std::env::split_paths(&env).next()
        && !first.as_os_str().is_empty()
    {
        return Some(first);
    }
    dirs::home_dir().map(|home| home.join(".kube").join("config"))
}

/// Reads the contexts defined in a kubeconfig file.
///
/// `explicit` is the path the connection names; `None` falls back to
/// `$KUBECONFIG` then `~/.kube/config` via [`resolve_kubeconfig_path`]. A
/// missing, unreadable or malformed file yields an empty result rather than an
/// error — the editor simply offers nothing to pick, and the user can still
/// type a context by hand. Names are returned in file order with blanks and
/// duplicates removed.
///
/// Reads YAML with `serde_yaml`, taking only `contexts[].name` and
/// `current-context`; every other kubeconfig field (clusters, users, tokens) is
/// ignored and never parsed into memory.
#[must_use]
pub fn read_kubeconfig_contexts(explicit: Option<&std::path::Path>) -> KubeconfigContexts {
    let Some(path) = resolve_kubeconfig_path(explicit) else {
        return KubeconfigContexts::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return KubeconfigContexts::default();
    };
    let Ok(doc) = serde_yaml::from_str::<KubeconfigDoc>(&text) else {
        tracing::debug!(path = %path.display(), "kubeconfig is not valid YAML; offering no contexts");
        return KubeconfigContexts::default();
    };

    let mut names: Vec<String> = Vec::new();
    for entry in doc.contexts {
        let name = entry.name.trim();
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    let current = doc
        .current_context
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    KubeconfigContexts { names, current }
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
    /// Returns `true` if the value contains shell metacharacters that could
    /// enable command injection when passed as a single argument to kubectl.
    fn contains_shell_metachar(value: &str) -> bool {
        value.contains(' ')
            || value.contains(';')
            || value.contains('|')
            || value.contains('&')
            || value.contains('\0')
            || value.contains('\n')
    }

    /// Builds `kubectl exec -it <pod> -- <shell>` command
    fn build_exec_command(config: &KubernetesConfig) -> Option<Vec<String>> {
        // Validate shell value before building command
        if Self::contains_shell_metachar(&config.shell) {
            tracing::warn!(
                shell = %config.shell,
                "Rejecting suspicious Kubernetes shell value"
            );
            return None;
        }

        let mut cmd = vec!["kubectl".to_string()];

        Self::append_global_args(&mut cmd, config);

        cmd.push("exec".to_string());
        cmd.push("-it".to_string());

        // Pod name (required for exec)
        cmd.push(config.pod.as_ref()?.clone());

        // Container
        if let Some(ref container) = config.container
            && !container.is_empty()
        {
            cmd.push("-c".to_string());
            cmd.push(container.clone());
        }

        // Custom args — filter out values with injection characters
        for arg in &config.custom_args {
            if arg.contains('\0') || arg.contains('\n') {
                tracing::warn!(arg = %arg, "Skipping Kubernetes custom arg with unsafe characters");
                continue;
            }
            cmd.push(arg.clone());
        }

        // Shell command after --
        cmd.push("--".to_string());
        cmd.push(config.shell.clone());

        Some(cmd)
    }

    /// Builds `kubectl run` command for temporary busybox pod
    fn build_busybox_command(config: &KubernetesConfig) -> Option<Vec<String>> {
        // Validate shell value before building command
        if Self::contains_shell_metachar(&config.shell) {
            tracing::warn!(
                shell = %config.shell,
                "Rejecting suspicious Kubernetes shell value"
            );
            return None;
        }

        // Validate busybox image — reject values with shell metacharacters
        if Self::contains_shell_metachar(&config.busybox_image) {
            tracing::warn!(
                image = %config.busybox_image,
                "Rejecting suspicious Kubernetes busybox image value"
            );
            return None;
        }

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

        // Custom args — filter out values with injection characters
        for arg in &config.custom_args {
            if arg.contains('\0') || arg.contains('\n') {
                tracing::warn!(arg = %arg, "Skipping Kubernetes custom arg with unsafe characters");
                continue;
            }
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

        if let Some(ref context) = config.context
            && !context.is_empty()
        {
            cmd.push("--context".to_string());
            cmd.push(context.clone());
        }

        if let Some(ref namespace) = config.namespace
            && !namespace.is_empty()
        {
            cmd.push("--namespace".to_string());
            cmd.push(namespace.clone());
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
    fn reads_context_names_and_current_from_a_kubeconfig() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = tmp.path().join("config");
        // A raw string so YAML indentation is preserved exactly (a `\`-continued
        // literal would eat the leading spaces and break the nesting).
        let kubeconfig = r"apiVersion: v1
current-context: staging
contexts:
- name: prod
  context: {cluster: c1, user: u1}
- name: staging
  context: {cluster: c2, user: u2}
- name: prod
  context: {cluster: c1, user: u1}
clusters: []
users: []
";
        std::fs::write(&path, kubeconfig).unwrap();

        let ctx = read_kubeconfig_contexts(Some(&path));
        // File order, duplicate "prod" collapsed.
        assert_eq!(ctx.names, vec!["prod".to_string(), "staging".to_string()]);
        assert_eq!(ctx.current.as_deref(), Some("staging"));
    }

    #[test]
    fn a_missing_or_malformed_kubeconfig_yields_no_contexts() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let missing = tmp.path().join("does-not-exist");
        assert_eq!(
            read_kubeconfig_contexts(Some(&missing)),
            KubeconfigContexts::default()
        );

        let malformed = tmp.path().join("bad");
        std::fs::write(&malformed, ": not : valid : yaml :\n\t- broken").unwrap();
        assert_eq!(
            read_kubeconfig_contexts(Some(&malformed)),
            KubeconfigContexts::default()
        );
    }

    #[test]
    fn explicit_path_wins_over_the_environment() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = tmp.path().join("explicit");
        std::fs::write(&path, "contexts:\n- name: only-ctx\n").unwrap();
        // resolve_kubeconfig_path returns the explicit path unchanged, so no
        // environment or home lookup happens.
        assert_eq!(
            resolve_kubeconfig_path(Some(&path)).as_deref(),
            Some(path.as_path())
        );
        assert_eq!(
            read_kubeconfig_contexts(Some(&path)).names,
            vec!["only-ctx"]
        );
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
