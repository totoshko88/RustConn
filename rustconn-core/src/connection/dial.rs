//! TCP dialling for the embedded clients and the reachability probes.
//!
//! `TcpStream::connect("host:port")` tries the resolved addresses one after
//! another, so a name whose first address never answers — typically an AAAA
//! record a VPN hands out but does not route — spends the whole connect
//! timeout there and never reaches the IPv4 address that works.
//! [`connect_tcp`] races the addresses instead, giving each a 250 ms head
//! start before the next one is tried (RFC 8305, "Happy Eyeballs").

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::net::TcpStream;

/// Head start an attempt gets before the next address is tried (RFC 8305 §5).
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Connects to `host` on `port`, over MPTCP when `mptcp` is set, using the
/// first of its addresses that answers.
///
/// `host` may be a name, an IP literal, or a bracketed IPv6 literal.
///
/// # Errors
///
/// Returns the resolver's error when `host` does not resolve, otherwise the
/// error of the last address that failed.
pub async fn connect_tcp(host: &str, port: u16, mptcp: bool) -> io::Result<TcpStream> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
    if mptcp {
        race(addrs, |addr| async move {
            super::mptcp::connect_mptcp_async(addr)
                .await
                .map_err(io::Error::other)
        })
        .await
    } else {
        race(addrs, TcpStream::connect).await
    }
}

/// Returns the first connection `connect` makes to any of `addrs`.
///
/// An attempt that fails outright hands over to the next address at once;
/// one that hangs is raced by the next address after [`ATTEMPT_DELAY`].
async fn race<T, F, Fut>(addrs: Vec<SocketAddr>, connect: F) -> io::Result<T>
where
    F: Fn(SocketAddr) -> Fut,
    Fut: Future<Output = io::Result<T>>,
{
    let mut queue = interleave_families(addrs);
    let mut attempts = FuturesUnordered::new();
    let mut last_error = None;
    attempts.extend(queue.pop_front().map(&connect));
    while !attempts.is_empty() {
        tokio::select! {
            Some(result) = attempts.next() => match result {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    last_error = Some(e);
                    attempts.extend(queue.pop_front().map(&connect));
                }
            },
            () = tokio::time::sleep(ATTEMPT_DELAY), if !queue.is_empty() => {
                attempts.extend(queue.pop_front().map(&connect));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "the host resolved to no addresses")
    }))
}

/// Alternates address families, keeping the resolver's preferred family
/// first (RFC 8305 §4), so one unreachable family costs a single delay.
fn interleave_families(addrs: Vec<SocketAddr>) -> VecDeque<SocketAddr> {
    let Some(prefers_v6) = addrs.first().map(SocketAddr::is_ipv6) else {
        return VecDeque::new();
    };
    let (preferred, other): (Vec<_>, Vec<_>) = addrs
        .into_iter()
        .partition(|addr| addr.is_ipv6() == prefers_v6);
    let mut interleaved = VecDeque::with_capacity(preferred.len() + other.len());
    let mut preferred = preferred.into_iter();
    let mut other = other.into_iter();
    loop {
        match (preferred.next(), other.next()) {
            (None, None) => return interleaved,
            (first, second) => interleaved.extend(first.into_iter().chain(second)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().expect("test address")
    }

    #[test]
    fn families_alternate_from_the_preferred_one() {
        let v6a = addr("[2001:db8::1]:3389");
        let v6b = addr("[2001:db8::2]:3389");
        let v4a = addr("192.0.2.1:3389");
        assert_eq!(
            interleave_families(vec![v6a, v6b, v4a]),
            VecDeque::from([v6a, v4a, v6b])
        );
        assert!(interleave_families(Vec::new()).is_empty());
    }

    #[tokio::test]
    async fn an_address_that_never_answers_does_not_block_the_next() {
        // The VPN case: the AAAA record is black-holed, the A record works.
        let dead = addr("[2001:db8::1]:3389");
        let alive = addr("192.0.2.1:3389");
        let attempt = race(vec![dead, alive], |addr| async move {
            if addr == dead {
                futures::future::pending::<()>().await;
            }
            Ok(addr)
        });
        let winner = tokio::time::timeout(Duration::from_secs(5), attempt).await;
        assert_eq!(winner.ok().and_then(Result::ok), Some(alive));
    }

    #[tokio::test]
    async fn the_last_error_is_returned_when_every_address_fails() {
        let addrs = vec![addr("192.0.2.1:3389"), addr("192.0.2.2:3389")];
        let result: io::Result<SocketAddr> = race(addrs, |addr| async move {
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                addr.to_string(),
            ))
        })
        .await;
        let error = result.expect_err("no address answers");
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert_eq!(error.to_string(), "192.0.2.2:3389");

        let none: io::Result<SocketAddr> = race(Vec::new(), |addr| async move { Ok(addr) }).await;
        assert_eq!(
            none.map_err(|e| e.kind()).err(),
            Some(io::ErrorKind::NotFound)
        );
    }

    #[tokio::test]
    async fn connects_to_a_local_listener_by_name_and_literal() {
        let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
            // A sandbox without loopback networking cannot exercise this.
            return;
        };
        let Ok(port) = listener.local_addr().map(|a| a.port()) else {
            return;
        };
        let accept = tokio::spawn(async move {
            for _ in 0..2 {
                let _ = listener.accept().await;
            }
        });
        // `localhost` usually resolves to `::1` first, where nothing listens:
        // the refusal must hand over to 127.0.0.1.
        assert!(connect_tcp("localhost", port, false).await.is_ok());
        assert!(connect_tcp("127.0.0.1", port, false).await.is_ok());
        accept.abort();
    }
}
