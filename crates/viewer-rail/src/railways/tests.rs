use super::*;
use viewer_gpu::camera::MapDefines;
use viewer_gpu::camera::MapSize;

fn heights() -> HeightField<'static> {
    HeightField {
        width: 32,
        height: 32,
        values: &[100; 1024],
    }
}
fn heightmap(byte: u8) -> Arc<IndexedImage> {
    Arc::new(IndexedImage {
        width: 32,
        height: 32,
        indices: vec![byte; 1024].into(),
        palette: [[0; 4]; 256],
    })
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
        id: viewer_scene_base::next_projection_id(),
        identity: Arc::new(()),
        heightmap: heightmap(100),
        defines: defines(),
        anchors: BTreeMap::from([
            (1, [4.0, 10.0, 8.0]),
            (2, [12.0, 10.0, 8.0]),
            (3, [20.0, 10.0, 8.0]),
        ]),
        ports: BTreeMap::new(),
        river_points: BTreeMap::new(),
        last: Mutex::new(None),
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
    content_reader_script::test_game::enter();
    let map = MapData {
        width: 32,
        height: 32,
        province_ids: vec![1; 1024],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    let rivers = content_reader_image::IndexedImage {
        width: 32,
        height: 32,
        indices: vec![255; 1024].into(),
        palette: [[0; 4]; 256],
    };
    let types = content_reader_key_names::buildings::parse(
        b"buildings={Hub={supply_node=yes} Port={is_port=yes spawn_point=Harbour}} spawn_points={Harbour={}}", &content_reader_script::key_names::KeyNames::default(),
    )
    .unwrap();
    let positions =
        content_reader_map::positions::parse_buildings(b"1;hUB;4;5;6;0;0\n1;hARBOUR;7;8;9;0;0\n")
            .unwrap();
    let geometry =
        RailGeometry::new(&map, &positions, &types, &rivers, heightmap(100), defines()).unwrap();
    assert_eq!(geometry.anchors[&1], [4.0, 5.0, 6.0]);
    assert_eq!(geometry.ports[&1], [7.0, 8.0, 9.0]);
    let ambiguous = content_reader_key_names::buildings::parse(
        b"buildings={Hub={supply_node=yes} hub={} Port={is_port=yes spawn_point=Harbour}} spawn_points={Harbour={} harbour={}}", &content_reader_script::key_names::KeyNames::default(),
    )
    .unwrap();
    let geometry = RailGeometry::new(
        &map,
        &positions,
        &ambiguous,
        &rivers,
        heightmap(100),
        defines(),
    )
    .unwrap();
    assert_ne!(geometry.anchors[&1], [4.0, 5.0, 6.0]);
    assert!(geometry.ports.is_empty());
}

#[test]
fn building_and_losing_edges_changes_the_current_network() {
    let geo = geography();
    assert!(geo.build_rows(&[], &[], &[]).vertices.is_empty());
    let one = geo.build_rows(&edges(&[(1, 2, 2)]), &[], &[]);
    assert_eq!(one.segments, 2);
    let two = geo.build_rows(&edges(&[(1, 2, 2), (2, 3, 4)]), &[], &[]);
    assert_eq!(two.segments, 3);
    assert_ne!(one.vertices, two.vertices);
    assert_eq!(geo.build_rows(&edges(&[(1, 2, 2)]), &[], &[]), one);
    assert!(geo.build_rows(&[], &[], &[]).vertices.is_empty());
}
#[test]
fn a_train_route_runs_over_the_boundary_crossings_and_out_of_its_first_province() {
    let geo = geography();
    let h = heights();
    let network = RailNetwork::new(&edges(&[(1, 2, 1), (2, 3, 1)]), &[], &[]);
    let route = geo.train_route(&[1, 2, 3], &network, &h).unwrap();
    assert!(!route.open_end);
    let route = route.points;
    assert!((route[0][0] - 4.8).abs() < 0.01, "{:?}", route[0]);
    assert!((route.last().unwrap()[0] - 19.2).abs() < 0.01);
    assert!(route.windows(2).all(|p| p[1][0] > p[0][0]));
    assert!(route.iter().all(|p| (p[2] - 8.0).abs() < 0.01));
    assert!(route.iter().all(|p| (p[1] - 10.0).abs() < 0.01));
    assert_eq!(geo.train_route(&[1], &network, &h), None);
}
#[test]
fn a_route_ends_before_a_province_whose_railway_lacks_its_segment() {
    let geo = geography();
    let h = heights();
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
    assert!(
        route.points.iter().all(|p| p[0] <= 8.01),
        "{:?}",
        route.points
    );
}
#[test]
fn triangles_are_skipped_but_supply_sources_receive_terminal_spurs() {
    let geo = geography();
    let triangle = edges(&[(1, 2, 2), (2, 3, 4), (1, 3, 3)]);
    assert!(geo.build_rows(&triangle, &[], &[]).vertices.is_empty());
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
        assert_eq!(geo.build_rows(&triangle, &[source], &[]).segments, 2);
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
fn truncated_directed_edges_are_not_completed_from_authored_content() {
    let mesh = geography().build_rows(
        &[RailEdgeRow {
            from: 1,
            to: 2,
            level: 1,
        }],
        &[],
        &[],
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

#[test]
fn rail_geometry_install_without_prepared_bank_preserves_bytes_and_runs_across_hours() {
    use viewer_gpu::test_support::gpu;
    let strip_count = |geometry: &PreparedRailGeometry| geometry.strips.as_ref().map(|(_, count)| *count);
    let joined = |runs: &[std::ops::Range<u32>]| (!runs.is_empty()).then(|| strip_indices(runs).len() as u32);
    let (_serial, device, queue) = gpu::device();
    fn bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        geometry: &PreparedRailGeometry,
    ) -> Vec<u8> {
        let Some(vertices) = &geometry.native_vertices else {
            assert!(geometry.strips.is_none());
            return Vec::new();
        };
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rail geometry oracle"),
            size: vertices.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(vertices, 0, &read, 0, vertices.size());
        queue.submit([encoder.finish()]);
        let slice = read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let result = slice.get_mapped_range().unwrap().to_vec();
        read.unmap();
        result
    }
    let mut adopted = Arc::new(PreparedRailGeometry::default());
    let mut previous = Vec::new();
    let mut previous_runs = Vec::new();
    for hour in 0..12 {
        let mesh = if hour % 6 == 0 {
            RailMesh::default()
        } else {
            let mut graph = edges(&[(1, 2, 1 + hour % 5)]);
            if hour % 3 != 0 {
                graph.extend(edges(&[(2, 3, 3)]));
            }
            geography().build_rows(&graph, &[], &[])
        };
        let prepared = std::thread::scope(|scope| {
            scope
                .spawn(|| PreparedRailGeometry::for_hour(Some(&device), &mesh))
                .join()
                .unwrap()
                .unwrap()
        });
        let prepared = prepared.expect("builder has a device");
        let geometry = PreparedRailGeometry::for_install(&device, &mesh, Some(&prepared)).unwrap();
        assert!(Arc::ptr_eq(&geometry, &prepared));
        let unprepared = PreparedRailGeometry::for_hour(None, &mesh).unwrap();
        assert!(unprepared.is_none());
        let fallback =
            PreparedRailGeometry::for_install(&device, &mesh, unprepared.as_ref()).unwrap();
        let install = if hour % 2 == 0 {
            geometry
        } else {
            Arc::clone(&fallback)
        };
        let retired = std::mem::replace(&mut adopted, install);
        assert_eq!(
            bytes(&device, &queue, &retired),
            previous,
            "retired hour {hour}"
        );
        assert_eq!(strip_count(&retired), joined(&previous_runs), "retired runs {hour}");
        let expected: &[u8] = bytemuck::cast_slice(&mesh.native_vertices);
        let actual = bytes(&device, &queue, &adopted);
        assert_eq!(actual, expected, "prepared bytes hour {hour}");
        assert_eq!(
            bytes(&device, &queue, &fallback),
            expected,
            "unprepared-hour install bytes hour {hour}"
        );
        assert_eq!(strip_count(&adopted), joined(&mesh.native_runs), "runs hour {hour}");
        assert_eq!(strip_count(&fallback), joined(&mesh.native_runs), "unprepared-hour install runs {hour}");
        previous = actual;
        previous_runs = mesh.native_runs;
    }
}

fn boundary(point: [i32; 2], kind: BridgeKind) -> Boundary {
    Boundary {
        points: vec![point],
        kind,
    }
}

fn wide() -> RailGeometry {
    RailGeometry {
        id: viewer_scene_base::next_projection_id(),
        identity: Arc::new(()),
        heightmap: heightmap(100),
        defines: defines(),
        anchors: BTreeMap::from([
            (1, [4.0, 10.0, 8.0]),
            (2, [12.0, 10.0, 8.0]),
            (3, [20.0, 10.0, 8.0]),
            (4, [28.0, 10.0, 8.0]),
            (5, [12.0, 10.0, 20.0]),
            (6, [20.0, 10.0, 20.0]),
        ]),
        ports: BTreeMap::from([(3, [21.0, 10.0, 9.0])]),
        river_points: BTreeMap::new(),
        boundaries: BTreeMap::from([
            ((1, 2), boundary([8, 8], BridgeKind::None)),
            ((2, 3), boundary([16, 8], BridgeKind::Small)),
            ((3, 4), boundary([24, 8], BridgeKind::None)),
            ((2, 5), boundary([12, 14], BridgeKind::None)),
            ((3, 6), boundary([20, 14], BridgeKind::Large)),
            ((5, 6), boundary([16, 20], BridgeKind::None)),
            ((2, 6), boundary([16, 14], BridgeKind::None)),
        ]),
        last: Mutex::new(None),
    }
}

fn network(pairs: &[(u32, u32, i32)], sources: &[u32], ports: &[u32]) -> Arc<RailNetwork> {
    let supply: Vec<_> = sources
        .iter()
        .map(|&province| ProvinceSupplyRow {
            province,
            supply_node_level: 1,
            ..Default::default()
        })
        .collect();
    let bases: Vec<_> = ports
        .iter()
        .map(|&province| NavalBaseRow {
            province,
            ..Default::default()
        })
        .collect();
    Arc::new(RailNetwork::new(&edges(pairs), &supply, &bases))
}

#[test]
fn patched_railways_equal_full_builds_step_by_step() {
    let geo = wide();
    let line = [(1, 2, 2), (2, 3, 3)];
    let longer = [(1, 2, 2), (2, 3, 3), (3, 4, 1)];
    let raised = [(1, 2, 2), (2, 3, 5), (3, 4, 1)];
    let triangle = [(1, 2, 2), (2, 3, 5), (3, 4, 1), (2, 6, 2), (3, 6, 2)];
    let broken = [(1, 2, 2), (2, 3, 5), (3, 4, 1), (2, 6, 2)];
    let unanchored = [(1, 2, 2), (2, 3, 5), (3, 4, 1), (2, 6, 2), (4, 7, 2)];
    let bridged = [
        (1, 2, 2),
        (2, 3, 5),
        (3, 4, 1),
        (2, 6, 2),
        (4, 7, 2),
        (5, 6, 1),
        (2, 5, 1),
        (3, 6, 4),
    ];
    let bare = [
        (1, 2, 2),
        (2, 3, 5),
        (2, 6, 2),
        (5, 6, 1),
        (2, 5, 1),
        (3, 6, 4),
    ];
    let steps = [
        network(&[], &[], &[]),
        network(&line, &[], &[]),
        network(&longer, &[], &[]),
        network(&raised, &[], &[]),
        network(&triangle, &[], &[]),
        network(&broken, &[], &[]),
        network(&broken, &[], &[]),
        network(&broken, &[4], &[]),
        network(&broken, &[], &[3]),
        network(&unanchored, &[], &[3]),
        network(&bridged, &[], &[3]),
        network(&bridged, &[], &[]),
        network(&bare, &[], &[]),
        network(&[], &[], &[]),
    ];
    let mut bases: Vec<Arc<RailBase>> = Vec::new();
    for (step, network) in steps.iter().enumerate() {
        let full = geo.build_network(network).unwrap();
        let (mesh, next) = geo
            .build_from(bases.last().map(|base| &**base), network)
            .unwrap();
        assert!(mesh.bit_eq(&full), "step {step}");
        match step {
            1 => assert!(!full.native_vertices.is_empty() && !full.bridges.is_empty()),
            6 | 7 => {
                let previous = bases.last().unwrap();
                let changed: Vec<_> = next
                    .runs
                    .iter()
                    .zip(&previous.runs)
                    .filter(|(new, old)| !Arc::ptr_eq(&new.vertices, &old.vertices))
                    .map(|(new, _)| new.province)
                    .collect();
                assert_eq!(changed, if step == 6 { vec![] } else { vec![4] });
            }
            9 => assert!(full.unanchored > 0),
            _ => {}
        }
        bases.push(Arc::new(next));
    }
    for (from, to) in [(10, 3), (12, 4), (8, 1)] {
        let (mesh, _) = geo.build_from(Some(&bases[from]), &steps[to]).unwrap();
        assert!(
            mesh.bit_eq(&geo.build_network(&steps[to]).unwrap()),
            "{from} to {to}"
        );
    }
}

#[test]
fn concurrent_patches_from_one_base_stay_exact_and_the_last_writer_wins() {
    let geo = wide();
    let start = network(&[(1, 2, 2), (2, 3, 3)], &[], &[]);
    let a = network(&[(1, 2, 2), (2, 3, 3), (3, 4, 1)], &[], &[3]);
    let b = network(&[(1, 2, 4), (2, 6, 2)], &[2], &[]);
    let c = network(&[(2, 3, 3), (3, 6, 4), (5, 6, 1)], &[], &[3]);
    let (_, first) = geo.build_from(None, &start).unwrap();
    geo.keep_base(Arc::new(first));
    let taken_a = geo.last_base();
    let taken_b = geo.last_base();
    assert!(Arc::ptr_eq(
        taken_a.as_ref().unwrap(),
        taken_b.as_ref().unwrap()
    ));
    let (mesh_a, base_a) = geo.build_from(taken_a.as_deref(), &a).unwrap();
    let (mesh_b, base_b) = geo.build_from(taken_b.as_deref(), &b).unwrap();
    assert!(mesh_a.bit_eq(&geo.build_network(&a).unwrap()));
    assert!(mesh_b.bit_eq(&geo.build_network(&b).unwrap()));
    geo.keep_base(Arc::new(base_b));
    geo.keep_base(Arc::new(base_a));
    let last = geo.last_base().unwrap();
    assert!(Arc::ptr_eq(&last.network, &a));
    let (mesh_c, _) = geo.build_from(Some(&last), &c).unwrap();
    assert!(mesh_c.bit_eq(&geo.build_network(&c).unwrap()));
    for network in [&a, &b, &c, &start, &a] {
        let full = geo.build_network(network).unwrap();
        assert!(geo.build(network).unwrap().bit_eq(&full));
        assert!(Arc::ptr_eq(&geo.last_base().unwrap().network, network));
    }
}

#[test]
fn a_foreign_base_rebuilds_the_owners_anchors_defines_and_bridges() {
    let first = wide();
    let mut second = wide();
    second.anchors.insert(2, [13.0, 10.0, 7.0]);
    second.defines.y_offset += 1.0;
    second.boundaries.get_mut(&(2, 3)).unwrap().kind = BridgeKind::Large;
    let network = network(&[(1, 2, 2), (2, 3, 3)], &[], &[]);
    let (old, base) = first.build_from(None, &network).unwrap();
    let full = second.build_network(&network).unwrap();
    assert!(!old.native_vertices.is_empty() && !old.bridges.is_empty());
    assert!(!old.bit_eq(&full));
    let (mesh, next) = second.build_from(Some(&base), &network).unwrap();
    assert!(mesh.bit_eq(&full));
    assert!(Arc::ptr_eq(&next.owner, &second.identity));
    assert!(
        next.runs
            .iter()
            .zip(&base.runs)
            .all(|(new, old)| !Arc::ptr_eq(&new.vertices, &old.vertices))
    );
    second.keep_base(Arc::new(base));
    assert!(second.last_base().is_none());
    second.keep_base(Arc::new(next));
    assert!(second.build(&network).unwrap().bit_eq(&full));
}

#[test]
fn construction_keeps_immutable_heights_and_rejects_a_different_height_owner() {
    let map = MapData {
        width: 32,
        height: 32,
        province_ids: (0..1024)
            .map(|pixel| if pixel % 32 < 16 { 1 } else { 2 })
            .collect(),
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    let rivers = content_reader_image::IndexedImage {
        width: 32,
        height: 32,
        indices: vec![255; 1024].into(),
        palette: [[0; 4]; 256],
    };
    let types = Buildings::default();
    let mut image = heightmap(100);
    let make =
        |heightmap| RailGeometry::new(&map, &[], &types, &rivers, heightmap, defines()).unwrap();
    let first = make(Arc::clone(&image));
    assert!(Arc::ptr_eq(&first.heightmap, &image));
    let network = network(&[(1, 2, 2)], &[], &[]);
    let before = first.build_network(&network).unwrap();
    assert!(!before.native_vertices.is_empty());
    assert!(first.build(&network).unwrap().bit_eq(&before));
    let base = first.last_base().unwrap();
    image = heightmap(200);
    let second = make(Arc::clone(&image));
    assert!(Arc::ptr_eq(&second.heightmap, &image));
    assert!(!Arc::ptr_eq(&first.heightmap, &second.heightmap));
    assert!(!Arc::ptr_eq(&first.identity, &second.identity));
    assert!(first.build(&network).unwrap().bit_eq(&before));
    let full = second.build_network(&network).unwrap();
    assert!(!before.bit_eq(&full));
    let (mesh, next) = second.build_from(Some(&base), &network).unwrap();
    assert!(mesh.bit_eq(&full));
    assert!(Arc::ptr_eq(&next.owner, &second.identity));
    assert!(
        next.runs
            .iter()
            .zip(&base.runs)
            .all(|(new, old)| !Arc::ptr_eq(&new.vertices, &old.vertices))
    );
    let (patched, repeated) = second.build_from(Some(&next), &network).unwrap();
    assert!(patched.bit_eq(&full));
    assert!(
        repeated
            .runs
            .iter()
            .zip(&next.runs)
            .all(|(new, old)| Arc::ptr_eq(&new.vertices, &old.vertices))
    );
}

#[test]
fn every_strip_is_joined_in_order_and_closed_by_the_restart_index() {
    let runs = [0..3, 5..9, 9..10];
    assert_eq!(
        strip_indices(&runs),
        [0, 1, 2, u32::MAX, 5, 6, 7, 8, u32::MAX, 9, u32::MAX],
        "each run's vertices in order, so each draws as the strip one call of it drew"
    );
    assert!(strip_indices(&[]).is_empty());
}
