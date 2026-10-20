//! Content-bound choices. Region and condition names use the recorded stack;
//! modifier titles use the recorded definition's key and argument, not a guessed
//! localisation key. Missing titles retain project wording and counted warnings.

use super::{Choice, Theatre};
use dashboard_base::{
    catalogue::{self, KeyedResults, Refusals, View},
    content,
};
use dashboard_keyed::query::Query;
use engine_executable::modifier_keys;
use installed_content_localisation::localisation::{Argument, Localisation};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

pub(super) const AUXILIARY_TABLES: &[TableId] = &[
    TableId::ModifierDefinitions,
    TableId::AirWings,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
];

// Native recorded fields and their native localisation selectors; mods supply
// the text. The last column is project wording only when that text is unavailable.
pub(super) const FLAGS: [(&str, &str, &str); 6] = [
    ("light_rain", "weather_rain_light", "Light rain"),
    ("heavy_rain", "weather_rain_heavy", "Heavy rain"),
    ("snow", "weather_snow", "Snow"),
    ("blizzard", "weather_blizzard", "Blizzard"),
    ("sandstorm", "weather_sandstorm", "Sandstorm"),
    ("arctic_water", "weather_arctic_water", "Arctic water"),
];

pub(super) const MODIFIERS: [(&str, u32, &str); 8] = [
    (
        "air_detection",
        modifier_keys::REGION_AIR_DETECTION,
        "Raw air detection modifier",
    ),
    (
        "naval_detection",
        modifier_keys::NAVAL_DETECTION,
        "Raw naval detection modifier",
    ),
    (
        "air_attack",
        modifier_keys::AIR_ATTACK_FACTOR,
        "Raw air attack modifier",
    ),
    (
        "air_defence",
        modifier_keys::AIR_DEFENCE_FACTOR,
        "Raw air defence modifier",
    ),
    (
        "army_attack_factor",
        modifier_keys::ARMY_ATTACK_FACTOR,
        "Raw army attack modifier",
    ),
    (
        "breakthrough_factor",
        modifier_keys::BREAKTHROUGH_FACTOR,
        "Raw breakthrough modifier",
    ),
    (
        "army_speed_factor",
        modifier_keys::ARMY_SPEED_FACTOR,
        "Raw army speed modifier",
    ),
    (
        "army_attack_speed_factor",
        modifier_keys::ARMY_ATTACK_SPEED_FACTOR,
        "Raw army attack speed modifier",
    ),
];

pub(super) struct Labels {
    pub localisation: Arc<Localisation>,
    warnings: Vec<String>,
}

pub(super) fn labels(query: &Query<'_>, warnings: &mut Vec<String>) -> Option<Arc<Labels>> {
    let result = (|| {
        let options = query
            .content
            .ok_or_else(|| dashboard_core::invalid("game folders are not chosen"))?;
        let key = content::localisation_key(options)?;
        query
            .source
            .state::<KeyedResults<content::LocalisationKey, Labels>>()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_or_init(&key, |_| {
                dashboard_text::metadata::check_cancel(query.cancelled)?;
                let mut notes = Vec::new();
                content::check_replay(
                    query.source.replay_ref(),
                    options.allow_unverified,
                    &mut notes,
                )?;
                let localisation = content::localisation(query.source, options)?;
                Ok(Labels {
                    localisation,
                    warnings: notes,
                })
            })
    })();
    match result {
        Ok(labels) => {
            warnings.extend(labels.warnings.iter().cloned());
            Some(labels)
        }
        Err(error) => {
            warnings.push(format!("1 weather content catalogue unavailable: {error}"));
            None
        }
    }
}

/// The actual query readers, reused by the lead's content-only product check.
/// Modifier titles and theatre membership additionally require recorded rows.
pub(super) fn content_warnings(
    layers: &installed_content_core::ResolvedLayers,
    language: &str,
) -> std::io::Result<Vec<String>> {
    let loc = Arc::new(installed_content_localisation::localisation::read(
        layers, language,
    )?);
    let labels = Labels {
        localisation: loc,
        warnings: Vec::new(),
    };
    let mut warnings = labels.warnings.clone();
    flag_choices(Some(&labels), &mut warnings);
    Ok(warnings)
}

pub(super) fn region_choices(
    ids: &BTreeSet<u32>,
    query: &Query<'_>,
    warnings: &mut Vec<String>,
) -> Vec<Choice> {
    let names = dashboard_keyed::regions::RegionNames::read(query);
    if let Ok(names) = &names {
        warnings.extend(names.warnings.iter().cloned());
    }
    let mut refused = Refusals::default();
    let choices = dashboard_keyed::regions::choices(&names, ids, &mut refused);
    warnings.extend(refused.counts("weather region names"));
    choices
        .into_iter()
        .map(|choice| Choice {
            id: choice.id,
            name: choice.name,
        })
        .collect()
}

pub(super) fn flag_choices(labels: Option<&Labels>, warnings: &mut Vec<String>) -> Vec<Choice> {
    let mut refused = Refusals::default();
    let choices = FLAGS
        .into_iter()
        .map(|(id, key, fallback)| {
            let name = labels
                .ok_or_else(|| "condition localisation unavailable".to_owned())
                .and_then(|labels| named(&labels.localisation, key, &[]))
                .unwrap_or_else(|why| {
                    refused.note(why, id);
                    fallback.into()
                });
            Choice {
                id: id.into(),
                name,
            }
        })
        .collect();
    warnings.extend(refused.counts("weather condition labels (project wording used)"));
    choices
}

fn named(
    loc: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    if !key.is_empty() && loc.get(key).is_none() {
        return Err(format!(
            "localised title {key} absent from the selected language"
        ));
    }
    catalogue::localised_with(loc, key, arguments, View::Plain)
}

pub(super) fn modifier_choices(
    frame: Option<&DecodedFrame>,
    labels: Option<&Labels>,
    query: &Query<'_>,
    warnings: &mut Vec<String>,
) -> Vec<Choice> {
    let definitions = frame.and_then(|frame| frame.complete::<tables::ModifierDefinitions>().ok());
    let by_index: HashMap<_, _> = definitions
        .unwrap_or_default()
        .iter()
        .map(|row| (row.index, row))
        .collect();
    let mut refused = Refusals::default();
    let choices = MODIFIERS
        .into_iter()
        .map(|(id, index, fallback)| {
            let name = (|| {
                let labels = labels.ok_or("modifier localisation unavailable")?;
                let definition = by_index
                    .get(&index)
                    .ok_or("modifier name definition not recorded")?;
                // Localisation uses C-string keys. The key's byte suffix is not a
                // second name and is never decoded as text.
                let key = definition
                    .name_key
                    .split(|byte| *byte == 0)
                    .next()
                    .unwrap_or_default();
                let key = std::str::from_utf8(key).map_err(|_| "modifier name key is not UTF-8")?;
                let language = &query.source.replay_ref().identity.language;
                let argument = dashboard_text::scripted_text::stored_text(
                    &labels.localisation,
                    language.strip_prefix("l_").unwrap_or(language),
                    &definition.argument,
                    "weather modifier argument",
                )?;
                named(
                    &labels.localisation,
                    key,
                    &[("ARG", Argument::Text(&argument))],
                )
            })()
            .unwrap_or_else(|why: String| {
                refused.note(why, id);
                fallback.into()
            });
            Choice {
                id: id.into(),
                name,
            }
        })
        .collect();
    warnings.extend(refused.counts("weather modifier labels (project wording used)"));
    choices
}

/// As in air-superiority: theatre -> group -> member wing -> mission region.
/// Complete tables prove empty theatres; a missing join refuses the membership,
/// never substitutes a partial, complete-looking region list.
pub(super) fn theatres(
    frame: Option<&DecodedFrame>,
    labels: Option<&Labels>,
    query: &Query<'_>,
    warnings: &mut Vec<String>,
) -> Option<Vec<Theatre>> {
    let result = frame
        .ok_or_else(|| "no accepted weather hour for theatre choices".to_owned())
        .and_then(|frame| {
            theatre_membership(
                frame,
                labels.map(|labels| labels.localisation.as_ref()),
                &query.source.replay_ref().identity.language,
                warnings,
            )
        });
    match result {
        Ok(theatres) => Some(theatres),
        Err(why) => {
            warnings.push(format!("1 weather air-theatre filter not shown: {why}"));
            None
        }
    }
}

pub(super) fn theatre_membership(
    frame: &DecodedFrame,
    localisation: Option<&Localisation>,
    recorded_language: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<Theatre>, String> {
    let theatres = frame
        .complete::<tables::AirTheatres>()
        .map_err(|why| why.to_string())?;
    let mut membership = dashboard_air_operations::theatres::regions(frame)?;
    let mut refused = Refusals::default();
    let choices = theatres
        .iter()
        .map(|theatre| {
            let name = localisation
                .ok_or_else(|| "air theatre name localisation unavailable".to_owned())
                .and_then(|loc| {
                    dashboard_text::scripted_text::stored_text(
                        loc,
                        recorded_language
                            .strip_prefix("l_")
                            .unwrap_or(recorded_language),
                        &theatre.name,
                        "Air theatre name",
                    )
                })
                .and_then(|name| {
                    if name.is_empty() {
                        Err("Air theatre name is empty".into())
                    } else {
                        Ok(name)
                    }
                })
                .unwrap_or_else(|why| {
                    refused.note(why, theatre.id.to_string());
                    "Air theatre name unavailable".to_owned()
                });
            Theatre {
                id: theatre.id.to_string(),
                name,
                regions: membership
                    .remove(&theatre.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect(),
            }
        })
        .collect();
    warnings.extend(refused.counts("weather air-theatre names"));
    Ok(choices)
}
