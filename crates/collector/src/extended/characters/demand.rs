//! Capacity observations supply hints only, never captured values.

use super::{
    Capacities, Memory, ccharacter, ccharacter_manager, cunit_leader, read, unit_identity,
};
use crate::country_scale::{DemandFailures, raise, rows};

static FAILURES: [DemandFailures; 7] = [const { DemandFailures::new() }; 7];
static LIVE_ROLES: [std::sync::atomic::AtomicUsize; 4] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 4];
static LIVE_TRAITS: [[std::sync::atomic::AtomicUsize; 2]; 2] =
    [const { [const { std::sync::atomic::AtomicUsize::new(0) }; 2] }; 2];

pub(super) fn initial_roles(part: usize) -> usize {
    LIVE_ROLES[part].load(std::sync::atomic::Ordering::Relaxed)
}

pub(super) fn initial_traits() -> [cunit_leader::TraitDemand; 2] {
    LIVE_TRAITS
        .each_ref()
        .map(|hint| cunit_leader::TraitDemand {
            rows: hint[0].load(std::sync::atomic::Ordering::Relaxed),
            bytes: hint[1].load(std::sync::atomic::Ordering::Relaxed),
        })
}

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::map(
        [
            "roster",
            "unit_leaders",
            "unit_leader_traits",
            "unit_leader_trait_experience",
            "country_leader_roles",
            "unit_leader_text",
            "advisor_definitions",
        ]
        .into_iter()
        .zip(FAILURES.iter().map(DemandFailures::diagnostics)),
    )
}

pub(super) fn presize(
    capacity: &mut Capacities,
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(), &'static str> {
    // 1406AE170 serializes both native character vectors. Every occurrence
    // keeps its own leader; duplicate pointers are demand, not deduplication.
    let roster = FAILURES[0].observe(ccharacter_manager::Roster::read(memory, base, state))?;
    super::census::remember_roster(roster.count() as usize);
    if let Ok(demand) =
        FAILURES[6].observe(engine::cadvisor::definitions_demand(memory, base, state))
    {
        let total_rows = demand[0].rows.saturating_add(demand[1].rows);
        let total_bytes = demand[0].bytes.saturating_add(demand[1].bytes);
        raise(&mut capacity.rows[2], rows(total_rows, 1));
        raise(&mut capacity.bytes[2], rows(total_bytes, 1));
        LIVE_ROLES[2].fetch_max(total_rows, std::sync::atomic::Ordering::Relaxed);
        super::advisors::remember_definition_demand(demand);
    }
    let leaders = engine::ccountry_leader::walk(memory, base, state, &mut []);
    let leaders = match leaders {
        engine::walk::Outcome::Rows { len } => Ok(len),
        engine::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("country leader demand overflow"),
        engine::walk::Outcome::Unavailable { reason } => Err(reason),
    };
    if let Ok(leaders) = FAILURES[4].observe(leaders) {
        raise(&mut capacity.rows[1], rows(leaders, 1));
        LIVE_ROLES[1].fetch_max(leaders, std::sync::atomic::Ordering::Relaxed);
    }
    let mut units = 0_usize;
    let mut unit_bytes = 0_usize;
    let mut traits = [cunit_leader::TraitDemand::default(); 2];
    let mut refusal = None;
    for index in 0..roster.count() {
        let unit = (|| {
            let view = roster.view(memory, index, [false, false, true, false])?;
            let (character, id) = view.reference();
            let pointer = character
                .checked_add(ccharacter::UNIT_LEADER)
                .and_then(|address| read::<u64>(&view, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("character unit leader pointer unreadable")?;
            if pointer == 0 {
                return Ok(None);
            }
            Ok(Some((pointer, unit_identity(memory, pointer, id)?)))
        })();
        let (pointer, id) = match FAILURES[1].observe(unit) {
            Ok(Some(unit)) => unit,
            Ok(None) => continue,
            Err(reason) => {
                refusal.get_or_insert(reason);
                continue;
            }
        };
        units = units
            .checked_add(1)
            .ok_or("unit leader sizing count overflow")?;
        let text = (|| {
            let demand = cunit_leader::read_text(memory, pointer, &mut []);
            if let engine::walk::Outcome::Unavailable { reason } = demand.outcome {
                return Err(reason);
            }
            unit_bytes = unit_bytes
                .checked_add(demand.required_bytes)
                .ok_or("unit leader text demand overflow")?;
            Ok(())
        })();
        if let Err(reason) = FAILURES[5].observe(text) {
            refusal.get_or_insert(reason);
        }
        for (index, total) in traits.iter_mut().enumerate() {
            let demand = cunit_leader::trait_demand(memory, base, pointer, id, index == 1)
                .and_then(|next| {
                    let next = cunit_leader::TraitDemand {
                        rows: total
                            .rows
                            .checked_add(next.rows)
                            .ok_or("unit leader trait sizing row count overflow")?,
                        bytes: total
                            .bytes
                            .checked_add(next.bytes)
                            .ok_or("unit leader trait sizing byte count overflow")?,
                    };
                    *total = next;
                    Ok(())
                });
            if let Err(reason) = FAILURES[index + 2].observe(demand) {
                refusal.get_or_insert(reason);
            }
        }
    }
    raise(&mut capacity.rows[3], rows(units, 1));
    LIVE_ROLES[0].fetch_max(units, std::sync::atomic::Ordering::Relaxed);
    raise(&mut capacity.bytes[3], rows(unit_bytes, 1));
    for (index, demand) in traits.into_iter().enumerate() {
        LIVE_TRAITS[index][0].fetch_max(demand.rows, std::sync::atomic::Ordering::Relaxed);
        LIVE_TRAITS[index][1].fetch_max(demand.bytes, std::sync::atomic::Ordering::Relaxed);
        raise(&mut capacity.rows[index + 4], rows(demand.rows, 1));
        raise(&mut capacity.bytes[index + 4], rows(demand.bytes, 1));
    }
    refusal.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests;
