//! Controlled-state prediction at the recruitment operand's supplying hour.
//! All work is playback-only; no additional native state census is needed.
use super::*;
use crate::products::country_people::{RecruitmentDefines, state_recruitment};
use replay_data::rows::{FactionAccountRow, Recorded};
use std::collections::BTreeMap;

/// What a member's lifetime, account and upgrade checks read at its supplying hour.
const SUPPLY_TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::FactionAccounts,
    TableId::LexicalTokens,
    TableId::SimulationFeatures,
];
/// Every table a supplying hour's checks and predictions read.
pub(super) fn native_tables() -> impl Iterator<Item = TableId> {
    SUPPLY_TABLES.iter().chain(TABLES).copied()
}
/// What a prediction reads there, only for a member it is reached for.
const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::FactionMembers,
    TableId::FactionAccounts,
    TableId::CountryCounterInputs,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::States,
    TableId::StateResourceInputs,
    TableId::StateCores,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
    TableId::SimulationFeatures,
];
fn need<T>(value: Option<T>, fact: impl Into<String>) -> Result<T, String> {
    value.ok_or_else(|| format!("{} unavailable", fact.into()))
}
fn scalar<T: Copy>(value: &Recorded<Option<T>>, fact: &str) -> Result<T, String> {
    need(
        value
            .require(fact)
            .map_err(|why| why.to_string())?
            .as_ref()
            .copied(),
        fact,
    )
}
fn account(frame: &DecodedFrame, slot: u32) -> Result<&FactionAccountRow, String> {
    need(
        frame
            .complete::<tables::FactionAccounts>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.kind == 0 && row.account == slot),
        format!("country slot {slot} faction account"),
    )
}
fn scaler(account: &FactionAccountRow) -> Result<i64, String> {
    Ok(need(
        account.contribution_subject_gain,
        "faction_subject_contribution_gain",
    )?
    .wrapping_add(100_000)
    .max(0))
}
fn supplying_hour(frame: &DecodedFrame, slot: u32) -> Result<i32, String> {
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let row = need(
        frame
            .complete::<tables::CountryStabilityInputs>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| crate::metadata::link_slot(links, row.country) == Some(slot)),
        "controller recruitment operand",
    )?;
    let hour = need(
        replay_data::sliced::read_hour(
            frame
                .complete::<tables::SliceReads>()
                .map_err(|why| why.to_string())?,
            TableId::CountryStabilityInputs,
            row.country,
        ),
        "controller recruitment supplying hour",
    )?;
    if hour > frame.hour || frame.hour.saturating_sub(hour) >= replay_data::cadence::STAGGER_PERIOD
    {
        return Err("controller recruitment supplying hour outside its daily window".into());
    }
    Ok(hour)
}
fn upgrade(
    frame: &DecodedFrame,
    account: &FactionAccountRow,
    cat: Option<&catalogue::Catalogue>,
) -> Result<i64, String> {
    let features = need(
        frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|why| why.to_string())?
            .first(),
        "runtime faction feature mask",
    )?;
    if features.dlc_mask & engine_dlc::dlc_feature_state::DEEPER_FACTIONS_MASK == 0 {
        return Ok(0);
    }
    for (&group, &token) in account.upgrade_groups.iter().zip(&account.upgrade_tokens) {
        let group = need(
            faction_names::template_spelling(frame, Some(group))?,
            "selected upgrade type spelling",
        )?;
        if group == "faction_member_upgrade_manpower" {
            let name = need(
                faction_names::template_spelling(frame, Some(token))?,
                "selected manpower upgrade spelling",
            )?;
            let cat = need(cat, "selected manpower upgrade content")?;
            return Ok(
                installed_content_core::faction_upgrades::cached(&cat.content.layers)?
                    .upgrade(name.as_bytes())?
                    .bonus_raw,
            );
        }
    }
    Ok(0)
}
#[derive(Clone, Copy)]
struct Defines {
    recruitment: RecruitmentDefines,
    yearly_growth: i64,
    contribution: i64,
}
fn defines(cat: Option<&catalogue::Catalogue>, shared: &SharedDefines) -> Result<Defines, String> {
    let cat = need(cat, "manpower prediction recorded content")?;
    let values = shared.get(cat)?;
    Ok(Defines {
        recruitment: RecruitmentDefines {
            speed: 0, // This prediction queries the held applied ratio, not mobilisation speed.
            noncore_base: values
                .fixed5("NCountry", "LOCAL_MANPOWER_ACCESSIBLE_NON_CORE_FACTOR")
                .map_err(|why| why.to_string())?,
            noncore_max: values
                .fixed5("NCountry", "MAX_NON_CORE_MANPOWER_FACTOR")
                .map_err(|why| why.to_string())?,
        },
        yearly_growth: values
            .fixed5("NCountry", "POPULATION_YEARLY_GROWTH_BASE")
            .map_err(|why| why.to_string())?,
        contribution: values
            .fixed5("NFactions", "FACTION_MANPOWER_GIVE_CONTRIBUTION_SCALAR")
            .map_err(|why| why.to_string())?,
    })
}
fn monthly_growth(base: i64, modifier: i64) -> i64 {
    let base = if crate::products::finite_fixed64(base) {
        base.wrapping_mul(100_000) / 1_200_000
    } else {
        u32::MAX as i64
    };
    base.wrapping_mul(modifier.wrapping_add(100_000)) / 100_000
}
/// Distribution deliberately retains both multiplications before division;
/// reducing the fraction first changes native overflow behavior.
fn distributed(population: i32, fraction: i64) -> i32 {
    (i64::from(population)
        .wrapping_mul(fraction)
        .wrapping_mul(100_000)
        / 10_000_000_000) as i32
}
fn transfer(available: &mut i32, population: i32, fraction: i64, requested: i32) -> i32 {
    let cap = (i64::from(population).wrapping_mul(fraction) / 100_000) as i32;
    let amount = requested.min(cap.wrapping_sub(*available).max(0)).max(0);
    *available = available.wrapping_add(amount);
    amount
}
/// Predict on copied pools. At least one person enters each distribution arm,
/// even with zero or negative growth; recruitment reconciliation follows it.
fn contributors(
    available: i32,
    locked: i32,
    population: i32,
    fraction: i64,
    growth: i64,
    bonus: i64,
) -> i32 {
    if bonus <= 0 {
        return 0;
    }
    let product = i64::from(population).wrapping_mul(growth);
    let rounded = product.wrapping_add(if product < 0 { -50_000 } else { 50_000 }) / 100_000;
    let next_population = population.wrapping_add(rounded as i32);
    let delta = next_population.wrapping_sub(population);
    let recruit = distributed(delta, fraction).max(1);
    let mut next_available = available.wrapping_add(recruit).max(0);
    let mut next_locked = locked
        .wrapping_add(delta.wrapping_sub(recruit).max(1))
        .max(0);
    let next_population = next_population.max(0);
    let cap = distributed(next_population, fraction);
    let excess = next_available.wrapping_sub(cap);
    if excess > 0 {
        let request = excess.min(cap.wrapping_sub(next_available).max(0)).max(0);
        let added = transfer(&mut next_available, next_population, fraction, request);
        let room = next_population
            .wrapping_sub(cap)
            .wrapping_sub(next_locked)
            .max(0);
        let amount = excess.wrapping_sub(added).min(room).max(0);
        next_locked = next_locked.wrapping_add(amount);
        next_available = next_available.wrapping_sub(amount.wrapping_add(added));
    }
    let excess = cap.wrapping_sub(next_population).wrapping_add(next_locked);
    if excess > 0 {
        // The returned transfer also changes the available pool.
        let _ = transfer(&mut next_available, next_population, fraction, excess);
    }
    let product = i64::from(next_available.wrapping_sub(available)).wrapping_mul(bonus);
    // Keep the floor adjustment's multiplication: extreme mod operands can
    // wrap here, so reducing this to a quotient-minus-one is not equivalent.
    let adjusted = if product % 100_000 != 0 {
        (product / 100_000)
            .wrapping_add(product >> 63)
            .wrapping_mul(100_000)
    } else {
        product
    };
    (adjusted / 100_000) as i32
}
fn expected(count: i32, coefficient: i64, scaler: i64) -> i64 {
    (i64::from(count)
        .wrapping_mul(coefficient)
        .wrapping_mul(100_000)
        / 100_000)
        .wrapping_mul(scaler)
        / 100_000
}
fn predict(frame: &DecodedFrame, slot: u32, bonus: i64, values: Defines) -> Result<i64, String> {
    let account = account(frame, slot)?;
    let scaler = scaler(account)?;
    if scaler == 0 || bonus <= 0 {
        return Ok(0);
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let same = |raw| crate::metadata::link_slot(links, raw) == Some(slot);
    let counters = need(
        frame
            .complete::<tables::CountryCounterInputs>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| same(row.country)),
        "controlled-state roster owner",
    )?;
    let controlled = need(
        counters
            .controlled_states
            .require("controlled states")
            .map_err(|why| why.to_string())?
            .as_deref(),
        "controlled-state roster",
    )?;
    if controlled.is_empty() {
        return Ok(0);
    }
    let growth = monthly_growth(
        values.yearly_growth,
        need(
            account.population_growth,
            "population_growth accumulator operand",
        )?,
    );
    if supplying_hour(frame, slot)? != frame.hour {
        return Err("controller recruitment does not supply this prediction hour".into());
    }
    let input = need(
        frame
            .complete::<tables::CountryStabilityInputs>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| same(row.country)),
        "controller recruitment operand",
    )?;
    let ratio = scalar(&input.recruitment_ratio, "controller recruitment ratio")?;
    // The country noncore key is reached only for a noncore controlled state.
    let mut noncore = None;
    let states = frame
        .complete::<tables::States>()
        .map_err(|why| why.to_string())?;
    let recruitment = frame
        .complete::<tables::StateResourceInputs>()
        .map_err(|why| why.to_string())?;
    let cores = frame
        .complete::<tables::StateCores>()
        .map_err(|why| why.to_string())?;
    let mut total = 0_i32;
    for id in controlled {
        let state = need(
            states.iter().find(|row| row.id == *id),
            format!("controlled state {id}"),
        )?;
        if !same(state.controller) {
            return Err(format!(
                "controlled state {id} controller disagrees with supplying roster"
            ));
        }
        let values_for_state = need(
            recruitment.iter().find(|row| row.state == *id),
            format!("state {id} recruitment operands"),
        )?;
        let core = need(
            cores.iter().find(|row| row.state == *id),
            format!("state {id} core roster"),
        )?
        .cores
        .iter()
        .any(|&raw| same(raw));
        let noncore = if core {
            0
        } else {
            *noncore
                .get_or_insert_with(|| {
                    let modifiers = need(
                        counters
                            .recruitment_modifiers
                            .require("controller recruitment modifiers")
                            .map_err(|why| why.to_string())?
                            .as_ref(),
                        "controller recruitment modifiers",
                    )?;
                    match modifiers[3] {
                        None => Ok(0),
                        Some(raw) => {
                            let registry =
                                replay_derive::modifier_registry::Registry::from_frame(frame)
                                    .map_err(|why| why.to_string())?;
                            let definition = registry
                                .named("non_core_manpower")
                                .map_err(|why| why.to_string())?;
                            registry
                                .effective(definition, raw)
                                .map_err(|why| why.to_string())
                        }
                    }
                })
                .as_ref()
                .map_err(Clone::clone)?
        };
        let fraction = state_recruitment(
            ratio,
            [
                scalar(
                    &values_for_state.local_manpower,
                    &format!("state {id} local_manpower"),
                )?,
                if core {
                    0
                } else {
                    scalar(
                        &values_for_state.local_noncore_manpower,
                        &format!("state {id} local_noncore_manpower"),
                    )?
                },
                scalar(
                    &values_for_state.recruitment_add,
                    &format!("state {id} recruitment_add"),
                )?,
                scalar(
                    &values_for_state.recruitment_factor,
                    &format!("state {id} recruitment_factor"),
                )?,
            ],
            core,
            noncore,
            values.recruitment,
        );
        total = total.wrapping_add(contributors(
            state.manpower_available,
            *state
                .manpower_locked
                .require("state locked manpower")
                .map_err(|why| format!("state {id}: {why}"))?,
            *state
                .population
                .require("state population")
                .map_err(|why| format!("state {id}: {why}"))?,
            fraction,
            growth,
            bonus,
        ));
    }
    Ok(expected(total, values.contribution, scaler))
}
pub(super) fn attach(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
    current: &DecodedFrame,
    faction: u64,
    cat: Option<&catalogue::Catalogue>,
    shared: &SharedDefines,
    reply: &mut FactionDetailReply,
) -> io::Result<()> {
    let mut detail = FactionRewardDetail {
        kind: "Expected manpower contribution".into(),
        text: String::new(),
        reasons: Vec::new(),
    };
    let mut frames = BTreeMap::new();
    let mut predicting = BTreeMap::new();
    let result = (|| -> Result<(), String> {
        let members = current
            .complete::<tables::FactionMembers>()
            .map_err(|why| why.to_string())?;
        let links = current
            .complete::<tables::CountryLinks>()
            .map_err(|why| why.to_string())?;
        for member in members.iter().filter(|row| row.faction == faction) {
            let result = (|| -> Result<(i64, i32, Option<i64>), String> {
                let slot = need(
                    crate::metadata::link_slot(links, member.country),
                    "member canonical country",
                )?;
                let row = account(current, slot)?;
                if scaler(row)? == 0 {
                    return Ok((0, current.hour, row.population_growth));
                }
                let hour = supplying_hour(current, slot)?;
                if source.replay_ref().gaps.iter().any(|gap| {
                    gap.branch == branch && gap.first_hour <= current.hour && gap.last_hour >= hour
                }) {
                    return Err("recruitment supplying hour crosses a recording gap".into());
                }
                if let std::collections::btree_map::Entry::Vacant(entry) = frames.entry(hour) {
                    entry.insert(
                        crate::prepared::query::native::frame_at(
                            source,
                            branch,
                            hour,
                            SUPPLY_TABLES,
                            cancelled,
                        )
                        .map_err(|why| why.to_string())?,
                    );
                }
                let frame = need(
                    frames.get(&hour).and_then(Option::as_deref),
                    format!("manpower supplying frame at hour {hour}"),
                )?;
                let old_country = need(
                    frame
                        .complete::<tables::Countries>()
                        .map_err(|why| why.to_string())?
                        .iter()
                        .find(|row| row.canonical_slot == slot),
                    "supplying country lifetime",
                )?;
                let country = need(
                    current
                        .complete::<tables::Countries>()
                        .map_err(|why| why.to_string())?
                        .iter()
                        .find(|row| row.canonical_slot == slot),
                    "current country lifetime",
                )?;
                if old_country.id != country.id {
                    return Err("country lifetime changed since recruitment supplying hour".into());
                }
                let row = account(frame, slot)?;
                let bonus = upgrade(frame, row, cat)?;
                let value = if scaler(row)? == 0 || bonus <= 0 {
                    0
                } else {
                    let values = defines(cat, shared)?;
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        predicting.entry(hour)
                    {
                        entry.insert(
                            crate::prepared::query::native::frame_at(
                                source, branch, hour, TABLES, cancelled,
                            )
                            .map_err(|why| why.to_string())?,
                        );
                    }
                    let frame = need(
                        predicting.get(&hour).and_then(Option::as_deref),
                        format!("manpower supplying frame at hour {hour}"),
                    )?;
                    predict(frame, slot, bonus, values)?
                };
                Ok((value, hour, row.population_growth))
            })();
            match result {
                Ok((value, hour, growth)) => {
                    if !detail.text.is_empty() {
                        detail.text.push('\n');
                    }
                    detail.text.push_str(&format!("Country {}: expected manpower contribution {value} (fixed5), supplying hour {hour}; population growth modifier {} (fixed5)", member.country, growth.map_or_else(|| "unavailable (not reached)".into(), |value| value.to_string())));
                }
                Err(why) => {
                    let why = format!("Country {}: {why}", member.country);
                    detail.reasons.push(why.clone());
                    reply.refusals.push(FactionRefusalDetail {
                        fact: "contribution_manpower_expected_gain".into(),
                        count: 1,
                        reason: why,
                    });
                }
            }
        }
        Ok(())
    })();
    if let Err(why) = result {
        detail.reasons.push(why.clone());
        reply.refusals.push(FactionRefusalDetail {
            fact: "contribution_manpower_expected_gain".into(),
            count: 1,
            reason: why,
        });
    }
    reply.presentations.push(detail);
    Ok(())
}

#[cfg(test)]
mod tests;
