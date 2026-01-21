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
//! - [`protocol`] - Protocol trait and implementations (SSH, RDP, VNC, SPICE)
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
pub mod cluster;
pub mod config;
pub mod connection;
pub mod dashboard;
pub mod dialog_utils;
pub mod document;
pub mod drag_drop;
pub mod error;
pub mod export;
pub mod ffi;
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

pub use automation::{
    CompiledRule, ConnectionTask, ExpectEngine, ExpectError, ExpectResult, ExpectRule,
    FolderConnectionTracker, KeyElement, KeySequence, KeySequenceError, KeySequenceResult,
    SpecialKey, TaskCondition, TaskError, TaskExecutor, TaskResult, TaskTiming,
};
pub use cluster::{
    Cluster, ClusterError, ClusterManager, ClusterMemberState, ClusterResult, ClusterSession,
    ClusterSessionStatus, ClusterSessionSummary,
};
pub use config::{AppSettings, ConfigManager, ConnectionSettings, SecretBackendType};
pub use connection::{
    check_interning_stats, check_port, check_port_async, get_interning_stats,
    intern_connection_strings, intern_hostname, intern_protocol_name, intern_username,
    log_interning_stats, log_interning_stats_with_warning, ConnectionManager, LazyGroupLoader,
    PortCheckError, PortCheckResult, SelectionState, VirtualScrollConfig,
};
pub use dashboard::{DashboardFilter, SessionStats};
pub use document::{
    Document, DocumentError, DocumentManager, DocumentResult, DOCUMENT_FORMAT_VERSION,
};
pub use drag_drop::{
    calculate_drop_position, calculate_indicator_y, calculate_row_index, is_valid_drop_position,
    DropConfig, DropPosition, ItemType,
};
pub use error::{
    ConfigError, ConfigResult, ImportError, ProtocolError, RustConnError, SecretError, SessionError,
};
pub use export::{
    BatchExportCancelHandle, BatchExportResult, BatchExporter, ExportError, ExportFormat,
    ExportOptions, ExportResult, ExportTarget, NativeExport, NativeImportError,
    BATCH_EXPORT_THRESHOLD, DEFAULT_EXPORT_BATCH_SIZE, NATIVE_FILE_EXTENSION,
    NATIVE_FORMAT_VERSION,
};
pub use ffi::{
    ConnectionState, FfiDisplay, FfiError, FfiResult, VncCredentialType, VncDisplay, VncError,
};
pub use import::{
    AnsibleInventoryImporter, AsbruImporter, BatchCancelHandle, BatchImportResult, BatchImporter,
    ImportResult, ImportSource, RemminaImporter, RoyalTsImporter, SkippedEntry, SshConfigImporter,
    BATCH_IMPORT_THRESHOLD, DEFAULT_IMPORT_BATCH_SIZE,
};
pub use models::{
    group_templates_by_protocol, Connection, ConnectionGroup, ConnectionHistoryEntry,
    ConnectionStatistics, ConnectionTemplate, Credentials, CustomProperty, HistorySettings,
    PasswordSource, PropertyType, ProtocolConfig, ProtocolType, RdpConfig, RdpGateway, Resolution,
    Snippet, SnippetVariable, SpiceConfig, SpiceImageCompression, SshAuthMethod, SshConfig,
    SshKeySource, TemplateError, VncConfig, WindowGeometry, WindowMode,
};
pub use password_generator::{
    estimate_crack_time, CharacterSet, PasswordGenerator, PasswordGeneratorConfig,
    PasswordGeneratorError, PasswordGeneratorResult, PasswordStrength,
};
pub use performance::{
    format_bytes, memory_optimizer, metrics, AllocationStats, BatchProcessor, CompactString,
    Debouncer, InternerStats, LazyInit, MemoryBreakdown, MemoryEstimate, MemoryOptimizer,
    MemoryPressure, MemorySnapshot, MemoryTracker, ObjectPool, OperationStats,
    OptimizationCategory, OptimizationRecommendation, PerformanceMetrics, PoolStats, ShrinkableVec,
    StringInterner, TimingGuard, VirtualScroller,
};
pub use progress::{
    CallbackProgressReporter, CancelHandle, LocalProgressReporter, NoOpProgressReporter,
    ProgressReporter,
};
pub use protocol::{
    build_freerdp_args, detect_aws_cli, detect_azure_cli, detect_boundary, detect_cloudflared,
    detect_gcloud_cli, detect_oci_cli, detect_provider, detect_rdp_client, detect_ssh_client,
    detect_tailscale, detect_teleport, detect_vnc_client, extract_geometry_from_args,
    get_zero_trust_provider_icon, has_decorations_flag, ClientDetectionResult, ClientInfo,
    CloudProvider, FreeRdpConfig, Protocol, ProtocolRegistry, ProviderIconCache, RdpProtocol,
    SshProtocol, VncProtocol,
};
pub use rdp_client::{
    convert_to_bgra, create_frame_update, create_frame_update_with_conversion,
    input::{
        ctrl_alt_del_sequence,
        find_best_standard_resolution,
        generate_resize_request,
        is_modifier_keyval,
        is_printable_keyval,
        keycode_to_scancode,
        keyval_to_scancode,
        should_resize,
        CoordinateTransform,
        // Keyboard input
        RdpScancode,
        MAX_RDP_HEIGHT,
        MAX_RDP_WIDTH,
        MIN_RDP_HEIGHT,
        MIN_RDP_WIDTH,
        SCANCODE_ALT,
        SCANCODE_CTRL,
        SCANCODE_DELETE,
        STANDARD_RESOLUTIONS,
    },
    is_embedded_rdp_available, keyval_to_unicode, ClipboardFormatInfo, PixelFormat,
    RdpClientCommand, RdpClientConfig, RdpClientError, RdpClientEvent, RdpRect,
    RdpSecurityProtocol,
};
#[cfg(feature = "rdp-embedded")]
pub use rdp_client::{AudioFormatInfo, RdpClient, RdpCommandSender, RdpEventReceiver};
pub use search::{
    benchmark, cache::SearchCache, ConnectionSearchResult, DebouncedSearchEngine, MatchHighlight,
    SearchEngine, SearchError, SearchFilter, SearchQuery, SearchResult,
};
pub use secret::{
    parse_keepassxc_version, resolve_with_callback, spawn_credential_resolution,
    AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, CredentialResolver,
    CredentialStatus, CredentialVerificationManager, DialogPreFillData, GroupCreationResult,
    KdbxExporter, KeePassHierarchy, KeePassStatus, KeePassXcBackend, LibSecretBackend,
    PendingCredentialResolution, SecretBackend, SecretManager, VerifiedCredentials,
    KEEPASS_ROOT_GROUP,
};
pub use session::{
    LogConfig, LogContext, LogError, LogResult, Session, SessionLogger, SessionManager,
    SessionState, SessionType,
};
pub use snippet::SnippetManager;
pub use spice_client::{
    build_spice_viewer_args, detect_spice_viewer, is_embedded_spice_available, launch_spice_viewer,
    SpiceClientCommand, SpiceClientConfig, SpiceClientError, SpiceClientEvent, SpiceCompression,
    SpiceRect, SpiceSecurityProtocol, SpiceSharedFolder, SpiceViewerLaunchResult,
};
#[cfg(feature = "spice-embedded")]
pub use spice_client::{SpiceClient, SpiceClientState, SpiceCommandSender, SpiceEventReceiver};
// Split view types (tab-scoped layouts)
pub use split::SplitDirection;
pub use split::{
    ColorId, ColorPool, DropResult, LeafPanel, PanelId, PanelNode, SessionId as SplitSessionId,
    SplitError, SplitLayoutModel, SplitNode, TabId, SPLIT_COLORS,
};
pub use ssh_agent::{
    parse_agent_output, parse_key_list, AgentError, AgentKey, AgentResult, AgentStatus,
    SshAgentManager,
};
pub use testing::{
    ConnectionTester, TestError, TestResult, TestSummary, DEFAULT_CONCURRENCY,
    DEFAULT_TEST_TIMEOUT_SECS,
};
pub use tracing::{
    field_names, get_tracing_config, init_tracing, is_tracing_initialized, span_names,
    TracingConfig, TracingError, TracingLevel, TracingOutput, TracingResult,
};
pub use variables::{Variable, VariableError, VariableManager, VariableResult, VariableScope};
pub use vnc_client::is_embedded_vnc_available;
#[cfg(feature = "vnc-embedded")]
pub use vnc_client::{
    VncClient, VncClientCommand, VncClientConfig, VncClientError, VncClientEvent, VncCommandSender,
    VncEventReceiver, VncRect,
};
pub use wol::{
    generate_magic_packet, send_magic_packet, send_wol, MacAddress, WolConfig, WolError, WolResult,
    DEFAULT_BROADCAST_ADDRESS, DEFAULT_WOL_PORT, DEFAULT_WOL_WAIT_SECONDS, MAGIC_PACKET_SIZE,
};
