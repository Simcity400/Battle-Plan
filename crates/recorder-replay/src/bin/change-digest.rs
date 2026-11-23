use rayon::prelude::*;
use replay_recording::{columnar_frames::Recording, schema::schema};
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{Listed, TableState, Value as Held},
    unit::{
        self, BlockPage,
        reader::{Advanced, Cursor, Extents, Lives, LogCursor, LogEvent, fetch},
    },
};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write as _,
    io::{self, Write as _},
};

const BLOCK: u32 = 720;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[derive(Clone, Copy)]
enum Value<'a> {
    Unrecorded,
    Missing,
    Number(i128),
    Bytes(&'a [u8]),
    Text(Option<&'a [u8]>, u32),
}

fn put_value(out: &mut Vec<u8>, value: Value<'_>) {
    match value {
        Value::Unrecorded => out.push(0),
        Value::Missing => out.push(1),
        Value::Number(number) => {
            out.push(2);
            out.extend_from_slice(&number.to_le_bytes());
        }
        Value::Bytes(bytes) => {
            out.push(3);
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        Value::Text(Some(text), _) => {
            out.push(4);
            out.extend_from_slice(&(text.len() as u64).to_le_bytes());
            out.extend_from_slice(text);
        }
        Value::Text(None, id) => {
            out.push(5);
            out.extend_from_slice(&id.to_le_bytes());
        }
    }
}

fn put_status<'t>(out: &mut Vec<u8>, status: Status, sampled: Option<i32>, text: &dyn Fn(u32) -> Option<&'t [u8]>) {
    match status {
        Status::NotRecorded => out.push(0),
        Status::Complete => out.push(1),
        Status::Truncated { missing } => {
            out.push(2);
            out.extend_from_slice(&missing.to_le_bytes());
        }
        Status::Unavailable { reason } => {
            out.push(3);
            put_value(out, Value::Text(text(reason), reason));
        }
    }
    match sampled {
        None => out.push(0),
        Some(hour) => {
            out.push(1);
            out.extend_from_slice(&hour.to_le_bytes());
        }
    }
}

trait Rows {
    fn columns(&self) -> usize;
    fn key(&self, slot: u32) -> Option<u128>;
    fn live(&self) -> Vec<u32>;
    fn value(&self, column: usize, slot: u32) -> Value<'_>;
}

#[derive(Default)]
struct HourChanges {
    read: Option<Vec<u32>>,
    deleted_keys: Vec<u128>,
    inserted: Vec<u32>,
    written: Vec<(u32, usize)>,
    touched: Vec<u32>,
    forgotten: Vec<u64>,
}

struct TableDigest {
    chain: blake3::Hasher,
    status: Option<(Status, Option<i32>)>,
    hours: u32,
    traced: bool,
}

fn shown(value: Value<'_>) -> String {
    match value {
        Value::Unrecorded => "unrecorded".into(),
        Value::Missing => "missing".into(),
        Value::Number(number) => number.to_string(),
        Value::Bytes(bytes) => hex(bytes),
        Value::Text(Some(text), _) => format!("{:?}", String::from_utf8_lossy(text)),
        Value::Text(None, id) => format!("text#{id}"),
    }
}

impl TableDigest {
    fn new() -> Self {
        Self {
            chain: blake3::Hasher::new(),
            status: None,
            hours: 0,
            traced: false,
        }
    }

    fn at_run_start(mut self) -> Self {
        self.status = Some((Status::NotRecorded, None));
        self
    }

    fn for_table(table: usize) -> Self {
        let traced = std::env::var("BATTLEPLAN_DIGEST_TRACE").or_else(|_| std::env::var("HOI4_DIGEST_TRACE")).is_ok_and(|name| name == format!("{:?}", TableId::ALL[table]));
        Self { traced, ..Self::new() }
    }

    fn hour<'t>(
        &mut self,
        hour: i32,
        status: (Status, Option<i32>),
        rows: &dyn Rows,
        changes: &HourChanges,
        text: &dyn Fn(u32) -> Option<&'t [u8]>,
        out: &mut Vec<u8>,
    ) {
        out.clear();
        out.extend_from_slice(&hour.to_le_bytes());
        let empty = out.len();
        if self.status != Some(status) {
            out.push(b'S');
            put_status(out, status.0, status.1, text);
            self.status = Some(status);
        }
        let keyed = |slots: &[u32]| {
            let mut keys: Vec<u128> = slots.iter().filter_map(|&slot| rows.key(slot)).collect();
            keys.sort_unstable();
            keys
        };
        if let Some(read) = &changes.read {
            out.push(b'R');
            for key in keyed(read) {
                out.extend_from_slice(&key.to_le_bytes());
            }
        }
        if !changes.deleted_keys.is_empty() {
            let mut keys = changes.deleted_keys.clone();
            keys.sort_unstable();
            out.push(b'D');
            out.extend_from_slice(&(keys.len() as u64).to_le_bytes());
            for key in keys {
                out.extend_from_slice(&key.to_le_bytes());
            }
        }
        if !changes.inserted.is_empty() {
            let mut rows_in: Vec<(u128, u32)> =
                changes.inserted.iter().map(|&slot| (rows.key(slot).unwrap_or(u128::MAX), slot)).collect();
            rows_in.sort_unstable();
            out.push(b'I');
            out.extend_from_slice(&(rows_in.len() as u64).to_le_bytes());
            for (key, slot) in rows_in {
                out.extend_from_slice(&key.to_le_bytes());
                for column in 0..rows.columns() {
                    put_value(out, rows.value(column, slot));
                }
            }
        }
        if !changes.forgotten.is_empty() {
            let mut ordinals = changes.forgotten.clone();
            ordinals.sort_unstable();
            out.push(b'F');
            out.extend_from_slice(&(ordinals.len() as u64).to_le_bytes());
            for ordinal in ordinals {
                out.extend_from_slice(&ordinal.to_le_bytes());
            }
        }
        if !changes.written.is_empty() {
            let mut cells: Vec<(u128, usize, u32)> = changes
                .written
                .iter()
                .filter(|(slot, _)| changes.inserted.binary_search(slot).is_err())
                .filter_map(|&(slot, column)| rows.key(slot).map(|key| (key, column, slot)))
                .collect();
            cells.sort_unstable();
            cells.dedup();
            if !cells.is_empty() {
                out.push(b'W');
                out.extend_from_slice(&(cells.len() as u64).to_le_bytes());
                for (key, column, slot) in cells {
                    out.extend_from_slice(&key.to_le_bytes());
                    out.extend_from_slice(&(column as u32).to_le_bytes());
                    put_value(out, rows.value(column, slot));
                }
            }
        }
        if !changes.touched.is_empty() {
            out.push(b'T');
            for key in keyed(&changes.touched) {
                out.extend_from_slice(&key.to_le_bytes());
            }
        }
        if self.traced && out.len() > empty {
            let mut cells: Vec<String> = changes
                .written
                .iter()
                .filter(|(slot, _)| changes.inserted.binary_search(slot).is_err())
                .filter_map(|&(slot, column)| rows.key(slot).map(|key| format!("{key:x}/{column}={}", shown(rows.value(column, slot)))))
                .collect();
            cells.sort();
            cells.dedup();
            let mut inserted: Vec<String> = changes
                .inserted
                .iter()
                .map(|&slot| format!("{:x}:{}", rows.key(slot).unwrap_or(0), (0..rows.columns()).map(|column| shown(rows.value(column, slot))).collect::<Vec<_>>().join(",")))
                .collect();
            inserted.sort();
            let mut deleted: Vec<String> = changes.deleted_keys.iter().map(|key| format!("{key:x}")).collect();
            deleted.sort();
            let read: Option<Vec<String>> = changes.read.as_ref().map(|read| keyed(read).iter().map(|key| format!("{key:x}")).collect());
            eprintln!("TRACE hour {hour} status {:?} record {} read {read:?} inserted {inserted:?} deleted {deleted:?} written {cells:?}", status, hex(&blake3::hash(out).as_bytes()[..6]));
        }
        if out.len() > empty {
            self.chain.update(&(out.len() as u64).to_le_bytes());
            self.chain.update(out);
        }
        self.hours += 1;
    }

    fn log_end<'t>(&mut self, count: u64, listed: &Listed, text: &dyn Fn(u32) -> Option<&'t [u8]>) -> (String, String, usize) {
        let chain = std::mem::replace(&mut self.chain, blake3::Hasher::new()).finalize();
        self.hours = 0;
        let mut out = Vec::new();
        if let Some((status, sampled)) = self.status {
            put_status(&mut out, status, sampled, text);
        }
        out.push(b'L');
        out.extend_from_slice(&count.to_le_bytes());
        let mut state = blake3::Hasher::new();
        state.update(&out);
        for ordinal in listed.ones() {
            state.update(&ordinal.to_le_bytes());
        }
        (hex(&chain.as_bytes()[..16]), hex(&state.finalize().as_bytes()[..16]), listed.len())
    }

    fn block_end<'t>(&mut self, rows: &dyn Rows, view: Option<Vec<u32>>, text: &dyn Fn(u32) -> Option<&'t [u8]>) -> (String, String, usize) {
        let chain = std::mem::replace(&mut self.chain, blake3::Hasher::new()).finalize();
        self.hours = 0;
        let mut state = blake3::Hasher::new();
        let mut out = Vec::new();
        if let Some((status, sampled)) = self.status {
            put_status(&mut out, status, sampled, text);
        }
        if let Some(view) = view {
            out.push(b'R');
            let mut keys: Vec<u128> = view.iter().filter_map(|&slot| rows.key(slot)).collect();
            keys.sort_unstable();
            keys.iter().for_each(|key| out.extend_from_slice(&key.to_le_bytes()));
        }
        state.update(&out);
        let mut live: Vec<(u128, u32)> = rows.live().into_iter().map(|slot| (rows.key(slot).unwrap_or(u128::MAX), slot)).collect();
        live.sort_unstable();
        for &(key, slot) in &live {
            out.clear();
            out.extend_from_slice(&key.to_le_bytes());
            for column in 0..rows.columns() {
                put_value(&mut out, rows.value(column, slot));
            }
            state.update(&out);
            if self.traced {
                eprintln!("DUMP {key:x} {}", (0..rows.columns()).map(|column| shown(rows.value(column, slot))).collect::<Vec<_>>().join(","));
            }
        }
        (hex(&chain.as_bytes()[..16]), hex(&state.finalize().as_bytes()[..16]), live.len())
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("recording") => recording_mode(&args[1..]),
        Some("journal") => journal_mode(&args[1..]),
        _ => Err(invalid(USAGE)),
    }
}

const USAGE: &str = "change-digest recording <format-12 recording> [--threads=N] [--first-block=N] [--newest] [--rows=<file>] [--seek] | change-digest journal <session directory> <capture> [--times=<factors>] [--start=<block> --start-file=<file>] [--hours=N]";

#[derive(Clone, Default)]
struct Archive(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);

impl replay_columnar::store::StringSource for Archive {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.lock().map_err(|_| invalid("The archive's lock is poisoned"))?.get(id as usize).cloned())
    }
}

fn journal_mode(args: &[String]) -> io::Result<()> {
    use replay_journal::{columnar_journal::{Record, Source}, multiplied::{Factors, Multiplied, Start}};
    use replay_columnar::{
        manifest::Manifest,
        store::{Store, journal::Journal as Adapter},
    };
    let [directory, capture, rest @ ..] = args else {
        return Err(invalid(USAGE));
    };
    let capture: u32 = capture.parse().map_err(|_| invalid(USAGE))?;
    let (mut block, mut file, mut limit, mut times) = (None, None, u64::MAX, None);
    for option in rest {
        if let Some(given) = option.strip_prefix("--start=") {
            block = Some(given.rsplit_once(',').map_or(given, |(_, block)| block).parse::<u32>().map_err(|_| invalid(USAGE))?);
        } else if let Some(given) = option.strip_prefix("--start-file=") {
            file = Some(std::path::PathBuf::from(given));
        } else if let Some(given) = option.strip_prefix("--hours=") {
            limit = given.parse().map_err(|_| invalid(USAGE))?;
        } else if let Some(given) = option.strip_prefix("--times=") {
            times = Some(Factors::read(std::path::Path::new(given))?);
        } else {
            return Err(invalid(USAGE));
        }
    }
    let start = match (block, file) {
        (None, None) => None,
        (Some(block), Some(file)) => Some(Start { block, file }),
        _ => return Err(invalid(USAGE)),
    };
    let first_block = start.as_ref().map_or(0, |start| start.block + 1);
    let tables = TableId::ALL.len();
    let factors = times.unwrap_or_else(|| Factors { factor: vec![1; tables], in_phase: vec![false; tables] });
    let mut source = Multiplied::open(std::path::Path::new(directory), capture, factors, start)?;
    let derived: Vec<_> = TableId::ALL.into_iter().filter(|table| table.is_derived()).map(TableId::index).collect();
    let kinds = schema()?;
    let columns: Vec<usize> = kinds.iter().map(Vec::len).collect();
    let digested: Vec<usize> = (0..kinds.len()).filter(|table| !derived.contains(table)).collect();
    let archive = Archive::default();
    let mut store = Store::new(kinds.clone(), replay_recording::schema::keeps_last().to_vec());
    store.declare_logs(&replay_data::rows::logs())?;
    store.read_strings_from(Box::new(archive.clone()));
    let mut adapter = Adapter::leaving(&derived, replay_recording::schema::names());
    let mut digests: Vec<TableDigest> = (0..kinds.len()).map(TableDigest::for_table).map(TableDigest::at_run_start).collect();
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut whole = blake3::Hasher::new();
    let (mut lines, mut ordinal, mut hours) = (0_u64, 0_u32, 0_u64);
    let mut run_seen = false;
    let started = std::time::Instant::now();
    let mut scratch = Vec::new();
    let mut emit = |store: &Store, digests: &mut [TableDigest], ordinal: u32, hours: u64, out: &mut io::BufWriter<_>| -> io::Result<()> {
        let block = first_block + ordinal / BLOCK;
        for &table in &digested {
            let held = store.table_state(table).expect("a schema table");
            let rows = Stored { table: held, kinds: store.kinds(table), store };
            let text = |id: u32| store.strings().get(id);
            let (chain, state, live) = match (held.count(), held.listed()) {
                (Some(count), Some(listed)) => digests[table].log_end(count, listed, &text),
                _ => digests[table].block_end(&rows, held.view().map(<[u32]>::to_vec), &text),
            };
            let line = format!("0 {block} {:?} {chain} {state} {live}", TableId::ALL[table]);
            whole.update(line.as_bytes());
            whole.update(b"\n");
            writeln!(out, "{line}")?;
            lines += 1;
        }
        eprintln!("{}", serde_json::json!({ "block": block, "hours": hours, "seconds": started.elapsed().as_secs_f64(), "store": store.bytes().0.total() }));
        Ok(())
    };
    loop {
        if hours >= limit {
            break;
        }
        let Some(record) = source.record()? else { break };
        match record {
            Record::Run { first_hour, .. } => {
                if std::mem::replace(&mut run_seen, true) {
                    return Err(invalid("change-digest journal digests a journal of one run, and this one holds another"));
                }
                store.begin_run(first_hour);
                ordinal = 0;
            }
            Record::Hour { manifest, mut sections, .. } => {
                let manifest = Manifest::read(manifest, &columns)?;
                archive.0.lock().map_err(|_| invalid("The archive's lock is poisoned"))?.extend(manifest.appends.iter().map(|text| text.to_vec()));
                adapter.begin_hour(&mut store, &manifest)?;
                for entry in &manifest.sections {
                    let bytes = sections.next(entry.bytes)?;
                    adapter.section(&mut store, entry, bytes)?;
                }
                adapter.applied(&mut store);
                for &table in &digested {
                    let held = store.table_state(table).expect("a schema table");
                    let log = &store.log().tables[table];
                    let mut changes = HourChanges::default();
                    if held.is_log() {
                        changes.inserted = (0..log.appended).collect();
                        changes.forgotten = log.forgotten.iter().flat_map(Clone::clone).collect();
                        let view = store.view(table);
                        let rows = Stored { table: view.state(), kinds: store.kinds(table), store: &store };
                        let text = |id: u32| store.strings().get(id);
                        digests[table].hour(manifest.hour, (held.status(), held.sampled_at()), &rows, &changes, &text, &mut scratch);
                        continue;
                    }
                    if log.whole {
                        changes.inserted = held.rows().collect();
                        changes.read = held.view().map(<[u32]>::to_vec);
                    } else if log.present {
                        changes.read = held.view().map(<[u32]>::to_vec);
                        changes.deleted_keys = log.deleted.iter().map(|&(_, key)| key).collect();
                        changes.inserted = log.inserted.clone();
                        changes.written = log.written.iter().map(|&(slot, column)| (slot, usize::from(column))).collect();
                        changes.touched = log.touched.clone();
                    }
                    let rows = Stored { table: held, kinds: store.kinds(table), store: &store };
                    let text = |id: u32| store.strings().get(id);
                    digests[table].hour(manifest.hour, (held.status(), held.sampled_at()), &rows, &changes, &text, &mut scratch);
                }
                store.end_hour(&mut unit::Discard)?;
                adapter.moved(&mut store);
                hours += 1;
                if ordinal % BLOCK == BLOCK - 1 {
                    emit(&store, &mut digests, ordinal, hours, &mut out)?;
                    store.end_block();
                    adapter.shed(&mut store);
                } else if ordinal == 0 {
                    adapter.shed(&mut store);
                }
                ordinal += 1;
            }
            Record::Finish { .. } => break,
            _ => {}
        }
        source.advance();
    }
    if ordinal % BLOCK != 0 {
        emit(&store, &mut digests, ordinal - 1, hours, &mut out)?;
    }
    writeln!(out, "lines {lines} digest {}", hex(&whole.finalize().as_bytes()[..16]))?;
    out.flush()?;
    eprintln!("{}", serde_json::json!({ "store_alone": true, "hours": hours, "seconds": started.elapsed().as_secs_f64(), "multiplier_s": source.spent.as_secs_f64() }));
    Ok(())
}

struct Stored<'a> {
    table: &'a TableState,
    kinds: &'a [Kind],
    store: &'a replay_columnar::store::Store,
}

impl Rows for Stored<'_> {
    fn columns(&self) -> usize {
        self.kinds.len()
    }

    fn key(&self, slot: u32) -> Option<u128> {
        self.table.key(slot)
    }

    fn live(&self) -> Vec<u32> {
        self.table.rows().collect()
    }

    fn value(&self, column: usize, slot: u32) -> Value<'_> {
        match self.table.value(column, slot) {
            Held::Unrecorded => Value::Unrecorded,
            Held::Missing => Value::Missing,
            Held::Number(bits) => {
                let signed = matches!(self.kinds[column], Kind::I8 | Kind::I16 | Kind::I32 | Kind::I64 | Kind::OptionalI32 | Kind::OptionalI64);
                Value::Number(if signed { i128::from(bits as i64) } else { i128::from(bits) })
            }
            Held::Text(id) => Value::Text(self.store.strings().get(id), id),
            Held::List(bytes) => Value::Bytes(bytes),
        }
    }
}

const EPOCHED: [TableId; 2] = [TableId::TagHolders, TableId::LookSpans];

fn epoched_lines(hour: i32, name: TableId, status: Status, cells: &TableState, texts: &Texts<'_>, held: &mut (String, BTreeSet<String>), out: &mut Vec<String>) {
    let shown = match status {
        Status::Complete => "complete",
        Status::Truncated { .. } => "truncated",
        Status::Unavailable { .. } => "unavailable",
        Status::NotRecorded => "not-recorded",
    };
    let mut rows = BTreeSet::new();
    if matches!(status, Status::Complete) {
        for slot in cells.rows() {
            let key = cells.key(slot).unwrap_or_default();
            rows.insert(match name {
                TableId::TagHolders => {
                    let tag = match cells.value(0, slot) {
                        Held::Text(id) => texts.get(id).map_or_else(|| format!("?{id}"), hex),
                        other => format!("{other:?}"),
                    };
                    format!("{} {} {} {} {tag}", (key >> 96) as u32, (key >> 64) as u32, (key >> 32) as u32, key as u32)
                }
                _ => {
                    let read = match cells.value(0, slot) {
                        Held::Number(bits) => bits.to_string(),
                        other => format!("{other:?}"),
                    };
                    format!("{} {} {} {read}", (key >> 64) as u32, key as u32, (key >> 32) as u32)
                }
            });
        }
    }
    let (last_status, last) = held;
    if last_status != shown {
        out.push(format!("{hour} {name:?} !{shown}"));
        *last_status = shown.to_owned();
    }
    out.extend(last.difference(&rows).map(|gone| format!("{hour} {name:?} - {gone}")));
    out.extend(rows.difference(last).map(|added| format!("{hour} {name:?} + {added}")));
    *last = rows;
}

fn state_line(table: &TableState, kinds: &[Kind], texts: &HashMap<u32, Vec<u8>>, status: (Status, Option<i32>)) -> String {
    let mut digest = TableDigest::new();
    digest.status = Some(status);
    let rows = Read { table, kinds, texts };
    let text = |id: u32| texts.get(&id).map(Vec::as_slice);
    digest.block_end(&rows, table.view().map(<[u32]>::to_vec), &text).1
}

struct Read<'a> {
    table: &'a TableState,
    kinds: &'a [Kind],
    texts: &'a HashMap<u32, Vec<u8>>,
}

impl Rows for Read<'_> {
    fn columns(&self) -> usize {
        self.kinds.len()
    }

    fn key(&self, slot: u32) -> Option<u128> {
        self.table.key(slot)
    }

    fn live(&self) -> Vec<u32> {
        self.table.rows().collect()
    }

    fn value(&self, column: usize, slot: u32) -> Value<'_> {
        match self.table.value(column, slot) {
            Held::Unrecorded => Value::Unrecorded,
            Held::Missing => Value::Missing,
            Held::Number(bits) => {
                let signed = matches!(self.kinds[column], Kind::I8 | Kind::I16 | Kind::I32 | Kind::I64 | Kind::OptionalI32 | Kind::OptionalI64);
                Value::Number(if signed { i128::from(bits as i64) } else { i128::from(bits) })
            }
            Held::Text(id) => Value::Text(self.texts.get(&id).map(Vec::as_slice), id),
            Held::List(bytes) => Value::Bytes(bytes),
        }
    }
}

struct Texts<'a> {
    own: HashMap<u32, Vec<u8>>,
    appended: &'a HashMap<u32, Vec<u8>>,
}

impl Texts<'_> {
    fn get(&self, id: u32) -> Option<&[u8]> {
        self.own.get(&id).or_else(|| self.appended.get(&id)).map(Vec::as_slice)
    }

    fn named(&self, cells: &TableState) -> HashMap<u32, Vec<u8>> {
        let mut out = HashMap::new();
        let mut name = |id: u32| {
            if let Some(text) = self.get(id) {
                out.entry(id).or_insert_with(|| text.to_vec());
            }
        };
        for column in 0..cells.columns().len() {
            if !matches!(cells.columns()[column].kind(), Kind::Bytes) {
                continue;
            }
            for slot in cells.rows() {
                if let Held::Text(id) = cells.value(column, slot) {
                    name(id);
                }
            }
        }
        if let Status::Unavailable { reason } = cells.status() {
            name(reason);
        }
        out
    }
}

#[derive(Default)]
struct Checked {
    restores: u64,
    differing: u64,
    tracks: u64,
    fetch: std::time::Duration,
    seek: Seeks,
}

#[derive(Clone, Copy, Default)]
struct Seeks {
    restores: u64,
    block_ends: u64,
    cells: u64,
    texts_missing: u64,
    differing: u64,
}

impl std::ops::AddAssign for Seeks {
    fn add_assign(&mut self, other: Self) {
        self.restores += other.restores;
        self.block_ends += other.block_ends;
        self.cells += other.cells;
        self.texts_missing += other.texts_missing;
        self.differing += other.differing;
    }
}

struct Seek {
    cells: TableState,
    texts: HashMap<u32, Vec<u8>>,
}

impl Seek {
    fn from(file: &dyn Extents, entry: &unit::restore::RestoreEntry, kinds: &[Kind], all: &[usize]) -> io::Result<Self> {
        let (cells, meta) = unit::reader::restore(file, entry, kinds, all)?;
        Ok(Self { cells, texts: meta.strings.into_iter().collect() })
    }

    fn block(&mut self, file: &dyn Extents, page: &BlockPage, table: usize, kinds: &[Kind], all: &[usize], start: u16) -> io::Result<()> {
        let entry = &page.tables[table];
        let mut cursor = Cursor::new(kinds, all, start);
        let mut written: Vec<Vec<u32>> = vec![Vec::new(); all.len()];
        let mut lives = Lives::default();
        let mut next = 0;
        loop {
            match cursor.advance(page.hours - 1, BLOCK as u16, &mut self.cells, &mut written, &mut lives)? {
                Advanced::Reached => return Ok(()),
                Advanced::Missing { .. } => {
                    let Some(part) = entry.parts.get(next) else { return Ok(()) };
                    let last = entry.parts.get(next + 1).map_or(page.hours - 1, |following| following.first - 1);
                    let part = fetch(file, part, last, all, cursor.prefix().as_ref())?;
                    self.texts.extend(part.strings().iter().cloned());
                    cursor.push(part);
                    next += 1;
                }
            }
            written.iter_mut().for_each(Vec::clear);
            lives.deleted.clear();
            lives.inserted.clear();
        }
    }

    fn compare(&self, forward: &TableState, texts: &Texts<'_>, reasons: &BTreeSet<u32>, seeks: &mut Seeks) {
        seeks.block_ends += 1;
        let by_key = |table: &TableState| -> HashMap<u128, u32> { table.rows().filter_map(|slot| Some((table.key(slot)?, slot))).collect() };
        let (ours, theirs) = (by_key(forward), by_key(&self.cells));
        seeks.differing += ours.keys().filter(|key| !theirs.contains_key(key)).count() as u64;
        seeks.differing += theirs.keys().filter(|key| !ours.contains_key(key)).count() as u64;
        let text = |id: u32, seeks: &mut Seeks| match self.texts.get(&id) {
            None => seeks.texts_missing += 1,
            Some(bytes) if texts.get(id) != Some(bytes.as_slice()) => seeks.differing += 1,
            Some(_) => {}
        };
        for (key, &slot) in &ours {
            let Some(&other) = theirs.get(key) else { continue };
            for column in 0..forward.columns().len() {
                seeks.cells += 1;
                match (forward.value(column, slot), self.cells.value(column, other)) {
                    (Held::Text(_), Held::Text(id)) => text(id, seeks),
                    (held, seen) if held != seen => seeks.differing += 1,
                    _ => {}
                }
            }
        }
        for &reason in reasons {
            text(reason, seeks);
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "one table's walk over the run, given whole")]
fn read_table(
    file: &dyn Extents,
    pages: &[BlockPage],
    kinds: &[Kind],
    table: usize,
    run: u32,
    run_first: i32,
    first_block: u32,
    appended: &HashMap<u32, Vec<u8>>,
    dump: bool,
    seek: bool,
) -> io::Result<(Vec<String>, Checked, Vec<String>)> {
    let name = TableId::ALL[table];
    let mut dumped = Vec::new();
    let mut epoched = (String::new(), BTreeSet::new());
    let all: Vec<usize> = (0..kinds.len()).collect();
    let mut digest = TableDigest::for_table(table).at_run_start();
    let mut checked = Checked::default();
    let mut texts = Texts { own: HashMap::new(), appended };
    let mut scratch = Vec::new();
    let mut lines = Vec::with_capacity(pages.len());
    let first = pages
        .first()
        .and_then(|page| page.tables[table].first.as_ref())
        .ok_or_else(|| invalid(format!("{name:?}: the run's first block holds no first restore")))?;
    let (mut cells, meta) = unit::reader::restore(file, first, kinds, &all)?;
    texts.own.extend(meta.strings);
    let mut seeking = if seek { Some(Seek::from(file, first, kinds, &all)?) } else { None };
    checked.seek.restores += u64::from(seek);
    let mut reasons = BTreeSet::new();
    {
        let (status, _, sampled, _) = unit::reader::status(&pages[0].tables[table], 0, run_first)?;
        let changes = HourChanges { inserted: cells.rows().collect(), read: cells.view().map(<[u32]>::to_vec), ..HourChanges::default() };
        let named = texts.named(&cells);
        let rows = Read { table: &cells, kinds, texts: &named };
        let text = |id: u32| texts.get(id);
        digest.hour(run_first, (status, sampled), &rows, &changes, &text, &mut scratch);
        if let Status::Unavailable { reason } = status {
            reasons.insert(reason);
        }
        if dump {
            epoched_lines(run_first, name, status, &cells, &texts, &mut epoched, &mut dumped);
        }
    }
    for (index, page) in pages.iter().enumerate() {
        let entry = &page.tables[table];
        let start_hour = if index == 0 { 1 } else { 0 };
        let mut cursor = Cursor::new(kinds, &all, start_hour);
        let mut next_part = 0;
        let mut written: Vec<Vec<u32>> = vec![Vec::new(); all.len()];
        let block_first = run_first + (page.block * BLOCK) as i32;
        for hour in start_hour..page.hours {
            written.iter_mut().for_each(Vec::clear);
            let mut lives = Lives::default();
            loop {
                match cursor.advance(hour, 1, &mut cells, &mut written, &mut lives).map_err(|error| invalid(format!("{name:?} block {} hour {hour}: {error}", page.block)))? {
                    Advanced::Reached => break,
                    Advanced::Missing { .. } => {
                        let Some(part) = entry.parts.get(next_part) else { break };
                        let last = entry.parts.get(next_part + 1).map_or(page.hours - 1, |next| next.first - 1);
                        let fetched = std::time::Instant::now();
                        let part = fetch(file, part, last, &all, cursor.prefix().as_ref())?;
                        checked.fetch += fetched.elapsed();
                        checked.tracks += part.track_count() as u64;
                        for (id, text) in part.strings() {
                            texts.own.insert(*id, text.clone());
                        }
                        cursor.push(part);
                        next_part += 1;
                    }
                }
            }
            let (status, _, sampled, _) = unit::reader::status(entry, hour, block_first)?;
            if let Status::Unavailable { reason } = status {
                reasons.insert(reason);
            }
            let mut touched = Vec::new();
            let mut read = Vec::new();
            cursor.marks(hour, hour, &mut touched, &mut read)?;
            let mut changes = HourChanges {
                deleted_keys: lives.deleted.iter().map(|&(_, _, key)| key).collect(),
                inserted: lives.inserted.iter().map(|&(_, slot, _)| slot).collect(),
                touched: touched.iter().map(|&(_, slot)| slot).collect(),
                ..HourChanges::default()
            };
            changes.inserted.sort_unstable();
            if matches!(status, Status::Truncated { .. }) {
                let mut view: Vec<u32> = read.iter().map(|&(_, slot)| slot).filter(|&slot| cells.live().get(slot)).collect();
                view.extend_from_slice(&changes.inserted);
                view.sort_unstable();
                view.dedup();
                changes.read = Some(view);
            }
            for (column, slots) in written.iter().enumerate() {
                changes.written.extend(slots.iter().map(|&slot| (slot, column)));
            }
            let rows = TextRows { table: &cells, kinds, texts: &texts };
            let text = |id: u32| texts.get(id);
            digest.hour(block_first + i32::from(hour), (status, sampled), &rows, &changes, &text, &mut scratch);
            if dump {
                epoched_lines(block_first + i32::from(hour), name, status, &cells, &texts, &mut epoched, &mut dumped);
            }
        }
        let named = texts.named(&cells);
        let rows = Read { table: &cells, kinds, texts: &named };
        let text = |id: u32| texts.get(id);
        let view = cells.view().map(<[u32]>::to_vec);
        let status = digest.status.unwrap_or((Status::NotRecorded, None));
        let (chain, state, live) = digest.block_end(&rows, view, &text);
        if let Some(seeking) = &mut seeking {
            let before = checked.seek;
            seeking.block(file, page, table, kinds, &all, start_hour)?;
            seeking.compare(&cells, &texts, &reasons, &mut checked.seek);
            if checked.seek.texts_missing + checked.seek.differing > before.texts_missing + before.differing {
                eprintln!(
                    "{}",
                    serde_json::json!({ "seek_differs": format!("{name:?}"), "block": first_block + page.block,
                        "texts_missing": checked.seek.texts_missing - before.texts_missing, "differing": checked.seek.differing - before.differing })
                );
            }
        }
        reasons.clear();
        if seek && let Some(restore) = &entry.restore {
            seeking = Some(Seek::from(file, restore, kinds, &all)?);
            checked.seek.restores += 1;
        }
        if let Some(restore) = &entry.restore {
            let (restored, meta) = unit::reader::restore(file, restore, kinds, &all)?;
            let mut held = Texts { own: texts.own.clone(), appended };
            held.own.extend(meta.strings);
            let restored_texts = held.named(&restored);
            let restored_state = state_line(&restored, kinds, &restored_texts, (restored.status(), restored.sampled_at()));
            checked.restores += 1;
            if restored_state != state || (restored.status(), restored.sampled_at()) != status {
                checked.differing += 1;
                eprintln!("{}", serde_json::json!({ "restore_differs": format!("{name:?}"), "block": first_block + page.block }));
            }
        }
        if entry.packed {
            cells.pack()?;
        }
        lines.push(format!("{run} {} {name:?} {chain} {state} {live}", first_block + page.block));
    }
    Ok((lines, checked, dumped))
}

#[allow(clippy::too_many_arguments, reason = "one table's walk over the run, given whole")]
fn read_log(
    file: &dyn Extents,
    pages: &[BlockPage],
    kinds: &[Kind],
    table: usize,
    run: u32,
    run_first: i32,
    first_block: u32,
    appended: &HashMap<u32, Vec<u8>>,
) -> io::Result<(Vec<String>, Checked, Vec<String>)> {
    let name = TableId::ALL[table];
    let all: Vec<usize> = (0..kinds.len()).collect();
    let mut digest = TableDigest::for_table(table).at_run_start();
    let mut checked = Checked::default();
    let mut texts = Texts { own: HashMap::new(), appended };
    let mut scratch = Vec::new();
    let mut lines = Vec::with_capacity(pages.len());
    let (mut count, mut listed) = (0_u64, Listed::default());
    for (index, page) in pages.iter().enumerate() {
        let entry = &page.tables[table];
        if entry.count != count {
            return Err(invalid(format!("{name:?} block {}: its entry counts {} rows before it, its parts {count}", page.block, entry.count)));
        }
        let mut cursor = LogCursor::new(kinds, &all, entry.parts.first().map_or(0, |part| part.first));
        let mut next_part = 0;
        let block_first = run_first + (page.block * BLOCK) as i32;
        for hour in 0..page.hours {
            let (mut rows, mut changes, mut disorder) = (None::<TableState>, HourChanges::default(), false);
            loop {
                let mut seen = |event: LogEvent<'_>| match event {
                    LogEvent::Appended { ordinal, rows: held, at, .. } => {
                        rows.get_or_insert_with(|| held.clone());
                        changes.inserted.push(at);
                        disorder |= ordinal != count;
                        count += 1;
                        listed.append(ordinal);
                    }
                    LogEvent::Forgotten { from, to, .. } => {
                        listed.clear_range(from, to);
                        changes.forgotten.extend(from..to);
                    }
                    LogEvent::Named { .. } => {}
                };
                match cursor.advance(hour, &mut seen).map_err(|error| invalid(format!("{name:?} block {} hour {hour}: {error}", page.block)))? {
                    Advanced::Reached => break,
                    Advanced::Missing { .. } => {
                        let Some(part) = entry.parts.get(next_part) else { break };
                        let last = entry.parts.get(next_part + 1).map_or(page.hours - 1, |next| next.first - 1);
                        let fetched = std::time::Instant::now();
                        let part = fetch(file, part, last, &all, cursor.prefix().as_ref())?;
                        checked.fetch += fetched.elapsed();
                        for (id, text) in part.strings() {
                            texts.own.insert(*id, text.clone());
                        }
                        cursor.push(part);
                        next_part += 1;
                    }
                }
            }
            if disorder {
                return Err(invalid(format!("{name:?} block {} hour {hour}: rows appended out of their order", page.block)));
            }
            let (status, _, sampled, _) = unit::reader::status(entry, hour, block_first)?;
            let hour_rows = rows.unwrap_or_else(|| TableState::new(kinds));
            let shown = TextRows { table: &hour_rows, kinds, texts: &texts };
            let text = |id: u32| texts.get(id);
            digest.hour(block_first + i32::from(hour), (status, sampled), &shown, &changes, &text, &mut scratch);
        }
        let text = |id: u32| texts.get(id);
        let status = digest.status.unwrap_or((Status::NotRecorded, None));
        let (chain, state, live) = digest.log_end(count, &listed, &text);
        let restored = unit::reader::log_state(file, pages, kinds, table, index)?;
        checked.restores += 1;
        if restored.count() != Some(count) || restored.listed() != Some(&listed) || (restored.status(), restored.sampled_at()) != status {
            checked.differing += 1;
            eprintln!("{}", serde_json::json!({ "restore_differs": format!("{name:?}"), "block": first_block + page.block }));
        }
        lines.push(format!("{run} {} {name:?} {chain} {state} {live}", first_block + page.block));
    }
    Ok((lines, checked, Vec::new()))
}

struct TextRows<'a> {
    table: &'a TableState,
    kinds: &'a [Kind],
    texts: &'a Texts<'a>,
}

impl Rows for TextRows<'_> {
    fn columns(&self) -> usize {
        self.kinds.len()
    }

    fn key(&self, slot: u32) -> Option<u128> {
        self.table.key(slot)
    }

    fn live(&self) -> Vec<u32> {
        self.table.rows().collect()
    }

    fn value(&self, column: usize, slot: u32) -> Value<'_> {
        match self.table.value(column, slot) {
            Held::Unrecorded => Value::Unrecorded,
            Held::Missing => Value::Missing,
            Held::Number(bits) => {
                let signed = matches!(self.kinds[column], Kind::I8 | Kind::I16 | Kind::I32 | Kind::I64 | Kind::OptionalI32 | Kind::OptionalI64);
                Value::Number(if signed { i128::from(bits as i64) } else { i128::from(bits) })
            }
            Held::Text(id) => Value::Text(self.texts.get(id), id),
            Held::List(bytes) => Value::Bytes(bytes),
        }
    }
}

fn recording_mode(args: &[String]) -> io::Result<()> {
    let [path, rest @ ..] = args else {
        return Err(invalid(USAGE));
    };
    let (mut threads, mut first_block, mut open) = (8_usize, 0_u32, replay_columnar::container::Open::Saved);
    let mut rows_out = None;
    let mut seek = false;
    for option in rest {
        if option == "--seek" {
            seek = true;
        } else if let Some(given) = option.strip_prefix("--rows=") {
            rows_out = Some(io::BufWriter::new(std::fs::File::create(given)?));
        } else if let Some(given) = option.strip_prefix("--threads=") {
            threads = given.parse().map_err(|_| invalid(USAGE))?;
        } else if let Some(given) = option.strip_prefix("--first-block=") {
            first_block = given.parse().map_err(|_| invalid(USAGE))?;
        } else if option == "--newest" {
            open = replay_columnar::container::Open::Newest;
        } else {
            return Err(invalid(USAGE));
        }
    }
    let kinds = schema()?;
    let started = std::time::Instant::now();
    let log = Recording::open_path_as(path, open)?;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().map_err(io::Error::other)?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut whole = blake3::Hasher::new();
    let (mut lines, mut checked, mut differing, mut tracks) = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut seeks = Seeks::default();
    let mut fetch_time = std::time::Duration::ZERO;
    for run in 0..log.run_count() {
        let directory = log.run(run)?.directory().clone();
        let pages = log.unit_pages(run, 0, directory.blocks())?;
        let mut appended: HashMap<u32, Vec<u8>> = HashMap::new();
        for head in log.heads(run)? {
            appended.extend(log.strings(&head)?);
        }
        eprintln!("{}", serde_json::json!({ "run": run, "blocks": pages.len(), "strings": appended.len(), "seconds": started.elapsed().as_secs_f64() }));
        let file = log.extents();
        let dump = |table: usize| rows_out.is_some() && EPOCHED.contains(&TableId::ALL[table]);
        let results: Vec<io::Result<(Vec<String>, Checked, Vec<String>)>> = pool.install(|| {
            (0..kinds.len())
                .into_par_iter()
                .map(|table| {
                    let read = match unit::reader::class_of(&pages, table) {
                        Ok(true) => read_log(file, &pages, &kinds[table], table, run, directory.first_hour, first_block, &appended),
                        Ok(false) => read_table(file, &pages, &kinds[table], table, run, directory.first_hour, first_block, &appended, dump(table), seek),
                        Err(error) => Err(error),
                    };
                    read.map_err(|error| invalid(format!("{:?}: {error}", TableId::ALL[table])))
                })
                .collect()
        });
        let mut by_block: Vec<Vec<String>> = vec![Vec::new(); pages.len()];
        for result in results {
            let (table_lines, table_checked, dumped) = result?;
            if let Some(rows_out) = rows_out.as_mut() {
                for line in dumped {
                    writeln!(rows_out, "{line}")?;
                }
            }
            for (block, line) in table_lines.into_iter().enumerate() {
                by_block[block].push(line);
            }
            checked += table_checked.restores;
            differing += table_checked.differing;
            tracks += table_checked.tracks;
            fetch_time += table_checked.fetch;
            seeks += table_checked.seek;
        }
        for block in by_block {
            for line in block {
                whole.update(line.as_bytes());
                whole.update(b"\n");
                writeln!(out, "{line}")?;
                lines += 1;
            }
        }
    }
    writeln!(out, "lines {lines} digest {}", hex(&whole.finalize().as_bytes()[..16]))?;
    out.flush()?;
    if let Some(mut rows_out) = rows_out {
        rows_out.flush()?;
    }
    eprintln!(
        "{}",
        serde_json::json!({ "read_back": true, "seconds": started.elapsed().as_secs_f64(), "restores_checked": checked, "restores_differing": differing,
            "tracks_opened": tracks, "fetch_ms": fetch_time.as_secs_f64() * 1e3,
            "seek": seek.then(|| serde_json::json!({ "restores": seeks.restores, "block_ends": seeks.block_ends, "cells": seeks.cells,
                "texts_missing": seeks.texts_missing, "differing": seeks.differing })) })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_columnar::{
        state::Cell,
        store::Store,
        unit::{Encoder, PartSink, Settings},
    };

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
            out.extend_from_slice(&self.bytes[offset as usize..][..stored as usize]);
            Ok(())
        }
    }

    #[test]
    fn a_log_quiet_in_its_run_s_first_hour_is_walked_from_its_first_part() {
        let kinds = schema().unwrap();
        let table = TableId::NuclearStrikes.index();
        let mut logs = vec![false; kinds.len()];
        logs[table] = true;
        let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
        store.declare_logs(&logs).unwrap();
        store.attach(Encoder::new(&kinds, Settings::at(3)).unwrap());
        let unrecorded: Vec<Option<Cell<'_>>> = vec![None; kinds[table].len()];
        let mut memory = Memory::default();
        store.begin_run(0);
        for hour in 0..4 {
            store.begin_hour(hour, &[]).unwrap();
            store.status(table, Status::Complete, true, Some(hour), true).unwrap();
            match hour {
                1 => {
                    let mut changes = store.table(table).unwrap();
                    changes.append(7, &unrecorded).unwrap();
                    changes.append(8, &unrecorded).unwrap();
                }
                2 => store.table(table).unwrap().forget(0).unwrap(),
                3 => store.table(table).unwrap().forget(1).unwrap(),
                _ => {}
            }
            store.end_hour(&mut memory).unwrap();
        }
        store.end_run(&mut memory).unwrap();
        assert_eq!(memory.pages[0].tables[table].parts[0].first, 1, "the first part starts after the quiet first hour");
        let (lines, checked, _) = read_log(&memory.bytes, &memory.pages, &kinds[table], table, 0, 0, 0, &HashMap::new()).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!((checked.restores, checked.differing), (1, 0), "the block's restore holds what the log's hours left");
    }
}