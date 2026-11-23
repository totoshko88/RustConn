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
//!
//! Three gaps in libadwaita are filled here, all checked against its 1.5 and
//! GTK's 4.14 and current sources:
//!
//! * On Wayland a drop that nothing accepts — the empty desktop, another
//!   application, the content of a RustConn window rather than its tab bar —
//!   ends with `GDK_DRAG_CANCEL_ERROR`, and libadwaita opens a new window only
//!   on `GDK_DRAG_CANCEL_NO_TARGET`. The tab is put back where it was. So every
//!   RustConn window reports a drag passing over it ([`accept_tab_drops`]), and
//!   a tab that comes back home goes to the window it was released over, or to
//!   a new window when it was released over none.
//! * On X11 the same drop on a RustConn window's content asks for a new
//!   window; [`new_window_view`] answers with that window's view instead.
//! * libadwaita never starts a drag for the only tab of a view (it moves the
//!   window instead), so [`drag_single_tab`] gives a lone tab a drag of its own.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

/// Told, once a dragged tab is back in its own view, which view it was dropped
/// on and at which position there.
type DropHandler = Rc<dyn Fn(&adw::TabPage, &adw::TabView, i32)>;

/// Opens a window for a tab dropped outside every window and returns its view.
type NewWindowHandler = Rc<dyn Fn() -> Option<adw::TabView>>;

/// The mime type of a lone tab's drag ([`drag_single_tab`]). Only RustConn
/// windows accept it, so a terminal never pastes it as text.
const SINGLE_TAB_MIME: &str = "application/x-rustconn-tab";

/// A tab dropped on something that refused the drop but wants the tab once it
/// is back home (a split panel). Expires so a stale claim never fires later.
struct Claim {
    page: glib::WeakRef<adw::TabPage>,
    at: Instant,
    action: Box<dyn FnOnce()>,
}

/// How long a claim stays valid: the cancel that follows a refused drop comes
/// within the same drag, well inside this.
const CLAIM_TTL: Duration = Duration::from_secs(2);

thread_local! {
    static CLAIM: RefCell<Option<Claim>> = const { RefCell::new(None) };
    static DROP_HANDLER: RefCell<Option<DropHandler>> = const { RefCell::new(None) };
    static NEW_WINDOW_HANDLER: RefCell<Option<NewWindowHandler>> = const { RefCell::new(None) };
    /// The view of the RustConn window a drag is over, if any.
    static DRAG_OVER: RefCell<Option<glib::WeakRef<adw::TabView>>> = const { RefCell::new(None) };
    /// The page a lone-tab drag carries.
    static SINGLE_TAB_PAGE: RefCell<Option<glib::WeakRef<adw::TabPage>>> =
        const { RefCell::new(None) };
}

/// The view of the RustConn window the current drag is over.
fn drag_over() -> Option<adw::TabView> {
    DRAG_OVER.with(|slot| slot.borrow().as_ref().and_then(glib::WeakRef::upgrade))
}

/// Registers that `page`, dragged from the tab bar, was dropped on a widget that
/// refused it, and that `action` should run once libadwaita has put it back in
/// its own view. Without this such a tab is taken for one dropped outside every
/// window and gets a window of its own.
pub(crate) fn claim_tab_drop(page: &adw::TabPage, action: impl FnOnce() + 'static) {
    CLAIM.with(|slot| {
        *slot.borrow_mut() = Some(Claim {
            page: page.downgrade(),
            at: Instant::now(),
            action: Box::new(action),
        });
    });
}

/// Takes the claim on `page`, if one is pending and fresh.
fn take_claim(page: &adw::TabPage) -> Option<Box<dyn FnOnce()>> {
    let claim = CLAIM.with(|slot| slot.borrow_mut().take())?;
    (claim_is_fresh(claim.at.elapsed()) && claim.page.upgrade().as_ref() == Some(page))
        .then_some(claim.action)
}

/// Whether a claim of age `age` is still valid.
fn claim_is_fresh(age: Duration) -> bool {
    age <= CLAIM_TTL
}

/// Runs the drop handler for `page` on the next idle.
fn report_drop(page: adw::TabPage, target: adw::TabView, position: i32) {
    if let Some(handler) = DROP_HANDLER.with(|slot| slot.borrow().clone()) {
        glib::idle_add_local_once(move || handler(&page, &target, position));
    }
}

/// Makes `window` a place a tab can be dropped on, anywhere in it, to move to
/// `view`.
///
/// It records that a drag is over the window, which decides where a tab
/// that comes back home goes, and it accepts a lone tab's drag.
pub(crate) fn accept_tab_drops(window: &impl IsA<gtk4::Widget>, view: &adw::TabView) {
    let motion = gtk4::DropControllerMotion::new();
    {
        let view = view.downgrade();
        motion.connect_enter(move |_, _, _| {
            DRAG_OVER.with(|slot| *slot.borrow_mut() = Some(view.clone()));
        });
    }
    {
        let view = view.downgrade();
        motion.connect_leave(move |_| {
            DRAG_OVER.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.as_ref().and_then(glib::WeakRef::upgrade) == view.upgrade() {
                    *slot = None;
                }
            });
        });
    }
    window.add_controller(motion);

    let formats = gtk4::gdk::ContentFormats::new(&[SINGLE_TAB_MIME]);
    let target = gtk4::DropTargetAsync::new(Some(formats), gtk4::gdk::DragAction::MOVE);
    target.connect_accept(|_, drop| drop.formats().contain_mime_type(SINGLE_TAB_MIME));
    {
        let view = view.downgrade();
        target.connect_drop(move |_, drop, _, _| {
            let page = SINGLE_TAB_PAGE.with(|slot| slot.borrow_mut().take());
            drop.finish(gtk4::gdk::DragAction::MOVE);
            if let (Some(page), Some(view)) = (page.and_then(|p| p.upgrade()), view.upgrade()) {
                let position = view.n_pages();
                report_drop(page, view, position);
            }
            true
        });
    }
    window.add_controller(target);
}

/// Lets the only tab of `view` be dragged out of `tab_bar`.
///
/// libadwaita starts no drag for a lone tab and moves the window instead. This
/// drag source is switched on only while `view` has one page, so libadwaita's
/// own drag keeps every other case. The drag lands on a RustConn window through
/// [`accept_tab_drops`]; released over none it opens a new window.
///
/// `may_drag` keeps a page out of it, such as the main window's Welcome
/// placeholder.
pub(crate) fn drag_single_tab<F>(tab_bar: &adw::TabBar, view: &adw::TabView, may_drag: F)
where
    F: Fn(&adw::TabPage) -> bool + 'static,
{
    let source = gtk4::DragSource::new();
    source.set_actions(gtk4::gdk::DragAction::MOVE);
    let sync = |source: &gtk4::DragSource, view: &adw::TabView| {
        source.set_propagation_phase(if view.n_pages() == 1 {
            gtk4::PropagationPhase::Capture
        } else {
            gtk4::PropagationPhase::None
        });
    };
    sync(&source, view);
    {
        let source = source.downgrade();
        view.connect_n_pages_notify(move |view| {
            if let Some(source) = source.upgrade() {
                sync(&source, view);
            }
        });
    }
    {
        let view = view.downgrade();
        source.connect_prepare(move |_, _, _| {
            let view = view.upgrade()?;
            if view.n_pages() != 1 {
                return None;
            }
            let page = view.nth_page(0);
            if !may_drag(&page) {
                return None;
            }
            SINGLE_TAB_PAGE.with(|slot| *slot.borrow_mut() = Some(page.downgrade()));
            Some(gtk4::gdk::ContentProvider::for_bytes(
                SINGLE_TAB_MIME,
                &glib::Bytes::from_static(b"tab"),
            ))
        });
    }
    // `drag-cancel` runs before `drag-end`. A drag the user called off (Escape)
    // forgets its page there, so `drag-end` opens no window for it; a drop that
    // nothing accepted also ends in a cancel, with another reason, and keeps it.
    source.connect_drag_cancel(|_, _, reason| {
        if !cancel_opens_window(reason) {
            tracing::debug!("lone tab drag cancelled by the user");
            SINGLE_TAB_PAGE.with(|slot| slot.borrow_mut().take());
        }
        false
    });
    source.connect_drag_end(|_, _, _| {
        // Still set when no RustConn window took the drop and the user did not
        // cancel the drag.
        let page = SINGLE_TAB_PAGE.with(|slot| slot.borrow_mut().take());
        if let Some(page) = page.and_then(|p| p.upgrade())
            && drag_over().is_none()
            && let Some(view) = new_window_view()
        {
            tracing::info!("lone tab released outside every window");
            report_drop(page, view, 0);
        }
    });
    tab_bar.add_controller(source);
}

/// Whether a lone-tab drag cancelled for `reason` still opens a new window.
///
/// Only the user calling the drag off keeps the tab where it is. A release
/// over nothing ends as `NoTarget` on X11 and as `Error` on Wayland (see the
/// module docs), and both mean "released outside every window".
fn cancel_opens_window(reason: gtk4::gdk::DragCancelReason) -> bool {
    reason != gtk4::gdk::DragCancelReason::UserCancelled
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

/// The `create-window` answer of every tab view: the view of the RustConn
/// window the tab was released over, or a fresh window's view. The dropped tab
/// then lands on it like on any other window's tab bar.
pub(crate) fn new_window_view() -> Option<adw::TabView> {
    if let Some(view) = drag_over() {
        tracing::debug!("tab released over a RustConn window; it goes there");
        return Some(view);
    }
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
        if return_cancelled_drag(view, page) {
            return;
        }
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
            report_drop(page, view, dropped_at);
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

/// Handles a dragged tab that libadwaita put back into its own view, and
/// reports whether `page` was one.
///
/// That is a drag that ended without a tab bar taking it: on Wayland, released
/// over the desktop, another application or a RustConn window's content. It
/// goes to the window it was released over, or to a new window when it was
/// released over none. Released over its own window it stays.
fn return_cancelled_drag(view: &adw::TabView, page: &adw::TabPage) -> bool {
    let entry = HOMES.with(|homes| {
        homes.borrow().iter().find_map(|home| {
            let home_view = home.view.upgrade()?;
            if home_view != *view || !(home.owns)(page) {
                return None;
            }
            let mut detached_at = home.detached_at.borrow_mut();
            let i = detached_at
                .iter()
                .position(|(p, _)| p.upgrade().is_some_and(|p| &p == page))?;
            Some(detached_at.remove(i))
        })
    });
    if entry.is_none() {
        return false;
    }
    if let Some(action) = take_claim(page) {
        tracing::info!("tab dropped on a split panel");
        glib::idle_add_local_once(action);
        return true;
    }
    match drag_over() {
        // Back in its own window: a drop on its own tab bar, which is a
        // reorder libadwaita already made, or a cancelled drag.
        Some(over) if over == *view => {}
        Some(over) => {
            tracing::info!("tab released over another RustConn window");
            let at = over.n_pages();
            report_drop(page.clone(), over, at);
        }
        None => {
            if let Some(new_view) = new_window_view() {
                tracing::info!("tab released outside every window");
                report_drop(page.clone(), new_view, 0);
            }
        }
    }
    true
}

/// Where a tab dropped at `position` goes in a tab bar whose movable tabs run
/// from `first` to `last`: libadwaita reports a drop position past the end, or
/// among the pinned tabs, which a reorder would refuse.
pub(crate) fn clamp_drop_position(position: i32, first: i32, last: i32) -> i32 {
    position.clamp(first, last.max(first))
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
    use super::{
        CLAIM_TTL, cancel_opens_window, claim_is_fresh, clamp_drop_position, return_position,
    };
    use gtk4::gdk::DragCancelReason;
    use std::time::Duration;

    #[test]
    fn an_escaped_lone_tab_drag_opens_no_window() {
        assert!(!cancel_opens_window(DragCancelReason::UserCancelled));
        assert!(cancel_opens_window(DragCancelReason::NoTarget));
        assert!(cancel_opens_window(DragCancelReason::Error));
    }

    #[test]
    fn a_claim_expires_after_its_ttl() {
        assert!(claim_is_fresh(Duration::ZERO));
        assert!(claim_is_fresh(CLAIM_TTL));
        assert!(!claim_is_fresh(CLAIM_TTL + Duration::from_millis(1)));
    }

    #[test]
    fn a_drop_past_the_end_lands_last() {
        assert_eq!(clamp_drop_position(7, 0, 3), 3);
    }

    #[test]
    fn a_drop_among_pinned_tabs_lands_after_them() {
        assert_eq!(clamp_drop_position(0, 2, 5), 2);
        assert_eq!(clamp_drop_position(1, 2, 5), 2);
    }

    #[test]
    fn a_drop_inside_the_range_keeps_its_place() {
        assert_eq!(clamp_drop_position(2, 0, 4), 2);
    }

    #[test]
    fn a_drop_into_a_view_of_only_pinned_tabs_lands_after_them() {
        // `last` below `first`: every tab is pinned, the new one goes after.
        assert_eq!(clamp_drop_position(0, 3, 2), 3);
    }

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
