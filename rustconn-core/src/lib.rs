//! `RustConn` Core Library
//!
//! This crate provides the core functionality for the `RustConn` connection manager,
//! including connection management, protocol handling, configuration, and import capabilities.
//!
//! # Crate Structure
//!
//! - [`models`] - Core data structures (Connection, Group, Protocol configs)
//! - [`config`] - Application settings and persistence
//! - [`connection`] - Connection CRUD operations and managers
//! - [`protocol`] - Protocol trait and implementations (SSH, RDP, VNC, SPICE, Telnet, Serial, SFTP, Kubernetes)
//! - [`import`] / [`export`] - Format converters (Remmina, Asbru-CM, SSH config, Ansible, MobaXterm)
//! - [`secret`] - Credential backends (`KeePassXC`, libsecret)
//! - [`search`] - Fuzzy search with caching and debouncing
//! - [`automation`] - Expect scripts, key sequences, tasks
//! - [`performance`] - Memory optimization, metrics, pooling
//!
//! # Feature Flags
//!
//! - `vnc-embedded` - Native VNC client via `vnc-rs` (default)
//! - `rdp-embedded` - Native RDP client via `IronRDP` (default)
//! - `spice-embedded` - Native SPICE client

// Enable missing_docs warning for public API documentation
#![warn(missing_docs)]

pub mod automation;
pub mod cli_download;
pub mod cluster;
pub mod config;
pub mod connection;
pub mod dialog_utils;
pub mod document;
pub mod drag_drop;
pub mod embedded_client_error;
pub mod error;
pub mod export;
pub mod ffi;
pub mod flatpak;
pub mod import;
pub mod models;
pub mod password_generator;
pub mod performance;
pub mod progress;
pub mod protocol;
pub mod rdp_client;
pub mod search;
pub mod secret;
pub mod session;
pub mod sftp;
pub mod snap;
pub mod snippet;
pub mod spice_client;
pub mod split;
pub mod ssh_agent;
pub mod terminal_themes;
pub mod testing;
pub mod tracing;
pub mod variables;
pub mod vnc_client;
pub mod wol;

// =============================================================================
// Convenience re-exports
//
// These flat re-exports exist for backward compatibility with property tests
// and integration tests. New code in `rustconn` (GUI) and `rustconn-cli`
// should import via modular paths (e.g. `rustconn_core::models::Connection`)
// rather than the flat namespace (`rustconn_core::Connection`).
// =============================================================================

pub use automation::{
    CompiledRule, ConnectionTask, ExpectEngine, ExpectError, ExpectResult, ExpectRule,
    FolderConnectionTracker, KeyElement, KeySequence, KeySequenceError, KeySequenceResult,
    SpecialKey, TaskCondition, TaskError, TaskExecutor, TaskResult, TaskTiming,
};
pub use cli_download::{
    ChecksumPolicy, CliDownloadError, CliDownloadResult, ComponentCategory,
    DOWNLOADABLE_COMPONENTS, DownloadCancellation, DownloadProgress, DownloadableComponent,
    InstallMethod, PackageManager, detect_package_manager, get_arch, get_available_components,
    get_cli_install_dir, get_component, get_components_by_category, get_installation_status,
    get_pinned_versions, get_system_install_command, get_user_friendly_error, install_component,
    uninstall_component,
};
pub use cluster::{
    Cluster, ClusterError, ClusterManager, ClusterMemberState, ClusterResult, ClusterSession,
    ClusterSessionStatus, ClusterSessionSummary,
};
pub use config::{
    AppSettings, ConfigManager, ConnectionSettings, SecretBackendType, StartupAction,
};
pub use connection::{
    ConnectionManager, LazyGroupLoader, PortCheckError, PortCheckResult, RetryConfig, RetryState,
    SelectionState, VirtualScrollConfig, check_interning_stats, check_port, check_port_async,
    get_interning_stats, intern_connection_strings, intern_hostname, intern_protocol_name,
    intern_username, log_interning_stats, log_interning_stats_with_warning,
};
pub use document::{
    DOCUMENT_FORMAT_VERSION, Document, DocumentError, DocumentManager, DocumentResult,
    EncryptionStrength,
};
pub use drag_drop::{
    DropConfig, DropPosition, ItemType, calculate_drop_position, calculate_indicator_y,
    calculate_row_index, is_valid_drop_position,
};
pub use embedded_client_error::EmbeddedClientError;
pub use error::{
    ConfigError, ConfigResult, ImportError, ProtocolError, RustConnError, SecretError,
    SessionError, SessionResult,
};
pub use export::{
    BATCH_EXPORT_THRESHOLD, BatchExportCancelHandle, BatchExportResult, BatchExporter,
    DEFAULT_EXPORT_BATCH_SIZE, ExportError, ExportFormat, ExportOptions, ExportResult,
    ExportTarget, NATIVE_FILE_EXTENSION, NATIVE_FORMAT_VERSION, NativeExport, NativeImportError,
};
pub use ffi::{
    ConnectionState, FfiDisplay, FfiError, FfiResult, VncCredentialType, VncDisplay, VncError,
};
pub use flatpak::is_flatpak;
// Deprecated flatpak-spawn functions (host_command, host_exec, host_has_command,
// host_spawn, host_which) are no longer re-exported since Flathub policy change in v0.7.7.
pub use import::{
    AnsibleInventoryImporter, AsbruImporter, BATCH_IMPORT_THRESHOLD, BatchCancelHandle,
    BatchImportResult, BatchImporter, DEFAULT_IMPORT_BATCH_SIZE, ImportResult, ImportSource,
    RemminaImporter, RoyalTsImporter, SkippedEntry, SshConfigImporter, VirtViewerImporter,
};
pub use models::{
    Connection, ConnectionGroup, ConnectionHistoryEntry, ConnectionStatistics, ConnectionTemplate,
    Credentials, CustomProperty, HistorySettings, KubernetesConfig, PasswordSource, PortForward,
    PortForwardDirection, PropertyType, ProtocolConfig, ProtocolType, RdpConfig, RdpGateway,
    Resolution, ScaleOverride, SerialBaudRate, SerialConfig, SerialDataBits, SerialFlowControl,
    SerialParity, SerialStopBits, Snippet, SnippetVariable, SpiceConfig, SpiceImageCompression,
    SshAuthMethod, SshConfig, SshKeySource, TelnetBackspaceSends, TelnetConfig, TelnetDeleteSends,
    TemplateError, VncConfig, WindowGeometry, WindowMode, group_templates_by_protocol,
};
pub use password_generator::{
    CharacterSet, PasswordGenerator, PasswordGeneratorConfig, PasswordGeneratorError,
    PasswordGeneratorResult, PasswordStrength, estimate_crack_time,
};
pub use performance::{
    AllocationStats, BatchProcessor, CompactString, Debouncer, InternerStats, LazyInit,
    MemoryBreakdown, MemoryEstimate, MemoryOptimizer, MemoryPressure, MemorySnapshot,
    MemoryTracker, ObjectPool, OperationStats, OptimizationCategory, OptimizationRecommendation,
    PerformanceMetrics, PoolStats, ShrinkableVec, StringInterner, TimingGuard, VirtualScroller,
    format_bytes, memory_optimizer, metrics,
};
pub use progress::{
    CallbackProgressReporter, CancelHandle, LocalProgressReporter, NoOpProgressReporter,
    ProgressReporter,
};
pub use protocol::{
    ClientDetectionResult, ClientInfo, CloudProvider, FreeRdpConfig, KubernetesProtocol, Protocol,
    ProtocolCapabilities, ProtocolRegistry, ProviderIconCache, RdpProtocol, SerialProtocol,
    SftpProtocol, SpiceProtocol, SshProtocol, TelnetProtocol, VncProtocol, build_freerdp_args,
    detect_aws_cli, detect_azure_cli, detect_boundary, detect_cloudflared, detect_gcloud_cli,
    detect_kubectl, detect_oci_cli, detect_picocom, detect_provider, detect_rdp_client,
    detect_ssh_client, detect_tailscale, detect_teleport, detect_telnet_client, detect_vnc_client,
    extract_geometry_from_args, get_protocol_icon_by_name, get_zero_trust_provider_icon,
    has_decorations_flag,
};
pub use rdp_client::keyboard_layout::{
    LAYOUT_US_ENGLISH, detect_keyboard_layout, xkb_name_to_klid,
};
#[cfg(feature = "rdp-embedded")]
pub use rdp_client::{AudioFormatInfo, RdpClient, RdpCommandSender, RdpEventReceiver};
pub use rdp_client::{
    ClipboardFormatInfo, PixelFormat, RdpClientCommand, RdpClientConfig, RdpClientError,
    RdpClientEvent, RdpRect, RdpSecurityProtocol, convert_to_bgra, create_frame_update,
    create_frame_update_with_conversion,
    input::{
        CoordinateTransform,
        MAX_RDP_HEIGHT,
        MAX_RDP_WIDTH,
        MIN_RDP_HEIGHT,
        MIN_RDP_WIDTH,
        // Keyboard input
        RdpScancode,
        SCANCODE_ALT,
        SCANCODE_CTRL,
        SCANCODE_DELETE,
        STANDARD_RESOLUTIONS,
        ctrl_alt_del_sequence,
        find_best_standard_resolution,
        generate_resize_request,
        is_modifier_keyval,
        is_printable_keyval,
        keycode_to_scancode,
        keyval_to_scancode,
        should_resize,
    },
    is_embedded_rdp_available, keyval_to_unicode,
};
pub use search::{
    ConnectionSearchResult, DebouncedSearchEngine, MatchHighlight, SearchEngine, SearchError,
    SearchFilter, SearchQuery, SearchResult, benchmark, cache::SearchCache,
};
pub use secret::{
    AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, CredentialResolver,
    CredentialStatus, CredentialVerificationManager, DialogPreFillData, GroupCreationResult,
    KEEPASS_ROOT_GROUP, KdbxExporter, KeePassHierarchy, KeePassStatus, KeePassXcBackend,
    LibSecretBackend, PassBackend, PendingCredentialResolution, SecretBackend, SecretManager,
    VerifiedCredentials, parse_keepassxc_version, resolve_with_callback,
    spawn_credential_resolution,
};
pub use session::{
    LogConfig, LogContext, LogError, LogResult, Session, SessionLogger, SessionManager,
    SessionState, SessionType,
};
pub use sftp::{
    build_mc_sftp_command, build_sftp_command, build_sftp_uri, build_sftp_uri_from_connection,
    ensure_key_in_agent, get_downloads_dir, get_ssh_key_path,
};
pub use snap::{
    get_config_dir, get_confinement_message, get_data_dir, get_known_hosts_path, get_ssh_dir,
    is_interface_connected, is_snap,
};
pub use snippet::SnippetManager;
#[cfg(feature = "spice-embedded")]
pub use spice_client::{SpiceClient, SpiceClientState, SpiceCommandSender, SpiceEventReceiver};
pub use spice_client::{
    SpiceClientCommand, SpiceClientConfig, SpiceClientError, SpiceClientEvent, SpiceCompression,
    SpiceRect, SpiceSecurityProtocol, SpiceSharedFolder, SpiceViewerLaunchResult,
    build_spice_viewer_args, detect_spice_viewer, is_embedded_spice_available, launch_spice_viewer,
};
// Split view types (tab-scoped layouts)
pub use split::SplitDirection;
pub use split::{
    ColorId, ColorPool, DropResult, LeafPanel, PanelId, PanelNode, SPLIT_COLORS,
    SessionId as SplitSessionId, SplitError, SplitLayoutModel, SplitNode, TabId,
};
pub use ssh_agent::{
    AgentError, AgentKey, AgentResult, AgentStatus, SshAgentManager, parse_agent_output,
    parse_key_list,
};
pub use testing::{
    ConnectionTester, DEFAULT_CONCURRENCY, DEFAULT_TEST_TIMEOUT_SECS, TestError, TestResult,
    TestSummary,
};
pub use tracing::{
    TracingConfig, TracingError, TracingLevel, TracingOutput, TracingResult, field_names,
    get_tracing_config, init_tracing, is_tracing_initialized, span_names,
};
pub use variables::{
    Variable, VariableError, VariableManager, VariableResult, VariableScope, variable_secret_key,
};
pub use vnc_client::is_embedded_vnc_available;
#[cfg(feature = "vnc-embedded")]
pub use vnc_client::{
    VncClient, VncClientCommand, VncClientConfig, VncClientError, VncClientEvent, VncCommandSender,
    VncEventReceiver, VncRect,
};
pub use wol::{
    DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS, MAGIC_PACKET_SIZE,
    MacAddress, WolConfig, WolError, WolResult, generate_magic_packet, send_magic_packet, send_wol,
};
