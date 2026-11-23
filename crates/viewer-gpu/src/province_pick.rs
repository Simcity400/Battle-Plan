
use content_reader_map::map::MapData;

use super::{camera::Camera, province_admission};

pub fn pick(
    camera: &Camera,
    screen: [f32; 2],
    viewport: [u32; 2],
    map: &MapData,
    heightmap: &[u8],
) -> Result<Option<u32>, String> {
    Ok(
        ground_point(camera, screen, viewport, map, heightmap)?
            .and_then(|point| raster(map, point)),
    )
}

pub fn ground_point(
    camera: &Camera,
    screen: [f32; 2],
    viewport: [u32; 2],
    map: &MapData,
    heightmap: &[u8],
) -> Result<Option<[f32; 3]>, String> {
    ground(camera, screen, viewport, map, heightmap, Views::Seams)
}

pub fn motion_ground_point(
    camera: &Camera,
    screen: [f32; 2],
    viewport: [u32; 2],
    map: &MapData,
    heightmap: &[u8],
) -> Result<Option<[f32; 3]>, String> {
    ground(camera, screen, viewport, map, heightmap, Views::Own)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Views {
    Seams,
    Own,
}

fn ground(
    camera: &Camera,
    screen: [f32; 2],
    viewport: [u32; 2],
    map: &MapData,
    heightmap: &[u8],
    mode: Views,
) -> Result<Option<[f32; 3]>, String> {
    let count = (map.width as usize)
        .checked_mul(map.height as usize)
        .ok_or("province pick: raster dimensions overflow")?;
    if count == 0 || map.province_ids.len() != count || heightmap.len() != count {
        return Err(format!(
            "province pick: raster size mismatch ({}x{}, province texels {}, height texels {})",
            map.width,
            map.height,
            map.province_ids.len(),
            heightmap.len()
        ));
    }
    if screen.iter().any(|value| !value.is_finite()) {
        return Ok(None);
    }
    if camera.free().is_some() {
        return Ok(free_ground(camera, screen, viewport, map, heightmap).map(
            |[x, y, z]| match mode {
                Views::Seams => [x.rem_euclid(camera.map.x()), y, z],
                Views::Own => [x, y, z],
            },
        ));
    }
    let views = match mode {
        Views::Seams => views(*camera, viewport),
        Views::Own => [Some(*camera), None, None],
    };
    let mut hits = [[0.0; 3]; 256];
    let mut count = 0;
    let mut best = None;
    let mut nearest = f32::MAX;
    for view in views.into_iter().flatten() {
        let direction = view.pick_ray(screen, viewport);
        let origin = view.eye();
        height_hits(
            origin,
            direction,
            [0.0, 0.0, map.width as f32, map.height as f32],
            map,
            heightmap,
            &mut hits,
            &mut count,
        );
        let mut view_nearest = f32::MAX;
        let mut view_best = None;
        for hit in &hits[..count] {
            let distance = distance(*hit, origin);
            if distance < view_nearest {
                view_nearest = distance;
                view_best = Some(*hit);
            }
        }
        if view_nearest < nearest {
            nearest = view_nearest;
            best = view_best;
        }
    }
    Ok(Some(best.unwrap_or_else(|| {
        plane_pick(camera, screen, viewport, mode)
    })))
}

pub fn free_ground(
    camera: &Camera,
    screen: [f32; 2],
    viewport: [u32; 2],
    map: &MapData,
    heightmap: &[u8],
) -> Option<[f32; 3]> {
    let count = (map.width as usize).checked_mul(map.height as usize)?;
    if count == 0 || heightmap.len() != count {
        return None;
    }
    let origin = camera.eye();
    let direction = camera.pick_ray(screen, viewport);
    let forward = camera.look_direction();
    let depth = direction[0] * forward[0] + direction[1] * forward[1] + direction[2] * forward[2];
    if depth.is_nan() || depth <= 0.0 {
        return None;
    }
    march(origin, direction, camera.lens().far / depth, map, heightmap)
}

fn march(
    origin: [f32; 3],
    direction: [f32; 3],
    far: f32,
    map: &MapData,
    heightmap: &[u8],
) -> Option<[f32; 3]> {
    const TOP: f32 = super::camera::frustum::TERRAIN_TOP;
    let [ox, oy, oz] = origin;
    let [dx, dy, dz] = direction;
    let mut start = 0.0f32;
    let mut end = far;
    if oy > TOP {
        if dy >= 0.0 {
            return None;
        }
        start = start.max((oy - TOP) / -dy);
    } else if dy > 0.0 {
        end = end.min((TOP - oy) / dy);
    }
    if start.is_nan() || end.is_nan() || start >= end {
        return None;
    }
    let width = map.width as i64;
    let rows = map.height as i64;
    let at = |t: f32| [ox + dx * t, oy + dy * t, oz + dz * t];
    let [x, _, z] = at(start);
    let mut cell = [x.floor() as i64, z.floor() as i64];
    let step = [dx.signum() as i64, dz.signum() as i64];
    let delta = [1.0 / dx.abs(), 1.0 / dz.abs()];
    let next = |position: f32, cell: i64, direction: f32, delta: f32| {
        if direction > 0.0 {
            (cell as f32 + 1.0 - position) * delta
        } else if direction < 0.0 {
            (position - cell as f32) * delta
        } else {
            f32::INFINITY
        }
    };
    let mut boundary = [
        start + next(x, cell[0], dx, delta[0]),
        start + next(z, cell[1], dz, delta[1]),
    ];
    let mut entered = start;
    let limit = ((end - start) * 2.0) as u64 + 4;
    for _ in 0..limit {
        let exit = boundary[0].min(boundary[1]).min(end);
        if (0..rows).contains(&cell[1]) {
            let column = cell[0].rem_euclid(width) as usize;
            let row = (rows - 1 - cell[1]) as usize;
            let height = (f32::from(heightmap[row * map.width as usize + column]) * 0.1).max(9.5);
            let [_, low_entry, _] = at(entered);
            let [_, low_exit, _] = at(exit);
            if low_entry.min(low_exit) <= height {
                let t = if low_entry <= height {
                    entered
                } else {
                    (oy - height) / -dy
                };
                let [px, _, pz] = at(t);
                return Some([px, height, pz]);
            }
        } else if (cell[1] < 0 && dz <= 0.0) || (cell[1] >= rows && dz >= 0.0) {
            return None;
        }
        if exit >= end {
            return None;
        }
        entered = exit;
        if boundary[0] <= boundary[1] {
            cell[0] += step[0];
            boundary[0] += delta[0];
        } else {
            cell[1] += step[1];
            boundary[1] += delta[1];
        }
    }
    None
}

fn views(mut camera: Camera, viewport: [u32; 2]) -> [Option<Camera>; 3] {
    let width = camera.map.x();
    let corners = province_admission::footprint(camera, viewport);
    let [east, west] = province_admission::reaches(corners, width);
    let [west_top, east_top, west_bottom, east_bottom] = corners;
    if east_top < 0.0 && east_bottom < 0.0 {
        camera.center[0] += width;
    }
    if width < west_top && width < west_bottom {
        camera.center[0] -= width;
    }
    let shifted = |shift: f32| {
        let mut view = camera;
        view.center[0] += shift;
        view
    };
    [
        Some(camera),
        east.then(|| shifted(-width)),
        west.then(|| shifted(width)),
    ]
}

fn plane_pick(camera: &Camera, screen: [f32; 2], viewport: [u32; 2], mode: Views) -> [f32; 3] {
    let views = match mode {
        Views::Seams => views(*camera, viewport),
        Views::Own => [Some(*camera), None, None],
    };
    for view in views.into_iter().flatten() {
        let direction = view.pick_ray(screen, viewport);
        let origin = view.eye();
        let distance = if direction[1].abs() >= 0.0001 {
            (origin[1] / direction[1]).abs()
        } else {
            4000.0
        };
        let x = direction[0] * distance + origin[0];
        if mode == Views::Own || (x >= 0.0 && x <= camera.map.x()) {
            return [x, 0.0, direction[2] * distance + origin[2]];
        }
    }
    [0.0; 3]
}

fn raster(map: &MapData, [x, _, z]: [f32; 3]) -> Option<u32> {
    if !x.is_finite() || !z.is_finite() {
        return None;
    }
    let (x, z) = (x as i32, z as i32);
    if x < 0 || z < 0 || x >= map.width as i32 || z >= map.height as i32 {
        return None;
    }
    let id =
        map.province_ids[(map.height as usize - 1 - z as usize) * map.width as usize + x as usize];
    (id != 0).then_some(id)
}

fn height_hits(
    origin: [f32; 3],
    direction: [f32; 3],
    [x, z, width, depth]: [f32; 4],
    map: &MapData,
    heightmap: &[u8],
    hits: &mut [[f32; 3]; 256],
    count: &mut usize,
) {
    if width >= 2.0 || depth >= 2.0 {
        let width = if width >= 2.0 { width * 0.5 } else { width };
        let depth = if depth >= 2.0 { depth * 0.5 } else { depth };
        for [x, z] in [
            [x, z],
            [x + width, z],
            [x, z + depth],
            [x + width, z + depth],
        ] {
            if intersects(origin, direction, [x, 0.0, z], [x + width, 25.5, z + depth]) {
                height_hits(
                    origin,
                    direction,
                    [x, z, width, depth],
                    map,
                    heightmap,
                    hits,
                    count,
                );
            }
        }
    } else if *count < hits.len() {
        let height = terrain_height(map, heightmap, [x, z]);
        let end = [x + width, height, z + depth];
        if intersects(origin, direction, [x, 0.0, z], end) {
            let distance = distance([end[0] - 0.5, height, end[2] - 0.5], origin);
            hits[*count] = [
                distance * direction[0] + origin[0],
                height,
                distance * direction[2] + origin[2],
            ];
            *count += 1;
        }
    }
}

pub fn terrain_height(map: &MapData, texels: &[u8], [x, z]: [f32; 2]) -> f32 {
    let x = ((x + 0.5) as i32).clamp(0, map.width as i32 - 1) as usize;
    let z = ((z + 0.5) as i32).clamp(0, map.height as i32 - 1) as usize;
    let byte = texels[(map.height as usize - 1 - z) * map.width as usize + x];
    (byte as f32 * 0.1).max(9.5)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let [x, y, z] = std::array::from_fn::<_, 3, _>(|axis| a[axis] - b[axis]);
    (y * y + x * x + z * z).sqrt()
}

fn intersects(origin: [f32; 3], direction: [f32; 3], min: [f32; 3], max: [f32; 3]) -> bool {
    let interval = |axis: usize| {
        let inverse = 1.0 / direction[axis];
        let (near, far) = if inverse < 0.0 {
            (max[axis], min[axis])
        } else {
            (min[axis], max[axis])
        };
        (
            (near - origin[axis]) * inverse,
            (far - origin[axis]) * inverse,
        )
    };
    let (x_near, x_far) = interval(0);
    let (y_near, y_far) = interval(1);
    if !(x_near <= y_far && y_near <= x_far) {
        return false;
    }
    let near = if y_near <= x_near { x_near } else { y_near };
    let far = if x_far <= y_far { x_far } else { y_far };
    let (z_near, z_far) = interval(2);
    if !(near <= z_far && z_near <= far) {
        return false;
    }
    let near = if z_near <= near { near } else { z_near };
    let far = if far <= z_far { far } else { z_far };
    near < f32::MAX && 0.0 < far
}

#[cfg(test)]
mod tests;
