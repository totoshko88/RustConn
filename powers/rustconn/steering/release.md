# RustConn Release Workflow

Автоматизує оновлення версії, залежностей, CLI та changelog у всіх packaging файлах.

## Коли використовувати

1. На початку нової гілки — підняти версію та створити порожній запис в CHANGELOG
2. Під час розробки — оновити залежності та CLI версії
3. Після завершення всіх фіч — розповсюдити changelog у всі packaging файли

## Етап 1: Початок нової версії

1. Прочитай поточну версію з `Cargo.toml` → `[workspace.package] version`
2. Підніми версію (patch/minor/major за вказівкою)
3. Оновити версію у всіх файлах (див. "Файли для оновлення версії")
4. Створити запис в CHANGELOG.md під `## [X.Y.Z] - YYYY-MM-DD`
5. Commit: `chore: bump version to X.Y.Z`

## Етап 2: Оновлення залежностей (кожен реліз)

### Cargo залежності

```bash
cargo update --dry-run          # Подивитись що оновиться
cargo update                    # Застосувати оновлення
cargo build                     # Перевірити компіляцію
cargo clippy --all-targets      # 0 warnings
cargo fmt --check               # Форматування
cargo test -p rustconn-core --test property_tests  # Property tests (timeout 180s)
```

Записати оновлені пакети в CHANGELOG.md секцію `### Dependencies`:
```markdown
### Dependencies
- **Updated**: crate1 X.Y.Z→X.Y.W, crate2 A.B.C→A.B.D
```

Включати тільки значущі оновлення (не кожен transitive dep). Групувати wasm-bindgen/js-sys/web-sys в один рядок.

### CLI версії (Flatpak downloads)

Файл: `rustconn-core/src/cli_download.rs`

Перевірити актуальність pinned versions для кожного компонента:

| Component | Де перевірити |
|-----------|---------------|
| TigerVNC | https://sourceforge.net/projects/tigervnc/files/stable/ |
| Teleport | https://goteleport.com/docs/changelog або GitHub releases |
| Tailscale | https://github.com/tailscale/tailscale/releases |
| Boundary | https://github.com/hashicorp/boundary/releases |
| Bitwarden CLI | https://github.com/bitwarden/clients/releases (cli-v* tags) |
| 1Password CLI | https://releases.1password.com/developers/cli/ |
| kubectl | https://kubernetes.io/releases/ |

При оновленні pinned version:
1. Оновити `pinned_version` в `DownloadableComponent`
2. Оновити `download_url` та `aarch64_url` (версія в URL)
3. Оновити `checksum` якщо `ChecksumPolicy::Static` — завантажити `.sha256` файл
4. Записати в CHANGELOG.md секцію `### Changed`:
   ```markdown
   ### Changed
   - **CLI downloads** — Tailscale 1.94.1→1.94.2, kubectl 1.35.0→1.35.1
   ```
5. `cargo build && cargo clippy --all-targets` — перевірити компіляцію

Компоненти з `SkipLatest` checksum та без `pinned_version` (AWS CLI, gcloud, cloudflared тощо) — не потребують оновлення URL.

## Етап 3: Фіналізація релізу

1. Переконатись що CHANGELOG.md містить повний опис змін, включаючи:
   - `### Added` — нові фічі
   - `### Fixed` — виправлення
   - `### Improved` — покращення
   - `### Changed` — зміни CLI versions тощо
   - `### Dependencies` — оновлення Cargo залежностей
   - `### Security` — якщо є security-related зміни
2. Розповсюдити changelog у всі packaging файли (див. нижче)
3. Синхронізувати версію у всіх packaging файлах
4. Виконати фінальні перевірки:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets    # 0 warnings
   cargo test --workspace        # ~120s
   cargo build --release         # Release build
   ```
5. Merge в main
6. `git tag -a vX.Y.Z -m "Release X.Y.Z" && git push origin main --tags`

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

### Fixed
- **Bug Name** — Description ([#N](url))

### Improved
- **Area** — Description

### Changed
- **CLI downloads** — Component X.Y.Z→X.Y.W

### Dependencies
- **Updated**: crate1 X.Y.Z→X.Y.W, crate2 A.B.C→A.B.D
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

## Синхронізація версій — чеклист

Перед тегуванням релізу перевірити що версія `X.Y.Z` присутня у ВСІХ файлах:

```bash
# Швидка перевірка (має знайти версію у всіх файлах):
grep -r "X.Y.Z" Cargo.toml debian/changelog packaging/ rustconn/assets/*.xml docs/USER_GUIDE.md docs/ARCHITECTURE.md
```

| Файл | Що перевірити |
|------|---------------|
| `Cargo.toml` | `version = "X.Y.Z"` |
| `debian/changelog` | `rustconn (X.Y.Z-1)` |
| `packaging/obs/debian.changelog` | `rustconn (X.Y.Z-1)` |
| `packaging/obs/rustconn.dsc` | `Version: X.Y.Z-1` |
| `packaging/obs/debian.dsc` | `Version: X.Y.Z-1` |
| `packaging/obs/rustconn.spec` | `Version: X.Y.Z` |
| `packaging/obs/rustconn.changes` | `- X.Y.Z` |
| `packaging/obs/AppImageBuilder.yml` | `version: X.Y.Z` |
| `packaging/flatpak/*.yml` | `tag: vX.Y.Z` |
| `packaging/flathub/*.yml` | `tag: vX.Y.Z` |
| `metainfo.xml` | `<release version="X.Y.Z"` |
| `docs/USER_GUIDE.md` | `Version X.Y.Z` |
| `docs/ARCHITECTURE.md` | `Version X.Y.Z` |

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

### CHANGELOG.md → RPM spec / .changes
```
### Added
- **Feature** — Description (#N)
```
→
```
- Added Feature (#N)
```

### CHANGELOG.md → Metainfo XML
- Прибрати markdown bold (`**...**`)
- Прибрати backticks
- Прибрати markdown посилання — залишити тільки текст
- Тримати кожен `<li>` коротким (1 рядок)
- Escape XML entities: `&` → `&amp;`

## Фінальний чеклист

```bash
# 1. Код
cargo fmt --check
cargo clippy --all-targets       # 0 warnings
cargo test --workspace           # ~120s (argon2 property tests повільні в debug mode)
cargo build --release

# 2. Версії синхронізовані (всі packaging файли)
# 3. CHANGELOG.md містить Dependencies та Changed (CLI) секції
# 4. Packaging changelogs оновлені
# 5. metainfo.xml має новий <release>
# 6. CLI versions в cli_download.rs актуальні
# 7. Cargo.lock оновлений (cargo update)
```
