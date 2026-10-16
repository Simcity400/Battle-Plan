use super::*;

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
        ribbon(&river, &heights, &mut mesh);
    }
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

/// A bitmap from rows of characters: `.` land, `S` source, `j`/`J` junction 1 and 2, digits and
/// `a`/`b` the body indices 0 to 11.
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
    // The source pixel plus five body pixels, carrying each body pixel's own index.
    assert_eq!(river.widths, [0, 3, 3, 4, 4, 5]);
    // World z is `height - 1 - row`, so row 1 of three is z 1.
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
    // The body pixel was walked even though the river was dropped, so it is not unreachable.
    assert_eq!(trace.unreachable, 0);
}

#[test]
fn a_junction_marker_stitches_its_tributary_onto_the_stream_it_joins() {
    // A stream running east with a tributary reaching it from the north. The tributary has no
    // source of its own, so only the junction marker can reach it.
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
    // The junction walk runs after every source, and it starts on the stream it joins rather
    // than on the marker, so the tributary's ribbon reaches the water it flows into.
    let joined = &trace.rivers[1];
    assert_eq!(joined.seed, Seed::Junction(1));
    assert_eq!(joined.seed_pixel, [3, 2]);
    // World z is `height - 1 - row`: the stream pixel under the marker, then the marker, then north.
    assert_eq!(joined.points, [[3.0, 1.0], [3.0, 2.0], [3.0, 3.0]]);
    // The stitch point and the marker both carry the stream's own index, so the ribbon is full
    // width where the two meet instead of pinching to the marker's index 1.
    assert_eq!(joined.widths, [3, 3, 4]);

    // Without the marker nothing reaches that tributary, and the engine draws nothing there.
    let unmarked = traced(&[rows[0], rows[1], "..........", rows[3], rows[4]]);
    assert_eq!(unmarked.rivers.len(), 1);
    assert_eq!(unmarked.unreachable, 1);
}

#[test]
fn a_loop_is_reported_circular_and_stops_the_walk() {
    // A stem into a ring: the walk goes round the ring and meets its own first ring pixel.
    let trace = traced(&["........", ".S33333.", "....3.3.", "....333.", "........"]);
    assert_eq!(trace.rivers.len(), 1);
    assert_eq!(trace.circular.len(), 1);
    // Every body pixel of the ring is walked exactly once before the walk closes on itself.
    assert_eq!(trace.unreachable, 0);
    assert_eq!(trace.rivers[0].points.len(), 11);
}

#[test]
fn body_pixels_no_walk_reaches_are_counted() {
    // A river with no source and no junction marker: the engine never traces it.
    let trace = traced(&[".....", ".333.", "....."]);
    assert_eq!(trace.rivers, []);
    assert_eq!(trace.unreachable, 3);
}

#[test]
fn the_walk_is_four_connected_and_clamps_at_the_seam() {
    // A diagonal step is not a step: the walk stops at the first body pixel.
    let diagonal = traced(&["......", ".S3...", "...3..", "......"]);
    assert_eq!(diagonal.rivers, []);
    assert_eq!(diagonal.too_small.len(), 1);
    assert_eq!(diagonal.unreachable, 1);
    // The world does not wrap here, so a river touching the left edge does not continue on the
    // right one.
    let seam = traced(&["......", "33S..3", "......"]);
    assert_eq!(seam.rivers.len(), 1);
    assert_eq!(seam.rivers[0].points.len(), 3);
    assert_eq!(seam.unreachable, 1);
}

#[test]
fn the_body_range_is_the_engines_and_not_the_shipped_palette() {
    assert!(!is_body(2));
    assert!(is_body(3));
    assert!(is_body(11));
    assert!(is_body(99));
    assert!(!is_body(100));
    assert!(!is_body(254));
    assert!(!is_body(255));
}

#[test]
fn palette_indices_determine_ribbon_widths() {
    // Ribbon width depends on palette index independently of colour and bank falloff.
    for (index, expected) in [
        (3u8, 1.11_f32),
        (4, 1.28),
        (6, 1.62),
        (7, 1.79),
        (9, 2.13),
        (10, 2.30),
        (11, 2.47),
    ] {
        assert!(
            (width_for_index(index) - expected).abs() < 0.0005,
            "index {index}: {} against {expected}",
            width_for_index(index)
        );
    }
    // Source and junction markers remain narrower than body classes.
    assert!(width_for_index(0) < width_for_index(3));
    assert!(width_for_index(11) > width_for_index(10));
    // Every index the engine accepts has a positive, finite width.
    for index in 0..=BODY_MAX_INDEX {
        assert!(width_for_index(index) > 0.0 && width_for_index(index).is_finite());
    }
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
        &mut mesh,
    );
    assert_eq!(mesh.vertices[0].position[3], 1.0);
    assert_eq!(mesh.vertices[120].position[3], 0.0);
    // Both terminal pairs collapse; the last repeats the penultimate offset and tangent.
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Long enough that the middle is past `RIVER_FADE_FROM` at both ends.
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
    )
    .unwrap();
    assert_eq!(trace.rivers.len(), 1);
    assert_eq!(mesh.ribbons, 1);
    let points = trace.rivers[0].points.len();
    assert_eq!(mesh.vertices.len(), points * 2);
    assert_eq!(mesh.indices.len(), (points - 1) * 6);

    // The two rails of one point share a position along the flow and differ across it.
    let (left, right) = (mesh.vertices[40], mesh.vertices[41]);
    assert_eq!(left.uv_tangent[0], 0.0);
    assert_eq!(right.uv_tangent[0], 1.0);
    assert_eq!(left.uv_tangent[1], right.uv_tangent[1]);
    // The river runs east, so the tangent is +x and the rails straddle it in z.
    assert!((left.uv_tangent[2] - 1.0).abs() < 1e-4);
    assert!(left.uv_tangent[3].abs() < 1e-4);
    let across = (left.position[2] - right.position[2]).abs();
    assert!(
        (across - width_for_index(6)).abs() < 1e-3,
        "ribbon is {across} wide"
    );
    // The along-flow coordinate is the point index, independent of arc length.
    let step = mesh.vertices[42].uv_tangent[1] - left.uv_tangent[1];
    assert_eq!(step, 1.0);
    assert_eq!(left.uv_tangent[1], 20.0);

    // A source fades at its first end; its final pair and middle are opaque.
    assert_eq!(mesh.vertices[0].position[3], 0.0);
    assert_eq!(mesh.vertices.last().unwrap().position[3], 1.0);
    assert_eq!(mesh.vertices[points].position[3], 1.0);
    // Byte 120 is sampled as height 12.0, without a sea-level clamp.
    assert_eq!(mesh.vertices[0].position[1], 12.0);
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
