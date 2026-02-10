# RustConn OBS Packaging

Файли для автоматичної збірки на [Open Build Service](https://build.opensuse.org/).

## Підтримувані дистрибутиви

| Дистрибутив | Версія | Rust джерело | Статус |
|-------------|--------|--------------|--------|
| openSUSE Tumbleweed | Rolling | System (1.88+) | ✅ |
| openSUSE Slowroll | Rolling (slow) | System (1.88+) | ✅ |
| openSUSE Leap | 16.0 | devel:languages:rust | ✅ |
| Fedora | 42+ | System (1.93) | ✅ |

**Примітка:** MSRV (Minimum Supported Rust Version) = 1.88

Ubuntu/Debian не підтримуються в OBS — системний Rust занадто старий:
- Ubuntu 24.04: Rust 1.75, Ubuntu 25.04: Rust 1.84, Ubuntu 25.10: Rust 1.85
- Ubuntu 26.04 (resolute) матиме Rust 1.88 — можна додати після релізу
- Використовуйте GitHub releases для .deb та AppImage пакетів.

## Автоматичне оновлення

При створенні нового релізу на GitHub, workflow автоматично:
1. Оновлює `_service` з новим тегом
2. Копіює `rustconn.changes` та `rustconn.spec`
3. Комітить зміни в OBS
4. Тригерить перезбірку всіх репозиторіїв

### Необхідні секрети GitHub

| Секрет | Опис |
|--------|------|
| `OBS_USERNAME` | Логін на build.opensuse.org |
| `OBS_PASSWORD` | Пароль на build.opensuse.org |

## Структура файлів

| Файл | Призначення |
|------|-------------|
| `_service` | Автоматичне завантаження з Git |
| `_multibuild` | Мультибілд (standard + appimage) |
| `rustconn.spec` | RPM spec для openSUSE/Fedora/RHEL |
| `rustconn.changes` | Changelog для RPM |
| `rustconn.dsc` | Debian source control |
| `debian.*` | Файли для збірки DEB |
| `AppImageBuilder.yml` | Конфігурація для AppImage |

## Залежності для збірки

### RPM (openSUSE)
```
cargo >= 1.87, rust >= 1.87, cargo-packaging
gtk4-devel >= 4.14, vte-devel, libadwaita-devel
alsa-devel, dbus-1-devel, openssl-devel, zstd
```

### RPM (Fedora/RHEL)
```
cargo >= 1.87, rust >= 1.87 (або rustup для старіших версій)
gtk4-devel >= 4.14, vte291-gtk4-devel, libadwaita-devel
alsa-lib-devel, dbus-devel, openssl-devel, zstd
```

### DEB (Ubuntu/Debian)
```
cargo >= 1.87, rustc >= 1.87
libgtk-4-dev >= 4.14, libvte-2.91-gtk4-dev, libadwaita-1-dev
libasound2-dev, libdbus-1-dev, libssl-dev, zstd
```

## Налаштування OBS

### 1. Створення проєкту

```bash
# Встановіть osc
# openSUSE: sudo zypper install osc
# Fedora: sudo dnf install osc

# Checkout проєкту
osc checkout home:totoshko88:rustconn/rustconn
cd home:totoshko88:rustconn/rustconn
```

### 2. Репозиторії для збірки

Рекомендовані репозиторії в OBS:

**RPM:**
- openSUSE_Tumbleweed
- openSUSE_Slowroll
- openSUSE_Leap_16.0
- Fedora_42

**DEB:**
- Debian 13+ (коли Rust >= 1.88 буде доступний)
- xUbuntu_26.04 (коли вийде)

### 3. Оновлення версії

```bash
# 1. Оновіть _service revision на новий тег
sed -i 's/revision>v.*/revision>v0.5.0</' _service

# 2. Оновіть rustconn.changes
# 3. Оновіть debian.changelog

# 4. Запустіть source service
osc service runall

# 5. Commit
osc commit -m "Update to 0.5.0"
```

## Встановлення

### openSUSE Tumbleweed
```bash
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/openSUSE_Tumbleweed/ rustconn
sudo zypper ref
sudo zypper in rustconn
```

### openSUSE Slowroll
```bash
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/openSUSE_Slowroll/ rustconn
sudo zypper ref
sudo zypper in rustconn
```

### openSUSE Leap 16.0
```bash
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/16.0/ rustconn
sudo zypper ref
sudo zypper in rustconn
```

### Fedora 42+
```bash
sudo dnf config-manager --add-repo \
  https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/Fedora_42/home:totoshko88:rustconn.repo
sudo dnf install rustconn
```

### Ubuntu 24.04
```bash
echo "deb https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_24.04/ /" \
  | sudo tee /etc/apt/sources.list.d/rustconn.list
curl -fsSL https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_24.04/Release.key \
  | sudo gpg --dearmor -o /etc/apt/trusted.gpg.d/rustconn.gpg
sudo apt update
sudo apt install rustconn
```

## Корисні команди

```bash
# Перегляд статусу збірки
osc results home:totoshko88:rustconn rustconn

# Перегляд логів
osc buildlog openSUSE_Tumbleweed x86_64

# Локальна тестова збірка
osc build openSUSE_Tumbleweed x86_64

# Перезапуск збірки
osc rebuild home:totoshko88:rustconn rustconn
```

## Troubleshooting

### Rust version too old
Для дистрибутивів зі старою версією Rust, spec автоматично встановлює rustup.
Переконайтесь, що `curl` доступний як BuildRequires.

### ALSA not found
Додайте `alsa-devel` (openSUSE) або `alsa-lib-devel` (Fedora) до BuildRequires.

### GTK4 version mismatch
Потрібен GTK4 >= 4.14. Доступний в:
- openSUSE Tumbleweed
- Fedora 40+
- Ubuntu 24.04+
- Debian 13+

