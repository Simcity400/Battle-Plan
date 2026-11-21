#[cfg(test)]
use replay_frame::frame::DecodedFrame;

use replay_data::{rows::RaidScope, rows::StateRow, rows::tables};
use replay_frame::frame::DecodedSource;

type Result<T> = std::result::Result<T, String>;

fn province(frame: &dyn DecodedSource, id: u32) -> Result<replay_frame::frame::Province> {
    let rows = frame.provinces().map_err(|error| error.to_string())?;
    rows.binary_search_by_key(&id, |row| row.id)
        .map(|at| rows.get(at).unwrap())
        .map_err(|_| format!("raid target province {id} is absent"))
}

fn state_row(frame: &dyn DecodedSource, id: u32) -> Result<&StateRow> {
    let rows = frame
        .complete::<tables::States>()
        .map_err(|error| error.to_string())?;
    rows.binary_search_by_key(&id, |row| row.id)
        .map(|at| &rows[at])
        .map_err(|_| format!("raid target state {id} is absent"))
}

pub fn country(frame: &dyn DecodedSource, scope: &RaidScope) -> Result<Option<u32>> {
    scope.country_with(|id| province(frame, id).map(|row| row.controller), |id| state_row(frame, id).map(|row| row.controller))
}

pub fn state(frame: &dyn DecodedSource, scope: &RaidScope) -> Result<Option<u32>> {
    scope.state_with(|id| province(frame, id).map(|row| row.state))
}

fn owner_tag(frame: &dyn DecodedSource, owner: u32) -> Result<&str> {
    if owner == 0 {
        return crate::country_names::reserved_zero_original_tag(frame);
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|error| error.to_string())?;
    let slot = links
        .binary_search_by_key(&owner, |row| row.id)
        .ok()
        .map(|at| links[at].canonical_slot)
        .ok_or_else(|| format!("raid target state's owner {owner} is unresolved"))?;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|error| error.to_string())?;
    let mut holders = countries.iter().filter(|row| row.canonical_slot == slot);
    let country = holders
        .next()
        .ok_or_else(|| format!("raid target state owner slot {slot} has no country"))?;
    if holders.next().is_some() {
        return Err(format!(
            "raid target state owner slot {slot} has ambiguous countries"
        ));
    }
    let appearance = frame
        .complete::<tables::CountryAppearance>()
        .map_err(|error| error.to_string())?;
    let row = appearance
        .binary_search_by_key(&country.id, |row| row.country)
        .ok()
        .map(|at| &appearance[at])
        .ok_or_else(|| {
            format!(
                "raid target state owner {} has no original-tag read",
                country.id
            )
        })?;
    crate::country_names::original_tag(country, row)
        .map_err(|reason| format!("raid target state owner {owner}'s {reason}"))
}

pub fn victory_point(
    frame: &dyn DecodedSource,
    id: u32,
    mut localise: impl FnMut(&str) -> Result<Option<String>>,
) -> Result<Option<String>> {
    let state = province(frame, id)?.state;
    let owner = if state == 0 {
        0
    } else {
        state_row(frame, state)?.owner
    };
    let tag = owner_tag(frame, owner)?;
    for key in [
        format!("{tag}_VICTORY_POINTS_{id}"),
        format!("VICTORY_POINTS_{id}"),
    ] {
        if let Some(name) = localise(&key)? {
            return Ok((!name.is_empty()).then_some(name));
        }
    }
    Ok(None)
}

pub fn location(
    frame: &dyn DecodedSource,
    scope: &RaidScope,
    localise: impl FnMut(&str) -> Result<Option<String>>,
    state_name: impl FnOnce(u32) -> Result<String>,
) -> Result<String> {
    let province = if scope.province != 0 {
        scope.province
    } else {
        scope.leader_province
    };
    if province != 0
        && let Some(name) = victory_point(frame, province, localise)?
    {
        return Ok(name);
    }
    match state(frame, scope)? {
        Some(id) => state_name(id),
        None => Ok("no_target_name".into()),
    }
}

#[cfg(test)]
mod tests;
