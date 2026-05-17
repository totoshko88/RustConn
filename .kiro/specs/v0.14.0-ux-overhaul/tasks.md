# v0.14.0 — UX Overhaul: План реалізації

## Статус трекінг

| # | Задача | Статус | Файли |
|---|--------|--------|-------|
| 1 | Connection Wizard — каркас | ✅ Done | `connection_wizard/mod.rs` |
| 2 | Wizard Step 1 — Protocol Selection | ✅ Done | `connection_wizard/protocol_page.rs` |
| 3 | Wizard Step 2 — Connection Details | ✅ Done | `connection_wizard/connection_page.rs` |
| 4 | Wizard Step 3 — Auth + Finish | ✅ Done | `connection_wizard/auth_page.rs` |
| 5 | Wizard — валідація та навігація | ✅ Done | `connection_wizard/mod.rs` |
| 6 | Wizard — інтеграція (actions) | ✅ Done | `window/mod.rs` |
| 7 | Quick Connect — runtime history | ✅ Done | `window/edit_dialogs.rs`, `window/types.rs` |
| 8 | Highlight Rules — ExpanderRow | ✅ Done | `dialogs/settings/mod.rs` |


---

## Архітектурні рішення (затверджено)

### Connection Wizard

- **Віджет:** `adw::Window` (modal) + `adw::NavigationView` (3 сторінки)
- **Протоколи:** Всі 11, логічно згруповані на Step 1
- **«Advanced...»:** Кнопка внизу ліворуч на КОЖНОМУ кроці → повний діалог
- **Завершення:** «Save» (secondary) + «Save & Connect» (suggested-action)
- **Ctrl+N:** Wizard; Ctrl+Shift+N → повний діалог
- **Jump Host:** Dropdown на Step 2 для SSH/MOSH/SFTP/RDP/VNC/SPICE
- **Color Profile:** Вибір кольору терміналу на Step 3 для VTE-протоколів

### Quick Connect History

- **Зберігання:** Runtime only (до закриття програми)
- **Протоколи:** SSH, RDP, VNC, Telnet (поточні в Quick Connect)
- **Кількість:** Максимум 15 записів (LIFO)

---

## Задача 1: Connection Wizard — каркас

### Структура модуля

```
rustconn/src/dialogs/connection_wizard/
├── mod.rs              // ConnectionWizard struct, навігація, збірка
├── protocol_page.rs    // Step 1: вибір протоколу
├── connection_page.rs  // Step 2: хост/порт/ім'я (адаптивно)
└── auth_page.rs        // Step 3: auth + color profile + finish
```

### ConnectionWizard API

```rust
pub struct ConnectionWizard {
    window: adw::Window,
    nav_view: adw::NavigationView,
    protocol_page: ProtocolPage,
    connection_page: ConnectionPage,
    auth_page: AuthPage,
    selected_protocol: Rc<RefCell<Option<ProtocolType>>>,
    on_complete: ConnectionWizardCallback,
}

pub enum WizardResult {
    Save(Connection),
    SaveAndConnect(Connection),
    OpenAdvanced(PartialConnection),
}

impl ConnectionWizard {
    pub fn new(parent: Option<&gtk4::Window>, state: SharedAppState) -> Self;
    pub fn present(&self);
}
```

### Розмір вікна: 500×520, `adw::Clamp` max 450px

---

## Задача 2: Wizard Step 1 — Protocol Selection

### Групування протоколів

```
┌─────────────────────────────────────────────────┐
│  New Connection                                  │
│                                                  │
│  ── Secure Shell ──────────────────────────────  │
│  [SSH]  [MOSH]  [SFTP]                          │
│                                                  │
│  ── Remote Desktop ────────────────────────────  │
│  [RDP]  [VNC]  [SPICE]                          │
│                                                  │
│  ── Terminal ──────────────────────────────────  │
│  [Telnet]  [Serial]                             │
│                                                  │
│  ── Cloud & Containers ────────────────────────  │
│  [Kubernetes]  [Zero Trust]                     │
│                                                  │
│  ── Other ─────────────────────────────────────  │
│  [Web]                                           │
│                                                  │
│  [Advanced...]                                   │
└─────────────────────────────────────────────────┘
```

### Логіка групування

| Група | Протоколи | Обґрунтування |
|-------|-----------|---------------|
| Secure Shell | SSH, MOSH, SFTP | Всі базуються на SSH |
| Remote Desktop | RDP, VNC, SPICE | Графічні протоколи |
| Terminal | Telnet, Serial | Прості текстові з'єднання |
| Cloud & Containers | Kubernetes, Zero Trust | Хмарні/контейнерні |
| Other | Web | Не є з'єднанням у класичному сенсі |

### Іконки

| Протокол | Іконка | Subtitle |
|----------|--------|----------|
| SSH | `utilities-terminal-symbolic` | Secure remote shell |
| MOSH | `network-cellular-signal-excellent-symbolic` | Mobile shell (roaming) |
| SFTP | `folder-remote-symbolic` | SSH file transfer |
| RDP | `computer-symbolic` | Windows Remote Desktop |
| VNC | `preferences-desktop-remote-desktop-symbolic` | Virtual Network Computing |
| SPICE | `video-display-symbolic` | Virtual machine display |
| Telnet | `network-wired-symbolic` | Unencrypted terminal |
| Serial | `media-removable-symbolic` | Serial console |
| Kubernetes | `application-x-executable-symbolic` | Pod shell (kubectl) |
| Zero Trust | `channel-secure-symbolic` | Cloud secure access |
| Web | `web-browser-symbolic` | Open URL in browser |

### Реалізація

- Кожна група = `adw::PreferencesGroup` з title
- Кожен протокол = `gtk4::Button` (flat) в `gtk4::FlowBox`
- Кнопка: іконка зверху + назва знизу, розмір ~90x80
- Клік → `nav_view.push()` на Step 2 з обраним протоколом
- «Advanced...» — `gtk4::Button` (flat, dim-label) внизу ліворуч

---

## Задача 3: Wizard Step 2 — Connection Details

### Адаптивні поля по протоколах

Сторінка перебудовується при переході з Step 1.
Всі поля через `adw::EntryRow` / `adw::SpinRow` / `adw::ComboRow`.

#### Secure Shell група (SSH, MOSH, SFTP)

| Поле | Тип | Default | Обов'язкове |
|------|-----|---------|-------------|
| Name | EntryRow | auto: "{protocol}: {host}" | Ні (auto) |
| Host | EntryRow | — | Так |
| Port | SpinRow | 22 | Так |
| Username | EntryRow | placeholder: поточний $USER | Ні |
| Jump Host | ComboRow | (None) + список SSH з'єднань | Ні |

Для MOSH — subtitle: _«Uses SSH for handshake, then UDP»_
Для SFTP — subtitle: _«File browser over SSH»_

#### Remote Desktop (RDP, VNC, SPICE)

| Поле | Тип | Default | Обов'язкове |
|------|-----|---------|-------------|
| Name | EntryRow | auto | Ні |
| Host | EntryRow | — | Так |
| Port | SpinRow | 3389/5900/5900 | Так |
| Username | EntryRow (RDP only) | — | Ні |
| Domain | EntryRow (RDP only) | — | Ні |
| Jump Host | ComboRow | (None) + SSH з'єднання | Ні |

#### Terminal (Telnet, Serial)

**Telnet:** Name, Host, Port(23)
**Serial:** Name, Device(/dev/ttyUSB0), Baud Rate(ComboRow: 115200)

#### Cloud & Containers

**Kubernetes:** Name, Context(opt), Namespace, Pod, Container(opt)

**Zero Trust** — Provider ComboRow (порядок):
1. **Custom Command** — _«Run any command for connection»_
2. AWS SSM → Target ID, Region, Profile(opt)
3. GCP IAP → Instance, Zone, Project
4. Azure Bastion → Resource ID, RG, Name
5. Azure SSH → VM Name, RG
6. Cloudflare → Hostname
7. Teleport → Host, Cluster(opt)
8. Tailscale → Host
9. Boundary → Target ID, Address(opt)
10. Hoop.dev → Connection Name, Gateway(opt)

#### Other: Web — Name, URL

### Jump Host (спільний для SSH/MOSH/SFTP/RDP/VNC/SPICE)

ComboRow з усіма SSH з'єднаннями зі state. Перший = "(None)".

### Валідація: Host не порожній, Port 1-65535, Next неактивна поки invalid.

---

## Задача 4: Wizard Step 3 — Authentication + Color Profile + Finish

### Які протоколи мають auth step

| Протокол | Auth | Color Profile |
|----------|------|---------------|
| SSH | Password / Key File / SSH Agent | ✅ (VTE) |
| MOSH | Password / Key File / SSH Agent | ✅ (VTE) |
| SFTP | Password / Key File / SSH Agent | ✅ (VTE) |
| RDP | Password (+ Domain) | ❌ (embedded) |
| VNC | Password | ❌ (embedded) |
| SPICE | Password (optional) | ❌ (embedded) |
| Telnet | — (skip auth) | ✅ (VTE) |
| Serial | — (skip auth) | ✅ (VTE) |
| Kubernetes | — (kubectl handles) | ✅ (VTE) |
| Zero Trust | — (provider handles) | ✅ (VTE) |
| Web | — (skip all) | ❌ |

### VTE Color Profile

Простий вибір для візуального розрізнення з'єднань.
`adw::PreferencesGroup` "Appearance":

1. **Theme** — `adw::ComboRow`: Default + theme_names()
2. **Custom colors** (ExpanderRow, optional):
   - Background: `ColorDialogButton`
   - Foreground: `ColorDialogButton`

Маппінг на `ConnectionThemeOverride` (вже є в моделі).

### Layout: Auth (SSH)

```
┌─────────────────────────────────────────────────┐
│  ← Back       Authentication                    │
│                                                  │
│  ── Method ────────────────────────────────────  │
│  │ ○ Password  ○ Key File  ○ SSH Agent       │  │
│  ──────────────────────────────────────────────  │
│  ── Credentials ───────────────────────────────  │
│  │ Password: [••••••••]                    👁 │  │
│  ──────────────────────────────────────────────  │
│  ── Appearance ────────────────────────────────  │
│  │ Theme:  [Default            ▾]            │  │
│  ──────────────────────────────────────────────  │
│                                                  │
│  [Advanced...]          [Save]  [Save & Connect] │
└─────────────────────────────────────────────────┘
```

### Layout: No auth, VTE (Telnet/K8s/ZeroTrust)

Summary + Appearance + buttons.

### Layout: Web (no auth, no color)

Summary + buttons only.

### Кнопки footer

- **Advanced...** — ліворуч, flat, dim-label
- **Save** — справа, secondary
- **Save & Connect** — крайня справа, suggested-action

---

## Задача 5: Wizard — валідація та навігація

### Навігація між кроками

- Step 1 → Step 2: автоматично при кліку на протокол
- Step 2 → Step 3: кнопка «Next» (активна тільки при valid)
- Step 3 → завершення: «Save» або «Save & Connect»
- Back: стандартна навігація NavigationView
- Advanced: збирає поля → відкриває ConnectionDialog → закриває Wizard

### PartialConnection

```rust
pub struct PartialConnection {
    pub protocol: ProtocolType,
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub domain: Option<String>,
    pub auth_method: Option<SshAuthMethod>,
    pub key_path: Option<PathBuf>,
    pub jump_host_id: Option<Uuid>,
    pub theme_override: Option<ConnectionThemeOverride>,
    pub zt_provider: Option<ZeroTrustProvider>,
    pub zt_command: Option<String>,
    pub serial_device: Option<String>,
    pub serial_baud: Option<u32>,
    pub k8s_context: Option<String>,
    pub k8s_namespace: Option<String>,
    pub k8s_pod: Option<String>,
    pub k8s_container: Option<String>,
    pub url: Option<String>,
}
```

### Auto-name: "{Protocol}: {host/device/pod/domain}"

---

## Задача 6: Wizard — інтеграція (actions, shortcuts, menu)

### Keyboard shortcuts

| Shortcut | Action | Результат |
|----------|--------|-----------|
| Ctrl+N | `win.new-connection` | Connection Wizard |
| Ctrl+Shift+N | `win.new-connection-advanced` | Full ConnectionDialog |

### Hamburger menu

- «New Connection» (Ctrl+N) → Wizard
- «New Connection (Advanced)...» (Ctrl+Shift+N) → Full dialog

### Обробка WizardResult

- Save → add_connection + refresh sidebar
- SaveAndConnect → add + refresh + connect
- OpenAdvanced → open ConnectionDialog pre-filled + close wizard

### Реєстрація: `pub mod connection_wizard;` в `dialogs/mod.rs`

---

## Задача 7: Quick Connect — runtime history

### Runtime Vec (не серіалізується), max 15, LIFO
### Протоколи: SSH, RDP, VNC, Telnet
### UI: ListBox під Host field, фільтрація при введенні
### Збереження при успішному підключенні

---

## Задача 8: Highlight Rules — ExpanderRow

Обгорнути в `adw::ExpanderRow`, згорнуто за замовчуванням, ~20 рядків.

---

## Залежності

1→2→3→4→5→6 (послідовно)
7, 8 — незалежні (паралельно)

## Критерії готовності

- [ ] cargo clippy 0 warnings
- [ ] cargo fmt clean
- [ ] i18n() на всіх user-facing strings
- [ ] Accessible labels на icon-only buttons
- [ ] po/update-pot.sh після нових рядків
- [ ] Wizard для всіх 11 протоколів
- [ ] Advanced передає дані коректно
- [ ] Jump Host для SSH/MOSH/SFTP/RDP/VNC/SPICE
- [ ] Color Profile → ConnectionThemeOverride
- [ ] Ctrl+N → Wizard, Ctrl+Shift+N → full dialog
- [ ] Quick Connect history з фільтрацією
