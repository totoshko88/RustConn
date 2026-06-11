# RustConn 0.16.0 — План релізу (складено 2026-06-11)

Джерела: `docs/release-roadmap-audit-2026-06.md` (залишки), GitHub issue #137,
туторіал Microsoft «Run Linux GUI apps with WSL», аналіз конкурентів
(Devolutions RDM Linux + форум фіче-реквестів, Ásbrú CM, MobaXterm).

Формат задач: **ID · Пріоритет · Опис · Промпт для ШІ · Критерій приймання** —
як в аудиті; промпти самодостатні.

---

## Блок A — Залишки аудиту (база: release-roadmap-audit-2026-06.md)

Виконано у 0.15.14: P1.1, P1.2, P1.3, P2.1, P2.2. P3.2 — хибна знахідка.
**Беремо у 0.16.0** (промпти й деталі — в аудиті, тут лише порядок):

| ID | Суть | Обсяг |
|----|------|-------|
| A1 = P3.1 | `TODO(0.16)` в RDP: переоцінити `catch_unwind` (connection.rs:231) — релізний TODO, адресований саме 0.16 | малий |
| A2 = P2.3 | `dialog_utils` → `thiserror::ValidationError` замість `Result<(), String>` | малий-середній |
| A3 = P2.4 | `expose_secret().to_string()` без `Zeroizing` у RDP/SPICE — мігрувати або задокументувати + Debug-no-leak тест | малий |
| A4 = P2.6 | `# Errors` секції для ~15–20 публічних fn у core | малий (docs-only) |
| A5 = P3.4 | Тести: `shell_escape.rs` (безпекочутливий!), `connection/retry.rs`, `port_check.rs`/`host_check.rs`, `smart_folder.rs` | середній |
| A6 = P2.5 | Розбити `dialogs/connection/dialog.rs` (5988 рядків) — механічно, нульовий ризик | середній |
| A7 = P3.3 | Дрібниці: tooltip cloudflared (коментар/i18n), клон `ConnectionGroup` у sorting.rs:90 | дрібний |
| — = P3.5 | RDPDR TODO — НЕ задача релізу, лише чек-лист бампа ironrdp | — |

---

## Блок B — Issue #137: Windows / WSL (докладна інструкція)

**Контекст issue:** запит на Windows-підтримку. karlbundy підтвердив, що RustConn
працює під Ubuntu 26.04 у WSL2 на Windows 11; award-wq не зміг запустити
(без деталей); нативний Windows-порт визнаний проблемним (GTK4/MSYS2 —
коментар Hammerklavier-cn). **Рішення для 0.16:** офіційна, перевірена
інструкція WSL2/WSLg + позиція щодо нативного порту в issue. Нативний порт —
out of scope.

### B1 — `docs/WSL.md` (нова сторінка, англійською)

Структура на основі learn.microsoft.com/windows/wsl/tutorials/gui-apps:

1. **Prerequisites**
   - Windows 10 Build 19044+ або Windows 11 (WSLg входить у WSL ≥ 0.47.1).
   - Тільки **WSL 2** (WSL 1 GUI-застосунки не підтримує; `wsl --set-version <distro> 2`).
   - vGPU-драйвер виробника (Intel/AMD/NVIDIA) для апаратного OpenGL — лінки як у MS-доці.
2. **Install / update WSL**
   - Свіже: `wsl --install` (admin PowerShell) → перезавантаження → юзер/пароль Ubuntu.
   - Наявне: `wsl --update` + `wsl --shutdown` для рестарту.
3. **Install RustConn inside the distro**
   - Ubuntu 24.04+/26.04: .deb з GitHub Releases або OBS-репозиторій
     (ті самі кроки, що в `docs/INSTALL.md`, з конкретними командами `apt`).
   - Перелік runtime-залежностей, які в мінімальному WSL-образі відсутні
     (gtk4, libadwaita, vte4-gtk4, freerdp3-х клієнт — звірити точні пакети
     при написанні).
4. **RustConn-specific WSL setup** — те, чого нема в MS-туторіалі і через що,
   ймовірно, у award-wq не запустилося:
   - **systemd + D-Bus**: `/etc/wsl.conf` → `[boot] systemd=true`, потім
     `wsl --shutdown`. Без session bus не працюють Secret Service
     (gnome-keyring), портали (sftp:// через файловий менеджер), нотифікації.
   - **gnome-keyring**: установка + ініціалізація, або явний вибір CLI-бекенда
     секретів (KeePassXC/Bitwarden/pass) у Settings → Secrets як альтернатива.
   - Запуск: з термінала `rustconn` або зі Start menu (Ubuntu → RustConn).
5. **Known limitations under WSLg** (чесний список):
   - системний трей: у WSLg немає StatusNotifierHost — трей-іконка недоступна;
   - продуктивність вбудованого RDP/VNC-рендеру нижча, ніж нативно;
   - звук — через PulseAudio-сервер WSLg (працює, але з латентністю);
   - serial-порти потребують `usbipd-win` (лінк на MS-доку usbipd).
6. **Troubleshooting**
   - «cannot open display» → лінк на офіційний гайд WSLg
     (github.com/microsoft/wslg/wiki/Diagnosing-"cannot-open-display"-type-issues-with-WSLg);
   - перевірити `echo $DISPLAY` / `$WAYLAND_DISPLAY`;
   - `wsl --version` (WSLg version у виводі);
   - застарілий Windows build → оновити.

**Обов'язково:** перед публікацією прогнати інструкцію вручну на реальній
WSL2-машині або підтвердити кроки через karlbundy в issue.

- **Промпт для ШІ:**
  > Create `docs/WSL.md` — a step-by-step guide for running RustConn on Windows
  > via WSL2/WSLg, following the structure of Microsoft's "Run Linux GUI apps
  > with WSL" tutorial: prerequisites (Windows 10 19044+/11, WSL2 only, vendor
  > vGPU drivers), `wsl --install`/`wsl --update`, installing the RustConn .deb
  > inside Ubuntu (reuse the exact apt/OBS commands from `docs/INSTALL.md`),
  > then RustConn-specific setup: enabling systemd in `/etc/wsl.conf` for
  > D-Bus/Secret Service, gnome-keyring or CLI secret backends, launching from
  > Start menu. Add "Known limitations" (no tray under WSLg, embedded client
  > performance, audio via WSLg PulseAudio, serial needs usbipd-win) and
  > "Troubleshooting" (link the official WSLg "cannot open display" wiki page).
  > Verify package names against `debian/control`. Link the new page from
  > README.md (Installation section) and `docs/INSTALL.md`. English, sentence
  > case headings, same style as other docs/ pages.
- **Приймання:** docs/WSL.md існує, злінкована з README та INSTALL.md; команди
  звірені з debian/control; інструкція перевірена на живій WSL2 (або
  підтверджена користувачем в issue).

### B2 — Закрити/оновити issue #137

Коментар в issue: лінк на docs/WSL.md, чітка позиція — нативний Windows-порт
не планується у найближчих релізах (GTK4+VTE на Windows — нерозв'язана
екосистемна проблема), WSL2 — підтримуваний шлях. Лишити issue відкритим з
лейблом `windows`/`documentation` або закрити як answered — рішення мейнтейнера.

---

## Блок C — Конкурентний аналіз: що беремо, що вже є, що відхиляємо

### C0 — Підсумок паритету (для release notes / маркетингу)

Перевірено по коду: більшість топ-реквестів форуму RDM Linux у RustConn
**уже реалізовані**: mouse jiggler для RDP (`dialogs/connection/rdp.rs:259`),
RemoteApp (`embedded_rdp`), serial-порти, «host online» перевірка
(`host_check.rs`), syntax highlighting (highlight rules), кастомні змінні
(`variables/`), keyboard shortcuts, scrollback-налаштування, кластерний
broadcast (= MobaXterm multi-execution), expect-автоматизація і WoL (= Ásbrú),
ProxyJump/SSH gateway, X11 forwarding, імпорт з усіх трьох конкурентів.
Варто явно перелічити це у README comparison / release notes.

### C1 — TOTP для з'єднань · ВІДХИЛЕНО (рішення 2026-06-11)

Розглядалося (RDM-реквест №23: TOTP у quick search), але **виключено зі скоупу**
з двох причин:

1. **Принципова:** зберігання TOTP-секрету поруч із паролем з'єднання руйнує
   саму ідею другого фактора — обидва фактори опиняються на одній машині в
   одному сховищі. Рішення мейнтейнера: не заохочувати цей патерн.
2. **Технічна:** наявний `SecretBackend` працює з фіксованою структурою
   `Credentials` без довільних полів; навіть `key_passphrase` реально
   персистять лише libsecret/pass (vault-CLI бекенди повертають `None`).
   Повноцінний TOTP вимагав би per-backend матриці: локальна генерація для
   libsecret/pass/keyring/KDBX + делегування для `bw get totp` /
   `op item get --otp` / KeePassXC `get-totp` — обсяг «середній+» з
   найдорожчою частиною саме у vault-сценаріях.

Якщо в issues з'явиться реальний запит — повертатися до дворівневого дизайну
(локальна генерація + делегування читання), не раніше 0.17.

### C2 — Нотатки до з'єднання + індикатор у сайдбарі · Пріоритет 1

- **Звідки:** RDM-реквести №5 та №19 (documentation/attachments badge) —
  двічі в топі форуму. У моделі `Connection` поля `notes` нема (перевірено);
  модуль `document/` — це контейнери-документи, не per-connection нотатки.
- **Обсяг:** малий-середній.
- **Дизайн:** `notes: Option<String>` у `Connection` (serde default — стара
  конфігурація читається без міграції); вкладка/поле «Notes» (TextView у
  PreferencesGroup) в діалозі з'єднання; маленька symbolic-іконка
  (`note-symbolic`) у рядку сайдбара коли нотатка непорожня + tooltip;
  пошук у сайдбарі та quick search враховує текст нотаток; експорт у
  native (.rcn) формат; CLI show/set.
- **Промпт для ШІ:**
  > Add per-connection notes. Core: `notes: Option<String>` with
  > `#[serde(default, skip_serializing_if = "Option::is_none")]` on the
  > `Connection` model; include notes text in search matching
  > (`rustconn-core/src/search/`) and in native export/import. GUI: a "Notes"
  > expander or tab in the connection dialog (gtk::TextView inside
  > adw::PreferencesGroup, 12px margins); in the sidebar row show a small
  > `note-symbolic` icon (with tooltip and accessible label) when notes are
  > non-empty — status must not rely on color alone. CLI: print notes in
  > `show`, allow `--notes` on add/update. i18n + POTFILES.in + update-pot;
  > CHANGELOG `### Added`. Property test: connection with notes round-trips
  > through native export/import.
- **Приймання:** нотатка зберігається/відновлюється, бейдж видно, пошук
  знаходить по тексту нотатки, старі конфіги читаються; тести зелені.

### C3 — Batch edit для мультивиділення · Пріоритет 2

- **Звідки:** RDM-реквест №16 (Batch Edit). У сайдбарі мультивиділення вже є
  (`sidebar/`, `multi_select`), але групової зміни властивостей нема
  (перевірено — «batch» у window/ відсутній).
- **Обсяг:** середній.
- **Дизайн:** для виділених N з'єднань — пункт контекст-меню «Edit N
  connections…»; діалог зі свідомо вузьким набором полів першої ітерації:
  група, теги, іконка, протокольно-агностичні прапорці (auto-reconnect,
  jiggler для RDP-вибірки). Кожне поле має чекбокс «застосувати» — змінюються
  лише увімкнені. Одна транзакція збереження, один undo-toast.
- **Промпт для ШІ:**
  > Implement batch edit for multi-selected sidebar connections. Add an "Edit
  > N connections…" context-menu item when 2+ connections are selected
  > (selection plumbing already exists in `rustconn/src/sidebar/`). Dialog
  > (adw, Clamp 600px): rows for group (ComboRow), tags, icon, and
  > auto-reconnect switch — each row paired with an "apply" CheckButton;
  > only checked fields are written to each selected connection. Apply in one
  > pass through `SharedAppState` (take-invoke-restore, no borrow across
  > callbacks), save config once, show one toast "Updated N connections" with
  > Undo restoring the snapshot. i18n with `i18n_f("Updated {} connections")`
  > pattern; register dialog in dialogs/mod.rs; CHANGELOG `### Added`.
- **Приймання:** зміна групи/тегів для 5 виділених — один запис конфігурації,
  Undo повертає стан; clippy чистий.

### C4 — Розглянуті й відхилені для 0.16 (з обґрунтуванням)

| Ідея (джерело) | Чому не беремо |
|---|---|
| Нативний Windows-порт (#137) | GTK4+VTE+libadwaita на Windows — нерозв'язана екосистемна проблема (підтверджено в коментарях issue); WSL-шлях покриває потребу |
| Мережеві тули MobaXterm (iperf, tcpdump, port scan) | Несумісно з фокусом «connection manager»; на Linux ці тули і так під рукою |
| Вбудований X-сервер / редактор / package manager (MobaXterm) | Специфіка Windows-платформи MobaXterm, на Linux безглуздо |
| Window arrangements 2x2/3x3 (RDM №9) | Split-термінали вже є (`split/`); грід-аранжування окремих вікон — рідкісний кейс, дорогий у GTK; відкласти до запитів користувачів |
| Devolutions Send, Certificate Generator, vaults/team edition | Корпоративна екосистема Devolutions, поза скоупом одноюзерного менеджера |
| Embedded web console для VMware (RDM №10) | Є протокол Web; спеціалізація під VMware — ніша, чекаємо запит |

---

## Рекомендований порядок виконання 0.16.0

| Етап | Задачі | Логіка |
|------|--------|--------|
| 1 | A1 (TODO 0.16), A2, A3, A4 | Дрібний техборг, розчищає core перед фічами |
| 2 | C2 Notes, B1+B2 WSL-дока | Незалежні, можна паралельно |
| 3 | C3 Batch edit | Після C2 (діалогова інфраструктура свіжа в голові) |
| 4 | A5 тести, A6 розбиття dialog.rs, A7 nit-и | Гігієна наприкінці; A6 — після всіх змін у діалозі з'єднань, щоб не ловити конфлікти |
| 5 | Реліз: changelog-пропагація по 6 файлах (`.kiro/steering/release-reminder.md`), `cargo update` + `### Dependencies`, метаінфо | Стандартний ритуал |

Після кожного етапу: `cargo fmt --all` + `cargo clippy --all-targets`
(0 попереджень) + `cargo test --workspace`; user-facing зміни → CHANGELOG;
нові i18n-рядки → `bash po/update-pot.sh` + msgmerge для 16 мов.

**Оцінка обсягу:** етапи 1–2 ~ тиждень роботи з ШІ-асистом; 3–4 ~ тиждень;
якщо скоуп тисне — C3 (batch edit) перший кандидат на перенос у 0.17 без
шкоди для цілісності релізу.