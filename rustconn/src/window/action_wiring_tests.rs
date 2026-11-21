//! Every action a menu points at must be registered by something.
//!
//! A menu item whose `win.<name>` is not registered is shown greyed out and
//! does nothing, and nothing in the build notices. This reads the sources, so
//! it also covers items that are only built at run time.
use std::collections::BTreeSet;
use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && !path.ends_with("action_wiring_tests.rs")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push(text);
        }
    }
}

/// String literals that directly follow `prefix` in `text`.
fn literals_after(text: &str, prefix: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for part in text.split(prefix).skip(1) {
        if let Some(rest) = part.trim_start().strip_prefix('"')
            && let Some(end) = rest.find('"')
        {
            found.insert(rest[..end].to_owned());
        }
    }
    found
}

#[test]
fn every_win_action_named_in_a_menu_is_registered() {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let all = files.join("\n");
    let registered: BTreeSet<String> = [
        "SimpleAction::new(",
        "SimpleAction::new_stateful(",
        "ActionEntry::builder(",
    ]
    .iter()
    .flat_map(|p| literals_after(&all, p))
    // `const FIRE_MACRO_ACTION: &str = "name"` handed to `SimpleAction::new`.
    .chain(literals_after(&all, "_ACTION: &str ="))
    .chain(by_id_pairs(&all))
    .collect();
    let named = win_names(&all);
    let missing: Vec<_> = named.iter().filter(|n| !registered.contains(*n)).collect();
    assert!(
        missing.is_empty(),
        "menu items name win actions nobody registers: {missing:?}"
    );
}

/// `register_selected_and_by_id(window, sidebar, "x", ..)` registers `x` and
/// `x-by-id`; the name is the first literal after the call.
fn by_id_pairs(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for part in text.split("register_selected_and_by_id(").skip(1) {
        let head: String = part.chars().take(160).collect();
        if let Some(start) = head.find('"')
            && let Some(len) = head[start + 1..].find('"')
        {
            let name = &head[start + 1..start + 1 + len];
            found.insert(name.to_owned());
            found.insert(format!("{name}-by-id"));
        }
    }
    found
}

/// The names after `"win.` (the literal-after helper sees the quote first).
fn win_names(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for part in text.split("\"win.").skip(1) {
        if let Some(end) = part.find('"') {
            let name = &part[..end];
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                found.insert(name.to_owned());
            }
        }
    }
    found
}
