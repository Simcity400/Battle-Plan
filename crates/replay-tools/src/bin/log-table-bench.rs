use replay_recording::schema::schema;
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableState},
    unit::{
        BLOCK, BlockPage, Encoder, PartSink, Settings,
        reader::{self, Advanced, Cursor, Extents, LogCursor, LogEvent, Lives},
    },
};
use replay_data::rows::TableId;
use std::{
    collections::VecDeque,
    io,
    sync::Mutex,
    time::{Duration, Instant},
};

const TABLES: [TableId; 3] = [TableId::CombatLogBattleEquipment, TableId::CombatLogEnemyEquipment, TableId::CombatLogEquipment];
const SHARES: [u64; 3] = [900, 70, 30];
const TEXT: u32 = 1 << 30;

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

struct Counted<'a> {
    bytes: &'a [u8],
    read: Mutex<(u64, u64)>,
}

impl Extents for Counted<'_> {
    fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()> {
        let mut held = self.read.lock().map_err(|_| io::Error::other("poisoned"))?;
        held.0 += 1;
        held.1 += length as u64;
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + length]);
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Log,
    LogNoForgets,
    State,
}

struct Row {
    numbers: Vec<Option<Cell<'static>>>,
    lists: Vec<Vec<u32>>,
    longs: Vec<Vec<u64>>,
    opaque: Vec<Vec<u8>>,
}

fn mix(seed: u64) -> u64 {
    let mut value = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value ^= value >> 29;
    value.wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 17
}

fn row(kinds: &[Kind], ordinal: u64) -> Row {
    let mut row = Row { numbers: Vec::new(), lists: Vec::new(), longs: Vec::new(), opaque: Vec::new() };
    for (column, &kind) in kinds.iter().enumerate() {
        let seed = mix(ordinal / 3 * 131 + column as u64);
        let small = seed % 400;
        let cell = match kind {
            Kind::U8 => Some(Cell::U8(small as u8)),
            Kind::I8 => Some(Cell::I8((small % 100) as i8)),
            Kind::U16 => Some(Cell::U16(small as u16)),
            Kind::I16 => Some(Cell::I16(small as i16 - 200)),
            Kind::U32 => Some(Cell::U32(if column % 2 == 0 { (ordinal / 40) as u32 } else { small as u32 * 7 })),
            Kind::I32 => Some(Cell::I32(small as i32 * 13 - 900)),
            Kind::U64 => Some(Cell::U64(ordinal / 40 * 1000 + small)),
            Kind::I64 => Some(Cell::I64(small as i64 * 1000 - 77)),
            Kind::Bool => Some(Cell::Bool(seed % 3 == 0)),
            Kind::OptionalI32 => Some(Cell::OptionalI32((seed % 5 != 0).then_some(small as i32))),
            Kind::OptionalU32 => Some(Cell::OptionalU32((seed % 5 != 0).then_some(small as u32))),
            Kind::OptionalI64 => Some(Cell::OptionalI64((seed % 5 != 0).then_some(small as i64))),
            Kind::OptionalU64 => Some(Cell::OptionalU64((seed % 5 != 0).then_some(small))),
            Kind::Bytes => Some(Cell::Bytes(TEXT + (seed % 16) as u32)),
            Kind::U32List => {
                row.lists.push((0..seed % 4).map(|at| (small + at * 3) as u32).collect());
                None
            }
            Kind::U64List => {
                row.longs.push((0..seed % 4).map(|at| small + at * 3).collect());
                None
            }
            Kind::Opaque => {
                row.opaque.push((0..seed % 12).map(|at| (small + at) as u8).collect());
                None
            }
        };
        row.numbers.push(cell);
    }
    row
}

fn cells<'a>(kinds: &[Kind], row: &'a Row) -> Vec<Option<Cell<'a>>> {
    let (mut lists, mut longs, mut opaque) = (row.lists.iter(), row.longs.iter(), row.opaque.iter());
    kinds
        .iter()
        .zip(&row.numbers)
        .map(|(&kind, cell)| match kind {
            Kind::U32List => Some(Cell::U32List(lists.next().expect("a list"))),
            Kind::U64List => Some(Cell::U64List(longs.next().expect("a list"))),
            Kind::Opaque => Some(Cell::Opaque(opaque.next().expect("bytes"))),
            _ => *cell,
        })
        .collect()
}

struct Outcome {
    mode: Mode,
    memory: Memory,
    rows: u64,
    forgets: u64,
    apply: Duration,
    end_hours: Duration,
    store_peak: usize,
    tables_peak: usize,
    stored: u64,
    lifetimes: u64,
    restores: u64,
    blocks: u64,
    first_hour: i32,
}

fn run(mode: Mode, kinds: &[Vec<Kind>], rows: u64, hours: u32, cap: u64, level: i32) -> io::Result<Outcome> {
    let tables = kinds.len();
    let mut store = Store::new(kinds.to_vec(), vec![false; tables]);
    if mode != Mode::State {
        store.declare_logs(&vec![true; tables])?;
    }
    store.attach(Encoder::new(kinds, Settings::at(level))?);
    let mut memory = Memory::default();
    let first_hour = 100_000;
    store.begin_run(first_hour);
    for id in 0..16 {
        store.pin_string(TEXT + id, format!("equipment {id}").as_bytes())?;
    }
    let per_hour: Vec<u64> = SHARES.iter().map(|share| rows * share / 1000 / u64::from(hours)).collect();
    let caps: Vec<u64> = SHARES.iter().map(|share| cap * share / 1000).collect();
    let mut listed: Vec<VecDeque<u64>> = vec![VecDeque::new(); tables];
    let mut ordinals = vec![0_u64; tables];
    let (mut apply, mut end_hours) = (Duration::ZERO, Duration::ZERO);
    let (mut store_peak, mut tables_peak, mut total, mut forgets) = (0, 0, 0_u64, 0_u64);
    for at in 0..hours {
        let hour = first_hour + at as i32;
        store.begin_hour(hour, &[])?;
        for table in 0..tables {
            store.status(table, Status::Complete, true, Some(hour), true)?;
        }
        let started = Instant::now();
        for table in 0..tables {
            let mut changes = store.table(table)?;
            for _ in 0..per_hour[table] {
                let ordinal = ordinals[table];
                ordinals[table] += 1;
                let held = row(&kinds[table], ordinal);
                let given = cells(&kinds[table], &held);
                let key = u128::from(ordinal) << 16 | table as u128;
                let place = match mode {
                    Mode::State => {
                        let slot = changes.insert_given(key)?;
                        for (column, cell) in given.iter().enumerate() {
                            changes.write(slot, column, cell.expect("every cell given"))?;
                        }
                        u64::from(slot)
                    }
                    _ => changes.append(key, &given)?,
                };
                listed[table].push_back(place);
                total += 1;
                if listed[table].len() as u64 > caps[table] && mode != Mode::LogNoForgets {
                    let oldest = listed[table].pop_front().expect("listed");
                    forgets += 1;
                    match mode {
                        Mode::State => changes.delete(u32::try_from(oldest).map_err(io::Error::other)?)?,
                        _ => changes.forget(oldest)?,
                    }
                }
            }
        }
        apply += started.elapsed();
        let started = Instant::now();
        store.end_hour(&mut memory)?;
        end_hours += started.elapsed();
        if at == 0 {
            store.shed();
        }
        if (at + 1) % 720 == 0 {
            let (bytes, by_table) = store.bytes();
            store_peak = store_peak.max(bytes.total());
            tables_peak = tables_peak.max(by_table.iter().map(|held| held.total()).sum());
            store.end_block();
        }
    }
    store.end_run(&mut memory)?;
    let (bytes, by_table) = store.bytes();
    store_peak = store_peak.max(bytes.total());
    tables_peak = tables_peak.max(by_table.iter().map(|held| held.total()).sum());
    let written = store.encoder().expect("an encoder").written;
    Ok(Outcome {
        mode,
        rows: total,
        forgets,
        apply,
        end_hours,
        store_peak,
        tables_peak,
        stored: written.stored(),
        lifetimes: written.lifetimes.0,
        restores: written.restores.0,
        blocks: memory.pages.len() as u64,
        first_hour,
        memory,
    })
}

fn read(outcome: &Outcome, kinds: &[Vec<Kind>]) -> io::Result<serde_json::Value> {
    let memory = &outcome.memory;
    let pages = &memory.pages;
    let last = pages.len() - 1;
    let counted = Counted { bytes: &memory.bytes, read: Mutex::new((0, 0)) };
    let table = 0;
    let all: Vec<usize> = (0..kinds[table].len()).collect();
    let span_block = 1;
    let span_first = outcome.first_hour + 720 * span_block;
    if outcome.mode == Mode::State {
        let started = Instant::now();
        let (mut cells, _) = reader::rebuild(&counted, pages, &kinds[table], table, span_block as usize - 1, outcome.first_hour)?;
        let state_ms = started.elapsed().as_secs_f64() * 1e3;
        *counted.read.lock().map_err(|_| io::Error::other("poisoned"))? = (0, 0);
        let started = Instant::now();
        let mut span_rows = 0_u64;
        {
            let page = &pages[span_block as usize];
            let entry = &page.tables[table];
            let mut cursor = Cursor::new(&kinds[table], &all, 0);
            let mut written = vec![Vec::new(); all.len()];
            let mut lives = Lives::default();
            let mut next = 0;
            loop {
                match cursor.advance(page.hours - 1, 1, &mut cells, &mut written, &mut lives)? {
                    Advanced::Reached => break,
                    Advanced::Missing { .. } => {
                        let Some(part) = entry.parts.get(next) else { break };
                        let end = entry.parts.get(next + 1).map_or(page.hours - 1, |following| following.first - 1);
                        cursor.push(reader::fetch(&counted, part, end, &all, cursor.prefix().as_ref())?);
                        next += 1;
                    }
                }
                written.iter_mut().for_each(Vec::clear);
                span_rows += lives.inserted.len() as u64;
                lives.inserted.clear();
                lives.deleted.clear();
            }
            span_rows += lives.inserted.len() as u64;
        }
        let span_ms = started.elapsed().as_secs_f64() * 1e3;
        let span_read = *counted.read.lock().map_err(|_| io::Error::other("poisoned"))?;
        let started = Instant::now();
        let (rebuilt, _) = reader::rebuild(&counted, pages, &kinds[table], table, last, outcome.first_hour)?;
        let listed_ms = started.elapsed().as_secs_f64() * 1e3;
        return Ok(serde_json::json!({ "state_at_span_start_ms": state_ms, "span_block_rows": span_rows, "span_block_ms": span_ms, "span_reads": span_read.0, "span_stored_mb": span_read.1 as f64 / 1e6,
            "listed_with_cells_ms": listed_ms, "listed": rebuilt.rows().count(), "listed_cells_bytes": rebuilt.bytes().total() }));
    }
    let started = Instant::now();
    let mut span_rows = 0_u64;
    reader::log_span(&counted, pages, &kinds[table], table, &all, outcome.first_hour, span_first..=span_first + 719, &mut |_, event| {
        if let LogEvent::Appended { .. } = event {
            span_rows += 1;
        }
    })?;
    let span_ms = started.elapsed().as_secs_f64() * 1e3;
    let span_read = *counted.read.lock().map_err(|_| io::Error::other("poisoned"))?;
    *counted.read.lock().map_err(|_| io::Error::other("poisoned"))? = (0, 0);
    let started = Instant::now();
    let state: TableState = reader::log_state(&counted, pages, &kinds[table], table, last)?;
    let state_ms = started.elapsed().as_secs_f64() * 1e3;
    let listed = state.listed().expect("a log");
    let (lowest, count) = (listed.lowest().unwrap_or(0), state.count().expect("a log"));
    let restore_read = *counted.read.lock().map_err(|_| io::Error::other("poisoned"))?;
    *counted.read.lock().map_err(|_| io::Error::other("poisoned"))? = (0, 0);
    let started = Instant::now();
    let mut cells_rows = 0_u64;
    reader::log_rows(&counted, pages, &kinds[table], table, &all, lowest..count, &mut |event| {
        if let LogEvent::Appended { ordinal, .. } = event
            && listed.get(ordinal)
        {
            cells_rows += 1;
        }
    })?;
    let cells_ms = started.elapsed().as_secs_f64() * 1e3;
    let cells_read = *counted.read.lock().map_err(|_| io::Error::other("poisoned"))?;
    let mut parts = 0;
    let mut raw = 0_u64;
    for (index, page) in pages.iter().enumerate() {
        let entry = &page.tables[table];
        let end = pages.get(index + 1).map_or(u64::MAX, |next| next.tables[table].count);
        if end <= lowest || entry.count >= count || entry.count == end {
            continue;
        }
        let Some(last_part) = entry.parts.iter().rposition(|part| part.ordinal < count) else { continue };
        parts += last_part + 1;
        raw += entry.parts[..=last_part].iter().flat_map(|part| &part.frames).map(|frame| u64::from(frame.raw)).sum::<u64>();
    }
    Ok(serde_json::json!({ "span_block_rows": span_rows, "span_block_ms": span_ms, "span_reads": span_read.0, "span_stored_mb": span_read.1 as f64 / 1e6,
        "listed_state_ms": state_ms, "listed": listed.len(), "listed_span": count - lowest, "restore_stored_bytes": restore_read.1,
        "listed_cells_ms": cells_ms, "listed_cells_rows": cells_rows, "listed_cells_parts": parts, "listed_cells_raw_mb": raw as f64 / 1e6,
        "listed_cells_stored_mb": cells_read.1 as f64 / 1e6, "listed_cells_reads": cells_read.0 }))
}

fn playback_log() -> io::Result<(Memory, Vec<Kind>)> {
    const RATE: u64 = 1_389;
    let kinds = vec![Kind::I64, Kind::U32, Kind::I32, Kind::U16, Kind::Bytes];
    let all = vec![kinds.clone()];
    let mut store = Store::new(all.clone(), vec![false]);
    store.declare_logs(&[true])?;
    store.attach(Encoder::new(&all, Settings::at(3))?);
    let mut memory = Memory::default();
    store.begin_run(0);
    for id in 0..64 {
        store.pin_string(TEXT + id, format!("name {id}").as_bytes())?;
    }
    let mut oldest = 0;
    for at in 0..10 * BLOCK {
        store.begin_hour(at as i32, &[])?;
        store.status(0, Status::Complete, true, Some(at as i32), true)?;
        let mut changes = store.table(0)?;
        for index in 0..RATE {
            let seed = u64::from(at) * RATE + index;
            let cells = [
                Some(Cell::I64(seed as i64 * -1_000_003)),
                Some(Cell::U32(seed as u32 * 7)),
                Some(Cell::I32(seed as i32 % 5_000 - 2_500)),
                Some(Cell::U16(seed as u16)),
                Some(Cell::Bytes(TEXT + (seed % 64) as u32)),
            ];
            changes.append(u128::from(seed) << 8 | 1, &cells)?;
        }
        let count = u64::from(at + 1) * RATE;
        if count - oldest > 10_000 {
            changes.forget_run(oldest, count - 10_000 - oldest)?;
            oldest = count - 10_000;
        }
        store.end_hour(&mut memory)?;
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    Ok((memory, kinds))
}

#[derive(Default, Clone, Copy)]
struct Took {
    ns: u64,
    fetch_ns: u64,
    parts: u64,
    walked: u64,
    stored: u64,
    decompressed: u64,
    rows: u64,
}

fn needed(pages: &[BlockPage], asked: &[u64]) -> Vec<(usize, usize, Vec<usize>)> {
    let mut blocks = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let entry = &page.tables[0];
        let end = pages.get(index + 1).map_or(u64::MAX, |next| next.tables[0].count);
        let mut holding: Vec<usize> = asked.iter().filter(|&&ordinal| ordinal >= entry.count && ordinal < end).map(|&ordinal| entry.parts.partition_point(|part| part.ordinal <= ordinal) - 1).collect();
        holding.dedup();
        if let Some(&furthest) = holding.last() {
            blocks.push((index, furthest, holding));
        }
    }
    blocks
}

fn before(memory: &Memory, kinds: &[Kind], columns: &[usize], asked: &[u64], walk: bool) -> io::Result<Took> {
    let pages = &memory.pages;
    let mut took = Took::default();
    let started = Instant::now();
    for (index, furthest, holding) in needed(pages, asked) {
        let page = &pages[index];
        let entry = &page.tables[0];
        let mut prefix = None;
        for part_index in 0..=furthest {
            let last = entry.parts.get(part_index + 1).map_or(page.hours - 1, |following| following.first - 1);
            let fetching = Instant::now();
            let part = reader::fetch(&memory.bytes, &entry.parts[part_index], last, columns, prefix.as_ref())?;
            took.fetch_ns += fetching.elapsed().as_nanos() as u64;
            (took.parts, took.stored, took.decompressed) = (took.parts + 1, took.stored + part.stored as u64, took.decompressed + part.decompressed as u64);
            prefix = Some(part.prefix());
            if !walk || holding.binary_search(&part_index).is_err() {
                continue;
            }
            took.walked += 1;
            let mut cursor = LogCursor::new(kinds, columns, entry.parts[part_index].first);
            cursor.push(part);
            let mut next = 0;
            cursor.advance(last, &mut |event| {
                if let LogEvent::Appended { ordinal, rows, at, .. } = event {
                    while next < asked.len() && asked[next] < ordinal {
                        next += 1;
                    }
                    if next < asked.len() && asked[next] == ordinal {
                        took.rows += 1;
                        std::hint::black_box(columns.iter().map(|&column| rows.value(column, at)).count());
                    }
                }
            })?;
        }
    }
    took.ns = started.elapsed().as_nanos() as u64;
    Ok(took)
}

fn after(memory: &Memory, kinds: &[Kind], columns: &[usize], asked: &[u64]) -> io::Result<(Took, u64)> {
    let counted = Counted { bytes: &memory.bytes, read: Mutex::new((0, 0)) };
    let (mut took, mut named) = (Took::default(), 0);
    let started = Instant::now();
    reader::log_rows_at(&counted, &memory.pages, kinds, 0, columns, asked, &mut |event| match event {
        LogEvent::Appended { rows, at, .. } => {
            took.rows += 1;
            std::hint::black_box(columns.iter().map(|&column| rows.value(column, at)).count());
        }
        LogEvent::Named { .. } => named += 1,
        LogEvent::Forgotten { .. } => {}
    })?;
    took.ns = started.elapsed().as_nanos() as u64;
    took.stored = counted.read.lock().map_err(|_| io::Error::other("poisoned"))?.1;
    Ok((took, named))
}

fn cursor_alone(memory: &Memory, kinds: &[Kind], columns: &[usize]) -> io::Result<f64> {
    let (mut ns, mut rows) = (0_u64, 0_u64);
    for page in &memory.pages {
        let entry = &page.tables[0];
        let mut cursor = LogCursor::new(kinds, columns, entry.parts[0].first);
        let mut prefix = None;
        for (index, part) in entry.parts.iter().enumerate() {
            let last = entry.parts.get(index + 1).map_or(page.hours - 1, |following| following.first - 1);
            let part = reader::fetch(&memory.bytes, part, last, columns, prefix.as_ref())?;
            prefix = Some(part.prefix());
            cursor.push(part);
        }
        let started = Instant::now();
        cursor.advance(page.hours - 1, &mut |event| rows += u64::from(matches!(event, LogEvent::Appended { .. })))?;
        ns += started.elapsed().as_nanos() as u64;
    }
    Ok(ns as f64 / rows as f64)
}

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn bench_asked(repeat: usize) -> io::Result<()> {
    let started = Instant::now();
    let (memory, kinds) = playback_log()?;
    let columns: Vec<usize> = (0..kinds.len()).collect();
    let count = 10 * u64::from(BLOCK) * 1_389;
    let parts: usize = memory.pages.iter().map(|page| page.tables[0].parts.len()).sum();
    println!("{}", serde_json::json!({ "bench": "log", "rows": count, "blocks": memory.pages.len(), "parts": parts, "unit_bytes": memory.bytes.len(), "made_s": started.elapsed().as_secs_f64() }));
    let spread: Vec<u64> = (0..1_000).map(|index| index * (count / 1_000) + index % 997).collect();
    let listed: Vec<u64> = (0..1_000).map(|index| count - 10_000 + index * 10).collect();
    for (name, asked) in [("spread over the log", spread), ("among the 10,000 listed", listed)] {
        for columns in [columns.clone(), vec![0, 4]] {
            let (mut old, mut fetched, mut new) = (Vec::new(), Vec::new(), Vec::new());
            let (mut last_old, mut last_fetch, mut last_new, mut named) = (Took::default(), Took::default(), Took::default(), 0);
            for _ in 0..repeat {
                last_old = before(&memory, &kinds, &columns, &asked, true)?;
                old.push(last_old.ns);
                last_fetch = before(&memory, &kinds, &columns, &asked, false)?;
                fetched.push(last_fetch.ns);
                (last_new, named) = after(&memory, &kinds, &columns, &asked)?;
                new.push(last_new.ns);
            }
            if last_old.rows != asked.len() as u64 || last_new.rows != asked.len() as u64 {
                return Err(io::Error::other(format!("{name}: {} and {} rows of {}", last_old.rows, last_new.rows, asked.len())));
            }
            let least = |values: &[u64]| values.iter().copied().min().unwrap_or(0) as f64 / 1e6;
            let (old_min, fetch_min, new_min) = (least(&old), least(&fetched), least(&new));
            let (old_ns, fetch_ns, new_ns) = (median(old), median(fetched), median(new));
            println!(
                "{}",
                serde_json::json!({ "bench": "asked", "where": name, "asked": asked.len(), "columns": columns,
                    "before_ms": old_ns as f64 / 1e6, "before_walk_ms": old_ns.saturating_sub(fetch_ns) as f64 / 1e6,
                    "after_ms": new_ns as f64 / 1e6, "fetch_ms": fetch_ns as f64 / 1e6, "after_decode_ms": new_ns.saturating_sub(fetch_ns) as f64 / 1e6,
                    "parts_fetched": last_fetch.parts, "parts_walked_before": last_old.walked, "parts_walked_after": needed(&memory.pages, &asked).iter().map(|block| block.2.len()).sum::<usize>(),
                    "stored_bytes": last_fetch.stored, "after_stored_bytes_read": last_new.stored, "decompressed_bytes": last_fetch.decompressed, "strings_named": named, "repeat": repeat,
                    "least": { "before_ms": old_min, "fetch_ms": fetch_min, "after_ms": new_min, "after_decode_ms": new_min - fetch_min } })
            );
        }
    }
    let alone: Vec<u64> = (0..repeat).map(|_| cursor_alone(&memory, &kinds, &columns).map(|ns| (ns * 1000.0) as u64)).collect::<io::Result<_>>()?;
    println!("{}", serde_json::json!({ "bench": "cursor", "ns_a_row": median(alone) as f64 / 1000.0, "repeat": repeat }));
    Ok(())
}

fn main() -> io::Result<()> {
    let (mut rows, mut hours, mut cap, mut level) = (2_000_000_u64, 1_467_u32, 200_000_u64, 7);
    if std::env::args().nth(1).as_deref() == Some("asked") {
        eprintln!("{}", serde_json::json!({ "build": { "features": [], "profile": if cfg!(debug_assertions) { "debug" } else { "release" } } }));
        let repeat = std::env::args().nth(2).and_then(|option| option.strip_prefix("--repeat=").and_then(|value| value.parse().ok())).unwrap_or(5);
        return bench_asked(repeat);
    }
    for option in std::env::args().skip(1) {
        let (name, value) = option.split_once('=').ok_or_else(|| io::Error::other("options are --name=value"))?;
        match name {
            "--rows" => rows = value.parse().map_err(io::Error::other)?,
            "--hours" => hours = value.parse().map_err(io::Error::other)?,
            "--cap" => cap = value.parse().map_err(io::Error::other)?,
            "--level" => level = value.parse().map_err(io::Error::other)?,
            _ => return Err(io::Error::other(format!("unknown option {name}"))),
        }
    }
    eprintln!("{}", serde_json::json!({ "build": { "features": [], "profile": if cfg!(debug_assertions) { "debug" } else { "release" } } }));
    let schema = schema()?;
    let kinds: Vec<Vec<Kind>> = TABLES.iter().map(|table| schema[table.index()].clone()).collect();
    for mode in [Mode::Log, Mode::LogNoForgets, Mode::State] {
        let outcome = run(mode, &kinds, rows, hours, cap, level)?;
        let readers = read(&outcome, &kinds)?;
        println!(
            "{}",
            serde_json::json!({ "mode": format!("{mode:?}"), "level": level, "hours": hours, "rows": outcome.rows, "forgets": outcome.forgets,
                "apply_ns_per_row": outcome.apply.as_nanos() as f64 / outcome.rows as f64,
                "end_hours_s": outcome.end_hours.as_secs_f64(),
                "store_peak_bytes": outcome.store_peak, "tables_peak_bytes": outcome.tables_peak,
                "unit_stored_bytes": outcome.stored, "lifetimes_stored_bytes": outcome.lifetimes, "restores_stored_bytes": outcome.restores,
                "blocks": outcome.blocks, "file_bytes": outcome.memory.bytes.len(), "readers": readers })
        );
    }
    Ok(())
}
