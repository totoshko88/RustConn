#!/usr/bin/env bash
# PreToolUse guard: block writes that violate the two hardest-to-reverse
# architectural invariants.
#
#   1. rustconn-core / rustconn-cli are GUI-free — no gtk4 / adw / vte4.
#   2. `unsafe` lives only in the sanctioned rustconn-*-sys crates
#      (unsafe_code = "deny" workspace-wide, Cargo.toml, re-opened by a
#      crate-level #![expect(unsafe_code, …)] in each of those crates).
#
# Reads the PreToolUse JSON on stdin, exits 2 with an explanation on stderr to
# block, exits 0 silently otherwise.
#
# Why a command hook and not an agent prompt: an agent action cannot decide, so
# every write was cancelled, acknowledged and re-issued — two round trips per
# edit, on every edit. This does the same two greps for free.
#
# Fails OPEN. A guard that blocks every write because jq changed its output
# format would be far worse than one that occasionally misses a violation, and
# both invariants are also hard compile errors, so the build is the real net.

set -uo pipefail

# Anything unexpected -> allow the write.
trap 'exit 0' ERR

payload=$(cat) || exit 0
command -v jq >/dev/null 2>&1 || exit 0

# fs_write / fs_append / str_replace use `path`, delete_file uses `targetFile`,
# the KiroGraph write tools use `file`.
path=$(printf '%s' "$payload" | jq -r '.tool_input.path // .tool_input.targetFile // .tool_input.file // ""' 2>/dev/null) || exit 0
[ -n "$path" ] || exit 0
case "$path" in
*.rs) ;;
*) exit 0 ;;
esac

# The text being introduced: `text` (fs_write / fs_append), `newStr`
# (str_replace), `new_str` / `pairs[].new_str` / `content` / `rewrite` (the
# KiroGraph str_replace, multi_str_replace, insert_at and ast_grep_rewrite).
# delete_file has none of them: nothing is being introduced, so nothing to check.
content=$(printf '%s' "$payload" | jq -r '
    .tool_input | .text // .newStr // .new_str // .content // .rewrite
    // ([.pairs[]?.new_str | strings] | join("\n")) // ""' 2>/dev/null) || exit 0
[ -n "$content" ] || exit 0

# Normalise to a repo-relative path so the crate prefixes below match whether
# the caller passed an absolute or a relative path — and regardless of what the
# checkout directory happens to be called or which cwd the hook runs in. Asking
# git for the root beats matching on a directory name, which the previous
# `${rel##*/RustConn/}` did: a clone named `rustconn/` or `RustConn-fork/`, or a
# cwd outside the repo, left `rel` absolute, so invariant 1 was skipped and
# invariant 2 blocked legitimate `rustconn-*-sys` edits; and because the strip
# was greedy, a second `/RustConn/` anywhere below the root cut the crate
# prefix off and disabled both.
abs=$path
case "$abs" in
/*) ;;
*) abs=$PWD/$abs ;;
esac

# Anchor the lookup at the nearest existing ancestor: the target file usually
# does not exist yet (this hook runs *before* the write), and neither may its
# directory.
anchor=$(dirname "$abs")
while [ "$anchor" != "/" ] && [ ! -d "$anchor" ]; do
    anchor=$(dirname "$anchor")
done
[ -d "$anchor" ] || anchor=$PWD

# `|| true` keeps the ERR trap out of it: no git, not a checkout, or a
# dubious-ownership refusal all just leave repo_root empty.
repo_root=$(git -C "$anchor" rev-parse --show-toplevel 2>/dev/null || true)

if [ -n "$repo_root" ]; then
    rel=${abs#"$repo_root"/}
else
    # Fall back to the cwd prefix. If that leaves `rel` absolute, the crate arms
    # below simply do not match and the write is allowed — the fail-open
    # contract at the top of this file.
    rel=${abs#"$PWD"/}
fi

fail() {
    printf 'crate-boundary-guard: %s\n' "$1" >&2
    printf '  file: %s\n' "$rel" >&2
    printf '  offending line(s):\n' >&2
    printf '%s\n' "$2" | sed 's/^/    /' >&2
    exit 2
}

# --- Invariant 1: no GUI toolkit in the GUI-free crates ---------------------
# Their own tests/ are exempt, matching the original guard.
case "$rel" in
rustconn-core/tests/* | rustconn-cli/tests/*) ;;
rustconn-core/* | rustconn-cli/*)
    gui=$(printf '%s' "$content" | grep -nE '(^|[^[:alnum:]_])(use[[:space:]]+(gtk4|adw|libadwaita|vte4)|(gtk4|adw|vte4)::)' || true)
    if [ -n "$gui" ]; then
        fail 'GUI toolkit import in a GUI-free crate. Move this code to rustconn/.' "$gui"
    fi
    ;;
esac

# --- Invariant 2: unsafe only in the sanctioned FFI crates ------------------
# The pattern is deliberately the crate-name shape (rustconn-<x>-sys), not a
# hardcoded list: the point of the invariant is that unsafe stays inside a
# small, separately reviewable FFI crate, not that there is exactly one of
# them. There are five today (rustconn-pty-sys, rustconn-locale-sys,
# rustconn-env-sys, rustconn-dock-sys, rustconn-lock-sys). Each re-opens the
# lint with a crate-level #![expect(unsafe_code, reason = "…")] in its own
# src/lib.rs, so adding one is a visible, reviewed act.
#
# `extern` is in the keyword list for edition-2024 `unsafe extern "C" { … }` and
# `unsafe extern "C" fn`. The compiler's `unsafe_code = "deny"` already rejects
# both, so this is defence-in-depth. The `*/rustconn-*-sys/*` arm is a
# belt-and-braces exemption for the case where the normalisation above could not
# find a repo root: better to miss a violation than to block a legitimate edit to
# an FFI crate, per the fail-open contract.
case "$rel" in
rustconn-*-sys/* | */rustconn-*-sys/*) ;;
*)
    bad=$(printf '%s' "$content" | grep -nE '(^|[^[:alnum:]_])unsafe[[:space:]]+(fn|impl|trait|extern)|(^|[^[:alnum:]_])unsafe[[:space:]]*\{' || true)
    if [ -n "$bad" ]; then
        fail 'unsafe outside a rustconn-*-sys crate (unsafe_code = "deny"). New FFI belongs in a dedicated rustconn-*-sys crate.' "$bad"
    fi
    ;;
esac

exit 0
