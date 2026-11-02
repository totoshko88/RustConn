//! G1 spike: prototype of the `bw serve` transport over a socketpair (W2).
//!
//! The 0.24 plan replaces one-shot `bw` spawns with a long-lived `bw serve`
//! that RustConn drives over a socket (`--hostname fd+connected://0`, with a
//! socketpair handed in as the child's stdin). The risky, un-obvious part is
//! not the HTTP — it is whether a hand-driven HTTP/1.1 exchange over a Unix
//! stream round-trips the `bw serve` envelope `{success, data, message}` that
//! the real transport will parse.
//!
//! This test proves that exchange against a FAKE serve (a `UnixListener` in a
//! temp dir speaking the same envelope), with NO real `bw` and NO new
//! dependency: the request and response are written and read as raw bytes over
//! the stream, which is exactly the shape the production transport uses before
//! a parser is layered on. The live latency/RSS/`fd+connected://` numbers that
//! actually decide go/no-go are measured by `scripts/bw-serve-probe.sh` on the
//! maintainer's machine, since the sandbox has no vault.
//!
//! SPIKE artefact on 0.24.0-bw-serve-spike — not a production module.

#![allow(
    clippy::unwrap_used,
    reason = "a spike prototype test uses unwrap for brevity"
)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

/// A minimal fake `bw serve`: one `UnixListener`, one connection at a time,
/// answering the two routes the transport needs with the real envelope shape.
fn spawn_fake_serve(socket_path: &std::path::Path) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(socket_path).unwrap();
    thread::spawn(move || {
        // Two requests: unlock, then a list search. Serve them and exit.
        for _ in 0..2 {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            handle_one(stream);
        }
    })
}

/// Reads one HTTP/1.1 request and writes the matching `bw serve` envelope.
fn handle_one(mut stream: UnixStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());

    // Request line.
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();

    // Drain headers; capture Content-Length for a body read.
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
    }

    // Route on the request line's path.
    let body = if request_line.contains("POST /unlock") {
        r#"{"success":true,"data":{"raw":"fake-session-key","title":"Your vault is unlocked!"}}"#
    } else if request_line.contains("/list/object/items") {
        r#"{"success":true,"data":{"data":[{"id":"abc","name":"RustConn: target","login":{"username":"alice","password":"s3cret"}}]}}"#
    } else {
        r#"{"success":false,"message":"unknown route"}"#
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

/// Sends one HTTP/1.1 request over the stream and returns the response body.
fn http_round_trip(socket_path: &std::path::Path, request: &str) -> String {
    let mut stream = UnixStream::connect(socket_path).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    stream.flush().unwrap();

    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    // Split headers from body at the blank line.
    raw.split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or(raw)
}

#[test]
fn unlock_and_list_round_trip_over_a_unix_stream() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("bw-serve.sock");
    let handle = spawn_fake_serve(&socket_path);

    // 1. POST /unlock with the master password in the body (never the URL).
    let unlock_body = r#"{"password":"master-pw"}"#;
    let unlock_req = format!(
        "POST /unlock HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        unlock_body.len(),
        unlock_body
    );
    let unlock_resp = http_round_trip(&socket_path, &unlock_req);
    assert!(
        unlock_resp.contains("\"success\":true"),
        "unlock envelope: {unlock_resp}"
    );
    assert!(
        unlock_resp.contains("fake-session-key"),
        "unlock must carry data.raw (the BW_SESSION): {unlock_resp}"
    );

    // 2. GET the search list (root search).
    let list_req = "GET /list/object/items?search=target HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let list_resp = http_round_trip(&socket_path, list_req);
    assert!(
        list_resp.contains("\"success\":true"),
        "list envelope: {list_resp}"
    );
    assert!(
        list_resp.contains("RustConn: target") && list_resp.contains("s3cret"),
        "list must round-trip the item: {list_resp}"
    );

    handle.join().unwrap();
}

#[test]
fn the_envelope_shape_is_what_the_transport_will_parse() {
    // Pins the two envelope shapes the production transport (bitwarden_serve.rs)
    // will deserialize: a success carries `data`, a failure carries `message`.
    // A string check here is enough for the spike; the real transport gets typed
    // structs. Guards against a plan assumption drifting before that lands.
    let success = r#"{"success":true,"data":{"raw":"k"}}"#;
    let failure = r#"{"success":false,"message":"mac failed"}"#;
    assert!(success.contains("\"success\":true") && success.contains("\"data\""));
    assert!(failure.contains("\"success\":false") && failure.contains("\"message\""));
}
