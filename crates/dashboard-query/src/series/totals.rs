
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
            let value = read.and_then(|read| i32::try_from(read).ok()).map_or(Err(Gap::WindowIncomplete), |read| window(read));
            push(&mut out, lo, hi, value);
            hour = hi + 1;
        }
        if hour <= last {
            push(&mut out, hour, last, Err(Gap::WindowIncomplete));
        }
        Ok(out)
    }

    pub fn totals_ranked(&self, hour: i32, n: usize, parts: Parts<'_>, back: Back<'_>, keep: &dyn Fn(i64) -> bool) -> io::Result<Vec<String>> {
        let running = self.running()?;
        let columns = self.total_columns(running)?;
        let Some(stretch) = self.query.timeline.at(hour).copied() else {
            return Ok(Vec::new());
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
        let Some((since, Some(read))) = clock(hour)? else { return Ok(Vec::new()) };
        if read < stretch.first {
            return Ok(Vec::new());
        }
        let wanted = [columns.value, columns.reason];
        let Some(now) = snapshot(read, &wanted, read::Keys::All)? else { return Ok(Vec::new()) };
        let start = match back {
            None => None,
            Some(back) => {
                let Some(start) = back(read).filter(|&start| start >= stretch.first && since.is_none_or(|since| since <= start)) else { return Ok(Vec::new()) };
                if clock(start)?.and_then(|(_, read)| read) != Some(start) {
                    return Ok(Vec::new());
                }
                match snapshot(start, &wanted, read::Keys::All)? {
                    Some(before) => Some((start, before)),
                    None => return Ok(Vec::new()),
                }
            }
        };
        let set = parts(stretch.run);
        let value_of = |rows: &HashMap<u128, Vec<Option<i64>>>, keys: &[(u128, u128)], rest: Option<u128>| -> Option<i128> {
            let mut sum: i128 = 0;
            for &(_, key) in keys {
                let cells = rows.get(&key).or_else(|| rest.and_then(|rest| rows.get(&rest)))?;
                sum += i128::from(cells[0]?);
            }
            Some(sum)
        };
        let mut scored: Vec<(i128, &str)> = Vec::new();
        for (tag, runs) in &self.query.holders.tags {
            let Some(run) = runs.iter().find(|run| run.first <= hour && hour <= run.last) else { continue };
            let Holder::Found { slot, id } = run.holder else { continue };
            let (keys, rest) = self.set_keys(running, self.series.key.of(slot, id), &set)?;
            let Some(at) = value_of(&now, &keys, rest) else { continue };
            let value = match &start {
                None => at,
                Some((start, before)) => {
                    let same = self.query.holders.at(tag, *start).is_some_and(|held| held.holder == run.holder && held.last >= hour);
                    let Some(before) = value_of(before, &keys, rest).filter(|_| same) else { continue };
                    at - before
                }
            };
            if let Ok(value) = i64::try_from(value)
                && keep(value)
            {
                scored.push((i128::from(value), tag.as_str()));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        Ok(scored.into_iter().take(n).map(|(_, tag)| tag.to_owned()).collect())
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
