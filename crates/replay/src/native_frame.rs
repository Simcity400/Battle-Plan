//! A run's hours as columnar sections and manifests, from the frame input the collector's
//! conversion hands the row encoder today: one [`TableWriter`] per table and the run's
//! dictionary. A run is one branch's continuous hours; a new branch starts a new writer.
//!
//! Each table's sample hour follows the row decoder: a complete table read this hour is
//! sampled at it, an unread table the hour leaves alone keeps its sample, and any other view
//! (changed unread, truncated, unavailable, not recorded) has none. A staggered table's
//! off-slot refusal carries its complete rows, as the row encoder carries it between keyframes,
//! and a truncated read is refused where the row encoder writes a keyframe: at the hours
//! [`RowFrame::keyframe`] names, and otherwise at a run's first hour.
//!
//! A recorded hour ([`FrameWriter::hour_with`]) writes each table it names as the recording
//! decodes it, wherever the run starts: at the recording's sample hour, a truncated view still
//! truncated, and a refusal not carried.
//!
//! An hour's tables may be prepared and committed on several threads at once ([`Lanes`]). The
//! lanes read the run's dictionary without adding to it; a table that would add a string, or
//! fails, is prepared again on the calling thread in table order. Tables a lane prepared added
//! no string, so every id is the one a single thread assigns, and so are the bytes.

use crate::{
    columnar_file::{RestartState, referenced_ids},
    columnar_frames::LAST_COMPLETE,
    frame::{FrameEncoder, FrameInput, Metrics, TRUNCATED_KEYFRAME_REASON, TableInput, TableView},
    native_columns::{self, RowCells, TableHour, TableInput as Native},
    format::FormatVersion,
    rows::{ChangedRows, RowCodec, RowsPairVisit, RowsVisit, TABLE_COUNT, TableId, TableRows},
    scope::{self, CaptureMode},
};
use replay_columnar::{
    manifest::{self, Manifest, Section, Status},
    restart::Last,
    writer::{Dictionary, Prepared, Sample, Strings, Sweep, TableWriter},
};
use replay_data::{BranchId, sliced::SliceUpdate};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicI32, AtomicUsize, Ordering},
    },
    time::Instant,
};

/// Hours a table found densely changed is written complete before it is diffed again.
const DENSE_HOURS: i32 = 24;

#[cfg(test)]
mod tests;

/// What the row encoder makes of the hour the frame writer prepares beside it.
#[derive(Clone, Copy)]
pub struct RowFrame<'a> {
    /// The encoder before it commits the hour: its baselines are the rows the run's tables
    /// hold once both committed the hour before, which complete tables are diffed with.
    pub baselines: Option<&'a FrameEncoder>,
    /// The encoder writes the hour as a keyframe, which refuses a truncated read.
    pub keyframe: bool,
    /// The complete tables the encoder wrote as changes from its baselines, with the rows those
    /// changes wrote (`PreparedFrame::changed_rows`): taken with the baselines, they are the
    /// table's diff.
    pub changed: &'a [(usize, ChangedRows)],
}

/// Threads an hour's tables are prepared and committed on at once.
pub trait Lanes: Sync {
    /// The threads [`Lanes::run`] runs a job on, the caller's included.
    fn count(&self) -> usize;
    /// Runs `job` once on each of them and returns when every run has finished.
    fn run(&self, job: &(dyn Fn() + Sync));
    /// Whether the calling lane takes another table. A lane that stops leaves the rest to
    /// the others; the caller finishes whatever no lane took.
    fn proceeds(&self) -> bool {
        true
    }
}

/// The calling thread alone.
pub struct OneThread;

impl Lanes for OneThread {
    fn count(&self) -> usize {
        1
    }

    fn run(&self, job: &(dyn Fn() + Sync)) {
        job();
    }
}

/// One thread's buffers for the tables it prepares.
struct Lane {
    cells: RowCells,
    /// The held keys an hour removes.
    removed: Vec<u128>,
    /// A diff's changed rows, each with its previous row's index.
    pairs: Vec<(u32, Option<u32>)>,
}

/// One run's held columns and dictionary.
pub struct FrameWriter {
    tables: Vec<TableWriter>,
    /// Each table's column count, as manifests are written with.
    columns: Vec<usize>,
    dictionary: Dictionary,
    /// Each lane's buffers; the first is the calling thread's.
    lanes: Vec<Lane>,
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
    /// Each table's wall preparing and committing its hours, in nanoseconds.
    table_ns: Vec<u64>,
    /// Each table's shared complete rows its last committed hour held, kept alive so the
    /// same allocation arriving again is known to be the same rows.
    shared: Vec<Option<Arc<TableRows>>>,
    /// Each table's hour a lane prepared, until the hour is assembled in table order.
    prepared: Vec<Option<io::Result<TableHour>>>,
    /// Each table's wall in its last hour: lanes take the costliest tables first.
    costs: Vec<u64>,
    /// Each table's wall on a lane in the hour's current step.
    walls: Vec<u64>,
    /// Every table index, costliest first.
    order: Vec<usize>,
    /// Each table's hours a lane prepared that were prepared again in table order.
    retried: Vec<u64>,
    /// Each owner-sliced table's held keys by owner, while its last hour was a slice update of
    /// owners or left its rows as they were.
    owners: Vec<Option<Owners>>,
    /// The branch and hour of the last committed hour.
    last: Option<(BranchId, i32)>,
    /// Each table's first hour diffed again after it was found densely changed.
    dense: Vec<AtomicI32>,
}

/// A table's held keys by owner: owners ascending, each owner's keys ascending.
type Owners = Vec<(u32, Vec<u128>)>;

/// An owner slice update's owners and its rows' keys, ascending, which its commit holds.
struct Slice {
    owners: Vec<u32>,
    keys: Vec<u128>,
}

/// Every table's hour, prepared and not yet applied.
pub struct FrameHour {
    branch: BranchId,
    hour: i32,
    tables: Vec<TableHour>,
    /// Each table's shared complete rows, which the commit keeps.
    shared: Vec<Option<Arc<TableRows>>>,
    /// Each table's owner slice update, whose keys the commit holds by owner.
    slices: Vec<Option<Slice>>,
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

    /// Each table's preparation (its section, manifest entry and census) with its table
    /// index, in table order.
    pub fn prepared(&self) -> impl Iterator<Item = (usize, &Prepared)> {
        self.tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| Some((index, table.prepared.as_ref()?)))
    }
}

/// What a recording's delta names of a complete table's hour: the keys of the rows it
/// changed or added, and the keys it removed.
#[derive(Clone, Copy, Debug)]
pub struct Changed<'a> {
    pub keys: &'a [u128],
    pub removed: &'a [u128],
}

/// What a recording holds of one table's hour beside its view.
#[derive(Clone, Copy, Debug, Default)]
pub struct Source<'a> {
    /// The view's sample hour, as the recording decodes it.
    pub sample: Option<i32>,
    /// What its delta names, for a complete table it stores as one.
    pub delta: Option<Changed<'a>>,
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
            let mut writer = TableWriter::new(&kinds)?;
            if LAST_COMPLETE.contains(&id) {
                writer.keep_last();
            }
            tables.push(writer);
            let change = id.last_change();
            absent.push(if change.present(format) {
                None
            } else {
                let gap = change
                    .gap(id, format)
                    .ok_or_else(|| invalid("An absent table has no catalogued gap"))?;
                Some(replay_data::codec::try_format(format_args!("{gap}"))?)
            });
        }
        let mut lanes = Vec::new();
        engine_allocation::reserve(&mut lanes, 1)?;
        lanes.push(Lane {
            cells: RowCells::default(),
            removed: Vec::new(),
            pairs: Vec::new(),
        });
        Ok(Self {
            tables,
            columns,
            dictionary: Dictionary::new(),
            lanes,
            absent,
            starting: true,
            poisoned: false,
            since_restart: Vec::new(),
            pending: false,
            table_ns: engine_allocation::filled(TABLE_COUNT, 0)?,
            shared: engine_allocation::default_vec(TABLE_COUNT)?,
            prepared: engine_allocation::default_vec(TABLE_COUNT)?,
            costs: engine_allocation::filled(TABLE_COUNT, 0)?,
            walls: engine_allocation::filled(TABLE_COUNT, 0)?,
            order: engine_allocation::collect_vec(0..TABLE_COUNT)?,
            retried: engine_allocation::filled(TABLE_COUNT, 0)?,
            owners: engine_allocation::default_vec(TABLE_COUNT)?,
            last: None,
            dense: engine_allocation::collect_vec(
                (0..TABLE_COUNT).map(|_| AtomicI32::new(i32::MIN)),
            )?,
        })
    }

    /// Each table's column count, as this run's manifests are written with.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    /// Each table's wall preparing and committing the run's hours, in nanoseconds.
    pub fn table_ns(&self) -> &[u64] {
        &self.table_ns
    }

    /// Each table's hours a lane prepared that were prepared again in table order.
    pub fn retried(&self) -> &[u64] {
        &self.retried
    }

    /// Prepares every table's hour. Nothing held changes until [`FrameWriter::commit`].
    pub fn hour(&mut self, input: &FrameInput) -> io::Result<FrameHour> {
        self.hour_on(input, &[], None, &OneThread)
    }

    fn prepare(
        &mut self,
        input: &FrameInput,
        recorded: &[Source<'_>],
        row: Option<RowFrame<'_>>,
        lanes: &dyn Lanes,
    ) -> io::Result<FrameHour> {
        let mode = input.capture_mode()?;
        let keyframe = row.map_or(self.starting, |row| row.keyframe);
        let changed = row.map_or(&[][..], |row| row.changed);
        // The encoder's baselines are the rows this run's tables hold once both committed the
        // hour before this one.
        let follows = self.last.is_some_and(|(branch, hour)| {
            branch == input.branch && hour.checked_add(1) == Some(input.hour)
        });
        let mut previous = Vec::new();
        if let Some(RowFrame {
            baselines: Some(encoder),
            ..
        }) = row
            && follows
            && encoder.last_committed() == self.last
        {
            engine_allocation::reserve(&mut previous, TABLE_COUNT)?;
            previous.extend(TableId::ALL.iter().map(|&id| encoder.committed_rows(id)));
        }
        let mut read = Vec::new();
        engine_allocation::resize_default(&mut read, TABLE_COUNT.div_ceil(64))?;
        for (index, id) in TableId::ALL.into_iter().enumerate() {
            // A table the recording omits keeps no read time, as the row encoder stores none.
            let stored = self.absent[index].is_none() && scope::records(mode, id);
            if stored && input.metrics.table_ns[index] != 0 {
                read[index / 64] |= 1 << (index % 64);
            }
        }
        if lanes.count() > 1 {
            self.side_by_side(input, recorded, &previous, changed, keyframe, &read, mode, lanes)?;
        }
        let mut tables = Vec::new();
        engine_allocation::reserve(&mut tables, TABLE_COUNT)?;
        let mut shared = Vec::new();
        engine_allocation::reserve(&mut shared, TABLE_COUNT)?;
        let mut slices = Vec::new();
        engine_allocation::reserve(&mut slices, TABLE_COUNT)?;
        let mut statuses = Vec::new();
        engine_allocation::reserve(&mut statuses, TABLE_COUNT)?;
        let mut samples = Vec::new();
        let mut sections = Vec::new();
        let Self {
            tables: writers,
            dictionary,
            lanes: buffers,
            absent,
            shared: held,
            starting,
            prepared: on_lanes,
            costs,
            walls,
            table_ns,
            retried,
            owners,
            dense,
            ..
        } = self;
        let part = Part {
            input,
            recorded,
            previous: &previous,
            changed,
            read: &read,
            mode,
            absent,
            held,
            owners,
            dense,
            starting: *starting,
            keyframe,
        };
        for (index, table) in input.tables.iter().enumerate() {
            let started = Instant::now();
            let on_lane = on_lanes[index].take();
            let lane_wall = if on_lane.is_some() { walls[index] } else { 0 };
            if matches!(on_lane, Some(Err(_))) {
                retried[index] += 1;
            }
            let mut prepared = match on_lane {
                Some(Ok(hour)) => hour,
                _ => part.table(
                    index,
                    &mut writers[index],
                    Strings::Interning(&mut *dictionary),
                    &mut buffers[0],
                )?,
            };
            let was_read = read[index / 64] & (1 << (index % 64)) != 0;
            let writer = &writers[index];
            let changed = prepared
                .prepared
                .as_ref()
                .is_some_and(|prepared| prepared.section.is_some());
            let implied = if was_read {
                Some(input.hour)
            } else {
                writer.acquisition().last_read
            };
            // A row keyframe writes every table anew, so an unread one keeps no sample.
            let sample = match recorded.get(index) {
                Some(source) => source.sample,
                None => match prepared.evidence.status {
                    Status::Complete if was_read || (!changed && !keyframe) => implied,
                    _ => None,
                },
            };
            prepared.evidence.sample = if sample == implied {
                Sample::FromReadBit
            } else {
                engine_allocation::reserve(&mut samples, 1)?;
                samples.push((index, sample));
                sample.map_or(Sample::Reset, Sample::Carried)
            };
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
            shared.push(match table {
                TableInput::SharedComplete(rows) => Some(Arc::clone(rows)),
                _ => None,
            });
            slices.push(match table {
                TableInput::SliceUpdate { update, .. } if owner_slice(update) => Some(Slice {
                    owners: engine_allocation::copy_vec(update.owners())?,
                    keys: engine_allocation::collect_vec(
                        update.rows().iter().map(|row| row.key()),
                    )?,
                }),
                _ => None,
            });
            let wall = lane_wall + started.elapsed().as_nanos() as u64;
            costs[index] = wall;
            table_ns[index] += wall;
        }
        let appends = dictionary.take_appended();
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
            branch: input.branch,
            hour: input.hour,
            tables,
            shared,
            slices,
            statuses: held_statuses,
            read: held_read,
            appends,
            manifest,
        })
    }

    /// Prepares the hour's tables on `lanes`, the costliest first, with the dictionary read
    /// only; each outcome waits in `prepared` for the hour's assembly.
    fn side_by_side(
        &mut self,
        input: &FrameInput,
        recorded: &[Source<'_>],
        previous: &[Option<&TableRows>],
        changed: &[(usize, ChangedRows)],
        keyframe: bool,
        read: &[u64],
        mode: CaptureMode,
        lanes: &dyn Lanes,
    ) -> io::Result<()> {
        while self.lanes.len() < lanes.count() {
            engine_allocation::reserve(&mut self.lanes, 1)?;
            let staging = self.lanes[0].cells.staging();
            self.lanes.push(Lane {
                cells: RowCells::new(staging),
                removed: Vec::new(),
                pairs: Vec::new(),
            });
        }
        self.costliest_first();
        let Self {
            tables: writers,
            dictionary,
            lanes: buffers,
            absent,
            shared: held,
            owners,
            dense,
            starting,
            prepared,
            walls,
            order,
            ..
        } = self;
        let part = Part {
            input,
            recorded,
            previous,
            changed,
            read,
            mode,
            absent,
            held,
            owners,
            dense,
            starting: *starting,
            keyframe,
        };
        let turns = engine_allocation::collect_vec(
            writers
                .iter_mut()
                .zip(prepared.iter_mut())
                .zip(walls.iter_mut())
                .map(|((writer, hour), wall)| Mutex::new((writer, hour, wall))),
        )?;
        let buffers = engine_allocation::collect_vec(buffers.iter_mut().map(Mutex::new))?;
        let (next, lane) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let (dictionary, order) = (&*dictionary, &**order);
        let job = || {
            let Some(buffers) = buffers.get(lane.fetch_add(1, Ordering::Relaxed)) else {
                return;
            };
            let mut buffers = lock(buffers);
            while lanes.proceeds() {
                let Some(&index) = order.get(next.fetch_add(1, Ordering::Relaxed)) else {
                    break;
                };
                let mut turn = lock(&turns[index]);
                let (writer, hour, wall) = &mut *turn;
                let started = Instant::now();
                **hour = Some(part.table(
                    index,
                    writer,
                    Strings::Reading(dictionary),
                    &mut buffers,
                ));
                **wall = started.elapsed().as_nanos() as u64;
            }
        };
        lanes.run(&job);
        Ok(())
    }

    /// Orders the tables by their last hour's wall, the costliest first.
    fn costliest_first(&mut self) {
        let costs = &self.costs;
        self.order
            .sort_unstable_by_key(|&index| std::cmp::Reverse(costs[index]));
    }

    /// Applies a prepared hour.
    pub fn commit(&mut self, hour: FrameHour) -> io::Result<()> {
        self.commit_on(hour, &OneThread)
    }

    /// Applies a prepared hour, its tables on `lanes`.
    pub fn commit_on(&mut self, hour: FrameHour, lanes: &dyn Lanes) -> io::Result<()> {
        if self.poisoned {
            return Err(invalid("A failed hour ended this columnar run"));
        }
        if let Err(error) = engine_allocation::reserve(&mut self.since_restart, hour.appends.len()) {
            self.poisoned = true;
            return Err(error);
        }
        let FrameHour {
            branch,
            hour: at,
            tables: hours,
            shared,
            slices,
            appends,
            ..
        } = hour;
        if let Err(error) = self.apply(at, hours, shared, slices, lanes) {
            self.poisoned = true;
            return Err(error);
        }
        self.last = Some((branch, at));
        self.since_restart.extend(appends);
        self.starting = false;
        self.pending = false;
        Ok(())
    }

    /// Commits each table's hour and holds its shared rows; the rows they replace are freed
    /// on the lane that commits them.
    fn apply(
        &mut self,
        at: i32,
        hours: Vec<TableHour>,
        shared: Vec<Option<Arc<TableRows>>>,
        slices: Vec<Option<Slice>>,
        lanes: &dyn Lanes,
    ) -> io::Result<()> {
        let Self {
            tables: writers,
            shared: held,
            owners,
            walls,
            order,
            costs,
            table_ns,
            ..
        } = self;
        let turns = engine_allocation::collect_vec(
            writers
                .iter_mut()
                .zip(held.iter_mut().zip(owners.iter_mut()))
                .zip(walls.iter_mut())
                .zip(hours.into_iter().zip(shared).zip(slices))
                .map(|(((writer, held), wall), hour)| Mutex::new((writer, held, wall, Some(hour)))),
        )?;
        let failed = Mutex::new(None);
        let next = AtomicUsize::new(0);
        let order = &**order;
        let commit = |index: usize| {
            let mut turn = lock(&turns[index]);
            let (writer, (held, owners), wall, hour) = &mut *turn;
            let Some(((hour, shared), slice)) = hour.take() else {
                return;
            };
            let started = Instant::now();
            let kept = hour.prepared.is_none() && hour.evidence.status == Status::Complete;
            let committed = writer.commit(hour.prepared, hour.evidence, at).and_then(|()| {
                **held = shared;
                let id = TableId::ALL[index];
                **owners = match (slice, owners.take()) {
                    (Some(slice), Some(mut known)) => {
                        hold_slice(&mut known, &slice, id)?;
                        Some(known)
                    }
                    (Some(_), None) => Some(held_owners(&**writer, id)?),
                    (None, known) => known.filter(|_| kept),
                };
                Ok(())
            });
            if let Err(error) = committed {
                lock(&failed).get_or_insert(error);
            }
            **wall = started.elapsed().as_nanos() as u64;
        };
        if lanes.count() > 1 {
            lanes.run(&|| {
                while lanes.proceeds() {
                    let Some(&index) = order.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    commit(index);
                }
            });
        }
        // Whatever no lane took.
        while let Some(&index) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
            commit(index);
        }
        drop(turns);
        if let Some(error) = failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
            return Err(error);
        }
        for ((cost, total), wall) in costs.iter_mut().zip(table_ns.iter_mut()).zip(walls.iter()) {
            *cost += wall;
            *total += wall;
        }
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
            let mut ids = referenced_ids(writer.table(), writer.acquisition().refusal)?;
            if let Some(Last::Apart(apart, _)) = writer.last() {
                let more = referenced_ids(apart, None)?;
                engine_allocation::reserve(&mut ids, more.len())?;
                ids.extend(more);
            }
            tables.push(RestartState {
                bytes: writer.restart()?,
                ids,
            });
        }
        Ok((std::mem::take(&mut self.since_restart), tables))
    }

    /// [`FrameWriter::hour`] of a recorded hour: each table `recorded` names at its index is
    /// written as the recording decodes it. It is sampled at its [`Source::sample`], at a
    /// run's start too; a truncated view stays truncated at a run's start, whose rows the
    /// recording's run read; and a refusal is not carried. A complete table whose delta it
    /// names changed only the rows the delta names, so only those are compared with the held
    /// table; a run's first hour, and a table whose last hour was not complete, compare whole.
    pub fn hour_with(
        &mut self,
        input: &FrameInput,
        recorded: &[Source<'_>],
    ) -> io::Result<FrameHour> {
        self.hour_on(input, recorded, None, &OneThread)
    }

    /// [`FrameWriter::hour_with`], its tables prepared on `lanes`. A complete table is diffed
    /// with the rows `committed` holds for it when they are the run's last committed hour's.
    pub fn hour_on(
        &mut self,
        input: &FrameInput,
        recorded: &[Source<'_>],
        row: Option<RowFrame<'_>>,
        lanes: &dyn Lanes,
    ) -> io::Result<FrameHour> {
        if self.poisoned {
            return Err(invalid("A failed hour ended this columnar run"));
        }
        // An uncommitted hour's interned strings would be lost to its successor's manifest.
        if self.pending {
            self.poisoned = true;
            return Err(invalid("A prepared hour commits before the next"));
        }
        let hour = self.prepare(input, recorded, row, lanes);
        self.poisoned = hour.is_err();
        self.pending = hour.is_ok();
        hour
    }

    /// Each table's writer, holding what the run's committed hours left.
    pub fn tables(&self) -> &[TableWriter] {
        &self.tables
    }

    /// A run's writer handing each table's rows to its writer as `staging` says;
    /// [`FrameWriter::new`] stages them by column.
    pub fn with_staging(staging: native_columns::Staging) -> io::Result<Self> {
        let mut writer = Self::new()?;
        writer.lanes[0].cells = RowCells::new(staging);
        Ok(writer)
    }

    /// A run's writer whose tables' hours clear and finish the columns `sweep` says;
    /// [`FrameWriter::new`] sweeps only those an hour touched.
    pub fn with_sweep(sweep: Sweep) -> io::Result<Self> {
        let mut writer = Self::new()?;
        for table in &mut writer.tables {
            table.set_sweep(sweep);
        }
        Ok(writer)
    }
}

/// What every table of an hour reads, shared by the lanes preparing them.
struct Part<'a> {
    input: &'a FrameInput,
    recorded: &'a [Source<'a>],
    /// Each table's rows at the run's last committed hour, when the encoder lent them.
    previous: &'a [Option<&'a TableRows>],
    /// The rows the encoder's diff of a complete table with those rows wrote.
    changed: &'a [(usize, ChangedRows)],
    /// The manifest's read bits.
    read: &'a [u64],
    mode: CaptureMode,
    absent: &'a [Option<String>],
    /// Each table's shared rows the last committed hour held.
    held: &'a [Option<Arc<TableRows>>],
    owners: &'a [Option<Owners>],
    dense: &'a [AtomicI32],
    starting: bool,
    /// A truncated read is refused this hour, as the row keyframe refuses it.
    keyframe: bool,
}

impl Part<'_> {
    /// One table's hour from the form its conversion took.
    fn table(
        &self,
        index: usize,
        writer: &mut TableWriter,
        strings: Strings<'_>,
        lane: &mut Lane,
    ) -> io::Result<TableHour> {
        let id = TableId::ALL[index];
        let input = &self.input.tables[index];
        let read = self.read[index / 64] & (1 << (index % 64)) != 0;
        let source = self.recorded.get(index).copied();
        // The rows the held table holds, and the rows the row encoder's diff with them wrote.
        let known = !self.starting && writer.status() == Status::Complete;
        let previous = self.previous.get(index).copied().flatten().filter(|_| known);
        let written = previous.and_then(|_| {
            let at = self.changed.binary_search_by_key(&index, |(table, _)| *table).ok()?;
            Some(&self.changed[at].1)
        });
        if let TableInput::SliceUpdate { update, .. } = input
            && written.is_none()
        {
            removed(id, writer, update, self.owners[index].as_ref(), &mut lane.removed)?;
        }
        // The row encoder carries an off-slot refusal over a complete baseline from an
        // earlier hour of the branch; a recorded refusal is one it did not carry.
        let carried = source.is_none()
            && matches!(input, TableInput::Sampled(TableView::Unavailable { reason })
                if reason == replay_data::cadence::SKIPPED_REASON)
            && replay_data::cadence::is_staggered(id)
            && writer.status() == Status::Complete
            && writer.acquisition().available_from.is_some();
        // A delta changes the complete rows the run's last hour held.
        let delta = source
            .and_then(|source| source.delta)
            .filter(|_| !self.starting && writer.status() == Status::Complete);
        let held = Visit {
            writer,
            strings,
            cells: &mut lane.cells,
            read,
            // The recording's run need not start here: its truncated view stays truncated.
            keyframe: self.keyframe && source.is_none(),
        };
        if let Some(reason) = &self.absent[index] {
            return held.unavailable(reason);
        }
        if !scope::records(self.mode, id) {
            return held.not_recorded();
        }
        let complete = match input {
            TableInput::Sampled(TableView::Complete(rows)) => Some(rows),
            TableInput::SharedComplete(rows) => Some(&**rows),
            _ => None,
        };
        if let (Some(rows), Some(delta)) = (complete, delta) {
            if delta.keys.is_empty() && delta.removed.is_empty() {
                return held.same();
            }
            return rows.visit(held.with(Form::Keyed(delta)));
        }
        if let TableInput::Sampled(TableView::Complete(rows)) = input
            && let Some(last) = previous
        {
            return match written {
                Some(written) => Self::written(rows, last, held, written),
                None => self.diff(index, rows, last, held, &mut lane.pairs, &mut lane.removed),
            };
        }
        match input {
            _ if carried => held.same(),
            TableInput::ComparedEqual(_) => held.same(),
            TableInput::Sampled(view) => held.view(view),
            // The rows the last committed hour held, so the table is unchanged; the row encoder
            // reuses its equality for the same allocation too.
            TableInput::SharedComplete(rows)
                if self.held[index]
                    .as_ref()
                    .is_some_and(|last| Arc::ptr_eq(last, rows)) =>
            {
                held.same()
            }
            // A re-read of the rows the last committed hour held: only the rows that differ.
            TableInput::SharedComplete(rows) => {
                if let (Some(last), Some(written)) = (previous, written) {
                    return Self::written(rows, last, held, written);
                }
                let last = self.held[index].as_deref().filter(|_| known).or(previous);
                match last {
                    Some(last) => {
                        self.diff(index, rows, last, held, &mut lane.pairs, &mut lane.removed)
                    }
                    None => rows.visit(held.with(Form::Complete)),
                }
            }
            // Membership is unchanged by the comparison's contract: nothing is removed. Each
            // changed row's index is its baseline's, so it pairs with its previous row.
            TableInput::ComparedChanges { rows, indices, .. } => match previous {
                Some(last) => match rows.visit_pair(
                    last,
                    Compared {
                        held,
                        indices,
                        pairs: &mut lane.pairs,
                    },
                ) {
                    Ok(hour) => hour,
                    Err(Compared { held, .. }) => rows.visit(held.with(Form::Changes(&[]))),
                },
                None => rows.visit(held.with(Form::Changes(&[]))),
            },
            TableInput::SliceUpdate { update, .. } => match (previous, written) {
                (Some(last), Some(written)) => Self::written(&update.rows, last, held, written),
                _ => update.rows.visit(held.with(Form::Changes(&lane.removed))),
            },
        }
    }
}

impl Part<'_> {
    /// `rows` as the changes the row encoder's diff with `last`, the rows the held table
    /// holds, wrote.
    fn written(
        rows: &TableRows,
        last: &TableRows,
        held: Visit<'_>,
        written: &ChangedRows,
    ) -> io::Result<TableHour> {
        match rows.visit_pair(last, Written { held, written }) {
            Ok(hour) => hour,
            Err(Written { held, .. }) => rows.visit(held.with(Form::Complete)),
        }
    }

    /// `rows` as the changes from `last`, the rows the held table holds, in the lane's
    /// buffers.
    fn diff(
        &self,
        index: usize,
        rows: &TableRows,
        last: &TableRows,
        held: Visit<'_>,
        pairs: &mut Vec<(u32, Option<u32>)>,
        removed: &mut Vec<u128>,
    ) -> io::Result<TableHour> {
        let diff = Diff {
            held,
            hour: self.input.hour,
            dense: &self.dense[index],
            pairs,
            removed,
        };
        match rows.visit_pair(last, diff) {
            Ok(hour) => hour,
            Err(Diff { held, .. }) => rows.visit(held.with(Form::Complete)),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `update` replaces whole owners' rows, and nothing else.
fn owner_slice(update: &SliceUpdate) -> bool {
    update.ranges.is_empty() && update.keys.is_empty()
}

/// The owner a key of table `id` belongs to, as slice updates name it.
fn owner_of(id: TableId, key: u128) -> u32 {
    (key >> id.key_bits().saturating_sub(32)) as u32
}

/// The held table's keys by owner.
fn held_owners(writer: &TableWriter, id: TableId) -> io::Result<Owners> {
    let table = writer.table();
    let mut keys = Vec::new();
    for slot in 0..table.slots() as u32 {
        if let Some(key) = table.key(slot) {
            engine_allocation::reserve(&mut keys, 1)?;
            keys.push(key);
        }
    }
    keys.sort_unstable();
    let mut owners = Vec::new();
    for run in keys.chunk_by(|a, b| owner_of(id, *a) == owner_of(id, *b)) {
        engine_allocation::reserve(&mut owners, 1)?;
        owners.push((owner_of(id, run[0]), engine_allocation::copy_vec(run)?));
    }
    Ok(owners)
}

/// Holds a committed owner slice update: each of its owners now holds exactly its rows' keys.
fn hold_slice(owners: &mut Owners, slice: &Slice, id: TableId) -> io::Result<()> {
    let mut keys = &slice.keys[..];
    for &owner in &slice.owners {
        let (own, rest) = keys.split_at(keys.partition_point(|&key| owner_of(id, key) <= owner));
        keys = rest;
        match owners.binary_search_by_key(&owner, |(held, _)| *held) {
            Ok(at) => {
                let held = &mut owners[at].1;
                held.clear();
                engine_allocation::reserve(held, own.len())?;
                held.extend_from_slice(own);
            }
            Err(at) => {
                engine_allocation::reserve(owners, 1)?;
                owners.insert(at, (owner, engine_allocation::copy_vec(own)?));
            }
        }
    }
    Ok(())
}

/// The held rows a slice update replaces and does not carry again: from the held keys of its
/// owners when they are known, otherwise from every held row.
fn removed(
    id: TableId,
    writer: &TableWriter,
    update: &SliceUpdate,
    owners: Option<&Owners>,
    out: &mut Vec<u128>,
) -> io::Result<()> {
    out.clear();
    if let Some(owners) = owners.filter(|_| owner_slice(update)) {
        // Owners ascending and each owner's keys ascending: the keys come out in order.
        for &owner in update.owners() {
            let Ok(at) = owners.binary_search_by_key(&owner, |(held, _)| *held) else {
                continue;
            };
            for &key in &owners[at].1 {
                if !update.rows.contains_key(key) {
                    engine_allocation::reserve(out, 1)?;
                    out.push(key);
                }
            }
        }
        return Ok(());
    }
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
    Keyed(Changed<'a>),
}

struct Visit<'a> {
    writer: &'a mut TableWriter,
    strings: Strings<'a>,
    cells: &'a mut RowCells,
    read: bool,
    /// A truncated read is refused, as the row keyframe refuses it.
    keyframe: bool,
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
            TableView::Truncated { missing, .. } if self.keyframe => {
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

    /// Every row of a complete table at its place in `previous`, the rows the held table holds.
    fn aligned<R: RowCodec>(self, rows: &[R], previous: &[R]) -> io::Result<TableHour> {
        let read = self.read;
        self.write(Native::Aligned {
            rows,
            previous,
            read,
            sample: Sample::FromReadBit,
        })
    }

    /// The rows `pairs` names, each beside its previous row when it has one; `removed` are gone.
    fn paired<R: RowCodec>(
        self,
        rows: &[R],
        previous: &[R],
        pairs: &[(u32, Option<u32>)],
        removed: &[u128],
    ) -> io::Result<TableHour> {
        let read = self.read;
        self.write(Native::Paired {
            rows,
            previous,
            pairs,
            removed,
            read,
            sample: Sample::FromReadBit,
        })
    }

    fn write<R: RowCodec>(self, input: Native<'_, R>) -> io::Result<TableHour> {
        native_columns::table_hour(self.writer, self.strings, self.cells, input)
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
            Form::Keyed(Changed { keys, removed }) => Native::Keyed {
                rows,
                keys,
                removed,
                read,
                sample,
            },
        })
    }
}

/// A complete table against the rows the held table holds, both in key order: the rows that
/// changed or came, each beside its previous row, and the keys of those that went, which write
/// what the complete rows write. Rows out of key order write the complete table. A table
/// changed in more than three quarters of its rows is not diffed for [`DENSE_HOURS`]; it
/// writes every row beside its previous one while its rows keep their places, otherwise the
/// complete table.
struct Diff<'a> {
    held: Visit<'a>,
    hour: i32,
    dense: &'a AtomicI32,
    pairs: &'a mut Vec<(u32, Option<u32>)>,
    removed: &'a mut Vec<u128>,
}

impl RowsPairVisit for Diff<'_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R], last: &[R]) -> io::Result<TableHour> {
        let Self {
            held,
            hour,
            dense,
            pairs,
            removed,
        } = self;
        let aligned = || {
            rows.len() == last.len()
                && rows.iter().zip(last).all(|(row, last)| row.key() == last.key())
        };
        let dense_rows = |held: Visit<'_>| {
            if aligned() {
                held.aligned(rows, last)
            } else {
                held.with(Form::Complete).rows(rows)
            }
        };
        if hour < dense.load(Ordering::Relaxed) {
            return dense_rows(held);
        }
        let limit = rows.len().max(last.len()) / 4 * 3;
        pairs.clear();
        removed.clear();
        let (mut at, mut before) = (0, 0);
        let (mut newest, mut oldest) = (None, None);
        loop {
            let (key, old) = (rows.get(at).map(R::key), last.get(before).map(R::key));
            let (current, held_key) = match (key, old) {
                (None, None) => break,
                (Some(key), Some(old)) => (
                    Some(key).filter(|&key| key <= old),
                    Some(old).filter(|&old| old <= key),
                ),
                (key, old) => (key, old),
            };
            if let Some(key) = current {
                if newest.is_some_and(|newest| newest >= key) {
                    return held.with(Form::Complete).rows(rows);
                }
                newest = Some(key);
            }
            if let Some(old) = held_key {
                if oldest.is_some_and(|oldest| oldest >= old) {
                    return held.with(Form::Complete).rows(rows);
                }
                oldest = Some(old);
            }
            let (row, previous) = (index(at)?, index(before)?);
            match (current, held_key) {
                (Some(_), Some(_)) => {
                    if rows[at] != last[before] {
                        engine_allocation::reserve(pairs, 1)?;
                        pairs.push((row, Some(previous)));
                    }
                    at += 1;
                    before += 1;
                }
                (Some(_), None) => {
                    engine_allocation::reserve(pairs, 1)?;
                    pairs.push((row, None));
                    at += 1;
                }
                (None, Some(old)) => {
                    engine_allocation::reserve(removed, 1)?;
                    removed.push(old);
                    before += 1;
                }
                (None, None) => unreachable!("one side has a row"),
            }
            if pairs.len() + removed.len() > limit {
                dense.store(hour.saturating_add(DENSE_HOURS), Ordering::Relaxed);
                return dense_rows(held);
            }
        }
        held.paired(rows, last, pairs, removed)
    }
}

/// A complete table's rows the row encoder's diff wrote, each beside its baseline row.
struct Written<'a> {
    held: Visit<'a>,
    written: &'a ChangedRows,
}

impl RowsPairVisit for Written<'_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R], last: &[R]) -> io::Result<TableHour> {
        let Self { held, written } = self;
        held.paired(rows, last, &written.pairs, &written.removed)
    }
}

/// A row index as pairs hold it.
fn index(at: usize) -> io::Result<u32> {
    u32::try_from(at).map_err(|_| invalid("A table holds more than 2^32 rows"))
}

/// A comparison's changed rows beside their baseline rows, which the held table holds.
struct Compared<'a> {
    held: Visit<'a>,
    indices: &'a [usize],
    pairs: &'a mut Vec<(u32, Option<u32>)>,
}

impl RowsPairVisit for Compared<'_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R], last: &[R]) -> io::Result<TableHour> {
        let Self {
            held,
            indices,
            pairs,
        } = self;
        if indices.len() != rows.len() || indices.iter().any(|&before| before >= last.len()) {
            return Err(invalid("A compared change names no baseline row"));
        }
        pairs.clear();
        engine_allocation::reserve(pairs, rows.len())?;
        for (at, &before) in indices.iter().enumerate() {
            pairs.push((index(at)?, Some(index(before)?)));
        }
        held.paired(rows, last, pairs, &[])
    }
}

/// The row type of an hour that visits no rows.
type Never = replay_data::rows::ClockRow;

/// Kind-only while a capture runs, whatever diagnostic scope encloses it: the live writer
/// must not allocate to fail. Offline writers keep the message.
fn invalid(what: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, what)
    }
}
