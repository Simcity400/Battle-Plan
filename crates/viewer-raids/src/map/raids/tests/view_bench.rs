
use std::sync::Arc;
use viewer_runtime::parallel::Workers;

use content_reader_raids::{
    raids::BattalionRequirement, raids::SourceKind, raids::StartingPoint, raids::UnitRequirement,
};
use replay_data::{
    rows::DivisionTemplateIconRow, rows::NavalBaseRow, rows::NukeRow, rows::RaidCountryInputRow,
    rows::RaidPathDistanceRow, rows::TemplateSubunit,
};

use super::{
    reach_bench::{HEIGHT, MARINES, WIDTH, base_provinces, grid, hour},
    *,
};

const NAVAL_BASE: u32 = 10;
const SUPPLY_NODE: u32 = 11;
const MARINE: u32 = 20;
const LARGE: u32 = 30;
const VARIANT: u64 = 0x0000_0006_0000_0046;
const WINGS: u32 = 3;
const TEMPLATE: u64 = 0x0000_0005_0000_0014;

fn raw(slot: u32) -> u32 {
    1_000 + slot
}

fn xy(province: u32) -> (u32, u32) {
    ((province - 1) % WIDTH, (province - 1) / WIDTH)
}

fn kinds() -> Vec<RaidType> {
    let targets = |index: usize| {
        let building = if index.is_multiple_of(2) {
            "naval_base"
        } else {
            "supply_node"
        };
        vec![TargetType::Building(
            content_reader_raids::raids::BuildingSelector {
                types: vec![building.into()],
                ..Default::default()
            },
        )]
    };
    let marines = vec![UnitRequirement {
        battalions: vec![BattalionRequirement {
            name: "marine".into(),
            min: Some(1),
            max: None,
        }],
        ..Default::default()
    }];
    let bombers = vec![UnitRequirement {
        equipment: vec![content_reader_raids::raids::EquipmentRequirement {
            types: vec!["strategic_bomber".into()],
            min: Some(10),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let base = |index: usize, starting_point: StartingPoint, units: &Vec<UnitRequirement>| {
        RaidType {
            name: format!("raid_{index}"),
            category: Some("army".into()),
            show_target: parse("NOT = { is_in_faction_with = FROM }"),
            target_types: targets(index),
            command_power_raw: 10 * 100_000,
            max_distance_raw: Some(500_000 * (2 + (index % 5) as i64)),
            starting_point,
            unit_requirements: units.clone(),
            ..Default::default()
        }
    };
    let from = |types: Vec<SourceKind>| StartingPoint {
        types,
        ..Default::default()
    };
    let buildings = |names: &[&str], faction: bool| StartingPoint {
        building_types: names.iter().map(|name| (*name).into()).collect(),
        allow_faction_buildings: faction,
        ..Default::default()
    };
    let mut out = Vec::new();
    for index in 0..23 {
        out.push(base(index, from(vec![SourceKind::AirBase]), &bombers));
    }
    for index in 23..25 {
        out.push(base(index, from(vec![SourceKind::NavalBase]), &marines));
    }
    for (index, point) in (25..31).zip([
        buildings(&["supply_node"], true),
        buildings(&["supply_node"], false),
        buildings(&["supply_node"], true),
        buildings(&["supply_node"], false),
        buildings(&["supply_node", "naval_base"], true),
        buildings(&["naval_base"], true),
    ]) {
        out.push(base(index, point, &marines));
    }
    out.push(base(31, from(vec![SourceKind::Carrier]), &bombers));
    for index in 32..36 {
        out.push(RaidType {
            any_province: true,
            ..base(index, from(vec![SourceKind::AirBase]), &bombers)
        });
    }
    out
}

fn world(countries: u32) -> (RaidAssets, DecodedFrame) {
    let (graph, geometry) = grid();
    let mut assets = assets();
    assets.moves = Ok(content_reader_unit_path::unit_path::MoveGraph::new(
        &graph, &geometry,
    ));
    assets.geometry = Arc::new(geometry);
    assets.paths = Ok(paths::PathSearch::new(graph));
    assets.defines.base_max_command_power = 100 * 100_000;
    assets.catalog = Catalog {
        types: kinds(),
        ..Default::default()
    };
    assets.trigger_content.buildings = content_reader_key_names::buildings::Buildings {
        types: ["naval_base", "supply_node"]
            .map(|name| {
                (
                    name.to_owned(),
                    BuildingType {
                        name: name.into(),
                        province_max: Some(10),
                        ..Default::default()
                    },
                )
            })
            .into(),
        ..Default::default()
    };
    super::set_counts(&mut assets);
    assets.equipment = content_reader_key_names::equipment::fixture::load(&[(
        "common/units/equipment/a.txt",
        "equipments = { large_plane_airframe = { is_archetype = yes type = strategic_bomber } }",
    )])
    .0;
    let mut frame = hour(countries);
    let provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    assets.trigger_content.state_provinces = BTreeMap::new();
    let mut first: BTreeMap<u32, u32> = BTreeMap::new();
    for row in &provinces {
        if row.state != 0 {
            assets
                .trigger_content
                .state_provinces
                .entry(row.state)
                .or_default()
                .push(row.id);
            first.entry(row.owner).or_insert(row.id);
        }
    }
    put::<tables::States>(
        &mut frame,
        assets
            .trigger_content
            .state_provinces
            .keys()
            .map(|&state| StateRow {
                id: state,
                owner: raw(state - 10_000),
                controller: raw(state - 10_000),
                ..Default::default()
            })
            .collect(),
    );
    let mut divisions = frame.complete::<tables::Divisions>().unwrap().to_vec();
    for division in divisions.iter_mut().take(MARINES) {
        division.roster_owner = raw(0);
        division.logical_owner = raw(0);
        division.template = TEMPLATE;
    }
    put::<tables::Divisions>(&mut frame, divisions);
    put::<tables::DivisionTemplateIcons>(
        &mut frame,
        vec![DivisionTemplateIconRow {
            template: TEMPLATE,
            subunits: vec![TemplateSubunit {
                token: MARINE,
                local: 0,
                count: 3,
            }],
            ..Default::default()
        }],
    );
    let mut countries_rows = frame.complete::<tables::Countries>().unwrap().to_vec();
    countries_rows[0].command_power_raw = 50 * 100_000;
    put::<tables::Countries>(&mut frame, countries_rows);
    put::<tables::RaidCountryInputs>(
        &mut frame,
        (0..countries)
            .map(|slot| RaidCountryInputRow {
                canonical_slot: slot,
                ..registered_launch_inputs()
            })
            .collect(),
    );
    put::<tables::CountryCommandPower>(&mut frame, registered_command_power(0..countries));
    put::<tables::Nukes>(
        &mut frame,
        (0..2)
            .map(|nuke_type| NukeRow {
                country: raw(0),
                nuke_type,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::RaidCountries>(
        &mut frame,
        (0..countries)
            .map(|slot| RaidCountryRow {
                canonical_slot: slot,
                country: raw(slot) as i32,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::LexicalTokens>(
        &mut frame,
        [
            (NAVAL_BASE, "naval_base"),
            (SUPPLY_NODE, "supply_node"),
            (MARINE, "marine"),
            (LARGE, "large_plane_airframe"),
        ]
        .map(|(token, name)| LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        })
        .into(),
    );
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    for (index, kind) in assets.catalog.types.iter().enumerate() {
        tokens.push(LexicalTokenRow {
            token: TYPE_TOKENS + index as u32,
            literal: kind.name.as_bytes().to_vec(),
        });
    }
    tokens.sort_by_key(|row| row.token);
    put::<tables::LexicalTokens>(&mut frame, tokens);
    let targets: Vec<u32> = first.values().copied().collect();
    let mut buildings: Vec<BuildingRow> = targets
        .iter()
        .flat_map(|&province| {
            [NAVAL_BASE, SUPPLY_NODE].map(|building_type| BuildingRow {
                location: province,
                building_type,
                level: 1,
                healthy_level: 1,
                ..Default::default()
            })
        })
        .collect();
    buildings.sort_by_key(|row| (row.location, row.building_type));
    put::<tables::ProvinceBuildings>(&mut frame, buildings);
    let mut bases: BTreeMap<u32, NavalBaseRow> = frame
        .complete::<tables::NavalBases>()
        .unwrap()
        .iter()
        .map(|row| (row.province, row.clone()))
        .collect();
    let owners: BTreeMap<u32, u32> = provinces.iter().map(|row| (row.id, row.owner)).collect();
    for &province in &targets {
        bases.entry(province).or_insert(NavalBaseRow {
            province,
            country: owners[&province],
            level: 1,
            healthy_level: 1,
            ..Default::default()
        });
    }
    put::<tables::NavalBases>(&mut frame, bases.into_values().collect());
    put::<tables::RaidTargetStatuses>(
        &mut frame,
        targets
            .iter()
            .flat_map(|&province| [NAVAL_BASE, SUPPLY_NODE].map(|kind| (province, kind)))
            .enumerate()
            .map(|(ordinal, (province, building_type))| RaidTargetStatusRow {
                ordinal: ordinal as u32,
                valid: 1,
                scope: RaidScope {
                    building_type,
                    building_location: province,
                    building_flags: 1,
                    ..Default::default()
                },
                detected: Vec::new(),
            })
            .collect(),
    );
    let faction: Vec<u32> = (0..300.min(countries)).collect();
    let base_id = |slot: u32| (u64::from(slot) << 32) | 0x41;
    put::<tables::AirBases>(
        &mut frame,
        faction
            .iter()
            .filter_map(|&slot| {
                Some(replay_data::rows::AirBaseRow {
                    id: base_id(slot),
                    state: Some(10_000 + slot),
                    province: Some(*first.get(&raw(slot))?),
                    capacity: 200,
                    ..Default::default()
                })
            })
            .collect(),
    );
    put::<tables::RaidAirSites>(
        &mut frame,
        faction
            .iter()
            .filter(|slot| first.contains_key(&raw(**slot)))
            .enumerate()
            .map(|(ordinal, &slot)| replay_data::rows::RaidAirSiteRow {
                canonical_slot: 0,
                ordinal: ordinal as u32,
                list: 0,
                base: base_id(slot),
                container_capacity_raw: Some(100_000),
            })
            .collect(),
    );
    let wing_id = |index: u32| (u64::from(index + 1) << 32) | 0x45;
    put::<tables::AirWings>(
        &mut frame,
        (0..WINGS)
            .map(|index| replay_data::rows::AirWingRow {
                id: wing_id(index),
                owner: raw(0),
                air_base: base_id(0),
                planes: 50,
                base_range_raw: 1000 * 100_000,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::WingEquipment>(
        &mut frame,
        (0..WINGS)
            .map(|index| replay_data::rows::WingEquipmentRow {
                wing: wing_id(index),
                ordinal: 0,
                variant: VARIANT,
                amount_raw: 20 * 100_000,
            })
            .collect(),
    );
    put::<tables::VariantDefinitions>(
        &mut frame,
        vec![replay_data::rows::VariantDefinitionRow {
            id: VARIANT,
            archetype_token: LARGE,
            ..Default::default()
        }],
    );
    let mut sources: Vec<u32> = base_provinces();
    sources.extend(targets.iter().filter(|&&province| {
        let (x, _) = xy(province);
        x < 30
    }));
    sources.sort_unstable();
    sources.dedup();
    let mut memo = Vec::new();
    for &source in &sources {
        let (sx, sy) = xy(source);
        for &target in &targets {
            let (tx, ty) = xy(target);
            memo.push(RaidPathDistanceRow {
                dispatcher: replay_data::rows::RaidPathDispatcher::Naval,
                source,
                target,
                distance: i64::from(sx.abs_diff(tx) + sy.abs_diff(ty)) * 50_000,
                ok: 1,
            });
        }
    }
    memo.extend(memo.clone().into_iter().map(|mut row| {
        row.dispatcher = replay_data::rows::RaidPathDispatcher::Land;
        row
    }));
    memo.sort_by_key(RaidPathDistanceRow::key);
    put::<tables::RaidPathDistances>(&mut frame, memo);
    (assets, frame)
}

#[cfg(not(windows))]
fn memory() -> (f64, f64) {
    (0.0, 0.0)
}

const CHURN_HOURS: i32 = 90;
const CHURN_TARGETS: usize = 48;
const FLAG: u32 = 40;
const TYPE_TOKENS: u32 = 100;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

fn churn_world() -> (RaidAssets, DecodedFrame) {
    churn_world_keeping(CHURN_TARGETS)
}

fn churn_world_keeping(keep: usize) -> (RaidAssets, DecodedFrame) {
    let (mut assets, mut frame) = world(310);
    let kinds = &mut assets.catalog.types;
    kinds[2].show_target = parse("has_war_with = FROM");
    kinds[6].available = parse("has_country_flag = reuse_flag");
    kinds[8].available = parse("num_of_nukes > 0");
    kinds[4].show_target = parse("var:target_state = { is_controlled_by = FROM }");
    kinds[7].target_types = vec![TargetType::Building(
        content_reader_raids::raids::BuildingSelector {
            types: vec!["naval_base".into()],
            minimum_level: Some(2),
            ..Default::default()
        },
    )];
    let targets = frame
        .complete::<tables::RaidTargetStatuses>()
        .unwrap()
        .to_vec();
    let step = (targets.len() / keep).max(1);
    let mut kept: Vec<_> = targets.into_iter().step_by(step).take(keep).collect();
    for (at, row) in kept.iter_mut().enumerate() {
        row.ordinal = at as u32;
    }
    put::<tables::RaidTargetStatuses>(&mut frame, kept);
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    tokens.push(LexicalTokenRow {
        token: FLAG,
        literal: b"reuse_flag".to_vec(),
    });
    tokens.sort_by_key(|row| row.token);
    put::<tables::LexicalTokens>(&mut frame, tokens);
    put::<tables::ScriptFlags>(&mut frame, Vec::new());
    put::<tables::RaidCooldowns>(&mut frame, Vec::new());
    let mut members = frame.complete::<tables::FactionMembers>().unwrap().to_vec();
    members.truncate(12);
    put::<tables::FactionMembers>(&mut frame, members);
    (assets, frame)
}

fn churn(frame: &mut DecodedFrame, rng: &mut Rng, hour: i32) -> usize {
    let targets = frame
        .complete::<tables::RaidTargetStatuses>()
        .unwrap()
        .to_vec();
    let target = &targets[rng.below(targets.len())];
    let location = target.scope.building_location;
    let family = rng.below(15);
    match family {
        0 => {
            let mut rows = targets.clone();
            let at = rng.below(rows.len());
            if rng.chance(50) {
                rows[at].valid ^= 1;
            } else if rows[at].detected.is_empty() {
                rows[at].detected = vec![1];
            } else {
                rows[at].detected.clear();
            }
            put::<tables::RaidTargetStatuses>(frame, rows);
        }
        1 => {
            let mut rows = frame.complete::<tables::Countries>().unwrap().to_vec();
            rows[0].command_power_raw = if rng.chance(50) { 5 } else { 50 } * 100_000;
            put::<tables::Countries>(frame, rows);
        }
        2 => {
            let rows = if rng.chance(60) {
                vec![RaidCooldownRow {
                    owner_slot: 0,
                    ordinal: 0,
                    raid_type: TYPE_TOKENS + rng.below(32) as u32,
                    scope: target.scope.clone(),
                    days: 0,
                }]
            } else {
                Vec::new()
            };
            put::<tables::RaidCooldowns>(frame, rows);
        }
        3 => {
            let mut rows = frame.complete::<tables::Divisions>().unwrap().to_vec();
            let at = rng.below(MARINES);
            let (x, y) = (rng.below(30) as u32, rng.below(HEIGHT as usize) as u32);
            rows[at].province = super::reach_bench::id(x, y);
            put::<tables::Divisions>(frame, rows);
        }
        4 => {
            let mut rows = frame.complete::<tables::AirWings>().unwrap().to_vec();
            let at = rng.below(rows.len());
            if rng.chance(50) {
                rows[at].base_range_raw = (100 + rng.below(1200) as i64) * 100_000;
            } else {
                rows[at].air_base = (u64::from(rng.below(300) as u32) << 32) | 0x41;
            }
            put::<tables::AirWings>(frame, rows);
        }
        5 => {
            let mut rows = frame.complete::<tables::RaidAirSites>().unwrap().to_vec();
            if rng.chance(50) && rows.len() > 1 {
                rows.remove(rng.below(rows.len()));
            } else if let Some(last) = rows.last().cloned() {
                rows.push(replay_data::rows::RaidAirSiteRow {
                    ordinal: last.ordinal + 1,
                    base: (u64::from(rng.below(300) as u32) << 32) | 0x41,
                    ..last
                });
            }
            put::<tables::RaidAirSites>(frame, rows);
        }
        6 => {
            let mut rows = frame
                .complete::<tables::RaidPathDistances>()
                .unwrap()
                .to_vec();
            if rng.chance(50) && !rows.is_empty() {
                rows.remove(rng.below(rows.len()));
            } else {
                rows.push(RaidPathDistanceRow {
                    dispatcher: replay_data::rows::RaidPathDispatcher::Naval,
                    source: base_provinces()[rng.below(10)],
                    target: location,
                    distance: rng.below(40) as i64 * 50_000,
                    ok: rng.chance(80) as u8,
                });
                rows.sort_by_key(RaidPathDistanceRow::key);
                rows.dedup_by_key(|row| row.key());
            }
            put::<tables::RaidPathDistances>(frame, rows);
        }
        7 => {
            let mut rows = frame.complete::<tables::FactionMembers>().unwrap().to_vec();
            let country = raw(rng.below(310) as u32);
            match rows.iter().position(|row| row.country == country) {
                Some(at) => {
                    rows.remove(at);
                }
                None => rows.push(FactionMemberRow {
                    country,
                    faction: 1,
                    ordinal: 1_000 + hour as u32,
                }),
            }
            let faction = rows
                .iter()
                .find(|row| row.country == raw(0))
                .map_or(0, |row| row.faction);
            put::<tables::FactionMembers>(frame, rows);
            reserved_faction(frame, faction);
        }
        8 => {
            let mut rows = frame.complete::<tables::WarRelations>().unwrap().to_vec();
            let enemy = raw(300 + rng.below(10) as u32);
            match rows.iter().position(|row| row.first_country == raw(0)) {
                Some(at) if rng.chance(50) => rows[at].end_hour = hour,
                _ => rows.push(replay_data::rows::WarRelationRow {
                    first_country: raw(0),
                    second_country: enemy,
                    start_hour: hour,
                    ..Default::default()
                }),
            }
            put::<tables::WarRelations>(frame, rows);
        }
        9 => {
            let rows = if rng.chance(50) {
                vec![replay_data::rows::ScriptFlagRow {
                    scope_kind: 1,
                    scope_id: raw(0),
                    token: FLAG,
                    value: 1,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            };
            put::<tables::ScriptFlags>(frame, rows);
        }
        10 => {
            let other = raw(rng.below(310) as u32);
            if rng.chance(50) {
                let mut rows = frame.complete::<tables::Provinces>().unwrap().to_vec();
                if let Some(row) = rows.iter_mut().find(|row| row.id == location) {
                    row.controller = other;
                }
                put::<tables::Provinces>(frame, rows);
            } else {
                let state = frame
                    .complete::<tables::Provinces>()
                    .unwrap()
                    .iter()
                    .find(|row| row.id == location)
                    .map(|row| row.state);
                let mut rows = frame.complete::<tables::States>().unwrap().to_vec();
                if let Some(row) = rows.iter_mut().find(|row| Some(row.id) == state) {
                    row.controller = other;
                }
                put::<tables::States>(frame, rows);
            }
        }
        11 => {
            let mut rows = frame
                .complete::<tables::ProvinceBuildings>()
                .unwrap()
                .to_vec();
            if let Some(row) = rows.iter_mut().find(|row| row.location == location) {
                row.level = if row.level == 1 { 2 } else { 1 };
            }
            put::<tables::ProvinceBuildings>(frame, rows);
        }
        12 => {
            let mut rows = frame.complete::<tables::NavalBases>().unwrap().to_vec();
            match rows.iter().position(|row| row.province == location) {
                Some(at) => {
                    rows.remove(at);
                }
                None => {
                    rows.push(NavalBaseRow {
                        province: location,
                        level: 1,
                        healthy_level: 1,
                        ..Default::default()
                    });
                    rows.sort_by_key(|row| row.province);
                }
            }
            put::<tables::NavalBases>(frame, rows);
        }
        13 => {
            let mut rows = frame.complete::<tables::Nukes>().unwrap().to_vec();
            if let Some(row) = rows.first_mut() {
                row.amount_raw = if row.amount_raw == 0 {
                    10_000_000_000_000
                } else {
                    0
                };
            }
            put::<tables::Nukes>(frame, rows);
        }
        _ => {}
    }
    family
}

fn shown(icons: &RaidIcons) -> String {
    format!("{:?}", (&icons.icons, &icons.report))
}

fn evaluated_at(cycle: &cycle::Cycle) -> Vec<Option<replay_data::Hour>> {
    (0..cycle.progress().1)
        .map(|at| cycle.kept(at).map(|kept| kept.0))
        .collect()
}

fn run_hour(
    assets: &RaidAssets,
    frame: &DecodedFrame,
    cycle: &mut cycle::Cycle,
    steps: usize,
) -> Result<RaidIcons, String> {
    let paths = paths::HourPaths::new(assets, frame);
    let world = World::new(frame, assets.trigger_content.facts()).unwrap();
    let layer = HourLayer::new(assets, &world, 0, &paths);
    cycle.begin(assets, &layer.sweep)?;
    for _ in 0..steps {
        cycle.step(&layer.sweep);
    }
    Ok(layer.icons(cycle, &mut IconCache::default()))
}

fn append_target(frame: &mut DecodedFrame, province: u32) {
    let mut rows = frame
        .complete::<tables::RaidTargetStatuses>()
        .unwrap()
        .to_vec();
    rows.push(RaidTargetStatusRow {
        ordinal: rows.len() as u32,
        valid: 1,
        scope: RaidScope {
            building_type: SUPPLY_NODE,
            building_location: province,
            building_flags: 1,
            ..Default::default()
        },
        detected: Vec::new(),
    });
    put::<tables::RaidTargetStatuses>(frame, rows);
}

#[test]
fn a_cycle_evaluates_a_batch_per_hour_from_a_rolling_cursor() {
    content_reader_script::test_game::enter();
    let (mut assets, mut frame) = churn_world_keeping(5);
    assets.batch = Ok(2);
    let mut cycle = cycle::Cycle::new(frame.branch, 0);
    for hour in 0..3 {
        frame.hour = hour;
        run_hour(&assets, &frame, &mut cycle, 1).unwrap();
    }
    assert_eq!(
        evaluated_at(&cycle),
        [Some(2), Some(0), Some(1), Some(1), Some(2)],
        "rows 0-1, 2-3, then 4 and 0"
    );
    assert_eq!(cycle.progress(), (2, 5));
    frame.hour = 3;
    append_target(&mut frame, 7);
    let before = cycle.clone();
    let icons = run_hour(&assets, &frame, &mut cycle, 1).unwrap();
    assert_eq!(icons.report.cycle, (2, 6));
    assert!(!icons.report.cycle_complete());
    assert!(
        icons
            .report
            .warnings()
            .iter()
            .any(|line| line.contains("2 of 6 targets"))
    );
    assert_eq!(
        evaluated_at(&cycle),
        [Some(2), Some(3), Some(3), Some(1), Some(2), None]
    );
    let paths = paths::HourPaths::new(&assets, &frame);
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let layer = HourLayer::new(&assets, &world, 0, &paths);
    let mut paused = before;
    paused.begin(&assets, &layer.sweep).unwrap();
    assert_eq!(
        paused.progress(),
        (0, 6),
        "a rebuild frame makes every row pending"
    );
    for _ in 0..3 {
        paused.step(&layer.sweep);
    }
    assert!(paused.complete());
    assert_eq!(evaluated_at(&paused), [Some(3); 6]);
    paused.step(&layer.sweep);
    assert_eq!(
        paused.progress(),
        (6, 6),
        "a complete hour evaluates nothing more"
    );
}

#[test]
fn a_kept_row_shows_its_last_evaluation_until_evaluated_again() {
    content_reader_script::test_game::enter();
    let (mut assets, mut frame) = churn_world();
    assets.batch = Ok(10);
    let mut cycle = cycle::Cycle::new(frame.branch, 0);
    let first = run_hour(&assets, &frame, &mut cycle, 1).unwrap();
    let kept = cycle.kept(3).cloned().expect("row 3 is in the first batch");
    let full = icons(&assets, &frame, 0, &paths::HourPaths::new(&assets, &frame));
    assert!(first.report.offers < full.report.offers);
    assert_eq!(first.report.cycle, (10, CHURN_TARGETS));
    frame.hour = 1;
    let mut members = frame.complete::<tables::FactionMembers>().unwrap().to_vec();
    members.extend((12..310).map(|slot| FactionMemberRow {
        country: raw(slot),
        faction: 1,
        ordinal: 1_000 + slot,
    }));
    put::<tables::FactionMembers>(&mut frame, members);
    run_hour(&assets, &frame, &mut cycle, 1).unwrap();
    assert!(Arc::ptr_eq(&kept, &cycle.kept(3).cloned().unwrap()));
    assert_eq!(cycle.kept(12).map(|kept| kept.0), Some(1));
    assert!(cycle.kept(25).is_none());
}

#[test]
fn a_changed_or_lost_row_starts_the_cycle_again() {
    content_reader_script::test_game::enter();
    let (mut assets, mut frame) = churn_world();
    assets.batch = Ok(10);
    let mut cycle = cycle::Cycle::new(frame.branch, 0);
    for hour in 0..5 {
        frame.hour = hour;
        run_hour(&assets, &frame, &mut cycle, 1).unwrap();
    }
    let mut lost = frame.clone();
    lost.hour = 5;
    let mut rows = lost
        .complete::<tables::RaidTargetStatuses>()
        .unwrap()
        .to_vec();
    rows.remove(20);
    for (at, row) in rows.iter_mut().enumerate() {
        row.ordinal = at as u32;
    }
    put::<tables::RaidTargetStatuses>(&mut lost, rows);
    let reason = run_hour(&assets, &lost, &mut cycle.clone(), 1).unwrap_err();
    assert!(reason.contains("went from 48 rows to 47"), "{reason}");
    let mut moved = frame.clone();
    moved.hour = 5;
    let mut rows = moved
        .complete::<tables::RaidTargetStatuses>()
        .unwrap()
        .to_vec();
    rows[20].scope.building_location += 1;
    put::<tables::RaidTargetStatuses>(&mut moved, rows);
    let reason = run_hour(&assets, &moved, &mut cycle.clone(), 1).unwrap_err();
    assert!(
        reason.contains("status 20 names another target"),
        "{reason}"
    );
    let mut again = cycle::Cycle::new(lost.branch, 0);
    let icons = run_hour(&assets, &lost, &mut again, 1).unwrap();
    assert_eq!(icons.report.cycle, (10, 47));
    assert_eq!(
        evaluated_at(&again)
            .iter()
            .filter(|hour| hour.is_some())
            .count(),
        10
    );
}

#[test]
fn stopped_and_full_army_searches_answer_alike() {
    content_reader_script::test_game::enter();
    let (assets, mut frame) = churn_world();
    let mut rng = Rng(0x0bad_5eed_1234_5678);
    for hour in 0..6 {
        frame.hour = hour;
        churn(&mut frame, &mut rng, hour);
    }
    let id = super::reach_bench::id;
    let sources: Vec<u32> = (0..HEIGHT)
        .step_by(7)
        .flat_map(|y| [id(3, y), id(20, y), id(25, y), id(40, y)])
        .collect();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let cells = reach::Cells::default();
    let moves = reach::Moves::new(&world, assets.movement_reads(), &cells);
    let graph = assets.moves.as_ref().unwrap();
    let mover = content_reader_unit_path::unit_path::Mover {
        graph,
        world: &moves,
    };
    for division in 1..=MARINES as u64 {
        let army = moves.army(division).unwrap();
        let full = mover.reach(&army);
        for &source in &sources {
            let expected = match mover.army_reaches_in(&army, source, assets.avoid_enemy, || &full)
            {
                Ok(reached) => Eval::from_bool(reached),
                Err(reason) => Eval::unknown(reason),
            };
            assert_eq!(
                moves.reaches(division, source),
                expected,
                "{division} {source}"
            );
        }
    }
}

#[test]
fn a_complete_cycle_equals_a_full_evaluation_under_random_changes() {
    content_reader_script::test_game::enter();
    for seed in [0x9e37_79b9_7f4a_7c15, 0x2545_f491_4f6c_dd1d] {
        let (mut assets, mut frame) = churn_world();
        assets.batch = Ok(7);
        let mut rng = Rng(seed);
        let mut cycle = cycle::Cycle::new(frame.branch, 0);
        let mut cache = IconCache::default();
        let (mut pauses, mut families) = (0, 0_u16);
        for hour in 0..CHURN_HOURS {
            frame.hour = hour;
            for _ in 0..1 + rng.below(3) {
                families |= 1 << churn(&mut frame, &mut rng, hour);
            }
            let paths = paths::HourPaths::new(&assets, &frame);
            let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
            let layer = HourLayer::new(&assets, &world, 0, &paths);
            cycle.begin(&assets, &layer.sweep).unwrap();
            let mut reference = cycle.clone();
            cycle.step(&layer.sweep);
            assert_eq!(
                shown(&layer.icons(&cycle, &mut cache)),
                shown(&layer.icons(&cycle, &mut IconCache::default())),
                "seed {seed:#x} hour {hour}: kept icons"
            );
            if rng.chance(15) {
                while !cycle.complete() {
                    cycle.step(&layer.sweep);
                }
                let paths = paths::HourPaths::new(&assets, &frame);
                let fresh = HourLayer::new(&assets, &world, 0, &paths);
                reference.begin(&assets, &fresh.sweep).unwrap();
                while !reference.complete() {
                    reference.step(&fresh.sweep);
                }
                assert_eq!(
                    shown(&layer.icons(&cycle, &mut cache)),
                    shown(&fresh.icons(&reference, &mut IconCache::default())),
                    "seed {seed:#x} hour {hour}"
                );
                pauses += 1;
                if pauses > 5 && families == (1 << 15) - 1 {
                    break;
                }
            }
        }
        assert!(pauses > 5, "seed {seed:#x}: {pauses} pauses");
        assert_eq!(
            families,
            (1 << 15) - 1,
            "seed {seed:#x}: every input family"
        );
    }
}

#[test]
fn a_cycle_carried_across_skipped_hours_equals_a_full_evaluation() {
    content_reader_script::test_game::enter();
    let (mut assets, mut frame) = churn_world();
    assets.batch = Ok(7);
    let mut rng = Rng(0xd1b5_4a32_d192_ed03);
    let mut cycle = cycle::Cycle::new(frame.branch, 0);
    let mut cache = IconCache::default();
    let (mut skips, mut pauses) = (0, 0);
    for hour in 0..CHURN_HOURS {
        frame.hour = hour;
        for _ in 0..1 + rng.below(3) {
            churn(&mut frame, &mut rng, hour);
        }
        if hour > 0 && rng.chance(40) {
            skips += 1;
            continue;
        }
        let paths = paths::HourPaths::new(&assets, &frame);
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let layer = HourLayer::new(&assets, &world, 0, &paths);
        cycle.begin(&assets, &layer.sweep).unwrap();
        let mut reference = cycle.clone();
        cycle.step(&layer.sweep);
        if rng.chance(20) {
            while !cycle.complete() {
                cycle.step(&layer.sweep);
            }
            let paths = paths::HourPaths::new(&assets, &frame);
            let fresh = HourLayer::new(&assets, &world, 0, &paths);
            reference.begin(&assets, &fresh.sweep).unwrap();
            while !reference.complete() {
                reference.step(&fresh.sweep);
            }
            assert_eq!(
                shown(&layer.icons(&cycle, &mut cache)),
                shown(&fresh.icons(&reference, &mut IconCache::default())),
                "hour {hour}"
            );
            pauses += 1;
        }
    }
    assert!(
        skips > 20 && pauses > 3,
        "{skips} skips, {pauses} pauses"
    );
}

#[test]
fn a_complete_cycle_equals_the_full_evaluation_from_row_zero() {
    content_reader_script::test_game::enter();
    let (mut assets, mut frame) = churn_world();
    assets.batch = Ok(5);
    let mut cycle = cycle::Cycle::new(frame.branch, 0);
    for hour in 0..4 {
        frame.hour = hour;
        run_hour(&assets, &frame, &mut cycle, 1).unwrap();
    }
    let paths = paths::HourPaths::new(&assets, &frame);
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let layer = HourLayer::new(&assets, &world, 0, &paths);
    cycle.begin(&assets, &layer.sweep).unwrap();
    while !cycle.complete() {
        cycle.step(&layer.sweep);
    }
    let full = icons(&assets, &frame, 0, &paths::HourPaths::new(&assets, &frame));
    assert!(full.report.offers > 0);
    assert_eq!(
        shown(&layer.icons(&cycle, &mut IconCache::default())),
        shown(&full)
    );
}

fn transcript(cycle: &cycle::Cycle, icons: &RaidIcons) -> String {
    let rows: Vec<String> = (0..cycle.progress().1)
        .map(|at| format!("{:?}", cycle.kept(at)))
        .collect();
    format!("{rows:?}\n{}", shown(icons))
}

fn churn_hours(seed: u64, hours: i32, unrecorded: bool) -> (RaidAssets, Vec<DecodedFrame>) {
    let (assets, mut frame) = churn_world();
    if unrecorded {
        let rows = frame
            .complete::<tables::RaidTargetStatuses>()
            .unwrap()
            .to_vec();
        let paired = rows
            .into_iter()
            .flat_map(|row| {
                let mut supply = row.clone();
                supply.scope.building_type = SUPPLY_NODE;
                [row, supply]
            })
            .enumerate()
            .map(|(at, mut row)| {
                row.ordinal = at as u32;
                row
            })
            .collect();
        put::<tables::RaidTargetStatuses>(&mut frame, paired);
    }
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    for hour in 0..hours {
        frame.hour = hour;
        for _ in 0..1 + rng.below(3) {
            churn(&mut frame, &mut rng, hour);
        }
        if hour % 5 == 4 {
            let members: BTreeSet<u32> = frame
                .complete::<tables::FactionMembers>()
                .unwrap()
                .iter()
                .map(|row| row.country)
                .collect();
            let mut rows = frame.complete::<tables::Provinces>().unwrap().to_vec();
            let owned: Vec<usize> = (0..rows.len())
                .filter(|&at| members.contains(&rows[at].owner))
                .collect();
            let member = owned[rng.below(owned.len())];
            let other = rng.below(rows.len());
            if rng.chance(50) {
                let content = &assets.trigger_content.state_provinces[&rows[member].state];
                let mut buildings = frame
                    .complete::<tables::ProvinceBuildings>()
                    .unwrap()
                    .to_vec();
                buildings.retain(|row| !content.contains(&row.location));
                put::<tables::ProvinceBuildings>(&mut frame, buildings);
            }
            if rng.chance(50) {
                rows[other].state = rows[member].state;
            } else {
                rows[member].state = rows[other].state;
            }
            put::<tables::Provinces>(&mut frame, rows);
        }
        if unrecorded {
            put::<tables::RaidPathDistances>(&mut frame, Vec::new());
        }
        out.push(frame.clone());
    }
    (assets, out)
}

fn split_run(
    assets: &RaidAssets,
    frames: &[DecodedFrame],
    workers: &Workers,
    part_rows: usize,
) -> (Vec<String>, usize, usize) {
    let mut cycle = cycle::Cycle::new(frames[0].branch, 0);
    let (mut out, mut again, mut prefetched) = (Vec::new(), 0, 0);
    for (hour, frame) in frames.iter().enumerate() {
        let paths = paths::HourPaths::new(assets, frame);
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let mut layer = HourLayer::new(assets, &world, 0, &paths);
        layer.sweep.workers = workers.clone();
        layer.sweep.part_rows = part_rows;
        if cycle.begin(assets, &layer.sweep).is_err() {
            cycle = cycle::Cycle::new(frame.branch, 0);
            cycle.begin(assets, &layer.sweep).unwrap();
        }
        loop {
            cycle.step(&layer.sweep);
            out.push(transcript(
                &cycle,
                &layer.icons(&cycle, &mut IconCache::default()),
            ));
            if hour % 4 != 3 || cycle.complete() {
                break;
            }
        }
        again += layer.sweep.again.load(std::sync::atomic::Ordering::Relaxed);
        prefetched += layer
            .sweep
            .prefetched
            .load(std::sync::atomic::Ordering::Relaxed);
    }
    (out, again, prefetched)
}

#[test]
fn a_batch_split_across_threads_equals_the_batch_in_order() {
    content_reader_script::test_game::enter();
    let pool = viewer_runtime::builder_pool::builder_workers().unwrap();
    let (mut again, mut prefetched) = (0, 0);
    let ahead = reach::RAN_AHEAD.load(std::sync::atomic::Ordering::Relaxed);
    for (seed, batch, part_rows, unrecorded) in [
        (0x9e37_79b9_7f4a_7c15, 2 * CHURN_TARGETS, 1, true),
        (0x2545_f491_4f6c_dd1d, 30, 3, true),
        (0x6a09_e667_f3bc_c908, 7, 2, true),
        (0xd1b5_4a32_d192_ed03, 30, 8, false),
        (0x0bad_5eed_1234_5678, 5, 8, false),
    ] {
        let (mut assets, frames) = churn_hours(seed, 16, unrecorded);
        assets.batch = Ok(batch);
        let (reference, _, _) = split_run(&assets, &frames, &Default::default(), part_rows);
        for threads in [1, 3, 8] {
            let workers = pool.clone().limited(threads);
            let (split, parts_again, tasks) = split_run(&assets, &frames, &workers, part_rows);
            again += parts_again;
            prefetched += tasks;
            assert_eq!(split.len(), reference.len());
            for (at, (split, reference)) in split.iter().zip(&reference).enumerate() {
                assert!(
                    split == reference,
                    "seed {seed:#x}, batch {batch}, {threads} threads: batch {at} differs\n{split}\n{reference}"
                );
            }
        }
    }
    assert!(again > 0, "no part was evaluated again");
    assert!(prefetched > 0, "no batch prefetched");
    assert!(
        reach::RAN_AHEAD.load(std::sync::atomic::Ordering::Relaxed) > ahead,
        "no search ran ahead"
    );
}
