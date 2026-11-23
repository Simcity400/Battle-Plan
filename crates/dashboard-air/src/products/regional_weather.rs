
use dashboard_base::products::{LoadContext, Product};
use dashboard_world::products::place_names;
use dashboard_base::{Source, catalogue::{self, Catalogue, KeyedResults, Refusals, View}, products::at_hour::{At, AtHour, Read}};
use dashboard_query::{Query, read};
use engine_executable::modifier_keys;
use content_reader_core::province_graph::ProvinceGraph;
use content_reader_localisation::localisation::Argument;
use replay_columnar::summary;
use replay_data::{
    rows::{Recorded, TableId, WeatherProvinceRow, WeatherRegionRow, tables},
    temperature::{Content, Memo, Plan, Planned, State},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub const DAY_HOURS: i32 = 24;

pub(crate) const FLAGS: [(&str, &str, &str); 6] = [
    ("light_rain", "weather_rain_light", "Light rain"),
    ("heavy_rain", "weather_rain_heavy", "Heavy rain"),
    ("snow", "weather_snow", "Snow"),
    ("blizzard", "weather_blizzard", "Blizzard"),
    ("sandstorm", "weather_sandstorm", "Sandstorm"),
    ("arctic_water", "weather_arctic_water", "Arctic water"),
];

pub(crate) const MODIFIERS: [(&str, u32, &str); 8] = [
    ("air_detection", modifier_keys::REGION_AIR_DETECTION, "Raw air detection modifier"),
    ("naval_detection", modifier_keys::NAVAL_DETECTION, "Raw naval detection modifier"),
    ("air_attack", modifier_keys::AIR_ATTACK_FACTOR, "Raw air attack modifier"),
    ("air_defence", modifier_keys::AIR_DEFENCE_FACTOR, "Raw air defence modifier"),
    ("army_attack_factor", modifier_keys::ARMY_ATTACK_FACTOR, "Raw army attack modifier"),
    ("breakthrough_factor", modifier_keys::BREAKTHROUGH_FACTOR, "Raw breakthrough modifier"),
    ("army_speed_factor", modifier_keys::ARMY_SPEED_FACTOR, "Raw army speed modifier"),
    ("army_attack_speed_factor", modifier_keys::ARMY_ATTACK_SPEED_FACTOR, "Raw army attack speed modifier"),
];

pub struct RegionalWeather;
pub struct WeatherProvinces;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "RegionalWeatherShow"))]
pub enum Show {
    Conditions,
    Modifier,
    Temperature,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionalWeatherParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The most columns the days fold into: the view's pixel columns."]
    pub width: u32,
    pub show: Show,
    #[doc = " The modifier shown; the first recorded one without."]
    #[serde(default)]
    pub modifier: Option<String>,
    #[doc = " Decimal region ids; every region without."]
    #[serde(default)]
    pub regions: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "RegionalWeatherChoice"))]
pub struct Choice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "RegionalWeatherColumn"))]
pub struct Column {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The latest hour its days were sampled at; none without one."]
    pub sampled_hour: Option<i32>,
    #[doc = " The latest read of WeatherRegions at or before a sampled day's hour."]
    pub read_hour: Option<i32>,
    #[doc = " Its sampled days."]
    pub recorded: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "RegionalWeatherColumnMeasure"))]
pub struct ColumnMeasure {
    #[doc = " `<flag>`, `mod:<modifier>` or `temp`."]
    pub id: String,
    #[doc = " Sampled days with the flag set, or with a value, per column."]
    pub days: Vec<u32>,
    #[doc = " Values: each column's lowest raw value, the exact sum of its days' and its highest, as"]
    #[doc = " decimal text; none without a value."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub values: Option<Vec<Option<[String; 3]>>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "RegionalWeatherColumnRegion"))]
pub struct ColumnRegion {
    pub id: String,
    #[doc = " Conditions: the days the region's conditions were recorded, per column."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub observed: Option<Vec<u32>>,
    #[doc = " Conditions: those days with no condition set, per column."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub clear: Option<Vec<u32>>,
    #[doc = " A flag set on no sampled day, or a value on none, is left out."]
    pub measures: Vec<ColumnMeasure>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "RegionalWeatherColumns"))]
pub struct Columns {
    #[doc = " Days per column, counted from the window's first day."]
    pub days: u32,
    pub columns: Vec<Column>,
    pub regions: Vec<ColumnRegion>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RegionalWeatherReply {
    pub first_hour: i32,
    pub last_hour: i32,
    pub day_hours: i32,
    pub regions: Vec<Choice>,
    pub flags: Vec<Choice>,
    pub modifiers: Vec<Choice>,
    #[doc = " `available`, or why no temperature is shown."]
    pub temperature: String,
    pub columns: Columns,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(RegionalWeatherReply);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionalWeatherProvincesParams {
    pub branch_id: String,
    pub region: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RegionalWeatherProvince {
    pub id: String,
    pub name: Option<String>,
    #[doc = " The stored snow word, fixed5; permanent snow is not added."]
    pub snow: Option<String>,
    #[doc = " Whether the game shows mud: the raw mud byte is nonzero."]
    pub mud: Option<bool>,
    #[doc = " The replayed temperature, fixed5 degrees Celsius."]
    pub temperature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RegionalWeatherProvincesReply {
    pub read_hour: Option<i32>,
    pub region: String,
    pub provinces: Vec<RegionalWeatherProvince>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(RegionalWeatherProvincesReply);

pub struct Weather {
    members: BTreeMap<u32, Vec<u32>>,
    names: BTreeMap<u32, Result<String, String>>,
    pass: Result<(Content, Plan), String>,
    cat: Arc<Catalogue>,
    warnings: Vec<String>,
}

fn weather(source: &Source, load: &LoadContext) -> io::Result<Arc<Weather>> {
    let options = load.content()?;
    let cat = catalogue::catalogue(source, options)?;
    source.state::<KeyedResults<catalogue::RecordedBinding, Weather>>().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_init(&catalogue::recorded_binding(options), |_| {
        let layers = &cat.content.layers;
        let mut warnings: Vec<String> = cat.verification.iter().chain(cat.warnings.iter()).cloned().collect();
        let regions = dashboard_base::content::strategic_regions(layers)?;
        warnings.extend(regions.notes.iter().cloned());
        let names = regions.regions.keys().map(|&id| (id, dashboard_world::products::region_names::region_name(&cat, &regions, id))).collect();
        let (members, pass) = match ProvinceGraph::read(layers) {
            Err(error) => {
                warnings.push(format!("1 weather province membership not shown: {error}"));
                (BTreeMap::new(), Err(format!("Temperature unavailable: {error}")))
            }
            Ok((graph, geometry)) => {
                let members = regions.regions.iter().map(|(&id, region)| (id, region.native_provinces(|province| geometry.centroids.contains_key(&province)))).collect();
                let pass = content_reader_weather::weather::temperature::read_with(layers, &graph, &geometry)
                    .map(|pass| {
                        let plan = Plan::new(&pass);
                        (pass, plan)
                    })
                    .map_err(|error| format!("Temperature unavailable: {error}"));
                (members, pass)
            }
        };
        Ok(Weather { members, names, pass, cat: Arc::clone(&cat), warnings })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Day {
    pub first: i32,
    pub last: i32,
    pub at: Option<(u32, i32)>,
    pub read: Option<i32>,
}

pub(crate) fn days(first: i32, last: i32, known: &[(u32, i32, i32)], reads: &[(i32, i32)]) -> Vec<Day> {
    if first > last {
        return Vec::new();
    }
    (first.div_euclid(DAY_HOURS)..=last.div_euclid(DAY_HOURS))
        .map(|day| {
            let (from, to) = ((day * DAY_HOURS).max(first), (day * DAY_HOURS + DAY_HOURS - 1).min(last));
            let at = known.iter().filter(|span| span.1 <= to && span.2 >= from).last().map(|span| (span.0, span.2.min(to)));
            let read = at.and_then(|(_, at)| {
                let index = reads.partition_point(|run| run.0 <= at).checked_sub(1)?;
                Some(reads[index].1.min(at))
            });
            Day { first: from, last: to, at, read }
        })
        .collect()
}

type Daily = BTreeMap<u32, Vec<Option<i128>>>;

pub(crate) fn fold(days: &[Day], width: u32, measures: &[(String, Daily)], conditions: bool) -> io::Result<Columns> {
    let size = days.len().div_ceil(width.max(1) as usize).max(1);
    let columns = days
        .chunks(size)
        .map(|chunk| Column {
            first_hour: chunk[0].first,
            last_hour: chunk[chunk.len() - 1].last,
            sampled_hour: chunk.iter().filter_map(|day| day.at.map(|(_, at)| at)).max(),
            read_hour: chunk.iter().filter_map(|day| day.read).max(),
            recorded: chunk.iter().filter(|day| day.at.is_some()).count() as u32,
        })
        .collect();
    let mut regions: BTreeMap<u32, (Vec<bool>, Vec<bool>, Vec<ColumnMeasure>)> = BTreeMap::new();
    for (id, daily) in measures {
        let flag = FLAGS.iter().any(|(flag, _, _)| flag == id);
        for (&region, values) in daily {
            let held = regions.entry(region).or_insert_with(|| (vec![false; days.len()], vec![false; days.len()], Vec::new()));
            let measure = if flag {
                for (day, value) in values.iter().enumerate() {
                    held.0[day] |= value.is_some();
                    held.1[day] |= value.is_some_and(|value| value != 0);
                }
                ColumnMeasure { id: id.clone(), days: values.chunks(size).map(|chunk| chunk.iter().filter(|value| value.is_some_and(|value| value != 0)).count() as u32).collect(), values: None }
            } else {
                let mut counts = Vec::new();
                let stats = values
                    .chunks(size)
                    .map(|chunk| {
                        let held: Vec<i128> = chunk.iter().flatten().copied().collect();
                        counts.push(held.len() as u32);
                        let (Some(low), Some(high)) = (held.iter().min(), held.iter().max()) else { return Ok(None) };
                        let sum = held.iter().try_fold(0_i128, |sum, value| sum.checked_add(*value)).ok_or_else(|| dashboard_core::invalid("Weather column sum exceeds its exact range"))?;
                        Ok(Some([low.to_string(), sum.to_string(), high.to_string()]))
                    })
                    .collect::<io::Result<_>>()?;
                ColumnMeasure { id: id.clone(), days: counts, values: Some(stats) }
            };
            if measure.days.iter().any(|days| *days > 0) {
                held.2.push(measure);
            }
        }
    }
    let per_column = |keep: &dyn Fn(usize) -> bool| -> Vec<u32> { (0..days.len()).collect::<Vec<_>>().chunks(size).map(|chunk| chunk.iter().filter(|&&day| keep(day)).count() as u32).collect() };
    let regions = regions
        .into_iter()
        .map(|(id, (observed, set, measures))| ColumnRegion {
            id: id.to_string(),
            observed: conditions.then(|| per_column(&|day| observed[day])),
            clear: conditions.then(|| per_column(&|day| observed[day] && !set[day])),
            measures,
        })
        .collect();
    Ok(Columns { days: size as u32, columns, regions })
}

pub(crate) fn modifiers(row: &WeatherRegionRow) -> [Option<i128>; 8] {
    let mut values = [None; 8];
    if let Recorded::Value(Some(value)) = row.air_detection {
        values[0] = Some(i128::from(value.unwrap_or(0)));
    }
    if row.map_modifiers_known == Recorded::Value(true) {
        for (at, value) in [row.naval_detection, row.air_attack, row.air_defence].into_iter().enumerate() {
            if let Recorded::Value(value) = value {
                values[1 + at] = Some(i128::from(value.unwrap_or(0)));
            }
        }
    }
    if let Recorded::Value(Some(combat)) = &row.combat {
        for (at, value) in [combat.army_attack_factor, combat.breakthrough_factor, combat.army_speed_factor, combat.army_attack_speed_factor].into_iter().enumerate() {
            values[4 + at] = Some(i128::from(value.unwrap_or(0)));
        }
    }
    values
}

pub(crate) fn mean(members: &[u32], states: &[State]) -> Option<i128> {
    if members.is_empty() {
        return None;
    }
    let sum = members.iter().try_fold(0_i128, |sum, &id| states.get(id as usize).map(|state| sum + i128::from(state.temperature)))?;
    Some(sum / members.len() as i128)
}

fn states<'m>(memo: &'m mut Memo, weather: &Weather, branch: u64, hour: i32, rows: &[WeatherProvinceRow]) -> Result<&'m [State], String> {
    let (pass, plan) = weather.pass.as_ref().map_err(Clone::clone)?;
    let states = memo.states_changed(branch, hour, rows, Ok(&Planned::new(pass, plan)), None)?;
    if states.len() != pass.provinces.len() {
        return Err(format!("content has {} weather province slots, the recording {}", pass.provinces.len(), states.len()));
    }
    Ok(states)
}

fn named(cat: &Catalogue, key: &str, arguments: &[(&str, Argument<'_>)]) -> Result<String, String> {
    if !key.is_empty() && cat.localisation.get(key).is_none() {
        return Err(format!("localised title {key} absent from the selected language"));
    }
    catalogue::localised_with(&cat.localisation, key, arguments, View::Plain)
}

fn flag_choices(cat: &Catalogue, warnings: &mut Vec<String>) -> Vec<Choice> {
    let mut refused = Refusals::default();
    let choices = FLAGS
        .into_iter()
        .map(|(id, key, fallback)| Choice {
            id: id.into(),
            name: named(cat, key, &[]).unwrap_or_else(|why| {
                refused.note(why, id);
                fallback.into()
            }),
        })
        .collect();
    warnings.extend(refused.counts("weather condition labels (project wording used)"));
    choices
}

fn modifier_choices(frame: Option<&DecodedFrame>, cat: &Catalogue, warnings: &mut Vec<String>) -> Vec<Choice> {
    let definitions = frame.and_then(|frame| frame.complete::<tables::ModifierDefinitions>().ok());
    let by_index: HashMap<_, _> = definitions.unwrap_or_default().iter().map(|row| (row.index, row)).collect();
    let mut refused = Refusals::default();
    let choices = MODIFIERS
        .into_iter()
        .map(|(id, index, fallback)| {
            let name = (|| {
                let definition = by_index.get(&index).ok_or("modifier name definition not recorded")?;
                let key = definition.name_key.split(|byte| *byte == 0).next().unwrap_or_default();
                let key = std::str::from_utf8(key).map_err(|_| "modifier name key is not UTF-8")?;
                let argument = dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, &definition.argument, "weather modifier argument")?;
                named(cat, key, &[("ARG", Argument::Text(&argument))])
            })()
            .unwrap_or_else(|why: String| {
                refused.note(why, id);
                fallback.into()
            });
            Choice { id: id.into(), name }
        })
        .collect();
    warnings.extend(refused.counts("weather modifier labels (project wording used)"));
    choices
}

fn history(query: &Query<'_>, params: &RegionalWeatherParams, weather: &Weather, cancelled: &AtomicBool) -> io::Result<RegionalWeatherReply> {
    let recording = query.replay.recording();
    let (first, last) = (params.first_hour.max(query.timeline.first), params.last_hour.min(query.timeline.last));
    let mut warnings = weather.warnings.clone();
    let mut known: Vec<(u32, i32, i32)> = Vec::new();
    let mut reads: Vec<(i32, i32)> = Vec::new();
    let mut rows: BTreeSet<u32> = BTreeSet::new();
    let sampled = if params.show == Show::Temperature { TableId::WeatherProvinces } else { TableId::WeatherRegions };
    for stretch in query.timeline.within(first, last) {
        let (from, to) = (stretch.first.max(first), stretch.last.min(last));
        known.extend(read::statuses(recording, stretch.run, sampled, from, to)?.into_iter().filter(|run| replay_recording::columnar_frames::complete(run.status)).map(|run| (stretch.run, run.first, run.last)));
        reads.extend(dashboard_query::cost::storage(|| replay_lanes::lanes::reads(recording, stretch.run, sampled, from, to))?);
    }
    let days = days(first, last, &known, &reads);
    let wanted: Option<BTreeSet<u32>> = params.regions.as_ref().map(|ids| ids.iter().filter_map(|id| id.parse().ok()).collect());
    let keeps = |region: u32| wanted.as_ref().is_none_or(|wanted| wanted.contains(&region));
    let mut measures: Vec<(String, Daily)> = Vec::new();
    let temperature = weather.pass.as_ref().map(|_| "available".to_owned()).unwrap_or_else(Clone::clone);
    match params.show {
        Show::Conditions => {
            for (flag, _, _) in FLAGS {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let column = dashboard_declare::field(TableId::WeatherRegions, flag)?;
                let mut daily: Daily = BTreeMap::new();
                for stretch in query.timeline.within(first, last) {
                    let runs = read::runs(recording, stretch.run, TableId::WeatherRegions, summary::Source::Cell(column), None, stretch.first.max(first), stretch.last.min(last))?;
                    for (key, runs) in runs {
                        let region = key as u32;
                        rows.insert(region);
                        if !keeps(region) {
                            continue;
                        }
                        let values = daily.entry(region).or_insert_with(|| vec![None; days.len()]);
                        for (index, day) in days.iter().enumerate() {
                            let Some((run, at)) = day.at else { continue };
                            if run != stretch.run {
                                continue;
                            }
                            let held = runs.partition_point(|held| held.0 <= at).checked_sub(1).map(|at| runs[at]);
                            if let Some((_, end, Some(value))) = held
                                && at <= end
                            {
                                values[index] = Some(i128::from(value));
                            }
                        }
                    }
                }
                measures.push((flag.to_owned(), daily));
            }
        }
        Show::Modifier | Show::Temperature => {
            let modifier = params.modifier.as_deref().unwrap_or(MODIFIERS[0].0);
            let slot = MODIFIERS.iter().position(|(id, _, _)| *id == modifier).ok_or_else(|| dashboard_core::invalid("Invalid weather modifier"))?;
            let mut daily: Daily = BTreeMap::new();
            let mut memo = Memo::default();
            let mut refused = Refusals::default();
            let mut seeker = replay_lanes::at::Seeker::new(recording)?;
            for (index, day) in days.iter().enumerate() {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let Some((run, at)) = day.at else { continue };
                let frame = dashboard_query::cost::storage(|| seeker.frame(run, at, &[replay_lanes::at::Read::all(sampled)]))?;
                if params.show == Show::Modifier {
                    let Ok(held) = frame.complete::<tables::WeatherRegions>() else { continue };
                    for row in held {
                        rows.insert(row.id);
                        if keeps(row.id)
                            && let Some(value) = modifiers(row)[slot]
                        {
                            daily.entry(row.id).or_insert_with(|| vec![None; days.len()])[index] = Some(value);
                        }
                    }
                    continue;
                }
                let found = frame.complete::<tables::WeatherProvinces>().map_err(|error| error.to_string()).and_then(|held| states(&mut memo, weather, frame.branch, at, held));
                match found {
                    Err(why) => refused.note(why, format!("hour {at}")),
                    Ok(states) => {
                        for (&region, members) in weather.members.iter().filter(|(region, _)| keeps(**region)) {
                            rows.insert(region);
                            if let Some(value) = mean(members, states) {
                                daily.entry(region).or_insert_with(|| vec![None; days.len()])[index] = Some(value);
                            }
                        }
                    }
                }
            }
            if params.show == Show::Temperature {
                warnings.extend(refused.counts("weather temperature days"));
                measures.push(("temp".to_owned(), daily));
            } else {
                measures.push((format!("mod:{modifier}"), daily));
            }
        }
    }
    if first > last {
        let hour = if params.last_hour < query.timeline.first { query.timeline.first } else { query.timeline.last };
        if let Some(stretch) = query.timeline.at(hour)
            && let Ok(held) = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, stretch.run, hour, &[replay_lanes::at::Read::all(TableId::WeatherRegions)]))
            && let Ok(held) = held.complete::<tables::WeatherRegions>()
        {
            rows.extend(held.iter().map(|row| row.id));
        }
    }
    let columns = fold(&days, params.width, &measures, params.show == Show::Conditions)?;
    let shown = days.iter().rev().find_map(|day| day.at);
    let auxiliary = match shown {
        Some((run, at)) => dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, at, &[replay_lanes::at::Read::all(TableId::ModifierDefinitions)])).ok(),
        None => None,
    };
    let mut refused = Refusals::default();
    let regions = rows
        .iter()
        .filter(|region| weather.names.contains_key(region) && (weather.members.is_empty() || weather.members.get(region).is_some_and(|members| !members.is_empty())))
        .map(|&region| Choice {
            id: region.to_string(),
            name: match weather.names.get(&region) {
                Some(Ok(name)) => name.clone(),
                _ => {
                    refused.note(weather.names.get(&region).and_then(|name| name.as_ref().err()).cloned().unwrap_or_default(), region.to_string());
                    format!("Region {region}")
                }
            },
        })
        .collect();
    warnings.extend(refused.counts("weather region names"));
    let flags = flag_choices(&weather.cat, &mut warnings);
    let modifiers = modifier_choices(auxiliary.as_ref(), &weather.cat, &mut warnings);
    Ok(RegionalWeatherReply { first_hour: first, last_hour: last, day_hours: DAY_HOURS, regions, flags, modifiers, temperature, columns, warnings })
}

impl Product for RegionalWeather {
    const KEY: &'static str = "regional-weather";
    const CONTENT_DEPENDENT: bool = true;
    type Params = RegionalWeatherParams;
    type Reply = RegionalWeatherReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.first_hour > params.last_hour || params.width == 0 {
            return Err("Invalid weather window");
        }
        for region in params.regions.iter().flatten() {
            if region.parse::<u32>().map(|id| id.to_string() != *region).unwrap_or(true) {
                return Err("Invalid weather region");
            }
        }
        if params.modifier.as_deref().is_some_and(|chosen| !MODIFIERS.iter().any(|(id, _, _)| *id == chosen)) {
            return Err("Invalid weather modifier");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            RegionalWeatherParams::decl(),
            RegionalWeatherReply::decl(),
            Show::decl(),
            Choice::decl(),
            Columns::decl(),
            Column::decl(),
            ColumnRegion::decl(),
            ColumnMeasure::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let weather = weather(source, context)?;
        let reply = history(&query, params, &weather, cancelled)?;
        dashboard_base::products::check_reply_budget(&reply).map_err(|_| dashboard_core::invalid("Regional weather reply exceeds transport budget; nothing was truncated"))?;
        Ok(reply)
    }
}

pub struct ProvincesContext {
    weather: Result<Arc<Weather>, String>,
    places: Result<Arc<place_names::Content>, String>,
}

impl ProvincesContext {
    fn members(&self, region: &str) -> Option<&[u32]> {
        let weather = self.weather.as_ref().ok()?;
        weather.members.get(&region.parse::<u32>().ok()?).map(Vec::as_slice)
    }
}

fn province_name(frame: &DecodedFrame, places: &Result<Arc<place_names::Content>, String>, named: &place_names::Places<'_>, id: u32) -> Result<String, String> {
    let content = places.as_ref().map_err(Clone::clone)?;
    let name = named.province(content, id, false);
    name.name.ok_or_else(|| name.missing.unwrap_or_else(|| format!("province {id} has no name at {}", frame.hour)))
}

pub(crate) fn provinces(
    frame: &DecodedFrame,
    members: &[u32],
    name_of: &dyn Fn(u32) -> Result<String, String>,
    temperatures: Result<Vec<Option<i64>>, String>,
    warnings: &mut Vec<String>,
) -> Vec<RegionalWeatherProvince> {
    let weather = frame.complete::<tables::WeatherProvinces>();
    let rows: BTreeMap<u32, &WeatherProvinceRow> = weather.as_ref().ok().into_iter().flat_map(|rows| rows.iter()).map(|row| (row.id, row)).collect();
    let mut refused = Refusals::default();
    let mut out = Vec::with_capacity(members.len());
    for (at, &id) in members.iter().enumerate() {
        let identity = format!("province {id}");
        let name = name_of(id).map_err(|why| refused.note(format!("name: {why}"), &identity)).ok();
        let (snow, mud) = match rows.get(&id) {
            Some(row) => {
                let mud = row.raw_mud.require("raw mud of weather provinces").map(|&mud| mud != 0).map_err(|error| refused.note(format!("raw mud: {error}"), &identity)).ok();
                (Some(row.snow.to_string()), mud)
            }
            None => {
                let reason = weather.as_ref().err().map(ToString::to_string).unwrap_or_else(|| "province has no recorded weather row".into());
                refused.note(format!("snow and raw mud: {reason}"), &identity);
                (None, None)
            }
        };
        let temperature = match &temperatures {
            Ok(values) => values[at].map(|value| value.to_string()).or_else(|| {
                refused.note("temperature: province has no reconstructed slot", &identity);
                None
            }),
            Err(reason) => {
                refused.note(format!("temperature: {reason}"), &identity);
                None
            }
        };
        out.push(RegionalWeatherProvince { id: id.to_string(), name, snow, mud, temperature });
    }
    warnings.extend(refused.counts("weather province fields"));
    out
}

impl AtHour for WeatherProvinces {
    const KEY: &'static str = "regional-weather-provinces";
    const TITLE: &'static str = "Province weather";
    const CONTENT_DEPENDENT: bool = true;
    type Params = RegionalWeatherProvincesParams;
    type Reply = RegionalWeatherProvincesReply;
    type Context = ProvincesContext;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.region.parse::<u32>().map(drop).map_err(|_| "Invalid weather region")
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![RegionalWeatherProvincesParams::decl(), RegionalWeatherProvincesReply::decl(), RegionalWeatherProvince::decl()]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::WeatherProvinces];
        tables.extend(place_names::TABLES);
        tables
    }

    fn reads_in(params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame, at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let context = at.context;
        let members = context.members(&params.region).filter(|_| context.places.is_ok()).unwrap_or_default();
        let mut reads = place_names::reads(members, &[], step, held);
        if members.is_empty() {
            reads.clear();
        }
        if step == 0 {
            reads.push(Read::all(TableId::WeatherProvinces));
        }
        reads
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(ProvincesContext {
            weather: weather(source, load).map_err(|error| error.to_string()),
            places: place_names::Content::read(source, load).map_err(|error| error.to_string()),
        })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let region = params.region.parse::<u32>().map_err(|_| dashboard_core::invalid("Invalid weather region"))?;
        let mut warnings = Vec::new();
        let read_hour = frame.tables[TableId::WeatherProvinces.index()].sampled_at().or_else(|| frame.complete::<tables::WeatherProvinces>().is_ok().then_some(frame.hour));
        let named = place_names::Places::new(frame);
        let name_of = |id: u32| province_name(frame, &at.context.places, &named, id);
        let provinces = match &at.context.weather {
            Err(why) => {
                warnings.push(format!("1 weather province roster not shown: region {region}: {why}"));
                Vec::new()
            }
            Ok(weather) => {
                warnings.extend(weather.warnings.iter().cloned());
                match weather.members.get(&region) {
                    None => {
                        warnings.push(format!("1 weather province roster not shown: region {region} is absent from the recorded content stack"));
                        Vec::new()
                    }
                    Some(members) => {
                        let mut memo = Memo::default();
                        let temperatures = frame
                            .complete::<tables::WeatherProvinces>()
                            .map_err(|error| error.to_string())
                            .and_then(|rows| states(&mut memo, weather, frame.branch, frame.hour, rows))
                            .map(|states| members.iter().map(|&id| states.get(id as usize).map(|state| state.temperature)).collect());
                        provinces(frame, members, &name_of, temperatures, &mut warnings)
                    }
                }
            }
        };
        Ok(RegionalWeatherProvincesReply { read_hour, region: params.region.clone(), provinces, warnings })
    }
}
