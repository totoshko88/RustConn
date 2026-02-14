# RustConn 0.8.5 — Задачі релізу

Трекер задач для релізу 0.8.5. Кожна задача має чіткий scope, acceptance criteria та залежності.

Пов'язані issues: [#11](https://github.com/totoshko88/RustConn/issues/11), [#10](https://github.com/totoshko88/RustConn/issues/10), [#9](https://github.com/totoshko88/RustConn/issues/9), [#7](https://github.com/totoshko88/RustConn/issues/7)

---

## Загальні принципи

- Зовнішні клієнти як основа (picocom/minicom, xdg-open, sftp CLI)
- GUI + CLI уніфіковані: кожен новий протокол/фіча доступні в обох інтерфейсах
- Після кожної фічі — оновлення README.md, docs/ARCHITECTURE.md, docs/USER_GUIDE.md
- GNOME-native, але з фолбеками для KDE/Cosmic (xdg-open замість nautilus)
- Діалоги адаптивні: мінімальні розміри замість фіксованих, responsive breakpoints

---

## Фаза 1 — Serial Console (#11)

### 1.1 Модель даних (rustconn-core)
- [x] Додати `ProtocolType::Serial` в `rustconn-core/src/models/protocol.rs`
- [x] Створити `SerialConfig` struct:
  - `device: String` (шлях, напр. `/dev/ttyUSB0`)
  - `baud_rate: u32` (default: 115200)
  - `data_bits: SerialDataBits` (enum: Five, Six, Seven, Eight)
  - `stop_bits: SerialStopBits` (enum: One, Two)
  - `parity: SerialParity` (enum: None, Odd, Even)
  - `flow_control: SerialFlowControl` (enum: None, Hardware, Software)
  - `custom_args: Vec<String>`
- [x] Додати `ProtocolConfig::Serial(SerialConfig)` variant
- [x] Реалізувати `Default` для `SerialConfig` (115200/8N1/None)
- [x] Додати `Display` для Serial в `ProtocolType::fmt()`
- [x] Оновити `ProtocolType::default_port()` → 0 для Serial (не мережевий)
- [x] Оновити `ProtocolType::as_str()` → `"serial"`

**Acceptance:** `cargo test -p rustconn-core` проходить, серіалізація/десеріалізація SerialConfig працює.

### 1.2 Protocol trait (rustconn-core)
- [x] Створити `rustconn-core/src/protocol/serial.rs` з `SerialProtocol`
- [x] Реалізувати `Protocol` trait:
  - `protocol_id()` → `"serial"`
  - `display_name()` → `"Serial"`
  - `default_port()` → 0
  - `validate_connection()` — перевірка що device не порожній
  - `capabilities()` → `ProtocolCapabilities::terminal()`
  - `build_command()` → `picocom` з аргументами (baud, parity, databits, flow)
- [x] Зареєструвати в `ProtocolRegistry` (`protocol/mod.rs`)
- [x] Додати unit тести (аналогічно telnet.rs)

**Acceptance:** `SerialProtocol::build_command()` генерує коректну команду picocom.

### 1.3 CLI підтримка (rustconn-cli)
- [x] Додати `"serial"` в `parse_protocol()` → `(ProtocolType::Serial, 0)`
- [x] Додати `build_serial_command()` (аналогічно `build_telnet_command()`)
- [x] Додати serial-специфічні прапорці в `cmd_add`:
  - `--device` (шлях до пристрою)
  - `--baud-rate` (default 115200)
  - `--data-bits`, `--stop-bits`, `--parity`, `--flow-control`
- [x] Оновити `cmd_show()` для відображення serial-параметрів
- [x] Оновити `cmd_update()` для зміни serial-параметрів
- [x] Оновити `ConnectionOutput::from()` для serial полів
- [x] Оновити help text: `"Supported protocols: ssh, rdp, vnc, spice, telnet, serial"`

**Acceptance:** `rustconn-cli add "Router Console" --protocol serial --device /dev/ttyUSB0 --baud-rate 9600` створює з'єднання; `rustconn-cli connect "Router Console"` запускає picocom.

### 1.4 GUI — Connection Dialog (rustconn)
- [x] Створити `rustconn/src/dialogs/connection/serial.rs` — Serial таб
  - Device entry з кнопкою Browse (FileDialog для /dev/tty*)
  - Baud rate dropdown (9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600)
  - Data bits dropdown (5, 6, 7, 8)
  - Stop bits dropdown (1, 2)
  - Parity dropdown (None, Odd, Even)
  - Flow control dropdown (None, Hardware, Software)
  - Custom args entry
- [x] Зареєструвати serial таб в `dialog.rs` (protocol_stack, protocol_dropdown)
- [x] Додати serial в `ConnectionDialogData` збереження/завантаження

**Acceptance:** Створення serial з'єднання через GUI, всі параметри зберігаються та відновлюються при редагуванні.

### 1.5 GUI — Terminal spawn (rustconn)
- [x] Додати `spawn_serial()` в `terminal/mod.rs` (аналогічно `spawn_telnet()`)
- [x] Додати іконку для serial в `adaptive_tabs.rs` → `get_protocol_icon()` (напр. `"phone-symbolic"`)
- [x] Додати serial фільтр в sidebar (якщо є protocol filter)
- [x] Обробка помилки "picocom not found" → toast з підказкою встановити
- [x] Перевірка доступу до device: якщо `Permission denied` → toast
  "Cannot access {device}. Add your user to the 'dialout' group: sudo usermod -aG dialout $USER"

**Acceptance:** Подвійний клік на serial з'єднання відкриває VTE термінал з picocom сесією.

### 1.5a Sandbox-сумісність Serial

**Flatpak:**
- [x] Додати `--device=all` в `finish-args` всіх Flatpak manifests
  (flatpak, local, flathub) — дає доступ до `/dev/tty*` пристроїв
- [x] Додати `picocom` як build module в Flatpak manifests
  (аналогічно `inetutils` для telnet)
- [x] Задокументувати: serial потребує `--device=all` permission

**Snap (якщо буде snapcraft.yaml):**
- [ ] Додати `serial-port` plug
- [ ] Задокументувати: `snap connect rustconn:serial-port`

**Native (deb/rpm/AppImage):**
- [ ] Задокументувати вимогу групи `dialout`:
  `sudo usermod -aG dialout $USER` (потрібен re-login)

**Acceptance:** Serial працює в Flatpak з `--device=all`; toast з підказкою
при Permission denied на native.

### 1.6 Property тести (rustconn-core)
- [x] Додати `SerialConfig` strategy в property тести
- [x] Додати Serial variant в `ProtocolConfig` strategy
- [x] Зареєструвати в `tests/properties/mod.rs`

**Acceptance:** `cargo test -p rustconn-core --test property_tests` проходить з Serial coverage.

### 1.7 Документація
- [x] README.md — додати Serial в таблицю протоколів
- [x] docs/ARCHITECTURE.md — оновити Protocol section
- [x] docs/USER_GUIDE.md — секція Serial Console з прикладами
- [x] CHANGELOG.md — entry для Serial Console

---

## Фаза 2 — SFTP через зовнішній клієнт (#10)

### 2.1 Модель (rustconn-core)
- [x] Додати `sftp_enabled: bool` в `SshConfig` (default: false)
- [x] Оновити серіалізацію/десеріалізацію SshConfig

**Acceptance:** Існуючі SSH з'єднання десеріалізуються без помилок (serde default).

### 2.2 SFTP launcher (rustconn-core)
- [x] Створити `rustconn-core/src/sftp.rs` модуль:
  - `build_sftp_uri(user: &str, host: &str, port: u16) -> String` → `sftp://user@host:port`
  - `build_sftp_command(connection: &Connection) -> Option<Vec<String>>` → `["sftp", "-P", port, "user@host"]`
- [x] Зареєструвати модуль в `lib.rs`

**Sandbox-сумісність (Flatpak/Snap):**
- GUI (rustconn): використовувати `gtk4::UriLauncher::new(uri).launch()` — працює через
  D-Bus portal `org.freedesktop.portal.OpenURI`, автоматично делегує хостовому
  файловому менеджеру (nautilus, dolphin, cosmic-files, thunar) без потреби
  визначати DE вручну. Працює в native, Flatpak, Snap однаково.
- CLI (rustconn-cli): `xdg-open sftp://...` — CLI не працює в sandbox,
  тому прямий виклик xdg-open коректний.
- `sftp` CLI (для `--cli` режиму): доступний напряму, бо CLI не sandboxed.
- НЕ використовувати `Command::new("nautilus")` / `Command::new("dolphin")` —
  це не потрібно, portal вирішує це автоматично.

**Acceptance:** `build_sftp_uri()` генерує коректний URI; unit тести проходять.

### 2.3 CLI підтримка (rustconn-cli)
- [x] Додати `sftp` subcommand: `rustconn-cli sftp <connection-name>`
  - Опція `--cli` — використати `sftp` CLI замість файлового менеджера
  - За замовчуванням — відкрити файловий менеджер з sftp:// URI
- [x] Додати `--sftp` прапорець в `cmd_add` / `cmd_update` для SSH з'єднань

**Acceptance:** `rustconn-cli sftp "My Server"` відкриває файловий менеджер або sftp CLI сесію.

### 2.4 GUI — SSH сесія (rustconn)
- [ ] Додати кнопку "Open SFTP" в toolbar SSH сесії (іконка `folder-remote-symbolic`)
- [ ] При натисканні:
  1. Побудувати URI через `build_sftp_uri()`
  2. Відкрити через `gtk4::UriLauncher::new(&uri).launch(Some(&window), ...)` —
     працює через D-Bus portal, sandbox-safe
  3. Toast "Opening SFTP..."
  4. При помилці — toast "Could not open SFTP. Check that your file manager supports sftp:// URIs"
- [ ] Додати `sftp_enabled` checkbox в SSH таб connection dialog
- [ ] Показувати кнопку SFTP тільки якщо `sftp_enabled == true`

**Acceptance:** Кнопка SFTP в SSH сесії відкриває файловий менеджер хоста через portal; працює в native, Flatpak, Snap.

### 2.5 Контекстне меню sidebar
- [ ] Додати "Open SFTP" в right-click меню для SSH з'єднань з `sftp_enabled`
- [ ] Працює без активної сесії (не потрібно бути підключеним)

**Acceptance:** Right-click → "Open SFTP" відкриває файловий менеджер.

### 2.6 Документація
- [ ] README.md — додати SFTP в features
- [ ] docs/ARCHITECTURE.md — описати sftp модуль
- [ ] docs/USER_GUIDE.md — секція SFTP з прикладами
- [ ] CHANGELOG.md — entry

---

## Фаза 3 — Responsive / Adaptive UI (#9)

### 3.1 Аудит фіксованих розмірів діалогів
- [ ] Замінити фіксовані `default_width`/`default_height` на адаптивну схему:

  Поточні фіксовані розміри (17 діалогів):
  - 750×650: Connection, Export, Template, Password Generator
  - 750×500: History, Variables, Clusters, Templates list
  - 900×600: Log Viewer
  - 750×800: Import
  - 550×550: Snippet, Cluster, Statistics
  - 550×500: Shortcuts
  - 600×700: Flatpak Components
  - 500×420: WoL
  - 400×*: Password, Document, Terminal Search, Progress, SSH Agent

  Цільова схема:
  - Великі діалоги (Connection, Import, Export, Template, Log Viewer):
    - `default_width(750)` → `default_width(600)` + `width_request(350)`
    - `default_height(650)` → `default_height(500)` + `height_request(300)`
  - Середні діалоги (History, Variables, Snippets, Clusters, Statistics):
    - `default_width(550-750)` → `default_width(500)` + `width_request(320)`
    - `default_height(500-550)` → `default_height(400)` + `height_request(280)`
  - Малі діалоги (Password, Document, Progress):
    - Залишити `default_width(400)` + `width_request(280)`
    - Видалити `resizable(false)` де можливо

- [ ] Додати `set_size_request()` з мінімальними розмірами замість `resizable(false)`

**Acceptance:** Всі діалоги можна зменшити до ~350px ширини без обрізання контенту; на 1080p виглядають як раніше.

### 3.2 Breakpoints для головного вікна
- [ ] Додати breakpoint 600sp: колапс split view secondary panels
- [ ] Додати breakpoint 360sp: compact mode для header bar (іконки без тексту)
- [ ] Перевірити існуючий breakpoint 400sp (sidebar overlay) — працює коректно

**Acceptance:** На вікні 360px шириною — sidebar overlay, compact header, single panel view.

### 3.3 Адаптивні діалоги — внутрішній layout
- [ ] Connection dialog: замінити горизонтальні tab layouts на вертикальні при ширині < 500px
  - Використати `adw::Breakpoint` всередині діалогу або перевірку `get_width()`
- [ ] Settings dialog: аналогічно
- [ ] Import/Export: стек вертикально при вузькому вікні

**Acceptance:** Connection dialog юзабельний на екрані 400px шириною.

### 3.4 Scrollable content
- [ ] Перевірити всі діалоги на наявність `ScrolledWindow` для основного контенту
- [ ] Додати scroll де відсутній (особливо для форм з багатьма полями)
- [ ] Serial таб, SSH таб, RDP таб — всі мають бути scrollable

**Acceptance:** Жоден діалог не обрізає контент при зменшенні вікна.

### 3.5 Документація
- [ ] docs/USER_GUIDE.md — секція про адаптивний UI, підтримку малих екранів
- [ ] CHANGELOG.md — entry

---

## Фаза 4 — Terminal Rich Search (#7)

### 4.1 Розширення TerminalSearchDialog
- [ ] Додати regex toggle (CheckButton "Regex")
  - Використати `vte4::Regex` для regex пошуку
  - При невалідному regex — показати помилку під search entry
- [ ] Додати "Highlight All" toggle
  - Використати `Terminal::match_add_regex()` для підсвічування всіх збігів
  - Очищати highlights при закритті діалогу
- [ ] Додати лічильник збігів: "Match 3 of 17"
- [ ] Додати історію пошуку (dropdown з останніми 10 запитами)
  - Зберігати в `TerminalSettings` або in-memory per session
- [ ] Keyboard shortcut: Ctrl+Shift+F для відкриття (якщо ще немає)

**Acceptance:** Regex пошук працює в VTE терміналі; всі збіги підсвічуються; навігація між збігами.

### 4.2 CLI підтримка пошуку (rustconn-cli)
- [ ] Не потрібна — terminal search це GUI-only фіча (VTE widget)
- [ ] Задокументувати в USER_GUIDE що пошук доступний тільки в GUI

### 4.3 Timestamps в логах сесії
- [ ] Додати `log_timestamps: bool` в `TerminalSettings` (rustconn-core)
- [ ] При `log_timestamps == true` — prepend `[HH:MM:SS]` до кожного рядка в лог-файлі
- [ ] Реалізувати в `session/logging.rs` або відповідному модулі
- [ ] Додати toggle в Settings → Terminal tab

**Acceptance:** Лог-файл сесії містить timestamps коли опція увімкнена; Log Viewer показує їх.

### 4.4 Документація
- [ ] docs/USER_GUIDE.md — секція Terminal Search з описом regex, highlights
- [ ] CHANGELOG.md — entry

---

## Фаза 5 — Фіналізація релізу

### 5.1 Версія
- [x] Оновити `workspace.package.version` в `Cargo.toml` → `"0.8.5"`
- [x] Оновити версію в docs/USER_GUIDE.md, docs/ARCHITECTURE.md
- [ ] Оновити debian/changelog
- [ ] Оновити packaging/obs/rustconn.changes, rustconn.spec
- [x] Оновити packaging/flatpak manifests
- [x] Оновити metainfo.xml

### 5.2 CHANGELOG.md
- [x] Фінальний entry `[0.8.5]` з усіма змінами
- [x] Перевірити що всі issue references присутні

### 5.3 Тестування
- [ ] `cargo fmt --check` — без помилок
- [ ] `cargo clippy --all-targets` — без warnings
- [ ] `cargo test` — всі тести проходять
- [ ] `cargo test -p rustconn-core --test property_tests` — property тести з новими strategies
- [ ] Ручне тестування Serial (якщо є пристрій)
- [ ] Ручне тестування SFTP на GNOME, KDE (якщо доступні)
- [ ] Ручне тестування responsive UI на вузькому вікні (400px)
- [ ] Ручне тестування terminal search з regex

### 5.4 README.md
- [ ] Оновити feature list
- [ ] Оновити таблицю протоколів
- [ ] Оновити screenshots якщо потрібно

---

## Порядок виконання

```
Фаза 1 (Serial)     ████████████████░░░░░░░░░░░░░░  ~4-5 днів
Фаза 2 (SFTP)       ░░░░░░░░░░░░░░░░████████░░░░░░  ~2-3 дні
Фаза 3 (Responsive) ░░░░░░░░░░░░░░░░░░░░░░░░██████  ~2-3 дні
Фаза 4 (Search)     ░░░░░░░░░░░░░░░░░░░░░░░░░░████  ~1-2 дні
Фаза 5 (Release)    ░░░░░░░░░░░░░░░░░░░░░░░░░░░░██  ~1 день
```

Загальна оцінка: ~10-14 днів

## Залежності між фазами

- Фаза 1 і 2 незалежні, можна паралелити
- Фаза 3 краще робити після 1 і 2 (serial таб теж має бути responsive)
- Фаза 4 незалежна від інших
- Фаза 5 — тільки після завершення всіх фаз
