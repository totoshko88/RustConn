//! What a session's exit means: a refused start-up or a dropped connection.
//!
//! The GUI decides from this whether to forget a cached credential and
//! whether automatic reconnection may run. Both decisions read the same
//! inputs: the exit code, the protocol, how long the session lived and the
//! last lines left on its screen.

use super::cloud_login::output_tail;

/// Longest a session may have run, in seconds, for its failure to be read as
/// a refused start-up (rejected credentials or an expired sign-in).
///
/// A client refusing credentials does so while it starts, before any remote
/// shell exists. A session that fails later ends with the remote host's output
/// on screen, and ordinary remote text (`docker push` saying "not logged in",
/// an `ls` saying "Permission denied", a log line about an expired JWT) would
/// otherwise turn a dropped network into an authentication failure and switch
/// off automatic reconnection. Sixty seconds leaves room for a slow token
/// refresh and an MFA prompt. If credentials do expire mid-session, the
/// automatic reconnect then fails at start-up and is recognised there.
pub const STARTUP_FAILURE_WINDOW_SECS: i64 = 60;

/// ssh's own messages for a connection lost to the network.
///
/// OpenSSH words the keepalive timeout `Timeout, server <host> not
/// responding.`, so the marker stops before the host name.
const NETWORK_MARKERS: &[&str] = &[
    "timed out",
    "timeout, server",
    "connection reset",
    "broken pipe",
];

/// ssh's own messages for rejected credentials.
const AUTH_MARKERS: &[&str] = &[
    "permission denied",
    "authentication failed",
    "too many authentication failures",
];

/// Returns whether an ssh exit should be treated as an authentication failure.
///
/// OpenSSH exits 255 for any error of its own, so a rejected key and a
/// `ServerAliveInterval`/`ServerAliveCountMax` timeout look alike by exit code.
/// Only `protocol == "ssh"` with exit code 255 can be an authentication
/// failure. For those, the last lines of `screen_text` are read from the
/// bottom up and the first known message decides: a network message
/// (`timed out`, `Timeout, server not responding`, `Connection reset`,
/// `Broken pipe`) means no, an authentication message (`Permission denied`,
/// `Authentication failed`, `Too many authentication failures`) means yes.
/// The last line wins because ssh prints its own reason last, while earlier
/// lines of a long session are the remote host's output. With no known
/// message, the exit counts as an authentication failure only when the
/// session lived at most [`STARTUP_FAILURE_WINDOW_SECS`].
#[must_use]
pub fn ssh_exit_is_auth_failure(
    exit_code: i32,
    protocol: &str,
    lifetime_secs: i64,
    screen_text: &str,
) -> bool {
    if protocol != "ssh" || exit_code != 255 {
        return false;
    }
    let tail = output_tail(screen_text);
    for line in tail.lines().rev() {
        if NETWORK_MARKERS.iter().any(|m| line.contains(m)) {
            return false;
        }
        if AUTH_MARKERS.iter().any(|m| line.contains(m)) {
            return true;
        }
    }
    lifetime_secs <= STARTUP_FAILURE_WINDOW_SECS
}

/// evil-winrm's message for credentials the WinRM service refused.
const WINRM_AUTH_MARKER: &str = "winrmauthorizationerror";

/// Returns whether a WinRM client exit was the service refusing the sign-in.
///
/// evil-winrm prints `WinRM::WinRMAuthorizationError` and exits 1 when the
/// account is rejected, which includes a local administrator stripped of its
/// rights by UAC remote restrictions. The host still answers the port probe,
/// so without this verdict automatic reconnection retried the same refused
/// sign-in in a loop. Like [`ssh_exit_is_auth_failure`], only a failure within
/// [`STARTUP_FAILURE_WINDOW_SECS`] counts: later output is the remote shell's.
#[must_use]
pub fn winrm_exit_is_auth_failure(
    exit_code: i32,
    protocol: &str,
    lifetime_secs: i64,
    screen_text: &str,
) -> bool {
    protocol == "winrm"
        && exit_code != 0
        && lifetime_secs <= STARTUP_FAILURE_WINDOW_SECS
        && output_tail(screen_text).contains(WINRM_AUTH_MARKER)
}

#[cfg(test)]
mod tests {
    #[test]
    fn winrm_authorization_error_is_an_auth_failure() {
        let screen = "Info: Establishing connection to remote endpoint\n\
             Error: An error of type WinRM::WinRMAuthorizationError happened, message is \
             WinRM::WinRMAuthorizationError\n";
        assert!(winrm_exit_is_auth_failure(1, "winrm", 21, screen));
        assert!(!winrm_exit_is_auth_failure(1, "winrm", 600, screen));
        assert!(!winrm_exit_is_auth_failure(0, "winrm", 21, screen));
        assert!(!winrm_exit_is_auth_failure(1, "ssh", 21, screen));
        assert!(!winrm_exit_is_auth_failure(
            1,
            "winrm",
            21,
            "Error: Connection refused"
        ));
    }

    use super::*;

    #[test]
    fn other_protocols_and_exit_codes_are_never_auth_failures() {
        assert!(!ssh_exit_is_auth_failure(255, "zerotrust", 1, ""));
        assert!(!ssh_exit_is_auth_failure(1, "ssh", 1, "Permission denied"));
        assert!(!ssh_exit_is_auth_failure(0, "ssh", 1, ""));
    }

    #[test]
    fn a_fresh_255_with_no_message_is_an_auth_failure() {
        assert!(ssh_exit_is_auth_failure(255, "ssh", 3, ""));
    }

    #[test]
    fn the_window_boundary_is_inclusive() {
        assert!(ssh_exit_is_auth_failure(
            255,
            "ssh",
            STARTUP_FAILURE_WINDOW_SECS,
            ""
        ));
        assert!(!ssh_exit_is_auth_failure(
            255,
            "ssh",
            STARTUP_FAILURE_WINDOW_SECS + 1,
            ""
        ));
    }

    #[test]
    fn a_keepalive_timeout_is_not_an_auth_failure_even_when_fresh() {
        let text = "user@host:~$ \nTimeout, server host not responding.\n";
        assert!(!ssh_exit_is_auth_failure(255, "ssh", 5, text));
        let text = "Timeout, server not responding.";
        assert!(!ssh_exit_is_auth_failure(255, "ssh", 5, text));
        let text = "ssh: connect to host h port 22: Connection timed out";
        assert!(!ssh_exit_is_auth_failure(255, "ssh", 5, text));
    }

    #[test]
    fn a_rejected_key_is_an_auth_failure_whatever_the_lifetime() {
        let text = "user@host: Permission denied (publickey).";
        assert!(ssh_exit_is_auth_failure(255, "ssh", 300, text));
        let text = "Received disconnect from 10.0.0.1 port 22:2: Too many authentication failures";
        assert!(ssh_exit_is_auth_failure(255, "ssh", 300, text));
    }

    #[test]
    fn the_last_message_wins() {
        let text = "$ ls /root\nls: cannot open directory '/root': Permission denied\n\
                    $ \nclient_loop: send disconnect: Broken pipe\n";
        assert!(!ssh_exit_is_auth_failure(255, "ssh", 5, text));
        let text = "read: Connection reset by peer\nuser@host: Permission denied (publickey).";
        assert!(ssh_exit_is_auth_failure(255, "ssh", 300, text));
    }

    #[test]
    fn markers_are_case_insensitive() {
        assert!(!ssh_exit_is_auth_failure(255, "ssh", 5, "CONNECTION RESET"));
        assert!(ssh_exit_is_auth_failure(
            255,
            "ssh",
            300,
            "AUTHENTICATION FAILED"
        ));
    }
}
