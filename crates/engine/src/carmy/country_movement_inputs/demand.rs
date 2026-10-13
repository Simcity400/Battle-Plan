//! Off-thread sizing hints from live headers and narrow movement rosters.
//!
//! These observations only prepare arenas. The acquisition callback still
//! authenticates its own complete domains and reports their actual demand.
//! No route payload, speed getter, map entity or client timing state is read.

use crate::{
    Memory, ccountry, cgame_state, cpdx_array, cpdx_array::Array, crailway_gun,
    creference_registry::HeaderResolver, cunit, read,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MovingOwners {
    pub logical: usize,
}

/// Each header floor survives an unrelated header or mover-census refusal.
/// A failed mover census never publishes a partial count as an empty census.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveDemand {
    pub country_slots: Result<usize, &'static str>,
    pub raw_links: Result<usize, &'static str>,
    pub region_bound: Result<usize, &'static str>,
    pub movers: Result<MovingOwners, &'static str>,
}

impl Default for LiveDemand {
    fn default() -> Self {
        Self {
            country_slots: Err("country movement country slots not observed"),
            raw_links: Err("country movement raw links not observed"),
            region_bound: Err("country movement region bound not observed"),
            movers: Err("country movement moving owners not observed"),
        }
    }
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .filter(|_| object != 0)
        .and_then(|at| Array::read(memory, at, stride))
        .ok_or("country movement demand array unreadable or invalid")
}

fn owner(memory: &impl Memory, unit: usize, offset: usize) -> Result<u32, &'static str> {
    unit.checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("country movement demand owner unreadable")
}

fn division_identity(memory: &impl Memory, base: usize, unit: usize) -> Result<(), &'static str> {
    // Match the division ETA source gate: its primary CArmy identity and
    // nonzero stored handle suffice. The secondary vtable is not an ETA gate.
    let vtable = read::<u64>(memory, unit)
        .filter(|_| unit != 0)
        .ok_or("country movement demand division identity unreadable")?;
    let class = owner(memory, unit, cunit::CLASS)?;
    let handle = unit
        .checked_add(cunit::HANDLE)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("country movement demand division handle unreadable")?;
    let expected = base
        .checked_add(crate::carmy::VTABLE_RVA)
        .ok_or("country movement demand module overflow")?;
    if vtable != expected as u64 || class != 0 || handle == 0 {
        return Err("country movement demand division identity invalid");
    }
    Ok(())
}

fn moving_owners<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    headers: [Array; 2],
) -> Result<MovingOwners, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    if countries.arrays() != headers {
        return Err("country movement demand country headers changed");
    }
    // One live-sized byte per canonical slot counts logical owners. This
    // allocation happens only in the off-thread observer, never the callback.
    // An advisory hint must not trigger the recorder's terminal OOM hook.
    let mut owners = Vec::new();
    owners
        .try_reserve_exact(countries.count() as usize)
        .map_err(|_| "country movement demand owner scratch allocation failed")?;
    owners.resize(countries.count() as usize, 0_u8);
    let mut registry = HeaderResolver::default();
    let mut found_country = false;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found_country = true;
        for (roster, division) in [
            (ccountry::DIVISIONS, true),
            (ccountry::RAILWAY_UNITS, false),
        ] {
            let units = array(memory, country.pointer, roster, 8)?;
            cpdx_array::pointers(
                memory,
                units,
                "country movement demand unit pointer unreadable",
                |_, unit| {
                    if division {
                        // The existing CArmy census skips nullable division slots.
                        if unit == 0 {
                            return Ok(());
                        }
                        division_identity(memory, base, unit)?;
                    } else {
                        // The railway census refuses a null counted gun slot.
                        if unit == 0 {
                            return Err("country movement demand railway roster has null slot");
                        }
                        let (_, class) =
                            cunit::counter_identity(memory, base, unit, &mut registry)?;
                        let expected = base
                            .checked_add(crailway_gun::SECONDARY_VTABLE_RVA)
                            .ok_or("country movement demand module overflow")?;
                        if class != 13
                            || unit
                                .checked_add(crate::carmy::SECONDARY_BASE)
                                .and_then(|at| read::<u64>(memory, at))
                                != Some(expected as u64)
                        {
                            return Err("country movement demand railway identity mismatch");
                        }
                    }
                    let roster_owner = owner(memory, unit, cunit::ROSTER_OWNER)?;
                    if countries.resolve(roster_owner)?.canonical_slot != country.canonical_slot {
                        return Err("country movement demand unit roster owner mismatch");
                    }
                    // Only the ordinary header decides demand. First edges, route
                    // contents, flags and terrain do not suppress this sizing floor.
                    array(memory, unit, cunit::ORDINARY_ROUTE_DATA, 4)?;
                    let logical = countries.resolve(owner(memory, unit, cunit::LOGICAL_OWNER)?)?;
                    owners[logical.canonical_slot as usize] = 1;
                    Ok(())
                },
            )?;
        }
    }
    if !found_country {
        return Err("country movement demand has no authenticated countries");
    }
    Ok(MovingOwners {
        logical: owners.iter().filter(|&&flag| flag != 0).count(),
    })
}

/// Observe complete canonical division and railway rosters without acquiring
/// their simulation rows. Shared movement coefficients are sized from distinct
/// demanded owners, rather than an all-country coefficient matrix.
pub fn observe_live(memory: &impl Memory, base: usize, state: usize) -> LiveDemand {
    let country_array = array(memory, state, cgame_state::COUNTRIES, 8);
    let links = array(memory, state, cgame_state::COUNTRY_LINKS, 4);
    let region_bound = super::region_demand(memory, state);
    let movers = match (country_array, links) {
        (Ok(countries), Ok(links)) => moving_owners(memory, base, state, [countries, links]),
        (Err(reason), _) | (_, Err(reason)) => Err(reason),
    };
    LiveDemand {
        country_slots: country_array.map(|array| array.count as usize),
        raw_links: links.map(|array| array.count as usize),
        region_bound,
        movers,
    }
}

#[cfg(test)]
mod tests;
