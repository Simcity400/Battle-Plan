use super::*;
use replay_data::rows::ShipCounterInputs;

fn defines() -> Defines {
    Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: Vec::new(),
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing: Ok(Default::default()),
        positioning_impacts: [0; 3],
    }
}

fn member(ship: u64, flags: u64) -> NavalCombatShipRow {
    NavalCombatShipRow {
        side: 0,
        member_source: 0,
        group_index: 0,
        battle_slot: ship as u32,
        ship,
        state: 2,
        live_ship: 1,
        formation_flags: flags,
        ..Default::default()
    }
}

fn model(ship: u64, positive_equipment_flags: u64) -> ShipModelRow {
    ShipModelRow {
        ship,
        counter: Some(ShipCounterInputs {
            positive_equipment_flags,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn models(rows: &[ShipModelRow]) -> Keyed<'_, ShipModelRow> {
    Keyed::new(rows, |row| row.ship).unwrap()
}

#[test]
fn empty_demand_is_fully_screened_and_signed_ceil_matches_native() {
    let detail = NavalCombatSideDetails::default();
    assert_eq!(
        calculate(0, 0, &detail, &[], &models(&[]), &defines()),
        Ok(Screening {
            capital: ONE,
            carrier: ONE,
            country_bonus_applied: false,
        })
    );
    assert_eq!(ceil_fixed(100_001), 2);
    assert_eq!(ceil_fixed(-100_001), -1);
    assert_eq!(ceil_fixed(-99_999), 0);
}

#[test]
fn rounded_demands_use_stored_positioning_and_two_stage_fixed_arithmetic() {
    let mut authored = defines();
    authored.summary_fixed[7] = 40_000;
    authored.summary_fixed[8] = 4 * ONE;
    authored.summary_fixed[10] = 2 * ONE;
    let side = NavalCombatSideDetails {
        positioning: 50_000,
        capital_screening: 3,
        carrier_screening: 7,
        ..Default::default()
    };
    let ships = [member(1, 1 << 6), member(2, 1 << 8), member(3, 1 << 22)];
    assert_eq!(
        calculate(0, 0, &side, &ships, &models(&[]), &authored),
        Ok(Screening {
            capital: 10_000,
            carrier: 40_000,
            country_bonus_applied: false,
        })
    );
    assert_eq!(efficiency(2, 3, 70_000), 46_666);
}

#[test]
fn low_positioning_floors_factor_at_one_percent() {
    let mut authored = defines();
    authored.summary_fixed[7] = 150_000;
    authored.summary_fixed[8] = ONE;
    let ships = [member(1, 1 << 6), member(2, 1 << 8)];
    assert_eq!(
        calculate(
            0,
            0,
            &NavalCombatSideDetails::default(),
            &ships,
            &models(&[]),
            &authored,
        ),
        Ok(Screening {
            capital: 1_000,
            carrier: ONE,
            country_bonus_applied: false,
        })
    );
}

#[test]
fn census_uses_forward_state_mask_and_carrier_before_submarine_bit() {
    let mut ships = vec![
        member(1, 1 << 6),
        member(2, 1 << 8),
        member(3, 1 << 7),
        member(4, 0),
        member(5, 1 << 6),
        member(6, 1 << 8),
    ];
    ships[2].detection_state = 0;
    ships[3].state = 4;
    ships[4].member_source = 1;
    ships[5].group_index = u32::MAX;
    let carrier_with_sub = member(7, (1 << 7) | (1 << 22));
    ships.push(carrier_with_sub);
    let mut convoy = member(8, 0);
    convoy.live_ship = 0;
    convoy.live_convoy = 1;
    ships.push(convoy);
    let mut foreign = member(9, 1 << 6);
    foreign.combat = 99;
    ships.push(foreign);
    let model_rows = [model(4, 1 << 37)];
    let counts = census(0, 0, &ships, &models(&model_rows)).unwrap();
    assert_eq!(counts.capitals, 1);
    assert_eq!(counts.screens, 1);
    assert_eq!(counts.carriers, 0);
    assert_eq!(counts.convoys, 1);
    assert_eq!(counts.eligible_submarines, 2);
    ships[3].state = 2;
    let counts = census(0, 0, &ships, &models(&model_rows)).unwrap();
    assert_eq!(counts.carriers, 1);
}

#[test]
fn missing_required_carrier_model_fails_closed() {
    let ship = member(1, 0);
    assert_eq!(
        census(0, 0, &[ship], &models(&[])).err(),
        Some("current live ship has no ShipModels row")
    );
    assert!(census(0, 0, &[member(1, 1 << 6)], &models(&[])).is_ok());
}

#[test]
fn positioning_modifiers_use_four_own_scopes_only_with_countries() {
    let mut authored = defines();
    authored.summary_fixed[8] = 2 * ONE;
    let ships = [member(1, 1 << 6), member(2, 1 << 8)];
    let mut side = NavalCombatSideDetails::default();
    side.summary.countries.push(7);
    for (scope, raw) in [10_000, 20_000, -5_000, 5_000, 90_000]
        .into_iter()
        .enumerate()
    {
        side.summary.modifiers[scope][1].present = 1;
        side.summary.modifiers[scope][1].raw = raw;
    }
    assert_eq!(
        calculate(0, 0, &side, &ships, &models(&[]), &authored)
            .unwrap()
            .capital,
        65_000
    );
    side.summary.countries.clear();
    assert_eq!(
        calculate(0, 0, &side, &ships, &models(&[]), &authored)
            .unwrap()
            .capital,
        50_000
    );
}

#[test]
fn modifier_eligibility_clamp_and_convoy_bonus_follow_native_gates() {
    let mut side = NavalCombatSideDetails::default();
    side.summary.countries.push(7);
    side.summary.modifiers[0][1].present = 1;
    side.summary.modifiers[0][1].raw = 150_000;
    side.summary.modifier_metadata[1] = [0x10, 0x4];
    side.summary.feature_flags = 0x4;
    side.summary.eligibility_callback = 1;
    assert_eq!(modifier(&side.summary, 0, 1), Some(ONE));
    side.summary.feature_flags = 0x8;
    assert_eq!(modifier(&side.summary, 0, 1), Some(0));
    side.summary.eligibility_callback = 0;
    assert_eq!(modifier(&side.summary, 0, 1), Some(0));
    side.summary.eligibility_callback = 2;
    assert_eq!(modifier(&side.summary, 0, 1), None);

    side.summary.modifiers[0][1].present = 0;
    side.summary.eligibility_callback = 0;
    side.summary.modifiers[0][3].present = 1;
    side.summary.modifiers[0][3].raw = 120_000;
    let mut authored = defines();
    authored.summary_fixed[8] = ONE;
    let capital = member(1, 1 << 6);
    assert_eq!(
        calculate(
            0,
            0,
            &side,
            std::slice::from_ref(&capital),
            &models(&[]),
            &authored
        ),
        Ok(Screening {
            capital: 120_000,
            carrier: ONE,
            country_bonus_applied: true,
        })
    );
    let submarine = member(2, 1 << 7);
    assert_eq!(
        calculate(
            0,
            0,
            &side,
            &[capital.clone(), submarine.clone()],
            &models(&[]),
            &authored
        )
        .unwrap()
        .capital,
        0
    );
    let mut incoming = submarine;
    incoming.state = 4;
    assert_eq!(
        calculate(0, 0, &side, &[capital, incoming], &models(&[]), &authored)
            .unwrap()
            .capital,
        120_000
    );
    side.summary.countries.clear();
    assert_eq!(
        calculate(0, 0, &side, &[member(1, 1 << 6)], &models(&[]), &authored)
            .unwrap()
            .capital,
        0
    );
}
