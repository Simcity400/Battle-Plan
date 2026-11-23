use dashboard_text::naval_battle_name::Picker;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_foundation::tooltip_text::state_name;
use viewer_foundation::tooltip_text::{TextIcons, builtin, token};
use super::content::Content as StaticContent;
use dashboard_text::table_access::{Available, rows};

struct Province<'a> {
    id: u32,
    rename: Result<Option<&'a [u8]>, String>,
    vp_tag: Result<&'a str, String>,
    state: Option<Result<StateNames<'a>, String>>,
    region_name_key: Option<&'a str>,
}

pub(crate) struct StateNames<'a> {
    pub(crate) id: u32,
    pub(crate) custom_name: &'a [u8],
    pub(crate) owner_name: &'a [u8],
    pub(crate) recorded_language: &'a str,
    pub(crate) name_key: Result<&'a str, &'a str>,
}

pub(crate) fn state_display_name(
    icons: &dyn TextIcons,
    loc: &Localisation,
    state: &StateNames<'_>,
) -> Result<String, String> {
    let id = state.id;
    let (custom, owner) = (state.custom_name, state.owner_name);
    state_name(loc, state.recorded_language, id, custom, owner, || {
        let key = state
            .name_key
            .map_err(|reason| format!("state {id}'s definition name is unknown: {reason}"))?;
        token(icons, loc, key.as_bytes()).map_err(|reason| format!("state {id}'s name: {reason}"))
    })
}

fn province_name(
    icons: &dyn TextIcons,
    loc: &Localisation,
    province: &Province<'_>,
) -> Result<String, String> {
    let id = province.id;
    let rename = province
        .rename
        .as_ref()
        .map_err(|reason| format!("province {id}'s recorded rename is unknown: {reason}"))?;
    if let Some(bytes) = rename.filter(|bytes| !bytes.is_empty()) {
        return token(icons, loc, bytes)
            .map_err(|reason| format!("province {id}'s recorded rename: {reason}"));
    }
    let tag = province.vp_tag.as_deref().map_err(|reason| {
        format!("province {id}'s qualified victory point key needs a tag, but {reason}")
    })?;
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
        Some(Err(reason)) => Err(format!("province {id}'s state display name: {reason}")),
        None => match province.region_name_key {
            Some(key) => token(icons, loc, key.as_bytes())
                .map_err(|reason| format!("province {id}'s region name: {reason}")),
            None => Ok(id.to_string()),
        },
    }
}

pub(super) fn title(
    static_content: &StaticContent,
    frame: &DecodedFrame,
    province: u32,
    port_strike: bool,
) -> io::Result<Result<String, String>> {
    let (icons, loc) = (static_content.assets.as_ref(), &static_content.localisation);
    let Some(region) = static_content.region_of.get(&province).copied() else {
        return Ok(Err(format!(
            "province {province} is in no content strategic region"
        )));
    };
    let Some(definition) = static_content.strategic_regions.get(&region) else {
        return Ok(Err(format!(
            "strategic region {region} has no content definition"
        )));
    };
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
    let [factor, cutoff] = match &static_content.assets.naval_name_defines {
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
    let geometry = &static_content.province_geometry;
    let position = |id: u32| geometry.map_position(id).unwrap_or([0, 0]);
    let picker = Picker {
        width: static_content.map.width as i32,
        factor,
        cutoff,
        position: &position,
        victory_points: &victory_points,
    };
    let candidates = static_content
        .battle_name_provinces
        .get(&region)
        .map_or(&[][..], Vec::as_slice);
    let name = match picker.pick(province, candidates) {
        Err(reason) => return Ok(Err(reason)),
        Ok(None) => region_name,
        Ok(Some(picked)) => match province_display_name(static_content, frame, picked)? {
            Ok(name) => name,
            Err(reason) => return Ok(Err(reason)),
        },
    };
    Ok(builtin(
        icons,
        loc,
        "NAVAL_BATTLE_TITLE",
        &[("NAME", Argument::Text(&name))],
    ))
}

pub(super) fn province_display_name(
    static_content: &StaticContent,
    frame: &DecodedFrame,
    picked: u32,
) -> io::Result<Result<String, String>> {
    let (icons, loc) = (static_content.assets.as_ref(), &static_content.localisation);
    let provinces = rows::<tables::Provinces>(frame)?;
    let province = provinces
        .rows()
        .binary_search_by_key(&picked, |row| row.id)
        .ok()
        .map(|at| &provinces.rows()[at]);

    let renames = rows::<tables::ProvinceNames>(frame)?;
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
            Available::NotRecorded { mode } => Err(format!(
                "the ProvinceNames table is not recorded in a {mode} recording"
            )),
        },
    };
    let states = rows::<tables::States>(frame)?;
    let states_complete = states.is_complete();
    let vp_tag = match province {
        None => Err(format!("province {picked} has no Provinces row")),
        Some(row) => vp_tag(frame, row.state, states.rows(), states_complete)?,
    };
    let state = province.and_then(|row| {
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
                    let name_key = static_content
                        .state_name_keys
                        .get(&id)
                        .ok_or_else(|| format!("state {id} has no content definition"))?;
                    Ok(StateNames {
                        id,
                        custom_name: &recorded.custom_name,
                        owner_name: &recorded.owner_name,
                        recorded_language: &static_content.recorded_language,
                        name_key: name_key.as_deref().map_err(String::as_str),
                    })
                })
            });
    let state = match province {
        Some(_) => state,
        None => Some(Err(format!("province {picked} has no Provinces row"))),
    };
    let region_name_key = static_content
        .region_of
        .get(&picked)
        .copied()
        .and_then(|region| static_content.strategic_regions.get(&region))
        .map(|region| region.name_key.as_str());
    Ok(province_name(
        icons,
        loc,
        &Province {
            id: picked,
            rename,
            vp_tag: vp_tag.as_deref().map_err(Clone::clone),
            state,
            region_name_key,
        },
    ))
}

fn vp_tag(
    frame: &DecodedFrame,
    state: u32,
    states: &[replay_data::rows::StateRow],
    states_complete: bool,
) -> io::Result<Result<String, String>> {
    let (subject, slot) = if state == 0 {
        (
            "the country at index 0, which a province without a state qualifies with".to_owned(),
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
    Ok(
        content_reader_country_names::country_names::original_or_own_tag(frame, slot)
            .map(str::to_owned)
            .map_err(|reason| format!("{subject}: {reason}")),
    )
}

#[cfg(test)]
mod tests;
