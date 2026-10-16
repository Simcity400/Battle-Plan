//! The engine's fixed5 (`x 100000`) vector arithmetic the battle-plan
//! geometry is built in. Truncating signed division and the per-product
//! truncation order are kept, because they decide ties and signs near zero.

/// One fixed5 world unit.
pub const SCALE: i64 = 100_000;

/// A fixed5 `(x, z)` map point.
pub type Point = [i64; 2];

/// A smoothing define pair: `*_SMOOTHNESS` (fixed5) and `*_SMOOTHEN_PASSES`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Smoothing {
    pub smoothness: i64,
    pub passes: u32,
}

/// Floor integer square root, then signed fixed5 division. The zero vector
/// stays zero; a zero length after the root gives 0xFFFFFFFF lanes.
pub fn normalize([x, z]: Point) -> Point {
    if x == 0 && z == 0 {
        return [0, 0];
    }
    let squared = z.wrapping_mul(z).wrapping_add(x.wrapping_mul(x));
    let length = if squared > 1 {
        squared.isqrt()
    } else {
        squared
    };
    let denominator = i64::from(length as i32);
    if denominator == 0 {
        return [i64::from(u32::MAX); 2];
    }
    [x, z].map(|value| {
        if value.wrapping_add(0x7fff_ffff_ffff_fffe) as u64 >= 0xffff_ffff_ffff_fffd {
            i64::from(u32::MAX)
        } else {
            value.wrapping_mul(SCALE) / denominator
        }
    })
}

/// Divide a vector by `count` fixed5 units, truncating each component.
pub fn divide([x, z]: Point, count: i64) -> Point {
    let denominator = count * SCALE;
    if denominator == 0 {
        return [i64::from(u32::MAX); 2];
    }
    [x, z].map(|value| value.wrapping_mul(SCALE) / denominator)
}

/// The vertical cross-product component for inputs with zero vertical lanes.
/// Divide each product by 100000 before subtracting.
pub fn cross_y(a: Point, b: Point) -> i64 {
    a[1] * b[0] / SCALE - a[0] * b[1] / SCALE
}

/// Rotate by 0x26598 (157080, a quarter turn) using the fixed5 polynomial
/// sine, truncating both products before combining. At 157080 the sine is
/// 100453, so this is not an exact quarter turn.
pub fn rotate_quarter([x, z]: Point) -> Point {
    let sine = fixed_sine(157_080);
    let cosine = fixed_sine(314_160);
    [
        x * cosine / SCALE - z * sine / SCALE,
        z * cosine / SCALE + x * sine / SCALE,
    ]
}

/// Fixed5 polynomial sine.
fn fixed_sine(radians: i64) -> i64 {
    let phase = radians.rem_euclid(628_319);
    let mut x = if phase > 314_159 {
        phase - 314_159
    } else {
        phase
    };
    if x > 157_080 {
        x = 314_160 - x;
    }
    let cube = (x * x / SCALE) * x / SCALE;
    let fifth = (cube * x / SCALE) * x / SCALE;
    let sine = x - cube / 6 + fifth / 120;
    if phase > 314_159 { -sine } else { sine }
}

/// Round half away from zero.
pub fn round(value: i64) -> i64 {
    (value + if value < 0 { -50_000 } else { 50_000 }) / SCALE
}

/// Move every interior point towards the midpoint of its old neighbours.
pub fn smooth(points: &mut [Point], smoothness: i64) {
    if points.len() < 3 {
        return;
    }
    let source = points.to_vec();
    for index in 1..source.len() - 1 {
        for axis in 0..2 {
            let mid = (source[index - 1][axis] + source[index + 1][axis]) * 50_000 / SCALE;
            points[index][axis] =
                (SCALE - smoothness) * source[index][axis] / SCALE + mid * smoothness / SCALE;
        }
    }
}

/// Insert a midpoint at each odd index using a fixed5 half-scale product of
/// the component difference, with wrapping multiplication and truncating division.
pub fn subdivide(points: &mut Vec<Point>) {
    if points.len() < 2 {
        return;
    }
    let mut out = Vec::with_capacity(points.len() * 2);
    for (index, &point) in points.iter().enumerate() {
        if index > 0 {
            let previous = points[index - 1];
            out.push([
                previous[0] + (point[0] - previous[0]) * 50_000 / SCALE,
                previous[1] + (point[1] - previous[1]) * 50_000 / SCALE,
            ]);
        }
        out.push(point);
    }
    *points = out;
}

/// Smooth once, then repeat `passes` times: subdivide when `subdivided`,
/// then smooth. Centroid paths always subdivide; border chains use the flag.
pub fn curve(points: &mut Vec<Point>, smoothing: Smoothing, subdivided: bool) {
    smooth(points, smoothing.smoothness);
    for _ in 0..smoothing.passes {
        if subdivided {
            subdivide(points);
        }
        smooth(points, smoothing.smoothness);
    }
}

pub fn from_pixel([x, z]: [i32; 2]) -> Point {
    [i64::from(x) * SCALE, i64::from(z) * SCALE]
}

pub fn to_world([x, z]: Point) -> [f32; 2] {
    [x as f32 / SCALE as f32, z as f32 / SCALE as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_rotation_and_rounding_keep_native_integer_laws() {
        assert_eq!(normalize([0, 0]), [0, 0]);
        assert_eq!(normalize([3, 4]), [60_000, 80_000]);
        assert_eq!(normalize([SCALE, SCALE]), [70_710, 70_710]);
        assert_eq!(rotate_quarter([SCALE, 0]), [-1, 100_453]);
        assert_eq!(round(-50_000), -1);
        assert_eq!(round(-49_999), 0);
        assert_eq!(round(50_000), 1);
        assert_eq!(divide([7, -7], 2), [3, -3]);
    }

    #[test]
    fn fixed_smoothing_truncates_each_product() {
        let mut points = vec![[0, 0], [10 * SCALE, 10 * SCALE], [20 * SCALE, 0]];
        smooth(&mut points, 50_000);
        assert_eq!(points[1], [10 * SCALE, 5 * SCALE]);
        let mut points = vec![[0, 0], [3, 0]];
        subdivide(&mut points);
        assert_eq!(points, vec![[0, 0], [1, 0], [3, 0]]);
        let mut points = vec![[0, 0], [SCALE, 0], [2 * SCALE, 0], [3 * SCALE, 0]];
        curve(
            &mut points,
            Smoothing {
                smoothness: 50_000,
                passes: 2,
            },
            true,
        );
        assert_eq!(points.len(), 13);
        let mut points = vec![[0, 0], [SCALE, SCALE], [2 * SCALE, 0]];
        curve(
            &mut points,
            Smoothing {
                smoothness: 50_000,
                passes: 2,
            },
            false,
        );
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn the_cross_lane_sign_decides_the_side() {
        // Tangent east, move north: y of east x north.
        assert!(cross_y([SCALE, 0], [0, SCALE]) < 0);
        assert!(cross_y([0, SCALE], [SCALE, 0]) > 0);
    }
}
