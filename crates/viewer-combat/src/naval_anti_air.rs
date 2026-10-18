//! Fresh anti-air value displayed by the active naval-combat window.
use engine_executable::modifier_keys as keys;
use replay_data::rows::NavalCombatShipRow;

const ONE: i64 = 100_000;

/// the game visits every live ship occurrence in the registered forward
/// groups, without filtering member state or detection. the game uses the
/// task force's default commander rather than the combat leader for keys
/// 0x88/0x89. The base lookup joins the shared hourly ShipModels census.
pub(super) fn calculate(
    combat: u64,
    side: u8,
    positioning: i64,
    ships: &[NavalCombatShipRow],
    ship_to_fleet_ratio: i64,
    mut anti_air_base: impl FnMut(u64) -> Option<i64>,
) -> Result<i64, &'static str> {
    let mut sum = 0_i64;
    for row in ships.iter().filter(|row| {
        row.combat == combat
            && row.side == side
            && row.member_source == 0
            && row.group_index != u32::MAX
            && row.live_ship != 0
    }) {
        let base = anti_air_base(row.ship).ok_or("live ship AA base is unavailable")?;
        let mut context = row.weapon_context.clone();
        context
            .sources
            .retain(|source| !matches!(source.scope, 9 | 10));
        for source in &mut context.sources {
            source.scope = match source.scope {
                31 => 9,
                32 => 10,
                other => other,
            };
        }
        let additive = super::naval_environment::modifier(&context, keys::NAVY_ANTI_AIR_ATTACK)
            .ok_or("ship AA additive modifier is unavailable")?;
        let multiplicative =
            super::naval_environment::modifier(&context, keys::NAVY_ANTI_AIR_ATTACK_FACTOR)
                .ok_or("ship AA multiplicative modifier is unavailable")?;
        let ship_aa = base
            .wrapping_add(additive)
            .wrapping_mul(ONE.wrapping_add(multiplicative))
            / ONE;
        sum = sum.wrapping_add(ship_aa);
    }
    let fleet = sum.wrapping_mul(ship_to_fleet_ratio) / ONE;
    Ok(fleet.wrapping_mul(positioning) / ONE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::NavalShipModifierSource, rows::NavalShipModifierTerm};

    fn ship(state: u32, source: u8, group: u32) -> NavalCombatShipRow {
        NavalCombatShipRow {
            combat: 7,
            side: 1,
            ship: 45,
            state,
            member_source: source,
            group_index: group,
            live_ship: 1,
            ..Default::default()
        }
    }

    #[test]
    fn forward_live_occurrences_include_reserve_and_repeat_ship() {
        let mut convoy = ship(2, 0, 0);
        convoy.live_ship = 0;
        convoy.live_convoy = 1;
        let rows = [
            ship(16, 0, 0),
            ship(2, 0, 1),
            ship(2, 1, 1),
            ship(2, 0, u32::MAX),
            convoy,
        ];
        // Two eligible occurrences of 1.5, then ratio 0.5 and position 0.8.
        assert_eq!(
            calculate(7, 1, 80_000, &rows, 50_000, |_| Some(150_000)),
            Ok(120_000)
        );
        assert_eq!(calculate(7, 0, 80_000, &rows, 50_000, |_| None), Ok(0));
        assert_eq!(
            calculate(7, 1, 80_000, &rows, 50_000, |_| None),
            Err("live ship AA base is unavailable")
        );
    }

    #[test]
    fn default_commander_terms_replace_combat_leader_terms() {
        let mut row = ship(2, 0, 0);
        let term = |key, raw| NavalShipModifierTerm {
            key,
            raw,
            ..Default::default()
        };
        row.weapon_context.sources = vec![
            NavalShipModifierSource {
                scope: 9,
                terms: vec![term(0x88, 90_000)],
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 31,
                terms: vec![term(0x88, 20_000), term(0x89, -10_000)],
                ..Default::default()
            },
        ];
        // (1.0 + 0.2) * (1 - 0.1) = 1.08, then fleet ratio 0.5 and
        // fresh positioning 0.8, with a truncation after each multiply.
        assert_eq!(
            calculate(7, 1, 80_000, &[row], 50_000, |_| Some(100_000)),
            Ok(43_200)
        );
    }
}
