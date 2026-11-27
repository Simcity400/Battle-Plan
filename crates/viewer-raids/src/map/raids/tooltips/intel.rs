use super::super::{RaidAssets, script::World};
use replay_data::{
    rows::RaidCounterOperative, rows::RaidIntelInputRow, rows::RaidScope, rows::tables,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Defines {
    pub lanes: [i64; 4],
    pub network: i64,
    pub neighbour: i64,
    pub air: i64,
    pub decryption: i64,
    pub counter: i64,
    pub base_counter: i64,
    pub operative_base: i64,
    pub stacking: i64,
    pub foreign_agent: i64,
    pub crypto: dashboard_text::cryptology::Defines,
    pub required: i64,
}
#[cfg(test)]
impl Default for Defines {
    fn default() -> Self {
        Self {
            lanes: [0; 4],
            network: 0,
            neighbour: 0,
            air: 0,
            decryption: 0,
            counter: 0,
            base_counter: 0,
            operative_base: 0,
            stacking: 0,
            foreign_agent: 0,
            crypto: dashboard_text::cryptology::Defines {
                base: 0,
                upgrade: 0,
            },
            required: 0,
        }
    }
}

impl Defines {
    pub fn read(d: &content_reader_defines::defines::Defines) -> Result<Self, String> {
        let raid = |name| d.fixed5("NRaids", name).map_err(|e| e.to_string());
        let op = |name| d.fixed5("NOperatives", name).map_err(|e| e.to_string());
        Ok(Self {
            lanes: [
                raid("TARGET_INTEL_PER_CIVILIAN_INTEL_OVER_COUNTRY")?,
                raid("TARGET_INTEL_PER_ARMY_INTEL_OVER_COUNTRY")?,
                raid("TARGET_INTEL_PER_NAVY_INTEL_OVER_COUNTRY")?,
                raid("TARGET_INTEL_PER_AIRFORCE_INTEL_OVER_COUNTRY")?,
            ],
            network: raid("TARGET_INTEL_PER_NETWORK_STRENGTH")?,
            neighbour: raid("TARGET_INTEL_FROM_CONTROLLED_NEIGHBOUR_STATES")?,
            air: raid("TARGET_INTEL_PER_AIR_SUPERIORITY")?,
            decryption: raid("TARGET_INTEL_FROM_DECRYPTION")?,
            counter: raid("TARGET_INTEL_PENALTY_PER_ENEMY_COUNTER_INTEL")?,
            base_counter: op("BASE_COUNTER_INTELLIGENCE_RATING")?,
            operative_base: op("OPERATIVE_BASE_INTEL_AGENCY_DEFENSE")?,
            stacking: op("COUNTER_INTELLIGENCE_STACKING_FACTOR")?,
            foreign_agent: op("COUNTER_INTELLIGENCE_FOREIGN_AGENT_FACTOR")?,
            crypto: dashboard_text::cryptology::Defines::read(d)?,
            required: raid("TARGET_DETECTION_INTEL_TRESHOLD")?,
        })
    }
}

fn times(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}

pub fn air_ratio(friendly: i64, enemy: i64) -> Result<i64, String> {
    let total = friendly.wrapping_add(enemy);
    if total < 1 {
        return Ok(50_000);
    }
    if (friendly as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return Ok(0xffff_ffff);
    }
    friendly
        .wrapping_mul(100_000)
        .checked_div(total)
        .ok_or_else(|| "air superiority signed division overflow".into())
}

pub(crate) fn counteroperative_base(
    operative: &RaidCounterOperative,
    base: i64,
    foreign: i64,
) -> i64 {
    let value = base
        .wrapping_add(operative.defense)
        .wrapping_add(operative.context);
    if foreign == 0 {
        value
    } else {
        times(value, 100_000_i64.wrapping_add(foreign))
    }
}

pub(crate) fn contributions(
    operatives: &[RaidCounterOperative],
    base: i64,
    factor: i64,
    foreign: i64,
) -> Result<Vec<(u64, i64)>, String> {
    let mut unique = BTreeMap::new();
    for operative in operatives {
        let value = counteroperative_base(operative, base, foreign);
        if unique
            .insert(operative.handle, value)
            .is_some_and(|old| old != value)
        {
            return Err("repeated counteroperative has conflicting operands".into());
        }
    }
    let mut values: Vec<_> = unique
        .into_iter()
        .map(|(handle, value)| (value, handle as u32, (handle >> 32) as u32, handle))
        .collect();
    values.sort_unstable();
    let mut out = Vec::with_capacity(values.len());
    let mut scale = None;
    for (value, _, _, handle) in values.into_iter().rev() {
        match scale {
            None => {
                out.push((handle, value));
                scale = Some(factor);
            }
            Some(weight) => {
                out.push((handle, times(value, weight)));
                scale = Some(times(weight, factor));
            }
        }
    }
    Ok(out)
}

fn stacked(
    operatives: &[RaidCounterOperative],
    base: i64,
    factor: i64,
    foreign: i64,
) -> Result<i64, String> {
    Ok(contributions(operatives, base, factor, foreign)?
        .into_iter()
        .fold(0_i64, |sum, (_, value)| sum.wrapping_add(value)))
}

pub fn defense(row: &RaidIntelInputRow, d: &Defines) -> Result<i64, String> {
    match row.strategic_present {
        0 => Ok(0),
        1 => Ok(d
            .base_counter
            .wrapping_add(row.country_defense)
            .wrapping_add(
                stacked(
                    &row.operatives,
                    d.operative_base,
                    d.stacking,
                    d.foreign_agent,
                )?
                .max(0),
            )),
        _ => Err("RaidIntelInputs strategic_present is not a boolean".into()),
    }
}

pub fn decrypted(
    amount: i64,
    active: u8,
    strength: Option<i64>,
    d: &Defines,
) -> Result<bool, String> {
    dashboard_text::cryptology::fully_decrypted(
        amount,
        active,
        strength.map(|strength| dashboard_text::cryptology::threshold(strength, d.crypto)),
    )
    .ok_or_else(|| "target has no authenticated cryptology owner".into())
}

pub struct Terms {
    pub lane: i64,
    pub network: i64,
    pub neighbour: bool,
    pub air: i64,
    pub decrypted: bool,
    pub counter: i64,
}
pub fn total(terms: Terms, lane: usize, d: &Defines) -> i64 {
    times(terms.lane, d.lanes[lane])
        .wrapping_add(times(terms.network, d.network))
        .wrapping_add(if terms.neighbour { d.neighbour } else { 0 })
        .wrapping_add(
            terms
                .air
                .wrapping_sub(50_000)
                .wrapping_mul(d.air)
                .wrapping_mul(100)
                / 100_000,
        )
        .wrapping_add(if terms.decrypted { d.decryption } else { 0 })
        .wrapping_sub(times(terms.counter, d.counter))
}

pub fn scores(world: &World<'_>, country: u32, region: u32) -> Result<(i64, i64), String> {
    let rows = world
        .frame()
        .complete::<tables::RaidAirScores>()
        .map_err(|e| e.to_string())?;
    let mut found = false;
    for row in rows
        .iter()
        .filter(|r| world.slot(r.country) == Some(country))
    {
        found = true;
        if row.region == region {
            return Ok((row.friendly, row.enemy));
        }
    }
    if found {
        Ok((0, 0))
    } else {
        Err(format!(
            "RaidAirScores has no owner witness for country {country}"
        ))
    }
}

pub fn current(
    assets: &RaidAssets,
    world: &World<'_>,
    scope: &RaidScope,
    ours: u32,
    lane: usize,
    d: &Defines,
) -> Result<i64, String> {
    if ours == 0 {
        return Ok(0);
    }
    let target = super::super::target_country(world, scope)?.ok_or("target has no controller")?;
    let state = target_state(world, scope)?.ok_or("target has no state")?;
    let frame = world.frame();
    let inputs = frame
        .complete::<tables::RaidIntelInputs>()
        .map_err(|e| e.to_string())?;
    let target_inputs = inputs
        .iter()
        .find(|row| world.slot(row.country) == Some(target))
        .ok_or("target country lacks RaidIntelInputs")?;
    let ours_inputs = inputs
        .iter()
        .find(|row| world.slot(row.country) == Some(ours))
        .ok_or("viewing country lacks RaidIntelInputs")?;
    let intel = frame
        .complete::<tables::CountryIntel>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| world.slot(row.owner_country) == Some(ours) && row.target_index == target)
        .map_or(0, |r| [r.civilian, r.army, r.navy, r.air][lane]);
    let network = if ours_inputs.strategic_present == 0 {
        0
    } else {
        frame
            .complete::<tables::IntelNetworks>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|r| {
                world.slot(r.owner_country) == Some(ours)
                    && world.slot(r.target_country) == Some(target)
                    && r.state_id == state as i32
            })
            .map_or(0, |r| r.local_strength_raw)
    };
    let neighbours = assets
        .trigger_content
        .neighbor_states
        .as_ref()
        .map_err(Clone::clone)?;
    let neighbour = neighbours
        .get(&state)
        .ok_or("target state has no neighbour roster")?
        .iter()
        .any(|id| world.state_controller(*id) == Some(ours));
    let first = world
        .state_order(state)
        .map_err(|why| why.to_string())?
        .first()
        .ok_or("target state has no province")?;
    let region = *assets
        .regions
        .get(first)
        .ok_or("target state has no strategic region")?;
    let (friendly, enemy) = scores(world, ours, region)?;
    let cryptology = frame
        .complete::<tables::Cryptology>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|r| {
            world.slot(r.owner_country) == Some(ours)
                && world.slot(r.target_country) == Some(target)
        });
    let decrypted = match cryptology {
        None => false,
        Some(row) => decrypted(
            row.amount_raw,
            row.bonus_active,
            target_inputs.crypto_strength,
            d,
        )?,
    };
    Ok(total(
        Terms {
            lane: intel,
            network,
            neighbour,
            air: air_ratio(friendly, enemy)?,
            decrypted,
            counter: defense(target_inputs, d)?,
        },
        lane,
        d,
    ))
}

pub fn target_state(world: &World<'_>, scope: &RaidScope) -> Result<Option<u32>, String> {
    content_reader_raids::raids::target::state(world.frame(), scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_air_neutral_ratio_and_sentinel() {
        assert_eq!(air_ratio(0, 0), Ok(50000));
        assert_eq!(air_ratio(1, 2), Ok(33333));
        assert_eq!(air_ratio(i64::MAX, 0), Ok(0xffff_ffff));
        assert_eq!(air_ratio(i64::MAX, 1), Ok(50000));
    }
    #[test]
    fn counter_operatives_are_unique_and_stack_largest_first() {
        let op = |handle, defense| RaidCounterOperative {
            handle,
            defense,
            context: 0,
        };
        assert_eq!(
            stacked(
                &[op(1, 99999), op(2, 88888), op(1, 99999), op(3, 77777)],
                0,
                33333,
                0
            ),
            Ok(138269)
        );
        assert!(stacked(&[op(1, 2), op(1, 3)], 0, 33333, 0).is_err());
        let d = Defines {
            base_counter: 100001,
            operative_base: 0,
            stacking: 40_000,
            ..Default::default()
        };
        let mut row = RaidIntelInputRow {
            strategic_present: 1,
            country_defense: -3,
            operatives: vec![op(1, -10)],
            ..Default::default()
        };
        assert_eq!(defense(&row, &d), Ok(99998));
        row.strategic_present = 0;
        assert_eq!(defense(&row, &d), Ok(0));
        row.strategic_present = 2;
        assert!(defense(&row, &d).is_err());
    }
    #[test]
    fn foreign_agent_factor_multiplies_each_base_before_stacking() {
        let op = |handle, defense| RaidCounterOperative {
            handle,
            defense,
            context: 0,
        };
        let operatives = [op(1, 100_000), op(2, 50_000)];
        assert_eq!(
            contributions(&operatives, 0, 50_000, 0),
            Ok(vec![(1, 100_000), (2, 25_000)])
        );
        assert_eq!(
            contributions(&operatives, 0, 50_000, 50_000),
            Ok(vec![(1, 150_000), (2, 37_500)])
        );
        let d = Defines {
            base_counter: 10,
            stacking: 40_000,
            foreign_agent: 50_000,
            ..Default::default()
        };
        let row = RaidIntelInputRow {
            strategic_present: 1,
            operatives: operatives.to_vec(),
            ..Default::default()
        };
        assert_eq!(defense(&row, &d), Ok(180_010));
    }

    #[test]
    fn decryption_bonus_and_rounding_threshold_are_distinct() {
        let d = Defines {
            crypto: dashboard_text::cryptology::Defines {
                base: 200000,
                upgrade: 300000,
            },
            ..Default::default()
        };
        assert_eq!(decrypted(-1, 1, None, &d), Ok(true));
        assert_eq!(decrypted(799999, 0, Some(150000), &d), Ok(false));
        assert_eq!(decrypted(800000, 0, Some(150000), &d), Ok(true));
        assert_eq!(decrypted(500000, 0, Some(149999), &d), Ok(true));
        assert!(decrypted(500000, 0, None, &d).is_err());
    }
    #[test]
    fn six_terms_keep_fixed_division_order_and_subtract_counterintelligence() {
        let d = Defines {
            lanes: [1, 2, 3, 4],
            network: 99999,
            neighbour: 5,
            air: 99999,
            decryption: 7,
            counter: 99999,
            ..Default::default()
        };
        let terms = || Terms {
            lane: 100001,
            network: 2,
            neighbour: true,
            air: 50001,
            decrypted: true,
            counter: 2,
        };
        for lane in 0..4 {
            assert_eq!(total(terms(), lane, &d), 112 + lane as i64);
        }
    }
}
