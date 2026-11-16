//! Scripts manager dialog: list, edit, validate, delete and run `.rsc`
//! scripts from the script library.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Entry, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow, TextView,
};
use libadwaita as adw;
use rustconn_core::script::ScriptManager;

use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::window::types::SharedNotebook;
use uuid::Uuid;

const BUNDLED_EXAMPLES: &[(&str, &str)] = &[
    (
        "example-shell-uptime",
        // A shell prompt ending in `$` (local shell, most SSH hosts).
        // `capture` grabs the uptime text itself; `readline` would return
        // the echoed `uptime` command line.
        "expect \"\\$\" 10\nsend \"uptime{ENTER}\"\ncapture /up\\s+([^,]+)/ up 10\nprint \"Uptime: $up\"\n",
    ),
    (
        "example-openwrt-uptime",
        // OpenWrt / BusyBox root prompt ends in `#` (root@OpenWrt:~#).
        "expect \"#\" 15\nsend \"uptime{ENTER}\"\ncapture /up\\s+([^,]+)/ up 10\nprint \"Uptime: $up\"\n",
    ),
    (
        "example-ping",
        // Inside `send`, a variable must be written `${name}`; `$name` is
        // literal text there (see the language reference).
        "ask \"Host to ping:\" host\nsend \"ping -c 3 ${host}{ENTER}\"\nwait 8000\n",
    ),
    (
        "example-router-first-run",
        // A first-run wizard: asks in clear text, validates the answer and
        // branches. No password is requested or stored.
        //
        // `uci show ...` is captured on the quoted value (`hostname='...'`),
        // not on `hostname=`, because the echoed command line contains the
        // latter and would satisfy the capture before the output arrives.
        "expect \"#\" 15\n\n:ask\nask \"Configure this router now? Type yes or no:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ configure\nif_goto $answer /(?i)^\\s*no\\s*$/ skip\nprint \"Please answer yes or no.\"\ngoto ask\n\n:skip\nmsg \"Leaving the router unchanged.\" ok\ngoto done\n\n:configure\nsend \"uci show system.@system[0].hostname{ENTER}\"\ncapture /hostname='([^']+)'/ old_name 10\nprint \"Current hostname: $old_name\"\nask \"New hostname (letters, digits and dashes only):\" new_name\nif_goto $new_name /^[A-Za-z0-9][A-Za-z0-9-]*$/ apply\nprint \"Invalid hostname. Nothing was changed.\"\ngoto done\n\n:apply\nsend \"uci set system.@system[0].hostname=${new_name} && uci commit system{ENTER}\"\nexpect \"#\" 15\nprint \"Hostname set to $new_name. It takes effect on the next boot.\"\n\n:done\nprint \"First-run helper finished.\"\n",
    ),
    (
        "example-run-command",
        "set cmd \"echo hello from script\"\nrun \"$cmd\" 5 out=o code=c\nprint \"Exit code: $c\"\nprint \"Output: $o\"\n",
    ),
    (
        "example-cisco-save-config",
        // The SecureCRT "Cisco Save Running/Startup Config to Local File"
        // shape, reduced to this DSL: read the running configuration and save
        // it to a timestamped local file, after an explicit yes. The device is
        // never written to. No local shell and no absolute path, so the same
        // script runs on Linux, macOS and Windows; the file lands in
        // RustConn's working directory unless the answer is a full path.
        //
        // The two ENTERs make a fresh prompt arrive even when the session
        // already shows the old one: the first captures the host name, the
        // second the mode character.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script needs privileged EXEC mode (the prompt ends in the hash sign). Run enable, then start it again.\" ok\ngoto done\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:privileged\nset file cisco-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Cisco save-config script finished.\"\n",
    ),
    (
        "example-cisco-inventory",
        // The inventory shape: two read-only show commands, one timestamped
        // log per device, and the same explicit yes before a local file is
        // written. Prompt regex and paging command are the two Cisco-specific
        // assumptions; edit them for another vendor.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script needs privileged EXEC mode (the prompt ends in the hash sign). Run enable, then start it again.\" ok\ngoto done\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:privileged\nset file cisco-inventory-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the inventory to '${file}'? Type yes, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nsend \"show version{ENTER}\"\nexpect \"/[#>]\\s*$/\" 60\nsend \"show interfaces status{ENTER}\"\nexpect \"/[#>]\\s*$/\" 60\nlog\nprint \"Inventory saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Cisco inventory script finished.\"\n",
    ),
    (
        "example-huawei-save-config",
        // The Huawei VRP profile of the save example: user view `<host>`,
        // `display` instead of `show`, and `screen-length 0 temporary`
        // instead of `terminal length 0`. System view `[host]` is refused
        // because the paging command is a user-view command.
        "getprompt prompt\nif_goto $prompt /^\\s*\\[/ config_mode\nsend \"{ENTER}\"\ncapture /<([A-Za-z0-9][A-Za-z0-9._-]*)>\\s*$/ device 10\nset file huawei-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the current configuration to '${file}'? Type yes, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nset file $answer\n\n:save\nsend \"screen-length 0 temporary{ENTER}\"\nexpect \"/<[A-Za-z0-9][A-Za-z0-9._-]*>\\s*$/\" 15\nlog $file\nprint \"Capturing the current configuration...\"\nsend \"display current-configuration{ENTER}\"\nexpect \"/<[A-Za-z0-9][A-Za-z0-9._-]*>\\s*$/\" 120\nlog\nprint \"Current configuration saved to $file\"\ngoto done\n\n:config_mode\nmsg \"This script must run in user view. Leave system view with return, then start the script again.\" ok\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Huawei save-config script finished.\"\n",
    ),
    (
        "example-hsgq-save-config",
        // HSGQ EPON OLT profile: `OLT#` is enable mode, `OLT>` is user
        // mode and `OLT(config)#` is configuration mode. HSGQ documents
        // `show current-config` but no pager command, so no paging line is
        // sent; a `--More--` pause would time the expect out, never save a
        // truncated file silently.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script needs enable mode (the prompt ends in the hash sign). Run enable, then start it again.\" ok\ngoto done\n\n:config_mode\nmsg \"This script must run in enable mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:privileged\nset file hsgq-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the current configuration to '${file}'? Type yes, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nset file $answer\n\n:save\nlog $file\nprint \"Capturing the current configuration...\"\nsend \"show current-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Current configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"HSGQ save-config script finished.\"\n",
    ),
];

/// Writes the bundled examples into the library when they are missing.
///
/// An existing file is never overwritten: the user may have edited it.
fn seed_bundled_examples(manager: &ScriptManager) {
    for (id, source) in BUNDLED_EXAMPLES {
        if !manager.has_script(id) {
            let _ = manager.save_script(id, source);
        }
    }
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
            .content_width(820)
            .content_height(600)
            .build();

        let header = adw::HeaderBar::new();
        let new_btn = Button::from_icon_name("document-new-symbolic");
        new_btn.set_tooltip_text(Some(&i18n("New Script")));
        new_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("New Script"))]);
        header.pack_start(&new_btn);

        let help_btn = Button::from_icon_name("help-about-symbolic");
        help_btn.set_tooltip_text(Some(&i18n("Script language reference")));
        help_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Script language reference",
        ))]);
        header.pack_end(&help_btn);

        let content = GtkBox::new(Orientation::Vertical, 0);
        content.append(&header);

        let body = GtkBox::new(Orientation::Horizontal, 12);
        body.set_margin_top(12);
        body.set_margin_bottom(12);
        body.set_margin_start(12);
        body.set_margin_end(12);

        let script_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .width_request(220)
            .build();
        let placeholder = adw::StatusPage::builder()
            .icon_name("utilities-terminal-symbolic")
            .title(i18n("No Scripts"))
            .description(i18n("Scripts are stored in the script library folder."))
            .build();
        script_list.set_placeholder(Some(&placeholder));
        let list_scroll = ScrolledWindow::builder()
            .child(&script_list)
            .width_request(220)
            .hexpand(false)
            .build();
        body.append(&list_scroll);

        let editor_group = GtkBox::new(Orientation::Vertical, 8);
        editor_group.set_hexpand(true);

        let id_entry = Entry::new();
        id_entry.set_placeholder_text(Some(&i18n("Script id (file name)")));
        let id_row = adw::EntryRow::builder().title(i18n("Script ID")).build();
        id_row.set_child(Some(&id_entry));

        let editor = TextView::builder()
            .hexpand(true)
            .vexpand(true)
            .monospace(true)
            .left_margin(8)
            .right_margin(8)
            .top_margin(8)
            .bottom_margin(8)
            .build();
        editor_group.append(&id_row);
        let editor_scroll = ScrolledWindow::builder()
            .child(&editor)
            .hexpand(true)
            .vexpand(true)
            .build();
        editor_group.append(&editor_scroll);

        let status_label = Label::new(Some(&i18n("Select or create a script.")));
        status_label.set_halign(gtk4::Align::Start);
        status_label.add_css_class("dim-label");
        editor_group.append(&status_label);

        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);

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
            .sensitive(false)
            .build();
        button_box.append(&run_btn);
        button_box.append(&stop_btn);
        button_box.append(&save_btn);
        button_box.append(&delete_btn);
        editor_group.append(&button_box);

        body.append(&editor_group);

        let clamp = adw::Clamp::builder().maximum_size(820).child(&body).build();
        content.append(&clamp);
        dialog.set_child(Some(&content));

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
            label.set_hexpand(true);

            let delete_btn = Button::from_icon_name("user-trash-symbolic");
            delete_btn.add_css_class("flat");
            delete_btn.set_valign(gtk4::Align::Center);
            delete_btn.set_tooltip_text(Some(&i18n("Delete script")));
            delete_btn
                .update_property(&[gtk4::accessible::Property::Label(&i18n("Delete script"))]);

            let box_ = GtkBox::new(Orientation::Horizontal, 8);
            box_.append(&label);
            box_.append(&delete_btn);
            row.set_child(Some(&box_));
            list.append(&row);
            ids_store.push(id.clone());

            let id_delete = id.clone();
            let list_c = list.clone();
            let manager_c = manager.clone();
            let ids_c = ids.clone();
            delete_btn.connect_clicked(move |btn| {
                let list_c = list_c.clone();
                let manager_c = manager_c.clone();
                let ids_c = ids_c.clone();
                confirm_delete_script(btn, &id_delete, move || {
                    Self::refresh_list(&list_c, &manager_c, &ids_c);
                });
            });
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
                    status_list
                        .set_label(&i18n("Editing script. Save writes it back to the library."));
                }
                Err(e) => {
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
        new_btn.connect_clicked(move |_| {
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
            confirm_delete_script(btn, &id, move || {
                id_entry_delete.set_text("");
                editor_delete.buffer().set_text("");
                status_delete.set_label(&i18n("Script deleted."));
                save_delete.set_sensitive(false);
                run_delete.set_sensitive(false);
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

                    match crate::script_runner::ScriptRunner::start_for_session(
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
                        Ok(()) => {
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
                        }
                        Err(e) => {
                            status_run
                                .set_label(&format!("{}: {e}", i18n("Could not start script")));
                        }
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

/// Writes the bundled example scripts into the library when they are missing.
///
/// Called once at startup and again when the Scripts manager opens, so the
/// examples exist in the auto-run dropdowns even if the manager has never been
/// opened. An existing file is never overwritten: the user may have edited it.
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
        fn ask(&mut self, prompt: &str, echo: bool) {
            self.events.push(format!("ask:{prompt}:{echo}"));
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
        assert!(engine.on_output(output.as_bytes()), "output was not taken");
        engine.pump(host);
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
                .any(|e| e == "ask:New hostname (letters, digits and dashes only)::true")
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
        assert!(path.starts_with("cisco-R1-"), "{path}");
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

    /// A user-EXEC prompt (`>`) is refused before anything is read or written.
    #[test]
    fn cisco_save_config_refuses_user_exec() {
        let mut host = Recorder::default();
        let mut engine = engine_for("example-cisco-save-config");
        engine.pump(&mut host);
        feed(&mut engine, &mut host, "\r\nR1> ");
        feed(&mut engine, &mut host, "\r\nR1> ");

        assert!(engine.is_finished());
        assert!(
            host.events
                .iter()
                .any(|e| e.starts_with("msg:This script needs privileged EXEC mode"))
        );
        assert!(!host.events.iter().any(|e| e.starts_with("log:")));
        assert!(!host.events.iter().any(|e| e.starts_with("ask:")));
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
        assert!(path.starts_with("huawei-SW1-"), "{path}");
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
        assert!(path.starts_with("hsgq-OLT-"), "{path}");
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

    /// HSGQ configuration mode and user mode are both refused: only the
    /// `OLT#` enable prompt may read the configuration.
    #[test]
    fn hsgq_save_config_refuses_config_and_user_mode() {
        for prompt in ["OLT(config)#", "OLT>"] {
            let mut host = Recorder {
                prompt: Some(prompt.to_string()),
                ..Recorder::default()
            };
            let mut engine = engine_for("example-hsgq-save-config");
            engine.pump(&mut host);

            if prompt == "OLT>" {
                // User mode is detected from the second prompt capture.
                feed(&mut engine, &mut host, "\r\nOLT> ");
                feed(&mut engine, &mut host, "\r\nOLT> ");
            }

            assert!(engine.is_finished(), "{prompt}");
            assert!(
                host.events.iter().any(|e| e.starts_with("msg:")),
                "{prompt}"
            );
            assert!(
                !host.events.iter().any(|e| e.starts_with("log:")),
                "{prompt}"
            );
            assert!(
                !host.events.iter().any(|e| e.starts_with("ask:")),
                "{prompt}"
            );
        }
    }

    /// Both vendor profiles cancel without reading the configuration or
    /// opening a local file.
    #[test]
    fn vendor_save_configs_cancel_without_reading_or_writing() {
        for (id, prompt, captures) in [
            ("example-huawei-save-config", "\r\n<SW1> ", 1),
            ("example-hsgq-save-config", "\r\nOLT# ", 2),
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
                    .any(|e| e.contains("current-config") || e.contains("current-configuration")),
                "{id}"
            );
            assert!(
                host.events.iter().any(|e| e == "print:Nothing was saved."),
                "{id}"
            );
        }
    }
}
