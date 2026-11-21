
use crate::{
    invalid,
    manifest::{Manifest, Status},
    section::{self, Changes, Scratch},
    state::{Kind, Table},
};
use std::{fmt, io};

#[cfg(test)]
mod tests;

#[derive(Default, PartialEq, Eq)]
pub struct Dictionary {
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dictionary").field("len", &self.len()).finish_non_exhaustive()
    }
}

impl Dictionary {
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    pub fn get(&self, id: u32) -> Option<&[u8]> {
        let at = id as usize;
        let end = *self.ends.get(at)?;
        let start = at.checked_sub(1).map_or(0, |before| self.ends[before]);
        Some(&self.bytes[start..end])
    }

    pub fn append(&mut self, text: &[u8]) -> io::Result<()> {
        if u32::try_from(self.ends.len()).is_err() {
            return Err(invalid("A run's strings pass 2^32"));
        }
        engine_allocation::reserve(&mut self.bytes, text.len())?;
        engine_allocation::reserve(&mut self.ends, 1)?;
        self.bytes.extend_from_slice(text);
        self.ends.push(self.bytes.len());
        Ok(())
    }
}

fn view_read(view: &mut Vec<u32>, changes: &Changes, table: &Table) -> io::Result<()> {
    let live = table.live();
    view.clear();
    engine_allocation::reserve(view, changes.read.len() + changes.inserted.len())?;
    view.extend(changes.read.iter().copied().filter(|&slot| live.get(slot)));
    view.extend_from_slice(&changes.inserted);
    view.sort_unstable();
    view.dedup();
    Ok(())
}

fn clear_hour(changes: &mut Changes) {
    changes.truncated = false;
    changes.read.clear();
    changes.changed.clear();
    changes.deleted.clear();
    changes.inserted.clear();
    changes.touched.clear();
    changes.filled.clear();
    changes.fill_slots.clear();
    changes.masks.clear();
    changes.written.clear();
    changes.deleted_keys.clear();
    changes.fill_masks.clear();
}

pub struct State {
    kinds: Vec<Vec<Kind>>,
    columns: Vec<usize>,
    tables: Vec<Table>,
    changes: Vec<Changes>,
    statuses: Vec<Status>,
    sampled: Vec<Option<i32>>,
    views: Vec<Vec<u32>>,
    present: Vec<usize>,
    dictionary: Dictionary,
    scratch: Scratch,
}

impl State {
    pub fn new(schema: Vec<Vec<Kind>>) -> io::Result<Self> {
        let columns = engine_allocation::collect_vec(schema.iter().map(Vec::len))?;
        Self::recorded(schema, columns)
    }

    pub fn recorded(schema: Vec<Vec<Kind>>, columns: Vec<usize>) -> io::Result<Self> {
        if columns.len() != schema.len() || columns.iter().zip(&schema).any(|(&held, kinds)| held > kinds.len()) {
            return Err(invalid("A journal's columns pass the schema's"));
        }
        let count = schema.len();
        let mut tables = engine_allocation::vec(count)?;
        for kinds in &schema {
            tables.push(Table::new(kinds)?);
        }
        Ok(Self {
            columns,
            tables,
            changes: engine_allocation::collect_vec((0..count).map(|_| Changes::default()))?,
            statuses: engine_allocation::filled(count, Status::NotRecorded)?,
            sampled: engine_allocation::filled(count, None)?,
            views: engine_allocation::collect_vec((0..count).map(|_| Vec::new()))?,
            kinds: schema,
            present: Vec::new(),
            dictionary: Dictionary::default(),
            scratch: Scratch::default(),
        })
    }

    pub fn table(&self, table: usize) -> Option<&Table> {
        self.tables.get(table)
    }

    pub fn changes(&self, table: usize) -> Option<&Changes> {
        self.changes.get(table)
    }

    pub fn status(&self, table: usize) -> Option<Status> {
        self.statuses.get(table).copied()
    }

    pub fn sampled_at(&self, table: usize) -> Option<i32> {
        self.sampled.get(table).copied().flatten()
    }

    pub fn truncated_view(&self, table: usize) -> Option<&[u32]> {
        matches!(self.statuses.get(table)?, Status::Truncated { .. }).then(|| self.views[table].as_slice())
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }

    pub fn apply<'s>(&mut self, manifest: &Manifest<'_>, mut section: impl FnMut(usize) -> io::Result<&'s [u8]>) -> io::Result<()> {
        self.begin_hour(manifest)?;
        for (index, entry) in manifest.sections.iter().enumerate() {
            let table = entry.table;
            let bytes = section(index)?;
            if bytes.len() as u64 != entry.bytes {
                return Err(invalid("Section length differs from its manifest's"));
            }
            let truncated = matches!(self.statuses[table], Status::Truncated { .. });
            let recorded = self.columns[table];
            section::apply(bytes, &self.kinds[table][..recorded], truncated, &mut self.tables[table], &mut self.scratch, &mut self.changes[table])?;
            for column in recorded..self.kinds[table].len() {
                self.tables[table].unrecord(column, &self.changes[table].inserted)?;
            }
            if truncated {
                view_read(&mut self.views[table], &self.changes[table], &self.tables[table])?;
            }
            engine_allocation::reserve(&mut self.present, 1)?;
            self.present.push(table);
        }
        Ok(())
    }

    fn begin_hour(&mut self, manifest: &Manifest<'_>) -> io::Result<()> {
        let count = self.tables.len();
        if manifest.statuses.len() != count || manifest.read.len() != count.div_ceil(64) {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        for &table in &self.present {
            clear_hour(&mut self.changes[table]);
        }
        self.present.clear();
        for append in &manifest.appends {
            self.dictionary.append(append)?;
        }
        for table in 0..count {
            self.statuses[table] = manifest.statuses[table];
            if manifest.read[table / 64] >> (table % 64) & 1 != 0 {
                self.sampled[table] = Some(manifest.hour);
            }
        }
        for &(table, hour) in &manifest.samples {
            *self.sampled.get_mut(table).ok_or_else(|| invalid("A sample names a table past the schema"))? = hour;
        }
        let mut sections = manifest.sections.iter().peekable();
        for table in 0..count {
            while sections.next_if(|entry| entry.table < table).is_some() {}
            let truncated = matches!(self.statuses[table], Status::Truncated { .. });
            let present = sections.next_if(|entry| entry.table == table).is_some();
            if truncated && !present {
                return Err(invalid("A partly read table names no rows it read"));
            }
            if !truncated {
                self.views[table].clear();
            }
        }
        Ok(())
    }
}
