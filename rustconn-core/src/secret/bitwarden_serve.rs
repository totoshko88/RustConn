//! Persistent `bw serve` transport (W2 P2-transport).
//!
//! RustConn's Bitwarden backend (`super::bitwarden`) spawns one `bw` process
//! per vault operation. Each spawn pays the Node.js startup cost (~2.7 s on the
//! maintainer's machine), so a root-search retrieve that runs three `bw list`
//! calls took ~13.7 s. The Bitwarden CLI can instead run a long-lived local
//! HTTP server (`bw serve`) that keeps the unlocked vault in memory and answers
//! each operation over a socket; the G1 spike measured unlock at 2.6 s and a
//! 56-item root-search retrieve at 0.7 s over that transport, an order of
//! magnitude faster (`.agents/tasks/g1-spike-notes.md`).
//!
//! # Transport choice — loopback TCP
//!
//! The 0.24 plan assumed a socketpair handed to `bw serve --hostname
//! fd+connected://0` (an anonymous fd, no filesystem path, safe under the snap
//! AppArmor profile). The G1 probe found that even `bw` 2026.10.1 — the newest
//! the maintainer runs — does not advertise `fd+connected://` in `serve --help`;
//! it offers only TCP `--hostname`/`--port`. A filesystem `unix://` socket is
//! supported from CLI 2025.9.0+ but fails under snap with `EACCES` (the profile
//! only permits sockets under `/run/user/<uid>/snap.bw/`). Loopback TCP is the
//! one transport that works on every packaging the probe reached, so this module
//! binds `bw serve` to `127.0.0.1` on a random ephemeral port.
//!
//! The security trade-off of a loopback port — any process under the same uid
//! can reach it — is bounded: the CLI keeps its `Origin`/`Host` CSRF protection
//! (we never pass `--disable-origin-protection`), the port is random and chosen
//! by the kernel (bind to `127.0.0.1:0`, then read it back), and the session key
//! an attacker could obtain already lives in this process's memory anyway. A
//! future CLI that advertises `fd+connected://` can be adopted opportunistically
//! without changing the request layer; only `ServeProcess::spawn` would gain a
//! socketpair branch.
//!
//! # What this module is (and is not)
//!
//! This is the transport only: spawn/health/shutdown of the `bw serve` child and
//! typed request/response over HTTP/1.1. It does not yet replace the verbs in
//! `super::bitwarden`; wiring `BitwardenBackend` to prefer this transport (the
//! `BwTransport { Spawn, Serve }` switch) is the next W2 step and is gated on the
//! maintainer's per-environment GO. Nothing here runs unless the crate is built
//! with the `bitwarden-serve` feature.
//!
//! # Secret hygiene
//!
//! The master password reaches `bw serve` only in the JSON body of `POST
//! /unlock`, never on a URL or argv — the leak (`/proc/<pid>/cmdline`, world
//! readable) that the one-shot `bw create item <base64>` path still has. Request
//! and response bodies that can carry a secret are held in `Zeroizing`
//! buffers, and no response body is ever logged.

use std::process::Stdio;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use hyper::client::conn::http1;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use zeroize::Zeroizing;

use crate::error::{SecretError, SecretResult};

/// The loopback host `bw serve` binds to. Never `all`, never a routable address:
/// the server must not be reachable off the machine.
const SERVE_HOST: &str = "127.0.0.1";

/// `Host` header sent with every request. `bw serve`'s CSRF guard only accepts a
/// loopback authority; the port is filled in once the child's port is known.
const SERVE_HOST_HEADER_BASE: &str = "127.0.0.1";

/// How long to wait for the child's HTTP endpoint to start answering `GET
/// /status` after spawn. The spike saw the first response ~0.73 s after spawn;
/// this allows generous headroom for a cold Node start on a slow box.
const SERVE_READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Per-request timeout once the server is up. An unlock does KDF work in Node and
/// can take a couple of seconds; a hung request past this means the child is
/// wedged and the caller falls back to one-shot spawns.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The `{success, data, message}` envelope every `bw serve` route returns.
///
/// A success carries `data` (object or array, route-dependent); a failure
/// carries `message`. The data shape is decoded per route by the caller, so it
/// stays a raw [`serde_json::Value`] here.
#[derive(Debug, Deserialize)]
pub struct ServeEnvelope {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Route-specific payload on success (`null` when the route returns none).
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    /// Human-readable reason on failure.
    #[serde(default)]
    pub message: Option<String>,
}

impl ServeEnvelope {
    /// Returns the `data` payload on success, or a [`SecretError`] built from
    /// `message` on failure. The message is CLI-authored (e.g. "mac failed",
    /// "Vault is locked") and carries no secret, so it is safe to surface.
    ///
    /// # Errors
    /// Returns [`SecretError::ConnectionFailed`] carrying the CLI's `message`
    /// when `success` is false.
    pub fn into_data(self) -> SecretResult<serde_json::Value> {
        if self.success {
            Ok(self.data.unwrap_or(serde_json::Value::Null))
        } else {
            Err(SecretError::ConnectionFailed(format!(
                "bw serve: {}",
                self.message.as_deref().unwrap_or("request failed")
            )))
        }
    }
}

/// Which transport a Bitwarden operation uses.
///
/// The backend starts on [`Self::Spawn`] (one `bw` process per call) and, once
/// the maintainer records a GO and the wiring lands, prefers [`Self::Serve`]
/// with a one-shot fall back to `Spawn` if the serve child dies. Defined here so
/// the switch has a home before the backend consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BwTransport {
    /// One `bw` child per vault operation. The historical path; always works.
    #[default]
    Spawn,
    /// A persistent `bw serve` child driven over HTTP. Faster; falls back to
    /// [`Self::Spawn`] on failure.
    Serve,
}

/// A running `bw serve` child bound to a random loopback port.
///
/// Dropping it kills the child (`kill_on_drop`), which closes the HTTP endpoint
/// and drops the unlocked vault from the Node process's memory. Call
/// [`Self::lock`] first for an orderly lock when the session is ending while the
/// machine keeps running.
pub struct ServeProcess {
    child: Child,
    port: u16,
}

impl ServeProcess {
    /// Spawns `bw serve` on `127.0.0.1:<random port>` and waits for it to answer.
    ///
    /// The port is picked by binding a throwaway [`TcpListener`] to
    /// `127.0.0.1:0`, reading the kernel-assigned port, and freeing it before
    /// handing the number to `bw serve`. There is a small race between freeing
    /// the probe socket and `bw` binding it, but on loopback with an ephemeral
    /// port the window is negligible and a bind failure surfaces as the child
    /// exiting, which [`Self::wait_until_ready`] reports.
    ///
    /// `bw_cmd` is the resolved CLI path (see [`super::bitwarden::resolve_bw_cmd`]).
    /// `session_key`, when present, is passed as `BW_SESSION` so a `bw serve`
    /// started with an existing session comes up already unlocked.
    ///
    /// # Errors
    /// Returns [`SecretError`] if a port cannot be reserved, the child cannot be
    /// spawned, or the endpoint does not answer within [`SERVE_READY_TIMEOUT`].
    pub async fn spawn(bw_cmd: &str, session_key: Option<&SecretString>) -> SecretResult<Self> {
        let port = reserve_loopback_port().await?;

        let mut cmd = Command::new(bw_cmd);
        cmd.env("PATH", crate::cli_download::get_extended_path());
        cmd.kill_on_drop(true);
        cmd.arg("serve")
            .arg("--hostname")
            .arg(SERVE_HOST)
            .arg("--port")
            .arg(port.to_string());
        // Never --disable-origin-protection: the CSRF guard is what keeps a
        // malicious local page from driving the vault through this port.
        if let Some(session) = session_key {
            cmd.env("BW_SESSION", session.expose_secret());
        }
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let child = cmd
            .spawn()
            .map_err(|e| SecretError::ConnectionFailed(format!("failed to spawn bw serve: {e}")))?;

        let process = Self { child, port };
        process.wait_until_ready().await?;
        Ok(process)
    }

    /// The loopback port the child is serving on.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Polls `GET /status` until it answers or [`SERVE_READY_TIMEOUT`] elapses.
    async fn wait_until_ready(&self) -> SecretResult<()> {
        let deadline = tokio::time::Instant::now() + SERVE_READY_TIMEOUT;
        loop {
            if self.request("GET", "/status", None).await.is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(SecretError::ConnectionFailed(
                    "bw serve did not become ready in time".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Sends one HTTP/1.1 request to the serve child and returns the decoded
    /// envelope.
    ///
    /// `path` includes any query string (already percent-encoded by the caller).
    /// `body`, when present, is sent as `application/json`; it is held in a
    /// `Zeroizing` buffer by the caller, and hyper takes an owned [`Bytes`]
    /// copy for the single request, dropped when the exchange ends.
    ///
    /// # Errors
    /// Returns [`SecretError`] on a connect/transport error, a non-success HTTP
    /// status, a body that is not the `bw serve` envelope, or a timeout.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> SecretResult<ServeEnvelope> {
        let fut = self.request_inner(method, path, body);
        tokio::time::timeout(REQUEST_TIMEOUT, fut)
            .await
            .map_err(|_| SecretError::ConnectionFailed("bw serve request timed out".to_string()))?
    }

    async fn request_inner(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> SecretResult<ServeEnvelope> {
        let stream = TcpStream::connect((SERVE_HOST, self.port))
            .await
            .map_err(|e| SecretError::ConnectionFailed(format!("bw serve connect failed: {e}")))?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = http1::handshake::<_, Full<Bytes>>(io)
            .await
            .map_err(|e| SecretError::ConnectionFailed(format!("bw serve handshake: {e}")))?;
        // Drive the connection; it finishes when the response is read.
        tokio::spawn(async move {
            let _ = conn.await;
        });

        // hyper needs an owned body; copy the caller's (zeroized) slice into the
        // single Bytes for this request. Empty when there is no body.
        let payload = body.map(Bytes::copy_from_slice).unwrap_or_default();
        let authority = format!("{SERVE_HOST_HEADER_BASE}:{}", self.port);
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(hyper::header::HOST, authority);
        if body.is_some() {
            builder = builder.header(hyper::header::CONTENT_TYPE, "application/json");
        }
        let request = builder
            .body(Full::new(payload))
            .map_err(|e| SecretError::ConnectionFailed(format!("bw serve request build: {e}")))?;

        let response = sender
            .send_request(request)
            .await
            .map_err(|e| SecretError::ConnectionFailed(format!("bw serve send: {e}")))?;
        let status = response.status();
        let collected = response
            .into_body()
            .collect()
            .await
            .map_err(|e| SecretError::ConnectionFailed(format!("bw serve body: {e}")))?
            .to_bytes();
        // The response can carry a password (a retrieved login). Keep it in a
        // zeroizing buffer and never log it.
        let body_bytes = Zeroizing::new(collected.to_vec());

        if status != StatusCode::OK {
            // A non-200 still usually carries the envelope; try it so the CLI's
            // message reaches the caller, but never the raw body.
            if let Ok(env) = serde_json::from_slice::<ServeEnvelope>(&body_bytes) {
                return Ok(env);
            }
            return Err(SecretError::ConnectionFailed(format!(
                "bw serve returned HTTP {}",
                status.as_u16()
            )));
        }

        serde_json::from_slice::<ServeEnvelope>(&body_bytes)
            .map_err(|e| SecretError::ConnectionFailed(format!("bw serve: bad envelope: {e}")))
    }

    /// Unlocks the vault over `POST /unlock`.
    ///
    /// The master password is sent only in the JSON body. On success the CLI
    /// returns a session key in `data.raw`; it is returned here as a
    /// [`SecretString`] for the caller to store (e.g. in `BW_SESSION_STORE` for
    /// any spawn fallback) and never logged.
    ///
    /// # Errors
    /// Returns [`SecretError`] if the request fails or the vault refuses the
    /// password.
    pub async fn unlock(&self, master_password: &SecretString) -> SecretResult<SecretString> {
        // Build the JSON body in a zeroizing buffer so the password never sits in
        // a plain String. serde_json emits the password escaped and quoted.
        let body = Zeroizing::new(
            serde_json::to_vec(&serde_json::json!({
                "password": master_password.expose_secret(),
            }))
            .map_err(|e| {
                SecretError::ConnectionFailed(format!("bw serve: unlock body encode: {e}"))
            })?,
        );
        let data = self
            .request("POST", "/unlock", Some(&body))
            .await?
            .into_data()?;
        let raw = data
            .get("raw")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SecretError::ConnectionFailed(
                    "bw serve: unlock response missing session key".to_string(),
                )
            })?;
        Ok(SecretString::from(raw.to_string()))
    }

    /// Locks the vault over `POST /lock`, dropping the decrypted data from the
    /// serve process without killing it.
    ///
    /// # Errors
    /// Returns [`SecretError`] if the request fails.
    pub async fn lock(&self) -> SecretResult<()> {
        self.request("POST", "/lock", None).await?.into_data()?;
        Ok(())
    }

    /// Kills the serve child and waits for it to exit.
    ///
    /// Idempotent-ish: a second call after the child is gone is a cheap no-op.
    /// Dropping the [`ServeProcess`] also kills the child, so this is only needed
    /// when a caller wants to await the exit.
    pub async fn shutdown(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }

    /// Kills the serve child without waiting for it to exit, for a caller that
    /// must not block and may have no runtime (a lock-event handler). Like any
    /// `kill_on_drop` child, the exit status is reaped later by tokio.
    pub fn kill(mut self) {
        let _ = self.child.start_kill();
    }

    /// A `ServeProcess` around an arbitrary child, for tests of the code that
    /// owns one; the port is never used.
    #[cfg(test)]
    pub(super) const fn from_child(child: Child) -> Self {
        Self { child, port: 0 }
    }
}

/// Reserves an ephemeral loopback port by binding and immediately freeing it.
///
/// Returns the port number for `bw serve` to bind. See [`ServeProcess::spawn`]
/// for the small bind race this leaves and why it is acceptable on loopback.
async fn reserve_loopback_port() -> SecretResult<u16> {
    let listener = TcpListener::bind((SERVE_HOST, 0))
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("bw serve: no loopback port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| SecretError::ConnectionFailed(format!("bw serve: local_addr: {e}")))?
        .port();
    drop(listener);
    Ok(port)
}

/// One `bw` operation mapped onto a `bw serve` HTTP route.
///
/// Built by [`map_cli_to_route`] from the same `&[&str]` argv the spawn path
/// passes to `bw`, so the backend keeps describing operations in CLI terms and
/// the transport owns the translation. `body` is a JSON document (already the
/// decoded item/folder, not base64) sent with `POST`/`PUT`.
struct ServeRoute {
    method: &'static str,
    path: String,
    body: Option<Vec<u8>>,
    /// How to reshape the envelope's `data` into what the spawn path's stdout
    /// parser expects (see [`DataShape::unwrap_data`]).
    shape: DataShape,
}

/// How a route's `data` payload maps to the `bw` CLI stdout the callers parse.
#[derive(Clone, Copy)]
enum DataShape {
    /// `bw status` prints the status object; serve wraps it as
    /// `data.template`. Unwrap one level.
    StatusTemplate,
    /// `bw list …` prints a bare array; serve wraps it as `data.data`. Return
    /// the inner array.
    ListArray,
    /// `bw create/edit/get …` prints the object; serve returns it as `data`.
    /// Return `data` verbatim.
    Object,
    /// `bw sync`/`lock` print a message, not JSON the caller parses; the empty
    /// string is fine.
    None,
}

impl DataShape {
    /// Reshapes an envelope `data` value into the string the spawn path's
    /// `stdout` would have carried, so callers' `serde_json::from_str` is
    /// unchanged.
    fn unwrap_data(self, data: serde_json::Value) -> SecretResult<String> {
        let reshaped = match self {
            Self::StatusTemplate => data
                .get("template")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            Self::ListArray => data
                .get("data")
                .cloned()
                .unwrap_or(serde_json::Value::Array(Vec::new())),
            Self::Object => data,
            Self::None => return Ok(String::new()),
        };
        serde_json::to_string(&reshaped)
            .map_err(|e| SecretError::ConnectionFailed(format!("bw serve: reshape data: {e}")))
    }
}

/// Maps a `bw` CLI argv to the `bw serve` route that performs the same thing.
///
/// `input` is the stdin the spawn path would have piped (the base64-encoded
/// item/folder for `create`/`edit`); it is decoded back to the JSON document the
/// serve body wants. Returns `None` for an argv the serve transport does not
/// implement, so the caller falls back to a one-shot spawn for that verb rather
/// than guessing.
///
/// Supported: `status`, `sync`, `lock`, `list folders`, `list items --search T`,
/// `create folder <b64>`, `create item` (stdin), `edit item <id>` (stdin),
/// `delete item <id>`.
fn map_cli_to_route(args: &[&str], input: Option<&[u8]>) -> Option<ServeRoute> {
    // The body for create/edit comes in as base64 (argv for folder, stdin for
    // item); decode it to the raw JSON document the serve endpoint expects.
    let decode_body = |encoded: &[u8]| -> Option<Vec<u8>> {
        let text = std::str::from_utf8(encoded).ok()?.trim();
        base64_decode(text)
    };

    match args {
        ["status"] => Some(ServeRoute {
            method: "GET",
            path: "/status".to_string(),
            body: None,
            shape: DataShape::StatusTemplate,
        }),
        ["sync"] => Some(ServeRoute {
            method: "POST",
            path: "/sync".to_string(),
            body: None,
            shape: DataShape::None,
        }),
        ["lock"] => Some(ServeRoute {
            method: "POST",
            path: "/lock".to_string(),
            body: None,
            shape: DataShape::None,
        }),
        ["list", "folders"] => Some(ServeRoute {
            method: "GET",
            path: "/list/object/folders".to_string(),
            body: None,
            shape: DataShape::ListArray,
        }),
        ["list", "items", "--search", term] => Some(ServeRoute {
            method: "GET",
            path: format!("/list/object/items?search={}", encode_query_value(term)),
            body: None,
            shape: DataShape::ListArray,
        }),
        ["create", "folder", encoded] => Some(ServeRoute {
            method: "POST",
            path: "/object/folder".to_string(),
            body: decode_body(encoded.as_bytes()),
            shape: DataShape::Object,
        }),
        ["create", "item"] => Some(ServeRoute {
            method: "POST",
            path: "/object/item".to_string(),
            body: input.and_then(decode_body),
            shape: DataShape::Object,
        }),
        ["edit", "item", id] => Some(ServeRoute {
            method: "PUT",
            path: format!("/object/item/{id}"),
            body: input.and_then(decode_body),
            shape: DataShape::Object,
        }),
        ["delete", "item", id] => Some(ServeRoute {
            method: "DELETE",
            path: format!("/object/item/{id}"),
            body: None,
            shape: DataShape::None,
        }),
        _ => None,
    }
}

/// Whether the serve transport implements the verb in `args`.
///
/// Lets the backend skip taking the serve lock (and starting a child) for a verb
/// that would only fall back to a spawn anyway.
#[must_use]
pub fn is_supported_verb(args: &[&str]) -> bool {
    map_cli_to_route(args, None).is_some()
}

/// Decodes standard (padded) base64 without pulling in a new crate.
///
/// `data-encoding` is already a direct dependency of this crate; use its
/// `BASE64` alphabet so this matches what `super::bitwarden`'s
/// `base64_encode`/`base64_encode_secret` produce.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    data_encoding::BASE64.decode(text.as_bytes()).ok()
}

impl ServeProcess {
    /// Runs a `bw` operation described by its CLI argv over the serve transport.
    ///
    /// This is the single entry point `super::bitwarden` calls when the serve
    /// transport is active: it maps the argv to a route, performs it, and
    /// returns the JSON string the spawn path's `stdout` would have carried, so
    /// the caller's parser is unchanged. The body (item/folder) is held in a
    /// `Zeroizing` buffer because it carries the plaintext password.
    ///
    /// # Errors
    /// Returns [`SecretError::ConnectionFailed`] with an
    /// `unsupported-serve-verb` marker for a verb the transport does not
    /// implement (so the caller falls back to spawn), or the transport/CLI
    /// error otherwise.
    pub async fn run_cli(&self, args: &[&str], input: Option<&[u8]>) -> SecretResult<String> {
        let ServeRoute {
            method,
            path,
            body,
            shape,
        } = map_cli_to_route(args, input).ok_or_else(|| {
            SecretError::ConnectionFailed(format!(
                "unsupported-serve-verb: {}",
                args.iter().take(2).copied().collect::<Vec<_>>().join(" ")
            ))
        })?;
        let body = body.map(Zeroizing::new);
        let env = self
            .request(method, &path, body.as_deref().map(|b| &b[..]))
            .await?;
        let data = env.into_data()?;
        shape.unwrap_data(data)
    }
}

/// Percent-encodes a `bw serve` search/query value for a URL query string.
///
/// `bw serve` routes a root search through `GET /list/object/items?search=<term>`.
/// The term can carry spaces, `&`, `#` and non-ASCII, so it must be encoded;
/// this keeps only the unreserved set (RFC 3986) literal and percent-encodes
/// every other byte, which `bw serve` (Express `req.query`) decodes back.
#[must_use]
pub fn encode_query_value(value: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        let keep = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if keep {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener as StdTcpListener;
    use std::thread;

    /// A minimal fake `bw serve`: a loopback TCP listener that answers the three
    /// routes the transport drives with the real envelope shape, then exits. The
    /// real `bw` cannot run in CI (no vault, snap-confine refuses), so this pins
    /// the transport's request/response handling without it.
    fn spawn_fake_serve(count: usize) -> (u16, thread::JoinHandle<Vec<String>>) {
        let listener = StdTcpListener::bind((SERVE_HOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let mut request_lines = Vec::new();
            for _ in 0..count {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                request_lines.push(handle_one(stream));
            }
            request_lines
        });
        (port, handle)
    }

    /// Reads one HTTP/1.1 request, records its request line and body, and writes
    /// the matching `bw serve` envelope. Returns the request line plus any body,
    /// so a test can assert the password never rode in the URL.
    fn handle_one(mut stream: std::net::TcpStream) -> String {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();

        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                break;
            }
            // hyper sends header names lower-cased, so match case-insensitively.
            if let Some((name, value)) = trimmed.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        let body = if content_length > 0 {
            let mut buf = vec![0u8; content_length];
            reader.read_exact(&mut buf).unwrap();
            String::from_utf8_lossy(&buf).to_string()
        } else {
            String::new()
        };

        let payload = if request_line.contains("POST /unlock") {
            r#"{"success":true,"data":{"raw":"fake-session-key","title":"unlocked"}}"#
        } else if request_line.contains("/list/object/items") {
            r#"{"success":true,"data":{"data":[{"id":"abc","name":"RustConn: target"}]}}"#
        } else if request_line.contains("GET /status") {
            r#"{"success":true,"data":{"template":{"status":"unlocked"}}}"#
        } else if request_line.contains("POST /lock") {
            r#"{"success":true,"data":{"title":"locked"}}"#
        } else {
            r#"{"success":false,"message":"unknown route"}"#
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
        format!("{}\n{body}", request_line.trim_end())
    }

    /// A `ServeProcess` is normally built by spawning `bw`; for the transport
    /// tests we point one at the fake serve's port with no child.
    fn serve_at(port: u16) -> ServeProcess {
        // SAFETY of the dummy child: `std::process::Command::new("true")` is a
        // real, short-lived process so `kill_on_drop`/`wait` are well-defined; we
        // only ever use the `port`, never the child, in these tests.
        let child = Command::new("true").spawn().expect("spawn true");
        ServeProcess { child, port }
    }

    #[tokio::test]
    async fn status_round_trips_the_envelope() {
        let (port, handle) = spawn_fake_serve(1);
        let serve = serve_at(port);
        let env = serve.request("GET", "/status", None).await.unwrap();
        assert!(env.success);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn unlock_sends_the_password_only_in_the_body() {
        let (port, handle) = spawn_fake_serve(1);
        let serve = serve_at(port);
        let key = serve
            .unlock(&SecretString::from("master-pw".to_string()))
            .await
            .unwrap();
        assert_eq!(key.expose_secret(), "fake-session-key");

        let captured = handle.join().unwrap();
        let exchange = &captured[0];
        let (request_line, body) = exchange.split_once('\n').unwrap();
        // The password must be in the body, never in the request line (URL/argv).
        assert!(
            request_line.starts_with("POST /unlock"),
            "request line: {request_line}"
        );
        assert!(
            !request_line.contains("master-pw"),
            "password leaked into the request line: {request_line}"
        );
        assert!(
            body.contains("master-pw"),
            "password should be in the JSON body: {body}"
        );
    }

    #[tokio::test]
    async fn list_search_round_trips_the_item() {
        let (port, handle) = spawn_fake_serve(1);
        let serve = serve_at(port);
        let path = format!("/list/object/items?search={}", encode_query_value("target"));
        let data = serve
            .request("GET", &path, None)
            .await
            .unwrap()
            .into_data()
            .unwrap();
        let items = data.get("data").and_then(|d| d.as_array()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "RustConn: target");
        handle.join().unwrap();
    }

    #[test]
    fn a_failure_envelope_becomes_an_error() {
        let env = ServeEnvelope {
            success: false,
            data: None,
            message: Some("Vault is locked".to_string()),
        };
        let err = env.into_data().unwrap_err();
        assert!(err.to_string().contains("Vault is locked"), "{err}");
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode_query_value("aws"), "aws");
        assert_eq!(encode_query_value("a b&c"), "a%20b%26c");
        assert_eq!(encode_query_value("host.example-1_x"), "host.example-1_x");
        // Non-ASCII is encoded byte by byte (UTF-8).
        assert_eq!(encode_query_value("é"), "%C3%A9");
    }

    #[test]
    fn transport_default_is_spawn() {
        assert_eq!(BwTransport::default(), BwTransport::Spawn);
    }

    // ---- verb mapping (P3) ----

    /// `(argv, stdin, expected method, expected path)` for the mapping table.
    type VerbCase<'a> = (&'a [&'a str], Option<&'a [u8]>, &'a str, &'a str);

    #[test]
    fn maps_every_supported_verb_to_the_right_route() {
        let b64 = data_encoding::BASE64.encode(br#"{"name":"RustConn"}"#);
        let cases: &[VerbCase] = &[
            (&["status"], None, "GET", "/status"),
            (&["sync"], None, "POST", "/sync"),
            (&["lock"], None, "POST", "/lock"),
            (&["list", "folders"], None, "GET", "/list/object/folders"),
            (
                &["delete", "item", "abc"],
                None,
                "DELETE",
                "/object/item/abc",
            ),
        ];
        for (args, input, method, path) in cases {
            let route = map_cli_to_route(args, *input)
                .unwrap_or_else(|| panic!("verb should map: {args:?}"));
            assert_eq!(route.method, *method, "{args:?}");
            assert_eq!(route.path, *path, "{args:?}");
        }
        // create folder decodes the base64 argv into the JSON body.
        let route = map_cli_to_route(&["create", "folder", &b64], None).unwrap();
        assert_eq!(route.method, "POST");
        assert_eq!(route.path, "/object/folder");
        assert_eq!(route.body.as_deref(), Some(&br#"{"name":"RustConn"}"#[..]));
    }

    #[test]
    fn list_search_route_encodes_the_term() {
        let route = map_cli_to_route(&["list", "items", "--search", "a b"], None).unwrap();
        assert_eq!(route.method, "GET");
        assert_eq!(route.path, "/list/object/items?search=a%20b");
    }

    #[test]
    fn create_and_edit_item_take_the_body_from_stdin() {
        let item = data_encoding::BASE64.encode(br#"{"type":1,"name":"x"}"#);
        let create = map_cli_to_route(&["create", "item"], Some(item.as_bytes())).unwrap();
        assert_eq!(create.method, "POST");
        assert_eq!(create.path, "/object/item");
        assert_eq!(
            create.body.as_deref(),
            Some(&br#"{"type":1,"name":"x"}"#[..])
        );

        let edit = map_cli_to_route(&["edit", "item", "id9"], Some(item.as_bytes())).unwrap();
        assert_eq!(edit.method, "PUT");
        assert_eq!(edit.path, "/object/item/id9");
        assert_eq!(edit.body.as_deref(), Some(&br#"{"type":1,"name":"x"}"#[..]));
    }

    #[test]
    fn an_unmapped_verb_is_none_so_the_caller_falls_back_to_spawn() {
        assert!(map_cli_to_route(&["login", "--apikey"], None).is_none());
        assert!(map_cli_to_route(&["get", "item", "x"], None).is_none());
        assert!(!is_supported_verb(&["login", "--apikey"]));
        assert!(is_supported_verb(&["status"]));
    }

    #[test]
    fn data_shapes_reshape_to_cli_stdout() {
        // status: serve wraps the object under data.template.
        let status = serde_json::json!({"template": {"status": "unlocked"}});
        let out = DataShape::StatusTemplate.unwrap_data(status).unwrap();
        assert_eq!(out, r#"{"status":"unlocked"}"#);

        // list: serve wraps the array under data.data; callers expect a bare array.
        let list = serde_json::json!({"object": "list", "data": [{"id": "a"}]});
        let out = DataShape::ListArray.unwrap_data(list).unwrap();
        assert_eq!(out, r#"[{"id":"a"}]"#);

        // object: returned verbatim.
        let obj = serde_json::json!({"id": "a", "name": "n"});
        let out = DataShape::Object.unwrap_data(obj.clone()).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&out).unwrap(),
            obj
        );

        // none: sync/lock carry no parsed stdout.
        assert_eq!(
            DataShape::None
                .unwrap_data(serde_json::Value::Null)
                .unwrap(),
            ""
        );
    }

    #[test]
    fn base64_decode_round_trips_the_encoder_the_backend_uses() {
        let doc = br#"{"type":1,"name":"RustConn: x"}"#;
        let encoded = data_encoding::BASE64.encode(doc);
        assert_eq!(base64_decode(&encoded).as_deref(), Some(&doc[..]));
        assert!(base64_decode("not valid base64 !!!").is_none());
    }
}
