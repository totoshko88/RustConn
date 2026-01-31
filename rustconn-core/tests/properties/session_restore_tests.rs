//! Property tests for session restore functionality

use proptest::prelude::*;
use rustconn_core::session::{
    PanelRestoreData, SessionRestoreData, SessionRestoreState, SessionType, SplitLayoutRestoreData,
    RESTORE_STATE_VERSION,
};
use uuid::Uuid;

/// Strategy for generating valid session types
fn session_type_strategy() -> impl Strategy<Value = SessionType> {
    prop_oneof![Just(SessionType::Embedded), Just(SessionType::External),]
}

/// Strategy for generating valid protocol names
fn protocol_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ssh".to_string()),
        Just("rdp".to_string()),
        Just("vnc".to_string()),
        Just("spice".to_string()),
    ]
}

/// Strategy for generating connection names
fn connection_name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9 _-]{0,49}".prop_map(|s| s.trim().to_string())
}

/// Strategy for generating panel IDs
fn panel_id_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,19}".prop_map(|s| s.to_string())
}

proptest! {
    /// Property: SessionRestoreData preserves all fields through builder pattern
    #[test]
    fn session_restore_data_builder_preserves_fields(
        name in connection_name_strategy(),
        protocol in protocol_strategy(),
        session_type in session_type_strategy(),
        panel_id in panel_id_strategy(),
        tab_index in 0usize..100,
    ) {
        let conn_id = Uuid::new_v4();
        let data = SessionRestoreData::new(
            conn_id,
            name.clone(),
            protocol.clone(),
            session_type.clone(),
        )
        .with_panel_id(panel_id.clone())
        .with_tab_index(tab_index);

        prop_assert_eq!(data.connection_id, conn_id);
        prop_assert_eq!(data.connection_name, name);
        prop_assert_eq!(data.protocol, protocol);
        prop_assert_eq!(data.panel_id, Some(panel_id));
        prop_assert_eq!(data.tab_index, Some(tab_index));
    }

    /// Property: SplitLayoutRestoreData clamps ratio to valid range
    #[test]
    fn split_layout_clamps_ratio(
        ratio in -1.0f64..2.0,
        horizontal in any::<bool>(),
    ) {
        let layout = SplitLayoutRestoreData::split(horizontal, ratio);

        prop_assert!(layout.split_ratio >= 0.1);
        prop_assert!(layout.split_ratio <= 0.9);
        prop_assert!(layout.is_split);
        prop_assert_eq!(layout.horizontal, horizontal);
    }

    /// Property: SessionRestoreState serialization round-trip preserves data
    #[test]
    fn session_restore_state_json_roundtrip(
        session_count in 0usize..5,
        maximized in any::<bool>(),
    ) {
        let mut state = SessionRestoreState::new();
        state.set_window_maximized(maximized);

        for i in 0..session_count {
            let session = SessionRestoreData::new(
                Uuid::new_v4(),
                format!("Connection {i}"),
                "ssh".to_string(),
                SessionType::Embedded,
            );
            state.add_session(session);
        }

        let json = state.to_json().expect("serialization should succeed");
        let restored = SessionRestoreState::from_json(&json)
            .expect("deserialization should succeed");

        prop_assert_eq!(restored.session_count(), session_count);
        prop_assert_eq!(restored.window_maximized, maximized);
        prop_assert_eq!(restored.version, RESTORE_STATE_VERSION);
    }

    /// Property: SessionRestoreState clear removes all sessions
    #[test]
    fn session_restore_state_clear_removes_all(
        session_count in 1usize..10,
    ) {
        let mut state = SessionRestoreState::new();

        for i in 0..session_count {
            let session = SessionRestoreData::new(
                Uuid::new_v4(),
                format!("Connection {i}"),
                "rdp".to_string(),
                SessionType::External,
            );
            state.add_session(session);
        }
        state.set_active_session(Uuid::new_v4());
        state.set_split_layout(SplitLayoutRestoreData::split(true, 0.5));

        prop_assert!(state.has_sessions());

        state.clear();

        prop_assert!(!state.has_sessions());
        prop_assert!(state.active_session_id.is_none());
        prop_assert!(state.split_layout.is_none());
    }

    /// Property: Window geometry is preserved correctly
    #[test]
    fn window_geometry_preserved(
        x in -10000i32..10000,
        y in -10000i32..10000,
        width in 100i32..5000,
        height in 100i32..5000,
    ) {
        let mut state = SessionRestoreState::new();
        state.set_window_geometry(x, y, width, height);

        prop_assert_eq!(state.window_geometry, Some((x, y, width, height)));

        // Verify through JSON round-trip
        let json = state.to_json().expect("serialization should succeed");
        let restored = SessionRestoreState::from_json(&json)
            .expect("deserialization should succeed");

        prop_assert_eq!(restored.window_geometry, Some((x, y, width, height)));
    }

    /// Property: Panel restore data preserves session info
    #[test]
    fn panel_restore_data_preserves_session(
        panel_id in panel_id_strategy(),
        position in 0.0f64..1.0,
        has_session in any::<bool>(),
    ) {
        let session = if has_session {
            Some(SessionRestoreData::new(
                Uuid::new_v4(),
                "Test".to_string(),
                "vnc".to_string(),
                SessionType::Embedded,
            ))
        } else {
            None
        };

        let panel = PanelRestoreData {
            panel_id: panel_id.clone(),
            session,
            position,
        };

        prop_assert_eq!(panel.panel_id, panel_id);
        prop_assert!((panel.position - position).abs() < f64::EPSILON);
        prop_assert_eq!(panel.session.is_some(), has_session);
    }

    /// Property: Split layout with panels preserves order
    #[test]
    fn split_layout_preserves_panel_order(
        panel_count in 0usize..5,
        horizontal in any::<bool>(),
    ) {
        let mut layout = SplitLayoutRestoreData::split(horizontal, 0.5);

        let panel_ids: Vec<String> = (0..panel_count)
            .map(|i| format!("panel-{i}"))
            .collect();

        for id in &panel_ids {
            layout.add_panel(PanelRestoreData {
                panel_id: id.clone(),
                session: None,
                position: 0.5,
            });
        }

        prop_assert_eq!(layout.panels.len(), panel_count);

        for (i, panel) in layout.panels.iter().enumerate() {
            prop_assert_eq!(&panel.panel_id, &panel_ids[i]);
        }
    }
}

#[test]
fn test_session_restore_state_version() {
    let state = SessionRestoreState::new();
    assert_eq!(state.version, RESTORE_STATE_VERSION);
}

#[test]
fn test_session_restore_data_touch_updates_timestamp() {
    let mut data = SessionRestoreData::new(
        Uuid::new_v4(),
        "Test".to_string(),
        "ssh".to_string(),
        SessionType::Embedded,
    );

    let original_saved_at = data.saved_at;
    std::thread::sleep(std::time::Duration::from_millis(10));
    data.touch();

    assert!(data.saved_at > original_saved_at);
}

#[test]
fn test_split_layout_default_is_not_split() {
    let layout = SplitLayoutRestoreData::default();
    assert!(!layout.is_split);
    assert!(layout.panels.is_empty());
}
