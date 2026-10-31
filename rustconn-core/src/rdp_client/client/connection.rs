use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_connector::{
    BitmapConfig, ClientConnector, Config, ConnectionOutcome, ConnectionResult, Credentials,
    DesktopSize, ServerName,
};
use ironrdp_dvc::DrdynvcClient;
use ironrdp_echo::client::EchoClient;
#[cfg(feature = "gfx-h264")]
use ironrdp_egfx::client::GraphicsPipelineClient;
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::rdp::capability_sets::{
    BitmapCodecs, MajorPlatformType, client_codecs_capabilities,
};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_rdpdr::Rdpdr;
use ironrdp_rdpsnd::client::Rdpsnd;
use ironrdp_tokio::TokioFramed;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use secrecy::ExposeSecret;
use tokio::net::TcpStream;

use super::super::audio::RustConnAudioBackend;
use super::super::clipboard::RustConnClipboardBackend;
#[cfg(feature = "gfx-h264")]
use super::super::gfx_handler::{RustConnGfxHandler, try_load_openh264};
use super::super::kerberos::{KerberosSettings, kerberos_settings_for};
use super::super::rdpdr::{RustConnRdpdrBackend, cups_default_printer, list_cups_printers};
use super::super::{RdpClientConfig, RdpClientError, RdpClientEvent};

/// Turns an `ironrdp` connector failure into a typed [`RdpClientError`].
///
/// `ConnectorError`'s `Display` renders only the kind label plus the upstream
/// message; the machine-readable detail the GUI needs — the CredSSP `NTSTATUS`
/// in particular — lives in the kind's `Debug`. Both are kept so
/// [`crate::rdp_client::failure::classify_rdp_failure`] can tell a rejected
/// credential apart from a protocol incompatibility. Without the `NTSTATUS`,
/// a wrong password looked like a broken server and triggered a pointless
/// external `FreeRDP` fallback with the very same credentials.
fn map_connector_error(context: &str, display: &str, kind_debug: &str) -> RdpClientError {
    let detail = format!("{context}: {display} [kind: {kind_debug}]");
    if crate::rdp_client::failure::is_authentication_failure(&detail) {
        RdpClientError::AuthenticationFailed(detail)
    } else {
        RdpClientError::ConnectionFailed(detail)
    }
}

/// Whether sspi's own lookup finds a KDC for `realm`.
///
/// That lookup is `SSPI_KDC_URL_<REALM>`, then `SSPI_KDC_URL`, then the
/// `[realms] <REALM> kdc` entry of `krb5.conf` (`KRB5_CONFIG`, or
/// `/etc/krb5.conf` and `/usr/local/etc/krb5.conf`). Its DNS SRV step is compiled
/// out on Linux, where IronRDP builds sspi without `dns_resolver`. The realm is
/// first mapped through `krb5.conf` `[domain_realm]` the way sspi maps the client
/// principal's realm, so the question is about the realm the AS-REQ will carry.
///
/// `detect_kdc_url` is the call sspi itself makes when no KDC is pinned, so an
/// entry it cannot parse counts as not found here too. `sspi` is IronRDP's own
/// re-export, so this always asks the sspi version that does the sign-in.
fn sspi_finds_kdc(realm: &str) -> bool {
    use ironrdp_connector::sspi;

    let realm = sspi::kerberos::client::principal::get_client_principal_realm("", realm);
    sspi::detect_kdc_url(&realm).is_some()
}

/// Builds IronRDP's `KerberosConfig` from what [`kerberos_settings_for`] chose.
///
/// `KerberosConfig::new` fails only when the KDC URL does not parse. The address
/// was normalized on the way here, so that should not happen; if it does,
/// Kerberos goes ahead without the address rather than refusing to connect.
fn kerberos_connector_config(
    config: &RdpClientConfig,
    settings: KerberosSettings,
) -> Option<ironrdp_connector::credssp::KerberosConfig> {
    use ironrdp_connector::credssp::KerberosConfig;

    let KerberosSettings {
        kdc_url,
        client_hostname,
    } = settings;
    tracing::info!(
        protocol = "rdp",
        host = %config.host,
        kdc = ?kdc_url,
        "Kerberos NLA enabled"
    );
    match KerberosConfig::new(kdc_url, client_hostname.clone()) {
        Ok(kerberos) => Some(kerberos),
        Err(error) => {
            tracing::warn!(
                protocol = "rdp",
                host = %config.host,
                %error,
                "IronRDP rejected the KDC address; continuing without it"
            );
            KerberosConfig::new(None, client_hostname).ok()
        }
    }
}

/// Applies the trust-on-first-use certificate check to the presented certificate.
///
/// IronRDP performs no CA validation, so this is the only place the server
/// certificate is checked. When `config.ignore_certificate` is set the check is
/// skipped entirely (equivalent to xfreerdp `/cert:ignore`). Otherwise the
/// certificate's SHA-256 fingerprint is compared against RustConn's own TOFU
/// store: a first sighting is recorded and accepted, a matching fingerprint is
/// accepted, and a changed fingerprint aborts the connection with
/// [`RdpClientError::CertificateChanged`] so the GUI can ask the user.
///
/// A store I/O failure is logged and treated as acceptance rather than a hard
/// stop: the previous behaviour accepted every certificate unconditionally, so
/// failing open here is no weaker than that while the common path still gets the
/// TOFU guarantee.
fn verify_server_certificate(
    config: &RdpClientConfig,
    cert: &x509_cert::Certificate,
) -> Result<(), RdpClientError> {
    use x509_cert::der::Encode as _;

    if config.ignore_certificate {
        tracing::warn!(
            protocol = "rdp",
            host = %config.host,
            port = config.port,
            "Server certificate not checked (ignore_certificate is set)"
        );
        return Ok(());
    }

    let der = match cert.to_der() {
        Ok(der) => der,
        Err(e) => {
            tracing::warn!(
                protocol = "rdp",
                host = %config.host,
                port = config.port,
                error = %e,
                "Could not re-encode the server certificate for the TOFU check — accepting"
            );
            return Ok(());
        }
    };

    let fingerprint = crate::rdp_client::tofu::fingerprint_certificate(&der);

    match crate::rdp_client::tofu::verify_or_store(&config.host, config.port, &fingerprint) {
        Ok(crate::rdp_client::tofu::TofuVerdict::FirstUse) => {
            tracing::info!(
                protocol = "rdp",
                host = %config.host,
                port = config.port,
                %fingerprint,
                "First connection to this host — server certificate fingerprint recorded"
            );
            Ok(())
        }
        Ok(crate::rdp_client::tofu::TofuVerdict::Match) => {
            tracing::debug!(
                protocol = "rdp",
                host = %config.host,
                port = config.port,
                "Server certificate fingerprint matches the stored value"
            );
            Ok(())
        }
        Ok(crate::rdp_client::tofu::TofuVerdict::Changed { stored }) => {
            tracing::warn!(
                protocol = "rdp",
                host = %config.host,
                port = config.port,
                new_fingerprint = %fingerprint,
                old_fingerprint = %stored,
                "Server certificate has changed since the last connection"
            );
            Err(RdpClientError::CertificateChanged {
                host: config.host.clone(),
                port: config.port,
                new_fingerprint: fingerprint,
                old_fingerprint: stored,
            })
        }
        Err(e) => {
            tracing::warn!(
                protocol = "rdp",
                host = %config.host,
                port = config.port,
                error = %e,
                "TOFU certificate store unavailable — accepting the certificate"
            );
            Ok(())
        }
    }
}

/// Transport layer: either a direct TCP connection or a gateway tunnel.
enum GatewayOrTcp {
    Tcp(TcpStream),
    #[cfg(feature = "rd-gateway")]
    Gateway(ironrdp_mstsgu::GwClient),
}

/// Owns an IronRDP gateway target and erases its password on every exit path.
#[cfg(feature = "rd-gateway")]
struct ZeroizingGatewayTarget(ironrdp_mstsgu::GwConnectTarget);

#[cfg(feature = "rd-gateway")]
impl std::ops::Deref for ZeroizingGatewayTarget {
    type Target = ironrdp_mstsgu::GwConnectTarget;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "rd-gateway")]
impl Drop for ZeroizingGatewayTarget {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.gw_pass.zeroize();
    }
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
    /// The microphone channel's link to the session loop, when the connection
    /// asked for the microphone.
    pub microphone: Option<MicrophoneLink>,
}

/// What the session loop needs to carry microphone audio (MS-RDPEAI).
///
/// The GUI's packets go into `sink`; the `AUDIO_INPUT` channel answers with
/// DRDYNVC messages outside the server's turn, which arrive on `uplink_rx`
/// for the session loop to send.
pub(super) struct MicrophoneLink {
    pub sink: crate::rdp_client::microphone::MicrophoneSink,
    pub uplink_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<ironrdp_svc::SvcMessage>>,
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
pub(super) async fn establish_connection(
    config: &RdpClientConfig,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
) -> Result<ConnectionSetup, RdpClientError> {
    // An RD Connection Broker answers the first connection with a Server
    // Redirection naming the session host; the client then connects there.
    // A broker may chain once more, so two hops are followed.
    const MAX_REDIRECTIONS: usize = 2;

    let mut target = config.clone();
    let mut redirect: Option<Redirect> = None;
    for _ in 0..=MAX_REDIRECTIONS {
        match connect_once(&target, redirect.as_ref(), event_tx.clone()).await? {
            Attempt::Connected(setup) => return Ok(*setup),
            Attempt::Redirected(pdu) => {
                let (next, next_redirect) = follow_redirection(&target, &pdu);
                tracing::info!(
                    protocol = "rdp",
                    from = %target.host,
                    to = %next.host,
                    routing_token = next_redirect.routing_token.is_some(),
                    "Following the connection broker's redirection"
                );
                target = next;
                redirect = Some(next_redirect);
            }
        }
    }
    Err(RdpClientError::ProtocolError(
        "Server redirection to another session host: too many redirections".to_string(),
    ))
}

/// How a broker's Server Redirection asks the next connection to identify
/// itself (MS-RDPBCGR 2.2.13.1).
struct Redirect {
    /// Load-balancing data to send as the X.224 routing token
    routing_token: Option<String>,
    /// The session the broker placed the user in
    session_id: u32,
    /// Opaque password cookie to send in the Client Info PDU; a credential,
    /// so it is zeroized on drop and never printed
    password_cookie: Option<zeroize::Zeroizing<Vec<u8>>>,
}

impl std::fmt::Debug for Redirect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redirect")
            .field("routing_token", &self.routing_token)
            .field("session_id", &self.session_id)
            .field(
                "password_cookie",
                &self.password_cookie.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// The outcome of one connection attempt.
enum Attempt {
    Connected(Box<ConnectionSetup>),
    Redirected(Box<ironrdp_pdu::rdp::server_redirection::ServerRedirectionPdu>),
}

/// Builds the next connection from a Server Redirection PDU.
///
/// The target is the session host's FQDN when the broker gives one, which is
/// what TLS and Kerberos expect, else its address, else the same host — a
/// redirection that only carries a routing token reconnects to the broker.
/// The port, gateway and credentials stay; a user name or domain in the PDU
/// replaces the configured one.
fn follow_redirection(
    config: &RdpClientConfig,
    pdu: &ironrdp_pdu::rdp::server_redirection::ServerRedirectionPdu,
) -> (RdpClientConfig, Redirect) {
    fn clean(value: Option<&String>) -> Option<String> {
        value
            .map(|value| value.trim_end_matches('\0').trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    let mut next = config.clone();
    if let Some(host) = clean(pdu.target_fqdn.as_ref())
        .or_else(|| clean(pdu.target_net_address.as_ref()))
        .or_else(|| {
            pdu.target_net_addresses
                .as_ref()
                .and_then(|addresses| clean(addresses.first()))
        })
    {
        next.host = host;
    }
    if let Some(username) = clean(pdu.username.as_ref()) {
        next.username = Some(username);
    }
    if let Some(domain) = clean(pdu.domain.as_ref()) {
        next.domain = Some(domain);
    }
    // A broker's own routing value has served its purpose.
    next.load_balance_info = None;

    let redirect = Redirect {
        routing_token: pdu
            .load_balance_info
            .as_deref()
            .and_then(routing_token_from_bytes),
        session_id: pdu.session_id,
        password_cookie: pdu.password.clone().map(zeroize::Zeroizing::new),
    };
    (next, redirect)
}

/// Turns the load-balancing bytes of a Server Redirection into the printable
/// routing token the X.224 Connection Request carries, without the CRLF the
/// encoder appends itself. `None` for bytes that cannot be sent as one.
fn routing_token_from_bytes(bytes: &[u8]) -> Option<String> {
    let token = std::str::from_utf8(bytes).ok()?.trim_end_matches("\r\n");
    (!token.is_empty() && token.bytes().all(|byte| (0x20..=0x7E).contains(&byte)))
        .then(|| token.to_owned())
}

/// One connection attempt: TCP or gateway, TLS, NLA and capabilities.
/// Opens the direct TCP connection to the RDP host.
async fn connect_direct(
    config: &RdpClientConfig,
    connect_timeout: tokio::time::Duration,
) -> Result<(GatewayOrTcp, SocketAddr), RdpClientError> {
    let dial = crate::connection::connect_tcp(&config.host, config.port, config.mptcp);
    let stream = match tokio::time::timeout(connect_timeout, dial).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(RdpClientError::ConnectionFailed(format!(
                "Failed to connect to {}: {e}",
                config.server_address()
            )));
        }
        Err(_) => return Err(RdpClientError::Timeout),
    };
    let _ = stream.set_nodelay(true);
    // Without this a suspend leaves the socket half-open and the session loop
    // parks in read() forever — a frozen picture with no error (issue #248).
    crate::connection::set_interactive_keepalive(&stream, "rdp");
    let addr = stream
        .local_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    Ok((GatewayOrTcp::Tcp(stream), addr))
}

#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
async fn connect_once(
    config: &RdpClientConfig,
    redirect: Option<&Redirect>,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
) -> Result<Attempt, RdpClientError> {
    use tokio::time::{Duration, timeout};

    let connect_timeout = Duration::from_secs(config.timeout_secs);

    // Phase 1: Establish TCP connection (or gateway tunnel)
    #[cfg(feature = "rd-gateway")]
    let (stream, client_addr) = if config.uses_gateway() {
        // Connect through RD Gateway (MS-TSGU) via ironrdp-mstsgu
        let gw = &config.gateway;
        // `ironrdp-mstsgu` hands this endpoint straight to `TcpStream::connect`,
        // so the port must always be present — a bare hostname is rejected as
        // an invalid socket address.
        let gw_endpoint = format!("{}:{}", gw.hostname, gw.port);
        // `ironrdp-mstsgu` 0.0.1 hard-codes the tunnel target port to 3389
        // (`HTTP_CHANNEL_PACKET`), so a non-standard target port cannot be
        // reached through the embedded tunnel. GUI callers route those
        // connections to the external FreeRDP client instead; warn in case a
        // caller ever skips that check.
        if config.port != 3389 {
            tracing::warn!(
                protocol = "rdp",
                port = config.port,
                "RD Gateway tunnel always targets port 3389; configured port is ignored"
            );
        }
        // Gateway account: a dedicated gateway user when configured, otherwise
        // the session user, domain-qualified for AD-backed gateways. An empty
        // user is rejected by the gateway's HTTP Basic challenge.
        let gw_user = crate::rdp_client::gateway::resolve_gateway_user(
            gw.username.as_deref(),
            config.username.as_deref(),
            config.domain.as_deref(),
        );
        // Gateway password: reuse the session password when no explicit
        // gateway password is set. `ironrdp-mstsgu` requires an owned String, and
        // this is the only allocation holding it: the `String` is *moved* into
        // `gw_pass`, and moving a `String` transfers the heap buffer rather than
        // copying it, so `ZeroizingGatewayTarget`'s `Drop` erases the same bytes
        // `expose_secret().to_string()` wrote. `GwClient::connect` takes the
        // target by reference, so that `Drop` runs on every exit path below,
        // including both `return Err`.
        //
        // Wrapping this in `Zeroizing` and then copying it into the field looks
        // like belt and braces and is the opposite: it adds a second plaintext
        // allocation that did not exist, to guard an early return that cannot
        // happen — there is no fallible operation between here and the struct
        // literal. Leave it as a move.
        let gw_pass = config
            .password
            .as_ref()
            .map(|secret| secret.expose_secret().to_string())
            .unwrap_or_default();
        let gw_target = ZeroizingGatewayTarget(ironrdp_mstsgu::GwConnectTarget {
            gw_endpoint,
            gw_user,
            gw_pass,
            smart_card: None,
            server: config.host.clone(),
        });
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
        connect_direct(config, connect_timeout).await?
    };

    #[cfg(not(feature = "rd-gateway"))]
    let (stream, client_addr) = connect_direct(config, connect_timeout).await?;

    // Phase 2: Build IronRDP connector configuration
    let connector_config = build_connector_config(config);
    let mut connector = ClientConnector::new(connector_config, client_addr);

    // Routing to the right session host: the configured broker routing value
    // on the first connection, then what the broker's redirection asks for.
    let routing_token = redirect
        .map_or_else(
            || config.load_balance_info.clone(),
            |redirect| redirect.routing_token.clone(),
        )
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty());
    if let Some(token) = routing_token {
        connector = connector.with_load_balance_info(token);
    }
    if let Some(redirect) = redirect {
        use ironrdp_pdu::gcc::{ClientClusterData, RedirectionFlags, RedirectionVersion};
        connector = connector.with_cluster_data(ClientClusterData {
            flags: RedirectionFlags::REDIRECTION_SUPPORTED
                | RedirectionFlags::REDIRECTED_SESSION_FIELD_VALID,
            redirection_version: RedirectionVersion::V4,
            redirected_session_id: redirect.session_id,
        });
        if let Some(cookie) = redirect.password_cookie.as_ref() {
            connector = connector.with_redirection_password_cookie(cookie.to_vec());
        }
    }

    // Phase 2.5: Add clipboard channel if enabled
    if config.clipboard_direction.enabled() {
        let clipboard_backend = RustConnClipboardBackend::new_with_direction(
            event_tx.clone(),
            config.clipboard_direction,
        );
        let cliprdr: CliprdrClient = ironrdp_cliprdr::Cliprdr::new(Box::new(clipboard_backend));
        connector.static_channels.insert(cliprdr);
        tracing::debug!(direction = ?config.clipboard_direction, "Clipboard channel enabled");
    }

    // Phase 2.6: Add the RDPDR and RDPSND channels — see `needs_rdpdr`.
    if needs_rdpdr(config) {
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
    }

    // Register DRDYNVC (Dynamic Virtual Channel) with Echo client for RTT measurement.
    // DisplayControlClient is also registered here for dynamic resolution changes (MS-RDPEDISP).
    // The Echo channel (MS-RDPEECO) responds to server echo requests, enabling the server
    // to measure round-trip time and report it back via Auto-Detect PDU.
    // DisplayControlClient::new takes a capabilities callback. We send the
    // monitor layout on demand via ActiveStage::encode_resize, so nothing is emitted when
    // capabilities arrive — return an empty message set.
    let dc_ready_tx = event_tx.clone();
    let mut drdynvc = DrdynvcClient::new()
        .with_dynamic_channel(ironrdp_displaycontrol::client::DisplayControlClient::new(
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

    // Register the EGFX graphics pipeline (MS-RDPEGFX) unless the connection
    // asks for the Legacy or RemoteFX path, which is also the retry when the
    // pipeline fails (issue #218).
    //
    // ironrdp-egfx decodes ClearCodec, Planar and RFX Progressive itself, so
    // the channel works without OpenH264; the decoder adds AVC420, and AVC444
    // where it returns YUV planes, and the client advertises only what it can
    // decode. Up to ironrdp-egfx 0.3 nothing but H.264 could be painted, so a
    // session without OpenH264 stayed off the pipeline to avoid the frozen
    // desktop of issue #262; that reason is gone.
    //
    // Advertising the pipeline in the early capability flags is what makes a
    // Windows host open the channel at all ([MS-RDPEGFX] 1.5.1). The connector
    // could not set it before ironrdp-connector 0.11, and such a host then kept
    // the session on bitmap updates — blurry with RemoteFX, heavy without.
    #[cfg(feature = "gfx-h264")]
    {
        use crate::rdp_client::graphics::GraphicsMode;
        if matches!(
            config.graphics_mode,
            GraphicsMode::Legacy | GraphicsMode::RemoteFx
        ) {
            tracing::info!(
                graphics_mode = ?config.graphics_mode,
                "EGFX pipeline skipped (graphics_mode forces Legacy/RemoteFX path)"
            );
        } else {
            let h264_decoder = try_load_openh264();
            if h264_decoder.is_none() {
                // Tell the GUI why quality dropped: H.264 was wanted but no
                // decoder loaded (issue #262). The reason distinguishes "not
                // found" from the actionable "found but not a Cisco build".
                if let Some(reason) = super::super::gfx_handler::openh264_unavailable_reason() {
                    let _ = event_tx.send(RdpClientEvent::H264Unavailable { reason });
                }
                tracing::warn!(
                    reason = "openh264_unavailable",
                    "EGFX pipeline registered without H.264 — the server picks a non-AVC codec"
                );
            }
            let has_h264 = h264_decoder.is_some();
            let handler = RustConnGfxHandler::new(event_tx.clone());
            let gfx_client = GraphicsPipelineClient::new(Box::new(handler), h264_decoder);
            drdynvc = drdynvc.with_dynamic_channel(gfx_client);
            connector.config.support_dyn_vc_gfx_protocol = true;
            tracing::info!(h264 = has_h264, "EGFX pipeline registered");
        }
    }

    // Microphone (MS-RDPEAI). Its data packets are sent between the server's
    // turns, so the channel queues them for the session loop.
    let microphone = if config.microphone_enabled {
        use crate::rdp_client::microphone::{MicrophoneSink, RustConnMicrophone};

        let sink: MicrophoneSink = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (uplink_tx, uplink_rx) = tokio::sync::mpsc::unbounded_channel();
        let uplink: ironrdp_rdpeai::client::DvcUplink = Box::new(move |_channel_id, messages| {
            uplink_tx
                .send(messages)
                .map_err(|_| ironrdp_pdu::pdu_other_err!("AUDIO_INPUT session loop has ended"))
        });
        let handler = RustConnMicrophone::new(std::sync::Arc::clone(&sink), event_tx.clone());
        drdynvc = drdynvc.with_dynamic_channel(ironrdp_rdpeai::client::RdpeaiClient::new(
            Box::new(handler),
            uplink,
        ));
        tracing::info!("AUDIO_INPUT channel registered (microphone)");
        Some(MicrophoneLink { sink, uplink_rx })
    } else {
        None
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

    // Separates "the server never answered" from "the server answered and
    // then stalled mid-handshake" (issue #392): set once the TLS upgrade and
    // TOFU certificate check succeed, read only after the `timeout(..)` below
    // has resolved. `&AtomicBool` is `Send + Sync`, so the async block can
    // hold a shared reference without an `Arc`; `Cell` would not do, since
    // `&Cell<_>` is not `Send`. `Ordering::Relaxed` is enough because the flag
    // guards nothing else — the `.await` boundary already happened-after the
    // store by the time this function reads it.
    let secure_channel_up = AtomicBool::new(false);

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
                let kind = format!("{:?}", e.kind());
                tracing::error!(
                    protocol = "rdp",
                    error = %e,
                    error_kind = %kind,
                    "IronRDP connect_begin failed"
                );
                map_connector_error("Connection begin failed", &e.to_string(), &kind)
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

        // Trust-on-first-use certificate check. IronRDP does not validate the
        // certificate against a CA store (RDP servers are overwhelmingly
        // self-signed), so RustConn applies the same model SSH uses for host
        // keys: record the fingerprint the first time a host is seen and reject
        // a later connection whose fingerprint has changed. `ignore_certificate`
        // opts out entirely, matching xfreerdp `/cert:ignore`.
        verify_server_certificate(config, &server_cert)?;
        // TLS is up and the server's identity is accepted: a timeout from here
        // on means the server is reachable and speaking RDP, not that it is
        // unreachable (issue #392).
        secure_channel_up.store(true, Ordering::Relaxed);

        // Extract server public key from certificate
        let server_public_key = ironrdp_tls::extract_tls_server_public_key(&server_cert)
            .map(<[u8]>::to_vec)
            .unwrap_or_default();

        let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

        let mut upgraded_framed = TokioFramed::new(Box::new(upgraded_stream) as RdpStream);

        // Create network client for Kerberos/AAD authentication
        let mut network_client = ReqwestNetworkClient::new();

        // Kerberos for NLA (issue #351): off unless the profile opts in and NLA
        // is on. sspi signs in with the stored password — a `kinit` ticket is
        // never read — and needs a KDC: the profile's KDC Address, else what sspi
        // finds in SSPI_KDC_URL_<REALM>, SSPI_KDC_URL or krb5.conf, else the
        // realm's own DNS name (see `kerberos_settings_for`). A KDC that is
        // missing or unreachable fails the sign-in; there is no NTLM fallback to
        // catch it, and `failure::classify_auth_failure` names the cause.
        let kerberos_config = kerberos_settings_for(config, sspi_finds_kdc)
            .and_then(|settings| kerberos_connector_config(config, settings));

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
        // Re-checked for ironrdp-connector 0.12: the upstream connect_finalize
        // panic reports remain open (e.g.
        // https://github.com/Devolutions/IronRDP/issues/1016), so the wrapper
        // stays. Re-evaluate on the next IronRDP bump.
        let finalize_future = AssertUnwindSafe(ironrdp_tokio::connect_finalize(
            upgraded,
            connector,
            &mut upgraded_framed,
            &mut network_client,
            ServerName::new(&config.host),
            server_public_key,
            kerberos_config,
        ));

        let connection_result = match futures::FutureExt::catch_unwind(finalize_future).await {
            Ok(Ok(ConnectionOutcome::Connected(result))) => result,
            // A connection broker hands the session to another host;
            // `establish_connection` connects there next.
            Ok(Ok(ConnectionOutcome::Redirected(pdu))) => {
                tracing::info!(
                    protocol = "rdp",
                    "IronRDP connect_finalize: the server redirected the session"
                );
                return Ok(Finalized::Redirected(pdu));
            }
            Ok(Err(e)) => {
                let kind = format!("{:?}", e.kind());
                tracing::error!(
                    protocol = "rdp",
                    error = %e,
                    error_kind = %kind,
                    "IronRDP connect_finalize failed"
                );
                return Err(map_connector_error(
                    "Connection finalize failed",
                    &e.to_string(),
                    &kind,
                ));
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

        Ok::<_, RdpClientError>(Finalized::Connected(Box::new((
            upgraded_framed,
            connection_result,
        ))))
    })
    .await;

    if let Ok(result) = handshake_result {
        match result? {
            Finalized::Connected(connected) => {
                let (framed, connection_result) = *connected;
                Ok(Attempt::Connected(Box::new(ConnectionSetup {
                    framed,
                    connection_result,
                    microphone,
                })))
            }
            Finalized::Redirected(pdu) => Ok(Attempt::Redirected(pdu)),
        }
    } else if secure_channel_up.load(Ordering::Relaxed) {
        // TLS (and TOFU) succeeded before the budget ran out: the server is
        // reachable and speaking RDP, but the connect sequence (NLA, licensing,
        // or capabilities) never finished. Hand the session to the external
        // FreeRDP client instead of the generic `Timeout`/`Other` path below —
        // see `rdp_client::failure::post_tls_stall_error` (issue #392).
        tracing::warn!(
            protocol = "rdp",
            host = %config.host,
            port = %config.port,
            timeout_secs = handshake_secs,
            is_tunnel,
            "IronRDP connection sequence stalled after the TLS upgrade \
             (sign-in/licensing/capabilities); handing the session to the \
             external client"
        );
        Err(crate::rdp_client::failure::post_tls_stall_error(
            handshake_secs,
        ))
    } else {
        tracing::error!(
            protocol = "rdp",
            host = %config.host,
            port = %config.port,
            timeout_secs = handshake_secs,
            is_tunnel,
            "RDP handshake timed out (before the TLS upgrade completed). \
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

/// How the connection sequence of one attempt ended.
enum Finalized {
    Connected(Box<(UpgradedFramed, ConnectionResult)>),
    Redirected(Box<ironrdp_pdu::rdp::server_redirection::ServerRedirectionPdu>),
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
    // - Quality: lossy_compression=false (lossless), no RemoteFX, all visual effects
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
        keyboard_type: KeyboardType::IBM_ENHANCED,
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
        // What ironrdp-connector 0.10 sent implicitly: one monitor, no
        // physical size, orientation from the desktop size, device scale 100.
        monitor_layout: None,
        monitor_extended_layout: None,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        relative_mouse_input: false,
        connection_type: ironrdp_pdu::gcc::ConnectionType::Lan,
        enable_standard_rdp_security: false,
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
        // RemoteApp always goes to FreeRDP, so the embedded client never
        // enters the RAIL connection model.
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::empty(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        // Ask the server to log the session in with the credentials this PDU
        // already carries (`INFO_AUTOLOGON`).
        //
        // Windows never needed it: the session is established from the
        // CredSSP/NLA exchange, so the flag being off went unnoticed. xrdp does
        // need it — its session manager only skips its own login screen when
        // the flag is set, so connecting to a Linux host got past the first
        // stage with the password accepted and then stopped at the greeter,
        // asking for the same credentials again. FreeRDP sets this flag
        // whenever credentials are supplied, which is why the very same host
        // logs straight in from Remmina.
        //
        // Off when there is nothing to log in with: the flag would promise an
        // automatic logon the PDU cannot deliver, and a server that trusts it
        // may drop the connection instead of showing its own prompt.
        autologon: config
            .username
            .as_ref()
            .is_some_and(|username| !username.is_empty())
            && config
                .password
                .as_ref()
                .is_some_and(|password| !password.expose_secret().is_empty()),
        enable_audio_playback: config.audio_enabled,
        // INFO_AUDIOCAPTURE, only alongside a registered AUDIO_INPUT channel.
        enable_audio_capture: config.microphone_enabled,
        performance_flags,
        license_cache: None,
        timezone_info: get_timezone_info(),
        // Bulk compression (XCRUSH, RDP 6.1; the server may pick anything up
        // to it). It shrinks every PDU the server sends — the lossless
        // bitmaps of the Quality mode above all, which otherwise go out
        // uncompressed on a session without the graphics pipeline.
        //
        // It was off until ironrdp-session 0.12 because the session rebuilt
        // its decompressor on every Deactivation-Reactivation Sequence (a
        // window resize), lost the shared history and aborted with a
        // bulk-decompression error (issue #200). The session now owns one
        // decompressor and `ActiveStage::reactivate` keeps it.
        compression_type: Some(ironrdp_pdu::rdp::client_info::CompressionType::Rdp61),
        enable_server_pointer: true,
        // Use hardware pointer - server sends cursor bitmap separately
        // This avoids cursor artifacts in the framebuffer
        pointer_software_rendering: false,
        // Multitransport (UDP sideband) — not implemented yet
        multitransport_flags: None,
        // Set once the EGFX channel is registered (`connect_with_stream`):
        // advertising the graphics pipeline without a processor for it leaves
        // the desktop blank.
        support_dyn_vc_gfx_protocol: false,
    }
}

/// Whether the session needs the device redirection channel (RDPDR).
///
/// RDPDR and RDPSND go together both ways. RDPDR needs RDPSND (MS-RDPEFS),
/// and a Windows host starts sound on RDPSND only once RDPDR is joined too —
/// FreeRDP registers RDPDR whenever sound is on for that reason. Audio used to
/// get RDPSND alone when nothing was shared, and the server never played a
/// sound: no format list, no wave, no error.
fn needs_rdpdr(config: &RdpClientConfig) -> bool {
    !config.shared_folders.is_empty() || config.printer_enabled || config.audio_enabled
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
/// - `codecs`: RemoteFX for Balanced only; empty (lossless or RLE bitmaps) for
///   Quality and Speed
fn build_bitmap_config(mode: crate::models::RdpPerformanceMode) -> Option<BitmapConfig> {
    use crate::models::RdpPerformanceMode;

    match mode {
        RdpPerformanceMode::Quality => {
            // Best quality: lossless 32-bit bitmaps (RDP 6.0 planar).
            // drawing_flags = ALLOW_SKIP_ALPHA only (no color subsampling)
            //
            // No RemoteFX: it is a lossy wavelet codec, and a server offered it
            // prefers it, so advertising it here undid the lossless request.
            // That decides what the user sees whenever the server does not open
            // the GFX pipeline — some Windows hosts and policies never do — and
            // text came out soft and smeared. Lossless costs bandwidth; Balanced
            // keeps RemoteFX for links where that matters.
            Some(BitmapConfig {
                lossy_compression: false,
                color_depth: 32,
                codecs: BitmapCodecs(vec![]),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_tokens_lose_the_crlf_and_must_be_printable() {
        assert_eq!(
            routing_token_from_bytes(b"Cookie: msts=3640205228.15629.0000\r\n").as_deref(),
            Some("Cookie: msts=3640205228.15629.0000")
        );
        assert_eq!(
            routing_token_from_bytes(b"tsv://MS Terminal Services Plugin.1.Sessions").as_deref(),
            Some("tsv://MS Terminal Services Plugin.1.Sessions")
        );
        assert_eq!(routing_token_from_bytes(b"\r\n"), None);
        assert_eq!(routing_token_from_bytes(&[0xFF, 0x00]), None);
        assert_eq!(routing_token_from_bytes(b"tab\there"), None);
    }

    #[test]
    fn a_redirection_moves_to_the_session_host_with_its_ticket() {
        use ironrdp_pdu::rdp::server_redirection::ServerRedirectionPdu;

        let mut broker = RdpClientConfig::new("broker.corp.example");
        broker.username = Some("jdoe".to_string());
        broker.load_balance_info = Some("tsv://MS Terminal Services Plugin.1.Sessions".to_string());

        let pdu = ServerRedirectionPdu {
            session_id: 42,
            target_fqdn: Some("host7.corp.example\0".to_string()),
            target_net_address: Some("10.0.0.7".to_string()),
            load_balance_info: Some(b"Cookie: msts=1.2.3\r\n".to_vec()),
            password: Some(vec![1, 2, 3]),
            domain: Some("CORP".to_string()),
            ..ServerRedirectionPdu::default()
        };
        let (next, redirect) = follow_redirection(&broker, &pdu);
        assert_eq!(
            next.host, "host7.corp.example",
            "FQDN wins over the address"
        );
        assert_eq!(next.port, broker.port);
        assert_eq!(
            next.username.as_deref(),
            Some("jdoe"),
            "kept when the PDU has none"
        );
        assert_eq!(next.domain.as_deref(), Some("CORP"));
        assert_eq!(
            next.load_balance_info, None,
            "the broker's value has been used"
        );
        assert_eq!(
            redirect.routing_token.as_deref(),
            Some("Cookie: msts=1.2.3")
        );
        assert_eq!(redirect.session_id, 42);
        assert!(
            !format!("{redirect:?}").contains("[1, 2, 3]"),
            "the cookie must not print"
        );

        // A redirection with only a routing token reconnects to the same host.
        let token_only = ServerRedirectionPdu {
            load_balance_info: Some(b"Cookie: msts=9\r\n".to_vec()),
            ..ServerRedirectionPdu::default()
        };
        let (same, _) = follow_redirection(&broker, &token_only);
        assert_eq!(same.host, "broker.corp.example");
    }

    #[test]
    fn sound_brings_the_device_channel_with_it() {
        let base = RdpClientConfig::new("server.example.com");
        assert!(!needs_rdpdr(&base));
        assert!(needs_rdpdr(&base.clone().with_audio(true)));
        assert!(needs_rdpdr(&base.with_printer(true)));
    }

    /// Quality asks for lossless bitmaps and offers no lossy codec a server
    /// would prefer; only Balanced offers RemoteFX.
    #[test]
    fn only_balanced_offers_remotefx() {
        use crate::models::RdpPerformanceMode;

        for (mode, lossy, offers_codecs) in [
            (RdpPerformanceMode::Quality, false, false),
            (RdpPerformanceMode::Balanced, true, true),
            (RdpPerformanceMode::Speed, true, false),
        ] {
            let config = build_bitmap_config(mode).expect("every mode sends a bitmap config");
            assert_eq!(config.lossy_compression, lossy, "{mode:?}");
            assert_eq!(config.color_depth, 32, "{mode:?}");
            assert_eq!(!config.codecs.0.is_empty(), offers_codecs, "{mode:?}");
        }
    }

    /// Issue #392: a connect sequence that stalls *before* TLS (the server
    /// accepts the TCP connection but never answers X.224) must keep failing
    /// as a non-fallback error, exactly as before this change. The post-TLS
    /// branch cannot be driven the same way without a TLS server in the test,
    /// so it is covered by `failure::post_tls_stall_falls_back_to_the_external_client`
    /// instead; this test only pins down the "before TLS" side of the new
    /// `if secure_channel_up.load(..)` branch added to this function.
    #[tokio::test]
    async fn stall_before_tls_stays_a_non_fallback_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding a loopback listener must not fail in this sandbox");
        let port = listener
            .local_addr()
            .expect("bound listener has an addr")
            .port();

        // Accept the connection and hold it open without writing anything, so
        // `connect_begin`'s X.224 exchange never completes and the handshake
        // timeout fires before the TLS upgrade is ever reached.
        tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                // Hold the socket open for the rest of the test (dropped when
                // the single-threaded test runtime shuts down) instead of
                // writing or closing it, so the client's X.224 read blocks.
                let _socket = socket;
                std::future::pending::<()>().await;
            }
        });

        let config = RdpClientConfig {
            host: "127.0.0.1".to_string(),
            port,
            timeout_secs: 1,
            clipboard_direction: crate::models::RdpClipboardDirection::Off,
            ..Default::default()
        };
        let (tx, _rx) = std::sync::mpsc::channel();

        let result = establish_connection(&config, tx).await;

        // `ConnectionSetup` is not `Debug` (it holds a framed IronRDP stream),
        // so `Result::expect_err` cannot be used here.
        let Err(err) = result else {
            panic!("a server that never answers X.224 must not connect");
        };
        let class = crate::rdp_client::failure::classify_rdp_failure(&err.to_string());
        assert_eq!(
            class,
            crate::rdp_client::failure::RdpFailureClass::Other,
            "error: {err}"
        );
        assert!(
            !class.warrants_freerdp_fallback(),
            "a pre-TLS stall must not trigger the external-client fallback: {err}"
        );
    }
}
