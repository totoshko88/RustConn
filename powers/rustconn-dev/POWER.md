---
name: "rustconn-dev"
displayName: "RustConn Development"
description: "GTK4/Rust connection manager development with strict clippy, property tests, and crate boundaries"
keywords: ["rustconn", "rust", "clippy", "fmt", "cargo", "gtk4", "libadwaita", "adw", "ssh", "rdp", "vnc", "spice", "connection manager", "terminal", "vte", "wayland", "property test", "proptest", "thiserror", "secrecy", "gnome", "linux"]
author: "RustConn Team"
---

# RustConn Development Power

Linux connection manager для SSH, RDP, VNC, SPICE. GTK4/libadwaita GUI, Wayland-first.

## Quick Reference

| Task | Command |
|------|---------|
| Check compilation | `cargo check --all-targets` |
| Clippy | `cargo clippy --all-targets` |
| Clippy + fix | `cargo clippy --all-targets --fix --allow-dirty` |
| Format check | `cargo fmt --check` |
| Format | `cargo fmt` |
| All tests | `cargo test` |
| Property tests | `cargo test -p rustconn-core --test property_tests` |
| Build release | `cargo build --release` |
| Run GUI | `cargo run -p rustconn` |
| Run CLI | `cargo run -p rustconn-cli` |

## Onboarding

При першій активації перевір середовище:

```bash
# Rust toolchain (MSRV 1.88)
rustc --version

# Clippy
cargo clippy --version

# Перевірка що все компілюється
cargo check --all-targets
```

---

## Crate Boundaries

**Головне правило: "Чи потрібен GTK?"**

| Відповідь | Крейт | Обмеження |
|-----------|-------|-----------|
| **Ні** | `rustconn-core` | GUI-free — ЗАБОРОНЕНО `gtk4`, `vte4`, `adw` |
| **Так** | `rustconn` | Може імпортувати GTK |
| CLI | `rustconn-cli` | Тільки `rustconn-core` |

### Куди додавати код

| Тип фічі | Локація | Дія |
|----------|---------|-----|
| Data model | `rustconn-core/src/models/` | Re-export в `models.rs` |
| Protocol | `rustconn-core/src/protocol/` | Implement `Protocol` trait |
| Import format | `rustconn-core/src/import/` | Implement `ImportSource` trait |
| Export format | `rustconn-core/src/export/` | Implement `ExportTarget` trait |
| Secret backend | `rustconn-core/src/secret/` | Implement `SecretBackend` trait |
| Dialog | `rustconn/src/dialogs/` | Register в `dialogs/mod.rs` |
| Property test | `rustconn-core/tests/properties/` | Register в `properties/mod.rs` |

---

## Code Patterns

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("description: {0}")]
    Variant(String),
}
```

### Credentials (ОБОВ'ЯЗКОВО SecretString)

```rust
use secrecy::SecretString;
let password: SecretString = SecretString::new(value.into());
```

### Identifiers

```rust
let id = uuid::Uuid::new_v4();
```

### Timestamps

```rust
let now: chrono::DateTime<chrono::Utc> = chrono::Utc::now();
```

### Async Traits

```rust
#[async_trait::async_trait]
impl MyTrait for MyStruct {
    async fn method(&self) -> Result<(), Error> { /* ... */ }
}
```

---

## Strict Rules

| ✅ REQUIRED | ❌ FORBIDDEN |
|-------------|--------------|
| `Result<T, Error>` для fallible функцій | `unwrap()`/`expect()` (крім provably impossible) |
| `thiserror` для всіх error types | Error types без `#[derive(thiserror::Error)]` |
| `SecretString` для credentials | Plain `String` для паролів/ключів |
| `tokio` для async | Змішування async runtimes |
| GUI-free `rustconn-core` | `gtk4`/`vte4`/`adw` в `rustconn-core` |
| `adw::` widgets | Deprecated GTK patterns |

---

## Testing

### Property Tests

Локація: `rustconn-core/tests/properties/`

```bash
# Всі property тести
cargo test -p rustconn-core --test property_tests

# Конкретний тест
cargo test -p rustconn-core --test property_tests test_name
```

**⏱️ Час виконання:** Повний тестовий набір може виконуватись до 1 хвилини. Після внесення змін у код перший запуск тестів потребує перекомпіляції — зачекай завершення перед наступною командою.

**Новий property test модуль:**
1. Створи файл в `rustconn-core/tests/properties/`
2. Зареєструй в `rustconn-core/tests/properties/mod.rs`

### Temp Files

Завжди використовуй `tempfile` crate:

```rust
use tempfile::TempDir;
let temp_dir = TempDir::new()?;
```

---

## Pre-Commit Workflow

```bash
# 1. Форматування
cargo fmt

# 2. Clippy з виправленнями
cargo clippy --all-targets --fix --allow-dirty

# 3. Перевірка компіляції
cargo build --all-targets

# 4. Тести
cargo test
```

---

## Clippy Troubleshooting

### `cognitive_complexity`
Функція занадто складна → розбий на менші функції.

### `too_many_arguments`
Більше 7 аргументів → створи struct для параметрів.

### `missing_errors_doc`
Додай `# Errors` секцію в документацію для `Result` функцій.

### Clippy не бачить змін
```bash
cargo clean
cargo clippy --all-targets
```

---

## Release Checklist

### Обов'язкові перевірки

```bash
cargo fmt
cargo clippy --all-targets  # 0 warnings
cargo build --release
cargo test
cargo test -p rustconn-core --test property_tests
```

### Файли для оновлення версії

| Файл | Що оновити |
|------|------------|
| `Cargo.toml` | `version` в `[workspace.package]` |
| `CHANGELOG.md` | Нова секція `## [X.Y.Z] - YYYY-MM-DD` |
| `docs/USER_GUIDE.md` | `**Version X.Y.Z**` |
| `snap/snapcraft.yaml` | `version: 'X.Y.Z'` |
| `debian/changelog` | Нова секція |
| `packaging/obs/debian.changelog` | Нова секція |
| `packaging/obs/rustconn.changes` | Нова секція |
| `packaging/obs/rustconn.spec` | `Version:` та `%changelog` |
| `packaging/obs/AppImageBuilder.yml` | `version:` |
| `packaging/flatpak/*.yml` | `tag: vX.Y.Z` |
| `packaging/flathub/*.yml` | `tag: vX.Y.Z` |

### Процес релізу

1. Оновити версії у всіх файлах
2. `cargo fmt && cargo clippy --all-targets --fix --allow-dirty`
3. `cargo build --all-targets && cargo test`
4. Commit + merge в main
5. `git tag -a vX.Y.Z -m "Release X.Y.Z" && git push origin main --tags`

---

## UI Patterns (rustconn/)

| Pattern | Implementation |
|---------|----------------|
| Widgets | `adw::` over `gtk::` equivalents |
| Toasts | `adw::ToastOverlay` |
| Dialogs | `adw::Dialog` або `gtk::Window` + `set_modal(true)` |
| Layout | Sidebar `gtk::ListView` + `gtk::Notebook` tabs |
| Spacing | 12px margins, 6px між related elements (GNOME HIG) |

---

## State Management

```rust
pub type SharedAppState = Rc<RefCell<AppState>>;
```

- Pass `&SharedAppState` для mutable access
- Manager structs own data і handle I/O
- Async: `with_runtime()` для thread-local tokio runtime
