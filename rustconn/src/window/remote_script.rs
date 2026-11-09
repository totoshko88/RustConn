//! "Run Local Script…": runs a local bash script on the selected SSH hosts.
//!
//! The ssh arguments, jump hosts and passwords come from
//! [`build_key_setup_launch`], i.e. from the same code a live session uses; the
//! run itself is [`rustconn_core::remote_script`].

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;
use rustconn_core::models::PasswordSource;
use rustconn_core::remote_script::{
    self, DEFAULT_PARALLEL, DEFAULT_TIMEOUT, MAX_PARALLEL, RunOptions, ScriptEvent,
    ScriptEventSink, ScriptFileError, ScriptHost, ScriptOutcome,
};
use secrecy::SecretString;
use uuid::Uuid;

use super::protocols::{resolve_globals_with_ask, start_gateway_probe_quiet, substitute_variables};
use super::protocols_ssh::build_key_setup_launch;
use super::types::SharedSidebar;
use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::utils::spawn_blocking_with_callback;

/// How often script progress is copied to the window.
const EVENT_POLL: Duration = Duration::from_millis(50);
/// Most events applied per poll, so a chatty script cannot stall the window.
const EVENTS_PER_POLL: usize = 500;

/// One line of the host list.
struct HostRow {
    status: gtk4::Label,
}

/// The window of one run.
struct RunView {
    dialog: adw::Dialog,
    stack: gtk4::Stack,
    title: adw::WindowTitle,
    run_button: gtk4::Button,
    stop_button: gtk4::Button,
    list: gtk4::ListBox,
    output_view: gtk4::TextView,
    rows: RefCell<Vec<HostRow>>,
    output: RefCell<Vec<String>>,
    selected: Cell<usize>,
    cancel: Arc<AtomicBool>,
    /// The answer for hosts set to ask at connect, until the run has started.
    typed_password: RefCell<Option<SecretString>>,
    succeeded: Cell<usize>,
    failed: Cell<usize>,
    finished: Cell<usize>,
}

fn script_error_text(error: &ScriptFileError) -> String {
    match error {
        ScriptFileError::Unreadable(_) => i18n("The script could not be read"),
        ScriptFileError::NotAFile => i18n("Choose a script file, not a folder"),
        ScriptFileError::Empty => i18n("The script is empty"),
        ScriptFileError::TooLarge { .. } => i18n("The script is larger than 1 MiB"),
        ScriptFileError::Binary => i18n("This is a binary file, not a script"),
    }
}

fn outcome_text(outcome: ScriptOutcome) -> String {
    match outcome {
        ScriptOutcome::Succeeded => i18n("Succeeded"),
        ScriptOutcome::ScriptFailed { code: Some(code) } => {
            i18n_f("Failed with exit code {}", &[&code.to_string()])
        }
        ScriptOutcome::ScriptFailed { code: None } => i18n("Stopped by a signal"),
        ScriptOutcome::ConnectionFailed => i18n("Connection failed"),
        ScriptOutcome::TimedOut => i18n("Timed out"),
        ScriptOutcome::Cancelled => i18n("Cancelled"),
        ScriptOutcome::NotStarted => i18n("Could not start"),
    }
}

/// Opens the dialog for the connections and groups selected in the sidebar.
pub(super) fn open_dialog(
    window: &adw::ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    let mut selected = sidebar.get_selected_ids();
    if selected.is_empty()
        && let Some(item) = sidebar.get_selected_item()
        && let Ok(id) = Uuid::parse_str(&item.id())
    {
        selected.push(id);
    }

    let Ok(state_ref) = state.try_borrow() else {
        return;
    };
    let connections = state_ref.list_connections_owned();
    let groups = state_ref.list_groups_owned();
    drop(state_ref);
    let targets = remote_script::script_targets(&selected, &connections, &groups);
    if targets.hosts.is_empty() {
        crate::toast::show_toast_on_window(
            window.upcast_ref::<gtk4::Window>(),
            &i18n("No SSH connections are selected"),
            crate::toast::ToastType::Warning,
        );
        return;
    }
    let host_ids: Vec<Uuid> = targets.hosts.iter().map(|c| c.id).collect();
    let host_names: Vec<String> = targets.hosts.iter().map(|c| c.name.clone()).collect();
    let asks_at_connect = targets
        .hosts
        .iter()
        .any(|c| c.password_source == PasswordSource::Prompt);

    let hosts_row = adw::ActionRow::builder()
        .title(i18n("Hosts"))
        .subtitle(host_names.join(", "))
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    if targets.skipped > 0 {
        hosts_row.set_title(&i18n_f(
            "Hosts (not using SSH, left out: {})",
            &[&targets.skipped.to_string()],
        ));
    }
    let path_row = adw::EntryRow::builder().title(i18n("Script File")).build();
    let browse_label = i18n("Choose File");
    let browse = gtk4::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(&browse_label)
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    browse.update_property(&[gtk4::accessible::Property::Label(&browse_label)]);
    path_row.add_suffix(&browse);
    let parallel_row = adw::SpinRow::with_range(1.0, MAX_PARALLEL as f64, 1.0);
    parallel_row.set_title(&i18n("Hosts at Once"));
    parallel_row.set_value(DEFAULT_PARALLEL as f64);
    let timeout_row = adw::SpinRow::with_range(1.0, 1440.0, 1.0);
    timeout_row.set_title(&i18n("Time Limit per Host (minutes)"));
    timeout_row.set_value((DEFAULT_TIMEOUT.as_secs() / 60) as f64);

    let group = adw::PreferencesGroup::builder()
        .description(i18n(
            "Runs with bash on each host as its login user. Nothing is saved on the hosts. A host you have never connected to is refused until its key is known.",
        ))
        .build();
    group.add(&hosts_row);
    group.add(&path_row);
    group.add(&parallel_row);
    group.add(&timeout_row);
    // ponytail: one answer for every host that asks, which suits a fleet sharing
    // an account; make it one row per host if hosts differ in practice.
    let password_row = adw::PasswordEntryRow::builder()
        .title(i18n("Password for Hosts That Ask at Connect"))
        .visible(asks_at_connect)
        .build();
    group.add(&password_row);
    let error_label = gtk4::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .visible(false)
        .build();
    let setup = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    setup.set_margin_top(12);
    setup.set_margin_bottom(12);
    setup.set_margin_start(12);
    setup.set_margin_end(12);
    setup.append(&group);
    setup.append(&error_label);

    let list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    let output_view = gtk4::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk4::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(8)
        .bottom_margin(8)
        .build();
    let results = gtk4::Paned::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .position(280)
        .start_child(
            &gtk4::ScrolledWindow::builder()
                .hscrollbar_policy(gtk4::PolicyType::Never)
                .child(&list)
                .build(),
        )
        .end_child(&gtk4::ScrolledWindow::builder().child(&output_view).build())
        .build();

    let stack = gtk4::Stack::new();
    stack.add_named(&setup, Some("setup"));
    stack.add_named(&results, Some("results"));

    let title = adw::WindowTitle::new(&i18n("Run Local Script"), "");
    let run_button = gtk4::Button::builder()
        .label(i18n("Run"))
        .css_classes(["suggested-action"])
        .build();
    let stop_button = gtk4::Button::builder()
        .label(i18n("Stop"))
        .css_classes(["destructive-action"])
        .visible(false)
        .build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    header.pack_start(&run_button);
    header.pack_start(&stop_button);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    let dialog = adw::Dialog::builder()
        .content_width(820)
        .content_height(540)
        .child(&toolbar)
        .build();

    let view = Rc::new(RunView {
        dialog: dialog.clone(),
        stack,
        title,
        run_button: run_button.clone(),
        stop_button: stop_button.clone(),
        list: list.clone(),
        output_view,
        rows: RefCell::default(),
        output: RefCell::default(),
        selected: Cell::new(0),
        cancel: Arc::new(AtomicBool::new(false)),
        typed_password: RefCell::default(),
        succeeded: Cell::new(0),
        failed: Cell::new(0),
        finished: Cell::new(0),
    });

    stop_button.connect_clicked({
        let view = Rc::clone(&view);
        move |button| {
            view.cancel.store(true, Ordering::Relaxed);
            button.set_sensitive(false);
        }
    });
    list.connect_row_selected({
        let view = Rc::clone(&view);
        move |_, row| {
            let Some(row) = row else { return };
            let index = usize::try_from(row.index()).unwrap_or(0);
            view.selected.set(index);
            let text = view.output.borrow().get(index).cloned().unwrap_or_default();
            view.output_view.buffer().set_text(&text);
        }
    });
    browse.connect_clicked({
        let path_row = path_row.clone();
        let dialog = dialog.clone();
        move |_| {
            let chooser = gtk4::FileDialog::builder()
                .title(i18n("Choose Script"))
                .modal(true)
                .build();
            let parent = dialog.root().and_downcast::<gtk4::Window>();
            let path_row = path_row.clone();
            chooser.open(
                parent.as_ref(),
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        path_row.set_text(&path.to_string_lossy());
                    }
                },
            );
        }
    });
    run_button.connect_clicked({
        let view = Rc::clone(&view);
        let state = state.clone();
        move |_| {
            let path = PathBuf::from(path_row.text().trim());
            let script = match remote_script::load_script(&path) {
                Ok(script) => script,
                Err(error) => {
                    error_label.set_text(&script_error_text(&error));
                    error_label.set_visible(true);
                    return;
                }
            };
            error_label.set_visible(false);
            let typed = password_row.text();
            *view.typed_password.borrow_mut() =
                (!typed.is_empty()).then(|| SecretString::from(typed.to_string()));
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "both spin rows are bounded to small positive ranges"
            )]
            let (parallel, timeout_minutes) =
                (parallel_row.value() as usize, timeout_row.value() as u64);
            let options = RunOptions {
                parallel,
                timeout: Duration::from_secs(timeout_minutes * 60),
                cancel: Arc::clone(&view.cancel),
            };
            confirm_and_start(
                &view,
                &state,
                host_ids.clone(),
                host_names.clone(),
                &path,
                script,
                options,
            );
        }
    });

    dialog.present(Some(window));
}

/// Running code on many machines at once deserves one explicit question.
fn confirm_and_start(
    view: &Rc<RunView>,
    state: &SharedAppState,
    host_ids: Vec<Uuid>,
    host_names: Vec<String>,
    path: &std::path::Path,
    script: Vec<u8>,
    options: RunOptions,
) {
    let file_name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let confirm = adw::AlertDialog::new(
        Some(&i18n_f("Run ‘{}’?", &[&file_name])),
        Some(&i18n_f(
            "It will run on these hosts as their login users, and its changes cannot be undone: {}",
            &[&host_names.join(", ")],
        )),
    );
    confirm.add_response("cancel", &i18n("Cancel"));
    confirm.add_response("run", &i18n("Run"));
    confirm.set_response_appearance("run", adw::ResponseAppearance::Destructive);
    confirm.set_default_response(Some("cancel"));
    confirm.set_close_response("cancel");

    let parent = view.dialog.clone();
    let view = Rc::clone(view);
    let state = state.clone();
    let start = Cell::new(Some((host_ids, host_names, script, options)));
    confirm.connect_response(None, move |_, response| {
        if response == "run"
            && let Some((ids, names, script, options)) = start.take()
        {
            begin_run(&view, &state, ids, &names, script, options);
        }
    });
    confirm.present(Some(&parent));
}

/// Shows the results page and picks each host's jump host.
fn begin_run(
    view: &Rc<RunView>,
    state: &SharedAppState,
    host_ids: Vec<Uuid>,
    host_names: &[String],
    script: Vec<u8>,
    options: RunOptions,
) {
    for name in host_names {
        let status = gtk4::Label::builder().label(i18n("Waiting")).build();
        status.add_css_class("dim-label");
        let row = adw::ActionRow::builder()
            .title(name)
            .use_markup(false)
            .build();
        row.add_suffix(&status);
        view.list.append(&row);
        view.rows.borrow_mut().push(HostRow { status });
        view.output.borrow_mut().push(String::new());
    }
    view.list.select_row(view.list.row_at_index(0).as_ref());
    view.stack.set_visible_child_name("results");
    view.run_button.set_visible(false);
    view.stop_button.set_visible(true);
    view.dialog.set_can_close(false);

    let outstanding = Rc::new(Cell::new(host_ids.len()));
    let next = Rc::new(Cell::new(Some((host_ids.clone(), script, options))));
    let on_probed: Rc<dyn Fn()> = Rc::new({
        let view = Rc::clone(view);
        let state = state.clone();
        move || {
            outstanding.set(outstanding.get() - 1);
            if outstanding.get() == 0
                && let Some((ids, script, options)) = next.take()
            {
                prepare_hosts(&view, &state, ids, script, options);
            }
        }
    });
    for id in host_ids {
        let done = Rc::clone(&on_probed);
        if !start_gateway_probe_quiet(state, id, move || done()) {
            on_probed();
        }
    }
}

/// Works out each host's route, off the main thread where it can block.
fn prepare_hosts(
    view: &Rc<RunView>,
    state: &SharedAppState,
    ids: Vec<Uuid>,
    script: Vec<u8>,
    options: RunOptions,
) {
    let lookups: Vec<(String, u16, Option<String>)> = {
        let state_ref = state.borrow();
        ids.iter()
            .map(|id| {
                let globals = resolve_globals_with_ask(state, *id);
                state_ref.get_connection(*id).map_or_else(
                    || (String::new(), 22, None),
                    |conn| {
                        (
                            substitute_variables(&conn.host, &globals),
                            conn.port,
                            conn.username
                                .as_ref()
                                .map(|u| substitute_variables(u, &globals)),
                        )
                    },
                )
            })
            .collect()
    };

    let view = Rc::clone(view);
    let state = state.clone();
    spawn_blocking_with_callback(
        move || {
            lookups
                .into_iter()
                .map(|(host, port, user)| {
                    rustconn_core::ssh_tunnel::ssh_config_declares_proxy(
                        &host,
                        port,
                        user.as_deref(),
                    )
                })
                .collect::<Vec<bool>>()
        },
        move |declares| build_hosts(view, state, ids, declares, script, options),
    );
}

/// Builds the launch of each host, one per main-loop turn.
///
/// ponytail: `ensure_connection_password` and the bastion lookups read the vault
/// on the main thread, as a reconnect does; one host per turn keeps the window
/// painting between reads. Move the lookups off-thread if a slow vault makes the
/// start of a large run feel stuck.
fn build_hosts(
    view: Rc<RunView>,
    state: SharedAppState,
    ids: Vec<Uuid>,
    declares: Vec<bool>,
    script: Vec<u8>,
    options: RunOptions,
) {
    let mut queue = ids.into_iter().zip(declares).enumerate();
    let mut hosts = Vec::new();
    let mut row_of_host = Vec::new();
    let mut launch = Some((script, options));
    glib::idle_add_local(move || {
        if view.cancel.load(Ordering::Relaxed) {
            for row in view.rows.borrow().iter() {
                row.status.set_text(&i18n("Cancelled"));
            }
            finish(&view);
            return glib::ControlFlow::Break;
        }
        let Some((row, (id, declares_proxy))) = queue.next() else {
            if let Some((script, options)) = launch.take() {
                start_workers(
                    &view,
                    std::mem::take(&mut hosts),
                    std::mem::take(&mut row_of_host),
                    script,
                    options,
                );
            }
            return glib::ControlFlow::Break;
        };
        let connection = state
            .try_borrow()
            .ok()
            .and_then(|s| s.get_connection(id).cloned());
        let Some(connection) = connection else {
            record_outcome(&view, row, ScriptOutcome::NotStarted);
            return glib::ControlFlow::Continue;
        };
        let password = state
            .try_borrow_mut()
            .ok()
            .and_then(|mut s| s.ensure_connection_password(id))
            .or_else(|| {
                (connection.password_source == PasswordSource::Prompt)
                    .then(|| view.typed_password.borrow().clone())
                    .flatten()
            });
        let groups = state.borrow().list_groups_owned();
        let launch_info =
            build_key_setup_launch(&connection, &state, &groups, password, Some(declares_proxy));
        hosts.push(ScriptHost {
            name: connection.name,
            target: launch_info.target,
            credentials: launch_info.credentials,
        });
        row_of_host.push(row);
        glib::ControlFlow::Continue
    });
}

/// Runs the hosts on a worker thread and copies progress to the window.
fn start_workers(
    view: &Rc<RunView>,
    hosts: Vec<ScriptHost>,
    row_of_host: Vec<usize>,
    script: Vec<u8>,
    options: RunOptions,
) {
    view.typed_password.borrow_mut().take();
    if hosts.is_empty() {
        finish(view);
        return;
    }
    let total = hosts.len();
    let (sender, receiver) = mpsc::channel();
    let sink: ScriptEventSink = Arc::new(move |event| {
        let _ = sender.send(event);
    });
    std::thread::spawn(move || {
        remote_script::run_script(&Arc::new(script), &hosts, &options, &sink);
    });

    let view = Rc::clone(view);
    let mut done = 0;
    glib::timeout_add_local(EVENT_POLL, move || {
        for _ in 0..EVENTS_PER_POLL {
            let Ok(event) = receiver.try_recv() else {
                break;
            };
            if matches!(event, ScriptEvent::Finished { .. }) {
                done += 1;
            }
            apply_event(&view, event, &row_of_host);
        }
        if done == total {
            finish(&view);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn apply_event(view: &Rc<RunView>, event: ScriptEvent, row_of_host: &[usize]) {
    match event {
        ScriptEvent::Started { host } => {
            if let Some(row) = view.rows.borrow().get(row_of_host[host]) {
                row.status.set_text(&i18n("Running"));
            }
        }
        ScriptEvent::Output { host, line, .. } => append_output(view, row_of_host[host], &line),
        ScriptEvent::OutputTruncated { host } => {
            append_output(view, row_of_host[host], &i18n("[further output dropped]"));
        }
        ScriptEvent::Finished { host, outcome } => {
            record_outcome(view, row_of_host[host], outcome);
        }
    }
}

fn append_output(view: &Rc<RunView>, row: usize, line: &str) {
    if let Some(text) = view.output.borrow_mut().get_mut(row) {
        text.push_str(line);
        text.push('\n');
    }
    if view.selected.get() == row {
        let buffer = view.output_view.buffer();
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, &format!("{line}\n"));
        view.output_view
            .scroll_to_mark(&buffer.get_insert(), 0.0, false, 0.0, 1.0);
    }
}

fn record_outcome(view: &Rc<RunView>, row: usize, outcome: ScriptOutcome) {
    view.finished.set(view.finished.get() + 1);
    let succeeded = outcome == ScriptOutcome::Succeeded;
    if succeeded {
        view.succeeded.set(view.succeeded.get() + 1);
    } else {
        view.failed.set(view.failed.get() + 1);
    }
    if let Some(host_row) = view.rows.borrow().get(row) {
        host_row.status.set_text(&outcome_text(outcome));
        host_row.status.remove_css_class("dim-label");
        host_row
            .status
            .add_css_class(if succeeded { "success" } else { "error" });
    }
    show_progress(view);
}

fn show_progress(view: &RunView) {
    view.title.set_subtitle(&i18n_f(
        "Succeeded: {}, not succeeded: {}",
        &[
            &view.succeeded.get().to_string(),
            &view.failed.get().to_string(),
        ],
    ));
}

fn finish(view: &RunView) {
    show_progress(view);
    view.stop_button.set_visible(false);
    view.dialog.set_can_close(true);
}
