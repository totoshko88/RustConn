//! Operations on the active terminal: key sequences, clipboard, text I/O, settings, colours and highlight rules.
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

/// Extracts the text of the line under the cursor of a VTE terminal.
///
/// Returns the cursor's row via `text_range_format`. When that row is empty
/// (prompt glyphs not yet committed to the grid), falls back to the last
/// non-empty line of the screen ending at the cursor. Returns `None` when no
/// non-empty text can be extracted. Never panics. Backs
/// `TerminalNotebook::get_cursor_line_text` for cursor-position-based prompt
/// detection (issue #194).
fn cursor_line_text(terminal: &Terminal) -> Option<String> {
    let col_count = terminal.column_count();
    // `cursor_position()` returns `(column, row)` with the row in absolute
    // buffer coordinates — the same coordinates `text_range_format` takes.
    let (_col, row) = terminal.cursor_position();
    let (cursor_text, _len) =
        terminal.text_range_format(vte4::Format::Text, row, 0, row, col_count);
    if let Some(line) = cursor_text {
        let line = line.to_string();
        if !line.trim().is_empty() {
            return Some(line);
        }
    }

    // Fallback: last non-empty line of the screen ending at the cursor.
    // Rows are absolute buffer coordinates, so reading from 0 would return the
    // oldest scrollback lines whenever the buffer is not empty — which is the
    // normal case once a reconnect keeps the previous history (issue #253).
    let row_count = terminal.row_count();
    let start = (row - row_count + 1).max(0);
    let (grid_text, _len) =
        terminal.text_range_format(vte4::Format::Text, start, 0, row, col_count);
    grid_text.and_then(|g| {
        g.to_string()
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(str::to_owned)
    })
}

impl TerminalNotebook {
    /// Executes a key sequence on a terminal session
    ///
    /// Sends text, special keys (as VTE escape codes), and handles
    /// `{WAIT:ms}` delays using glib timers.
    pub fn execute_key_sequence(&self, session_id: Uuid, sequence: &KeySequence) {
        let Some(terminal) = self.get_terminal(session_id) else {
            tracing::warn!(%session_id, "Cannot execute key sequence: terminal not found");
            return;
        };

        tracing::info!(
            %session_id,
            elements = sequence.len(),
            "Executing key sequence"
        );

        // Collect elements and schedule them with cumulative delay
        let elements: Vec<KeyElement> = sequence.elements.clone();
        let mut cumulative_delay_ms: u64 = 0;

        for element in elements {
            if let KeyElement::Wait(ms) = &element {
                cumulative_delay_ms += u64::from(*ms);
            } else {
                let terminal_clone = terminal.clone();
                let delay = cumulative_delay_ms;

                match &element {
                    KeyElement::Text(text) => {
                        let text = text.clone();
                        if delay == 0 {
                            script_recorder::suppress_recording(|| {
                                terminal_clone.feed_child(text.as_bytes());
                            });
                        } else {
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(delay),
                                move || {
                                    // Suppressed inside the closure, not around
                                    // the loop: a delayed send fires long after
                                    // this function has returned, so the
                                    // thread-local guard has to be entered at the
                                    // moment the bytes are written.
                                    script_recorder::suppress_recording(|| {
                                        terminal_clone.feed_child(text.as_bytes());
                                    });
                                },
                            );
                        }
                    }
                    KeyElement::SpecialKey(key) => {
                        let bytes = key.to_vte_bytes();
                        if delay == 0 {
                            script_recorder::suppress_recording(|| {
                                terminal_clone.feed_child(bytes);
                            });
                        } else {
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(delay),
                                move || {
                                    script_recorder::suppress_recording(|| {
                                        terminal_clone.feed_child(bytes);
                                    });
                                },
                            );
                        }
                    }
                    KeyElement::Variable(name) => {
                        // Variables should be substituted before reaching here
                        tracing::warn!(
                            variable = %name,
                            "Unresolved variable in key sequence"
                        );
                    }
                    // `Wait` is peeled off by the `if let KeyElement::Wait(ms)`
                    // above and folded into `cumulative_delay_ms`; this `match`
                    // only runs in the `else` branch, so it can never be `Wait`.
                    KeyElement::Wait(_) => {
                        unreachable!("Wait is handled by the outer if-let and never reaches here")
                    }
                }
            }
        }
    }

    /// Copies selected text from the active terminal to clipboard
    pub fn copy_to_clipboard(&self) {
        if let Some(terminal) = self.get_active_terminal()
            && let Some(text) = terminal.text_selected(vte4::Format::Text)
        {
            terminal.display().clipboard().set_text(&text);
        }
    }

    /// Pastes text from clipboard to the active terminal
    pub fn paste_from_clipboard(&self) {
        if let Some(terminal) = self.get_active_terminal() {
            safe_paste::paste_into_terminal(&terminal);
        }
    }

    /// Scrolls the active terminal to the previous or next shell prompt.
    ///
    /// Reads the active session's OSC 133 [`mark_registry::MarkRegistry`], finds
    /// the nearest prompt strictly above (`previous`) or below the top of the
    /// current viewport, and scrolls there. A silent no-op when there is no such
    /// prompt, no shell integration on this session (empty registry), or on a
    /// build below the `vte-0-78` feature (the registry is never populated).
    /// (shell-integration-osc133 spec, task 4.)
    pub fn scroll_to_adjacent_prompt(&self, previous: bool) {
        tracing::debug!(previous, "scroll_to_adjacent_prompt: action invoked");
        let Some(session_id) = self.get_active_session_id() else {
            tracing::debug!("scroll_to_adjacent_prompt: no active session");
            return;
        };
        let Some(terminal) = self.get_terminal(session_id) else {
            tracing::debug!(%session_id, "scroll_to_adjacent_prompt: no terminal");
            return;
        };
        let registry = self.mark_registries.borrow().get(&session_id).cloned();
        let Some(registry) = registry else {
            tracing::debug!(%session_id, "scroll_to_adjacent_prompt: no registry");
            return;
        };
        tracing::debug!(
            %session_id,
            registry_empty = registry.borrow().is_empty(),
            "scroll_to_adjacent_prompt: registry resolved"
        );

        let Some(adjustment) = terminal.vadjustment() else {
            tracing::debug!(%session_id, "scroll_to_adjacent_prompt: no vadjustment");
            return;
        };
        // The viewport's top buffer row — the same coordinate space the registry
        // stores, documented in `highlight_overlay`.
        let viewport_top = adjustment.value() as i64;
        // The adjustment's lower bound is the oldest row still in the buffer:
        // VTE moves it up as scrollback is trimmed. Discard marks below it before
        // querying so a stale row can never be a navigation target (task 2.5).
        let oldest_row = adjustment.lower() as i64;

        let mut registry = registry.borrow_mut();
        registry.trim_below(oldest_row);
        let target = if previous {
            registry.prev_before(viewport_top)
        } else {
            registry.next_after(viewport_top)
        };
        tracing::debug!(
            viewport_top,
            oldest_row,
            ?target,
            "scroll_to_adjacent_prompt: computed target"
        );
        if let Some(row) = target {
            adjustment.set_value(row as f64);
        }
    }

    /// Sends text to the active terminal
    ///
    /// The send is marked automated so script recording does not capture it —
    /// this path is how macros and snippets reach the terminal, and a snippet
    /// carrying a resolved secret must not end up in a recorded file.
    pub fn send_text(&self, text: &str) {
        if let Some(terminal) = self.get_active_terminal() {
            script_recorder::suppress_recording(|| terminal.feed_child(text.as_bytes()));
        }
    }

    /// Sends text to a specific terminal session
    ///
    /// Suppresses script recording for the same reason as [`Self::send_text`].
    pub fn send_text_to_session(&self, session_id: Uuid, text: &str) {
        if let Some(terminal) = self.get_terminal(session_id) {
            script_recorder::suppress_recording(|| terminal.feed_child(text.as_bytes()));
        }
    }

    /// Displays output text in a specific terminal session
    pub fn display_output(&self, session_id: Uuid, text: &str) {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed(text.as_bytes());
        }
    }

    /// Gets the current terminal text content for prompt and banner detection.
    ///
    /// Reads the visible viewport. VTE addresses the whole scrollback and the
    /// visible area in one coordinate system, so rows `0..row_count` are the
    /// *oldest* scrollback lines as soon as anything has scrolled off — the
    /// same trap the highlight overlay had to fix (issue #154). Anchoring to
    /// the viewport keeps this correct now that a reconnect can start on a
    /// non-empty buffer (issue #253).
    #[must_use]
    pub fn get_terminal_text(&self, session_id: Uuid) -> Option<String> {
        self.get_terminal(session_id).map(|terminal| {
            let row_count = terminal.row_count();
            let col_count = terminal.column_count();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "adjustment value is a row index bounded by the scrollback size"
            )]
            let top = terminal
                .vadjustment()
                .map_or(0_i64, |adjustment| adjustment.value() as i64);
            let (text, _len) =
                terminal.text_range_format(vte4::Format::Text, top, 0, top + row_count, col_count);
            text.map_or_else(String::new, |g| g.to_string())
        })
    }

    /// Returns the text of the line under the cursor, for password-prompt detection.
    ///
    /// Delegates to the session's VTE terminal: extracts the cursor's row via
    /// `text_range_format`, falling back to the last non-empty grid line when the
    /// cursor row is empty (e.g. prompt glyphs not yet committed). Returns `None`
    /// only when the session has no terminal. Never panics. See issue #194.
    #[must_use]
    pub fn get_cursor_line_text(&self, session_id: Uuid) -> Option<String> {
        let terminal = self.get_terminal(session_id)?;
        cursor_line_text(&terminal)
    }

    /// Applies terminal settings to all existing terminals
    pub fn apply_settings(&self, settings: &rustconn_core::config::TerminalSettings) {
        // The global theme is painted over every theme the quick picker chose.
        self.session_themes.borrow_mut().clear();
        let terminals = self.terminals.borrow();
        for terminal in terminals.values() {
            config::configure_terminal_with_settings(terminal, settings);
        }
    }

    /// Re-applies per-connection erase modes after global settings change.
    ///
    /// [`Self::apply_settings`] runs every live terminal back through
    /// [`config::configure_terminal_with_settings`], which reinstalls the global
    /// defaults — so saving anything in Preferences → Terminal used to silently
    /// put Backspace back to `^?` on a session that had asked for `^H`, until it
    /// was reconnected (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)). Call this
    /// straight after `apply_settings`, exactly as
    /// [`Self::reapply_theme_overrides`] is called for the same reason.
    ///
    /// A session whose connection has since been deleted, or whose protocol has
    /// no such setting, is re-set to the defaults rather than skipped: its
    /// terminal has just been overwritten too, and the defaults are what it
    /// should be showing.
    pub fn reapply_erase_modes<F>(&self, get_erase_modes: F)
    where
        F: Fn(Uuid) -> Option<(BackspaceSends, DeleteSends)>,
    {
        let terminals = self.terminals.borrow();
        let session_info = self.session_info.borrow();
        for (session_id, terminal) in terminals.iter() {
            let (backspace_sends, delete_sends) = session_info
                .get(session_id)
                .and_then(|info| get_erase_modes(info.connection_id))
                .unwrap_or_default();
            config::apply_erase_mode(terminal, backspace_sends, delete_sends);
        }
    }

    /// Re-applies per-connection theme overrides after global settings change.
    ///
    /// When global terminal settings are applied, they overwrite any
    /// per-connection color customizations. This method restores those
    /// overrides by looking up each session's connection and re-applying
    /// its `theme_override` (if any).
    pub fn reapply_theme_overrides<F>(&self, theme_name: &str, get_theme_override: F)
    where
        F: Fn(Uuid) -> Option<rustconn_core::models::ConnectionThemeOverride>,
    {
        let base_theme = TerminalTheme::resolve(theme_name, crate::app::system_is_dark());
        let terminals = self.terminals.borrow();
        let session_info = self.session_info.borrow();
        for (session_id, terminal) in terminals.iter() {
            if let Some(info) = session_info.get(session_id)
                && let Some(theme_override) = get_theme_override(info.connection_id)
            {
                config::apply_theme_override_with_base(terminal, &theme_override, &base_theme);
            }
        }
    }

    /// Repaints every live terminal with the current colour theme.
    ///
    /// For the case no setting changed: `color_theme` is
    /// [`FOLLOW_SYSTEM_THEME`][rustconn_core::terminal_themes::FOLLOW_SYSTEM_THEME]
    /// and the desktop flipped light/dark, so the *resolved* palette is different
    /// while `settings.toml` is untouched. Nothing else on that path repaints.
    ///
    /// Colours only — unlike [`Self::apply_settings`] this leaves font, scrollback
    /// and erase bindings alone, so it needs no `reapply_erase_modes` chaser and
    /// cannot undo a per-session Backspace choice (issue #271).
    /// Per-connection overrides are layered back on for the same reason
    /// [`Self::reapply_theme_overrides`] exists (issue #99).
    pub fn reapply_colors<F>(&self, theme_name: &str, get_theme_override: F)
    where
        F: Fn(Uuid) -> Option<rustconn_core::models::ConnectionThemeOverride>,
    {
        let base_theme = TerminalTheme::resolve(theme_name, crate::app::system_is_dark());
        // Every terminal is repainted below, picked themes included.
        self.session_themes.borrow_mut().clear();
        let terminals = self.terminals.borrow();
        let session_info = self.session_info.borrow();
        for (session_id, terminal) in terminals.iter() {
            config::setup_colors_with_theme(terminal, theme_name);
            if let Some(info) = session_info.get(session_id)
                && let Some(theme_override) = get_theme_override(info.connection_id)
            {
                config::apply_theme_override_with_base(terminal, &theme_override, &base_theme);
            }
        }
    }

    /// Repaints one session with the named theme, as Settings does.
    ///
    /// Not saved, and not layered with the connection's colour override: the
    /// user just chose this theme for this session. Remembered for
    /// [`Self::session_theme`] until a global repaint, a reconnect or the
    /// session's end paints over it.
    pub fn set_session_theme(&self, session_id: Uuid, theme_name: &str) {
        if let Some(terminal) = self.terminals.borrow().get(&session_id) {
            config::setup_colors_with_theme(terminal, theme_name);
            self.session_themes
                .borrow_mut()
                .insert(session_id, theme_name.to_string());
        }
    }

    /// The theme [`Self::set_session_theme`] last painted on `session_id`,
    /// if nothing has painted over it since.
    #[must_use]
    pub fn session_theme(&self, session_id: Uuid) -> Option<String> {
        self.session_themes.borrow().get(&session_id).cloned()
    }

    /// Sets up highlight rules for a terminal session.
    ///
    /// Compiles global and per-connection [`HighlightRule`]s using
    /// [`CompiledHighlightRules::compile_with_options`] and gives the session a
    /// fresh [`HighlightOverlay`], the transparent layer that draws colored
    /// backgrounds and underlines on top of the VTE terminal and repaints
    /// itself as the terminal changes.
    ///
    /// When `include_builtin_defaults` is `false` the built-in
    /// ERROR/WARNING/CRITICAL/FATAL rules are not applied (issue #343).
    ///
    /// Safe to call again for the same session — on reconnect, or when
    /// [`Self::reapply_highlight_rules`] pushes new Settings: the previous
    /// overlay is dropped first, which removes its layer, its signal handlers
    /// and its hover regexes instead of stacking a second set (issue #343).
    ///
    /// VTE's `match_add_regex()` is still registered for hover-underline
    /// feedback, but the actual colored rendering is done by the overlay.
    pub fn set_highlight_rules(
        &self,
        session_id: Uuid,
        global_rules: &[HighlightRule],
        per_conn_rules: &[HighlightRule],
        include_builtin_defaults: bool,
    ) {
        let compiled = CompiledHighlightRules::compile_with_options(
            global_rules,
            per_conn_rules,
            include_builtin_defaults,
        );

        // Drop the previous overlay outside the map borrow: its `Drop` talks to
        // GTK and VTE.
        let previous = self.highlight_overlays.borrow_mut().remove(&session_id);
        drop(previous);

        let terminal = self.terminals.borrow().get(&session_id).cloned();
        let Some(terminal) = terminal else {
            self.session_highlight_rules
                .borrow_mut()
                .insert(session_id, compiled);
            return;
        };

        // Still register with VTE for hover-underline feedback. The tags go to
        // the overlay, which unregisters them when it is replaced.
        let mut match_tags = Vec::new();
        for rule in compiled.source_patterns() {
            let pattern = &rule.pattern;
            match vte4::Regex::for_match(pattern, PCRE2_MULTILINE) {
                Ok(vte_regex) => {
                    match_tags.push(terminal.match_add_regex(&vte_regex, 0));
                    tracing::trace!(
                        %session_id,
                        rule_name = %rule.name,
                        "Registered VTE highlight regex"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        %session_id,
                        rule_name = %rule.name,
                        pattern = %pattern,
                        "Failed to register VTE highlight regex: {e}"
                    );
                }
            }
        }

        // Store compiled rules first so the overlay draw func can access them
        self.session_highlight_rules
            .borrow_mut()
            .insert(session_id, compiled);

        let overlay = HighlightOverlay::attach(
            &terminal,
            Rc::clone(&self.session_highlight_rules),
            session_id,
            match_tags,
        );
        self.highlight_overlays
            .borrow_mut()
            .insert(session_id, overlay);
    }

    /// Re-applies highlight rules to every open session that has them.
    ///
    /// Settings used to reach only sessions started afterwards, so turning the
    /// built-in rules off or editing a global rule changed nothing on screen
    /// until a reconnect (issue #343). `get_connection_rules` returns a
    /// connection's own rules; a session whose connection is gone (a quick
    /// connect, a deleted entry) gets the global rules alone. Sessions that
    /// never had highlighting — the local shell, the mc file browser, embedded
    /// viewers — are left alone.
    pub fn reapply_highlight_rules<F>(
        &self,
        global_rules: &[HighlightRule],
        include_builtin_defaults: bool,
        get_connection_rules: F,
    ) where
        F: Fn(Uuid) -> Option<Vec<HighlightRule>>,
    {
        let sessions: Vec<(Uuid, Uuid)> = {
            let highlighted = self.session_highlight_rules.borrow();
            let session_info = self.session_info.borrow();
            highlighted
                .keys()
                .map(|session_id| {
                    let connection_id = session_info
                        .get(session_id)
                        .map_or_else(Uuid::nil, |info| info.connection_id);
                    (*session_id, connection_id)
                })
                .collect()
        };
        for (session_id, connection_id) in sessions {
            let per_conn_rules = get_connection_rules(connection_id).unwrap_or_default();
            self.set_highlight_rules(
                session_id,
                global_rules,
                &per_conn_rules,
                include_builtin_defaults,
            );
        }
    }

    // === Cluster terminal tracking ===
}
