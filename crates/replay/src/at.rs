//! Chosen tables of a run at an hour, each read on its own thread from its own lane
//! ([`crate::lanes::at_keys`]): as the store's snapshots ([`snapshots`]), or as the typed frame the
//! playhead hands the dashboard ([`frame`]), each table's rows built only for the keys asked; or
//! at many hours in turn ([`Cursor`]), each table's reader moved on from the hour asked before.

use crate::{
    cells::TableReader,
    columnar_frames::Recording,
    format::FormatVersion,
    frame::{DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, SharedView, TableView, Unsayable, cells_rows_of, cells_view, not_selected},
    lanes::{self, Snapshot},
    rows::{TABLE_COUNT, TableId},
    scope::CaptureMode,
};
use rayon::prelude::*;
use replay_columnar::{manifest::Status, unit::BLOCK};
use std::io;

pub use crate::lanes::Keys;

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// One table to read at an hour: its columns (every column when none; in a frame, a row's other
/// fields hold their defaults, never decoded) and the rows asked.
#[derive(Clone, Copy, Debug)]
pub struct Read<'a> {
    pub table: TableId,
    pub columns: Option<&'a [usize]>,
    pub keys: Keys<'a>,
}

impl Read<'_> {
    /// Every column and row of `table`.
    pub fn all(table: TableId) -> Self {
        Self { table, columns: None, keys: Keys::All }
    }
}

/// `reads`' tables of run `run` after `hour`, in the order asked, each read on its own thread.
pub fn snapshots(recording: &Recording, run: u32, hour: i32, reads: &[Read<'_>]) -> io::Result<Vec<Snapshot>> {
    reads.par_iter().map(|read| lanes::at_keys(recording, run, read.table, hour, read.columns, read.keys)).collect()
}

/// A frame holding `reads`' tables after `hour` of `run` (every other table not selected), each
/// read on its own thread; a table's status and sample hour as its lane gives them, its rows
/// those its read asks for. A table withheld or damaged, or holding a row that cannot say a cell
/// is not recorded, is unavailable by name; the rest are built. Named by the run's branch.
pub fn frame(recording: &Recording, run: u32, hour: i32, reads: &[Read<'_>]) -> io::Result<DecodedFrame> {
    let held = recording.run(run)?;
    let ordinal = held.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))?;
    chosen_once(reads)?;
    let (mode, format) = (CaptureMode::from_byte(recording.root_identity().capture_mode)?, FormatVersion::CURRENT.version());
    let views = reads.par_iter().map(|read| view(recording, run, hour, read, mode, format)).collect::<io::Result<Vec<_>>>()?;
    assembled(recording, run, ordinal, held.branch, hour, reads, views)
}

/// Each table of `reads` named once.
fn chosen_once(reads: &[Read<'_>]) -> io::Result<()> {
    let mut chosen = [false; TABLE_COUNT];
    for read in reads {
        if std::mem::replace(&mut chosen[read.table.index()], true) {
            return Err(invalid(format!("{:?} is read twice in one frame", read.table)));
        }
    }
    Ok(())
}

/// The frame of `reads`' views (each with whether the hour read its table) at `hour`, run
/// ordinal `ordinal`, its metrics from the block's clock.
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

/// `read`'s table as the frame's typed view, and whether the hour read it.
fn view(recording: &Recording, run: u32, hour: i32, read: &Read<'_>, mode: CaptureMode, format: u16) -> io::Result<(SharedView, bool)> {
    let snapshot = match lanes::at_keys(recording, run, read.table, hour, read.columns, read.keys) {
        Ok(snapshot) => snapshot,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok((TableView::Unavailable { reason: error.to_string() }.into(), false)),
        Err(error) => return Err(error),
    };
    Ok((typed(read, &snapshot.view(), snapshot.status, &snapshot.rows, snapshot.sampled_at, mode, format)?, snapshot.was_read))
}

/// `read`'s table as a frame's typed view: from its cells, status and sample hour, of the rows asked.
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
    let mut made = SharedView::from(made);
    if complete {
        made = made.with_sampled_at(sampled_at);
    }
    Ok(made)
}

/// [`frame`]s of the same reads at hours asked in turn: each table keeps its reader, moved on from
/// the hour asked before ([`TableReader::go_to`]), so frames at many hours cost one restore of each
/// table and the blocks between, not a restore an hour. Later hours cost least; an earlier hour, or
/// one past a nearer restore, is sought.
pub struct Cursor<'r, 'a> {
    recording: &'r Recording,
    reads: Vec<Read<'a>>,
    /// The run its readers read, and each read's reader, or why its table holds no rows there.
    readers: Option<(u32, Vec<Result<TableReader, String>>)>,
    /// A frame failed: a reader may be left between hours, so none is handed on.
    failed: bool,
    mode: CaptureMode,
    format: u16,
}

/// The readers a [`Cursor`] leaves ([`Cursor::into_readers`]), each where its last frame left
/// it: a later cursor of the same tables and columns on the same run starts from them
/// ([`Cursor::resume`]), so replies asking frames one after another restore no table again.
pub struct Readers {
    reads: Vec<(TableId, Option<Vec<usize>>)>,
    run: u32,
    ordinals: u32,
    readers: Vec<Result<TableReader, String>>,
}

/// What a reader of `reads` is a reader of: each table and its columns (a read's keys are applied
/// to the reader's rows, so any keys share it).
fn read_tables(reads: &[Read<'_>]) -> Vec<(TableId, Option<Vec<usize>>)> {
    reads.iter().map(|read| (read.table, read.columns.map(<[usize]>::to_vec))).collect()
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
            format: FormatVersion::CURRENT.version(),
        })
    }

    /// A cursor of `reads` starting from the readers an earlier one left, when they read the
    /// same tables and columns of a run still holding the hours it held; else as [`Self::new`].
    pub fn resume(recording: &'r Recording, reads: &[Read<'a>], left: Option<Readers>) -> io::Result<Self> {
        let mut cursor = Self::new(recording, reads)?;
        if let Some(left) = left
            && left.reads == read_tables(reads)
            && recording.run(left.run).is_ok_and(|held| held.ordinals == left.ordinals)
        {
            cursor.readers = Some((left.run, left.readers));
        }
        Ok(cursor)
    }

    /// Its readers, to start a later cursor where this one stopped; none if a frame failed or
    /// none was read.
    pub fn into_readers(self) -> Option<Readers> {
        if self.failed {
            return None;
        }
        let (run, readers) = self.readers?;
        let ordinals = self.recording.run(run).ok()?.ordinals;
        Some(Readers { reads: read_tables(&self.reads), run, ordinals, readers })
    }

    /// The frame [`frame`] gives of its reads at `hour` of run `run`.
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

/// [`frame`]s of whatever tables and keys each hour asks, at hours asked in turn: as a [`Cursor`],
/// each table read keeps its reader (one per table and columns, opened at its first read), moved
/// on from the hour it last read, so a reply naming things at many hours restores each table once
/// and reads the blocks between, whichever keys each hour wants.
pub struct Seeker<'r> {
    recording: &'r Recording,
    run: Option<u32>,
    readers: Vec<((TableId, Option<Vec<usize>>), Result<TableReader, String>)>,
    mode: CaptureMode,
    format: u16,
}

impl<'r> Seeker<'r> {
    pub fn new(recording: &'r Recording) -> io::Result<Self> {
        Ok(Self { recording, run: None, readers: Vec::new(), mode: CaptureMode::from_byte(recording.root_identity().capture_mode)?, format: FormatVersion::CURRENT.version() })
    }

    /// The frame [`frame`] gives of `reads` at `hour` of run `run`.
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
        // A failed move may leave a reader between hours: none is kept past it.
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

/// `read`'s reader of run `run`, or why its table holds no rows there (withheld or damaged).
fn opened(recording: &Recording, run: u32, read: &Read<'_>) -> io::Result<Result<TableReader, String>> {
    match TableReader::new(recording, run, read.table, read.columns) {
        Ok(reader) => Ok(Ok(reader)),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(Err(error.to_string())),
        Err(error) => Err(error),
    }
}

/// `read`'s table at ordinal `ordinal` of run `run`, its reader moved there, as [`view`] gives it.
/// A reader failing on damage is replaced by a fresh one, so the next hour is sought, never
/// advanced from what a failed move left.
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
