use super::{Keys, Selection, TableId, check_cancel};
use replay_columnar::container::Revision;
use replay_lanes::{lanes::{StatusRun, status_reasons}, points::cells};
use replay_recording::columnar_frames::{Recording, Run, Source, TableCoverage};
use std::{collections::{BTreeMap, BTreeSet, btree_map::Entry}, io, sync::{Arc, Weak, atomic::{AtomicBool, Ordering}}};

#[cfg(test)]
mod tests;

const TABLE: TableId = TableId::ScriptVariableValues;
const RETAINED_BYTES: usize = 16 * 1024 * 1024;
const TREE_BYTES_PER_SCOPE: usize = 512;

#[derive(Default)]
pub(super) struct VariableDates {
    held: Option<(Weak<Source>, Revision, u32, Option<Index>)>,
}

struct Index {
    scopes: BTreeMap<u128, Vec<i32>>,
    statuses: Vec<StatusRun>,
    validated_start: Option<(i32, bool)>,
}

impl VariableDates {
    pub(super) fn dates(
        &mut self, recording: &Recording, run: u32, first: i32, last: i32, selection: &Selection, cancelled: &AtomicBool,
    ) -> io::Result<Option<Vec<i32>>> {
        if selection.fields != 1 || selection.ends
            || selection.rows.as_ref().is_some_and(|rows| rows.iter().any(|&(lo, hi)| lo as u32 != 0 || hi as u32 != u32::MAX))
        {
            return Ok(None);
        }
        if recording.columns().get(TABLE.index()) != Some(&1) || recording.read_columns(TABLE.index()) != [0]
            || !matches!(recording.table_coverage(TABLE.index(), run), TableCoverage::Valid)
        {
            self.held = None;
            return Ok(None);
        }
        let Ok(span) = recording.run(run) else { return Ok(None) };
        let Some(start) = span.ordinal(first) else { return Ok(None) };
        if last > span.last_hour() || first > last { return Ok(None); }
        let source = Arc::downgrade(&recording.source());
        let revision = recording.revision();
        if self.held.as_ref().is_none_or(|(s, r, at, _)| !s.ptr_eq(&source) || *r != revision || *at != run) {
            self.held = None;
            self.held = Some((source, revision, run, Index::load(recording, run, cancelled, RETAINED_BYTES)?));
        }
        let Some(index) = self.held.as_mut().and_then(|(_, _, _, index)| index.as_mut()) else { return Ok(None) };
        if start != 0 && !index.validates(recording, run, first) { return Ok(None); }
        Ok(Some(index.dates(first, last, selection)))
    }
}

impl Index {
    fn load(recording: &Recording, run: u32, cancelled: &AtomicBool, budget: usize) -> io::Result<Option<Self>> {
        let Ok(span) = recording.run(run) else { return Ok(None) };
        let (first, last) = (span.first_hour, span.last_hour());
        if !Self::check_restores(recording, run, span, cancelled)? { return Ok(None); }
        let mut scopes = BTreeMap::<u128, Vec<i32>>::new();
        let mut statuses = Vec::new();
        let mut bytes = size_of::<Self>();
        let loaded = dashboard_query::cost::storage(|| {
            cells(recording, run, TABLE, &[0], Keys::All, first, last, Some(&mut statuses), &mut |hour, key, _| {
                if cancelled.load(Ordering::Relaxed) { return false; }
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
        check_cancel(cancelled)?;
        if loaded.is_err() || bytes + statuses.capacity() * size_of::<StatusRun>() > budget
            || status_reasons(recording, run, TABLE, &statuses).is_err()
        {
            return Ok(None);
        }
        Ok(Some(Self { scopes, statuses, validated_start: None }))
    }

    fn check_restores(recording: &Recording, run: u32, span: &Run, cancelled: &AtomicBool) -> io::Result<bool> {
        for block in 0..(span.pages() as u32).saturating_sub(1) {
            check_cancel(cancelled)?;
            let Ok(entry) = recording.table_entry(run, block, TABLE.index()) else { return Ok(false) };
            if entry.unit.restore.is_some() && !check_start(recording, run, span.block_first_hour(block + 1)) {
                return Ok(false);
            }
        }
        Ok(true)
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
