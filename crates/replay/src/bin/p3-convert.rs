//! Offline converter: a recording to the columnar recording the live recorder writes of the
//! same hours, through its own [`FrameWriter`] and [`FileWriter`] at [`Settings::RECORDING`].
//! Gated by applying every section to a walker's tables and comparing them with the
//! recording's rows: every written slot each hour; at every unit's end and every run's end,
//! every live slot and the writer's held tables; every hour's stored metrics against its
//! manifest; and every table's status (a truncated table's missing count, a refusal's reason)
//! and sample hour each hour, in its manifest and as its writer holds them, against the
//! recording's view.
//!
//! Output: a version 4 `H4COLUMN` recording (`replay_columnar::envelope`), a run per
//! continuous span of one branch, and with `--run-units`, a run per that many units of a
//! span, for measuring walks over many runs; `--restart-units` restarts at another cadence
//! than the recorder's. A complete table the recording stores as a delta is compared with the
//! held table only at the rows the delta names.

use replay::{
    columnar_file::{Closed, FileWriter, Finished, Hour as FileHour, Position, Settings},
    columnar_frames::schema,
    container::{Replay, ReplayStream},
    native_frame::{Changed, FrameWriter, Source},
};
use replay_columnar::{
    envelope, framing,
    manifest::{Manifest, Status},
    section::{self, Scratch},
    state::{Cell, Kind as Column, Table},
    writer::Census,
};
use replay_data::{
    codec::Decoder,
    rows::{Kind, RowCodec, RowsVisit, TABLE_COUNT, TableId, TableRows},
};
use replay_frame::frame::{FrameInput, Metrics, READ_TIME_NOT_RECORDED, SharedView, TableInput, TableUpdate, TableView};
use serde_json::json;
use std::{
    ffi::OsString,
    fs::File,
    io::{self, BufWriter},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// A field's value parsed from replay-data field bytes.
#[derive(Debug, PartialEq, Eq)]
enum Parsed<'a> {
    Int(i128),
    Missing,
    Bytes(&'a [u8]),
    U32s(Vec<u32>),
    U64s(Vec<u64>),
}

fn int<const N: usize>(bytes: &[u8], signed: bool) -> io::Result<i128> {
    let array: [u8; N] = bytes.try_into().map_err(|_| invalid("fixed field width"))?;
    let mut wide = [0_u8; 16];
    wide[..N].copy_from_slice(&array);
    let value = i128::from_le_bytes(wide);
    let shift = 128 - 8 * N as u32;
    Ok(if signed { (value << shift) >> shift } else { value })
}

fn prefixed(field: &[u8]) -> io::Result<(usize, &[u8])> {
    let count = field.get(..4).ok_or_else(|| invalid("field has no length"))?;
    Ok((u32::from_le_bytes(count.try_into().unwrap()) as usize, &field[4..]))
}

fn parse(kind: Kind, field: &[u8]) -> io::Result<Parsed<'_>> {
    let optional = |width: usize, signed: bool| -> io::Result<Parsed<'_>> {
        match field.split_first() {
            Some((0, [])) => Ok(Parsed::Missing),
            Some((1, rest)) if rest.len() == width => Ok(Parsed::Int(match width {
                4 => int::<4>(rest, signed)?,
                _ => int::<8>(rest, signed)?,
            })),
            _ => Err(invalid("optional field is malformed")),
        }
    };
    Ok(match kind {
        Kind::U8 | Kind::Bool => Parsed::Int(int::<1>(field, false)?),
        Kind::I8 => Parsed::Int(int::<1>(field, true)?),
        Kind::U16 => Parsed::Int(int::<2>(field, false)?),
        Kind::I16 => Parsed::Int(int::<2>(field, true)?),
        Kind::U32 => Parsed::Int(int::<4>(field, false)?),
        Kind::I32 => Parsed::Int(int::<4>(field, true)?),
        Kind::U64 => Parsed::Int(int::<8>(field, false)?),
        Kind::I64 => Parsed::Int(int::<8>(field, true)?),
        Kind::OptionalI32 => optional(4, true)?,
        Kind::OptionalU32 => optional(4, false)?,
        Kind::OptionalI64 => optional(8, true)?,
        Kind::OptionalU64 => optional(8, false)?,
        Kind::Bytes | Kind::Opaque => {
            let (length, bytes) = prefixed(field)?;
            if bytes.len() != length {
                return Err(invalid("byte field length"));
            }
            Parsed::Bytes(bytes)
        }
        Kind::U32List => {
            let (count, items) = prefixed(field)?;
            if items.len() != count * 4 {
                return Err(invalid("list field count"));
            }
            Parsed::U32s(items.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().unwrap())).collect())
        }
        Kind::U64List => {
            let (count, items) = prefixed(field)?;
            if items.len() != count * 8 {
                return Err(invalid("list field count"));
            }
            Parsed::U64s(items.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().unwrap())).collect())
        }
    })
}

/// The cell a parsed field must read back as; a text cell's id names one of the run's `strings`.
fn same(cell: Cell<'_>, parsed: &Parsed<'_>, strings: &[Vec<u8>]) -> bool {
    match (cell, parsed) {
        (Cell::U8(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I8(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::U16(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I16(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::U32(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I32(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::U64(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I64(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::Bool(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::OptionalI32(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::OptionalU32(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::OptionalI64(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::OptionalU64(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::Bytes(id), Parsed::Bytes(expected)) => strings.get(id as usize).is_some_and(|bytes| bytes.as_slice() == *expected),
        (Cell::Opaque(value), Parsed::Bytes(expected)) => value == *expected,
        (Cell::U32List(value), Parsed::U32s(expected)) => value == expected.as_slice(),
        (Cell::U64List(value), Parsed::U64s(expected)) => value == expected.as_slice(),
        _ => false,
    }
}

fn optional_same(value: Option<i128>, parsed: &Parsed<'_>) -> bool {
    match (value, parsed) {
        (None, Parsed::Missing) => true,
        (Some(value), Parsed::Int(expected)) => value == *expected,
        _ => false,
    }
}

/// One recorded row's field bytes, where each field ends, and a bit per field it did not record.
#[derive(Default)]
struct Fields {
    bytes: Vec<u8>,
    ends: Vec<usize>,
    unrecorded: u64,
}

impl Fields {
    fn read(&mut self, kinds: &[Kind], rows: &TableRows, index: usize) -> io::Result<()> {
        self.bytes.clear();
        self.ends.clear();
        self.unrecorded = rows.encode_row_recorded_fields(index, &mut self.bytes)?;
        let mut decoder = Decoder::new(self.bytes.as_slice());
        for (column, kind) in kinds.iter().enumerate() {
            if self.unrecorded >> column & 1 == 0 {
                kind.decode(&mut decoder)?;
            }
            self.ends.push(self.bytes.len() - decoder.remaining());
        }
        Ok(())
    }

    fn recorded(&self, column: usize) -> bool {
        self.unrecorded >> column & 1 == 0
    }

    fn field(&self, column: usize) -> &[u8] {
        let start = column.checked_sub(1).map_or(0, |before| self.ends[before]);
        &self.bytes[start..self.ends[column]]
    }
}

/// The index of the row at a key among key-ordered rows.
struct Find(u128);

impl RowsVisit for Find {
    type Output = Option<usize>;

    fn rows<R: RowCodec>(self, rows: &[R]) -> Option<usize> {
        rows.binary_search_by_key(&self.0, R::key).ok()
    }
}

/// The walker's `table` holds row `index` of the recording's `rows`, keyed `key`: each cell
/// recorded as the row records it, and the run's `strings` naming its text.
fn check(id: TableId, table: &Table, rows: &TableRows, index: usize, key: u128, strings: &[Vec<u8>], fields: &mut Fields) -> io::Result<()> {
    let slot = table.slot(key).ok_or_else(|| invalid(format!("{id:?} row {key:#x} is not held")))?;
    fields.read(id.schema(), rows, index)?;
    for (column, &kind) in id.schema().iter().enumerate() {
        if table.recorded(column, slot) != fields.recorded(column) {
            return Err(invalid(format!("{id:?} slot {slot} column {column} recorded differently")));
        }
        if !fields.recorded(column) {
            continue;
        }
        let cell = table
            .column(column)
            .and_then(|values| values.cell(slot))
            .ok_or_else(|| invalid("walker column missing"))?;
        if !same(cell, &parse(kind, fields.field(column))?, strings) {
            return Err(invalid(format!("{id:?} slot {slot} column {column} differs")));
        }
    }
    Ok(())
}

/// Every slot a section wrote holds the recording's row this hour, and no row it deleted is
/// in the hour.
fn check_written(id: TableId, table: &Table, changes: &section::Changes, rows: &TableRows, strings: &[Vec<u8>], fields: &mut Fields) -> io::Result<()> {
    for &slot in changes.changed.iter().chain(&changes.inserted).chain(&changes.fill_slots) {
        let key = table.key(slot).ok_or_else(|| invalid(format!("{id:?} slot {slot} lost its row")))?;
        let index = rows.visit(Find(key)).ok_or_else(|| invalid(format!("{id:?} row {key:#x} is not in its hour")))?;
        check(id, table, rows, index, key, strings, fields)?;
    }
    if let Some(key) = changes.deleted_keys.iter().find(|&&key| rows.contains_key(key)) {
        return Err(invalid(format!("{id:?} row {key:#x} is deleted but in its hour")));
    }
    Ok(())
}

/// A table's status and sample hour at `hour` are its recording view's: a truncated table's
/// missing count, and a refusal's reason among the run's `strings`.
fn check_evidence(id: TableId, hour: i32, status: Status, sample: Option<i32>, view: &SharedView, strings: &[Vec<u8>]) -> io::Result<()> {
    let recorded = match (status, &**view) {
        (Status::Complete, TableView::Complete(_)) | (Status::NotRecorded, TableView::NotRecorded { .. }) => true,
        (Status::Truncated { missing }, TableView::Truncated { missing: recorded, .. }) => missing == *recorded,
        (Status::Unavailable { reason }, TableView::Unavailable { reason: recorded }) => {
            strings.get(reason as usize).is_some_and(|text| text.as_slice() == recorded.as_bytes())
        }
        _ => false,
    };
    if !recorded {
        return Err(invalid(format!("{id:?} at hour {hour} is {status:?}, not the recording's view")));
    }
    if sample != view.sampled_at() {
        return Err(invalid(format!("{id:?} at hour {hour} is sampled at {sample:?}, not at the recording's {:?}", view.sampled_at())));
    }
    Ok(())
}

/// Every table's status and sample hour in an hour's manifest are the recording's `views`':
/// `sampled` follows each table's sample hour from the run's manifests, as a walker does.
fn check_manifest(manifest: &Manifest<'_>, views: &[SharedView], strings: &[Vec<u8>], sampled: &mut [Option<i32>]) -> io::Result<()> {
    if manifest.statuses.len() != views.len() || sampled.len() != views.len() {
        return Err(invalid("the manifest's tables are not the recording's"));
    }
    for (table, sample) in sampled.iter_mut().enumerate() {
        if manifest.read[table / 64] >> (table % 64) & 1 != 0 {
            *sample = Some(manifest.hour);
        }
    }
    for &(table, hour) in &manifest.samples {
        *sampled.get_mut(table).ok_or_else(|| invalid("a sample names a table past the schema"))? = hour;
    }
    for (index, (&status, view)) in manifest.statuses.iter().zip(views).enumerate() {
        check_evidence(TableId::ALL[index], manifest.hour, status, sampled[index], view, strings)?;
    }
    Ok(())
}

/// One table's report totals over every run.
#[derive(Clone, Copy, Default)]
struct Totals {
    sections: u64,
    bytes: u64,
    inserts: u64,
    deletes: u64,
    changed: u64,
    census: Census,
    truncated: u64,
    /// This table's units: raw and stored bytes.
    units: (u64, u64),
    /// This table's restarts: raw bytes, and compressed alone.
    restarts: (u64, u64),
}

impl Totals {
    fn count(&mut self, hour: Census) {
        let total = &mut self.census;
        total.set += hour.set;
        total.add += hour.add;
        total.patch += hour.patch;
        total.fills += hour.fills;
        total.fill_slots += hour.fill_slots;
        total.fill_exceptions += hour.fill_exceptions;
    }
}

/// Raw and stored bytes written per kind, and dictionary entries over every run, for the
/// report.
#[derive(Default)]
struct Written {
    manifests: (u64, u64),
    chunks: (u64, u64),
    states: (u64, u64),
    entries: u64,
    restarts: u64,
}

/// One table as a walker holds it from the run's sections, and what its last section changed.
struct Walked {
    table: Table,
    changes: section::Changes,
}

/// The run being converted: its branch, last hour and ordinals so far, the recorder's writer,
/// and the walker beside it.
struct Run {
    branch: u64,
    last_hour: i32,
    ordinal: u32,
    frames: FrameWriter,
    walked: Vec<Walked>,
    /// The run's strings, in id order, from its manifests' appends.
    strings: Vec<Vec<u8>>,
    /// Each table's view at the run's last hour.
    views: Vec<SharedView>,
    /// Each table's sample hour as a walker of the run's manifests holds it.
    sampled: Vec<Option<i32>>,
}

impl Run {
    fn new(branch: u64, hour: i32, schema: &[Vec<Column>]) -> io::Result<Self> {
        Ok(Self {
            branch,
            last_hour: hour,
            ordinal: 0,
            frames: FrameWriter::new()?,
            walked: schema
                .iter()
                .map(|kinds| Ok(Walked { table: Table::new(kinds)?, changes: section::Changes::default() }))
                .collect::<io::Result<_>>()?,
            strings: Vec::new(),
            views: Vec::with_capacity(TABLE_COUNT),
            sampled: vec![None; TABLE_COUNT],
        })
    }

    /// The writer holds each table's status and sample hour as the recording's `views` give
    /// them at `hour`, so its restarts carry them too.
    fn check_committed(&self, hour: i32, views: &[SharedView]) -> io::Result<()> {
        for (index, (writer, view)) in self.frames.tables().iter().zip(views).enumerate() {
            check_evidence(TableId::ALL[index], hour, writer.status(), writer.acquisition().last_read, view, &self.strings)?;
        }
        Ok(())
    }

    /// The writer's held tables are the walker's, and the walker holds every row of each
    /// table's last view the writer took rows from, and for a complete one nothing else.
    fn check_all(&self, fields: &mut Fields) -> io::Result<()> {
        for (index, (writer, walked)) in self.frames.tables().iter().zip(&self.walked).enumerate() {
            let id = TableId::ALL[index];
            if writer.table() != &walked.table {
                return Err(invalid(format!("{id:?}'s held table is not the walker's")));
            }
            let rows = match (writer.status(), &*self.views[index]) {
                (Status::Complete, TableView::Complete(rows)) => {
                    if walked.table.live().count() != rows.len() {
                        return Err(invalid(format!("{id:?} walker holds a different row count")));
                    }
                    rows
                }
                (Status::Truncated { .. }, TableView::Truncated { rows, .. }) => rows,
                (Status::Unavailable { .. }, TableView::Unavailable { .. }) | (Status::NotRecorded, TableView::NotRecorded { .. }) => continue,
                (status, _) => return Err(invalid(format!("{id:?} is held {status:?}, not as its last view"))),
            };
            for (at, row) in rows.iter().enumerate() {
                check(id, &walked.table, rows, at, row.key(), &self.strings, fields)?;
            }
        }
        Ok(())
    }
}

/// Adds a closed position's units and restart to the report's totals.
fn account(closed: &Closed, totals: &mut [Totals], written: &mut Written) {
    for unit in &closed.units {
        let sum = if unit.stream == framing::MANIFESTS {
            &mut written.manifests
        } else {
            &mut totals[unit.stream as usize - 1].units
        };
        *sum = (sum.0 + unit.raw, sum.1 + unit.stored);
    }
    if let Some(restart) = &closed.restart {
        written.chunks = (written.chunks.0 + restart.chunk.raw, written.chunks.1 + restart.chunk.stored);
        for table in &restart.tables {
            let (raw, stored) = (table.extent.raw, table.extent.stored);
            let sum = &mut totals[table.table as usize].restarts;
            *sum = (sum.0 + raw, sum.1 + stored);
            written.states = (written.states.0 + raw, written.states.1 + stored);
        }
        written.restarts += 1;
    }
}

/// Closes the run's full unit position, with the recorder's restart when due.
fn close_position(file: &mut FileWriter<BufWriter<File>>, run: &mut Run, restart: bool, totals: &mut [Totals], written: &mut Written, fields: &mut Fields) -> io::Result<()> {
    run.check_all(fields)?;
    let closed = if restart {
        let (strings, tables) = run.frames.restart()?;
        file.close_with_restart(&strings, &tables)?
    } else {
        file.close_position()?
    };
    account(&closed, totals, written);
    Ok(())
}

/// Closes the run's last position, short or not, and writes the run's directory.
fn finish_run(file: &mut FileWriter<BufWriter<File>>, run: &Run, totals: &mut [Totals], written: &mut Written, fields: &mut Fields) -> io::Result<()> {
    run.check_all(fields)?;
    let closed = file.finish_run()?;
    account(&closed, totals, written);
    written.entries += run.strings.len() as u64;
    Ok(())
}

struct Options {
    input: PathBuf,
    output: PathBuf,
    settings: Settings,
    /// The envelope's unit hours times the restart units, which fits a `u32`.
    restart_hours: u32,
    /// The envelope's unit hours times `--run-units`: the most a run holds; a span's later
    /// hours start a run of their own.
    run_hours: Option<u32>,
}

fn options(mut args: impl Iterator<Item = OsString>) -> io::Result<Options> {
    let usage = || invalid(format!("p3-convert <recording.battleplan> <out.h4col> [--restart-units {}] [--run-units <n>]", Settings::RECORDING.restart_units));
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        return Err(usage());
    };
    let (mut restart_units, mut run_units) = (Settings::RECORDING.restart_units, None);
    while let Some(flag) = args.next() {
        let value = args.next().and_then(|value| value.into_string().ok()).ok_or_else(usage)?;
        let number = || value.parse::<i64>().map_err(|_| usage());
        match flag.to_str() {
            Some("--restart-units") => restart_units = u32::try_from(number()?).map_err(|_| usage())?,
            Some("--run-units") => run_units = Some(u32::try_from(number()?).map_err(|_| usage())?),
            _ => return Err(usage()),
        }
    }
    if restart_units == 0 || run_units == Some(0) {
        return Err(usage());
    }
    let restart_hours = envelope::UNIT_ORDINALS
        .checked_mul(restart_units)
        .ok_or_else(|| invalid("--restart-units units pass 2^32 hours"))?;
    let run_hours = run_units
        .map(|units| envelope::UNIT_ORDINALS.checked_mul(units).ok_or_else(|| invalid("--run-units units pass 2^32 hours")))
        .transpose()?;
    Ok(Options {
        input: PathBuf::from(input),
        output: PathBuf::from(output),
        settings: Settings { restart_units, ..Settings::RECORDING },
        restart_hours,
        run_hours,
    })
}

/// What one file is, whichever name or hard link reaches it.
#[cfg(windows)]
fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx};
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: an open handle and an owned FILE_ID_INFO of the size passed.
    if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo, std::ptr::from_mut(&mut info).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}

#[cfg(unix)]
fn identity(file: &File) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

/// The unfinished output and the counts file beside `output`, refused where any of them, or
/// `output` itself, is the recording at `input`: writing one would destroy the recording.
fn written_paths(input: &Path, output: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let source = identity(&File::open(input)?)?;
    let beside = |suffix: &str| {
        let mut path = output.as_os_str().to_owned();
        path.push(suffix);
        PathBuf::from(path)
    };
    let (partial, counts) = (beside(".partial"), beside(".counts.json"));
    for path in [output, &partial, &counts] {
        let reaches = match File::open(path) {
            Ok(file) => identity(&file)? == source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        if reaches {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is the recording being converted", path.display()),
            ));
        }
    }
    Ok((partial, counts))
}

/// Removes the unfinished output unless it was published.
struct Unfinished(Option<PathBuf>);

impl Drop for Unfinished {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn main() -> io::Result<()> {
    let Options { input, output, settings, restart_hours, run_hours } = options(std::env::args_os().skip(1))?;
    let source_bytes = std::fs::metadata(&input)?.len();
    let replay = Replay::open_path(&input)?;
    let (partial, counts) = written_paths(&input, &output)?;
    let mut stream = ReplayStream::new(&TableId::ALL)?;
    // Declared before `file`, so a failure closes the file before removing it.
    let mut unfinished = Unfinished(Some(partial.clone()));
    let schema = schema()?;
    let column_counts: Vec<usize> = schema.iter().map(Vec::len).collect();
    let mut content = Vec::new();
    replay_data::identity::IdentityCodec::encode(&replay.identity, &mut content)?;
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&schema),
        campaign: replay.campaign_id.clone(),
        content,
        capture_mode: replay.capture_mode.byte(),
    };
    let mut file = FileWriter::create(BufWriter::new(File::create(&partial)?), identity, column_counts, settings)?;
    let mut run: Option<Run> = None;
    let mut totals = vec![Totals::default(); TABLE_COUNT];
    let mut written = Written::default();
    let (mut scratch, mut fields) = (Scratch::default(), Fields::default());
    // Each table's complete rows the hour before, shared again while the recording leaves them unchanged.
    let mut carried: Vec<Option<Arc<TableRows>>> = vec![None; TABLE_COUNT];
    let mut changed_keys: Vec<Vec<u128>> = vec![Vec::new(); TABLE_COUNT];
    let (mut encode_seconds, mut hours) = (0.0_f64, 0_u32);
    let started = Instant::now();
    while stream.next_block_recorded_with(
        &replay,
        |recorded, decoded| {
            let begin = Instant::now();
            // A branch change or a gap ends the run, as does `--run-units`; the next starts
            // from empty tables and its own dictionary.
            let follows = run.as_ref().is_some_and(|run| run.branch == decoded.branch && run.last_hour.checked_add(1) == Some(decoded.hour));
            let continues = follows && run.as_ref().is_some_and(|run| run_hours.is_none_or(|hours| run.ordinal < hours));
            if !continues {
                // A run `--run-units` ends is continued by the next; others end at a branch change or a gap.
                let epoch = match (run.is_some(), follows) {
                    (false, _) => envelope::epoch::START,
                    (true, true) => envelope::epoch::CONTINUATION,
                    (true, false) => envelope::epoch::GAP,
                };
                if let Some(ended) = run.take() {
                    finish_run(&mut file, &ended, &mut totals, &mut written, &mut fields)?;
                }
                file.start_run(decoded.branch, epoch, decoded.hour)?;
                run = Some(Run::new(decoded.branch, decoded.hour, &schema)?);
            }
            let state = run.as_mut().expect("a run under way");
            // The hour as the recorder's frame input, and the rows each complete table's delta names.
            let tables = std::array::from_fn(|index| match &*decoded.tables[index] {
                TableView::Complete(rows) => {
                    let unchanged = matches!(recorded.tables[index], TableUpdate::Unchanged { .. });
                    let shared = match &carried[index] {
                        Some(held) if unchanged => Arc::clone(held),
                        _ => Arc::new(rows.clone()),
                    };
                    carried[index] = Some(Arc::clone(&shared));
                    TableInput::SharedComplete(shared)
                }
                view => {
                    carried[index] = None;
                    TableInput::Sampled(view.clone())
                }
            });
            let input = FrameInput {
                branch: decoded.branch,
                hour: decoded.hour,
                tables,
                metrics: decoded.metrics.stored(),
            };
            for (keys, update) in changed_keys.iter_mut().zip(recorded.tables.iter()) {
                keys.clear();
                if let TableUpdate::Delta { changed, .. } = update {
                    keys.extend(changed.iter().map(|delta| delta.key));
                }
            }
            // Each table as the recording holds it: its view's sample hour, and what its delta names.
            let sources: Vec<Source<'_>> = changed_keys
                .iter()
                .zip(recorded.tables.iter())
                .zip(decoded.tables.iter())
                .map(|((keys, update), view)| Source {
                    sample: view.sampled_at(),
                    delta: match update {
                        TableUpdate::Delta { deleted, .. } => Some(Changed { keys, removed: deleted }),
                        TableUpdate::Unchanged { .. } => Some(Changed { keys: &[], removed: &[] }),
                        _ => None,
                    },
                })
                .collect();
            let prepared = state.frames.hour_with(&input, &sources)?;
            // A block keeps which tables were read, never for how long: the manifest holds it all.
            let manifest = Manifest::read(&prepared.manifest, state.frames.columns())?;
            let mut rebuilt = Metrics {
                total_ns: manifest.total_ns,
                pool_bytes: manifest.pool_bytes,
                dropped_frames: manifest.dropped_frames,
                ..Metrics::default()
            };
            for (table, ns) in rebuilt.table_ns.iter_mut().enumerate() {
                if manifest.read[table / 64] >> (table % 64) & 1 != 0 {
                    *ns = READ_TIME_NOT_RECORDED;
                }
            }
            if rebuilt != decoded.metrics.stored() {
                return Err(invalid(format!("hour {} metrics differ from the manifest's", decoded.hour)));
            }
            state.strings.extend(manifest.appends.iter().map(|bytes| bytes.to_vec()));
            check_manifest(&manifest, &decoded.tables, &state.strings, &mut state.sampled)?;
            // The walker applies each section; every slot it wrote holds the recording's row.
            for (index, section) in prepared.sections() {
                let id = TableId::ALL[index];
                let walked = &mut state.walked[index];
                let truncated = matches!(prepared.statuses()[index], Status::Truncated { .. });
                section::apply(section, &schema[index], truncated, &mut walked.table, &mut scratch, &mut walked.changes)?;
                let rows = match &*decoded.tables[index] {
                    TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
                    _ => return Err(invalid(format!("{id:?} has a section but no rows at hour {}", decoded.hour))),
                };
                check_written(id, &walked.table, &walked.changes, rows, &state.strings, &mut fields)?;
                let (total, changes) = (&mut totals[index], &walked.changes);
                total.sections += 1;
                total.bytes += section.len() as u64;
                total.inserts += changes.inserted.len() as u64;
                total.deletes += changes.deleted.len() as u64;
                total.changed += changes.changed.len() as u64;
            }
            for (index, table) in prepared.prepared() {
                totals[index].count(table.census);
            }
            for (total, status) in totals.iter_mut().zip(prepared.statuses()) {
                if matches!(status, Status::Truncated { .. }) {
                    total.truncated += 1;
                }
            }
            let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
            let position = file.hour(FileHour {
                hour: prepared.hour(),
                manifest: &prepared.manifest,
                sections: &sections,
                statuses: prepared.statuses(),
                read: prepared.read(),
            })?;
            drop(sections);
            state.frames.commit(prepared)?;
            state.check_committed(decoded.hour, &decoded.tables)?;
            state.views.clear();
            state.views.extend(decoded.tables.iter().cloned());
            state.last_hour = decoded.hour;
            state.ordinal += 1;
            hours += 1;
            if let Position::Full { restart } = position {
                close_position(&mut file, state, restart, &mut totals, &mut written, &mut fields)?;
            }
            encode_seconds += begin.elapsed().as_secs_f64();
            Ok(())
        },
        || Ok(()),
    )? {}
    if let Some(ended) = run.take() {
        finish_run(&mut file, &ended, &mut totals, &mut written, &mut fields)?;
    }
    let Finished { file, walker_memory, index_bytes } = file.finish(|file| io::Write::flush(file))?;
    let file = file.into_inner().map_err(io::Error::from)?;
    // Durable before it is published under its name.
    file.sync_all()?;
    let total = file.metadata()?.len();
    drop(file);
    std::fs::rename(&partial, &output)?;
    unfinished.0 = None;
    let per_hour = |bytes: u64| bytes as f64 / f64::from(hours.max(1));
    let mut memory = walker_memory;
    memory.sort_unstable();
    let (walker_peak, walker_median) = (memory.last().copied().unwrap_or(0), memory.get(memory.len() / 2).copied().unwrap_or(0));
    let fifty = |bytes: u64| per_hour(bytes) * 438_000.0 / 1e9;
    let restarts = written.restarts;
    let sections = totals.iter().fold((0, 0), |sum: (u64, u64), table| (sum.0 + table.units.0, sum.1 + table.units.1));
    let restart_alone = totals.iter().fold((0, 0), |sum: (u64, u64), table| (sum.0 + table.restarts.0, sum.1 + table.restarts.1));
    let tables: Vec<serde_json::Value> = totals
        .iter()
        .enumerate()
        .filter(|(_, table)| table.sections > 0 || table.restarts.0 > 0)
        .map(|(index, table)| json!({
            "table": format!("{:?}", TableId::ALL[index]),
            "sections": table.sections,
            "section_bytes": table.bytes,
            "units_raw": table.units.0,
            "units_stored": table.units.1,
            "inserts": table.inserts,
            "deletes": table.deletes,
            "changed_slots": table.changed,
            "set_values": table.census.set,
            "add_values": table.census.add,
            "patch_values": table.census.patch,
            "fills": table.census.fills,
            "fill_slots": table.census.fill_slots,
            "fill_exceptions": table.census.fill_exceptions,
            "truncated_hours": table.truncated,
            "restart_raw": table.restarts.0,
            "restart_stored_alone": table.restarts.1,
        }))
        .collect();
    let report = json!({
        "hours": hours,
        "gate": "every written slot each hour, every live slot and the writer's held tables at every unit's and run's end, every hour's stored metrics, and every table's status, missing count, refusal reason and sample hour each hour in its manifest and its writer: no difference",
        "unit_hours": envelope::UNIT_ORDINALS,
        "restart_hours": restart_hours,
        "run_hours": run_hours,
        "level": settings.level,
        "restart_window_log": settings.window_log,
        "walker_decode_memory": {
            "budget_mb": settings.budget as f64 / 1e6,
            "peak_mb": walker_peak as f64 / 1e6,
            "median_mb": walker_median as f64 / 1e6,
            "scope": "per unit grid position, the sum over its streams of min(unit raw bytes, 2^window)",
        },
        "source_bytes": source_bytes,
        "output_bytes": total,
        "ratio_to_source": total as f64 / source_bytes as f64,
        "gb_per_50_years": fifty(total),
        "source_gb_per_50_years": fifty(source_bytes),
        "encode_and_gate_ms_per_hour": encode_seconds * 1000.0 / f64::from(hours.max(1)),
        "wall_seconds": started.elapsed().as_secs_f64(),
        "dictionary_entries": written.entries,
        "manifests": {"raw": written.manifests.0, "stored": written.manifests.1},
        "sections": {"raw": sections.0, "stored": sections.1, "stored_per_hour": per_hour(sections.1)},
        "restarts": {
            "count": restarts,
            "chunks": {"raw": written.chunks.0, "stored": written.chunks.1},
            "states": {"raw": written.states.0, "stored": written.states.1},
            "stored_per_restart": (written.chunks.1 + written.states.1) as f64 / restarts.max(1) as f64,
            "tables_alone": {"raw": restart_alone.0, "stored": restart_alone.1},
        },
        "index_bytes": index_bytes,
        "tables": tables,
    });
    std::fs::write(counts, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn arguments(text: &[&str]) -> impl Iterator<Item = OsString> {
        text.iter().map(OsString::from).collect::<Vec<_>>().into_iter()
    }

    /// A fresh folder of its own, removed afterwards.
    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("p3-convert-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_restart_interval_past_32_bits_is_refused_before_converting() {
        let most = u32::MAX / envelope::UNIT_ORDINALS;
        let (past, most_text) = ((most + 1).to_string(), most.to_string());
        let refused = options(arguments(&["in", "out", "--restart-units", &past])).err().unwrap();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
        assert!(refused.to_string().contains("--restart-units"), "{refused}");
        let largest = options(arguments(&["in", "out", "--restart-units", &most_text])).unwrap();
        assert_eq!(largest.restart_hours, most * envelope::UNIT_ORDINALS);
        assert_eq!(largest.settings, Settings { restart_units: most, ..Settings::RECORDING });
        assert!(options(arguments(&["in", "out", "--restart-units", "0"])).is_err());
        // Units are the envelope's grid.
        assert!(options(arguments(&["in", "out", "--unit-hours", "360"])).is_err());
    }

    #[test]
    fn a_conversion_writes_at_the_recorders_settings() {
        let defaults = options(arguments(&["in", "out"])).unwrap();
        assert_eq!(defaults.settings, Settings::RECORDING);
        assert_eq!(defaults.restart_hours, Settings::RECORDING.restart_units * envelope::UNIT_ORDINALS);
        for flag in ["--level", "--window-log", "--budget-mb"] {
            assert!(options(arguments(&["in", "out", flag, "3"])).is_err(), "{flag}");
        }
    }

    #[test]
    fn runs_end_only_at_branch_changes_and_gaps_unless_cut_into_units() {
        assert_eq!(options(arguments(&["in", "out"])).unwrap().run_hours, None);
        let cut = options(arguments(&["in", "out", "--run-units", "9"])).unwrap();
        assert_eq!(cut.run_hours, Some(9 * envelope::UNIT_ORDINALS));
        assert!(options(arguments(&["in", "out", "--run-units", "0"])).is_err());
        let past = (u32::MAX / envelope::UNIT_ORDINALS + 1).to_string();
        let refused = options(arguments(&["in", "out", "--run-units", &past])).err().unwrap();
        assert!(refused.to_string().contains("--run-units"), "{refused}");
    }

    #[test]
    fn no_written_path_may_be_the_recording_under_any_name() {
        let folder = Folder::new();
        let recording = folder.0.join("campaign.battleplan");
        std::fs::write(&recording, b"recording").unwrap();
        let refused = |output: &Path| written_paths(&recording, output).unwrap_err().kind() == io::ErrorKind::InvalidInput;
        assert!(refused(&recording));
        assert!(refused(&folder.0.join(".").join("campaign.battleplan")));
        let linked = folder.0.join("linked.h4col");
        std::fs::hard_link(&recording, &linked).unwrap();
        assert!(refused(&linked));
        let beside = folder.0.join("beside.h4col");
        std::fs::hard_link(&recording, folder.0.join("beside.h4col.partial")).unwrap();
        assert!(refused(&beside));
        let counted = folder.0.join("counted.h4col");
        std::fs::hard_link(&recording, folder.0.join("counted.h4col.counts.json")).unwrap();
        assert!(refused(&counted));
        let other = folder.0.join("other.h4col");
        std::fs::write(&other, b"an earlier conversion").unwrap();
        let (partial, counts) = written_paths(&recording, &other).unwrap();
        assert_eq!(partial, folder.0.join("other.h4col.partial"));
        assert_eq!(counts, folder.0.join("other.h4col.counts.json"));
        assert!(written_paths(&recording, &folder.0.join("new.h4col")).is_ok());
        assert_eq!(std::fs::read(&recording).unwrap(), b"recording");
    }

    /// The gate refuses a table whose status, missing count, refusal reason or sample hour is
    /// not its recording view's, in the manifest or as its writer holds it.
    #[test]
    fn the_gate_refuses_evidence_other_than_the_recordings() {
        let (countries, divisions) = (TableId::Countries.index(), TableId::Divisions.index());
        let strings = vec![b"refused".to_vec(), b"other".to_vec()];
        let mut views: Vec<SharedView> = (0..TABLE_COUNT)
            .map(|_| SharedView::from(TableView::Unavailable { reason: "refused".into() }))
            .collect();
        // Countries complete and unread since hour 4, divisions truncated.
        views[countries] = SharedView::from(TableView::Complete(TableRows::Countries(Vec::new()))).with_sampled_at(Some(4));
        views[divisions] = SharedView::from(TableView::Truncated { rows: TableRows::Divisions(Vec::new()), missing: 3 });
        let mut statuses = vec![Status::Unavailable { reason: 0 }; TABLE_COUNT];
        statuses[countries] = Status::Complete;
        statuses[divisions] = Status::Truncated { missing: 3 };
        let check = |statuses: &[Status], read: &[usize], samples: Vec<(usize, Option<i32>)>| {
            let mut bits = vec![0_u64; TABLE_COUNT.div_ceil(64)];
            for &table in read {
                bits[table / 64] |= 1 << (table % 64);
            }
            let manifest = Manifest {
                branch: 1,
                hour: 5,
                flags: 0,
                total_ns: 0,
                pool_bytes: 0,
                dropped_frames: 0,
                appends: Vec::new(),
                statuses: statuses.to_vec(),
                read: bits,
                samples,
                sections: Vec::new(),
            };
            // The walker last had countries sampled at hour 4.
            let mut sampled = vec![None; TABLE_COUNT];
            sampled[countries] = Some(4);
            check_manifest(&manifest, &views, &strings, &mut sampled)
        };
        check(&statuses, &[], Vec::new()).unwrap();
        check(&statuses, &[divisions], vec![(divisions, None)]).unwrap();
        let other = |table: usize, status: Status| {
            let mut statuses = statuses.clone();
            statuses[table] = status;
            statuses
        };
        assert!(check(&other(countries, Status::Truncated { missing: 1 }), &[], Vec::new()).is_err());
        assert!(check(&other(divisions, Status::Truncated { missing: 2 }), &[], Vec::new()).is_err());
        assert!(check(&other(divisions, Status::Complete), &[], Vec::new()).is_err());
        let refused = TableId::ResearchInputs.index();
        assert!(check(&other(refused, Status::Unavailable { reason: 1 }), &[], Vec::new()).is_err());
        assert!(check(&other(refused, Status::NotRecorded), &[], Vec::new()).is_err());
        // A read bit samples a table at the hour, unless a sample says otherwise.
        assert!(check(&statuses, &[countries], Vec::new()).is_err());
        assert!(check(&statuses, &[divisions], Vec::new()).is_err());
        assert!(check(&statuses, &[], vec![(countries, None)]).is_err());
        check(&statuses, &[countries], vec![(countries, Some(4))]).unwrap();
        // The writer's committed status and sample hour are checked alike.
        check_evidence(TableId::Countries, 5, Status::Complete, Some(4), &views[countries], &strings).unwrap();
        assert!(check_evidence(TableId::Countries, 5, Status::Complete, Some(3), &views[countries], &strings).is_err());
        assert!(check_evidence(TableId::Divisions, 5, Status::Unavailable { reason: 0 }, None, &views[divisions], &strings).is_err());
    }

    #[test]
    fn an_unfinished_output_is_removed_and_a_published_one_kept() {
        let folder = Folder::new();
        let failed = folder.0.join("failed.h4col.partial");
        std::fs::write(&failed, b"H4COLUMN").unwrap();
        drop(Unfinished(Some(failed.clone())));
        assert!(!failed.exists());
        let published = folder.0.join("published.h4col");
        std::fs::write(&published, b"H4COLUMN").unwrap();
        let mut unfinished = Unfinished(Some(published.clone()));
        unfinished.0 = None;
        drop(unfinished);
        assert!(published.exists());
    }
}
