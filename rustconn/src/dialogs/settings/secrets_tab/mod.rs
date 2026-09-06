//! Secrets settings tab using libadwaita components

pub(crate) mod detection;
mod keyring;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Entry, FileDialog, FileFilter, Label, Orientation, StringList, glib,
};
use libadwaita as adw;
use rustconn_core::config::{SecretBackendType, SecretSettings};
use rustconn_core::secret::{CredentialStorage, set_session_key};
use secrecy::SecretString;

use self::detection::{
    BwVaultStatus, LocalBackendState, SecretCliDetection, backend_readiness,
    check_bitwarden_status_sync, detect_secret_backends, read_passbolt_server_url_sync,
};
use self::keyring::{
    delete_bw_api_credentials_from_keyring, delete_bw_password_from_keyring,
    delete_kdbx_password_from_keyring, delete_op_token_from_keyring,
    delete_pb_passphrase_from_keyring, delete_portable_passphrase_from_keyring,
    get_bw_password_from_keyring, get_kdbx_password_from_keyring, get_op_token_from_keyring,
    get_pb_passphrase_from_keyring, get_portable_passphrase_from_keyring,
    save_bw_api_credentials_to_keyring, save_bw_password_to_keyring, save_kdbx_password_to_keyring,
    save_op_token_to_keyring, save_pb_passphrase_to_keyring, save_portable_passphrase_to_keyring,
};
use crate::i18n::{i18n, i18n_f};

/// Which backends the system keyring could **not** supply a secret for when the
/// dialog opened.
///
/// A flag stays `true` while the backend's storage choice is "System keyring"
/// but the lookup came back empty — the keyring is *known* to be missing that
/// secret, usually because an earlier write failed (D-Bus down, or KWallet
/// slower than the keyring module's 5-second save timeout).
///
/// This is what makes a retry possible. `SecretSettings::has_new_runtime_secret`
/// only counts a secret that is newly present or *different*, so retyping the
/// same password after a failed write looked like "nothing changed" and the save
/// — and with it the keyring write — was skipped. See [`Self::needs_write`].
#[derive(Debug, Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one independent flag per backend; a set would hide which backend is meant"
)]
pub struct KeyringGaps {
    /// The KeePass database password is not in the keyring.
    pub kdbx: bool,
    /// The Bitwarden master password is not in the keyring.
    pub bitwarden: bool,
    /// The 1Password service account token is not in the keyring.
    pub onepassword: bool,
    /// The Passbolt GPG passphrase is not in the keyring.
    pub passbolt: bool,
    /// The portable credential file passphrase is not in the keyring.
    pub portable: bool,
}

impl KeyringGaps {
    /// Seeds the gaps pessimistically from a saved configuration.
    ///
    /// Every backend that selected the keyring starts out "missing"; the async
    /// loaders clear their flag once a lookup actually returns a value.
    fn from_settings(settings: &SecretSettings) -> Self {
        Self {
            kdbx: settings.kdbx_save_to_keyring,
            bitwarden: settings.bitwarden_save_to_keyring,
            onepassword: settings.onepassword_save_to_keyring,
            passbolt: settings.passbolt_save_to_keyring,
            portable: settings.portable_save_to_keyring,
        }
    }

    /// Clears one backend's gap in a shared cell after a lookup returned a
    /// value. Takes the whole [`Cell`] because [`Self`] is `Copy`, so there is
    /// no borrow to hold across the GTK callback that calls this.
    fn resolve(cell: &Cell<Self>, mark: fn(&mut Self)) {
        let mut gaps = cell.get();
        mark(&mut gaps);
        cell.set(gaps);
    }

    /// Reports whether a collected secret still has to be written even though
    /// the persisted fields and the runtime comparison both say "unchanged".
    ///
    /// True only when the keyring is the selected persistence layer, the keyring
    /// is known not to hold the secret, and the dialog actually collected one. A
    /// blank entry never qualifies, so an untouched open/close round trip on a
    /// healthy keyring stays a no-op.
    #[must_use]
    pub fn needs_write(self, collected: &SecretSettings) -> bool {
        (self.kdbx
            && collected.kdbx_save_to_keyring
            && collected.kdbx_use_password
            && collected.kdbx_password.is_some())
            || (self.bitwarden
                && collected.bitwarden_save_to_keyring
                && collected.bitwarden_password.is_some())
            || (self.onepassword
                && collected.onepassword_save_to_keyring
                && collected.onepassword_service_account_token.is_some())
            || (self.passbolt
                && collected.passbolt_save_to_keyring
                && collected.passbolt_passphrase.is_some())
            || (self.portable
                && collected.portable_save_to_keyring
                && collected.portable_passphrase.is_some())
    }
}

/// Return type for secrets page - contains all widgets needed for dynamic visibility.
///
/// **Note for `collect_secret_settings()`**: only the following fields are read during
/// settings collection. The close handler in `mod.rs` creates a temporary instance
/// with dummy values for the remaining display-only fields. When adding a new widget
/// that must participate in collection, add it to the `collect_secret_settings()`
/// function AND update the temporary instance construction in the close handler.
///
/// Fields used by collect: `secret_backend_dropdown`, `enable_fallback`,
/// `kdbx_path_entry`, `kdbx_password_entry`, `kdbx_enabled_row`, `kdbx_storage_combo`,
/// `kdbx_key_file_entry`, `kdbx_use_key_file_check`, `kdbx_use_password_check`,
/// `bitwarden_password_entry`, `bitwarden_storage_combo`, `bitwarden_use_api_key_check`,
/// `bitwarden_client_id_entry`, `bitwarden_client_secret_entry`,
/// `onepassword_token_entry`, `onepassword_storage_combo`,
/// `passbolt_passphrase_entry`, `passbolt_storage_combo`, `passbolt_server_url_entry`,
/// `pass_store_dir_entry`.
#[expect(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct SecretsPageWidgets {
    pub page: adw::PreferencesPage,
    /// Backend selector. Its rows, order and index→backend mapping come from
    /// [`backend_choices`]; the row's subtitle tracks the current choice.
    pub secret_backend_dropdown: adw::ComboRow,
    pub enable_fallback: adw::SwitchRow,
    /// Opens the credential transfer dialog.
    ///
    /// Left unwired by [`create_secrets_page`]: the transfer is driven by the
    /// connection and group lists, which this page never sees.
    /// `SettingsDialog::connect_credential_transfer` attaches the handler.
    pub transfer_button: Button,
    /// Opens the portable file's passphrase change dialog.
    ///
    /// Left unwired here for the same reason as `transfer_button`: a successful
    /// change has to be installed in the session settings and the live backend,
    /// and this page holds neither. `SettingsDialog::connect_portable_passphrase_change`
    /// attaches the handler.
    pub portable_change_passphrase_button: Button,
    pub kdbx_path_entry: Entry,
    pub kdbx_password_entry: adw::PasswordEntryRow,
    pub kdbx_enabled_row: adw::SwitchRow,
    /// 3-state credential storage selector for KeePassXC database password.
    pub kdbx_storage_combo: adw::ComboRow,
    pub kdbx_status_label: Label,
    pub kdbx_browse_button: Button,
    pub kdbx_check_button: Button,
    pub keepassxc_status_container: GtkBox,
    pub kdbx_key_file_entry: Entry,
    pub kdbx_key_file_browse_button: Button,
    pub kdbx_use_key_file_check: adw::SwitchRow,
    pub kdbx_use_password_check: adw::SwitchRow,
    // Additional rows for visibility control
    pub kdbx_group: adw::PreferencesGroup,
    pub auth_group: adw::PreferencesGroup,
    pub status_group: adw::PreferencesGroup,
    pub password_row: adw::PasswordEntryRow,
    pub key_file_row: adw::ActionRow,
    // Bitwarden widgets
    pub bitwarden_group: adw::PreferencesGroup,
    pub bitwarden_status_label: Label,
    pub bitwarden_unlock_button: Button,
    pub bitwarden_password_entry: adw::PasswordEntryRow,
    /// 3-state credential storage selector for Bitwarden master password.
    pub bitwarden_storage_combo: adw::ComboRow,
    pub bitwarden_use_api_key_check: adw::SwitchRow,
    pub bitwarden_client_id_entry: Entry,
    pub bitwarden_client_secret_entry: adw::PasswordEntryRow,
    /// Detected Bitwarden CLI command path (updated async)
    pub bitwarden_cmd: Rc<RefCell<String>>,
    // 1Password widgets
    pub onepassword_group: adw::PreferencesGroup,
    pub onepassword_status_label: Label,
    pub onepassword_signin_button: Button,
    // Passbolt widgets
    pub passbolt_group: adw::PreferencesGroup,
    pub passbolt_status_label: Label,
    pub passbolt_server_url_entry: Entry,
    pub passbolt_open_vault_button: Button,
    pub passbolt_passphrase_entry: adw::PasswordEntryRow,
    /// 3-state credential storage selector for Passbolt GPG passphrase.
    pub passbolt_storage_combo: adw::ComboRow,
    // 1Password credential widgets
    pub onepassword_token_entry: adw::PasswordEntryRow,
    /// 3-state credential storage selector for 1Password service account token.
    pub onepassword_storage_combo: adw::ComboRow,
    /// Cached result of `which secret-tool` (populated by background detection)
    pub secret_tool_available: Rc<RefCell<Option<bool>>>,
    /// Backends whose keyring lookup came back empty at dialog-open time.
    ///
    /// Seeded by [`load_secret_settings`] and cleared per backend by the async
    /// keyring loaders. Read by the dialog's dirty check so a retyped secret can
    /// retry a keyring write that silently failed.
    pub keyring_gaps: Rc<Cell<KeyringGaps>>,
    /// Detected 1Password CLI command path (updated async)
    pub onepassword_cmd: Rc<RefCell<String>>,
    // Pass widgets
    pub pass_group: adw::PreferencesGroup,
    pub pass_store_dir_entry: Entry,
    pub pass_store_dir_browse_button: Button,
    pub pass_status_label: Label,
    /// Machine-bound encrypted file group. It has nothing to configure, but it
    /// tells the user where the file is and what is in it — selecting the backend
    /// used to display nothing at all.
    pub encrypted_file_group: adw::PreferencesGroup,
    // Portable encrypted file widgets
    pub portable_group: adw::PreferencesGroup,
    pub portable_path_entry: Entry,
    pub portable_browse_button: Button,
    pub portable_passphrase_entry: adw::PasswordEntryRow,
    /// Second entry guarding against a mistyped passphrase, which would
    /// otherwise produce an unopenable store with no way to find out.
    pub portable_confirm_entry: adw::PasswordEntryRow,
    /// 3-state credential storage selector for portable file passphrase.
    pub portable_storage_combo: adw::ComboRow,
    /// The portable group's shared outcome row.
    ///
    /// Exposed so `SettingsDialog::connect_portable_passphrase_change` can report
    /// into the same slot as *Create File* and *Copy Credentials*, rather than
    /// adding a fourth status row to a group that already has two.
    pub portable_status_label: Label,
}

/// Finds the preferences dialog a widget lives in, for reporting an outcome.
///
/// `AdwPreferencesDialog` is an `AdwDialog`, not a `GtkRoot`, so `root()` walks
/// past it to the application window and the downcast fails. `ancestor` is the
/// lookup that actually finds it.
fn enclosing_preferences_dialog(widget: &impl IsA<gtk4::Widget>) -> Option<adw::PreferencesDialog> {
    widget
        .ancestor(adw::PreferencesDialog::static_type())
        .and_downcast::<adw::PreferencesDialog>()
}

/// Shows a toast on a preferences dialog.
fn show_toast(dialog: &adw::PreferencesDialog, message: &str, timeout_secs: u32) {
    let toast = adw::Toast::new(message);
    toast.set_timeout(timeout_secs);
    dialog.add_toast(toast);
}

/// Index in the storage `StringList` for [`CredentialStorage::None`].
const STORAGE_NONE_INDEX: u32 = 0;
/// Index in the storage `StringList` for [`CredentialStorage::EncryptedFile`].
const STORAGE_ENCRYPTED_INDEX: u32 = 1;
/// Index in the storage `StringList` for [`CredentialStorage::SystemKeyring`].
const STORAGE_KEYRING_INDEX: u32 = 2;

/// Maps a [`CredentialStorage`] to its `StringList` index.
const fn storage_to_index(storage: CredentialStorage) -> u32 {
    match storage {
        CredentialStorage::None => STORAGE_NONE_INDEX,
        CredentialStorage::EncryptedFile => STORAGE_ENCRYPTED_INDEX,
        CredentialStorage::SystemKeyring => STORAGE_KEYRING_INDEX,
    }
}

/// Maps a `StringList` index back to a [`CredentialStorage`]. Unknown indices
/// fall back to [`CredentialStorage::None`].
const fn index_to_storage(idx: u32) -> CredentialStorage {
    match idx {
        STORAGE_ENCRYPTED_INDEX => CredentialStorage::EncryptedFile,
        STORAGE_KEYRING_INDEX => CredentialStorage::SystemKeyring,
        _ => CredentialStorage::None,
    }
}

/// Builds an `AdwComboRow` with the canonical 3-state credential storage
/// choice: "Don't save" / "Encrypted file (machine-specific)" /
/// "System keyring (recommended)".
///
/// The combo enforces availability of `secret-tool` for the keyring option:
/// if the user picks "System keyring" while `secret_tool_available` is
/// **confirmed false** (`Some(false)`), the selection is reverted to the
/// previous one and `status_label` shows a warning. While detection is still
/// pending (`None`), the selection is allowed — this prevents corruption of
/// previously-saved config values loaded before async detection completes
/// (issue #259).
fn make_storage_combo(
    title: &str,
    secret_tool_available: Rc<RefCell<Option<bool>>>,
    status_label: Label,
) -> adw::ComboRow {
    let model = StringList::new(&[
        i18n("Don't save").as_str(),
        i18n("Encrypted file (machine-specific)").as_str(),
        i18n("System keyring (recommended)").as_str(),
    ]);
    let combo = adw::ComboRow::builder()
        .title(title)
        .subtitle(i18n("How to persist the credential between sessions"))
        .model(&model)
        .selected(STORAGE_NONE_INDEX)
        .build();

    // Track previous selection so we can revert if the user picks keyring
    // while secret-tool is unavailable.
    let previous: Rc<RefCell<u32>> = Rc::new(RefCell::new(STORAGE_NONE_INDEX));
    {
        let combo_clone = combo.clone();
        let previous_clone = previous.clone();
        combo.connect_selected_notify(move |c| {
            let new_sel = c.selected();
            // Only revert when detection has COMPLETED and confirmed that
            // secret-tool is absent. While detection is pending (None) we
            // allow the selection — the value came from a previously-saved
            // config that was already working (#259).
            if new_sel == STORAGE_KEYRING_INDEX && *secret_tool_available.borrow() == Some(false) {
                let revert_to = *previous_clone.borrow();
                update_status_label(
                    &status_label,
                    &i18n("System keyring unavailable — install libsecret (secret-tool)"),
                    "warning",
                );
                tracing::warn!("secret-tool not found, cannot use system keyring");
                // Revert without re-triggering this handler infinitely:
                // selected_notify will still fire but the guard above is a
                // no-op for non-keyring indices.
                combo_clone.set_selected(revert_to);
                return;
            }
            *previous_clone.borrow_mut() = new_sel;
        });
    }

    combo
}

/// Reads the current [`CredentialStorage`] choice from a storage combo.
fn storage_combo_value(combo: &adw::ComboRow) -> CredentialStorage {
    index_to_storage(combo.selected())
}

/// Sets a storage combo to a value loaded from the saved configuration.
///
/// `set_selected` does emit `selected-notify`, so the availability guard in
/// [`make_storage_combo`] runs — it is simply a no-op at load time, because
/// detection has not resolved yet. That is deliberate: a position that came
/// from a previously-saved config must survive being displayed (issue #259).
fn set_storage_combo_value(combo: &adw::ComboRow, storage: CredentialStorage) {
    combo.set_selected(storage_to_index(storage));
}

/// Warns about keyring selections the machine turns out not to support.
///
/// Runs once background detection reports that `secret-tool` is missing. The
/// selections are deliberately left as they are: they came from a saved
/// configuration, and rewriting them behind the user's back is exactly what
/// destroyed the KeePassXC setting in issue
/// [#259](https://github.com/totoshko88/RustConn/issues/259). The guard in
/// [`make_storage_combo`] still blocks picking the keyring from here on, so
/// this only has to explain the situation.
fn warn_about_unavailable_keyring(combos: &[(&adw::ComboRow, &Label)]) {
    for (combo, status_label) in combos {
        if combo.selected() == STORAGE_KEYRING_INDEX {
            update_status_label(
                status_label,
                &i18n("System keyring unavailable — install libsecret (secret-tool)"),
                "warning",
            );
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Backend selector
// ──────────────────────────────────────────────────────────────────────────────

/// Selector position of the `KeePassXC` / KDBX backend.
const BACKEND_KEEPASSXC_INDEX: u32 = 0;
/// Selector position of the platform system keyring (libsecret, or Keychain on macOS).
const BACKEND_SYSTEM_KEYRING_INDEX: u32 = 1;
/// Selector position of the Bitwarden backend.
const BACKEND_BITWARDEN_INDEX: u32 = 2;
/// Selector position of the 1Password backend.
const BACKEND_ONEPASSWORD_INDEX: u32 = 3;
/// Selector position of the Passbolt backend.
const BACKEND_PASSBOLT_INDEX: u32 = 4;
/// Selector position of the `pass` backend.
const BACKEND_PASS_INDEX: u32 = 5;
/// Selector position of the machine-bound encrypted file backend.
const BACKEND_ENCRYPTED_FILE_INDEX: u32 = 6;
/// Selector position of the portable, passphrase-protected file backend.
const BACKEND_PORTABLE_INDEX: u32 = 7;

/// One row of the backend selector.
pub struct BackendChoice {
    /// Name shown in the row and in the popup list.
    pub label: String,
    /// One line under the name saying what this backend actually does.
    ///
    /// Shown in the popup list, and as the row's subtitle for the current
    /// choice. Without it the two file backends read as near-duplicates:
    /// nothing in "Encrypted file" next to "Portable encrypted file" says that
    /// the difference is where the encryption key comes from, which is the only
    /// thing that matters when picking between them.
    description: String,
    /// Configuration value this row selects.
    pub backend: SecretBackendType,
}

/// The backend selector's rows, in model order.
///
/// One table instead of the four hand-written index matches this page used to
/// carry: the labels, the [`SecretBackendType`] each position maps to, and now
/// the explanations, were written out separately in the page builder, the
/// visibility handler, [`load_secret_settings`] and [`collect_secret_settings`],
/// so reordering the list silently changed what a saved configuration meant.
pub fn backend_choices() -> Vec<BackendChoice> {
    /// Shorthand so the table below reads as a table.
    fn choice(label: &str, description: String, backend: SecretBackendType) -> BackendChoice {
        BackendChoice {
            label: label.to_owned(),
            description,
            backend,
        }
    }

    // Labels come from `SecretBackendType::display_name()` rather than being
    // written out again here. They were duplicated until a message elsewhere
    // needed the same names and had nothing to call, so the banner printed Rust
    // variant names (`MacOsKeychain`) instead. One source, two consumers: this
    // table for the selector, `display_name()` for prose.
    //
    // Product names are not translated, which is why `display_name()` returns
    // English and only the three descriptive labels are wrapped in `i18n()`.
    // Index 1 is the platform system keyring: libsecret on Linux/BSD, the native
    // Keychain on macOS (libsecret does not exist there).
    #[cfg(target_os = "macos")]
    let keyring = choice(
        SecretBackendType::MacOsKeychain.display_name(),
        i18n("This Mac's login keychain, unlocked with your session"),
        SecretBackendType::MacOsKeychain,
    );
    #[cfg(not(target_os = "macos"))]
    let keyring = choice(
        SecretBackendType::LibSecret.display_name(),
        i18n("This computer's login keyring, unlocked with your session"),
        SecretBackendType::LibSecret,
    );

    vec![
        choice(
            SecretBackendType::KeePassXc.display_name(),
            i18n("A KeePass database, through the KeePassXC application"),
            SecretBackendType::KeePassXc,
        ),
        keyring,
        choice(
            SecretBackendType::Bitwarden.display_name(),
            i18n("Your Bitwarden vault, through the bw command line tool"),
            SecretBackendType::Bitwarden,
        ),
        choice(
            SecretBackendType::OnePassword.display_name(),
            i18n("Your 1Password vault, through the op command line tool"),
            SecretBackendType::OnePassword,
        ),
        choice(
            SecretBackendType::Passbolt.display_name(),
            i18n("Your Passbolt server, through the passbolt command line tool"),
            SecretBackendType::Passbolt,
        ),
        choice(
            SecretBackendType::Pass.display_name(),
            i18n("The pass password store, encrypted with your GPG key"),
            SecretBackendType::Pass,
        ),
        // The two file backends differ only in where the key comes from, so each
        // description names that and nothing else. Descriptive rather than
        // product names, so these two are translated — sentence case per GNOME HIG.
        //
        // These two labels stay *literal* inside `i18n()` instead of calling
        // `display_name()` like the rows above. `po/update-pot.sh` runs xgettext,
        // which extracts by matching `i18n("…")` on a string literal: pass it an
        // expression and the string is silently dropped from the catalogue and
        // ships untranslated in all 17 locales. The strings are asserted equal to
        // `display_name()` in this module's tests so the two cannot drift.
        choice(
            &i18n("Encrypted file"),
            i18n("A file on this computer, encrypted with a key tied to this machine"),
            SecretBackendType::EncryptedFile,
        ),
        choice(
            &i18n("Portable encrypted file"),
            i18n(
                "A file encrypted with a passphrase you choose, so it opens on your other computers",
            ),
            SecretBackendType::PortableEncryptedFile,
        ),
    ]
}

/// Maps a selector position to the backend it selects.
///
/// An index outside the table falls back to the configuration default, which is
/// what a config written by a newer version would produce.
pub fn index_to_backend(index: u32) -> SecretBackendType {
    backend_choices()
        .get(index as usize)
        .map_or_else(SecretBackendType::default, |choice| choice.backend)
}

/// Maps a backend to its selector position.
///
/// Several variants share a row: the KDBX file backend is reached through the
/// `KeePassXC` entry, and the two system-keyring variants share the one entry
/// whose identity depends on the platform. That folding is why this is a match
/// rather than a table lookup.
pub const fn backend_to_index(backend: SecretBackendType) -> u32 {
    match backend {
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => BACKEND_KEEPASSXC_INDEX,
        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
            BACKEND_SYSTEM_KEYRING_INDEX
        }
        SecretBackendType::Bitwarden => BACKEND_BITWARDEN_INDEX,
        SecretBackendType::OnePassword => BACKEND_ONEPASSWORD_INDEX,
        SecretBackendType::Passbolt => BACKEND_PASSBOLT_INDEX,
        SecretBackendType::Pass => BACKEND_PASS_INDEX,
        SecretBackendType::EncryptedFile => BACKEND_ENCRYPTED_FILE_INDEX,
        SecretBackendType::PortableEncryptedFile => BACKEND_PORTABLE_INDEX,
    }
}

/// Puts the selected backend's explanation into the selector row's subtitle.
///
/// Called on every selection change *and* explicitly after `set_selected`,
/// because selecting the value that is already current emits no
/// `selected-notify` — which is the common case when a saved configuration is
/// loaded, and would have left the row describing the wrong backend.
fn sync_backend_subtitle(row: &adw::ComboRow) {
    let choices = backend_choices();
    if let Some(choice) = choices.get(row.selected() as usize) {
        row.set_subtitle(&choice.description);
    }
}

/// Builds the popup-list factory for the backend selector: the name, with its
/// explanation on a second, dimmed line.
///
/// `AdwComboRow` renders one line per item by default, which is why the selector
/// could not say how two similarly named backends differ. Only the *list*
/// factory is replaced: the collapsed row keeps the plain name and carries the
/// explanation in its own subtitle, so the row itself does not grow to two lines
/// of its own on top of the title.
fn backend_list_factory(descriptions: Vec<String>) -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };
        let name = Label::builder().xalign(0.0).build();
        // `max_width_chars` with `wrap` keeps a long explanation from stretching
        // the popover to the width of the whole dialog.
        let description = Label::builder()
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(40)
            .css_classes(["dim-label", "caption"])
            .build();
        let column = GtkBox::new(Orientation::Vertical, 2);
        column.append(&name);
        column.append(&description);
        item.set_child(Some(&column));
    });

    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };
        let Some(column) = item.child().and_downcast::<GtkBox>() else {
            return;
        };
        let Some(name) = column.first_child().and_downcast::<Label>() else {
            return;
        };
        let Some(description) = name.next_sibling().and_downcast::<Label>() else {
            return;
        };
        let text = item
            .item()
            .and_downcast::<gtk4::StringObject>()
            .map(|obj| obj.string().to_string())
            .unwrap_or_default();
        name.set_label(&text);
        // The model is a `StringList`, so the explanation is matched by position
        // rather than carried on the item. Both come from `backend_choices()` in
        // the same order, and a position past the end simply shows no second
        // line instead of the wrong one.
        description.set_label(
            descriptions
                .get(item.position() as usize)
                .map_or("", String::as_str),
        );
        description.set_visible(!description.label().is_empty());
    });

    factory
}

// ──────────────────────────────────────────────────────────────────────────────
// Credential file rows
// ──────────────────────────────────────────────────────────────────────────────

/// Expands a leading `~` in a user-typed path, returning `None` for a blank one.
///
/// Every path row on this page is free text, and a `~/Dropbox/…` typed into one
/// was stored verbatim: `PathBuf` gives `~` no special meaning, so the file
/// ended up in a literal directory called `~` next to the working directory.
/// Only a leading `~/` is handled — `~user` needs the password database and is
/// not something a GTK settings row should be resolving.
pub fn expand_user_path(text: &str) -> Option<std::path::PathBuf> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = if trimmed == "~" {
        dirs::home_dir()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir().map(|home| home.join(rest))
    } else {
        None
    };
    Some(expanded.unwrap_or_else(|| std::path::PathBuf::from(trimmed)))
}

/// Fills `label` with the state of a credential file, off the GTK main thread.
///
/// Three states matter to someone setting one of the file backends up, and none
/// of them used to be visible anywhere: the file holds passwords, it does not
/// exist yet, or it is there and cannot be read. A user who had typed a path had
/// no way to tell which, and found out only when a connection failed to save.
///
/// `count` runs on a blocking thread. The read is small, but the portable store
/// is frequently on a cloud-sync or FUSE mount where even `metadata` can block
/// for seconds, and freezing the Settings dialog on the setup this row exists to
/// explain would be the wrong trade.
fn refresh_credential_file_status<F>(label: &Label, path: std::path::PathBuf, count: F)
where
    F: FnOnce(&std::path::Path) -> Result<usize, String> + Send + 'static,
{
    let label = label.clone();
    glib::spawn_future_local(async move {
        let outcome = gtk4::gio::spawn_blocking(move || {
            // `exists()` and the count in the same closure: splitting them would
            // put a second round trip to the mount on the main thread.
            path.exists().then(|| count(&path))
        })
        .await;

        let (text, css) = match outcome {
            Ok(None) => (i18n("Not created yet"), "dim-label"),
            Ok(Some(Ok(0))) => (i18n("Ready, no passwords yet"), "dim-label"),
            // A count rather than a plural form: "Passwords stored: 3" needs no
            // plural rules, and this page would otherwise be the first user of
            // `ngettext` in the project — 16 catalogues' worth of plural forms
            // for one row.
            Ok(Some(Ok(count))) => (
                i18n_f("Passwords stored: {}", &[&count.to_string()]),
                "success",
            ),
            Ok(Some(Err(e))) => (i18n_f("Cannot read the file: {}", &[&e]), "error"),
            Err(_panic) => (i18n("Could not check the file"), "error"),
        };
        update_status_label(&label, &text, css);
    });
}

/// Validates the portable passphrase entries before an action that uses them.
///
/// Returns the passphrase, or the reason it cannot be used yet.
///
/// A store that does not exist cannot have its passphrase checked against
/// anything, and the first write makes whatever was typed the only key to the
/// file forever, so the confirmation is required there and optional for a file
/// that is already present. That is the rule [`collect_secret_settings`] and
/// `rustconn-cli` already apply; the migration button used to accept an
/// unconfirmed passphrase for a destination it was about to create, which is the
/// one case where a typo cannot be found out later.
fn portable_passphrase_for_action(
    passphrase: &adw::PasswordEntryRow,
    confirm: &adw::PasswordEntryRow,
    store_path: &std::path::Path,
) -> Result<SecretString, String> {
    let pass_text = passphrase.text();
    let confirm_text = confirm.text();

    if pass_text.is_empty() {
        return Err(i18n(
            "Enter the passphrase that will protect the portable file",
        ));
    }
    if !confirm_text.is_empty() && confirm_text != pass_text {
        return Err(i18n("The two passphrases do not match"));
    }
    if confirm_text.is_empty() && !store_path.exists() {
        return Err(i18n(
            "Repeat the passphrase to confirm it. A new file cannot be recovered if the passphrase is wrong.",
        ));
    }
    Ok(SecretString::from(pass_text.to_string()))
}

/// Builds a "File" row whose suffix label reports the file's state.
///
/// Returns the row and its label; the caller refreshes the label with
/// [`refresh_credential_file_status`] whenever the path it describes changes.
fn credential_file_status_row() -> (adw::ActionRow, Label) {
    let label = Label::builder()
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .wrap(true)
        // Wrapping without a ceiling lets a long error push the row's title out
        // of the dialog instead of taking a second line.
        .max_width_chars(32)
        .justify(gtk4::Justification::Right)
        .label(i18n("Checking..."))
        .css_classes(["dim-label"])
        .build();
    let row = adw::ActionRow::builder()
        .title(i18n("File"))
        .activatable(false)
        .build();
    row.add_suffix(&label);
    (row, label)
}

#[cfg(test)]
mod backend_table_tests {
    use super::{BACKEND_PORTABLE_INDEX, backend_choices, backend_to_index, index_to_backend};

    /// The table and the `BACKEND_*_INDEX` constants are two separate statements
    /// of the same fact, and `backend_to_index` is a third. Without this,
    /// reordering `backend_choices()` would still silently change what a saved
    /// configuration means — which is the failure the table was introduced to
    /// prevent.
    #[test]
    fn the_table_order_agrees_with_the_index_mapping() {
        for (position, choice) in backend_choices().iter().enumerate() {
            let index = u32::try_from(position).expect("the table is eight rows long");
            assert_eq!(
                backend_to_index(choice.backend),
                index,
                "row {position} ({}) does not map back to its own position",
                choice.label
            );
            assert_eq!(
                index_to_backend(index),
                choice.backend,
                "position {position} does not resolve to the backend in that row"
            );
        }
    }

    /// The two variants that share a row with another must still land on it, or a
    /// configuration written on the other platform would select the wrong entry.
    #[test]
    fn shared_rows_fold_onto_one_position() {
        use rustconn_core::config::SecretBackendType;

        assert_eq!(
            backend_to_index(SecretBackendType::KdbxFile),
            backend_to_index(SecretBackendType::KeePassXc)
        );
        assert_eq!(
            backend_to_index(SecretBackendType::LibSecret),
            backend_to_index(SecretBackendType::MacOsKeychain)
        );
    }

    /// An index from a newer version's configuration must not silently select a
    /// neighbouring backend.
    #[test]
    fn an_index_past_the_table_falls_back_to_the_default() {
        use rustconn_core::config::SecretBackendType;

        assert_eq!(
            index_to_backend(BACKEND_PORTABLE_INDEX + 1),
            SecretBackendType::default()
        );
    }
}

#[cfg(test)]
mod portable_confirmation_tests {
    use super::portable_passphrase_is_unconfirmed;

    /// Nothing typed is not a refusal. This is the case that used to warn about a
    /// passphrase the user had never entered: with no portable file on disk the
    /// "confirmation required" rule applied to two empty fields and reported a
    /// discard, every time Preferences was opened and closed.
    #[test]
    fn an_empty_passphrase_is_never_a_refusal() {
        let absent = std::path::Path::new("/nonexistent/rustconn-portable-store.enc");

        assert!(!portable_passphrase_is_unconfirmed(absent, "", ""));
        assert!(!portable_passphrase_is_unconfirmed(
            absent,
            "",
            "typed only here"
        ));
    }

    /// A store that does not exist yet cannot check the passphrase against
    /// anything, so the confirmation is mandatory rather than optional.
    #[test]
    fn a_new_store_requires_the_confirmation() {
        let absent = std::path::Path::new("/nonexistent/rustconn-portable-store.enc");

        assert!(portable_passphrase_is_unconfirmed(
            absent,
            "correct horse",
            ""
        ));
        assert!(portable_passphrase_is_unconfirmed(
            absent,
            "correct horse",
            "correct hors"
        ));
        assert!(!portable_passphrase_is_unconfirmed(
            absent,
            "correct horse",
            "correct horse"
        ));
    }

    /// An existing store checks the passphrase against the file, so a blank
    /// confirmation means "I did not retype it" and is accepted — but a filled
    /// one that disagrees is still a typo worth refusing.
    #[test]
    fn an_existing_store_accepts_a_blank_confirmation() {
        let dir = tempfile::tempdir().expect("temp dir");
        let present = dir.path().join("store.enc");
        std::fs::write(&present, b"not a real store, only has to exist").expect("write");

        assert!(!portable_passphrase_is_unconfirmed(
            &present,
            "correct horse",
            ""
        ));
        assert!(!portable_passphrase_is_unconfirmed(
            &present,
            "correct horse",
            "correct horse"
        ));
        assert!(portable_passphrase_is_unconfirmed(
            &present,
            "correct horse",
            "correct hors"
        ));
    }
}

/// Reports whether a typed portable passphrase will be discarded when saving.
///
/// A confirmation that is merely blank is accepted for a store that already
/// exists — the passphrase is checked against the file itself there, and "I did
/// not retype it" is the same answer the other backends accept. For a store that
/// does not exist yet there is nothing to check against and the first write
/// makes whatever was typed the key forever, so the confirmation is required.
///
/// Returns `false` when nothing was typed. That case used to reach the same
/// refusal branch as a real mismatch, so opening Preferences on a machine with
/// no portable file and closing it again logged a warning about a passphrase the
/// user had never entered.
///
/// [`collect_secret_settings`] and the dialog's close handler both call this, so
/// the value that is dropped and the message that reports it cannot disagree.
pub fn portable_passphrase_is_unconfirmed(
    store_path: &std::path::Path,
    pass_text: &str,
    confirm_text: &str,
) -> bool {
    if pass_text.is_empty() {
        return false;
    }
    let confirmed = if store_path.exists() {
        confirm_text.is_empty() || confirm_text == pass_text
    } else {
        !confirm_text.is_empty() && confirm_text == pass_text
    };
    !confirmed
}

// Shared by the two branches that can report a weak passphrase, so the wording
// cannot drift between "weak" alone and "weak and unconfirmed".
fn passphrase_weakness_message(strength: rustconn_core::secret::PassphraseStrength) -> String {
    if matches!(
        strength,
        rustconn_core::secret::PassphraseStrength::TooShort
    ) {
        i18n(
            "A passphrase this short can be guessed quickly. This file is meant to be copied to your other computers, so the passphrase is the only thing protecting it.",
        )
    } else {
        i18n(
            "This passphrase would not take long to guess. Several unrelated words make a much stronger one, and are easier to remember than symbols.",
        )
    }
}

/// Creates the secrets settings page using AdwPreferencesPage
pub fn create_secrets_page() -> SecretsPageWidgets {
    let page = adw::PreferencesPage::builder()
        .title(i18n("Secrets"))
        .icon_name("dialog-password-symbolic")
        .build();

    // === Secret Backend Group ===
    let backend_group = adw::PreferencesGroup::builder()
        .title(i18n("Secret Backend"))
        .description(i18n("Choose how passwords are stored"))
        .build();

    // The selector's contents, order and index→backend mapping all come from
    // `backend_choices()`; see there for why they are one table.
    let choices = backend_choices();
    let backend_labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
    let backend_descriptions: Vec<String> = choices.iter().map(|c| c.description.clone()).collect();
    let backend_strings = StringList::new(&backend_labels);

    // An `AdwComboRow` rather than a `GtkDropDown` in a suffix: the row is what
    // has a subtitle to put the current backend's explanation in, and it is the
    // widget the project's own HIG notes ask for inside a preferences group.
    let secret_backend_dropdown = adw::ComboRow::builder()
        .title(i18n("Backend"))
        .model(&backend_strings)
        .selected(BACKEND_KEEPASSXC_INDEX)
        .build();
    secret_backend_dropdown.set_list_factory(Some(&backend_list_factory(backend_descriptions)));
    // The subtitle carries the current choice's explanation instead of a static
    // "Primary password storage method", which described the row rather than
    // anything the user had selected.
    sync_backend_subtitle(&secret_backend_dropdown);
    backend_group.add(&secret_backend_dropdown);

    // Version info row - shows version of selected backend
    let version_label = Label::builder()
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .build();
    let version_row = adw::ActionRow::builder().title(i18n("Version")).build();
    version_row.add_suffix(&version_label);
    backend_group.add(&version_row);

    // Whether the selected backend can actually store a password. Shown for
    // *every* backend, which is the change: it used to be revealed only when the
    // system keyring was selected, on the reasoning that "the other backends
    // already display their own status rows". Four of the eight had no status row
    // at all, two of the four that did got stuck on "Detecting..." when their CLI
    // was missing, and the row that existed answered a different question than
    // the Version row above it without saying so. So the page could show a
    // version number for a Bitwarden that was not logged in and call that the
    // whole report — which is how issue #312 got as far as it did.
    //
    // The label always names the state, so status is never conveyed by colour
    // alone (GNOME HIG / WCAG).
    let availability_label = Label::builder()
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .label(i18n("Checking..."))
        .css_classes(["dim-label"])
        .wrap(true)
        .max_width_chars(36)
        .xalign(1.0)
        .build();
    let availability_row = adw::ActionRow::builder().title(i18n("Status")).build();
    availability_row.add_suffix(&availability_label);
    backend_group.add(&availability_row);

    // An `AdwSwitchRow`, not a `GtkCheckButton` in a row prefix: a checkbox in a
    // boxed list is the odd one out on this page and against the project's HIG
    // notes.
    //
    // The subtitle names the encrypted file because that is what the fallback
    // actually is: `SecretManager::build_from_settings` appends
    // `EncryptedFileBackend`, and has since the libsecret fallback was dropped
    // for being useless when libsecret is the *failing* backend (#201). The text
    // still said "Use libsecret", and on macOS "Use the macOS Keychain" — naming
    // a store the code stopped using. It is also no longer platform-conditional,
    // because the encrypted file is the same on every platform.
    //
    // "Look for passwords in" rather than "Use … if unavailable" because this
    // switch now governs *reads* only. A write that the chosen backend refuses
    // asks the user where to put it instead of silently relocating it — see
    // `save_password_to_vault`.
    let enable_fallback = adw::SwitchRow::builder()
        .title(i18n("Also read from the encrypted file"))
        .subtitle(i18n(
            "Look for passwords in this computer's encrypted file too, so ones saved before you switched backend still resolve",
        ))
        .active(true)
        .build();
    backend_group.add(&enable_fallback);

    // Switching backend does not move anything, and until now the only thing that
    // could was a button inside the portable group that copied from one hardcoded
    // source. This sits at the group level because a transfer is between two
    // backends and belongs to neither.
    //
    // The handler needs the connection list, which the Secrets page has no access
    // to, so it is wired by `SettingsDialog::connect_credential_transfer` once the
    // dialog has the application state.
    let transfer_button = Button::builder()
        .label(i18n("Copy Passwords…"))
        .valign(gtk4::Align::Center)
        .build();
    let transfer_row = adw::ActionRow::builder()
        .title(i18n("Move between stores"))
        .subtitle(i18n(
            "Copy the passwords you already have from one store into another, for example into the portable file",
        ))
        .activatable_widget(&transfer_button)
        .build();
    transfer_row.add_suffix(&transfer_button);
    backend_group.add(&transfer_row);

    page.add(&backend_group);

    // Version info label — will be populated by async detection
    // Use placeholder defaults; real values arrive from background thread.
    let keepassxc_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let bitwarden_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let onepassword_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let passbolt_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let pass_version: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Cached secret-tool availability — populated by background detection thread.
    // `None` = not yet checked, `Some(true/false)` = result known.
    let secret_tool_available: Rc<RefCell<Option<bool>>> = Rc::new(RefCell::new(None));

    // Which backends the keyring could not supply a secret for. Seeded from the
    // saved configuration by `load_secret_settings` and cleared below whenever a
    // lookup actually returns a value, so the dialog's dirty check can tell
    // "the keyring already holds this" from "the keyring never got it".
    let keyring_gaps: Rc<Cell<KeyringGaps>> = Rc::new(Cell::new(KeyringGaps::default()));

    // Track whether async detection has completed
    let detection_complete: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

    // The whole detection result, kept so the Status row can be recomputed for
    // whichever backend is selected. This used to hold only the system keyring's
    // `BackendAvailability`, which is why the row could only speak about one
    // backend: the other seven probes were rendered into their own labels once
    // and then thrown away.
    let detection_result: Rc<RefCell<Option<SecretCliDetection>>> = Rc::new(RefCell::new(None));

    // Shared mutable command paths for callbacks (updated by async detection)
    let bitwarden_cmd: Rc<RefCell<String>> = Rc::new(RefCell::new("bw".to_string()));
    let onepassword_cmd: Rc<RefCell<String>> = Rc::new(RefCell::new("op".to_string()));

    // Initial version display — "Detecting..."
    version_label.set_text(&i18n("Detecting..."));
    version_label.add_css_class("dim-label");

    // === Bitwarden Configuration Group ===
    let bitwarden_group = adw::PreferencesGroup::builder()
        .title(i18n("Bitwarden"))
        .description(i18n("Configure Bitwarden CLI integration"))
        .build();

    // Password entry for unlocking (PasswordEntryRow: built-in peek icon,
    // caps-lock warning and focus on row click)
    let bitwarden_password_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Master Password"))
        .tooltip_text(i18n("Required to unlock vault"))
        .build();
    bitwarden_group.add(&bitwarden_password_entry);

    // Save password checkbox for Bitwarden (encrypted in settings file)
    let bitwarden_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector (replaces the previous pair of
    // "Save password" + "Save to system keyring" CheckButtons + mutual
    // exclusion logic). See `make_storage_combo` for behaviour.
    let bitwarden_storage_combo = make_storage_combo(
        &i18n("Save master password"),
        secret_tool_available.clone(),
        bitwarden_status_label.clone(),
    );
    bitwarden_group.add(&bitwarden_storage_combo);

    // API Key authentication switch. An `AdwSwitchRow` for the same reason as
    // `enable_fallback` above: a bare `GtkSwitch` in an `AdwActionRow` suffix is
    // the odd one out in a boxed list.
    let bitwarden_use_api_key_check = adw::SwitchRow::builder()
        .title(i18n("Use API key authentication"))
        .subtitle(i18n(
            "For automation or 2FA methods not supported by CLI (FIDO2, Duo)",
        ))
        .build();
    bitwarden_group.add(&bitwarden_use_api_key_check);

    // API Client ID entry
    let bitwarden_client_id_entry = Entry::builder()
        .placeholder_text(i18n("client_id"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let bw_client_id_row = adw::ActionRow::builder()
        .title(i18n("Client ID"))
        .subtitle(i18n(
            "From Bitwarden web vault → Settings → Security → Keys",
        ))
        .build();
    bw_client_id_row.add_suffix(&bitwarden_client_id_entry);
    bw_client_id_row.set_activatable_widget(Some(&bitwarden_client_id_entry));
    bitwarden_group.add(&bw_client_id_row);

    // API Client Secret entry
    let bitwarden_client_secret_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Client Secret"))
        .tooltip_text(i18n("Keep this secret safe"))
        .build();
    bitwarden_group.add(&bitwarden_client_secret_entry);

    // Setup visibility for API key fields
    let bw_client_id_row_clone = bw_client_id_row.clone();
    let bw_client_secret_entry_clone = bitwarden_client_secret_entry.clone();
    bitwarden_use_api_key_check.connect_active_notify(move |row| {
        let state = row.is_active();
        bw_client_id_row_clone.set_visible(state);
        bw_client_secret_entry_clone.set_visible(state);
    });

    // Initial visibility - hide API key fields by default
    bw_client_id_row.set_visible(false);
    bitwarden_client_secret_entry.set_visible(false);

    let bitwarden_unlock_button = Button::builder()
        .label(i18n("Unlock"))
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .tooltip_text(i18n("Unlock Bitwarden vault"))
        .build();

    let bw_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    bw_status_box.append(&bitwarden_status_label);
    bw_status_box.append(&bitwarden_unlock_button);

    let bw_status_row = adw::ActionRow::builder()
        .title(i18n("Vault Status"))
        .subtitle(i18n("Login with 'bw login' in terminal first"))
        .build();
    bw_status_row.add_suffix(&bw_status_box);
    bitwarden_group.add(&bw_status_row);

    // Connect unlock button
    {
        let status_label = bitwarden_status_label.clone();
        let password_entry = bitwarden_password_entry.clone();
        let bw_cmd = bitwarden_cmd.clone();
        let storage_combo = bitwarden_storage_combo.clone();
        bitwarden_unlock_button.connect_clicked(move |button| {
            let password_text = password_entry.text();
            let save_to_keyring =
                storage_combo_value(&storage_combo) == CredentialStorage::SystemKeyring;

            // Resolve password from keyring or text field, wrapping intermediate
            // plaintext copies in Zeroizing so they are wiped on drop
            // (M-PUBLIC-DEBUG / SecretString patterns).
            let password = if password_text.is_empty() && save_to_keyring {
                if let Some(val) = get_bw_password_from_keyring() {
                    use secrecy::ExposeSecret;
                    zeroize::Zeroizing::new(val.expose_secret().to_string())
                } else {
                    update_status_label(&status_label, &i18n("Enter password"), "warning");
                    return;
                }
            } else if password_text.is_empty() {
                update_status_label(&status_label, &i18n("Enter password"), "warning");
                return;
            } else {
                zeroize::Zeroizing::new(password_text.to_string())
            };

            button.set_sensitive(false);
            update_status_label(&status_label, &i18n("Unlocking..."), "dim-label");

            let bw_cmd_str = bw_cmd.borrow().clone();

            // Note: do not log password length — it leaks bruteforce metadata.
            tracing::debug!(
                bw_cmd = %bw_cmd_str,
                password_source = if password_text.is_empty() { "keyring" } else { "manual" },
                has_password = !password.is_empty(),
                "Bitwarden GUI: unlock button clicked"
            );

            // Run unlock asynchronously to avoid blocking the GTK main loop.
            let status_label_async = status_label.clone();
            let button_async = button.clone();
            // Both copies stay `Zeroizing`. `password` above is already
            // `Zeroizing`, and `password.to_string()` was quietly demoting it to a
            // bare `String` that then moved into two closures and dropped
            // unwiped — the master password left in freed heap, which is the one
            // thing the wrapper three statements up exists to prevent.
            let password_owned = zeroize::Zeroizing::new(password.to_string());
            let password_for_keyring = zeroize::Zeroizing::new(password.to_string());
            // `bw_cmd_str` is only logged now. The unlock itself no longer needs
            // it: core resolves the command through `get_bw_cmd()`, which is where
            // the Flatpak host lookup lives.
            glib::spawn_future_local(async move {
                let (session_result, raw_stderr) = gtk4::gio::spawn_blocking(move || {
                    // One call, not a hand-rolled --raw/verbose ladder. Core's
                    // `unlock_vault_blocking` runs the same two strategies plus a
                    // stdin fallback for older CLIs, and adds the three things this
                    // site was missing: the extended PATH a sandboxed `bw` needs,
                    // `--nointeraction`, and a deadline (issue #312).
                    // `.as_str()`, not `&password_owned`: `SecretString::from` is
                    // generic, and deref coercion does not apply through a generic
                    // bound, so a `Zeroizing<String>` has to be unwrapped.
                    match rustconn_core::secret::unlock_vault_blocking(&SecretString::from(
                        password_owned.as_str(),
                    )) {
                        Ok(session) => (Some(session), String::new()),
                        // The reason is matched against below to pick a message,
                        // so it has to survive. It carries `bw`'s stderr and never
                        // the password: the password reaches `bw` through the
                        // environment, so it cannot appear in an argv echoed back.
                        Err(e) => (None, e.to_string()),
                    }
                })
                .await
                .unwrap_or((None, String::new()));

                if let Some(session_key) = session_result {
                    // No length field. The handler above already declines to log
                    // the master password's length as bruteforce metadata, and a
                    // session key is no different in kind.
                    tracing::info!("Bitwarden GUI: unlock succeeded");
                    // Already a `SecretString` from core, so it is stored as it
                    // arrived. It used to come back as a bare `String` that this
                    // line re-wrapped, which left the key in freed heap.
                    set_session_key(session_key);
                    update_status_label(&status_label_async, &i18n("Unlocked"), "success");

                    if save_to_keyring {
                        save_bw_password_to_keyring(&password_for_keyring);
                    }
                } else {
                    tracing::warn!(
                        raw_stderr = %raw_stderr,
                        "Bitwarden GUI: unlock failed"
                    );
                    let msg = if raw_stderr.contains("Invalid master password") {
                        i18n("Invalid password")
                    } else if raw_stderr.contains("not logged in") {
                        i18n("Not logged in")
                    } else {
                        i18n("Unlock failed")
                    };
                    update_status_label(&status_label_async, &msg, "error");
                }

                button_async.set_sensitive(true);
            });
        });
    }

    page.add(&bitwarden_group);

    // === 1Password Configuration Group ===
    let onepassword_group = adw::PreferencesGroup::builder()
        .title(i18n("1Password"))
        .description(i18n("Configure 1Password CLI integration"))
        .build();

    // Service account token entry
    let onepassword_token_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Service Account Token"))
        .tooltip_text(i18n(
            "For headless/automated access (OP_SERVICE_ACCOUNT_TOKEN)",
        ))
        .build();
    onepassword_group.add(&onepassword_token_entry);

    // Save password checkbox (encrypted in settings file)
    let onepassword_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector for the 1Password service account
    // token (replaces the previous "Save token" + "Save to system keyring"
    // CheckButton pair plus mutual-exclusion logic).
    let onepassword_storage_combo = make_storage_combo(
        &i18n("Save token"),
        secret_tool_available.clone(),
        onepassword_status_label.clone(),
    );
    onepassword_group.add(&onepassword_storage_combo);

    let onepassword_signin_button = Button::builder()
        .label(i18n("Sign In"))
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .tooltip_text(i18n("Sign in to 1Password (opens terminal)"))
        .build();

    let op_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    op_status_box.append(&onepassword_status_label);
    op_status_box.append(&onepassword_signin_button);

    let op_status_row = adw::ActionRow::builder()
        .title(i18n("Account Status"))
        .subtitle(i18n(
            "Sign in with 'op signin' in terminal or use biometric unlock",
        ))
        .build();
    op_status_row.add_suffix(&op_status_box);
    onepassword_group.add(&op_status_row);

    // Connect signin button - opens terminal for interactive signin
    {
        let status_label = onepassword_status_label.clone();
        let op_cmd = onepassword_cmd.clone();
        onepassword_signin_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            update_status_label(&status_label, &i18n("Opening terminal..."), "dim-label");

            // Try to open a terminal with op signin
            // This requires user interaction for biometric or password
            let op_cmd_str = op_cmd.borrow().clone();
            let xfce_cmd = format!("{op_cmd_str} signin");
            let terminal_cmds: [(&str, Vec<&str>); 4] = [
                ("gnome-terminal", vec!["--", &op_cmd_str, "signin"]),
                ("konsole", vec!["-e", &op_cmd_str, "signin"]),
                ("xfce4-terminal", vec!["-e", &xfce_cmd]),
                ("xterm", vec!["-e", &op_cmd_str, "signin"]),
            ];

            let mut launched = false;
            for (term, args) in &terminal_cmds {
                if rustconn_core::which::is_available(term)
                    && std::process::Command::new(term)
                        .args(args.iter().copied())
                        .spawn()
                        .is_ok()
                {
                    launched = true;
                    update_status_label(&status_label, &i18n("Check terminal"), "warning");
                    break;
                }
            }

            if !launched {
                update_status_label(&status_label, &i18n("No terminal found"), "error");
            }

            button.set_sensitive(true);
        });
    }

    page.add(&onepassword_group);

    // === Passbolt Configuration Group ===
    let passbolt_group = adw::PreferencesGroup::builder()
        .title(i18n("Passbolt"))
        .description(i18n("Configure Passbolt CLI integration"))
        .build();

    // Server URL entry
    let passbolt_server_url_entry = Entry::builder()
        .placeholder_text("https://passbolt.example.org")
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let pb_url_row = adw::ActionRow::builder()
        .title(i18n("Server URL"))
        .subtitle(i18n("Passbolt web vault address"))
        .build();
    pb_url_row.add_suffix(&passbolt_server_url_entry);
    pb_url_row.set_activatable_widget(Some(&passbolt_server_url_entry));
    passbolt_group.add(&pb_url_row);

    // GPG Passphrase entry
    let passbolt_passphrase_entry = adw::PasswordEntryRow::builder()
        .title(i18n("GPG Passphrase"))
        .tooltip_text(i18n("Required to decrypt credentials from Passbolt"))
        .build();
    passbolt_group.add(&passbolt_passphrase_entry);

    // Save passphrase checkbox (encrypted in settings file)
    let passbolt_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector for the Passbolt GPG passphrase
    // (replaces the previous pair of CheckButtons + mutual-exclusion logic).
    let passbolt_storage_combo = make_storage_combo(
        &i18n("Save passphrase"),
        secret_tool_available.clone(),
        passbolt_status_label.clone(),
    );
    passbolt_group.add(&passbolt_storage_combo);

    let passbolt_open_vault_button = Button::builder()
        .label(i18n("Open Vault"))
        .valign(gtk4::Align::Center)
        .sensitive(false)
        .tooltip_text(i18n("Open Passbolt web vault in browser"))
        .build();

    let pb_status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    pb_status_box.append(&passbolt_status_label);
    pb_status_box.append(&passbolt_open_vault_button);

    let pb_status_row = adw::ActionRow::builder()
        .title(i18n("Server Status"))
        .subtitle(i18n("Configure with 'passbolt configure' in terminal"))
        .build();
    pb_status_row.add_suffix(&pb_status_box);
    passbolt_group.add(&pb_status_row);

    // Connect Open Vault button
    {
        let url_entry = passbolt_server_url_entry.clone();
        let status_label = passbolt_status_label.clone();
        passbolt_open_vault_button.connect_clicked(move |_| {
            let url_text = url_entry.text();
            let url = if url_text.is_empty() {
                // Try reading from CLI config as fallback
                read_passbolt_server_url_sync()
            } else {
                Some(url_text.to_string())
            };

            if let Some(ref server_url) = url {
                let result = std::process::Command::new(rustconn_core::secret::url_open_command())
                    .arg(server_url)
                    .spawn();
                if result.is_err() {
                    update_status_label(&status_label, &i18n("Failed to open browser"), "error");
                }
            } else {
                update_status_label(
                    &status_label,
                    &i18n("Enter server URL or run 'passbolt configure'"),
                    "warning",
                );
            }
        });
    }

    page.add(&passbolt_group);

    // === Pass (Unix Password Manager) Group ===
    let pass_group = adw::PreferencesGroup::builder()
        .title(i18n("Pass"))
        .description(i18n("Configure Pass (passwordstore.org) integration"))
        .build();

    // Store directory entry with browse button
    let pass_store_dir_entry = Entry::builder()
        .placeholder_text(i18n("~/.password-store"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();

    let pass_store_dir_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Choose password store directory"))
        .build();
    pass_store_dir_browse_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Choose password store directory",
    ))]);

    let pass_dir_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    pass_dir_box.append(&pass_store_dir_entry);
    pass_dir_box.append(&pass_store_dir_browse_button);

    let pass_dir_row = adw::ActionRow::builder()
        .title(i18n("Store Directory"))
        .subtitle(i18n("Location of password-store (leave empty for default)"))
        .build();
    pass_dir_row.add_suffix(&pass_dir_box);
    pass_group.add(&pass_dir_row);

    // Status label showing initialization status
    let pass_status_label = Label::builder()
        .label(i18n("Detecting..."))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();

    let pass_status_row = adw::ActionRow::builder()
        .title(i18n("Initialization Status"))
        .subtitle(i18n("Run 'pass init &lt;gpg-id&gt;' to initialize"))
        .build();
    pass_status_row.add_suffix(&pass_status_label);
    pass_group.add(&pass_status_row);

    // Setup browse button for pass store directory
    {
        let entry = pass_store_dir_entry.clone();
        pass_store_dir_browse_button.connect_clicked(move |button| {
            let entry_clone = entry.clone();
            let dialog = FileDialog::builder()
                .title(i18n("Select Password Store Directory"))
                .modal(true)
                .build();

            if let Some(window) = button
                .root()
                .and_then(|r| r.downcast::<gtk4::Window>().ok())
            {
                dialog.select_folder(Some(&window), gtk4::gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result {
                        let path = file.path();
                        if let Some(p) = path {
                            entry_clone.set_text(&p.to_string_lossy());
                        }
                    }
                });
            }
        });
    }

    page.add(&pass_group);

    // === Machine-Bound Encrypted File Group ===
    //
    // This backend has nothing to configure — the path is fixed and the key comes
    // from the machine — but selecting it used to show *nothing at all*, so there
    // was no way to tell the choice had registered, where the file was, or how it
    // differs from the portable one right below it.
    let encrypted_file_group = adw::PreferencesGroup::builder()
        .title(i18n("Encrypted File"))
        .description(i18n(
            "Credentials in a file on this computer. The key is derived from this machine, so the file cannot be opened anywhere else — and cannot be recovered if the machine is lost.",
        ))
        .build();

    let encrypted_file_path = rustconn_core::secret::default_encrypted_store_path();
    let encrypted_path_row = adw::ActionRow::builder()
        .title(i18n("File path"))
        .subtitle(encrypted_file_path.display().to_string())
        .subtitle_selectable(true)
        .activatable(false)
        .build();
    encrypted_file_group.add(&encrypted_path_row);

    let (encrypted_status_row, encrypted_status_label) = credential_file_status_row();
    encrypted_file_group.add(&encrypted_status_row);
    {
        let path = encrypted_file_path.clone();
        refresh_credential_file_status(&encrypted_status_label, path, |p| {
            rustconn_core::secret::migration::encrypted_entry_count(p).map_err(|e| e.to_string())
        });
    }

    page.add(&encrypted_file_group);

    // === Portable Encrypted File Group ===
    let portable_group = adw::PreferencesGroup::builder()
        .title(i18n("Portable Encrypted File"))
        .description(i18n(
            "Credentials in a file encrypted with a passphrase you choose. Put it in a folder your cloud client syncs and the same file opens on your other computers. There is no way to recover the passphrase, so it is asked for twice.",
        ))
        .build();

    // File path row.
    //
    // The placeholder is the real default path rather than a description of one:
    // the entry is empty until the user types something, and the location that
    // was actually in use — `~/.local/share/rustconn/credentials-portable.enc` —
    // appeared nowhere in the interface at all.
    let default_portable_path = rustconn_core::secret::resolve_portable_store_path(None);
    let portable_path_entry = Entry::builder()
        .placeholder_text(default_portable_path.display().to_string())
        .hexpand(true)
        .build();
    let portable_browse_button = Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(i18n("Browse"))
        .valign(gtk4::Align::Center)
        .build();
    portable_browse_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for portable credential file",
    ))]);
    let portable_path_box = GtkBox::new(gtk4::Orientation::Horizontal, 6);
    portable_path_box.append(&portable_path_entry);
    portable_path_box.append(&portable_browse_button);
    // The previous subtitle, "Can be a cloud-synced directory", asked for a
    // directory. A directory is accepted here without complaint and then fails at
    // the rename on first write, as a generic "cannot save the password".
    let portable_path_row = adw::ActionRow::builder()
        .title(i18n("File path"))
        .subtitle(i18n("Name a file inside a folder your cloud client syncs. Leave empty for the default location."))
        .build();
    portable_path_row.add_suffix(&portable_path_box);
    portable_group.add(&portable_path_row);

    let (portable_file_status_row, portable_file_status_label) = credential_file_status_row();
    portable_group.add(&portable_file_status_row);

    // Tracks the entry rather than the saved setting, so the row answers "is
    // there a file where I just pointed this?" while the path is being typed.
    let refresh_portable_status = {
        let entry = portable_path_entry.clone();
        let label = portable_file_status_label.clone();
        move || {
            let path = rustconn_core::secret::resolve_portable_store_path(
                expand_user_path(entry.text().as_str()).as_deref(),
            );
            refresh_credential_file_status(&label, path, |p| {
                rustconn_core::secret::portable_entry_count(p).map_err(|e| e.to_string())
            });
        }
    };
    refresh_portable_status();
    {
        let refresh = refresh_portable_status.clone();
        portable_path_entry.connect_changed(move |_| refresh());
    }

    // Browse button: pick an existing store, or name a new one.
    //
    // The mode depends on whether the target is already there, because the two
    // situations are genuinely different. On the first machine the file does not
    // exist yet and has to be named, which only a save dialog allows. On the
    // second machine the file arrived over the cloud sync and is merely being
    // pointed at — a save dialog would answer that with an "already exists,
    // replace it?" warning about the very file the user is trying to keep.
    {
        let entry = portable_path_entry.clone();
        portable_browse_button.connect_clicked(move |button| {
            let entry_clone = entry.clone();
            let current = entry.text();
            let existing = expand_user_path(current.as_str()).is_some_and(|path| path.is_file());

            let Some(window) = button
                .root()
                .and_then(|r| r.downcast::<gtk4::Window>().ok())
            else {
                return;
            };

            let dialog = FileDialog::builder()
                .title(i18n("Select Portable Credential File"))
                .modal(true)
                .build();

            let on_chosen = move |result: Result<gtk4::gio::File, glib::Error>| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry_clone.set_text(&path.to_string_lossy());
                }
            };

            if existing {
                dialog.open(Some(&window), gtk4::gio::Cancellable::NONE, on_chosen);
            } else {
                dialog.set_initial_name(Some(rustconn_core::secret::PORTABLE_STORE_FILE_NAME));
                dialog.save(Some(&window), gtk4::gio::Cancellable::NONE, on_chosen);
            }
        });
    }

    // Passphrase row, plus a confirmation.
    //
    // The confirmation is not ceremony here. This passphrase is the only key to
    // every credential in the file, it is never checked against anything when
    // the file is first created, and there is no recovery path: a typo produces
    // a store that opens with a passphrase the user does not know they typed.
    let portable_passphrase_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Passphrase"))
        .build();
    portable_group.add(&portable_passphrase_entry);

    let portable_confirm_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Confirm passphrase"))
        .build();
    portable_group.add(&portable_confirm_entry);

    // The passphrase validator gets a row of its own, rather than sharing the
    // group's status row with everything else.
    //
    // It runs on every keystroke and clears the row whenever it has no complaint,
    // so while it shared the slot it wiped whatever else had just been put there:
    // the "System keyring unavailable" warning that explains a reverted choice,
    // the path reported by Create File, the outcome of a credential copy. One
    // keystroke and the message the user needed was gone. Splitting the slots is
    // what makes each writer's message the writer's to clear.
    let portable_validation_label = Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    let portable_validation_row = adw::ActionRow::builder().activatable(false).build();
    portable_validation_row.add_prefix(&portable_validation_label);
    portable_validation_label
        .bind_property("visible", &portable_validation_row, "visible")
        .sync_create()
        .build();
    portable_group.add(&portable_validation_row);

    let portable_status_label = Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    let portable_status_row = adw::ActionRow::builder().activatable(false).build();
    portable_status_row.add_prefix(&portable_status_label);
    // The row follows the label rather than being toggled alongside it.
    //
    // Hiding only the label left the row in the boxed list as a full-height
    // empty band between "Confirm passphrase" and "Save passphrase", because
    // hiding a prefix widget does not remove the row that holds it. Every
    // writer of this label — the passphrase validator, the migration handler
    // and `update_status_label` via `make_storage_combo` — would have had to
    // remember to hide the row too, and the third one is not even in this file.
    // A binding makes the empty row unrepresentable instead.
    portable_status_label
        .bind_property("visible", &portable_status_row, "visible")
        .sync_create()
        .build();
    portable_group.add(&portable_status_row);

    // Live mismatch feedback, so the error is visible while typing rather than
    // discovered when the credentials cannot be read back.
    {
        let passphrase = portable_passphrase_entry.clone();
        let confirm = portable_confirm_entry.clone();
        let status = portable_validation_label.clone();
        let path_entry = portable_path_entry.clone();
        let update = move || {
            let pass_text = passphrase.text();
            let confirm_text = confirm.text();
            let mismatch = !confirm_text.is_empty() && pass_text != confirm_text;
            // A store that does not exist yet cannot check the passphrase, so the
            // confirmation is required rather than optional — say so here instead
            // of letting Save quietly drop it.
            let creating_new_store = !rustconn_core::secret::resolve_portable_store_path(
                expand_user_path(path_entry.text().as_str()).as_deref(),
            )
            .exists();
            let unconfirmed_new =
                creating_new_store && !pass_text.is_empty() && confirm_text.is_empty();

            // Strength is only assessed for a store being created. For one that
            // already exists this entry is how it is *opened*, and the passphrase
            // it wants is whatever it was created with — telling the user it is
            // weak there would be a complaint about a decision they can no longer
            // change from this field, on the way to unlocking their own file.
            //
            // The verdict is deliberately not logged: `strength = ?s` in a
            // tracing field records "this user's passphrase is weak", which helps
            // exactly one audience.
            let weakness = if creating_new_store {
                Some(rustconn_core::secret::assess_passphrase(pass_text.as_str()))
                    .filter(|strength| strength.deserves_a_warning())
            } else {
                None
            };

            if mismatch {
                status.set_label(&i18n("The two passphrases do not match"));
                status.add_css_class("error");
                status.remove_css_class("warning");
                status.set_visible(true);
                confirm.add_css_class("error");
            } else if unconfirmed_new {
                // Checked before the strength verdict, not after. The verdict used
                // to rank higher, on the reasoning that there is no point
                // confirming a passphrase that should be replaced — but it is
                // advice, phrased as advice, while this is the one condition that
                // makes Save drop the passphrase. Ranking advice above the blocker
                // meant a weak *and* unconfirmed passphrase showed only the
                // advice, and Save then discarded it with nothing on screen having
                // said it would. Both are shown instead, requirement first.
                let mut message = i18n(
                    "Repeat the passphrase to confirm it. A new file cannot be recovered if the passphrase is wrong.",
                );
                if let Some(strength) = weakness {
                    message.push('\n');
                    message.push_str(&passphrase_weakness_message(strength));
                }
                status.set_label(&message);
                status.add_css_class("error");
                status.remove_css_class("warning");
                status.set_visible(true);
                confirm.remove_css_class("error");
            } else if let Some(strength) = weakness {
                status.set_label(&passphrase_weakness_message(strength));
                status.remove_css_class("error");
                status.add_css_class("warning");
                status.set_visible(true);
                confirm.remove_css_class("error");
            } else {
                status.set_visible(false);
                status.remove_css_class("error");
                status.remove_css_class("warning");
                confirm.remove_css_class("error");
            }
        };
        // The path matters to this check, not just the two passphrase fields:
        // whether the confirmation is required at all depends on the store
        // already existing. Without this the requirement could appear silently —
        // type the passphrase while the path points at an existing file, then
        // point it at a new one, and Save would drop the passphrase while the
        // label still showed the state from before the path changed.
        let on_pass = update.clone();
        let on_path = update.clone();
        portable_passphrase_entry.connect_changed(move |_| on_pass());
        portable_path_entry.connect_changed(move |_| on_path());
        portable_confirm_entry.connect_changed(move |_| update());
    }

    // Storage combo (how to persist the passphrase locally).
    //
    // The group's own status label, not a fresh one: `make_storage_combo` writes
    // "System keyring unavailable" into whatever it is handed, and a `Label` that
    // is never added to a container puts that warning nowhere. The combo would
    // then revert the user's choice with no explanation.
    let portable_storage_combo = make_storage_combo(
        &i18n("Save passphrase"),
        Rc::clone(&secret_tool_available),
        portable_status_label.clone(),
    );
    portable_group.add(&portable_storage_combo);

    // Create the file as its own step.
    //
    // Until now the file appeared as a side effect of the first credential save,
    // so a fresh setup could not be confirmed or corrected: a mistyped directory
    // surfaced much later as "could not save the password". The other thing that
    // created it, "Copy Credentials", refuses when there is nothing to copy —
    // exactly the state a new installation is in. On a second machine the same
    // button checks that the passphrase opens the file the sync client delivered,
    // without rewriting it.
    let portable_create_button = Button::builder()
        .label(i18n("Create File"))
        .halign(gtk4::Align::Start)
        .valign(gtk4::Align::Center)
        .build();
    let portable_create_row = adw::ActionRow::builder()
        .title(i18n("Set up the file"))
        .subtitle(i18n(
            "Create the file now, or check that your passphrase opens one that is already there",
        ))
        .activatable_widget(&portable_create_button)
        .build();
    portable_create_row.add_suffix(&portable_create_button);
    portable_group.add(&portable_create_row);

    {
        let passphrase_entry = portable_passphrase_entry.clone();
        let confirm_entry = portable_confirm_entry.clone();
        let path_entry = portable_path_entry.clone();
        let status = portable_status_label.clone();
        let refresh = refresh_portable_status.clone();
        portable_create_button.connect_clicked(move |button| {
            let path = rustconn_core::secret::resolve_portable_store_path(
                expand_user_path(path_entry.text().as_str()).as_deref(),
            );

            let passphrase =
                match portable_passphrase_for_action(&passphrase_entry, &confirm_entry, &path) {
                    Ok(passphrase) => passphrase,
                    Err(message) => {
                        update_status_label(&status, &message, "error");
                        passphrase_entry.grab_focus();
                        return;
                    }
                };

            // On a blocking thread: this is an Argon2id derivation with the
            // file's own parameters, and for an existing file those come from a
            // shared folder. The Settings dialog froze for the duration when the
            // passphrase check ran inline, which is the bug this avoids repeating.
            update_status_label(&status, &i18n("Setting up the file…"), "dim-label");
            button.set_sensitive(false);

            let status_async = status.clone();
            let button_async = button.clone();
            let refresh_async = refresh.clone();
            let work_path = path.clone();
            glib::spawn_future_local(async move {
                let outcome = gtk4::gio::spawn_blocking(move || {
                    rustconn_core::secret::prepare_portable_store(&work_path, &passphrase)
                })
                .await;
                button_async.set_sensitive(true);

                let (message, css) = match outcome {
                    Ok(Ok(rustconn_core::secret::PortableStoreSetup::Created)) => (
                        i18n_f("Created {}", &[&path.display().to_string()]),
                        "success",
                    ),
                    Ok(Ok(rustconn_core::secret::PortableStoreSetup::AlreadyUsable)) => (
                        i18n("That file is already there and your passphrase opens it"),
                        "success",
                    ),
                    Ok(Err(rustconn_core::error::SecretError::IncorrectPassphrase)) => (
                        i18n(
                            "That passphrase does not open the existing portable file. Enter the passphrase the file was created with.",
                        ),
                        "error",
                    ),
                    Ok(Err(e)) => (
                        i18n_f("Cannot set up the file: {}", &[&e.to_string()]),
                        "error",
                    ),
                    Err(_panic) => (i18n("Could not set up the file"), "error"),
                };
                update_status_label(&status_async, &message, css);
                refresh_async();
            });
        });
    }

    // Change the passphrase.
    //
    // Unwired here: re-keying the file is only half of a passphrase change. The
    // other half is replacing every copy of the old one — the session settings,
    // the live backend, the remembered copy — and none of those are reachable from
    // this page. `SettingsDialog::connect_portable_passphrase_change` does it.
    let portable_change_passphrase_button = Button::builder()
        .label(i18n("Change Passphrase…"))
        .halign(gtk4::Align::Start)
        .valign(gtk4::Align::Center)
        .build();
    let portable_change_passphrase_row = adw::ActionRow::builder()
        .title(i18n("Passphrase"))
        .subtitle(i18n(
            "Choose a new passphrase and encrypt every password in the file again under it",
        ))
        .activatable_widget(&portable_change_passphrase_button)
        .build();
    portable_change_passphrase_row.add_suffix(&portable_change_passphrase_button);
    portable_group.add(&portable_change_passphrase_row);

    // Migrate button row
    let portable_migrate_button = Button::builder()
        .label(i18n("Copy Credentials"))
        .tooltip_text(i18n(
            "Copy passwords from the machine-bound encrypted file into this portable file",
        ))
        .halign(gtk4::Align::Start)
        .valign(gtk4::Align::Center)
        .build();
    let portable_migrate_row = adw::ActionRow::builder()
        .title(i18n("Existing passwords"))
        .subtitle(i18n(
            "Re-encrypt credentials from the machine-bound file with your passphrase",
        ))
        .activatable_widget(&portable_migrate_button)
        .build();
    portable_migrate_row.add_suffix(&portable_migrate_button);
    portable_group.add(&portable_migrate_row);

    // Wire up migrate button
    {
        let passphrase_entry_clone = portable_passphrase_entry.clone();
        let confirm_entry_clone = portable_confirm_entry.clone();
        let path_entry_clone = portable_path_entry.clone();
        let status_clone = portable_status_label.clone();
        let refresh_status_clone = refresh_portable_status.clone();
        portable_migrate_button.connect_clicked(move |button| {
            let source_path = rustconn_core::secret::default_encrypted_store_path();
            let dest_path = rustconn_core::secret::resolve_portable_store_path(
                expand_user_path(path_entry_clone.text().as_str()).as_deref(),
            );

            // Refuse rather than flash: the reason has to stay on screen, since
            // the fix (type the passphrase, make both fields agree) is not
            // obvious from a border that recolours for two seconds.
            let passphrase = match portable_passphrase_for_action(
                &passphrase_entry_clone,
                &confirm_entry_clone,
                &dest_path,
            ) {
                Ok(passphrase) => passphrase,
                Err(message) => {
                    update_status_label(&status_clone, &message, "error");
                    passphrase_entry_clone.grab_focus();
                    return;
                }
            };

            let Some(prefs_dialog) = enclosing_preferences_dialog(button) else {
                tracing::warn!("Migration button is not inside a preferences dialog");
                return;
            };

            // Nothing to copy is worth saying out loud — the button being
            // enabled implies there might be.
            let entry_count = rustconn_core::secret::migration::encrypted_entry_count(&source_path)
                .unwrap_or_default();
            if entry_count == 0 {
                show_toast(&prefs_dialog, &i18n("No stored passwords to copy"), 3);
                return;
            }

            // Appending under the wrong passphrase would produce a file whose
            // halves need two different ones. The core store refuses that, but
            // checking first turns a failed migration into a clear message.
            //
            // On a blocking thread, like the unlock dialog does it: the check is
            // an Argon2id derivation with the *file's* parameters, and the file
            // comes from a shared folder. Even with the header's cost ceilings
            // that is long enough to freeze the Settings dialog, and running it
            // on the GTK main thread was the one place in this feature that did.
            let verify_path = dest_path.clone();
            let verify_pass = passphrase.clone();
            let status_async = status_clone.clone();
            let button_async = button.clone();
            let refresh_async = refresh_status_clone.clone();
            update_status_label(&status_clone, &i18n("Checking the passphrase…"), "dim-label");
            button.set_sensitive(false);

            glib::spawn_future_local(async move {
                let outcome = gtk4::gio::spawn_blocking(move || {
                    rustconn_core::secret::verify_portable_passphrase(&verify_path, &verify_pass)
                })
                .await;
                button_async.set_sensitive(true);

                let failure = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(
                        if matches!(e, rustconn_core::error::SecretError::IncorrectPassphrase) {
                            // One line on purpose: xgettext does not collapse
                            // Rust's `\<newline>`, so a wrapped literal reaches
                            // the catalogue with the source indentation and never
                            // matches at runtime.
                            i18n(
                                "That passphrase does not open the existing portable file. Enter the passphrase the file was created with.",
                            )
                        } else {
                            i18n_f("Cannot open the portable file: {}", &[&e.to_string()])
                        },
                    ),
                    Err(_join_err) => Some(i18n("Could not check the passphrase")),
                };

                if let Some(message) = failure {
                    update_status_label(&status_async, &message, "error");
                    return;
                }
                status_async.set_visible(false);

                crate::dialogs::portable_migration::show_migration_wizard(
                    &button_async,
                    entry_count,
                    move |response| {
                        if response
                            == crate::dialogs::portable_migration::MigrationResponse::Transfer
                        {
                            let refresh_after = refresh_async.clone();
                            crate::dialogs::portable_migration::run_migration(
                                source_path.clone(),
                                dest_path.clone(),
                                passphrase.clone(),
                                prefs_dialog.clone(),
                                // The file row would otherwise keep reporting the
                                // count from before the copy, which is the state
                                // it exists to stop being invisible.
                                move || refresh_after(),
                            );
                        }
                    },
                );
            });
        });
    }

    page.add(&portable_group);

    // === KeePass Database Group ===
    let kdbx_group = adw::PreferencesGroup::builder()
        .title(i18n("KeePass Database"))
        .description(i18n(
            "Configure KDBX file integration (works with KeePassXC, GNOME Secrets, etc.)",
        ))
        .build();

    let kdbx_enabled_row = adw::SwitchRow::builder()
        .title(i18n("KDBX Integration"))
        .subtitle(i18n("Enable direct database access"))
        .build();
    kdbx_group.add(&kdbx_enabled_row);

    // Database path with browse button
    let kdbx_path_entry = Entry::builder()
        .placeholder_text(i18n("Select .kdbx file"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let kdbx_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Browse for database file"))
        .build();
    kdbx_browse_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for KeePass database file",
    ))]);
    let kdbx_path_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    kdbx_path_box.append(&kdbx_path_entry);
    kdbx_path_box.append(&kdbx_browse_button);

    let kdbx_path_row = adw::ActionRow::builder()
        .title(i18n("Database File"))
        .build();
    kdbx_path_row.add_suffix(&kdbx_path_box);
    kdbx_group.add(&kdbx_path_row);

    page.add(&kdbx_group);

    // === Authentication Group ===
    let auth_group = adw::PreferencesGroup::builder()
        .title(i18n("Authentication"))
        .description(i18n("Database unlock methods"))
        .build();

    // Use password switch — an `AdwSwitchRow`, like the rest of this page.
    let kdbx_use_password_check = adw::SwitchRow::builder()
        .title(i18n("Use password"))
        .active(true)
        .build();
    auth_group.add(&kdbx_use_password_check);

    // Password entry (the row itself; `password_row` aliases it for the
    // visibility toggling driven by the "Use password" switch)
    let kdbx_password_entry = adw::PasswordEntryRow::builder()
        .title(i18n("Database password"))
        .build();
    let password_row = kdbx_password_entry.clone();
    auth_group.add(&kdbx_password_entry);

    // Save password checkbox
    let kdbx_status_label = Label::builder()
        .label(i18n("Not connected"))
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        // Long error strings (e.g. "Invalid database password or key file")
        // must not overflow the row next to the Check button (#182). Cap the
        // width and ellipsize; update_status_label sets the full text as a
        // tooltip so nothing is lost.
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .css_classes(["dim-label"])
        .build();

    // 3-state credential storage selector for the KeePassXC database
    // password (replaces the previous pair of CheckButtons + mutual-exclusion
    // logic).
    let kdbx_storage_combo = make_storage_combo(
        &i18n("Save password"),
        secret_tool_available.clone(),
        kdbx_status_label.clone(),
    );
    auth_group.add(&kdbx_storage_combo);

    // Use key file switch — an `AdwSwitchRow`, like the rest of this page.
    let kdbx_use_key_file_check = adw::SwitchRow::builder()
        .title(i18n("Use key file"))
        .build();
    auth_group.add(&kdbx_use_key_file_check);

    // Key file path with browse button
    let kdbx_key_file_entry = Entry::builder()
        .placeholder_text(i18n("Select .keyx or .key file"))
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let kdbx_key_file_browse_button = Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Browse for key file"))
        .build();
    kdbx_key_file_browse_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Browse for key file",
    ))]);
    let key_file_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(gtk4::Align::Center)
        .build();
    key_file_box.append(&kdbx_key_file_entry);
    key_file_box.append(&kdbx_key_file_browse_button);

    let key_file_row = adw::ActionRow::builder().title(i18n("Key File")).build();
    key_file_row.add_suffix(&key_file_box);
    auth_group.add(&key_file_row);

    page.add(&auth_group);

    // === Status Group ===
    let status_group = adw::PreferencesGroup::builder()
        .title(i18n("KDBX Status"))
        .build();

    // Check connection button
    let kdbx_check_button = Button::builder()
        .label(i18n("Check"))
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n("Test database connection"))
        .build();

    let status_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .valign(gtk4::Align::Center)
        .build();
    status_box.append(&kdbx_status_label);
    status_box.append(&kdbx_check_button);

    let status_row = adw::ActionRow::builder()
        .title(i18n("Connection Status"))
        .build();
    status_row.add_suffix(&status_box);
    status_group.add(&status_row);

    page.add(&status_group);

    // Setup visibility connections for password fields. The storage combo
    // tracks the password row, hidden when password auth is disabled.
    let password_row_clone = password_row.clone();
    let kdbx_storage_combo_clone = kdbx_storage_combo.clone();
    kdbx_use_password_check.connect_active_notify(move |row| {
        let state = row.is_active();
        password_row_clone.set_visible(state);
        kdbx_storage_combo_clone.set_visible(state);
    });

    // Setup visibility connections for key file fields
    let key_file_row_clone = key_file_row.clone();
    kdbx_use_key_file_check.connect_active_notify(move |row| {
        key_file_row_clone.set_visible(row.is_active());
    });

    // Setup visibility for KeePass sections when integration is enabled/disabled
    let auth_group_clone = auth_group.clone();
    let status_group_clone = status_group.clone();
    kdbx_enabled_row.connect_active_notify(move |row| {
        let state = row.is_active();
        auth_group_clone.set_visible(state);
        status_group_clone.set_visible(state);
    });

    // Setup visibility for Bitwarden, 1Password, Passbolt, and Pass groups based on backend
    // Indices: 0=KeePassXC, 1=libsecret, 2=Bitwarden, 3=1Password, 4=Passbolt, 5=Pass
    let bitwarden_group_clone = bitwarden_group.clone();
    let onepassword_group_clone = onepassword_group.clone();
    let passbolt_group_clone = passbolt_group.clone();
    let pass_group_clone = pass_group.clone();
    let portable_group_clone = portable_group.clone();
    let encrypted_file_group_clone = encrypted_file_group.clone();
    let kdbx_group_clone = kdbx_group.clone();
    let auth_group_clone2 = auth_group.clone();
    let status_group_clone2 = status_group.clone();
    let kdbx_enabled_row_clone = kdbx_enabled_row.clone();
    let version_label_clone = version_label.clone();
    let version_row_clone = version_row.clone();
    let keepassxc_version_clone = keepassxc_version.clone();
    let bitwarden_version_clone = bitwarden_version.clone();
    let onepassword_version_clone = onepassword_version.clone();
    let passbolt_version_clone = passbolt_version.clone();
    let pass_version_clone = pass_version.clone();
    let detection_complete_clone = detection_complete.clone();
    let availability_label_clone = availability_label.clone();
    let detection_result_clone = detection_result.clone();
    // The two prerequisites the probes cannot see, read live so the Status row
    // follows what is in the fields rather than what was last saved.
    let kdbx_path_entry_status = kdbx_path_entry.clone();
    let portable_passphrase_status = portable_passphrase_entry.clone();
    // Clones for on-demand keyring loading when user switches backend
    let bw_status_label_switch = bitwarden_status_label.clone();
    let op_token_entry_switch = onepassword_token_entry.clone();
    let op_status_label_switch = onepassword_status_label.clone();
    let pb_passphrase_entry_switch = passbolt_passphrase_entry.clone();
    let kdbx_password_entry_switch = kdbx_password_entry.clone();
    let keyring_gaps_switch = keyring_gaps.clone();
    secret_backend_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        // The two file backends are told apart only by this line once the popup
        // has closed.
        sync_backend_subtitle(dropdown);

        bitwarden_group_clone.set_visible(selected == BACKEND_BITWARDEN_INDEX);
        onepassword_group_clone.set_visible(selected == BACKEND_ONEPASSWORD_INDEX);
        passbolt_group_clone.set_visible(selected == BACKEND_PASSBOLT_INDEX);
        pass_group_clone.set_visible(selected == BACKEND_PASS_INDEX);
        encrypted_file_group_clone.set_visible(selected == BACKEND_ENCRYPTED_FILE_INDEX);
        portable_group_clone.set_visible(selected == BACKEND_PORTABLE_INDEX);
        let show_kdbx = selected == BACKEND_KEEPASSXC_INDEX;
        kdbx_group_clone.set_visible(show_kdbx);
        // Auth and status groups depend on both backend selection and kdbx_enabled
        let kdbx_enabled = kdbx_enabled_row_clone.is_active();
        auth_group_clone2.set_visible(show_kdbx && kdbx_enabled);
        status_group_clone2.set_visible(show_kdbx && kdbx_enabled);

        // Status: can the backend just selected actually store a password. Every
        // backend answers, which is what makes choosing from this list informed
        // rather than hopeful.
        render_backend_readiness(
            &availability_label_clone,
            detection_result_clone.borrow().as_ref(),
            index_to_backend(selected),
            &LocalBackendState {
                kdbx_enabled,
                kdbx_path: expand_user_path(kdbx_path_entry_status.text().as_str()),
                portable_passphrase_entered: !portable_passphrase_status.text().is_empty(),
            },
        );

        // Helper to set version label text and style
        let detected = *detection_complete_clone.borrow();
        let set_ver = |ver: &Option<String>| {
            version_row_clone.set_visible(true);
            version_label_clone.remove_css_class("error");
            version_label_clone.remove_css_class("success");
            version_label_clone.remove_css_class("dim-label");
            if let Some(ref v) = *ver {
                version_label_clone.set_text(&format!("v{v}"));
                version_label_clone.add_css_class("success");
            } else if detected {
                version_label_clone.set_text(&i18n("Not installed"));
                version_label_clone.add_css_class("error");
            } else {
                version_label_clone.set_text(&i18n("Detecting..."));
                version_label_clone.add_css_class("dim-label");
            }
        };

        // Update version label based on selected backend
        match selected {
            BACKEND_KEEPASSXC_INDEX => set_ver(&keepassxc_version_clone.borrow()),
            BACKEND_SYSTEM_KEYRING_INDEX => version_row_clone.set_visible(false),
            BACKEND_BITWARDEN_INDEX => set_ver(&bitwarden_version_clone.borrow()),
            BACKEND_ONEPASSWORD_INDEX => set_ver(&onepassword_version_clone.borrow()),
            BACKEND_PASSBOLT_INDEX => set_ver(&passbolt_version_clone.borrow()),
            BACKEND_PASS_INDEX => set_ver(&pass_version_clone.borrow()),
            _ => version_row_clone.set_visible(false),
        }

        // On-demand keyring loading when user switches to a new backend
        match selected {
            BACKEND_BITWARDEN_INDEX => {
                // Bitwarden selected — trigger auto-unlock from keyring
                let status_label = bw_status_label_switch.clone();
                let gaps = keyring_gaps_switch.clone();
                glib::spawn_future_local(async move {
                    let result = gtk4::gio::spawn_blocking(move || {
                        // No `ExposeSecret` here: the master password goes to core
                        // as a `SecretString` and is never unwrapped in the GUI.
                        let bw_cmd = rustconn_core::secret::get_bw_cmd();
                        let password = get_bw_password_from_keyring();
                        let password = password?;
                        let bw_status = check_bitwarden_status_sync(&bw_cmd);
                        if !bw_status.should_try_unlock() {
                            let (text, css) = bw_status.to_status_pair();
                            return Some((text, css, None));
                        }
                        match rustconn_core::secret::unlock_vault_blocking(&password) {
                            Ok(session_key) => {
                                let (text, css) = BwVaultStatus::Unlocked.to_status_pair();
                                Some((text, css, Some(session_key)))
                            }
                            Err(e) => {
                                // Never silent: the whole point of #312 is that a
                                // skipped or failed auto-unlock left no trace.
                                tracing::warn!(
                                    error = %e,
                                    "Bitwarden auto-unlock from keyring failed"
                                );
                                let (text, css) = BwVaultStatus::Locked.to_status_pair();
                                Some((text, css, None))
                            }
                        }
                    })
                    .await
                    .ok()
                    .flatten();
                    if let Some((text, css, session_key)) = result {
                        // A result at all means the keyring did hold the master
                        // password (the blocking step bails out otherwise).
                        KeyringGaps::resolve(&gaps, |g| g.bitwarden = false);
                        if let Some(key) = session_key {
                            // Stored as it arrived. It used to travel back as a
                            // bare `String` that this line re-wrapped, leaving the
                            // key in freed heap.
                            set_session_key(key);
                        }
                        update_status_label(&status_label, &text, css);
                    }
                });
            }
            BACKEND_ONEPASSWORD_INDEX => {
                // 1Password selected — load token from keyring
                let token_entry = op_token_entry_switch.clone();
                let status_label = op_status_label_switch.clone();
                let gaps = keyring_gaps_switch.clone();
                glib::spawn_future_local(async move {
                    let token = gtk4::gio::spawn_blocking(get_op_token_from_keyring)
                        .await
                        .ok()
                        .flatten();
                    if let Some(token) = token {
                        use secrecy::ExposeSecret;
                        KeyringGaps::resolve(&gaps, |g| g.onepassword = false);
                        token_entry.set_text(token.expose_secret());
                        update_status_label(
                            &status_label,
                            &i18n("Token loaded from keyring"),
                            "success",
                        );
                    }
                });
            }
            BACKEND_PASSBOLT_INDEX => {
                // Passbolt selected — load passphrase from keyring
                let passphrase_entry = pb_passphrase_entry_switch.clone();
                let gaps = keyring_gaps_switch.clone();
                glib::spawn_future_local(async move {
                    let passphrase = gtk4::gio::spawn_blocking(get_pb_passphrase_from_keyring)
                        .await
                        .ok()
                        .flatten();
                    if let Some(passphrase) = passphrase {
                        use secrecy::ExposeSecret;
                        KeyringGaps::resolve(&gaps, |g| g.passbolt = false);
                        passphrase_entry.set_text(passphrase.expose_secret());
                    }
                });
            }
            BACKEND_KEEPASSXC_INDEX => {
                // KeePassXC selected — load password from keyring
                let password_entry = kdbx_password_entry_switch.clone();
                let gaps = keyring_gaps_switch.clone();
                glib::spawn_future_local(async move {
                    let password = gtk4::gio::spawn_blocking(get_kdbx_password_from_keyring)
                        .await
                        .ok()
                        .flatten();
                    if let Some(password) = password {
                        use secrecy::ExposeSecret;
                        KeyringGaps::resolve(&gaps, |g| g.kdbx = false);
                        password_entry.set_text(password.expose_secret());
                    }
                });
            }
            _ => {} // LibSecret, Pass, macOS Keychain — stateless
        }
    });

    // Initial visibility based on default states (KeePassXC selected by default)
    key_file_row.set_visible(false);
    password_row.set_visible(true);
    kdbx_storage_combo.set_visible(true);
    auth_group.set_visible(false);
    status_group.set_visible(false);
    bitwarden_group.set_visible(false);
    onepassword_group.set_visible(false);
    passbolt_group.set_visible(false);
    pass_group.set_visible(false);
    encrypted_file_group.set_visible(false);
    portable_group.set_visible(false);

    // Initial version display set above as "Detecting..."

    // Setup browse button for database file
    let kdbx_path_entry_clone = kdbx_path_entry.clone();
    kdbx_browse_button.connect_clicked(move |button| {
        let entry = kdbx_path_entry_clone.clone();
        let dialog = FileDialog::builder()
            .title(i18n("Select KeePass Database"))
            .modal(true)
            .build();

        let filter = FileFilter::new();
        filter.add_pattern("*.kdbx");
        filter.set_name(Some(&i18n("KeePass Database (*.kdbx)")));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let root = button.root();
        let window = root.and_then(|r| r.downcast::<gtk4::Window>().ok());

        dialog.open(
            window.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry.set_text(&path.display().to_string());
                }
            },
        );
    });

    // Setup browse button for key file
    let kdbx_key_file_entry_clone = kdbx_key_file_entry.clone();
    kdbx_key_file_browse_button.connect_clicked(move |button| {
        let entry = kdbx_key_file_entry_clone.clone();
        let dialog = FileDialog::builder()
            .title(i18n("Select Key File"))
            .modal(true)
            .build();

        let filter = FileFilter::new();
        filter.add_pattern("*.keyx");
        filter.add_pattern("*.key");
        filter.set_name(Some(&i18n("Key Files (*.keyx, *.key)")));

        let all_filter = FileFilter::new();
        all_filter.add_pattern("*");
        all_filter.set_name(Some(&i18n("All Files")));

        let filters = gtk4::gio::ListStore::new::<FileFilter>();
        filters.append(&filter);
        filters.append(&all_filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let root = button.root();
        let window = root.and_then(|r| r.downcast::<gtk4::Window>().ok());

        dialog.open(
            window.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry.set_text(&path.display().to_string());
                }
            },
        );
    });

    // Setup check connection button
    let kdbx_path_entry_check = kdbx_path_entry.clone();
    let kdbx_password_entry_check = kdbx_password_entry.clone();
    let kdbx_key_file_entry_check = kdbx_key_file_entry.clone();
    let kdbx_use_password_check_clone = kdbx_use_password_check.clone();
    let kdbx_use_key_file_check_clone = kdbx_use_key_file_check.clone();
    let kdbx_status_label_check = kdbx_status_label.clone();
    kdbx_check_button.connect_clicked(move |_| {
        let path_text = kdbx_path_entry_check.text();
        if path_text.is_empty() {
            update_status_label(
                &kdbx_status_label_check,
                &i18n("No database selected"),
                "warning",
            );
            return;
        }

        let kdbx_path = std::path::PathBuf::from(path_text.as_str());

        let password = if kdbx_use_password_check_clone.is_active() {
            let pwd = kdbx_password_entry_check.text();
            if pwd.is_empty() {
                None
            } else {
                Some(pwd.to_string())
            }
        } else {
            None
        };

        let key_file = if kdbx_use_key_file_check_clone.is_active() {
            let kf = kdbx_key_file_entry_check.text();
            if kf.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(kf.as_str()))
            }
        } else {
            None
        };

        update_status_label(&kdbx_status_label_check, &i18n("Checking..."), "dim-label");

        // Run verification asynchronously to avoid blocking the GTK main loop
        // (KDBX key derivation with argon2 can take seconds).
        let status_label_async = kdbx_status_label_check.clone();
        glib::spawn_future_local(async move {
            let result = gtk4::gio::spawn_blocking(move || {
                let password_secret = password.map(secrecy::SecretString::from);
                rustconn_core::secret::KeePassStatus::verify_kdbx_credentials(
                    &kdbx_path,
                    password_secret.as_ref(),
                    key_file.as_deref(),
                )
            })
            .await;

            match result {
                Ok(Ok(())) => {
                    update_status_label(&status_label_async, &i18n("Connected"), "success");
                }
                Ok(Err(e)) => {
                    update_status_label(&status_label_async, &e.to_string(), "error");
                }
                Err(_join_err) => {
                    update_status_label(&status_label_async, &i18n("Verification failed"), "error");
                }
            }
        });
    });

    let keepassxc_status_container = GtkBox::new(Orientation::Vertical, 6);

    // Schedule async CLI detection on background thread
    {
        let version_label = version_label.clone();
        let version_row = version_row.clone();
        let bw_status_label = bitwarden_status_label.clone();
        let bw_unlock_btn = bitwarden_unlock_button.clone();
        let bw_cmd_rc = bitwarden_cmd.clone();
        let op_status_label = onepassword_status_label.clone();
        let op_signin_btn = onepassword_signin_button.clone();
        let op_cmd_rc = onepassword_cmd.clone();
        let dropdown = secret_backend_dropdown.clone();
        let pb_status_label = passbolt_status_label.clone();
        let pb_vault_btn = passbolt_open_vault_button.clone();
        let pb_open_button = passbolt_open_vault_button.clone();
        let pb_url_entry = passbolt_server_url_entry.clone();
        let pass_status_label = pass_status_label.clone();
        let kpxc_ver = keepassxc_version.clone();
        let bw_ver = bitwarden_version.clone();
        let op_ver = onepassword_version.clone();
        let pb_ver = passbolt_version.clone();
        let pass_ver = pass_version.clone();
        let det_complete = detection_complete.clone();
        let st_avail = secret_tool_available.clone();
        // Storage combos and their status labels, so a keyring selection loaded
        // from config can be flagged once detection lands (#259).
        let storage_combos_det = [
            (
                bitwarden_storage_combo.clone(),
                bitwarden_status_label.clone(),
            ),
            (
                onepassword_storage_combo.clone(),
                onepassword_status_label.clone(),
            ),
            (
                passbolt_storage_combo.clone(),
                passbolt_status_label.clone(),
            ),
            (kdbx_storage_combo.clone(), kdbx_status_label.clone()),
        ];
        let availability_label_det = availability_label.clone();
        let detection_result_det = detection_result.clone();
        let kdbx_enabled_row_det = kdbx_enabled_row.clone();
        let kdbx_path_entry_det = kdbx_path_entry.clone();
        let portable_passphrase_det = portable_passphrase_entry.clone();

        // Run detection on a real OS thread so the GTK main loop stays idle
        // and can render frames while detection runs in the background.
        // GTK widgets are not Send, so we use a channel to pass results back.
        // Read on the main thread — it comes out of a GTK entry, which is not
        // `Send`, and the probe needs it to look at the same store the backend
        // will use rather than at the ambient `$PASSWORD_STORE_DIR`.
        let pass_store_dir = {
            let text = pass_store_dir_entry.text().to_string();
            (!text.trim().is_empty()).then_some(text)
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let det = detect_secret_backends(pass_store_dir);
            let _ = tx.send(det);
        });

        // Poll the channel from the main thread; GTK widgets stay here.
        // 50ms timeout instead of idle_add_local: an idle source would spin
        // the main loop at 100% CPU until the detection thread finishes.
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            match rx.try_recv() {
                Ok(det) => {
                    // Store detected command paths
                    *bw_cmd_rc.borrow_mut() = det.bitwarden_cmd.clone();
                    rustconn_core::secret::set_bw_cmd(&det.bitwarden_cmd);
                    // Cloned, like `bitwarden_cmd` on the line above: moving the
                    // field out would partially move `det`, and the Status row
                    // below needs it whole.
                    *op_cmd_rc.borrow_mut() = det.onepassword_cmd.clone();

                    // Store versions for dropdown callback
                    *kpxc_ver.borrow_mut() = det.keepassxc_version.clone();
                    *bw_ver.borrow_mut() = det.bitwarden_version.clone();
                    *op_ver.borrow_mut() = det.onepassword_version.clone();
                    *pb_ver.borrow_mut() = det.passbolt_version.clone();
                    *pass_ver.borrow_mut() = det.pass_version.clone();
                    *det_complete.borrow_mut() = true;
                    *st_avail.borrow_mut() = Some(det.secret_tool_available);
                    if !det.secret_tool_available {
                        // Until now the guard could not tell "no keyring" from
                        // "not asked yet", so a keyring selection restored from
                        // config was allowed through unremarked (#259).
                        let pairs: Vec<(&adw::ComboRow, &Label)> = storage_combos_det
                            .iter()
                            .map(|(combo, label)| (combo, label))
                            .collect();
                        warn_about_unavailable_keyring(&pairs);
                        tracing::warn!(
                            "secret-tool not found; existing keyring selections were left \
                             untouched and flagged in the Secrets tab"
                        );
                    }
                    // Keep the whole result: the Status row is recomputed from it
                    // every time the selection changes, so it cannot be rendered
                    // once and discarded the way the keyring probe used to be.
                    //
                    // Rendered from the stored clone rather than from `det`, which
                    // is partially moved by this point — `det.onepassword_cmd` is
                    // handed to `op_cmd_rc` above — so it can no longer be
                    // borrowed whole.
                    *detection_result_det.borrow_mut() = Some(det.clone());

                    let selected = dropdown.selected();
                    let local = LocalBackendState {
                        kdbx_enabled: kdbx_enabled_row_det.is_active(),
                        kdbx_path: expand_user_path(kdbx_path_entry_det.text().as_str()),
                        portable_passphrase_entered: !portable_passphrase_det.text().is_empty(),
                    };
                    render_backend_readiness(
                        &availability_label_det,
                        detection_result_det.borrow().as_ref(),
                        index_to_backend(selected),
                        &local,
                    );

                    // Update version label for currently selected backend
                    let cur_ver = match selected {
                        0 => &det.keepassxc_version,
                        2 => &det.bitwarden_version,
                        3 => &det.onepassword_version,
                        4 => &det.passbolt_version,
                        5 => &det.pass_version,
                        _ => &None,
                    };
                    version_label.remove_css_class("dim-label");
                    version_label.remove_css_class("error");
                    version_label.remove_css_class("success");
                    if selected == 1 {
                        version_row.set_visible(false);
                    } else if let Some(v) = cur_ver {
                        version_label.set_text(&format!("v{v}"));
                        version_label.add_css_class("success");
                    } else {
                        version_label.set_text(&i18n("Not installed"));
                        version_label.add_css_class("error");
                    }

                    // Update Bitwarden status
                    bw_unlock_btn.set_sensitive(det.bitwarden_installed);
                    if let Some((text, css)) = det.bitwarden_status {
                        update_status_label(&bw_status_label, &text, css);
                    } else {
                        update_status_label(&bw_status_label, &i18n("Not installed"), "error");
                    }

                    // Update 1Password status
                    op_signin_btn.set_sensitive(det.onepassword_installed);
                    if let Some((text, css)) = det.onepassword_status {
                        update_status_label(&op_status_label, &text, css);
                    } else {
                        update_status_label(&op_status_label, &i18n("Not installed"), "error");
                    }

                    // Update Passbolt status. The `else` matters: `passbolt_status`
                    // is `None` exactly when the CLI is missing, and without a
                    // branch for it the label kept its initial "Detecting..."
                    // forever — so the one case the user most needs told about
                    // was the one case the page stayed silent on. Same for Pass
                    // below. Bitwarden and 1Password already had this branch,
                    // which is why only two of the four were affected.
                    pb_vault_btn.set_sensitive(det.passbolt_installed);
                    if let Some((text, css)) = det.passbolt_status {
                        update_status_label(&pb_status_label, &text, css);
                        if det.passbolt_installed {
                            pb_open_button.set_sensitive(true);
                        }
                    } else {
                        update_status_label(&pb_status_label, &i18n("Not installed"), "error");
                    }

                    // Update Pass status label
                    if let Some((text, css)) = det.pass_status {
                        update_status_label(&pass_status_label, &text, css);
                    } else {
                        update_status_label(&pass_status_label, &i18n("Not installed"), "error");
                    }

                    // Update Passbolt URL from detection if empty
                    if pb_url_entry.text().is_empty()
                        && let Some(ref url) = det.passbolt_server_url
                    {
                        pb_url_entry.set_text(url);
                    }

                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    SecretsPageWidgets {
        page,
        secret_backend_dropdown,
        enable_fallback,
        transfer_button,
        portable_change_passphrase_button,
        kdbx_path_entry,
        kdbx_password_entry,
        kdbx_enabled_row,
        kdbx_storage_combo,
        kdbx_status_label,
        kdbx_browse_button,
        kdbx_check_button,
        keepassxc_status_container,
        kdbx_key_file_entry,
        kdbx_key_file_browse_button,
        kdbx_use_key_file_check,
        kdbx_use_password_check,
        kdbx_group,
        auth_group,
        status_group,
        password_row,
        key_file_row,
        bitwarden_group,
        bitwarden_status_label,
        bitwarden_unlock_button,
        bitwarden_password_entry,
        bitwarden_storage_combo,
        bitwarden_use_api_key_check,
        bitwarden_client_id_entry,
        bitwarden_client_secret_entry,
        bitwarden_cmd,
        onepassword_group,
        onepassword_status_label,
        onepassword_signin_button,
        passbolt_group,
        passbolt_status_label,
        passbolt_server_url_entry,
        passbolt_open_vault_button,
        passbolt_passphrase_entry,
        passbolt_storage_combo,
        onepassword_token_entry,
        onepassword_storage_combo,
        secret_tool_available,
        keyring_gaps,
        onepassword_cmd,
        pass_group,
        pass_store_dir_entry,
        pass_store_dir_browse_button,
        pass_status_label,
        encrypted_file_group,
        portable_group,
        portable_path_entry,
        portable_browse_button,
        portable_passphrase_entry,
        portable_confirm_entry,
        portable_storage_combo,
        portable_status_label,
    }
}

/// Sets a status label's text, tooltip, visibility and severity class.
///
/// The previous doc comment on this function read "Gets CLI version from command
/// output", left over from a neighbour it was moved past.
///
/// `pub(crate)` so `SettingsDialog::connect_portable_passphrase_change` can write
/// into the portable group's status row; every other caller is in this module.
pub(crate) fn update_status_label(label: &Label, text: &str, css_class: &str) {
    label.set_text(text);
    // A label handed text has something to say, so it is revealed here rather
    // than at each call site. Most status labels in this page live in a row of
    // their own and are visible already; the portable group's is hidden until
    // there is a message, and `make_storage_combo` writes its "System keyring
    // unavailable" warning into exactly that one. Without this the combo
    // reverted the user's choice with no visible explanation.
    label.set_visible(true);
    // Full text in a tooltip so ellipsized status (e.g. long errors) stays
    // readable on hover (#182).
    label.set_tooltip_text(Some(text));
    label.remove_css_class("success");
    label.remove_css_class("warning");
    label.remove_css_class("error");
    label.remove_css_class("dim-label");
    label.add_css_class(css_class);
}

/// Renders the Status row for `backend`.
///
/// Replaces `render_keyring_availability`, which could only describe the system
/// keyring because that was the only probe result the page kept. The verdict
/// itself is computed in [`backend_readiness`]; this only paints it.
fn render_backend_readiness(
    label: &Label,
    detection: Option<&SecretCliDetection>,
    backend: SecretBackendType,
    local: &LocalBackendState,
) {
    let readiness = backend_readiness(detection, backend, local);
    update_status_label(label, &readiness.label(), readiness.css_class());
}

pub fn load_secret_settings(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    let backend_index = backend_to_index(settings.preferred_backend);
    // `set_selected` emits `selected-notify`, so the handler in
    // `create_secrets_page` puts the backend's explanation in the row's subtitle
    // and shows the matching group. The duplicate visibility block further down
    // covers the case where the saved backend is already the selected one and no
    // notify fires.
    widgets.secret_backend_dropdown.set_selected(backend_index);
    sync_backend_subtitle(&widgets.secret_backend_dropdown);
    widgets.enable_fallback.set_active(settings.enable_fallback);
    widgets.kdbx_enabled_row.set_active(settings.kdbx_enabled);

    if let Some(path) = &settings.kdbx_path {
        widgets
            .kdbx_path_entry
            .set_text(&path.display().to_string());
    }

    if let Some(key_file) = &settings.kdbx_key_file {
        widgets
            .kdbx_key_file_entry
            .set_text(&key_file.display().to_string());
    }

    widgets
        .kdbx_use_password_check
        .set_active(settings.kdbx_use_password);
    widgets
        .kdbx_use_key_file_check
        .set_active(settings.kdbx_use_key_file);
    set_storage_combo_value(&widgets.kdbx_storage_combo, settings.kdbx_storage());

    // Load Bitwarden storage choice
    set_storage_combo_value(
        &widgets.bitwarden_storage_combo,
        settings.bitwarden_storage(),
    );

    // Load Bitwarden API key setting
    widgets
        .bitwarden_use_api_key_check
        .set_active(settings.bitwarden_use_api_key);

    // Load Bitwarden API credentials if available (from encrypted storage)
    if let Some(ref client_id) = settings.bitwarden_client_id {
        use secrecy::ExposeSecret;
        widgets
            .bitwarden_client_id_entry
            .set_text(client_id.expose_secret());
    }
    if let Some(ref client_secret) = settings.bitwarden_client_secret {
        use secrecy::ExposeSecret;
        widgets
            .bitwarden_client_secret_entry
            .set_text(client_secret.expose_secret());
    }

    // Load Passbolt server URL
    if let Some(ref url) = settings.passbolt_server_url {
        widgets.passbolt_server_url_entry.set_text(url);
    }

    // Load 1Password service account token if available
    if let Some(ref token) = settings.onepassword_service_account_token {
        use secrecy::ExposeSecret;
        widgets
            .onepassword_token_entry
            .set_text(token.expose_secret());
    }

    // Pre-fill the remaining password entries from the runtime secrets the app
    // already holds, the way the 1Password token and Bitwarden API fields above
    // always have. Without it a blank entry made a storage-mode change destroy
    // the secret: switching "Encrypted file" → "System keyring" dropped the blob
    // from disk and wrote nothing to the keyring, so the password was gone at the
    // next restart. The async keyring loaders below overwrite these with the
    // keyring's own copy when they find one.
    if let Some(ref password) = settings.kdbx_password {
        use secrecy::ExposeSecret;
        widgets
            .kdbx_password_entry
            .set_text(password.expose_secret());
    }
    if let Some(ref password) = settings.bitwarden_password {
        use secrecy::ExposeSecret;
        widgets
            .bitwarden_password_entry
            .set_text(password.expose_secret());
    }
    if let Some(ref passphrase) = settings.passbolt_passphrase {
        use secrecy::ExposeSecret;
        widgets
            .passbolt_passphrase_entry
            .set_text(passphrase.expose_secret());
    }

    // Assume every keyring-backed backend is missing its secret until a lookup
    // below proves otherwise. The dirty check reads this so a secret retyped
    // after a failed keyring write still counts as something to save (a value
    // identical to the one in memory is invisible to `has_new_runtime_secret`).
    widgets
        .keyring_gaps
        .set(KeyringGaps::from_settings(settings));

    set_storage_combo_value(
        &widgets.onepassword_storage_combo,
        settings.onepassword_storage(),
    );

    // Load Passbolt storage choice
    set_storage_combo_value(&widgets.passbolt_storage_combo, settings.passbolt_storage());

    // Load Pass store directory
    if let Some(ref path) = settings.pass_store_dir {
        widgets
            .pass_store_dir_entry
            .set_text(&path.display().to_string());
    }

    // Load portable encrypted file settings
    if let Some(ref path) = settings.portable_file_path {
        widgets
            .portable_path_entry
            .set_text(&path.display().to_string());
    }
    if let Some(ref passphrase) = settings.portable_passphrase {
        use secrecy::ExposeSecret;
        widgets
            .portable_passphrase_entry
            .set_text(passphrase.expose_secret());
        // Mirror it into the confirmation so an already-known passphrase does
        // not read as a mismatch the moment the page opens.
        widgets
            .portable_confirm_entry
            .set_text(passphrase.expose_secret());
    }
    set_storage_combo_value(&widgets.portable_storage_combo, settings.portable_storage());

    // Show/hide groups based on selected backend
    let show_kdbx = backend_index == BACKEND_KEEPASSXC_INDEX;
    widgets.kdbx_group.set_visible(show_kdbx);
    widgets
        .auth_group
        .set_visible(show_kdbx && settings.kdbx_enabled);
    widgets
        .status_group
        .set_visible(show_kdbx && settings.kdbx_enabled);
    widgets
        .bitwarden_group
        .set_visible(backend_index == BACKEND_BITWARDEN_INDEX);
    widgets
        .onepassword_group
        .set_visible(backend_index == BACKEND_ONEPASSWORD_INDEX);
    widgets
        .passbolt_group
        .set_visible(backend_index == BACKEND_PASSBOLT_INDEX);
    widgets
        .pass_group
        .set_visible(backend_index == BACKEND_PASS_INDEX);
    widgets
        .encrypted_file_group
        .set_visible(backend_index == BACKEND_ENCRYPTED_FILE_INDEX);
    widgets
        .portable_group
        .set_visible(backend_index == BACKEND_PORTABLE_INDEX);
    widgets.password_row.set_visible(settings.kdbx_use_password);
    widgets
        .kdbx_storage_combo
        .set_visible(settings.kdbx_use_password);
    widgets.key_file_row.set_visible(settings.kdbx_use_key_file);

    let status_text = if settings.kdbx_enabled {
        if settings.kdbx_path.is_some() {
            i18n("Configured")
        } else {
            i18n("Database path required")
        }
    } else {
        i18n("Disabled")
    };

    widgets.kdbx_status_label.set_text(&status_text);

    widgets.kdbx_status_label.remove_css_class("success");
    widgets.kdbx_status_label.remove_css_class("warning");
    widgets.kdbx_status_label.remove_css_class("error");
    widgets.kdbx_status_label.remove_css_class("dim-label");

    let status_css_class = if settings.kdbx_enabled {
        if settings.kdbx_path.is_some() {
            "success"
        } else {
            "warning"
        }
    } else {
        "dim-label"
    };
    widgets.kdbx_status_label.add_css_class(status_css_class);

    // Load credentials from keyring ONLY for the preferred backend (lazy init).
    // Other backends' credentials are loaded on-demand when the user switches
    // to them via the dropdown.
    match settings.preferred_backend {
        SecretBackendType::Bitwarden => {
            load_bitwarden_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::OnePassword => {
            load_onepassword_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::Passbolt => {
            load_passbolt_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
            load_kdbx_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::PortableEncryptedFile => {
            // Not stateless: the store's passphrase is a settings-tab field, and
            // it is the one credential the page must be able to pre-fill.
            load_portable_credentials_from_keyring(widgets, settings);
        }
        SecretBackendType::LibSecret
        | SecretBackendType::MacOsKeychain
        | SecretBackendType::Pass
        | SecretBackendType::EncryptedFile => {
            // Stateless backends — nothing to load from keyring.
            // (EncryptedFile keeps its entries in its own file; no settings-tab
            // credential fields to populate.)
        }
    }
}

/// Loads Bitwarden credentials from keyring and performs auto-unlock.
fn load_bitwarden_credentials_from_keyring(
    widgets: &SecretsPageWidgets,
    settings: &SecretSettings,
) {
    if !settings.bitwarden_save_to_keyring {
        return;
    }
    let status_label = widgets.bitwarden_status_label.clone();
    let gaps = widgets.keyring_gaps.clone();
    tracing::debug!("Scheduling Bitwarden auto-unlock from keyring (async)");
    glib::spawn_future_local({
        let status_label = status_label.clone();
        async move {
            let t_bw = std::time::Instant::now();
            let result = gtk4::gio::spawn_blocking(move || {
                // No `ExposeSecret` here: the master password goes to core as a
                // `SecretString` and is never unwrapped in the GUI.
                let bw_cmd = rustconn_core::secret::get_bw_cmd();
                let password = get_bw_password_from_keyring();
                let password = if let Some(p) = password {
                    p
                } else {
                    tracing::debug!("No keyring password found for auto-unlock");
                    return None;
                };
                tracing::debug!(
                    bw_cmd = %bw_cmd,
                    "Got keyring password, checking vault status"
                );
                let bw_status = check_bitwarden_status_sync(&bw_cmd);
                if !bw_status.should_try_unlock() {
                    tracing::debug!(
                        ?bw_status,
                        "Bitwarden auto-unlock: nothing to unlock, reporting the probed state"
                    );
                    let (text, css) = bw_status.to_status_pair();
                    return Some((text, css, None));
                }
                tracing::debug!(
                    ?bw_status,
                    "Bitwarden auto-unlock: attempting an unlock with the keyring password"
                );
                match rustconn_core::secret::unlock_vault_blocking(&password) {
                    Ok(session_key) => {
                        let (text, css) = BwVaultStatus::Unlocked.to_status_pair();
                        Some((text, css, Some(session_key)))
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Bitwarden auto-unlock from keyring failed"
                        );
                        let (text, css) = BwVaultStatus::Locked.to_status_pair();
                        Some((text, css, None))
                    }
                }
            })
            .await
            .ok()
            .flatten();
            tracing::debug!(
                elapsed_ms = t_bw.elapsed().as_millis(),
                "load_secret_settings — Bitwarden auto-unlock COMPLETED"
            );

            if let Some((text, css, session_key)) = result {
                // A result at all means the keyring did hold the master password
                // — the blocking step returns `None` when the lookup is empty.
                KeyringGaps::resolve(&gaps, |g| g.bitwarden = false);
                if let Some(key) = session_key {
                    // Stored as it arrived; no bare `String` round trip.
                    set_session_key(key);
                    tracing::info!("Bitwarden auto-unlocked from keyring");
                }
                update_status_label(&status_label, &text, css);
            }
        }
    });
}

/// Loads 1Password service account token from keyring.
fn load_onepassword_credentials_from_keyring(
    widgets: &SecretsPageWidgets,
    settings: &SecretSettings,
) {
    if !settings.onepassword_save_to_keyring {
        return;
    }
    let token_entry = widgets.onepassword_token_entry.clone();
    let status_label = widgets.onepassword_status_label.clone();
    let gaps = widgets.keyring_gaps.clone();
    tracing::debug!("Scheduling 1Password token auto-load from keyring (async)");
    glib::spawn_future_local(async move {
        let t_op = std::time::Instant::now();
        let token = gtk4::gio::spawn_blocking(get_op_token_from_keyring)
            .await
            .ok()
            .flatten();
        tracing::debug!(
            elapsed_ms = t_op.elapsed().as_millis(),
            "load_secret_settings — 1Password keyring COMPLETED"
        );

        if let Some(token) = token {
            use secrecy::ExposeSecret;
            tracing::debug!("1Password token loaded from keyring");
            KeyringGaps::resolve(&gaps, |g| g.onepassword = false);
            token_entry.set_text(token.expose_secret());
            update_status_label(&status_label, &i18n("Token loaded from keyring"), "success");
            tracing::info!("1Password token set from keyring");
        } else {
            tracing::debug!("No 1Password token found in keyring");
        }
    });
}

/// Loads Passbolt passphrase from keyring.
fn load_passbolt_credentials_from_keyring(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    if !settings.passbolt_save_to_keyring {
        return;
    }
    let passphrase_entry = widgets.passbolt_passphrase_entry.clone();
    let gaps = widgets.keyring_gaps.clone();
    tracing::debug!("Scheduling Passbolt passphrase auto-load (async)");
    glib::spawn_future_local(async move {
        let t_pb = std::time::Instant::now();
        let passphrase = gtk4::gio::spawn_blocking(get_pb_passphrase_from_keyring)
            .await
            .ok()
            .flatten();
        tracing::debug!(
            elapsed_ms = t_pb.elapsed().as_millis(),
            "load_secret_settings — Passbolt keyring COMPLETED"
        );

        if let Some(passphrase) = passphrase {
            use secrecy::ExposeSecret;
            tracing::debug!("Passbolt passphrase loaded from keyring");
            KeyringGaps::resolve(&gaps, |g| g.passbolt = false);
            passphrase_entry.set_text(passphrase.expose_secret());
            tracing::info!("Passbolt passphrase restored from keyring");
        } else {
            tracing::debug!("No Passbolt passphrase found in keyring");
        }
    });
}

/// Loads the portable credential file passphrase from keyring.
///
/// Fills the confirmation entry too, so a passphrase the user never retyped is
/// not reported back to them as a mismatch.
fn load_portable_credentials_from_keyring(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    if !settings.portable_save_to_keyring {
        return;
    }
    let passphrase_entry = widgets.portable_passphrase_entry.clone();
    let confirm_entry = widgets.portable_confirm_entry.clone();
    let gaps = widgets.keyring_gaps.clone();
    tracing::debug!("Scheduling portable passphrase auto-load (async)");
    glib::spawn_future_local(async move {
        let passphrase = gtk4::gio::spawn_blocking(get_portable_passphrase_from_keyring)
            .await
            .ok()
            .flatten();

        if let Some(passphrase) = passphrase {
            use secrecy::ExposeSecret;
            KeyringGaps::resolve(&gaps, |g| g.portable = false);
            passphrase_entry.set_text(passphrase.expose_secret());
            confirm_entry.set_text(passphrase.expose_secret());
            tracing::info!("Portable file passphrase restored from keyring");
        } else {
            tracing::debug!("No portable file passphrase found in keyring");
        }
    });
}

/// Loads KeePassXC password from keyring.
fn load_kdbx_credentials_from_keyring(widgets: &SecretsPageWidgets, settings: &SecretSettings) {
    if !settings.kdbx_save_to_keyring {
        return;
    }
    let password_entry = widgets.kdbx_password_entry.clone();
    let gaps = widgets.keyring_gaps.clone();
    tracing::debug!("Scheduling KDBX password auto-load (async)");
    glib::spawn_future_local(async move {
        let t_kdbx = std::time::Instant::now();
        let password = gtk4::gio::spawn_blocking(get_kdbx_password_from_keyring)
            .await
            .ok()
            .flatten();
        tracing::debug!(
            elapsed_ms = t_kdbx.elapsed().as_millis(),
            "load_secret_settings — KDBX keyring COMPLETED"
        );

        if let Some(password) = password {
            use secrecy::ExposeSecret;
            tracing::debug!("KDBX password loaded from keyring");
            KeyringGaps::resolve(&gaps, |g| g.kdbx = false);
            password_entry.set_text(password.expose_secret());
            tracing::info!("KDBX password restored from keyring");
        } else {
            tracing::debug!("No KDBX password found in keyring");
        }
    });
}

/// Collects secret settings from UI controls
pub fn collect_secret_settings(
    widgets: &SecretsPageWidgets,
    settings: &Rc<RefCell<rustconn_core::config::AppSettings>>,
) -> SecretSettings {
    let preferred_backend = index_to_backend(widgets.secret_backend_dropdown.selected());

    // Every path row on this page goes through `expand_user_path`: a typed
    // `~/…` was stored verbatim, and `PathBuf` gives `~` no meaning, so the
    // file was looked for in a directory literally called `~`.
    let kdbx_path = expand_user_path(widgets.kdbx_path_entry.text().as_str());

    // The "Use key file" / "Use password" switches decide which credentials the
    // KDBX backend is handed. Until 0.19.10 they only hid the rows, so a key
    // file left in the entry was still passed to the database after the switch
    // had been turned off.
    let kdbx_use_key_file = widgets.kdbx_use_key_file_check.is_active();
    let kdbx_use_password = widgets.kdbx_use_password_check.is_active();

    let kdbx_key_file = kdbx_use_key_file
        .then(|| expand_user_path(widgets.kdbx_key_file_entry.text().as_str()))
        .flatten();

    let (kdbx_password, kdbx_password_encrypted) = {
        let storage = storage_combo_value(&widgets.kdbx_storage_combo);
        match storage {
            CredentialStorage::EncryptedFile if kdbx_use_password => {
                let password_text = widgets.kdbx_password_entry.text();
                if password_text.is_empty() {
                    (
                        None,
                        settings.borrow().secrets.kdbx_password_encrypted.clone(),
                    )
                } else {
                    let password = secrecy::SecretString::new(password_text.to_string().into());
                    let encrypted = settings
                        .borrow()
                        .secrets
                        .kdbx_password_encrypted
                        .clone()
                        .or_else(|| Some("encrypted_password_placeholder".to_string()));
                    (Some(password), encrypted)
                }
            }
            // System keyring: the collected password is the *runtime* copy.
            // `kdbx_password` is `#[serde(skip)]`, so carrying it here never
            // writes the secret to disk — it is what
            // `save_pending_keyring_credentials()` hands to the keyring, and it
            // lets the database unlock without waiting for a restart. Returning
            // `None` here (0.19.17–0.19.19) meant the deferred keyring save had
            // nothing to store and silently did nothing (issue #272).
            // Still no encrypted blob: the keyring is the persistence layer.
            CredentialStorage::SystemKeyring if kdbx_use_password => {
                let password_text = widgets.kdbx_password_entry.text();
                if password_text.is_empty() {
                    (None, None)
                } else {
                    (
                        Some(secrecy::SecretString::new(password_text.to_string().into())),
                        None,
                    )
                }
            }
            // For None storage ("Don't save"), or password authentication turned off:
            // never write an encrypted blob or store to keyring. However, if the
            // user typed a password and password auth is active, carry it as the
            // session-only runtime copy so credential resolution works without
            // an on-demand unlock prompt for the rest of this session (#273).
            // `kdbx_password` is `#[serde(skip)]`, so this never reaches disk.
            CredentialStorage::None if kdbx_use_password => {
                let password_text = widgets.kdbx_password_entry.text();
                if password_text.is_empty() {
                    (None, None)
                } else {
                    (
                        Some(secrecy::SecretString::new(password_text.to_string().into())),
                        None,
                    )
                }
            }
            CredentialStorage::EncryptedFile
            | CredentialStorage::SystemKeyring
            | CredentialStorage::None => (None, None),
        }
    };

    // Collect Bitwarden password if save is enabled
    let bitwarden_storage = storage_combo_value(&widgets.bitwarden_storage_combo);
    let (bitwarden_password, bitwarden_password_encrypted) = match bitwarden_storage {
        CredentialStorage::EncryptedFile => {
            let password_text = widgets.bitwarden_password_entry.text();
            if password_text.is_empty() {
                // Keep existing encrypted password if field is empty but
                // encrypted-file storage is selected.
                (
                    None,
                    settings
                        .borrow()
                        .secrets
                        .bitwarden_password_encrypted
                        .clone(),
                )
            } else {
                let password = secrecy::SecretString::new(password_text.to_string().into());
                let encrypted = settings
                    .borrow()
                    .secrets
                    .bitwarden_password_encrypted
                    .clone()
                    .or_else(|| Some("encrypted_password_placeholder".to_string()));
                (Some(password), encrypted)
            }
        }
        // Same shape as the KDBX keyring branch: carry the typed password as
        // the runtime-only copy so the deferred keyring save has something to
        // store, and write no blob (issue #272).
        CredentialStorage::SystemKeyring => {
            let password_text = widgets.bitwarden_password_entry.text();
            if password_text.is_empty() {
                (None, None)
            } else {
                (
                    Some(secrecy::SecretString::new(password_text.to_string().into())),
                    None,
                )
            }
        }
        CredentialStorage::None => (None, None),
    };

    // Collect Bitwarden API key settings
    let bitwarden_use_api_key = widgets.bitwarden_use_api_key_check.is_active();
    let bitwarden_save_to_keyring = bitwarden_storage == CredentialStorage::SystemKeyring;

    let (bitwarden_client_id, bitwarden_client_id_encrypted) = if bitwarden_use_api_key {
        let client_id_text = widgets.bitwarden_client_id_entry.text();
        if client_id_text.is_empty() {
            // Keep existing encrypted value if field is empty — unless the
            // keyring is the persistence layer, where no blob belongs on disk.
            (
                None,
                if bitwarden_save_to_keyring {
                    None
                } else {
                    settings
                        .borrow()
                        .secrets
                        .bitwarden_client_id_encrypted
                        .clone()
                },
            )
        } else {
            let client_id = secrecy::SecretString::new(client_id_text.to_string().into());
            let encrypted = if bitwarden_save_to_keyring {
                None
            } else {
                settings
                    .borrow()
                    .secrets
                    .bitwarden_client_id_encrypted
                    .clone()
                    .or_else(|| Some("encrypted_client_id_placeholder".to_string()))
            };
            (Some(client_id), encrypted)
        }
    } else {
        (None, None)
    };

    let (bitwarden_client_secret, bitwarden_client_secret_encrypted) = if bitwarden_use_api_key {
        let client_secret_text = widgets.bitwarden_client_secret_entry.text();
        if client_secret_text.is_empty() {
            // Keep existing encrypted value if field is empty — unless the
            // keyring is the persistence layer.
            (
                None,
                if bitwarden_save_to_keyring {
                    None
                } else {
                    settings
                        .borrow()
                        .secrets
                        .bitwarden_client_secret_encrypted
                        .clone()
                },
            )
        } else {
            let client_secret = secrecy::SecretString::new(client_secret_text.to_string().into());
            let encrypted = if bitwarden_save_to_keyring {
                None
            } else {
                settings
                    .borrow()
                    .secrets
                    .bitwarden_client_secret_encrypted
                    .clone()
                    .or_else(|| Some("encrypted_client_secret_placeholder".to_string()))
            };
            (Some(client_secret), encrypted)
        }
    } else {
        (None, None)
    };

    // Collect 1Password service account token
    let onepassword_storage = storage_combo_value(&widgets.onepassword_storage_combo);
    let (onepassword_service_account_token, onepassword_service_account_token_encrypted) =
        match onepassword_storage {
            CredentialStorage::EncryptedFile => {
                let token_text = widgets.onepassword_token_entry.text();
                if token_text.is_empty() {
                    (
                        None,
                        settings
                            .borrow()
                            .secrets
                            .onepassword_service_account_token_encrypted
                            .clone(),
                    )
                } else {
                    let token = secrecy::SecretString::new(token_text.to_string().into());
                    let encrypted = settings
                        .borrow()
                        .secrets
                        .onepassword_service_account_token_encrypted
                        .clone()
                        .or_else(|| Some("encrypted_token_placeholder".to_string()));
                    (Some(token), encrypted)
                }
            }
            // Runtime-only copy for the deferred keyring save, no blob on disk
            // — same shape as the KDBX keyring branch (issue #272).
            CredentialStorage::SystemKeyring => {
                let token_text = widgets.onepassword_token_entry.text();
                if token_text.is_empty() {
                    (None, None)
                } else {
                    (
                        Some(secrecy::SecretString::new(token_text.to_string().into())),
                        None,
                    )
                }
            }
            CredentialStorage::None => (None, None),
        };

    // Collect Passbolt passphrase
    let passbolt_storage = storage_combo_value(&widgets.passbolt_storage_combo);
    let (passbolt_passphrase, passbolt_passphrase_encrypted) = match passbolt_storage {
        CredentialStorage::EncryptedFile => {
            let passphrase_text = widgets.passbolt_passphrase_entry.text();
            if passphrase_text.is_empty() {
                (
                    None,
                    settings
                        .borrow()
                        .secrets
                        .passbolt_passphrase_encrypted
                        .clone(),
                )
            } else {
                let passphrase = secrecy::SecretString::new(passphrase_text.to_string().into());
                let encrypted = settings
                    .borrow()
                    .secrets
                    .passbolt_passphrase_encrypted
                    .clone()
                    .or_else(|| Some("encrypted_passphrase_placeholder".to_string()));
                (Some(passphrase), encrypted)
            }
        }
        // Runtime-only copy for the deferred keyring save, no blob on disk —
        // same shape as the KDBX keyring branch (issue #272).
        CredentialStorage::SystemKeyring => {
            let passphrase_text = widgets.passbolt_passphrase_entry.text();
            if passphrase_text.is_empty() {
                (None, None)
            } else {
                (
                    Some(secrecy::SecretString::new(
                        passphrase_text.to_string().into(),
                    )),
                    None,
                )
            }
        }
        CredentialStorage::None => (None, None),
    };

    // Collect the portable file passphrase.
    //
    // A passphrase that does not match its confirmation is treated as not
    // entered. The alternative is worse than it sounds: this value becomes the
    // only key to every credential in the portable file, it is not checked
    // against anything when the file is first created, and there is no recovery.
    // Writing a typo would produce a store that opens with a passphrase nobody
    // knows.
    //
    // Both refusal conditions — a mismatch, and a missing confirmation for a
    // store that does not exist yet — are shown inline next to the two entries
    // before Save is reached, so dropping the value here is not a silent
    // refusal. That claim used to be made about the mismatch alone, while a
    // missing confirmation could be hidden behind the passphrase-strength
    // advice or go unrechecked after the path changed; keep the two in step if
    // either side is edited.
    let portable_storage = storage_combo_value(&widgets.portable_storage_combo);
    let (portable_passphrase, portable_passphrase_encrypted) = {
        let pass_text = widgets.portable_passphrase_entry.text();
        let confirm_text = widgets.portable_confirm_entry.text();

        // An empty confirmation means "I did not retype it", which is fine only
        // when there is a file to check the passphrase against — the save path
        // verifies it and reports a mismatch. For a store that does not exist
        // yet there is nothing to check against, and the first write makes
        // whatever was typed the key to the file forever, so the confirmation is
        // required. This is the same rule `rustconn-cli` applies when it decides
        // whether to prompt twice.
        let store_path = rustconn_core::secret::resolve_portable_store_path(
            expand_user_path(widgets.portable_path_entry.text().as_str()).as_deref(),
        );
        let discarded = portable_passphrase_is_unconfirmed(
            &store_path,
            pass_text.as_str(),
            confirm_text.as_str(),
        );

        if pass_text.is_empty() || discarded {
            if discarded {
                tracing::warn!(
                    "Portable passphrase was not confirmed — not saving it; a new store \
                     requires the confirmation entry"
                );
            }
            // Keep whatever is already persisted rather than dropping the blob:
            // a blank field means "I did not retype it", the same rule the other
            // backends follow.
            let existing = if portable_storage == CredentialStorage::EncryptedFile {
                settings
                    .borrow()
                    .secrets
                    .portable_passphrase_encrypted
                    .clone()
            } else {
                None
            };
            (None, existing)
        } else {
            match portable_storage {
                CredentialStorage::None => (None, None),
                // `apply_storage_persistence` re-encrypts from the runtime value
                // just before the settings are written, so a placeholder is
                // enough to record "an encrypted copy is wanted here".
                CredentialStorage::EncryptedFile => (
                    Some(secrecy::SecretString::new(pass_text.to_string().into())),
                    settings
                        .borrow()
                        .secrets
                        .portable_passphrase_encrypted
                        .clone()
                        .or_else(|| Some("encrypted_passphrase_placeholder".to_string())),
                ),
                // Keyring-backed: the runtime copy is the only one that reaches
                // the keyring, and no blob goes to disk.
                CredentialStorage::SystemKeyring => (
                    Some(secrecy::SecretString::new(pass_text.to_string().into())),
                    None,
                ),
            }
        }
    };

    // Keyring saves are deferred — performed asynchronously after the dialog
    // closes to avoid blocking the GTK main loop (D-Bus round-trip). The caller
    // should invoke `save_pending_keyring_credentials()` after processing the
    // collected settings.
    let kdbx_storage = storage_combo_value(&widgets.kdbx_storage_combo);

    let mut collected = SecretSettings {
        preferred_backend,
        enable_fallback: widgets.enable_fallback.is_active(),
        kdbx_path,
        kdbx_enabled: widgets.kdbx_enabled_row.is_active(),
        kdbx_password,
        kdbx_password_encrypted,
        kdbx_key_file,
        kdbx_use_key_file,
        kdbx_use_password,
        bitwarden_password,
        bitwarden_password_encrypted,
        bitwarden_use_api_key,
        bitwarden_client_id,
        bitwarden_client_id_encrypted,
        bitwarden_client_secret,
        bitwarden_client_secret_encrypted,
        bitwarden_save_to_keyring,
        kdbx_save_to_keyring: kdbx_storage == CredentialStorage::SystemKeyring,
        onepassword_service_account_token,
        onepassword_service_account_token_encrypted,
        onepassword_save_to_keyring: onepassword_storage == CredentialStorage::SystemKeyring,
        passbolt_passphrase,
        passbolt_passphrase_encrypted,
        passbolt_save_to_keyring: passbolt_storage == CredentialStorage::SystemKeyring,
        passbolt_server_url: {
            let url_text = widgets.passbolt_server_url_entry.text();
            if url_text.is_empty() {
                None
            } else {
                Some(url_text.to_string())
            }
        },
        // Collect Pass store directory
        pass_store_dir: expand_user_path(widgets.pass_store_dir_entry.text().as_str()),
        // Collect portable encrypted file settings. An empty entry stays `None`,
        // which is what makes the default location the default rather than a
        // path this dialog writes into the config the first time it is opened.
        portable_file_path: expand_user_path(widgets.portable_path_entry.text().as_str()),
        portable_passphrase,
        portable_passphrase_encrypted,
        portable_save_to_keyring: portable_storage == CredentialStorage::SystemKeyring,
    };

    // A password entry the keyring could not pre-fill stays blank, so switching
    // storage to "System keyring" without retyping would collect no secret at
    // all — the blob is dropped from disk and nothing reaches the keyring, and
    // the password is gone at the next restart. Take the runtime copy the app
    // already holds instead. Blank still means "I did not retype it", never
    // "delete it".
    {
        let current = settings.borrow();
        collected.carry_over_runtime_secrets(&current.secrets);
    }

    collected
}

/// Outcome of the deferred keyring writes and revocations.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyringSaveOutcome {
    /// Credentials that could not be written to the system keyring.
    ///
    /// Non-zero means the secret exists in memory only and will be gone after a
    /// restart, so the user has to be told (issue #272 follow-up).
    pub write_failures: u32,
    /// Stale keyring entries that could not be removed.
    pub revoke_failures: u32,
}

/// Saves credentials to the system keyring based on the storage choices in
/// `current`, and revokes entries that `current` no longer stores there
/// (compared against `previous`, the settings in force before the save).
///
/// Call this **asynchronously** after collecting settings (e.g. via
/// `glib::spawn_future_local` + `gio::spawn_blocking`) to avoid blocking the GTK
/// main loop.
pub fn save_pending_keyring_credentials(
    previous: &SecretSettings,
    current: &SecretSettings,
) -> KeyringSaveOutcome {
    use secrecy::ExposeSecret;
    let mut outcome = KeyringSaveOutcome::default();

    if current.kdbx_save_to_keyring
        && current.kdbx_use_password
        && let Some(ref pw) = current.kdbx_password
        && !save_kdbx_password_to_keyring(pw.expose_secret())
    {
        outcome.write_failures += 1;
    }
    if current.bitwarden_save_to_keyring
        && let Some(ref pw) = current.bitwarden_password
        && !save_bw_password_to_keyring(pw.expose_secret())
    {
        outcome.write_failures += 1;
    }
    // The API key pair follows the Bitwarden storage choice: with the keyring
    // selected no encrypted blob is written to disk, so the keyring is the only
    // place these can live.
    if current.bitwarden_save_to_keyring
        && current.bitwarden_use_api_key
        && let (Some(id), Some(secret)) = (
            current.bitwarden_client_id.as_ref(),
            current.bitwarden_client_secret.as_ref(),
        )
        && !save_bw_api_credentials_to_keyring(id, secret)
    {
        outcome.write_failures += 1;
    }
    if current.onepassword_save_to_keyring
        && let Some(ref token) = current.onepassword_service_account_token
        && !save_op_token_to_keyring(token.expose_secret())
    {
        outcome.write_failures += 1;
    }
    if current.passbolt_save_to_keyring
        && let Some(ref pp) = current.passbolt_passphrase
        && !save_pb_passphrase_to_keyring(pp.expose_secret())
    {
        outcome.write_failures += 1;
    }
    if current.portable_save_to_keyring
        && let Some(ref pp) = current.portable_passphrase
        && !save_portable_passphrase_to_keyring(pp.expose_secret())
    {
        outcome.write_failures += 1;
    }

    outcome.revoke_failures = revoke_stale_keyring_credentials(previous, current);

    outcome
}

/// Removes keyring entries whose backend no longer stores its secret there.
///
/// Returns the number of deletions that failed. Switching a backend to
/// "Encrypted file" / "Don't save" (or turning it off) previously left the
/// keyring entry behind forever, so a stored secret could never be revoked.
fn revoke_stale_keyring_credentials(previous: &SecretSettings, current: &SecretSettings) -> u32 {
    let revocations = current.keyring_revocations(previous);
    if !revocations.any() {
        return 0;
    }

    let mut failures = 0u32;
    if revocations.kdbx_password && !delete_kdbx_password_from_keyring() {
        failures += 1;
    }
    if revocations.bitwarden_password && !delete_bw_password_from_keyring() {
        failures += 1;
    }
    if revocations.bitwarden_api_credentials && !delete_bw_api_credentials_from_keyring() {
        failures += 1;
    }
    if revocations.onepassword_token && !delete_op_token_from_keyring() {
        failures += 1;
    }
    if revocations.passbolt_passphrase && !delete_pb_passphrase_from_keyring() {
        failures += 1;
    }
    if revocations.portable_passphrase && !delete_portable_passphrase_from_keyring() {
        failures += 1;
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every selector row must map back to the position it was built at.
    ///
    /// `backend_to_index` folds `KdbxFile` onto the `KeePassXC` row and the two
    /// keyring variants onto one platform-dependent row, so it is a hand-written
    /// match rather than a lookup — which is exactly the kind of thing that goes
    /// stale when a row is inserted. A mismatch here means a saved configuration
    /// selects a different backend than the one the user picked.
    #[test]
    fn backend_choices_round_trip_through_index() {
        for (index, choice) in backend_choices().iter().enumerate() {
            let index = u32::try_from(index).expect("selector has far fewer than u32::MAX rows");
            assert_eq!(
                backend_to_index(choice.backend),
                index,
                "row {index} holds {:?} but backend_to_index sends it elsewhere",
                choice.backend
            );
            assert_eq!(
                index_to_backend(index),
                choice.backend,
                "index_to_backend({index}) disagrees with the table"
            );
        }
    }

    /// The two descriptive labels are written out as literals in
    /// `backend_choices` so xgettext can extract them, which means they are a
    /// second copy of `display_name()`. This is the assertion that comment
    /// promises: if one is reworded, the other has to follow.
    #[test]
    fn descriptive_labels_match_display_name() {
        let choices = backend_choices();
        for backend in [
            SecretBackendType::EncryptedFile,
            SecretBackendType::PortableEncryptedFile,
        ] {
            let row = &choices[backend_to_index(backend) as usize];
            assert_eq!(
                row.label,
                backend.display_name(),
                "selector label for {backend:?} drifted from display_name()"
            );
        }
    }

    /// `from_backend_id` exists to turn a chain report back into something that
    /// can be named to a user, so every backend the chain can contain has to be
    /// recognised. The id strings live in eight separate files next to their
    /// `SecretBackend` impls; this is the only place they are checked together.
    #[test]
    fn backend_ids_map_back_to_their_variant() {
        for (id, expected) in [
            ("libsecret", SecretBackendType::LibSecret),
            ("bitwarden", SecretBackendType::Bitwarden),
            ("onepassword", SecretBackendType::OnePassword),
            ("passbolt", SecretBackendType::Passbolt),
            ("pass", SecretBackendType::Pass),
            ("macos_keychain", SecretBackendType::MacOsKeychain),
            ("encrypted_file", SecretBackendType::EncryptedFile),
            (
                "portable_encrypted_file",
                SecretBackendType::PortableEncryptedFile,
            ),
        ] {
            assert_eq!(
                SecretBackendType::from_backend_id(id),
                Some(expected),
                "backend_id {id:?} no longer maps to {expected:?}"
            );
        }
        assert_eq!(SecretBackendType::from_backend_id("nonesuch"), None);
    }
}
