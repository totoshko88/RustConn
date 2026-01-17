---
name: "rustconn-dev"
displayName: "RustConn Development"
description: "Development workflow for RustConn project with automated clippy, fmt, and build checks"
keywords: ["rustconn", "rust", "clippy", "fmt", "cargo"]
author: "RustConn Team"
---

# RustConn Development

## Overview

Автоматизований workflow для розробки RustConn — перевірка коду через clippy, форматування, тести та збірка.

## Конфігурація проєкту

### Rust версія
- **MSRV:** 1.87
- **Edition:** 2021

### Clippy налаштування
- Рівень: `pedantic` + `nursery` (суворий)
- `unsafe_code = "forbid"` — unsafe заборонено
- Cognitive complexity: 25
- Max arguments: 7

### Форматування
- Max width: 100 символів
- Tabs: 4 пробіли
- Line endings: Unix (LF)
- Auto reorder imports

## Команди

### Швидка перевірка
```bash
# Перевірка компіляції (швидко)
cargo check --all-targets

# Clippy з усіма warnings
cargo clippy --all-targets

# Clippy з автовиправленням
cargo clippy --all-targets --fix --allow-dirty
```

### Форматування
```bash
# Перевірка форматування
cargo fmt --check

# Автоформатування
cargo fmt
```

### Тести
```bash
# Всі тести
cargo test

# Property тести
cargo test -p rustconn-core --test property_tests

# Конкретний тест
cargo test -p rustconn-core test_name
```

### Збірка
```bash
# Debug
cargo build

# Release
cargo build --release
```

## Workflow перед комітом

Виконай послідовно:

```bash
# 1. Форматування
cargo fmt

# 2. Clippy з виправленнями
cargo clippy --all-targets --fix --allow-dirty

# 3. Перевірка що все компілюється
cargo build --all-targets

# 4. Тести
cargo test
```

## Структура крейтів

| Крейт | Призначення | GUI залежності |
|-------|-------------|----------------|
| `rustconn-core` | Бізнес-логіка | ❌ Заборонено |
| `rustconn` | GTK4 GUI | ✅ gtk4, vte4, adw |
| `rustconn-cli` | CLI | ❌ Тільки core |

**Правило:** Якщо код не потребує GTK → `rustconn-core`

## Типові помилки Clippy

### `cognitive_complexity`
Функція занадто складна. Розбий на менші функції.

### `too_many_arguments`
Більше 7 аргументів. Створи struct для параметрів.

### `missing_errors_doc`
Додай `# Errors` секцію в документацію для функцій що повертають `Result`.

## Патерни коду

### Помилки
```rust
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("опис: {0}")]
    Variant(String),
}
```

### Credentials
```rust
use secrecy::SecretString;
let password: SecretString = SecretString::new(value.into());
```

### Async traits
```rust
#[async_trait::async_trait]
impl MyTrait for MyStruct {
    async fn method(&self) -> Result<(), Error> { /* ... */ }
}
```

## Troubleshooting

### Clippy не бачить змін
```bash
cargo clean
cargo clippy --all-targets
```

### Конфлікт форматування
```bash
cargo fmt
git diff  # перевір зміни
```

### Тести падають після clippy --fix
Clippy іноді робить некоректні виправлення. Перевір `git diff` і відкоти якщо потрібно.

---

## Release Checklist

### Перед релізом — обов'язкові перевірки

```bash
# 1. Форматування
cargo fmt

# 2. Clippy (має бути 0 warnings)
cargo clippy --all-targets

# 3. Збірка
cargo build --release

# 4. Всі тести
cargo test

# 5. Property тести окремо (можуть бути повільними)
cargo test -p rustconn-core --test property_tests
```

### Файли для оновлення версії

При піднятті версії (наприклад `0.6.3` → `0.6.4`) оновити:

| Файл | Що оновити |
|------|------------|
| `Cargo.toml` | `version = "X.Y.Z"` в `[workspace.package]` |
| `CHANGELOG.md` | Додати нову секцію `## [X.Y.Z] - YYYY-MM-DD` |
| `docs/USER_GUIDE.md` | `**Version X.Y.Z**` в заголовку |
| `snap/snapcraft.yaml` | `version: 'X.Y.Z'` |
| `debian/changelog` | Нова секція зверху |
| `packaging/obs/debian.changelog` | Нова секція зверху |
| `packaging/obs/rustconn.changes` | Нова секція зверху |
| `packaging/obs/rustconn.spec` | `Version:` та `%changelog` |
| `packaging/obs/AppImageBuilder.yml` | `version:` в `AppDir.app_info` |
| `packaging/flatpak/io.github.totoshko88.RustConn.yml` | `tag: vX.Y.Z` |
| `packaging/flathub/io.github.totoshko88.RustConn.yml` | `tag: vX.Y.Z` |

### Формат changelog записів

**CHANGELOG.md:**
```markdown
## [X.Y.Z] - YYYY-MM-DD

### Added
- Нова функція

### Changed
- Зміна поведінки

### Fixed
- Виправлення багу
```

**debian/changelog:**
```
rustconn (X.Y.Z-1) unstable; urgency=medium

  * Опис змін

 -- Anton Isaiev <totoshko88@gmail.com>  Day, DD Mon YYYY HH:MM:SS +0200
```

**RPM spec %changelog:**
```
* Day Mon DD YYYY Anton Isaiev <totoshko88@gmail.com> - X.Y.Z-0
- Опис змін
```

### Процес релізу

1. **Оновити версії** у всіх файлах (див. таблицю вище)
2. **Закомітити** зміни версій
3. **Запустити pre-commit workflow:**
   ```bash
   cargo fmt
   cargo clippy --all-targets --fix --allow-dirty
   cargo build --all-targets
   cargo test
   ```
4. **Закомітити** виправлення (якщо є)
5. **Merge в main** (якщо на feature branch)
6. **Створити тег:**
   ```bash
   git tag -a vX.Y.Z -m "Release X.Y.Z"
   git push origin main --tags
   ```
7. **Перевірити CI** — GitHub Actions мають зібрати всі пакети

### Flathub/Flatpak — додаткові кроки

Після створення тегу потрібно оновити `cargo-sources.json`:

```bash
# Встановити flatpak-cargo-generator (якщо немає)
pip install aiohttp toml

# Згенерувати нові cargo sources
python3 flatpak-cargo-generator.py Cargo.lock -o packaging/flathub/cargo-sources.json
```

### Snap Store — публікація

Snap автоматично публікується при пуші тегу `v*` якщо налаштований `SNAP_STORE_TOKEN` секрет.

Для ручної публікації:
```bash
snapcraft login
snapcraft upload rustconn_X.Y.Z_amd64.snap --release=stable
```
