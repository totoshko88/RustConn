//! Keeps a session tab in the tab view that owns it, and turns a drag into a
//! proper move.
//!
//! libadwaita lets a tab be dragged from one `AdwTabView` to any other in the
//! process. Since a cluster can open in a window of its own there are several
//! (the main window's and one per cluster window), and each keeps its own
//! page → session bookkeeping that a drag would bypass: a session dragged out of
//! a cluster window would sit in the main tab bar unknown to the notebook, and
//! closing it there would end nothing. So a tab that a drag took to another view
//! is handed straight back, and the window layer is then told where it was
//! dropped ([`set_drop_handler`]) and moves the session there through the same
//! paths *Move Tab to Main Window* and *Move to New Window* use. A tab dropped
//! outside every window asks for a new one ([`set_new_window_handler`]).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

/// Told, once a dragged tab is back in its own view, which view it was dropped
/// on and at which position there.
type DropHandler = Rc<dyn Fn(&adw::TabPage, &adw::TabView, i32)>;

/// Opens a window for a tab dropped outside every window and returns its view.
type NewWindowHandler = Rc<dyn Fn() -> Option<adw::TabView>>;

thread_local! {
    static DROP_HANDLER: RefCell<Option<DropHandler>> = const { RefCell::new(None) };
    static NEW_WINDOW_HANDLER: RefCell<Option<NewWindowHandler>> = const { RefCell::new(None) };
}

/// Sets what happens to a tab dragged onto another window's tab bar.
///
/// The handler runs after the tab is back in its own view, so it starts from
/// consistent bookkeeping. Without one the tab simply stays home.
pub(crate) fn set_drop_handler<F>(handler: F)
where
    F: Fn(&adw::TabPage, &adw::TabView, i32) + 'static,
{
    DROP_HANDLER.with(|slot| *slot.borrow_mut() = Some(Rc::new(handler)));
}

/// Sets how a window is opened for a tab dropped outside every window.
pub(crate) fn set_new_window_handler<F>(handler: F)
where
    F: Fn() -> Option<adw::TabView> + 'static,
{
    NEW_WINDOW_HANDLER.with(|slot| *slot.borrow_mut() = Some(Rc::new(handler)));
}

/// The `create-window` answer of every tab view: a fresh window's view, which
/// the dropped tab then lands on like on any other window's tab bar.
pub(crate) fn new_window_view() -> Option<adw::TabView> {
    let handler = NEW_WINDOW_HANDLER.with(|slot| slot.borrow().clone());
    let view = handler.and_then(|handler| handler());
    if view.is_none() {
        tracing::debug!("tab dropped outside every window; no window opened");
    }
    view
}

/// A tab view that keeps its own tabs, with how it recognises them.
struct Home {
    view: glib::WeakRef<adw::TabView>,
    owns: Rc<dyn Fn(&adw::TabPage) -> bool>,
    /// Where each of its pages stood when a drag took it away.
    detached_at: Rc<RefCell<Vec<(glib::WeakRef<adw::TabPage>, i32)>>>,
}

thread_local! {
    static HOMES: RefCell<Vec<Home>> = const { RefCell::new(Vec::new()) };
}

/// Hands back every tab of `home` that a drag transferred to another view.
///
/// `is_home` answers whether a page is still one of `home`'s sessions. Every
/// path that closes or parks a page drops it from that bookkeeping before
/// `close_page_finish`, so a page that arrives in another view while `is_home`
/// still claims it can only have been dragged there.
///
/// The arrival is caught by the *receiving* view's `page-attached`: libadwaita
/// moves a page between views without reparenting its child, so nothing on the
/// page itself says that it left. Every view that calls this both keeps its own
/// tabs and watches for the tabs of the others.
pub(crate) fn return_transferred_tabs<F>(home: &adw::TabView, is_home: F)
where
    F: Fn(&adw::TabPage) -> bool + 'static,
{
    let is_home: Rc<dyn Fn(&adw::TabPage) -> bool> = Rc::new(is_home);
    let detached_at: Rc<RefCell<Vec<(glib::WeakRef<adw::TabPage>, i32)>>> =
        Rc::new(RefCell::new(Vec::new()));

    {
        let is_home = Rc::clone(&is_home);
        let detached_at = Rc::clone(&detached_at);
        home.connect_page_detached(move |_, page, position| {
            if is_home(page) {
                let mut detached_at = detached_at.borrow_mut();
                detached_at.retain(|(p, _)| p.upgrade().is_some_and(|p| &p != page));
                detached_at.push((page.downgrade(), position));
            }
        });
    }

    home.connect_page_attached(|view, page, dropped_at| {
        let owner = HOMES.with(|homes| {
            homes.borrow().iter().find_map(|home| {
                let home_view = home.view.upgrade()?;
                (home_view != *view && (home.owns)(page))
                    .then(|| (home_view, Rc::clone(&home.detached_at)))
            })
        });
        let Some((home, detached_at)) = owner else {
            return;
        };
        let position = {
            let mut detached_at = detached_at.borrow_mut();
            let found = detached_at
                .iter()
                .position(|(p, _)| p.upgrade().is_some_and(|p| &p == page));
            found.map_or_else(|| home.n_pages(), |i| detached_at.remove(i).1)
        };
        // From idle, so libadwaita finishes the drop first.
        let view_weak = view.downgrade();
        let home_weak = home.downgrade();
        let page_weak = page.downgrade();
        glib::idle_add_local_once(move || {
            let (Some(view), Some(home), Some(page)) = (
                view_weak.upgrade(),
                home_weak.upgrade(),
                page_weak.upgrade(),
            ) else {
                return;
            };
            if !(0..view.n_pages()).any(|i| view.nth_page(i) == page) {
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
                dropped_at,
                "tab dragged to another window; returning it to its own"
            );
            view.transfer_page(&page, &home, position);
            // The move itself starts from the next idle, once libadwaita has
            // settled the page back home.
            if let Some(handler) = DROP_HANDLER.with(|slot| slot.borrow().clone()) {
                glib::idle_add_local_once(move || handler(&page, &view, dropped_at));
            }
        });
    });

    HOMES.with(|homes| {
        let mut homes = homes.borrow_mut();
        homes.retain(|home| home.view.upgrade().is_some());
        homes.push(Home {
            view: home.downgrade(),
            owns: is_home,
            detached_at,
        });
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
