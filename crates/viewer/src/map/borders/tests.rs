const FADES: [[f32; 2]; 2] = [[200.0, 300.0], [400.0, 500.0]];
use super::*;

fn flat(width: u32, height: u32) -> Vec<u8> {
    vec![95; (width * height) as usize]
}

fn always(class: BorderClass) -> impl Fn(u32, u32) -> Option<BorderClass> {
    move |_, _| Some(class)
}

/// No province in these maps is water, so no chain takes the wide smoothing.
fn dry(_province: u32) -> bool {
    false
}

/// A map split into a top and a bottom province, whose single boundary runs from the map's left
/// edge to its right edge without touching the x seam.
fn split_rows(width: u32, height: u32, row: u32) -> Vec<u32> {
    let mut ids = Vec::new();
    for y in 0..height {
        for _ in 0..width {
            ids.push(if y < row { 1 } else { 2 });
        }
    }
    ids
}

#[test]
fn two_provinces_trace_one_polyline_on_pixel_crack_midpoints() {
    let ids = split_rows(8, 8, 4);
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let traced = trace(&map, &always(BorderClass::Country), &dry).unwrap();
    assert_eq!(traced.polylines.len(), 1);
    let polyline = &traced.polylines[0];
    assert_eq!((polyline.province_a, polyline.province_b), (1, 2));
    assert_eq!(polyline.class, BorderClass::Country);
    assert!(!polyline.closed);
    assert_eq!(polyline.points.len(), 8, "one point per pixel edge crossed");
    // Every point is the mean of the two pixel centres its crack separates: on the pixel
    // column across the row, half a pixel between the two rows along it, converted to world z.
    let (_, z) = world_from_pixel(8, 0.0, 3.5);
    for (index, point) in polyline.points.iter().enumerate() {
        assert_eq!(*point, [index as f32, z]);
    }
    assert_eq!(z, 3.5);
    assert!(traced.one_pixel_provinces.is_empty());
}

#[test]
fn an_island_traces_a_closed_loop() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut ids = vec![1_u32; 64];
    for y in 3..5 {
        for x in 3..5 {
            ids[y * 8 + x] = 2;
        }
    }
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let traced = trace(&map, &always(BorderClass::Province), &dry).unwrap();
    assert_eq!(traced.polylines.len(), 1);
    let polyline = &traced.polylines[0];
    assert!(polyline.closed, "a province inside another closes");
    assert_eq!(
        polyline.points.len(),
        9,
        "two cracks on each of four sides, and the first point repeated as the last"
    );
    assert_eq!(polyline.points[0], polyline.points[8]);
    assert_eq!(polyline.points[0], [3.0, world_from_pixel(8, 0.0, 4.5).1]);
    let mut sorted = polyline.points[..8].to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let expected: Vec<[f32; 2]> = [
        [2.5, 3.0],
        [2.5, 4.0],
        [3.0, 2.5],
        [3.0, 4.5],
        [4.0, 2.5],
        [4.0, 4.5],
        [4.5, 3.0],
        [4.5, 4.0],
    ]
    .into_iter()
    .map(|[x, y]| {
        let (x, z) = world_from_pixel(8, x, y);
        [x, z]
    })
    .collect();
    let mut expected = expected;
    expected.sort_by(|left, right| left.partial_cmp(right).unwrap());
    assert_eq!(sorted, expected);
    // Walking the loop never steps more than one pixel corner at a time.
    for window in polyline.points[..8].windows(2) {
        let step = (window[1][0] - window[0][0]).hypot(window[1][1] - window[0][1]);
        assert!(step <= 1.001, "step {step}");
    }
}

#[test]
fn the_x_seam_is_a_boundary_and_cuts_the_chain() {
    // Two provinces side by side: they meet down the middle of the map and again across the
    // seam, because the world wraps.
    let mut ids = Vec::new();
    for _ in 0..8 {
        for x in 0..8 {
            ids.push(if x < 4 { 1_u32 } else { 2 });
        }
    }
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let traced = trace(&map, &always(BorderClass::Country), &dry).unwrap();
    assert_eq!(traced.polylines.len(), 2, "the middle and the seam");
    let mut columns: Vec<f32> = traced
        .polylines
        .iter()
        .map(|line| line.points[0][0])
        .collect();
    columns.sort_by(|left, right| left.partial_cmp(right).unwrap());
    // The crack between columns 3 and 4, and the seam crack between column 7 and column 0.
    assert_eq!(columns, vec![3.5, 7.5]);
    for polyline in &traced.polylines {
        assert_eq!(polyline.points.len(), 8);
        assert!(!polyline.closed);
    }
}

/// Three provinces meeting at one lattice corner: the two chains that end there must both
/// carry that corner, or their strips stop a map unit short of each other and the line reads
/// as a dashed line.
#[test]
fn chains_that_meet_at_a_triple_point_share_its_corner() {
    // Left half is province 1; the right half is 2 above row 4 and 3 below it, so 1, 2 and 3
    // all meet at the lattice corner (4, 4).
    let mut ids = Vec::new();
    for y in 0..8_u32 {
        for x in 0..8_u32 {
            ids.push(match (x < 4, y < 4) {
                (true, _) => 1_u32,
                (false, true) => 2,
                (false, false) => 3,
            });
        }
    }
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let components = BoundaryComponents::new(&map);
    assert!(components.is_junction([4, 4]), "1, 2 and 3 meet here");
    assert!(!components.is_junction([4, 2]), "only 1 and 2 meet here");
    assert!(
        !components.is_junction([2, 4]),
        "province 1 on all four sides"
    );
    let traced = trace(&map, &always(BorderClass::Country), &dry).unwrap();
    let corner = {
        let (x, z) = world_from_pixel(8, 3.5, 3.5);
        [x, z]
    };
    let touching: Vec<&Polyline> = traced
        .polylines
        .iter()
        .filter(|line| line.points.contains(&corner))
        .collect();
    assert_eq!(
        touching.len(),
        3,
        "the 1|2 chain, the 1|3 chain and the 2|3 chain all end at the corner"
    );
    for polyline in touching {
        let ends = [
            polyline.points[0],
            polyline.points[polyline.points.len() - 1],
        ];
        assert!(ends.contains(&corner), "{:?}", polyline.points);
    }
}

#[test]
fn smoothing_leaves_the_endpoints_where_the_trace_put_them() {
    let points = [[0.0, 0.0], [1.0, 0.0], [2.0, 4.0], [3.0, 0.0]];
    let smoothed = smooth(&points, false);
    // The endpoints are the join with the next chain and are never moved.
    assert_eq!(smoothed[0], points[0]);
    assert_eq!(smoothed[3], points[3]);
    // Interior points are the mean of themselves and their two neighbours.
    assert_eq!(smoothed[1], [1.0, 4.0 / 3.0]);
    assert_eq!(smoothed[2], [2.0, 4.0 / 3.0]);
    // A straight run stays exactly where it is: no slide along itself.
    let straight = [[0.0, 7.0], [1.0, 7.0], [2.0, 7.0], [3.0, 7.0]];
    assert_eq!(smooth(&straight, false), straight.to_vec());
    // Too short to filter.
    assert_eq!(smooth(&[[1.0, 2.0]], false), vec![[1.0, 2.0]]);
    assert_eq!(
        smooth(&[[1.0, 2.0], [3.0, 4.0]], true),
        vec![[1.0, 2.0], [3.0, 4.0]]
    );
}

#[test]
fn a_water_to_water_chain_takes_the_wide_two_pass_filter() {
    // A spike on an otherwise straight run: one 3-tap pass leaves a third of it, two passes of
    // a radius that opens up to six flatten it far harder.
    let mut points: Vec<[f32; 2]> = (0..17).map(|i| [i as f32, 0.0]).collect();
    points[8][1] = 9.0;
    let narrow = smooth(&points, false);
    let wide = smooth(&points, true);
    assert_eq!(narrow[8], [8.0, 3.0]);
    // Radius at index 8 of a 17 point chain is min(8, 8, 6) = 6, so the first pass spreads the
    // spike over thirteen taps and the second pass spreads what is left again.
    assert!(wide[8][1] < 1.0, "{:?}", wide[8]);
    assert!(wide[8][1] > 0.0);
    // Neither filter moves an endpoint.
    assert_eq!(narrow[0], points[0]);
    assert_eq!(wide[0], points[0]);
    assert_eq!(wide[16], points[16]);
    // The radius is capped, so a point seven from the end still averages thirteen taps.
    assert_eq!(WIDE_SMOOTHING_RADIUS, 6);
}

#[test]
fn widened_quads_carry_the_right_rails_uv_and_winding() {
    let ids = split_rows(8, 8, 4);
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let values = flat(8, 8);
    let heights = HeightField {
        width: 8,
        height: 8,
        values: &values,
    };
    let meshes = build(&map, &always(BorderClass::Country), &dry, &heights, 1.5).unwrap();
    let mesh = meshes.mesh(BorderClass::Country);
    assert_eq!(mesh.draws.len(), 1);
    let draw = mesh.draws[0];
    assert_eq!(draw.vertex_count, 16, "eight points, two rails");
    assert_eq!(draw.index_count, 7 * 6);
    assert_eq!(draw.base_vertex, 0);
    assert_eq!(draw.chunk, 0);
    let vertices = &mesh.chunks[0];
    assert_eq!(vertices.len(), 16);
    let half = 1.5 * 0.5;
    let (_, z) = world_from_pixel(8, 0.0, 3.5);
    for point in 0..8 {
        let first = vertices[point * 2];
        let second = vertices[point * 2 + 1];
        // The boundary runs east, so the rails sit south and north of it by half the width.
        assert_eq!(first.uv, [0, point as i16]);
        assert_eq!(second.uv, [1, point as i16]);
        assert!((first.position[2] - (z - half)).abs() < 1e-5, "{first:?}");
        assert!((second.position[2] - (z + half)).abs() < 1e-5, "{second:?}");
        assert_eq!(first.position[0], second.position[0]);
        // A flat heightmap of byte 95 is exactly sea level.
        assert_eq!(first.position[1], WATER_HEIGHT);
    }
    // The first point is the first crack midpoint, and smoothing left it there.
    assert_eq!(vertices[0].position[0], 0.0);
    // The shared index buffer is the engine's quad pattern.
    assert_eq!(meshes.indices[..6], [0, 2, 1, 2, 3, 1]);
    assert_eq!(meshes.indices[6..12], [2, 4, 3, 4, 5, 3]);
    assert_eq!(meshes.indices.len(), 7 * 6);
    // Nothing landed in the other five buckets.
    for class in BorderClass::ALL {
        if class != BorderClass::Country {
            assert!(meshes.mesh(class).draws.is_empty());
            assert_eq!(meshes.mesh(class).vertex_count(), 0);
        }
    }
}

#[test]
fn heights_come_from_the_field_under_each_rail() {
    let ids = split_rows(8, 8, 4);
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    // Row 3 of the bitmap is high ground, everything else is at sea level.
    let mut values = flat(8, 8);
    for x in 0..8 {
        values[3 * 8 + x] = 195;
    }
    let heights = HeightField {
        width: 8,
        height: 8,
        values: &values,
    };
    let meshes = build(&map, &always(BorderClass::Country), &dry, &heights, 1.5).unwrap();
    let vertices = &meshes.mesh(BorderClass::Country).chunks[0];
    // The north rail is at row 2.75: one quarter of row 2 and three quarters of row 3.
    // The south rail is over flat sea-level terrain.
    assert_eq!(vertices[1].position[1], 17.0);
    assert_eq!(vertices[0].position[1], WATER_HEIGHT);
    // The field itself: world z for bitmap row 3 of 8 is 4.
    assert_eq!(heights.height_at_world(0.0, 4.0), 19.5);
    assert_eq!(heights.height_at_world(0.0, 3.0), WATER_HEIGHT);
    // x wraps and y clamps rather than panicking.
    assert_eq!(heights.height_at_world(-1.0, 4.0), 19.5);
    assert_eq!(heights.height_at_world(1e9, -1e9), WATER_HEIGHT);
}

#[test]
fn the_classifier_decides_which_boundaries_survive() {
    // Three vertical stripes: 1 | 2 | 3, with the seam putting 3 next to 1 as well.
    let mut ids = Vec::new();
    for _ in 0..8 {
        for x in 0..8 {
            ids.push(match x {
                0..=2 => 1_u32,
                3..=5 => 2,
                _ => 3,
            });
        }
    }
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let classify = |first: u32, second: u32| match (first, second) {
        (1, 2) => Some(BorderClass::State),
        (2, 3) => Some(BorderClass::Impassable),
        _ => None,
    };
    let traced = trace(&map, &classify, &dry).unwrap();
    assert_eq!(traced.polylines.len(), 2, "the 1|3 seam was refused");
    let values = flat(8, 8);
    let heights = HeightField {
        width: 8,
        height: 8,
        values: &values,
    };
    let meshes = build(&map, &classify, &dry, &heights, 1.5).unwrap();
    assert_eq!(meshes.mesh(BorderClass::State).draws.len(), 1);
    assert_eq!(meshes.mesh(BorderClass::Impassable).draws.len(), 1);
    assert_eq!(meshes.mesh(BorderClass::Country).draws.len(), 0);
    assert_eq!(meshes.mesh(BorderClass::Province).draws.len(), 0);
    // The classifier always sees the lower id first.
    let seen = std::cell::RefCell::new(Vec::new());
    let record = |first: u32, second: u32| {
        seen.borrow_mut().push((first, second));
        None
    };
    trace(&map, &record, &dry).unwrap();
    let mut pairs = seen.into_inner();
    pairs.sort_unstable();
    assert_eq!(pairs, vec![(1, 2), (1, 3), (2, 3)]);
}

#[test]
fn one_pixel_provinces_are_reported_without_discarding_their_contour() {
    let mut ids = vec![1_u32; 64];
    ids[3 * 8 + 3] = 9;
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let traced = trace(&map, &always(BorderClass::Province), &dry).unwrap();
    assert_eq!(traced.one_pixel_provinces, vec![9]);
    assert_eq!(traced.polylines.len(), 1);
    assert!(traced.polylines[0].closed);
    assert_eq!(traced.polylines[0].points.len(), 5);
}

#[test]
fn diagonal_contacts_use_component_labels_in_bottom_up_order() {
    let ids = [1, 2, 1, 2, 1, 2];
    let map = ProvinceMap {
        width: 3,
        height: 2,
        ids: &ids,
    };
    let components = BoundaryComponents::new(&map);
    assert_eq!(components.components, [4, 5, 6, 1, 2, 3]);
    // Only two province ids, but four components meet at this corner.
    assert!(components.is_junction([1, 1]));
    // Matching province ids at the left/right seam are not flood-filled together.
    assert_ne!(components.components[0], components.components[2]);
}

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
fn chunking_splits_a_long_polyline_and_keeps_the_strip_continuous() {
    let ids = split_rows(8, 8, 4);
    let map = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    let values = flat(8, 8);
    let heights = HeightField {
        width: 8,
        height: 8,
        values: &values,
    };
    // Eight vertices per chunk is four points per run, so the eight point boundary needs three
    // runs that overlap by a point each.
    let meshes =
        build_with_limit(&map, &always(BorderClass::Country), &dry, &heights, 1.5, 8).unwrap();
    let mesh = meshes.mesh(BorderClass::Country);
    assert_eq!(mesh.chunks.len(), 3);
    assert_eq!(mesh.draws.len(), 3);
    for chunk in &mesh.chunks {
        assert!(chunk.len() <= 8);
    }
    assert_eq!(mesh.draws[0].vertex_count, 8);
    assert_eq!(mesh.draws[0].index_count, 3 * 6);
    assert_eq!(mesh.draws[1].chunk, 1);
    assert_eq!(mesh.draws[1].base_vertex, 0);
    // The runs overlap by one point, so the last point of a run and the first of the next are
    // the same vertex position, and uv.y keeps counting.
    let first = mesh.chunks[0][6];
    let second = mesh.chunks[1][0];
    assert_eq!(first.position, second.position);
    assert_eq!(first.uv, [0, 3]);
    assert_eq!(second.uv, [0, 3]);
    assert_eq!(mesh.chunks[2][0].uv, [0, 6]);
    // Every point still appears: three runs of four points with two shared points.
    // Three runs of four, four and two points, the middle two sharing a point with a
    // neighbour: 4 + 4 + 2 points become 20 vertices for the 8 point boundary.
    assert_eq!(mesh.vertex_count(), 20);
    assert_eq!(
        meshes.indices.len(),
        3 * 6,
        "the longest run is four points"
    );
    // A limit too small to hold a segment is a named error.
    assert!(build_with_limit(&map, &always(BorderClass::Country), &dry, &heights, 1.5, 3).is_err());
}

#[test]
fn the_camera_fades_match_the_defines() {
    assert_eq!(class_alpha(BorderClass::Province, 0.0, FADES), 1.0);
    assert_eq!(class_alpha(BorderClass::Province, 200.0, FADES), 1.0);
    assert_eq!(class_alpha(BorderClass::Province, 250.0, FADES), 0.5);
    assert_eq!(class_alpha(BorderClass::Province, 300.0, FADES), 0.0);
    assert_eq!(class_alpha(BorderClass::Province, 1000.0, FADES), 0.0);
    assert_eq!(class_alpha(BorderClass::State, 300.0, FADES), 1.0);
    assert_eq!(class_alpha(BorderClass::State, 400.0, FADES), 1.0);
    assert_eq!(class_alpha(BorderClass::State, 450.0, FADES), 0.5);
    assert_eq!(class_alpha(BorderClass::State, 500.0, FADES), 0.0);
    for class in [
        BorderClass::Country,
        BorderClass::Impassable,
        BorderClass::Sea,
        BorderClass::SeaRegion,
    ] {
        assert_eq!(class_alpha(class, 0.0, FADES), 1.0);
        assert_eq!(class_alpha(class, 3000.0, FADES), 1.0);
    }
    assert_eq!(class_alpha(BorderClass::Province, f32::NAN, FADES), 1.0);
}

#[test]
fn the_bucket_order_is_the_engines() {
    let names: Vec<&str> = BorderClass::ALL
        .iter()
        .map(|class| class.texture_name())
        .collect();
    assert_eq!(
        names,
        vec![
            "province",
            "state",
            "country",
            "impassable",
            "sea",
            "sea_region"
        ]
    );
    for (bucket, class) in BorderClass::ALL.iter().enumerate() {
        assert_eq!(class.bucket(), bucket);
    }
}

#[test]
fn world_and_pixel_space_round_trip() {
    assert_eq!(world_from_pixel(2048, 10.0, 0.0), (10.0, 2047.0));
    assert_eq!(world_from_pixel(2048, 10.0, 2047.0), (10.0, 0.0));
    assert_eq!(world_from_pixel(4096, 10.0, 0.0), (10.0, 4095.0));
    let (x, z) = world_from_pixel(2048, 1234.5, 678.25);
    assert_eq!(pixel_from_world(2048, x, z), (1234.5, 678.25));
}

#[test]
fn bad_input_is_a_named_error_rather_than_a_panic() {
    let ids = vec![1_u32; 16];
    let map = ProvinceMap {
        width: 4,
        height: 4,
        ids: &ids,
    };
    let values = flat(2, 2);
    let small = HeightField {
        width: 2,
        height: 2,
        values: &values,
    };
    assert!(
        build(&map, &always(BorderClass::Province), &dry, &small, 1.5)
            .unwrap_err()
            .to_string()
            .contains("height field is 2 x 2")
    );
    let ragged = HeightField {
        width: 4,
        height: 4,
        values: &values,
    };
    assert!(
        build(&map, &always(BorderClass::Province), &dry, &ragged, 1.5)
            .unwrap_err()
            .to_string()
            .contains("4 samples for 4 x 4")
    );
    let short = ProvinceMap {
        width: 8,
        height: 8,
        ids: &ids,
    };
    assert!(trace(&short, &always(BorderClass::Province), &dry).is_err());
}
