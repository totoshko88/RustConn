# RustConn — Архітектурний аудит

**Дата:** 2026-02-21
**Версія:** 0.9.0
**Аудитор:** Principal Rust Software Architect
**Фокус:** GNOME HIG, Wayland-native, Adwaita, Flatpak, Security, UX, Code Quality

---

## Загальна оцінка

RustConn — зрілий, добре структурований GTK4/libadwaita додаток із чітким розділенням GUI/Core/CLI.
Архітектура трьох крейтів дотримана коректно, `unsafe_code = "forbid"` забезпечено глобально.
Проєкт має 1241+ property тестів, структуроване логування через `tracing`, і підтримку 14 мов.

**Сильні сторони:**
- Чіткий поділ GUI/Core/CLI без порушень кордонів крейтів
- Wayland-first підхід (wlfreerdp пріоритет, gdk4-wayland)
- Правильне використання `adw::ToastOverlay`, `adw::PreferencesDialog`, `adw::StatusPage`
- `SecretString` для більшості credential полів
- Graceful degradation для optional features (tray, audio, embedded clients)
- Atomic writes для конфігурації, debounced persistence

**Основні проблеми:**
- 25+ залишкових `eprintln!` замість `tracing` (порушення strict rule)
- 10+ місць де паролі зберігаються як plain `String` замість `SecretString`
- 11+ викликів `std::env::set_var` в multi-threaded контексті (unsafe з Rust 1.66)
- Діалоги використовують `adw::Window` замість `adw::Dialog` (libadwaita 1.5+)
- Embedded widgets без `Drop` — потенційний leak child processes
- Inconsistency між packaging форматами (locale, features, icons)

---

## СЕКЦІЯ 1: Безпека (Security)

### SEC-01 [HIGH] Plain String паролі замість SecretString ✅ ЧАСТКОВО

**Статус:** Частково виправлено у 0.9.0.

**Виправлено:**
- `FreeRdpConfig.password` → `SecretString`
- `RdpConfig.password` → `SecretString` (embedded_rdp/types.rs)
- `SpiceClientConfig.password` → `SecretString`

**Залишилось:**
- `rustconn-core/src/secret/kdbx.rs:23` — `KdbxEntry.password`
- `rustconn-core/src/secret/bitwarden.rs:81,100` — `BitwardenLogin.password`, `BitwardenLoginTemplate.password`
- `rustconn-core/src/secret/keepassxc.rs:45,67` — `KeePassXcMessage.password`, `KeePassXcEntry.password`
- `rustconn-core/src/secret/passbolt.rs:65` — `PassboltResource.password`
- `rustconn-core/src/import/rdm.rs:35` — `RdmConnection.password`
- `rustconn-core/src/variables/mod.rs:29` — `Variable.value` (з TODO коментарем)
- `rustconn/src/dialogs/mod.rs:65` — `ConnectionDialogResult.password`
- `rustconn/src/dialogs/password.rs:26` — `PasswordDialogResult.password`

**Як є:**
```rust
pub password: Option<String>,
```

**Як варто:**
```rust
use secrecy::SecretString;
pub password: Option<SecretString>,
```

Для serde десеріалізації (Bitwarden/KeePassXC JSON) використовувати `secrecy::serde` feature
або конвертувати одразу після десеріалізації.

---

### SEC-02 [HIGH] FreeRDP пароль через командний рядок ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. `thread.rs` тепер використовує `/from-stdin` + stdin pipe, аналогічно до `SafeFreeRdpLauncher`.

**Проблема:** `FreeRdpThread::launch_freerdp()` передає пароль через `/p:{password}` в аргументах процесу.
Пароль видимий через `/proc/PID/cmdline`. `SafeFreeRdpLauncher` в `launcher.rs` правильно використовує
`/from-stdin` + stdin pipe, але `thread.rs` — ні.

**Де:** `rustconn/src/embedded_rdp/thread.rs` — метод `launch_freerdp()`

**Як варто:** Уніфікувати з `SafeFreeRdpLauncher` — передавати пароль через stdin.

---

### SEC-03 [MEDIUM] std::env::set_var в multi-threaded контексті ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. `BW_SESSION` замінено на thread-safe `RwLock` storage в `bitwarden.rs`. `OP_SERVICE_ACCOUNT_TOKEN` `set_var` видалено (вже передавався через `Command::env()`). Решта `set_var` (i18n, sftp, SSH agent) задокументовані як safe — виконуються до запуску tokio runtime.

**Проблема:** `std::env::set_var()` не є thread-safe з Rust 1.66+. В Rust 2024 edition це буде hard error.
Знайдено 11+ викликів в коді, деякі відбуваються після запуску tokio runtime.

**Де:**
- `rustconn-core/src/secret/bitwarden.rs:928,992,1023` — `BW_SESSION`
- `rustconn-core/src/sftp.rs:211,213` — `SSH_AUTH_SOCK`, `SSH_AGENT_PID` (документовано як startup-only)
- `rustconn/src/i18n.rs:258,274,278` — `LANGUAGE`, `LC_MESSAGES`
- `rustconn/src/dialogs/settings/mod.rs:279` — `SSH_AUTH_SOCK`
- `rustconn/src/dialogs/settings/secrets_tab.rs:584,1995,2023` — `BW_SESSION`, `OP_SERVICE_ACCOUNT_TOKEN`

**Як варто:** Зберігати session keys в `AppState` або thread-safe контейнері (`Arc<RwLock<HashMap<String, String>>>`),
передавати через `Command::env()` при запуску процесів. Для `LANGUAGE`/`LC_MESSAGES` — викликати тільки при старті.

---

### SEC-04 [MEDIUM] KDBX функції приймають/повертають паролі як &str/String ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. Всі KDBX функції в `secret/status.rs` мігровані на `SecretString` + `SecretResult`. Всі callers оновлені.

**Де:** `rustconn-core/src/secret/status.rs` — всі `get_password_from_kdbx*`, `save_password_to_kdbx*` функції.

**Як є:**
```rust
pub fn get_password_from_kdbx(path: &Path, db_password: &str, entry: &str) -> Result<Option<String>, String>
```

**Як варто:**
```rust
pub fn get_password_from_kdbx(path: &Path, db_password: &SecretString, entry: &str) -> SecretResult<Option<SecretString>>
```

Також замінити `Result<_, String>` на `SecretError` (порушення правила thiserror).

---

### SEC-05 [LOW] Кастомний base64 в bitwarden.rs

**Де:** `rustconn-core/src/secret/bitwarden.rs:1049-1080` — hand-rolled `base64_encode()`.

**Як варто:** Використати `ring::test::from_hex` або додати `base64` крейт. Кастомна криптографічна
утиліта — ризик помилок і складніший аудит.

---

### SEC-06 [LOW] SSH custom_options без санітизації

**Де:** `rustconn-core/src/models/protocol.rs` — `SshConfig.build_command_args()` передає
`custom_options` HashMap як `-o key=value` без перевірки на небезпечні директиви (ProxyCommand тощо).

**Як варто:** Додати blocklist небезпечних SSH опцій (аналогічно до SSH config export).

---

## СЕКЦІЯ 2: GNOME HIG / Adwaita

### HIG-01 [MEDIUM] Діалоги використовують adw::Window замість adw::Dialog ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. Всі 8 діалогів мігровані на `adw::Dialog`: HistoryDialog, ProgressDialog, PasswordDialog, LogViewerDialog, FlatpakComponentsDialog, ImportDialog, ExportDialog, DocumentProtectionDialog. Escape key, modal behavior, adaptive sizing працюють автоматично.

**Проблема:** Всі кастомні діалоги (`PasswordDialog`, `HistoryDialog`, `ImportDialog`, `ExportDialog`,
`LogViewerDialog`, `FlatpakComponentsDialog`, `ProgressDialog`, `DocumentProtectionDialog`) використовують
`adw::Window` з ручними header bars. Libadwaita 1.5+ надає `adw::Dialog` який автоматично обробляє
modal behavior, close gestures, adaptive sizing.

**Як є:**
```rust
let window = adw::Window::builder()
    .modal(true)
    .transient_for(parent)
    .default_width(750)
    .default_height(500)
    .build();
let header = adw::HeaderBar::builder().show_end_title_buttons(false).build();
```

**Як варто:**
```rust
let dialog = adw::Dialog::builder()
    .title("Import Connections")
    .content_width(750)
    .content_height(500)
    .build();
// adw::Dialog автоматично обробляє modal, close gesture, Escape key
```

**Виняток:** `SettingsDialog` правильно використовує `adw::PreferencesDialog`. ✅

---

### HIG-02 [MEDIUM] Відсутній adw::Clamp в деяких діалогах

**Де:** `ProgressDialog`, `PasswordDialog`, `NewDocumentDialog`, `DocumentProtectionDialog` —
контент може розтягуватись на широких екранах.

**Як варто:** Обгорнути контент в `adw::Clamp::builder().maximum_size(600).build()`.

---

### HIG-03 [LOW] Дублювання header bar pattern

**Проблема:** Паттерн `set_show_end_title_buttons(false)` + Cancel/Action buttons дублюється ~15 разів.

**Як варто:** Створити helper:
```rust
fn create_dialog_header(title: &str, cancel_label: &str, action_label: &str) -> (adw::HeaderBar, Button, Button)
```

---

### HIG-04 [LOW] PasswordDialog використовує gtk::Entry замість gtk::PasswordEntry

**Де:** `rustconn/src/dialogs/password.rs` — поле пароля використовує `Entry` з `visibility(false)`.

**Як варто:** Використати `gtk4::PasswordEntry` який надає вбудовану іконку peek і кращу accessibility.

---

### HIG-05 [LOW] Відсутнє підтвердження для Clear History

**Де:** `rustconn/src/dialogs/history.rs` — кнопка "Clear History" видаляє всі записи без підтвердження.

**Як варто:** Показати `adw::AlertDialog` перед деструктивною дією.

---

## СЕКЦІЯ 3: Accessibility

### A11Y-01 [MEDIUM] Форми діалогів без accessible label relations

**Проблема:** Діалоги з `Grid` layout (PasswordDialog, DocumentProtectionDialog, ConnectionDialog)
мають окремі Label і Entry віджети без `gtk4::accessible::Relation::LabelledBy`.
Screen readers не зможуть асоціювати label з полем.

**Як є:**
```rust
let label = Label::new(Some("Username:"));
let entry = Entry::new();
grid.attach(&label, 0, 0, 1, 1);
grid.attach(&entry, 1, 0, 1, 1);
```

**Як варто:**
```rust
let label = Label::new(Some("Username:"));
let entry = Entry::new();
entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
grid.attach(&label, 0, 0, 1, 1);
grid.attach(&entry, 1, 0, 1, 1);
```

**Позитив:** Sidebar, filter buttons, toolbar buttons вже мають accessible labels. ✅

---

## СЕКЦІЯ 4: Wayland

### WL-01 [INFO] Wayland subsurface — Cairo fallback by design

**Де:** `rustconn/src/wayland_surface.rs` — `RenderingMode::detect()` завжди повертає `CairoFallback`.
Native `wl_subsurface` не реалізований через `unsafe_code = "forbid"`.

**Статус:** Це свідоме архітектурне рішення. Cairo rendering працює на обох Wayland і X11.
Документувати це обмеження в USER_GUIDE.

---

### WL-02 [LOW] Stubbed Wayland handles в embedded viewers

**Де:**
- `rustconn/src/embedded_rdp/buffer.rs` — `WaylandSurfaceHandle` з no-op методами
- `rustconn/src/embedded_vnc_types.rs` — `VncWaylandSurface` з no-op методами

**Як варто:** Або видалити stubs, або додати `#[cfg(feature = "wayland-native-subsurface")]` guard
щоб не плутати при code review.

---

## СЕКЦІЯ 5: Flatpak / Packaging

### PKG-01 [HIGH] Debian --all-features без build-deps для spice-embedded ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. `--all-features` видалено з `debian/rules`.

**Де:** `debian/rules` — `cargo build --release --offline --all-features` включає `spice-embedded`,
але `debian/control` не має build dependency для `spice-client` crate system libs.

**Як варто:** Або прибрати `--all-features` (як в RPM/Flatpak), або додати відповідні build-deps.

---

### PKG-02 [HIGH] Locale (.mo) файли не встановлюються в Debian/RPM ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. Додано компіляцію .po → .mo в `debian/rules` (+ `gettext` в Build-Depends) і `rustconn.spec` (+ `gettext-tools` в BuildRequires).

**Де:**
- `debian/rules` — немає кроку компіляції/встановлення .po → .mo
- `packaging/obs/rustconn.spec` — аналогічно

**Як варто:** Додати в обидва:
```bash
for po_file in po/*.po; do
  lang=$(basename "$po_file" .po)
  mkdir -p $DESTDIR/usr/share/locale/$lang/LC_MESSAGES
  msgfmt -o $DESTDIR/usr/share/locale/$lang/LC_MESSAGES/rustconn.mo "$po_file"
done
```

---

### PKG-03 [MEDIUM] Flatpak local manifest без locale ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. Додано компіляцію locale в `io.github.totoshko88.RustConn.local.yml`.

**Де:** `packaging/flatpak/io.github.totoshko88.RustConn.local.yml` — немає кроку компіляції .po → .mo
(на відміну від main і flathub маніфестів).

---

### PKG-04 [MEDIUM] `<categories>` в metainfo.xml deprecated ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. Блок `<categories>` видалено з metainfo.xml.

**Де:** `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml` — блок `<categories>` deprecated
з AppStream 0.16. Категорії вже є в `.desktop` файлі.

**Як варто:** Видалити `<categories>` блок з metainfo.xml.

---

### PKG-05 [MEDIUM] VTE версія: маніфести 0.78.7 vs changelog 0.83.90

**Де:** Всі три Flatpak маніфести використовують VTE 0.78.7, але changelog v0.8.7 згадує оновлення до 0.83.90.
Flathub x-checker-data обмежує `< 0.79.0`.

**Як варто:** Узгодити версію VTE між маніфестами і changelog.

---

### PKG-06 [LOW] Midnight Commander source URL — HTTP замість HTTPS

**Де:** Всі три Flatpak маніфести: `http://ftp.midnight-commander.org/mc-4.8.33.tar.xz`

**Як варто:** Перевірити наявність HTTPS дзеркала. Flathub віддає перевагу HTTPS.

---

### PKG-07 [LOW] --device=all без linter-exception в flathub.json

**Де:** `packaging/flathub/flathub.json` — `--device=all` не задокументований в linter-exceptions.

**Як варто:** Додати:
```json
"--device=all": "Serial console access via picocom requires device access"
```

---

### PKG-08 [LOW] Desktop file — відсутні SingleMainWindow і DBusActivatable

**Де:** `rustconn/assets/io.github.totoshko88.RustConn.desktop`

**Як варто:** Додати `SingleMainWindow=true` (GNOME HIG рекомендація для single-instance apps).

---

### PKG-09 [LOW] Debian freerdp2-x11 без альтернативи freerdp3 ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. `Recommends: freerdp2-x11 | freerdp3-x11 | freerdp3-wayland`.

**Де:** `debian/control` — `Recommends: freerdp2-x11`. Changelog v0.7.3 додав підтримку FreeRDP 3.x.

**Як варто:** `Recommends: freerdp2-x11 | freerdp3-x11 | freerdp3-wayland`

---

### PKG-10 [LOW] libadwaita feature: Cargo.toml v1_5 vs changelog v1_6

**Де:** `Cargo.toml` — `features = ["v1_5"]`, changelog v0.5.9 — "Updated to v1_6".

**Як варто:** Узгодити. Якщо використовуються API з v1_6, потрібно оновити feature flag.

---

## СЕКЦІЯ 6: Логування (eprintln! → tracing)

### LOG-01 [HIGH] 25+ eprintln! замість tracing ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. Всі 22 `eprintln!` виклики мігровані на `tracing` (session manager, VNC fallback, RDP connection, terminal spawn, export, template deletion). Залишені тільки допустимі винятки: `app.rs` (tracing ще не ініціалізований) і `rustconn-cli/src/main.rs` (CLI stderr).

**Проблема:** Strict rule проєкту забороняє `eprintln!` для логування. Знайдено 25+ порушень.

**Критичні (connection failure paths):**
| Файл | Рядок | Контекст |
|------|-------|----------|
| `rustconn/src/window/rdp_vnc.rs` | 413 | RDP reconnect failed |
| `rustconn/src/window/rdp_vnc.rs` | 482 | RDP connection failed |
| `rustconn/src/window/rdp_vnc.rs` | 544 | Failed to start RDP session |
| `rustconn/src/window/rdp_vnc.rs` | 866 | VNC reconnect failed |
| `rustconn/src/window/rdp_vnc.rs` | 872 | Failed to connect VNC session |
| `rustconn/src/session/vnc.rs` | 371,482,489 | VNC fallback messages |
| `rustconn/src/terminal/mod.rs` | 690 | Failed to spawn command |

**Інші:**
| Файл | Рядок | Контекст |
|------|-------|----------|
| `rustconn/src/app.rs` | 78,113 | State init / shutdown flush |
| `rustconn/src/window/templates.rs` | 162 | Failed to delete template |
| `rustconn/src/dialogs/export.rs` | 824 | Failed to open location |
| `rustconn-core/src/session/manager.rs` | 251,261,265,269,319,343 | Session logging |
| `rustconn-core/src/variables/manager.rs` | 300 | Undefined variable (debug_assertions) |
| `rustconn-core/src/import/asbru.rs` | 390 | Protocol detection (debug_assertions) |

**Як є:**
```rust
eprintln!("RDP connection failed for '{}': {}", conn_name, e);
```

**Як варто:**
```rust
tracing::error!(?e, connection = %conn_name, "RDP connection failed");
toast_overlay.add_toast(adw::Toast::new(&gettext("Connection failed. Check credentials.")));
```

**Виняток:** `app.rs:1221` (libadwaita init) — tracing може бути ще не ініціалізований. Допустимо.
`rustconn-cli/src/main.rs:27` — CLI stderr output для користувача. Допустимо.

---

## СЕКЦІЯ 7: Dead Code / Незавершений функціонал

### DC-01 [MEDIUM] Split view integration не завершена ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. Мертві методи `handle_session_disconnect()` і `clear_session()` видалені з `adapter.rs`. Disconnect cleanup працює через `SplitViewBridge::clear_session_from_panes()`.

**Де:** `rustconn/src/split_view/adapter.rs:307,341`
- `handle_session_disconnect()` — "Will be used in window integration tasks"
- `clear_session()` — аналогічно

**Наслідок:** При disconnect сесії split panels можуть не очищатись коректно.

**Як варто:** Інтегрувати в window layer або видалити з TODO tracking issue.

---

### DC-02 [MEDIUM] Credential caching API частково реалізований ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0. 8 мертвих методів credential caching і `verification_manager` поле видалені з `AppState`.

**Де:** `rustconn/src/state.rs` — 6+ методів credential cache (`has_cached_credentials`,
`clear_cached_credentials`, `clear_all_cached_credentials`, `refresh_cached_credentials`,
`mark_credentials_verified`) позначені `#[allow(dead_code)]`.

**Як варто:** Або інтегрувати в UI (auto-clear при logout, refresh при reconnect), або видалити.

---

### DC-03 [LOW] SharedToastOverlay helper methods не використовуються

**Де:** `rustconn/src/state.rs` або `MainWindow` — `show_toast`, `show_success`, `show_warning`,
`show_error` позначені dead_code. Callers використовують `adw::Toast` напряму.

**Як варто:** Або мігрувати callers на ці helpers, або видалити.

---

### DC-04 [LOW] Deprecated flatpak functions

**Де:** `rustconn-core/src/flatpak.rs` — `host_command`, `host_has_command`, `host_which`,
`host_exec`, `host_spawn` позначені `#[deprecated(since = "0.8.7")]`.

**Як варто:** Видалити в наступному major release.

---

### DC-05 [LOW] Stale phase comments

**Де:**
- `rustconn-core/src/protocol/vnc.rs` — "Phase 5" (vnc-rs вже реалізований)
- `rustconn-core/src/protocol/spice.rs` — "Phase 7" (spice-embedded вже є)

**Як варто:** Оновити коментарі.

---

### DC-06 [LOW] Search history popover — click handlers не підключені

**Де:** `rustconn/src/sidebar/search.rs:163-185` — `create_history_popover()` показує історію
пошуку, але клік по елементу не заповнює search entry. Також містить dev notes в коді.

---

### DC-07 [LOW] ExportResult2 — awkward naming

**Де:** `rustconn-core/src/export/mod.rs` — `ExportResult2<T>` замість нормального імені.

---

## СЕКЦІЯ 8: Resource Management

### RES-01 [MEDIUM] Embedded widgets без Drop — leak child processes

**Проблема:** `EmbeddedRdpWidget`, `EmbeddedVncWidget`, `EmbeddedSpiceWidget` не реалізують `Drop`.
Якщо widget dropped без явного `disconnect()`, child process (FreeRDP, vncviewer, remote-viewer)
може залишитись запущеним.

**Де:**
- `rustconn/src/embedded_rdp/mod.rs`
- `rustconn/src/embedded_vnc.rs`
- `rustconn/src/embedded_spice.rs`

**Як варто:** Додати `Drop` implementation:
```rust
impl Drop for EmbeddedRdpWidget {
    fn drop(&mut self) {
        self.disconnect();
    }
}
```

Або використати RAII wrapper для child process.

---

### RES-02 [LOW] Tray stub allocates orphaned mpsc channel

**Де:** `rustconn/src/tray.rs` — `#[cfg(not(feature = "tray"))]` stub створює `mpsc::channel()`
де sender одразу відкидається. `try_recv()` завжди повертає `None`.

**Як варто:** Повертати `None` без алокації каналу.

---

## СЕКЦІЯ 9: UX Issues

### UX-01 [MEDIUM] Blocking UI при import/export

**Де:**
- `rustconn/src/dialogs/import.rs` — `do_import()` виконується синхронно на GTK main thread
- `rustconn/src/dialogs/export.rs` — `do_export()` аналогічно

**Наслідок:** UI freezes при великих файлах (Remmina з сотнями з'єднань).

**Як варто:** Використати `spawn_async` pattern з progress indicator.

---

### UX-02 [LOW] ProgressDialog — 24px margins замість 12px

**Де:** `rustconn/src/dialogs/progress.rs` — GNOME HIG рекомендує 12px для dialog content.

---

### UX-03 [LOW] Settings dummy widgets anti-pattern

**Де:** `rustconn/src/dialogs/settings/mod.rs` — `setup_close_handler()` створює ~20 dummy widgets
для `collect_secret_settings()`. Якщо `SecretsPageWidgets` отримає нові поля, dummy construction
мовчки використає wrong defaults.

**Як варто:** Рефакторити `collect_secret_settings()` щоб приймати окремі widget references.

---

## СЕКЦІЯ 10: Build / Dependencies

### DEP-01 [LOW] Hand-rolled base64 замість crate

**Де:** `rustconn-core/src/secret/bitwarden.rs:1049-1080`

**Як варто:** Використати `data_encoding` або `base64` crate.

---

### DEP-02 [LOW] SessionResult type alias shadowed

**Де:** `rustconn-core/src/session/mod.rs` re-exports non-generic `SessionResult`,
а `rustconn-core/src/error.rs` визначає generic `SessionResult<T>`.

**Як варто:** Перейменувати один з них для уникнення плутанини.

---

## СЕКЦІЯ 11: Keyboard Shortcuts

### KB-01 [LOW] Діалоги без Escape/Ctrl+W shortcuts ✅ ВИПРАВЛЕНО

**Статус:** Виправлено у 0.9.0 разом з HIG-01. Міграція на `adw::Dialog` автоматично забезпечує Escape key handling для всіх 8 мігрованих діалогів.

**Де:** `HistoryDialog`, `LogViewerDialog`, `FlatpakComponentsDialog`, `ImportDialog`, `ExportDialog`
не підключають `setup_dialog_shortcuts()`.

**Як варто:** Підключити або мігрувати на `adw::Dialog` (який обробляє Escape автоматично).

---

## Уточнюючі питання

1. **Wayland subsurface:** Чи планується реалізація native wl_subsurface через окремий unsafe crate
   (наприклад, `wayland-client`), чи Cairo fallback залишається довгостроковим рішенням?

2. **spice-embedded:** Feature flag існує, `spice-client` crate підключений, але feature disabled by default.
   Який статус native SPICE embedding? Чи є плани активувати?

3. **Credential caching:** 6+ dead_code методів в state.rs. Чи це planned feature для наступного релізу,
   чи abandoned code?

4. **adw::Dialog migration:** Чи є блокери для міграції з `adw::Window` на `adw::Dialog`?
   Наприклад, потреба в custom header bar layout?

5. **Rust edition 2024:** Чи є план міграції? `std::env::set_var` стане hard error.
   Bitwarden backend найбільш affected (3 виклики в async контексті).

6. **VTE version:** Маніфести використовують 0.78.7, changelog згадує 0.83.90.
   Яка цільова версія? Чи є API breaking changes?

7. **FreeRDP thread path:** `embedded_rdp/thread.rs` — `launch_freerdp()` все ще передає пароль
   через CLI args. Це legacy path чи активно використовується? Чи можна видалити на користь
   `SafeFreeRdpLauncher`?

8. **Split view disconnect:** `handle_session_disconnect()` не інтегрований. Чи є tracking issue?
   Чи disconnect cleanup працює через інший шлях?

9. **Import/Export blocking:** Чи є реальні кейси з великими файлами (1000+ з'єднань)?
   Якщо ні, async migration може бути low priority.

10. **Locale packaging:** Debian/RPM пакети без .mo файлів — це known issue чи oversight?
    Build.rs компілює locale для `cargo run`, але packaging scripts не встановлюють їх.

---

## Відповіді на уточнюючі питання

### 1. Wayland subsurface — довгострокова стратегія

Cairo fallback залишається довгостроковим рішенням. Причини:

- `unsafe_code = "forbid"` — фундаментальне обмеження проєкту. Крейт `wayland-client` потребує unsafe для raw `wl_surface` маніпуляцій.
- Створення окремого unsafe крейту (наприклад `rustconn-wayland`) суперечить філософії проєкту і додає значну складність підтримки.
- Поточна реалізація в `wayland_surface.rs` вже має повну інфраструктуру: `ShmBuffer`, `DamageRect`, `WaylandSubsurface`, `EmbeddedRenderer` — але все працює через Cairo fallback.
- GTK4 сам обробляє Wayland compositor integration для своїх віджетів. Embedded viewers (IronRDP, vnc-rs) рендерять в `DrawingArea` через Cairo, що працює коректно на обох Wayland і X11.
- Реальний performance bottleneck — не Cairo rendering, а мережевий протокол (RDP/VNC).

**Рішення:** Документувати в USER_GUIDE як архітектурне обмеження. Не планувати native `wl_subsurface`.

---

### 2. spice-embedded — статус

Feature flag `spice-embedded` існує, `spice-client = "0.2.0"` підключений як optional dependency в `rustconn-core/Cargo.toml`. Модуль `spice_client/mod.rs` має:
- Повну інфраструктуру: `config.rs`, `error.rs`, `event.rs` з типами `SpiceClientConfig`, `SpiceClientError`, `SpiceClientCommand`, `SpiceClientEvent`
- `client.rs` підключається тільки з `#[cfg(feature = "spice-embedded")]`
- Fallback функції: `detect_spice_viewer()`, `build_spice_viewer_args()`, `launch_spice_viewer()` для `remote-viewer`
- `is_embedded_spice_available()` повертає `false` коли feature вимкнений

Feature вимкнений за замовчуванням тому що `spice-client` крейт ще не стабільний. Зовнішній fallback через `remote-viewer` працює і є основним шляхом для SPICE.

**Рішення:** Залишити як є. Активувати коли `spice-client` крейт досягне стабільності. Код не мертвий — це підготовлена інфраструктура з feature gate.

---

### 3. Credential caching — мертвий код, позначити для видалення

Перевірено: 6 методів в `state.rs` позначені `#[allow(dead_code)]` і не викликаються ніде в кодовій базі:
- `has_cached_credentials()`
- `clear_cached_credentials()`
- `clear_all_cached_credentials()`
- `refresh_cached_credentials()`
- `mark_credentials_verified()`
- `mark_credentials_unverified()`

Також є `CredentialVerificationManager` і `CachedCredentials` структури які використовуються тільки цими методами.

**Рішення:** Позначити для видалення. Це abandoned code — credential caching планувався але не був інтегрований в UI. Якщо знадобиться в майбутньому, можна відновити з git history.

---

### 4. adw::Dialog migration — блокерів немає

Перевірено поточний паттерн в `password.rs`, `history.rs` та інших діалогах:
```rust
let window = adw::Window::builder()
    .modal(true)
    .transient_for(parent)
    .build();
let header = adw::HeaderBar::new();
header.set_show_end_title_buttons(false);
// Cancel + Action buttons в header bar
```

Блокерів для міграції на `adw::Dialog` немає. Причини:
- Проєкт вже використовує `libadwaita 0.8` з feature `v1_5` — `adw::Dialog` доступний з libadwaita 1.5.
- Жоден діалог не використовує custom header bar layout який би не підтримувався `adw::Dialog`.
- `adw::Dialog` автоматично обробляє: modal behavior, Escape key, close gesture, adaptive sizing — це вирішить також KB-01 (діалоги без Escape shortcut).
- `SettingsDialog` вже правильно використовує `adw::PreferencesDialog`.

Міграція — механічна робота: замінити `adw::Window` на `adw::Dialog`, прибрати ручний header bar, адаптувати `present()` виклик. Оцінка: ~2-3 години на всі 8 діалогів.

---

### 5. Rust edition 2024 — оцінка складності міграції

Поточний стан: edition 2021, MSRV 1.88.

`std::env::set_var` стане hard error в edition 2024. Знайдено 11+ викликів:

**Bitwarden (3 виклики, найскладніші):**
- `bitwarden.rs:928` — `set_var("BW_SESSION", session_key)` після unlock з saved password
- `bitwarden.rs:992` — те саме після unlock з keyring password
- `bitwarden.rs:1023` — те саме після re-login + unlock

Ці виклики відбуваються в async контексті (tokio runtime вже запущений). Рішення: зберігати `BW_SESSION` в `Arc<RwLock<HashMap>>` в `AppState`, передавати через `Command::env()` при запуску `bw` процесів. `thread.rs` вже використовує цей підхід (коментар в коді підтверджує).

**i18n (3 виклики, середня складність):**
- `i18n.rs:258` — `set_var("LANGUAGE", lang)`
- `i18n.rs:274` — `set_var("LC_MESSAGES", &full_locale)`
- `i18n.rs:278` — `set_var("LC_MESSAGES", "en_US.UTF-8")`

Ці виклики потрібні для gettext. Рішення: викликати тільки при старті програми до запуску tokio runtime, або використати `unsafe { set_var() }` з документацією (але це порушує `unsafe_code = "forbid"`). Альтернатива: перейти на `gettext-rs` API який не потребує env vars.

**Settings dialogs (5 викликів, прості):**
- `secrets_tab.rs:584,1995,2023` — `BW_SESSION`, `OP_SERVICE_ACCOUNT_TOKEN`
- `settings/mod.rs:279` — `SSH_AUTH_SOCK`

Рішення: аналогічно Bitwarden — зберігати в `AppState`, передавати через `Command::env()`.

**Оцінка складності міграції:** Середня. ~1-2 дні роботи. Основна складність — рефакторинг Bitwarden backend для передачі session key через структуру замість env var. i18n потребує дослідження gettext-rs API.

**Рекомендація:** Не мігрувати на edition 2024 зараз. Виправити `set_var` виклики поступово, починаючи з Bitwarden (найкритичніший — async контекст). Міграцію на edition 2024 планувати після виправлення всіх `set_var`.

---

### 6. VTE version — тільки LTS версії

Поточний стан:
- Всі три Flatpak маніфести: VTE **0.78.7** (з `https://download.gnome.org/sources/vte/0.78/`)
- Flathub x-checker-data обмежує: `versions: < '0.79.0'`
- `rustconn/Cargo.toml`: `vte4 = "0.9"` з feature `v0_72`
- Changelog 0.8.7 згадує "VTE updated to 0.83.90" — це помилка в changelog, фактично маніфести не оновлювались

VTE 0.78.x — це LTS гілка для GNOME 46/47. VTE 0.79+ — development branch для GNOME 48+.

**Рішення:** Залишити VTE 0.78.7 (LTS). Оновлювати тільки до наступної LTS гілки коли GNOME Platform runtime в Flatpak оновиться. Виправити changelog — прибрати згадку про 0.83.90 або уточнити що це стосувалось тільки dev-тестування. x-checker-data `< 0.79.0` правильно обмежує до LTS.

---

### 7. FreeRDP thread.rs — підтвердження: можна видалити legacy path

Перевірено: `embedded_rdp/thread.rs:launch_freerdp()` **дійсно передає пароль через `/p:{password}`** в CLI args (рядок 420):
```rust
cmd.arg(format!("/p:{password}"));
```

Це legacy path. `SafeFreeRdpLauncher` в `launcher.rs` правильно використовує `/from-stdin` + `stdin.write()`:
```rust
cmd.arg("/from-stdin");
cmd.stdin(Stdio::piped());
// ... після spawn:
let _ = writeln!(stdin, "{password}");
```

`thread.rs` використовується для embedded RDP (wlfreerdp в GTK widget). `launcher.rs` — для external FreeRDP.

**Рішення:** Так, варто видалити legacy path в `thread.rs`. Замінити `/p:{password}` на `/from-stdin` + stdin pipe, аналогічно до `launcher.rs`. Це пряме порушення SEC-02 і видимість пароля через `/proc/PID/cmdline`.

---

### 8. Split view disconnect — працює через інший шлях

Перевірено: `handle_session_disconnect()` і `clear_session()` в `adapter.rs` дійсно не викликаються. Але disconnect cleanup **працює через інший шлях**:

`SplitViewBridge::clear_session_from_panes()` в `bridge.rs:1008` — це метод який фактично виконує ту саму роботу. Він викликається з 8+ місць в `window/mod.rs`:
- При закритті вкладки (рядки 1124, 1130)
- При переміщенні сесії між панелями (рядки 2107, 2374)
- При виході з split view (рядок 2569)
- При cleanup (рядки 2681, 2686)

Також disconnect callbacks в `window/rdp_vnc.rs` і `window/protocols.rs` викликають `notebook.mark_tab_disconnected()` + `sidebar.decrement_session_count()`.

**Рішення:** `handle_session_disconnect()` і `clear_session()` в `adapter.rs` — це дублюючий мертвий код. `SplitViewBridge` вже має робочу реалізацію `clear_session_from_panes()`. Методи в adapter.rs можна видалити. Це не проблема — disconnect cleanup працює коректно.

---

### 9. Import/Export blocking — низький пріоритет, але варто

Перевірено: `do_import()` в `import.rs` — повністю синхронний. Викликає `importer.import()` напряму на GTK main thread. Аналогічно `do_export()`.

Реальні кейси з великими файлами можливі:
- Remmina зберігає кожне з'єднання в окремому файлі — сканування `~/.local/share/remmina/` з 500+ файлами може зайняти 1-2 секунди
- SSH config з 200+ хостів — парсинг швидкий, але I/O може блокувати
- MobaXterm/Royal TS файли можуть бути великими

Для типового використання (10-50 з'єднань) блокування непомітне. Для enterprise сценаріїв (500+ з'єднань) — UI freeze помітний.

**Рішення:** Низький пріоритет. Варто мігрувати на `spawn_async` з progress indicator коли з'явиться час. Не блокер для релізу.

---

### 10. Locale packaging — oversight, варто виправити

Перевірено:
- `debian/rules` — **немає** кроку компіляції .po → .mo. Тільки бінарники, іконки, desktop file, metainfo.
- `packaging/obs/rustconn.spec` — **аналогічно**, немає locale installation.
- `rustconn/build.rs` — компілює .po → .mo в `OUT_DIR/locale/` для `cargo run`. Це працює тільки для development.
- Flatpak маніфести (flathub, flatpak) — **правильно** компілюють locale в build-commands.
- `packaging/flatpak/io.github.totoshko88.RustConn.local.yml` — **не перевірено**, можливо теж без locale (PKG-03).

Це oversight — Debian/RPM пакети будуть без перекладів.

**Щодо `Project-Id-Version`:** Всі .po файли крім `uk.po` мають `Project-Id-Version: rustconn 0.8.7` — це версія коли i18n був доданий. `uk.po` має `0.9.0` (оновлено нами). `rustconn.pot` теж має `0.8.7`.

`Project-Id-Version` — це метадані PO-файлу, вони **не впливають на роботу gettext**. `msgfmt` компілює .po → .mo ігноруючи це поле. Воно використовується тільки інструментами перекладу (Poedit, Weblate) для відображення версії.

**Рішення:**
1. Додати locale installation в `debian/rules` і `rustconn.spec` (HIGH — PKG-02)
2. Оновити `Project-Id-Version` в усіх .po та .pot файлах до `0.9.0` — це косметичне, але правильне. Можна зробити скриптом:
   ```bash
   sed -i 's/Project-Id-Version: rustconn 0.8.7/Project-Id-Version: rustconn 0.9.0/' po/*.po po/*.pot
   ```
3. Надалі оновлювати `Project-Id-Version` при кожному релізі через `update-pot.sh`

---

## Пріоритизація

| Пріоритет | Всього | Виправлено | Залишилось | Категорія |
|-----------|--------|------------|------------|-----------|
| HIGH | 5 | 5 | 0 | ~~SEC-01~~ (частково), ~~SEC-02~~, ~~PKG-01~~, ~~PKG-02~~, ~~LOG-01~~ |
| MEDIUM | 10 | 7 | 3 | ~~SEC-03~~, ~~SEC-04~~, ~~HIG-01~~, HIG-02, A11Y-01, ~~PKG-03~~, ~~PKG-04~~, PKG-05, ~~DC-01~~, ~~DC-02~~, RES-01, UX-01 |
| LOW | 17 | 2 | 15 | ~~PKG-09~~, ~~KB-01~~, SEC-01 (залишок), SEC-05, SEC-06, HIG-03, HIG-04, HIG-05, WL-02, PKG-06, PKG-07, PKG-08, PKG-10, DC-03, DC-04, DC-05, DC-06, DC-07, DEP-01, DEP-02, UX-02, UX-03, RES-02 |
| INFO | 1 | 0 | 1 | WL-01 |

**Виправлено у 0.9.0:** 14 з 33 знахідок (42%)

**Залишилось — рекомендований порядок:**
1. SEC-01 залишок (SecretString для KeePassXC, Bitwarden, Passbolt, RDM, Variables, Dialog results)
2. RES-01 (Drop for embedded widgets — leak child processes)
3. A11Y-01 (accessible label relations в діалогах)
4. HIG-02 (adw::Clamp для широких екранів)
5. PKG-05 (VTE version consistency)
6. UX-01 (async import/export)
7. Решта LOW за зручністю
