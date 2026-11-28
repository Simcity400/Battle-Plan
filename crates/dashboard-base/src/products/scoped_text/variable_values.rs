use super::{VARIABLES, complete_values};
use replay_columnar::{manifest::Status, store::Value};
use replay_recording::{cells::TableReader, columnar_frames::{Recording, TableCoverage}};
use std::collections::BTreeMap;

type Sample = (u32, i32, Option<i64>);

pub(super) struct ScopeValues {
    run: u32,
    first: i32,
    last: i32,
    scope: u128,
    samples: Box<[Sample]>,
}

impl ScopeValues {
    pub(super) fn load(recording: &Recording, run: u32, scope: u128, budget: usize) -> Option<Self> {
        let limit = budget.checked_sub(size_of::<Self>())? / size_of::<Sample>();
        if recording.columns().get(VARIABLES.index()) != Some(&VARIABLES.schema().len())
            || recording.read_columns(VARIABLES.index()) != [0]
            || !matches!(recording.table_coverage(VARIABLES.index(), run), TableCoverage::Valid)
        {
            return None;
        }
        let span = recording.run(run).ok()?;
        let mut reader = open(recording, run, 0)?;
        let mut samples = Vec::new();
        for ordinal in 0..span.ordinals {
            if ordinal != 0 {
                if !reader.step(recording).ok()? { return None; }
                let (block, offset) = span.place(ordinal);
                if offset == 0 && recording.table_entry(run, block - 1, VARIABLES.index()).ok()?.unit.restore.is_some() {
                    let restored = open(recording, run, ordinal)?;
                    if !same_cells(&reader, &restored) { return None; }
                }
            }
            let cells = reader.cells();
            if reader.status() != Status::Complete || !complete_values(cells) { return None; }
            let mut at = BTreeMap::new();
            let changes = reader.changes();
            for &(_, key) in &changes.deleted {
                if key >> 32 == scope { at.insert(key as u32, None); }
            }
            let mut changed = |slot| {
                if let Some(key) = cells.key(slot).filter(|&key| key >> 32 == scope) {
                    let Value::Number(value) = cells.value(0, slot) else { return None };
                    at.insert(key as u32, Some(value as i64));
                }
                Some(())
            };
            if ordinal == 0 {
                cells.live().ones().try_for_each(changed)?;
            } else {
                changes.inserted.iter().chain(changes.written.iter().flatten()).copied().try_for_each(&mut changed)?;
            }
            if limit - samples.len() < at.len() {
                return None;
            }
            let hour = reader.hour()?;
            samples.extend(at.into_iter().map(|(key, value)| (key, hour, value)));
        }
        samples.sort_unstable_by_key(|&(key, hour, _)| (key, hour));
        Some(Self { run, first: span.first_hour, last: span.last_hour(), scope, samples: samples.into_boxed_slice() })
    }

    pub(super) fn bytes(&self) -> usize {
        size_of::<Self>() + size_of_val(&*self.samples)
    }

    pub(super) fn read(&self, run: u32, hour: i32, key: Option<u128>) -> Option<Option<i64>> {
        if run != self.run || !(self.first..=self.last).contains(&hour) { return None; }
        let Some(key) = key else { return Some(None) };
        if key >> 32 != self.scope { return None; }
        let key = key as u32;
        let end = self.samples.partition_point(|&(held, at, _)| (held, at) <= (key, hour));
        Some(end.checked_sub(1).and_then(|at| {
            let (held, _, value) = self.samples[at];
            (held == key).then_some(value).flatten()
        }))
    }
}

fn open(recording: &Recording, run: u32, ordinal: u32) -> Option<TableReader> {
    let mut reader = TableReader::new(recording, run, VARIABLES, None).ok()?;
    reader.index_keys();
    reader.seek(recording, ordinal).ok()?;
    (reader.columns() == [0]).then_some(reader)
}

fn same_cells(streamed: &TableReader, restored: &TableReader) -> bool {
    let (a, b) = (streamed.cells(), restored.cells());
    streamed.status() == restored.status() && a.slots() == b.slots()
        && (0..a.slots()).all(|slot| {
            let key = a.key(slot);
            key == b.key(slot) && (key.is_none() || a.value(0, slot) == b.value(0, slot))
        })
}
