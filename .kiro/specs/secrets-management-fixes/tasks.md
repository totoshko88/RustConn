# План імплементації: Виправлення управління секретами

## Огляд

П'ять виправлень у підсистемі секретів RustConn. Зміни торкаються крейтів `rustconn` (GUI) та `rustconn-core` (бізнес-логіка). Нових модулів не створюється — виправляється існуючий код.

## Задачі

- [x] 1. Виправлення 5+2: Розшифрування пароля Bitwarden та auto-unlock при старті
  - [x] 1.1 Додати розшифрування `bitwarden_password_encrypted` в `AppState::new()`
    - У файлі `rustconn/src/state.rs`, функція `AppState::new()`, після блоку валідації KDBX (~рядок 237)
    - Додати виклик `settings.secrets.decrypt_bitwarden_password()` коли `bitwarden_password_encrypted.is_some()`
    - Логувати результат через `tracing::info!` / `tracing::warn!`
    - _Requirements: 5.1, 5.2, 5.3_

  - [x] 1.2 Додати eager auto_unlock Bitwarden в `AppState::new()`
    - Після розшифрування пароля (1.1), коли `preferred_backend == Bitwarden`
    - Викликати `auto_unlock(&settings.secrets)` через `crate::async_utils::with_runtime`
    - При успіху — `tracing::info!("Bitwarden vault unlocked at startup")`
    - При помилці — `tracing::warn!` без блокування ініціалізації
    - `auto_unlock` вже встановлює `BW_SESSION` через `std::env::set_var`
    - _Requirements: 2.1, 2.2, 2.3, 2.4_

  - [x] 1.3 Написати property тест для round-trip шифрування/розшифрування пароля Bitwarden
    - **Property 4: Round-trip шифрування/розшифрування пароля Bitwarden**
    - **Validates: Requirements 5.2**
    - Створити файл `rustconn-core/tests/properties/secret_fixes_tests.rs`
    - Зареєструвати модуль в `rustconn-core/tests/properties/mod.rs`
    - Генерувати випадкові непорожні рядки, encrypt → decrypt, перевірити рівність

- [x] 2. Checkpoint — Перевірити що тести проходять
  - Запустити `cargo test -p rustconn-core --test property_tests secret_fixes` та `cargo clippy --all-targets`
  - Переконатися що всі тести проходять, запитати користувача якщо є питання.

- [x] 3. Виправлення 1: Диспетчеризація бекенду при Load from Vault для груп
  - [x] 3.1 Замінити захардкоджений LibSecretBackend в `connection_dialogs.rs`
    - У файлі `rustconn/src/window/connection_dialogs.rs`, гілка `else` (~рядок 432-460)
    - Замінити `LibSecretBackend::new("rustconn")` на диспетчеризацію через `select_backend_for_load`
    - Для Bitwarden — використати `auto_unlock` перед `retrieve`
    - Для OnePassword — `OnePasswordBackend::new()`
    - Для Passbolt — `PassboltBackend::new()`
    - Для LibSecret — залишити `LibSecretBackend::new("rustconn")`
    - Потрібно передати `secret_settings` у замикання (clone перед move)
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5_

- [x] 4. Виправлення 3: Узгодження ключів для Inherit з не-KeePass бекендами
  - [x] 4.1 Додати обробку Inherit для не-KeePass бекендів в `resolve_credentials_blocking`
    - У файлі `rustconn/src/state.rs`, функція `resolve_credentials_blocking` (~рядок 1360)
    - Після блоку `if connection.password_source == PasswordSource::Inherit && kdbx_enabled` (~рядок 1437)
    - Додати новий блок для `Inherit && !kdbx_enabled`
    - Обхід ієрархії груп аналогічно KeePass гілці
    - Використати `select_backend_for_load` для визначення бекенду
    - Lookup key: `group.id.to_string()`
    - Диспетчеризація: Bitwarden (з auto_unlock), OnePassword, Passbolt, LibSecret
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

  - [x] 4.2 Перевірити що `save_group_password_to_vault` використовує `group.id.to_string()` як lookup_key
    - У файлах `rustconn/src/window/connection_dialogs.rs` (~рядок 623) та `rustconn/src/window/edit_dialogs.rs` (~рядок 961)
    - Перевірити що `lookup_key` що передається = `group.id.to_string()`
    - Якщо ні — виправити виклики щоб передавати UUID групи
    - _Requirements: 3.2_

  - [x] 4.3 Написати property тест для узгодженості lookup_key
    - **Property 1: Узгодженість lookup_key для групових паролів (round-trip)**
    - **Validates: Requirements 3.1, 3.2, 3.3**
    - Генерувати випадкові ConnectionGroup, перевірити що save key == resolve key

  - [x] 4.4 Написати property тест для обходу ієрархії при Inherit
    - **Property 2: Обхід ієрархії при Inherit**
    - **Validates: Requirements 3.4**
    - Генерувати ієрархію груп (1-5 рівнів) з різними PasswordSource
    - Перевірити що Inherit пропускає проміжні групи і доходить до Vault

  - [x] 4.5 Написати property тест для merge облікових даних групи
    - **Property 3: Об'єднання облікових даних групи**
    - **Validates: Requirements 3.5**
    - Генерувати Credentials та ConnectionGroup з різними комбінаціями username/domain
    - Перевірити що merge правильно об'єднує поля

- [x] 5. Checkpoint — Перевірити що тести проходять
  - Запустити `cargo test -p rustconn-core --test property_tests secret_fixes` та `cargo clippy --all-targets`
  - Переконатися що всі тести проходять, запитати користувача якщо є питання.

- [x] 6. Виправлення 4: Автоматичне збереження пароля при підключенні з Vault
  - [x] 6.1 Додати auto-save для RDP при password_source == Vault
    - У файлі `rustconn/src/window/rdp_vnc.rs`, RDP password callback (~рядок 107)
    - Змінити умову з `if creds.save_credentials` на `if should_save`
    - `should_save = creds.save_credentials || connection.password_source == Vault`
    - _Requirements: 4.1, 4.4_

  - [x] 6.2 Додати auto-save для VNC при password_source == Vault
    - У файлі `rustconn/src/window/rdp_vnc.rs`, VNC password callback (~рядок 678)
    - Аналогічна зміна як для RDP
    - _Requirements: 4.1, 4.4_

- [x] 7. Виправлення 6: Toast сповіщення при помилках збереження
  - [x] 7.1 Додати Toast callback у `save_password_to_vault` та `save_group_password_to_vault`
    - У файлі `rustconn/src/state.rs`
    - У callback помилки додати відправку Toast через канал або прямий виклик
    - Повідомлення: "Failed to save password to vault" (до 60 символів)
    - Зберегти існуючий `tracing::error!` для деталей
    - _Requirements: 6.1, 6.2, 6.3, 6.4_

- [x] 8. Фінальний checkpoint — Перевірити що всі тести проходять
  - Запустити `cargo test -p rustconn-core --test property_tests` та `cargo clippy --all-targets`
  - Переконатися що всі тести проходять, запитати користувача якщо є питання.

## Примітки

- Задачі позначені `*` є опціональними і можуть бути пропущені для швидшого MVP
- Кожна задача посилається на конкретні вимоги для трасування
- Checkpoints забезпечують інкрементальну валідацію
- Property тести валідують універсальні властивості коректності
- Unit тести валідують конкретні приклади та edge cases
