use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;

use ironrdp::cliprdr::CliprdrClient;
use ironrdp::connector::{
    BitmapConfig, ClientConnector, Config, ConnectionResult, Credentials, DesktopSize, ServerName,
};
use ironrdp::dvc::DrdynvcClient;
use ironrdp::echo::client::EchoClient;
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::{
    BitmapCodecs, MajorPlatformType, client_codecs_capabilities,
};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::rdpdr::Rdpdr;
use ironrdp::rdpsnd::client::Rdpsnd;
#[cfg(feature = "gfx-h264")]
use ironrdp_egfx::client::GraphicsPipelineClient;
use ironrdp_tokio::TokioFramed;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use secrecy::ExposeSecret;
use tokio::net::TcpStream;

use super::super::audio::RustConnAudioBackend;
use super::super::clipboard::RustConnClipboardBackend;
#[cfg(feature = "gfx-h264")]
use super::super::gfx_handler::{GfxFrameUpdate, RustConnGfxHandler, try_load_openh264};
use super::super::rdpdr::{RustConnRdpdrBackend, cups_default_printer, list_cups_printers};
use super::super::{RdpClientConfig, RdpClientError, RdpClientEvent};

/// Transport layer: either a direct TCP connection or a gateway tunnel.
enum GatewayOrTcp {
    Tcp(TcpStream),
    #[cfg(feature = "rd-gateway")]
    Gateway(ironrdp_mstsgu::GwClient),
}

/// Helper trait that combines `AsyncRead + AsyncWrite + Unpin + Send + Sync` for type-erased streams.
pub(super) trait AsyncReadWrite:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + Sync
{
}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + Sync> AsyncReadWrite for T {}

/// Type-erased async stream used after TLS upgrade.
/// Supports both direct TCP and gateway-tunneled connections.
pub(super) type RdpStream = Box<dyn AsyncReadWrite>;

pub(super) type UpgradedFramed = TokioFramed<RdpStream>;

/// Result of a successful RDP connection establishment.
///
/// Wraps the framed transport, connection metadata, and optional GFX channel
/// receiver (present only when `gfx-h264` feature is enabled).
pub(super) struct ConnectionSetup {
    /// Framed TLS stream for the active session
    pub framed: UpgradedFramed,
    /// IronRDP connection result with negotiated capabilities
    pub connection_result: ConnectionResult,
    /// Receiver for decoded GFX frame updates from the EGFX pipeline.
    /// The session loop drains this to blit RGBA→BGRA into the framebuffer.
    #[cfg(feature = "gfx-h264")]
    pub gfx_update_rx: std::sync::mpsc::Receiver<GfxFrameUpdate>,
}

/// Establishes the RDP connection and returns the framed stream and connection result.
///
/// When `gfx-h264` is enabled, also returns the GFX frame update receiver
/// for the session loop to drain decoded bitmap updates from the EGFX pipeline.
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
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub(super) async fn establish_connection(
    config: &RdpClientConfig,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
) -> Result<ConnectionSetup, RdpClientError> {
    use tokio::time::{Duration, timeout};

    let server_addr = config.server_address();
    let connect_timeout = Duration::from_secs(config.timeout_secs);

    // Phase 1: Establish TCP connection (or gateway tunnel)
    #[cfg(feature = "rd-gateway")]
    let (stream, client_addr) = if config.uses_gateway() {
        // Connect through RD Gateway (MS-TSGU) via ironrdp-mstsgu
        let gw = &config.gateway;
        let gw_endpoint = if gw.port == 443 {
            gw.hostname.clone()
        } else {
            format!("{}:{}", gw.hostname, gw.port)
        };
        let gw_user = gw.username.clone().unwrap_or_default();
        // Gateway password: reuse session password when no explicit gateway password is set.
        // This matches FreeRDP's behaviour (credentials are shared by default).
        // NOTE: ironrdp-mstsgu takes an owned String — we zeroize the intermediate.
        // ponytail: the final String passed to ironrdp-mstsgu is not zeroized on drop;
        // upgrade when ironrdp-mstsgu accepts SecretString or &str.
        let gw_pass = {
            use zeroize::Zeroizing;
            let tmp = config
                .password
                .as_ref()
                .map(|s| Zeroizing::new(s.expose_secret().to_string()))
                .unwrap_or_default();
            (*tmp).clone()
        };
        let gw_target = ironrdp_mstsgu::GwConnectTarget {
            gw_endpoint,
            gw_user,
            gw_pass,
            server: config.host.clone(),
        };
        let client_name = hostname::get().map_or_else(
            |_| "RustConn".to_string(),
            |h| h.to_string_lossy().into_owned(),
        );

        tracing::info!(
            protocol = "rdp",
            gateway = %config.gateway.hostname,
            target = %config.host,
            "Connecting through RD Gateway (MS-TSGU)"
        );

        let gw_result = timeout(
            connect_timeout,
            ironrdp_mstsgu::GwClient::connect(&gw_target, &client_name),
        )
        .await;

        match gw_result {
            Ok(Ok((gw_client, addr))) => (GatewayOrTcp::Gateway(gw_client), addr),
            Ok(Err(e)) => {
                return Err(RdpClientError::ConnectionFailed(format!(
                    "RD Gateway connection failed: {e}"
                )));
            }
            Err(_) => {
                return Err(RdpClientError::Timeout);
            }
        }
    } else {
        let tcp_result = timeout(connect_timeout, TcpStream::connect(&server_addr)).await;
        let stream = match tcp_result {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                stream
            }
            Ok(Err(e)) => {
                return Err(RdpClientError::ConnectionFailed(format!(
                    "Failed to connect to {server_addr}: {e}"
                )));
            }
            Err(_) => {
                return Err(RdpClientError::Timeout);
            }
        };
        let addr = stream
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        (GatewayOrTcp::Tcp(stream), addr)
    };

    #[cfg(not(feature = "rd-gateway"))]
    let (stream, client_addr) = {
        let tcp_result = if config.mptcp {
            // MPTCP path: resolve hostname then use MPTCP socket
            let resolved = timeout(connect_timeout, async {
                let addr = tokio::net::lookup_host(&server_addr)
                    .await
                    .map_err(|e| {
                        std::io::Error::other(format!("Failed to resolve {server_addr}: {e}"))
                    })?
                    .next()
                    .ok_or_else(|| {
                        std::io::Error::other(format!("No addresses found for {server_addr}"))
                    })?;
                crate::connection::mptcp::connect_mptcp_async(addr)
                    .await
                    .map_err(|e| std::io::Error::other(e.to_string()))
            })
            .await;
            match resolved {
                Ok(inner) => Ok(inner),
                Err(_) => Err(std::io::Error::other("timeout")),
            }
        } else {
            let r = timeout(connect_timeout, TcpStream::connect(&server_addr)).await;
            match r {
                Ok(inner) => Ok(inner),
                Err(_) => Err(std::io::Error::other("timeout")),
            }
        };
        let stream = match tcp_result {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                stream
            }
            Ok(Err(e)) => {
                return Err(RdpClientError::ConnectionFailed(format!(
                    "Failed to connect to {server_addr}: {e}"
                )));
            }
            Err(e) if e.to_string() == "timeout" => {
                return Err(RdpClientError::Timeout);
            }
            Err(e) => {
                return Err(RdpClientError::ConnectionFailed(format!(
                    "Failed to connect to {server_addr}: {e}"
                )));
            }
        };
        let addr = stream
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        (GatewayOrTcp::Tcp(stream), addr)
    };

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

    // Phase 2.6: Add RDPDR channel for shared folders and/or printer if configured.
    // Note: RDPDR requires RDPSND channel to be present per MS-RDPEFS spec.
    let needs_rdpdr = !config.shared_folders.is_empty() || config.printer_enabled;
    if needs_rdpdr {
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

        // Per-drive path mapping for the backend (empty when printer-only).
        let drive_paths: std::collections::HashMap<u32, String> = config
            .shared_folders
            .iter()
            .enumerate()
            .map(|(idx, folder)| {
                let device_id = idx as u32 + 1;
                (device_id, folder.path.to_string_lossy().into_owned())
            })
            .collect();

        // Build the printer device_id -> CUPS queue map. Printer IDs sit past
        // the drive IDs (drives occupy 1..=N) so they never collide. Each
        // forwarded queue is announced as its own redirected printer, and the
        // backend uses this map to route each job back to the right local queue.
        // BTreeMap keeps insertion order by key, so no extra sort is needed for
        // the announce loop (default printer has the highest ID → announced last).
        let mut printer_queues: std::collections::BTreeMap<u32, String> =
            std::collections::BTreeMap::new();
        if config.printer_enabled {
            let base = config.shared_folders.len() as u32;

            // Decide which queues to forward: an explicit subset, or all local
            // queues when none are listed.
            let mut queues = if config.printers.is_empty() {
                list_cups_printers()
            } else {
                config.printers.clone()
            };

            // Announce the CUPS default LAST so it wins IronRDP's hardcoded
            // DEFAULTPRINTER flag race (see plan Appendix A).
            if let Some(default) = cups_default_printer()
                && let Some(pos) = queues.iter().position(|q| *q == default)
            {
                let d = queues.remove(pos);
                queues.push(d);
            }

            for (idx, queue) in queues.into_iter().enumerate() {
                let device_id = base + 1 + idx as u32;
                printer_queues.insert(device_id, queue);
            }
        }

        let rdpdr_backend =
            RustConnRdpdrBackend::new(drive_paths, printer_queues.clone().into_iter().collect());
        let mut rdpdr = Rdpdr::new(Box::new(rdpdr_backend), computer_name);
        if !initial_drives.is_empty() {
            rdpdr = rdpdr.with_drives(Some(initial_drives));
        }

        for (device_id, queue) in &printer_queues {
            tracing::debug!("RDPDR: registering printer {device_id} -> '{queue}'");
            rdpdr = rdpdr.with_printer(*device_id, queue.clone());
        }

        connector.static_channels.insert(rdpdr);
    } else if config.audio_enabled {
        // No shared folders but audio is enabled - add RDPSND channel
        let audio_backend = RustConnAudioBackend::new(event_tx.clone());
        let rdpsnd = Rdpsnd::new(Box::new(audio_backend));
        connector.static_channels.insert(rdpsnd);
        tracing::debug!("Audio channel enabled (without RDPDR)");
    }

    // Register DRDYNVC (Dynamic Virtual Channel) with Echo client for RTT measurement.
    // DisplayControlClient is also registered here for dynamic resolution changes (MS-RDPEDISP).
    // The Echo channel (MS-RDPEECO) responds to server echo requests, enabling the server
    // to measure round-trip time and report it back via Auto-Detect PDU.
    // ironrdp 0.16: DisplayControlClient::new takes a capabilities callback. We send the
    // monitor layout on demand via ActiveStage::encode_resize, so nothing is emitted when
    // capabilities arrive — return an empty message set.
    let dc_ready_tx = event_tx.clone();
    #[cfg_attr(
        not(feature = "gfx-h264"),
        expect(unused_mut, reason = "mut needed when gfx-h264 adds a channel")
    )]
    let mut drdynvc = DrdynvcClient::new()
        .with_dynamic_channel(ironrdp::displaycontrol::client::DisplayControlClient::new(
            move |_caps| {
                // Capabilities have arrived → the Display Control channel is
                // ready for MS-RDPEDISP resize. Signal the GUI so the initial
                // "snap to settled size" goes over Display Control instead of a
                // premature reconnect. No monitor layout is emitted here — it is
                // sent on demand via ActiveStage::encode_resize.
                let _ = dc_ready_tx.send(RdpClientEvent::DisplayControlReady);
                Ok(Vec::new())
            },
        ))
        .with_dynamic_channel(EchoClient::new());

    // Register EGFX Graphics Pipeline for H.264/AVC decoding when available.
    //
    // Respects `config.graphics_mode`: Legacy and RemoteFx modes skip the EGFX
    // channel entirely, forcing the server to use bitmap/RemoteFX updates
    // through the static channel path. This enables a retry-without-GFX
    // strategy when the GFX pipeline fails (issue #218).
    //
    // Fallback behavior (Req 6):
    // - When `try_load_openh264()` returns None (library missing), the
    //   `GraphicsPipelineClient` is created with `h264_decoder: None`. This causes
    //   ironrdp-egfx to NOT advertise AVC codecs in capability exchange, so the
    //   server falls back to uncompressed/RFX-progressive within the GFX channel.
    // - When the `gfx-h264` feature is disabled at compile time, this entire block
    //   is absent — no EGFX DVC is registered, and the session uses the existing
    //   RemoteFX/Legacy rendering path identically to before (Req 6 AC 2, AC 5).
    // - When `graphics_mode` is Legacy or RemoteFx, the EGFX DVC is not
    //   registered even if the feature is enabled — the session uses the
    //   RemoteFX/Legacy path without the GFX pipeline.
    #[cfg(feature = "gfx-h264")]
    let gfx_update_rx = {
        use crate::rdp_client::graphics::GraphicsMode;
        let skip_gfx = matches!(
            config.graphics_mode,
            GraphicsMode::Legacy | GraphicsMode::RemoteFx
        );

        let (gfx_update_tx, gfx_update_rx) = std::sync::mpsc::channel::<GfxFrameUpdate>();
        if skip_gfx {
            // Drop the sender — receiver's try_recv() will return Disconnected
            // immediately, which is fine (the session loop uses `while let Ok`).
            drop(gfx_update_tx);
            tracing::info!(
                graphics_mode = ?config.graphics_mode,
                "EGFX pipeline skipped (graphics_mode forces Legacy/RemoteFX path)"
            );
        } else {
            let h264_decoder = try_load_openh264();
            let h264_available = h264_decoder.is_some();
            let handler = RustConnGfxHandler::new(gfx_update_tx, event_tx.clone());
            let gfx_client = GraphicsPipelineClient::new(Box::new(handler), h264_decoder);
            drdynvc = drdynvc.with_dynamic_channel(gfx_client);
            tracing::info!(h264_available, "EGFX pipeline registered");
        }
        gfx_update_rx
    };

    connector.static_channels.insert(drdynvc);
    tracing::debug!("DRDYNVC registered with DisplayControl + Echo channels");

    // Phase 3: Perform RDP connection sequence (TLS + NLA + capabilities)
    // Wrap the entire handshake in a timeout — on heavily loaded servers the
    // TCP connect succeeds quickly but TLS/NLA can hang indefinitely.
    // For tunnel connections (127.0.0.1), use a shorter timeout since the
    // TCP connect is instant and any delay means the remote host is unreachable.
    let is_tunnel = config.host == "127.0.0.1" || config.host == "localhost";
    let handshake_secs = if is_tunnel {
        config.timeout_secs.min(15)
    } else {
        config.timeout_secs.saturating_mul(2).max(60)
    };
    let handshake_timeout = Duration::from_secs(handshake_secs);

    let handshake_result = timeout(handshake_timeout, async {
        // Type-erase the transport: both TcpStream and GwClient implement
        // AsyncRead + AsyncWrite + Unpin + Send.
        let raw_stream: Box<dyn AsyncReadWrite> = match stream {
            GatewayOrTcp::Tcp(tcp) => Box::new(tcp),
            #[cfg(feature = "rd-gateway")]
            GatewayOrTcp::Gateway(gw) => Box::new(gw),
        };
        let mut framed = TokioFramed::new(raw_stream);

        // Begin connection (X.224 negotiation)
        tracing::debug!(
            protocol = "rdp",
            host = %config.host,
            port = %config.port,
            "Starting X.224 connection negotiation"
        );
        let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
            .await
            .map_err(|e| {
                RdpClientError::ConnectionFailed(format!("Connection begin failed: {e}"))
            })?;

        tracing::debug!(
            protocol = "rdp",
            "X.224 negotiation complete, starting TLS upgrade"
        );

        // TLS upgrade - returns stream and server certificate.
        // Note: IronRDP does not validate the server certificate against a CA
        // store. This is equivalent to xfreerdp /cert:ignore and is standard
        // for RDP where most servers use self-signed certificates.
        let initial_stream = framed.into_inner_no_leftover();

        let (upgraded_stream, server_cert) = ironrdp_tls::upgrade(initial_stream, &config.host)
            .await
            .map_err(|e| RdpClientError::ConnectionFailed(format!("TLS upgrade failed: {e}")))?;

        tracing::debug!(
            protocol = "rdp",
            "TLS upgrade complete, proceeding to NLA/capabilities"
        );

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

        let mut upgraded_framed = TokioFramed::new(Box::new(upgraded_stream) as RdpStream);

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
        // Wrap in catch_unwind: IronRDP may panic on unexpected server
        // responses in edge cases. Convert the panic into an error
        // so the GUI can fall back to FreeRDP instead of crashing.
        // Checked for 0.19.0: ironrdp 0.17 is now in use. The upstream
        // connect_finalize panic reports remain open (e.g.
        // https://github.com/Devolutions/IronRDP/issues/1016), so the
        // wrapper stays. Re-evaluate on the next ironrdp bump (>0.17).
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
        let (framed, connection_result) = result?;
        Ok(ConnectionSetup {
            framed,
            connection_result,
            #[cfg(feature = "gfx-h264")]
            gfx_update_rx,
        })
    } else {
        tracing::error!(
            protocol = "rdp",
            host = %config.host,
            port = %config.port,
            timeout_secs = handshake_secs,
            is_tunnel,
            "RDP handshake timed out (TLS/NLA phase). \
             The remote host may be unreachable or RDP service not running."
        );
        if is_tunnel {
            Err(RdpClientError::ConnectionFailed(format!(
                "RDP handshake timed out after {handshake_secs}s — \
                 the remote host may be unreachable through the SSH tunnel \
                 or the RDP service is not running"
            )))
        } else {
            Err(RdpClientError::Timeout)
        }
    }
}

/// Builds `IronRDP` connector configuration from our config
fn build_connector_config(config: &RdpClientConfig) -> Config {
    // Always use UsernamePassword credentials
    // If username or password is missing, use empty strings
    // The server will prompt for credentials if needed
    //
    // The ironrdp connector API requires an owned plain `String` password by
    // value; the copy's lifetime (and zeroization) is controlled by ironrdp,
    // so wrapping the intermediate in `Zeroizing` protects only the temporary
    // that bridges expose_secret() → Credentials construction. Re-check on
    // ironrdp bumps whether a secrecy-aware credentials type became available.
    let credentials = {
        use zeroize::Zeroizing;
        let pw = config
            .password
            .as_ref()
            .map(|s| Zeroizing::new(s.expose_secret().to_string()))
            .unwrap_or_default();
        Credentials::UsernamePassword {
            username: config.username.clone().unwrap_or_default(),
            // Clone into ironrdp's owned String; `pw` is zeroized on drop.
            password: (*pw).clone(),
        }
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
        // Alternate shell for RemoteApp / CyberArk PSM scenarios
        alternate_shell: config
            .remote_app
            .as_ref()
            .map(|app| app.program.clone())
            .unwrap_or_default(),
        work_dir: config
            .remote_app
            .as_ref()
            .and_then(|app| app.working_dir.clone())
            .unwrap_or_default(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: config.audio_enabled,
        performance_flags,
        license_cache: None,
        timezone_info: get_timezone_info(),
        // Bulk (MPPC/NCRUSH/XCRUSH) FastPath compression is intentionally
        // disabled (matches the upstream ironrdp-client default of `None`).
        //
        // When enabled, the server sends compressed FastPath updates that keep
        // their compression history across a Deactivation-Reactivation Sequence
        // (window resize). ironrdp-session recreates the FastPath processor —
        // and thus its bulk decompressor — from scratch on reactivation, with
        // no API to preserve the negotiated history. A fresh decompressor then
        // desynchronises from the server and the session aborts with a
        // "no decompressor is configured" / bulk-decompression error (issue #200).
        //
        // Graphics are already compressed by RemoteFX (Quality/Balanced) or
        // RLE/RDP6 bitmap compression (Speed), so the extra bulk layer buys
        // little bandwidth while breaking every resize. Keep it off until
        // ironrdp exposes a way to carry the decompressor across reactivation.
        compression_type: None,
        enable_server_pointer: true,
        // Use hardware pointer - server sends cursor bitmap separately
        // This avoids cursor artifacts in the framebuffer
        pointer_software_rendering: false,
        // Multitransport (UDP sideband) — not implemented yet
        multitransport_flags: None,
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
