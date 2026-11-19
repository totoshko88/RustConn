//! Read-only live session sharing over HTTP/SSE.
//!
//! `ShareServer` listens on a local address and streams raw terminal bytes
//! published by the GUI to browsers that present a matching token. There is no
//! upload path: viewers can only read.

use std::fmt;
use std::fmt::Write as _;
use std::io;
use std::net::{IpAddr, SocketAddr};

use secrecy::{ExposeSecret, SecretString};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

/// Errors returned by [`ShareServer::start`].
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    /// The TCP listener could not be bound or queried.
    #[error("share server I/O error: {0}")]
    Io(#[from] io::Error),
}

const MAX_REQUEST: usize = 16 * 1024;
const CHANNEL_CAPACITY: usize = 64;

/// Static viewer page. The token is deliberately absent: the browser reads it
/// from its own URL query and forwards it to `/events`.
const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>Live session</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/xterm.css">
<style>html,body{margin:0;height:100%;background:#111}#term{height:100vh;padding:4px;box-sizing:border-box;overflow:auto}</style>
</head><body><div id="term"></div><script src="/xterm.js"></script><script>
var params = new URLSearchParams(location.search);
var token = params.get('token') || '';
var cols = Math.min(Math.max(parseInt(params.get('cols'), 10) || 80, 20), 500);
var rows = Math.min(Math.max(parseInt(params.get('rows'), 10) || 24, 5), 300);
var term = new Terminal({cols: cols, rows: rows, scrollback: 2000, disableStdin: true,
  fontSize: 13, theme: {background: '#111111', foreground: '#eeeeee'}});
term.open(document.getElementById('term'));
var src = new EventSource('/events?token=' + encodeURIComponent(token));
src.onmessage = function (e) {
  var bytes;
  try { bytes = Uint8Array.from(atob(e.data), function (c) { return c.charCodeAt(0); }); }
  catch (err) { return; }
  term.write(bytes);
};
</script></body></html>
"#;

/// Vendored xterm.js (MIT) — a real terminal emulator, so colours, cursor
/// addressing and full-screen programs like `atop` render instead of leaking
/// escape sequences into a `<pre>`.
const XTERM_JS: &str = include_str!("share_assets/xterm.min.js");
const XTERM_CSS: &str = include_str!("share_assets/xterm.css");

/// A read-only live session sharing server.
///
/// Dropping the handle does not stop the server; call [`ShareServer::stop`].
pub struct ShareServer {
    bind: SocketAddr,
    token: SecretString,
    publisher: broadcast::Sender<Vec<u8>>,
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
        let listener = TcpListener::bind(bind).await?;
        let bound = listener.local_addr()?;
        let (publisher, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let accept_task = tokio::spawn(accept_loop(
            listener,
            token.clone(),
            publisher.clone(),
            shutdown_rx,
        ));
        tracing::info!(%bound, "share: server started");
        Ok(Self {
            bind: bound,
            token,
            publisher,
            shutdown_tx,
            accept_task,
        })
    }

    /// Returns a sender the GUI uses to publish raw terminal output bytes.
    pub fn publisher(&self) -> broadcast::Sender<Vec<u8>> {
        self.publisher.clone()
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
            "http://{host}:{}/?token={}",
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

async fn accept_loop(
    listener: TcpListener,
    token: SecretString,
    publisher: broadcast::Sender<Vec<u8>>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            incoming = listener.accept() => {
                match incoming {
                    Ok((stream, peer)) => {
                        tracing::debug!(%peer, "share: viewer connected");
                        let (token, publisher, shutdown) =
                            (token.clone(), publisher.clone(), shutdown.clone());
                        tokio::spawn(async move {
                            serve_connection(stream, token, publisher, shutdown).await;
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
    token: SecretString,
    publisher: broadcast::Sender<Vec<u8>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(err) => {
            tracing::debug!(%err, "share: could not read request");
            return;
        }
    };
    let request_line = request.lines().next().unwrap_or("");
    let mut words = request_line.split_whitespace();
    let method = words.next().unwrap_or("");
    let target = words.next().unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

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
            let supplied = query_value(query, "token").unwrap_or_default();
            // Constant-time compare: fold XOR over every padded byte slot and the
            // length difference too, so no mismatch short-circuits the loop.
            let valid = constant_time_eq(supplied.as_bytes(), token.expose_secret().as_bytes());
            if !valid {
                let _ = write_status(&mut stream, 403, "Forbidden").await;
                return;
            }
            let _ = serve_events(&mut stream, publisher, &mut shutdown).await;
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

async fn serve_events(
    stream: &mut TcpStream,
    publisher: broadcast::Sender<Vec<u8>>,
    shutdown: &mut watch::Receiver<bool>,
) -> io::Result<()> {
    if *shutdown.borrow() {
        return Ok(());
    }
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(head.as_bytes()).await?;
    stream.flush().await?;
    let mut rx = publisher.subscribe();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            incoming = rx.recv() => {
                match incoming {
                    Ok(bytes) if !bytes.is_empty() => {
                        let frame = sse_frame(&bytes);
                        stream.write_all(frame.as_bytes()).await?;
                        stream.flush().await?;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    Ok(())
}

// Constant-time compare: fold byte XOR across every slot, never short-circuiting.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let diff = (0..n).fold(a.len() ^ b.len(), |acc, i| {
        acc | (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize
    });
    diff == 0
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(input: &str) -> String {
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
            format!("http://10.20.30.40:{port}/?token=token-xyz")
        );
        let loopback6 = std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);
        assert_eq!(
            server.local_url_for(loopback6),
            format!("http://[::1]:{port}/?token=token-xyz")
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
            .write_all(b"GET /events?token=wrong-token HTTP/1.1\r\nHost: localhost\r\n\r\n")
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
            .write_all(b"GET /?token=viewer-token HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(!response.contains(embedded), "{response}");
        assert!(response.contains("EventSource"), "{response}");
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
            .write_all(b"GET /events?token=viewer-token HTTP/1.1\r\nHost: localhost\r\n\r\n")
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
}
