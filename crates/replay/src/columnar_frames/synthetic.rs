//! Hours written as the native recorder writes them and walked as preparation walks them, each
//! also seen as the row decoder's frame of them: for tests holding a family's columnar path to
//! its row path on the same hours.

use super::{Views, schema};
use crate::{
    format::FormatVersion,
    frame::DecodedFrame,
    native_columns::{RowCells, TableInput, table_hour},
    rows::{RowCodec, TABLE_COUNT, TableId},
    scope::CaptureMode,
};
use replay_columnar::{
    manifest::{self, Manifest, Section, Status},
    walk::State,
    writer::{Dictionary as Strings, TableWriter},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    io,
};

pub use crate::native_columns::TableInput as Input;
pub use replay_columnar::writer::Sample;

/// A walk of synthetic hours: write each table's input with [`Self::table`], then end the
/// hour with [`Self::end`].
pub struct Synthetic {
    state: State,
    writers: HashMap<TableId, TableWriter>,
    strings: Strings,
    cells: RowCells,
    views: Views,
    columns: Vec<usize>,
    branch: u64,
    hour: i32,
    first: bool,
    statuses: Vec<Status>,
    read: Vec<u64>,
    samples: Vec<(usize, Option<i32>)>,
    sections: Vec<(Section, Vec<u8>)>,
}

impl Synthetic {
    /// A walk of branch `branch` whose first hour is `hour`, its frames holding `tables`.
    pub fn new(tables: &[TableId], branch: u64, hour: i32) -> io::Result<Self> {
        let schema = schema()?;
        Ok(Self {
            columns: schema.iter().map(Vec::len).collect(),
            state: State::new(schema)?,
            writers: HashMap::new(),
            strings: Strings::new(),
            cells: RowCells::default(),
            views: Views::new(tables, FormatVersion::CURRENT, CaptureMode::Full)?,
            branch,
            hour,
            first: true,
            statuses: vec![Status::NotRecorded; TABLE_COUNT],
            read: vec![0; TABLE_COUNT.div_ceil(64)],
            samples: Vec::new(),
            sections: Vec::new(),
        })
    }

    /// The walker's state as of the last hour ended: a restart's, for a segment starting at
    /// the next hour.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Writes `table`'s input for the hour under way, as the native recorder writes it.
    pub fn table<R: RowCodec>(
        &mut self,
        table: TableId,
        input: TableInput<'_, R>,
    ) -> io::Result<()> {
        let writer = match self.writers.entry(table) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(TableWriter::new(&schema()?[table.index()])?),
        };
        let written = table_hour(writer, &mut self.strings, &mut self.cells, input)?;
        let index = table.index();
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
            self.sections.push((
                Section {
                    table: index,
                    bytes: bytes.len() as u64,
                    columns: prepared.columns.clone(),
                    ops: prepared.ops,
                },
                bytes.clone(),
            ));
        }
        writer.commit(written.prepared, written.evidence, self.hour)
    }

    /// Ends the hour under way: the walker applies it, and `visit` sees the walker's state,
    /// the hour's manifest, the row decoder's frame of it, and whether the hour starts the walk.
    pub fn end<T>(
        &mut self,
        visit: impl FnOnce(&State, &Manifest<'_>, &DecodedFrame, bool) -> T,
    ) -> io::Result<T> {
        self.sections.sort_by_key(|(section, _)| section.table);
        let appends = self.strings.take_appended();
        let manifest = Manifest {
            branch: self.branch,
            hour: self.hour,
            flags: if self.samples.is_empty() {
                0
            } else {
                manifest::SAMPLES
            },
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: appends.iter().map(|append| &append[..]).collect(),
            statuses: std::mem::replace(&mut self.statuses, vec![Status::NotRecorded; TABLE_COUNT]),
            read: std::mem::replace(&mut self.read, vec![0; TABLE_COUNT.div_ceil(64)]),
            samples: std::mem::take(&mut self.samples),
            sections: self
                .sections
                .iter()
                .map(|(section, _)| section.clone())
                .collect(),
        };
        // The manifest's own bytes, read back as the walker reads them.
        let bytes = manifest.write(&self.columns)?;
        let manifest = Manifest::read(&bytes, &self.columns)?;
        let sections = std::mem::take(&mut self.sections);
        self.state.apply(&manifest, |at| Ok(&sections[at].1[..]))?;
        let frame = self.views.advance(&self.state, &manifest, self.first)?;
        let starts = std::mem::replace(&mut self.first, false);
        let seen = visit(&self.state, &manifest, &frame, starts);
        self.hour += 1;
        Ok(seen)
    }
}
