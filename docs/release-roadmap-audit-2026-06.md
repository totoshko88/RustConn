# RustConn — Аудит кодової бази та релізний роадмап (червень 2026, база 0.15.13)

> **Статус (2026-06-11):** у гілці 0.15.14 виконано P1.1, P1.2, P1.3, P2.1, P2.2.
> P3.2 виявився хибною знахідкою (див. пункт). Лишаються: P2.3–P2.6, P3.1, P3.3–P3.5.

Методологія: 4 паралельні дослідження (мертвий код, GNOME HIG/UX, продуктивність,
архітектура/якість core) + `cargo clippy --all-targets` (чистий, 0 попереджень).
Кожна знахідка верифікована вручну за file:line перед включенням. Хибні та
завищені файндінги винесено в окремий розділ наприкінці.

Кожна задача має формат: **ID · Пріоритет · Місце в коді · Опис · Промпт для
ШІ-інструмента · Критерій приймання**. Промпти самодостатні — їх можна
вставляти в ШІ-агент без додаткового контексту.

---

## Пріоритет 1 — Відчутні для користувача проблеми відгуку UI

### P1.1 — UI-фриз 1.5 с при кожному RDP/VNC-підключенні через FreeRDP

- **Місце:** `rustconn/src/embedded.rs:472` (`start_with_geometry`), викликається з
  `rustconn/src/window/rdp_vnc.rs:806` та `rustconn/src/embedded.rs:338` — головний потік GTK.
- **Проблема:** після `cmd.spawn()` стоїть `std::thread::sleep(1500ms)` для раннього
  виявлення помилок (сертифікат/автентифікація) через `child.try_wait()`. Це блокує
  головний цикл GTK на 1.5 с при **кожному** підключенні зовнішнім клієнтом —
  вікно не перемальовується, анімації та ввід замерзають.
- **Вплив:** високий, відтворюється завжди.
- **Промпт для ШІ:**
  > In `rustconn/src/embedded.rs`, `start_with_geometry` blocks the GTK main thread
  > with `std::thread::sleep(1500ms)` after `cmd.spawn()` to detect immediate FreeRDP
  > failures via `child.try_wait()`. Refactor to non-blocking: return `Ok(())`
  > immediately after a successful spawn, store the child via `tab.set_process(child)`,
  > then schedule `glib::timeout_add_local` polling `try_wait()` every ~250ms up to
  > 1500ms total; if the process exited with failure, read stderr, run
  > `Self::parse_freerdp_error`, and surface the error through the same UI path the
  > caller in `rustconn/src/window/rdp_vnc.rs:806` uses today (the caller currently
  > branches on the sync `Result` — restructure it to accept an error callback or to
  > handle a late-failure signal). Keep the 1500ms total detection window (documented
  > magic constant). Do not hold any `RefCell` borrow across the timeout callback.
- **Приймання:** RDP-підключення не фризить вікно; помилка сертифіката/автентифікації
  досі показується користувачу; `cargo clippy` чистий; ручна перевірка з невалідним хостом.

### P1.2 — Постійні 50 мс-полінги головного циклу для трею

- **Місце:** `rustconn/src/app.rs:672` (50 мс, назавжди), `rustconn/src/app.rs:764`
  (2 с, повертає `Continue` навіть коли трей `None`).
- **Проблема:** таймер на 50 мс будить головний цикл ~20 разів/с увесь час роботи
  застосунку лише щоб зробити `try_recv()` на каналі повідомлень трею. Це постійне
  фонове споживання CPU/батареї навіть в idle. (Таймер ініціалізації на
  `app.rs:206` — НЕ проблема: він робить `Break` після створення трею.)
- **Вплив:** високий для ноутбуків (idle wakeups), середній для десктопів.
- **Промпт для ШІ:**
  > In `rustconn/src/app.rs` around line 672 there is a permanent
  > `glib::timeout_add_local(50ms)` polling `tray.try_recv()` for `TrayMessage`s, and
  > at line 764 a 2s timer that returns `Continue` even when the tray is `None`.
  > Replace the 50ms polling with an event-driven channel: switch the tray message
  > channel to `async_channel::unbounded()` (or `glib::MainContext::channel`-equivalent),
  > have the tray backend thread send into it, and consume it with
  > `glib::spawn_future_local` + `while let Ok(msg) = rx.recv().await` on the main
  > context — preserving the existing weak-ref guards (break the loop when the app or
  > window upgrade fails). For the 2s state-sync timer, make it return early without
  > work when the tray is `None` and consider starting it only after the tray is
  > actually created (it is set up in the same function scope where tray init
  > completes). Check `rustconn/src/tray*` for the sender side. Keep behavior for all
  > `TrayMessage` variants identical.
- **Приймання:** `grep timeout_add_local rustconn/src/app.rs` не містить 50 мс-полінгу;
  трей-меню (Show/Hide, підключення) працює як раніше; idle wakeups застосунку
  помітно нижчі (перевірити `powertop` або `strace -c -p`).

### P1.3 — Послідовне опитування 6+ CLI-бекендів секретів блокує вкладку Secrets

- **Місце:** `rustconn/src/dialogs/settings/secrets_tab/detection.rs` (весь
  `detect_secret_backends`, ~389 рядків), викликається з фонового потоку в
  `secrets_tab/mod.rs`.
- **Проблема:** детекція запускає 10+ дочірніх процесів **послідовно**
  (`keepassxc-cli --version`, `bw --version` по кількох шляхах, `bw status`,
  `op whoami`, passbolt, pass, secret-tool...). Кожен — 100–500 мс; сумарно 1–5 с
  до появи статусів у вкладці. Потік фоновий (UI не фризиться), але результат
  користувач чекає довго, і детекція повторюється при кожному відкритті налаштувань.
- **Вплив:** високий для UX вкладки налаштувань.
- **Промпт для ШІ:**
  > In `rustconn/src/dialogs/settings/secrets_tab/detection.rs`,
  > `detect_secret_backends()` probes ~6 secret-manager CLIs sequentially with
  > `std::process::Command::output()`. Parallelize: run each backend's probe in its
  > own scoped thread (`std::thread::scope`) and join the results into
  > `SecretCliDetection`, so total latency equals the slowest probe instead of the
  > sum. Respect the project rule "backend probe timeout 5s" (see
  > `.kiro/steering/secrets-guide.md`). Additionally cache the detection result for
  > the lifetime of the settings dialog process (e.g. `OnceLock<SecretCliDetection>`
  > or a timestamped cache with ~60s TTL) so reopening Settings doesn't respawn all
  > probes; keep the manual "refresh" path bypassing the cache. Do not log secrets;
  > keep tracing fields structured.
- **Приймання:** вкладка Secrets показує статуси за час ~найповільнішого процесу;
  повторне відкриття діалогу — миттєве; ручний refresh усе ще перезапускає детекцію.

---

## Пріоритет 2 — Технічний борг, що варто закрити в реліз 0.16

### P2.1 — Синхронний запис history.toml на головному потоці при кожному старті/завершенні сесії

- **Місце:** `rustconn/src/state/sessions.rs:234,242,250,276` →
  `rustconn-core/src/config/manager.rs:433` (`save_history`).
- **Проблема:** `record_connection_start/end` викликаються з window-коду
  (`window/protocols*.rs`, `window/session_lifecycle.rs`) під `state_mut` borrow на
  головному потоці й одразу серіалізують + пишуть TOML на диск. На повільному
  диску/NFS це мікрофризи; плюс зайві записи (2 на кожну сесію).
- **Вплив:** середній (малий файл, але sync I/O на main thread + знос SSD).
- **Промпт для ШІ:**
  > In `rustconn/src/state/sessions.rs`, `record_connection_start` and
  > `record_connection_end` call `self.save_history()` synchronously on the GTK main
  > thread (TOML serialize + disk write via
  > `rustconn-core/src/config/manager.rs::save_history`). Introduce debounced saving:
  > mark history dirty and flush via a single `glib::timeout_add_local_once`-scheduled
  > save ~2s after the last change, plus an unconditional flush on app shutdown (find
  > the existing shutdown path in `rustconn/src/app.rs` / window close handler and add
  > the flush there). Move the actual file write off the main thread
  > (`std::thread::spawn` with a snapshot `Vec<ConnectionHistoryEntry>` — the entries
  > are owned data). Make sure no `RefCell` borrow is held when the flush closure
  > runs (take-invoke-restore pattern per `.kiro/steering/window-guide.md`). Keep
  > `save_history` errors logged via `tracing::warn!`.
- **Приймання:** одна сесія = максимум один запис history.toml (debounce);
  історія не губиться при закритті застосунку; тести workspace зелені.

### P2.2 — Подвійний `suggested-action` в одному діалозі/сторінці (порушення HIG)

- **Місце:** `rustconn/src/dialogs/connection/data_tab.rs:64,100` («Add Variable» і
  «Add Property» обидва suggested); `rustconn/src/dialogs/tunnel_builder/step_connection.rs:145,179`
  (кнопка empty-state і «Next» обидві suggested на одній NavigationPage).
- **Проблема:** правило проєкту та HIG — одна `suggested-action` на діалог
  (`.kiro/steering/gnome-hig.md`). Дві сині кнопки розмивають первинну дію.
- **Вплив:** середній (візуальна ієрархія).
- **Промпт для ШІ:**
  > Enforce single `suggested-action` per dialog page. In
  > `rustconn/src/dialogs/connection/data_tab.rs` remove the `suggested-action` CSS
  > class from both "Add Variable" (line ~64) and "Add Property" (line ~100) buttons —
  > they are secondary list-management actions inside a preferences tab, not primary
  > dialog actions; use default flat style (consider `flat` + an `list-add-symbolic`
  > icon with text label). In
  > `rustconn/src/dialogs/tunnel_builder/step_connection.rs` keep `suggested-action`
  > only on the "Next" button (line ~179) and change the empty-state "New SSH
  > Connection" pill button (line ~145) to `["pill"]` only. Verify no other dialog
  > page ends up with zero or two suggested actions afterwards.
- **Приймання:** `grep -rn suggested-action rustconn/src/dialogs/` — жоден файл-діалог
  не має двох одночасно видимих suggested-кнопок; візуальна перевірка обох діалогів.

### P2.3 — `dialog_utils` повертає `Result<(), String>` замість структурованої помилки

- **Місце:** `rustconn-core/src/dialog_utils.rs` (`validate_name`, `validate_host`,
  `validate_port`, `validate_icon`); також `rustconn/src/state/sessions.rs:299`
  (`save_history -> Result<(), String>`).
- **Проблема:** правило проєкту — помилки в core через `thiserror::Error`, щоб
  виклики могли pattern-match'ити варіанти. `String`-помилки цьому суперечать.
- **Вплив:** середній (API-якість, i18n помилок).
- **Промпт для ШІ:**
  > In `rustconn-core/src/dialog_utils.rs` the validation functions return
  > `Result<(), String>`. Define `#[derive(Debug, thiserror::Error)] pub enum
  > ValidationError` with structured variants (e.g. `EmptyName`, `NameTooLong { max:
  > usize }`, `InvalidHost { reason: ... }`, `InvalidPort`, `InvalidIcon`), update the
  > four validators and all call sites (search `dialog_utils::` across the workspace —
  > GUI dialogs map errors to user-facing strings; wrap those messages in `i18n()` at
  > the GUI call site, never inside core). Add `# Errors` doc sections to each public
  > validator. Update the existing tests for these validators. Run
  > `bash po/update-pot.sh` if new user-facing strings appear in the GUI layer.
- **Приймання:** у `rustconn-core` немає публічних `Result<_, String>` у
  `dialog_utils.rs`; клієнти компілюються; clippy чистий; тести зелені.

### P2.4 — `expose_secret().to_string()` без `Zeroizing` у RDP/SPICE-клієнтах

- **Місце:** `rustconn-core/src/rdp_client/client/connection.rs:311`
  (`build_connector_config` → ironrdp `Credentials::UsernamePassword`),
  `rustconn-core/src/spice_client/client.rs:440` (`set_password`).
- **Критична оцінка:** обидва місця передають пароль у сторонні API, що приймають
  `String` **за значенням** — обгортання проміжного значення в `Zeroizing` не
  зануляє копію, яка житиме всередині ironrdp/spice-клієнта. Тобто це не «витік»,
  а обмеження сторонніх API. Проте правило проєкту вимагає `Zeroizing` для
  проміжних значень, і зараз ці два місця — єдині невідповідності в продакшн-коді.
- **Вплив:** низько-середній (формальна відповідність + документація обмеження).
- **Промпт для ШІ:**
  > Two production sites create plain `String` copies of passwords for third-party
  > APIs: `rustconn-core/src/rdp_client/client/connection.rs:311` (ironrdp
  > `Credentials::UsernamePassword { password: String }`) and
  > `rustconn-core/src/spice_client/client.rs:440` (`set_password(String)`). For each:
  > (1) check whether the current ironrdp / spice client versions in Cargo.lock accept
  > anything better than `String` (e.g. a secrecy type) — if yes, migrate; (2) if not,
  > keep the `String` but add an explanatory comment matching the existing
  > passbolt.rs precedent ("third-party API requires owned String; the copy's
  > lifetime is controlled by <crate>"), and wrap any *intermediate* buffers in
  > `zeroize::Zeroizing`. Also add the missing manual-`Debug`-no-leak test if these
  > config structs derive `Debug` while holding the plain password (check
  > `RdpClientConfig`/spice config Debug impls per
  > `.kiro/steering/rust-pragmatic-guidelines.md`).
- **Приймання:** обидва місця або мігровані, або задокументовані; тест на
  не-витік у `Debug` існує для відповідних структур; clippy чистий.

### P2.5 — Розбити `dialogs/connection/dialog.rs` (5988 рядків)

- **Місце:** `rustconn/src/dialogs/connection/dialog.rs` — найбільший файл проєкту.
  Наступні: `terminal/mod.rs` (3119), `dialogs/template.rs` (3053),
  `models/protocol.rs` (2994), `window/mod.rs` (2907).
- **Проблема:** файл-гігант ускладнює і ревʼю, і роботу ШІ-інструментів (не влазить
  у контекст). Сусідні файли вкладки вже виділені (`data_tab.rs`, `serial.rs`...) —
  патерн розбиття існує.
- **Вплив:** середній (підтримуваність), нульовий ризик для користувача якщо
  робити суто механічно.
- **Промпт для ШІ:**
  > Mechanically split `rustconn/src/dialogs/connection/dialog.rs` (5988 lines) into
  > submodules within `rustconn/src/dialogs/connection/`, following the existing
  > pattern of sibling files (`data_tab.rs`, `serial.rs`). Suggested cut lines:
  > field/widget construction per tab, signal-handler wiring, validation+save logic,
  > and load-from-Connection population. Pure code motion only: no behavior changes,
  > no renames of public items, keep `pub(crate)` visibility minimal, register new
  > modules in `connection/mod.rs`. Work in slices of ~1000 lines per step and run
  > `cargo check -p rustconn` after each step. Finish with `cargo fmt --all`,
  > `cargo clippy --all-targets` (0 warnings) and `cargo test --workspace`.
  > Do the same later for `dialogs/template.rs` as a follow-up.
- **Приймання:** жоден файл у `dialogs/connection/` не перевищує ~1500 рядків;
  диф — тільки переміщення коду; тести/клліппі зелені.

### P2.6 — Відсутні `# Errors` секції у публічних функціях core

- **Місце (вибірка):** `rustconn-core/src/ssh_tunnel.rs` (`find_free_port`,
  `create_tunnel`, `probe_tunnel_remote`), `host_check.rs` (`check_host_online`),
  `variables/manager.rs` (`resolve`, `substitute`, `parse_references`,
  `detect_cycles`), `dialog_utils.rs`, `tunnel_manager.rs::stop`. Всього ~15–20 функцій.
- **Вплив:** середній (правило M-CANONICAL-DOCS проєкту; документаційний борг).
- **Промпт для ШІ:**
  > Audit `rustconn-core/src` for public functions returning `Result` that lack an
  > `# Errors` doc section (rule: `.kiro/steering/rust-pragmatic-guidelines.md`).
  > Known offenders: ssh_tunnel.rs (`find_free_port`, `create_tunnel`,
  > `probe_tunnel_remote`), host_check.rs (`check_host_online`),
  > variables/manager.rs (`resolve`, `substitute`, `parse_references`,
  > `detect_cycles`), dialog_utils.rs validators, tunnel_manager.rs (`stop`). For each,
  > add a concise `# Errors` section naming the concrete error variants and when they
  > occur (and `# Panics` if any). Documentation-only change; verify with
  > `cargo doc -p rustconn-core --no-deps` building cleanly.
- **Приймання:** перелічені функції мають `# Errors`; `cargo doc` без попереджень.

---

## Пріоритет 3 — Поліпшення нижчого пріоритету / гігієна

### P3.1 — Виконати `TODO(0.16)` у RDP: переоцінити `catch_unwind`

- **Місце:** `rustconn-core/src/rdp_client/client/connection.rs:231`.
- **Опис:** коментар прямо адресований релізу 0.16: перевірити, чи IronRDP 0.15+
  виправив джерела панік, і якщо так — зняти `catch_unwind`-обгортку. Це
  релізна задача, а не мертвий код.
- **Промпт для ШІ:**
  > `rustconn-core/src/rdp_client/client/connection.rs:231` has
  > `TODO(0.16): re-evaluate whether catch_unwind is still needed`. Check the ironrdp
  > version pinned in Cargo.lock and its changelog for fixes to the panic sources this
  > wrapper guards against. If fixed: remove the `catch_unwind`, simplify the error
  > path, and note it in CHANGELOG.md `### Changed`. If not fixed: update the TODO to
  > `TODO(0.17)` with a link to the upstream issue. Run the rdp_client tests.

### P3.2 — ~~Обмежити ріст PCM-буфера аудіо~~ ХИБНА ЗНАХІДКА

- **Статус (верифіковано 2026-06-11):** буфер **уже обмежений** —
  `AudioBuffer` у `rustconn/src/audio.rs` використовує `VecDeque<i16>` з
  `max_size` (~4 с при 48 кГц стерео) і скидає найстаріші семпли при
  переповненні (`push_pcm_data`, рядки ~70–81); поведінку покриває наявний
  тест `test_audio_buffer_overflow`. Сабагент процитував неіснуючий код
  (`extend_from_slice` на полі `samples`). Дій не потрібно.

### P3.3 — Дрібні UX/i18n правки

- `rustconn/src/dialogs/connection_wizard/connection_page.rs:216` — tooltip
  `"cloudflared access ssh --hostname ..."` не локалізований. Це приклад команди,
  тож рішення: або лишити з коментарем «command example, intentionally untranslated»,
  або обгорнути в `i18n()` якщо текст міститиме пояснення.
- `rustconn/src/window/sorting.rs:90` — `.map(|g| (*g).clone())` клонує
  `ConnectionGroup` при заповненні сайдбару; замінити на передачу `Rc`/посилань,
  якщо підпис споживача дозволяє. Мікрооптимізація — робити «при нагоді».
- **Промпт для ШІ:**
  > Two small cleanups: (1) in
  > `rustconn/src/dialogs/connection_wizard/connection_page.rs:216` the tooltip
  > "cloudflared access ssh --hostname ..." is a literal command example — add a
  > one-line comment stating it is intentionally not wrapped in i18n(), or wrap it if
  > you reword it into explanatory text. (2) in `rustconn/src/window/sorting.rs:90`
  > avoid deep-cloning `ConnectionGroup` during sidebar population — pass `Rc` clones
  > or references through to the consumer if signatures allow; if the consumer needs
  > owned values, leave as-is and skip.

### P3.4 — Точкові прогалини тестового покриття в core

- **Критична оцінка:** заява «~40 файлів без тестів» завищена — багато модулів
  мають inline-тести. Реально варті покриття (логіка з крайовими випадками,
  безпека):
  - `rustconn-core/src/shell_escape.rs` — **безпекочутливий** (екранування для shell), тестів нема;
  - `rustconn-core/src/connection/retry.rs` — логіка backoff/реконекту;
  - `rustconn-core/src/connection/port_check.rs`, `host_check.rs` — мережеві перевірки;
  - `rustconn-core/src/smart_folder.rs` — матчинг правил фільтрації.
- **Промпт для ШІ:**
  > Add tests for four under-covered rustconn-core modules, following
  > `.kiro/steering/test-patterns.md` (proptest 1.x, register modules in
  > `tests/properties/mod.rs` / `tests/integration/mod.rs`, `tempfile` for temp files,
  > `SecretString` for credential fixtures): (1) `shell_escape.rs` — property tests
  > asserting escaped output is safe for POSIX shell (no unescaped metacharacters;
  > round-trip via `sh -c 'printf %s'` style check is acceptable as integration test);
  > (2) `connection/retry.rs` — backoff sequence, cancel token honored, no retry on
  > the documented skip conditions; (3) `connection/port_check.rs` + `host_check.rs` —
  > behavior on closed port / unreachable host with bounded timeouts (use
  > 127.0.0.1 ephemeral listener via `std::net::TcpListener::bind("127.0.0.1:0")`);
  > (4) `smart_folder.rs` — rule matching edge cases (empty query, case sensitivity,
  > multiple criteria). Keep total added runtime under ~10s.

### P3.5 — Моніторинговий TODO у RDPDR

- **Місце:** `rustconn-core/src/rdp_client/rdpdr.rs:183` — чекає на
  `ClientDriveNotifyChangeDirectoryResponse` в ironrdp. Дія: при кожному бампі
  ironrdp перевіряти, чи зʼявилось API. Не задача релізу — пункт чек-листа
  оновлення залежностей.

---

## Відхилені та деградовані файндінги (критична оцінка)

Ці пункти агенти позначили як проблеми, але верифікація їх не підтвердила.
Включено, щоб ніхто не витрачав на них час у 0.16:

1. **`serial.rs:81` — «блокуючий» `read_dir("/dev")` у клік-хендлері** — відхилено.
   `/dev` — віртуальна ФС у памʼяті, читання займає мікросекунди; виносити в
   потік — зайва складність без виграшу.
2. **`app.rs:206` — «вічний» 50 мс-таймер ініціалізації трею** — відхилено:
   таймер повертає `ControlFlow::Break` одразу після створення трею або помилки.
3. **«Надмірні клони» у `window/smart_folders.rs`** — деградовано до nit:
   клони `Rc` — інкремент лічильника; клон `AppSettings` відбувається один раз
   на дію користувача, не в циклі. Окремої задачі не вартує.
4. **Мертвий код** — фактично відсутній: clippy чистий, усі
   `#[allow(dead_code)]` (keepassxc.rs, rdpdr.rs, bitwarden.rs) обґрунтовані
   (serde-десеріалізація/Debug-логування), невикористаних залежностей і
   закоментованих блоків не знайдено. Кодова база в цьому сенсі зразкова.
   Єдина дія — P3.1 (TODO 0.16).
5. **HIG-відповідність загалом** — аудит підтвердив високий рівень: жодного
   deprecated-віджета (`gtk::MessageDialog`, `FileChooserDialog`, `TreeView`...),
   іконки-кнопки мають і tooltip, і accessible label, adw::Clamp/відступи
   консистентні, sentence case дотриманий. Залишилось лише P2.2 і P3.3.
6. **«Зеленим» залишається й архітектура:** нуль порушень меж крейтів
   (gtk у core відсутній), нуль `unwrap` у продакшн-шляхах поза доведеними
   інваріантами, нуль `println!` поза CLI, нуль `anyhow` у core, нуль функцій
   із 7+ параметрами, take-invoke-restore у window-коді дотриманий.

---

## Рекомендований порядок виконання для релізу 0.16

| Етап | Задачі | Обґрунтування |
|------|--------|---------------|
| 1 | P1.1, P1.2 | Найвидиміші для користувача: фриз при підключенні + idle CPU |
| 2 | P1.3, P2.1 | I/O-латентність: налаштування секретів, history-записи |
| 3 | P2.2, P2.3, P2.4 | HIG + помилки + секрети — дрібні, незалежні, легко ревʼюються |
| 4 | P3.1, P3.2, P3.4 | Релізний TODO, буфер аудіо, тести безпекочутливих модулів |
| 5 | P2.5, P2.6, P3.3 | Рефакторинг-гігієна: розбиття dialog.rs, доки, nit-и |

Після кожного етапу: `cargo fmt --all` + `cargo clippy --all-targets` (0 попереджень)
+ `cargo test --workspace`; для user-facing змін — запис у `CHANGELOG.md`; нові
i18n-рядки → `bash po/update-pot.sh` + `msgmerge` для 16 мов.
