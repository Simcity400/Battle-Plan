
use dashboard_base::products::{LoadContext, Product, at_hour, regions::{ActiveRegions, NoRegions, automatic, no_regions}};
use dashboard_base::{Source, catalogue::{self, Catalogue, KeyedResults, Refusals, View}, span::HourSpan};
use dashboard_follow::panel::Panel;
use dashboard_query::{Query, read};
use engine_executable::modifier_keys;
use content_reader_map::province_graph::ProvinceGraph;
use content_reader_localisation::localisation::Argument;
use replay_data::{
    rows::{Recorded, TableId, WeatherProvinceRow, WeatherRegionRow, tables},
    temperature::{Content, Memo, Plan, Planned, State},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
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
    #[doc = " The window's first hour; with `windowDays`, left out: the window is that many days ending with the last hour's day."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub first_hour: Option<i32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub window_days: Option<i32>,
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
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub pinned_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hidden_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<usize>,
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
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WeatherReadout {
    Condition {
        id: String,
        name: String,
        since_hour: Option<i32>,
        since_bounded: bool,
        as_of_hour: i32,
    },
    Value {
        value: String,
        as_of_hour: i32,
    },
    NotRecorded {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WeatherLegendItem {
    pub id: String,
    pub name: String,
    pub ordinal: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "RegionalWeatherColumnRegion"))]
pub struct ColumnRegion {
    pub id: String,
    pub pinned: bool,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub readout: Option<WeatherReadout>,
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
    #[doc = " The last hour through which nothing this reply shows changes: read beside the reply, never among its rows."]
    pub valid_through: i32,
    pub day_hours: i32,
    pub regions: Vec<Choice>,
    pub flags: Vec<Choice>,
    pub modifiers: Vec<Choice>,
    #[doc = " `available`, or why no temperature is shown."]
    pub temperature: String,
    pub columns: Columns,
    pub legend: Vec<WeatherLegendItem>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub extrema: Option<[String; 2]>,
    pub gaps: Vec<HourSpan>,
    #[doc = " Why the most active regions are not a proven rank at the asked hour: the hour's weather, or a region's, is not recorded. A top then lists only the pinned regions."]
    pub rank_missing: Option<String>,
    #[doc = " Why no region is listed in the ranked view; set only when no region is listed."]
    pub empty: Option<NoRegions>,
    pub active_unlisted: ActiveRegions,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(RegionalWeatherReply);

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
            let at = known.iter().filter(|span| span.1 <= to && span.2 >= from).next_back().map(|span| (span.0, span.2.min(to)));
            let read = at.and_then(|(_, at)| {
                let index = reads.partition_point(|run| run.0 <= at).checked_sub(1)?;
                Some(reads[index].1.min(at))
            });
            Day { first: from, last: to, at, read }
        })
        .collect()
}

type Daily = BTreeMap<u32, Vec<Option<i128>>>;

const CONDITION_PRIORITY: [&str; 6] = ["blizzard", "sandstorm", "heavy_rain", "snow", "light_rain", "arctic_water"];

#[cfg(test)]
thread_local! { static GRIDS_FOLDED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn chunk_ranges(len: usize, size: usize, preserve_terminal: bool) -> Vec<(usize, usize)> {
    if len == 0 {
        return Vec::new();
    }
    if preserve_terminal && size > 1 && len > 1 && len % size != 1 {
        let mut ranges: Vec<(usize, usize)> = (0..len - 1).step_by(size).map(|start| (start, (start + size).min(len - 1))).collect();
        ranges.push((len - 1, len));
        ranges
    } else {
        (0..len).step_by(size).map(|start| (start, (start + size).min(len))).collect()
    }
}

#[cfg(test)]
pub(crate) fn fold(days: &[Day], width: u32, measures: &[(String, Daily)], conditions: bool) -> io::Result<Columns> {
    fold_inner(days, width, measures, conditions, false, None)
}

#[cfg(test)]
pub(crate) fn fold_preserving_terminal(days: &[Day], width: u32, measures: &[(String, Daily)], conditions: bool) -> io::Result<Columns> {
    fold_inner(days, width, measures, conditions, true, None)
}

fn fold_inner(
    days: &[Day],
    width: u32,
    measures: &[(String, Daily)],
    conditions: bool,
    preserve_terminal: bool,
    selected: Option<&HashSet<u32>>,
) -> io::Result<Columns> {
    let size = days.len().div_ceil(width.max(1) as usize).max(1);
    let ranges = chunk_ranges(days.len(), size, preserve_terminal);
    let columns = ranges
        .iter()
        .map(|&(start, end)| {
            let chunk = &days[start..end];
            Column {
                first_hour: chunk[0].first,
                last_hour: chunk[chunk.len() - 1].last,
                sampled_hour: chunk.iter().filter_map(|day| day.at.map(|(_, at)| at)).max(),
                read_hour: chunk.iter().filter_map(|day| day.read).max(),
                recorded: chunk.iter().filter(|day| day.at.is_some()).count() as u32,
            }
        })
        .collect();
    let mut regions: BTreeMap<u32, (Vec<bool>, Vec<bool>, Vec<ColumnMeasure>)> = BTreeMap::new();
    for (id, daily) in measures {
        let flag = FLAGS.iter().any(|(flag, _, _)| flag == id);
        for (&region, values) in daily {
            if selected.is_some_and(|selected| !selected.contains(&region)) {
                continue;
            }
            let held = regions.entry(region).or_insert_with(|| (vec![false; days.len()], vec![false; days.len()], Vec::new()));
            let measure = if flag {
                for (day, value) in values.iter().enumerate() {
                    held.0[day] |= value.is_some();
                    held.1[day] |= value.is_some_and(|value| value != 0);
                }
                ColumnMeasure {
                    id: id.clone(),
                    days: ranges.iter().map(|&(start, end)| values[start..end].iter().filter(|value| value.is_some_and(|value| value != 0)).count() as u32).collect(),
                    values: None,
                }
            } else {
                let mut counts = Vec::with_capacity(ranges.len());
                let stats = ranges
                    .iter()
                    .map(|&(start, end)| {
                        let held: Vec<i128> = values[start..end].iter().flatten().copied().collect();
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
    #[cfg(test)]
    GRIDS_FOLDED.set(regions.len());
    let per_column = |keep: &dyn Fn(usize) -> bool| -> Vec<u32> { ranges.iter().map(|&(start, end)| (start..end).filter(|&day| keep(day)).count() as u32).collect() };
    let regions = regions
        .into_iter()
        .map(|(id, (observed, set, measures))| ColumnRegion {
            id: id.to_string(),
            pinned: false,
            readout: None,
            observed: conditions.then(|| per_column(&|day| observed[day])),
            clear: conditions.then(|| per_column(&|day| observed[day] && !set[day])),
            measures,
        })
        .collect();
    Ok(Columns { days: size as u32, columns, regions })
}

pub(crate) fn region_condition_on_day(region: u32, day: usize, measures: &[(String, Daily)]) -> Option<&'static str> {
    let mut observed = false;
    let mut active: [bool; 6] = [false; 6];
    for (id, daily) in measures {
        if let Some(pos) = CONDITION_PRIORITY.iter().position(|flag| flag == id)
            && let Some(values) = daily.get(&region)
            && let Some(Some(value)) = values.get(day)
        {
            observed = true;
            if *value != 0 {
                active[pos] = true;
            }
        }
    }
    if !observed {
        return None;
    }
    for (pos, &flag) in CONDITION_PRIORITY.iter().enumerate() {
        if active[pos] {
            return Some(flag);
        }
    }
    Some("clear")
}

pub(crate) fn region_readout(
    region: u32,
    days: &[Day],
    measures: &[(String, Daily)],
    show: Show,
    flags: &[Choice],
    temperature_status: &str,
) -> WeatherReadout {
    let Some(last) = days.last() else {
        return WeatherReadout::NotRecorded { reason: "No recorded days in this window".into() };
    };
    let Some((_, as_of_hour)) = last.at else {
        return WeatherReadout::NotRecorded { reason: "Weather was not recorded at this hour".into() };
    };
    let last_idx = days.len() - 1;
    match show {
        Show::Conditions => {
            let Some(current) = region_condition_on_day(region, last_idx, measures) else {
                return WeatherReadout::NotRecorded { reason: "Region weather was not recorded at this hour".into() };
            };
            let mut since_idx = last_idx;
            while since_idx > 0 {
                let prev = since_idx - 1;
                if days[prev].at.is_none() || region_condition_on_day(region, prev, measures) != Some(current) {
                    break;
                }
                since_idx = prev;
            }
            let name = if current == "clear" {
                "Clear".to_owned()
            } else {
                flags.iter().find(|choice| choice.id == current).map_or_else(|| current.to_owned(), |choice| choice.name.clone())
            };
            WeatherReadout::Condition {
                id: current.to_owned(),
                name,
                since_hour: days[since_idx].at.map(|(_, hour)| hour),
                since_bounded: true,
                as_of_hour,
            }
        }
        Show::Temperature | Show::Modifier => {
            let value = measures.first().and_then(|(_, daily)| daily.get(&region)).and_then(|values| values.get(last_idx)).copied().flatten();
            match value {
                Some(v) => WeatherReadout::Value { value: v.to_string(), as_of_hour },
                None => {
                    let reason = if show == Show::Temperature && temperature_status != "available" {
                        temperature_status.to_owned()
                    } else {
                        "Weather reading was not recorded at this hour".to_owned()
                    };
                    WeatherReadout::NotRecorded { reason }
                }
            }
        }
    }
}

const NOT_RECORDED_AT_HOUR: &str = "Weather was not recorded at this hour, so the most active regions are not known";
const REGION_NOT_RECORDED: &str = "A region's weather was not recorded at this hour, so the most active regions are not known";
const READ_IN_PART: &str = "Weather was read only in part at this hour, so the most active regions are not known";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HourRank {
    pub ranked: Vec<u32>,
    pub missing: Option<String>,
}

pub(crate) fn rank_at_hour(candidates: &[u32], days: &[Day], measures: &[(String, Daily)], show: Show, whole: bool) -> HourRank {
    let Some(last) = days.len().checked_sub(1).filter(|&last| days[last].at.is_some_and(|(_, at)| at == days[last].last)) else {
        return HourRank { ranked: Vec::new(), missing: Some(NOT_RECORDED_AT_HOUR.to_owned()) };
    };
    let mut scored: Vec<(u32, u128)> = Vec::new();
    let mut unknown = false;
    for &region in candidates {
        let score = match show {
            Show::Conditions => region_condition_on_day(region, last, measures).map(|condition| CONDITION_PRIORITY.iter().position(|flag| *flag == condition).map_or(0, |at| (CONDITION_PRIORITY.len() - at) as u128)),
            Show::Temperature | Show::Modifier => measures.first().and_then(|(_, daily)| daily.get(&region)).and_then(|values| values.get(last)).copied().flatten().map(i128::unsigned_abs),
        };
        match score {
            None => unknown = true,
            Some(score) if score > 0 => scored.push((region, score)),
            Some(_) => {}
        }
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let missing = if !whole { Some(READ_IN_PART) } else if unknown { Some(REGION_NOT_RECORDED) } else { None };
    HourRank { ranked: scored.into_iter().map(|(region, _)| region).collect(), missing: missing.map(str::to_owned) }
}

pub(crate) fn visible_extrema(displayed: &[u32], measures: &[(String, Daily)], show: Show) -> Option<[String; 2]> {
    if show == Show::Conditions {
        return None;
    }
    let (_, daily) = measures.first()?;
    let mut low: Option<i128> = None;
    let mut high: Option<i128> = None;
    for region in displayed {
        if let Some(values) = daily.get(region) {
            for &value in values.iter().flatten() {
                low = Some(low.map_or(value, |cur| cur.min(value)));
                high = Some(high.map_or(value, |cur| cur.max(value)));
            }
        }
    }
    low.zip(high).map(|(l, h)| [l.to_string(), h.to_string()])
}

pub(crate) fn visible_legend(displayed: &[u32], days: &[Day], measures: &[(String, Daily)], show: Show, flags: &[Choice]) -> Vec<WeatherLegendItem> {
    if show != Show::Conditions {
        return Vec::new();
    }
    let mut present: BTreeSet<&str> = BTreeSet::new();
    for &region in displayed {
        for day in 0..days.len() {
            if let Some(cond) = region_condition_on_day(region, day, measures) {
                present.insert(cond);
            }
        }
    }
    let mut items = Vec::new();
    for (ordinal, (id, _, _)) in FLAGS.iter().enumerate() {
        if present.contains(id) {
            let name = flags.iter().find(|choice| choice.id == *id).map_or_else(|| (*id).to_owned(), |choice| choice.name.clone());
            items.push(WeatherLegendItem { id: (*id).to_owned(), name, ordinal: ordinal as u32 });
        }
    }
    if present.contains("clear") {
        items.push(WeatherLegendItem { id: "clear".into(), name: "Clear".into(), ordinal: FLAGS.len() as u32 });
    }
    items
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

enum Sample {
    Regions(Result<(Vec<WeatherRegionRow>, Option<String>), String>),
    Temperature(Result<Vec<(u32, Option<i128>)>, String>),
}

type Samples = BTreeMap<(u32, i32), Arc<Sample>>;

#[derive(Default)]
struct WindowSamples {
    last: Option<(usize, i32)>,
    samples: Samples,
    modifiers: Option<(replay_frame::frame::ViewIdentity, Vec<Choice>, Vec<String>)>,
}

struct Playback<'a, 'step> {
    source: &'a Source,
    held: &'a mut WindowSamples,
    step: at_hour::DatedStep<'step>,
}

#[derive(Default)]
struct RegionSampleReader(Option<replay_lanes::at::Readers>);

fn region_sample_frame(source: &Source, query: &Query<'_>, run: u32, hour: i32) -> io::Result<DecodedFrame> {
    let state = source.state::<RegionSampleReader>();
    let mut held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let recording = query.replay.recording();
    let reads = [replay_lanes::at::Read::all(TableId::WeatherRegions)];
    let mut cursor = replay_lanes::at::Cursor::resume(recording, &reads, held.0.take())?;
    let frame = dashboard_query::cost::storage(|| cursor.frame(run, hour))?;
    at_hour::count_rows(|| match frame.tables[TableId::WeatherRegions.index()].held() {
        replay_frame::frame::TableView::Complete(rows) | replay_frame::frame::TableView::Truncated { rows, .. } => rows.len() as u64,
        _ => 0,
    });
    held.0 = cursor.into_readers();
    Ok(frame)
}

fn history(query: &Query<'_>, params: &RegionalWeatherParams, weather: &Weather, mut playback: Option<Playback<'_, '_>>, cancelled: &AtomicBool) -> io::Result<RegionalWeatherReply> {
    let recording = query.replay.recording();
    let asked = match (params.first_hour, params.window_days) {
        (Some(first), _) => first,
        (None, Some(days)) => params.last_hour.div_euclid(DAY_HOURS).saturating_sub(days - 1).saturating_mul(DAY_HOURS),
        (None, None) => params.last_hour,
    };
    let (first, last) = (asked.max(query.timeline.first), params.last_hour.min(query.timeline.last));
    let mut warnings = weather.warnings.clone();
    let mut known: Vec<(u32, i32, i32)> = Vec::new();
    let mut complete: Vec<(i32, i32)> = Vec::new();
    let mut status_runs: Vec<(Vec<replay_lanes::lanes::StatusRun>, HashMap<u32, String>)> = Vec::new();
    let mut terminal_failure: Option<String> = None;
    let mut terminal_reason: Option<String> = None;
    let mut terminal_witnessed: HashSet<u32> = HashSet::new();
    let mut reads: Vec<(i32, i32)> = Vec::new();
    let mut rows: BTreeSet<u32> = BTreeSet::new();
    let sampled = if params.show == Show::Temperature { TableId::WeatherProvinces } else { TableId::WeatherRegions };
    for stretch in query.timeline.within(first, last) {
        let (from, to) = (stretch.first.max(first), stretch.last.min(last));
        let whole = |status| if params.show == Show::Temperature { replay_recording::columnar_frames::complete(status) } else { replay_recording::columnar_frames::available(status) };
        let statuses = read::statuses(recording, stretch.run, sampled, from, to)?;
        let reasons = read::status_reasons(recording, stretch.run, sampled, &statuses)?;
        complete.extend(statuses.iter().filter(|run| replay_recording::columnar_frames::complete(run.status)).map(|run| (run.first, run.last)));
        known.extend(statuses.iter().filter(|run| whole(run.status)).map(|run| (stretch.run, run.first, run.last)));
        status_runs.push((statuses, reasons));
        reads.extend(dashboard_query::cost::storage(|| replay_lanes::lanes::reads(recording, stretch.run, sampled, from, to))?);
    }
    let day_missing = |from: i32, to: i32| -> Option<(i32, String)> {
        status_runs.iter().rev().find_map(|(runs, reasons)| {
            runs.iter()
                .rev()
                .find(|run| run.first <= to && run.last >= from && !replay_recording::columnar_frames::complete(run.status))
                .and_then(|run| {
                    let hour = run.last.min(to);
                    let why = crate::products::status_rows(run.status, reasons, query.replay.capture_mode).missing_reason(sampled, hour)?;
                    Some((hour, why))
                })
        })
    };
    let mut days = days(first, last, &known, &reads);
    let days_len = days.len();
    let (mut held, held_choices, projected, reader_source) = if let Some(playback) = playback.as_mut() {
        let retain = playback.step.step.is_some_and(|input| input.step.last() == params.last_hour && !input.step.gap());
        let reader_source = (retain && sampled == TableId::WeatherRegions).then_some(playback.source);
        let projected = if let Some((_, at)) = days.last().and_then(|day| day.at)
            && at == params.last_hour
            && playback.step.step.is_none_or(|input| input.step.last() == at && !input.step.gap())
        {
            Some(playback.step.frame(playback.source, query, at, &[sampled, TableId::ModifierDefinitions].map(at_hour::Read::all), |_| None)?)
        } else { None };
        (retain.then_some(&mut playback.held.samples), retain.then_some(&mut playback.held.modifiers), projected, reader_source)
    } else { (None, None, None, None) };
    if let Some(held) = held.as_deref_mut() {
        let present: BTreeSet<_> = days.iter().filter_map(|day| day.at).collect();
        held.retain(|key, _| present.contains(key));
    }
    let wanted: Option<BTreeSet<u32>> = params.regions.as_ref().map(|ids| ids.iter().filter_map(|id| id.parse().ok()).collect());
    let keeps = |region: u32| wanted.as_ref().is_none_or(|wanted| wanted.contains(&region));
    let mut frames = at_hour::Frames::new(query)?;
    let mut memo = Memo::default();
    let mut sample = |run, at| -> io::Result<Arc<Sample>> {
        let key = (run, at);
        if let Some(saved) = held.as_ref().and_then(|held| held.get(&key)) {
            return Ok(Arc::clone(saved));
        }
        let exact;
        let frame = if let Some(frame) = projected.filter(|frame| frame.hour == at) { frame } else {
            exact = if let Some(source) = reader_source {
                region_sample_frame(source, query, run, at)?
            } else {
                frames.frame(at, &[at_hour::Read::all(sampled)], "Regional weather")?
            };
            &exact
        };
        let witness = if sampled == TableId::WeatherRegions {
            Sample::Regions(dashboard_text::metadata::recorded_rows::<tables::WeatherRegions>(frame)?
                .into_rows(sampled, at).map(|(rows, incomplete)| (rows.to_vec(), incomplete)))
        } else {
            let branch = recording.run(run)?.branch;
            Sample::Temperature(frame.complete::<tables::WeatherProvinces>().map_err(|error| error.to_string())
                .and_then(|rows| states(&mut memo, weather, branch, at, rows))
                .map(|states| weather.members.iter().filter(|(region, _)| keeps(**region)).map(|(&region, members)| (region, mean(members, states))).collect()))
        };
        let witness = Arc::new(witness);
        if let Some(held) = held.as_deref_mut() {
            held.insert(key, Arc::clone(&witness));
        }
        Ok(witness)
    };
    let mut measures: Vec<(String, Daily)> = Vec::new();
    let temperature = weather.pass.as_ref().map(|_| "available".to_owned()).unwrap_or_else(Clone::clone);
    match params.show {
        Show::Conditions => {
            let mut daily: Vec<Daily> = FLAGS.iter().map(|_| BTreeMap::new()).collect();
            let mut refused = Refusals::default();
            for (index, day) in days.iter_mut().enumerate() {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let Some((run, at)) = day.at else {
                    if let Some((at, why)) = day_missing(day.first, day.last) {
                        if index + 1 == days_len {
                            terminal_failure = Some(format!("Weather was not available at this hour ({why}), so the most active regions are not known"));
                            terminal_reason = Some(why.clone());
                        }
                        refused.note(why, format!("hour {at}"));
                    }
                    continue;
                };
                let witness = sample(run, at)?;
                let Sample::Regions(found) = witness.as_ref() else { return Err(dashboard_core::refused("The weather sample holds no region rows")) };
                match found {
                    Ok((held, incomplete)) => {
                        for row in held {
                            rows.insert(row.id);
                            if index + 1 == days_len {
                                terminal_witnessed.insert(row.id);
                            }
                            if !keeps(row.id) {
                                continue;
                            }
                            let flags = [row.light_rain, row.heavy_rain, row.snow, row.blizzard, row.sandstorm, row.arctic_water];
                            for (daily_flag, value) in daily.iter_mut().zip(flags) {
                                daily_flag.entry(row.id).or_insert_with(|| vec![None; days_len])[index] = Some(i128::from(value));
                            }
                        }
                        if let Some(why) = incomplete {
                            if index + 1 == days_len {
                                terminal_reason = Some(why.clone());
                                if at == day.last {
                                    terminal_failure = Some(format!("Weather was read only in part at this hour ({why}), so the most active regions are not known"));
                                }
                            }
                            refused.note(why, format!("hour {at}"));
                        }
                        if index + 1 == days_len
                            && at < day.last
                            && let Some((miss_at, why)) = day_missing(day.last, day.last)
                        {
                            terminal_failure = Some(format!("Weather was not available at this hour ({why}), so the most active regions are not known"));
                            refused.note(why, format!("hour {miss_at}"));
                        }
                    }
                    Err(why) => {
                        day.at = None;
                        day.read = None;
                        if index + 1 == days_len {
                            terminal_failure = Some(format!("Weather was not available at this hour ({why}), so the most active regions are not known"));
                            terminal_reason = Some(why.clone());
                        }
                        refused.note(why, format!("hour {at}"));
                    }
                }
            }
            warnings.extend(refused.counts("weather condition days"));
            measures.extend(FLAGS.iter().zip(daily).map(|((flag, _, _), daily)| ((*flag).to_owned(), daily)));
        }
        Show::Modifier | Show::Temperature => {
            let modifier = params.modifier.as_deref().unwrap_or(MODIFIERS[0].0);
            let slot = MODIFIERS.iter().position(|(id, _, _)| *id == modifier).ok_or_else(|| dashboard_core::invalid("Invalid weather modifier"))?;
            let mut daily: Daily = BTreeMap::new();
            let mut refused = Refusals::default();
            for (index, day) in days.iter_mut().enumerate() {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let Some((run, at)) = day.at else {
                    if let Some((at, why)) = day_missing(day.first, day.last) {
                        if index + 1 == days_len {
                            terminal_failure = Some(format!("Weather was not available at this hour ({why}), so the most active regions are not known"));
                            terminal_reason = Some(why.clone());
                        }
                        refused.note(why, format!("hour {at}"));
                    }
                    continue;
                };
                let witness = sample(run, at)?;
                if let Sample::Regions(found) = witness.as_ref() {
                    match found {
                        Ok((held, incomplete)) => {
                            for row in held {
                                rows.insert(row.id);
                                if index + 1 == days_len {
                                    terminal_witnessed.insert(row.id);
                                }
                                if keeps(row.id)
                                    && let Some(value) = modifiers(row)[slot]
                                {
                                    daily.entry(row.id).or_insert_with(|| vec![None; days_len])[index] = Some(value);
                                }
                            }
                            if let Some(why) = incomplete {
                                if index + 1 == days_len {
                                    terminal_reason = Some(why.clone());
                                    if at == day.last {
                                        terminal_failure = Some(format!("Weather was read only in part at this hour ({why}), so the most active regions are not known"));
                                    }
                                }
                                refused.note(why, format!("hour {at}"));
                            }
                            if index + 1 == days_len
                                && at < day.last
                                && let Some((miss_at, why)) = day_missing(day.last, day.last)
                            {
                                terminal_failure = Some(format!("Weather was not available at this hour ({why}), so the most active regions are not known"));
                                refused.note(why, format!("hour {miss_at}"));
                            }
                        }
                        Err(why) => {
                            day.at = None;
                            day.read = None;
                            if index + 1 == days_len {
                                terminal_failure = Some(format!("Weather was not available at this hour ({why}), so the most active regions are not known"));
                                terminal_reason = Some(why.clone());
                            }
                            refused.note(why, format!("hour {at}"));
                        }
                    }
                    continue;
                }
                let Sample::Temperature(found) = witness.as_ref() else { return Err(dashboard_core::refused("The weather sample holds no temperature readings")) };
                match found {
                    Err(why) => {
                        if index + 1 == days_len {
                            terminal_failure = Some(format!("Temperature was not known at this hour ({why}), so the most active regions are not known"));
                        }
                        refused.note(why, format!("hour {at}"))
                    }
                    Ok(values) => {
                        for &(region, value) in values {
                            rows.insert(region);
                            if let Some(value) = value {
                                daily.entry(region).or_insert_with(|| vec![None; days_len])[index] = Some(value);
                            }
                        }
                        if index + 1 == days_len
                            && at < day.last
                            && let Some((miss_at, why)) = day_missing(day.last, day.last)
                        {
                            terminal_failure = Some(format!("Temperature was not known at this hour ({why}), so the most active regions are not known"));
                            refused.note(why, format!("hour {miss_at}"));
                        }
                    }
                }
            }
            if params.show == Show::Temperature {
                warnings.extend(refused.counts("weather temperature days"));
                measures.push(("temp".to_owned(), daily));
            } else {
                warnings.extend(refused.counts("weather modifier days"));
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
    let all_candidates: Vec<u32> = rows
        .iter()
        .copied()
        .filter(|region| weather.names.contains_key(region) && (weather.members.is_empty() || weather.members.get(region).is_some_and(|members| !members.is_empty())))
        .collect();
    let use_ranking = params.regions.is_none() && (params.top.is_some() || params.pinned_regions.is_some() || params.hidden_regions.is_some());
    let mut rank_missing = None;
    let mut hidden_active = 0;
    let mut active_unlisted = ActiveRegions::default();
    let ranked_order: Option<Vec<(u32, bool)>> = if use_ranking {
        let mut seen = HashSet::new();
        let pinned: Vec<u32> = params.pinned_regions.iter().flatten().copied().filter(|id| seen.insert(*id)).collect();
        let hidden: HashSet<u32> = params.hidden_regions.iter().flatten().copied().collect();
        let candidates: Vec<u32> = all_candidates.iter().copied().filter(|id| !seen.contains(id)).collect();
        let whole = days.last().is_some_and(|day| complete.iter().any(|&(first, end)| first <= day.last && day.last <= end));
        let HourRank { ranked, missing } = rank_at_hour(&candidates, &days, &measures, params.show, whole);
        rank_missing = terminal_failure.or(missing);
        let top = if rank_missing.is_some() { 0 } else { params.top.unwrap_or(usize::MAX) };
        let chosen = automatic(ranked, &hidden, top, rank_missing.clone());
        hidden_active = chosen.hidden;
        active_unlisted = chosen.active_unlisted;
        Some(pinned.into_iter().map(|id| (id, true)).chain(chosen.listed.into_iter().map(|id| (id, false))).collect())
    } else {
        None
    };
    let selected = ranked_order.as_ref().map(|order| order.iter().map(|&(id, _)| id).collect());
    let mut columns = fold_inner(&days, params.width, &measures, params.show == Show::Conditions, true, selected.as_ref())?;
    let shown = days.iter().rev().find_map(|day| day.at);
    let mut refused = Refusals::default();
    let regions: Vec<Choice> = all_candidates
        .iter()
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
    let modifiers = if let (Some((_, at)), Some(frame), Some(held)) = (shown, projected, held_choices)
        && frame.hour == at
    {
        let identity = frame.tables[TableId::ModifierDefinitions.index()].identity();
        if held.as_ref().is_some_and(|(old, ..)| !old.same(&identity)) { *held = None; }
        let (_, choices, notes) = held.get_or_insert_with(|| {
            let mut notes = Vec::new();
            (identity, modifier_choices(Some(frame), &weather.cat, &mut notes), notes)
        });
        warnings.extend(notes.iter().cloned());
        choices.clone()
    } else {
        let auxiliary = shown.and_then(|(run, at)| dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, at, &[replay_lanes::at::Read::all(TableId::ModifierDefinitions)])).ok());
        modifier_choices(auxiliary.as_ref(), &weather.cat, &mut warnings)
    };
    let displayed_order = ranked_order.unwrap_or_else(|| {
        columns.regions.iter().filter_map(|r| Some((r.id.parse::<u32>().ok()?, params.regions.is_some()))).collect()
    });
    let mut by_id: HashMap<u32, ColumnRegion> = columns.regions.into_iter().filter_map(|r| Some((r.id.parse::<u32>().ok()?, r))).collect();
    let col_count = columns.columns.len();
    let terminal_unsampled = days.last().is_none_or(|day| day.at.is_none());
    columns.regions = displayed_order
        .iter()
        .map(|&(region, pinned)| {
            let mut entry = by_id.remove(&region).unwrap_or_else(|| ColumnRegion {
                id: region.to_string(),
                pinned,
                readout: None,
                observed: (params.show == Show::Conditions).then(|| vec![0; col_count]),
                clear: (params.show == Show::Conditions).then(|| vec![0; col_count]),
                measures: Vec::new(),
            });
            entry.pinned = pinned;
            entry.readout = Some(match (region_readout(region, &days, &measures, params.show, &flags, &temperature), &terminal_reason) {
                (WeatherReadout::NotRecorded { .. }, Some(reason))
                    if !terminal_witnessed.contains(&region) && (terminal_unsampled || params.show != Show::Temperature || temperature == "available") =>
                {
                    WeatherReadout::NotRecorded { reason: reason.clone() }
                }
                (readout, _) => readout,
            });
            entry
        })
        .collect();
    let displayed_ids: Vec<u32> = displayed_order.iter().map(|&(id, _)| id).collect();
    let extrema = visible_extrema(&displayed_ids, &measures, params.show);
    let legend = visible_legend(&displayed_ids, &days, &measures, params.show, &flags);
    let gaps: Vec<HourSpan> = query
        .timeline
        .unheld
        .iter()
        .filter_map(|gap| {
            let a = gap.first.max(first);
            let b = gap.last.min(last);
            (a <= b).then_some(HourSpan { first_hour: a, last_hour: b })
        })
        .collect();
    let empty = no_regions(!use_ranking || !displayed_ids.is_empty(), rank_missing.as_deref(), hidden_active, 0);
    let valid_through = valid_through(query, sampled, last)?;
    Ok(RegionalWeatherReply { first_hour: first, last_hour: last, valid_through, day_hours: DAY_HOURS, regions, flags, modifiers, temperature, columns, legend, extrema, gaps, rank_missing, empty, active_unlisted, warnings })
}

fn valid_through(query: &Query<'_>, sampled: TableId, last: i32) -> io::Result<i32> {
    let day_end = last.div_euclid(DAY_HOURS).saturating_mul(DAY_HOURS).saturating_add(DAY_HOURS - 1);
    let Some(stretch) = query.timeline.at(last) else { return Ok(last) };
    let through = day_end.min(stretch.last);
    if through <= last {
        return Ok(last);
    }
    let statuses = read::statuses(query.replay.recording(), stretch.run, sampled, last, through)?;
    if statuses
        .iter()
        .find(|run| run.first <= last && last <= run.last)
        .is_some_and(|run| !replay_recording::columnar_frames::complete(run.status))
    {
        return Ok(last);
    }
    let next_status = statuses.iter().find(|run| run.first > last).map(|run| run.first);
    let next_read = dashboard_query::cost::storage(|| replay_lanes::lanes::reads(query.replay.recording(), stretch.run, sampled, last + 1, through))?.first().map(|read| read.0);
    let next = match (next_read, next_status) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    Ok(next.map_or(through, |next| next - 1).max(last))
}

impl Product for RegionalWeather {
    const KEY: &'static str = "regional-weather";
    const CONTENT_DEPENDENT: bool = true;
    type Params = RegionalWeatherParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, windowDays, lastHour: End, width, show, modifier, regions, pinnedRegions, hiddenRegions, top, "pinnedRegions[]", "hiddenRegions[]"];
    type Reply = RegionalWeatherReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.first_hour.is_some_and(|first| first > params.last_hour) || params.first_hour.is_some() == params.window_days.is_some() || params.window_days.is_some_and(|days| days < 1) || params.width == 0 {
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
            WeatherReadout::decl(),
            WeatherLegendItem::decl(),
            HourSpan::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        load_history(source, params, context, None, cancelled)
    }
}

fn load_history(source: &Source, params: &RegionalWeatherParams, load: &LoadContext, playback: Option<Playback<'_, '_>>, cancelled: &AtomicBool) -> io::Result<RegionalWeatherReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let weather = weather(source, load)?;
    let reply = history(&query, params, &weather, playback, cancelled)?;
    dashboard_base::products::check_reply_budget(&reply).map_err(|_| dashboard_core::invalid("Regional weather reply exceeds transport budget; nothing was truncated"))?;
    Ok(reply)
}

pub fn panel(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let held = Mutex::new(WindowSamples::default());
    at_hour::dated_product::<RegionalWeather>(source, load, asked,
        |request, hour| request["lastHour"] = hour.into(), |params, _| params.last_hour, |_, _| {},
        move |source, params, load, cancelled, step| {
            let mut held = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let hour = params.last_hour;
            let input = step.step.map(|input| input.step).filter(|step| step.last() == hour && !step.gap());
            let piece = input.and_then(|step| step.piece());
            if !input.is_some_and(|step| !step.reset() && held.last == piece.zip(step.first().checked_sub(1))) {
                *held = WindowSamples::default();
            }
            let reply = load_history(source, params, load, Some(Playback { source, held: &mut held, step }), cancelled);
            if let Some(piece) = piece.filter(|_| reply.is_ok()) { held.last = Some((piece, hour)); }
            else { *held = WindowSamples::default(); }
            reply
        })
}
