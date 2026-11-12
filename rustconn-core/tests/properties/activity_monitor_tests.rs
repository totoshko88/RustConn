//! Property-based tests for the terminal activity monitor module.
//!
//! Tests cover `MonitorMode`, `ActivityMonitorConfig`, and `ActivityMonitorDefaults`
//! from `rustconn_core::activity_monitor`.

use proptest::prelude::*;
use rustconn_core::activity_monitor::{
    ActivityMonitorConfig, ActivityMonitorDefaults, MonitorMode,
};

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

/// Strategy that produces an arbitrary `MonitorMode` variant.
///
/// Driven by [`MonitorMode::all`] rather than a hand-written `prop_oneof!`: the list
/// used to be spelled out here, so adding a variant left every property in this file
/// silently untested against it while the suite still passed.
fn arb_monitor_mode() -> impl Strategy<Value = MonitorMode> {
    prop::sample::select(MonitorMode::all())
}

/// Strategy that produces an arbitrary `ActivityMonitorDefaults`.
fn arb_defaults() -> impl Strategy<Value = ActivityMonitorDefaults> {
    (arb_monitor_mode(), any::<u32>(), any::<u32>()).prop_map(
        |(mode, quiet_period_secs, silence_timeout_secs)| ActivityMonitorDefaults {
            mode,
            quiet_period_secs,
            silence_timeout_secs,
        },
    )
}

/// Strategy that produces an arbitrary `ActivityMonitorConfig`.
fn arb_config() -> impl Strategy<Value = ActivityMonitorConfig> {
    (
        proptest::option::of(arb_monitor_mode()),
        proptest::option::of(any::<u32>()),
        proptest::option::of(any::<u32>()),
    )
        .prop_map(
            |(mode, quiet_period_secs, silence_timeout_secs)| ActivityMonitorConfig {
                mode,
                quiet_period_secs,
                silence_timeout_secs,
            },
        )
}

// ---------------------------------------------------------------------------
// Property 5: Mode cycling visits every mode exactly once
// Feature: terminal-activity-monitor, Property 5
//
// Was "cycling is a 3-cycle", asserting `next()` three times is the identity. That
// hard-coded the number of variants into the property, so adding `Command` failed
// here rather than in the one place that decides the order. The property that
// actually matters is that the cycle closes over *all* modes — that is what makes
// every mode reachable from the per-tab Monitor menu.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn cycling_returns_to_the_start_after_one_pass(mode in arb_monitor_mode()) {
        let mut cycled = mode;
        for _ in 0..MonitorMode::all().len() {
            cycled = cycled.next();
        }
        prop_assert_eq!(cycled, mode);
    }

    #[test]
    fn cycling_never_stalls_on_the_same_mode(mode in arb_monitor_mode()) {
        // A `next()` arm that returns `self` would make the menu look broken while
        // every "cycle closes" assertion still held.
        prop_assert_ne!(mode.next(), mode);
    }
}

#[test]
fn mode_cycle_order() {
    // The specific order is Off → Activity → Silence → Command → Off: the two timing
    // modes first, then the event-driven one, then back to off.
    assert_eq!(MonitorMode::Off.next(), MonitorMode::Activity);
    assert_eq!(MonitorMode::Activity.next(), MonitorMode::Silence);
    assert_eq!(MonitorMode::Silence.next(), MonitorMode::Command);
    assert_eq!(MonitorMode::Command.next(), MonitorMode::Off);
}

#[test]
fn cycling_reaches_every_mode() {
    let mut seen = Vec::new();
    let mut mode = MonitorMode::Off;
    for _ in 0..MonitorMode::all().len() {
        seen.push(mode);
        mode = mode.next();
    }
    for candidate in MonitorMode::all() {
        assert!(
            seen.contains(candidate),
            "{candidate:?} cannot be reached by cycling, so the menu cannot select it"
        );
    }
}

// ---------------------------------------------------------------------------
// Property 6: Serialization round-trip for ActivityMonitorConfig
// Feature: terminal-activity-monitor, Property 6: Serialization round-trip
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn serde_roundtrip_config(config in arb_config()) {
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ActivityMonitorConfig =
            serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(config, deserialized);
    }
}

// ---------------------------------------------------------------------------
// Property 7: Config resolution prefers per-connection overrides
// Feature: terminal-activity-monitor, Property 7: Config resolution prefers per-connection overrides
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn config_resolution_prefers_override(
        global in arb_defaults(),
        config in arb_config(),
    ) {
        // effective_mode: per-connection when Some, global when None
        let effective_mode = config.effective_mode(&global);
        if let Some(m) = config.mode {
            prop_assert_eq!(effective_mode, m);
        } else {
            prop_assert_eq!(effective_mode, global.mode);
        }

        // effective_quiet_period: per-connection (clamped) when Some, global (clamped) when None
        let effective_quiet = config.effective_quiet_period(&global);
        if let Some(v) = config.quiet_period_secs {
            prop_assert_eq!(effective_quiet, v.clamp(1, 300));
        } else {
            prop_assert_eq!(effective_quiet, global.quiet_period_secs.clamp(1, 300));
        }

        // effective_silence_timeout: per-connection (clamped) when Some, global (clamped) when None
        let effective_silence = config.effective_silence_timeout(&global);
        if let Some(v) = config.silence_timeout_secs {
            prop_assert_eq!(effective_silence, v.clamp(1, 600));
        } else {
            prop_assert_eq!(effective_silence, global.silence_timeout_secs.clamp(1, 600));
        }
    }
}

// ---------------------------------------------------------------------------
// Property 8: Timeout clamping for quiet_period and silence_timeout
// Feature: terminal-activity-monitor, Property 8: Timeout clamping
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn quiet_period_clamped(v in any::<u32>()) {
        let defaults = ActivityMonitorDefaults {
            quiet_period_secs: v,
            ..ActivityMonitorDefaults::default()
        };
        let effective = defaults.effective_quiet_period();
        prop_assert!(effective >= 1);
        prop_assert!(effective <= 300);
        prop_assert_eq!(effective, v.clamp(1, 300));
    }

    #[test]
    fn silence_timeout_clamped(v in any::<u32>()) {
        let defaults = ActivityMonitorDefaults {
            silence_timeout_secs: v,
            ..ActivityMonitorDefaults::default()
        };
        let effective = defaults.effective_silence_timeout();
        prop_assert!(effective >= 1);
        prop_assert!(effective <= 600);
        prop_assert_eq!(effective, v.clamp(1, 600));
    }

    #[test]
    fn config_quiet_period_clamped(v in any::<u32>()) {
        let global = ActivityMonitorDefaults::default();
        let config = ActivityMonitorConfig {
            mode: None,
            quiet_period_secs: Some(v),
            silence_timeout_secs: None,
        };
        let effective = config.effective_quiet_period(&global);
        prop_assert!(effective >= 1);
        prop_assert!(effective <= 300);
        prop_assert_eq!(effective, v.clamp(1, 300));
    }

    #[test]
    fn config_silence_timeout_clamped(v in any::<u32>()) {
        let global = ActivityMonitorDefaults::default();
        let config = ActivityMonitorConfig {
            mode: None,
            quiet_period_secs: None,
            silence_timeout_secs: Some(v),
        };
        let effective = config.effective_silence_timeout(&global);
        prop_assert!(effective >= 1);
        prop_assert!(effective <= 600);
        prop_assert_eq!(effective, v.clamp(1, 600));
    }
}

// ---------------------------------------------------------------------------
// Property 3: Off mode suppresses all notifications
// Feature: terminal-activity-monitor, Property 3: Off mode suppresses all notifications
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn off_mode_effective_mode_returns_off(
        global in arb_defaults(),
        quiet in proptest::option::of(any::<u32>()),
        silence in proptest::option::of(any::<u32>()),
    ) {
        // When mode is explicitly Off, effective_mode must return Off
        // regardless of global defaults or other config values.
        let config = ActivityMonitorConfig {
            mode: Some(MonitorMode::Off),
            quiet_period_secs: quiet,
            silence_timeout_secs: silence,
        };
        prop_assert_eq!(config.effective_mode(&global), MonitorMode::Off);
    }

    #[test]
    fn off_mode_via_global_default(
        quiet_period_secs in any::<u32>(),
        silence_timeout_secs in any::<u32>(),
        config_quiet in proptest::option::of(any::<u32>()),
        config_silence in proptest::option::of(any::<u32>()),
    ) {
        // When per-connection mode is None and global default is Off,
        // effective_mode must return Off.
        let global = ActivityMonitorDefaults {
            mode: MonitorMode::Off,
            quiet_period_secs,
            silence_timeout_secs,
        };
        let config = ActivityMonitorConfig {
            mode: None,
            quiet_period_secs: config_quiet,
            silence_timeout_secs: config_silence,
        };
        prop_assert_eq!(config.effective_mode(&global), MonitorMode::Off);
    }
}
