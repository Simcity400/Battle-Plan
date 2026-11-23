
use crate::rows::ModifierDefinitionRow;
pub use game_model::fixed5::power;
use engine_native_parts::modifier_definition::Operands;

const SCALE: i64 = 100_000;
const REFUSED: i64 = 0xffff_ffff;

fn divisible(numerator: i64) -> bool {
    (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

fn fraction(numerator: i64, denominator: i64) -> i64 {
    if divisible(numerator) {
        numerator.wrapping_mul(SCALE).wrapping_div(denominator)
    } else {
        REFUSED
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Defines {
    pub convoy_defense_max_convoy_to_ship_ratio: i64,
    pub convoy_defense_max_region_to_taskforce_ratio: i64,
    pub coordination_effect_on_convoy_defense_efficiency: i64,
    pub convoy_raid_max_region_to_taskforce_ratio: i64,
    pub coordination_effect_on_convoy_raid_efficiency: i64,
    pub mission_efficiency_pow_factor: i64,
    pub convoy_escort_presence_warning_threshold: i64,
}

impl Defines {
    pub fn read<E>(
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        let mut navy = |name| fixed5("NNavy", name);
        Ok(Self {
            convoy_defense_max_convoy_to_ship_ratio: navy(
                "CONVOY_DEFENSE_MAX_CONVOY_TO_SHIP_RATIO",
            )?,
            convoy_defense_max_region_to_taskforce_ratio: navy(
                "CONVOY_DEFENSE_MAX_REGION_TO_TASKFORCE_RATIO",
            )?,
            coordination_effect_on_convoy_defense_efficiency: navy(
                "COORDINATION_EFFECT_ON_CONVOY_DEFENSE_EFFICIENCY",
            )?,
            convoy_raid_max_region_to_taskforce_ratio: navy(
                "CONVOY_RAID_MAX_REGION_TO_TASKFORCE_RATIO",
            )?,
            coordination_effect_on_convoy_raid_efficiency: navy(
                "COORDINATION_EFFECT_ON_CONVOY_RAID_EFFICIENCY",
            )?,
            mission_efficiency_pow_factor: navy("MISSION_EFFICIENCY_POW_FACTOR")?,
            convoy_escort_presence_warning_threshold: fixed5(
                "NInterface",
                "CONVOY_ESCORT_PRESENCE_WARNING_THRESHOLD",
            )?,
        })
    }
}

pub fn effective(
    definition: &ModifierDefinitionRow,
    features: Option<u32>,
    raw: Option<i64>,
) -> Result<i64, &'static str> {
    let Some(raw) = raw else {
        return Ok(0);
    };
    let eligibility_flags = match (definition.feature_selector, definition.eligibility) {
        (0, _) | (_, 0) => 0,
        _ => features.ok_or("a feature-gated modifier needs the recorded DLC feature mask")?,
    };
    Ok(Operands {
        flags: definition.flags,
        selector: definition.feature_selector,
        eligibility_flags,
    }
    .effective(raw))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Location {
    pub terrain: i64,
    pub weather: i64,
    pub weather_reduction: i64,
    pub local: i64,
}

pub fn combined(
    country: i64,
    location: Option<Location>,
    leader: i64,
    headquarters: impl IntoIterator<Item = i64>,
) -> i64 {
    let mut value = country;
    if let Some(location) = location {
        value = value
            .wrapping_add(location.terrain)
            .wrapping_add(location.weather.wrapping_mul(location.weather_reduction) / SCALE)
            .wrapping_add(location.local);
    }
    let strongest = headquarters.into_iter().fold(0, i64::max);
    value.wrapping_add(leader).wrapping_add(strongest)
}

pub fn headquarters_sum(values: [i64; 4]) -> i64 {
    values
        .into_iter()
        .fold(0, |sum, value| sum.wrapping_add(value))
}

pub fn weather_reduction(modifier: i64) -> i64 {
    SCALE.wrapping_add(modifier).clamp(0, SCALE)
}

pub fn terrain_mean(classes: impl IntoIterator<Item = (i32, i64)>) -> i64 {
    let (mut count, mut sum) = (0_i32, 0_i64);
    for (ships, value) in classes {
        count = count.wrapping_add(ships);
        sum = sum.wrapping_add(i64::from(ships).wrapping_mul(value));
    }
    if count == 0 {
        0
    } else {
        fraction(sum, i64::from(count).wrapping_mul(SCALE))
    }
}

pub fn fuel_fraction(fuel: i64, requested: i64) -> i64 {
    if requested < 1 || !divisible(fuel) {
        SCALE
    } else {
        fraction(fuel, requested).clamp(0, SCALE)
    }
}

pub fn underway_bonus(numerator: i32, denominator: i32, modifier: i64, define: i64) -> i64 {
    let ratio = if denominator == 0 {
        SCALE
    } else {
        fraction(
            i64::from(numerator).wrapping_mul(SCALE),
            i64::from(denominator).wrapping_mul(SCALE),
        )
    }
    .clamp(-SCALE, SCALE);
    modifier
        .wrapping_add(SCALE)
        .wrapping_mul(define)
        .wrapping_div(SCALE)
        .wrapping_mul(ratio)
        .wrapping_div(SCALE)
}

#[allow(clippy::too_many_arguments)]
pub fn mission_range(
    base: i64,
    additive: i64,
    factor: i64,
    fuel: i64,
    requested: i64,
    fuel_define: i64,
    underway: i64,
    distance_define: i64,
) -> i64 {
    let fuel = fuel_fraction(fuel, requested);
    let mut factor = factor;
    if fuel < SCALE {
        factor = factor.wrapping_add(
            SCALE
                .wrapping_sub(fuel)
                .wrapping_mul(fuel_define)
                .wrapping_div(SCALE),
        );
    }
    let scale = factor
        .wrapping_add(underway)
        .wrapping_add(SCALE)
        .max(10_000);
    let range = base
        .wrapping_add(additive)
        .wrapping_mul(scale)
        .wrapping_div(SCALE)
        .wrapping_mul(distance_define)
        .wrapping_div(SCALE);
    fraction(range, 200_000)
}

fn coordinated(coordination: i64, effect: i64, ratio: i64) -> i64 {
    (coordination.wrapping_mul(effect) / SCALE)
        .wrapping_add(SCALE)
        .wrapping_mul(ratio)
        / SCALE
}

pub fn escort_capacity(coordination: i64, defines: &Defines) -> i64 {
    coordinated(
        coordination,
        defines.coordination_effect_on_convoy_defense_efficiency,
        defines.convoy_defense_max_convoy_to_ship_ratio,
    )
}

pub fn escort_region_ratio(coordination: i64, defines: &Defines) -> i64 {
    coordinated(
        coordination,
        defines.coordination_effect_on_convoy_defense_efficiency,
        defines.convoy_defense_max_region_to_taskforce_ratio,
    )
}

pub fn raid_region_ratio(coordination: i64, defines: &Defines) -> i64 {
    coordinated(
        coordination,
        defines.coordination_effect_on_convoy_raid_efficiency,
        defines.convoy_raid_max_region_to_taskforce_ratio,
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EscortShare {
    pub ships: u32,
    pub capacity: i64,
}

pub fn escort_efficiency(
    shares: impl IntoIterator<Item = EscortShare>,
    convoys: i32,
    escort: i64,
    region_ratio: i64,
    range: [i32; 2],
    defines: &Defines,
) -> Result<i64, &'static str> {
    let mut ships = 0_u32;
    let mut capacity = 0_i64;
    let mut count = 0_i32;
    for share in shares {
        ships = ships.wrapping_add(share.ships);
        capacity = capacity.wrapping_add(share.capacity);
        count = count.wrapping_add(1);
    }
    let forces = i64::from(count).wrapping_mul(SCALE);
    let average = if forces == 0 {
        0
    } else {
        fraction(capacity, forces)
    };
    let factor = escort.wrapping_add(SCALE);
    let covered = i64::from(ships as i32)
        .wrapping_mul(average)
        .wrapping_mul(factor)
        / SCALE;
    let convoys = i64::from(convoys).wrapping_mul(SCALE);
    let convoy = if convoys == 0 {
        0
    } else {
        fraction(covered, convoys).clamp(0, SCALE)
    };
    let region = (region_ratio.wrapping_mul(forces) / SCALE).wrapping_mul(factor) / SCALE;
    let span = i64::from(range[1].wrapping_sub(range[0])).wrapping_mul(SCALE);
    let coverage = if span == 0 {
        0
    } else {
        fraction(region, span).clamp(0, SCALE)
    };
    power(
        coverage.wrapping_mul(convoy) / SCALE,
        defines.mission_efficiency_pow_factor,
    )
}

pub fn raiding_efficiency(
    fleet: bool,
    range: [i32; 2],
    raiders: i32,
    region_ratio: i64,
    raiding_factor: i64,
    raiding: i64,
    defines: &Defines,
) -> Result<i64, &'static str> {
    let span = range[1].wrapping_sub(range[0]);
    if !fleet || span == 0 {
        return Ok(SCALE);
    }
    let strength = (i64::from(raiders)
        .wrapping_mul(SCALE)
        .wrapping_mul(region_ratio)
        / SCALE)
        .wrapping_mul(raiding_factor.wrapping_add(SCALE))
        / SCALE;
    let coverage = fraction(strength, i64::from(span).wrapping_mul(SCALE));
    let value = (raiding.wrapping_add(SCALE).wrapping_mul(coverage) / SCALE).clamp(0, SCALE);
    power(value, defines.mission_efficiency_pow_factor)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registration {
    pub kind: u32,
    pub fleet: u64,
    pub convoys: i32,
}

pub fn escort_ledger(
    registrations: &[Registration],
    mut efficiency: impl FnMut(usize) -> Result<i64, &'static str>,
) -> Result<i64, &'static str> {
    if registrations.is_empty() {
        return Ok(-SCALE);
    }
    let mut best = -SCALE;
    let mut convoys = -1_i32;
    let mut fleets = std::collections::HashSet::new();
    for (index, registration) in registrations.iter().enumerate() {
        if registration.kind != 4 || !fleets.insert(registration.fleet) {
            continue;
        }
        best = best.max(efficiency(index)?);
        convoys = convoys.max(registration.convoys);
    }
    Ok(if best == -SCALE || convoys == -1 {
        -SCALE
    } else {
        best
    })
}

pub fn raiding_ledger(efficiencies: impl IntoIterator<Item = i64>) -> i64 {
    let mut sum = 0_u64;
    let mut count = 0_i32;
    for efficiency in efficiencies {
        sum = sum.wrapping_add(efficiency as u64);
        count = count.wrapping_add(1);
    }
    let count = i64::from(count).wrapping_mul(SCALE);
    if count == 0 {
        0
    } else if sum.wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        (sum.wrapping_mul(SCALE as u64) as i64).wrapping_div(count)
    } else {
        REFUSED
    }
}

pub fn presence(tracked: bool, positive: i32, total: i32) -> i64 {
    if !tracked {
        return -SCALE;
    }
    let total = i64::from(total).wrapping_mul(SCALE);
    if total == 0 {
        SCALE
    } else if i64::from(positive).wrapping_mul(SCALE) < -0x7fff_ffff_ffff_fffe {
        REFUSED
    } else {
        i64::from(positive)
            .wrapping_mul(SCALE * SCALE)
            .wrapping_div(total)
    }
}

pub fn escort_shown(escort: i64, presence: i64) -> i64 {
    let presence = if presence == -SCALE { SCALE } else { presence };
    escort.wrapping_mul(presence) / SCALE
}

pub fn escort_alert(escort: i64, presence: i64, defines: &Defines) -> bool {
    (escort as u64) < SCALE as u64
        || (0..defines.convoy_escort_presence_warning_threshold).contains(&presence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defines() -> Defines {
        Defines {
            convoy_defense_max_convoy_to_ship_ratio: 1_000_000,
            convoy_defense_max_region_to_taskforce_ratio: 150_000,
            coordination_effect_on_convoy_defense_efficiency: 50_000,
            convoy_raid_max_region_to_taskforce_ratio: 150_000,
            coordination_effect_on_convoy_raid_efficiency: 50_000,
            mission_efficiency_pow_factor: 50_000,
            convoy_escort_presence_warning_threshold: 50_000,
        }
    }

    #[test]
    fn p2_range_uses_stored_fuel_underway_and_native_guards() {
        assert_eq!(fuel_fraction(25_000, 100_000), 25_000);
        assert_eq!(fuel_fraction(-1, 0), 100_000);
        assert_eq!(fuel_fraction(i64::MIN, 1), 100_000);
        assert_eq!(underway_bonus(1, 2, 20_000, 50_000), 30_000);
        assert_eq!(underway_bonus(-2, 1, 0, 50_000), -50_000);
        assert_eq!(underway_bonus(0, 0, 0, 50_000), 50_000);
        assert_eq!(
            mission_range(
                10_000_000, 2_000_000, 10_000, 50_000, 100_000, -50_000, 30_000, 100_000
            ),
            6_900_000
        );
        assert_eq!(
            mission_range(-1_000_000, 0, -200_000, 0, 0, 0, 0, 100_000),
            -50_000
        );
        assert_eq!(terrain_mean([(2, 100_000), (1, -50_000)]), 50_000);
        assert_eq!(terrain_mean([]), 0);
    }

    #[test]
    fn defines_are_read_by_name_and_the_first_failure_refuses() {
        let mut names = Vec::new();
        let read = Defines::read(|section, name| {
            names.push((section, name));
            Ok::<_, ()>(names.len() as i64)
        })
        .unwrap();
        assert_eq!(read.convoy_defense_max_convoy_to_ship_ratio, 1);
        assert_eq!(read.convoy_escort_presence_warning_threshold, 7);
        assert_eq!(
            names.last(),
            Some(&("NInterface", "CONVOY_ESCORT_PRESENCE_WARNING_THRESHOLD"))
        );
        assert!(
            Defines::read(|_, name| if name.starts_with("MISSION") {
                Err(name)
            } else {
                Ok(0)
            })
            .is_err()
        );
    }

    #[test]
    fn effective_values_follow_the_modifier_getter() {
        let plain = ModifierDefinitionRow::default();
        assert_eq!(effective(&plain, None, None), Ok(0));
        assert_eq!(effective(&plain, None, Some(-7)), Ok(-7));
        let clamped = ModifierDefinitionRow {
            flags: engine_native_parts::modifier_definition::CLAMP_MAXIMUM,
            ..plain.clone()
        };
        assert_eq!(effective(&clamped, None, Some(-7)), Ok(0));
        assert_eq!(effective(&clamped, None, Some(250_000)), Ok(100_000));
        let gated = ModifierDefinitionRow {
            feature_selector: 2,
            eligibility: 1,
            ..plain.clone()
        };
        assert!(effective(&gated, None, Some(9)).is_err());
        assert_eq!(effective(&gated, Some(4), Some(9)), Ok(0));
        assert_eq!(effective(&gated, Some(6), Some(9)), Ok(9));
        let unset = ModifierDefinitionRow {
            eligibility: 0,
            ..gated
        };
        assert_eq!(effective(&unset, None, Some(9)), Ok(0));
        assert_eq!(effective(&unset, None, None), Ok(0));
    }

    #[test]
    fn combined_adds_location_terms_only_when_located_and_takes_the_strongest_headquarters() {
        let location = Location {
            terrain: 1_000,
            weather: -20_000,
            weather_reduction: 50_000,
            local: 300,
        };
        assert_eq!(combined(5_000, Some(location), 700, [400, -9]), -2_600);
        assert_eq!(combined(5_000, None, 700, [-400]), 5_700);
        assert_eq!(headquarters_sum([1, 2, 3, 4]), 10);
        assert_eq!(weather_reduction(-30_000), 70_000);
        assert_eq!(weather_reduction(-130_000), 0);
        assert_eq!(weather_reduction(10_000), 100_000);
    }

    #[test]
    fn escort_efficiency_combines_convoy_and_region_coverage() {
        let defines = Defines {
            mission_efficiency_pow_factor: 100_000,
            ..defines()
        };
        assert_eq!(escort_capacity(0, &defines), 1_000_000);
        assert_eq!(escort_region_ratio(20_000, &defines), 165_000);
        let shares = [
            EscortShare {
                ships: 3,
                capacity: 1_000_000,
            },
            EscortShare {
                ships: 1,
                capacity: 1_000_000,
            },
        ];
        let value = escort_efficiency(shares, 40, 10_000, 150_000, [1, 5], &defines).unwrap();
        assert_eq!(value, power(82_500, 100_000).unwrap());
        assert_eq!(
            escort_efficiency(shares, 0, 10_000, 150_000, [1, 5], &defines).unwrap(),
            power(0, 100_000).unwrap()
        );
        assert_eq!(
            escort_efficiency(shares, 40, 10_000, 150_000, [5, 5], &defines).unwrap(),
            power(0, 100_000).unwrap()
        );
    }

    #[test]
    fn raiding_efficiency_is_full_without_a_fleet_or_range() {
        let defines = Defines {
            mission_efficiency_pow_factor: 100_000,
            ..defines()
        };
        assert_eq!(raid_region_ratio(0, &defines), 150_000);
        assert_eq!(
            raiding_efficiency(false, [1, 5], 2, 150_000, 0, 0, &defines),
            Ok(100_000)
        );
        assert_eq!(
            raiding_efficiency(true, [5, 5], 2, 150_000, 0, 0, &defines),
            Ok(100_000)
        );
        let value = raiding_efficiency(true, [0, 8], 2, 150_000, 20_000, -10_000, &defines);
        assert_eq!(value, power(40_500, 100_000));
    }

    #[test]
    fn ledgers_keep_the_native_sentinels_and_first_fleet_rule() {
        let escort = |fleet, convoys| Registration {
            kind: 4,
            fleet,
            convoys,
        };
        let ledger = [
            Registration {
                kind: 3,
                fleet: 1,
                convoys: 99,
            },
            escort(1, 5),
            escort(1, 50),
            escort(2, 7),
        ];
        let mut asked = Vec::new();
        let value = escort_ledger(&ledger, |index| {
            asked.push(index);
            Ok([0, 40_000, 90_000, 60_000][index])
        });
        assert_eq!((value, asked), (Ok(60_000), vec![1, 3]));
        assert_eq!(escort_ledger(&[], |_| Ok(0)), Ok(-100_000));
        assert_eq!(escort_ledger(&ledger[..1], |_| Ok(0)), Ok(-100_000));
        assert_eq!(escort_ledger(&[escort(1, -1)], |_| Ok(5)), Ok(-100_000));
        assert_eq!(raiding_ledger([40_000, 60_000, 80_000]), 60_000);
        assert_eq!(raiding_ledger([]), 0);
        assert_eq!(presence(false, 0, 0), -100_000);
        assert_eq!(presence(true, 0, 0), 100_000);
        assert_eq!(presence(true, 1, 3), 33_333);
    }

    #[test]
    fn the_shown_escort_scales_by_presence_and_alerts_on_either_input() {
        let defines = defines();
        assert_eq!(escort_shown(80_000, -100_000), 80_000);
        assert_eq!(escort_shown(80_000, 50_000), 40_000);
        assert_eq!(escort_shown(-100_000, -100_000), -100_000);
        assert!(!escort_alert(-100_000, -100_000, &defines));
        assert!(escort_alert(99_999, 100_000, &defines));
        assert!(!escort_alert(100_000, 100_000, &defines));
        assert!(escort_alert(100_000, 0, &defines));
        assert!(escort_alert(-100_000, 49_999, &defines));
        assert!(!escort_alert(100_000, 50_000, &defines));
    }

    const POW_25000_HALF: i64 = 50_003;
    const POW_82500_HALF: i64 = 90_831;
    const POW_40500_THREE_HALVES: i64 = 25_777;
    const POW_250000_SQUARE: i64 = 624_991;

    #[test]
    fn power_preserves_fixed5_scalar_expectations() {
        for (base, exponent, expected) in [
            (100_000, 50_000, 100_000),
            (0, 50_000, 100_000),
            (25_000, 50_000, POW_25000_HALF),
            (82_500, 50_000, POW_82500_HALF),
            (40_500, 150_000, POW_40500_THREE_HALVES),
            (250_000, 200_000, POW_250000_SQUARE),
        ] {
            assert_eq!(power(base, exponent), Ok(expected), "{base} ^ {exponent}");
        }
        assert_eq!(power(-25_000, 50_000), Ok(POW_25000_HALF));
        assert_eq!(power(-25_000, 50_001), Ok(-power(25_000, 50_001).unwrap()));
    }
}
