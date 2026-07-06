# Requirements Document

## Introduction

Today RustConn's split view works only for VTE terminal-based sessions (ssh, local, sftp, telnet,
serial, kubernetes, zerotrust*). The split actions in `rustconn/src/window/split_view_actions.rs`
explicitly reject RDP, VNC, and SPICE sessions with the toast "Split view is available for
terminal-based sessions only", because those protocols render into embedded viewer widgets
(IronRDP / VNC / SPICE) instead of a VTE terminal.

This feature delivers full, production-grade split-view support for all **in-process embedded**
protocol windows (RDP, VNC, SPICE) so that they behave the same as terminal sessions inside a split
container. This includes splitting, reparenting the live embedded viewer between panels,
drag-and-drop, Select-Tab placement, focus styling, close-pane, eviction to a new root tab,
per-container color coding, per-panel dynamic resolution/scaling, focus-scoped special keys and
clipboard, and toolbar overflow in narrow panels. Terminal-only features (specifically keystroke
broadcast) must degrade correctly when a split container contains one or more embedded panels.

**Scope — in-process embedded only.** This feature applies exclusively to sessions rendered by an
in-process embedded viewer widget: IronRDP embedded RDP, embedded VNC, and embedded SPICE. Sessions
displayed through an external process/viewer (xfreerdp, vncviewer, an external SPICE viewer) are out
of scope: they have no in-process GTK widget to reparent and therefore cannot be placed in a split.
Attempting to split such a session is an explicit, declined operation with a clear message.

The split system spans two crates and this boundary must be preserved:

- `rustconn-core::split` — pure data models (`SplitLayoutModel`, `PanelNode`, `SplitNode`,
  `PanelId` / `TabId` / `SessionId`, `DropResult`, `ColorPool`) with no GTK dependencies.
- `rustconn::split_view` — GTK4/libadwaita adapter (`SplitViewAdapter`, `SplitViewBridge`,
  `DropSource` / `DropOutcome`) that wires the core model to widgets.

This feature targets the 0.18.1 development line. 0.18.1 is an in-progress development branch and is
not being released as part of this work; no release execution is in scope here.

## Glossary

- **Embedded_Session**: An active session whose content is rendered by an embedded viewer widget
  rather than a VTE terminal. Applies to the RDP, VNC, and SPICE protocols.
- **Terminal_Session**: An active session whose content is a VTE terminal. Applies to the ssh,
  local, sftp, telnet, serial, kubernetes, and zerotrust* protocols.
- **Embedded_Viewer_Widget**: The single GTK widget instance that renders an Embedded_Session
  (VNC `VncSessionWidget`, `EmbeddedRdpWidget`, or `EmbeddedSpiceWidget`). It holds the live
  protocol connection and can have only one GTK parent at a time.
- **External_Process_Session**: A session displayed via an external process fallback (for example
  xfreerdp or vncviewer) that has no in-process GTK viewer widget and therefore cannot be embedded
  into a Panel.
- **Split_View_System**: The overall feature that arranges sessions into a tree of resizable Panels
  within a single Container_Tab.
- **Split_Layout_Model**: The GTK-free core model in `rustconn-core::split` that tracks the panel
  tree, panel-to-session assignments, focus, and split structure.
- **Split_View_Adapter**: The GTK layer (`rustconn::split_view::adapter::SplitViewAdapter`) that
  builds and rebuilds widgets from the Split_Layout_Model.
- **Split_View_Bridge**: The GTK layer (`rustconn::split_view::bridge::SplitViewBridge`) that
  provides the session-oriented API used by window actions and manages placement of content
  widgets into Panels.
- **Panel**: A single leaf region of a split container that displays at most one session's content.
- **Empty_Panel**: A Panel that currently displays no session.
- **Container_Tab**: A single tab whose content is a split container holding two or more Panels.
- **Root_Tab**: A top-level tab in the tab bar that is not currently placed inside a Panel.
- **Session_Provider**: The callback that supplies the list of sessions eligible to be placed into
  an Empty_Panel via Select-Tab.
- **Broadcast_Controller**: The Split_View_Bridge subsystem that mirrors keystrokes across
  Terminal_Sessions in a split container.
- **Container_Color**: The single color index allocated to a split container and applied to all its
  Panels and their tab indicators.
- **Content_Widget**: The GTK widget placed into a Panel to display a session, either a VTE terminal
  (Terminal_Session) or an Embedded_Viewer_Widget (Embedded_Session).
- **Embedded_Toolbar**: The action toolbar that is part of each Embedded_Viewer_Widget's container
  (CSS class `embedded-toolbar`). It holds protocol actions such as Fit resolution, Copy, Paste,
  Autotype, Scripts, Ctrl+Alt+Del, Quick actions, and Save Files (the exact set varies per protocol).
- **Overflow_Menu**: A single menu control ("⋯") that collects secondary Embedded_Toolbar actions
  when the Panel is too narrow to display the full toolbar.
- **Primary_Toolbar_Actions**: The Embedded_Toolbar actions that remain directly visible even in a
  narrow Panel: Fit resolution and Ctrl+Alt+Del.
- **Secondary_Toolbar_Actions**: The Embedded_Toolbar actions that collapse into the Overflow_Menu in
  a narrow Panel: Copy, Paste, Autotype, Scripts, Quick actions, and Save Files.
- **Minimum_Desktop_Resolution**: The smallest remote desktop resolution the embedded client will
  request (640x480 device pixels for RDP).
- **Reconnect_Banner**: The in-widget banner (already part of each Embedded_Viewer_Widget's
  container) that displays a disconnected state and a Reconnect control.

## Requirements

### Requirement 1: Allow splitting embedded protocol sessions

**User Story:** As a user viewing an RDP, VNC, or SPICE session, I want to split its tab, so that I
can view multiple remote desktops or a remote desktop alongside a terminal in one tab.

#### Acceptance Criteria

1. WHEN the split-horizontal action is activated and the active session is an Embedded_Session, THE Split_View_System SHALL create a horizontal split and place the active session in the original Panel.
2. WHEN the split-vertical action is activated and the active session is an Embedded_Session, THE Split_View_System SHALL create a vertical split and place the active session in the original Panel.
3. WHEN a split action is activated for an Embedded_Session, THE Split_View_System SHALL display the split container and SHALL suppress the message "Split view is available for terminal-based sessions only".
4. WHERE the active session is an External_Process_Session, WHEN a split action is activated, THE Split_View_System SHALL decline the split and display the message "Split view is not available for external-viewer sessions. Switch this connection to embedded mode to use split." and SHALL leave the layout unchanged.
5. WHEN a split action is activated and no session is active, THE Split_View_System SHALL leave the layout unchanged.

### Requirement 2: Reparent embedded viewer widgets between panels

**User Story:** As a user, I want an embedded session to keep its live connection when it moves
between panels, so that splitting or rearranging does not disconnect or reset the remote session.

#### Acceptance Criteria

1. WHEN an Embedded_Session is placed into a Panel, THE Split_View_Bridge SHALL display that session's existing Embedded_Viewer_Widget in the Panel.
2. WHEN an Embedded_Viewer_Widget is moved from one Panel to another Panel, THE Split_View_Bridge SHALL detach the widget from its previous parent before attaching it to the target Panel.
3. WHILE an Embedded_Viewer_Widget is displayed in a Panel, THE Split_View_Bridge SHALL preserve the widget's live protocol connection without disconnecting or reconnecting it.
4. WHILE an Embedded_Viewer_Widget exists for a session that is not displayed in any Panel, THE Split_View_Bridge SHALL preserve the widget's live protocol connection.
5. IF an Embedded_Viewer_Widget cannot be located for a session that is requested to be shown in a Panel, THEN THE Split_View_Bridge SHALL leave the Panel empty and record a diagnostic log entry.
6. THE Split_View_Bridge SHALL retain a single instance of each Embedded_Viewer_Widget across all placement and reparenting operations.

### Requirement 3: Drag-and-drop of embedded sessions between panels

**User Story:** As a user, I want to drag an embedded session between panels, so that I can rearrange
my split layout with the pointer.

#### Acceptance Criteria

1. WHEN an Embedded_Session is dragged from a Root_Tab onto an Empty_Panel, THE Split_View_System SHALL place the session in the target Panel and remove the source Root_Tab.
2. WHEN an Embedded_Session is dragged onto a Panel that already displays a session, THE Split_View_System SHALL place the dragged session in the target Panel and move the previously displayed session to a new Root_Tab.
3. WHEN an Embedded_Session is dragged from one Panel to another Panel, THE Split_View_System SHALL place the session in the target Panel and clear the source Panel.
4. WHILE a draggable item is over an Empty_Panel, THE Split_View_System SHALL apply the empty-target drop highlight.
5. WHILE a draggable item is over a Panel that already displays a session, THE Split_View_System SHALL apply the occupied-target drop highlight.
6. WHEN a drag operation over a Panel terminates by drop, by cancellation, or by leaving the Panel, THE Split_View_System SHALL remove all drop highlight styling from that Panel.

### Requirement 4: Select-Tab placement for embedded sessions

**User Story:** As a user, I want to choose an embedded session from a list to fill an empty panel,
so that I can populate a split without dragging.

#### Acceptance Criteria

1. WHEN the Session_Provider is queried for an Empty_Panel, THE Split_View_System SHALL include eligible Embedded_Sessions in the returned list.
2. THE Session_Provider SHALL exclude sessions that are already displayed in a Panel of the same split container from the returned list.
3. THE Session_Provider SHALL exclude External_Process_Sessions from the returned list.
4. WHEN a user selects an Embedded_Session from the Select-Tab list for an Empty_Panel, THE Split_View_System SHALL place that session's Embedded_Viewer_Widget in the Panel.
5. THE Split_View_System SHALL place into an Empty_Panel via Select-Tab only sessions that were included in the Session_Provider list for that Panel.
6. WHEN an Embedded_Session is placed via Select-Tab and that session was displayed in a different split container, THE Split_View_System SHALL clear the session from the previous container before placing it.
7. WHEN an Embedded_Session is placed via Select-Tab, THE Split_View_System SHALL display a placeholder in the session's original Root_Tab indicating where the session is now displayed.

### Requirement 5: Focus styling and input routing for embedded panels

**User Story:** As a user, I want the focused embedded panel to be visually indicated and to receive
keyboard and pointer input, so that I know which remote desktop I am controlling.

#### Acceptance Criteria

1. WHEN a user clicks a Panel that displays an Embedded_Session, THE Split_View_System SHALL mark that Panel as the focused Panel.
2. WHEN a Panel that displays an Embedded_Session becomes the focused Panel, THE Split_View_System SHALL apply the focus styling to that Panel and remove focus styling from all other Panels in the container.
3. WHEN a Panel that displays an Embedded_Session becomes the focused Panel, THE Split_View_System SHALL grant keyboard focus to that Panel's Embedded_Viewer_Widget.
4. WHEN a user clicks a Panel that displays an Embedded_Session, THE Split_View_System SHALL keep the split container's Content_Widget displayed instead of switching to another tab.

### Requirement 6: Close pane and eviction for embedded sessions

**User Story:** As a user, I want to close a panel that holds an embedded session, so that I can
collapse a split without losing the session.

#### Acceptance Criteria

1. WHEN the close action is invoked on a Panel that displays an Embedded_Session, THE Split_View_System SHALL remove that Panel from the split container.
2. WHEN the last remaining split in a Container_Tab is collapsed, THE Split_View_System SHALL restore the Container_Tab to a single-session tab.
3. WHEN a session is evicted from a Panel during a drop, THE Split_View_System SHALL move the evicted session to a new Root_Tab.
4. WHEN an Embedded_Session is evicted to a new Root_Tab, THE Split_View_System SHALL reparent the session's Embedded_Viewer_Widget into the new Root_Tab and preserve the live protocol connection.
5. WHEN the close button is clicked on an Empty_Panel, THE Split_View_System SHALL set focus to that Panel and remove it from the split container.

### Requirement 7: Per-container color coding for embedded sessions

**User Story:** As a user, I want embedded sessions in a split to share the container's color, so
that I can visually associate panels and their tab indicators.

#### Acceptance Criteria

1. WHEN a split container is first created, THE Split_View_System SHALL allocate one Container_Color for the container.
2. WHEN an Embedded_Session is placed in a Panel of a split container, THE Split_View_System SHALL apply the Container_Color to that session's tab indicator.
3. THE Split_View_System SHALL apply the same Container_Color to every Panel within a single split container.
4. WHEN removal of an Embedded_Session from a split container begins, THE Split_View_System SHALL clear the Container_Color from that session's tab indicator immediately.

### Requirement 8: Broadcast behavior with embedded panels

**User Story:** As a user, I want keystroke broadcast to behave predictably when embedded sessions
are present, so that broadcast never targets a remote desktop that cannot receive mirrored VTE
keystrokes.

#### Acceptance Criteria

1. WHILE a split container displays one or more Embedded_Sessions, THE Broadcast_Controller SHALL exclude every Embedded_Session from keystroke mirroring.
2. WHILE the focused Panel displays an Embedded_Session, THE Split_View_System SHALL hide the broadcast toggle control.
3. WHILE a split container displays fewer than two Terminal_Sessions, THE Split_View_System SHALL hide the broadcast toggle control.
4. WHILE broadcast is active and a split container contains both Terminal_Sessions and Embedded_Sessions, THE Broadcast_Controller SHALL mirror keystrokes only among the Terminal_Sessions.
5. WHEN an Embedded_Session is placed into a split container while broadcast is active, THE Broadcast_Controller SHALL keep keystroke mirroring among existing Terminal_Sessions unchanged.

### Requirement 9: Session lifecycle within a split

**User Story:** As a user, I want disconnect and reconnect of an embedded session to be handled
while it is inside a split, so that a dropped connection does not corrupt the split layout.

#### Acceptance Criteria

1. WHEN an Embedded_Session displayed in a Panel loses its protocol connection, THE Split_View_System SHALL keep the Panel open, keep that session's Embedded_Viewer_Widget displayed in the Panel in a disconnected state, and show the standard reconnect banner with a Reconnect control inside that Panel.
2. WHEN an Embedded_Session displayed in a Panel loses its protocol connection, THE Split_View_System SHALL NOT close the Panel or collapse the split.
3. WHEN the Reconnect control inside a Panel is activated, THE Split_View_System SHALL reconnect the session and display the reconnected Embedded_Viewer_Widget in the same Panel.
4. WHEN a Panel displaying an Embedded_Session is closed, THE Split_View_System SHALL stop any recording associated with that session.
5. IF an Embedded_Session is closed while displayed in a Panel, THEN THE Split_View_System SHALL release that session's Embedded_Viewer_Widget so that its resources are freed.

### Requirement 10: Preserve the core crate boundary

**User Story:** As a maintainer, I want the split feature to keep business logic GUI-free, so that
the core model stays testable and the crate boundary holds.

#### Acceptance Criteria

1. THE Split_Layout_Model SHALL represent panel-to-session assignments using the protocol-agnostic SessionId type.
2. THE Split_Layout_Model SHALL exclude GTK, libadwaita, and VTE types from its public interface and internal state.
3. THE Split_Layout_Model SHALL produce the same placement, eviction, and split results for an Embedded_Session as for a Terminal_Session.
4. THE Split_View_Bridge SHALL place Terminal_Sessions and Embedded_Sessions into Panels through a single Content_Widget placement path so that both session types are handled uniformly.
5. THE rustconn-core crate SHALL exclude imports of gtk4, libadwaita, and vte4.

### Requirement 11: Mixed terminal and embedded splits

**User Story:** As a user, I want to combine terminal and embedded sessions in one split, so that I
can monitor a server terminal next to its remote desktop.

#### Acceptance Criteria

1. WHEN a Terminal_Session is placed in a Panel of a split container that already contains an Embedded_Session, THE Split_View_System SHALL display both sessions in their respective Panels.
2. WHEN an Embedded_Session is placed in a Panel of a split container that already contains a Terminal_Session, THE Split_View_System SHALL display both sessions in their respective Panels.
3. WHILE a split container contains both Terminal_Sessions and Embedded_Sessions, THE Split_View_System SHALL apply the Container_Color to every Panel in the container.
4. WHEN focus moves between a Terminal_Session Panel and an Embedded_Session Panel, THE Split_View_System SHALL grant input focus to the Content_Widget of the newly focused Panel.
### Requirement 12: Adaptive toolbar overflow for embedded viewers

**User Story:** As a user, I want an embedded session's toolbar to stay usable when its area is
narrow — whether inside a split Panel or a single-session tab in a small application window — so that
no controls are clipped and the remote view is not crowded.

This behavior is a property of the Embedded_Viewer_Widget itself and SHALL apply regardless of the
container: a split Panel, a single-session tab, or a narrow/small application window.

#### Acceptance Criteria

1. WHILE the width available to an Embedded_Viewer_Widget is below the toolbar overflow threshold, THE Embedded_Viewer_Widget SHALL collapse the Secondary_Toolbar_Actions into a single Overflow_Menu control.
2. WHILE the width available to an Embedded_Viewer_Widget is below the toolbar overflow threshold, THE Embedded_Viewer_Widget SHALL keep the Primary_Toolbar_Actions directly visible.
3. WHILE the width available to an Embedded_Viewer_Widget is at or above the toolbar overflow threshold, THE Embedded_Viewer_Widget SHALL display the full Embedded_Toolbar without an Overflow_Menu.
4. THE Embedded_Viewer_Widget SHALL keep every toolbar action reachable at any available width, either directly or through the Overflow_Menu.
5. WHEN the width available to an Embedded_Viewer_Widget crosses the toolbar overflow threshold, THE Embedded_Viewer_Widget SHALL move the Secondary_Toolbar_Actions between the toolbar and the Overflow_Menu accordingly.
6. THE Embedded_Toolbar SHALL NOT overflow or clip its controls at any application window size that the application otherwise supports.

### Requirement 13: Adaptive resolution and scaling for embedded viewers

**User Story:** As a user, I want an embedded remote desktop to fill its area and stay legible even
when that area is very small or an unusual shape — in a split Panel or in a small/narrow application
window — so that I can shrink the window without losing or clipping the remote view.

This behavior is a property of the Embedded_Viewer_Widget itself and SHALL apply regardless of the
container: a split Panel, a single-session tab, or a narrow/small application window.

#### Acceptance Criteria

1. WHEN the area available to an Embedded_Viewer_Widget changes AND its *logical* (CSS) size is at least the Minimum_Desktop_Resolution, THE Embedded_Viewer_Widget SHALL request a matching remote desktop resolution for that area at the display's DPI (full HiDPI/retina where the display is scaled), debounced, without reconnecting where the protocol supports live resolution change (RDP Display Control / MS-RDPEDISP).
2. WHEN the *logical* (CSS) size available to an Embedded_Viewer_Widget is smaller than the Minimum_Desktop_Resolution, THE Embedded_Viewer_Widget SHALL request a remote resolution that matches the area's aspect ratio scaled up (2×, or 3× when smaller) to at least the Minimum_Desktop_Resolution, SHALL request a fixed 100% remote scale/DPI so the remote cursor and UI stay normal-sized (not enlarged), and SHALL locally downscale the rendered frame to fully fill the area — so the whole remote desktop stays visible and dense (small) rather than a cramped, oversized fragment, and no reconnect is triggered.
3. WHERE the protocol does not support live resolution change, THE Embedded_Viewer_Widget SHALL scale the fixed remote frame to fill the available area.
4. THE Embedded_Viewer_Widget SHALL scale the rendered frame so that a small or oddly shaped area is fully filled without leaving empty (letterboxed) regions.
5. WHILE a resize is in progress, THE Embedded_Viewer_Widget SHALL scale the current frame to fit the area for immediate visual feedback before any new resolution takes effect.
6. THE Embedded_Viewer_Widget SHALL NOT reconnect solely because its available area changed when the protocol supports live resolution change.

### Requirement 14: Focus-scoped special keys and clipboard

**User Story:** As a user, I want special-key and clipboard actions to target the embedded session I
am currently focused on, so that I never send input to the wrong remote desktop.

#### Acceptance Criteria

1. WHILE a Panel displaying an Embedded_Session is focused, THE Split_View_System SHALL route the Ctrl+Alt+Del action to that focused session.
2. WHILE a Panel displaying an Embedded_Session is focused, THE Split_View_System SHALL route clipboard copy and paste actions to that focused session.
3. WHEN focus leaves a Panel displaying an Embedded_Session, THE Embedded_Viewer_Widget SHALL release any keys it is tracking as held so that no modifier remains stuck in the remote session.
4. THE Split_View_System SHALL NOT capture the compositor shortcuts Super and Alt+Tab into a focused Embedded_Session.
