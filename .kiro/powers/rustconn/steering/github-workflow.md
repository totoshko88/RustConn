# GitHub Workflow — RustConn

Working with GitHub via the MCP server.

## PR Workflow

### Creating a PR

```
Use GitHub MCP: create_pull_request
- base: main
- head: <current branch>
- title: short description (up to 70 characters)
- body: structured description (see template below)
```

### PR Template

```markdown
## Summary

[1-2 sentences on what changed and why]

## Changes

- [Specific change 1]
- [Specific change 2]

## Testing

- [ ] `cargo fmt --check` — pass
- [ ] `cargo clippy --all-targets` — 0 warnings
- [ ] `cargo test --workspace` — pass
- [ ] Manual GUI testing (if UI changes)

## Notes

[What was not included, known limitations, blocked features]
```

### Review Checklist (self-review before merge)

1. Are all new user-facing strings wrapped in `i18n()`?
2. Do passwords use `SecretString`?
3. No `unwrap()`/`expect()` in new code?
4. Is CHANGELOG.md updated?
5. Do property tests cover the new logic?

## Issue Workflow

### Creating a bug issue

```
Use GitHub MCP: create_issue
- title: [Component] Short description
- body: Steps to reproduce, expected vs actual, logs
- labels: bug
```

### Creating a feature issue

```
Use GitHub MCP: create_issue
- title: [Component] Feature description
- body: Use case, proposed solution, alternatives considered
- labels: enhancement
```

## Release Workflow (with GitHub)

1. Ensure all PRs are merged into main
2. Complete the release checklist (see steering `release.md`)
3. Create tag: `git tag -a vX.Y.Z -m "Release X.Y.Z"`
4. Push: `git push origin main --tags`
5. GitHub Actions will create the release automatically

### Checking CI status

```
Use GitHub MCP: list_commits (to check recent commits)
Or: get_pull_request (to check CI checks on a PR)
```

## Useful GitHub MCP commands

| Task | Tool |
|------|------|
| List open issues | `list_issues` with state=open |
| Search code in repo | `search_code` |
| Check PR status | `get_pull_request` |
| List recent commits | `list_commits` |
| Get file from repo | `get_file_contents` |
