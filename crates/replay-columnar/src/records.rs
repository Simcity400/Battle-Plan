//! A block's hour records: every hour's [`Manifest`] but its branch and hour, which its run
//! gives, written once across the block, each part in its own stream and as its change from
//! the hour before: which tables changed status and to what, which were read and which have
//! changes, and per such table how its changed columns and op flags differ from its last.
//! The statuses the block starts with lead it, so it reads alone.
//!
//! Payload: hours (varint) | tables (varint) | each table's status at the block's start | a
//! bit per stream that is not empty | each such stream's length (varint) | the streams.

use crate::{
    block::{get_set, put_set, put_varint},
    invalid,
    leaf::{Reader, Writer},
    manifest::{self, Manifest, Section, Status},
};
use std::io;

const FLAGS: usize = 0;
const METRICS: usize = 1;
const APPENDS: usize = 2;
const STATUSES: usize = 3;
const READ: usize = 4;
const SAMPLES: usize = 5;
const PRESENT: usize = 6;
const SECTIONS: usize = 7;
const STREAMS: usize = 8;

fn put_status(out: &mut Vec<u8>, status: Status) {
    match status {
        Status::NotRecorded => out.push(0),
        Status::Complete => out.push(1),
        Status::Truncated { missing } => {
            out.push(2);
            put_varint(out, u64::from(missing));
        }
        Status::Unavailable { reason } => {
            out.push(3);
            put_varint(out, u64::from(reason));
        }
    }
}

fn get_status(reader: &mut Reader<'_>) -> io::Result<Status> {
    Ok(match reader.u8()? {
        0 => Status::NotRecorded,
        1 => Status::Complete,
        2 => Status::Truncated {
            missing: reader.varint_u32()?,
        },
        3 => Status::Unavailable {
            reason: reader.varint_u32()?,
        },
        other => return Err(invalid(format!("Table status {other}"))),
    })
}

fn bits_to_list(words: &[u64], into: &mut Vec<u32>) {
    into.clear();
    for (index, &word) in words.iter().enumerate() {
        let mut rest = word;
        while rest != 0 {
            into.push(index as u32 * 64 + rest.trailing_zeros());
            rest &= rest - 1;
        }
    }
}

/// Writes one block's hour records.
pub struct RecordsEncoder {
    columns: Vec<usize>,
    hours: u32,
    start: Vec<Status>,
    statuses: Vec<Status>,
    previous_read: Vec<u32>,
    previous_present: Vec<u32>,
    /// Each table's last changed columns and op flags.
    previous_columns: Vec<Vec<u64>>,
    previous_ops: Vec<u8>,
    previous_pool: u64,
    streams: [Vec<u8>; STREAMS],
    list: Vec<u32>,
    scratch: Vec<u32>,
}

impl RecordsEncoder {
    /// `columns` holds each table's column count; `statuses` each table's status as the block
    /// starts.
    pub fn new(columns: &[usize], statuses: &[Status]) -> io::Result<Self> {
        if statuses.len() != columns.len() {
            return Err(invalid("Statuses disagree with the schema's tables"));
        }
        Ok(Self {
            columns: columns.to_vec(),
            hours: 0,
            start: statuses.to_vec(),
            statuses: statuses.to_vec(),
            previous_read: Vec::new(),
            previous_present: Vec::new(),
            previous_columns: columns.iter().map(|&count| vec![0; count.div_ceil(64)]).collect(),
            previous_ops: vec![0; columns.len()],
            previous_pool: 0,
            streams: Default::default(),
            list: Vec::new(),
            scratch: Vec::new(),
        })
    }

    pub fn hour(&mut self, manifest: &Manifest<'_>) -> io::Result<()> {
        let tables = self.columns.len();
        if manifest.statuses.len() != tables || manifest.read.len() != tables.div_ceil(64) {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        self.hours += 1;
        let [flags, metrics, appends, statuses, read, samples, present, sections] = &mut self.streams;
        flags.push(manifest.flags);
        put_varint(metrics, manifest.total_ns);
        put_varint(metrics, crate::block::zig(manifest.pool_bytes.wrapping_sub(self.previous_pool) as i64));
        self.previous_pool = manifest.pool_bytes;
        put_varint(metrics, manifest.dropped_frames);
        put_varint(appends, manifest.appends.len() as u64);
        for append in &manifest.appends {
            put_varint(appends, append.len() as u64);
            appends.extend_from_slice(append);
        }
        let changed = (0..tables).filter(|&table| manifest.statuses[table] != self.statuses[table]);
        let count = changed.clone().count();
        put_varint(statuses, count as u64);
        let mut next = 0;
        for table in changed {
            put_varint(statuses, (table - next) as u64);
            next = table + 1;
            put_status(statuses, manifest.statuses[table]);
        }
        self.statuses.copy_from_slice(&manifest.statuses);
        bits_to_list(&manifest.read, &mut self.list);
        put_set(read, &mut self.previous_read, &self.list, &mut self.scratch);
        if manifest.flags & manifest::SAMPLES != 0 {
            put_varint(samples, manifest.samples.len() as u64);
            let mut next = 0;
            for &(table, hour) in &manifest.samples {
                put_varint(samples, (table - next) as u64);
                next = table + 1;
                put_varint(samples, hour.map_or(0, |hour| crate::block::zig(i64::from(hour)) + 1));
            }
        }
        self.list.clear();
        self.list.extend(manifest.sections.iter().map(|section| section.table as u32));
        put_set(present, &mut self.previous_present, &self.list, &mut self.scratch);
        for section in &manifest.sections {
            let table = section.table;
            if section.columns.len() != self.previous_columns[table].len() {
                return Err(invalid("A section's columns disagree with its table"));
            }
            for (word, previous) in section.columns.iter().zip(&mut self.previous_columns[table]) {
                put_varint(sections, word ^ *previous);
                *previous = *word;
            }
            sections.push(section.ops ^ self.previous_ops[table]);
            self.previous_ops[table] = section.ops;
        }
        Ok(())
    }

    /// The statuses after the hours written so far.
    pub fn statuses(&self) -> &[Status] {
        &self.statuses
    }

    pub fn finish(self) -> io::Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.varint(u64::from(self.hours))?;
        writer.varint(self.columns.len() as u64)?;
        let mut out = writer.into_bytes();
        for &status in &self.start {
            put_status(&mut out, status);
        }
        let mut present = 0u8;
        for (index, stream) in self.streams.iter().enumerate() {
            if !stream.is_empty() {
                present |= 1 << index;
            }
        }
        out.push(present);
        for stream in self.streams.iter().filter(|stream| !stream.is_empty()) {
            put_varint(&mut out, stream.len() as u64);
        }
        for stream in &self.streams {
            out.extend_from_slice(stream);
        }
        Ok(out)
    }
}

/// Reads one block's hour records.
pub struct RecordsDecoder<'a> {
    columns: Vec<usize>,
    hours: u32,
    read_hours: u32,
    statuses: Vec<Status>,
    previous_read: Vec<u32>,
    previous_present: Vec<u32>,
    previous_columns: Vec<Vec<u64>>,
    previous_ops: Vec<u8>,
    previous_pool: u64,
    streams: Vec<Reader<'a>>,
    list: Vec<u32>,
    merged: Vec<u32>,
}

impl<'a> RecordsDecoder<'a> {
    /// The bytes its own state holds, its payload aside.
    pub fn held_bytes(&self) -> usize {
        8 * self.columns.capacity()
            + self.statuses.capacity() * std::mem::size_of::<Status>()
            + 4 * (self.previous_read.capacity() + self.previous_present.capacity())
            + self
                .previous_columns
                .iter()
                .map(|columns| 8 * columns.capacity())
                .sum::<usize>()
            + self.previous_ops.capacity()
            + self.streams.capacity() * std::mem::size_of::<Reader<'_>>()
    }

    /// The decoder where it stands, over `to`, a copy of `from`, which its payload lies in.
    pub fn rebased<'b>(&self, from: &[u8], to: &'b [u8]) -> io::Result<RecordsDecoder<'b>> {
        let mut streams = engine_allocation::vec(self.streams.len())?;
        for stream in &self.streams {
            streams.push(stream.rebased(from, to)?);
        }
        let mut previous_columns = engine_allocation::vec(self.previous_columns.len())?;
        for columns in &self.previous_columns {
            previous_columns.push(engine_allocation::copy_vec(columns)?);
        }
        Ok(RecordsDecoder {
            columns: engine_allocation::copy_vec(&self.columns)?,
            hours: self.hours,
            read_hours: self.read_hours,
            statuses: engine_allocation::copy_vec(&self.statuses)?,
            previous_read: engine_allocation::copy_vec(&self.previous_read)?,
            previous_present: engine_allocation::copy_vec(&self.previous_present)?,
            previous_columns,
            previous_ops: engine_allocation::copy_vec(&self.previous_ops)?,
            previous_pool: self.previous_pool,
            streams,
            list: Vec::new(),
            merged: Vec::new(),
        })
    }

    pub fn new(columns: &[usize], payload: &'a [u8]) -> io::Result<Self> {
        let mut reader = Reader::new(payload);
        let hours = reader.varint_u32()?;
        if reader.count()? != columns.len() {
            return Err(invalid("Hour records disagree with the schema's tables"));
        }
        let mut statuses = engine_allocation::vec(columns.len())?;
        for _ in 0..columns.len() {
            statuses.push(get_status(&mut reader)?);
        }
        let present = reader.u8()?;
        let mut lengths = [0usize; STREAMS];
        for (index, length) in lengths.iter_mut().enumerate() {
            if present >> index & 1 == 1 {
                *length = reader.count()?;
            }
        }
        let mut at = reader.at();
        let mut streams = Vec::with_capacity(STREAMS);
        for length in lengths {
            let end = at
                .checked_add(length)
                .filter(|&end| end <= payload.len())
                .ok_or_else(|| invalid("An hour record stream passes its payload"))?;
            streams.push(Reader::new(&payload[at..end]));
            at = end;
        }
        if at != payload.len() {
            return Err(invalid("Hour records have bytes after their streams"));
        }
        Ok(Self {
            columns: columns.to_vec(),
            hours,
            read_hours: 0,
            statuses,
            previous_read: Vec::new(),
            previous_present: Vec::new(),
            previous_columns: columns.iter().map(|&count| vec![0; count.div_ceil(64)]).collect(),
            previous_ops: vec![0; columns.len()],
            previous_pool: 0,
            streams,
            list: Vec::new(),
            merged: Vec::new(),
        })
    }

    pub fn hours(&self) -> u32 {
        self.hours
    }

    /// Each table's status before the next hour.
    pub fn statuses(&self) -> &[Status] {
        &self.statuses
    }

    /// The next hour's record into `manifest`, of `branch` at `hour`.
    pub fn hour(&mut self, branch: u64, hour: i32, manifest: &mut Manifest<'a>) -> io::Result<()> {
        if self.read_hours == self.hours {
            return Err(invalid("A block holds fewer hour records than its hours"));
        }
        self.read_hours += 1;
        let tables = self.columns.len();
        manifest.branch = branch;
        manifest.hour = hour;
        let streams = &mut self.streams;
        manifest.flags = streams[FLAGS].u8()?;
        manifest.total_ns = streams[METRICS].varint()?;
        self.previous_pool = self
            .previous_pool
            .wrapping_add(crate::block::unzig(streams[METRICS].varint()?) as u64);
        manifest.pool_bytes = self.previous_pool;
        manifest.dropped_frames = streams[METRICS].varint()?;
        let appends = streams[APPENDS].count()?;
        if appends > streams[APPENDS].left() {
            return Err(invalid("An hour record appends more strings than its bytes"));
        }
        manifest.appends.clear();
        engine_allocation::reserve(&mut manifest.appends, appends)?;
        for _ in 0..appends {
            let length = streams[APPENDS].count()?;
            manifest.appends.push(streams[APPENDS].raw(length)?);
        }
        let changed = streams[STATUSES].count()?;
        let mut next = 0usize;
        for _ in 0..changed {
            let table = next
                .checked_add(streams[STATUSES].count()?)
                .filter(|&table| table < tables)
                .ok_or_else(|| invalid("A status change names a table past the schema"))?;
            self.statuses[table] = get_status(&mut streams[STATUSES])?;
            next = table + 1;
        }
        manifest.statuses.clear();
        engine_allocation::reserve(&mut manifest.statuses, tables)?;
        manifest.statuses.extend_from_slice(&self.statuses);
        get_set(&mut streams[READ], &mut self.previous_read, &mut self.list, &mut self.merged)?;
        manifest.read.clear();
        engine_allocation::resize_default(&mut manifest.read, tables.div_ceil(64))?;
        for &table in &self.previous_read {
            if table as usize >= tables {
                return Err(invalid("An hour record reads a table past the schema"));
            }
            manifest.read[table as usize / 64] |= 1 << (table % 64);
        }
        manifest.samples.clear();
        if manifest.flags & manifest::SAMPLES != 0 {
            let count = streams[SAMPLES].count()?;
            if count == 0 || count > tables {
                return Err(invalid("Manifest samples disagree with its tables"));
            }
            engine_allocation::reserve(&mut manifest.samples, count)?;
            let mut next = 0usize;
            for _ in 0..count {
                let table = next
                    .checked_add(streams[SAMPLES].count()?)
                    .filter(|&table| table < tables)
                    .ok_or_else(|| invalid("A sample names a table past the schema"))?;
                next = table + 1;
                let hour = match streams[SAMPLES].varint()? {
                    0 => None,
                    code => Some(
                        i32::try_from(crate::block::unzig(code - 1))
                            .map_err(|_| invalid("A sample hour passes 32 bits"))?,
                    ),
                };
                manifest.samples.push((table, hour));
            }
        }
        get_set(&mut streams[PRESENT], &mut self.previous_present, &mut self.list, &mut self.merged)?;
        manifest.sections.clear();
        engine_allocation::reserve(&mut manifest.sections, self.previous_present.len())?;
        for &table in &self.previous_present {
            let table = table as usize;
            if table >= tables {
                return Err(invalid("An hour record names a table past the schema"));
            }
            let mut columns = engine_allocation::vec(self.previous_columns[table].len())?;
            for previous in &mut self.previous_columns[table] {
                *previous ^= streams[SECTIONS].varint()?;
                columns.push(*previous);
            }
            let count = self.columns[table];
            if count % 64 != 0 && columns.last().is_some_and(|&last| last >> (count % 64) != 0) {
                return Err(invalid("An hour record names a column past its table"));
            }
            self.previous_ops[table] ^= streams[SECTIONS].u8()?;
            manifest.sections.push(Section {
                table,
                bytes: 0,
                columns,
                ops: self.previous_ops[table],
            });
        }
        Ok(())
    }

    /// Refuses records holding more than their hours.
    pub fn finish(&self) -> io::Result<()> {
        if self.read_hours != self.hours || self.streams.iter().any(|stream| stream.left() != 0) {
            return Err(invalid("Hour records hold more than their hours"));
        }
        Ok(())
    }
}
