//! Tab context menu setup and population.
//!
//! Extracted from `terminal/mod.rs` to reduce module complexity.

use gtk4::gdk;
use rustconn_core::DetachVerdict;
use rustconn_core::activity_monitor::MonitorMode;

use super::*;

/// The facts about the right-clicked tab that the context menu adapts to.
///
/// Bundled rather than passed as a row of booleans so the call site names every
/// flag it sets.
#[derive(Debug, Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "five independent per-tab facts, each gating one menu section"
)]
pub struct TabMenuState {
    /// Activity or silence monitoring mode of the tab's session, if any.
    pub monitor_mode: Option<MonitorMode>,
    /// The tab belongs to a tab group.
    pub has_group: bool,
    /// The tab is pinned.
    pub is_pinned: bool,
    /// At least one tab in the window belongs to a group.
    pub any_groups_exist: bool,
    /// The tab's session may be offered a move to its own window.
    pub can_detach: bool,
    /// The tab hosts a split layout, so it can be offered "Remove Split".
    pub hosts_split: bool,
}

/// Reports whether the detach section is offered for a verdict.
///
/// A split owner keeps the item: activating it explains that the split layout
/// has to be removed first (Requirement 4.3), which is clearer than a silently
/// missing entry. Every other blocking verdict hides the whole section, so no
/// inert item is ever shown (Requirement 3.2), and a page with no session — the
/// Welcome tab — never reaches this function at all (Requirement 4.5).
const fn offers_detach(verdict: DetachVerdict) -> bool {
    matches!(verdict, DetachVerdict::Allowed | DetachVerdict::SplitOwner)
}

impl TerminalNotebook {
    /// Sets up the tab context menu with group management actions.
    ///
    /// The menu is shown on right-click via `adw::TabView::set_menu_model`.
    /// The `setup-menu` signal stores the target page so actions can find it.
    pub(crate) fn setup_tab_context_menu(&self) {
        // Stable GMenu instance — set as the TabView menu model once.
        // The `connect_setup_menu` callback clears and re-populates its
        // items before each show.  Because the *same* GMenu object stays
        // registered, the popover's reference is never invalidated — this
        // prevents the SIGSEGV that occurred when `set_menu_model()` was
        // called repeatedly with a brand-new GMenu each time.
        let menu = gio::Menu::new();
        self.tab_view.set_menu_model(Some(&menu));

        // Shared cell to store the page that was right-clicked
        let context_page: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));

        let context_page_setup = context_page.clone();
        let sessions_for_menu = self.sessions.clone();
        let session_info_for_menu = self.session_info.clone();
        let activity_for_menu = self.activity_coordinator.clone();
        let detach_hooks_for_menu = self.detach_hooks();
        let menu_for_setup = menu;
        self.tab_view.connect_setup_menu(move |_tab_view, page| {
            *context_page_setup.borrow_mut() = page.cloned();

            // Determine the current monitor mode, group membership and
            // detachability for the right-clicked tab
            let state = page
                .map(|page| {
                    let sessions = sessions_for_menu.borrow();
                    let session_id = sessions.iter().find(|(_, p)| *p == page).map(|(id, _)| *id);
                    let mode = session_id.and_then(|sid| {
                        let coordinator = activity_for_menu.borrow();
                        let coordinator = coordinator.as_ref()?;
                        coordinator.get_mode(sid)
                    });
                    // A page with no session (the Welcome tab) is never
                    // detachable, so the verdict is only asked for real sessions.
                    // The same verdict already distinguishes a tab that hosts a
                    // split layout, so "Remove Split" reuses it rather than
                    // re-deriving split membership from the colour map.
                    let verdict = session_id.map(|sid| detach_hooks_for_menu.verdict(sid));
                    let can_detach = verdict.is_some_and(offers_detach);
                    let hosts_split = verdict == Some(DetachVerdict::SplitOwner);
                    let info_ref = session_info_for_menu.borrow();
                    let has_group = session_id
                        .and_then(|sid| info_ref.get(&sid).and_then(|i| i.tab_group.clone()))
                        .is_some();
                    TabMenuState {
                        monitor_mode: mode,
                        has_group,
                        is_pinned: page.is_pinned(),
                        // Check if ANY tab has a group assigned (for showing
                        // group-related actions)
                        any_groups_exist: info_ref.values().any(|i| i.tab_group.is_some()),
                        can_detach,
                        hosts_split,
                    }
                })
                .unwrap_or_default();

            // Mutate the existing menu in-place (clear + re-populate)
            menu_for_setup.remove_all();
            Self::populate_tab_context_menu(&menu_for_setup, state);
        });

        // Create action group
        let action_group = gio::SimpleActionGroup::new();

        // "Set Group..." action — shows an entry dialog
        let set_group_action = gio::SimpleAction::new("set-group", None);
        let context_page_set = context_page.clone();
        let session_info = self.session_info.clone();
        let sessions = self.sessions.clone();
        let tab_group_manager = self.tab_group_manager.clone();

        set_group_action.connect_activate(move |_, _| {
            let target_page = context_page_set.borrow().clone();
            let Some(target_page) = target_page else {
                return;
            };
            let session_id = {
                let sessions_ref = sessions.borrow();
                sessions_ref
                    .iter()
                    .find(|(_, p)| *p == &target_page)
                    .map(|(id, _)| *id)
            };
            let Some(session_id) = session_id else {
                return;
            };

            // Build the group chooser dialog
            let dialog = adw::AlertDialog::builder()
                .heading(i18n("Set Tab Group"))
                .build();

            let content_box = GtkBox::new(Orientation::Vertical, 12);

            // Show existing groups as clickable buttons
            let known_groups = tab_group_manager.borrow().group_names();
            let entry = gtk4::Entry::builder()
                .placeholder_text(i18n("New group name…"))
                .hexpand(true)
                .build();

            if known_groups.is_empty() {
                let label = gtk4::Label::new(Some(&i18n("Enter a group name for this tab")));
                label.set_halign(gtk4::Align::Start);
                label.add_css_class("dim-label");
                content_box.append(&label);
            } else {
                let groups_label = gtk4::Label::new(Some(&i18n("Existing groups:")));
                groups_label.set_halign(gtk4::Align::Start);
                groups_label.add_css_class("dim-label");
                content_box.append(&groups_label);

                let flow_box = gtk4::FlowBox::new();
                flow_box.set_selection_mode(gtk4::SelectionMode::None);
                flow_box.set_max_children_per_line(4);
                flow_box.set_min_children_per_line(1);
                flow_box.set_row_spacing(6);
                flow_box.set_column_spacing(6);
                flow_box.set_homogeneous(false);

                let mut sorted_groups = known_groups;
                sorted_groups.sort();

                for group_name in &sorted_groups {
                    let btn = gtk4::Button::with_label(group_name);
                    btn.add_css_class("pill");
                    let entry_clone = entry.clone();
                    let name = group_name.clone();
                    btn.connect_clicked(move |_| {
                        entry_clone.set_text(&name);
                    });
                    flow_box.append(&btn);
                }
                content_box.append(&flow_box);

                let or_label = gtk4::Label::new(Some(&i18n("or enter a new name:")));
                or_label.set_halign(gtk4::Align::Start);
                or_label.add_css_class("dim-label");
                content_box.append(&or_label);
            }

            // Pre-fill with current group if any
            if let Some(info) = session_info.borrow().get(&session_id)
                && let Some(ref group) = info.tab_group
            {
                entry.set_text(group);
            }

            content_box.append(&entry);
            dialog.set_extra_child(Some(&content_box));
            dialog.add_response("cancel", &i18n("Cancel"));
            dialog.add_response("apply", &i18n("Apply"));
            dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("apply"));
            dialog.set_close_response("cancel");

            // Enter key triggers "apply" via set_default_response above

            let session_info_clone = session_info.clone();
            let tab_group_manager_clone = tab_group_manager.clone();
            let sessions_clone = sessions.clone();

            dialog.connect_response(None, move |_dialog, response| {
                if response != "apply" {
                    return;
                }
                let group_name = entry.text().trim().to_string();
                if group_name.is_empty() {
                    return;
                }

                let color_index = tab_group_manager_clone
                    .borrow_mut()
                    .get_or_assign_color(&group_name);

                if let Some(info) = session_info_clone.borrow_mut().get_mut(&session_id) {
                    info.tab_group = Some(group_name.clone());
                    info.tab_color_index = Some(color_index);
                }

                // Apply group label prefix to tab title (independent of split indicator)
                if let Some(page) = sessions_clone.borrow().get(&session_id) {
                    let current_title = page.title().to_string();
                    let base_title = current_title
                        .find("] ")
                        .and_then(|pos| {
                            if current_title.starts_with('[') {
                                Some(&current_title[pos + 2..])
                            } else {
                                None
                            }
                        })
                        .unwrap_or(&current_title);
                    page.set_title(&format!("[{group_name}] {base_title}"));
                }

                // Update tooltip to include group name
                if let Some(page) = sessions_clone.borrow().get(&session_id) {
                    let current_tooltip = page.tooltip().unwrap_or_default();
                    let base_tooltip = current_tooltip
                        .as_str()
                        .rsplit_once("\n[")
                        .map_or(current_tooltip.as_str(), |(base, _)| base);
                    page.set_tooltip(&format!("{base_tooltip}\n[{group_name}]"));
                }

                tracing::debug!(
                    session_id = %session_id,
                    group = group_name,
                    color_index,
                    "Tab assigned to group via context menu"
                );
            });

            // Present the dialog
            if let Some(root) = target_page.child().root()
                && let Some(window) = root.downcast_ref::<gtk4::Window>()
            {
                dialog.present(Some(window));
            }
        });
        action_group.add_action(&set_group_action);

        // "Remove from Group" action
        let remove_group_action = gio::SimpleAction::new("remove-group", None);
        let context_page_remove = context_page.clone();
        let session_info = self.session_info.clone();
        let sessions = self.sessions.clone();
        let color_tabs_by_protocol = self.color_tabs_by_protocol.clone();
        let split_session_colors = self.split_session_colors.clone();

        remove_group_action.connect_activate(move |_, _| {
            let target_page = context_page_remove.borrow().clone();
            let Some(target_page) = target_page else {
                return;
            };
            let session_id = {
                let sessions_ref = sessions.borrow();
                sessions_ref
                    .iter()
                    .find(|(_, p)| *p == &target_page)
                    .map(|(id, _)| *id)
            };
            let Some(session_id) = session_id else {
                return;
            };

            // Clear group from session info
            let protocol = {
                let mut info_ref = session_info.borrow_mut();
                if let Some(info) = info_ref.get_mut(&session_id) {
                    info.tab_group = None;
                    info.tab_color_index = None;
                    Some(info.protocol.clone())
                } else {
                    None
                }
            };

            // Restore appropriate indicator — group no longer uses indicator_icon,
            // so just restore protocol color if enabled
            let has_split_color = split_session_colors.borrow().contains_key(&session_id);

            if !has_split_color
                && *color_tabs_by_protocol.borrow()
                && let Some(ref proto) = protocol
                && let Some(page) = sessions.borrow().get(&session_id)
            {
                let (r, g, b) = rustconn_core::get_protocol_color_rgb(proto);
                if let Some(icon) = Self::create_protocol_color_icon(r, g, b, 16) {
                    page.set_indicator_icon(Some(&icon));
                    page.set_indicator_activatable(false);
                }
            }

            // Remove group label prefix from tab title
            if let Some(page) = sessions.borrow().get(&session_id) {
                let current_title = page.title().to_string();
                if let Some(pos) = current_title.find("] ")
                    && current_title.starts_with('[')
                {
                    page.set_title(&current_title[pos + 2..]);
                }
            }

            // Restore original tooltip (remove group suffix)
            if let Some(page) = sessions.borrow().get(&session_id) {
                let tooltip = page.tooltip().unwrap_or_default();
                let tooltip_str = tooltip.as_str();
                if let Some(base) = tooltip_str.rsplit_once("\n[") {
                    page.set_tooltip(base.0);
                }
            }

            tracing::debug!(session_id = %session_id, "Tab removed from group via context menu");
        });
        action_group.add_action(&remove_group_action);

        // "Close All in Group" action — closes all tabs belonging to the same group
        let close_all_group_action = gio::SimpleAction::new("close-all-in-group", None);
        let context_page_close_group = context_page.clone();
        let sessions_for_close_group = self.sessions.clone();
        let session_info_for_close_group = self.session_info.clone();
        let tab_view_for_close_group = self.tab_view.clone();

        close_all_group_action.connect_activate(move |_, _| {
            let target_page = context_page_close_group.borrow().clone();
            let Some(target_page) = target_page else {
                return;
            };
            // Find the group name of the right-clicked tab
            let group_name = {
                let sessions_ref = sessions_for_close_group.borrow();
                let session_id = sessions_ref
                    .iter()
                    .find(|(_, p)| *p == &target_page)
                    .map(|(id, _)| *id);
                session_id.and_then(|sid| {
                    session_info_for_close_group
                        .borrow()
                        .get(&sid)
                        .and_then(|i| i.tab_group.clone())
                })
            };
            let Some(group_name) = group_name else {
                return;
            };

            // Collect all session IDs in this group
            let sessions_to_close: Vec<Uuid> = {
                let info_ref = session_info_for_close_group.borrow();
                let sessions_ref = sessions_for_close_group.borrow();
                info_ref
                    .iter()
                    .filter(|(_, info)| info.tab_group.as_deref() == Some(group_name.as_str()))
                    .filter_map(|(sid, _)| sessions_ref.get(sid).map(|page| (*sid, page.clone())))
                    .map(|(sid, _)| sid)
                    .collect()
            };

            // Show confirmation dialog
            let count = sessions_to_close.len();
            if count == 0 {
                return;
            }

            let confirm = adw::AlertDialog::builder()
                .heading(i18n("Close All in Group"))
                .body(i18n_f(
                    "Close {} tabs in group '{}'?",
                    &[&count.to_string(), &group_name],
                ))
                .build();
            confirm.add_response("cancel", &i18n("Cancel"));
            confirm.add_response("close", &i18n("Close"));
            confirm.set_response_appearance("close", adw::ResponseAppearance::Destructive);
            confirm.set_default_response(Some("cancel"));
            confirm.set_close_response("cancel");

            let sessions_for_confirm = sessions_for_close_group.clone();
            let tab_view_for_confirm = tab_view_for_close_group.clone();
            confirm.connect_response(None, move |_dialog, response| {
                if response != "close" {
                    return;
                }
                // Collect pages first, then drop the borrow before calling close_page.
                // close_page triggers connect_close_page which also borrows sessions.
                let pages: Vec<adw::TabPage> = {
                    let sessions_ref = sessions_for_confirm.borrow();
                    sessions_to_close
                        .iter()
                        .filter_map(|sid| sessions_ref.get(sid).cloned())
                        .collect()
                };
                for page in &pages {
                    tab_view_for_confirm.close_page(page);
                }
                tracing::debug!(
                    group = group_name,
                    count,
                    "Closed all tabs in group via context menu"
                );
            });

            if let Some(root) = target_page.child().root()
                && let Some(window) = root.downcast_ref::<gtk4::Window>()
            {
                confirm.present(Some(window));
            }
        });
        action_group.add_action(&close_all_group_action);

        // "Close All Tabs" action
        let close_all_action = gio::SimpleAction::new("close-all", None);
        let tab_view_for_close_all = self.tab_view.clone();
        close_all_action.connect_activate(move |_, _| {
            let pages: Vec<_> = (0..tab_view_for_close_all.n_pages())
                .map(|i| tab_view_for_close_all.nth_page(i))
                .collect();
            for page in pages {
                tab_view_for_close_all.close_page(&page);
            }
        });
        action_group.add_action(&close_all_action);

        // "Close Others" action — close all except selected
        let close_others_action = gio::SimpleAction::new("close-others", None);
        let tab_view_for_close_others = self.tab_view.clone();
        close_others_action.connect_activate(move |_, _| {
            let selected = tab_view_for_close_others.selected_page();
            let pages: Vec<_> = (0..tab_view_for_close_others.n_pages())
                .map(|i| tab_view_for_close_others.nth_page(i))
                .filter(|p| selected.as_ref() != Some(p))
                .collect();
            for page in pages {
                tab_view_for_close_others.close_page(&page);
            }
        });
        action_group.add_action(&close_others_action);

        // "Close to the Left" action
        let close_left_action = gio::SimpleAction::new("close-left", None);
        let tab_view_for_close_left = self.tab_view.clone();
        close_left_action.connect_activate(move |_, _| {
            if let Some(selected) = tab_view_for_close_left.selected_page() {
                let pos = tab_view_for_close_left.page_position(&selected);
                let pages: Vec<_> = (0..pos)
                    .map(|i| tab_view_for_close_left.nth_page(i))
                    .collect();
                for page in pages {
                    tab_view_for_close_left.close_page(&page);
                }
            }
        });
        action_group.add_action(&close_left_action);

        // "Close to the Right" action
        let close_right_action = gio::SimpleAction::new("close-right", None);
        let tab_view_for_close_right = self.tab_view.clone();
        close_right_action.connect_activate(move |_, _| {
            if let Some(selected) = tab_view_for_close_right.selected_page() {
                let pos = tab_view_for_close_right.page_position(&selected);
                let pages: Vec<_> = ((pos + 1)..tab_view_for_close_right.n_pages())
                    .map(|i| tab_view_for_close_right.nth_page(i))
                    .collect();
                for page in pages {
                    tab_view_for_close_right.close_page(&page);
                }
            }
        });
        action_group.add_action(&close_right_action);

        // "Close All Ungrouped" action — close tabs without a tab group
        let close_ungrouped_action = gio::SimpleAction::new("close-ungrouped", None);
        let tab_view_for_close_ungrouped = self.tab_view.clone();
        let sessions_for_close_ungrouped = self.sessions.clone();
        let session_info_for_close_ungrouped = self.session_info.clone();
        close_ungrouped_action.connect_activate(move |_, _| {
            let info = session_info_for_close_ungrouped.borrow();
            let sessions_ref = sessions_for_close_ungrouped.borrow();
            let ungrouped_pages: Vec<_> = sessions_ref
                .iter()
                .filter(|(sid, _)| info.get(sid).and_then(|i| i.tab_group.as_ref()).is_none())
                .map(|(_, page)| page.clone())
                .collect();
            drop(info);
            drop(sessions_ref);
            for page in ungrouped_pages {
                tab_view_for_close_ungrouped.close_page(&page);
            }
        });
        action_group.add_action(&close_ungrouped_action);

        // "Pin Tab" action
        let pin_action = gio::SimpleAction::new("pin", None);
        let context_page_pin = context_page.clone();
        let tab_view_pin = self.tab_view.clone();
        pin_action.connect_activate(move |_, _| {
            if let Some(page) = context_page_pin.borrow().clone() {
                tab_view_pin.set_page_pinned(&page, true);
            }
        });
        action_group.add_action(&pin_action);

        // "Unpin Tab" action
        let unpin_action = gio::SimpleAction::new("unpin", None);
        let context_page_unpin = context_page.clone();
        let tab_view_unpin = self.tab_view.clone();
        unpin_action.connect_activate(move |_, _| {
            if let Some(page) = context_page_unpin.borrow().clone() {
                tab_view_unpin.set_page_pinned(&page, false);
            }
        });
        action_group.add_action(&unpin_action);

        // "Close Tab" action
        let close_action = gio::SimpleAction::new("close", None);
        let context_page_close = context_page.clone();
        let tab_view_clone = self.tab_view.clone();
        close_action.connect_activate(move |_, _| {
            if let Some(page) = context_page_close.borrow().clone() {
                tab_view_clone.close_page(&page);
            }
        });
        action_group.add_action(&close_action);

        // "Move to New Window" action — hands the session to the window layer,
        // which re-checks the verdict and explains a rejection with a toast.
        // "Remove Split" — returns every session in this tab's split layout to
        // its own tab, closing none of them (issue #252). The page is selected
        // first because `win.unsplit` acts on the active tab; without it a
        // right-click on a background split tab would dismantle the wrong
        // layout. Selecting is the expected side effect anyway — the user is
        // about to change that tab's contents.
        let unsplit_action = gio::SimpleAction::new("unsplit", None);
        let context_page_unsplit = context_page.clone();
        let tab_view_for_unsplit = self.tab_view.clone();
        unsplit_action.connect_activate(move |_, _| {
            let Some(page) = context_page_unsplit.borrow().clone() else {
                return;
            };
            tab_view_for_unsplit.set_selected_page(&page);
            if let Some(window) = tab_view_for_unsplit
                .root()
                .and_then(|root| root.downcast::<gtk4::ApplicationWindow>().ok())
            {
                gtk4::prelude::ActionGroupExt::activate_action(&window, "unsplit", None);
            } else {
                tracing::warn!("tab.unsplit: could not find ApplicationWindow");
            }
        });
        action_group.add_action(&unsplit_action);

        let detach_action = gio::SimpleAction::new("detach", None);
        let context_page_detach = context_page.clone();
        let sessions_for_detach = self.sessions.clone();
        let detach_hooks = self.detach_hooks();
        detach_action.connect_activate(move |_, _| {
            if let Some(session_id) =
                Self::context_menu_session_id(&context_page_detach, &sessions_for_detach)
            {
                let _ = detach_hooks
                    .notify_detach_request(session_id, super::DetachPresentation::default());
            }
        });
        action_group.add_action(&detach_action);

        // "Move to New Window on…" submenu entries — the target is the monitor
        // index in the default display's monitor list.
        let detach_monitor_action =
            gio::SimpleAction::new("detach-to-monitor", Some(glib::VariantTy::UINT32));
        let context_page_detach_monitor = context_page.clone();
        let sessions_for_detach_monitor = self.sessions.clone();
        let detach_hooks_monitor = self.detach_hooks();
        detach_monitor_action.connect_activate(move |_, param| {
            let Some(monitor) = param.and_then(glib::Variant::get::<u32>) else {
                tracing::warn!("tab.detach-to-monitor activated without a monitor index");
                return;
            };
            if let Some(session_id) = Self::context_menu_session_id(
                &context_page_detach_monitor,
                &sessions_for_detach_monitor,
            ) {
                let selected = gdk::Display::default()
                    .and_then(|display| display.monitors().item(monitor))
                    .and_downcast::<gdk::Monitor>();
                let preference = super::DetachMonitor::from_monitor(monitor, selected.as_ref());
                let _ = detach_hooks_monitor.notify_detach_request(
                    session_id,
                    super::DetachPresentation {
                        fullscreen: true,
                        monitor: Some(preference),
                    },
                );
            }
        });
        action_group.add_action(&detach_monitor_action);

        // "Cycle Monitor" action — one step per activation, through every mode in
        // `MonitorMode::next()` order: Off → Activity → Silence → Command → Off.
        // Command joined the cycle when the mode was added; this comment said the
        // three-mode cycle for a while after that, which matters because reaching
        // Command from Off takes three activations and a stale list makes that
        // look like the menu is not working.
        let cycle_monitor_action = gio::SimpleAction::new("cycle-monitor", None);
        let context_page_monitor = context_page;
        let sessions_for_monitor = self.sessions.clone();
        let activity_for_action = self.activity_coordinator.clone();
        cycle_monitor_action.connect_activate(move |_, _| {
            let target_page = context_page_monitor.borrow().clone();
            let Some(target_page) = target_page else {
                return;
            };
            let session_id = {
                let sessions_ref = sessions_for_monitor.borrow();
                sessions_ref
                    .iter()
                    .find(|(_, p)| *p == &target_page)
                    .map(|(id, _)| *id)
            };
            let Some(session_id) = session_id else {
                return;
            };

            let coordinator = activity_for_action.borrow();
            let Some(coordinator) = coordinator.as_ref() else {
                return;
            };
            let new_mode = coordinator.cycle_mode(session_id);
            tracing::debug!(
                session_id = %session_id,
                mode = ?new_mode,
                "Monitor mode cycled via context menu"
            );
        });
        action_group.add_action(&cycle_monitor_action);

        // Attach action group to the TabView widget and TabBar
        // The TabBar needs the action group because the context menu popover
        // is parented to the TabBar, and GTK looks up actions by walking
        // up the widget tree from the popover's parent.
        self.tab_view
            .insert_action_group("tab", Some(&action_group));
        self.tab_bar.insert_action_group("tab", Some(&action_group));
    }

    /// Resolves the right-clicked page to its session id.
    ///
    /// Mirrors the lookup `tab.set-group` performs, so every handler in this
    /// action group finds its session the same way.
    fn context_menu_session_id(
        context_page: &Rc<RefCell<Option<adw::TabPage>>>,
        sessions: &Rc<RefCell<HashMap<Uuid, adw::TabPage>>>,
    ) -> Option<Uuid> {
        let target_page = context_page.borrow().clone()?;
        let sessions_ref = sessions.borrow();
        sessions_ref
            .iter()
            .find(|(_, page)| *page == &target_page)
            .map(|(id, _)| *id)
    }

    /// Builds the "Move to New Window on…" submenu, or `None` for one monitor.
    ///
    /// With a single monitor there is no choice to present, so the submenu is
    /// omitted rather than shown with one entry (Requirement 8.3).
    fn monitor_detach_submenu() -> Option<gio::Menu> {
        let monitors = gdk::Display::default()?.monitors();
        let count = monitors.n_items();
        if count < 2 {
            return None;
        }

        let submenu = gio::Menu::new();
        for index in 0..count {
            let monitor = monitors.item(index).and_downcast::<gdk::Monitor>();
            let label = Self::monitor_label(index, monitor.as_ref());
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(
                Some("tab.detach-to-monitor"),
                Some(&index.to_variant()),
            );
            submenu.append_item(&item);
        }
        Some(submenu)
    }

    /// Names a monitor for the submenu, for example "Monitor 1 (DP-1)".
    ///
    /// The connector is the name the desktop's display settings show; the model
    /// is the fallback, and a monitor that reports neither is listed by number
    /// alone.
    fn monitor_label(index: u32, monitor: Option<&gdk::Monitor>) -> String {
        let number = (index + 1).to_string();
        let descriptor =
            monitor.and_then(|monitor| monitor.connector().or_else(|| monitor.model()));
        match descriptor {
            Some(descriptor) => i18n_f("Monitor {} ({})", &[&number, descriptor.as_str()]),
            None => i18n_f("Monitor {}", &[&number]),
        }
    }

    /// Populates the tab context menu model in-place.
    ///
    /// The caller must pass an existing `gio::Menu` that has already been set
    /// as the `TabView` menu model.  This avoids replacing the model object
    /// (which would invalidate the popover's reference and cause a SIGSEGV on
    /// rapid repeated right-clicks).
    pub(crate) fn populate_tab_context_menu(menu: &gio::Menu, state: TabMenuState) {
        // Pin/Unpin section
        let pin_section = gio::Menu::new();
        if state.is_pinned {
            pin_section.append(Some(&i18n("Unpin Tab")), Some("tab.unpin"));
        } else {
            pin_section.append(Some(&i18n("Pin Tab")), Some("tab.pin"));
        }
        menu.append_section(None, &pin_section);

        // Group section — adaptive: only show group actions when groups exist
        let group_section = gio::Menu::new();
        // HIG: the ellipsis marks an item that opens a dialog, and it is the
        // single character U+2026 rather than three periods.
        group_section.append(Some(&i18n("Set Group…")), Some("tab.set-group"));
        if state.has_group {
            group_section.append(Some(&i18n("Remove from Group")), Some("tab.remove-group"));
            group_section.append(
                Some(&i18n("Close All in Group")),
                Some("tab.close-all-in-group"),
            );
        }
        menu.append_section(None, &group_section);

        // Monitor section with current mode in label
        let monitor_section = gio::Menu::new();
        let mode = state.monitor_mode.unwrap_or(MonitorMode::Off);
        let label = i18n_f("Monitor: {}", &[&i18n(mode.display_name())]);
        monitor_section.append(Some(&label), Some("tab.cycle-monitor"));
        menu.append_section(None, &monitor_section);

        // Split section — only for the tab that hosts a split layout, and the
        // reason the detach item directly below it is currently refused
        // (issue #252).
        if state.hosts_split {
            let split_section = gio::Menu::new();
            split_section.append(Some(&i18n("Remove Split")), Some("tab.unsplit"));
            menu.append_section(None, &split_section);
        }
        // Detach section — sits directly above the close section, and is
        // omitted entirely for a session that cannot be moved to its own window.
        if state.can_detach {
            let detach_section = gio::Menu::new();
            detach_section.append(Some(&i18n("Move to New Window")), Some("tab.detach"));
            if let Some(monitors) = Self::monitor_detach_submenu() {
                // No ellipsis: this opens a submenu, not a dialog (HIG).
                detach_section.append_submenu(Some(&i18n("Move to New Window on")), &monitors);
            }
            menu.append_section(None, &detach_section);
        }

        // Close section — minimal by default, expanded when groups exist
        let close_section = gio::Menu::new();
        close_section.append(Some(&i18n("Close Tab")), Some("tab.close"));
        close_section.append(Some(&i18n("Close Others")), Some("tab.close-others"));
        close_section.append(Some(&i18n("Close to the Left")), Some("tab.close-left"));
        close_section.append(Some(&i18n("Close to the Right")), Some("tab.close-right"));
        if state.any_groups_exist {
            close_section.append(
                Some(&i18n("Close All Ungrouped")),
                Some("tab.close-ungrouped"),
            );
        }
        close_section.append(Some(&i18n("Close All Tabs")), Some("tab.close-all"));
        menu.append_section(None, &close_section);
    }
}

#[cfg(test)]
mod tests {
    use rustconn_core::DetachVerdict;

    use super::offers_detach;

    #[test]
    fn a_detachable_session_gets_the_menu_item() {
        assert!(offers_detach(DetachVerdict::Allowed));
    }

    #[test]
    fn a_split_owner_keeps_the_item_so_the_restriction_can_be_explained() {
        assert!(offers_detach(DetachVerdict::SplitOwner));
    }

    #[test]
    fn every_other_blocking_verdict_hides_the_section() {
        for verdict in [
            DetachVerdict::AlreadyDetached,
            DetachVerdict::ExternalViewer,
            DetachVerdict::SplitGuest,
        ] {
            assert!(
                !offers_detach(verdict),
                "{} must not show an inert menu item",
                verdict.reason_key()
            );
        }
    }
}
