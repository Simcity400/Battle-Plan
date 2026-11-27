
pub(super) mod bulk;
mod logs;
mod numbers;

pub use logs::{LogCursor, LogEvent, class_of, log_rows, log_rows_at, log_span, log_state, log_state_named};
#[cfg(test)]
mod tests;

use super::{
    BLOCK, FrameEntry, LIFETIMES, MISSING, PartEntry, RAW, READ, RUNS, SECOND, STRINGS, TOUCHED, TableEntry, UNRECORDED, column_stream, get_status,
    get_code, get_varint, get_wide, lists, unzig, unzig_wide,
};
use crate::{
    invalid,
    manifest::Status,
    state::Kind,
    store::{Lists, Strings, TableState, Value},
};
use engine_allocation::zstd::{Decompressor, Holder};
use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

static READER_ZSTD: Holder = Holder::new("unit reader");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeCounts {
    pub log_span_rows: u64,
    pub log_rows_at_rows: u64,
    pub column_cells: u64,
    pub status_blocks: u64,
}

thread_local! {
    static DECODE_COUNTS: RefCell<Option<DecodeCounts>> = const { RefCell::new(None) };
}

pub fn counting_decodes() -> bool {
    DECODE_COUNTS.with(|cell| cell.borrow().is_some())
}

pub fn count_decodes<T>(run: impl FnOnce() -> T) -> (T, DecodeCounts) {
    let previous = DECODE_COUNTS.with(|cell| cell.borrow_mut().replace(DecodeCounts::default()));
    struct Reset(Option<DecodeCounts>);
    impl Drop for Reset {
        fn drop(&mut self) {
            DECODE_COUNTS.with(|cell| {
                *cell.borrow_mut() = self.0;
            });
        }
    }
    let _reset = Reset(previous);
    let out = run();
    let counts = DECODE_COUNTS.with(|cell| cell.borrow().unwrap_or_default());
    (out, counts)
}

pub(super) fn record_log_span_row(columns: usize) {
    DECODE_COUNTS.with(|cell| {
        if let Some(counts) = cell.borrow_mut().as_mut() {
            counts.log_span_rows += 1;
            counts.column_cells += columns as u64;
        }
    });
}

pub(super) fn record_log_rows_at(rows: usize, columns: usize) {
    DECODE_COUNTS.with(|cell| {
        if let Some(counts) = cell.borrow_mut().as_mut() {
            counts.log_rows_at_rows += rows as u64;
            counts.column_cells += (rows * columns) as u64;
        }
    });
}

fn record_status_block() {
    DECODE_COUNTS.with(|cell| {
        if let Some(counts) = cell.borrow_mut().as_mut() {
            counts.status_blocks += 1;
        }
    });
}

const PAD: usize = 16;

pub const SPARE_BYTES: usize = 32 << 20;

static SPARE_CAP: AtomicUsize = AtomicUsize::new(SPARE_BYTES);

static SPARE: Mutex<Spare> = Mutex::new(Spare { frames: Vec::new(), bytes: 0 });

struct Spare {
    frames: Vec<Vec<u8>>,
    bytes: usize,
}

impl Spare {
    fn take(&mut self, wanted: usize) -> Option<Vec<u8>> {
        let fits = self.frames.iter().enumerate().filter(|(_, held)| held.capacity() >= wanted).min_by_key(|(_, held)| held.capacity()).map(|(at, _)| at);
        let at = fits.or_else(|| self.frames.iter().enumerate().max_by_key(|(_, held)| held.capacity()).map(|(at, _)| at))?;
        let buffer = self.frames.swap_remove(at);
        self.bytes -= buffer.capacity();
        Some(buffer)
    }
}

fn spare() -> std::sync::MutexGuard<'static, Spare> {
    SPARE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn spare_cap(bytes: usize) {
    SPARE_CAP.store(bytes, Ordering::Relaxed);
    let mut held = spare();
    while held.bytes > bytes {
        let Some(frame) = held.frames.pop() else { break };
        held.bytes -= frame.capacity();
    }
}

pub fn release_spare() -> usize {
    let mut held = spare();
    let bytes = held.bytes;
    *held = Spare { frames: Vec::new(), bytes: 0 };
    bytes
}

pub fn spare_bytes() -> usize {
    spare().bytes
}

pub trait Extents: Sync {
    fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()>;
}

impl Extents for std::fs::File {
    fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.resize(length, 0);
        let mut done = 0;
        while done < length {
            #[cfg(windows)]
            let read = std::os::windows::fs::FileExt::seek_read(self, &mut out[done..], offset + done as u64)?;
            #[cfg(unix)]
            let read = std::os::unix::fs::FileExt::read_at(self, &mut out[done..], offset + done as u64)?;
            if read == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "A unit frame passes its file"));
            }
            done += read;
        }
        Ok(())
    }
}

impl Extents for Vec<u8> {
    fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| invalid("A unit frame passes its bytes"))?;
        let bytes = self.get(start..start + length).ok_or_else(|| invalid("A unit frame passes its bytes"))?;
        out.clear();
        out.extend_from_slice(bytes);
        Ok(())
    }
}

const OLDER: u16 = u16::MAX;
const NEVER: u16 = u16::MAX;
const SKIP: u32 = u32::MAX;

const RANGE: usize = 64;

const GROUP: usize = 64;

struct Reading {
    decompressor: Option<Decompressor>,
    stored: Vec<u8>,
}

impl Reading {
    const fn new() -> Self {
        Self { decompressor: None, stored: Vec::new() }
    }

    fn decompressor(&mut self) -> io::Result<&mut Decompressor> {
        if self.decompressor.is_none() {
            self.decompressor = Some(Decompressor::new(&READER_ZSTD)?);
        }
        Ok(self.decompressor.as_mut().expect("made"))
    }

    fn buffer(&mut self, raw: usize) -> Vec<u8> {
        let mut buffer = spare().take(raw + PAD).unwrap_or_default();
        buffer.clear();
        buffer.reserve(raw + PAD);
        buffer
    }
}

thread_local! {
    static READING: RefCell<Reading> = const { RefCell::new(Reading::new()) };
}

fn reading<T>(work: impl FnOnce(&mut Reading) -> io::Result<T>) -> io::Result<T> {
    let mut work = Some(work);
    let held = READING.try_with(|held| held.try_borrow_mut().ok().map(|mut held| (work.take().expect("once"))(&mut held)));
    match held {
        Ok(Some(result)) => result,
        _ => (work.take().expect("once"))(&mut Reading::new()),
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Span {
    frame: u32,
    start: u32,
    end: u32,
}

#[derive(Debug, Default)]
struct Frames {
    frames: Vec<Vec<u8>>,
    streams: Vec<(u32, Span)>,
}

impl Frames {
    fn span(&self, id: u32) -> Option<Span> {
        self.streams.binary_search_by_key(&id, |&(held, _)| held).ok().map(|at| self.streams[at].1)
    }

    fn stream(&self, id: u32) -> Option<&[u8]> {
        self.span(id).map(|span| &self.frames[span.frame as usize][span.start as usize..span.end as usize])
    }

    fn upto(&self, span: Span) -> &[u8] {
        &self.frames[span.frame as usize][..span.end as usize]
    }

    fn padded(&self, span: Span) -> &[u8] {
        &self.frames[span.frame as usize]
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        if self.frames.is_empty() {
            return;
        }
        let cap = SPARE_CAP.load(Ordering::Relaxed);
        let mut held = spare();
        for frame in self.frames.drain(..) {
            if held.bytes + frame.capacity() <= cap {
                held.bytes += frame.capacity();
                held.frames.push(frame);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Prefix(Arc<Frames>);

#[derive(Debug, Default)]
struct Tracks {
    slot: Vec<u32>,
    at: Vec<u32>,
    end: Vec<u32>,
    next: Vec<u16>,
    born: Vec<u16>,
    last: Vec<u64>,
    gap: Vec<u16>,
    left: Vec<u32>,
    payload: Vec<u32>,
    due: u16,
    mins: Vec<u16>,
    max_slot: u32,
}

impl Tracks {
    fn len(&self) -> usize {
        self.slot.len()
    }

    fn push(&mut self, slot: u32, at: u32, end: u32, next: u16) {
        self.slot.push(slot);
        self.at.push(at);
        self.end.push(end);
        self.next.push(next);
        self.max_slot = self.max_slot.max(slot);
    }

    fn finish(&mut self, model: u8, first_gaps: &[u16]) {
        let count = self.len();
        match model {
            SECOND => self.last = vec![0; count],
            RUNS => {
                self.gap = first_gaps.to_vec();
                self.left = vec![0; count];
                self.payload = vec![0; count];
            }
            _ => {}
        }
        self.mins = self.next.chunks(GROUP).map(|group| group.iter().copied().min().unwrap_or(NEVER)).collect();
        self.due = self.mins.iter().copied().min().unwrap_or(NEVER);
    }

    fn unfinished(&self) -> bool {
        self.next.iter().any(|&next| next != NEVER)
    }
}

#[derive(Debug, Default)]
struct Opened {
    span: Span,
    model: u8,
    older: Tracks,
    born: Tracks,
    pool: Vec<(u32, u32)>,
    inserted: usize,
    inserted_end: usize,
}

impl Opened {
    fn track_count(&self) -> usize {
        self.older.len() + self.born.len()
    }

    fn order(&self) -> Vec<(bool, usize)> {
        let (mut older, mut born) = (0, 0);
        let mut order = Vec::with_capacity(self.track_count());
        while older < self.older.len() || born < self.born.len() {
            let take_older = born == self.born.len() || older < self.older.len() && self.older.slot[older] <= self.born.slot[born];
            if take_older {
                order.push((false, older));
                older += 1;
            } else {
                order.push((true, born));
                born += 1;
            }
        }
        order
    }

    fn group(&self, born: bool) -> &Tracks {
        if born { &self.born } else { &self.older }
    }
}

fn open_stream(frame: &[u8], span: Span, first: u16) -> io::Result<Opened> {
    let (start, end) = (span.start as usize, span.end as usize);
    let model = *frame.get(start).filter(|_| start < end).ok_or_else(|| invalid("An empty unit stream"))?;
    if model > SECOND {
        return Err(invalid(format!("Unit stream model {model}")));
    }
    let mut damaged = false;
    let mut at = start + 1;
    let count = numbers::varint(frame, &mut at, &mut damaged);
    let directory = numbers::varint(frame, &mut at, &mut damaged);
    let pool_bytes = numbers::varint(frame, &mut at, &mut damaged);
    let inserted_bytes = numbers::varint(frame, &mut at, &mut damaged);
    let pool_start = at as u64 + directory;
    let inserted = pool_start + pool_bytes;
    let events = inserted + inserted_bytes;
    if damaged || events > end as u64 || count > directory {
        return Err(invalid("A unit stream's parts pass its bytes"));
    }
    let (pool_start, inserted, events) = (pool_start as usize, inserted as usize, events as usize);
    let mut opened = Opened { span, model, inserted, inserted_end: events, ..Opened::default() };
    let tracks = count as usize;
    for held in [&mut opened.older.slot, &mut opened.older.at, &mut opened.older.end] {
        held.reserve_exact(tracks);
    }
    opened.older.next.reserve_exact(tracks);
    let (mut older_gaps, mut born_gaps) = (Vec::new(), Vec::new());
    if model == RUNS {
        older_gaps.reserve_exact(tracks);
    }
    let mut slot = 0_u32;
    let mut place = events;
    for _ in 0..count {
        let gap = numbers::varint(frame, &mut at, &mut damaged);
        let head = numbers::varint(frame, &mut at, &mut damaged);
        let born = if head & 1 == 1 { Some(numbers::varint(frame, &mut at, &mut damaged)) } else { None };
        let length = numbers::varint(frame, &mut at, &mut damaged);
        slot = u32::try_from(gap).ok().and_then(|gap| slot.checked_add(gap)).ok_or_else(|| invalid("A unit track's slot passes 32 bits"))?;
        if damaged || at > pool_start {
            return Err(invalid("A unit stream's directory disagrees with its length"));
        }
        let track_end = place as u64 + length;
        if length == 0 || track_end > end as u64 {
            return Err(invalid("A unit track passes its stream"));
        }
        let track_end = track_end as usize;
        let mut cursor = place;
        let hours = numbers::hours(frame, &mut cursor, &mut damaged);
        let origin = match born {
            Some(offset) if offset > u64::from(BLOCK) => return Err(invalid("A unit track is born past its block")),
            Some(offset) => u32::from(first) + offset as u32,
            None => u32::from(first),
        };
        if damaged || cursor > track_end || origin + hours > 2 * BLOCK {
            return Err(invalid("A unit track's first event passes its track"));
        }
        let next = (origin + hours) as u16;
        match born {
            Some(_) => {
                opened.born.push(slot, cursor as u32, track_end as u32, next);
                opened.born.born.push(origin as u16);
                if model == RUNS {
                    born_gaps.push(hours as u16);
                }
            }
            None => {
                opened.older.push(slot, cursor as u32, track_end as u32, next);
                if model == RUNS {
                    older_gaps.push(hours as u16);
                }
            }
        }
        place = track_end;
    }
    if at != pool_start {
        return Err(invalid("A unit stream's directory disagrees with its length"));
    }
    if place != end {
        return Err(invalid("A unit stream's tracks disagree with its length"));
    }
    opened.older.finish(model, &older_gaps);
    opened.born.finish(model, &born_gaps);
    let pool_bytes = &frame[..inserted];
    let mut cursor = pool_start;
    while cursor < inserted {
        let length = get_varint(pool_bytes, &mut cursor)?;
        if cursor as u64 + length > inserted as u64 {
            return Err(invalid("A pooled literal passes its pool"));
        }
        opened.pool.push((cursor as u32, length as u32));
        cursor += length as usize;
    }
    Ok(opened)
}

#[derive(Clone, Copy, Debug, Default)]
struct Life {
    at: usize,
    hour: u16,
    key: u128,
}

#[derive(Debug)]
pub struct Part {
    pub first: u16,
    pub last: u16,
    frames: Arc<Frames>,
    columns: Vec<Option<Opened>>,
    lifetimes: Option<Span>,
    strings: Vec<(u32, Vec<u8>)>,
    read: Option<Opened>,
    touched: Option<Opened>,
    pub stored: usize,
    pub decompressed: usize,
    pub reads: usize,
    pub open_ns: u64,
    pub read_ns: u64,
    pub verify_ns: u64,
    pub decompress_ns: u64,
    life: Life,
    lives: u64,
    log: bool,
}

impl Part {
    pub fn is_log(&self) -> bool {
        self.log
    }

    pub fn track_count(&self) -> usize {
        self.columns.iter().flatten().map(Opened::track_count).sum()
    }

    pub fn strings(&self) -> &[(u32, Vec<u8>)] {
        &self.strings
    }

    pub fn prefix(&self) -> Prefix {
        Prefix(self.frames.clone())
    }

    fn finished(&self) -> io::Result<()> {
        if self.columns.iter().flatten().any(|opened| opened.older.unfinished() || opened.born.unfinished()) {
            return Err(invalid("A unit track holds events past its part"));
        }
        if self.lifetimes.is_some_and(|span| self.life.at != span.end as usize) {
            return Err(invalid("A unit part's lifetimes hold hours past it"));
        }
        Ok(())
    }
}

pub fn fetch(source: &dyn Extents, entry: &PartEntry, last: u16, columns: &[usize], previous: Option<&Prefix>) -> io::Result<Part> {
    fetch_streams(source, entry, last, columns, previous, false)
}

fn fetch_streams(source: &dyn Extents, entry: &PartEntry, last: u16, columns: &[usize], previous: Option<&Prefix>, names_only: bool) -> io::Result<Part> {
    let mut wanted: Vec<u32> = columns.iter().map(|&column| column_stream(column)).collect();
    wanted.push(STRINGS);
    if !names_only { wanted.extend([LIFETIMES, READ, TOUCHED]); }
    wanted.sort_unstable();
    let wants = |id: u32| wanted.binary_search(&id).is_ok();
    let mut held = Frames::default();
    let (mut stored_total, mut decompressed, mut reads) = (0, 0, 0);
    let (mut read_ns, mut verify_ns, mut decompress_ns) = (0_u64, 0_u64, 0_u64);
    let frames: Vec<&FrameEntry> = entry.frames.iter().filter(|frame| frame.streams.iter().any(|&stream| wants(stream))).collect();
    reading(|reading| {
        let mut run = std::mem::take(&mut reading.stored);
        let (mut run_from, mut run_end) = (0_u64, 0_u64);
        for (at, frame) in frames.iter().enumerate() {
            if at == 0 || frame.offset < run_from || frame.offset + u64::from(frame.stored) > run_end {
                let mut end = frame.offset + u64::from(frame.stored);
                for next in &frames[at + 1..] {
                    if next.offset != end {
                        break;
                    }
                    end += u64::from(next.stored);
                }
                let length = usize::try_from(end - frame.offset).map_err(|_| invalid("A unit part's frames pass the address space"))?;
                let started = std::time::Instant::now();
                reads += 1;
                let read = source.read_at(frame.offset, length, &mut run);
                read_ns += started.elapsed().as_nanos() as u64;
                if let Err(error) = read {
                    reading.stored = run;
                    return Err(error);
                }
                (run_from, run_end) = (frame.offset, end);
            }
            let from = (frame.offset - run_from) as usize;
            let stored = &run[from..from + frame.stored as usize];
            let started = std::time::Instant::now();
            let read = verify(frame, stored);
            verify_ns += started.elapsed().as_nanos() as u64;
            let mut out = reading.buffer(frame.raw as usize);
            let started = std::time::Instant::now();
            let result = read.and_then(|()| {
                let zstd = |error: engine_allocation::zstd::Error| invalid(format!("A unit frame does not decompress: {}", error.name()));
                let decompressor = reading.decompressor()?;
                if frame.chained {
                    let stream = frame.streams[0];
                    let prefix = previous
                        .and_then(|prefix| prefix.0.stream(stream))
                        .ok_or_else(|| invalid("A chained unit part is read without the part before it"))?;
                    decompressor.decompress_append_prefixed(prefix, stored, &mut out).map_err(zstd)?;
                } else {
                    decompressor.decompress_append(stored, &mut out).map_err(zstd)?;
                }
                if out.len() != frame.raw as usize {
                    return Err(invalid("A unit frame decompresses to another length"));
                }
                Ok(())
            });
            decompress_ns += started.elapsed().as_nanos() as u64;
            stored_total += stored.len();
            if let Err(error) = result {
                reading.stored = run;
                return Err(error);
            }
            decompressed += out.len();
            let index = held.frames.len() as u32;
            let raw = out.len();
            if !frame.small {
                held.streams.push((frame.streams[0], Span { frame: index, start: 0, end: raw as u32 }));
            } else {
                let mut at = 0;
                let count = get_varint(&out, &mut at)? as usize;
                let mut heads = Vec::with_capacity(count.min(raw));
                for _ in 0..count {
                    let id = get_varint(&out, &mut at)? as u32;
                    let length = get_varint(&out, &mut at)?;
                    heads.push((id, length));
                }
                for (id, length) in heads {
                    let end = at as u64 + length;
                    if end > raw as u64 {
                        return Err(invalid("A small stream passes its frame"));
                    }
                    if wants(id) {
                        held.streams.push((id, Span { frame: index, start: at as u32, end: end as u32 }));
                    }
                    at = end as usize;
                }
            }
            out.resize(raw + PAD, 0);
            held.frames.push(out);
        }
        reading.stored = run;
        Ok(())
    })?;
    held.streams.sort_unstable_by_key(|&(id, _)| id);
    if held.streams.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("A unit part holds a stream twice"));
    }
    let first = entry.first;
    let opening = std::time::Instant::now();
    let open = |id: u32| held.span(id).map(|span| open_stream(held.padded(span), span, first)).transpose();
    let mut opened = Vec::with_capacity(columns.len());
    for &column in columns {
        opened.push(open(column_stream(column))?);
    }
    let read = open(READ)?;
    let touched = open(TOUCHED)?;
    let mut strings = Vec::new();
    if let Some(bytes) = held.stream(STRINGS) {
        let mut at = 0;
        let mut id = 0_u32;
        for _ in 0..get_varint(bytes, &mut at)? {
            id = id.wrapping_add(get_varint(bytes, &mut at)? as u32);
            let length = get_varint(bytes, &mut at)? as usize;
            strings.push((id, bytes.get(at..at + length).ok_or_else(|| invalid("A string passes its stream"))?.to_vec()));
            at += length;
        }
    }
    let lifetimes = held.span(LIFETIMES);
    let open_ns = opening.elapsed().as_nanos() as u64;
    Ok(Part {
        first,
        last,
        frames: Arc::new(held),
        columns: opened,
        lifetimes,
        strings,
        read,
        touched,
        stored: stored_total,
        decompressed,
        reads,
        open_ns,
        read_ns,
        verify_ns,
        decompress_ns,
        life: Life { at: lifetimes.map_or(0, |span| span.start as usize), hour: first, key: 0 },
        lives: 0,
        log: entry.log,
    })
}

pub fn part_text(source: &dyn Extents, parts: &[PartEntry], hour: u16, id: u32) -> io::Result<Option<Vec<u8>>> {
    let mut previous = None;
    for entry in parts.iter().take_while(|entry| entry.first <= hour) {
        let part = fetch_streams(source, entry, hour, &[], previous.as_ref(), true)?;
        if let Some((_, text)) = part.strings().iter().find(|(held, _)| *held == id) { return Ok(Some(text.clone())); }
        previous = Some(part.prefix());
    }
    Ok(None)
}

fn verify(frame: &FrameEntry, stored: &[u8]) -> io::Result<()> {
    if blake3::hash(stored).as_bytes()[..16] != frame.digest {
        return Err(invalid("A unit frame differs from its digest"));
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lives {
    pub deleted: Vec<(u16, u32, u128)>,
    pub inserted: Vec<(u16, u32, u128)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advanced {
    Reached,
    Missing { last: Option<u16> },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spent {
    pub numeric_events: u64,
    pub numeric: std::time::Duration,
    pub numeric_scanned: u64,
    pub numeric_visited: u64,
    pub list_events: u64,
    pub list: std::time::Duration,
    pub lives: u64,
    pub lifetimes: std::time::Duration,
    pub bytes: u64,
}

pub struct Cursor {
    columns: Vec<usize>,
    list: Vec<bool>,
    parts: std::collections::VecDeque<Part>,
    passed: Option<Prefix>,
    hour: u16,
    born: Vec<u16>,
    born_slots: Vec<u32>,
    texts: HashMap<u32, Vec<u8>>,
    slots: Vec<u32>,
    cell: Vec<u8>,
    scratch: Vec<u8>,
    forms: [u64; 8],
    pub spent: Spent,
    timed: bool,
    lap: Option<std::time::Instant>,
    passing: Vec<bool>,
    open: HashMap<u32, usize, foldhash::fast::RandomState>,
    skipped: HashMap<u32, u128, foldhash::fast::RandomState>,
    targets: Vec<u32>,
}

pub type Gone<'a> = &'a mut dyn FnMut(u16, u32, u128, &TableState);

type Seeing<'a, 'b> = &'a mut (dyn FnMut(u16, u32, u128, &TableState) + 'b);

impl Cursor {
    pub fn new(kinds: &[Kind], columns: &[usize], hour: u16) -> Self {
        let kind = |column: usize| kinds.get(column).copied().unwrap_or(Kind::U8);
        Self {
            list: columns.iter().map(|&column| matches!(kind(column), Kind::U32List | Kind::U64List | Kind::Opaque)).collect(),
            columns: columns.to_vec(),
            parts: std::collections::VecDeque::new(),
            passed: None,
            hour,
            born: Vec::new(),
            born_slots: Vec::new(),
            texts: HashMap::new(),
            slots: Vec::new(),
            cell: Vec::new(),
            scratch: Vec::new(),
            forms: [0; 8],
            spent: Spent::default(),
            timed: false,
            lap: None,
            passing: Vec::new(),
            open: HashMap::default(),
            skipped: HashMap::default(),
            targets: Vec::new(),
        }
    }

    pub fn reset(&mut self, hour: u16) {
        self.parts.clear();
        self.passed = None;
        self.hour = hour;
        for &slot in &self.born_slots {
            self.born[slot as usize] = OLDER;
        }
        self.born_slots.clear();
        self.texts.clear();
        self.skipped.clear();
    }

    pub fn time(&mut self, timed: bool) {
        self.timed = timed;
    }

    pub fn push(&mut self, part: Part) {
        for (id, text) in &part.strings {
            self.texts.insert(*id, text.clone());
        }
        self.passed = None;
        self.parts.push_back(part);
    }

    pub fn last_part(&self) -> Option<&Part> {
        self.parts.back()
    }

    pub fn prefix(&self) -> Option<Prefix> {
        self.parts.back().map(Part::prefix).or_else(|| self.passed.clone())
    }

    pub fn text(&self, id: u32) -> Option<&[u8]> {
        self.texts.get(&id).map(Vec::as_slice)
    }

    pub fn hour(&self) -> u16 {
        self.hour
    }


    pub fn list_forms(&self) -> [u64; 8] {
        self.forms
    }

    pub fn advance(&mut self, to: u16, step: u16, cells: &mut TableState, written: &mut [Vec<u32>], lives: &mut Lives) -> io::Result<Advanced> {
        self.advance_by(to, step, cells, written, lives, None)
    }

    pub fn advance_seeing(&mut self, to: u16, step: u16, cells: &mut TableState, written: &mut [Vec<u32>], lives: &mut Lives, gone: Gone<'_>) -> io::Result<Advanced> {
        self.advance_by(to, step, cells, written, lives, Some(gone))
    }

    fn advance_by(&mut self, to: u16, step: u16, cells: &mut TableState, written: &mut [Vec<u32>], lives: &mut Lives, mut gone: Option<Seeing<'_, '_>>) -> io::Result<Advanced> {
        if self.parts.iter().any(|part| part.log) {
            return Err(invalid("A log's part was given to a state table's cursor: a log has rows by hour, not a state, and is read with a LogCursor"));
        }
        while self.hour <= to {
            let Some(part) = self.parts.front() else {
                return Ok(Advanced::Missing { last: self.hour.checked_sub(1) });
            };
            if self.hour > part.last {
                part.finished()?;
                let part = self.parts.pop_front().expect("a part under way");
                self.passed = (self.parts.is_empty() && u32::from(part.last) + 1 < BLOCK).then(|| part.prefix());
                for &slot in &self.born_slots {
                    self.born[slot as usize] = OLDER;
                }
                self.born_slots.clear();
                continue;
            }
            if self.hour < part.first {
                return Err(invalid("A unit part starts after the hour its cursor is at"));
            }
            let end = to.min(part.last).min(self.hour.saturating_add(step.max(1) - 1));
            self.step(self.hour, end, cells, written, lives, gone.as_deref_mut())?;
            self.hour = end + 1;
        }
        Ok(Advanced::Reached)
    }

    fn step(&mut self, from: u16, to: u16, cells: &mut TableState, written: &mut [Vec<u32>], lives: &mut Lives, gone: Option<Seeing<'_, '_>>) -> io::Result<()> {
        self.lap = self.timed.then(std::time::Instant::now);
        self.tracks(from, to, false, cells, written)?;
        if self.lifetimes(from, to, cells, lives, gone)? {
            self.spent.lifetimes += lap(&mut self.lap);
        }
        self.tracks(from, to, true, cells, written)
    }

    fn lifetimes(&mut self, from: u16, to: u16, cells: &mut TableState, lives: &mut Lives, mut gone: Option<Seeing<'_, '_>>) -> io::Result<bool> {
        let part = self.parts.front_mut().expect("a part under way");
        let Some(span) = part.lifetimes else { return Ok(false) };
        let frames = part.frames.clone();
        let raw = frames.upto(span);
        let mut life = part.life;
        self.passing.clear();
        if gone.is_none() {
            passing(raw, life, to, &mut self.passing, &mut self.open, &mut self.slots)?;
        }
        let mut inserts = 0;
        while life.at < raw.len() {
            let mut at = life.at;
            let hour = u32::from(life.hour) + get_varint(raw, &mut at)?.min(u64::from(BLOCK) + 1) as u32;
            if hour > u32::from(to) {
                break;
            }
            let hour = hour as u16;
            let deleted = get_varint(raw, &mut at)? as usize;
            let inserted = get_varint(raw, &mut at)? as usize;
            if deleted.saturating_add(inserted) > raw.len() {
                return Err(invalid("A lifetime counts more rows than its stream holds"));
            }
            self.slots.clear();
            for count in [deleted, inserted] {
                let mut previous = 0_i64;
                for _ in 0..count {
                    previous = previous.wrapping_add(unzig(get_varint(raw, &mut at)?));
                    self.slots.push(u32::try_from(previous).map_err(|_| invalid("A lifetime names a slot outside 32 bits"))?);
                }
            }
            let mut first = 0;
            while first < deleted {
                let mut end = first + 1;
                while end < deleted && self.slots[end] == self.slots[end - 1] + 1 {
                    end += 1;
                }
                let (low, high) = (self.slots[first], self.slots[end - 1] + 1);
                if gone.is_none() && end - first >= RANGE && (self.skipped.is_empty() || (low..high).all(|slot| !self.skipped.contains_key(&slot))) {
                    cells.delete_rows(low, high, |slot, key| lives.deleted.push((hour, slot, key)))?;
                    first = end;
                    continue;
                }
                for &slot in &self.slots[first..end] {
                if let Some(key) = self.skipped.remove(&slot) {
                    lives.deleted.push((hour, slot, key));
                    continue;
                }
                let key = cells.key(slot).ok_or_else(|| invalid("A lifetime deletes a slot holding no row"))?;
                if let Some(gone) = gone.as_deref_mut() {
                    let born = self.born.get(slot as usize).copied().unwrap_or(OLDER);
                    if born != OLDER && born >= from {
                        finish_born(part, &frames, &self.columns, &self.list, slot, born, cells, &mut self.cell, &mut self.scratch, &mut self.forms)?;
                    }
                    gone(hour, slot, key, cells);
                }
                cells.delete_row(slot)?;
                lives.deleted.push((hour, slot, key));
                }
                first = end;
            }
            self.targets.clear();
            for &slot in &self.slots[deleted..] {
                life.key = life.key.wrapping_add(unzig_wide(get_wide(raw, &mut at)?) as u128);
                let skip = self.passing.get(inserts).copied().unwrap_or(false) && slot < cells.slots() && !cells.live().get(slot);
                inserts += 1;
                if skip {
                    self.skipped.insert(slot, life.key);
                } else {
                    cells.insert_row(slot, life.key)?;
                }
                if self.born.len() <= slot as usize {
                    self.born.resize(slot as usize + 1, OLDER);
                }
                if self.born[slot as usize] == OLDER {
                    self.born_slots.push(slot);
                }
                self.born[slot as usize] = hour;
                lives.inserted.push((hour, slot, life.key));
                self.targets.push(if skip { SKIP } else { slot });
            }
            for (selected, &column) in self.columns.iter().enumerate() {
                let Some(opened) = part.columns[selected].as_mut() else {
                    for &slot in self.targets.iter().filter(|&&slot| slot != SKIP) {
                        cells.unrecord_cell(column, slot)?;
                    }
                    continue;
                };
                if !self.list[selected] {
                    let held = cells.numbers(column).ok_or_else(|| invalid("A numeric stream for a column that holds no numbers"))?;
                    numbers::inserted(frames.padded(opened.span), &mut opened.inserted, opened.inserted_end, held, &self.targets)?;
                    continue;
                }
                let raw = frames.upto(opened.span);
                for &slot in &self.targets {
                    inserted_list(opened, raw, cells, column, slot)?;
                }
            }
            self.spent.bytes += (at - life.at) as u64;
            life.at = at;
            life.hour = hour;
            part.lives += (deleted + inserted) as u64;
            self.spent.lives += (deleted + inserted) as u64;
        }
        if !self.skipped.is_empty() {
            return Err(invalid("A row the step deletes again outlives it"));
        }
        let applied = life.at != part.life.at;
        part.life = life;
        Ok(applied)
    }

    fn tracks(&mut self, from: u16, to: u16, inserted: bool, cells: &mut TableState, written: &mut [Vec<u32>]) -> io::Result<()> {
        let part = self.parts.front_mut().expect("a part under way");
        let frames = &*part.frames;
        for lists in [false, true] {
            let mut count = numbers::Count::default();
            let mut list_events = 0;
            for (selected, &column) in self.columns.iter().enumerate() {
                if self.list[selected] != lists {
                    continue;
                }
                let Some(opened) = part.columns[selected].as_mut() else { continue };
                if opened.older.due > to && opened.born.due > to {
                    continue;
                }
                for born in [false, true] {
                    if !born && inserted {
                        continue;
                    }
                    if lists {
                        let mut held = cells.lists(column).ok_or_else(|| invalid("A list stream for a column that holds no lists"))?;
                        list_events += advance_lists(opened, born, frames, from, to, inserted, &mut held, &self.born, &mut written[selected], &mut self.cell, &mut self.scratch, &mut self.forms, &mut count.bytes)?;
                        continue;
                    }
                    let pass = numbers::Pass { bytes: frames.padded(opened.span), from, to, inserted, born: &self.born };
                    let model = opened.model;
                    let tracks = if born { &mut opened.born } else { &mut opened.older };
                    let held = cells.numbers(column).ok_or_else(|| invalid("A numeric stream for a column that holds no numbers"))?;
                    numbers::advance(model, tracks, born, &pass, held, &mut written[selected], &mut count)?;
                    if count.damaged {
                        return Err(invalid(format!("A unit stream of column {column} is damaged: an event runs past its track")));
                    }
                }
            }
            self.spent.bytes += count.bytes;
            if lists {
                if list_events > 0 {
                    self.spent.list += lap(&mut self.lap);
                }
                self.spent.list_events += list_events;
            } else {
                if count.scanned > 0 {
                    self.spent.numeric += lap(&mut self.lap);
                }
                self.spent.numeric_events += count.events;
                self.spent.numeric_scanned += count.scanned;
                self.spent.numeric_visited += count.visited;
            }
        }
        Ok(())
    }

    pub fn marks(&self, from: u16, to: u16, touched: &mut Vec<(u16, u32)>, read: &mut Vec<(u16, u32)>) -> io::Result<()> {
        match self.parts.front() {
            Some(part) => part.marks(from, to, touched, read),
            None => Ok(()),
        }
    }
}

impl Part {
    pub fn marks(&self, from: u16, to: u16, touched: &mut Vec<(u16, u32)>, read: &mut Vec<(u16, u32)>) -> io::Result<()> {
        self.marked(self.touched.as_ref(), from, to, touched)?;
        self.marked(self.read.as_ref(), from, to, read)
    }

    pub fn reads(&self, from: u16, to: u16, read: &mut Vec<(u16, u32)>) -> io::Result<()> {
        self.marked(self.read.as_ref(), from, to, read)
    }

    fn marked(&self, opened: Option<&Opened>, from: u16, to: u16, out: &mut Vec<(u16, u32)>) -> io::Result<()> {
        let Some(opened) = opened else { return Ok(()) };
        let raw = self.frames.upto(opened.span);
        for (born, index) in opened.order() {
            let tracks = opened.group(born);
            let mut hour = u32::from(tracks.next[index]);
            let mut at = tracks.at[index] as usize;
            let track = raw.get(..tracks.end[index] as usize).ok_or_else(|| invalid("A track passes its stream"))?;
            loop {
                if hour > u32::from(self.last) {
                    return Err(invalid("A unit track marks an hour past its part"));
                }
                if hour > u32::from(to) {
                    break;
                }
                if hour >= u32::from(from) {
                    out.push((hour as u16, tracks.slot[index]));
                }
                if at >= track.len() {
                    break;
                }
                hour += get_varint(track, &mut at)?.min(u64::from(BLOCK) + 1) as u32;
            }
        }
        Ok(())
    }
}

fn lap(last: &mut Option<std::time::Instant>) -> std::time::Duration {
    let Some(held) = *last else { return std::time::Duration::ZERO };
    let now = std::time::Instant::now();
    *last = Some(now);
    now - held
}

fn passing(raw: &[u8], life: Life, to: u16, passing: &mut Vec<bool>, open: &mut HashMap<u32, usize, foldhash::fast::RandomState>, slots: &mut Vec<u32>) -> io::Result<()> {
    passing.clear();
    open.clear();
    let (mut at, mut hour) = (life.at, u32::from(life.hour));
    while at < raw.len() {
        hour += get_varint(raw, &mut at)?.min(u64::from(BLOCK) + 1) as u32;
        if hour > u32::from(to) {
            break;
        }
        let deleted = get_varint(raw, &mut at)? as usize;
        let inserted = get_varint(raw, &mut at)? as usize;
        if deleted.saturating_add(inserted) > raw.len() {
            return Err(invalid("A lifetime counts more rows than its stream holds"));
        }
        slots.clear();
        for count in [deleted, inserted] {
            let mut previous = 0_i64;
            for _ in 0..count {
                previous = previous.wrapping_add(unzig(get_varint(raw, &mut at)?));
                slots.push(previous as u32);
            }
        }
        for slot in &slots[..deleted] {
            if let Some(index) = open.remove(slot) {
                passing[index] = true;
            }
        }
        for &slot in &slots[deleted..] {
            open.insert(slot, passing.len());
            passing.push(false);
            get_wide(raw, &mut at)?;
        }
    }
    Ok(())
}

fn finish_born(part: &mut Part, frames: &Frames, columns: &[usize], list: &[bool], slot: u32, born: u16, cells: &mut TableState, cell: &mut Vec<u8>, scratch: &mut Vec<u8>, forms: &mut [u64; 8]) -> io::Result<()> {
    for (selected, &column) in columns.iter().enumerate() {
        let Some(opened) = part.columns[selected].as_mut() else { continue };
        let tracks = &mut opened.born;
        let mut index = tracks.slot.partition_point(|&held| held < slot);
        while index < tracks.len() && tracks.slot[index] == slot && tracks.born[index] != born {
            index += 1;
        }
        if index >= tracks.len() || tracks.slot[index] != slot || tracks.next[index] == NEVER {
            continue;
        }
        let (at, end, hour) = (tracks.at[index] as usize, tracks.end[index] as usize, u32::from(tracks.next[index]));
        if list[selected] {
            let mut held = cells.lists(column).ok_or_else(|| invalid("A list stream for a column that holds no lists"))?;
            let raw = frames.upto(opened.span);
            let unrecorded = held.unrecorded(slot);
            let (_, now, unrecorded) = list_events(raw, &opened.pool, at, end, hour, u32::from(NEVER) - 1, &mut held, slot, unrecorded, cell, scratch, forms)?;
            match now {
                Now::Page => {}
                Now::Scratch => held.set(slot, cell)?,
                Now::Stream(start, length) => held.set(slot, &raw[start..start + length])?,
            }
            held.record(slot, !unrecorded);
        } else {
            let (value, state) = match cells.value(column, slot) {
                Value::Number(bits) => (bits, 0),
                Value::Text(id) => (u64::from(id), 0),
                Value::Missing => (0, 1),
                _ => (0, 2),
            };
            let (value, state) = numbers::track(opened.model, frames.padded(opened.span), at, end, value, state).ok_or_else(|| invalid(format!("A unit stream of column {column} is damaged: an event runs past its track")))?;
            match state {
                0 if cells.column(column).is_some_and(|held| held.kind() == Kind::Bytes) => cells.put_cell(column, slot, Value::Text(value as u32))?,
                0 => cells.put_cell(column, slot, Value::Number(value))?,
                1 => cells.put_cell(column, slot, Value::Missing)?,
                _ => cells.unrecord_cell(column, slot)?,
            }
        }
        let tracks = &mut part.columns[selected].as_mut().expect("opened").born;
        tracks.at[index] = tracks.end[index];
        tracks.next[index] = NEVER;
    }
    Ok(())
}

fn inserted_list(opened: &mut Opened, raw: &[u8], cells: &mut TableState, column: usize, slot: u32) -> io::Result<()> {
    let at = &mut opened.inserted;
    if *at >= opened.inserted_end {
        return Err(invalid("A part inserts more rows than its column holds values"));
    }
    let raw = &raw[..opened.inserted_end];
    let head = get_varint(raw, at)?;
    if slot == SKIP {
        if head & 2 == 0 {
            *at = at.checked_add((head >> 2) as usize).filter(|&end| end <= raw.len()).ok_or_else(|| invalid("An inserted literal passes its stream"))?;
        }
        return Ok(());
    }
    if head & 2 == 2 {
        let (start, length) = *opened.pool.get((head >> 2) as usize).ok_or_else(|| invalid("An inserted value names no pooled literal"))?;
        cells.put_cell(column, slot, Value::List(&raw[start as usize..(start + length) as usize]))?;
    } else {
        let length = (head >> 2) as usize;
        let cell = raw.get(*at..*at + length).ok_or_else(|| invalid("An inserted literal passes its stream"))?;
        *at += length;
        cells.put_cell(column, slot, Value::List(cell))?;
    }
    if head & 1 == 1 {
        cells.unrecord_cell(column, slot)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Now {
    Page,
    Scratch,
    Stream(usize, usize),
}

fn advance_lists(
    opened: &mut Opened,
    born_group: bool,
    frames: &Frames,
    from: u16,
    to: u16,
    inserted: bool,
    lists: &mut Lists<'_>,
    born_at: &[u16],
    written: &mut Vec<u32>,
    cell: &mut Vec<u8>,
    scratch: &mut Vec<u8>,
    forms: &mut [u64; 8],
    bytes: &mut u64,
) -> io::Result<u64> {
    let raw = frames.upto(opened.span);
    let Opened { older, born, pool, .. } = opened;
    let tracks = if born_group { born } else { older };
    if tracks.len() == 0 || tracks.due > to {
        return Ok(0);
    }
    let to = u32::from(to);
    let mut applied = 0;
    let mut earliest = NEVER;
    for index in 0..tracks.len() {
        let next = tracks.next[index];
        if u32::from(next) > to {
            earliest = earliest.min(next);
            continue;
        }
        let slot = tracks.slot[index];
        if born_group {
            let now = tracks.born[index] >= from;
            if now != inserted {
                earliest = earliest.min(next);
                continue;
            }
            if now && (!lists.live.get(slot) || born_at.get(slot as usize).copied() != Some(tracks.born[index])) {
                tracks.at[index] = tracks.end[index];
                tracks.next[index] = NEVER;
                continue;
            }
        }
        let (at, end) = (tracks.at[index] as usize, tracks.end[index] as usize);
        let unrecorded = lists.unrecorded(slot);
        let (at, hour, now, unrecorded, events) = {
            let mut events = 0;
            let (at, hour, now, unrecorded) = list_steps(raw, pool, at, end, u32::from(next), to, lists, slot, unrecorded, cell, scratch, forms, &mut events)?;
            (at, hour, now, unrecorded, events)
        };
        applied += events;
        match now {
            Now::Page => {}
            Now::Scratch => lists.set(slot, cell)?,
            Now::Stream(start, length) => lists.set(slot, &raw[start..start + length])?,
        }
        lists.record(slot, !unrecorded);
        written.push(slot);
        *bytes += u64::from(at as u32 - tracks.at[index]);
        tracks.at[index] = at as u32;
        tracks.next[index] = hour as u16;
        earliest = earliest.min(hour as u16);
    }
    tracks.due = earliest;
    Ok(applied)
}

fn list_events(raw: &[u8], pool: &[(u32, u32)], at: usize, end: usize, hour: u32, to: u32, lists: &mut Lists<'_>, slot: u32, unrecorded: bool, cell: &mut Vec<u8>, scratch: &mut Vec<u8>, forms: &mut [u64; 8]) -> io::Result<(usize, Now, bool)> {
    let mut events = 0;
    let (at, _, now, unrecorded) = list_steps(raw, pool, at, end, hour, to, lists, slot, unrecorded, cell, scratch, forms, &mut events)?;
    Ok((at, now, unrecorded))
}

fn list_steps(
    raw: &[u8],
    pool: &[(u32, u32)],
    mut at: usize,
    end: usize,
    mut hour: u32,
    to: u32,
    lists: &mut Lists<'_>,
    slot: u32,
    mut unrecorded: bool,
    cell: &mut Vec<u8>,
    scratch: &mut Vec<u8>,
    forms: &mut [u64; 8],
    events: &mut u64,
) -> io::Result<(usize, u32, Now, bool)> {
    let mut now = Now::Page;
    loop {
        let form = *raw.get(at).ok_or_else(|| invalid("A list track ends before its form"))?;
        at += 1;
        forms[usize::from(form) & 7] += 1;
        match form {
            lists::LITERAL => {
                let length = get_varint(raw, &mut at)? as usize;
                if at + length > raw.len() {
                    return Err(invalid("A literal passes its stream"));
                }
                now = Now::Stream(at, length);
                at += length;
                unrecorded = false;
            }
            lists::POOLED => {
                let (start, length) = *pool.get(get_varint(raw, &mut at)? as usize).ok_or_else(|| invalid("An event names no pooled literal"))?;
                now = Now::Stream(start as usize, length as usize);
                unrecorded = false;
            }
            lists::PATCH => {
                if let Now::Stream(start, length) = now {
                    cell.clear();
                    cell.extend_from_slice(&raw[start..start + length]);
                    now = Now::Scratch;
                }
                let target = match now {
                    Now::Page => lists.get_mut(slot),
                    _ => &mut cell[..],
                };
                at = lists::patch(raw, at, target).map_err(|error| failed(error, slot, hour, form))?;
                unrecorded = false;
            }
            lists::EDIT | lists::PIECES => {
                let old = match now {
                    Now::Page => lists.get(slot),
                    Now::Scratch => &cell[..],
                    Now::Stream(start, length) => &raw[start..start + length],
                };
                at = lists::rebuild(form, raw, at, old, scratch).map_err(|error| failed(error, slot, hour, form))?;
                std::mem::swap(cell, scratch);
                now = Now::Scratch;
                unrecorded = false;
            }
            lists::GONE => unrecorded = true,
            lists::SAME => {}
            other => return Err(invalid(format!("List form {other}"))),
        }
        *events += 1;
        if at >= end {
            if at > end {
                return Err(invalid("A list event runs past its track"));
            }
            hour = u32::from(NEVER);
            break;
        }
        hour += get_varint(raw, &mut at)?.min(u64::from(BLOCK) + 1) as u32;
        if hour > to {
            break;
        }
    }
    Ok((at, hour, now, unrecorded))
}

fn failed(error: io::Error, slot: u32, hour: u32, form: u8) -> io::Error {
    invalid(format!("{error}: slot {slot}, hour {hour}, form {form}"))
}

pub fn status(entry: &TableEntry, hour: u16, first_hour: i32) -> io::Result<(Status, bool, Option<i32>, Option<u16>)> {
    let bytes = &entry.status;
    let mut at = 0;
    let runs = get_varint(bytes, &mut at)? as usize;
    let mut held = (Status::NotRecorded, false, None);
    let mut start = 0_u16;
    for _ in 0..runs {
        start = start.wrapping_add(get_varint(bytes, &mut at)? as u16);
        let status = get_status(bytes, &mut at)?;
        let read = *bytes.get(at).ok_or_else(|| invalid("A status run ends inside it"))? != 0;
        at += 1;
        let sample = get_varint(bytes, &mut at)?;
        if start > hour {
            return Ok((held.0, held.1, held.2, Some(start)));
        }
        held = (status, read, sampled(sample, first_hour + i32::from(hour)));
    }
    Ok((held.0, held.1, held.2, None))
}

fn sampled(code: u64, hour: i32) -> Option<i32> {
    match code {
        0 => None,
        1 => Some(hour),
        value => {
            let value = (value - 2) as u32;
            Some(((value >> 1) as i32) ^ -((value & 1) as i32))
        }
    }
}

pub fn reads(entry: &TableEntry, hours: u16, first_hour: i32, each: &mut dyn FnMut(i32, i32)) -> io::Result<()> {
    let mut emit = |from: u16, until: u16, sample: u64| {
        let until = until.min(hours);
        if from >= until {
            return;
        }
        let (first, last) = (first_hour + i32::from(from), first_hour + i32::from(until - 1));
        match (sample, sampled(sample, first)) {
            (1, _) => each(first, last),
            (_, Some(hour)) if (first..=last).contains(&hour) => each(hour, hour),
            _ => {}
        }
    };
    let bytes = &entry.status;
    let mut at = 0;
    let runs = get_varint(bytes, &mut at)? as usize;
    let mut start = 0_u16;
    let mut held: Option<(u16, u64)> = None;
    for _ in 0..runs {
        start = start.wrapping_add(get_varint(bytes, &mut at)? as u16);
        get_status(bytes, &mut at)?;
        bytes.get(at).ok_or_else(|| invalid("A status run ends inside it"))?;
        at += 1;
        let sample = get_varint(bytes, &mut at)?;
        if let Some((from, code)) = held {
            emit(from, start, code);
        }
        held = Some((start, sample));
    }
    if let Some((from, code)) = held {
        emit(from, hours, code);
    }
    Ok(())
}

pub fn samples_held(entry: &TableEntry) -> io::Result<bool> {
    let bytes = &entry.status;
    let mut at = 0;
    let runs = get_varint(bytes, &mut at)? as usize;
    for _ in 0..runs {
        get_varint(bytes, &mut at)?;
        get_status(bytes, &mut at)?;
        bytes.get(at).ok_or_else(|| invalid("A status run ends inside it"))?;
        at += 1;
        if get_varint(bytes, &mut at)? != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn statuses(entry: &TableEntry, hours: u16, first_hour: i32, each: &mut dyn FnMut(i32, i32, Status)) -> io::Result<()> {
    record_status_block();
    let mut emit = |from: u16, until: u16, status: Status| {
        let until = until.min(hours);
        if from < until {
            each(first_hour + i32::from(from), first_hour + i32::from(until - 1), status);
        }
    };
    let bytes = &entry.status;
    let mut at = 0;
    let runs = get_varint(bytes, &mut at)? as usize;
    let mut start = 0_u16;
    let mut held: (u16, Status) = (0, Status::NotRecorded);
    for _ in 0..runs {
        start = start.wrapping_add(get_varint(bytes, &mut at)? as u16);
        let status = get_status(bytes, &mut at)?;
        bytes.get(at).ok_or_else(|| invalid("A status run ends inside it"))?;
        at += 1;
        get_varint(bytes, &mut at)?;
        emit(held.0, start, held.1);
        held = (start, status);
    }
    emit(held.0, hours, held.1);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    Difference(i64),
    Value(u64),
    Missing,
    Unrecorded,
    List(u8, &'a [u8]),
}

impl Part {
    pub fn tracks(&self, at: usize, list: bool, mut each: impl FnMut(u32, Option<u16>, &[(u16, Event<'_>)])) -> io::Result<()> {
        let Some(Some(opened)) = self.columns.get(at) else { return Ok(()) };
        let raw = self.frames.upto(opened.span);
        let mut events = Vec::new();
        for (born, index) in opened.order() {
            let tracks = opened.group(born);
            events.clear();
            let mut hour = tracks.next[index];
            let mut gap = tracks.gap.get(index).copied().unwrap_or(0);
            let mut cursor = tracks.at[index] as usize;
            let end = tracks.end[index] as usize;
            let track = raw.get(..end).ok_or_else(|| invalid("A track passes its stream"))?;
            let mut last = 0_i64;
            loop {
                if list {
                    let start = cursor;
                    let form = *track.get(cursor).ok_or_else(|| invalid("A list track ends before its form"))?;
                    lists::skip(track, &mut cursor, false)?;
                    if form == lists::POOLED {
                        let mut index = start + 1;
                        let (place, length) = *opened.pool.get(get_varint(track, &mut index)? as usize).ok_or_else(|| invalid("An event names no pooled literal"))?;
                        events.push((hour, Event::List(lists::LITERAL, &raw[place as usize..(place + length) as usize])));
                    } else {
                        events.push((hour, Event::List(form, &track[start + 1..cursor])));
                    }
                } else {
                    let code = get_code(track, &mut cursor)?;
                    let repeats = if opened.model == RUNS { get_varint(track, &mut cursor)?.min(u64::from(BLOCK)) } else { 0 };
                    for repeat in 0..=repeats {
                        if repeat > 0 {
                            hour = hour.wrapping_add(gap);
                        }
                        let event = match code {
                            MISSING => {
                                last = 0;
                                Event::Missing
                            }
                            UNRECORDED => {
                                last = 0;
                                Event::Unrecorded
                            }
                            code if code & 1 == 1 => return Err(invalid("A track's event code is reserved")),
                            code => {
                                let number = unzig((code >> 1) as u64);
                                match opened.model {
                                    RAW => Event::Value(number as u64),
                                    SECOND => {
                                        last = last.wrapping_add(number);
                                        Event::Difference(last)
                                    }
                                    _ => Event::Difference(number),
                                }
                            }
                        };
                        events.push((hour, event));
                    }
                }
                if cursor >= end {
                    break;
                }
                gap = get_varint(track, &mut cursor)?.min(u64::from(BLOCK)) as u16;
                hour = hour.wrapping_add(gap);
            }
            each(tracks.slot[index], born.then(|| tracks.born[index]), &events);
        }
        Ok(())
    }
}

fn chunk_and_prefix(source: &dyn Extents, chunk: &super::restore::Chunk, reading: &mut Reading, out: &mut Vec<u8>, prefix: &mut Vec<u8>) -> io::Result<bool> {
    let zstd = |error: engine_allocation::zstd::Error| invalid(format!("A restore chunk does not decompress: {}", error.name()));
    let mut stored = std::mem::take(&mut reading.stored);
    let result = (|| {
        if let Some(frame) = &chunk.prefix {
            source.read_at(frame.offset, frame.stored as usize, &mut stored)?;
            verify(frame, &stored)?;
            prefix.clear();
            prefix.reserve(frame.raw as usize);
            reading.decompressor()?.decompress_append(&stored, prefix).map_err(zstd)?;
            if prefix.len() != frame.raw as usize {
                return Err(invalid("A restore chunk's prefix decompresses to another length"));
            }
        }
        source.read_at(chunk.frame.offset, chunk.frame.stored as usize, &mut stored)?;
        verify(&chunk.frame, &stored)?;
        out.clear();
        out.reserve(chunk.frame.raw as usize + PAD);
        match &chunk.prefix {
            Some(_) => reading.decompressor()?.decompress_append_prefixed(prefix, &stored, out).map_err(zstd)?,
            None => reading.decompressor()?.decompress_append(&stored, out).map_err(zstd)?,
        };
        if out.len() != chunk.frame.raw as usize {
            return Err(invalid("A restore chunk decompresses to another length"));
        }
        out.resize(out.len() + PAD, 0);
        Ok(chunk.prefix.is_some())
    })();
    reading.stored = stored;
    result
}

pub fn restore(source: &dyn Extents, entry: &super::restore::RestoreEntry, kinds: &[Kind], columns: &[usize]) -> io::Result<(TableState, super::restore::Meta)> {
    restore_into(source, entry, kinds, columns, false, false, None).map(|(table, meta, _)| (table, meta))
}

pub fn restore_keyed(source: &dyn Extents, entry: &super::restore::RestoreEntry, kinds: &[Kind], columns: &[usize]) -> io::Result<(TableState, super::restore::Meta)> {
    restore_into(source, entry, kinds, columns, true, false, None).map(|(table, meta, _)| (table, meta))
}

pub fn restore_with_string_tables(source: &dyn Extents, entry: &super::restore::RestoreEntry, kinds: &[Kind], columns: &[usize], keyed: bool) -> io::Result<(TableState, super::restore::Meta, Vec<super::restore::StringTable>)> {
    restore_into(source, entry, kinds, columns, keyed, true, None)
}

type Restored = (TableState, super::restore::Meta, Vec<super::restore::StringTable>);

pub fn restore_text(source: &dyn Extents, entry: &super::restore::RestoreEntry, id: u32) -> io::Result<Option<Vec<u8>>> {
    let chunk = entry.streams.iter().find(|(id, _)| *id == super::restore::META)
        .and_then(|(_, chunks)| chunks.first()).ok_or_else(|| invalid("A restore holds no meta stream"))?;
    reading(|reading| {
        let (mut raw, mut prefix) = (Vec::new(), Vec::new());
        let against = chunk_and_prefix(source, chunk, reading, &mut raw, &mut prefix)?;
        let own = super::restore::meta_text(&raw[..chunk.frame.raw as usize], id)?;
        if own.is_some() || !against { return Ok(own); }
        super::restore::meta_text(&prefix, id)
    })
}

pub fn restore_selected(source: &dyn Extents, entry: &super::restore::RestoreEntry, kinds: &[Kind], columns: &[usize], wants: &dyn Fn(u128) -> bool) -> io::Result<(TableState, Strings)> {
    let (cells, _, strings) = restore_into(source, entry, kinds, columns, true, true, Some(wants))?;
    Ok((cells, Strings::restored(strings)))
}

fn restore_into(source: &dyn Extents, entry: &super::restore::RestoreEntry, kinds: &[Kind], columns: &[usize], keyed: bool, lazy: bool, wants: Option<&dyn Fn(u128) -> bool>) -> io::Result<Restored> {
    use super::restore::{self as held, META, ROWS, ROWS_CHUNK};
    if entry.log {
        return logs::log_restore(source, entry, kinds, lazy);
    }
    reading(|reading| {
        let mut table = TableState::new(kinds);
        let stream = |id: u32| entry.streams.iter().find(|(stream, _)| *stream == id).map(|(_, chunks)| chunks.as_slice()).unwrap_or(&[]);
        let (mut raw, mut prefix) = (Vec::new(), Vec::new());
        let meta_chunk = stream(META).first().ok_or_else(|| invalid("A restore holds no meta stream"))?;
        let against = chunk_and_prefix(source, meta_chunk, reading, &mut raw, &mut prefix)?;
        let mut tables = Vec::new();
        let (meta, whole_slots) = if lazy {
            let (meta, own) = held::read_meta_lazily(std::mem::take(&mut raw), meta_chunk.frame.raw as usize, kinds, &mut table)?;
            let (whole_slots, whole) = if against {
                let length = prefix.len();
                let (whole, strings) = held::read_meta_lazily(std::mem::take(&mut prefix), length, kinds, &mut TableState::new(kinds))?;
                (Some(whole.slots), strings.without_held_in(&own)?)
            } else {
                (None, held::StringTable::default())
            };
            tables.extend([own, whole].into_iter().filter(|strings| !strings.is_empty()));
            tables.shrink_to_fit();
            (meta, whole_slots)
        } else {
            let mut meta = held::read_meta(&raw[..meta_chunk.frame.raw as usize], kinds, &mut table)?;
            let whole_slots = if against {
                let whole = held::read_meta(&prefix, kinds, &mut TableState::new(kinds))?;
                meta.strings.extend(whole.strings);
                Some(whole.slots)
            } else {
                None
            };
            (meta, whole_slots)
        };
        table.set_slots(meta.slots);
        let range = |chunks: &[super::restore::Chunk], index: usize| {
            let from = chunks[index].first;
            let to = chunks.get(index + 1).map_or(meta.slots, |next| next.first).max(from);
            (from, to.min(meta.slots.max(from)))
        };
        let rows = stream(ROWS);
        if !rows.is_empty() || keyed {
            table.enable_index();
        }
        let mut keys = Vec::new();
        for index in 0..rows.len() {
            let against = chunk_and_prefix(source, &rows[index], reading, &mut raw, &mut prefix)?;
            let bytes = &raw[..rows[index].frame.raw as usize];
            let (from, to) = range(rows, index);
            if against {
                let whole_slots = whole_slots.ok_or_else(|| invalid("A restore's rows are against a whole one its meta is not"))?;
                let whole_to = from.saturating_add(ROWS_CHUNK).min(whole_slots).max(from);
                bulk::changed_rows(bytes, from, to, &prefix, whole_to, &mut table, &mut keys)?;
            } else {
                bulk::rows(bytes, from, to, &mut table, &mut keys)?;
            }
        }
        if !keyed {
            table.clear_index();
        }
        let mut selected = wants.map(|wants| {
            let mut compact = TableState::new(kinds);
            compact.set_status(table.status(), table.sampled_at());
            compact.enable_index();
            let mut slots = Vec::new();
            for slot in table.rows() {
                let key = table.key(slot).expect("a live slot");
                if wants(key) {
                    let at = slots.len() as u32;
                    compact.insert_at(at, key)?;
                    slots.push((slot, at));
                }
            }
            Ok::<_, io::Error>((compact, slots))
        }).transpose()?;
        let (mut values, mut cells) = (Vec::new(), Vec::new());
        for &column in columns {
            let chunks = stream(held::column_stream(column));
            if chunks.is_empty() {
                selected.as_mut().map_or(&mut table, |(compact, _)| compact).unrecord_rows(column)?;
                continue;
            }
            let list = matches!(kinds.get(column), Some(Kind::U32List | Kind::U64List | Kind::Opaque));
            for index in 0..chunks.len() {
                chunk_and_prefix(source, &chunks[index], reading, &mut raw, &mut prefix)?;
                let end = chunks[index].frame.raw as usize;
                let (from, to) = range(chunks, index);
                if let Some((compact, slots)) = &mut selected {
                    bulk::selected_cells(&raw, end, (from, to), column, table.live(), compact, slots)?;
                } else if list {
                    bulk::lists(&raw, end, from, to, column, &mut table, &mut cells)?;
                } else {
                    bulk::numbers(&raw, end, from, to, column, &mut table, &mut values)?;
                }
            }
        }
        Ok((selected.map_or(table, |(compact, _)| compact), meta, tables))
    })
}

pub trait PageEntry {
    fn block(&self) -> u32;
    fn hours(&self) -> u16;
    fn table_entry(&self, table: usize) -> Option<&TableEntry>;
}

impl PageEntry for super::BlockPage {
    fn block(&self) -> u32 {
        self.block
    }

    fn hours(&self) -> u16 {
        self.hours
    }

    fn table_entry(&self, table: usize) -> Option<&TableEntry> {
        self.tables.get(table)
    }
}

pub fn restored_before<P: PageEntry>(pages: &[P], table: usize, at: usize) -> Option<(usize, &super::restore::RestoreEntry)> {
    for index in (0..=at.min(pages.len().checked_sub(1)?)).rev() {
        let entry = pages[index].table_entry(table)?;
        if let Some(restore) = &entry.restore {
            return Some((index, restore));
        }
        if let Some(first) = &entry.first {
            return Some((index, first));
        }
    }
    None
}

pub fn rebuild<P: PageEntry>(source: &dyn Extents, pages: &[P], kinds: &[Kind], table: usize, at: usize, run_first: i32) -> io::Result<(TableState, HashMap<u32, Vec<u8>>)> {
    if class_of(pages.get(..=at).unwrap_or(pages), table)? {
        return Err(invalid(format!("Table {table} is a log: it has rows by hour, not a state")));
    }
    let (restored, entry) = restored_before(pages, table, at).ok_or_else(|| invalid("No restore of the table is written"))?;
    let columns: Vec<usize> = (0..kinds.len()).collect();
    let (mut cells, meta) = restore(source, entry, kinds, &columns)?;
    let mut texts: HashMap<u32, Vec<u8>> = meta.strings.into_iter().collect();
    let first = pages[restored].table_entry(table).expect("table entry").restore.is_none();
    let replay_from = if first { restored } else { restored + 1 };
    for (index, page) in pages.iter().enumerate().take(at + 1).skip(replay_from) {
        let entry = page.table_entry(table).expect("table entry");
        let hours = page.hours();
        let start = if first && index == restored { 1 } else { 0 };
        let mut cursor = Cursor::new(kinds, &columns, start);
        let mut written = vec![Vec::new(); columns.len()];
        let mut lives = Lives::default();
        let mut next = 0;
        loop {
            match cursor.advance(hours - 1, BLOCK as u16, &mut cells, &mut written, &mut lives)? {
                Advanced::Reached => break,
                Advanced::Missing { .. } => {
                    let Some(part) = entry.parts.get(next) else { break };
                    let last = entry.parts.get(next + 1).map_or(hours - 1, |following| following.first - 1);
                    let part = fetch(source, part, last, &columns, cursor.prefix().as_ref())?;
                    for (id, text) in part.strings() {
                        texts.insert(*id, text.clone());
                    }
                    cursor.push(part);
                    next += 1;
                }
            }
            written.iter_mut().for_each(Vec::clear);
            lives.deleted.clear();
            lives.inserted.clear();
        }
        if entry.packed && index != restored {
            cells.pack()?;
        }
    }
    let page = &pages[at];
    let hours = page.hours();
    let (status, _, sampled, _) = status(page.table_entry(table).expect("table entry"), hours - 1, run_first + (page.block() * BLOCK) as i32)?;
    cells.set_status(status, sampled);
    Ok((cells, texts))
}
