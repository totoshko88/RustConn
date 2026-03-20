# RustConn 0.10.2 — Роадмап

Глибокий аналіз коду 0.10.0 / 0.10.1 з фокусом на недороблений функціонал,
мертвий код, баги від користувачів та пропозиції покращення.

> **Методологія:** Кожна пропозиція критично оцінена з урахуванням реальних
> обмежень VTE/GTK4, історії рішень проєкту та складності імплементації.

---

## 1. КРИТИЧНІ НЕДОРОБКИ (функціонал заявлений, але не працює)

### 1.1 `session_recording_enabled` не перевіряється при підключенні

**Проблема:** Поле `Connection::session_recording_enabled` зберігається в моделі
та редагується в діалозі з'єднання (Advanced tab → "Record Session" toggle), але
**ніде в коді підключення** (`window/protocols.rs`, `window/terminal_actions.rs`)
це поле не перевіряється. Автоматичний запис при підключенні не працює.

**Доказ:** `grep -r "session_recording_enabled" rustconn/src/` знаходить лише:
- `terminal/mod.rs:2210` — коментар `#[allow(dead_code)]`
- `dialogs/connection/dialog.rs:4904,6764` — тільки UI save/load

**Файли для виправлення:**
- `rustconn/src/window/protocols.rs` — `start_ssh_connection()`, `start_serial_connection()`,
  `start_mosh_connection()`, `start_telnet_connection()`, `start_k8s_connection()`

**Пропозиція:**
```rust
// Після створення terminal tab і перед spawn:
if conn.session_recording_enabled {
    notebook.start_recording(
        session_id,
        &conn.name,
        SanitizeConfig::default(),
        ssh_params,
    );
}
```

**Критична оцінка:** Виправлення просте — 5-10 рядків в кожній `start_*_connection()`.
Але є нюанс: `start_recording()` відправляє `script` через `feed_child()`, а
spawn команди підключення теж використовує PTY. Потрібно гарантувати що `script`
запуститься **до** spawn підключення, інакше перші секунди сесії не запишуться.
Порядок: `start_recording()` → невелика затримка → `spawn_*()`.

**Пріоритет:** P0 — заявлена фіча не працює.

---

### 1.2 `highlight_rules` не застосовуються до VTE терміналів

**Проблема:** Per-connection `highlight_rules` зберігаються в моделі `Connection`
та редагуються в діалозі, але **ніколи не передаються** в `TerminalNotebook` при
підключенні. Метод `TerminalNotebook::set_highlight_rules()` існує (з позначкою
`#[allow(dead_code)]`), але не викликається з connection flow.

**Доказ:**
- `terminal/mod.rs:2505` — `#[allow(dead_code)] // Will be called by connection flow when highlighting is wired up`
- `grep -r "set_highlight_rules" rustconn/src/window/` — 0 результатів

**Файли для виправлення:**
- `rustconn/src/window/protocols.rs` — всі `start_*_connection()` функції

**Пропозиція:**
```rust
// Після створення terminal tab:
if !conn.highlight_rules.is_empty() {
    let global_rules = state.borrow().settings().highlight_rules.clone();
    notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
}
```

**Критична оцінка:** Виправлення просте — аналогічно 1.1. Метод
`set_highlight_rules()` вже реалізований і протестований. Єдине питання —
глобальні правила з Settings потрібно теж передати (merge global + per-connection).
Перевірити що `set_highlight_rules()` правильно мерджить обидва набори.

**Пріоритет:** P0 — заявлена фіча не працює.

---

## 2. БАГИ ЗАПИСУ/ВІДТВОРЕННЯ СЕСІЙ (зауваження користувачів)

> **Контекст архітектурного рішення:** Запис сесій використовує зовнішню утиліту
> `script` (через `feed_child()` в PTY), а НЕ VTE callback `connect_commit()`.
> Причина: `connect_commit()` перехоплює лише **INPUT** (натискання клавіш →
> PTY), а не **OUTPUT** (PTY → екран). Для `scriptreplay`-сумісного відтворення
> потрібен саме output stream, який записує тільки `script`. Підхід через
> `connect_commit()` був випробуваний і відхилений на етапі проєктування.
> Всі пропозиції нижче працюють в рамках `script`-підходу.

### 2.1 Команда `script` видна в терміналі при старті запису

**Проблема:** `start_recording()` (`terminal/mod.rs:2296`) відправляє команду
`script -q -f --log-out '...' --log-timing '...'` через `feed_child()`, потім
намагається стерти її через `feed(b"\x1b[1A\x1b[2K")`. Але `feed()` пише
безпосередньо в буфер VTE (клієнтська сторона), а ехо команди приходить
асинхронно від shell через PTY. Escape-послідовність стирає попередній рядок
(prompt), а не команду `script`.

**Пропозиція (варіант A — затримка перед стиранням):**
```rust
let cmd = format!(
    " script -q -f --log-out '{data_str}' --log-timing '{timing_str}'\n"
);
// Пробіл на початку — bash/zsh не додають команду в history (HISTCONTROL)
terminal.feed_child(cmd.as_bytes());
// Затримка щоб ехо встигло прийти через PTY:
let terminal_clone = terminal.clone();
glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
    terminal_clone.feed(b"\x1b[1A\x1b[2K");
});
```

**Пропозиція (варіант B — script + clear):**
```rust
let cmd = format!(
    " script -q -f --log-out '{data_str}' --log-timing '{timing_str}'; clear\n"
);
terminal.feed_child(cmd.as_bytes());
```

**Критична оцінка:**
- Варіант A: затримка 50ms — евристика. На повільних системах або remote
  сесіях ехо може прийти пізніше. Можна збільшити до 100-150ms, але це
  все одно race condition. Працюватиме в 95%+ випадків для локальних сесій.
- Варіант B: `clear` надійніший — повністю очищає екран. Але `clear` теж
  запишеться в `script` output і з'явиться при відтворенні як артефакт.
  Потрібно фільтрувати при playback (див. 2.3).
- Пробіл на початку працює тільки якщо shell має `HISTCONTROL=ignorespace`
  або `HISTCONTROL=ignoreboth` (bash default). Для zsh потрібен
  `setopt HIST_IGNORE_SPACE`. Для fish — не працює взагалі.
- **Рекомендація:** Варіант A з затримкою 100ms — найменш інвазивний.

**Пріоритет:** P1

---

### 2.2 Подвійний `exit` та довге очікування при зупинці запису

**Проблема:** `stop_recording()` (`terminal/mod.rs:2323`) відправляє
`terminal.feed_child(b"exit\n")` для завершення sub-shell `script`. Проблеми:

1. `exit` видно в терміналі як текст
2. Якщо користувач вже набрав `exit` для закриття сесії, `script` sub-shell
   отримує два `exit` — один від користувача, один від коду
3. Для remote recording — SCP операції (`std::process::Command`) блокують
   GTK main thread, що створює відчуття "зависання" (до 10-30с на повільних
   з'єднаннях)

**Пропозиція — Ctrl+D замість exit:**
```rust
// Відправити Ctrl+D (EOF) замість exit — не створює видимого виводу
if let Some(terminal) = self.get_terminal(session_id) {
    terminal.feed_child(b"\x04"); // EOT — завершує script sub-shell без echo
}
```

**Пропозиція — SCP в background thread:**
```rust
// Замість std::process::Command на GTK main thread:
if let Some(remote_info) = self.remote_recordings.borrow_mut().remove(&session_id) {
    let recording_paths = self.recording_paths.borrow_mut().remove(&session_id);
    // Перенести SCP в background thread через spawn_blocking_with_callback
    crate::utils::spawn_blocking_with_callback(
        move || {
            // SCP data + timing files
            // Cleanup remote temp files
        },
        move |result| {
            // Generate .meta.json sidecar на GTK thread
        },
    );
}
```

**Критична оцінка:**
- Ctrl+D (EOF/`\x04`): завершує `script` sub-shell чисто, без видимого
  виводу. Але якщо в sub-shell запущена інтерактивна програма (vim, htop),
  Ctrl+D може бути проігнорований або інтерпретований інакше. Для типового
  shell prompt — працює надійно.
- Подвійний exit: Ctrl+D вирішує проблему — якщо script вже завершився
  (користувач набрав exit), Ctrl+D просто ігнорується основним shell.
- SCP blocking: `spawn_blocking_with_callback` — правильний патерн, вже
  використовується в проєкті (vault ops, port checks). Потрібно перенести
  всю SCP логіку + cleanup + metadata generation.
- **Рекомендація:** Обидві зміни — Ctrl+D + async SCP. Низький ризик.

**Пріоритет:** P1

---

### 2.3 Втрата команд при відтворенні

**Проблема:** `RecordingReader::open()` (`session/recording.rs:120`) викликає
`strip_script_header()` для видалення рядка "Script started on ...". Але:

1. Ехо команди `script ...` записується в data file, але escape-послідовність
   стирання (`\x1b[1A\x1b[2K`) відправлена через `feed()` і **не записується**
   `script`-ом. При відтворенні команда `script` з'являється як артефакт.
2. `strip_script_header()` коректно обробляє header — знаходить перший `\n`,
   видаляє header і коригує timing entries. Але якщо header розбитий на
   кілька timing entries, зсув delays може бути неточним (перевірено: код
   коректний, ітерує поки `remaining > 0`).
3. `PlaybackController::schedule_next()` використовує
   `glib::timeout_add_local_once(delay, ...)` — при дуже малих delays
   (< 1ms) GTK може об'єднати callbacks, але це рідкісний edge case.

**Пропозиція — фільтрація script echo при відкритті:**
```rust
// В RecordingReader::open() — після strip_script_header():
let (data, timing_entries) = strip_script_header(data, timing_entries);
// Додатково видалити ехо команди "script -q -f --log-out ..."
let (data, timing_entries) = strip_script_command_echo(data, timing_entries);
```

```rust
fn strip_script_command_echo(
    mut data: Vec<u8>,
    mut timing: Vec<(Duration, usize)>,
) -> (Vec<u8>, Vec<(Duration, usize)>) {
    // Шукати патерн "script -q -f --log-out" в перших N байтах
    // Видалити від початку рядка до \n включно
    // Аналогічно strip_script_header — коригувати timing entries
}
```

**Критична оцінка:**
- Фільтрація script echo — надійний підхід. Патерн `script -q -f --log-out`
  унікальний і не з'явиться в звичайному виводі терміналу.
- Потрібно також фільтрувати `clear` escape sequences якщо використовується
  варіант B з 2.1 (послідовність `\x1b[H\x1b[2J\x1b[3J`).
- Мінімальний delay (`max(1ms)`) — корисний як захист, але не є основною
  причиною втрати команд. Основна причина — артефакти script echo.
- **Рекомендація:** Додати `strip_script_command_echo()` аналогічно до
  існуючого `strip_script_header()`. Складність низька — код-шаблон вже є.

**Пріоритет:** P1

---

## 3. БАГ: groups.toml — зростання розміру та синхронізація

### 3.1 Аналіз проблеми

**Результат дослідження:** Код збереження groups.toml **коректний** — типова
система Rust запобігає плутанині між `ConnectionsFile` та `GroupsFile`.
`save_groups()` використовує `GROUPS_FILE` ("groups.toml") і серіалізує
`GroupsFile { groups: Vec<ConnectionGroup> }`.

**Причина зростання розміру:** Модель `ConnectionGroup` розширювалась між
релізами — додані поля `username`, `domain`, `password_source`, `description`,
`icon`. Якщо користувач має багато груп з credential inheritance, файл зростає.

**Причина "копії connections.toml":** Ймовірно, користувач бачить схожу
структуру TOML (UUID, timestamps, назви) і вважає що це дублікат. Або це
артефакт старої версії / ручного копіювання.

### 3.2 Синхронізація конфігурації між ПК

Конфігурація зберігається в `~/.config/rustconn/`:
```
~/.config/rustconn/
├── config.toml           # Налаштування додатку
├── connections.toml      # З'єднання (містить хости, порти, username)
├── groups.toml           # Ієрархія груп
├── snippets.toml         # Команди-шаблони
├── clusters.toml         # Кластери broadcast
├── templates.toml        # Шаблони з'єднань
├── history.toml          # Історія підключень
├── smart_folders.toml    # Smart Folders
└── trash.toml            # Кошик
```

**Рекомендації для синхронізації:**

1. **Git (рекомендовано):**
   ```bash
   cd ~/.config/rustconn
   git init
   echo "history.toml" >> .gitignore  # Історія — локальна
   echo "trash.toml" >> .gitignore
   git add -A && git commit -m "Initial config"
   git remote add origin <url>
   ```
   Переваги: версіонування, merge conflicts видно, працює offline.

2. **Syncthing / rsync:**
   ```bash
   # Syncthing: додати ~/.config/rustconn як shared folder
   # rsync: cron job
   rsync -avz ~/.config/rustconn/ user@remote:~/.config/rustconn/
   ```

3. **CLI export/import:**
   ```bash
   # Експорт на ПК-1:
   rustconn-cli export --format json --output connections.json
   # Імпорт на ПК-2:
   rustconn-cli import --format json connections.json
   ```

4. **Backup/Restore (вбудовано):**
   Settings → Backup → Export створює ZIP архів всіх конфігураційних файлів.

**Пропозиція для 0.10.2:** Додати в User Guide секцію "Синхронізація між ПК"
з цими рекомендаціями. Розглянути вбудований sync через Git або cloud storage.

**Пріоритет:** P2 — документація + UX покращення.

---

## 4. GITHUB ISSUES

### 4.1 Issue #64 — .rdp файл не відкривається при подвійному кліку

**Аналіз:**
1. Desktop file (`rustconn.desktop`) оголошує `MimeType=application/x-rdp;`
2. Код CLI парсингу (`main.rs:140`) коректно розпізнає `.rdp` аргумент
3. Код обробки (`window/mod.rs:3557`) коректно парсить і підключається

**Кореневі причини:**
- **Відсутній MIME type XML файл.** `application/x-rdp` — не стандартний
  freedesktop MIME type. Потрібен XML файл що маппить розширення `.rdp` на цей
  MIME type. Без нього DE не знає що `.rdp` → `application/x-rdp`.
- **Flatpak:** Потрібно встановити XML в `/app/share/mime/packages/` і запустити
  `update-mime-database`.

**Пропозиція — створити MIME type файл:**
```xml
<!-- rustconn/assets/io.github.totoshko88.RustConn-rdp.xml -->
<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-rdp">
    <comment>Remote Desktop Protocol file</comment>
    <glob pattern="*.rdp"/>
    <glob pattern="*.RDP"/>
  </mime-type>
</mime-info>
```

Встановити в Flatpak manifest:
```yaml
- install -Dm644 assets/io.github.totoshko88.RustConn-rdp.xml
    /app/share/mime/packages/io.github.totoshko88.RustConn-rdp.xml
```

**Sidebar stretching з довгими іменами:**
- `rustconn/src/sidebar/view.rs` — Label для назви з'єднання не має `ellipsize`
  та `max_width_chars`. Довга назва розтягує sidebar.

```rust
// sidebar/view.rs — додати до label:
label.set_ellipsize(pango::EllipsizeMode::End);
label.set_max_width_chars(35);
```

**Критична оцінка:** MIME type XML — стандартний підхід freedesktop, працює
надійно. Потрібно додати файл в `build.rs` або Flatpak manifest для
автоматичної інсталяції. `update-mime-database` потрібен тільки для native
packages — Flatpak робить це автоматично через `finish-args`.
Sidebar ellipsize — тривіальне виправлення, без ризиків.

**Пріоритет:** P1

---

### 4.2 Issue #62 — picocom не знайдено у Flatpak

**Аналіз:** picocom **присутній** у всіх трьох Flatpak маніфестах:
- `packaging/flatpak/io.github.totoshko88.RustConn.yml:97`
- `packaging/flatpak/io.github.totoshko88.RustConn.local.yml:97`
- `packaging/flathub/io.github.totoshko88.RustConn.yml:110`

Збірка: `make && install -Dm755 picocom /app/bin/picocom`

**Можливі причини:**
1. Збірка picocom може тихо провалитись (відсутні build deps в sandbox)
2. Користувач може мати стару версію Flatpak де picocom ще не було додано
3. `detect_picocom()` використовує `--help` flag — picocom може повертати
   non-zero exit code для `--help`

**Пропозиція:**
```rust
// detection.rs — picocom --help повертає exit code 1, використати fallback:
pub fn detect_picocom() -> ClientInfo {
    if let Some(info) = try_detect_client("picocom", "picocom", &["--help"]) {
        return info;
    }
    // Fallback: перевірити чи бінарник існує без запуску
    if which_binary("picocom").is_some() {
        return ClientInfo {
            name: "picocom".to_string(),
            installed: true,
            version: None,
            path: which_binary("picocom"),
        };
    }
    ClientInfo::not_installed("picocom", "Install picocom package")
}
```

**Відповідь користувачу:** Попросити перевірити версію Flatpak (`flatpak info
io.github.totoshko88.RustConn`) і оновити до 0.10.1. Якщо проблема залишається —
запустити `flatpak run --command=picocom io.github.totoshko88.RustConn --help`
для діагностики.

**Критична оцінка:** Найімовірніша причина — `picocom --help` повертає non-zero
exit code (picocom 3.x повертає 1 для `--help`). Fallback через `which_binary`
— надійний. Але потрібно перевірити чи `detect_picocom()` взагалі існує як
окрема функція, чи picocom детектується через загальний `detect_client()`.
Якщо загальний — потрібно додати picocom-specific fallback.

**Пріоритет:** P1

---

### 4.3 Issue #61 — RDP не підключається (ironrdp-tokio panic)

**Аналіз:** Баг `copy_from_slice` в `ironrdp-tokio 0.8.0` **вже оброблений**
через `catch_unwind` (`rdp_client/client/connection.rs:195`). Panic
перехоплюється і конвертується в `RdpClientError::ConnectionFailed`.

**Чому користувач бачить "indefinite connection":**
1. Handshake timeout = `timeout_secs * 2` (мінімум 60с). Під час TLS/NLA фази
   UI показує "Connecting..." — це виглядає як зависання.
2. Після IronRDP failure — fallback на FreeRDP. Якщо FreeRDP не встановлений,
   з'єднання тихо провалюється без чіткого повідомлення.
3. Panic відбувається в KDC TCP response parser — тільки коли сервер використовує
   Kerberos (AD-joined Windows). Для простих NLA з'єднань panic не виникає.

**Пропозиція:**
```rust
// embedded_rdp/connection.rs — додати toast при fallback:
Err(e) => {
    let reason = format!("IronRDP connection failed: {e}");
    self.report_fallback(&reason);
    // Показати toast користувачу:
    if let Some(overlay) = &*self.toast_overlay.borrow() {
        overlay.show_warning(&format!(
            "Native RDP failed, trying FreeRDP... ({})",
            e.to_string().chars().take(80).collect::<String>()
        ));
    }
    self.cleanup_embedded_mode();
}

// Якщо FreeRDP теж не знайдено — чітке повідомлення:
// "RDP connection failed. Install FreeRDP 3.x for external mode."
```

**Відповідь користувачу:** Встановити FreeRDP 3.x (`wlfreerdp3` або `xfreerdp3`).
В Flatpak FreeRDP вже вбудований. Для deb — `sudo apt install freerdp3-x11` або
`freerdp3-wayland`.

**Критична оцінка:** Toast при fallback — правильний підхід, не ламає існуючу
логіку. Потрібно перевірити чи `toast_overlay` доступний в контексті
`embedded_rdp/connection.rs` (може бути `None` якщо embedded mode не
ініціалізований). Зменшення handshake timeout з 60с до 15-20с теж варто
розглянути — 60с занадто довго для UX.

**Пріоритет:** P1

---

## 5. МЕРТВИЙ КОД ТА НЕДОРОБКИ

### 5.1 MOSH: `--ssh` аргумент — аналіз квотування

**Файл:** `rustconn-core/src/protocol/mosh.rs:61`

```rust
cmd.push(format!("--ssh=ssh -p {ssh_port}"));
```

**Аналіз:** VTE `spawn_async()` передає argv напряму через `exec` (без shell).
Кожен елемент `Vec<String>` стає окремим OS-аргументом. Тобто mosh отримує
`--ssh=ssh -p 2222` як **один** аргумент. Mosh парсить `=` і отримує значення
`ssh -p 2222`, яке потім передає в shell для запуску SSH. Shell коректно
розбиває `ssh -p 2222` на `ssh`, `-p`, `2222`.

**Критична оцінка:** Поточний код **коректний** для exec-based spawn (VTE).
Додавання лапок (`--ssh=\"ssh -p 2222\"`) було б **помилкою** — mosh отримав
би значення `"ssh -p 2222"` з лапками, і shell спробував би запустити бінарник
з назвою `"ssh` (з лапкою).

Проблема може виникнути тільки якщо mosh запускається через shell wrapper
(`sh -c "mosh --ssh=ssh -p 2222 user@host"`) — тоді shell розіб'є аргументи
неправильно. Але в RustConn mosh запускається через `spawn_command()` →
`spawn_async()` (exec), тому квотування не потрібне.

**Однак:** Це питання стає неактуальним через баг 6.1b — mosh dispatch
відсутній і з'єднання взагалі не створюються. Після виправлення 6.1b
потрібно протестувати `--ssh` з нестандартним портом.

**Пріоритет:** P3 — код коректний, потрібна лише верифікація після 6.1b.

---

### 5.2 Smart Folders: немає автооновлення при зміні з'єднань

**Проблема:** `SmartFoldersSidebar::update()` викликається вручну. Коли
користувач додає/видаляє/редагує з'єднання, Smart Folders sidebar не
оновлюється автоматично — потрібно перезапустити додаток або вручну тригернути
оновлення.

**Пропозиція:** Підписатись на зміни в `ConnectionManager` і викликати
`smart_folders_sidebar.update()` після кожної зміни з'єднань.

**Критична оцінка:** Потрібно знайти правильну точку підписки. Якщо
`ConnectionManager` не має signal/callback механізму, потрібно додати його
або використати існуючий broadcast channel. Ризик — часті оновлення при
масових операціях (import 100 з'єднань). Потрібен debounce (100-200ms).

**Пріоритет:** P2

---

### 5.3 Ad-hoc Broadcast: немає візуального зворотного зв'язку

**Проблема:** `BroadcastController` (`rustconn/src/broadcast.rs`) — чиста логіка
без UI інтеграції для індикації активного broadcast. Користувач не бачить:
- Які термінали обрані для broadcast
- Чи активний broadcast mode
- Скільки терміналів отримують input

**Пропозиція:** Додати CSS клас `broadcast-active` до tab header обраних
терміналів, показати badge з кількістю обраних терміналів на toolbar кнопці.

**Критична оцінка:** CSS клас — простий і надійний підхід. Badge на toolbar
кнопці потребує `adw::ButtonContent` або overlay. Потрібно перевірити чи
`BroadcastController` має доступ до tab widgets для додавання CSS класу.

**Пріоритет:** P2

---

### 5.4 CSV Import: GUI не дозволяє обрати delimiter

**Проблема:** `handle_csv_file_import()` (`dialogs/import.rs:2005`) створює
`CsvImporter::new()` з default options (comma delimiter). GUI не надає
можливості обрати delimiter (semicolon, tab) хоча `CsvParseOptions` це
підтримує.

**Пропозиція:** Додати DropDown з варіантами delimiter перед кнопкою імпорту.

**Критична оцінка:** Тривіальне UI доповнення. `CsvParseOptions` вже підтримує
різні delimiters — потрібно лише прокинути вибір з GUI. Низький ризик.

**Пріоритет:** P3

---

### 5.5 Script Credentials: кнопка "Test Script" — немає feedback

**Проблема:** Кнопка "Test Script" в діалозі з'єднання (`general_tab.rs:294`)
створена, але потрібно перевірити чи підключений handler що показує результат
тесту (success/error toast).

**Критична оцінка:** Потрібна верифікація — можливо handler вже є. Якщо ні —
додати toast з результатом виконання скрипта. Низький ризик.

**Пріоритет:** P3

---

### 5.6 Per-connection Terminal Theming: немає preview

**Проблема:** `ColorDialogButton` widgets в Advanced tab дозволяють обрати
кольори, але немає live preview — користувач бачить результат тільки після
підключення.

**Пропозиція:** Додати маленький `DrawingArea` preview що показує обрані кольори
(вже є `DrawingArea` в return type `create_advanced_tab()`).

**Критична оцінка:** Nice-to-have. `DrawingArea` з кольоровими прямокутниками —
простий підхід. Але live preview з реальним текстом (як в terminal emulator
settings) значно складніший і потребує mock VTE widget.

**Пріоритет:** P3

---

## 6. ПРОПОЗИЦІЇ ПОКРАЩЕННЯ

### 6.1 ~~Recording: перейти на вбудований recorder замість `script`~~ — ВІДХИЛЕНО

> **Ця пропозиція некоректна і видалена з роадмапу.**
>
> `connect_commit()` перехоплює лише **INPUT** (натискання клавіш → PTY), а не
> **OUTPUT** (PTY → екран). Для `scriptreplay`-сумісного відтворення потрібен
> output stream. `script` — єдиний надійний спосіб записати output в форматі,
> сумісному з `scriptreplay`. Підхід через `connect_commit()` був випробуваний
> і відхилений на етапі проєктування.
>
> Покращення запису реалізуються в рамках `script`-підходу (див. секцію 2).

---

### 6.1b MOSH: відсутній dispatch в `start_connection()` — з'єднання не працюють

**Проблема:** `get_protocol_string()` повертає `"mosh"` для Mosh з'єднань
(`window/types.rs:76`), але в `start_connection()` (`window/mod.rs:2636`)
**немає гілки `"mosh"`** в match. Mosh потрапляє в `_ => None` — з'єднання
тихо не створюється.

`MoshProtocol::build_command()` (`protocol/mosh.rs:54`) коректно генерує
argv (`["mosh", "--ssh=ssh -p 2222", "user@host"]`), але цей код ніколи
не викликається з connection flow.

**Файли для виправлення:**
- `rustconn/src/window/protocols.rs` — додати `start_mosh_connection()`
- `rustconn/src/window/mod.rs` — додати `"mosh"` гілку в dispatch

**Пропозиція:**
```rust
// window/protocols.rs:
pub fn start_mosh_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    let protocol = MoshProtocol::new();
    let command = protocol.build_command(conn)?;
    let session_id = notebook.create_terminal_tab(/* ... */);
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    notebook.spawn_command(session_id, &argv, None, None);
    // Setup logging if enabled
    Some(session_id)
}

// window/mod.rs — в match:
"mosh" => protocols::start_mosh_connection(
    state, notebook, sidebar, connection_id, &conn_clone, logging_enabled,
),
```

**Критична оцінка:** Виправлення просте — аналогічно `start_serial_connection()`
яка теж використовує `build_command()` → `spawn_command()`. Потрібно також
додати port check (mosh використовує SSH port для початкового handshake).

**Пріоритет:** P0 — протокол повністю не працює.

---

### 6.2 Sidebar: ellipsize для довгих назв

Жоден Label в sidebar (`sidebar/view.rs`) не має `ellipsize`. Довгі назви
з'єднань, груп, або імпортованих .rdp файлів розтягують sidebar.

**Критична оцінка:** Дублює 4.1b. Реалізувати разом — один PR.

**Пріоритет:** P1

---

### 6.3 RDP: прогрес-індикатор фаз підключення

Замість generic "Connecting..." показувати фазу: TCP → TLS → NLA → Session.
Це зменшить відчуття "зависання" при повільних серверах.

**Критична оцінка:** IronRDP має callback-based API з фазами підключення.
Потрібно перевірити чи `ironrdp-tokio` надає progress callbacks. Якщо ні —
можна показувати хоча б "TCP connected" після успішного TCP handshake і
"Authenticating..." після TLS. Середня складність.

**Пріоритет:** P2

---

### 6.4 Config sync: вбудована підтримка

Розглянути додавання CLI команди `rustconn-cli config sync` що:
- Експортує конфігурацію в Git repo
- Імпортує з Git repo з merge strategy
- Підтримує `.gitignore` для history/trash

**Критична оцінка:** Висока складність — потрібна інтеграція з git2 або
виклик git CLI. Merge conflicts для TOML файлів складні для автоматичного
вирішення. Рекомендація: почати з документації (3.2) і розглянути CLI sync
в наступному релізі (0.11.x).

**Пріоритет:** P3

---

## 7. ЗВЕДЕНА ТАБЛИЦЯ ПРІОРИТЕТІВ

| # | Опис | Пріоритет | Складність | Статус |
|---|------|-----------|------------|--------|
| 1.1 | session_recording_enabled не працює | P0 | Низька | ✅ Реалізовано |
| 1.2 | highlight_rules не застосовуються | P0 | Низька | ✅ Реалізовано |
| 6.1b | MOSH dispatch відсутній | P0 | Низька | ✅ Реалізовано |
| 2.1 | script команда видна при записі | P1 | Середня | ✅ Реалізовано (затримка 100ms) |
| 2.2 | Подвійний exit при зупинці | P1 | Середня | ✅ Реалізовано (Ctrl+D + async SCP) |
| 2.3 | Втрата команд при відтворенні | P1 | Середня | ✅ Реалізовано (strip_script_command_echo) |
| 4.1 | .rdp file association (#64) | P1 | Низька | ✅ Реалізовано (MIME XML + всі пакети) |
| 4.1b | Sidebar stretching (#64) | P1 | Низька | ✅ Реалізовано (ellipsize + max_width_chars) |
| 4.2 | picocom у Flatpak (#62) | P1 | Низька | ✅ Реалізовано (which_binary fallback) |
| 4.3 | RDP indefinite connect (#61) | P1 | Середня | ✅ Реалізовано (user-friendly error msg) |
| 3.2 | Config sync документація | P2 | Низька | ✅ Реалізовано (USER_GUIDE.md) |
| 5.4 | CSV delimiter auto-detect | P3 | Низька | ✅ Реалізовано (auto-detect з вмісту) |
| 5.2 | Smart Folders auto-refresh | P2 | Середня | ⏳ Відкладено (віджет не інтегрований) |
| 5.3 | Broadcast visual feedback | P2 | Середня | ⏳ Відкладено (UI wiring не готовий) |
| 6.3 | RDP progress indicator | P2 | Середня | ⏳ Відкладено (потребує IronRDP API) |
| 5.1 | MOSH --ssh квотування (коректне) | P3 | — | ✅ Верифіковано як коректне |
| 5.5 | Script credentials test feedback | P3 | Низька | ⏳ Відкладено |
| 5.6 | Theme preview | P3 | Низька | ⏳ Відкладено |
| 6.4 | Config sync CLI | P3 | Висока | ⏳ Відкладено (0.11.x) |
| ~~6.1~~ | ~~Перехід на connect_commit()~~ | ~~ВІДХИЛЕНО~~ | — | — |
