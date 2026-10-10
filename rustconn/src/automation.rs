//! Automation manager for terminal sessions
//!
//! This module provides "Expect"-like functionality for terminal sessions,
//! allowing automatic responses to specific text patterns in the output.
//! Pattern matching logic is delegated to `ExpectEngine` from `rustconn-core`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::glib;
use gtk4::glib::ControlFlow;
use rustconn_core::automation::{ExpectEngine, ExpectRule};
use uuid::Uuid;
use vte4::prelude::*;
use vte4::{Format, Terminal};
use zeroize::{Zeroize, Zeroizing};

/// How many times one credential-carrying rule may answer in a single session.
///
/// `sudo` itself allows three tries before it gives up, and `pam_faillock`
/// commonly locks an account at three failures, so a rule that has already fed
/// the password three times is either done or feeding a credential that does not
/// work. Continuing past this cannot help and can lock the account out. The cap
/// applies only to rules that resolved `${password}` — a hand-written repeating
/// rule that answers a pager or a menu has no credential to spend and is left
/// unbounded.
const MAX_CREDENTIAL_FIRES: u32 = 3;

/// What a rule last answered, so it cannot answer the same prompt twice.
struct AnsweredPrompt {
    /// The active prompt line this rule answered, trimmed.
    ///
    /// `None` once that prompt has left the active line — the latch is open
    /// again, while `count` deliberately survives. That split is the whole point:
    /// the latch is per prompt occurrence, the tally is per session, so a genuine
    /// retry is allowed but an unbounded loop of them is not.
    ///
    /// The latch clears as soon as the active line reads as anything else, which
    /// is what still lets a genuine retry through: `sudo` prints
    /// `Sorry, try again.` before its next prompt, so the active line
    /// demonstrably changed in between. Without the latch a non-one-shot rule
    /// re-fires on the very next redraw, because the prompt it just answered is
    /// still the last non-empty line — a scroll, a status-bar repaint or a clock
    /// tick is enough.
    line: Option<String>,
    /// How many times this rule has fired in this session.
    count: u32,
}

/// Shared state for automation engine
struct AutomationState {
    /// The expect engine that handles pattern matching and priority sorting
    engine: ExpectEngine,
    /// Per-rule creation timestamps for timeout tracking
    created_at: HashMap<Uuid, Instant>,
    /// Last content to detect changes; scrubbed because terminals may echo input.
    last_content: Zeroizing<String>,
    /// Counter for polling cycles
    poll_count: u32,
    /// Rules whose response resolved `${password}`, so they carry a credential.
    ///
    /// Kept beside the engine rather than on [`ExpectRule`] because it is a
    /// property of *this* resolution, not of the persisted rule: the same stored
    /// rule carries a secret only when a variable actually resolved into it.
    credential_rules: HashSet<Uuid>,
    /// Per-rule latch and fire tally, keyed by rule id.
    answered: HashMap<Uuid, AnsweredPrompt>,
}

impl Drop for AutomationState {
    fn drop(&mut self) {
        // Scrub any remaining rule responses that may contain credentials
        // resolved from the vault (issue #257). Without this, a session that
        // closes before all one-shot rules fire leaves passwords in freed memory.
        self.engine.zeroize_responses();
    }
}

/// A connection-resolved rule whose response is scrubbed if setup is abandoned.
pub(crate) struct PreparedExpectRule {
    id: Uuid,
    pattern: String,
    response: Zeroizing<String>,
    priority: i32,
    timeout_ms: Option<u32>,
    one_shot: bool,
    delay_ms: Option<u32>,
    /// Whether the template referenced `${password}` before substitution.
    ///
    /// Recorded from the template rather than sniffed from the resolved text,
    /// which cannot be inspected without reading the credential back out.
    carries_credential: bool,
}

impl PreparedExpectRule {
    fn into_expect_rule(mut self) -> ExpectRule {
        ExpectRule {
            id: self.id,
            pattern: std::mem::take(&mut self.pattern),
            response: std::mem::take(&mut *self.response),
            priority: self.priority,
            timeout_ms: self.timeout_ms,
            enabled: true,
            one_shot: self.one_shot,
            delay_ms: self.delay_ms,
        }
    }
}

/// A matched rule's response, waiting to be written to the session.
///
/// Carried out of the borrow on the shared state so nothing is held while the
/// terminal is written to, and so a delayed response can be moved into a timer.
struct PendingResponse {
    rule_id: Uuid,
    /// Scrubbed on drop — it may be a resolved credential (issue #257).
    response: Zeroizing<String>,
    one_shot: bool,
    delay_ms: Option<u32>,
    /// The trimmed prompt line that justified this response.
    ///
    /// Not a secret — it is the prompt, not the answer — and it is what a delayed
    /// send re-checks the grid against before writing.
    expect_line: String,
}

/// Manages automation for a terminal session
///
/// The `state` field holds the shared automation state that is accessed by the
/// polling timer. Even though it's not directly read after construction, it must
/// be kept alive to prevent the `Rc` from being dropped while the timer is active.
pub struct AutomationSession {
    /// Shared state accessed by the polling timer callback.
    /// Kept alive to maintain the `Rc` reference count.
    state: Rc<RefCell<AutomationState>>,
    /// Polling source, removed when a session is replaced or its tab closes.
    timer: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Drop for AutomationSession {
    fn drop(&mut self) {
        if let Some(source_id) = self.timer.borrow_mut().take() {
            source_id.remove();
        }
        self.state.borrow_mut().engine.clear();
    }
}

impl AutomationSession {
    /// Returns the number of remaining rules
    #[must_use]
    pub fn remaining_triggers(&self) -> usize {
        self.state.borrow().engine.len()
    }

    /// Returns whether all rules have been processed
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.state.borrow().engine.is_empty()
    }

    /// Creates a new automation session from pre-resolved expect rules
    ///
    /// Rules should already have variable substitution applied to their responses.
    pub(crate) fn new(terminal: Terminal, rules: Vec<PreparedExpectRule>) -> Self {
        tracing::info!("AutomationSession: Created with {} rules", rules.len());
        for rule in &rules {
            // The response is deliberately NOT logged, only its length: a rule
            // may answer a credential prompt, and `tracing` output is not run
            // through the redaction that session logs get (`sanitize_output`).
            tracing::info!(
                "AutomationSession: Rule id={}, pattern='{}', response_len={}, priority={}, one_shot={}",
                rule.id,
                rule.pattern,
                rule.response.len(),
                rule.priority,
                rule.one_shot,
            );
        }

        let now = Instant::now();
        let mut created_at = HashMap::new();
        for rule in &rules {
            created_at.insert(rule.id, now);
        }

        // Collected before `into_expect_rule` consumes the prepared rules: the
        // flag does not survive into `ExpectRule`, which is the persisted type.
        let credential_rules: HashSet<Uuid> = rules
            .iter()
            .filter(|rule| rule.carries_credential)
            .map(|rule| rule.id)
            .collect();

        // Move resolved responses into the core engine only after all setup
        // bookkeeping succeeds. Abandoned prepared rules zeroize on drop.
        let rules = rules
            .into_iter()
            .map(PreparedExpectRule::into_expect_rule)
            .collect();
        let engine = match ExpectEngine::from_rules(rules) {
            Ok(engine) => engine,
            Err(e) => {
                tracing::error!("AutomationSession: Failed to build engine: {e}");
                ExpectEngine::new()
            }
        };

        let state = Rc::new(RefCell::new(AutomationState {
            engine,
            created_at,
            last_content: Zeroizing::new(String::new()),
            poll_count: 0,
            credential_rules,
            answered: HashMap::new(),
        }));

        // Start polling timer to check terminal content. The source ID is kept
        // with the session so reconnect/tab teardown cancels the old callback
        // before it can send a resolved response into a replacement process.
        let state_clone = Rc::clone(&state);
        let terminal_weak = terminal.downgrade();
        let timer = Rc::new(RefCell::new(None));
        let timer_for_callback = Rc::clone(&timer);

        let source_id = glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(terminal) = terminal_weak.upgrade() else {
                timer_for_callback.borrow_mut().take();
                return ControlFlow::Break;
            };

            Self::check_terminal_content(&terminal, &state_clone);

            // Continue polling while we have rules
            if state_clone.borrow().engine.is_empty() {
                tracing::debug!("AutomationSession: No more rules, stopping polling");
                timer_for_callback.borrow_mut().take();
                ControlFlow::Break
            } else {
                ControlFlow::Continue
            }
        });
        *timer.borrow_mut() = Some(source_id);

        Self { state, timer }
    }

    /// Process escape sequences in response string
    fn process_escapes(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.peek() {
                    Some('n') => {
                        result.push('\n');
                        chars.next();
                    }
                    Some('r') => {
                        result.push('\r');
                        chars.next();
                    }
                    Some('t') => {
                        result.push('\t');
                        chars.next();
                    }
                    Some('\\') => {
                        result.push('\\');
                        chars.next();
                    }
                    _ => result.push(c),
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    /// Reads the terminal's whole visible grid as text.
    ///
    /// Scrubbed on drop: a terminal echoes what was typed into it, so the grid
    /// can hold a credential a moment after one was sent.
    fn visible_text(terminal: &Terminal) -> Zeroizing<String> {
        let row_count = terminal.row_count();
        Zeroizing::new(
            if let (Some(text), _) = terminal.text_range_format(
                Format::Text,
                0,             // start row
                0,             // start col
                row_count - 1, // end row (last visible row)
                -1,            // end col (-1 = end of line)
            ) {
                text.to_string()
            } else {
                String::new()
            },
        )
    }

    /// Index of the active prompt line: the last line with anything on it.
    ///
    /// A forward scan, because `str::lines` is not a double-ended iterator.
    /// Returns `None` for an empty or all-blank grid, which fails closed — no
    /// rule fires rather than every rule firing against nothing.
    fn active_line_index(content: &str) -> Option<usize> {
        let mut active = None;
        for (index, line) in content.lines().enumerate() {
            if !line.trim().is_empty() {
                active = Some(index);
            }
        }
        active
    }

    /// The text of the active prompt line, trimmed.
    fn active_line_text(content: &str) -> Option<&str> {
        let index = Self::active_line_index(content)?;
        content.lines().nth(index).map(str::trim)
    }

    /// Whether a rule that matched line `index` may fire.
    ///
    /// A rule that can fire repeatedly is restricted to the active prompt line,
    /// because the whole visible grid is rescanned on every change and a prompt
    /// that has already been answered stays on screen. Without this a non-one-shot
    /// sudo rule re-fires on the next screen update and types the password as a
    /// shell command. One-shot rules keep scanning everything: they are removed
    /// the first time they match, so they cannot repeat.
    fn should_fire(one_shot: bool, index: usize, active_line: Option<usize>) -> bool {
        one_shot || active_line == Some(index)
    }

    fn check_terminal_content(terminal: &Terminal, state: &Rc<RefCell<AutomationState>>) {
        let mut state_ref = state.borrow_mut();

        // Skip if no rules left
        if state_ref.engine.is_empty() {
            return;
        }

        state_ref.poll_count += 1;

        // Remove expired rules (check every 50 polls ≈ 5 seconds to avoid
        // cloning created_at HashMap on every 100ms tick)
        if state_ref.poll_count.is_multiple_of(50) {
            let now = Instant::now();
            let created_at_snapshot = state_ref.created_at.clone();
            let expired_count = state_ref
                .engine
                .remove_expired_individual(now, &created_at_snapshot);
            if expired_count > 0 {
                // Clean up created_at entries for removed rules
                let active_ids: std::collections::HashSet<Uuid> =
                    state_ref.engine.rules().iter().map(|r| r.id).collect();
                state_ref.created_at.retain(|id, _| active_ids.contains(id));
                tracing::info!(
                    "AutomationSession: Removed {} expired rules, {} remaining",
                    expired_count,
                    state_ref.engine.len()
                );
            }
        }

        if state_ref.engine.is_empty() {
            return;
        }

        let content = Self::visible_text(terminal);

        // Check if content changed
        let content_changed = content != state_ref.last_content;

        // Log periodically
        if state_ref.poll_count.is_multiple_of(500) {
            let (cursor_col, cursor_row) = terminal.cursor_position();
            tracing::debug!(
                "AutomationSession: Poll #{}, cursor at ({}, {}), content len {}",
                state_ref.poll_count,
                cursor_row,
                cursor_col,
                content.len()
            );
        }

        // Skip pattern matching if content hasn't changed
        if !content_changed {
            return;
        }

        state_ref.last_content = content.clone();

        let active_line = Self::active_line_index(&content);
        // Owned, because the latch below is updated while `content` is still
        // borrowed by the match loop.
        let active_text: Option<String> =
            Self::active_line_text(&content).map(std::string::ToString::to_string);

        // Open the latch of any rule whose answered prompt is no longer the
        // active line: that prompt went away, so the next one is a new occurrence
        // and answering it again is correct. This is what keeps a genuine `sudo`
        // retry working while a redraw of the same prompt stays blocked. The fire
        // tally is left alone — see `AnsweredPrompt::line`.
        for answered in state_ref.answered.values_mut() {
            if answered.line.as_deref() != active_text.as_deref() {
                answered.line = None;
            }
        }

        // Responses are wrapped in `Zeroizing` so that credentials resolved into
        // them (issue #257) are scrubbed from memory as soon as they are sent.
        let mut matches: Vec<PendingResponse> = Vec::new();

        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }

            // Use engine's match_line which handles trimming and priority
            if let Some(compiled) = state_ref.engine.match_line(line) {
                let rule = &compiled.rule;

                // Skip if we already matched this rule in this cycle
                if matches.iter().any(|pending| pending.rule_id == rule.id) {
                    continue;
                }

                if !Self::should_fire(rule.one_shot, index, active_line) {
                    continue;
                }

                // The latch: this rule already answered the prompt that is still
                // the active line, so this is a redraw and not a new prompt.
                if let Some(answered) = state_ref.answered.get(&rule.id)
                    && answered.line.is_some()
                {
                    continue;
                }

                // Spending a credential is capped; see `MAX_CREDENTIAL_FIRES`.
                let carries_credential = state_ref.credential_rules.contains(&rule.id);
                if carries_credential
                    && let Some(answered) = state_ref.answered.get(&rule.id)
                    && MAX_CREDENTIAL_FIRES <= answered.count
                {
                    tracing::warn!(
                        rule_id = %rule.id,
                        fires = answered.count,
                        cap = MAX_CREDENTIAL_FIRES,
                        "Rule has already answered its cap of credential prompts; refusing \
                         further injection so the account is not locked out"
                    );
                    continue;
                }

                tracing::info!(
                    rule_id = %rule.id,
                    pattern = %rule.pattern,
                    matched_line_len = line.trim().len(),
                    delay_ms = ?rule.delay_ms,
                    "AutomationSession matched expect rule"
                );

                // Escapes were already expanded by `prepare_rules_from_config`,
                // before substitution — doing it here as well would reinterpret
                // backslashes that came out of a resolved variable.
                let response = zeroize::Zeroizing::new(rule.response.clone());
                // Length only — see the note in `new()`: a response may carry a
                // password, and tracing output is not redacted.
                tracing::info!(
                    "AutomationSession: Sending response for rule id={} ({} bytes)",
                    rule.id,
                    response.len()
                );

                let rule_id = rule.id;
                let one_shot = rule.one_shot;
                let delay_ms = rule.delay_ms;
                // The line that justified this response. A delayed send re-reads
                // the grid and refuses if this is no longer the active line.
                let expect_line = line.trim().to_string();

                // Close the latch and count the fire before the response leaves,
                // so a rule cannot be re-armed by a redraw that lands while a
                // delay timer is still pending.
                let answered = state_ref.answered.entry(rule_id).or_insert(AnsweredPrompt {
                    line: None,
                    count: 0,
                });
                answered.line = Some(expect_line.clone());
                answered.count = answered.count.saturating_add(1);

                matches.push(PendingResponse {
                    rule_id,
                    response,
                    one_shot,
                    delay_ms,
                    expect_line,
                });
            }
        }

        // Remove one-shot rules that matched and zeroize their stored response
        // so the credential does not linger in the freed allocation.
        for pending in &matches {
            if pending.one_shot {
                if let Some(rule) = state_ref.engine.get_rule_mut(pending.rule_id) {
                    rule.response.zeroize();
                }
                state_ref.engine.remove_by_id(pending.rule_id);
                state_ref.created_at.remove(&pending.rule_id);
            }
        }

        // Drop borrow before sending
        drop(state_ref);

        // Send responses — `Zeroizing` scrubs the String on drop, whether that
        // happens here or after a delay timer has run.
        for pending in matches {
            match pending.delay_ms {
                None => crate::terminal::script_recorder::suppress_recording(|| {
                    terminal.feed_child(pending.response.as_bytes());
                }),
                Some(delay_ms) => {
                    // A weak handle, so a tab closed inside the delay window
                    // drops the response instead of writing to a dead widget.
                    let terminal_weak = terminal.downgrade();
                    let response = pending.response;
                    let expect_line = pending.expect_line;
                    let rule_id = pending.rule_id;
                    glib::timeout_add_local_once(
                        Duration::from_millis(u64::from(delay_ms)),
                        move || {
                            let Some(terminal) = terminal_weak.upgrade() else {
                                return;
                            };
                            // The grid is re-read here on purpose. Up to five
                            // seconds can pass, and the prompt that justified
                            // this response may be gone: the user answered it by
                            // hand, `sudo` timed out, the command finished. The
                            // response would then be typed into whatever now
                            // reads stdin — for a credential that means a shell
                            // command line, the scrollback and the shell's
                            // history. Matching the active line against the line
                            // that fired is what keeps the delay from becoming
                            // the very leak the active-line rule prevents.
                            let content = Self::visible_text(&terminal);
                            if Self::active_line_text(&content) != Some(expect_line.as_str()) {
                                tracing::warn!(
                                    %rule_id,
                                    "Prompt left the active line during the configured delay; \
                                     dropping the response instead of typing it blind"
                                );
                                return;
                            }
                            // Suppressed here for the same reason as the
                            // undelayed branch: this response may be a
                            // password, and a script recording running
                            // alongside must not capture it.
                            crate::terminal::script_recorder::suppress_recording(|| {
                                terminal.feed_child(response.as_bytes());
                            });
                        },
                    );
                }
            }
        }
    }
}

/// Resolves an `ExpectRule` list into rules ready to hand to [`AutomationSession`].
///
/// Escape sequences are expanded and `${VAR}` references substituted, in that
/// order; disabled rules, rules with an invalid regex, and rules whose response
/// could not be substituted are dropped.
///
/// `var_manager` is expected to carry the connection's built-in `${password}`,
/// `${username}`, `${host}` and `${port}`, its connection-local variables, and
/// the global variables — see `window::protocols::automation_variables`, which
/// assembles them. Without the built-ins those four resolve against nothing
/// (issue #257); without the locals a `${var}` defined only on the connection is
/// dropped as undefined and the prompt is left to the user (issue #317).
pub(crate) fn prepare_rules_from_config(
    rules: &[ExpectRule],
    var_manager: &rustconn_core::variables::VariableManager,
) -> Vec<PreparedExpectRule> {
    let mut prepared = Vec::new();

    for rule in rules {
        if !rule.enabled {
            continue;
        }

        // Validate pattern
        if rule.validate_pattern().is_err() {
            tracing::warn!(
                pattern = %rule.pattern,
                "Skipping expect rule with invalid regex"
            );
            continue;
        }

        // Escapes first, substitution second. The other order would run the
        // resolved values through `process_escapes` as well, so a password
        // containing a backslash (`pa\ss` → `pa` + an unknown escape, `a\nb` →
        // an embedded newline) would be silently rewritten before it was sent.
        let template = Zeroizing::new(AutomationSession::process_escapes(&rule.response));

        // Read from the template, before substitution: afterwards the credential
        // is indistinguishable from any other resolved text without reading it
        // back out. Same token `automation_variables` gates the vault lookup on.
        let carries_credential = template.contains("${password}");

        // Substitute ${VAR} references in the response text.
        let resolved_response = match var_manager.substitute_for_terminal_input(
            &template,
            rustconn_core::variables::VariableScope::Global,
        ) {
            Ok(substitution) => {
                if !substitution.unresolved.is_empty() {
                    // Names only, never values. An unresolved credential must
                    // leave the prompt for the user instead of typing `${...}`
                    // and potentially consuming an authentication attempt.
                    tracing::warn!(
                        rule_id = %rule.id,
                        pattern = %rule.pattern,
                        unresolved = %substitution.unresolved.join(", "),
                        "Expect response references undefined variables; skipping rule"
                    );
                    continue;
                }
                substitution.text
            }
            Err(e) => {
                // Previously this fell back to the raw template, which typed the
                // literal `${password}` into the session. Sending nothing leaves
                // the prompt to the user, who can still answer it by hand.
                tracing::warn!(
                    rule_id = %rule.id,
                    pattern = %rule.pattern,
                    error = %e,
                    "Variable substitution failed in expect response; skipping rule"
                );
                continue;
            }
        };

        prepared.push(PreparedExpectRule {
            id: rule.id,
            pattern: rule.pattern.clone(),
            response: resolved_response,
            priority: rule.priority,
            timeout_ms: rule.timeout_ms,
            one_shot: rule.one_shot,
            delay_ms: rule.delay_ms,
            carries_credential,
        });
    }

    prepared
}

#[cfg(test)]
mod tests {
    use rustconn_core::variables::{Variable, VariableManager};

    use super::*;

    /// The pattern the built-in "Sudo Password" template ships with.
    const SUDO_PATTERN: &str = r"\[sudo\] password for \w+:";

    fn manager_with(name: &str, value: &str) -> VariableManager {
        let mut manager = VariableManager::new();
        manager.set_global(Variable::new(name, value));
        manager
    }

    fn sudo_rule(response: &str) -> ExpectRule {
        ExpectRule::new(SUDO_PATTERN, response)
            .with_priority(10)
            .with_timeout(30_000)
    }

    /// The active-prompt-line rule, which used to be asserted only in prose
    /// because `check_terminal_content` needs a live `vte4::Terminal`. These two
    /// helpers are the whole mechanism, so testing them tests the guard.
    mod active_line {
        use super::super::AutomationSession;

        #[test]
        fn the_active_line_is_the_last_line_with_anything_on_it() {
            let grid = "$ sudo -v\n[sudo] password for u:\n\n\n";
            assert_eq!(AutomationSession::active_line_index(grid), Some(1));
            assert_eq!(
                AutomationSession::active_line_text(grid),
                Some("[sudo] password for u:")
            );
        }

        #[test]
        fn whitespace_only_lines_do_not_count_as_active() {
            assert_eq!(
                AutomationSession::active_line_index("a\n   \n\t\n"),
                Some(0)
            );
        }

        /// Fails closed: nothing to answer rather than everything matching.
        #[test]
        fn an_empty_grid_has_no_active_line() {
            assert_eq!(AutomationSession::active_line_index(""), None);
            assert_eq!(AutomationSession::active_line_index("\n  \n\t"), None);
            assert_eq!(AutomationSession::active_line_text("   "), None);
        }

        /// A one-shot rule scans the whole grid; a repeating one is pinned to the
        /// active line, which is what stops a sudo rule re-firing on a redraw.
        #[test]
        fn only_a_one_shot_rule_may_fire_off_the_active_line() {
            assert!(AutomationSession::should_fire(true, 0, Some(5)));
            assert!(!AutomationSession::should_fire(false, 0, Some(5)));
            assert!(AutomationSession::should_fire(false, 5, Some(5)));
        }

        /// With no active line a repeating rule cannot fire at all.
        #[test]
        fn a_repeating_rule_never_fires_without_an_active_line() {
            assert!(!AutomationSession::should_fire(false, 0, None));
        }

        /// A prompt under a tmux/screen status bar is not the last non-empty line,
        /// so injection does not happen. Documented as the accepted trade-off: the
        /// guard fails closed, which is the right direction for a credential.
        #[test]
        fn a_status_bar_below_the_prompt_keeps_a_repeating_rule_from_firing() {
            let grid = "[sudo] password for u:\n[0] 0:bash*  \"host\" 12:00\n";
            assert_eq!(AutomationSession::active_line_index(grid), Some(1));
            assert!(!AutomationSession::should_fire(false, 0, Some(1)));
        }
    }

    /// Issue #257: the stock template used to answer the prompt with a bare
    /// newline because nothing defined `${password}`.
    #[test]
    fn sudo_template_resolves_the_connection_password() {
        let manager = manager_with("password", "hunter2");
        let prepared = prepare_rules_from_config(&[sudo_rule("${password}\n")], &manager);

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].response.as_str(), "hunter2\n");
    }

    /// Only a rule that referenced `${password}` is treated as spending a
    /// credential, so the fire cap does not apply to an ordinary repeating rule.
    #[test]
    fn only_a_password_template_is_marked_as_carrying_a_credential() {
        let manager = manager_with("password", "hunter2");
        let prepared = prepare_rules_from_config(&[sudo_rule("${password}\n")], &manager);
        assert!(prepared[0].carries_credential);

        let manager = manager_with("answer", "yes");
        let prepared = prepare_rules_from_config(&[sudo_rule("${answer}\n")], &manager);
        assert!(
            !prepared[0].carries_credential,
            "a rule answering with an ordinary variable spends no credential"
        );

        let prepared = prepare_rules_from_config(&[sudo_rule("q\n")], &VariableManager::new());
        assert!(!prepared[0].carries_credential);
    }

    /// A password made entirely of the characters `substitute_for_command`
    /// rejects has to reach the prompt unchanged — nothing here goes to a shell.
    #[test]
    fn a_password_full_of_shell_metacharacters_survives() {
        let manager = manager_with("password", "a;b|c&d`e$f(g)h<i>j!k");
        let prepared = prepare_rules_from_config(&[sudo_rule("${password}\n")], &manager);

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].response.as_str(), "a;b|c&d`e$f(g)h<i>j!k\n");
    }

    /// Escapes are expanded on the template before substitution, so a backslash
    /// inside the resolved value is not reinterpreted.
    #[test]
    fn a_backslash_in_the_password_is_not_treated_as_an_escape() {
        let manager = manager_with("password", r"pa\nss");
        // The literal two characters `\` and `n` as they arrive from config.toml.
        let prepared = prepare_rules_from_config(&[sudo_rule(r"${password}\n")], &manager);

        assert_eq!(prepared.len(), 1);
        // The template's trailing `\n` became a real newline; the one inside the
        // password did not.
        assert_eq!(prepared[0].response.as_str(), "pa\\nss\n");
    }

    /// The template's own escape sequence still has to be expanded.
    #[test]
    fn the_template_escape_sequence_becomes_a_real_newline() {
        let manager = VariableManager::new();
        let prepared = prepare_rules_from_config(&[sudo_rule(r"yes\n")], &manager);

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].response.as_str(), "yes\n");
    }

    /// An undefined reference must not consume a remote authentication attempt.
    #[test]
    fn an_undefined_reference_drops_the_rule() {
        let manager = VariableManager::new();
        let prepared = prepare_rules_from_config(&[sudo_rule("${password}\n")], &manager);

        assert!(prepared.is_empty());
    }

    /// A value carrying a line break would submit the answer early, so the rule
    /// is dropped instead of sending the raw template (which used to type the
    /// literal `${password}` at the prompt).
    #[test]
    fn a_rule_whose_value_contains_a_newline_is_dropped() {
        let manager = manager_with("password", "first\nsecond");
        let prepared = prepare_rules_from_config(&[sudo_rule("${password}\n")], &manager);

        assert!(prepared.is_empty());
    }

    #[test]
    fn disabled_rules_and_invalid_patterns_are_dropped() {
        let manager = VariableManager::new();
        let rules = vec![
            sudo_rule("ok\n").with_enabled(false),
            ExpectRule::new("[unclosed", "ok\n"),
            sudo_rule("kept\n"),
        ];

        let prepared = prepare_rules_from_config(&rules, &manager);

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].response.as_str(), "kept\n");
    }

    /// Priority, timeout, id and the one-shot flag are carried through, since
    /// `AutomationSession` keys its expiry bookkeeping on them.
    #[test]
    fn rule_metadata_is_preserved() {
        let manager = manager_with("password", "hunter2");
        let original = sudo_rule("${password}\n");
        let id = original.id;

        let prepared = prepare_rules_from_config(&[original], &manager);

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].id, id);
        assert_eq!(prepared[0].pattern, SUDO_PATTERN);
        assert_eq!(prepared[0].priority, 10);
        assert_eq!(prepared[0].timeout_ms, Some(30_000));
        assert!(prepared[0].one_shot);
    }

    /// The elevated-credentials path end to end at the level this file owns: the
    /// generated rules must resolve `${password}` from the credential cache and
    /// arrive with their delay intact, or the feature is a config field nothing
    /// acts on.
    #[test]
    fn generated_elevated_rules_resolve_the_password_and_keep_their_delay() {
        let elevated = rustconn_core::models::ElevatedCredentials {
            enabled: true,
            custom_prompts: Vec::new(),
            delay_ms: 250,
        };
        let rules = rustconn_core::elevated_credentials_rules(&elevated);
        assert_ne!(rules, []);

        let manager = manager_with("password", "hunter2");
        let prepared = prepare_rules_from_config(&rules, &manager);

        assert_eq!(prepared.len(), rules.len());
        for rule in &prepared {
            assert_eq!(
                rule.response.as_str(),
                "hunter2\n",
                "the placeholder must be resolved and its \\n expanded"
            );
            assert_eq!(rule.delay_ms, Some(250));
            assert!(
                !rule.one_shot,
                "sudo is run more than once in a session; repeat firing is bounded \
                 by matching the active prompt line only"
            );
        }
    }

    /// Without a resolved password the rules are dropped rather than typing a
    /// literal `${password}` into the session — the same contract a hand-written
    /// rule has.
    #[test]
    fn generated_elevated_rules_are_dropped_without_a_password() {
        let elevated = rustconn_core::models::ElevatedCredentials {
            enabled: true,
            ..rustconn_core::models::ElevatedCredentials::default()
        };
        let rules = rustconn_core::elevated_credentials_rules(&elevated);
        let empty = rustconn_core::variables::VariableManager::new();

        assert!(prepare_rules_from_config(&rules, &empty).is_empty());
    }
}
