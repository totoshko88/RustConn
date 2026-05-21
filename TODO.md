# RustConn — Audit Backlog (post-hotfix)

> Generated from the 0.14.1 audit (2026-05-19).
> Hotfix items live in CHANGELOG `[Unreleased]`; everything below is **out of hotfix**
> and scheduled for 0.15.x or later.

Severity legend:
- **blocker** — feature broken / data loss / declared in README but missing
- **major** — significant deviation from HIG, missing CLI parity, design rule violation
- **minor** — code smell or limited UX impact
- **nit** — cosmetic / docs

---

## ARCH-1 [major] ✅ Винести pre-connect probe-bypass логіку в `Connection` (done in 0.14.4)

Реалізовано `Connection::bypasses_direct_probe()` та `Connection::should_pre_connect_check()`
у `rustconn-core/src/models/connection.rs`. Покриті протоколи: SSH (jump_host_id, proxy_command),
SFTP (аналогічно SSH), RDP (jump_host_id, gateway), VNC (jump_host_id),
SPICE (jump_host_id, proxy), ZeroTrust (завжди), Web (завжди).

GUI call sites (`protocols.rs`, `credentials.rs`, `edit_actions.rs`) використовують
централізовані методи. Inherited proxy jump (з групи) обробляється окремо на call site
(потребує зовнішнього контексту груп).

Property tests: `rustconn-core/tests/properties/connection_probe_tests.rs` — 16 тестів
покривають усі протоколи та комбінації bypass/settings/skip_port_check.

---

## ARCH-2 [major] ✅ File-locking конфігу (done in 0.14.4)

`rustconn-core/src/config/manager.rs` тепер використовує `fs2` advisory lock (`.lock` файл
у config_dir) перед кожною операцією запису. Паралельні `rustconn` GUI + `rustconn-cli add`
більше не дають lost-update — другий процес чекає звільнення lock.
Додано `ConfigError::Lock` варіант, тести `test_acquire_lock_exclusive` та
`test_concurrent_save_with_lock`.

---

## ARCH-3 [major] window_mode — UI vs реальність

Поле `Connection.window_mode` обробляється лише у `window/rdp_vnc.rs` для RDP/VNC.
Для SSH/SPICE/Telnet/Mosh/K8s/Serial/ZeroTrust значення мовчки ігнорується.

Варіант A (рекомендований): прибрати поле з UI повсюдно (або сховати для не-RDP/VNC),
лишити в моделі для backward compatibility.
Варіант B: поширити обробку на всі протоколи (External -> окремий adw::ApplicationWindow,
Fullscreen -> gtk::Window::fullscreen()).

Імпакт: усуває false expectations. Варіант A безпечний.

---

## ARCH-4 [major] ✅ Перевести `add_key()` на `&SecretString` (done in 0.14.4)

`rustconn-core/src/ssh_agent/mod.rs:500` тепер приймає `passphrase: Option<&SecretString>`.
Проміжні рядки (escaped, script_content) обгорнуті в `Zeroizing::new()`.
Call site у `ssh_agent_tab.rs` обгортає GString у `SecretString::from()`.
Public API change — semver-break для 0.15.0 (але ми ще на 0.14.x internal).

---

## ARCH-5 [major] Декомпозиція файлів >2000 рядків

11 кандидатів. По одному файлу за раз, кожен PR — pure-move без логіки.

| File | Lines | Подальша структура |
|------|-------|-------------------|
| dialogs/connection/dialog.rs | 7176 | per-tab modules уже існують частково; винести validate(), build_connection(), populate_from() у dialog/{builders,validation,persistence}.rs |
| window/mod.rs | 4000 | actions -> window/actions/{connection,group,session,sync,view}.rs |
| terminal/mod.rs | 3803 | playback, recording, snippets — у вже існуючі submodules |
| dialogs/template.rs | 3511 | builtin-templates у template/builtin.rs |
| window/edit_dialogs.rs | 3213 | Edit Group -> window/edit_dialogs/group.rs з PreferencesDialog |
| window/protocols.rs | 3099 | per-protocol launch logic у window/protocols/{...}.rs |
| rustconn-core/src/models/protocol.rs | 2967 | per-protocol struct'и в окремі файли |
| dialogs/settings/secrets_tab.rs | 2533 | per-backend модулі |
| dialogs/import.rs | 2518 | source-detect та per-source UI у import/sources.rs |
| state.rs | 2501 | sub-state структури state/{connections,sessions,sync}.rs |
| embedded_vnc.rs | 2055 | UI у embedded_vnc/ui.rs (об'єднати з embedded_vnc_ui.rs) |

Імпакт: час компіляції падає, code review простіший, ризик регресій низький при чистому move.

---

## SEC-1 [major] CLI `--password` -> Zeroizing одразу

`rustconn-cli/src/cli.rs:1119` — `password: Option<String>`. ~~Між clap і `SecretString::from`
пароль існує як plain heap String + видимий у /proc/<pid>/cmdline.~~

✅ **Done in [Unreleased]**: `--password` argv обгорнуто в `Zeroizing<String>` одразу після parse;
додано `--password-stdin` як безпечну альтернативу (stdin pipe, не видно в procfs);
`--password` deprecated з runtime warning. Повне видалення `--password` — у 0.15.0 (breaking).

---

## SEC-2 [minor] Askpass на CoW-FS

`rustconn-core/src/ssh_agent/mod.rs:519`. Перезапис нулями ненадійний на APFS/btrfs.
Альтернатива — pipe-на-stdin замість файлу, або memfd_create на Linux.

---

## CLI-1 [blocker] CLI add/update — додати поля Connection

### Wave 1 ✅ (done in 0.14.4)

Загальні поля додані до `add` та `update`:
- `--tags TAG[,TAG...]` — comma-separated tags
- `--description TEXT` — connection description
- `--group NAME` — assign to group (creates if missing)
- `--domain DOMAIN` — Windows domain for RDP/SPICE
- `--window-mode MODE` — embedded/external/fullscreen
- `--skip-port-check` — skip pre-connect TCP check

Для `update` додатково:
- `--add-tag TAG` — додати тег (repeatable)
- `--remove-tag TAG` — видалити тег (repeatable)
- `--skip-port-check=false` — зняти прапорець

Додано helper `find_or_create_group_id()` у `util.rs`.

### Wave 2 (SSH) ✅ (done in 0.14.4)

SSH/SFTP advanced fields додані до `add` та `update`:
- `--x11-forwarding` — enable X11 forwarding (-X)
- `--agent-forwarding` — enable SSH agent forwarding (-A)
- `--compression` — enable compression (-C)
- `--startup-command TEXT` — command to execute on connection
- `--proxy-command TEXT` — SSH ProxyCommand
- `--ssh-option K=V` — custom SSH option (repeatable)
- `--local-forward L:H:P` — local port forwarding (repeatable)
- `--remote-forward R:H:P` — remote port forwarding (repeatable)
- `--dynamic-forward PORT` — dynamic SOCKS forwarding (repeatable)

Додано helper `apply_ssh_wave2_fields()`, `parse_port_forward()`, `parse_dynamic_forward()` у `add.rs`.

### Wave 2 (RDP) ✅ (done in 0.14.4)

RDP advanced fields додані до `add` та `update`:
- `--gateway HOST` — RDP gateway hostname
- `--gateway-port PORT` — gateway port (default 443)
- `--gateway-username USER` — gateway username
- `--remote-app-program PATH` — RemoteApp program
- `--remote-app-args ARGS` — RemoteApp arguments
- `--remote-app-name NAME` — RemoteApp display name
- `--resolution WxH` — screen resolution (e.g. 1920x1080)
- `--color-depth BITS` — color depth (8/15/16/24/32)
- `--disable-nla` — disable Network Level Authentication
- `--keyboard-layout KLID` — keyboard layout override
- `--audio-redirect` — enable audio redirection
- `--shared-folder NAME:PATH` — shared folder (repeatable)

Додано helper `apply_rdp_fields()`, `apply_rdp_fields_update()`, `parse_resolution()`, `parse_shared_folder()`.

VNC/SPICE/MOSH/Serial — ✅ (done in 0.14.4+).

Імпакт: повний паритет з GUI, headless management як обіцяно у README.

---

## CLI-2 [blocker] ✅ Команди верхнього рівня (done in 0.14.4+)

Додано у Commands enum та реалізовано:

- `History` (list / clear / show <id>) — перегляд історії підключень
- `Pin { name }` / `Unpin { name }` — закріпити/відкріпити з'єднання
- `Tag` (add / remove / list) — управління тегами
- `Move { name, --group }` — перемістити з'єднання в групу
- `Monitor` (enable / disable / metrics) — управління моніторингом

Файли: `commands/{history,pin,tag,move_cmd,monitor}.rs`.

---

## CLI-3 [major] ✅ Auto-detect imports (done in 0.14.4+)

Додано `--auto` flag до Import command:
- `Import { --auto, file: Option<PathBuf> }` — conflicts_with "file"
- Виклик `is_available()` для Asbru (`~/.config/asbru-cm/`), Remmina (`~/.local/share/remmina/`),
  SSH config (`~/.ssh/config`)
- Автоматичний імпорт знайдених джерел з дедуплікацією

---

## CLI-4 [major] ✅ CSV options + import dry-run (done in 0.14.4+)

Додано:
- `Export { --csv-delimiter [comma|semicolon|tab], --csv-fields FIELDS }` — кастомізація CSV експорту
- `Import { --dry-run }` — показати що буде імпортовано без збереження
- Поля `csv_delimiter: Option<char>` та `csv_fields: Option<Vec<String>>` додані до `ExportOptions`

---

## UX-1 [major] Міграція великих діалогів на adw::Dialog

25 діалогів досі на adw::Window. High-impact:

connection/dialog.rs:1729, template.rs:127, import.rs:52, export.rs:93, cluster.rs:55,
snippet.rs:63, smart_folder.rs:66, variables.rs:61, recording.rs:76, tunnel.rs:44.

Pattern (як password.rs):

    let dialog = adw::Dialog::builder()
        .title(i18n("New Connection"))
        .content_width(600)
        .content_height(720)
        .build();
    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&clamped_content));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(parent_widget));

Імпакт: bottom-sheet на narrow, auto-close on Escape, drag-to-close.

---

## UX-2 [major] ConnectionDialog adaptive

Після UX-1 додати breakpoint + AdwClamp:

    let breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            500.0, adw::LengthUnit::Sp,
        ),
    );
    breakpoint.add_setter(&view_switcher_bar, "reveal", &true.to_value());
    dialog.add_breakpoint(breakpoint);

    let clamp = adw::Clamp::builder().maximum_size(600).build();
    clamp.set_child(Some(&main_box));

---

## UX-3 [major] Edit Group -> PreferencesDialog tabs

`window/edit_dialogs.rs:625-1351` пакує SSH+Sync+Automation+Dynamic+Description у один Box.
Розбити на adw::PreferencesDialog:

- Identity: name, icon, parent, description.
- SSH Inheritance: key path, auth method, ProxyJump, agent socket.
- Cloud Sync: sync_mode, sync_file, access_devices.
- Automation: expect_rules, post_login_scripts.
- Dynamic Folder: script, workdir, timeout, refresh_interval.

---

## UX-4 [major] ✅ Quick Connect history персистити (done in 0.14.4)

`window/types.rs` — `QuickConnectHistoryEntry` тепер має методи `from_persisted()`/`to_persisted()`
для конвертації у `rustconn_core::config::QuickConnectHistoryItem`.
Додано поле `quick_connect_history: Vec<QuickConnectHistoryItem>` у `AppSettings` (без секретів —
лише protocol/host/port/username, `skip_serializing_if = Vec::is_empty`).
`load_quick_connect_history()` читає з settings при ініціалізації MainWindow,
`persist_quick_connect_history()` зберігає після кожного нового connect.

---

## UX-5 [major] Wizard SecurityKey + fluid Advanced

1. У `dialogs/connection_wizard/auth_page.rs:91-101` додати CheckButton "Security Key (FIDO2)".
2. Замість close+open при OpenAdvanced — push сторінки advanced'а у тому ж NavigationView.

---

## UX-6 [minor] OK/Cancel pair у dialog_header()

`dialogs/widgets.rs:25-39` повертає start_btn (Cancel). Прибрати: adw::Dialog сам ловить Escape.

    pub fn dialog_header(end_label: &str) -> (adw::HeaderBar, Button) { ... }

Користувачі: password.rs:64, document.rs:73, 337, 562.

---

## UX-7 [minor] ✅ CheckButton-у-ActionRow -> AdwSwitchRow (done in 0.14.3)

25 toggles converted across `dialogs/settings/{ui_tab,terminal_tab,monitoring_tab,logging_tab}.rs`.
Pattern: `CheckButton` + `AdwActionRow` → `AdwSwitchRow`. Signal: `connect_toggled` → `connect_active_notify`.

**UX-7b ✅** (done in 0.14.3): `secrets_tab.rs` 4 backend pairs of "Save password" + "Save to keyring"
CheckButtons collapsed into a single `AdwComboRow` with three canonical choices ("Don't save" /
"Encrypted file (machine-specific)" / "System keyring (recommended)"). The hand-rolled mutual-exclusion
code is gone; `secret-tool` availability is enforced inside `make_storage_combo()`. Persistence is
unchanged: `CredentialStorage` enum + `*_storage()` / `set_*_storage()` helpers on `SecretSettings` map
to/from the legacy `*_password_encrypted` + `*_save_to_keyring` fields, so old configs round-trip
without a migration step. Property tests in
`rustconn-core/tests/properties/credential_storage_tests.rs` cover the mapping table, the round-trip,
the legacy-conflict resolution, and the field-clearing semantics for "None" / "SystemKeyring".

Affected backends: KeePassXC, Bitwarden, 1Password, Passbolt.

---

## UX-8 [minor] Color scheme: AdwToggleGroup замість 3 ToggleButton у Box

`dialogs/settings/ui_tab.rs:40-89`. На libadwaita 1.7+ -> AdwToggleGroup.
Якщо <1.7 -> AdwComboRow з 3 варіантами.

---

## UX-9 [minor] ✅ Auto-reconnect банер — attempt N/M (done in 0.14.2)

`terminal/mod.rs:2143-2200` — банер тепер показує `i18n_f("Auto-reconnecting (attempt {}/{})", ...)`
з live updates через background→UI channel.

---

## UX-10 [minor] ✅ external_window.rs -> libadwaita (done in 0.14.3)

`rustconn/src/external_window.rs` — migrated from `gtk4::ApplicationWindow` + `gtk4::HeaderBar`
to `adw::ApplicationWindow` + `adw::ToolbarView` + `adw::HeaderBar`. Consistent with the rest
of the application, inherits Adwaita styling and color scheme support.

---

## UX-11 [minor] ✅ Icon-buttons без accessible::Property::Label (done in 0.14.2)

Всі icon-only buttons у проекті вже мають `update_property(&[gtk4::accessible::Property::Label(...)])`:
window/snippets.rs (Execute/Edit/Delete), dialogs/connection/ssh.rs (ssh_key_browse),
window/edit_dialogs.rs (ssh_key_browse_btn, save_btn, connect_btn), window/ui.rs (all header buttons),
sidebar/mod.rs (help, filter_toggle), sidebar_ui.rs (all toolbar + bulk action buttons),
smart_folder_ui.rs (add_button), embedded.rs (fullscreen, disconnect), terminal/mod.rs (overview).

---

## UX-12 [nit] ✅ Search-syntax help popover локалізувати (done in 0.14.2)

`sidebar/search.rs:11-42` — замінено hardcoded EN markup на `i18n("Search Syntax")` +
`add_css_class("heading")` для заголовка, 6 рядків синтаксису через `i18n_f()` з `{}`-плейсхолдерами.

---

## UX-13 [nit] ✅ ui.rs:77 stale tooltip (done in 0.14.4)

`rustconn-core/src/search/command_palette.rs:130`: Command Palette description for "New Group"
showed "Ctrl+Shift+N" but the actual keybinding is "Ctrl+Shift+G" (`keybindings.rs:169`). Fixed.

---

## TEST-1 [minor] Property tests на регресуючі сценарії

Додати у `rustconn-core/tests/properties/`:

- `csv_port_overflow.rs` — генерувати CSV з port > u16::MAX, очікувати Err.
- `connection_probe.rs` — після ARCH-1, для випадково згенерованого Connection.
- `concurrent_save.rs` — два ConfigManager одночасно, після ARCH-2.
- `sync_path_traversal.rs` — fuzz sync_file з .., абсолютними шляхами.

---

## DOC-1 [minor] ✅ Оновити docs/CLI_REFERENCE.md (done in 0.14.4)

Версія оновлена до "0.14.4". Додано повну документацію для:
- CLI-1: всі advanced fields (SSH/RDP/VNC/SPICE/MOSH/Serial) з таблицями та прикладами
- CLI-2: history, pin/unpin, tag, move, monitor — окремі секції з usage
- CLI-3: `--auto` flag з описом auto-detect джерел
- CLI-4: `--csv-delimiter`, `--csv-fields`, `--dry-run` з прикладами

---

## RDP-1 [minor] ✅ Додаткові Quick Actions для Windows (done in [Unreleased])

Додано 3 нові дії до меню "⋮" в RDP-сесії:
- `disk-management` — Win+R → `diskmgmt.msc` (управління дисками)
- `resource-monitor` — Win+R → `resmon` (детальний моніторинг CPU/RAM/Disk/Network)
- `computer-management` — Win+R → `compmgmt.msc` (диски, служби, користувачі, event log)

Загалом тепер 9 quick actions (було 6).

---

## RDP-2 [major] Run Script — виконання PowerShell-скриптів через clipboard-paste

Новий тип дії для RDP quick actions: замість тільки key sequences, підтримка
clipboard→paste workflow для складних скриптів (очищення temp, ротація логів IIS тощо).

Механізм:
1. Покласти PowerShell-команду в локальний clipboard через RDP clipboard channel
2. Відкрити PowerShell (Win+R → powershell → Enter)
3. Зачекати ~500ms
4. Ctrl+V → Enter

Потребує:
- Новий enum `QuickActionSequence { KeySequence(...), ClipboardThenKeys { text, keys } }`
- Зміна `SendKeySequence` command або новий `RunScript` command в IronRDP client
- UI: окрема секція "Scripts" в меню або окрема кнопка

Scheduled: 0.16.x

---

## CODE-1 [minor] Усунути дрібну дубльованість

- ✅ `vault_ops.rs:482-493` — `collect_descendant_groups` замінено на `rustconn_core::models::collect_descendant_group_ids()` (O(n) BFS замість O(n²) рекурсії). Done in 0.14.4.
- `cli_download/extract.rs::find_binary_*` vs `dialogs/settings/clients_tab.rs:613` — об'єднати.
- `commands/connect.rs:69-110` ZeroTrust build_command — делегувати у ProtocolRegistry.

---

## Roadmap suggestion

| Release | Items |
|---------|-------|
| 0.14.2 (hotfix) | ARCH-1 ✅, UX-9 ✅, UX-11 ✅, TEST-1 (connection_probe) ✅ |
| 0.14.3 (UI polish) | UX-7 ✅, UX-7b ✅, UX-10 ✅, SEC-1 ✅ |
| 0.14.4 | UX-13 ✅, UX-12 ✅, DOC-1 ✅, ARCH-1 ✅, ARCH-2 ✅, ARCH-4 ✅, UX-4 ✅, CLI-1 wave 1 ✅, CLI-1 wave 2 SSH ✅, CLI-1 wave 2 RDP ✅, CLI-1 wave 2 VNC/SPICE/MOSH/Serial ✅, CLI-2 ✅, CLI-3 ✅, CLI-4 ✅ |
| 0.15.x | UX-1 (high-impact dialogs), UX-2, UX-5 |
| 0.16.0 | ARCH-3 (decision), ARCH-5 (decomposition by file) |
| 0.16.x | UX-3, UX-6, UX-8, TEST-1 (решта), CODE-1 |
| 0.17.0 | UX-1 решта (low-impact dialogs) |
