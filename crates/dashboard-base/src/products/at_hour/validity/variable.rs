use super::{Keys, Selection, TableId};
use crate::products::read_ahead::{self, Preparation};
use replay::campaign::Replay;
use replay_lanes::{lanes::{StatusRun, status_reasons}, points::cells};
use replay_recording::columnar_frames::{Recording, Run, TableCoverage};
use std::{collections::{BTreeMap, BTreeSet, btree_map::Entry}, io, sync::{Arc, Mutex, PoisonError}};

#[cfg(test)]
mod tests;

const TABLE: TableId = TableId::ScriptVariableValues;
const RETAINED_BYTES: usize = 16 * 1024 * 1024;
const TREE_BYTES_PER_SCOPE: usize = 512;
const BYTES_PER_RUN: usize = 512;

#[derive(Default)]
pub(super) struct VariableDates {
    runs: BTreeMap<u32, (Arc<Preparation>, Option<Index>)>,
    bytes: usize,
    building: bool,
}

struct Index {
    scopes: BTreeMap<u128, Vec<i32>>,
    statuses: Vec<StatusRun>,
    validated_start: Option<(i32, bool)>,
    bytes: usize,
    names: Option<Box<[i32]>>,
}

impl VariableDates {
    pub(super) fn prepare(state: &Arc<Mutex<Self>>, replay: Arc<Replay>, run: u32) -> io::Result<Option<Arc<Preparation>>> {
        let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((ready, _)) = held.runs.get(&run) { return ready.pending(); }
        if held.bytes + BYTES_PER_RUN > RETAINED_BYTES || !indexable(replay.recording(), run) { return Ok(None); }
        let ready = Arc::<Preparation>::default();
        held.bytes += BYTES_PER_RUN;
        held.runs.insert(run, (Arc::clone(&ready), None));
        if !held.building {
            let state = Arc::clone(state);
            held.building = read_ahead::spawn("script variable dates", move || Self::build(&state, &replay)).is_ok();
            if !held.building { ready.publish(Ok(())); }
        }
        ready.pending()
    }

    fn build(state: &Mutex<Self>, replay: &Replay) {
        job_priority::background_thread();
        loop {
            let (run, ready, budget) = {
                let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
                let Some((&run, (ready, _))) = held.runs.iter().find(|(_, (ready, _))| ready.result().is_none()) else {
                    held.building = false;
                    return;
                };
                (run, Arc::clone(ready), RETAINED_BYTES - held.bytes)
            };
            ready.run(|| {
                if let Some(index) = Index::load(replay.recording(), run, budget) {
                    let span = replay.recording().run(run)?;
                    let selection = Selection { fields: 1, ..Selection::default() };
                    let names = selection.dates(replay, run, TableId::ScriptVariableNames, span.first_hour, span.last_hour())
                        .ok().map(Vec::into_boxed_slice);
                    state.lock().unwrap_or_else(PoisonError::into_inner).publish(run, index, names);
                }
                Ok(())
            });
        }
    }

    fn publish(&mut self, run: u32, mut index: Index, names: Option<Box<[i32]>>) {
        if index.bytes > RETAINED_BYTES - self.bytes { return; }
        self.bytes += index.bytes;
        index.names = names.filter(|names| size_of_val(&**names) <= RETAINED_BYTES - self.bytes);
        self.bytes += index.names.as_deref().map_or(0, size_of_val);
        self.runs.get_mut(&run).expect("admitted run").1 = Some(index);
    }

    pub(super) fn dates(
        &mut self, recording: &Recording, run: u32, table: TableId, first: i32, last: i32, selection: &Selection,
    ) -> Option<Vec<i32>> {
        if selection.fields != 1 || selection.ends
            || selection.rows.as_ref().is_some_and(|rows| rows.iter().any(|&(lo, hi)| lo as u32 != 0 || hi as u32 != u32::MAX))
        {
            return None;
        }
        if !indexable(recording, run) { return None; }
        let span = recording.run(run).ok()?;
        let start = span.ordinal(first)?;
        if last > span.last_hour() || first > last { return None; }
        let (_, index) = self.runs.get_mut(&run)?;
        let index = index.as_mut()?;
        if table == TableId::ScriptVariableNames {
            let names = index.names.as_deref()?;
            return (selection.rows.is_none() && start == 0 && last == span.last_hour()).then(|| names.to_vec());
        }
        if start != 0 && !index.validates(recording, run, first) { return None; }
        Some(index.dates(first, last, selection))
    }
}

fn indexable(recording: &Recording, run: u32) -> bool {
    recording.columns().get(TABLE.index()) == Some(&1) && recording.read_columns(TABLE.index()) == [0]
        && matches!(recording.table_coverage(TABLE.index(), run), TableCoverage::Valid)
}

impl Index {
    fn load(recording: &Recording, run: u32, budget: usize) -> Option<Self> {
        if budget < size_of::<Self>() { return None; }
        let span = recording.run(run).ok()?;
        let (first, last) = (span.first_hour, span.last_hour());
        if !Self::check_restores(recording, run, span) { return None; }
        let mut scopes = BTreeMap::<u128, Vec<i32>>::new();
        let mut statuses = Vec::new();
        let mut bytes = size_of::<Self>();
        let loaded = dashboard_query::cost::storage(|| {
            cells(recording, run, TABLE, &[0], Keys::All, first, last, Some(&mut statuses), &mut |hour, key, _| {
                if hour == first { return true; }
                let dates = match scopes.entry(key >> 32) {
                    Entry::Vacant(entry) => {
                        bytes += TREE_BYTES_PER_SCOPE;
                        entry.insert(Vec::new())
                    }
                    Entry::Occupied(entry) => entry.into_mut(),
                };
                if dates.last() != Some(&hour) {
                    let before = dates.capacity();
                    dates.push(hour);
                    bytes += (dates.capacity() - before) * size_of::<i32>();
                }
                bytes <= budget
            })
        });
        bytes += statuses.capacity() * size_of::<StatusRun>();
        if loaded.is_err() || bytes > budget
            || status_reasons(recording, run, TABLE, &statuses).is_err()
        {
            return None;
        }
        Some(Self { scopes, statuses, validated_start: None, bytes, names: None })
    }

    fn check_restores(recording: &Recording, run: u32, span: &Run) -> bool {
        for block in 0..(span.pages() as u32).saturating_sub(1) {
            let Ok(entry) = recording.table_entry(run, block, TABLE.index()) else { return false };
            if entry.unit.restore.is_some() && !check_start(recording, run, span.block_first_hour(block + 1)) {
                return false;
            }
        }
        true
    }

    fn validates(&mut self, recording: &Recording, run: u32, start: i32) -> bool {
        if let Some((at, valid)) = self.validated_start
            && at == start
        {
            return valid;
        }
        let valid = check_start(recording, run, start);
        self.validated_start = Some((start, valid));
        valid
    }

    fn dates(&self, first: i32, last: i32, selection: &Selection) -> Vec<i32> {
        let mut dates: BTreeSet<_> = self.statuses.iter().map(|status| status.first)
            .filter(|&hour| first < hour && hour <= last).collect();
        let mut include = |scope: &Vec<i32>| {
            let begin = scope.partition_point(|&hour| hour <= first);
            let end = scope.partition_point(|&hour| hour <= last);
            dates.extend(&scope[begin..end]);
        };
        match &selection.rows {
            Some(ranges) => {
                for &(lo, hi) in ranges {
                    self.scopes.range((lo >> 32)..=(hi >> 32)).for_each(|(_, scope)| include(scope));
                }
            }
            None => self.scopes.values().for_each(include),
        }
        dates.into_iter().collect()
    }
}

fn check_start(recording: &Recording, run: u32, hour: i32) -> bool {
    dashboard_query::cost::storage(|| {
        cells(recording, run, TABLE, &[0], Keys::These(&[]), hour, hour, None, &mut |_, _, _| true).is_ok()
    })
}
