//! Hours written as the collector writes its journal, a table at a time, for tests: each hour's
//! manifest and sections, which the recorder's writer takes ([`super::fixtures`]).

use super::schema;
use crate::{
    native_columns::{RowCells, TableInput, table_hour},
    rows::{RowCodec, TABLE_COUNT, TableId},
};
use replay_columnar::{
    manifest::{self, Manifest, Section, Status},
    writer::{Dictionary as Strings, TableWriter},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    io,
};

pub use crate::native_columns::TableInput as Input;
pub use replay_columnar::writer::Sample;

/// Synthetic hours: write each table's input with [`Self::table`], then end the hour with
/// [`Self::end_encoded`].
pub struct Synthetic {
    writers: HashMap<TableId, TableWriter>,
    strings: Strings,
    cells: RowCells,
    columns: Vec<usize>,
    branch: u64,
    hour: i32,
    written: [bool; TABLE_COUNT],
    statuses: Vec<Status>,
    read: Vec<u64>,
    samples: Vec<(usize, Option<i32>)>,
    sections: Vec<(Section, Vec<u8>)>,
}

impl Synthetic {
    /// Hours of branch `branch` whose first hour is `hour`.
    pub fn new(branch: u64, hour: i32) -> io::Result<Self> {
        Ok(Self {
            columns: schema()?.iter().map(Vec::len).collect(),
            writers: HashMap::new(),
            strings: Strings::new(),
            cells: RowCells::default(),
            branch,
            hour,
            written: [false; TABLE_COUNT],
            statuses: vec![Status::NotRecorded; TABLE_COUNT],
            read: vec![0; TABLE_COUNT.div_ceil(64)],
            samples: Vec::new(),
            sections: Vec::new(),
        })
    }

    /// Writes `table`'s input for the hour under way, as the collector writes it.
    pub fn table<R: RowCodec>(&mut self, table: TableId, input: TableInput<'_, R>) -> io::Result<()> {
        let index = table.index();
        if self.written[index] {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{table:?} was already written this hour")));
        }
        let writer = match self.writers.entry(table) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(TableWriter::new(&schema()?[table.index()])?),
        };
        let written = table_hour(writer, &mut self.strings, &mut self.cells, input)?;
        self.statuses[index] = written.evidence.status;
        if written.evidence.read {
            self.read[index / 64] |= 1 << (index % 64);
        }
        match written.evidence.sample {
            Sample::FromReadBit => {}
            Sample::Reset => self.samples.push((index, None)),
            Sample::Carried(hour) => self.samples.push((index, Some(hour))),
        }
        if let Some(prepared) = &written.prepared
            && let Some(bytes) = &prepared.section
        {
            self.sections.push((Section { table: index, bytes: bytes.len() as u64, columns: prepared.columns.clone(), ops: prepared.ops }, bytes.clone()));
        }
        writer.commit(written.prepared, written.evidence, self.hour)?;
        self.written[index] = true;
        Ok(())
    }

    /// Ends the hour under way: `visit` sees its manifest's bytes and its sections, joined.
    pub(crate) fn end_encoded<T>(&mut self, visit: impl FnOnce(&[u8], &[u8]) -> T) -> io::Result<T> {
        self.sections.sort_by_key(|(section, _)| section.table);
        self.samples.sort_unstable_by_key(|&(table, _)| table);
        let appends = self.strings.take_appended();
        let manifest = Manifest {
            branch: self.branch,
            hour: self.hour,
            flags: if self.samples.is_empty() { 0 } else { manifest::SAMPLES },
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: appends.iter().map(|append| &append[..]).collect(),
            statuses: std::mem::replace(&mut self.statuses, vec![Status::NotRecorded; TABLE_COUNT]),
            read: std::mem::replace(&mut self.read, vec![0; TABLE_COUNT.div_ceil(64)]),
            samples: std::mem::take(&mut self.samples),
            sections: self.sections.iter().map(|(section, _)| section.clone()).collect(),
        };
        let bytes = manifest.write(&self.columns)?;
        let sections = std::mem::take(&mut self.sections);
        let joined: Vec<u8> = sections.iter().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
        self.written.fill(false);
        let seen = visit(&bytes, &joined);
        self.hour += 1;
        Ok(seen)
    }
}
