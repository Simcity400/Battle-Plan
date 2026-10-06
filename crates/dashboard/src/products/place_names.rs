//! Names for drawn province/hour pairs, at each row's read hour.
//!
//! NCombatLogView::CCombatItem::UpdateDisplay@1416E07C0 reads the recorded
//! province id (+0x440) and uses 140E69320: a nonempty raw CProvince+0x40
//! override through localisation (even an empty result), then ResolveVictoryPointLocalization
//! @140E68E30's state-owner original-or-own tag + `_VICTORY_POINTS_<id>`,
//! then `VICTORY_POINTS_<id>`. Only defined victory-point keys are used.
//! State display_name@1409CC380 prefers the stored custom+0x38 and
//! owner+0xE8 strings, then the content template. set_state_name@140310B50
//! localises the custom string; SetOwner@1409D1710 -> 1409D3770 stores the
//! localised owner-qualified string only at an owner change or load. Those
//! strings are never reconstructed from today's owner or a different language.
//! A nonempty victory-point name is the battle name; otherwise use the state's
//! display name, without prefixing a province id or appending another name.
//! Provinces without a state use their strategic-region name (battle name ladder
//! 0x140E69320, CProvince+0xC8); region membership/names use installed_content::regions.
//! A known primary name needs no state-name lookup. Unresolved names remain null;
//! the caller shows Unnamed province and keeps the id only in its tooltip.
//! Provinces, States, Countries, CountryLinks and CountryAppearance are hourly;
//! ProvinceNames keeps its recorded daily read (cadence.rs), including carried
//! overrides. IDs and raw owner links are 32-bit; names retain byte strings.

use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, recorded_rows};
use crate::{
    Source,
    catalogue::{self, Catalogue, Refusals},
    metadata::{check_cancel, validate_campaign},
    scripted_text::{self, ScriptedText, StateNames},
    transport::json_len,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct PlaceNames;

const NAME_TABLES: [TableId; 7] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNamesParams {
    pub branch_id: String,
    pub places: Vec<PlaceNameRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNameRequest {
    pub province_id: u32,
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaceName {
    pub province_id: u32,
    pub hour: i32,
    /// Native province display name; null when no name resolves.
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaceNamesReply {
    pub places: Vec<PlaceName>,
    pub warnings: Vec<String>,
}

struct RegionNames {
    provinces: BTreeMap<u32, Result<String, String>>,
    warnings: Vec<String>,
}
/// Region names read for one catalogue.
type CachedRegionNames = (Arc<Catalogue>, Result<Arc<RegionNames>, String>);

#[derive(Default)]
struct PlaceState {
    content: Option<CachedRegionNames>,
}

fn region_names(source: &Source, cat: &Arc<Catalogue>) -> Result<Arc<RegionNames>, String> {
    let state = source.state::<PlaceState>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, names)) = &state.content
        && Arc::ptr_eq(cached, cat)
    {
        return names.clone();
    }
    let names = installed_content::regions::read(&cat.content.layers)
        .map(|regions| {
            let mut provinces = BTreeMap::new();
            for region in regions.regions.values() {
                for &province in &region.provinces {
                    let name = cat
                        .localisation
                        .localise(region.name_key.as_bytes())
                        .map_err(|error| error.to_string())
                        .and_then(|name| catalogue::plain_text(&name));
                    if provinces.insert(province, name).is_some() {
                        provinces.insert(
                            province,
                            Err(format!(
                                "province {province}: multiple strategic-region memberships refused"
                            )),
                        );
                    }
                }
            }
            Arc::new(RegionNames {
                provinces,
                warnings: regions.notes,
            })
        })
        .map_err(|error| format!("Strategic-region names unavailable: {error}"));
    state.content = Some((Arc::clone(cat), names.clone()));
    names
}

fn name_at(
    frame: &DecodedFrame,
    text: &ScriptedText<'_>,
    province_id: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
) -> (Option<String>, Option<String>) {
    let province = match recorded_rows::<tables::Provinces>(frame)
        .ok()
        .and_then(|rows| rows.iter().find(|province| province.id == province_id))
    {
        Some(province) => province,
        None => {
            return (
                None,
                Some(format!(
                    "Not in this recording: province {province_id} at this hour"
                )),
            );
        }
    };
    let province_name = text
        .province_display_name_primary(province_id)
        .and_then(|name| name.map(|name| catalogue::plain_text(&name)).transpose());
    if let Ok(Some(name)) = &province_name {
        return if name.is_empty() {
            (
                None,
                Some(format!(
                    "province {province_id}: native display name is empty"
                )),
            )
        } else {
            (Some(name.clone()), None)
        };
    }
    if province.state == 0 {
        let region = regions
            .map_err(str::to_owned)
            .and_then(|regions| {
                regions
                    .get(&province_id)
                    .ok_or_else(|| {
                        format!("province {province_id}: no content-defined strategic region")
                    })?
                    .clone()
            })
            .and_then(|name| {
                if name.is_empty() {
                    Err(format!(
                        "province {province_id}: native region display name is empty"
                    ))
                } else {
                    Ok(name)
                }
            });
        // Region identity is independent of an unread province override/owner.
        return match (province_name, region) {
            (Ok(_), Ok(region)) => (Some(region), None),
            (Err(reason), Ok(region)) => (Some(region), Some(reason)),
            (Ok(_), Err(reason)) => (None, Some(reason)),
            (Err(province), Err(region)) => (None, Some(format!("{province}; {region}"))),
        };
    }
    let state_name = recorded_rows::<tables::States>(frame)
        .map_err(|error| error.to_string())
        .and_then(|rows| {
            rows.iter()
                .find(|state| state.id == province.state)
                .ok_or_else(|| format!("state {}: Not in this recording", province.state))
        })
        .and_then(|state| {
            scripted_text::state_display_name(
                state,
                cat,
                names,
                scripted_text::StateNameUsage::Place,
            )
        });
    let state_name = state_name.and_then(|name| {
        if name.is_empty() {
            Err(format!(
                "province {province_id}: native state display name is empty"
            ))
        } else {
            Ok(name)
        }
    });
    match (province_name, state_name) {
        (Ok(_), Ok(state)) => (Some(state), None),
        (Err(reason), Ok(state)) => (Some(state), Some(reason)),
        (Ok(_), Err(reason)) => (None, Some(reason)),
        (Err(province), Err(state)) => (None, Some(format!("{province}; {state}"))),
    }
}

fn named_place(request: &PlaceNameRequest, name: (Option<String>, Option<String>)) -> PlaceName {
    let (name, missing) = name;
    PlaceName {
        province_id: request.province_id,
        hour: request.hour,
        name,
        missing,
    }
}

fn read_places(
    replay: &Replay,
    branch: u64,
    requests: &[PlaceNameRequest],
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<PlaceName>> {
    let mut places: Vec<_> = requests
        .iter()
        .map(|request| {
            named_place(
                request,
                (
                    None,
                    Some("Not in this recording: the row hour was not recorded".into()),
                ),
            )
        })
        .collect();
    let mut blocks = BTreeMap::<usize, BTreeMap<i32, Vec<usize>>>::new();
    for (at, request) in requests.iter().enumerate() {
        if let Some(block) = replay.index.iter().position(|block| {
            block.branch == branch
                && block.first_hour <= request.hour
                && request.hour <= block.last_hour
        }) {
            blocks
                .entry(block)
                .or_default()
                .entry(request.hour)
                .or_default()
                .push(at);
        }
    }
    for (block, mut hours) in blocks {
        check_cancel(cancelled)?;
        let frames = match replay.read_block_selected(&replay.index[block], &NAME_TABLES) {
            Ok(frames) => frames,
            Err(error) => {
                for at in hours.values().flatten() {
                    places[*at].missing = Some(format!(
                        "Not in this recording: unreadable names block: {error}"
                    ));
                }
                continue;
            }
        };
        let mut decoder = FrameDecoder::for_tables(&NAME_TABLES);
        for frame in frames {
            if hours.is_empty() {
                break;
            }
            check_cancel(cancelled)?;
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) => decoded,
                Err(error) => {
                    for at in hours.values().flatten() {
                        places[*at].missing = Some(format!(
                            "Not in this recording: invalid names baseline: {error}"
                        ));
                    }
                    break;
                }
            };
            validate_campaign(decoded, replay)?;
            if let Some(requested) = hours.remove(&frame.hour) {
                let text = ScriptedText::new(
                    decoded,
                    &cat.localisation,
                    cat.scripted_content().map_err(crate::invalid)?,
                    0,
                );
                for at in requested {
                    places[at] = named_place(
                        &requests[at],
                        name_at(
                            decoded,
                            &text,
                            requests[at].province_id,
                            cat,
                            names,
                            regions,
                        ),
                    );
                }
            }
        }
    }
    Ok(places)
}

impl Product for PlaceNames {
    const KEY: &'static str = "place-names";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PlaceNamesParams;
    type Reply = PlaceNamesReply;

    fn types() -> Vec<String> {
        vec![
            PlaceNamesParams::decl(),
            PlaceNameRequest::decl(),
            PlaceName::decl(),
            PlaceNamesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid place names branch");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(params).map_err(crate::invalid)?;
        check_cancel(cancelled)?;
        let branch = params
            .branch_id
            .parse::<u64>()
            .map_err(|_| crate::invalid("Invalid place names branch"))?;
        let replay = source.replay_ref();
        if !replay.branches.iter().any(|entry| entry.id == branch) {
            return Err(crate::invalid("Unknown place names branch"));
        }
        let cat = context
            .content
            .as_ref()
            .ok_or_else(|| "Place names need the recorded content stack and language".to_owned())
            .and_then(|options| {
                catalogue::catalogue(source, options).map_err(|error| error.to_string())
            });
        let mut warnings = Vec::new();
        let places = match cat {
            Ok(cat) => {
                warnings.extend(cat.verification.iter().chain(&cat.warnings).cloned());
                let names = cat.cached_states().map(|states| &states.names);
                let regions = region_names(source, &cat);
                match &names {
                    Ok(_) => {}
                    Err(reason) => warnings.push(reason.clone()),
                }
                match &regions {
                    Ok(regions) => warnings.extend(regions.warnings.iter().cloned()),
                    Err(reason) => warnings.push(reason.clone()),
                }
                let places = read_places(
                    replay,
                    branch,
                    &params.places,
                    &cat,
                    names.as_ref().copied().map_err(String::as_str),
                    regions
                        .as_ref()
                        .map(|regions| &regions.provinces)
                        .map_err(String::as_str),
                    cancelled,
                )?;
                warnings.extend(cat.refusal_lines());
                places
            }
            Err(reason) => {
                warnings.push(reason.clone());
                params
                    .places
                    .iter()
                    .map(|request| named_place(request, (None, Some(reason.clone()))))
                    .collect()
            }
        };
        let mut refused = Refusals::default();
        for place in &places {
            if let Some(reason) = &place.missing {
                refused.note(
                    reason,
                    format!("province {} at {}", place.province_id, place.hour),
                );
            }
        }
        warnings.extend(refused.lines("place names"));
        warnings.sort();
        warnings.dedup();
        let reply = PlaceNamesReply { places, warnings };
        json_len(&reply, MAX_PRODUCT_REPLY_BYTES)
            .ok_or_else(|| crate::invalid("The Place names reply exceeds its size budget"))?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
