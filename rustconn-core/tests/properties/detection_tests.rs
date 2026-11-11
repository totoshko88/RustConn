//! Property-based tests for client detection and provider detection
//!
//! These tests validate the correctness properties for protocol client detection
//! and cloud provider detection as defined in the design document.
//!
//! **Feature: rustconn-bugfixes, Property 9: Client Detection**
//!
//! **Feature: rustconn-fixes-v2, Property 10: VNC Viewer Detection**
//!
//! **Feature: rustconn-fixes-v2, Property 4: AWS SSM Command Detection**
//!
//! **Feature: rustconn-fixes-v2, Property 5: GCloud Command Detection**

use proptest::prelude::*;
use rustconn_core::protocol::icons::{CloudProvider, detect_provider};
use rustconn_core::protocol::{
    ClientDetectionResult, ClientInfo, detect_rdp_client, detect_ssh_client, detect_vnc_client,
    detect_vnc_viewer_name, detect_vnc_viewer_path,
};

// ============================================================================
// Property Tests for Client Detection
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: rustconn-bugfixes, Property 9: Client Detection**
    //
    // For any installed client binary, detection SHALL return installed=true
    // with version string.

    /// Property: ClientInfo structure is consistent
    /// If installed is true, path must be Some
    /// If installed is false, install_hint should be Some
    #[test]
    fn prop_client_info_consistency(_seed in any::<u64>()) {
        // Test SSH client detection
        let ssh_info = detect_ssh_client();
        validate_client_info_consistency(&ssh_info);

        // Test RDP client detection
        let rdp_info = detect_rdp_client();
        validate_client_info_consistency(&rdp_info);

        // Test VNC client detection
        let vnc_info = detect_vnc_client();
        validate_client_info_consistency(&vnc_info);
    }

    /// Property: Detection results are deterministic
    /// Multiple calls should return the same result
    #[test]
    fn prop_detection_is_deterministic(_seed in any::<u64>()) {
        // SSH detection should be deterministic
        let ssh1 = detect_ssh_client();
        let ssh2 = detect_ssh_client();
        prop_assert_eq!(ssh1.installed, ssh2.installed);
        prop_assert_eq!(ssh1.name, ssh2.name);
        prop_assert_eq!(ssh1.path, ssh2.path);

        // RDP detection should be deterministic
        let rdp1 = detect_rdp_client();
        let rdp2 = detect_rdp_client();
        prop_assert_eq!(rdp1.installed, rdp2.installed);
        prop_assert_eq!(rdp1.name, rdp2.name);
        prop_assert_eq!(rdp1.path, rdp2.path);

        // VNC detection should be deterministic
        let vnc1 = detect_vnc_client();
        let vnc2 = detect_vnc_client();
        prop_assert_eq!(vnc1.installed, vnc2.installed);
        prop_assert_eq!(vnc1.name, vnc2.name);
        prop_assert_eq!(vnc1.path, vnc2.path);
    }

    /// Property: ClientDetectionResult contains all three protocols
    #[test]
    fn prop_detection_result_complete(_seed in any::<u64>()) {
        let result = ClientDetectionResult::detect_all();

        // All three clients should have non-empty names
        prop_assert!(!result.ssh.name.is_empty(), "SSH client name should not be empty");
        prop_assert!(!result.rdp.name.is_empty(), "RDP client name should not be empty");
        prop_assert!(!result.vnc.name.is_empty(), "VNC client name should not be empty");
    }

    /// Property: Installed clients have valid paths
    #[test]
    fn prop_installed_clients_have_valid_paths(_seed in any::<u64>()) {
        let ssh_info = detect_ssh_client();
        if ssh_info.installed {
            prop_assert!(ssh_info.path.is_some(), "Installed SSH client must have path");
            if let Some(path) = &ssh_info.path {
                prop_assert!(path.exists(), "SSH client path must exist: {:?}", path);
            }
        }

        let rdp_info = detect_rdp_client();
        if rdp_info.installed {
            prop_assert!(rdp_info.path.is_some(), "Installed RDP client must have path");
            if let Some(path) = &rdp_info.path {
                prop_assert!(path.exists(), "RDP client path must exist: {:?}", path);
            }
        }

        let vnc_info = detect_vnc_client();
        if vnc_info.installed {
            prop_assert!(vnc_info.path.is_some(), "Installed VNC client must have path");
            if let Some(path) = &vnc_info.path {
                prop_assert!(path.exists(), "VNC client path must exist: {:?}", path);
            }
        }
    }

    /// Property: Not installed clients have installation hints
    #[test]
    fn prop_not_installed_clients_have_hints(_seed in any::<u64>()) {
        let ssh_info = detect_ssh_client();
        if !ssh_info.installed {
            prop_assert!(
                ssh_info.install_hint.is_some(),
                "Not installed SSH client must have install hint"
            );
        }

        let rdp_info = detect_rdp_client();
        if !rdp_info.installed {
            prop_assert!(
                rdp_info.install_hint.is_some(),
                "Not installed RDP client must have install hint"
            );
        }

        let vnc_info = detect_vnc_client();
        if !vnc_info.installed {
            prop_assert!(
                vnc_info.install_hint.is_some(),
                "Not installed VNC client must have install hint"
            );
        }
    }

    // ========================================================================
    // **Feature: rustconn-fixes-v2, Property 10: VNC Viewer Detection**
    //
    // For any system with at least one VNC viewer installed, the detection
    // function should return a valid viewer path.
    // ========================================================================

    /// Property 10: VNC Viewer Detection
    /// For any system with at least one VNC viewer installed, the detection
    /// function should return a valid viewer path.
    #[test]
    fn prop_vnc_viewer_detection_consistency(_seed in any::<u64>()) {
        // Get VNC client info and viewer detection results
        let vnc_info = detect_vnc_client();
        let viewer_name = detect_vnc_viewer_name();
        let viewer_path = detect_vnc_viewer_path();

        // If VNC client is installed, viewer detection should also succeed
        if vnc_info.installed {
            prop_assert!(
                viewer_name.is_some(),
                "If VNC client is installed, detect_vnc_viewer_name() should return Some"
            );
            prop_assert!(
                viewer_path.is_some(),
                "If VNC client is installed, detect_vnc_viewer_path() should return Some"
            );

            // The path should exist
            if let Some(path) = &viewer_path {
                prop_assert!(
                    path.exists(),
                    "VNC viewer path should exist: {:?}",
                    path
                );
            }
        }

        // If viewer_name returns Some, viewer_path should also return Some
        if viewer_name.is_some() {
            prop_assert!(
                viewer_path.is_some(),
                "If viewer_name is Some, viewer_path should also be Some"
            );
        }

        // If viewer_path returns Some, viewer_name should also return Some
        if viewer_path.is_some() {
            prop_assert!(
                viewer_name.is_some(),
                "If viewer_path is Some, viewer_name should also be Some"
            );
        }
    }

    /// Property 10: VNC viewer detection is deterministic
    /// Multiple calls should return the same result
    #[test]
    fn prop_vnc_viewer_detection_deterministic(_seed in any::<u64>()) {
        let name1 = detect_vnc_viewer_name();
        let name2 = detect_vnc_viewer_name();
        prop_assert_eq!(name1, name2, "VNC viewer name detection should be deterministic");

        let path1 = detect_vnc_viewer_path();
        let path2 = detect_vnc_viewer_path();
        prop_assert_eq!(path1, path2, "VNC viewer path detection should be deterministic");
    }

    /// Property 10: VNC viewer name matches known viewers
    /// If a viewer is detected, it should be one of the known VNC viewers
    #[test]
    fn prop_vnc_viewer_is_known_viewer(_seed in any::<u64>()) {
        let known_viewers = [
            "vncviewer",
            "tigervnc",
            "gvncviewer",
            "xvnc4viewer",
            "vinagre",
            "remmina",
            "krdc",
        ];

        if let Some(viewer_name) = detect_vnc_viewer_name() {
            prop_assert!(
                known_viewers.contains(&viewer_name.as_str()),
                "Detected VNC viewer '{}' should be one of the known viewers: {:?}",
                viewer_name,
                known_viewers
            );
        }
    }

    // ========================================================================
    // **Feature: rustconn-fixes-v2, Property 4: AWS SSM Command Detection**
    //
    // For any command containing "aws ssm", "aws-ssm", or EC2 instance ID
    // patterns (i-*), the provider detection should return AWS.
    // ========================================================================

    /// Property 4: AWS SSM Command Detection
    /// For any command containing AWS SSM patterns, detection should return AWS
    #[test]
    fn prop_aws_ssm_command_detection(
        instance_id in "[a-f0-9]{8,17}",
        region in "(us|eu|ap)-(east|west|central|north|south|northeast|southeast)-[1-3]",
        profile in "[a-zA-Z][a-zA-Z0-9_-]{0,10}",
    ) {
        // Test various AWS SSM command patterns
        let commands = vec![
            format!("aws ssm start-session --target i-{instance_id}"),
            format!("aws ssm start-session --target i-{instance_id} --region {region}"),
            format!("aws ssm start-session --target i-{instance_id} --profile {profile}"),
            format!("/usr/bin/aws ssm start-session --target i-{instance_id}"),
            format!("aws-ssm start-session --target i-{instance_id}"),
        ];

        for cmd in &commands {
            let provider = detect_provider(cmd);
            prop_assert_eq!(
                provider,
                CloudProvider::Aws,
                "Command '{}' should be detected as AWS, got {:?}",
                cmd,
                provider
            );
        }
    }

    /// Property 4: AWS SSM instance ID pattern detection
    /// Commands with EC2 instance ID patterns should be detected as AWS
    #[test]
    fn prop_aws_instance_id_detection(
        instance_id in "[a-f0-9]{8,17}",
    ) {
        // Test instance ID patterns
        let commands = vec![
            format!("--target i-{instance_id}"),
            format!("--target=i-{instance_id}"),
            format!("ssm start-session --target i-{instance_id}"),
        ];

        for cmd in &commands {
            let provider = detect_provider(cmd);
            prop_assert_eq!(
                provider,
                CloudProvider::Aws,
                "Command with instance ID '{}' should be detected as AWS, got {:?}",
                cmd,
                provider
            );
        }
    }

    /// Property 4: AWS managed instance ID pattern detection
    /// Commands with managed instance ID patterns (mi-*) should be detected as AWS
    #[test]
    fn prop_aws_managed_instance_id_detection(
        instance_id in "[a-f0-9]{17}",
    ) {
        // Test managed instance ID patterns
        let commands = vec![
            format!("--target mi-{instance_id}"),
            format!("--target=mi-{instance_id}"),
            format!("ssm start-session --target mi-{instance_id}"),
        ];

        for cmd in &commands {
            let provider = detect_provider(cmd);
            prop_assert_eq!(
                provider,
                CloudProvider::Aws,
                "Command with managed instance ID '{}' should be detected as AWS, got {:?}",
                cmd,
                provider
            );
        }
    }

    // ========================================================================
    // **Feature: rustconn-fixes-v2, Property 5: GCloud Command Detection**
    //
    // For any command containing "gcloud" or "iap-tunnel", the provider
    // detection should return Google Cloud.
    // ========================================================================

    /// Property 5: GCloud Command Detection
    /// For any command containing GCloud patterns, detection should return GCloud
    #[test]
    fn prop_gcloud_command_detection(
        instance in "[a-z][a-z0-9-]{0,20}",
        zone in "(us|europe|asia)-(central|east|west|north|south)[1-9]-[a-c]",
        project in "[a-z][a-z0-9-]{0,20}",
    ) {
        // Test various GCloud command patterns
        let commands = vec![
            format!("gcloud compute ssh {instance} --zone {zone}"),
            format!("gcloud compute ssh {instance} --zone {zone} --project {project}"),
            format!("gcloud compute ssh {instance} --tunnel-through-iap"),
            format!("/usr/bin/gcloud compute ssh {instance}"),
            format!("gcloud compute start-iap-tunnel {instance} 22 --zone {zone}"),
        ];

        for cmd in &commands {
            let provider = detect_provider(cmd);
            prop_assert_eq!(
                provider,
                CloudProvider::Gcloud,
                "Command '{}' should be detected as GCloud, got {:?}",
                cmd,
                provider
            );
        }
    }

    /// Property 5: GCloud IAP tunnel detection
    /// Commands with IAP tunnel patterns should be detected as GCloud
    #[test]
    fn prop_gcloud_iap_tunnel_detection(
        instance in "[a-z][a-z0-9-]{0,20}",
        port in 1u16..65535u16,
    ) {
        // Test IAP tunnel patterns
        let commands = vec![
            format!("iap-tunnel {instance} {port}"),
            format!("--tunnel-through-iap"),
            format!("compute ssh {instance} --tunnel-through-iap"),
        ];

        for cmd in &commands {
            let provider = detect_provider(cmd);
            prop_assert_eq!(
                provider,
                CloudProvider::Gcloud,
                "Command with IAP tunnel '{}' should be detected as GCloud, got {:?}",
                cmd,
                provider
            );
        }
    }

    /// Property: Provider detection is deterministic
    /// Multiple calls with the same command should return the same result
    #[test]
    fn prop_provider_detection_deterministic(
        command in "[a-zA-Z0-9 /_-]{1,100}",
    ) {
        let result1 = detect_provider(&command);
        let result2 = detect_provider(&command);
        prop_assert_eq!(
            result1,
            result2,
            "Provider detection should be deterministic for command '{}'",
            command
        );
    }
}

/// Helper function to validate ClientInfo consistency
fn validate_client_info_consistency(info: &ClientInfo) {
    // Name should never be empty
    assert!(!info.name.is_empty(), "Client name should not be empty");

    if info.installed {
        // Installed clients must have a path
        assert!(
            info.path.is_some(),
            "Installed client '{}' must have a path",
            info.name
        );
        // Install hint is not needed for installed clients
    } else {
        // Not installed clients should have an install hint
        assert!(
            info.install_hint.is_some(),
            "Not installed client '{}' should have an install hint",
            info.name
        );
        // Path should be None for not installed clients
        assert!(
            info.path.is_none(),
            "Not installed client '{}' should not have a path",
            info.name
        );
    }
}

// ============================================================================
// Unit Tests for Client Detection
// ============================================================================

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_client_info_installed_constructor() {
        use std::path::PathBuf;

        let info = ClientInfo::installed(
            "Test",
            PathBuf::from("/usr/bin/test"),
            Some("1.0".to_string()),
        );
        assert!(info.installed);
        assert_eq!(info.name, "Test");
        assert_eq!(info.path, Some(PathBuf::from("/usr/bin/test")));
        assert_eq!(info.version, Some("1.0".to_string()));
        assert!(info.install_hint.is_none());
    }

    #[test]
    fn test_client_info_not_installed_constructor() {
        let info = ClientInfo::not_installed("Test", "Install with: apt install test");
        assert!(!info.installed);
        assert_eq!(info.name, "Test");
        assert!(info.path.is_none());
        assert!(info.version.is_none());
        assert_eq!(
            info.install_hint,
            Some("Install with: apt install test".to_string())
        );
    }

    #[test]
    fn test_detect_all_returns_three_clients() {
        let result = ClientDetectionResult::detect_all();

        // Should have all three protocol clients
        assert!(!result.ssh.name.is_empty());
        assert!(!result.rdp.name.is_empty());
        assert!(!result.vnc.name.is_empty());
    }

    #[test]
    fn test_ssh_detection_returns_valid_info() {
        let info = detect_ssh_client();

        // Name should be set
        assert!(!info.name.is_empty());

        // Consistency check
        if info.installed {
            assert!(info.path.is_some());
        } else {
            assert!(info.install_hint.is_some());
        }
    }

    #[test]
    fn test_rdp_detection_returns_valid_info() {
        let info = detect_rdp_client();

        // Name should be set
        assert!(!info.name.is_empty());

        // Consistency check
        if info.installed {
            assert!(info.path.is_some());
        } else {
            assert!(info.install_hint.is_some());
        }
    }

    #[test]
    fn test_vnc_detection_returns_valid_info() {
        let info = detect_vnc_client();

        // Name should be set
        assert!(!info.name.is_empty());

        // Consistency check
        if info.installed {
            assert!(info.path.is_some());
        } else {
            assert!(info.install_hint.is_some());
        }
    }

    // ========================================================================
    // Unit tests for VNC viewer detection (Property 10)
    // ========================================================================

    #[test]
    fn test_vnc_viewer_name_and_path_consistency() {
        // If name is Some, path should also be Some (and vice versa)
        let name = detect_vnc_viewer_name();
        let path = detect_vnc_viewer_path();

        if name.is_some() {
            assert!(
                path.is_some(),
                "If viewer name is detected, path should also be detected"
            );
        }

        if path.is_some() {
            assert!(
                name.is_some(),
                "If viewer path is detected, name should also be detected"
            );
        }
    }

    #[test]
    fn test_vnc_viewer_path_exists_if_detected() {
        if let Some(path) = detect_vnc_viewer_path() {
            assert!(
                path.exists(),
                "Detected VNC viewer path should exist: {:?}",
                path
            );
        }
    }

    #[test]
    fn test_vnc_viewer_name_is_known() {
        let known_viewers = [
            "vncviewer",
            "tigervnc",
            "gvncviewer",
            "xvnc4viewer",
            "vinagre",
            "remmina",
            "krdc",
        ];

        if let Some(name) = detect_vnc_viewer_name() {
            assert!(
                known_viewers.contains(&name.as_str()),
                "Detected viewer '{}' should be a known VNC viewer",
                name
            );
        }
    }

    #[test]
    fn test_vnc_client_and_viewer_detection_agree() {
        let client_info = detect_vnc_client();
        let viewer_name = detect_vnc_viewer_name();

        // If client is installed, viewer should be detected
        if client_info.installed {
            assert!(
                viewer_name.is_some(),
                "If VNC client is installed, viewer name should be detected"
            );
        }
    }
}

#[cfg(test)]
mod provider_icon_unit_tests {
    use super::*;

    #[test]
    fn test_cloud_provider_icon_names_are_unique() {
        let providers = CloudProvider::all();
        let icon_names: Vec<&str> = providers.iter().map(|p| p.icon_name()).collect();

        // Generic is the fallback, so it's okay if it's not unique
        let non_generic: Vec<&str> = icon_names
            .iter()
            .filter(|&&name| name != "system-run-symbolic")
            .copied()
            .collect();

        let unique_count = {
            let mut sorted = non_generic.clone();
            sorted.sort();
            sorted.dedup();
            sorted.len()
        };

        assert_eq!(
            non_generic.len(),
            unique_count,
            "Non-generic provider icon names should be unique"
        );
    }

    #[test]
    fn test_detect_provider_returns_correct_icon_name() {
        // Test that detect_provider returns providers with correct icon names
        // Note: We use standard GTK symbolic icons since provider-specific icons
        // (aws-symbolic, etc.) are not available in standard icon themes
        assert_eq!(
            detect_provider("aws ssm").icon_name(),
            "network-workgroup-symbolic"
        );
        assert_eq!(
            detect_provider("gcloud compute").icon_name(),
            "weather-overcast-symbolic"
        );
        assert_eq!(
            detect_provider("az network").icon_name(),
            "weather-few-clouds-symbolic"
        );
        assert_eq!(
            detect_provider("unknown command").icon_name(),
            "system-run-symbolic"
        );
    }
}

// ============================================================================
// Hoop.dev ZeroTrust Provider — Strategies and Property Tests
// ============================================================================

/// Strategy for generating arbitrary `HoopDevConfig` values.
///
/// - `connection_name`: non-empty string matching `[a-zA-Z0-9_-]{1,50}`
/// - `gateway_url`: `None` or a URL-like string
/// - `grpc_url`: `None` or a host:port string
fn arb_hoop_dev_config() -> impl Strategy<Value = rustconn_core::models::HoopDevConfig> {
    (
        "[a-zA-Z0-9_-]{1,50}",                                       // connection_name
        prop::option::of("https?://[a-z0-9.-]{1,30}(:[0-9]{2,5})?"), // gateway_url
        prop::option::of("[a-z0-9.-]{1,30}:[0-9]{2,5}"),             // grpc_url
    )
        .prop_map(|(connection_name, gateway_url, grpc_url)| {
            rustconn_core::models::HoopDevConfig {
                connection_name,
                gateway_url,
                grpc_url,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: hoop-dev-zerotrust, Property 1: HoopDevConfig serialization round-trip**
    //
    // For any valid HoopDevConfig, serializing to JSON then deserializing
    // must produce an equivalent HoopDevConfig.
    #[test]
    fn prop_hoop_dev_config_roundtrip(config in arb_hoop_dev_config()) {
        let json = serde_json::to_string(&config)
            .map_err(|e| proptest::test_runner::TestCaseError::fail(format!("serialize: {e}")))?;
        let parsed: rustconn_core::models::HoopDevConfig = serde_json::from_str(&json)
            .map_err(|e| proptest::test_runner::TestCaseError::fail(format!("deserialize: {e}")))?;
        prop_assert_eq!(config, parsed, "Round-trip must preserve HoopDevConfig");
    }

    // **Feature: hoop-dev-zerotrust, Property 2: None fields omitted from serialized JSON**
    //
    // When gateway_url or grpc_url is None, the serialized JSON must not
    // contain the corresponding key.
    #[test]
    fn prop_hoop_dev_none_fields_omitted(config in arb_hoop_dev_config()) {
        let json = serde_json::to_string(&config)
            .map_err(|e| proptest::test_runner::TestCaseError::fail(format!("serialize: {e}")))?;

        if config.gateway_url.is_none() {
            prop_assert!(
                !json.contains("gateway_url"),
                "JSON should not contain 'gateway_url' when it is None. Got: {json}"
            );
        }
        if config.grpc_url.is_none() {
            prop_assert!(
                !json.contains("grpc_url"),
                "JSON should not contain 'grpc_url' when it is None. Got: {json}"
            );
        }
    }

    // **Feature: hoop-dev-zerotrust, Property 3: Validation accepts valid configs and rejects empty connection_name**
    //
    // ZeroTrustConfig::validate() returns Ok(()) iff connection_name.trim()
    // is non-empty. Empty or whitespace-only names must be rejected.
    #[test]
    fn prop_hoop_dev_validation_correctness(config in arb_hoop_dev_config()) {
        use rustconn_core::models::{ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig};

        let zt = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(config.clone()),
            custom_args: vec![],
        };

        let result = zt.validate();
        // The arb_hoop_dev_config strategy always produces non-empty connection_name
        prop_assert!(
            result.is_ok(),
            "Valid HoopDevConfig should pass validation: {result:?}"
        );
    }

    // Companion: empty / whitespace connection_name must be rejected
    #[test]
    fn prop_hoop_dev_validation_rejects_empty(
        whitespace in "[ \\t]{0,10}",
        gateway_url in prop::option::of("https?://[a-z0-9.-]{1,20}"),
        grpc_url in prop::option::of("[a-z0-9.-]{1,20}:[0-9]{2,5}"),
    ) {
        use rustconn_core::models::{ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig};

        let cfg = rustconn_core::models::HoopDevConfig {
            connection_name: whitespace,
            gateway_url,
            grpc_url,
        };
        let zt = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(cfg),
            custom_args: vec![],
        };

        let result = zt.validate();
        prop_assert!(
            result.is_err(),
            "Empty/whitespace connection_name must be rejected"
        );
    }
}

// ============================================================================
// Hoop.dev Unit Tests for Detection and Flatpak Component
// ============================================================================

#[cfg(test)]
mod hoop_dev_tests {
    use rustconn_core::protocol::detect_hoop;

    #[test]
    fn test_hoop_detection_returns_valid_info() {
        let info = detect_hoop();
        // Name should always be set regardless of installation status
        assert!(
            !info.name.is_empty(),
            "Hoop.dev client name must not be empty"
        );

        if info.installed {
            assert!(info.path.is_some(), "Installed hoop must have a path");
        } else {
            assert!(
                info.install_hint.is_some(),
                "Not-installed hoop must have an install hint"
            );
        }
    }

    #[test]
    fn test_hoop_downloadable_component() {
        let component = rustconn_core::cli_download::get_component("hoop");
        assert!(
            component.is_some(),
            "hoop component must exist in DOWNLOADABLE_COMPONENTS"
        );

        let c = component.expect("checked above");
        assert_eq!(c.id, "hoop");
        assert_eq!(c.name, "Hoop.dev");
        assert_eq!(c.binary_name, "hoop");
        assert_eq!(c.install_subdir, "hoop");
        assert_eq!(
            c.category,
            rustconn_core::cli_download::ComponentCategory::ZeroTrust
        );
        assert!(c.works_in_sandbox, "hoop should work in sandbox");
        assert!(c.download_url.is_some(), "hoop must have a download URL");
    }
}
