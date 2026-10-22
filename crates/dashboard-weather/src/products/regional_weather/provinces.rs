//! One region at one reached hour. Membership, names and each weather field
//! refuse independently; no prepared history or other block is scanned.

use super::{
    RegionalWeatherParams, RegionalWeatherProvince, RegionalWeatherProvincesReply,
    RegionalWeatherResponse, temperature,
};
use dashboard_base::{
    catalogue::{self, KeyedResults, Refusals},
    content,
    prepared::query::progressive,
    products::{LoadContext, place_names},
};
use dashboard_keyed::query::{self as keyed, Query};
use installed_content_core::province_geometry::ProvinceGeometry;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io, sync::Arc};

// The shared at-frame place-name reader's selected inputs, plus province
// weather. Campaign is included by the frame cache as well.
const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
    TableId::WeatherProvinces,
];

/// Independent of weather settings, terrain and pass content: failure to
/// reconstruct temperature must not hide the member roster, snow or raw mud.
struct Membership {
    regions: BTreeMap<u32, Vec<u32>>,
    warnings: Vec<String>,
}

fn membership(query: &Query<'_>) -> io::Result<Arc<Membership>> {
    let options = query.content.ok_or_else(|| {
        dashboard_core::invalid(
            "province membership needs the recorded content stack; game folders are not chosen",
        )
    })?;
    query
        .source
        .state::<KeyedResults<catalogue::RecordedBinding, Membership>>()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_init(&catalogue::recorded_binding(options), |_| {
            dashboard_text::metadata::check_cancel(query.cancelled)?;
            let mut warnings = Vec::new();
            content::check_replay(
                query.source.replay_ref(),
                options.allow_unverified,
                &mut warnings,
            )?;
            let resolved = query.source.content(&options.options)?;
            let geometry = ProvinceGeometry::read(&resolved.layers)?;
            let regions = installed_content_regions::regions::read(&resolved.layers)?;
            dashboard_text::metadata::check_cancel(query.cancelled)?;
            warnings.extend(regions.notes.iter().cloned());
            Ok(Membership {
                regions: temperature::membership(&regions, &geometry),
                warnings,
            })
        })
}

/// Each source owns one drill memo per branch. Binding changes reset only
/// that memo; this never mutates the prepared observer or publishes history.
#[derive(Default)]
struct Temperatures(BTreeMap<u64, temperature::Observation>);

fn temperatures(
    query: &Query<'_>,
    frame: &DecodedFrame,
    members: &[u32],
    warnings: &mut Vec<String>,
) -> io::Result<Result<Vec<Option<i64>>, String>> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    let state = query.source.state::<Temperatures>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let observation = state.0.entry(query.branch).or_default();
    observation.bind(query.source, query.content, query.cancelled)?;
    warnings.extend(observation.warnings());
    let values = observation.states(frame).map(|states| {
        members
            .iter()
            .map(|&id| states.get(id as usize).map(|state| state.temperature))
            .collect()
    });
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    Ok(values)
}

fn names(
    query: &Query<'_>,
    frame: &DecodedFrame,
    members: &[u32],
    warnings: &mut Vec<String>,
) -> io::Result<BTreeMap<u32, place_names::PlaceName>> {
    let reply = place_names::at_frame(
        query.source,
        &place_names::PlaceNamesParams {
            branch_id: query.branch.to_string(),
            land: false,
            places: members
                .iter()
                .map(|&province_id| place_names::PlaceNameRequest {
                    province_id,
                    hour: frame.hour,
                })
                .collect(),
            states: vec![],
        },
        &LoadContext {
            content: query.content.cloned(),
        },
        frame,
        query.cancelled,
    )?;
    warnings.extend(reply.warnings);
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    Ok(reply
        .places
        .into_iter()
        .map(|place| (place.province_id, place))
        .collect())
}

fn finish(
    query: &Query<'_>,
    reply: RegionalWeatherProvincesReply,
) -> io::Result<serde_json::Value> {
    keyed::finish(query, &RegionalWeatherResponse::Provinces(reply))
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

pub(super) fn query(
    query: &Query<'_>,
    params: &RegionalWeatherParams,
) -> io::Result<serde_json::Value> {
    let hour = params.hour.expect("validated province hour");
    let region = params.region.as_ref().expect("validated province region");
    let (first, last) = query.bounds()?;
    if hour < first || hour > last {
        return Err(dashboard_core::invalid(
            "Province weather hour is outside the branch",
        ));
    }
    let mut reply = RegionalWeatherProvincesReply {
        hour,
        read_hour: None,
        region: region.clone(),
        provinces: vec![],
        warnings: vec![],
    };
    let membership = match membership(query) {
        Ok(membership) => membership,
        Err(error) if retryable(&error) => return Err(error),
        Err(error) => {
            reply.warnings.push(format!(
                "1 weather province roster not shown: region {region}: {error}"
            ));
            return finish(query, reply);
        }
    };
    reply.warnings.extend(membership.warnings.iter().cloned());
    let Some(members) = membership
        .regions
        .get(&region.parse().expect("validated region"))
    else {
        reply.warnings.push(format!("1 weather province roster not shown: region {region} is absent from the recorded content stack"));
        return finish(query, reply);
    };
    if members.is_empty() {
        return finish(query, reply);
    }
    // This view is a transient read independent of the preparation prefix.
    // Never read a prior hour or scan the branch to fill a missing operand.
    let frame =
        match progressive::frame_at(query.source, query.branch, hour, TABLES, query.cancelled) {
            Ok(frame) => frame.filter(|frame| frame.hour == hour),
            Err(error) if retryable(&error) => return Err(error),
            Err(error) => {
                reply
                    .warnings
                    .push(format!("1 weather province frame unavailable: {error}"));
                None
            }
        };
    let Some(frame) = frame else {
        reply.provinces = members
            .iter()
            .map(|id| RegionalWeatherProvince {
                id: id.to_string(),
                name: None,
                snow: None,
                mud: None,
                temperature: None,
            })
            .collect();
        reply.warnings.push(format!(
            "{} weather province details not shown: no usable recorded frame at hour {hour}",
            members.len()
        ));
        return finish(query, reply);
    };
    at_frame(query, &frame, members, reply)
}

/// The exact-frame projection, also used by synthetic field-refusal tests.
fn at_frame(
    query: &Query<'_>,
    frame: &DecodedFrame,
    members: &[u32],
    mut reply: RegionalWeatherProvincesReply,
) -> io::Result<serde_json::Value> {
    let hour = frame.hour;
    let weather = frame.complete::<tables::WeatherProvinces>();
    if weather.is_ok() {
        reply.read_hour = frame.tables[TableId::WeatherProvinces.index()]
            .sampled_at()
            .filter(|&read| read <= hour)
            .or_else(|| {
                dashboard_text::metadata::slot_read(
                    query.source.replay_ref().format,
                    &frame.metrics,
                    TableId::WeatherProvinces,
                    hour,
                )
                .then_some(hour)
            });
    }
    let temperatures = temperatures(query, frame, members, &mut reply.warnings)?;
    let places = match names(query, frame, members, &mut reply.warnings) {
        Ok(places) => Ok(places),
        Err(error) if retryable(&error) => return Err(error),
        Err(error) => Err(error.to_string()),
    };
    let rows: BTreeMap<_, _> = weather
        .as_ref()
        .ok()
        .into_iter()
        .flat_map(|rows| rows.iter())
        .map(|row| (row.id, row))
        .collect();
    let mut refused = Refusals::default();
    for (at, &id) in members.iter().enumerate() {
        dashboard_text::metadata::check_cancel(query.cancelled)?;
        let identity = format!("province {id}");
        let name = match &places {
            Ok(places) => match places.get(&id) {
                Some(place) => {
                    if let Some(reason) = &place.missing {
                        refused.note(format!("name: {reason}"), &identity);
                    } else if place.name.is_none() {
                        refused.note("name: no place name resolves", &identity);
                    }
                    place.name.clone()
                }
                None => {
                    refused.note(
                        "name: province missing from the shared place-name reply",
                        &identity,
                    );
                    None
                }
            },
            Err(reason) => {
                refused.note(format!("name: {reason}"), &identity);
                None
            }
        };
        let (snow, mud) = match rows.get(&id) {
            Some(row) => {
                // The game names mud on a province whenever its raw byte is nonzero,
                // the rule the viewer's province weather tooltip applies.
                let mud = match row.raw_mud.require("WeatherProvinces.raw_mud") {
                    Ok(&mud) => Some(mud != 0),
                    Err(error) => {
                        refused.note(format!("raw mud: {error}"), &identity);
                        None
                    }
                };
                (Some(row.snow.to_string()), mud)
            }
            None => {
                let reason = weather
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "province has no recorded weather row".into());
                refused.note(format!("snow and raw mud: {reason}"), &identity);
                (None, None)
            }
        };
        let temperature = match &temperatures {
            Ok(values) => match values[at] {
                Some(value) => Some(value.to_string()),
                None => {
                    refused.note("temperature: province has no reconstructed slot", &identity);
                    None
                }
            },
            Err(reason) => {
                refused.note(format!("temperature: {reason}"), &identity);
                None
            }
        };
        reply.provinces.push(RegionalWeatherProvince {
            id: id.to_string(),
            name,
            snow,
            mud,
            temperature,
        });
    }
    reply
        .warnings
        .extend(refused.counts("weather province fields"));
    finish(query, reply)
}

#[cfg(test)]
mod tests;
