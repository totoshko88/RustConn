---
inclusion: fileMatch
fileMatchPattern: "CHANGELOG.md"
---

# CHANGELOG.md — Format Rules

Based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) + project conventions.

## Structure

```markdown
# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [X.Y.Z] - YYYY-MM-DD

### Added
### Fixed
### Changed
### Removed
### Improved
### Documentation
### Dependencies
```

## Section order (within a version)

1. **Added** — new features
2. **Fixed** — bug fixes (most common)
3. **Changed** — behavior changes / breaking
4. **Removed** — removed features
5. **Improved** — non-breaking improvements (refactors, performance, UX polish)
6. **Documentation** — docs-only changes
7. **Dependencies** — dep updates (list as `**name X.Y.Z → A.B.C**`)

Omit empty sections — only include sections that have entries.

## Entry format

- Start with `**Bold summary (issue #NNN)**` — a short headline, optionally with issue reference
- Follow with explanation in the same bullet (no sub-bullets unless truly needed)
- Write in past tense for Fixed ("fixed", "resolved"), present for Added ("adds", "allows")
- Be specific: name the function, crate, or UI element affected
- For security fixes: mention the attack vector and resolution

## Issue references

- Format: `(issue #123)` at the end of the bold summary
- Link format at bottom of file (optional): `[#123]: https://github.com/totoshko88/RustConn/issues/123`

## Dependencies section

- Group updates as a comma-separated list: `**Updated**: pkg1 X→Y, pkg2 X→Y`
- For security-critical deps, make a separate bullet with advisory details
- Include the Flatpak/OBS manifest dependency version bumps here too

## Rules

- One `## [X.Y.Z]` section per release — never duplicate version headers
- Date format: `YYYY-MM-DD` (ISO 8601)
- Unreleased work goes under `## [Unreleased]` at the top (added when needed, removed at release)
- Never leave placeholder text — every entry must be real content
- The `sync-package-versions` hook and `release-version` steering handle version propagation — CHANGELOG content is always manual
