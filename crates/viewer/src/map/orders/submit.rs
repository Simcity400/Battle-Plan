//! Stored-control builder and forward loop removal. Offset fronts resubmit
//! their smoothed points through this builder; paradrops disable intersection checks.
//!
//! These are graphics objects built by this viewer. No native control array,
//! selection, clock or other client history is recorded.

use super::fixed::{Point, SCALE};

/// Stop accepting another point at 8192 stored controls. An insertion reaches
/// 8191 and still appends its target, so the cap is 8192.
const POINT_CAP: usize = 0x2000;

fn squared_delta(a: Point, b: Point) -> i64 {
    let [x, z] = [a[0].wrapping_sub(b[0]), a[1].wrapping_sub(b[1])];
    // Divide the two products separately.
    (x.wrapping_mul(x) / SCALE).wrapping_add(z.wrapping_mul(z) / SCALE)
}

fn divided_step(value: i64, steps: i32) -> i64 {
    let denominator = i64::from(steps) * SCALE;
    if denominator == 0 || value.wrapping_add(0x7fff_ffff_ffff_fffe) as u64 >= 0xffff_ffff_ffff_fffd
    {
        i64::from(u32::MAX)
    } else {
        value.wrapping_mul(SCALE) / denominator
    }
}

/// x86 `CVTTSS2SI` uses the signed minimum for an invalid conversion rather
/// than Rust's saturating float conversion.
fn truncate(value: f32) -> i32 {
    if !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

/// Submit points in source order. `Some` runs the offset-front intersection
/// pass with the content's `ROOT_FRONT_MAX_INTERSECTION_TESTS_*` value;
/// negative values mean the whole existing list, as the native reader does.
pub(super) fn controls(points: &[Point], intersection_tests: Option<i32>) -> Vec<Point> {
    let mut controls = Vec::new();
    for &point in points {
        let count = controls.len();
        if count >= POINT_CAP {
            continue;
        }
        if count < 2 {
            controls.extend([point, point]);
            continue;
        }
        // Base the threshold on the count before this submission, rather than its
        // chord length or the final control count.
        let fraction = (count as i64 * 10_000_000_000 / 4_000_000).min(SCALE);
        let threshold = fraction * 300_000 / SCALE + 800;
        let steps;
        if squared_delta(point, controls[count - 2]) > threshold {
            let last = controls[count - 1];
            steps = truncate(
                (squared_delta(point, last) as f32 / SCALE as f32).sqrt()
                    / (threshold as f32 / SCALE as f32).sqrt(),
            );
            // Test the wrapping signed step count minus one, including CVTTSS2SI's
            // exceptional minimum result.
            let insertions = steps.wrapping_sub(1);
            if insertions > 0 {
                let step = [
                    divided_step(point[0].wrapping_sub(last[0]), steps),
                    divided_step(point[1].wrapping_sub(last[1]), steps),
                ];
                let mut inserted = last;
                for _ in 0..insertions {
                    inserted = [
                        inserted[0].wrapping_add(step[0]),
                        inserted[1].wrapping_add(step[1]),
                    ];
                    controls.push(inserted);
                    if controls.len() >= POINT_CAP - 1 {
                        break;
                    }
                }
            }
            controls.push(point);
        } else {
            controls[count - 1] = point;
            steps = 1;
        }
        if let Some(limit) = intersection_tests {
            remove_loop(&mut controls, threshold, steps, limit);
        }
    }
    controls
}

fn world(point: Point) -> [f32; 2] {
    point.map(|value| value as f32 / SCALE as f32)
}

/// Closed segment intersection. Nonparallel lines meet only when both
/// parameters are in [0,1]; no epsilon adjustment applies.
fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let (a, b, c, d) = (world(a), world(b), world(c), world(d));
    let ab = [b[0] - a[0], b[1] - a[1]];
    let cd = [d[0] - c[0], d[1] - c[1]];
    let denominator = cd[1] * ab[0] - cd[0] * ab[1];
    if denominator == 0.0 {
        return false;
    }
    let ca = [c[0] - a[0], c[1] - a[1]];
    let t = (ca[0] * cd[1] - ca[1] * cd[0]) / denominator;
    let u = (ca[0] * ab[1] - ca[1] * ab[0]) / denominator;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
}

/// Forward loop removal. The first crossing removes the earlier segment's
/// start through the later segment's end-exclusive range. Do not insert the
/// computed crossing, reverse the list, or repeat the search after removal.
fn remove_loop(controls: &mut Vec<Point>, threshold: i64, steps: i32, limit: i32) {
    let count = controls.len();
    if count < 3 {
        return;
    }
    let start = |back: i32| {
        if back < 0 {
            0
        } else {
            count.saturating_sub(back as usize + 1)
        }
    };
    let first = start(limit);
    let recent = start(steps);
    for earlier in first..count - 1 {
        for later in recent.max(earlier + 2)..count - 1 {
            if intersects(
                controls[earlier],
                controls[earlier + 1],
                controls[later],
                controls[later + 1],
            ) {
                controls.drain(earlier..later + 1);
                return;
            }
        }
    }
    // Also trim a near-closed tail. Start at control zero even when the crossing
    // scan starts later. Test the budget after each distance test, so zero still
    // tests the first point.
    let budget = if limit < 0 { count } else { limit as usize };
    let last = world(controls[count - 1]);
    let maximum_squared = threshold as f32 / SCALE as f32 * 1.5;
    for earlier in 0..count.saturating_sub(4) {
        let point = world(controls[earlier]);
        let dx = point[0] - last[0];
        let dz = point[1] - last[1];
        if dz * dz + dx * dx <= maximum_squared {
            controls.drain(earlier..count - 1);
            return;
        }
        if earlier >= budget {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i64, z: i64) -> Point {
        [x * SCALE, z * SCALE]
    }

    #[test]
    fn crossing_prunes_at_the_first_native_pair_without_inserting_the_crossing() {
        let mut points = vec![p(0, 0), p(8, 8), p(0, 8), p(8, 0), p(9, 0)];
        remove_loop(&mut points, 0, 3, -1);
        assert_eq!(points, vec![p(8, 0), p(9, 0)]);
        // The last two segments alone cannot see the older crossing.
        let mut points = vec![p(0, 0), p(8, 8), p(0, 8), p(8, 0), p(9, 0)];
        remove_loop(&mut points, 0, 3, 1);
        assert_eq!(points.len(), 5);
    }

    #[test]
    fn zero_crossing_budget_still_checks_the_first_tail_distance() {
        let mut points = vec![p(0, 0), p(4, 0), p(4, 4), p(0, 4), [1, 1]];
        remove_loop(&mut points, 15_760, 1, 0);
        assert_eq!(points, vec![[1, 1]]);
        // Parallel collinear segments are never called an intersection.
        assert!(!intersects(p(0, 0), p(4, 0), p(2, 0), p(6, 0)));
        // Nonparallel endpoint intersections are included.
        assert!(intersects(p(0, 0), p(4, 0), p(4, 0), p(4, 4)));
    }

    #[test]
    fn resubmission_retains_duplicates_densifies_and_caps_the_native_list() {
        assert_eq!(
            controls(&[p(0, 0), [30_000, 0]], None),
            vec![p(0, 0), [30_000, 0]]
        );
        let out = controls(&[p(0, 0), p(10, 0)], None);
        assert_eq!(out.len(), 27);
        assert_eq!(out[0], out[1]);
        assert_eq!(out[2], [40_000, 0]);
        assert_eq!(out.last(), Some(&p(10, 0)));
        let out = controls(&[p(0, 0), p(10_000, 0), p(10_001, 0)], None);
        assert_eq!(out.len(), POINT_CAP);
        assert_eq!(out.last(), Some(&p(10_000, 0)));
    }

    #[test]
    fn squared_distance_keeps_native_per_product_truncation() {
        assert_eq!(squared_delta([224, 224], [0, 0]), 0);
        assert_eq!(squared_delta([317, 317], [0, 0]), 2);
        assert_eq!(truncate(f32::NAN), i32::MIN);
        assert_eq!(truncate(2_147_483_648.0), i32::MIN);
        assert_eq!(truncate(-2_147_483_648.0), i32::MIN);
    }

    #[test]
    fn exceptional_step_count_keeps_the_wrapping_native_insertion_branch() {
        // The third point is far enough from the penultimate control to
        // enter the insertion branch, but its delta from the last control
        // has a negative wrapping square. sqrt -> CVTTSS2SI(MIN), and
        // wrapping MIN - 1 is positive; the ordinary point cap stops it.
        let target = [3_036_900_500, 0];
        let out = controls(&[[0, 0], [-SCALE, 0], target], None);
        assert_eq!(out.len(), POINT_CAP);
        assert_eq!(&out[..4], &[[0, 0], [0, 0], [-50_000, 0], [-SCALE, 0]]);
        assert_eq!(out[4], [-100_001, 0]);
        assert_eq!(out.last(), Some(&target));
    }
}
