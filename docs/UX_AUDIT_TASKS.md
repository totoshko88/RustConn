# RustConn UX Audit — Задачі для покращення

**Дата аудиту:** 2026-02-28
**Версія:** 0.9.3
**Аудитор:** Lead UX

---

## 1. Обробка помилок та повідомлення

### 1.1 Уніфікація візуального розрізнення типів повідомлень
**Пріоритет:** Високий
**Файли:** `rustconn/src/alert.rs`, `rustconn/src/toast.rs`

- `show_error()` та `show_success()` в `alert.rs` викликають ідентичний `show_alert()` — немає візуального розрізнення між успіхом та помилкою. Додати іконки або CSS-стилі для розрізнення.
- Toast CSS-класи (`toast-info`, `toast-success`, `toast-warning`, `toast-error`) визначені в `ToastType::css_class()`, але не застосовуються в `show_toast_with_type()` — тости не отримують CSS-клас. Додати `toast.add_css_class(toast_type.css_class())`.
- Перевірити наявність відповідних CSS-правил для цих класів у стилях додатку.

### 1.2 Втрата повідомлень при відсутності ToastOverlay
**Пріоритет:** Високий
**Файли:** `rustconn/src/toast.rs`

- `show_toast_on_window()` при неможливості знайти `ToastOverlay` лише логує через `tracing::warn` — повідомлення для користувача втрачається. Додати fallback через `adw::AlertDialog` або гарантувати наявність overlay у всіх вікнах.

### 1.3 Стандартизація поверхні помилок
**Пріоритет:** Середній

Встановити чіткі правила (задокументувати в steering):
| Ситуація | Поверхня |
|----------|----------|
| Транзієнтна помилка (timeout, auth fail) | `adw::Toast` (high priority) |
| Потребує рішення користувача | `adw::AlertDialog` |
| Помилка валідації форми | Inline label + `show_validation_error()` |
| Фонова операція | Тільки `tracing` |

### 1.4 Відсутність підказок для відновлення
**Пріоритет:** Середній

Помилки підключення не містять actionable підказок. Додати контекстні рекомендації:
- Auth failed → "Перевірте облікові дані або SSH-ключ"
- Connection refused → "Перевірте що хост доступний та порт відкритий"
- Client not found → "Встановіть {client} або перевірте PATH"
- Timeout → "Перевірте мережеве з'єднання або збільшіть timeout"

### 1.5 WoL діалог — неконсистентний feedback
**Пріоритет:** Низький
**Файли:** `rustconn/src/dialogs/wol.rs`

- Статус відображається через `Label` з CSS-класами, а не через Toast як в решті додатку. Розглянути уніфікацію або залишити як є (діалог має свій контекст).

---

## 2. CRUD повнота

### 2.1 Історія підключень — неповний CRUD
**Пріоритет:** Середній
**Файли:** `rustconn/src/dialogs/history.rs`

- Є: Read (список), Clear All, Connect from history
- Немає: Видалення окремих записів, пошук/фільтрація в історії, експорт історії
- Додати: кнопку видалення на кожному рядку, поле пошуку

### 2.2 Сесії — відсутні bulk-операції
**Пріоритет:** Середній

- Немає: "Close All Tabs", "Close Other Tabs", "Close Tabs to the Right"
- Немає: Експорт сесійного логу з UI (тільки файли на диску)
- Додати контекстне меню на табах з цими операціями

### 2.3 Snippets — відсутній експорт/імпорт
**Пріоритет:** Низький
**Файли:** `rustconn/src/dialogs/snippet.rs`

- Snippets не можна експортувати/імпортувати окремо від повного бекапу
- Немає історії виконання snippet
- Розглянути додавання export/import для snippets

### 2.4 Bulk-операції для підключень
**Пріоритет:** Середній
**Файли:** `rustconn/src/sidebar/mod.rs`

- Sidebar має `bulk_actions_bar`, `select_all()`, `clear_selection()`, `get_selected_ids()` — інфраструктура є
- Перевірити що bulk delete, bulk move to group, bulk connect повністю працюють
- Додати bulk edit (зміна групи, тегів, протоколу для вибраних)

---

## 3. Стандартизація діалогів

### 3.1 Тип діалогового вікна
**Пріоритет:** Середній

Змішане використання типів вікон:
| Діалог | Поточний тип | Рекомендація |
|--------|-------------|--------------|
| ConnectionDialog | `adw::Window` | ✅ OK |
| SettingsDialog | `adw::PreferencesDialog` | ✅ OK |
| HistoryDialog | `adw::Dialog` | ✅ OK (lightweight) |
| CommandPaletteDialog | `adw::Dialog` | ✅ OK (overlay) |
| FlatpakComponentsDialog | `adw::Dialog` | ✅ OK |
| ExportDialog | `adw::Window` | ✅ OK (complex) |
| ImportDialog | `adw::Window` | ✅ OK (complex) |
| ClusterDialog | `adw::Window` | Розглянути `adw::Dialog` |
| SnippetDialog | `adw::Window` | Розглянути `adw::Dialog` |
| VariablesDialog | `adw::Window` | Розглянути `adw::Dialog` |
| WolDialog | `adw::Window` | Розглянути `adw::Dialog` |
| TemplateDialog | `adw::Window` | ✅ OK (complex) |

Рекомендація: прості діалоги (Cluster, Snippet, Variables, WoL) перевести на `adw::Dialog` для кращої інтеграції з GNOME HIG (sheet-style presentation).

### 3.2 Кнопки Cancel/Close
**Пріоритет:** Низький

- Діалоги з дією збереження: "Cancel" / "Save" або "Cancel" / "Create" ✅
- Діалоги тільки для перегляду: "Close" ✅
- WoL: "Cancel" / "Send" ✅
- Перевірити консистентність по всіх діалогах

### 3.3 Використання adw::PreferencesGroup
**Пріоритет:** Низький

- WoL діалог використовує `adw::PreferencesGroup` + `adw::EntryRow` — сучасний підхід ✅
- Snippet діалог використовує `Entry` + `Grid` — старіший підхід
- Variables діалог використовує `Entry` + `Grid` — старіший підхід
- Розглянути міграцію Snippet та Variables на `adw::PreferencesGroup` / `adw::EntryRow`

---

## 4. Протоколи — повнота та консистентність

### 4.1 SPICE — відсутній embedded клієнт
**Пріоритет:** Низький (feature-gated)

- SPICE працює тільки через зовнішній `remote-viewer`
- Feature flag `spice-embedded` існує але не реалізований
- Задокументувати обмеження в User Guide

### 4.2 Kubernetes — відсутній port forwarding
**Пріоритет:** Низький

- `kubectl port-forward` не інтегрований
- Розглянути додавання як окремий тип K8s-з'єднання

### 4.3 Консистентність валідації протоколів
**Пріоритет:** Середній
**Файли:** `rustconn/src/validation.rs`, `rustconn/src/dialogs/connection/dialog.rs`

- `ConnectionDialogData::validate()` перевіряє базові поля (name, host)
- Перевірити що кожен протокол має специфічну валідацію:
  - SSH: host required, port valid
  - RDP: host required, port valid
  - VNC: host required, port valid
  - Serial: device path required, baud rate valid
  - K8s: pod name required (exec mode), namespace valid
  - SPICE: host required
  - Telnet: host required
- Inline validation (`setup_inline_validation_for`) підключена — перевірити покриття всіх полів

---

## 5. Secret Management — покращення

### 5.1 Відсутність попереджень про ротацію/закінчення
**Пріоритет:** Низький

- Немає механізму відстеження віку credentials
- Розглянути: опціональне поле "expires at" для credentials з нагадуванням

### 5.2 Аудит доступу до credentials
**Пріоритет:** Низький

- Немає логу хто/коли отримував доступ до credentials
- Розглянути: `tracing::info!` при кожному resolve credentials (вже частково є)

---

## 6. User Guide — оновлення та розширення

### 6.1 Оновити версію
**Пріоритет:** Високий
**Файли:** `docs/USER_GUIDE.md`

- Поточна версія в документі: 0.9.3 — оновити до актуальної

### 6.2 Додати відсутні розділи
**Пріоритет:** Високий

Наступні функції реалізовані але не задокументовані або задокументовані недостатньо:

1. **Encrypted Documents** — повний розділ (New, Open, Save, Close, Protection, шифрування AES-256-GCM)
2. **RDM Import** — формат Remote Desktop Manager (JSON) не згаданий в розділі Import
3. **Virt-Viewer Import** — формат .vv файлів не згаданий в розділі Import
4. **Security-Key/FIDO2** — метод автентифікації згаданий побіжно, потрібен окремий підрозділ з вимогами (libfido2, hardware key)
5. **Custom Properties** — згадані в Connection tabs але без прикладів використання
6. **Automation (Expect Rules)** — потрібні детальні приклади (auto-login, sudo password, menu navigation)
7. **Tasks (Pre/Post Connection)** — потрібні приклади (VPN connect before, cleanup after)
8. **Connection Testing** — розділ Test Connection дуже короткий, додати деталі про port check

### 6.3 Розширити існуючі розділи
**Пріоритет:** Середній

1. **Import** — додати RDM та Virt-Viewer формати, описати Import Preview та Merge Strategy
2. **Export** — додати опцію "Export selected only", описати batch export для великих колекцій
3. **Clusters** — додати приклади використання (deploy to all, monitoring check)
4. **Templates** — додати приклади створення та використання з Quick Connect
5. **Troubleshooting** — додати розділи:
   - Serial device access issues
   - Kubernetes connection issues
   - Remote Monitoring not showing data
   - Waypipe not working
   - Import failures (encoding, format mismatch)

### 6.4 Додати розділ Adaptive UI
**Пріоритет:** Низький

- Breakpoints задокументовані коротко — розширити з скріншотами/описами поведінки
- Мінімальні розміри діалогів
- Поведінка sidebar на вузьких екранах

---

## 7. GNOME HIG відповідність

### 7.1 Alert діалоги — візуальне розрізнення
**Пріоритет:** Середній
**Файли:** `rustconn/src/alert.rs`

- `show_error()` та `show_success()` ідентичні — HIG рекомендує різні іконки/стилі
- Рекомендація: `show_error()` → heading з іконкою помилки, `show_success()` → heading з іконкою успіху
- Або видалити `show_success()` і використовувати Toast для успішних операцій (HIG pattern)

### 7.2 Password Generator — lifecycle діалогу
**Пріоритет:** Низький
**Файли:** `rustconn/src/dialogs/password_generator.rs`

- Реалізований як функція `show_password_generator_dialog()`, а не як struct
- Перевірити що діалог коректно обробляє Escape, має proper transient parent

### 7.3 Accessibility
**Пріоритет:** Середній

- Перевірити `tooltip-text` на всіх інтерактивних елементах sidebar toolbar
- Перевірити `accessible-role` на custom widgets (ConnectionItem, split view panels)
- Перевірити tab order в складних діалогах (Connection dialog з 10+ табами)
- Перевірити що всі іконки-кнопки мають accessible label

---

## 8. Flatpak-специфічні покращення

### 8.1 CLI Components — версіонування
**Пріоритет:** Низький
**Файли:** `rustconn/src/dialogs/flatpak_components.rs`

- Немає можливості вибрати конкретну версію CLI при встановленні
- Немає rollback при невдалому оновленні
- Розглянути: показувати встановлену версію та доступну версію

### 8.2 Offline режим
**Пріоритет:** Низький

- Flatpak Components потребує інтернет для завантаження
- Розглянути: кешування завантажених пакетів, offline installation з файлу

---

## 9. Нові функції (пропозиції)

### 9.1 Settings Export/Import
**Пріоритет:** Середній

- Немає можливості бекапу/відновлення налаштувань окремо від підключень
- Додати: Menu → Tools → Export Settings / Import Settings
- Формат: JSON або TOML

### 9.2 Connection Pinning / Favorites
**Пріоритет:** Реалізовано ✅

- Favorites задокументовані в User Guide та реалізовані (right-click → Pin to Favorites)
- Перевірити що функціонал повністю працює

### 9.3 Passphrase Generation
**Пріоритет:** Низький
**Файли:** `rustconn/src/dialogs/password_generator.rs`, `rustconn-core/src/password_generator.rs`

- Password Generator генерує тільки випадкові символи
- Додати режим passphrase (EFF wordlist, configurable word count, separator)

### 9.4 Tab Context Menu — розширення
**Пріоритет:** Середній

Додати в контекстне меню табів:
- "Close Other Tabs"
- "Close Tabs to the Right"
- "Close All Tabs"
- "Duplicate Tab" (reconnect)
- "Move to Split Pane"

### 9.5 Import Preview UI
**Пріоритет:** Середній
**Файли:** `rustconn-core/src/import/preview.rs`, `rustconn/src/dialogs/import.rs`

- `ImportPreview` та `MergeStrategy` реалізовані в core
- Перевірити що UI діалог імпорту показує preview перед застосуванням
- Якщо ні — додати крок preview з можливістю вибору дій для кожного з'єднання

### 9.6 Per-Connection Terminal Settings
**Пріоритет:** Низький

- Всі terminal settings глобальні
- Розглянути: override font, color theme, scrollback per connection

---

## 10. Консистентність i18n

### 10.1 Перевірка покриття
**Пріоритет:** Середній

- Перевірити що ВСІ user-visible strings обгорнуті в `i18n()` / `i18n_f()`
- Особлива увага: toast messages, validation errors, status labels
- WoL діалог: `"Magic packet sent to {mac}"` — використовує `format!()` замість `i18n_f()`
- Перевірити `po/POTFILES.in` на повноту

### 10.2 Формат повідомлень
**Пріоритет:** Низький

- Перевірити що немає конкатенації перекладених фрагментів
- Всі динамічні значення через positional placeholders

---

## Підсумок пріоритетів

| Пріоритет | Кількість задач | Ключові |
|-----------|----------------|---------|
| Високий | 4 | Error visual distinction, toast CSS, User Guide version, User Guide missing sections |
| Середній | 12 | Error recovery hints, history CRUD, bulk ops, dialog standardization, protocol validation, accessibility, import preview, i18n coverage |
| Низький | 10 | WoL feedback, snippet export, SPICE embedded, K8s port-forward, credential rotation, passphrase gen, Flatpak versioning |
