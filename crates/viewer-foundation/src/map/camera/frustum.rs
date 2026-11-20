//! A free camera's view volume and the ground it reaches.
//!
//! The game's camera always looks down at the water plane, so its four corner rays bound what it
//! sees. A free camera can look at the horizon or the sky, where a corner ray never meets the
//! ground. Its view volume is cut at a depth instead, then clipped to the band of heights the map
//! occupies; the clipped volume's corners bound the ground it reaches.

use super::Camera;

/// The height of the highest terrain surface: 255 height-map steps of a tenth.
pub const TERRAIN_TOP: f32 = 25.5;

/// The height the map's objects reach: a province box stands this tall
/// ([`province_admission`](crate::map::province_admission)).
pub const OBJECT_TOP: f32 = 70.0;

/// The eight corners of the view volume between view depths `near` and `far`: near top-left,
/// top-right, bottom-left, bottom-right, then the far ones in the same order.
pub fn corners(camera: &Camera, viewport: [u32; 2], near: f32, far: f32) -> [[f32; 3]; 8] {
    let [right, up, forward] = camera.axes();
    let lens = camera.lens();
    let tangent_y = (lens.fov_y * 0.5).tan();
    let tangent_x = tangent_y * viewport[0].max(1) as f32 / viewport[1].max(1) as f32;
    let eye = camera.eye();
    let mut out = [[0.0; 3]; 8];
    for (slot, (depth, [sx, sy])) in out
        .iter_mut()
        .zip([near, far].into_iter().flat_map(|depth| {
            [[-1.0, 1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]].map(|corner| (depth, corner))
        }))
    {
        *slot = std::array::from_fn(|axis| {
            eye[axis]
                + forward[axis] * depth
                + right[axis] * sx * tangent_x * depth
                + up[axis] * sy * tangent_y * depth
        });
    }
    out
}

/// [`corners`] of a volume holding the view volume of every camera at this eye turned by up to
/// `turn` ([`super::Slack`]); `None` when the turn reaches a corner ray's own angle, which no
/// volume of this shape holds.
///
/// A turned view's direction `v` stands within `turn` of one of this view's, so each side plane's
/// unit normal `n` and the forward axis `f` give `n.v >= -turn` and `f.v >= c - turn`, where `c` is
/// the corner rays' cosine. A side leaned out to `n + l f` with `l = turn / (c - turn)` holds it,
/// which widens each tangent by `l` times its secant. The turned view's depths run from `near` to
/// `far` along its own axis, `(c - turn) near` to `(1 + turn s) far` along this one's, with `s`
/// the corners' secant.
pub fn turned_corners(camera: &Camera, viewport: [u32; 2], near: f32, far: f32, turn: f32) -> Option<[[f32; 3]; 8]> {
    let [right, up, forward] = camera.axes();
    let tangent_y = (camera.lens().fov_y * 0.5).tan();
    let tangent_x = tangent_y * viewport[0].max(1) as f32 / viewport[1].max(1) as f32;
    let secant = (1.0 + tangent_x * tangent_x + tangent_y * tangent_y).sqrt();
    let cosine = 1.0 / secant;
    if !(turn < cosine * 0.5) {
        return None;
    }
    let lean = turn / (cosine - turn);
    let wide = |tangent: f32| tangent + lean * (1.0 + tangent * tangent).sqrt();
    let [tangent_x, tangent_y] = [wide(tangent_x), wide(tangent_y)];
    let [near, far] = [near * (cosine - turn), far.max(near) * (1.0 + turn * secant)];
    let eye = camera.eye();
    let mut out = [[0.0; 3]; 8];
    for (slot, (depth, [sx, sy])) in out.iter_mut().zip([near, far].into_iter().flat_map(|depth| {
        [[-1.0, 1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]].map(|corner| (depth, corner))
    })) {
        *slot = std::array::from_fn(|axis| {
            eye[axis] + forward[axis] * depth + right[axis] * sx * tangent_x * depth + up[axis] * sy * tangent_y * depth
        });
    }
    Some(out)
}

/// The view volume's twelve edges, as corner indices of [`corners`].
const EDGES: [[usize; 2]; 12] = [
    [0, 1],
    [1, 3],
    [3, 2],
    [2, 0],
    [4, 5],
    [5, 7],
    [7, 6],
    [6, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];

/// The corners of the convex volume `corners` encloses once clipped to heights `low..=high`.
///
/// A clipped convex volume's corners are the original corners inside the band and the points
/// where its edges cross either bounding plane. Empty when the volume misses the band.
pub fn in_band(corners: &[[f32; 3]; 8], low: f32, high: f32) -> Vec<[f32; 3]> {
    let mut points: Vec<[f32; 3]> = corners
        .iter()
        .copied()
        .filter(|point| (low..=high).contains(&point[1]))
        .collect();
    for [a, b] in EDGES {
        let (a, b) = (corners[a], corners[b]);
        for plane in [low, high] {
            let (da, db) = (a[1] - plane, b[1] - plane);
            if (da < 0.0) != (db < 0.0) && da != db {
                let t = da / (da - db);
                points.push(std::array::from_fn(|axis| {
                    a[axis] + (b[axis] - a[axis]) * t
                }));
            }
        }
    }
    points.retain(|point| point.iter().all(|value| value.is_finite()));
    points
}

/// The `(x0, z0, x1, z1)` box around `points`, `None` when there are none.
pub fn bounds(points: &[[f32; 3]]) -> Option<[f32; 4]> {
    let first = points.first()?;
    let mut rect = [first[0], first[2], first[0], first[2]];
    for point in &points[1..] {
        rect[0] = rect[0].min(point[0]);
        rect[1] = rect[1].min(point[2]);
        rect[2] = rect[2].max(point[0]);
        rect[3] = rect[3].max(point[2]);
    }
    Some(rect)
}

/// The view volume's six planes in the projection's row-vector convention, each `[a, b, c, d]`
/// with `a x + b y + c z + d >= 0` inside: left, right, top, bottom, near, far.
pub fn planes(matrix: &[[f32; 4]; 4]) -> [[f32; 4]; 6] {
    let column = |index: usize| -> [f32; 4] { std::array::from_fn(|row| matrix[row][index]) };
    let [x, y, z, w] = [column(0), column(1), column(2), column(3)];
    let add = |a: [f32; 4], b: [f32; 4], sign: f32| std::array::from_fn(|i| a[i] + sign * b[i]);
    // Depth runs 0..1, so the near plane is z >= 0 and the far plane z <= w.
    [
        add(w, x, 1.0),
        add(w, x, -1.0),
        add(w, y, -1.0),
        add(w, y, 1.0),
        z,
        add(w, z, -1.0),
    ]
}
