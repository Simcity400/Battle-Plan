use super::*;
use crate::{
    frame::{Metrics, TableView},
    rows::{
        AttackStats, AttackTerrain, BuildingRow, CommanderAttackStats, CountryAttackInputs,
        CountryAttackModifiers, CountryCounterInputRow, CountryLinkRow, CountryRow,
        DivisionAttackInputs, DivisionAttackStats, DivisionRow, DivisionStatusRow, LexicalTokenRow,
        ModifierDefinitionRow, PacketHour, PoliticalRuntimeScalars, ProvinceRow, RaidAirScoreRow,
        RegionCombatWeather, SimulationFeatureRow, TableId, TableRows, WeatherProvinceRow,
        WeatherRegionRow, Witnessed,
    },
};
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Clone)]
struct Mock {
    controllers: BTreeMap<u32, u32>,
    same: bool,
    war: bool,
    terrain: Fact<i64>,
    commander: Fact<Option<Commander>>,
    crossing: Fact<Option<u8>>,
    snow: Fact<bool>,
    scalars: Fact<Scalars>,
    defines: Fact<Defines>,
    fort_level: Fact<i32>,
    fort_base: Fact<i64>,
    region: Fact<Option<[i64; 2]>>,
    factor: Fact<i64>,
    night_factor: Fact<i64>,
    night_modifier: Fact<i64>,
    enemy_air: Fact<bool>,
    asked: RefCell<Vec<&'static str>>,
}

impl Default for Mock {
    fn default() -> Self {
        Self {
            controllers: BTreeMap::from([(1, 10), (2, 20)]),
            same: false,
            war: true,
            terrain: Ok(0),
            commander: Ok(None),
            crossing: Ok(None),
            snow: Ok(false),
            scalars: Ok(Scalars {
                river: [Ok(0), Ok(0)],
                fort: [Ok(0), Ok(0)],
                snow: Ok(0),
                night: Ok(0),
            }),
            defines: Ok(Defines {
                river_small: Ok(-30_000),
                river_large: Ok(-60_000),
                night: Ok(-50_000),
                fort: Ok(-150_000),
            }),
            fort_level: Ok(0),
            fort_base: Err("fort base unasked".into()),
            region: Ok(None),
            factor: Err("factor unasked".into()),
            night_factor: Ok(0),
            night_modifier: Err("night modifier unasked".into()),
            enemy_air: Ok(false),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl Mock {
    fn ask<T: Clone>(&self, name: &'static str, value: &Fact<T>) -> Fact<T> {
        self.asked.borrow_mut().push(name);
        value.clone()
    }
}

impl Inputs for Mock {
    fn controller(&self, province: u32) -> Fact<u32> {
        self.controllers
            .get(&province)
            .copied()
            .ok_or_else(|| format!("province {province} unknown"))
    }
    fn same_country(&self, _: u32, _: u32) -> Fact<bool> {
        Ok(self.same)
    }
    fn at_war(&self, _: u32) -> Fact<bool> {
        Ok(self.war)
    }
    fn terrain_attack(&self, _: u32) -> Fact<i64> {
        self.ask("terrain", &self.terrain)
    }
    fn commander(&self, _: u32) -> Fact<Option<Commander>> {
        self.ask("commander", &self.commander)
    }
    fn crossing(&self, _: u32, _: u32) -> Fact<Option<u8>> {
        self.ask("crossing", &self.crossing)
    }
    fn has_snow(&self, _: u32) -> Fact<bool> {
        self.ask("snow", &self.snow)
    }
    fn scalars(&self) -> Fact<Scalars> {
        self.ask("scalars", &self.scalars)
    }
    fn defines(&self) -> Fact<Defines> {
        self.ask("defines", &self.defines)
    }
    fn fort_level(&self, _: u32) -> Fact<i32> {
        self.ask("fort level", &self.fort_level)
    }
    fn fort_base(&self, _: u32) -> Fact<i64> {
        self.ask("fort base", &self.fort_base)
    }
    fn region_weather(&self, _: u32) -> Fact<Option<[i64; 2]>> {
        self.ask("region", &self.region)
    }
    fn weather_factor(&self) -> Fact<i64> {
        self.ask("factor", &self.factor)
    }
    fn night_factor(&self, _: u32) -> Fact<i64> {
        self.ask("night factor", &self.night_factor)
    }
    fn night_modifier(&self) -> Fact<i64> {
        self.ask("night modifier", &self.night_modifier)
    }
    fn enemy_air(&self) -> Fact<bool> {
        self.ask("enemy air", &self.enemy_air)
    }
}

fn rows(mock: &Mock) -> Vec<Row> {
    let preview = super::preview(mock, 1, 2).unwrap().unwrap();
    assert!(preview.refusals.is_empty(), "{:?}", preview.refusals);
    preview.rows
}

#[test]
fn outer_gates_return_before_any_operand() {
    for mock in [
        Mock {
            controllers: BTreeMap::from([(1, 0), (2, 20)]),
            ..Default::default()
        },
        Mock {
            controllers: BTreeMap::from([(1, 10), (2, u32::MAX)]),
            ..Default::default()
        },
        Mock {
            controllers: BTreeMap::from([(1, 10), (2, 10)]),
            ..Default::default()
        },
        Mock {
            same: true,
            ..Default::default()
        },
        Mock {
            war: false,
            ..Default::default()
        },
    ] {
        assert_eq!(preview(&mock, 1, 2), Ok(None));
        assert!(mock.asked.borrow().is_empty());
    }
}

#[test]
fn every_entry_follows_the_native_order_and_arithmetic() {
    let mock = Mock {
        terrain: Ok(-15_000),
        commander: Ok(Some(Commander {
            terrain: Ok(5_000),
            river: Ok(10_000),
            fort: Ok(20_000),
            snow: Ok(2_500),
            night: Ok(1_000),
        })),
        crossing: Ok(Some(3)),
        snow: Ok(true),
        scalars: Ok(Scalars {
            river: [Ok(-5_000), Ok(7_000)],
            fort: [Ok(10_000), Ok(-400_000)],
            snow: Ok(-12_500),
            night: Ok(4_000),
        }),
        fort_level: Ok(2),
        fort_base: Ok(-150_000),
        region: Ok(Some([-20_000, -10_000])),
        factor: Ok(50_000),
        night_factor: Ok(80_001),
        night_modifier: Ok(25_000),
        enemy_air: Ok(true),
        ..Default::default()
    };
    let fort_factor = -300_000 + 20_000;
    assert_eq!(
        rows(&mock),
        [
            Row {
                kind: TERRAIN,
                value: -10_000
            },
            Row {
                kind: RIVER,
                value: -55_000
            },
            Row {
                kind: FORT,
                value: 10_000 + fort_factor
            },
            Row {
                kind: WEATHER,
                value: -10_000
            },
            Row {
                kind: WEATHER,
                value: -10_000
            },
            Row {
                kind: NIGHT,
                value: 4_000 - 50_000 + 25_000 + 1_000
            },
            Row {
                kind: ENEMY_AIR,
                value: 0
            },
        ]
    );
}

#[test]
fn zero_values_omit_their_entries_but_night_always_lists() {
    let mock = Mock {
        night_factor: Ok(100_000),
        night_modifier: Ok(50_000),
        ..Default::default()
    };
    assert_eq!(
        rows(&mock),
        [Row {
            kind: NIGHT,
            value: 0
        }]
    );
    let mock = Mock {
        night_factor: Ok(NIGHT_THRESHOLD),
        ..Default::default()
    };
    assert!(rows(&mock).is_empty());
    assert!(!mock.asked.borrow().contains(&"night modifier"));
}

#[test]
fn small_rivers_and_admission_by_either_value() {
    let mut mock = Mock {
        crossing: Ok(Some(2)),
        scalars: Ok(Scalars {
            river: [Ok(30_000), Ok(29_000)],
            fort: [Ok(0), Ok(0)],
            snow: Ok(0),
            night: Ok(0),
        }),
        ..Default::default()
    };
    assert_eq!(
        rows(&mock),
        [Row {
            kind: RIVER,
            value: 0
        }]
    );
    mock.scalars = Ok(Scalars {
        river: [Ok(30_000), Ok(30_000)],
        fort: [Ok(0), Ok(0)],
        snow: Ok(0),
        night: Ok(0),
    });
    assert!(rows(&mock).is_empty());
    for kind in [0, 1, 4] {
        mock.crossing = Ok(Some(kind));
        assert!(rows(&mock).is_empty());
    }
}

#[test]
fn a_fort_never_raises_attack() {
    let mock = Mock {
        fort_level: Ok(1),
        fort_base: Ok(-100_000),
        scalars: Ok(Scalars {
            river: [Ok(0), Ok(0)],
            fort: [Ok(500_000), Ok(100_000)],
            snow: Ok(0),
            night: Ok(0),
        }),
        ..Default::default()
    };
    assert!(rows(&mock).is_empty());
    let mock = Mock {
        fort_level: Ok(-3),
        ..Default::default()
    };
    assert!(rows(&mock).is_empty());
}

#[test]
fn a_missing_operand_refuses_only_its_entries() {
    let mock = Mock {
        terrain: Ok(-10_000),
        crossing: Ok(Some(2)),
        snow: Ok(true),
        scalars: Ok(Scalars {
            river: [Err("river unread".into()), Ok(0)],
            fort: [Ok(0), Ok(0)],
            snow: Ok(-5_000),
            night: Ok(0),
        }),
        region: Err("no region weather".into()),
        ..Default::default()
    };
    let preview = super::preview(&mock, 1, 2).unwrap().unwrap();
    assert_eq!(
        preview.rows,
        [
            Row {
                kind: TERRAIN,
                value: -10_000
            },
            Row {
                kind: WEATHER,
                value: -5_000
            },
        ]
    );
    assert_eq!(
        preview.refusals,
        [
            ("river", "river unread".to_owned()),
            ("region weather", "no region weather".to_owned())
        ]
    );
    let mock = Mock {
        commander: Err("group unread".into()),
        ..Default::default()
    };
    let preview = super::preview(&mock, 1, 2).unwrap().unwrap();
    assert_eq!(preview.refusals, [("terrain", "group unread".to_owned())]);
}

#[test]
fn a_refused_define_refuses_only_the_entries_that_read_it() {
    let defines = |night: Fact<i64>| {
        Ok(Defines {
            river_small: Ok(-30_000),
            river_large: Err("large river refused".into()),
            night,
            fort: Err("fort refused".into()),
        })
    };
    let mock = Mock {
        crossing: Ok(Some(2)),
        scalars: Ok(Scalars {
            river: [Ok(-5_000), Ok(0)],
            fort: [Ok(0), Ok(0)],
            snow: Ok(0),
            night: Ok(4_000),
        }),
        defines: defines(Ok(-50_000)),
        night_factor: Ok(90_000),
        night_modifier: Ok(0),
        ..Default::default()
    };
    assert_eq!(
        rows(&mock),
        [
            Row {
                kind: RIVER,
                value: -35_000
            },
            Row {
                kind: NIGHT,
                value: -46_000
            },
        ]
    );
    let mock = Mock {
        crossing: Ok(Some(3)),
        defines: defines(Err("night refused".into())),
        ..mock
    };
    let preview = super::preview(&mock, 1, 2).unwrap().unwrap();
    assert!(preview.rows.is_empty(), "{:?}", preview.rows);
    assert_eq!(
        preview.refusals,
        [
            ("river", "large river refused".to_owned()),
            ("night", "night refused".to_owned())
        ]
    );
}

#[test]
fn region_weather_skips_the_factor_when_both_values_are_zero() {
    let mock = Mock {
        region: Ok(Some([0, 0])),
        ..Default::default()
    };
    assert!(rows(&mock).is_empty());
    assert!(!mock.asked.borrow().contains(&"factor"));
    let mock = Mock {
        region: Ok(Some([-20_000, 5_000])),
        factor: Ok(0),
        ..Default::default()
    };
    assert!(rows(&mock).is_empty());
}

#[test]
fn snow_and_fort_helpers_wrap_and_clamp_as_the_game() {
    assert!(has_snow(1, 0, 100));
    assert!(!has_snow(5, -5, 100));
    assert!(!has_snow(500, 0, 0));
    assert!(!has_snow(i64::MAX, 1, 1));
    assert_eq!(fort_base(None, 7, -150_000), -150_000);
    assert_eq!(fort_base(Some(-50_000), 0, -150_000), -75_000);
    assert_eq!(fort([0, 0], 0, 0, -1), None);
}

const DIVISION: u64 = (7 << 32) | 1;
const GROUP: u64 = (3 << 32) | 53;
const LEADER: u64 = (9 << 32) | 45;
const FORT_TOKEN: u32 = 900;

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 500,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture has no sample".into(),
            }
            .into()
        }),
    };
    synthetic_game::enter_fixed_modifiers();
    let mut keys = engine_executable::modifier_keys::all()
        .unwrap()
        .collect::<Vec<_>>();
    keys.sort_unstable_by_key(|key| key.index);
    put(
        &mut frame,
        TableRows::ModifierDefinitions(
            keys.iter()
                .map(|key| ModifierDefinitionRow {
                    index: key.index,
                    token: key.index + 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::LexicalTokens(
            keys.iter()
                .map(|key| LexicalTokenRow {
                    token: key.index + 1,
                    literal: key.name.as_bytes().to_vec(),
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            runtime_scalars: rows::Recorded::Value(PoliticalRuntimeScalars {
                sample_hour: PacketHour::Host,
                river_small: Some(-30_000),
                river_large: Some(-60_000),
                night: Some(-50_000),
                fort: Some(-150_000),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            roster_owner: 11,
            logical_owner: 11,
            expeditionary_owner: 0,
            province: 1,
            orders_group: GROUP,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 22,
                canonical_slot: 2,
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::Countries(vec![
            CountryRow {
                id: 11,
                canonical_slot: 1,
                ..Default::default()
            },
            CountryRow {
                id: 22,
                canonical_slot: 2,
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::Provinces(vec![
            ProvinceRow {
                id: 1,
                controller: 11,
                fort_bunker: rows::Recorded::Value(None),
                ..Default::default()
            },
            ProvinceRow {
                id: 2,
                state: 4,
                controller: 22,
                fort_positive: 1,
                fort_bunker: rows::Recorded::Value(Some(-20_000)),
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::ProvinceBuildings(vec![
            BuildingRow {
                location: 2,
                building_type: FORT_TOKEN,
                healthy_level: 2,
                active: true,
                ..Default::default()
            },
            BuildingRow {
                location: 2,
                building_type: FORT_TOKEN + 1,
                healthy_level: 5,
                active: false,
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 11,
                attack_modifiers: rows::Recorded::Value(Some(CountryAttackModifiers {
                    land_night_attack: Some(25_000),
                    land_bunker_effectiveness_factor: None,
                })),
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 22,
                attack_modifiers: rows::Recorded::Value(Some(CountryAttackModifiers {
                    land_night_attack: None,
                    land_bunker_effectiveness_factor: Some(-50_000),
                })),
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: DIVISION,
            attack_stats: rows::Recorded::Value(Some(DivisionAttackStats {
                terrain: vec![0, -10_000, -40_000, -1],
                river: [-5_000, 7_000],
                fort: [10_000, -400_000],
                snow: -12_500,
                night: 4_000,
            })),
            weather_factor: rows::Recorded::Value(Some(50_000)),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::OrdersGroupCounterInputs(vec![rows::OrdersGroupCounterInputRow {
            group: GROUP,
            commander_attack: rows::Recorded::Value(Some(Some(CommanderAttackStats {
                leader: LEADER,
                terrain: vec![1, 2, 5_000],
                fort: 20_000,
                river: 10_000,
                snow: 2_500,
                night: 1_000,
            }))),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::WeatherProvinces(vec![WeatherProvinceRow {
            id: 2,
            snow: 40_000,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::WeatherRegions(vec![
            WeatherRegionRow {
                id: 5,
                max_snow: 100_000,
                combat: rows::Recorded::Value(Some(RegionCombatWeather::default())),
                ..Default::default()
            },
            WeatherRegionRow {
                id: 6,
                max_snow: 100_000,
                combat: rows::Recorded::Value(Some(RegionCombatWeather {
                    army_attack_factor: Some(-20_000),
                    breakthrough_factor: Some(-10_000),
                    ..Default::default()
                })),
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::RaidAirScores(vec![RaidAirScoreRow {
            country: 11,
            region: 5,
            friendly: 0,
            enemy: 30_000,
            impact: Default::default(),
            friendly_missions: 0,
            enemy_missions: 0,
            air_spotting_scale: Some(0),
            menu: replay_data::rows::Recorded::Value(None),
        }]),
    );
    frame
}

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}

struct Content {
    index: i32,
    attack: i64,
    night: i64,
}

impl Context for Content {
    fn terrain(&self, province: u32) -> Fact<Terrain> {
        if province != 2 {
            return Err("only the target has terrain".into());
        }
        Ok(Terrain {
            index: self.index,
            attack: self.attack,
        })
    }
    fn crossing(&self, from: u32, to: u32) -> Fact<Option<u8>> {
        Ok(((from, to) == (1, 2)).then_some(3))
    }
    fn region(&self, province: u32) -> Fact<Option<u32>> {
        Ok(Some(if province == 1 { 5 } else { 6 }))
    }
    fn land_fort(&self, building_type: u32) -> Fact<i32> {
        match building_type {
            FORT_TOKEN => Ok(3),
            _ => Ok(1),
        }
    }
    fn night_factor(&self, _: u32) -> Fact<i64> {
        Ok(self.night)
    }
    fn at_war(&self, receiver: u32, target: u32) -> Fact<bool> {
        Ok((receiver, target) == (11, 22))
    }
}

#[test]
fn recorded_inputs_derive_the_list_with_the_recorded_content() {
    let frame = frame();
    let content = Content {
        index: 2,
        attack: 3_000,
        night: 90_000,
    };
    let inputs = FrameInputs::new(&frame, &content, DIVISION);
    let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
    assert!(preview.refusals.is_empty(), "{:?}", preview.refusals);
    let fort_factor = -270_000 + 20_000;
    assert_eq!(
        preview.rows,
        [
            Row {
                kind: TERRAIN,
                value: -2_000
            },
            Row {
                kind: RIVER,
                value: -55_000
            },
            Row {
                kind: FORT,
                value: 10_000 + fort_factor
            },
            Row {
                kind: WEATHER,
                value: -10_000
            },
            Row {
                kind: WEATHER,
                value: -10_000
            },
            Row {
                kind: NIGHT,
                value: -20_000
            },
            Row {
                kind: ENEMY_AIR,
                value: 0
            },
        ]
    );
}

#[test]
fn recorded_content_variations_change_only_what_they_reach() {
    let frame = frame();
    let content = Content {
        index: 5,
        attack: 0,
        night: 0,
    };
    let inputs = FrameInputs::new(&frame, &content, DIVISION);
    let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
    assert_eq!(preview.refusals.len(), 1, "{:?}", preview.refusals);
    assert_eq!(preview.refusals[0].0, "terrain");
    let content = Content {
        index: 4,
        attack: 40_000,
        night: 0,
    };
    let inputs = FrameInputs::new(&frame, &content, DIVISION);
    let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
    assert_eq!(
        preview.rows[0],
        Row {
            kind: TERRAIN,
            value: -1 + 40_000 + 1
        }
    );
}

#[test]
fn a_recorded_define_refused_alone_refuses_only_its_entry() {
    let content = Content {
        index: 2,
        attack: 3_000,
        night: 90_000,
    };
    let preview = |edit: fn(&mut PoliticalRuntimeScalars)| {
        let mut frame = frame();
        let mut scalars = PoliticalRuntimeScalars {
            sample_hour: PacketHour::Host,
            river_small: Some(-30_000),
            river_large: Some(-60_000),
            night: Some(-50_000),
            fort: Some(-150_000),
            ..Default::default()
        };
        edit(&mut scalars);
        put(
            &mut frame,
            TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                runtime_scalars: rows::Recorded::Value(scalars),
                ..Default::default()
            }]),
        );
        let inputs = FrameInputs::new(&frame, &content, DIVISION);
        let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
        let kinds = preview.rows.iter().map(|row| row.kind).collect::<Vec<_>>();
        (kinds, preview.refusals)
    };
    let refused = |name: &'static str, define: &str| {
        vec![(name, format!("{define} was refused when recorded"))]
    };
    assert_eq!(
        preview(|scalars| scalars.fort = None),
        (
            vec![TERRAIN, RIVER, WEATHER, WEATHER, NIGHT, ENEMY_AIR],
            refused("fort", "BASE_FORT_PENALTY")
        )
    );
    assert_eq!(
        preview(|scalars| scalars.river_large = None),
        (
            vec![TERRAIN, FORT, WEATHER, WEATHER, NIGHT, ENEMY_AIR],
            refused("river", "RIVER_CROSSING_PENALTY_LARGE")
        )
    );
    assert_eq!(
        preview(|scalars| scalars.night = None),
        (
            vec![TERRAIN, RIVER, FORT, WEATHER, WEATHER, ENEMY_AIR],
            refused("night", "BASE_NIGHT_ATTACK_PENALTY")
        )
    );
    assert_eq!(
        preview(|scalars| scalars.river_small = None),
        (
            vec![TERRAIN, RIVER, FORT, WEATHER, WEATHER, NIGHT, ENEMY_AIR],
            Vec::new()
        )
    );
}

#[test]
fn format_101_operands_answer_for_their_recordings() {
    let mut frame = frame();
    let attack = |terrain: i32, attack: i64| AttackTerrain {
        terrain,
        attack: Some(attack),
    };
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: DIVISION,
            attack: rows::Recorded::Value(Witnessed(Some(DivisionAttackInputs {
                sample_hour: PacketHour::Stamped(500),
                template: Some(0),
                stats: AttackStats {
                    river: [Some(-5_000), Some(7_000)],
                    fort: [Some(10_000), Some(-400_000)],
                    snow: Some(-12_500),
                    night: Some(4_000),
                },
                terrain: Some(vec![attack(1, -1), attack(2, -7_000)]),
                acclimatization: Some(50_000),
            }))),
            attack_stats: rows::Recorded::NotRecorded,
            weather_factor: rows::Recorded::NotRecorded,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::OrdersGroupCounterInputs(vec![rows::OrdersGroupCounterInputRow {
            group: GROUP,
            commander_attack: rows::Recorded::NotRecorded,
            attack: rows::Recorded::Value(Witnessed(Some(rows::OrdersGroupAttackInputs {
                sample_hour: PacketHour::Stamped(500),
                commander: Some(None),
            }))),
            ..Default::default()
        }]),
    );
    let mut countries = vec![
        CountryCounterInputRow {
            country: 11,
            attack_modifiers: rows::Recorded::NotRecorded,
            attack: rows::Recorded::Value(Witnessed(Some(CountryAttackInputs {
                sample_hour: PacketHour::Stamped(500),
                land_night_attack: Some(25_000),
                land_bunker_effectiveness_factor: None,
            }))),
            ..Default::default()
        },
        CountryCounterInputRow {
            country: 22,
            attack_modifiers: rows::Recorded::NotRecorded,
            attack: rows::Recorded::Value(Witnessed(Some(CountryAttackInputs {
                sample_hour: PacketHour::Stamped(500),
                land_night_attack: None,
                land_bunker_effectiveness_factor: Some(-50_000),
            }))),
            ..Default::default()
        },
    ];
    put(
        &mut frame,
        TableRows::CountryCounterInputs(countries.clone()),
    );
    put(
        &mut frame,
        TableRows::Provinces(vec![
            ProvinceRow {
                id: 1,
                controller: 11,
                fort_bunker: rows::Recorded::NotRecorded,
                ..Default::default()
            },
            ProvinceRow {
                id: 2,
                controller: 22,
                fort_bunker: rows::Recorded::NotRecorded,
                attack: rows::Recorded::Value(Witnessed(Some(rows::ProvinceAttackInputs {
                    sample_hour: PacketHour::Stamped(500),
                    fort_sum: Some(6),
                    land_bunker_effectiveness_factor: Some(-20_000),
                }))),
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::WeatherRegions(vec![WeatherRegionRow {
            id: 6,
            max_snow: 100_000,
            combat: rows::Recorded::NotRecorded,
            attack: rows::Recorded::Value(Witnessed(Some(rows::WeatherRegionAttackInputs {
                sample_hour: PacketHour::Stamped(500),
                attack: Some(-20_000),
                breakthrough: Some(-10_000),
            }))),
            ..Default::default()
        }]),
    );
    let content = Content {
        index: 2,
        attack: 999_999,
        night: 90_000,
    };
    let inputs = FrameInputs::new(&frame, &content, DIVISION);
    let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
    assert!(preview.refusals.is_empty(), "{:?}", preview.refusals);
    let fort_factor = -270_000;
    assert_eq!(
        preview.rows,
        [
            Row {
                kind: TERRAIN,
                value: -7_000
            },
            Row {
                kind: RIVER,
                value: -65_000
            },
            Row {
                kind: FORT,
                value: 10_000 + fort_factor
            },
            Row {
                kind: WEATHER,
                value: -12_500
            },
            Row {
                kind: WEATHER,
                value: -10_000
            },
            Row {
                kind: NIGHT,
                value: -21_000
            },
            Row {
                kind: ENEMY_AIR,
                value: 0
            },
        ]
    );
    countries[0].attack = rows::Recorded::Value(Witnessed(Some(CountryAttackInputs {
        sample_hour: PacketHour::Stamped(499),
        land_night_attack: Some(25_000),
        land_bunker_effectiveness_factor: None,
    })));
    put(&mut frame, TableRows::CountryCounterInputs(countries));
    let inputs = FrameInputs::new(&frame, &content, DIVISION);
    let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
    assert_eq!(preview.refusals.len(), 1);
    assert_eq!(preview.refusals[0].0, "night");
}

#[test]
fn absent_tables_refuse_by_name_and_reserved_controllers_are_not_recorded() {
    let mut frame = frame();
    *frame.tables[TableId::RaidAirScores.index()] = TableView::Unavailable {
        reason: "raid scores unread".into(),
    };
    let content = Content {
        index: 2,
        attack: 0,
        night: 0,
    };
    let inputs = FrameInputs::new(&frame, &content, DIVISION);
    let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
    assert_eq!(preview.refusals.len(), 1);
    assert_eq!(preview.refusals[0].0, "enemy air");
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 22,
                canonical_slot: 0,
            },
        ]),
    );
    let inputs = FrameInputs::new(&frame, &content, DIVISION);
    let preview = super::preview(&inputs, 1, 2).unwrap().unwrap();
    assert!(
        preview
            .refusals
            .iter()
            .any(|(name, reason)| *name == "fort" && reason.contains("reserved"))
    );
}
