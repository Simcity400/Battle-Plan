//! Camera admission and the province boxes it tests.
//!
//! The update list includes admitted and presentation-dirty provinces. Scheduling, units, buildings,
//! counter sources and faction pings share admission.
//!
//! A province's box is its bitmap extent rounded outward to even coordinates ([`province_boxes`]),
//! standing from the ground to [`BOX_HEIGHT`]. It is admitted when, in any of the camera's seam
//! views ([`Admission::new`]), no side plane has the whole box outside it: the box's corner
//! nearest the inside of each plane is tested, and only a strictly
//! positive distance outside rejects.
//!
//! The function also keeps, for each admitted province, the admitting view nearest the eye, which
//! passes on to the province's updates. The viewer places nothing per seam view, so
//! that choice is not ported.

use std::collections::BTreeMap;

use super::{camera::Camera, map_buildings::ProvinceMap};

/// the box's height above the ground.
const BOX_HEIGHT: f32 = 70.0;

/// Each camera view carries SIDE_MARGIN, including seam copies. Move its four side planes outward by
/// that margin.
const SIDE_MARGIN: f32 = 10.0;

/// One view's four side planes, outside positive.
type Planes = [[f32; 4]; 4];

/// The views of one camera that admit provinces.
#[derive(Clone, Debug)]
pub struct Admission {
    /// Main view followed by reached seam views.
    views: Vec<Planes>,
}

impl Admission {
    /// Start with the main camera. A footprint beyond the east edge adds a view one map width west; a
    /// footprint beyond the west edge adds one east.
    ///
    /// Cap normalized corner rays at five times eye height, then force y to the water plane. Recenter
    /// a wholly seam-crossing footprint by one map width while retaining original seam flags.
    pub fn new(camera: Camera, viewport: [u32; 2]) -> Self {
        Self {
            views: views(camera, viewport)
                .into_iter()
                .map(|view| side_planes(view, viewport))
                .collect(),
        }
    }

    /// Whether a province box `[min x, min z, max x, max z]` is admitted.
    pub fn admits(&self, bounds: [f32; 4]) -> bool {
        self.admits_box([
            [bounds[0], 0.0, bounds[1]],
            [bounds[2], BOX_HEIGHT, bounds[3]],
        ])
    }

    /// A consumer's own `[min, max]` world box against the same camera views.
    pub(super) fn admits_box(&self, bounds: [[f32; 3]; 2]) -> bool {
        self.views
            .iter()
            .any(|planes| admits_box(bounds, 0.0, planes))
    }

    /// Whether `province`'s box is admitted; a province the bitmap never paints, province 0
    /// included, has none.
    pub fn admits_province(&self, boxes: &BTreeMap<u32, [f32; 4]>, province: u32) -> bool {
        boxes
            .get(&province)
            .is_some_and(|bounds| self.admits(*bounds))
    }
}

/// Preserve carrier order for entity envelopes. Geometry remains canonical: the east copy uses
/// eye minus map width, then the west copy uses eye plus map width.
pub(super) fn views(mut camera: Camera, viewport: [u32; 2]) -> Vec<Camera> {
    let width = camera.map.x();
    let corners = footprint(camera, viewport);
    let [east, west] = reaches(corners, width);
    let [west_top, east_top, west_bottom, east_bottom] = corners;
    if east_top < 0.0 && east_bottom < 0.0 {
        camera.center[0] += width;
    }
    if width < west_top && width < west_bottom {
        camera.center[0] -= width;
    }
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

/// Footprint x coordinates in top-left, top-right, bottom-left, bottom-right order, with normalized
/// rays and the fixed downward threshold/range cap.
pub(super) fn footprint(camera: Camera, viewport: [u32; 2]) -> [f32; 4] {
    let [width, height] = viewport.map(|size| size.max(1) as f32);
    let x = |screen| {
        let [a, b, c] = camera.ray(screen, viewport);
        //  divides each component by length (unlike the plane's reciprocal).
        let length = (a * a + b * b + c * c).sqrt();
        let [a, b] = if length == 0.0 {
            [0.0; 2]
        } else {
            [a / length, b / length]
        };
        let eye = camera.eye();
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
        a * distance + eye[0]
    };
    [
        x([0.0, 0.0]),
        x([width, 0.0]),
        x([0.0, height]),
        x([width, height]),
    ]
}

/// Determine east and west wrap reach from the footprint. Test west corners against the west edge
/// first, and east corners against the east edge first.
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

/// Left, right, top and bottom planes from view-projection: normalize, negate so outside is positive,
/// then expand by SIDE_MARGIN.
pub(super) fn side_planes(camera: Camera, viewport: [u32; 2]) -> Planes {
    let m = camera.view_projection(viewport);
    let plane =
        |column: usize, sign: f32| std::array::from_fn(|row| m[row][3] + sign * m[row][column]);
    [plane(0, 1.0), plane(0, -1.0), plane(1, -1.0), plane(1, 1.0)].map(|plane| {
        let [a, b, c, d] = normalize(plane);
        [-a, -b, -c, -d - SIDE_MARGIN]
    })
}

/// the plane scaled by the reciprocal of its normal's length, unless that is 0.
fn normalize([a, b, c, d]: [f32; 4]) -> [f32; 4] {
    let length = (a * a + b * b + c * c).sqrt();
    if length == 0.0 {
        return [a, b, c, d];
    }
    let scale = 1.0 / length;
    [a * scale, b * scale, c * scale, scale * d]
}

/// One view's box test, also used by trees, ambient buckets
/// arrows and names; contact survives.
pub(super) fn admits_box(bounds: [[f32; 3]; 2], offset: f32, planes: &Planes) -> bool {
    !planes.iter().any(|&plane| outside(plane, bounds, offset))
}

/// Test the box corner nearest the plane's interior, evaluating x, y, then z.
fn outside([a, b, c, d]: [f32; 4], bounds: [[f32; 3]; 2], offset: f32) -> bool {
    let x = if a > 0.0 { bounds[0][0] } else { bounds[1][0] } + offset;
    let y = if b > 0.0 { bounds[0][1] } else { bounds[1][1] };
    let z = if c > 0.0 { bounds[0][2] } else { bounds[1][2] };
    x * a + b * y + c * z + d > 0.0
}

/// Every painted province's box `[min x, min z, max x, max z]` in map units.
///
/// Grow bottom-up province bounds from bitmap runs and round outward to even coordinates after each
/// run. Rounding is idempotent. An unpainted province has no box.
pub fn province_boxes(map: &ProvinceMap<'_>) -> BTreeMap<u32, [f32; 4]> {
    let mut boxes: BTreeMap<u32, [i32; 4]> = BTreeMap::new();
    if map.width == 0 {
        return BTreeMap::new();
    }
    for (row, ids) in map.ids.chunks(map.width as usize).enumerate() {
        // Bitmap rows run north to south; world z runs south to north.
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

/// Painted edge cases shared by consumer tests: inside, margin, excessive height, beyond the
/// margin, and a single odd-coordinate pixel whose even-rounded box alone reaches the view.
#[cfg(test)]
pub(super) fn edge_boxes() -> BTreeMap<u32, [f32; 4]> {
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
    use crate::map::camera::{MapDefines, MapSize, Placement};

    const VIEWPORT: [u32; 2] = [1024, 768];

    fn camera(center: [f32; 2], height: f32) -> Camera {
        Placement { center, height }.on(MapSize::TEST, MapDefines::test())
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

    /// Whether a world point lies inside the view, by projection alone.
    fn on_screen(camera: Camera, point: [f32; 3]) -> bool {
        camera.project(point, VIEWPORT).is_some_and(|[x, y]| {
            (0.0..=VIEWPORT[0] as f32).contains(&x) && (0.0..=VIEWPORT[1] as f32).contains(&y)
        })
    }

    /// Past the view's right edge, a box is admitted while its corner nearest the view is within
    /// ten units of the plane.
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

    /// Low over the map the bottom plane leans back past the vertical, so it meets a box's top
    /// corner first. At height 50 over (3000, 1000) (unit normal y -0.243, z -0.970) a box on
    /// rows 956..958 is 23.8 units out at the ground but 6.8 at its top, 70 up: the margin
    /// admits it by its top alone. On rows 930..932 its top is 32 units out; a 256-high one
    /// would be 13 inside.
    #[test]
    fn a_box_stands_seventy_high() {
        let camera = camera([3000.0, 1000.0], 50.0);
        let admission = Admission::new(camera, VIEWPORT);
        let [_, _, _, [_, down, south, d]] = admission.views[0];
        assert!(down < 0.0);
        let top_shows = [2998.0, 956.0, 3000.0, 958.0];
        // Without its height the box's ground corner is outside.
        assert!(outside(
            [0.0, 0.0, south, d],
            [
                [top_shows[0], 0.0, top_shows[1]],
                [top_shows[2], BOX_HEIGHT, top_shows[3]]
            ],
            0.0,
        ));
        assert!(admission.admits(top_shows));
        assert!(!admission.admits([2998.0, 930.0, 3000.0, 932.0]));
    }

    /// A view past a seam admits the other edge of the map through a copy one map width away,
    /// and a camera panned a whole width and more away is moved back first.
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
        // A footprint wider than the map reaches both ways.
        assert_eq!(
            reaches([-1.0, width + 1.0, -1.0, width + 1.0], width),
            [true; 2]
        );
    }

    /// rounds each box's minima down and its exclusive maxima up to even
    /// coordinates; bitmap rows run north to south.
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
            assert!(!outside(plane, bounds, 0.0), "contact: {plane:?}");
            let mut past = plane;
            past[3] += 0.001;
            assert!(outside(past, bounds, 0.0), "past contact: {past:?}");
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
        // Recomputing flags after moving would also enable the west-shifted view.
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
}
