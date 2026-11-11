//! "Run Local Script…": runs a local script on the selected hosts, bash over SSH
//! or PowerShell over WinRM.
//!
//! The ssh arguments, jump hosts and passwords come from
//! [`build_key_setup_launch`], i.e. from the same code a live session uses; the
//! run itself is [`rustconn_core::remote_script`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
    self, DEFAULT_HOST_TIMEOUT, DEFAULT_PARALLEL, LaunchError, MAX_PARALLEL, RemoteHost,
    RunOptions, ScriptEvent, ScriptEventSink, ScriptFileError, ScriptKind, ScriptOutcome,
    ScriptTransport, WinRmTarget,
};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use super::protocols::{
    resolve_globals_with_ask, ssh_tunnel_params_via_jump, start_gateway_probe_quiet,
    substitute_variables,
};
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
///
/// The status is the row's subtitle, which a screen reader reads with the
/// host's name; a suffix label is not part of what the row announces. The
/// icon repeats it as a shape, so a screen reader skips it.
struct HostRow {
    row: adw::ActionRow,
    icon: gtk4::Image,
}

impl HostRow {
    fn new(name: &str) -> Self {
        let icon = gtk4::Image::builder()
            .accessible_role(gtk4::AccessibleRole::Presentation)
            .visible(false)
            .build();
        let row = adw::ActionRow::builder()
            .title(name)
            .subtitle(i18n("Waiting"))
            .use_markup(false)
            .build();
        row.add_prefix(&icon);
        Self { row, icon }
    }

    /// Sets the status text and its icon; `None` hides the icon.
    fn set_status(&self, text: &str, icon: Option<&str>) {
        self.row.set_subtitle(text);
        self.icon.set_icon_name(icon);
        self.icon.set_visible(icon.is_some());
    }
}

/// The icon of a finished host: a check for success, a warning for a run that
/// was stopped and an error for everything else, so a failed host stands out
/// without relying on colour.
fn outcome_icon(outcome: ScriptOutcome) -> &'static str {
    match outcome {
        ScriptOutcome::Succeeded => "object-select-symbolic",
        ScriptOutcome::TimedOut | ScriptOutcome::Cancelled => "dialog-warning-symbolic",
        ScriptOutcome::ScriptFailed { .. }
        | ScriptOutcome::ConnectionFailed
        | ScriptOutcome::NotStarted => "dialog-error-symbolic",
    }
}

/// The icon of a host whose script is running.
const RUNNING_ICON: &str = "content-loading-symbolic";

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
    /// The answers of hosts set to ask at connect, until the run has started.
    typed_passwords: RefCell<HashMap<Uuid, SecretString>>,
    succeeded: Cell<usize>,
    failed: Cell<usize>,
    finished: Cell<usize>,
    /// Set when the user asked to close a running window: it closes once
    /// every host has stopped.
    close_when_done: Cell<bool>,
}

/// Says why a host was not started; core's text is untranslated.
fn launch_error_text(error: &LaunchError) -> String {
    match error {
        LaunchError::SshNotFound => {
            i18n("ssh was not found. Install OpenSSH to run scripts on SSH hosts.")
        }
        LaunchError::PythonNotFound => i18n(
            "python3 was not found. Install Python 3 with the pywinrm library to run scripts on WinRM hosts.",
        ),
        LaunchError::PyWinRmNotInstalled if rustconn_core::is_sandboxed() => i18n(
            "The pywinrm library is not installed. Install pywinrm in Menu → Components to run scripts on WinRM hosts.",
        ),
        LaunchError::PyWinRmNotInstalled => i18n(
            "The pywinrm library is not installed. Install python3-winrm from your distribution, or run: pip install pywinrm",
        ),
        LaunchError::NoWinRmPassword => {
            i18n("WinRM needs a password, and none is saved or entered for this host")
        }
        LaunchError::InvalidTarget { detail } => {
            i18n_f("This host cannot be reached with ssh: {}", &[detail])
        }
        LaunchError::PasswordNotStaged { detail } => {
            i18n_f("The password could not be handed to ssh: {}", &[detail])
        }
        LaunchError::TunnelFailed { detail } => i18n_f(
            "The SSH tunnel through the jump host could not be opened: {}",
            &[detail],
        ),
        LaunchError::RequestNotEncoded { detail } => {
            i18n_f("The WinRM request could not be prepared: {}", &[detail])
        }
        LaunchError::SpawnFailed { program, detail } => {
            i18n_f("{} could not be started: {}", &[program, detail])
        }
    }
}

/// Shows `error` under the setup form and marks the script field, or clears
/// both with `None`. Every setup error is about the chosen file. The label
/// appears silently, so the error is also read out.
fn show_setup_error(label: &gtk4::Label, path_row: &adw::EntryRow, error: Option<&str>) {
    crate::dialogs::widgets::set_invalid(path_row, error.is_some());
    label.set_visible(error.is_some());
    if let Some(text) = error {
        label.set_text(text);
        path_row
            .upcast_ref::<gtk4::Widget>()
            .announce(text, gtk4::AccessibleAnnouncementPriority::High);
    }
}

fn script_error_text(error: &ScriptFileError) -> String {
    match error {
        ScriptFileError::Unreadable(_) => i18n("The script could not be read"),
        ScriptFileError::NotAFile => i18n("Choose a script file, not a folder"),
        ScriptFileError::Empty => i18n("The script is empty"),
        ScriptFileError::TooLarge { max } => i18n_f(
            "The script is larger than {}",
            &[glib::format_size_full(*max, glib::FormatSizeFlags::IEC_UNITS).as_str()],
        ),
        ScriptFileError::Binary => i18n("This is a binary file, not a script"),
    }
}

/// The typed password of each host set to ask at connect.
///
/// With `shared` (the "Use One Password for All Hosts" switch on) every host
/// gets that one; otherwise each host gets its own field. An empty field gives
/// that host no password, never the one typed for another host.
fn typed_passwords(
    fields: impl Iterator<Item = (Uuid, SecretString)>,
    shared: Option<SecretString>,
) -> HashMap<Uuid, SecretString> {
    let given = |secret: SecretString| (!secret.expose_secret().is_empty()).then_some(secret);
    match shared {
        Some(shared) => {
            let Some(shared) = given(shared) else {
                return HashMap::new();
            };
            fields.map(|(id, _)| (id, shared.clone())).collect()
        }
        None => fields
            .filter_map(|(id, secret)| given(secret).map(|p| (id, p)))
            .collect(),
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
    let bash = remote_script::script_targets(&selected, &connections, &groups, ScriptKind::Bash);
    let powershell =
        remote_script::script_targets(&selected, &connections, &groups, ScriptKind::PowerShell);
    if bash.hosts.is_empty() && powershell.hosts.is_empty() {
        crate::toast::show_toast_on_window(
            window.upcast_ref::<gtk4::Window>(),
            &i18n("No SSH or WinRM connections are selected"),
            crate::toast::ToastType::Warning,
        );
        return;
    }
    let plan = |targets: &remote_script::ScriptTargets<'_>| -> (Vec<Uuid>, Vec<String>) {
        (
            targets.hosts.iter().map(|c| c.id).collect(),
            targets.hosts.iter().map(|c| c.name.clone()).collect(),
        )
    };
    let bash_plan = plan(&bash);
    let powershell_plan = plan(&powershell);

    let all_names: Vec<String> = bash_plan
        .1
        .iter()
        .chain(&powershell_plan.1)
        .cloned()
        .collect();
    let left_out = bash.skipped.saturating_sub(powershell.hosts.len());
    let hosts_row = adw::ActionRow::builder()
        .title(i18n("Hosts"))
        .subtitle(all_names.join(", "))
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    if left_out > 0 {
        hosts_row.set_title(&i18n_f(
            "Hosts (not using SSH or WinRM, left out: {})",
            &[&left_out.to_string()],
        ));
    }
    let asking_hosts: Vec<(Uuid, String)> = bash
        .hosts
        .iter()
        .chain(&powershell.hosts)
        .filter(|c| c.password_source == PasswordSource::Prompt)
        .map(|c| (c.id, c.name.clone()))
        .collect();
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
    timeout_row.set_value((DEFAULT_HOST_TIMEOUT.as_secs() / 60) as f64);

    let group = adw::PreferencesGroup::builder()
        .description(i18n(
            "A .ps1 file runs in PowerShell on WinRM hosts; any other file runs with bash on SSH hosts, as the login user. Nothing is saved on the hosts. An SSH host you have never connected to is refused until its key is known.",
        ))
        .build();
    group.add(&hosts_row);
    group.add(&path_row);
    group.add(&parallel_row);
    group.add(&timeout_row);
    let passwords_group = adw::PreferencesGroup::builder()
        .title(i18n("Passwords"))
        .description(i18n(
            "For hosts set to ask at connect. A host whose field is empty gets no password.",
        ))
        .visible(!asking_hosts.is_empty())
        .build();
    // One answer for every host only when asked for: an empty field used to
    // reuse the one above it, which sent a password to hosts nobody typed it for.
    let same_password_row = adw::SwitchRow::builder()
        .title(i18n("Use One Password for All Hosts"))
        .visible(asking_hosts.len() > 1)
        .build();
    passwords_group.add(&same_password_row);
    let shared_password_row = adw::PasswordEntryRow::builder()
        .title(i18n("Password for All Hosts"))
        .visible(false)
        .build();
    passwords_group.add(&shared_password_row);
    let password_rows: Vec<(Uuid, adw::PasswordEntryRow)> = asking_hosts
        .iter()
        .map(|(id, name)| {
            let row = adw::PasswordEntryRow::builder().title(name).build();
            passwords_group.add(&row);
            (*id, row)
        })
        .collect();
    same_password_row.connect_active_notify({
        let shared_password_row = shared_password_row.clone();
        let host_rows: Vec<adw::PasswordEntryRow> =
            password_rows.iter().map(|(_, row)| row.clone()).collect();
        move |switch| {
            let one = switch.is_active();
            shared_password_row.set_visible(one);
            for row in &host_rows {
                row.set_visible(!one);
            }
        }
    });
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
    setup.append(&passwords_group);
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
    output_view.update_property(&[gtk4::accessible::Property::Label(&i18n("Script Output"))]);
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
    let setup_scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&setup)
        .build();
    stack.add_named(&setup_scroller, Some("setup"));
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
        .title(i18n("Run Local Script"))
        .content_width(820)
        .content_height(540)
        .child(&toolbar)
        .build();
    // On a narrow window the output goes below the host list.
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        500.0,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(
        &results,
        "orientation",
        Some(&gtk4::Orientation::Vertical.to_value()),
    );
    dialog.add_breakpoint(breakpoint);

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
        typed_passwords: RefCell::default(),
        succeeded: Cell::new(0),
        failed: Cell::new(0),
        finished: Cell::new(0),
        close_when_done: Cell::new(false),
    });

    // While a run is going `can_close` is false, so Escape and the close
    // button land here instead of doing nothing.
    dialog.connect_close_attempt({
        let view = Rc::downgrade(&view);
        move |_| {
            if let Some(view) = view.upgrade() {
                confirm_stop(&view);
            }
        }
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
            if let Some(host_row) = view.rows.borrow().get(index) {
                view.output_view
                    .update_property(&[gtk4::accessible::Property::Label(&i18n_f(
                        "Output of {}",
                        &[&host_row.row.title()],
                    ))]);
            }
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
    path_row.connect_changed({
        let error_label = error_label.clone();
        move |row| show_setup_error(&error_label, row, None)
    });
    run_button.connect_clicked({
        let view = Rc::clone(&view);
        let state = state.clone();
        move |_| {
            let path = PathBuf::from(path_row.text().trim());
            let script = match remote_script::load_script(&path) {
                Ok(script) => script,
                Err(error) => {
                    show_setup_error(&error_label, &path_row, Some(&script_error_text(&error)));
                    return;
                }
            };
            let (host_ids, host_names) = match ScriptKind::of_path(&path) {
                ScriptKind::Bash => &bash_plan,
                ScriptKind::PowerShell => &powershell_plan,
            };
            if host_ids.is_empty() {
                show_setup_error(
                    &error_label,
                    &path_row,
                    Some(&i18n(
                        "None of the selected hosts can run this type of script",
                    )),
                );
                return;
            }
            show_setup_error(&error_label, &path_row, None);
            // Each field's text goes straight into a `SecretString`, with no named
            // copy left to drop unzeroed. The GTK entry buffer itself cannot be
            // zeroed from Rust (secrets-guide.md); the fields are emptied once the
            // run is confirmed.
            let shared = same_password_row
                .is_active()
                .then(|| SecretString::from(shared_password_row.text().to_string()));
            *view.typed_passwords.borrow_mut() = typed_passwords(
                password_rows
                    .iter()
                    .map(|(id, row)| (*id, SecretString::from(row.text().to_string()))),
                shared,
            );
            let entries = password_rows
                .iter()
                .map(|(_, row)| row.clone())
                .chain(std::iter::once(shared_password_row.clone()))
                .collect();
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
                entries,
            );
        }
    });

    dialog.present(Some(window));
}

/// Running code on many machines at once deserves one explicit question.
///
/// The typed passwords are already in `view.typed_passwords`. Once the run is
/// confirmed `password_rows` are emptied, so the setup page, which stays alive
/// behind the results, no longer holds them; a cancelled question keeps them for
/// the next try.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site; each argument is a distinct piece of the run"
)]
fn confirm_and_start(
    view: &Rc<RunView>,
    state: &SharedAppState,
    host_ids: Vec<Uuid>,
    host_names: Vec<String>,
    path: &std::path::Path,
    script: Vec<u8>,
    options: RunOptions,
    password_rows: Vec<adw::PasswordEntryRow>,
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
            for row in &password_rows {
                row.set_text("");
            }
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
        let host_row = HostRow::new(name);
        view.list.append(&host_row.row);
        view.rows.borrow_mut().push(host_row);
        view.output.borrow_mut().push(String::new());
    }
    view.list.select_row(view.list.row_at_index(0).as_ref());
    view.stack.set_visible_child_name("results");
    view.run_button.set_visible(false);
    view.stop_button.set_visible(true);
    // Run had focus and is gone; Stop is what the keyboard needs next.
    view.stop_button.grab_focus();
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
                row.set_status(
                    &i18n("Cancelled"),
                    Some(outcome_icon(ScriptOutcome::Cancelled)),
                );
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
            .or_else(|| view.typed_passwords.borrow().get(&id).cloned());
        let transport = if connection.protocol == rustconn_core::models::ProtocolType::WinRm {
            winrm_transport(&state, &connection, password)
        } else {
            let groups = state.borrow().list_groups_owned();
            let launch_info = build_key_setup_launch(
                &connection,
                &state,
                &groups,
                password,
                Some(declares_proxy),
                // Nobody watches a run: a bastion in a generated ProxyCommand
                // must already be known, never accepted on first sight.
                rustconn_core::ssh_tunnel::HopHostKeys::KnownOnly,
            );
            Some(ScriptTransport::Ssh {
                target: launch_info.target,
                credentials: launch_info.credentials,
            })
        };
        let Some(transport) = transport else {
            record_outcome(&view, row, ScriptOutcome::NotStarted);
            return glib::ControlFlow::Continue;
        };
        hosts.push(RemoteHost {
            name: connection.name,
            transport,
        });
        row_of_host.push(row);
        glib::ControlFlow::Continue
    });
}

/// How a WinRM host is reached: its address after variables, and any tunnel.
fn winrm_transport(
    state: &SharedAppState,
    connection: &rustconn_core::Connection,
    password: Option<SecretString>,
) -> Option<ScriptTransport> {
    let globals = resolve_globals_with_ask(state, connection.id);
    let mut resolved = connection.clone();
    resolved.host = substitute_variables(&connection.host, &globals);
    resolved.username = connection
        .username
        .as_ref()
        .map(|user| substitute_variables(user, &globals));
    // Nobody watches a run: the hops past the first refuse an unknown key too.
    let tunnel = ssh_tunnel_params_via_jump(
        state,
        connection,
        resolved.host.clone(),
        resolved.port,
        rustconn_core::ssh_tunnel::HopHostKeys::KnownOnly,
    );
    WinRmTarget::for_connection(&resolved, password, tunnel).map(ScriptTransport::WinRm)
}

/// Runs the hosts on a worker thread and copies progress to the window.
fn start_workers(
    view: &Rc<RunView>,
    hosts: Vec<RemoteHost>,
    row_of_host: Vec<usize>,
    script: Vec<u8>,
    options: RunOptions,
) {
    view.typed_passwords.borrow_mut().clear();
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
                row.set_status(&i18n("Running"), Some(RUNNING_ICON));
            }
        }
        ScriptEvent::Output { host, line, .. } => append_output(view, row_of_host[host], &line),
        ScriptEvent::OutputTruncated { host } => {
            append_output(view, row_of_host[host], &i18n("[further output dropped]"));
        }
        ScriptEvent::NotStarted { host, reason } => {
            append_output(view, row_of_host[host], &launch_error_text(&reason));
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
        host_row.set_status(&outcome_text(outcome), Some(outcome_icon(outcome)));
    }
    show_progress(view);
}

fn progress_text(view: &RunView) -> String {
    i18n_f(
        "Succeeded: {}, not succeeded: {}",
        &[
            &view.succeeded.get().to_string(),
            &view.failed.get().to_string(),
        ],
    )
}

fn show_progress(view: &RunView) {
    view.title.set_subtitle(&progress_text(view));
}

/// Asks whether to stop a run the user tried to close.
fn confirm_stop(view: &Rc<RunView>) {
    // Stop was already pressed: the answer is known, close once it has stopped.
    if view.cancel.load(Ordering::Relaxed) {
        stop_and_close(view);
        return;
    }
    let alert = adw::AlertDialog::new(
        Some(&i18n("Stop the Script?")),
        Some(&i18n(
            "It is stopped on every host still running it, and the window closes. Changes it has already made stay in place.",
        )),
    );
    alert.add_response("cancel", &i18n("Keep Running"));
    alert.add_response("stop", &i18n("Stop and Close"));
    alert.set_response_appearance("stop", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    alert.set_close_response("cancel");
    alert.connect_response(None, {
        let view = Rc::clone(view);
        move |_, response| {
            if response == "stop" {
                stop_and_close(&view);
            }
        }
    });
    alert.present(Some(&view.dialog));
}

/// Stops the run and closes the window, at once if the run has already ended.
fn stop_and_close(view: &RunView) {
    view.cancel.store(true, Ordering::Relaxed);
    view.stop_button.set_sensitive(false);
    // The run may have ended while the question was open.
    if view.dialog.can_close() {
        view.dialog.close();
    } else {
        view.close_when_done.set(true);
    }
}

fn finish(view: &RunView) {
    show_progress(view);
    view.stop_button.set_visible(false);
    view.dialog.set_can_close(true);
    if view.close_when_done.get() {
        view.dialog.close();
        return;
    }
    // Stop had focus and is gone; the results are what is left to read.
    view.list.grab_focus();
    // The header subtitle changes silently; say once that the run is over.
    view.list.announce(
        &i18n_f("Script finished. {}", &[&progress_text(view)]),
        gtk4::AccessibleAnnouncementPriority::Medium,
    );
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret, SecretString};
    use uuid::Uuid;

    use super::typed_passwords;

    fn secret(text: &str) -> SecretString {
        SecretString::from(text.to_string())
    }

    #[test]
    fn an_empty_field_gives_no_password() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let typed = typed_passwords([(a, secret("first")), (b, secret(""))].into_iter(), None);
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[&a].expose_secret(), "first");
        assert!(!typed.contains_key(&b), "b must not reuse a's password");
    }

    #[test]
    fn one_password_goes_to_every_host() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let typed = typed_passwords(
            [(a, secret("ignored")), (b, secret(""))].into_iter(),
            Some(secret("shared")),
        );
        assert_eq!(typed[&a].expose_secret(), "shared");
        assert_eq!(typed[&b].expose_secret(), "shared");
        let none = typed_passwords([(a, secret("ignored"))].into_iter(), Some(secret("")));
        assert!(none.is_empty(), "an empty shared field sends nothing");
    }

    #[test]
    fn every_outcome_has_an_icon_and_failures_differ_from_success() {
        use rustconn_core::remote_script::ScriptOutcome;

        let success = super::outcome_icon(ScriptOutcome::Succeeded);
        for outcome in [
            ScriptOutcome::ScriptFailed { code: Some(1) },
            ScriptOutcome::ScriptFailed { code: None },
            ScriptOutcome::ConnectionFailed,
            ScriptOutcome::TimedOut,
            ScriptOutcome::Cancelled,
            ScriptOutcome::NotStarted,
        ] {
            let icon = super::outcome_icon(outcome);
            assert!(icon.ends_with("-symbolic"), "{outcome:?}: {icon}");
            assert_ne!(icon, success, "{outcome:?}");
            assert_ne!(icon, super::RUNNING_ICON, "{outcome:?}");
        }
        assert_eq!(
            super::outcome_icon(ScriptOutcome::ConnectionFailed),
            "dialog-error-symbolic"
        );
        assert_eq!(
            super::outcome_icon(ScriptOutcome::TimedOut),
            "dialog-warning-symbolic"
        );
    }
}
