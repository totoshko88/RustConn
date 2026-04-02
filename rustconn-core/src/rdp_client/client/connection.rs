use super::super::audio::RustConnAudioBackend;
use super::super::clipboard::RustConnClipboardBackend;
use super::super::rdpdr::RustConnRdpdrBackend;
use super::super::{RdpClientConfig, RdpClientError, RdpClientEvent};
use ironrdp::cliprdr::CliprdrClient;
use ironrdp::connector::{
    BitmapConfig, ClientConnector, Config, ConnectionResult, Credentials, DesktopSize, ServerName,
};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::{
    BitmapCodecs, MajorPlatformType, client_codecs_capabilities,
};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::rdpdr::Rdpdr;
use ironrdp::rdpsnd::client::Rdpsnd;
use ironrdp_tokio::TokioFramed;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use secrecy::ExposeSecret;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use tokio::net::TcpStream;

pub type UpgradedFramed = TokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

/// Establishes the RDP connection and returns the framed stream and connection result.
///
/// # TLS Certificate Policy
///
/// IronRDP performs a TLS handshake but does not validate the server certificate
/// against a trusted CA store. This is standard practice for RDP — most RDP
/// servers use self-signed certificates. The behavior is equivalent to
/// `xfreerdp /cert:ignore`.
///
/// A future improvement could implement TOFU (Trust On First Use) by storing
/// the server certificate fingerprint on first connection and rejecting
/// changed certificates on subsequent connections.
// The future is not Send because IronRDP's AsyncNetworkClient is not Send.
// This is fine because we run on a single-threaded Tokio runtime.
#[allow(clippy::future_not_send)]
#[allow(clippy::too_many_lines)]
pub async fn establish_connection(
    config: &RdpClientConfig,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
) -> Result<(UpgradedFramed, ConnectionResult), RdpClientError> {
    use tokio::time::{Duration, timeout};

    let server_addr = config.server_address();
    let connect_timeout = Duration::from_secs(config.timeout_secs);

    // Phase 1: Establish TCP connection
    let tcp_result = timeout(connect_timeout, TcpStream::connect(&server_addr)).await;

    let stream = match tcp_result {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(RdpClientError::ConnectionFailed(format!(
                "Failed to connect to {server_addr}: {e}"
            )));
        }
        Err(_) => {
            return Err(RdpClientError::Timeout);
        }
    };

    let client_addr = stream
        .local_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));

    // Phase 2: Build IronRDP connector configuration
    let connector_config = build_connector_config(config);
    let mut connector = ClientConnector::new(connector_config, client_addr);

    // Phase 2.5: Add clipboard channel if enabled
    if config.clipboard_enabled {
        let clipboard_backend = RustConnClipboardBackend::new(event_tx.clone());
        let cliprdr: CliprdrClient = ironrdp::cliprdr::Cliprdr::new(Box::new(clipboard_backend));
        connector.static_channels.insert(cliprdr);
        tracing::debug!("Clipboard channel enabled");
    }

    // Phase 2.6: Add RDPDR channel for shared folders if configured
    // Note: RDPDR requires RDPSND channel to be present per MS-RDPEFS spec
    if !config.shared_folders.is_empty() {
        // Add RDPSND channel first (required for RDPDR)
        // Use real audio backend if audio is enabled, otherwise noop
        let rdpsnd = if config.audio_enabled {
            let audio_backend = RustConnAudioBackend::new(event_tx.clone());
            Rdpsnd::new(Box::new(audio_backend))
        } else {
            let audio_backend = RustConnAudioBackend::disabled(event_tx.clone());
            Rdpsnd::new(Box::new(audio_backend))
        };
        connector.static_channels.insert(rdpsnd);

        // Get computer name for display in Windows Explorer
        let computer_name = hostname::get().map_or_else(
            |_| "RustConn".to_string(),
            |h| h.to_string_lossy().into_owned(),
        );

        // Create initial drives list from shared folders config
        let initial_drives: Vec<(u32, String)> = config
            .shared_folders
            .iter()
            .enumerate()
            .map(|(idx, folder)| {
                let device_id = idx as u32 + 1;
                tracing::debug!(
                    "RDPDR: registering drive {} '{}' -> {:?}",
                    device_id,
                    folder.name,
                    folder.path
                );
                (device_id, folder.name.clone())
            })
            .collect();

        // Create backend for the first shared folder
        if let Some(folder) = config.shared_folders.first() {
            let base_path = folder.path.to_string_lossy().into_owned();
            let rdpdr_backend = RustConnRdpdrBackend::new(base_path);
            let rdpdr = Rdpdr::new(Box::new(rdpdr_backend), computer_name)
                .with_drives(Some(initial_drives));
            connector.static_channels.insert(rdpdr);
        }
    } else if config.audio_enabled {
        // No shared folders but audio is enabled - add RDPSND channel
        let audio_backend = RustConnAudioBackend::new(event_tx.clone());
        let rdpsnd = Rdpsnd::new(Box::new(audio_backend));
        connector.static_channels.insert(rdpsnd);
        tracing::debug!("Audio channel enabled (without RDPDR)");
    }

    // Phase 3: Perform RDP connection sequence (TLS + NLA + capabilities)
    // Wrap the entire handshake in a timeout — on heavily loaded servers the
    // TCP connect succeeds quickly but TLS/NLA can hang indefinitely.
    let handshake_timeout = Duration::from_secs(config.timeout_secs.saturating_mul(2).max(60));

    let handshake_result = timeout(handshake_timeout, async {
        let mut framed = TokioFramed::new(stream);

        // Begin connection (X.224 negotiation)
        let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
            .await
            .map_err(|e| {
                RdpClientError::ConnectionFailed(format!("Connection begin failed: {e}"))
            })?;

        // TLS upgrade - returns stream and server certificate.
        // Note: IronRDP does not validate the server certificate against a CA
        // store. This is equivalent to xfreerdp /cert:ignore and is standard
        // for RDP where most servers use self-signed certificates.
        let initial_stream = framed.into_inner_no_leftover();

        let (upgraded_stream, server_cert) = ironrdp_tls::upgrade(initial_stream, &config.host)
            .await
            .map_err(|e| RdpClientError::ConnectionFailed(format!("TLS upgrade failed: {e}")))?;

        tracing::warn!(
            protocol = "rdp",
            host = %config.host,
            port = %config.port,
            "TLS certificate not validated (no CA verification). \
             This is standard for RDP self-signed certificates."
        );

        // Extract server public key from certificate
        let server_public_key = ironrdp_tls::extract_tls_server_public_key(&server_cert)
            .map(<[u8]>::to_vec)
            .unwrap_or_default();

        let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

        let mut upgraded_framed = TokioFramed::new(upgraded_stream);

        // Create network client for Kerberos/AAD authentication
        let mut network_client = ReqwestNetworkClient::new();

        // Log connection parameters for debugging
        tracing::debug!(
            "IronRDP connect_finalize: host={}, nla={}, has_username={}, has_password={}",
            config.host,
            config.nla_enabled,
            config.username.is_some(),
            config.password.is_some()
        );

        // Complete connection (NLA, licensing, capabilities)
        // Wrap in catch_unwind: ironrdp-tokio 0.8.0 has a bug where
        // `copy_from_slice` panics on 64-bit systems in the KDC TCP
        // response parser (usize::to_be_bytes() returns 8 bytes but
        // destination is 4 bytes). Convert the panic into an error
        // so the GUI can fall back to FreeRDP instead of crashing.
        let finalize_future = AssertUnwindSafe(ironrdp_tokio::connect_finalize(
            upgraded,
            connector,
            &mut upgraded_framed,
            &mut network_client,
            ServerName::new(&config.host),
            server_public_key,
            None, // No Kerberos config
        ));

        let connection_result = match futures::FutureExt::catch_unwind(finalize_future).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => {
                tracing::error!(
                    "IronRDP connect_finalize failed: {:?}, error_kind={:?}",
                    e,
                    e.kind()
                );
                return Err(RdpClientError::ConnectionFailed(format!(
                    "Connection finalize failed: {e}"
                )));
            }
            Err(panic_payload) => {
                let msg = panic_payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic_payload.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic in IronRDP");
                tracing::error!(
                    protocol = "rdp",
                    panic = %msg,
                    "IronRDP connect_finalize panicked (upstream bug)"
                );
                return Err(RdpClientError::ConnectionFailed(format!(
                    "IronRDP internal error: {msg}"
                )));
            }
        };

        Ok::<_, RdpClientError>((upgraded_framed, connection_result))
    })
    .await;

    if let Ok(result) = handshake_result {
        result
    } else {
        tracing::error!(
            protocol = "rdp",
            host = %config.host,
            port = %config.port,
            timeout_secs = handshake_timeout.as_secs(),
            "RDP handshake timed out (TLS/NLA phase). Server may be overloaded."
        );
        Err(RdpClientError::Timeout)
    }
}

/// Builds `IronRDP` connector configuration from our config
fn build_connector_config(config: &RdpClientConfig) -> Config {
    // Always use UsernamePassword credentials
    // If username or password is missing, use empty strings
    // The server will prompt for credentials if needed
    let credentials = Credentials::UsernamePassword {
        username: config.username.clone().unwrap_or_default(),
        password: config
            .password
            .as_ref()
            .map(|s| s.expose_secret().to_string())
            .unwrap_or_default(),
    };

    // NOTE: BitmapConfig affects two things:
    // 1. ClientGccBlocks.core.supported_color_depths in BasicSettingsExchange
    // 2. BitmapCodecs capability in CapabilitiesExchange (ClientConfirmActive)
    //
    // Performance mode controls:
    // - Quality: lossy_compression=false (lossless), RemoteFX codec, all visual effects
    // - Balanced: lossy_compression=true (allows dynamic quality), RemoteFX codec
    // - Speed: lossy_compression=true, no RemoteFX (legacy bitmap), minimal effects
    //
    // IMPORTANT: color_depth MUST be 32 for AWS EC2 compatibility!
    // - color_depth=32 -> BPP32|BPP16 + WANT_32_BPP_SESSION (works)
    // - color_depth=24 -> BPP24 only, no WANT_32_BPP_SESSION (fails on AWS EC2)
    let bitmap_config = build_bitmap_config(config.performance_mode);

    // Build performance flags based on performance mode
    let performance_flags = build_performance_flags(config.performance_mode);

    Config {
        credentials,
        domain: config.domain.clone(),
        enable_tls: true,
        enable_credssp: config.nla_enabled,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: config
            .keyboard_layout
            .unwrap_or_else(super::super::keyboard_layout::detect_keyboard_layout),
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize {
            width: config.width,
            height: config.height,
        },
        desktop_scale_factor: config.scale_factor,
        bitmap: bitmap_config,
        client_build: 0,
        client_name: String::from("RustConn"),
        client_dir: String::new(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: config.audio_enabled,
        performance_flags,
        license_cache: None,
        timezone_info: get_timezone_info(),
        enable_server_pointer: true,
        // Use hardware pointer - server sends cursor bitmap separately
        // This avoids cursor artifacts in the framebuffer
        pointer_software_rendering: false,
    }
}

/// Builds performance flags based on the performance mode
fn build_performance_flags(mode: crate::models::RdpPerformanceMode) -> PerformanceFlags {
    use crate::models::RdpPerformanceMode;

    match mode {
        RdpPerformanceMode::Quality => {
            // Best quality: enable font smoothing and desktop composition
            PerformanceFlags::ENABLE_FONT_SMOOTHING | PerformanceFlags::ENABLE_DESKTOP_COMPOSITION
        }
        RdpPerformanceMode::Balanced => {
            // Balanced: default flags (disable full window drag and menu animations, enable font smoothing)
            PerformanceFlags::default()
        }
        RdpPerformanceMode::Speed => {
            // Best speed: disable all visual effects for maximum performance
            PerformanceFlags::DISABLE_WALLPAPER
                | PerformanceFlags::DISABLE_FULLWINDOWDRAG
                | PerformanceFlags::DISABLE_MENUANIMATIONS
                | PerformanceFlags::DISABLE_THEMING
                | PerformanceFlags::DISABLE_CURSOR_SHADOW
                | PerformanceFlags::DISABLE_CURSORSETTINGS
        }
    }
}

/// Builds bitmap configuration based on the performance mode
///
/// This controls:
/// - `lossy_compression`: Whether server can use lossy compression for better bandwidth
/// - `color_depth`: Always 32 for AWS EC2 compatibility
/// - `codecs`: RemoteFX for Quality/Balanced, empty (legacy) for Speed
fn build_bitmap_config(mode: crate::models::RdpPerformanceMode) -> Option<BitmapConfig> {
    use crate::models::RdpPerformanceMode;

    match mode {
        RdpPerformanceMode::Quality => {
            // Best quality: lossless compression, RemoteFX codec
            // drawing_flags = ALLOW_SKIP_ALPHA only (no color subsampling)
            Some(BitmapConfig {
                lossy_compression: false,
                color_depth: 32,
                codecs: client_codecs_capabilities(&[]).unwrap_or_else(|_| BitmapCodecs(vec![])),
            })
        }
        RdpPerformanceMode::Balanced => {
            // Balanced: lossy compression allowed, RemoteFX codec
            // drawing_flags = ALLOW_SKIP_ALPHA | ALLOW_DYNAMIC_COLOR_FIDELITY | ALLOW_SUBSAMPLING
            // Server can dynamically adjust quality based on bandwidth
            Some(BitmapConfig {
                lossy_compression: true,
                color_depth: 32,
                codecs: client_codecs_capabilities(&[]).unwrap_or_else(|_| BitmapCodecs(vec![])),
            })
        }
        RdpPerformanceMode::Speed => {
            // Best speed: lossy compression, no RemoteFX (legacy bitmap updates)
            // Uses basic RLE compression which is faster but lower quality
            // Good for slow/unreliable connections
            Some(BitmapConfig {
                lossy_compression: true,
                color_depth: 32,
                // Empty codecs = no RemoteFX, use legacy bitmap updates
                codecs: BitmapCodecs(vec![]),
            })
        }
    }
}

/// Gets the local timezone information
fn get_timezone_info() -> TimezoneInfo {
    let offset = chrono::Local::now().offset().local_minus_utc();
    // Bias is UTC - Local in minutes
    let bias = -(offset / 60);

    TimezoneInfo {
        bias,
        ..TimezoneInfo::default()
    }
}
