use super::{placed::{Pool, Taken, Warm}, *};
use dashboard_follow::panel::{ProductReader, ProductStep};
use playhead::{Step, changes::Changes, lane::Lane, pages::WorldPages};
use replay_columnar::{manifest::Status, store::TableView as StoreView};
use replay_data::{rows::{RowChanges, TableRows}, scope::CaptureMode};
use replay_frame::frame::{SharedView, TableView, Unsayable, cells_rows_of};
use std::collections::hash_map::Entry;

impl Keys {
    fn contains(&self, key: u128) -> bool {
        match self {
            Self::All => true,
            Self::These(keys) => keys.binary_search(&key).is_ok(),
            Self::Range(lo, hi) => (*lo..=*hi).contains(&key),
            Self::Ranges(ranges) => ranges.get(ranges.partition_point(|&(_, hi)| hi < key)).is_some_and(|&(lo, _)| lo <= key),
        }
    }
}

pub(super) struct Projection {
    run: usize,
    columns: Vec<usize>,
    format: u16,
    fields: u64,
    validity: u64,
    selection: Option<Keys>,
    at: Option<i32>,
}

#[derive(Default)]
pub struct Edits {
    pub previous: BTreeMap<TableId, SharedView>,
    pub tables: BTreeMap<TableId, BTreeMap<u128, u64>>,
    pub replaced: BTreeSet<TableId>,
}

impl Edits {
    pub fn fields(&self, table: TableId, mask: u64) -> bool {
        self.replaced.contains(&table) || self.tables.get(&table).is_some_and(|keys| keys.values().any(|fields| mask & fields != 0))
    }

    pub fn only_columns(&self, table: TableId, mask: u64) -> bool {
        self.replaced.is_empty() && self.tables.iter().all(|(id, rows)| *id == table && rows.values().all(|fields| fields & !mask == 0))
    }
}

fn field_mask(table: TableId, columns: Option<&[usize]>) -> u64 {
    columns.map_or(table.full_mask(), |v| v.iter().fold(0, |m, &c| m | 1 << c))
}

pub(super) fn capture_mode(pages: &WorldPages) -> io::Result<CaptureMode> {
    CaptureMode::from_byte(pages.recording().root_identity().capture_mode)
}

impl Projection {
    pub(super) fn new(pages: &WorldPages, run: usize, table: TableId, columns: Option<Vec<usize>>, cold: Option<Vec<usize>>, raw: bool) -> io::Result<Self> {
        if let Some(reason) = pages.withheld(table.index()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, reason.to_owned()));
        }
        let fields = field_mask(table, columns.as_deref());
        let omitted = field_mask(table, cold.as_deref()) & !fields;
        let mut validity = 0;
        if !raw {
            for column in 0..64 {
                let bit = 1u64 << column;
                if omitted & bit != 0 && table.rows_from_wire_fields(&[0], bit, &mut |_, _| Ok(replay_data::rows::Field::NotRecorded)).is_err() {
                    validity |= bit;
                }
            }
        }
        let recording = pages.recording();
        let mut physical = match columns {
            Some(mut columns) => {
                columns.extend((0..64).filter(|&c| validity & (1 << c) != 0));
                columns
            }
            None => recording.read_columns(table.index()),
        };
        let recorded = recording.columns()[table.index()];
        physical.retain(|&column| column < recorded);
        physical.sort_unstable();
        physical.dedup();
        Ok(Self { run, columns: physical, format: recording.format(), fields, validity, selection: None, at: None })
    }

    fn lane<'s>(&self, table: TableId, hour: i32, step: ProductStep<'s>) -> Option<(&'s Step, &'s Lane, &'s Changes)> {
        let lane = step.lanes.get(table.index())?;
        let change = step.step.table(table.index())?;
        (lane.run() == self.run && change.head() == Some(hour) && change.log().is_none() && lane.keyed()
            && self.columns.iter().all(|column| lane.requested().contains(column))).then_some((step.step, lane, change))
    }

    fn read(&mut self, input: &mut Input<'_>, read: &Read, sample: bool, view: &mut SharedView, edits: &mut Edits) -> io::Result<()> {
        if self.selection.as_ref() != Some(&read.keys) {
            self.at = None;
        }
        let Input { pool, hour, count, step, placed, mode, .. } = input;
        let (run, table) = (self.run, read.table.index());
        match step.and_then(|step| self.lane(read.table, *hour, step)) {
            Some((step, lane, change)) => self.from_lane(read, sample, view, edits, count, lane, Some((step, change)), *mode)?,
            None => {
                let warm = if step.is_some_and(|step| step.lanes_table(table)) {
                    let columns = self.columns.clone();
                    let wake = step.and_then(|step| step.wake.cloned());
                    pool.warm(run, table, &columns, *hour, wake, |lane| self.from_lane(read, sample, view, edits, count, lane, None, *mode))
                } else { Warm::Cold };
                match warm {
                    Warm::Read(result) => result?,
                    Warm::Warming => return Err(io::Error::new(io::ErrorKind::WouldBlock, format!("{:?} is being placed beside the step", read.table))),
                    Warm::Cold => {
                        let placed = placed.get_or_insert_with(|| pool.take(run, *hour));
                        let lane = placed.lane(table, &self.columns, *hour)?;
                        self.from_lane(read, sample, view, edits, count, lane, None, *mode)?;
                    }
                }
            }
        }
        self.selection = Some(read.keys.clone());
        self.at = Some(*hour);
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one table's read, its lane and the reply's edits")]
    pub(super) fn from_lane(&mut self, read: &Read, sample: bool, view: &mut SharedView, edits: &mut Edits, count: &mut u64, lane: &Lane, step: Option<(&Step, &Changes)>, mode: CaptureMode) -> io::Result<()> {
        let table = read.table;
        let (status, sampled) = lane.status();
        let other = match status {
            Status::Complete | Status::Truncated { .. } => None,
            Status::NotRecorded => Some(TableView::NotRecorded { mode }),
            Status::Unavailable { .. } => Some(TableView::Unavailable { reason: lane.reason().and_then(|reason| std::str::from_utf8(reason).ok()).unwrap_or("A table's reason is not text").to_owned() }),
        };
        if let Some(made) = other {
            if *view != made {
                *view = made.into();
                edits.replaced.insert(table);
            }
            return Ok(());
        }
        let cells = lane.view();
        let continues = |step: &Step, change: &Changes| status == Status::Complete && self.at.is_some_and(|at| at.checked_add(1) == Some(step.first())) && !change.reset();
        let (held, change) = match (view.held(), step) {
            (TableView::Complete(held), Some((step, change))) if continues(step, change) => (held, change),
            _ => return self.whole(read, &cells, status, lane.read_rows(), sampled, sample, view, edits, count),
        };
        let mut dirty: BTreeMap<u128, u64> = BTreeMap::new();
        for &key in change.deleted().keys().into_iter().flatten().chain(change.inserted().keys().into_iter().flatten()) {
            if read.keys.contains(key) { dirty.insert(key, u64::MAX); }
        }
        let renumber = change.renumber();
        let now = |slot: u32| match renumber {
            Some(old) => old.binary_search(&slot).ok().map(|at| at as u32),
            None => Some(slot),
        };
        for column in change.columns() {
            let mask = 1u64 << column.column();
            if (self.fields | self.validity) & mask == 0 { continue; }
            let validity = (self.fields & mask == 0).then(|| lane.cells().column(column.column())).flatten();
            for slot in column.slots().iter().filter_map(|&slot| now(slot)) {
                if validity.is_some_and(|cells| cells.recorded(slot)) { continue; }
                if let Some(key) = lane.cells().key(slot).filter(|&k| read.keys.contains(k)) {
                    *dirty.entry(key).or_default() |= mask;
                }
            }
        }
        if dirty.is_empty() {
            if view.sampled_at() != sampled {
                *view = view.clone().with_sampled_at(sampled);
                if sample { edits.replaced.insert(table); }
            }
            return Ok(());
        }
        let mut keyed = Vec::new();
        let mut deleted = Vec::new();
        for &key in dirty.keys() {
            if let Some(slot) = cells.slot(key) { keyed.push((key, slot)); }
            else if held.contains_key(key) { deleted.push(key); }
        }
        *count += keyed.len() as u64;
        let fresh = match cells_rows_of(table, &cells, &keyed, self.format, self.fields | self.validity) {
            Ok(rows) => rows,
            Err(e) if Unsayable::of(&e).is_some() => return self.whole(read, &cells, status, None, sampled, sample, view, edits, count),
            Err(e) => return Err(e),
        };
        if let TableView::Complete(old) = &mut **view {
            let previous = std::mem::replace(old, TableRows::empty(table));
            *old = previous.apply_changes(RowChanges::new(fresh, deleted))?;
        }
        *view = view.clone().with_sampled_at(sampled);
        edits.tables.insert(table, dirty);
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one table's cells at an hour and the reply's edits")]
    fn whole(&self, read: &Read, cells: &StoreView<'_>, status: Status, read_rows: Option<&[u32]>, sampled: Option<i32>, sample: bool, view: &mut SharedView, edits: &mut Edits, count: &mut u64) -> io::Result<()> {
        let mut keyed = Vec::new();
        let mut keep = |slot| {
            if let Some(key) = cells.key(slot).filter(|&k| read.keys.contains(k)) { keyed.push((key, slot)); }
        };
        if let Some(rows) = read_rows { rows.iter().copied().for_each(keep); }
        else if let Keys::These(keys) = &read.keys { keyed.extend(keys.iter().filter_map(|&key| Some((key, cells.slot(key)?)))); }
        else { cells.rows().for_each(&mut keep); }
        keyed.sort_unstable();
        *count += keyed.len() as u64;
        let made = cells_rows_of(read.table, cells, &keyed, self.format, self.fields | self.validity).map(|rows| match status {
            Status::Truncated { missing } => TableView::Truncated { rows, missing },
            _ => TableView::Complete(rows),
        });
        settle(read.table, made, sampled, sample, view, edits)
    }

    #[expect(clippy::too_many_arguments, reason = "one raw table's read, lane and edits")]
    fn read_raw(
        &mut self,
        input: &mut Input<'_>,
        read: &Read,
        sample: bool,
        raw: &mut RawTables,
        decode: fn(TableId, &StoreView<'_>, u32, u128) -> Option<RawRow>,
        edits: &mut Edits,
    ) -> io::Result<()> {
        if self.selection.as_ref() != Some(&read.keys) {
            self.at = None;
        }
        let Input { pool, hour, count, step, placed, mode, .. } = input;
        let (run, table) = (self.run, read.table.index());
        match step.and_then(|step| self.lane(read.table, *hour, step)) {
            Some((step, lane, change)) => self.raw_from_lane(read, sample, raw, decode, edits, count, lane, Some((step, change)), *mode)?,
            None => {
                let warm = if step.is_some_and(|step| step.lanes_table(table)) {
                    let columns = self.columns.clone();
                    let wake = step.and_then(|step| step.wake.cloned());
                    pool.warm(run, table, &columns, *hour, wake, |lane| self.raw_from_lane(read, sample, raw, decode, edits, count, lane, None, *mode))
                } else { Warm::Cold };
                match warm {
                    Warm::Read(result) => result?,
                    Warm::Warming => return Err(io::Error::new(io::ErrorKind::WouldBlock, format!("{:?} is being placed beside the step", read.table))),
                    Warm::Cold => {
                        let placed = placed.get_or_insert_with(|| pool.take(run, *hour));
                        let lane = placed.lane(table, &self.columns, *hour)?;
                        self.raw_from_lane(read, sample, raw, decode, edits, count, lane, None, *mode)?;
                    }
                }
            }
        }
        self.selection = Some(read.keys.clone());
        self.at = Some(*hour);
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one raw table's cells at an hour and edits")]
    fn raw_from_lane(
        &mut self,
        read: &Read,
        sample: bool,
        raw: &mut RawTables,
        decode: fn(TableId, &StoreView<'_>, u32, u128) -> Option<RawRow>,
        edits: &mut Edits,
        count: &mut u64,
        lane: &Lane,
        step: Option<(&Step, &Changes)>,
        mode: CaptureMode,
    ) -> io::Result<()> {
        let table = read.table;
        let (status, sampled) = lane.status();
        let non_complete = match status {
            Status::Complete => None,
            Status::Truncated { missing } => {
                let cells = lane.view();
                let mut keyed = Vec::new();
                let mut keep = |slot| {
                    if let Some(key) = cells.key(slot).filter(|&k| read.keys.contains(k)) { keyed.push((key, slot)); }
                };
                if let Some(rows) = lane.read_rows() { rows.iter().copied().for_each(keep); }
                else if let Keys::These(keys) = &read.keys { keyed.extend(keys.iter().filter_map(|&key| Some((key, cells.slot(key)?)))); }
                else { cells.rows().for_each(&mut keep); }
                keyed.sort_unstable();
                *count += keyed.len() as u64;
                match cells_rows_of(table, &cells, &keyed, self.format, self.fields | self.validity) {
                    Ok(_) => Some(RawTable::Truncated { missing }),
                    Err(e) if Unsayable::of(&e).is_some() => Some(RawTable::Unavailable { reason: e.to_string() }),
                    Err(e) => return Err(e),
                }
            }
            Status::NotRecorded => Some(RawTable::NotRecorded { mode }),
            Status::Unavailable { .. } => Some(RawTable::Unavailable {
                reason: lane.reason().and_then(|reason| std::str::from_utf8(reason).ok()).unwrap_or("A table's reason is not text").to_owned(),
            }),
        };
        if let Some(made) = non_complete {
            let same = match (raw.get_mut(table), &made) {
                (Some(RawTable::Truncated { missing: a }), RawTable::Truncated { missing: b }) => a == b,
                (Some(RawTable::NotRecorded { mode: a }), RawTable::NotRecorded { mode: b }) => a == b,
                (Some(RawTable::Unavailable { reason: a }), RawTable::Unavailable { reason: b }) => a == b,
                _ => false,
            };
            if !same {
                raw.insert(table, made, None);
                edits.replaced.insert(table);
            }
            return Ok(());
        }
        let cells = lane.view();
        let continues = |step: &Step, change: &Changes| self.at.is_some_and(|at| at.checked_add(1) == Some(step.first())) && !change.reset();
        if let (Some(RawTable::Complete(rows)), Some((step, change))) = (raw.get_mut(table), step)
            && continues(step, change)
        {
            let mut dirty: BTreeMap<u128, u64> = BTreeMap::new();
            for &key in change.deleted().keys().into_iter().flatten().chain(change.inserted().keys().into_iter().flatten()) {
                if read.keys.contains(key) { dirty.insert(key, u64::MAX); }
            }
            let renumber = change.renumber();
            let now = |slot: u32| match renumber {
                Some(old) => old.binary_search(&slot).ok().map(|at| at as u32),
                None => Some(slot),
            };
            for column in change.columns() {
                let mask = 1u64 << column.column();
                if self.fields & mask == 0 { continue; }
                for slot in column.slots().iter().filter_map(|&slot| now(slot)) {
                    if let Some(key) = lane.cells().key(slot).filter(|&k| read.keys.contains(k)) {
                        *dirty.entry(key).or_default() |= mask;
                    }
                }
            }
            if dirty.is_empty() {
                if raw.sampled_at(table) != sampled {
                    raw.set_sampled_at(table, sampled);
                    if sample { edits.replaced.insert(table); }
                }
                return Ok(());
            }
            for &key in dirty.keys() {
                if let Some(slot) = cells.slot(key) {
                    *count += 1;
                    if let Some(row) = decode(table, &cells, slot, key) {
                        rows.insert(key, row);
                    } else {
                        rows.remove(&key);
                    }
                } else {
                    rows.remove(&key);
                }
            }
            raw.set_sampled_at(table, sampled);
            edits.tables.insert(table, dirty);
            return Ok(());
        }
        let mut keyed = Vec::new();
        let mut keep = |slot| {
            if let Some(key) = cells.key(slot).filter(|&k| read.keys.contains(k)) { keyed.push((key, slot)); }
        };
        if let Some(rows) = lane.read_rows() { rows.iter().copied().for_each(keep); }
        else if let Keys::These(keys) = &read.keys { keyed.extend(keys.iter().filter_map(|&key| Some((key, cells.slot(key)?)))); }
        else { cells.rows().for_each(&mut keep); }
        keyed.sort_unstable();
        *count += keyed.len() as u64;
        let mut rows = BTreeMap::new();
        for (key, slot) in keyed {
            if let Some(row) = decode(table, &cells, slot, key) {
                rows.insert(key, row);
            }
        }
        raw.insert(table, RawTable::Complete(rows), sampled);
        edits.replaced.insert(table);
        Ok(())
    }
}

fn settle(table: TableId, made: io::Result<TableView>, sampled: Option<i32>, sample: bool, view: &mut SharedView, edits: &mut Edits) -> io::Result<()> {
    let made = match made {
        Ok(made) => made,
        Err(e) if Unsayable::of(&e).is_some() => TableView::Unavailable { reason: e.to_string() },
        Err(e) => return Err(e),
    };
    if let (TableView::Complete(held), TableView::Complete(rows)) = (view.held(), &made) {
        let changes = rows.changes_since(held)?.ok_or_else(|| io::Error::other(format!("{table:?} rows of another table")))?;
        let keys: BTreeMap<u128, u64> = changes.rows().iter().map(|row| row.key()).chain(changes.deleted().iter().copied()).map(|key| (key, u64::MAX)).collect();
        if keys.is_empty() {
            if view.sampled_at() != sampled {
                *view = view.clone().with_sampled_at(sampled);
                if sample { edits.replaced.insert(table); }
            }
            return Ok(());
        }
        *view = SharedView::from(made).with_sampled_at(sampled);
        edits.tables.insert(table, keys);
        return Ok(());
    }
    *view = match made {
        TableView::Complete(rows) => SharedView::from(TableView::Complete(rows)).with_sampled_at(sampled),
        made => made.into(),
    };
    edits.replaced.insert(table);
    Ok(())
}

type Projections = HashMap<TableId, Result<Projection, String>>;

pub(super) struct Input<'a> {
    pub(super) pool: &'a Arc<Pool>,
    pub(super) run: usize,
    pub(super) hour: i32,
    pub(super) mode: CaptureMode,
    pub(super) count: &'a mut u64,
    pub(super) step: Option<ProductStep<'a>>,
    pub(super) placed: Option<Taken>,
}

impl Input<'_> {
    pub(super) fn frame_into<P: AtHour>(&mut self, reads: &[Read], raw: &mut RawTables, branch: u64) -> io::Result<DecodedFrame> {
        let mut frame = empty(branch, self.hour);
        let mut tables = Projections::new();
        let mut raw_rows = 0u64;
        for read in reads {
            let fields = P::columns(read.table);
            if P::raw_table(read.table) {
                let before = *self.count;
                self.read_raw(&mut tables, read, raw, P::decode_raw_row, || (fields.clone(), fields.clone()), false, &mut Edits::default())?;
                raw_rows += *self.count - before;
            } else {
                self.read(&mut tables, read, &mut frame.tables[read.table.index()], || (fields.clone(), fields.clone()), false, &mut Edits::default())?;
            }
        }
        count_frame(&frame, reads);
        if raw_rows > 0 {
            count_rows(|| raw_rows);
        }
        Ok(frame)
    }

    fn read(&mut self, tables: &mut Projections, read: &Read, view: &mut SharedView, columns: impl Fn() -> (Option<Vec<usize>>, Option<Vec<usize>>), sample: bool, edits: &mut Edits) -> io::Result<()> {
        let table = read.table;
        let (selected, cold) = columns();
        let made = |selected, cold| match Projection::new(&self.pool.pages, self.run, table, selected, cold, false) {
            Ok(projection) => Ok(Ok(projection)),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(Err(e.to_string())),
            Err(e) => Err(e),
        };
        let slot = match tables.entry(table) {
            Entry::Occupied(mut entry) if entry.get().as_ref().is_ok_and(|held| held.fields != field_mask(table, selected.as_deref())) => {
                *entry.get_mut() = made(selected, cold)?;
                entry.into_mut()
            }
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(made(selected, cold)?),
        };
        let reason = match slot {
            Ok(projection) => match projection.read(self, read, sample, view, edits) {
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    tables.remove(&table);
                    e.to_string()
                }
                result => return result,
            },
            Err(reason) if view.held().is_not_selected() => reason.clone(),
            Err(_) => return Ok(()),
        };
        *view = TableView::Unavailable { reason }.into();
        edits.replaced.insert(table);
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one raw table read with its projection and edits")]
    fn read_raw(
        &mut self,
        tables: &mut Projections,
        read: &Read,
        raw: &mut RawTables,
        decode: fn(TableId, &StoreView<'_>, u32, u128) -> Option<RawRow>,
        columns: impl Fn() -> (Option<Vec<usize>>, Option<Vec<usize>>),
        sample: bool,
        edits: &mut Edits,
    ) -> io::Result<()> {
        let table = read.table;
        let (selected, cold) = columns();
        let made = |selected, cold| match Projection::new(&self.pool.pages, self.run, table, selected, cold, true) {
            Ok(projection) => Ok(Ok(projection)),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(Err(e.to_string())),
            Err(e) => Err(e),
        };
        let slot = match tables.entry(table) {
            Entry::Occupied(mut entry) if entry.get().as_ref().is_ok_and(|held| held.fields != field_mask(table, selected.as_deref())) => {
                *entry.get_mut() = made(selected, cold)?;
                entry.into_mut()
            }
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(made(selected, cold)?),
        };
        let reason = match slot {
            Ok(projection) => match projection.read_raw(self, read, sample, raw, decode, edits) {
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    tables.remove(&table);
                    e.to_string()
                }
                result => return result,
            },
            Err(reason) => {
                if matches!(raw.get_mut(table), Some(RawTable::Unavailable { reason: held }) if held == reason) {
                    return Ok(());
                }
                reason.clone()
            }
        };
        raw.insert(table, RawTable::Unavailable { reason }, None);
        edits.replaced.insert(table);
        Ok(())
    }

    pub(super) fn live_values(&mut self, live: &Live, frame: &DecodedFrame, covered: impl Fn(&Group) -> bool) -> io::Result<Value> {
        let mut from_frame: Option<Vec<Vec<dashboard_follow::live::CellValue>>> = None;
        let mut states = Vec::with_capacity(live.groups.len());
        for (at, group) in live.groups.iter().enumerate() {
            let values = if covered(group) {
                let held = from_frame.get_or_insert_with(|| live.values(frame));
                std::mem::take(&mut held[at])
            } else {
                self.raw_group_values(group)?
            };
            states.push(GroupState::Values(values));
        }
        Ok(live.json(&states))
    }

    fn raw_group_values(&mut self, group: &Group) -> io::Result<Vec<dashboard_follow::live::CellValue>> {
        use dashboard_follow::live::CellValue;
        let width = group.columns.len();
        let table = group.table.index();
        if let Some(reason) = self.pool.pages.withheld(table) {
            return Ok(vec![CellValue::Missing(unavailable_table(table, reason)); group.keys.len() * width]);
        }
        let columns = recorded_columns(&self.pool.pages, group.table, &group.columns);
        if let Some(lane) = self.step.and_then(|step| {
            let lane = step.lanes.get(table)?;
            let change = step.step.table(table)?;
            (lane.run() == self.run
                && change.head() == Some(self.hour)
                && change.log().is_none()
                && lane.keyed()
                && columns.iter().all(|column| lane.requested().contains(column)))
                .then_some(lane)
        }) {
            return read_group_cells(group, lane);
        }
        let warm = if self.step.is_some_and(|step| step.lanes_table(table)) {
            let wake = self.step.and_then(|step| step.wake.cloned());
            self.pool.warm(self.run, table, &columns, self.hour, wake, |lane| read_group_cells(group, lane))
        } else {
            Warm::Cold
        };
        let result = match warm {
            Warm::Read(result) => result,
            Warm::Warming => return Err(io::Error::new(io::ErrorKind::WouldBlock, format!("{:?} is being placed beside the step", group.table))),
            Warm::Cold => {
                let placed = self.placed.get_or_insert_with(|| self.pool.take(self.run, self.hour));
                placed.lane(table, &columns, self.hour).and_then(|lane| read_group_cells(group, lane))
            }
        };
        match result {
            Ok(values) => Ok(values),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                Ok(vec![CellValue::Missing(unavailable_table(table, &error.to_string())); group.keys.len() * width])
            }
            Err(error) => Err(error),
        }
    }
}

fn unavailable_table(table: usize, reason: &str) -> String {
    dashboard_follow::unrecorded(table, Status::Unavailable { reason: 0 }, Some(reason)).unwrap_or_else(|| reason.to_owned())
}

fn recorded_columns(pages: &WorldPages, table: TableId, columns: &[usize]) -> Vec<usize> {
    let recorded = pages.recording().columns().get(table.index()).copied().unwrap_or(0);
    let mut out: Vec<usize> = columns.iter().copied().filter(|&column| column < recorded).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn read_group_cells(group: &Group, lane: &Lane) -> io::Result<Vec<dashboard_follow::live::CellValue>> {
    use dashboard_follow::live::{CellValue, NO_ROW, NO_VALUE, NOT_RECORDED};
    use replay_columnar::store::Value as StoreValue;
    let width = group.columns.len();
    let table = group.table.index();
    if let Some(reason) = lane.withheld() {
        return Ok(vec![CellValue::Missing(unavailable_table(table, reason)); group.keys.len() * width]);
    }
    let status = lane.status().0;
    let text = lane.reason().and_then(|bytes| std::str::from_utf8(bytes).ok());
    if let Some(reason) = dashboard_follow::unrecorded(table, status, text) {
        return Ok(vec![CellValue::Missing(reason); group.keys.len() * width]);
    }
    let cells = lane.view();
    let mut out = Vec::with_capacity(group.keys.len() * width);
    for &key in &group.keys {
        let Some(slot) = cells.slot(key) else {
            out.extend(std::iter::repeat_n(CellValue::Missing(NO_ROW.to_owned()), width));
            continue;
        };
        for &column in &group.columns {
            out.push(match cells.value(column, slot) {
                StoreValue::Number(bits) => CellValue::Number(bits as i64),
                StoreValue::Missing => CellValue::Missing(NO_VALUE.to_owned()),
                StoreValue::Unrecorded => CellValue::Missing(NOT_RECORDED.to_owned()),
                StoreValue::Text(_) | StoreValue::List(_) => {
                    let field = group.table.field_name(column).unwrap_or("?");
                    CellValue::Missing(format!("{:?}'s {field} is not a number, which a live cell does not read", group.table))
                }
            });
        }
    }
    Ok(out)
}

#[derive(Default)]
struct Supplemental {
    columns: HashMap<TableId, Vec<usize>>,
}

impl Supplemental {
    fn set(&mut self, pages: &WorldPages, live: &Live, covered: impl Fn(&Group) -> bool) {
        self.columns.clear();
        let uncovered: BTreeSet<TableId> = live.groups.iter().filter(|group| !covered(group)).map(|group| group.table).collect();
        for group in live.groups.iter().filter(|group| uncovered.contains(&group.table)) {
            let columns = self.columns.entry(group.table).or_default();
            columns.extend(recorded_columns(pages, group.table, &group.columns));
            columns.sort_unstable();
            columns.dedup();
        }
    }

    fn values(&mut self, live: &Live, frame: &mut DecodedFrame, input: &mut Input<'_>) -> io::Result<Value> {
        let columns = &self.columns;
        input.live_values(live, frame, |group| !columns.contains_key(&group.table))
    }
}

struct Stream {
    tables: Projections,
    raw: RawTables,
    extra: Supplemental,
    plans: Vec<(Vec<Read>, i32)>,
    frame: Box<DecodedFrame>,
    run: u32,
    identity_through: i32,
    live: Arc<Live>,
    data: Arc<PanelData>,
    values_at: Option<i32>,
    reply: Option<Box<dyn std::any::Any + Send>>,
    sampled: Option<BTreeSet<TableId>>,
    rebuild: bool,
}

pub struct Held<'a>(&'a mut Arc<PanelData>);

impl std::ops::Deref for Held<'_> {
    type Target = Value;
    fn deref(&self) -> &Value {
        static NONE: Value = Value::Null;
        self.0.as_ref().as_ref().unwrap_or(&NONE)
    }
}

impl std::ops::DerefMut for Held<'_> {
    fn deref_mut(&mut self) -> &mut Value {
        let data = Arc::make_mut(self.0);
        if data.is_err() { *data = Ok(Value::Null); }
        match data { Ok(value) => value, Err(_) => unreachable!("made a value above") }
    }
}

fn current_values(state: &mut Stream, input: &mut Input<'_>) -> io::Result<Arc<PanelData>> {
    if state.values_at != Some(input.hour) && state.data.is_ok() {
        if !state.live.is_empty() {
            let live = state.extra.values(&state.live, &mut state.frame, input)?;
            if let Ok(Value::Object(data)) = Arc::make_mut(&mut state.data) { data.insert("live".into(), live); }
        }
        state.values_at = Some(input.hour);
    }
    Ok(state.data.clone())
}

fn sampling<T>(frame: &DecodedFrame, sampled: &mut Option<BTreeSet<TableId>>, compute: impl FnOnce() -> T) -> T {
    let (value, views) = replay_frame::frame::view_reads::samples(compute);
    let tables = sampled.get_or_insert_with(BTreeSet::new);
    for view in &views {
        if let Some(&table) = TableId::ALL.iter().find(|table| view.is(&frame.tables[table.index()])) { tables.insert(table); }
    }
    value
}

pub(super) fn reader<P: AtHour>(source: &Source, pool: Arc<Pool>, params: Arc<P::Params>, context: Arc<P::Context>, query: Query<'_>, warnings: Vec<String>) -> ProductReader
where P::Context: Sync, P::Params: Sync {
    let (timeline, holders, executable) = (query.timeline, query.holders, source.tables());
    use dashboard_query::identity::{Country, Holder};
    let owner = P::owner(&params);
    let owned = |tag: &str, run: &dashboard_query::identity::HolderRun| match owner {
        None => true,
        Some(Country::Tag(owned)) => owned == tag,
        Some(Country::Slot(slot)) => matches!(run.holder, Holder::Found { slot: held, .. } if held == *slot),
    };
    let mut identity_edges: Vec<i32> = holders.tags.iter()
        .flat_map(|(tag, runs)| runs.iter().filter(move |run| owned(tag, run)))
        .flat_map(|r| [Some(r.first), r.last.checked_add(1)].into_iter().flatten()).collect();
    identity_edges.sort_unstable();
    identity_edges.dedup();
    let stream = Mutex::new(None::<Stream>);
    Arc::new(move |hour, values, cancelled, step| {
        let mut held = stream.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut count = 0;
        let result = engine_executable::executable_tables::scope(executable, || -> io::Result<_> {
            let run = timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{} hour is not recorded", P::TITLE)))?.run;
            let read_run = pool.pages.run_of(run).ok_or_else(|| dashboard_core::refused(format!("{} hour is in a run the recording cannot read", P::TITLE)))?;
            let mut input = Input { pool: &pool, run: read_run, hour, mode: capture_mode(&pool.pages)?, count: &mut count, step: step.filter(|s| s.step.last() == hour), placed: None };
            if let Some(state) = held.as_mut().filter(|s| s.frame.hour == hour && s.run == run && !s.rebuild) {
                return if values { current_values(state, &mut input) } else { Ok(state.data.clone()) };
            }
            let owner = holder::<P>(&holders, &params, hour)?;
            let seed = held.as_ref().is_none_or(|s| s.run != run || hour < s.frame.hour);
            if seed {
                *held = Some(Stream { tables: HashMap::new(), raw: RawTables::default(), extra: Supplemental::default(), values_at: None, plans: Vec::new(), frame: Box::new(empty(timeline.id, hour)), run, identity_through: hour, live: Arc::default(), data: Arc::new(Err(String::new())), reply: None, sampled: None, rebuild: true });
            }
            let state = held.as_mut().expect("seeded");
            let previous_hour = state.frame.hour;
            state.frame.hour = hour;
            let identity_changed = seed || hour > state.identity_through;
            if identity_changed {
                state.identity_through = identity_edges.get(identity_edges.partition_point(|&h| h <= hour)).map_or(i32::MAX, |&h| h - 1);
            }
            let mut edits = Edits::default();
            let mut selected = BTreeSet::new();
            let at = ReadsAt { holders: &holders, context: &*context, run, hour, since: (!seed).then_some(previous_hour) };
            let mut tried: HashMap<TableId, Vec<u64>> = HashMap::new();
            let clocked = hour != previous_hour && P::clock_dependent(&state.plans, &context);
            loop {
                selected.clear();
                for stage in 0.. {
                    crate::metadata::check_cancel(cancelled)?;
                    if stage >= state.plans.len() {
                        let planned = sampling(&state.frame, &mut state.sampled, || P::retained_reads_raw(&params, owner, stage, &state.frame, &state.raw, &at));
                        state.plans.push(planned);
                    } else if state.rebuild || identity_changed || clocked || hour > state.plans[stage].1 || P::plan_changes(stage, &edits, &state.frame, &at) {
                        state.plans[stage] = sampling(&state.frame, &mut state.sampled, || P::retained_reads_raw(&params, owner, stage, &state.frame, &state.raw, &at));
                    }
                    let (asks, _) = &state.plans[stage];
                    if asks.is_empty() { state.plans.truncate(stage + 1); break; }
                    for read in asks {
                        let table = read.table;
                        selected.insert(table);
                        let sample = state.sampled.as_ref().is_none_or(|sampled| sampled.contains(&table));
                        if P::raw_table(table) {
                            input.read_raw(&mut state.tables, read, &mut state.raw, P::decode_raw_row, || (P::projection(&params, table, &state.plans, &context), P::columns(table)), sample, &mut edits)?;
                        } else {
                            let view = &mut state.frame.tables[table.index()];
                            if !seed && P::compares_previous(table) { edits.previous.entry(table).or_insert_with(|| view.clone()); }
                            input.read(&mut state.tables, read, view, || (P::projection(&params, table, &state.plans, &context), P::columns(table)), sample, &mut edits)?;
                        }
                    }
                }
                let projected = |table: TableId| field_mask(table, P::projection(&params, table, &state.plans, &context).as_deref());
                let moved: Vec<(TableId, u64, u64)> = state.tables.iter()
                    .filter_map(|(&table, held)| held.as_ref().ok().filter(|held| selected.contains(&table) && held.fields != projected(table)).map(|held| (table, held.fields, projected(table))))
                    .collect();
                if moved.is_empty() { break; }
                for (table, read, asked) in moved {
                    let masks = tried.entry(table).or_default();
                    masks.push(read);
                    if masks.contains(&asked) { return Err(io::Error::other(format!("{} reads {table:?} under projections that do not settle", P::TITLE))); }
                }
            }
            state.tables.retain(|table, _| {
                if selected.contains(table) { true } else {
                    state.frame.tables[table.index()] = not_selected();
                    edits.replaced.insert(*table);
                    false
                }
            });
            state.raw.retain(|table| selected.contains(table));
            let cells = Cells::default();
            let at = At { owner, holders: &holders, context: &*context, cancelled, hour, live: &cells };
            let all_complete = selected.iter().all(|table| {
                if P::raw_table(*table) {
                    matches!(state.raw.get_mut(*table), Some(RawTable::Complete(_)))
                } else {
                    matches!(state.frame.tables[table.index()].held(), TableView::Complete(_))
                }
            });
            if !state.rebuild && !identity_changed && all_complete {
                let changed = !edits.tables.is_empty() || !edits.replaced.is_empty();
                if !changed && !P::clock_dependent(&state.plans, &context) {
                    return if values { current_values(state, &mut input) } else { Ok(state.data.clone()) };
                }
                if state.data.is_ok() {
                    let kept = state.data.clone();
                    if P::advance(&state.frame, &params, &at, &edits, &state.live, &mut Held(&mut state.data))? {
                        state.reply = None;
                        return if values { current_values(state, &mut input) } else { Ok(state.data.clone()) };
                    }
                    state.data = kept;
                }
            }
            let mut own = OwnRefusals::default();
            let mut reply = sampling(&state.frame, &mut state.sampled, || own_refusals(&mut own, || P::reply_raw(&state.frame, &state.raw, &params, &at)))?;
            finish(reply.warnings_mut(), &own, &warnings);
            let live = cells.finish();
            let projected = |table: TableId| P::projection(&params, table, &state.plans, &context);
            state.extra.set(&pool.pages, &live, |g| !P::raw_table(g.table) && state.plans.iter().flat_map(|(reads, _)| reads).rev().find(|r| r.table == g.table)
                .is_some_and(|r| covers(r, g) && projected(g.table).is_none_or(|fields| g.columns.iter().all(|column| fields.contains(column)))));
            if !seed && !state.rebuild && state.data.is_ok() && *state.live == live
                && state.reply.as_ref().and_then(|last| last.downcast_ref::<P::Reply>()).is_some_and(|last| *last == reply) {
                return if values { current_values(state, &mut input) } else { Ok(state.data.clone()) };
            }
            state.live = Arc::new(live);
            let live = if state.live.is_empty() { None } else { Some(state.extra.values(&state.live, &mut state.frame, &mut input)?) };
            let data = serde_json::to_value(WithLive { reply: &reply, live })?;
            state.reply = Some(Box::new(reply));
            state.rebuild = false;
            state.values_at = Some(hour);
            state.data = Arc::new(Ok(data));
            Ok(state.data.clone())
        });
        let pending = result.as_ref().is_err_and(|e| e.kind() == io::ErrorKind::WouldBlock) && held.is_some();
        let result = match result {
            Ok(data) => data,
            Err(_) if pending => {
                let state = held.as_mut().expect("held while pending");
                state.rebuild = true;
                state.data.clone()
            }
            Err(e) => { *held = None; Arc::new(Err(e.to_string())) }
        };
        let fresh = !pending && held.as_ref().is_some_and(|s| s.values_at == Some(hour));
        let live = held.as_ref().map(|s| s.live.clone()).filter(|live| !live.is_empty());
        let lanes = held.as_ref().map_or_else(Vec::new, |s| {
            let mut declared: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for (table, projection) in &s.tables {
                if let Ok(projection) = projection {
                    declared.entry(table.index()).or_default().extend(&projection.columns);
                }
            }
            for (table, columns) in &s.extra.columns {
                if pool.pages.withheld(table.index()).is_none() {
                    declared.entry(table.index()).or_default().extend(columns);
                }
            }
            declared
                .into_iter()
                .map(|(table, mut columns)| {
                    columns.sort_unstable();
                    columns.dedup();
                    Declared { table, columns, keyed: true, passing: false, hourly: false }
                })
                .collect()
        });
        dashboard_follow::panel::ProductRead { value: result, rows: count, fresh, pending, live, lanes }
    })
}
