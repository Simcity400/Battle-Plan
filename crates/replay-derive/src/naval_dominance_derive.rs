//! Navy regional dominance source arithmetic over raw recorded operands.
//! Local selection and the client's evaluated interface are never inputs.

const SCALE: i64 = 100_000;
const REFUSED: i64 = 0xffff_ffff;

fn fraction(numerator: i64, denominator: i64) -> i64 {
    if denominator == 0 {
        return 0;
    }
    if (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        REFUSED
    } else {
        numerator.wrapping_mul(SCALE).wrapping_div(denominator)
    }
}

/// Content-defined weights, all in native fixed5 units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Defines {
    pub range_neutral: i64,
    pub speed_neutral: i64,
    pub carrier_size: i64,
    pub heavy_gun_attack: i64,
    pub strike_force_fraction: i64,
    pub strike_multiregion_decay: i64,
    pub naval_base: i64,
    pub air_base: i64,
    pub radar: i64,
}

impl Defines {
    pub fn read<E>(
        mut fixed: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            range_neutral: fixed("NNavy", "DOMINANCE_PER_SHIP_PER_RANGE_NEUTRAL")?.max(0),
            speed_neutral: fixed("NNavy", "DOMINANCE_PER_SHIP_PER_SPEED_NEUTRAL")?.max(0),
            carrier_size: fixed("NNavy", "DOMINANCE_PER_SHIP_PER_CARRIER_SIZE")?.max(0),
            heavy_gun_attack: fixed("NNavy", "DOMINANCE_PER_SHIP_PER_HEAVY_GUN_ATTACK")?.max(0),
            strike_force_fraction: fixed("NNavy", "NAVAL_DOMINANCE_STRIKE_FORCE_FRACTION")?,
            strike_multiregion_decay: fixed(
                "NNavy",
                "NAVAL_DOMINANCE_STRIKE_FORCE_MULTIREGION_DECAY",
            )?
            .clamp(0, SCALE),
            naval_base: fixed("NNavy", "NAVAL_BASE_DOMINANCE_FACTOR")?.max(0),
            air_base: fixed("NNavy", "AIR_BASE_DOMINANCE_FACTOR")?.max(0),
            radar: fixed("NNavy", "RADAR_DOMINANCE_FACTOR")?.max(0),
        })
    }
}

/// Current selected design's operands: definition weight, range, speed,
/// classification-selected carrier size and the heavy attack statistic.
pub fn ship_power(raw: [i64; 5], defines: Defines) -> i64 {
    let [weight, range, speed, carrier, attack] = raw;
    let range_divisor = defines.range_neutral.wrapping_mul(200_000) / SCALE;
    let speed_divisor = defines.speed_neutral.wrapping_mul(200_000) / SCALE;
    let range = fraction(range.wrapping_sub(defines.range_neutral), range_divisor);
    let speed = fraction(speed.wrapping_sub(defines.speed_neutral), speed_divisor);
    let carrier = carrier.wrapping_mul(defines.carrier_size) / SCALE;
    let attack = attack.wrapping_mul(defines.heavy_gun_attack) / SCALE;
    speed
        .wrapping_add(attack)
        .wrapping_add(carrier)
        .wrapping_add(range)
        .wrapping_add(SCALE)
        .wrapping_mul(weight)
        / SCALE
}

/// Native mission admission is ordered: empty ships, inactive mission and
/// inaccessible region return zero without demanding ship power or efficiency.
pub fn mission_power<E>(
    ships: i32,
    active: u8,
    accessible: bool,
    kind: u32,
    current_region_matches: bool,
    power: impl FnOnce() -> Result<i64, E>,
    efficiency: impl FnOnce() -> Result<i64, E>,
) -> Result<i64, E> {
    if ships == 0 || active == 0 || !accessible {
        return Ok(0);
    }
    let efficiency = match kind {
        1 if current_region_matches => SCALE,
        1 => return Ok(0),
        2..=4 => efficiency()?,
        _ => return Ok(0),
    };
    Ok(power()?.wrapping_mul(efficiency) / SCALE)
}

/// Strike-force contribution depends on the accessible-list count, including
/// every stored occurrence; it has no extra lower clamp for modded decay.
pub fn strike_effect(power: i64, accessible_regions: i32, defines: Defines) -> i64 {
    let power = power.wrapping_mul(defines.strike_force_fraction) / SCALE;
    let decay = i64::from(accessible_regions.wrapping_sub(1))
        .wrapping_mul(SCALE)
        .wrapping_mul(defines.strike_multiregion_decay)
        / SCALE;
    SCALE.wrapping_sub(decay).wrapping_mul(power) / SCALE
}

/// Counts use signed wrapping sums before their content coefficients.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildingCounts {
    pub naval_base: i32,
    pub air_base: i32,
    pub radar: i32,
}

impl BuildingCounts {
    pub fn modifiers(self, defines: Defines) -> [i64; 3] {
        [
            i64::from(self.naval_base).wrapping_mul(defines.naval_base),
            i64::from(self.air_base).wrapping_mul(defines.air_base),
            i64::from(self.radar).wrapping_mul(defines.radar),
        ]
    }
}

/// Target folding orders air bases, radar, then naval bases. Display's
/// independent multiplier ordering belongs to its own text branch.
pub fn building_factor(modifiers: [i64; 3]) -> i64 {
    let [naval, air, radar] = modifiers;
    naval.wrapping_add(SCALE).wrapping_mul(
        radar
            .wrapping_add(SCALE)
            .wrapping_mul(air.wrapping_add(SCALE).wrapping_mul(SCALE) / SCALE)
            / SCALE,
    ) / SCALE
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub patrol: i64,
    pub strike_multiplier: i64,
    pub enemy_raiding: i64,
    pub building_multiplier: i64,
    pub target: i64,
}

/// The caller supplies ledger occurrences in their retained order. Enemy
/// strike effects multiply raider power before the source law's final bound.
pub fn sources(
    patrol: i64,
    friendly_strike: i64,
    enemy_raiding: i64,
    enemy_strike: i64,
    building_multiplier: i64,
) -> Sources {
    let enemy_raiding = SCALE.wrapping_add(enemy_strike).wrapping_mul(enemy_raiding) / SCALE;
    let target = building_multiplier
        .wrapping_add(friendly_strike)
        .wrapping_mul(patrol)
        / SCALE;
    Sources {
        patrol,
        strike_multiplier: SCALE.wrapping_add(friendly_strike),
        enemy_raiding,
        building_multiplier,
        target: target.wrapping_sub(enemy_raiding).max(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_define_clamps_match_native_registry_groups() {
        let negative = Defines::read::<()>(|_, _| Ok(-123)).unwrap();
        assert_eq!(negative.range_neutral, 0);
        assert_eq!(negative.carrier_size, 0);
        assert_eq!(negative.naval_base, 0);
        assert_eq!(negative.strike_multiregion_decay, 0);
        assert_eq!(negative.strike_force_fraction, -123);
        assert_eq!(
            Defines::read::<()>(|_, _| Ok(300_000))
                .unwrap()
                .strike_multiregion_decay,
            100_000
        );
    }
    #[test]
    fn ship_power_uses_neutral_terms_and_independent_zero_divisors() {
        let d = Defines {
            range_neutral: 200_000,
            speed_neutral: 100_000,
            carrier_size: 10_000,
            heavy_gun_attack: 1_000,
            ..Defines::default()
        };
        assert_eq!(
            ship_power([300_000, 200_000, 100_000, 200_000, 1_000_000], d),
            390_000
        );
        assert_eq!(
            ship_power([300_000, 200_000, 100_000, 0, 0], Defines::default()),
            300_000
        );
    }
    #[test]
    fn rejected_missions_demand_no_design_or_efficiency() {
        for (ships, active, accessible, kind, region) in [
            (0, 1, true, 2, true),
            (1, 0, true, 2, true),
            (1, 1, false, 2, true),
            (1, 1, true, 1, false),
            (1, 1, true, 9, true),
        ] {
            assert_eq!(
                mission_power::<()>(
                    ships,
                    active,
                    accessible,
                    kind,
                    region,
                    || panic!("power"),
                    || panic!("efficiency")
                ),
                Ok(0)
            );
        }
        assert_eq!(
            mission_power::<()>(
                1,
                1,
                true,
                1,
                true,
                || Ok(10),
                || panic!("patrol efficiency")
            ),
            Ok(10)
        );
    }
    #[test]
    fn source_terms_keep_native_order_and_modded_negative_effects() {
        let d = Defines {
            strike_force_fraction: 10_000,
            strike_multiregion_decay: 75_000,
            ..Defines::default()
        };
        assert_eq!(strike_effect(100_000, 3, d), -5_000);
        let result = sources(1_000_000, 20_000, 100_000, 30_000, 110_000);
        assert_eq!(
            (
                result.target,
                result.enemy_raiding,
                result.strike_multiplier
            ),
            (1_300_000, 130_000, 120_000)
        );
        assert_eq!(
            sources(1_000_000, 0, -100_000, 0, 100_000).target,
            1_100_000
        );
    }
}
