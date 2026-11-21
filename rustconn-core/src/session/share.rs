//! Read-only live session sharing over HTTP/SSE.
//!
//! `ShareServer` listens on a local address and streams raw terminal bytes
//! published by the GUI to browsers that present a matching token. There is no
//! upload path: viewers can only read.

use std::fmt;
use std::fmt::Write as _;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, broadcast, watch};
use tokio::task::JoinHandle;

/// Errors returned by [`ShareServer::start`].
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    /// The TCP listener could not be bound or queried.
    #[error("share server I/O error: {0}")]
    Io(#[from] io::Error),
}

const MAX_REQUEST: usize = 16 * 1024;

/// Resource limits of a [`ShareServer`].
///
/// The defaults suit a LAN tool: a handful of viewers, and no connection may
/// sit idle. They stop idle or slow sockets (slowloris) from using up tasks and
/// file descriptors.
#[derive(Debug, Clone, Copy)]
pub struct ShareLimits {
    /// Open sockets at once, whatever they are doing. Extra ones are closed.
    pub max_connections: usize,
    /// Authenticated live viewers at once. Extra ones get `503`.
    pub max_viewers: usize,
    /// Time a client has to send its whole request.
    pub request_timeout: Duration,
    /// Time one write to a viewer may take before the viewer is dropped.
    pub write_timeout: Duration,
    /// Chunks kept for a slow viewer before it is told to resync.
    pub channel_capacity: usize,
}

impl Default for ShareLimits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_viewers: 8,
            request_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(15),
            channel_capacity: 64,
        }
    }
}

/// Counts live viewers; the guard gives the slot back when it drops.
pub(super) struct ViewerSlot(Arc<AtomicUsize>);

impl ViewerSlot {
    pub(super) fn acquire(count: &Arc<AtomicUsize>, max: usize) -> Option<Self> {
        count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < max).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(count)))
    }
}

impl Drop for ViewerSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Remembers whether the shared program is on the alternate screen.
///
/// A viewer that joins in the middle of `htop` or `vim` then starts there too. Without
/// it the viewer draws the full-screen program into its normal buffer, and the
/// program's exit sequence (`ESC [ ? 1049 l`) then has nothing to clear.
#[derive(Debug, Default)]
pub struct AltScreen {
    active: AtomicBool,
    tail: std::sync::Mutex<Vec<u8>>,
}
impl AltScreen {
    /// Sequences that switch the alternate screen, longest first.
    const MODES: [(&'static [u8], bool); 6] = [
        (b"\x1b[?1049h", true),
        (b"\x1b[?1049l", false),
        (b"\x1b[?1047h", true),
        (b"\x1b[?1047l", false),
        (b"\x1b[?47h", true),
        (b"\x1b[?47l", false),
    ];
    /// Scans one chunk of terminal output; the last switch in it wins. Call
    /// this for every chunk, also while nobody watches.
    pub fn observe(&self, bytes: &[u8]) {
        if !bytes.contains(&0x1b) && self.tail.lock().map_or(true, |t| t.is_empty()) {
            return;
        }
        let Ok(mut tail) = self.tail.lock() else {
            return;
        };
        let mut window = std::mem::take(&mut *tail);
        window.extend_from_slice(bytes);
        let mut state = None;
        for (at, _) in window.iter().enumerate().filter(|(_, b)| **b == 0x1b) {
            if let Some((_, on)) = Self::MODES
                .iter()
                .find(|(seq, _)| window[at..].starts_with(seq))
            {
                state = Some(*on);
            }
        }
        if let Some(on) = state {
            self.active.store(on, Ordering::Release);
        }
        // Keep the end of the chunk: a sequence may be split across two reads.
        let from = window.len().saturating_sub(7);
        if let Some(esc) = window[from..].iter().position(|b| *b == 0x1b) {
            tail.extend_from_slice(&window[from + esc..]);
        }
    }
    /// True while the program is on the alternate screen.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    /// What a joining viewer must receive first, if anything.
    pub(crate) fn join_prefix(&self) -> Option<&'static [u8]> {
        self.is_active().then_some(b"\x1b[?1049h".as_slice())
    }
}
/// What every connection task needs.
#[derive(Clone)]
struct Shared {
    token: SecretString,
    publisher: broadcast::Sender<Vec<u8>>,
    alt: Arc<AltScreen>,
    limits: ShareLimits,
    viewers: Arc<AtomicUsize>,
}

/// Static viewer page. The token is deliberately absent: the browser reads it
/// from the URL fragment (never sent to a server, not in `Referer`), removes it
/// from the address bar, and sends it to `/events` in an `Authorization` header.
const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>Live session</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/xterm.css">
<style>html,body{margin:0;height:100%;background:#111}#term{height:100vh;padding:4px;box-sizing:border-box;overflow:auto}</style>
</head><body><div id="term"></div><script src="/xterm.js"></script><script>
var params = new URLSearchParams(location.hash.slice(1));
var token = params.get('token') || '';
var cols = Math.min(Math.max(parseInt(params.get('cols'), 10) || 80, 20), 500);
var rows = Math.min(Math.max(parseInt(params.get('rows'), 10) || 24, 5), 300);
history.replaceState(null, '', location.pathname);
var term = new Terminal({cols: cols, rows: rows, scrollback: 2000, disableStdin: true,
  fontSize: 13, theme: {background: '#111111', foreground: '#eeeeee'}});
term.open(document.getElementById('term'));
function handle(block) {
  var kind = 'message', data = '';
  block.split('\n').forEach(function (line) {
    if (line.indexOf('event:') === 0) { kind = line.slice(6).trim(); }
    else if (line.indexOf('data:') === 0) { data += line.slice(5).trim(); }
  });
  if (kind === 'reset') {
    term.reset();
    term.write('[some output was skipped; the screen may be incomplete]\r\n');
    return;
  }
  try { term.write(Uint8Array.from(atob(data), function (c) { return c.charCodeAt(0); })); }
  catch (err) { }
}
fetch('/events', {headers: {'Authorization': 'Bearer ' + token}}).then(function (res) {
  if (!res.ok) { term.write('Cannot watch this session (' + res.status + ').\r\n'); return; }
  var reader = res.body.getReader(), decoder = new TextDecoder(), buf = '';
  function pump() {
    return reader.read().then(function (r) {
      if (r.done) { term.write('\r\n[session sharing ended]\r\n'); return; }
      buf += decoder.decode(r.value, {stream: true});
      var i;
      while ((i = buf.indexOf('\n\n')) >= 0) { handle(buf.slice(0, i)); buf = buf.slice(i + 2); }
      return pump();
    });
  }
  return pump();
}).catch(function () { term.write('\r\n[connection lost]\r\n'); });
</script></body></html>
"#;

/// Vendored xterm.js (MIT) — a real terminal emulator, so colours, cursor
/// addressing and full-screen programs like `atop` render instead of leaking
/// escape sequences into a `<pre>`.
pub(crate) const XTERM_JS: &str = include_str!("share_assets/xterm.min.js");
pub(crate) const XTERM_CSS: &str = include_str!("share_assets/xterm.css");

/// A read-only live session sharing server.
///
/// Dropping the handle does not stop the server; call [`ShareServer::stop`].
pub struct ShareServer {
    bind: SocketAddr,
    token: SecretString,
    publisher: broadcast::Sender<Vec<u8>>,
    alt: Arc<AltScreen>,
    shutdown_tx: watch::Sender<bool>,
    accept_task: JoinHandle<()>,
}

impl fmt::Debug for ShareServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShareServer")
            .field("bind", &self.bind)
            .field("token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl ShareServer {
    /// Binds `bind` and spawns the accept loop.
    ///
    /// # Errors
    ///
    /// Returns [`ShareError::Io`] if the listener cannot be bound.
    pub async fn start(bind: SocketAddr, token: SecretString) -> Result<Self, ShareError> {
        Self::start_with_limits(bind, token, ShareLimits::default()).await
    }

    /// Like [`ShareServer::start`] with explicit [`ShareLimits`].
    ///
    /// # Errors
    ///
    /// Returns [`ShareError::Io`] if the listener cannot be bound.
    pub async fn start_with_limits(
        bind: SocketAddr,
        token: SecretString,
        limits: ShareLimits,
    ) -> Result<Self, ShareError> {
        let listener = TcpListener::bind(bind).await?;
        let bound = listener.local_addr()?;
        let (publisher, _) = broadcast::channel(limits.channel_capacity.max(1));
        let alt = Arc::new(AltScreen::default());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shared = Shared {
            token: token.clone(),
            publisher: publisher.clone(),
            alt: Arc::clone(&alt),
            limits,
            viewers: Arc::new(AtomicUsize::new(0)),
        };
        let accept_task = tokio::spawn(accept_loop(listener, shared, shutdown_rx));
        tracing::info!(%bound, "share: server started");
        Ok(Self {
            bind: bound,
            token,
            publisher,
            alt,
            shutdown_tx,
            accept_task,
        })
    }

    /// Returns a sender the GUI uses to publish raw terminal output bytes.
    ///
    /// `receiver_count()` is the number of live viewers; with none, publishing
    /// can be skipped.
    pub fn publisher(&self) -> broadcast::Sender<Vec<u8>> {
        self.publisher.clone()
    }
    /// The alternate-screen tracker the GUI feeds with every output chunk.
    pub fn alt_screen(&self) -> Arc<AltScreen> {
        Arc::clone(&self.alt)
    }

    /// Returns the viewer URL, including the token. For the user only: never log it.
    ///
    /// `host` is what the viewer should connect to; a server bound to
    /// `0.0.0.0` has no usable address of its own.
    #[must_use]
    pub fn local_url_for(&self, host: IpAddr) -> String {
        // IPv6 literals need brackets in a URL authority.
        let host = match host {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        format!(
            "http://{host}:{}/#token={}",
            self.bind.port(),
            percent_encode_token(self.token.expose_secret())
        )
    }

    /// Returns the viewer URL for the bind address (loopback by default).
    #[must_use]
    pub fn local_url(&self) -> String {
        self.local_url_for(self.bind.ip())
    }

    /// Stops accepting connections and waits for the accept loop to exit.
    pub async fn stop(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.accept_task.await;
        tracing::debug!("share: server stopped");
    }
}

async fn accept_loop(listener: TcpListener, shared: Shared, mut shutdown: watch::Receiver<bool>) {
    let slots = Arc::new(Semaphore::new(shared.limits.max_connections));
    loop {
        tokio::select! {
            incoming = listener.accept() => {
                match incoming {
                    Ok((stream, peer)) => {
                        let Ok(permit) = Arc::clone(&slots).try_acquire_owned() else {
                            tracing::debug!(%peer, "share: connection limit reached");
                            continue; // dropping the stream closes it
                        };
                        tracing::debug!(%peer, "share: viewer connected");
                        let (shared, shutdown) = (shared.clone(), shutdown.clone());
                        tokio::spawn(async move {
                            serve_connection(stream, shared, shutdown).await;
                            drop(permit);
                        });
                    }
                    Err(err) => tracing::debug!(%err, "share: accept failed"),
                }
            }
            _ = shutdown.changed() => break,
        }
    }
}

async fn serve_connection(
    mut stream: TcpStream,
    shared: Shared,
    mut shutdown: watch::Receiver<bool>,
) {
    // Stop and the request deadline both end the wait, so neither an idle socket
    // nor a stop during connect can keep the task alive.
    let request = tokio::select! {
        read = tokio::time::timeout(shared.limits.request_timeout, read_request(&mut stream)) => {
            match read {
                Ok(Ok(request)) => request,
                Ok(Err(err)) => {
                    tracing::debug!(%err, "share: could not read request");
                    return;
                }
                Err(_) => {
                    tracing::debug!("share: request timed out");
                    return;
                }
            }
        }
        _ = shutdown.changed() => return,
    };
    let request_line = request.lines().next().unwrap_or("");
    let mut words = request_line.split_whitespace();
    let method = words.next().unwrap_or("");
    let target = words.next().unwrap_or("");
    let path = target.split_once('?').map_or(target, |(path, _)| path);

    if method != "GET" {
        let _ = write_status(&mut stream, 404, "Not Found").await;
        return;
    }
    match path {
        "/" => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                INDEX_HTML.len()
            );
            let _ = write_response(&mut stream, &head, INDEX_HTML.as_bytes()).await;
        }
        "/xterm.js" => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nCache-Control: public, max-age=86400\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                XTERM_JS.len()
            );
            let _ = write_response(&mut stream, &head, XTERM_JS.as_bytes()).await;
        }
        "/xterm.css" => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/css; charset=utf-8\r\nCache-Control: public, max-age=86400\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                XTERM_CSS.len()
            );
            let _ = write_response(&mut stream, &head, XTERM_CSS.as_bytes()).await;
        }
        "/events" => {
            let supplied = zeroize::Zeroizing::new(bearer_token(&request).unwrap_or_default());
            // Constant-time compare: fold XOR over every padded byte slot and the
            // length difference too, so no mismatch short-circuits the loop.
            let valid =
                constant_time_eq(supplied.as_bytes(), shared.token.expose_secret().as_bytes());
            if !valid {
                let _ = write_status(&mut stream, 403, "Forbidden").await;
                return;
            }
            let Some(_slot) = ViewerSlot::acquire(&shared.viewers, shared.limits.max_viewers)
            else {
                let _ = write_status(&mut stream, 503, "Too Many Viewers").await;
                return;
            };
            let _ = serve_events(&mut stream, &shared, &mut shutdown).await;
        }
        _ => {
            let _ = write_status(&mut stream, 404, "Not Found").await;
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> io::Result<String> {
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

async fn write_response(stream: &mut TcpStream, head: &str, body: &[u8]) -> io::Result<()> {
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

async fn write_status(stream: &mut TcpStream, code: u16, reason: &str) -> io::Result<()> {
    let head =
        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    write_response(stream, &head, b"").await
}

fn sse_frame(bytes: &[u8]) -> String {
    // data-encoding is already a direct dependency; BASE64 is line-free, so one
    // `data:` line per frame is enough.
    format!("data: {}\n\n", data_encoding::BASE64.encode(bytes))
}

/// Writes one chunk, giving up on a viewer that does not read.
async fn write_timed(stream: &mut TcpStream, bytes: &[u8], limit: Duration) -> io::Result<()> {
    let write = async {
        stream.write_all(bytes).await?;
        stream.flush().await
    };
    tokio::time::timeout(limit, write)
        .await
        .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
}

/// Tells the page to clear its terminal: chunks were skipped, so the parser may
/// be inside an escape sequence and the screen no longer matches the session.
const RESET_FRAME: &str = "event: reset\ndata: lagged\n\n";

async fn serve_events(
    stream: &mut TcpStream,
    shared: &Shared,
    shutdown: &mut watch::Receiver<bool>,
) -> io::Result<()> {
    if *shutdown.borrow() {
        return Ok(());
    }
    // Subscribe before the head goes out, so nothing published after the
    // viewer saw "200" can be missed.
    let mut rx = shared.publisher.subscribe();
    let limit = shared.limits.write_timeout;
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    write_timed(stream, head.as_bytes(), limit).await?;
    if let Some(prefix) = shared.alt.join_prefix() {
        write_timed(stream, sse_frame(prefix).as_bytes(), limit).await?;
    }
    let mut sink = [0u8; 256];
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            // A viewer sends nothing after its request. Reading notices a
            // closed socket at once, so its slot is freed even if the
            // session is quiet.
            gone = stream.read(&mut sink) => {
                if !matches!(gone, Ok(n) if n > 0) {
                    break;
                }
            }
            incoming = rx.recv() => {
                match incoming {
                    Ok(bytes) if !bytes.is_empty() => {
                        write_timed(stream, sse_frame(&bytes).as_bytes(), limit).await?;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "share: viewer lagged, asking it to resync");
                        write_timed(stream, RESET_FRAME.as_bytes(), limit).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    Ok(())
}

/// The token from an `Authorization: Bearer` request header.
fn bearer_token(request: &str) -> Option<String> {
    request.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("authorization") {
            return None;
        }
        let value = value.trim();
        let (scheme, token) = value.split_once(' ')?;
        scheme
            .eq_ignore_ascii_case("bearer")
            .then(|| token.trim().to_owned())
    })
}

// Constant-time compare: fold byte XOR across every slot, never short-circuiting.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let diff = (0..n).fold(a.len() ^ b.len(), |acc, i| {
        acc | (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize
    });
    diff == 0
}

pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(hi), Some(lo)) => {
                if let (Some(h), Some(l)) = (hex(*hi), hex(*lo)) {
                    out.push(h << 4 | l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            (b'+', _, _) => {
                out.push(b' ');
                i += 1;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode_token(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_screen_follows_the_last_switch_even_when_split() {
        let alt = AltScreen::default();
        assert!(alt.join_prefix().is_none());
        alt.observe(b"prompt\x1b[?10");
        alt.observe(b"49h\x1b[Hhtop");
        assert!(alt.is_active());
        assert_eq!(alt.join_prefix(), Some(b"\x1b[?1049h".as_slice()));
        alt.observe(b"\x1b[?1049l$ ");
        assert!(!alt.is_active());
        alt.observe(b"\x1b[?1049l\x1b[?1049h");
        assert!(alt.is_active());
    }
    #[test]
    fn sse_frames_carry_base64_payloads() {
        assert_eq!(sse_frame(b""), "data: \n\n");
        assert_eq!(sse_frame(b"foo"), "data: Zm9v\n\n");
        assert_eq!(sse_frame(b"hello"), "data: aGVsbG8=\n\n");
        assert_eq!(sse_frame(&[0xff, 0x00]), "data: /wA=\n\n");
    }

    #[test]
    fn constant_time_compare_accepts_only_exact_bytes() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secrex"));
        assert!(!constant_time_eq(b"secret", b"secret-extra"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn token_url_encoding_round_trips() {
        assert_eq!(percent_decode("a%20b+c%2Fd"), "a b c/d");
        assert_eq!(percent_decode(&percent_encode_token("a&b+c/d")), "a&b+c/d");
    }

    #[tokio::test]
    async fn local_url_names_the_host_a_viewer_should_use() {
        let server = ShareServer::start(
            "0.0.0.0:0".parse().unwrap(),
            SecretString::from("token-xyz"),
        )
        .await
        .unwrap();
        let port = server.bind.port();
        let lan = std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 20, 30, 40));
        assert_eq!(
            server.local_url_for(lan),
            format!("http://10.20.30.40:{port}/#token=token-xyz")
        );
        let loopback6 = std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);
        assert_eq!(
            server.local_url_for(loopback6),
            format!("http://[::1]:{port}/#token=token-xyz")
        );
        server.stop().await;
    }

    #[tokio::test]
    async fn debug_output_redacts_token() {
        let secret = "super-secret-token-xyz";
        let server = ShareServer::start("127.0.0.1:0".parse().unwrap(), SecretString::from(secret))
            .await
            .unwrap();
        let debug = format!("{server:?}");
        assert!(!debug.contains(secret), "{debug}");
        assert!(debug.contains("REDACTED"), "{debug}");
        server.stop().await;
    }

    #[tokio::test]
    async fn bad_token_is_rejected_with_403() {
        let server = ShareServer::start(
            "127.0.0.1:0".parse().unwrap(),
            SecretString::from("right-token"),
        )
        .await
        .unwrap();
        let mut stream = TcpStream::connect(server.bind).await.unwrap();
        stream
            .write_all(b"GET /events HTTP/1.1\r\nAuthorization: Bearer wrong-token\r\n\r\n")
            .await
            .unwrap();
        let mut buf = [0u8; 128];
        let n = stream.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert!(
            response.ends_with("\r\n\r\n"),
            "403 must have no body: {response}"
        );
        server.stop().await;
    }

    #[tokio::test]
    async fn index_page_has_no_token_and_uses_a_terminal_emulator() {
        let embedded = "token-must-not-be-in-html";
        let server =
            ShareServer::start("127.0.0.1:0".parse().unwrap(), SecretString::from(embedded))
                .await
                .unwrap();
        let mut stream = TcpStream::connect(server.bind).await.unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(!response.contains(embedded), "{response}");
        assert!(response.contains("Authorization"), "{response}");
        assert!(!response.contains("EventSource"), "{response}");
        assert!(response.contains("new Terminal"), "{response}");
        assert!(response.contains("/xterm.js"), "{response}");
        // Terminal bytes go to xterm.js, never into HTML.
        assert!(!response.contains("innerHTML"), "{response}");
        server.stop().await;
    }

    #[tokio::test]
    async fn vendored_terminal_assets_are_served() {
        let server = ShareServer::start(
            "127.0.0.1:0".parse().unwrap(),
            SecretString::from("viewer-token"),
        )
        .await
        .unwrap();
        for (path, needle) in [("/xterm.js", "Terminal"), ("/xterm.css", ".xterm")] {
            let mut stream = TcpStream::connect(server.bind).await.unwrap();
            stream
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            let response = String::from_utf8_lossy(&buf);
            assert!(
                response.starts_with("HTTP/1.1 200"),
                "{path}: {}",
                response.lines().next().unwrap_or("")
            );
            assert!(response.contains(needle), "{path}: missing {needle}");
        }
        server.stop().await;
    }

    #[tokio::test]
    async fn published_bytes_reach_a_viewer() {
        let server = ShareServer::start(
            "127.0.0.1:0".parse().unwrap(),
            SecretString::from("viewer-token"),
        )
        .await
        .unwrap();
        let publisher = server.publisher();
        let mut stream = TcpStream::connect(server.bind).await.unwrap();
        stream
            .write_all(b"GET /events HTTP/1.1\r\nAuthorization: Bearer viewer-token\r\n\r\n")
            .await
            .unwrap();
        let mut buf = [0u8; 512];
        let n = stream.read(&mut buf).await.unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
        publisher.send(b"hi".to_vec()).unwrap();
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&buf[..n]), "data: aGk=\n\n");
        server.stop().await;
    }

    const HEAD_LEN_PROBE: usize = 4096;

    fn limits() -> ShareLimits {
        ShareLimits {
            max_connections: 4,
            max_viewers: 2,
            request_timeout: Duration::from_millis(150),
            write_timeout: Duration::from_millis(500),
            channel_capacity: 2,
        }
    }

    async fn start_limited(limits: ShareLimits) -> ShareServer {
        ShareServer::start_with_limits(
            "127.0.0.1:0".parse().unwrap(),
            SecretString::from("viewer-token"),
            limits,
        )
        .await
        .unwrap()
    }

    /// Opens `/events` and returns the stream after its response head.
    async fn open_viewer(server: &ShareServer) -> TcpStream {
        let mut stream = TcpStream::connect(server.bind).await.unwrap();
        stream
            .write_all(b"GET /events HTTP/1.1\r\nAuthorization: Bearer viewer-token\r\n\r\n")
            .await
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        assert!(head.starts_with(b"HTTP/1.1 200"), "{head:?}");
        stream
    }

    async fn read_some(stream: &mut TcpStream) -> String {
        let mut buf = vec![0u8; HEAD_LEN_PROBE];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("timed out waiting for data")
            .unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    #[tokio::test]
    async fn idle_socket_is_closed_after_request_timeout() {
        let server = start_limited(limits()).await;
        let mut idle = TcpStream::connect(server.bind).await.unwrap();
        let mut buf = [0u8; 8];
        let n = tokio::time::timeout(Duration::from_secs(2), idle.read(&mut buf))
            .await
            .expect("server kept an idle socket open")
            .unwrap();
        assert_eq!(n, 0);
        server.stop().await;
    }

    #[tokio::test]
    async fn dripping_a_request_does_not_extend_the_deadline() {
        let server = start_limited(limits()).await;
        let mut slow = TcpStream::connect(server.bind).await.unwrap();
        let mut closed = false;
        for _ in 0..30 {
            if slow.write_all(b"X").await.is_err() {
                closed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let mut buf = [0u8; 8];
        let n = slow.read(&mut buf).await.unwrap_or(0);
        assert!(closed || n == 0);
        server.stop().await;
    }

    #[tokio::test]
    async fn many_idle_sockets_cannot_starve_a_real_viewer() {
        let server = start_limited(limits()).await;
        let idle: Vec<_> = connect_many(&server, 100).await;
        // Past the timeout every idle socket is gone, and a viewer gets in.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut viewer = open_viewer(&server).await;
        server.publisher().send(b"ok".to_vec()).unwrap();
        assert_eq!(read_some(&mut viewer).await, "data: b2s=\n\n");
        drop(idle);
        server.stop().await;
    }

    async fn connect_many(server: &ShareServer, count: usize) -> Vec<TcpStream> {
        let mut out = Vec::new();
        for _ in 0..count {
            if let Ok(stream) = TcpStream::connect(server.bind).await {
                out.push(stream);
            }
        }
        out
    }

    #[tokio::test]
    async fn connections_over_the_cap_are_closed() {
        let mut limits = limits();
        limits.request_timeout = Duration::from_secs(30);
        let server = start_limited(limits).await;
        let held = connect_many(&server, 4).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut extra = TcpStream::connect(server.bind).await.unwrap();
        let mut buf = [0u8; 8];
        let n = tokio::time::timeout(Duration::from_secs(2), extra.read(&mut buf))
            .await
            .expect("over-cap socket was kept open")
            .unwrap_or(0);
        assert_eq!(n, 0);
        drop(held);
        server.stop().await;
    }

    #[tokio::test]
    async fn viewers_over_the_cap_get_503_and_slots_come_back() {
        let server = start_limited(limits()).await;
        let first = open_viewer(&server).await;
        let _second = open_viewer(&server).await;
        let mut third = TcpStream::connect(server.bind).await.unwrap();
        third
            .write_all(b"GET /events HTTP/1.1\r\nAuthorization: Bearer viewer-token\r\n\r\n")
            .await
            .unwrap();
        assert!(read_some(&mut third).await.starts_with("HTTP/1.1 503"));
        // Reconnect: once a viewer leaves, its slot is free again.
        drop(first);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _again = open_viewer(&server).await;
        server.stop().await;
    }

    #[tokio::test]
    async fn lagged_viewer_is_told_to_reset_then_keeps_streaming() {
        let server = start_limited(limits()).await;
        let mut viewer = open_viewer(&server).await;
        let publisher = server.publisher();
        // Capacity is 2 and the current-thread runtime cannot run the viewer's
        // task during this loop, so it must lag.
        for chunk in [b"\x1b[3".as_slice(), b"1m", b"red", b"\x1b[0m", b"end"] {
            publisher.send(chunk.to_vec()).unwrap();
        }
        let mut seen = String::new();
        while !seen.contains("ZW5k") {
            seen.push_str(&read_some(&mut viewer).await);
        }
        let reset = seen.find("event: reset").expect("no reset frame");
        let first_data = seen.find("data: ").unwrap();
        assert!(reset <= first_data, "reset must precede stale data: {seen}");
        assert!(
            !seen.contains("Wzsx"),
            "skipped chunk was delivered: {seen}"
        );
        server.stop().await;
    }

    #[tokio::test]
    async fn no_viewers_means_no_receivers() {
        let server = start_limited(limits()).await;
        assert_eq!(server.publisher().receiver_count(), 0);
        let viewer = open_viewer(&server).await;
        assert_eq!(server.publisher().receiver_count(), 1);
        drop(viewer);
        server.stop().await;
    }

    #[tokio::test]
    async fn stop_closes_pending_and_live_connections() {
        let server = start_limited(limits()).await;
        let mut pending = TcpStream::connect(server.bind).await.unwrap();
        let mut viewer = open_viewer(&server).await;
        tokio::time::timeout(Duration::from_secs(2), server.stop())
            .await
            .expect("stop hung");
        let mut buf = [0u8; 8];
        for stream in [&mut pending, &mut viewer] {
            let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
                .await
                .expect("socket outlived stop")
                .unwrap_or(0);
            assert_eq!(n, 0);
        }
    }

    #[tokio::test]
    async fn token_in_the_query_string_is_not_accepted() {
        let server = start_limited(limits()).await;
        let mut stream = TcpStream::connect(server.bind).await.unwrap();
        stream
            .write_all(b"GET /events?token=viewer-token HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        assert!(read_some(&mut stream).await.starts_with("HTTP/1.1 403"));
        server.stop().await;
    }

    #[test]
    fn bearer_header_is_parsed_case_insensitively() {
        let req = "GET /events HTTP/1.1\r\nHost: x\r\nauthorization: bearer abc\r\n\r\n";
        assert_eq!(bearer_token(req).as_deref(), Some("abc"));
        assert_eq!(bearer_token("GET / HTTP/1.1\r\n\r\n"), None);
    }
}
