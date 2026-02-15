---
name: "rustconn"
displayName: "RustConn"
description: "Development and release workflow for RustConn — GTK4/Rust connection manager with strict clippy, property tests, crate boundaries, and automated packaging"
keywords: ["rustconn", "rust", "clippy", "fmt", "cargo", "release", "version", "changelog", "packaging", "gtk4", "property test", "proptest"]
author: "Anton Isaiev"
---

# RustConn Development Power

Linux connection manager для SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes, Zero Trust.
GTK4/libadwaita GUI, Wayland-first. Rust 2021 edition, MSRV 1.88, three-crate Cargo workspace.

## Available Steering Files

- **release.md** — Повний процес релізу: оновлення версій у всіх packaging файлах, конвертація changelog, чеклист

## Development Flow

1. Створити нову гілку від main
2. Підняти версію та створити запис в CHANGELOG.md (використай steering `release.md`)
3. Поступово реалізувати фічі
4. Після кожної фічі — автоматичні перевірки (fmt, clippy, test) через сабагент
5. Ручне тестування GUI
6. Merge в main
7. `git tag -a vX.Y.Z -m "Release X.Y.Z" && git push origin main --tags` — тригерить CI

## Automated Checks

Після завершення реалізації фічі, делегуй перевірки в `general-task-execution` сабагент:

```
Виконай послідовно і поверни результат (pass/fail + помилки якщо є):
1. cargo fmt --check
2. cargo clippy --all-targets (має бути 0 warnings)
3. cargo test --workspace (timeout 90s, тести йдуть ~50s)
Якщо fmt або clippy мають помилки — виправ автоматично і перезапусти.
```

Це економить контекст основного діалогу від тисяч рядків виводу cargo.

## Quick Reference

| Task | Command |
|------|---------|
| Check compilation | `cargo check --all-targets` |
| Clippy | `cargo clippy --all-targets` |
| Clippy + fix | `cargo clippy --all-targets --fix --allow-dirty` |
| Format | `cargo fmt` |
| Format check | `cargo fmt --check` |
| All tests | `cargo test --workspace` |
| Property tests | `cargo test -p rustconn-core --test property_tests` |
| Build release | `cargo build --release` |
| Run GUI | `cargo run -p rustconn` |
| Run CLI | `cargo run -p rustconn-cli` |

## Crate Boundaries

**Головне правило: "Чи потрібен GTK?"**

| Відповідь | Крейт | Обмеження |
|-----------|-------|-----------|
| Ні | `rustconn-core` | GUI-free — ЗАБОРОНЕНО `gtk4`, `vte4`, `adw` |
| Так | `rustconn` | Може імпортувати GTK |
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

## Strict Rules

| ✅ REQUIRED | ❌ FORBIDDEN |
|-------------|--------------|
| `Result<T, Error>` для fallible функцій | `unwrap()`/`expect()` (крім provably impossible) |
| `thiserror` для всіх error types | Error types без `#[derive(thiserror::Error)]` |
| `SecretString` для credentials | Plain `String` для паролів/ключів |
| `tokio` для async | Змішування async runtimes |
| GUI-free `rustconn-core` | `gtk4`/`vte4`/`adw` в `rustconn-core` |
| `adw::` widgets | Deprecated GTK patterns |
| Line width 100 chars, 4 spaces, LF | Tabs, CRLF, long lines |
| `unsafe_code = "forbid"` | Будь-який unsafe код |

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

## Testing

### Property Tests

Локація: `rustconn-core/tests/properties/`

⏱️ Повний тестовий набір ~50 секунд. Завжди чекай завершення (timeout 90s).

Новий property test модуль:
1. Створи файл в `rustconn-core/tests/properties/`
2. Зареєструй в `rustconn-core/tests/properties/mod.rs`

Temp files — завжди `tempfile` crate.

## UI Patterns (rustconn/)

| Pattern | Implementation |
|---------|----------------|
| Widgets | `adw::` over `gtk::` equivalents |
| Toasts | `adw::ToastOverlay` |
| Dialogs | `adw::Dialog` або `gtk::Window` + `set_modal(true)` |
| Spacing | 12px margins, 6px між related elements (GNOME HIG) |
| Wayland | Уникати X11-specific APIs |

## State Management

```rust
pub type SharedAppState = Rc<RefCell<AppState>>;
```

- Pass `&SharedAppState` для mutable access
- Manager structs own data і handle I/O
- Async: `with_runtime()` для thread-local tokio runtime
- Ніколи не тримати borrow через async boundary або GTK callbacks

## Clippy Troubleshooting

| Lint | Рішення |
|------|---------|
| `cognitive_complexity` | Розбий на менші функції |
| `too_many_arguments` | Створи struct для параметрів |
| `missing_errors_doc` | Додай `# Errors` секцію |
| Clippy не бачить змін | `cargo clean && cargo clippy --all-targets` |
