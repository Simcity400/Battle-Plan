use super::{Keys, Replay, Selection, TableId, VariableDates};
use crate::{Source, products::read_ahead::{self, Preparation}};
use replay_columnar::summary::Shape;
use replay_lanes::{lanes::status_reasons, points::cells};
use replay_recording::columnar_frames::TableCoverage;
use std::{collections::BTreeMap, io, sync::{Arc, Mutex, PoisonError}};

#[cfg(test)]
mod tests;

const RETAINED_BYTES: usize = 8 * 1024 * 1024;
const RUN_BYTES: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateSelection { pub table: TableId, pub fields: u64, pub shift: Option<u8> }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum DateWindows { Run, Blocks }

type PreparedKey = (u32, DateWindows, &'static [DateSelection]);
type PrepareRun = Box<dyn FnOnce(&Replay, u32) + Send>;
type PreparedEntry = (Arc<Preparation>, Option<Arc<RunDates>>, Option<PrepareRun>);

#[derive(Default)]
pub(super) struct PreparedDates {
    runs: BTreeMap<PreparedKey, PreparedEntry>,
    bytes: usize,
    building: bool,
}

pub(super) struct RunDates {
    windows: Box<[WindowDates]>,
}

struct WindowDates {
    span: (i32, i32),
    tables: Box<[TableDates]>,
}

struct TableDates {
    table: TableId,
    fields: u64,
    shift: Option<u8>,
    dates: Box<[i32]>,
    changes: Box<[(u128, i32)]>,
}

impl PreparedDates {
    pub(super) fn prepare(source: &Source, run: u32, windows: DateWindows, specs: &'static [DateSelection],
        extra: impl FnOnce(&Replay, u32) + Send + 'static,
    ) -> io::Result<Option<Arc<Preparation>>> {
        let replay = source.replay();
        let variables = source.state::<VariableDates>();
        let variable_build = VariableDates::prepare(&variables, Arc::clone(&replay), run)?;
        let state = source.state::<Self>();
        let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (run, windows, specs);
        if let Some((ready, ..)) = held.runs.get(&key) { return ready.pending(); }
        let reserved = RUN_BYTES;
        if reserved > RETAINED_BYTES - held.bytes { return Ok(variable_build); }
        let ready = Arc::<Preparation>::default();
        if !held.building {
            let state = Arc::clone(&state);
            if read_ahead::spawn("selected proof dates", move || Self::build(&state, &replay, &variables)).is_err() {
                return Ok(variable_build);
            }
            held.building = true;
        }
        held.bytes += reserved;
        held.runs.insert(key, (Arc::clone(&ready), None, Some(Box::new(extra))));
        ready.pending()
    }

    fn build(state: &Mutex<Self>, replay: &Arc<Replay>, variables: &Arc<Mutex<VariableDates>>) {
        job_priority::background_thread();
        loop {
            let ((run, windows, specs), ready, budget, extra) = {
                let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
                let budget = RETAINED_BYTES - held.bytes;
                let Some((&key, (ready, _, extra))) = held.runs.iter_mut().find(|(_, (ready, ..))| ready.result().is_none()) else {
                    held.building = false;
                    return;
                };
                (key, Arc::clone(ready), budget, extra.take())
            };
            ready.run(|| {
                if let Some(extra) = extra { extra(replay, run); }
                let (dates, bytes) = RunDates::load(replay, run, specs, windows, budget)?;
                let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
                if bytes <= RETAINED_BYTES - held.bytes {
                    held.bytes += bytes;
                    held.runs.get_mut(&(run, windows, specs)).expect("admitted date selection").1 = Some(Arc::new(dates));
                }
                drop(held);
                if let Some(variable_build) = VariableDates::prepare(variables, Arc::clone(replay), run)? {
                    variable_build.wait()?;
                }
                Ok(())
            });
        }
    }

    pub(super) fn get(&self, run: u32, windows: DateWindows, specs: &'static [DateSelection]) -> Option<Arc<RunDates>> {
        self.runs.get(&(run, windows, specs))?.1.clone()
    }
}

impl DateSelection {
    fn load(&self, replay: &Replay, run: u32, (first, last): (i32, i32), budget: usize) -> Option<TableDates> {
        let recording = replay.recording();
        let index = self.table.index();
        let columns: Vec<_> = (0..64).filter(|&column| self.fields & (1 << column) != 0).collect();
        let available = recording.read_columns(index);
        if self.shift.is_some_and(|shift| shift >= 128)
            || !matches!(recording.table_coverage(index, run), TableCoverage::Valid)
            || columns.iter().any(|column| !available.contains(column))
            || self.shift.is_some() && dashboard_declare::summaries().ok()?.iter()
                .any(|(table, shape)| *table == index && !matches!(shape, Shape::Log(_)))
        {
            return None;
        }
        if self.shift.is_some() {
            let span = recording.run(run).ok()?;
            for block in 0..(span.pages() as u32).saturating_sub(1) {
                if recording.table_entry(run, block, index).ok()?.unit.restore.is_some() {
                    let hour = span.block_first_hour(block + 1);
                    cells(recording, run, self.table, &columns, Keys::These(&[]), hour, hour, None,
                        &mut |_, _, _| true).ok()?;
                }
            }
        }
        let mut statuses = Vec::new();
        let mut changes = Vec::new();
        let mut overflow = false;
        let limit = budget / size_of::<(u128, i32)>();
        cells(recording, run, self.table, &columns, Keys::All, first, last, Some(&mut statuses), &mut |hour, key, _| {
            let group = self.shift.map_or(0, |shift| key >> shift);
            if hour != first && changes.last() != Some(&(group, hour)) {
                if changes.len() == limit {
                    overflow = true;
                    return false;
                }
                changes.push((group, hour));
            }
            true
        }).ok()?;
        if overflow { return None; }
        status_reasons(recording, run, self.table, &statuses).ok()?;
        changes.sort_unstable();
        changes.dedup();
        let dates = TableDates {
            table: self.table, fields: self.fields, shift: self.shift,
            dates: statuses.into_iter().map(|status| status.first).collect(),
            changes: changes.into_boxed_slice(),
        };
        (dates.bytes() <= budget).then_some(dates)
    }

    fn load_block(&self, replay: &Replay, run: u32, (first, last): (i32, i32), budget: usize) -> Option<TableDates> {
        if self.shift.is_some() { return None; }
        let selection = Selection { fields: self.fields, rows: None, ends: false };
        let dates = selection.dates(replay, run, self.table, first, last).ok()?.into_boxed_slice();
        let table = TableDates { table: self.table, fields: self.fields, shift: None, dates, changes: Box::default() };
        (table.bytes() <= budget).then_some(table)
    }
}

impl RunDates {
    fn load(replay: &Replay, run: u32, specs: &[DateSelection], mode: DateWindows, mut budget: usize) -> io::Result<(Self, usize)> {
        let info = replay.recording().run(run)?;
        let available = budget;
        let mut windows = Vec::new();
        let whole = (mode == DateWindows::Run).then_some((info.first_hour, info.last_hour()));
        let blocks = replay.index.iter().filter(|block| mode == DateWindows::Blocks && block.run == run)
            .map(|block| (block.first_hour, block.last_hour));
        for span in whole.into_iter().chain(blocks) {
            let headers = size_of::<WindowDates>().saturating_add(specs.len().saturating_mul(size_of::<TableDates>()));
            let Some(after_headers) = budget.checked_sub(headers) else { break };
            budget = after_headers;
            let mut tables = Vec::with_capacity(specs.len());
            for spec in specs {
                let table = match mode {
                    DateWindows::Run => spec.load(replay, run, span, budget),
                    DateWindows::Blocks => spec.load_block(replay, run, span, budget),
                };
                if let Some(table) = table {
                    budget -= table.bytes();
                    tables.push(table);
                }
            }
            budget += (specs.len() - tables.len()) * size_of::<TableDates>();
            windows.push(WindowDates { span, tables: tables.into_boxed_slice() });
        }
        windows.sort_unstable_by_key(|window| window.span);
        Ok((Self { windows: windows.into_boxed_slice() }, available - budget))
    }

    pub(super) fn next(&self, table: TableId, selection: &Selection, span: (i32, i32), hour: i32) -> Option<Option<i32>> {
        let index = self.windows.binary_search_by_key(&span, |window| window.span).ok()?;
        self.windows[index].tables.iter().find(|dates| dates.table == table)?.next(selection, hour)
    }
}

impl TableDates {
    fn bytes(&self) -> usize { size_of_val(&*self.dates) + size_of_val(&*self.changes) }

    fn next(&self, selection: &Selection, hour: i32) -> Option<Option<i32>> {
        if selection.fields != self.fields || selection.ends { return None; }
        let changes = match selection.rows.as_deref() {
            None => self.changes.iter().filter_map(|&(_, date)| (date > hour).then_some(date)).min(),
            Some(ranges) => {
                let shift = self.shift?;
                let mask = (1_u128 << shift) - 1;
                if ranges.iter().any(|&(lo, hi)| lo & mask != 0 || hi & mask != mask) { return None; }
                ranges.iter().filter_map(|&(lo, hi)| {
                    let start = self.changes.partition_point(|&(key, _)| key < lo >> shift);
                    let end = self.changes.partition_point(|&(key, _)| key <= hi >> shift);
                    self.changes[start..end].iter().filter_map(|&(_, date)| (date > hour).then_some(date)).min()
                }).min()
            }
        };
        let date = self.dates.get(self.dates.partition_point(|&date| date <= hour)).copied();
        Some(changes.into_iter().chain(date).min())
    }
}
