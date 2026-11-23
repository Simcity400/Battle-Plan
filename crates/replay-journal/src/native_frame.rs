
use crate::{
    columnar_file::{RestartState, referenced_ids},
    native_columns::{self, Pair, RowCells, TableHour, TableInput as Native},
};
use replay_data::{
    format::FormatVersion,
    rows::{ChangedRows, Field, List, Opaque, RowCodec, RowsPairVisit, RowsVisit, TABLE_COUNT, TableId, TableRows},
    scope::{self, CaptureMode},
};
use replay_frame::frame::{
    FrameEncoder, FrameInput, Metrics, TRUNCATED_KEYFRAME_REASON, TableInput, TableView,
};
use replay_recording::schema::LAST_COMPLETE;
use replay_columnar::{
    manifest::{self, Manifest, Section, Status},
    restart::Last,
    state::Cell,
    writer::{Dictionary, Evidence, Prepared, Sample, Strings, Sweep, TableWriter},
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

const DENSE_HOURS: i32 = 24;
const INTERN_COOLDOWN_HOURS: i32 = 24;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub struct RowFrame<'a> {
    pub baselines: Option<&'a FrameEncoder>,
    pub keyframe: bool,
    pub changed: &'a [(usize, ChangedRows)],
}

pub trait Lanes: Sync {
    fn count(&self) -> usize;
    fn run(&self, job: &(dyn Fn() + Sync));
    fn proceeds(&self) -> bool {
        true
    }
}

pub struct OneThread;

impl Lanes for OneThread {
    fn count(&self) -> usize {
        1
    }

    fn run(&self, job: &(dyn Fn() + Sync)) {
        job();
    }
}

struct Lane {
    cells: RowCells,
    removed: Vec<u128>,
    pairs: Vec<Pair>,
}

pub struct FrameWriter {
    tables: Vec<TableWriter>,
    columns: Vec<usize>,
    dictionary: Dictionary,
    lanes: Vec<Lane>,
    absent: Vec<Option<String>>,
    starting: bool,
    poisoned: bool,
    since_restart_from: u32,
    pending: bool,
    table_ns: Vec<u64>,
    shared: Vec<Option<Arc<TableRows>>>,
    prepared: Vec<Option<io::Result<TableHour>>>,
    costs: Vec<u64>,
    walls: Vec<u64>,
    order: Vec<usize>,
    retried: Vec<u64>,
    owners: Vec<Option<Owners>>,
    last: Option<(BranchId, i32)>,
    dense: Vec<AtomicI32>,
    intern_until: Vec<i32>,
    quiet_hours: Vec<u8>,
}

enum Owners {
    U64(Vec<u64>),
    U128(Vec<u128>),
}

impl Owners {
    fn from_keys(id: TableId, count: usize, keys: impl Iterator<Item = u128>) -> io::Result<Self> {
        if id.key_bits() <= 64 {
            let mut narrow = engine_allocation::vec(count)?;
            let mut wide = None;
            for key in keys {
                if let Some(wide) = wide.as_mut() {
                    engine_allocation::reserve(wide, 1)?;
                    wide.push(key);
                } else if let Ok(small) = u64::try_from(key) {
                    engine_allocation::reserve(&mut narrow, 1)?;
                    narrow.push(small);
                } else {
                    let mut promoted = engine_allocation::vec(count.max(narrow.len() + 1))?;
                    promoted.extend(narrow.iter().copied().map(u128::from));
                    promoted.push(key);
                    narrow = Vec::new();
                    wide = Some(promoted);
                }
            }
            if let Some(wide) = wide {
                Ok(Self::U128(wide))
            } else {
                Ok(Self::U64(narrow))
            }
        } else {
            let mut wide = engine_allocation::vec(count)?;
            for key in keys {
                engine_allocation::reserve(&mut wide, 1)?;
                wide.push(key);
            }
            Ok(Self::U128(wide))
        }
    }

    fn sort_unstable(&mut self) {
        match self {
            Self::U64(keys) => keys.sort_unstable(),
            Self::U128(keys) => keys.sort_unstable(),
        }
    }
}

struct Slice {
    owners: Vec<u32>,
    keys: Owners,
}

pub struct FrameHour {
    branch: BranchId,
    hour: i32,
    tables: Vec<TableHour>,
    shared: Vec<Option<Arc<TableRows>>>,
    slices: Vec<Option<Slice>>,
    statuses: Vec<Status>,
    read: Vec<u64>,
    pub manifest: Vec<u8>,
}

impl FrameHour {
    pub fn sections(&self) -> impl Iterator<Item = (usize, &[u8])> {
        self.tables.iter().enumerate().filter_map(|(index, table)| {
            let section = table.prepared.as_ref()?.section.as_deref()?;
            Some((index, section))
        })
    }

    pub fn hour(&self) -> i32 {
        self.hour
    }

    pub fn statuses(&self) -> &[Status] {
        &self.statuses
    }

    pub fn read(&self) -> &[u64] {
        &self.read
    }

    pub fn prepared(&self) -> impl Iterator<Item = (usize, &Prepared)> {
        self.tables
            .iter()
            .enumerate()
            .filter_map(|(index, table)| Some((index, table.prepared.as_ref()?)))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Changed<'a> {
    pub keys: &'a [u128],
    pub removed: &'a [u128],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Source<'a> {
    pub sample: Option<i32>,
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
            let kinds = replay_recording::column_kind::columns(id.schema())?;
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
            since_restart_from: 0,
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
            intern_until: engine_allocation::filled(TABLE_COUNT, i32::MIN)?,
            quiet_hours: engine_allocation::filled(TABLE_COUNT, 0)?,
        })
    }

    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    pub fn table_ns(&self) -> &[u64] {
        &self.table_ns
    }

    pub fn retried(&self) -> &[u64] {
        &self.retried
    }

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
            intern_until,
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
                intern_until[index] = input.hour.saturating_add(INTERN_COOLDOWN_HOURS);
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
            let sample = match recorded.get(index) {
                Some(source) => source.sample,
                None => match prepared.evidence.status {
                    Status::Complete if was_read || !changed => implied,
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
                TableInput::SharedComplete(rows) if Arc::strong_count(rows) > 1 => {
                    Some(Arc::clone(rows))
                }
                _ => None,
            });
            slices.push(match table {
                TableInput::SliceUpdate { update, .. }
                    if owner_slice(update) && previous.get(index).copied().flatten().is_none() =>
                {
                    let id = TableId::ALL[index];
                    Some(Slice {
                        owners: engine_allocation::copy_vec(update.owners())?,
                        keys: Owners::from_keys(
                            id,
                            update.rows().len(),
                            update.rows().iter().map(|row| row.key()),
                        )?,
                    })
                }
                _ => None,
            });
            let wall = lane_wall + started.elapsed().as_nanos() as u64;
            costs[index] = wall;
            table_ns[index] += wall;
        }
        let appended = dictionary.take_appended_range();
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
            appends: engine_allocation::collect_vec(
                appended.filter_map(|id| dictionary.get(id)),
            )?,
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
            manifest,
        })
    }

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
            intern_until,
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
        let writers = Disjoint::new(writers);
        let prepared = Disjoint::new(prepared);
        let walls = Disjoint::new(walls);
        let buffers = Disjoint::new(buffers);
        let (next, lane) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let (dictionary, order, intern_until) = (&*dictionary, &**order, &**intern_until);
        let job = || {
            let lane_index = lane.fetch_add(1, Ordering::Relaxed);
            if lane_index >= buffers.len {
                return;
            }
            // SAFETY: each worker claims a distinct `lane_index < buffers.len`.
            let buffers = unsafe { buffers.get_mut(lane_index) };
            while lanes.proceeds() {
                let Some(&index) = order.get(next.fetch_add(1, Ordering::Relaxed)) else {
                    break;
                };
                if input.hour < intern_until[index] {
                    continue;
                }
                // SAFETY: `order` is a permutation of `0..TABLE_COUNT`, so each `index` is claimed once.
                let (writer, hour, wall) = unsafe {
                    (
                        writers.get_mut(index),
                        prepared.get_mut(index),
                        walls.get_mut(index),
                    )
                };
                let started = Instant::now();
                *hour = Some(part.table(
                    index,
                    writer,
                    Strings::Reading(dictionary),
                    buffers,
                ));
                *wall = started.elapsed().as_nanos() as u64;
            }
        };
        lanes.run(&job);
        Ok(())
    }

    fn costliest_first(&mut self) {
        let costs = &self.costs;
        self.order
            .sort_unstable_by_key(|&index| std::cmp::Reverse(costs[index]));
    }

    pub fn commit(&mut self, hour: FrameHour) -> io::Result<()> {
        self.commit_on(hour, &OneThread)
    }

    pub fn commit_on(&mut self, hour: FrameHour, lanes: &dyn Lanes) -> io::Result<()> {
        if self.poisoned {
            return Err(invalid("A failed hour ended this columnar run"));
        }
        let FrameHour {
            branch,
            hour: at,
            tables: hours,
            shared,
            slices,
            ..
        } = hour;
        if let Err(error) = self.apply(at, hours, shared, slices, lanes) {
            self.poisoned = true;
            return Err(error);
        }
        self.last = Some((branch, at));
        self.starting = false;
        self.pending = false;
        for lane in &mut self.lanes {
            lane.cells.release_scratch();
            lane.removed.clear();
            if lane.removed.capacity().saturating_mul(size_of::<u128>()) > 32 * 1024 {
                lane.removed = Vec::new();
            }
            lane.pairs.clear();
            if lane.pairs.capacity().saturating_mul(size_of::<Pair>()) > 32 * 1024 {
                lane.pairs = Vec::new();
            }
        }
        Ok(())
    }

    fn apply(
        &mut self,
        at: i32,
        mut hours: Vec<TableHour>,
        mut shared: Vec<Option<Arc<TableRows>>>,
        mut slices: Vec<Option<Slice>>,
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
            quiet_hours,
            ..
        } = self;
        let failed = Mutex::new(None);
        {
            let writers = Disjoint::new(writers);
            let held = Disjoint::new(held);
            let owners = Disjoint::new(owners);
            let wall_slots = Disjoint::new(walls);
            let quiet_slots = Disjoint::new(quiet_hours);
            let hours = Disjoint::new(&mut hours);
            let shared = Disjoint::new(&mut shared);
            let slices = Disjoint::new(&mut slices);
            let next = AtomicUsize::new(0);
            let order = &**order;
            let commit = |index: usize| {
                // SAFETY: `order` is a permutation of `0..TABLE_COUNT`, so each `index` is claimed once.
                let (writer, held, owners, wall, quiet, hour, shared, slice) = unsafe {
                    (
                        writers.get_mut(index),
                        held.get_mut(index),
                        owners.get_mut(index),
                        wall_slots.get_mut(index),
                        quiet_slots.get_mut(index),
                        hours.get_mut(index),
                        shared.get_mut(index),
                        slices.get_mut(index),
                    )
                };
                let prepared = hour.prepared.take();
                let evidence = hour.evidence;
                let timed = prepared.is_some();
                let sectioned = prepared
                    .as_ref()
                    .is_some_and(|prepared| prepared.section.is_some());
                let started = timed.then(Instant::now);
                let kept = !timed && evidence.status == Status::Complete;
                let committed = writer.commit(prepared, evidence, at).and_then(|()| {
                    *held = shared.take();
                    let id = TableId::ALL[index];
                    *owners = match (slice.take(), owners.take()) {
                        (Some(slice), Some(mut known)) => {
                            hold_slice(&mut known, &slice, id)?;
                            Some(known)
                        }
                        (Some(_), None) => Some(held_owners(&*writer, id)?),
                        (None, known) => known.filter(|_| kept),
                    };
                    if sectioned {
                        *quiet = 0;
                    } else {
                        let streak = quiet.saturating_add(1);
                        *quiet = streak;
                        if streak == 2 {
                            writer.drop_scratch();
                        }
                    }
                    Ok(())
                });
                if let Err(error) = committed {
                    lock(&failed).get_or_insert(error);
                }
                *wall = started.map_or(0, |started| started.elapsed().as_nanos() as u64);
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
            while let Some(&index) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                commit(index);
            }
        }
        if let Some(error) = failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
            return Err(error);
        }
        for ((cost, total), wall) in costs.iter_mut().zip(table_ns.iter_mut()).zip(walls.iter()) {
            *cost += wall;
            *total += wall;
        }
        Ok(())
    }

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
        let appends = self.dictionary.slice_from(self.since_restart_from)?;
        self.since_restart_from = self.dictionary.len() as u32;
        Ok((appends, tables))
    }

    pub fn hour_with(
        &mut self,
        input: &FrameInput,
        recorded: &[Source<'_>],
    ) -> io::Result<FrameHour> {
        self.hour_on(input, recorded, None, &OneThread)
    }

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
        if self.pending {
            self.poisoned = true;
            return Err(invalid("A prepared hour commits before the next"));
        }
        let hour = self.prepare(input, recorded, row, lanes);
        self.poisoned = hour.is_err();
        self.pending = hour.is_ok();
        hour
    }

    pub fn tables(&self) -> &[TableWriter] {
        &self.tables
    }

    pub fn materialize_complete(&self, id: TableId) -> io::Result<Option<TableRows>> {
        let writer = &self.tables[id.index()];
        if writer.status() != Status::Complete {
            return Ok(None);
        }
        let table = writer.table();
        let mut keyed = engine_allocation::vec(table.live().count())?;
        for slot in table.live().ones() {
            let key = table
                .key(slot)
                .ok_or_else(|| invalid("A live columnar slot holds no row key"))?;
            keyed.push((key, slot));
        }
        keyed.sort_unstable();
        let keys = engine_allocation::collect_vec(keyed.iter().map(|&(key, _)| key))?;
        let format = FormatVersion::CURRENT.version();
        let dictionary = &self.dictionary;
        let rows = id.rows_from_wire_fields(&keys, id.full_mask(), &mut |at, column| {
            let slot = keyed[at].1;
            if id.column_withheld_at(column, format) || !table.recorded(column, slot) {
                return Ok(Field::NotRecorded);
            }
            let cell = table
                .column(column)
                .and_then(|values| values.cell(slot))
                .ok_or_else(|| invalid("A live columnar slot has no cell"))?;
            Ok(match cell {
                Cell::U8(value) => Field::U8(value),
                Cell::I8(value) => Field::I8(value),
                Cell::U16(value) => Field::U16(value),
                Cell::I16(value) => Field::I16(value),
                Cell::U32(value) => Field::U32(value),
                Cell::I32(value) => Field::I32(value),
                Cell::U64(value) => Field::U64(value),
                Cell::I64(value) => Field::I64(value),
                Cell::Bool(value) => Field::Bool(value),
                Cell::OptionalI32(value) => Field::OptionalI32(value),
                Cell::OptionalU32(value) => Field::OptionalU32(value),
                Cell::OptionalI64(value) => Field::OptionalI64(value),
                Cell::OptionalU64(value) => Field::OptionalU64(value),
                Cell::Bytes(string_id) => Field::Bytes(
                    dictionary
                        .get(string_id)
                        .ok_or_else(|| invalid("A text cell names a string not in the dictionary"))?,
                ),
                Cell::U32List(values) => Field::U32List(List::Native(values)),
                Cell::U64List(values) => Field::U64List(List::Native(values)),
                Cell::Opaque(bytes) => Field::Opaque(Opaque::Encoded(bytes, format)),
            })
        })?;
        Ok(Some(rows))
    }

    pub fn with_staging(staging: native_columns::Staging) -> io::Result<Self> {
        let mut writer = Self::new()?;
        writer.lanes[0].cells = RowCells::new(staging);
        Ok(writer)
    }

    pub fn with_sweep(sweep: Sweep) -> io::Result<Self> {
        let mut writer = Self::new()?;
        for table in &mut writer.tables {
            table.set_sweep(sweep);
        }
        Ok(writer)
    }
}

struct Part<'a> {
    input: &'a FrameInput,
    recorded: &'a [Source<'a>],
    previous: &'a [Option<&'a TableRows>],
    changed: &'a [(usize, ChangedRows)],
    read: &'a [u64],
    mode: CaptureMode,
    absent: &'a [Option<String>],
    held: &'a [Option<Arc<TableRows>>],
    owners: &'a [Option<Owners>],
    dense: &'a [AtomicI32],
    starting: bool,
    keyframe: bool,
}

impl Part<'_> {
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
        let known = !self.starting && writer.status() == Status::Complete;
        let previous = self.previous.get(index).copied().flatten().filter(|_| known);
        let written = previous.and_then(|_| {
            let at = self.changed.binary_search_by_key(&index, |(table, _)| *table).ok()?;
            Some(&self.changed[at].1)
        });
        if let TableInput::SliceUpdate { update, .. }
        | TableInput::UniformOptionalI32 { update, .. } = input
            && written.is_none()
            && previous.is_none()
        {
            removed(id, writer, update, self.owners[index].as_ref(), &mut lane.removed)?;
        }
        let carried_complete = source.is_none()
            && !self.starting
            && matches!(input, TableInput::Sampled(TableView::Unavailable { reason })
                if reason == replay_data::cadence::SKIPPED_REASON)
            && replay_data::cadence::is_staggered(id)
            && writer.status() == Status::Complete
            && writer.acquisition().available_from.is_some();
        let delta = source
            .and_then(|source| source.delta)
            .filter(|_| !self.starting && writer.status() == Status::Complete);
        let held = Visit {
            writer,
            strings,
            cells: &mut lane.cells,
            read,
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
                Some(written) => Self::written(rows, last, held, written, &mut lane.pairs),
                None => self.diff(index, rows, last, held, &mut lane.pairs, &mut lane.removed),
            };
        }
        match input {
            _ if carried_complete => held.same(),
            TableInput::ComparedEqual(_) => held.same(),
            TableInput::Sampled(view) => held.view(view),
            TableInput::SharedComplete(rows)
                if self.held[index]
                    .as_ref()
                    .is_some_and(|last| Arc::ptr_eq(last, rows)) =>
            {
                held.same()
            }
            TableInput::SharedComplete(rows) => {
                if let (Some(last), Some(written)) = (previous, written) {
                    return Self::written(rows, last, held, written, &mut lane.pairs);
                }
                let last = self.held[index].as_deref().filter(|_| known).or(previous);
                match last {
                    Some(last) => {
                        self.diff(index, rows, last, held, &mut lane.pairs, &mut lane.removed)
                    }
                    None => rows.visit(held.with(Form::Complete)),
                }
            }
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
                (Some(last), Some(written)) => {
                    Self::written(&update.rows, last, held, written, &mut lane.pairs)
                }
                (Some(last), None) => Self::slice_diff(
                    id,
                    update,
                    last,
                    held,
                    &mut lane.pairs,
                    &mut lane.removed,
                    None,
                ),
                (None, _) => update.rows.visit(held.with(Form::Changes(&lane.removed))),
            },
            TableInput::UniformOptionalI32 {
                column,
                value,
                update,
                ..
            } => match previous {
                Some(last) => Self::slice_diff(
                    id,
                    update,
                    last,
                    held,
                    &mut lane.pairs,
                    &mut lane.removed,
                    Some((*column, *value)),
                ),
                None => Self::changes_uniform(
                    &update.rows,
                    held,
                    &mut lane.pairs,
                    &lane.removed,
                    (*column, *value),
                ),
            },
        }
    }
}

impl Part<'_> {
    fn written(
        rows: &TableRows,
        last: &TableRows,
        held: Visit<'_>,
        written: &ChangedRows,
        pairs: &mut Vec<Pair>,
    ) -> io::Result<TableHour> {
        match rows.visit_pair(last, Written { held, written, pairs }) {
            Ok(hour) => hour,
            Err(Written { held, .. }) => rows.visit(held.with(Form::Complete)),
        }
    }

    fn changes_uniform(
        rows: &TableRows,
        held: Visit<'_>,
        pairs: &mut Vec<Pair>,
        removed: &[u128],
        uniform: (usize, Option<i32>),
    ) -> io::Result<TableHour> {
        rows.visit(ChangesUniform {
            held,
            pairs,
            removed,
            uniform,
        })
    }

    fn diff(
        &self,
        index: usize,
        rows: &TableRows,
        last: &TableRows,
        held: Visit<'_>,
        pairs: &mut Vec<Pair>,
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

    fn slice_diff(
        id: TableId,
        update: &SliceUpdate,
        last: &TableRows,
        held: Visit<'_>,
        pairs: &mut Vec<Pair>,
        removed: &mut Vec<u128>,
        uniform_optional_i32: Option<(usize, Option<i32>)>,
    ) -> io::Result<TableHour> {
        let diff = SliceDiff {
            id,
            update,
            held,
            pairs,
            removed,
            uniform_optional_i32,
        };
        match update.rows.visit_pair(last, diff) {
            Ok(hour) => hour,
            Err(_) => Err(invalid("slice update rows differ from baseline type")),
        }
    }
}

struct Disjoint<'a, T> {
    ptr: *mut T,
    len: usize,
    _marker: std::marker::PhantomData<&'a mut [T]>,
}

// SAFETY: callers partition indices into disjoint positions so no two threads ever
// alias the same element.
unsafe impl<T: Send> Sync for Disjoint<'_, T> {}

impl<T> Disjoint<'_, T> {
    fn new(slice: &mut [T]) -> Self {
        Self {
            ptr: slice.as_mut_ptr(),
            len: slice.len(),
            _marker: std::marker::PhantomData,
        }
    }

    /// # Safety
    /// `index < self.len` and no other reference to element `index` exists concurrently.
    #[allow(clippy::mut_from_ref, reason = "disjoint per-index access across workers")]
    unsafe fn get_mut(&self, index: usize) -> &mut T {
        debug_assert!(index < self.len);
        unsafe { &mut *self.ptr.add(index) }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn owner_slice(update: &SliceUpdate) -> bool {
    !update.owners.is_empty() && update.ranges.is_empty() && update.keys.is_empty()
}

fn owner_of(id: TableId, key: u128) -> u32 {
    (key >> id.key_bits().saturating_sub(32)) as u32
}

fn owner_of_u64(id: TableId, key: u64) -> u32 {
    (key >> id.key_bits().saturating_sub(32)) as u32
}

fn held_owners(writer: &TableWriter, id: TableId) -> io::Result<Owners> {
    let table = writer.table();
    let mut keys = Owners::from_keys(
        id,
        table.live().count(),
        table.live().ones().filter_map(|slot| table.key(slot)),
    )?;
    keys.sort_unstable();
    Ok(keys)
}

fn merge_slice_keys<T: Copy>(
    owners: &mut Vec<T>,
    slice_owners: &[u32],
    slice_keys: &[T],
    owner_of_key: impl Fn(T) -> u32,
) -> io::Result<()> {
    let mut old_at = 0usize;
    let mut replaced_old = 0usize;
    for &owner in slice_owners {
        let start = owners[old_at..].partition_point(|&key| owner_of_key(key) < owner) + old_at;
        let end = owners[start..].partition_point(|&key| owner_of_key(key) <= owner) + start;
        replaced_old += end - start;
        old_at = end;
    }
    let final_len = owners.len() - replaced_old + slice_keys.len();
    let mut out = engine_allocation::vec(final_len)?;
    old_at = 0;
    let mut keys = slice_keys;
    for &owner in slice_owners {
        let start = owners[old_at..].partition_point(|&key| owner_of_key(key) < owner) + old_at;
        let end = owners[start..].partition_point(|&key| owner_of_key(key) <= owner) + start;
        out.extend_from_slice(&owners[old_at..start]);
        let (own, rest) = keys.split_at(keys.partition_point(|&key| owner_of_key(key) <= owner));
        keys = rest;
        out.extend_from_slice(own);
        old_at = end;
    }
    out.extend_from_slice(&owners[old_at..]);
    *owners = out;
    Ok(())
}

fn hold_slice(owners: &mut Owners, slice: &Slice, id: TableId) -> io::Result<()> {
    match (owners, &slice.keys) {
        (Owners::U64(known), Owners::U64(incoming)) => {
            merge_slice_keys(known, &slice.owners, incoming, |key| owner_of_u64(id, key))
        }
        (Owners::U128(known), Owners::U128(incoming)) => {
            merge_slice_keys(known, &slice.owners, incoming, |key| owner_of(id, key))
        }
        (Owners::U128(known), Owners::U64(incoming)) => {
            let widened = engine_allocation::collect_vec(incoming.iter().copied().map(u128::from))?;
            merge_slice_keys(known, &slice.owners, &widened, |key| owner_of(id, key))
        }
        (slot @ Owners::U64(_), Owners::U128(incoming)) => {
            let Owners::U64(old) = std::mem::replace(slot, Owners::U128(Vec::new())) else {
                unreachable!();
            };
            let mut widened = engine_allocation::collect_vec(old.into_iter().map(u128::from))?;
            merge_slice_keys(&mut widened, &slice.owners, incoming, |key| {
                owner_of(id, key)
            })?;
            *slot = Owners::U128(widened);
            Ok(())
        }
    }
}

fn removed(
    id: TableId,
    writer: &TableWriter,
    update: &SliceUpdate,
    owners: Option<&Owners>,
    out: &mut Vec<u128>,
) -> io::Result<()> {
    out.clear();
    if update.owners.is_empty() && update.keys.is_empty() && update.ranges.is_empty() {
        return Ok(());
    }
    if !update.keys.is_empty() && update.owners.is_empty() && update.ranges.is_empty() {
        let table = writer.table();
        for &key in &update.keys {
            if !update.rows.contains_key(key) && table.slot(key).is_some() {
                engine_allocation::reserve(out, 1)?;
                out.push(key);
            }
        }
        return Ok(());
    }
    if let Some(owners) = owners.filter(|_| owner_slice(update)) {
        match owners {
            Owners::U64(keys) => {
                for &owner in update.owners() {
                    let start = keys.partition_point(|&key| owner_of_u64(id, key) < owner);
                    for &key in &keys[start..] {
                        if owner_of_u64(id, key) != owner {
                            break;
                        }
                        let wide = u128::from(key);
                        if !update.rows.contains_key(wide) {
                            engine_allocation::reserve(out, 1)?;
                            out.push(wide);
                        }
                    }
                }
            }
            Owners::U128(keys) => {
                for &owner in update.owners() {
                    let start = keys.partition_point(|&key| owner_of(id, key) < owner);
                    for &key in &keys[start..] {
                        if owner_of(id, key) != owner {
                            break;
                        }
                        if !update.rows.contains_key(key) {
                            engine_allocation::reserve(out, 1)?;
                            out.push(key);
                        }
                    }
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

    fn aligned<R: RowCodec>(self, rows: &[R], previous: &[R]) -> io::Result<TableHour> {
        let read = self.read;
        self.write(Native::Aligned {
            rows,
            previous,
            read,
            sample: Sample::FromReadBit,
        })
    }

    fn paired<R: RowCodec>(
        self,
        rows: &[R],
        previous: &[R],
        pairs: &[Pair],
        removed: &[u128],
        uniform_optional_i32: Option<(usize, Option<i32>)>,
    ) -> io::Result<TableHour> {
        let read = self.read;
        self.write(Native::Paired {
            rows,
            previous,
            pairs,
            removed,
            uniform_optional_i32,
            read,
            sample: Sample::FromReadBit,
        })
    }

    fn write<R: RowCodec>(self, input: Native<'_, R>) -> io::Result<TableHour> {
        native_columns::table_hour(self.writer, self.strings, self.cells, input)
    }
}

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

#[inline]
fn row_diff<R: RowCodec>(row: &R, previous: &R, narrow: bool) -> Option<u64> {
    if narrow {
        let diff = row.differing_fields(previous);
        (diff != 0).then_some(diff)
    } else {
        (row != previous).then_some(u64::MAX)
    }
}

struct ChangesUniform<'a> {
    held: Visit<'a>,
    pairs: &'a mut Vec<Pair>,
    removed: &'a [u128],
    uniform: (usize, Option<i32>),
}

impl RowsVisit for ChangesUniform<'_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R]) -> io::Result<TableHour> {
        let Self {
            held,
            pairs,
            removed,
            uniform,
        } = self;
        pairs.clear();
        engine_allocation::reserve(pairs, rows.len())?;
        for at in 0..rows.len() {
            pairs.push(Pair::new(index(at)?, None));
        }
        held.paired(rows, &[], pairs, removed, Some(uniform))
    }
}

struct Diff<'a> {
    held: Visit<'a>,
    hour: i32,
    dense: &'a AtomicI32,
    pairs: &'a mut Vec<Pair>,
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
        let narrow = held.writer.kinds().len() <= 64;
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
                    if let Some(diff) = row_diff(&rows[at], &last[before], narrow) {
                        push_item(
                            pairs,
                            Pair {
                                row,
                                previous: Some(previous),
                                diff,
                            },
                        )?;
                    }
                    at += 1;
                    before += 1;
                }
                (Some(_), None) => {
                    push_item(pairs, Pair::new(row, None))?;
                    at += 1;
                }
                (None, Some(old)) => {
                    push_item(removed, old)?;
                    before += 1;
                }
                (None, None) => unreachable!("one side has a row"),
            }
            if pairs.len() + removed.len() > limit {
                dense.store(hour.saturating_add(DENSE_HOURS), Ordering::Relaxed);
                return dense_rows(held);
            }
        }
        held.paired(rows, last, pairs, removed, None)
    }
}

struct Written<'a> {
    held: Visit<'a>,
    written: &'a ChangedRows,
    pairs: &'a mut Vec<Pair>,
}

impl RowsPairVisit for Written<'_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R], last: &[R]) -> io::Result<TableHour> {
        let Self {
            held,
            written,
            pairs,
        } = self;
        pairs.clear();
        engine_allocation::reserve(pairs, written.pairs.len())?;
        pairs.extend(
            written
                .pairs
                .iter()
                .map(|&(row, previous)| Pair::new(row, previous)),
        );
        held.paired(rows, last, pairs, &written.removed, None)
    }
}

#[inline]
fn push_item<T>(values: &mut Vec<T>, value: T) -> io::Result<()> {
    if values.len() == values.capacity() {
        engine_allocation::reserve(values, 1)?;
    }
    values.push(value);
    Ok(())
}

fn gallop<R>(rows: &[R], holds: impl Fn(&R) -> bool) -> usize {
    let mut low = 0;
    let mut high = 1;
    while high <= rows.len() && holds(&rows[high - 1]) {
        low = high;
        high = high.saturating_mul(2);
    }
    low + rows[low..high.min(rows.len())].partition_point(holds)
}

fn merge_slice_diff<R: RowCodec>(
    rows: &[R],
    new_span: std::ops::Range<usize>,
    last: &[R],
    old_span: std::ops::Range<usize>,
    narrow: bool,
    pairs: &mut Vec<Pair>,
    removed: &mut Vec<u128>,
) -> io::Result<()> {
    let mut at = new_span.start;
    let mut before = old_span.start;
    while at < new_span.end && before < old_span.end {
        match rows[at].key().cmp(&last[before].key()) {
            std::cmp::Ordering::Equal => {
                if let Some(diff) = row_diff(&rows[at], &last[before], narrow) {
                    push_item(
                        pairs,
                        Pair {
                            row: index(at)?,
                            previous: Some(index(before)?),
                            diff,
                        },
                    )?;
                }
                at += 1;
                before += 1;
            }
            std::cmp::Ordering::Less => {
                push_item(pairs, Pair::new(index(at)?, None))?;
                at += 1;
            }
            std::cmp::Ordering::Greater => {
                push_item(removed, last[before].key())?;
                before += 1;
            }
        }
    }
    while at < new_span.end {
        push_item(pairs, Pair::new(index(at)?, None))?;
        at += 1;
    }
    while before < old_span.end {
        push_item(removed, last[before].key())?;
        before += 1;
    }
    Ok(())
}

struct SliceDiff<'a> {
    id: TableId,
    update: &'a SliceUpdate,
    held: Visit<'a>,
    pairs: &'a mut Vec<Pair>,
    removed: &'a mut Vec<u128>,
    uniform_optional_i32: Option<(usize, Option<i32>)>,
}

impl RowsPairVisit for SliceDiff<'_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R], last: &[R]) -> io::Result<TableHour> {
        let Self {
            id,
            update,
            held,
            pairs,
            removed,
            uniform_optional_i32,
        } = self;
        let narrow = held.writer.kinds().len() <= 64;
        pairs.clear();
        removed.clear();
        if !update.ranges.is_empty() {
            let (mut old_cursor, mut new_cursor) = (0, 0);
            for range in &update.ranges {
                let old_start =
                    old_cursor + gallop(&last[old_cursor..], |row| row.key() < range.start);
                let old_end = old_start + gallop(&last[old_start..], |row| row.key() < range.end);
                old_cursor = old_end;
                let new_start =
                    new_cursor + gallop(&rows[new_cursor..], |row| row.key() < range.start);
                let new_end = new_start + gallop(&rows[new_start..], |row| row.key() < range.end);
                new_cursor = new_end;
                merge_slice_diff(
                    rows,
                    new_start..new_end,
                    last,
                    old_start..old_end,
                    narrow,
                    pairs,
                    removed,
                )?;
            }
        } else if !update.keys.is_empty() {
            let (mut old_cursor, mut new_cursor) = (0, 0);
            for &key in &update.keys {
                old_cursor += gallop(&last[old_cursor..], |row| row.key() < key);
                let before =
                    (old_cursor < last.len() && last[old_cursor].key() == key).then_some(old_cursor);
                new_cursor += gallop(&rows[new_cursor..], |row| row.key() < key);
                let at =
                    (new_cursor < rows.len() && rows[new_cursor].key() == key).then_some(new_cursor);
                match (at, before) {
                    (Some(at), Some(before)) => {
                        if let Some(diff) = row_diff(&rows[at], &last[before], narrow) {
                            push_item(
                                pairs,
                                Pair {
                                    row: index(at)?,
                                    previous: Some(index(before)?),
                                    diff,
                                },
                            )?;
                        }
                    }
                    (Some(at), None) => {
                        push_item(pairs, Pair::new(index(at)?, None))?;
                    }
                    (None, Some(_)) => {
                        push_item(removed, key)?;
                    }
                    (None, None) => {}
                }
            }
        } else if !update.owners.is_empty() {
            let (mut old_cursor, mut new_cursor) = (0, 0);
            for &owner in &update.owners {
                let old_start = old_cursor
                    + gallop(&last[old_cursor..], |row| owner_of(id, row.key()) < owner);
                let old_end = old_start
                    + gallop(&last[old_start..], |row| owner_of(id, row.key()) <= owner);
                old_cursor = old_end;
                let new_start = new_cursor
                    + gallop(&rows[new_cursor..], |row| owner_of(id, row.key()) < owner);
                let new_end = new_start
                    + gallop(&rows[new_start..], |row| owner_of(id, row.key()) <= owner);
                new_cursor = new_end;
                merge_slice_diff(
                    rows,
                    new_start..new_end,
                    last,
                    old_start..old_end,
                    narrow,
                    pairs,
                    removed,
                )?;
            }
        }
        held.paired(rows, last, pairs, removed, uniform_optional_i32)
    }
}

fn index(at: usize) -> io::Result<u32> {
    u32::try_from(at).map_err(|_| invalid("A table holds more than 2^32 rows"))
}

struct Compared<'a> {
    held: Visit<'a>,
    indices: &'a [usize],
    pairs: &'a mut Vec<Pair>,
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
            pairs.push(Pair::new(index(at)?, Some(index(before)?)));
        }
        held.paired(rows, last, pairs, &[], None)
    }
}

type Never = replay_data::rows::ClockRow;

fn invalid(what: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, what)
    }
}
