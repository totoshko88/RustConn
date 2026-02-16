# Документ вимог: Виправлення управління секретами

## Вступ

Цей документ описує п'ять критичних виправлень у підсистемі управління секретами RustConn. Поточна реалізація має проблеми з диспетчеризацією бекендів, автоматичним розблокуванням при старті, успадкуванням паролів для не-KeePass бекендів, збереженням паролів при підключенні з джерелом Vault, та розшифруванням пароля Bitwarden при старті. Ці виправлення забезпечать коректну роботу секретів незалежно від обраного бекенду (LibSecret, KeePass, Bitwarden, 1Password, Passbolt).

## Глосарій

- **Secret_Backend**: Реалізація трейту `SecretBackend` для зберігання/отримання облікових даних (LibSecret, KeePass, Bitwarden, 1Password, Passbolt)
- **Preferred_Backend**: Бекенд, обраний користувачем у Settings → Secrets → preferred_backend
- **PasswordSource**: Enum, що визначає джерело пароля з'єднання: Vault, Prompt, Variable, Inherit, None
- **Inherit**: Режим PasswordSource, при якому пароль береться з батьківської групи по ієрархії
- **Lookup_Key**: Рядок-ідентифікатор для пошуку облікових даних у сховищі секретів
- **Connection_Dialog**: Діалог швидкого створення/редагування з'єднання (`connection_dialogs.rs`)
- **Auto_Unlock**: Процес автоматичного розблокування сховища секретів при старті застосунку
- **BW_SESSION**: Змінна середовища, що містить ключ сесії розблокованого Bitwarden vault
- **Toast_Overlay**: Компонент `adw::ToastOverlay` для показу сповіщень користувачу
- **Credential_Resolver**: Компонент `CredentialResolver` у `rustconn-core`, що відповідає за пошук облікових даних
- **AppState**: Центральний стан застосунку, ініціалізується при старті в `AppState::new()`
- **SecretSettings**: Структура налаштувань секретів у `AppSettings.secrets`

## Вимоги

### Вимога 1: Диспетчеризація бекенду при завантаженні з Vault для груп

**User Story:** Як користувач, я хочу щоб кнопка "Load from Vault" у діалозі з'єднання використовувала обраний мною бекенд секретів, щоб паролі завантажувались з правильного сховища.

#### Критерії прийняття

1. WHEN користувач натискає "Load from Vault" для групи у Connection_Dialog, THE Connection_Dialog SHALL використовувати `select_backend_for_load` для визначення бекенду замість захардкодженого LibSecret
2. WHEN Preferred_Backend встановлено як Bitwarden, THE Connection_Dialog SHALL завантажувати пароль групи через `BitwardenBackend` з попереднім `auto_unlock`
3. WHEN Preferred_Backend встановлено як OnePassword, THE Connection_Dialog SHALL завантажувати пароль групи через `OnePasswordBackend`
4. WHEN Preferred_Backend встановлено як Passbolt, THE Connection_Dialog SHALL завантажувати пароль групи через `PassboltBackend`
5. IF Secret_Backend недоступний при завантаженні, THEN THE Connection_Dialog SHALL показати зрозуміле повідомлення про помилку через Toast_Overlay та записати деталі через `tracing::error!`

### Вимога 2: Автоматичне розблокування бекенду секретів при старті

**User Story:** Як користувач, я хочу щоб Bitwarden автоматично розблокувався при старті застосунку, щоб операції з секретами не потребували окремого розблокування при кожному зверненні.

#### Критерії прийняття

1. WHEN AppState ініціалізується і Preferred_Backend є Bitwarden, THE AppState SHALL розшифрувати `bitwarden_password_encrypted` з SecretSettings
2. WHEN пароль Bitwarden розшифровано або доступний через keyring, THE AppState SHALL виконати `auto_unlock` та встановити BW_SESSION у змінну середовища
3. IF Auto_Unlock Bitwarden завершується помилкою, THEN THE AppState SHALL записати попередження через `tracing::warn!` та продовжити ініціалізацію без блокування
4. WHILE BW_SESSION встановлено у змінній середовища, THE Secret_Backend SHALL використовувати існуючу сесію замість повторного розблокування при кожній операції

### Вимога 3: Успадкування паролів для всіх бекендів

**User Story:** Як користувач, я хочу щоб PasswordSource::Inherit працював однаково для всіх бекендів секретів, щоб з'єднання у групі отримували пароль від батьківської групи незалежно від обраного сховища.

#### Критерії прийняття

1. WHEN з'єднання має PasswordSource::Inherit і Preferred_Backend не є KeePass, THE Credential_Resolver SHALL шукати пароль групи використовуючи той самий Lookup_Key формат, що використовується при збереженні
2. WHEN `save_group_password_to_vault` зберігає пароль групи для не-KeePass бекенду, THE AppState SHALL використовувати `group.id.to_string()` як Lookup_Key
3. WHEN `resolve_inherited_credentials` шукає пароль групи для не-KeePass бекенду, THE Credential_Resolver SHALL використовувати `group.id.to_string()` як Lookup_Key
4. WHEN група має PasswordSource::Inherit, THE Credential_Resolver SHALL продовжити пошук у батьківській групі по ієрархії
5. WHEN група має PasswordSource::Vault і пароль знайдено, THE Credential_Resolver SHALL об'єднати знайдені облікові дані з username/domain групи


### Вимога 4: Автоматичне збереження пароля при підключенні з джерелом Vault

**User Story:** Як користувач, я хочу щоб коли джерело пароля встановлено як Vault і пароль відсутній у сховищі, введений мною пароль автоматично зберігався у Vault, щоб не потрібно було зберігати його вручну.

#### Критерії прийняття

1. WHEN PasswordSource є Vault і пароль не знайдено у сховищі і користувач вводить пароль через діалог, THE AppState SHALL автоматично зберегти введений пароль у Vault через обраний Secret_Backend
2. WHEN автоматичне збереження пароля у Vault завершується успішно, THE AppState SHALL записати інформацію через `tracing::info!`
3. IF автоматичне збереження пароля у Vault завершується помилкою, THEN THE AppState SHALL показати Toast з повідомленням про помилку збереження та записати деталі через `tracing::error!`
4. WHEN PasswordSource є Prompt і користувач не відмітив "Save credentials", THE AppState SHALL зберегти пароль лише у кеш без запису у Vault

### Вимога 5: Розшифрування пароля Bitwarden при старті

**User Story:** Як користувач, я хочу щоб збережений зашифрований пароль Bitwarden розшифровувався при старті застосунку, щоб `auto_unlock` міг використати його для автоматичного розблокування.

#### Критерії прийняття

1. WHEN AppState ініціалізується і `bitwarden_password_encrypted` присутній у SecretSettings, THE AppState SHALL викликати `decrypt_bitwarden_password()` для розшифрування пароля
2. WHEN розшифрування пароля Bitwarden успішне, THE SecretSettings SHALL містити розшифрований пароль у полі `bitwarden_password` для подальшого використання `auto_unlock`
3. IF розшифрування пароля Bitwarden завершується помилкою, THEN THE AppState SHALL записати попередження через `tracing::warn!` та продовжити ініціалізацію

### Вимога 6: Сповіщення користувача про помилки секретів

**User Story:** Як користувач, я хочу отримувати зрозумілі сповіщення коли операції з секретами завершуються помилкою, щоб я міг вжити відповідних заходів.

#### Критерії прийняття

1. WHEN збереження пароля у Vault завершується помилкою, THE AppState SHALL показати Toast через Toast_Overlay з повідомленням довжиною до 60 символів
2. WHEN Secret_Backend недоступний при спробі збереження або завантаження, THE AppState SHALL показати Toast з інформацією про недоступність бекенду
3. THE AppState SHALL записувати повні деталі помилки через `tracing::error!` при кожній невдалій операції з секретами
4. THE Toast повідомлення SHALL бути зрозумілими для користувача без технічних деталей, шляхів до файлів або стек-трейсів
