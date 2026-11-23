use super::*;
use std::cell::Cell;
use viewer_gpu::camera::MapDefines;

const W: f32 = 5632.0;
use content_reader_map::adjacencies::{Adjacency, IMPASSABLE, SEA_CROSSING};

fn link(from: u32, to: u32, kind: u8) -> Adjacency {
    Adjacency {
        from,
        to,
        kind,
        through: -1,
        start: (-1, -1),
        stop: (-1, -1),
        rule: None,
    }
}

fn members(ids: &[u32]) -> BTreeSet<u32> {
    ids.iter().copied().collect()
}

#[test]
fn pixel_neighbours_wrap_in_x_and_stop_at_the_poles() {
    let ids = [1, 2, 3, 1, 4, 3];
    let graph = ProvinceGraph::build(3, 2, &ids, &[]).unwrap();
    assert_eq!(graph.components(&members(&[1, 3])), vec![vec![1, 3]]);
    assert_eq!(graph.components(&members(&[2, 4])), vec![vec![2, 4]]);
    assert_eq!(graph.components(&members(&[2, 3])), vec![vec![2, 3]]);
}

#[test]
fn raster_records_include_the_four_province_diagonal() {
    let ids = [1, 2, 9, 3, 4, 9];
    let graph = ProvinceGraph::build(3, 2, &ids, &[]).unwrap();
    assert_eq!(graph.components(&members(&[2, 3])), vec![vec![2, 3]]);
    assert_eq!(graph.components(&members(&[1, 4])), vec![vec![1], vec![4]]);
    let zero = ProvinceGraph::build(2, 1, &[0, 5], &[]).unwrap();
    assert_eq!(zero.edge_count(), 0);
}

#[test]
fn separated_islands_split_until_a_sea_link_joins_them() {
    let ids = [1, 0, 2, 0, 1, 0, 2, 0];
    let graph = ProvinceGraph::build(4, 2, &ids, &[]).unwrap();
    assert_eq!(graph.components(&members(&[1, 2])), vec![vec![1], vec![2]]);
    let linked = ProvinceGraph::build(4, 2, &ids, &[link(1, 2, SEA_CROSSING)]).unwrap();
    assert_eq!(linked.components(&members(&[1, 2])), vec![vec![1, 2]]);
    let rule = ProvinceGraph::build(4, 2, &ids, &[link(1, 2, 0)]).unwrap();
    assert_eq!(rule.components(&members(&[1, 2])), vec![vec![1, 2]]);
    let impassable = ProvinceGraph::build(4, 2, &ids, &[link(1, 2, IMPASSABLE)]).unwrap();
    assert_eq!(impassable.components(&members(&[1, 2])), vec![vec![1, 2]]);
}

#[test]
fn a_component_needs_a_path_inside_the_member_set() {
    let ids = [1, 2, 3, 0];
    let graph = ProvinceGraph::build(4, 1, &ids, &[]).unwrap();
    assert_eq!(graph.components(&members(&[1, 3])), vec![vec![1], vec![3]]);
    assert_eq!(graph.components(&members(&[1, 2, 3])), vec![vec![1, 2, 3]]);
    assert_eq!(
        graph.components(&members(&[3, 1])),
        vec![vec![1], vec![3]],
        "components come back in ascending order"
    );
}

#[test]
fn a_mismatched_province_image_is_refused() {
    assert!(ProvinceGraph::build(3, 2, &[1, 2, 3], &[]).is_err());
    assert!(ProvinceGraph::build(0, 0, &[], &[]).is_err());
}

#[test]
fn the_height_sample_interpolates_half_pixel_centres_before_water_clamping() {
    let heightmap = IndexedImage {
        width: 2,
        height: 2,
        indices: vec![0, 200, 95, 150].into(),
        palette: [[0; 4]; 256],
    };
    assert_eq!(height_at(&heightmap, 0.5, -0.5), WATER_HEIGHT);
    assert_eq!(height_at(&heightmap, 1.5, -0.5), 20.0);
    assert_eq!(height_at(&heightmap, 0.5, 0.5), WATER_HEIGHT);
    assert_eq!(height_at(&heightmap, 1.5, 0.5), 15.0);
    assert!((height_at(&heightmap, 1.0, 0.0) - 11.125).abs() < 1e-5);
}

fn world_name(key: String, x: f32, z: f32, glyphs: usize, wrapped: bool) -> WorldName {
    let mut corners = Vec::new();
    let mut uvs = Vec::new();
    for glyph in 0..glyphs {
        let left = x + glyph as f32 * 4.0;
        corners.extend([
            [left, 10.0, z + 4.0],
            [left + 4.0, 10.0, z + 4.0],
            [left + 4.0, 10.0, z],
            [left, 10.0, z],
        ]);
        uvs.push([0.0, 0.0, 0.1, 0.1]);
    }
    WorldName {
        strategic_provinces: None,
        key,
        text: String::new(),
        pages: vec![0; glyphs],
        corners,
        uvs,
        colors: vec![[1.0; 4]; glyphs],
        aabb: [x, z, x + glyphs as f32 * 4.0, z + 4.0],
        wrapped,
        font_size: 1.0,
        spaces: 0,
        alpha: 1.0,
    }
}

fn many_names(count: usize) -> Placements {
    let columns = 150;
    Placements {
        country: (0..count)
            .map(|index| {
                let x = (index % columns) as f32 * (W / columns as f32);
                let z = (index / columns) as f32 * 20.0;
                world_name(format!("N{index}"), x, z, 12, index == 0)
            })
            .collect(),
        generation: 7,
        ..Placements::default()
    }
}

fn camera(center: [f32; 2], height: f32) -> Camera {
    Camera {
        center,
        height,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    }
}

#[test]
fn a_name_across_font_pages_splits_into_one_entry_per_page_run() {
    let mut name = world_name("A".into(), 0.0, 0.0, 4, false);
    name.pages = vec![0, 0, 1, 0];
    let placements = Placements {
        country: vec![name],
        generation: 1,
        ..Placements::default()
    };
    let (chunks, names) = static_names(&placements, u64::MAX);
    assert_eq!(
        names
            .iter()
            .map(|n| (n.page, n.vertices.clone()))
            .collect::<Vec<_>>(),
        [(0, 0..12), (1, 12..18), (0, 18..24)]
    );
    assert_eq!(chunks[0].len(), 24);
}

#[test]
fn labels_follow_arrow_admission_and_preserve_page_runs() {
    let mut first = world_name("first".into(), -5000., 0., 3, true);
    first.pages = vec![2, 0, 2];
    first.alpha = 0.375;
    let (_, names) = static_names(
        &Placements {
            country: vec![first, world_name("second".into(), 100., 1000., 1, false)],
            ..Default::default()
        },
        u64::MAX,
    );
    let draws = arrow_draws(&names, &[(2, 1), (0, 0)]);
    assert_eq!(
        draws
            .iter()
            .map(|draw| (
                draw.page,
                draw.view,
                draw.vertices.clone(),
                draw.alpha_offset
            ))
            .collect::<Vec<_>>(),
        [
            (0, 2, 18..24, [1., 0.]),
            (2, 0, 0..6, [0.375, 0.]),
            (0, 0, 6..12, [0.375, 0.]),
            (2, 0, 12..18, [0.375, 0.]),
        ]
    );
    assert!(arrow_draws(&names, &[]).is_empty());
    assert_eq!(arrow_draws(&names, &[(1, 1)]).len(), 1);
}

#[test]
fn native_view_owns_every_font_page_of_an_admitted_name_once() {
    let mut name = world_name("A".into(), 100., 1000., 3, false);
    name.pages = vec![0, 1, 0];
    let (_, names) = static_names(
        &Placements {
            country: vec![name],
            ..Default::default()
        },
        u64::MAX,
    );
    let view = camera([100., 1000.], 300.);
    let draws = native_copies(&names, view, [1920, 1080], 0.255, &[0., 0.]);
    assert_eq!(
        draws
            .iter()
            .map(|draw| (draw.page, draw.view))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 0), (0, 0)]
    );
    assert!(
        draws
            .iter()
            .all(|draw| draw.alpha_offset == [names::label_alpha(0.255), 0.])
    );
    assert!(native_copies(&names, view, [1920, 1080], 0., &[0.]).is_empty());
}

#[test]
fn native_wrapped_box_wins_when_both_boxes_survive() {
    let (_, names) = static_names(
        &Placements {
            country: vec![world_name("seam".into(), W - 8., 1000., 4, true)],
            ..Default::default()
        },
        u64::MAX,
    );
    let draws = native_copies(
        &names,
        camera([W / 2., 1000.], 10000.),
        [1920, 1080],
        1.,
        &[0., -W, W],
    );
    assert_eq!(draws.len(), 1);
    assert_eq!(draws[0].view, 0);
    assert_eq!(draws[0].alpha_offset, [1., -W]);
}

#[test]
fn native_culling_tests_the_complete_name_before_submitting_pages() {
    let (_, mut names) = static_names(
        &Placements {
            country: vec![world_name("pages".into(), 100., 1000., 1, false)],
            ..Default::default()
        },
        u64::MAX,
    );
    let mut outside = names[0].clone();
    outside.page = 2;
    outside.bounds = [[-5000., 0., 0.], [-4999., 1., 1.]];
    names.insert(0, outside);
    let draws = native_copies(&names, camera([100., 1000.], 300.), [1920, 1080], 1., &[0.]);
    assert_eq!(
        draws.iter().map(|draw| draw.page).collect::<Vec<_>>(),
        [2, 0]
    );
    assert!(
        native_copies(
            &names[..1],
            camera([100., 1000.], 300.),
            [1920, 1080],
            1.,
            &[0.]
        )
        .is_empty()
    );
}

#[test]
fn names_keep_the_camera_side_margin() {
    let camera = camera([3000., 1000.], 50.);
    let viewport = [1024, 768];
    let m = camera.view_projection(viewport);
    let plane =
        |column: usize, sign: f32| std::array::from_fn(|row| -(m[row][3] + sign * m[row][column]));
    let open = viewer_gpu::province_admission::OPEN;
    let raw = [
        plane(0, 1.),
        plane(0, -1.),
        plane(1, 1.),
        plane(1, -1.),
        open,
        open,
    ];
    for (x, expected) in [(3035., 1), (3060., 0)] {
        let (_, names) = static_names(
            &Placements {
                country: vec![world_name("margin".into(), x, 1000., 1, false)],
                ..Default::default()
            },
            u64::MAX,
        );
        assert!(!admits_box(names[0].bounds, 0., &raw));
        assert_eq!(
            native_copies(&names, camera, viewport, 1., &[0.]).len(),
            expected
        );
    }
}

#[test]
fn static_names_are_built_once_and_chunked_under_the_buffer_limit() {
    content_reader_script::test_game::enter();
    let placements = many_names(30);
    let (chunks, names) = static_names(&placements, u64::MAX);
    assert_eq!(chunks.len(), 1);
    assert_eq!(names.len(), 30);
    assert_eq!(chunks[0].len(), 30 * 12 * 6);
    assert!(
        names
            .windows(2)
            .all(|pair| pair[0].vertices.end == pair[1].vertices.start)
    );
    let limit = 72 * 2 + 50;
    let (chunks, names) = static_names(&placements, limit);
    assert_eq!(chunks.len(), 15);
    assert!(chunks.iter().all(|chunk| chunk.len() as u64 <= limit));
    let total: usize = chunks.iter().map(Vec::len).sum();
    assert_eq!(total, 30 * 72);
    assert!(
        names
            .iter()
            .all(|name| name.vertices.end <= chunks[name.chunk].len() as u32)
    );
}

#[test]
fn the_fade_settles_at_the_content_cutoff_and_takes_one_second() {
    let mut defines = MapDefines::test();
    defines.country_names_cutoff = 260.0;
    let at = |height| Camera {
        center: [0.0, 0.0],
        height,
        map: MapSize::TEST,
        defines,
        pose: viewer_gpu::camera::Pose::Map,
    };
    assert_eq!(LayerFade::instant(at(260.0)), LayerFade(1.0));
    let low = LayerFade::instant(at(259.9));
    assert_eq!(low, LayerFade(0.0));
    let half = low.step(at(310.0), 0.5);
    assert_eq!(half, LayerFade(0.5));
    let settled = half.step(at(310.0), 0.5);
    assert_eq!(settled, LayerFade(1.0));
    assert_eq!(settled.step(at(310.0), -1.0), settled);
}

fn placed_name(text: &str, glyphs: usize) -> WorldName {
    WorldName {
        strategic_provinces: None,
        key: "GER".into(),
        text: text.into(),
        pages: vec![0; glyphs],
        corners: vec![[0.0; 3]; glyphs * 4],
        uvs: vec![[0.0; 4]; glyphs],
        colors: vec![[1.0; 4]; glyphs],
        aabb: [0.0; 4],
        wrapped: false,
        font_size: 1.0,
        spaces: 0,
        alpha: 1.0,
    }
}

#[test]
fn a_placed_area_is_reused_while_the_last_placement_shows_it() {
    let key = |area: u32| ("GER".to_owned(), "Germany".to_owned(), vec![area], false);
    let placed = PlacedAreas::default();
    let builds = Cell::new(0);
    let place = |areas: &[u32]| {
        let generation = placed.begin();
        for &area in areas {
            placed.place(generation, key(area), || {
                builds.set(builds.get() + 1);
                Some(placed_name("Germany", 7))
            });
        }
        placed.end(generation);
    };
    place(&[1, 2]);
    place(&[1, 2]);
    assert_eq!(builds.get(), 2, "an area is placed once");
    place(&[1, 3]);
    assert_eq!(builds.get(), 3);
    assert_eq!(placed.lock().placed.len(), 2, "only what the last placement showed is kept");
    place(&[2]);
    assert_eq!(builds.get(), 4, "an area no longer shown is placed again");
}

fn name_frame(controllers: [u32; 2]) -> DecodedFrame {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::ProvinceRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut tables: [TableView; TableId::ALL.len()] =
        std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        });
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [(0, b"GER"), (1, b"SOV")]
            .map(|(slot, tag)| CountryRow {
                canonical_slot: slot,
                id: 100 + slot,
                tag: tag.to_vec(),
                ..Default::default()
            })
            .into(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [0, 1]
            .map(|slot| CountryLinkRow {
                id: 100 + slot,
                canonical_slot: slot,
            })
            .into(),
    ));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        [1, 2]
            .into_iter()
            .zip(controllers)
            .map(|(id, controller)| ProvinceRow {
                id,
                owner: 100,
                controller,
                ..Default::default()
            })
            .collect(),
    ));
    DecodedFrame {
        branch: 1,
        hour: 1,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

#[test]
fn named_areas_change_with_control_and_nothing_else() {
    let map = MapData {
        width: 2,
        height: 1,
        province_ids: vec![1, 2],
        definitions: [1, 2]
            .map(|id| {
                (
                    id,
                    content_reader_map::map::ProvinceDefinition {
                        id,
                        rgb: [0; 3],
                        kind: content_reader_map::map::ProvinceKind::Land,
                        continent: 0,
                    },
                )
            })
            .into(),
        country_colors: Default::default(),
    };
    let localisation = Localisation::default();
    let areas =
        |controllers| NameAreas::read(&map, &localisation, &name_frame(controllers), None).unwrap();
    assert_eq!(areas([100, 100]), areas([100, 100]));
    assert_ne!(areas([100, 100]), areas([100, 101]));
    let both = GroupLabels {
        groups: BTreeMap::from([
            (0, (9, Ok("Axis".to_owned()))),
            (1, (9, Ok("Axis".to_owned()))),
        ]),
        others_named: true,
    };
    let factions = NameAreas::read(
        &map,
        &localisation,
        &name_frame([100, 101]),
        Some(&Ok(both)),
    )
    .unwrap();
    assert_eq!(factions.countries.len(), 1);
    assert_eq!(factions.countries[0].text, Ok("Axis".to_owned()));
    assert_eq!(factions.countries[0].members, BTreeSet::from([1, 2]));
    let one = GroupLabels {
        groups: BTreeMap::from([(1, (9, Ok("Axis".to_owned())))]),
        others_named: true,
    };
    let mixed =
        NameAreas::read(&map, &localisation, &name_frame([100, 101]), Some(&Ok(one))).unwrap();
    assert_eq!(
        mixed
            .countries
            .iter()
            .map(|area| (area.tag.as_str(), area.faction))
            .collect::<Vec<_>>(),
        [("GER", false), ("group 9", true)]
    );
    let refused = NameAreas::read(
        &map,
        &localisation,
        &name_frame([100, 101]),
        Some(&Err("no factions".to_owned())),
    )
    .unwrap();
    assert!(refused.countries.is_empty());
    assert_eq!(
        refused.warnings,
        ["The map mode's group names are refused: no factions"]
    );
    let player = GroupLabels {
        groups: BTreeMap::from([(1, (1, Ok("Player".to_owned())))]),
        others_named: false,
    };
    let players = NameAreas::read(
        &map,
        &localisation,
        &name_frame([100, 101]),
        Some(&Ok(player)),
    )
    .unwrap();
    assert_eq!(players.countries.len(), 1);
    assert_eq!(players.countries[0].text, Ok("Player".to_owned()));
    assert_eq!(players.countries[0].members, BTreeSet::from([2]));
}

#[test]
fn state_areas_name_each_states_provinces_and_go_close_in() {
    use replay_data::{rows::StateRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let map = MapData {
        width: 3,
        height: 1,
        province_ids: vec![1, 2, 3],
        definitions: [1, 2, 3]
            .map(|id| {
                (
                    id,
                    content_reader_map::map::ProvinceDefinition {
                        id,
                        rgb: [0; 3],
                        kind: content_reader_map::map::ProvinceKind::Land,
                        continent: 0,
                    },
                )
            })
            .into(),
        country_colors: Default::default(),
    };
    let mut frame = name_frame([100, 101]);
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    provinces[0].state = 7;
    provinces[1].state = 8;
    frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(provinces)).into();
    frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [7, 8, 9]
            .map(|id| StateRow {
                id,
                ..Default::default()
            })
            .into(),
    ))
    .into();
    let states = NameAreas::states(&map, &frame, |state| match state.id {
        7 => Ok("Seven".to_owned()),
        _ => Err("unnamed".to_owned()),
    })
    .unwrap();
    let named: Vec<_> = states
        .countries
        .iter()
        .map(|area| (area.tag.as_str(), area.text.clone(), area.members.clone()))
        .collect();
    assert_eq!(
        named,
        [
            ("state 7", Ok("Seven".to_owned()), BTreeSet::from([1])),
            ("state 8", Err("unnamed".to_owned()), BTreeSet::from([2])),
        ]
    );
    assert!(states.countries.iter().all(|area| area.faction));
    let localisation = Localisation::default();
    let far = NameAreas::read(&map, &localisation, &frame, None).unwrap();
    let both = far.clone().with_near(states.clone());
    assert_eq!(both.near.as_deref(), Some(&states));
    assert_ne!(both, far);
}

#[test]
fn strategic_names_group_land_and_sea_by_content_and_withhold_unresolved_names() {
    use content_reader_map::map::{CountryColors, ProvinceDefinition, ProvinceKind};
    use content_reader_regions::regions::StrategicRegion;
    let map = MapData {
        width: 3,
        height: 1,
        province_ids: vec![1, 2, 3],
        definitions: [
            (1, ProvinceKind::Land),
            (2, ProvinceKind::Sea),
            (3, ProvinceKind::Lake),
        ]
        .map(|(id, kind)| {
            (
                id,
                ProvinceDefinition {
                    id,
                    kind,
                    rgb: [0; 3],
                    continent: 0,
                },
            )
        })
        .into(),
        country_colors: CountryColors::default(),
    };
    let regions = [
        (
            19,
            StrategicRegion {
                id: 19,
                provinces: vec![1, 2, 999],
                ..Default::default()
            },
        ),
        (
            31,
            StrategicRegion {
                id: 31,
                provinces: vec![3],
                ..Default::default()
            },
        ),
    ]
    .into();
    let areas = NameAreas::strategic(&map, &regions, |r| {
        if r.id == 19 {
            Ok("Synthetic coast".into())
        } else {
            Err("synthetic naming refusal".into())
        }
    });
    assert_eq!(areas.countries[0].members, BTreeSet::from([1, 2]));
    assert_eq!(areas.countries[0].text, Ok("Synthetic coast".into()));
    assert_eq!(areas.countries[1].members, BTreeSet::from([3]));
    assert!(areas.countries.iter().all(|area| area.faction));
    assert!(areas.countries[1].text.is_err());
}

#[test]
fn strategic_hover_admits_one_connected_area_across_font_pages_and_wrapped_copies() {
    let mut first = world_name("synthetic coast".into(), 100.0, 1000.0, 2, true);
    first.strategic_provinces = Some(Arc::from([1, 2]));
    first.pages = vec![0, 2];
    let mut second = first.clone();
    second.key = "synthetic inland".into();
    second.strategic_provinces = Some(Arc::from([3]));
    let mut disconnected = first.clone();
    disconnected.strategic_provinces = Some(Arc::from([4, 5]));
    let ordinary = world_name("synthetic country".into(), 100.0, 1000.0, 1, false);
    let (_, names) = static_names(
        &Placements {
            country: vec![first, second, disconnected, ordinary],
            ..Default::default()
        },
        u64::MAX,
    );
    let draw = |hover| {
        native_hover_copies(
            &names,
            camera([100.0, 1000.0], 310.0),
            [1920, 1080],
            1.0,
            &[0.0, -W, W],
            StrategicNames::Hover(hover),
        )
    };
    assert_eq!(
        draw(Some(1))
            .iter()
            .map(|d| (d.page, d.vertices.start))
            .collect::<Vec<_>>(),
        [(0, 0), (2, 6), (0, 36)]
    );
    assert_eq!(
        draw(Some(3))
            .iter()
            .map(|d| (d.page, d.vertices.start))
            .collect::<Vec<_>>(),
        [(0, 12), (2, 18), (0, 36)]
    );
    assert_eq!(draw(Some(1)), draw(Some(2)));
    assert_eq!(
        draw(Some(4))
            .iter()
            .map(|d| (d.page, d.vertices.start))
            .collect::<Vec<_>>(),
        [(0, 24), (2, 30), (0, 36)],
        "hover selects the connected area rather than every component of its region"
    );
    assert_eq!(
        draw(Some(0)).len(),
        1,
        "a picked province without a region names no strategic area"
    );
    assert_eq!(
        draw(None).len(),
        1,
        "hover-only names hide strategic areas when the pointer leaves the map"
    );
    assert_eq!(
        native_copies(
            &names,
            camera([100.0, 1000.0], 310.0),
            [1920, 1080],
            1.0,
            &[0.0, -W, W],
        )
        .len(),
        7,
        "show-all still admits every strategic area and ordinary country name"
    );
    assert!(
        native_hover_copies(
            &names,
            camera([100.0, 1000.0], 310.0),
            [1920, 1080],
            0.0,
            &[0.0],
            StrategicNames::Hover(Some(1))
        )
        .is_empty()
    );
    assert_eq!(
        names
            .iter()
            .map(|n| n.strategic_provinces.as_deref())
            .collect::<Vec<_>>(),
        [
            Some([1, 2].as_slice()),
            Some([1, 2].as_slice()),
            Some([3].as_slice()),
            Some([3].as_slice()),
            Some([4, 5].as_slice()),
            Some([4, 5].as_slice()),
            None,
        ]
    );
    assert!(Arc::ptr_eq(
        names[0].strategic_provinces.as_ref().unwrap(),
        names[1].strategic_provinces.as_ref().unwrap()
    ));
}

#[test]
fn strategic_region_switch_resets_the_fade_only_on_the_hover_path() {
    let current = LayerFade(1.0);
    assert_eq!(current.strategic_hover(false, Some(19), Some(19)), current);
    assert_eq!(
        current.strategic_hover(false, Some(31), Some(19)),
        LayerFade(0.0)
    );
    assert_eq!(current.strategic_hover(true, Some(31), Some(19)), current);
    assert_eq!(current.strategic_hover(false, None, Some(19)), current);
    let reset = current.strategic_hover(false, Some(19), None);
    let camera = camera([100.0, 1000.0], 310.0);
    assert_eq!(reset.step(camera, 0.25).0, 0.25);
    assert_eq!(reset.step(camera, 1.5).0, 1.0);
}

#[test]
fn dense_components_equal_the_lookup_walk() {
    let ids = [
        1, 1, 2, 2, 0, 5, 5, 9,
        1, 3, 3, 2, 0, 5, 12, 9,
        40, 40, 3, 0, 0, 0, 12, 12,
    ];
    let rows = [link(40, 5, SEA_CROSSING)];
    let graph = ProvinceGraph::build(8, 3, &ids, &rows).unwrap();
    let lookup = lookup_graph(8, 3, &ids, &rows);
    let pool = [0, 1, 2, 3, 5, 9, 12, 40, 77];
    for mask in 0u32..1 << pool.len() {
        let members: BTreeSet<u32> = pool
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1u32 << bit) != 0)
            .map(|(_, &id)| id)
            .collect();
        assert_eq!(
            graph.components(&members),
            lookup_components(&lookup, &members),
            "{members:?}"
        );
    }
    for step in [
        &[1][..],
        &[1, 3],
        &[1, 3, 12, 40],
        &[3, 12],
        &[12, 40, 9, 77],
        &[5],
        &[],
    ] {
        let area = members(step);
        assert_eq!(
            graph.components(&area),
            lookup_components(&lookup, &area),
            "{step:?}"
        );
    }
}

fn lookup_graph(
    width: usize,
    height: usize,
    ids: &[u32],
    rows: &[Adjacency],
) -> HashMap<u32, Vec<u32>> {
    let mut graph: HashMap<u32, BTreeSet<u32>> = HashMap::new();
    let mut add = |a: u32, b: u32| {
        if a != b {
            graph.entry(a).or_default().insert(b);
            graph.entry(b).or_default().insert(a);
        }
    };
    for (id, neighbours) in content_reader_map::movement::raster_neighbours(width, height, ids)
        .unwrap()
        .iter()
        .enumerate()
    {
        for &next in neighbours {
            add(id as u32, next);
        }
    }
    for row in rows {
        add(row.from, row.to);
    }
    graph
        .into_iter()
        .map(|(id, neighbours)| (id, neighbours.into_iter().collect()))
        .collect()
}

fn lookup_components(graph: &HashMap<u32, Vec<u32>>, members: &BTreeSet<u32>) -> Vec<Vec<u32>> {
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut out = Vec::new();
    for &start in members {
        if !seen.insert(start) {
            continue;
        }
        let mut component = vec![start];
        let mut stack = vec![start];
        while let Some(province) = stack.pop() {
            for &next in graph.get(&province).map_or(&[][..], Vec::as_slice) {
                if members.contains(&next) && seen.insert(next) {
                    component.push(next);
                    stack.push(next);
                }
            }
        }
        component.sort_unstable();
        out.push(component);
    }
    out
}

#[test]
fn sparse_authored_ids_and_absent_members_use_compact_ordinals() {
    let huge = i32::MAX as u32;
    let ids = [1, 2];
    let rows = [
        link(1, huge, SEA_CROSSING),
        link(huge, 1_000_000, IMPASSABLE),
        link(huge, 1, 0),
        link(huge, huge, 0),
    ];
    let graph = ProvinceGraph::build(2, 1, &ids, &rows).unwrap();
    let lookup = lookup_graph(2, 1, &ids, &rows);
    assert_eq!(graph.ids, [1, 2, 1_000_000, huge]);
    assert_eq!(graph.rows.len(), 5);
    assert_eq!(graph.flat.len(), 6);
    assert_eq!(graph.edge_count(), 3);
    let pool = [0, 1, 2, 77, 1_000_000, huge - 1, huge, u32::MAX];
    for mask in 0u32..1 << pool.len() {
        let area = pool
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1u32 << bit) != 0)
            .map(|(_, &id)| id)
            .collect();
        assert_eq!(
            graph.components(&area),
            lookup_components(&lookup, &area),
            "{area:?}"
        );
    }
    assert_eq!(
        graph.components(&members(&pool)),
        vec![
            vec![0],
            vec![1, 2, 1_000_000, huge],
            vec![77],
            vec![huge - 1],
            vec![u32::MAX]
        ]
    );
    let sparse_raster = ProvinceGraph::build(2, 1, &[1, huge], &rows).unwrap();
    assert_eq!(sparse_raster.ids, [1, 1_000_000, huge]);
    assert_eq!(sparse_raster.edge_count(), 2);
    let mut raster_lookup = lookup.clone();
    raster_lookup.remove(&2);
    raster_lookup.get_mut(&1).unwrap().retain(|&id| id != 2);
    for area in [members(&pool), members(&[huge, 77]), members(&[1, huge])] {
        assert_eq!(
            sparse_raster.components(&area),
            lookup_components(&raster_lookup, &area)
        );
    }
    let empty = ProvinceGraph::build(1, 1, &[1], &[]).unwrap();
    assert!(empty.ids.is_empty());
    assert_eq!(
        empty.components(&members(&[huge, 1, 0])),
        vec![vec![0], vec![1], vec![huge]]
    );
}

#[test]
fn authored_localized_name_color_runs_survive_glyph_emission() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_name_pass_colors_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |rel: &str, bytes: &[u8]| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    let mut dds = vec![0u8; 128 + 16 * 4];
    dds[0..4].copy_from_slice(b"DDS ");
    dds[4..8].copy_from_slice(&124u32.to_le_bytes());
    dds[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
    dds[12..16].copy_from_slice(&8u32.to_le_bytes());
    dds[16..20].copy_from_slice(&8u32.to_le_bytes());
    dds[76..80].copy_from_slice(&32u32.to_le_bytes());
    dds[80..84].copy_from_slice(&0x4u32.to_le_bytes());
    dds[84..88].copy_from_slice(b"DXT5");
    write(
        "interface/fonts.gfx",
        b"bitmapfonts = {\n\
          textcolors = { Y = { 255 204 0 } }\n\
          bitmapfont = {\n\
            name = \"f\"\n\
            path = \"gfx/fonts/f\"\n\
            textcolors = { G = { 51 204 77 } }\n\
          }\n\
        }\n",
    );
    write(
        "gfx/fonts/f.fnt",
        b"common lineHeight=10 base=8 scaleW=8 scaleH=8 pages=1\n\
          char id=65 x=0 y=0 width=5 height=7 xoffset=0 yoffset=1 xadvance=6 page=0\n\
          char id=66 x=0 y=0 width=5 height=7 xoffset=0 yoffset=1 xadvance=6 page=0\n\
          char id=67 x=0 y=0 width=5 height=7 xoffset=0 yoffset=1 xadvance=6 page=0\n\
          char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=4 page=0\n",
    );
    write("gfx/fonts/f.dds", &dds);

    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(root.as_path(), &[][..])])
        .unwrap();
    let interface = viewer_native_gui::sprites::Interface::load(
        &layers,
        content_reader_image::native::Decode::default(),
    )
    .unwrap();
    let mut images = Vec::new();
    let font = text::load(&layers, &interface, "f", "english", &mut images).unwrap();

    let (w, h) = (200u32, 100u32);
    let mut ids = vec![0u32; (w * h) as usize];
    for y in 30..=69 {
        for x in 20..=179 {
            ids[(y * w + x) as usize] = 1;
        }
    }
    let map = MapData {
        width: w,
        height: h,
        province_ids: ids,
        definitions: BTreeMap::from([(
            1,
            content_reader_map::map::ProvinceDefinition {
                id: 1,
                rgb: [0; 3],
                kind: content_reader_map::map::ProvinceKind::Land,
                continent: 0,
            },
        )]),
        country_colors: Default::default(),
    };
    let heightmap = IndexedImage {
        width: w,
        height: h,
        indices: vec![120; (w * h) as usize].into(),
        palette: [[0; 4]; 256],
    };
    let loc: Localisation = [("GER".to_owned(), "A§YB§GC§!B§!A".to_owned())]
        .into_iter()
        .collect();
    let authored = loc.get("GER").unwrap().to_owned();
    let content = NameContent {
        id: 1,
        graph: ProvinceGraph::build(w, h, &map.province_ids, &[]).unwrap(),
        bounds: names::scan_province_bounds(&ProvinceMap {
            width: w,
            height: h,
            ids: &map.province_ids,
        }),
        bridged: HashSet::new(),
        localisation: Arc::new(loc),
    };
    let placed = super::world_name(
        &PlacementInputs {
            map: &map,
            heightmap: &heightmap,
            content: &content,
            font: &font,
        },
        "GER",
        &authored,
        &[1],
        false,
    )
    .expect("placed coloured world name");
    let yellow = viewer_native_gui::native_text::packed([1.0, 204.0 / 255.0, 0.0, 1.0]);
    let green =
        viewer_native_gui::native_text::packed([51.0 / 255.0, 204.0 / 255.0, 77.0 / 255.0, 1.0]);
    let expected = vec![[1.0; 4], yellow, green, yellow, [1.0; 4]];
    assert_eq!(placed.glyph_count(), 5);
    assert_eq!(placed.colors, expected);

    let (chunks, _) = static_names(
        &Placements {
            country: vec![placed],
            generation: 1,
            ..Placements::default()
        },
        u64::MAX,
    );
    let vertex_colors: Vec<[f32; 4]> = chunks[0]
        .chunks_exact(6)
        .map(|quad| quad[0].color)
        .collect();
    assert_eq!(vertex_colors, expected);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stock_signature_mapname_shader_renders_authored_color_run_pixels() {
    use eframe::wgpu::util::DeviceExt;
    use shader_pipeline::{Compiled, ScalarKind, Shader, VertexInput, gpu, library::Effect};
    use viewer_gpu::gpu_contract::{DEPTH_FORMAT, read_texture};
    content_reader_script::test_game::enter();
    let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
    let source = Arc::new(
        content_reader_gui::pdxshader::program::read_with(b"mapname.shader", &|path| {
            Some(if path == b"mapname.shader" {
                b"BlendState BlendState { BlendEnable = yes SourceBlend = src_alpha DestBlend = inv_src_alpha }\n\
                  DepthStencilState DepthStencilState { DepthEnable = no DepthWriteMask = depth_write_zero StencilEnable = no }\n\
                  Effect mapname { DepthStencilState = \"DepthStencilState\" }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap(),
    );
    let vertex = Shader {
        wgsl: "struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };\n\
               @vertex fn main(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @builtin(vertex_index) vid: u32) -> Out {\n\
                   var out: Out;\n\
                   out.pos = vec4<f32>(p.x, p.z, 0.5, 1.0);\n\
                   out.uv = vec2<f32>(uv.x, select(1.0, 0.5, vid >= 12u));\n\
                   return out;\n\
               }"
            .into(),
        independent_draws: true,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: vec![
            VertexInput {
                kind: ScalarKind::Float,
                location: 0,
                semantic: "POSITION".into(),
                components: 3,
            },
            VertexInput {
                kind: ScalarKind::Float,
                location: 1,
                semantic: "TEXCOORD0".into(),
                components: 2,
            },
        ],
        clock: Default::default(),
    };
    let pixel = Shader {
        wgsl: "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {\n\
                   return vec4<f32>(1.0, uv.y, 1.0, 1.0 + uv.x * 0.0);\n\
               }"
            .into(),
        independent_draws: true,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: Default::default(),
    };
    let effect = Effect::from_compiled(
        source,
        Vec::new(),
        vec![Ok(Arc::new(Compiled { vertex, pixel }))],
    );
    let format = eframe::wgpu::TextureFormat::Rgba8Unorm;
    let compiled = super::super::name_shader::Names::with_technique(
        &device,
        &effect,
        1,
        format,
        DEPTH_FORMAT,
        Some("mapname"),
    )
    .unwrap();
    let green = [0.2, 0.8, 0.3, 1.0];
    let quad = |x0: f32, x1: f32, color: [f32; 4]| {
        let v = |x: f32, z: f32| NameVertex {
            position: [x, 0.0, z],
            uv: [0.5, 0.5],
            color,
        };
        [
            v(x0, -0.5),
            v(x1, -0.5),
            v(x1, 0.5),
            v(x0, -0.5),
            v(x1, 0.5),
            v(x0, 0.5),
        ]
    };
    let mut vertices = Vec::new();
    vertices.extend(quad(-0.9, -0.4, [1.0, 1.0, 1.0, 1.0]));
    vertices.extend(quad(-0.25, 0.25, green));
    vertices.extend(quad(0.4, 0.9, [1.0, 1.0, 1.0, 1.0]));
    let buffer = device.create_buffer_init(&eframe::wgpu::util::BufferInitDescriptor {
        label: Some("stock mapname tint test"),
        contents: bytemuck::cast_slice(&vertices),
        usage: eframe::wgpu::BufferUsages::VERTEX,
    });
    let page_tex = device.create_texture(&eframe::wgpu::TextureDescriptor {
        label: None,
        size: eframe::wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: eframe::wgpu::TextureDimension::D2,
        format: eframe::wgpu::TextureFormat::Rgba8Unorm,
        usage: eframe::wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let page = gpu::TextureBinding::from(page_tex.create_view(&Default::default()));
    let arena = gpu::Arena::new(&device);
    let binding = compiled.bindings(&arena, &[], [1.0, 0.0], &page).unwrap();
    let target_size = eframe::wgpu::Extent3d {
        width: 64,
        height: 16,
        depth_or_array_layers: 1,
    };
    let color_tex = device.create_texture(&eframe::wgpu::TextureDescriptor {
        label: None,
        size: target_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: eframe::wgpu::TextureDimension::D2,
        format,
        usage: eframe::wgpu::TextureUsages::RENDER_ATTACHMENT | eframe::wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth_tex = device.create_texture(&eframe::wgpu::TextureDescriptor {
        label: None,
        size: target_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: eframe::wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: eframe::wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&Default::default());
    let depth_view = depth_tex.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&eframe::wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(eframe::wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: eframe::wgpu::Operations {
                    load: eframe::wgpu::LoadOp::Clear(eframe::wgpu::Color::BLACK),
                    store: eframe::wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(eframe::wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(eframe::wgpu::Operations {
                    load: eframe::wgpu::LoadOp::Clear(1.0),
                    store: eframe::wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(eframe::wgpu::Operations {
                    load: eframe::wgpu::LoadOp::Clear(0),
                    store: eframe::wgpu::StoreOp::Store,
                }),
            }),
            ..Default::default()
        });
        compiled.begin(&mut pass);
        binding.bind_after(&mut pass, None);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..vertices.len() as u32, 0..1);
    }
    queue.submit([encoder.finish()]);
    let raw = read_texture(&device, &queue, &color_tex, 4).unwrap();
    let pixel_rgb = |x: usize, y: usize| -> [f32; 3] {
        let at = (y * 64 + x) * 4;
        [0, 1, 2].map(|c| f32::from(raw[at + c]) / 255.0)
    };
    let left = pixel_rgb(10, 8);
    let mid = pixel_rgb(32, 8);
    let right = pixel_rgb(54, 8);
    assert!(
        (left[0] - 1.0).abs() < 0.05 && (left[1] - 1.0).abs() < 0.05 && (left[2] - 1.0).abs() < 0.05,
        "left base glyph should render white, got {left:?}"
    );
    assert!(
        (mid[0] - green[0]).abs() < 0.05
            && (mid[1] - green[1]).abs() < 0.05
            && (mid[2] - green[2]).abs() < 0.05,
        "middle colour-run glyph should render green {green:?}, got {mid:?}"
    );
    assert!(
        (right[0] - 1.0).abs() < 0.05
            && (right[1] - 0.5).abs() < 0.05
            && (right[2] - 1.0).abs() < 0.05,
        "right reset glyph with vertex_index >= 12 should render [1.0, 0.5, 1.0], got {right:?}"
    );
}
