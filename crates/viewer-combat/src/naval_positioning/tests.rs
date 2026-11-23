use super::*;
use replay_data::{rows::NavalCombatPlane, rows::NavalCombatSummaryModifier};
use std::time::Instant;

fn displayed(input: &Inputs<'_>, defines: &Defines) -> Option<i64> {
    displayed_terms(input, defines).map(|terms| terms.value)
}

fn defines() -> Defines {
    Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![ONE],
        base_carrier_sortie_efficiency: ONE,
        selected_sortie_initial_time: 24,
        piercing: Ok(Default::default()),
        positioning_impacts: [0; 3],
    }
}

#[test]
fn initial_defender_modifiers_end_at_duration_boundary_and_stored_value_is_not_displayed() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 40_000;
    definitions.summary_integer[1] = 12;
    let mut side = NavalCombatSideDetails {
        positioning: 91_000,
        ..Default::default()
    };
    side.summary.countries.push(42);
    side.summary.modifiers[0][0] = NavalCombatSummaryModifier {
        raw: 5_000,
        present: 1,
    };
    side.summary.modifiers[3][2] = NavalCombatSummaryModifier {
        raw: 7_000,
        present: 1,
    };
    side.summary.escort_bonus = 2_000;
    let opponent = NavalCombatSideDetails::default();
    let mut input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: true,
        duration: 11,
        surface_detection: [0; 2],
        active_ship_slots: [0; 2],
        air: [Air::default(); 2],
    };
    assert_eq!(displayed(&input, &definitions), Some(54_000));
    input.duration = 12;
    assert_eq!(displayed(&input, &definitions), Some(49_000));
    input.defender = false;
    input.duration = 11;
    assert_eq!(displayed(&input, &definitions), Some(49_000));
}

#[test]
fn initial_positioning_reads_the_opposing_admiral_and_ordinary_the_own() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 40_000;
    definitions.summary_integer[1] = 12;
    let mut side = NavalCombatSideDetails::default();
    side.summary.countries.push(42);
    let cell = |raw| NavalCombatSummaryModifier { raw, present: 1 };
    side.summary.modifiers[3][0] = cell(20_000);
    side.summary.modifiers[4][0] = cell(3_000);
    side.summary.modifiers[3][2] = cell(7_000);
    side.summary.modifiers[4][2] = cell(11_000);
    let opponent = NavalCombatSideDetails::default();
    let mut input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: true,
        duration: 11,
        surface_detection: [0; 2],
        active_ship_slots: [0; 2],
        air: [Air::default(); 2],
    };
    assert_eq!(displayed(&input, &definitions), Some(50_000));
    input.duration = 12;
    assert_eq!(displayed(&input, &definitions), Some(47_000));
    input.duration = 11;
    input.defender = false;
    assert_eq!(displayed(&input, &definitions), Some(47_000));
}

#[test]
fn empty_primary_country_list_skips_modifiers_and_escort() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 30_000;
    definitions.summary_integer[1] = 12;
    let mut side = NavalCombatSideDetails::default();
    side.summary.escort_bonus = 20_000;
    side.summary.modifiers[0][2] = NavalCombatSummaryModifier {
        raw: 10_000,
        present: 1,
    };
    let opponent = NavalCombatSideDetails::default();
    let input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: true,
        duration: 0,
        surface_detection: [0; 2],
        active_ship_slots: [0; 2],
        air: [Air::default(); 2],
    };
    assert_eq!(displayed(&input, &definitions), Some(30_000));
}

#[test]
fn modifiers_gate_feature_and_clamp_before_sum() {
    let mut side = NavalCombatSideDetails::default();
    let summary = &mut side.summary;
    summary.modifier_metadata[2] = [0x10, 4];
    summary.modifiers[0][2] = NavalCombatSummaryModifier {
        raw: 120_000,
        present: 1,
    };
    assert_eq!(modifier(summary, 0, 2), Some(0));
    summary.eligibility_callback = 1;
    summary.feature_flags = 4;
    assert_eq!(modifier(summary, 0, 2), Some(ONE));
    summary.eligibility_callback = 2;
    assert_eq!(modifier(summary, 0, 2), None);
}

#[test]
fn ratio_penalties_respect_count_gate_opposing_admiral_fill_and_positive_only() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 90_000;
    definitions.summary_fixed[3] = 10_000;
    definitions.summary_fixed[4] = 30_000;
    definitions.summary_fixed[5] = 20_000;
    definitions.summary_fixed[6] = 30_000;
    definitions.summary_integer[0] = 4;
    let mut side = NavalCombatSideDetails::default();
    side.summary.modifiers[4][4] = NavalCombatSummaryModifier {
        raw: 50_000,
        present: 1,
    };
    let opponent = NavalCombatSideDetails {
        commander: 1,
        ..Default::default()
    };
    let mut input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: false,
        duration: 0,
        surface_detection: [0; 2],
        active_ship_slots: [4, 2],
        air: [
            Air {
                weighted_planes: 50_000,
                average_base_fill: 0,
            },
            Air {
                weighted_planes: 100_000,
                average_base_fill: 50_000,
            },
        ],
    };
    assert_eq!(displayed(&input, &definitions), Some(70_000));
    input.active_ship_slots = [3, 2];
    assert_eq!(displayed(&input, &definitions), Some(85_000));
    input.active_ship_slots = [4, 8];
    assert_eq!(displayed(&input, &definitions), Some(85_000));
}

#[test]
fn surface_detection_adds_only_positive_capped_advantage() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 50_000;
    definitions.summary_fixed[1] = ONE;
    definitions.summary_fixed[2] = 2_000;
    let side = NavalCombatSideDetails::default();
    let opponent = NavalCombatSideDetails::default();
    let mut input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: false,
        duration: 0,
        surface_detection: [120_000, 20_000],
        active_ship_slots: [0; 2],
        air: [Air::default(); 2],
    };
    assert_eq!(displayed(&input, &definitions), Some(51_000));
    input.surface_detection = [1_000_000, 0];
    assert_eq!(displayed(&input, &definitions), Some(52_000));
    input.surface_detection = [0, 1_000_000];
    assert_eq!(displayed(&input, &definitions), Some(50_000));
}

#[test]
fn native_signed_ratio_guard_excludes_only_extreme_numerators() {
    assert!(!ratio_numerator_valid(i64::MIN));
    assert!(!ratio_numerator_valid(i64::MIN + 1));
    assert!(ratio_numerator_valid(i64::MIN + 2));
    assert!(ratio_numerator_valid(-1));
    assert!(ratio_numerator_valid(0));
    assert!(ratio_numerator_valid(i64::MAX - 1));
    assert!(!ratio_numerator_valid(i64::MAX));

    let mut definitions = defines();
    definitions.summary_integer[0] = 0;
    definitions.summary_fixed[3] = ONE;
    definitions.summary_fixed[4] = ONE;
    let side = NavalCombatSideDetails::default();
    let opponent = NavalCombatSideDetails::default();
    let input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: false,
        duration: 0,
        surface_detection: [0; 2],
        active_ship_slots: [i32::MAX, 0],
        air: [Air::default(); 2],
    };
    assert_eq!(ship_ratio_penalty(&input, &definitions), Some(0));
}

#[test]
fn ship_slots_include_unresolved_members_but_skip_retired_or_inactive() {
    let row = |slot, source, state| NavalCombatShipRow {
        side: 0,
        battle_slot: slot,
        member_source: source,
        group_index: 0,
        state,
        live_ship: 0,
        live_convoy: 0,
        ..Default::default()
    };
    let a = row(0, 0, 0);
    let b = row(1, 0, 0x34);
    let c = row(2, 1, 0);
    let d = row(3, 0, 0);
    let rows = [&a, &b, &c, &d];
    assert_eq!(active_ship_slots(&rows, 0), 2);
    assert_eq!(active_ship_slots(&rows, 1), 0);
}

#[test]
fn missing_modifier_witness_and_recent_join_use_final_clamp() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 80_000;
    let mut side = NavalCombatSideDetails::default();
    side.summary.countries.push(42);
    side.summary.recent_join_penalty = 90_000;
    side.summary.modifiers[0][2] = NavalCombatSummaryModifier {
        raw: -10_000,
        present: 1,
    };
    let opponent = NavalCombatSideDetails::default();
    let input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: false,
        duration: 0,
        surface_detection: [0; 2],
        active_ship_slots: [0; 2],
        air: [Air::default(); 2],
    };
    assert_eq!(displayed(&input, &definitions), Some(0));
    side.summary.recent_join_penalty = -90_000;
    let input = Inputs {
        side: &side,
        opponent: &opponent,
        defender: false,
        duration: 0,
        surface_detection: [0; 2],
        active_ship_slots: [0; 2],
        air: [Air::default(); 2],
    };
    assert_eq!(displayed(&input, &definitions), Some(70_000));
}

#[test]
fn adapter_preserves_provable_zero_paths_without_dense_or_model_tables() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 40_000;
    let detail = NavalCombatDetailsRow {
        combat: 7,
        ..Default::default()
    };
    let content = naval_air_groups::Content::default();
    assert_eq!(
        positioning_values(
            &detail,
            Some(&[]),
            None,
            None,
            None,
            None,
            &content,
            Some(&definitions),
        ),
        [Some(40_000); 2]
    );
    assert_eq!(
        positioning_values(
            &detail,
            None,
            None,
            None,
            None,
            None,
            &content,
            Some(&definitions),
        ),
        [None; 2]
    );
    assert_eq!(
        positioning_values(&detail, Some(&[]), None, None, None, None, &content, None,),
        [None; 2]
    );
}

#[test]
fn adapter_requires_dense_witness_only_for_a_side_that_uses_its_air() {
    let mut definitions = defines();
    definitions.summary_fixed[0] = 40_000;
    let mut detail = NavalCombatDetailsRow {
        combat: 7,
        ..Default::default()
    };
    detail.sides[0].planes.push(NavalCombatPlane {
        source: 0,
        group_index: 0,
        air_base: 9,
        current_planes: 10,
        efficiency: replay_data::rows::NavalCombatAirEfficiency {
            source: 2,
            ..Default::default()
        },
        ..Default::default()
    });
    let content = naval_air_groups::Content::default();
    assert_eq!(
        positioning_values(
            &detail,
            Some(&[]),
            None,
            None,
            None,
            None,
            &content,
            Some(&definitions),
        ),
        [Some(40_000), None]
    );
    let dense_rows = [replay_data::rows::NavalCombatAirBaseWingHandlesRow {
        base: 9,
        wings: Vec::new(),
    }];
    assert_eq!(
        positioning_values(
            &detail,
            Some(&[]),
            None,
            None,
            None,
            DenseBases::new(&dense_rows, &[]),
            &content,
            Some(&definitions),
        ),
        [Some(40_000); 2]
    );
}

#[test]
fn relation_scopes_preserve_zero_sortie_carrier_air_defender_positioning_and_aa() {
    content_reader_script::test_game::enter();
    use replay_data::{
        rows::AirWingRow, rows::NavalCombatAirBaseWingHandlesRow, rows::NavalCombatAirEfficiency,
        rows::NavalCombatAirEfficiencyModifier, rows::NavalShipModifierSource,
        rows::ShipCounterInputs,
    };
    let root = std::env::temp_dir().join(format!(
        "hoi4_naval_zero_sortie_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("common/modifiers")).unwrap();
    std::fs::write(
        root.join("common/modifiers/fixture.txt"),
        concat!(
            "screening_bonus={sortie_efficiency=0.1}\n",
            "capital_screening_bonus={fighter_sortie_efficiency=0.1}\n",
            "relation={valid_relation_trigger={FROM={NOT={has_war_with=ROOT}}}}\n",
        ),
    )
    .unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let mut warnings = Vec::new();
    let content = naval_air_groups::Content::load(
        &layers,
        &mut warnings,
        content_reader_key_names::equipment::Database::default(),
    );
    std::fs::remove_dir_all(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");

    let members: Vec<_> = (0..5)
        .map(|index| NavalCombatShipRow {
            combat: 7,
            side: u8::from(index == 4),
            battle_slot: index,
            ship: 100 + u64::from(index),
            state: 2,
            live_ship: 1,
            formation_flags: if index < 4 { 1 << 22 } else { 1 << 8 },
            weapon_context: replay_data::rows::NavalShipWeaponContext {
                sources: vec![NavalShipModifierSource {
                    scope: 26,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    let models: Vec<_> = members
        .iter()
        .map(|member| ShipModelRow {
            ship: member.ship,
            counter: Some(ShipCounterInputs {
                maximum_organisation: ONE,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    let live_ships: Vec<_> = members
        .iter()
        .map(|member| ShipRow {
            id: member.ship,
            organisation: 66_666,
            ..Default::default()
        })
        .collect();
    let bases: Vec<_> = (0..4)
        .map(|index| AirBaseRow {
            id: 200 + index,
            host: 100 + index,
            ..Default::default()
        })
        .collect();
    let wings: Vec<_> = (0..8)
        .map(|index| AirWingRow {
            id: 300 + index,
            planes: 30,
            ..Default::default()
        })
        .collect();
    let dense: Vec<_> = (0..4)
        .map(|index| NavalCombatAirBaseWingHandlesRow {
            base: 200 + index,
            wings: vec![300 + index, 304 + index],
        })
        .collect();
    let mut detail = NavalCombatDetailsRow {
        combat: 7,
        duration: 24,
        ..Default::default()
    };
    detail.sides[0].capital_screening = ONE;
    detail.sides[0].carrier_screening = ONE;
    for index in 0_u32..8 {
        let mut efficiency = NavalCombatAirEfficiency {
            source: 3,
            container_present: 1,
            base_crowding: ONE,
            night: ONE,
            ..Default::default()
        };
        efficiency.modifiers[0] = NavalCombatAirEfficiencyModifier {
            raw: -ONE,
            present: 1,
        };
        detail.sides[0].planes.push(NavalCombatPlane {
            source: 0,
            group_index: 0,
            air_base: 200 + u64::from(index % 4),
            initial_planes: 30,
            current_planes: 30,
            carrier_member_present: 1,
            carrier_slot: (index % 4) as i32,
            efficiency,
            ..Default::default()
        });
    }
    let mut definitions = defines();
    definitions.summary_fixed[0] = ONE;
    definitions.summary_fixed[12] = 100 * ONE;
    let positioning = positioning_values(
        &detail,
        Some(&members),
        Keyed::new(&models, |model| model.ship),
        Some(&bases),
        Some(&live_ships),
        DenseBases::new(&dense, &wings),
        &content,
        Some(&definitions),
    );
    assert_eq!(positioning, [Some(ONE); 2]);
    assert_eq!(
        super::super::naval_anti_air::calculate(
            7,
            1,
            positioning[1].unwrap(),
            &members,
            50_000,
            |_| Some(0),
        ),
        Ok(0),
    );
    let borrowed: Vec<_> = members.iter().collect();
    let joins = CarrierJoins::new(
        Some(&borrowed),
        Some(&bases),
        Some(&live_ships),
        Keyed::new(&models, |model| model.ship),
    );
    for plane in &detail.sides[0].planes {
        assert_eq!(
            naval_air_groups::display_efficiency_parts(
                plane,
                &detail.sides[0],
                0,
                detail.duration,
                &joins,
                &content,
                Some(&definitions),
            ),
            Some((0, 0, 66_666)),
        );
    }
}

const SHIPS: u32 = 40_000;
const WINGS: u32 = 100_000;

struct World {
    members: Vec<NavalCombatShipRow>,
    bases: Vec<AirBaseRow>,
    live_ships: Vec<ShipRow>,
    models: Vec<ShipModelRow>,
    dense: Vec<replay_data::rows::NavalCombatAirBaseWingHandlesRow>,
    wings: Vec<replay_data::rows::AirWingRow>,
    detail: NavalCombatDetailsRow,
    definitions: Defines,
}

fn world(carriers: u32) -> World {
    use replay_data::{
        rows::AirWingRow, rows::NavalCombatAirBaseWingHandlesRow, rows::NavalCombatAirEfficiency,
        rows::NavalShipModifierSource, rows::ShipCounterInputs,
    };
    let ship_id = |ship: u32| 1_000 + u64::from(ship);
    let base_id = |ship: u32| 100_000 + u64::from(ship);
    let wing_id = |wing: u32| 1_000_000 + u64::from(wing);
    let side_of = |ship: u32| (ship % 2) as u8;
    let mut members = Vec::new();
    for (combat, ships) in [(7, 0..carriers), (8, 0..carriers), (9, carriers..SHIPS)] {
        for side in [0, 1] {
            for ship in ships.clone().filter(|&ship| side_of(ship) == side) {
                let mut row = NavalCombatShipRow {
                    combat,
                    side,
                    battle_slot: ship / 2,
                    ship: ship_id(ship),
                    live_ship: 1,
                    state: 2,
                    ..Default::default()
                };
                row.weapon_context.sources.push(NavalShipModifierSource {
                    scope: 26,
                    ..Default::default()
                });
                members.push(row);
            }
        }
    }
    let bases = (0..SHIPS)
        .map(|ship| AirBaseRow {
            id: base_id(ship),
            host: ship_id(ship),
            ..Default::default()
        })
        .collect();
    let live_ships = (0..SHIPS)
        .map(|ship| ShipRow {
            id: ship_id(ship),
            organisation: 500_000,
            ..Default::default()
        })
        .collect();
    let models = (0..SHIPS)
        .map(|ship| ShipModelRow {
            ship: ship_id(ship),
            counter: Some(ShipCounterInputs {
                maximum_organisation: 1_000_000,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    let mut dense: Vec<_> = (0..SHIPS)
        .map(|ship| NavalCombatAirBaseWingHandlesRow {
            base: base_id(ship),
            wings: Vec::new(),
        })
        .collect();
    for wing in 0..WINGS {
        let base = if wing < carriers {
            wing
        } else {
            carriers + (wing - carriers) % (SHIPS - carriers)
        };
        dense[base as usize].wings.push(wing_id(wing));
    }
    let wings = (0..WINGS)
        .map(|wing| AirWingRow {
            id: wing_id(wing),
            planes: 10,
            ..Default::default()
        })
        .collect();
    let mut detail = NavalCombatDetailsRow {
        combat: 7,
        duration: 24,
        ..Default::default()
    };
    for ship in 0..carriers {
        let side = side_of(ship);
        detail.sides[usize::from(side)]
            .planes
            .push(NavalCombatPlane {
                group_index: 0,
                air_base: base_id(ship),
                current_planes: 10 * (1 + i32::from(side)),
                carrier_slot: (ship / 2) as i32,
                carrier_member_present: 1,
                efficiency: NavalCombatAirEfficiency {
                    source: 3,
                    container_present: 1,
                    base_crowding: ONE,
                    ..Default::default()
                },
                ..Default::default()
            });
    }
    let mut definitions = defines();
    definitions.summary_fixed[0] = 40_000;
    definitions.summary_fixed[5] = ONE;
    definitions.summary_fixed[6] = ONE;
    definitions.summary_fixed[12] = 20 * ONE;
    World {
        members,
        bases,
        live_ships,
        models,
        dense,
        wings,
        detail,
        definitions,
    }
}

#[test]
fn carrier_air_joins_are_indexed_at_country_scale() {
    use super::super::{naval_anti_air, naval_screening};
    for carriers in [1_500_u32, 5_000] {
        let w = world(carriers);
        let content = naval_air_groups::Content::default();
        let refresh = |models: &[ShipModelRow], wings: &[replay_data::rows::AirWingRow]| {
            let models = Keyed::new(models, |model| model.ship);
            let values = positioning_values(
                &w.detail,
                Some(&w.members),
                models,
                Some(&w.bases),
                Some(&w.live_ships),
                DenseBases::new(&w.dense, wings),
                &content,
                Some(&w.definitions),
            );
            let models = models.expect("ordered ship models");
            let others = std::array::from_fn::<_, 2, _>(|side| {
                let screening = naval_screening::calculate(
                    7,
                    side as u8,
                    &w.detail.sides[side],
                    &w.members,
                    &models,
                    &w.definitions,
                );
                let anti_air = values[side].map(|positioning| {
                    naval_anti_air::calculate(
                        7,
                        side as u8,
                        positioning,
                        &w.members,
                        w.definitions.summary_fixed[13],
                        |ship| {
                            models
                                .get(ship)
                                .and_then(|row| row.counter.as_ref())
                                .map(|counter| counter.anti_air_base)
                        },
                    )
                });
                (screening.is_ok(), anti_air.is_some_and(|aa| aa.is_ok()))
            });
            (values, others)
        };
        let started = Instant::now();
        let (values, others) = refresh(&w.models, &w.wings);
        let elapsed = started.elapsed();
        assert_eq!(values, [Some(27_500), Some(40_000)]);
        assert_eq!(others, [(true, true); 2]);
        eprintln!(
            "naval window refresh, {carriers} carriers in combat, {SHIPS} ships, {WINGS} wings: {elapsed:?} (test build)"
        );

        let mut roster = w.wings.clone();
        let outside = roster.pop().expect("wings");
        assert!(
            w.dense[carriers as usize..]
                .iter()
                .any(|row| row.wings.contains(&outside.id))
        );
        assert_eq!(refresh(&w.models, &roster).0, values);
        let mut models = w.models.clone();
        models[1].counter = None;
        assert_eq!(refresh(&models, &w.wings).0, [None; 2]);
    }
}
