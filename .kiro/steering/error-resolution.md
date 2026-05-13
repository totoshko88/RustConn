---
inclusion: manual
---

# Error Resolution Guide — RustConn

Типові помилки компілятора та їх архітектурно-правильні рішення для цього проекту.
Не давай поверхневих фіксів — шукай root cause.

## Ownership & Borrowing

| Помилка | Поверхневий фікс ❌ | Правильне рішення ✅ |
|---------|---------------------|---------------------|
| E0382 (use after move) | `.clone()` | `Rc<T>` / `Arc<T>` для shared data; передай `&T` якщо ownership не потрібен |
| E0505 (borrow while moved) | `.clone()` перед borrow | Restructure: спочатку borrow, потім move |
| E0502 (mutable + immutable borrow) | `RefCell` скрізь | Розділи на окремі поля або використай take-invoke-restore pattern |
| E0597 (lifetime too short) | `'static` | Передай owned data або використай `Rc`; в GTK callbacks — `clone!` macro |

## RefCell / Rc Patterns (GTK4 специфіка)

| Проблема | Рішення |
|----------|---------|
| `BorrowMutError` в runtime | **take-invoke-restore**: `let val = state.borrow_mut().field.take(); ...; state.borrow_mut().field = val;` |
| Borrow через async boundary | Клонуй `Rc` перед `spawn_local`, не тримай `Ref`/`RefMut` через `.await` |
| Circular Rc references | `Rc::new_cyclic` або `Weak<T>` для зворотних посилань |

## Async / Tokio

| Помилка | Рішення |
|---------|---------|
| "Cannot start runtime within runtime" | Використай `with_runtime()` helper — thread-local Runtime |
| `Send` bound not satisfied | GTK об'єкти не Send — використай `spawn_local` або channel pattern |
| Timeout на vault operations | Завжди `tokio::time::timeout(Duration::from_secs(10), ...)` |

## Clippy Lints

| Lint | Рішення |
|------|---------|
| `cognitive_complexity` | Виділи inner logic в окрему fn; для GTK — builder pattern |
| `too_many_arguments` (>7) | Створи struct: `struct ConnectionParams { ... }` |
| `missing_errors_doc` | Додай `/// # Errors\n/// Returns error if...` |
| `significant_drop_tightening` | Явно `drop(guard)` перед наступною операцією |
| `option_if_let_else` (allowed) | Ігноруй — дозволено в .clippy.toml |

## thiserror Patterns

| Ситуація | Pattern |
|----------|---------|
| Wrapping std::io::Error | `#[error("operation failed: {0}")] Io(#[from] std::io::Error)` |
| Wrapping з контекстом | `#[error("failed to {action}: {source}")] WithContext { action: String, source: Box<dyn std::error::Error + Send + Sync> }` |
| Enum → Display для UI | Implement `display_name()` → wrap в `i18n()` at call site |

## SecretString Patterns

| Ситуація | Pattern |
|----------|---------|
| Отримати пароль з UI | `SecretString::new(entry.text().to_string().into())` |
| Передати в CLI | `cmd.stdin(Stdio::piped()); child.stdin.write_all(secret.expose_secret().as_bytes())` |
| Тимчасова String | `let tmp = Zeroizing::new(secret.expose_secret().to_string()); use tmp; // auto-zeroize on drop` |
| Порівняння | `secret1.expose_secret() == secret2.expose_secret()` (в scope де обидва доступні) |

## GTK4 / libadwaita

| Проблема | Рішення |
|----------|---------|
| Widget not showing | Перевір `.set_visible(true)` та що parent має `child`/`append` |
| Signal handler memory leak | `connect_*` з `clone!(@weak self as this =>` |
| Dialog не закривається | `dialog.close()` або `dialog.set_visible(false)` + `dialog.destroy()` |
| Wayland: no window position | Не використовуй `set_position` — Wayland не підтримує |
