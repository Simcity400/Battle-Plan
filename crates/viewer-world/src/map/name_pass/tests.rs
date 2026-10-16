use super::*;
use std::cell::Cell;
use viewer_foundation::map::camera::MapDefines;

const W: f32 = 5632.0;
use installed_content::adjacencies::{Adjacency, IMPASSABLE, SEA_CROSSING};

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
    // Three columns, two rows: 1 2 3 / 1 4 3.
    let ids = [1, 2, 3, 1, 4, 3];
    let graph = ProvinceGraph::build(3, 2, &ids, &[]).unwrap();
    // Column 2 wraps onto column 0, so 3 and 1 are neighbours.
    assert_eq!(graph.components(&members(&[1, 3])), vec![vec![1, 3]]);
    // 2 and 4 touch vertically; 2 and 3 touch horizontally.
    assert_eq!(graph.components(&members(&[2, 4])), vec![vec![2, 4]]);
    assert_eq!(graph.components(&members(&[2, 3])), vec![vec![2, 3]]);
}

/// A four-province crossing joins its diagonal; province zero has no neighbours.
#[test]
fn raster_records_include_the_four_province_diagonal() {
    // Rows north first: 1 2 / 3 4 with a sea column; 2 and 3 meet only
    // at the crossing.
    let ids = [1, 2, 9, 3, 4, 9];
    let graph = ProvinceGraph::build(3, 2, &ids, &[]).unwrap();
    assert_eq!(graph.components(&members(&[2, 3])), vec![vec![2, 3]]);
    assert_eq!(graph.components(&members(&[1, 4])), vec![vec![1], vec![4]]);
    let zero = ProvinceGraph::build(2, 1, &[0, 5], &[]).unwrap();
    assert_eq!(zero.edge_count(), 0);
}

#[test]
fn separated_islands_split_until_a_sea_link_joins_them() {
    // 1 0 2 0: two provinces with sea on both sides of each, the way
    // Denmark's islands sit. The trailing sea column absorbs the x wrap.
    let ids = [1, 0, 2, 0, 1, 0, 2, 0];
    let graph = ProvinceGraph::build(4, 2, &ids, &[]).unwrap();
    assert_eq!(graph.components(&members(&[1, 2])), vec![vec![1], vec![2]]);
    let linked = ProvinceGraph::build(4, 2, &ids, &[link(1, 2, SEA_CROSSING)]).unwrap();
    assert_eq!(linked.components(&members(&[1, 2])), vec![vec![1, 2]]);
    // A strait row with an empty type column counts too.
    let rule = ProvinceGraph::build(4, 2, &ids, &[link(1, 2, 0)]).unwrap();
    assert_eq!(rule.components(&members(&[1, 2])), vec![vec![1, 2]]);
    // An `impassable` row is a neighbour record too, which the walk
    // follows (BlackICE's 4391;12526).
    let impassable = ProvinceGraph::build(4, 2, &ids, &[link(1, 2, IMPASSABLE)]).unwrap();
    assert_eq!(impassable.components(&members(&[1, 2])), vec![vec![1, 2]]);
}

#[test]
fn a_component_needs_a_path_inside_the_member_set() {
    // 1 2 3 in a row, with the wrap disabled by a fourth column of zeros.
    let ids = [1, 2, 3, 0];
    let graph = ProvinceGraph::build(4, 1, &ids, &[]).unwrap();
    // 1 and 3 only touch through 2, which is not a member here.
    assert_eq!(graph.components(&members(&[1, 3])), vec![vec![1], vec![3]]);
    assert_eq!(graph.components(&members(&[1, 2, 3])), vec![vec![1, 2, 3]]);
    // The order is by smallest member, so a headless render is stable.
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
        indices: vec![0, 200, 95, 150],
        palette: [[0; 4]; 256],
    };
    assert_eq!(height_at(&heightmap, 0.5, -0.5), WATER_HEIGHT);
    assert_eq!(height_at(&heightmap, 1.5, -0.5), 20.0);
    assert_eq!(height_at(&heightmap, 0.5, 0.5), WATER_HEIGHT);
    assert_eq!(height_at(&heightmap, 1.5, 0.5), 15.0);
    assert!((height_at(&heightmap, 1.0, 0.0) - 11.125).abs() < 1e-5);
}

/// A synthetic placed name of `glyphs` glyphs at world `(x, z)`.
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
        key,
        text: String::new(),
        pages: vec![0; glyphs],
        corners,
        uvs,
        aabb: [x, z, x + glyphs as f32 * 4.0, z + 4.0],
        wrapped,
        font_size: 1.0,
        spaces: 0,
        alpha: 1.0,
    }
}

/// 15,000 country names on a grid across the whole map.
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
        pose: viewer_foundation::map::camera::Pose::Map,
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
    // Arrow admission owns the view and order, even when glyph bounds
    // lie outside the camera and the name has a wrapped country box.
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
    let open = viewer_foundation::map::province_admission::OPEN;
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let placements = many_names(15_000);
    let (chunks, names) = static_names(&placements, u64::MAX);
    assert_eq!(chunks.len(), 1);
    assert_eq!(names.len(), 15_000);
    assert_eq!(chunks[0].len(), 15_000 * 12 * 6);
    // The names tile the chunk in placement order.
    assert!(
        names
            .windows(2)
            .all(|pair| pair[0].vertices.end == pair[1].vertices.start)
    );
    // A small device limit splits between names, never inside one.
    let limit = 72 * 1000 + 50;
    let (chunks, names) = static_names(&placements, limit);
    assert_eq!(chunks.len(), 15);
    assert!(chunks.iter().all(|chunk| chunk.len() as u64 <= limit));
    let total: usize = chunks.iter().map(Vec::len).sum();
    assert_eq!(total, 15_000 * 72);
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
        pose: viewer_foundation::map::camera::Pose::Map,
    };
    assert_eq!(LayerFade::instant(at(260.0)), LayerFade(1.0));
    let low = LayerFade::instant(at(259.9));
    assert_eq!(low, LayerFade(0.0));
    let half = low.step(at(310.0), 0.5);
    assert_eq!(half, LayerFade(0.5));
    let settled = half.step(at(310.0), 0.5);
    assert_eq!(settled, LayerFade(1.0));
    // A negative delta never walks backwards.
    assert_eq!(settled.step(at(310.0), -1.0), settled);
}

fn placed_name(text: &str, glyphs: usize) -> WorldName {
    WorldName {
        key: "GER".into(),
        text: text.into(),
        pages: vec![0; glyphs],
        corners: vec![[0.0; 3]; glyphs * 4],
        uvs: vec![[0.0; 4]; glyphs],
        aabb: [0.0; 4],
        wrapped: false,
        font_size: 1.0,
        spaces: 0,
        alpha: 1.0,
    }
}

#[test]
fn a_placed_area_is_reused_until_the_byte_bound_evicts_the_oldest() {
    let key = |area: u32| ("GER".to_owned(), "Germany".to_owned(), vec![area]);
    let one = area_bytes(&key(1), Some(&placed_name("Germany", 7)));
    // Room for three areas; an overflow evicts down to three quarters.
    let placed = PlacedAreas::with_limit(one * 3);
    let builds = Cell::new(0);
    let place = |area| {
        let generation = placed.begin();
        placed.place(generation, key(area), || {
            builds.set(builds.get() + 1);
            Some(placed_name("Germany", 7))
        })
    };
    place(2);
    place(1);
    assert!(place(1).is_some());
    assert_eq!(builds.get(), 2, "an area is placed once");
    place(3);
    // Area 1 is used again, then a fourth area overflows the bound:
    // the least recently used areas, 2 then 3, go.
    place(1);
    place(4);
    assert_eq!(builds.get(), 4);
    assert!(placed.lock().bytes <= one * 3);
    place(1);
    place(4);
    assert_eq!(builds.get(), 4, "the recently used areas were kept");
    place(2);
    place(3);
    assert_eq!(
        builds.get(),
        6,
        "the least recently used areas were evicted"
    );
}

fn name_frame(controllers: [u32; 2]) -> DecodedFrame {
    use replay::{
        frame::{Metrics, TableView},
        rows::{CountryLinkRow, CountryRow, ProvinceRow, TableId, TableRows},
    };
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
                    installed_content::map::ProvinceDefinition {
                        id,
                        rgb: [0; 3],
                        kind: installed_content::map::ProvinceKind::Land,
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
    // SOV takes province 2: both countries' areas change.
    assert_ne!(areas([100, 100]), areas([100, 101]));
    // In the factions mode, GER and SOV in one faction form one area under its name; a
    // country outside every faction keeps its own.
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
    // Group tables the recording lacks withhold the mode's names, saying why, rather than
    // writing the countries'.
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
    // In the players mode, a country no player holds carries no name.
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
    use replay::frame::TableView;
    use replay::rows::{StateRow, TableId, TableRows};
    let map = MapData {
        width: 3,
        height: 1,
        province_ids: vec![1, 2, 3],
        definitions: [1, 2, 3]
            .map(|id| {
                (
                    id,
                    installed_content::map::ProvinceDefinition {
                        id,
                        rgb: [0; 3],
                        kind: installed_content::map::ProvinceKind::Land,
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
        TableView::Complete(TableRows::Provinces(provinces.into())).into();
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
    // A state with no province on the map has no area.
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
    // An unnamed state carries no name rather than a tag.
    assert!(states.countries.iter().all(|area| area.faction));
    let localisation = Localisation::default();
    let far = NameAreas::read(&map, &localisation, &frame, None).unwrap();
    let both = far.clone().with_near(states.clone());
    assert_eq!(both.near.as_deref(), Some(&states));
    assert_ne!(both, far);
}
