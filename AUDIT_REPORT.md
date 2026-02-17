# RustConn — Аудит кодової бази

**Дата:** 2026-02-16  
**Версія:** 0.8.6  
**Автор аудиту:** Kiro (Rust Software Architect)

---

## Зміст

1. [Безпека](#1-безпека)
2. [Якість коду: дублювання, мертвий код, покращення](#2-якість-коду)
3. [GUI: GNOME HIG та мобільна підтримка](#3-gui-gnome-hig)
4. [Зовнішні компоненти та CLI-клієнти](#4-зовнішні-компоненти)
5. [Flathub: відповідність вимогам](#5-flathub)
6. [CLI: відповідність clig.dev](#6-cli-cligdev)
7. [Питання для уточнення](#7-питання)

---

## 1. Безпека

### 🔴 P0 — Критичні

- [x] **SEC-01: Плейсхолдери SHA256 у cli_download.rs** ✅ v0.8.7
  - 8 із 14 компонентів мають фейкові чексуми: `"aws-cli-latest-no-checksum"`, `"kubectl-latest-no-checksum"`, `"c2d3e4f5a6b7...placeholder..."` тощо
  - Верифікація фактично обходиться для AWS CLI, SSM Plugin, kubectl, gcloud, cloudflared, Bitwarden, 1Password, Teleport
  - Тільки Tailscale та Boundary мають реальні чексуми
  - **Рішення:** Перейти на pinned-версії з реальними чексумами. Для "latest" URL — завантажувати `.sha256` файл з офіційного джерела та порівнювати
  ```rust
  // Замість:
  sha256: Some("aws-cli-latest-no-checksum"),
  
  // Реалізувати:
  pub enum ChecksumSource {
      /// Статичний SHA256
      Static(&'static str),
      /// URL до файлу з чексумою (завантажується перед основним файлом)
      RemoteFile(&'static str),
      /// Не перевіряти (з попередженням користувачу)
      None,
  }
  ```
  - Показувати `adw::AlertDialog` з попередженням при встановленні без верифікації

- [x] **SEC-02: Ін'єкція через змінні у командах** ✅ v0.8.7
  - `variables/mod.rs` підтримує `${variable_name}` синтаксис
  - Значення змінних підставляються у команди без санітизації
  - Якщо змінна містить shell-метасимволи і використовується в `build_command()`, можлива ін'єкція
  - **Рішення:** Додати шар валідації
  ```rust
  // rustconn-core/src/variables/mod.rs
  /// Sanitizes a variable value for safe use in shell commands.
  /// Rejects values containing shell metacharacters.
  pub fn sanitize_for_command(value: &str) -> Result<&str, VariableError> {
      let forbidden = ['|', ';', '&', '$', '`', '(', ')', '{', '}', '<', '>', '\n', '\r'];
      if value.chars().any(|c| forbidden.contains(&c)) {
          return Err(VariableError::UnsafeValue {
              reason: "contains shell metacharacters".into(),
          });
      }
      Ok(value)
  }
  ```

### 🟡 P1 — Важливі

- [x] **SEC-03: Логування повної команди в stderr (CLI)** ✅ v0.8.7
  - `execute_connection_command()` виводить `eprintln!("Executing: {} {}", program, args.join(" "))`
  - `custom_args` з конфігу передаються напряму — можуть містити чутливі дані
  - **Рішення:** Маскувати аргументи, що можуть містити паролі
  ```rust
  fn format_command_for_log(cmd: &ConnectionCommand) -> String {
      let masked_args: Vec<String> = cmd.args.iter().map(|a| {
          if a.starts_with("/p:") || a.starts_with("--password") {
              format!("{}=****", a.split('=').next().unwrap_or(a))
          } else {
              a.clone()
          }
      }).collect();
      format!("{} {}", cmd.program, masked_args.join(" "))
  }
  ```

- [x] **SEC-04: Документ-шифрування — фіксовані параметри Argon2** ✅ v0.8.7
  - Production: `m=65536, t=3, p=4` — добре, але не конфігурується
  - **Рішення:** Додати `EncryptionStrength` enum у `DocumentManager`
  ```rust
  pub enum EncryptionStrength {
      Standard,  // m=65536, t=3, p=4
      High,      // m=131072, t=4, p=8
      Maximum,   // m=262144, t=6, p=8
  }
  ```

- [ ] **SEC-05: SSH Agent passphrase handling**
  - `add_key()` використовує `SSH_ASKPASS_REQUIRE=force` але не обробляє інтерактивний ввід passphrase
  - **Рішення:** Використати PTY або `expect`-подібну бібліотеку для інтерактивного вводу

### 🟢 P2 — Рекомендації

- [ ] **SEC-06: Документувати lifecycle кредів**
  - Кредиціали кешуються в `SecretManager` з TTL, але немає документації коли вони очищуються
  - **Рішення:** Додати `/// # Security` секцію до `SecretManager` з описом lifecycle

- [x] **SEC-07: Додати property-тести для ін'єкцій** ✅ v0.8.7
  ```rust
  // rustconn-core/tests/properties/variable_injection.rs
  proptest! {
      #[test]
      fn variable_value_sanitization(value in ".*") {
          let result = sanitize_for_command(&value);
          if result.is_ok() {
              // Значення не містить небезпечних символів
              assert!(!value.contains(';'));
              assert!(!value.contains('|'));
          }
      }
  }
  ```

---

## 2. Якість коду

### 🔴 P0 — Критичні

- [x] **CODE-01: Монолітний CLI — 5000+ рядків в одному main.rs** ✅ v0.8.7
  - `rustconn-cli/src/main.rs` містить ВСЕ: парсинг, команди, форматування, помилки
  - **Рішення:** Розбити на модулі:
  ```
  rustconn-cli/src/
  ├── main.rs              // entry point, ~50 рядків
  ├── cli.rs               // Cli struct, Commands enum
  ├── error.rs             // CliError
  ├── format.rs            // OutputFormat, table/json/csv formatters
  ├── commands/
  │   ├── mod.rs
  │   ├── connect.rs       // build_*_command(), execute_connection_command()
  │   ├── list.rs          // cmd_list()
  │   ├── add.rs           // cmd_add()
  │   ├── export_import.rs // cmd_export(), cmd_import()
  │   ├── wol.rs           // cmd_wol()
  │   ├── snippet.rs       // cmd_snippet_*()
  │   ├── group.rs         // cmd_group_*()
  │   ├── template.rs      // cmd_template_*()
  │   ├── cluster.rs       // cmd_cluster_*()
  │   ├── variable.rs      // cmd_var_*()
  │   └── secret.rs        // cmd_secret_*()
  └── output.rs            // print helpers
  ```

### 🟡 P1 — Важливі

- [x] **CODE-02: `--config` прапорець оголошений але не використовується** ✅ v0.8.7 (CLI-01)
  - `Cli.config: Option<PathBuf>` (рядок 29) ніколи не передається в `ConfigManager::new()`
  - **Рішення:** Або видалити, або реалізувати:
  ```rust
  fn get_config_manager(config_path: Option<&Path>) -> Result<ConfigManager, CliError> {
      match config_path {
          Some(path) => ConfigManager::with_path(path),
          None => ConfigManager::new(),
      }.map_err(|e| CliError::Config(e.to_string()))
  }
  ```

- [x] **CODE-03: Дублювання build_command() між CLI та core** ✅ v0.8.7
  - `rustconn-cli/src/main.rs` має `build_rdp_command()`, `build_vnc_command()`, `build_spice_command()`
  - `rustconn-core/src/protocol/` має `Protocol::build_command()` для SSH, Serial, Kubernetes
  - RDP/VNC/SPICE повертають `None` з `Protocol::build_command()` — логіка тільки в CLI
  - **Рішення:** Перенести всі `build_*_command()` в `Protocol::build_command()` у core
  ```rust
  // rustconn-core/src/protocol/rdp.rs
  impl Protocol for RdpProtocol {
      fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
          let mut args = vec![format!("/v:{}:{}", connection.host, connection.port)];
          // ... решта логіки з CLI
          Some(std::iter::once("xfreerdp".to_string()).chain(args).collect())
      }
  }
  ```

- [x] **CODE-04: Дублювання VNC viewer detection** ✅ v0.8.7
  - `detect_vnc_viewer_path()` та `detect_vnc_viewer_name()` мають ідентичний список viewers
  - **Рішення:** Витягти в константу
  ```rust
  const VNC_VIEWERS: &[&str] = &[
      "vncviewer", "tigervnc", "gvncviewer", "xvnc4viewer",
      "vinagre", "remmina", "krdc",
  ];
  
  pub fn detect_vnc_viewer_path() -> Option<PathBuf> {
      VNC_VIEWERS.iter().find_map(|v| which_binary(v))
  }
  
  pub fn detect_vnc_viewer_name() -> Option<String> {
      VNC_VIEWERS.iter().find(|v| which_binary(v).is_some()).map(|v| v.to_string())
  }
  ```

- [x] **CODE-05: Дублювання icon mapping** ✅ v0.8.7
  - `adaptive_tabs.rs::TabInfo::get_protocol_icon()` дублює `protocol/icons.rs::get_protocol_icon()`
  - **Рішення:** Використовувати `rustconn_core::protocol::icons::get_protocol_icon()` замість локальної копії

### 🟢 P2 — Рекомендації

- [x] **CODE-06: Мертвий код — `flatpak.rs` модуль** ✅ v0.8.7
  - Документація каже: `flatpak-spawn --host` не працює після видалення `--talk-name=org.freedesktop.Flatpak`
  - Модуль залишений "for backward compatibility" але фактично не використовується у Flatpak
  - **Рішення:** Додати `#[deprecated]` або `cfg` guard:
  ```rust
  #[deprecated(since = "0.7.7", note = "flatpak-spawn --host disabled per Flathub policy")]
  pub fn host_command(program: &str) -> Command { ... }
  ```

- [ ] **CODE-07: `eprintln!` замість `tracing` у CLI**
  - Product rule вимагає `tracing` для structured logging
  - CLI використовує `println!`/`eprintln!` скрізь
  - **Рішення:** Додати `tracing-subscriber` з `--verbose` прапорцем:
  ```rust
  // rustconn-cli/src/main.rs
  fn setup_logging(verbose: bool) {
      let filter = if verbose { "debug" } else { "warn" };
      tracing_subscriber::fmt()
          .with_env_filter(filter)
          .with_writer(std::io::stderr)
          .init();
  }
  ```

- [ ] **CODE-08: Відсутність мінімальної перевірки версій CLI**
  - `detection.rs` визначає наявність клієнта, але не перевіряє мінімальну версію
  - Наприклад, FreeRDP 2.x vs 3.x мають різний API аргументів
  - **Рішення:** Додати `min_version` до `ClientInfo`:
  ```rust
  pub struct ClientInfo {
      // ...existing fields...
      pub min_version: Option<&'static str>,
      pub version_compatible: bool,
  }
  ```

---

## 3. GUI: GNOME HIG та мобільна підтримка

### 🟡 P1 — Важливі

- [x] **GUI-01: Деякі діалоги використовують `gtk4::Window` замість `adw::Window`** ✅ v0.8.7
  - `show_send_text_dialog()` створює `gtk4::Window` напряму
  - GNOME HIG рекомендує `adw::Window` або `adw::Dialog` для всіх модальних вікон
  - **Рішення:**
  ```rust
  // Замість:
  let dialog = gtk4::Window::builder()
      .title("Send Text to Session")
      .transient_for(parent)
      .build();
  
  // Використовувати:
  let dialog = adw::Dialog::builder()
      .title("Send Text to Session")
      .build();
  dialog.present(Some(parent));
  ```

- [ ] **GUI-02: Протокольні фільтри переповнюють на мобільних**
  - 8 кнопок фільтрів (SSH, RDP, VNC, SPICE, Telnet, Serial, ZeroTrust, K8s) у linked group
  - На 360px екрані — ~45px на кнопку (замало для touch)
  - **Рішення:** Додати breakpoint для приховування рідкісних протоколів:
  ```rust
  let bp_mobile = adw::Breakpoint::new(
      adw::BreakpointCondition::new_length(
          adw::BreakpointConditionLengthType::MaxWidth,
          400.0,
          adw::LengthUnit::Sp,
      )
  );
  // Приховати Telnet, Serial, ZeroTrust, K8s на мобільних
  bp_mobile.add_setter(&telnet_filter, "visible", Some(&false.to_value()));
  bp_mobile.add_setter(&serial_filter, "visible", Some(&false.to_value()));
  ```

- [x] **GUI-03: Sidebar мінімальна ширина 200px — забагато для телефонів** ✅ v0.8.7
  - 200px на 360px екрані = 55% ширини
  - **Рішення:** Зменшити до 150px або використати breakpoint:
  ```rust
  container.set_width_request(150); // Мінімум для мобільних
  ```

- [x] **GUI-04: Відсутні accessible names для icon-only кнопок** ✅ v0.8.7
  - Кнопки фільтрів, close-кнопки, local shell — мають tooltip але не accessible name
  - **Рішення:**
  ```rust
  ssh_filter.update_property(&[
      gtk4::accessible::Property::Label("Filter SSH connections")
  ]);
  close_button.update_property(&[
      gtk4::accessible::Property::Label("Close tab")
  ]);
  local_shell_btn.update_property(&[
      gtk4::accessible::Property::Label("Open local shell terminal")
  ]);
  ```

- [x] **GUI-05: Валідація форм не анонсується screen readers** ✅ v0.8.7
  - CSS клас `error` додається, але немає ARIA-подібного оголошення
  - **Рішення:** Використано `update_state()` з `State::Invalid(AccessibleInvalidState)` та `update_relation()` з `Relation::ErrorMessage(&[&Accessible])` у `validation.rs`

### 🟢 P2 — Рекомендації

- [ ] **GUI-06: Split view на мобільних**
  - Кнопки split view приховуються при 600sp, але split-контейнери можуть бути заплутаними на телефонах
  - **Рішення:** Повністю вимкнути split view при <400sp

- [ ] **GUI-07: Tray polling кожні 250ms**
  - Може спричиняти зайве навантаження CPU у idle
  - **Рішення:** Перейти на event-driven оновлення через канали:
  ```rust
  // Замість polling:
  glib::timeout_add_local(Duration::from_millis(250), move || { ... });
  
  // Event-driven:
  let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);
  rx.attach(None, move |msg: TrayMessage| {
      update_tray(&tray, &msg);
      glib::ControlFlow::Continue
  });
  ```

- [ ] **GUI-08: Непослідовні відступи у діалогах**
  - Connection dialog: 12px margins
  - Sidebar: 6px margins
  - **Рішення:** Стандартизувати: 6px між пов'язаними елементами, 12px між секціями (GNOME HIG)

- [ ] **GUI-09: Drag-and-drop недоступний для keyboard-only**
  - Drop indicator показує візуальний фідбек, але немає клавіатурної альтернативи
  - **Рішення:** Додати Ctrl+M для "Move to group" через діалог вибору групи

- [ ] **GUI-10: Навігація по історії пошуку**
  - Немає клавіатурних шорткатів для навігації по історії (стрілки вгору/вниз)
  - **Рішення:** Додати arrow key handler у search entry

- [ ] **GUI-11: Додати `<recommends>` у metainfo для мобільних**
  ```xml
  <recommends>
    <display_length compare="ge">360</display_length>
    <control>keyboard</control>
    <control>pointing</control>
    <control>touch</control>
  </recommends>
  ```

### ✅ Що зроблено добре

- `adw::OverlaySplitView` з breakpoints (400sp, 600sp) — відмінна адаптивність
- `adw::ToolbarView` для всіх вікон — правильний HIG pattern
- `adw::AlertDialog` для підтверджень — не deprecated `MessageDialog`
- `adw::ToastOverlay` з пріоритетами — правильні нотифікації
- `adw::StatusPage` для empty states — семантична структура
- Breakpoints використовують `sp` units — підтримка Large Text
- Wayland-first: немає X11-specific API, Cairo fallback для X11
- Gesture support: swipe для sidebar show/hide
- Adaptive tabs з overflow menu

---

## 4. Зовнішні компоненти та CLI-клієнти

### 🔴 P0 — Критичні

- [x] **EXT-01: Flatpak не може запускати зовнішні клієнти, але UI пропонує їх завантажити** ✅ v0.8.7
  - `flatpak-spawn --host` вимкнений (видалено `--talk-name=org.freedesktop.Flatpak`)
  - Але `cli_download.rs` + Settings → Clients tab пропонують завантажити xfreerdp, cloud CLI тощо
  - Завантажені CLI не можуть бути запущені з Flatpak sandbox
  - **Рішення:** Приховати кнопку завантаження у Flatpak для компонентів, що потребують host access:
  ```rust
  // rustconn/src/dialogs/flatpak_components.rs
  fn should_show_download(component: &DownloadableComponent) -> bool {
      if rustconn_core::flatpak::is_flatpak() {
          // У Flatpak показувати тільки компоненти, що працюють у sandbox
          // (наприклад, kubectl через мережу, але не xfreerdp)
          matches!(component.category, ComponentCategory::ContainerOrchestration)
      } else {
          component.is_downloadable()
      }
  }
  ```

- [x] **EXT-02: Hardcoded версії та URL у DOWNLOADABLE_COMPONENTS** ✅ v0.8.7
  - Версії зашиті статично: `tigervnc-1.16.0`, `teleport-v18.6.8`, `tailscale_1.94.1`, `boundary_0.21.0`, `kubectl v1.35.0`
  - Немає механізму автоматичного оновлення
  - **Рішення:** Реалізувати version resolver:
  ```rust
  // rustconn-core/src/cli_download/version_resolver.rs
  
  /// Resolves the latest version of a component from its official source.
  #[async_trait::async_trait]
  pub trait VersionResolver: Send + Sync {
      /// Returns (version, download_url, sha256_url) for the latest release.
      async fn resolve_latest(&self) -> Result<ResolvedVersion, CliDownloadError>;
  }
  
  pub struct ResolvedVersion {
      pub version: String,
      pub download_url: String,
      pub checksum_url: Option<String>,
      pub checksum: Option<String>,
  }
  
  // Для GitHub releases (cloudflared, boundary, bitwarden, 1password):
  pub struct GitHubReleaseResolver {
      pub owner: &'static str,
      pub repo: &'static str,
      pub asset_pattern: &'static str, // regex для вибору asset
  }
  
  // Для kubectl:
  pub struct KubectlResolver; // GET https://dl.k8s.io/release/stable.txt
  
  // Для Tailscale:
  pub struct TailscaleResolver; // GET https://pkgs.tailscale.com/stable/ + parse
  ```

### 🟡 P1 — Важливі

- [ ] **EXT-03: Тільки x86_64 архітектура**
  - Всі URL у `DOWNLOADABLE_COMPONENTS` — для `linux-amd64` / `x86_64`
  - Немає підтримки aarch64/arm64
  - **Рішення:** Додати arch detection:
  ```rust
  fn get_arch() -> &'static str {
      if cfg!(target_arch = "x86_64") { "amd64" }
      else if cfg!(target_arch = "aarch64") { "arm64" }
      else { "unknown" }
  }
  
  // У DownloadableComponent:
  pub download_urls: &'static [(&'static str, &'static str)], // [(arch, url)]
  ```

- [ ] **EXT-04: Встановлення CLI поза Flatpak — тільки download**
  - Для нативних пакетів (deb/rpm/snap) немає інтеграції з системним пакетним менеджером
  - **Рішення:** Додати `InstallMethod::SystemPackage`:
  ```rust
  pub enum InstallMethod {
      Download,
      Pip,
      CustomScript,
      /// Install via system package manager (apt, dnf, pacman, zypper)
      SystemPackage {
          apt: Option<&'static str>,    // "freerdp3-wayland"
          dnf: Option<&'static str>,    // "freerdp"
          pacman: Option<&'static str>, // "freerdp"
          zypper: Option<&'static str>, // "freerdp"
      },
  }
  
  fn detect_package_manager() -> Option<PackageManager> {
      if which_binary("apt").is_some() { Some(PackageManager::Apt) }
      else if which_binary("dnf").is_some() { Some(PackageManager::Dnf) }
      else if which_binary("pacman").is_some() { Some(PackageManager::Pacman) }
      else if which_binary("zypper").is_some() { Some(PackageManager::Zypper) }
      else { None }
  }
  ```
  - Показувати команду встановлення у toast/dialog:
  ```
  "FreeRDP not found. Install: sudo apt install freerdp3-wayland"
  ```

- [ ] **EXT-05: Немає перевірки мінімальної версії CLI**
  - `detection.rs` визначає наявність, але не перевіряє сумісність
  - FreeRDP 2.x vs 3.x мають різний API аргументів (`/v:` vs `--server`)
  - **Рішення:** Додати `min_version` та `parse_semver()`:
  ```rust
  pub struct ClientRequirement {
      pub binary: &'static str,
      pub min_version: Option<(u32, u32, u32)>,
      pub version_args: &'static [&'static str],
  }
  
  fn check_version_compatible(info: &ClientInfo, min: (u32, u32, u32)) -> bool {
      info.version.as_ref()
          .and_then(|v| parse_semver(v))
          .is_some_and(|v| v >= min)
  }
  ```

- [ ] **EXT-06: Version check timeout 6s — повільно для UI**
  - `VERSION_CHECK_TIMEOUT = 6s` з polling кожні 50ms
  - Settings → Clients tab може зависати на 6s × кількість CLI
  - **Рішення:** Вже є паралельна детекція (v0.8.3), але варто додати progress indicator:
  ```rust
  // Показувати spinner для кожного CLI окремо
  // Замість блокуючого detect_all(), використовувати async з callback
  ```

### 🟢 P2 — Рекомендації

- [ ] **EXT-07: Автоматизація оновлення версій через CI**
  - Створити GitHub Action для перевірки нових версій:
  ```yaml
  # .github/workflows/check-cli-versions.yml
  name: Check CLI versions
  on:
    schedule:
      - cron: '0 6 * * 1' # Щопонеділка
  jobs:
    check:
      runs-on: ubuntu-latest
      steps:
        - name: Check kubectl
          run: |
            LATEST=$(curl -sL https://dl.k8s.io/release/stable.txt)
            echo "kubectl: $LATEST"
        - name: Check Tailscale
          run: |
            LATEST=$(curl -sL https://pkgs.tailscale.com/stable/ | grep -oP 'tailscale_\K[\d.]+' | head -1)
            echo "tailscale: $LATEST"
        # ... інші CLI
  ```

- [ ] **EXT-08: Кешування результатів client detection**
  - Кожне відкриття Settings → Clients запускає повну детекцію
  - **Рішення:** Кешувати результати з TTL 5 хвилин:
  ```rust
  pub struct CachedDetection {
      result: ClientDetectionResult,
      timestamp: std::time::Instant,
  }
  
  static CACHE: OnceLock<RwLock<Option<CachedDetection>>> = OnceLock::new();
  ```

---

## 5. Flathub: відповідність вимогам

### Аналіз за Flathub Quality Guidelines

| Критерій | Статус | Коментар |
|----------|--------|----------|
| Reverse-DNS app ID | ✅ | `io.github.totoshko88.RustConn` |
| metadata_license | ✅ | `CC0-1.0` |
| project_license | ⚠️ | `GPL-3.0+` у metainfo vs `GPL-3.0-or-later` у Cargo.toml — різне представлення |
| developer id + name | ✅ | `io.github.totoshko88` / `Anton Isaiev` |
| Brand colors (light + dark) | ✅ | `#9141ac` / `#613583` |
| Icon SVG ≥256px | ✅ | SVG + PNG 256x256 |
| Icon reasonable footprint | ✅ | Потрібна ручна перевірка з icon grid |
| Screenshots ≥3 | ✅ | 3 скріншоти 1920×1080 |
| Screenshot captions | ✅ | Є для всіх |
| Description ≥2 paragraphs | ✅ | 5 параграфів з списками |
| Release notes | ✅ | 30+ релізів з описами |
| URL homepage + bugtracker | ✅ | GitHub |
| content_rating | ✅ | OARS 1.1 (empty = all ages) |
| Runtime not EOL | ✅ | GNOME Platform 49 |
| Desktop file | ✅ | Правильний формат |
| Flathub verification | ✅ | Верифіковано: https://flathub.org/en/apps/io.github.totoshko88.RustConn |

### 🟡 P1 — Покращення

- [x] **FH-01: Уніфікувати SPDX ліцензію** ✅ v0.8.7
  - metainfo: `GPL-3.0+` (старий формат)
  - Cargo.toml: `GPL-3.0-or-later` (новий SPDX)
  - **Рішення:** Змінити в metainfo на `GPL-3.0-or-later`:
  ```xml
  <project_license>GPL-3.0-or-later</project_license>
  ```

- [x] **FH-02: Додати `<translation>` елемент** ✅ v0.8.7
  - Flathub рекомендує вказувати систему перекладу
  - Реалізовано gettext інфраструктуру: `gettext-rs` crate, `i18n` модуль, `po/` директорія, `<translation type="gettext">rustconn</translation>` у metainfo.xml
  - **Рішення:** Реалізовано:
  ```xml
  <translation type="gettext">rustconn</translation>
  ```

- [x] **FH-03: Додати `<recommends>` та `<requires>`** ✅ v0.8.7
  - Flathub використовує для фільтрації на мобільних пристроях
  - **Рішення:**
  ```xml
  <requires>
    <display_length compare="ge">360</display_length>
  </requires>
  <recommends>
    <control>keyboard</control>
    <control>pointing</control>
    <control>touch</control>
    <display_length compare="ge">768</display_length>
  </recommends>
  <supports>
    <control>keyboard</control>
    <control>pointing</control>
    <control>touch</control>
  </supports>
  ```

- [ ] **FH-04: Додати screenshot для dark theme**
  - Flathub Quality Guidelines рекомендують скріншоти для обох тем
  - **Рішення:** Додати 1-2 скріншоти dark theme:
  ```xml
  <screenshot>
    <caption>Dark theme with active SSH session</caption>
    <image type="source" width="1920" height="1080">
      https://raw.githubusercontent.com/.../screenshots/dark_ssh.png
    </image>
  </screenshot>
  ```

- [ ] **FH-05: Brand colors — перевірити контраст з іконкою**
  - Light: `#9141ac` (фіолетовий), Dark: `#613583` (темно-фіолетовий)
  - Flathub рекомендує: "colors are not too similar to the app icon"
  - **Рішення:** Перевірити через [banner preview](https://docs.flathub.org/banner-preview/) — якщо іконка теж фіолетова, обрати complementary color

### 🟢 P2 — Рекомендації

- [ ] **FH-06: Додати `x-checker-data` для всіх modules у Flathub manifest**
  - Зараз тільки rustconn має `x-checker-data` для auto-updates
  - VTE, inetutils, picocom, libsecret, mc — без автоматичної перевірки
  - **Рішення:**
  ```yaml
  - name: vte
    sources:
      - type: archive
        url: https://download.gnome.org/sources/vte/0.78/vte-0.78.7.tar.xz
        x-checker-data:
          type: gnome
          name: vte
          stable-only: true
  ```

- [ ] **FH-07: Розглянути Flatpak extensions для опціональних CLI**
  - Замість завантаження CLI у sandbox, використати Flatpak extensions
  - **Рішення (довгострокове):**
  ```yaml
  # У manifest:
  add-extensions:
    io.github.totoshko88.RustConn.Clients:
      directory: clients
      no-autodownload: true
      autodelete: true
  ```

---

## 6. CLI: відповідність clig.dev

### Аналіз за clig.dev Guidelines

| Принцип | Статус | Деталі |
|---------|--------|--------|
| Subcommand structure | ✅ | 19 команд, вкладені підкоманди (snippet list/show/add/delete/run) |
| `--version` | ✅ | `#[command(author, version)]` + `propagate_version = true` |
| `--help` для всіх команд | ✅ | clap derive з `/// doc comments` |
| Exit codes | ✅ | 0 success, 1 general, 2 connection failure |
| Errors to stderr | ✅ | `eprintln!("Error: {e}")` |
| Machine-readable output | ✅ | `--format table\|json\|csv` |
| Flags vs args | ✅ | `#[arg(short, long)]` для опцій, positional для ідентифікаторів |
| `--verbose` / `--quiet` | ❌ | Відсутні |
| `--no-color` / `NO_COLOR` | ❌ | Відсутні |
| `--dry-run` | ❌ | `connect` робить `exec()` без preview |
| Shell completions | ❌ | clap_complete не підключений |
| stdin/pipe detection | ❌ | Немає `isatty()` перевірки |
| `--config` працює | ❌ | Оголошений але не використовується |
| Structured logging | ❌ | `println!`/`eprintln!` замість `tracing` |
| Pager for long output | ❌ | `list` з 1000+ з'єднань виводить все одразу |

### 🔴 P0 — Критичні

- [x] **CLI-01: Підключити `--config` або видалити** ✅ v0.8.7
  - Прапорець оголошений (рядок 29) але ніколи не передається в `ConfigManager`
  - Це порушує принцип "don't have flags that do nothing"
  - **Рішення:**
  ```rust
  // У кожній команді:
  let config_manager = match &cli.config {
      Some(path) => ConfigManager::with_path(path)?,
      None => ConfigManager::new()?,
  };
  ```
  - Потрібно додати `ConfigManager::with_path()` у rustconn-core

### 🟡 P1 — Важливі

- [x] **CLI-02: Додати `--verbose` / `--quiet`** ✅ v0.8.7
  - clig.dev: "If your program is not a simple query, provide a --verbose flag"
  - **Рішення:**
  ```rust
  #[derive(Parser)]
  pub struct Cli {
      /// Increase output verbosity (-v, -vv, -vvv)
      #[arg(short, long, action = clap::ArgAction::Count, global = true)]
      pub verbose: u8,
  
      /// Suppress all output except errors
      #[arg(short, long, global = true)]
      pub quiet: bool,
  
      // ...existing fields...
  }
  
  fn setup_logging(verbose: u8, quiet: bool) {
      let filter = match (quiet, verbose) {
          (true, _) => "error",
          (_, 0) => "warn",
          (_, 1) => "info",
          (_, 2) => "debug",
          _ => "trace",
      };
      tracing_subscriber::fmt()
          .with_env_filter(filter)
          .with_writer(std::io::stderr)
          .init();
  }
  ```

- [x] **CLI-03: Додати `--no-color` та `NO_COLOR` env** ✅ v0.8.7
  - clig.dev: "Respect NO_COLOR environment variable"
  - **Рішення:**
  ```rust
  fn use_color() -> bool {
      // Respect NO_COLOR (https://no-color.org/)
      if std::env::var("NO_COLOR").is_ok() {
          return false;
      }
      // Check if stdout is a terminal
      std::io::stdout().is_terminal()
  }
  ```
  - Додати `colored` або `owo-colors` crate для кольорового виводу

- [x] **CLI-04: Додати `--dry-run` для `connect`** ✅ v0.8.7
  - Зараз `connect` робить `exec()` і замінює процес без попередження
  - clig.dev: "If your command has a potentially dangerous action, provide a --dry-run flag"
  - **Рішення:**
  ```rust
  Commands::Connect {
      name: String,
      /// Show the command that would be executed without running it
      #[arg(long)]
      dry_run: bool,
  }
  
  // У cmd_connect():
  if dry_run {
      println!("{} {}", command.program, command.args.join(" "));
      return Ok(());
  }
  ```

- [x] **CLI-05: Додати shell completions** ✅ v0.8.7
  - clap підтримує `clap_complete` для bash, zsh, fish, powershell
  - **Рішення:**
  ```rust
  Commands::Completions {
      /// Shell to generate completions for
      #[arg(value_enum)]
      shell: clap_complete::Shell,
  }
  
  fn cmd_completions(shell: clap_complete::Shell) {
      let mut cmd = Cli::command();
      clap_complete::generate(shell, &mut cmd, "rustconn-cli", &mut std::io::stdout());
  }
  ```

- [x] **CLI-06: Pager для довгого виводу** ✅ v0.8.7
  - `list` з 1000+ з'єднань виводить все одразу
  - clig.dev: "Use a pager if you are outputting a lot of text"
  - **Рішення:**
  ```rust
  fn output_with_pager(content: &str) -> Result<(), CliError> {
      if !std::io::stdout().is_terminal() || content.lines().count() < 40 {
          print!("{content}");
          return Ok(());
      }
      // Pipe through less
      let mut child = std::process::Command::new("less")
          .args(["-FIRX"])
          .stdin(std::process::Stdio::piped())
          .spawn()
          .unwrap_or_else(|_| {
              // Fallback: print directly
              print!("{content}");
              std::process::exit(0);
          });
      if let Some(stdin) = child.stdin.as_mut() {
          use std::io::Write;
          let _ = stdin.write_all(content.as_bytes());
      }
      let _ = child.wait();
      Ok(())
  }
  ```

### 🟢 P2 — Рекомендації

- [ ] **CLI-07: Pipe detection — автоматичний JSON**
  - clig.dev: "If stdin is not an interactive terminal, prefer structured output"
  - **Рішення:**
  ```rust
  fn default_format() -> OutputFormat {
      if std::io::stdout().is_terminal() {
          OutputFormat::Table
      } else {
          OutputFormat::Json
      }
  }
  ```

- [ ] **CLI-08: Підказки при помилках**
  - clig.dev: "Suggest possible corrections when user input is invalid"
  - **Рішення:** Використати fuzzy matching для connection names:
  ```rust
  fn find_connection(connections: &[Connection], name: &str) -> Result<&Connection, CliError> {
      // Exact match
      if let Some(conn) = connections.iter().find(|c| c.name == name) {
          return Ok(conn);
      }
      // Fuzzy suggestions
      let suggestions: Vec<_> = connections.iter()
          .filter(|c| c.name.to_lowercase().contains(&name.to_lowercase()))
          .take(3)
          .collect();
      if !suggestions.is_empty() {
          let names: Vec<_> = suggestions.iter().map(|c| c.name.as_str()).collect();
          return Err(CliError::ConnectionNotFound(
              format!("'{}' not found. Did you mean: {}?", name, names.join(", "))
          ));
      }
      Err(CliError::ConnectionNotFound(format!("'{name}' not found")))
  }
  ```

- [x] **CLI-09: Confirmation для деструктивних операцій** ✅ v0.8.7
  - `delete` видаляє без підтвердження
  - clig.dev: "If input or output is a terminal, confirm before doing anything destructive"
  - **Рішення:**
  ```rust
  fn confirm(message: &str) -> bool {
      if !std::io::stdin().is_terminal() {
          return true; // Non-interactive: assume yes
      }
      eprint!("{message} [y/N] ");
      let mut input = String::new();
      std::io::stdin().read_line(&mut input).is_ok()
          && input.trim().eq_ignore_ascii_case("y")
  }
  ```

- [ ] **CLI-10: `man` page generation**
  - clap підтримує `clap_mangen` для генерації man pages
  - **Рішення:** Додати build script або окрему команду:
  ```rust
  // build.rs або Commands::ManPage
  fn generate_man_pages() {
      let cmd = Cli::command();
      let man = clap_mangen::Man::new(cmd);
      // Write to stdout or file
  }
  ```

---

## 7. Відповіді на питання

### 1. Flatpak extensions vs embedded-only

**Відповідь:** Потрібне докладне пояснення як працюють extensions.

**Як працюють Flatpak Extensions:**

Flatpak extensions — це механізм модульного розширення додатків без збільшення основного пакету. Вони працюють як "плагіни", що монтуються у файлову систему sandbox під час запуску.

**Архітектура:**

```
io.github.totoshko88.RustConn (основний додаток)
├── /app/bin/rustconn          ← основний бінарник
├── /app/clients/              ← точка монтування для extension
│   ├── xfreerdp               ← з'являється якщо extension встановлений
│   ├── kubectl
│   └── ...
```

**Як це працює технічно:**

1. У маніфесті основного додатку оголошується extension point:
```yaml
# io.github.totoshko88.RustConn.yml
finish-args:
  - --filesystem=xdg-data/rustconn:create

add-extensions:
  io.github.totoshko88.RustConn.Clients:
    directory: clients           # монтується в /app/clients/
    no-autodownload: true        # не завантажується автоматично
    autodelete: true             # видаляється разом з додатком
    subdirectories: true         # дозволяє під-extensions
    merge-dirs: bin              # об'єднує bin/ директорії
```

2. Кожен клієнт пакується як окремий extension:
```yaml
# io.github.totoshko88.RustConn.Clients.FreeRDP.yml
id: io.github.totoshko88.RustConn.Clients.FreeRDP
branch: stable
runtime: io.github.totoshko88.RustConn
sdk: org.gnome.Sdk//49

modules:
  - name: freerdp
    buildsystem: cmake-ninja
    config-opts:
      - -DWITH_WAYLAND=ON
      - -DWITH_X11=OFF
    sources:
      - type: archive
        url: https://github.com/FreeRDP/FreeRDP/releases/download/3.12.0/freerdp-3.12.0.tar.gz
```

3. Користувач встановлює extension окремо:
```bash
flatpak install io.github.totoshko88.RustConn.Clients.FreeRDP
```

4. Додаток бачить бінарники в `/app/clients/bin/`:
```rust
fn find_extension_binary(name: &str) -> Option<PathBuf> {
    let ext_path = PathBuf::from("/app/clients/bin").join(name);
    if ext_path.exists() { Some(ext_path) } else { None }
}
```

**Переваги:**
- Основний пакет залишається легким (~15 MB)
- Користувач встановлює тільки потрібні клієнти
- Кожен extension оновлюється незалежно
- Не потрібен `flatpak-spawn --host` — все працює в sandbox
- Flathub підтримує extensions нативно

**Недоліки:**
- Кожен extension потрібно окремо пакувати та підтримувати на Flathub
- Збільшує складність CI/CD
- Деякі CLI (AWS CLI, gcloud) великі (~200 MB) і складні для пакування
- Потрібна координація версій між основним додатком та extensions

**Рекомендація для RustConn:**
Embedded клієнти (IronRDP, vnc-rs) — основна стратегія. Extensions мають сенс тільки для:
- FreeRDP 3.x (як fallback для складних RDP сценаріїв)
- kubectl (для Kubernetes протоколу)
- picocom (для Serial, вже бандлиться)

Cloud CLI (aws, gcloud, az) краще залишити для нативних інсталяцій — вони занадто великі та часто оновлюються.

---

### 2. i18n: gettext vs fluent

**Відповідь:** Потрібне докладне пояснення різниці та рекомендація.

**gettext (GNU gettext)**

Класична система локалізації, стандарт для GNOME/GTK додатків.

```rust
// Використання:
use gettextrs::gettext;
println!("{}", gettext("Connection failed"));

// Множина:
use gettextrs::ngettext;
println!("{}", ngettext("1 connection", "{n} connections", count));
```

Файли перекладу — `.po` (текстові, зручні для перекладачів):
```po
# uk.po
msgid "Connection failed"
msgstr "З'єднання не вдалося"

msgid "Delete connection '%s'?"
msgstr "Видалити з'єднання '%s'?"
```

Інструменти: `xgettext` (витягує рядки), `msgfmt` (компілює), Weblate/Transifex/Damned Lies (платформи перекладу).

**Переваги gettext:**
- Стандарт GNOME — всі перекладачі знають формат
- Інтеграція з Damned Lies (GNOME Translation Project)
- `<translation type="gettext">rustconn</translation>` у metainfo — Flathub автоматично показує % перекладу
- Зрілий тулінг: `xgettext` автоматично витягує рядки
- Rust crate: `gettextrs` (обгортка над libintl)

**Недоліки gettext:**
- Обмежена підтримка складної граматики (роди, відмінки)
- `.po` файли можуть бути великими
- Потрібен `libintl` (є в GNOME runtime)

**Project Fluent (Mozilla)**

Сучасна система, розроблена Mozilla для Firefox.

```rust
// Використання:
use fluent::{FluentBundle, FluentResource};
let msg = bundle.get_message("connection-failed").unwrap();
// → "З'єднання не вдалося"

// Складна граматика:
let msg = bundle.get_message("delete-confirm").unwrap();
// delete-confirm = Видалити { $gender ->
//     [masculine] з'єднання "{$name}"
//     [feminine] групу "{$name}"
//    *[other] елемент "{$name}"
// }?
```

Файли перекладу — `.ftl`:
```ftl
# uk.ftl
connection-failed = З'єднання не вдалося
delete-confirm = Видалити з'єднання «{ $name }»?
connections-count = { $count ->
    [one] { $count } з'єднання
    [few] { $count } з'єднання
   *[other] { $count } з'єднань
}
```

**Переваги Fluent:**
- Краща підтримка складної граматики (роди, відмінки, множина)
- Чистий Rust (без C залежностей)
- Асиметрична локалізація — кожна мова може мати свою структуру
- Rust crates: `fluent`, `fluent-bundle`, `fluent-syntax`

**Недоліки Fluent:**
- Не стандарт GNOME — перекладачі можуть не знати формат
- Немає інтеграції з Damned Lies
- Немає `<translation type="fluent">` у AppStream — Flathub не покаже % перекладу
- Менше тулінгу для автоматичного витягування рядків

**Порівняльна таблиця:**

| Критерій | gettext | Fluent |
|----------|---------|--------|
| GNOME стандарт | ✅ Так | ❌ Ні |
| Flathub `<translation>` | ✅ Так | ❌ Ні |
| Damned Lies | ✅ Так | ❌ Ні |
| Weblate підтримка | ✅ Так | ✅ Так |
| Складна граматика | ⚠️ Обмежена | ✅ Відмінна |
| Pure Rust | ❌ libintl | ✅ Так |
| Зрілість тулінгу | ✅ 30+ років | ⚠️ ~7 років |
| Кількість перекладачів | ✅ Величезна | ⚠️ Менша |

**Рекомендація для RustConn:**

**gettext** — однозначно. Причини:
1. RustConn — GNOME додаток, gettext є стандартом екосистеми
2. Flathub Quality Guidelines вимагають `<translation>` — тільки gettext підтримується
3. GNOME Translation Project (Damned Lies) дає доступ до тисяч волонтерів-перекладачів
4. `libintl` вже є в GNOME Platform runtime (не потрібно бандлити)
5. Граматика RustConn достатньо проста — gettext покриває всі потреби

Fluent має сенс для складних додатків з багатою граматикою (Firefox, Thunderbird), але для connection manager це overkill.

---

### 3. Мобільна підтримка (Phosh/GNOME Mobile)

**Відповідь:** Поки ні. Поточна адаптивність через `adw::OverlaySplitView` + breakpoints достатня для планшетів. Повна мобільна підтримка (Phosh) потребує `adw::NavigationView` для деяких flow, що є значною переробкою.

---

### 4. CLI модуляризація

**Відповідь:** Так, виконано. ✅ CODE-01 у v0.8.7 — `main.rs` розбитий на 18 модулів.

---

### 5. FreeRDP 2.x vs 3.x

**Відповідь:** Тільки FreeRDP 3.x. Wayland-native підтримка є тільки у 3.x. `build_rdp_command()` потрібно оновити на FreeRDP 3.x синтаксис аргументів (`/v:` залишається у 3.x, але деякі прапорці змінились). FreeRDP 2.x detection можна залишити з попередженням "FreeRDP 2.x detected, please upgrade to 3.x for Wayland support".

---

### 6. Snap packaging

**Відповідь:** Snap є у списку, але не пройшов валідацію Snap Store. Модуль `snap.rs` залишається для сумісності. Snap manifest відсутній у репозиторії.

---

### 7. Flathub verification

**Відповідь:** Так, верифікація пройдена. Додаток опублікований: https://flathub.org/en/apps/io.github.totoshko88.RustConn

Оновлено статус у таблиці Flathub Quality Guidelines.

---

### 8. Property tests coverage

**Відповідь:** Цільового показника покриття немає. Поточний стан: ~2600 тестів (1300+ property tests). Додаткові тести додаються за потребою при виявленні проблем.

---

## Зведена таблиця пріоритетів

| ID | Категорія | Пріоритет | Опис |
|----|-----------|-----------|------|
| SEC-01 | Безпека | 🔴 P0 | ~~Плейсхолдери SHA256~~ ✅ |
| SEC-02 | Безпека | 🔴 P0 | ~~Ін'єкція через змінні~~ ✅ |
| EXT-01 | Компоненти | 🔴 P0 | ~~Flatpak непрацюючі CLI~~ ✅ |
| EXT-02 | Компоненти | 🔴 P0 | ~~Hardcoded версії~~ ✅ |
| CLI-01 | CLI | 🔴 P0 | ~~`--config` не працює~~ ✅ |
| CODE-01 | Код | 🔴 P0 | ~~Монолітний CLI 5000+ рядків~~ ✅ |
| SEC-03 | Безпека | 🟡 P1 | ~~Логування чутливих аргументів~~ ✅ |
| SEC-04 | Безпека | 🟡 P1 | ~~Фіксовані параметри Argon2~~ ✅ |
| SEC-05 | Безпека | 🟡 P1 | SSH Agent passphrase handling |
| CODE-02 | Код | 🟡 P1 | ~~`--config` dead code~~ ✅ |
| CODE-03 | Код | 🟡 P1 | ~~Дублювання build_command()~~ ✅ |
| CODE-04 | Код | 🟡 P1 | ~~Дублювання VNC viewer list~~ ✅ |
| CODE-05 | Код | 🟡 P1 | ~~Дублювання icon mapping~~ ✅ |
| GUI-01 | GUI | 🟡 P1 | ~~gtk4::Window → adw::Dialog~~ ✅ |
| GUI-02 | GUI | 🟡 P1 | Фільтри переповнюють на мобільних |
| GUI-03 | GUI | 🟡 P1 | ~~Sidebar 200px~~ ✅ |
| GUI-04 | GUI | 🟡 P1 | ~~Відсутні accessible names~~ ✅ |
| GUI-05 | GUI | 🟡 P1 | ~~Валідація не анонсується screen readers~~ ✅ |
| EXT-03 | Компоненти | 🟡 P1 | Тільки x86_64 |
| EXT-04 | Компоненти | 🟡 P1 | Немає SystemPackage install method |
| EXT-05 | Компоненти | 🟡 P1 | Немає min version check |
| EXT-06 | Компоненти | 🟡 P1 | Version check timeout 6s |
| FH-01 | Flathub | 🟡 P1 | ~~SPDX ліцензія inconsistent~~ ✅ |
| FH-02 | Flathub | 🟡 P1 | ~~Немає `<translation>`~~ ✅ |
| FH-03 | Flathub | 🟡 P1 | ~~Немає `<recommends>`~~ ✅ |
| FH-04 | Flathub | 🟡 P1 | Немає dark theme screenshots |
| FH-05 | Flathub | 🟡 P1 | Brand colors контраст |
| CLI-02 | CLI | 🟡 P1 | ~~Немає --verbose/--quiet~~ ✅ |
| CLI-03 | CLI | 🟡 P1 | ~~Немає --no-color / NO_COLOR~~ ✅ |
| CLI-04 | CLI | 🟡 P1 | ~~Немає --dry-run~~ ✅ |
| CLI-05 | CLI | 🟡 P1 | ~~Немає shell completions~~ ✅ |
| CLI-06 | CLI | 🟡 P1 | ~~Немає pager~~ ✅ |
| SEC-06 | Безпека | 🟢 P2 | Документація credential lifecycle |
| SEC-07 | Безпека | 🟢 P2 | ~~Property-тести для ін'єкцій~~ ✅ |
| CODE-06 | Код | 🟢 P2 | ~~Мертвий код flatpak.rs~~ ✅ |
| CODE-07 | Код | 🟢 P2 | tracing замість println у CLI |
| CODE-08 | Код | 🟢 P2 | Min version check для CLI |
| GUI-06 | GUI | 🟢 P2 | Split view на мобільних |
| GUI-07 | GUI | 🟢 P2 | Tray polling → event-driven |
| GUI-08 | GUI | 🟢 P2 | Непослідовні відступи |
| GUI-09 | GUI | 🟢 P2 | D&D keyboard alternative |
| GUI-10 | GUI | 🟢 P2 | Навігація по історії пошуку |
| GUI-11 | GUI | 🟢 P2 | `<recommends>` у metainfo |
| EXT-07 | Компоненти | 🟢 P2 | CI для перевірки версій |
| EXT-08 | Компоненти | 🟢 P2 | Кешування client detection |
| FH-06 | Flathub | 🟢 P2 | x-checker-data для modules |
| FH-07 | Flathub | 🟢 P2 | Flatpak extensions |
| CLI-07 | CLI | 🟢 P2 | Auto JSON при pipe |
| CLI-08 | CLI | 🟢 P2 | Fuzzy suggestions |
| CLI-09 | CLI | 🟢 P2 | ~~Confirmation для delete~~ ✅ |
| CLI-10 | CLI | 🟢 P2 | Man pages |

---

**Загальна оцінка (оновлено після v0.8.7):**

| Область | Оцінка | Коментар |
|---------|--------|----------|
| Безпека | 9/10 | SecretString, thiserror, ring, ChecksumPolicy, variable injection prevention, Argon2 strength |
| Якість коду | 9/10 | CLI модуляризований, дублювання усунено, deprecated dead code |
| GNOME HIG | 9/10 | Відмінне використання libadwaita, breakpoints, адаптивність, accessible validation |
| Мобільна підтримка | 7/10 | Базова адаптивність є, потрібні breakpoints для <400sp |
| Зовнішні компоненти | 7/10 | ChecksumPolicy, pinned versions, Flatpak filtering; залишається arch та version check |
| Flathub | 9/10 | Верифіковано, SPDX fixed, recommends/requires додано |
| CLI (clig.dev) | 9/10 | verbose/quiet, no-color, dry-run, completions, pager, confirmation, config |
| Wayland | 9/10 | Wayland-first, немає X11 API, Cairo fallback |
| Тестування | 9/10 | ~2600 тестів, property tests для injection prevention |

**Виконано:** 28/46 задач (61%) — всі P0, більшість P1
**Залишилось:** 18 задач (SEC-05, SEC-06, GUI-02/06-11, EXT-03-08, FH-04-07, CODE-07-08, CLI-07/08/10)
