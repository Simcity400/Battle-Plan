//! The coarse level of the map's line meshes, borders and rivers: which points it keeps, and the
//! cameras that draw it.
//!
//! Zoomed out, a traced line has several points to a pixel, so its triangles are far smaller than
//! a pixel and the GPU's time goes to setting them up. The coarse level drops every point whose
//! line stands within [`TOLERANCE`] world units of the line without it, and draws only where that
//! is under [`MAX_SHIFT_PIXELS`] of a pixel, so crossing the threshold moves nothing visibly. The
//! strip is widened again from the kept points, so its width and colours are the fine level's.
use viewer_foundation::map::camera::Camera;

/// World units a coarse line may stand off the fine one.
pub const TOLERANCE: f32 = 0.5;
/// World units the coarse line's height may stand off the fine one's: a strip that sinks under the
/// terrain fails its depth test, so this is one height step.
pub const HEIGHT_TOLERANCE: f32 = 0.1;
/// The most a coarse line may stand off the fine one where it is drawn, in pixels.
pub const MAX_SHIFT_PIXELS: f32 = 0.3;

/// Whether this camera draws the coarse level into a target `viewport` pixels in size.
///
/// The nearest ground a map camera sees is at least its height away, so a world unit there covers
/// at most `viewport height / (2 height tan(fov / 2))` pixels. A free eye can stand next to a line
/// at any height, so it always draws the fine level.
pub fn coarse(camera: &Camera, viewport: [u32; 2]) -> bool {
    camera.free().is_none() && camera.height >= threshold_height(camera, viewport[1])
}

/// Of an index list holding `fine` indices of the fine level then `coarse` of the coarse level, the
/// ones drawn when the camera draws the coarse level or not.
pub fn drawn(fine: u32, coarse: u32, at_coarse: bool) -> std::ops::Range<u32> {
    if at_coarse { fine..fine + coarse } else { 0..fine }
}

/// The camera height from which a map camera draws the coarse level, `viewport_height` pixels high.
pub fn threshold_height(camera: &Camera, viewport_height: u32) -> f32 {
    let tan = (camera.lens().fov_y * 0.5).tan();
    TOLERANCE / MAX_SHIFT_PIXELS * viewport_height as f32 / (2.0 * tan)
}

/// How far `point` stands off the segment `a`–`b`, and where along it the nearest point lies
/// (0 at `a`, 1 at `b`): what an attribute is interpolated at.
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

/// The indices of the points a coarse line keeps, ascending: the first and last, every anchor, and
/// every point whose `deviation(a, b, i)` from the line between the kept points `a` and `b` either
/// side of it exceeds 1 (Douglas–Peucker).
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
