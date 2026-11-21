use viewer_foundation::map::camera::Camera;

pub const TOLERANCE: f32 = 0.5;
pub const HEIGHT_TOLERANCE: f32 = 0.1;
pub const MAX_SHIFT_PIXELS: f32 = 0.3;

pub fn coarse(camera: &Camera, viewport: [u32; 2]) -> bool {
    camera.free().is_none() && camera.height >= threshold_height(camera, viewport[1])
}

pub fn drawn(fine: u32, coarse: u32, at_coarse: bool) -> std::ops::Range<u32> {
    if at_coarse { fine..fine + coarse } else { 0..fine }
}

pub fn threshold_height(camera: &Camera, viewport_height: u32) -> f32 {
    let tan = (camera.lens().fov_y * 0.5).tan();
    TOLERANCE / MAX_SHIFT_PIXELS * viewport_height as f32 / (2.0 * tan)
}

pub fn offset(a: [f32; 2], b: [f32; 2], point: [f32; 2]) -> (f32, f32) {
    let d = [b[0] - a[0], b[1] - a[1]];
    let p = [point[0] - a[0], point[1] - a[1]];
    let length = d[0] * d[0] + d[1] * d[1];
    let t = if length > 0.0 {
        ((p[0] * d[0] + p[1] * d[1]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let near = [p[0] - d[0] * t, p[1] - d[1] * t];
    ((near[0] * near[0] + near[1] * near[1]).sqrt(), t)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn simplify(count: usize, anchors: &[usize], deviation: impl Fn(usize, usize, usize) -> f32) -> Vec<usize> {
    if count <= 2 {
        return (0..count).collect();
    }
    let mut keep = vec![false; count];
    keep[0] = true;
    keep[count - 1] = true;
    for &anchor in anchors {
        if let Some(slot) = keep.get_mut(anchor) {
            *slot = true;
        }
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for (index, &kept) in keep.iter().enumerate().skip(1) {
        if kept {
            spans.push((start, index));
            start = index;
        }
    }
    while let Some((a, b)) = spans.pop() {
        let mut worst = (1.0_f32, None);
        for index in a + 1..b {
            let off = deviation(a, b, index);
            if off > worst.0 {
                worst = (off, Some(index));
            }
        }
        if let (_, Some(split)) = worst {
            keep[split] = true;
            spans.push((a, split));
            spans.push((split, b));
        }
    }
    keep.iter().enumerate().filter(|(_, kept)| **kept).map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests;
