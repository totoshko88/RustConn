//! Property-based and unit tests for the session-placement predicate.
//!
//! **Feature: detachable-session-windows**
//!
//! Covers [`detach_verdict`]: determinism over repeated calls, the full 2^4
//! `DetachContext` flag matrix, verdict precedence when several blocking
//! conditions hold at once, and [`DetachVerdict::reason_key`] totality.
//!
//! GUI-free by design — the predicate lives in `rustconn-core`, so these tests
//! need no display and no GTK.

use proptest::prelude::*;
use rustconn_core::{DetachContext, DetachVerdict, detach_verdict, may_place_in_split};

/// Every verdict variant, for totality checks.
const ALL_VERDICTS: [DetachVerdict; 5] = [
    DetachVerdict::Allowed,
    DetachVerdict::AlreadyDetached,
    DetachVerdict::ExternalViewer,
    DetachVerdict::SplitOwner,
    DetachVerdict::SplitGuest,
];

/// Collects every blocking condition a context satisfies.
///
/// Stated from the requirements rather than from the predicate's if/else chain:
/// each of the four blocking verdicts has exactly one flag that justifies it, so
/// a verdict is only defensible when its own flag is set. Deliberately says
/// nothing about which blocker wins — precedence is pinned separately, by name,
/// in the unit tests below.
fn blockers_of(context: DetachContext) -> Vec<DetachVerdict> {
    let mut blockers = Vec::new();
    if context.is_detached {
        blockers.push(DetachVerdict::AlreadyDetached);
    }
    if !context.renders_in_process {
        blockers.push(DetachVerdict::ExternalViewer);
    }
    if context.is_split_owner {
        blockers.push(DetachVerdict::SplitOwner);
    }
    if context.is_split_guest {
        blockers.push(DetachVerdict::SplitGuest);
    }
    blockers
}

/// Builds a context from the four flags in declaration order.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "mirrors the four independent DetachContext flags one-to-one so the matrix stays readable"
)]
const fn ctx(
    renders_in_process: bool,
    is_split_owner: bool,
    is_split_guest: bool,
    is_detached: bool,
) -> DetachContext {
    DetachContext {
        renders_in_process,
        is_split_owner,
        is_split_guest,
        is_detached,
    }
}

/// Enumerates the full 2^4 flag matrix.
fn all_contexts() -> Vec<DetachContext> {
    let mut out = Vec::with_capacity(16);
    for renders in [false, true] {
        for owner in [false, true] {
            for guest in [false, true] {
                for detached in [false, true] {
                    out.push(ctx(renders, owner, guest, detached));
                }
            }
        }
    }
    out
}

/// Strategy over the whole (finite) context space.
fn detach_context_strategy() -> impl Strategy<Value = DetachContext> {
    (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>())
        .prop_map(|(renders, owner, guest, detached)| ctx(renders, owner, guest, detached))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Feature: detachable-session-windows — detachability predicate
    //
    // *For any* `DetachContext`, `detach_verdict` returns the same verdict on
    // every call, so context menu population, the keyboard action, and sidebar
    // routing always agree.
    #[test]
    fn prop_detach_verdict_is_deterministic(context in detach_context_strategy()) {
        let first = detach_verdict(&context);
        for _ in 0..8 {
            prop_assert_eq!(detach_verdict(&context), first);
        }

        // A structurally equal context (rebuilt from the same flags) also agrees.
        let rebuilt = ctx(
            context.renders_in_process,
            context.is_split_owner,
            context.is_split_guest,
            context.is_detached,
        );
        prop_assert_eq!(detach_verdict(&rebuilt), first);
    }

    // *For any* `DetachContext`, the verdict names exactly one blocking
    // condition, and that condition holds for the context — so a rejection can
    // never be explained by a reason the session does not actually have.
    #[test]
    fn prop_verdict_names_a_blocker_that_holds(context in detach_context_strategy()) {
        let verdict = detach_verdict(&context);
        let blockers = blockers_of(context);

        if blockers.is_empty() {
            prop_assert_eq!(verdict, DetachVerdict::Allowed);
        } else {
            prop_assert_ne!(verdict, DetachVerdict::Allowed);
            prop_assert!(
                blockers.contains(&verdict),
                "{:?} is not a blocker of {:?}",
                verdict,
                context
            );
            prop_assert_eq!(
                blockers.iter().filter(|b| **b == verdict).count(),
                1,
                "a verdict must name exactly one blocking condition"
            );
        }
    }

    // *For any* `DetachContext`, a detach is permitted exactly when no blocking
    // flag is set, and `is_allowed` agrees with the verdict variant.
    #[test]
    fn prop_is_allowed_matches_flags(context in detach_context_strategy()) {
        let verdict = detach_verdict(&context);
        let no_blocker = context.renders_in_process
            && !context.is_split_owner
            && !context.is_split_guest
            && !context.is_detached;

        prop_assert_eq!(verdict.is_allowed(), no_blocker);
        prop_assert_eq!(verdict == DetachVerdict::Allowed, no_blocker);
    }

    // *For any* `DetachContext`, a split layout only accepts a session whose
    // in-process widget is free to move: never one that lives in a detached
    // window (the picker must not rip a widget out of its own window), never one
    // that already sits in someone else's split, and never one drawn by an
    // external viewer. Anything the detach path accepts satisfies all three, so
    // it is placeable too — stated as implications rather than as an equality
    // with `detach_verdict`, which also ranks reasons this question does not
    // care about.
    #[test]
    fn prop_detached_session_is_never_placed_in_a_split(context in detach_context_strategy()) {
        let placeable = may_place_in_split(&context);
        if context.is_detached {
            prop_assert!(!placeable, "a detached session must be refused: {:?}", context);
            prop_assert_eq!(detach_verdict(&context), DetachVerdict::AlreadyDetached);
        }
        if context.is_split_guest || !context.renders_in_process {
            prop_assert!(
                !placeable,
                "a session with no free in-process widget must be refused: {:?}",
                context
            );
        }
        if detach_verdict(&context) == DetachVerdict::Allowed {
            prop_assert!(
                placeable,
                "a detachable session must also be placeable: {:?}",
                context
            );
        }
    }

    // *For any* `DetachContext`, the verdict's reason key is non-empty and
    // identifies the verdict uniquely, so the GUI always has exactly one
    // translatable explanation to show.
    #[test]
    fn prop_reason_key_is_available_for_every_context(context in detach_context_strategy()) {
        let verdict = detach_verdict(&context);
        let key = verdict.reason_key();
        prop_assert!(!key.is_empty());

        let owners: Vec<DetachVerdict> = ALL_VERDICTS
            .into_iter()
            .filter(|candidate| candidate.reason_key() == key)
            .collect();
        prop_assert_eq!(owners, vec![verdict]);
    }
}

#[test]
fn full_flag_matrix_is_covered_and_exhaustive() {
    let contexts = all_contexts();
    assert_eq!(contexts.len(), 16, "the flag matrix has 2^4 combinations");

    let unique: std::collections::HashSet<_> = contexts.iter().copied().collect();
    assert_eq!(unique.len(), 16, "every combination appears exactly once");

    for context in contexts {
        let verdict = detach_verdict(&context);
        let blockers = blockers_of(context);
        assert_eq!(
            verdict.is_allowed(),
            blockers.is_empty(),
            "allowed must mean no blocker for {context:?}"
        );
        if !blockers.is_empty() {
            assert!(
                blockers.contains(&verdict),
                "{verdict:?} is not a blocker of {context:?}"
            );
        }
    }
}

#[test]
fn every_verdict_variant_is_reachable() {
    let reached: std::collections::HashSet<DetachVerdict> =
        all_contexts().iter().map(detach_verdict).collect();

    for verdict in ALL_VERDICTS {
        assert!(
            reached.contains(&verdict),
            "verdict {verdict:?} is unreachable from the flag matrix"
        );
    }
    assert_eq!(reached.len(), ALL_VERDICTS.len());
}

#[test]
fn reason_keys_are_distinct_and_non_empty() {
    let mut keys = std::collections::HashSet::new();
    for verdict in ALL_VERDICTS {
        let key = verdict.reason_key();
        assert!(!key.is_empty(), "{verdict:?} has an empty reason key");
        assert!(
            keys.insert(key),
            "reason key {key} is used by more than one verdict"
        );
    }
    assert_eq!(keys.len(), ALL_VERDICTS.len());
}

#[test]
fn already_detached_wins_over_every_other_blocker() {
    // All blocking flags at once: the session already has a window, which is
    // the state the user needs to hear about.
    assert_eq!(
        detach_verdict(&ctx(false, true, true, true)),
        DetachVerdict::AlreadyDetached
    );
    assert_eq!(
        detach_verdict(&ctx(true, true, true, true)),
        DetachVerdict::AlreadyDetached
    );
}

#[test]
fn external_viewer_wins_over_split_membership() {
    assert_eq!(
        detach_verdict(&ctx(false, true, true, false)),
        DetachVerdict::ExternalViewer
    );
    assert_eq!(
        detach_verdict(&ctx(false, false, true, false)),
        DetachVerdict::ExternalViewer
    );
}

#[test]
fn split_owner_wins_over_split_guest() {
    assert_eq!(
        detach_verdict(&ctx(true, true, true, false)),
        DetachVerdict::SplitOwner
    );
}

#[test]
fn plain_in_process_session_is_allowed() {
    let verdict = detach_verdict(&ctx(true, false, false, false));
    assert_eq!(verdict, DetachVerdict::Allowed);
    assert!(verdict.is_allowed());
    assert_eq!(verdict.reason_key(), "detach-allowed");
}

#[test]
fn split_placement_refuses_a_detached_session_and_accepts_a_tabbed_one() {
    // The case the split "Select Tab" picker had to be taught (issue #236).
    assert!(!may_place_in_split(&ctx(true, false, false, true)));
    // A plain tabbed session and a split owner are the two placeable states.
    assert!(may_place_in_split(&ctx(true, false, false, false)));
    assert!(may_place_in_split(&ctx(true, true, false, false)));
    // Nothing to place: an external viewer has no in-process widget, and a
    // guest's widget already sits in someone else's split.
    assert!(!may_place_in_split(&ctx(false, false, false, false)));
    assert!(!may_place_in_split(&ctx(true, false, true, false)));
}

#[test]
fn single_blocking_flags_map_to_their_own_verdict() {
    assert_eq!(
        detach_verdict(&ctx(false, false, false, false)),
        DetachVerdict::ExternalViewer
    );
    assert_eq!(
        detach_verdict(&ctx(true, true, false, false)),
        DetachVerdict::SplitOwner
    );
    assert_eq!(
        detach_verdict(&ctx(true, false, true, false)),
        DetachVerdict::SplitGuest
    );
    assert_eq!(
        detach_verdict(&ctx(true, false, false, true)),
        DetachVerdict::AlreadyDetached
    );
}
