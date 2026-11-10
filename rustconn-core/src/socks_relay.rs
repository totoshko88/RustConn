//! A local SOCKS5 relay that logs in to an upstream proxy.
//!
//! Browsers cannot be given a SOCKS username and password: WebKitGTK and Chromium
//! have no setting for them, and a command line would expose them anyway. So the
//! browser talks to this relay on loopback without a login, and the relay
//! performs the RFC 1929 login on each connection it opens upstream.
//!
//! Like `ssh -D`, the relay accepts anyone on this computer while it runs. The
//! password itself never leaves this process except to the proxy.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use zeroize::Zeroizing;

/// How long the upstream proxy may take to accept a connection or answer.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the accept loop checks whether the relay was stopped.
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// Connections carried at once; a browser opens tens, never thousands.
const MAX_CONNECTIONS: usize = 512;

const SOCKS_VERSION: u8 = 5;
const AUTH_NONE: u8 = 0;
const AUTH_PASSWORD: u8 = 2;
const AUTH_UNACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 1;
const ATYP_IPV4: u8 = 1;
const ATYP_DOMAIN: u8 = 3;
const ATYP_IPV6: u8 = 4;
const REPLY_GENERAL_FAILURE: u8 = 1;
const REPLY_COMMAND_UNSUPPORTED: u8 = 7;

/// Why a relay could not be started.
#[derive(Debug, Error)]
pub enum SocksRelayError {
    /// The username or password does not fit the SOCKS login format.
    #[error("the SOCKS username and password must each be 1 to 255 bytes")]
    InvalidLogin,
    /// The proxy address is not `socks5://host:port`.
    #[error("the SOCKS proxy address is not valid")]
    InvalidProxy,
    /// The local listener could not be opened.
    #[error("could not open the local relay: {0}")]
    Listen(#[from] io::Error),
}

/// A running relay. Dropping it stops the relay and closes new connections.
#[derive(Debug)]
pub struct SocksRelay {
    local_port: u16,
    stop: Arc<AtomicBool>,
}

struct Upstream {
    host: String,
    port: u16,
    username: String,
    password: SecretString,
}

impl SocksRelay {
    /// Starts a relay to `proxy` (`socks5://host:port`).
    ///
    /// # Errors
    ///
    /// Returns a [`SocksRelayError`] for an unusable login or address, or when
    /// the local listener cannot be opened.
    pub fn start(
        proxy: &str,
        username: &str,
        password: SecretString,
    ) -> Result<Self, SocksRelayError> {
        let valid = |len: usize| (1..=255).contains(&len);
        if !valid(username.len()) || !valid(password.expose_secret().len()) {
            return Err(SocksRelayError::InvalidLogin);
        }
        let (host, port) = proxy
            .strip_prefix("socks5://")
            .and_then(|address| address.rsplit_once(':'))
            .ok_or(SocksRelayError::InvalidProxy)?;
        let port = port.parse().map_err(|_| SocksRelayError::InvalidProxy)?;
        let upstream = Arc::new(Upstream {
            host: host.trim_matches(['[', ']']).to_string(),
            port,
            username: username.to_string(),
            password,
        });

        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let local_port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        std::thread::spawn(move || accept_loop(&listener, &upstream, &stopped));
        Ok(Self { local_port, stop })
    }

    /// The address to point a browser at.
    #[must_use]
    pub fn proxy_uri(&self) -> String {
        format!("socks5://127.0.0.1:{}", self.local_port)
    }
}

impl Drop for SocksRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn accept_loop(listener: &TcpListener, upstream: &Arc<Upstream>, stop: &AtomicBool) {
    let open = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((client, _)) => {
                if open.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue;
                }
                open.fetch_add(1, Ordering::Relaxed);
                let upstream = Arc::clone(upstream);
                let open = Arc::clone(&open);
                std::thread::spawn(move || {
                    if let Err(error) = relay_connection(client, &upstream) {
                        tracing::debug!(%error, "SOCKS relay connection ended");
                    }
                    open.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(error) => {
                tracing::warn!(%error, "SOCKS relay stopped accepting");
                break;
            }
        }
    }
}

fn reply_failure(client: &mut TcpStream, code: u8) -> io::Result<()> {
    client.write_all(&[SOCKS_VERSION, code, 0, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
}

fn protocol_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Reads the rest of a request or reply after its 4-byte head: address and port.
fn read_address(stream: &mut TcpStream, address_type: u8) -> io::Result<Vec<u8>> {
    let length = match address_type {
        ATYP_IPV4 => 4,
        ATYP_IPV6 => 16,
        ATYP_DOMAIN => {
            let mut length = [0u8];
            stream.read_exact(&mut length)?;
            let mut rest = vec![0; usize::from(length[0]) + 2];
            stream.read_exact(&mut rest)?;
            rest.insert(0, length[0]);
            return Ok(rest);
        }
        _ => return Err(protocol_error("unknown SOCKS address type")),
    };
    let mut rest = vec![0; length + 2];
    stream.read_exact(&mut rest)?;
    Ok(rest)
}

fn connect_upstream(upstream: &Upstream) -> io::Result<TcpStream> {
    let mut last = protocol_error("the SOCKS proxy address did not resolve");
    for address in (upstream.host.as_str(), upstream.port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, UPSTREAM_TIMEOUT) {
            Ok(stream) => {
                stream.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
                return Ok(stream);
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// Opens an authenticated connection to the upstream proxy.
fn login(upstream: &Upstream) -> io::Result<TcpStream> {
    let mut stream = connect_upstream(upstream)?;
    stream.write_all(&[SOCKS_VERSION, 1, AUTH_PASSWORD])?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice)?;
    if choice != [SOCKS_VERSION, AUTH_PASSWORD] {
        return Err(protocol_error("the SOCKS proxy refused password login"));
    }

    let username = upstream.username.as_bytes();
    let password = upstream.password.expose_secret().as_bytes();
    let mut message = Zeroizing::new(Vec::with_capacity(3 + username.len() + password.len()));
    message.push(1);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "SocksRelay::start limits both to 255 bytes"
    )]
    {
        message.push(username.len() as u8);
        message.extend_from_slice(username);
        message.push(password.len() as u8);
    }
    message.extend_from_slice(password);
    stream.write_all(&message)?;

    let mut status = [0u8; 2];
    stream.read_exact(&mut status)?;
    if status[1] != 0 {
        return Err(protocol_error("the SOCKS proxy rejected the login"));
    }
    Ok(stream)
}

fn relay_connection(mut client: TcpStream, upstream: &Upstream) -> io::Result<()> {
    client.set_nonblocking(false)?;
    client.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;

    let mut head = [0u8; 2];
    client.read_exact(&mut head)?;
    if head[0] != SOCKS_VERSION {
        return Err(protocol_error("the browser did not speak SOCKS5"));
    }
    let mut methods = vec![0; usize::from(head[1])];
    client.read_exact(&mut methods)?;
    if !methods.contains(&AUTH_NONE) {
        client.write_all(&[SOCKS_VERSION, AUTH_UNACCEPTABLE])?;
        return Ok(());
    }
    client.write_all(&[SOCKS_VERSION, AUTH_NONE])?;

    let mut request = [0u8; 4];
    client.read_exact(&mut request)?;
    if request[1] != CMD_CONNECT {
        return reply_failure(&mut client, REPLY_COMMAND_UNSUPPORTED);
    }
    let address = read_address(&mut client, request[3])?;

    let mut server = match login(upstream) {
        Ok(server) => server,
        Err(error) => {
            reply_failure(&mut client, REPLY_GENERAL_FAILURE)?;
            return Err(error);
        }
    };
    server.write_all(&request)?;
    server.write_all(&address)?;

    let mut reply = [0u8; 4];
    server.read_exact(&mut reply)?;
    let bound = read_address(&mut server, reply[3])?;
    client.write_all(&reply)?;
    client.write_all(&bound)?;
    if reply[1] != 0 {
        return Ok(());
    }

    client.set_read_timeout(None)?;
    server.set_read_timeout(None)?;
    let (mut client_read, mut server_write) = (client.try_clone()?, server.try_clone()?);
    let uploads = std::thread::spawn(move || {
        let _ = io::copy(&mut client_read, &mut server_write);
        let _ = server_write.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = uploads.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SOCKS5 proxy that insists on a login, and answers a CONNECT by echoing
    /// one line back. Returns its port.
    fn fake_proxy(user: &'static [u8], pass: &'static [u8]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = stream.unwrap();
                std::thread::spawn(move || {
                    let mut greeting = [0u8; 3];
                    s.read_exact(&mut greeting).unwrap();
                    assert_eq!(greeting, [5, 1, 2]);
                    s.write_all(&[5, 2]).unwrap();
                    let mut head = [0u8; 2];
                    s.read_exact(&mut head).unwrap();
                    let mut got_user = vec![0; usize::from(head[1])];
                    s.read_exact(&mut got_user).unwrap();
                    let mut len = [0u8];
                    s.read_exact(&mut len).unwrap();
                    let mut got_pass = vec![0; usize::from(len[0])];
                    s.read_exact(&mut got_pass).unwrap();
                    if got_user != user || got_pass != pass {
                        s.write_all(&[1, 1]).unwrap();
                        return;
                    }
                    s.write_all(&[1, 0]).unwrap();
                    let mut request = [0u8; 4];
                    s.read_exact(&mut request).unwrap();
                    assert_eq!(request[1], CMD_CONNECT);
                    let address = read_address(&mut s, request[3]).unwrap();
                    assert_eq!(&address[1..12], b"example.com");
                    s.write_all(&[5, 0, 0, ATYP_IPV4, 1, 2, 3, 4, 0, 80])
                        .unwrap();
                    let mut line = [0u8; 5];
                    s.read_exact(&mut line).unwrap();
                    s.write_all(&line).unwrap();
                });
            }
        });
        port
    }

    fn relay(port: u16, password: &str) -> SocksRelay {
        SocksRelay::start(
            &format!("socks5://127.0.0.1:{port}"),
            "alice",
            SecretString::from(password.to_string()),
        )
        .unwrap()
    }

    /// Does the browser's side of the exchange; returns the CONNECT reply code.
    fn browser_connect(relay: &SocksRelay) -> (TcpStream, u8) {
        let mut s = TcpStream::connect(("127.0.0.1", relay.local_port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(&[5, 1, AUTH_NONE]).unwrap();
        let mut choice = [0u8; 2];
        s.read_exact(&mut choice).unwrap();
        assert_eq!(choice, [5, AUTH_NONE]);
        let mut request = vec![5, CMD_CONNECT, 0, ATYP_DOMAIN, 11];
        request.extend_from_slice(b"example.com");
        request.extend_from_slice(&[0, 80]);
        s.write_all(&request).unwrap();
        let mut reply = [0u8; 10];
        s.read_exact(&mut reply).unwrap();
        (s, reply[1])
    }

    #[test]
    fn a_connection_is_logged_in_and_carried_both_ways() {
        let relay = relay(fake_proxy(b"alice", b"s3cret"), "s3cret");
        let (mut s, code) = browser_connect(&relay);
        assert_eq!(code, 0);
        s.write_all(b"hello").unwrap();
        let mut echoed = [0u8; 5];
        s.read_exact(&mut echoed).unwrap();
        assert_eq!(&echoed, b"hello");
    }

    #[test]
    fn a_rejected_login_fails_the_browser_connection() {
        let relay = relay(fake_proxy(b"alice", b"s3cret"), "wrong");
        assert_eq!(browser_connect(&relay).1, REPLY_GENERAL_FAILURE);
    }

    #[test]
    fn a_dead_proxy_fails_the_browser_connection() {
        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = dead.local_addr().unwrap().port();
        drop(dead);
        assert_eq!(browser_connect(&relay(port, "x")).1, REPLY_GENERAL_FAILURE);
    }

    #[test]
    fn dropping_the_relay_stops_it() {
        let relay = relay(fake_proxy(b"a", b"b"), "b");
        let port = relay.local_port;
        drop(relay);
        std::thread::sleep(ACCEPT_POLL * 4);
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn an_unusable_login_is_refused() {
        let start = |user: &str, pass: &str| {
            SocksRelay::start("socks5://p:1", user, SecretString::from(pass.to_string()))
        };
        assert!(matches!(start("", "x"), Err(SocksRelayError::InvalidLogin)));
        assert!(matches!(start("x", ""), Err(SocksRelayError::InvalidLogin)));
        assert!(matches!(
            start(&"u".repeat(256), "x"),
            Err(SocksRelayError::InvalidLogin)
        ));
        assert!(matches!(
            SocksRelay::start("nope", "x", SecretString::from("x".to_string())),
            Err(SocksRelayError::InvalidProxy)
        ));
    }

    #[test]
    fn debug_output_holds_no_password() {
        let relay = relay(fake_proxy(b"a", b"b"), "hunter2");
        assert!(!format!("{relay:?}").contains("hunter2"));
    }
}
