#!/usr/bin/env bash
# The mechanical half of the Definition of Done, as one command with one log and
# one exit code.
#
# Steering `verification-checklist.md` and `quality-gate.md` describe this in
# prose, which meant every agent and every developer reassembled it from the
# checklist by hand, in a different order, with a different idea of which steps
# were optional. The parts that need judgement — is the new code in the right
# crate, is the ceiling in that ponytail marker honest, does this string need
# i18n — stay in those documents. This script is only the part a machine can
# decide.
#
# Usage:
#   scripts/verify.sh            fast gates + fmt + machete + clippy + rustdoc
#   scripts/verify.sh --quick    fast gates only (right for .md / .po-only work)
#   scripts/verify.sh --tests    also cargo test --workspace  (~2.5 min wall)
#   scripts/verify.sh --cached   do NOT clean the workspace crates first, so
#                                clippy may report a cache hit as a pass
#
# Cleaning the workspace crates before clippy is the default, and `--cached` is
# the escape hatch rather than `--fresh` being the opt-in it used to be. The
# Definition of Done requires a clippy run that actually re-checked; this script
# used to note a cache hit as a WARN and then exit 0, so the tool that exists to
# decide whether the work is done reported success for a run that verified
# nothing. Only workspace crates are cleaned, which is seconds — cleaning
# dependencies too would turn a 40-second re-check into a five-minute one for no
# extra coverage, since a dependency cannot acquire a warning without a lock
# change.
#
# Everything goes to target/verify.log. Cargo output is redirected, never piped:
# a pipe through tail/grep is the main way the output ends up lost entirely.
#
# What this deliberately does NOT check, because something else already does:
#
#   * dbg!/todo!/println!/eprintln! — the workspace denies dbg_macro, todo,
#     print_stdout and print_stderr in clippy, so a leftover is a clippy error.
#     A separate grep would also false-positive on rustconn-cli, where printing
#     is the interface.
#   * unsafe outside the -sys crates — unsafe_code = "deny" makes that a compile
#     error, which clippy surfaces.
#
# The GUI-import check below is the one deliberate duplicate: it is the most
# common boundary break and it fails in a second rather than after a full clippy.
#
# CI does not call this script, on purpose. `.github/workflows/ci.yml` splits the
# same work across parallel jobs — fmt, clippy, hygiene, i18n, test, test-core,
# property-tests, msrv, cargo-deny — which finish sooner than one serial run and
# tell you which gate failed from the job name alone. This script is for a
# developer or an agent working locally, where one command and one log is the
# point. Do not "unify" them by replacing the jobs with this.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

quick=0
tests=0
fresh=1
for arg in "$@"; do
    case "$arg" in
    --quick) quick=1 ;;
    --tests) tests=1 ;;
    --cached) fresh=0 ;;
    # Accepted so an existing habit or script does not break; it is now the
    # default, so there is nothing to turn on.
    --fresh) fresh=1 ;;
    -h | --help)
        sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *)
        printf 'unknown option: %s (see --help)\n' "$arg" >&2
        exit 2
        ;;
    esac
done

CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
command -v "$CARGO" >/dev/null 2>&1 || CARGO=cargo

log="target/verify.log"
mkdir -p target
: >"$log"

failed=0
declare -a results=()

say() { printf '%s\n' "$*"; }

record() {
    # record <name> <status>
    results+=("$2	$1")
    [ "$2" = FAIL ] && failed=$((failed + 1))
    return 0
}

run_gate() {
    # run_gate <name> <command...>
    local name="$1"
    shift
    printf '\n===== %s =====\n' "$name" >>"$log"
    if "$@" >>"$log" 2>&1; then
        say "  ok    $name"
        record "$name" OK
    else
        say "  FAIL  $name"
        record "$name" FAIL
    fi
}

skip_gate() {
    say "  skip  $1 ($2)"
    results+=("SKIP	$1 — $2")
}

# ── Serialisation ────────────────────────────────────────────────────────────
# Two cargo runs share the target-dir lock and both look like a hang.
if pgrep -f "[c]argo (build|test|clippy|check)" >/dev/null 2>&1; then
    say 'FAIL: another cargo run is in progress — it holds the target-dir lock.'
    say '      Wait for it, or check with: pgrep -af cargo'
    exit 1
fi

say "RustConn verify — log: $log"
say ''
say 'Fast gates'

# ── Fast gates ───────────────────────────────────────────────────────────────

if command -v typos >/dev/null 2>&1; then
    run_gate 'typos' typos
else
    skip_gate 'typos' 'not installed'
fi

# `check-jump-host-wiring` is not an i18n check but belongs with these: it is a
# grep over a connection between two files, which is what this group is, and it
# needs no toolchain.
#
# `test-hooks` asserts what the .kiro/hooks guards actually do with a payload —
# the release, push, crate-boundary and cargo-serialisation rules — so a claim in
# hooks-map.md that stops being true fails here instead of at the next incident.
for s in check-potfiles check-i18n-escapes check-po-complete check-ai-docs \
    check-jump-host-wiring test-hooks; do
    if [ -x "scripts/$s.sh" ]; then
        run_gate "$s" "scripts/$s.sh"
    else
        skip_gate "$s" 'script missing'
    fi
done

# GUI imports must not reach the headless crates.
gui_leak() {
    local hits
    hits=$(grep -rnE '^[[:space:]]*use[[:space:]]+(gtk4?|adw|vte4)(::|[[:space:]]*;)' \
        --include='*.rs' rustconn-core/src rustconn-cli/src 2>/dev/null || true)
    if [ -n "$hits" ]; then
        printf 'GUI imports found in a headless crate:\n%s\n' "$hits"
        return 1
    fi
    return 0
}
run_gate 'crate boundaries (no GUI imports in core/cli)' gui_leak

if [ "$quick" -eq 1 ]; then
    say ''
    say '--quick: skipping fmt, machete, clippy and tests.'
else
    say ''
    say 'Cargo gates'

    if [ "$fresh" -eq 1 ]; then
        # Only the workspace crates. Cleaning dependencies too would turn a
        # 40-second re-check into a five-minute one for no extra coverage.
        mapfile -t members < <(
            awk '/^members = \[/,/^\]/' Cargo.toml |
                sed -n 's/^[[:space:]]*"\([^"]*\)".*/\1/p'
        )
        clean_args=()
        for m in "${members[@]}"; do clean_args+=(-p "$m"); done
        say '  ...  cleaning workspace crates so clippy re-checks for real'
        "$CARGO" clean "${clean_args[@]}" >>"$log" 2>&1 || true
    fi

    # fmt is captured to its own file, not just recorded pass/fail, so the
    # parse-error preflight below can read it. A `cargo fmt --check` failure has
    # two very different meanings: a formatting *diff* (rustfmt would restyle
    # some lines) versus rustfmt being unable to *parse* the file at all
    # ("error: expected item" / "cannot parse"), which means the tree does not
    # compile. Continuing to a ~2.5 min `cargo test --workspace` against code
    # that cannot parse wastes that time for a guaranteed failure — the exact
    # trap hit on 2026-09-22, when a corrupted file (an `impl` nested in another)
    # sent verify.sh into a full test run it could never pass. clippy still runs
    # (it pinpoints the parse error far better than fmt), only the long test
    # phase is skipped.
    fmt_parse_error=0
    fmt_log="target/verify-fmt.log"
    printf '\n===== %s =====\n' 'cargo fmt --check' >>"$log"
    if "$CARGO" fmt --all -- --check >"$fmt_log" 2>&1; then
        say '  ok    cargo fmt --check'
        record 'cargo fmt --check' OK
    else
        say '  FAIL  cargo fmt --check'
        record 'cargo fmt --check' FAIL
        # A parse failure prints an `error:` line; a mere formatting diff does
        # not (rustfmt emits a unified diff with no `error:` prefix).
        if grep -qE '^error(\[|:)|cannot parse|expected item, found' "$fmt_log"; then
            fmt_parse_error=1
        fi
    fi
    cat "$fmt_log" >>"$log"

    if "$CARGO" machete --version >/dev/null 2>&1; then
        run_gate 'cargo machete' "$CARGO" machete
    else
        skip_gate 'cargo machete' 'not installed'
    fi

    # --all-targets, never --all-features: the latter enables a gtk3 path that
    # fails at build time on a missing gdk-3.0.pc.
    #
    # `-- -D warnings` matches what the CI clippy job runs. Without it clippy
    # exits 0 on a pedantic warning and this gate reported `ok` for a tree CI
    # would reject — the Definition of Done says zero warnings, so the gate has
    # to fail on one.
    #
    # On macOS the default feature set cannot be built at all: `web-embedded` is
    # in `default` and pulls WebKitGTK 6.0 through webkit6, whose -sys build
    # scripts fail on a missing javascriptcoregtk-6.0.pc and libsoup-3.0.pc. The
    # feature's own comment in rustconn/Cargo.toml says "Linux only". So this
    # gate reported a failed clippy run on the maintainer's own machine for a
    # reason that has nothing to do with the tree — a gate that cannot run where
    # it is needed. It now uses the same canonical feature set the macOS bundle is
    # built with, read from macos-build.sh so there is one list and not two.
    clippy_features=()
    if [ "$(uname -s)" = Darwin ] && [ -x scripts/macos-build.sh ]; then
        macos_features=$(./scripts/macos-build.sh --print-features 2>/dev/null | tail -n 1)
        if [ -n "$macos_features" ]; then
            clippy_features=(--no-default-features --features "$macos_features")
            say "  ...  macOS: clippy over the bundle's feature set ($macos_features)"
        fi
    fi

    # Clippy's output goes to a file of its own as well as to the shared log, so
    # the "did it compile anything" check below can read a file that holds exactly
    # one clippy run.
    #
    # It used to record the shared log's line count and then `tail -n +N` after the
    # run. That was wrong twice: `wc -l` pads its output on BSD, so `tail` rejected
    # `-n +      18` as an illegal offset and the check scanned nothing; and once
    # the padding was stripped the offset still came out at the whole file, so the
    # check read one line and again found nothing. Both times it reported that
    # clippy had compiled nothing while the log plainly showed several hundred
    # `Checking` lines — a gate lying in the direction of failure, which is the
    # cheap direction, but a gate that has to be argued with is a gate that gets
    # deleted. There is no offset to get wrong now.
    clippy_log="target/verify-clippy.log"
    printf '\n===== %s =====\n' 'cargo clippy --all-targets' >>"$log"
    if "$CARGO" clippy --all-targets "${clippy_features[@]}" -- -D warnings \
        >"$clippy_log" 2>&1; then
        say '  ok    cargo clippy --all-targets'
        record 'cargo clippy --all-targets' OK
    else
        say '  FAIL  cargo clippy --all-targets'
        record 'cargo clippy --all-targets' FAIL
    fi
    cat "$clippy_log" >>"$log"

    # A cache hit prints "Finished ... in 0.2s" and reports zero warnings without
    # looking at anything. That is not a pass, and until 0.21.0 this recorded it
    # as a WARN — which never incremented the failure count, so the script exited
    # 0 and the run looked done. With the clean above now the default, reaching
    # here means something is wrong rather than merely unlucky, so it fails.
    # Under --cached it stays a warning, because the caller asked for the fast
    # path and is entitled to know what they gave up rather than be refused.
    #
    # ANSI escapes are stripped before looking. Cargo colours its output whenever
    # CARGO_TERM_COLOR=always, and then a progress line reads
    #   \e[1m\e[92m    Checking\e[0m rustconn-core v0.21.0 (...)
    # where the first escape precedes the indentation — so `^[[:space:]]*` never
    # reaches the verb — and a second sits between the verb and the crate name,
    # so the trailing space cannot match either. Nothing exports that variable
    # locally, which is why this pattern looked fine here for a whole release
    # while the identical copy in the CI workflow, where the variable *is* set,
    # failed the v0.21.0 release commit with clippy green and zero warnings.
    # The ESC comes from printf rather than being spelled in the pattern so the
    # sed works under BSD as well as GNU.
    #
    # The strip writes a file rather than piping into grep, because this script
    # runs under `set -o pipefail` and `grep -q` exits on its first match: the
    # `sed` upstream is then still writing into a closed descriptor, dies of
    # SIGPIPE, and pipefail reports the pipeline as failed *because* the grep
    # succeeded. That is not theoretical — the piped version of this fix turned
    # the CI copy permanently red. It survived one run here only by winning the
    # race, which is the worst way for a gate to be wrong: right until it is not.
    esc=$(printf '\033')
    clippy_plain="target/verify-clippy-plain.log"
    sed "s/${esc}\[[0-9;]*[a-zA-Z]//g" "$clippy_log" >"$clippy_plain"
    if ! grep -qE '^[[:space:]]*(Checking|Compiling) ' "$clippy_plain"; then
        if [ "$fresh" -eq 1 ]; then
            say '  FAIL  clippy compiled nothing even after cleaning — that run verified nothing.'
            results+=("FAIL	cargo clippy — nothing re-checked")
            failed=$((failed + 1))
        else
            say '  WARN  clippy did not compile anything — that run verified nothing.'
            say '        Drop --cached to force a real re-check.'
            results+=("WARN	cargo clippy — cache hit, nothing re-checked")
        fi
    fi

    # Every packaging build — deb, RPM, AppImage, Flatpak, snap — compiles the
    # CLI as `-p rustconn-cli --features full`. The gate above uses default
    # features, and `rustconn-cli` defaults to *nothing*: the `client-launch` and
    # `secret-management` modules are `#[cfg(feature = ...)]` and are not
    # compiled at all, so a type error inside them is invisible here.
    #
    # That is not hypothetical. v0.20.9 was tagged with a three-argument call to
    # a four-argument `build_sftp_browser_uri` inside the `client-launch` block:
    # every local gate was green, and all four packaging jobs failed on it. CI
    # does cover it, via `cargo test -p rustconn-cli --features full`, but that
    # job runs on the push that carries the tag — too late to stop the release,
    # and on that day Actions was down and it never ran at all.
    run_gate 'cargo clippy -p rustconn-cli --features full' \
        "$CARGO" clippy -p rustconn-cli --features full --all-targets -- -D warnings

    # rustdoc lints (broken intra-doc links, unclosed HTML, bare URLs) are only
    # evaluated by rustdoc, so clippy above cannot see them. Same feature set as
    # clippy, for the same macOS reason. --document-private-items because these
    # docs are read by people working on the code, not by crate consumers.
    run_gate 'cargo doc (-D warnings)' \
        env RUSTDOCFLAGS='-D warnings' \
        "$CARGO" doc --no-deps --document-private-items "${clippy_features[@]}"

    if [ "$tests" -eq 1 ] && [ "$fmt_parse_error" -eq 1 ]; then
        # See the preflight note at the fmt gate: the tree does not parse, so a
        # ~2.5 min test run is a guaranteed, expensive failure. clippy above has
        # already recorded the real fault; skip the tests rather than burn the
        # wall time. This is the one place the "collect every failure" design
        # bends, and only for the long phase — the parse error is still a FAIL in
        # the summary via the fmt gate.
        skip_gate 'cargo test (workspace + cli)' 'tree does not parse — see cargo fmt --check'
    elif [ "$tests" -eq 1 ]; then
        say '  ...  cargo test --workspace (~2.5 min)'
        if [ "${#clippy_features[@]}" -gt 0 ]; then
            # Same reason the clippy gate above substitutes a feature set on
            # macOS: `--workspace` resolves `rustconn`'s default features, and
            # `web-embedded` is one of them, so the run dies in the `webkit6`
            # build scripts on a missing javascriptcoregtk-6.0.pc — nothing to do
            # with the tree. The clippy gate was fixed for this earlier in 0.21.0
            # and the test gate was not, so `verify.sh --tests` still reported a
            # failure on the maintainer's own platform.
            #
            # Split per crate rather than `--workspace` with the flags, because
            # `--no-default-features` applies to every selected package and the
            # other crates' defaults are wanted.
            run_gate 'cargo test -p rustconn-core' "$CARGO" test -p rustconn-core
            run_gate 'cargo test -p rustconn (macOS features)' \
                "$CARGO" test -p rustconn "${clippy_features[@]}"
            run_gate 'cargo test the -sys crates' \
                "$CARGO" test -p rustconn-pty-sys -p rustconn-locale-sys \
                -p rustconn-env-sys -p rustconn-dock-sys
        else
            run_gate 'cargo test --workspace' "$CARGO" test --workspace
        fi
        run_gate 'cargo test -p rustconn-cli --features full' \
            "$CARGO" test -p rustconn-cli --features full
    fi
fi

# ── Summary ──────────────────────────────────────────────────────────────────
say ''
say 'Summary'
for r in "${results[@]}"; do
    printf '  %s\n' "$(printf '%s' "$r" | sed 's/\t/  /')"
done

say ''
if [ "$failed" -gt 0 ]; then
    say "$failed gate(s) failed. Details in $log"
    exit 1
fi
say "All gates passed. Details in $log"
exit 0
