# UX Improvements Roadmap

## Версія 0.13.14 — Quick Wins (hotfix)

Мінімальні зміни, які покращують UX без архітектурних змін.
Кожна — до 30 хвилин роботи.

---

### 1. Import кнопка на Welcome page

**Проблема:** Welcome page пропонує «New Connection» та «Quick Connect», але не «Import».
Для користувачів, які мігрують з PuTTY, Remmina, mRemoteNG, SecureCRT — імпорт є першою дією.

**Де:** `rustconn/src/split_view/bridge.rs` → `create_welcome_content()`

**Реалізація:**
```rust
let import_btn = gtk4::Button::builder()
    .label(&i18n("Import Connections"))
    .css_classes(["pill"])
    .action_name("win.import")
    .build();
actions.append(&import_btn);
```

**Складність:** ~5 рядків коду.

---

### 2. Empty state для Template Manager

**Проблема:** `TemplateManagerDialog` показує порожній `ListBox` без пояснення.
Recordings та History вже мають `adw::StatusPage` placeholder — Templates ні.

**Де:** `rustconn/src/dialogs/template.rs` → `TemplateManagerDialog::new()`

**Реалізація:**
```rust
let placeholder = adw::StatusPage::builder()
    .icon_name("document-page-setup-symbolic")
    .title(i18n("No Templates"))
    .description(i18n("Create a template to quickly set up new connections with predefined settings."))
    .build();
templates_list.set_placeholder(Some(&placeholder));
```

**Складність:** ~5 рядків коду.

---

### 3. Reconnect countdown у банері

**Проблема:** При auto-reconnect банер показує лише «Session disconnected» + кнопку.
Користувач не знає, що auto-reconnect працює у фоні та коли буде наступна спроба.

**Де:** `rustconn/src/terminal/mod.rs` → `show_reconnect_overlay()`

**Реалізація:**
- Додати `Label` з текстом «Reconnecting in Xs...» поруч з існуючим label
- Оновлювати через `glib::timeout_add_local_once` кожну секунду
- При ручному натисканні «Reconnect» — скасувати countdown
- Якщо auto-reconnect вимкнено (`retry_config.enabled == false`) — не показувати countdown

**Складність:** ~40 рядків коду. Потрібно передати `RetryConfig` в overlay.

---

### 4. Session restore preview

**Проблема:** Prompt «Restore sessions?» не показує, які саме сесії будуть відновлені.
Користувач натискає «Yes» наосліп.

**Де:** `rustconn/src/app.rs` або `rustconn/src/window/sessions.rs` — де показується restore prompt.

**Реалізація:**
- В body `adw::AlertDialog` додати список з'єднань:
  ```
  • SSH: production-server
  • RDP: windows-desktop
  • SSH: staging-db
  ```
- Формат: `{protocol}: {connection_name}` для кожної збереженої сесії

**Складність:** ~15 рядків коду (форматування body string).

---

## Версія 0.14.0 — UX Overhaul

Структурні покращення, які потребують нових компонентів або значних змін.

---

### 5. Connection Wizard (покроковий майстер нового з'єднання)

**Проблема:**
`ConnectionDialog` має 250+ полів у структурі. Для нового користувача це overwhelming —
відкривається величезна форма з вкладками SSH, RDP, VNC, SPICE, Serial, Kubernetes,
Zero Trust, кожна з десятками опцій. Більшість користувачів потребують лише:
протокол → хост → порт → username → пароль.

**Обґрунтування (GNOME HIG):**
GNOME HIG рекомендує assistant/wizard pattern для задач з чіткою послідовністю кроків,
де кожен крок залежить від попереднього (див. GNOME Initial Setup, GNOME Boxes first-run).
Повний діалог залишається для досвідчених користувачів через «Edit» або «Advanced».

**Архітектура:**

```
┌─────────────────────────────────────────┐
│  Step 1: Protocol                       │
│  ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐     │
│  │ SSH │ │ RDP │ │ VNC │ │ ... │     │
│  └─────┘ └─────┘ └─────┘ └─────┘     │
│                              [Next →]   │
├─────────────────────────────────────────┤
│  Step 2: Connection                     │
│  Name:     [___________________]        │
│  Host:     [___________________]        │
│  Port:     [22___]                      │
│  Username: [___________________]        │
│                              [Next →]   │
├─────────────────────────────────────────┤
│  Step 3: Authentication                 │
│  ○ Password  ○ Key file  ○ Agent       │
│  Password: [___________________]        │
│                            [Connect]    │
└─────────────────────────────────────────┘
```

**Де:** Новий файл `rustconn/src/dialogs/connection_wizard.rs`

**Компоненти:**
- `adw::NavigationView` з 3 сторінками (`adw::NavigationPage`)
- Крок 1: Grid з кнопками протоколів (іконка + назва), як у GNOME Boxes
- Крок 2: `adw::PreferencesGroup` з EntryRow для Name, Host, Port, Username
- Крок 3: Залежить від протоколу — для SSH: password/key/agent; для RDP: password + domain
- Кнопка «Advanced...» на кожному кроці → відкриває повний `ConnectionDialog` з заповненими полями
- Після «Connect» — зберігає з'єднання + одразу підключається

**Валідація:**
- Крок 2: Host не порожній, Port в діапазоні 1-65535 (real-time, як вже є в ConnectionDialog)
- «Next» неактивна поки обов'язкові поля порожні

**Інтеграція:**
- `win.new-connection` → відкриває Wizard замість повного діалогу
- `win.new-connection-advanced` → відкриває повний діалог (для меню та power users)
- Ctrl+N → Wizard
- Контекстне меню «Edit» → завжди повний діалог

**Складність:** Висока. ~500-700 рядків нового коду. Потребує рефакторингу action routing.

---

### 6. Quick Connect — історія та автодоповнення

**Проблема:**
Quick Connect не зберігає попередні з'єднання. Sidebar search має history popover,
але Quick Connect — ні. Користувач, який часто підключається до тимчасових серверів,
змушений вводити адресу щоразу.

**Де:** `rustconn/src/window/edit_dialogs.rs` → `show_quick_connect_dialog_with_state()`

**Реалізація:**
1. Додати поле `quick_connect_history: Vec<QuickConnectEntry>` в `AppSettings`
   ```rust
   struct QuickConnectEntry {
       host: String,
       port: u16,
       protocol: String,  // "ssh", "rdp", "telnet"
       username: Option<String>,
       last_used: chrono::DateTime<chrono::Utc>,
   }
   ```
2. Зберігати останні 10 quick-з'єднань при успішному підключенні
3. В діалозі Quick Connect додати `gtk4::ListBox` під полем хоста з історією
4. При введенні тексту — фільтрувати історію (fuzzy match по host)
5. Клік по елементу історії — заповнює всі поля

**Автодоповнення хостів:**
- Збирати хости з існуючих з'єднань (`connections.iter().map(|c| &c.host)`)
- Показувати як suggestions під полем хоста
- Використати custom popover (EntryCompletion deprecated в GTK4)

**Складність:** Середня. ~150-200 рядків + зміни в config serialization.

---

### 7. Tab Grouping UI

**Проблема:**
`TabGroupManager` існує в коді (кольорові індикатори на вкладках), але немає UI
для ручного створення та призначення груп. Користувач з 10+ відкритими вкладками
не може візуально розділити «Production» від «Staging» або «Project A» від «Project B».

**Де:**
- Контекстне меню вкладки: `rustconn/src/terminal/mod.rs`
- Tab group manager: вже існує

**Реалізація:**
1. Додати контекстне меню на вкладці (right-click на TabBar):
   - «Assign to Group →» → підменю з існуючими групами + «New Group...»
   - «Remove from Group»
2. Діалог «New Tab Group»:
   - Поле назви (Entry)
   - Вибір кольору (з палітри split-view кольорів, які вже є)
3. Кольоровий індикатор на вкладці (вже підтримується через `TabGroupManager`)
4. Зберігати групи в session state (не в config — це runtime-only)

**GNOME HIG відповідність:**
- GNOME Web (Epiphany) має Tab Groups з кольоровими маркерами
- Firefox/Chrome мають аналогічну функціональність
- Патерн: кольорова смужка під/над вкладкою

**Складність:** Середня. ~200-300 рядків. Основна робота — контекстне меню TabView.

---

### 8. Highlight Rules — окрема секція в Settings

**Проблема:**
Terminal page в Settings об'єднує Terminal + Logging + Highlight Rules.
Це 3 логічно різні домени на одному скролі. Highlight Rules з динамічним списком
правил (кожне з regex, кольором, стилем) займає багато вертикального простору.

**Де:** `rustconn/src/dialogs/settings/mod.rs` → `SettingsDialog::new()`

**Варіанти:**
1. **ExpanderRow** — згорнути Highlight Rules в `adw::ExpanderRow` (мінімальна зміна)
2. **Окрема PreferencesPage** — «Highlighting» з іконкою `format-text-highlight-symbolic`
3. **Subpage** — `adw::PreferencesDialog` підтримує navigation до subpage

**Рекомендація:** Варіант 1 (ExpanderRow) — мінімальна зміна, не ламає існуючу навігацію.

**Складність:** Низька. ~20 рядків (обгортка в ExpanderRow).

---

## Пріоритизація 0.14.0

| Порядок | Пропозиція | Складність | Вплив на UX |
|---------|-----------|-----------|-------------|
| 1 | Connection Wizard | Висока | Критичний для onboarding |
| 2 | Quick Connect history | Середня | Щоденне використання |
| 3 | Tab Grouping UI | Середня | Power users з 10+ tabs |
| 4 | Highlight Rules separation | Низька | Чистота Settings |

---

## Принципи реалізації

- **GNOME HIG first** — кожне рішення перевіряти на відповідність HIG
- **Progressive disclosure** — прості дії прості, складні доступні
- **Не ламати існуюче** — Wizard доповнює, а не замінює повний діалог
- **i18n** — всі нові рядки через `i18n()` / `i18n_f()`
- **Accessibility** — tooltip + accessible label на кожному icon-only елементі
