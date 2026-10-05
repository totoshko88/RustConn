//! GUI-side rules for per-connection command macros.
//!
//! One place decides whether a macro may be saved (the connection editor) and
//! whether its keybind may be registered (the dispatcher in
//! `window/macro_dispatch.rs`), so the two cannot disagree. The headless half of
//! each rule lives in `rustconn-core` ([`CommandMacro::validate_command`],
//! [`validate_macro_keybind`]); this module adds what needs GTK — that a key
//! name exists and the fixed widget-level shortcuts — and the translated
//! messages.

use rustconn_core::config::keybindings::accelerators_equivalent;
use rustconn_core::config::{KeybindingSettings, MacroKeybindError, validate_macro_keybind};
use rustconn_core::models::{CommandMacro, CommandMacroError, ProtocolType};

use crate::i18n::{i18n, i18n_f};

/// Whether sessions of `protocol` are typed into, so macros apply to them.
///
/// Graphical and web sessions have no terminal to type into; a macro accel
/// registered for one would only swallow the key.
#[must_use]
pub const fn protocol_takes_macros(protocol: ProtocolType) -> bool {
    !matches!(
        protocol,
        ProtocolType::Rdp | ProtocolType::Vnc | ProtocolType::Spice | ProtocolType::Web
    )
}

/// Returns the macro's keybind trimmed, or `None` when it is unbound.
#[must_use]
pub fn bound_keybind(macro_: &CommandMacro) -> Option<&str> {
    macro_
        .keybind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
}

/// Finds the macro bound to `accel`, whatever the modifier order or alias.
///
/// The first match wins; the editor refuses to save two macros with the same
/// keybind, so a second one can only come from a hand-edited or synced file.
#[must_use]
pub fn find_by_keybind<'a>(macros: &'a [CommandMacro], accel: &str) -> Option<&'a CommandMacro> {
    macros
        .iter()
        .find(|m| bound_keybind(m).is_some_and(|k| accelerators_equivalent(k, accel)))
}

/// Returns the index of an earlier macro that already uses the keybind of the
/// macro at `index`, if any.
#[must_use]
pub fn duplicate_of(macros: &[CommandMacro], index: usize) -> Option<usize> {
    let accel = bound_keybind(macros.get(index)?)?;
    macros[..index]
        .iter()
        .position(|m| bound_keybind(m).is_some_and(|k| accelerators_equivalent(k, accel)))
}

/// Whether GTK can parse `accel` into a real key.
fn gtk_key_exists(accel: &str) -> bool {
    gtk4::accelerator_parse(accel).is_some_and(|(key, _mods)| key.name().is_some())
}

/// Translated reason `accel` cannot be a macro keybind, `None` when it can.
///
/// Checks the headless rules first (a Ctrl/Alt/Super modifier, no collision
/// with the configurable shortcuts), then that GTK knows the key (`key_exists`,
/// injected so this can be tested without a display), then the fixed
/// shortcuts owned by widget key controllers (VTE zoom, sidebar row actions),
/// which never appear in the keybindings table.
fn keybind_problem_with(
    accel: &str,
    keybindings: &KeybindingSettings,
    key_exists: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Err(e) = validate_macro_keybind(accel, keybindings) {
        return Some(keybind_error_message(&e));
    }
    if !key_exists(accel) {
        return Some(keybind_error_message(&MacroKeybindError::Invalid));
    }
    crate::dialogs::fixed_shortcut_accels()
        .into_iter()
        .find(|(fixed, _)| accelerators_equivalent(fixed, accel))
        .map(|(_, label)| i18n_f("This shortcut is already used by “{}”", &[&label]))
}

/// Translated message for a [`MacroKeybindError`].
#[must_use]
pub fn keybind_error_message(error: &MacroKeybindError) -> String {
    match error {
        MacroKeybindError::Invalid => i18n("Not a valid shortcut, e.g. <Control><Alt>r"),
        MacroKeybindError::MissingModifier => {
            i18n("A macro shortcut needs Ctrl, Alt or Super; only F1 to F24 can be used alone")
        }
        MacroKeybindError::AppShortcut { label, .. } => {
            i18n_f("This shortcut is already used by “{}”", &[&i18n(label)])
        }
    }
}

/// Translated message for a [`CommandMacroError`].
#[must_use]
pub fn command_error_message(error: CommandMacroError) -> String {
    match error {
        CommandMacroError::EmptyCommand => i18n("Enter the command to send"),
        CommandMacroError::ControlCharacter => i18n(
            "The command cannot contain line breaks or control characters; use the Run switch to press Enter",
        ),
    }
}

/// Translated reason the macro at `index` cannot be saved, `None` when it can.
///
/// A completely blank row is not a problem: the editor drops it on save.
#[must_use]
pub fn macro_problem(
    macros: &[CommandMacro],
    index: usize,
    keybindings: &KeybindingSettings,
) -> Option<String> {
    macro_problem_with(macros, index, keybindings, gtk_key_exists)
}

/// [`macro_problem`] with the GTK key lookup injected.
fn macro_problem_with(
    macros: &[CommandMacro],
    index: usize,
    keybindings: &KeybindingSettings,
    key_exists: impl Fn(&str) -> bool,
) -> Option<String> {
    let target = macros.get(index)?;
    if target.is_blank() {
        return None;
    }
    if let Err(e) = target.validate_command() {
        return Some(command_error_message(e));
    }
    let accel = bound_keybind(target)?;
    if let Some(problem) = keybind_problem_with(accel, keybindings, key_exists) {
        return Some(problem);
    }
    duplicate_of(macros, index).map(|_| i18n("Another macro already uses this shortcut"))
}

/// Validates every macro the editor is about to save.
///
/// # Errors
/// Returns the translated problem of the first macro that cannot be saved,
/// prefixed with its name or position.
pub fn validate_for_save(
    macros: &[CommandMacro],
    keybindings: &KeybindingSettings,
) -> Result<(), String> {
    for index in 0..macros.len() {
        if let Some(problem) = macro_problem(macros, index, keybindings) {
            let name = macros[index].name.trim();
            let which = if name.is_empty() {
                (index + 1).to_string()
            } else {
                name.to_string()
            };
            return Err(i18n_f("Command macro “{}”: {}", &[&which, &problem]));
        }
    }
    Ok(())
}

/// Keybinds the dispatcher may register for `macros`.
///
/// A macro that fails any save-time rule — including one loaded from a config
/// or sync file written elsewhere — is skipped with a warning naming the macro
/// and the rule, never the command text.
#[must_use]
pub fn registrable_keybinds(
    macros: &[CommandMacro],
    keybindings: &KeybindingSettings,
) -> Vec<String> {
    registrable_keybinds_with(macros, keybindings, gtk_key_exists)
}

/// [`registrable_keybinds`] with the GTK key lookup injected.
fn registrable_keybinds_with(
    macros: &[CommandMacro],
    keybindings: &KeybindingSettings,
    key_exists: impl Fn(&str) -> bool + Copy,
) -> Vec<String> {
    let mut accels = Vec::new();
    for (index, candidate) in macros.iter().enumerate() {
        let Some(accel) = bound_keybind(candidate) else {
            continue;
        };
        if let Some(problem) = macro_problem_with(macros, index, keybindings, key_exists) {
            tracing::warn!(
                macro_name = %candidate.name,
                accel,
                reason = %problem,
                "command macro keybind not registered"
            );
            continue;
        }
        accels.push(accel.to_string());
    }
    accels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macro_with(name: &str, command: &str, keybind: Option<&str>) -> CommandMacro {
        CommandMacro {
            name: name.into(),
            command: command.into(),
            keybind: keybind.map(Into::into),
            send_newline: true,
        }
    }

    #[test]
    fn macro_accels_only_for_terminal_protocols() {
        for p in [
            ProtocolType::Rdp,
            ProtocolType::Vnc,
            ProtocolType::Spice,
            ProtocolType::Web,
        ] {
            assert!(!protocol_takes_macros(p), "{p:?}");
        }
        for p in [
            ProtocolType::Ssh,
            ProtocolType::Telnet,
            ProtocolType::Serial,
            ProtocolType::Kubernetes,
            ProtocolType::Mosh,
            ProtocolType::ZeroTrust,
        ] {
            assert!(protocol_takes_macros(p), "{p:?}");
        }
    }

    #[test]
    fn macro_find_by_keybind_ignores_spelling() {
        let macros = vec![
            macro_with("a", "ls", Some("<Control><Alt>a")),
            macro_with("b", "df", Some(" <Alt><Primary>b ")),
            macro_with("c", "id", None),
        ];
        assert_eq!(
            find_by_keybind(&macros, "<Control><Alt>b").map(|m| m.name.as_str()),
            Some("b")
        );
        assert_eq!(
            find_by_keybind(&macros, "<Alt><Control>A").map(|m| m.name.as_str()),
            Some("a")
        );
        assert!(find_by_keybind(&macros, "<Control><Alt>c").is_none());
    }

    #[test]
    fn macro_find_by_keybind_follows_reordering() {
        // The dispatcher keys its action by keybind, so editing the list order
        // cannot make an old registration fire a different macro.
        let mut macros = vec![
            macro_with("first", "ls", Some("<Control><Alt>1")),
            macro_with("second", "df", Some("<Control><Alt>2")),
        ];
        macros.swap(0, 1);
        assert_eq!(
            find_by_keybind(&macros, "<Control><Alt>1").map(|m| m.name.as_str()),
            Some("first")
        );
        macros.remove(1);
        assert!(find_by_keybind(&macros, "<Control><Alt>1").is_none());
    }

    #[test]
    fn macro_duplicate_keybind_is_reported_on_the_later_one() {
        let macros = vec![
            macro_with("a", "ls", Some("<Control><Alt>x")),
            macro_with("b", "df", None),
            macro_with("c", "id", Some("<Alt><Control>X")),
        ];
        assert_eq!(duplicate_of(&macros, 0), None);
        assert_eq!(duplicate_of(&macros, 1), None);
        assert_eq!(duplicate_of(&macros, 2), Some(0));
    }

    #[test]
    fn macro_problem_rejects_bad_commands_and_keybinds() {
        let kb = KeybindingSettings::default();
        let ok = |_: &str| true;
        let one = |m: CommandMacro| macro_problem_with(&[m], 0, &kb, ok);

        assert!(
            one(macro_with("", "", None)).is_none(),
            "blank row is dropped, not rejected"
        );
        assert!(one(macro_with("n", "uptime", None)).is_none());
        assert!(one(macro_with("n", "uptime", Some("<Control><Alt>u"))).is_none());
        assert!(
            one(macro_with("named", "", None)).is_some(),
            "empty command"
        );
        assert!(
            one(macro_with("", "", Some("<Control><Alt>u"))).is_some(),
            "keybind only"
        );
        assert!(one(macro_with("n", "ls\r", None)).is_some(), "control char");
        assert!(one(macro_with("n", "ls", Some("u"))).is_some(), "bare key");
        assert!(one(macro_with("n", "ls", Some("<Control><Alt>u"))).is_none());
        assert!(
            macro_problem_with(
                &[macro_with("n", "ls", Some("<Control><Alt>u"))],
                0,
                &kb,
                |_| false
            )
            .is_some(),
            "unknown key name"
        );
    }

    #[test]
    fn macro_registrable_keybinds_skip_invalid_and_duplicates() {
        let kb = KeybindingSettings::default();
        let app_accel = rustconn_core::default_keybindings()
            .into_iter()
            .map(|d| d.default_accel_list()[0].to_string())
            .find(|a| a.contains("<Control>"))
            .expect("a Ctrl default exists");
        let macros = vec![
            macro_with("ok", "ls", Some("<Control><Alt>j")),
            macro_with("dup", "df", Some("<Alt><Control>j")),
            macro_with("bare", "id", Some("j")),
            macro_with("app", "id", Some(&app_accel)),
            macro_with("cr", "id\nrm", Some("<Control><Alt>k")),
            macro_with("unbound", "id", None),
            macro_with("fkey", "top", Some("F7")),
        ];
        assert_eq!(
            registrable_keybinds_with(&macros, &kb, |_| true),
            vec!["<Control><Alt>j".to_string(), "F7".to_string()]
        );
    }
}
