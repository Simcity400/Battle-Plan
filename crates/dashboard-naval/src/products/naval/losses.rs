
pub mod tiles;

use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, Product};
use dashboard_land::products::forces::{ForceAtRow, ForcePart, ForceRun};
use dashboard_world::products::group_members::GroupsAt;
use dashboard_base::Source;
use dashboard_declare::Key;
use dashboard_query::{
    Query,
    identity::Holder,
    read,
    series::{Part, ranked::GroupSelection},
};
use game_model::calendar::{HOUR_EPOCH, MONTH_LENGTHS};
use replay_recording::columnar_frames::Recording;
use replay_columnar::summary;
use replay_data::rows::{NavalLossTotalRow as Row, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
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
pub(super) const MISSING_BASELINE: &str = "Naval losses are not known at this hour: their tables were not read whole after the counting began.";

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

fn parts(kind: NavalKind) -> Vec<ForcePart> {
    kind.kinds()
        .iter()
        .map(|&kind| {
            let (key, name) = if kind == Row::SHIPS { ("ships", "Ships") } else { ("convoys", "Convoys") };
            ForcePart { key: key.into(), name: Some(name.into()), unresolved: false }
        })
        .collect()
}

pub(super) fn month_bases(counts: &Counts, kinds: &[u8], hour: i32, period: NavalPeriod) -> Vec<Option<i32>> {
    match period {
        NavalPeriod::Running => vec![None; kinds.len()],
        NavalPeriod::Month => kinds.iter().map(|&kind| Some((month_start(hour) - 1).max(counts.baseline(kind, hour).unwrap_or(hour)))).collect(),
    }
}

#[cfg(test)]
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

pub(super) fn part_runs(
    part: &Part,
    lo: i32,
    hi: i32,
    counts: &Counts,
    attribution: NavalAttribution,
    kind: NavalKind,
    period: NavalPeriod,
    out: &mut Vec<ForceRun>,
) {
    let mut edges = vec![lo];
    counts.edges(&mut edges);
    if period == NavalPeriod::Month {
        edges.extend(month_starts(lo, hi));
    }
    edges.retain(|&edge| lo <= edge && edge <= hi);
    edges.sort_unstable();
    edges.dedup();
    let attribution = attribution.code();
    for (index, &start) in edges.iter().enumerate() {
        let end = edges.get(index + 1).map_or(hi, |next| next - 1);
        let owner = Key::Holder.of(part.slot, part.id);
        let values: Vec<Option<i64>> = kind.kinds().iter().map(|&kind| counts.count(kind, Row::holder_key(owner, attribution, kind), start, period)).collect();
        let total = values.iter().try_fold(0i64, |sum, value| sum.checked_add((*value)?));
        let starts = part.starts && start == part.first;
        match out.last_mut() {
            Some(previous) if !starts && previous.last_hour + 1 == start && previous.total == total && previous.values == values => previous.last_hour = end,
            _ => out.push(ForceRun { first_hour: start, last_hour: end, starts_segment: starts, total, values, reference_total: None }),
        }
    }
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

pub struct NavalLossesAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalLossesAtParams {
    pub branch_id: String,
    pub hour: i32,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The group whose members at the hour are the rows; never with countries or a top."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    pub attribution: NavalAttribution,
    pub kind: NavalKind,
    pub period: NavalPeriod,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalLossesAtReply {
    pub first_hour: i32,
    pub last_hour: i32,
    pub parts: Vec<ForcePart>,
    pub rows: Vec<ForceAtRow>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalLossesAtReply);

fn next_month_start(hour: i32) -> i32 {
    let start = month_start(hour);
    start.saturating_add(MONTH_LENGTHS[month_index(start)] * 24)
}

fn validate_at(params: &NavalLossesAtParams) -> Result<(), &'static str> {
    if params.branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    if params.top.is_some_and(|top| top == 0 || !params.countries.is_empty()) {
        return Err("Invalid country selection");
    }
    if params.group.is_some() && (params.top.is_some() || !params.countries.is_empty()) {
        return Err("A selection names a group, a top or countries, one of them");
    }
    params.countries.iter().try_for_each(dashboard_base::products::validate_country)
}

#[expect(clippy::too_many_arguments, reason = "the hour, its stretch and the selection the bounds prove")]
fn proven_bounds(query: &Query<'_>, run: u32, hour: i32, stretch_first: i32, stretch_last: i32, period: NavalPeriod, attribution: u8, kinds: &[u8], chosen: Option<&[u128]>) -> io::Result<(i32, i32)> {
    let recording = query.replay.recording();
    let mut lo = stretch_first;
    let mut hi = stretch_last;
    if period == NavalPeriod::Month {
        lo = lo.max(month_start(hour));
        hi = hi.min(next_month_start(hour).saturating_sub(1));
    }
    let clamp = |runs: &Runs, lo: &mut i32, hi: &mut i32| {
        let index = runs.partition_point(|run| run.1 < hour);
        if let Some(&(first, last, _)) = runs.get(index) {
            if first <= hour {
                *lo = (*lo).max(first);
                *hi = (*hi).min(last);
            } else {
                *hi = (*hi).min(first.saturating_sub(1));
                if index > 0 {
                    *lo = (*lo).max(runs[index - 1].1.saturating_add(1));
                }
            }
        } else if let Some(&(_, last, _)) = runs.last() {
            *lo = (*lo).max(last.saturating_add(1));
        }
    };
    let run_keys: Vec<u128> = kinds.iter().map(|&kind| Row::run_key(kind)).collect();
    for column in [KNOWN, COUNTED_FROM, LOST_FROM] {
        for runs in read::runs(recording, run, TABLE, summary::Source::Cell(column), Some(&run_keys), lo, hi)?.values() {
            clamp(runs, &mut lo, &mut hi);
        }
    }
    let owners: BTreeSet<u128> = match chosen {
        Some(chosen) => chosen.iter().copied().collect(),
        None => query
            .holders
            .tags
            .values()
            .flatten()
            .filter_map(|held| match held.holder {
                Holder::Found { slot, id } => Some(Key::Holder.of(slot, id)),
                Holder::Unresolved => None,
            })
            .collect(),
    };
    let mut keys: Vec<u128> = owners.iter().flat_map(|&owner| kinds.iter().map(move |&kind| Row::holder_key(owner, attribution, kind))).collect();
    keys.sort_unstable();
    keys.dedup();
    for column in [TOTAL, UNKNOWN] {
        for runs in read::runs(recording, run, TABLE, summary::Source::Cell(column), Some(&keys), lo, hi)?.values() {
            clamp(runs, &mut lo, &mut hi);
        }
    }
    for runs in query.holders.tags.values() {
        let index = runs.partition_point(|run| run.last < hour);
        if let Some(held) = runs.get(index) {
            if held.first <= hour {
                lo = lo.max(held.first);
                hi = hi.min(held.last);
            } else {
                hi = hi.min(held.first.saturating_sub(1));
                if index > 0 {
                    lo = lo.max(runs[index - 1].last.saturating_add(1));
                }
            }
        } else if let Some(last) = runs.last() {
            lo = lo.max(last.last.saturating_add(1));
        }
    }
    for &(first, last, _) in &query.holders.unknown {
        if first <= hour && hour <= last {
            lo = lo.max(first);
            hi = hi.min(last);
        } else if last < hour {
            lo = lo.max(last.saturating_add(1));
        } else if first > hour {
            hi = hi.min(first.saturating_sub(1));
        }
    }
    Ok((lo.min(hour), hi.max(hour)))
}

pub fn reply_at(query: &Query<'_>, params: &NavalLossesAtParams, groups: &GroupsAt, cancelled: &AtomicBool) -> io::Result<NavalLossesAtReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let hour = params.hour;
    let reply_parts = parts(params.kind);
    let Some(stretch) = query.timeline.at(hour) else {
        return Ok(NavalLossesAtReply {
            first_hour: hour,
            last_hour: hour,
            parts: reply_parts,
            rows: Vec::new(),
            missing: Some("This hour is not recorded".into()),
            warnings: Vec::new(),
        });
    };
    let recording = query.replay.recording();
    let kinds = params.kind.kinds();
    let attribution = params.attribution.code();
    let from = match params.period {
        NavalPeriod::Running => hour,
        NavalPeriod::Month => (month_start(hour) - 1).clamp(stretch.first, hour),
    };
    let counts = Counts::read(recording, stretch.run, kinds, &[], from, hour)?;
    let missing_baseline = kinds.iter().any(|&kind| counts.baseline(kind, hour).is_none());
    let bounds = |chosen: Option<&[u128]>, window: Option<(i32, i32)>| -> io::Result<(i32, i32)> {
        let (first, last) = proven_bounds(query, stretch.run, hour, stretch.first, stretch.last, params.period, attribution, kinds, chosen)?;
        Ok(window.map_or((first, last), |(from, until)| (first.max(from).min(hour), last.min(until).max(hour))))
    };
    let mut window = None;
    let countries: Vec<Country> = match &params.group {
        None => params.countries.clone(),
        Some(group) => {
            let members = groups.members(group, hour)?;
            window = Some((members.from, members.until));
            if let Some(reason) = members.refused {
                let (first_hour, last_hour) = bounds(None, window)?;
                return Ok(NavalLossesAtReply { first_hour, last_hour, parts: reply_parts, rows: Vec::new(), missing: Some(reason), warnings: Vec::new() });
            }
            members.tags.into_iter().map(Country::from).collect()
        }
    };
    let ranked = params.top.is_some() || (countries.is_empty() && params.group.is_none());
    let chosen: Option<Vec<u128>> = (!ranked).then(|| {
        countries
            .iter()
            .filter_map(|country| match query.holders.at(country, hour)?.holder {
                Holder::Found { slot, id } => Some(Key::Holder.of(slot, id)),
                Holder::Unresolved => None,
            })
            .collect()
    });
    let (first_hour, last_hour) = bounds(chosen.as_deref(), window)?;
    let mut warnings = Vec::new();
    if missing_baseline && ranked {
        return Ok(NavalLossesAtReply { first_hour, last_hour, parts: reply_parts, rows: Vec::new(), missing: Some(MISSING_BASELINE.into()), warnings });
    }
    let at_hour = |at: i32| -> io::Result<HashMap<u128, i64>> {
        let snapshots = read::at(recording, stretch.run, &[read::Read { table: TABLE, columns: Some(&[TOTAL][..]), keys: read::Keys::All }], at)?;
        let snapshot = snapshots.first().ok_or_else(|| invalid("Naval loss totals were not read"))?;
        let view = snapshot.view();
        Ok(snapshot.rows.iter().filter_map(|&(key, slot)| Some((key, view.number(TOTAL, slot)?))).collect())
    };
    let now = at_hour(hour)?;
    let bases = month_bases(&counts, kinds, hour, params.period);
    let mut before: HashMap<i32, HashMap<u128, i64>> = HashMap::new();
    for &base in bases.iter().flatten() {
        if base < hour && base >= stretch.first && !before.contains_key(&base) {
            before.insert(base, at_hour(base)?);
        }
    }
    let part_values = |owner: u128| -> (Option<i64>, Vec<Option<i64>>) {
        let values: Vec<Option<i64>> = kinds
            .iter()
            .zip(&bases)
            .map(|(&kind, base)| {
                if base.is_some_and(|base| base >= hour) {
                    return Some(0);
                }
                let key = Row::holder_key(owner, attribution, kind);
                let prev = base.and_then(|base| before.get(&base)).map_or(0, |before| before.get(&key).copied().unwrap_or(0));
                now.get(&key).copied().unwrap_or(0).checked_sub(prev)
            })
            .collect();
        let total = values.iter().try_fold(0i64, |sum, value| sum.checked_add((*value)?));
        (total, values)
    };
    let mut used_keys = Vec::new();
    let mut rows: Vec<ForceAtRow> = Vec::new();
    if countries.is_empty() && params.group.is_none() {
        let unresolved = query.holders.unknown.iter().any(|&(first, last, _)| first <= hour && hour <= last)
            || query.holders.tags.values().flatten().any(|run| run.first <= hour && hour <= run.last && run.holder == Holder::Unresolved);
        for (tag, runs) in &query.holders.tags {
            if let Some(Holder::Found { slot, id }) = runs.iter().find(|held| held.first <= hour && hour <= held.last).map(|held| held.holder) {
                let owner = Key::Holder.of(slot, id);
                let (total, values) = part_values(owner);
                if params.top.is_some() && total.is_none_or(|value| value <= 0) {
                    continue;
                }
                for &kind in kinds {
                    used_keys.push(Row::holder_key(owner, attribution, kind));
                }
                rows.push(ForceAtRow {
                    country: Country::from(tag.as_str()),
                    slot: Some(slot),
                    tag: tag.clone(),
                    total,
                    reference_total: None,
                    missing: None,
                    values,
                    parts_missing: None,
                });
            }
        }
        rows.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.tag.cmp(&b.tag)));
        if let Some(limit) = params.top {
            if unresolved {
                return Ok(NavalLossesAtReply {
                    first_hour,
                    last_hour,
                    parts: reply_parts,
                    rows: Vec::new(),
                    missing: Some(format!("The top {limit} naval losses are not known: country holders could not be resolved at this hour.")),
                    warnings,
                });
            }
            rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
    } else {
        let mut seen = BTreeSet::new();
        let mut held_slots = BTreeSet::new();
        for country in &countries {
            if !seen.insert(country.clone()) {
                continue;
            }
            let Some(run) = query.holders.at(country, hour) else {
                if query.holders.unknown.iter().any(|&(first, last, _)| first <= hour && hour <= last) {
                    rows.push(ForceAtRow {
                        country: country.clone(),
                        slot: None,
                        tag: query.holders.name(country, hour),
                        total: None,
                        reference_total: None,
                        missing: Some(query.holders.unnamed(country, hour)),
                        values: vec![None; kinds.len()],
                        parts_missing: None,
                    });
                }
                continue;
            };
            if let Holder::Found { slot, .. } = run.holder
                && !held_slots.insert(slot)
            {
                continue;
            }
            let tag = query.holders.name(country, hour);
            match run.holder {
                Holder::Found { slot, .. } if missing_baseline => rows.push(ForceAtRow {
                    country: country.clone(),
                    slot: Some(slot),
                    tag,
                    total: None,
                    reference_total: None,
                    missing: Some(MISSING_BASELINE.into()),
                    values: vec![None; kinds.len()],
                    parts_missing: None,
                }),
                Holder::Found { slot, id } => {
                    let owner = Key::Holder.of(slot, id);
                    let (total, values) = part_values(owner);
                    for &kind in kinds {
                        used_keys.push(Row::holder_key(owner, attribution, kind));
                    }
                    rows.push(ForceAtRow {
                        country: country.clone(),
                        slot: Some(slot),
                        tag,
                        total,
                        reference_total: None,
                        missing: total.is_none().then(|| "Naval losses could not be computed at this hour".into()),
                        values,
                        parts_missing: None,
                    });
                }
                Holder::Unresolved => rows.push(ForceAtRow {
                    country: country.clone(),
                    slot: None,
                    tag,
                    total: None,
                    reference_total: None,
                    missing: Some(query.holders.not_known(country, hour).unwrap_or_else(|| "Country holder could not be resolved at this hour".into())),
                    values: vec![None; kinds.len()],
                    parts_missing: None,
                }),
            }
        }
        rows.sort_unstable_by(|a, b| match (a.total, b.total) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.tag.cmp(&b.tag),
        });
    }
    used_keys.sort_unstable();
    used_keys.dedup();
    warn(recording, stretch.run, kinds, &used_keys, hour, hour, &mut warnings)?;
    let mut seen_warnings = BTreeSet::new();
    warnings.retain(|warning| seen_warnings.insert(warning.clone()));
    Ok(NavalLossesAtReply { first_hour, last_hour, parts: reply_parts, rows, missing: None, warnings })
}

impl Product for NavalLossesAt {
    const KEY: &'static str = "naval-losses-at";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = NavalLossesAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, countries, top, group, attribution, kind, period, "countries[]"];
    type Reply = NavalLossesAtReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            NavalLossesAtParams::decl(),
            NavalLossesAtReply::decl(),
            ForceAtRow::decl(),
            ForcePart::decl(),
            NavalAttribution::decl(),
            NavalKind::decl(),
            NavalPeriod::decl(),
            tiles::NavalLossesTileFamily::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_at(params)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let groups = GroupsAt::load(source, context, &params.branch_id, params.group.as_ref())?;
        reply_at(&query, params, &groups, cancelled)
    }
}

#[cfg(test)]
mod at_tests {
    use super::*;

    #[test]
    fn next_month_start_advances_exactly_one_calendar_month() {
        let jan_15 = HOUR_EPOCH + 14 * 24 + 5;
        assert_eq!(month_start(jan_15), HOUR_EPOCH);
        assert_eq!(next_month_start(jan_15), HOUR_EPOCH + 31 * 24);
    }
}
