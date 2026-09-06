#!/usr/bin/env bash
# PreToolUse guard: an agent profile must declare which model it runs on.
#
# Why a missing field is worth blocking over. `kiro-cli chat --list-models`
# reports `"default_model":"auto"`, so a profile with no `model:` does not run on
# "whatever is cheapest" — it runs on the router, at 1.0x, chosen per request.
# That is the wrong default in both directions at once: `rust-quality-check`
# executes three fixed cargo commands and has clippy as its arbiter, so it should
# be on the 0.05x tier; `unsafe-reviewer` decides whether a SAFETY comment is
# verifiable and nothing re-checks it, so it should be on the 2.2x tier. Left
# unset, both sit at 1.0x and the profile looks deliberate.
#
# All five profiles were unset until 2026-09-06. The field is easy to forget
# precisely because nothing goes wrong visibly when it is missing.
#
# What this does NOT do: validate the ID against the catalogue. An unrecognised
# ID falls back to the default model with a warning, which is the same outcome as
# omitting the field — not worth a fail-closed check that would block every new
# model Kiro adds. Refresh the ID list with:
#
#     kiro-cli chat --list-models --format json
#
# and note that the catalogue differs per client: the IDE offers models the CLI
# does not, so a profile pinned to an IDE-only ID silently falls back in a CLI
# session. Prefer IDs present in both.
#
# Fails OPEN on anything unexpected, like the other guards here: the cost of a
# missed field is a 1.0x run, the cost of a stuck guard is a blocked session.

set -uo pipefail

trap 'exit 0' ERR

payload=$(cat) || exit 0
command -v jq >/dev/null 2>&1 || exit 0

path=$(printf '%s' "$payload" | jq -r '.tool_input.path // .tool_input.targetFile // ""' 2>/dev/null) || exit 0
[ -n "$path" ] || exit 0

# Only agent profiles, in either supported spelling.
case "$path" in
*/.kiro/agents/*.md | .kiro/agents/*.md | */.kiro/agents/*.json | .kiro/agents/*.json) ;;
*) exit 0 ;;
esac

# A deletion removes the profile; there is no model to declare.
[ "$(printf '%s' "$payload" | jq -r '.tool_input.targetFile // ""' 2>/dev/null)" = "$path" ] && exit 0

# `model:` in Markdown front matter, `"model"` in JSON.
declares_model() {
    printf '%s' "$1" | grep -qE '^[[:space:]]*(model:[[:space:]]*[^[:space:]]|"model"[[:space:]]*:[[:space:]]*"[^"]+")'
}

block() {
    printf 'agent-model-guard: %s\n' "$1" >&2
    cat >&2 <<'EOF'
  Every agent profile declares its own model. The default is `auto` (1.0x, chosen
  per request), which is the wrong tier for both ends of the range:

    predictable work with a machine arbiter   qwen3-coder-next  0.05x
    tool-driven navigation                    claude-haiku-4.5  0.4x
    judgement a human reads                   claude-sonnet-4.6 1.3x
    judgement nothing re-checks               claude-opus-4.5   2.2x

  Pick by whether anything verifies the agent's answer, not by how simple the
  task looks. Rationale and the full table: steering cost-discipline.md.
EOF
    exit 2
}

text=$(printf '%s' "$payload" | jq -r '.tool_input.text // ""' 2>/dev/null) || exit 0

if [ -n "$text" ]; then
    # Whole-file write: the text is the profile, so it must carry the field.
    declares_model "$text" && exit 0
    block 'new agent profile has no `model:` field.'
fi

# Partial edit (str_replace / fs_append). The fragment is not the profile, so the
# question is whether the edit *removes* a field the file already has.
[ -f "$path" ] || exit 0

old=$(printf '%s' "$payload" | jq -r '.tool_input.oldStr // ""' 2>/dev/null) || exit 0
new=$(printf '%s' "$payload" | jq -r '.tool_input.newStr // ""' 2>/dev/null) || exit 0

if [ -n "$old" ] && declares_model "$old" && ! declares_model "$new"; then
    block 'this edit removes the `model:` field from an agent profile.'
fi

exit 0
