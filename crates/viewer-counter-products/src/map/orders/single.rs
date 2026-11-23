
use viewer_gpu::province_geometry::{Centers, province_at};

use content_reader_map::map::MapData;

use super::fixed::{self, Point, SCALE};

const MAX_RAY_SAMPLES: usize = 1_025;
const MAX_CONTROLS: usize = 8_192;

pub(super) fn points(
    centers: &Centers,
    map: &MapData,
    province: u32,
    predecessor_path: &[u32],
    centroid: impl Fn(u32) -> Option<[i32; 2]>,
) -> Vec<Point> {
    let Some(&center) = centers.centers.get(&province) else {
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
    None
}

fn squared_distance([ax, az]: Point, [bx, bz]: Point) -> i64 {
    let x = bx - ax;
    let z = bz - az;
    x * x / SCALE + z * z / SCALE
}

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
    use viewer_gpu::test_support::province_geometry::*;

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
        let points = super::points(&geometry, &map, 1, &[], lookup);
        assert_eq!(points.len(), 22);
        assert_eq!(points[0], [482_840; 2]);
        assert_eq!(points[1], points[0]);
        assert_eq!(points.last(), Some(&[-82_840; 2]));
        assert!(super::points(&geometry, &map, 7, &[], lookup).is_empty());
        let points = super::points(&geometry, &map, 1, &[2, 1], lookup);
        assert_eq!(points.len(), 14);
        assert_eq!(points[0], [200_000, 451_130]);
        assert_eq!(points.last(), Some(&[200_000, -51_130]));
    }
}
