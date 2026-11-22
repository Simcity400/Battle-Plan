
use std::collections::BTreeMap;

use super::camera::{Camera, Slack, frustum::{self, OBJECT_TOP}};
use crate::province_map::ProvinceMap;

const BOX_HEIGHT: f32 = OBJECT_TOP;

const SIDE_MARGIN: f32 = 10.0;

pub const OPEN: [f32; 4] = [0.0, 0.0, 0.0, f32::MIN];

pub type Planes = [[f32; 4]; 6];

#[derive(Clone, Debug)]
pub struct Admission {
    views: Vec<Planes>,
    eyes: Vec<[f32; 3]>,
    turn: f32,
}

impl Admission {
    pub fn new(camera: Camera, viewport: [u32; 2]) -> Self {
        Self::held(camera, viewport, Slack::NONE)
    }

    pub fn padded(camera: Camera, viewport: [u32; 2], pad: f32) -> Self {
        Self::held(camera, viewport, Slack { pad, turn: 0.0 })
    }

    pub fn held(camera: Camera, viewport: [u32; 2], slack: Slack) -> Self {
        let views = views_out(camera, viewport, slack);
        Self {
            eyes: views.iter().map(Camera::eye).collect(),
            views: views
                .into_iter()
                .map(|view| planes_out(view, viewport, SIDE_MARGIN + slack.pad))
                .collect(),
            turn: slack.turn,
        }
    }

    pub fn admits(&self, bounds: [f32; 4]) -> bool {
        self.admits_box([
            [bounds[0], 0.0, bounds[1]],
            [bounds[2], BOX_HEIGHT, bounds[3]],
        ])
    }

    pub fn admits_box(&self, bounds: [[f32; 3]; 2]) -> bool {
        self.views.iter().zip(&self.eyes).any(|(planes, eye)| {
            let reach = if self.turn > 0.0 { self.turn * furthest(bounds, *eye) } else { 0.0 };
            !planes.iter().any(|&plane| outside(plane, bounds, 0.0, reach))
        })
    }

    pub fn admits_province(&self, boxes: &BTreeMap<u32, [f32; 4]>, province: u32) -> bool {
        boxes
            .get(&province)
            .is_some_and(|bounds| self.admits(*bounds))
    }
}

pub fn views(camera: Camera, viewport: [u32; 2]) -> Vec<Camera> {
    views_out(camera, viewport, Slack::NONE)
}

fn views_out(mut camera: Camera, viewport: [u32; 2], slack: Slack) -> Vec<Camera> {
    if camera.free().is_some() {
        return free_views(camera, viewport, slack);
    }
    let pad = slack.pad;
    let width = camera.map.x();
    let corners = footprint(camera, viewport);
    let [west_top, east_top, west_bottom, east_bottom] = corners;
    let [east, west] = reaches([west_top - pad, east_top + pad, west_bottom - pad, east_bottom + pad], width);
    camera.center[0] += crossed(corners, width);
    let mut views = vec![camera];
    for (shift, reached) in [(-width, east), (width, west)] {
        if reached {
            let mut view = camera;
            view.center[0] += shift;
            views.push(view);
        }
    }
    views
}

pub fn seam_shift(camera: Camera, viewport: [u32; 2]) -> f32 {
    if camera.free().is_some() {
        return 0.0;
    }
    crossed(footprint(camera, viewport), camera.map.x())
}

fn crossed([west_top, east_top, west_bottom, east_bottom]: [f32; 4], width: f32) -> f32 {
    let mut shift = 0.0;
    if east_top < 0.0 && east_bottom < 0.0 {
        shift += width;
    }
    if width < west_top && width < west_bottom {
        shift -= width;
    }
    shift
}

fn free_views(camera: Camera, viewport: [u32; 2], slack: Slack) -> Vec<Camera> {
    let mut views = vec![camera];
    let width = camera.map.x();
    if width.is_nan() || width <= 0.0 {
        return views;
    }
    let shift = (camera.center[0] / width).floor() * width;
    views[0].center[0] -= shift;
    let main = views[0];
    let range = main.object_range();
    let reach = if slack.turn > 0.0 {
        frustum::turned_corners(&main, viewport, main.lens().near, range, slack.turn)
            .and_then(|corners| frustum::bounds(&frustum::in_band(&corners, 0.0, BOX_HEIGHT)))
    } else {
        main.reach(viewport, range, [0.0, BOX_HEIGHT])
    };
    let Some([x0, _, x1, _]) = reach else {
        return views;
    };
    let [x0, x1] = [x0 - SIDE_MARGIN - slack.pad, x1 + SIDE_MARGIN + slack.pad];
    for copies in seam_copies(x0, x1, width) {
        let mut view = main;
        view.center[0] -= copies as f32 * width;
        views.push(view);
    }
    views
}

pub fn seam_copies(x0: f32, x1: f32, width: f32) -> Vec<i32> {
    if !(width > 0.0 && x0.is_finite() && x1.is_finite()) {
        return Vec::new();
    }
    let first = (x0 / width).floor() as i32;
    let last = ((x1 / width).ceil() as i32 - 1).max(first);
    let reach = first.unsigned_abs().max(last.unsigned_abs()) as i32;
    (1..=reach)
        .flat_map(|distance| [distance, -distance])
        .filter(|copy| (first..=last).contains(copy))
        .collect()
}

pub(super) fn footprint(camera: Camera, viewport: [u32; 2]) -> [f32; 4] {
    let [top_left, top_right, bottom_left, bottom_right, _] = water_footprint(camera, viewport);
    [top_left[0], top_right[0], bottom_left[0], bottom_right[0]]
}

pub fn water_footprint(camera: Camera, viewport: [u32; 2]) -> [[f32; 2]; 5] {
    let [width, height] = viewport.map(|size| size.max(1) as f32);
    let eye = camera.eye();
    let point = |screen| {
        let [a, b, c] = camera.ray(screen, viewport);
        let length = (a * a + b * b + c * c).sqrt();
        let [a, b, c] = if length == 0.0 {
            [0.0; 3]
        } else {
            [a / length, b / length, c / length]
        };
        let cap = eye[1] * 5.0;
        let distance = if b < -0.001 {
            let water = (eye[1] - super::camera::WATER_HEIGHT) / b.abs();
            if water >= 0.0 {
                if water <= cap { water } else { cap }
            } else {
                0.0
            }
        } else {
            cap
        };
        [a * distance + eye[0], c * distance + eye[2]]
    };
    [
        point([0.0, 0.0]),
        point([width, 0.0]),
        point([0.0, height]),
        point([width, height]),
        point([width * 0.5, height * 0.5]),
    ]
}

pub(super) fn reaches(
    [west_top, east_top, west_bottom, east_bottom]: [f32; 4],
    width: f32,
) -> [bool; 2] {
    let [mut east, mut west] = [false; 2];
    if west_top < 0.0 || west_bottom < 0.0 {
        west = true;
    } else if width < west_top || width < west_bottom {
        east = true;
    }
    if width < east_top || width < east_bottom {
        east = true;
    } else if east_top < 0.0 || east_bottom < 0.0 {
        west = true;
    }
    [east, west]
}

pub fn side_planes(camera: Camera, viewport: [u32; 2]) -> Planes {
    planes_out(camera, viewport, SIDE_MARGIN)
}

fn planes_out(camera: Camera, viewport: [u32; 2], margin: f32) -> Planes {
    let m = camera.view_projection(viewport);
    let plane =
        |column: usize, sign: f32| std::array::from_fn(|row| m[row][3] + sign * m[row][column]);
    let [left, right, top, bottom] = [plane(0, 1.0), plane(0, -1.0), plane(1, -1.0), plane(1, 1.0)]
        .map(|plane| {
            let [a, b, c, d] = normalize(plane);
            [-a, -b, -c, -d - margin]
        });
    let [near, far] = match camera.free() {
        None => [OPEN; 2],
        Some(free) => {
            let [fx, fy, fz] = camera.look_direction();
            let eye = camera.eye();
            let depth = fx * eye[0] + fy * eye[1] + fz * eye[2];
            let range = camera.object_range();
            [
                [-fx, -fy, -fz, depth + free.lens.near - margin],
                [fx, fy, fz, -depth - range - margin],
            ]
        }
    };
    [left, right, top, bottom, near, far]
}

fn normalize([a, b, c, d]: [f32; 4]) -> [f32; 4] {
    let length = (a * a + b * b + c * c).sqrt();
    if length == 0.0 {
        return [a, b, c, d];
    }
    let scale = 1.0 / length;
    [a * scale, b * scale, c * scale, scale * d]
}

pub fn admits_box(bounds: [[f32; 3]; 2], offset: f32, planes: &Planes) -> bool {
    !planes.iter().any(|&plane| outside(plane, bounds, offset, 0.0))
}

fn outside([a, b, c, d]: [f32; 4], bounds: [[f32; 3]; 2], offset: f32, reach: f32) -> bool {
    let x = if a > 0.0 { bounds[0][0] } else { bounds[1][0] } + offset;
    let y = if b > 0.0 { bounds[0][1] } else { bounds[1][1] };
    let z = if c > 0.0 { bounds[0][2] } else { bounds[1][2] };
    x * a + b * y + c * z + d > reach
}

fn furthest(bounds: [[f32; 3]; 2], eye: [f32; 3]) -> f32 {
    let span = |axis: usize| (bounds[0][axis] - eye[axis]).abs().max((bounds[1][axis] - eye[axis]).abs());
    let [x, y, z] = [span(0), span(1), span(2)];
    (x * x + y * y + z * z).sqrt()
}

pub fn province_boxes(map: &ProvinceMap<'_>) -> BTreeMap<u32, [f32; 4]> {
    let mut boxes: BTreeMap<u32, [i32; 4]> = BTreeMap::new();
    if map.width == 0 {
        return BTreeMap::new();
    }
    for (row, ids) in map.ids.chunks(map.width as usize).enumerate() {
        let z = map.height as i32 - 1 - row as i32;
        let mut start = 0;
        while start < ids.len() {
            let id = ids[start];
            let end = start + ids[start..].iter().take_while(|next| **next == id).count();
            if id != 0 {
                let entry = boxes
                    .entry(id)
                    .or_insert([start as i32, z, end as i32 - 1, z]);
                entry[0] = entry[0].min(start as i32);
                entry[1] = entry[1].min(z);
                entry[2] = entry[2].max(end as i32 - 1);
                entry[3] = entry[3].max(z);
            }
            start = end;
        }
    }
    boxes
        .into_iter()
        .map(|(id, b)| {
            let even = |value: i32| (value & !1) as f32;
            (id, [even(b[0]), even(b[1]), even(b[2] + 2), even(b[3] + 2)])
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub fn edge_boxes() -> BTreeMap<u32, [f32; 4]> {
    let (width, height) = (3048, 1002);
    let mut ids = vec![0; (width * height) as usize];
    for (id, x, z) in [
        (1, 2999, 999),
        (2, 3041, 1000),
        (3, 2999, 931),
        (4, 3047, 1000),
        (5, 3045, 1000),
        (6, 2999, 957),
    ] {
        ids[((height - 1 - z) * width + x) as usize] = id;
    }
    province_boxes(&ProvinceMap {
        width,
        height,
        ids: &ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Free, Lens, MapDefines, MapSize, Placement, Pose};

    const VIEWPORT: [u32; 2] = [1024, 768];

    fn camera(center: [f32; 2], height: f32) -> Camera {
        Placement {
            center,
            height,
            pose: Pose::Map,
        }
        .on(MapSize::TEST, MapDefines::test())
    }

    #[test]
    fn the_seam_move_keeps_the_camera_on_the_maps_copies() {
        let base = camera([0., 1000.], 300.);
        let width = base.map.x();
        for start in [2.5 * width, -3.25 * width, 0.5 * width, width] {
            let mut moved = camera([start, 1000.], 300.);
            for _ in 0..16 {
                moved.center[0] += seam_shift(moved, VIEWPORT);
            }
            let [west_top, east_top, west_bottom, east_bottom] = footprint(moved, VIEWPORT);
            assert!(!(east_top < 0. && east_bottom < 0.), "{start}: wholly west");
            assert!(
                !(width < west_top && width < west_bottom),
                "{start}: wholly east"
            );
            assert_eq!(
                (moved.center[0] - start).rem_euclid(width),
                0.,
                "{start}: moves are whole map widths"
            );
            assert_eq!(seam_shift(moved, VIEWPORT), 0., "{start}: settled");
        }
        let straddling = camera([0., 1000.], 300.);
        assert_eq!(seam_shift(straddling, VIEWPORT), 0.);
        let far = camera([2.5 * width, 1000.], 300.);
        assert_eq!(seam_shift(far, VIEWPORT), -width);
    }

    #[test]
    fn entity_carriers_keep_native_order_and_original_footprint_flags() {
        let mut wide = camera([50., 1000.], 1500.);
        wide.map.width = 100;
        let carriers = views(wide, VIEWPORT);
        assert_eq!(
            carriers
                .iter()
                .map(|view| view.center[0])
                .collect::<Vec<_>>(),
            [50., -50., 150.]
        );
        let crossed = camera([-4000., 1000.], 50.);
        assert!(footprint(crossed, VIEWPORT).iter().all(|x| *x < 0.));
        let carriers = views(crossed, VIEWPORT);
        assert_eq!(
            carriers
                .iter()
                .map(|view| view.center[0])
                .collect::<Vec<_>>(),
            [
                crossed.center[0] + crossed.map.x(),
                crossed.center[0] + 2. * crossed.map.x()
            ]
        );
    }

    fn on_screen(camera: Camera, point: [f32; 3]) -> bool {
        camera.project(point, VIEWPORT).is_some_and(|[x, y]| {
            (0.0..=VIEWPORT[0] as f32).contains(&x) && (0.0..=VIEWPORT[1] as f32).contains(&y)
        })
    }

    #[test]
    fn the_side_planes_stand_ten_units_out() {
        let camera = camera([3000.0, 1000.0], 50.0);
        let admission = Admission::new(camera, VIEWPORT);
        let near = [3040.0, 999.0, 3042.0, 1001.0];
        assert!(!on_screen(camera, [near[0], 0.0, near[3]]));
        assert!(!on_screen(camera, [near[0], BOX_HEIGHT, near[3]]));
        assert!(admission.admits(near));
        assert!(!admission.admits([3046.0, 999.0, 3048.0, 1001.0]));
    }

    #[test]
    fn a_box_stands_seventy_high() {
        let camera = camera([3000.0, 1000.0], 50.0);
        let admission = Admission::new(camera, VIEWPORT);
        let [_, _, _, [_, down, south, d], _, _] = admission.views[0];
        assert!(down < 0.0);
        let top_shows = [2998.0, 956.0, 3000.0, 958.0];
        assert!(outside(
            [0.0, 0.0, south, d],
            [
                [top_shows[0], 0.0, top_shows[1]],
                [top_shows[2], BOX_HEIGHT, top_shows[3]]
            ],
            0.0,
            0.0,
        ));
        assert!(admission.admits(top_shows));
        assert!(!admission.admits([2998.0, 930.0, 3000.0, 932.0]));
    }

    #[test]
    fn a_view_past_a_seam_admits_the_other_edge() {
        let width = MapSize::TEST.x();
        let cell = |x: f32| [x, 999.0, x + 2.0, 1001.0];
        let east = Admission::new(camera([width - 5.0, 1000.0], 300.0), VIEWPORT);
        assert_eq!(east.views.len(), 2);
        assert!(east.admits(cell(0.0)) && east.admits(cell(width - 2.0)));
        assert!(!east.admits(cell(width / 2.0)));
        let west = Admission::new(camera([5.0, 1000.0], 300.0), VIEWPORT);
        assert_eq!(west.views.len(), 2);
        assert!(west.admits(cell(width - 2.0)) && west.admits(cell(0.0)));
        let far = Admission::new(camera([-width - 5.0, 1000.0], 300.0), VIEWPORT);
        assert!(far.admits(cell(width - 2.0)) && far.admits(cell(0.0)));
        assert_eq!(
            reaches([-1.0, width + 1.0, -1.0, width + 1.0], width),
            [true; 2]
        );
    }

    #[test]
    fn province_boxes_round_outward_to_even_edges() {
        let boxes = province_boxes(&ProvinceMap {
            width: 3,
            height: 2,
            ids: &[1, 1, 2, 2, 1, 3],
        });
        assert_eq!(boxes[&1], [0.0, 0.0, 2.0, 2.0]);
        assert_eq!(boxes[&2], [0.0, 0.0, 4.0, 2.0]);
        assert_eq!(boxes[&3], [2.0, 0.0, 4.0, 2.0]);
        let admission = Admission::new(camera([1.0, 1.0], 300.0), VIEWPORT);
        assert!(admission.admits_province(&boxes, 3));
        assert!(!admission.admits_province(&boxes, 4));
    }

    #[test]
    fn contact_survives_and_the_nearest_inside_corner_is_tested() {
        let bounds = [[2.0, 0.0, 4.0], [6.0, BOX_HEIGHT, 8.0]];
        for plane in [
            [1.0, 0.0, 0.0, -2.0],
            [-1.0, 0.0, 0.0, 6.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0, 70.0],
            [0.0, 0.0, 1.0, -4.0],
            [0.0, 0.0, -1.0, 8.0],
        ] {
            assert!(!outside(plane, bounds, 0.0, 0.0), "contact: {plane:?}");
            let mut past = plane;
            past[3] += 0.001;
            assert!(outside(past, bounds, 0.0, 0.0), "past contact: {past:?}");
        }
        assert_eq!(normalize([0.0; 4]), [0.0; 4]);
        assert_eq!(normalize([3.0, 4.0, 0.0, 10.0]), [0.6, 0.8, 0.0, 2.0]);
    }

    #[test]
    fn recentering_moves_once_and_retains_the_original_seam_flags() {
        let width = MapSize::TEST.x();
        let far = camera([-2.0 * width - 5.0, 1000.0], 300.0);
        let admission = Admission::new(far, VIEWPORT);
        assert_eq!(admission.views.len(), 2);
        assert_eq!(
            admission.views[0],
            side_planes(camera([-width - 5.0, 1000.0], 300.0), VIEWPORT)
        );
        assert_eq!(
            admission.views[1],
            side_planes(camera([-5.0, 1000.0], 300.0), VIEWPORT)
        );
        let crossed = Admission::new(camera([-300.0, 1000.0], 300.0), VIEWPORT);
        assert_eq!(crossed.views.len(), 2);
    }

    #[test]
    fn footprint_caps_rays_even_when_the_water_is_beyond_the_horizon() {
        let mut camera = camera([3000.0, 1000.0], 50.0);
        camera.defines.tilt_lead = 1000.0;
        assert!(camera.unproject_ground([0.0, 0.0], VIEWPORT).is_none());
        let corners = footprint(camera, VIEWPORT);
        assert!(corners.iter().all(|x| x.is_finite()));
        assert!(
            corners
                .iter()
                .all(|x| (x - camera.eye()[0]).abs() <= 5.0 * camera.height)
        );
        assert_eq!(
            reaches([0.0, camera.map.x(), 0.0, camera.map.x()], camera.map.x()),
            [false; 2]
        );
    }

    fn free_camera(eye: [f32; 3], yaw: f32) -> Camera {
        let lens = Lens {
            far: 20_000.0,
            near: 1.0,
            ..Lens::MAP
        };
        Placement {
            center: [eye[0], eye[2]],
            height: eye[1],
            pose: Pose::Free(Free::new(yaw, -0.05, lens)),
        }
        .on(MapSize::TEST, MapDefines::test())
    }

    #[test]
    fn a_free_eye_admits_within_its_object_range_and_not_behind_it() {
        let eye = free_camera([3000.0, 30.0, 1000.0], 0.0);
        let admission = Admission::new(eye, VIEWPORT);
        let cell = |z: f32| [2999.0, z, 3001.0, z + 2.0];
        assert!(admission.admits(cell(1500.0)));
        let range = MapDefines::test().map_objects_cutoff;
        assert!(admission.admits(cell(1000.0 + range - 20.0)));
        assert!(!admission.admits(cell(1000.0 + range + 50.0)));
        assert!(!admission.admits(cell(900.0)));
        let map = Admission::new(camera([3000.0, 1000.0], 300.0), VIEWPORT);
        assert_eq!(map.views[0][4], OPEN);
        assert_eq!(map.views[0][5], OPEN);
    }

    #[test]
    fn a_free_eye_beside_the_seam_admits_both_edges() {
        let width = MapSize::TEST.x();
        let along_east = free_camera([width - 50.0, 30.0, 1000.0], std::f32::consts::FRAC_PI_2);
        let seen = views(along_east, VIEWPORT);
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].center[0], width - 50.0);
        assert_eq!(seen[1].center[0], width - 50.0 - width);
        let admission = Admission::new(along_east, VIEWPORT);
        assert!(admission.admits([1000.0, 999.0, 1002.0, 1001.0]));
        let wandered = free_camera([3.0 * width + 10.0, 30.0, 1000.0], 0.0);
        assert_eq!(views(wandered, VIEWPORT)[0].center[0], 10.0);
        assert_eq!(seam_copies(-10.0, width + 10.0, width), [1, -1]);
        assert_eq!(seam_copies(10.0, 2.5 * width, width), [1, 2]);
        assert!(seam_copies(10.0, width - 10.0, width).is_empty());
    }

    #[test]
    fn a_camera_inside_the_slack_admits_nothing_the_held_admission_left_out() {
        use crate::camera::{PAD as pad, TURN};
        let sample = |held: Admission, seen: Admission, what: &str, xs: std::ops::Range<i32>, zs: std::ops::Range<i32>| {
            let mut tested = 0;
            for x in xs.step_by(20) {
                for z in zs.clone().step_by(20) {
                    let box_ = [x as f32, z as f32, x as f32 + 20., z as f32 + 20.];
                    if seen.admits(box_) {
                        tested += 1;
                        assert!(held.admits(box_), "{box_:?}: {what}");
                    }
                }
            }
            assert!(tested > 0, "the sample reaches the view: {what}");
        };
        for height in [60., 110., 400.] {
            let held = camera([3200., 1500.], height);
            let padded = Admission::padded(held, VIEWPORT, pad);
            for travel in [[pad, 0.], [-pad, 0.], [0., pad], [0., -pad], [pad * 0.7, pad * 0.7]] {
                let moved = camera([held.center[0] + travel[0], held.center[1] + travel[1]], height);
                let what = format!("at {height} after {travel:?}");
                sample(padded.clone(), Admission::new(moved, VIEWPORT), &what, 2600..3800, 900..2100);
            }
        }
        let slack = Slack { pad, turn: TURN };
        let a = 0.5 * TURN;
        let width = MapSize::TEST.x();
        for (eye, yaw, pitch) in [([3200., 60., 1500.], 1.2, -0.2), ([width - 300., 100., 1500.], 1.5, -0.1), ([2000., 30., 800.], 4.0, 0.2)] {
            let held = free_camera(eye, yaw);
            let held = Camera { pose: Pose::Free(Free::new(yaw, pitch, held.lens())), ..held };
            let admission = Admission::held(held, VIEWPORT, slack);
            for (delta, travel) in [([a, 0., 0.], [0., 0.]), ([-a, 0., 0.], [pad * 0.7, -pad * 0.7]), ([0., a, 0.], [0., pad]), ([0., 0., 1.9 * a], [0., 0.]), ([0., -a, 0.], [0., -pad]), ([0.6 * a, 0.6 * a, -0.6 * a], [-pad, 0.])] {
                let Pose::Free(pose) = held.pose else { unreachable!() };
                let turned = Free { yaw: pose.yaw + delta[0], pitch: pose.pitch + delta[1], roll: delta[2], ..pose };
                let moved = Camera { center: [held.center[0] + travel[0], held.center[1] + travel[1]], pose: Pose::Free(turned), ..held };
                let (_, turn) = held.turned_from(&moved).unwrap();
                assert!(turn <= TURN, "{delta:?} turns {turn}");
                let what = format!("{eye:?} turned {delta:?} after {travel:?}");
                sample(admission.clone(), Admission::new(moved, VIEWPORT), &what, -2000..(width as i32 + 2000), 0..3000);
            }
        }
    }
}
