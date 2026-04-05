# Requirements Document

## Introduction

Terminal Activity Monitor adds per-session activity and inactivity (silence) detection to RustConn terminal tabs, inspired by KDE Konsole. Users can opt in to notifications when a terminal produces new output after a quiet period (activity monitoring) or when a terminal stops producing output for a configurable duration (silence monitoring). Notifications are delivered through tab indicator icons, CSS animations, in-app toasts, and desktop notifications. The feature integrates with the existing `TerminalSession`, `MonitoringCoordinator` pattern, VTE signals, toast system, and `adw::TabPage` indicator API.

GitHub issue: https://github.com/totoshko88/RustConn/issues/72

## Glossary

- **Activity_Monitor**: The subsystem responsible for tracking terminal output events and firing notifications based on the selected monitoring mode.
- **Activity_Mode**: A monitoring mode where the Activity_Monitor watches for new terminal output after a configurable period of silence.
- **Silence_Mode**: A monitoring mode where the Activity_Monitor watches for the absence of terminal output for a configurable duration.
- **Silence_Timeout**: The number of seconds of no terminal output before the Activity_Monitor triggers a silence notification. Default: 30 seconds.
- **Activity_Quiet_Period**: The number of seconds of silence that must elapse before subsequent output triggers an activity notification. Default: 10 seconds.
- **Monitor_Mode_Enum**: A serializable enum (`Off`, `Activity`, `Silence`) stored in `rustconn-core` representing the current monitoring mode for a session or connection.
- **Tab_Indicator**: The icon displayed on an `adw::TabPage` via `set_indicator_icon()`.
- **Toast_System**: The existing `ToastOverlay` in `rustconn/src/toast.rs` used for in-app notifications.
- **Desktop_Notification**: A system-level notification sent via `gio::Notification` when the application window is not focused.
- **Terminal_Session**: The `TerminalSession` struct in `rustconn/src/terminal/types.rs` holding per-session metadata.
- **Activity_Coordinator**: A per-session state manager (following the `MonitoringCoordinator` pattern) that owns timers and state for activity detection.
- **Contents_Changed_Signal**: The VTE `contents_changed` signal already exposed via `TerminalNotebook::connect_contents_changed()`.
