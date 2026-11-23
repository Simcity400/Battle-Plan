
use crate::{columnar_file::{RestartState, referenced_ids}, native_columns::{self, RowCells, TableHour, TableInput as Native}};
use replay_recording::schema::LAST_COMPLETE;
use replay_frame::frame::{FrameEncoder, FrameInput, Metrics, TRUNCATED_KEYFRAME_REASON, TableInput, TableView};
use replay_data::{format::FormatVersion, rows::{ChangedRows, RowCodec, RowsPairVisit, RowsVisit, TABLE_COUNT, TableId, TableRows}, scope::{self, CaptureMode}};
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

const DENSE_HOURS: i32 = 24;

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
    pairs: Vec<(u32, Option<u32>)>,
}

pub struct FrameWriter {
    tables: Vec<TableWriter>,
    columns: Vec<usize>,
    dictionary: Dictionary,
    lanes: Vec<Lane>,
    absent: Vec<Option<String>>,
    starting: bool,
    poisoned: bool,
    since_restart: Vec<Box<[u8]>>,
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
}

type Owners = Vec<(u32, Vec<u128>)>;

struct Slice {
    owners: Vec<u32>,
    keys: Vec<u128>,
}

pub struct FrameHour {
    branch: BranchId,
    hour: i32,
    tables: Vec<TableHour>,
    shared: Vec<Option<Arc<TableRows>>>,
    slices: Vec<Option<Slice>>,
    statuses: Vec<Status>,
    read: Vec<u64>,
    appends: Vec<Box<[u8]>>,
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
        if let TableInput::SliceUpdate { update, .. } = input
            && written.is_none()
        {
            removed(id, writer, update, self.owners[index].as_ref(), &mut lane.removed)?;
        }
        let carried = source.is_none()
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
                Some(written) => Self::written(rows, last, held, written),
                None => self.diff(index, rows, last, held, &mut lane.pairs, &mut lane.removed),
            };
        }
        match input {
            _ if carried => held.same(),
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

fn owner_slice(update: &SliceUpdate) -> bool {
    update.ranges.is_empty() && update.keys.is_empty()
}

fn owner_of(id: TableId, key: u128) -> u32 {
    (key >> id.key_bits().saturating_sub(32)) as u32
}

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

fn removed(
    id: TableId,
    writer: &TableWriter,
    update: &SliceUpdate,
    owners: Option<&Owners>,
    out: &mut Vec<u128>,
) -> io::Result<()> {
    out.clear();
    if let Some(owners) = owners.filter(|_| owner_slice(update)) {
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

fn index(at: usize) -> io::Result<u32> {
    u32::try_from(at).map_err(|_| invalid("A table holds more than 2^32 rows"))
}

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

type Never = replay_data::rows::ClockRow;

fn invalid(what: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, what)
    }
}
