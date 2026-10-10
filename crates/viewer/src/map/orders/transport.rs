//! Paradrop arrow geometry and lift.
//!
//! The source is `ArmyOrders.paradrop_target` (the save's `invasion_source`);
//! the target is the first path province. Densify one straight source-to-target
//! segment and lift it into a sine arc without smoothing.
//!
//! An invasion follows the source, sea route, then the first path province.
//! Wrap each centroid against the previous control, append the raw adjacency
//! crossing, then apply move smoothing. The recorded move world supplies the
//! sea route through its mode-0 path search.

use super::fixed::{Point, SCALE};

/// Append the centroid then raw crossing, wrapping each inserted point
/// against the preceding one before ORDER_MOVE smoothing.
pub fn invasion_controls(
    path: &[u32],
    width: i64,
    centroid: impl Fn(u32) -> Option<Point>,
    crossing: impl Fn(u32, u32) -> Option<Point>,
) -> Option<Vec<Point>> {
    let mut out: Vec<Point> = Vec::with_capacity(path.len() * 2);
    let mut push = |mut point: Point| {
        if let Some(previous) = out.last() {
            wrap_x(previous[0], &mut point[0], width);
        }
        out.push(point);
    };
    for (at, &province) in path.iter().enumerate() {
        push(centroid(province)?);
        if let Some(&next) = path.get(at + 1)
            && let Some(point) = crossing(province, next)
        {
            push(point);
        }
    }
    Some(out)
}

/// Bring `x` within half the map width of `reference`, both fixed5, with
/// `width` in whole pixels. Use integer half-width and strict greater-than ties.
fn wrap_x(reference: i64, x: &mut i64, width: i64) {
    if width <= 0 {
        return;
    }
    let half = (width / 2) * SCALE;
    while (reference - *x).abs() > half {
        *x += if reference - *x < 0 { -width } else { width } * SCALE;
    }
}

/// Paradrop stored controls from the source and target centroids (fixed5).
/// The first builder call pushes the source twice; the second splits the
/// segment into threshold-length steps, or replaces the last control with the
/// target when the segment is shorter.
pub fn paradrop_controls(source: Point, mut target: Point, width: i64) -> Vec<Point> {
    wrap_x(source[0], &mut target[0], width);
    super::submit::controls(&[source, target], None)
}

/// Paradrop height inputs, fixed once per arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lift {
    /// Terrain height under the first and the last control.
    pub from: f32,
    pub to: f32,
    /// `min(length * ARROW_PARADROP_HEIGHT_TO_LEN, ARROW_PARADROP_HEIGHT_MAX)`.
    pub amplitude: f32,
}

impl Lift {
    /// Per-arrow terms: chord length from the integer square root of the raw
    /// fixed5 squares, and sea-clamped terrain height at each end, with x wrapped
    /// once into the map.
    pub fn new(
        controls: &[Point],
        width: f32,
        to_length: f32,
        maximum: f32,
        height: &dyn Fn(f32, f32) -> f32,
    ) -> Option<Self> {
        if controls.len() <= 1 {
            return None;
        }
        let (first, last) = (controls[0], controls[controls.len() - 1]);
        let (dx, dz) = (first[0] - last[0], first[1] - last[1]);
        let squared = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz));
        // Skip the square-root loop for signed values <= 1, then narrow the result
        // to signed32 even after fixed5 overflow.
        let root = if squared > 1 {
            squared.isqrt()
        } else {
            squared
        };
        let length = (root as i32) as f32 / SCALE as f32;
        let wrap = |x: f32| {
            if x >= width {
                x - width
            } else if x < 0.0 {
                x + width
            } else {
                x
            }
        };
        let at = |point: Point| {
            height(
                wrap(point[0] as f32 / SCALE as f32),
                point[1] as f32 / SCALE as f32,
            )
        };
        Some(Self {
            from: at(first),
            to: at(last),
            amplitude: (length * to_length).min(maximum),
        })
    }

    /// The height at `t`, the strip sample's fraction along the arrow:
    /// `h0 + (h1 - h0) * t + sin(t * pi) * amplitude`.
    pub fn at(&self, t: f32) -> f32 {
        self.from
            + (self.to - self.from) * t
            + (t * f32::from_bits(0x4049_0FDB)).sin() * self.amplitude
    }

    /// Use stored-control indices, including the two head samples, rather than
    /// a fraction of the ribbon's spatial length.
    pub fn at_control(&self, index: usize, count: usize, has_head: bool) -> f32 {
        self.at(index as f32 / (count + if has_head { 2 } else { 0 }) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: i64 = 5632;

    #[test]
    fn invasion_controls_wrap_each_centroid_and_raw_crossing_in_order() {
        let centers = [
            (1, [2 * SCALE, 0]),
            (2, [(W - 2) * SCALE, SCALE]),
            (3, [(W - 6) * SCALE, 0]),
        ];
        assert_eq!(
            invasion_controls(
                &[1, 2, 3],
                W,
                |id| centers.iter().find(|c| c.0 == id).map(|c| c.1),
                |a, b| (a == 1 && b == 2).then_some([(W - 1) * SCALE, 0])
            ),
            Some(vec![
                [2 * SCALE, 0],
                [-SCALE, 0],
                [-2 * SCALE, SCALE],
                [-6 * SCALE, 0]
            ])
        );
        assert_eq!(
            invasion_controls(
                &[1, 9],
                W,
                |id| centers.iter().find(|c| c.0 == id).map(|c| c.1),
                |_, _| None
            ),
            None
        );
    }

    #[test]
    fn the_wrap_brings_a_point_within_half_the_map() {
        let mut x = (W - 4) * SCALE;
        wrap_x(4 * SCALE, &mut x, W);
        assert_eq!(x, -4 * SCALE);
        let mut x = 100 * SCALE;
        wrap_x(4 * SCALE, &mut x, W);
        assert_eq!(x, 100 * SCALE);
        // Exactly half a map is not past it.
        let mut x = (4 + W / 2) * SCALE;
        wrap_x(4 * SCALE, &mut x, W);
        assert_eq!(x, (4 + W / 2) * SCALE);
    }

    #[test]
    fn a_short_paradrop_is_its_two_ends_and_a_long_one_is_densified() {
        let source = [10 * SCALE, 10 * SCALE];
        // 0.3 px: 9000 squared fixed5, under the 15760 threshold.
        let near = [10 * SCALE + 30_000, 10 * SCALE];
        assert_eq!(paradrop_controls(source, near, W), vec![source, near]);
        // 10 px: sqrt(100) / sqrt(0.1576) = 25.19 -> 25 steps of 0.4 px.
        let far = [20 * SCALE, 10 * SCALE];
        let controls = paradrop_controls(source, far, W);
        assert_eq!(controls.len(), 2 + 24 + 1);
        assert_eq!(controls[0], source);
        assert_eq!(controls[1], source);
        assert_eq!(controls[2], [10 * SCALE + 40_000, 10 * SCALE]);
        assert_eq!(controls[controls.len() - 1], far);
    }

    #[test]
    fn a_paradrop_across_the_seam_takes_the_short_way() {
        let source = [2 * SCALE, 0];
        let target = [(W - 2) * SCALE, 0];
        let controls = paradrop_controls(source, target, W);
        assert_eq!(controls[controls.len() - 1], [-2 * SCALE, 0]);
    }

    #[test]
    fn the_lift_is_a_sine_arc_over_the_terrain_line() {
        let controls = [[0, 0], [0, 0], [100 * SCALE, 0]];
        let height = |x: f32, _z: f32| x / 100.0;
        let lift = Lift::new(&controls, W as f32, 0.1, 5.0, &height).unwrap();
        assert_eq!(lift.amplitude, 5.0);
        assert_eq!((lift.from, lift.to), (0.0, 1.0));
        assert_eq!(lift.at(0.0), 0.0);
        assert!((lift.at(0.5) - 5.5).abs() < 1e-5);
        assert!((lift.at(1.0) - 1.0).abs() < 1e-5);
        let short = Lift::new(&[[0, 0], [10 * SCALE, 0]], W as f32, 0.1, 5.0, &height).unwrap();
        assert!((short.amplitude - 1.0).abs() < 1e-6);
        assert_eq!(Lift::new(&[[0, 0]], W as f32, 0.1, 5.0, &height), None);
    }

    #[test]
    fn wrapped_negative_length_preserves_native_signed32_narrowing() {
        let lift = Lift::new(
            &[[0, 0], [3_i64 << 30, 12_345]],
            90_000.0,
            1.0,
            f32::MAX,
            &|_, _| 0.0,
        )
        .unwrap();
        // The wrapped signed64 squared sum is negative. Native skips sqrt
        // and its low signed32 word is 12345^2, not a clamped zero.
        assert_eq!(lift.amplitude, 152_399_025_i32 as f32 / SCALE as f32);
    }

    #[test]
    fn coincident_ends_have_no_arc_and_keep_the_terrain_height() {
        let lift = Lift::new(&[[0, 0], [0, 0]], W as f32, 0.1, 5.0, &|_, _| 3.0).unwrap();
        assert_eq!(lift.amplitude, 0.0);
        for index in 0..=4 {
            assert_eq!(lift.at_control(index, 2, true), 3.0);
        }
    }
}
