//! Recorded raid target joins, shared by the dashboard and map hovers.
use replay_data::{rows::ProvinceRow, rows::RaidScope, rows::StateRow, rows::tables};
use replay_frame::frame::DecodedFrame;

type Result<T> = std::result::Result<T, String>;

fn province(frame: &DecodedFrame, id: u32) -> Result<&ProvinceRow> {
    let rows = frame
        .complete::<tables::Provinces>()
        .map_err(|error| error.to_string())?;
    rows.binary_search_by_key(&id, |row| row.id)
        .map(|at| &rows[at])
        .map_err(|_| format!("raid target province {id} is absent"))
}

fn state_row(frame: &DecodedFrame, id: u32) -> Result<&StateRow> {
    let rows = frame
        .complete::<tables::States>()
        .map_err(|error| error.to_string())?;
    rows.binary_search_by_key(&id, |row| row.id)
        .map(|at| &rows[at])
        .map_err(|_| format!("raid target state {id} is absent"))
}

/// Target country priority: building, province, state, resolved captive
/// leader's captor, then leader province. A resolved captor's raw word
/// suppresses the leader-province fallback.
pub fn country(frame: &DecodedFrame, scope: &RaidScope) -> Result<Option<u32>> {
    let raw = if scope.building_type != 0 {
        if scope.building_country > 0 {
            scope.building_country as u32
        } else if scope.building_flags & 1 != 0 {
            province(frame, scope.building_location)?.controller
        } else {
            state_row(frame, scope.building_location)?.controller
        }
    } else if scope.province != 0 {
        province(frame, scope.province)?.controller
    } else if scope.state != 0 {
        state_row(frame, scope.state)?.controller
    } else if let Some(captor) = scope.leader_country {
        // Copy the captor's raw country word unchanged; no signed-positive
        // test or leader-province retry follows that read.
        captor as u32
    } else if scope.leader_province != 0 {
        province(frame, scope.leader_province)?.controller
    } else {
        0
    };
    Ok((raw != 0).then_some(raw))
}

/// Plain location priority: explicit state, province, leader province,
/// then building state. A provincial building resolves through its province.
pub fn state(frame: &DecodedFrame, scope: &RaidScope) -> Result<Option<u32>> {
    if scope.state != 0 {
        return Ok(Some(scope.state));
    }
    let id = if scope.province != 0 {
        province(frame, scope.province)?.state
    } else if scope.leader_province != 0 {
        province(frame, scope.leader_province)?.state
    } else if scope.building_type == 0 {
        0
    } else if scope.building_flags & 1 != 0 {
        province(frame, scope.building_location)?.state
    } else if scope.building_location as i32 <= 0 {
        // A state building rejects a nonpositive signed location.
        0
    } else {
        scope.building_location
    };
    // Province and leader locations require a state pointer, as does a
    // provincial building's province. The recorder retains an observed null
    // as state 0; an unreadable pointer is a table gap.
    Ok((id != 0).then_some(id))
}

fn owner_tag(frame: &DecodedFrame, owner: u32) -> Result<&str> {
    // Owner zero still resolves reserved country slot zero, whose
    // original or own tag supplies the qualified localisation key.
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

/// Victory-point localisation tests key existence, not text length:
/// an existing empty qualified name suppresses the bare-key retry.
/// Plain location then falls back to the state's display name.
/// The callback keeps text controls until after this emptiness test.
pub fn victory_point(
    frame: &DecodedFrame,
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

/// Plain target location, without a province rename override. Consumers
/// resolve localisation through their recorded content stack and supply
/// the shared stored-state-name rule in their display representation.
pub fn location(
    frame: &DecodedFrame,
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
        // A null state supplies the literal `no_target_name` without localisation.
        None => Ok("no_target_name".into()),
    }
}

#[cfg(test)]
mod tests;
