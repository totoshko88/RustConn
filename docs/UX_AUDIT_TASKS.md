# RustConn UX Audit — Задачі для покращення

**Дата:** 2026-02-28 | **Версія:** 0.9.4 | **Аудитор:** Lead UX

**Принцип пріоритизації:** оцінка з позиції реального користувача connection manager —
частота сценарію × біль від відсутності × складність реалізації. Інженерний рефакторинг
без видимого впливу на UX позначений окремо.

---

## 1. Connection CRUD

### Поточний стан
Повний CRUD: діалог з 11+ вкладками, Trash з Undo, Test Connection, Pre-connect port check,
Quick Connect (Ctrl+K), Duplicate (Ctrl+D), Copy/Paste.

### Задачі

Bulk операції (Delete, Move to Group, Select All) вже реалізовані через Group Operations Mode
(кнопка `view-list-symbolic` в sidebar toolbar). Toolbar: `[New Group] [Move to Group...] [Select All] [Clear] [Delete]`.

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| C-01 | Low | **Clone with Edit** — при Ctrl+D відкривати діалог редагування копії | Зараз копія створюється мовчки. Корисно, але не блокує — можна зробити Ctrl+D → Ctrl+E |
| C-02 | Low | **Expand inline validation** — підключити валідатори з `validation.rs` до всіх полів діалогу | `setup_inline_validation_for` покриває name/host/port; решта валідується лише при Save |

---

## 2. Group Management

### Поточний стан
Ієрархічні групи, credentials inheritance, drag-drop, sorting.
Модель має description, icon, password_source — але UI для редагування обмежений inline rename + context menu.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| G-01 | High | **Recursive Group Delete з вибором** — при видаленні групи пропонувати "Move children to root" або "Delete all" | `delete_group_cascade` існує в `ConnectionManager`, але UI завжди робить move-to-root без запиту. Ризик втрати даних якщо користувач очікує каскадне видалення |
| G-02 | Medium | **Group Edit Dialog** — простий діалог (не tabbed) для name, description, icon, credentials | Поля `description` та `icon` є в моделі, але недоступні через UI. Не потрібен повноцінний tabbed dialog — достатньо одного `adw::PreferencesGroup` |
| G-03 | Low | **Group connection count** — показувати кількість з'єднань в tooltip групи | `count_connections_in_group` існує, не відображається. Мінімальний effort, корисна інформація |

---

## 3. Error Handling & User Messaging

### Поточний стан
Ієрархія помилок (`RustConnError` → domain-specific), Toast system з типами/пріоритетами/іконками,
Alert dialogs, structured logging через `tracing`.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| E-01 | High | **Actionable Error Toasts** — додати кнопку дії (Retry, Open Settings) до error toasts при з'єднанні | `ToastOverlay` підтримує actions. "Connection failed" + кнопка "Retry" — реальне покращення для найчастішого error сценарію |
| E-02 | Medium | **Standardize Toast Format** — уніфікувати: `"{Action} {object}. {Suggestion}"` | Різні частини коду: "Created successfully", "Connection 'X' created", "Failed to connect". Потрібен єдиний стиль |
| E-03 | Medium | **i18n audit для toast titles** — "Warning" та "Error" в `toast.rs` `custom_title()` не обгорнуті в `i18n()` | Пряме порушення i18n правил з product.md |
| E-04 | Low | **Error recovery hints** — додати subtitle до error toasts для типових проблем (SSH key not found, client missing) | Troubleshooting є в User Guide, але не в UI. Low — бо частота помилок у налаштованих з'єднаннях низька |

---

## 4. Settings

### Поточний стан
4 сторінки (Terminal, Interface, Secrets, Connection), `adw::PreferencesDialog` з пошуком.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| S-01 | Medium | **Settings Backup/Restore** — export/import settings у файл | Реальний сценарій — міграція на нову машину. Але workaround є: скопіювати `~/.config/rustconn/` або native export (.rcn). Тому Medium, не High |
| S-02 | Medium | **Per-Protocol Defaults** — default port, resolution, encoding per protocol | Hardcoded defaults (SSH=22, RDP=3389). Якщо команда використовує SSH на порту 2222 — потрібно змінювати кожне з'єднання |
| S-03 | Low | **Reset to Defaults per section** — кнопка скидання для кожної секції | Є для keybindings, немає для інших. Рідкісний сценарій |

---

## 5. Import/Export

### Поточний стан
9 import форматів, 7 export форматів, merge strategies, batch processing, import preview.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| IE-01 | Medium | **Selective Export** — checkbox list для вибору з'єднань/груп | Реальний сценарій: поділитися 10 з'єднаннями з колегою з 200. Але частота низька — export це разова операція |
| IE-02 | Medium | **Export format validation** — warning якщо з'єднання несумісні з форматом | Kubernetes → SSH Config export мовчки ігнорує несумісні з'єднання. Потрібен хоча б warning |
| IE-03 | Low | **Import per-item conflict resolution** — вибір дії для кожного конфлікту | `ImportPreview` підтримує per-item actions в core, але UI показує лише глобальну стратегію. Рідкісний сценарій |

---

## 6. Protocol Dialogs

### Поточний стан
Повне покриття: SSH (5 auth methods, forwarding, waypipe), RDP (resolution, audio, shared folders, HiDPI),
VNC, SPICE, Telnet, Serial (повні параметри), Kubernetes (busybox mode).

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| P-01 | Medium | **SSH Key path validation** — перевіряти існування файлу при виборі ключа | Помилка виявляється лише при з'єднанні. File chooser вже є, але ручне введення шляху не валідується |
| P-02 | Low | **Protocol option tooltips** — tooltip для складних опцій (proxy jump, waypipe, SPICE compression) | Корисно для нових користувачів, але досвідчені знають що це |
| P-03 | Low | **Serial device picker** — dropdown з доступними `/dev/tty*` пристроями | Зараз ручне введення. Зручно, але Flatpak sandbox ускладнює detection |

---

## 7. Session Management

### Поточний стан
VTE terminal tabs, embedded RDP/VNC/SPICE, split view, session restore, logging (3 modes),
terminal search з regex.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| SS-01 | High | **Session Reconnect** — кнопка "Reconnect" в disconnected tab | Найчастіший friction point: з'єднання обірвалось → потрібно знайти його в sidebar → double-click. Кнопка в tab вирішує це одним кліком |
| SS-02 | Medium | **Log Rotation** — автоматичне обмеження розміру та ротація лог-файлів | Settings має "Retention Days", але немає size limit. При активному логуванні файли ростуть необмежено |
| SS-03 | Low | **Session duration in tab tooltip** — показувати час з'єднання | Nice-to-have, мінімальний effort |

---

## 8. Search & Filtering

### Поточний стан
Fuzzy search з кешуванням, protocol filtering, tag/group filtering, command palette (Ctrl+P),
search history.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| SR-01 | Low | **Search syntax hint** — placeholder або tooltip з доступними операторами | `SearchEngine` підтримує оператори (protocol:, tag:, host:), але користувач не знає про них |
| SR-02 | Low | **Search by custom properties** — індексувати custom properties для пошуку | Custom properties існують, але не searchable. Корисно лише для power users з великою кількістю з'єднань |

---

## 9. Secret Management

### Поточний стан
7 backends (KeePassXC, libsecret, KDBX, Bitwarden, 1Password, Passbolt, Pass),
async resolution, TTL caching, encrypted master passwords.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| SC-01 | Medium | **Backend unavailable toast at startup** — одноразовий toast якщо preferred backend недоступний | Fallback працює мовчки. Користувач може не знати що credentials беруться з fallback backend замість primary |
| SC-02 | Low | **Credential test** — кнопка "Test" в connection dialog для перевірки credentials перед збереженням | Test Connection вже є, але він тестує повне з'єднання. Окремий credential test — рідкісна потреба |

---

## 10. Split View

### Поточний стан
Horizontal/vertical split, color pool, focus navigation (Ctrl+`), tab grouping.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| SV-01 | Low | **Layout presets** — швидкі шаблони (2x1, 1x2, 2x2) | Зручно, але split створюється двома shortcut-ами. Економія — 1-2 натискання |

---

## 11. Cluster Management

### Поточний стан
Broadcast mode, session status per member, CRUD через діалоги.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| CL-01 | Medium | **Cluster from sidebar selection** — створити кластер з виділених з'єднань | Зараз: відкрити діалог → вибрати з'єднання по одному. З multi-select в sidebar це має бути одна дія |
| CL-02 | Low | **Cluster status indicator** — агрегований статус (all/partial/none connected) | `ClusterListRow` не показує скільки members online |

---

## 12. Automation

### Поточний стан
Expect rules (text/regex), pre/post tasks, variable substitution, pattern tester.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| A-01 | Medium | **Task timeout** — configurable timeout для pre/post connection tasks | Tasks можуть зависнути (наприклад, VPN connect що чекає input). Немає timeout — UI блокується |
| A-02 | Low | **Automation templates** — готові expect rules для типових сценаріїв (sudo, SSH host key confirm) | Кожен rule створюється з нуля. Але User Guide вже має приклади — можна скопіювати |

---

## 13. Flatpak Integration

### Поточний стан
Downloadable CLI tools, SHA256 verification, progress + cancel, auto PATH.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| F-01 | Medium | **"Install" action in missing-CLI toast** — коли CLI відсутній у Flatpak, toast з кнопкою що відкриває Flatpak Components | Зараз generic "command not found". Користувач може не знати про Flatpak Components діалог |
| F-02 | Low | **Installed version display** — показувати версію встановленого CLI в компонентах | Зараз лише Install/Remove без інформації про версію |

---

## 14. Keyboard & Accessibility

### Поточний стан
25+ customizable shortcuts, shortcuts dialog з пошуком, keybinding recording.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| K-01 | Medium | **Shortcut conflict detection** — warning при призначенні вже зайнятого shortcut | Зараз можна призначити Ctrl+N двом діям без попередження. Реальний баг |
| K-02 | Medium | **Tooltip consistency** — додати `tooltip-text` до всіх кнопок sidebar toolbar | Не всі кнопки мають tooltips. GNOME HIG вимагає tooltip для кожного інтерактивного елемента |

---

## 15. Document Management

### Поточний стан
Encrypted documents (AES-256-GCM), password protection, CRUD, dirty indicator.

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| D-01 | Low | **Document search** — пошук по вмісту документів | Документи не індексуються. Корисно при 10+ документах, але це secondary feature |

---

## 16. Graceful Degradation

### Поточний стан
Всі fallback paths з product.md реалізовані (tray, KeePassXC, embedded RDP/VNC, audio, waypipe).

### Задачі

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| GD-01 | Medium | **Flatpak CLI fallback** — toast з "Install" кнопкою замість generic error (= F-01) | Дублює F-01, об'єднати |
| GD-02 | Low | **External client min version** — warning якщо FreeRDP/TigerVNC занадто старі | Clients tab показує версії, але не перевіряє мінімальні вимоги. Рідкісний сценарій |

---

## 17. Уніфікація (інженерні задачі)

Ці задачі не мають прямого впливу на UX, але покращують maintainability та consistency коду.

| # | Пріоритет | Задача | Обґрунтування |
|---|-----------|--------|---------------|
| U-01 | Medium | **Toast vs Alert rule** — формалізувати та задокументувати: recoverable → toast, decision → dialog | product.md описує правило, але код не завжди слідує йому |
| U-02 | Low | **Dialog widget consistency** — Connection dialog використовує `adw::Window`, решта — `adw::Dialog` | Функціонально однаково для користувача, але ускладнює підтримку |
| U-03 | Low | **Search in all list dialogs** — додати пошук до Cluster list та Log viewer | History та Shortcuts мають пошук, Cluster list та Log viewer — ні |
| U-04 | Low | **Callback pattern** — обрати один підхід (`run<F>` або `set_callback` + `present`) | Чистий рефакторинг, нуль впливу на UX |

---


## 18. User Guide — оновлення та розширення

### Відсутні секції

| # | Пріоритет | Секція | Обґрунтування |
|---|-----------|--------|---------------|
| UG-01 | High | **Zero Trust Providers** — покрокове налаштування кожного провайдера (AWS SSM, GCP IAP, Azure Bastion, OCI, Cloudflare, Teleport, Tailscale, Boundary) | Складні в налаштуванні, в User Guide лише "Provider-specific". Без документації користувач не зможе налаштувати |
| UG-02 | High | **Security Best Practices** — вибір backend, master password, keyring, credential hygiene | Критично для connection manager. Зараз розкидано по Troubleshooting без структури |
| UG-03 | Medium | **FAQ** — часті питання | Troubleshooting покриває технічні проблеми, не загальні ("як перенести на іншу машину?", "де зберігаються паролі?") |
| UG-04 | Medium | **Migration Guide** — end-to-end міграція з Remmina, MobaXterm, Royal TS | Import є, але немає guide "як повністю перейти з X на RustConn" |

### Секції що потребують розширення

| # | Пріоритет | Секція | Проблема |
|---|-----------|--------|----------|
| UG-05 | High | **Templates** — 12 рядків | Не описано: створення з існуючого з'єднання, редагування, видалення, CLI. Major feature без документації |
| UG-06 | High | **Snippets** — 10 рядків | Не описано: синтаксис змінних, приклади, виконання в терміналі, CLI |
| UG-07 | High | **Clusters** — 6 рядків | Не описано: додавання/видалення members, broadcast workflow, disconnect all, CLI |
| UG-08 | High | **Group Operations Mode** — не задокументований | Sidebar має повноцінний multi-select режим (bulk delete, move to group, select all) через кнопку в toolbar, але User Guide описує лише одиничні операції (Rename, Move, Delete) |
| UG-08 | Medium | **Troubleshooting** — 8 сценаріїв | Відсутні: Serial device access, Kubernetes problems, Flatpak permissions, monitoring issues, Pass backend |
| UG-09 | Medium | **Import/Export** — базовий опис | Відсутні: per-format limitations, batch workflow, приклади файлів |
| UG-10 | Medium | **Encrypted Documents** — базовий опис | Відсутні: use cases, backup considerations |
| UG-11 | Low | **Connection History** — 4 рядки | Відсутні: фільтрація, пошук, connect from history |
| UG-12 | Low | **Connection Statistics** — 3 рядки | Відсутні: що відстежується, як інтерпретувати |

### Помилки та неточності

| # | Пріоритет | Проблема |
|---|-----------|----------|
| UG-13 | High | **Quick Connect shortcut conflict** — "First Connection" каже Ctrl+K, "Keyboard Shortcuts" каже Ctrl+Shift+Q. Одне з двох неправильне |
| UG-14 | Medium | **Pass backend в Troubleshooting** — описаний в Settings, відсутній в Troubleshooting (всі інші backends мають секцію) |
| UG-15 | Low | **Table of Contents** — не включає Adaptive UI |

---

## Зведена таблиця пріоритетів

### 🔴 High (8 задач) — реальний біль користувача або критичні gaps в документації

| ID | Задача | Чому High |
|----|--------|-----------|
| SS-01 | Session Reconnect кнопка | Найчастіший friction: disconnect → шукати в sidebar → double-click |
| G-01 | Recursive Group Delete з вибором | Захист від втрати даних; `delete_group_cascade` є в core, UI не використовує |
| E-01 | Actionable Error Toasts (Retry) | Найчастіший error flow: connection failed → нічого не можна зробити крім повторного кліку |
| UG-01 | User Guide: Zero Trust Providers | 8 провайдерів без документації — unusable feature |
| UG-05 | User Guide: Templates (12→60+ рядків) | Major feature без документації |
| UG-06 | User Guide: Snippets (10→40+ рядків) | Major feature без документації |
| UG-07 | User Guide: Clusters (6→40+ рядків) | Major feature без документації |
| UG-08 | User Guide: Group Operations Mode | Повноцінний multi-select режим не задокументований |

### 🟡 Medium (18 задач) — помітне покращення, але є workaround

| ID | Задача |
|----|--------|
| G-02 | Group Edit Dialog (simple) |
| E-02 | Standardize Toast Format |
| E-03 | i18n audit для toast titles |
| S-01 | Settings Backup/Restore |
| S-02 | Per-Protocol Defaults |
| IE-01 | Selective Export |
| IE-02 | Export format validation |
| P-01 | SSH Key path validation |
| SS-02 | Log Rotation |
| SC-01 | Backend unavailable toast |
| CL-01 | Cluster from sidebar selection |
| A-01 | Task timeout |
| F-01 | Flatpak "Install" action in toast |
| K-01 | Shortcut conflict detection |
| K-02 | Tooltip consistency |
| U-01 | Toast vs Alert rule |
| UG-02–04 | User Guide: Security, FAQ, Migration |
| UG-08–10 | User Guide: Troubleshooting, Import/Export, Documents |
| UG-13–14 | User Guide: shortcut conflict fix, Pass troubleshooting |

### 🟢 Low (19 задач) — nice-to-have або рідкісні сценарії

C-01, C-02, G-03, E-04, S-03, IE-03, P-02, P-03, SS-03, SR-01, SR-02,
SC-02, SV-01, CL-02, A-02, F-02, GD-02, D-01, U-02, U-03, U-04, UG-11, UG-12, UG-15

---

### Видалені задачі (з попередньої версії)

Наступні задачі видалені як feature creep, over-engineering, або нульовий вплив на UX:

| Видалено | Причина |
|----------|---------|
| Bulk Delete / Move (C-01 old) | Вже реалізовано: Group Operations Mode в sidebar має multi-select, bulk delete з confirmation, move to group з hierarchical dropdown |
| Bulk Edit полів (C-01 old) | З'єднання мають унікальні параметри; bulk edit port/username — штучний сценарій |
| Settings Profiles | Over-engineering; один набір налаштувань достатній для 99% користувачів |
| Saved Filters (SR-02 old) | У типового користувача 20-50 з'єднань; fuzzy search достатній |
| Document Templates | Documents — secondary feature; templates для них — зайвий шар |
| Broadcast to Split Panes | Дублює cluster broadcast |
| Keyboard Navigation Guide | In-app tutorial — перебір; shortcuts dialog достатній |
| RTL Layout Testing | Немає RTL мов в LINGUAS, немає попиту |
| Collapsible Advanced Sections | Діалог вже має tabs/stack; додаткове згортання ускладнює |
| Settings Change Log | Diff view для settings — over-engineering |
| Credential Expiry Warning | TTL cache — internal detail; користувач просто вводить пароль знову |
| Backend Fallback Chain UI | Fallback працює автоматично; UI індикація — noise |
| Component Update Check | Flatpak CLI tools оновлюються рідко; manual check достатній |
| Dialog Factory / guidelines | Інженерна задача без впливу на UX; guidelines достатньо в product.md |
| Connection Info Panel | "View Details" вже є; окрема панель — дублювання |
| Group Tags | Групи — контейнери; tags на контейнерах — зайвий рівень організації |
| Import/Export History | Разова операція; журнал не потрібен |
| Export to RDM / Virt-Viewer | Асиметрія import/export не є проблемою; export потрібен рідше |
| Context-Aware Error Messages | Складна реалізація, мінімальний вплив |
| Error History quick access | Log viewer доступний через меню; quick access — marginal improvement |
| Backend Status Dashboard | KeePass button в sidebar + Settings → Secrets достатньо |

---

**Загалом: 44 задачі** (8 High, 18 Medium, 19 Low)
**Видалено: 27 задач** як bloat, over-engineering, або вже реалізований функціонал
