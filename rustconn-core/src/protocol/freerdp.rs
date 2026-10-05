//! `FreeRDP` command builder for external mode RDP connections
//!
//! This module provides functions to build `FreeRDP` command-line arguments
//! for external mode RDP connections. It supports window decorations,
//! geometry persistence, and various RDP options.

use std::path::PathBuf;

use secrecy::SecretString;

use crate::models::{
    RdpAudioMode, RdpDisplayMode, RdpGateway, RdpSecurityLayer, Resolution, ScaleOverride,
    WindowGeometry, build_remote_app_freerdp_args,
};

/// A shared folder for RDP drive redirection
#[derive(Debug, Clone)]
pub struct SharedFolder {
    /// Local directory path to share
    pub local_path: PathBuf,
    /// Share name visible in the remote session
    pub share_name: String,
}

/// Configuration for `FreeRDP` external mode
///
/// This is the single input to [`build_freerdp_args`], which is the only place
/// in the workspace that decides what an external FreeRDP client is told. Both
/// GUI launch paths — the `External` client mode and the fallback taken when the
/// embedded IronRDP client cannot serve a connection — build one of these.
/// They used to own a hand-written argument list each, and the two lists drifted:
/// only one of them emitted `/gateway:`, only one sanitised the user's extra
/// arguments, and neither passed on the display scale or the colour depth the
/// connection editor collects.
#[derive(Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
pub struct FreeRdpConfig {
    /// Target hostname or IP address
    pub host: String,
    /// Target port (default: 3389)
    pub port: u16,
    /// Username for authentication
    pub username: Option<String>,
    /// Password for authentication
    pub password: Option<SecretString>,
    /// Domain for authentication
    pub domain: Option<String>,
    /// How the client window is sized.
    ///
    /// Only [`RdpDisplayMode::Custom`] reads `resolution`.
    pub display_mode: RdpDisplayMode,
    /// Fixed resolution for [`RdpDisplayMode::Custom`].
    ///
    /// `None` is honoured rather than substituted: a `Custom` mode with no
    /// resolution falls back to filling the screen, because FreeRDP's own
    /// default of `1024x768` is not a size any display has.
    pub resolution: Option<Resolution>,
    /// Remote DPI override. [`ScaleOverride::Auto`] sends none.
    pub scale_override: ScaleOverride,
    /// Live compositor scale as a percentage, read only for
    /// [`ScaleOverride::Native`]
    pub system_scale_percent: u16,
    /// Session colour depth (`/bpp:`). `None` leaves it to the server.
    pub color_depth: Option<u8>,
    /// Enable clipboard sharing
    pub clipboard_enabled: bool,
    /// Shared folders for drive redirection
    pub shared_folders: Vec<SharedFolder>,
    /// Map the local default printer into the session
    pub printer_enabled: bool,
    /// Where the session audio is played
    pub audio_mode: RdpAudioMode,
    /// RD Gateway to tunnel through, reusing the session credentials
    pub gateway: Option<RdpGateway>,
    /// `RemoteApp` program path or alias
    pub remote_app_program: Option<String>,
    /// `RemoteApp` command-line arguments
    pub remote_app_args: Option<String>,
    /// `RemoteApp` display name
    pub remote_app_name: Option<String>,
    /// Security layer selection
    pub security_layer: RdpSecurityLayer,
    /// TLS security level (0–5). `None` leaves FreeRDP's default.
    pub tls_security_level: Option<u8>,
    /// Disable Network Level Authentication while keeping other methods
    pub disable_nla: bool,
    /// Request dynamic desktop resizing (`/dynamic-resolution`).
    ///
    /// Mutually exclusive with smart sizing: FreeRDP rejects the two together,
    /// and a server that ignores MS-RDPEDISP gains nothing from it. Smart sizing
    /// wins when both are asked for, so this argument is dropped in that case —
    /// see [`FreeRdpSizing`] (issue #341).
    pub dynamic_resolution: bool,
    /// Scale the remote framebuffer to the client window (`/smart-sizing`).
    ///
    /// Lets a fixed-resolution session from a legacy server (e.g. Windows
    /// 2008 R2, which cannot do dynamic resolution) be resized by scaling its
    /// content, instead of staying unreadably small on a HiDPI display
    /// (issue #341).
    pub smart_sizing: bool,
    /// Additional `FreeRDP` arguments
    pub extra_args: Vec<String>,
    /// Window geometry for external mode
    pub window_geometry: Option<WindowGeometry>,
    /// Whether to remember window position
    pub remember_window_position: bool,
    /// Whether to ignore certificate errors (skip verification)
    pub ignore_certificate: bool,
    /// Enable FIDO2/WebAuthn device redirection
    pub fido2_enabled: bool,
    /// Explicit FreeRDP client binary. `None` auto-detects. Consumed by the
    /// launcher when choosing which binary to spawn, not by the argument
    /// builder (issue #340).
    pub client_override: Option<String>,
    /// The connection opted into Kerberos for NLA (`RdpConfig::kerberos_enabled`).
    ///
    /// FreeRDP's own negotiation already tries Kerberos before NTLM, so a
    /// desktop session needs no argument for it. This switch does two things
    /// on the external path: it lifts the NTLM-only restriction
    /// (`/auth-pkg-list:ntlm`) a `RemoteApp` session otherwise gets — an AD
    /// "Protected Users" account has NTLM disabled domain-wide and can never
    /// sign in through that list (issue #351) — and it gates
    /// [`Self::kdc_proxy_url`], exactly as on the embedded client.
    pub kerberos_enabled: bool,
    /// The connection's KDC Address (`RdpConfig::kdc_proxy_url`), read only
    /// while [`Self::kerberos_enabled`] is set.
    ///
    /// FreeRDP can only be pointed at an MS-KKDCP proxy, and only at one
    /// served under `/KdcProxy` — see [`freerdp_kerberos_arg`]. Any other
    /// address leaves the KDC to the system krb5 configuration.
    pub kdc_proxy_url: Option<String>,
}

/// Written by hand so that it agrees with [`FreeRdpConfig::new`].
///
/// `#[derive(Default)]` gave `port: 0`, `width: 0` and `clipboard_enabled:
/// false` — a configuration that cannot connect — while `new()` gives a working
/// one. The same divergence is documented for `RdpConfig` and `SpiceConfig` in
/// `models::protocol`; callers reach for `..Default::default()` and are entitled
/// to get the same baseline either way.
impl Default for FreeRdpConfig {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl FreeRdpConfig {
    /// Creates a new `FreeRDP` configuration with default settings
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 3389,
            username: None,
            password: None,
            domain: None,
            display_mode: RdpDisplayMode::default(),
            resolution: None,
            scale_override: ScaleOverride::default(),
            system_scale_percent: 100,
            color_depth: None,
            clipboard_enabled: true,
            shared_folders: Vec::new(),
            printer_enabled: false,
            audio_mode: RdpAudioMode::default(),
            gateway: None,
            remote_app_program: None,
            remote_app_args: None,
            remote_app_name: None,
            security_layer: RdpSecurityLayer::default(),
            tls_security_level: None,
            disable_nla: false,
            // Preserves the historical default: every external session asked
            // for dynamic resolution before it became configurable (issue #341).
            dynamic_resolution: true,
            smart_sizing: false,
            extra_args: Vec::new(),
            window_geometry: None,
            remember_window_position: true,
            ignore_certificate: false,
            fido2_enabled: false,
            client_override: None,
            kerberos_enabled: false,
            kdc_proxy_url: None,
        }
    }

    /// Sets how the client window is sized
    #[must_use]
    pub const fn with_display_mode(mut self, mode: RdpDisplayMode) -> Self {
        self.display_mode = mode;
        self
    }

    /// Returns whether this configuration launches a `RemoteApp` (RAIL) session
    #[must_use]
    pub fn is_remote_app(&self) -> bool {
        self.remote_app_program
            .as_ref()
            .is_some_and(|program| !program.is_empty())
    }

    /// Sets the port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the username
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the password
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(SecretString::from(password.into()));
        self
    }

    /// Sets the domain
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets a fixed resolution, only honoured by [`RdpDisplayMode::Custom`]
    #[must_use]
    pub const fn with_resolution(mut self, width: u32, height: u32) -> Self {
        self.resolution = Some(Resolution::new(width, height));
        self
    }

    /// Enables or disables clipboard sharing
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    /// Sets shared folders for drive redirection
    #[must_use]
    pub fn with_shared_folders(mut self, folders: Vec<SharedFolder>) -> Self {
        self.shared_folders = folders;
        self
    }

    /// Adds extra `FreeRDP` arguments
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Enables or disables dynamic desktop resizing (`/dynamic-resolution`)
    #[must_use]
    pub const fn with_dynamic_resolution(mut self, enabled: bool) -> Self {
        self.dynamic_resolution = enabled;
        self
    }

    /// Enables or disables framebuffer scaling to the window (`/smart-sizing`)
    #[must_use]
    pub const fn with_smart_sizing(mut self, enabled: bool) -> Self {
        self.smart_sizing = enabled;
        self
    }

    /// Sets the window geometry for external mode
    #[must_use]
    pub const fn with_window_geometry(mut self, geometry: WindowGeometry) -> Self {
        self.window_geometry = Some(geometry);
        self
    }

    /// Sets whether to remember window position
    #[must_use]
    pub const fn with_remember_window_position(mut self, remember: bool) -> Self {
        self.remember_window_position = remember;
        self
    }
}

/// Returns whether an argument contains a `FreeRDP` secret field.
///
/// Matching is case-insensitive for `p`, `password`, `gp`, `gateway-password`,
/// and `pth`. Top-level, standalone, colon-, equals-, whitespace-, and
/// comma-delimited fields are checked after leading ASCII whitespace, slashes,
/// and GNU-style option hyphens are removed.
#[must_use]
pub fn contains_freerdp_secret_field(arg: &str) -> bool {
    arg.split(',').any(|field| {
        let (name, _) = split_freerdp_field(field);
        is_freerdp_secret_name(name)
    })
}

/// Returns whether an argument selects a `FreeRDP` shell or proxy command.
///
/// Leading ASCII whitespace, slashes, and option hyphens are ignored, and
/// matching is case-insensitive across top-level and composite fields.
#[must_use]
pub fn is_freerdp_shell_or_proxy_arg(arg: &str) -> bool {
    arg.split(',').any(|field| {
        let (name, _) = split_freerdp_field(field);
        is_freerdp_shell_or_proxy_name(name)
    })
}

/// Returns whether a `FreeRDP` secret field consumes the following argument.
///
/// This detects standalone top-level fields and trailing standalone fields in
/// comma-delimited composites after the same normalization as the main helper.
#[must_use]
pub fn freerdp_secret_field_takes_following_value(arg: &str) -> bool {
    let trailing_field = arg.rsplit(',').next().unwrap_or(arg);
    let (name, has_value_delimiter) = split_freerdp_field(trailing_field);
    !has_value_delimiter && is_freerdp_secret_name(name)
}

fn normalize_freerdp_field(field: &str) -> &str {
    field.trim_start_matches(|character: char| {
        character.is_ascii_whitespace() || character == '/' || character == '-'
    })
}

fn split_freerdp_field(field: &str) -> (&str, bool) {
    let normalized = normalize_freerdp_field(field);
    normalized
        .find(|character: char| {
            character == ':' || character == '=' || character.is_ascii_whitespace()
        })
        .map_or((normalized, false), |name_end| {
            (&normalized[..name_end], true)
        })
}

fn is_freerdp_secret_name(name: &str) -> bool {
    const SECRET_FIELDS: [&str; 5] = ["p", "password", "gp", "gateway-password", "pth"];
    SECRET_FIELDS
        .iter()
        .any(|secret| name.eq_ignore_ascii_case(secret))
}

fn is_freerdp_shell_or_proxy_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("shell") || name.eq_ignore_ascii_case("proxy")
}

/// Returns whether the trailing blocked `FreeRDP` field consumes the next argument.
///
/// This covers standalone secret, shell, and proxy fields after the same
/// normalization used by the other `FreeRDP` argument helpers.
#[must_use]
pub fn is_standalone_freerdp_blocked_field(arg: &str) -> bool {
    let trailing_field = arg.rsplit(',').next().unwrap_or(arg);
    let (name, has_value_delimiter) = split_freerdp_field(trailing_field);
    !has_value_delimiter && (is_freerdp_secret_name(name) || is_freerdp_shell_or_proxy_name(name))
}

/// The smart-sizing switch as `xfreerdp3 /?` documents it,
/// `/smart-sizing[:<width>x<height>]`. Sent without a size, which scales the
/// session to the window.
const SMART_SIZING_FLAG: &str = "/smart-sizing";

/// Dynamic desktop resizing. FreeRDP documents the boolean as
/// `+dynamic-resolution`; the `/` form enables a boolean switch just the same
/// and is what every RustConn release has sent.
const DYNAMIC_RESOLUTION_FLAG: &str = "/dynamic-resolution";

/// How a `FreeRDP` launch sizes the remote desktop, once the connection's
/// switches and the user's custom arguments have both been taken into account.
///
/// FreeRDP refuses dynamic resolution and smart sizing together — the pair is
/// a command-line parse error (`-1002`, "mutually exclusive") and the client
/// exits before connecting. Smart sizing therefore wins wherever it comes from:
/// the connection's switch, or a custom argument such as `/smart-sizing` or
/// `/smart-sizing:1920x1080`. Before this also covered custom arguments, a
/// hand-written `/smart-sizing` still collided with the `/dynamic-resolution`
/// RustConn adds by default (issue #341).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeRdpSizing {
    /// The sizing switch RustConn adds itself, if any.
    pub flag: Option<&'static str>,
    /// Whether custom `dynamic-resolution` arguments have to be dropped.
    drop_custom_dynamic_resolution: bool,
}

impl FreeRdpSizing {
    /// Resolves the sizing from the two switches and the custom arguments.
    #[must_use]
    pub fn resolve(dynamic_resolution: bool, smart_sizing: bool, extra_args: &[String]) -> Self {
        let custom_smart_sizing = extra_args.iter().any(|arg| requests_smart_sizing(arg));
        if smart_sizing || custom_smart_sizing {
            // A custom `/smart-sizing` already asks for it, possibly with a
            // size, so RustConn does not add a second one.
            let flag = if custom_smart_sizing {
                None
            } else {
                Some(SMART_SIZING_FLAG)
            };
            return Self {
                flag,
                drop_custom_dynamic_resolution: true,
            };
        }
        let flag = if dynamic_resolution {
            Some(DYNAMIC_RESOLUTION_FLAG)
        } else {
            None
        };
        Self {
            flag,
            drop_custom_dynamic_resolution: false,
        }
    }

    /// Whether a custom argument has to be dropped for this sizing to launch.
    ///
    /// Only a `dynamic-resolution` argument, and only when smart sizing is on.
    /// It is dropped with a warning rather than failing the launch, like the
    /// other refused custom arguments.
    #[must_use]
    pub fn drops(&self, arg: &str) -> bool {
        self.drop_custom_dynamic_resolution && is_dynamic_resolution_arg(arg)
    }
}

/// Splits a top-level `FreeRDP` argument into its sigil and option name.
///
/// `/smart-sizing:1920x1080` is `('/', "smart-sizing")`, `+clipboard` is
/// `('+', "clipboard")`. `None` for anything that is not an option.
fn freerdp_option(arg: &str) -> Option<(char, &str)> {
    let arg = arg.trim_start();
    let sigil = arg.chars().next()?;
    if !matches!(sigil, '/' | '+' | '-') {
        return None;
    }
    // The sigil is one ASCII byte.
    let rest = &arg[1..];
    let name_end = rest.find([':', '=']).unwrap_or(rest.len());
    Some((sigil, &rest[..name_end]))
}

/// Whether a custom argument turns smart sizing on (`/` or `+`, any size).
fn requests_smart_sizing(arg: &str) -> bool {
    matches!(
        freerdp_option(arg),
        Some(('/' | '+', name)) if name.eq_ignore_ascii_case("smart-sizing")
    )
}

/// Whether a custom argument sets dynamic resolution, in either direction.
fn is_dynamic_resolution_arg(arg: &str) -> bool {
    matches!(
        freerdp_option(arg),
        Some((_, name)) if name.eq_ignore_ascii_case("dynamic-resolution")
    )
}

/// Builds `FreeRDP` command-line arguments from configuration
///
/// This function generates the command-line arguments for `FreeRDP` (xfreerdp/wlfreerdp)
/// based on the provided configuration. It includes:
/// - Authentication options (username, password, domain)
/// - Display options (resolution, dynamic resolution)
/// - Window options (decorations, geometry)
/// - Feature options (clipboard)
///
/// # Arguments
///
/// * `config` - The `FreeRDP` configuration
///
/// # Returns
///
/// A vector of command-line arguments for `FreeRDP`
#[must_use]
pub fn build_freerdp_args(config: &FreeRdpConfig) -> Vec<String> {
    let mut args = Vec::new();

    // Domain
    if let Some(ref domain) = config.domain
        && !domain.is_empty()
    {
        args.push(format!("/d:{domain}"));
    }

    // Username
    if let Some(ref username) = config.username {
        args.push(format!("/u:{username}"));
    }

    // Password — handled externally via ephemeral args file (/args-from:)
    // to survive RD Connection Broker redirects (issue #218). The password
    // never appears on argv or stdin. The caller is responsible for writing
    // the args file and passing the /args-from: switch separately.

    push_display_args(&mut args, config);

    // Certificate handling — conditional based on connection settings.
    // Default is TOFU (trust-on-first-use), matching SSH known_hosts behavior.
    if config.ignore_certificate {
        args.push("/cert:ignore".to_string());
    } else {
        args.push("/cert:tofu".to_string());
    }

    // Sizing behaviour. Dynamic resolution and smart sizing are mutually
    // exclusive in FreeRDP, so smart sizing wins — from the switch or from a
    // custom argument — and conflicting custom arguments are dropped in
    // `push_extra_args` (issue #341).
    let sizing = FreeRdpSizing::resolve(
        config.dynamic_resolution,
        config.smart_sizing,
        &config.extra_args,
    );
    if let Some(flag) = sizing.flag {
        args.push(flag.to_string());
    }

    // Decorations flag for window controls. Kept for every display mode: the
    // fullscreen and multi-monitor modes drop decorations themselves, and a
    // caller reading the argument list can still tell a windowed session was
    // asked for.
    args.push("/decorations".to_string());

    // Window geometry
    if config.remember_window_position
        && let Some(ref geometry) = config.window_geometry
    {
        args.push(format!("/x:{}", geometry.x));
        args.push(format!("/y:{}", geometry.y));
    }

    // Clipboard
    if config.clipboard_enabled {
        args.push("+clipboard".to_string());
    }

    push_redirection_args(&mut args, config);
    push_security_args(&mut args, config);

    // Audio routing is always stated explicitly. With no audio argument FreeRDP
    // leaves AudioPlayback and RemoteConsoleAudio both false, which Windows
    // reads as "no audio device in this session" — the user could neither hear
    // the session locally nor leave the sound on the remote machine (issue
    // #245). Emitted before extra_args so a hand-written /sound or /audio-mode
    // there still takes precedence.
    args.push(config.audio_mode.freerdp_arg().to_string());

    push_extra_args(&mut args, config, sizing);
    push_gateway_args(&mut args, config);
    push_remote_app_args(&mut args, config);

    // Server address (must be last)
    if config.port == 3389 {
        args.push(format!("/v:{}", config.host));
    } else {
        args.push(format!("/v:{}:{}", config.host, config.port));
    }

    args
}

/// Pushes the arguments that decide the session's size, DPI and colour depth.
fn push_display_args(args: &mut Vec<String>, config: &FreeRdpConfig) {
    args.extend(config.display_mode.freerdp_args(config.resolution.as_ref()));

    if let Some(depth) = config.color_depth {
        args.push(format!("/bpp:{depth}"));
    }

    args.extend(
        config
            .scale_override
            .freerdp_scale_args(config.system_scale_percent),
    );
}

/// Pushes drive and printer redirection arguments.
fn push_redirection_args(args: &mut Vec<String>, config: &FreeRdpConfig) {
    for folder in &config.shared_folders {
        // A share pointing at a path that no longer exists makes FreeRDP fail
        // the whole RDPDR channel, so it is skipped rather than passed on.
        if !folder.local_path.exists() {
            continue;
        }
        // FreeRDP `/drive:<name>,<path>` is comma-delimited; a comma in the
        // share name would split the argument and corrupt the path.
        let safe_name = folder.share_name.replace(',', "_");
        args.push(format!(
            "/drive:{safe_name},{}",
            folder.local_path.display()
        ));
    }

    // Map the local default printer into the session via CUPS.
    if config.printer_enabled {
        args.push("/printer".to_string());
    }

    // FIDO2/WebAuthn device redirection (FreeRDP 3.x).
    // Allows using local FIDO2 security keys for authentication in the remote session.
    if config.fido2_enabled {
        args.push("/fido".to_string());
    }
}

/// Pushes the security layer, TLS level and NLA arguments.
fn push_security_args(args: &mut Vec<String>, config: &FreeRdpConfig) {
    if let Some(security) = config.security_layer.freerdp_arg() {
        args.push(security.to_string());
    }

    // Level 0 enables TLS 1.0 for legacy servers; FreeRDP's own default is 1.
    if let Some(level) = config.tls_security_level {
        args.push(format!("/tls-seclevel:{level}"));
    }

    // FreeRDP 3.x syntax: disable NLA while leaving the other methods available.
    if config.disable_nla {
        args.push("/sec:nla:off".to_string());
    } else if let Some(kerberos) =
        freerdp_kerberos_arg(config.kerberos_enabled, config.kdc_proxy_url.as_deref())
    {
        // Without NLA there is no CredSSP exchange for Kerberos to take part in.
        args.push(kerberos);
    }
}

/// Path a Windows KDC proxy (MS-KKDCP) is served under, and the only one
/// FreeRDP can reach — see [`freerdp_kerberos_arg`].
const FREERDP_KDC_PROXY_PATH: &str = "/KdcProxy";

/// Returns the `FreeRDP` 3 `/kerberos:kdc-url:` argument for a KDC Address.
///
/// The option does not take a URL. FreeRDP's MIT krb5 glue
/// (`winpr/libwinpr/sspi/Kerberos/krb5glue_mit.c`) writes
/// `kdc = https://<value>/KdcProxy` into a private krb5 profile, so the value
/// must be the proxy's `host[:port]`. Only a stored `https://host[:port]/KdcProxy`
/// address can be expressed that way; a domain controller (`tcp://`, `udp://`),
/// a plain `http://` proxy or any other path cannot, and is left out with a
/// warning — the KDC then comes from the system krb5 configuration. `None`
/// as well when Kerberos is off, as on the embedded client, which reads the
/// address only then.
///
/// Shared by both external argument builders, [`build_freerdp_args`] and
/// `RdpProtocol::build_args`.
#[must_use]
pub fn freerdp_kerberos_arg(kerberos_enabled: bool, kdc_proxy_url: Option<&str>) -> Option<String> {
    if !kerberos_enabled {
        return None;
    }
    let normalized = match crate::rdp_client::normalize_kdc_url(kdc_proxy_url?) {
        Ok(normalized) => normalized?,
        Err(error) => {
            tracing::warn!(%error, "Ignoring an invalid KDC Address for FreeRDP");
            return None;
        }
    };
    let proxy = normalized.strip_prefix("https://").and_then(|rest| {
        let (authority, path) = rest.split_at(rest.find(['/', '?', '#'])?);
        let path = path.strip_suffix('/').unwrap_or(path);
        path.eq_ignore_ascii_case(FREERDP_KDC_PROXY_PATH)
            .then_some(authority)
    });
    let Some(authority) = proxy else {
        tracing::warn!(
            kdc_address = %normalized,
            "FreeRDP accepts only an https://<host>/KdcProxy KDC proxy; leaving the KDC to the system krb5 configuration"
        );
        return None;
    };
    Some(format!("/kerberos:kdc-url:{authority}"))
}

/// Pushes the user's extra arguments that [`filter_extra_args`] keeps.
fn push_extra_args(args: &mut Vec<String>, config: &FreeRdpConfig, sizing: FreeRdpSizing) {
    args.extend(filter_extra_args(&config.extra_args, sizing));
}

/// Returns a connection's custom `FreeRDP` arguments without the ones that are
/// unsafe or that `FreeRDP` would refuse.
///
/// Secret-bearing fields would put a credential on the `FreeRDP` argument
/// vector, and `/shell:`/`/proxy:` change what actually gets executed — the
/// arguments can come from an imported or synced profile, not only from the
/// user. A `dynamic-resolution` argument cannot go out beside smart sizing (see
/// [`FreeRdpSizing`]). All are dropped with a warning rather than failing the
/// launch, so a stale custom argument cannot lock a user out of a working
/// connection. Every launch path goes through this one filter: the embedded
/// wlfreerdp launch used to apply only the sizing rule, so the rest reached
/// `FreeRDP` there unchecked.
#[must_use]
pub fn filter_extra_args(extra_args: &[String], sizing: FreeRdpSizing) -> Vec<String> {
    let mut kept = Vec::with_capacity(extra_args.len());
    let mut skip_next_value = false;
    for arg in extra_args {
        if skip_next_value {
            skip_next_value = false;
            continue;
        }
        if contains_freerdp_secret_field(arg) || is_freerdp_shell_or_proxy_arg(arg) {
            skip_next_value = is_standalone_freerdp_blocked_field(arg);
            tracing::warn!("Blocked dangerous FreeRDP extra arg");
            continue;
        }
        if sizing.drops(arg) {
            tracing::warn!(
                argument = %arg,
                "Dropped a custom dynamic-resolution argument: smart sizing is on and FreeRDP refuses the two together"
            );
            continue;
        }
        kept.push(arg.clone());
    }
    kept
}

/// Pushes the RD Gateway argument.
///
/// FreeRDP 3.x removed the short `/g:` / `/gu:` / `/gp:` aliases in favour of
/// the unified `/gateway:` option (see xfreerdp3(1)); the old aliases are
/// rejected as "Unexpected keyword" and the client exits before connecting
/// (issue #187). FreeRDP reuses the session credentials (`/u:`, `/d:` and the
/// `/p:` from the args file) for the gateway, matching the working manual
/// command `xfreerdp /gateway:g:HOST /u:NAME /d:DOMAIN`. An explicit gateway
/// user is only added when it differs from the session user; a distinct gateway
/// account would also need its own password, which RustConn does not store yet.
fn push_gateway_args(args: &mut Vec<String>, config: &FreeRdpConfig) {
    let Some(ref gateway) = config.gateway else {
        return;
    };
    if gateway.hostname.is_empty() {
        return;
    }

    let mut value = format!("g:{}:{}", gateway.hostname, gateway.port);
    if let Some(ref gateway_user) = gateway.username
        && !gateway_user.is_empty()
        && config.username.as_deref() != Some(gateway_user.as_str())
    {
        value.push_str(",u:");
        value.push_str(gateway_user);
    }
    args.push(format!("/gateway:{value}"));
}

/// Pushes the `RemoteApp` (RAIL) arguments.
fn push_remote_app_args(args: &mut Vec<String>, config: &FreeRdpConfig) {
    args.extend(build_remote_app_freerdp_args(
        config.remote_app_program.as_deref(),
        config.remote_app_args.as_deref(),
        config.remote_app_name.as_deref(),
    ));

    // With RemoteApp on xfreerdp3, force NTLM authentication. xfreerdp3 on the
    // host often lacks Kerberos realm configuration, causing NLA to fail even
    // with correct credentials. NTLM works reliably for standalone (non-domain)
    // Windows servers. A connection that opted into Kerberos keeps FreeRDP's
    // own negotiation instead: an AD "Protected Users" account has NTLM
    // disabled domain-wide and could never sign in through the forced list
    // (issue #351).
    if config.is_remote_app() && !config.kerberos_enabled {
        args.push("/auth-pkg-list:ntlm".to_string());
    }
}

/// Checks if the `FreeRDP` arguments contain the decorations flag
///
/// # Arguments
///
/// * `args` - The `FreeRDP` command-line arguments
///
/// # Returns
///
/// `true` if the `/decorations` flag is present
#[must_use]
pub fn has_decorations_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "/decorations")
}

/// Extracts window geometry from `FreeRDP` arguments
///
/// # Arguments
///
/// * `args` - The `FreeRDP` command-line arguments
///
/// # Returns
///
/// The extracted window geometry if both `/x:` and `/y:` are present
#[must_use]
pub fn extract_geometry_from_args(args: &[String]) -> Option<(i32, i32)> {
    let mut x = None;
    let mut y = None;

    for arg in args {
        if let Some(val) = arg.strip_prefix("/x:") {
            x = val.parse().ok();
        } else if let Some(val) = arg.strip_prefix("/y:") {
            y = val.parse().ok();
        }
    }

    match (x, y) {
        (Some(x_val), Some(y_val)) => Some((x_val, y_val)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_freerdp_args_basic() {
        let config = FreeRdpConfig::new("server.example.com");
        let args = build_freerdp_args(&config);

        // The default display mode fills the monitor; `width`/`height` are only
        // read by `RdpDisplayMode::Custom`.
        assert!(args.contains(&"/size:100%".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("/w:")));
        assert!(args.contains(&"/decorations".to_string()));
        assert!(args.contains(&"/v:server.example.com".to_string()));
        // Dynamic resolution is on by default (the historical behaviour), and
        // smart-sizing is off (issue #341).
        assert!(args.contains(&"/dynamic-resolution".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("smart-sizing")));
    }

    #[test]
    fn test_build_freerdp_args_custom_resolution() {
        let config = FreeRdpConfig::new("server.example.com")
            .with_display_mode(RdpDisplayMode::Custom)
            .with_resolution(2560, 1440);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/w:2560".to_string()));
        assert!(args.contains(&"/h:1440".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("/size:")));
    }

    #[test]
    fn test_build_freerdp_args_fullscreen() {
        let config =
            FreeRdpConfig::new("server.example.com").with_display_mode(RdpDisplayMode::Fullscreen);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/f".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("/w:")));
    }

    #[test]
    fn test_build_freerdp_args_all_monitors() {
        let config =
            FreeRdpConfig::new("server.example.com").with_display_mode(RdpDisplayMode::AllMonitors);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/multimon".to_string()));
    }

    /// The colour depth and the display scale the connection editor collects
    /// used to reach the embedded viewer only; every external launch ignored
    /// them.
    #[test]
    fn test_build_freerdp_args_forwards_depth_and_scale() {
        let config = FreeRdpConfig {
            color_depth: Some(16),
            scale_override: ScaleOverride::Scale200,
            ..FreeRdpConfig::new("server.example.com")
        };
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/bpp:16".to_string()));
        assert!(args.contains(&"/scale-desktop:200".to_string()));
        assert!(args.contains(&"/scale-device:180".to_string()));
    }

    /// The `External` client mode never emitted a gateway argument, so a
    /// gateway-backed connection dialled the target host directly.
    #[test]
    fn test_build_freerdp_args_emits_gateway() {
        let config = FreeRdpConfig {
            gateway: Some(RdpGateway {
                hostname: "gw.example.com".to_string(),
                port: 443,
                username: Some("gwuser".to_string()),
            }),
            ..FreeRdpConfig::new("server.example.com").with_username("admin")
        };
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/gateway:g:gw.example.com:443,u:gwuser".to_string()));
    }

    /// A gateway user identical to the session user is redundant: FreeRDP
    /// already reuses the session credentials for the gateway.
    #[test]
    fn test_build_freerdp_args_omits_redundant_gateway_user() {
        let config = FreeRdpConfig {
            gateway: Some(RdpGateway {
                hostname: "gw.example.com".to_string(),
                port: 443,
                username: Some("admin".to_string()),
            }),
            ..FreeRdpConfig::new("server.example.com").with_username("admin")
        };
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/gateway:g:gw.example.com:443".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_security_layer_and_tls_level() {
        let config = FreeRdpConfig {
            security_layer: RdpSecurityLayer::Tls,
            tls_security_level: Some(0),
            disable_nla: true,
            ..FreeRdpConfig::new("server.example.com")
        };
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/sec:tls".to_string()));
        assert!(args.contains(&"/tls-seclevel:0".to_string()));
        assert!(args.contains(&"/sec:nla:off".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_remote_app_forces_ntlm() {
        let config = FreeRdpConfig {
            remote_app_program: Some("notepad.exe".to_string()),
            ..FreeRdpConfig::new("server.example.com")
        };
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/auth-pkg-list:ntlm".to_string()));
        assert!(args.iter().any(|arg| arg.starts_with("/app:")));
    }

    /// An AD "Protected Users" account has NTLM disabled, so a RemoteApp
    /// session that opted into Kerberos must not be restricted to NTLM
    /// (issue #351).
    #[test]
    fn remote_app_with_kerberos_leaves_the_auth_package_to_freerdp() {
        let config = FreeRdpConfig {
            remote_app_program: Some("notepad.exe".to_string()),
            kerberos_enabled: true,
            ..FreeRdpConfig::new("server.example.com")
        };
        let args = build_freerdp_args(&config);

        assert!(!args.iter().any(|arg| arg.starts_with("/auth-pkg-list")));
        assert!(args.iter().any(|arg| arg.starts_with("/app:")));
    }

    /// FreeRDP already negotiates Kerberos first, so with no KDC Address a
    /// desktop session gets the same command line either way.
    #[test]
    fn kerberos_without_remote_app_changes_no_argument() {
        let plain = FreeRdpConfig::new("server.example.com");
        let kerberos = FreeRdpConfig {
            kerberos_enabled: true,
            ..FreeRdpConfig::new("server.example.com")
        };

        assert_eq!(build_freerdp_args(&kerberos), build_freerdp_args(&plain));
        assert!(
            !build_freerdp_args(&plain)
                .iter()
                .any(|arg| arg.starts_with("/auth-pkg-list"))
        );
    }

    /// The KDC Address used to reach only the embedded client. An MS-KKDCP
    /// proxy now reaches the external one as FreeRDP 3's `/kerberos:kdc-url:`,
    /// which takes the proxy's host and appends `/KdcProxy` itself.
    #[test]
    fn kdc_proxy_reaches_the_external_client_as_kerberos_kdc_url() {
        let config = FreeRdpConfig {
            kerberos_enabled: true,
            kdc_proxy_url: Some("https://gw.example.com:8443/KdcProxy".to_string()),
            ..FreeRdpConfig::new("server.example.com")
        };
        let args = build_freerdp_args(&config);

        assert!(
            args.contains(&"/kerberos:kdc-url:gw.example.com:8443".to_string()),
            "{args:?}"
        );
        assert_eq!(
            args.last().map(String::as_str),
            Some("/v:server.example.com")
        );
    }

    /// The KDC Address is read only with Kerberos on, as on the embedded client,
    /// and not at all without NLA, which is what carries Kerberos.
    #[test]
    fn kdc_proxy_needs_kerberos_and_nla() {
        let url = Some("https://gw.example.com/KdcProxy".to_string());
        let kerberos_off = FreeRdpConfig {
            kdc_proxy_url: url.clone(),
            ..FreeRdpConfig::new("server.example.com")
        };
        let nla_off = FreeRdpConfig {
            kerberos_enabled: true,
            disable_nla: true,
            kdc_proxy_url: url,
            ..FreeRdpConfig::new("server.example.com")
        };

        for config in [kerberos_off, nla_off] {
            assert!(
                !build_freerdp_args(&config)
                    .iter()
                    .any(|arg| arg.starts_with("/kerberos")),
                "{config:?}"
            );
        }
    }

    /// FreeRDP cannot be pointed at a domain controller or at a proxy under
    /// another path, so those addresses produce no argument rather than a wrong
    /// one; `/KdcProxy` itself matches case-insensitively, with or without a
    /// trailing slash.
    #[test]
    fn freerdp_kerberos_arg_accepts_only_a_kdc_proxy() {
        for (stored, expected) in [
            ("https://gw.example.com/KdcProxy", Some("gw.example.com")),
            ("HTTPS://gw.example.com/kdcproxy/", Some("gw.example.com")),
            (
                "https://[2001:db8::1]:443/KdcProxy",
                Some("[2001:db8::1]:443"),
            ),
            ("https://gw.example.com", None),
            ("https://gw.example.com/Other/KdcProxy", None),
            ("https://gw.example.com/KdcProxy?x=1", None),
            ("http://gw.example.com/KdcProxy", None),
            ("dc1.example.com", None),
            ("tcp://dc1.example.com:88", None),
            ("udp://dc1.example.com:88", None),
            ("   ", None),
            ("ldap://dc1.example.com", None),
        ] {
            assert_eq!(
                freerdp_kerberos_arg(true, Some(stored)),
                expected.map(|authority| format!("/kerberos:kdc-url:{authority}")),
                "{stored}"
            );
        }
        assert_eq!(freerdp_kerberos_arg(true, None), None);
        assert_eq!(
            freerdp_kerberos_arg(false, Some("https://gw.example.com/KdcProxy")),
            None
        );
    }

    /// Audio is always stated: FreeRDP's implicit default is no audio device in
    /// the session at all (issue #245).
    #[test]
    fn test_build_freerdp_args_always_states_audio() {
        let args = build_freerdp_args(&FreeRdpConfig::new("server.example.com"));
        let audio = RdpAudioMode::default().freerdp_arg();

        assert!(args.contains(&audio.to_string()));
    }

    /// A share whose comma would split the `/drive:` argument must be
    /// neutralised, not passed on.
    #[test]
    fn test_build_freerdp_args_sanitises_share_name_commas() {
        let temp_dir = std::env::temp_dir();
        let config =
            FreeRdpConfig::new("server.example.com").with_shared_folders(vec![SharedFolder {
                share_name: "Home,Docs".to_string(),
                local_path: temp_dir.clone(),
            }]);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&format!("/drive:Home_Docs,{}", temp_dir.display())));
    }

    #[test]
    fn test_build_freerdp_args_with_credentials() {
        let config = FreeRdpConfig::new("server.example.com")
            .with_username("admin")
            .with_password("secret")
            .with_domain("CORP");
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/u:admin".to_string()));
        // Password is no longer on the arg list — it's passed via a
        // separate ephemeral args file (/args-from:file:) at launch time
        assert!(!args.iter().any(|a| a.starts_with("/p:")));
        assert!(!args.iter().any(|a| a == "/from-stdin"));
        assert!(args.contains(&"/d:CORP".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_with_geometry() {
        let geometry = WindowGeometry::new(100, 200, 1920, 1080);
        let config = FreeRdpConfig::new("server.example.com")
            .with_window_geometry(geometry)
            .with_remember_window_position(true);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/x:100".to_string()));
        assert!(args.contains(&"/y:200".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_geometry_disabled() {
        let geometry = WindowGeometry::new(100, 200, 1920, 1080);
        let config = FreeRdpConfig::new("server.example.com")
            .with_window_geometry(geometry)
            .with_remember_window_position(false);
        let args = build_freerdp_args(&config);

        // Geometry should NOT be included when remember_window_position is false
        assert!(!args.iter().any(|a| a.starts_with("/x:")));
        assert!(!args.iter().any(|a| a.starts_with("/y:")));
    }

    #[test]
    fn test_has_decorations_flag() {
        let args_with = vec!["/decorations".to_string(), "/v:host".to_string()];
        let args_without = vec!["/v:host".to_string()];

        assert!(has_decorations_flag(&args_with));
        assert!(!has_decorations_flag(&args_without));
    }

    #[test]
    fn test_extract_geometry_from_args() {
        let args = vec![
            "/x:100".to_string(),
            "/y:200".to_string(),
            "/v:host".to_string(),
        ];
        let geometry = extract_geometry_from_args(&args);
        assert_eq!(geometry, Some((100, 200)));

        let args_partial = vec!["/x:100".to_string(), "/v:host".to_string()];
        let geometry_partial = extract_geometry_from_args(&args_partial);
        assert_eq!(geometry_partial, None);
    }

    #[test]
    fn test_build_freerdp_args_custom_port() {
        let config = FreeRdpConfig::new("server.example.com").with_port(3390);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/v:server.example.com:3390".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_clipboard_disabled() {
        let config = FreeRdpConfig::new("server.example.com").with_clipboard(false);
        let args = build_freerdp_args(&config);

        assert!(!args.contains(&"+clipboard".to_string()));
    }

    #[test]
    fn test_build_freerdp_args_with_shared_folders() {
        // Create a temp directory that exists for the test
        let temp_dir = std::env::temp_dir();

        let folders = vec![
            SharedFolder {
                share_name: "Documents".to_string(),
                local_path: temp_dir.clone(),
            },
            SharedFolder {
                share_name: "Downloads".to_string(),
                local_path: temp_dir,
            },
        ];

        let config = FreeRdpConfig::new("server.example.com").with_shared_folders(folders);
        let args = build_freerdp_args(&config);

        // Check that drive arguments are present
        let drive_args: Vec<_> = args.iter().filter(|a| a.starts_with("/drive:")).collect();
        assert_eq!(drive_args.len(), 2);

        // Verify format: /drive:share_name,/path
        assert!(drive_args[0].starts_with("/drive:Documents,"));
        assert!(drive_args[1].starts_with("/drive:Downloads,"));
    }

    #[test]
    fn test_build_freerdp_args_shared_folders_nonexistent_path() {
        let folders = vec![SharedFolder {
            share_name: "NonExistent".to_string(),
            local_path: PathBuf::from("/nonexistent/path/that/does/not/exist"),
        }];

        let config = FreeRdpConfig::new("server.example.com").with_shared_folders(folders);
        let args = build_freerdp_args(&config);

        // Non-existent paths should be skipped
        assert!(!args.iter().any(|a| a.starts_with("/drive:")));
    }

    /// Counts the arguments that set a given `FreeRDP` option, whatever the sigil.
    fn count_option(args: &[String], name: &str) -> usize {
        args.iter()
            .filter(|arg| matches!(freerdp_option(arg), Some((_, option)) if option == name))
            .count()
    }

    #[test]
    fn dynamic_resolution_can_be_disabled() {
        let config = FreeRdpConfig::new("server.example.com").with_dynamic_resolution(false);
        let args = build_freerdp_args(&config);

        assert_eq!(count_option(&args, "dynamic-resolution"), 0);
        assert_eq!(count_option(&args, "smart-sizing"), 0);
    }

    #[test]
    fn smart_sizing_emits_documented_switch_and_suppresses_dynamic_resolution() {
        // Both requested: smart-sizing wins, dynamic-resolution is dropped so
        // FreeRDP does not reject the mutually exclusive pair (issue #341). The
        // pair, not the bare `/smart-sizing`, is what FreeRDP 3.x refuses:
        // `/smart-sizing[:WxH]` is the documented form.
        let config = FreeRdpConfig::new("server.example.com")
            .with_dynamic_resolution(true)
            .with_smart_sizing(true);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/smart-sizing".to_string()));
        assert_eq!(count_option(&args, "smart-sizing"), 1);
        assert_eq!(count_option(&args, "dynamic-resolution"), 0);
    }

    #[test]
    fn smart_sizing_alone_still_drops_dynamic_resolution() {
        let config = FreeRdpConfig::new("server.example.com")
            .with_dynamic_resolution(false)
            .with_smart_sizing(true);
        let args = build_freerdp_args(&config);

        assert!(args.contains(&"/smart-sizing".to_string()));
        assert_eq!(count_option(&args, "dynamic-resolution"), 0);
    }

    /// The reporter's own step: `/smart-sizing` typed as a custom argument,
    /// with Dynamic resolution left at its default. RustConn's
    /// `/dynamic-resolution` used to go out beside it and FreeRDP refused the
    /// command line (issue #341).
    #[test]
    fn custom_smart_sizing_argument_suppresses_default_dynamic_resolution() {
        for custom in ["/smart-sizing", "/smart-sizing:1920x1080", "+smart-sizing"] {
            let config =
                FreeRdpConfig::new("server.example.com").with_extra_args(vec![custom.to_string()]);
            let args = build_freerdp_args(&config);

            assert_eq!(count_option(&args, "dynamic-resolution"), 0, "{custom}");
            // The user's own argument is the only smart-sizing one, size intact.
            assert_eq!(count_option(&args, "smart-sizing"), 1, "{custom}");
            assert!(args.contains(&custom.to_string()), "{custom}");
        }
    }

    #[test]
    fn smart_sizing_switch_drops_custom_dynamic_resolution_only() {
        let config = FreeRdpConfig::new("server.example.com")
            .with_smart_sizing(true)
            .with_extra_args(vec![
                "+dynamic-resolution".to_string(),
                "/dynamic-resolution".to_string(),
                "/sound".to_string(),
            ]);
        let args = build_freerdp_args(&config);

        assert_eq!(count_option(&args, "dynamic-resolution"), 0);
        assert!(args.contains(&"/sound".to_string()));
        assert!(args.contains(&"/smart-sizing".to_string()));
    }

    #[test]
    fn custom_dynamic_resolution_is_kept_without_smart_sizing() {
        let config = FreeRdpConfig::new("server.example.com")
            .with_dynamic_resolution(false)
            .with_extra_args(vec!["/dynamic-resolution".to_string()]);
        let args = build_freerdp_args(&config);

        assert_eq!(count_option(&args, "dynamic-resolution"), 1);
    }

    /// Turning smart sizing *off* in a custom argument does not count as asking
    /// for it, so dynamic resolution stays.
    #[test]
    fn custom_smart_sizing_off_does_not_suppress_dynamic_resolution() {
        let config = FreeRdpConfig::new("server.example.com")
            .with_extra_args(vec!["-smart-sizing".to_string()]);
        let args = build_freerdp_args(&config);

        assert_eq!(count_option(&args, "dynamic-resolution"), 1);
    }

    #[test]
    fn sizing_resolution_for_the_embedded_client() {
        // The embedded client always asks for dynamic resolution, unless a
        // custom argument turns smart sizing on.
        let plain = FreeRdpSizing::resolve(true, false, &[]);
        assert_eq!(plain.flag, Some("/dynamic-resolution"));
        assert!(!plain.drops("/dynamic-resolution"));

        let custom = FreeRdpSizing::resolve(true, false, &["/smart-sizing".to_string()]);
        assert_eq!(custom.flag, None);
        assert!(custom.drops("+dynamic-resolution"));
        assert!(!custom.drops("/smart-sizing"));
        assert!(!custom.drops("/sound"));
    }

    /// The one filter every launch path uses; the embedded wlfreerdp launch
    /// used to apply only the sizing rule and passed the rest through.
    #[test]
    fn filter_extra_args_drops_secrets_shells_and_proxies_and_keeps_the_rest() {
        let custom: Vec<String> = [
            "/sound",
            "/p:hunter2",
            "/shell:calc.exe",
            "/proxy:http://proxy.example:8080",
            "/gateway:g:gw.example,p:composite-password",
            "+clipboard",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let sizing = FreeRdpSizing::resolve(true, false, &custom);

        assert_eq!(
            filter_extra_args(&custom, sizing),
            vec!["/sound".to_string(), "+clipboard".to_string()]
        );
    }

    /// A standalone secret flag takes the next argument as its value, and the
    /// value must go with it.
    #[test]
    fn filter_extra_args_drops_the_value_after_a_standalone_secret_flag() {
        let custom: Vec<String> = ["/p", "hunter2", "/sound"]
            .into_iter()
            .map(String::from)
            .collect();
        let sizing = FreeRdpSizing::resolve(true, false, &custom);

        assert_eq!(
            filter_extra_args(&custom, sizing),
            vec!["/sound".to_string()]
        );
    }

    #[test]
    fn filter_extra_args_applies_the_sizing_rule() {
        let custom: Vec<String> = ["/smart-sizing", "/dynamic-resolution"]
            .into_iter()
            .map(String::from)
            .collect();
        let sizing = FreeRdpSizing::resolve(true, false, &custom);

        assert_eq!(
            filter_extra_args(&custom, sizing),
            vec!["/smart-sizing".to_string()]
        );
    }
}

#[cfg(test)]
mod secret_argument_tests {
    use super::*;

    #[test]
    fn detects_top_level_and_composite_secret_aliases() {
        for argument in [
            " /P:session-secret",
            "//PASSWORD:password-secret",
            "/gateway:g:host, /Gp:gateway-secret",
            "/gateway:g:host,//GATEWAY-PASSWORD:alias-secret",
            "/gateway:g:host,PTH:hash-secret",
            "/p whitespace-secret",
            "/gateway:g:host,gp whitespace-gateway-secret",
        ] {
            assert!(
                contains_freerdp_secret_field(argument),
                "expected secret field in {argument:?}"
            );
        }

        for argument in [
            "/gateway:g:host,u:user",
            "/drive:Documents,/home/user/Documents",
            "/u:alice",
        ] {
            assert!(
                !contains_freerdp_secret_field(argument),
                "unexpected secret field in {argument:?}"
            );
        }
    }

    #[test]
    fn filters_secret_aliases_and_normalized_blocked_prefixes() {
        let blocked = [
            " /P:session-secret",
            "//PASSWORD:password-secret",
            "/gateway:g:host, /Gp:gateway-secret",
            "/gateway:g:host,//GATEWAY-PASSWORD:alias-secret",
            "/gateway:g:host,PTH:hash-secret",
            "/p whitespace-secret",
            "/gateway:g:host,gp whitespace-gateway-secret",
            "--password",
            "split-secret",
            "  /SHELL:command-secret",
            "\t//PrOxY:proxy-secret",
        ];
        let mut extra_args = blocked.iter().map(ToString::to_string).collect::<Vec<_>>();
        extra_args.push("/gateway:g:host,u:user".to_string());

        let args = build_freerdp_args(
            &FreeRdpConfig::new("server.example.com").with_extra_args(extra_args),
        );

        assert!(args.iter().any(|arg| arg == "/gateway:g:host,u:user"));
        for secret in [
            "session-secret",
            "password-secret",
            "gateway-secret",
            "alias-secret",
            "hash-secret",
            "whitespace-secret",
            "split-secret",
            "command-secret",
            "proxy-secret",
        ] {
            assert!(args.iter().all(|arg| !arg.contains(secret)));
        }
    }
}
