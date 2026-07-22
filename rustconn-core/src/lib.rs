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
//! - [`secret`] - Credential backends and resolvers; host keyring integration is optional
//! - [`search`] - Fuzzy search with caching and debouncing
//! - [`automation`] - Expect scripts, key sequences, tasks
//! - [`performance`] - Connection-string interner (dedup) + search debouncer
//!
//! # Feature Flags
//!
//! The default feature set is intentionally empty so `rustconn-core` can be used
//! as a headless domain library.
//!
//! - `system-keyring` - Host keyring integration (`oo7`/macOS Keychain)
//! - `vnc-embedded` - Native VNC client via `vnc-rs`
//! - `rdp-embedded` - Native RDP client via `IronRDP`
//! - `gfx-h264` - RDP EGFX/H.264 pipeline support
//! - `rd-gateway` - Native RD Gateway tunneling for embedded RDP
//!
//! SPICE sessions use an external viewer (virt-viewer/remote-viewer); the
//! native embedded SPICE client was removed in 0.18.0.

// Enable missing_docs warning for public API documentation
#![warn(missing_docs)]
// Inline test modules import items via `use super::*` and sometimes also via explicit
// `use crate::...` paths — the latter are flagged as redundant. Suppressing here avoids
// touching 146 inline test modules.
#![cfg_attr(
    test,
    allow(
        redundant_imports,
        clippy::print_stderr,
        reason = "test modules use super::* plus explicit imports; tests may print diagnostics"
    )
)]

// Domain model, persistence, and headless management modules.
pub mod activity_monitor;
pub mod automation;
pub mod busy;
pub mod cache;
pub mod cli_download;
pub mod cluster;
pub mod config;
pub mod connection;
pub mod dialog_utils;
pub mod display_geometry;
pub mod document;
pub mod drag_drop;
pub mod dynamic_folder;
pub mod embedded_client_error;
pub mod error;
pub mod export;
pub mod flatpak;
pub mod highlight;
pub mod host_check;
pub mod import;
pub mod models;
pub mod monitoring;
pub mod password_generator;
pub mod performance;
pub mod progress;
pub mod protocol;
pub mod search;
pub mod secret;
pub mod session;
pub mod sftp;
pub mod shell_escape;
pub mod smart_folder;
pub mod snap;
pub mod snippet;
pub mod split;
pub mod ssh_agent;
pub mod ssh_tunnel;
pub mod sync;
pub mod template;
pub mod terminal_themes;
pub mod testing;
pub mod tracing;
pub mod tunnel_manager;
pub mod tunnel_preview;
pub mod variables;
pub mod wol;

pub mod workspace;

// Optional desktop/client integration modules. These modules expose pure config
// and availability types without their heavy runtime dependencies unless the
// corresponding feature is enabled.
pub mod rdp_client;
pub mod spice_client;
pub mod vnc_client;

// =============================================================================
// Convenience re-exports
//
// These flat re-exports exist for backward compatibility with property tests
// and integration tests. New code in `rustconn` (GUI) and `rustconn-cli`
// should import via modular paths (e.g. `rustconn_core::models::Connection`)
// rather than the flat namespace (`rustconn_core::Connection`).
// =============================================================================

pub use activity_monitor::{ActivityMonitorConfig, ActivityMonitorDefaults, MonitorMode};
pub use automation::{
    AutomationTemplate, CompiledRule, ConnectionTask, ExpectEngine, ExpectError, ExpectResult,
    ExpectRule, FolderConnectionTracker, KeyElement, KeySequence, KeySequenceError,
    KeySequenceResult, SpecialKey, TaskCondition, TaskError, TaskExecutor, TaskResult, TaskTiming,
    builtin_templates, templates_for_protocol,
};
pub use busy::{BusyGuard, BusyStack};
pub use cache::{CacheRef, Cached, DEFAULT_CACHE_TTL_SECS, LoadCacheObject};
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
    AppSettings, ConfigManager, ConnectionSettings, KeybindingCategory, KeybindingDef,
    KeybindingSettings, SecretBackendType, StartupAction, default_keybindings,
    default_passthrough_exceptions, is_valid_accelerator,
};
pub use connection::{
    ConnectionManager, LazyGroupLoader, PortCheckError, PortCheckResult, RetryConfig, RetryState,
    SelectionState, check_interning_stats, check_port, check_port_async, get_interning_stats,
    intern_connection_strings, intern_hostname, intern_protocol_name, intern_username,
    log_interning_stats, log_interning_stats_with_warning, looks_like_password_prompt,
};
pub use display_geometry::{DesktopRequest, desktop_request_for_area};
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
pub use flatpak::{
    copy_key_to_flatpak_ssh, get_flatpak_known_hosts_path, get_flatpak_ssh_dir, is_flatpak,
    is_portal_path, resolve_key_path,
};
pub use highlight::{
    CompiledHighlightRules, HighlightMatch, Rgb, builtin_defaults, parse_hex_color,
};
// Deprecated flatpak-spawn functions (host_command, host_exec, host_has_command,
// host_spawn, host_which) are no longer re-exported since Flathub policy change in v0.7.7.
pub use import::{
    AnsibleInventoryImporter, AsbruImporter, BATCH_IMPORT_THRESHOLD, BatchCancelHandle,
    BatchImportResult, BatchImporter, DEFAULT_IMPORT_BATCH_SIZE, ImportResult, ImportSource,
    LibvirtXmlImporter, RdpFileImporter, RemminaImporter, RoyalTsImporter, SkippedEntry,
    SshConfigImporter, VirtViewerImporter,
};
pub use models::{
    Connection, ConnectionGroup, ConnectionHistoryEntry, ConnectionStatistics, ConnectionTemplate,
    Credentials, CustomProperty, DynamicConnectionEntry, DynamicFolderConfig, DynamicFolderResult,
    HighlightRule, HistorySettings, KubernetesConfig, MoshConfig, MoshPredictMode, PasswordSource,
    PortForward, PortForwardDirection, PropertyType, ProtocolConfig, ProtocolType, RdpConfig,
    RdpGateway, Resolution, ScaleOverride, SerialBaudRate, SerialConfig, SerialDataBits,
    SerialFlowControl, SerialParity, SerialStopBits, Snippet, SnippetTarget, SnippetVariable,
    SpiceConfig, SpiceImageCompression, SshAuthMethod, SshConfig, SshKeySource, StandaloneTunnel,
    TelnetBackspaceSends, TelnetConfig, TelnetDeleteSends, TemplateError, TunnelStatus, VncConfig,
    WindowGeometry, WindowMode, WorkspaceEntry, WorkspaceProfile, WorkspaceSplitLayout,
    collect_descendant_group_ids, group_templates_by_protocol,
};
pub use monitoring::{
    CollectorHandle, CpuSnapshot, DiskMetrics, LoadAverage, METRICS_COMMAND, MemoryMetrics,
    MetricsComputer, MetricsEvent, MetricsParser, MonitoringConfig, MonitoringError,
    MonitoringResult, MonitoringSettings, NetworkMetrics, NetworkSnapshot, RemoteMetrics,
    RemoteOsType, SYSTEM_INFO_COMMAND, SystemInfo, close_all_control_sockets, close_control_socket,
    close_dead_control_sockets, ssh_control_path, ssh_exec_factory, start_collector,
};
pub use password_generator::{
    CharacterSet, PasswordGenerator, PasswordGeneratorConfig, PasswordGeneratorError,
    PasswordGeneratorResult, PasswordStrength, estimate_crack_time,
};
pub use performance::{Debouncer, InternerStats, StringInterner, interner};
pub use progress::{
    CallbackProgressReporter, CancelHandle, LocalProgressReporter, NoOpProgressReporter,
    ProgressReporter,
};
pub use protocol::{
    ClientDetectionResult, ClientInfo, CloudProvider, FreeRdpConfig, KubernetesProtocol,
    MoshProtocol, PROTOCOL_TAB_CSS_CLASSES, Protocol, ProtocolCapabilities, ProtocolRegistry,
    ProviderIconCache, RdpProtocol, SerialProtocol, SftpProtocol, SpiceProtocol, SshProtocol,
    TelnetProtocol, VncProtocol, build_freerdp_args, detect_aws_cli, detect_azure_cli,
    detect_boundary, detect_cloudflared, detect_gcloud_cli, detect_hoop, detect_kubectl,
    detect_mosh, detect_oci_cli, detect_picocom, detect_provider, detect_rdp_client,
    detect_ssh_client, detect_tailscale, detect_teleport, detect_telnet_client, detect_vnc_client,
    extract_geometry_from_args, get_protocol_color_rgb, get_protocol_icon,
    get_protocol_icon_by_name, get_protocol_tab_css_class, get_zero_trust_provider_icon,
    has_decorations_flag,
};
pub use rdp_client::keyboard_layout::{
    LAYOUT_US_ENGLISH, detect_keyboard_layout, xkb_name_to_klid,
};
pub use rdp_client::quick_actions::{
    QUICK_ACTIONS, QuickAction, build_enter_sequence, build_hotkey_sequence, build_open_run_dialog,
    run_command_for,
};
pub use rdp_client::{
    AudioFormatInfo, ClipboardFormatInfo, PixelFormat, RdpClientCommand, RdpClientConfig,
    RdpClientError, RdpClientEvent, RdpRect, RdpSecurityProtocol, convert_to_bgra,
    create_frame_update, create_frame_update_with_conversion,
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
#[cfg(feature = "rdp-embedded")]
pub use rdp_client::{RdpClient, RdpCommandSender, RdpEventReceiver};
pub use search::cache::SearchCache;
pub use search::command_palette::{
    CommandPaletteAction, PaletteItem, PaletteMode, builtin_commands, parse_palette_input,
};
pub use search::{
    ConnectionSearchResult, DebouncedSearchEngine, MatchHighlight, SearchEngine, SearchError,
    SearchFilter, SearchQuery, SearchResult, benchmark,
};
// Host keyring backends are compiled only when explicitly requested. The
// headless default keeps DBus/macOS Security.framework out of rustconn-core.
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub use secret::LibSecretBackend;
pub use secret::{
    AsyncCredentialResolver, AsyncCredentialResult, CACHE_TTL_SECONDS, CancellationToken,
    CredentialResolver, CredentialStatus, CredentialVerificationManager, DialogPreFillData,
    GroupCreationResult, KEEPASS_ROOT_GROUP, KdbxExporter, KeePassHierarchy, KeePassStatus,
    PassBackend, PendingCredentialResolution, SecretBackend, SecretManager, VerifiedCredentials,
    parse_keepassxc_version, resolve_with_callback, spawn_credential_resolution,
};
pub use session::{
    LogConfig, LogContext, LogError, LogResult, Session, SessionLogger, SessionManager,
    SessionState, SessionType,
};
pub use sftp::{
    build_mc_sftp_command, build_sftp_browser_uri, build_sftp_command, build_sftp_uri,
    build_sftp_uri_from_connection, ensure_key_in_agent, get_downloads_dir, get_ssh_key_path,
    resolve_remote_home,
};
pub use snap::{
    get_config_dir, get_confinement_message, get_data_dir, get_known_hosts_path, get_ssh_dir,
    is_interface_connected, is_sandboxed, is_snap,
};
pub use snippet::SnippetManager;
pub use spice_client::{
    SpiceClientConfig, SpiceClientError, SpiceCompression, SpiceSecurityProtocol,
    SpiceSharedFolder, build_spice_viewer_args, detect_spice_viewer,
};
// Split view types (tab-scoped layouts)
pub use split::SplitDirection;
pub use split::{
    ColorId, ColorPool, DropResult, LeafPanel, PanelId, PanelNode, SPLIT_COLORS, SplitError,
    SplitLayoutModel, SplitNode, TabGroupManager, TabId,
};
pub use ssh_agent::{
    AgentError, AgentKey, AgentResult, AgentStatus, SshAgentManager, parse_agent_output,
    parse_key_list,
};
pub use sync::{
    Inventory, InventoryEntry, SYNC_TAG_PREFIX, SyncResult, default_port_for_protocol,
    load_inventory, parse_inventory_json, parse_inventory_yaml, sync_inventory, sync_tag,
};
pub use template::{
    PREDEFINED_TEMPLATES, PredefinedTemplate, TemplateCategory, TemplateManager,
    all_predefined_templates, find_predefined_template, templates_by_category,
};
pub use testing::{
    ConnectionTester, DEFAULT_CONCURRENCY, DEFAULT_TEST_TIMEOUT_SECS, TestError, TestResult,
    TestSummary,
};
pub use tracing::span_names;
pub use variables::{
    Variable, VariableError, VariableManager, VariableResult, VariableScope,
    variable_kdbx_lookup_key, variable_secret_key,
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
pub use workspace::WorkspaceProfileManager;
