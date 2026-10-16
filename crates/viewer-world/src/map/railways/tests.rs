use super::*;
use viewer_foundation::map::camera::MapDefines;
use viewer_foundation::map::camera::MapSize;

fn heights() -> HeightField<'static> {
    HeightField {
        width: 32,
        height: 32,
        values: &[100; 1024],
    }
}
fn defines() -> RailDefines {
    RailDefines {
        y_offset: 0.9,
        bridge_y_offset: 0.6,
        half_widths: [2.0, 2.25],
        half_gaps: [1.2, 1.3],
        levels: 5,
    }
}
fn geography() -> RailGeometry {
    RailGeometry {
        defines: defines(),
        anchors: BTreeMap::from([
            (1, [4.0, 10.0, 8.0]),
            (2, [12.0, 10.0, 8.0]),
            (3, [20.0, 10.0, 8.0]),
        ]),
        ports: BTreeMap::new(),
        river_points: BTreeMap::new(),
        boundaries: BTreeMap::from([
            (
                (1, 2),
                Boundary {
                    points: vec![[8, 8]],
                    kind: BridgeKind::None,
                },
            ),
            (
                (2, 3),
                Boundary {
                    points: vec![[16, 8]],
                    kind: BridgeKind::None,
                },
            ),
            (
                (1, 3),
                Boundary {
                    points: vec![[12, 10]],
                    kind: BridgeKind::None,
                },
            ),
        ]),
    }
}
fn edges(pairs: &[(u32, u32, i32)]) -> Vec<RailEdgeRow> {
    pairs
        .iter()
        .flat_map(|&(a, b, level)| {
            [
                RailEdgeRow {
                    from: a,
                    to: b,
                    level,
                },
                RailEdgeRow {
                    from: b,
                    to: a,
                    level,
                },
            ]
        })
        .collect()
}
#[test]
fn anchor_slots_fold_ascii_case_and_skip_ambiguous_slots() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let map = MapData {
        width: 32,
        height: 32,
        province_ids: vec![1; 1024],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    let rivers = installed_content::image::IndexedImage {
        width: 32,
        height: 32,
        indices: vec![255; 1024],
        palette: [[0; 4]; 256],
    };
    let types = installed_content::buildings::parse(
        b"buildings={Hub={supply_node=yes} Port={is_port=yes spawn_point=Harbour}} spawn_points={Harbour={}}", &installed_content::key_names::KeyNames::default(),
    )
    .unwrap();
    let positions =
        installed_content::positions::parse_buildings(b"1;hUB;4;5;6;0;0\n1;hARBOUR;7;8;9;0;0\n").unwrap();
    let geometry =
        RailGeometry::new(&map, &positions, &types, &rivers, &heights(), defines()).unwrap();
    assert_eq!(geometry.anchors[&1], [4.0, 5.0, 6.0]);
    assert_eq!(geometry.ports[&1], [7.0, 8.0, 9.0]);
    let ambiguous = installed_content::buildings::parse(
        b"buildings={Hub={supply_node=yes} hub={} Port={is_port=yes spawn_point=Harbour}} spawn_points={Harbour={} harbour={}}", &installed_content::key_names::KeyNames::default(),
    )
    .unwrap();
    let geometry =
        RailGeometry::new(&map, &positions, &ambiguous, &rivers, &heights(), defines()).unwrap();
    assert_ne!(geometry.anchors[&1], [4.0, 5.0, 6.0]);
    assert!(geometry.ports.is_empty());
}

#[test]
fn native_strips_preserve_triangles_and_do_not_join_provinces() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mesh = geography().build_rows(&edges(&[(1, 2, 2), (2, 3, 4)]), &[], &[], &heights());
    let pack = |v: RailVertex| {
        [
            v.position[0],
            v.position[1],
            v.position[2],
            1.0,
            v.uv[0],
            v.uv[1],
        ]
    };
    let visible = |t: &[[f32; 6]; 3]| t[0] != t[1] && t[1] != t[2] && t[0] != t[2];
    let old: Vec<_> = mesh
        .indices
        .chunks_exact(3)
        .map(|t| {
            [
                pack(mesh.vertices[t[0] as usize]),
                pack(mesh.vertices[t[1] as usize]),
                pack(mesh.vertices[t[2] as usize]),
            ]
        })
        .filter(visible)
        .collect();
    let mut native = Vec::new();
    let mut end = 0;
    for run in &mesh.native_runs {
        assert_eq!(run.start, end);
        let vertices = &mesh.native_vertices[run.start as usize..run.end as usize];
        for (i, t) in vertices.windows(3).enumerate() {
            let triangle = if i % 2 == 0 {
                [t[0], t[1], t[2]]
            } else {
                [t[1], t[0], t[2]]
            };
            if visible(&triangle) {
                native.push(triangle);
            }
        }
        end = run.end;
    }
    assert_eq!(end as usize, mesh.native_vertices.len());
    assert_eq!(mesh.native_runs.len(), 3);
    assert_eq!(native, old);
    assert_eq!(std::mem::size_of::<[f32; 6]>(), 24);
}

#[test]
fn building_and_losing_edges_changes_the_current_network() {
    let geo = geography();
    let h = heights();
    assert!(geo.build_rows(&[], &[], &[], &h).vertices.is_empty());
    let one = geo.build_rows(&edges(&[(1, 2, 2)]), &[], &[], &h);
    assert_eq!(one.segments, 2);
    let two = geo.build_rows(&edges(&[(1, 2, 2), (2, 3, 4)]), &[], &[], &h);
    assert_eq!(two.segments, 3);
    assert_ne!(one.vertices, two.vertices);
    assert_eq!(geo.build_rows(&edges(&[(1, 2, 2)]), &[], &[], &h), one);
    assert!(geo.build_rows(&[], &[], &[], &h).vertices.is_empty());
}
#[test]
fn a_train_route_runs_over_the_boundary_crossings_and_out_of_its_first_province() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let geo = geography();
    let h = heights();
    let network = RailNetwork::new(&edges(&[(1, 2, 1), (2, 3, 1)]), &[], &[]);
    let route = geo.train_route(&[1, 2, 3], &network, &h).unwrap();
    assert!(!route.open_end);
    let route = route.points;
    // The terminal stubs end a fifth of the way out from the anchor, and
    // the interior province is the curve between its two crossings.
    assert!((route[0][0] - 4.8).abs() < 0.01, "{:?}", route[0]);
    assert!((route.last().unwrap()[0] - 19.2).abs() < 0.01);
    assert!(route.windows(2).all(|p| p[1][0] > p[0][0]));
    assert!(route.iter().all(|p| (p[2] - 8.0).abs() < 0.01));
    // terrain height alone, without the strip's 0.9 lift.
    assert!(route.iter().all(|p| (p[1] - 10.0).abs() < 0.01));
    assert_eq!(geo.train_route(&[1], &network, &h), None);
}
#[test]
fn a_route_ends_before_a_province_whose_railway_lacks_its_segment() {
    let geo = geography();
    let h = heights();
    // Province 2 skips the 1-3 curve (triangle); 1 has two neighbours, so
    // it has a spur only as a supply source.
    let triangle = edges(&[(1, 2, 2), (2, 3, 4), (1, 3, 3)]);
    let network = RailNetwork::new(&triangle, &[], &[]);
    assert_eq!(geo.train_route(&[1, 2, 3], &network, &h), None);
    let source = ProvinceSupplyRow {
        province: 1,
        supply_node_level: 1,
        ..Default::default()
    };
    let network = RailNetwork::new(&triangle, &[source], &[]);
    let route = geo.train_route(&[1, 2, 3], &network, &h).unwrap();
    assert!(route.open_end);
    // Only province 1's spur remains, ending at its crossing with 2.
    assert!(
        route.points.iter().all(|p| p[0] <= 8.01),
        "{:?}",
        route.points
    );
}
#[test]
fn triangles_are_skipped_but_supply_sources_receive_terminal_spurs() {
    let geo = geography();
    let h = heights();
    let triangle = edges(&[(1, 2, 2), (2, 3, 4), (1, 3, 3)]);
    assert!(geo.build_rows(&triangle, &[], &[], &h).vertices.is_empty());
    for source in [
        ProvinceSupplyRow {
            province: 2,
            supply_node_level: 1,
            ..Default::default()
        },
        ProvinceSupplyRow {
            province: 2,
            naval_base_level: 1,
            ..Default::default()
        },
        ProvinceSupplyRow {
            province: 2,
            capital_country_index: 1,
            ..Default::default()
        },
        ProvinceSupplyRow {
            province: 2,
            flags: 1,
            ..Default::default()
        },
    ] {
        assert_eq!(geo.build_rows(&triangle, &[source], &[], &h).segments, 2);
    }
}
#[test]
fn bridge_kind_selects_gap_and_leadin() {
    for (kind, gap, width) in [(BridgeKind::Small, 2.4, 4.0), (BridgeKind::Large, 2.6, 4.5)] {
        let (half_width, half_gap) = defines().bridge(kind);
        let start = Crossing {
            position: [4.0, 10.0, 8.0],
            direction: [1.0, 0.0, 0.0],
            half_width,
            half_gap,
        };
        let end = Crossing {
            position: [20.0, 10.0, 8.0],
            direction: [-1.0, 0.0, 0.0],
            half_width,
            half_gap,
        };
        let curve = Curve::new(start, end);
        let mut mesh = RailMesh::default();
        curve.emit(&mut mesh, 3, &heights(), &defines());
        assert!((mesh.vertices[0].position[0] - (4.0 + gap / 2.0)).abs() < 1e-5);
        assert!((mesh.vertices[2].position[0] - (4.0 + width / 2.0)).abs() < 1e-5);
        assert!((mesh.vertices.last().unwrap().position[0] - (20.0 - gap / 2.0)).abs() < 1e-5);
        assert!((mesh.vertices[0].uv[0] - gap / 2.0).abs() < 1e-5);
        assert!((mesh.vertices.last().unwrap().uv[0] - (16.0 - gap / 2.0)).abs() < 1e-5);
        for pair in mesh.vertices.chunks_exact(2) {
            assert!((length(sub(pair[0].position, pair[1].position)) - 1.0).abs() < 1e-5);
            assert!((pair[0].position[1] - 10.9).abs() < 1e-5);
            assert_eq!(pair[0].uv[1], 0.4);
            assert_eq!(pair[1].uv[1], 0.6);
        }
    }
}
#[test]
fn harbour_anchor_uses_naval_base_membership_even_at_zero_level() {
    let mut geo = geography();
    geo.ports.insert(1, [5.0, 10.0, 9.0]);
    let zero_level_base = NavalBaseRow {
        province: 1,
        level: 0,
        ..Default::default()
    };
    let supply_with_level = ProvinceSupplyRow {
        province: 1,
        naval_base_level: 1,
        ..Default::default()
    };
    let graph = edges(&[(1, 2, 1)]);
    let member = RailNetwork::new(&graph, &[], &[zero_level_base]);
    assert_eq!(geo.anchor(&member, 1), Some(&[5.0, 10.0, 9.0]));
    assert!(!member.sources.contains(&1));
    let level_without_member = RailNetwork::new(&graph, &[supply_with_level], &[]);
    assert_eq!(
        geo.anchor(&level_without_member, 1),
        Some(&[4.0, 10.0, 8.0])
    );
    assert!(level_without_member.sources.contains(&1));
}
#[test]
fn the_rail_fade_settles_first_then_moves_at_the_cutoff_speed() {
    let mut camera = Camera {
        center: [0.0; 2],
        height: 200.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    camera.defines.railway_cutoff = 300.0;
    camera.defines.railway_cutoff_speed = 3.0;
    camera.height = 200.0;
    let mut fade = RailFade::default();
    assert_eq!(fade.update(&camera, 5.0), 1.0);
    camera.height = 300.0;
    assert!((fade.update(&camera, 5.1) - 0.7).abs() < 1e-5);
    assert!(fade.trains_visible());
    assert_eq!(fade.update(&camera, 6.0), 0.0);
    assert!(!fade.trains_visible());
    camera.height = 299.0;
    assert!((fade.update(&camera, 6.2) - 0.6).abs() < 1e-5);
    // A clock that runs backwards, as a seek or a new dump, settles.
    camera.height = 400.0;
    assert_eq!(fade.update(&camera, 0.0), 0.0);
    camera.height = 100.0;
    assert_eq!(fade.update(&camera, 0.0), 0.0);
    assert_eq!(fade.update(&camera, 1.0), 1.0);
}
#[test]
fn truncated_directed_edges_are_not_completed_from_authored_content() {
    let mesh = geography().build_rows(
        &[RailEdgeRow {
            from: 1,
            to: 2,
            level: 1,
        }],
        &[],
        &[],
        &heights(),
    );
    assert_eq!(mesh.segments, 1);
    let end = &mesh.vertices[mesh.vertices.len() - 2..];
    assert!(((end[0].position[0] + end[1].position[0]) * 0.5 - 4.8).abs() < 1e-5);
}
#[test]
fn height_is_bilinear_at_the_center_with_the_native_half_pixel_offset() {
    let h = HeightField {
        width: 2,
        height: 2,
        values: &[120, 140, 100, 120],
    };
    assert!((rail_height(&h, 0.0, 0.0) - 12.0).abs() < 1e-5);
    assert_eq!(
        rail_height(
            &HeightField {
                width: 2,
                height: 2,
                values: &[0; 4]
            },
            0.0,
            0.0
        ),
        9.5
    );
}
