//! A run's hours as columnar sections and manifests, from the frame input the collector's
//! conversion hands the row encoder today: one [`TableWriter`] per table and the run's
//! dictionary. A run is one branch's continuous hours; a new branch starts a new writer.
//!
//! Each table's sample hour follows the row decoder: a complete table read this hour is
//! sampled at it, an unread table the hour leaves alone keeps its sample, and any other view
//! (changed unread, truncated, unavailable, not recorded) has none. A staggered table's
//! off-slot refusal carries its complete rows, as the row encoder carries it between keyframes,
//! and a run's first hour refuses a truncated read, as the row keyframe a run starts with does.

use crate::{
    columnar_file::{RestartState, referenced_ids},
    frame::{FrameInput, Metrics, TRUNCATED_KEYFRAME_REASON, TableInput, TableView},
    native_columns::{self, RowCells, TableHour, TableInput as Native},
    format::FormatVersion,
    rows::{RowCodec, RowsVisit, TABLE_COUNT, TableId},
    scope::{self, CaptureMode},
};
use replay_columnar::{
    manifest::{self, Manifest, Section, Status},
    writer::{Dictionary, Sample, TableWriter},
};
use replay_data::sliced::SliceUpdate;
use std::io::{self, Write};

#[cfg(test)]
mod tests;

/// One run's held columns and dictionary.
pub struct FrameWriter {
    tables: Vec<TableWriter>,
    /// Each table's column count, as manifests are written with.
    columns: Vec<usize>,
    dictionary: Dictionary,
    cells: RowCells,
    removed: Vec<u128>,
    /// Each table the current format leaves out, with the reason a recording decodes for it.
    absent: Vec<Option<String>>,
    /// No hour of the run has committed yet.
    starting: bool,
    /// An hour failed after it interned strings: the run cannot continue.
    poisoned: bool,
    /// The run's strings committed hours appended since the last restart, in id order.
    since_restart: Vec<Box<[u8]>>,
    /// An hour is prepared and not yet committed: its strings are interned.
    pending: bool,
}

/// Every table's hour, prepared and not yet applied.
pub struct FrameHour {
    hour: i32,
    tables: Vec<TableHour>,
    statuses: Vec<Status>,
    read: Vec<u64>,
    /// The strings the hour appends to the run's dictionary, in id order.
    appends: Vec<Box<[u8]>>,
    /// The hour's manifest; its sections are [`FrameHour::sections`], in table order.
    pub manifest: Vec<u8>,
}

impl FrameHour {
    /// Each written section with its table index, in table order.
    pub fn sections(&self) -> impl Iterator<Item = (usize, &[u8])> {
        self.tables.iter().enumerate().filter_map(|(index, table)| {
            let section = table.prepared.as_ref()?.section.as_deref()?;
            Some((index, section))
        })
    }

    pub fn hour(&self) -> i32 {
        self.hour
    }

    /// Each table's status, as the manifest holds them.
    pub fn statuses(&self) -> &[Status] {
        &self.statuses
    }

    /// The read bits, as the manifest holds them.
    pub fn read(&self) -> &[u64] {
        &self.read
    }
}

impl FrameWriter {
    pub fn new() -> io::Result<Self> {
        let mut tables = Vec::new();
        engine_allocation::reserve(&mut tables, TABLE_COUNT)?;
        let mut columns = Vec::new();
        engine_allocation::reserve(&mut columns, TABLE_COUNT)?;
        let mut absent = Vec::new();
        engine_allocation::reserve(&mut absent, TABLE_COUNT)?;
        let format = FormatVersion::CURRENT.version();
        for id in TableId::ALL {
            let kinds = native_columns::columns(id.schema())?;
            columns.push(kinds.len());
            tables.push(TableWriter::new(&kinds)?);
            let change = id.last_change();
            absent.push(if change.present(format) {
                None
            } else {
                Some(
                    change
                        .table_gap(id, format)
                        .ok_or_else(|| invalid("An absent table has no catalogued gap"))?,
                )
            });
        }
        Ok(Self {
            tables,
            columns,
            dictionary: Dictionary::new(),
            cells: RowCells::default(),
            removed: Vec::new(),
            absent,
            starting: true,
            poisoned: false,
            since_restart: Vec::new(),
            pending: false,
        })
    }

    /// Each table's column count, as this run's manifests are written with.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    /// Prepares every table's hour. Nothing held changes until [`FrameWriter::commit`].
    pub fn hour(&mut self, input: &FrameInput) -> io::Result<FrameHour> {
        if self.poisoned {
            return Err(invalid("A failed hour ended this columnar run"));
        }
        // An uncommitted hour's interned strings would be lost to its successor's manifest.
        if self.pending {
            self.poisoned = true;
            return Err(invalid("A prepared hour commits before the next"));
        }
        let hour = self.prepare(input);
        self.poisoned = hour.is_err();
        self.pending = hour.is_ok();
        hour
    }

    fn prepare(&mut self, input: &FrameInput) -> io::Result<FrameHour> {
        let mut tables = Vec::new();
        engine_allocation::reserve(&mut tables, TABLE_COUNT)?;
        let mut statuses = Vec::new();
        engine_allocation::reserve(&mut statuses, TABLE_COUNT)?;
        let mut read = Vec::new();
        engine_allocation::resize_default(&mut read, TABLE_COUNT.div_ceil(64))?;
        let mut samples = Vec::new();
        let mut sections = Vec::new();
        let mode = input.capture_mode()?;
        for (index, table) in input.tables.iter().enumerate() {
            // A table the recording omits keeps no read time, as the row encoder stores none.
            let stored = self.absent[index].is_none() && scope::records(mode, TableId::ALL[index]);
            let was_read = stored && input.metrics.table_ns[index] != 0;
            if was_read {
                read[index / 64] |= 1 << (index % 64);
            }
            let mut prepared = self.table(index, table, was_read, mode)?;
            let writer = &self.tables[index];
            let changed = prepared
                .prepared
                .as_ref()
                .is_some_and(|prepared| prepared.section.is_some());
            prepared.evidence.sample = match prepared.evidence.status {
                Status::Complete if was_read || !changed => Sample::FromReadBit,
                _ => Sample::Reset,
            };
            let implied = if was_read {
                Some(input.hour)
            } else {
                writer.acquisition().last_read
            };
            if prepared.evidence.sample == Sample::Reset && implied.is_some() {
                engine_allocation::reserve(&mut samples, 1)?;
                samples.push((index, None));
            }
            statuses.push(prepared.evidence.status);
            if let Some(section) = prepared.prepared.as_ref()
                && let Some(bytes) = &section.section
            {
                engine_allocation::reserve(&mut sections, 1)?;
                sections.push(Section {
                    table: index,
                    bytes: bytes.len() as u64,
                    columns: engine_allocation::copy_vec(&section.columns)?,
                    ops: section.ops,
                });
            }
            tables.push(prepared);
        }
        let appends = self.dictionary.take_appended();
        let held_statuses = engine_allocation::copy_vec(&statuses)?;
        let held_read = engine_allocation::copy_vec(&read)?;
        let Metrics {
            total_ns,
            pool_bytes,
            dropped_frames,
            ..
        } = input.metrics;
        let manifest = Manifest {
            branch: input.branch,
            hour: input.hour,
            flags: if samples.is_empty() {
                0
            } else {
                manifest::SAMPLES
            },
            total_ns,
            pool_bytes,
            dropped_frames,
            appends: engine_allocation::collect_vec(appends.iter().map(|bytes| &**bytes))?,
            statuses,
            read,
            samples,
            sections,
        }
        .write(&self.columns)?;
        Ok(FrameHour {
            hour: input.hour,
            tables,
            statuses: held_statuses,
            read: held_read,
            appends,
            manifest,
        })
    }

    /// One table's hour from the form its conversion took.
    fn table(
        &mut self,
        index: usize,
        input: &TableInput,
        read: bool,
        mode: CaptureMode,
    ) -> io::Result<TableHour> {
        let id = TableId::ALL[index];
        if let TableInput::SliceUpdate { update, .. } = input {
            removed(id, &self.tables[index], update, &mut self.removed)?;
        }
        let writer = &self.tables[index];
        // The row encoder carries an off-slot refusal over a complete baseline from an
        // earlier hour of the branch.
        let carried = matches!(input, TableInput::Sampled(TableView::Unavailable { reason })
            if reason == replay_data::cadence::SKIPPED_REASON)
            && replay_data::cadence::is_staggered(id)
            && writer.status() == Status::Complete
            && writer.acquisition().available_from.is_some();
        let held = Visit {
            writer: &mut self.tables[index],
            dictionary: &mut self.dictionary,
            cells: &mut self.cells,
            read,
            starting: self.starting,
        };
        if let Some(reason) = &self.absent[index] {
            return held.unavailable(reason);
        }
        if !scope::records(mode, id) {
            return held.not_recorded();
        }
        match input {
            _ if carried => held.same(),
            TableInput::ComparedEqual(_) => held.same(),
            TableInput::Sampled(view) => held.view(view),
            TableInput::SharedComplete(rows) => rows.visit(held.with(Form::Complete)),
            // Membership is unchanged by the comparison's contract: nothing is removed.
            TableInput::ComparedChanges { rows, .. } => rows.visit(held.with(Form::Changes(&[]))),
            TableInput::SliceUpdate { update, .. } => {
                update.rows.visit(held.with(Form::Changes(&self.removed)))
            }
        }
    }

    /// Applies a prepared hour in table order.
    pub fn commit(&mut self, hour: FrameHour) -> io::Result<()> {
        if self.poisoned {
            return Err(invalid("A failed hour ended this columnar run"));
        }
        if let Err(error) = engine_allocation::reserve(&mut self.since_restart, hour.appends.len()) {
            self.poisoned = true;
            return Err(error);
        }
        for (writer, table) in self.tables.iter_mut().zip(hour.tables) {
            if let Err(error) = writer.commit(table.prepared, table.evidence, hour.hour) {
                self.poisoned = true;
                return Err(error);
            }
        }
        self.since_restart.extend(hour.appends);
        self.starting = false;
        self.pending = false;
        Ok(())
    }

    /// The restart a position closing after the last committed hour carries: each table's
    /// state with the dictionary ids it references, and the run's strings appended since the
    /// previous restart, which this takes.
    pub fn restart(&mut self) -> io::Result<(Vec<Box<[u8]>>, Vec<RestartState>)> {
        if self.poisoned {
            return Err(invalid("A failed hour ended this columnar run"));
        }
        if self.pending {
            return Err(invalid("A restart follows the last hour's commit"));
        }
        let mut tables = Vec::new();
        engine_allocation::reserve(&mut tables, self.tables.len())?;
        for writer in &self.tables {
            tables.push(RestartState {
                bytes: writer.restart()?,
                ids: referenced_ids(writer.table(), writer.acquisition().refusal)?,
            });
        }
        Ok((std::mem::take(&mut self.since_restart), tables))
    }
}

/// The held rows a slice update replaces and does not carry again.
fn removed(
    id: TableId,
    writer: &TableWriter,
    update: &SliceUpdate,
    out: &mut Vec<u128>,
) -> io::Result<()> {
    out.clear();
    let table = writer.table();
    let live = table.live();
    for slot in 0..table.slots() as u32 {
        if !live.get(slot) {
            continue;
        }
        let Some(key) = table.key(slot) else {
            continue;
        };
        if update.replaces(id, key) && !update.rows.contains_key(key) {
            engine_allocation::reserve(out, 1)?;
            out.push(key);
        }
    }
    out.sort_unstable();
    Ok(())
}

/// How a table's rows this hour cover its held rows.
#[derive(Clone, Copy)]
enum Form<'a> {
    Complete,
    Truncated(u32),
    Changes(&'a [u128]),
}

struct Visit<'a> {
    writer: &'a mut TableWriter,
    dictionary: &'a mut Dictionary,
    cells: &'a mut RowCells,
    read: bool,
    starting: bool,
}

impl<'a> Visit<'a> {
    fn same(self) -> io::Result<TableHour> {
        let read = self.read;
        self.write::<Never>(Native::Same {
            read,
            sample: Sample::FromReadBit,
        })
    }

    fn unavailable(self, reason: &str) -> io::Result<TableHour> {
        let read = self.read;
        self.write::<Never>(Native::Unavailable {
            reason,
            read,
            sample: Sample::Reset,
        })
    }

    fn not_recorded(self) -> io::Result<TableHour> {
        let read = self.read;
        self.write::<Never>(Native::NotRecorded {
            read,
            sample: Sample::Reset,
        })
    }

    fn view(self, view: &TableView) -> io::Result<TableHour> {
        match view {
            TableView::Complete(rows) => rows.visit(self.with(Form::Complete)),
            TableView::Truncated { missing, .. } if self.starting => {
                let mut reason = [0_u8; 64];
                let left = {
                    let mut out = &mut reason[..];
                    write!(out, "{TRUNCATED_KEYFRAME_REASON}{missing} rows not read")?;
                    out.len()
                };
                let len = reason.len() - left;
                let reason = std::str::from_utf8(&reason[..len])
                    .map_err(|_| invalid("A refusal reason is not text"))?;
                self.unavailable(reason)
            }
            TableView::Truncated { rows, missing } => rows.visit(self.with(Form::Truncated(*missing))),
            TableView::Unavailable { reason } => self.unavailable(reason),
            TableView::NotRecorded { .. } => self.not_recorded(),
        }
    }

    fn with<'f>(self, form: Form<'f>) -> Rows<'a, 'f> {
        Rows { visit: self, form }
    }

    fn write<R: RowCodec>(self, input: Native<'_, R>) -> io::Result<TableHour> {
        native_columns::table_hour(self.writer, self.dictionary, self.cells, input)
    }
}

/// A table's typed rows in one [`Form`], written when [`TableRows::visit`] hands them over.
struct Rows<'a, 'f> {
    visit: Visit<'a>,
    form: Form<'f>,
}

impl RowsVisit for Rows<'_, '_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R]) -> io::Result<TableHour> {
        let (read, sample) = (self.visit.read, Sample::FromReadBit);
        self.visit.write(match self.form {
            Form::Complete => Native::Snapshot {
                rows,
                partial: None,
                read,
                sample,
            },
            Form::Truncated(missing) => Native::Snapshot {
                rows,
                partial: Some(missing),
                read,
                sample,
            },
            Form::Changes(removed) => Native::Changes {
                rows,
                removed,
                read,
                sample,
            },
        })
    }
}

/// The row type of an hour that visits no rows.
type Never = replay_data::rows::ClockRow;

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}
