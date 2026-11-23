//! Tab titles, tooltips, split colours, broadcast/shared markers, groups and pinning.
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

impl TerminalNotebook {
    /// Sets a color indicator on a tab to show it's in a split pane
    /// Applies a colored left border to the tab's title in the TabBar
    pub fn set_tab_split_color(&self, session_id: Uuid, color_index: usize) {
        // Track split color so protocol/clear operations don't overwrite it
        self.split_session_colors
            .borrow_mut()
            .insert(session_id, color_index);

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            // Remove any existing tab color classes from the page's child
            for (_, tab_class) in crate::split_view::SPLIT_PANE_COLORS {
                page.child().remove_css_class(tab_class);
            }
            // Remove old indicator classes
            for i in 0..6 {
                page.child()
                    .remove_css_class(&format!("split-indicator-{}", i));
            }

            // Add the new tab color class to the page's child
            let tab_class = crate::split_view::get_tab_color_class(color_index);
            page.child().add_css_class(tab_class);

            // Add indicator class for potential CSS styling
            let indicator_class = format!("split-indicator-{}", color_index);
            page.child().add_css_class(&indicator_class);

            // Create a colored circle icon for the indicator
            // This provides a visible colored indicator in the tab header
            if let Some(icon) = crate::split_view::create_colored_circle_icon(color_index, 16) {
                page.set_indicator_icon(Some(&icon));
                page.set_indicator_tooltip("");
            } else {
                // Fallback to symbolic icon if colored icon creation fails
                let icon = gio::ThemedIcon::new("media-record-symbolic");
                page.set_indicator_icon(Some(&icon));
                page.set_indicator_tooltip("");
            }
            page.set_indicator_activatable(false);
        }

        // R6.2: reflect the new split membership in the sidebar marker. The
        // borrows above are scoped to the block, so re-reading the map here is
        // safe.
        self.notify_split_colors_changed();
    }

    /// Removes the split color indicator from a tab
    pub fn clear_tab_split_color(&self, session_id: Uuid) {
        // Remove from split color tracking
        self.split_session_colors.borrow_mut().remove(&session_id);

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            page.set_indicator_icon(gio::Icon::NONE);
            page.set_indicator_tooltip("");

            // Remove all tab color classes and indicator classes from the page's child
            let child = page.child();
            for (_, tab_class) in crate::split_view::SPLIT_PANE_COLORS {
                child.remove_css_class(tab_class);
            }
            // Remove indicator classes
            for i in 0..6 {
                child.remove_css_class(&format!("split-indicator-{}", i));
            }
        }

        // R6.2: a session left the split — clear/refresh its sidebar marker.
        self.notify_split_colors_changed();
    }

    /// Marks or unmarks a tab as a member of the cross-tab broadcast group.
    ///
    /// The marker is a title prefix (`⇄ `) rather than the tab indicator icon,
    /// which is already claimed by the split colour, the protocol colour and the
    /// offline state; the prefix stacks with all of them and with a group label
    /// prefix. Idempotent: the prefix is stripped before being re-applied, so
    /// repeated calls never double it, and a tooltip line is kept in sync so the
    /// membership is discoverable on hover (issue #329).
    pub fn set_broadcast_member_marker(&self, session_id: Uuid, member: bool) {
        const MARKER: &str = BROADCAST_TITLE_MARKER;

        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let title = page.title().to_string();
            let base = title.strip_prefix(MARKER).unwrap_or(&title);
            if member {
                page.set_title(&format!("{MARKER}{base}"));
            } else {
                page.set_title(base);
            }

            let tooltip = page.tooltip().unwrap_or_default().to_string();
            page.set_tooltip(&Self::broadcast_tooltip(&tooltip, member));
        }
    }

    /// Marks or unmarks a tab as shared read-only with other viewers.
    ///
    /// Sharing must never be hidden: the tab title carries a `◉ ` prefix and the
    /// tab menu offers Stop Sharing for as long as it lasts. Like the broadcast
    /// marker, it is stripped before being re-applied, so repeated calls never
    /// double it.
    pub fn set_shared_marker(&self, session_id: Uuid, shared: bool) {
        if shared {
            self.shared_sessions.borrow_mut().insert(session_id);
        } else {
            self.shared_sessions.borrow_mut().remove(&session_id);
        }
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let title = page.title().to_string();
            page.set_title(&with_title_marker(
                &title,
                SHARED_TITLE_MARKER,
                shared,
                &[
                    BROADCAST_TITLE_MARKER,
                    SHARED_TITLE_MARKER,
                    SCRIPT_TITLE_MARKER,
                ],
            ));
        }
        // The ◉ prefix is a glyph; give assistive technology the meaning too.
        let terminal = self.terminals.borrow().get(&session_id).cloned();
        if let Some(terminal) = terminal {
            if shared {
                terminal.update_property(&[gtk4::accessible::Property::Description(
                    &crate::i18n::i18n("This session is shared read-only"),
                )]);
            } else {
                terminal.reset_property(gtk4::AccessibleProperty::Description);
            }
        }
    }

    /// Re-appends the lines a tooltip mutator lifted out, in their fixed order:
    /// a split tab's panes line (issue #421), then the working-directory line.
    fn with_lifted_lines(
        mut tooltip: String,
        panes: Option<String>,
        cwd: Option<String>,
    ) -> String {
        if let Some(panes) = panes {
            tooltip.push_str(&panes);
        }
        if let Some(cwd) = cwd {
            tooltip.push_str(&cwd);
        }
        tooltip
    }

    /// The tooltip with its broadcast line added (`member`) or removed.
    ///
    /// The CWD and panes lines are lifted out first, so neither is mistaken for
    /// the broadcast suffix nor dropped by a toggle, and re-appended after it.
    pub(super) fn broadcast_tooltip(tooltip: &str, member: bool) -> String {
        const TOOLTIP_LINE: &str = "\n[Broadcast]";
        let (without_cwd, cwd_line) = Self::split_cwd_line(tooltip);
        let (without_cwd, panes_line) = split_title::split_panes_line(&without_cwd);
        let base = without_cwd
            .strip_suffix(TOOLTIP_LINE)
            .unwrap_or(&without_cwd);
        let rebuilt = if member {
            format!("{base}{TOOLTIP_LINE}")
        } else {
            base.to_owned()
        };
        Self::with_lifted_lines(rebuilt, panes_line, cwd_line)
    }

    /// The tooltip with its group line set to `group_name`.
    ///
    /// Shared by [`Self::set_tab_group`] and the tab menu's **Set Group…**, so
    /// the two cannot drift. The CWD and panes lines are lifted out first, so
    /// the `\n[` split cannot land on a `[Broadcast]`/old group line and
    /// discard the lines beyond it, and re-appended after the group line.
    pub(super) fn grouped_tooltip(tooltip: &str, group_name: &str) -> String {
        let (without_cwd, cwd_line) = Self::split_cwd_line(tooltip);
        let (without_cwd, panes_line) = split_title::split_panes_line(&without_cwd);
        let base = without_cwd
            .rsplit_once("\n[")
            .map_or(without_cwd.as_str(), |(base, _)| base);
        Self::with_lifted_lines(format!("{base}\n[{group_name}]"), panes_line, cwd_line)
    }

    /// The tooltip without its group line, or `None` when it has none.
    ///
    /// Lifts the CWD and panes lines like [`Self::grouped_tooltip`], so
    /// removing a group keeps the directory line rather than cutting it off.
    pub(super) fn ungrouped_tooltip(tooltip: &str) -> Option<String> {
        let (without_cwd, cwd_line) = Self::split_cwd_line(tooltip);
        let (without_cwd, panes_line) = split_title::split_panes_line(&without_cwd);
        let (base, _) = without_cwd.rsplit_once("\n[")?;
        Some(Self::with_lifted_lines(
            base.to_owned(),
            panes_line,
            cwd_line,
        ))
    }

    /// Assigns a session to a named tab group.
    ///
    /// The group is assigned a color from the palette. The tab indicator is
    /// updated to show the group color (unless a split color is active).
    pub fn set_tab_group(&self, session_id: Uuid, group_name: &str) {
        let color_index = self
            .tab_group_manager
            .borrow_mut()
            .get_or_assign_color(group_name);

        if let Some(info) = self.session_info.borrow_mut().get_mut(&session_id) {
            info.tab_group = Some(group_name.to_owned());
            info.tab_color_index = Some(color_index);
        }

        // Apply group label prefix to tab title (independent of split/protocol indicator)
        self.apply_group_color(session_id, color_index);

        // Update tooltip to include group name.
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let current_tooltip = page.tooltip().unwrap_or_default().to_string();
            page.set_tooltip(&Self::grouped_tooltip(&current_tooltip, group_name));
        }

        tracing::debug!(session_id = %session_id, group = group_name, color_index, "Tab assigned to group");
    }

    /// Renames one session's tab from a script.
    ///
    /// Unlike [`Self::rename_connection_sessions`] this touches only the open
    /// tab; the saved connection keeps its name, so reconnecting restores the
    /// original title.
    pub fn set_session_caption(&self, session_id: Uuid, caption: &str) {
        let chrome = self.tab_chrome();
        Self::apply_session_label(&chrome, session_id, caption);
    }

    /// Renames every open session of a connection, returning the ids it touched.
    ///
    /// A connection rename used to leave open sessions showing the old name.
    /// Updates the session metadata and the tab chrome (title, tooltip, group
    /// prefix); the caller updates whatever else names the session — the title
    /// of a detached window, for one (issue #236).
    pub fn rename_connection_sessions(&self, connection_id: Uuid, new_name: &str) -> Vec<Uuid> {
        let affected: Vec<Uuid> = self
            .session_info
            .borrow()
            .iter()
            .filter(|(_, info)| info.connection_id == connection_id)
            .map(|(id, _)| *id)
            .collect();

        let chrome = self.tab_chrome();
        for session_id in &affected {
            Self::apply_session_label(&chrome, *session_id, new_name);
        }
        if !affected.is_empty() {
            tracing::debug!(
                connection = %connection_id,
                sessions = affected.len(),
                "renamed open sessions after a connection rename"
            );
        }
        affected
    }

    /// Applies a new label to one session: its metadata, the tab title and the
    /// tooltip.
    ///
    /// Takes the two maps rather than `&self` because the tab context menu
    /// reaches it from a `'static` action closure, which holds clones of both
    /// and cannot borrow the notebook.
    ///
    /// The tab chrome is touched here, and the split pane header showing the
    /// session through the relabel hook. A session moved to its own window has
    /// no tab, so for it only the stored name changes here; its window title is
    /// renamed by the caller or read from `info.name` when it is placed.
    ///
    /// The tab hosting a split layout names its focused pane (issue #421), so
    /// a relabelled session — the owner, or a guest in one of its panes, which
    /// Rename Tab targets when the guest's pane has focus — is re-derived there
    /// once the label is stored.
    pub(super) fn apply_session_label(
        chrome: &split_title::TabChrome,
        session_id: Uuid,
        label: &str,
    ) {
        let (group, host) = {
            let mut info_ref = chrome.session_info.borrow_mut();
            let Some(info) = info_ref.get_mut(&session_id) else {
                return;
            };
            info.name = label.to_owned();
            (info.tab_group.clone(), info.host.clone())
        };
        Self::label_session_view(
            &chrome.terminals,
            &chrome.session_widgets,
            session_id,
            label,
        );

        // The page is bound to its own `let` first: an `if let` scrutinee
        // temporary would keep the `sessions` borrow alive across the two GTK
        // setters below.
        let page = chrome.sessions.borrow().get(&session_id).cloned();
        let retitled = page.is_some();
        if let Some(page) = page {
            page.set_title(&tab_title(label, group.as_deref()));
            // A split tab's header, panes and directory lines are put back by
            // the refresh below.
            let old_tooltip = page.tooltip().unwrap_or_default();
            page.set_tooltip(&Self::relabelled_tooltip(
                old_tooltip.as_str(),
                label,
                host.as_deref(),
                group.as_deref(),
            ));
        }

        Self::refresh_split_tabs_for(chrome, session_id, retitled.then_some(label));

        // The split pane header showing the session follows the rename too.
        // The hook is cloned out first, so it runs with no notebook borrow.
        let hook = chrome.on_session_relabelled.borrow().clone();
        if let Some(hook) = hook {
            hook(session_id, label);
        }
    }

    /// Gives a session's view the name a screen reader announces for it.
    ///
    /// Without one Orca says only "terminal" or reads an unnamed image, and in
    /// a split layout every pane sounds the same. A terminal is named after
    /// `label`, the session name its tab shows; an embedded RDP or VNC viewer
    /// gets "Remote desktop <code>label</code>" and a hint on taking the keyboard back
    /// ([`crate::embedded::label_remote_view`]). The name lives on the widget,
    /// so it moves with it into a split pane or a detached window. Called when
    /// the tab is created and from [`Self::apply_session_label`] on a rename.
    pub(super) fn label_session_view(
        terminals: &RefCell<HashMap<Uuid, Terminal>>,
        session_widgets: &RefCell<HashMap<Uuid, SessionWidgetStorage>>,
        session_id: Uuid,
        label: &str,
    ) {
        // Cloned out so no map borrow is held while GTK updates the widget.
        let terminal = terminals.borrow().get(&session_id).cloned();
        if let Some(terminal) = terminal {
            terminal.update_property(&[gtk4::accessible::Property::Label(label)]);
            return;
        }
        let viewer = match session_widgets.borrow().get(&session_id) {
            Some(SessionWidgetStorage::EmbeddedRdp(widget)) => Some(widget.drawing_area().clone()),
            Some(SessionWidgetStorage::Vnc(widget)) => Some(widget.drawing_area().clone()),
            _ => None,
        };
        if let Some(viewer) = viewer {
            crate::embedded::label_remote_view(&viewer, label);
        }
    }

    /// The tooltip of a relabelled tab, built from scratch by `tab_tooltip`.
    ///
    /// `tab_tooltip` knows nothing of the CWD line, so the one in `old` is
    /// carried over and re-appended, keeping it alive across a rename.
    pub(super) fn relabelled_tooltip(
        old: &str,
        label: &str,
        host: Option<&str>,
        group: Option<&str>,
    ) -> String {
        let cwd_line = Self::split_cwd_line(old).1;
        Self::with_lifted_lines(Self::tab_tooltip(label, host, group), None, cwd_line)
    }

    /// Returns the group name for a session, if any.
    #[must_use]
    pub fn get_tab_group(&self, session_id: Uuid) -> Option<String> {
        self.session_info
            .borrow()
            .get(&session_id)
            .and_then(|i| i.tab_group.clone())
    }

    /// Returns whether a session's tab is pinned.
    ///
    /// Reads the TabView page rather than session metadata: pinning is live
    /// page state owned by the TabView, and the tab context menu flips it
    /// through `set_page_pinned` without telling the notebook. A session with
    /// no page — parked in a split, or in its own window — reports `false`,
    /// which is what the restore path wants: it comes back as an ordinary tab.
    #[must_use]
    pub fn is_session_pinned(&self, session_id: Uuid) -> bool {
        self.sessions
            .borrow()
            .get(&session_id)
            .is_some_and(|page| page.is_pinned())
    }

    /// Pins or unpins a session's tab.
    ///
    /// The same entry point the session restore path uses to reapply a pin that
    /// was set before the restart, so there is one way to reach page pinning
    /// rather than two that can drift. The page is cloned out of the map first:
    /// `set_page_pinned` emits `page-pinned`, and holding the `sessions` borrow
    /// across the call would let a handler that reaches the notebook re-borrow
    /// it.
    pub fn set_session_pinned(&self, session_id: Uuid, pinned: bool) {
        let page = self.sessions.borrow().get(&session_id).cloned();
        if let Some(page) = page {
            self.tab_view.set_page_pinned(&page, pinned);
        }
    }

    /// Applies a group label prefix to a tab title.
    pub(super) fn apply_group_color(&self, session_id: Uuid, _color_index: usize) {
        if let Some(page) = self.sessions.borrow().get(&session_id)
            && let Some(info) = self.session_info.borrow().get(&session_id)
            && let Some(ref group_name) = info.tab_group
        {
            // Compose from `info.name`, the structural base label, not from the
            // rendered title: stripping a prefix off the title would eat the
            // first segment of a user label that happens to look like `[x] y`.
            // A split tab names its focused pane instead (issue #421).
            let name = self
                .split_tab_shown_name(session_id)
                .unwrap_or_else(|| info.name.clone());
            page.set_title(&tab_title(&name, Some(group_name)));
        }
    }
}

#[cfg(test)]
mod tab_tooltip_tests {
    use super::TerminalNotebook;

    #[test]
    fn title_only_tooltip_is_just_the_title() {
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", None, None),
            "prod-db"
        );
        // An empty host must not add a blank second line.
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", Some(""), None),
            "prod-db"
        );
    }

    #[test]
    fn host_and_group_each_get_their_own_line() {
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", Some("10.0.0.5"), None),
            "prod-db\n10.0.0.5"
        );
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", None, Some("Production")),
            "prod-db\n[Production]"
        );
        // The group line stays last, so the group strip/append logic in
        // `set_tab_group` keeps finding it and leaves the host line alone.
        assert_eq!(
            TerminalNotebook::tab_tooltip("prod-db", Some("10.0.0.5"), Some("Production")),
            "prod-db\n10.0.0.5\n[Production]"
        );
    }

    // The working-directory line is added by `update_session_cwd` through
    // `strip_cwd_line`, which must remove exactly its own line wherever it sits.
    const CWD: &str = TerminalNotebook::CWD_TOOLTIP_PREFIX; // "\n📁 "

    #[test]
    fn strip_cwd_line_is_a_noop_without_one() {
        assert_eq!(
            TerminalNotebook::strip_cwd_line("prod-db\n10.0.0.5"),
            "prod-db\n10.0.0.5"
        );
    }

    #[test]
    fn strip_cwd_line_removes_a_trailing_cwd() {
        let t = format!("prod-db\n10.0.0.5{CWD}/var/log");
        assert_eq!(TerminalNotebook::strip_cwd_line(&t), "prod-db\n10.0.0.5");
    }

    #[test]
    fn strip_cwd_line_removes_a_cwd_followed_by_a_group_line() {
        // A group toggle may append after the CWD line; the strip must remove
        // only the CWD line and keep the group line and its leading newline.
        let t = format!("prod-db{CWD}/home/me\n[Production]");
        assert_eq!(
            TerminalNotebook::strip_cwd_line(&t),
            "prod-db\n[Production]"
        );
    }

    #[test]
    fn stripping_then_appending_replaces_in_place() {
        let t = format!("prod-db{CWD}/old\n[Production]");
        let base = TerminalNotebook::strip_cwd_line(&t);
        assert_eq!(base, "prod-db\n[Production]");
        // Appending the fresh CWD line, then stripping again, returns the base —
        // the update is idempotent however many times the directory changes.
        let updated = format!("{base}{CWD}/new");
        assert_eq!(updated, format!("prod-db\n[Production]{CWD}/new"));
        assert_eq!(TerminalNotebook::strip_cwd_line(&updated), base);
    }

    #[test]
    fn split_cwd_line_returns_the_line_for_reappending() {
        let t = format!("prod-db\n10.0.0.5{CWD}/var/log");
        let (without, cwd) = TerminalNotebook::split_cwd_line(&t);
        assert_eq!(without, "prod-db\n10.0.0.5");
        assert_eq!(cwd.as_deref(), Some(format!("{CWD}/var/log").as_str()));
        // Re-appending the lifted line restores the original exactly.
        assert_eq!(format!("{without}{}", cwd.unwrap()), t);
    }

    #[test]
    fn split_cwd_line_is_none_without_one() {
        let (without, cwd) = TerminalNotebook::split_cwd_line("prod-db\n[Production]");
        assert_eq!(without, "prod-db\n[Production]");
        assert_eq!(cwd, None);
    }

    // The tooltip mutators that rebuild the host/group/broadcast portion must
    // lift the CWD line out first and re-append it, so a group assignment, a
    // broadcast toggle or a rename never drops or mis-orders the directory.
    // The tests call the transformations the mutators themselves use.

    fn broadcast_tooltip(tooltip: &str, member: bool) -> String {
        TerminalNotebook::broadcast_tooltip(tooltip, member)
    }

    fn group_tooltip(tooltip: &str, group: &str) -> String {
        TerminalNotebook::grouped_tooltip(tooltip, group)
    }

    #[test]
    fn tab_menu_set_group_keeps_the_cwd_line_last() {
        // Review of #421, finding 5: the tab menu's Set Group lifted the panes
        // line but not the directory line, so the group line landed after 📁.
        let t = format!("prod-db\n10.0.0.5{CWD}/var/log");
        assert_eq!(
            group_tooltip(&t, "Production"),
            format!("prod-db\n10.0.0.5\n[Production]{CWD}/var/log")
        );
        // Regrouping replaces the group line in place.
        let regrouped = group_tooltip(&group_tooltip(&t, "Production"), "Staging");
        assert_eq!(
            regrouped,
            format!("prod-db\n10.0.0.5\n[Staging]{CWD}/var/log")
        );
    }

    #[test]
    fn removing_a_group_keeps_the_cwd_and_panes_lines() {
        const PANES: &str = "\nPanes: server_A, server_B";
        let t = format!("server_A\n10.0.0.5\n[Production]{PANES}{CWD}/var/log");
        assert_eq!(
            TerminalNotebook::ungrouped_tooltip(&t),
            Some(format!("server_A\n10.0.0.5{PANES}{CWD}/var/log"))
        );
        // No group line: nothing to remove, the tooltip is left alone.
        assert_eq!(
            TerminalNotebook::ungrouped_tooltip(&format!("prod-db{CWD}/var/log")),
            None
        );
    }

    #[test]
    fn a_split_tabs_panes_line_survives_broadcast_and_group_changes() {
        // Issue #421: the panes line is appended last, after a broadcast line;
        // turning broadcast off must still find and strip `[Broadcast]`, and
        // a group assignment must not cut the panes line off with it.
        const PANES: &str = "\nPanes: server_A, server_B";
        let split = format!("server_A\n10.0.0.5\n[Broadcast]{PANES}");
        assert_eq!(
            broadcast_tooltip(&split, false),
            format!("server_A\n10.0.0.5{PANES}")
        );
        assert_eq!(
            group_tooltip(&split, "Production"),
            format!("server_A\n10.0.0.5\n[Production]{PANES}")
        );
        // With a directory line too, both lines come back.
        let with_cwd = format!("server_A{PANES}{CWD}/var/log");
        assert_eq!(
            broadcast_tooltip(&with_cwd, true),
            format!("server_A\n[Broadcast]{PANES}{CWD}/var/log")
        );
    }

    #[test]
    fn broadcast_toggle_keeps_the_cwd_line() {
        let with_cwd = format!("prod-db\n10.0.0.5{CWD}/var/log");
        // Enabling broadcast keeps the directory, with broadcast before it.
        let on = broadcast_tooltip(&with_cwd, true);
        assert_eq!(on, format!("prod-db\n10.0.0.5\n[Broadcast]{CWD}/var/log"));
        // Disabling it again strips only the broadcast line, directory intact.
        let off = broadcast_tooltip(&on, false);
        assert_eq!(off, with_cwd);
    }

    #[test]
    fn group_assignment_keeps_the_cwd_line_even_with_broadcast_present() {
        // The exact clobber case: broadcast line sits before the CWD line.
        let t = format!("prod-db\n10.0.0.5\n[Broadcast]{CWD}/var/log");
        let grouped = group_tooltip(&t, "Production");
        // The group split lands on `[Broadcast]` (not the CWD line), and the
        // directory is re-appended last rather than discarded.
        assert_eq!(
            grouped,
            format!("prod-db\n10.0.0.5\n[Production]{CWD}/var/log")
        );
    }

    #[test]
    fn osc7_uri_keeps_the_path_for_a_remote_host() {
        // The remote case GIO's File::path() drops to None — we must keep the
        // path and ignore the authority.
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file://remotehost/var/log"),
            Some("/var/log".to_string())
        );
        // Local / empty authority still works.
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file:///home/me"),
            Some("/home/me".to_string())
        );
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file://localhost/etc"),
            Some("/etc".to_string())
        );
    }

    #[test]
    fn osc7_uri_percent_decodes_and_rejects_non_file() {
        assert_eq!(
            TerminalNotebook::path_from_osc7_uri("file://host/home/a%20b/%D1%82"),
            Some("/home/a b/т".to_string())
        );
        // Non-file scheme or no path → None (CWD line removed).
        assert_eq!(TerminalNotebook::path_from_osc7_uri("http://host/x"), None);
        assert_eq!(TerminalNotebook::path_from_osc7_uri("file://host"), None);
    }

    #[test]
    fn rename_keeps_the_cwd_line() {
        // `apply_session_label` rebuilds via `tab_tooltip` then re-appends CWD.
        let old = format!("prod-db\n10.0.0.5{CWD}/var/log");
        let rebuilt = TerminalNotebook::relabelled_tooltip(&old, "renamed", Some("10.0.0.5"), None);
        assert_eq!(rebuilt, format!("renamed\n10.0.0.5{CWD}/var/log"));
    }
}
