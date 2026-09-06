#!/usr/bin/env bash
# Deterministic inventory of a change set, for handing to a fresh session.
#
#     ./scripts/change-inventory.sh [<base>]      # default base: HEAD
#
# Writes target/change-inventory.md and prints its path.
#
# Why this exists. The review workflow here opens a *new* session on an expensive
# model to verify a change, sometimes twice before a release. A fresh session knows
# nothing, so its first act is discovery: list files, guess which crates matter,
# work out which tests are relevant. That is the most expensive way to learn facts
# a shell command already knows — the model pays per token to rediscover a diff.
#
# So compute the facts for nothing and let the expensive model spend its tokens on
# judgement instead. It opens one file and starts reviewing.
#
# Everything here is read-only: `git diff`, `git status`, path arithmetic. No
# cargo, no clippy, no network. It is safe to run while a build is going, and it
# finishes in well under a second on this repo.
#
# The clippy log is referenced, never produced: running clippy from here would
# contend for the target-dir lock, and the log is usually already there from the
# quality gate that preceded the commit.

set -uo pipefail

base=${1:-HEAD}

repo=$(git rev-parse --show-toplevel 2>/dev/null) || {
    printf 'change-inventory: not a git checkout\n' >&2
    exit 1
}
cd "$repo" || exit 1

git rev-parse --verify --quiet "$base" >/dev/null || {
    printf 'change-inventory: unknown base revision: %s\n' "$base" >&2
    exit 1
}

out="target/change-inventory.md"
mkdir -p target || exit 1

# Tracked modifications plus untracked additions — the same definition the rest of
# the tooling uses, so the inventory cannot disagree with the Stop report.
changed=$(
    {
        git diff --name-only "$base" 2>/dev/null
        git ls-files --others --exclude-standard 2>/dev/null
    } | sort -u | sed '/^$/d'
)

# Crates are top-level directories carrying a Cargo.toml. Derived rather than
# hardcoded: a seventh crate should appear here without anyone remembering to
# update this script.
crates_touched=""
while IFS= read -r f; do
    [ -n "$f" ] || continue
    top=${f%%/*}
    [ "$top" = "$f" ] && continue
    [ -f "$top/Cargo.toml" ] || continue
    case " $crates_touched " in
    *" $top "*) ;;
    *) crates_touched="$crates_touched $top" ;;
    esac
done <<<"$changed"
crates_touched=${crates_touched# }

# Test files that live in the touched crates. A dependency-graph walk would be
# better and KiroGraph can do it (`kirograph_affected`); this stays a plain
# filesystem answer so the script needs nothing but git.
tests=""
for c in $crates_touched; do
    [ -d "$c/tests" ] || continue
    found=$(find "$c/tests" -name '*.rs' -type f 2>/dev/null | sort || true)
    [ -n "$found" ] && tests="$tests$found"$'\n'
done
tests=$(printf '%s' "$tests" | sed '/^$/d')

{
    printf '# Change inventory\n\n'
    printf -- '- base: `%s` (`%s`)\n' "$base" "$(git rev-parse --short "$base" 2>/dev/null || echo '?')"
    printf -- '- branch: `%s`\n' "$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')"
    printf -- '- generated: %s\n\n' "$(date -Is 2>/dev/null || date)"

    if [ -z "$changed" ]; then
        printf 'No changes against this base. Nothing to review.\n'
    else
        printf '## Files\n\n```\n'
        git diff --stat "$base" 2>/dev/null || true
        untracked=$(git ls-files --others --exclude-standard 2>/dev/null | sed '/^$/d')
        if [ -n "$untracked" ]; then
            printf '\nuntracked (not in the diffstat above):\n'
            printf '%s\n' "$untracked" | sed 's/^/  /'
        fi
        printf '```\n\n'

        printf '## Crates touched\n\n'
        if [ -n "$crates_touched" ]; then
            for c in $crates_touched; do printf -- '- `%s`\n' "$c"; done
        else
            printf 'None — the change is outside the crate trees (docs, packaging, .kiro).\n'
        fi
        printf '\n'

        printf '## Tests in those crates\n\n'
        if [ -n "$tests" ]; then
            printf '%s\n' "$tests" | sed 's/^/- `/; s/$/`/'
            printf '\nFor the dependency-aware set rather than the co-located one:\n'
            printf '`kirograph_affected` with the file list above.\n'
        else
            printf 'No `tests/` directory in the touched crates.\n'
        fi
        printf '\n'

        printf '## Review scope\n\n'
        # Name the specialised reviews the change requires, so the fresh session
        # does not have to work out which invariants are in play.
        needed=""
        printf '%s\n' "$changed" | grep -qE '^rustconn-(pty|locale|env|dock)-sys/.*\.rs$' &&
            needed="$needed- \`unsafe-reviewer\` — a rustconn-*-sys crate changed\n"
        printf '%s\n' "$changed" | grep -qE '^(rustconn-core/src/secret/.*\.rs|.*credential[^/]*\.rs|.*credentials[^/]*\.rs|.*password[^/]*\.rs)$' &&
            needed="$needed- \`security-reviewer\` — credential-handling code changed\n"
        printf '%s\n' "$changed" | grep -qxF 'po/uk.po' &&
            needed="$needed- \`uk-translation-reviewer\` — po/uk.po changed\n"
        printf '%s\n' "$changed" | grep -q '^CHANGELOG\.md$' ||
            needed="$needed- No CHANGELOG.md entry in this range — confirm the change is not user-facing\n"
        if [ -n "$needed" ]; then
            printf '%b' "$needed"
        else
            printf 'No specialised review triggered by the paths in this change.\n'
        fi
        printf '\n'

        printf '## Quality gate\n\n'
        if [ -f target/clippy.log ]; then
            printf 'Existing log: `target/clippy.log` (%s). Confirm it is newer than the change before trusting it.\n' \
                "$(date -r target/clippy.log -Is 2>/dev/null || echo 'unknown time')"
        else
            printf 'No `target/clippy.log`. Run the gate through the `rust-quality-check` sub-agent\n'
            printf 'rather than from this session — see steering `shell-environment.md`.\n'
        fi
        printf '\nRemember a cached clippy prints `Finished ... in 0.2s` and reports zero warnings\n'
        printf 'without checking anything.\n'
    fi
} >"$out.tmp" || exit 1

mv -f "$out.tmp" "$out" || exit 1

printf '%s\n' "$out"
