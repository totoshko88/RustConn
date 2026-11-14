//! A window that hosts several sessions, one per tab.
//!
//! Opening a cluster "in a new window" puts every member here instead of in the
//! main window's tab bar, so a cluster can live on its own monitor or workspace.
//!
//! Each tab holds a *detached* session, in exactly the sense of
//! [`crate::detached_window`]: the content box comes from
//! [`crate::terminal::TerminalNotebook::take_session_content`], the notebook
//! keeps owning the session, and every way a session can end or move — closing
//! it, a remote disconnect, moving it back to the main window — goes through
//! the same notebook calls a single-session detached window uses. This window
//! is only a second place to show such sessions; it owns no session state.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, gio, glib};
use libadwaita as adw;
use uuid::Uuid;

use crate::i18n::{i18n, i18n_f};

/// Default size of a session group window.
const DEFAULT_WIDTH: i32 = 1100;
const DEFAULT_HEIGHT: i32 = 720;

/// Callback keyed by the session it concerns.
type SessionCallback = Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>;

/// One hosted session.
struct Member {
    page: adw::TabPage,
    connection_id: Uuid,
}

/// A top-level window with a tab per hosted session.
pub struct SessionGroupWindow {
    window: adw::ApplicationWindow,
    tab_view: adw::TabView,
    toast_overlay: adw::ToastOverlay,
    members: Rc<RefCell<HashMap<Uuid, Member>>>,
    /// Sessions whose page is being dropped because the session moved away or
    /// already ended, so the `close-page` handler must not end it (again).
    leaving: Rc<RefCell<Vec<Uuid>>>,
    /// Set once the window itself is closing, so emptying it does not ask it to
    /// close a second time.
    closing: Rc<Cell<bool>>,
    /// Kept alive for its `clicked` handler.
    attach_button: Button,
    on_close: SessionCallback,
    on_attach: SessionCallback,
}

impl SessionGroupWindow {
    /// Creates an empty window titled after the group it shows.
    ///
    /// The window is not presented yet; add a session and call
    /// [`Self::present`].
    #[must_use]
    pub fn new(app: &adw::Application, title: &str) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(i18n_f("{} — RustConn", &[title]))
            .default_width(DEFAULT_WIDTH)
            .default_height(DEFAULT_HEIGHT)
            .width_request(360)
            .height_request(240)
            .build();

        #[cfg(target_os = "macos")]
        window.add_css_class("macos");

        let tab_view = adw::TabView::new();
        let tab_bar = adw::TabBar::builder()
            .view(&tab_view)
            .autohide(false)
            .build();

        let attach_button = Button::from_icon_name("view-restore-symbolic");
        let attach_label = i18n("Move Tab to Main Window");
        attach_button.set_tooltip_text(Some(&attach_label));
        attach_button.update_property(&[gtk4::accessible::Property::Label(&attach_label)]);

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(title, "")));
        header.pack_start(&attach_button);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.add_top_bar(&tab_bar);
        toolbar_view.set_content(Some(&tab_view));

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar_view));
        window.set_content(Some(&toast_overlay));

        // The same per-window wiring a detached session window gets.
        crate::app::install_layout_independent_accels(&window, app);
        crate::app::watch_window_for_compact(window.upcast_ref());
        crate::app::recompute_window_compact(window.upcast_ref());

        let group = Rc::new(Self {
            window,
            tab_view,
            toast_overlay,
            members: Rc::new(RefCell::new(HashMap::new())),
            leaving: Rc::new(RefCell::new(Vec::new())),
            closing: Rc::new(Cell::new(false)),
            attach_button,
            on_close: Rc::new(RefCell::new(None)),
            on_attach: Rc::new(RefCell::new(None)),
        });
        group.connect_close_page();
        group.connect_close_request();
        group.connect_attach_button();
        group.connect_focus_on_switch();
        tracing::info!(group = title, "session group window created");
        group
    }

    /// The session shown by `page`, if it is one of ours.
    fn session_of(members: &HashMap<Uuid, Member>, page: &adw::TabPage) -> Option<Uuid> {
        members
            .iter()
            .find(|(_, member)| &member.page == page)
            .map(|(session_id, _)| *session_id)
    }

    /// Ends the session of a tab the user closes, then lets the page go.
    fn connect_close_page(&self) {
        let members = Rc::clone(&self.members);
        let leaving = Rc::clone(&self.leaving);
        let closing = Rc::clone(&self.closing);
        let on_close = Rc::clone(&self.on_close);
        let window = self.window.downgrade();
        self.tab_view.connect_close_page(move |tab_view, page| {
            let session = Self::session_of(&members.borrow(), page);
            if let Some(session_id) = session {
                members.borrow_mut().remove(&session_id);
                let was_leaving = {
                    let mut leaving = leaving.borrow_mut();
                    let before = leaving.len();
                    leaving.retain(|id| *id != session_id);
                    leaving.len() != before
                };
                if was_leaving {
                    tracing::debug!(session = %session_id, "group window: tab dropped without teardown");
                } else {
                    tracing::info!(session = %session_id, "group window: tab closed by user");
                    if let Some(ref callback) = *on_close.borrow() {
                        callback(session_id);
                    }
                }
            }
            tab_view.close_page_finish(page, true);

            // A window with no tab left has nothing to show.
            if members.borrow().is_empty()
                && !closing.get()
                && let Some(window) = window.upgrade()
            {
                closing.set(true);
                // Not from inside the page's own close handler.
                glib::idle_add_local_once(move || window.close());
            }
            glib::Propagation::Stop
        });
    }

    /// Ends every hosted session when the user closes the window.
    fn connect_close_request(&self) {
        let members = Rc::clone(&self.members);
        let closing = Rc::clone(&self.closing);
        let on_close = Rc::clone(&self.on_close);
        self.window.connect_close_request(move |_window| {
            closing.set(true);
            // Taken out first: ending a session reaches back here through the
            // notebook's session-ended hook, which must find nothing left.
            let sessions: Vec<Uuid> = members.borrow_mut().drain().map(|(id, _)| id).collect();
            tracing::info!(sessions = sessions.len(), "session group window closed");
            for session_id in sessions {
                if let Some(ref callback) = *on_close.borrow() {
                    callback(session_id);
                }
            }
            glib::Propagation::Proceed
        });
    }

    /// Moves the selected tab back to the main window from the header button.
    fn connect_attach_button(&self) {
        let members = Rc::clone(&self.members);
        let tab_view = self.tab_view.clone();
        let on_attach = Rc::clone(&self.on_attach);
        self.attach_button.connect_clicked(move |_| {
            let session = tab_view
                .selected_page()
                .and_then(|page| Self::session_of(&members.borrow(), &page));
            if let Some(session_id) = session
                && let Some(ref callback) = *on_attach.borrow()
            {
                callback(session_id);
            }
        });
    }

    /// Puts keyboard focus into the session of the tab that was switched to.
    fn connect_focus_on_switch(&self) {
        self.tab_view.connect_selected_page_notify(|tab_view| {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                glib::idle_add_local_once(move || {
                    child.child_focus(gtk4::DirectionType::TabForward);
                });
            }
        });
    }

    /// Adds a session as a new tab and selects it.
    ///
    /// `content` is the box handed over by `take_session_content`.
    pub fn add_session(
        &self,
        session_id: Uuid,
        connection_id: Uuid,
        title: &str,
        protocol: &str,
        content: &gtk4::Box,
    ) {
        let page = self.tab_view.append(content);
        page.set_title(title);
        let icon = rustconn_core::protocol::icons::get_protocol_icon_by_name(protocol);
        page.set_icon(Some(&gio::ThemedIcon::new(icon)));
        self.members.borrow_mut().insert(
            session_id,
            Member {
                page,
                connection_id,
            },
        );
        tracing::info!(session = %session_id, "session added to group window");
    }

    /// Drops a session's tab without ending the session.
    ///
    /// For a session that moved back to the main window or has already ended.
    /// Returns whether the session was hosted here.
    pub fn remove_session(&self, session_id: Uuid) -> bool {
        let page = self
            .members
            .borrow()
            .get(&session_id)
            .map(|member| member.page.clone());
        let Some(page) = page else {
            return false;
        };
        self.leaving.borrow_mut().push(session_id);
        self.tab_view.close_page(&page);
        true
    }

    /// Reports whether a session is shown in this window.
    #[must_use]
    pub fn contains(&self, session_id: Uuid) -> bool {
        self.members.borrow().contains_key(&session_id)
    }

    /// Reports whether the window is on its way out and must not get new tabs.
    #[must_use]
    pub fn is_closing(&self) -> bool {
        self.closing.get()
    }

    /// The session of the selected tab.
    #[must_use]
    pub fn selected_session(&self) -> Option<Uuid> {
        let page = self.tab_view.selected_page()?;
        Self::session_of(&self.members.borrow(), &page)
    }

    /// Selects a session's tab and raises the window; `false` when not hosted.
    pub fn present_session(&self, session_id: Uuid) -> bool {
        let page = self
            .members
            .borrow()
            .get(&session_id)
            .map(|member| member.page.clone());
        let Some(page) = page else {
            return false;
        };
        self.tab_view.set_selected_page(&page);
        self.window.present();
        true
    }

    /// Retitles the tabs of a renamed connection; returns how many changed.
    pub fn rename_connection(&self, connection_id: Uuid, new_name: &str) -> usize {
        let pages: Vec<adw::TabPage> = self
            .members
            .borrow()
            .values()
            .filter(|member| member.connection_id == connection_id)
            .map(|member| member.page.clone())
            .collect();
        for page in &pages {
            page.set_title(new_name);
        }
        pages.len()
    }

    /// Marks a session's tab as disconnected, or clears the mark.
    pub fn set_session_disconnected(&self, session_id: Uuid, disconnected: bool) -> bool {
        let page = self
            .members
            .borrow()
            .get(&session_id)
            .map(|member| member.page.clone());
        let Some(page) = page else {
            return false;
        };
        if disconnected {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new("network-offline-symbolic")));
            page.set_indicator_tooltip(&i18n("Session disconnected"));
        } else {
            page.set_indicator_icon(gio::Icon::NONE);
            page.set_indicator_tooltip("");
        }
        true
    }

    /// Closes the selected tab, ending its session.
    pub fn close_selected(&self) {
        if let Some(page) = self.tab_view.selected_page() {
            self.tab_view.close_page(&page);
        }
    }

    /// Selects the tab after (`forward`) or before the current one, wrapping.
    pub fn select_adjacent(&self, forward: bool) {
        let moved = if forward {
            self.tab_view.select_next_page()
        } else {
            self.tab_view.select_previous_page()
        };
        if !moved && self.tab_view.n_pages() > 1 {
            let wrap_to = if forward {
                0
            } else {
                self.tab_view.n_pages() - 1
            };
            self.tab_view
                .set_selected_page(&self.tab_view.nth_page(wrap_to));
        }
    }

    /// Returns the underlying window.
    #[must_use]
    pub const fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    /// Returns the toast overlay, for session-scoped feedback.
    #[must_use]
    pub const fn toast_overlay(&self) -> &adw::ToastOverlay {
        &self.toast_overlay
    }

    /// Shows the window and gives it keyboard focus.
    pub fn present(&self) {
        self.window.present();
    }

    /// Closes the window, ending every session still hosted in it.
    pub fn close(&self) {
        self.window.close();
    }

    /// Sets the callback that ends a session (tab closed, or window closed).
    pub fn set_on_close<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_close.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback that moves a session back to the main window.
    pub fn set_on_attach<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_attach.borrow_mut() = Some(Box::new(callback));
    }
}
