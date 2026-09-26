//! The single-province, non-move branch of `BuildOffensiveBoundary@141243C90`
//! (its `count == 1 && type != 1` tail): a line whose path is one province and
//! that did not snap to a border.
//!
//! The native line extends from an owned pixel in both half-unit directions,
//! stopping at the first sample outside the province. It submits those outside
//! positions to `rail_arrow_geometry_builder@141251430` with smoothing off and
//! skips the ordinary smoothing loop. Province pixels and centroids come only
//! from the matching map content.

use std::collections::BTreeMap;

use installed_content::map::MapData;

use super::fixed::{self, Point, SCALE};

const MAX_RAY_SAMPLES: usize = 1_025;
const MAX_CONTROLS: usize = 8_192;

/// The native nearest-owned-pixel search result, cached once per map for the
/// provinces whose integer centroid lies outside their own raster.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Centers {
    centers: BTreeMap<u32, [i64; 2]>,
}

impl Centers {
    pub(super) fn build(map: &MapData, centroids: &[Option<[i32; 2]>]) -> Self {
        let mut centers = BTreeMap::new();
        let largest = map.province_ids.iter().copied().max().unwrap_or(0) as usize;
        let mut candidates = vec![None; largest + 1];
        for (province, centroid) in centroids.iter().enumerate() {
            let Some(centroid) = centroid else { continue };
            if province == 0 || province > largest {
                continue;
            }
            let centroid = centroid.map(i64::from);
            if province_at(map, centroid) == province as u32 {
                centers.insert(province as u32, centroid);
            } else {
                candidates[province] = Some((centroid, i64::MAX, None));
            }
        }
        // 140A507D0 builds province runs in increasing world Z then X, and the
        // search (141245525) accepts only a strictly smaller Manhattan distance,
        // so the first pixel wins ties, including on disconnected shapes.
        if candidates.iter().any(Option::is_some) {
            for z in 0..map.height {
                let row = (map.height - 1 - z) as usize * map.width as usize;
                for x in 0..map.width {
                    let province = map.province_ids[row + x as usize];
                    let Some((centroid, distance, closest)) = &mut candidates[province as usize]
                    else {
                        continue;
                    };
                    let point = [i64::from(x), i64::from(z)];
                    let next = (point[0] - centroid[0]).abs() + (point[1] - centroid[1]).abs();
                    if next < *distance {
                        *distance = next;
                        *closest = Some(point);
                    }
                }
            }
        }
        centers.extend(
            candidates
                .into_iter()
                .enumerate()
                .filter_map(|(province, candidate)| {
                    candidate?.2.map(|point| (province as u32, point))
                }),
        );
        Self { centers }
    }

    /// The stored controls, including the native first-point duplicate and the
    /// inserted controls. The caller must not smooth them again.
    pub(super) fn points(
        &self,
        map: &MapData,
        province: u32,
        predecessor_path: &[u32],
        centroid: impl Fn(u32) -> Option<[i32; 2]>,
    ) -> Vec<Point> {
        let Some(&center) = self.centers.get(&province) else {
            return Vec::new();
        };
        let Some(direction) = direction(province, predecessor_path, centroid) else {
            return Vec::new();
        };
        let center = center.map(|v| v * SCALE);
        let step = direction.map(|v| v * 50_000 / SCALE);
        let mut controls = Vec::new();
        if let Some(point) = ray_endpoint(map, province, center, step) {
            append_control(&mut controls, point);
        }
        if let Some(point) = ray_endpoint(map, province, center, step.map(|v| -v)) {
            append_control(&mut controls, point);
        }
        controls
    }
}

fn province_at(map: &MapData, [x, z]: [i64; 2]) -> u32 {
    if x < 0 || z < 0 || x >= i64::from(map.width) || z >= i64::from(map.height) {
        return 0;
    }
    let row = map.height as usize - 1 - z as usize;
    map.province_ids[row * map.width as usize + x as usize]
}

/// The diagonal `(1, 1)`, or the quarter-turned direction from the last
/// different predecessor province. `FFFFFFFF` stops the scan and keeps the
/// diagonal; it never skips back to an earlier valid province.
fn direction(
    province: u32,
    predecessor_path: &[u32],
    centroid: impl Fn(u32) -> Option<[i32; 2]>,
) -> Option<Point> {
    let mut direction = fixed::normalize([SCALE, SCALE]);
    if let Some(&previous) = predecessor_path.iter().rev().find(|&&id| id != province)
        && previous != u32::MAX
    {
        let target = fixed::from_pixel(centroid(province)?);
        let previous = fixed::from_pixel(centroid(previous)?);
        direction = fixed::rotate_quarter(fixed::normalize([
            target[0] - previous[0],
            target[1] - previous[1],
        ]));
    }
    Some(direction)
}

fn ray_endpoint(map: &MapData, province: u32, center: Point, step: Point) -> Option<Point> {
    let mut point = center;
    for _ in 0..MAX_RAY_SAMPLES {
        point = [point[0] + step[0], point[1] + step[1]];
        let sampled = province_at(map, point.map(fixed::round));
        if sampled == 0 || sampled != province {
            return Some(point);
        }
    }
    // 1025 samples inside the province submit nothing; neither the last
    // interior pixel nor a fabricated endpoint is drawn.
    None
}

fn squared_distance([ax, az]: Point, [bx, bz]: Point) -> i64 {
    let x = bx - ax;
    let z = bz - az;
    x * x / SCALE + z * z / SCALE
}

/// The geometry half of `141251430` with smoothing off (`param_3 = 0`).
fn append_control(controls: &mut Vec<Point>, point: Point) {
    let count = controls.len();
    if count >= MAX_CONTROLS {
        return;
    }
    if count < 2 {
        controls.extend([point, point]);
        return;
    }
    let threshold = (count as i64 * 2_500).min(SCALE) * 300_000 / SCALE + 800;
    if squared_distance(controls[count - 2], point) <= threshold {
        controls[count - 1] = point;
        return;
    }
    let mut previous = controls[count - 1];
    // Native CVTSI2SS/DIVSS/SQRTSS/DIVSS/CVTTSS2SI, in f32.
    let distance = (squared_distance(previous, point) as f32 / SCALE as f32).sqrt();
    let spacing = (threshold as f32 / SCALE as f32).sqrt();
    let count = (distance / spacing) as i32;
    if count > 1 {
        let divisor = i64::from(count) * SCALE;
        let step = [
            (point[0] - previous[0]) * SCALE / divisor,
            (point[1] - previous[1]) * SCALE / divisor,
        ];
        for _ in 0..count - 1 {
            previous = [previous[0] + step[0], previous[1] + step[1]];
            controls.push(previous);
            if controls.len() >= MAX_CONTROLS - 1 {
                break;
            }
        }
    }
    controls.push(point);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_map(width: u32, height: u32, province_ids: &[u32]) -> MapData {
        assert_eq!(province_ids.len(), (width * height) as usize);
        MapData {
            width,
            height,
            province_ids: province_ids.to_vec(),
            definitions: BTreeMap::new(),
            country_colors: Default::default(),
        }
    }

    fn centroids(entries: &[(u32, [i32; 2])]) -> Vec<Option<[i32; 2]>> {
        let mut out = vec![None; 16];
        for &(id, c) in entries {
            out[id as usize] = Some(c);
        }
        out
    }

    #[test]
    fn nearest_owned_pixel_uses_world_row_then_column_tie_order() {
        let map = fixture_map(3, 3, &[0, 1, 0, 1, 0, 1, 0, 1, 0]);
        let geometry = Centers::build(&map, &centroids(&[(1, [1, 1])]));
        assert_eq!(geometry.centers[&1], [1, 0]);
        let map = fixture_map(3, 1, &[1, 0, 1]);
        let geometry = Centers::build(&map, &centroids(&[(1, [1, 0])]));
        assert_eq!(geometry.centers[&1], [0, 0]);
    }

    #[test]
    fn predecessor_scan_stops_at_the_last_different_id_even_when_invalid() {
        let c = centroids(&[(1, [2, 2]), (2, [0, 2])]);
        let lookup = |id: u32| c.get(id as usize).copied().flatten();
        assert_eq!(direction(1, &[2, 1, 1], lookup), Some([-1, 100_453]));
        assert_eq!(
            direction(1, &[2, u32::MAX, 1], lookup),
            Some([70_710, 70_710])
        );
        assert_eq!(direction(1, &[9, 1], lookup), None);
    }

    #[test]
    fn rays_submit_the_first_outside_position_without_wrapping() {
        let map = fixture_map(3, 1, &[1, 1, 1]);
        assert_eq!(
            ray_endpoint(&map, 1, [SCALE, 0], [50_000, 0]),
            Some([250_000, 0])
        );
        assert_eq!(
            ray_endpoint(&map, 1, [SCALE, 0], [-50_000, 0]),
            Some([-50_000, 0])
        );
        assert_eq!(ray_endpoint(&map, 1, [SCALE, 0], [0, 0]), None);
    }

    #[test]
    fn appended_controls_keep_duplicates_and_truncated_increments() {
        let mut points = Vec::new();
        append_control(&mut points, [0, 0]);
        append_control(&mut points, [SCALE + 1, 0]);
        assert_eq!(points, [[0, 0], [0, 0], [50_000, 0], [SCALE + 1, 0]]);
        let mut close = vec![[0, 0], [0, 0]];
        append_control(&mut close, [39_750, 0]);
        assert_eq!(close, [[0, 0], [39_750, 0]]);
    }

    #[test]
    fn single_province_points_keep_the_dense_native_control_sequence() {
        let map = fixture_map(5, 5, &[1; 25]);
        let c = centroids(&[(1, [2, 2]), (2, [0, 2])]);
        let lookup = |id: u32| c.get(id as usize).copied().flatten();
        let geometry = Centers::build(&map, &c);
        let points = geometry.points(&map, 1, &[], lookup);
        assert_eq!(points.len(), 22);
        assert_eq!(points[0], [482_840; 2]);
        assert_eq!(points[1], points[0]);
        assert_eq!(points.last(), Some(&[-82_840; 2]));
        assert!(geometry.points(&map, 7, &[], lookup).is_empty());
        let points = geometry.points(&map, 1, &[2, 1], lookup);
        assert_eq!(points.len(), 14);
        assert_eq!(points[0], [200_000, 451_130]);
        assert_eq!(points.last(), Some(&[200_000, -51_130]));
    }
}
