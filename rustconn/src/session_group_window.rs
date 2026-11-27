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
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, gio, glib};
use libadwaita as adw;
use rustconn_core::split::{BalancedSplitStep, balanced_split_plan};
use uuid::Uuid;

use crate::i18n::{i18n, i18n_f, with_accel};

/// Default size of a session group window.
const DEFAULT_WIDTH: i32 = 1100;
const DEFAULT_HEIGHT: i32 = 720;

/// Callback keyed by the session it concerns.
type SessionCallback = Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>;

/// Callback with no argument (the set of hosted tabs changed).
type PlainCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// Callback with the state the user asked for (the broadcast button clicked).
type ToggleCallback = Rc<RefCell<Option<Box<dyn Fn(bool)>>>>;

/// Callback with a session and whether its tile takes part in the broadcast.
type TileBroadcastCallback = Rc<RefCell<Option<Box<dyn Fn(Uuid, bool)>>>>;

/// One hosted session.
struct Member {
    page: adw::TabPage,
    /// The session's content box. It sits in the page's slot box, or in a tile
    /// while the window is tiled.
    content: gtk4::Box,
    connection_id: Uuid,
}

/// Where each tile of the grid goes: the tree the balanced split plan makes.
#[derive(Debug, PartialEq, Eq)]
enum TileNode {
    /// The tile at this index (tab order).
    Leaf(usize),
    Split {
        /// `true` puts `first` above `second`; `false` puts it to the left.
        horizontal: bool,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl TileNode {
    /// Replaces the leaf `target` with a split of it and the new leaf `new`.
    fn split_leaf(&mut self, target: usize, new: usize, horizontal: bool) -> bool {
        match self {
            Self::Leaf(index) if *index == target => {
                *self = Self::Split {
                    horizontal,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(target, new, horizontal)
                    || second.split_leaf(target, new, horizontal)
            }
        }
    }

    /// The leaf indices, left to right and top to bottom.
    #[cfg(test)]
    fn leaves(&self, out: &mut Vec<usize>) {
        match self {
            Self::Leaf(index) => out.push(*index),
            Self::Split { first, second, .. } => {
                first.leaves(out);
                second.leaves(out);
            }
        }
    }
}

/// Turns a balanced split plan into a tree. Leaf `0` is the first tile; step
/// `i` adds leaf `i + 1`.
fn tile_tree(plan: &[BalancedSplitStep]) -> TileNode {
    let mut root = TileNode::Leaf(0);
    for (i, step) in plan.iter().enumerate() {
        root.split_leaf(step.target, i + 1, step.horizontal);
    }
    root
}

/// Tile mode: every tab of the window side by side in a balanced grid.
///
/// A tab page holds a *slot* box and the session content lives in it. In tile
/// mode the content moves into a tile and the tab view stays as it is (hidden),
/// so the selected page, close, move-back and broadcast all work unchanged.
struct TileView {
    tab_view: adw::TabView,
    tab_bar: adw::TabBar,
    stack: gtk4::Stack,
    tiles: gtk4::Box,
    toggle: gtk4::ToggleButton,
    toggle_label: gtk4::Label,
    members: Rc<RefCell<HashMap<Uuid, Member>>>,
    tiled: Cell<bool>,
    /// Sessions the user took out of the broadcast with their tile's check.
    excluded: Rc<RefCell<HashSet<Uuid>>>,
    /// The **Broadcast** check of each tile now shown.
    checks: RefCell<HashMap<Uuid, gtk4::CheckButton>>,
    /// Whether the checks are shown (the broadcast is on, or a tile is out).
    checks_visible: Cell<bool>,
    /// Set while the checks are brought in line with `excluded`, so their
    /// toggled handlers do not report it as a click.
    syncing_checks: Rc<Cell<bool>>,
    on_tile_broadcast: TileBroadcastCallback,
}

impl TileView {
    /// Takes `content` out of the box that holds it.
    fn lift(content: &gtk4::Box) {
        if let Some(parent) = content.parent().and_downcast::<gtk4::Box>() {
            parent.remove(content);
        }
    }

    /// Whether keyboard focus is already inside `content`.
    fn holds_focus(content: &gtk4::Box) -> bool {
        content
            .root()
            .and_downcast::<gtk4::Window>()
            .and_then(|window| gtk4::prelude::RootExt::focus(&window))
            .is_some_and(|focus| {
                focus == *content.upcast_ref::<gtk4::Widget>() || focus.is_ancestor(content)
            })
    }

    fn focus_content(content: &gtk4::Box) {
        let content = content.clone();
        glib::idle_add_local_once(move || {
            content.child_focus(gtk4::DirectionType::TabForward);
        });
    }

    /// The content of the selected tab.
    fn selected_content(&self) -> Option<gtk4::Box> {
        let page = self.tab_view.selected_page()?;
        self.members
            .borrow()
            .values()
            .find(|member| member.page == page)
            .map(|member| member.content.clone())
    }

    /// Shows or hides the toggle and keeps the grid in step with the tabs.
    /// Call it after the set of tabs changed.
    fn refresh(&self) {
        let count = self.members.borrow().len();
        self.toggle.set_visible(count >= 2);
        if self.tiled.get() {
            if count >= 2 {
                self.rebuild();
            } else {
                self.exit();
            }
        }
    }

    fn set_toggle_state(&self, tiled: bool) {
        let (label, tooltip) = if tiled {
            (i18n("Show Tabs"), i18n("Go back to one tab at a time"))
        } else {
            (
                i18n("Tile Tabs"),
                i18n("Show all tabs of this window side by side"),
            )
        };
        self.toggle_label.set_label(&label);
        self.toggle.set_tooltip_text(Some(&tooltip));
        self.toggle
            .update_property(&[gtk4::accessible::Property::Label(&label)]);
        self.toggle.set_active(tiled);
    }

    fn enter(&self) {
        if self.tiled.get() || self.members.borrow().len() < 2 {
            self.set_toggle_state(self.tiled.get());
            return;
        }
        self.tiled.set(true);
        self.tab_bar.set_visible(false);
        self.stack.set_visible_child_name("tiles");
        self.set_toggle_state(true);
        self.rebuild();
    }

    fn exit(&self) {
        self.tiled.set(false);
        for member in self.members.borrow().values() {
            Self::lift(&member.content);
            if let Some(slot) = member.page.child().downcast_ref::<gtk4::Box>() {
                slot.append(&member.content);
            }
        }
        self.clear_tiles();
        self.stack.set_visible_child_name("tabs");
        self.tab_bar.set_visible(true);
        self.set_toggle_state(false);
        if let Some(content) = self.selected_content() {
            Self::focus_content(&content);
        }
    }

    fn clear_tiles(&self) {
        self.checks.borrow_mut().clear();
        while let Some(child) = self.tiles.first_child() {
            self.tiles.remove(&child);
        }
    }

    /// Builds the grid again from the tabs that are left, in tab order.
    fn rebuild(&self) {
        let mut ordered: Vec<(i32, Uuid, adw::TabPage, gtk4::Box)> = self
            .members
            .borrow()
            .iter()
            .map(|(sid, member)| {
                (
                    self.tab_view.page_position(&member.page),
                    *sid,
                    member.page.clone(),
                    member.content.clone(),
                )
            })
            .collect();
        ordered.sort_by_key(|(position, _, _, _)| *position);
        for (_, _, _, content) in &ordered {
            Self::lift(content);
        }
        self.clear_tiles();
        if ordered.is_empty() {
            return;
        }

        let plan = balanced_split_plan(
            f64::from(self.stack.width()),
            f64::from(self.stack.height()),
            ordered.len() - 1,
        );
        let cells: Vec<gtk4::Box> = ordered
            .iter()
            .map(|(_, sid, page, content)| self.build_tile(*sid, page, content))
            .collect();
        let root = Self::build_widget(&tile_tree(&plan), &cells);
        self.tiles.append(&root);
        if let Some(content) = self.selected_content() {
            Self::focus_content(&content);
        }
    }

    /// One tile: a narrow title (tab name, disconnected mark) over the session.
    fn build_tile(&self, sid: Uuid, page: &adw::TabPage, content: &gtk4::Box) -> gtk4::Box {
        let cell = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        cell.add_css_class("split-panel");
        cell.set_hexpand(true);
        cell.set_vexpand(true);

        let title = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        title.set_margin_start(6);
        title.set_margin_end(6);
        let label = gtk4::Label::new(None);
        label.add_css_class("caption");
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        page.bind_property("title", &label, "label")
            .sync_create()
            .build();
        let indicator = gtk4::Image::new();
        indicator.set_pixel_size(12);
        page.bind_property("indicator-icon", &indicator, "gicon")
            .sync_create()
            .build();
        page.bind_property("indicator-icon", &indicator, "visible")
            .sync_create()
            .transform_to(|_, icon: Option<gio::Icon>| Some(icon.is_some()))
            .build();
        page.bind_property("indicator-tooltip", &indicator, "tooltip-text")
            .sync_create()
            .build();
        title.append(&label);
        title.append(&indicator);

        // Takes this tile out of the cluster window's broadcast, and back in.
        let check = gtk4::CheckButton::with_label(&i18n("Broadcast"));
        check.set_active(!self.excluded.borrow().contains(&sid));
        check.set_visible(self.checks_visible.get());
        check.set_focus_on_click(false);
        check.set_tooltip_text(Some(&i18n(
            "Untick to keep this pane out of the keystroke broadcast",
        )));
        let callback = Rc::clone(&self.on_tile_broadcast);
        let syncing = Rc::clone(&self.syncing_checks);
        check.connect_toggled(move |check| {
            if syncing.get() {
                return;
            }
            if let Some(ref callback) = *callback.borrow() {
                callback(sid, check.is_active());
            }
        });
        title.append(&check);
        self.checks.borrow_mut().insert(sid, check);
        cell.append(&title);

        content.set_vexpand(true);
        cell.append(content);

        // The focused tile is the selected tab, so every action that works on
        // the selected tab acts on it.
        let focus = gtk4::EventControllerFocus::new();
        let tab_view = self.tab_view.clone();
        let page = page.clone();
        let cell_weak = cell.downgrade();
        focus.connect_enter(move |_| {
            tab_view.set_selected_page(&page);
            if let Some(cell) = cell_weak.upgrade() {
                cell.add_css_class("focused-panel");
            }
        });
        let cell_weak = cell.downgrade();
        focus.connect_leave(move |_| {
            if let Some(cell) = cell_weak.upgrade() {
                cell.remove_css_class("focused-panel");
            }
        });
        cell.add_controller(focus);
        cell
    }

    /// Shows or hides the **Broadcast** check of every tile, ticked for every
    /// tab that is not kept out.
    fn set_checks_visible(&self, visible: bool) {
        self.checks_visible.set(visible);
        let excluded = self.excluded.borrow().clone();
        self.syncing_checks.set(true);
        for (sid, check) in self.checks.borrow().iter() {
            check.set_active(!excluded.contains(sid));
            check.set_visible(visible);
        }
        self.syncing_checks.set(false);
    }

    fn build_widget(node: &TileNode, cells: &[gtk4::Box]) -> gtk4::Widget {
        match node {
            TileNode::Leaf(index) => cells[*index].clone().upcast(),
            TileNode::Split {
                horizontal,
                first,
                second,
            } => {
                let orientation = if *horizontal {
                    gtk4::Orientation::Vertical
                } else {
                    gtk4::Orientation::Horizontal
                };
                let paned = gtk4::Paned::new(orientation);
                paned.set_hexpand(true);
                paned.set_vexpand(true);
                paned.set_resize_start_child(true);
                paned.set_resize_end_child(true);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);
                paned.set_start_child(Some(&Self::build_widget(first, cells)));
                paned.set_end_child(Some(&Self::build_widget(second, cells)));
                // Halves, once the size is known; the user may drag it later.
                let placed = Cell::new(false);
                paned.connect_notify_local(Some("max-position"), move |paned, _| {
                    let max = paned.max_position();
                    if !placed.get() && max > 0 {
                        placed.set(true);
                        paned.set_position(max / 2);
                    }
                });
                paned.upcast()
            }
        }
    }
}

/// Sessions a window would end: each tab counts its own weight, at least one.
fn live_sessions(tabs: impl Iterator<Item = Uuid>, weight: impl Fn(Uuid) -> usize) -> usize {
    tabs.map(|id| weight(id).max(1)).sum()
}

/// Whether closing a tab must be confirmed first (pure).
///
/// Only the user's close of a tab that holds a split of two or more sessions
/// asks, as in the main window. A tab dropped without teardown (`leaving`) and
/// one the user already confirmed close at once; a one-session tab closes as
/// before.
const fn split_tab_needs_confirm(sessions_in_tab: usize, leaving: bool, confirmed: bool) -> bool {
    sessions_in_tab >= 2 && !leaving && !confirmed
}

/// Asks before ending sessions: **Cancel** (default) or **Close All Sessions**.
fn ask_end_sessions(
    window: &adw::ApplicationWindow,
    heading: &str,
    body: &str,
    on_answer: impl Fn(bool) + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("close", &i18n("Close All Sessions"));
    dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        on_answer(response == "close");
    });
    dialog.present(Some(window));
}

/// A top-level window with a tab per hosted session.
pub struct SessionGroupWindow {
    window: adw::ApplicationWindow,
    tab_view: adw::TabView,
    toast_overlay: adw::ToastOverlay,
    members: Rc<RefCell<HashMap<Uuid, Member>>>,
    /// Tile mode (issue #436). Not remembered between openings of the window.
    tiling: Rc<TileView>,
    /// Sessions whose page is being dropped because the session moved away or
    /// already ended, so the `close-page` handler must not end it (again).
    leaving: Rc<RefCell<Vec<Uuid>>>,
    /// Split tabs whose close the user confirmed; consumed by `close-page`.
    confirmed: Rc<RefCell<HashSet<Uuid>>>,
    /// Split tabs with a close question open, so a second click asks nothing.
    asking: Rc<RefCell<HashSet<Uuid>>>,
    /// Set once the window itself is closing, so emptying it does not ask it to
    /// close a second time.
    closing: Rc<Cell<bool>>,
    /// Kept alive for its `clicked` handler.
    attach_button: Button,
    /// Header toggle for broadcasting to every tab of this window (issue #436).
    /// Shown only while two or more tabs can take part.
    broadcast_toggle: gtk4::ToggleButton,
    /// Shown while that broadcast is on, because it reaches tabs that are not
    /// on screen. Its **Stop** button turns the broadcast off.
    broadcast_banner: adw::Banner,
    on_close: SessionCallback,
    /// How many sessions a tab holds: 1, or the panes of a moved split.
    session_weight: Rc<RefCell<Option<Box<dyn Fn(Uuid) -> usize>>>>,
    on_attach: SessionCallback,
    on_members_changed: PlainCallback,
    on_broadcast_toggled: ToggleCallback,
    /// Sessions kept out of the broadcast; shared with the tile view.
    excluded: Rc<RefCell<HashSet<Uuid>>>,
    /// Whether enough terminal tabs are here for per-tile **Broadcast** checks
    /// (see [`crate::split_view::MIN_PANES_FOR_BROADCAST_CHECKS`]).
    checks_offered: Cell<bool>,
    on_tile_broadcast: TileBroadcastCallback,
}

impl SessionGroupWindow {
    /// Creates an empty window titled after the group it shows.
    ///
    /// An empty `title` makes a window of no particular group, such as the one a
    /// tab dropped outside every window opens; it is called RustConn.
    ///
    /// The window is not presented yet; add a session and call
    /// [`Self::present`].
    #[must_use]
    pub fn new(app: &adw::Application, title: &str) -> Rc<Self> {
        let (window_title, title) = if title.is_empty() {
            ("RustConn".to_owned(), "RustConn")
        } else {
            (i18n_f("{} — RustConn", &[title]), title)
        };
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(window_title)
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
        // A window's only tab can still be dragged to another window.
        crate::terminal::drag_single_tab(&tab_bar, &tab_view, |_| true);

        let attach_button = Button::from_icon_name("view-restore-symbolic");
        let attach_label = i18n("Move to Main Window");
        attach_button.set_tooltip_text(Some(&with_accel(&attach_label, "win.toggle-detach")));
        attach_button.update_property(&[gtk4::accessible::Property::Label(&attach_label)]);

        let broadcast_toggle = Self::build_broadcast_toggle();
        let broadcast_banner = crate::toast::plain_banner("");
        broadcast_banner.set_button_label(Some(&i18n("Stop")));
        broadcast_banner.add_css_class("accent");

        let (tile_toggle, tile_label) = Self::build_tile_toggle();
        let stack = gtk4::Stack::new();
        let tiles = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        stack.add_named(&tab_view, Some("tabs"));
        stack.add_named(&tiles, Some("tiles"));
        stack.set_visible_child_name("tabs");
        let members = Rc::new(RefCell::new(HashMap::new()));
        let excluded: Rc<RefCell<HashSet<Uuid>>> = Rc::new(RefCell::new(HashSet::new()));
        let on_tile_broadcast: TileBroadcastCallback = Rc::new(RefCell::new(None));
        let tiling = Rc::new(TileView {
            tab_view: tab_view.clone(),
            tab_bar: tab_bar.clone(),
            stack: stack.clone(),
            tiles,
            toggle: tile_toggle.clone(),
            toggle_label: tile_label,
            members: Rc::clone(&members),
            tiled: Cell::new(false),
            excluded: Rc::clone(&excluded),
            checks: RefCell::new(HashMap::new()),
            checks_visible: Cell::new(false),
            syncing_checks: Rc::new(Cell::new(false)),
            on_tile_broadcast: Rc::clone(&on_tile_broadcast),
        });

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(title, "")));
        header.pack_start(&attach_button);
        header.pack_end(&broadcast_toggle);
        header.pack_end(&tile_toggle);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.add_top_bar(&broadcast_banner);
        // The tab bar sits in the content, not among the top bars: those are
        // wrapped in a GtkWindowHandle, which turned dragging a tab out into
        // moving the window as well. The main window keeps its tab bar out of
        // a handle the same way.
        let body = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        stack.set_vexpand(true);
        body.append(&tab_bar);
        body.append(&stack);
        toolbar_view.set_content(Some(&body));

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
            members,
            tiling,
            leaving: Rc::new(RefCell::new(Vec::new())),
            confirmed: Rc::new(RefCell::new(HashSet::new())),
            asking: Rc::new(RefCell::new(HashSet::new())),
            closing: Rc::new(Cell::new(false)),
            attach_button,
            broadcast_toggle,
            broadcast_banner,
            on_close: Rc::new(RefCell::new(None)),
            session_weight: Rc::new(RefCell::new(None)),
            on_attach: Rc::new(RefCell::new(None)),
            on_members_changed: Rc::new(RefCell::new(None)),
            on_broadcast_toggled: Rc::new(RefCell::new(None)),
            excluded,
            checks_offered: Cell::new(false),
            on_tile_broadcast,
        });
        group.connect_close_page();
        group.connect_close_request();
        group.connect_attach_button();
        group.connect_broadcast_controls();
        group.connect_tile_toggle(&tile_toggle);
        group.connect_focus_on_switch();
        group.keep_tabs_here();
        tracing::info!(group = title, "session group window created");
        group
    }

    /// The header toggle for tile mode; hidden until two tabs exist.
    fn build_tile_toggle() -> (gtk4::ToggleButton, gtk4::Label) {
        let toggle = gtk4::ToggleButton::new();
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
        icon.set_pixel_size(16);
        let label = gtk4::Label::new(Some(&i18n("Tile Tabs")));
        label.add_css_class("caption");
        content.append(&icon);
        content.append(&label);
        toggle.set_child(Some(&content));
        toggle.set_tooltip_text(Some(&i18n("Show all tabs of this window side by side")));
        toggle.update_property(&[gtk4::accessible::Property::Label(&i18n("Tile Tabs"))]);
        toggle.add_css_class("pill");
        toggle.set_visible(false);
        (toggle, label)
    }

    /// Enters or leaves tile mode when the user clicks the toggle.
    fn connect_tile_toggle(&self, toggle: &gtk4::ToggleButton) {
        let tiling = Rc::downgrade(&self.tiling);
        toggle.connect_clicked(move |toggle| {
            if let Some(tiling) = tiling.upgrade() {
                if toggle.is_active() {
                    tiling.enter();
                } else {
                    tiling.exit();
                }
            }
        });
    }

    /// The header toggle, styled like the main window's **Group Broadcast**.
    fn build_broadcast_toggle() -> gtk4::ToggleButton {
        let toggle = gtk4::ToggleButton::new();
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let icon = gtk4::Image::from_icon_name("network-transmit-receive-symbolic");
        icon.set_pixel_size(16);
        let label = gtk4::Label::new(Some(&i18n("Tab Broadcast")));
        label.add_css_class("caption");
        content.append(&icon);
        content.append(&label);
        toggle.set_child(Some(&content));
        toggle.set_tooltip_text(Some(&with_accel(
            &i18n("Mirror keystrokes to all tabs in this window"),
            "win.toggle-group-broadcast",
        )));
        toggle.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Toggle Tab Broadcast",
        ))]);
        toggle.add_css_class("pill");
        toggle.set_visible(false);
        toggle
    }

    /// Reports clicks on the toggle and the banner's **Stop** as the state the
    /// user asked for; the owner decides and calls [`Self::show_broadcast`].
    fn connect_broadcast_controls(&self) {
        let on_toggled = Rc::clone(&self.on_broadcast_toggled);
        self.broadcast_toggle.connect_clicked(move |toggle| {
            if let Some(ref callback) = *on_toggled.borrow() {
                callback(toggle.is_active());
            }
        });
        let on_toggled = Rc::clone(&self.on_broadcast_toggled);
        self.broadcast_banner.connect_button_clicked(move |_| {
            if let Some(ref callback) = *on_toggled.borrow() {
                callback(false);
            }
        });
    }

    /// Runs the members-changed callback.
    fn notify_members_changed(callback: &PlainCallback) {
        if let Some(ref callback) = *callback.borrow() {
            callback();
        }
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
        let on_members_changed = Rc::clone(&self.on_members_changed);
        let tiling = Rc::downgrade(&self.tiling);
        let window = self.window.downgrade();
        let weight = Rc::clone(&self.session_weight);
        let confirmed = Rc::clone(&self.confirmed);
        let asking = Rc::clone(&self.asking);
        self.tab_view.connect_close_page(move |tab_view, page| {
            let session = Self::session_of(&members.borrow(), page);
            if let Some(session_id) = session {
                // A moved split ends with every session it shows, so the user
                // is asked first. The window closing for good asks on its own.
                let sessions_in_tab = weight.borrow().as_ref().map_or(1, |weigh| weigh(session_id));
                let was_confirmed = confirmed.borrow_mut().remove(&session_id);
                if !closing.get()
                    && split_tab_needs_confirm(
                        sessions_in_tab,
                        leaving.borrow().contains(&session_id),
                        was_confirmed,
                    )
                    && let Some(window) = window.upgrade()
                {
                    // Cancel and every answer leave the tab as it was; a
                    // confirmed close comes back through this handler.
                    tab_view.close_page_finish(page, false);
                    if asking.borrow_mut().insert(session_id) {
                        let only_tab = members.borrow().len() == 1;
                        let (heading, body) = if only_tab {
                            (
                                i18n("Close this window?"),
                                i18n_f(
                                    "This ends all {} sessions in this window.",
                                    &[&sessions_in_tab.to_string()],
                                ),
                            )
                        } else {
                            (
                                i18n("Close Split Tab?"),
                                i18n_f(
                                    "This ends all {} sessions in this split.",
                                    &[&sessions_in_tab.to_string()],
                                ),
                            )
                        };
                        let (tab_view, page) = (tab_view.clone(), page.clone());
                        let (confirmed, asking) = (Rc::clone(&confirmed), Rc::clone(&asking));
                        ask_end_sessions(&window, &heading, &body, move |close| {
                            asking.borrow_mut().remove(&session_id);
                            if close {
                                confirmed.borrow_mut().insert(session_id);
                                tab_view.close_page(&page);
                            }
                        });
                    }
                    return glib::Propagation::Stop;
                }
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
            if session.is_some() {
                if let Some(tiling) = tiling.upgrade() {
                    tiling.refresh();
                }
                Self::notify_members_changed(&on_members_changed);
            }

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

    /// Ends every hosted session when the user closes the window, after asking
    /// when two or more are still live (as the split-close dialog does).
    fn connect_close_request(&self) {
        let members = Rc::clone(&self.members);
        let closing = Rc::clone(&self.closing);
        let on_close = Rc::clone(&self.on_close);
        let weight = Rc::clone(&self.session_weight);
        self.window.connect_close_request(move |window| {
            let live = live_sessions(members.borrow().keys().copied(), |id| {
                weight.borrow().as_ref().map_or(1, |weigh| weigh(id))
            });
            if !closing.get() && live >= 2 {
                let window_weak = window.downgrade();
                let closing = Rc::clone(&closing);
                ask_end_sessions(
                    window,
                    &i18n("Close this window?"),
                    &i18n_f(
                        "This ends all {} sessions in this window.",
                        &[&live.to_string()],
                    ),
                    move |close| {
                        if close && let Some(window) = window_weak.upgrade() {
                            // Set before closing, so the request goes through.
                            closing.set(true);
                            window.close();
                        }
                    },
                );
                return glib::Propagation::Stop;
            }
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

    /// Keeps every hosted tab's bookkeeping in this window.
    ///
    /// A drag would carry a page past the bookkeeping of both windows, so a tab
    /// dragged onto another window's tab bar is handed back first; the window
    /// layer then moves its session there properly (see
    /// `crate::terminal::set_drop_handler`). A tab dropped outside every window
    /// gets a new window the same way.
    fn keep_tabs_here(&self) {
        let members = Rc::clone(&self.members);
        crate::terminal::return_transferred_tabs(&self.tab_view, move |page| {
            members
                .try_borrow()
                .is_ok_and(|members| Self::session_of(&members, page).is_some())
        });
        self.tab_view
            .connect_create_window(|_| crate::terminal::new_window_view());
    }

    /// Puts keyboard focus into the session of the tab that was switched to.
    fn connect_focus_on_switch(&self) {
        let tiling = Rc::downgrade(&self.tiling);
        self.tab_view.connect_selected_page_notify(move |tab_view| {
            if let Some(page) = tab_view.selected_page() {
                if let Some(tiling) = tiling.upgrade()
                    && tiling.tiled.get()
                {
                    // The session is in a tile; a click there selected this
                    // page, and a key switch has to move focus to the tile.
                    if let Some(content) = tiling.selected_content()
                        && !TileView::holds_focus(&content)
                    {
                        TileView::focus_content(&content);
                    }
                    return;
                }
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
        icon: &str,
        content: &gtk4::Box,
    ) {
        let slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        slot.append(content);
        let page = self.tab_view.append(&slot);
        page.set_title(title);
        // Custom icon from the connection's settings (emoji or GTK icon name),
        // else the protocol icon — the sidebar row makes the same choice.
        let protocol_icon = rustconn_core::protocol::icons::get_protocol_icon_by_name(protocol);
        page.set_icon(Some(&crate::icon_render::tab_icon(icon, protocol_icon)));
        self.members.borrow_mut().insert(
            session_id,
            Member {
                page,
                content: content.clone(),
                connection_id,
            },
        );
        tracing::info!(session = %session_id, "session added to group window");
        self.tiling.refresh();
        Self::notify_members_changed(&self.on_members_changed);
    }

    /// The view holding this window's tabs.
    #[must_use]
    pub const fn tab_view(&self) -> &adw::TabView {
        &self.tab_view
    }

    /// The session a page of this window shows.
    #[must_use]
    pub fn session_for_page(&self, page: &adw::TabPage) -> Option<Uuid> {
        Self::session_of(&self.members.borrow(), page)
    }

    /// The content box of a hosted session, as `add_session` received it.
    #[must_use]
    pub fn session_content(&self, session_id: Uuid) -> Option<gtk4::Box> {
        self.members
            .borrow()
            .get(&session_id)
            .map(|member| member.content.clone())
    }

    /// Moves a session's tab to `position` (clamped to the view) and selects it.
    pub fn place_session(&self, session_id: Uuid, position: i32) {
        let page = self
            .members
            .borrow()
            .get(&session_id)
            .map(|member| member.page.clone());
        if let Some(page) = page {
            let position = crate::terminal::clamp_drop_position(
                position,
                self.tab_view.n_pinned_pages(),
                self.tab_view.n_pages() - 1,
            );
            self.tab_view.reorder_page(&page, position);
            self.tab_view.set_selected_page(&page);
        }
    }

    /// Turns this window's Tab Broadcast off, if it is on.
    ///
    /// For a tab dragged in: it must not start receiving keystrokes typed for
    /// the tabs that were already here.
    pub fn stop_broadcast(&self) {
        if self.broadcast_toggle.is_active()
            && let Some(ref callback) = *self.on_broadcast_toggled.borrow()
        {
            callback(false);
        }
    }

    /// Whether no session is hosted here.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.borrow().is_empty()
    }

    /// Every hosted session (order unspecified).
    #[must_use]
    pub fn sessions(&self) -> Vec<Uuid> {
        self.members.borrow().keys().copied().collect()
    }

    /// Shows the broadcast toggle when `available`, and its state.
    ///
    /// `count` is how many tabs the broadcast reaches; it names them in the
    /// banner, which is revealed while `active`.
    pub fn show_broadcast(&self, available: bool, active: bool, count: usize) {
        self.broadcast_toggle.set_visible(available || active);
        self.broadcast_toggle.set_active(active);
        if active {
            self.broadcast_toggle.add_css_class("broadcasting");
            self.broadcast_banner.set_title(&i18n_f(
                "Tab Broadcast active — keystrokes mirrored to {} tabs in this window",
                &[&count.to_string()],
            ));
        } else {
            self.broadcast_toggle.remove_css_class("broadcasting");
        }
        self.broadcast_banner.set_revealed(active);
        // The tile checks stay while a tile is out, so it can be put back. With
        // fewer than three terminal tabs they are not offered at all: the
        // header-bar button already covers two.
        self.tiling.set_checks_visible(
            self.checks_offered.get() && (active || !self.excluded.borrow().is_empty()),
        );
    }

    /// Sessions the user took out of this window's broadcast.
    #[must_use]
    pub fn excluded_sessions(&self) -> HashSet<Uuid> {
        self.excluded.borrow().clone()
    }

    /// Forgets exclusions of sessions that are no longer hosted here.
    ///
    /// `hosted` are the terminal tabs here. Below
    /// [`crate::split_view::MIN_PANES_FOR_BROADCAST_CHECKS`] of them no tile
    /// gets a check, so every exclusion is dropped: no tab stays out with no
    /// check to put it back.
    pub fn retain_excluded(&self, hosted: &HashSet<Uuid>) {
        let offered = hosted.len() >= crate::split_view::MIN_PANES_FOR_BROADCAST_CHECKS;
        self.checks_offered.set(offered);
        self.excluded
            .borrow_mut()
            .retain(|sid| offered && hosted.contains(sid));
    }

    /// Sets the callback run when a tile's **Broadcast** check is clicked. The
    /// session is recorded as in or out before the callback runs.
    pub fn set_on_tile_broadcast<F>(&self, callback: F)
    where
        F: Fn(Uuid, bool) + 'static,
    {
        let excluded = Rc::clone(&self.excluded);
        *self.on_tile_broadcast.borrow_mut() = Some(Box::new(move |sid, included| {
            if included {
                excluded.borrow_mut().remove(&sid);
            } else {
                excluded.borrow_mut().insert(sid);
            }
            callback(sid, included);
        }));
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
        // A programmatic close (app shutdown) is not asked about.
        self.closing.set(true);
        self.window.close();
    }

    /// Sets the callback that ends a session (tab closed, or window closed).
    pub fn set_on_close<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_close.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets how many sessions a tab holds, so a moved split counts its panes.
    pub fn set_session_weight<F>(&self, weight: F)
    where
        F: Fn(Uuid) -> usize + 'static,
    {
        *self.session_weight.borrow_mut() = Some(Box::new(weight));
    }

    /// Retitles one session's tab, as the main window's tab follows a split's
    /// focused pane. Returns whether the session is hosted here.
    pub fn set_session_title(&self, session_id: Uuid, title: &str) -> bool {
        let page = self
            .members
            .borrow()
            .get(&session_id)
            .map(|member| member.page.clone());
        page.is_some_and(|page| {
            page.set_title(title);
            true
        })
    }

    /// Sets the callback that moves a session back to the main window.
    pub fn set_on_attach<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_attach.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback run after a tab is added or dropped.
    pub fn set_on_members_changed<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_members_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback run when the user asks to turn the broadcast on or
    /// off, from the header toggle or the banner's **Stop**.
    pub fn set_on_broadcast_toggled<F>(&self, callback: F)
    where
        F: Fn(bool) + 'static,
    {
        *self.on_broadcast_toggled.borrow_mut() = Some(Box::new(callback));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves_of(count: usize) -> Vec<usize> {
        let plan = balanced_split_plan(1280.0, 720.0, count - 1);
        let mut leaves = Vec::new();
        tile_tree(&plan).leaves(&mut leaves);
        leaves
    }

    #[test]
    fn only_a_users_close_of_a_split_tab_asks() {
        assert!(split_tab_needs_confirm(2, false, false));
        assert!(split_tab_needs_confirm(5, false, false));
        assert!(!split_tab_needs_confirm(1, false, false));
        assert!(!split_tab_needs_confirm(0, false, false));
        assert!(!split_tab_needs_confirm(3, true, false));
        assert!(!split_tab_needs_confirm(3, false, true));
    }

    #[test]
    fn a_split_tab_counts_every_pane() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let weight = |id| if id == a { 3 } else { 0 };
        assert_eq!(live_sessions([a].into_iter(), weight), 3);
        assert_eq!(live_sessions([a, b].into_iter(), weight), 4);
        assert_eq!(live_sessions(std::iter::empty(), weight), 0);
    }

    #[test]
    fn one_tab_is_a_single_leaf() {
        assert_eq!(tile_tree(&[]), TileNode::Leaf(0));
    }

    #[test]
    fn two_tabs_split_the_wide_area_side_by_side() {
        let plan = balanced_split_plan(1280.0, 720.0, 1);
        assert_eq!(
            tile_tree(&plan),
            TileNode::Split {
                horizontal: false,
                first: Box::new(TileNode::Leaf(0)),
                second: Box::new(TileNode::Leaf(1)),
            }
        );
    }

    #[test]
    fn every_tab_gets_exactly_one_tile() {
        for count in 1..=9 {
            let mut leaves = leaves_of(count);
            leaves.sort_unstable();
            assert_eq!(leaves, (0..count).collect::<Vec<_>>(), "{count} tabs");
        }
    }

    /// Entering and leaving tile mode hands each session's content back to its
    /// own slot, and closing a tile rebuilds the grid from the tabs left.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact session_group_window::tests::tile_and_untile_keep_every_content`"]
    fn tile_and_untile_keep_every_content() {
        if libadwaita::init().is_err() {
            return;
        }
        let tab_view = adw::TabView::new();
        let tab_bar = adw::TabBar::builder().view(&tab_view).build();
        let stack = gtk4::Stack::new();
        let tiles = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        stack.add_named(&tab_view, Some("tabs"));
        stack.add_named(&tiles, Some("tiles"));
        let members = Rc::new(RefCell::new(HashMap::new()));
        let view = TileView {
            tab_view: tab_view.clone(),
            tab_bar,
            stack,
            tiles,
            toggle: gtk4::ToggleButton::new(),
            toggle_label: gtk4::Label::new(None),
            members: Rc::clone(&members),
            tiled: Cell::new(false),
            excluded: Rc::new(RefCell::new(HashSet::new())),
            checks: RefCell::new(HashMap::new()),
            checks_visible: Cell::new(false),
            syncing_checks: Rc::new(Cell::new(false)),
            on_tile_broadcast: Rc::new(RefCell::new(None)),
        };
        let add = |name: &str| {
            let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            let slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            slot.append(&content);
            let page = tab_view.append(&slot);
            page.set_title(name);
            let id = Uuid::new_v4();
            members.borrow_mut().insert(
                id,
                Member {
                    page,
                    content: content.clone(),
                    connection_id: Uuid::new_v4(),
                },
            );
            (id, content, slot)
        };
        let (first, content_a, slot_a) = add("a");
        let (_, content_b, slot_b) = add("b");
        view.refresh();
        assert!(view.toggle.is_visible());

        view.enter();
        assert!(view.tiled.get());
        assert!(content_a.parent().is_some_and(|p| p != slot_a));
        assert!(content_b.parent().is_some_and(|p| p != slot_b));

        // A third tab joins: the grid is rebuilt and holds all three.
        let (_, content_c, _) = add("c");
        view.refresh();
        assert!(content_c.parent().is_some());
        assert!(content_a.parent().is_some_and(|p| p != slot_a));

        // One tab goes away: two tiles are left.
        members.borrow_mut().remove(&first);
        view.refresh();
        assert!(view.tiled.get());

        view.exit();
        assert!(!view.tiled.get());
        assert_eq!(content_b.parent().as_ref(), Some(slot_b.upcast_ref()));
        assert_eq!(view.tiles.first_child(), None);
    }
}
