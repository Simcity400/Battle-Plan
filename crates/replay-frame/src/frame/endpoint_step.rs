use super::*;
use crate::{
    FORMAT_VERSION,
    codec::{put_bytes, put_string},
    rows::{EndpointRows, full_mask},
    value::Value,
};

mod writer;
pub use writer::EndpointWriter;
#[cfg(any(test, feature = "test-support"))]
pub use writer::key_by_key;

const MAGIC: &[u8; 4] = b"H4ES";
const VERSION: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub branch: BranchId,
    pub hour: Hour,
}
impl Point {
    fn of(frame: &DecodedFrame) -> Self {
        Self {
            branch: frame.branch,
            hour: frame.hour,
        }
    }
    fn read(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            branch: input.u64()?,
            hour: input.i32()?,
        })
    }
    fn write(self, out: &mut Vec<u8>) -> io::Result<()> {
        put(&self.branch, out)?;
        put(&self.hour, out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BoundaryKind {
    CompleteReset,
    Delta,
    Partial,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipBoundary {
    pub table: TableId,
    pub hour: Hour,
    pub kind: BoundaryKind,
    pub inserted: Vec<u128>,
    pub deleted: Vec<u128>,
}

#[derive(Debug)]
enum Operation {
    Carry,
    Complete(TableRows),
    Patch {
        rows: EndpointRows,
        deleted: Vec<u128>,
    },
    Truncated {
        rows: TableRows,
        missing: u32,
    },
    Unavailable(String),
    NotRecorded(CaptureMode),
}
#[derive(Debug)]
struct Section {
    table: TableId,
    sample: Option<Hour>,
    operation: Operation,
    boundaries: Vec<MembershipBoundary>,
}

/// An owned, fully bounded payload. No borrowed read buffer survives decoding.
#[derive(Debug)]
pub struct EndpointStep {
    pub from: Option<Point>,
    pub to: Point,
    pub metrics: Metrics,
    sections: Vec<Section>,
}

impl EndpointStep {
    pub fn boundaries(&self) -> impl Iterator<Item = &MembershipBoundary> {
        self.sections.iter().flat_map(|section| &section.boundaries)
    }

    /// The tables it holds a section for, in ascending table order.
    pub fn tables(&self) -> impl Iterator<Item = TableId> + '_ {
        self.sections.iter().map(|section| section.table)
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut input = Decoder::new(bytes);
        if input.take(4)? != MAGIC || input.u16()? != VERSION || input.u16()? != FORMAT_VERSION {
            return Err(invalid("unsupported endpoint schema"));
        }
        let from = if input.bool()? {
            Some(Point::read(&mut input)?)
        } else {
            None
        };
        let to = Point::read(&mut input)?;
        if from.is_some_and(|from| from.branch != to.branch || from.hour >= to.hour) {
            return Err(invalid("endpoint does not advance its branch"));
        }
        let metrics = Metrics {
            total_ns: input.u64()?,
            table_ns: Value::decode(&mut input)?,
            pool_bytes: input.u64()?,
            dropped_frames: input.u64()?,
        };
        let count = input.count(6)?;
        if count > TABLE_COUNT {
            return Err(invalid("endpoint has too many tables"));
        }
        let mut sections = allocation::vec(count)?;
        let mut previous = None;
        for _ in 0..count {
            let index = usize::from(input.u16()?);
            let table = *TableId::ALL
                .get(index)
                .ok_or_else(|| invalid("unknown endpoint table"))?;
            if previous.is_some_and(|old| index <= old) {
                return Err(invalid("endpoint tables must be unique and ascending"));
            }
            previous = Some(index);
            let length = input.u32()? as usize;
            let mut section = Decoder::new(input.take(length)?);
            let sample: Option<Hour> = Value::decode(&mut section)?;
            let operation = match section.u8()? {
                0 if from.is_some() => Operation::Carry,
                1 => {
                    let rows = table.decode_checkpoint_rows(&mut section)?;
                    validate_complete(table, &rows, to.hour)?;
                    Operation::Complete(rows)
                }
                2 if from.is_some() => Operation::Patch {
                    rows: table.decode_endpoint_rows(&mut section)?,
                    deleted: read_keys(table, &mut section)?,
                },
                3 => {
                    let missing = section.u32()?;
                    if missing == 0 {
                        return Err(invalid("truncated endpoint has no missing rows"));
                    }
                    let rows = table.decode_checkpoint_rows(&mut section)?;
                    validate_rows(table, &rows)?;
                    Operation::Truncated { rows, missing }
                }
                4 => {
                    let reason = section.string()?;
                    if reason.is_empty() {
                        return Err(invalid("unavailable endpoint needs a reason"));
                    }
                    Operation::Unavailable(reason)
                }
                5 => Operation::NotRecorded(CaptureMode::from_byte(section.u8()?)?),
                _ => return Err(invalid("invalid endpoint operation")),
            };
            if sample.is_some_and(|hour| {
                hour > to.hour
                    || matches!(
                        operation,
                        Operation::Truncated { .. }
                            | Operation::Unavailable(_)
                            | Operation::NotRecorded(_)
                    )
            }) {
                return Err(invalid("invalid endpoint sample hour"));
            }
            let boundaries = read_boundaries(table, from, to, &mut section)?;
            section.finish()?;
            sections.push(Section {
                table,
                sample,
                operation,
                boundaries,
            });
        }
        input.finish()?;
        Ok(Self {
            from,
            to,
            metrics,
            sections,
        })
    }

    pub(super) fn replaced_change_tables(&self) -> [bool; TABLE_COUNT] {
        let mut replaced = [false; TABLE_COUNT];
        for section in &self.sections {
            replaced[section.table.index()] =
                !matches!(section.operation, Operation::Carry) || !section.boundaries.is_empty();
        }
        replaced
    }

    pub(super) fn prepare_change_facts(
        &self,
        previous: Option<&DecodedFrame>,
        budget: &allocation::credit::MemoryBudget,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<(
        [Option<Arc<ChangeFacts>>; TABLE_COUNT],
        [Option<Arc<FieldStamp>>; TABLE_COUNT],
    )> {
        use super::decoder::{change_bytes, field_stamp, stamp_bytes};
        if self.from != previous.map(Point::of) {
            return Err(invalid("endpoint baseline differs"));
        }
        let mut required = 0;
        for section in &self.sections {
            let bytes = match &section.operation {
                Operation::Complete(_) => stamp_bytes(),
                Operation::Patch { rows, deleted } => {
                    allocation::add(stamp_bytes(), change_bytes(rows.len(), deleted.len())?)?
                }
                Operation::Carry if !section.boundaries.is_empty() => stamp_bytes(),
                _ => 0,
            };
            required = allocation::add(required, bytes)?;
        }
        let mut credit = super::decoder::change_credit(budget, required, Some(cancelled))?;
        let mut changes = std::array::from_fn(|_| None);
        let mut stamps = std::array::from_fn(|_| None);
        for section in &self.sections {
            let index = section.table.index();
            let before = previous.map(|frame| &frame.tables[index]);
            let reset = section
                .boundaries
                .iter()
                .any(|boundary| !matches!(boundary.kind, BoundaryKind::Delta));
            match &section.operation {
                Operation::Complete(_) => {
                    stamps[index] = Some(field_stamp(
                        None,
                        None,
                        self.to.hour,
                        credit.split_off(stamp_bytes()),
                    )?);
                }
                Operation::Carry if !section.boundaries.is_empty() => {
                    stamps[index] = Some(field_stamp(
                        None,
                        None,
                        self.to.hour,
                        credit.split_off(stamp_bytes()),
                    )?);
                }
                Operation::Patch { rows, deleted } => {
                    let before = before.ok_or_else(|| invalid("endpoint patch has no baseline"))?;
                    let facts_credit = credit.split_off(change_bytes(rows.len(), deleted.len())?);
                    let mut facts = ChangeFacts {
                        previous: before.identity(),
                        changed: allocation::vec(rows.len())?,
                        deleted: allocation::copy_vec(deleted)?,
                        fields: 0,
                        membership: !deleted.is_empty()
                            || section.boundaries.iter().any(|boundary| {
                                !boundary.inserted.is_empty() || !boundary.deleted.is_empty()
                            }),
                        _credit: facts_credit,
                    };
                    for (key, fields, inserted) in rows.edits() {
                        facts.fields |= fields;
                        facts.membership |= inserted;
                        facts.changed.push(RowChange {
                            key,
                            fields,
                            inserted,
                        });
                    }
                    stamps[index] = Some(field_stamp(
                        (!reset).then_some(before.3.as_deref()).flatten(),
                        Some(&facts),
                        self.to.hour,
                        credit.split_off(stamp_bytes()),
                    )?);
                    if !reset {
                        changes[index] = Some(Arc::new(facts));
                    }
                }
                _ => {}
            }
        }
        Ok((changes, stamps))
    }

    pub fn apply(self, previous: Option<&DecodedFrame>) -> io::Result<DecodedFrame> {
        if self.from != previous.map(Point::of) {
            return Err(invalid("endpoint baseline differs"));
        }
        let mut tables = std::array::from_fn(|_| not_selected());
        for section in self.sections {
            let table = section.table;
            let view: SharedView = match section.operation {
                Operation::Carry => previous
                    .ok_or_else(|| invalid("endpoint carry has no baseline"))?
                    .tables[table.index()]
                .clone(),
                Operation::Complete(rows) => TableView::Complete(rows).into(),
                Operation::Truncated { rows, missing } => {
                    TableView::Truncated { rows, missing }.into()
                }
                Operation::Unavailable(reason) => TableView::Unavailable { reason }.into(),
                Operation::NotRecorded(mode) => TableView::NotRecorded { mode }.into(),
                Operation::Patch { rows, deleted } => {
                    let before =
                        previous.ok_or_else(|| invalid("endpoint patch has no baseline"))?;
                    let TableView::Complete(before) = &*before.tables[table.index()] else {
                        return Err(invalid("endpoint patch needs complete baseline"));
                    };
                    let result = before
                        .try_clone_validated()?
                        .apply_endpoint_rows(rows, &deleted)?;
                    validate_complete(table, &result, self.to.hour)?;
                    TableView::Complete(result).into()
                }
            };
            if section
                .sample
                .is_some_and(|hour| hour > self.to.hour || !matches!(*view, TableView::Complete(_)))
            {
                return Err(invalid("invalid endpoint sample hour"));
            }
            // Carrying a Clock into another hour is not a complete hour.
            if let TableView::Complete(rows) = &*view {
                validate_singleton(table, rows, self.to.hour)?;
            }
            tables[table.index()] = view.with_sampled_at(section.sample);
        }
        Ok(DecodedFrame {
            branch: self.to.branch,
            hour: self.to.hour,
            tables,
            metrics: self.metrics,
        })
    }

    pub fn apply_owned(self, previous: Option<DecodedFrame>) -> io::Result<DecodedFrame> {
        self.apply_owned_parallel(previous, 1, &|| {})
    }

    pub fn apply_owned_parallel(
        self,
        previous: Option<DecodedFrame>,
        workers: usize,
        on_worker: &(dyn Fn() + Sync),
    ) -> io::Result<DecodedFrame> {
        if self.from != previous.as_ref().map(Point::of) {
            return Err(invalid("endpoint baseline differs"));
        }
        let held = match previous {
            Some(previous) => previous.tables.map(Some),
            None => std::array::from_fn(|_| None),
        };
        self.apply_tables_parallel(held, workers, on_worker)
    }

    pub(super) fn apply_shared_parallel(
        self,
        previous: Option<Arc<DecodedFrame>>,
        workers: usize,
        on_worker: &(dyn Fn() + Sync),
    ) -> io::Result<DecodedFrame> {
        if self.from != previous.as_deref().map(Point::of) {
            return Err(invalid("endpoint baseline differs"));
        }
        let held = match previous {
            Some(previous) => super::split_shared_tables(previous).1.map(Some),
            None => std::array::from_fn(|_| None),
        };
        self.apply_tables_parallel(held, workers, on_worker)
    }

    fn apply_tables_parallel(
        self,
        mut held: [Option<SharedView>; TABLE_COUNT],
        workers: usize,
        on_worker: &(dyn Fn() + Sync),
    ) -> io::Result<DecodedFrame> {
        let hour = self.to.hour;
        let mut tables = std::array::from_fn(|_| not_selected());
        let threads = workers.clamp(1, self.sections.len().max(1));
        if threads == 1 {
            for section in self.sections {
                let view = held[section.table.index()].take();
                let (table, view) = apply_section(section, view, hour)?;
                tables[table.index()] = view;
            }
        } else {
            let mut jobs = allocation::vec(self.sections.len())?;
            for section in self.sections {
                let view = held[section.table.index()].take();
                jobs.push((section, view));
            }
            jobs.sort_unstable_by_key(|(section, view)| std::cmp::Reverse(weight(section, view)));
            let mut failed: Option<(usize, io::Error)> = None;
            for (table, view) in apply_jobs(jobs, threads, on_worker, hour)? {
                let index = table.index();
                match view {
                    Ok(view) => tables[index] = view,
                    Err(error) if failed.as_ref().is_none_or(|(first, _)| index < *first) => {
                        failed = Some((index, error));
                    }
                    Err(_) => {}
                }
            }
            if let Some((_, error)) = failed {
                return Err(error);
            }
        }
        Ok(DecodedFrame {
            branch: self.to.branch,
            hour,
            tables,
            metrics: self.metrics,
        })
    }
}

fn apply_section(
    section: Section,
    held: Option<SharedView>,
    hour: Hour,
) -> io::Result<(TableId, SharedView)> {
    let table = section.table;
    let view: SharedView = match section.operation {
        Operation::Carry => held.ok_or_else(|| invalid("endpoint carry has no baseline"))?,
        Operation::Complete(rows) => TableView::Complete(rows).into(),
        Operation::Truncated { rows, missing } => TableView::Truncated { rows, missing }.into(),
        Operation::Unavailable(reason) => TableView::Unavailable { reason }.into(),
        Operation::NotRecorded(mode) => TableView::NotRecorded { mode }.into(),
        Operation::Patch { rows, deleted } => {
            let before = held.ok_or_else(|| invalid("endpoint patch has no baseline"))?;
            let before = match before.try_unwrap() {
                Ok(TableView::Complete(before)) => before,
                Err(shared) => match &*shared {
                    TableView::Complete(before) => before.try_clone_validated()?,
                    _ => return Err(invalid("endpoint patch needs complete baseline")),
                },
                Ok(_) => return Err(invalid("endpoint patch needs complete baseline")),
            };
            let membership = rows.inserts() || !deleted.is_empty();
            let result = before.apply_endpoint_rows(rows, &deleted)?;
            if membership {
                validate_complete(table, &result, hour)?;
            } else {
                validate_applied(table, &result, hour)?;
            }
            TableView::Complete(result).into()
        }
    };
    if section
        .sample
        .is_some_and(|sample| sample > hour || !matches!(*view, TableView::Complete(_)))
    {
        return Err(invalid("invalid endpoint sample hour"));
    }
    // Carrying a Clock into another hour is not a complete hour.
    if let TableView::Complete(rows) = &*view {
        validate_singleton(table, rows, hour)?;
    }
    Ok((table, view.with_sampled_at(section.sample)))
}

fn weight(section: &Section, held: &Option<SharedView>) -> usize {
    match &section.operation {
        Operation::Carry | Operation::Unavailable(_) | Operation::NotRecorded(_) => 0,
        Operation::Complete(rows) | Operation::Truncated { rows, .. } => rows.len(),
        Operation::Patch { rows, .. } => {
            rows.len()
                + held.as_deref().map_or(0, |view| match view {
                    TableView::Complete(before) => before.len(),
                    _ => 0,
                })
        }
    }
}

fn apply_jobs(
    jobs: Vec<(Section, Option<SharedView>)>,
    threads: usize,
    on_worker: &(dyn Fn() + Sync),
    hour: Hour,
) -> io::Result<Vec<(TableId, io::Result<SharedView>)>> {
    let count = jobs.len();
    let queue = std::sync::Mutex::new(jobs.into_iter());
    let work = || -> io::Result<Vec<(TableId, io::Result<SharedView>)>> {
        let mut done = allocation::vec(count)?;
        loop {
            let next = queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next();
            let Some((section, view)) = next else {
                return Ok(done);
            };
            let table = section.table;
            let result = apply_section(section, view, hour).map(|(_, view)| view);
            done.push((table, result));
        }
    };
    std::thread::scope(|scope| {
        let mut others = allocation::vec(threads - 1)?;
        for _ in 1..threads {
            let worker = std::thread::Builder::new()
                .name("endpoint table worker".into())
                .spawn_scoped(scope, || {
                    on_worker();
                    work()
                });
            // A worker the system cannot start leaves its share to the others.
            match worker {
                Ok(worker) => others.push(worker),
                Err(_) => break,
            }
        }
        let mut all = work()?;
        for other in others {
            let done = other
                .join()
                .map_err(|_| io::Error::other("an endpoint table worker panicked"))??;
            allocation::reserve(&mut all, done.len())?;
            all.extend(done);
        }
        Ok(all)
    })
}

/// Physical typed bytes, including the independently bounded history records.
#[derive(Clone, Debug)]
pub struct EndpointStats {
    pub bytes: usize,
    pub table_bytes: [usize; TABLE_COUNT],
    pub edited_rows: usize,
    pub deleted_rows: usize,
    pub boundary_bytes: usize,
}
impl Default for EndpointStats {
    fn default() -> Self {
        Self {
            bytes: 0,
            table_bytes: [0; TABLE_COUNT],
            edited_rows: 0,
            deleted_rows: 0,
            boundary_bytes: 0,
        }
    }
}
fn put<T: Value>(value: &T, out: &mut Vec<u8>) -> io::Result<()> {
    Value::encode(value, out)
}
fn put_count(count: usize, out: &mut Vec<u8>) -> io::Result<()> {
    put(
        &u32::try_from(count).map_err(|_| invalid("endpoint count exceeds u32"))?,
        out,
    )
}
fn write_keys(keys: &[u128], out: &mut Vec<u8>) -> io::Result<()> {
    put_count(keys.len(), out)?;
    allocation::reserve(out, allocation::mul(keys.len(), 16)?)?;
    for key in keys {
        out.extend_from_slice(&key.to_le_bytes());
    }
    Ok(())
}
fn read_keys(table: TableId, input: &mut Decoder<'_>) -> io::Result<Vec<u128>> {
    let count = input.count(16)?;
    let mut keys = allocation::vec(count)?;
    for _ in 0..count {
        let key = u128::from_le_bytes(input.take(16)?.try_into().unwrap());
        table.check_key(key)?;
        if keys.last().is_some_and(|&old| key <= old) {
            return Err(invalid("endpoint keys must be unique and ascending"));
        }
        keys.push(key);
    }
    Ok(keys)
}
fn read_boundaries(
    table: TableId,
    from: Option<Point>,
    to: Point,
    input: &mut Decoder<'_>,
) -> io::Result<Vec<MembershipBoundary>> {
    let count = input.count(13)?;
    let mut boundaries = allocation::vec(count)?;
    let mut previous = from.map(|from| from.hour);
    for _ in 0..count {
        let hour = input.i32()?;
        if hour > to.hour || previous.is_some_and(|old| hour <= old) {
            return Err(invalid("endpoint boundary hour is out of order"));
        }
        previous = Some(hour);
        let kind = match input.u8()? {
            0 => BoundaryKind::CompleteReset,
            1 => BoundaryKind::Delta,
            2 => BoundaryKind::Partial,
            3 => BoundaryKind::Unavailable,
            _ => return Err(invalid("invalid endpoint boundary kind")),
        };
        let inserted = read_keys(table, input)?;
        let deleted = read_keys(table, input)?;
        if kind != BoundaryKind::Delta && !deleted.is_empty()
            || kind == BoundaryKind::Unavailable && !inserted.is_empty()
            || inserted
                .iter()
                .any(|key| deleted.binary_search(key).is_ok())
        {
            return Err(invalid("invalid endpoint membership boundary"));
        }
        boundaries.push(MembershipBoundary {
            table,
            hour,
            kind,
            inserted,
            deleted,
        });
    }
    Ok(boundaries)
}
fn write_boundaries(boundaries: &[MembershipBoundary], out: &mut Vec<u8>) -> io::Result<()> {
    put_count(boundaries.len(), out)?;
    for boundary in boundaries {
        put(&boundary.hour, out)?;
        put(&(boundary.kind as u8), out)?;
        write_keys(&boundary.inserted, out)?;
        write_keys(&boundary.deleted, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
