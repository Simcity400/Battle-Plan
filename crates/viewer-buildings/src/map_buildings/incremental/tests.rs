use super::*;
use content_reader_core::buildings;
use replay_data::rows::{
    CountryLinkRow, FloatingHarborDeploymentRow, LexicalTokenRow, LocalPlayerRow, NavalBaseRow,
    ProvinceRow, ProvinceSupplyRow, RaidScope, RaidTargetStatusRow, StateRow, TABLE_COUNT,
    TableRows,
};
use replay_frame::frame::{Metrics, SharedView, TableView};
use viewer_foundation::perspective::{Choice, Country};

#[test]
fn streamed_slot_chunks_preserve_order() {
    let count = CHUNK_SIZE * 20 + 7;
    let mut builder = ChunkBuilder::new();
    for value in 0..count {
        builder.push([value as u64; 32]).unwrap();
    }
    let chunks = builder.finish().unwrap();
    assert_eq!(chunks.len(), count);
    for value in 0..count {
        assert_eq!(chunks.get(value), Some(&[value as u64; 32]));
    }
}

#[test]
fn installed_live_buildings_keep_their_producer_pages_across_lookahead() {
    let assets = assets();
    let keys: Vec<_> = (0..5)
        .map(|index| {
            Arc::new(map_display_data::buildings::AssetKey::Selected {
                entity: format!("fixture{index}"),
                namespace: Vec::new(),
                role: 0,
                contract: 0,
                force_pow2_textures: false,
            })
        })
        .collect();
    assets.display_keys.set(keys.clone()).unwrap();
    assets
        .display_bindings
        .set(Ok(keys
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index))
            .collect()))
        .unwrap();
    let hour = Hour::first();
    let frame = hour.frame();
    let vision = hour.vision(&frame);
    let built = assets
        .build(
            &frame,
            &vision,
            &terrain(hour.cities),
            Err("reference"),
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap();
    let wire = assets
        .display(
            &built.prepared,
            [1; 32],
            vision.ours(),
            vision.fog(),
            Vec::new(),
            None,
        )
        .unwrap();
    let ahead = assets
        .display(
            &built.prepared,
            [1; 32],
            vision.ours(),
            !vision.fog(),
            Vec::new(),
            None,
        )
        .unwrap();
    assert!(!wire.slots.ptr_eq(&ahead.slots));
    let mut history = BuildingHistory::default();
    let installed = history.install_display(&wire, &assets).unwrap();
    assert!(installed.slots.ptr_eq(&built.prepared.slots));
    assert!(installed.bit_eq(&built.prepared));
    let encoded = product_codec::encode(&wire).unwrap();
    let restored = product_codec::decode(&encoded).unwrap();
    let mut restored_history = BuildingHistory::default();
    let restored = restored_history
        .install_display(&restored, &assets)
        .unwrap();
    assert!(!restored.slots.ptr_eq(&built.prepared.slots));
    assert!(restored.bit_eq(&built.prepared));
    let receiver = self::assets();
    receiver.display_binding.set([1; 32]).unwrap();
    receiver
        .display_bindings
        .set(Ok(assets
            .display_keys
            .get()
            .unwrap()
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, key)| (key, 4 - index))
            .collect()))
        .unwrap();
    let rebound = BuildingHistory::default()
        .install_display(&wire, &receiver)
        .unwrap();
    let decoded = product_codec::decode(&encoded).unwrap();
    let restored = BuildingHistory::default()
        .install_display(&decoded, &receiver)
        .unwrap();
    assert!(!rebound.slots.ptr_eq(&built.prepared.slots));
    assert!(rebound.bit_eq(&restored));
    assert!(!rebound.bit_eq(&built.prepared));
}

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
    content_reader_core::test_game::enter();
    let types = buildings::parse(TYPES, &content_reader_core::key_names::KeyNames::default()).unwrap();
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
    campaign: SharedView,
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
            campaign: viewer_scene_base::test_support::test_campaign(),
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
        let mut tables: [SharedView; TABLE_COUNT] = views.map(Into::into);
        tables[TableId::Campaign.index()] = self.campaign.clone();
        DecodedFrame {
            branch: self.branch,
            hour: self.hour,
            tables,
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
    let spans = || viewer_runtime::profiling::BuilderSpans::new(false);
    let full = assets.build(&frame, &vision, &city, Err("reference"), &mut spans());
    let patched = assets.build(&frame, &vision, &city, base.ok_or("no base"), &mut spans());
    match (full, patched) {
        (Ok(full), Ok(mut patched)) => {
            assert!(
                patched.prepared.bit_eq(&full.prepared),
                "hour {}",
                hour.hour
            );
            assert_eq!(patched.summary, full.summary, "hour {}", hour.hour);
            assert_eq!(patched.tallies, full.tallies, "hour {}", hour.hour);
            assert_eq!(patched.totals, full.totals, "hour {}", hour.hour);
            assert_eq!(patched.damage_refused, full.damage_refused);
            let carried = base
                .filter(|base| base.assets == assets.id && base.observation.branch == hour.branch);
            let repainted: Vec<u32> = (assets.ordinary_slot_count()..assets.slot_count())
                .filter(|&id| {
                    carried.is_none_or(|base| !base.slots[id].same_bits(&full.prepared.slots[id]))
                })
                .map(|id| id as u32)
                .collect();
            let emitted: Vec<u32> = patched
                .floating_output
                .as_ref()
                .unwrap()
                .changes
                .iter()
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(emitted, repainted, "hour {}", hour.hour);
            let next = patched.base(assets, &vision, assets.city_bits(&city));
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
    run(&mut hour, &|h| h.links.push((102, 2)), patched(0));
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
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap();
    assert_eq!(built.path, BuildPath::Patched { slots: 1 });
    assert!(built.floating_output.as_ref().unwrap().changes.is_empty());
    assert!(base.slots[0].entity.is_some() && built.prepared.slots[0].entity.is_none());
    let chunks = (WIDE as usize + 7).div_ceil(CHUNK_SIZE);
    assert_eq!(built.prepared.slots.shared_chunks(&base.slots), chunks - 1);
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
    assert_eq!(*assets.city_bits(&city), vec![true, false]);
    assert_eq!(*assets.city_bits(&terrain(false)), vec![false, false]);
    assert_eq!(*assets.city_bits(&terrain(true)), vec![true, true]);
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
    let (mut zero, mut negative) = (same.clone(), same.clone());
    zero.popup_speed = 0.0;
    negative.popup_speed = -0.0;
    assert!(negative == zero, "derived equality takes -0.0 for 0.0");
    assert!(!negative.bit_eq(&zero));
    other.slots.edit(0, |slot| {
        let instance = &mut slot.entity_mut().instance;
        assert_eq!(instance.position[1].to_bits(), 0.0_f32.to_bits());
        instance.position[1] = -0.0;
    });
    assert!(other != same, "slot equality compares instance bits");
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
        .patched(&[(5, 1_000)], size * 2 + 1, &[7, 8], |a, b| a == b)
        .unwrap()
        .unwrap();
    assert_eq!(patched[5], 1_000);
    assert!((0..size * 2 + 1).all(|id| id == 5 || patched[id] == id as u32));
    assert_eq!((patched[size * 2 + 1], patched[size * 2 + 2]), (7, 8));
    assert_eq!(patched.shared_chunks(&values), 1);
    assert!(
        values
            .patched(&[], 1, &[], |a, b| a == b)
            .unwrap()
            .is_none(),
        "another length"
    );
}

#[test]
fn a_building_base_keeps_closed_operands_without_source_pins() {
    let assets = assets();
    let hour = Hour::first();
    let frame = hour.frame();
    let vision = hour.vision(&frame);
    let mut built = assets
        .build(
            &frame,
            &vision,
            &terrain(false),
            Err("reference"),
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap();
    let holders = Arc::strong_count(&vision);
    let base = built.base(&assets, &vision, assets.city_bits(&terrain(false)));
    assert_eq!(Arc::strong_count(&vision), holders);
    assert_eq!(
        (base.observation.branch, base.observation.hour),
        (frame.branch, frame.hour)
    );
    assert_eq!(base.vision.ours, vision.ours());
    assert_eq!(base.vision.fog, vision.fog());
    assert_eq!(
        base.vision.provinces.as_deref().map(|v| v.flat()),
        vision.province_fog()
    );
    drop(vision);
    drop(hour);
    for view in frame.tables {
        assert!(
            view.try_unwrap().is_ok(),
            "building products must not pin a source table"
        );
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

#[test]
fn compact_building_observations_patch_changed_keys_and_share_unrelated_paths() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let defines = Defines {
        shrink_distance: 1.0,
        popup_speed: 1.0,
        destroyed: 0,
        recovered: 1,
        damaged: 50000,
        max_levels: 20,
    };
    let mut hour = Hour::first();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let raw = encoder.encode(hour.frame(), true).unwrap();
    let frame = decoder.advance(&raw).unwrap();
    let first = Observation::read(frame, &defines, 0, None).unwrap();
    hour.hour += 1;
    hour.state_rows[0].ordinal += 1;
    let raw = encoder.encode(hour.frame(), false).unwrap();
    let frame = decoder.advance(&raw).unwrap();
    let quiet = Observation::read(frame, &defines, 0, Some(&first)).unwrap();
    assert!(quiet.state_buildings.ptr_eq(&first.state_buildings));
    assert!(quiet.building_changes[0].is_empty());
    hour.hour += 1;
    hour.state_rows[0].level += 1;
    let raw = encoder.encode(hour.frame(), false).unwrap();
    let frame = decoder.advance(&raw).unwrap();
    let changed = Observation::read(frame, &defines, 0, Some(&quiet)).unwrap();
    let expected: BTreeMap<_, _> = hour
        .state_rows
        .iter()
        .map(|row| (row_key(row), slot_read(row, &defines)))
        .collect();
    assert_eq!(
        changed.state_buildings.iter().collect::<Vec<_>>(),
        expected.iter().collect::<Vec<_>>()
    );
    assert_eq!(changed.building_changes[0], [row_key(&hour.state_rows[0])]);
    assert!(changed.building_tokens[0].ptr_eq(&quiet.building_tokens[0]));
    assert!(changed.province_buildings.ptr_eq(&quiet.province_buildings));
    assert!(changed.building_changes[1].is_empty());
    drop(decoder);
}
#[test]
fn changed_detection_rows_preserve_last_ordinal_scope_winners_and_ignore_other_fields() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let defines = Defines {
        shrink_distance: 1.0,
        popup_speed: 1.0,
        destroyed: 0,
        recovered: 1,
        damaged: 50000,
        max_levels: 20,
    };
    let mut hour = Hour::first();
    let mut duplicate = raid(0);
    duplicate.ordinal = 9;
    hour.raids = vec![raid(1), duplicate];
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let raw = encoder.encode(hour.frame(), true).unwrap();
    let frame = decoder.advance(&raw).unwrap();
    let mut previous = Observation::read(frame, &defines, 0, None).unwrap();
    let scope = (BUNKER, 9, 1, 0);
    assert_eq!(
        previous.targets.as_ref().unwrap().winners.get(&scope),
        Some(&false)
    );
    for change in 0..4 {
        hour.hour += 1;
        match change {
            0 => hour.raids[0].detected[0] = 0,
            1 => hour.raids[1].valid = 0,
            2 => {
                hour.raids.pop();
            }
            _ => hour.provinces[0].2 += 1,
        }
        let raw = encoder.encode(hour.frame(), false).unwrap();
        let frame = decoder.advance(&raw).unwrap();
        let next = Observation::read(frame, &defines, 0, Some(&previous)).unwrap();
        let expected: BTreeMap<_, _> = raid_targets(frame)
            .unwrap()
            .into_iter()
            .map(|(scope, bits)| (scope, bits.first().is_some_and(|bits| bits & 1 != 0)))
            .collect();
        assert_eq!(
            next.targets
                .as_ref()
                .unwrap()
                .winners
                .iter()
                .collect::<Vec<_>>(),
            expected.iter().collect::<Vec<_>>()
        );
        assert!(next.targets.as_ref().unwrap().changes.is_empty());
        if change == 1 {
            assert!(
                next.targets
                    .as_ref()
                    .unwrap()
                    .rows
                    .values
                    .ptr_eq(&previous.targets.as_ref().unwrap().rows.values)
            );
        }
        if change == 3 {
            assert_eq!(next.controllers[0].changes, [hour.provinces[0].0]);
            assert!(
                next.controllers[1]
                    .values
                    .ptr_eq(&previous.controllers[1].values)
            );
        }
        previous = next;
    }
}

#[test]
fn persistent_slot_chunks_do_not_coalesce_signed_zero_or_nan_payloads() {
    let make = |x| PreparedSlot {
        source: None,
        id: 0,
        drawn_at_distance: Some(false),
        entity: Some(
            PreparedEntity {
                healthy: Selection {
                    admitted: true,
                    mesh: 1,
                    definition: 1,
                    variant: 0,
                },
                ruined: Selection {
                    admitted: true,
                    mesh: 1,
                    definition: 1,
                    variant: 0,
                },
                destroyed: false,
                uncertain: false,
                damaged: false,
                disable_grow_animation: false,
                instance: ModelInstance {
                    position: [x, 0.0, 0.0],
                    ..Default::default()
                },
            }
            .retain(),
        ),
    };
    let original = Chunked::from_values(vec![make(0.0)]).unwrap();
    let signed = original
        .patched(&[(0, make(-0.0))], 1, &[], PreparedSlot::same_bits)
        .unwrap()
        .unwrap();
    assert_eq!(
        signed[0].entity.as_ref().unwrap().instance.position[0].to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(
        original[0].entity.as_ref().unwrap().instance.position[0].to_bits(),
        0.0f32.to_bits()
    );
    let nan = f32::from_bits(0x7fc0_1234);
    let first = signed
        .patched(&[(0, make(nan))], 1, &[], PreparedSlot::same_bits)
        .unwrap()
        .unwrap();
    let same = first
        .patched(&[(0, make(nan))], 1, &[], PreparedSlot::same_bits)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(
        first.chunks.get(0).unwrap(),
        same.chunks.get(0).unwrap()
    ));
    let other = first
        .patched(
            &[(0, make(f32::from_bits(0x7fc0_4321)))],
            1,
            &[],
            PreparedSlot::same_bits,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        other[0].entity.as_ref().unwrap().instance.position[0].to_bits(),
        0x7fc0_4321
    );
}

#[test]
fn immutable_city_bank_reuses_hub_operands_and_rejects_replaced_bank() {
    let assets = assets();
    let hour = Hour::first();
    let frame = hour.frame();
    let vision = hour.vision(&frame);
    let mut pixels = Arc::new(vec![false; 64]);
    let city = CityTerrain {
        width: 16,
        height: 4,
        spawn_city: &pixels,
    };
    let binding = CityBank::new(&city, &pixels).unwrap();
    let mut built = assets
        .build(
            &frame,
            &vision,
            &city,
            Err("cold"),
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap();
    let first = built.city.take().unwrap();
    let mut base = built.base(&assets, &vision, first.clone());
    base.city_bank = Some(binding);
    let same = CityBank::new(&city, &pixels).unwrap();
    assert!(first.ptr_eq(base.city_for(&same, assets.id).unwrap()));
    assert!(base.city_for(&same, assets.id.wrapping_add(1)).is_none());
    drop(city);
    Arc::make_mut(&mut pixels)[0] = true;
    let replaced = CityTerrain {
        width: 16,
        height: 4,
        spawn_city: &pixels,
    };
    let binding = CityBank::new(&replaced, &pixels).unwrap();
    assert!(base.city_for(&binding, assets.id).is_none());
    assert!(CityBank::new(&terrain(false), &pixels).is_err());
}

#[test]
fn floating_source_folds_preserve_first_ordinal_completeness_and_unrelated_paths() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let mut hour = Hour::first();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let mut supply = ProvinceSupplyRow {
        province: SEA,
        controller_index: 2,
        flags: 1,
        ..Default::default()
    };
    let mut deployments: Vec<_> = (0..1501)
        .map(|ordinal| FloatingHarborDeploymentRow {
            country: 2,
            ordinal,
            province: if ordinal < 2 {
                SEA as i32
            } else {
                ordinal as i32 + 100
            },
            status: u8::from(ordinal != 0),
        })
        .collect();
    let mut previous: Option<FloatingInputs> = None;
    for step in 0..8 {
        hour.hour += 1;
        match step {
            1 => supply.capital_country_index = 7,
            2 => deployments[0].status = 1,
            3 => {
                deployments.remove(0);
            }
            4 => deployments[0].province = -1,
            5 => supply.flags = 0,
            6 => supply.flags = 1,
            _ => {}
        }
        let mut frame = hour.frame();
        frame.tables[TableId::ProvinceSupply.index()] =
            TableView::Complete(TableRows::ProvinceSupply(vec![supply.clone()])).into();
        let complete = step != 6;
        frame.tables[TableId::FloatingHarborDeployments.index()] = if complete {
            TableView::Complete(TableRows::FloatingHarborDeployments(deployments.clone()))
        } else {
            TableView::Truncated {
                rows: TableRows::FloatingHarborDeployments(deployments.clone()),
                missing: 1,
            }
        }
        .into();
        let raw = encoder.encode(frame, step == 0).unwrap();
        let frame = decoder.advance(&raw).unwrap();
        let folded = FloatingInputs::read(frame, previous.as_ref()).unwrap();
        for province in [SEA, 100, 101, 500, u32::MAX] {
            let row = (province == SEA).then_some(&supply);
            assert_eq!(
                folded.deployed(province),
                floating_harbor_deployed(province, row, true, &deployments, complete),
                "step {step}, province {province}"
            );
        }
        if step == 1 {
            let old = previous.as_ref().unwrap();
            assert!(folded.supply.values.ptr_eq(&old.supply.values));
            assert!(folded.deployments.values.ptr_eq(&old.deployments.values));
            assert!(
                folded
                    .by_country_province_ordinal
                    .ptr_eq(&old.by_country_province_ordinal)
            );
        }
        previous = Some(folded);
    }
}

fn reference_floating(
    assets: &MapBuildingAssets,
    frame: &DecodedFrame,
    vision: &Vision,
) -> (Vec<PreparedSlot>, MapBuildingSummary) {
    let supply = rows::<tables::ProvinceSupply>(frame).unwrap();
    let deployments = rows::<tables::FloatingHarborDeployments>(frame).unwrap();
    let mut summary = MapBuildingSummary::default();
    let mut harbors = Vec::new();
    let ordinary_slots = assets.ordinary_slot_count();
    for (position, _) in &assets.floating {
        let id = (ordinary_slots + harbors.len()) as u32;
        let entity = (|| {
            let province = position.adjacent_sea;
            if (province as i32) <= 0 {
                return None;
            }
            match vision
                .province_byte(province)
                .map(|byte| byte >= FLOATING_HARBOR_VISIBILITY)
            {
                Some(true) => {}
                Some(false) => {
                    summary.fogged += 1;
                    return None;
                }
                None => {
                    *summary
                        .unknown_floating_harbors
                        .entry("the fog byte of the harbour's sea province is unknown")
                        .or_default() += 1;
                    return None;
                }
            }
            let deployed = floating_harbor_deployed(
                province,
                supply.rows().iter().find(|row| row.province == province),
                supply.is_complete(),
                deployments.rows(),
                deployments.is_complete(),
            );
            if deployed.is_none() {
                *summary
                    .unknown_floating_harbors
                    .entry("the controller's floating-harbour deployment is unknown")
                    .or_default() += 1;
            }
            let mesh = assets.floating_mesh.filter(|_| deployed == Some(true))?;
            summary.placed += 1;
            let selection = Selection {
                admitted: true,
                mesh,
                definition: mesh,
                variant: 0,
            };
            Some(
                PreparedEntity {
                    healthy: selection,
                    ruined: selection,
                    destroyed: false,
                    uncertain: false,
                    damaged: false,
                    disable_grow_animation: false,
                    instance: ModelInstance {
                        position: [position.x, position.y, position.z],
                        rotation: position.rotation,
                        scale: 1.0,
                        animation_seed: id,
                        ..ModelInstance::default()
                    },
                }
                .retain(),
            )
        })();
        harbors.push(PreparedSlot {
            source: None,
            id,
            drawn_at_distance: Some(false),
            entity,
        });
    }
    (harbors, summary)
}

#[test]
fn floating_output_delta_and_tail_patch_match_the_frozen_scalar_reference() {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let assets = assets();
    let mut hour = Hour::first();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    let mut base: Option<BuildingsBase> = None;
    for step in 0..10 {
        hour.hour += 1;
        match step {
            2 => hour.deployed = 0,
            3 => {
                hour.bytes().insert(SEA, 0);
            }
            4 => {
                hour.bytes().remove(&SEA);
            }
            5 => {
                hour.bytes().insert(SEA, 255);
                hour.deployed = 1;
            }
            6 => hour.fog = Some(None),
            7 => hour.fog = None,
            8 => hour.fog = Some(Some(bytes())),
            _ => {}
        }
        let encoded = encoder.encode(hour.frame(), step == 0).unwrap();
        let frame = decoder.advance(&encoded).unwrap();
        let vision = hour.vision(frame);
        let city = terrain(false);
        let (expected, expected_summary) = reference_floating(&assets, frame, &vision);
        let mut built = assets
            .build(
                frame,
                &vision,
                &city,
                base.as_ref().ok_or("cold"),
                &mut viewer_runtime::profiling::BuilderSpans::new(false),
            )
            .unwrap();
        let output = built.floating_output.as_ref().unwrap();
        assert!(output.iter().zip(&expected).all(|(a, b)| a.same_bits(b)));
        let mut summary = MapBuildingSummary::default();
        output.apply(&mut summary);
        assert_eq!(summary, expected_summary);
        let ordinary = assets.ordinary_slot_count();
        for (offset, expected) in expected.iter().enumerate() {
            assert!(built.prepared.slots[ordinary + offset].same_bits(expected));
        }
        if step == 1 {
            assert!(output.changes.is_empty());
            assert!(built.prepared.slots.ptr_eq(&base.as_ref().unwrap().slots));
        }
        let city_bits = built.city.take().unwrap();
        base = Some(built.base(&assets, &vision, city_bits));
    }
}

#[test]
fn fog_changes_preserve_unknowns_and_only_dirty_native_visibility_boundaries() {
    let mut hour = Hour::first();
    let assets = assets();
    let frame = hour.frame();
    let vision = hour.vision(&frame);
    let mut built = assets
        .build(
            &frame,
            &vision,
            &terrain(hour.cities),
            Err("reference"),
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap();
    let city = built.city.take().unwrap();
    let mut base = built.base(&assets, &vision, city);
    let mut previous = BTreeMap::from([
        (1, 0),
        (2, MAP_OBJECT_VISIBILITY - 1),
        (3, MAP_OBJECT_VISIBILITY),
        (4, FLOATING_HARBOR_VISIBILITY - 1),
        (5, FLOATING_HARBOR_VISIBILITY),
        (6, u8::MAX),
        (7, 0),
    ]);
    base.vision.provinces = Some(Arc::new(
        viewer_foundation::perspective::values::Values::from_iter(
            previous.iter().map(|(&key, &value)| (key, value)),
        ),
    ));
    let mut next = (*vision).clone();
    previous.remove(&7);
    previous.insert(1, MAP_OBJECT_VISIBILITY - 1);
    previous.insert(2, MAP_OBJECT_VISIBILITY);
    previous.insert(3, FLOATING_HARBOR_VISIBILITY - 1);
    previous.insert(4, FLOATING_HARBOR_VISIBILITY);
    previous.insert(5, u8::MAX);
    previous.insert(6, FLOATING_HARBOR_VISIBILITY - 1);
    previous.insert(8, 0);
    next.set_visibility(previous);
    let changed = fog_changes(Some(&base), &next).unwrap().unwrap();
    assert_eq!(
        changed.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        vec![2, 4, 6, 7, 8]
    );
    assert_eq!(base.vision.sees_province_object(7), Some(false));
    assert_eq!(next.sees_province_object(7, false), None);
    assert_eq!(base.vision.sees_province_object(8), None);
    assert_eq!(next.sees_province_object(8, false), Some(false));
    hour.fog = None;
    let without_fog = hour.vision(&frame);
    assert!(fog_changes(Some(&base), &without_fog).unwrap().is_none());
}
