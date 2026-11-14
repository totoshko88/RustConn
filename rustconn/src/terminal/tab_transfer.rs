//! Keeps a session tab in the tab view that owns it.
//!
//! libadwaita lets a tab be dragged from one `AdwTabView` to any other in the
//! process. Since a cluster can open in a window of its own there are several
//! (the main window's and one per cluster window), and each keeps its own
//! page → session bookkeeping that a drag would bypass: a session dragged out of
//! a cluster window would sit in the main tab bar unknown to the notebook, and
//! closing it there would end nothing. Moving a session between windows goes
//! through *Move Tab to Main Window* / *Move to New Window*, so a tab that a drag
//! took to another view is handed straight back.

use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

/// Hands back every tab of `home` that a drag transferred to another view.
///
/// `is_home` answers whether a page is still one of `home`'s sessions. Every
/// path that closes or parks a page drops it from that bookkeeping before
/// `close_page_finish`, so a page that leaves while `is_home` still claims it
/// can only have been transferred.
pub(crate) fn return_transferred_tabs<F>(home: &adw::TabView, is_home: F)
where
    F: Fn(&adw::TabPage) -> bool + 'static,
{
    let is_home = Rc::new(is_home);
    home.connect_page_detached(move |home, page, position| {
        if !is_home(page) {
            return;
        }
        // The page is not in its new view yet when it leaves this one; send it
        // back once its child has a new parent, and from idle, so libadwaita
        // finishes the drop first.
        let child = page.child();
        let handler = Rc::new(std::cell::Cell::new(None));
        let handler_slot = Rc::clone(&handler);
        let home_weak = home.downgrade();
        // Weak: the page owns the child this handler is connected to.
        let page_weak = page.downgrade();
        let is_home = Rc::clone(&is_home);
        let id = child.connect_parent_notify(move |child| {
            if child.parent().is_none() {
                return;
            }
            if let Some(id) = handler_slot.take() {
                child.disconnect(id);
            }
            let home_weak = home_weak.clone();
            let page_weak = page_weak.clone();
            let is_home = Rc::clone(&is_home);
            glib::idle_add_local_once(move || {
                let (Some(home), Some(page)) = (home_weak.upgrade(), page_weak.upgrade()) else {
                    return;
                };
                if !is_home(&page) {
                    return;
                }
                let Some(other) = page
                    .child()
                    .ancestor(adw::TabView::static_type())
                    .and_downcast::<adw::TabView>()
                else {
                    return;
                };
                if other == home {
                    return;
                }
                let position = return_position(
                    page.is_pinned(),
                    position,
                    home.n_pinned_pages(),
                    home.n_pages(),
                );
                tracing::info!(
                    position,
                    "tab dragged to another window's tab bar; returning it"
                );
                other.transfer_page(&page, &home, position);
            });
        });
        handler.set(Some(id));
    });
}

/// Where a returning tab goes: back at `position`, inside its own section.
///
/// libadwaita refuses a pinned page after an unpinned one and the reverse, so
/// the old position is clamped to the pinned section or to the rest.
fn return_position(pinned: bool, position: i32, n_pinned: i32, n_pages: i32) -> i32 {
    if pinned {
        position.clamp(0, n_pinned)
    } else {
        position.clamp(n_pinned, n_pages.max(n_pinned))
    }
}

#[cfg(test)]
mod tests {
    use super::return_position;

    #[test]
    fn an_unpinned_tab_returns_to_its_place_after_the_pinned_ones() {
        assert_eq!(return_position(false, 3, 1, 5), 3);
        assert_eq!(return_position(false, 0, 2, 5), 2);
        assert_eq!(return_position(false, 9, 0, 4), 4);
    }

    #[test]
    fn a_pinned_tab_stays_in_the_pinned_section() {
        assert_eq!(return_position(true, 1, 2, 5), 1);
        assert_eq!(return_position(true, 4, 2, 5), 2);
        assert_eq!(return_position(true, -1, 0, 3), 0);
    }

    #[test]
    fn an_empty_view_takes_the_tab_at_the_start() {
        assert_eq!(return_position(false, 3, 0, 0), 0);
        assert_eq!(return_position(true, 3, 0, 0), 0);
    }
}
