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
    BitmapCodecs, CaptureFlags, Codec, CodecProperty, EntropyBits, MajorPlatformType,
    RemoteFxContainer, RfxCaps, RfxCapset, RfxClientCapsContainer, RfxICap, RfxICapFlags,
};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::rdpdr::Rdpdr;
use ironrdp::rdpsnd::client::Rdpsnd;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::TokioFramed;
use std::net::SocketAddr;
use tokio::net::TcpStream;

pub type UpgradedFramed = TokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

/// Establishes the RDP connection and returns the framed stream and connection result.
// The future is not Send because IronRDP's AsyncNetworkClient is not Send.
// This is fine because we run on a single-threaded Tokio runtime.
#[allow(clippy::future_not_send)]
pub async fn establish_connection(
    config: &RdpClientConfig,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
) -> Result<(UpgradedFramed, ConnectionResult), RdpClientError> {
    use tokio::time::{timeout, Duration};

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
                // #[allow(clippy::cast_possible_truncation)]
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

    // Phase 3: Perform RDP connection sequence
    let mut framed = TokioFramed::new(stream);

    // Begin connection (X.224 negotiation)
    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| RdpClientError::ConnectionFailed(format!("Connection begin failed: {e}")))?;

    // TLS upgrade - returns stream and server certificate
    let initial_stream = framed.into_inner_no_leftover();

    let (upgraded_stream, server_cert) =
        ironrdp_tls::upgrade(initial_stream, &config.host)
            .await
            .map_err(|e| RdpClientError::ConnectionFailed(format!("TLS upgrade failed: {e}")))?;

    // Extract server public key from certificate
    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&server_cert)
        .map(|k| k.to_vec())
        .unwrap_or_default();

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

    let mut upgraded_framed = TokioFramed::new(upgraded_stream);

    // Create network client for Kerberos/AAD authentication
    let mut network_client = ReqwestNetworkClient::new();

    // Complete connection (NLA, licensing, capabilities)
    // New API in ironrdp 0.14: connect_finalize(upgraded, connector, framed, network_client, server_name, server_public_key, kerberos_config)
    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut network_client,
        ServerName::new(&config.host),
        server_public_key,
        None, // No Kerberos config
    )
    .await
    .map_err(|e| RdpClientError::ConnectionFailed(format!("Connection finalize failed: {e}")))?;

    Ok((upgraded_framed, connection_result))
}

/// Builds `IronRDP` connector configuration from our config
fn build_connector_config(config: &RdpClientConfig) -> Config {
    // Always use UsernamePassword credentials
    // If username or password is missing, use empty strings
    // The server will prompt for credentials if needed
    let credentials = Credentials::UsernamePassword {
        username: config.username.clone().unwrap_or_default(),
        password: config.password.clone().unwrap_or_default(),
    };

    // Configure RemoteFX codec for better image quality
    // Use color depth from config (derived from performance mode)
    let bitmap_config = Some(BitmapConfig {
        lossy_compression: true,
        color_depth: u32::from(config.color_depth),
        codecs: build_bitmap_codecs(),
    });

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
        keyboard_layout: 0x0409, // US English
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize {
            width: config.width,
            height: config.height,
        },
        desktop_scale_factor: 0,
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

/// Builds bitmap codecs configuration with `RemoteFX` support
fn build_bitmap_codecs() -> BitmapCodecs {
    // RemoteFX codec for high-quality graphics
    let remotefx_codec = Codec {
        id: 3, // CODEC_ID_REMOTEFX
        property: CodecProperty::RemoteFx(RemoteFxContainer::ClientContainer(
            RfxClientCapsContainer {
                capture_flags: CaptureFlags::empty(),
                caps_data: RfxCaps(RfxCapset(vec![RfxICap {
                    flags: RfxICapFlags::empty(),
                    entropy_bits: EntropyBits::Rlgr3,
                }])),
            },
        )),
    };

    // ImageRemoteFx codec (required for GFX)
    let image_remotefx_codec = Codec {
        id: 0x4, // CODEC_ID_IMAGE_REMOTEFX
        property: CodecProperty::ImageRemoteFx(RemoteFxContainer::ClientContainer(
            RfxClientCapsContainer {
                capture_flags: CaptureFlags::empty(),
                caps_data: RfxCaps(RfxCapset(vec![RfxICap {
                    flags: RfxICapFlags::empty(),
                    entropy_bits: EntropyBits::Rlgr3,
                }])),
            },
        )),
    };

    BitmapCodecs(vec![remotefx_codec, image_remotefx_codec])
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
