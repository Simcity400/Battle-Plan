
use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, Product};
use dashboard_land::products::forces::{self, ForcePart, ForceRun, ForceSeries, ForceSizeReply};
use dashboard_world::products::army_windows::{ArmySpan, windows};
use dashboard_base::Source;
use dashboard_declare::Key;
use dashboard_query::{
    Query,
    identity::Holder,
    read,
    series::{Line, Part},
};
use game_model::calendar::{HOUR_EPOCH, MONTH_LENGTHS};
use replay_recording::columnar_frames::Recording;
use replay_columnar::summary;
use replay_data::rows::{NavalLossTotalRow as Row, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TOTAL: usize = 0;
const UNKNOWN: usize = 1;
const COUNTED_FROM: usize = 2;
const KNOWN: usize = 3;
const LOST_FROM: usize = 4;
const TABLE: TableId = TableId::NavalLossTotals;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavalAttribution {
    Lost,
    SunkBy,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavalKind {
    Ships,
    Convoys,
    Both,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavalPeriod {
    Running,
    Month,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalLossesParams {
    pub branch_id: String,
    pub countries: Vec<Country>,
    pub top: Option<usize>,
    pub attribution: NavalAttribution,
    pub kind: NavalKind,
    pub period: NavalPeriod,
    #[doc = " None is the window manifest; a window's runs name it by its index."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub window: Option<usize>,
}

impl NavalAttribution {
    fn code(self) -> u8 {
        match self {
            Self::Lost => Row::LOST,
            Self::SunkBy => Row::SUNK_BY,
        }
    }
}

impl NavalKind {
    pub fn kinds(self) -> &'static [u8] {
        match self {
            Self::Ships => &[Row::SHIPS],
            Self::Convoys => &[Row::CONVOYS],
            Self::Both => &[Row::SHIPS, Row::CONVOYS],
        }
    }
}

type Runs = Vec<(i32, i32, Option<i64>)>;

fn at(runs: Option<&Runs>, hour: i32) -> Option<i64> {
    let runs = runs?;
    let index = runs.partition_point(|run| run.1 < hour);
    runs.get(index).filter(|run| run.0 <= hour).and_then(|run| run.2)
}

pub fn month_start(hour: i32) -> i32 {
    let elapsed = i64::from(hour) - i64::from(HOUR_EPOCH);
    let year = elapsed.div_euclid(365 * 24);
    let mut day = elapsed.rem_euclid(365 * 24) / 24;
    let mut before = 0i64;
    for length in MONTH_LENGTHS {
        if day < i64::from(length) {
            break;
        }
        day -= i64::from(length);
        before += i64::from(length);
    }
    (i64::from(HOUR_EPOCH) + year * 365 * 24 + before * 24) as i32
}

fn month_starts(first: i32, last: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let mut start = month_start(first);
    loop {
        let length = MONTH_LENGTHS[month_index(start)];
        start += length * 24;
        if start > last {
            return out;
        }
        out.push(start);
    }
}

fn month_index(start: i32) -> usize {
    let mut day = (i64::from(start) - i64::from(HOUR_EPOCH)).rem_euclid(365 * 24) / 24;
    for (index, length) in MONTH_LENGTHS.into_iter().enumerate() {
        if day < i64::from(length) {
            return index;
        }
        day -= i64::from(length);
    }
    11
}

pub struct Counts {
    known: HashMap<u128, Runs>,
    counted_from: HashMap<u128, Runs>,
    totals: HashMap<u128, Runs>,
}

impl Counts {
    pub fn read(recording: &Recording, run: u32, kinds: &[u8], keys: &[u128], from: i32, to: i32) -> io::Result<Self> {
        let runs: Vec<u128> = kinds.iter().map(|&kind| Row::run_key(kind)).collect();
        let column = |column: usize, keys: &[u128]| -> io::Result<HashMap<u128, Runs>> {
            if keys.is_empty() {
                return Ok(HashMap::new());
            }
            read::runs(recording, run, TABLE, summary::Source::Cell(column), Some(keys), from, to)
        };
        Ok(Self { known: column(KNOWN, &runs)?, counted_from: column(COUNTED_FROM, &runs)?, totals: column(TOTAL, keys)? })
    }

    #[cfg(test)]
    pub fn of(known: HashMap<u128, Runs>, counted_from: HashMap<u128, Runs>, totals: HashMap<u128, Runs>) -> Self {
        Self { known, counted_from, totals }
    }

    pub fn baseline(&self, kind: u8, hour: i32) -> Option<i32> {
        let key = Row::run_key(kind);
        (at(self.known.get(&key), hour)? != 0).then_some(())?;
        let from = i32::try_from(at(self.counted_from.get(&key), hour)?).ok()?;
        (from <= hour).then_some(from)
    }

    pub fn running(&self, kind: u8, key: u128, hour: i32) -> Option<i64> {
        self.baseline(kind, hour)?;
        Some(at(self.totals.get(&key), hour).unwrap_or(0))
    }

    pub fn count(&self, kind: u8, key: u128, hour: i32, period: NavalPeriod) -> Option<i64> {
        let now = self.running(kind, key, hour)?;
        match period {
            NavalPeriod::Running => Some(now),
            NavalPeriod::Month => {
                let base = (month_start(hour) - 1).max(self.baseline(kind, hour)?);
                if base >= hour {
                    return Some(0);
                }
                now.checked_sub(self.running(kind, key, base)?)
            }
        }
    }

    fn edges(&self, out: &mut Vec<i32>) {
        for runs in self.known.values().chain(self.counted_from.values()).chain(self.totals.values()) {
            for &(first, last, _) in runs {
                out.extend([first, last.saturating_add(1)]);
            }
        }
    }
}

fn validate(params: &NavalLossesParams) -> Result<(), &'static str> {
    if params.branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    if params.top.is_some_and(|top| top == 0 || !params.countries.is_empty()) {
        return Err("Invalid country selection");
    }
    params.countries.iter().try_for_each(dashboard_base::products::validate_country)
}

fn parts(kind: NavalKind) -> Vec<ForcePart> {
    kind.kinds()
        .iter()
        .map(|&kind| {
            let (key, name) = if kind == Row::SHIPS { ("ships", "Ships") } else { ("convoys", "Convoys") };
            ForcePart { key: key.into(), name: Some(name.into()), unresolved: false }
        })
        .collect()
}

fn ranked(query: &Query<'_>, run: u32, params: &NavalLossesParams, hour: i32, top: usize, warnings: &mut Vec<String>) -> io::Result<Vec<String>> {
    let recording = query.replay.recording();
    let kinds = params.kind.kinds();
    let attribution = params.attribution.code();
    let mut held: Vec<(String, u128)> = Vec::new();
    for (tag, runs) in &query.holders.tags {
        if let Some(Holder::Found { slot, id }) = runs.iter().find(|held| held.first <= hour && hour <= held.last).map(|held| held.holder) {
            held.push((tag.clone(), Key::Holder.of(slot, id)));
        }
    }
    let at_hour = |hour: i32| -> io::Result<HashMap<u128, i64>> {
        let snapshots = read::at(recording, run, &[read::Read { table: TABLE, columns: Some(&[TOTAL][..]), keys: read::Keys::All }], hour)?;
        let snapshot = snapshots.first().ok_or_else(|| invalid("Naval loss totals were not read"))?;
        let view = snapshot.view();
        Ok(snapshot.rows.iter().filter_map(|&(key, slot)| Some((key, view.number(TOTAL, slot)?))).collect())
    };
    let counts = Counts::read(recording, run, kinds, &[], hour, hour)?;
    if kinds.iter().any(|&kind| counts.baseline(kind, hour).is_none()) {
        warnings.push("Naval losses are not known at the ranked hour: their tables were not read whole after the counting began.".into());
        return Ok(Vec::new());
    }
    let now = at_hour(hour)?;
    let bases = month_bases(&counts, kinds, hour, params.period);
    let mut before: HashMap<i32, HashMap<u128, i64>> = HashMap::new();
    if bases.iter().any(Option::is_some) {
        let stretch = query.timeline.at(hour).ok_or_else(|| invalid("The ranked hour is not recorded"))?;
        for &base in bases.iter().flatten() {
            if base < hour && base >= stretch.first && !before.contains_key(&base) {
                before.insert(base, at_hour(base)?);
            }
        }
    }
    let mut scored: Vec<(i64, String)> = held.into_iter().map(|(tag, owner)| (score(owner, attribution, kinds, &bases, hour, &now, &before), tag)).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(scored.into_iter().take(top).map(|(_, tag)| tag).collect())
}

pub(super) fn month_bases(counts: &Counts, kinds: &[u8], hour: i32, period: NavalPeriod) -> Vec<Option<i32>> {
    match period {
        NavalPeriod::Running => vec![None; kinds.len()],
        NavalPeriod::Month => kinds.iter().map(|&kind| Some((month_start(hour) - 1).max(counts.baseline(kind, hour).unwrap_or(hour)))).collect(),
    }
}

pub(super) fn score(owner: u128, attribution: u8, kinds: &[u8], bases: &[Option<i32>], hour: i32, now: &HashMap<u128, i64>, before: &HashMap<i32, HashMap<u128, i64>>) -> i64 {
    kinds.iter().zip(bases).fold(0i64, |sum, (&kind, base)| {
        if base.is_some_and(|base| base >= hour) {
            return sum;
        }
        let key = Row::holder_key(owner, attribution, kind);
        let held = base.and_then(|base| before.get(&base)).map_or(0, |before| before.get(&key).copied().unwrap_or(0));
        sum.saturating_add(now.get(&key).copied().unwrap_or(0) - held)
    })
}

fn part_runs(part: &Part, lo: i32, hi: i32, counts: &Counts, params: &NavalLossesParams, out: &mut Vec<ForceRun>) {
    let mut edges = vec![lo];
    counts.edges(&mut edges);
    if params.period == NavalPeriod::Month {
        edges.extend(month_starts(lo, hi));
    }
    edges.retain(|&edge| lo <= edge && edge <= hi);
    edges.sort_unstable();
    edges.dedup();
    let attribution = params.attribution.code();
    for (index, &start) in edges.iter().enumerate() {
        let end = edges.get(index + 1).map_or(hi, |next| next - 1);
        let owner = Key::Holder.of(part.slot, part.id);
        let values: Vec<Option<i64>> = params.kind.kinds().iter().map(|&kind| counts.count(kind, Row::holder_key(owner, attribution, kind), start, params.period)).collect();
        let total = values.iter().try_fold(0i64, |sum, value| sum.checked_add((*value)?));
        let starts = part.starts && start == part.first;
        match out.last_mut() {
            Some(previous) if !starts && previous.last_hour + 1 == start && previous.total == total && previous.values == values => previous.last_hour = end,
            _ => out.push(ForceRun { first_hour: start, last_hour: end, starts_segment: starts, total, values, reference_total: None }),
        }
    }
}

pub struct NavalLosses;

impl Product for NavalLosses {
    const KEY: &'static str = "naval-losses";
    const CONTENT_DEPENDENT: bool = false;
    type Params = NavalLossesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, attribution, kind, period, window, "countries[]"];
    type Reply = ForceSizeReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalLossesParams::decl(), ForceSizeReply::decl(), NavalAttribution::decl(), NavalKind::decl(), NavalPeriod::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(params)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        reply(&query, params, cancelled)
    }
}

pub fn reply(query: &Query<'_>, params: &NavalLossesParams, cancelled: &AtomicBool) -> io::Result<ForceSizeReply> {
    let windows = windows(query)?;
    let interval = params.window.map(|index| windows.get(index).cloned().ok_or_else(|| invalid("Unknown history window"))).transpose()?;
    let mut reply = ForceSizeReply {
        branch_id: params.branch_id.clone(),
        windows: if params.window.is_none() { windows } else { Vec::new() },
        window: params.window,
        interval: interval.clone(),
        parts: parts(params.kind),
        series: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(range) = interval else { return Ok(reply) };
    let (first, last) = (range.first_hour, range.last_hour);
    let stretches: Vec<_> = query.timeline.within(first, last).collect();
    let last_stretch = stretches.last().copied().ok_or_else(|| invalid("The window's hours are not recorded"))?;
    let series = dashboard_declare::series("naval-losses").ok_or_else(|| invalid("naval-losses is not declared"))?;
    let line = Line::new(query, series, series.measure("total").ok_or_else(|| invalid("naval-losses has no total"))?)?;
    let tags: Vec<Country> = match params.top {
        Some(top) => ranked(query, last_stretch.run, params, last_stretch.last, top, &mut reply.warnings)?.into_iter().map(Country::from).collect(),
        None => {
            let mut seen = BTreeSet::new();
            params.countries.iter().filter(|tag| seen.insert((*tag).clone())).cloned().collect()
        }
    };
    let mut held: BTreeMap<Country, Vec<Part>> = BTreeMap::new();
    for tag in &tags {
        dashboard_text::metadata::check_cancel(cancelled)?;
        held.insert(tag.clone(), line.parts(tag, first, last));
    }
    let kinds = params.kind.kinds();
    let attribution = params.attribution.code();
    let recording = query.replay.recording();
    let mut runs_by_tag: BTreeMap<Country, Vec<ForceRun>> = BTreeMap::new();
    for stretch in &stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut keys: Vec<u128> = held
            .values()
            .flatten()
            .filter(|part| part.run == stretch.run)
            .flat_map(|part| kinds.iter().map(move |&kind| Row::holder_key(Key::Holder.of(part.slot, part.id), attribution, kind)))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let full_first = query.timeline.at(stretch.first).map_or(stretch.first, |held| held.first);
        let from = match params.period {
            NavalPeriod::Running => stretch.first,
            NavalPeriod::Month => (month_start(stretch.first) - 1).clamp(full_first, stretch.first),
        };
        let counts = Counts::read(recording, stretch.run, kinds, &keys, from, stretch.last)?;
        for (tag, parts) in &held {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let runs = runs_by_tag.entry(tag.clone()).or_default();
            for part in parts.iter().filter(|part| part.run == stretch.run) {
                let (lo, hi) = (stretch.first.max(part.first), stretch.last.min(part.last));
                if lo <= hi {
                    part_runs(part, lo, hi, &counts, params, runs);
                }
            }
        }
        warn(recording, stretch.run, kinds, &keys, stretch.first, stretch.last, &mut reply.warnings)?;
    }
    for (tag, parts) in &held {
        reply.series.push(ForceSeries {
            country: tag.clone(),
            identities: if params.top.is_none() { forces::identities(parts) } else { Vec::new() },
            runs: runs_by_tag.remove(tag).unwrap_or_default(),
            segments: forces::segments(query, tag, first, last),
            membership: params.top.map(|_| vec![ArmySpan { first_hour: first, last_hour: last }]),
        });
    }
    let mut seen_warnings = BTreeSet::new();
    reply.warnings.retain(|warning| seen_warnings.insert(warning.clone()));
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

fn warn(recording: &Recording, run: u32, kinds: &[u8], keys: &[u128], first: i32, last: i32, warnings: &mut Vec<String>) -> io::Result<()> {
    let runs: Vec<u128> = kinds.iter().map(|&kind| Row::run_key(kind)).collect();
    let lost = read::runs(recording, run, TABLE, summary::Source::Cell(LOST_FROM), Some(&runs), first, last)?;
    for (key, held) in &lost {
        if let Some(from) = held.iter().filter_map(|run| run.2).min() {
            let what = if *key as u8 == Row::SHIPS { "Ships" } else { "Convoys" };
            warnings.push(format!(
                "{what} sunk are lower bounds from hour {from}: hours went unread long enough for the game to forget sinkings, or its retention is not known."
            ));
        }
    }
    if !keys.is_empty() {
        let unknown = read::runs(recording, run, TABLE, summary::Source::Cell(UNKNOWN), Some(keys), first, last)?;
        let refused = unknown.values().filter_map(|held| held.last().and_then(|run| run.2)).filter(|&count| count > 0).sum::<i64>();
        if refused > 0 {
            warnings.push(format!("{refused} sinkings of the countries shown could not be counted: the game listed them with an hour or a flag out of range."));
        }
    }
    Ok(())
}
