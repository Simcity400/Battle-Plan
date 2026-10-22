//! Regional weather, stored as exact coalesced hourly runs. Names are
//! `<regionId>|<flag>`, `<regionId>|mod:<modifier>` and `<regionId>|temp`.
//! Temperature is an app average of all member provinces in fixed5 degrees
//! Celsius. Every flag byte, including zero, is preserved independently.
//! An absent modifier run is unavailable, never an implicit zero. Known
//! missing accumulator keys have explicit zero runs.
//!
//! With `daily: true`, runs sample each calendar day's last reached recorded
//! hour, not its last *known weather* hour, and cover that day within the
//! accepted window. Equal consecutive days coalesce. Supplying reads date
//! acquisition, not a native per-region weather update. Optional theatre choices
//! describe a reached weather hour, not historical theatre membership.
//! `view: "provinces"` reads one region's members at an exact recorded hour
//! through the shared frame cache, independently of the history prefix.

use dashboard_base::{
    prepared::query::progressive,
    products::{Product, ReplyWarnings},
};
use dashboard_keyed::{
    daily,
    query::{self as keyed, KeyedSeries, KeyedWindow, Query},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};
use ts_rs::TS;

pub mod block;
mod columns;
mod content;
mod provinces;
mod temperature;
#[cfg(test)]
mod tests;

pub struct RegionalWeather;

pub const TEMPERATURE_REFUSAL: &str = "Temperature unavailable: preparation has not bound the recorded content stack; call regional_weather::block::Observer::bind before folding each branch";
/// Raw temperature units per degree Celsius (fixed5).
pub const TEMPERATURE_SCALE: i64 = replay_data::temperature::SCALE;
/// Raw modifier entries use the recording's fixed5 units, without clamping,
/// effective country factors, or conversion into a generic weather penalty.
pub const MODIFIER_SCALE: i64 = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionalWeatherParams {
    pub branch_id: String,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub regions: Option<Vec<String>>,
    pub show: Option<RegionalWeatherShow>,
    pub modifier: Option<String>,
    pub daily: Option<bool>,
    #[serde(default)]
    pub view: RegionalWeatherView,
    #[ts(optional)]
    pub region: Option<String>,
    #[ts(optional)]
    pub hour: Option<i32>,
    /// Daily history folded into at most this many columns: the viewport's pixel columns.
    #[ts(optional)]
    pub width: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
pub enum RegionalWeatherView {
    #[default]
    History,
    Provinces,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
pub enum RegionalWeatherShow {
    Conditions,
    Modifier,
    Temperature,
}

impl RegionalWeatherParams {
    fn selects(&self, measure: &str) -> bool {
        match self.show {
            None => true,
            Some(RegionalWeatherShow::Conditions) => {
                content::FLAGS.iter().any(|(id, _, _)| *id == measure)
            }
            Some(RegionalWeatherShow::Temperature) => measure == "temp",
            Some(RegionalWeatherShow::Modifier) => {
                measure.strip_prefix("mod:") == self.modifier.as_deref()
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "RegionalWeatherChoice")]
pub struct Choice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "RegionalWeatherTheatre")]
pub struct Theatre {
    pub id: String,
    pub name: String,
    /// Decimal strategic-region ids, as in the region choices.
    pub regions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RegionalWeatherReply {
    pub window: KeyedWindow,
    pub day_hours: i32,
    pub regions: Vec<Choice>,
    pub flags: Vec<Choice>,
    pub modifiers: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub theatres: Option<Vec<Theatre>>,
    /// `available` when preparation bound the recorded temperature content.
    /// Missing/refused hours still have no temperature run.
    pub temperature: String,
    /// With `width`, the days folded into columns; the window then keeps no runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub columns: Option<columns::Columns>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RegionalWeatherProvince {
    pub id: String,
    pub name: Option<String>,
    /// The stored snow word, fixed5; permanent snow is not added.
    pub snow: Option<String>,
    /// Whether the game shows mud: the raw mud byte is nonzero (the
    /// separate display mud byte never stands in).
    pub mud: Option<bool>,
    /// Checksum-proven reconstructed fixed5 degrees Celsius.
    pub temperature: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RegionalWeatherProvincesReply {
    pub hour: i32,
    pub read_hour: Option<i32>,
    pub region: String,
    pub provinces: Vec<RegionalWeatherProvince>,
    pub warnings: Vec<String>,
}

/// The reply of either view, tagged with the view that was asked for.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(tag = "view", rename_all = "lowercase")]
pub enum RegionalWeatherResponse {
    History(RegionalWeatherReply),
    Provinces(RegionalWeatherProvincesReply),
}

impl ReplyWarnings for RegionalWeatherResponse {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::History(reply) => &mut reply.warnings,
            Self::Provinces(reply) => &mut reply.warnings,
        }
    }
}

impl Product for RegionalWeather {
    const KEY: &'static str = "regional-weather";
    const CONTENT_DEPENDENT: bool = true;
    type Params = RegionalWeatherParams;
    type Reply = RegionalWeatherResponse;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        keyed::validate_window(&params.branch_id, params.first_hour, params.last_hour)?;
        for region in params.regions.iter().flatten().chain(params.region.iter()) {
            let id = region
                .parse::<u32>()
                .map_err(|_| "Invalid weather region")?;
            if id.to_string() != *region {
                return Err("Invalid weather region");
            }
        }
        if params.view == RegionalWeatherView::Provinces
            && (params.region.is_none() || params.hour.is_none())
        {
            return Err("Province weather view needs region and hour");
        }
        if params
            .modifier
            .as_deref()
            .is_some_and(|selected| !content::MODIFIERS.iter().any(|(id, _, _)| *id == selected))
        {
            return Err("Invalid weather modifier");
        }
        if params.show == Some(RegionalWeatherShow::Modifier) && params.modifier.is_none() {
            return Err("A weather modifier is required");
        }
        if params
            .width
            .is_some_and(|width| width == 0 || params.daily != Some(true))
        {
            return Err("Weather columns need daily samples and a width");
        }
        Ok(())
    }

    fn types() -> Vec<String> {
        vec![
            Self::Params::decl(),
            Self::Reply::decl(),
            RegionalWeatherReply::decl(),
            RegionalWeatherProvincesReply::decl(),
            RegionalWeatherProvince::decl(),
            RegionalWeatherView::decl(),
            RegionalWeatherShow::decl(),
            Choice::decl(),
            Theatre::decl(),
            columns::Columns::decl(),
            columns::Column::decl(),
            columns::ColumnRegion::decl(),
            columns::ColumnMeasure::decl(),
            KeyedWindow::decl(),
            KeyedSeries::decl(),
            dashboard_base::products::country_groups::HourSpan::decl(),
        ]
    }
}

/// Region and condition label checks over the recorded content stack. Modifier
/// titles, theatre choices and temperature pass/anchor evidence need a recording.
pub fn content_warnings(
    layers: &installed_content_core::ResolvedLayers,
    language: &str,
) -> io::Result<Vec<String>> {
    let mut warnings = content::content_warnings(layers, language)?;
    warnings.extend(temperature::content_warnings(layers));
    Ok(warnings)
}

/// Prepared weather runs, restricted to the requested regions. A cold query
/// returns the same reply contract with an empty window; the caller marks the
/// accepted prefix and pending tail on the root object.
pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    let params = keyed::params::<RegionalWeather>(query)?;
    if params.view == RegionalWeatherView::Provinces {
        return provinces::query(query, &params);
    }
    let selected = params.regions.as_ref().map(|ids| {
        ids.iter()
            .map(|id| id.parse::<u32>().expect("validated region"))
            .collect::<BTreeSet<_>>()
    });
    let (first, last) = query.window()?;
    let daily = params.daily.unwrap_or(false);
    let read_first = if daily && first <= last {
        daily::day_start(first)
    } else {
        first
    };
    let mut regions = BTreeSet::new();
    if let Some(selected) = &selected {
        regions.extend(selected);
    }
    let mut window = keyed::empty(first, last);
    let mut temperature = TEMPERATURE_REFUSAL.to_owned();
    let mut warnings = Vec::new();
    if let Some(mut opened) = query.open::<block::Observer>()? {
        temperature = opened.scan.observer.temperature_status().to_owned();
        warnings.extend(opened.scan.observer.temperature_warnings());
        for name in opened.scan.names() {
            if let Some((region, _)) = split_name(name) {
                regions.insert(region);
            }
        }
        window = opened.window(read_first, last, query.through(), |name| {
            split_name(name).is_some_and(|(region, measure)| {
                selected.as_ref().is_none_or(|set| set.contains(&region))
                    && (params.selects(measure)
                        // A flag witnesses region presence for a modifier-only
                        // request; discard it after counting missing witnesses.
                        || (params.show == Some(RegionalWeatherShow::Modifier)
                            && measure == "light_rain"))
            })
        })?;
    }
    // Keep the actual reached hour before daily spans expand towards the start
    // of their calendar day. Never seek a projected (possibly unrecorded) hour.
    let auxiliary_hour = window.known.last().map(|span| span.last_hour);
    if daily {
        let through = query.through().map_or(last, |through| last.min(through));
        if first > through {
            window = keyed::empty(first, last);
        }
        daily::Days::requested(query, first, last).sample(&mut window, query.cancelled)?;
        window.first_hour = first;
        window.last_hour = last;
    }
    if temperature != "available" {
        warnings.push(format!("1 weather measure not shown: {temperature}"));
    }
    modifier_refusals(&window, &params, &mut warnings);
    window.series.retain(|series| {
        split_name(&series.name).is_some_and(|(_, measure)| params.selects(measure))
    });
    let labels = content::labels(query, &mut warnings);
    if let Ok(names) = dashboard_keyed::regions::RegionNames::read(query) {
        regions.extend(names.ids());
    }
    let region_choices = content::region_choices(&regions, query, &mut warnings);
    let flags = content::flag_choices(labels.as_deref(), &mut warnings);
    // One reached metadata frame, shared by the frame cache. The numerical
    // history above never seeks frames or reconstructs the branch on request.
    let auxiliary = match auxiliary_hour {
        Some(hour) => match progressive::frame_at(
            query.source,
            query.branch,
            hour,
            content::AUXILIARY_TABLES,
            query.cancelled,
        ) {
            Ok(frame) => frame,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(error);
            }
            Err(error) => {
                warnings.push(format!("1 weather choice snapshot unavailable: {error}"));
                None
            }
        },
        None => None,
    };
    let modifiers = content::modifier_choices(
        auxiliary.as_deref(),
        labels.as_deref(),
        query,
        &mut warnings,
    );
    let theatres = content::theatres(
        auxiliary.as_deref(),
        labels.as_deref(),
        query,
        &mut warnings,
    );
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    let folded = match params.width {
        Some(width) => {
            let conditions = matches!(params.show, None | Some(RegionalWeatherShow::Conditions));
            let folded = columns::fold(&window, width, conditions)?;
            window.known.clear();
            window.reads.clear();
            window.series.clear();
            Some(folded)
        }
        None => None,
    };
    let reply = RegionalWeatherReply {
        window,
        day_hours: daily::DAY_HOURS,
        regions: region_choices,
        flags,
        modifiers,
        theatres,
        temperature,
        columns: folded,
        warnings,
    };
    keyed::finish(query, &RegionalWeatherResponse::History(reply))
}

/// Names: `<regionId>|<flag>`, `<regionId>|mod:<modifier>` or `<regionId>|temp`. Flags and
/// modifier identifiers are fixed recorded field identities, not content names.
fn split_name(name: &str) -> Option<(u32, &str)> {
    let (region, measure) = name.split_once('|')?;
    Some((region.parse().ok()?, measure))
}

fn modifier_refusals(
    window: &KeyedWindow,
    params: &RegionalWeatherParams,
    warnings: &mut Vec<String>,
) {
    if matches!(
        params.show,
        Some(RegionalWeatherShow::Conditions | RegionalWeatherShow::Temperature)
    ) {
        return;
    }
    let by_name: std::collections::HashMap<_, _> = window
        .series
        .iter()
        .map(|series| (series.name.as_str(), series))
        .collect();
    let mut refused = dashboard_base::catalogue::Refusals::default();
    for presence in window
        .series
        .iter()
        .filter(|series| series.name.ends_with("|light_rain"))
    {
        let Some((region, _)) = split_name(&presence.name) else {
            continue;
        };
        for (modifier, _, _) in content::MODIFIERS {
            if params.show == Some(RegionalWeatherShow::Modifier)
                && params.modifier.as_deref() != Some(modifier)
            {
                continue;
            }
            let name = format!("{region}|mod:{modifier}");
            let runs = by_name
                .get(name.as_str())
                .map_or(&[][..], |series| series.runs.as_slice());
            let mut at = 0;
            let uncovered = presence.runs.iter().any(|span| {
                let mut hour = i64::from(span.0);
                while at < runs.len() && runs[at].1 < span.0 {
                    at += 1;
                }
                while at < runs.len() && i64::from(runs[at].0) <= hour {
                    hour = hour.max(i64::from(runs[at].1) + 1);
                    if hour > i64::from(span.1) {
                        break;
                    }
                    at += 1;
                }
                hour <= i64::from(span.1)
            });
            if uncovered {
                refused.note("regional modifier witness unavailable", name);
            }
        }
    }
    warnings.extend(refused.counts("weather modifier histories"));
}
