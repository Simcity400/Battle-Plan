use super::*;

#[test]
fn rail_height_interpolates_in_x_then_z_and_clamps_sample_indices() {
    let values = [150, 170, 100, 110];
    let heights = HeightField {
        width: 2,
        height: 2,
        values: &values,
    };
    assert_eq!(heights.border_height_at_world(0.5, 0.25), 11.875);
    // Outside the bitmap the base indices clamp; the fractional coordinate extrapolates.
    assert_eq!(heights.border_height_at_world(-0.5, 0.25), 10.625);
    assert_eq!(heights.border_height_at_world(-10.0, 0.0), WATER_HEIGHT);
}

#[test]
fn world_and_pixel_space_round_trip() {
    assert_eq!(world_from_pixel(2048, 10.0, 0.0), (10.0, 2047.0));
    assert_eq!(world_from_pixel(2048, 10.0, 2047.0), (10.0, 0.0));
    assert_eq!(world_from_pixel(4096, 10.0, 0.0), (10.0, 4095.0));
    let (x, z) = world_from_pixel(2048, 1234.5, 678.25);
    assert_eq!(pixel_from_world(2048, x, z), (1234.5, 678.25));
}

/// The surface follows the terrain pass's triangles: one vertex per pixel, each cell split along
/// the diagonal its index pattern picks, the map wrapping in x and the water plane underneath.
#[test]
fn surface_height_follows_the_drawn_triangles() {
    // Four by two pixels, rows top first: the bottom row (z = 0) is 100, 200, 0, 0 and the top
    // row (z = 1) is 200, 100, 0, 0.
    let values = [200, 100, 0, 0, 100, 200, 0, 0];
    let field = HeightField {
        width: 4,
        height: 2,
        values: &values,
    };
    // Vertices sit at their bytes times a tenth.
    assert_eq!(field.surface_height(0.0, 0.0), 10.0);
    assert_eq!(field.surface_height(1.0, 0.0), 20.0);
    // Cell (0, 0) has an even sum, so its diagonal runs from (0, 0) to (1, 1): both 10 high
    // along it, while bilinear sampling would give 15 at the centre.
    assert!((field.surface_height(0.5, 0.5) - 10.0).abs() < 1e-5);
    assert!((field.surface_height(0.75, 0.25) - 15.0).abs() < 1e-5);
    // Cell (1, 0) has an odd sum, so its diagonal runs from (2, 0) to (1, 1) and its south-west
    // triangle is (1, 0), (2, 0) and (1, 1): 20, 0 and 10 high.
    assert!((field.surface_height(1.25, 0.25) - 12.5).abs() < 1e-5);
    // (2, 0) is 0, under the water plane.
    assert_eq!(field.surface_height(2.0, 0.0), WATER_HEIGHT);
    // The map wraps in x: column 4 is column 0 again.
    assert_eq!(
        field.surface_height(4.0, 0.0),
        field.surface_height(0.0, 0.0)
    );
    assert_eq!(
        field.surface_height(-4.0, 0.0),
        field.surface_height(0.0, 0.0)
    );
    // z clamps to the map.
    assert_eq!(field.surface_height(0.0, -5.0), 10.0);
    assert_eq!(field.surface_height(0.0, 9.0), 20.0);
}
