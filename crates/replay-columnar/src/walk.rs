//! A recording's current state while it is walked: every schema table, what each changed
//! in the last hour applied, and the string dictionary. Where an hour's bytes live is the
//! framing's business: an hour applies from its manifest and its sections' bytes.

use crate::{
    invalid,
    leaf::{Reader, Writer},
    manifest::{Manifest, Status},
    restart::{self, Acquisition},
    section::{self, Changes, Scratch},
    state::{Kind, Table},
};
use std::io;

#[cfg(test)]
mod tests;

/// The recording's strings by id, appended in order and never changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictionary {
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl Dictionary {
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    pub fn get(&self, id: u32) -> Option<&[u8]> {
        let id = id as usize;
        let end = *self.ends.get(id)?;
        let start = id.checked_sub(1).map_or(0, |previous| self.ends[previous]);
        Some(&self.bytes[start..end])
    }

    pub fn append(&mut self, text: &[u8]) -> io::Result<()> {
        engine_allocation::reserve(&mut self.bytes, text.len())?;
        engine_allocation::reserve(&mut self.ends, 1)?;
        self.bytes.extend_from_slice(text);
        self.ends.push(self.bytes.len());
        Ok(())
    }

    /// Strings `first..` as one chunk: a count, then each string length-prefixed, as a
    /// manifest carries its appends.
    pub fn chunk(&self, first: usize) -> io::Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.varint(self.len().saturating_sub(first) as u64)?;
        for id in first..self.len() {
            let text = self.get(id as u32).expect("id below len");
            writer.varint(text.len() as u64)?;
            writer.raw(text)?;
        }
        Ok(writer.into_bytes())
    }

    /// Appends a chunk's strings, in order.
    pub fn read_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(bytes);
        let count = reader.count()?;
        for _ in 0..count {
            let length = reader.count()?;
            self.append(reader.raw(length)?)?;
        }
        reader.finish()
    }
}

/// Every schema table while a branch is walked.
pub struct State {
    kinds: Vec<Vec<Kind>>,
    columns: Vec<usize>,
    tables: Vec<Table>,
    changes: Vec<Changes>,
    statuses: Vec<Status>,
    /// Each table's sample hour.
    sampled: Vec<Option<i32>>,
    /// Each truncated table's rows read: its last hour's read and inserted slots.
    views: Vec<Vec<u32>>,
    /// Tables whose changes the last hour set, to clear before the next.
    present: Vec<usize>,
    /// Tables whose sections are not applied: no reader of the walk reads them.
    skipped: Vec<bool>,
    dictionary: Dictionary,
    scratch: Scratch,
}

impl State {
    /// `schema` holds each table's column kinds, in the recording's table order.
    pub fn new(schema: Vec<Vec<Kind>>) -> io::Result<Self> {
        let count = schema.len();
        let mut tables = engine_allocation::vec(count)?;
        for kinds in &schema {
            tables.push(Table::new(kinds)?);
        }
        Ok(Self {
            columns: engine_allocation::collect_vec(schema.iter().map(Vec::len))?,
            tables,
            changes: engine_allocation::collect_vec((0..count).map(|_| Changes::default()))?,
            statuses: engine_allocation::filled(count, Status::NotRecorded)?,
            sampled: engine_allocation::filled(count, None)?,
            views: engine_allocation::collect_vec((0..count).map(|_| Vec::new()))?,
            skipped: engine_allocation::filled(count, false)?,
            kinds: schema,
            present: Vec::new(),
            dictionary: Dictionary::default(),
            scratch: Scratch::default(),
        })
    }

    /// Each table's column count, as a manifest is read with.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    /// Leaves the sections of the tables `skipped` marks unapplied from the next hour on:
    /// they keep their rows and record no changes. Their statuses still follow each hour.
    pub fn skip(&mut self, skipped: &[bool]) -> io::Result<()> {
        if skipped.len() != self.skipped.len() {
            return Err(invalid("A selection disagrees with the schema's tables"));
        }
        self.skipped.copy_from_slice(skipped);
        Ok(())
    }

    /// Whether `table`'s sections are left unapplied.
    pub fn skips(&self, table: usize) -> bool {
        self.skipped.get(table).copied().unwrap_or(false)
    }

    pub fn table(&self, table: usize) -> Option<&Table> {
        self.tables.get(table)
    }

    /// What `table` changed in the last hour applied; empty when it had no section.
    pub fn changes(&self, table: usize) -> Option<&Changes> {
        self.changes.get(table)
    }

    pub fn status(&self, table: usize) -> Option<Status> {
        self.statuses.get(table).copied()
    }

    /// The hour of the acquisition `table`'s view holds; none when that is not known.
    pub fn sampled_at(&self, table: usize) -> Option<i32> {
        self.sampled.get(table).copied().flatten()
    }

    /// The slots of the rows a truncated `table` read, ascending; None while it is not
    /// truncated. Its other live slots hold rows from earlier reads.
    pub fn truncated_view(&self, table: usize) -> Option<&[u32]> {
        matches!(self.statuses.get(table)?, Status::Truncated { .. })
            .then(|| self.views[table].as_slice())
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }

    /// Replaces `table` with a restart's.
    pub fn restore(&mut self, table: usize, bytes: &[u8]) -> io::Result<Acquisition> {
        let kinds = self
            .kinds
            .get(table)
            .ok_or_else(|| invalid("Restart names a table past the schema"))?;
        let restored = restart::read(bytes, kinds, &mut self.scratch)?;
        self.tables[table] = restored.table;
        self.statuses[table] = restored.status;
        self.sampled[table] = restored.acquisition.last_read;
        self.views[table] = restored.view;
        self.changes[table] = Changes::default();
        Ok(restored.acquisition)
    }

    /// Replaces the dictionary with a restart's chunks, in order.
    pub fn restore_dictionary<'b>(
        &mut self,
        chunks: impl IntoIterator<Item = &'b [u8]>,
    ) -> io::Result<()> {
        self.dictionary = Dictionary::default();
        for chunk in chunks {
            self.dictionary.read_chunk(chunk)?;
        }
        Ok(())
    }

    /// Applies one hour: its appends, statuses and every present section of a table it does
    /// not skip, whose bytes `section(index)` gives for the manifest's `index`th section.
    pub fn apply<'s>(
        &mut self,
        manifest: &Manifest<'_>,
        mut section: impl FnMut(usize) -> io::Result<&'s [u8]>,
    ) -> io::Result<()> {
        if manifest.statuses.len() != self.tables.len() {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        for &table in &self.present {
            let changes = &mut self.changes[table];
            changes.truncated = false;
            changes.read.clear();
            changes.changed.clear();
            changes.deleted.clear();
            changes.inserted.clear();
            changes.touched.clear();
            changes.filled.clear();
            changes.fill_slots.clear();
            changes.masks.clear();
            changes.deleted_keys.clear();
            changes.fill_masks.clear();
        }
        self.present.clear();
        for append in &manifest.appends {
            self.dictionary.append(append)?;
        }
        if manifest.read.len() != self.tables.len().div_ceil(64) {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        self.statuses.copy_from_slice(&manifest.statuses);
        for (table, sampled) in self.sampled.iter_mut().enumerate() {
            if manifest.read[table / 64] >> (table % 64) & 1 != 0 {
                *sampled = Some(manifest.hour);
            }
        }
        for &(table, hour) in &manifest.samples {
            *self
                .sampled
                .get_mut(table)
                .ok_or_else(|| invalid("A sample names a table past the schema"))? = hour;
        }
        let mut sections = manifest.sections.iter().peekable();
        for (table, view) in self.views.iter_mut().enumerate() {
            let truncated = matches!(self.statuses[table], Status::Truncated { .. });
            let present = sections.next_if(|entry| entry.table == table).is_some();
            if truncated && !present {
                return Err(invalid("A partly read table names no rows it read"));
            }
            if !truncated {
                view.clear();
            }
        }
        for (index, entry) in manifest.sections.iter().enumerate() {
            let table = entry.table;
            if self.skipped[table] {
                continue;
            }
            let bytes = section(index)?;
            if bytes.len() as u64 != entry.bytes {
                return Err(invalid("Section length differs from its manifest's"));
            }
            let truncated = matches!(self.statuses[table], Status::Truncated { .. });
            section::apply(
                bytes,
                &self.kinds[table],
                truncated,
                &mut self.tables[table],
                &mut self.scratch,
                &mut self.changes[table],
            )?;
            if truncated {
                let changes = &self.changes[table];
                let live = self.tables[table].live();
                let view = &mut self.views[table];
                view.clear();
                engine_allocation::reserve(view, changes.read.len() + changes.inserted.len())?;
                // A read row the hour deleted is gone; a slot it deleted and filled again holds
                // the inserted row, once.
                view.extend(changes.read.iter().copied().filter(|&slot| live.get(slot)));
                view.extend_from_slice(&changes.inserted);
                view.sort_unstable();
                view.dedup();
            }
            engine_allocation::reserve(&mut self.present, 1)?;
            self.present.push(table);
        }
        Ok(())
    }
}
