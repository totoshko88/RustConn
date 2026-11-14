//! Network Scan dialog
//!
//! A quick scan of an IPv4 range for hosts answering on the ports RustConn
//! connects to, reachable from the Tools menu. The range is typed the way it is
//! usually written — a network with its mask, or a first and a last address —
//! and every host found can be turned into a connection with one click.
//!
//! The scan itself is [`rustconn_core::network_scan`]; this file only drives it
//! from a worker thread and shows what comes back.

use std::cell::RefCell;
use std::net::Ipv4Addr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Orientation, glib};
use libadwaita as adw;
use rustconn_core::network_scan::{
    self, HostResult, NetworkScanError, ScanOptions, ScanPort, ScanProgress, ScanTarget,
};

use crate::dialogs::widgets::dialog_pack_action;
use crate::i18n::{i18n, i18n_f};

/// Called when the user asks for a connection to a found host. Returns whether
/// the connection was created.
type AddConnectionCallback = Rc<dyn Fn(Ipv4Addr, ScanPort) -> bool>;

/// What a worker thread reports back to the dialog.
enum ScanMessage {
    Progress(ScanProgress),
    /// The scan could not start at all.
    Failed,
}

/// The widgets a running scan updates.
#[derive(Clone)]
struct ScanView {
    scan_button: Button,
    range_row: adw::EntryRow,
    status_label: gtk4::Label,
    progress_bar: gtk4::ProgressBar,
    results_group: adw::PreferencesGroup,
    results_list: gtk4::ListBox,
    /// Addresses already listed, kept sorted so a new row can be inserted at
    /// its place: hosts answer out of order.
    listed: Rc<RefCell<Vec<Ipv4Addr>>>,
    /// Set to stop the running scan; replaced for every new scan.
    cancel: Rc<RefCell<Arc<AtomicBool>>>,
    /// Whether a scan is running, which is what the header button toggles on.
    running: Rc<std::cell::Cell<bool>>,
    on_add: AddConnectionCallback,
}

/// Standalone Network Scan dialog
pub struct NetworkScanDialog {
    dialog: adw::Dialog,
}

impl NetworkScanDialog {
    /// Creates the dialog. `on_add` creates a connection for a found host.
    #[must_use]
    pub fn new(on_add: impl Fn(Ipv4Addr, ScanPort) -> bool + 'static) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Network Scan"))
            .content_width(520)
            .content_height(560)
            .build();

        let header = adw::HeaderBar::new();
        let scan_button = Button::with_label(&i18n("Scan"));
        scan_button.add_css_class("suggested-action");
        dialog_pack_action(&header, &scan_button);

        let content = GtkBox::new(Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let range_group = adw::PreferencesGroup::builder()
            .title(i18n("Addresses"))
            .description(i18n(
                "A network with its mask (192.168.1.0/24 or 192.168.1.0/255.255.255.0), or a first and a last address (192.168.1.10-192.168.1.50).",
            ))
            .build();
        let range_row = adw::EntryRow::builder()
            .title(i18n("Network or address range"))
            .build();
        if let Some(guess) = network_scan::local_network_guess() {
            range_row.set_text(&guess);
        }
        range_group.add(&range_row);
        content.append(&range_group);

        let status_label = gtk4::Label::new(Some(&i18n(
            "Looks for hosts answering on the SSH, RDP, VNC, Telnet, HTTP and HTTPS ports.",
        )));
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_wrap(true);
        status_label.set_xalign(0.0);
        status_label.add_css_class("dim-label");
        content.append(&status_label);

        let progress_bar = gtk4::ProgressBar::new();
        progress_bar.set_visible(false);
        progress_bar.update_property(&[gtk4::accessible::Property::Label(&i18n("Scan progress"))]);
        content.append(&progress_bar);

        let results_group = adw::PreferencesGroup::builder()
            .title(i18n("Hosts Found"))
            .visible(false)
            .build();
        let results_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        results_group.add(&results_list);
        content.append(&results_group);

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .child(&content)
            .build();
        let scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .child(&clamp)
            .build();

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&scroller));
        dialog.set_child(Some(&toolbar_view));

        let view = ScanView {
            scan_button: scan_button.clone(),
            range_row: range_row.clone(),
            status_label,
            progress_bar,
            results_group,
            results_list,
            listed: Rc::new(RefCell::new(Vec::new())),
            cancel: Rc::new(RefCell::new(Arc::new(AtomicBool::new(false)))),
            running: Rc::new(std::cell::Cell::new(false)),
            on_add: Rc::new(on_add),
        };

        // The header button starts a scan, and stops the one that is running.
        let view_for_button = view.clone();
        scan_button.connect_clicked(move |_| {
            if view_for_button.running.get() {
                view_for_button
                    .cancel
                    .borrow()
                    .store(true, Ordering::Relaxed);
            } else {
                view_for_button.start();
            }
        });
        // Enter in the range field starts the scan too.
        let view_for_entry = view.clone();
        range_row.connect_entry_activated(move |_| {
            if !view_for_entry.running.get() {
                view_for_entry.start();
            }
        });
        // Closing the dialog must not leave a scan running behind it.
        let cancel_on_close = Rc::clone(&view.cancel);
        dialog.connect_closed(move |_| {
            cancel_on_close.borrow().store(true, Ordering::Relaxed);
        });

        Self { dialog }
    }

    /// Presents the dialog
    pub fn present(&self, parent: &impl IsA<gtk4::Widget>) {
        self.dialog.present(Some(parent));
    }
}

/// Explains a rejected range in the user's language.
fn describe_error(error: &NetworkScanError) -> String {
    match error {
        NetworkScanError::Empty => i18n("Enter a network or an address range."),
        NetworkScanError::InvalidAddress(text) => i18n_f("“{}” is not an IPv4 address.", &[text]),
        NetworkScanError::InvalidMask(text) => i18n_f(
            "“{}” is not a prefix length (0 to 32) or a netmask.",
            &[text],
        ),
        NetworkScanError::ReversedRange { .. } => {
            i18n("The last address must not be before the first one.")
        }
        NetworkScanError::TooLarge { count, max } => i18n_f(
            "This range covers {} addresses; a scan is limited to {}.",
            &[&count.to_string(), &max.to_string()],
        ),
    }
}

/// Lists a host's open ports as "SSH (22), HTTPS (443)".
fn describe_ports(host: &HostResult) -> String {
    if host.open_ports.is_empty() {
        return i18n("Answers, but none of the scanned ports is open");
    }
    host.open_ports
        .iter()
        .map(|&port| match network_scan::known_port(port) {
            Some(known) => format!("{} ({port})", known.service),
            None => port.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl ScanView {
    /// Shows a status line, as an error or as plain information.
    fn set_status(&self, text: &str, is_error: bool) {
        self.status_label.set_text(text);
        if is_error {
            self.status_label.remove_css_class("dim-label");
            self.status_label.add_css_class("error");
        } else {
            self.status_label.remove_css_class("error");
            self.status_label.add_css_class("dim-label");
        }
    }

    /// Validates the range and starts scanning it.
    fn start(&self) {
        let target = match ScanTarget::parse(&self.range_row.text()) {
            Ok(target) => target,
            Err(error) => {
                tracing::debug!(%error, "network scan: range rejected");
                self.range_row.add_css_class("error");
                self.set_status(&describe_error(&error), true);
                return;
            }
        };
        self.range_row.remove_css_class("error");

        // A fresh list for every scan.
        self.results_list.remove_all();
        self.listed.borrow_mut().clear();
        self.results_group.set_visible(false);
        self.progress_bar.set_fraction(0.0);
        self.progress_bar.set_visible(true);
        self.running.set(true);
        self.scan_button.set_label(&i18n("Stop"));
        self.scan_button.remove_css_class("suggested-action");
        self.set_status(
            &i18n_f(
                "Scanning {} to {}…",
                &[&target.first().to_string(), &target.last().to_string()],
            ),
            false,
        );

        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = Arc::clone(&cancel);
        tracing::info!(
            first = %target.first(),
            last = %target.last(),
            hosts = target.host_count(),
            "network scan started"
        );

        let (sender, receiver) = async_channel::unbounded::<ScanMessage>();
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let result =
                network_scan::scan_blocking(&target, &ScanOptions::default(), &cancel, |step| {
                    // A closed channel means the dialog is gone; the cancel
                    // flag set on close ends the scan on its own.
                    let _ = progress_sender.try_send(ScanMessage::Progress(step));
                });
            if let Err(error) = result {
                tracing::error!(%error, "network scan could not start");
                let _ = sender.try_send(ScanMessage::Failed);
            }
        });

        let view = self.clone();
        glib::spawn_future_local(async move {
            let mut failed = false;
            let mut scanned = 0;
            while let Ok(message) = receiver.recv().await {
                match message {
                    ScanMessage::Progress(step) => {
                        scanned = step.done;
                        view.progress_bar
                            .set_fraction(crate::utils::progress_fraction(
                                u64::from(step.done),
                                u64::from(step.total),
                            ));
                        if let Some(host) = step.host {
                            view.add_host(&host);
                        }
                    }
                    ScanMessage::Failed => failed = true,
                }
            }
            // The channel closes when the worker thread ends: finished,
            // stopped or failed.
            view.finish(scanned, failed);
        });
    }

    /// Puts the dialog back in its idle state and sums the scan up.
    fn finish(&self, scanned: u32, failed: bool) {
        self.running.set(false);
        self.progress_bar.set_visible(false);
        self.scan_button.set_label(&i18n("Scan"));
        self.scan_button.add_css_class("suggested-action");

        if failed {
            self.set_status(&i18n("The scan could not be started."), true);
            return;
        }
        let found = self.listed.borrow().len();
        let stopped = self.cancel.borrow().load(Ordering::Relaxed);
        let summary = if stopped {
            i18n_f(
                "Stopped after {} addresses — {} hosts found.",
                &[&scanned.to_string(), &found.to_string()],
            )
        } else if found == 0 {
            i18n_f(
                "No host answered among {} addresses. A host whose firewall drops every scanned port stays invisible.",
                &[&scanned.to_string()],
            )
        } else {
            i18n_f(
                "{} addresses scanned — {} hosts found.",
                &[&scanned.to_string(), &found.to_string()],
            )
        };
        self.set_status(&summary, false);
    }

    /// Adds a row for a host that answered, keeping the list in address order.
    fn add_host(&self, host: &HostResult) {
        let row = adw::ActionRow::builder()
            .title(host.address.to_string())
            .subtitle(describe_ports(host))
            .build();

        // One button per open port RustConn can connect to.
        for known in host
            .open_ports
            .iter()
            .filter_map(|&port| network_scan::known_port(port))
        {
            if known.protocol.is_none() {
                continue;
            }
            let button = Button::with_label(known.service);
            button.set_valign(gtk4::Align::Center);
            let tooltip = i18n_f("Add a {} connection to this host", &[known.service]);
            button.set_tooltip_text(Some(&tooltip));
            button.update_property(&[gtk4::accessible::Property::Label(&tooltip)]);
            let on_add = Rc::clone(&self.on_add);
            let address = host.address;
            button.connect_clicked(move |button| {
                if on_add(address, known) {
                    // Done once: a second click would add a duplicate.
                    button.set_sensitive(false);
                    button.set_icon_name("object-select-symbolic");
                }
            });
            row.add_suffix(&button);
        }

        let mut listed = self.listed.borrow_mut();
        let position = listed.partition_point(|address| *address < host.address);
        listed.insert(position, host.address);
        self.results_list
            .insert(&row, i32::try_from(position).unwrap_or(-1));
        self.results_group.set_visible(true);
    }
}
