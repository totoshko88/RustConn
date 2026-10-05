#!/usr/bin/env bash
# PostFileSave check for Rust sources: flag a doc-comment claim, on a line THIS
# change adds, that names a `snake_case` identifier the code does not contain.
#
# Why this exists. The 0.22.12 audit found three doc claims that named things the
# code did not have: the `search` module advised using `search_parallel` (no such
# function), a `PropertyType::Url` claim about clickable rendering (not
# implemented), and a module header promising "all downloads are verified using
# SHA256" for a path that skipped the check. The compiler and clippy never catch
# this — a doc comment is not type-checked against the code it describes.
#
# Why it is this narrow (rewritten 2026-10-05, measured). The first version
# scanned the WHOLE file on every save and cleared a name only if it was a
# fn/struct/enum/... definition. On the 126 .rs files of release 0.23 that is 610
# findings (137 KB) for one pass — `true` x34, `false` x24, then struct fields,
# parameters, extern fns — and the hook fired 308 times in one turn, so the flush
# pasted ~1.5k near-identical lines into the next prompt. Now:
#   - only doc lines added relative to HEAD (`git diff -U0`); an untracked file
#     counts as all-added. A claim that was already committed has been reviewed;
#   - only files under some crate's src/ — never target/, tests, benches;
#   - a name is "found" if it appears as a whole word on any non-comment line of
#     any crate's src/ — fields, params, methods and locals all count, so what is
#     left is a name the code never spells at all;
#   - Rust keywords and literals (`true`, `self`, ...) are ignored;
#   - a finding already in the report is not appended again, and at most
#     $max_per_file findings are written per save.
# Same 0.23 set after the rewrite: 4 findings. Still a NOTE, never a block.
#
# Delivery goes through the shared report (KIRO_SESSION_REPORT, default
# target/.kiro-session-report), NOT stdout: PostFileSave discards a command
# hook's stdout. session-report.sh flush prints it on the next UserPromptSubmit.
#
# Fails OPEN: no jq, no git, unreadable file -> allow, say nothing.

set -uo pipefail

trap 'exit 0' ERR

max_per_file=20

payload=$(cat) || exit 0
command -v jq >/dev/null 2>&1 || exit 0

file=$(printf '%s' "$payload" | jq -r '.file_path // ""' 2>/dev/null) || exit 0
[ -n "$file" ] || exit 0

# Anchor at the git root the way translation-sync.sh / edit-journal.sh do.
abs=$file
case "$abs" in
/*) ;;
*) abs=$PWD/$abs ;;
esac
anchor=$(dirname "$abs")
while [ "$anchor" != "/" ] && [ ! -d "$anchor" ]; do anchor=$(dirname "$anchor"); done
[ -d "$anchor" ] || anchor=$PWD
repo=$(git -C "$anchor" rev-parse --show-toplevel 2>/dev/null || true)
[ -n "$repo" ] || exit 0
cd "$repo" 2>/dev/null || exit 0
rel=${abs#"$repo"/}

case "$rel" in
target/* | */target/*) exit 0 ;;
*/src/*.rs | src/*.rs) ;;
*) exit 0 ;;
esac
[ -f "$rel" ] || exit 0

# Doc lines this change adds.
if git ls-files --error-unmatch -- "$rel" >/dev/null 2>&1; then
    added=$(git diff -U0 HEAD -- "$rel" 2>/dev/null | grep -E '^\+[^+]' | cut -c2- || true)
else
    added=$(cat -- "$rel" 2>/dev/null || true)
fi
idents=$(printf '%s\n' "$added" | grep -E '^[[:space:]]*//[/!]' |
    grep -oE '`[a-z][a-z0-9_]{3,}`' | tr -d '`' | sort -u |
    grep -vxE 'true|false|self|none|some|null|async|await|move|impl|loop|match|static|const|unsafe|where|break|continue|return|crate|super|type|enum|struct|trait|while' ||
    true)
[ -n "$idents" ] || exit 0

# Every crate's src/ is a search root: a name used anywhere in the workspace is
# not a lie about the workspace.
roots=()
for d in */src src; do [ -d "$d" ] && roots+=("$d"); done
[ "${#roots[@]}" -gt 0 ] || exit 0

# One pass over the code, not one per identifier: the words from $idents that
# occur on some non-comment line. Counted into a variable rather than tested
# with `grep -q`, which under pipefail can SIGPIPE its producer and read as
# "not found".
found=$(grep -rh --include='*.rs' -vE '^[[:space:]]*//' "${roots[@]}" 2>/dev/null |
    grep -owF -f <(printf '%s\n' "$idents") 2>/dev/null | sort -u || true)

missing=$(comm -23 <(printf '%s\n' "$idents") <(printf '%s\n' "$found") 2>/dev/null || true)
[ -n "$missing" ] || exit 0

report=${KIRO_SESSION_REPORT:-target/.kiro-session-report}
mkdir -p "$(dirname "$report")" 2>/dev/null || exit 0

n=0
while IFS= read -r ident; do
    [ -n "$ident" ] || continue
    [ "$n" -lt "$max_per_file" ] || break
    line="doc-claims: ${rel} doc comment adds \`${ident}\`, which no code line in */src spells. Verify the doc still matches the code."
    grep -qxF -- "$line" "$report" 2>/dev/null && continue
    printf '%s\n' "$line" >>"$report" 2>/dev/null || true
    n=$((n + 1))
done <<<"$missing"

exit 0
