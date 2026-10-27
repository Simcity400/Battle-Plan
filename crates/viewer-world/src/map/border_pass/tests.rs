use super::*;
use installed_content_core::{adjacencies::SEA_CROSSING, map::CountryColors, map::ProvinceDefinition};

#[test]
fn faction_hover_color_uses_the_content_index_and_refuses_missing_quads() {
    let defines = installed_content_core::defines::Defines::new().unwrap();
    defines
        .load(
            "fixture",
            br#"NDefines = {
        NMapMode = { FACTION_THEATER_HIGHLIGHT_COLOR_INDEX = 1 },
        NGraphics = { BORDER_COLOR_CUSTOM_HIGHLIGHTS = {
            0.1, 0.2, 0.3, 0.4, 0.7, 0.6, 0.5, 0.9
        } }
    }"#,
        )
        .unwrap();
    assert_eq!(
        faction_highlight_color(&defines).unwrap(),
        [0.7, 0.6, 0.5, 0.9]
    );
    for index in [-1, 2, 246] {
        defines
            .load(
                "fixture index",
                format!("NDefines.NMapMode.FACTION_THEATER_HIGHLIGHT_COLOR_INDEX = {index}")
                    .as_bytes(),
            )
            .unwrap();
        assert!(faction_highlight_color(&defines).is_err());
    }
}

/// Six provinces: 1 and 2 land, 3 impassable land, 4 and 5 sea in different regions, 6 sea with
/// 4.
fn map() -> MapData {
    let kinds = [
        (1, ProvinceKind::Land),
        (2, ProvinceKind::Land),
        (3, ProvinceKind::Land),
        (4, ProvinceKind::Sea),
        (5, ProvinceKind::Sea),
        (6, ProvinceKind::Lake),
    ];
    MapData {
        width: 6,
        height: 1,
        province_ids: vec![1, 2, 3, 4, 5, 6],
        definitions: kinds
            .into_iter()
            .map(|(id, kind)| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind,
                        continent: 0,
                    },
                )
            })
            .collect(),
        country_colors: CountryColors::default(),
    }
}

fn state(id: u32, provinces: &[u32], impassable: bool) -> (u32, StateDefinition) {
    (
        id,
        StateDefinition {
            id,
            impassable,
            provinces: provinces.to_vec(),
            ..Default::default()
        },
    )
}

fn region(id: u32, provinces: &[u32]) -> (u32, StrategicRegion) {
    (
        id,
        StrategicRegion {
            id,
            name_key: format!("STRATEGICREGION_{id}"),
            provinces: provinces.to_vec(),
            naval_terrain: None,
            static_modifiers: false,
            static_modifier_names: Vec::new(),
            ..Default::default()
        },
    )
}

fn topology_of(adjacencies: &[Adjacency]) -> Topology {
    Topology::new(
        &map(),
        &[state(1, &[1, 2], false), state(2, &[3], true)]
            .into_iter()
            .collect(),
        &[region(10, &[1, 2, 3, 4, 6]), region(11, &[5])]
            .into_iter()
            .collect(),
        adjacencies,
    )
    .unwrap()
}

fn adjacency(from: u32, to: u32, kind: u8) -> Adjacency {
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

#[test]
fn water_edges_take_the_sea_classes_and_a_coast_takes_none() {
    let topology = topology_of(&[]);
    // Two sea provinces in one strategic region.
    assert_eq!(topology.static_class(4, 6), Some(BorderClass::Sea));
    // Two sea provinces in different strategic regions.
    assert_eq!(topology.static_class(4, 5), Some(BorderClass::SeaRegion));
    // A coast has no material: 0xff suppresses its row.
    assert_eq!(topology.static_class(1, 4), None);
    assert_eq!(topology.static_class(3, 5), None);
    // And it must not fall into the land bucket either, or the hour would give it a class.
    assert_eq!(topology.trace_class(1, 4), None);
    assert_eq!(topology.trace_class(3, 5), None);
    // An edge with itself is no edge at all.
    assert_eq!(topology.static_class(4, 4), None);
    assert_eq!(topology.trace_class(2, 2), None);
}

#[test]
fn impassable_comes_from_the_state_flag_and_from_an_adjacency_row() {
    let topology = topology_of(&[]);
    // Province 3 is in an impassable state and 2 is not.
    assert_eq!(topology.static_class(2, 3), Some(BorderClass::Impassable));
    // Two ordinary land provinces are the hour's business.
    assert_eq!(topology.static_class(1, 2), None);
    assert_eq!(topology.trace_class(1, 2), Some(BorderClass::Province));
    // An `impassable` adjacency row names the pair itself, in either order.
    let rows = [adjacency(2, 1, IMPASSABLE)];
    assert_eq!(
        topology_of(&rows).static_class(1, 2),
        Some(BorderClass::Impassable)
    );
    // A `sea` row is a strait, not an impassable edge.
    let rows = [adjacency(1, 2, SEA_CROSSING)];
    assert_eq!(topology_of(&rows).static_class(1, 2), None);
    // The last row naming a pair sets its kind on both records.
    let rows = [adjacency(1, 2, IMPASSABLE), adjacency(2, 1, SEA_CROSSING)];
    assert_eq!(topology_of(&rows).static_class(1, 2), None);
    let rows = [adjacency(1, 2, SEA_CROSSING), adjacency(2, 1, IMPASSABLE)];
    assert_eq!(
        topology_of(&rows).static_class(1, 2),
        Some(BorderClass::Impassable)
    );
}

#[test]
fn isolated_land_is_hidden_before_authored_adjacencies_are_loaded() {
    let mut map = map();
    map.province_ids = vec![1, 4, 2, 5, 3, 6];
    assert_eq!(
        hidden_provinces(&map, 7),
        [false, true, true, true, false, false, false]
    );
    for kind in [SEA_CROSSING, IMPASSABLE] {
        let topology = Topology::new(
            &map,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[adjacency(1, 2, kind)],
        )
        .unwrap();
        assert_eq!(topology.trace_class(1, 2), None);
        assert_eq!(topology.trace_class(1, 4), None);
        assert_eq!(topology.trace_class(4, 5), Some(BorderClass::Sea));
    }
    // Horizontal wrapping counts as land adjacency; a sea override cannot erase it.
    map.province_ids = vec![1, 4, 5, 6, 3, 2];
    assert!(!hidden_provinces(&map, 7)[1]);
    map.width = 2;
    map.height = 2;
    map.province_ids = vec![4, 1, 2, 5];
    let hidden = hidden_provinces(&map, 7);
    assert!(!hidden[1] && !hidden[2]);
}

/// Provinces 1 and 2 in state 7 under country 1; 3 in state 8 under country 1; 4 under
/// country 2.
fn regions() -> Regions {
    Regions {
        country_of: vec![0, 2, 1, 2, 2, 0].into(),
        owner_of: vec![0, 1, 1, 1, 2, 0].into(),
        state_of: vec![0, 7, 7, 8, 9, 0].into(),
        country_colors: Default::default(),
        warnings: Vec::new(),
        strategic_attributes: Default::default(),
        mode: None,
    }
}

#[test]
fn the_land_class_is_country_then_state_then_province() {
    let regions = regions();
    assert_eq!(land_class(&regions, 1, 2), BorderClass::Province);
    assert_eq!(land_class(&regions, 2, 3), BorderClass::State);
    assert_eq!(land_class(&regions, 3, 4), BorderClass::Country);
    // An unowned province meets an owned one as a country border.
    assert_eq!(land_class(&regions, 1, 5), BorderClass::Country);
    // A province id past the tables is unowned rather than a panic.
    assert_eq!(land_class(&regions, 1, 900), BorderClass::Country);
}

#[test]
fn the_quad_pattern_matches_the_engine_and_offsets_by_the_base_vertex() {
    let mut indices = Vec::new();
    run_indices(0, 3, &mut indices);
    assert_eq!(indices, [0, 2, 1, 2, 3, 1, 2, 4, 3, 4, 5, 3]);
    let mut offset = Vec::new();
    run_indices(10, 2, &mut offset);
    assert_eq!(offset, [10, 12, 11, 12, 13, 11]);
    // A single point is not a segment.
    let mut none = Vec::new();
    run_indices(0, 1, &mut none);
    run_indices(0, 0, &mut none);
    assert!(none.is_empty());
}

/// Every test row joins provinces 1..8, which this calls land; 9 is the sea
/// the crossings pass through.
fn land(province: u32) -> bool {
    province != 9
}

fn strait(from: u32, to: u32, start: (i32, i32), stop: (i32, i32)) -> Adjacency {
    Adjacency {
        from,
        to,
        kind: SEA_CROSSING,
        through: 9,
        start,
        stop,
        rule: None,
    }
}

/// The four corners, the two texture axes and the whole tile count the native
/// builder writes, on a run that is not a whole number of tiles.
#[test]
fn a_strait_becomes_one_quad_on_the_water_plane_tiled_a_whole_number_of_times() {
    // A twelve-unit horizontal run is exactly three four-unit tiles.
    let rows = [strait(1, 2, (10, 4), (22, 4))];
    let quad = strait_quads(&rows, &grid(1, &[&[1]]), &land).unwrap();
    assert_eq!(quad.len(), 6);
    let xs = quad.iter().map(|v| v.position[0]).collect::<Vec<_>>();
    let zs = quad.iter().map(|v| v.position[2]).collect::<Vec<_>>();
    assert_eq!(xs.iter().cloned().fold(f32::MAX, f32::min), 10.0);
    assert_eq!(xs.iter().cloned().fold(f32::MIN, f32::max), 22.0);
    let (low, high) = (
        zs.iter().cloned().fold(f32::MAX, f32::min),
        zs.iter().cloned().fold(f32::MIN, f32::max),
    );
    // The corners sit half a width either side of the line, so the ribbon is one
    // width across and stays centred on the endpoints' own z.
    assert!((high - low - STRAIT_WIDTH).abs() < 1e-4, "{low} .. {high}");
    assert!(((low + high) / 2.0 - 4.0).abs() < 1e-4);
    // `mirror_plane_height` is a constant: the corners never follow the terrain.
    assert!(quad.iter().all(|v| v.position[1] == WATER_HEIGHT));
    // `u` crosses the ribbon and `v` counts whole tiles along it.
    let vs = quad.iter().map(|v| v.uv[1]).collect::<Vec<_>>();
    assert_eq!(vs.iter().cloned().fold(f32::MIN, f32::max), 3.0);
    assert!(quad.iter().all(|v| v.uv[0] == 0.0 || v.uv[0] == 1.0));
    // A run shorter than one tile still carries a whole one.
    let short = strait_quads(&[strait(1, 2, (10, 4), (11, 4))], &grid(1, &[&[1]]), &land).unwrap();
    assert_eq!(short.iter().map(|v| v.uv[1]).fold(f32::MIN, f32::max), 1.0);
}

/// A map from rows written north first, with 9 as the sea and every other id land.
fn grid(width: u32, rows: &[&[u32]]) -> MapData {
    MapData {
        width,
        height: rows.len() as u32,
        province_ids: rows.iter().flat_map(|row| row.iter().copied()).collect(),
        definitions: Default::default(),
        country_colors: CountryColors::default(),
    }
}

/// The engine keeps a record with a non-zero through whose (higher-id) province is land, which
/// is what separates a strait from a canal.
#[test]
fn records_with_a_through_on_land_become_quads() {
    let map = grid(1, &[&[1]]);
    let mut impassable = strait(3, 4, (1, 1), (3, 3));
    impassable.kind = IMPASSABLE;
    impassable.through = -1;
    let rows = [
        strait(1, 2, (1, 1), (3, 3)),
        impassable,
        adjacency(5, 6, 0),
        // A zero-length ribbon has no direction and is skipped.
        strait(7, 8, (5, 5), (5, 5)),
    ];
    assert_eq!(strait_quads(&rows, &map, &land).unwrap().len(), 12);
    // The Kiel and Panama canals join two sea provinces and carry no dashes.
    let canal = [strait(9, 9, (1, 1), (3, 3))];
    assert!(strait_quads(&canal, &map, &land).unwrap().is_empty());
    // Nor does a crossing visited from a sea province.
    let coastal = [strait(1, 9, (1, 1), (3, 3))];
    assert!(strait_quads(&coastal, &map, &land).unwrap().is_empty());
    // An empty crossing kind still draws when the through value is nonzero.
    let mut rule_only = strait(1, 2, (1, 1), (3, 3));
    rule_only.kind = 0;
    assert_eq!(strait_quads(&[rule_only], &map, &land).unwrap().len(), 6);
    // A `sea` row with an empty `through` stores 0, which the builder skips.
    let mut unset = strait(1, 2, (1, 1), (3, 3));
    unset.through = 0;
    assert!(strait_quads(&[unset], &map, &land).unwrap().is_empty());
}

/// An end the csv leaves out is the province's nearest pixel to the other, found by the native
/// scan order, and the higher-id province's end is `A`.
#[test]
fn a_missing_end_is_the_nearest_pixel_and_the_higher_id_end_leads() {
    // North first: province 2 west, 5 east, sea between. World z counts up from the bottom.
    // The last column of sea keeps 5 from touching 2 across the wrap.
    let map = grid(
        8,
        &[
            &[2, 2, 9, 9, 9, 5, 5, 9],
            &[2, 2, 9, 9, 9, 9, 5, 9],
            &[2, 9, 9, 9, 9, 9, 5, 9],
        ],
    );
    let mut derived = strait(2, 5, (0, 0), (0, 0));
    derived.start = (-1, -1);
    derived.stop = (0, 0);
    let quad = strait_quads(&[derived], &map, &land).unwrap();
    assert_eq!(quad.len(), 6);
    // The nearest pair is (1, 2) in province 2 and (5, 2) in province 5; `A` is 5's end.
    let (a0, a1) = (quad[0].position, quad[1].position);
    let centre = [(a0[0] + a1[0]) / 2.0, (a0[2] + a1[2]) / 2.0];
    assert_eq!(centre, [5.0, 2.0]);
    let b = [(quad[5].position[0] + quad[2].position[0]) / 2.0];
    assert_eq!(b, [1.0]);
    // One end given keeps it and derives only the other.
    let mut half = strait(2, 5, (0, 0), (6, 0));
    half.start = (-1, 5);
    let quad = strait_quads(&[half], &map, &land).unwrap();
    let (a0, a1) = (quad[0].position, quad[1].position);
    assert_eq!([(a0[0] + a1[0]) / 2.0, (a0[2] + a1[2]) / 2.0], [6.0, 0.0]);
}

/// For a pair the bitmap already made neighbours the loader updates the existing records with
/// the ends swapped, so the higher-id province leads from the csv's `start`.
#[test]
fn touching_provinces_take_their_csv_ends_the_other_way_round() {
    let apart = grid(4, &[&[2, 9, 5, 9]]);
    let touching = grid(2, &[&[2, 5]]);
    let row = [strait(2, 5, (10, 0), (20, 0))];
    let lead = |map: &MapData| {
        let quad = strait_quads(&row, map, &land).unwrap();
        (quad[0].position[0] + quad[1].position[0]) / 2.0
    };
    // Province 5, the higher id, leads: from `stop` when apart, from `start` when touching.
    assert_eq!(lead(&apart), 20.0);
    assert_eq!(lead(&touching), 10.0);
    // The raster wraps in x, so 2 and 5 touch across the map's edge.
    assert_eq!(lead(&grid(3, &[&[2, 9, 5]])), 10.0);
}

/// Two raster neighbours of one province keep their raster order whatever the order of the
/// rows that update them (vanilla 3675's 671 and 668, adjacencies.csv lines 211 and 212).
#[test]
fn updated_raster_records_keep_their_raster_order() {
    // The raster lists 2 before 3 among 5's neighbours; the csv names 3 first.
    let map = grid(4, &[&[2, 5, 3, 9]]);
    let rows = [
        strait(3, 5, (30, 0), (31, 0)),
        strait(2, 5, (20, 0), (21, 0)),
    ];
    let quad = strait_quads(&rows, &map, &land).unwrap();
    let lead = |at: usize| (quad[at].position[0] + quad[at + 1].position[0]) / 2.0;
    // Each touching pair's record of 5 takes its row's `start`.
    assert_eq!((lead(0), lead(6)), (20.0, 30.0));
}

/// A repeated row (Makeshift Bridges' 3898;3915 on lines 296 and 388) updates the pair its
/// first row made: one ribbon, with the ends the other way round, as for a bitmap neighbour.
#[test]
fn a_repeated_row_updates_one_pair_of_records() {
    let apart = grid(4, &[&[2, 9, 5, 9]]);
    let rows = [
        strait(2, 5, (10, 0), (20, 0)),
        strait(2, 5, (30, 0), (40, 0)),
    ];
    let quad = strait_quads(&rows, &apart, &land).unwrap();
    assert_eq!(quad.len(), 6);
    assert_eq!((quad[0].position[0] + quad[1].position[0]) / 2.0, 30.0);
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
    // A view wider than the map needs both.
    assert_eq!(
        wrap_offsets(Some([-100.0, 0.0, 5700.0, 100.0]), 5632.0),
        [0.0, -5632.0, 5632.0]
    );
}

/// A free eye's reach can run several map widths: every copy it overlaps draws, its own first,
/// nearer copies before farther ones and west before east, matching the game camera's order.
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
    // An eye on a later copy still lists the first copy and the ones it sees.
    assert_eq!(
        free_wrap_offsets(Some([6000.0, 0.0, 12_000.0, 100.0]), width),
        [0.0, width, 2.0 * width]
    );
}
