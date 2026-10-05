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
    /// The zero-screen country correction was positive. 14160A7B0 appends
    /// its 0x1B3 breakdown even when this is false and the branch ran.
    pub country_bonus_applied: bool,
}

#[derive(Default)]
pub(super) struct Census {
    pub capitals: i64,
    pub screens: i64,
    pub carriers: i64,
    pub convoys: i64,
    pub eligible_submarines: i64,
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
pub(super) fn ceil_fixed(value: i64) -> i64 {
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
pub(super) fn census(
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
    let mut country_bonus_applied = false;
    if capital == 0 && !summary.countries.is_empty() && counts.eligible_submarines == 0 {
        let bonus =
            modifier(summary, 0, 3).ok_or("convoy screening modifier callback is unsupported")?;
        if bonus > 0 {
            // Native applies this after the first clamp, with no second cap.
            capital = capital.wrapping_add(bonus);
            country_bonus_applied = true;
        }
    }
    let capital_demand = ceil_fixed(
        counts
            .convoys
            .wrapping_mul(defines.summary_fixed[11])
            .wrapping_add(counts.carriers.wrapping_mul(defines.summary_fixed[10])),
    );
    let carrier = efficiency(counts.capitals, capital_demand, factor);
    Ok(Screening {
        capital,
        carrier,
        country_bonus_applied,
    })
}

#[cfg(test)]
mod tests;
