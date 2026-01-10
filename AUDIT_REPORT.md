# RustConn Audit & Modernization Report

**Date:** January 10, 2026  
**Project:** RustConn - Linux Connection Manager  
**Version:** 0.5.9  
**Auditor:** Kiro AI Assistant

---

## Executive Summary

RustConn is a well-structured GTK4/libadwaita connection manager with solid architectural foundations. The codebase demonstrates good adherence to Rust idioms and GNOME HIG patterns.

**Overall Health: 🟢 Good**

| Category | Status | Priority Items |
|----------|--------|----------------|
| Code Quality | 🟢 Good | ✅ Clippy suppressions reduced, ✅ `unwrap()` fixed in Cairo code |
| GTK4/Adwaita Migration | 🟢 Complete | ✅ Dialogs migrated to `adw::Window` |
| Crate Boundaries | 🟢 OK | No violations found |
| Dependencies | 🟡 Moderate | Some outdated crates, pinned versions |
| Technical Debt | 🟡 Moderate | Large files, dead code markers |

---

## Section 1: Rust Code Improvements

### ✅ RESOLVED: Crate Boundary Violation

**Status:** No GTK code found in `rustconn-core`. The crate boundaries are properly maintained.

---

### ✅ RESOLVED: Clippy Lint Suppressions

**Location:** `rustconn/src/main.rs`

Global clippy suppressions reduced from 30+ to 5 essential ones:
- `clippy::too_many_lines` - GUI code often has long setup functions
- `clippy::type_complexity` - GTK callback types are inherently complex
- `clippy::cast_possible_truncation` - Intentional casts in GUI code
- `clippy::cast_sign_loss` - Intentional casts in GUI code
- `clippy::cast_possible_wrap` - Intentional casts in GUI code

---

### ✅ RESOLVED: `unwrap()` Usage in Cairo Drawing Code

Fixed `unwrap()` calls in Cairo drawing code with `if let Ok(extents)` pattern:
- `embedded_rdp_ui.rs` - Cairo text extents
- `embedded_vnc.rs` - Cairo text extents
- `embedded.rs` - Cairo text extents

---

### 🟡 HIGH: `block_on()` in GUI Code

**Location:** `rustconn/src/state.rs` (lines 824, 845, 866, 978)

Blocking async operations in GUI thread can cause UI freezes:

```rust
// CURRENT — Blocks GUI thread
with_runtime(|rt| {
    rt.block_on(async {
        secret_manager.store(&id_str, &creds).await
    })
})
```

**Recommended:** Use `glib::spawn_future_local()` for async operations:
```rust
// RECOMMENDED — Non-blocking
glib::spawn_future_local(async move {
    if let Err(e) = secret_manager.store(&id_str, &creds).await {
        // Handle error via channel or callback
    }
});
```

---

### 🟢 LOW: Large Files Should Be Split

| File | Lines | Recommendation |
|------|-------|----------------|
| `dialogs/connection.rs` | 6,459 | Split into `connection_basic.rs`, `connection_protocol.rs`, `connection_automation.rs` |
| `window.rs` | 2,965 | Already split via `window_*.rs` pattern — good |
| `state.rs` | 2,043 | Consider extracting credential operations to `state_credentials.rs` |
| `sidebar.rs` | 1,900+ | Consider extracting filter logic to `sidebar_filter.rs` |

---

### 🟢 LOW: Dead Code Markers

Numerous `#[allow(dead_code)]` annotations suggest API surface area that may need pruning:

```rust
// Example from window.rs
#[allow(dead_code)] // Part of KeePass integration API
pub fn refresh_keepass_status(&self) { ... }
```

**Recommendation:** Audit each `dead_code` annotation:
- If truly unused → remove the code
- If part of public API → document why it exists
- If for future use → add `// TODO:` comment with timeline

---

## Section 2: UI/UX Modernization

### ✅ Adwaita Migration Status: Complete

The project correctly uses modern libadwaita patterns:

| Pattern | Status | Notes |
|---------|--------|-------|
| `adw::Application` | ✅ Used | `app.rs:47` |
| `adw::ApplicationWindow` | ✅ Used | Main window |
| `adw::PreferencesWindow` | ✅ Used | Settings dialog |
| `adw::PreferencesPage/Group` | ✅ Used | Throughout dialogs |
| `adw::ToastOverlay` | ✅ Used | Toast notifications |
| `adw::StatusPage` | ✅ Used | Empty states |
| `adw::HeaderBar` | ✅ Used | Main header |
| `adw::ViewStack/Switcher` | ✅ Used | Connection dialog tabs |
| `adw::StyleManager` | ✅ Used | Color scheme handling |
| `adw::Window` | ✅ Used | All modal dialogs |

---

### ✅ RESOLVED: Legacy `gtk4::Window` Usage

All dialogs migrated from `gtk4::Window` to `adw::Window` with `adw::HeaderBar`:
- `window_sessions.rs` - Sessions manager and send text dialogs
- `window_operations.rs` - Delete and move dialogs
- `window_groups.rs` - Move to group dialog
- `window_snippets.rs` - Snippets manager, picker, variable input dialogs
- `error_display.rs` - Error dialog
- `dashboard.rs` - Connection dashboard dialog

---

### ✅ RESOLVED: Hardcoded Colors

Replaced hardcoded RGB values with Adwaita semantic colors:
- `sidebar_types.rs` - Status colors now use `@success_color`, `@warning_color`, `@error_color`
- `app.rs` - Status indicators now use semantic colors

---

### 🟢 LOW: About Dialog Could Use `adw::AboutDialog`

**Location:** `app.rs:654`

~~Currently uses `gtk4::AboutDialog`. Consider `adw::AboutDialog` for better integration (available in libadwaita 1.5+, but current version is 0.8).~~

**Status:** ✅ RESOLVED - Migrated to `adw::AboutDialog` with modern features (issue URL, copyright, application icon).

---

## Section 3: Dependency Matrix

| Crate | Current | Latest* | Status | Notes |
|-------|---------|---------|--------|-------|
| `tokio` | 1.49 | 1.49 | ✅ Current | |
| `gtk4` | 0.10.2 | 0.10.x | ✅ Current | Matches GTK 4.14 |
| `libadwaita` | 0.8 (v1_6) | 0.8.x | ✅ Current | Full feature set enabled |
| `vte4` | 0.9 | 0.9.x | ✅ Current | |
| `serde` | 1.0 | 1.0.x | ✅ Current | |
| `thiserror` | 2.0.17 | 2.0.x | ✅ Current | |
| `uuid` | 1.11 | 1.11.x | ✅ Updated | |
| `chrono` | 0.4 | 0.4.x | ✅ Current | |
| `clap` | 4.5.23 | 4.5.x | ✅ Current | |
| `regex` | 1.11 | 1.11.x | ✅ Updated | |
| `ring` | 0.17 | 0.17.x | ✅ Current | |
| `argon2` | 0.5 | 0.5.x | ✅ Current | |
| `secrecy` | 0.10 | 0.10.x | ✅ Current | |
| `dirs` | 6.0 | 6.0.x | ✅ Current | |
| `quick-xml` | 0.38 | 0.38.x | ✅ Current | |
| `notify` | 8.2 | 8.x | ✅ Current | Removed macOS feature |
| `ironrdp` | 0.13 | 0.13.x | ✅ Current | |
| `picky` | =7.0.0-rc.17 | 7.0.0+ | 🔴 Pinned | sspi/rand_core conflict |
| `vnc-rs` | 0.5 | 0.5.x | ✅ Current | |
| `spice-client` | 0.2.0 | 0.2.x | ✅ Current | |
| `ksni` | 0.3 | 0.3.x | 🟡 Blocked | zvariant version conflict |
| `resvg` | 0.45 | 0.45.x | ✅ Current | |
| `cpal` | 0.17 | 0.17.x | ✅ Current | |
| `tempfile` | 3.15 | 3.x | ✅ Updated | |
| `proptest` | 1.6 | 1.6.x | ✅ Updated | |
| `criterion` | 0.8 | 0.8.x | ✅ Current | |
| `zip` | 2.2 | 2.2.x | ✅ Updated | |

*Latest versions as of January 2026

### Dependency Issues

#### 🔴 Pinned `picky` Version
```toml
# CURRENT — Pinned due to conflict
picky = "=7.0.0-rc.17"
```

**Issue:** The `sspi` crate (used by `ironrdp`) has a `rand_core` version conflict with newer `picky` releases.

**Recommendation:** Monitor `ironrdp` releases for resolution. Document the pin with a `MONITORING` comment (already done ✅).

#### 🟡 `ksni` Version Blocked

The `ksni` 0.3.3 crate has a `zvariant` version conflict with newer `zbus` versions. This prevents running `cargo update` without causing build failures.

**Recommendation:** Monitor `ksni` releases for `zvariant` 5.x compatibility.

#### ✅ RESOLVED: Feature Bloat Check

The `notify` crate macOS feature has been removed:
```toml
# UPDATED
notify = { version = "8.2", default-features = false }
```

#### ✅ RESOLVED: `PreferencesWindow` Deprecation

Migrated Settings dialog from deprecated `PreferencesWindow` to `PreferencesDialog` (libadwaita 1.5+):
- Changed `adw::PreferencesWindow` to `adw::PreferencesDialog`
- Updated `run()` method to accept parent widget parameter
- Changed close handler from `connect_close_request` to `connect_closed`
- Updated toast notification handling in SSH Agent tab

---

## Section 4: Action Plan

### ✅ Phase 1: Critical Fixes (COMPLETED)

1. ~~**Delete misplaced GTK code in rustconn-core**~~ - No violation found
2. ✅ **Audit `unwrap()` in Cairo drawing code** - Fixed with `if let Ok(...)` pattern

### ✅ Phase 2: High Priority (COMPLETED)

3. ✅ **Reduce global clippy suppressions** - Reduced from 30+ to 5 essential ones
4. ✅ **Replace `gtk4::Window` with `adw::Window`** - All 9 dialog files updated
5. ✅ **Convert hardcoded colors to semantic** - Updated CSS in `app.rs` and `sidebar_types.rs`

### ✅ Phase 3: Dependency Updates (COMPLETED)

6. ✅ **Update workspace dependencies**
   - `uuid` 1.6 → 1.11
   - `regex` 1.10 → 1.11
   - `proptest` 1.4 → 1.6
   - `tempfile` 3.24 → 3.15
   - `zip` 2.1 → 2.2
   - `libadwaita` feature `v1_4` → `v1_6`

7. ✅ **Remove unnecessary features**
   - Removed `macos_kqueue` from `notify` crate

### ✅ Phase 4: libadwaita 1.6 Migration (COMPLETED)

8. ✅ **Migrate `PreferencesWindow` to `PreferencesDialog`**
   - Settings dialog now uses modern `adw::PreferencesDialog`
   - Updated close handler to use `connect_closed` signal
   - Updated toast notification handling

### Phase 5: Medium Priority (Next Sprint)

9. **Refactor `block_on()` calls in GUI**
   - Implement async credential resolution with callbacks
   - Use `glib::spawn_future_local()` pattern

10. **Split large files**
   - `dialogs/connection.rs` → multiple focused modules
   - `state.rs` → extract credential operations

### Phase 6: Low Priority (Backlog)

11. **Audit dead code annotations**
    - Review each `#[allow(dead_code)]`
    - Remove truly unused code
    - Document intentionally kept code

12. ✅ **Migrate to `adw::AboutDialog`** (COMPLETED)
    - Migrated from `gtk4::AboutDialog` to `adw::AboutDialog`
    - Added issue URL, copyright, application icon support
    - Added custom links: Releases, Details, License (GPL v3.0)
    - Added "Made with ❤️ in Ukraine 🇺🇦" to Acknowledgments

13. **Monitor blocked dependencies**
    - `ksni` for zvariant 5.x compatibility
    - `picky`/`sspi` for rand_core resolution

14. **Continue adw widget migrations** (COMPLETED)
    - ✅ `ActionRow` + `Switch` → `adw::SwitchRow` in password_generator.rs (6 switches)
    - ✅ `ActionRow` + `Switch` → `adw::SwitchRow` in cluster.rs (1 switch)
    - ✅ `ActionRow` + `Switch` → `adw::SwitchRow` in export.rs (2 switches)
    - ✅ `ActionRow` + `Entry` → `adw::EntryRow` in window_connection_dialogs.rs (New Group)
    - ✅ `ActionRow` + `Entry` → `adw::EntryRow` in window_edit_dialogs.rs (Edit Group, Rename)
    - ✅ `ActionRow` + `SpinButton` → `adw::SpinRow` in ui_tab.rs (session max age)
    - ✅ `gtk4::AlertDialog` → `adw::AlertDialog` (50+ usages migrated via `alert.rs` helper)

---

## Section 5: AlertDialog Migration Summary

### ✅ COMPLETED: Full Migration to `adw::AlertDialog`

Created `rustconn/src/alert.rs` helper module with:
- `show_alert()` - Simple info/error alert with OK button
- `show_confirm()` - Confirmation dialog with Cancel/Confirm buttons
- `show_error()` - Error alert (alias for `show_alert`)
- `show_success()` - Success alert (alias for `show_alert`)
- `show_validation_error()` - Validation error with standard heading
- `show_save_changes()` - Three-button dialog (Don't Save/Cancel/Save)

**Files migrated:**
| File | Usages | Status |
|------|--------|--------|
| `window_groups.rs` | 2 | ✅ |
| `window_snippets.rs` | 4 | ✅ |
| `window_edit_dialogs.rs` | 15 | ✅ |
| `window_operations.rs` | 8 | ✅ |
| `window_templates.rs` | 4 | ✅ |
| `window_sessions.rs` | 1 | ✅ |
| `window_clusters.rs` | 6 | ✅ |
| `window_connection_dialogs.rs` | 8 | ✅ |
| `window.rs` | 1 | ✅ |
| `app.rs` | 1 | ✅ |
| `dialogs/connection.rs` | 5 | ✅ |
| `dialogs/document.rs` | 1 | ✅ |

---

## Appendix: Verification Commands

```bash
# Verify crate boundary (should return no results)
grep -r "gtk4::\|adw::\|vte4::" rustconn-core/src/

# Check for unwrap usage
grep -rn "\.unwrap()" rustconn/src/ --include="*.rs" | grep -v test

# Verify clippy passes
cargo clippy --all-targets -- -D warnings

# Run tests
cargo test --all

# Check formatting
cargo fmt --check
```

---

*Report generated by Kiro AI Assistant*
