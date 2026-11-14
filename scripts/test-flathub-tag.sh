#!/usr/bin/env bash
# Regression suite for scripts/flathub-tag.sh.
#
# Every case runs against a TEMPORARY COPY of the Flathub manifest. The real
# packaging/flathub/ tree is never written (packaging/AGENTS.md, "Flathub is
# maintainer-only"); the last case checks that it is byte-for-byte unchanged.
#
# Covers: bad version, missing and duplicate tag: lines, Cargo.toml mismatch
# with and without --force, --dry-run writing nothing, idempotency, the commit:
# line handling, and that the line the script writes satisfies the pattern
# release.sh's packaging gate greps for (read out of release.sh, not copied).
#
# Usage: scripts/test-flathub-tag.sh [-v]     -v also prints the passing cases
# Needs bash, git, awk, sed. No network.
set -uo pipefail
repo=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo 'test-flathub-tag: not inside a git checkout' >&2
    exit 1
}
cd "$repo" || exit 1
verbose=0
[ "${1:-}" = "-v" ] && verbose=1

script=scripts/flathub-tag.sh
real=packaging/flathub/io.github.totoshko88.RustConn.yml
work=$(mktemp -d "${TMPDIR:-/tmp}/rustconn-test-flathub-tag.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
real_sum_before=$(cksum <"$real")

pass=0
fail=0
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
# run <args...>  -> sets $rc and $out (stdout+stderr)
run() {
    out=$("$script" "$@" 2>&1)
    rc=$?
}
expect_rc() { # expect_rc <code> <label> <args...>
    local want=$1 label=$2
    shift 2
    run "$@"
    if [ "$rc" = "$want" ]; then
        report ok "$label"
    else
        report FAIL "$label" "expected exit $want, got $rc: $out"
    fi
}
check() { # check <label> <detail-on-failure> <command...>
    local label=$1 detail=$2
    shift 2
    if "$@"; then
        report ok "$label"
    else
        report FAIL "$label" "$detail"
    fi
}
fresh() { # fresh <name> -> path of a new copy of the real manifest
    cp "$real" "$work/$1.yml"
    printf '%s' "$work/$1.yml"
}

cur=$(awk -F'"' '/^\[workspace\.package\]/{p=1} p&&/^version[[:space:]]*=/{print $2; exit}' Cargo.toml)
[ -n "$cur" ] || {
    echo 'test-flathub-tag: cannot read the Cargo.toml version' >&2
    exit 1
}
other=999.0.0
[ "$cur" != "$other" ] || other=998.0.0

# The gate pattern, read from release.sh the way the script and bump-version.sh
# read it — then expanded for the version under test.
gate_pat() { # gate_pat <version>
    local re=${1//./\\.} p
    p=$(awk '/^PKG_FILES=\($/ {f=1; next} f && /^\)$/ {exit} f' scripts/release.sh |
        grep -n "\"$real\"" | cut -d: -f1)
    [ -n "$p" ] || return 1
    p=$(awk '/^PKG_PATS=\($/ {f=1; next} f && /^\)$/ {exit} f' scripts/release.sh |
        sed -n "${p}p" | sed -E 's/^[[:space:]]*"(.*)"[[:space:]]*$/\1/')
    p=${p//\$VERSION_RE/$re}
    printf '%s' "${p//\\\\/\\}"
}

# ── Arguments ────────────────────────────────────────────────────────────────
m=$(fresh args)
expect_rc 2 'no version is refused' --manifest "$m"
expect_rc 2 'v-prefixed version is refused' "v$cur" --manifest "$m"
expect_rc 2 'two-part version is refused' 1.2 --manifest "$m"
expect_rc 2 'suffixed version is refused' "$cur-rc1" --manifest "$m"
expect_rc 2 'unknown option is refused' "$cur" --bogus --manifest "$m"
expect_rc 1 'missing manifest is refused' "$cur" --manifest "$work/nope.yml"
check 'refused runs leave the copy untouched' 'copy changed' cmp -s "$real" "$m"

# ── Cargo.toml mismatch and --force ──────────────────────────────────────────
m=$(fresh force)
expect_rc 1 'version != Cargo.toml is refused without --force' "$other" --manifest "$m"
check 'refused mismatch writes nothing' 'copy changed' cmp -s "$real" "$m"
expect_rc 0 '--force accepts a version != Cargo.toml' "$other" --force --manifest "$m"
check '--force wrote the tag line' "$(grep -n 'tag:' "$m")" \
    grep -qE "^[[:space:]]+tag: v${other//./\\.}\$" "$m"

# ── Dry run ──────────────────────────────────────────────────────────────────
m=$(fresh dry)
expect_rc 0 '--dry-run succeeds' "$other" --force --dry-run --manifest "$m"
check '--dry-run writes nothing' 'copy changed' cmp -s "$real" "$m"
case "$out" in
*"+        tag: v$other"*) report ok '--dry-run prints the unified diff' ;;
*) report FAIL '--dry-run prints the unified diff' "$out" ;;
esac

# ── Write, gate pattern, idempotency ─────────────────────────────────────────
m=$(fresh write)
sed -E 's/^([[:space:]]+)tag: v[0-9.]+$/\1tag: v0.0.1/' "$m" >"$work/w" && cat "$work/w" >"$m"
expect_rc 0 'writes the current version' "$cur" --manifest "$m"
case "$out" in
*'Next: ./scripts/release.sh --dry-run'*) report ok 'prints the next command' ;;
*) report FAIL 'prints the next command' "$out" ;;
esac
pat=$(gate_pat "$cur") || pat=""
if [ -n "$pat" ] && grep -qE -- "$pat" "$m"; then
    report ok "release.sh gate pattern ($pat) matches the written line"
else
    report FAIL 'release.sh gate pattern matches the written line' "pattern '$pat'"
fi
pat_old=$(gate_pat 0.0.1)
if grep -qE -- "$pat_old" "$m"; then
    report FAIL 'old version no longer satisfies the gate' "still matches $pat_old"
else
    report ok 'old version no longer satisfies the gate'
fi
diff -u "$real" "$m" | grep -E '^[-+][^-+]' >"$work/delta"
if [ "$(wc -l <"$work/delta")" -le 2 ] && ! grep -vqE '^[-+][[:space:]]+tag: v' "$work/delta"; then
    report ok 'only the tag line differs from the original'
else
    report FAIL 'only the tag line differs from the original' "$(cat "$work/delta")"
fi
cp "$m" "$work/after-first"
expect_rc 0 'second run succeeds' "$cur" --manifest "$m"
check 'second run changes nothing (idempotent)' 'copy changed' cmp -s "$work/after-first" "$m"
case "$out" in
*'nothing to change'*) report ok 'second run says nothing to change' ;;
*) report FAIL 'second run says nothing to change' "$out" ;;
esac

# ── Missing / duplicate / malformed tag line ─────────────────────────────────
m=$(fresh none)
grep -vE '^[[:space:]]+tag: v' "$m" >"$work/n" && cat "$work/n" >"$m"
expect_rc 1 'missing tag: line is refused' "$cur" --manifest "$m"

m=$(fresh dup)
awk '{print} /^[[:space:]]+tag: v/ {print}' "$m" >"$work/d" && cat "$work/d" >"$m"
cp "$m" "$work/dup-before"
expect_rc 1 'duplicate tag: line is refused' "$cur" --manifest "$m"
check 'duplicate refusal writes nothing' 'copy changed' cmp -s "$work/dup-before" "$m"

m=$(fresh trailing)
sed -E 's/^([[:space:]]+tag: v[0-9.]+)$/\1  # pinned/' "$m" >"$work/t" && cat "$work/t" >"$m"
expect_rc 1 'tag: line with a trailing comment is refused' "$cur" --manifest "$m"

# ── commit: line in the app source ───────────────────────────────────────────
# No local tag for $other, so the hash must be left exactly as it was.
m=$(fresh commit)
sha=0123456789abcdef0123456789abcdef01234567
awk -v sha="$sha" '{print} /^[[:space:]]+tag: v/ {match($0, /^[[:space:]]+/); printf "%scommit: %s\n", substr($0, 1, RLENGTH), sha}' \
    "$m" >"$work/c" && cat "$work/c" >"$m"
expect_rc 0 'commit: pin without a local tag still writes the tag' "$other" --force --manifest "$m"
check 'commit: pin left unchanged without a local tag' "$(grep -n 'commit:' "$m")" \
    grep -qE "^[[:space:]]+commit: $sha\$" "$m"
case "$out" in
*'left unchanged'*) report ok 'says the commit: pin was left unchanged' ;;
*) report FAIL 'says the commit: pin was left unchanged' "$out" ;;
esac

# With a local tag the hash comes from git. Use a tag that exists already, so
# the suite creates none; skip when the checkout has no v* tag at all.
have=$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname | head -n1)
if [ -n "$have" ]; then
    want=$(git rev-parse "refs/tags/$have^{commit}")
    m=$(fresh commit-tag)
    awk -v sha="$sha" '{print} /^[[:space:]]+tag: v/ {match($0, /^[[:space:]]+/); printf "%scommit: %s\n", substr($0, 1, RLENGTH), sha}' \
        "$m" >"$work/c" && cat "$work/c" >"$m"
    expect_rc 0 "commit: pin resolved from local tag $have" "${have#v}" --force --manifest "$m"
    check 'commit: set to the tag commit' "$(grep -n 'commit:' "$m")" \
        grep -qE "^[[:space:]]+commit: $want\$" "$m"
else
    printf '  skip  commit: from local tag (no v* tag in this checkout)\n'
fi

# ── The real manifest was never touched ──────────────────────────────────────
check "$real unchanged" 'the real Flathub manifest was modified' \
    test "$(cksum <"$real")" = "$real_sum_before"

printf 'test-flathub-tag: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
