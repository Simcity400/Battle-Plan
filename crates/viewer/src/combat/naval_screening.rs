//! The active naval window's fresh screen and capital-screen calculations.
use super::keyed::Keyed;
use installed_content::naval_combat_defines::Defines;
use replay::rows::{
    NavalCombatShipRow, NavalCombatSideDetails, NavalCombatSummaryInputs, ShipModelRow,
};

const ONE: i64 = 100_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Screening {
    /// Screens protecting capitals and convoys (the capital GUI percentage).
    pub capital: i64,
    /// Capitals protecting carriers and convoys (the carrier GUI percentage).
    pub carrier: i64,
}

#[derive(Default)]
struct Census {
    capitals: i64,
    screens: i64,
    carriers: i64,
    convoys: i64,
    eligible_submarines: i64,
}

// 140551820: absent or feature-ineligible modifiers contribute zero. Metadata
// bit 4 clamps the individual value, before the four source values are added.
fn modifier(summary: &NavalCombatSummaryInputs, source: usize, key: usize) -> Option<i64> {
    let value = &summary.modifiers[source][key];
    if value.present == 0 {
        return Some(0);
    }
    let [flags, selector] = summary.modifier_metadata[key];
    if selector != 0 {
        match summary.eligibility_callback {
            0 => return Some(0),
            1 if selector & summary.feature_flags == 0 => return Some(0),
            1 => (),
            _ => return None,
        }
    }
    Some(if flags & 0x10 != 0 {
        value.raw.clamp(0, ONE)
    } else {
        value.raw
    })
}

// 14047BDE0 divides signed fixed5 by ONE and rounds upward only when a
// positive remainder was truncated. Thus negative fractional demand rounds
// toward zero, as the native routine does.
fn ceil_fixed(value: i64) -> i64 {
    value / ONE + i64::from(value > 0 && value % ONE != 0)
}

// The native fixed divider uses 0xffffffff on a zero denominator or its
// signed-overflow guard. Category counts and authored ratios have much
// narrower valid ranges, but keep the sentinel for malformed operands too.
fn fixed_ratio(count: i64, demand: i64) -> i64 {
    let Some(denominator) = demand.checked_mul(ONE) else {
        return 0xffff_ffff;
    };
    let Some(numerator) = count
        .checked_mul(ONE)
        .and_then(|value| value.checked_mul(ONE))
    else {
        return 0xffff_ffff;
    };
    if denominator == 0 {
        0xffff_ffff
    } else {
        numerator / denominator
    }
}

fn efficiency(count: i64, demand: i64, factor: i64) -> i64 {
    if count == 0 && demand == 0 {
        return ONE;
    }
    // 14160A7B0 first divides count by the rounded demand, then multiplies
    // by the positioning factor; the truncation between those stages matters.
    fixed_ratio(count, demand)
        .wrapping_mul(factor)
        .wrapping_div(ONE)
        .clamp(0, ONE)
}

/// 14160A7B0 calls 141C55600 with state mask 0xB and detection parameter 1.
/// Parameter 1 bypasses the detection filter. Only registered forward groups
/// enter either census; a live ship and convoy reference are tested separately.
fn census(
    combat: u64,
    side: u8,
    ships: &[NavalCombatShipRow],
    models: &Keyed<'_, ShipModelRow>,
) -> Result<Census, &'static str> {
    let mut counts = Census::default();
    for row in ships.iter().filter(|row| {
        row.combat == combat
            && row.side == side
            && row.member_source == 0
            && row.group_index != u32::MAX
    }) {
        if row.live_ship != 0 {
            let flags = row.formation_flags;
            // outside the category census. It excludes 4, 16 and 32.
            if row.state & 0x34 == 0 && flags & (1 << 7) != 0 {
                counts.eligible_submarines = counts.eligible_submarines.wrapping_add(1);
            }
            if row.state & 0xb != 0 {
                if flags & (1 << 6) != 0 {
                    counts.capitals = counts.capitals.wrapping_add(1);
                } else if flags & (1 << 8) != 0 {
                    counts.screens = counts.screens.wrapping_add(1);
                } else if flags & (1 << 7) == 0 {
                    let carrier_bits = (1 << 22) | (1 << 37);
                    let carrier = flags & carrier_bits != 0
                        || models
                            .get(row.ship)
                            .ok_or("current live ship has no ShipModels row")?
                            .counter
                            .as_ref()
                            .ok_or("current live ship has no counter operands")?
                            .positive_equipment_flags
                            & carrier_bits
                            != 0;
                    if carrier {
                        counts.carriers = counts.carriers.wrapping_add(1);
                    }
                }
            }
        }
        if row.state & 0xb != 0 && row.live_convoy != 0 {
            counts.convoys = counts.convoys.wrapping_add(1);
        }
    }
    Ok(counts)
}

/// Reconstruct the values refreshed by 1417D9C60. That caller computes these
/// before its fresh positioning header, so the input is stored +0x138, never
/// the later displayed positioning calculation or stored screening +0x128/130.
pub(super) fn calculate(
    combat: u64,
    side: u8,
    details: &NavalCombatSideDetails,
    ships: &[NavalCombatShipRow],
    models: &Keyed<'_, ShipModelRow>,
    defines: &Defines,
) -> Result<Screening, &'static str> {
    let counts = census(combat, side, ships, models)?;
    let summary = &details.summary;
    let mut modifiers = 0_i64;
    if !summary.countries.is_empty() {
        for source in 0..4 {
            modifiers = modifiers.wrapping_add(
                modifier(summary, source, 1).ok_or("summary modifier callback is unsupported")?,
            );
        }
    }
    let penalty = defines.summary_fixed[7];
    let factor = ONE
        .wrapping_add(modifiers)
        .wrapping_sub(penalty.wrapping_sub(details.positioning.wrapping_mul(penalty) / ONE))
        .max(1_000);
    let screen_demand = ceil_fixed(
        counts
            .convoys
            .wrapping_mul(defines.summary_fixed[9])
            .wrapping_add(
                counts
                    .capitals
                    .wrapping_add(counts.carriers)
                    .wrapping_mul(defines.summary_fixed[8]),
            ),
    );
    let mut capital = efficiency(counts.screens, screen_demand, factor);
    if capital == 0 && !summary.countries.is_empty() && counts.eligible_submarines == 0 {
        let bonus =
            modifier(summary, 0, 3).ok_or("convoy screening modifier callback is unsupported")?;
        if bonus > 0 {
            // Native applies this after the first clamp, with no second cap.
            capital = capital.wrapping_add(bonus);
        }
    }
    let capital_demand = ceil_fixed(
        counts
            .convoys
            .wrapping_mul(defines.summary_fixed[11])
            .wrapping_add(counts.carriers.wrapping_mul(defines.summary_fixed[10])),
    );
    let carrier = efficiency(counts.capitals, capital_demand, factor);
    Ok(Screening { capital, carrier })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::ShipCounterInputs;

    fn defines() -> Defines {
        Defines {
            summary_fixed: [0; 14],
            summary_integer: [0; 3],
            sortie_stance_ratios: Vec::new(),
            base_carrier_sortie_efficiency: 0,
            selected_sortie_initial_time: 0,
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
                carrier: ONE
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
            // These recorded values are deliberately unrelated to the fresh
            // window result and must have no influence on it.
            capital_screening: 3,
            carrier_screening: 7,
            ..Default::default()
        };
        let ships = [member(1, 1 << 6), member(2, 1 << 8), member(3, 1 << 22)];
        assert_eq!(
            calculate(0, 0, &side, &ships, &models(&[]), &authored),
            Ok(Screening {
                capital: 10_000,
                carrier: 40_000
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
                carrier: ONE
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
        ships[2].detection_state = 0; // detect=1 bypasses this test.
        ships[3].state = 4; // incoming is outside the category mask.
        ships[4].member_source = 1; // retired group is not traversed.
        ships[5].group_index = u32::MAX; // paired-only group is not traversed.
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
        // A capital's formation bit settles its category without the model.
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
                carrier: ONE
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
}
