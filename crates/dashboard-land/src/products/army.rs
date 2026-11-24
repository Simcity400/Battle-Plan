
use dashboard_base::products::Country;
use dashboard_world::products::army_windows::{ArmySpan, windows};
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::Source;
use dashboard_declare::Key;
use dashboard_query::{
    Query,
    identity::Holder,
    read,
    series::{Line, NavigationKey, Part, SeriesKeyReply},
};
use content_reader_script::sub_units::SubUnits;
use replay_recording::columnar_frames::Recording;
use replay_columnar::{store::TableView, summary};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

mod at_hour;
mod template_roster;
pub use at_hour::{ArmyAt, ArmyAtParams};
pub use template_roster::{TemplateRoster, TemplateRosterParams, TemplateRosterReply};

pub const SERIES: &str = "army-size";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub struct ArmySize;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ArmyMeasure {
    Manpower,
    Divisions,
    Strength,
    Battalions,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ArmyAttribution {
    Owner,
    Host,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmySizeParams {
    pub branch_id: String,
    pub countries: Vec<Country>,
    pub top: Option<usize>,
    pub measure: ArmyMeasure,
    pub count_for: ArmyAttribution,
    pub by_type: bool,
    #[doc = " None is the window manifest; histories name a window by its index."]
    #[serde(default)]
    pub window: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyType {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub total: Option<f64>,
    pub raw: String,
}

impl Serialize for ArmyRun {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.first_hour, self.total, (self.last_hour != self.first_hour).then_some(self.last_hour), &self.raw).serialize(serializer)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyMixRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub values: Vec<Option<f64>>,
    #[doc = " Decimal counts keep exact labels; values are chart geometry only."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub raw_values: Option<Vec<Option<String>>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyIdentityRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmySeries {
    pub country: Country,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub identities: Option<Vec<ArmyIdentityRun>>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<ArmyRun>"))]
    pub runs: Vec<ArmyRun>,
    pub segments: Vec<ArmySpan>,
    pub membership: Option<Vec<ArmySpan>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub mix: Option<Vec<ArmyMixRun>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmySizeReply {
    pub branch_id: String,
    pub windows: Vec<ArmySpan>,
    pub window: Option<usize>,
    pub interval: Option<ArmySpan>,
    pub available: bool,
    pub measures: Vec<ArmyMeasure>,
    pub types: Vec<ArmyType>,
    pub series: Vec<ArmySeries>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ArmySizeReply);

const MEASURES: [ArmyMeasure; 4] = [ArmyMeasure::Manpower, ArmyMeasure::Divisions, ArmyMeasure::Strength, ArmyMeasure::Battalions];

fn measure_name(measure: ArmyMeasure, attribution: ArmyAttribution) -> &'static str {
    match (measure, attribution) {
        (ArmyMeasure::Divisions, ArmyAttribution::Owner) => "ownerDivisions",
        (ArmyMeasure::Strength, ArmyAttribution::Owner) => "ownerStrength",
        (ArmyMeasure::Divisions, ArmyAttribution::Host) => "hostDivisions",
        (ArmyMeasure::Strength, ArmyAttribution::Host) => "hostStrength",
        (ArmyMeasure::Battalions, ArmyAttribution::Owner) => "ownerBattalions",
        (ArmyMeasure::Battalions, ArmyAttribution::Host) => "hostBattalions",
        (ArmyMeasure::Manpower, ArmyAttribution::Owner) => "ownerManpower",
        (ArmyMeasure::Manpower, ArmyAttribution::Host) => "hostManpower",
    }
}

fn line<'q, 'a>(query: &'q Query<'a>, measure: ArmyMeasure, attribution: ArmyAttribution) -> io::Result<Line<'q, 'a>> {
    let series = dashboard_declare::series("army-totals").ok_or_else(|| invalid("Army totals are not declared"))?;
    let declared = series.measure(measure_name(measure, attribution)).ok_or_else(|| invalid("Army measure is not declared"))?;
    debug_assert_eq!(series.key, Key::Slot);
    Line::new(query, series, declared)
}

pub fn tile_line<'q, 'a>(query: &'q Query<'a>, params: &Value) -> io::Result<Line<'q, 'a>> {
    let family: Family = dashboard_query::named::named(params.get("family").unwrap_or(&Value::Null)).map_err(|error| invalid(format!("Invalid army tile request: {error}")))?;
    if family.top.is_some() {
        return Err(invalid("A ranked army chart is not served as tiles"));
    }
    line(query, family.measure, family.count_for)
}

fn geometry(raw: i64, measure: ArmyMeasure) -> f64 {
    raw as f64 / if measure == ArmyMeasure::Strength { 100_000.0 } else { 1.0 }
}

impl Product for ArmySize {
    const KEY: &'static str = "army-size";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ArmySizeParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, measure, countFor, byType, window, "countries[]"];
    type Reply = ArmySizeReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ArmyMeasure::decl(),
            ArmyAttribution::decl(),
            ArmySizeParams::decl(),
            ArmyType::decl(),
            "type ArmyRun = [number, number | null, number | null, string?];".into(),
            ArmyMixRun::decl(),
            ArmyIdentityRun::decl(),
            ArmySpan::decl(),
            ArmySeries::decl(),
            ArmySizeReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params.top.is_some_and(|top| top == 0 || !params.countries.is_empty()) {
            return Err("Invalid country selection");
        }
        for tag in &params.countries {
            dashboard_base::products::validate_country(tag)?;
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        window_reply(source, &query, params, context, cancelled)
    }
}

fn window_reply(source: &Source, query: &Query<'_>, params: &ArmySizeParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ArmySizeReply> {
    let windows = windows(query)?;
    let interval = params
        .window
        .map(|index| windows.get(index).cloned().ok_or_else(|| invalid("Unknown army history window")))
        .transpose()?;
    let mut reply = ArmySizeReply {
        branch_id: params.branch_id.clone(),
        windows: if params.window.is_none() { windows } else { Vec::new() },
        window: params.window,
        interval: interval.clone(),
        available: true,
        measures: MEASURES.to_vec(),
        types: Vec::new(),
        series: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(range) = interval else {
        return Ok(reply);
    };
    let line = line(query, params.measure, params.count_for)?;
    let (first, last) = (range.first_hour, range.last_hour);
    let ranked: Option<BTreeMap<Country, Vec<dashboard_base::span::HourSpan>>> = match params.top {
        Some(top) => {
            let width = ((i64::from(last) - i64::from(first) + 1) / 6).max(1) as u32;
            let lines = line.ranked(first, last, width, top)?;
            Some(
                lines
                    .into_iter()
                    .map(|line| (Country::from(line.tag), line.membership.into_iter().map(|(first_hour, last_hour)| dashboard_base::span::HourSpan { first_hour, last_hour }).collect()))
                    .collect(),
            )
        }
        None => None,
    };
    let tags: Vec<Country> = match &ranked {
        Some(members) => members.keys().cloned().collect(),
        None => {
            let mut seen = BTreeSet::new();
            params.countries.iter().filter(|tag| seen.insert((*tag).clone())).cloned().collect()
        }
    };
    let mut parts: BTreeMap<Country, Vec<Part>> = BTreeMap::new();
    for tag in &tags {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let held = line.parts(tag, first, last);
        if !held.is_empty() || params.top.is_none() {
            parts.insert(tag.clone(), held);
        }
    }
    let by_type = params.by_type && params.measure == ArmyMeasure::Battalions;
    let attribution = u128::from(params.count_for == ArmyAttribution::Host);
    let totals = line.source();
    let (found, mix) = rayon::join(
        || totals_of(query, totals, &parts),
        || by_type.then(|| mix_reads(query.replay.recording(), attribution, &parts, |run| crate::products::army::cached_army_mix(source, run), cancelled)).transpose(),
    );
    let (found, mix) = (found?, mix?);
    let runs = runs_by_tag(&found, &parts);
    let membership = ranked;
    let wanted: Vec<Country> = match &membership {
        Some(members) => members.keys().cloned().collect(),
        None => tags,
    };
    for tag in wanted {
        let Some(tag_parts) = parts.get(&tag) else { continue };
        let identities = params.top.is_none().then(|| {
            tag_parts
                .iter()
                .fold(Vec::<ArmyIdentityRun>::new(), |mut held, part| {
                    match held.last_mut() {
                        Some(last) if last.slot == part.slot && last.country_id == part.id && last.last_hour + 1 == part.first => last.last_hour = part.last,
                        _ => held.push(ArmyIdentityRun { first_hour: part.first, last_hour: part.last, slot: part.slot, country_id: part.id }),
                    }
                    held
                })
        });
        let segments = segments(query, &tag, first, last);
        let spans: Vec<(i32, i32)> = match (&membership, &identities) {
            (Some(members), _) => members.get(&tag).into_iter().flatten().map(|span| (span.first_hour, span.last_hour)).collect(),
            (None, Some(identities)) => identities.iter().map(|identity| (identity.first_hour, identity.last_hour)).collect(),
            (None, None) => Vec::new(),
        };
        let runs = runs
            .get(&tag)
            .into_iter()
            .flatten()
            .filter_map(|&(run_first, run_last, value)| Some((run_first, run_last, value?)))
            .flat_map(|(run_first, run_last, value)| {
                spans.iter().filter_map(move |&(span_first, span_last)| {
                    let (lo, hi) = (run_first.max(span_first), run_last.min(span_last));
                    (lo <= hi).then(|| ArmyRun { first_hour: lo, last_hour: hi, total: Some(geometry(value, params.measure)), raw: value.to_string() })
                })
            })
            .collect();
        reply.series.push(ArmySeries {
            country: tag.clone(),
            identities,
            runs,
            segments,
            membership: membership.as_ref().map(|members| {
                members.get(&tag).into_iter().flatten().map(|span| ArmySpan { first_hour: span.first_hour, last_hour: span.last_hour }).collect()
            }),
            mix: None,
        });
    }
    if params.by_type && params.measure == ArmyMeasure::Battalions {
        reply.types = composition(source, query, &parts, &found, mix.as_ref().unwrap_or(&MixReads::new()), &mut reply.series, &mut reply.warnings, context, cancelled)?;
    }
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

type Found = HashMap<(u32, i32, i32), HashMap<u128, read::Runs>>;

#[cfg(test)]
fn totals_runs(query: &Query<'_>, line: &Line<'_, '_>, parts: &BTreeMap<Country, Vec<Part>>) -> io::Result<Found> {
    totals_of(query, line.source(), parts)
}

fn totals_of(query: &Query<'_>, source: summary::Source, parts: &BTreeMap<Country, Vec<Part>>) -> io::Result<Found> {
    let recording = query.replay.recording();
    let mut by_run: HashMap<(u32, i32, i32), Vec<u128>> = HashMap::new();
    for part in parts.values().flatten() {
        by_run.entry((part.run, part.first, part.last)).or_default().push(part.key);
    }
    let mut found = Found::new();
    for ((run, first, last), mut keys) in by_run {
        keys.sort_unstable();
        keys.dedup();
        found.insert((run, first, last), read::runs(recording, run, TableId::ArmyTotals, source, Some(&keys), first, last)?);
    }
    Ok(found)
}

fn runs_by_tag(found: &Found, parts: &BTreeMap<Country, Vec<Part>>) -> BTreeMap<Country, Vec<(i32, i32, Option<i64>)>> {
    parts
        .iter()
        .map(|(tag, parts)| {
            let runs = parts
                .iter()
                .flat_map(|part| found.get(&(part.run, part.first, part.last)).and_then(|runs| runs.get(&part.key)).cloned().unwrap_or_default())
                .collect();
            (tag.clone(), runs)
        })
        .collect()
}

fn segments(query: &Query<'_>, tag: &Country, first: i32, last: i32) -> Vec<ArmySpan> {
    query
        .holders
        .runs(tag)
        .iter()
        .filter(|run| matches!(run.holder, Holder::Found { .. }) && run.first <= last && run.last >= first)
        .map(|run| ArmySpan { first_hour: run.first, last_hour: run.last.min(last) })
        .collect()
}

pub(super) fn group_info(units: Option<&SubUnits>, catalogue: Option<&dashboard_base::catalogue::Catalogue>, name: &str) -> (String, ArmyType, Option<String>) {
    let group = units.map_or_else(|| Err("game content is unavailable".into()), |units| units.group(name.as_bytes()));
    let (key, label, mut warning) = match group {
        Ok(Some(group)) => {
            let identity = match content_reader_parser::tokens::Token::c_str(group.as_bytes()) {
                content_reader_parser::tokens::Token::Name(name) => String::from_utf8(name).unwrap_or_else(|_| group.to_owned()),
                content_reader_parser::tokens::Token::Rejected => String::new(),
            };
            (format!("group:{identity}"), group, None)
        }
        Ok(None) => ("group:".into(), "", None),
        Err(why) => (format!("unknown:{name}"), name, Some(format!("{name}: unknown unit group: {why}"))),
    };
    let localised = if label.is_empty() {
        Some(Ok(String::new()))
    } else {
        let titled = format!("group_{label}_title");
        catalogue.and_then(|catalogue| {
            let key = if catalogue.localisation.get(&titled).is_some() {
                titled.as_str()
            } else if catalogue.localisation.get(label).is_some() {
                label
            } else {
                return None;
            };
            Some(dashboard_text::catalogue::localised(
                &catalogue.localisation,
                key,
                dashboard_text::catalogue::View::Plain,
            ))
        })
    };
    let label = match localised {
        Some(Ok(label)) => label,
        result => {
            let reason = result.and_then(Result::err).unwrap_or_else(|| "localized title is absent from the recorded language".into());
            if warning.is_none() {
                warning = Some(format!("{key}: {reason}"));
            }
            "Unit group name unavailable".into()
        }
    };
    (key.clone(), ArmyType { key, name: label }, warning)
}

pub fn check_content(catalogue: &dashboard_base::catalogue::Catalogue) -> io::Result<Vec<dashboard_core::findings::Finding>> {
    use dashboard_core::findings::{Finding, Kind};
    let units = content_reader_key_names::unit_leader::sub_units(&catalogue.content.layers)?;
    let mut findings: Vec<_> = units.warnings().iter().map(|message| Finding::new(Kind::Refused, "unit group reader", message)).collect();
    for name in units.registered() {
        match std::str::from_utf8(name) {
            Ok(name) => {
                let (_, _, warning) = group_info(Some(&units), Some(catalogue), name);
                if let Some(reason) = warning {
                    findings.push(Finding::new(Kind::Refused, name, reason));
                }
            }
            Err(_) => findings.push(Finding::new(Kind::Refused, name.escape_ascii().to_string(), "Sub-unit name is not UTF-8")),
        }
    }
    Ok(findings)
}

#[derive(Default)]
struct Groups {
    content: Option<(Arc<dashboard_base::catalogue::Catalogue>, Result<SubUnits, String>)>,
}

fn composition(
    source: &Source,
    query: &Query<'_>,
    parts: &BTreeMap<Country, Vec<Part>>,
    found: &Found,
    mix: &MixReads,
    series: &mut [ArmySeries],
    warnings: &mut Vec<String>,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Vec<ArmyType>> {
    let catalogue = match context.content.as_ref().map(|options| dashboard_base::catalogue::catalogue(source, options)).transpose() {
        Ok(catalogue) => catalogue,
        Err(error) => {
            warnings.push(format!("Unit-group content unavailable: {error}"));
            None
        }
    };
    let state = source.state::<Groups>();
    let mut groups = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(catalogue) = &catalogue
        && groups.content.as_ref().is_none_or(|(old, _)| !Arc::ptr_eq(old, catalogue))
    {
        let units = content_reader_key_names::unit_leader::sub_units(&catalogue.content.layers).map_err(|error| error.to_string());
        groups.content = Some((Arc::clone(catalogue), units));
    }
    let units = catalogue.as_ref().and_then(|_| groups.content.as_ref().map(|(_, units)| units));
    if let Some(Err(reason)) = units {
        warnings.push(format!("Unit-group reader refused: {reason}"));
    }
    let units = units.and_then(|units| units.as_ref().ok());
    if let Some(units) = units {
        warnings.extend(units.warnings().iter().cloned());
    }
    let (slot_runs, names) = mixes_of(query.replay.recording(), parts, found, mix, cancelled)?;
    let mut mixes: BTreeMap<Country, MixRuns> = BTreeMap::new();
    let mut tokens: BTreeSet<u32> = BTreeSet::new();
    for line in series.iter() {
        let mut runs = Vec::new();
        for part in parts.get(&line.country).into_iter().flatten() {
            for (first, last, held) in slot_runs.get(&(part.run, part.first, part.last, part.slot)).into_iter().flatten() {
                tokens.extend(held.keys().copied());
                runs.push((*first, *last, held.clone()));
            }
        }
        mixes.insert(line.country.clone(), runs);
    }
    let mut mapping: HashMap<u32, String> = HashMap::new();
    let mut types: BTreeMap<String, ArmyType> = BTreeMap::new();
    let mut unresolved = 0;
    let mut seen_names = BTreeSet::new();
    for token in &tokens {
        let name = names.get(token).cloned().unwrap_or_else(|| format!("Unresolved sub-unit token {token}"));
        let (key, info, warning) = group_info(units, catalogue.as_deref(), &name);
        mapping.insert(*token, key.clone());
        types.insert(key, info);
        if seen_names.insert(name)
            && let Some(warning) = warning
        {
            unresolved += 1;
            warnings.push(warning);
        }
    }
    if unresolved > 0 {
        warnings.push(format!("{unresolved} sub-unit group/name lookups are unresolved or refused; battalions remain counted."));
    }
    let types: Vec<ArmyType> = types.into_values().collect();
    let positions: HashMap<&String, usize> = types.iter().enumerate().map(|(at, info)| (&info.key, at)).collect();
    for line in series.iter_mut() {
        let spans: Vec<(i32, i32)> = line.runs.iter().map(|run| (run.first_hour, run.last_hour)).collect();
        let mut out: Vec<ArmyMixRun> = Vec::new();
        for (first, last, parts) in mixes.get(&line.country).into_iter().flatten() {
            let mut values = vec![0i64; types.len()];
            for (token, count) in parts {
                values[positions[&mapping[token]]] += count;
            }
            let raw: Vec<Option<String>> = values.iter().map(|value| Some(value.to_string())).collect();
            for &(span_first, span_last) in &spans {
                let (lo, hi) = ((*first).max(span_first), (*last).min(span_last));
                if lo > hi {
                    continue;
                }
                match out.last_mut() {
                    Some(previous) if previous.last_hour + 1 == lo && previous.raw_values.as_ref() == Some(&raw) => previous.last_hour = hi,
                    _ => out.push(ArmyMixRun {
                        first_hour: lo,
                        last_hour: hi,
                        values: values.iter().map(|value| Some(*value as f64)).collect(),
                        raw_values: Some(raw.clone()),
                    }),
                }
            }
        }
        line.mix = (!out.is_empty()).then_some(out);
    }
    Ok(types)
}

type MixRuns = Vec<(i32, i32, BTreeMap<u32, i64>)>;

#[cfg(test)]
fn mix_runs(
    recording: &Recording,
    attribution: u128,
    parts: &BTreeMap<Country, Vec<Part>>,
    found: &Found,
    lives: impl Fn(u32) -> io::Result<Arc<Vec<(u128, i64, i64)>>>,
    cancelled: &AtomicBool,
) -> io::Result<(HashMap<(u32, i32, i32, u32), MixRuns>, HashMap<u32, String>)> {
    let read = mix_reads(recording, attribution, parts, lives, cancelled)?;
    mixes_of(recording, parts, found, &read, cancelled)
}

type MixReads = HashMap<(u32, i32, i32), HashMap<u128, read::Runs>>;

fn mix_reads(
    recording: &Recording,
    attribution: u128,
    parts: &BTreeMap<Country, Vec<Part>>,
    lives: impl Fn(u32) -> io::Result<Arc<Vec<(u128, i64, i64)>>>,
    cancelled: &AtomicBool,
) -> io::Result<MixReads> {
    let battalions = summary::Source::Cell(dashboard_declare::field(TableId::ArmyMix, "battalions")?);
    let mut out = MixReads::new();
    for (&(run, first, last), slots) in &stretch_slots(parts) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lives = lives(run)?;
        let mut keys = Vec::new();
        for &slot in slots.keys() {
            let low = u128::from(slot) << 40 | attribution << 32;
            let start = lives.partition_point(|&(key, ..)| key < low);
            keys.extend(
                lives[start..]
                    .iter()
                    .take_while(|&&(key, ..)| key <= low | u128::from(u32::MAX))
                    .filter(|&&(_, born, gone)| born <= i64::from(last) && gone >= i64::from(first))
                    .map(|&(key, ..)| key),
            );
        }
        out.insert((run, first, last), if keys.is_empty() { HashMap::new() } else { read::runs(recording, run, TableId::ArmyMix, battalions, Some(&keys), first, last)? });
    }
    Ok(out)
}

fn stretch_slots(parts: &BTreeMap<Country, Vec<Part>>) -> BTreeMap<(u32, i32, i32), BTreeMap<u32, u128>> {
    let mut by_stretch: BTreeMap<(u32, i32, i32), BTreeMap<u32, u128>> = BTreeMap::new();
    for part in parts.values().flatten() {
        by_stretch.entry((part.run, part.first, part.last)).or_default().insert(part.slot, part.key);
    }
    by_stretch
}

fn mixes_of(
    recording: &Recording,
    parts: &BTreeMap<Country, Vec<Part>>,
    found: &Found,
    read: &MixReads,
    cancelled: &AtomicBool,
) -> io::Result<(HashMap<(u32, i32, i32, u32), MixRuns>, HashMap<u32, String>)> {
    let no_mix = HashMap::new();
    let mut out = HashMap::new();
    let mut named: BTreeMap<u32, (i32, BTreeSet<u32>)> = BTreeMap::new();
    for (&(run, first, last), slots) in &stretch_slots(parts) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mix = read.get(&(run, first, last)).unwrap_or(&no_mix);
        let mut by_slot: HashMap<u32, Vec<(u32, &read::Runs)>> = HashMap::new();
        for (key, runs) in mix.iter() {
            by_slot.entry((key >> 40) as u32).or_default().push((*key as u32, runs));
        }
        let totals = found.get(&(run, first, last));
        let names = named.entry(run).or_insert((last, BTreeSet::new()));
        names.0 = names.0.max(last);
        for (&slot, key) in slots {
            let known = totals.and_then(|runs| runs.get(key)).map_or(&[][..], Vec::as_slice);
            let runs = mixes_over(first, last, known, by_slot.get(&slot).map_or(&[][..], Vec::as_slice));
            for (_, _, held) in &runs {
                names.1.extend(held.keys().copied());
            }
            out.insert((run, first, last, slot), runs);
        }
    }
    let literal = dashboard_declare::field(TableId::LexicalTokens, "literal")?;
    let mut names = HashMap::new();
    for (run, (hour, tokens)) in named {
        if tokens.is_empty() {
            continue;
        }
        let keys: Vec<u128> = tokens.iter().map(|&token| u128::from(token)).collect();
        let held = read::at(recording, run, &[read::Read { table: TableId::LexicalTokens, columns: Some(&[literal]), keys: read::Keys::These(&keys) }], hour)?;
        let (view, rows) = (held[0].view(), &held[0].rows);
        for token in tokens {
            let slot = rows.binary_search_by_key(&u128::from(token), |&(key, _)| key).ok().map(|at| rows[at].1);
            names.entry(token).or_insert_with(|| token_name(&view, slot, token));
        }
    }
    Ok((out, names))
}

fn holding(runs: &[(i32, i32, Option<i64>)], at: &mut usize, hour: i32) -> Option<i64> {
    while runs.get(*at).is_some_and(|run| run.1 < hour) {
        *at += 1;
    }
    runs.get(*at).filter(|run| run.0 <= hour).and_then(|run| run.2)
}

fn mixes_over(first: i32, last: i32, known: &[(i32, i32, Option<i64>)], tokens: &[(u32, &read::Runs)]) -> MixRuns {
    let mut edges = vec![first];
    for &(from, to, _) in known.iter().chain(tokens.iter().flat_map(|(_, runs)| runs.iter())) {
        edges.extend([from, to.saturating_add(1)]);
    }
    edges.retain(|&edge| first <= edge && edge <= last);
    edges.sort_unstable();
    edges.dedup();
    let (mut known_at, mut at) = (0, vec![0; tokens.len()]);
    let mut out: MixRuns = Vec::new();
    for (index, &start) in edges.iter().enumerate() {
        let end = edges.get(index + 1).map_or(last, |next| next - 1);
        if holding(known, &mut known_at, start).is_none() {
            continue;
        }
        let mix: BTreeMap<u32, i64> = tokens.iter().zip(&mut at).filter_map(|((token, runs), at)| holding(runs, at, start).map(|count| (*token, count))).collect();
        match out.last_mut() {
            Some(previous) if previous.1 + 1 == start && previous.2 == mix => previous.1 = end,
            _ => out.push((start, end, mix)),
        }
    }
    out
}

fn token_name(tokens: &TableView<'_>, slot: Option<u32>, token: u32) -> String {
    let unresolved = || format!("Unresolved sub-unit token {token}");
    let Ok(literal) = dashboard_declare::field(TableId::LexicalTokens, "literal") else { return unresolved() };
    let Some(slot) = slot else { return unresolved() };
    read::text(tokens, literal, slot).and_then(|bytes| std::str::from_utf8(bytes).ok()).map_or_else(unresolved, str::to_owned)
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<Family>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Family {
    #[expect(dead_code, reason = "names the series this request is routed by")]
    kind: String,
    measure: ArmyMeasure,
    count_for: ArmyAttribution,
    top: Option<usize>,
}

pub fn series_params_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<SeriesParams>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeriesParams {
    branch_id: String,
    family: Family,
    #[serde(default)]
    countries: Vec<Country>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    key: Option<NavigationKey>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyChartLine {
    pub country: Country,
    pub identities: Vec<ArmyIdentityRun>,
    pub summary: dashboard_query::chart::ChartSummary,
    pub membership: Option<Vec<ArmySpan>>,
    pub missing: Vec<dashboard_query::series::MissingSpan>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyChartReply {
    pub series: Vec<ArmyChartLine>,
}

fn hour(value: Option<f64>, name: &str) -> io::Result<i32> {
    value
        .filter(|value| value.is_finite())
        .map(|value| value.floor() as i32)
        .ok_or_else(|| invalid(format!("The request names no {name}")))
}

pub fn series(_source: &Source, query: &Query<'_>, product: &str, params: &Value, _context: &LoadContext) -> io::Result<Value> {
    let params: SeriesParams = dashboard_query::named::named(params).map_err(|error| invalid(format!("Invalid army series request: {error}")))?;
    let _ = &params.branch_id;
    let line = line(query, params.family.measure, params.family.count_for)?;
    let (from, to) = match product {
        "series-next-change" => (hour(params.first_hour, "firstHour")?, hour(params.last_hour, "lastHour")?),
        _ => (hour(params.from, "from")?, hour(params.to, "to")?),
    };
    match product {
        "series-summary" => {
            let width = params.width.filter(|width| (1..=8192).contains(width)).ok_or_else(|| invalid("Invalid width"))?;
            let top = params.family.top.ok_or_else(|| invalid("An army chart of chosen countries is served as tiles"))?;
            let (mut tags, mut charts, mut spans) = (Vec::new(), Vec::new(), Vec::new());
            for ranked in line.ranked(from, to, width, top)? {
                tags.push(Country::from(ranked.tag));
                charts.push((ranked.summary, ranked.missing));
                spans.push(ranked.membership.into_iter().map(|(first_hour, last_hour)| ArmySpan { first_hour, last_hour }).collect::<Vec<_>>());
            }
            let membership = Some(spans);
            let mut out = Vec::with_capacity(tags.len());
            for (at, (tag, (summary, missing))) in tags.into_iter().zip(charts).enumerate() {
                let parts = line.parts(&tag, from, to);
                out.push(ArmyChartLine {
                    identities: parts.iter().map(|part| ArmyIdentityRun { first_hour: part.first, last_hour: part.last, slot: part.slot, country_id: part.id }).collect(),
                    membership: membership.as_ref().map(|members: &Vec<Vec<ArmySpan>>| members[at].clone()),
                    country: tag,
                    summary,
                    missing,
                });
            }
            Ok(serde_json::to_value(ArmyChartReply { series: out })?)
        }
        "series-next-change" => {
            let key = params.key.ok_or_else(|| invalid("The request names no key"))?;
            let origin = params.from.map(|from| from.floor() as i32);
            let declared = dashboard_declare::series("army-totals").and_then(|series| series.measure(measure_name(params.family.measure, params.family.count_for))).ok_or_else(|| invalid("Army measure is not declared"))?;
            let tags = match params.family.top {
                Some(top) => {
                    let at = match (origin, key) {
                        (Some(origin), _) => origin.clamp(from, to),
                        (None, NavigationKey::End) => to,
                        (None, _) => from,
                    };
                    line.rank_at(at, top, false)?.unwrap_or_default().into_iter().map(Country::from).collect()
                }
                None => params.countries.clone(),
            };
            let hour = line.navigate(&tags, &[declared], origin, key, (from, to))?;
            Ok(serde_json::to_value(SeriesKeyReply { hour: hour.map(f64::from) })?)
        }
        _ => Err(invalid(format!("The army series does not answer {product}"))),
    }
}

#[cfg(test)]
mod tests;

enum RunKey {}

type CachedRuns = dashboard_base::queries::RunCache<RunKey, Vec<(u128, i64, i64)>>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    CachedRuns::forget(source);
}

pub fn cached_army_mix(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<Vec<(u128, i64, i64)>>> {
    CachedRuns::held(&source.state::<CachedRuns>(), run, || dashboard_query::cost::lifetimes(source.replay_ref().recording(), run, replay_data::rows::TableId::ArmyMix))
}
