#!/usr/bin/env bash
# Regression suite for the Kiro hook scripts in .kiro/hooks/bin and the matchers
# in .kiro/hooks/*.json.
#
# Each guard gets a JSON payload on stdin, shaped the way the hook engine sends
# it, and the suite asserts the exit code (or, for the ask-gate, the decision on
# stdout). Nothing a payload names is ever executed. Process state that a guard
# consults — a running cargo, a running verify.sh — is simulated with a `pgrep`
# shim, so the result does not depend on what else is running, including the
# verify.sh that calls this suite. The edit journal is redirected to a scratch
# file through KIRO_EDIT_JOURNAL, so the real one is never touched.
#
# Why this exists: hooks-map.md recorded a guard claim that "was never re-tested
# against the script", and the 2026-09-28 audit found seven bypasses of
# release-manual-only-guard — the PATH-prefixed form that shell-environment.md
# itself prescribes among them — plus a false positive in agent-model-guard and
# whole tool families no matcher covered. None of that was visible from reading
# the code, and one bypass a reading predicted did not exist. A behaviour
# hooks-map.md states about a guard should have a row here.
#
# Usage: scripts/test-hooks.sh [-v]      -v also prints the passing cases
# Needs bash, jq and git. Without jq it skips: every guard fails open without jq,
# so there is nothing meaningful to assert.

set -uo pipefail

repo=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo 'test-hooks: not inside a git checkout' >&2
    exit 1
}
cd "$repo" || exit 1

if ! command -v jq >/dev/null 2>&1; then
    echo 'test-hooks: skip, jq not installed (every guard fails open without it)'
    exit 0
fi

verbose=0
[ "${1:-}" = "-v" ] && verbose=1

bin=${HOOKS_BIN:-.kiro/hooks/bin}
work=$(mktemp -d "${TMPDIR:-/tmp}/rustconn-test-hooks.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT

# pgrep shim: prints $FAKE_PGREP_PIDS and succeeds, or prints nothing and fails.
mkdir -p "$work/shim"
cat >"$work/shim/pgrep" <<'EOF'
#!/usr/bin/env bash
[ -n "${FAKE_PGREP_PIDS:-}" ] || exit 1
printf '%s\n' $FAKE_PGREP_PIDS
EOF
chmod +x "$work/shim/pgrep"

pass=0
fail=0
journal="$work/journal"

report() { # report ok|FAIL <label> [detail]
    if [ "$1" = ok ]; then
        pass=$((pass + 1))
        [ "$verbose" -eq 1 ] && printf '  ok    %s\n' "$2"
    else
        fail=$((fail + 1))
        printf '  FAIL  %s  (%s)\n' "$2" "${3:-}"
    fi
    return 0
}

# run_hook <script> <payload> [pids]   -> sets $rc and $out
run_hook() {
    local tmp
    tmp=$(mktemp -d "$work/tmp.XXXXXX")
    out=$(printf '%s' "$2" | env PATH="$work/shim:$PATH" TMPDIR="$tmp" \
        FAKE_PGREP_PIDS="${3:-}" KIRO_EDIT_JOURNAL="$journal" \
        KIRO_SESSION_REPORT="$work/report" \
        "$bin/$1" 2>/dev/null)
    rc=$?
}

expect_exit() { # expect_exit <code> <script> <label> <payload> [pids]
    run_hook "$2" "$4" "${5:-}"
    if [ "$rc" = "$1" ]; then
        report ok "$2: $3"
    else
        report FAIL "$2: $3" "expected exit $1, got $rc"
    fi
}

shell_payload() { # shell_payload <command> [timeout-ms] [tool-name]
    jq -cn --arg c "$1" --argjson t "${2:-0}" --arg tool "${3:-execute_bash}" '
        {tool_name: $tool, tool_input: ({command: $c} + (if $t > 0 then {timeout: $t} else {} end))}'
}

write_payload() { # write_payload <tool> <tool_input-json>
    jq -cn --arg tool "$1" --argjson in "$2" '{tool_name: $tool, tool_input: $in}'
}

# ── release-manual-only-guard ────────────────────────────────────────────────
g=release-manual-only-guard.sh
while IFS= read -r c; do
    [ -n "$c" ] && expect_exit 0 "$g" "allow  $c" "$(shell_payload "$c")"
done <<'EOF'
./scripts/release.sh --dry-run
PATH="$HOME/.cargo/bin:$PATH" ./scripts/release.sh --dry-run
./scripts/release.sh --dry-run > target/dryrun.log 2>&1; echo $? > target/dryrun.rc
./scripts/release.sh --help
git push --dry-run origin main
git push -n origin main
echo "then git push it"
echo "./scripts/release.sh --yes"
git tag -l
git tag -d v0.22.9
git log --oneline -3; git status --short
grep -n scripts/release.sh docs/CI_BUILD_FLOW.md
git commit -m "fix: release.sh --yes is refused"
EOF
while IFS= read -r c; do
    [ -n "$c" ] && expect_exit 2 "$g" "deny   $c" "$(shell_payload "$c")"
done <<'EOF'
./scripts/release.sh
./scripts/release.sh --yes
./scripts/release.sh --dry-run --yes
cd /home/totoshko88/Documents/RustConn && ./scripts/release.sh --yes
nohup sh -c "./scripts/release.sh --yes"
bash -c './scripts/release.sh --yes'
bash scripts/release.sh --yes
PATH="$HOME/.cargo/bin:$PATH" ./scripts/release.sh --yes
PATH="$HOME/.cargo/bin:$PATH" ./scripts/release.sh
env PATH=/x ./scripts/release.sh --yes
timeout 900 ./scripts/release.sh --yes
./scripts/release.sh --yes; df -h
./scripts/release.sh && git push --dry-run origin main
./scripts/release.sh --dry-run; git push origin main
./scripts/release.sh --dry-run ; git push origin main
cd scripts && ./release.sh --yes
eval "./scripts/release.sh --yes"
echo ok $(./scripts/release.sh --yes)
git push origin main
git push origin main; echo -n x
git push --tags
git -C /tmp/x push origin HEAD
git tag v0.22.9
git tag -a v0.22.9 -m "release"
EOF

# ── bash-serialization-guard ─────────────────────────────────────────────────
g=bash-serialization-guard.sh
expect_exit 0 "$g" 'allow  pgrep -af cargo' "$(shell_payload 'pgrep -af cargo')"
expect_exit 0 "$g" 'allow  a short sleep outside a loop' "$(shell_payload 'sleep 2')"
expect_exit 0 "$g" 'allow  cargo clippy with timeout headroom' \
    "$(shell_payload 'cargo clippy --all-targets > target/c.log 2>&1' 900000)"
expect_exit 0 "$g" 'allow  verify.sh while no runner is alive' \
    "$(shell_payload './scripts/verify.sh --tests > target/v.log 2>&1' 900000)"
expect_exit 0 "$g" 'allow  cargo test started with control_bash_process' \
    "$(jq -cn '{tool_name: "control_bash_process",
        tool_input: {action: "start", command: "cargo test --workspace > target/t.log 2>&1"}}')"
expect_exit 2 "$g" 'deny   R1 a long sleep' "$(shell_payload 'sleep 115; echo W10')"
expect_exit 2 "$g" 'deny   R1 a sleep inside a loop' \
    "$(shell_payload 'while [ ! -f target/x.rc ]; do sleep 1; done')"
expect_exit 2 "$g" 'deny   R2 piped cargo output' \
    "$(shell_payload 'cargo test --workspace | tail -5' 900000)"
expect_exit 2 "$g" 'deny   R3 a second cargo while one runs' \
    "$(shell_payload 'cargo clippy --all-targets > target/c.log 2>&1' 900000)" 4242
expect_exit 2 "$g" 'deny   R4 cargo test with the default timeout' \
    "$(shell_payload 'cargo test --workspace > target/t.log 2>&1')"
expect_exit 2 "$g" 'deny   R5 a second verify.sh while one runs' \
    "$(shell_payload './scripts/verify.sh --quick > target/v.log 2>&1' 900000)" 4242
expect_exit 2 "$g" 'deny   R1 through the kirograph_exec spelling' \
    "$(shell_payload 'sleep 30' 0 mcp_kirograph_kirograph_exec)"

# ── agent-model-guard ────────────────────────────────────────────────────────
g=agent-model-guard.sh
profile=.kiro/agents/rust-quality-check.md
expect_exit 0 "$g" 'allow  fs_append a rule to an existing profile' \
    "$(write_payload fs_append "$(jq -cn --arg p "$profile" '{path: $p, text: "\n- One more rule.\n"}')")"
expect_exit 2 "$g" 'deny   fs_write a new profile without model' \
    "$(write_payload fs_write '{"path": ".kiro/agents/probe-new.md", "text": "---\nname: probe\n---\nbody\n"}')"
expect_exit 0 "$g" 'allow  fs_write a new profile with model' \
    "$(write_payload fs_write '{"path": ".kiro/agents/probe-new.md", "text": "---\nname: probe\nmodel: claude-haiku-4.5\n---\nbody\n"}')"
expect_exit 2 "$g" 'deny   str_replace that removes model' \
    "$(write_payload str_replace "$(jq -cn --arg p "$profile" '{path: $p, oldStr: "model: qwen3-coder-next", newStr: ""}')")"
expect_exit 0 "$g" 'allow  str_replace that keeps model' \
    "$(write_payload str_replace "$(jq -cn --arg p "$profile" '{path: $p, oldStr: "Be terse.", newStr: "Be terse, always."}')")"
expect_exit 0 "$g" 'allow  a file outside .kiro/agents' \
    "$(write_payload fs_write '{"path": "docs/probe.md", "text": "no model here"}')"

# ── crate-boundary-guard ─────────────────────────────────────────────────────
g=crate-boundary-guard.sh
cb() { # cb <code> <label> <tool> <tool_input-json>
    expect_exit "$1" "$g" "$2" "$(write_payload "$3" "$4")"
}
cb 2 'deny   gtk4 import in rustconn-core' fs_write \
    '{"path": "rustconn-core/src/probe.rs", "text": "use gtk4::prelude::*;\n"}'
cb 2 'deny   adw path in rustconn-cli via str_replace' str_replace \
    '{"path": "rustconn-cli/src/main.rs", "oldStr": "x", "newStr": "let app = adw::Application::new();"}'
cb 0 'allow  gtk4 in rustconn-core tests/' fs_write \
    '{"path": "rustconn-core/tests/probe.rs", "text": "use gtk4::prelude::*;\n"}'
cb 0 'allow  gtk4 in rustconn' fs_write \
    '{"path": "rustconn/src/probe.rs", "text": "use gtk4::prelude::*;\n"}'
cb 2 'deny   unsafe block in rustconn-core' fs_write \
    '{"path": "rustconn-core/src/probe.rs", "text": "unsafe { libc::getpid() };\n"}'
cb 0 'allow  unsafe block in a -sys crate' fs_write \
    '{"path": "rustconn-pty-sys/src/probe.rs", "text": "unsafe { libc::getpid() };\n"}'
cb 2 'deny   unsafe fn in rustconn' fs_write \
    '{"path": "rustconn/src/probe.rs", "text": "unsafe fn raw() {}\n"}'
cb 0 'allow  the word unsafe in a comment' fs_write \
    '{"path": "rustconn-core/src/probe.rs", "text": "// this is not unsafe code\n"}'
cb 0 'allow  delete_file' delete_file '{"targetFile": "rustconn-core/src/probe.rs"}'
cb 2 'deny   gtk4 through kirograph_str_replace' mcp_kirograph_kirograph_str_replace \
    '{"file": "rustconn-core/src/probe.rs", "old_str": "a", "new_str": "use gtk4::prelude::*;"}'
cb 2 'deny   unsafe through kirograph_insert_at' mcp_kirograph_kirograph_insert_at \
    '{"file": "rustconn-cli/src/main.rs", "content": "unsafe { x() }", "line": 1}'
cb 2 'deny   vte4 through kirograph_multi_str_replace' mcp_kirograph_kirograph_multi_str_replace \
    '{"file": "rustconn-core/src/lib.rs", "pairs": [{"old_str": "a", "new_str": "use vte4::TerminalExt;"}]}'
cb 2 'deny   unsafe through kirograph_ast_grep_rewrite' mcp_kirograph_kirograph_ast_grep_rewrite \
    '{"file": "rustconn-core/src/lib.rs", "pattern": "$A", "rewrite": "unsafe { $A }"}'

# ── edit-journal ─────────────────────────────────────────────────────────────
g=edit-journal.sh
ej() { # ej present|absent|once <label> <tool> <tool_input-json> <expected-path>
    local want=$1 label=$2 n
    run_hook "$g" "$(write_payload "$3" "$4")"
    n=$(grep -cxF -- "$5" "$journal" 2>/dev/null || true)
    case "$want:${n:-0}" in
    present:[1-9]* | once:1 | absent:0) report ok "$g: $label" ;;
    *) report FAIL "$g: $label" "want $want, found $5 ${n:-0} time(s)" ;;
    esac
}
: >"$journal"
ej present 'journal an fs_write' fs_write '{"path": "rustconn/src/probe.rs", "text": "x"}' rustconn/src/probe.rs
ej once 'deduplicate a second write' fs_write '{"path": "rustconn/src/probe.rs", "text": "y"}' rustconn/src/probe.rs
ej present 'journal a delete_file' delete_file '{"targetFile": "CHANGELOG.md"}' CHANGELOG.md
ej absent 'skip bookkeeping under target/' fs_write '{"path": "target/probe.log", "text": "x"}' target/probe.log
ej present 'make an absolute path repo-relative' fs_write \
    "$(jq -cn --arg p "$repo/po/uk.po" '{path: $p, text: "x"}')" po/uk.po
ej present 'journal a smart_relocate source' smart_relocate \
    '{"sourcePath": "docs/probe-a.md", "destinationPath": "docs/probe-b.md"}' docs/probe-a.md
ej present 'journal a smart_relocate destination' smart_relocate \
    '{"sourcePath": "docs/probe-a.md", "destinationPath": "docs/probe-b.md"}' docs/probe-b.md
ej present 'journal a semantic_rename' semantic_rename \
    '{"path": "rustconn-core/src/lib.rs", "line": 1, "character": 1, "oldName": "a", "newName": "b"}' rustconn-core/src/lib.rs
ej present 'journal a kirograph write' mcp_kirograph_kirograph_str_replace \
    '{"file": "rustconn-cli/src/main.rs", "old_str": "a", "new_str": "b"}' rustconn-cli/src/main.rs

# ── commit-review-gate ───────────────────────────────────────────────────────
g=commit-review-gate.sh
crg() { # crg ask|pass <label> <journal-line> <command>
    printf '%s\n' "$3" >"$journal"
    run_hook "$g" "$(shell_payload "$4")"
    if printf '%s' "$out" | grep -q '"permissionDecision":"ask"'; then got=ask; else got=pass; fi
    if [ "$got" = "$1" ]; then report ok "$g: $2"; else report FAIL "$g: $2" "want $1, got $got"; fi
}
crg ask 'unsafe review for a -sys crate' rustconn-pty-sys/src/lib.rs 'git commit -m x'
crg ask 'unsafe review for a fifth -sys crate' rustconn-foo-sys/src/lib.rs 'git commit -m x'
crg ask 'security review for credential code' rustconn-core/src/secret/keyring.rs 'git commit -m x'
crg ask 'translation review for po/uk.po' po/uk.po 'git commit -m x'
crg pass 'no review for ordinary GUI code' rustconn/src/app.rs 'git commit -m x'
crg pass 'a dry-run commit records nothing' rustconn-pty-sys/src/lib.rs 'git commit --dry-run'
crg pass 'a mention is not a commit' rustconn-pty-sys/src/lib.rs 'echo "git commit"'
crg ask 'a commit after git add' rustconn-pty-sys/src/lib.rs 'git add a && git commit -m "x"'
crg ask 'an env-prefixed commit' rustconn-pty-sys/src/lib.rs 'GIT_EDITOR=true git commit'

# ── changelog-entry-guard ────────────────────────────────────────────────────
g=changelog-entry-guard.sh
ceg() { # ceg ask|pass <label> <journal-lines> <command>
    printf '%s\n' "$3" >"$journal"
    run_hook "$g" "$(shell_payload "$4")"
    if printf '%s' "$out" | grep -q '"permissionDecision":"ask"'; then got=ask; else got=pass; fi
    if [ "$got" = "$1" ]; then report ok "$g: $2"; else report FAIL "$g: $2" "want $1, got $got"; fi
}
ceg ask 'src change without a CHANGELOG edit' 'rustconn-core/src/spice_client/mod.rs' 'git commit -m x'
ceg ask 'GUI src change without a CHANGELOG edit' 'rustconn/src/window/protocols.rs' 'git commit -m x'
ceg pass 'src change WITH a CHANGELOG edit' 'rustconn-core/src/spice_client/mod.rs
CHANGELOG.md' 'git commit -m x'
ceg pass 'only a test file changed' 'rustconn-core/src/foo_tests.rs' 'git commit -m x'
ceg pass 'only a tests/ file changed' 'rustconn-core/tests/properties/foo.rs' 'git commit -m x'
ceg pass 'only a doc changed' 'docs/AI_DEVELOPMENT.md' 'git commit -m x'
ceg pass 'a dry-run commit records nothing' 'rustconn-core/src/spice_client/mod.rs' 'git commit --dry-run'
ceg pass 'a mention is not a commit' 'rustconn-core/src/spice_client/mod.rs' 'echo "git commit"'
ceg ask 'a commit after git add' 'rustconn-core/src/spice_client/mod.rs' 'git add a && git commit -m x'

# ── doc-claims-scan ──────────────────────────────────────────────────────────
# A NOTE hook: it writes to the session report and always exits 0. The report is
# redirected to "$work/report" (run_hook sets KIRO_SESSION_REPORT), so this suite
# — and verify.sh, which runs it — never writes into the real one.
g=doc-claims-scan.sh
dcs_payload=$(jq -cn --arg f "rustconn-core/src/search/mod.rs" '{file_path: $f}')
expect_exit 0 "$g" 'exits 0 on a real .rs save' "$dcs_payload"
expect_exit 0 "$g" 'exits 0 on a non-rs file' "$(jq -cn '{file_path: "docs/x.md"}')"
expect_exit 0 "$g" 'exits 0 with no file_path' '{}'

# What it reports, against a scratch repo so nothing in this checkout is touched.
sr="$work/scratch-repo"
mkdir -p "$sr/demo/src" "$sr/target/src"
git -C "$sr" init -q 2>/dev/null
cat >"$sr/demo/src/lib.rs" <<'EOF'
/// Committed claim about `ghost_committed`.
pub struct Conn { jump_host_id: u32 }
pub fn open(retry_count: u8) -> bool { let _ = retry_count; true }
EOF
git -C "$sr" add -A && git -C "$sr" -c user.name=t -c user.email=t@t commit -qm init 2>/dev/null
printf '%s\n' '/// Uses `jump_host_id`, `retry_count`, `true` and `self`.' \
    '/// Mentions `ghost_function` twice: `ghost_function`.' >"$sr/demo/src/new.rs"
cp "$sr/demo/src/new.rs" "$sr/target/src/new.rs"
dcs() { # dcs <label> <file> <expected-finding-count>
    local n
    run_hook "$g" "$(jq -cn --arg f "$sr/$2" '{file_path: $f}')"
    n=$(grep -c '^doc-claims:' "$work/report" 2>/dev/null || true)
    if [ "${n:-0}" = "$3" ]; then
        report ok "$g: $1"
    else
        report FAIL "$g: $1" "expected $3 finding(s), got ${n:-0}"
    fi
}
rm -f "$work/report"
dcs 'fields, params and literals count as found; only the ghost is reported' demo/src/new.rs 1
dcs 'a second save of the same file does not duplicate the finding' demo/src/new.rs 1
rm -f "$work/report"
dcs 'a claim on an already committed line is not re-reported' demo/src/lib.rs 0
dcs 'target/ is never scanned' target/src/new.rs 0
rm -f "$work/report"

# ── session-report flush ─────────────────────────────────────────────────────
# The flush output lands in the user's prompt: it must be de-duplicated and
# capped, with the overflow kept in <report>.full rather than dropped.
g=session-report.sh
{
    for i in $(seq 1 30); do printf 'dup line\n'; done
    for i in $(seq 1 120); do printf 'finding %s\n' "$i"; done
} >"$work/report"
out=$(KIRO_SESSION_REPORT="$work/report" "$bin/$g" flush 2>/dev/null)
lines=$(printf '%s\n' "$out" | wc -l | tr -d ' ')
dups=$(printf '%s\n' "$out" | grep -cx 'dup line' || true)
if [ "$lines" -le 41 ] && [ "$dups" = 1 ] && [ ! -e "$work/report" ] &&
    grep -qx 'finding 120' "$work/report.full" 2>/dev/null; then
    report ok "$g: flush de-duplicates, caps at 40 lines and keeps the rest in .full"
else
    report FAIL "$g: flush cap" "printed $lines lines, 'dup line' x$dups, report gone=$([ -e "$work/report" ] && echo no || echo yes)"
fi
printf 'one\n' >"$work/report"
rm -f "$work/report.full"
out=$(KIRO_SESSION_REPORT="$work/report" "$bin/$g" flush 2>/dev/null)
if [ "$out" = one ] && [ ! -e "$work/report.full" ]; then
    report ok "$g: a short report is printed as-is, with no .full"
else
    report FAIL "$g: short flush" "got '$out'"
fi

# ── matchers in .kiro/hooks/*.json ───────────────────────────────────────────
# Written anchored, so substring and full-match semantics agree.
expect_match() { # expect_match <hook> yes|no <tool>...
    local hook=$1 want=$2 m t got
    shift 2
    m=$(jq -r '.hooks[0].matcher // ""' ".kiro/hooks/$hook.json")
    for t in "$@"; do
        if printf '%s' "$t" | grep -qE -- "$m"; then got=yes; else got=no; fi
        if [ "$got" = "$want" ]; then
            report ok "$hook.json matcher: $t -> $want"
        else
            report FAIL "$hook.json matcher: $t" "want $want"
        fi
    done
}
kg_writes=(mcp_kirograph_kirograph_str_replace mcp_kirograph_kirograph_multi_str_replace
    mcp_kirograph_kirograph_insert_at mcp_kirograph_kirograph_ast_grep_rewrite
    @kirograph/kirograph_str_replace @kirograph/kirograph_insert_at)
for h in bash-serialization-guard release-manual-only-guard; do
    expect_match "$h" yes execute_bash control_bash_process mcp_kirograph_kirograph_exec @kirograph/kirograph_exec
    expect_match "$h" no read_code fs_write mcp_kirograph_kirograph_read
done
expect_match commit-review-gate yes execute_bash mcp_kirograph_kirograph_exec @kirograph/kirograph_exec
expect_match commit-review-gate no read_code fs_write
expect_match changelog-entry-guard yes execute_bash mcp_kirograph_kirograph_exec @kirograph/kirograph_exec
expect_match changelog-entry-guard no read_code fs_write
expect_match doc-claims-scan yes foo.rs models/protocol.rs
expect_match doc-claims-scan no foo.md foo.json foo.toml
expect_match edit-journal yes fs_write fs_append str_replace delete_file smart_relocate semantic_rename "${kg_writes[@]}"
expect_match edit-journal no read_code read_file execute_bash mcp_kirograph_kirograph_read
expect_match crate-boundary-guard yes fs_write fs_append str_replace "${kg_writes[@]}"
expect_match crate-boundary-guard no read_code execute_bash

# ── hook file shape ──────────────────────────────────────────────────────────
# `timeout` belongs on the hook entry, beside `action` (kiro.dev/docs/hooks.md).
# Inside `action` it is ignored and the 60 s default applies instead.
for f in .kiro/hooks/*.json; do
    if jq -e '[.hooks[] | select(.action.timeout != null)] | length == 0' "$f" >/dev/null 2>&1; then
        report ok "$(basename "$f"): timeout is not inside action"
    else
        report FAIL "$(basename "$f")" 'timeout sits inside action, where the engine ignores it'
    fi
done

# Every script a hook execs must be executable, in the tree and in git. A
# 100644 kirograph-sync.sh failed with EACCES on every Stop for a week and looked,
# from the outside, exactly like a hook that ran and found nothing.
for f in .kiro/hooks/*.json; do
    while IFS= read -r s; do
        [ -n "$s" ] || continue
        mode=$(git ls-files -s -- "$s" 2>/dev/null | cut -d' ' -f1)
        if [ -x "$s" ] && { [ -z "$mode" ] || [ "$mode" = 100755 ]; }; then
            report ok "$(basename "$f"): $s is executable"
        else
            report FAIL "$(basename "$f"): $s" "not executable (tree -x: $([ -x "$s" ] && echo yes || echo no), git mode: ${mode:-untracked})"
        fi
    done < <(jq -r '.hooks[].action.command // empty' "$f" | grep -oE '\.kiro/hooks/bin/[A-Za-z0-9_.-]+\.sh' | sort -u)
done

printf 'test-hooks: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
