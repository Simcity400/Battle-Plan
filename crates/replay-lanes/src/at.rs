use replay_recording::{cells::TableReader, columnar_frames::Recording};
use replay_frame::frame::{DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, SharedView, TableView, Unsayable, cells_rows_of, cells_view, not_selected};
use crate::lanes::{self, Snapshot};
use replay_data::{rows::{TABLE_COUNT, TableId}, scope::CaptureMode};
use rayon::prelude::*;
use replay_columnar::{manifest::Status, unit::BLOCK};
use std::io;

pub use crate::lanes::Keys;

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug)]
pub struct Read<'a> {
    pub table: TableId,
    pub columns: Option<&'a [usize]>,
    pub keys: Keys<'a>,
}

impl Read<'_> {
    pub fn all(table: TableId) -> Self {
        Self { table, columns: None, keys: Keys::All }
    }
}

pub fn snapshots(recording: &Recording, run: u32, hour: i32, reads: &[Read<'_>]) -> io::Result<Vec<Snapshot>> {
    reads.par_iter().map(|read| lanes::at_keys(recording, run, read.table, hour, read.columns, read.keys)).collect()
}

pub fn frame(recording: &Recording, run: u32, hour: i32, reads: &[Read<'_>]) -> io::Result<DecodedFrame> {
    let held = recording.run(run)?;
    let ordinal = held.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))?;
    chosen_once(reads)?;
    let (mode, format) = (CaptureMode::from_byte(recording.root_identity().capture_mode)?, recording.format());
    let views = reads.par_iter().map(|read| view(recording, run, hour, read, mode, format)).collect::<io::Result<Vec<_>>>()?;
    assembled(recording, run, ordinal, held.branch, hour, reads, views)
}

fn chosen_once(reads: &[Read<'_>]) -> io::Result<()> {
    let mut chosen = [false; TABLE_COUNT];
    for read in reads {
        if std::mem::replace(&mut chosen[read.table.index()], true) {
            return Err(invalid(format!("{:?} is read twice in one frame", read.table)));
        }
    }
    Ok(())
}

fn assembled(recording: &Recording, run: u32, ordinal: u32, branch: u64, hour: i32, reads: &[Read<'_>], views: Vec<(SharedView, bool)>) -> io::Result<DecodedFrame> {
    let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
    let mut table_ns = [0; TABLE_COUNT];
    for (read, (view, was_read)) in reads.iter().zip(views) {
        if was_read {
            table_ns[read.table.index()] = READ_TIME_NOT_RECORDED;
        }
        tables[read.table.index()] = view;
    }
    let clock = recording.clock(run, ordinal / BLOCK)?;
    let at = clock.get((ordinal % BLOCK) as usize).copied().unwrap_or_default();
    let metrics = Metrics { total_ns: at.total_ns, table_ns, pool_bytes: at.pool_bytes, dropped_frames: at.dropped_frames };
    Ok(DecodedFrame { branch, hour, tables, metrics })
}

fn view(recording: &Recording, run: u32, hour: i32, read: &Read<'_>, mode: CaptureMode, format: u16) -> io::Result<(SharedView, bool)> {
    let snapshot = match lanes::at_keys(recording, run, read.table, hour, read.columns, read.keys) {
        Ok(snapshot) => snapshot,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok((TableView::Unavailable { reason: error.to_string() }.into(), false)),
        Err(error) => return Err(error),
    };
    Ok((typed(read, &snapshot.view(), snapshot.status, &snapshot.rows, snapshot.sampled_at, mode, format)?, snapshot.was_read))
}

fn typed(read: &Read<'_>, cells: &replay_columnar::store::TableView<'_>, status: Status, rows: &[(u128, u32)], sampled_at: Option<i32>, mode: CaptureMode, format: u16) -> io::Result<SharedView> {
    let fields = read.columns.map_or(u64::MAX, |columns| columns.iter().filter(|&&column| column < 64).fold(0, |fields, &column| fields | 1 << column));
    let made = match status {
        Status::Complete => cells_rows_of(read.table, cells, rows, format, fields).map(TableView::Complete),
        Status::Truncated { missing } => cells_rows_of(read.table, cells, rows, format, fields).map(|rows| TableView::Truncated { rows, missing }),
        status => cells_view(read.table, cells, status, None, mode, format),
    };
    let made = match made {
        Ok(made) => made,
        Err(error) if Unsayable::of(&error).is_some() => TableView::Unavailable { reason: error.to_string() },
        Err(error) => return Err(error),
    };
    let complete = matches!(made, TableView::Complete(_));
    let mut made = SharedView::from(made).with_keys(selection(read.keys));
    if complete {
        made = made.with_sampled_at(sampled_at);
    }
    Ok(made)
}

fn selection(keys: Keys<'_>) -> Option<std::sync::Arc<replay_frame::frame::Keys>> {
    use replay_frame::frame::Keys as Selected;
    let selected = match keys {
        Keys::All => return None,
        Keys::These(keys) => {
            let mut keys = keys.to_vec();
            keys.sort_unstable();
            keys.dedup();
            Selected::These(keys)
        }
        Keys::Range(low, high) => Selected::Range(low, high),
        Keys::Ranges(ranges) => Selected::Ranges(ranges.to_vec()),
    };
    Some(std::sync::Arc::new(selected))
}

pub struct Cursor<'r, 'a> {
    recording: &'r Recording,
    reads: Vec<Read<'a>>,
    readers: Option<(u32, Vec<Result<TableReader, String>>)>,
    failed: bool,
    mode: CaptureMode,
    format: u16,
}

pub struct Readers {
    source: std::sync::Weak<replay_recording::columnar_frames::Source>,
    revision: replay_columnar::container::Revision,
    reads: Vec<ReadKey>,
    run: u32,
    readers: Vec<Result<TableReader, String>>,
}

type ReadKey = (TableId, Vec<usize>, Option<String>, bool);

fn read_tables(recording: &Recording, reads: &[Read<'_>]) -> io::Result<Vec<ReadKey>> {
    reads
        .iter()
        .map(|read| {
            let index = read.table.index();
            let recorded = recording.columns()[index].min(read.table.schema().len());
            let columns = replay_recording::cells::read_columns(recording, recorded, index, read.columns);
            Ok((read.table, columns, recording.withheld_reason(index).map(str::to_owned), recording.predates(index).is_some()))
        })
        .collect()
}

impl<'r, 'a> Cursor<'r, 'a> {
    pub fn new(recording: &'r Recording, reads: &[Read<'a>]) -> io::Result<Self> {
        chosen_once(reads)?;
        Ok(Self {
            recording,
            reads: reads.to_vec(),
            readers: None,
            failed: false,
            mode: CaptureMode::from_byte(recording.root_identity().capture_mode)?,
            format: recording.format(),
        })
    }

    pub fn resume(recording: &'r Recording, reads: &[Read<'a>], left: Option<Readers>) -> io::Result<Self> {
        let mut cursor = Self::new(recording, reads)?;
        if let Some(left) = left
            && left.source.upgrade().is_some_and(|source| std::sync::Arc::ptr_eq(&source, &recording.source()))
            && left.revision == recording.revision()
            && left.reads == read_tables(recording, reads)?
        {
            cursor.readers = Some((left.run, left.readers));
        }
        Ok(cursor)
    }

    pub fn into_readers(self) -> Option<Readers> {
        if self.failed {
            return None;
        }
        let (run, readers) = self.readers?;
        let reads = read_tables(self.recording, &self.reads).ok()?;
        Some(Readers { source: std::sync::Arc::downgrade(&self.recording.source()), revision: self.recording.revision(), reads, run, readers })
    }

    pub fn frame(&mut self, run: u32, hour: i32) -> io::Result<DecodedFrame> {
        let frame = self.read(run, hour);
        self.failed |= frame.is_err();
        frame
    }

    fn read(&mut self, run: u32, hour: i32) -> io::Result<DecodedFrame> {
        let recording = self.recording;
        let held = recording.run(run)?;
        let ordinal = held.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))?;
        if self.readers.as_ref().is_none_or(|(at, _)| *at != run) {
            let readers = self.reads.iter().map(|read| opened(recording, run, read)).collect::<io::Result<Vec<_>>>()?;
            self.readers = Some((run, readers));
        }
        let Some((_, readers)) = self.readers.as_mut() else { return Err(invalid("A frame cursor holds no readers")) };
        let (mode, format) = (self.mode, self.format);
        let views = readers.par_iter_mut().zip(&self.reads).map(|(reader, read)| moved(recording, (run, ordinal), reader, read, mode, format)).collect::<io::Result<Vec<_>>>()?;
        assembled(recording, run, ordinal, held.branch, hour, &self.reads, views)
    }
}

pub struct Seeker<'r> {
    recording: &'r Recording,
    run: Option<u32>,
    readers: Vec<((TableId, Option<Vec<usize>>), Result<TableReader, String>)>,
    mode: CaptureMode,
    format: u16,
}

impl<'r> Seeker<'r> {
    pub fn new(recording: &'r Recording) -> io::Result<Self> {
        Ok(Self { recording, run: None, readers: Vec::new(), mode: CaptureMode::from_byte(recording.root_identity().capture_mode)?, format: recording.format() })
    }

    pub fn frame(&mut self, run: u32, hour: i32, reads: &[Read<'_>]) -> io::Result<DecodedFrame> {
        let recording = self.recording;
        let held = recording.run(run)?;
        let ordinal = held.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))?;
        chosen_once(reads)?;
        if self.run != Some(run) {
            self.readers.clear();
            self.run = Some(run);
        }
        let mut taken = Vec::with_capacity(reads.len());
        for read in reads {
            let key = (read.table, read.columns.map(<[usize]>::to_vec));
            let reader = match self.readers.iter().position(|(held, _)| *held == key) {
                Some(at) => self.readers.swap_remove(at),
                None => (key, opened(recording, run, read)?),
            };
            taken.push(reader);
        }
        let (mode, format) = (self.mode, self.format);
        let views = taken.par_iter_mut().zip(reads).map(|((_, reader), read)| moved(recording, (run, ordinal), reader, read, mode, format)).collect::<io::Result<Vec<_>>>();
        if views.is_ok() {
            self.readers.extend(taken);
        }
        assembled(recording, run, ordinal, held.branch, hour, reads, views?)
    }

    #[cfg(test)]
    fn restores(&self) -> Vec<(TableId, u32)> {
        self.readers.iter().filter_map(|((table, _), reader)| reader.as_ref().ok().map(|reader| (*table, reader.restores()))).collect()
    }
}

fn opened(recording: &Recording, run: u32, read: &Read<'_>) -> io::Result<Result<TableReader, String>> {
    match TableReader::new(recording, run, read.table, read.columns) {
        Ok(reader) => Ok(Ok(reader)),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(Err(error.to_string())),
        Err(error) => Err(error),
    }
}

fn moved(recording: &Recording, (run, ordinal): (u32, u32), slot: &mut Result<TableReader, String>, read: &Read<'_>, mode: CaptureMode, format: u16) -> io::Result<(SharedView, bool)> {
    let reader = match slot {
        Ok(reader) => reader,
        Err(reason) => return Ok((TableView::Unavailable { reason: reason.clone() }.into(), false)),
    };
    let rows = match reader.go_to(recording, ordinal).and_then(|()| lanes::asked_rows(reader, read.keys)) {
        Ok(rows) => rows,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            *slot = opened(recording, run, read)?;
            return Ok((TableView::Unavailable { reason: error.to_string() }.into(), false));
        }
        Err(error) => return Err(error),
    };
    Ok((typed(read, &reader.view(), reader.status(), &rows, reader.sampled_at(), mode, format)?, reader.was_read()))
}
