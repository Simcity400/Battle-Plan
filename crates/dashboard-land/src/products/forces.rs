
use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, Product};
use super::army::ArmyIdentityRun;
use dashboard_world::products::army_windows::ArmySpan;
use dashboard_base::Source;
use dashboard_base::{
    catalogue::{self, Catalogue, View},
};
use dashboard_declare::{Measure, Series};
use dashboard_query::{
    Query,
    identity::Holder,
    read,
    series::{Line, Part},
};
use content_reader_core::unit_leader::{SubUnit, SubUnits};
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{AirForceWingSignatureRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const UNIDENTIFIED: &str = ":not-identified";
const NOT_KNOWN: &str = ":not-known";
const UNRESOLVED: &str = ":";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub struct AirForceSize;
pub struct NavySize;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AirForceMeasure {
    Planes,
    Wings,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AirForceOwnership {
    Roster,
    Effective,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavyMeasure {
    Ships,
    Strength,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavyOwnership {
    Roster,
    Logical,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavyGrouping {
    Class,
    Type,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirForceParams {
    pub branch_id: String,
    pub countries: Vec<Country>,
    pub top: Option<usize>,
    pub measure: AirForceMeasure,
    pub ownership: AirForceOwnership,
    #[doc = " None is the window manifest; a window's runs name it by its index."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub window: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavyParams {
    pub branch_id: String,
    pub countries: Vec<Country>,
    pub top: Option<usize>,
    pub ownership: NavyOwnership,
    pub grouping: NavyGrouping,
    pub measure: NavyMeasure,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub window: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForcePart {
    pub key: String,
    pub name: Option<String>,
    pub unresolved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    #[doc = " Null where the total is not recorded."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub total: Option<i64>,
    #[doc = " Parallel to the reply's parts; every one null where the hour's rows do not split the total."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number | null>"))]
    pub values: Vec<Option<i64>>,
    #[doc = " The navy's maximum hull HP (fixed5) for the strength measure, null for ships or where not"]
    #[doc = " recorded; absent for the air force."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, type = "number | null"))]
    pub reference_total: Option<Option<i64>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceSeries {
    pub country: Country,
    pub identities: Vec<ArmyIdentityRun>,
    pub runs: Vec<ForceRun>,
    pub segments: Vec<ArmySpan>,
    pub membership: Option<Vec<ArmySpan>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceSizeReply {
    pub branch_id: String,
    pub windows: Vec<ArmySpan>,
    pub window: Option<usize>,
    pub interval: Option<ArmySpan>,
    pub parts: Vec<ForcePart>,
    pub series: Vec<ForceSeries>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ForceSizeReply);

#[derive(Clone, Copy)]
struct Race {
    totals: &'static str,
    total: &'static str,
    reference: Option<&'static str>,
    parts: &'static str,
    part: &'static str,
    ownership_shift: u32,
    ownership: u128,
}

fn declared(kind: &str) -> io::Result<&'static Series> {
    dashboard_declare::series(kind).ok_or_else(|| invalid(format!("{kind} is not declared")))
}

fn measure(series: &'static Series, name: &str) -> io::Result<&'static Measure> {
    series.measure(name).ok_or_else(|| invalid(format!("{} has no measure {name}", series.kind)))
}

fn air_race(measure: AirForceMeasure, ownership: AirForceOwnership) -> Race {
    let (total, owner) = match (measure, ownership) {
        (AirForceMeasure::Planes, AirForceOwnership::Roster) => ("rosterPlanes", 0),
        (AirForceMeasure::Wings, AirForceOwnership::Roster) => ("rosterWings", 0),
        (AirForceMeasure::Planes, AirForceOwnership::Effective) => ("effectivePlanes", 1),
        (AirForceMeasure::Wings, AirForceOwnership::Effective) => ("effectiveWings", 1),
    };
    match measure {
        AirForceMeasure::Planes => Race { totals: "air-force-totals", total, reference: None, parts: "air-force-parts", part: "planes", ownership_shift: 65, ownership: owner },
        AirForceMeasure::Wings => Race { totals: "air-force-totals", total, reference: None, parts: "air-force-wing-kinds", part: "wings", ownership_shift: 32, ownership: owner },
    }
}

fn navy_race(measure: NavyMeasure, ownership: NavyOwnership) -> Race {
    let (total, reference, owner) = match (measure, ownership) {
        (NavyMeasure::Ships, NavyOwnership::Roster) => ("rosterShips", "rosterMaxStrength", 0),
        (NavyMeasure::Strength, NavyOwnership::Roster) => ("rosterStrength", "rosterMaxStrength", 0),
        (NavyMeasure::Ships, NavyOwnership::Logical) => ("logicalShips", "logicalMaxStrength", 1),
        (NavyMeasure::Strength, NavyOwnership::Logical) => ("logicalStrength", "logicalMaxStrength", 1),
    };
    let part = if measure == NavyMeasure::Ships { "ships" } else { "strength" };
    Race { totals: "navy-totals", total, reference: Some(reference), parts: "navy-classes", part, ownership_shift: 32, ownership: owner }
}

fn validate_selection(branch_id: &str, tags: &[Country], top: Option<usize>) -> Result<(), &'static str> {
    if branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    if top.is_some_and(|top| top == 0 || !tags.is_empty()) {
        return Err("Invalid country selection");
    }
    for tag in tags {
        dashboard_base::products::validate_country(tag)?;
    }
    Ok(())
}

impl Product for AirForceSize {
    const KEY: &'static str = "air-force-size";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirForceParams;
    type Reply = ForceSizeReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            AirForceParams::decl(),
            ForceSizeReply::decl(),
            AirForceMeasure::decl(),
            AirForceOwnership::decl(),
            ForcePart::decl(),
            ForceRun::decl(),
            ForceSeries::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_selection(&params.branch_id, &params.countries, params.top)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let race = air_race(params.measure, params.ownership);
        let asked = Asked { branch_id: &params.branch_id, tags: &params.countries, top: params.top, window: params.window };
        window_reply(source, &query, race, asked, cancelled, |recording, run, hour, parts, warnings| {
            let content = Content::read(source, context, "Air equipment type names", warnings);
            match params.measure {
                AirForceMeasure::Planes => air_planes(recording, run, hour, &parts.keys().copied().collect(), &content, warnings),
                AirForceMeasure::Wings => air_wings(recording, run, hour, parts, &content, warnings),
            }
        })
    }
}

impl Product for NavySize {
    const KEY: &'static str = "navy-size";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavyParams;
    type Reply = ForceSizeReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavyParams::decl(), ForceSizeReply::decl(), NavyMeasure::decl(), NavyOwnership::decl(), NavyGrouping::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_selection(&params.branch_id, &params.countries, params.top)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let race = navy_race(params.measure, params.ownership);
        let asked = Asked { branch_id: &params.branch_id, tags: &params.countries, top: params.top, window: params.window };
        let mut reply = window_reply(source, &query, race, asked, cancelled, |recording, run, hour, parts, warnings| {
            let content = Content::read(source, context, "Ship class names", warnings);
            navy_classes(recording, run, hour, &parts.keys().copied().collect(), &content, params.grouping, warnings)
        })?;
        if params.measure == NavyMeasure::Ships {
            for run in reply.series.iter_mut().flat_map(|series| series.runs.iter_mut()) {
                run.reference_total = Some(None);
            }
        }
        Ok(reply)
    }
}

#[derive(Clone, Copy)]
struct Asked<'a> {
    branch_id: &'a str,
    tags: &'a [Country],
    top: Option<usize>,
    window: Option<usize>,
}

#[derive(Default)]
struct Named {
    parts: Vec<ForcePart>,
    of: HashMap<u128, usize>,
}

impl Named {
    fn group(resolved: BTreeMap<u128, ForcePart>) -> Self {
        let mut keys: BTreeMap<String, ForcePart> = BTreeMap::new();
        for part in resolved.values() {
            keys.entry(part.key.clone()).or_insert_with(|| part.clone());
        }
        let parts: Vec<ForcePart> = keys.into_values().collect();
        let at: HashMap<&str, usize> = parts.iter().enumerate().map(|(index, part)| (part.key.as_str(), index)).collect();
        let of = resolved.iter().map(|(part, named)| (*part, at[named.key.as_str()])).collect();
        Self { parts, of }
    }
}

fn holding(runs: &[(i32, i32, Option<i64>)], at: &mut usize, hour: i32) -> Option<i64> {
    while runs.get(*at).is_some_and(|run| run.1 < hour) {
        *at += 1;
    }
    runs.get(*at).filter(|run| run.0 <= hour).and_then(|run| run.2)
}

fn split(total: Option<i64>, held: &[(usize, Option<i64>)], groups: usize) -> Vec<Option<i64>> {
    let Some(total) = total else { return vec![None; groups] };
    let mut known = 0i128;
    let mut out = vec![Some(0i64); groups];
    for &(group, value) in held {
        let Some(value) = value else { continue };
        known += i128::from(value);
        out[group] = out[group].and_then(|sum| sum.checked_add(value));
    }
    if known != i128::from(total) || out.iter().any(Option::is_none) {
        return vec![None; groups];
    }
    out
}

pub fn segments(query: &Query<'_>, tag: &Country, first: i32, last: i32) -> Vec<ArmySpan> {
    query
        .holders
        .runs(tag)
        .iter()
        .filter(|run| matches!(run.holder, Holder::Found { .. }) && run.first <= last && run.last >= first)
        .map(|run| ArmySpan { first_hour: run.first, last_hour: run.last.min(last) })
        .collect()
}

pub fn identities(parts: &[Part]) -> Vec<ArmyIdentityRun> {
    parts.iter().fold(Vec::<ArmyIdentityRun>::new(), |mut held, part| {
        match held.last_mut() {
            Some(last) if last.slot == part.slot && last.country_id == part.id && last.last_hour + 1 == part.first => last.last_hour = part.last,
            _ => held.push(ArmyIdentityRun { first_hour: part.first, last_hour: part.last, slot: part.slot, country_id: part.id }),
        }
        held
    })
}

type Owned<'r> = Vec<(usize, &'r [(i32, i32, Option<i64>)])>;

fn part_runs(part: &Part, lo: i32, hi: i32, totals: &[(i32, i32, Option<i64>)], reference: Option<&[(i32, i32, Option<i64>)]>, owned: &Owned<'_>, groups: usize, out: &mut Vec<ForceRun>) {
    let mut edges = vec![lo];
    for &(from, to, _) in totals.iter().chain(reference.into_iter().flatten()).chain(owned.iter().flat_map(|(_, runs)| runs.iter())) {
        edges.extend([from, to.saturating_add(1)]);
    }
    edges.retain(|&edge| lo <= edge && edge <= hi);
    edges.sort_unstable();
    edges.dedup();
    let (mut total_at, mut reference_at, mut at) = (0, 0, vec![0; owned.len()]);
    for (index, &start) in edges.iter().enumerate() {
        let end = edges.get(index + 1).map_or(hi, |next| next - 1);
        let total = holding(totals, &mut total_at, start);
        let reference_total = reference.map(|runs| holding(runs, &mut reference_at, start));
        let held: Vec<(usize, Option<i64>)> = owned.iter().zip(&mut at).map(|((group, runs), at)| (*group, holding(runs, at, start))).collect();
        let values = split(total, &held, groups);
        let starts = part.starts && start == part.first;
        match out.last_mut() {
            Some(previous) if !starts && previous.last_hour + 1 == start && previous.total == total && previous.values == values && previous.reference_total == reference_total => {
                previous.last_hour = end;
            }
            _ => out.push(ForceRun { first_hour: start, last_hour: end, starts_segment: starts, total, values, reference_total }),
        }
    }
}

fn window_reply(
    source: &Source,
    query: &Query<'_>,
    race: Race,
    asked: Asked<'_>,
    cancelled: &AtomicBool,
    name: impl FnOnce(&Recording, u32, i32, &BTreeMap<u128, i32>, &mut Vec<String>) -> io::Result<Named>,
) -> io::Result<ForceSizeReply> {
    let windows = dashboard_world::products::army_windows::windows(query)?;
    let interval = asked.window.map(|index| windows.get(index).cloned().ok_or_else(|| invalid("Unknown history window"))).transpose()?;
    let mut reply = ForceSizeReply {
        branch_id: asked.branch_id.to_owned(),
        windows: if asked.window.is_none() { windows } else { Vec::new() },
        window: asked.window,
        interval: interval.clone(),
        parts: Vec::new(),
        series: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(range) = interval else { return Ok(reply) };
    let (first, last) = (range.first_hour, range.last_hour);
    let run = query.timeline.at(first).ok_or_else(|| invalid("The window's hours are not recorded"))?.run;
    let totals = declared(race.totals)?;
    let line = Line::new(query, totals, measure(totals, race.total)?)?;
    let membership: Option<BTreeMap<Country, Vec<(i32, i32)>>> = match asked.top {
        Some(top) => {
            let width = ((i64::from(last) - i64::from(first) + 1) / 6).max(1) as u32;
            Some(line.ranked(first, last, width, top)?.into_iter().map(|line| (Country::from(line.tag), line.membership)).collect())
        }
        None => None,
    };
    let tags: Vec<Country> = match &membership {
        Some(members) => members.keys().cloned().collect(),
        None => {
            let mut seen = BTreeSet::new();
            asked.tags.iter().filter(|tag| seen.insert((*tag).clone())).cloned().collect()
        }
    };
    let mut held: BTreeMap<Country, Vec<Part>> = BTreeMap::new();
    for tag in &tags {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let parts: Vec<Part> = line.parts(tag, first, last).into_iter().filter(|part| part.run == run).collect();
        if !parts.is_empty() || asked.top.is_none() {
            held.insert(tag.clone(), parts);
        }
    }
    let mut owners: Vec<u128> = held.values().flatten().map(|part| part.key).collect();
    owners.sort_unstable();
    owners.dedup();
    let recording = query.replay.recording();
    let total_runs = if owners.is_empty() { HashMap::new() } else { read::runs(recording, run, totals.table, line.source(), Some(&owners), first, last)? };
    let reference_runs = match race.reference {
        Some(name) if !owners.is_empty() => read::runs(recording, run, totals.table, dashboard_declare::source(totals.table, measure(totals, name)?)?, Some(&owners), first, last)?,
        _ => HashMap::new(),
    };
    let parts_series = declared(race.parts)?;
    let spec = parts_series.parts.ok_or_else(|| invalid(format!("{} has no parts", parts_series.kind)))?;
    let lives = dashboard_base::queries::lifetimes(source, run, parts_series.table)?;
    let mut keys = Vec::new();
    for &owner in &owners {
        let (lo, hi) = spec.range(owner);
        let start = lives.partition_point(|&(key, ..)| key < lo);
        keys.extend(
            lives[start..]
                .iter()
                .take_while(|&&(key, ..)| key <= hi)
                .filter(|&&(key, born, gone)| born <= i64::from(last) && gone >= i64::from(first) && (spec.part(key) >> race.ownership_shift) & 0xff == race.ownership)
                .map(|&(key, _, gone)| (key, gone.min(i64::from(last)) as i32)),
        );
    }
    let mut met: BTreeMap<u128, i32> = BTreeMap::new();
    for &(key, at) in &keys {
        let held = met.entry(spec.part(key)).or_insert(at);
        *held = (*held).max(at);
    }
    let keys: Vec<u128> = keys.into_iter().map(|(key, _)| key).collect();
    let part_source = dashboard_declare::source(parts_series.table, measure(parts_series, race.part)?)?;
    let parts_read = if keys.is_empty() { HashMap::new() } else { read::runs(recording, run, parts_series.table, part_source, Some(&keys), first, last)? };
    let named = name(recording, run, last, &met, &mut reply.warnings)?;
    let groups = named.parts.len();
    let mut by_owner: HashMap<u128, Owned<'_>> = HashMap::new();
    for (key, runs) in &parts_read {
        if let Some(&group) = named.of.get(&spec.part(*key)) {
            by_owner.entry(spec.owner(*key)).or_default().push((group, runs.as_slice()));
        }
    }
    for (tag, parts) in &held {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let identities = identities(parts);
        let spans: Vec<(i32, i32)> = match &membership {
            Some(members) => members.get(tag).cloned().unwrap_or_default(),
            None => identities.iter().map(|identity| (identity.first_hour, identity.last_hour)).collect(),
        };
        let mut runs = Vec::new();
        for part in parts {
            let totals = total_runs.get(&part.key).map_or(&[][..], Vec::as_slice);
            let reference = race.reference.map(|_| reference_runs.get(&part.key).map_or(&[][..], Vec::as_slice));
            let empty = Owned::new();
            let owned = by_owner.get(&part.key).unwrap_or(&empty);
            for &(span_first, span_last) in &spans {
                let (lo, hi) = (span_first.max(part.first), span_last.min(part.last));
                if lo <= hi {
                    part_runs(part, lo, hi, totals, reference, owned, groups, &mut runs);
                }
            }
        }
        reply.series.push(ForceSeries {
            country: tag.clone(),
            identities: if asked.top.is_none() { identities } else { Vec::new() },
            runs,
            segments: segments(query, tag, first, last),
            membership: membership.as_ref().map(|members| members.get(tag).into_iter().flatten().map(|&(first_hour, last_hour)| ArmySpan { first_hour, last_hour }).collect()),
        });
    }
    let mut named = named;
    if named.parts.is_empty() && reply.series.iter().flat_map(|series| &series.runs).any(|run| run.total.is_some_and(|total| total != 0)) {
        named.parts.push(ForcePart { key: NOT_KNOWN.into(), name: Some("Not known".into()), unresolved: true });
        for run in reply.series.iter_mut().flat_map(|series| series.runs.iter_mut()) {
            run.values = vec![if run.total == Some(0) { Some(0) } else { None }];
        }
    }
    let unresolved = named.parts.iter().filter(|part| part.unresolved).count();
    if unresolved > 0 {
        reply.warnings.push(format!("{unresolved} parts could not be resolved through the recorded content; each remains visible."));
    }
    reply.parts = named.parts;
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

struct Content {
    catalogue: Option<Arc<Catalogue>>,
}

impl Content {
    fn read(source: &Source, context: &LoadContext, what: &str, warnings: &mut Vec<String>) -> Self {
        let catalogue = match context.content().and_then(|options| catalogue::catalogue(source, options)) {
            Ok(catalogue) => {
                warnings.extend(catalogue.refusal_lines());
                Some(catalogue)
            }
            Err(error) => {
                warnings.push(format!("{what} are unavailable: {error}"));
                None
            }
        };
        Self { catalogue }
    }

    fn localise(&self, key: &str) -> Option<String> {
        let catalogue = self.catalogue.as_ref()?;
        catalogue.localisation.get(key)?;
        catalogue.localised(key, View::Plain)
    }
}

fn literals(recording: &Recording, run: u32, hour: i32, tokens: &BTreeSet<u32>) -> io::Result<HashMap<u32, String>> {
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let literal = dashboard_declare::field(TableId::LexicalTokens, "literal")?;
    let keys: Vec<u128> = tokens.iter().map(|&token| u128::from(token)).collect();
    let held = read::at(recording, run, &[read::Read { table: TableId::LexicalTokens, columns: Some(&[literal]), keys: read::Keys::These(&keys) }], hour)?;
    let view = held[0].view();
    Ok(held[0]
        .rows
        .iter()
        .filter_map(|&(key, slot)| Some((key as u32, std::str::from_utf8(read::text(&view, literal, slot)?).ok()?.to_owned())))
        .collect())
}

fn variant_tokens(recording: &Recording, run: u32, hour: i32, variants: &BTreeSet<u64>) -> io::Result<HashMap<u64, String>> {
    if variants.is_empty() {
        return Ok(HashMap::new());
    }
    let column = dashboard_declare::field(TableId::VariantDefinitions, "equipment_token")?;
    let keys: Vec<u128> = variants.iter().map(|&variant| u128::from(variant)).collect();
    let held = read::at(recording, run, &[read::Read { table: TableId::VariantDefinitions, columns: Some(&[column]), keys: read::Keys::These(&keys) }], hour)?;
    let view = held[0].view();
    let tokens: HashMap<u64, u32> = held[0].rows.iter().filter_map(|&(key, slot)| Some((key as u64, u32::try_from(view.number(column, slot)?).ok()?))).collect();
    let spelled = literals(recording, run, hour, &tokens.values().copied().collect())?;
    Ok(tokens.into_iter().filter_map(|(variant, token)| Some((variant, spelled.get(&token)?.clone()))).collect())
}

fn variant_part(variant: u64, token: Option<&str>, units: Option<&content_reader_core::equipment::Database>, content: &Content, warnings: &mut BTreeSet<String>) -> ForcePart {
    if let (Some(token), Some(units)) = (token, units) {
        let archetype = units
            .named(token)
            .ok_or_else(|| format!("{token} is not authored equipment"))
            .and_then(|kind| kind.archetype.ok_or_else(|| format!("equipment {token} has no known archetype")))
            .and_then(|ordinal| units.equipment[ordinal].name_str().map(str::to_owned).map_err(|error| error.to_string()));
        match archetype {
            Ok(key) if !key.starts_with(UNRESOLVED) => return ForcePart { name: content.localise(&key), key, unresolved: false },
            Ok(key) => {
                warnings.insert(format!("{token}: archetype {key} uses the reserved unresolved mark; its variants remain separate."));
            }
            Err(reason) => {
                warnings.insert(format!("{reason}; its variants remain separate."));
            }
        }
    }
    let key = match token {
        Some(token) => format!("{UNRESOLVED}variant/{variant}/{token}"),
        None => format!("{UNRESOLVED}variant/{variant}"),
    };
    ForcePart { key, name: Some(format!("Variant {variant}")), unresolved: true }
}

fn unidentified() -> ForcePart {
    ForcePart { key: UNIDENTIFIED.into(), name: Some("Other / not identified".into()), unresolved: true }
}

fn equipment(content: &Content, warnings: &mut Vec<String>) -> Option<Arc<content_reader_core::equipment::Database>> {
    let catalogue = content.catalogue.as_ref()?;
    match catalogue::equipment_units(catalogue) {
        Ok(units) => {
            warnings.extend(units.misreads.iter().cloned());
            Some(units)
        }
        Err(error) => {
            warnings.push(format!("Air equipment content is unavailable: {error}"));
            None
        }
    }
}

fn air_planes(recording: &Recording, run: u32, hour: i32, parts: &BTreeSet<u128>, content: &Content, warnings: &mut Vec<String>) -> io::Result<Named> {
    let variant_of = |part: u128| ((part >> 64) & 1 == 1).then_some(part as u64);
    let variants: BTreeSet<u64> = parts.iter().filter_map(|&part| variant_of(part)).collect();
    let tokens = variant_tokens(recording, run, hour, &variants)?;
    let units = equipment(content, warnings);
    let mut notes = BTreeSet::new();
    let resolved = parts
        .iter()
        .map(|&part| {
            let named = match variant_of(part) {
                Some(variant) => variant_part(variant, tokens.get(&variant).map(String::as_str), units.as_deref(), content, &mut notes),
                None => unidentified(),
            };
            (part, named)
        })
        .collect();
    warnings.extend(notes);
    Ok(Named::group(resolved))
}

fn air_wings(recording: &Recording, run: u32, hour: i32, met: &BTreeMap<u128, i32>, content: &Content, warnings: &mut Vec<String>) -> io::Result<Named> {
    let parts: BTreeSet<u128> = met.keys().copied().collect();
    let mut by_hour: BTreeMap<i32, Vec<u128>> = BTreeMap::new();
    for (&part, &at) in met {
        by_hour.entry(at).or_default().push(u128::from(part as u32));
    }
    let mut signatures: HashMap<u32, Vec<u64>> = HashMap::new();
    if !by_hour.is_empty() {
        let column = dashboard_declare::field(TableId::AirForceWingSignatures, "variants")?;
        for (at, mut keys) in by_hour {
            keys.sort_unstable();
            keys.dedup();
            let held = read::at(recording, run, &[read::Read { table: TableId::AirForceWingSignatures, columns: Some(&[column]), keys: read::Keys::These(&keys) }], at)?;
            let view = held[0].view();
            for &(key, slot) in &held[0].rows {
                let bytes = view.list(column, slot).or_else(|| view.text(column, slot).and_then(|id| view.string(id)));
                let Some(bytes) = bytes else { continue };
                match (AirForceWingSignatureRow { number: key as u32, variants: bytes.to_vec() }).variants() {
                    Ok(variants) => {
                        signatures.insert(key as u32, variants);
                    }
                    Err(error) => warnings.push(format!("Wing composition {key}: {error}")),
                }
            }
        }
    }
    let parts = &parts;
    let variants: BTreeSet<u64> = signatures.values().flatten().copied().collect();
    let tokens = variant_tokens(recording, run, hour, &variants)?;
    let units = equipment(content, warnings);
    let mut notes = BTreeSet::new();
    let mut resolved = BTreeMap::new();
    for &part in parts {
        let named = match signatures.get(&(part as u32)) {
            None => ForcePart { key: format!("{UNRESOLVED}composition/{}", part as u32), name: Some(format!("Wing composition {}", part as u32)), unresolved: true },
            Some(variants) if variants.is_empty() => unidentified(),
            Some(variants) => {
                let mut kinds: BTreeMap<String, ForcePart> = BTreeMap::new();
                for &variant in variants {
                    let named = variant_part(variant, tokens.get(&variant).map(String::as_str), units.as_deref(), content, &mut notes);
                    kinds.insert(named.key.clone(), named);
                }
                if kinds.len() == 1 {
                    kinds.into_values().next().expect("one kind")
                } else {
                    let keys: Vec<&String> = kinds.keys().collect();
                    ForcePart {
                        key: format!("mixed:{}", serde_json::to_string(&keys).expect("string keys")),
                        name: Some(format!("Mixed types: {}", kinds.values().map(|kind| kind.name.as_deref().unwrap_or("Equipment type name unavailable")).collect::<Vec<_>>().join(" / "))),
                        unresolved: kinds.values().any(|kind| kind.unresolved),
                    }
                }
            }
        };
        resolved.insert(part, named);
    }
    warnings.extend(notes);
    Ok(Named::group(resolved))
}

fn navy_classes(recording: &Recording, run: u32, hour: i32, parts: &BTreeSet<u128>, content: &Content, grouping: NavyGrouping, warnings: &mut Vec<String>) -> io::Result<Named> {
    let tokens: BTreeSet<u32> = parts.iter().map(|&part| part as u32).collect();
    let spelled = literals(recording, run, hour, &tokens)?;
    let units: Option<Arc<SubUnits>> = content.catalogue.as_ref().and_then(|catalogue| {
        match catalogue.content.layers.derived::<catalogue::CachedResult<SubUnits>>().get_or_init(|| content_reader_core::unit_leader::sub_units(&catalogue.content.layers)) {
            Ok(units) => {
                warnings.extend(units.warnings().iter().cloned());
                Some(units)
            }
            Err(error) => {
                warnings.push(format!("Ship class content is unavailable: {error}"));
                None
            }
        }
    });
    let mut resolved = BTreeMap::new();
    for &part in parts {
        let token = part as u32;
        let spelling = spelled.get(&token);
        let mut refusal = None;
        let found = spelling.zip(units.as_deref()).and_then(|(spelling, units)| {
            let SubUnit::Index(index) = units.get(spelling) else { return None };
            let registered = units.name(index)?.ok()?;
            match grouping {
                NavyGrouping::Class => Some((format!("class:{registered}"), content.localise(registered))),
                NavyGrouping::Type => {
                    let mask = units.category(registered.as_bytes()).ok()?;
                    let names = match content_reader_core::equipment::category_names(mask) {
                        Ok(names) if !names.is_empty() => names,
                        Ok(_) => return None,
                        Err(bits) => {
                            refusal = Some(format!("unknown native category bits {bits:#x}"));
                            return None;
                        }
                    };
                    let name = names.iter().map(|key| content.localise(key)).collect::<Option<Vec<_>>>().map(|names| names.join(" / "));
                    Some((format!("type:{mask}"), name))
                }
            }
        });
        let named = match found {
            Some((key, name)) => ForcePart { key, name, unresolved: false },
            None => {
                let visible = spelling.filter(|name| !name.is_empty()).cloned().unwrap_or_else(|| format!("Other / not identified (Sub-unit {token})"));
                let reason = refusal.as_deref().unwrap_or("the recorded content does not resolve this ship class or broad role");
                warnings.push(format!("{visible}: {reason}; kept as its own part."));
                ForcePart {
                    key: format!("{UNRESOLVED}ship:{}", serde_json::to_string(&(token, spelling)).expect("class identity")),
                    name: Some("Ship class name unavailable".into()),
                    unresolved: true,
                }
            }
        };
        resolved.insert(part, named);
    }
    Ok(Named::group(resolved))
}

#[cfg(test)]
mod tests;
