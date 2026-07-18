---
inclusion: manual
description: "Генерує conventional commit повідомлення на основі поточного git diff. Формат: type(scope): description"
---

Help generate a conventional commit message for RustConn. Steps:

1. Run `git diff --cached --stat` to check staged changes. If nothing staged, run `git diff --stat` for unstaged changes and tell the developer to stage first.

2. Run `git diff --cached` (or `git diff` if nothing staged) to understand the actual changes.

3. Determine:
   - **type**: feat, fix, docs, style, refactor, test, chore, perf, ci, build
   - **scope**: rustconn-core, rustconn-cli, rustconn (gui), i18n, packaging, ci
   - **description**: imperative mood, lowercase, no period, max 50 chars

4. Generate the commit message in format: `type(scope): description`
   - If changes span multiple scopes, omit scope or use the dominant one
   - For body: add blank line + bullet points explaining WHY (not WHAT)

5. If there are multiple logical changes, suggest splitting into separate commits.

6. Present the message and ask: "Use this? (copy with: git commit -m '...')" — do NOT auto-commit.
