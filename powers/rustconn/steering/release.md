# RustConn Release Workflow

Автоматизує оновлення версії та changelog у всіх packaging файлах.

## Коли використовувати

1. На початку нової гілки — підняти версію та створити порожній запис в CHANGELOG
2. Після завершення всіх фіч — розповсюдити changelog у всі packaging файли

## Процес початку нової версії

1. Прочитай поточну версію з `Cargo.toml` → `[workspace.package] version`
2. Підніми версію (patch/minor/major за вказівкою)
3. Оновити версію у всіх файлах (див. список нижче)
4. Створити запис в CHANGELOG.md під `## [X.Y.Z] - YYYY-MM-DD`
5. Commit: `chore: bump version to X.Y.Z`

## Процес фіналізації релізу

1. Переконатись що CHANGELOG.md містить повний опис змін
2. Розповсюдити changelog у всі packaging файли
3. Виконати перевірки: `cargo fmt && cargo clippy --all-targets && cargo test`
4. Merge в main
5. `git tag -a vX.Y.Z -m "Release X.Y.Z" && git push origin main --tags`

## Файли для оновлення версії

### 1. `Cargo.toml` (workspace root)
```toml
[workspace.package]
version = "X.Y.Z"
```

### 2. `CHANGELOG.md`
Джерело правди. Формат:
```markdown
## [Unreleased]

## [X.Y.Z] - YYYY-MM-DD

### Added
- **Feature Name** — Description ([#N](url))
```

### 3. `docs/USER_GUIDE.md`
Перший рядок після заголовка:
```
**Version X.Y.Z** | GTK4/libadwaita Connection Manager for Linux
```

### 4. `docs/ARCHITECTURE.md`
Перший рядок після заголовка:
```
**Version X.Y.Z** | Last updated: Month YYYY
```

### 5. `debian/changelog`
Нова секція ЗВЕРХУ файлу:
```
rustconn (X.Y.Z-1) unstable; urgency=medium

  * Version bump to X.Y.Z
  * [changelog entries]

 -- Anton Isaiev <totoshko88@gmail.com>  DAY, DD MON YYYY HH:MM:SS +0200
```
DAY — скорочений день тижня (Mon, Tue, Wed, Thu, Fri, Sat, Sun).

### 6. `packaging/obs/debian.changelog`
Той самий формат що й `debian/changelog`.

### 7. `packaging/obs/rustconn.changes`
```
-------------------------------------------------------------------
DAY MON DD YYYY Anton Isaiev <totoshko88@gmail.com> - X.Y.Z

- Version bump to X.Y.Z
  * [changelog entries]

```
Дата: `Sun Feb 15 2026` (без ком).

### 8. `packaging/obs/rustconn.spec`
Оновити `Version:` в шапці + `Summary:` якщо змінились протоколи.
Додати секцію в `%changelog` ЗВЕРХУ:
```
* DAY MON DD YYYY Anton Isaiev <totoshko88@gmail.com> - X.Y.Z-0
- Version bump to X.Y.Z
- [changelog entries]
```

### 9. `packaging/obs/rustconn.dsc`
```
Version: X.Y.Z-1
Files:
 00000000000000000000000000000000 0 rustconn_X.Y.Z.orig.tar.xz
 00000000000000000000000000000000 0 rustconn_X.Y.Z-1.debian.tar.xz
```

### 10. `packaging/obs/debian.dsc`
```
Version: X.Y.Z-1
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
НЕ змінювати — використовує локальний path.

### 14. `packaging/flathub/io.github.totoshko88.RustConn.yml`
```yaml
        tag: vX.Y.Z
```

### 15. `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml`
Додати `<release>` ЗВЕРХУ `<releases>`:
```xml
    <release version="X.Y.Z" date="YYYY-MM-DD">
      <description>
        <p>Version X.Y.Z - [короткий опис]:</p>
        <ul>
          <li>[пункт]</li>
        </ul>
      </description>
    </release>
```
Також оновити `<description>` якщо змінились протоколи/фічі.

### 16. `snap/snapcraft.yaml` (якщо існує)
```yaml
version: 'X.Y.Z'
```

## Конвертація Changelog

### CHANGELOG.md → Debian
```
### Added
- **Feature** — Description (#N)
```
→
```
  * Added Feature (#N):
    - Description
```

### CHANGELOG.md → Metainfo XML
- Прибрати markdown bold (`**...**`)
- Прибрати backticks
- Прибрати markdown посилання
- Тримати кожен `<li>` коротким (1 рядок)

## Чеклист після оновлення

```bash
cargo check
cargo fmt --check
cargo clippy --all-targets  # 0 warnings
cargo test --workspace      # ~50s
```
