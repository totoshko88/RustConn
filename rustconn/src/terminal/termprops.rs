//! The one place that knows whether this build can observe VTE termprops.
//!
//! Termprops arrived in VTE 0.78 and are how a terminal reports structured facts
//! about what the remote side is doing — as opposed to the raw byte stream every
//! other part of this module deals with. RustConn uses exactly one of them:
//! `vte.shell.postexec`, which the shell sets through OSC 133 when a command it ran
//! at the prompt has returned, carrying that command's exit code.
//!
//! Two supported build targets still ship VTE 0.76 — Ubuntu 24.04 deb and the
//! noble-based AppImage — so the API is behind the
//! `vte-0-78` feature and this module is the only file that says so. Callers just
//! call [`connect_command_finished`] and get a working no-op on a build without it;
//! its return value distinguishes "nothing was connected" from "connected and the
//! remote side is quiet", which is what the startup log reports.
//!
//! The Command monitoring mode is offered by the picker regardless. Hiding it on a
//! build without the feature was considered and dropped: the picker index would then
//! mean different modes in different builds, and a `settings.toml` carrying
//! `Command` would be silently rewritten to `Off` the next time that user saved.
//! Offering a mode that stays quiet is the same observable outcome as a remote host
//! without shell integration, which the row subtitle already explains.
//!
//! # What this deliberately does not do
//!
//! *Progress.* `vte.progress.value` and `vte.progress.hint` are readable with the
//! same 0.78 API, but there is nowhere to put a percentage: `AdwTabPage` offers one
//! `indicator-icon`, which five different meanings already write to, and its `icon`
//! is the tab's protocol icon. Showing progress needs a priority scheme across
//! those writers first, which is a refactor rather than a feature.
//!
//! *Tab icons from the remote shell.* `vte.icon.image` is documented by VTE 0.84 as
//! "always unset" — upstream does not populate it yet, so there is nothing to read
//! whatever the bindings expose.
//!
//! *Current directory.* `vte.cwd` needs `ref_termprop_uri()`, which `vte4` 0.10
//! leaves ungenerated. The deprecated `current_directory_uri()` still works and
//! nothing here uses either.
//!
//! Retire the feature gate once no supported target is below VTE 0.78, and keep the
//! module: the wrapper around the ephemeral read below is worth having regardless.

#[cfg(feature = "vte-0-78")]
use std::cell::RefCell;
#[cfg(feature = "vte-0-78")]
use std::rc::Rc;

#[cfg(feature = "vte-0-78")]
use vte4::Terminal;

#[cfg(feature = "vte-0-78")]
use super::mark_registry::MarkRegistry;

/// The termprop VTE sets when a command run at the prompt has returned.
///
/// Spelled out rather than taken from `vte4-sys`, which this crate does not depend
/// on directly. Matches `VTE_TERMPROP_SHELL_POSTEXEC`.
#[cfg(feature = "vte-0-78")]
const SHELL_POSTEXEC: &str = "vte.shell.postexec";

/// Calls `on_finished` each time the remote shell reports a command has returned.
///
/// The argument is the command's exit code, or `None` when the shell signalled the
/// event without one. Returns `false` on a build that cannot observe termprops, so
/// a caller can tell "not wired" from "wired and quiet".
///
/// Requires OSC 133 shell integration on the *remote* side — `vte.sh` sourced, or
/// an equivalent. Without it this never fires, which is why the setting that
/// depends on it says so.
#[cfg(feature = "vte-0-78")]
pub fn connect_command_finished<F>(terminal: &Terminal, on_finished: F) -> bool
where
    F: Fn(Option<u64>) + 'static,
{
    use vte4::prelude::TerminalExt;

    // Subscribed with a detail so GLib filters by name for us: this fires only for
    // `vte.shell.postexec` and not for every termprop the remote side sets.
    terminal.connect_termprop_changed(Some(SHELL_POSTEXEC), move |terminal, _name| {
        // The value is ephemeral — VTE holds it only for the duration of this
        // emission, so reading it here is not an optimisation but the only place it
        // can be read at all.
        on_finished(terminal.termprop_uint(SHELL_POSTEXEC));
    });
    true
}

/// Calls `on_finished` each time the remote shell reports a command has returned.
///
/// This build has no termprop support, so nothing is connected and `false` is
/// returned. See the module docs.
#[cfg(not(feature = "vte-0-78"))]
pub fn connect_command_finished<F>(_terminal: &vte4::Terminal, _on_finished: F) -> bool
where
    F: Fn(Option<u64>) + 'static,
{
    false
}

/// Feeds a session's [`MarkRegistry`] from the remote shell's OSC 133 events.
///
/// On every `vte.shell.postexec` — the shell reporting that a command run at the
/// prompt has returned — this records a prompt mark at the cursor's current row
/// and attaches the command's exit code to it. vte4 0.10 binds only the
/// `postexec` termprop, not a separate prompt-start (`A`) one, so the cursor row
/// at command-end is the anchor: it is the line the shell has returned the prompt
/// to, which is exactly the row navigation should jump to. The registry collapses
/// a repeat at the same row and ignores an out-of-order one, so an empty Enter or
/// a redraw does not create phantom marks.
///
/// Returns `false` on a build that cannot observe termprops (so a caller can tell
/// "not wired" from "wired and quiet"), same contract as
/// [`connect_command_finished`]. Requires OSC 133 shell integration on the remote
/// side; without it the handler never fires and the registry stays empty, which
/// is what keeps the whole feature dormant on a host without `vte.sh`.
#[cfg(feature = "vte-0-78")]
pub fn connect_shell_integration(terminal: &Terminal, registry: Rc<RefCell<MarkRegistry>>) -> bool {
    // `vadjustment` comes from gtk4's ScrollableExt (VTE's Terminal is a
    // GtkScrollable); `cursor_position`/`termprop_uint` from vte4's TerminalExt.
    use gtk4::prelude::*;
    use vte4::prelude::TerminalExt;

    terminal.connect_termprop_changed(Some(SHELL_POSTEXEC), move |terminal, _name| {
        // cursor_position() returns (column, row) in the live grid; the row is
        // the line the prompt has returned to. Offset by the scrollback already
        // above the viewport so the stored row is in the same buffer coordinate
        // space MarkRegistry::trim_below and the navigation scroll use. The
        // adjustment is absent only before the widget is realized, which cannot
        // be the case while the shell is emitting output — treat it as zero.
        let (_col, cursor_row) = terminal.cursor_position();
        let scroll_offset = terminal.vadjustment().map_or(0, |adj| adj.value() as i64);
        let buffer_row = scroll_offset + cursor_row;

        let exit_code = terminal.termprop_uint(SHELL_POSTEXEC);

        let mut reg = registry.borrow_mut();
        reg.record_prompt(buffer_row);
        reg.set_last_exit(exit_code);
        tracing::debug!(
            buffer_row,
            scroll_offset,
            cursor_row,
            ?exit_code,
            registry_empty = reg.is_empty(),
            "OSC133 postexec: recorded prompt mark"
        );
    });
    true
}

/// Feeds a session's [`MarkRegistry`](super::mark_registry::MarkRegistry) from
/// the remote shell's OSC 133 events.
///
/// This build has no termprop support, so nothing is connected and `false` is
/// returned. See the module docs.
#[cfg(not(feature = "vte-0-78"))]
pub fn connect_shell_integration(
    _terminal: &vte4::Terminal,
    _registry: std::rc::Rc<std::cell::RefCell<super::mark_registry::MarkRegistry>>,
) -> bool {
    false
}

// There is deliberately no test module here.
//
// Everything this file does needs a live `VteTerminal`, which needs a GTK display,
// and the one thing that could be checked without one — that `cfg!(feature =
// "vte-0-78")` equals itself — asserts nothing while looking like coverage. What is
// worth pinning is that a finished command produces the right notification, and that
// lives where the decision does: `ActivityCoordinator::on_command_finished`, which is
// plain state and is tested in `activity_coordinator.rs`.
