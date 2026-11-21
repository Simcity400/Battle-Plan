use crate::rows::{CountryCommandPowerRow, CountryStabilityInputRow};

pub use crate::fixed_point::{fixed5_to_q15, q15_to_fixed5};

fn clamp(value: i64, minimum: i64, maximum: i64) -> i64 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms {
    pub terms: Vec<(&'static str, Option<i64>)>,
    pub unclamped: Option<i64>,
    pub value: Option<i64>,
}

fn sum(terms: &[(&'static str, Option<i64>)]) -> Option<i64> {
    terms
        .iter()
        .try_fold(0_i64, |total, (_, term)| Some(total.wrapping_add((*term)?)))
}

pub fn popularity_term(factor: i64, popularity: i64) -> i64 {
    let product = factor.wrapping_mul(popularity) / 100_000;
    if [i64::MAX, i64::MIN, i64::MIN + 1].contains(&product) {
        0xffff_ffff
    } else {
        product.wrapping_mul(100_000) / 10_000_000
    }
}

pub fn stability(row: &CountryStabilityInputRow) -> Terms {
    let at_war = row.at_war != 0;
    let offensive = at_war && row.offensive_war != 0;
    let defensive = at_war && row.defensive_war != 0;
    let war = if offensive || defensive {
        (|| {
            let mut war = row
                .war_stability_factor?
                .wrapping_add(row.base_stability_war_factor);
            if offensive {
                war = war.wrapping_add(row.offensive_war_stability_factor?);
            }
            if defensive {
                war = war.wrapping_add(row.defensive_war_stability_factor?);
            }
            Some(war)
        })()
    } else {
        Some(0)
    };
    let popularity = row.party_popularity_stability_factor.map(|factor| {
        popularity_term(
            row.base_stability_party_popularity_factor
                .wrapping_add(factor),
            row.ruling_party_popularity_raw,
        )
    });
    let terms = vec![
        ("base", Some(row.stability_raw)),
        ("war", war),
        ("popularity", popularity),
        (
            "propaganda",
            Some(q15_to_fixed5(row.propaganda_stability_q15)),
        ),
        ("stabilityFactor", row.stability_factor),
        (
            "coastalProtection",
            Some(row.coastal_protection_stability_raw),
        ),
    ];
    let unclamped = sum(&terms);
    Terms {
        value: unclamped.map(|value| clamp(value, row.min_stability, row.max_stability)),
        terms,
        unclamped,
    }
}

pub fn interception_term(row: &CountryStabilityInputRow) -> i64 {
    let scaled = fixed5_to_q15(row.interception_sum_raw)
        .wrapping_mul(row.interception_war_support_scale_q15)
        >> 15;
    let value = q15_to_fixed5(scaled);
    let impact = row.interception_bombing_war_support_impact;
    let lowest = impact.wrapping_neg();
    if value < lowest {
        lowest
    } else if value > impact {
        impact
    } else {
        value
    }
}

pub fn war_support(row: &CountryStabilityInputRow) -> Terms {
    let at_war = row.at_war != 0;
    let terms = vec![
        ("base", Some(row.war_support_raw)),
        (
            "offensiveWar",
            Some(if at_war && row.offensive_war != 0 {
                row.war_support_offensive_war
            } else {
                0
            }),
        ),
        (
            "defensiveWar",
            Some(if at_war && row.defensive_war != 0 {
                row.war_support_defensive_war
            } else {
                0
            }),
        ),
        (
            "tension",
            Some(
                row.world_tension_raw
                    .wrapping_mul(row.war_support_tension_impact)
                    / 100_000,
            ),
        ),
        ("bombing", Some(q15_to_fixed5(row.bombing_war_support_q15))),
        ("interception", Some(interception_term(row))),
        ("heroes", Some(q15_to_fixed5(row.heroes_war_support_q15))),
        ("convoys", Some(q15_to_fixed5(row.convoys_war_support_q15))),
        (
            "propaganda",
            Some(q15_to_fixed5(row.propaganda_war_support_q15)),
        ),
        ("warSupportFactor", row.war_support_factor),
    ];
    let unclamped = sum(&terms);
    Terms {
        value: unclamped.map(|value| clamp(value, row.min_war_support, row.max_war_support)),
        terms,
        unclamped,
    }
}

pub fn max_command_power(
    row: &CountryStabilityInputRow,
    launch: Option<&CountryCommandPowerRow>,
) -> Terms {
    let max = launch.and_then(|launch| launch.max_command_power);
    let mult = launch.and_then(|launch| launch.max_command_power_factor);
    let allocated = launch.map(|launch| launch.allocated_command_power_raw);
    let value = (|| {
        let base = row.base_max_command_power.wrapping_add(max?);
        Some((mult?.wrapping_add(100_000).wrapping_mul(base) / 100_000).wrapping_sub(allocated?))
    })();
    Terms {
        terms: vec![
            ("baseMaxCommandPower", Some(row.base_max_command_power)),
            ("maxCommandPower", max),
            ("maxCommandPowerMult", mult),
            ("allocatedCommandPower", allocated),
        ],
        unclamped: value,
        value,
    }
}
