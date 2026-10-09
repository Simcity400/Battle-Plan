//! The naval battle title and naval result name: 1415B06C0.
//!
//! With no port-strike result (RefreshCombatDetails 1417DA2F0 passes none; a
//! result passes itself and 1415B06C0 tests its +0x128 byte) the name is the
//! battle province's strategic region name, replaced by the localised name of
//! the best-scoring province of the region's states once that score reaches
//! `NNavy.BATTLE_NAME_VP_CUTOFF`, all through `NAVAL_BATTLE_TITLE` with the
//! text argument `NAME` (142727E7C). A port strike localises
//! `NAVAL_PORT_STRIKE_TITLE` with the region name alone.
use super::binding::{TextIcons, builtin, token};
use crate::{
    document::Document,
    table_access::{Available, rows},
};
use dashboard::naval_battle_name::Picker;
pub(crate) use dashboard::naval_battle_name::battle_name_candidates;
use installed_content::localisation::{Argument, Localisation};
use replay::{frame::DecodedFrame, rows::tables};
use std::{collections::BTreeMap, io};

/// The recorded province facts 140E69320's name ladder reads.
struct Province<'a> {
    id: u32,
    /// The recorded rename (CProvince+0x40, `ProvinceNames`): `Ok(None)` for
    /// none, or why the hour cannot say.
    rename: Result<Option<&'a [u8]>, String>,
    /// The tag 140E68E30 qualifies the victory point key with: the
    /// province's state owner's original tag or tag (CCountry::
    /// GetOriginalTagOrTag 1406DF830), or why it is unavailable.
    vp_tag: Result<&'a str, String>,
    /// The province's recorded state (CProvince+0xC0, `Provinces.state`):
    /// `None` for no state, else the state's recorded names and the name key
    /// of its definition, or why they are unavailable.
    state: Option<Result<StateNames<'a>, String>>,
    /// The key of the province's strategic region name (CProvince+0xC8),
    /// which the ladder takes for a province without a state.
    region_name_key: Option<&'a str>,
}

/// What CState::display_name (1409CC380) reads.
struct StateNames<'a> {
    id: u32,
    /// CState+0x38 (`States.custom_name`) and +0xE8 (`States.owner_name`).
    custom_name: &'a [u8],
    owner_name: &'a [u8],
    /// The recording client's language, in which the game stored those two.
    recorded_language: &'a str,
    /// The definition's `name` key (CStateTemplate+0x40), or why the
    /// definition's reading cannot be followed.
    name_key: Result<&'a str, &'a str>,
}

/// CState::display_name (1409CC380) through [`super::text::state_name`]:
/// the template's name is localised from the `name` key when CStateTemplate's
/// reader (140AAF580) reads the file (a key the localisation lacks is its own
/// text). 140E69320 copies the returned string.
fn state_display_name(
    icons: &dyn TextIcons,
    loc: &Localisation,
    state: &StateNames<'_>,
) -> Result<String, String> {
    let id = state.id;
    let (custom, owner) = (state.custom_name, state.owner_name);
    super::text::state_name(loc, state.recorded_language, id, custom, owner, || {
        let key = state
            .name_key
            .map_err(|reason| format!("state {id}'s definition name is unknown: {reason}"))?;
        token(icons, loc, key.as_bytes()).map_err(|reason| format!("state {id}'s name: {reason}"))
    })
}

/// 140E69320 for a province of a state: a nonempty recorded rename through
/// the localiser (a key the localisation lacks is its own text); else
/// ResolveVictoryPointLocalization (140E68E30), which localises the first of
/// `<TAG>_VICTORY_POINTS_<id>` and `VICTORY_POINTS_<id>` the localisation
/// holds, or gives nothing; a nonempty result is the name. Otherwise the
/// state's display name ([`state_display_name`]), else for a province
/// without a state its strategic region's name, else its id.
fn province_name(
    icons: &dyn TextIcons,
    loc: &Localisation,
    province: &Province<'_>,
) -> Result<String, String> {
    let id = province.id;
    let rename = province.rename.as_ref().map_err(|reason| {
        format!("province {id}'s recorded rename (CProvince+0x40) is unknown: {reason}")
    })?;
    if let Some(bytes) = rename.filter(|bytes| !bytes.is_empty()) {
        return token(icons, loc, bytes)
            .map_err(|reason| format!("province {id}'s recorded rename: {reason}"));
    }
    let tag = province.vp_tag.as_deref().map_err(|reason| {
        format!("province {id}'s qualified victory point key (140E68E30) needs a tag, but {reason}")
    })?;
    // 140E68E30 picks one key, and its localised text, empty included, is
    // the result; 140E69320 goes on only when that is empty.
    let chosen = [
        format!("{tag}_VICTORY_POINTS_{id}"),
        format!("VICTORY_POINTS_{id}"),
    ]
    .into_iter()
    .find(|key| loc.get(key).is_some());
    if let Some(key) = chosen {
        let text = token(icons, loc, key.as_bytes())?;
        if !text.is_empty() {
            return Ok(text);
        }
    }
    match &province.state {
        Some(Ok(state)) => state_display_name(icons, loc, state),
        Some(Err(reason)) => Err(format!(
            "province {id}'s state display name (CState::display_name 1409CC380): {reason}"
        )),
        None => match province.region_name_key {
            Some(key) => token(icons, loc, key.as_bytes())
                .map_err(|reason| format!("province {id}'s region name: {reason}")),
            None => Ok(id.to_string()),
        },
    }
}

/// The naval battle title for `province`, or for a result whose port-strike
/// byte (+0x128) is `port_strike`; the error names the missing operand.
pub(super) fn title(
    document: &Document,
    frame: &DecodedFrame,
    province: u32,
    port_strike: bool,
) -> io::Result<Result<String, String>> {
    let (icons, loc) = (&document.icons.combat, &document.names.localisation);
    let Some(region) = document.counters.region(province) else {
        return Ok(Err(format!(
            "province {province} is in no content strategic region"
        )));
    };
    let Some(definition) = document.strategic_regions.get(&region) else {
        return Ok(Err(format!(
            "strategic region {region} has no content definition"
        )));
    };
    // CStrategicRegion::GetName (140E0EB10): the template's name, localised
    // when the definition is read (1415938B0); the instance override is only
    // a nudger edit or a copy of that same name.
    let region_name = match token(icons, loc, definition.name_key.as_bytes()) {
        Ok(name) => name,
        Err(reason) => return Ok(Err(format!("strategic region {region}'s name: {reason}"))),
    };
    if port_strike {
        return Ok(builtin(
            icons,
            loc,
            "NAVAL_PORT_STRIKE_TITLE",
            &[("NAME", Argument::Text(&region_name))],
        ));
    }
    let [factor, cutoff] = match &document.icons.combat.naval_name_defines {
        Ok(defines) => *defines,
        Err(reason) => {
            return Ok(Err(format!(
                "the NNavy battle name defines are unavailable ({reason})"
            )));
        }
    };
    let provinces = rows::<tables::Provinces>(frame)?;
    let complete = provinces.is_complete();
    let provinces: BTreeMap<_, _> = provinces.rows().iter().map(|row| (row.id, row)).collect();
    let victory_points = |id: u32| {
        provinces.get(&id).map(|row| row.victory_points).ok_or_else(|| {
            if complete {
                format!("province {id} has no Provinces row")
            } else {
                format!("the Provinces table is incomplete this hour, so province {id}'s victory points are unknown")
            }
        })
    };
    let geometry = &document.province_geometry;
    // A province with no pixels keeps the zero position 1413F4FD0 leaves.
    let position = |id: u32| geometry.map_position(id).unwrap_or([0, 0]);
    let picker = Picker {
        width: document.map.width as i32,
        factor,
        cutoff,
        position: &position,
        victory_points: &victory_points,
    };
    let candidates = document
        .battle_name_provinces
        .get(&region)
        .map_or(&[][..], Vec::as_slice);
    let name = match picker.pick(province, candidates) {
        Err(reason) => return Ok(Err(reason)),
        Ok(None) => region_name,
        Ok(Some(picked)) => {
            let renames = rows::<tables::ProvinceNames>(frame)?;
            // The sparse table proves no rename only when complete; a row
            // present in a partial one is still the recorded rename.
            let rename = match renames
                .rows()
                .iter()
                .rev()
                .find(|row| row.province == picked)
            {
                Some(row) => Ok(Some(row.name.as_slice())),
                None => match &renames {
                    Available::Complete(_) => Ok(None),
                    Available::Truncated { missing, .. } => Err(format!(
                        "the ProvinceNames table is truncated this hour ({missing} rows not reached) and holds no row for it"
                    )),
                    Available::Unavailable { reason } => Err(format!(
                        "the ProvinceNames table is unavailable this hour ({reason})"
                    )),
                },
            };
            // The picked province's state from Provinces.state, its names
            // from the States row and its definition's `name` key.
            let states = rows::<tables::States>(frame)?;
            let states_complete = states.is_complete();
            let vp_tag = match provinces.get(&picked) {
                None => Err(format!("province {picked} has no Provinces row")),
                Some(row) => vp_tag(frame, row.state, states.rows(), states_complete)?,
            };
            let state = provinces.get(&picked).and_then(|row| {
                (row.state != 0).then(|| {
                    let id = row.state;
                    let recorded = states.rows().iter().find(|state| state.id == id);
                    let recorded = recorded.ok_or_else(|| {
                        if states_complete {
                            format!("state {id} has no States row")
                        } else {
                            format!("the States table is incomplete this hour, so state {id}'s names are unknown")
                        }
                    })?;
                    let name_key = document
                        .state_name_keys
                        .get(&id)
                        .ok_or_else(|| format!("state {id} has no content definition"))?;
                    Ok(StateNames {
                        id,
                        custom_name: &recorded.custom_name,
                        owner_name: &recorded.owner_name,
                        recorded_language: &document.recorded_language,
                        name_key: name_key.as_deref().map_err(String::as_str),
                    })
                })
            });
            let state = match provinces.get(&picked) {
                Some(_) => state,
                None => Some(Err(format!("province {picked} has no Provinces row"))),
            };
            let region_name_key = document
                .counters
                .region(picked)
                .and_then(|region| document.strategic_regions.get(&region))
                .map(|region| region.name_key.as_str());
            match province_name(
                icons,
                loc,
                &Province {
                    id: picked,
                    rename,
                    vp_tag: vp_tag.as_deref().map_err(Clone::clone),
                    state,
                    region_name_key,
                },
            ) {
                Ok(name) => name,
                Err(reason) => return Ok(Err(reason)),
            }
        }
    };
    Ok(builtin(
        icons,
        loc,
        "NAVAL_BATTLE_TITLE",
        &[("NAME", Argument::Text(&name))],
    ))
}

/// The tag 140E68E30 qualifies a province's victory point key with, through
/// CCountry::GetOriginalTagOrTag (1406DF830), which takes the original tag
/// (+0x130C) when it is positive. The independent original-only suffix
/// plus the country's own tag supplies the getter, without the name generator's dynamic gate.
/// The country is the province's state owner
/// (CState+0xC8), resolved by resolve_country_reference (140BA56A0): a null
/// link is country index 0, any other its canonical slot (140BA6240). A
/// province without a state takes ConstructCountryLinkFromCountryIndex(0)
/// (140BA4BB0): FindCountryLinkFromSlot (1401DA340) gives the first link to
/// index 0, or 0 itself, which both resolve to the country at index 0.
fn vp_tag(
    frame: &DecodedFrame,
    state: u32,
    states: &[replay::rows::StateRow],
    states_complete: bool,
) -> io::Result<Result<String, String>> {
    let (subject, slot) = if state == 0 {
        (
            "the country at index 0, which a province without a state qualifies with (140BA4BB0)"
                .to_owned(),
            0,
        )
    } else {
        let Some(row) = states.iter().find(|row| row.id == state) else {
            return Ok(Err(if states_complete {
                format!("state {state} has no States row")
            } else {
                format!(
                    "the States table is incomplete this hour, so state {state}'s owner is unknown"
                )
            }));
        };
        let owner = row.owner;
        let slot = if owner == 0 {
            0
        } else {
            let links = rows::<tables::CountryLinks>(frame)?;
            let Some(slot) = links
                .rows()
                .iter()
                .find(|link| link.id == owner)
                .map(|link| link.canonical_slot)
            else {
                return Ok(Err(if links.is_complete() {
                    format!("state {state}'s owner link {owner} has no CountryLinks row")
                } else {
                    format!(
                        "the CountryLinks table is incomplete this hour, so state {state}'s owner link {owner} is unresolved"
                    )
                }));
            };
            slot
        };
        (format!("state {state}'s owner (slot {slot})"), slot)
    };
    Ok(installed_content::country_names::original_or_own_tag(frame, slot)
        .map(str::to_owned)
        .map_err(|reason| format!("{subject}: {reason}")))
}

#[cfg(test)]
mod tests;
