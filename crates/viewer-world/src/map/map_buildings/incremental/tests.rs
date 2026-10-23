use super::*;
use installed_content_core::buildings;
use replay_data::rows::{
    CountryLinkRow, FloatingHarborDeploymentRow, LexicalTokenRow, LocalPlayerRow, NavalBaseRow,
    ProvinceRow, ProvinceSupplyRow, RaidScope, RaidTargetStatusRow, StateRow, TABLE_COUNT,
    TableRows,
};
use replay_frame::frame::{Metrics, TableView};
use viewer_foundation::map::perspective::{Choice, Country};

// Both factory types redirect to one declared bucket, in declaration order.
const TYPES: &[u8] = br#"
buildings = {
    arms_factory = { spawn_point = factory_spawn show_on_map = 6 level_cap = { state_max = 20 } }
    industrial_complex = { spawn_point = factory_spawn show_on_map = 6 level_cap = { state_max = 20 } }
    bunker = { show_on_map = 1 need_detection = yes level_cap = { province_max = 10 } }
    supply_node = { show_on_map = 1 always_shown = yes supply_node = yes level_cap = { province_max = 1 } }
}
spawn_points = {
    factory_spawn = { type = state max = 6 }
}
"#;
const FACTORY: &str = "factory_spawn";
const ARMS: u32 = 77;
const COMPLEX: u32 = 78;
const BUNKER: u32 = 79;
const HUB: u32 = 80;
const WIDE: u32 = 600;
const SEA: u32 = 14;

fn assets() -> MapBuildingAssets {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let types = buildings::parse(TYPES, &installed_content_core::key_names::KeyNames::default()).unwrap();
    let site = |state, province: u32| BuildingPosition {
        state,
        kind: String::new(),
        x: province as f32,
        y: 0.0,
        z: 1.0,
        rotation: 0.0,
        adjacent_sea: 0,
    };
    let slots = BTreeMap::from([
        (
            (7, FACTORY.to_owned()),
            (0..WIDE)
                .map(|at| (site(7, 1000 + at), Some(1000 + at)))
                .collect(),
        ),
        (
            (8, FACTORY.to_owned()),
            vec![(site(8, 12), Some(12)), (site(8, 13), Some(13))],
        ),
        ((9, "bunker".to_owned()), vec![(site(7, 9), Some(9))]),
        (
            (10, "supply_node".to_owned()),
            vec![(site(7, 10), Some(10))],
        ),
        (
            (11, "supply_node".to_owned()),
            vec![(site(7, 11), Some(11))],
        ),
        ((12, "bunker".to_owned()), vec![(site(8, 12), Some(12))]),
    ]);
    let mut assets = MapBuildingAssets::assemble(
        ModelLibrary::default(),
        &types,
        Defines {
            shrink_distance: 180.0,
            destroyed: 6_000_000,
            recovered: 8_000_000,
            damaged: 80_000,
            max_levels: 15,
            popup_speed: 5.0,
        },
        slot_groups(std::iter::empty(), &BTreeMap::new()),
        slots,
        BTreeMap::from([
            (("arms_factory".to_owned(), 0), 0),
            (("bunker".to_owned(), 0), 2),
            (("supply_node".to_owned(), 0), 3),
        ]),
        BTreeMap::new(),
    );
    assets.floating = vec![(
        BuildingPosition {
            adjacent_sea: SEA,
            ..site(8, SEA)
        },
        SEA,
    )];
    assets.floating_mesh = Some(4);
    let factory: Vec<&str> = assets.candidates[FACTORY]
        .iter()
        .map(|&index| assets.facts.ordered[index].name.as_str())
        .collect();
    assert_eq!(
        factory,
        ["arms_factory", "industrial_complex"],
        "both factory types are candidates of the factory bucket, in declaration order"
    );
    assets
}

fn terrain(cities: bool) -> CityTerrain<'static> {
    static CITIES: [bool; 64] = [true; 64];
    static NONE: [bool; 64] = [false; 64];
    CityTerrain {
        width: 16,
        height: 4,
        spawn_city: if cities { &CITIES } else { &NONE },
    }
}

fn building(
    location: u32,
    building_type: u32,
    level: i16,
    healthy: i16,
    flags: u32,
) -> BuildingRow {
    BuildingRow {
        location,
        building_type,
        level,
        healthy_level: healthy,
        status_flags: flags,
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Held {
    Complete,
    Truncated,
    Unavailable,
}

impl Held {
    fn view(self, rows: TableRows) -> TableView {
        match self {
            Self::Complete => TableView::Complete(rows),
            Self::Truncated => TableView::Truncated { rows, missing: 1 },
            Self::Unavailable => TableView::Unavailable {
                reason: "not read this hour".into(),
            },
        }
    }
}

#[derive(Clone)]
struct Hour {
    branch: u64,
    hour: i32,
    tokens: Vec<(u32, &'static str)>,
    state_rows: Vec<BuildingRow>,
    state_held: Held,
    province_rows: Vec<BuildingRow>,
    province_held: Held,
    links: Vec<(u32, u32)>,
    player: u32,
    provinces: Vec<(u32, u32, u32)>,
    states: Vec<(u32, u32)>,
    raids: Vec<RaidTargetStatusRow>,
    raids_held: Held,
    ports: Vec<u32>,
    ports_held: Held,
    deployed: u8,
    cities: bool,
    fog: Option<Option<BTreeMap<u32, u8>>>,
    refused: bool,
}

fn raid(detected: u8) -> RaidTargetStatusRow {
    RaidTargetStatusRow {
        ordinal: 0,
        valid: 1,
        scope: RaidScope {
            building_type: BUNKER,
            building_location: 9,
            building_flags: 1,
            ..Default::default()
        },
        detected: vec![detected],
    }
}

fn bytes() -> BTreeMap<u32, u8> {
    let mut bytes: BTreeMap<u32, u8> = (1000..1000 + WIDE).map(|province| (province, 60)).collect();
    bytes.extend((9..14).map(|province| (province, 60)));
    bytes.insert(SEA, 200);
    bytes
}

impl Hour {
    fn first() -> Self {
        Self {
            branch: 1,
            hour: 100,
            tokens: vec![
                (ARMS, "arms_factory"),
                (COMPLEX, "industrial_complex"),
                (BUNKER, "bunker"),
                (HUB, "supply_node"),
            ],
            state_rows: vec![building(7, ARMS, 10, 10, 0), building(8, ARMS, 4, 4, 0)],
            state_held: Held::Complete,
            province_rows: vec![
                building(9, BUNKER, 1, 1, 1),
                building(10, HUB, 1, 1, 1),
                building(11, HUB, 1, 1, 1),
                building(12, BUNKER, 1, 1, 1),
            ],
            province_held: Held::Complete,
            links: vec![(100, 0), (101, 1)],
            player: 100,
            provinces: vec![
                (9, 7, 101),
                (10, 7, 100),
                (11, 7, 100),
                (12, 8, 100),
                (13, 8, 100),
            ],
            states: vec![(7, 100), (8, 100)],
            raids: vec![raid(0)],
            raids_held: Held::Complete,
            ports: vec![10],
            ports_held: Held::Complete,
            deployed: 1,
            cities: false,
            fog: Some(Some(bytes())),
            refused: false,
        }
    }

    fn bytes(&mut self) -> &mut BTreeMap<u32, u8> {
        self.fog
            .as_mut()
            .and_then(Option::as_mut)
            .expect("fog bytes")
    }

    fn frame(&self) -> DecodedFrame {
        let mut views: [TableView; TABLE_COUNT] = std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        });
        let mut set = |table: TableId, view: TableView| views[table.index()] = view;
        set(
            TableId::LexicalTokens,
            TableView::Complete(TableRows::LexicalTokens(
                self.tokens
                    .iter()
                    .map(|&(token, literal)| LexicalTokenRow {
                        token,
                        literal: literal.as_bytes().to_vec(),
                    })
                    .collect(),
            )),
        );
        set(
            TableId::StateBuildings,
            if self.refused {
                TableView::Complete(TableRows::Countries(Vec::new()))
            } else {
                self.state_held
                    .view(TableRows::StateBuildings(self.state_rows.clone()))
            },
        );
        set(
            TableId::ProvinceBuildings,
            self.province_held
                .view(TableRows::ProvinceBuildings(self.province_rows.clone())),
        );
        set(
            TableId::CountryLinks,
            TableView::Complete(TableRows::CountryLinks(
                self.links
                    .iter()
                    .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            )),
        );
        set(
            TableId::LocalPlayer,
            TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                country: self.player,
                ..Default::default()
            }])),
        );
        set(
            TableId::Provinces,
            TableView::Complete(TableRows::Provinces(
                self.provinces
                    .iter()
                    .map(|&(id, state, controller)| ProvinceRow {
                        id,
                        state,
                        controller,
                        ..Default::default()
                    })
                    .collect(),
            )),
        );
        set(
            TableId::States,
            TableView::Complete(TableRows::States(
                self.states
                    .iter()
                    .map(|&(id, controller)| StateRow {
                        id,
                        controller,
                        ..Default::default()
                    })
                    .collect(),
            )),
        );
        set(
            TableId::RaidTargetStatuses,
            self.raids_held
                .view(TableRows::RaidTargetStatuses(self.raids.clone())),
        );
        set(
            TableId::NavalBases,
            self.ports_held.view(TableRows::NavalBases(
                self.ports
                    .iter()
                    .map(|&province| NavalBaseRow {
                        province,
                        ..Default::default()
                    })
                    .collect(),
            )),
        );
        set(
            TableId::ProvinceSupply,
            TableView::Complete(TableRows::ProvinceSupply(vec![ProvinceSupplyRow {
                province: SEA,
                controller_index: 1,
                flags: 1,
                ..Default::default()
            }])),
        );
        set(
            TableId::FloatingHarborDeployments,
            TableView::Complete(TableRows::FloatingHarborDeployments(vec![
                FloatingHarborDeploymentRow {
                    country: 1,
                    ordinal: 0,
                    province: SEA as i32,
                    status: self.deployed,
                },
            ])),
        );
        DecodedFrame {
            branch: self.branch,
            hour: self.hour,
            tables: views.map(Into::into),
            metrics: Metrics::default(),
        }
    }

    fn vision(&self, frame: &DecodedFrame) -> Arc<Vision> {
        let mut vision = Vision::read(
            frame,
            &self.links.iter().copied().collect(),
            &BTreeMap::from([
                (0, "GER".to_owned()),
                (1, "ENG".to_owned()),
                (2, "ITA".to_owned()),
            ]),
            &Choice {
                country: Country::Recorded,
                fog: self.fog.is_some(),
            },
            Err("synthetic fog content"),
        )
        .unwrap();
        if let Some(Some(bytes)) = &self.fog {
            vision.set_visibility(bytes.iter().map(|(&province, &byte)| (province, byte)));
        }
        Arc::new(vision)
    }
}

fn step(
    assets: &MapBuildingAssets,
    hour: &Hour,
    base: Option<&BuildingsBase>,
) -> (Option<BuildingsBase>, Option<BuildPath>) {
    let frame = hour.frame();
    let vision = hour.vision(&frame);
    let city = terrain(hour.cities);
    let spans = || viewer_foundation::profiling::BuilderSpans::new(false);
    let full = assets.build(&frame, &vision, &city, Err("reference"), &mut spans());
    let patched = assets.build(&frame, &vision, &city, base.ok_or("no base"), &mut spans());
    match (full, patched) {
        (Ok(full), Ok(patched)) => {
            assert!(
                patched.prepared.bit_eq(&full.prepared),
                "hour {}",
                hour.hour
            );
            assert_eq!(patched.summary, full.summary, "hour {}", hour.hour);
            assert_eq!(patched.tallies, full.tallies, "hour {}", hour.hour);
            assert_eq!(patched.totals, full.totals, "hour {}", hour.hour);
            assert_eq!(patched.damage_refused, full.damage_refused);
            let next = patched.base(assets, &frame, &vision, assets.city_bits(&city));
            let links = hour.links.iter().copied().collect();
            let reread = GateInputs::reread(&frame, &links, vision.ours()).unwrap();
            assert_eq!(*next.gates, reread, "hour {}", hour.hour);
            (Some(next), Some(patched.path))
        }
        (Err(full), Err(patched)) => {
            assert_eq!(patched.to_string(), full.to_string(), "hour {}", hour.hour);
            (None, None)
        }
        (full, patched) => panic!(
            "hour {}: full {:?}, patched {:?}",
            hour.hour,
            full.map(|built| built.path),
            patched.map(|built| built.path)
        ),
    }
}

#[test]
fn patched_fields_equal_full_builds_hour_by_hour_and_fall_back_where_they_must() {
    let assets = assets();
    let mut hour = Hour::first();
    let mut base: Option<BuildingsBase> = None;
    let mut run = |hour: &mut Hour, edit: &dyn Fn(&mut Hour), expected: BuildPath| {
        hour.hour += 40;
        edit(hour);
        let (next, path) = step(&assets, hour, base.as_ref());
        assert_eq!(path, Some(expected), "hour {}", hour.hour);
        base = next;
    };
    let full = BuildPath::Full;
    let patched = |slots| BuildPath::Patched { slots };
    let wide = WIDE as usize;
    run(&mut hour, &|_| {}, full("no base"));
    run(&mut hour, &|_| {}, patched(0));
    run(&mut hour, &|h| h.state_rows[0].level = 12, patched(wide));
    run(
        &mut hour,
        &|h| h.state_rows.push(building(8, COMPLEX, 10, 10, 0)),
        patched(2),
    );
    run(
        &mut hour,
        &|h| {
            h.state_rows.remove(1);
        },
        patched(2),
    );
    run(
        &mut hour,
        &|h| h.state_rows.insert(1, building(8, ARMS, 4, 4, 0)),
        patched(2),
    );
    run(
        &mut hour,
        &|h| h.state_rows[0].healthy_level = 3,
        patched(wide),
    );
    run(&mut hour, &|h| h.state_rows[0].ordinal = 9, patched(0));
    run(&mut hour, &|h| h.state_rows[0].active = true, patched(0));
    run(
        &mut hour,
        &|h| {
            h.bytes().insert(1000, 40);
            h.bytes().insert(1001, 70);
        },
        patched(1),
    );
    run(
        &mut hour,
        &|h| {
            h.bytes().insert(1500, 40);
        },
        patched(0),
    );
    run(&mut hour, &|h| h.ports.push(11), patched(1));
    run(&mut hour, &|h| h.provinces[0].2 = 100, patched(1));
    run(&mut hour, &|h| h.raids = vec![raid(1)], patched(1));
    run(&mut hour, &|h| h.states[1].1 = 101, patched(0));
    run(&mut hour, &|h| h.provinces[4].1 = 7, patched(0));
    run(&mut hour, &|h| h.deployed = 0, patched(0));
    run(&mut hour, &|h| h.cities = true, patched(2));
    run(&mut hour, &|h| h.cities = false, patched(2));
    let unordered = full("building rows are not in strict key order");
    run(
        &mut hour,
        &|h| h.state_rows.push(building(8, COMPLEX, 10, 10, 0)),
        unordered.clone(),
    );
    run(
        &mut hour,
        &|h| {
            h.state_rows.pop();
        },
        unordered,
    );
    run(&mut hour, &|_| {}, patched(0));
    let states = full("the StateBuildings completeness changed");
    run(
        &mut hour,
        &|h| h.state_held = Held::Truncated,
        states.clone(),
    );
    run(
        &mut hour,
        &|_| {},
        full("the base's city damage was refused"),
    );
    run(&mut hour, &|h| h.state_held = Held::Complete, states);
    run(&mut hour, &|_| {}, patched(0));
    let provinces = full("the ProvinceBuildings completeness changed");
    run(
        &mut hour,
        &|h| h.province_held = Held::Truncated,
        provinces.clone(),
    );
    run(&mut hour, &|h| h.province_held = Held::Complete, provinces);
    run(&mut hour, &|_| {}, patched(0));
    let ports = full("the NavalBases completeness changed");
    for held in [Held::Truncated, Held::Unavailable, Held::Complete] {
        run(&mut hour, &|h| h.ports_held = held, ports.clone());
    }
    run(&mut hour, &|_| {}, patched(0));
    let raids = full("the RaidTargetStatuses completeness changed");
    for held in [Held::Truncated, Held::Unavailable, Held::Complete] {
        run(&mut hour, &|h| h.raids_held = held, raids.clone());
    }
    run(&mut hour, &|_| {}, patched(0));
    run(
        &mut hour,
        &|h| h.state_rows.push(building(9, 99, 1, 1, 0)),
        full("this hour's city damage is refused"),
    );
    run(
        &mut hour,
        &|_| {},
        full("the base's city damage was refused"),
    );
    run(
        &mut hour,
        &|h| {
            h.state_rows.pop();
        },
        full("the base's city damage was refused"),
    );
    run(&mut hour, &|_| {}, patched(0));
    run(
        &mut hour,
        &|h| h.branch = 2,
        full("the base is on another branch"),
    );
    run(&mut hour, &|_| {}, patched(0));
    run(
        &mut hour,
        &|h| h.hour -= 120,
        full("the base is not an earlier hour"),
    );
    run(&mut hour, &|_| {}, patched(0));
    run(
        &mut hour,
        &|h| h.tokens.push((81, "not_a_building")),
        patched(0),
    );
    run(&mut hour, &|h| h.tokens[2].1 = "BUNKER", patched(0));
    run(&mut hour, &|h| h.tokens[2].1 = "bunker", patched(0));
    run(
        &mut hour,
        &|h| h.state_rows.push(building(8, 81, 10, 10, 0)),
        patched(0),
    );
    run(
        &mut hour,
        &|h| h.tokens[4].1 = "INDUSTRIAL_COMPLEX",
        patched(2),
    );
    run(&mut hour, &|h| h.tokens[4].1 = "ARMS_FACTORY", patched(2));
    run(&mut hour, &|h| h.tokens[4].1 = "not_a_building", patched(2));
    run(
        &mut hour,
        &|h| {
            h.state_rows.pop();
        },
        patched(0),
    );
    run(
        &mut hour,
        &|h| h.links.push((102, 2)),
        full("the country links changed"),
    );
    let viewer = full("the viewing slot or the fog switch changed");
    run(&mut hour, &|h| h.player = 101, viewer.clone());
    run(&mut hour, &|h| h.fog = None, viewer.clone());
    run(&mut hour, &|h| h.state_rows[0].level = 14, patched(wide));
    run(&mut hour, &|h| h.fog = Some(None), viewer);
    let gap = full("a fog rebuild gap leaves the fog bytes undecided");
    run(&mut hour, &|_| {}, gap.clone());
    run(&mut hour, &|h| h.fog = Some(Some(bytes())), gap);
    run(&mut hour, &|_| {}, patched(0));
    hour.hour += 40;
    let mut refused = hour.clone();
    refused.refused = true;
    let (next, path) = step(&assets, &refused, base.as_ref());
    assert!(next.is_none() && path.is_none());
    let (next, path) = step(&assets, &hour, None);
    assert_eq!(path, Some(full("no base")));
    let other = self::assets();
    let (_, path) = step(&other, &hour, next.as_ref());
    assert_eq!(path, Some(full("the base belongs to other content")));
}

#[test]
fn a_changed_define_builds_in_full() {
    let mut assets = assets();
    let mut hour = Hour::first();
    let (base, _) = step(&assets, &hour, None);
    assets.defines.max_levels = 10;
    hour.hour += 40;
    let (_, path) = step(&assets, &hour, base.as_ref());
    assert_eq!(
        path,
        Some(BuildPath::Full("the building content operands changed"))
    );
}

#[test]
fn a_patched_hour_shares_every_chunk_it_did_not_change() {
    let assets = assets();
    let mut hour = Hour::first();
    let (base, _) = step(&assets, &hour, None);
    let base = base.unwrap();
    hour.hour += 40;
    hour.bytes().insert(1000, 10);
    let frame = hour.frame();
    let vision = hour.vision(&frame);
    let built = assets
        .build(
            &frame,
            &vision,
            &terrain(false),
            Ok(&base),
            &mut viewer_foundation::profiling::BuilderSpans::new(false),
        )
        .unwrap();
    assert_eq!(built.path, BuildPath::Patched { slots: 1 });
    assert!(base.slots[0].entity.is_some() && built.prepared.slots[0].entity.is_none());
    let chunks = (WIDE as usize + 7).div_ceil(CHUNK_SIZE);
    assert_eq!(built.prepared.slots.shared_chunks(&base.slots), chunks - 2);
    assert_eq!(
        built.tallies.shared_chunks(&base.tallies),
        (WIDE as usize + 6).div_ceil(CHUNK_SIZE) - 1
    );
}

#[test]
fn cached_plan_inputs_match_expected_sites_and_changes() {
    let assets = assets();
    static COLUMN: [bool; 64] = {
        let mut flags = [false; 64];
        let mut at = 10;
        while at < 64 {
            flags[at] = true;
            at += 16;
        }
        flags
    };
    let city = CityTerrain {
        width: 16,
        height: 4,
        spawn_city: &COLUMN,
    };
    assert_eq!(assets.city_bits(&city), vec![true, false]);
    assert_eq!(assets.city_bits(&terrain(false)), vec![false, false]);
    assert_eq!(assets.city_bits(&terrain(true)), vec![true, true]);
    let old = BTreeMap::from([(1, 1), (2, 2), (4, 4), (6, 6), (9, 9)]);
    let new = BTreeMap::from([(2, 2), (3, 3), (4, 5), (7, 7), (9, 9)]);
    assert_eq!(changed_entries(&old, &new), vec![1, 3, 4, 6, 7]);
    assert_eq!(changed_entries(&new, &new), Vec::<u32>::new());
    assert_eq!(changed_entries(&BTreeMap::new(), &new), vec![2, 3, 4, 7, 9]);
    assert_eq!(changed_entries(&old, &BTreeMap::new()), vec![1, 2, 4, 6, 9]);
    assert_eq!(assets.sites().slots.len(), WIDE as usize + 6);
    assert_eq!(assets.slot_count(), WIDE as usize + 7);
}

#[test]
fn token_maps_diffed_from_another_table_equal_full_reads() {
    let assets = assets();
    let (keys, index) = (&assets.facts.keys, &assets.facts.index);
    let t = |token: u32, literal: &[u8]| (token, literal.to_vec());
    let dictionary = |tokens: &[(u32, Vec<u8>)], held: Held| {
        let mut frame = Hour::first().frame();
        frame.tables[TableId::LexicalTokens.index()] = held
            .view(TableRows::LexicalTokens(
                tokens
                    .iter()
                    .map(|(token, literal)| LexicalTokenRow {
                        token: *token,
                        literal: literal.clone(),
                    })
                    .collect(),
            ))
            .into();
        frame
    };
    let base = vec![
        t(77, b"arms_factory"),
        t(78, b"industrial_complex"),
        t(79, b"bunker"),
        t(80, b"supply_node"),
    ];
    let mut grown = base.clone();
    grown.insert(0, t(0, b"arms_factory"));
    grown.extend([
        t(81, b"not_a_building"),
        t(82, b"BUNKER"),
        t(90, b"supply_node"),
    ]);
    let shrunk: Vec<_> = grown
        .iter()
        .filter(|(token, _)| *token != 78)
        .cloned()
        .collect();
    let respelled: Vec<_> = shrunk
        .iter()
        .map(|(token, literal)| match token {
            79 => t(79, b"INDUSTRIAL_COMPLEX"),
            80 => t(80, b"nothing"),
            81 => t(81, &[0xff, 0xfe]),
            82 => t(82, b"bunker\0"),
            _ => (*token, literal.clone()),
        })
        .collect();
    let steps = [
        dictionary(&base, Held::Complete),
        dictionary(&grown, Held::Complete),
        dictionary(&shrunk, Held::Complete),
        dictionary(&respelled, Held::Complete),
        dictionary(&respelled, Held::Truncated),
        dictionary(&base, Held::Complete),
    ];
    for pair in steps.windows(2) {
        let old = token_types(&pair[0], keys, index).unwrap();
        let full = token_types(&pair[1], keys, index).unwrap();
        assert_eq!(
            token_types_from(&pair[0], &old, &pair[1], keys, index),
            Some(full)
        );
    }
    for frame in &steps {
        let full = token_types(frame, keys, index).unwrap();
        assert_eq!(*assets.token_map(frame).unwrap(), full);
    }
    let unordered = dictionary(&[t(79, b"bunker"), t(77, b"arms_factory")], Held::Complete);
    let old = token_types(&steps[0], keys, index).unwrap();
    assert_eq!(
        token_types_from(&steps[0], &old, &unordered, keys, index),
        None
    );
    assert!(token_types(&unordered, keys, index).is_err());
    assert!(assets.token_map(&unordered).is_err());
    let full = token_types(&steps[1], keys, index).unwrap();
    assert_eq!(*assets.token_map(&steps[1]).unwrap(), full);
}

#[test]
fn the_exact_comparator_tells_signed_zeros_apart() {
    let assets = assets();
    let (base, _) = step(&assets, &Hour::first(), None);
    let built = base.unwrap();
    let mut other = PreparedBuildings {
        branch: 1,
        hour: 100,
        shrink_distance: assets.defines.shrink_distance,
        popup_speed: assets.defines.popup_speed,
        slots: built.slots.clone(),
        groups: Arc::clone(&assets.groups),
        city_damage: Default::default(),
    };
    let same = other.clone();
    assert!(other.bit_eq(&same));
    let instance = &mut other.slots.get_mut(0).entity.as_mut().unwrap().instance;
    assert_eq!(instance.position[1].to_bits(), 0.0_f32.to_bits());
    instance.position[1] = -0.0;
    assert!(other == same, "derived equality takes -0.0 for 0.0");
    assert!(!other.bit_eq(&same));
}

#[test]
fn changed_keys_name_every_difference_and_refuse_unordered_rows() {
    let key = |row: &(u32, u32)| row.0;
    assert_eq!(
        changed_keys(&[(1, 1), (2, 2), (4, 4)], &[(2, 2), (3, 3), (4, 5)], key),
        Some(vec![1, 3, 4])
    );
    assert_eq!(changed_keys(&[(1, 1)], &[(1, 1)], key), Some(Vec::new()));
    assert_eq!(changed_keys(&[(2, 0), (1, 0)], &[], key), None);
    assert_eq!(changed_keys(&[], &[(1, 0), (1, 1)], key), None);
}

#[test]
fn patched_chunks_keep_untouched_chunks_and_replace_the_tail() {
    let size = CHUNK_SIZE;
    let values: Chunked<u32> = (0..(size as u32 * 2 + 3)).collect();
    assert_eq!(values.chunks.len(), 3);
    assert_eq!(values.len(), size * 2 + 3);
    assert!((0..size * 2 + 3).all(|id| values[id] == id as u32));
    let patched = values
        .patched(&[(5, 1_000)], size * 2 + 1, &[7, 8])
        .unwrap();
    assert_eq!(patched[5], 1_000);
    assert!((0..size * 2 + 1).all(|id| id == 5 || patched[id] == id as u32));
    assert_eq!((patched[size * 2 + 1], patched[size * 2 + 2]), (7, 8));
    assert_eq!(patched.shared_chunks(&values), 1);
    assert!(values.patched(&[], 1, &[]).is_none(), "another length");
}

#[test]
fn a_building_base_keeps_only_its_plan_tables_and_vision_operands() {
    use replay_frame::frame::SharedView;
    use viewer_foundation::map::memo::NOT_AN_INPUT;
    let assets = assets();
    let hour = Hour::first();
    let mut frame = hour.frame();
    frame.metrics.total_ns = 123;
    let vision = hour.vision(&frame);
    let built = assets
        .build(
            &frame,
            &vision,
            &terrain(false),
            Err("reference"),
            &mut viewer_foundation::profiling::BuilderSpans::new(false),
        )
        .unwrap();
    let holders = Arc::strong_count(&vision);
    let base = built.base(&assets, &frame, &vision, assets.city_bits(&terrain(false)));
    assert_eq!(Arc::strong_count(&vision), holders);
    assert_eq!(
        (base.frame.branch, base.frame.hour),
        (frame.branch, frame.hour)
    );
    assert_eq!(base.frame.metrics, Metrics::default());
    let retained = [
        TableId::StateBuildings,
        TableId::ProvinceBuildings,
        TableId::CountryLinks,
        TableId::NavalBases,
        TableId::RaidTargetStatuses,
    ];
    for (index, table) in base.frame.tables.iter().enumerate() {
        if retained.iter().any(|id| id.index() == index) {
            assert!(SharedView::ptr_eq(table, &frame.tables[index]));
            assert_eq!(table, &frame.tables[index]);
        } else {
            assert!(
                matches!(&**table, TableView::Unavailable { reason } if reason == NOT_AN_INPUT)
            );
            assert!(!SharedView::ptr_eq(table, &frame.tables[index]));
        }
    }
    assert_eq!(base.vision.ours, vision.ours());
    assert_eq!(base.vision.fog, vision.fog());
    assert_eq!(
        base.vision.visibility_gap,
        vision.visibility_gap().is_some()
    );
    assert_eq!(base.vision.provinces.as_deref(), vision.province_fog());
    let last = assets.last_tokens.lock().unwrap();
    let (tokens, _) = last.as_ref().unwrap();
    for (index, table) in tokens.tables.iter().enumerate() {
        if index == TableId::LexicalTokens.index() {
            assert!(SharedView::ptr_eq(table, &frame.tables[index]));
        } else {
            assert!(
                matches!(&**table, TableView::Unavailable { reason } if reason == NOT_AN_INPUT)
            );
        }
    }
}

#[test]
fn retained_fog_bytes_preserve_the_province_object_predicate() {
    let mut vision = Vision::sharing(7, [7]);
    vision.set_visibility((0u8..=u8::MAX).map(|byte| (u32::from(byte), byte)));
    let operands = VisionOperands::read(&vision);
    assert!(Arc::ptr_eq(
        operands.provinces.as_ref().unwrap(),
        vision.shared_province_fog().unwrap()
    ));
    for province in 0..=256 {
        assert_eq!(
            operands.sees_province_object(province),
            vision.sees_province_object(province, false)
        );
    }
    vision.set_visibility([(49, 255), (50, 0), (256, 255)]);
    assert!(!Arc::ptr_eq(
        operands.provinces.as_ref().unwrap(),
        vision.shared_province_fog().unwrap()
    ));
    assert_eq!(operands.sees_province_object(49), Some(false));
    assert_eq!(operands.sees_province_object(50), Some(true));
    assert_eq!(operands.sees_province_object(256), None);
    let clear = vision.without_fog();
    let operands = VisionOperands::read(&clear);
    assert!(!operands.fog && !operands.visibility_gap && operands.provinces.is_none());
    assert_eq!(
        operands.sees_province_object(256),
        clear.sees_province_object(256, false)
    );
    let mut hour = Hour::first();
    hour.fog = Some(None);
    let gap = hour.vision(&hour.frame());
    let operands = VisionOperands::read(&gap);
    assert!(operands.fog && operands.visibility_gap && operands.provinces.is_none());
}
