
use super::{Ask, Line, Part, invalid};
use crate::{
    identity::{Country, Holder},
    read,
};
use dashboard_declare::{Column, RunningTotal};
use replay_lanes::lanes::Runs;
use replay_recording::columnar_frames::{Damage, DamageScope};
use replay_columnar::{manifest::Status, summary::Source};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    NotRecorded,
    Cell { part: u128, reason: u8 },
    WindowIncomplete,
    Restarted,
    OutOfRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryGap {
    Missing(Gap),
    Damage(Arc<str>),
}

type HistoryValue = Result<i64, HistoryGap>;
type HistoryRuns = Vec<(i32, i32, HistoryValue)>;
type ColumnRuns = HashMap<usize, HashMap<u128, Runs>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotalRun {
    pub first: i32,
    pub last: i32,
    pub value: HistoryValue,
    pub opens: bool,
}

pub type Back<'f> = Option<&'f dyn Fn(i32) -> Option<i32>>;

pub type Parts<'f> = &'f dyn Fn(u32) -> Vec<u128>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    pub reads: Vec<i32>,
    pub since: Vec<i32>,
}

pub struct ClockAt {
    pub read: Option<i32>,
    pub since: Option<i32>,
    pub since_from: Option<i32>,
    pub next_change: Option<i32>,
}

fn block_damage(error: &io::Error) -> bool {
    matches!(Damage::of(error).map(|d| &d.scope), Some(DamageScope::Table { .. } | DamageScope::Page { .. }))
}

fn at(runs: Option<&Runs>, hour: i32) -> Option<i64> {
    let runs = runs?;
    let index = runs.partition_point(|run| run.1 < hour);
    runs.get(index).filter(|run| run.0 <= hour).and_then(|run| run.2)
}

fn clock_at(runs: &HashMap<u128, Runs>, key: u128, hour: i32) -> Option<i32> {
    at(runs.get(&key), hour).and_then(|value| i32::try_from(value).ok())
}

struct Columns {
    value: usize,
    reason: usize,
    since: usize,
    as_of: usize,
}

pub const TOTALS_HOUR_UNRECORDED: &str = "This hour is not recorded on this branch";
pub const TOTALS_NOT_READ: &str = "Production was not read whole by this hour";
pub const TOTALS_WINDOW_UNRECORDED: &str = "The count at the window's start was not recorded, so what was delivered in it is not known";
pub const TOTALS_VALUE_UNKNOWN: &str = "A country's amount is not known at this hour, so the ranking cannot be proved";

pub type RankedCells<'a> = &'a dyn Fn(u128) -> Option<Option<i64>>;

fn sum_parts<P: Copy, E>(
    keys: impl Iterator<Item = (P, u128)>, rest: Option<u128>, cell: impl Fn(P, u128) -> Option<Result<i64, E>>,
) -> Result<Option<i128>, E> {
    let mut total = 0_i128;
    for (part, key) in keys {
        let Some(value) = cell(part, key).or_else(|| rest.and_then(|key| cell(part, key))) else { return Ok(None) };
        total += i128::from(value?);
    }
    Ok(Some(total))
}

#[derive(Clone, Copy)]
struct WindowEndpoint {
    read: Option<i32>,
    since: Option<i32>,
    sum: Result<i128, Gap>,
}

struct TotalWindow {
    read: i32,
    start: Option<i32>,
    opening_read: Option<i32>,
    since: Option<i32>,
}

impl TotalWindow {
    fn value(&self, first: i32, sum: impl Fn(i32) -> Result<i128, Gap>) -> Result<i64, Gap> {
        let start = self.start.filter(|&at| self.read >= first && at >= first && self.opening_read == Some(at)).ok_or(Gap::WindowIncomplete)?;
        if self.since.is_some_and(|since| since > start) { return Err(Gap::Restarted); }
        i64::try_from(sum(self.read)? - sum(start)?).map_err(|_| Gap::OutOfRange)
    }
}

impl From<Gap> for HistoryGap {
    fn from(gap: Gap) -> Self { Self::Missing(gap) }
}

fn after_restart<E: From<Gap>>(value: Result<i64, E>, start: Option<i32>, since: Option<i32>) -> Result<i64, E> {
    value.and_then(|value| if start.zip(since).is_some_and(|(start, since)| since > start) { Err(Gap::Restarted.into()) } else { Ok(value) })
}

pub fn ranked_total(
    keys: impl Clone + Iterator<Item = u128>, rest: Option<u128>, now: RankedCells<'_>,
    before: Option<RankedCells<'_>>, keep: &dyn Fn(i64) -> bool,
) -> Result<Option<i64>, &'static str> {
    let sum = |cell: RankedCells<'_>| sum_parts(keys.clone().map(|key| ((), key)), rest, |(), key| cell(key).map(|value| value.ok_or(TOTALS_VALUE_UNKNOWN)));
    let Some(at) = sum(now)? else { return Ok(None) };
    let value = match before {
        None => at,
        Some(before) => match sum(before)? {
            Some(before) => at - before,
            None if i64::try_from(at).is_ok_and(|value| !keep(value)) => return Ok(None),
            None => return Err(TOTALS_WINDOW_UNRECORDED),
        },
    };
    let value = i64::try_from(value).map_err(|_| TOTALS_VALUE_UNKNOWN)?;
    Ok(keep(value).then_some(value))
}

impl<'q, 'a> Line<'q, 'a> {
    fn running(&self) -> io::Result<&'static RunningTotal> {
        dashboard_declare::running_total(self.series.kind).ok_or_else(|| invalid(format!("{} is no running total", self.series.kind)))
    }

    fn total_columns(&self, running: &RunningTotal) -> io::Result<Columns> {
        let Column::Cell(name) = self.measure.column else {
            return Err(invalid(format!("{}'s {} is computed, not a running total", self.series.kind, self.measure.name)));
        };
        let reason = running.reason(name).ok_or_else(|| invalid(format!("{}'s {name} names no reason column", self.series.kind)))?;
        let table = self.series.table;
        Ok(Columns {
            value: dashboard_declare::field(table, name)?,
            reason: dashboard_declare::field(table, reason)?,
            since: dashboard_declare::field(table, running.since)?,
            as_of: dashboard_declare::field(table, running.as_of)?,
        })
    }

    fn set_keys(&self, running: &RunningTotal, owner: u128, parts: &[u128]) -> io::Result<(Vec<(u128, u128)>, Option<u128>)> {
        let layout = self.series.parts.ok_or_else(|| invalid(format!("{} has no parts", self.series.kind)))?;
        Ok((parts.iter().map(|&part| (part, layout.key(owner, part))).collect(), running.rest.map(|rest| layout.key(owner, rest))))
    }

    fn column_runs(&self, run: u32, column: usize, keys: &[u128], first: i32, last: i32) -> io::Result<HashMap<u128, Runs>> {
        let rows = read::runs(self.query.replay.recording(), run, self.series.table, Source::Cell(column), Some(keys), first, last)?;
        #[cfg(test)]
        tests::HISTORY_ROWS.set(tests::HISTORY_ROWS.get() + rows.values().map(Vec::len).sum::<usize>());
        Ok(rows)
    }

    pub fn total_history<C: Clone + Into<Country>>(&self, tags: &[C], parts: Parts<'_>, back: Back<'_>, from: i32, to: i32) -> io::Result<Vec<Vec<TotalRun>>> {
        if self.series.key.unowned() {
            return Err(invalid(format!("{} is rows no country owns", self.series.kind)));
        }
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let timeline = &self.query.timeline;
        let mut out = Vec::with_capacity(tags.len());
        for tag in tags {
            let whole: Vec<Part> = self.held(&Ask::whole(tag.clone()), timeline.first, timeline.last);
            let opening = whole.first().map(|part| part.first);
            let mut runs: Vec<TotalRun> = Vec::new();
            for part in whole.iter().filter(|part| part.first <= to && part.last >= from) {
                let owner = self.series.key.of(part.slot, part.id);
                let (keys, rest) = self.set_keys(running, owner, &parts(part.run))?;
                let shown = (from.max(part.first), to.min(part.last));
                let mut opened = Some(part.first) != opening && shown.0 == part.first;
                self.history_spans(part.run, shown, |first, last| match back {
                    None => self.part_totals(part, &keys, rest, &columns, (first, last)),
                    Some(back) => self.part_windows(running, part, &keys, rest, &columns, (first, last), back),
                }, |first, last, result| {
                    let values = result.unwrap_or_else(|reason| vec![(first, last, Err(HistoryGap::Damage(reason)))]);
                    for (first, last, value) in values {
                        let damage = matches!(value, Err(HistoryGap::Damage(_)));
                        runs.push(TotalRun { first, last, value, opens: opened });
                        opened = damage;
                    }
                })?;
            }
            out.push(runs);
        }
        Ok(out)
    }

    fn history_spans<T>(&self, run: u32, (first, last): (i32, i32), read: impl Fn(i32, i32) -> io::Result<T>, mut emit: impl FnMut(i32, i32, Result<T, Arc<str>>)) -> io::Result<()> {
        match read(first, last) {
            Ok(value) => { emit(first, last, Ok(value)); return Ok(()) }
            Err(error) if !block_damage(&error) => return Err(error),
            Err(_) => {}
        }
        let recording = self.query.replay.recording();
        let held = recording.run(run)?;
        let mut lo = first;
        loop {
            let block = held.place(read::ordinal(recording, run, lo)?).0;
            let hi = held.block_first_hour(block).saturating_add(replay_columnar::unit::BLOCK as i32 - 1).min(last);
            match read(lo, hi) {
                Ok(value) => emit(lo, hi, Ok(value)),
                Err(error) if block_damage(&error) => {
                    let (mut from, mut to) = (lo, hi);
                    while from < to {
                        let mid = from + (to - from) / 2;
                        match read(lo, mid) {
                            Ok(_) => from = mid + 1,
                            Err(error) if block_damage(&error) => to = mid,
                            Err(error) => return Err(error),
                        }
                    }
                    if lo < from { emit(lo, from - 1, Ok(read(lo, from - 1)?)); }
                    emit(from, hi, Err(Arc::from(error.to_string())));
                }
                Err(error) => return Err(error),
            }
            if hi == last { return Ok(()); }
            lo = hi + 1;
        }
    }

    pub fn totals_at(&self, countries: &[Country], parts: Parts<'_>, back: Back<'_>, hour: i32) -> io::Result<Vec<Result<i64, Gap>>> {
        if self.series.key.unowned() {
            return Err(invalid(format!("{} is rows no country owns", self.series.kind)));
        }
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let Some(stretch) = self.query.timeline.at(hour) else { return Ok(vec![Err(Gap::NotRecorded); countries.len()]) };
        let parts = parts(stretch.run);
        let mut wanted = BTreeSet::new();
        let mut owners = Vec::with_capacity(countries.len());
        for country in countries {
            let owner = match self.query.holders.at(country, hour) {
                Some(held) if let Holder::Found { slot, id } = held.holder => {
                    let (keys, rest) = self.set_keys(running, self.series.key.of(slot, id), &parts)?;
                    wanted.extend(keys.iter().map(|&(_, key)| key).chain(rest));
                    Some((held.first.max(stretch.first), keys, rest))
                }
                _ => None,
            };
            owners.push(owner);
        }
        let Some(first) = owners.iter().flatten().map(|(first, _, _)| *first).min() else {
            return Ok(vec![Err(Gap::NotRecorded); countries.len()]);
        };
        let wanted: Vec<_> = wanted.into_iter().collect();
        let amounts = |at| [columns.value, columns.reason].into_iter()
            .map(|col| Ok((col, self.column_runs(stretch.run, col, &wanted, at, at)?))).collect::<io::Result<HashMap<_, _>>>();
        let clock = |at| self.clock_values(stretch.run, running, &columns, at);
        if let Some(back) = back {
            let (read, since_hour) = clock(hour)?;
            let dates = read.filter(|&r| first <= r && r <= hour)
                .and_then(|r| back(r).filter(|&s| first <= s && s <= hour).map(|s| (r, s)));
            let Some((read, start)) = dates else {
                return Ok(owners.iter().map(|owner| Err(if owner.is_some() { Gap::WindowIncomplete } else { Gap::NotRecorded })).collect());
            };
            let window = TotalWindow {
                read, start: Some(start), opening_read: clock(start)?.0, since: if read == hour { since_hour } else { clock(read)?.1 },
            };
            let now = amounts(read)?;
            let before = amounts(start)?;
            return Ok(owners.into_iter().map(|owner| {
                let (first, keys, rest) = owner.ok_or(Gap::NotRecorded)?;
                let value = window.value(first, |at| Self::set_at(if at == read { &now } else { &before }, &columns, &keys, rest, at));
                after_restart(value, Some(start), since_hour)
            }).collect());
        }
        let now = amounts(hour)?;
        Ok(owners.into_iter().map(|owner| {
            let (_, keys, rest) = owner.ok_or(Gap::NotRecorded)?;
            i64::try_from(Self::set_at(&now, &columns, &keys, rest, hour)?).map_err(|_| Gap::OutOfRange)
        }).collect())
    }

    fn set_at(runs: &ColumnRuns, columns: &Columns, keys: &[(u128, u128)], rest: Option<u128>, hour: i32) -> Result<i128, Gap> {
        let cell = |col, key| at(runs.get(&col).and_then(|by| by.get(&key)), hour);
        sum_parts(keys.iter().copied(), rest, |part, key| {
            cell(columns.value, key).map(Ok).or_else(|| cell(columns.reason, key).map(|why| Err(Gap::Cell { part, reason: why as u8 })))
        })?.ok_or(Gap::NotRecorded)
    }

    fn set_runs(&self, run: u32, columns: &[usize], keys: &[(u128, u128)], rest: Option<u128>, first: i32, last: i32) -> io::Result<ColumnRuns> {
        let mut wanted: Vec<u128> = keys.iter().map(|&(_, key)| key).chain(rest).collect();
        wanted.sort_unstable();
        wanted.dedup();
        columns.iter().map(|&column| Ok((column, self.column_runs(run, column, &wanted, first, last)?))).collect()
    }

    fn part_totals(&self, part: &Part, keys: &[(u128, u128)], rest: Option<u128>, columns: &Columns, (first, last): (i32, i32)) -> io::Result<HistoryRuns> {
        let runs = self.set_runs(part.run, &[columns.value, columns.reason], keys, rest, first, last)?;
        let mut cuts: BTreeSet<i32> = BTreeSet::from([first]);
        for by in runs.values() {
            for held in by.values() {
                cuts.extend(held.iter().flat_map(|run| [run.0, run.1.saturating_add(1)]).filter(|&hour| first < hour && hour <= last));
            }
        }
        let mut out = Vec::new();
        let cuts: Vec<i32> = cuts.into_iter().collect();
        for (index, &from) in cuts.iter().enumerate() {
            let to = cuts.get(index + 1).map_or(last, |next| next - 1);
            let value = Self::set_at(&runs, columns, keys, rest, from).and_then(|sum| i64::try_from(sum).map_err(|_| Gap::OutOfRange));
            push(&mut out, from, to, value.map_err(HistoryGap::Missing));
        }
        Ok(out)
    }

    fn part_windows(&self, running: &RunningTotal, part: &Part, keys: &[(u128, u128)], rest: Option<u128>, columns: &Columns, (first, last): (i32, i32), back: &dyn Fn(i32) -> Option<i32>) -> io::Result<HistoryRuns> {
        let load = |lo, hi| Ok((
            self.set_runs(part.run, &[columns.since, columns.as_of], &[(0, running.clock)], None, lo, hi)?,
            self.set_runs(part.run, &[columns.value, columns.reason], keys, rest, lo, hi)?,
        ));
        let (clock, amounts) = load(first, last)?;
        let since = &clock[&columns.since];
        let reads = clock[&columns.as_of].get(&running.clock).map_or(&[][..], Vec::as_slice);
        let sample = |clock: &ColumnRuns, amounts: &ColumnRuns, at| WindowEndpoint {
            read: clock_at(&clock[&columns.as_of], running.clock, at),
            since: clock_at(&clock[&columns.since], running.clock, at),
            sum: Self::set_at(amounts, columns, keys, rest, at),
        };
        let dates: BTreeSet<_> = reads.iter().filter_map(|r| r.2.and_then(|v| i32::try_from(v).ok()))
            .flat_map(|r| std::iter::once(r).chain(back(r))).filter(|&at| part.first <= at && at < first).collect();
        let dates: Vec<_> = dates.into_iter().collect();
        let mut earlier = HashMap::new();
        let run = self.query.replay.recording().run(part.run)?;
        let block = |at: i32| (i64::from(at) - i64::from(run.first_hour)) / i64::from(replay_columnar::unit::BLOCK);
        for dates in dates.chunk_by(|a, b| block(*a) == block(*b)) {
            self.history_spans(part.run, (dates[0], dates[dates.len() - 1]), load, |lo, hi, result| {
                for &at in &dates[dates.partition_point(|&at| at < lo)..dates.partition_point(|&at| at <= hi)] {
                    earlier.insert(at, result.as_ref().map(|(clock, amounts)| sample(clock, amounts, at)).map_err(Arc::clone));
                }
            })?;
        }
        let endpoint = |at| match earlier.get(&at) {
            Some(result) => result.clone().map_err(HistoryGap::Damage),
            None => Ok(sample(&clock, &amounts, at)),
        };
        let window = |read| {
            let Some(start) = back(read).filter(|&at| at >= part.first && read >= part.first) else {
                return Err(HistoryGap::Missing(Gap::WindowIncomplete));
            };
            let now = endpoint(read)?;
            let before = endpoint(start)?;
            TotalWindow { read, start: Some(start), opening_read: before.read, since: now.since }
                .value(part.first, |at| if at == read { now.sum } else { before.sum }).map_err(HistoryGap::Missing)
        };
        let mut out = Vec::new();
        let mut hour = first;
        for &(run_first, run_last, read) in reads.iter().filter(|run| run.1 >= first && run.0 <= last) {
            let (lo, hi) = (run_first.max(first), run_last.min(last));
            if lo > hour {
                push(&mut out, hour, lo - 1, Err(HistoryGap::Missing(Gap::WindowIncomplete)));
            }
            let read = read.and_then(|read| i32::try_from(read).ok());
            let value = read.map_or(Err(HistoryGap::Missing(Gap::WindowIncomplete)), window);
            let mut cuts: Vec<i32> = since.get(&running.clock).into_iter().flatten().map(|run| run.0).filter(|&at| lo < at && at <= hi).collect();
            cuts.insert(0, lo);
            for (index, &from) in cuts.iter().enumerate() {
                let to = cuts.get(index + 1).map_or(hi, |next| next - 1);
                push(&mut out, from, to, after_restart(value.clone(), read.and_then(back), clock_at(since, running.clock, from)));
            }
            hour = hi + 1;
        }
        if hour <= last {
            push(&mut out, hour, last, Err(HistoryGap::Missing(Gap::WindowIncomplete)));
        }
        Ok(out)
    }

    pub fn totals_ranked(&self, hour: i32, n: usize, parts: Parts<'_>, back: Back<'_>, keep: &dyn Fn(i64) -> bool) -> io::Result<Result<Vec<String>, String>> {
        if let Some(reason) = self.query.holders.not_ranked(hour) {
            return Ok(Err(reason));
        }
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let Some(stretch) = self.query.timeline.at(hour).copied() else {
            return Ok(Err(TOTALS_HOUR_UNRECORDED.to_owned()));
        };
        let recording = self.query.replay.recording();
        let table = self.series.table;
        let snapshot = |at: i32, wanted: &[usize], keys: read::Keys<'_>| -> io::Result<Option<HashMap<u128, Vec<Option<i64>>>>> {
            let read = crate::cost::storage(|| replay_lanes::lanes::at_keys(recording, stretch.run, table, at, Some(wanted), keys))?;
            if !matches!(read.status, Status::Complete) {
                return Ok(None);
            }
            let view = read.view();
            Ok(Some(read.rows.iter().map(|&(key, slot)| (key, wanted.iter().map(|&column| view.number(column, slot)).collect())).collect()))
        };
        let clock_keys = [running.clock];
        let clock = |at: i32| -> io::Result<Option<(Option<i32>, Option<i32>)>> {
            let held = snapshot(at, &[columns.since, columns.as_of], read::Keys::These(&clock_keys))?;
            Ok(held.and_then(|rows| rows.get(&running.clock).map(|cells| (cells[0].and_then(|hour| i32::try_from(hour).ok()), cells[1].and_then(|hour| i32::try_from(hour).ok())))))
        };
        let Some((since, Some(read))) = clock(hour)? else { return Ok(Err(TOTALS_NOT_READ.to_owned())) };
        if read < stretch.first {
            return Ok(Err(TOTALS_NOT_READ.to_owned()));
        }
        let wanted = [columns.value, columns.reason];
        let Some(now) = snapshot(read, &wanted, read::Keys::All)? else { return Ok(Err(TOTALS_NOT_READ.to_owned())) };
        let start = match back {
            None => None,
            Some(back) => {
                let Some(start) = back(read).filter(|&start| start >= stretch.first && since.is_none_or(|since| since <= start)) else { return Ok(Err(TOTALS_WINDOW_UNRECORDED.to_owned())) };
                if let Some(reason) = self.query.holders.not_ranked(start) {
                    return Ok(Err(reason));
                }
                if clock(start)?.and_then(|(_, read)| read) != Some(start) {
                    return Ok(Err(TOTALS_WINDOW_UNRECORDED.to_owned()));
                }
                match snapshot(start, &wanted, read::Keys::All)? {
                    Some(before) => Some((start, before)),
                    None => return Ok(Err(TOTALS_WINDOW_UNRECORDED.to_owned())),
                }
            }
        };
        let set = parts(stretch.run);
        let now_value = |key| now.get(&key).map(|cells| cells[0]);
        let before_value = |key| start.as_ref()?.1.get(&key).map(|cells| cells[0]);
        let before = start.as_ref().map(|_| &before_value as RankedCells<'_>);
        let mut scored: Vec<(i128, &str)> = Vec::new();
        for tag in self.query.holders.tags.keys() {
            let Some(run) = self.query.holders.at(tag, hour) else { continue };
            let Holder::Found { slot, id } = run.holder else { continue };
            let (keys, rest) = self.set_keys(running, self.series.key.of(slot, id), &set)?;
            match ranked_total(keys.iter().map(|&(_, key)| key), rest, &now_value, before, keep) {
                Ok(Some(value)) => scored.push((i128::from(value), tag.as_str())),
                Ok(None) => {}
                Err(reason) => return Ok(Err(reason.to_owned())),
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        Ok(Ok(scored.into_iter().take(n).map(|(_, tag)| tag.to_owned()).collect()))
    }

    fn clock_values(&self, run: u32, running: &RunningTotal, columns: &Columns, hour: i32) -> io::Result<(Option<i32>, Option<i32>)> {
        let runs = self.set_runs(run, &[columns.as_of, columns.since], &[(0, running.clock)], None, hour, hour)?;
        Ok((clock_at(&runs[&columns.as_of], running.clock, hour), clock_at(&runs[&columns.since], running.clock, hour)))
    }

    pub fn observation_at(&self, hour: i32) -> io::Result<(Option<i32>, Option<i32>)> {
        let Some(stretch) = self.query.timeline.at(hour) else { return Ok((None, None)) };
        let running = self.running()?;
        let (read, since) = self.clock_values(stretch.run, running, &self.total_columns(running)?, hour)?;
        Ok((read.filter(|&at| at <= hour), since))
    }

    pub fn clock_at(&self, hour: i32) -> io::Result<ClockAt> {
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let Some(stretch) = self.query.timeline.at(hour).copied() else {
            return Ok(ClockAt { read: None, since: None, since_from: None, next_change: None });
        };
        let (read, since) = self.clock_values(stretch.run, running, &columns, hour)?;
        let read = read.filter(|&at| at <= hour);
        let recording = self.query.replay.recording();
        let next = |column| match crate::cost::storage(|| replay_lanes::lanes::next(recording, stretch.run, self.series.table, column, running.clock, hour)) {
            Err(error) if block_damage(&error) => Ok(hour.checked_add(1)),
            result => result,
        };
        let next_change = [next(columns.as_of)?, next(columns.since)?].into_iter().flatten().min().filter(|&at| at <= stretch.last);
        let start = read.unwrap_or(stretch.first).max(stretch.first);
        let since_from = if start < hour {
            match self.column_runs(stretch.run, columns.since, &[running.clock], start, hour) {
                Ok(runs) => runs.get(&running.clock).and_then(|runs| runs.last()).map(|run| run.0),
                Err(error) if block_damage(&error) => Some(hour),
                Err(error) => return Err(error),
            }
        } else { None };
        Ok(ClockAt { read, since, since_from, next_change })
    }

    pub fn since_runs(&self, from: i32, to: i32) -> io::Result<Vec<(i32, i32, Result<Option<i32>, Arc<str>>)>> {
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let mut out = Vec::new();
        for stretch in self.query.timeline.within(from, to) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            self.history_spans(stretch.run, (first, last), |lo, hi|
                self.column_runs(stretch.run, columns.since, &[running.clock], lo, hi),
                |lo, hi, held| match held {
                    Ok(runs) => out.extend(runs.get(&running.clock).into_iter().flatten()
                        .map(|r| (r.0.max(lo), r.1.min(hi), Ok(r.2.and_then(|v| i32::try_from(v).ok()))))),
                    Err(reason) => out.push((lo, hi, Err(reason))),
                })?;
        }
        Ok(out)
    }

    pub fn clock(&self, from: i32, to: i32) -> io::Result<Clock> {
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let (mut reads, mut since) = (BTreeSet::new(), BTreeSet::new());
        for stretch in self.query.timeline.within(from, to) {
            let runs = self.set_runs(stretch.run, &[columns.since, columns.as_of], &[(0, running.clock)], None, stretch.first, stretch.last)?;
            let values = |column: usize| runs.get(&column).and_then(|by| by.get(&running.clock)).into_iter().flatten().filter_map(|run| run.2.and_then(|hour| i32::try_from(hour).ok()));
            reads.extend(values(columns.as_of).filter(|&read| from <= read && read <= to));
            since.extend(values(columns.since));
        }
        Ok(Clock { reads: reads.into_iter().collect(), since: since.into_iter().collect() })
    }
}

fn push(out: &mut HistoryRuns, first: i32, last: i32, value: HistoryValue) {
    match out.last_mut() {
        Some(open) if open.2 == value && open.1.checked_add(1) == Some(first) => open.1 = last,
        _ => out.push((first, last, value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        pub(super) static HISTORY_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn single_hour_batch_matches_the_selected_totals_and_their_gaps() {
        use crate::identity::{HolderRun, Holders};
        use crate::timeline::{Join, Stretch, Timeline};
        use replay_data::rows::{ProducedValueRow, Recorded, TableId};
        use replay_journal::synthetic::{Input, Sample};

        let (first, last) = (100, 110);
        let replay = crate::fixture::bare(first, last, |frame, hour| {
            let mut rows = vec![ProducedValueRow { country: ProducedValueRow::CLOCK, since: Some(if hour >= 109 { 108 } else { first }),
                as_of: Some(if hour == 107 { 106 } else { hour }), ..Default::default() }];
            for country in [7, 9] {
                if hour == first { continue; }
                let units = match hour { 103 => Recorded::NotRecorded, 104 => Recorded::Value(i64::MAX), _ => Recorded::Value(i64::from(country) + i64::from(hour)) };
                rows.push(ProducedValueRow { country, archetype: 1, units, units_reason: ProducedValueRow::LOST_OUTPUT, ..Default::default() });
                rows.push(ProducedValueRow { country, archetype: ProducedValueRow::REST, units: Recorded::Value(2), ..Default::default() });
            }
            frame.table(TableId::ProducedValues, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit })
        }).unwrap();
        let holder = |first, last, slot| HolderRun { first, last, holder: Holder::Found { slot, id: slot }, starts_segment: true };
        let holders = Holders::new([
            ("AAA".into(), vec![holder(first, last, 7)]),
            ("BBB".into(), vec![holder(first, 105, 7), holder(106, last, 9)]),
            ("CCC".into(), vec![HolderRun { first, last, holder: Holder::Unresolved, starts_segment: true }]),
        ].into(), Vec::new());
        let timeline = Timeline { id: 1, first, last, stretches: vec![Stretch { run: 0, first, last, join: Join::Start }], unheld: Vec::new() };
        let query = crate::Query { replay: &replay, timeline: timeline.into(), holders: holders.into() };
        let series = dashboard_declare::series("produced-values").unwrap();
        let line = Line::new(&query, series, series.measure("units").unwrap()).unwrap();
        let tags = ["AAA", "BBB", "CCC", "DDD"].map(|tag| Country::Tag(tag.into()));
        let parts = |_| vec![1, 2];
        for (hour, start, first_value) in [
            (99, None, Err(Gap::NotRecorded)), (100, None, Err(Gap::NotRecorded)), (101, None, Ok(110)),
            (103, None, Err(Gap::Cell { part: 1, reason: ProducedValueRow::LOST_OUTPUT })), (104, None, Err(Gap::OutOfRange)),
            (105, Some(100), Err(Gap::NotRecorded)), (107, Some(101), Ok(5)),
            (108, Some(107), Err(Gap::WindowIncomplete)), (109, Some(106), Err(Gap::Restarted)),
        ] {
            let opening = |_| start;
            let back = start.map(|_| &opening as &dyn Fn(i32) -> Option<i32>);
            HISTORY_ROWS.set(0);
            let single = line.totals_at(&tags, &parts, back, hour).unwrap();
            let batch_rows = HISTORY_ROWS.replace(0);
            let history = line.total_history(&tags, &parts, back, hour, hour).unwrap();
            let history_rows = HISTORY_ROWS.replace(0);
            assert_eq!(single.iter().map(|v| v.map_err(HistoryGap::Missing)).collect::<Vec<_>>(), history.iter().map(|runs| runs.first().map_or(Err(HistoryGap::Missing(Gap::NotRecorded)), |r| r.value.clone())).collect::<Vec<_>>(), "hour {hour}, opening {start:?}");
            assert_eq!(single[0], first_value);
            assert_eq!(single[2..], [Err(Gap::NotRecorded); 2]);
            if hour == 107 {
                assert_eq!(single[1], Err(Gap::WindowIncomplete), "a different holder at the opening");
                eprintln!("window history rows: {history_rows} per-country, {batch_rows} batched");
                assert!(batch_rows < history_rows);
            }
        }
        let reads = |countries: &[Country]| {
            crate::cost::take_calls();
            line.totals_at(countries, &parts, Some(&|_| Some(106)), 108).unwrap();
            crate::cost::take_calls()
        };
        assert_eq!(reads(&tags[..1]), reads(&tags), "additional lines share the same dated storage reads");
        let series = dashboard_declare::series("country-measure").unwrap();
        let other = Line::new(&query, series, series.measure("politicalPower").unwrap()).unwrap();
        assert_eq!(other.totals_at(&tags, &parts, None, last).unwrap_err().to_string(), other.total_history(&tags, &parts, None, last, last).unwrap_err().to_string());
    }

    #[test]
    fn single_hour_window_matches_history_across_a_damaged_production_block() {
        use crate::identity::{HolderRun, Holders};
        use crate::timeline::{Join, Stretch, Timeline};
        use replay::campaign::Replay;
        use replay_columnar::unit;
        use replay_data::rows::{ProducedValueRow, Recorded, TableId};
        use replay_journal::synthetic::{Input, Sample};
        use replay_recording::columnar_frames::{Damage, DamageScope};

        let table = TableId::ProducedValues;
        let restore_block = unit::Settings::at(1).restore_blocks - 1;
        let damaged_block = restore_block - 1;
        let block = unit::BLOCK as i32;
        let opening = damaged_block as i32 * block - 1;
        let changed = opening + 2;
        let carried = (damaged_block as i32 + 1) * block + 3;
        let restored = (restore_block as i32 + 1) * block + 3;
        let last = restored + 3;
        let clean = crate::fixture::bare(0, last, |frame, hour| {
            let read = [0, opening, changed, carried, restored, last].into_iter().take_while(|at| *at <= hour).last();
            let mut rows = vec![ProducedValueRow { country: ProducedValueRow::CLOCK, since: Some(0), as_of: read, ..Default::default() }];
            for (country, units) in [(7, if hour < changed { 9 } else { 16 }), (8, 11)] {
                rows.push(ProducedValueRow { country, archetype: 1, units: Recorded::Value(units), ..Default::default() });
            }
            frame.table(table, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit })
        }).unwrap();
        let entry = |block| clean.recording().table_entry(0, block, table.index()).unwrap().unit;
        let damaged_entry = entry(damaged_block);
        assert!(damaged_entry.restore.is_none() && entry(restore_block).restore.is_some());
        let units = unit::column_stream(dashboard_declare::field(table, "units").unwrap());
        let frame = damaged_entry.parts.iter().flat_map(|part| &part.frames).find(|frame| frame.streams.contains(&units)).unwrap();
        let mut bytes = clean.bytes().to_vec();
        bytes[(frame.offset + u64::from(frame.stored / 2)) as usize] ^= 0x40;
        let damaged = Replay::open(bytes).unwrap();
        let tags = ["AAA", "BBB"].map(Country::from);
        let held = |slot| vec![HolderRun { first: 0, last, holder: Holder::Found { slot, id: slot }, starts_segment: true }];
        let holders = std::sync::Arc::new(Holders::new([("AAA".into(), held(7)), ("BBB".into(), held(8))].into(), vec![]));
        let timeline = std::sync::Arc::new(Timeline { id: 1, first: 0, last, stretches: vec![Stretch { run: 0, first: 0, last, join: Join::Start }], unheld: vec![] });
        let series = dashboard_declare::series("produced-values").unwrap();
        let scope = DamageScope::Table { run: 0, block: damaged_block, table: table.index() as u32 };
        let refusal = |error: io::Error| {
            assert_eq!(Damage::of(&error).unwrap().scope, scope);
            error.to_string()
        };
        for (replay, corrupt) in [(&clean, false), (&damaged, true)] {
            let query = crate::Query { replay, holders: holders.clone(), timeline: timeline.clone() };
            let line = Line::new(&query, series, series.measure("units").unwrap()).unwrap();
            let parts = |_| vec![1];
            for (hour, read, start, dependent) in [
                (changed + 1, changed, opening, true), (carried + 1, carried, opening, true),
                (restored - 1, carried, opening, true), (restored + 1, restored, changed, true),
                (restored + 1, restored, opening, false),
            ] {
                let back = |_| Some(start);
                let batch = line.totals_at(&tags, &parts, Some(&back), hour).map_err(refusal);
                let ranked = line.totals_ranked(hour, 2, &parts, Some(&back), &|v| v > 0).map_err(refusal);
                let chart = line.total_history(&tags, &parts, Some(&back), hour, hour).unwrap();
                if corrupt && dependent {
                    let reason = batch.unwrap_err();
                    assert_eq!(ranked.unwrap_err(), reason);
                    for country in chart {
                        assert_eq!(country[0].value, Err(HistoryGap::Damage(Arc::from(reason.as_str()))));
                    }
                } else {
                    let difference = if start == changed { 0 } else { 7 };
                    assert_eq!(batch.unwrap(), [Ok(difference), Ok(0)]);
                    assert_eq!(ranked.unwrap().unwrap(), if difference > 0 { vec!["AAA"] } else { vec![] });
                    for (country, value) in chart.iter().zip([difference, 0]) {
                        assert_eq!(country[..], [TotalRun { first: hour, last: hour, value: Ok(value), opens: false }]);
                    }
                    let clock = line.clock_at(hour).unwrap();
                    let next = if read == changed { carried } else if read == carried { restored } else { last };
                    assert_eq!((clock.read, clock.since, clock.since_from, clock.next_change), (Some(read), Some(0), Some(read), Some(next)));
                }
            }
            if corrupt {
                let before = line.clock_at(opening).unwrap();
                assert_eq!((before.read, before.since), (Some(opening), Some(0)));
                let unreadable = (opening + 1..=changed).find(|&at| line.totals_at(&tags, &parts, None, at).is_err()).unwrap();
                assert!(before.next_change.is_some_and(|next| next <= unreadable), "clock at {opening} held until {:?} across damage at {unreadable}", before.next_change);
                let back = |_| Some(opening);
                for window in [None, Some(&back as &dyn Fn(_) -> _)] {
                    let history = line.total_history(&tags, &parts, window, opening, last).unwrap();
                    for (i, country) in history.iter().enumerate() {
                        let at = |hour| country.iter().find(|r| r.first <= hour && hour <= r.last).unwrap();
                        assert_eq!(at(opening).value, Ok(if window.is_some() { 0 } else { [9, 11][i] }));
                        for hour in [changed, carried] {
                            assert!(matches!(&at(hour).value, Err(HistoryGap::Damage(reason)) if reason.contains("differs from its digest")));
                        }
                        let resumed = at(restored);
                        assert_eq!(resumed.value, Ok(if window.is_some() { [7, 0][i] } else { [16, 11][i] }));
                        let last_damage = country.iter().rposition(|r| matches!(r.value, Err(HistoryGap::Damage(_)))).unwrap();
                        assert!(country[last_damage + 1].opens);
                        assert!(country[last_damage].last < resumed.first);
                    }
                }
            }
        }
    }

    #[test]
    fn ranked_total_uses_the_races_fallback_window_and_refusal_order() {
        use crate::{identity::{HolderRun, Holders}, timeline::{Join, Stretch, Timeline}};
        use replay_data::rows::{ProducedValueRow, Recorded, TableId};
        use replay_journal::synthetic::{Input, Sample};

        let (first, last) = (100, 103);
        let key = |part| ProducedValueRow::key_of(7, part);
        let keys = [key(1), key(2)];
        let rest = ProducedValueRow::REST;
        let cell = |hour, k| match (hour, k) {
            (101, k) if k == key(1) => Some(Some(5)),
            (102, k) if k == key(1) => Some(None),
            (103, k) if k == key(1) => Some(Some(i64::MAX)),
            (101..=103, k) if k == key(rest) => Some(Some(2)),
            _ => None,
        };
        let replay = crate::fixture::bare(first, last, |frame, hour| {
            let mut rows = vec![ProducedValueRow { country: ProducedValueRow::CLOCK, since: Some(first), as_of: Some(hour), ..Default::default() }];
            for part in [1, rest] {
                if let Some(value) = cell(hour, key(part)) {
                    rows.push(ProducedValueRow { country: 7, archetype: part, units: value.map_or(Recorded::NotRecorded, Recorded::Value), ..Default::default() });
                }
            }
            frame.table(TableId::ProducedValues, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit })
        }).unwrap();
        let holder = HolderRun { first, last, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: true };
        let timeline = Timeline { id: 1, first, last, stretches: vec![Stretch { run: 0, first, last, join: Join::Start }], unheld: Vec::new() };
        let query = crate::Query { replay: &replay, timeline: timeline.into(), holders: Holders::new([("AAA".into(), vec![holder])].into(), Vec::new()).into() };
        let series = dashboard_declare::series("produced-values").unwrap();
        let line = Line::new(&query, series, series.measure("units").unwrap()).unwrap();
        for (at, from, expected) in [
            (101, None, Ok(Some(7))),
            (101, Some(100), Err(TOTALS_WINDOW_UNRECORDED)),
            (103, None, Err(TOTALS_VALUE_UNKNOWN)),
            (102, Some(100), Err(TOTALS_VALUE_UNKNOWN)),
        ] {
            let opening = |k| cell(from.unwrap(), k);
            let keep = |v| v > 0;
            assert_eq!(ranked_total(keys.into_iter(), Some(key(rest)), &|k| cell(at, k), from.map(|_| &opening as RankedCells<'_>), &keep), expected);
            let back = |_| from;
            assert_eq!(line.totals_ranked(at, 1, &|_| vec![1, 2], from.map(|_| &back as &dyn Fn(_) -> _), &keep).unwrap(),
                expected.map(|_| vec!["AAA".to_owned()]).map_err(str::to_owned));
        }
    }

    #[test]
    fn a_contender_without_an_opening_row_refuses_a_window_it_delivered_in_and_is_skipped_where_it_delivered_nothing() {
        use crate::identity::{HolderRun, Holders};
        use crate::timeline::{Join, Stretch, Timeline};
        use replay_data::rows::{ProducedValueRow, Recorded, TableId};
        use replay_journal::synthetic::{Input, Sample};
        let (first, last, read, opening) = (90, 130, 124, 100);
        for (units, expected) in [(5_i64, Err(TOTALS_WINDOW_UNRECORDED.to_owned())), (0, Ok(Vec::new()))] {
            let replay = crate::fixture::bare(first, last, |frame, hour| {
                let clock = ProducedValueRow { country: ProducedValueRow::CLOCK, archetype: 0, since: Some(first), as_of: Some(hour), ..Default::default() };
                let produced = ProducedValueRow { country: 7, archetype: ProducedValueRow::WHOLE, units: Recorded::Value(units), ..Default::default() };
                let rows = if hour == opening { vec![clock] } else { vec![produced, clock] };
                frame.table(TableId::ProducedValues, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit })
            })
            .unwrap();
            let holders = Holders::new([("GER".to_owned(), vec![HolderRun { first, last, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: true }])].into(), Vec::new());
            let timeline = Timeline { id: 1, first, last, stretches: vec![Stretch { run: 0, first, last, join: Join::Start }], unheld: Vec::new() };
            let query = crate::Query { replay: &replay, timeline: std::sync::Arc::new(timeline), holders: std::sync::Arc::new(holders) };
            let series = dashboard_declare::series("produced-values").unwrap();
            let line = Line::new(&query, series, series.measure("units").unwrap()).unwrap();
            let whole = |_: u32| vec![u128::from(ProducedValueRow::WHOLE)];
            let from_opening = |_: i32| Some(opening);
            assert_eq!(
                line.totals_ranked(read, 5, &whole, Some(&from_opening), &|value| value > 0).unwrap(),
                expected,
                "an opening without the country's row proves no amount for {units} delivered units"
            );
        }
    }

    #[test]
    fn a_daily_ranking_whose_comparison_hour_has_unknown_holders_is_refused_though_production_was_read_after_it() {
        use crate::identity::{HolderRun, Holders};
        use crate::timeline::{Join, Stretch, Timeline};
        use replay_data::rows::{ProducedValueRow, Recorded, TableId};
        use replay_journal::synthetic::{Input, Sample};
        let (first, last, read, compared) = (90, 130, 124, 100);
        let replay = crate::fixture::bare(first, last, |frame, hour| {
            let produced = ProducedValueRow { country: 7, archetype: ProducedValueRow::WHOLE, units: Recorded::Value(i64::from(hour)), ..Default::default() };
            let clock = ProducedValueRow { country: ProducedValueRow::CLOCK, archetype: 0, since: Some(first), as_of: Some(if hour == read { read } else { hour - 1 }), ..Default::default() };
            frame.table(TableId::ProducedValues, Input::Snapshot { rows: &[produced, clock], partial: None, read: true, sample: Sample::FromReadBit })
        })
        .unwrap();
        let reason = "Countries was not complete at these hours";
        let held = |from: i32, to: i32| HolderRun { first: from, last: to, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: true };
        let holders = Holders::new([("GER".to_owned(), vec![held(first, compared - 1), held(compared + 1, last)])].into(), vec![(compared, compared, reason.to_owned())]);
        let timeline = Timeline { id: 1, first, last, stretches: vec![Stretch { run: 0, first, last, join: Join::Start }], unheld: Vec::new() };
        let query = crate::Query { replay: &replay, timeline: std::sync::Arc::new(timeline), holders: std::sync::Arc::new(holders) };
        let series = dashboard_declare::series("produced-values").unwrap();
        let line = Line::new(&query, series, series.measure("units").unwrap()).unwrap();
        let whole = |_: u32| vec![u128::from(ProducedValueRow::WHOLE)];
        let day_before = |_: i32| Some(compared);
        assert_eq!(line.totals_ranked(read, 5, &whole, None, &|_| true).unwrap(), Ok(vec!["GER".to_owned()]), "the totals at the read hour rank GER");
        assert_eq!(
            line.totals_ranked(read, 5, &whole, Some(&day_before), &|_| true).unwrap(),
            Err(format!("Who holds each country is not known at this hour, so they cannot be ranked: {reason}")),
            "the comparison hour's holders are not known, though production there was read late: refused, never an empty ranking"
        );
    }

    #[test]
    fn a_ranking_at_an_hour_whose_holders_are_not_known_is_refused_with_the_recorded_reason() {
        use crate::fixture::{Data, FIRST, Fixture};
        let fixture = Fixture::new(Data::Power, FIRST..=FIRST + 40);
        let query = fixture.query();
        let reason = "Countries was not complete at these hours";
        let holders = std::sync::Arc::new(crate::identity::Holders::new(query.holders.tags.clone(), vec![(FIRST + 10, FIRST + 12, reason.to_owned())]));
        let query = crate::Query { holders, ..query };
        let series = dashboard_declare::series("country-measure").unwrap();
        let line = Line::new(&query, series, series.measure("politicalPower").unwrap()).unwrap();
        let ranked = line.totals_ranked(FIRST + 11, 5, &|_| Vec::new(), None, &|_| true).unwrap();
        assert_eq!(ranked, Err(format!("Who holds each country is not known at this hour, so they cannot be ranked: {reason}")), "no country is ranked, and the ranking says why, never an empty list");
    }

    #[test]
    fn a_set_sums_its_parts_reads_the_rest_for_a_part_without_a_row_and_is_unknown_where_one_is() {
        let columns = Columns { value: 0, reason: 1, since: 2, as_of: 3 };
        let (a, b, rest) = (10u128, 11u128, 12u128);
        let value: HashMap<u128, Runs> = HashMap::from([(a, vec![(0, 9, Some(4)), (10, 19, None)]), (b, vec![(5, 19, Some(3))]), (rest, vec![(0, 14, Some(0)), (15, 19, None)])]);
        let reason: HashMap<u128, Runs> = HashMap::from([(a, vec![(0, 19, Some(0))]), (b, vec![(5, 19, Some(0))]), (rest, vec![(0, 14, Some(0)), (15, 19, Some(6))])]);
        let runs = HashMap::from([(0usize, value), (1usize, reason)]);
        let set = [(1u128, a), (2u128, b)];
        let at = |hour: i32, rest: Option<u128>| Line::set_at(&runs, &columns, &set, rest, hour);
        assert_eq!(at(7, Some(rest)), Ok(7), "both parts' rows");
        assert_eq!(at(2, Some(rest)), Ok(4), "a part without a row reads the rest's zero");
        assert_eq!(at(12, Some(rest)), Err(Gap::Cell { part: 1, reason: 0 }), "a part's cell not recorded names the part");
        assert_eq!(at(2, None), Err(Gap::NotRecorded), "no rest: a part without a row is not recorded");
        let only_b = [(2u128, b)];
        assert_eq!(Line::set_at(&runs, &columns, &only_b, Some(rest), 16), Ok(3));
        let without = [(3u128, 99u128)];
        assert_eq!(Line::set_at(&runs, &columns, &without, Some(rest), 16), Err(Gap::Cell { part: 3, reason: 6 }), "the rest's reason, the asked part named");
        assert_eq!(Line::set_at(&runs, &columns, &without, Some(99), 16), Err(Gap::NotRecorded), "no rest row: not recorded");
        let mut out = Vec::new();
        push(&mut out, 0, 4, Ok(1));
        push(&mut out, 5, 9, Ok(1));
        push(&mut out, 11, 12, Ok(1));
        assert_eq!(out, vec![(0, 9, Ok(1)), (11, 12, Ok(1))], "equal neighbours join, a gap between does not");
    }
}
