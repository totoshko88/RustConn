---
inclusion: always
---

# RustConn — Project Rules

Communication language: Ukrainian.

## Architecture (3 crates)

| Crate | Purpose | Restrictions |
|-------|---------|-------------|
| `rustconn-core` | Business logic, models, protocols | **FORBIDDEN**: gtk4, adw, vte4 |
| `rustconn-cli` | CLI interface | Only rustconn-core |
| `rustconn` | GTK4/libadwaita GUI | May import everything |

## Absolute Rules

- `unsafe_code = "forbid"` — no unsafe whatsoever
- Passwords/keys → `secrecy::SecretString`, never plain String
- Intermediate `expose_secret().to_string()` → wrap in `zeroize::Zeroizing::new()`
- Errors → `thiserror::Error`, never `unwrap()`/`expect()`
- Logging → `tracing`, never `println!`/`eprintln!`
- i18n → `i18n()` / `i18n_f()` with `{}` placeholders for all user-facing strings
- `display_name()` values used in UI → wrap in `i18n()` at call site
- After new i18n strings → `bash po/update-pot.sh` + `msgmerge --update` for 16 languages
- Rust 2024 edition: let-chains instead of collapsible_if
- Never `set_var`/`remove_var` (unsafe in Rust 2024)

## Quick Commands

```
cargo fmt --all                    # Format
cargo clippy --all-targets         # Lint (0 warnings)
cargo test --workspace             # Tests (~120s, argon2 is slow)
cargo test -p rustconn-core --test property_tests  # Property tests only
bash po/update-pot.sh              # Regenerate POT after new i18n strings
```

## Quality Checks

Delegate to `rust-quality-check` sub-agent for fmt+clippy+tests instead of running in main context.
For quick single-file validation → `getDiagnostics`.

### Self-Check Rules (no hooks — apply mentally)

When writing `.rs` files, verify BEFORE writing:
- **Crate boundary**: `rustconn-core/` and `rustconn-cli/` must NOT contain `use gtk4`, `use adw`, `use vte4`, `gtk4::`, `adw::`, `vte4::`. Move GUI code to `rustconn/`.
- **No unsafe**: never write `unsafe {`, `unsafe fn`, `unsafe impl`, `unsafe trait`.

After writing `.rs` files in `rustconn/src/`, verify:
- **i18n**: all user-facing strings (`.set_label()`, `.set_title()`, `.set_tooltip_text()`, `Button::with_label()`) wrapped in `i18n()` or `i18n_f()`. Ignore: tracing, CSS, icons, action names.
- **Credentials** (in secret/password/credential files): `SecretString` for passwords, `.zeroize()` intermediates, no secrets in logs/args/errors.
- **Protocol files**: business logic in rustconn-core, GTK in rustconn.

### When to Run fmt/clippy/tests

- **Do NOT** run `cargo fmt`/`cargo clippy` automatically on every change — use `getDiagnostics` for quick validation.
- Run `rust-quality-check` sub-agent only when: (a) about to commit, (b) user explicitly asks, (c) finishing a multi-file feature.
- Run tests only when: (a) user explicitly asks, (b) finishing a spec task, (c) before release.
- After completing work, inform the user: "Готово. Запустити перевірку якості (fmt+clippy)?" — wait for confirmation.

### Test Run Rules (CRITICAL)

- **NEVER** pipe `cargo test` through `tail`, `grep`, or any filter — run directly to see progress.
- **NEVER** start `cargo test` if another instance is already running (`pgrep -f 'cargo test'`).
- Tests take ~120s (argon2 property tests). This is normal — wait for completion, do NOT assume timeout.
- If a hook or sub-agent already ran tests in this turn, do NOT re-run them.
- Use timeout 180s for test commands.

## 16 Translation Languages

be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn
