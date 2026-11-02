use crate::{
    encoding::{self, Output},
    keyset::{self, Cache, Index},
    manifest::{Selection, invalid},
    memory::cell,
    readset::{Entry, Filter, Rows},
};
use replay::columns;
use replay_columnar::{state::Cell, walk::State};
use std::{fs::File, io, path::Path};

pub struct Gated {
    pending: Index,
    delivered: Index,
    visible: Index,
    dates: Index,
    row_dates: Index,
    file: File,
    previous: Vec<u8>,
    status: Vec<u8>,
    source_status: Option<columns::Status>,
    rescan: bool,
    started: bool,
}

fn date(hour: i32) -> u128 {
    u128::from((hour as u32) ^ (1 << 31))
}

impl Gated {
    pub fn new(path: &Path, spool: &Path, cache: &Cache) -> io::Result<Self> {
        let suffix = |name| {
            path.with_file_name(format!(
                "{}.{name}",
                path.file_name().unwrap().to_string_lossy()
            ))
        };
        Ok(Self {
            pending: Index::new(&suffix("pending"), cache)?,
            delivered: Index::new(&suffix("delivered"), cache)?,
            visible: Index::new(&suffix("visible"), cache)?,
            dates: Index::new(&suffix("dates"), cache)?,
            row_dates: Index::new(&suffix("row-dates"), cache)?,
            file: File::open(spool)?,
            previous: Vec::new(),
            status: Vec::new(),
            source_status: None,
            rescan: true,
            started: false,
        })
    }

    fn unschedule(&mut self, key: u128) -> io::Result<()> {
        if let Some(date) = self.row_dates.get((key, 0))? {
            self.dates.remove((date.into(), key))?;
            self.row_dates.remove((key, 0))?;
        }
        Ok(())
    }

    fn touch(&mut self, state: &State, entry: &Entry, slot: u32) -> io::Result<()> {
        let table = columns::table(state, entry.selected.table)?;
        let Some(key) = table.key(slot).filter(|_| table.live().get(slot)) else {
            return Ok(());
        };
        self.pending.put((key, 0), 1)?;
        if entry.filter == Rows::RelationSlices {
            self.unschedule(key)?;
            if entry.accepts_key(key) {
                let field = columns::field(entry.selected.table, "sample_hour")?;
                let Cell::I32(hour) = cell(table, field, slot)? else {
                    return Err(invalid("Slice date is not i32"));
                };
                self.dates.put((date(hour), key), 1)?;
                self.row_dates.put((key, 0), date(hour) as u64)?;
            }
        }
        Ok(())
    }

    pub fn hour(
        &mut self,
        state: &State,
        selected: &Selection,
        hour: i32,
        first: bool,
        filter: (&Filter, usize),
        record: &mut Vec<u8>,
        output: &mut Output,
    ) -> io::Result<()> {
        let entry = &filter.0.readset.tables[filter.1];
        let changes = columns::changes(state, selected.table)?;
        let now = columns::status(state, selected.table);
        let reset = first || self.source_status != Some(now) || changes.truncated;
        self.source_status = Some(now);
        self.rescan |= reset;
        let table = columns::table(state, selected.table)?;
        if self.started && !self.rescan {
            for &key in &changes.deleted_keys {
                self.pending.put((key, 0), 1)?;
                if entry.filter == Rows::RelationSlices {
                    self.unschedule(key)?;
                }
            }
            for &slot in &changes.inserted {
                self.touch(state, entry, slot)?;
            }
            for (&slot, &mask) in changes
                .changed
                .iter()
                .zip(&changes.written)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            {
                if selected
                    .fields
                    .iter()
                    .any(|field| mask & (1_u64 << field) != 0)
                {
                    self.touch(state, entry, slot)?;
                }
            }
        }
        if !filter.0.active(filter.1) {
            return Ok(());
        }
        // An active-hour marker observes held state; only explicit deltas replace rows.
        encoding::header(record, hour, 8, selected.table as u16, 0);
        output.record(record)?;
        encoding::status(state, selected, hour, record, Some(filter))?;
        if !self.started || self.status[4..] != record[4..] {
            output.record(record)?;
            self.status.clone_from(record);
        }
        if self.rescan {
            self.pending.clear()?;
            self.visible.clear()?;
            self.dates.clear()?;
            self.row_dates.clear()?;
            let mut next = Some((0, 0));
            while let Some(start) = next {
                let Some((key, _)) = self.delivered.first(start)? else {
                    break;
                };
                self.pending.put(key, 1)?;
                next = keyset::after(key);
            }
            if let Some(view) = columns::truncated(state, selected.table) {
                for &slot in view {
                    if let Some(key) = table.key(slot) {
                        self.visible.put((key, 0), 1)?;
                    }
                    self.touch(state, entry, slot)?;
                }
            } else if columns::complete(state, selected.table) {
                for slot in table.live().ones() {
                    self.touch(state, entry, slot)?;
                }
            }
            self.rescan = false;
        }
        if entry.filter == Rows::RelationSlices {
            let mut next = Some((0, 0));
            while let Some(start) = next {
                let Some((key, _)) = self.delivered.first(start)? else {
                    break;
                };
                self.pending.put(key, 1)?;
                next = keyset::after(key);
            }
            let mut next = Some((date(hour), 0));
            while let Some(start) = next {
                let Some((key, _)) = self.dates.first(start)? else {
                    break;
                };
                if key.0 != date(hour) {
                    break;
                }
                self.pending.put((key.1, 0), 1)?;
                next = keyset::after(key);
            }
        }
        let mut next = Some((0, 0));
        while let Some(start) = next {
            let Some((key, _)) = self.pending.first(start)? else {
                break;
            };
            next = keyset::after(key);
            let previous = self.delivered.get(key)?;
            let slot = table.slot(key.0).filter(|&slot| table.live().get(slot));
            let slot = match slot {
                Some(slot)
                    if entry.accepts(state, slot, hour)?
                        && (columns::complete(state, selected.table)
                            || self.visible.get(key)?.is_some()) =>
                {
                    Some(slot)
                }
                _ => None,
            };
            if let Some(slot) = slot {
                encoding::row(
                    state,
                    selected,
                    hour,
                    slot,
                    selected.fields.iter().copied(),
                    1,
                    record,
                )?;
                if let Some(offset) = previous {
                    output.read_record(&mut self.file, offset, &mut self.previous)?;
                    if self.previous[4..] == record[4..] {
                        continue;
                    }
                }
                let offset = output.position();
                output.record(record)?;
                self.delivered.put(key, offset)?;
            } else if previous.is_some() {
                encoding::header(record, hour, 2, selected.table as u16, key.0);
                output.record(record)?;
                self.delivered.remove(key)?;
            }
        }
        self.pending.clear()?;
        self.started = true;
        Ok(())
    }
}
