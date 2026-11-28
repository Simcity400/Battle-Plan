
pub mod tiles;

use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, Product};
use dashboard_land::products::forces::{ForceAtRow, ForcePart, ForceRun};
use dashboard_world::products::group_members::GroupsAt;
use dashboard_base::Source;
use dashboard_declare::Key;
use dashboard_query::{
    Query,
    identity::{Holder, Holders, TAG_UNLINKED},
    read,
    series::{MissingSpan, Part, ranked::GroupSelection},
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
    lost_from: HashMap<u128, Runs>,
    refused: HashMap<u128, Runs>,
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
        Ok(Self {
            known: column(KNOWN, &runs)?,
            counted_from: column(COUNTED_FROM, &runs)?,
            totals: column(TOTAL, keys)?,
            lost_from: column(LOST_FROM, &runs)?,
            refused: column(UNKNOWN, keys)?,
        })
    }

    #[cfg(test)]
    pub fn of(known: HashMap<u128, Runs>, counted_from: HashMap<u128, Runs>, totals: HashMap<u128, Runs>) -> Self {
        Self { known, counted_from, totals, lost_from: HashMap::new(), refused: HashMap::new() }
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
        match self.opening(kind, hour, period) {
            None => Some(now),
            Some(base) => {
                if base >= hour {
                    return Some(0);
                }
                now.checked_sub(self.running(kind, key, base)?)
            }
        }
    }

    fn opening(&self, kind: u8, hour: i32, period: NavalPeriod) -> Option<i32> {
        match period {
            NavalPeriod::Running => None,
            NavalPeriod::Month => Some((month_start(hour) - 1).max(self.baseline(kind, hour)?)),
        }
    }

    fn retention(&self, kind: u8, hour: i32, period: NavalPeriod) -> Option<String> {
        let run = Row::run_key(kind);
        let from = at(self.lost_from.get(&run), hour)?;
        if let Some(base) = self.opening(kind, hour, period) && from <= i64::from(base) {
            let known = self.known.get(&run)?;
            let index = known.partition_point(|&(_, last, _)| last < base);
            let mut next = i64::from(base);
            for &(first, last, value) in &known[index..] {
                if i64::from(first) > next || value != Some(1) { break; }
                if last >= hour { return None; }
                next = i64::from(last) + 1;
            }
        }
        let what = if kind == Row::SHIPS { "ship sinkings" } else { "convoy losses" };
        let date = i32::try_from(from).map(dashboard_text::division_history::date).unwrap_or_else(|_| "an unknown date".into());
        Some(format!("Some {what} may be missing from {date}: hours went unread long enough for the game to forget sinkings, or its retention is not known."))
    }

    fn refused_count(&self, kind: u8, key: u128, hour: i32, period: NavalPeriod) -> i64 {
        let runs = self.refused.get(&key);
        let before = self.opening(kind, hour, period).and_then(|base| at(runs, base)).unwrap_or(0);
        at(runs, hour).unwrap_or(0).saturating_sub(before).max(0)
    }

    fn partial(&self, kind: u8, key: u128, hour: i32, period: NavalPeriod) -> Option<String> {
        let retention = self.retention(kind, hour, period);
        let count = self.refused_count(kind, key, hour, period);
        if count == 0 {
            return retention;
        }
        let what = match (kind == Row::SHIPS, count == 1) {
            (true, true) => "ship sinking", (true, false) => "ship sinkings",
            (false, true) => "convoy loss", (false, false) => "convoy losses",
        };
        let refused = format!("{count} {what} could not be counted: the game listed an hour or a flag out of range.");
        Some(match retention { Some(reason) => format!("{reason} {refused}"), None => refused })
    }

    fn edges(&self, out: &mut Vec<i32>, kind: u8, key: u128, lo: i32, hi: i32) {
        let run = Row::run_key(kind);
        for runs in [self.known.get(&run), self.counted_from.get(&run), self.lost_from.get(&run), self.totals.get(&key), self.refused.get(&key)].into_iter().flatten() {
            let index = runs.partition_point(|&(_, last, _)| last < lo);
            for &(first, last, _) in runs[index..].iter().take_while(|&&(first, _, _)| first <= hi) {
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
    kinds.iter().map(|&kind| counts.opening(kind, hour, period)).collect()
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

fn combined(reasons: impl Iterator<Item = String>) -> Option<String> {
    reasons.reduce(|mut first, next| {
        first.push(' ');
        first.push_str(&next);
        first
    })
}

#[derive(Clone, Default)]
pub(super) struct LossHistory {
    pub runs: Vec<ForceRun>,
    pub partial: Vec<MissingSpan>,
}

pub(super) fn part_runs(
    part: &Part,
    lo: i32,
    hi: i32,
    counts: &Counts,
    attribution: NavalAttribution,
    kind: NavalKind,
    period: NavalPeriod,
    out: &mut LossHistory,
) {
    let mut edges = vec![lo];
    let owner = Key::Holder.of(part.slot, part.id);
    let attribution = attribution.code();
    for &kind in kind.kinds() {
        counts.edges(&mut edges, kind, Row::holder_key(owner, attribution, kind), lo, hi);
    }
    if period == NavalPeriod::Month {
        edges.extend(month_starts(lo, hi));
    }
    edges.retain(|&edge| lo <= edge && edge <= hi);
    edges.sort_unstable();
    edges.dedup();
    for (index, &start) in edges.iter().enumerate() {
        let end = edges.get(index + 1).map_or(hi, |next| next - 1);
        let values: Vec<Option<i64>> = kind.kinds().iter().map(|&kind| counts.count(kind, Row::holder_key(owner, attribution, kind), start, period)).collect();
        let total = values.iter().try_fold(0i64, |sum, value| sum.checked_add((*value)?));
        let starts = part.starts && start == part.first;
        if total.is_some() && let Some(reason) = combined(kind.kinds().iter().filter_map(|&kind| counts.partial(kind, Row::holder_key(owner, attribution, kind), start, period))) {
            dashboard_base::tile_runs::push_span(&mut out.partial, start, end, &reason);
        }
        match out.runs.last_mut() {
            Some(previous) if !starts && previous.last_hour + 1 == start && previous.total == total && previous.values == values => previous.last_hour = end,
            _ => out.runs.push(ForceRun { first_hour: start, last_hour: end, starts_segment: starts, total, values, reference_total: None }),
        }
    }
}

fn unlinked_tag_at(holders: &Holders, hour: i32) -> bool {
    holders.tags.values().any(|runs| {
        let at = runs.partition_point(|run| run.last < hour);
        runs.get(at).is_some_and(|run| run.first <= hour && run.holder == Holder::Unresolved)
    })
}

fn missing_holder<'a>(holders: &'a Holders, country: &Country, hour: i32, holder: Option<Holder>, unlinked: impl FnOnce() -> bool) -> Option<&'a str> {
    match holder {
        Some(Holder::Unresolved) => {
            let at = holders.unknown.partition_point(|&(_, last, _)| last < hour);
            Some(holders.unknown.get(at).filter(|(first, ..)| *first <= hour).map_or(TAG_UNLINKED, |(_, _, reason)| reason))
        }
        None if matches!(country, Country::Slot(_)) && unlinked() => Some(TAG_UNLINKED),
        _ => None,
    }
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
fn proven_bounds(query: &Query<'_>, run: u32, hour: i32, stretch_first: i32, stretch_last: i32, period: NavalPeriod, attribution: u8, kinds: &[u8], countries: Option<&[Country]>) -> io::Result<(i32, i32)> {
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
    let statuses: Runs = read::statuses(recording, run, TABLE, lo, hi)?.into_iter().map(|status| (status.first, status.last, Some(i64::from(replay_recording::columnar_frames::complete(status.status))))).collect();
    clamp(&statuses, &mut lo, &mut hi);
    let found = |holder: Holder| match holder {
        Holder::Found { slot, id } => Some(Key::Holder.of(slot, id)),
        Holder::Unresolved => None,
    };
    let owners: BTreeSet<u128> = match countries {
        Some(countries) => countries.iter().filter_map(|country| found(query.holders.at(country, hour)?.holder)).collect(),
        None => query.holders.tags.values().flatten().filter_map(|held| found(held.holder)).collect(),
    };
    let mut keys: Vec<u128> = owners.iter().flat_map(|&owner| kinds.iter().map(move |&kind| Row::holder_key(owner, attribution, kind))).collect();
    keys.sort_unstable();
    keys.dedup();
    for column in [TOTAL, UNKNOWN] {
        for runs in read::runs(recording, run, TABLE, summary::Source::Cell(column), Some(&keys), lo, hi)?.values() {
            clamp(runs, &mut lo, &mut hi);
        }
    }
    if let Some(countries) = countries {
        let mut slots = BTreeSet::new();
        for country in countries {
            let held = query.holders.at(country, hour);
            let (first, last) = held.map_or_else(|| query.holders.interval(hour), |run| (run.first, run.last));
            lo = lo.max(first);
            hi = hi.min(last);
            if let Some(Holder::Found { slot, .. }) = held.map(|run| run.holder) {
                slots.insert(slot);
            }
        }
        for run in query.holders.tags.values().flatten() {
            let Holder::Found { slot, .. } = run.holder else { continue };
            if !slots.contains(&slot) {
                continue;
            }
            if run.first <= hour && hour <= run.last {
                lo = lo.max(run.first);
                hi = hi.min(run.last);
            } else if run.last < hour {
                lo = lo.max(run.last.saturating_add(1));
            } else {
                hi = hi.min(run.first.saturating_sub(1));
            }
        }
    } else {
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
    let mut counts = Counts::read(recording, stretch.run, kinds, &[], from, hour)?;
    let missing_baseline = kinds.iter().all(|&kind| counts.baseline(kind, hour).is_none());
    let kind_missing: Vec<Option<String>> = kinds
        .iter()
        .map(|&kind| {
            counts.baseline(kind, hour).is_none().then(|| match (missing_baseline, kind == Row::SHIPS) {
                (true, _) => MISSING_BASELINE.to_owned(),
                (false, true) => "Ships sunk are not known at this hour: their losses were not read whole after the counting began".to_owned(),
                (false, false) => "Convoys sunk are not known at this hour: their losses were not read whole after the counting began".to_owned(),
            })
        })
        .collect();
    let bounds = |countries: Option<&[Country]>, window: Option<(i32, i32)>| -> io::Result<(i32, i32)> {
        let (first, last) = proven_bounds(query, stretch.run, hour, stretch.first, stretch.last, params.period, attribution, kinds, countries)?;
        Ok(window.map_or((first, last), |(from, until)| (first.max(from).min(hour), last.min(until).max(hour))))
    };
    let mut window = None;
    let countries: Vec<Country> = match &params.group {
        None => params.countries.clone(),
        Some(group) => {
            let members = groups.members(group, hour)?;
            window = Some((members.from, members.until));
            if let Some(reason) = members.refused {
                let (first_hour, last_hour) = bounds(Some(&[]), window)?;
                return Ok(NavalLossesAtReply { first_hour, last_hour, parts: reply_parts, rows: Vec::new(), missing: Some(reason), warnings: Vec::new() });
            }
            members.tags.into_iter().map(Country::from).collect()
        }
    };
    let ranked = params.top.is_some() || (countries.is_empty() && params.group.is_none());
    let (first_hour, last_hour) = bounds((!ranked).then_some(countries.as_slice()), window)?;
    let mut warnings = Vec::new();
    if missing_baseline && ranked {
        return Ok(NavalLossesAtReply { first_hour, last_hour, parts: reply_parts, rows: Vec::new(), missing: Some(MISSING_BASELINE.into()), warnings });
    }
    let loss_keys = |country: &Country| {
        let owner = query.holders.at(country, hour).and_then(|run| match run.holder {
            Holder::Found { slot, id } => Some(Key::Holder.of(slot, id)),
            Holder::Unresolved => None,
        });
        owner.into_iter().flat_map(move |owner| kinds.iter().map(move |&kind| Row::holder_key(owner, attribution, kind)))
    };
    let chosen: Option<Vec<u128>> = (!ranked).then(|| {
        let mut keys: Vec<u128> = countries.iter().flat_map(loss_keys).collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    });
    let at_hour = |at: i32| -> io::Result<Option<HashMap<u128, Option<i64>>>> {
        let keys = chosen.as_deref().map_or(read::Keys::All, read::Keys::These);
        let snapshots = read::at(recording, stretch.run, &[read::Read { table: TABLE, columns: Some(&[TOTAL][..]), keys }], at)?;
        let snapshot = snapshots.first().ok_or_else(|| invalid("Naval loss totals were not read"))?;
        if !replay_recording::columnar_frames::complete(snapshot.status) {
            return Ok(None);
        }
        let view = snapshot.view();
        Ok(Some(snapshot.rows.iter().map(|&(key, slot)| (key, view.number(TOTAL, slot))).collect()))
    };
    let held = |snapshot: &HashMap<u128, Option<i64>>, key: u128| snapshot.get(&key).copied().unwrap_or(Some(0));
    let now = at_hour(hour)?;
    let bases = month_bases(&counts, kinds, hour, params.period);
    let mut before: HashMap<i32, Option<HashMap<u128, Option<i64>>>> = HashMap::new();
    for &base in bases.iter().flatten() {
        if base < hour && base >= stretch.first && !before.contains_key(&base) {
            before.insert(base, at_hour(base)?);
        }
    }
    let part_values = |owner: u128| -> (Option<i64>, Vec<Option<i64>>, Option<String>) {
        let mut why = None;
        let values: Vec<Option<i64>> = kinds
            .iter()
            .zip(&bases)
            .zip(&kind_missing)
            .map(|((&kind, base), missing)| {
                let key = Row::holder_key(owner, attribution, kind);
                if let Some(reason) = missing {
                    why.get_or_insert_with(|| reason.clone());
                    return None;
                }
                if base.is_some_and(|base| base >= hour) {
                    return Some(0);
                }
                let prev = match base {
                    None => Some(0),
                    Some(base) => match before.get(&base) {
                        Some(Some(before)) if counts.baseline(kind, *base).is_some() => held(before, key),
                        _ => None,
                    },
                };
                let Some(prev) = prev else {
                    why.get_or_insert_with(|| "The month's opening count was not recorded".to_owned());
                    return None;
                };
                let Some(closing) = now.as_ref().and_then(|now| held(now, key)) else {
                    why.get_or_insert_with(|| "Naval loss totals were not read whole at this hour".to_owned());
                    return None;
                };
                closing.checked_sub(prev)
            })
            .collect();
        let total = values.iter().try_fold(0i64, |sum, value| sum.checked_add((*value)?));
        (total, values, why)
    };
    let unlinked = std::cell::LazyCell::new(|| unlinked_tag_at(&query.holders, hour));
    let mut rows: Vec<ForceAtRow> = Vec::new();
    if countries.is_empty() && params.group.is_none() {
        let unresolved = query.holders.unknown.iter().any(|&(first, last, _)| first <= hour && hour <= last)
            || *unlinked;
        for (tag, runs) in &query.holders.tags {
            if let Some(Holder::Found { slot, id }) = runs.iter().find(|held| held.first <= hour && hour <= held.last).map(|held| held.holder) {
                let owner = Key::Holder.of(slot, id);
                let (total, values, why) = part_values(owner);
                if params.top.is_some() && total.is_some_and(|value| value <= 0) {
                    continue;
                }
                rows.push(ForceAtRow {
                    country: Country::from(tag.as_str()),
                    slot: Some(slot),
                    tag: tag.clone(),
                    total,
                    reference_total: None,
                    missing: total.is_none().then(|| why.unwrap_or_else(|| "Naval losses could not be computed at this hour".into())),
                    values,
                    partial: None,
                    parts_partial: None,
                    parts_missing: None,
                });
            }
        }
        rows.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.tag.cmp(&b.tag)));
        if unresolved && params.top.is_none() {
            warnings.push("Some countries could not be resolved at this hour: their naval losses are not listed.".into());
        }
        if let Some(limit) = params.top {
            if let Some(reason) = rows.iter().find_map(|row| row.missing.clone()) {
                return Ok(NavalLossesAtReply {
                    first_hour,
                    last_hour,
                    parts: reply_parts,
                    rows: Vec::new(),
                    missing: Some(format!("The top {limit} naval losses are not known: {reason}.")),
                    warnings,
                });
            }
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
            let holder = query.holders.at(country, hour).map(|run| run.holder);
            match holder {
                Some(Holder::Found { slot, id }) => {
                    if !held_slots.insert(slot) {
                        continue;
                    }
                    let owner = Key::Holder.of(slot, id);
                    let (total, values, why) = part_values(owner);
                    rows.push(ForceAtRow {
                        country: country.clone(),
                        slot: Some(slot),
                        tag: query.holders.name(country, hour),
                        total,
                        reference_total: None,
                        missing: total.is_none().then(|| why.unwrap_or_else(|| "Naval losses could not be computed at this hour".into())),
                        values,
                        partial: None,
                        parts_partial: None,
                        parts_missing: None,
                    });
                }
                _ => {
                    if let Some(reason) = missing_holder(&query.holders, country, hour, holder, || *unlinked) {
                        rows.push(ForceAtRow {
                            country: country.clone(),
                            slot: None,
                            tag: country.to_string(),
                            total: None,
                            reference_total: None,
                            missing: Some(format!("Who holds {country} is not known: {reason}")),
                            values: vec![None; kinds.len()],
                            partial: None,
                            parts_partial: None,
                            parts_missing: None,
                        });
                    }
                }
            }
        }
        rows.sort_unstable_by(|a, b| match (a.total, b.total) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.tag.cmp(&b.tag),
        });
    }
    let mut used_keys: Vec<_> = rows.iter().filter(|row| row.slot.is_some()).flat_map(|row| loss_keys(&row.country)).collect();
    used_keys.sort_unstable();
    used_keys.dedup();
    warnings.extend(kinds.iter().filter_map(|&kind| counts.retention(kind, hour, params.period)));
    if !used_keys.is_empty() {
        counts.refused = read::runs(recording, stretch.run, TABLE, summary::Source::Cell(UNKNOWN), Some(&used_keys), from, hour)?;
        let refused: i64 = used_keys.iter().map(|&key| counts.refused_count(key as u8, key, hour, params.period)).sum();
        if refused > 0 {
            warnings.push(format!("{refused} sinkings of the countries shown could not be counted: the game listed them with an hour or a flag out of range."));
        }
    }
    for row in &mut rows {
        let mut reasons = kinds.iter().zip(loss_keys(&row.country)).zip(&row.values)
            .map(|((&kind, key), value)| value.and_then(|_| counts.partial(kind, key, hour, params.period)));
        if kinds.len() == 1 {
            row.partial = reasons.next().flatten();
        } else {
            let parts: Vec<_> = reasons.collect();
            if parts.iter().any(Option::is_some) {
                if row.total.is_some() {
                    row.partial = combined(parts.iter().flatten().cloned());
                }
                row.parts_partial = Some(parts);
            }
        }
    }
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
    fn a_month_with_complete_reads_starts_clear_of_last_months_refusals_and_retention_gap() {
        let kind = Row::SHIPS;
        let run = Row::run_key(kind);
        let key = Row::holder_key(3, Row::LOST, kind);
        let open = month_start(HOUR_EPOCH);
        let prior = open - 20;
        let last = open + 12;
        let counts = Counts {
            known: HashMap::from([(run, vec![(prior, open + 5, Some(1)), (open + 6, open + 6, Some(0)), (open + 7, last, Some(1))])]),
            counted_from: HashMap::from([(run, vec![(prior, last, Some(i64::from(prior)))])]),
            totals: HashMap::new(),
            lost_from: HashMap::from([(run, vec![(prior + 1, last, Some(i64::from(prior + 1)))])]),
            refused: HashMap::from([(key, vec![(prior + 2, open + 3, Some(2)), (open + 4, last, Some(3))])]),
        };
        assert!(counts.partial(kind, key, open, NavalPeriod::Running).unwrap().contains("2 ship sinkings"));
        assert_eq!(counts.partial(kind, key, open, NavalPeriod::Month), None);
        let refused = counts.partial(kind, key, open + 4, NavalPeriod::Month).unwrap();
        assert!(refused.starts_with("1 ship sinking"));
        let resumed = counts.partial(kind, key, last, NavalPeriod::Month).unwrap();
        assert!(resumed.contains(&dashboard_text::division_history::date(prior + 1)) && resumed.contains(&refused));
    }

    #[test]
    fn quiet_naval_losses_do_not_reopen_totals_during_playback() {
        use crate::registrations::NAVAL_LOSSES_NAVALLOSSESAT;
        use dashboard_base::{SourceKey, test_fixtures::{CAMPAIGN, played, records, source}};
        use replay::campaign::Replay;
        use replay_data::rows::{SunkShipEntry, SunkShipsRow, TableRows};
        use replay_frame::frame::TableView;
        use serde_json::json;
        use std::sync::Arc;

        let first = HOUR_EPOCH + 100;
        let frames = (first..=first + 20).map(|hour| {
            let mut frame = source(1, hour, Some(i64::from(hour)));
            let entries = [first + 5, first + 12].into_iter().filter(|&sink_hour| sink_hour <= hour).map(|sink_hour| {
                SunkShipEntry { sink_hour, victim_country: 9001, ship_name: sink_hour.to_string().into_bytes(), ..Default::default() }
            }).collect();
            *frame.tables[TableId::SunkShips.index()] = TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }]));
            frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
            frame
        }).collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Arc::new(Source::from_replay(SourceKey { source_id: "quiet-naval-losses".into(), revision: "1".into() }, "naval.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
        let load = LoadContext { content: None };
        let registration = NAVAL_LOSSES_NAVALLOSSESAT;
        for period in ["running", "month"] {
            let mut asked = json!({"branchId":"1","countries":["GER"],"attribution":"lost","kind":"ships","period":period});
            let mut follow = asked.clone();
            follow["panel"] = registration.key.into();
            let panel = (registration.panel.unwrap().open)(&source, &load, &follow).unwrap();
            for read in played(source.replay_ref(), panel.exact().unwrap(), first, first + 20) {
                asked["hour"] = read.hour.into();
                let expected = registration.answer(&source, asked.clone(), &load, &false.into()).unwrap();
                assert_eq!(read.value.as_ref().as_ref().unwrap(), &expected, "{period} at {}", read.hour);
                let offset = read.hour - first;
                if matches!(offset, 5 | 12) {
                    assert_eq!(read.product_calls, 1, "new sinking at {}", read.hour);
                } else if offset > 0 {
                    assert_eq!((read.product_calls, read.visited), (0, 0), "quiet hour {}", read.hour);
                }
            }
        }
    }

    #[test]
    fn next_month_start_advances_exactly_one_calendar_month() {
        let jan_15 = HOUR_EPOCH + 14 * 24 + 5;
        assert_eq!(month_start(jan_15), HOUR_EPOCH);
        assert_eq!(next_month_start(jan_15), HOUR_EPOCH + 31 * 24);
    }
}
