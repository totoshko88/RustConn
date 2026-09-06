# po/ — translations

- **`ls po/*.po` is the authoritative locale count.** Do not trust a number written
  in prose anywhere in this repo, including in the root `AGENTS.md`. That number has
  already been wrong once (16 vs 17, missing `ka`).
- After adding or changing an `i18n()` / `i18n_f()` string in Rust:
  `bash po/update-pot.sh`, then `msgmerge --update` the affected catalogues.
- A source file containing `i18n()` must be listed in `POTFILES.in` or
  `update-pot.sh` silently drops its strings. The `translation-sync` hook adds new
  GUI files on save; `./scripts/check-potfiles.sh` is the CI gate for when it did
  not fire.

## The three gates

Run all three before claiming a translation change is done:

```bash
./scripts/check-potfiles.sh      # POTFILES.in matches reality
./scripts/check-i18n-escapes.sh  # no \u{...} in translatable literals
./scripts/check-po-complete.sh   # no fuzzy or missing entries
```

## Conventions

- Placeholders are `{}`, positional and in source order. A translation may not
  reorder them and may not drop one.
- No `\u{...}` escapes in a translatable literal — write the character. The escape
  survives extraction and reaches the user verbatim.
- `uk.po` has a dedicated reviewer: the `commit-review-gate` hook asks for the
  `uk-translation-reviewer` sub-agent at commit time when the change touched
  `po/uk.po`. It enforces DSTU terminology, imperative mood for UI actions and
  Kharkiv orthography. Do not hand-tune Ukrainian strings against your own instinct —
  let the reviewer run and act on what it says. Until 2026-09-06 this fired on every
  *save* of the file, which meant a full review after every `msgmerge` that rewrote
  the catalogue without changing a single translation.
- `rustconn.pot` carries `Project-Id-Version: rustconn X.Y.Z` and is one of the 19
  files `scripts/release.sh` checks for the release version. It is generated —
  regenerate it, do not hand-edit the version line.
