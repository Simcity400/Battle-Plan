
use super::{Ask, Line, Part, invalid};
use crate::{
    identity::{Country, Holder},
    read,
};
use dashboard_declare::{Column, RunningTotal};
use replay_lanes::lanes::Runs;
use replay_columnar::{manifest::Status, summary::Source};
use std::{
    collections::{BTreeSet, HashMap},
    io,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    NotRecorded,
    Cell { part: u128, reason: u8 },
    WindowIncomplete,
    Restarted,
    OutOfRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TotalRun {
    pub first: i32,
    pub last: i32,
    pub value: Result<i64, Gap>,
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

const REACH: i32 = 2 * 744;

struct Held<'r> {
    value: Option<&'r Runs>,
    reason: Option<&'r Runs>,
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
        read::runs(self.query.replay.recording(), run, self.series.table, Source::Cell(column), Some(keys), first, last)
    }

    pub fn totals<C: Clone + Into<Country>>(&self, tags: &[C], parts: Parts<'_>, back: Back<'_>, from: i32, to: i32) -> io::Result<Vec<Vec<TotalRun>>> {
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
                let values = match back {
                    None => self.part_totals(part, &keys, rest, &columns, shown)?,
                    Some(back) => self.part_windows(running, part, &keys, rest, &columns, shown, back)?,
                };
                let opens = Some(part.first) != opening;
                runs.extend(values.into_iter().map(|(first, last, value)| TotalRun { first, last, value, opens: opens && first == part.first }));
            }
            out.push(runs);
        }
        Ok(out)
    }

    fn set_at(runs: &HashMap<usize, HashMap<u128, Runs>>, columns: &Columns, keys: &[(u128, u128)], rest: Option<u128>, hour: i32) -> Result<i128, Gap> {
        let held = |key: u128| Held { value: runs.get(&columns.value).and_then(|by| by.get(&key)), reason: runs.get(&columns.reason).and_then(|by| by.get(&key)) };
        let mut sum: i128 = 0;
        for &(part, key) in keys {
            let mut cell = held(key);
            if at(cell.reason, hour).is_none() && at(cell.value, hour).is_none() {
                match rest {
                    Some(rest) => cell = held(rest),
                    None => return Err(Gap::NotRecorded),
                }
            }
            match (at(cell.value, hour), at(cell.reason, hour)) {
                (Some(value), _) => sum += i128::from(value),
                (None, Some(reason)) => return Err(Gap::Cell { part, reason: reason as u8 }),
                (None, None) => return Err(Gap::NotRecorded),
            }
        }
        Ok(sum)
    }

    fn set_runs(&self, run: u32, columns: &[usize], keys: &[(u128, u128)], rest: Option<u128>, first: i32, last: i32) -> io::Result<HashMap<usize, HashMap<u128, Runs>>> {
        let mut wanted: Vec<u128> = keys.iter().map(|&(_, key)| key).chain(rest).collect();
        wanted.sort_unstable();
        wanted.dedup();
        columns.iter().map(|&column| Ok((column, self.column_runs(run, column, &wanted, first, last)?))).collect()
    }

    fn part_totals(&self, part: &Part, keys: &[(u128, u128)], rest: Option<u128>, columns: &Columns, (first, last): (i32, i32)) -> io::Result<Vec<(i32, i32, Result<i64, Gap>)>> {
        let runs = self.set_runs(part.run, &[columns.value, columns.reason], keys, rest, first, last)?;
        let mut cuts: BTreeSet<i32> = BTreeSet::from([first]);
        for by in runs.values() {
            for held in by.values() {
                cuts.extend(held.iter().flat_map(|run| [run.0, run.1.saturating_add(1)]).filter(|&hour| first < hour && hour <= last));
            }
        }
        let mut out: Vec<(i32, i32, Result<i64, Gap>)> = Vec::new();
        let cuts: Vec<i32> = cuts.into_iter().collect();
        for (index, &from) in cuts.iter().enumerate() {
            let to = cuts.get(index + 1).map_or(last, |next| next - 1);
            let value = Self::set_at(&runs, columns, keys, rest, from).and_then(|sum| i64::try_from(sum).map_err(|_| Gap::OutOfRange));
            push(&mut out, from, to, value);
        }
        Ok(out)
    }

    fn part_windows(&self, running: &RunningTotal, part: &Part, keys: &[(u128, u128)], rest: Option<u128>, columns: &Columns, (first, last): (i32, i32), back: &dyn Fn(i32) -> Option<i32>) -> io::Result<Vec<(i32, i32, Result<i64, Gap>)>> {
        let mut reach = REACH;
        let clock = loop {
            let from = first.saturating_sub(reach).max(part.first);
            let clock = self.set_runs(part.run, &[columns.since, columns.as_of], &[(0, running.clock)], None, from, last)?;
            let start = clock_at(&clock[&columns.as_of], running.clock, first).and_then(back);
            if from == part.first || start.is_some_and(|start| start >= from) {
                break (from, clock);
            }
            reach = reach.saturating_mul(4);
        };
        let (from, clock) = clock;
        let since = &clock[&columns.since];
        let as_of = &clock[&columns.as_of];
        let runs = self.set_runs(part.run, &[columns.value, columns.reason], keys, rest, from, last)?;
        let window = |read: i32| -> Result<i64, Gap> {
            if read < part.first {
                return Err(Gap::WindowIncomplete);
            }
            let start = back(read).filter(|&start| start >= part.first && clock_at(as_of, running.clock, start) == Some(start)).ok_or(Gap::WindowIncomplete)?;
            if clock_at(since, running.clock, read).is_some_and(|since| since > start) {
                return Err(Gap::Restarted);
            }
            let now = Self::set_at(&runs, columns, keys, rest, read)?;
            let before = Self::set_at(&runs, columns, keys, rest, start)?;
            i64::try_from(now - before).map_err(|_| Gap::OutOfRange)
        };
        let mut out: Vec<(i32, i32, Result<i64, Gap>)> = Vec::new();
        let reads: &Runs = as_of.get(&running.clock).map_or(&EMPTY, |runs| runs);
        let mut hour = first;
        for &(run_first, run_last, read) in reads.iter().filter(|run| run.1 >= first && run.0 <= last) {
            let (lo, hi) = (run_first.max(first), run_last.min(last));
            if lo > hour {
                push(&mut out, hour, lo - 1, Err(Gap::WindowIncomplete));
            }
            let read = read.and_then(|read| i32::try_from(read).ok());
            let value = read.map_or(Err(Gap::WindowIncomplete), |read| window(read));
            let mut cuts: Vec<i32> = since.get(&running.clock).into_iter().flatten().map(|run| run.0).filter(|&at| lo < at && at <= hi).collect();
            cuts.insert(0, lo);
            for (index, &from) in cuts.iter().enumerate() {
                let to = cuts.get(index + 1).map_or(hi, |next| next - 1);
                let restarted = read.and_then(back).is_some_and(|start| clock_at(since, running.clock, from).is_some_and(|since| since > start));
                push(&mut out, from, to, if restarted && value.is_ok() { Err(Gap::Restarted) } else { value });
            }
            hour = hi + 1;
        }
        if hour <= last {
            push(&mut out, hour, last, Err(Gap::WindowIncomplete));
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
        let value_of = |rows: &HashMap<u128, Vec<Option<i64>>>, keys: &[(u128, u128)], rest: Option<u128>| -> Result<Option<i128>, &'static str> {
            let mut sum: i128 = 0;
            for &(_, key) in keys {
                let Some(cells) = rows.get(&key).or_else(|| rest.and_then(|rest| rows.get(&rest))) else { return Ok(None) };
                sum += i128::from(cells[0].ok_or(TOTALS_VALUE_UNKNOWN)?);
            }
            Ok(Some(sum))
        };
        let mut scored: Vec<(i128, &str)> = Vec::new();
        for (tag, runs) in &self.query.holders.tags {
            let Some(run) = runs.iter().find(|run| run.first <= hour && hour <= run.last) else { continue };
            let Holder::Found { slot, id } = run.holder else { continue };
            let (keys, rest) = self.set_keys(running, self.series.key.of(slot, id), &set)?;
            let at = match value_of(&now, &keys, rest) {
                Ok(Some(at)) => at,
                Ok(None) => continue,
                Err(reason) => return Ok(Err(reason.to_owned())),
            };
            let value = match &start {
                None => at,
                Some((_, before)) => {
                    match value_of(before, &keys, rest) {
                        Ok(Some(before)) => at - before,
                        Ok(None) if i64::try_from(at).is_ok_and(|value| !keep(value)) => continue,
                        Ok(None) => return Ok(Err(TOTALS_WINDOW_UNRECORDED.to_owned())),
                        Err(reason) => return Ok(Err(reason.to_owned())),
                    }
                }
            };
            let Ok(value) = i64::try_from(value) else { return Ok(Err(TOTALS_VALUE_UNKNOWN.to_owned())) };
            if keep(value) {
                scored.push((i128::from(value), tag.as_str()));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        Ok(Ok(scored.into_iter().take(n).map(|(_, tag)| tag.to_owned()).collect()))
    }

    pub fn clock_at(&self, hour: i32) -> io::Result<ClockAt> {
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let Some(stretch) = self.query.timeline.at(hour).copied() else {
            return Ok(ClockAt { read: None, since: None, since_from: None, next_change: None });
        };
        let runs = self.set_runs(stretch.run, &[columns.since, columns.as_of], &[(0, running.clock)], None, hour, hour)?;
        let value = |column: usize| {
            runs.get(&column)
                .and_then(|by| by.get(&running.clock))
                .and_then(|held| held.iter().find(|run| run.0 <= hour && hour <= run.1))
                .and_then(|run| run.2)
                .and_then(|at| i32::try_from(at).ok())
        };
        let recording = self.query.replay.recording();
        let next = |column: usize| crate::cost::storage(|| replay_lanes::lanes::next(recording, stretch.run, self.series.table, column, running.clock, hour));
        let next_change = [next(columns.as_of)?, next(columns.since)?].into_iter().flatten().min().filter(|&at| at <= stretch.last);
        let read = value(columns.as_of).filter(|&read| read <= hour);
        let since_start = read.map_or(stretch.first, |read| read.max(stretch.first));
        let since_from = match since_start < hour {
            true => self
                .set_runs(stretch.run, &[columns.since], &[(0, running.clock)], None, since_start, hour)?
                .get(&columns.since)
                .and_then(|by| by.get(&running.clock))
                .and_then(|held| held.iter().find(|run| run.0 <= hour && hour <= run.1))
                .map(|run| run.0),
            false => None,
        };
        Ok(ClockAt { read, since: value(columns.since), since_from, next_change })
    }

    pub fn since_runs(&self, from: i32, to: i32) -> io::Result<Vec<(i32, i32, Option<i32>)>> {
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let mut out = Vec::new();
        for stretch in self.query.timeline.within(from, to) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            let runs = self.set_runs(stretch.run, &[columns.since], &[(0, running.clock)], None, first, last)?;
            let held = runs.get(&columns.since).and_then(|by| by.get(&running.clock)).into_iter().flatten();
            out.extend(held.map(|run| (run.0.max(first), run.1.min(last), run.2.and_then(|hour| i32::try_from(hour).ok()))));
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

static EMPTY: Runs = Vec::new();

fn push(out: &mut Vec<(i32, i32, Result<i64, Gap>)>, first: i32, last: i32, value: Result<i64, Gap>) {
    match out.last_mut() {
        Some(open) if open.2 == value && open.1.checked_add(1) == Some(first) => open.1 = last,
        _ => out.push((first, last, value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
