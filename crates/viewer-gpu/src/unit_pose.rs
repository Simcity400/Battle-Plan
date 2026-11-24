use crate::{province_map::HeightField, ribbon_geometry::{normalize, sub}};
use content_reader_map::province_geometry::native_fixed_sqrt;
use content_reader_raids::{raids::RaidArrowKind, raids::RaidGraphicsDefines};
const SCALE: i64 = 100_000;
type Point = [i64; 2];
const LAUNCHED: i32 = 4;
const ENDED: i32 = 5;

pub fn clamped_lerp(a: f32, b: f32, t: f32) -> f32 {
    if !(a.is_finite() && b.is_finite() && t.is_finite()) {
        return a + t * (b - a);
    }
    if (0.0 < a || b < 0.0) && (a < 0.0 || 0.0 < b) {
        if t == 1.0 {
            return b;
        }
        let value = t.mul_add(b - a, a);
        return if (1.0 < t) != (a < b) {
            value.min(b)
        } else {
            value.max(b)
        };
    }
    (1.0 - t) * a + b * t
}

pub fn air_slope(x: f32, low: f32, high: f32, steepness: f32, length: f32) -> f32 {
    if length == 0.0 {
        return high;
    }
    let clamped = if x <= length {
        if 0.0 <= x { x } else { 0.0 }
    } else {
        length
    };
    let mut t = clamped / length;
    let u = (1.0 - t) * steepness;
    let base = (1.0 - steepness * steepness).sqrt();
    let denominator = 1.0 - base;
    if denominator != 0.0 {
        t = ((1.0 - u * u).sqrt() - base) / denominator;
    }
    clamped_lerp(low, high, t)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ends {
    pub distance: f32,
    pub first: f32,
    pub last: f32,
}

impl Ends {
    pub fn new(controls: &[Point], width: i64, height: &dyn Fn(Point) -> f32) -> Option<Self> {
        let (&first, &last) = (controls.first()?, controls.last()?);
        let round = |value: i64| {
            if value < 0 {
                ((value - 50_000) / SCALE) as i32
            } else {
                ((value + 50_000) / SCALE) as i32
            }
        };
        let dx = round(first[0]).wrapping_sub(round(last[0]));
        let dz = round(first[1]).wrapping_sub(round(last[1]));
        let width = width as i32;
        let dx = if dx.wrapping_abs() <= width / 2 {
            dx
        } else {
            width.wrapping_sub(dx.wrapping_abs())
        };
        let squared = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz));
        Some(Self {
            distance: native_fixed_sqrt(squared as u32) as f32 / 100_000.0,
            first: height(first),
            last: height(last),
        })
    }
}

pub fn air_height(ends: Ends, i: i32, n: i32, defines: &RaidGraphicsDefines) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let d = ends.distance;
    let reach = d / (defines.air_target_length + defines.air_source_length);
    let scale = if 1.0 < reach { 1.0 } else { reach };
    let top = scale * defines.air_height
        + if ends.first < ends.last {
            ends.last
        } else {
            ends.first
        };
    let x = (i as f32 / n as f32) * d;
    let towards = air_slope(
        d - x,
        ends.last,
        top,
        defines.air_target_steepness,
        scale * defines.air_target_length,
    );
    let from = air_slope(
        x,
        ends.first,
        top,
        defines.air_source_steepness,
        scale * defines.air_source_length,
    );
    if towards < from { towards } else { from }
}

pub fn ballistic_height(ends: Ends, i: i32, n: i32, defines: &RaidGraphicsDefines) -> f32 {
    let d = ends.distance;
    if n == 0 || d == 0.0 {
        return 0.0;
    }
    let mut a = -defines.ballistic_shape;
    if a == 0.0 {
        return 0.0;
    }
    let (h0, h1) = (ends.first, ends.last);
    let rise = h1 - h0;
    let mut b = rise / d - a * d;
    let maximum = defines.ballistic_max_height;
    if maximum < -(b * b) / (a * 4.0) + h0 {
        let climb = if rise <= 0.0 { 0.0 } else { rise };
        let mut apex = climb + 0.01;
        if climb + 0.01 <= maximum - h0 {
            apex = maximum - h0;
        }
        if apex == 0.0 {
            return 0.0;
        }
        let k = d * d * apex;
        let root = ((apex - rise) * k).sqrt() + apex * d;
        a = -(root * root) / (k * d * d);
        b = rise / d - a * d;
    }
    let x = (i as f32 / n as f32) * d;
    x * a * x + x * b + h0
}

pub fn animation_state<'s>(
    stages: &'s [content_reader_raids::raids::RaidAnimationStage],
    progress: f32,
    has_state: &dyn Fn(&str) -> bool,
) -> Option<&'s content_reader_raids::raids::RaidAnimationStage> {
    let mut chosen = None;
    for stage in stages {
        if progress < stage.at_progress_raw as f32 / 100_000.0 {
            break;
        }
        if has_state(&stage.state) {
            chosen = Some(stage);
        }
    }
    chosen
}

pub fn movement_progress(phase: i32, p0: f32, p1: f32, tick: f32) -> f32 {
    match phase {
        LAUNCHED => clamped_lerp(p0, p1, tick),
        ENDED => clamped_lerp(1.0, 1.0, tick),
        _ => 0.0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitPlacement {
    pub position: [f32; 3],
    pub facing: [f32; 3],
    pub scale: f32,
    pub progress: f32,
    pub stages: Vec<content_reader_raids::raids::RaidAnimationStage>,
    pub entity: Option<String>,
    pub seed: u32,
    pub track: UnitTrack,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitTrack {
    controls: Vec<Point>,
    kind: RaidArrowKind,
    ends: Option<Ends>,
    defines: RaidGraphicsDefines,
    progress: [f32; 2],
    start: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPose {
    pub position: [f32; 3],
    pub facing: [f32; 3],
    pub progress: f32,
}

impl UnitTrack {
    #[expect(clippy::too_many_arguments, reason = "the arrow's controls and kind, the heights and defines, and the raid's fixed-point progress")]
    pub fn new(
        controls: &[Point],
        kind: RaidArrowKind,
        heights: &HeightField<'_>,
        defines: &RaidGraphicsDefines,
        travelled: i64,
        distance: i64,
        speed: i64,
        start_offset_raw: i64,
    ) -> Option<Self> {
        if controls.len() < 2 {
            return None;
        }
        let elapsed = elapsed_hours(travelled, speed);
        let remaining = remaining_hours(distance, travelled, speed);
        let length = stored_length(controls);
        let offset = start_offset_raw as f32 / 100_000.0;
        let width = i64::from(heights.width);
        Some(Self {
            controls: controls.to_vec(),
            kind,
            ends: Ends::new(controls, width, &|p| {
                heights.fixed_height(p[0], p[1], width)
            }),
            defines: defines.clone(),
            progress: [
                phase_progress(elapsed, remaining) as f32 / 100_000.0,
                next_hour_progress(elapsed, remaining) as f32 / 100_000.0,
            ],
            start: if offset <= length { offset } else { length } / length,
        })
    }

    pub fn progress(&self, tick: f32) -> f32 {
        movement_progress(LAUNCHED, self.progress[0], self.progress[1], tick)
    }

    pub fn fraction(&self, tick: f32) -> f32 {
        clamped_lerp(self.start, 1.0, self.progress(tick))
    }

    pub fn place(&self, heights: &HeightField<'_>, tick: f32) -> Option<UnitPose> {
        let progress = self.progress(tick);
        let fraction = self.fraction(tick);
        let count = self.controls.len() as i32;
        let defines = &self.defines;
        let height = |i: usize, x: f32, z: f32| match (self.kind, self.ends) {
            (RaidArrowKind::Air, Some(ends)) => air_height(ends, i as i32, count, defines),
            (RaidArrowKind::Ballistic, Some(ends)) => {
                ballistic_height(ends, i as i32, count, defines)
            }
            _ => heights.native_height(x, z),
        };
        let ([x, y, z], tangent) = arrow_point(&self.controls, fraction, &height)?;
        let map_width = heights.width as f32;
        let x = if map_width < x { x - map_width } else { x };
        let offset = defines.unit_offset;
        Some(UnitPose {
            position: [x + offset[0], y + offset[1], z + offset[2]],
            facing: [-tangent[0], -tangent[1], -tangent[2]],
            progress,
        })
    }
}

fn point_along(controls: &[Point], distance: f32) -> (usize, f32) {
    let mut walked = 0.0f32;
    for index in 0..controls.len().saturating_sub(1) {
        let dx = controls[index + 1][0] as f32 / 100_000.0 - controls[index][0] as f32 / 100_000.0;
        let dz = controls[index + 1][1] as f32 / 100_000.0 - controls[index][1] as f32 / 100_000.0;
        let run = (dz * dz + dx * dx).sqrt();
        if run != 0.0 && distance <= run + walked {
            return (index, (distance - walked) / run);
        }
        walked += run;
    }
    (controls.len().saturating_sub(2), 1.0)
}

pub fn stored_length(controls: &[Point]) -> f32 {
    controls
        .windows(2)
        .map(|pair| {
            let dx = pair[1][0] as f32 / 100_000.0 - pair[0][0] as f32 / 100_000.0;
            let dz = pair[1][1] as f32 / 100_000.0 - pair[0][1] as f32 / 100_000.0;
            (dz * dz + dx * dx).sqrt()
        })
        .sum()
}
fn control_point(
    controls: &[Point],
    i: usize,
    height: &dyn Fn(usize, f32, f32) -> f32,
) -> [f32; 3] {
    let x = controls[i][0] as f32 / 100_000.0;
    let z = controls[i][1] as f32 / 100_000.0;
    [x, height(i, x, z), z]
}

fn control_direction(
    controls: &[Point],
    i: usize,
    height: &dyn Fn(usize, f32, f32) -> f32,
) -> [f32; 3] {
    let here = control_point(controls, i, height);
    if i + 1 == controls.len() {
        normalize(sub(here, control_point(controls, i - 1, height)))
    } else {
        normalize(sub(control_point(controls, i + 1, height), here))
    }
}

pub fn arrow_point(
    controls: &[Point],
    u: f32,
    height: &dyn Fn(usize, f32, f32) -> f32,
) -> Option<([f32; 3], [f32; 3])> {
    if controls.len() < 2 {
        return None;
    }
    let (index, t) = if (0.0..=1.0).contains(&u) {
        point_along(controls, stored_length(controls) * u)
    } else {
        (0, 0.0)
    };
    let (a, b) = (
        control_point(controls, index, height),
        control_point(controls, index + 1, height),
    );
    let (da, db) = (
        control_direction(controls, index, height),
        control_direction(controls, index + 1, height),
    );
    let lerp = |p: [f32; 3], q: [f32; 3]| {
        [
            (q[0] - p[0]) * t + p[0],
            (q[1] - p[1]) * t + p[1],
            (q[2] - p[2]) * t + p[2],
        ]
    };
    Some((lerp(a, b), normalize(lerp(da, db))))
}

pub fn straight_controls(
    mut from: [f32; 3],
    mut to: [f32; 3],
    width: f32,
    segments: Option<(f32, i32)>,
) -> Vec<Point> {
    if width * 0.5 < (to[0] - from[0]).abs() {
        if to[0] <= from[0] {
            to[0] += width;
        } else {
            from[0] += width;
        }
    }
    let mut count = 1;
    if let Some((length, maximum)) = segments {
        count = maximum;
        let distance = ((to[1] - from[1]) * (to[1] - from[1])
            + (to[0] - from[0]) * (to[0] - from[0])
            + (to[2] - from[2]) * (to[2] - from[2]))
            .sqrt();
        if 0.0 < length {
            let wanted = (distance / length).ceil() as i32;
            count = if wanted <= maximum {
                wanted.max(1)
            } else {
                maximum
            };
        }
    }
    let mut out = vec![fixed(from)];
    for step in 1..count.max(1) {
        let t = step as f32 / count as f32;
        out.push(fixed([
            (to[0] - from[0]) * t + from[0],
            (to[1] - from[1]) * t + from[1],
            (to[2] - from[2]) * t + from[2],
        ]));
    }
    out.push(fixed(to));
    out
}

pub fn fixed(point: [f32; 3]) -> Point {
    let round = |value: f32| ((f32::from(u8::from(0.0 <= value)) + value * 100_000.0) - 0.5) as i64;
    [round(point[0]), round(point[2])]
}

pub fn unwrap(x: f32, reference: f32, width: f32) -> f32 {
    let mut x = x;
    while width * 0.5 < (reference - x).abs() {
        x += if reference - x < 0.0 { -width } else { width };
    }
    x
}
fn finite(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

pub fn elapsed_hours(travelled: i64, speed: i64) -> i32 {
    let mut value = if speed == 0 || !finite(travelled) {
        0xffff_ffff
    } else {
        let value = travelled.wrapping_mul(SCALE) / speed;
        if value == value / SCALE * SCALE {
            return (value / SCALE) as i32;
        }
        value
    };
    value = (value / SCALE + (value >> 63)) * SCALE;
    (value / SCALE) as i32
}

pub fn remaining_hours(distance: i64, travelled: i64, speed: i64) -> i32 {
    let rest = distance.wrapping_sub(travelled);
    let mut value = if speed == 0 || !finite(rest) {
        0xffff_ffff
    } else {
        let value = rest.wrapping_mul(SCALE) / speed;
        if value == value / SCALE * SCALE {
            return (value / SCALE) as i32;
        }
        value
    };
    value = (value / SCALE + i64::from(0 < value)) * SCALE;
    (value / SCALE) as i32
}

pub fn phase_progress(elapsed: i32, remaining: i32) -> i64 {
    if remaining.wrapping_add(elapsed) < 1 {
        return 0;
    }
    let total = i64::from(remaining.wrapping_add(elapsed)) * SCALE;
    if total != 0 && -0x7fff_ffff_ffff_ffff < i64::from(elapsed) * SCALE {
        i64::from(elapsed) * 10_000_000_000 / total
    } else {
        0xffff_ffff
    }
}

pub fn next_hour_progress(elapsed: i32, remaining: i32) -> i64 {
    let total = i64::from(elapsed.wrapping_add(remaining)) * SCALE;
    let mut next = (i64::from(elapsed) + 1) * SCALE;
    if total == 0 {
        return 0;
    }
    if total < next {
        next = total;
    }
    if next < -0x7fff_ffff_ffff_fffe {
        0xffff_ffff
    } else {
        next * SCALE / total
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    use content_reader_raids::raids::PathArrowDefines;
    pub fn defines() -> RaidGraphicsDefines {
        RaidGraphicsDefines {
            preparing_color: [0.7, 0.7, 0.7, 1.0],
            ready_color: [0.7, 0.7, 0.0, 0.9],
            active_color: [1.0, 0.0, 0.0, 0.9],
            ballistic_shape: 0.02,
            ballistic_max_height: 30.0,
            ballistic_max_segment_length: 1.0,
            ballistic_max_segments: 100,
            air_height: 10.0,
            air_source_steepness: 0.2,
            air_source_length: 50.0,
            air_target_steepness: 0.2,
            air_target_length: 0.0,
            air_max_segment_length: 1.0,
            air_max_segments: 100,
            naval: PathArrowDefines {
                subdivisions: 2,
                sharp_turn_smoothness: 0.0,
                use_midpoints: true,
            },
            land: PathArrowDefines {
                subdivisions: 20,
                sharp_turn_smoothness: 0.1,
                use_midpoints: true,
            },
            unit_base_scale: 2.0,
            unit_offset: [0.0; 3],
            speed_multiplier_raw: 10_000,
        }
    }

    static FLAT: [u8; 256] = [0; 256];
    pub fn flat() -> HeightField<'static> {
        HeightField {
            width: 128,
            height: 2,
            values: &FLAT,
        }
    }

    pub fn line_track(travelled: i64, offset: i64) -> UnitTrack {
        let controls = [[0, 0], [5_000_000, 0]];
        UnitTrack::new(
            &controls,
            RaidArrowKind::Line,
            &flat(),
            &defines(),
            travelled,
            100_000,
            30_000,
            offset,
        )
        .unwrap()
    }

    pub fn test_track() -> (UnitTrack, HeightField<'static>) {
        (line_track(60_000, 1_500_000), flat())
    }
}

pub const PLANE_UNRECORDED: &str =
    "the raid's travelled distance (RaidRow.travelled) is not in this recording";

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    #[test]
    fn a_straight_arrow_wraps_its_ends_and_cuts_ceil_length_segments() {
        let line = straight_controls([10.0, 5.0, 10.0], [20.0, 5.0, 10.0], 1000.0, None);
        assert_eq!(line, [[1_000_000, 1_000_000], [2_000_000, 1_000_000]]);
        let air = straight_controls(
            [10.0, 5.0, 10.0],
            [20.0, 5.0, 10.0],
            1000.0,
            Some((1.0, 100)),
        );
        assert_eq!(air.len(), 11);
        assert_eq!(air[1], [1_100_000, 1_000_000]);
        let far = straight_controls([0.0; 3], [400.0, 0.0, 0.0], 1000.0, Some((1.0, 100)));
        assert_eq!(far.len(), 101);
        let capped = straight_controls([0.0; 3], [4.0, 0.0, 0.0], 1000.0, Some((0.0, 7)));
        assert_eq!(capped.len(), 8);
        let seam = straight_controls([2.0, 0.0, 0.0], [998.0, 0.0, 0.0], 1000.0, None);
        assert_eq!(seam, [[100_200_000, 0], [99_800_000, 0]]);
    }

    #[test]
    fn a_float_point_rounds_half_away_from_zero() {
        assert_eq!(fixed([0.000005, 0.0, -0.000005]), [1, -1]);
        assert_eq!(fixed([12.345_67, 0.0, -1.0]), [1_234_567, -100_000]);
    }

    #[test]
    fn the_air_arrow_climbs_from_the_source_to_its_cruise_height() {
        let defines = defines();
        let ends = Ends {
            distance: 100.0,
            first: 10.0,
            last: 12.0,
        };
        assert_eq!(air_height(ends, 0, 10, &defines), 10.0);
        assert_eq!(air_height(ends, 6, 10, &defines), 22.0);
        assert_eq!(air_height(ends, 10, 10, &defines), 22.0);
        assert_eq!(air_height(ends, 3, 0, &defines), 0.0);
        let short = Ends {
            distance: 25.0,
            ..ends
        };
        assert_eq!(air_height(short, 10, 10, &defines), 17.0);
    }

    #[test]
    fn the_ballistic_arc_is_refitted_to_its_maximum_height() {
        let mut defines = defines();
        let ends = Ends {
            distance: 100.0,
            first: 0.0,
            last: 0.0,
        };
        assert!((ballistic_height(ends, 5, 10, &defines) - 30.0).abs() < 1e-3);
        assert!(ballistic_height(ends, 0, 10, &defines).abs() < 1e-6);
        assert!(ballistic_height(ends, 10, 10, &defines).abs() < 1e-3);
        defines.ballistic_max_height = 100.0;
        assert!((ballistic_height(ends, 5, 10, &defines) - 50.0).abs() < 1e-3);
        defines.ballistic_shape = 0.0;
        assert_eq!(ballistic_height(ends, 5, 10, &defines), 0.0);
    }

    #[test]
    fn the_callbacks_measure_the_rounded_ends_across_the_seam() {
        let flat = |_: Point| 0.5;
        let ends = Ends::new(&[[0, 0], [100 * SCALE, 0]], 1000, &flat).unwrap();
        assert!((ends.distance - 100.0).abs() < 1e-2);
        let seam = Ends::new(&[[10 * SCALE, 0], [990 * SCALE, 0]], 1000, &flat).unwrap();
        assert!((seam.distance - 20.0).abs() < 1e-2);
        assert_eq!((seam.first, seam.last), (0.5, 0.5));
        assert_eq!(Ends::new(&[], 1000, &flat), None);
    }

    #[test]
    fn the_unit_stands_at_its_start_offset_in_the_launch_hour() {
        assert_eq!(line_track(0, 1_500_000).fraction(0.0), 0.3);
        let half = line_track(60_000, 1_500_000).fraction(0.0);
        assert!((half - 0.65).abs() < 1e-6);
        let ticked = line_track(60_000, 1_500_000).fraction(0.5);
        assert!((ticked - 0.7375).abs() < 1e-6);
        assert_eq!(line_track(0, 9_000_000).fraction(0.0), 1.0);
        assert!((stored_length(&[[0, 0], [300_000, 400_000], [300_000, 0]]) - 9.0).abs() < 1e-6);
        let one = [[0, 0]];
        assert_eq!(
            UnitTrack::new(
                &one,
                RaidArrowKind::Line,
                &flat(),
                &defines(),
                0,
                100_000,
                30_000,
                0
            ),
            None
        );
    }

    #[test]
    fn playback_glides_the_unit_by_the_tick_progress_within_the_hour() {
        let track = line_track(60_000, 1_500_000);
        let heights = flat();
        let start = track.place(&heights, 0.0).unwrap();
        for tick in [0.25f32, 0.5, 0.75] {
            let pose = track.place(&heights, tick).unwrap();
            let progress = 0.5 + 0.25 * tick;
            assert!((pose.progress - progress).abs() < 1e-6, "{tick}: {pose:?}");
            let fraction = 0.3 + 0.7 * progress;
            let expected = arrow_point(&[[0, 0], [5_000_000, 0]], fraction, &|_, x, z| {
                heights.native_height(x, z)
            })
            .unwrap()
            .0;
            let offset = defines().unit_offset;
            for axis in 0..3 {
                assert!(
                    (pose.position[axis] - (expected[axis] + offset[axis])).abs() < 1e-4,
                    "{tick}: {pose:?} against {expected:?}"
                );
            }
            assert!(pose.position[0] > start.position[0]);
            assert_eq!(pose.facing, start.facing);
        }
        let (a, b) = (
            start.position[0],
            track.place(&heights, 1.0).unwrap().position[0],
        );
        assert!((track.place(&heights, 0.5).unwrap().position[0] - (a + b) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn the_arrow_point_interpolates_controls_heights_and_tangents() {
        let controls = [[0, 0], [10 * SCALE, 0], [10 * SCALE, 10 * SCALE]];
        let height = |i: usize, _: f32, _: f32| i as f32;
        let (at, tangent) = arrow_point(&controls, 0.25, &height).unwrap();
        assert_eq!(at, [5.0, 0.5, 0.0]);
        let (a, b) = (normalize([10.0, 1.0, 0.0]), normalize([0.0, 1.0, 10.0]));
        let expected = normalize([
            (a[0] + b[0]) * 0.5,
            (a[1] + b[1]) * 0.5,
            (a[2] + b[2]) * 0.5,
        ]);
        assert!((0..3).all(|axis| (tangent[axis] - expected[axis]).abs() < 1e-6));
        let (at, _) = arrow_point(&controls, 0.75, &height).unwrap();
        assert_eq!(at, [10.0, 1.5, 5.0]);
        assert_eq!(
            arrow_point(&controls, 1.5, &height).unwrap().0,
            [0.0, 0.0, 0.0]
        );
        assert_eq!(arrow_point(&controls[..1], 0.5, &height), None);
    }

    #[test]
    fn the_animation_stage_is_the_last_reached_one_the_entity_has() {
        use content_reader_raids::raids::RaidAnimationStage;
        let stage = |state: &str, at: i64| RaidAnimationStage {
            state: state.into(),
            intro: None,
            at_progress_raw: at,
        };
        let stages = [
            stage("idle", 0),
            stage("idle2", 33_000),
            stage("idle3", 66_000),
        ];
        let all = |_: &str| true;
        let pick = |progress, has: &dyn Fn(&str) -> bool| {
            animation_state(&stages, progress, has).map(|stage| stage.state.as_str())
        };
        assert_eq!(pick(0.0, &all), Some("idle"));
        assert_eq!(pick(0.329, &all), Some("idle"));
        assert_eq!(pick(0.33, &all), Some("idle2"));
        assert_eq!(pick(0.9, &all), Some("idle3"));
        let basic = |name: &str| name != "idle3";
        assert_eq!(pick(0.9, &basic), Some("idle2"));
        let none = |_: &str| false;
        assert_eq!(pick(0.9, &none), None);
        assert_eq!(animation_state(&[stage("late", 50_000)], 0.1, &all), None);
        assert_eq!(line_track(0, 0).progress(0.0), 0.0);
    }
}
