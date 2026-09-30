//! Current naval-combat surface detection from shared simulation operands.
//!
//! This does not bind the positioning widget. The caller must supply complete
//! tables where needed. Carrier aircraft counts must be derived from the game's
//! dense air-base containers, not its separately indexed registration table.

use super::{keyed::Keyed, naval_environment::modifier_default_leader};
use engine::modifier_keys as keys;
use replay::rows::{NavalCombatShipRow, ShipModelRow};
use std::collections::BTreeMap;

const ONE: i64 = 100_000;

/// CarrierAirBonus@140C23CA0. The native carrier predicate includes both the
/// ship's current formation flags and flags in its positive equipment pool;
/// ShipModels supplies both current values without a duplicate combat copy.
/// CountAirBaseAircraft@140C3E1D0 walks dense CAirBase+0x48 containers.
/// AirBaseRegistrations comes from the separately indexed +0x30 array; a
/// duplicate country ordinal can remain in the dense vector after the indexed
/// slot is overwritten. The caller supplies counts derived from complete dense
/// occurrence rows and complete AirWings. An absent base key is unknown, whereas
/// an explicit dense row with no wings has a known zero count.
fn carrier_air_bonus(
    member: &NavalCombatShipRow,
    model: &ShipModelRow,
    dense_counts: Option<&BTreeMap<u64, i32>>,
) -> Option<i64> {
    let flags = model.formation_flags | model.counter.as_ref()?.positive_equipment_flags;
    if flags & ((1_u64 << 22) | (1_u64 << 37)) == 0
        || member.weapon_context.air_base == 0
        || member.weapon_context.air_base_maximum == 0
        || member.weapon_context.air_detection_base == 0
    {
        return Some(0);
    }
    let count = *dense_counts?.get(&member.weapon_context.air_base)?;
    let capacity = i64::from(member.weapon_context.air_base_maximum) * ONE;
    // 140C23CA0 does signed integer division and clamps the fixed-point fill.
    // Its preceding count*100000 overflow guard is unreachable for an i32
    // count; the later 64-bit numerator can wrap for anomalously huge counts.
    let fill = i64::from(count)
        .wrapping_mul(10_000_000_000)
        .wrapping_div(capacity)
        .clamp(0, ONE);
    Some(fill.wrapping_mul(member.weapon_context.air_detection_base) / ONE)
}

/// CShip::GetSurfaceDetection@140C2B3A0. Its commander=0 modifier call uses
/// the fleet default leader (scopes 31/32), not the combat side's selected
/// leader (scopes 9/10).
fn ship_surface_detection(
    member: &NavalCombatShipRow,
    model: &ShipModelRow,
    dense_counts: Option<&BTreeMap<u64, i32>>,
) -> Option<i64> {
    let modifier = modifier_default_leader(&member.weapon_context, keys::NAVAL_DETECTION)?;
    let base = ONE
        .wrapping_add(modifier)
        .wrapping_mul(member.weapon_context.detection_base)
        / ONE;
    Some(
        base.wrapping_add(carrier_air_bonus(member, model, dense_counts)?)
            .max(0),
    )
}

/// live ships in one combat side. Source-1 historical/retired members and
/// paired-only members do not participate, and state bits 2, 4 and 5 veto a
/// member. `models` is the binding's shared ShipModels lookup, `None` for an
/// incomplete table; `dense_counts=None` means a missing or incomplete
/// dense-base/AirWings census.
pub(super) fn sum_active_surface_detection(
    members: &[&NavalCombatShipRow],
    models: Option<Keyed<'_, ShipModelRow>>,
    dense_counts: Option<&BTreeMap<u64, i32>>,
    side: u8,
) -> Option<i64> {
    let active: Vec<_> = members
        .iter()
        .copied()
        .filter(|member| {
            member.side == side
                && member.member_source == 0
                && member.group_index != u32::MAX
                && member.state & 0x34 == 0
                && member.live_ship != 0
        })
        .collect();
    if active.is_empty() {
        return Some(0);
    }
    let models = models?;
    let mut total = 0_i64;
    for member in active {
        let model = models.get(member.ship)?;
        total = total.wrapping_add(ship_surface_detection(member, model, dense_counts)?);
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{NavalShipModifierSource, NavalShipModifierTerm};

    fn keyed(models: &[ShipModelRow]) -> Option<Keyed<'_, ShipModelRow>> {
        Keyed::new(models, |model| model.ship)
    }

    fn term(scope: u8, key: u32, raw: i64) -> NavalShipModifierSource {
        NavalShipModifierSource {
            scope,
            terms: vec![NavalShipModifierTerm {
                key,
                raw,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn member(ship: u64) -> NavalCombatShipRow {
        NavalCombatShipRow {
            ship,
            live_ship: 1,
            group_index: 0,
            weapon_context: replay::rows::NavalShipWeaponContext {
                detection_base: ONE,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn model(ship: u64) -> ShipModelRow {
        ShipModelRow {
            ship,
            counter: Some(Default::default()),
            ..Default::default()
        }
    }

    #[test]
    fn default_leader_replaces_selected_combat_leader_including_severity() {
        let mut ship = member(1);
        ship.weapon_context.sources = vec![
            term(0, 0x2a, 10_000),
            term(9, 0x2a, 50_000),
            term(10, 0x2a, 40_000),
            term(31, 0x2a, 20_000),
            term(32, 0x2a, 30_000),
        ];
        assert_eq!(
            sum_active_surface_detection(&[&ship], keyed(&[model(1)]), None, 0),
            Some(160_000)
        );
        ship.weapon_context.sources = vec![
            term(11, 0x2a, 10_000),
            term(9, 0x1bb, 50_000),
            term(31, 0x1bb, 20_000),
        ];
        ship.weapon_context.sources[0].weights[0] = 1;
        assert_eq!(
            sum_active_surface_detection(&[&ship], keyed(&[model(1)]), None, 0),
            Some(112_000),
            "the critical severity's recursive modifier keeps the same leader"
        );
    }

    #[test]
    fn carrier_aircraft_count_requires_an_explicit_dense_base_row() {
        let mut ship = member(1);
        ship.weapon_context.air_base = 7;
        ship.weapon_context.air_base_maximum = 10;
        ship.weapon_context.air_detection_base = 40_000;
        let mut carrier = model(1);
        carrier
            .counter
            .get_or_insert_default()
            .positive_equipment_flags = 1 << 37;
        assert_eq!(sum_active_surface_detection(&[&ship], None, None, 0), None);
        assert_eq!(
            sum_active_surface_detection(&[&ship], keyed(&[carrier.clone()]), None, 0),
            None,
            "an indexed registration census cannot prove a dense count"
        );
        assert_eq!(
            sum_active_surface_detection(
                &[&ship],
                keyed(&[carrier.clone()]),
                Some(&BTreeMap::new()),
                0,
            ),
            None,
            "a complete dense table without this combat base cannot prove zero"
        );
        assert_eq!(
            sum_active_surface_detection(
                &[&ship],
                keyed(&[carrier.clone()]),
                Some(&BTreeMap::from([(7, 0)])),
                0,
            ),
            Some(100_000),
            "an explicit empty dense base row proves zero"
        );
        assert_eq!(
            sum_active_surface_detection(
                &[&ship],
                keyed(&[carrier.clone()]),
                Some(&BTreeMap::from([(7, 7)])),
                0,
            ),
            Some(128_000)
        );
        let mut formation_carrier = model(1);
        formation_carrier.formation_flags = 1 << 22;
        assert_eq!(
            sum_active_surface_detection(
                &[&ship],
                keyed(&[formation_carrier]),
                Some(&BTreeMap::from([(7, 7)])),
                0,
            ),
            Some(128_000)
        );
        assert_eq!(
            sum_active_surface_detection(
                &[&ship],
                keyed(&[carrier.clone()]),
                Some(&BTreeMap::from([(7, 20)])),
                0,
            ),
            Some(140_000),
            "the aircraft fill clamps to one"
        );
        ship.weapon_context.air_base_maximum = -10;
        assert_eq!(
            sum_active_surface_detection(
                &[&ship],
                keyed(&[carrier.clone()]),
                Some(&BTreeMap::from([(7, -7)])),
                0,
            ),
            Some(128_000),
            "signed division preserves two negative operands"
        );
        ship.weapon_context.air_base_maximum = 10;
        let noncarrier = model(1);
        assert_eq!(
            sum_active_surface_detection(&[&ship], keyed(&[noncarrier]), None, 0),
            Some(100_000)
        );
        ship.weapon_context.air_base_maximum = 0;
        assert_eq!(
            sum_active_surface_detection(&[&ship], keyed(&[carrier.clone()]), None, 0),
            Some(100_000),
            "zero capacity proves a zero bonus without the dense count"
        );
        ship.weapon_context.air_base_maximum = 10;
        ship.weapon_context.air_detection_base = 0;
        assert_eq!(
            sum_active_surface_detection(&[&ship], keyed(&[carrier.clone()]), None, 0),
            Some(100_000),
            "zero bonus base also makes the dense count irrelevant"
        );
        ship.weapon_context.air_detection_base = 40_000;
        ship.weapon_context.air_base = 0;
        assert_eq!(
            sum_active_surface_detection(&[&ship], keyed(&[carrier]), None, 0),
            Some(100_000)
        );
    }

    #[test]
    fn only_forward_live_unvetoed_members_contribute() {
        let good = member(1);
        let mut retired = member(2);
        retired.member_source = 1;
        let mut paired = member(3);
        paired.group_index = u32::MAX;
        let mut withdrawn = member(4);
        withdrawn.state = 0x4;
        let mut wrong_side = member(5);
        wrong_side.side = 1;
        let mut convoy = member(6);
        convoy.live_ship = 0;
        assert_eq!(
            sum_active_surface_detection(
                &[&good, &retired, &paired, &withdrawn, &wrong_side, &convoy],
                keyed(&[model(1)]),
                None,
                0,
            ),
            Some(ONE)
        );
        assert_eq!(
            sum_active_surface_detection(&[&retired], None, None, 0),
            Some(0)
        );
    }
}
