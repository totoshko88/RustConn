//! Classification of embedded RDP connection failures.
//!
//! The embedded IronRDP client reports failures to the GUI as plain strings
//! (`RdpClientEvent::Error`), because the error crosses a thread channel and
//! upstream `ironrdp` error types are not `Clone`. The GUI has to decide what
//! to do with each failure:
//!
//! * retry with a different graphics mode,
//! * hand the session over to the external `FreeRDP` client,
//! * or surface an authentication error and stop.
//!
//! Historically that decision was a list of `msg.contains(..)` checks inlined
//! in the GTK layer, which silently broke every time an upstream error string
//! changed (issues [#199], [#234], [#235]). The matching now lives here as a
//! pure, unit-tested function so the GUI only switches on the resulting
//! [`RdpFailureClass`].
//!
//! What to *tell* the user about a rejected sign-in is a separate, finer
//! question, answered by [`classify_auth_failure`]: a Kerberos KDC that cannot
//! be found needs different advice from a wrong password (issue [#351]). It
//! never changes the fallback decision.
//!
//! [#199]: https://github.com/totoshko88/RustConn/issues/199
//! [#234]: https://github.com/totoshko88/RustConn/issues/234
//! [#235]: https://github.com/totoshko88/RustConn/issues/235
//! [#351]: https://github.com/totoshko88/RustConn/issues/351

/// What kind of failure ended (or prevented) an embedded RDP session.
///
/// Ordered from "most specific" to "least specific"; [`classify_rdp_failure`]
/// returns the first matching class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpFailureClass {
    /// The server rejected the supplied credentials (CredSSP/NLA `NTSTATUS`
    /// logon failure, disabled or locked account, expired password).
    Authentication,
    /// The MS-TSGU tunnel to the RD Gateway could not be established.
    GatewayFailure,
    /// The GFX pipeline produced no decodable frame or failed to decode.
    GraphicsPipeline,
    /// The server offers only a security protocol IronRDP does not implement.
    SecurityUnsupported,
    /// IronRDP and the server disagree on the wire protocol, including a
    /// connect sequence that stalls after the TLS upgrade has already
    /// succeeded (issue #392, e.g. a GNOME Remote Desktop Remote Login
    /// server waiting for a Server Redirection IronRDP cannot follow).
    ProtocolIncompatible,
    /// Anything else: unreachable host, timeout, TLS failure, local error.
    Other,
}

impl RdpFailureClass {
    /// Returns `true` when handing the session to external `FreeRDP` can help.
    #[must_use]
    pub const fn warrants_freerdp_fallback(self) -> bool {
        matches!(
            self,
            Self::SecurityUnsupported
                | Self::ProtocolIncompatible
                | Self::GraphicsPipeline
                | Self::GatewayFailure
        )
    }

    /// Returns `true` when fallback would weaken the negotiated security.
    #[must_use]
    pub const fn requires_explicit_consent(self) -> bool {
        matches!(self, Self::SecurityUnsupported)
    }
}

/// `NTSTATUS` codes CredSSP/NLA returns for credential problems.
const AUTH_NSTATUS_CODES: &[&str] = &[
    "0xc0000064", // STATUS_NO_SUCH_USER
    "0xc000006d", // STATUS_LOGON_FAILURE
    "0xc000006a", // STATUS_WRONG_PASSWORD
    "0xc000006e", // STATUS_ACCOUNT_RESTRICTION
    "0xc000006f", // STATUS_INVALID_LOGON_HOURS
    "0xc0000070", // STATUS_INVALID_WORKSTATION
    "0xc0000071", // STATUS_PASSWORD_EXPIRED
    "0xc0000072", // STATUS_ACCOUNT_DISABLED
    "0xc000015b", // STATUS_LOGON_TYPE_NOT_GRANTED
    "0xc0000193", // STATUS_ACCOUNT_EXPIRED
    "0xc0000224", // STATUS_PASSWORD_MUST_CHANGE
    "0xc0000234", // STATUS_ACCOUNT_LOCKED_OUT
];

/// Symbolic `NTSTATUS` names and other markers of a credential rejection.
const AUTH_MARKERS: &[&str] = &[
    "authentication failed",
    "status_no_such_user",
    "status_logon_failure",
    "status_wrong_password",
    "status_password_expired",
    "status_password_must_change",
    "status_account_disabled",
    "status_account_expired",
    "status_account_locked_out",
    "status_account_restriction",
    "status_logon_type_not_granted",
    "accessdenied",
];

/// Markers of a failed MS-TSGU tunnel to the RD Gateway.
///
/// Produced by the gateway branch of the embedded connect path; the external
/// FreeRDP client speaks the same protocol with a wider set of authentication
/// methods (`ironrdp-mstsgu` only offers HTTP Basic), so it is worth a try.
const GATEWAY_MARKERS: &[&str] = &["rd gateway connection failed"];

/// Marker for a connect sequence that stalled *after* the TLS upgrade.
///
/// Built and matched by [`post_tls_stall_error`] / [`classify_rdp_failure`]
/// from a single constant so the text and the classifier can never drift
/// apart. Lower-case, because `classify_rdp_failure` compares against
/// `msg.to_ascii_lowercase()`.
///
/// Issue [#392]: against GNOME Remote Desktop's Remote Login mode, IronRDP
/// completes X.224 and TLS, then waits the full handshake budget inside
/// `connect_finalize` (NLA → MCS → licensing → capabilities) with no error.
/// The server most likely expects an RDP Server Redirection to one-time
/// RDSTLS credentials that IronRDP never advertises support for, so the
/// server has nothing to send back; `ironrdp-connector` 0.10 does not
/// implement that negotiation (upstream TODO #139, `cluster: None`). A PDU
/// the connector cannot decode would surface as a decode error, not a silent
/// stall, so the redirection explanation is the likelier of the two, not a
/// confirmed cause. Before this marker existed, that stall produced the
/// generic `"Operation timed out"` message, which matches
/// [`NON_FALLBACK_MARKERS`] and stranded the session with no fallback even
/// though the server is reachable and speaks RDP — Remmina and GNOME
/// Connections (both FreeRDP-based) connect to the same host.
///
/// [#392]: https://github.com/totoshko88/RustConn/issues/392
const POST_TLS_STALL_MARKER: &str =
    "connection sequence stalled after the secure channel was established";

/// Builds the [`crate::embedded_client_error::RdpClientError`] the connect code
/// returns when the handshake timeout expires *after* the TLS upgrade (and TOFU
/// certificate check) succeeded.
///
/// `handshake_secs` is the budget that just expired, folded into the message so
/// the log/GUI text says how long the client waited. The message deliberately
/// contains none of `"tls"`, `"transport"`, `"timed out"`, `"certificate"`, or
/// any auth/license marker — see [`NON_FALLBACK_MARKERS`], [`AUTH_MARKERS`] and
/// [`LICENSE_MARKERS`] — so it is classified purely through
/// [`POST_TLS_STALL_MARKER`] and cannot accidentally also match one of those.
///
/// See [`POST_TLS_STALL_MARKER`] for why this fires and issue [#392].
///
/// [#392]: https://github.com/totoshko88/RustConn/issues/392
#[must_use]
pub fn post_tls_stall_error(handshake_secs: u64) -> crate::embedded_client_error::RdpClientError {
    crate::embedded_client_error::RdpClientError::ConnectionFailed(format!(
        "IronRDP {POST_TLS_STALL_MARKER}: sign-in, licensing or capabilities \
         did not finish within {handshake_secs}s"
    ))
}

/// TLS, certificate, and transport failures that must not trigger fallback.
const NON_FALLBACK_MARKERS: &[&str] = &[
    "tls",
    "certificate",
    "ssl_cert_not_on_server",
    "unknown issuer",
    "x509",
    "transport",
    "connection refused",
    "connection reset",
    "connection timed out",
    "operation timed out",
    "dns_name_not_found",
    "host not found",
    "network is unreachable",
    "no route to host",
];

/// Markers of a GFX/EGFX pipeline problem.
///
/// `gfx unsupported codec` covers the case where the server sends surface
/// content in a codec `ironrdp-egfx` cannot decode. Retrying without GFX is the
/// right response: the RemoteFX/bitmap path has no such gap (issue [#262]).
///
/// [#262]: https://github.com/totoshko88/RustConn/issues/262
const GRAPHICS_MARKERS: &[&str] = &[
    "no-frame-watchdog",
    "gfx pipeline decode failure",
    "gfx unsupported codec",
];

/// Explicit security protocols IronRDP cannot speak.
///
/// Two spellings of "Standard RDP Security" appear, from two different code
/// paths, and both must fall back to external FreeRDP (which implements the
/// RC4 standard security IronRDP does not):
///
/// * `"standard rdp security"` — prose, as in `ironrdp`'s
///   `"server only supports Standard RDP Security"` negotiation error
///   (issue [#235]).
/// * `"standard_rdp_security"` — the `SecurityProtocol` enum variant name,
///   emitted verbatim by `ironrdp-connector` 0.10 when the server down-selects
///   to standard security after the client advertised SSL/HYBRID/HYBRID_EX:
///   `"client advertised SSL | HYBRID | HYBRID_EX, but server selected
///   STANDARD_RDP_SECURITY"`. xrdp's default `security_layer=negotiate` picks
///   this, so without the underscore spelling the session fell into `Other`
///   and was stranded with no fallback, even though external FreeRDP connects.
///
/// [#235]: https://github.com/totoshko88/RustConn/issues/235
const SECURITY_MARKERS: &[&str] = &[
    "standard rdp security",
    "standard_rdp_security",
    "ssl_not_allowed_by_server",
    "hybrid_required_by_server",
    "unsupported security protocol",
];

/// Specific IronRDP/server protocol mismatches that justify fallback.
/// Generic finalize and negotiation wrappers are intentionally excluded.
const PROTOCOL_MARKERS: &[&str] = &[
    "serverdemandactive",
    "serverdeactivateall",
    "invalid state (this is a bug)",
    "unexpected share control pdu",
    "unsupported pdu",
    "decode error",
    "unsupported fast-path update code",
    // A broker redirection chain the embedded client gave up following.
    "server redirection",
    // A rejected `BasicSecurityHeader`. Kept here rather than in
    // [`LICENSE_MARKERS`] because the header is common to every Standard RDP
    // Security PDU, so it identifies the fallback but not the phase.
    "securityheaderflags",
];

/// Markers naming the RDP licensing exchange as the phase that failed.
///
/// A subset of [`PROTOCOL_MARKERS`] rather than a class of its own: handing the
/// session to the external client is still the right response, so the *decision*
/// does not change. What changes is what the user is told — `"decode error"` is
/// broad enough to swallow this case, and "server incompatible" says nothing
/// about a server that is merely running RDS licensing.
///
/// Both entries name the phase itself. `securityHeaderFlags` — the field the
/// decoder actually rejects — is deliberately **not** here: it is a field of
/// `BasicSecurityHeader`, which every Standard RDP Security PDU carries, so a
/// decode failure mentioning it says nothing about *where* in the connection it
/// happened. Matching on it would report an unrelated header failure as "the
/// server requires RDS licensing". It stays in [`PROTOCOL_MARKERS`] instead,
/// where it only affects the fallback decision, which is the same either way.
///
/// The cause is upstream and open:
/// [IronRDP #1629](https://github.com/Devolutions/IronRDP/issues/1629). A
/// Windows host with RDS licensing sends `SEC_AUTODETECT_REQ` (an RTT probe,
/// flag `0x1000`) during the licensing exchange; `ironrdp-connector` feeds
/// whatever arrives to the licensing decoder without checking the channel or the
/// security header, and `LicenseHeader::decode` rejects it because
/// `SEC_LICENSE_PKT` (`0x0080`) is absent. Not to be confused with
/// [#1457](https://github.com/Devolutions/IronRDP/issues/1457) /
/// [#1458](https://github.com/Devolutions/IronRDP/pull/1458), which relaxed
/// `BasicSecurityHeader` and does not touch this check — and is in any case not
/// yet published (`ironrdp-pdu` on crates.io is still 0.9.0).
const LICENSE_MARKERS: &[&str] = &["server_new_license", "licenseexchangestate"];

/// Returns `true` when the failure happened in the RDP licensing exchange.
///
/// Used only to choose the message shown to the user; see [`LICENSE_MARKERS`].
#[must_use]
pub fn is_license_exchange_failure(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    LICENSE_MARKERS.iter().any(|m| lower.contains(m))
}

/// Classifies an embedded RDP failure message into a [`RdpFailureClass`].
///
/// Authentication is checked first, then RD Gateway tunnel failures, then a
/// post-TLS connect stall ([`POST_TLS_STALL_MARKER`], issue #392) — both are
/// checked ahead of the generic TLS/certificate/transport markers so that
/// wording in the wrapped cause can never re-classify them as `Other`. TLS,
/// certificate, and transport roots take precedence over the remaining
/// fallback-worthy protocol markers.
#[must_use]
pub fn classify_rdp_failure(msg: &str) -> RdpFailureClass {
    let lower = msg.to_ascii_lowercase();

    if is_authentication_failure_lower(&lower) {
        return RdpFailureClass::Authentication;
    }
    // Checked before the transport markers on purpose: a gateway failure wraps
    // the underlying cause ("TCP connect", "TLS connect", "WS Upgrade"), and
    // those words would otherwise classify the whole message as a plain
    // transport error and strand the session (issue #246). The external client
    // implements the same tunnel with more authentication methods, so handing
    // it over is worthwhile whatever the inner cause was.
    if GATEWAY_MARKERS.iter().any(|m| lower.contains(m)) {
        return RdpFailureClass::GatewayFailure;
    }
    // Checked before NON_FALLBACK_MARKERS for the same reason as the gateway
    // check above: the message built by `post_tls_stall_error` is deliberately
    // free of transport/TLS/timeout wording today, but a later edit to that
    // text (e.g. adding "TLS") must not silently fall back into `Other` and
    // re-strand the session (issue #392).
    if lower.contains(POST_TLS_STALL_MARKER) {
        return RdpFailureClass::ProtocolIncompatible;
    }
    if NON_FALLBACK_MARKERS.iter().any(|m| lower.contains(m)) {
        return RdpFailureClass::Other;
    }
    if GRAPHICS_MARKERS.iter().any(|m| lower.contains(m)) {
        return RdpFailureClass::GraphicsPipeline;
    }
    if SECURITY_MARKERS.iter().any(|m| lower.contains(m)) {
        return RdpFailureClass::SecurityUnsupported;
    }
    if PROTOCOL_MARKERS.iter().any(|m| lower.contains(m))
        || LICENSE_MARKERS.iter().any(|m| lower.contains(m))
    {
        return RdpFailureClass::ProtocolIncompatible;
    }
    RdpFailureClass::Other
}

/// Returns `true` when the message describes a rejected credential.
#[must_use]
pub fn is_authentication_failure(msg: &str) -> bool {
    is_authentication_failure_lower(&msg.to_ascii_lowercase())
}

fn is_authentication_failure_lower(lower: &str) -> bool {
    AUTH_NSTATUS_CODES.iter().any(|c| lower.contains(c))
        || AUTH_MARKERS.iter().any(|m| lower.contains(m))
}

/// Account-state `NTSTATUS` codes a TLS-only external retry can never fix.
///
/// These say *who* the account is, not *how* it signed in: a plain-TLS retry
/// would hit the identical refusal, so it is pointless and would only double
/// the delay before the real error is shown. `STATUS_LOGON_FAILURE`
/// (`0xc000006d`), `STATUS_WRONG_PASSWORD` (`0xc000006a`) and `STATUS_NO_SUCH_USER`
/// (`0xc0000064`) are deliberately **absent**: those are the exact statuses a
/// KRdp/FreeRDP-3.32 server returns for its broken `HYBRID_EX` NTLM path even
/// with correct credentials (issue [#366]), so they must stay retryable.
///
/// [#366]: https://github.com/totoshko88/RustConn/issues/366
const TLS_ONLY_FALLBACK_BLOCKED_STATUSES: &[&str] = &[
    "0xc000006e", // STATUS_ACCOUNT_RESTRICTION
    "0xc000006f", // STATUS_INVALID_LOGON_HOURS
    "0xc0000070", // STATUS_INVALID_WORKSTATION
    "0xc0000071", // STATUS_PASSWORD_EXPIRED
    "0xc0000072", // STATUS_ACCOUNT_DISABLED
    "0xc000015b", // STATUS_LOGON_TYPE_NOT_GRANTED
    "0xc0000193", // STATUS_ACCOUNT_EXPIRED
    "0xc0000224", // STATUS_PASSWORD_MUST_CHANGE
    "0xc0000234", // STATUS_ACCOUNT_LOCKED_OUT
];

/// Returns `true` when a CredSSP/NLA rejection is worth one external-FreeRDP
/// retry pinned to **TLS only** (`/sec:tls`, which drops `HYBRID`/`HYBRID_EX`).
///
/// A KRdp 6.6.x server built on FreeRDP 3.32.x advertises `HYBRID_EX` ("Ext")
/// security it cannot complete: the embedded IronRDP client (and the default
/// FreeRDP negotiation) let the server pick `HYBRID_EX`, the server's NTLM/SAM
/// path fails internally, and the client sees `STATUS_LOGON_FAILURE`
/// (`0xc000006d`) even with correct credentials. This is byte-for-byte what a
/// genuinely wrong password looks like, which is why issue [#366] was reported
/// as "invalid username or password" (FreeRDP issue #13567; KRdp fixed it in
/// 6.7.90). Remmina connects to the same server because it negotiates plain
/// TLS, which never offers `HYBRID_EX` — confirmed live: with `/sec:tls` the
/// FreeRDP negotiation drops to `nego_enable_ext: FALSE`.
///
/// So RustConn hands this one case to external FreeRDP with the security layer
/// pinned to TLS (`/sec:tls`), matching Remmina. The retry is narrow:
///
/// * it fires **only** for the ambiguous `InvalidToken` CredSSP shape (a
///   `credssp` message carrying `invalidtoken`, or the raw
///   `0xc000006d`/`0xc000006a`/`0xc0000064` logon statuses), never for the
///   account-state codes in [`TLS_ONLY_FALLBACK_BLOCKED_STATUSES`], which TLS
///   cannot fix;
/// * the caller fires it **once**, so a server that rejects the TLS-only path
///   too surfaces the real auth error (with the KRdp advice) instead of looping.
///
/// Pure and unit-tested here so the GUI does not grow another `msg.contains(..)`
/// chain that rots when an upstream error string changes (issues [#199], [#235]).
///
/// [#366]: https://github.com/totoshko88/RustConn/issues/366
/// [#199]: https://github.com/totoshko88/RustConn/issues/199
/// [#235]: https://github.com/totoshko88/RustConn/issues/235
#[must_use]
pub fn warrants_tls_only_fallback(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();

    // An account-state refusal is unambiguous and TLS cannot change it.
    if TLS_ONLY_FALLBACK_BLOCKED_STATUSES
        .iter()
        .any(|c| lower.contains(c))
    {
        return false;
    }

    // The CredSSP `InvalidToken` shape: either the sspi error type is named, or
    // the server returned one of the "wrong credential" logon statuses that the
    // KRdp HYBRID_EX regression also uses to turn the exchange away.
    let is_invalid_token = lower.contains("credssp")
        && (lower.contains("invalidtoken") || lower.contains("invalid_token"));
    let is_logon_reject = lower.contains("0xc000006d") // STATUS_LOGON_FAILURE
        || lower.contains("0xc000006a") // STATUS_WRONG_PASSWORD
        || lower.contains("0xc0000064"); // STATUS_NO_SUCH_USER

    is_invalid_token || is_logon_reject
}

/// What a rejected CredSSP/NLA sign-in was rejected for, as far as the message says.
///
/// Finer than [`RdpFailureClass::Authentication`]: that class decides that no
/// external client is tried, this one picks the advice shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthFailureKind {
    /// sspi found no KDC for the realm: no KDC Address, nothing in the
    /// environment and nothing in `krb5.conf`.
    NoKdc {
        /// The realm, when sspi's message names one, as it does for a
        /// cross-realm referral.
        realm: Option<String>,
    },
    /// A KDC was named but could not be reached over TCP or UDP, or the KDC
    /// proxy failed.
    KdcUnreachable,
    /// `STATUS_ACCOUNT_RESTRICTION`: an account restriction refused the sign-in,
    /// such as the NTLM ban on members of AD "Protected Users".
    AccountRestriction,
    /// Logon hours or workstation restrictions, or a KDC policy refusal.
    LogonRestriction,
    /// A wrong user name or password, or a user the domain does not know.
    InvalidCredentials,
    /// The password has expired.
    PasswordExpired,
    /// The password must be changed before the first sign-in.
    PasswordMustChange,
    /// The account is disabled.
    AccountDisabled,
    /// The account is locked out.
    AccountLockedOut,
    /// The account has expired.
    AccountExpired,
    /// The KDC revoked the account's credentials: disabled, locked out or
    /// expired, without saying which.
    AccountRevoked,
    /// The account may not sign in to this server, for example without the
    /// Remote Desktop logon right.
    LogonTypeNotGranted,
    /// The clocks of this computer and of the domain differ by more than
    /// Kerberos allows.
    ClockSkew,
    /// The KDC does not know the service principal `TERMSRV/<host>`, usually
    /// because the host is not the server's DNS name.
    UnknownServerPrincipal,
    /// Any other CredSSP/NLA failure.
    Generic,
}

/// `NTSTATUS` codes a CredSSP server returns, and what each says about the sign-in.
///
/// `0xc0000070` is `STATUS_INVALID_WORKSTATION`. The GUI used to word it as
/// "password must be changed", which is `0xc0000224`.
static NTSTATUS_KINDS: &[(&str, AuthFailureKind)] = &[
    ("0xc000006d", AuthFailureKind::InvalidCredentials), // STATUS_LOGON_FAILURE
    ("0xc000006a", AuthFailureKind::InvalidCredentials), // STATUS_WRONG_PASSWORD
    ("0xc0000064", AuthFailureKind::InvalidCredentials), // STATUS_NO_SUCH_USER
    ("0xc000006e", AuthFailureKind::AccountRestriction), // STATUS_ACCOUNT_RESTRICTION
    ("0xc000006f", AuthFailureKind::LogonRestriction),   // STATUS_INVALID_LOGON_HOURS
    ("0xc0000070", AuthFailureKind::LogonRestriction),   // STATUS_INVALID_WORKSTATION
    ("0xc0000071", AuthFailureKind::PasswordExpired),    // STATUS_PASSWORD_EXPIRED
    ("0xc0000072", AuthFailureKind::AccountDisabled),    // STATUS_ACCOUNT_DISABLED
    ("0xc000015b", AuthFailureKind::LogonTypeNotGranted), // STATUS_LOGON_TYPE_NOT_GRANTED
    ("0xc0000193", AuthFailureKind::AccountExpired),     // STATUS_ACCOUNT_EXPIRED
    ("0xc0000224", AuthFailureKind::PasswordMustChange), // STATUS_PASSWORD_MUST_CHANGE
    ("0xc0000234", AuthFailureKind::AccountLockedOut),   // STATUS_ACCOUNT_LOCKED_OUT
    ("status_logon_failure", AuthFailureKind::InvalidCredentials),
];

/// `KRB-ERROR` texts as sspi 0.21 words them (its `utils.rs` error-code table).
static KERBEROS_KINDS: &[(&str, AuthFailureKind)] = &[
    // KRB_AP_ERR_SKEW (37), which sspi files under `ErrorKind::TimeSkew`.
    ("clock skew too great", AuthFailureKind::ClockSkew),
    ("timeskew", AuthFailureKind::ClockSkew),
    // KDC_ERR_S_PRINCIPAL_UNKNOWN (7): the domain has no `TERMSRV/<host>`.
    (
        "server not found in kerberos database",
        AuthFailureKind::UnknownServerPrincipal,
    ),
    // KDC_ERR_C_PRINCIPAL_UNKNOWN (6)
    (
        "client not found in kerberos database",
        AuthFailureKind::InvalidCredentials,
    ),
    // KDC_ERR_PREAUTH_FAILED (24): the password did not decrypt the timestamp.
    (
        "pre-authentication information was invalid",
        AuthFailureKind::InvalidCredentials,
    ),
    // KDC_ERR_KEY_EXPIRED (23)
    ("password has expired", AuthFailureKind::PasswordExpired),
    // KDC_ERR_CLIENT_REVOKED (18)
    (
        "clients credentials have been revoked",
        AuthFailureKind::AccountRevoked,
    ),
    // KDC_ERR_POLICY (12)
    (
        "kdc policy rejects request",
        AuthFailureKind::LogonRestriction,
    ),
];

/// sspi's wording when neither the configuration nor its own lookup names a KDC.
const NO_KDC_MARKER: &str = "no kdc server found";

/// Contexts of the KDC transport errors raised by `ironrdp-tokio`'s
/// `ReqwestNetworkClient` (0.10). Its `Display` carries the context only, not
/// the underlying I/O error.
const KDC_TRANSPORT_MARKERS: &[&str] = &[
    "failed to send kdc request",
    "failed to receive kdc response",
    "[kdcproxy @",
    "cannot bind udp socket",
    "failed to send udp request",
    "failed to receive udp request",
];

/// Says what a CredSSP/NLA failure was about, to pick the advice shown to the user.
///
/// `None` when the message is not about the NLA sign-in at all — a refused TCP
/// connection, a TLS failure, a timeout — which the caller words itself.
/// Checked from the most specific to the least: server `NTSTATUS` codes,
/// Kerberos `KRB-ERROR` texts, a KDC that is missing or unreachable, an early
/// user authorization refusal, and finally any other CredSSP failure as
/// [`AuthFailureKind::Generic`].
///
/// Never consulted for the fallback decision: whether an external client is
/// tried is [`classify_rdp_failure`]'s call alone.
#[must_use]
pub fn classify_auth_failure(msg: &str) -> Option<AuthFailureKind> {
    let lower = msg.to_ascii_lowercase();
    if let Some(kind) = first_listed(&lower, NTSTATUS_KINDS) {
        return Some(kind);
    }
    if let Some(kind) = first_listed(&lower, KERBEROS_KINDS) {
        return Some(kind);
    }
    if lower.contains(NO_KDC_MARKER) {
        return Some(AuthFailureKind::NoKdc {
            realm: realm_named_in(msg, &lower),
        });
    }
    if KDC_TRANSPORT_MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(AuthFailureKind::KdcUnreachable);
    }
    // `EarlyUserAuthResult::AccessDenied`: the server turned the account away
    // after CredSSP succeeded, which is what a missing Remote Desktop logon
    // right does.
    if lower.contains("accessdenied") {
        return Some(AuthFailureKind::LogonTypeNotGranted);
    }
    lower
        .contains("credssp")
        .then_some(AuthFailureKind::Generic)
}

/// The kind of the first table entry whose marker occurs in `lower`.
fn first_listed(lower: &str, table: &[(&str, AuthFailureKind)]) -> Option<AuthFailureKind> {
    table
        .iter()
        .find(|(marker, _)| lower.contains(marker))
        .map(|(_, kind)| kind.clone())
}

/// The realm in sspi's ``No KDC server found for realm `X` `` wording, if any.
fn realm_named_in(msg: &str, lower: &str) -> Option<String> {
    const PREFIX: &str = "for realm `";
    // `lower` is `msg` lower-cased byte for byte, so offsets carry over.
    let start = lower.find(PREFIX)? + PREFIX.len();
    let rest = msg.get(start..)?;
    let realm = rest.get(..rest.find('`')?)?.trim();
    (!realm.is_empty()).then(|| realm.to_owned())
}

/// How many times a session whose connection dropped is reconnected before
/// the failure is reported — Remote Desktop Connection's default for the
/// `.rdp` `autoreconnect max retries`.
pub const AUTO_RECONNECT_MAX_ATTEMPTS: u32 = 20;

/// The wait before automatic reconnection attempt `attempt` (counted from 1):
/// 1, 2, 4 and 8 seconds, then 10 seconds for every later attempt.
///
/// Short at first, so a brief Wi-Fi or VPN hiccup is bridged quickly, and
/// capped, so twenty attempts cover a few minutes of outage — about the
/// window in which Windows keeps a disconnected session for reconnection by
/// default — without hammering an unreachable host.
#[must_use]
pub fn auto_reconnect_delay(attempt: u32) -> std::time::Duration {
    const CAP_SECS: u64 = 10;
    let secs = 1_u64
        .checked_shl(attempt.saturating_sub(1))
        .map_or(CAP_SECS, |secs| secs.min(CAP_SECS));
    std::time::Duration::from_secs(secs)
}

#[cfg(test)]
mod reconnect_tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn reconnect_delay_doubles_then_caps_at_ten_seconds() {
        let secs: Vec<u64> = (1..=7).map(|n| auto_reconnect_delay(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 10, 10, 10]);
        assert_eq!(auto_reconnect_delay(0), Duration::from_secs(1));
        assert_eq!(auto_reconnect_delay(u32::MAX), Duration::from_secs(10));
    }

    #[test]
    fn every_attempt_fits_in_a_few_minutes() {
        let total: u64 = (1..=AUTO_RECONNECT_MAX_ATTEMPTS)
            .map(|n| auto_reconnect_delay(n).as_secs())
            .sum();
        assert!((60..=300).contains(&total), "{total} s");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUE_235: &str = "Connection failed: Connection begin failed: \
         negotiation failure: server only supports Standard RDP Security";

    const CREDSSP_LOGON_FAILURE: &str = "Connection failed: Connection finalize failed: \
         CredSSP server returned an error status; \
         nstatus: Some(NStatusCode(0xc000006d))";

    /// Verbatim `ironrdp-connector` 0.10 error when a server (e.g. xrdp with the
    /// default `security_layer=negotiate`) down-selects to standard RDP security
    /// after the client advertised the enhanced protocols. The variant name
    /// `STANDARD_RDP_SECURITY` has underscores, not spaces, so it does not match
    /// the prose `"standard rdp security"` marker from issue #235.
    const CONNECTOR_STANDARD_RDP_DOWNSELECT: &str = "Connection failed: Connection begin failed: \
         [Initiation] reason: client advertised SSL | HYBRID | HYBRID_EX, \
         but server selected STANDARD_RDP_SECURITY \
         [kind: Reason(\"client advertised SSL | HYBRID | HYBRID_EX, \
         but server selected STANDARD_RDP_SECURITY\")]";

    #[test]
    fn standard_rdp_security_is_security_unsupported() {
        let class = classify_rdp_failure(ISSUE_235);
        assert_eq!(class, RdpFailureClass::SecurityUnsupported);
        assert!(class.warrants_freerdp_fallback());
    }

    #[test]
    fn connector_standard_rdp_downselect_is_security_unsupported() {
        // The underscored `STANDARD_RDP_SECURITY` enum-variant spelling from
        // ironrdp-connector 0.10 must classify exactly like the prose #235 form,
        // so an xrdp (or any standard-security-only) server falls back to the
        // external FreeRDP client instead of being stranded in `Other`.
        let class = classify_rdp_failure(CONNECTOR_STANDARD_RDP_DOWNSELECT);
        assert_eq!(class, RdpFailureClass::SecurityUnsupported);
        assert!(class.warrants_freerdp_fallback());
        // It is a security-unsupported case, not the CredSSP/HYBRID_EX one, so
        // it must not also trip the TLS-only (#366) retry gate.
        assert!(!warrants_tls_only_fallback(
            CONNECTOR_STANDARD_RDP_DOWNSELECT
        ));
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(
            classify_rdp_failure("UNEXPECTED SHARE CONTROL PDU"),
            RdpFailureClass::ProtocolIncompatible
        );
        assert_eq!(
            classify_rdp_failure("GfX PiPeLiNe DeCoDe FaIlUrE"),
            RdpFailureClass::GraphicsPipeline
        );
        assert_eq!(
            classify_rdp_failure("SERVER ONLY SUPPORTS STANDARD RDP SECURITY"),
            RdpFailureClass::SecurityUnsupported
        );
    }

    #[test]
    fn credssp_logon_failure_is_authentication() {
        let class = classify_rdp_failure(CREDSSP_LOGON_FAILURE);
        assert_eq!(class, RdpFailureClass::Authentication);
        assert!(!class.warrants_freerdp_fallback());
    }

    #[test]
    fn krdp_hybrid_ex_rejection_warrants_a_tls_only_fallback() {
        // The verbatim shape issue #366 produces: KRdp (FreeRDP 3.32 Ext-security
        // regression) rejects the CredSSP IronRDP sent because the server picked
        // HYBRID_EX, with STATUS_LOGON_FAILURE — indistinguishable from a wrong
        // password, so this must route to a /sec:tls external retry.
        assert!(warrants_tls_only_fallback(CREDSSP_LOGON_FAILURE));
        assert!(warrants_tls_only_fallback(
            "CredSSP [kind: Credssp(Error { error_type: InvalidToken, description: \
             \"CredSSP server returned an error status\", nstatus: Some(NStatusCode(0xc000006d)) })]"
        ));
        for msg in [
            "nstatus: Some(NStatusCode(0xC000006D))",
            "STATUS_WRONG_PASSWORD 0xc000006a",
            "0xc0000064 no such user",
        ] {
            assert!(warrants_tls_only_fallback(msg), "message: {msg}");
        }
    }

    #[test]
    fn account_state_refusals_do_not_warrant_a_tls_only_fallback() {
        // These say who the account is, not how it authenticated: a TLS-only
        // retry cannot change them, so it must not fire and waste a round-trip.
        for code in [
            "0xc000006e", // ACCOUNT_RESTRICTION
            "0xc000006f", // INVALID_LOGON_HOURS
            "0xc0000070", // INVALID_WORKSTATION
            "0xc0000071", // PASSWORD_EXPIRED
            "0xc0000072", // ACCOUNT_DISABLED
            "0xc000015b", // LOGON_TYPE_NOT_GRANTED
            "0xc0000193", // ACCOUNT_EXPIRED
            "0xc0000224", // PASSWORD_MUST_CHANGE
            "0xc0000234", // ACCOUNT_LOCKED_OUT
        ] {
            let msg = format!(
                "CredSSP [kind: Credssp(Error {{ error_type: InvalidToken, \
                 nstatus: Some(NStatusCode({code})) }})]"
            );
            assert!(
                !warrants_tls_only_fallback(&msg),
                "account-state code wrongly retried: {code}"
            );
        }
    }

    #[test]
    fn non_credssp_failures_do_not_warrant_a_tls_only_fallback() {
        for msg in [
            ISSUE_235,
            ISSUE_351_NO_KDC_GUI,
            "TLS handshake failed: invalid peer certificate",
            "Failed to connect to host:3389: connection refused",
            "Account restriction nstatus: Some(NStatusCode(0xc000006e))",
        ] {
            assert!(!warrants_tls_only_fallback(msg), "message: {msg}");
        }
    }

    #[test]
    fn missing_account_statuses_are_authentication() {
        for marker in [
            "nstatus: Some(NStatusCode(0xC0000064))",
            "nstatus: Some(NStatusCode(0xc0000193))",
            "STATUS_NO_SUCH_USER",
            "status_account_expired",
        ] {
            assert_eq!(
                classify_rdp_failure(marker),
                RdpFailureClass::Authentication,
                "marker was not classified as authentication: {marker}"
            );
        }
    }

    #[test]
    fn tls_and_certificate_failures_never_fall_back() {
        for msg in [
            "Connection finalize failed: TLS handshake failed: invalid peer certificate",
            "Unexpected Share Control Pdu after certificate verification failed",
            "SSL_CERT_NOT_ON_SERVER during negotiation",
        ] {
            let class = classify_rdp_failure(msg);
            assert_eq!(class, RdpFailureClass::Other, "message: {msg}");
            assert!(!class.warrants_freerdp_fallback(), "message: {msg}");
        }
    }

    #[test]
    fn transport_failures_never_fall_back() {
        for msg in [
            "negotiation failed: ERRCONNECT_CONNECT_TRANSPORT_FAILED",
            "ServerDemandActive: connection reset by peer",
            "Connection refused (os error 111)",
        ] {
            let class = classify_rdp_failure(msg);
            assert_eq!(class, RdpFailureClass::Other, "message: {msg}");
            assert!(!class.warrants_freerdp_fallback(), "message: {msg}");
        }
    }

    /// Issue #392: a connect sequence that stalls *after* TLS has to reach the
    /// external-client fallback the same way the GRD #199 case does.
    #[test]
    fn post_tls_stall_falls_back_to_the_external_client() {
        let msg = post_tls_stall_error(60).to_string();
        let class = classify_rdp_failure(&msg);
        assert_eq!(
            class,
            RdpFailureClass::ProtocolIncompatible,
            "message: {msg}"
        );
        assert!(class.warrants_freerdp_fallback(), "message: {msg}");
        assert!(!class.requires_explicit_consent(), "message: {msg}");
        assert!(!is_authentication_failure(&msg), "message: {msg}");
        assert_eq!(classify_auth_failure(&msg), None, "message: {msg}");
        assert!(!warrants_tls_only_fallback(&msg), "message: {msg}");
        // The "server incompatible" banner is right here, not the licensing one.
        assert!(!is_license_exchange_failure(&msg), "message: {msg}");
    }

    /// Issue #392: a timeout that happens *before* TLS — an unreachable host, a
    /// refused port, an unknown name, or a failed TLS upgrade — must keep
    /// failing without a fallback. Only the post-TLS stall gets one.
    #[test]
    fn pre_tls_timeouts_and_unreachable_hosts_never_fall_back() {
        for msg in [
            crate::embedded_client_error::RdpClientError::Timeout.to_string(),
            "Connection failed: Failed to connect to 192.0.2.1:3389: \
             Connection refused (os error 111)"
                .to_owned(),
            "Connection failed: Failed to connect to nohost.invalid:3389: \
             failed to lookup address information: Name or service not known"
                .to_owned(),
            "Connection failed: RDP handshake timed out after 15s — the remote \
             host may be unreachable through the SSH tunnel or the RDP service \
             is not running"
                .to_owned(),
            "Connection failed: TLS upgrade failed: invalid peer certificate".to_owned(),
        ] {
            let class = classify_rdp_failure(&msg);
            assert_eq!(class, RdpFailureClass::Other, "message: {msg}");
            assert!(!class.warrants_freerdp_fallback(), "message: {msg}");
        }
    }

    #[test]
    fn generic_finalize_and_negotiation_are_not_protocol_markers() {
        for msg in [
            "Connection finalize failed",
            "connect_finalize failed",
            "negotiation failure",
            "NegotiationError",
        ] {
            assert_eq!(classify_rdp_failure(msg), RdpFailureClass::Other);
        }
    }

    #[test]
    fn specific_protocol_and_graphics_markers_still_fall_back() {
        assert_eq!(
            classify_rdp_failure("invalid state (this is a bug)"),
            RdpFailureClass::ProtocolIncompatible
        );
        assert_eq!(
            classify_rdp_failure("NO-FRAME-WATCHDOG: no decodable frame"),
            RdpFailureClass::GraphicsPipeline
        );
    }

    /// Exact message built by the GFX unsupported-codec branch of the GUI event
    /// loop; it must reach the Legacy retry rather than being reported as-is.
    #[test]
    fn unsupported_gfx_codec_retries_without_gfx() {
        let class =
            classify_rdp_failure("gfx unsupported codec: Avc444v2 (5 surface updates dropped)");
        assert_eq!(class, RdpFailureClass::GraphicsPipeline);
        assert!(class.warrants_freerdp_fallback());
        assert!(!class.requires_explicit_consent());
    }

    #[test]
    fn gateway_tunnel_failure_falls_back_to_external_client() {
        // Exact message built by the gateway branch of `establish_connection`.
        let class = classify_rdp_failure("RD Gateway connection failed: WS Upgrade error");
        assert_eq!(class, RdpFailureClass::GatewayFailure);
        assert!(class.warrants_freerdp_fallback());
        assert!(!class.requires_explicit_consent());
    }

    #[test]
    fn gateway_failure_wins_over_wrapped_transport_cause() {
        // The wrapped cause carries transport/TLS wording; the gateway class
        // must still win so the session reaches the external client (#246).
        for msg in [
            "RD Gateway connection failed: TCP connect: connection refused (os error 111)",
            "RD Gateway connection failed: TLS connect: invalid peer certificate",
            "RD Gateway connection failed: custom error: host not found",
        ] {
            let class = classify_rdp_failure(msg);
            assert_eq!(class, RdpFailureClass::GatewayFailure, "message: {msg}");
            assert!(class.warrants_freerdp_fallback(), "message: {msg}");
        }
    }

    #[test]
    fn rejected_gateway_credentials_stay_authentication() {
        // Credential rejection outranks the gateway marker: the external client
        // would be refused by the same account.
        let class = classify_rdp_failure(
            "RD Gateway connection failed: nstatus: Some(NStatusCode(0xc000006d))",
        );
        assert_eq!(class, RdpFailureClass::Authentication);
        assert!(!class.warrants_freerdp_fallback());
    }

    #[test]
    fn direct_transport_failures_are_not_gateway_failures() {
        assert_eq!(
            classify_rdp_failure("Failed to connect to host.internal:3389: connection refused"),
            RdpFailureClass::Other
        );
    }

    #[test]
    fn only_legacy_security_fallback_requires_explicit_consent() {
        assert!(RdpFailureClass::SecurityUnsupported.requires_explicit_consent());
        for class in [
            RdpFailureClass::Authentication,
            RdpFailureClass::GatewayFailure,
            RdpFailureClass::GraphicsPipeline,
            RdpFailureClass::ProtocolIncompatible,
            RdpFailureClass::Other,
        ] {
            assert!(!class.requires_explicit_consent(), "class: {class:?}");
        }
    }

    #[test]
    fn auth_wins_over_non_fallback_and_protocol_markers() {
        let msg = "TLS handshake; unexpected Share Control Pdu; \
                   nstatus: Some(NStatusCode(0xc000006d))";
        assert_eq!(classify_rdp_failure(msg), RdpFailureClass::Authentication);
    }

    /// Verbatim from a user report against a Windows host with RDS licensing.
    const LICENSE_EXCHANGE_FAILURE: &str = "Connection failed: Connection finalize failed: \
         [decode during SERVER_NEW_LICENSE/LicenseExchangeState::UpgradeLicense] decode error \
         [kind: Decode(Error { context: \"<ironrdp_pdu::rdp::server_license::LicenseHeader as \
         ironrdp_core::decode::Decode<'_>>::decode\", kind: InvalidField { field: \
         \"securityHeaderFlags\", reason: \"invalid security header flags\" }, source: None })]";

    #[test]
    fn license_exchange_failure_is_recognised() {
        assert!(is_license_exchange_failure(LICENSE_EXCHANGE_FAILURE));
    }

    #[test]
    fn license_exchange_failure_still_falls_back() {
        // The class must not change: the external client connects to these hosts.
        let class = classify_rdp_failure(LICENSE_EXCHANGE_FAILURE);
        assert_eq!(class, RdpFailureClass::ProtocolIncompatible);
        assert!(class.warrants_freerdp_fallback());
        assert!(!class.requires_explicit_consent());
    }

    #[test]
    fn license_markers_are_classified_without_the_generic_decode_wording() {
        // `"decode error"` is what catches this message today; the licensing
        // markers have to stand on their own so a reworded upstream error still
        // reaches the fallback.
        let class = classify_rdp_failure(
            "[decode during SERVER_NEW_LICENSE/LicenseExchangeState::UpgradeLicense]",
        );
        assert_eq!(class, RdpFailureClass::ProtocolIncompatible);
    }

    #[test]
    fn unrelated_failures_are_not_license_failures() {
        for msg in [
            ISSUE_235,
            CREDSSP_LOGON_FAILURE,
            "unexpected Share Control Pdu",
            "gfx unsupported codec: Avc444v2",
        ] {
            assert!(
                !is_license_exchange_failure(msg),
                "wrongly reported as a licensing failure: {msg}"
            );
        }
    }

    #[test]
    fn a_security_header_failure_outside_licensing_is_not_a_license_failure() {
        // `BasicSecurityHeader` is common to every Standard RDP Security PDU, so
        // its field name alone cannot say the licensing exchange was the phase
        // that failed — but it must still reach the external client.
        let msg = "decode error [kind: Decode(Error { context: \
                   \"<ironrdp_pdu::rdp::BasicSecurityHeader as \
                   ironrdp_core::decode::Decode<'_>>::decode\", kind: InvalidField { field: \
                   \"securityHeaderFlags\", reason: \"invalid security header flags\" } })]";

        assert!(
            !is_license_exchange_failure(msg),
            "a BasicSecurityHeader failure must not be reported as RDS licensing"
        );
        let class = classify_rdp_failure(msg);
        assert_eq!(class, RdpFailureClass::ProtocolIncompatible);
        assert!(class.warrants_freerdp_fallback());
    }

    /// Verbatim from issue #351: the core log line, NLA with Kerberos on.
    const ISSUE_351_NO_KDC_LOG: &str = "CredSSP error_kind=Credssp(Error { error_type: \
         NoAuthenticatingAuthority, description: \"No KDC server found\", nstatus: None })";

    /// Verbatim from issue #351: what the GUI received for the same failure.
    const ISSUE_351_NO_KDC_GUI: &str = "Connection failed: Connection finalize failed: [CredSSP @ \
         /usr/src/packages/BUILD/vendor/ironrdp-async-0.10.0/src/connector.rs:107] CredSSP \
         [kind: Credssp(Error { error_type: NoAuthenticatingAuthority, description: \
         \"No KDC server found\", nstatus: None })]";

    /// Verbatim from issue #351: the core log line, NLA with NTLM, refused because
    /// the account is in Protected Users.
    const ISSUE_351_NTLM_LOG: &str = "CredSSP error_kind=Credssp(Error { error_type: \
         InvalidToken, description: \"CredSSP server returned an error status\", \
         nstatus: Some(NStatusCode(0xc000006e)) })";

    /// Verbatim from issue #351: what the GUI received for the same failure.
    const ISSUE_351_NTLM_GUI: &str = "Authentication failed: Connection finalize failed: \
         [CredSSP @ /usr/src/packages/BUILD/vendor/ironrdp-async-0.10.0/src/connector.rs:107] \
         CredSSP [kind: Credssp(Error { error_type: InvalidToken, description: \
         \"CredSSP server returned an error status\", nstatus: Some(NStatusCode(0xc000006e)) })]";

    /// A CredSSP failure the way `map_connector_error` words a server status.
    fn credssp_status(code: &str) -> String {
        format!(
            "Authentication failed: Connection finalize failed: [CredSSP @ connector.rs:107] \
             CredSSP [kind: Credssp(Error {{ error_type: InvalidToken, description: \
             \"CredSSP server returned an error status\", nstatus: Some(NStatusCode({code})) }})]"
        )
    }

    /// A Kerberos failure the way `map_connector_error` words an sspi error.
    fn sspi_error(error_type: &str, description: &str) -> String {
        format!(
            "Connection failed: Connection finalize failed: [CredSSP @ connector.rs:107] \
             CredSSP [kind: Credssp(Error {{ error_type: {error_type}, description: \
             \"{description}\", nstatus: None }})]"
        )
    }

    #[test]
    fn issue_351_missing_kdc_is_named_and_still_does_not_fall_back() {
        for msg in [ISSUE_351_NO_KDC_LOG, ISSUE_351_NO_KDC_GUI] {
            assert_eq!(
                classify_auth_failure(msg),
                Some(AuthFailureKind::NoKdc { realm: None }),
                "message: {msg}"
            );
        }
        let class = classify_rdp_failure(ISSUE_351_NO_KDC_GUI);
        assert_eq!(class, RdpFailureClass::Other);
        assert!(!class.warrants_freerdp_fallback());
    }

    #[test]
    fn issue_351_ntlm_refusal_is_an_account_restriction() {
        for msg in [ISSUE_351_NTLM_LOG, ISSUE_351_NTLM_GUI] {
            assert_eq!(
                classify_auth_failure(msg),
                Some(AuthFailureKind::AccountRestriction),
                "message: {msg}"
            );
        }
        let class = classify_rdp_failure(ISSUE_351_NTLM_GUI);
        assert_eq!(class, RdpFailureClass::Authentication);
        assert!(!class.warrants_freerdp_fallback());
    }

    #[test]
    fn every_handled_ntstatus_code_has_its_own_kind() {
        for (code, expected) in [
            ("0xc000006d", AuthFailureKind::InvalidCredentials),
            ("0xc000006a", AuthFailureKind::InvalidCredentials),
            ("0xc0000064", AuthFailureKind::InvalidCredentials),
            ("0xc000006e", AuthFailureKind::AccountRestriction),
            ("0xc000006f", AuthFailureKind::LogonRestriction),
            ("0xc0000070", AuthFailureKind::LogonRestriction),
            ("0xc0000071", AuthFailureKind::PasswordExpired),
            ("0xc0000072", AuthFailureKind::AccountDisabled),
            ("0xc000015b", AuthFailureKind::LogonTypeNotGranted),
            ("0xc0000193", AuthFailureKind::AccountExpired),
            ("0xc0000224", AuthFailureKind::PasswordMustChange),
            ("0xc0000234", AuthFailureKind::AccountLockedOut),
        ] {
            let msg = credssp_status(code);
            assert_eq!(
                classify_auth_failure(&msg),
                Some(expected.clone()),
                "code: {code}"
            );
            assert_eq!(
                classify_auth_failure(&msg.to_ascii_uppercase()),
                Some(expected),
                "upper-case code: {code}"
            );
            // The finer kind never changes the fallback decision.
            assert!(
                !classify_rdp_failure(&msg).warrants_freerdp_fallback(),
                "code: {code}"
            );
        }
    }

    #[test]
    fn a_rejected_password_keeps_its_message_and_never_falls_back() {
        let msg = credssp_status("0xc000006d");
        assert_eq!(
            classify_auth_failure(&msg),
            Some(AuthFailureKind::InvalidCredentials)
        );
        assert_eq!(classify_rdp_failure(&msg), RdpFailureClass::Authentication);
        assert_eq!(
            classify_auth_failure("STATUS_LOGON_FAILURE"),
            Some(AuthFailureKind::InvalidCredentials)
        );
    }

    #[test]
    fn kerberos_errors_have_their_own_kinds() {
        for (error_type, description, expected) in [
            (
                "TimeSkew",
                "clock skew too great",
                AuthFailureKind::ClockSkew,
            ),
            (
                "TimeSkew",
                "clock skew too great. Additional error text: \\\"skew\\\"",
                AuthFailureKind::ClockSkew,
            ),
            (
                "UnknownCredentials",
                "server not found in Kerberos database",
                AuthFailureKind::UnknownServerPrincipal,
            ),
            (
                "UnknownCredentials",
                "client not found in Kerberos database",
                AuthFailureKind::InvalidCredentials,
            ),
            (
                "KdcInvalidRequest",
                "pre-authentication information was invalid",
                AuthFailureKind::InvalidCredentials,
            ),
            (
                "InvalidParameter",
                "password has expired; change password to reset",
                AuthFailureKind::PasswordExpired,
            ),
            (
                "UnknownCredentials",
                "clients credentials have been revoked",
                AuthFailureKind::AccountRevoked,
            ),
            (
                "KdcInvalidRequest",
                "KDC policy rejects request",
                AuthFailureKind::LogonRestriction,
            ),
        ] {
            let msg = sspi_error(error_type, description);
            assert_eq!(
                classify_auth_failure(&msg),
                Some(expected),
                "description: {description}"
            );
        }
    }

    #[test]
    fn a_cross_realm_kdc_miss_names_the_realm() {
        let msg = sspi_error(
            "NoAuthenticatingAuthority",
            "No KDC server found for realm `DEV.AAG.LOCAL`",
        );
        assert_eq!(
            classify_auth_failure(&msg),
            Some(AuthFailureKind::NoKdc {
                realm: Some("DEV.AAG.LOCAL".to_owned()),
            })
        );
    }

    #[test]
    fn an_unreachable_kdc_is_not_reported_as_a_bad_password() {
        for msg in [
            "Connection failed: Connection finalize failed: \
             [failed to send KDC request over TCP @ \
             /usr/src/packages/BUILD/vendor/ironrdp-tokio-0.10.0/src/reqwest.rs:47] \
             custom error [kind: Custom]",
            "Connection failed: Connection finalize failed: [KdcProxy @ reqwest.rs:113] \
             custom error [kind: Custom]",
            "Connection failed: Connection finalize failed: [failed to send UDP request @ \
             reqwest.rs:91] custom error [kind: Custom]",
        ] {
            assert_eq!(
                classify_auth_failure(msg),
                Some(AuthFailureKind::KdcUnreachable),
                "message: {msg}"
            );
            assert!(
                !classify_rdp_failure(msg).warrants_freerdp_fallback(),
                "message: {msg}"
            );
        }
    }

    #[test]
    fn an_early_authorization_refusal_is_a_missing_logon_right() {
        let msg = "Authentication failed: Connection finalize failed: [CredSSP @ connector.rs:166] \
                   access denied [kind: AccessDenied]";
        assert_eq!(
            classify_auth_failure(msg),
            Some(AuthFailureKind::LogonTypeNotGranted)
        );
    }

    #[test]
    fn other_credssp_failures_are_generic() {
        for msg in [
            sspi_error("InternalError", "unexpected status: CompleteNeeded"),
            "CredSSP [kind: Credssp(Error { error_type: OutOfSequence })]".to_owned(),
            "Credssp failure".to_owned(),
        ] {
            assert_eq!(
                classify_auth_failure(&msg),
                Some(AuthFailureKind::Generic),
                "message: {msg}"
            );
        }
    }

    #[test]
    fn failures_outside_the_sign_in_have_no_auth_kind() {
        for msg in [
            ISSUE_235,
            "Failed to connect to host.internal:3389: connection refused",
            "TLS upgrade failed: invalid peer certificate",
            "Operation timed out",
            LICENSE_EXCHANGE_FAILURE,
        ] {
            assert_eq!(classify_auth_failure(msg), None, "message: {msg}");
        }
    }
}
