//! Title and tooltip of a tab that hosts a split layout (issue #421).
//!
//! A split layout lives on its owner session's `TabPage`, so left alone the tab
//! keeps naming the owner — or whichever session last retitled it — while the
//! user works in another pane. The bridge reports which pane has focus
//! ([`SplitTitleSource`]); this module decides what the tab says about it and
//! applies it. Every protocol that can sit in a pane goes through here, because
//! the decision reads only session ids and names.
//!
//! The title is changed by swapping the session name at its end. Everything in
//! front of the name — the `[group] ` label, the `●REC` and `⇄ ` markers — is
//! owned by other features that edit the title in place, so it is kept as is.
//! The tooltip gains one line listing the panes; it is lifted out and put back
//! by every tooltip mutator, the same way the working-directory line is. Its
//! header (name and host) and directory line describe the same session as the
//! title, and the owner's own header comes back when the split ends.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use libadwaita as adw;
use uuid::Uuid;

use super::{TerminalNotebook, TerminalSession, tab_title};
use crate::i18n::i18n;
use crate::split_view::SplitTitleSource;

/// What a split tab currently derives its title from, per owner session.
pub(crate) type SplitTabTitles = Rc<RefCell<HashMap<Uuid, SplitTabTitle>>>;

/// The title state of one split tab.
#[derive(Debug, Clone)]
pub(crate) struct SplitTabTitle {
    /// The session in the focused pane, if that pane holds one.
    focused: Option<Uuid>,
    /// The sessions shown in the panes, in layout order.
    displayed: Vec<Uuid>,
    /// The session name currently at the end of the tab title.
    ///
    /// Kept as the string rather than looked up again, because a rename changes
    /// the session's name before the title, and the old name is what has to be
    /// found at the end of the title to be replaced.
    shown_name: String,
    /// The header (name and host lines) of the owner's own tooltip, put back
    /// verbatim when the tab stops hosting a split.
    owner_header: String,
}

/// The maps a tab's chrome — title and tooltip — is written from.
///
/// One bundle rather than four parameters, because the tab context menu's
/// `'static` closures and the terminal's directory handler reach these paths
/// without a `&TerminalNotebook`, and hold clones of the maps instead.
#[derive(Clone)]
pub(crate) struct TabChrome {
    pub(super) sessions: Rc<RefCell<HashMap<Uuid, adw::TabPage>>>,
    pub(super) session_info: Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
    pub(super) split_tab_titles: SplitTabTitles,
    pub(super) session_cwd: Rc<RefCell<HashMap<Uuid, String>>>,
    pub(super) on_session_relabelled: SessionRelabelHook,
    /// The session views, renamed for screen readers along with the tab.
    pub(super) terminals: Rc<RefCell<HashMap<Uuid, vte4::Terminal>>>,
    pub(super) session_widgets: Rc<RefCell<HashMap<Uuid, super::SessionWidgetStorage>>>,
}

/// Told a session's new name after a rename, so whatever else shows it — the
/// header of the split pane that holds it — can follow (issue #421).
pub(crate) type SessionRelabelHook = Rc<RefCell<Option<Rc<dyn Fn(Uuid, &str)>>>>;

/// Picks the session a split tab is named after.
///
/// The focused pane's session wins. A focused pane that is empty falls back to
/// the owner while it is still displayed, then to the first pane that shows a
/// session, so the title never names a session that left the layout.
#[must_use]
pub fn title_session(owner: Uuid, focused: Option<Uuid>, displayed: &[Uuid]) -> Uuid {
    focused
        .filter(|session| displayed.contains(session))
        .or_else(|| displayed.contains(&owner).then_some(owner))
        .or_else(|| displayed.first().copied())
        .unwrap_or(owner)
}

/// Replaces the session name at the end of a tab title, keeping every prefix.
///
/// Returns `None` when the title does not end with `shown`, which means
/// something rebuilt the title from scratch; the caller recomposes it then.
#[must_use]
pub fn replace_title_name(title: &str, shown: &str, name: &str) -> Option<String> {
    title
        .strip_suffix(shown)
        .map(|prefix| format!("{prefix}{name}"))
}

/// The session a tab is named after: for a split tab, the one [`title_session`]
/// picks from its recorded state; for any other tab, the tab's own session.
///
/// Whatever acts on "the session this tab names" — Rename Tab is the case —
/// resolves it here, so it acts on the session the user reads in the tab bar
/// rather than on the split's owner.
#[must_use]
pub(super) fn named_session(split_tab_titles: &SplitTabTitles, tab_session: Uuid) -> Uuid {
    split_tab_titles
        .borrow()
        .get(&tab_session)
        .map_or(tab_session, |entry| {
            title_session(tab_session, entry.focused, &entry.displayed)
        })
}

/// The session **Rename Tab…** relabels from a tab, or `None` when the tab
/// does not offer the item.
///
/// Only a Local Shell is relabelled from its tab; a saved connection is renamed
/// by editing the connection. The menu gate and the action both call this, so
/// the session checked is the session renamed: the one the tab is named after
/// ([`named_session`]), not the split's owner. A split owned by a Local Shell
/// with an SSH guest focused therefore offers no Rename Tab, and one owned by
/// SSH with a Local Shell guest focused offers it for the guest.
#[must_use]
pub(super) fn rename_tab_target(
    split_tab_titles: &SplitTabTitles,
    tab_session: Uuid,
    is_local_shell: impl Fn(Uuid) -> bool,
) -> Option<Uuid> {
    let named = named_session(split_tab_titles, tab_session);
    is_local_shell(named).then_some(named)
}

/// Start of the tooltip line that lists a split tab's panes.
///
/// Translated, and also what finds the line again: the locale cannot change
/// while the program runs, so the marker is stable for the life of a tooltip.
fn panes_line_marker() -> String {
    // Translators: starts the tab tooltip line listing the sessions shown in a
    // split view, e.g. "Panes: server_A, server_B". Keep it on one line.
    format!("\n{} ", i18n("Panes:"))
}

/// Builds the tooltip line that lists the sessions in a split tab's panes.
///
/// `None` with fewer than two sessions: one session is already the tab title.
/// The names are escaped, because a tab tooltip is Pango markup and a name
/// such as `R&D` would otherwise make GTK drop the whole tooltip.
#[must_use]
pub fn panes_tooltip_line(names: &[String]) -> Option<String> {
    (names.len() >= 2).then(|| {
        let escaped: Vec<String> = names
            .iter()
            .map(|name| gtk4::glib::markup_escape_text(name).to_string())
            .collect();
        format!("{}{}", panes_line_marker(), escaped.join(", "))
    })
}

/// Splits a tooltip into the text without one marked line and that line.
///
/// The line runs from `marker` (which starts with `\n`) to the next line break
/// or the end of the text, and is returned verbatim so it can be re-appended.
#[must_use]
pub fn split_marked_line(tooltip: &str, marker: &str) -> (String, Option<String>) {
    let Some(start) = tooltip.find(marker) else {
        return (tooltip.to_owned(), None);
    };
    let content_start = start + marker.len();
    let end = tooltip[content_start..]
        .find('\n')
        .map_or(tooltip.len(), |rel| content_start + rel);
    let mut without = String::with_capacity(tooltip.len());
    without.push_str(&tooltip[..start]);
    without.push_str(&tooltip[end..]);
    (without, Some(tooltip[start..end].to_owned()))
}

/// Splits a tooltip into its text without the panes line, and that line.
///
/// Every mutator that rebuilds part of a tab tooltip lifts this line out first
/// and re-appends it, so a group assignment or a broadcast toggle on a split
/// tab does not lose it.
#[must_use]
pub fn split_panes_line(tooltip: &str) -> (String, Option<String>) {
    split_marked_line(tooltip, &panes_line_marker())
}

/// Splits a tooltip into its header — the session's name and host lines — and
/// the lines after it, after lifting the panes and working-directory lines out.
///
/// The header ends at the first `\n[` line (group, `[Broadcast]`), the layout
/// `tab_tooltip` writes and `set_tab_group` already relies on.
fn split_header(tooltip: &str) -> (String, String) {
    let (without_cwd, _) = TerminalNotebook::split_cwd_line(tooltip);
    let (mut header, _) = split_panes_line(&without_cwd);
    let rest = header
        .find("\n[")
        .map_or_else(String::new, |end| header.split_off(end));
    (header, rest)
}

/// The tooltip header for a session: its name, then its host on a line of its
/// own. Both are escaped, because a tab tooltip is Pango markup and a name
/// such as `R&D` would otherwise make GTK drop the whole tooltip.
#[must_use]
pub fn tooltip_header(name: &str, host: Option<&str>) -> String {
    let mut header = gtk4::glib::markup_escape_text(name).to_string();
    if let Some(host) = host.filter(|host| !host.is_empty()) {
        header.push('\n');
        header.push_str(&gtk4::glib::markup_escape_text(host));
    }
    header
}

/// Rebuilds a tab tooltip around `header`, the panes line and the directory.
///
/// The group and broadcast lines belong to the tab and are kept. The header,
/// the panes line and the directory describe a session, so all three are
/// replaced: a split tab's tooltip speaks of the session the tab is named
/// after, never of one pane's name with another pane's host or directory.
/// The panes line goes in front of the directory line, the order every other
/// tooltip mutator keeps, so the two lines do not swap between refreshes.
#[must_use]
fn rebuilt_tooltip(tooltip: &str, parts: &SplitTooltip<'_>) -> String {
    let (_, rest) = split_header(tooltip);
    let mut out = format!("{}{rest}", parts.header);
    if let Some(line) = parts.panes_line {
        out.push_str(line);
    }
    if let Some(cwd) = parts.cwd {
        out.push_str(TerminalNotebook::CWD_TOOLTIP_PREFIX);
        out.push_str(cwd);
    }
    out
}

/// The session-describing parts of a split tab's tooltip; see
/// [`rebuilt_tooltip`].
struct SplitTooltip<'a> {
    /// Name and host lines, already escaped ([`tooltip_header`]).
    header: &'a str,
    /// The panes line, `None` to remove it.
    panes_line: Option<&'a str>,
    /// The working directory, `None` to remove the line.
    cwd: Option<&'a str>,
}

impl TerminalNotebook {
    /// Re-derives the title and tooltip of the tab hosting a split layout.
    ///
    /// Wired as the bridge's title listener, so it runs after every change of
    /// the focused pane or of the pane contents (issue #421). A layout that is
    /// down to one pane is an ordinary tab again and gets its own title back.
    pub fn update_split_tab_title(&self, source: &SplitTitleSource) {
        if !source.is_split {
            self.clear_split_tab_title(source.owner);
            return;
        }
        {
            let mut titles = self.split_tab_titles.borrow_mut();
            if let Some(entry) = titles.get_mut(&source.owner) {
                entry.focused = source.focused;
                entry.displayed.clone_from(&source.displayed);
            } else {
                // The first time the layout is reported, the title still ends
                // with the owner's own name and the tooltip starts with the
                // owner's own header; both are kept to be put back later.
                let Some(shown_name) = self
                    .session_info
                    .borrow()
                    .get(&source.owner)
                    .map(|info| info.name.clone())
                else {
                    return;
                };
                let owner_header = self
                    .sessions
                    .borrow()
                    .get(&source.owner)
                    .and_then(adw::TabPage::tooltip)
                    .map(|tooltip| split_header(&tooltip).0)
                    .unwrap_or_default();
                titles.insert(
                    source.owner,
                    SplitTabTitle {
                        focused: source.focused,
                        displayed: source.displayed.clone(),
                        shown_name,
                        owner_header,
                    },
                );
            }
        }
        Self::apply_split_tab_title(&self.tab_chrome(), source.owner);
        self.notify_split_title_changed();
    }

    /// The maps the tab chrome is written from, for paths without `&self`.
    pub(super) fn tab_chrome(&self) -> TabChrome {
        TabChrome {
            sessions: Rc::clone(&self.sessions),
            session_info: Rc::clone(&self.session_info),
            split_tab_titles: Rc::clone(&self.split_tab_titles),
            session_cwd: Rc::clone(&self.session_cwd),
            on_session_relabelled: Rc::clone(&self.on_session_relabelled),
            terminals: Rc::clone(&self.terminals),
            session_widgets: Rc::clone(&self.session_widgets),
        }
    }

    /// The session the selected tab is named after: for a split tab the one in
    /// its focused pane, otherwise the tab's own session. `None` for a page
    /// with no session (the Welcome tab).
    ///
    /// Whatever describes "the connection on screen" next to the tab title —
    /// the header-bar subtitle — reads this, so it never names one pane while
    /// the title names another.
    #[must_use]
    pub fn active_named_session_id(&self) -> Option<Uuid> {
        self.get_active_session_id()
            .map(|session| named_session(&self.split_tab_titles, session))
    }

    /// Sets the hook told after a split tab's title source changes: the
    /// focused pane, the pane contents, or the split ending.
    pub fn set_on_split_title_changed<F>(&self, hook: F)
    where
        F: Fn() + 'static,
    {
        *self.on_split_title_changed.borrow_mut() = Some(Rc::new(hook));
    }

    /// Calls the split-title hook with no notebook borrow held.
    fn notify_split_title_changed(&self) {
        let hook = self.on_split_title_changed.borrow().clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Sets the hook told every session rename (Rename Tab, a connection
    /// rename), wired by the window to the split pane headers.
    pub fn set_on_session_relabelled<F>(&self, hook: F)
    where
        F: Fn(Uuid, &str) + 'static,
    {
        *self.on_session_relabelled.borrow_mut() = Some(Rc::new(hook));
    }

    /// Gives a tab that stopped hosting a split its own title back.
    ///
    /// The owner's name returns to the end of the title, the owner's own header
    /// and directory return to the tooltip, and the panes line leaves it. A
    /// no-op for a tab that was not titled as a split.
    pub fn clear_split_tab_title(&self, owner: Uuid) {
        let Some(entry) = self.split_tab_titles.borrow_mut().remove(&owner) else {
            return;
        };
        self.restore_owner_chrome(owner, entry);
        // The tab is named after its owner again, whatever happened above.
        self.notify_split_title_changed();
    }

    /// Writes the owner's own title and tooltip back onto its tab.
    fn restore_owner_chrome(&self, owner: Uuid, entry: SplitTabTitle) {
        let page = self.sessions.borrow().get(&owner).cloned();
        let Some(page) = page else {
            return;
        };
        let Some((name, host, group)) = self
            .session_info
            .borrow()
            .get(&owner)
            .map(|info| (info.name.clone(), info.host.clone(), info.tab_group.clone()))
        else {
            return;
        };
        let header = if entry.owner_header.is_empty() {
            tooltip_header(&name, host.as_deref())
        } else {
            entry.owner_header
        };
        let cwd = self.session_cwd.borrow().get(&owner).cloned();
        Self::set_split_tab_chrome(
            &page,
            &entry.shown_name,
            &name,
            group.as_deref(),
            &SplitTooltip {
                header: &header,
                panes_line: None,
                cwd: cwd.as_deref(),
            },
        );
    }

    /// The name at the end of a split tab's title, when the tab is titled as one.
    ///
    /// Callers that recompose a title from scratch use it in place of the
    /// owner's own name, so the composed title still names the focused pane.
    pub(super) fn split_tab_shown_name(&self, owner: Uuid) -> Option<String> {
        self.split_tab_titles
            .borrow()
            .get(&owner)
            .map(|entry| entry.shown_name.clone())
    }

    /// Re-applies every split tab affected by a session's new name.
    ///
    /// `session_id` has just been relabelled. If it owns a split tab whose title
    /// was rebuilt from scratch with the new label, that label is now what ends
    /// the title. Every split tab that shows the session, as owner or in a pane,
    /// is then re-derived, so the title and the panes line carry the new name.
    /// A session whose working directory changed is re-derived the same way
    /// (`retitled_owner_label` is `None` then), since the tooltip shows the
    /// named session's directory.
    ///
    /// Returns whether any split tab shows the session.
    pub(super) fn refresh_split_tabs_for(
        chrome: &TabChrome,
        session_id: Uuid,
        retitled_owner_label: Option<&str>,
    ) -> bool {
        let owners: Vec<Uuid> = {
            let mut titles = chrome.split_tab_titles.borrow_mut();
            if let Some(label) = retitled_owner_label
                && let Some(entry) = titles.get_mut(&session_id)
            {
                label.clone_into(&mut entry.shown_name);
                // The owner's tooltip was rebuilt with the new label too, so
                // that is the header to put back when the split ends.
                if let Some(tooltip) = chrome
                    .sessions
                    .borrow()
                    .get(&session_id)
                    .and_then(adw::TabPage::tooltip)
                {
                    entry.owner_header = split_header(&tooltip).0;
                }
            }
            titles
                .iter()
                .filter(|(owner, entry)| {
                    **owner == session_id || entry.displayed.contains(&session_id)
                })
                .map(|(owner, _)| *owner)
                .collect()
        };
        for owner in &owners {
            Self::apply_split_tab_title(chrome, *owner);
        }
        !owners.is_empty()
    }

    /// Writes a split tab's title and tooltip from its recorded state.
    ///
    /// The title, the tooltip header (name and host) and the directory line
    /// all describe the named session, so Orca reading the tab never pairs one
    /// pane's name with another pane's host. Takes the maps rather than `&self`
    /// so the tab context menu's `'static` rename closure can reach it, as it
    /// does `apply_session_label`.
    fn apply_split_tab_title(chrome: &TabChrome, owner: Uuid) {
        let Some(entry) = chrome.split_tab_titles.borrow().get(&owner).cloned() else {
            return;
        };
        let page = chrome.sessions.borrow().get(&owner).cloned();
        let Some(page) = page else {
            return;
        };
        let target = title_session(owner, entry.focused, &entry.displayed);
        let (name, host, group, names) = {
            let info = chrome.session_info.borrow();
            let Some((name, host)) = info
                .get(&target)
                .map(|session| (session.name.clone(), session.host.clone()))
            else {
                tracing::debug!(%owner, %target, "split title: named session has no info");
                return;
            };
            let group = info
                .get(&owner)
                .and_then(|session| session.tab_group.clone());
            let names: Vec<String> = entry
                .displayed
                .iter()
                .filter_map(|id| info.get(id).map(|session| session.name.clone()))
                .collect();
            (name, host, group, names)
        };
        if let Some(stored) = chrome.split_tab_titles.borrow_mut().get_mut(&owner) {
            stored.shown_name.clone_from(&name);
        }
        let panes_line = panes_tooltip_line(&names);
        let header = tooltip_header(&name, host.as_deref());
        let cwd = chrome.session_cwd.borrow().get(&target).cloned();
        // Set last and outside every borrow: the setters emit `notify`, and the
        // window title follows the tab title from that signal.
        Self::set_split_tab_chrome(
            &page,
            &entry.shown_name,
            &name,
            group.as_deref(),
            &SplitTooltip {
                header: &header,
                panes_line: panes_line.as_deref(),
                cwd: cwd.as_deref(),
            },
        );
    }

    /// Puts `name` at the end of a tab title in place of `shown` and rebuilds
    /// the session-describing parts of its tooltip from `tooltip`.
    fn set_split_tab_chrome(
        page: &adw::TabPage,
        shown: &str,
        name: &str,
        group: Option<&str>,
        tooltip: &SplitTooltip<'_>,
    ) {
        let title = page.title().to_string();
        let new_title = replace_title_name(&title, shown, name).unwrap_or_else(|| {
            tracing::debug!("split title: title was rebuilt elsewhere, recomposing it");
            tab_title(name, group)
        });
        if new_title != title {
            page.set_title(&new_title);
        }
        let current = page.tooltip().unwrap_or_default().to_string();
        let new_tooltip = rebuilt_tooltip(&current, tooltip);
        if new_tooltip != current {
            page.set_tooltip(&new_tooltip);
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use super::{
        SplitTabTitle, SplitTabTitles, SplitTooltip, named_session, panes_tooltip_line,
        rebuilt_tooltip, rename_tab_target, replace_title_name, split_header, split_panes_line,
        title_session, tooltip_header,
    };
    use crate::terminal::TerminalNotebook;

    /// A split owned by `owner` showing `owner` and `guest`, focus on `focused`.
    fn split_of(owner: Uuid, guest: Uuid, focused: Uuid) -> SplitTabTitles {
        let titles: SplitTabTitles = Rc::new(RefCell::new(HashMap::new()));
        titles.borrow_mut().insert(
            owner,
            SplitTabTitle {
                focused: Some(focused),
                displayed: vec![owner, guest],
                shown_name: String::new(),
                owner_header: String::new(),
            },
        );
        titles
    }

    #[test]
    fn rename_tab_is_offered_for_the_named_session_only() {
        // Review of #421, finding 1: the gate looked at the owner while the
        // action renamed the focused pane's session.
        let shell = Uuid::new_v4();
        let ssh = Uuid::new_v4();
        let is_local_shell = |id: Uuid| id == shell;

        // Local Shell owner, SSH guest focused: the tab names the SSH session,
        // a saved connection, so no Rename Tab.
        let titles = split_of(shell, ssh, ssh);
        assert_eq!(rename_tab_target(&titles, shell, is_local_shell), None);
        // Focus back on the shell: offered, and it renames the shell.
        let titles = split_of(shell, ssh, shell);
        assert_eq!(
            rename_tab_target(&titles, shell, is_local_shell),
            Some(shell)
        );

        // SSH owner, Local Shell guest focused: offered, for the guest.
        let titles = split_of(ssh, shell, shell);
        assert_eq!(rename_tab_target(&titles, ssh, is_local_shell), Some(shell));
        let titles = split_of(ssh, shell, ssh);
        assert_eq!(rename_tab_target(&titles, ssh, is_local_shell), None);
    }

    #[test]
    fn a_single_pane_tab_offers_rename_tab_as_before() {
        let shell = Uuid::new_v4();
        let ssh = Uuid::new_v4();
        let titles: SplitTabTitles = Rc::new(RefCell::new(HashMap::new()));
        let is_local_shell = |id: Uuid| id == shell;
        assert_eq!(
            rename_tab_target(&titles, shell, is_local_shell),
            Some(shell)
        );
        assert_eq!(rename_tab_target(&titles, ssh, is_local_shell), None);
    }

    #[test]
    fn rename_tab_targets_the_session_the_split_tab_names() {
        // Review of #421: Rename Tab on a split tab whose focused pane is a
        // guest must relabel the guest, the session the tab bar shows.
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let titles: SplitTabTitles = Rc::new(RefCell::new(HashMap::new()));
        titles.borrow_mut().insert(
            a,
            SplitTabTitle {
                focused: Some(b),
                displayed: vec![a, b],
                shown_name: "server_B".to_owned(),
                owner_header: "server_A".to_owned(),
            },
        );
        assert_eq!(named_session(&titles, a), b);
        // Focus back on the owner's pane: the owner is renamed.
        if let Some(entry) = titles.borrow_mut().get_mut(&a) {
            entry.focused = Some(a);
        }
        assert_eq!(named_session(&titles, a), a);
    }

    #[test]
    fn a_single_pane_tab_still_renames_its_own_session() {
        let a = Uuid::new_v4();
        let titles: SplitTabTitles = Rc::new(RefCell::new(HashMap::new()));
        assert_eq!(named_session(&titles, a), a);
    }

    #[test]
    fn pane_names_are_escaped_for_the_markup_tooltip() {
        let names = vec!["R&D".to_owned(), "<prod>".to_owned()];
        assert_eq!(
            panes_tooltip_line(&names).as_deref(),
            Some("\nPanes: R&amp;D, &lt;prod&gt;")
        );
    }

    /// Rebuilds `tooltip` as `rebuilt_tooltip` would for a split tab.
    fn rebuild(tooltip: &str, header: &str, panes: Option<&str>, cwd: Option<&str>) -> String {
        rebuilt_tooltip(
            tooltip,
            &SplitTooltip {
                header,
                panes_line: panes,
                cwd,
            },
        )
    }

    #[test]
    fn the_panes_line_stays_in_front_of_the_directory_line() {
        let cwd = TerminalNotebook::CWD_TOOLTIP_PREFIX;
        let tooltip = format!("server_A\n10.0.0.5{cwd}/var/log");
        let panes = "\nPanes: server_A, server_B";
        let header = "server_A\n10.0.0.5";
        let first = rebuild(&tooltip, header, Some(panes), Some("/var/log"));
        assert_eq!(first, format!("server_A\n10.0.0.5{panes}{cwd}/var/log"));
        // A refresh with other panes keeps the same order.
        let other = "\nPanes: server_A, server_C";
        assert_eq!(
            rebuild(&first, header, Some(other), Some("/var/log")),
            format!("server_A\n10.0.0.5{other}{cwd}/var/log")
        );
        assert_eq!(rebuild(&first, header, None, Some("/var/log")), tooltip);
    }

    #[test]
    fn the_tooltip_describes_the_focused_session_not_the_owner() {
        // Review of #421, finding 2: a tab titled server_B showed server_A's
        // name, host and directory above the panes line.
        let cwd = TerminalNotebook::CWD_TOOLTIP_PREFIX;
        let owner = format!("server_A\n10.0.0.5\n[Prod]\n[Broadcast]{cwd}/home/a");
        let panes = "\nPanes: server_A, server_B";
        let header = tooltip_header("server_B", Some("10.0.0.6"));
        let focused_b = rebuild(&owner, &header, Some(panes), Some("/srv/b"));
        // The tab's own lines — group and broadcast — stay; the rest is B's.
        assert_eq!(
            focused_b,
            format!("server_B\n10.0.0.6\n[Prod]\n[Broadcast]{panes}{cwd}/srv/b")
        );
        // Focus back on A, whose shell has not reported a directory: no line.
        let header = tooltip_header("server_A", Some("10.0.0.5"));
        assert_eq!(
            rebuild(&focused_b, &header, Some(panes), None),
            format!("server_A\n10.0.0.5\n[Prod]\n[Broadcast]{panes}")
        );
        // A session with no host (a Local Shell) has a one-line header.
        assert_eq!(tooltip_header("Local Shell", None), "Local Shell");
        assert_eq!(tooltip_header("Local Shell", Some("")), "Local Shell");
    }

    #[test]
    fn the_owner_header_is_put_back_verbatim_after_the_split() {
        // The header is captured before the split and restored when it ends,
        // so an unsplit tab's tooltip is the one it had before.
        let cwd = TerminalNotebook::CWD_TOOLTIP_PREFIX;
        let before = format!("server_A\n[Prod]{cwd}/home/a");
        let (owner_header, rest) = split_header(&before);
        assert_eq!(owner_header, "server_A");
        assert_eq!(rest, "\n[Prod]");
        let split = rebuild(
            &before,
            &tooltip_header("server_B", Some("10.0.0.6")),
            Some("\nPanes: server_A, server_B"),
            Some("/srv/b"),
        );
        assert_eq!(
            rebuild(&split, &owner_header, None, Some("/home/a")),
            before
        );
    }

    #[test]
    fn the_tooltip_header_is_escaped_for_markup() {
        // An owner named `R&D` made GTK drop the whole split tooltip.
        assert_eq!(
            tooltip_header("R&D <lab>", Some("10.0.0.5")),
            "R&amp;D &lt;lab&gt;\n10.0.0.5"
        );
    }

    #[test]
    fn the_focused_pane_names_the_tab() {
        // Issue #421: server_A owns the tab, server_B joined last; clicking
        // server_A's pane must name the tab after server_A again.
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert_eq!(title_session(a, Some(b), &[a, b]), b);
        assert_eq!(title_session(a, Some(a), &[a, b]), a);
        // The owner is not special: a guest's pane focused names the guest.
        let c = Uuid::new_v4();
        assert_eq!(title_session(a, Some(c), &[a, b, c]), c);
    }

    #[test]
    fn an_empty_focused_pane_falls_back_to_the_owner_then_the_first_pane() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        assert_eq!(title_session(a, None, &[b, a]), a);
        // The owner's pane was closed: the first remaining session names it.
        assert_eq!(title_session(a, None, &[b, c]), b);
        // A focused session that already left the layout is ignored.
        assert_eq!(title_session(a, Some(c), &[a, b]), a);
        // Nothing displayed at all: the tab is still the owner's.
        assert_eq!(title_session(a, None, &[]), a);
    }

    #[test]
    fn the_name_is_swapped_and_every_prefix_kept() {
        assert_eq!(
            replace_title_name("server_A", "server_A", "server_B").as_deref(),
            Some("server_B")
        );
        assert_eq!(
            replace_title_name("⇄ ●REC [Prod] server_A", "server_A", "server_B").as_deref(),
            Some("⇄ ●REC [Prod] server_B")
        );
        // Swapping back restores the title exactly.
        let there = replace_title_name("[Prod] server_A", "server_A", "server_B");
        let back = there.and_then(|t| replace_title_name(&t, "server_B", "server_A"));
        assert_eq!(back.as_deref(), Some("[Prod] server_A"));
    }

    #[test]
    fn a_title_rebuilt_elsewhere_is_not_guessed_at() {
        assert_eq!(replace_title_name("renamed", "server_A", "server_B"), None);
    }

    #[test]
    fn the_tooltip_lists_every_pane_only_in_a_real_split() {
        let names = vec!["server_A".to_owned(), "server_B".to_owned()];
        assert_eq!(
            panes_tooltip_line(&names).as_deref(),
            Some("\nPanes: server_A, server_B")
        );
        assert_eq!(panes_tooltip_line(&names[..1]), None);
        assert_eq!(panes_tooltip_line(&[]), None);
    }

    #[test]
    fn the_panes_line_is_replaced_not_stacked() {
        let header = "server_A\n10.0.0.5";
        let first = rebuild(header, header, Some("\nPanes: server_A, server_B"), None);
        assert_eq!(first, "server_A\n10.0.0.5\nPanes: server_A, server_B");
        let second = rebuild(&first, header, Some("\nPanes: server_A, server_C"), None);
        assert_eq!(second, "server_A\n10.0.0.5\nPanes: server_A, server_C");
        // Back to a single pane: the line goes, the rest stays.
        assert_eq!(rebuild(&second, header, None, None), "server_A\n10.0.0.5");
    }

    #[test]
    fn the_panes_line_is_lifted_from_between_other_lines() {
        let tooltip = "server_A\nPanes: server_A, server_B\n[Prod]";
        let (without, line) = split_panes_line(tooltip);
        assert_eq!(without, "server_A\n[Prod]");
        assert_eq!(line.as_deref(), Some("\nPanes: server_A, server_B"));
        assert_eq!(split_panes_line("server_A\n[Prod]").1, None);
    }
}
