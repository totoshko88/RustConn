# RustConn Development Plan

## Current Focus: Automation & Bug Fixes

### 1. Automation "Expect" Engine
- [x] Implement `AutomationSession` logic
- [x] Integrate with `TerminalNotebook`
- [x] Update Data Models (`AutomationConfig`)
- [ ] **Verify Functionality** (In Progress)
    - Debugging why `contents-changed` signal isn't providing text.
    - Added `cursor-moved` signal listener.
    - Added verbose logging to trace VTE behavior.

### 2. Tray Icon Bug
- [ ] **Investigate Empty Menu**
    - User reported tray menu is blank/dark.
    - Suspect `ksni` menu construction or theme issue.
    - Need to verify if `StandardItem` labels are being rendered correctly.
    - Possible GTK4/DBus menu compatibility issue.

### 3. Next Steps
- Once automation is verified, clean up debug prints.
- Fix the tray icon menu rendering.
- Proceed with Week 5 tasks (Dependency Updates).
