use super::*;

fn test_defines() -> RiverDefines {
    RiverDefines {
        fade_from: 20.0,
        fade_to: 3.0,
    }
}

#[test]
fn flow_runs_preserve_native_group_parity_and_index_order() {
    let heights = HeightField {
        width: 8,
        height: 8,
        values: &[120; 64],
    };
    let mut river = River {
        points: vec![[2.0, 2.0], [3.0, 2.0], [4.0, 2.0]],
        widths: vec![3; 3],
        seed: Seed::Source,
        seed_pixel: [2, 5],
    };
    let mut mesh = RiverMesh::default();
    for seed in [
        Seed::Source,
        Seed::Junction(1),
        Seed::Junction(1),
        Seed::Junction(2),
    ] {
        river.seed = seed;
        ribbon(&river, &heights, test_defines(), &mut mesh);
    }
    assert_eq!(
        (&mesh.coarse_runs, &mesh.coarse_indices, mesh.vertices.len()),
        (&mesh.runs, &mesh.indices, 4 * 3 * 2),
        "three points with a fade between them keep every point, so the coarse level draws the fine quads"
    );
    assert_eq!(
        mesh.runs,
        vec![
            super::super::river_shader::Run {
                indices: 0..12,
                direction: -1.0
            },
            super::super::river_shader::Run {
                indices: 12..36,
                direction: 1.0
            },
            super::super::river_shader::Run {
                indices: 36..48,
                direction: -1.0
            },
        ]
    );
}

fn image(rows: &[&str]) -> (u32, u32, Vec<u8>) {
    let height = rows.len() as u32;
    let width = rows[0].len() as u32;
    let mut indices = Vec::new();
    for row in rows {
        assert_eq!(row.len() as u32, width, "every row must be the same length");
        for character in row.chars() {
            indices.push(match character {
                '.' => 255,
                'S' => SOURCE_INDEX,
                'j' => 1,
                'J' => 2,
                'a' => 10,
                'b' => 11,
                digit => digit.to_digit(10).expect("river index") as u8,
            });
        }
    }
    (width, height, indices)
}

fn traced(rows: &[&str]) -> Trace {
    let (width, height, indices) = image(rows);
    trace(&RiverImage {
        width,
        height,
        indices: &indices,
    })
    .unwrap()
}

#[test]
fn a_straight_river_walks_from_its_source_to_its_last_body_pixel() {
    let trace = traced(&[".......", ".S33445", "......."]);
    assert_eq!(trace.rivers.len(), 1);
    assert_eq!(trace.too_small, []);
    assert_eq!(trace.unreachable, 0);
    let river = &trace.rivers[0];
    assert_eq!(river.seed, Seed::Source);
    assert_eq!(river.widths, [0, 3, 3, 4, 4, 5]);
    assert_eq!(river.points[0], [1.0, 1.0]);
    assert_eq!(river.points[5], [6.0, 1.0]);
}

#[test]
fn a_two_pixel_river_is_reported_too_small_and_dropped() {
    let trace = traced(&[".....", ".S3..", "....."]);
    assert_eq!(trace.rivers, []);
    assert_eq!(
        trace.too_small,
        [Reported {
            x: 1,
            y: 1,
            points: 2
        }]
    );
    assert_eq!(trace.unreachable, 0);
}

#[test]
fn a_junction_marker_stitches_its_tributary_onto_the_stream_it_joins() {
    let rows = [
        "..........",
        "...4......",
        "...j......",
        ".S3366777.",
        "..........",
    ];
    let trace = traced(&rows);
    assert_eq!(trace.junctions, 1);
    assert_eq!(trace.junctions_used, 1);
    assert_eq!(trace.rivers.len(), 2);
    assert_eq!(trace.unreachable, 0);
    let joined = &trace.rivers[1];
    assert_eq!(joined.seed, Seed::Junction(1));
    assert_eq!(joined.seed_pixel, [3, 2]);
    assert_eq!(joined.points, [[3.0, 1.0], [3.0, 2.0], [3.0, 3.0]]);
    assert_eq!(joined.widths, [3, 3, 4]);

    let unmarked = traced(&[rows[0], rows[1], "..........", rows[3], rows[4]]);
    assert_eq!(unmarked.rivers.len(), 1);
    assert_eq!(unmarked.unreachable, 1);
}

#[test]
fn a_loop_is_reported_circular_and_stops_the_walk() {
    let trace = traced(&["........", ".S33333.", "....3.3.", "....333.", "........"]);
    assert_eq!(trace.rivers.len(), 1);
    assert_eq!(trace.circular.len(), 1);
    assert_eq!(trace.unreachable, 0);
    assert_eq!(trace.rivers[0].points.len(), 11);
}

#[test]
fn body_pixels_no_walk_reaches_are_counted() {
    let trace = traced(&[".....", ".333.", "....."]);
    assert_eq!(trace.rivers, []);
    assert_eq!(trace.unreachable, 3);
}

#[test]
fn the_walk_is_four_connected_and_clamps_at_the_seam() {
    let diagonal = traced(&["......", ".S3...", "...3..", "......"]);
    assert_eq!(diagonal.rivers, []);
    assert_eq!(diagonal.too_small.len(), 1);
    assert_eq!(diagonal.unreachable, 1);
    let seam = traced(&["......", "33S..3", "......"]);
    assert_eq!(seam.rivers.len(), 1);
    assert_eq!(seam.rivers[0].points.len(), 3);
    assert_eq!(seam.unreachable, 1);
}

#[test]
fn smoothing_updates_forward_and_preserves_two_endpoint_points() {
    let mut points = [
        [0.0, 0.0],
        [1.0, 0.0],
        [2.0, 5.0],
        [3.0, 0.0],
        [4.0, 0.0],
        [5.0, 0.0],
        [6.0, 0.0],
    ];
    smooth_river(&mut points);
    assert_eq!(points[0..2], [[0.0, 0.0], [1.0, 0.0]]);
    assert_eq!(points[2], [2.0, 1.0]);
    assert_eq!(points[3], [3.0, 0.2]);
    assert_eq!(points[4], [4.0, 0.24000001]);
    assert_eq!(points[5..], [[5.0, 0.0], [6.0, 0.0]]);
}

#[test]
fn junctions_taper_and_fade_at_the_far_end_only() {
    let river = River {
        points: (0..61).map(|i| [i as f32, 2046.0]).collect(),
        widths: vec![6; 61],
        seed: Seed::Junction(1),
        seed_pixel: [0, 1],
    };
    let values = flat(64, 3);
    let mut mesh = RiverMesh::default();
    ribbon(
        &river,
        &HeightField {
            width: 64,
            height: 3,
            values: &values,
        },
        test_defines(),
        &mut mesh,
    );
    assert_eq!(mesh.vertices[0].position[3], 1.0);
    assert_eq!(mesh.vertices[120].position[3], 0.0);
    for i in [118, 120] {
        assert_eq!(mesh.vertices[i].position, mesh.vertices[i + 1].position);
    }
    let width = (mesh.vertices[116].position[2] - mesh.vertices[117].position[2]).abs();
    assert!((width - 0.1).abs() < 0.0002);
    assert_eq!(
        mesh.vertices[118].uv_tangent[2..],
        mesh.vertices[120].uv_tangent[2..]
    );
}

#[test]
fn river_height_bilinearly_filters_bytes_before_scaling() {
    let mut values = vec![0; 2 * 2048];
    values[..4].copy_from_slice(&[100, 200, 80, 120]);
    let field = HeightField {
        width: 2,
        height: 2048,
        values: &values,
    };
    assert_eq!(river_height(&field, 0.25, 2046.5), 10.75);
    assert_eq!(river_height(&field, 0.0, 2046.0), 8.0);
}

#[test]
fn junction_partition_keeps_every_marker_and_orders_both_keys() {
    for count in [0, 1, 32, 33, 41, 42, 264, 1024] {
        let mut items: Vec<_> = (0..count).map(|i| (i, 0, 1 + (i % 7 == 0) as u8)).collect();
        sort_junctions(&mut items);
        assert!(items.windows(2).all(|p| p[0].2 <= p[1].2));
        items.sort_by_key(|p| p.0);
        assert!(items.iter().enumerate().all(|(i, p)| p.0 as usize == i));
    }
}

fn flat(width: u32, height: u32) -> Vec<u8> {
    vec![120; (width * height) as usize]
}

#[test]
fn a_ribbon_carries_the_tangent_the_uv_and_the_end_fade() {
    let mut row = String::from(".S");
    row.push_str(&"6".repeat(60));
    row.push('.');
    let rows = [".".repeat(row.len()), row.clone(), ".".repeat(row.len())];
    let (width, height, indices) = image(&rows.iter().map(String::as_str).collect::<Vec<_>>());
    let values = flat(width, height);
    let (mesh, trace) = build(
        &RiverImage {
            width,
            height,
            indices: &indices,
        },
        &HeightField {
            width,
            height,
            values: &values,
        },
        test_defines(),
    )
    .unwrap();
    assert_eq!(trace.rivers.len(), 1);
    assert_eq!(mesh.ribbons, 1);
    let points = trace.rivers[0].points.len();
    let (fine, coarse) = mesh.vertices.split_at(points * 2);
    assert_eq!(mesh.indices.len(), (points - 1) * 6);

    let (left, right) = (fine[40], fine[41]);
    assert_eq!(left.uv_tangent[0], 0.0);
    assert_eq!(right.uv_tangent[0], 1.0);
    assert_eq!(left.uv_tangent[1], right.uv_tangent[1]);
    assert!((left.uv_tangent[2] - 1.0).abs() < 1e-4);
    assert!(left.uv_tangent[3].abs() < 1e-4);
    let across = (left.position[2] - right.position[2]).abs();
    assert!(
        (across - width_for_index(6)).abs() < 1e-3,
        "ribbon is {across} wide"
    );
    let step = fine[42].uv_tangent[1] - left.uv_tangent[1];
    assert_eq!(step, 1.0);
    assert_eq!(left.uv_tangent[1], 20.0);

    assert_eq!(fine[0].position[3], 0.0);
    assert_eq!(fine.last().unwrap().position[3], 1.0);
    assert_eq!(fine[points].position[3], 1.0);
    assert_eq!(fine[0].position[1], 12.0);

    assert!(coarse.len() * 4 < fine.len(), "{} of {}", coarse.len(), fine.len());
    assert_eq!(mesh.coarse_indices.len(), (coarse.len() / 2 - 1) * 6);
    assert!(mesh.coarse_indices.iter().all(|&index| index as usize >= fine.len()));
    assert_eq!(mesh.coarse_runs.len(), 1);
    assert_eq!(mesh.coarse_runs[0].direction, mesh.runs[0].direction);
    let fine_of = |vertex: &RiverVertex| fine[vertex.uv_tangent[1] as usize * 2 + vertex.uv_tangent[0] as usize];
    for vertex in coarse {
        assert_eq!(*vertex, fine_of(vertex));
    }
    let kept: Vec<usize> = coarse.iter().step_by(2).map(|vertex| vertex.uv_tangent[1] as usize).collect();
    assert_eq!((kept[0], *kept.last().unwrap()), (0, points - 1));
    assert!(coarse_rail_error(fine, coarse) <= viewer_scene_base::line_lod::TOLERANCE + 1e-4);
    for segment in kept.windows(2) {
        let (a, b) = (segment[0], segment[1]);
        for index in a..=b {
            let t = (index - a) as f32 / (b - a) as f32;
            for side in 0..2 {
                let (start, end, fine) = (fine[a * 2 + side], fine[b * 2 + side], fine[index * 2 + side]);
                let alpha = start.position[3] + (end.position[3] - start.position[3]) * t;
                assert!((fine.position[3] - alpha).abs() <= COARSE_ALPHA_TOLERANCE + 1e-4, "point {index}");
            }
        }
    }
}

fn coarse_rail_error(fine: &[RiverVertex], coarse: &[RiverVertex]) -> f32 {
    let xz = |vertex: &RiverVertex| [vertex.position[0], vertex.position[2]];
    let kept: Vec<usize> = coarse.iter().step_by(2).map(|vertex| vertex.uv_tangent[1] as usize).collect();
    let mut worst = 0.0_f32;
    for (at, &index) in kept.iter().enumerate() {
        for side in 0..2 {
            let here = xz(&coarse[at * 2 + side]);
            worst = worst.max(distance(here, xz(&fine[index * 2 + side])));
            if let Some(&next) = kept.get(at + 1) {
                let there = xz(&coarse[(at + 1) * 2 + side]);
                for dropped in index + 1..next {
                    worst = worst.max(viewer_scene_base::line_lod::offset(here, there, xz(&fine[dropped * 2 + side])).0);
                }
            }
        }
    }
    worst
}

#[test]
fn a_wide_river_keeps_its_banks_within_the_tolerance_round_a_small_sharp_bend() {
    let mut points: Vec<[f32; 2]> = (0..400).map(|x| [x as f32, 8.0]).collect();
    for point in &mut points[200..203] {
        point[1] = 9.0;
    }
    let river = River {
        points,
        widths: vec![99; 400],
        seed: Seed::Source,
        seed_pixel: [0, 8],
    };
    let values = flat(8, 8);
    let mut mesh = RiverMesh::default();
    ribbon(
        &river,
        &HeightField {
            width: 8,
            height: 8,
            values: &values,
        },
        test_defines(),
        &mut mesh,
    );
    let (fine, coarse) = mesh.vertices.split_at(800);
    let error = coarse_rail_error(fine, coarse);
    assert!(error <= viewer_scene_base::line_lod::TOLERANCE + 1e-4, "a coarse bank stands {error} off");
    assert!(coarse.len() * 10 < fine.len(), "{} of {}", coarse.len(), fine.len());
}

#[test]
fn a_bend_in_the_fade_keeps_the_fade_within_its_tolerance_as_well_as_the_banks() {
    for (bend, lift, width) in (1..40).flat_map(|bend| [0.6_f32, 1.0, 2.0, 3.5].into_iter().flat_map(move |lift| [24_u8, 60, 99].map(|width| (bend, lift, width)))) {
        let mut points: Vec<[f32; 2]> = (0..60).map(|x| [x as f32, 8.0]).collect();
        for point in &mut points[bend..bend + 2] {
            point[1] = 8.0 + lift;
        }
        let river = River {
            points,
            widths: vec![width; 60],
            seed: Seed::Source,
            seed_pixel: [0, 8],
        };
        let values = flat(8, 8);
        let mut mesh = RiverMesh::default();
        ribbon(
            &river,
            &HeightField {
                width: 8,
                height: 8,
                values: &values,
            },
            test_defines(),
            &mut mesh,
        );
        let (fine, coarse) = mesh.vertices.split_at(120);
        assert!(coarse_rail_error(fine, coarse) <= viewer_scene_base::line_lod::TOLERANCE + 1e-4, "bend at {bend}, lift {lift}, width {width}");
        let kept: Vec<usize> = coarse.iter().step_by(2).map(|vertex| vertex.uv_tangent[1] as usize).collect();
        let xz = |vertex: &RiverVertex| [vertex.position[0], vertex.position[2]];
        for (at, segment) in kept.windows(2).enumerate() {
            let (a, b) = (segment[0], segment[1]);
            for side in 0..2 {
                let (start, end) = (&coarse[at * 2 + side], &coarse[(at + 1) * 2 + side]);
                for index in a..=b {
                    let fine = &fine[index * 2 + side];
                    let (_, t) = viewer_scene_base::line_lod::offset(xz(start), xz(end), xz(fine));
                    let alpha = start.position[3] + (end.position[3] - start.position[3]) * t;
                    assert!(
                        (fine.position[3] - alpha).abs() <= COARSE_ALPHA_TOLERANCE + 1e-4,
                        "bend at {bend}, lift {lift}, width {width}: rail {side} point {index} fades {} against {alpha}",
                        fine.position[3]
                    );
                }
            }
        }
    }
}

#[test]
fn a_ribbon_under_the_sea_keeps_its_sampled_height() {
    let rows = [".......", ".S33333", "......."];
    let (width, height, indices) = image(&rows);
    let values = vec![10; (width * height) as usize];
    let (mesh, _) = build(
        &RiverImage {
            width,
            height,
            indices: &indices,
        },
        &HeightField {
            width,
            height,
            values: &values,
        },
        test_defines(),
    )
    .unwrap();
    assert!(!mesh.is_empty());
    for vertex in &mesh.vertices {
        assert_eq!(vertex.position[1], 1.0);
    }
}

#[test]
fn a_mismatched_height_field_is_refused() {
    let (width, height, indices) = image(&[".....", ".S33.", "....."]);
    let values = flat(width, height + 1);
    assert!(
        build(
            &RiverImage {
                width,
                height,
                indices: &indices
            },
            &HeightField {
                width,
                height: height + 1,
                values: &values
            },
            test_defines(),
        )
        .is_err()
    );
    assert!(
        trace(&RiverImage {
            width,
            height,
            indices: &indices[..3]
        })
        .is_err()
    );
    assert!(
        trace(&RiverImage {
            width: 0,
            height: 0,
            indices: &[]
        })
        .is_err()
    );
}

#[test]
fn river_fade_resolves_non_default_defines_and_refuses_missing_or_invalid_ones() {
    let load = |body: &str| {
        let defines = Defines::new().unwrap();
        defines
            .load(
                "common/defines/00_graphics.lua",
                format!("NDefines = {{ NGraphics = {{ {body} }} }}").as_bytes(),
            )
            .unwrap();
        RiverDefines::load(&defines)
    };
    let custom = load("RIVER_FADE_FROM = 40.0, RIVER_FADE_TO = 10.0").unwrap();
    assert_eq!(
        custom,
        RiverDefines {
            fade_from: 40.0,
            fade_to: 10.0,
        }
    );
    let mut row = String::from(".S");
    row.push_str(&"6".repeat(60));
    row.push('.');
    let rows = [".".repeat(row.len()), row.clone(), ".".repeat(row.len())];
    let (width, height, indices) = image(&rows.iter().map(String::as_str).collect::<Vec<_>>());
    let values = flat(width, height);
    let (default_mesh, _) = build(
        &RiverImage {
            width,
            height,
            indices: &indices,
        },
        &HeightField {
            width,
            height,
            values: &values,
        },
        test_defines(),
    )
    .unwrap();
    let (custom_mesh, _) = build(
        &RiverImage {
            width,
            height,
            indices: &indices,
        },
        &HeightField {
            width,
            height,
            values: &values,
        },
        custom,
    )
    .unwrap();
    assert_eq!(default_mesh.vertices[15 * 2].position[3], 1.0);
    assert!(
        (custom_mesh.vertices[15 * 2].position[3] - 0.7).abs() < 1e-6,
        "{}",
        custom_mesh.vertices[15 * 2].position[3]
    );
    assert_eq!(custom_mesh.vertices[4 * 2].position[3], 0.0);

    let missing_from = load("RIVER_FADE_TO = 3.0").unwrap_err().to_string();
    assert!(missing_from.contains("RIVER_FADE_FROM"), "{missing_from}");
    let missing_to = load("RIVER_FADE_FROM = 20.0").unwrap_err().to_string();
    assert!(missing_to.contains("RIVER_FADE_TO"), "{missing_to}");
    let inverted = load("RIVER_FADE_FROM = 5.0, RIVER_FADE_TO = 10.0")
        .unwrap_err()
        .to_string();
    assert!(
        inverted.contains("RIVER_FADE_FROM") && inverted.contains("RIVER_FADE_TO"),
        "{inverted}"
    );
}
