//! Reusable widget builders for dialogs
//!
//! This module provides builder patterns for common libadwaita widgets used
//! across dialogs, reducing code duplication and ensuring consistent
//! styling following GNOME HIG.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, CheckButton, DropDown, Entry, SpinButton, StringList};
use libadwaita as adw;

use crate::i18n::{i18n, i18n_f};

/// Creates a standard dialog header bar following GNOME HIG.
///
/// Returns `(header_bar, end_button)`. The action button is placed at the end
/// with the `suggested-action` CSS class.
///
/// The native window controls stay on the **start** side: for an `adw::Dialog`
/// that renders the close (✕) button. Escape closing a dialog is discoverable
/// only to someone who already knows the shortcut; GNOME HIG expects a visible
/// close affordance in the header, and hiding both sides left dialogs — the RDP
/// password prompt among them — with no on-screen way out. The end-side buttons
/// stay hidden so the primary action is the only button packed there.
///
/// # Arguments
///
/// * `end_label` - Label for the end (action) button.
#[must_use]
pub fn dialog_header(end_label: &str) -> (adw::HeaderBar, Button) {
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(true);

    let end_btn = Button::builder()
        .label(i18n(end_label))
        .css_classes(["suggested-action"])
        .build();

    header.pack_end(&end_btn);

    (header, end_btn)
}

/// Packs an action button into a dialog header bar.  On macOS uses `pack_end`
/// (right) to keep action buttons away from the left-side close button; on
/// other platforms `pack_start` (left, GNOME HIG).
pub fn dialog_pack_action(header: &adw::HeaderBar, button: &impl IsA<gtk4::Widget>) {
    #[cfg(target_os = "macos")]
    header.pack_end(button);
    #[cfg(not(target_os = "macos"))]
    header.pack_start(button);
}

/// Whether a Jump Host dropdown position names a saved connection rather than
/// "(None)" (index 0 of every Jump Host dropdown) or no selection.
#[must_use]
pub const fn names_jump_host(selected: u32) -> bool {
    selected != 0 && selected != gtk4::INVALID_LIST_POSITION
}

/// Editor for an ordered list of fallback jump hosts.
///
/// Shared by the connection and group editors, which store the same list.
#[derive(Clone)]
pub struct FallbackJumpHostsEditor {
    inner: std::rc::Rc<FallbackJumpHostsInner>,
}

struct FallbackJumpHostsInner {
    expander: adw::ExpanderRow,
    add_row: adw::ActionRow,
    add_dropdown: DropDown,
    add_button: Button,
    /// Every saved connection that can act as a gateway, as `(id, label)`.
    options: std::cell::RefCell<Vec<(uuid::Uuid, String)>>,
    /// The connection being edited, which cannot be its own gateway.
    owner: std::cell::Cell<Option<uuid::Uuid>>,
    selected: std::cell::RefCell<Vec<uuid::Uuid>>,
    /// What each entry of `add_dropdown` adds, parallel to its model.
    addable: std::cell::RefCell<Vec<uuid::Uuid>>,
    rows: std::cell::RefCell<Vec<FallbackRow>>,
}

/// One listed fallback jump host and its buttons.
struct FallbackRow {
    row: adw::ActionRow,
    up: Button,
    down: Button,
    remove: Button,
}

impl FallbackRow {
    fn button(&self, action: RowAction) -> &Button {
        match action {
            RowAction::Up => &self.up,
            RowAction::Down => &self.down,
            RowAction::Remove => &self.remove,
        }
    }
}

/// A row button's action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    Up,
    Down,
    Remove,
}

/// The edit a rebuild follows, which decides where keyboard focus goes.
///
/// A rebuild replaces every row, so the focused button is destroyed (or, for
/// Add, unparented) and the next Tab would start at the top of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refocus {
    /// The list was loaded, not edited: leave focus where it is.
    Keep,
    /// `action` was clicked on the row that was at `index`.
    Row { index: usize, action: RowAction },
    /// An entry was added.
    Added,
}

/// The widget that receives focus after a rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    Unchanged,
    Button { index: usize, action: RowAction },
    AddDropdown,
}

impl Refocus {
    /// Where focus goes in the rebuilt list of `len` entries; `can_add` is
    /// whether the Add row is still sensitive.
    fn target(self, len: usize, can_add: bool) -> FocusTarget {
        let fallback = if can_add {
            FocusTarget::AddDropdown
        } else {
            FocusTarget::Unchanged
        };
        let Some(last) = len.checked_sub(1) else {
            return match self {
                Self::Keep => FocusTarget::Unchanged,
                _ => fallback,
            };
        };
        match self {
            Self::Keep => FocusTarget::Unchanged,
            Self::Added if can_add => FocusTarget::AddDropdown,
            // Nothing left to add: stay on the entry just added.
            Self::Added => FocusTarget::Button {
                index: last,
                action: RowAction::Remove,
            },
            // The next entry slides into place, so Remove can be pressed again.
            Self::Row {
                index,
                action: RowAction::Remove,
            } => FocusTarget::Button {
                index: index.min(last),
                action: RowAction::Remove,
            },
            // Follow the moved entry. At the top or bottom the arrow just used
            // is insensitive, so take the other one; a single entry has neither.
            Self::Row { index, action } => {
                let at = if action == RowAction::Up {
                    index.saturating_sub(1)
                } else {
                    index + 1
                }
                .min(last);
                let can_go = |action| match action {
                    RowAction::Up => at > 0,
                    RowAction::Down => at < last,
                    RowAction::Remove => true,
                };
                let other = if action == RowAction::Up {
                    RowAction::Down
                } else {
                    RowAction::Up
                };
                let action = [action, other, RowAction::Remove]
                    .into_iter()
                    .find(|action| can_go(*action))
                    .unwrap_or(RowAction::Remove);
                FocusTarget::Button { index: at, action }
            }
        }
    }
}

/// Applies `action` to entry `id` wherever it is now, and returns the
/// position it had before, or `None` when it is no longer in the list.
///
/// A move at the top or bottom leaves the list as it is.
fn apply_row_action(
    selected: &mut Vec<uuid::Uuid>,
    id: uuid::Uuid,
    action: RowAction,
) -> Option<usize> {
    let index = selected.iter().position(|entry| *entry == id)?;
    let other = match action {
        RowAction::Up => index.checked_sub(1),
        RowAction::Down => Some(index + 1).filter(|other| *other < selected.len()),
        RowAction::Remove => {
            selected.remove(index);
            return Some(index);
        }
    };
    if let Some(other) = other {
        selected.swap(index, other);
    }
    Some(index)
}

impl FallbackJumpHostsEditor {
    /// Creates an empty editor.
    #[must_use]
    pub fn new() -> Self {
        let expander = adw::ExpanderRow::builder()
            .title(i18n("Fallback Jump Hosts"))
            .subtitle(i18n("Tried in order when the Jump Host is unreachable"))
            .build();

        let add_dropdown = DropDown::builder().valign(gtk4::Align::Center).build();
        enable_string_search(&add_dropdown);
        add_dropdown.set_size_request(200, -1);
        add_dropdown.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Fallback Jump Host to Add",
        ))]);
        let add_label = i18n("Add Fallback Jump Host");
        let add_button = Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(&add_label)
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        add_button.update_property(&[gtk4::accessible::Property::Label(&add_label)]);
        let add_row = adw::ActionRow::builder().title(&add_label).build();
        add_row.add_suffix(&add_dropdown);
        add_row.add_suffix(&add_button);

        let editor = Self {
            inner: std::rc::Rc::new(FallbackJumpHostsInner {
                expander,
                add_row,
                add_dropdown,
                add_button,
                options: std::cell::RefCell::default(),
                owner: std::cell::Cell::default(),
                selected: std::cell::RefCell::default(),
                addable: std::cell::RefCell::default(),
                rows: std::cell::RefCell::default(),
            }),
        };
        let weak = std::rc::Rc::downgrade(&editor.inner);
        editor.inner.add_button.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            let index = inner.add_dropdown.selected() as usize;
            let chosen = inner.addable.borrow().get(index).copied();
            if let Some(id) = chosen {
                inner.selected.borrow_mut().push(id);
                FallbackJumpHostsInner::rebuild(&inner, Refocus::Added);
            }
        });
        FallbackJumpHostsInner::rebuild(&editor.inner, Refocus::Keep);
        editor
    }

    /// The row to place in a preferences group or another expander.
    #[must_use]
    pub fn widget(&self) -> &adw::ExpanderRow {
        &self.inner.expander
    }

    /// Sets the connections that can be picked, as `(id, label)`.
    pub fn set_options(&self, options: Vec<(uuid::Uuid, String)>) {
        *self.inner.options.borrow_mut() = options;
        FallbackJumpHostsInner::rebuild(&self.inner, Refocus::Keep);
    }

    /// Loads the stored list; `owner` is the connection being edited, if any.
    pub fn set_value(&self, owner: Option<uuid::Uuid>, selected: &[uuid::Uuid]) {
        self.inner.owner.set(owner);
        *self.inner.selected.borrow_mut() = selected.to_vec();
        FallbackJumpHostsInner::rebuild(&self.inner, Refocus::Keep);
    }

    /// Returns the list in failover order.
    #[must_use]
    pub fn value(&self) -> Vec<uuid::Uuid> {
        self.inner.selected.borrow().clone()
    }
}

impl Default for FallbackJumpHostsEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl FallbackJumpHostsInner {
    fn label_of(&self, id: uuid::Uuid) -> String {
        self.options
            .borrow()
            .iter()
            .find(|(option, _)| *option == id)
            .map_or_else(|| i18n("(unknown connection)"), |(_, label)| label.clone())
    }

    fn rebuild(this: &std::rc::Rc<Self>, refocus: Refocus) {
        for entry in this.rows.borrow_mut().drain(..) {
            this.expander.remove(&entry.row);
        }
        this.expander.remove(&this.add_row);

        let selected = this.selected.borrow().clone();
        for (index, id) in selected.iter().enumerate() {
            let label = this.label_of(*id);
            let row = adw::ActionRow::builder()
                .title(&label)
                .use_markup(false)
                .build();
            // The tooltip sits next to the host's name; a screen reader that
            // lands on the button hears only the accessible label, so it
            // names the host.
            let up = Self::row_button(
                this,
                "go-up-symbolic",
                &i18n("Move Up"),
                &i18n_f("Move {} up", &[&label]),
                *id,
                RowAction::Up,
            );
            up.set_sensitive(index > 0);
            let down = Self::row_button(
                this,
                "go-down-symbolic",
                &i18n("Move Down"),
                &i18n_f("Move {} down", &[&label]),
                *id,
                RowAction::Down,
            );
            down.set_sensitive(index + 1 < selected.len());
            let remove = Self::row_button(
                this,
                "list-remove-symbolic",
                &i18n("Remove"),
                &i18n_f("Remove {}", &[&label]),
                *id,
                RowAction::Remove,
            );
            row.add_suffix(&up);
            row.add_suffix(&down);
            row.add_suffix(&remove);
            this.expander.add_row(&row);
            this.rows.borrow_mut().push(FallbackRow {
                row,
                up,
                down,
                remove,
            });
        }

        let addable: Vec<(uuid::Uuid, String)> = this
            .options
            .borrow()
            .iter()
            .filter(|(id, _)| !selected.contains(id) && this.owner.get() != Some(*id))
            .cloned()
            .collect();
        let labels: Vec<&str> = addable.iter().map(|(_, label)| label.as_str()).collect();
        this.add_dropdown.set_model(Some(&StringList::new(&labels)));
        this.add_row.set_sensitive(!addable.is_empty());
        *this.addable.borrow_mut() = addable.into_iter().map(|(id, _)| id).collect();
        this.expander.add_row(&this.add_row);

        match refocus.target(selected.len(), this.add_row.is_sensitive()) {
            FocusTarget::Unchanged => {}
            FocusTarget::AddDropdown => {
                this.add_dropdown.grab_focus();
            }
            FocusTarget::Button { index, action } => {
                if let Some(entry) = this.rows.borrow().get(index) {
                    entry.button(action).grab_focus();
                }
            }
        }
    }

    /// Builds a button that applies `action` to entry `id`.
    fn row_button(
        this: &std::rc::Rc<Self>,
        icon: &str,
        tooltip: &str,
        accessible_label: &str,
        id: uuid::Uuid,
        action: RowAction,
    ) -> Button {
        let button = Button::builder()
            .icon_name(icon)
            .tooltip_text(tooltip)
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        button.update_property(&[gtk4::accessible::Property::Label(accessible_label)]);
        let weak = std::rc::Rc::downgrade(this);
        button.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            // A second click can land before the idle rebuild. The button
            // names its entry, not a position, so it still acts on that entry.
            let applied = apply_row_action(&mut inner.selected.borrow_mut(), id, action);
            let Some(index) = applied else { return };
            // Later than this click: rebuilding removes the button being clicked.
            let refocus = Refocus::Row { index, action };
            gtk4::glib::idle_add_local_once(move || Self::rebuild(&inner, refocus));
        });
        button
    }
}

/// Common label strings used across dialogs
pub mod labels {
    use crate::i18n::i18n;

    /// Label for root group in group dropdown
    pub fn root_group() -> String {
        i18n("(Root)")
    }

    /// Label for no selection in dropdowns
    pub fn none_label() -> String {
        i18n("(None)")
    }

    /// Label when no SSH keys are loaded from agent
    pub fn no_keys_loaded() -> String {
        i18n("(No keys loaded)")
    }
}

/// Builder for creating `adw::ActionRow` with a `CheckButton` suffix.
///
/// # Example
/// ```ignore
/// let (row, checkbox) = CheckboxRowBuilder::new("Enable Feature")
///     .subtitle("Description of the feature")
///     .active(true)
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct CheckboxRowBuilder {
    title: String,
    subtitle: Option<String>,
    active: bool,
}

impl CheckboxRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            active: false,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the initial active state of the checkbox.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Builds the `ActionRow` and `CheckButton`.
    ///
    /// Returns a tuple of (row, checkbox) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, CheckButton) {
        let checkbox = CheckButton::new();
        checkbox.set_active(self.active);

        let mut row_builder = adw::ActionRow::builder()
            .title(i18n(&self.title))
            .activatable_widget(&checkbox);

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(i18n(subtitle));
        }

        let row = row_builder.build();
        row.add_suffix(&checkbox);

        (row, checkbox)
    }
}

/// Builder for creating `adw::ActionRow` with an `Entry` suffix.
///
/// # i18n
///
/// `title`, `subtitle` and `placeholder` are passed through to the widget
/// **verbatim** — callers MUST wrap user-facing strings in `i18n()` /
/// `i18n_f()` at the call site so `xgettext` extracts them into the POT.
///
/// # Example
/// ```ignore
/// let (row, entry) = EntryRowBuilder::new(&i18n("Hostname"))
///     .subtitle(&i18n("Server address"))
///     .placeholder(&i18n("example.com"))
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct EntryRowBuilder {
    title: String,
    subtitle: Option<String>,
    placeholder: Option<String>,
    text: Option<String>,
}

impl EntryRowBuilder {
    /// Creates a new builder with the given **already-translated** title.
    ///
    /// Pass `&i18n("...")` so `xgettext` picks up the literal.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            placeholder: None,
            text: None,
        }
    }

    /// Sets the **already-translated** subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the **already-translated** placeholder text for the entry.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Sets the initial text value (raw, not translated).
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Builds the `ActionRow` and `Entry`.
    ///
    /// Returns a tuple of (row, entry) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, Entry) {
        let mut entry_builder = Entry::builder().hexpand(true).valign(gtk4::Align::Center);

        if let Some(placeholder) = &self.placeholder {
            entry_builder = entry_builder.placeholder_text(placeholder);
        }

        let entry = entry_builder.build();

        if let Some(text) = &self.text {
            entry.set_text(text);
        }

        // title / subtitle are expected to be already translated by the caller
        // (see struct doc comment).
        let mut row_builder = adw::ActionRow::builder().title(&self.title);

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(subtitle);
        }

        let row = row_builder.build();
        row.add_suffix(&entry);

        (row, entry)
    }
}

/// Builder for creating `adw::ActionRow` with a `SpinButton` suffix.
///
/// # Example
/// ```ignore
/// let (row, spin) = SpinRowBuilder::new("Port")
///     .subtitle("Connection port")
///     .range(1.0, 65535.0)
///     .value(22.0)
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct SpinRowBuilder {
    title: String,
    subtitle: Option<String>,
    min: f64,
    max: f64,
    step: f64,
    value: f64,
    digits: u32,
}

impl SpinRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            value: 0.0,
            digits: 0,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the range (min, max) for the spin button.
    #[must_use]
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// Sets the step increment.
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// Sets the initial value.
    #[must_use]
    pub fn value(mut self, value: f64) -> Self {
        self.value = value;
        self
    }

    /// Sets the number of decimal digits to display.
    #[must_use]
    pub fn digits(mut self, digits: u32) -> Self {
        self.digits = digits;
        self
    }

    /// Builds the `ActionRow` and `SpinButton`.
    ///
    /// Returns a tuple of (row, spin_button) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, SpinButton) {
        let adjustment =
            gtk4::Adjustment::new(self.value, self.min, self.max, self.step, 10.0, 0.0);

        let spin = SpinButton::builder()
            .adjustment(&adjustment)
            .digits(self.digits)
            .valign(gtk4::Align::Center)
            .build();

        let mut row_builder = adw::ActionRow::builder().title(i18n(&self.title));

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(i18n(subtitle));
        }

        let row = row_builder.build();
        row.add_suffix(&spin);

        (row, spin)
    }
}

/// Makes a `StringList`-backed [`DropDown`] type-to-search actually work.
///
/// `DropDown::set_enable_search(true)` shows a search entry, but it filters on
/// nothing unless the dropdown has an expression that yields the string to match
/// against. A dropdown created with `Expression::NONE` therefore shows a dead
/// search box (the jump-host pickers had exactly this bug). This installs a
/// property expression reading the `string` of each `StringObject` and turns
/// search on, so typing filters a long connection list to the matching entries.
///
/// Matching is case-insensitive **substring**, not GTK's default prefix: group
/// pickers list full paths (`Import/Dev/AimSmarter/RubyStudio`), and a prefix
/// match never finds `RubyStudio` there.
///
/// Call it on any dropdown whose model is a [`StringList`] and whose list can
/// grow long — the jump-host / tunnel pickers, where a user may have hundreds of
/// connections, and every group picker.
pub fn enable_string_search(dropdown: &DropDown) {
    dropdown.set_expression(Some(string_object_expression()));
    dropdown.set_enable_search(true);
    // `search-match-mode` is GTK 4.12; the crate's floor is `v4_14`.
    dropdown.set_search_match_mode(gtk4::StringFilterMatchMode::Substring);
}
/// [`enable_string_search`] for a `StringList`-backed [`adw::ComboRow`].
///
/// A ComboRow needs the same expression before `set_enable_search` filters
/// anything. Substring matching needs `ComboRow::set_search_match_mode`,
/// which is libadwaita 1.6, so it is behind `adw-1-6`.
// ponytail: a baseline (libadwaita 1.5) build keeps the ComboRow's prefix
// match, so an indented `└ name` label is not found by typing `name`; drop the
// cfg when `adw-1-6` is retired.
pub fn enable_combo_row_search(row: &adw::ComboRow) {
    row.set_expression(Some(string_object_expression()));
    row.set_enable_search(true);
    #[cfg(feature = "adw-1-6")]
    row.set_search_match_mode(gtk4::StringFilterMatchMode::Substring);
}
/// Expression reading the `string` property of each `StringObject`.
fn string_object_expression() -> gtk4::PropertyExpression {
    gtk4::PropertyExpression::new(
        gtk4::StringObject::static_type(),
        None::<gtk4::Expression>,
        "string",
    )
}

/// Builder for creating `adw::ActionRow` with a `DropDown` suffix.
///
/// # Example
/// ```ignore
/// let (row, dropdown) = DropdownRowBuilder::new("Protocol")
///     .subtitle("Connection protocol")
///     .items(&["SSH", "RDP", "VNC"])
///     .selected(0)
///     .build();
/// group.add(&row);
/// ```
#[derive(Default)]
pub struct DropdownRowBuilder {
    title: String,
    subtitle: Option<String>,
    items: Vec<String>,
    selected: u32,
}

impl DropdownRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            items: Vec::new(),
            selected: 0,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the dropdown items.
    #[must_use]
    pub fn items(mut self, items: &[&str]) -> Self {
        self.items = items.iter().map(|s| (*s).to_string()).collect();
        self
    }

    /// Sets the initially selected item index.
    #[must_use]
    pub fn selected(mut self, index: u32) -> Self {
        self.selected = index;
        self
    }

    /// Builds the `ActionRow` and `DropDown`.
    ///
    /// Returns a tuple of (row, dropdown) for adding to a preferences group
    /// and connecting signals.
    #[must_use]
    pub fn build(self) -> (adw::ActionRow, DropDown) {
        let items_refs: Vec<&str> = self.items.iter().map(String::as_str).collect();
        let string_list = StringList::new(&items_refs);
        let dropdown = DropDown::new(Some(string_list), gtk4::Expression::NONE);
        dropdown.set_selected(self.selected);

        let mut row_builder = adw::ActionRow::builder().title(i18n(&self.title));

        if let Some(subtitle) = &self.subtitle {
            row_builder = row_builder.subtitle(i18n(subtitle));
        }

        let row = row_builder.build();
        row.add_suffix(&dropdown);

        (row, dropdown)
    }
}

/// Builder for creating `adw::SwitchRow`.
///
/// # Example
/// ```ignore
/// let switch_row = SwitchRowBuilder::new("Enable Feature")
///     .subtitle("Description of the feature")
///     .active(true)
///     .build();
/// group.add(&switch_row);
/// ```
#[derive(Default)]
pub struct SwitchRowBuilder {
    title: String,
    subtitle: Option<String>,
    active: bool,
}

impl SwitchRowBuilder {
    /// Creates a new builder with the given title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            active: false,
        }
    }

    /// Sets the subtitle (description) for the row.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the initial active state of the switch.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Builds the `SwitchRow`.
    ///
    /// Returns the switch row for adding to a preferences group.
    #[must_use]
    pub fn build(self) -> adw::SwitchRow {
        let mut builder = adw::SwitchRow::builder()
            .title(i18n(&self.title))
            .active(self.active);

        if let Some(subtitle) = &self.subtitle {
            builder = builder.subtitle(i18n(subtitle));
        }

        builder.build()
    }
}

/// Marks `field` invalid or valid, for the eye and for assistive technology.
///
/// The red outline alone tells a screen reader nothing, and the HIG asks
/// that no information be carried by colour only, so the accessible
/// `invalid` state follows the `error` style class. Shared by the highlight
/// rule editors and the RDP connection editor's KDC Address.
pub fn set_invalid(field: &impl IsA<gtk4::Widget>, invalid: bool) {
    let widget = field.upcast_ref::<gtk4::Widget>();
    if invalid {
        widget.add_css_class("error");
        widget.update_state(&[gtk4::accessible::State::Invalid(
            gtk4::AccessibleInvalidState::True,
        )]);
    } else {
        widget.remove_css_class("error");
        widget.reset_state(gtk4::AccessibleState::Invalid);
    }
}

/// Shared text and field feedback for the two highlight rule editors.
///
/// The global editor in Settings and the per-connection one in the connection
/// editor used to accept anything silently: an invalid pattern or colour was
/// stored and then drew nothing, with no sign of why (issue #343). Both now
/// mark such a field with the `error` style class. The value itself is still
/// stored as typed, so nothing is lost mid-edit.
pub mod highlight_fields {
    use gtk4::prelude::*;

    use super::set_invalid;
    use crate::i18n::i18n;

    /// Label of the underline (foreground) colour field.
    ///
    /// The overlay cannot recolour the terminal's glyphs, so the foreground
    /// colour is drawn as an underline. It used to be labelled "Text colour",
    /// which promised something the terminal never did (issue #343).
    #[must_use]
    pub fn underline_colour_label() -> String {
        i18n("Underline colour (#RRGGBB)")
    }

    /// Label of the background colour field.
    #[must_use]
    pub fn background_colour_label() -> String {
        i18n("Background colour (#RRGGBB)")
    }

    /// Tooltip of the underline (foreground) colour field.
    #[must_use]
    pub fn underline_colour_tooltip() -> String {
        i18n(
            "Hexadecimal colour such as #00AAFF, drawn as an underline under the match. Leave empty for none. To recolour the text itself, give the connection an Output Filter such as ChromaTerm.",
        )
    }

    /// Tooltip of the background colour field.
    #[must_use]
    pub fn background_colour_tooltip() -> String {
        i18n("Hexadecimal colour such as #402020. Leave empty for no background.")
    }

    /// Marks a colour field invalid unless its text is empty or `#RRGGBB`.
    pub fn show_colour_validity(field: &impl IsA<gtk4::Widget>, text: &str) {
        set_invalid(field, !rustconn_core::highlight::is_valid_color_input(text));
    }

    /// Marks a pattern field invalid when its text is not a regular expression.
    ///
    /// An invalid pattern's tooltip becomes the regex engine's own message,
    /// which names the problem; a valid one gets `tooltip` back.
    pub fn show_pattern_validity(field: &impl IsA<gtk4::Widget>, text: &str, tooltip: &str) {
        match rustconn_core::highlight::validate_pattern(text) {
            Ok(()) => {
                set_invalid(field, false);
                field.set_tooltip_text(Some(tooltip));
            }
            Err(error) => {
                set_invalid(field, true);
                field.set_tooltip_text(Some(&error.to_string()));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        /// The style class and the accessible state are set and cleared
        /// together; a field corrected by the user must lose both.
        #[test]
        #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact <this test path>`"]
        fn an_invalid_colour_is_flagged_and_a_corrected_one_is_cleared() {
            use gtk4::prelude::*;

            if gtk4::init().is_err() {
                return;
            }
            let entry = gtk4::Entry::new();
            super::show_colour_validity(&entry, "#0а0ff");
            assert!(entry.has_css_class("error"));
            super::show_colour_validity(&entry, "#00AAFF");
            assert!(!entry.has_css_class("error"));
            super::show_colour_validity(&entry, "");
            assert!(
                !entry.has_css_class("error"),
                "an empty field means no colour"
            );
        }
    }
}

/// Adds a "Select KeePass Database" file-picker button to `row`, writing the
/// chosen `.kdbx` path into the row (issue #411).
///
/// Shared by the connection editor and the group editor so both KeePass
/// Database rows pick a file the same way.
pub fn add_kdbx_browse_suffix(row: &adw::EntryRow) {
    let button = Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(i18n("Select KeePass Database"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Select KeePass Database",
    ))]);
    row.add_suffix(&button);

    let row = row.clone();
    button.connect_clicked(move |button| {
        let filter = gtk4::FileFilter::new();
        filter.add_pattern("*.kdbx");
        filter.set_name(Some(&i18n("KeePass Database (*.kdbx)")));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        let dialog = gtk4::FileDialog::builder()
            .title(i18n("Select KeePass Database"))
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let Some(window) = button
            .root()
            .and_then(|r| r.downcast::<gtk4::Window>().ok())
        else {
            tracing::warn!("KeePass database picker: button has no window root");
            return;
        };
        let row = row.clone();
        dialog.open(
            Some(&window),
            gtk4::gio::Cancellable::NONE,
            move |result| match result {
                Ok(file) => {
                    if let Some(path) = file.path() {
                        row.set_text(&path.to_string_lossy());
                    } else {
                        tracing::warn!("KeePass database picker: chosen file has no local path");
                    }
                }
                Err(e) => tracing::debug!(error = %e, "KeePass database picker closed"),
            },
        );
    });
}

/// Marker function for xgettext to discover dialog header button labels.
/// These strings are used indirectly via `dialog_header()` and would otherwise
/// be invisible to the POT extraction tool.
///
/// This function is never called at runtime.
fn _i18n_markers() {
    // Button labels used in dialog_header() across the codebase
    i18n("Close");
    i18n("Save");
    i18n("Create");
    i18n("Export");
    i18n("Import");
    i18n("Copy");
    i18n("Connect");
    i18n("Send");
    i18n("Use Template");
    i18n("Done");

    // Rows built through `SwitchRowBuilder`, whose `build()` calls `i18n()` on a
    // *variable* — so `xgettext` sees nothing at the call site and these msgids
    // were absent from all 16 catalogues while rendering through gettext at
    // runtime, which is to say rendering in English forever. Same class of rot
    // as the seven strings 0.20.0 recovered, and `check-potfiles.sh` cannot
    // catch it: the file is listed, it is the string that is invisible.
    //
    // The Web connection panel is the builder's only caller. Adding the
    // floating-toolbar row (issue #260) is what surfaced it.
    i18n("JavaScript");
    i18n("Enable or disable JavaScript execution in the embedded browser");
    i18n("Navigation Toolbar");
    i18n(
        "Turn off to remove the floating toolbar and its reveal arrow. Keyboard shortcuts for back, forward, reload and zoom still work.",
    );
    i18n("Private / Incognito Mode");
    i18n("Open URL in a private browsing window (Firefox, Chrome, Brave)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_labels_functions() {
        // Labels return non-empty strings (actual content depends on locale)
        assert_ne!(labels::root_group(), "");
        assert_ne!(labels::none_label(), "");
        assert_ne!(labels::no_keys_loaded(), "");
    }

    fn button(index: usize, action: RowAction) -> FocusTarget {
        FocusTarget::Button { index, action }
    }

    fn row(index: usize, action: RowAction) -> Refocus {
        Refocus::Row { index, action }
    }

    #[test]
    fn loading_the_list_leaves_focus_alone() {
        assert_eq!(Refocus::Keep.target(3, true), FocusTarget::Unchanged);
        assert_eq!(Refocus::Keep.target(0, true), FocusTarget::Unchanged);
    }

    #[test]
    fn a_moved_entry_keeps_focus_on_its_arrow() {
        // Entry 2 of 3 moved up to 1: Move Up is still usable there.
        assert_eq!(
            row(2, RowAction::Up).target(3, true),
            button(1, RowAction::Up)
        );
        // Entry 1 moved up to the top, where Move Up is insensitive.
        assert_eq!(
            row(1, RowAction::Up).target(3, true),
            button(0, RowAction::Down)
        );
        assert_eq!(
            row(0, RowAction::Down).target(3, true),
            button(1, RowAction::Down)
        );
        // Entry 1 moved down to the bottom, where Move Down is insensitive.
        assert_eq!(
            row(1, RowAction::Down).target(3, true),
            button(2, RowAction::Up)
        );
    }

    #[test]
    fn removing_an_entry_focuses_the_one_that_takes_its_place() {
        assert_eq!(
            row(0, RowAction::Remove).target(2, true),
            button(0, RowAction::Remove)
        );
        // The last entry went: the new last one takes focus.
        assert_eq!(
            row(2, RowAction::Remove).target(2, true),
            button(1, RowAction::Remove)
        );
        // The list is empty: focus goes to the Add dropdown.
        assert_eq!(
            row(0, RowAction::Remove).target(0, true),
            FocusTarget::AddDropdown
        );
        assert_eq!(
            row(0, RowAction::Remove).target(0, false),
            FocusTarget::Unchanged
        );
    }

    #[test]
    fn adding_an_entry_returns_to_the_add_dropdown() {
        assert_eq!(Refocus::Added.target(2, true), FocusTarget::AddDropdown);
        // Nothing is left to add, so the dropdown is insensitive.
        assert_eq!(
            Refocus::Added.target(2, false),
            button(1, RowAction::Remove)
        );
    }

    #[test]
    fn a_stale_click_never_points_past_the_list() {
        assert_eq!(
            row(5, RowAction::Down).target(2, true),
            button(1, RowAction::Up)
        );
        assert_eq!(
            row(5, RowAction::Remove).target(2, true),
            button(1, RowAction::Remove)
        );
    }

    fn ids(count: u128) -> Vec<uuid::Uuid> {
        (1..=count).map(uuid::Uuid::from_u128).collect()
    }

    #[test]
    fn two_quick_moves_move_the_same_entry_twice() {
        let [a, b, c] = [1, 2, 3].map(uuid::Uuid::from_u128);
        let mut selected = ids(3);
        // Both clicks land on c's Move Up before the list redraws.
        assert_eq!(apply_row_action(&mut selected, c, RowAction::Up), Some(2));
        assert_eq!(apply_row_action(&mut selected, c, RowAction::Up), Some(1));
        assert_eq!(selected, vec![c, a, b]);
    }

    #[test]
    fn a_second_remove_click_does_not_remove_the_neighbour() {
        let [a, b, c] = [1, 2, 3].map(uuid::Uuid::from_u128);
        let mut selected = ids(3);
        assert_eq!(
            apply_row_action(&mut selected, b, RowAction::Remove),
            Some(1)
        );
        assert_eq!(apply_row_action(&mut selected, b, RowAction::Remove), None);
        assert_eq!(selected, vec![a, c]);
    }

    #[test]
    fn a_move_at_the_edge_changes_nothing() {
        let [a, _, c] = [1, 2, 3].map(uuid::Uuid::from_u128);
        let mut selected = ids(3);
        assert_eq!(apply_row_action(&mut selected, a, RowAction::Up), Some(0));
        assert_eq!(apply_row_action(&mut selected, c, RowAction::Down), Some(2));
        assert_eq!(selected, ids(3));
    }
}
