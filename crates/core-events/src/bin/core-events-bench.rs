use core_events::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs,
    hint::black_box,
    io,
    mem::ManuallyDrop,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};

struct MeasuredAllocator;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

const CHANGES: usize = 10_970;
const TABLES: usize = 120;
const COUNTRIES: usize = 1_500;
const COLUMNS: usize = 4;
const CAMPAIGN_HOURS: usize = 13 * 365 * 24;
const PERIOD: usize = 168;
const MAX_CHANGES: usize = CHANGES + CHANGES / 4;
const PROVINCES: usize = 30_000;
const REREAD_CHUNK_ROWS: usize = 1_024;
const RETAINED_CELLS: usize = (PROVINCES + (TABLES - 1) * COUNTRIES) * COLUMNS;
const MAX_EVENTS: usize = MAX_CHANGES * 2 + TABLES * 4;

struct Allowance {
    remaining: usize,
}
impl Allowance {
    fn charge(&mut self, bytes: usize) -> io::Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or(io::ErrorKind::OutOfMemory)?;
        Ok(())
    }
    fn vector<T>(&mut self, count: usize) -> io::Result<Vec<T>> {
        self.charge(
            count
                .checked_mul(size_of::<T>())
                .ok_or(io::ErrorKind::OutOfMemory)?,
        )?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| io::ErrorKind::OutOfMemory)?;
        Ok(values)
    }
    fn share<T>(&mut self, values: Vec<T>) -> io::Result<Arc<[T]>> {
        self.charge(
            values
                .len()
                .checked_mul(size_of::<T>())
                .and_then(|n| n.checked_add(2 * size_of::<usize>()))
                .ok_or(io::ErrorKind::OutOfMemory)?,
        )?;
        Ok(Arc::from(values))
    }
}

#[cfg(windows)]
fn thread_cycles() -> io::Result<u64> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn QueryThreadCycleTime(thread: *mut std::ffi::c_void, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0;
    if unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut cycles) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(cycles)
}
#[cfg(not(windows))]
fn thread_cycles() -> io::Result<u64> {
    Err(io::ErrorKind::Unsupported.into())
}

#[derive(Default)]
struct Measurement {
    cycles: u64,
    ns: u128,
}
impl Measurement {
    fn add(&mut self, start: (u64, Instant)) -> io::Result<()> {
        self.cycles += thread_cycles()? - start.0;
        self.ns += start.1.elapsed().as_nanos();
        Ok(())
    }
}
fn start() -> io::Result<(u64, Instant)> {
    Ok((thread_cycles()?, Instant::now()))
}

struct NoopFamily {
    subscription: Subscription,
}
impl Family for NoopFamily {
    fn subscription(&self) -> Subscription {
        self.subscription.clone()
    }
    fn begin(
        &mut self,
        _: &Start<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        Ok(())
    }
    fn hour(
        &mut self,
        at: &At<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        for event in at.events {
            black_box(event);
        }
        Ok(())
    }
    fn end(
        &mut self,
        _: &End<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<Boundary<'_>> {
        Ok(Boundary::ClassA)
    }
}
struct NoopPublication;
impl Publication for NoopPublication {
    fn publish(&mut self, _: PublicationRecord<'_>) -> io::Result<()> {
        Ok(())
    }
    fn coverage(&mut self, _: Coverage<'_>) -> io::Result<()> {
        Ok(())
    }
    fn provisional(&mut self, _: ProvisionalWindow<'_>) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
struct UnusedContext(Hour);
impl CallContext for UnusedContext {
    fn observation(&self, _: DeclarationId) -> io::Result<&dyn ObservationStream> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn close_window(&mut self, _: DeclarationId, _: Key<'_>) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn hour(&self) -> Hour {
        self.0
    }
    fn exact(&self, _: TableId, _: Key<'_>, _: FieldId) -> io::Result<CellValue<'_>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn previous(&self, _: TableId, _: Key<'_>, _: FieldId) -> io::Result<CellValue<'_>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn historical(
        &self,
        _: IndexId,
        _: Key<'_>,
        _: ProductFieldId,
        _: Hour,
    ) -> io::Result<ProductCell<'_>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn prefix(
        &self,
        _: DeclarationId,
        _: &[Key<'_>],
        _: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn secondary(
        &self,
        _: DeclarationId,
        _: Key<'_>,
        _: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn members(
        &self,
        _: RelationId,
        _: Key<'_>,
        _: &mut dyn FnMut(Key<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn snapshot(&self, _: TableId) -> io::Result<Lookup<'_, &dyn Rows>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn take_watch(&mut self, _: DeclarationId) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn schedule(&mut self, _: Timer<'_>) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn cancel(&mut self, _: Key<'_>) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn insert_filter(&mut self, _: DeclarationId, _: Key<'_>) -> io::Result<FilterInsertion<'_>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_filter(&mut self, _: DeclarationId, _: Key<'_>) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn dictionary(&self, _: u32) -> io::Result<Lookup<'_, &str>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn dictionary_additions(&self) -> &[DictionaryString<'_>] {
        &[]
    }
}

struct Payload {
    u32s: [u32; 2],
    u64s: [u64; 2],
    bytes: [u8; 8],
}
fn sample(kind: replay_data::rows::Kind, seed: u64, payload: &Payload) -> Value<'_> {
    use replay_data::rows::Kind;
    match kind {
        Kind::U8 => Value::U8(seed as u8),
        Kind::I8 => Value::I8(seed as i8),
        Kind::U16 => Value::U16(seed as u16),
        Kind::I16 => Value::I16(seed as i16),
        Kind::U32 => Value::U32(seed as u32),
        Kind::I32 => Value::I32(seed as i32),
        Kind::U64 => Value::U64(seed),
        Kind::I64 => Value::I64(seed as i64),
        Kind::Bool => Value::Bool(seed & 1 != 0),
        Kind::OptionalI32 => Value::OptionalI32(Some(seed as i32)),
        Kind::OptionalU32 => Value::OptionalU32(Some(seed as u32)),
        Kind::OptionalI64 => Value::OptionalI64(Some(seed as i64)),
        Kind::OptionalU64 => Value::OptionalU64(Some(seed)),
        Kind::Bytes => Value::Bytes(seed as u32),
        Kind::U32List => Value::U32List(&payload.u32s),
        Kind::U64List => Value::U64List(&payload.u64s),
        Kind::Opaque => Value::Opaque(&payload.bytes),
    }
}
struct ChangeValues<'a> {
    previous: CellValue<'a>,
    current: Value<'a>,
}
struct TableValues<'a> {
    table: TableId,
    fields: Arc<[FieldId]>,
    cells: Vec<CellValue<'a>>,
    rows: usize,
}
impl RowValues for TableValues<'_> {
    fn value(&self, _: Hour, slot: usize, field: FieldId) -> io::Result<CellValue<'_>> {
        self.cells
            .get(slot * self.fields.len() + field)
            .copied()
            .ok_or_else(|| io::ErrorKind::InvalidInput.into())
    }
}

fn changes(hour: usize) -> usize {
    let offset = (hour % 8) as isize * 2 - 7;
    (CHANGES as isize + offset * (CHANGES / 28) as isize) as usize
}
fn count_for(total: usize, table: usize) -> usize {
    total / TABLES + usize::from(table < total % TABLES)
}
fn build_values<'a>(
    values: &mut Vec<ChangeValues<'a>>,
    tables: &[TableValues<'_>],
    payloads: &'a [Payload],
    hour: usize,
) {
    values.clear();
    for (index, table) in tables.iter().enumerate() {
        for change in 0..count_for(changes(hour), index) {
            let ordinal = values.len();
            let kind = table.table.schema()[change % table.fields.len()];
            let seed = hour as u64 * MAX_CHANGES as u64 * 2 + ordinal as u64 * 2;
            values.push(ChangeValues {
                previous: CellValue::Known(sample(kind, seed, &payloads[ordinal * 2])),
                current: sample(kind, seed + 1, &payloads[ordinal * 2 + 1]),
            });
        }
    }
}
fn key(keys: &[[u8; 3]], slot: usize) -> Key<'_> {
    let bytes = &keys[slot % keys.len()];
    let len = bytes.iter().position(|byte| byte & 128 == 0).unwrap() + 1;
    Key::new(&bytes[..len]).unwrap()
}

fn build<'a>(
    events: &mut Vec<Event<'a>>,
    keys: &'a [[u8; 3]],
    tables: &'a [TableValues<'a>],
    values: &'a [ChangeValues<'a>],
    missing: &'a Missing<'a>,
    due: &'a [Timer<'a>],
    hour: usize,
) {
    let mut offset = 0;
    let evidence = ReadEvidence {
        manifest_bits: &[1],
        timing: SpanView::default(),
    };
    if hour % PERIOD == 0 {
        events.push(Event::Discontinuity(Discontinuity::SegmentStart {
            hours: Interval::hour(hour as Hour),
        }));
    }
    for (index, table) in tables.iter().enumerate() {
        let id = table.table;
        let count = count_for(values.len(), index);
        let reread = hour % PERIOD == 0 && index == (hour / PERIOD) % TABLES;
        if (hour + index) % 24 == 0 || reread {
            events.push(Event::Status {
                table: id,
                status: &Status::Complete,
                sample_hour: Some(hour as Hour),
            });
            events.push(Event::Acquired {
                table: id,
                evidence,
            });
            events.push(Event::Supplied {
                table: id,
                evidence,
            });
        }
        if reread {
            events.push(Event::Reread { table: id });
        }
        for row in 0..count / 100 + 1 {
            let slot = COUNTRIES - row - 1;
            events.push(Event::Gone {
                table: id,
                key: key(keys, slot),
                slot,
            });
            events.push(Event::Appeared {
                table: id,
                key: key(keys, slot),
                slot,
                fields: RowView::new(hour as Hour, slot, &table.fields, table),
                source: Appearance::Ordinary,
            });
        }
        for change in 0..count {
            let slot = (change * 43 + hour) % (COUNTRIES - 20);
            let field = change % table.fields.len();
            let cells = &values[offset + change];
            events.push(Event::Changed {
                table: id,
                key: key(keys, slot),
                field,
                slot,
                previous: &cells.previous,
                value: &cells.current,
            });
            if change % 50 == 0 {
                events.push(Event::Left {
                    cause: MembershipCause::Changed,
                    relation: RelationId(index),
                    value: key(keys, slot + 1),
                    member: key(keys, slot),
                });
                events.push(Event::Joined {
                    cause: MembershipCause::Changed,
                    relation: RelationId(index),
                    value: key(keys, slot + 2),
                    member: key(keys, slot),
                });
            }
        }
        for row in 0..count / 50 + 1 {
            let slot = COUNTRIES - row - 10;
            events.push(Event::Unknown {
                table: id,
                key: key(keys, slot),
                field: row % table.fields.len(),
                slot,
                missing,
            });
        }
        offset += count;
    }
    if hour % 24 == 0 {
        events.push(Event::Observed {
            observation: ObservationId(0),
        });
    }
    events.push(Event::Tick {
        hour: hour as Hour,
        starts_day: hour % 24 == 0,
        due,
    });
}

fn build_reread<'a>(
    events: &mut Vec<Event<'a>>,
    keys: &'a [[u8; 3]],
    table: &'a TableValues<'a>,
    hour: usize,
    first: usize,
    last: usize,
) {
    for slot in first..last {
        events.push(Event::Appeared {
            table: table.table,
            key: key(keys, slot),
            slot,
            fields: RowView::new(hour as Hour, slot, &table.fields, table),
            source: Appearance::Reread,
        });
    }
}

fn recycle<'a, 'b>(mut events: Vec<Event<'a>>) -> Vec<Event<'b>> {
    events.clear();
    let mut events = ManuallyDrop::new(events);
    unsafe { Vec::from_raw_parts(events.as_mut_ptr().cast(), 0, events.capacity()) }
}
fn dispatch(
    events: &[Event<'_>],
    hour: usize,
    family: &mut dyn Family,
    out: &mut dyn Publication,
) -> io::Result<()> {
    family.hour(
        &At {
            hour: hour as Hour,
            since: QuietView::default(),
            reads: &[],
            observations: &[],
            starts: hour % PERIOD == 0,
            opens: hour == 0,
            events,
        },
        &mut UnusedContext(hour as Hour),
        out,
    )
}
fn kind(event: &Event<'_>) -> usize {
    match event {
        Event::Appeared {
            source: Appearance::Ordinary,
            ..
        } => 0,
        Event::Appeared {
            source: Appearance::Reread,
            ..
        } => 1,
        Event::Gone { .. } => 2,
        Event::Changed { .. } => 3,
        Event::Unknown { .. } => 4,
        Event::Status { .. } => 5,
        Event::Reread { .. } => 6,
        Event::Acquired { .. } => 7,
        Event::Supplied { .. } => 8,
        Event::Observed { .. } => 9,
        Event::Discontinuity(_) => 10,
        Event::Joined { .. } => 11,
        Event::Left { .. } => 12,
        Event::Tick { .. } => 13,
    }
}

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(
        args.next()
            .unwrap_or_else(|| std::env::temp_dir().join("hoi4-core-events-bench").into_os_string()),
    );
    let hours = args
        .next()
        .map(|arg| arg.to_string_lossy().parse::<usize>())
        .transpose()
        .map_err(|_| io::ErrorKind::InvalidInput)?
        .unwrap_or(CAMPAIGN_HOURS);
    if hours == 0 || hours > Hour::MAX as usize || args.next().is_some() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut reader = Allowance {
        remaining: 900_000_000,
    };
    let mut transient = Allowance {
        remaining: 150_000_000,
    };
    let mut scheduler = Allowance {
        remaining: 100_000_000,
    };
    let mut keys = reader.vector::<[u8; 3]>(PROVINCES.max(COUNTRIES))?;
    for id in 0..PROVINCES.max(COUNTRIES) {
        keys.push([
            ((id & 127) as u8) | if id >= 128 { 128 } else { 0 },
            (((id >> 7) & 127) as u8) | if id >= 16384 { 128 } else { 0 },
            (id >> 14) as u8,
        ]);
    }
    let mut payloads = reader.vector::<Payload>(MAX_CHANGES * 2 + RETAINED_CELLS)?;
    for id in 0..MAX_CHANGES * 2 + RETAINED_CELLS {
        payloads.push(Payload {
            u32s: [id as u32, id as u32 + 1],
            u64s: [id as u64, id as u64 + 1],
            bytes: (id as u64).to_le_bytes(),
        });
    }
    let mut tables = reader.vector::<TableValues<'_>>(TABLES)?;
    let mut declarations = scheduler.vector::<Declaration>(TABLES)?;
    let mut retained_cell = 0;
    for table in std::iter::once(TableId::Provinces)
        .chain(
            TableId::ALL
                .iter()
                .copied()
                .filter(|table| *table != TableId::Provinces && !table.schema().is_empty()),
        )
        .take(TABLES)
    {
        let columns = table.schema().len().min(COLUMNS);
        let mut field_ids = scheduler.vector::<FieldId>(columns)?;
        field_ids.extend(0..columns);
        let fields = scheduler.share(field_ids)?;
        declarations.push(Declaration::Stream {
            table,
            fields: fields.clone(),
            filter: Filter::All,
        });
        let rows = if table == TableId::Provinces {
            PROVINCES
        } else {
            COUNTRIES
        };
        let mut cells = reader.vector::<CellValue<'_>>(rows * columns)?;
        for _ in 0..rows {
            for field in 0..columns {
                let seed = retained_cell;
                retained_cell += 1;
                cells.push(CellValue::Known(sample(
                    table.schema()[field],
                    seed as u64,
                    &payloads[MAX_CHANGES * 2 + seed],
                )));
            }
        }
        tables.push(TableValues {
            table,
            fields,
            cells,
            rows,
        });
    }
    let mut values = transient.vector::<ChangeValues<'_>>(MAX_CHANGES)?;
    let mut storage = transient.vector::<Event<'static>>(MAX_EVENTS)?;
    let mut row_storage = transient.vector::<Event<'static>>(REREAD_CHUNK_ROWS)?;
    let mut family = NoopFamily {
        subscription: Subscription::new(scheduler.share(declarations)?)
            .map_err(io::Error::other)?,
    };
    let mut out = NoopPublication;
    black_box(family.subscription());
    let mut building = Measurement::default();
    let mut ordering = Measurement::default();
    let mut dispatching = Measurement::default();
    let mut counts = [0u64; 14];
    let mut total = 0usize;
    let mut min_events = usize::MAX;
    let mut max_events = 0;
    let mut value_changes = 0usize;
    let mut allocations_before = 0;
    for iteration in 0..PERIOD + hours {
        let measured = iteration >= PERIOD;
        let hour = if measured {
            iteration - PERIOD
        } else {
            iteration
        };
        if iteration == PERIOD {
            allocations_before = ALLOCATIONS.load(Ordering::Relaxed);
            PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        let clock = start()?;
        build_values(black_box(&mut values), &tables, &payloads, hour);
        let missing = Missing {
            hours: Interval::hour(hour as Hour),
            kind: MissingKind::NotRecorded,
        };
        let due = [Timer {
            hour: hour as Hour,
            id: key(&keys, hour),
            key: key(&keys, hour),
        }];
        let mut events = recycle(storage);
        build(&mut events, &keys, &tables, &values, &missing, &due, hour);
        if measured {
            building.add(clock)?;
        }
        let clock = start()?;
        events.sort_unstable_by_key(Event::order);
        if measured {
            ordering.add(clock)?;
        }
        let reread = (hour % PERIOD == 0).then(|| &tables[(hour / PERIOD) % TABLES]);
        let rows = reread.map_or(0, |table| table.rows);
        let split = if reread.is_some() {
            events.partition_point(|event| {
                matches!(
                    event,
                    Event::Discontinuity(_)
                        | Event::Status { .. }
                        | Event::Acquired { .. }
                        | Event::Supplied { .. }
                        | Event::Observed { .. }
                        | Event::Reread { .. }
                )
            })
        } else {
            0
        };
        if split > 0 {
            let clock = start()?;
            dispatch(
                black_box(&events[..split]),
                hour,
                black_box(&mut family),
                &mut out,
            )?;
            if measured {
                dispatching.add(clock)?;
            }
        }
        if let Some(table) = reread {
            for first in (0..table.rows).step_by(REREAD_CHUNK_ROWS) {
                let clock = start()?;
                let mut chunk = recycle(row_storage);
                build_reread(
                    &mut chunk,
                    &keys,
                    table,
                    hour,
                    first,
                    (first + REREAD_CHUNK_ROWS).min(table.rows),
                );
                if measured {
                    building.add(clock)?;
                }
                let clock = start()?;
                chunk.sort_unstable_by_key(Event::order);
                if measured {
                    ordering.add(clock)?;
                }
                let clock = start()?;
                dispatch(black_box(&chunk), hour, black_box(&mut family), &mut out)?;
                if measured {
                    dispatching.add(clock)?;
                    counts[1] += chunk.len() as u64;
                }
                row_storage = recycle(chunk);
            }
        }
        let clock = start()?;
        dispatch(
            black_box(&events[split..]),
            hour,
            black_box(&mut family),
            &mut out,
        )?;
        if measured {
            dispatching.add(clock)?;
            total += events.len() + rows;
            min_events = min_events.min(events.len() + rows);
            max_events = max_events.max(events.len() + rows);
            value_changes += values.len();
            for event in &events {
                counts[kind(event)] += 1;
            }
        }
        storage = recycle(events);
    }
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations_before;
    let peak = PEAK.load(Ordering::Relaxed);
    let per_hour = total as f64 / hours as f64;
    let all_ns = building.ns + ordering.ns + dispatching.ns;
    let mut report = format!(
        "# core-events measurement\n\nSynthetic, no game data: {hours} hours; 13-year default {CAMPAIGN_HOURS}. {TABLES} tables, {COUNTRIES} country keys, up to {COLUMNS} schema columns. Mean Changed target {CHANGES}/hour; actual {:.3}. An eight-hour symmetric +/-25% pattern varies hour sizes; every {PERIOD} hours one Reread borrows a table's full rows, including a synthetic {PROVINCES}-row Provinces table. Reread rows stream in admitted {REREAD_CHUNK_ROWS}-row batches before ordinary events; the other tables hold {COUNTRIES} rows each. Ordinary inserts/deletes are about 1% each, Unknown about 2%, Joined/Left about 2% each, plus staggered daily read/status facts, observations, discontinuities and timer ticks. All kind counts below are measured. Input is deliberately unsorted table emission, with interleaved reverse changes and column/slot disorder. Each change writes distinct previous/current typed cells into reused storage, with separately owned distinct list/byte payloads. Reread rows borrow retained table cells.\n\nCPU uses QueryThreadCycleTime per phase. ns/event and us/hour use high-resolution elapsed time and include scheduling delays; cycles are not converted using an assumed clock frequency. Clock-call overhead is charged.\n\n| Kind | CPU cycles/event | elapsed ns/event | elapsed us/hour |\n|---|---:|---:|---:|\n",
        value_changes as f64 / hours as f64
    );
    use std::fmt::Write;
    for (name, phase) in [
        ("Build cells and events", &building),
        ("Order", &ordering),
        ("Dispatch", &dispatching),
    ] {
        writeln!(
            report,
            "| {name} | {:.3} | {:.3} | {:.3} |",
            phase.cycles as f64 / total as f64,
            phase.ns as f64 / total as f64,
            phase.ns as f64 / hours as f64 / 1000.
        )
        .unwrap();
    }
    writeln!(report, "\nReader budget (2.9): 150-400 us/hour, {:.3}-{:.3} ns/event at the measured mean {per_hour:.3} events/hour. Build/order/dispatch elapsed share: {:.3}% / {:.3}%; decode, reverse upkeep/fan-out, snapshots, hydration, real handlers and publication are not exercised. These figures do not project whole preparation or playback.\n\nWarmup: one {PERIOD}-hour shape cycle. Steady-state event counts vary {min_events}-{max_events}. Allocation calls: {allocations}; allocations/hour {:.6}, required 0.\n", 150_000. / per_hour, 400_000. / per_hour, all_ns as f64 / hours as f64 / 150_000. * 100., all_ns as f64 / hours as f64 / 400_000. * 100., allocations as f64 / hours as f64).unwrap();
    writeln!(report, "| Event kind | Count |\n|---|---:|").unwrap();
    for (name, count) in [
        "Appeared",
        "Reread Appeared",
        "Gone",
        "Changed",
        "Unknown",
        "Status",
        "Reread",
        "Acquired",
        "Supplied",
        "Observed",
        "Discontinuity",
        "Joined",
        "Left",
        "Tick",
    ]
    .into_iter()
    .zip(counts)
    {
        writeln!(report, "| {name} | {count} |").unwrap();
    }
    writeln!(report, "\n| Owner | Admitted backing bytes | Shared allowance (2.8) |\n|---|---:|---:|\n| Reader keys, payloads and retained row cells | {} | 900000000 |\n| Transient previous/current cells and event batch | {} | 150000000 |\n| Scheduler declarations and field lists | {} | 100000000 |\n| Allocator steady-state live-byte high-water | {peak} | whole preparation <3000000000 |\n\nFamily/publication heap, output bytes and disk/spool bytes: not exercised. Event size: {} bytes; base batch capacity: {} bytes; Reread chunk capacity: {} bytes; value-buffer capacity: {} bytes. The k-hour ring and readers in flight multiply these per-reader transient buffers against the shared 150000000-byte allowance. Every backing allocation is admitted before reserve; capacities are bounds of this synthetic workload, not game limits.\n\nSection 3 playback is not exercised; its allowance is 400000000 bytes and target 1440 game hours/wall second at 95 Hz.", 900_000_000 - reader.remaining, 150_000_000 - transient.remaining, 100_000_000 - scheduler.remaining, size_of::<Event<'_>>(), storage.capacity() * size_of::<Event<'_>>(), row_storage.capacity() * size_of::<Event<'_>>(), values.capacity() * size_of::<ChangeValues<'_>>()).unwrap();
    writeln!(report, "\nWindow retention is not exercised: 0 prefix, 0 per-key UntilClosed, 0 whole-segment windows. Readers must report WindowExtent::end = Segment (including never-closed keys and prefixes covering a short segment) with their retained bytes and hours; these costs are not measured here.").unwrap();
    fs::create_dir_all(&output)?;
    fs::write(output.join("report.md"), report)?;
    println!("{}", output.join("report.md").display());
    if allocations != 0 {
        return Err(io::Error::other(
            "steady-state allocation requirement failed",
        ));
    }
    Ok(())
}
