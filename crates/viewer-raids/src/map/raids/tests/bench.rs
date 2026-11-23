
use std::time::{Duration, Instant};

use super::*;
use content_reader_raids::{
    raids::BattalionRequirement, raids::EquipmentRequirement, raids::SourceKind,
    raids::StartingPoint,
};
use replay_data::{
    rows::AirBaseRow, rows::AirWingRow, rows::DivisionMovementRow, rows::DivisionRow,
    rows::DivisionStatusRow, rows::DivisionTemplateIconRow, rows::NavalBaseRow, rows::NukeRow,
    rows::RaidAirSiteRow, rows::RaidCountryInputRow, rows::RaidPathDistanceRow,
    rows::StockpileArchetypeRow, rows::StockpileRow, rows::TemplateSubunit,
    rows::VariantDefinitionRow, rows::VariantStatsRow, rows::WingEquipmentRow,
};

const LAND: u32 = 6;
const COLUMN: u32 = LAND + 1;
const FACTION: u32 = 50;
const DIVISIONS: u32 = 20;
const WINGS: u32 = 3;
const HOURS: i32 = 96;

const NAVAL_COMMANDO: u32 = 1;
const AIRBORNE: u32 = 2;
const BOMBER: u32 = 3;
const SABOTAGE: u32 = 4;
const NAVAL_BASE: u32 = 10;
const AIR_BASE: u32 = 11;
const MARINE: u32 = 20;
const PARATROOPER: u32 = 21;
const INFANTRY: u32 = 22;
const LARGE_PLANE: u32 = 30;
const TRANSPORT_PLANE: u32 = 31;

fn province(country: u32, row: u32) -> u32 {
    country * COLUMN + row + 1
}

fn raw(country: u32) -> u32 {
    1000 + country
}

fn kinds() -> Vec<RaidType> {
    let base = |name: &str, target: &str| RaidType {
        name: name.into(),
        category: Some("army".into()),
        show_target: parse("NOT = { is_in_faction_with = FROM }"),
        target_types: vec![TargetType::Building(
            content_reader_raids::raids::BuildingSelector {
                types: vec![target.into()],
                minimum_level: Some(1),
                ..Default::default()
            },
        )],
        command_power_raw: 10 * 100_000,
        max_distance_raw: Some(400 * 100_000),
        ..Default::default()
    };
    let battalions = |name: &str| {
        vec![content_reader_raids::raids::UnitRequirement {
            battalions: vec![BattalionRequirement {
                name: name.into(),
                min: Some(2),
                max: None,
            }],
            ..Default::default()
        }]
    };
    vec![
        RaidType {
            starting_point: StartingPoint {
                types: vec![SourceKind::NavalBase],
                ..Default::default()
            },
            unit_requirements: battalions("marine"),
            ..base("naval_commando", "naval_base")
        },
        RaidType {
            starting_point: StartingPoint {
                types: vec![SourceKind::AirBase],
                ..Default::default()
            },
            unit_requirements: battalions("paratrooper"),
            essential_equipment: vec![("transport_plane".into(), 1)],
            ..base("airborne", "air_base")
        },
        RaidType {
            starting_point: StartingPoint {
                types: vec![SourceKind::AirBase],
                ..Default::default()
            },
            unit_requirements: vec![content_reader_raids::raids::UnitRequirement {
                equipment: vec![EquipmentRequirement {
                    types: vec!["strategic_bomber".into()],
                    min: Some(10),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..base("bomber", "air_base")
        },
        RaidType {
            starting_point: StartingPoint {
                building_types: vec!["naval_base".into()],
                allow_faction_buildings: true,
                ..Default::default()
            },
            unit_requirements: battalions("paratrooper"),
            ..base("sabotage", "naval_base")
        },
    ]
}

fn world(countries: u32) -> (RaidAssets, DecodedFrame) {
    let mut assets = assets();
    let provinces = countries * COLUMN;
    let mut geometry = ProvinceGeometry {
        width: (countries * 4) as i32,
        height: (COLUMN * 4) as i32,
        ..Default::default()
    };
    let mut graph = content_reader_map::province_graph::ProvinceGraph::default();
    for country in 0..countries {
        for row in 0..COLUMN {
            let id = province(country, row);
            let (x, y) = ((country * 4) as i32, (row * 4) as i32);
            geometry.centroids.insert(id, [x + 2, y + 2]);
            geometry.bounds.insert(id, [x, y, 4, 4]);
            if row < LAND {
                geometry.land.insert(id);
            }
            let mut next = Vec::new();
            if row > 0 {
                next.push(province(country, row - 1));
            }
            if row + 1 < COLUMN {
                next.push(province(country, row + 1));
            }
            if country > 0 {
                next.push(province(country - 1, row));
            }
            if country + 1 < countries {
                next.push(province(country + 1, row));
            }
            graph
                .edges
                .insert(id, next.into_iter().map(|n| (n, Some(400_000))).collect());
        }
    }
    assets.geometry = geometry;
    assets.paths = Ok(paths::PathSearch::new(graph));
    assets.trigger_content.state_provinces = (0..countries)
        .map(|country| {
            (
                country + 1,
                (0..LAND).map(|row| province(country, row)).collect(),
            )
        })
        .collect();
    assets.sites = (0..countries)
        .map(|country| ((country + 1, Site::AirBase), vec![province(country, 0)]))
        .collect();
    assets.catalog = Catalog {
        types: kinds(),
        ..Default::default()
    };
    assets.trigger_content.buildings = content_reader_key_names::buildings::Buildings {
        types: ["naval_base", "air_base"]
            .map(|name| {
                (
                    name.to_owned(),
                    BuildingType {
                        name: name.into(),
                        air_base: name == "air_base",
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
        "equipments = { large_plane_airframe = { is_archetype = yes type = strategic_bomber } transport_plane = { is_archetype = yes type = air_transport } }",
    )])
    .0;
    assets.defines = RaidDefines {
        base_max_command_power: 100 * 100_000,
        naval_invasion_priority: 1,
        map_scale: 711_400,
        naval_commando_distance: 1500 * 100_000,
        ace_wing_size: 100 * 100_000,
        ace_wing_size_max_bonus: 300_000,
        unit_speeds: None,
    };

    let mut frame = frame();
    frame.hour = 0;
    let slots = 0..countries;
    put::<tables::Countries>(
        &mut frame,
        slots
            .clone()
            .map(|slot| CountryRow {
                id: raw(slot),
                canonical_slot: slot,
                tag: format!("C{slot:04}").into_bytes(),
                command_power_raw: 50 * 100_000,
                convoys_raw: Some(0),
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::CountryLinks>(
        &mut frame,
        slots
            .clone()
            .map(|slot| CountryLinkRow {
                id: raw(slot),
                canonical_slot: slot,
            })
            .collect(),
    );
    put::<tables::FactionMembers>(
        &mut frame,
        slots
            .clone()
            .map(|slot| FactionMemberRow {
                country: raw(slot),
                faction: u64::from(slot / FACTION) + 1,
                ordinal: slot % FACTION,
            })
            .collect(),
    );
    put::<tables::Provinces>(
        &mut frame,
        (1..=provinces)
            .map(|id| {
                let (country, row) = ((id - 1) / COLUMN, (id - 1) % COLUMN);
                let land = row < LAND;
                ProvinceRow {
                    id,
                    state: if land { country + 1 } else { 0 },
                    owner: if land { raw(country) } else { 0 },
                    controller: if land { raw(country) } else { 0 },
                    port_sea: if land {
                        province(country, LAND) as i32
                    } else {
                        0
                    },
                    ..Default::default()
                }
            })
            .collect(),
    );
    put::<tables::States>(
        &mut frame,
        slots
            .clone()
            .map(|slot| StateRow {
                id: slot + 1,
                owner: raw(slot),
                controller: raw(slot),
                ..Default::default()
            })
            .collect(),
    );
    let names = [
        (NAVAL_COMMANDO, "naval_commando"),
        (AIRBORNE, "airborne"),
        (BOMBER, "bomber"),
        (SABOTAGE, "sabotage"),
        (NAVAL_BASE, "naval_base"),
        (AIR_BASE, "air_base"),
        (MARINE, "marine"),
        (PARATROOPER, "paratrooper"),
        (INFANTRY, "infantry"),
        (LARGE_PLANE, "large_plane_airframe"),
        (TRANSPORT_PLANE, "transport_plane"),
    ];
    put::<tables::LexicalTokens>(
        &mut frame,
        names
            .map(|(token, name)| LexicalTokenRow {
                token,
                literal: name.as_bytes().to_vec(),
            })
            .into(),
    );
    let eligible = vec![NAVAL_COMMANDO, AIRBORNE, BOMBER, SABOTAGE];
    put::<tables::RaidCountries>(
        &mut frame,
        slots
            .clone()
            .map(|slot| RaidCountryRow {
                canonical_slot: slot,
                country: raw(slot) as i32,
                eligible_types: eligible.clone(),
                used_types: Vec::new(),
            })
            .collect(),
    );
    put::<tables::RaidCountryInputs>(
        &mut frame,
        slots
            .clone()
            .map(|slot| RaidCountryInputRow {
                canonical_slot: slot,
                autonomous: Some(0),
                ..registered_launch_inputs()
            })
            .collect(),
    );
    put::<tables::CountryCommandPower>(&mut frame, registered_command_power(slots.clone()));
    put::<tables::Nukes>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..2).map(move |nuke_type| NukeRow {
                    country: raw(slot),
                    nuke_type,
                    ..Default::default()
                })
            })
            .collect(),
    );
    put::<tables::ProvinceBuildings>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..LAND).map(move |row| BuildingRow {
                    location: province(slot, row),
                    building_type: NAVAL_BASE,
                    country: raw(slot),
                    level: 3,
                    healthy_level: 3,
                    ..Default::default()
                })
            })
            .collect(),
    );
    put::<tables::StateBuildings>(
        &mut frame,
        slots
            .clone()
            .map(|slot| BuildingRow {
                location: slot + 1,
                building_type: AIR_BASE,
                country: raw(slot),
                level: 5,
                healthy_level: 5,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::NavalBases>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..LAND).map(move |row| NavalBaseRow {
                    province: province(slot, row),
                    country: raw(slot),
                    level: 3,
                    healthy_level: 3,
                    ..Default::default()
                })
            })
            .collect(),
    );
    let base_id = |slot: u32| (u64::from(slot) << 32) | 0x41;
    put::<tables::AirBases>(
        &mut frame,
        slots
            .clone()
            .map(|slot| AirBaseRow {
                id: base_id(slot),
                state: Some(slot + 1),
                province: Some(province(slot, 0)),
                capacity: 200,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::RaidAirSites>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                let sites: Vec<u32> = if slot % FACTION == 0 {
                    (slot..(slot + FACTION).min(countries)).collect()
                } else {
                    vec![slot]
                };
                sites
                    .into_iter()
                    .enumerate()
                    .map(move |(ordinal, site)| RaidAirSiteRow {
                        canonical_slot: slot,
                        ordinal: ordinal as u32,
                        list: 0,
                        base: base_id(site),
                        container_capacity_raw: Some(100_000),
                    })
            })
            .collect(),
    );
    let template = |kind: u32| 0x0000_0005_0000_0000 | u64::from(kind);
    put::<tables::DivisionTemplateIcons>(
        &mut frame,
        [INFANTRY, MARINE, PARATROOPER]
            .map(|kind| DivisionTemplateIconRow {
                template: template(kind),
                subunits: vec![TemplateSubunit {
                    token: kind,
                    local: 0,
                    count: 4,
                }],
                ..Default::default()
            })
            .into(),
    );
    let division_id = |slot: u32, index: u32| (u64::from(slot * DIVISIONS + index) << 32) | 6;
    let mut divisions = Vec::new();
    for slot in slots.clone() {
        for index in 0..DIVISIONS {
            let kind = match (slot % FACTION, index % 2) {
                (0, 0) => MARINE,
                (0, _) => PARATROOPER,
                _ => INFANTRY,
            };
            divisions.push(DivisionRow {
                id: division_id(slot, index),
                roster_owner: raw(slot),
                logical_owner: raw(slot),
                province: province(slot, index % LAND),
                template: template(kind),
                hit_points: 100_000,
                organisation: 500_000,
                ..Default::default()
            });
        }
    }
    put::<tables::DivisionMovement>(
        &mut frame,
        divisions
            .iter()
            .map(|division| DivisionMovementRow {
                division: division.id,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::DivisionStatus>(
        &mut frame,
        divisions
            .iter()
            .map(|division| DivisionStatusRow {
                division: division.id,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::Divisions>(&mut frame, divisions);
    let wing_id = |slot: u32, index: u32| (u64::from(slot * WINGS + index) << 32) | 0x45;
    let variant = |slot: u32, index: u32| (u64::from(slot * 4 + index) << 32) | 0x46;
    put::<tables::AirWings>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..WINGS).map(move |index| AirWingRow {
                    id: wing_id(slot, index),
                    owner: raw(slot),
                    air_base: base_id(slot),
                    planes: 50,
                    base_range_raw: 1000 * 100_000,
                    ..Default::default()
                })
            })
            .collect(),
    );
    put::<tables::WingEquipment>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..WINGS).map(move |index| WingEquipmentRow {
                    wing: wing_id(slot, index),
                    ordinal: 0,
                    variant: variant(slot, 0),
                    amount_raw: 20 * 100_000,
                })
            })
            .collect(),
    );
    put::<tables::VariantDefinitions>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..4).map(move |index| VariantDefinitionRow {
                    id: variant(slot, index),
                    archetype_token: if index == 1 {
                        TRANSPORT_PLANE
                    } else {
                        LARGE_PLANE
                    },
                    classifier_flags: if index == 1 { 1 << 20 } else { 0 },
                    ..Default::default()
                })
            })
            .collect(),
    );
    put::<tables::VariantStats>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..4).map(move |index| VariantStatsRow {
                    variant: variant(slot, index),
                    values: [Some(1500 * 100_000); 10],
                    ..Default::default()
                })
            })
            .collect(),
    );
    put::<tables::Stockpiles>(
        &mut frame,
        slots
            .clone()
            .flat_map(|slot| {
                (0..4).map(move |index| StockpileRow {
                    country: raw(slot),
                    ordinal: index,
                    variant_handle: variant(slot, index),
                    amount_raw: 100 * 100_000,
                })
            })
            .collect(),
    );
    put::<tables::StockpileArchetypes>(
        &mut frame,
        slots
            .clone()
            .map(|slot| StockpileArchetypeRow {
                canonical_slot: slot,
                archetype: TRANSPORT_PLANE,
                units: 100,
            })
            .collect(),
    );
    let mut targets = Vec::new();
    for slot in slots.clone() {
        let mut target = |scope: RaidScope| {
            targets.push(RaidTargetStatusRow {
                ordinal: targets.len() as u32,
                valid: 1,
                scope,
                detected: Vec::new(),
            })
        };
        target(RaidScope {
            building_type: NAVAL_BASE,
            building_location: province(slot, LAND - 1),
            building_flags: 1,
            ..Default::default()
        });
        if slot % 5 == 0 {
            target(RaidScope {
                building_type: AIR_BASE,
                building_location: slot + 1,
                building_flags: 2,
                ..Default::default()
            });
        }
    }
    let mut memo = Vec::new();
    for member in 0..FACTION.min(countries) {
        for row in 0..LAND {
            let source = province(member, row);
            for target in &targets {
                if target.scope.building_type == NAVAL_BASE {
                    let to = target.scope.building_location;
                    let columns = i64::from(member.abs_diff((to - 1) / COLUMN));
                    memo.push(RaidPathDistanceRow {
                        dispatcher: replay_data::rows::RaidPathDispatcher::Naval,
                        source,
                        target: to,
                        distance: (columns + 1) * 400_000,
                        ok: 1,
                    });
                }
            }
        }
    }
    memo.sort_by_key(RaidPathDistanceRow::key);
    put::<tables::RaidPathDistances>(&mut frame, memo);
    put::<tables::RaidTargetStatuses>(&mut frame, targets);
    (assets, frame)
}

pub(super) struct Timed<'a> {
    pub(super) paths: paths::HourPaths<'a>,
    pub(super) reach: Tally,
    pub(super) distance: Tally,
}

#[derive(Default)]
pub(super) struct Tally(std::sync::atomic::AtomicU64);

impl Tally {
    pub(super) fn get(&self) -> Duration {
        Duration::from_nanos(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn add(&self, time: Duration) {
        self.0
            .fetch_add(time.as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

impl launch::RaidPaths for Timed<'_> {
    fn distance(
        &self,
        world: &World<'_>,
        source: &launch::PathSource,
        target: &RaidScope,
    ) -> Result<Option<i64>, String> {
        let start = Instant::now();
        let result = self.paths.distance(world, source, target);
        self.distance.add(start.elapsed());
        result
    }

    fn note(&self, source: &launch::PathSource, target: &RaidScope) {
        let start = Instant::now();
        self.paths.note(source, target);
        self.distance.add(start.elapsed());
    }

    fn recorded_rows(&self) -> Option<&[RaidPathDistanceRow]> {
        self.paths.recorded_rows()
    }

    fn path_target(&self, kind: u8, target: &RaidScope) -> Result<Option<u32>, String> {
        self.paths.path_target(kind, target)
    }

    fn army_reaches(&self, world: &World<'_>, division: u64, source_province: u32) -> Eval {
        let start = Instant::now();
        let result = self.paths.army_reaches(world, division, source_province);
        self.reach.add(start.elapsed());
        result
    }

    fn hour(&self) -> Option<&paths::HourPaths<'_>> {
        Some(&self.paths)
    }

    fn take_held(&self) -> Box<[paths::Held]> {
        self.paths.take_held()
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[test]
#[ignore = "synthetic raid layer bench; run with --release -- --ignored --nocapture"]
fn bench_raid_icons() {
    for countries in [1_500, 5_000] {
        let built = Instant::now();
        let (assets, base) = world(countries);
        let setup = built.elapsed();
        let hours: Vec<DecodedFrame> = (0..HOURS)
            .map(|hour| {
                let mut frame = base.clone();
                frame.hour = hour;
                let mut rows = frame.complete::<tables::Countries>().unwrap().to_vec();
                rows[0].command_power_raw = (50 + i64::from(hour % 3)) * 100_000;
                put::<tables::Countries>(&mut frame, rows);
                frame
            })
            .collect();
        let mut totals = Vec::new();
        let (mut reach, mut distance) = (Duration::ZERO, Duration::ZERO);
        let mut last = RaidReport::default();
        for frame in &hours {
            let timed = Timed {
                paths: paths::HourPaths::new(&assets, frame),
                reach: Tally::default(),
                distance: Tally::default(),
            };
            let start = Instant::now();
            let icons = icons(&assets, frame, 0, &timed);
            totals.push(start.elapsed());
            reach += timed.reach.get();
            distance += timed.distance.get();
            last = icons.report;
        }
        let total: Duration = totals.iter().sum();
        totals.sort();
        let per = |value: Duration| ms(value) / f64::from(HOURS);
        println!(
            "BENCH raids {countries} countries ({} targets, setup {:.0} ms): per hour mean {:.2} ms, p50 {:.2}, max {:.2}; army reach {:.2} ms, distances {:.2} ms, the rest {:.2} ms; {} icons ({} offers, {} grey unverified, {} withheld)",
            base.complete::<tables::RaidTargetStatuses>().unwrap().len(),
            ms(setup),
            per(total),
            ms(totals[totals.len() / 2]),
            ms(*totals.last().unwrap()),
            per(reach),
            per(distance),
            per(total - reach - distance),
            last.icons,
            last.offers,
            last.unverified_tint,
            last.withheld,
        );
        for (reason, count) in last.reasons.iter().take(5) {
            println!("  {reason} ({count})");
        }
        let start = Instant::now();
        for frame in &hours {
            std::hint::black_box(World::new(frame, assets.trigger_content.facts()).unwrap());
        }
        println!(
            "  of the rest, World::new {:.2} ms per hour",
            per(start.elapsed())
        );
    }
}
