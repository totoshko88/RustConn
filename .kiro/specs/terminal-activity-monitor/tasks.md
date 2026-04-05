# Tasks: Terminal Activity Monitor

## Task 1: Core data models in rustconn-core

- [x] 1.1 Create `rustconn-core/src/activity_monitor.rs` with `MonitorMode` enum (Off/Activity/Silence), `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]`, `next()` cycle method, `icon_name()`, `display_name()`
- [x] 1.2 Add `ActivityMonitorDefaults` struct with `mode: MonitorMode`, `quiet_period_secs: u32` (default 10), `silence_timeout_secs: u32` (default 30), `Default` impl, and `effective_quiet_period()` / `effective_silence_timeout()` clamping methods
- [x] 1.3 Add `ActivityMonitorConfig` struct with `Option<MonitorMode>`, `Option<u32>`, `Option<u32>` fields, serde skip_serializing_if, and `effective_mode()` / `effective_quiet_period()` / `effective_silence_timeout()` resolution methods taking `&ActivityMonitorDefaults`
- [x] 1.4 Register module in `rustconn-core/src/lib.rs` and re-export public types
- [x] 1.5 Add `pub activity_monitor_config: Option<ActivityMonitorConfig>` field to `Connection` struct with `#[serde(default, skip_serializing_if = "Option::is_none")]`
- [x] 1.6 Add `pub activity_monitor: ActivityMonitorDefaults` field to `AppSettings` with `#[serde(default)]`

## Task 2: Property-based tests

- [x] 2.1 Create `rustconn-core/tests/properties/activity_monitor_tests.rs` with proptest strategies for `MonitorMode`, `ActivityMonitorConfig`, `ActivityMonitorDefaults`
- [x] 2.2 Property test: mode cycling is a 3-cycle (Property 5)
- [x] 2.3 Property test: serialization round-trip for `ActivityMonitorConfig` (Property 6)
- [x] 2.4 Property test: config resolution prefers per-connection overrides (Property 7)
- [x] 2.5 Property test: timeout clamping for quiet_period and silence_timeout (Property 8)
- [x] 2.6 Property test: Off mode suppresses all notifications — `effective_mode` returns Off when mode=Off regardless of other config (Property 3)
- [x] 2.7 Register test module in `rustconn-core/tests/properties/mod.rs`

## Task 3: ActivityCoordinator in rustconn

- [x] 3.1 Create `rustconn/src/activity_coordinator.rs` with `ActivityCoordinator` struct, `SessionActivityState`, `NotificationType` enum
- [x] 3.2 Implement `new()`, `start()`, `stop()`, `stop_all()` lifecycle methods following `MonitoringCoordinator` pattern
- [x] 3.3 Implement `on_output()` — activity mode: check elapsed >= quiet_period, silence mode: reset glib timer, off mode: no-op. Returns `Option<NotificationType>`
- [x] 3.4 Implement `on_tab_switched()` — clears `notification_active` flag for the session
- [x] 3.5 Implement `cycle_mode()` and `set_mode()` / `get_mode()` for runtime mode changes
- [x] 3.6 Implement silence timer using `glib::timeout_add_local_once` — fires silence notification callback when timer expires
- [x] 3.7 Register module in `rustconn/src/lib.rs` or `rustconn/src/main.rs`

## Task 4: Wire VTE signals and notification delivery

- [x] 4.1 In session setup code, call `ActivityCoordinator::start()` with resolved config (per-connection or global defaults) after terminal is created
- [x] 4.2 Connect `TerminalNotebook::connect_contents_changed()` to `ActivityCoordinator::on_output()` for each SSH session
- [x] 4.3 On notification fire: set `adw::TabPage` indicator icon via `set_indicator_icon()`
- [x] 4.4 On notification fire: show toast via existing `ToastOverlay` with `i18n_f()` message
- [x] 4.5 On notification fire (window unfocused): send `gio::Notification` via `app.send_notification()`
- [x] 4.6 On tab switch (`TabView::connect_selected_page_notify`): call `ActivityCoordinator::on_tab_switched()` to clear indicator
- [x] 4.7 On session close (`connect_child_exited`): call `ActivityCoordinator::stop()` to clean up timers

## Task 5: Connection dialog UI

- [x] 5.1 Add "Activity Monitor" `adw::PreferencesGroup` to the connection dialog advanced tab with `adw::ComboRow` for mode (Off/Activity/Silence), `adw::SpinRow` for quiet period (1–300), `adw::SpinRow` for silence timeout (1–600)
- [x] 5.2 Wire sensitivity: quiet period spin visible only when mode=Activity, silence timeout spin visible only when mode=Silence
- [x] 5.3 Load `ActivityMonitorConfig` from `Connection` into UI on dialog open
- [x] 5.4 Collect `ActivityMonitorConfig` from UI on dialog save and store on `Connection`
- [x] 5.5 Use `i18n()` for all labels and descriptions

## Task 6: Settings dialog UI

- [x] 6.1 Add "Activity Monitor" `adw::PreferencesGroup` to the monitoring settings tab (or new tab) with default mode combo, quiet period spin, silence timeout spin
- [x] 6.2 Load `ActivityMonitorDefaults` from `AppSettings` into UI
- [x] 6.3 Collect `ActivityMonitorDefaults` from UI and save to `AppSettings`
- [x] 6.4 Use `i18n()` for all labels and descriptions

## Task 7: Tab context menu

- [x] 7.1 Add "Monitor Activity" action to the tab context menu with current mode displayed
- [x] 7.2 On action activation, call `ActivityCoordinator::cycle_mode()` and update the action label to reflect new mode
- [x] 7.3 Use `i18n()` for menu item labels

## Task 8: CSS styles

- [x] 8.1 Add `.tab-activity-indicator` and `.tab-silence-indicator` CSS classes to `rustconn/assets/style.css` for optional visual distinction on tab indicators

## Task 9: i18n strings

- [x] 9.1 Run `po/update-pot.sh` to extract new translatable strings into `rustconn.pot`
