//! Political-map province selection: 140B44760, 140B46960 and 140B44DD0.

use installed_content::map::MapData;

use super::{camera::Camera, province_admission};

/// Pick against native nearest-sampled height cells, then the y=0 plane.
pub fn pick(
    camera: &Camera,
    screen: [f32; 2],
    viewport: [u32; 2],
    map: &MapData,
    heightmap: &[u8],
) -> Result<Option<u32>, String> {
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
    let views = views(*camera, viewport);
    let mut hits = [[0.0; 3]; 256]; // Native candidate capacity, 140B44946.
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
        // 140B44985 evaluates the accumulated candidates against each view.
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
    let point = best.unwrap_or_else(|| plane_pick(camera, screen, viewport));
    Ok(raster(map, point))
}

/// Seam flags and the single-width recenter are 14124C8C0, shared with admission.
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

fn plane_pick(camera: &Camera, screen: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
    // 140B44360 uses y=0, |origin.y/direction.y|, epsilon .0001 and far 4000.
    for view in views(*camera, viewport).into_iter().flatten() {
        let direction = view.pick_ray(screen, viewport);
        let origin = view.eye();
        let distance = if direction[1].abs() >= 0.0001 {
            (origin[1] / direction[1]).abs()
        } else {
            4000.0
        };
        let x = direction[0] * distance + origin[0];
        if x >= 0.0 && x <= camera.map.x() {
            return [x, 0.0, direction[2] * distance + origin[2]];
        }
    }
    // Native no-view fallback writes the zero vector.
    [0.0; 3]
}

/// 140B44DD0 truncates coordinates; native raster rows count from the south.
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
    // 140B46960 halves only axes >=2, preserving native quadrant order.
    if width >= 2.0 || depth >= 2.0 {
        let width = if width >= 2.0 { width * 0.5 } else { width };
        let depth = if depth >= 2.0 { depth * 0.5 } else { depth };
        for [x, z] in [
            [x, z],
            [x + width, z],
            [x, z + depth],
            [x + width, z + depth],
        ] {
            // 140B4485E's coarse maximum is .1 * 255, not the sea plane.
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
        let height = height(map, heightmap, [x, z]);
        let end = [x + width, height, z + depth];
        if intersects(origin, direction, [x, 0.0, z], end) {
            // The game uses distance to the cell's upper corner minus .5, then
            // forces hit.y to sampled height; it does not intersect a triangle.
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

/// 140A51CC0: nearest texel, clamp both axes, multiply .1, clamp to 9.5.
fn height(map: &MapData, texels: &[u8], [x, z]: [f32; 2]) -> f32 {
    let x = ((x + 0.5) as i32).clamp(0, map.width as i32 - 1) as usize;
    let z = ((z + 0.5) as i32).clamp(0, map.height as i32 - 1) as usize;
    let byte = texels[(map.height as usize - 1 - z) * map.width as usize + x];
    (byte as f32 * 0.1).max(9.5)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let [x, y, z] = std::array::from_fn::<_, 3, _>(|axis| a[axis] - b[axis]);
    // 140B46AB1 adds y²+x² before z².
    (y * y + x * x + z * z).sqrt()
}

/// 1424E6270's slabs, including strict near/far interval tests.
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
