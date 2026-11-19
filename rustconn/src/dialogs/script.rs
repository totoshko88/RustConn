//! Scripts manager dialog: list, edit, validate, delete and run `.rsc`
//! scripts from the script library.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Entry, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow,
    SearchEntry, TextView,
};
use libadwaita as adw;
use rustconn_core::script::ScriptManager;

use super::script_examples::BUNDLED_EXAMPLES;

use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::window::types::SharedNotebook;
use uuid::Uuid;

/// Writes the bundled examples into the library, refreshing them when the
/// shipped source differs.
///
/// Bundled ids are managed content: an edited copy under an `example-*` id is
/// replaced on upgrade, so adaptation belongs in a copy saved under a new id.
/// An identical file is not rewritten, and a differing file is copied to
/// `<id>.rsc.bak` first. An existing backup is never overwritten: the next
/// copy goes to `<id>.rsc.bak.1`, `.bak.2`, and so on, so a user's edit
/// survives any number of upgrades.
fn seed_bundled_examples(manager: &ScriptManager) {
    seed_examples(manager, BUNDLED_EXAMPLES);
}

fn seed_examples(manager: &ScriptManager, examples: &[(&str, &str)]) {
    for (id, source) in examples {
        let current = match manager.load_script(id) {
            Ok(text) => Some(text),
            // No file yet: seed it. Any other failure means a file exists
            // that cannot be read; replacing it would lose it unbacked.
            Err(_) if !manager.scripts_dir().join(format!("{id}.rsc")).exists() => None,
            Err(error) => {
                tracing::warn!(%error, id, "could not read a bundled example; leaving it unchanged");
                continue;
            }
        };
        if current.as_deref() == Some(*source) {
            continue;
        }
        if let Some(current) = current {
            let path = manager.scripts_dir().join(format!("{id}.rsc"));
            if let Err(error) = back_up(&path, &current) {
                tracing::warn!(
                    %error,
                    path = %path.display(),
                    "could not back up an edited bundled example; leaving it unchanged"
                );
                continue;
            }
        }
        if let Err(error) = manager.save_script(id, source) {
            tracing::warn!(%error, id, "could not write a bundled example");
        }
    }
}

/// Copies `path` to the first free `<path>.bak[.N]`; a backup that already
/// holds the same text is reused.
fn back_up(path: &std::path::Path, current: &str) -> std::io::Result<()> {
    for n in 0u32.. {
        let suffix = if n == 0 {
            "bak".to_owned()
        } else {
            format!("bak.{n}")
        };
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{suffix}"));
        let backup = std::path::PathBuf::from(name);
        match std::fs::read_to_string(&backup) {
            Ok(existing) if existing == current => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return std::fs::write(&backup, current);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the loop ends by returning")
}

/// Scripts manager dialog.
pub struct ScriptDialog {
    dialog: adw::Dialog,
}

impl ScriptDialog {
    /// Creates the dialog for the given app state and notebook.
    #[must_use]
    pub fn new(
        parent: Option<&impl IsA<gtk4::Widget>>,
        state: SharedAppState,
        notebook: SharedNotebook,
    ) -> Self {
        let manager = Rc::new(ScriptManager::new(state.borrow().config_manager().clone()));

        let dialog = adw::Dialog::builder()
            .title(i18n("Scripts"))
            .content_width(900)
            .content_height(640)
            .build();

        // The dialog is clamped to its parent by libadwaita, so 900x640 never
        // overflows a small window; the panes below scroll instead.
        let header = adw::HeaderBar::new();

        let help_btn = Button::from_icon_name("help-about-symbolic");
        help_btn.add_css_class("flat");
        help_btn.set_tooltip_text(Some(&i18n("Script language reference")));
        help_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Script language reference",
        ))]);
        header.pack_end(&help_btn);

        // Two panes that collapse on a narrow window: the list is the sidebar,
        // the editor (or the empty state) is the content.
        let split = adw::OverlaySplitView::builder()
            .min_sidebar_width(220.0)
            .max_sidebar_width(300.0)
            .sidebar_width_fraction(0.32)
            .vexpand(true)
            .build();

        // Shown only once the split has collapsed; otherwise the list is
        // already on screen and the button would do nothing.
        let sidebar_toggle = gtk4::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text(i18n("Scripts"))
            .build();
        sidebar_toggle.add_css_class("flat");
        sidebar_toggle.update_property(&[gtk4::accessible::Property::Label(&i18n("Scripts"))]);
        split
            .bind_property("show-sidebar", &sidebar_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        split
            .bind_property("collapsed", &sidebar_toggle, "visible")
            .sync_create()
            .build();
        header.pack_start(&sidebar_toggle);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&split));
        dialog.set_child(Some(&toolbar));

        // Sidebar: an explicit New Script button, the search field and the list.
        let new_btn = Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name("document-new-symbolic")
                    .label(i18n("New Script"))
                    .build(),
            )
            .build();
        new_btn.set_tooltip_text(Some(&i18n("New Script")));
        new_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("New Script"))]);

        let script_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .valign(gtk4::Align::Start)
            .build();
        script_list.update_property(&[gtk4::accessible::Property::Label(&i18n("Scripts"))]);
        let placeholder = adw::StatusPage::builder()
            .icon_name("utilities-terminal-symbolic")
            .title(i18n("No Scripts"))
            .description(i18n("Scripts are stored in the script library folder."))
            .build();
        script_list.set_placeholder(Some(&placeholder));
        let list_scroll = ScrolledWindow::builder()
            .child(&script_list)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vexpand(true)
            .build();

        // Filters the library by script id as the user types. Matching is
        // case-insensitive and applies to the visible label only. A filter
        // function is used instead of hiding rows, so the list's placeholder
        // logic keeps working when nothing matches.
        let search = SearchEntry::new();
        search.set_placeholder_text(Some(&i18n("Search")));
        search.update_property(&[gtk4::accessible::Property::Label(&i18n("Search"))]);
        {
            let query = Rc::new(RefCell::new(String::new()));
            let filter_query = query.clone();
            script_list.set_filter_func(move |row: &ListBoxRow| {
                let name = row
                    .child()
                    .and_then(|child| child.downcast::<Label>().ok())
                    .map(|label| label.text().to_lowercase())
                    .unwrap_or_default();
                name.contains(filter_query.borrow().as_str())
            });

            let list = script_list.clone();
            search.connect_search_changed(move |entry| {
                *query.borrow_mut() = entry.text().to_lowercase();
                list.invalidate_filter();
            });
        }
        let list_group = GtkBox::new(Orientation::Vertical, 12);
        list_group.set_margin_top(12);
        list_group.set_margin_bottom(12);
        list_group.set_margin_start(12);
        list_group.set_margin_end(12);
        list_group.append(&new_btn);
        list_group.append(&search);
        list_group.append(&list_scroll);
        split.set_sidebar(Some(&list_group));

        // Content: an empty state until a script is picked or created, then
        // the id field, the framed editor, the status line and the actions.
        let id_entry = Entry::new();
        id_entry.set_placeholder_text(Some(&i18n("Script id (file name)")));
        id_entry.update_property(&[gtk4::accessible::Property::Label(&i18n("Script ID"))]);

        let editor = TextView::builder()
            .hexpand(true)
            .vexpand(true)
            .monospace(true)
            .left_margin(8)
            .right_margin(8)
            .top_margin(8)
            .bottom_margin(8)
            .build();
        editor.update_property(&[gtk4::accessible::Property::Label(&i18n("Script"))]);
        let editor_scroll = ScrolledWindow::builder()
            .child(&editor)
            .has_frame(true)
            .hexpand(true)
            .vexpand(true)
            .min_content_height(160)
            .build();

        let status_label = Label::new(Some(&i18n("Select or create a script.")));
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_hexpand(true);
        status_label.set_wrap(true);
        status_label.set_xalign(0.0);
        status_label.add_css_class("dim-label");

        let run_btn = Button::builder()
            .label(i18n("Run"))
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        let save_btn = Button::builder()
            .label(i18n("Save"))
            .sensitive(false)
            .build();
        let stop_btn = Button::builder()
            .label(i18n("Stop"))
            .sensitive(false)
            .build();
        let delete_btn = Button::builder()
            .label(i18n("Delete"))
            .css_classes(["destructive-action"])
            .sensitive(false)
            .build();

        // Delete sits apart on the left; the rest follow the usual order,
        // with the suggested action last.
        let button_box = GtkBox::new(Orientation::Horizontal, 6);
        button_box.append(&delete_btn);
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        button_box.append(&spacer);
        button_box.append(&stop_btn);
        button_box.append(&save_btn);
        button_box.append(&run_btn);

        let editor_group = GtkBox::new(Orientation::Vertical, 12);
        editor_group.set_margin_top(12);
        editor_group.set_margin_bottom(12);
        editor_group.set_margin_start(18);
        editor_group.set_margin_end(18);
        editor_group.append(&id_entry);
        editor_group.append(&editor_scroll);
        editor_group.append(&status_label);
        editor_group.append(&button_box);

        let empty_page = adw::StatusPage::builder()
            .icon_name("utilities-terminal-symbolic")
            .title(i18n("Select or create a script."))
            .build();
        let pages = gtk4::Stack::new();
        pages.add_named(&empty_page, Some("empty"));
        pages.add_named(&editor_group, Some("editor"));
        pages.set_visible_child_name("empty");
        split.set_content(Some(&pages));

        // On a collapsed (narrow) window, picking or creating a script moves
        // on to the editor.
        {
            let split_row = split.clone();
            script_list.connect_row_activated(move |_, _| {
                if split_row.is_collapsed() {
                    split_row.set_show_sidebar(false);
                }
            });
            let split_new = split.clone();
            new_btn.connect_clicked(move |_| {
                if split_new.is_collapsed() {
                    split_new.set_show_sidebar(false);
                }
            });
        }

        let parent_widget: Option<gtk4::Widget> = parent.map(|p| p.clone().upcast());

        // The reference opens in its own window, parented on the main window.
        {
            let help = help_btn.clone();
            let parent = parent_widget.clone();
            help.connect_clicked(move |_| {
                super::script_reference::show_script_reference(parent.as_ref());
            });
        }

        let ids = Rc::new(RefCell::new(Vec::new()));
        seed_bundled_examples(&manager);
        Self::refresh_list(&script_list, &manager, &ids);
        Self::connect_signals(
            &dialog,
            &script_list,
            &ids,
            &id_entry,
            &editor,
            &status_label,
            &save_btn,
            &delete_btn,
            &run_btn,
            &stop_btn,
            &new_btn,
            &pages,
            manager,
            state,
            notebook,
            parent_widget,
        );

        Self { dialog }
    }

    /// Presents the dialog.
    pub fn present(&self, parent: Option<&impl IsA<gtk4::Widget>>) {
        self.dialog.present(parent);
    }

    fn refresh_list(list: &ListBox, manager: &Rc<ScriptManager>, ids: &Rc<RefCell<Vec<String>>>) {
        while let Some(row) = list.row_at_index(0) {
            list.remove(&row);
        }
        let mut ids_store = ids.borrow_mut();
        ids_store.clear();

        for id in manager.list_scripts() {
            let row = ListBoxRow::new();
            let label = Label::new(Some(&id));
            label.set_halign(gtk4::Align::Start);
            label.set_xalign(0.0);
            label.set_margin_top(10);
            label.set_margin_bottom(10);
            label.set_margin_start(12);
            label.set_margin_end(12);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            label.set_tooltip_text(Some(&id));
            row.set_child(Some(&label));
            list.append(&row);
            ids_store.push(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn connect_signals(
        dialog: &adw::Dialog,
        script_list: &ListBox,
        ids: &Rc<RefCell<Vec<String>>>,
        id_entry: &Entry,
        editor: &TextView,
        status_label: &Label,
        save_btn: &Button,
        delete_btn: &Button,
        run_btn: &Button,
        stop_btn: &Button,
        new_btn: &Button,
        pages: &gtk4::Stack,
        manager: Rc<ScriptManager>,
        state: SharedAppState,
        notebook: SharedNotebook,
        parent: Option<gtk4::Widget>,
    ) {
        let manager_list = manager.clone();
        let ids_list = ids.clone();
        let id_entry_list = id_entry.clone();
        let editor_list = editor.clone();
        let status_list = status_label.clone();
        let save_list = save_btn.clone();
        let delete_list = delete_btn.clone();
        let run_list = run_btn.clone();
        let pages_list = pages.clone();

        script_list.connect_row_selected(move |_, row| {
            let Some(row) = row else {
                save_list.set_sensitive(false);
                delete_list.set_sensitive(false);
                run_list.set_sensitive(false);
                return;
            };
            let idx = row.index();
            if idx < 0 {
                return;
            }
            let id = ids_list
                .borrow()
                .get(idx as usize)
                .cloned()
                .unwrap_or_default();
            if id.is_empty() {
                return;
            }
            match manager_list.load_script(&id) {
                Ok(source) => {
                    id_entry_list.set_text(&id);
                    editor_list.buffer().set_text(&source);
                    save_list.set_sensitive(true);
                    delete_list.set_sensitive(true);
                    run_list.set_sensitive(true);
                    pages_list.set_visible_child_name("editor");
                    status_list
                        .set_label(&i18n("Editing script. Save writes it back to the library."));
                }
                Err(e) => {
                    pages_list.set_visible_child_name("editor");
                    status_list.set_label(&format!("{}: {e}", i18n("Could not load script")));
                }
            }
        });

        let manager_new = manager.clone();
        let id_entry_new = id_entry.clone();
        let editor_new = editor.clone();
        let status_new = status_label.clone();
        let save_new = save_btn.clone();
        let delete_new = delete_btn.clone();
        let run_new = run_btn.clone();
        let pages_new = pages.clone();
        let list_new = script_list.clone();
        new_btn.connect_clicked(move |_| {
            list_new.unselect_all();
            pages_new.set_visible_child_name("editor");
            id_entry_new.grab_focus();
            id_entry_new.set_text("");
            editor_new.buffer().set_text("");
            status_new.set_label(&i18n("New script. Enter an id and source, then Save."));
            save_new.set_sensitive(true);
            delete_new.set_sensitive(false);
            run_new.set_sensitive(false);
            let _ = &manager_new;
        });

        let manager_save = manager.clone();
        let script_list_save = script_list.clone();
        let ids_save = ids.clone();
        let id_entry_save = id_entry.clone();
        let editor_save = editor.clone();
        let status_save = status_label.clone();
        save_btn.connect_clicked(move |_| {
            let id = id_entry_save.text().trim().to_string();
            if id.is_empty() {
                status_save.set_label(&i18n("A script id is required."));
                return;
            }
            let source = editor_save.buffer().text(
                &editor_save.buffer().start_iter(),
                &editor_save.buffer().end_iter(),
                false,
            );
            match manager_save.save_script(&id, &source) {
                Ok(()) => {
                    status_save.set_label(&i18n("Script saved."));
                    Self::refresh_list(&script_list_save, &manager_save, &ids_save);
                }
                Err(e) => {
                    status_save.set_label(&format!("{}: {e}", i18n("Could not save script")));
                }
            }
        });

        let manager_delete = manager.clone();
        let script_list_delete = script_list.clone();
        let ids_delete = ids.clone();
        let id_entry_delete = id_entry.clone();
        let editor_delete = editor.clone();
        let status_delete = status_label.clone();
        let save_delete = save_btn.clone();
        let run_delete = run_btn.clone();
        let pages_delete = pages.clone();
        delete_btn.connect_clicked(move |btn| {
            let id = id_entry_delete.text().trim().to_string();
            if id.is_empty() {
                status_delete.set_label(&i18n("Select a script to delete."));
                return;
            }
            let manager_delete = manager_delete.clone();
            let script_list_delete = script_list_delete.clone();
            let ids_delete = ids_delete.clone();
            let id_entry_delete = id_entry_delete.clone();
            let editor_delete = editor_delete.clone();
            let status_delete = status_delete.clone();
            let save_delete = save_delete.clone();
            let run_delete = run_delete.clone();
            let pages_delete = pages_delete.clone();
            confirm_delete_script(btn, &id, move || {
                id_entry_delete.set_text("");
                editor_delete.buffer().set_text("");
                status_delete.set_label(&i18n("Script deleted."));
                save_delete.set_sensitive(false);
                run_delete.set_sensitive(false);
                pages_delete.set_visible_child_name("empty");
                Self::refresh_list(&script_list_delete, &manager_delete, &ids_delete);
            });
        });

        // Shared run state: the Stop button and the elapsed-time ticker both
        // need to know whether a script is in flight and when it started.
        let running = Rc::new(std::cell::Cell::new(false));
        let run_started = Rc::new(std::cell::RefCell::new(None::<std::time::Instant>));
        let run_session = Rc::new(std::cell::RefCell::new(None::<Uuid>));
        let ticker = Rc::new(std::cell::RefCell::new(None::<glib::SourceId>));

        let notebook_run = notebook.clone();
        let manager_run = manager.clone();
        let id_entry_run = id_entry.clone();
        let status_run = status_label.clone();
        let stop_run = stop_btn.clone();
        let run_btn_run = run_btn.clone();
        let running_run = running.clone();
        let started_run = run_started.clone();
        let session_run = run_session.clone();
        let ticker_run = ticker.clone();
        run_btn.connect_clicked(move |_| {
            let id = id_entry_run.text().trim().to_string();
            if id.is_empty() {
                status_run.set_label(&i18n("Select a script to run."));
                return;
            }
            let Some(session_id) = notebook_run.get_active_session_id() else {
                status_run.set_label(&i18n("Open a session first, then run the script."));
                return;
            };
            match manager_run.load_script(&id) {
                Ok(source) => {
                    let Some(runtime) = crate::script_runner::script_runtime() else {
                        status_run.set_label(&i18n("Script runtime is not ready."));
                        return;
                    };
                    let connection_id = notebook_run
                        .get_all_sessions()
                        .into_iter()
                        .find(|s| s.id == session_id)
                        .map(|s| s.connection_id)
                        .unwrap_or_default();

                    // The runner reports its outcome back here, so the dialog
                    // can show the end state and how long the run took.
                    let status_finish = status_run.clone();
                    let running_finish = running_run.clone();
                    let started_finish = started_run.clone();
                    let ticker_stop = ticker_run.clone();
                    let run_btn_finish = run_btn_run.clone();
                    let stop_btn_finish = stop_run.clone();
                    let on_finish: Rc<dyn Fn(crate::script_runner::ScriptOutcome)> =
                        Rc::new(move |outcome| {
                            if let Some(source) = ticker_stop.borrow_mut().take() {
                                source.remove();
                            }
                            let elapsed = started_finish
                                .borrow()
                                .map(|started: std::time::Instant| started.elapsed());
                            let seconds = elapsed.map_or(0.0, |d| d.as_secs_f64());
                            let label = match outcome {
                                crate::script_runner::ScriptOutcome::Finished => {
                                    i18n_f("Script finished in {} s.", &[&format!("{seconds:.1}")])
                                }
                                crate::script_runner::ScriptOutcome::Stopped => i18n_f(
                                    "Script stopped after {} s.",
                                    &[&format!("{seconds:.1}")],
                                ),
                                crate::script_runner::ScriptOutcome::Failed(reason) => i18n_f(
                                    "Script failed after {} s: {}",
                                    &[&format!("{seconds:.1}"), &reason],
                                ),
                            };
                            status_finish.set_label(&label);
                            running_finish.set(false);
                            run_btn_finish.set_sensitive(true);
                            stop_btn_finish.set_sensitive(false);
                        });

                    // The running state is set before the runner starts: a
                    // script that ends on its first step calls `on_finish`
                    // before `start_for_session` returns, and that callback
                    // must be the last word, not be overwritten afterwards.
                    *started_run.borrow_mut() = Some(std::time::Instant::now());
                    *session_run.borrow_mut() = Some(session_id);
                    running_run.set(true);
                    run_btn_run.set_sensitive(false);
                    stop_run.set_sensitive(true);
                    status_run.set_label(&i18n("Script running… 0.0 s"));
                    // Elapsed time, refreshed five times a second. The
                    // script itself needs no wall clock: the runner is
                    // event-driven, so this label is the only place a
                    // duration is shown.
                    let status_tick = status_run.clone();
                    let started_tick = started_run.clone();
                    *ticker_run.borrow_mut() = Some(glib::timeout_add_local(
                        std::time::Duration::from_millis(200),
                        move || {
                            let Some(started) = *started_tick.borrow() else {
                                return glib::ControlFlow::Break;
                            };
                            let seconds = started.elapsed().as_secs_f64();
                            status_tick.set_label(&i18n_f(
                                "Script running… {} s",
                                &[&format!("{seconds:.1}")],
                            ));
                            glib::ControlFlow::Continue
                        },
                    ));
                    if let Err(e) = crate::script_runner::ScriptRunner::start_for_session(
                        &source,
                        session_id,
                        connection_id,
                        state.clone(),
                        notebook_run.clone(),
                        runtime.sidebar.clone(),
                        runtime.monitoring.clone(),
                        runtime,
                        Some(on_finish),
                    ) {
                        if let Some(source) = ticker_run.borrow_mut().take() {
                            source.remove();
                        }
                        *started_run.borrow_mut() = None;
                        *session_run.borrow_mut() = None;
                        running_run.set(false);
                        run_btn_run.set_sensitive(true);
                        stop_run.set_sensitive(false);
                        status_run.set_label(&format!("{}: {e}", i18n("Could not start script")));
                    }
                }
                Err(e) => {
                    status_run.set_label(&format!("{}: {e}", i18n("Could not load script")));
                }
            }
            let _ = parent;
        });

        // Stop acts on whatever script is running on the active session,
        // whether this dialog started it or the connection's Automation tab
        // did. Sensitivity follows the runner's own state, so an auto-run
        // script stuck waiting for a prompt can be stopped from here too.
        let status_stop = status_label.clone();
        let notebook_stop = notebook.clone();
        let ticker_run_stop = ticker.clone();
        let started_stop = run_started.clone();
        let running_stop = running.clone();
        let run_btn_stop = run_btn.clone();
        stop_btn.connect_clicked(move |_| {
            let Some(session_id) = notebook_stop.get_active_session_id() else {
                status_stop.set_label(&i18n("Open a session first, then stop the script."));
                return;
            };
            // Read the state before stopping: the finish callback of a
            // dialog-started script runs inside the stop and clears it.
            let started_here = running_stop.get();
            let _ = (&ticker_run_stop, &started_stop, &run_btn_stop);
            if !crate::script_runner::stop_script_for_session(session_id) {
                status_stop.set_label(&i18n("No script is running on this session."));
                return;
            }
            if started_here {
                // The finish callback already reported the stop and the
                // elapsed time, and reset the Stop/Run buttons.
                return;
            }
            // A script started by automation has no run state in this dialog.
            status_stop.set_label(&i18n("Script stopped."));
        });

        // Keep Stop enabled exactly while the active session runs a script.
        let stop_sync = stop_btn.clone();
        let notebook_sync = notebook.clone();
        let sync_source =
            glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
                let running = notebook_sync
                    .get_active_session_id()
                    .is_some_and(crate::script_runner::is_script_running);
                stop_sync.set_sensitive(running);
                glib::ControlFlow::Continue
            });
        let sync_source = std::rc::Rc::new(std::cell::RefCell::new(Some(sync_source)));
        let sync_source_close = std::rc::Rc::clone(&sync_source);
        dialog.connect_closed(move |_| {
            if let Some(source) = sync_source_close.borrow_mut().take() {
                source.remove();
            }
        });
    }
}

/// Shows the script manager dialog.
pub fn show_scripts_manager(
    parent: &impl IsA<gtk4::Widget>,
    state: SharedAppState,
    notebook: SharedNotebook,
) {
    let dialog = ScriptDialog::new(Some(parent), state, notebook);
    dialog.present(Some(parent));
}

/// Returns a `ScriptManager` on the default config directory.
///
/// Falls back to `~/.config/rustconn` when the home directory cannot be
/// determined, so the auto-run pickers always have a library to list.
#[must_use]
pub fn library_script_manager() -> ScriptManager {
    let manager = if let Ok(manager) = rustconn_core::config::ConfigManager::new() {
        manager
    } else {
        let fallback = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
            .join("rustconn");
        rustconn_core::config::ConfigManager::with_config_dir(fallback)
    };
    ScriptManager::new(manager)
}

/// Writes the bundled example scripts into the library, refreshing each one
/// whose shipped source changed.
///
/// Called once at startup and again when the Scripts manager opens, so the
/// examples exist in the auto-run dropdowns even if the manager has never been
/// opened. An identical file is not rewritten; a differing file is copied to
/// `<id>.rsc.bak` before it is replaced.
pub fn ensure_bundled_examples() {
    seed_bundled_examples(&library_script_manager());
}

/// Returns the ids of all scripts currently in the library, sorted.
///
/// Used to populate the auto-run script pickers in the connection and group
/// Automation tabs. Falls back to the default config directory, which is where
/// the library lives in the normal application flow.
#[must_use]
pub fn library_script_ids() -> Vec<String> {
    library_script_manager().list_scripts()
}

/// The choices an auto-run script dropdown offers, and which one is selected.
///
/// `ids[i]` is the script id behind `labels[i]`. An `existing` id that is not
/// in the local library — the connection was imported or team-synced, or the
/// script was deleted here — is kept as an extra, marked choice and selected.
/// Without it the dropdown fell back to the first library script, and the next
/// Save silently replaced the user's script with that one.
#[must_use]
pub fn auto_script_choices(existing: Option<&str>) -> (Vec<String>, Vec<String>, u32) {
    let mut ids = library_script_ids();
    let mut labels = ids.clone();
    let selected = match existing.filter(|e| !e.is_empty()) {
        Some(id) => {
            if let Some(pos) = ids.iter().position(|i| i == id) {
                pos
            } else {
                ids.push(id.to_string());
                labels.push(i18n_f("{} (not in this library)", &[id]));
                ids.len() - 1
            }
        }
        None => 0,
    };
    (ids, labels, u32::try_from(selected).unwrap_or(0))
}

/// Asks before deleting script `id` from the library, then deletes it.
///
/// Deleting removes the file for every connection and group that runs it, so
/// it is never done on a single click. `on_deleted` runs only after the file
/// is gone; a failed delete is reported in a notification and leaves the
/// caller's state untouched.
pub fn confirm_delete_script(
    parent: &impl IsA<gtk4::Widget>,
    id: &str,
    on_deleted: impl FnOnce() + 'static,
) {
    let dialog = adw::AlertDialog::new(
        Some(&i18n("Delete Script?")),
        Some(&i18n_f(
            "“{}” will be removed from the script library, and every connection and group that runs it will stop running it.",
            &[id],
        )),
    );
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("delete", &i18n("Delete"));
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let id = id.to_string();
    let on_deleted = RefCell::new(Some(on_deleted));
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        match library_script_manager().delete_script(&id) {
            Ok(()) => {
                crate::toast::show_info_toast_on_active_window(&i18n_f(
                    "Deleted script “{}”",
                    &[&id],
                ));
                if let Some(on_deleted) = on_deleted.borrow_mut().take() {
                    on_deleted();
                }
            }
            Err(e) => {
                tracing::warn!(script = %id, error = %e, "could not delete script");
                crate::toast::show_error_toast_on_active_window(&format!(
                    "{}: {e}",
                    i18n("Could not delete script")
                ));
            }
        }
    });
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::BUNDLED_EXAMPLES;

    /// Every bundled example must parse; a broken example is shipped code.
    #[test]
    fn bundled_examples_parse() {
        for (id, source) in BUNDLED_EXAMPLES {
            rustconn_core::automation::parse_program(source)
                .unwrap_or_else(|e| panic!("bundled example `{id}` does not parse: {e}"));
        }
    }

    /// Upgrading rewrites a bundled example whose source changed, and leaves
    /// scripts outside the bundled ids alone.
    #[test]
    fn seeding_refreshes_a_changed_example() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config =
            rustconn_core::config::ConfigManager::with_config_dir(temp.path().join("config"));
        let manager = rustconn_core::script::ScriptManager::new(config);
        manager
            .save_script("example-ping", "print \"old version\"\n")
            .unwrap();
        manager.save_script("mine", "print \"mine\"\n").unwrap();

        super::seed_bundled_examples(&manager);

        assert_eq!(
            manager.load_script("example-ping").unwrap(),
            BUNDLED_EXAMPLES
                .iter()
                .find(|(id, _)| *id == "example-ping")
                .map(|(_, source)| *source)
                .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(manager.scripts_dir().join("example-ping.rsc.bak"))
                .expect("the edited copy is backed up"),
            "print \"old version\"\n"
        );
        assert_eq!(manager.load_script("mine").unwrap(), "print \"mine\"\n");
    }

    /// An edit survives two upgrades: the second one must not overwrite the
    /// backup of the first.
    #[test]
    fn seeding_keeps_the_original_edit_across_two_upgrades() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config =
            rustconn_core::config::ConfigManager::with_config_dir(temp.path().join("config"));
        let manager = rustconn_core::script::ScriptManager::new(config);
        manager
            .save_script("example-x", "print \"my edit\"\n")
            .unwrap();

        super::seed_examples(&manager, &[("example-x", "print \"A\"\n")]);
        super::seed_examples(&manager, &[("example-x", "print \"B\"\n")]);
        super::seed_examples(&manager, &[("example-x", "print \"B\"\n")]);

        let dir = manager.scripts_dir();
        assert_eq!(manager.load_script("example-x").unwrap(), "print \"B\"\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("example-x.rsc.bak")).unwrap(),
            "print \"my edit\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("example-x.rsc.bak.1")).unwrap(),
            "print \"A\"\n"
        );
    }

    use rustconn_core::automation::{KeySequence, PendingKind, ScriptEngine, ScriptHost};

    /// Records what a script asks the host to do, and answers nothing.
    #[derive(Default)]
    struct Recorder {
        events: Vec<String>,
        prompt: Option<String>,
    }

    impl ScriptHost for Recorder {
        fn send(&mut self, sequence: &KeySequence) {
            self.events.push(format!("send:{sequence}"));
        }
        fn print_message(&mut self, text: &str) {
            self.events.push(format!("print:{text}"));
        }
        fn set_log(&mut self, path: Option<&str>) -> bool {
            self.events.push(format!("log:{}", path.unwrap_or("")));
            true
        }
        fn get_prompt(&mut self) -> Option<String> {
            self.prompt.clone()
        }
        fn ask(&mut self, prompt: &str, echo: bool, default: Option<&str>) {
            self.events
                .push(format!("ask:{prompt}:{echo}:{}", default.unwrap_or("")));
        }
        fn message(&mut self, text: &str, ok_button: bool) {
            self.events.push(format!("msg:{text}:{ok_button}"));
        }
        fn start_session(&mut self, selector: &str, manual_login: bool, cluster: Option<&str>) {
            self.events.push(format!(
                "start:{selector}:{manual_login}:{}",
                cluster.unwrap_or("")
            ));
        }
        fn select_session(&mut self, selector: &str) {
            self.events.push(format!("select:{selector}"));
        }
        fn stop_session(&mut self, session: &str) {
            self.events.push(format!("stop:{session}"));
        }
        fn split(&mut self, first: &str, second: &str, vertical: bool) {
            self.events
                .push(format!("split:{first}:{second}:{vertical}"));
        }
        fn unsplit(&mut self, session: &str) {
            self.events.push(format!("unsplit:{session}"));
        }
        fn to_window(&mut self, session: &str) {
            self.events.push(format!("towindow:{session}"));
        }
        fn to_tab(&mut self, session: &str) {
            self.events.push(format!("totab:{session}"));
        }
        fn run_command(&mut self, command: &str, timeout_ms: Option<u32>) {
            self.events.push(format!("run:{command}:{timeout_ms:?}"));
        }
    }

    /// Returns a bundled example by id.
    fn example(id: &str) -> &'static str {
        BUNDLED_EXAMPLES
            .iter()
            .find(|(example_id, _)| *example_id == id)
            .map(|(_, source)| *source)
            .unwrap_or_else(|| panic!("no bundled example `{id}`"))
    }

    /// Parses a bundled example into an engine ready to pump.
    fn engine_for(id: &str) -> ScriptEngine {
        ScriptEngine::parse(example(id)).unwrap_or_else(|e| panic!("`{id}`: {e}"))
    }

    /// Feeds terminal output and lets the engine consume it.
    fn feed(engine: &mut ScriptEngine, host: &mut Recorder, output: &str) {
        assert!(
            engine.on_output(output.as_bytes()),
            "output was not taken (pending {:?}): {output:?}",
            engine.pending()
        );
        engine.pump(host);
    }

    /// The file-name part of a log path, which is now absolute through `${CWD}`.
    fn log_name(path: &str) -> &str {
        path.rsplit(['/', '\\']).next().unwrap_or(path)
    }

    /// Drives the first-run example up to its first question.
    fn first_run_at_question() -> (ScriptEngine, Recorder) {
        let mut host = Recorder::default();
        let mut engine =
            ScriptEngine::parse(example("example-router-first-run")).expect("example parses");
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert!(engine.on_output(b"root@OpenWrtMx4300:~# "));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Ask);
        (engine, host)
    }

    /// "no" must take the skip branch, which changes nothing on the device.
    #[test]
    fn first_run_example_no_branch_changes_nothing() {
        let (mut engine, mut host) = first_run_at_question();
        engine.on_ask_result(Some("no".to_string()));
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert!(
            host.events
                .iter()
                .any(|e| e == "msg:Leaving the router unchanged.:true")
        );
        assert!(
            host.events
                .iter()
                .any(|e| e == "print:First-run helper finished.")
        );
        // The skip branch must never touch the device.
        assert!(!host.events.iter().any(|e| e.starts_with("send:")));
    }

    /// An answer that is neither yes nor no re-asks instead of guessing.
    #[test]
    fn first_run_example_reasks_on_invalid_answer() {
        let (mut engine, mut host) = first_run_at_question();
        engine.on_ask_result(Some("maybe".to_string()));
        engine.pump(&mut host);

        assert_eq!(engine.pending(), PendingKind::Ask);
        assert!(
            host.events
                .iter()
                .any(|e| e == "print:Please answer yes or no.")
        );
        let asks = host.events.iter().filter(|e| e.starts_with("ask:")).count();
        assert_eq!(asks, 2, "the question is asked again");
    }

    /// "yes" takes the configure branch: read the hostname, then ask for a new
    /// one; an invalid hostname changes nothing, a valid one is sent.
    #[test]
    fn first_run_example_yes_branch_configures() {
        let (mut engine, mut host) = first_run_at_question();
        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);

        // Configure branch reads the current hostname.
        assert_eq!(engine.pending(), PendingKind::Capture);
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("uci show system.@system[0].hostname"))
        );

        assert!(engine.on_output(b"system.@system[0].hostname='OpenWrtMx4300'\r\n"));
        engine.pump(&mut host);

        // Now it asks for the new name, in clear text (echo = true).
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert!(
            host.events
                .iter()
                .any(|e| e == "print:Current hostname: OpenWrtMx4300")
        );
        assert!(
            host.events
                .iter()
                .any(|e| e == "ask:New hostname (letters, digits and dashes only)::true:")
        );

        // A valid name is applied.
        engine.on_ask_result(Some("my-router".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("uci set system.@system[0].hostname=my-router"))
        );

        // The prompt closes the script.
        assert!(engine.on_output(b"root@OpenWrtMx4300:~# "));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert!(
            host.events
                .iter()
                .any(|e| e.starts_with("print:Hostname set to my-router"))
        );
    }

    /// A hostname with a space or a shell metacharacter is rejected before it
    /// can reach the device.
    #[test]
    fn first_run_example_rejects_invalid_hostname() {
        let (mut engine, mut host) = first_run_at_question();
        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        engine.on_output(b"system.@system[0].hostname='OpenWrtMx4300'\r\n");
        engine.pump(&mut host);

        engine.on_ask_result(Some("bad name; rm -rf /".to_string()));
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert!(
            host.events
                .iter()
                .any(|e| e == "print:Invalid hostname. Nothing was changed.")
        );
        assert!(
            !host
                .events
                .iter()
                .any(|e| e.contains("uci set system.@system[0].hostname=")),
            "an invalid name is never sent"
        );
    }

    /// The uptime examples must match real prompt lines and the uptime output.
    #[test]
    fn uptime_example_matches_openwrt_output() {
        let program = rustconn_core::automation::parse_program(
            "expect \"#\" 15\nsend \"uptime{ENTER}\"\ncapture /up\\s+([^,]+)/ up 10\nprint \"Uptime: $up\"\n",
        )
        .expect("openwrt uptime example parses");
        assert_eq!(program.len(), 4);

        assert!("root@OpenWrt:~# ".contains('#'));

        let uptime = regex::Regex::new("up\\s+([^,]+)").expect("uptime regex");
        let caps = uptime
            .captures(" 14:30:00 up 2 days,  3:21,  load average: 0.00, 0.01, 0.05")
            .expect("uptime output matches");
        assert_eq!(&caps[1], "2 days");
    }

    /// OpenWrt's UCI dump lands in a timestamped log after an explicit yes,
    /// named after the host in the prompt.
    #[test]
    fn openwrt_save_uci_writes_a_timestamped_log_after_yes() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-openwrt-save-uci");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nroot@OpenWrtMx4300:~# ");
        feed(&mut engine, &mut host, "\r\nroot@OpenWrtMx4300:~# ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(
            log_name(&path).starts_with("openwrt-OpenWrtMx4300-"),
            "{path}"
        );
        assert!(host.events.iter().any(|e| e.contains("uci export")));

        feed(
            &mut engine,
            &mut host,
            "\r\npackage network\r\n\r\nconfig interface 'lan'\r\n\toption proto 'static'\r\nroot@OpenWrtMx4300:~# ",
        );
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:UCI configuration saved to {path}"))
        );
    }

    /// A non-root shell (`user@host:~$`) is refused before the dump.
    #[test]
    fn openwrt_save_uci_refuses_non_root() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-openwrt-save-uci");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nuser@OpenWrt:~$ ");
        feed(&mut engine, &mut host, "\r\nuser@OpenWrt:~$ ");

        assert!(engine.is_finished());
        assert!(
            host.events
                .iter()
                .any(|e| e.starts_with("msg:This script needs a root shell"))
        );
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert!(!host.events.iter().any(|e| e.starts_with("ask:")));
    }

    /// Drives the Cisco save example up to its confirmation question.
    fn cisco_save_at_question() -> (ScriptEngine, Recorder) {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-cisco-save-config");
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Capture);
        // First ENTER: the host name from the fresh prompt.
        feed(&mut engine, &mut host, "\r\nR1# ");
        assert_eq!(engine.pending(), PendingKind::Capture);
        // Second ENTER: the mode character.
        feed(&mut engine, &mut host, "\r\nR1# ");
        assert_eq!(engine.pending(), PendingKind::Ask);
        (engine, host)
    }

    /// The safe default: an empty answer cancels, removing every local write
    /// and every read of the running configuration.
    #[test]
    fn cisco_save_config_cancels_without_reading_or_writing() {
        let (mut engine, mut host) = cisco_save_at_question();
        engine.on_ask_result(Some(String::new()));
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert!(
            !host
                .events
                .iter()
                .any(|e| e.contains("show running-config"))
        );
        assert!(host.events.iter().any(|e| e == "print:Nothing was saved."));
    }

    /// `n` and `no` are cancellations too, in any case and with surrounding
    /// spaces; they must not become the file name.
    #[test]
    fn save_config_treats_no_as_cancel() {
        for answer in ["n", "no", "N", "No", "  no  "] {
            let (mut engine, mut host) = cisco_save_at_question();
            engine.on_ask_result(Some(answer.to_string()));
            engine.pump(&mut host);

            assert!(engine.is_finished(), "{answer:?}");
            assert_eq!(engine.error(), None, "{answer:?}");
            assert!(
                !host.events.iter().any(|event| event.starts_with("log:")),
                "{answer:?}"
            );
            assert!(
                !host
                    .events
                    .iter()
                    .any(|event| event.contains("show running-config")),
                "{answer:?}"
            );
            assert!(
                host.events
                    .iter()
                    .any(|event| event == "print:Nothing was saved."),
                "{answer:?}"
            );
        }
    }

    /// "yes" captures the running configuration into a file named after the
    /// device and the clock, and stops the log at the end.
    #[test]
    fn cisco_save_config_writes_a_timestamped_log_after_yes() {
        let (mut engine, mut host) = cisco_save_at_question();
        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        // `terminal length 0` returns to the prompt.
        feed(&mut engine, &mut host, "\r\nR1# ");
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(log_name(&path).starts_with("cisco-R1-"), "{path}");
        assert!(std::path::Path::new(&path).is_absolute(), "{path}");
        assert!(
            std::path::Path::new(&path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("log")),
            "{path}"
        );
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("show running-config"))
        );

        // The configuration arrives, then the prompt.
        feed(&mut engine, &mut host, "\r\nhostname R1\r\nR1# ");
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(
            host.events.iter().filter(|e| *e == "log:").count(),
            1,
            "the log is stopped after the capture"
        );
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:Running configuration saved to {path}"))
        );
    }

    /// A prompt that arrives while the save question is open belongs to the
    /// previous command; it must not satisfy the `expect` after the answer.
    #[test]
    fn cisco_save_config_ignores_a_late_prompt() {
        let (mut engine, mut host) = cisco_save_at_question();
        // The second setup ENTER's prompt arrives late, after the engine has
        // already moved to the question.
        engine.on_output(b"\r\nR1# ");
        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert!(
            !engine.is_finished(),
            "a stale prompt must not skip the read"
        );

        // `terminal length 0` answers with a prompt only.
        feed(&mut engine, &mut host, "\r\nR1# ");
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert!(
            !engine.is_finished(),
            "the log must wait for show running-config"
        );

        feed(&mut engine, &mut host, "\r\nhostname R1\r\nR1# ");
        assert!(engine.is_finished());
    }

    /// A user-EXEC prompt (`>`) sends `enable` and continues in privileged
    /// mode instead of refusing.
    #[test]
    fn cisco_save_config_escalates_from_user_exec() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-cisco-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nR1> ");
        feed(&mut engine, &mut host, "\r\nR1> ");
        assert_eq!(engine.pending(), PendingKind::Capture);
        assert!(host.events.iter().any(|e| e == "send:enable{ENTER}"));

        // The switch accepts `enable` without a password.
        feed(&mut engine, &mut host, "\r\nR1# ");
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert!(!host.events.iter().any(|e| e.starts_with("msg:")));
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
    }

    /// When `enable` asks for a password, the script asks for it masked and
    /// only then continues; the answer never reaches the transcript.
    #[test]
    fn enable_password_is_asked_and_used() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-cisco-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nR1> ");
        feed(&mut engine, &mut host, "\r\nR1> ");
        feed(&mut engine, &mut host, "\r\nPassword: ");

        assert_eq!(engine.pending(), PendingKind::Ask);
        assert!(
            host.events
                .iter()
                .any(|e| e == "ask:Enable password::false:"),
            "the enable password must be asked masked"
        );

        engine.on_ask_result(Some("s3cret".to_string()));
        engine.pump(&mut host);
        assert!(host.events.iter().any(|e| e == "send:s3cret{ENTER}"));

        feed(&mut engine, &mut host, "\r\nR1# ");
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
    }

    /// A rejected enable password leaves the session in user mode and stops
    /// before anything is read.
    #[test]
    fn rejected_enable_password_stops_the_script() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-cisco-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nR1> ");
        feed(&mut engine, &mut host, "\r\nR1> ");
        feed(&mut engine, &mut host, "\r\nPassword: ");
        engine.on_ask_result(Some("wrong".to_string()));
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\n% Access denied\r\nR1> ");

        assert!(engine.is_finished());
        assert!(
            host.events
                .iter()
                .any(|e| e.starts_with("msg:This script could not enter privileged mode"))
        );
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert_eq!(
            host.events.iter().filter(|e| e.starts_with("ask:")).count(),
            1,
            "only the enable password is asked; the save question never appears"
        );
    }

    /// The Huawei profile reads `display current-configuration` in user view,
    /// after the VRP paging command, into a timestamped log.
    #[test]
    fn huawei_save_config_writes_a_timestamped_log_after_yes() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-huawei-save-config");
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Capture);
        feed(&mut engine, &mut host, "\r\n<SW1> ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        // `screen-length 0 temporary` returns to the user-view prompt.
        feed(&mut engine, &mut host, "\r\n<SW1> ");
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(log_name(&path).starts_with("huawei-SW1-"), "{path}");
        assert!(
            std::path::Path::new(&path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("log")),
            "{path}"
        );
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("screen-length 0 temporary"))
        );
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("display current-configuration"))
        );

        feed(&mut engine, &mut host, "\r\nsysname SW1\r\n<SW1> ");
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:Current configuration saved to {path}"))
        );
    }

    /// Huawei system view is refused before the paging command, which is a
    /// user-view command, is sent.
    #[test]
    fn huawei_save_config_refuses_system_view() {
        let mut host = Recorder {
            prompt: Some("[SW1]".to_string()),
            ..Recorder::default()
        };
        let mut engine = engine_for("example-huawei-save-config");
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert!(
            host.events
                .iter()
                .any(|e| e.starts_with("msg:This script must run in user view"))
        );
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert!(!host.events.iter().any(|e| e.starts_with("ask:")));
        assert!(!host.events.iter().any(|e| e.starts_with("send:")));
    }

    /// The HSGQ profile reads `show current-config` in enable mode
    /// (`OLT#`), into a timestamped log.
    #[test]
    fn hsgq_save_config_writes_a_timestamped_log_after_yes() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-hsgq-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nOLT# ");
        feed(&mut engine, &mut host, "\r\nOLT# ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(log_name(&path).starts_with("hsgq-OLT-"), "{path}");
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("show current-config"))
        );

        feed(&mut engine, &mut host, "\r\nhostname OLT\r\nOLT# ");
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:Current configuration saved to {path}"))
        );
    }

    /// The D-Link xStack profile reads `show configuration running` in
    /// administrator mode, after `disable clipaging`, into a timestamped log.
    #[test]
    fn dlink_xstack_save_config_writes_a_timestamped_log_after_yes() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-dlink-xstack-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nDGS-3100-24# ");
        feed(&mut engine, &mut host, "\r\nDGS-3100-24# ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        // `disable clipaging` returns to the prompt.
        feed(&mut engine, &mut host, "\r\nDGS-3100-24# ");
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(
            log_name(&path).starts_with("dlink-xstack-DGS-3100-24-"),
            "{path}"
        );
        assert!(host.events.iter().any(|e| e.contains("disable clipaging")));
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("show configuration running"))
        );

        // The configuration arrives, then the prompt.
        feed(&mut engine, &mut host, "\r\nvlan 10\r\nDGS-3100-24# ");
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:Running configuration saved to {path}"))
        );
    }

    /// The DGS-1210 web-smart profile sends no paging command (none exists),
    /// reads `show config`, and names the file after the model before the
    /// colon instead of the logged-in account.
    #[test]
    fn dlink_dgs1210_save_config_writes_a_timestamped_log_after_yes() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-dlink-dgs1210-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nDGS-1210-24:admin# ");
        feed(&mut engine, &mut host, "\r\nDGS-1210-24:admin# ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(log_name(&path).starts_with("dlink-DGS-1210-24-"), "{path}");
        assert!(host.events.iter().any(|e| e.contains("show config")));
        assert!(
            !host
                .events
                .iter()
                .any(|e| e.starts_with("send:") && e.contains("clipaging"))
        );

        feed(&mut engine, &mut host, "\r\nvlan 10\r\nDGS-1210-24:admin# ");
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:Configuration saved to {path}"))
        );
    }

    /// Junos reads `show configuration | no-more` in operational mode; the
    /// `#` configuration prompt is refused.
    #[test]
    fn juniper_save_config_writes_a_timestamped_log_after_yes() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-juniper-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nadmin@SW1> ");
        feed(&mut engine, &mut host, "\r\nadmin@SW1> ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(log_name(&path).starts_with("juniper-SW1-"), "{path}");
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("show configuration | no-more"))
        );

        feed(
            &mut engine,
            &mut host,
            "\r\nsystem { host-name SW1; }\r\nadmin@SW1> ",
        );
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:Configuration saved to {path}"))
        );
    }

    /// Junos configuration mode (`user@host#`) is refused before anything is
    /// read or written.
    #[test]
    fn juniper_save_config_refuses_configuration_mode() {
        let mut host = Recorder {
            prompt: Some("admin@SW1#".to_string()),
            ..Recorder::default()
        };
        let mut engine = engine_for("example-juniper-save-config");
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert!(
            host.events
                .iter()
                .any(|e| e.starts_with("msg:This script must run in operational mode"))
        );
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert!(!host.events.iter().any(|e| e.starts_with("ask:")));
        assert!(!host.events.iter().any(|e| e.starts_with("send:")));
    }

    /// The TP-Link profile sends no paging command (none is documented) and
    /// reads `show running-config` in privileged EXEC mode.
    #[test]
    fn tplink_save_config_writes_a_timestamped_log_after_yes() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-tplink-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nSwitch# ");
        feed(&mut engine, &mut host, "\r\nSwitch# ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(log_name(&path).starts_with("tplink-Switch-"), "{path}");
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("show running-config"))
        );
        assert!(
            !host
                .events
                .iter()
                .any(|e| e.starts_with("send:") && e.contains("pagination"))
        );

        feed(&mut engine, &mut host, "\r\nvlan 10\r\nSwitch# ");
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
        assert!(
            host.events
                .iter()
                .any(|e| *e == format!("print:Running configuration saved to {path}"))
        );
    }

    /// Wrong-mode prompts are refused before anything is sent.
    #[test]
    fn save_config_profiles_refuse_wrong_mode() {
        for (id, prompt, feed_text, feeds) in [
            (
                "example-dlink-xstack-save-config",
                "DGS-3100-24(config)#",
                "",
                0,
            ),
            (
                "example-dlink-dgs1210-save-config",
                "DGS-1210-24:admin>",
                "\r\nDGS-1210-24:admin> ",
                2,
            ),
            ("example-tplink-save-config", "Switch(config)#", "", 0),
            ("example-h3c-save-config", "[H3C]", "", 0),
            ("example-zte-save-config", "ZXR10(config)#", "", 0),
            ("example-bdcom-save-config", "switch_config#", "", 0),
            ("example-ubiquiti-save-config", "ubnt@EdgeRouter#", "", 0),
            ("example-huawei-olt-save-config", "MA5680T(config)#", "", 0),
            ("example-aruba-save-config", "Switch(config)#", "", 0),
        ] {
            let mut host = Recorder {
                prompt: Some(prompt.to_string()),
                ..Recorder::default()
            };
            let mut engine = engine_for(id);
            engine.pump(&mut host);
            for _ in 0..feeds {
                feed(&mut engine, &mut host, feed_text);
            }

            assert!(engine.is_finished(), "{id} at {prompt}");
            assert!(
                host.events.iter().any(|e| e.starts_with("msg:")),
                "{id} at {prompt}"
            );
            assert!(
                !host.events.iter().any(|e| e.starts_with("log:")),
                "{id} at {prompt}"
            );
            assert!(
                !host.events.iter().any(|e| e.starts_with("ask:")),
                "{id} at {prompt}"
            );
            assert!(
                !host
                    .events
                    .iter()
                    .any(|e| e.starts_with("send:") && e.contains("config")),
                "{id} at {prompt}"
            );
        }
    }

    /// HSGQ configuration mode is refused; user mode escalates with `enable`
    /// instead.
    #[test]
    fn hsgq_save_config_refuses_config_mode_and_escalates_from_user_mode() {
        let mut host = Recorder {
            prompt: Some("OLT(config)#".to_string()),
            ..Recorder::default()
        };
        let mut engine = engine_for("example-hsgq-save-config");
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert!(host.events.iter().any(|e| e.starts_with("msg:")));
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert!(!host.events.iter().any(|e| e.starts_with("ask:")));

        let mut host = Recorder::default();
        let mut engine = engine_for("example-hsgq-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nOLT> ");
        feed(&mut engine, &mut host, "\r\nOLT> ");
        assert_eq!(engine.pending(), PendingKind::Capture);
        assert!(host.events.iter().any(|e| e == "send:enable{ENTER}"));

        feed(&mut engine, &mut host, "\r\nOLT# ");
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert_eq!(engine.error(), None);
    }

    /// The CIS/China profiles: each reads its vendor command into a
    /// timestamped log after an explicit yes.
    #[test]
    fn cis_and_china_profiles_write_a_timestamped_log_after_yes() {
        struct Case {
            id: &'static str,
            feeds: &'static [&'static str],
            prefix: &'static str,
            pager: Option<&'static str>,
            /// Prompt feeds needed between "yes" and the read command: one for
            /// a pager command or a RouterOS menu change, none otherwise.
            steps: u32,
            sent: &'static [&'static str],
            tail: &'static str,
        }

        let cases = [
            Case {
                id: "example-h3c-save-config",
                feeds: &["\r\n<H3C> "],
                prefix: "h3c-H3C-",
                pager: Some("screen-length disable"),
                steps: 1,
                sent: &["display current-configuration"],
                tail: "\r\nsysname H3C\r\n<H3C> ",
            },
            Case {
                id: "example-ruijie-save-config",
                feeds: &["\r\nRuijie# ", "\r\nRuijie# "],
                prefix: "ruijie-Ruijie-",
                pager: Some("terminal length 0"),
                steps: 1,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nRuijie# ",
            },
            Case {
                id: "example-zte-save-config",
                feeds: &["\r\nZXR10# ", "\r\nZXR10# "],
                prefix: "zte-ZXR10-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nZXR10# ",
            },
            Case {
                id: "example-eltex-save-config",
                feeds: &["\r\nconsole# ", "\r\nconsole# "],
                prefix: "eltex-console-",
                pager: None,
                steps: 0,
                sent: &["show running-config detailed"],
                tail: "\r\nvlan 10\r\nconsole# ",
            },
            Case {
                id: "example-bdcom-save-config",
                feeds: &["\r\nSwitch# ", "\r\nSwitch# "],
                prefix: "bdcom-Switch-",
                pager: Some("terminal length 0"),
                steps: 1,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nSwitch# ",
            },
            Case {
                id: "example-ubiquiti-save-config",
                feeds: &["\r\nubnt@EdgeRouter:~$ ", "\r\nubnt@EdgeRouter:~$ "],
                prefix: "ubiquiti-EdgeRouter-",
                pager: None,
                steps: 0,
                sent: &["show configuration commands"],
                tail: "\r\nset system host-name EdgeRouter\r\nubnt@EdgeRouter:~$ ",
            },
            Case {
                id: "example-raisecom-save-config",
                feeds: &["\r\nRaisecom# ", "\r\nRaisecom# "],
                prefix: "raisecom-Raisecom-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nRaisecom# ",
            },
            Case {
                id: "example-qtech-save-config",
                feeds: &["\r\nQTECH# ", "\r\nQTECH# "],
                prefix: "qtech-QTECH-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nQTECH# ",
            },
            Case {
                id: "example-zyxel-save-config",
                feeds: &["\r\nGS2210-24# ", "\r\nGS2210-24# "],
                prefix: "zyxel-GS2210-24-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nGS2210-24# ",
            },
            Case {
                id: "example-cdata-save-config",
                feeds: &["\r\nOLT# ", "\r\nOLT# "],
                prefix: "cdata-OLT-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nOLT# ",
            },
            Case {
                id: "example-vsol-save-config",
                feeds: &["\r\nOLT# ", "\r\nOLT# "],
                prefix: "vsol-OLT-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nOLT# ",
            },
            Case {
                id: "example-maipu-save-config",
                feeds: &["\r\nMaipu# ", "\r\nMaipu# "],
                prefix: "maipu-Maipu-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nMaipu# ",
            },
            Case {
                id: "example-hikvision-save-config",
                feeds: &["\r\nHikvision# ", "\r\nHikvision# "],
                prefix: "hikvision-Hikvision-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nHikvision# ",
            },
            Case {
                id: "example-dahua-save-config",
                feeds: &["\r\nDahua# ", "\r\nDahua# "],
                prefix: "dahua-Dahua-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nDahua# ",
            },
            Case {
                id: "example-netgear-save-config",
                feeds: &["\r\n(GS724Tv4)# ", "\r\n(GS724Tv4)# "],
                prefix: "netgear-GS724Tv4-",
                pager: None,
                steps: 0,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\n(GS724Tv4)# ",
            },
            Case {
                id: "example-aruba-save-config",
                feeds: &["\r\nSwitch# ", "\r\nSwitch# "],
                prefix: "aruba-Switch-",
                pager: Some("no page"),
                steps: 1,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nSwitch# ",
            },
            Case {
                id: "example-arista-save-config",
                feeds: &["\r\nswitch# ", "\r\nswitch# "],
                prefix: "arista-switch-",
                pager: Some("terminal length 0"),
                steps: 1,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nswitch# ",
            },
            Case {
                id: "example-extreme-save-config",
                feeds: &["\r\n*X460-48t.1 # ", "\r\n*X460-48t.1 # "],
                prefix: "extreme-X460-48t.1-",
                pager: Some("disable clipaging"),
                steps: 1,
                sent: &["show configuration"],
                tail: "\r\ncreate vlan ToR\r\n*X460-48t.1 # ",
            },
            Case {
                id: "example-allied-save-config",
                feeds: &["\r\nawplus# ", "\r\nawplus# "],
                prefix: "allied-awplus-",
                pager: Some("terminal length 0"),
                steps: 1,
                sent: &["show running-config"],
                tail: "\r\nvlan 10\r\nawplus# ",
            },
        ];

        for case in cases {
            let mut host = Recorder::default();
            let mut engine = engine_for(case.id);
            engine.pump(&mut host);
            for output in case.feeds {
                feed(&mut engine, &mut host, output);
            }
            assert_eq!(engine.pending(), PendingKind::Ask, "{}", case.id);

            engine.on_ask_result(Some("yes".to_string()));
            engine.pump(&mut host);

            if let Some(pager) = case.pager {
                assert!(
                    host.events
                        .iter()
                        .any(|e| e.starts_with("send:") && e.contains(pager)),
                    "{} does not send {pager}",
                    case.id
                );
            }
            for _ in 0..case.steps {
                assert_eq!(engine.pending(), PendingKind::Expect, "{}", case.id);
                feed(&mut engine, &mut host, case.feeds[0]);
            }

            let path = host
                .events
                .iter()
                .find_map(|e| e.strip_prefix("log:"))
                .unwrap_or_else(|| panic!("{} never opened a log", case.id))
                .to_string();
            assert!(
                log_name(&path).starts_with(case.prefix),
                "{}: {path}",
                case.id
            );
            for command in case.sent {
                assert!(
                    host.events
                        .iter()
                        .any(|e| e.starts_with("send:") && e.contains(command)),
                    "{} does not send {command}",
                    case.id
                );
            }

            feed(&mut engine, &mut host, case.tail);
            assert!(engine.is_finished(), "{}", case.id);
            assert_eq!(engine.error(), None, "{}", case.id);
            assert_eq!(
                host.events.iter().filter(|e| *e == "log:").count(),
                1,
                "{} left the log open",
                case.id
            );
        }
    }

    /// The Huawei OLT profile cannot disable paging, so the `pager`
    /// directive answers `---- More ----` and the dump still completes.
    #[test]
    fn huawei_olt_save_config_answers_the_pager() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-huawei-olt-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nMA5680T# ");
        feed(&mut engine, &mut host, "\r\nMA5680T# ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the log starts before the command")
            .to_string();
        assert!(log_name(&path).starts_with("huawei-olt-MA5680T-"), "{path}");
        assert!(
            host.events
                .iter()
                .any(|e| e.contains("display current-configuration"))
        );

        // The optional filter prompt does not end the wait.
        engine.on_output(b"\r\n { |ont|port|section|service-port|simple|| }:");
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);

        // Each `---- More ----` page is answered with a space.
        assert!(engine.on_output(b"\r\nline 1\r\n---- More ( Press 'Q' to break ) ----"));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Expect);
        assert!(host.events.iter().any(|e| e == "send: "));

        assert!(engine.on_output(b"\r\nline 2\r\nMA5680T# "));
        engine.pump(&mut host);
        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert_eq!(host.events.iter().filter(|e| *e == "log:").count(), 1);
    }

    /// Drives the OLT provisioning example to its final `Apply` question.
    fn provisioning_at_apply() -> (ScriptEngine, Recorder) {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-huawei-olt-provision-ont");
        engine.pump(&mut host);
        // Enable-mode prompt: device and mode are captured from one feed each.
        feed(&mut engine, &mut host, "\r\nMA5680T# ");
        feed(&mut engine, &mut host, "\r\nMA5680T# ");
        feed(&mut engine, &mut host, "\r\nMA5680T(config)# ");

        assert_eq!(engine.pending(), PendingKind::Ask);
        let first = host.events.last().cloned().unwrap_or_default();
        assert_eq!(first, "ask:Service VLAN (1-4094)::true:500");

        // Out-of-range answers are rejected before anything is contacted.
        engine.on_ask_result(Some("5000".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert!(
            host.events
                .iter()
                .any(|e| e == "print:The VLAN must be a whole number from 1 to 4094."),
            "an invalid VLAN is explained"
        );

        for answer in [
            "500",
            "0/19",
            "0",
            "1",
            "0",
            "1",
            "48575443AABBCCDD",
            "10",
            "10",
            "10",
            "10",
            "4",
            "RUSTCONN_EXAMPLE",
        ] {
            assert_eq!(engine.pending(), PendingKind::Ask);
            engine.on_ask_result(Some(answer.to_string()));
            engine.pump(&mut host);
        }

        (engine, host)
    }

    /// The provisioning example shows the collected values and writes nothing
    /// when the operator does not type yes.
    #[test]
    fn huawei_olt_provision_previews_and_cancels() {
        let (mut engine, mut host) = provisioning_at_apply();
        assert_eq!(engine.pending(), PendingKind::Ask);
        assert!(host.events.iter().any(|e| e == "print:  VLAN: 500"));
        assert!(
            host.events
                .iter()
                .any(|e| e == "print:  Uplink: 0/19 port 0")
        );
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));

        engine.on_ask_result(Some(String::new()));
        engine.pump(&mut host);

        assert!(engine.is_finished());
        assert_eq!(engine.error(), None);
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert!(
            host.events
                .iter()
                .all(|e| !e.starts_with("send:") || !e.contains("vlan "))
        );
    }

    /// Yes starts the write path: the transcript is logged, commands are
    /// sent in order, and no `save` is ever issued.
    #[test]
    fn huawei_olt_provision_writes_after_yes_without_saving() {
        let (mut engine, mut host) = provisioning_at_apply();
        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);

        let path = host
            .events
            .iter()
            .find_map(|e| e.strip_prefix("log:"))
            .expect("the transcript starts before the first command")
            .to_string();
        assert!(
            log_name(&path).starts_with("huawei-olt-provision-MA5680T-"),
            "{path}"
        );
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:vlan 500 smart{ENTER}")
        );

        feed(&mut engine, &mut host, "\r\nMA5680T(config)# ");
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:port vlan 500 0/19 0{ENTER}")
        );

        feed(&mut engine, &mut host, "\r\nMA5680T(config)# ");
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:ont-srvprofile gpon profile-id 10{ENTER}")
        );

        feed(
            &mut engine,
            &mut host,
            "\r\nMA5680T(config-gpon-srvprofile-10)# ",
        );
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:ont-port eth 4{ENTER}")
        );

        feed(
            &mut engine,
            &mut host,
            "\r\nMA5680T(config-srvprofile-gpon-10)# ",
        );
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:port vlan eth 1-4 500{ENTER}"),
            "all Ethernet ports are mapped to the service VLAN"
        );
        assert!(
            host.events
                .iter()
                .all(|e| !e.starts_with("send:") || !e.contains("save")),
            "provisioning must not save the configuration"
        );
    }

    /// The exports that carry secrets say so in the question, before the
    /// user answers yes.
    #[test]
    fn secret_bearing_exports_warn_in_the_question() {
        for id in ["example-mikrotik-save-config", "example-eltex-save-config"] {
            assert!(
                example(id).contains("The log will contain secrets."),
                "`{id}` must warn that the log contains secrets"
            );
        }
    }

    /// MikroTik sends the absolute `/export`, so a session left inside
    /// `/interface` still exports the whole configuration.
    #[test]
    fn mikrotik_save_config_uses_the_absolute_export_command() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-mikrotik-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\n[admin@MikroTik] /interface> ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:/system resource print{ENTER}"),
            "the script reads the RouterOS major version first"
        );
        assert_eq!(engine.pending(), PendingKind::Capture);

        feed(
            &mut engine,
            &mut host,
            "\r\nversion: 7.19.6 (stable)\r\n[admin@MikroTik] > ",
        );
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:/export verbose show-sensitive{ENTER}"),
            "RouterOS 7 needs show-sensitive for a complete export"
        );
        assert!(
            host.events.iter().any(|event| {
                event
                    .strip_prefix("log:")
                    .is_some_and(|path| log_name(path).starts_with("mikrotik-MikroTik-"))
            }),
            "the transcript uses the device name"
        );
    }

    /// RouterOS 6 shows secrets by default and may reject `show-sensitive`,
    /// so the same script must fall back to the plain verbose export.
    #[test]
    fn mikrotik_save_config_uses_plain_export_on_routeros_6() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-mikrotik-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\n[admin@MikroTik] > ");
        assert_eq!(engine.pending(), PendingKind::Ask);

        engine.on_ask_result(Some("yes".to_string()));
        engine.pump(&mut host);
        assert_eq!(engine.pending(), PendingKind::Capture);
        feed(
            &mut engine,
            &mut host,
            "\r\nversion: 6.49.17 (stable)\r\n[admin@MikroTik] > ",
        );
        assert!(
            host.events
                .iter()
                .any(|e| e == "send:/export verbose{ENTER}"),
            "RouterOS 6 uses the plain verbose export"
        );
        assert!(
            !host
                .events
                .iter()
                .any(|e| e == "send:/export verbose show-sensitive{ENTER}"),
            "the RouterOS 7 form is not sent"
        );
    }

    /// Both vendor profiles cancel without reading the configuration or
    /// opening a local file.
    #[test]
    fn vendor_save_configs_cancel_without_reading_or_writing() {
        for (id, prompt, captures) in [
            ("example-huawei-save-config", "\r\n<SW1> ", 1),
            ("example-hsgq-save-config", "\r\nOLT# ", 2),
            ("example-dlink-xstack-save-config", "\r\nDGS-3100-24# ", 2),
            (
                "example-dlink-dgs1210-save-config",
                "\r\nDGS-1210-24:admin# ",
                2,
            ),
            ("example-juniper-save-config", "\r\nadmin@SW1> ", 2),
            ("example-tplink-save-config", "\r\nSwitch# ", 2),
            ("example-h3c-save-config", "\r\n<H3C> ", 1),
            ("example-ruijie-save-config", "\r\nRuijie# ", 2),
            ("example-zte-save-config", "\r\nZXR10# ", 2),
            ("example-mikrotik-save-config", "\r\n[admin@MikroTik] > ", 1),
            ("example-eltex-save-config", "\r\nconsole# ", 2),
            ("example-bdcom-save-config", "\r\nSwitch# ", 2),
            ("example-ubiquiti-save-config", "\r\nubnt@EdgeRouter:~$ ", 2),
            ("example-huawei-olt-save-config", "\r\nMA5680T# ", 2),
            ("example-raisecom-save-config", "\r\nRaisecom# ", 2),
            ("example-qtech-save-config", "\r\nQTECH# ", 2),
            ("example-zyxel-save-config", "\r\nGS2210-24# ", 2),
            ("example-cdata-save-config", "\r\nOLT# ", 2),
            ("example-vsol-save-config", "\r\nOLT# ", 2),
            ("example-maipu-save-config", "\r\nMaipu# ", 2),
            ("example-hikvision-save-config", "\r\nHikvision# ", 2),
            ("example-dahua-save-config", "\r\nDahua# ", 2),
            ("example-netgear-save-config", "\r\n(GS724Tv4)# ", 2),
            ("example-aruba-save-config", "\r\nSwitch# ", 2),
            ("example-arista-save-config", "\r\nswitch# ", 2),
            ("example-extreme-save-config", "\r\n*X460-48t.1 # ", 2),
            ("example-allied-save-config", "\r\nawplus# ", 2),
        ] {
            let mut host = Recorder::default();
            let mut engine = engine_for(id);
            engine.pump(&mut host);
            for _ in 0..captures {
                feed(&mut engine, &mut host, prompt);
            }
            assert_eq!(engine.pending(), PendingKind::Ask, "{id}");

            engine.on_ask_result(Some(String::new()));
            engine.pump(&mut host);

            assert!(engine.is_finished(), "{id}");
            assert_eq!(engine.error(), None, "{id}");
            assert!(!host.events.iter().any(|e| e.starts_with("log:")), "{id}");
            assert!(
                !host
                    .events
                    .iter()
                    .any(|e| e.starts_with("send:") && e.contains("config")),
                "{id}"
            );
            assert!(
                host.events.iter().any(|e| e == "print:Nothing was saved."),
                "{id}"
            );
        }
    }
}
