//! Minimal read-only TFTP server (RFC 1350 RRQ, octet mode).
//!
//! Serves files from one root directory to network devices dumping config.
//! Uses tokio UDP only: no unsafe, no platform-specific APIs, no privileged
//! port binding (the caller chooses `bind`). One request is served at a time.

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::net::UdpSocket;

/// Default TFTP block size from RFC 1350.
const DEFAULT_BLKSIZE: usize = 512;
/// Number of retransmissions before abandoning a transfer.
const MAX_RETRIES: u8 = 3;
/// Retransmission timeout; RFC 1350 suggests one second.
const RETRY_TIMEOUT: Duration = Duration::from_secs(1);

/// Errors produced by the read-only TFTP server.
#[derive(Debug, Error)]
pub enum TftpError {
    /// The UDP socket could not be bound.
    #[error("failed to bind TFTP socket: {0}")]
    Bind(#[source] std::io::Error),
    /// The configured root could not be resolved.
    #[error("invalid TFTP root: {0}")]
    InvalidRoot(String),
    /// The incoming packet was not a valid RRQ.
    #[error("malformed TFTP request: {0}")]
    Malformed(String),
    /// The requested path failed a security check or escaped the root.
    #[error("TFTP path rejected: {0}")]
    RejectedPath(String),
    /// The requested file does not exist or is unreadable.
    #[error("TFTP file not found: {0}")]
    NotFound(String),
    /// An I/O error occurred while serving.
    #[error("TFTP I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A running read-only TFTP server.
pub struct TftpServer {
    local_addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl TftpServer {
    /// Binds a read-only TFTP server on `bind`, serving files under `root`.
    ///
    /// # Errors
    /// Returns [`TftpError::InvalidRoot`] if `root` cannot be canonicalized,
    /// [`TftpError::Bind`] if `bind` cannot be bound, and [`TftpError::Io`]
    /// if the local address cannot be read.
    pub async fn start(bind: SocketAddr, root: PathBuf) -> Result<Self, TftpError> {
        // Canonical root once so every later containment check compares like for like.
        let root = tokio::fs::canonicalize(&root)
            .await
            .map_err(|e| TftpError::InvalidRoot(e.to_string()))?;
        let socket = UdpSocket::bind(bind).await.map_err(TftpError::Bind)?;
        let local_addr = socket.local_addr()?;
        let task = tokio::spawn(serve(socket, root));
        Ok(Self { local_addr, task })
    }

    /// Returns the address the server actually listens on (useful for port 0).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops the server, cancelling any in-flight transfer.
    pub async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

struct ReadRequest {
    filename: String,
    blksize: Option<u16>,
    tsize: bool,
}

/// Reads one NUL-terminated field, advancing the cursor past its terminator.
fn field<'a>(packet: &'a [u8], pos: &mut usize) -> Result<&'a [u8], TftpError> {
    let rest = packet
        .get(*pos..)
        .ok_or_else(|| TftpError::Malformed("truncated packet".into()))?;
    let nul = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| TftpError::Malformed("unterminated field".into()))?;
    *pos += nul + 1;
    Ok(&rest[..nul])
}

/// Parses an RRQ packet: opcode, filename, `octet` mode and optional options.
fn parse_rrq(packet: &[u8]) -> Result<ReadRequest, TftpError> {
    let opcode = packet
        .get(..2)
        .ok_or_else(|| TftpError::Malformed("truncated opcode".into()))?;
    if opcode != [0, 1] {
        return Err(TftpError::Malformed("not a read request".into()));
    }
    let mut pos = 2;
    let filename = field(packet, &mut pos)?;
    let mode = field(packet, &mut pos)?;
    if !mode.eq_ignore_ascii_case(b"octet") {
        return Err(TftpError::Malformed("mode is not octet".into()));
    }
    let filename = std::str::from_utf8(filename)
        .map_err(|_| TftpError::Malformed("invalid filename encoding".into()))?;
    let mut blksize = None;
    let mut tsize = false;
    while pos < packet.len() {
        if packet[pos..].iter().all(|&b| b == 0) {
            break;
        }
        let key = field(packet, &mut pos)?;
        let value = field(packet, &mut pos)?;
        if key.eq_ignore_ascii_case(b"blksize") {
            if let Ok(s) = std::str::from_utf8(value)
                && let Ok(n) = s.parse::<u16>()
                && (8..=65464).contains(&n)
            {
                blksize = Some(n);
            }
        } else if key.eq_ignore_ascii_case(b"tsize") {
            tsize = true;
        }
    }
    Ok(ReadRequest {
        filename: filename.into(),
        blksize,
        tsize,
    })
}

/// Rejects any filename that is not a plain path inside the root.
///
/// A leading `/` is accepted and means the export root: RouterOS sends
/// `/probe.bin` for a file at the top of the TFTP directory.
fn validate_filename(filename: &str) -> Result<(), TftpError> {
    let reject = |why: &str| TftpError::RejectedPath(why.into());
    if filename.contains('\0') {
        return Err(reject("NUL byte"));
    }
    let mut names = 0usize;
    for component in Path::new(filename).components() {
        match component {
            Component::Normal(_) => names += 1,
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(reject("path escapes root"));
            }
        }
    }
    if names == 0 {
        return Err(reject("empty filename"));
    }
    Ok(())
}

/// Resolves a request to a canonical path, proving it lives under `root`.
async fn resolve_path(root: &Path, filename: &str) -> Result<PathBuf, TftpError> {
    validate_filename(filename)?;
    // join() would discard the root for a rooted path, so strip the prefix.
    let relative = filename.trim_start_matches('/');
    // canonicalize resolves symlinks, so a link pointing out of the root is caught.
    let canonical = tokio::fs::canonicalize(root.join(relative))
        .await
        .map_err(|_| TftpError::NotFound(filename.into()))?;
    if !canonical.starts_with(root) {
        return Err(TftpError::RejectedPath("symlink escapes root".into()));
    }
    Ok(canonical)
}

fn is_ack(packet: &[u8], block: u16) -> bool {
    packet.len() >= 4
        && packet[..2] == [0, 4]
        && u16::from_be_bytes([packet[2], packet[3]]) == block
}

/// Sends a packet until the peer ACKs `block`, retransmitting on timeout.
async fn send_until_ack(socket: &UdpSocket, peer: SocketAddr, packet: &[u8], block: u16) -> bool {
    let mut buf = [0u8; 64];
    for _ in 0..MAX_RETRIES {
        if socket.send_to(packet, peer).await.is_err() {
            return false;
        }
        let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
                Ok(Ok((len, src))) => {
                    if src == peer && is_ack(&buf[..len], block) {
                        return true;
                    }
                }
                Ok(Err(_)) => return false,
                Err(_) => break,
            }
        }
    }
    false
}

/// Sends an RFC 1350 ERROR packet (best effort).
async fn send_error(socket: &UdpSocket, peer: SocketAddr, code: u16, message: &str) {
    let mut packet = Vec::with_capacity(5 + message.len());
    packet.extend_from_slice(&7u16.to_be_bytes());
    packet.extend_from_slice(&code.to_be_bytes());
    packet.extend_from_slice(message.as_bytes());
    packet.push(0);
    if let Err(e) = socket.send_to(&packet, peer).await {
        tracing::debug!(error = %e, peer = %peer, "failed to send TFTP ERROR");
    }
}

/// Serves one RRQ: validate, send OACK if requested, then DATA blocks.
async fn handle(socket: &UdpSocket, root: &Path, packet: &[u8], peer: SocketAddr) {
    let req = match parse_rrq(packet) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, peer = %peer, "invalid TFTP request");
            send_error(socket, peer, 4, "illegal tftp operation").await;
            return;
        }
    };
    let path = match resolve_path(root, &req.filename).await {
        Ok(p) => p,
        Err(TftpError::NotFound(_)) => {
            send_error(socket, peer, 1, "file not found").await;
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, filename = %req.filename, "TFTP path rejected");
            send_error(socket, peer, 2, "access violation").await;
            return;
        }
    };
    let data = match tokio::fs::read(&path).await {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!(error = %e, "TFTP read failed");
            send_error(socket, peer, 1, "file not found").await;
            return;
        }
    };
    let blksize = req.blksize.unwrap_or(DEFAULT_BLKSIZE as u16) as usize;
    let blocks = data.len().div_ceil(blksize).max(1);
    if req.tsize || req.blksize.is_some() {
        let mut oack = vec![0, 6];
        if let Some(bs) = req.blksize {
            oack.extend_from_slice(b"blksize\0");
            oack.extend_from_slice(bs.to_string().as_bytes());
            oack.push(0);
        }
        if req.tsize {
            oack.extend_from_slice(b"tsize\0");
            oack.extend_from_slice(data.len().to_string().as_bytes());
            oack.push(0);
        }
        if !send_until_ack(socket, peer, &oack, 0).await {
            return;
        }
    }
    // shortcut: block numbers wrap after 65535 (~32 MiB at 512 B); config dumps are tiny
    for block in 1..=blocks {
        let start = (block - 1) * blksize;
        let end = (start + blksize).min(data.len());
        let mut packet = Vec::with_capacity(4 + end - start);
        packet.extend_from_slice(&3u16.to_be_bytes());
        packet.extend_from_slice(&(block as u16).to_be_bytes());
        packet.extend_from_slice(&data[start..end]);
        if !send_until_ack(socket, peer, &packet, block as u16).await {
            tracing::debug!(peer = %peer, block, "TFTP transfer abandoned");
            return;
        }
    }
}

/// Accept loop for a bound TFTP socket.
async fn serve(socket: UdpSocket, root: PathBuf) {
    let mut buf = [0u8; 4096]; // RRQ plus options fits comfortably
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, peer)) => handle(&socket, &root, &buf[..len], peer).await,
            Err(e) => {
                tracing::error!(error = %e, "TFTP receive failed, stopping");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rrq(filename: &[u8]) -> Vec<u8> {
        let mut packet = vec![0, 1];
        packet.extend_from_slice(filename);
        packet.extend_from_slice(b"\0octet\0");
        packet
    }

    #[test]
    fn parses_requests() {
        let plain = parse_rrq(&rrq(b"dump.cfg")).unwrap();
        assert_eq!(plain.filename, "dump.cfg");
        assert_eq!(plain.blksize, None);
        assert!(!plain.tsize);

        let mut packet = rrq(b"config.cfg");
        packet.extend_from_slice(b"blksize\x001024\x00tsize\x00\x00");
        let req = parse_rrq(&packet).unwrap();
        assert_eq!(req.filename, "config.cfg");
        assert_eq!(req.blksize, Some(1024));
        assert!(req.tsize);
    }

    #[test]
    fn parser_rejects_bad_requests() {
        assert!(parse_rrq(&[0, 2, 0]).is_err()); // WRQ, not RRQ
        assert!(parse_rrq(b"\x00\x01x\0netascii\0").is_err()); // wrong mode
        assert!(parse_rrq(&rrq(b"x\0y")).is_err()); // embedded NUL -> mode "y"
    }

    #[test]
    fn validates_filenames() {
        assert!(validate_filename("dump.cfg").is_ok());
        assert!(validate_filename("sub/dump.cfg").is_ok());
        assert!(validate_filename("/dump.cfg").is_ok()); // RouterOS sends a rooted path
        assert!(validate_filename("/").is_err());
        assert!(validate_filename("../etc/passwd").is_err());
        assert!(validate_filename("/../etc/passwd").is_err());
        assert!(validate_filename("a/../../b").is_err());
        assert!(validate_filename("bad\0name").is_err());
    }

    #[tokio::test]
    async fn resolve_accepts_rooted_request_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = tokio::fs::canonicalize(dir.path()).await.unwrap();
        std::fs::write(root.join("dump.cfg"), b"x").unwrap();
        let resolved = resolve_path(&root, "/dump.cfg").await.unwrap();
        assert_eq!(resolved, root.join("dump.cfg"));
    }

    #[tokio::test]
    async fn resolve_rejects_escape_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        assert!(resolve_path(&root, "../secret").await.is_err());
        assert!(resolve_path(&root, "/../etc/passwd").await.is_err());
        assert!(resolve_path(&root, "a/../../secret").await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resolve_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("root")).unwrap();
        let root = tokio::fs::canonicalize(dir.path().join("root"))
            .await
            .unwrap();
        std::fs::write(dir.path().join("secret"), b"secret").unwrap();
        symlink(dir.path().join("secret"), root.join("link")).unwrap();
        assert!(resolve_path(&root, "link").await.is_err());
    }

    #[tokio::test]
    async fn serves_small_file_in_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let payload = vec![b'x'; 1200]; // spans three 512-byte blocks
        std::fs::write(dir.path().join("dump.cfg"), &payload).unwrap();
        let server = TftpServer::start("127.0.0.1:0".parse().unwrap(), dir.path().into())
            .await
            .unwrap();
        let addr = server.local_addr();
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut request = rrq(b"/dump.cfg"); // RouterOS requests the rooted path
        request.extend_from_slice(b"blksize\x00512\x00tsize\x00\x00");
        client.send_to(&request, addr).await.unwrap();

        let mut buf = [0u8; 1024];
        let (_n, src) = client.recv_from(&mut buf).await.unwrap();
        assert_eq!(src, addr);
        assert_eq!(&buf[..2], &[0, 6]); // OACK
        client.send_to(&[0, 4, 0, 0], addr).await.unwrap(); // ACK block 0

        let mut received = Vec::new();
        let mut expected = 1u16;
        loop {
            let (n, _) = client.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..2], &[0, 3]); // DATA
            assert_eq!(
                u16::from_be_bytes([buf[2], buf[3]]),
                expected,
                "block numbers must be sequential"
            );
            let chunk = &buf[4..n];
            received.extend_from_slice(chunk);
            let ack = [0, 4, buf[2], buf[3]];
            client.send_to(&ack, addr).await.unwrap();
            if chunk.len() < DEFAULT_BLKSIZE {
                break;
            }
            expected += 1;
        }
        assert_eq!(received, payload);
        assert_eq!(expected, 3);
        server.stop().await;
    }
}
