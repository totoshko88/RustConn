# План рефакторингу RustConn GUI

## Поточний стан (після рефакторингу embedded_rdp.rs)

| Файл | Рядків | Статус |
|------|--------|--------|
| `window.rs` | 5478 | ✅ Частково рефакторено (було 7283) |
| `embedded_rdp.rs` | 3366 | ✅ Частково рефакторено (було 4234) |
| `sidebar.rs` | 2787 | ⚠️ Потребує рефакторингу |
| `embedded_vnc.rs` | 2304 | ⚠️ Потребує рефакторингу |
| `state.rs` | 1772 | ✅ Менше 2000 рядків |
| `terminal.rs` | 1571 | ✅ Менше 2000 рядків |
| `split_view.rs` | 1459 | ✅ Менше 2000 рядків |
| `wayland_surface.rs` | 1128 | ✅ Менше 2000 рядків |
| `embedded_spice.rs` | 1079 | ✅ Менше 2000 рядків |

## Виконано

### window.rs (7283 → 5478 рядків, -25%)
- ✅ `window_types.rs` (51 рядків) - Type aliases, `get_protocol_string()`
- ✅ `window_snippets.rs` (512 рядків) - Snippet management
- ✅ `window_templates.rs` (397 рядків) - Template management
- ✅ `window_sessions.rs` (402 рядків) - Session management
- ✅ `window_groups.rs` (178 рядків) - Group management dialogs
- ✅ `window_clusters.rs` (355 рядків) - Cluster management

### embedded_rdp.rs (4234 → 3366 рядків, -20%)
- ✅ `embedded_rdp_types.rs` (403 рядків) - Error types, enums, config, callbacks
- ✅ `embedded_rdp_buffer.rs` (272 рядків) - PixelBuffer, WaylandSurfaceHandle
- ✅ `embedded_rdp_launcher.rs` (228 рядків) - SafeFreeRdpLauncher

## Наступні кроки

### Пріоритет 1: sidebar.rs (2787 рядків)

Структура для виділення:
- `sidebar_types.rs` - типи та ConnectionItem
- `sidebar_tree.rs` - побудова дерева
- `sidebar_dnd.rs` - drag and drop
- `sidebar_context_menu.rs` - контекстне меню
- `sidebar_search.rs` - пошук та фільтрація

**Очікуване зменшення:** ~1000 рядків → 1800 рядків

### Пріоритет 2: embedded_vnc.rs (2304 рядків)

Структура для виділення:
- `embedded_vnc_types.rs` - типи та константи
- `embedded_vnc_config.rs` - VncConfig
- `embedded_vnc_input.rs` - обробка вводу

**Очікуване зменшення:** ~500 рядків → 1800 рядків

### Пріоритет 3: window.rs (5478 рядків) - продовження

Можна виділити ще:
- `window_connections.rs` - методи new/edit/delete/duplicate connection (~500 рядків)
- `window_import_export.rs` - import/export dialogs (~400 рядків)
- `window_split_view.rs` - split view actions (~200 рядків)
- `window_quick_connect.rs` - quick connect dialog (~150 рядків)

**Очікуване зменшення:** ~1250 рядків → 4200 рядків

### Пріоритет 4: embedded_rdp.rs (3366 рядків) - продовження

Можна виділити ще:
- `embedded_rdp_thread.rs` - FreeRdpThread (~300 рядків)
- `embedded_rdp_clipboard.rs` - ClipboardFileTransfer (~200 рядків)

**Очікуване зменшення:** ~500 рядків → 2800 рядків

## Загальні принципи рефакторингу

1. **Standalone функції** замість `impl MainWindow` методів де можливо
2. **Type aliases** виносити в окремий `*_types.rs` модуль
3. **Публічні методи** робити `pub` тільки якщо потрібно з інших модулів
4. **Тестувати** після кожного виділення модуля

## Порядок виконання

1. [x] `window.rs` → виділити snippets, templates, sessions, groups, clusters
2. [x] `embedded_rdp.rs` → виділити types, buffer, launcher
3. [ ] `sidebar.rs` → виділити types, tree, dnd
4. [ ] `embedded_vnc.rs` → виділити types, config
5. [ ] `window.rs` → виділити connections, import/export (опціонально)
6. [ ] `embedded_rdp.rs` → виділити thread, clipboard (опціонально)

## Метрики успіху

- Жоден файл не повинен перевищувати 2000 рядків
- Кожен модуль має одну відповідальність
- Компіляція та clippy без помилок після кожного кроку

## Прогрес

- **window.rs**: 7283 → 5478 рядків (-1805, -25%)
- **embedded_rdp.rs**: 4234 → 3366 рядків (-868, -20%)
- **Загалом виділено**: 2673 рядків у 9 нових модулів
