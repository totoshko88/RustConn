//! Balanced split planning shared by workspace restore and "Split with Other Tabs".
//!
//! Splitting only ever halves one panel, so a grid of `n` panels is built by
//! repeatedly splitting the **largest** panel along its **longest** side. The
//! plan is pure data (no GTK), so the GUI only has to execute it and the
//! algorithm stays testable.

/// One step of a balanced split plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BalancedSplitStep {
    /// Index of the panel to split. Panel `0` is the original one; the panel
    /// created by step `i` (0-based) has index `i + 1`.
    pub target: usize,
    /// `true` splits into top/bottom (`win.split-horizontal`), `false` into
    /// left/right (`win.split-vertical`).
    pub horizontal: bool,
}

/// Fallback 4:3 area used when the real size is unknown (not yet mapped).
const FALLBACK_SIZE: (f64, f64) = (1280.0, 960.0);

/// Plans `splits` successive splits of a `width` x `height` area.
///
/// A non-positive or non-finite size falls back to a 4:3 area, which still
/// yields a near-uniform grid. Ties on area prefer the longest side, then the
/// most recently created panel, so the plan is deterministic.
#[must_use]
pub fn balanced_split_plan(width: f64, height: f64, splits: usize) -> Vec<BalancedSplitStep> {
    let valid = |v: f64| v.is_finite() && v > 0.0;
    let (w, h) = if valid(width) && valid(height) {
        (width, height)
    } else {
        FALLBACK_SIZE
    };

    let mut panels: Vec<(f64, f64)> = vec![(w, h)];
    let mut plan = Vec::with_capacity(splits);
    for _ in 0..splits {
        // Same ordering the restore path always used: largest area, then
        // longest side, and the LAST of equal panels wins (like `max_by`).
        let key = |&(pw, ph): &(f64, f64)| (pw * ph, pw.max(ph));
        let mut target = 0;
        for (i, panel) in panels.iter().enumerate().skip(1) {
            let (area, side) = key(panel);
            let (best_area, best_side) = key(&panels[target]);
            if area
                .total_cmp(&best_area)
                .then(side.total_cmp(&best_side))
                .is_ge()
            {
                target = i;
            }
        }
        let (pw, ph) = panels[target];
        let horizontal = ph > pw;
        let half = if horizontal {
            (pw, ph / 2.0)
        } else {
            (pw / 2.0, ph)
        };
        panels[target] = half;
        panels.push(half);
        plan.push(BalancedSplitStep { target, horizontal });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_splits_gives_an_empty_plan() {
        assert!(balanced_split_plan(1000.0, 800.0, 0).is_empty());
    }

    #[test]
    fn wide_area_splits_left_right_first() {
        let plan = balanced_split_plan(1000.0, 500.0, 1);
        assert_eq!(
            plan,
            vec![BalancedSplitStep {
                target: 0,
                horizontal: false
            }]
        );
    }

    #[test]
    fn tall_area_splits_top_bottom_first() {
        assert!(balanced_split_plan(500.0, 1000.0, 1)[0].horizontal);
    }

    #[test]
    fn four_panels_make_a_two_by_two_grid() {
        let plan = balanced_split_plan(1000.0, 1000.0, 3);
        // Square: columns, then each half is tall and is split top/bottom.
        assert_eq!(
            plan[0],
            BalancedSplitStep {
                target: 0,
                horizontal: false
            }
        );
        assert_eq!(
            plan[1],
            BalancedSplitStep {
                target: 1,
                horizontal: true
            }
        );
        assert_eq!(
            plan[2],
            BalancedSplitStep {
                target: 0,
                horizontal: true
            }
        );
    }

    #[test]
    fn every_step_targets_an_existing_panel() {
        for n in 0..12 {
            for (i, step) in balanced_split_plan(1280.0, 720.0, n).iter().enumerate() {
                assert!(step.target <= i, "step {i} targets missing panel");
            }
        }
    }

    #[test]
    fn invalid_size_falls_back_instead_of_panicking() {
        for (w, h) in [
            (0.0, 0.0),
            (-1.0, 5.0),
            (f64::NAN, 3.0),
            (f64::INFINITY, 1.0),
        ] {
            assert_eq!(
                balanced_split_plan(w, h, 3),
                balanced_split_plan(1280.0, 960.0, 3)
            );
        }
    }
}
