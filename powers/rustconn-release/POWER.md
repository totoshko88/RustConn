---
name: "rustconn-release"
displayName: "RustConn Release"
description: "Automates version bumps and changelog propagation across all RustConn packaging files based on CHANGELOG.md content"
keywords: ["rustconn", "release", "version", "changelog", "packaging", "deb", "rpm", "flatpak", "snap", "appimage", "obs", "metainfo"]
author: "RustConn Team"
---

# RustConn Release Power

Автоматизує оновлення версії та changelog у всіх packaging файлах на основі вмісту `CHANGELOG.md`.

## Коли використовувати

Після того як `CHANGELOG.md` вже містить нову версію з повним описом змін, і потрібно:
1. Оновити версію у всіх packaging файлах
2. Розповсюдити changelog у форматі кожного пакету
3. Оновити metainfo XML для Flatpak/Flathub

## Процес

### Крок 1: Зчитати версію та changelog

Прочитай `CHANGELOG.md` і знайди останню версію (перший `## [X.Y.Z] - YYYY-MM-DD` після `## [Unreleased]`).

Витягни:
- `VERSION` — номер версії (наприклад `0.7.7`)
- `DATE` — дата релізу (наприклад `2026-02-08`)
- `CHANGELOG_ENTRIES` — всі пункти під цією версією (секції ### Fixed, ### Added, ### Improved, ### Refactored, ### Changed тощо)

### Крок 2: Оновити версію у всіх файлах

Оновити версію у наступних файлах. Для кожного файлу вказано точний формат.

---

## Файли для оновлення

### 1. `Cargo.toml` (workspace)

```toml
[workspace.package]
version = "X.Y.Z"
```

### 2. `CHANGELOG.md`

Вже оновлений — це джерело правди. Не змінювати.

### 3. `docs/USER_GUIDE.md`

Перший рядок після заголовка:
```
**Version X.Y.Z** | GTK4/libadwaita Connection Manager for Linux
```

### 4. `snap/snapcraft.yaml`

```yaml
version: 'X.Y.Z'
```

### 5. `debian/changelog`

Формат Debian changelog. Нова секція ЗВЕРХУ файлу:
```
rustconn (X.Y.Z-1) unstable; urgency=medium

  * Version bump to X.Y.Z
  * [changelog entries — кожен пункт як окремий рядок з * або -]

 -- Anton Isaiev <totoshko88@gmail.com>  DAY, DD MON YYYY HH:MM:SS +0200
```

Де `DAY` — скорочений день тижня (Mon, Tue, Wed, Thu, Fri, Sat, Sun).
Дата береться з `DATE` у changelog.

### 6. `packaging/obs/debian.changelog`

Той самий формат що й `debian/changelog`. Нова секція ЗВЕРХУ файлу.

### 7. `packaging/obs/rustconn.changes`

Формат OBS changes. Нова секція ЗВЕРХУ файлу:
```
-------------------------------------------------------------------
DAY MON DD YYYY Anton Isaiev <totoshko88@gmail.com> - X.Y.Z

- Version bump to X.Y.Z
  * [changelog entries]

```

Де дата у форматі: `Fri Feb 07 2026` (без ком).

### 8. `packaging/obs/rustconn.spec`

Оновити `Version:` у шапці:
```
Version:        X.Y.Z
```

Додати нову секцію у `%changelog` ЗВЕРХУ:
```
* DAY MON DD YYYY Anton Isaiev <totoshko88@gmail.com> - X.Y.Z-0
- Version bump to X.Y.Z
  * [changelog entries]
```

Де дата у форматі: `Fri Feb 07 2026`.

### 9. `packaging/obs/rustconn.dsc`

Оновити `Version:` та імена tar файлів:
```
Version: X.Y.Z-1
```
```
Files:
 00000000000000000000000000000000 0 rustconn_X.Y.Z.orig.tar.xz
 00000000000000000000000000000000 0 rustconn_X.Y.Z-1.debian.tar.xz
```

### 10. `packaging/obs/debian.dsc`

Оновити `Version:` та `DEBTRANSFORM-TAR:`:
```
Version: X.Y.Z-1
```
```
DEBTRANSFORM-TAR: rustconn-X.Y.Z.tar.xz
```

### 11. `packaging/obs/AppImageBuilder.yml`

```yaml
    version: X.Y.Z
```

### 12. `packaging/flatpak/io.github.totoshko88.RustConn.yml`

```yaml
        tag: vX.Y.Z
```

### 13. `packaging/flatpak/io.github.totoshko88.RustConn.local.yml`

Цей файл НЕ має git tag — він використовує локальний path. Не змінювати версію.

### 14. `packaging/flathub/io.github.totoshko88.RustConn.yml`

```yaml
        tag: vX.Y.Z
```

### 15. `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml`

Додати новий `<release>` елемент ЗВЕРХУ списку `<releases>`:
```xml
    <release version="X.Y.Z" date="YYYY-MM-DD">
      <description>
        <p>Version X.Y.Z - [короткий опис]:</p>
        <ul>
          <li>[пункт 1]</li>
          <li>[пункт 2]</li>
          ...
        </ul>
      </description>
    </release>
```

Опис має бути стислим — без markdown форматування, без посилань, без зірочок.
Кожен значущий пункт з changelog як окремий `<li>`.

---

## Конвертація Changelog

### З CHANGELOG.md у Debian формат

Markdown:
```
### Fixed
- **Keyboard Shortcuts** — `Delete`, `Ctrl+E` no longer intercept input (#4)

### Improved
- **Thread Safety** — Audio mutex locks use graceful fallback
```

Debian:
```
  * Fixed keyboard shortcuts intercepting VTE terminal input:
    - Delete, Ctrl+E no longer fire when terminal has focus (#4)
  * Improved thread safety:
    - Audio mutex locks use graceful fallback instead of unwrap()
```

### З CHANGELOG.md у Metainfo XML

Markdown:
```
### Fixed
- **Keyboard Shortcuts** — `Delete`, `Ctrl+E` no longer intercept input (#4)
```

XML:
```xml
<li>Fixed Delete, Ctrl+E intercepting VTE terminal and embedded viewer input (#4)</li>
```

Правила:
- Прибрати markdown bold (`**...**`)
- Прибрати backticks
- Прибрати markdown посилання, залишити тільки текст
- Об'єднати пов'язані пункти де можливо
- Тримати кожен `<li>` коротким (1 рядок)

---

## Чеклист після оновлення

Після оновлення всіх файлів виконати:

```bash
# 1. Перевірити що Cargo.toml версія правильна
cargo check

# 2. Форматування
cargo fmt --check

# 3. Clippy
cargo clippy --all-targets

# 4. Тести
cargo test
```

---

## Приклад повного оновлення

Для версії `0.7.7` з датою `2026-02-08` (Sunday):

| Файл | Зміна |
|------|-------|
| `Cargo.toml` | `version = "0.7.7"` |
| `docs/USER_GUIDE.md` | `**Version 0.7.7**` |
| `snap/snapcraft.yaml` | `version: '0.7.7'` |
| `debian/changelog` | Нова секція `rustconn (0.7.7-1)` |
| `packaging/obs/debian.changelog` | Нова секція `rustconn (0.7.7-1)` |
| `packaging/obs/rustconn.changes` | Нова секція з датою `Sun Feb 08 2026` |
| `packaging/obs/rustconn.spec` | `Version: 0.7.7` + `%changelog` |
| `packaging/obs/rustconn.dsc` | `Version: 0.7.7-1` + tar filenames |
| `packaging/obs/debian.dsc` | `Version: 0.7.7-1` + `DEBTRANSFORM-TAR` |
| `packaging/obs/AppImageBuilder.yml` | `version: 0.7.7` |
| `packaging/flatpak/*.yml` | `tag: v0.7.7` |
| `packaging/flathub/*.yml` | `tag: v0.7.7` |
| `metainfo.xml` | Новий `<release version="0.7.7">` |
