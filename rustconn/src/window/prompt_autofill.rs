//! Prompt-driven credential auto-fill for VTE sessions.
//!
//! SSH authenticates before the terminal exists, so its only interactive step
//! is a password prompt. Telnet and a serial console have no authentication
//! protocol at all: the device prints a prompt and expects the account name
//! and the password to be typed, in that order, in the terminal itself.
//!
//! Both cases are the same job — watch the line under the cursor, recognize a
//! prompt, type the matching credential exactly once — so they share this
//! module. Recognition is delegated to
//! [`LoginPromptMatcher`], which is
//! GUI-free and unit-tested, and can be overridden per connection or per
//! group for devices with unusual wording (issue #254).
//!
//! Nothing is ever typed twice: each stage of the sequence fires once
//! ([`LoginSequence`]). A device that re-prompts after a rejected credential
//! is left to the user, so a wrong stored password cannot walk an account into
//! a lockout; the server's echo of the account name on the prompt line just
//! answered is not a re-prompt.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::glib;
use rustconn_core::{LoginAction, LoginPromptMatcher, LoginSequence};
use secrecy::SecretString;
use uuid::Uuid;

use super::protocols::SharedNotebook;

/// How often the terminal is re-examined while waiting for a prompt.
///
/// VTE does not reliably emit `contents-changed` for a no-echo prompt written
/// with cursor-positioning escapes and no trailing newline (issue #194), so the
/// signals alone can miss it. 150 ms balances responsiveness against wake-ups.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// How long the terminal may stay *idle* before the watcher gives up.
///
/// Measured from the last terminal activity, not from spawn (issue #301). A
/// jump-host connection reaches the target's password prompt only after its
/// `ProxyCommand` chain has authenticated the bastion, which can take longer
/// than any fixed post-spawn budget: the reporter's target prompt arrived after
/// the `ssh -o ProxyCommand …` step, well past a wall-clock deadline that
/// started at spawn, so the watcher had already given up and the session hung
/// at `password:`. Restarting the clock on every `contents-changed` /
/// `cursor-moved` means the wait only expires once the terminal has been silent
/// this long — the connection has genuinely stalled — rather than while a slow
/// handshake or a long ProxyCommand is still making progress. It still covers a
/// device that spends seconds on a login banner, because the banner is output
/// and so keeps the clock alive. Overridden by
/// [`rustconn_core::models::AutomationConfig::login_timeout_secs`] when set.
const DEFAULT_AUTOFILL_DEADLINE: Duration = Duration::from_secs(10);

/// Absolute ceiling: no login watcher survives past this, regardless of
/// activity, to avoid a permanent wake-up on a device that prints a heartbeat.
const ABSOLUTE_TIMEOUT: Duration = Duration::from_mins(2);

/// Credentials to type, and how to recognize the prompts asking for them.
pub(crate) struct LoginAutofill {
    /// Account name to send at the username prompt, if the device asks for one.
    pub username: Option<String>,
    /// Password to send at the password prompt.
    pub password: Option<SecretString>,
    /// Prompt recognition, possibly customized for the device.
    pub matcher: LoginPromptMatcher,
    /// Protocol name for log fields (`"ssh"`, `"telnet"`, `"serial"`).
    pub protocol: &'static str,
    /// How long to wait for a prompt before giving up.
    ///
    /// `None` means use [`DEFAULT_AUTOFILL_DEADLINE`].
    pub deadline_secs: Option<u32>,
}

impl LoginAutofill {
    /// Returns `true` when there is nothing to type.
    pub(crate) const fn is_empty(&self) -> bool {
        self.username.is_none() && self.password.is_none()
    }
}

/// Starts watching `session_id` for login prompts and types the credentials.
///
/// Subscribes to both `contents-changed` and `cursor-moved` and additionally
/// polls every [`POLL_INTERVAL`], because either signal can fire before the
/// prompt glyphs reach the grid and a no-echo prompt may not produce another
/// signal afterwards (issue #194). All three paths run the same one-shot step,
/// so nothing is sent twice regardless of which observes the prompt first.
///
/// Does nothing when `spec` carries no credentials.
pub(crate) fn install_login_autofill(
    notebook: &SharedNotebook,
    session_id: Uuid,
    spec: LoginAutofill,
) {
    if spec.is_empty() {
        return;
    }

    let LoginAutofill {
        username,
        password,
        matcher,
        protocol,
        deadline_secs,
    } = spec;

    let send_username = username.is_some();
    // The sequencing (which prompt gets what, once, and when to stop) lives in
    // `LoginSequence`, GUI-free and unit-tested in rustconn-core.
    let sequence = Rc::new(RefCell::new(LoginSequence::new(
        username,
        password.is_some(),
    )));

    let deadline = deadline_secs
        .map(|s| Duration::from_secs(u64::from(s)))
        .unwrap_or(DEFAULT_AUTOFILL_DEADLINE);

    tracing::info!(
        protocol,
        %session_id,
        send_username,
        send_password = password.is_some(),
        custom_prompts = !matcher.is_default(),
        "Credentials available; will auto-fill on login prompt"
    );

    // One detect-and-send step, shared by the two signals and the timer. The
    // sequence returns each action at most once, so nothing fires twice no
    // matter who calls.
    let step = {
        let notebook = notebook.clone();
        let sequence = sequence.clone();
        Rc::new(move || {
            if sequence.borrow().is_done() {
                return;
            }
            let Some(line) = notebook.get_cursor_line_text(session_id) else {
                return;
            };
            // Decide under a short borrow and release it before typing, so a
            // signal VTE emits while we feed the child cannot re-enter a
            // borrowed sequence.
            let (action, user) = {
                let mut seq = sequence.borrow_mut();
                let action = seq.step(&matcher, &line);
                (action, seq.username().map(str::to_owned))
            };

            match action {
                Some(LoginAction::SendUsername) => {
                    if let Some(user) = user {
                        notebook.send_text_to_session(session_id, &format!("{user}\n"));
                        tracing::info!(protocol, "Username prompt detected; account name sent");
                    }
                }
                // The device asked for the password, after the name or straight
                // away (the account name may have travelled with the command
                // line, as `telnet -l user` does).
                Some(LoginAction::SendPassword) => {
                    if let Some(ref secret) = password {
                        use secrecy::ExposeSecret;
                        // Zeroizing so the plaintext is wiped as soon as VTE has
                        // it, rather than lingering in a dropped String.
                        let input =
                            zeroize::Zeroizing::new(format!("{}\n", secret.expose_secret()));
                        notebook.send_text_to_session(session_id, &input);
                        tracing::info!(protocol, "Password prompt detected; password sent");
                    }
                }
                // A new login prompt while waiting for the password means the
                // device rejected the credentials and started over. Stop here
                // and let the user decide — retrying automatically is how an
                // account gets locked out. The echo of our own answer on the
                // prompt line does not count (`LoginSequence::step`).
                Some(LoginAction::Stop) => {
                    tracing::info!(
                        protocol,
                        "Login prompt repeated after credentials were sent; auto-fill stopped"
                    );
                }
                None => {}
            }
        })
    };

    // The deadline is measured from the last sign of terminal activity, not from
    // here (issue #301). Every `contents-changed`/`cursor-moved` refreshes it,
    // so the watcher only expires after `deadline` of genuine silence — the
    // connection has stalled — rather than while a slow handshake or a long
    // `ProxyCommand` chain is still making progress toward the prompt.
    let last_activity = Rc::new(Cell::new(Instant::now()));
    let started = Instant::now();

    // Polling safety net. Self-cancels on completion, on session close, and once
    // the terminal has been idle for `deadline`. `Instant` is fine for this
    // in-process window; it does not advance across suspend, which for a login
    // in flight is moot.
    {
        let step = step.clone();
        let sequence = sequence.clone();
        let notebook = notebook.clone();
        let last_activity = last_activity.clone();
        glib::timeout_add_local(POLL_INTERVAL, move || {
            if sequence.borrow().is_done() {
                return glib::ControlFlow::Break;
            }
            // The tab was closed or the child exited — stop burning wake-ups.
            if notebook.get_terminal(session_id).is_none() {
                return glib::ControlFlow::Break;
            }
            // Absolute ceiling: a device printing periodic output resets the
            // idle timer indefinitely; this prevents the watcher from running
            // forever.
            if started.elapsed() >= ABSOLUTE_TIMEOUT {
                tracing::debug!(
                    protocol,
                    %session_id,
                    "Absolute timeout; stopping login auto-fill"
                );
                sequence.borrow_mut().finish();
                return glib::ControlFlow::Break;
            }
            if last_activity.get().elapsed() >= deadline {
                tracing::debug!(
                    protocol,
                    %session_id,
                    "Terminal idle past the auto-fill window with no login prompt; giving up"
                );
                sequence.borrow_mut().finish();
                return glib::ControlFlow::Break;
            }
            step();
            if sequence.borrow().is_done() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    // contents-changed: fires for most terminal output.
    {
        let on_contents_changed = step.clone();
        let last_activity = last_activity.clone();
        notebook.connect_contents_changed(session_id, move || {
            last_activity.set(Instant::now());
            on_contents_changed();
        });
    }

    // cursor-moved: fires for prompts drawn with cursor-positioning escapes and
    // no trailing newline, which is exactly what a password prompt looks like.
    notebook.connect_cursor_moved(session_id, move || {
        last_activity.set(Instant::now());
        step();
    });
}
