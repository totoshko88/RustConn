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
//! by every tooltip mutator, the same way the working-directory line is.

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
}

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

/// Replaces the panes line of a tooltip, removing it when `line` is `None`.
///
/// The line goes in front of the working-directory line, the order every
/// other tooltip mutator keeps, so the two lines do not swap between refreshes.
#[must_use]
fn with_panes_line(tooltip: &str, line: Option<&str>) -> String {
    let (without, _) = split_panes_line(tooltip);
    let (mut out, cwd_line) = TerminalNotebook::split_cwd_line(&without);
    if let Some(line) = line {
        out.push_str(line);
    }
    if let Some(cwd) = cwd_line {
        out.push_str(&cwd);
    }
    out
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
                // with the owner's own name.
                let Some(shown_name) = self
                    .session_info
                    .borrow()
                    .get(&source.owner)
                    .map(|info| info.name.clone())
                else {
                    return;
                };
                titles.insert(
                    source.owner,
                    SplitTabTitle {
                        focused: source.focused,
                        displayed: source.displayed.clone(),
                        shown_name,
                    },
                );
            }
        }
        Self::apply_split_tab_title(
            &self.sessions,
            &self.session_info,
            &self.split_tab_titles,
            source.owner,
        );
    }

    /// Gives a tab that stopped hosting a split its own title back.
    ///
    /// The owner's name returns to the end of the title and the panes line
    /// leaves the tooltip. A no-op for a tab that was not titled as a split.
    pub fn clear_split_tab_title(&self, owner: Uuid) {
        let Some(entry) = self.split_tab_titles.borrow_mut().remove(&owner) else {
            return;
        };
        let page = self.sessions.borrow().get(&owner).cloned();
        let Some(page) = page else {
            return;
        };
        let Some((name, group)) = self
            .session_info
            .borrow()
            .get(&owner)
            .map(|info| (info.name.clone(), info.tab_group.clone()))
        else {
            return;
        };
        Self::set_split_tab_chrome(&page, &entry.shown_name, &name, group.as_deref(), None);
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
    pub(super) fn refresh_split_tabs_for(
        sessions: &Rc<RefCell<HashMap<Uuid, adw::TabPage>>>,
        session_info: &Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
        split_tab_titles: &SplitTabTitles,
        session_id: Uuid,
        retitled_owner_label: Option<&str>,
    ) {
        let owners: Vec<Uuid> = {
            let mut titles = split_tab_titles.borrow_mut();
            if let Some(label) = retitled_owner_label
                && let Some(entry) = titles.get_mut(&session_id)
            {
                label.clone_into(&mut entry.shown_name);
            }
            titles
                .iter()
                .filter(|(owner, entry)| {
                    **owner == session_id || entry.displayed.contains(&session_id)
                })
                .map(|(owner, _)| *owner)
                .collect()
        };
        for owner in owners {
            Self::apply_split_tab_title(sessions, session_info, split_tab_titles, owner);
        }
    }

    /// Writes a split tab's title and tooltip from its recorded state.
    ///
    /// Takes the maps rather than `&self` so the tab context menu's `'static`
    /// rename closure can reach it, as it does `apply_session_label`.
    fn apply_split_tab_title(
        sessions: &Rc<RefCell<HashMap<Uuid, adw::TabPage>>>,
        session_info: &Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
        split_tab_titles: &SplitTabTitles,
        owner: Uuid,
    ) {
        let Some(entry) = split_tab_titles.borrow().get(&owner).cloned() else {
            return;
        };
        let page = sessions.borrow().get(&owner).cloned();
        let Some(page) = page else {
            return;
        };
        let target = title_session(owner, entry.focused, &entry.displayed);
        let (name, group, names) = {
            let info = session_info.borrow();
            let Some(name) = info.get(&target).map(|session| session.name.clone()) else {
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
            (name, group, names)
        };
        if let Some(stored) = split_tab_titles.borrow_mut().get_mut(&owner) {
            stored.shown_name.clone_from(&name);
        }
        let panes_line = panes_tooltip_line(&names);
        // Set last and outside every borrow: the setters emit `notify`, and the
        // window title follows the tab title from that signal.
        Self::set_split_tab_chrome(
            &page,
            &entry.shown_name,
            &name,
            group.as_deref(),
            panes_line.as_deref(),
        );
    }

    /// Puts `name` at the end of a tab title in place of `shown` and sets the
    /// panes line of its tooltip (removed when `panes_line` is `None`).
    fn set_split_tab_chrome(
        page: &adw::TabPage,
        shown: &str,
        name: &str,
        group: Option<&str>,
        panes_line: Option<&str>,
    ) {
        let title = page.title().to_string();
        let new_title = replace_title_name(&title, shown, name).unwrap_or_else(|| {
            tracing::debug!("split title: title was rebuilt elsewhere, recomposing it");
            tab_title(name, group)
        });
        if new_title != title {
            page.set_title(&new_title);
        }
        let tooltip = page.tooltip().unwrap_or_default().to_string();
        let new_tooltip = with_panes_line(&tooltip, panes_line);
        if new_tooltip != tooltip {
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
        SplitTabTitle, SplitTabTitles, named_session, panes_tooltip_line, replace_title_name,
        split_panes_line, title_session, with_panes_line,
    };
    use crate::terminal::TerminalNotebook;

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

    #[test]
    fn the_panes_line_stays_in_front_of_the_directory_line() {
        let cwd = TerminalNotebook::CWD_TOOLTIP_PREFIX;
        let tooltip = format!("server_A\n10.0.0.5{cwd}/var/log");
        let panes = "\nPanes: server_A, server_B";
        let first = with_panes_line(&tooltip, Some(panes));
        assert_eq!(first, format!("server_A\n10.0.0.5{panes}{cwd}/var/log"));
        // A refresh with other panes keeps the same order.
        let other = "\nPanes: server_A, server_C";
        assert_eq!(
            with_panes_line(&first, Some(other)),
            format!("server_A\n10.0.0.5{other}{cwd}/var/log")
        );
        assert_eq!(with_panes_line(&first, None), tooltip);
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
        let first = with_panes_line("server_A\n10.0.0.5", Some("\nPanes: server_A, server_B"));
        assert_eq!(first, "server_A\n10.0.0.5\nPanes: server_A, server_B");
        let second = with_panes_line(&first, Some("\nPanes: server_A, server_C"));
        assert_eq!(second, "server_A\n10.0.0.5\nPanes: server_A, server_C");
        // Back to a single pane: the line goes, the rest stays.
        assert_eq!(with_panes_line(&second, None), "server_A\n10.0.0.5");
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
