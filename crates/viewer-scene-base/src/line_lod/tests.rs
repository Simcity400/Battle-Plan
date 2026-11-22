use super::*;
use viewer_gpu::camera::{Free, Lens, MapDefines, MapSize, Placement, Pose};

fn map_camera(height: f32) -> Camera {
    Placement::map([2816.0, 1024.0], height).on(MapSize::TEST, MapDefines::test())
}

#[test]
fn the_coarse_level_starts_where_a_tolerance_is_under_the_largest_shift_a_pixel_allows() {
    let camera = map_camera(1000.0);
    let at_1080 = threshold_height(&camera, 1080);
    assert!((at_1080 - 2174.0).abs() < 0.5, "{at_1080}");
    assert!((threshold_height(&camera, 2160) - 2.0 * at_1080).abs() < 1e-2);
    let world_per_pixel = 2.0 * at_1080 * (Lens::MAP.fov_y * 0.5).tan() / 1080.0;
    assert!((TOLERANCE / world_per_pixel - MAX_SHIFT_PIXELS).abs() < 1e-4);

    assert!(coarse(&map_camera(3000.0), [1920, 1080]));
    assert!(coarse(&map_camera(at_1080 + 1.0), [1920, 1080]));
    assert!(!coarse(&map_camera(at_1080 - 1.0), [1920, 1080]));
    assert!(!coarse(&map_camera(150.0), [1920, 1080]));
    assert!(!coarse(&map_camera(3000.0), [3840, 2160]));
}

#[test]
fn a_free_eye_always_draws_the_fine_level() {
    let mut camera = map_camera(3000.0);
    camera.pose = Pose::Free(Free::new(0.0, -1.2, Lens::MAP));
    assert!(!coarse(&camera, [1920, 1080]));
}

fn line_deviation(points: &[[f32; 2]]) -> impl Fn(usize, usize, usize) -> f32 + '_ {
    |a, b, i| offset(points[a], points[b], points[i]).0 / TOLERANCE
}

#[test]
fn simplify_keeps_the_ends_and_every_point_off_the_line_by_more_than_the_tolerance() {
    let straight: Vec<[f32; 2]> = (0..10).map(|x| [x as f32, 0.2 * (x % 2) as f32]).collect();
    assert_eq!(simplify(straight.len(), &[], line_deviation(&straight)), vec![0, 9]);

    let bend = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 2.0], [4.0, 4.2], [5.0, 4.8], [6.0, 4.0]];
    let kept = simplify(bend.len(), &[], line_deviation(&bend));
    assert_eq!(kept, vec![0, 2, 4, 5, 6]);
    for window in kept.windows(2) {
        for i in window[0] + 1..window[1] {
            assert!(offset(bend[window[0]], bend[window[1]], bend[i]).0 <= TOLERANCE);
        }
    }
}

#[test]
fn simplify_keeps_anchors_and_short_lines_whole() {
    let straight: Vec<[f32; 2]> = (0..10).map(|x| [x as f32, 0.0]).collect();
    assert_eq!(simplify(straight.len(), &[3, 7, 40], line_deviation(&straight)), vec![0, 3, 7, 9]);
    assert_eq!(simplify(2, &[], |_, _, _| 9.0), vec![0, 1]);
    assert_eq!(simplify(1, &[], |_, _, _| 9.0), vec![0]);
}

#[test]
fn offset_measures_to_the_nearest_point_of_the_segment() {
    assert_eq!(offset([0.0, 0.0], [4.0, 0.0], [1.0, 3.0]), (3.0, 0.25));
    assert_eq!(offset([0.0, 0.0], [4.0, 0.0], [6.0, 0.0]), (2.0, 1.0));
    assert_eq!(offset([1.0, 1.0], [1.0, 1.0], [4.0, 5.0]), (5.0, 0.0));
}

#[test]
fn the_wrap_copies_are_the_ones_the_view_reaches() {
    assert_eq!(wrap_offsets(None, 5632.0), [0.0]);
    assert_eq!(
        wrap_offsets(Some([100.0, 0.0, 400.0, 100.0]), 5632.0),
        [0.0]
    );
    assert_eq!(
        wrap_offsets(Some([-40.0, 0.0, 400.0, 100.0]), 5632.0),
        [0.0, -5632.0]
    );
    assert_eq!(
        wrap_offsets(Some([5000.0, 0.0, 5700.0, 100.0]), 5632.0),
        [0.0, 5632.0]
    );
    assert_eq!(
        wrap_offsets(Some([-100.0, 0.0, 5700.0, 100.0]), 5632.0),
        [0.0, -5632.0, 5632.0]
    );
}

#[test]
fn a_free_reach_takes_every_copy_it_overlaps() {
    let width = 5632.0;
    assert_eq!(free_wrap_offsets(None, width), [0.0]);
    assert_eq!(
        free_wrap_offsets(Some([-100.0, 0.0, 5700.0, 100.0]), width),
        wrap_offsets(Some([-100.0, 0.0, 5700.0, 100.0]), width)
    );
    assert_eq!(
        free_wrap_offsets(Some([-12_000.0, 0.0, 15_000.0, 100.0]), width),
        [0.0, -width, width, -2.0 * width, 2.0 * width, -3.0 * width]
    );
    assert_eq!(
        free_wrap_offsets(Some([6000.0, 0.0, 12_000.0, 100.0]), width),
        [0.0, width, 2.0 * width]
    );
}
