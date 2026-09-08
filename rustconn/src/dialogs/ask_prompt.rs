//! Interactive prompt dialog for `@ask:` variables.
//!
//! When a connection references a variable whose value is an ASK directive (see
//! [`rustconn_core::AskSpec`]), RustConn asks the user for the value before the
//! connection launches. One dialog collects every ASK variable the connection
//! needs, then hands the answers back as ordinary [`Variable`]s that shadow the
//! stored directives for that connect.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::Variable;
use rustconn_core::variables::AskSpec;

use crate::i18n::{i18n, i18n_f};

/// One resolved input widget, kept so the response handler can read its value.
enum AskField {
    /// Free-text entry.
    Text(adw::EntryRow),
    /// Hidden/secret entry.
    Secret(adw::PasswordEntryRow),
    /// Fixed-choice dropdown, paired with the options it indexes into.
    Choice(adw::ComboRow, Vec<String>),
}

impl AskField {
    /// Reads the current value the user entered or selected.
    fn value(&self) -> String {
        match self {
            Self::Text(row) => row.text().to_string(),
            Self::Secret(row) => row.text().to_string(),
            Self::Choice(row, options) => options
                .get(row.selected() as usize)
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// Whether the answer is sensitive and must be stored as a secret variable.
    fn is_secret(&self) -> bool {
        matches!(self, Self::Secret(_))
    }
}

/// Shows a modal dialog asking for every interactive variable a connection needs.
///
/// `requests` pairs each ASK variable name with its parsed [`AskSpec`]. The
/// callback receives `Some(answers)` — one [`Variable`] per request, named after
/// the variable it answers — when the user confirms, or `None` when the dialog
/// is cancelled (the connection must then not start).
///
/// A secret answer is returned as a secret `Variable` so its buffer is scrubbed
/// on drop; a choice answer is the selected option string.
pub fn show_ask_dialog<F>(
    parent: &impl IsA<gtk4::Widget>,
    connection_name: &str,
    requests: &[(String, AskSpec)],
    callback: F,
) where
    F: Fn(Option<Vec<Variable>>) + 'static,
{
    let heading = i18n("Enter Connection Details");
    let body = i18n_f(
        "‘{}’ needs the following before it can connect.",
        &[connection_name],
    );

    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));

    let prefs_group = adw::PreferencesGroup::new();

    // Build one row per request, remembering the widget and the variable name.
    let mut fields: Vec<(String, AskField)> = Vec::with_capacity(requests.len());
    for (name, spec) in requests {
        let field = if spec.is_choice() {
            let options = spec.options.clone();
            let option_refs: Vec<&str> = options.iter().map(String::as_str).collect();
            let list = gtk4::StringList::new(&option_refs);
            let row = adw::ComboRow::new();
            row.set_title(&spec.prompt);
            row.set_model(Some(&list));
            prefs_group.add(&row);
            AskField::Choice(row, options)
        } else if spec.secret {
            let row = adw::PasswordEntryRow::new();
            row.set_title(&spec.prompt);
            prefs_group.add(&row);
            AskField::Secret(row)
        } else {
            let row = adw::EntryRow::new();
            row.set_title(&spec.prompt);
            prefs_group.add(&row);
            AskField::Text(row)
        };
        fields.push((name.clone(), field));
    }

    dialog.set_extra_child(Some(&prefs_group));

    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("connect", &i18n("Connect"));
    dialog.set_default_response(Some("connect"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);

    dialog.connect_response(None, move |_, response| {
        if response == "connect" {
            let answers: Vec<Variable> = fields
                .iter()
                .map(|(name, field)| {
                    let value = field.value();
                    if field.is_secret() {
                        Variable::new_secret(name, value)
                    } else {
                        Variable::new(name, value)
                    }
                })
                .collect();
            callback(Some(answers));
        } else {
            callback(None);
        }
    });

    dialog.present(Some(parent));
}
