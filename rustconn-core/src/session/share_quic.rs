//! WebTransport (HTTP/3 over QUIC) transport for read-only session sharing.
//!
//! The TCP relay in [`crate::session::share`] is limited to a host a browser can
//! reach directly. This module adds a browser-native viewer: a small HTTPS page
//! served over TCP-TLS and the terminal stream carried by WebTransport over
//! QUIC. STUN discovers the public UDP endpoint, so a viewer outside the LAN can
//! connect when the router keeps an endpoint-independent UDP mapping (or the
//! user forwards the port).
//!
//! # Why two ports
//!
//! A browser does not try HTTP/3 on a first connection: it needs a TCP page
//! that advertises the alternative service. The page is therefore served over
//! TCP-TLS with the same certificate, and the link points the WebTransport
//! client at the UDP port. When both fit on one port the sharer uses it for
//! both; otherwise the link carries the UDP port separately.
//!
//! # Trust model
//!
//! The server uses a fresh ECDSA P-256 self-signed certificate per share,
//! valid for at most two weeks as WebTransport requires. Its SHA-256 travels
//! inside the link fragment, and the browser pins exactly that certificate
//! through `serverCertificateHashes`, so the token is never sent to an
//! unauthenticated peer. The page is static; the token never appears in a
//! request the server could log.
//!
//! # NAT
//!
//! STUN creates the UDP mapping and a keepalive thread refreshes it. A full-cone
//! (endpoint-independent filtering) router accepts a viewer that the host has
//! not contacted; a restricted or symmetric NAT, including most CGNAT, drops
//! it, and no loopback trick can change that — a relay is then the only path.

use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tracing::{debug, info};
use wtransport::endpoint::endpoint_side;
use wtransport::{Identity, ServerConfig};

use super::share::{XTERM_CSS, XTERM_JS, constant_time_eq, percent_decode};

/// STUN binding requests, tried in order until one answers.
const STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.cloudflare.com:3478",
];
const STUN_MAGIC: u32 = 0x2112_A442;
const STUN_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Slow viewers drop frames instead of blocking the terminal.
const CHANNEL_CAPACITY: usize = 64;
/// A request line and its headers; anything larger is an attack, not a viewer.
const MAX_REQUEST: usize = 16 * 1024;

/// Static browser page. The token and certificate pin live in the URL
/// fragment, which the browser never sends to the page server.
const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>Live session</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/xterm.css">
<style>html,body{margin:0;height:100%;background:#111}#term{height:100vh;padding:4px;box-sizing:border-box;overflow:auto}
#status{position:fixed;left:8px;bottom:6px;margin:0;color:#ccc;font:12px system-ui;background:#111c;padding:2px 6px;border-radius:4px}
#status:empty{display:none}</style>
</head><body><div id="term"></div><p id="status">Connecting…</p>
<script src="/xterm.js"></script><script>
var hash = new URLSearchParams(location.hash.slice(1));
var token = hash.get('token') || '';
var fp = hash.get('fp') || '';
var cols = Math.min(Math.max(parseInt(hash.get('cols'), 10) || 80, 20), 500);
var rows = Math.min(Math.max(parseInt(hash.get('rows'), 10) || 24, 5), 300);
var statusEl = document.getElementById('status');
var term = new Terminal({cols: cols, rows: rows, scrollback: 2000, disableStdin: true,
  fontSize: 13, theme: {background: '#111111', foreground: '#eeeeee'}});
term.open(document.getElementById('term'));
function digest(hex) {
  if (!/^[0-9a-f]{64}$/i.test(hex)) return null;
  var out = new Uint8Array(32);
  for (var i = 0; i < 32; i++) out[i] = parseInt(hex.substr(i * 2, 2), 16);
  return out;
}
var pin = digest(fp);
if (!pin) {
  statusEl.textContent = 'This link has no valid certificate pin.';
} else if (!('WebTransport' in window)) {
  statusEl.textContent = 'This browser has no WebTransport support. Use Chrome or Edge 100+, Firefox 125+ or Safari 26.4+.';
} else {
  var wtPort = hash.get('wt');
  var wt = new URL(location.href);
  if (wtPort) wt.port = wtPort;
  wt.hash = '';
  wt.pathname = '/';
  wt.search = 'token=' + encodeURIComponent(token);
  var transport = new WebTransport(wt.href, {serverCertificateHashes: [{algorithm: 'sha-256', value: pin}]});
  transport.ready.then(function () {
    statusEl.textContent = '';
    var streams = transport.incomingUnidirectionalStreams.getReader();
    function nextStream() {
      streams.read().then(function (item) {
        if (item.done) return;
        var reader = item.value.getReader();
        function nextChunk() {
          reader.read().then(function (chunk) {
            if (chunk.done) { nextStream(); return; }
            term.write(chunk.value);
            nextChunk();
          }).catch(function (e) { statusEl.textContent = String(e); });
        }
        nextChunk();
      }).catch(function (e) { statusEl.textContent = String(e); });
    }
    nextStream();
  }).catch(function (e) {
    statusEl.textContent = 'Could not connect: ' + (e && e.message ? e.message : e);
  });
}
</script></body></html>
"#;

/// Errors produced by the WebTransport share transport.
#[derive(Debug, Error)]
pub enum ShareQuicError {
    /// A socket or the page listener could not be bound or used.
    #[error("share QUIC I/O error: {0}")]
    Io(#[from] io::Error),
    /// Certificate generation or TLS configuration failed.
    #[error("share QUIC TLS error: {0}")]
    Tls(String),
    /// A viewer could not reach or keep the connection.
    #[error("share QUIC connection failed: {0}")]
    Connect(String),
    /// The pasted link is not a share link this client understands.
    #[error("invalid share link: {0}")]
    Link(String),
}

/// A parsed `https://host:page_port/#token=…&fp=…&wt=…&cols=…&rows=…` link.
pub struct ShareLink {
    /// Host the sharer is reachable on.
    pub host: IpAddr,
    /// UDP port the WebTransport endpoint listens on.
    pub wt_port: u16,
    /// Read-only access token.
    pub token: SecretString,
    /// SHA-256 of the server certificate, pinned by the viewer.
    pub fingerprint: [u8; 32],
    /// Terminal width the viewer should open with.
    pub cols: u16,
    /// Terminal height the viewer should open with.
    pub rows: u16,
}

impl fmt::Debug for ShareLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShareLink")
            .field("host", &self.host)
            .field("wt_port", &self.wt_port)
            .field("token", &"[REDACTED]")
            .field("fingerprint", &self.fingerprint)
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .finish()
    }
}

impl ShareLink {
    /// Parses a link produced by [`QuicShareServer::viewer_link_for`].
    ///
    /// # Errors
    ///
    /// Returns [`ShareQuicError::Link`] when the scheme, address, token or
    /// fingerprint is missing or malformed.
    pub fn parse(input: &str) -> Result<Self, ShareQuicError> {
        let rest = input
            .trim()
            .strip_prefix("https://")
            .ok_or_else(|| ShareQuicError::Link("expected https://".into()))?;
        let (authority, fragment) = rest
            .split_once('#')
            .ok_or_else(|| ShareQuicError::Link("missing link fragment".into()))?;
        let authority = authority.split('/').next().unwrap_or(authority);
        let (host, page_port) = parse_authority(authority)
            .ok_or_else(|| ShareQuicError::Link("expected an IP address and port".into()))?;

        let mut token = None;
        let mut fingerprint = None;
        let mut wt_port = page_port;
        let mut cols = 80;
        let mut rows = 24;
        for pair in fragment.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match key {
                "token" if !value.is_empty() => {
                    token = Some(SecretString::from(percent_decode(value)));
                }
                "fp" => {
                    let bytes = data_encoding::HEXLOWER
                        .decode(value.as_bytes())
                        .map_err(|_| ShareQuicError::Link("fingerprint is not hex".into()))?;
                    fingerprint =
                        Some(<[u8; 32]>::try_from(bytes).map_err(|_| {
                            ShareQuicError::Link("fingerprint is not 32 bytes".into())
                        })?);
                }
                "wt" => {
                    if let Ok(port) = value.parse::<u16>() {
                        wt_port = port;
                    }
                }
                "cols" => {
                    if let Ok(value) = value.parse::<u16>() {
                        cols = value.clamp(20, 500);
                    }
                }
                "rows" => {
                    if let Ok(value) = value.parse::<u16>() {
                        rows = value.clamp(5, 300);
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            host,
            wt_port,
            token: token.ok_or_else(|| ShareQuicError::Link("missing token".into()))?,
            fingerprint: fingerprint
                .ok_or_else(|| ShareQuicError::Link("missing fingerprint".into()))?,
            cols,
            rows,
        })
    }

    /// URL the native WebTransport client connects to.
    fn url(&self) -> String {
        format!(
            "https://{}:{}/?token={}",
            url_host(self.host),
            self.wt_port,
            percent_encode_token(self.token.expose_secret())
        )
    }
}

fn parse_authority(authority: &str) -> Option<(IpAddr, u16)> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, rest) = rest.split_once(']')?;
        (host, rest.strip_prefix(':')?)
    } else {
        authority.rsplit_once(':')?
    };
    Some((host.parse().ok()?, port.parse().ok()?))
}

/// A running WebTransport share server.
pub struct QuicShareServer {
    endpoint: Arc<wtransport::Endpoint<endpoint_side::Server>>,
    udp_addr: SocketAddr,
    page_addr: SocketAddr,
    public_addrs: Vec<SocketAddr>,
    fingerprint: [u8; 32],
    token: SecretString,
    publisher: broadcast::Sender<Vec<u8>>,
    accept: JoinHandle<()>,
    page: JoinHandle<()>,
    _keepalive: Option<Keepalive>,
}

impl fmt::Debug for QuicShareServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuicShareServer")
            .field("udp_addr", &self.udp_addr)
            .field("page_addr", &self.page_addr)
            .field("public_addrs", &self.public_addrs)
            .field("token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl QuicShareServer {
    /// Binds the UDP socket, asks STUN for the public endpoint, then serves the
    /// page over TCP-TLS and the terminal over WebTransport.
    ///
    /// # Errors
    ///
    /// Returns [`ShareQuicError`] when the socket cannot be bound, the
    /// certificate cannot be generated, or the TLS endpoints cannot be
    /// created. STUN failure is not an error: the server still starts and
    /// [`Self::public_addrs`] stays empty.
    pub async fn start(bind: SocketAddr, token: SecretString) -> Result<Self, ShareQuicError> {
        let socket = std::net::UdpSocket::bind(bind)?;
        socket.set_nonblocking(true)?;
        let tokio_socket = UdpSocket::from_std(socket)?;
        let (public_addrs, keepalive_target) = discover_external(&tokio_socket).await;
        let socket = tokio_socket.into_std()?;
        start_on_socket(socket, token, public_addrs, keepalive_target).await
    }

    /// UDP endpoint the WebTransport server listens on.
    #[must_use]
    pub fn udp_addr(&self) -> SocketAddr {
        self.udp_addr
    }

    /// TCP endpoint the viewer page is served from.
    #[must_use]
    pub fn page_addr(&self) -> SocketAddr {
        self.page_addr
    }

    /// Public UDP endpoints reported by STUN, in discovery order.
    #[must_use]
    pub fn public_addrs(&self) -> &[SocketAddr] {
        &self.public_addrs
    }

    /// Sender the GUI uses to publish raw terminal output bytes.
    pub fn publisher(&self) -> broadcast::Sender<Vec<u8>> {
        self.publisher.clone()
    }

    /// Link that opens `target`'s page and streams over its WebTransport port.
    ///
    /// `target` is the UDP endpoint a viewer reaches: a STUN result (public IP
    /// and port) or a local address. The page is always on [`Self::page_addr`];
    /// the fragment carries the certificate pin, the access token and, when it
    /// differs, the WebTransport port.
    #[must_use]
    pub fn viewer_link_for(&self, target: SocketAddr, cols: u16, rows: u16) -> String {
        let cols = cols.clamp(20, 500);
        let rows = rows.clamp(5, 300);
        let wt = if target.port() == self.page_addr.port() {
            String::new()
        } else {
            format!("&wt={}", target.port())
        };
        format!(
            "https://{}:{}/#token={}&fp={}{wt}&cols={cols}&rows={rows}",
            url_host(target.ip()),
            self.page_addr.port(),
            percent_encode_token(self.token.expose_secret()),
            data_encoding::HEXLOWER.encode(&self.fingerprint)
        )
    }

    /// Closes the listeners and stops the keepalive thread.
    pub fn stop(self) {
        self.endpoint
            .close(wtransport::VarInt::from_u32(0), b"stop");
        self.accept.abort();
        self.page.abort();
    }
}

/// A native WebTransport viewer that feeds the shared terminal to `on_bytes`.
pub struct QuicShareClient {
    endpoint: wtransport::Endpoint<endpoint_side::Client>,
    task: JoinHandle<()>,
}

impl fmt::Debug for QuicShareClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuicShareClient").finish_non_exhaustive()
    }
}

impl QuicShareClient {
    /// Connects to a share link and streams terminal output into `on_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`ShareQuicError::Connect`] when the pinned certificate does not
    /// match, the host does not answer over UDP, or the token is refused.
    pub async fn connect(
        link: &ShareLink,
        on_bytes: impl Fn(Vec<u8>) + Send + 'static,
    ) -> Result<Self, ShareQuicError> {
        let config = wtransport::ClientConfig::builder()
            .with_bind_default()
            .with_server_certificate_hashes([wtransport::tls::Sha256Digest::from(link.fingerprint)])
            .build();
        let endpoint = wtransport::Endpoint::client(config)?;
        let connection = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(link.url()))
            .await
            .map_err(|_| ShareQuicError::Connect("timed out".into()))?
            .map_err(|error| ShareQuicError::Connect(error.to_string()))?;
        let task = tokio::spawn(async move {
            let Ok(mut recv) = connection.accept_uni().await else {
                debug!("share quic: server opened no stream");
                return;
            };
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match recv.read(&mut buf).await {
                    Ok(Some(n)) if n > 0 => on_bytes(buf[..n].to_vec()),
                    _ => break,
                }
            }
            connection.close(wtransport::VarInt::from_u32(0), b"done");
        });
        info!(host = %link.host, port = link.wt_port, "share quic: viewer connected");
        Ok(Self { endpoint, task })
    }

    /// Closes the connection and its reader task.
    pub fn stop(self) {
        self.endpoint
            .close(wtransport::VarInt::from_u32(0), b"stop");
        self.task.abort();
    }
}

/// Serves WebTransport on an already-bound socket and the page on TCP-TLS.
async fn start_on_socket(
    socket: std::net::UdpSocket,
    token: SecretString,
    public_addrs: Vec<SocketAddr>,
    keepalive_target: Option<SocketAddr>,
) -> Result<QuicShareServer, ShareQuicError> {
    socket.set_nonblocking(true)?;
    let bind_ip = socket.local_addr()?.ip();
    let keepalive_socket = keepalive_target.map(|_| socket.try_clone()).transpose()?;
    let identity = identity_for(&public_addrs)?;
    let certificate = identity
        .certificate_chain()
        .as_slice()
        .first()
        .ok_or_else(|| ShareQuicError::Tls("identity has no certificate".into()))?;
    let fingerprint = *certificate.hash().as_ref();
    let page_tls = page_tls_config(&identity)?;

    let server_config = ServerConfig::builder()
        .with_bind_socket(socket)
        .with_identity(identity.clone_identity())
        .keep_alive_interval(Some(Duration::from_secs(10)))
        .build();
    let endpoint = wtransport::Endpoint::server(server_config)?;
    let udp_addr = endpoint.local_addr()?;
    let endpoint = Arc::new(endpoint);

    // Prefer the UDP port for the page too, so one forwarded port number covers
    // both protocols; fall back to an ephemeral port when TCP already uses it.
    let page_listener = match TcpListener::bind((bind_ip, udp_addr.port())).await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind((bind_ip, 0)).await?,
    };
    let page_addr = page_listener.local_addr()?;

    let (publisher, _) = broadcast::channel(CHANNEL_CAPACITY);
    let accept = tokio::spawn(accept_loop(
        Arc::clone(&endpoint),
        token.clone(),
        publisher.clone(),
    ));
    let page = tokio::spawn(page_loop(
        page_listener,
        TlsAcceptor::from(Arc::new(page_tls)),
    ));
    let keepalive = keepalive_socket
        .zip(keepalive_target)
        .map(|(socket, target)| Keepalive::start(socket, target));
    info!(%udp_addr, %page_addr, public = ?public_addrs, "share quic: server started");
    Ok(QuicShareServer {
        endpoint,
        udp_addr,
        page_addr,
        public_addrs,
        fingerprint,
        token,
        publisher,
        accept,
        page,
        _keepalive: keepalive,
    })
}

/// Certificate for both the page and WebTransport, valid for the hosts the
/// link may name.
fn identity_for(public_addrs: &[SocketAddr]) -> Result<Identity, ShareQuicError> {
    let mut names = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    for interface in crate::network_scan::local_interfaces() {
        push_name(&mut names, interface.address);
    }
    for addr in public_addrs {
        push_name(&mut names, addr.ip());
    }
    Identity::self_signed(&names).map_err(|error| ShareQuicError::Tls(error.to_string()))
}

fn push_name(names: &mut Vec<String>, ip: IpAddr) {
    let name = ip.to_string();
    if !names.contains(&name) {
        names.push(name);
    }
}

fn page_tls_config(identity: &Identity) -> Result<rustls::ServerConfig, ShareQuicError> {
    let chain: Vec<CertificateDer<'static>> = identity
        .certificate_chain()
        .as_slice()
        .iter()
        .map(|cert| CertificateDer::from(cert.der().to_vec()))
        .collect();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        identity.private_key().secret_der().to_vec(),
    ));
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| ShareQuicError::Tls(error.to_string()))?
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .map_err(|error| ShareQuicError::Tls(error.to_string()))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

async fn accept_loop(
    endpoint: Arc<wtransport::Endpoint<endpoint_side::Server>>,
    token: SecretString,
    publisher: broadcast::Sender<Vec<u8>>,
) {
    loop {
        let incoming = endpoint.accept().await;
        let token = token.clone();
        let publisher = publisher.clone();
        tokio::spawn(async move {
            let Ok(request) = incoming.await else {
                debug!("share quic: handshake failed");
                return;
            };
            let supplied = percent_decode(query_value(request.path(), "token").unwrap_or(""));
            if !constant_time_eq(supplied.as_bytes(), token.expose_secret().as_bytes()) {
                debug!("share quic: token rejected");
                request.forbidden().await;
                return;
            }
            let Ok(connection) = request.accept().await else {
                debug!("share quic: session rejected");
                return;
            };
            serve_viewer(connection, publisher).await;
        });
    }
}

async fn serve_viewer(connection: wtransport::Connection, publisher: broadcast::Sender<Vec<u8>>) {
    let Ok(opening) = connection.open_uni().await else {
        debug!("share quic: viewer accepted no stream");
        return;
    };
    let Ok(mut send) = opening.await else {
        debug!("share quic: viewer stream did not open");
        return;
    };
    let mut receiver = publisher.subscribe();
    loop {
        tokio::select! {
            _ = connection.closed() => break,
            incoming = receiver.recv() => match incoming {
                Ok(bytes) => {
                    if send.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    debug!(skipped, "share quic: viewer lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    let _ = send.finish().await;
    connection.close(wtransport::VarInt::from_u32(0), b"done");
}

/// Serves the static page and its assets over one TLS connection.
async fn page_loop(listener: TcpListener, acceptor: TlsAcceptor) {
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            continue;
        };
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let Ok(mut stream) = acceptor.accept(stream).await else {
                debug!(%peer, "share quic: page TLS handshake failed");
                return;
            };
            if serve_page(&mut stream).await.is_err() {
                debug!(%peer, "share quic: page request failed");
            }
        });
    }
}

async fn serve_page(stream: &mut tokio_rustls::server::TlsStream<TcpStream>) -> io::Result<()> {
    let request = read_request(stream).await?;
    let request_line = request.lines().next().unwrap_or("");
    let mut words = request_line.split_whitespace();
    let method = words.next().unwrap_or("");
    let target = words.next().unwrap_or("");
    let path = target.split('?').next().unwrap_or(target);
    let response = if method == "GET" {
        match path {
            "/" | "/index.html" => {
                write_asset(stream, "text/html; charset=utf-8", INDEX_HTML).await
            }
            "/xterm.js" => write_asset(stream, "text/javascript; charset=utf-8", XTERM_JS).await,
            "/xterm.css" => write_asset(stream, "text/css; charset=utf-8", XTERM_CSS).await,
            _ => write_status(stream, 404, "Not Found").await,
        }
    } else {
        write_status(stream, 404, "Not Found").await
    };
    // Sends `close_notify`; a browser ignores its absence, a TLS client does not.
    let _ = stream.shutdown().await;
    response
}

async fn read_request(
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
) -> io::Result<String> {
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= MAX_REQUEST {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn write_asset(
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: public, max-age=86400\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write_response(stream, &head, body.as_bytes()).await
}

async fn write_status(
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    code: u16,
    reason: &str,
) -> io::Result<()> {
    let head =
        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    write_response(stream, &head, b"").await
}

async fn write_response(
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    head: &str,
    body: &[u8],
) -> io::Result<()> {
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

fn query_value<'a>(target: &'a str, key: &str) -> Option<&'a str> {
    let query = target.split_once('?').map_or("", |(_, query)| query);
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then_some(v)
    })
}

/// Discovers public UDP endpoints and the first STUN address that answered.
async fn discover_external(socket: &UdpSocket) -> (Vec<SocketAddr>, Option<SocketAddr>) {
    let local_is_v4 = socket.local_addr().map(|a| a.is_ipv4()).unwrap_or(true);
    let mut found = Vec::new();
    let mut first_server = None;
    for server in STUN_SERVERS {
        let Ok(resolved) = tokio::net::lookup_host(*server).await else {
            continue;
        };
        let Some(server_addr) = resolved
            .into_iter()
            .find(|addr| addr.is_ipv4() == local_is_v4)
        else {
            continue;
        };
        let Some(transaction) = random_transaction() else {
            continue;
        };
        let request = binding_request(&transaction);
        if socket.send_to(&request, server_addr).await.is_err() {
            continue;
        }
        let deadline = tokio::time::Instant::now() + STUN_TIMEOUT;
        let mut buf = [0u8; 1500];
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
                Ok(Ok((len, _))) => {
                    if let Some(mapped) = parse_stun(&buf[..len], &transaction) {
                        // Keepalive must refresh the mapping of the server
                        // that answered; a silent server would refresh nothing.
                        first_server.get_or_insert(server_addr);
                        if !found.contains(&mapped) {
                            found.push(mapped);
                        }
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
    }
    (found, first_server)
}

fn binding_request(transaction: &[u8; 12]) -> [u8; 20] {
    let mut request = [0u8; 20];
    request[..2].copy_from_slice(&0x0001u16.to_be_bytes());
    request[4..8].copy_from_slice(&STUN_MAGIC.to_be_bytes());
    request[8..].copy_from_slice(transaction);
    request
}

fn random_transaction() -> Option<[u8; 12]> {
    use ring::rand::{SecureRandom, SystemRandom};

    let mut transaction = [0u8; 12];
    SystemRandom::new().fill(&mut transaction).ok()?;
    Some(transaction)
}

fn parse_stun(packet: &[u8], transaction: &[u8; 12]) -> Option<SocketAddr> {
    if packet.len() < 20
        || u16::from_be_bytes([packet[0], packet[1]]) != 0x0101
        || u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]) != STUN_MAGIC
        || packet[8..20] != *transaction
    {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    let end = (20 + length).min(packet.len());
    let mut xor_mapped = None;
    let mut mapped = None;
    let mut pos = 20;
    while pos + 4 <= end {
        let kind = u16::from_be_bytes([packet[pos], packet[pos + 1]]);
        let len = usize::from(u16::from_be_bytes([packet[pos + 2], packet[pos + 3]]));
        let start = pos + 4;
        let stop = start + len;
        if stop > end {
            break;
        }
        match kind {
            0x0020 => xor_mapped = parse_xor_mapped(&packet[start..stop], transaction),
            0x0001 => mapped = parse_mapped(&packet[start..stop]),
            _ => {}
        }
        pos = stop + ((4 - (len % 4)) % 4);
    }
    xor_mapped.or(mapped)
}

fn parse_xor_mapped(value: &[u8], transaction: &[u8; 12]) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let port = u16::from_be_bytes([value[2], value[3]]) ^ 0x2112;
    match value[1] {
        0x01 => {
            let cookie = STUN_MAGIC.to_be_bytes();
            let ip = [
                value[4] ^ cookie[0],
                value[5] ^ cookie[1],
                value[6] ^ cookie[2],
                value[7] ^ cookie[3],
            ];
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port))
        }
        0x02 if value.len() >= 20 => {
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&STUN_MAGIC.to_be_bytes());
            key[4..].copy_from_slice(transaction);
            let mut ip = [0u8; 16];
            for (index, byte) in ip.iter_mut().enumerate() {
                *byte = value[4 + index] ^ key[index];
            }
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip)), port))
        }
        _ => None,
    }
}

fn parse_mapped(value: &[u8]) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let port = u16::from_be_bytes([value[2], value[3]]);
    match value[1] {
        0x01 => Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(value[4], value[5], value[6], value[7])),
            port,
        )),
        0x02 if value.len() >= 20 => {
            let mut ip = [0u8; 16];
            ip.copy_from_slice(&value[4..20]);
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip)), port))
        }
        _ => None,
    }
}

/// Refreshes the UDP mapping while no viewer has connected yet.
///
/// The QUIC endpoint owns the socket, so this thread sends binding requests
/// through a duplicate handle; replies land on the QUIC receive path and are
/// ignored as not being QUIC. Only the outbound packet matters: it resets the
/// NAT timer.
struct Keepalive {
    stop: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Keepalive {
    fn start(socket: std::net::UdpSocket, server: SocketAddr) -> Self {
        let (stop, wait) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut seconds = 0u32;
            // The timeout doubles as the tick: a stop message wakes it at once.
            while wait.recv_timeout(Duration::from_secs(1))
                == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                seconds += 1;
                if !seconds.is_multiple_of(15) {
                    continue;
                }
                if let Some(transaction) = random_transaction() {
                    let _ = socket.send_to(&binding_request(&transaction), server);
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Keepalive {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Formats an IP for the authority part of a URL.
fn url_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(_) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

fn percent_encode_token(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stun_response(transaction: &[u8; 12], mapped: SocketAddr) -> Vec<u8> {
        let family = if mapped.is_ipv4() { 0x01 } else { 0x02 };
        let mut attr = vec![0u8, 0x20, 0, 0, 0x00, family];
        attr.extend_from_slice(&(mapped.port() ^ 0x2112).to_be_bytes());
        let mut key = [0u8; 16];
        key[..4].copy_from_slice(&STUN_MAGIC.to_be_bytes());
        key[4..].copy_from_slice(transaction);
        match mapped.ip() {
            IpAddr::V4(ip) => {
                for (index, byte) in ip.octets().iter().enumerate() {
                    attr.push(byte ^ key[index]);
                }
            }
            IpAddr::V6(ip) => {
                for (index, byte) in ip.octets().iter().enumerate() {
                    attr.push(byte ^ key[index]);
                }
            }
        }
        let value_len = (attr.len() - 4) as u16;
        attr[2..4].copy_from_slice(&value_len.to_be_bytes());
        while attr.len() % 4 != 0 {
            attr.push(0);
        }
        let mut packet = vec![0x01, 0x01];
        packet.extend_from_slice(&(attr.len() as u16).to_be_bytes());
        packet.extend_from_slice(&STUN_MAGIC.to_be_bytes());
        packet.extend_from_slice(transaction);
        packet.extend_from_slice(&attr);
        packet
    }

    #[test]
    fn parses_xor_mapped_ipv4_and_ipv6() {
        let transaction = [7u8; 12];
        for mapped in [
            SocketAddr::from(([203, 0, 113, 9], 44333)),
            SocketAddr::from(([0x2001, 0xdb8, 0, 1, 0, 0, 0, 9], 44333)),
        ] {
            let packet = stun_response(&transaction, mapped);
            assert_eq!(
                parse_stun(&packet, &transaction),
                Some(mapped),
                "packet={packet:02x?}"
            );
        }
    }

    #[test]
    fn rejects_a_stun_response_with_the_wrong_transaction() {
        let packet = stun_response(&[1u8; 12], SocketAddr::from(([1, 2, 3, 4], 5)));
        assert_eq!(parse_stun(&packet, &[2u8; 12]), None);
    }

    #[tokio::test]
    async fn link_carries_the_pin_and_the_webtransport_port() {
        let server = test_server().await;
        let target = SocketAddr::from(([203, 0, 113, 9], 4433));
        let link = server.viewer_link_for(target, 120, 40);
        assert!(link.starts_with("https://203.0.113.9:"));
        assert!(link.contains(&format!(":{}", server.page_addr().port())));
        assert!(link.contains("#token=test-token"));
        assert!(link.contains(&format!(
            "&fp={}",
            data_encoding::HEXLOWER.encode(&server.fingerprint)
        )));
        assert!(link.contains("&wt=4433"));
        assert!(link.ends_with("&cols=120&rows=40"));
    }

    #[test]
    fn link_parses_what_the_server_writes() {
        let link = ShareLink::parse(
            "https://203.0.113.9:51988/#token=abc-123&fp=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f&wt=52000&cols=120&rows=40",
        )
        .unwrap();
        assert_eq!(link.host, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)));
        assert_eq!(link.wt_port, 52000);
        assert_eq!(link.token.expose_secret(), "abc-123");
        assert_eq!(link.fingerprint[31], 0x1f);
        assert_eq!((link.cols, link.rows), (120, 40));
        let debug = format!("{link:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("abc-123"));
    }

    #[test]
    fn link_defaults_the_webtransport_port_and_brackets_ipv6() {
        let link = ShareLink::parse(
            "https://[2001:db8::9]:4433/#token=x&fp=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f&cols=1&rows=9999",
        )
        .unwrap();
        assert_eq!(
            link.host,
            IpAddr::V6(Ipv6Addr::from([0x2001, 0xdb8, 0, 0, 0, 0, 0, 9]))
        );
        assert_eq!(link.wt_port, 4433);
        assert_eq!((link.cols, link.rows), (20, 300));
    }

    #[test]
    fn link_rejects_a_missing_fingerprint() {
        assert!(ShareLink::parse("https://127.0.0.1:1/#token=x").is_err());
    }

    #[tokio::test]
    async fn link_omits_a_webtransport_port_equal_to_the_page_port() {
        let server = test_server().await;
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), server.page_addr().port());
        let link = server.viewer_link_for(target, 20, 300);
        assert!(!link.contains("&wt="));
        assert!(link.contains("&cols=20&rows=300"));
    }

    #[tokio::test]
    async fn link_clamps_the_terminal_size_and_brackets_ipv6() {
        let server = test_server().await;
        let target = SocketAddr::from(([0x2001, 0xdb8, 0, 0, 0, 0, 0, 9], 1));
        let link = server.viewer_link_for(target, 1, 9999);
        assert!(link.starts_with("https://[2001:db8::9]:"));
        assert!(link.ends_with("&cols=20&rows=300"));
    }

    #[tokio::test]
    async fn debug_redacts_the_token() {
        let server = test_server().await;
        let debug = format!("{server:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("test-token"));
    }

    /// Server on loopback with STUN skipped, so tests stay offline.
    async fn test_server() -> QuicShareServer {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        start_on_socket(socket, SecretString::from("test-token"), Vec::new(), None)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn page_serves_the_webtransport_client() {
        let server = test_server().await;
        let body = fetch_page(&server).await;
        assert!(body.starts_with("HTTP/1.1 200 OK"));
        assert!(body.contains("new WebTransport"));
        assert!(body.contains("serverCertificateHashes"));
    }

    /// Fetches `/` from the page listener, accepting the share's self-signed
    /// certificate the way a user who clicked through the browser warning does.
    async fn fetch_page(server: &QuicShareServer) -> String {
        use tokio_rustls::rustls::client::danger::{
            HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
        };
        use tokio_rustls::rustls::crypto::ring;
        use tokio_rustls::rustls::pki_types::{ServerName, UnixTime};
        use tokio_rustls::rustls::{DigitallySignedStruct, SignatureScheme};

        #[derive(Debug)]
        struct NoVerify;

        impl ServerCertVerifier for NoVerify {
            fn verify_server_cert(
                &self,
                _end_entity: &CertificateDer<'_>,
                _intermediates: &[CertificateDer<'_>],
                _server_name: &ServerName<'_>,
                _ocsp_response: &[u8],
                _now: UnixTime,
            ) -> Result<ServerCertVerified, rustls::Error> {
                Ok(ServerCertVerified::assertion())
            }

            fn verify_tls12_signature(
                &self,
                message: &[u8],
                cert: &CertificateDer<'_>,
                dss: &DigitallySignedStruct,
            ) -> Result<HandshakeSignatureValid, rustls::Error> {
                tokio_rustls::rustls::crypto::verify_tls12_signature(
                    message,
                    cert,
                    dss,
                    &ring::default_provider().signature_verification_algorithms,
                )
            }

            fn verify_tls13_signature(
                &self,
                message: &[u8],
                cert: &CertificateDer<'_>,
                dss: &DigitallySignedStruct,
            ) -> Result<HandshakeSignatureValid, rustls::Error> {
                tokio_rustls::rustls::crypto::verify_tls13_signature(
                    message,
                    cert,
                    dss,
                    &ring::default_provider().signature_verification_algorithms,
                )
            }

            fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
                ring::default_provider()
                    .signature_verification_algorithms
                    .supported_schemes()
            }
        }

        let config = tokio_rustls::rustls::ClientConfig::builder_with_provider(Arc::new(
            ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let tcp = TcpStream::connect(server.page_addr()).await.unwrap();
        let name = ServerName::try_from("localhost").unwrap();
        let mut stream = connector.connect(name, tcp).await.unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut body = String::new();
        stream.read_to_string(&mut body).await.unwrap();
        body
    }

    #[tokio::test]
    async fn native_client_receives_published_bytes() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let server = start_on_socket(socket, SecretString::from("viewer-token"), Vec::new(), None)
            .await
            .unwrap();
        let link = ShareLink::parse(&server.viewer_link_for(server.udp_addr(), 80, 24)).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let client = QuicShareClient::connect(&link, move |bytes| {
            let _ = tx.send(bytes);
        })
        .await
        .unwrap();
        let publisher = server.publisher();
        tokio::time::timeout(Duration::from_secs(2), async {
            while publisher.receiver_count() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        publisher.send(b"hello".to_vec()).unwrap();
        let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, b"hello");
        client.stop();
        server.stop();
    }

    #[tokio::test]
    async fn viewer_with_a_wrong_token_is_rejected() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let server = start_on_socket(socket, SecretString::from("right"), Vec::new(), None)
            .await
            .unwrap();
        let config = wtransport::ClientConfig::builder()
            .with_bind_default()
            .with_server_certificate_hashes([wtransport::tls::Sha256Digest::from(
                server.fingerprint,
            )])
            .build();
        let endpoint = wtransport::Endpoint::client(config).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            endpoint.connect(format!(
                "https://127.0.0.1:{}/?token=wrong",
                server.udp_addr().port()
            )),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        server.stop();
    }
}
