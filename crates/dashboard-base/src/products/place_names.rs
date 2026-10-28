//! Names for province/hour and state/hour pairs, at each row's read hour.
//!
//! Province names try a nonempty recorded override through localisation
//! (even an empty result), then the state-owner original-or-own tag plus
//! `_VICTORY_POINTS_<id>`,
//! then `VICTORY_POINTS_<id>`. Only defined victory-point keys are used.
//! State names prefer the stored custom and owner-qualified strings, then
//! the content template. Custom strings are localised when assigned, and
//! owner-qualified strings only at an owner change or load. Those
//! strings are never reconstructed from today's owner or a different language.
//! A nonempty victory-point name is the battle name; otherwise use the state's
//! display name, without prefixing a province id or appending another name.
//! Provinces without a state use their strategic-region name;
//! region membership/names use installed_content_regions::regions.
//! Land requests use the approved primary name -> state -> unnamed ladder:
//! empty primary names try the state, and state-less provinces stay unnamed.
//! A known primary name needs no state-name lookup. Unresolved names remain null;
//! the caller shows Unnamed province and keeps the id only in its tooltip.
//! Provinces, States, Countries, CountryLinks, CountryAppearance and
//! ProvinceNames are hourly (older recordings read ProvinceNames daily and
//! carry it between reads). IDs and raw owner links are 32-bit; names retain
//! byte strings.

use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, recorded_rows};
use crate::{
    Source,
    catalogue::{self, Catalogue, Refusals},
    metadata::{check_cancel, validate_campaign},
    scripted_text::{self, ScriptedText, StateNames},
    transport::json_len,
};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct PlaceNames;

/// The tables province and state names are read from.
pub const NAME_TABLES: [TableId; 8] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
];
const STATE_TABLES: [TableId; 2] = [TableId::Campaign, TableId::States];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNamesParams {
    pub branch_id: String,
    /// Land's presentation ladder; other callers retain the native resolver.
    #[serde(default)]
    pub land: bool,
    pub places: Vec<PlaceNameRequest>,
    #[serde(default)]
    pub states: Vec<StateNameRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNameRequest {
    pub province_id: u32,
    pub hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateNameRequest {
    pub state_id: u32,
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaceName {
    pub province_id: u32,
    pub hour: i32,
    /// Native province display name; null when no name resolves.
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaceStateName {
    pub state_id: u32,
    pub hour: i32,
    /// Native state display name; null when no name resolves.
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaceNamesReply {
    pub places: Vec<PlaceName>,
    pub states: Vec<PlaceStateName>,
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
    let names = installed_content_regions::regions::read(&cat.content.layers)
        .map(|regions| {
            let provinces = region_provinces(&regions, |region| {
                if !cat.has_presentation_key(&region.name_key) {
                    Err(format!(
                        "localisation {} is missing; region title is unavailable",
                        region.name_key
                    ))
                } else {
                    cat.localisation
                        .localise(region.name_key.as_bytes())
                        .map_err(|error| error.to_string())
                        .and_then(|name| catalogue::plain_text(&name))
                }
            });
            Arc::new(RegionNames {
                provinces,
                warnings: regions.notes,
            })
        })
        .map_err(|error| format!("Strategic-region names unavailable: {error}"));
    state.content = Some((Arc::clone(cat), names.clone()));
    names
}

/// Each province's strategic-region name (`name`), refused for a province two regions list. A
/// province one region lists more than once is in that region once, as the game builds it.
fn region_provinces(
    regions: &installed_content_regions::regions::StrategicRegions,
    name: impl Fn(&installed_content_regions::regions::StrategicRegion) -> Result<String, String>,
) -> BTreeMap<u32, Result<String, String>> {
    let mut provinces = BTreeMap::new();
    for region in regions.regions.values() {
        let named = name(region);
        for province in region.native_provinces(|_| true) {
            if provinces.insert(province, named.clone()).is_some() {
                provinces.insert(
                    province,
                    Err(format!(
                        "province {province}: multiple strategic-region memberships refused"
                    )),
                );
            }
        }
    }
    provinces
}

fn name_at(
    frame: &DecodedFrame,
    text: &ScriptedText<'_>,
    province_id: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
    land: bool,
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
    if let Ok(Some(name)) = &province_name
        && (!land || !name.is_empty())
    {
        // Preserve successful empty localisation.
        return (Some(name.clone()), None);
    }
    if province.state == 0 {
        if land {
            let missing = format!("province {province_id}: no state name available");
            return (
                None,
                Some(match province_name {
                    Err(reason) => format!("{reason}; {missing}"),
                    Ok(_) => missing,
                }),
            );
        }
        let region = regions.map_err(str::to_owned).and_then(|regions| {
            regions
                .get(&province_id)
                .ok_or_else(|| {
                    format!("province {province_id}: no content-defined strategic region")
                })?
                .clone()
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
    // Template text remains valid when empty; absence of
    // recorded/template inputs remains an error, distinct from that result.
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

fn state_name_at(
    frame: &DecodedFrame,
    state_id: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
) -> (Option<String>, Option<String>) {
    let name = recorded_rows::<tables::States>(frame)
        .map_err(|error| error.to_string())
        .and_then(|rows| {
            rows.iter()
                .find(|state| state.id == state_id)
                .ok_or_else(|| format!("Not in this recording: state {state_id} at this hour"))
        })
        .and_then(|state| {
            scripted_text::state_display_name(
                state,
                cat,
                names,
                scripted_text::StateNameUsage::Place,
            )
        });
    match name {
        Ok(name) => (Some(name), None),
        Err(reason) => (None, Some(reason)),
    }
}

fn named_state(
    request: &StateNameRequest,
    name: (Option<String>, Option<String>),
) -> PlaceStateName {
    let (name, missing) = name;
    PlaceStateName {
        state_id: request.state_id,
        hour: request.hour,
        name,
        missing,
    }
}

enum RequestedName {
    Place(usize),
    State(usize),
}

impl RequestedName {
    fn missing<'a>(
        &self,
        places: &'a mut [PlaceName],
        states: &'a mut [PlaceStateName],
    ) -> &'a mut Option<String> {
        match self {
            Self::Place(at) => &mut places[*at].missing,
            Self::State(at) => &mut states[*at].missing,
        }
    }
}

fn read_names(
    replay: &Replay,
    branch: u64,
    params: &PlaceNamesParams,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<PlaceName>, Vec<PlaceStateName>)> {
    let mut places: Vec<_> = params
        .places
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
    let mut states: Vec<_> = params
        .states
        .iter()
        .map(|request| {
            named_state(
                request,
                (
                    None,
                    Some("Not in this recording: the row hour was not recorded".into()),
                ),
            )
        })
        .collect();
    let mut blocks = BTreeMap::<usize, BTreeMap<i32, Vec<RequestedName>>>::new();
    for (hour, request) in params
        .places
        .iter()
        .enumerate()
        .map(|(at, request)| (request.hour, RequestedName::Place(at)))
        .chain(
            params
                .states
                .iter()
                .enumerate()
                .map(|(at, request)| (request.hour, RequestedName::State(at))),
        )
    {
        // An hour a recorded gap covers is unobserved, even where its frame survived.
        if replay.gaps.iter().any(|gap| {
            gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour
        }) {
            *request.missing(&mut places, &mut states) = Some(format!(
                "Not in this recording: {}",
                replay.gap_reason(branch, hour)?
            ));
            continue;
        }
        if let Some(block) = replay.index.iter().position(|block| {
            block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
        }) {
            blocks
                .entry(block)
                .or_default()
                .entry(hour)
                .or_default()
                .push(request);
        }
    }
    let tables: &[TableId] = if params.places.is_empty() {
        &STATE_TABLES
    } else {
        &NAME_TABLES
    };
    for (block, mut hours) in blocks {
        check_cancel(cancelled)?;
        let frames = match replay.read_block_selected(&replay.index[block], tables) {
            Ok(frames) => frames,
            Err(error) => {
                for request in hours.values().flatten() {
                    *request.missing(&mut places, &mut states) = Some(format!(
                        "Not in this recording: unreadable names block: {error}"
                    ));
                }
                continue;
            }
        };
        let mut decoder = FrameDecoder::for_tables(tables);
        for frame in frames {
            if hours.is_empty() {
                break;
            }
            check_cancel(cancelled)?;
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) => decoded,
                Err(error) => {
                    for request in hours.values().flatten() {
                        *request.missing(&mut places, &mut states) = Some(format!(
                            "Not in this recording: invalid names baseline: {error}"
                        ));
                    }
                    break;
                }
            };
            validate_campaign(decoded, replay)?;
            if let Some(requested) = hours.remove(&frame.hour) {
                let text = if requested
                    .iter()
                    .any(|request| matches!(request, RequestedName::Place(_)))
                {
                    Some(ScriptedText::new(
                        decoded,
                        &cat.localisation,
                        cat.scripted_content().map_err(crate::invalid)?,
                        0,
                    ))
                } else {
                    None
                };
                for request in requested {
                    match request {
                        RequestedName::Place(at) => {
                            places[at] = named_place(
                                &params.places[at],
                                name_at(
                                    decoded,
                                    text.as_ref().unwrap(),
                                    params.places[at].province_id,
                                    cat,
                                    names,
                                    regions,
                                    params.land,
                                ),
                            );
                        }
                        RequestedName::State(at) => {
                            states[at] = named_state(
                                &params.states[at],
                                state_name_at(decoded, params.states[at].state_id, cat, names),
                            );
                        }
                    }
                }
            }
        }
    }
    Ok((places, states))
}

impl Product for PlaceNames {
    const KEY: &'static str = "place-names";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = PlaceNamesParams;
    type Reply = PlaceNamesReply;

    fn types() -> Vec<String> {
        vec![
            PlaceNamesParams::decl(),
            PlaceNameRequest::decl(),
            StateNameRequest::decl(),
            PlaceName::decl(),
            PlaceStateName::decl(),
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
        checked(resolve(source, params, context, |cat, names, regions| {
            read_names(replay, branch, params, cat, names, regions, cancelled)
        })?)
    }
}

fn resolve(
    source: &Source,
    params: &PlaceNamesParams,
    context: &LoadContext,
    read: impl FnOnce(
        &Catalogue,
        Result<&StateNames, &str>,
        Result<&BTreeMap<u32, Result<String, String>>, &str>,
    ) -> io::Result<(Vec<PlaceName>, Vec<PlaceStateName>)>,
) -> io::Result<PlaceNamesReply> {
    let cat = context
        .content
        .as_ref()
        .ok_or_else(|| "Place names need the recorded content stack and language".to_owned())
        .and_then(|options| {
            catalogue::catalogue(source, options).map_err(|error| error.to_string())
        });
    let mut warnings = Vec::new();
    let (places, states) = match cat {
        Ok(cat) => {
            warnings.extend(cat.verification.iter().chain(&cat.warnings).cloned());
            let names = cat.cached_states().map(|states| &states.names);
            let regions = if params.places.is_empty() || params.land {
                Ok(Arc::new(RegionNames {
                    provinces: BTreeMap::new(),
                    warnings: Vec::new(),
                }))
            } else {
                region_names(source, &cat)
            };
            match &names {
                Ok(_) => {}
                Err(reason) => warnings.push(reason.clone()),
            }
            match &regions {
                Ok(regions) => warnings.extend(regions.warnings.iter().cloned()),
                Err(reason) => warnings.push(reason.clone()),
            }
            let resolved = read(
                &cat,
                names.as_ref().copied().map_err(String::as_str),
                regions
                    .as_ref()
                    .map(|regions| &regions.provinces)
                    .map_err(String::as_str),
            )?;
            warnings.extend(cat.refusal_lines());
            resolved
        }
        Err(reason) => {
            warnings.push(reason.clone());
            (
                params
                    .places
                    .iter()
                    .map(|request| named_place(request, (None, Some(reason.clone()))))
                    .collect(),
                params
                    .states
                    .iter()
                    .map(|request| named_state(request, (None, Some(reason.clone()))))
                    .collect(),
            )
        }
    };
    Ok(PlaceNamesReply {
        places,
        states,
        warnings,
    })
}

/// Preparation passes the accepted decoded frame to the existing name ladder.
/// Refusal aggregation remains at the reply boundary so a drawn subset gets its own counts.
pub fn at_frame(
    source: &Source,
    params: &PlaceNamesParams,
    context: &LoadContext,
    frame: &DecodedFrame,
    flag: &AtomicBool,
) -> io::Result<PlaceNamesReply> {
    check_cancel(flag)?;
    resolve(source, params, context, |cat, names, regions| {
        let text = ScriptedText::new(
            frame,
            &cat.localisation,
            cat.scripted_content().map_err(crate::invalid)?,
            0,
        );
        let places = params
            .places
            .iter()
            .map(|request| {
                named_place(
                    request,
                    name_at(
                        frame,
                        &text,
                        request.province_id,
                        cat,
                        names,
                        regions,
                        params.land,
                    ),
                )
            })
            .collect();
        let states = params
            .states
            .iter()
            .map(|request| named_state(request, state_name_at(frame, request.state_id, cat, names)))
            .collect();
        Ok((places, states))
    })
}

pub fn checked(reply: PlaceNamesReply) -> io::Result<PlaceNamesReply> {
    let PlaceNamesReply {
        places,
        states,
        mut warnings,
    } = reply;
    let mut refused = Refusals::default();
    for place in &places {
        if let Some(reason) = &place.missing {
            refused.note(
                reason,
                format!("province {} at {}", place.province_id, place.hour),
            );
        }
    }
    for state in &states {
        if let Some(reason) = &state.missing {
            refused.note(
                reason,
                format!("state {} at {}", state.state_id, state.hour),
            );
        }
    }
    warnings.extend(refused.lines("place names"));
    warnings.sort();
    warnings.dedup();
    let reply = PlaceNamesReply {
        places,
        states,
        warnings,
    };
    json_len(&reply, MAX_PRODUCT_REPLY_BYTES)
        .ok_or_else(|| crate::invalid("The Place names reply exceeds its size budget"))?;
    Ok(reply)
}

#[cfg(test)]
mod tests;
