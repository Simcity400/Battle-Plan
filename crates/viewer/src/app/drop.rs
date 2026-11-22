
use viewer_gpu::camera::{Camera, WATER_HEIGHT, frustum::TERRAIN_TOP};
use viewer_gpu::province_map::HeightField;

const TOUCH: f32 = 1e-4;
const BISECTIONS: u32 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Pick {
    #[default]
    Idle,
    Choosing,
    At([f32; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Choice {
    Wait,
    Cancel,
    Start([f32; 2]),
}

pub(super) fn choose(
    escape: bool,
    clicked: Option<[f32; 3]>,
    south: bool,
    centre: Option<[f32; 3]>,
    width: f32,
) -> Choice {
    if escape {
        return Choice::Cancel;
    }
    match clicked.or(centre.filter(|_| south)) {
        Some([x, _, z]) => Choice::Start([x.rem_euclid(width.max(1.0)), z]),
        None => Choice::Wait,
    }
}

pub(super) fn ground_at(
    camera: &Camera,
    screen: [f32; 2],
    viewport: [u32; 2],
    field: &HeightField<'_>,
) -> Option<[f32; 3]> {
    let direction = camera.pick_ray(screen, viewport);
    let forward = camera.look_direction();
    let depth: f32 = (0..3).map(|axis| direction[axis] * forward[axis]).sum();
    if depth.is_nan() || depth <= 0.0 {
        return None;
    }
    first_meeting(
        camera.eye(),
        direction,
        camera.lens().far / depth,
        camera.map.y(),
        &|x, z| field.surface_height(x, z),
    )
}

fn within(origin: f32, slope: f32, low: f32, high: f32) -> Option<[f32; 2]> {
    if slope == 0.0 {
        return (low..=high)
            .contains(&origin)
            .then_some([f32::NEG_INFINITY, f32::INFINITY]);
    }
    let [a, b] = [(low - origin) / slope, (high - origin) / slope];
    Some([a.min(b), a.max(b)])
}

fn first_meeting(
    eye: [f32; 3],
    direction: [f32; 3],
    far: f32,
    rows: f32,
    ground: &dyn Fn(f32, f32) -> f32,
) -> Option<[f32; 3]> {
    let [dx, dy, dz] = direction;
    if !eye.iter().chain(&direction).all(|value| value.is_finite()) || far.is_nan() || far <= 0.0 {
        return None;
    }
    let [low_y, high_y] = within(eye[1], dy, f32::NEG_INFINITY, TERRAIN_TOP)?;
    let [low_z, high_z] = within(eye[2], dz, 0.0, rows)?;
    let start = low_y.max(low_z).max(0.0);
    let end = high_y.min(high_z).min(far);
    if start.is_nan() || end.is_nan() || start > end {
        return None;
    }
    let mut origin: [f32; 3] = std::array::from_fn(|axis| eye[axis] + direction[axis] * start);
    if start == low_y && start > 0.0 {
        origin[1] = TERRAIN_TOP;
    }
    let [ox, oy, oz] = origin;
    let mut end = end - start;
    let at = |t: f32| [ox + dx * t, oy + dy * t, oz + dz * t];
    let over_ground = |t: f32| {
        let [x, y, z] = at(t);
        y - ground(x, z)
    };
    let place = |t: f32| {
        let [x, _, z] = at(t);
        [x, ground(x, z).max(WATER_HEIGHT), z]
    };
    let entry = over_ground(0.0).min(oy - WATER_HEIGHT);
    if entry < -TOUCH {
        return None;
    }
    if entry <= TOUCH {
        return Some(place(0.0));
    }
    let water = (dy < 0.0)
        .then(|| (oy - WATER_HEIGHT) / -dy)
        .filter(|t| *t <= end);
    if let Some(water) = water {
        end = water;
    }
    let families = [(ox, dx), (oz, dz), (ox - oz, dx - dz), (ox + oz, dx + dz)];
    let mut lines = families.map(|(value, slope)| {
        if slope > 0.0 {
            value.floor() + 1.0
        } else if slope < 0.0 {
            value.ceil() - 1.0
        } else {
            f32::NAN
        }
    });
    let crossing = |family: usize, line: f32| {
        let (value, slope) = families[family];
        if line.is_nan() {
            f32::INFINITY
        } else {
            (line - value) / slope
        }
    };
    let mut a = 0.0;
    let mut above = over_ground(a);
    while a < end {
        let (family, next) = (0..4)
            .map(|family| (family, crossing(family, lines[family])))
            .fold((0, f32::INFINITY), |best, candidate| {
                if candidate.1 < best.1 {
                    candidate
                } else {
                    best
                }
            });
        let b = next.min(end);
        let below = over_ground(b);
        if below <= 0.0 {
            let [x, _, z] = at(a);
            if ground(x, z) > WATER_HEIGHT {
                return Some(place(a + (b - a) * above / (above - below)));
            }
            let (mut low, mut high) = (a, b);
            for _ in 0..BISECTIONS {
                let middle = 0.5 * (low + high);
                if over_ground(middle) > 0.0 {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return Some(place(high));
        }
        if next >= end {
            break;
        }
        lines[family] += families[family].1.signum();
        (a, above) = (b, below);
    }
    water.map(place)
}

impl Pick {
    pub(super) fn at(self) -> Option<[f32; 2]> {
        match self {
            Self::At(at) => Some(at),
            _ => None,
        }
    }

    pub(super) fn choosing(self) -> bool {
        self == Self::Choosing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_gpu::camera::{Free, Lens, MapDefines, MapSize, Pose};

    const WIDTH: u32 = 128;
    const VIEWPORT: [u32; 2] = [640, 480];

    fn heights(rows: u32, byte: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        (0..rows)
            .flat_map(|row| (0..WIDTH).map(move |column| (column, rows - 1 - row)))
            .map(|(column, row)| byte(column, row))
            .collect()
    }

    fn field(rows: u32, values: &[u8]) -> HeightField<'_> {
        HeightField {
            width: WIDTH,
            height: rows,
            values,
        }
    }

    fn map_camera(rows: u32, center: [f32; 2], height: f32) -> Camera {
        Camera {
            center,
            height,
            map: MapSize {
                width: WIDTH,
                height: rows,
            },
            defines: MapDefines::test(),
            pose: Pose::Map,
        }
    }

    fn free_camera(rows: u32, eye: [f32; 3], yaw: f32, pitch: f32, lens: Lens) -> Camera {
        Camera {
            center: [eye[0], eye[2]],
            height: eye[1],
            pose: Pose::Free(Free::new(yaw.to_radians(), pitch.to_radians(), lens)),
            ..map_camera(rows, [0.0; 2], 0.0)
        }
    }

    fn surface(field: &HeightField<'_>, x: f32, z: f32) -> f32 {
        field.surface_height(x, z).max(WATER_HEIGHT)
    }

    fn assert_nearest(camera: &Camera, screen: [f32; 2], field: &HeightField<'_>) -> [f32; 3] {
        let point = ground_at(camera, screen, VIEWPORT, field)
            .unwrap_or_else(|| panic!("ground under {screen:?}"));
        assert!(
            (point[1] - surface(field, point[0], point[2])).abs() < 1e-3,
            "{point:?}"
        );
        let shown = camera.project(point, VIEWPORT).unwrap();
        assert!(
            (shown[0] - screen[0]).abs() < 0.5 && (shown[1] - screen[1]).abs() < 0.5,
            "{screen:?} finds {point:?}, drawn at {shown:?}"
        );
        let eye = camera.eye();
        let rows = camera.map.y();
        let reach: f32 = (0..3)
            .map(|axis| (point[axis] - eye[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        let direction = camera.pick_ray(screen, VIEWPORT);
        let samples = 200_000;
        for sample in 0..samples {
            let t = reach * sample as f32 / samples as f32 - 0.01;
            let [x, y, z] = std::array::from_fn(|axis| eye[axis] + direction[axis] * t);
            if t > 0.0 && (0.0..=rows).contains(&z) {
                assert!(
                    y - surface(field, x, z) > -1e-3,
                    "{screen:?}: the ray meets the surface at {:?} before {point:?}",
                    [x, y, z]
                );
            }
        }
        point
    }

    #[test]
    fn the_nearest_ground_is_found_over_water_cliffs_and_rough_ground() {
        let water = heights(64, |_, _| 0);
        let camera = map_camera(64, [64.0, 32.0], 50.0);
        for screen in [[320.0, 240.0], [5.0, 470.0]] {
            assert_eq!(
                assert_nearest(&camera, screen, &field(64, &water))[1],
                WATER_HEIGHT
            );
        }
        let raised = heights(64, |_, _| 255);
        assert_eq!(
            assert_nearest(&camera, [320.0, 1.0], &field(64, &raised))[1],
            TERRAIN_TOP
        );
        let cliff = heights(64, |_, row| if row >= 25 { 255 } else { 95 });
        let point = assert_nearest(&camera, [320.0, 240.0], &field(64, &cliff));
        assert!(point[2] > 24.0 && point[2] < 25.0, "{point:?}");
        let rough = heights(64, |column, row| {
            ((column.wrapping_mul(73_856_093) ^ row.wrapping_mul(19_349_663))
                .wrapping_mul(2_654_435_761)
                >> 24) as u8
        });
        let low = free_camera(
            64,
            [40.3, 30.0, 3.7],
            20.0,
            -14.0,
            crate::controls::orbit::LENS,
        );
        for screen in [
            [320.0, 240.0],
            [100.0, 300.0],
            [600.0, 420.0],
            [30.0, 200.0],
        ] {
            assert_nearest(&low, screen, &field(64, &rough));
        }
    }

    #[test]
    fn a_coast_and_a_high_eye_meet_the_ground_where_it_is_drawn() {
        let coast = heights(64, |_, row| if row >= 25 { 255 } else { 0 });
        let camera = map_camera(64, [64.0, 32.0], 50.0);
        let point = assert_nearest(&camera, [320.0, 240.0], &field(64, &coast));
        assert!(point[1] > WATER_HEIGHT && point[2] < 25.0, "{point:?}");
        let raised = heights(64, |_, _| 255);
        let high = map_camera(64, [64.0, 32.0], 2048.0);
        assert_eq!(
            assert_nearest(&high, [1.0, 239.0], &field(64, &raised))[1],
            TERRAIN_TOP
        );
        let orbit = free_camera(
            64,
            [64.3, 3000.0, -1700.0],
            0.0,
            -60.0,
            crate::controls::orbit::LENS,
        );
        assert_eq!(
            assert_nearest(&orbit, [320.0, 240.0], &field(64, &raised))[1],
            TERRAIN_TOP
        );
    }

    #[test]
    fn a_grazing_ray_meets_the_ridge_in_front_of_the_water_behind_it() {
        let ridge = heights(128, |_, row| if row == 25 { 255 } else { 95 });
        let camera = free_camera(
            128,
            [64.0, 50.0, -90.31048],
            0.0,
            -12.0,
            crate::controls::orbit::LENS,
        );
        let point = assert_nearest(&camera, [320.0, 240.0], &field(128, &ridge));
        assert!(point[2] > 24.0 && point[2] < 26.0, "{point:?}");
        assert!(point[1] > 25.0, "{point:?}");
    }

    #[test]
    fn the_rows_are_entered_at_their_edges() {
        let level = heights(64, |_, _| 96);
        let south = map_camera(64, [64.0, 0.1], 50.0);
        let point = assert_nearest(&south, [320.0, 240.0], &field(64, &level));
        assert!(point[2] > 0.0 && point[2] < 0.5, "{point:?}");
        let north = free_camera(64, [64.3, 12.0, 66.0], 180.0, -30.0, Lens::MAP);
        let point = assert_nearest(&north, [320.0, 240.0], &field(64, &level));
        assert!(point[2] > 60.0 && point[2] <= 64.0, "{point:?}");
        let under = free_camera(64, [64.3, 9.9, 64.5], 180.0, -60.0, Lens::MAP);
        assert_eq!(
            ground_at(&under, [320.0, 240.0], VIEWPORT, &field(64, &level)),
            None
        );
    }

    #[test]
    fn nothing_is_met_above_the_horizon_or_off_the_map() {
        let water = heights(64, |_, _| 0);
        let edge = map_camera(64, [64.0, 0.0], 50.0);
        assert_eq!(
            ground_at(&edge, [320.0, 470.0], VIEWPORT, &field(64, &water)),
            None
        );
        let low = map_camera(64, [64.0, 32.0], 20.0);
        assert!(low.pick_ray([320.0, 10.0], VIEWPORT)[1] > 0.0);
        assert_eq!(
            ground_at(&low, [320.0, 10.0], VIEWPORT, &field(64, &water)),
            None
        );
        let level = free_camera(64, [64.5, 10.0, 60.5], 0.0, 0.0, Lens::MAP);
        assert_eq!(
            ground_at(&level, [320.0, 240.0], VIEWPORT, &field(64, &water)),
            None
        );
    }

    #[test]
    fn a_click_on_the_ground_starts_there_on_the_first_map_copy() {
        assert_eq!(
            choose(false, Some([5700.0, 12.0, 900.0]), false, None, 5632.0),
            Choice::Start([68.0, 900.0])
        );
        assert_eq!(
            choose(false, Some([-32.0, 12.0, 900.0]), false, None, 5632.0),
            Choice::Start([5600.0, 900.0])
        );
    }

    #[test]
    fn the_south_button_starts_over_the_middle_of_the_view_and_a_click_comes_first() {
        let centre = Some([3000.0, 10.0, 1500.0]);
        assert_eq!(
            choose(false, None, true, centre, 5632.0),
            Choice::Start([3000.0, 1500.0])
        );
        assert_eq!(choose(false, None, false, centre, 5632.0), Choice::Wait);
        assert_eq!(
            choose(false, Some([10.0, 10.0, 20.0]), true, centre, 5632.0),
            Choice::Start([10.0, 20.0])
        );
    }

    #[test]
    fn escape_goes_back_to_the_map_and_missing_ground_waits() {
        assert_eq!(
            choose(true, Some([10.0, 10.0, 20.0]), true, None, 5632.0),
            Choice::Cancel
        );
        assert_eq!(choose(false, None, true, None, 5632.0), Choice::Wait);
    }
}
