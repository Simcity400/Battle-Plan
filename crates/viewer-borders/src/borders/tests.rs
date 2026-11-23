use std::cell::RefCell;
const FADES: [[f32; 2]; 2] = [[200.0, 300.0], [400.0, 500.0]];
use super::*;

fn flat(width: u32, height: u32) -> Vec<u8> {
    vec![95; (width * height) as usize]
}

fn always(class: BorderClass) -> impl Fn(u32, u32) -> Option<BorderClass> {
    move |_, _| Some(class)
}

fn dry(_province: u32) -> bool {
    false
}

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
    let (_, z) = world_from_pixel(8, 0.0, 3.5);
    for (index, point) in polyline.points.iter().enumerate() {
        assert_eq!(*point, [index as f32, z]);
    }
    assert_eq!(z, 3.5);
    assert!(traced.one_pixel_provinces.is_empty());
}

#[test]
fn an_island_traces_a_closed_loop() {
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
    for window in polyline.points[..8].windows(2) {
        let step = (window[1][0] - window[0][0]).hypot(window[1][1] - window[0][1]);
        assert!(step <= 1.001, "step {step}");
    }
}

#[test]
fn the_x_seam_is_a_boundary_and_cuts_the_chain() {
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
    assert_eq!(columns, vec![3.5, 7.5]);
    for polyline in &traced.polylines {
        assert_eq!(polyline.points.len(), 8);
        assert!(!polyline.closed);
    }
}

#[test]
fn chains_that_meet_at_a_triple_point_share_its_corner() {
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
    assert_eq!(smoothed[0], points[0]);
    assert_eq!(smoothed[3], points[3]);
    assert_eq!(smoothed[1], [1.0, 4.0 / 3.0]);
    assert_eq!(smoothed[2], [2.0, 4.0 / 3.0]);
    let straight = [[0.0, 7.0], [1.0, 7.0], [2.0, 7.0], [3.0, 7.0]];
    assert_eq!(smooth(&straight, false), straight.to_vec());
    assert_eq!(smooth(&[[1.0, 2.0]], false), vec![[1.0, 2.0]]);
    assert_eq!(
        smooth(&[[1.0, 2.0], [3.0, 4.0]], true),
        vec![[1.0, 2.0], [3.0, 4.0]]
    );
}

#[test]
fn a_water_to_water_chain_takes_the_wide_two_pass_filter() {
    let mut points: Vec<[f32; 2]> = (0..17).map(|i| [i as f32, 0.0]).collect();
    points[8][1] = 9.0;
    let narrow = smooth(&points, false);
    let wide = smooth(&points, true);
    assert_eq!(narrow[8], [8.0, 3.0]);
    assert!(wide[8][1] < 1.0, "{:?}", wide[8]);
    assert!(wide[8][1] > 0.0);
    assert_eq!(narrow[0], points[0]);
    assert_eq!(wide[0], points[0]);
    assert_eq!(wide[16], points[16]);
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
        assert_eq!(first.uv, [0, point as i16]);
        assert_eq!(second.uv, [1, point as i16]);
        assert!((first.position[2] - (z - half)).abs() < 1e-5, "{first:?}");
        assert!((second.position[2] - (z + half)).abs() < 1e-5, "{second:?}");
        assert_eq!(first.position[0], second.position[0]);
        assert_eq!(first.position[1], WATER_HEIGHT);
    }
    assert_eq!(vertices[0].position[0], 0.0);
    assert_eq!((draw.coarse_base_vertex, draw.coarse_vertex_count), (0, 4));
    assert_eq!(mesh.coarse_chunks[0], [vertices[0], vertices[1], vertices[14], vertices[15]]);
    assert_eq!(meshes.indices[..6], [0, 2, 1, 2, 3, 1]);
    assert_eq!(meshes.indices[6..12], [2, 4, 3, 4, 5, 3]);
    assert_eq!(meshes.indices.len(), 7 * 6);
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
    assert_eq!(vertices[1].position[1], 17.0);
    assert_eq!(vertices[0].position[1], WATER_HEIGHT);
    assert_eq!(heights.height_at_world(0.0, 4.0), 19.5);
    assert_eq!(heights.height_at_world(0.0, 3.0), WATER_HEIGHT);
    assert_eq!(heights.height_at_world(-1.0, 4.0), 19.5);
    assert_eq!(heights.height_at_world(1e9, -1e9), WATER_HEIGHT);
}

#[test]
fn the_classifier_decides_which_boundaries_survive() {
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
    let seen = RefCell::new(Vec::new());
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
    assert!(components.is_junction([1, 1]));
    assert_ne!(components.components[0], components.components[2]);
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
    let first = mesh.chunks[0][6];
    let second = mesh.chunks[1][0];
    assert_eq!(first.position, second.position);
    assert_eq!(first.uv, [0, 3]);
    assert_eq!(second.uv, [0, 3]);
    assert_eq!(mesh.chunks[2][0].uv, [0, 6]);
    assert_eq!(
        mesh.draws.iter().map(|draw| (draw.chunk, draw.coarse_base_vertex, draw.coarse_vertex_count)).collect::<Vec<_>>(),
        [(0, 0, 4), (1, 0, 4), (2, 0, 4)]
    );
    assert_eq!(mesh.coarse_chunks.len(), 3);
    assert_eq!(mesh.coarse_chunks[0][2].position, mesh.coarse_chunks[1][0].position);
    let counters: Vec<i16> = mesh.coarse_chunks.iter().flat_map(|chunk| chunk.iter().step_by(2).map(|vertex| vertex.uv[1])).collect();
    assert_eq!(counters, [0, 3, 3, 6, 6, 7]);
    assert_eq!(mesh.vertex_count(), 20);
    assert_eq!(
        meshes.indices.len(),
        3 * 6,
        "the longest run is four points"
    );
    assert!(build_with_limit(&map, &always(BorderClass::Country), &dry, &heights, 1.5, 3).is_err());
}

#[test]
fn the_coarse_level_keeps_the_strip_within_the_tolerance_at_its_width() {
    let (width, height) = (16_u32, 8_u32);
    let ids: Vec<u32> = (0..height)
        .flat_map(|y| (0..width).map(move |x| if y < if (6..10).contains(&x) { 2 } else { 4 } { 1 } else { 2 }))
        .collect();
    let mut values = flat(width, height);
    for y in 0..height {
        for x in 12..14 {
            values[(y * width + x) as usize] = 135;
        }
    }
    let map = ProvinceMap { width, height, ids: &ids };
    let heights = HeightField { width, height, values: &values };
    let meshes = build(&map, &always(BorderClass::Country), &dry, &heights, 1.5).unwrap();
    let mesh = meshes.mesh(BorderClass::Country);
    assert_eq!(mesh.draws.len(), 1);
    let draw = mesh.draws[0];
    let fine = &mesh.chunks[0];
    let coarse = &mesh.coarse_chunks[0][draw.coarse_base_vertex as usize..][..draw.coarse_vertex_count as usize];
    assert!(4 < coarse.len() && coarse.len() < fine.len(), "{} of {}", coarse.len(), fine.len());
    let centre = |pair: &[Vertex]| {
        [(pair[0].position[0] + pair[1].position[0]) * 0.5, (pair[0].position[2] + pair[1].position[2]) * 0.5]
    };
    let line: Vec<[f32; 2]> = coarse.chunks(2).map(centre).collect();
    for pair in coarse.chunks(2) {
        let across = (pair[0].position[0] - pair[1].position[0]).hypot(pair[0].position[2] - pair[1].position[2]);
        assert!((across - 1.5).abs() < 1e-4, "{pair:?}");
        assert_eq!([pair[0].uv[0], pair[1].uv[0]], [0, 1]);
        assert_eq!(pair[0].uv[1], pair[1].uv[1]);
    }
    let counters: Vec<i16> = coarse.iter().step_by(2).map(|vertex| vertex.uv[1]).collect();
    assert!(counters.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!((counters[0], *counters.last().unwrap()), (0, fine[fine.len() - 1].uv[1]));
    for pair in fine.chunks(2) {
        let point = centre(pair);
        let off = line.windows(2).map(|segment| line_lod::offset(segment[0], segment[1], point).0).fold(f32::INFINITY, f32::min);
        assert!(off <= line_lod::TOLERANCE + 1e-4, "{point:?} is {off} off the coarse line");
    }
    assert!(line.iter().any(|point| (11.0..15.0).contains(&point[0])), "{line:?}");
    let triangles = |strip: &[Vertex]| -> Vec<[[f32; 3]; 3]> {
        let mut indices = Vec::new();
        super::super::border_pass::run_indices(0, strip.len() as u32 / 2, &mut indices);
        indices.chunks(3).map(|triangle| std::array::from_fn(|corner| strip[triangle[corner] as usize].position)).collect()
    };
    let zone = super::super::demilitarized::ZonePlan::of_draws(
        &meshes,
        vec![(BorderClass::Country, 0)],
        |_, _| true,
        |_, _| BorderClass::Country,
    );
    assert_eq!(zone.triangles(true), triangles(coarse));
    assert_eq!(zone.triangles(false), triangles(fine));
}

#[test]
fn the_camera_fades_match_the_defines() {
    assert_eq!(class_alpha(BorderClass::Province, 0.0, FADES, true), 1.0);
    assert_eq!(class_alpha(BorderClass::Province, 200.0, FADES, true), 1.0);
    assert_eq!(class_alpha(BorderClass::Province, 250.0, FADES, true), 0.5);
    assert_eq!(class_alpha(BorderClass::Province, 300.0, FADES, true), 0.0);
    assert_eq!(class_alpha(BorderClass::Province, 1000.0, FADES, true), 0.0);
    assert_eq!(class_alpha(BorderClass::State, 300.0, FADES, true), 1.0);
    assert_eq!(class_alpha(BorderClass::State, 400.0, FADES, true), 1.0);
    assert_eq!(class_alpha(BorderClass::State, 450.0, FADES, true), 0.5);
    assert_eq!(class_alpha(BorderClass::State, 500.0, FADES, true), 0.0);
    for class in [
        BorderClass::Country,
        BorderClass::Impassable,
        BorderClass::Sea,
        BorderClass::SeaRegion,
    ] {
        assert_eq!(class_alpha(class, 0.0, FADES, true), 1.0);
        assert_eq!(class_alpha(class, 3000.0, FADES, true), 1.0);
    }
    assert_eq!(
        class_alpha(BorderClass::Province, f32::NAN, FADES, true),
        1.0
    );
}

#[test]
fn a_map_mode_without_province_borders_gives_them_no_alpha_at_any_height() {
    use viewer_map_modes::map_mode::{MapMode, Stock, visibility};
    for stock in [Stock::Resources, Stock::Infrastructure, Stock::Manpower] {
        let shown = visibility::of(&MapMode::Stock(stock)).province_borders;
        assert!(!shown, "{stock:?}");
        for height in [0.0, 250.0, f32::NAN] {
            assert_eq!(
                class_alpha(BorderClass::Province, height, FADES, shown),
                0.0
            );
        }
        assert_eq!(class_alpha(BorderClass::State, 0.0, FADES, shown), 1.0);
        assert_eq!(class_alpha(BorderClass::Country, 0.0, FADES, shown), 1.0);
    }
    let political = visibility::of(&MapMode::default()).province_borders;
    assert_eq!(
        class_alpha(BorderClass::Province, 0.0, FADES, political),
        1.0
    );
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

#[test]
fn political_fill_uses_controller_and_borders_use_owner() {
    use viewer_map_modes::political::{regions, test_support::{map, occupied}};
    use content_reader_map::map::ProvinceKind;
    use viewer_scene_base::gradient::NO_REGION;
    let map = map();
    let frame = occupied();
    let control = regions(&map, &frame).unwrap();
    assert_eq!(control.country_of[1], 1);
    assert_eq!(control.country_of[3], 2);
    assert_ne!(control.country_of[1], control.country_of[3]);
    assert_eq!(control.owner_of[3], 1);
    assert_eq!(
        crate::border_pass::land_class(&control, 1, 3),
        crate::borders::BorderClass::State
    );
    assert_eq!(control.country_of[5], NO_REGION);
    assert_eq!(control.state_of[5], NO_REGION);
    assert!(control.warnings.is_empty());

    let mut lakes = map;
    lakes.definitions.get_mut(&3).unwrap().kind = ProvinceKind::Lake;
    lakes.definitions.get_mut(&5).unwrap().kind = ProvinceKind::Lake;
    let lake_control = regions(&lakes, &frame).unwrap();
    assert_eq!(lake_control.country_of, control.country_of);
    assert_eq!(lake_control.owner_of, control.owner_of);
    assert!(lake_control.warnings.is_empty());
}
