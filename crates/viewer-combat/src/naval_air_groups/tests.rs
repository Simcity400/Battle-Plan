use super::*;
use replay::rows::{NavalCombatAirEfficiency, NavalShipModifierSource, NavalShipModifierTerm};

#[test]
fn sortie_content_keys_spell_the_fixed_modifier_keys() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (index, name) in SORTIE.into_iter().zip(SORTIE_KEYS) {
        assert!(
            keys::all().any(|key| key.index == index && key.name == name),
            "{name}"
        );
    }
}

fn definitions() -> Defines {
    Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        sortie_stance_ratios: vec![80_000],
        base_carrier_sortie_efficiency: ONE,
        selected_sortie_initial_time: 0,
        piercing_thresholds: Vec::new(),
        piercing_critical_values: Vec::new(),
        positioning_impacts: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
    }
}

#[test]
fn sortie_and_host_organisation_are_distinct_hover_gates() {
    let plane = NavalCombatPlane {
        efficiency: NavalCombatAirEfficiency {
            source: 3,
            container_present: 1,
            base_crowding: ONE,
            ..Default::default()
        },
        ..Default::default()
    };
    let host = CarrierHost {
        ship: 7,
        organisation: 500_000,
        maximum_organisation: 1_000_000,
        pool_flags: 0,
    };
    assert_eq!(
        efficiency_parts(
            &plane,
            0,
            0,
            None,
            Some(host),
            &NavalCombatSideDetails::default(),
            &[[Some(0); 2]; 2],
            Some(&definitions()),
        ),
        Some((ONE, 50_000, 50_000))
    );
}

#[test]
fn refused_static_modifier_values_are_named_counted_and_unavailable() {
    let refusal = |modifier: &str, key: &str, reason: &str| {
        Err(static_modifiers::Refusal {
            file: "common/modifiers/x.txt".into(),
            modifier: modifier.into(),
            key: key.into(),
            reason: reason.into(),
        })
    };
    let mut warnings = Vec::new();
    let content = Content::from_values(
        vec![
            refusal(
                "screening_bonus",
                "sortie_efficiency",
                "line 1: sortie_efficiency has a block value",
            ),
            Ok(0),
            Ok(10_000),
            refusal(
                "capital_screening_bonus",
                "fighter_sortie_efficiency",
                "definition capital_screening_bonus has a key or tag field",
            ),
        ],
        &mut warnings,
    );
    assert_eq!(content.screening, [[None, Some(0)], [Some(10_000), None]]);
    assert_eq!(
        warnings,
        [
            "Static modifier screening_bonus.sortie_efficiency in common/modifiers/x.txt: line 1: sortie_efficiency has a block value; carrier air positioning that needs it is unavailable.",
            "Static modifier capital_screening_bonus.fighter_sortie_efficiency in common/modifiers/x.txt: definition capital_screening_bonus has a key or tag field; carrier air positioning that needs it is unavailable.",
            "Static modifiers: 2 of 4 screening sortie values refused.",
        ]
    );
    let mut warnings = Vec::new();
    Content::from_values(vec![Ok(0); 4], &mut warnings);
    assert!(warnings.is_empty());
}

fn test_efficiency(
    plane: &NavalCombatPlane,
    duration: i32,
    side_index: usize,
    source: (Option<&AirBaseRow>, Option<&NavalCombatShipRow>),
    side: &NavalCombatSideDetails,
    screening: &[[Option<i64>; 2]; 2],
    defines: Option<&Defines>,
) -> Option<i64> {
    let host = source.0.and_then(|base| {
        let ship = source.1?;
        if base.id != plane.air_base || base.host != ship.ship || ship.live_ship == 0 {
            return None;
        }
        Some(CarrierHost {
            ship: ship.ship,
            organisation: ship.organisation,
            maximum_organisation: super::super::naval_details::organisation_maximum(ship).ok()?,
            pool_flags: 0,
        })
    });
    efficiency(
        plane, duration, side_index, source.1, host, side, screening, defines,
    )
}

#[test]
fn sortie_factors_apply_native_screening_weather_and_organisation() {
    let mut base = AirBaseRow {
        id: 9,
        host: 7,
        ..Default::default()
    };
    let mut ship = NavalCombatShipRow {
        ship: 7,
        live_ship: 1,
        state: 2,
        battle_slot: 3,
        formation_flags: 1 << 39,
        organisation: 500_000,
        base_organisation: 1_000_000,
        ..Default::default()
    };
    ship.weapon_context.sources = vec![
        NavalShipModifierSource {
            scope: 26,
            ..Default::default()
        },
        NavalShipModifierSource {
            scope: 0,
            terms: vec![NavalShipModifierTerm {
                key: keys::SORTIE_EFFICIENCY,
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    let mut plane = NavalCombatPlane {
        air_base: 9,
        carrier_slot: 3,
        carrier_member_present: 1,
        efficiency: NavalCombatAirEfficiency {
            source: 3,
            container_present: 1,
            base_crowding: ONE,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut side = NavalCombatSideDetails::default();
    let screening = [[Some(0), Some(0)], [Some(10_000), Some(0)]];
    let defines = definitions();
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        Some(50_000)
    );
    side.capital_screening = ONE;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        Some(55_000)
    );
    plane.efficiency.modifiers[1].present = 1;
    plane.efficiency.modifiers[1].raw = -50_000;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        Some(27_500)
    );
    ship.live_ship = 0;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        None
    );
    ship.live_ship = 1;
    ship.organisation = 250_000;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        Some(13_750)
    );
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (None, Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        None
    );
    base.host = 8;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        None
    );
    base.host = 7;
    base.id = 10;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        None
    );
    plane.efficiency.source = 2;
    assert_eq!(
        test_efficiency(&plane, 0, 0, (None, None), &side, &screening, None),
        Some(ONE)
    );
    plane.efficiency.source = 0;
    assert_eq!(
        test_efficiency(&plane, 0, 0, (None, None), &side, &screening, None),
        Some(0)
    );
}

/// A live carrier member at slot 3 of a combat side, hosting base 9,
/// whose modifier context holds only the eligibility source: no source
/// carries a sortie key.
fn screened_carrier(formation_flags: u64) -> (NavalCombatPlane, NavalCombatShipRow) {
    let mut ship = NavalCombatShipRow {
        ship: 7,
        live_ship: 1,
        state: 2,
        battle_slot: 3,
        formation_flags,
        ..Default::default()
    };
    ship.weapon_context.sources.push(NavalShipModifierSource {
        scope: 26,
        ..Default::default()
    });
    let plane = NavalCombatPlane {
        air_base: 9,
        carrier_slot: 3,
        carrier_member_present: 1,
        efficiency: NavalCombatAirEfficiency {
            source: 3,
            container_present: 1,
            base_crowding: ONE,
            ..Default::default()
        },
        ..Default::default()
    };
    (plane, ship)
}

fn host(pool_flags: u64) -> Option<CarrierHost> {
    Some(CarrierHost {
        ship: 7,
        organisation: 500_000,
        maximum_organisation: 1_000_000,
        pool_flags,
    })
}

#[test]
fn static_screening_reads_global_metadata_without_a_dynamic_sortie_key() {
    let (mut plane, ship) = screened_carrier(1 << 39);
    let side = NavalCombatSideDetails {
        capital_screening: ONE,
        ..Default::default()
    };
    let screening = [[Some(0), Some(0)], [Some(10_000), Some(0)]];
    let defines = definitions();
    let run = |plane: &NavalCombatPlane| {
        efficiency(
            plane,
            0,
            0,
            Some(&ship),
            host(0),
            &side,
            &screening,
            Some(&defines),
        )
    };
    assert_eq!(run(&plane), Some(55_000));
    // Key 0x24's own definition metadata gates and clamps the static
    // value, although no accumulator of the ship holds that key.
    plane.efficiency.modifier_metadata[2] = [0, 4];
    assert_eq!(run(&plane), Some(50_000));
    plane.efficiency.eligibility_callback = 1;
    plane.efficiency.feature_flags = 4;
    assert_eq!(run(&plane), Some(55_000));
    plane.efficiency.eligibility_callback = 2;
    assert_eq!(run(&plane), None);
    plane.efficiency.eligibility_callback = 0;
    plane.efficiency.modifier_metadata[2] = [0x10, 0];
    let clamped = [[Some(0), Some(0)], [Some(300_000), Some(0)]];
    assert_eq!(
        efficiency(
            &plane,
            0,
            0,
            Some(&ship),
            host(0),
            &side,
            &clamped,
            Some(&defines),
        ),
        Some(100_000)
    );
    // A refused static value is needed only where the law reads it.
    let refused = [[None, Some(0)], [Some(10_000), Some(0)]];
    plane.efficiency.modifier_metadata[2] = [0, 0];
    assert_eq!(
        efficiency(
            &plane,
            0,
            0,
            Some(&ship),
            host(0),
            &side,
            &refused,
            Some(&defines),
        ),
        None
    );
    let unscreened = NavalCombatSideDetails::default();
    assert_eq!(
        efficiency(
            &plane,
            0,
            0,
            Some(&ship),
            host(0),
            &unscreened,
            &refused,
            Some(&defines),
        ),
        Some(50_000)
    );
}

#[test]
fn a_pool_only_carrier_takes_the_carrier_box_and_a_capital_box_only_capital_screening() {
    let side = NavalCombatSideDetails {
        capital_screening: 50_000,
        ..Default::default()
    };
    let screening = [[Some(20_000), Some(0)], [Some(10_000), Some(0)]];
    let defines = definitions();
    let (plane, ship) = screened_carrier(0);
    let run = |pool_flags| {
        efficiency(
            &plane,
            0,
            0,
            Some(&ship),
            host(pool_flags),
            &side,
            &screening,
            Some(&defines),
        )
    };
    // Box 2: capital 0.5 x 0.2; box 3 adds combined 0.5 x 0.1.
    assert_eq!(run(0), Some(55_000));
    assert_eq!(run(1 << 22), Some(57_500));
    assert_eq!(run(1 << 37), Some(57_500));
}

#[test]
fn display_box_follows_the_native_classification() {
    let row = |state, formation_flags, live_convoy, detection_state| NavalCombatShipRow {
        state,
        formation_flags,
        live_ship: 1,
        live_convoy,
        detection_state,
        ..Default::default()
    };
    assert_eq!(display_box(&row(4, 0, 0, 0), 0), 7);
    assert_eq!(display_box(&row(16, 0, 0, 0), 0), 7);
    assert_eq!(display_box(&row(32, 0, 0, 0), 0), 6);
    assert_eq!(display_box(&row(64, 0, 0, 0), 0), 0);
    assert_eq!(display_box(&row(1, 1 << 8, 1, 0), 0), 3);
    let mut unresolved = row(2, 1 << 22, 0, 0);
    unresolved.live_ship = 0;
    assert_eq!(display_box(&unresolved, 0), 0);
    assert_eq!(display_box(&row(8, 1 << 22, 0, 0), 0), 3);
    assert_eq!(display_box(&row(8, 1 << 8, 0, 0), 1 << 37), 3);
    assert_eq!(display_box(&row(2, (1 << 7) | (1 << 22), 0, 0), 0), 4);
    assert_eq!(display_box(&row(2, 1 << 7, 0, 1), 0), 5);
    assert_eq!(display_box(&row(2, 1 << 8, 0, 0), 0), 1);
    assert_eq!(display_box(&row(2, 1 << 39, 0, 0), 0), 3);
    assert_eq!(display_box(&row(2, 0, 0, 0), 0), 2);
}

#[test]
fn early_sortie_uses_authored_mission_ratio_and_missing_content_is_unavailable() {
    let base = AirBaseRow {
        id: 9,
        host: 7,
        ..Default::default()
    };
    let mut ship = NavalCombatShipRow {
        ship: 7,
        live_ship: 1,
        organisation: 500_000,
        base_organisation: 1_000_000,
        ..Default::default()
    };
    ship.weapon_context.sources.push(NavalShipModifierSource {
        scope: 26,
        ..Default::default()
    });
    let mut plane = NavalCombatPlane {
        air_base: 9,
        carrier_slot: -1,
        efficiency: NavalCombatAirEfficiency {
            source: 3,
            mission: 0,
            container_present: 1,
            base_crowding: ONE,
            ..Default::default()
        },
        ..Default::default()
    };
    let side = NavalCombatSideDetails::default();
    let mut defines = definitions();
    defines.selected_sortie_initial_time = 1;
    let screening = [[Some(0); 2]; 2];
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        Some(40_000)
    );
    // This is the parent NavalCombatDetails.duration; the plane has no
    // second copy of the same duration operand.
    assert_eq!(
        test_efficiency(
            &plane,
            1,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        Some(50_000)
    );
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            1,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        Some(10_000)
    );
    plane.efficiency.mission = 1;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            Some(&defines)
        ),
        None
    );
    plane.efficiency.mission = 0;
    assert_eq!(
        test_efficiency(
            &plane,
            0,
            0,
            (Some(&base), Some(&ship)),
            &side,
            &screening,
            None
        ),
        None
    );
}

#[test]
fn source3_without_combat_member_uses_shared_host_organisation() {
    let mut plane = NavalCombatPlane {
        air_base: 42,
        carrier_slot: -1,
        carrier_member_present: 0,
        efficiency: NavalCombatAirEfficiency {
            source: 3,
            container_present: 1,
            base_crowding: ONE,
            ..Default::default()
        },
        ..Default::default()
    };
    let bases = [AirBaseRow {
        id: 42,
        host: 77,
        ..Default::default()
    }];
    let ships = [ShipRow {
        id: 77,
        organisation: 500_000,
        ..Default::default()
    }];
    let models = [ShipModelRow {
        ship: 77,
        counter: Some(replay::rows::ShipCounterInputs {
            maximum_organisation: 1_000_000,
            ..Default::default()
        }),
        ..Default::default()
    }];
    let host = carrier_host(&plane, Some(&bases), Some(&ships), Some(&models));
    assert!(host.is_some());
    let side = NavalCombatSideDetails::default();
    let defines = definitions();
    assert_eq!(
        efficiency(
            &plane,
            0,
            0,
            None,
            host,
            &side,
            &[[Some(0); 2]; 2],
            Some(&defines)
        ),
        Some(50_000)
    );
    // A nonnegative slot can remain after its linked member pointer is
    // cleared, so slot alone cannot turn on member-specific modifiers.
    plane.carrier_slot = 5;
    assert_eq!(
        efficiency(
            &plane,
            0,
            0,
            None,
            host,
            &side,
            &[[Some(0); 2]; 2],
            Some(&defines)
        ),
        Some(50_000)
    );
    plane.carrier_member_present = 1;
    assert_eq!(
        efficiency(
            &plane,
            0,
            0,
            None,
            host,
            &side,
            &[[Some(0); 2]; 2],
            Some(&defines)
        ),
        None
    );
    assert_eq!(
        carrier_host(&plane, None, Some(&ships), Some(&models)),
        None
    );
    assert_eq!(
        carrier_host(&plane, Some(&bases), None, Some(&models)),
        None
    );
    assert_eq!(carrier_host(&plane, Some(&bases), Some(&ships), None), None);
}

fn carrier_host(
    plane: &NavalCombatPlane,
    bases: Option<&[AirBaseRow]>,
    ships: Option<&[ShipRow]>,
    models: Option<&[ShipModelRow]>,
) -> Option<CarrierHost> {
    let models = models.and_then(|rows| Keyed::new(rows, |row| row.ship));
    CarrierJoins::new(None, bases, ships, models).host(plane)
}

/// Counts of the `referenced` bases from dense rows `(base, wings)`.
fn dense_counts<const N: usize>(
    rows: &[(u64, &[u64])],
    wings: &[AirWingRow],
    referenced: [u64; N],
) -> Option<BTreeMap<u64, i32>> {
    let rows: Vec<_> = rows
        .iter()
        .map(|&(base, wings)| NavalCombatAirBaseWingHandlesRow {
            base,
            wings: wings.to_vec(),
        })
        .collect();
    DenseBases::new(&rows, wings)?.counts(referenced)
}

#[test]
fn dense_counts_read_only_the_referenced_rows_once_each() {
    let wing = |id, planes| AirWingRow {
        id,
        planes,
        ..Default::default()
    };
    let wings = [wing(5, 3), wing(6, 2)];
    let rows: [(u64, &[u64]); 3] = [(41, &[5, 6]), (42, &[5, 5]), (43, &[7])];
    // Base 43 names a wing outside the roster; only a combat that
    // references it loses every count. A repeated reference is one row.
    assert_eq!(
        dense_counts(&rows, &wings, [42, 41, 42, 44]),
        Some(BTreeMap::from([(41, 5), (42, 6)]))
    );
    assert_eq!(dense_counts(&rows, &wings, [42, 43]), None);
    // Repeated or disordered keys refuse the tables whole.
    assert_eq!(dense_counts(&[(42, &[]), (42, &[])], &wings, [41]), None);
    assert_eq!(dense_counts(&[(42, &[]), (41, &[])], &wings, [41]), None);
    assert_eq!(dense_counts(&rows, &[wing(6, 2), wing(5, 3)], [41]), None);
    // A dense row for base zero is refused when referenced.
    assert_eq!(dense_counts(&[(0, &[])], &wings, [0]), None);
    assert_eq!(
        dense_counts(&[(0, &[])], &wings, [41]),
        Some(BTreeMap::new())
    );
}

#[test]
fn joins_refuse_missing_rows_and_duplicate_or_disordered_keys() {
    let plane = NavalCombatPlane {
        air_base: 42,
        ..Default::default()
    };
    let base = |id, host| AirBaseRow {
        id,
        host,
        ..Default::default()
    };
    let ship = |id| ShipRow {
        id,
        organisation: 1,
        ..Default::default()
    };
    let model = |ship| ShipModelRow {
        ship,
        counter: Some(replay::rows::ShipCounterInputs::default()),
        ..Default::default()
    };
    let bases = [base(41, 76), base(42, 77)];
    let ships = [ship(76), ship(77)];
    let models = [model(76), model(77)];
    assert_eq!(
        carrier_host(&plane, Some(&bases), Some(&ships), Some(&models)).map(|host| host.ship),
        Some(77)
    );
    // A duplicate or out-of-order key refuses the whole table, even for
    // a key that would otherwise resolve.
    for bases in [[base(42, 77), base(42, 76)], [base(42, 77), base(41, 76)]] {
        assert_eq!(
            carrier_host(&plane, Some(&bases), Some(&ships), Some(&models)),
            None
        );
    }
    assert_eq!(
        carrier_host(
            &plane,
            Some(&bases),
            Some(&[ship(77), ship(77)]),
            Some(&models)
        ),
        None
    );
    assert_eq!(
        carrier_host(
            &plane,
            Some(&bases),
            Some(&ships),
            Some(&[model(77), model(76)])
        ),
        None
    );
    // Missing rows, a zero host and a model without counter operands.
    assert_eq!(
        carrier_host(&plane, Some(&bases[..1]), Some(&ships), Some(&models)),
        None
    );
    assert_eq!(
        carrier_host(&plane, Some(&[base(42, 0)]), Some(&ships), Some(&models)),
        None
    );
    assert_eq!(
        carrier_host(&plane, Some(&bases), Some(&ships[..1]), Some(&models)),
        None
    );
    let mut bare = model(77);
    bare.counter = None;
    assert_eq!(
        carrier_host(&plane, Some(&bases), Some(&ships), Some(&[model(76), bare])),
        None
    );

    // Two forward members of one side and group sharing the host ship
    // are ambiguous; the other side, another group and a source-1
    // member are separate keys.
    let member = |side, group, slot, source| NavalCombatShipRow {
        combat: 7,
        side,
        group_index: group,
        battle_slot: slot,
        member_source: source,
        ship: 77,
        ..Default::default()
    };
    let rows = [
        member(0, 0, 0, 0),
        member(0, 0, 1, 1),
        member(0, 1, 2, 0),
        member(0, 1, 3, 0),
        member(1, 0, 0, 0),
    ];
    let members: Vec<_> = rows.iter().collect();
    let joins = CarrierJoins::new(Some(&members), None, None, None);
    assert_eq!(joins.member(0, 0, 77).map(|row| row.battle_slot), Some(0));
    assert_eq!(joins.member(0, 1, 77), None);
    assert_eq!(joins.member(1, 0, 77).map(|row| row.side), Some(1));
    assert_eq!(joins.member(0, 0, 78), None);
    assert_eq!(joins.member(0, 2, 77), None);
    let joins = CarrierJoins::new(None, None, None, None);
    assert_eq!(joins.member(0, 0, 77), None);
}

fn no_joins() -> CarrierJoins<'static> {
    CarrierJoins::new(None, None, None, None)
}

fn positioning_sources<'a>(
    joins: &'a CarrierJoins<'a>,
    base_aircraft: Option<&'a BTreeMap<u64, i32>>,
    content: &'a Content,
    defines: &'a Defines,
) -> PositioningAirSources<'a> {
    PositioningAirSources {
        side_index: 0,
        duration: 0,
        joins,
        base_aircraft,
        content,
        defines: Some(defines),
    }
}

#[test]
fn positioning_air_counts_dense_wing_occurrences_even_with_refused_assignment() {
    let wings = [
        AirWingRow {
            id: 5,
            planes: 3,
            // The dense occurrence, not this assigned-base backlink,
            // controls the native carrier-base count.
            air_base: 999,
            ..Default::default()
        },
        AirWingRow {
            id: 6,
            planes: 2,
            ..Default::default()
        },
    ];
    let base_aircraft =
        dense_counts(&[(42, &[5, 5, 6])], &wings, [42]).expect("complete dense wing join");
    let mut defines = definitions();
    defines.summary_fixed[12] = 2_000_000;
    let content = Content::default();
    let joins = no_joins();
    let sources = positioning_sources(&joins, Some(&base_aircraft), &content, &defines);
    let side = NavalCombatSideDetails {
        planes: vec![NavalCombatPlane {
            group_index: 0,
            air_base: 42,
            current_planes: 10,
            efficiency: NavalCombatAirEfficiency {
                source: 2,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(base_aircraft[&42], 8);
    assert_eq!(
        positioning_air(&side, &sources),
        Some(PositioningAir {
            weighted_planes: 1_000_000,
            average_base_fill: 40_000,
        })
    );
}

#[test]
fn positioning_air_requires_an_explicit_dense_base_row_and_complete_wing_join() {
    let wings: [AirWingRow; 0] = [];
    let absent = dense_counts(&[(41, &[])], &wings, [42]).unwrap();
    let mut defines = definitions();
    defines.summary_fixed[12] = 2_000_000;
    let content = Content::default();
    let joins = no_joins();
    let sources = positioning_sources(&joins, Some(&absent), &content, &defines);
    let side = NavalCombatSideDetails {
        planes: vec![NavalCombatPlane {
            group_index: 0,
            air_base: 42,
            efficiency: NavalCombatAirEfficiency {
                source: 2,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(positioning_air(&side, &sources), None);
    let sources = positioning_sources(&joins, None, &content, &defines);
    assert_eq!(positioning_air(&side, &sources), None);
    // A recorded empty dense base has a known zero aircraft count.
    let empty = dense_counts(&[(42, &[])], &wings, [42]).unwrap();
    let sources = positioning_sources(&joins, Some(&empty), &content, &defines);
    assert_eq!(
        positioning_air(&side, &sources),
        Some(PositioningAir::default())
    );
    assert_eq!(dense_counts(&[(42, &[5])], &wings, [42]), None);
}

#[test]
fn positioning_air_weights_mixed_sortie_efficiency_and_all_admitted_slots() {
    let mut ship = NavalCombatShipRow {
        combat: 7,
        side: 0,
        member_source: 0,
        group_index: 0,
        ship: 77,
        live_ship: 1,
        base_organisation: 1_000_000,
        ..Default::default()
    };
    ship.weapon_context.sources.push(NavalShipModifierSource {
        scope: 26,
        ..Default::default()
    });
    let wings = [AirWingRow {
        id: 5,
        planes: 8,
        ..Default::default()
    }];
    let base_aircraft = dense_counts(&[(42, &[5])], &wings, [42]).unwrap();
    let mut defines = definitions();
    defines.summary_fixed[12] = 2_000_000;
    let content = Content {
        screening: [[Some(0); 2]; 2],
        equipment: None,
    };
    let ships = [ship];
    let bases = [AirBaseRow {
        id: 42,
        host: 77,
        ..Default::default()
    }];
    let live_ships = [ShipRow {
        id: 77,
        organisation: 500_000,
        ..Default::default()
    }];
    let models = [ShipModelRow {
        ship: 77,
        counter: Some(replay::rows::ShipCounterInputs {
            maximum_organisation: 1_000_000,
            ..Default::default()
        }),
        ..Default::default()
    }];
    let members: Vec<_> = ships.iter().collect();
    let joins = CarrierJoins::new(
        Some(&members),
        Some(&bases),
        Some(&live_ships),
        Keyed::new(&models, |row| row.ship),
    );
    let sources = positioning_sources(&joins, Some(&base_aircraft), &content, &defines);
    let side = NavalCombatSideDetails {
        planes: vec![
            NavalCombatPlane {
                group_index: 0,
                air_base: 42,
                carrier_slot: -1,
                current_planes: 10,
                efficiency: NavalCombatAirEfficiency {
                    source: 3,
                    container_present: 1,
                    base_crowding: ONE,
                    ..Default::default()
                },
                ..Default::default()
            },
            NavalCombatPlane {
                group_index: 0,
                air_base: 42,
                current_planes: 5,
                efficiency: NavalCombatAirEfficiency {
                    source: 2,
                    ..Default::default()
                },
                ..Default::default()
            },
            NavalCombatPlane {
                group_index: 0,
                current_planes: 100,
                efficiency: NavalCombatAirEfficiency::default(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert_eq!(
        positioning_air(&side, &sources),
        Some(PositioningAir {
            weighted_planes: 1_000_000,
            average_base_fill: 20_000,
        })
    );
}

#[test]
fn positioning_air_excludes_flagged_retired_and_paired_only_air() {
    let defines = definitions();
    let content = Content::default();
    let joins = no_joins();
    let sources = positioning_sources(&joins, None, &content, &defines);
    let active = NavalCombatPlane {
        source: 0,
        group_index: 0,
        current_planes: 10,
        efficiency: NavalCombatAirEfficiency {
            source: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut flagged = active.clone();
    flagged.flags[1] = 1;
    flagged.current_planes = 100;
    let mut retired = active.clone();
    retired.source = 1;
    retired.current_planes = 100;
    let mut side_array = active.clone();
    side_array.source = 2;
    side_array.current_planes = 100;
    let mut paired_only = active.clone();
    paired_only.group_index = u32::MAX;
    paired_only.current_planes = 100;
    let side = NavalCombatSideDetails {
        planes: vec![active, flagged, retired, side_array, paired_only],
        ..Default::default()
    };
    assert_eq!(
        positioning_air(&side, &sources),
        Some(PositioningAir {
            weighted_planes: 1_000_000,
            average_base_fill: 0,
        })
    );
}
