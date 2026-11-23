use engine_native::{Memory, ccountry, cprovince, cstate, walk::Outcome};
use engine_research::ctechnology_status;

pub(super) fn needed(flags: &[cstate::FlagRow]) -> bool {
    flags.iter().any(|row| row.border_conflict_active != 0)
}

fn complete<T>(rows: &[T], outcome: Outcome) -> Result<&[T], &'static str> {
    match outcome {
        Outcome::Rows { len } => rows.get(..len).ok_or("border demand census exceeds buffer"),
        Outcome::Unavailable { reason } => Err(reason),
        Outcome::Truncated { .. } => Err("border demand census incomplete"),
    }
}

pub(super) fn provinces<'a>(
    provinces: &'a [cprovince::Row],
    outcome: Outcome,
    flags: &[cstate::FlagRow],
) -> Result<&'a [cprovince::Row], &'static str> {
    let provinces = complete(provinces, outcome)?;
    if provinces.windows(2).any(|p| p[0].id >= p[1].id) {
        return Err("border demand province identities unordered or duplicate");
    }
    for province in provinces {
        if province.state != 0
            && flags
                .binary_search_by_key(&province.state, |flag| flag.state)
                .is_err()
        {
            return Err("border demand province state absent from census");
        }
    }
    Ok(provinces)
}

pub(super) fn land(land: &[(u32, u64)], provinces: &[cprovince::Row]) -> Result<(), &'static str> {
    for &(province, _) in land {
        if provinces
            .binary_search_by_key(&province, |row| row.id)
            .is_err()
        {
            return Err("border demand army province absent from census");
        }
    }
    Ok(())
}

pub(super) fn active(
    provinces: &[cprovince::Row],
    flags: &[cstate::FlagRow],
    province: u32,
) -> bool {
    let Ok(index) = provinces.binary_search_by_key(&province, |row| row.id) else {
        return false;
    };
    let state = provinces[index].state;
    flags
        .binary_search_by_key(&state, |row| row.state)
        .is_ok_and(|index| flags[index].border_conflict_active != 0)
}

pub(super) fn doctrine(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    provinces: &[cprovince::Row],
    flags: &[cstate::FlagRow],
    rows: &mut [ccountry::Row],
    outcome: Outcome,
) -> Result<(), &'static str> {
    let len = match outcome {
        Outcome::Rows { len } => len,
        Outcome::Unavailable { reason } => return Err(reason),
        Outcome::Truncated { .. } => return Err("border demand countries truncated"),
    };
    let rows = rows
        .get_mut(..len)
        .ok_or("border demand country count exceeds buffer")?;
    if rows
        .windows(2)
        .any(|rows| rows[0].canonical_slot >= rows[1].canonical_slot)
    {
        return Err("border demand country census unordered or duplicate");
    }
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    for province in provinces {
        if !flags
            .binary_search_by_key(&province.state, |flag| flag.state)
            .is_ok_and(|index| flags[index].border_conflict_active != 0)
        {
            continue;
        }
        let country = countries.resolve(province.controller)?;
        let index = rows
            .binary_search_by_key(&country.canonical_slot, |row| row.canonical_slot)
            .map_err(|_| "border controller absent from country census")?;
        let row = &mut rows[index];
        if row.id != country.id {
            return Err("border controller country identity changed");
        }
        if row.border_land_doctrine_level.is_none() {
            row.border_land_doctrine_level = Some(ctechnology_status::land_doctrine_level(
                memory,
                base,
                country.pointer,
            )?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
