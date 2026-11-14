//! Confirmation before the user closes a tab that hosts a split view.
//!
//! A split tab is one `TabPage` owned by one session; the other sessions of the
//! layout are parked guests with no tab of their own. Closing that page used to
//! end the owner silently and return the guests to tabs, while the tab title
//! named whichever pane had focus. The `close-page` handler now holds a user's
//! close of such a tab and asks here: **Remove Split** keeps every session,
//! **Close All Sessions** ends all of them, **Cancel** keeps the tab.
//!
//! Programmatic closes mark the session with `CloseIntent::Programmatic`
//! before calling `close_page`, so they never reach a dialog (see
//! `detach::split_close_decision`).

use super::detach::{CloseIntent, take_intent};
use super::*;

/// Asks the window layer which sessions a tab's split shows: the owner first,
/// then its guests. Empty when the session owns no split.
pub(super) type SplitMembersProvider = Rc<RefCell<Option<Box<dyn Fn(Uuid) -> Vec<Uuid>>>>>;

/// How many session names a dialog body lists before it counts the rest.
const MAX_NAMES_SHOWN: usize = 3;

/// Splits a list of names into the ones shown and the count left out (pure).
#[must_use]
pub(super) fn summarize_names(names: &[String], max_shown: usize) -> (Vec<&str>, usize) {
    let shown: Vec<&str> = names.iter().take(max_shown).map(String::as_str).collect();
    let rest = names.len().saturating_sub(shown.len());
    (shown, rest)
}

/// Formats session names for a dialog body: `'a', 'b', 'c' and 2 more`.
fn names_text(names: &[String]) -> String {
    let (shown, rest) = summarize_names(names, MAX_NAMES_SHOWN);
    let list = shown
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    if rest == 0 {
        list
    } else {
        i18n_f("{} and {} more", &[&list, &rest.to_string()])
    }
}

/// The answer to the single-tab dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleOutcome {
    Cancel,
    RemoveSplit,
    CloseAll,
}

impl SingleOutcome {
    fn from_response(response: &str) -> Self {
        match response {
            "unsplit" => Self::RemoveSplit,
            "close_all" => Self::CloseAll,
            _ => Self::Cancel,
        }
    }
}

/// A split host found in a batch of tabs, with the guests its split shows.
pub(super) struct SplitHost {
    owner: Uuid,
    guests: Vec<Uuid>,
}

/// The shared notebook state the split close confirmation needs.
///
/// Holds the same `Rc` handles as the notebook rather than a reference to it,
/// so a dialog response closure can outlive the call without an `Rc` cycle
/// back to the notebook (as `DetachHooks` does for the tab menu).
#[derive(Clone)]
pub(super) struct SplitCloseHooks {
    tab_view: adw::TabView,
    sessions: Rc<RefCell<HashMap<Uuid, adw::TabPage>>>,
    session_info: Rc<RefCell<HashMap<Uuid, TerminalSession>>>,
    intents: Rc<RefCell<HashMap<Uuid, CloseIntent>>>,
    members: SplitMembersProvider,
    pending: Rc<RefCell<HashSet<Uuid>>>,
}

impl TerminalNotebook {
    /// Bundles the state the split close confirmation works on.
    pub(super) fn split_close_hooks(&self) -> SplitCloseHooks {
        SplitCloseHooks {
            tab_view: self.tab_view.clone(),
            sessions: Rc::clone(&self.sessions),
            session_info: Rc::clone(&self.session_info),
            intents: Rc::clone(&self.close_intents),
            members: Rc::clone(&self.split_members),
            pending: Rc::clone(&self.split_close_pending),
        }
    }

    /// Sets how the notebook learns which sessions a tab's split shows.
    ///
    /// The provider gets a session id and returns the owner first, then its
    /// guests, or nothing when the session owns no split.
    pub fn set_split_members_provider<F>(&self, provider: F)
    where
        F: Fn(Uuid) -> Vec<Uuid> + 'static,
    {
        *self.split_members.borrow_mut() = Some(Box::new(provider));
    }

    /// The sessions shown by the split the given session owns, owner first.
    #[must_use]
    pub fn split_members(&self, session_id: Uuid) -> Vec<Uuid> {
        self.split_close_hooks().members(session_id)
    }

    /// Closes a session's tab as the user would, through the tab's own close.
    ///
    /// Unlike [`Self::close_tab`] it records no programmatic intent, so a tab
    /// that hosts a split asks first. Nothing is torn down before the answer.
    pub fn request_close_tab(&self, session_id: Uuid) {
        let page = self.sessions.borrow().get(&session_id).cloned();
        if let Some(page) = page {
            self.tab_view.close_page(&page);
        } else {
            tracing::debug!(session = %session_id, "request_close_tab: no tab page");
        }
    }

    /// Records that the next close of this session's tab is the application's.
    pub(super) fn mark_programmatic_close(&self, session_id: Uuid) {
        self.split_close_hooks()
            .mark(session_id, CloseIntent::Programmatic);
    }
}

impl SplitCloseHooks {
    /// The sessions of the split the given session owns, owner first.
    ///
    /// Fails open: with no provider, or while the window layer is busy with its
    /// bridge map, the tab closes as it did before this dialog existed.
    pub(super) fn members(&self, session_id: Uuid) -> Vec<Uuid> {
        let Ok(provider) = self.members.try_borrow() else {
            tracing::warn!(session = %session_id, "split members provider busy; not asking");
            return Vec::new();
        };
        provider
            .as_ref()
            .map_or_else(Vec::new, |provider| provider(session_id))
    }

    /// Consumes the recorded intent of a session's close (default: the user).
    pub(super) fn take_intent(&self, session_id: Uuid) -> CloseIntent {
        take_intent(&mut self.intents.borrow_mut(), session_id)
    }

    /// Records the intent of the next close of a session's tab.
    pub(super) fn mark(&self, session_id: Uuid, intent: CloseIntent) {
        self.intents.borrow_mut().insert(session_id, intent);
    }

    fn name(&self, session_id: Uuid) -> String {
        self.session_info
            .borrow()
            .get(&session_id)
            .map(|info| info.name.clone())
            .unwrap_or_default()
    }

    fn page(&self, session_id: Uuid) -> Option<adw::TabPage> {
        self.sessions.borrow().get(&session_id).cloned()
    }

    /// Whether a page is still in the tab view (a dialog may outlive it).
    fn holds_page(&self, page: &adw::TabPage) -> bool {
        (0..self.tab_view.n_pages()).any(|i| self.tab_view.nth_page(i) == *page)
    }

    /// Closes sessions through their tab, without asking.
    ///
    /// Used for the guests of a split once its owner has closed: the owner's
    /// teardown (`on_split_cleanup`) gave each guest its tab back, so each one
    /// is ended through the ordinary `close-page` teardown.
    pub(super) fn close_forced(&self, session_ids: &[Uuid]) {
        for &session_id in session_ids {
            let Some(page) = self.page(session_id) else {
                tracing::warn!(
                    session = %session_id,
                    "split close: session has no tab page to close"
                );
                continue;
            };
            self.mark(session_id, CloseIntent::Programmatic);
            self.tab_view.close_page(&page);
        }
    }

    /// Finds the tabs among `session_ids` that host a split of two or more.
    pub(super) fn split_hosts(&self, session_ids: &[Uuid]) -> Vec<SplitHost> {
        session_ids
            .iter()
            .filter_map(|&owner| {
                let members = self.members(owner);
                (super::detach::split_close_decision(
                    !members.is_empty(),
                    members.len(),
                    CloseIntent::User,
                ) == super::detach::SplitCloseDecision::Ask)
                    .then(|| SplitHost {
                        owner,
                        guests: members.into_iter().filter(|&id| id != owner).collect(),
                    })
            })
            .collect()
    }

    /// The text a bulk close adds when the batch contains split tabs.
    pub(super) fn batch_split_note(&self, hosts: &[SplitHost]) -> String {
        let names: Vec<String> = hosts
            .iter()
            .flat_map(|host| host.guests.iter())
            .map(|&guest| self.name(guest))
            .collect();
        i18n_f(
            "Some of these tabs hold a split view. Closing them also disconnects the other sessions shown there: {}.",
            &[&names_text(&names)],
        )
    }

    /// Closes a batch the user confirmed, then the guests of its split tabs.
    pub(super) fn close_confirmed(&self, pages: &[adw::TabPage], hosts: &[SplitHost]) {
        for host in hosts {
            if self.page(host.owner).is_some() {
                self.mark(host.owner, CloseIntent::BatchConfirmed);
            }
        }
        for page in pages {
            if self.holds_page(page) {
                self.tab_view.close_page(page);
            }
        }
        let guests: Vec<Uuid> = hosts
            .iter()
            .flat_map(|host| host.guests.iter().copied())
            .collect();
        self.close_forced(&guests);
    }

    /// Closes a batch of tabs, asking once when it includes a split tab.
    pub(super) fn close_pages(&self, pages: Vec<adw::TabPage>) {
        let ids: Vec<Uuid> = {
            let sessions = self.sessions.borrow();
            pages
                .iter()
                .filter_map(|page| sessions.iter().find(|(_, p)| *p == page).map(|(id, _)| *id))
                .collect()
        };
        let hosts = self.split_hosts(&ids);
        if hosts.is_empty() {
            for page in &pages {
                self.tab_view.close_page(page);
            }
            return;
        }
        let Some(root) = self.tab_view.root() else {
            tracing::warn!("bulk close: no window to ask in; nothing closed");
            return;
        };
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Close Tabs With Split Views?"))
            .body(self.batch_split_note(&hosts))
            .build();
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("close_all", &i18n("Close All Sessions"));
        dialog.set_response_appearance("close_all", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let hooks = self.clone();
        let pages = Rc::new(pages);
        let hosts = Rc::new(hosts);
        dialog.connect_response(None, move |_, response| {
            if response != "close_all" {
                return;
            }
            let hooks = hooks.clone();
            let pages = Rc::clone(&pages);
            let hosts = Rc::clone(&hosts);
            // Run after the response signal has returned, outside any borrow
            // the dispatching code may hold.
            glib::idle_add_local_once(move || hooks.close_confirmed(&pages, &hosts));
        });
        dialog.present(Some(&root));
    }

    /// Holds the close of a split tab and asks what to do with it.
    ///
    /// Called from the `close-page` handler, which returns without finishing
    /// the close; every answer calls `close_page_finish(page, false)` exactly
    /// once and then does its work.
    pub(super) fn present_single(&self, page: &adw::TabPage, owner: Uuid, members: &[Uuid]) {
        if !self.pending.borrow_mut().insert(owner) {
            // A dialog for this tab is already open.
            self.tab_view.close_page_finish(page, false);
            return;
        }
        let Some(root) = self.tab_view.root() else {
            tracing::warn!(session = %owner, "split close: no window to ask in; tab kept");
            self.pending.borrow_mut().remove(&owner);
            self.tab_view.close_page_finish(page, false);
            return;
        };
        let guests: Vec<Uuid> = members.iter().copied().filter(|&id| id != owner).collect();
        let guest_names: Vec<String> = guests.iter().map(|&id| self.name(id)).collect();
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Close Split Tab?"))
            .body(i18n_f(
                "This tab belongs to '{}'. Its split view also shows {}.\n\nRemove Split keeps every session open in its own tab. Close All Sessions disconnects all of them.",
                &[&self.name(owner), &names_text(&guest_names)],
            ))
            .build();
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("unsplit", &i18n("Remove Split"));
        dialog.add_response("close_all", &i18n("Close All Sessions"));
        dialog.set_response_appearance("close_all", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("unsplit"));
        dialog.set_close_response("cancel");
        let hooks = self.clone();
        let page = page.clone();
        let guests = Rc::new(guests);
        dialog.connect_response(None, move |_, response| {
            let outcome = SingleOutcome::from_response(response);
            let hooks = hooks.clone();
            let page = page.clone();
            let guests = Rc::clone(&guests);
            // Run after the response signal has returned, outside any borrow
            // the dispatching code may hold.
            glib::idle_add_local_once(move || {
                hooks.finish_single(&page, owner, &guests, outcome);
            });
        });
        dialog.present(Some(&root));
    }

    /// Carries out the answer to the single-tab dialog.
    fn finish_single(
        &self,
        page: &adw::TabPage,
        owner: Uuid,
        guests: &[Uuid],
        outcome: SingleOutcome,
    ) {
        self.pending.borrow_mut().remove(&owner);
        self.tab_view.close_page_finish(page, false);
        // While the dialog was open the page was closing, so libadwaita
        // ignored any close the application asked for meanwhile (a clean exit,
        // a script) and left its mark behind. Honour it now.
        let late_close = self.intents.borrow_mut().remove(&owner).is_some();
        tracing::debug!(session = %owner, ?outcome, late_close, "split close answered");
        match outcome {
            SingleOutcome::CloseAll => {
                self.close_forced(&[owner]);
                self.close_forced(guests);
            }
            SingleOutcome::Cancel | SingleOutcome::RemoveSplit if late_close => {
                self.close_forced(&[owner]);
            }
            SingleOutcome::RemoveSplit => self.remove_split(page),
            SingleOutcome::Cancel => {}
        }
    }

    /// Dissolves the split of `page` through `win.unsplit`, as `tab.unsplit`
    /// does: that action acts on the selected tab, so the page is selected
    /// first.
    fn remove_split(&self, page: &adw::TabPage) {
        self.tab_view.set_selected_page(page);
        let Some(window) = self
            .tab_view
            .root()
            .and_then(|root| root.downcast::<gtk4::ApplicationWindow>().ok())
        else {
            tracing::warn!("split close: could not find ApplicationWindow for Remove Split");
            return;
        };
        if !gtk4::prelude::ActionGroupExt::is_action_enabled(&window, "unsplit") {
            tracing::debug!("split close: win.unsplit is disabled; split kept");
            return;
        }
        gtk4::prelude::ActionGroupExt::activate_action(&window, "unsplit", None);
    }
}

#[cfg(test)]
mod tests {
    use super::summarize_names;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("host{i}")).collect()
    }

    #[test]
    fn summarize_names_shows_all_up_to_limit() {
        for n in 1..=3 {
            let list = names(n);
            let (shown, rest) = summarize_names(&list, 3);
            assert_eq!(shown.len(), n);
            assert_eq!(rest, 0);
        }
    }

    #[test]
    fn summarize_names_truncates_to_three_plus_count() {
        let list = names(5);
        let (shown, rest) = summarize_names(&list, 3);
        assert_eq!(shown, ["host0", "host1", "host2"]);
        assert_eq!(rest, 2);
    }

    #[test]
    fn summarize_names_empty_and_one() {
        let (shown, rest) = summarize_names(&[], 3);
        assert!(shown.is_empty());
        assert_eq!(rest, 0);
        let one = names(1);
        assert_eq!(summarize_names(&one, 3), (vec!["host0"], 0));
    }
}
