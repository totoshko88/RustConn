# rustconn-core

Domain logic: models, config, CRUD managers, import/export, protocol data,
credential abstractions. Root `AGENTS.md` still applies; this is what is specific
to this tree.

- No GUI crates (root rule). If a change seems to need a widget, it belongs in
  `rustconn/`; what belongs here is the data it operates on.
- Default features stay **headless**. Embedded clients, RD Gateway/GFX, host
  keyring support and CLI download support are all behind features, and
  `cargo test -p rustconn-core` with no features must keep passing.
- Errors are `thiserror::Error` enums, never `anyhow`. Callers in the GUI and CLI
  pattern-match the variants, so a stringly-typed error here breaks them
  (M-ERRORS-CANONICAL-STRUCTS).
- Public functions returning `Result` need a `/// # Errors` section. Public
  functions that can panic on a programming bug need `/// # Panics`.
- `display_name()` returns the untranslated form. Wrapping it in `i18n()` is the
  *call site's* job, in `rustconn/` — do not translate here, there is no GUI
  locale context in a headless crate.
- New types: prefer a concrete noun over `…Manager`. `ConnectionManager`,
  `SessionManager` and `SecretManager` keep their names for compatibility; new
  code gets `ConnectionStore`, `CredentialResolver`, `SnippetCatalog`
  (M-CONCISE-NAMES).
- A type holding a `SecretString` needs a test proving its `Debug` does not leak.
  `secrecy` redacts itself, wrappers around it are not automatically safe
  (M-PUBLIC-DEBUG).

Property tests: `cargo test -p rustconn-core --test property_tests`.
