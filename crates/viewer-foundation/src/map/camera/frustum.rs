
use super::Camera;

pub const TERRAIN_TOP: f32 = 25.5;

pub const OBJECT_TOP: f32 = 70.0;

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

pub fn planes(matrix: &[[f32; 4]; 4]) -> [[f32; 4]; 6] {
    let column = |index: usize| -> [f32; 4] { std::array::from_fn(|row| matrix[row][index]) };
    let [x, y, z, w] = [column(0), column(1), column(2), column(3)];
    let add = |a: [f32; 4], b: [f32; 4], sign: f32| std::array::from_fn(|i| a[i] + sign * b[i]);
    [
        add(w, x, 1.0),
        add(w, x, -1.0),
        add(w, y, -1.0),
        add(w, y, 1.0),
        z,
        add(w, z, -1.0),
    ]
}
