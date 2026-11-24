
use crate::{
    invalid,
    manifest::Status,
    state::Kind,
    store::{Strings, Value},
};
use engine_allocation::zstd::{Compressor, Holder};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::{Arc, mpsc},
};

pub mod lists;
pub mod reader;
pub mod restore;
#[cfg(test)]
mod tests;

pub fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub(crate) fn put_wide(out: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[inline]
pub fn get_varint(bytes: &[u8], at: &mut usize) -> io::Result<u64> {
    let (mut value, mut shift) = (0_u64, 0);
    loop {
        let byte = *bytes.get(*at).ok_or_else(|| invalid("A unit stream ends inside a number"))?;
        *at += 1;
        if shift >= 64 || u64::from(byte & 0x7f) >> (64 - shift).min(7) != 0 {
            return Err(invalid("A unit stream's number passes 64 bits"));
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(value);
        }
        shift += 7;
    }
}

#[inline]
pub(crate) fn get_wide(bytes: &[u8], at: &mut usize) -> io::Result<u128> {
    get_within(bytes, at, 128, "A unit stream's number passes 128 bits")
}

#[inline]
pub(crate) fn get_code(bytes: &[u8], at: &mut usize) -> io::Result<u128> {
    get_within(bytes, at, 65, "A track's event code passes 65 bits")
}

#[inline]
fn get_within(bytes: &[u8], at: &mut usize, width: u32, past: &'static str) -> io::Result<u128> {
    let (mut value, mut shift) = (0_u128, 0_u32);
    loop {
        let byte = *bytes.get(*at).ok_or_else(|| invalid("A unit stream ends inside a number"))?;
        *at += 1;
        if shift >= width || u32::from(byte & 0x7f) >> (width - shift).min(7) != 0 {
            return Err(invalid(past));
        }
        value |= u128::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(value);
        }
        shift += 7;
    }
}

#[inline]
pub(crate) fn zig(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

#[inline]
pub(crate) fn unzig(value: u64) -> i64 {
    (value >> 1) as i64 ^ -((value & 1) as i64)
}

fn zig_wide(value: i128) -> u128 {
    ((value << 1) ^ (value >> 127)) as u128
}

pub(crate) fn unzig_wide(value: u128) -> i128 {
    (value >> 1) as i128 ^ -((value & 1) as i128)
}

pub const BLOCK: u32 = 720;

pub const LIFETIMES: u32 = 0;
pub const STRINGS: u32 = 1;
pub const READ: u32 = 2;
pub const TOUCHED: u32 = 3;

pub const fn column_stream(column: usize) -> u32 {
    column as u32 + 4
}

pub const DIFFERENCES: u8 = 0;
pub const RUNS: u8 = 1;
pub const RAW: u8 = 2;
pub const SECOND: u8 = 3;

pub const MISSING: u128 = 1;
pub const UNRECORDED: u128 = 3;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub level: i32,
    pub cap: usize,
    pub budget: usize,
    pub small: usize,
    pub window_log: u32,
    pub queue: usize,
    pub runs: u64,
    pub restore_events: u64,
    pub restore_blocks: u32,
    pub whole_every: u32,
    pub pack_margin: u32,
}

impl Settings {
    pub const fn at(level: i32) -> Self {
        Self {
            level,
            cap: 1 << 20,
            budget: 48 << 20,
            small: 4096,
            window_log: 21,
            queue: 8 << 20,
            runs: 4,
            restore_events: 500_000,
            restore_blocks: 6,
            whole_every: 4,
            pack_margin: 65_536,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Chunks {
    cuts: Vec<u32>,
    whole: Vec<FrameEntry>,
    written: Vec<u64>,
}

impl Chunks {
    fn cut(&mut self, cuts: Vec<u32>) {
        self.written = vec![0; cuts.len().div_ceil(64)];
        self.cuts = cuts;
        self.whole.clear();
    }

    #[inline]
    fn mark(&mut self, slot: u32) {
        let index = if self.cuts.len() <= 1 { 0 } else { self.cuts.partition_point(|&cut| cut <= slot).saturating_sub(1) };
        if let Some(word) = self.written.get_mut(index / 64) {
            *word |= 1 << (index % 64);
        }
    }

    fn clean(&self, index: usize) -> bool {
        self.written.get(index / 64).is_some_and(|word| word >> (index % 64) & 1 == 0)
    }

    fn whole_range(&self, index: usize, slots: u32) -> (u32, u32) {
        let from = self.cuts[index];
        (from, self.cuts.get(index + 1).copied().unwrap_or(slots).min(slots).max(from))
    }
}

#[derive(Clone, Debug, Default)]
struct Rule {
    events: u64,
    blocks: u32,
    since_whole: u32,
    restored: bool,
    chunks: Vec<Chunks>,
    whole_slots: u32,
    last: Option<restore::RestoreEntry>,
    whole_strings: HashSet<u32, foldhash::fast::FixedState>,
    status: Option<(Status, Option<i32>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameEntry {
    pub streams: Vec<u32>,
    pub offset: u64,
    pub stored: u32,
    pub raw: u32,
    pub digest: [u8; 16],
    pub chained: bool,
    pub small: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartEntry {
    pub first: u16,
    pub frames: Vec<FrameEntry>,
    pub ordinal: u64,
    pub log: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableEntry {
    pub parts: Vec<PartEntry>,
    pub status: Vec<u8>,
    pub first: Option<restore::RestoreEntry>,
    pub restore: Option<restore::RestoreEntry>,
    pub packed: bool,
    pub log: bool,
    pub count: u64,
}

pub(crate) fn put_frame(out: &mut Vec<u8>, frame: &FrameEntry, previous: &mut u64) {
    put_varint(out, frame.streams.len() as u64);
    frame.streams.iter().for_each(|&stream| put_varint(out, u64::from(stream)));
    put_varint(out, frame.offset.wrapping_sub(*previous));
    *previous = frame.offset;
    put_varint(out, u64::from(frame.stored));
    put_varint(out, u64::from(frame.raw));
    out.extend_from_slice(&frame.digest);
    out.push(u8::from(frame.chained) | u8::from(frame.small) << 1);
}

pub(crate) fn get_frame(bytes: &[u8], at: &mut usize, previous: &mut u64) -> io::Result<FrameEntry> {
    let count = get_varint(bytes, at)? as usize;
    if count > bytes.len() {
        return Err(invalid("A frame counts more streams than its page holds"));
    }
    let streams = (0..count).map(|_| get_varint(bytes, at).map(|id| id as u32)).collect::<io::Result<_>>()?;
    let offset = previous.wrapping_add(get_varint(bytes, at)?);
    *previous = offset;
    let stored = get_varint(bytes, at)? as u32;
    let raw = get_varint(bytes, at)? as u32;
    let digest: [u8; 16] = bytes.get(*at..*at + 16).ok_or_else(|| invalid("A block page ends inside a digest"))?.try_into().expect("sixteen bytes");
    *at += 16;
    let flags = *bytes.get(*at).ok_or_else(|| invalid("A block page ends inside a frame"))?;
    *at += 1;
    if flags > 3 {
        return Err(invalid(format!("Frame flags {flags}")));
    }
    Ok(FrameEntry { streams, offset, stored, raw, digest, chained: flags & 1 != 0, small: flags & 2 != 0 })
}

fn put_restore(out: &mut Vec<u8>, entry: &restore::RestoreEntry, previous: &mut u64) {
    out.push(u8::from(entry.whole));
    put_varint(out, entry.streams.len() as u64);
    for (stream, chunks) in &entry.streams {
        put_varint(out, u64::from(*stream));
        put_varint(out, chunks.len() as u64);
        let mut slot = 0_u32;
        for chunk in chunks {
            put_varint(out, u64::from(chunk.first - slot));
            slot = chunk.first;
            put_frame(out, &chunk.frame, previous);
            match &chunk.prefix {
                None => out.push(0),
                Some(prefix) => {
                    out.push(1);
                    put_frame(out, prefix, previous);
                }
            }
        }
    }
}

fn get_restore(bytes: &[u8], at: &mut usize, previous: &mut u64) -> io::Result<restore::RestoreEntry> {
    let whole = *bytes.get(*at).ok_or_else(|| invalid("A block page ends inside a restore"))? != 0;
    *at += 1;
    let count = get_varint(bytes, at)? as usize;
    if count > bytes.len() {
        return Err(invalid("A restore counts more streams than its page holds"));
    }
    let mut streams = Vec::with_capacity(count);
    for _ in 0..count {
        let stream = get_varint(bytes, at)? as u32;
        let chunks = get_varint(bytes, at)? as usize;
        if chunks > bytes.len() {
            return Err(invalid("A restore counts more chunks than its page holds"));
        }
        let mut held = Vec::with_capacity(chunks);
        let mut slot = 0_u32;
        for _ in 0..chunks {
            slot += get_varint(bytes, at)? as u32;
            let frame = get_frame(bytes, at, previous)?;
            let prefix = match *bytes.get(*at).ok_or_else(|| invalid("A block page ends inside a chunk"))? {
                0 => {
                    *at += 1;
                    None
                }
                _ => {
                    *at += 1;
                    Some(get_frame(bytes, at, previous)?)
                }
            };
            held.push(restore::Chunk { first: slot, frame, prefix });
        }
        streams.push((stream, held));
    }
    Ok(restore::RestoreEntry { whole, streams, log: false })
}

impl TableEntry {
    pub fn write(&self, out: &mut Vec<u8>) {
        put_varint(out, self.parts.len() as u64);
        let mut previous = 0_u64;
        for part in &self.parts {
            put_varint(out, u64::from(part.first));
            put_varint(out, part.frames.len() as u64);
            for frame in &part.frames {
                put_frame(out, frame, &mut previous);
            }
        }
        put_varint(out, self.status.len() as u64);
        out.extend_from_slice(&self.status);
        out.push(u8::from(self.first.is_some()) | u8::from(self.restore.is_some()) << 1 | u8::from(self.packed) << 2 | u8::from(self.log) << 3);
        if self.log {
            put_varint(out, u64::from(self.count));
            let mut ordinal = self.count;
            for part in &self.parts {
                put_varint(out, part.ordinal - ordinal);
                ordinal = part.ordinal;
            }
        }
        for entry in [&self.first, &self.restore].into_iter().flatten() {
            put_restore(out, entry, &mut previous);
        }
    }

    pub fn read(bytes: &[u8], at: &mut usize) -> io::Result<Self> {
        let parts = get_varint(bytes, at)? as usize;
        if parts > bytes.len() {
            return Err(invalid("A table entry counts more parts than its page holds"));
        }
        let mut entry = TableEntry::default();
        let mut previous = 0_u64;
        for _ in 0..parts {
            let first = get_varint(bytes, at)? as u16;
            let frames = get_varint(bytes, at)? as usize;
            if frames > bytes.len() {
                return Err(invalid("A part counts more frames than its page holds"));
            }
            let mut part = PartEntry { first, frames: Vec::with_capacity(frames), ..PartEntry::default() };
            for _ in 0..frames {
                part.frames.push(get_frame(bytes, at, &mut previous)?);
            }
            entry.parts.push(part);
        }
        let length = get_varint(bytes, at)? as usize;
        entry.status = bytes.get(*at..at.saturating_add(length)).ok_or_else(|| invalid("A block page ends inside a status"))?.to_vec();
        *at += length;
        let flags = *bytes.get(*at).ok_or_else(|| invalid("A block page ends inside a table"))?;
        *at += 1;
        if flags > 15 {
            return Err(invalid(format!("Table entry flags {flags}")));
        }
        entry.log = flags & 8 != 0;
        if entry.log {
            if flags & 4 != 0 {
                return Err(invalid("A log's table entry says it was packed"));
            }
            entry.count = get_varint(bytes, at)?;
            let mut ordinal = entry.count;
            for part in &mut entry.parts {
                ordinal = ordinal.checked_add(get_varint(bytes, at)?).ok_or_else(|| invalid("A log part's ordinal passes 64 bits"))?;
                part.ordinal = ordinal;
                part.log = true;
            }
        }
        if flags & 1 != 0 {
            entry.first = Some(get_restore(bytes, at, &mut previous)?);
        }
        if flags & 2 != 0 {
            entry.restore = Some(get_restore(bytes, at, &mut previous)?);
        }
        entry.packed = flags & 4 != 0;
        for restore in [&mut entry.first, &mut entry.restore].into_iter().flatten() {
            restore.log = entry.log;
        }
        Ok(entry)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockPage {
    pub block: u32,
    pub hours: u16,
    pub tables: Vec<TableEntry>,
}

impl BlockPage {
    pub fn write(&self, out: &mut Vec<u8>) {
        put_varint(out, u64::from(self.block));
        put_varint(out, u64::from(self.hours));
        put_varint(out, self.tables.len() as u64);
        for table in &self.tables {
            table.write(out);
        }
    }

    pub fn read(bytes: &[u8], at: &mut usize) -> io::Result<Self> {
        let block = get_varint(bytes, at)? as u32;
        let hours = get_varint(bytes, at)? as u16;
        let count = get_varint(bytes, at)? as usize;
        if count > bytes.len() {
            return Err(invalid("A block page counts more tables than it holds"));
        }
        let mut tables = Vec::with_capacity(count);
        for _ in 0..count {
            tables.push(TableEntry::read(bytes, at)?);
        }
        Ok(Self { block, hours, tables })
    }
}

pub trait PartSink {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64>;
    fn block(&mut self, page: BlockPage) -> io::Result<()>;
    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()>;
}

pub struct Discard;

impl PartSink for Discard {
    fn frame(&mut self, _: &[u8]) -> io::Result<u64> {
        Ok(0)
    }

    fn block(&mut self, _: BlockPage) -> io::Result<()> {
        Ok(())
    }

    fn read_back(&mut self, _: u64, _: u32, _: &mut Vec<u8>) -> io::Result<()> {
        Err(invalid("A unit written nowhere has no restore to read back"))
    }
}

fn digest(bytes: &[u8]) -> [u8; 16] {
    blake3::hash(bytes).as_bytes()[..16].try_into().expect("sixteen bytes")
}

fn put_status(out: &mut Vec<u8>, status: Status) {
    match status {
        Status::NotRecorded => out.push(0),
        Status::Complete => out.push(1),
        Status::Truncated { missing } => {
            out.push(2);
            put_varint(out, u64::from(missing));
        }
        Status::Unavailable { reason } => {
            out.push(3);
            put_varint(out, u64::from(reason));
        }
    }
}

pub(crate) fn get_status(bytes: &[u8], at: &mut usize) -> io::Result<Status> {
    let tag = *bytes.get(*at).ok_or_else(|| invalid("A status run ends inside a status"))?;
    *at += 1;
    Ok(match tag {
        0 => Status::NotRecorded,
        1 => Status::Complete,
        2 => Status::Truncated { missing: get_varint(bytes, at)? as u32 },
        3 => Status::Unavailable { reason: get_varint(bytes, at)? as u32 },
        other => return Err(invalid(format!("Status tag {other}"))),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HourStatus {
    pub status: Status,
    pub read: bool,
    pub sampled: Option<i32>,
}

fn sample_code(sampled: Option<i32>, hour: i32) -> u64 {
    match sampled {
        None => 0,
        Some(held) if held == hour => 1,
        Some(held) => u64::from(((held << 1) ^ (held >> 31)) as u32) + 2,
    }
}

fn status_runs(runs: &[(u16, Status, bool, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, runs.len() as u64);
    let mut previous = 0_u16;
    for &(hour, status, read, sample) in runs {
        put_varint(&mut out, u64::from(hour - previous));
        previous = hour;
        put_status(&mut out, status);
        out.push(u8::from(read));
        put_varint(&mut out, sample);
    }
    out
}

#[derive(Default)]
struct Pending {
    buffer: Vec<u8>,
    hour: u32,
    next: u32,
    events: u64,
    literals: Vec<u8>,
    places: Vec<(u32, u32)>,
    by_digest: HashMap<u64, Vec<u32>, foldhash::fast::FixedState>,
}

impl Pending {
    fn new() -> Self {
        Self { hour: u32::MAX, ..Self::default() }
    }

    fn bytes(&self) -> usize {
        self.buffer.capacity() + self.literals.capacity() + self.places.capacity() * 8 + self.by_digest.capacity() * 40
    }

    fn head(&mut self, hour: u16, slot: u32) {
        if self.hour != u32::from(hour) || slot < self.next {
            self.buffer.push(0);
            put_varint(&mut self.buffer, u64::from(hour));
            self.hour = u32::from(hour);
            self.next = 0;
        }
        put_varint(&mut self.buffer, u64::from(slot - self.next) + 1);
        self.next = slot;
        self.events += 1;
    }

    fn literal(&mut self, literal: &[u8]) -> u32 {
        let hash = blake3::hash(literal);
        let key = u64::from_le_bytes(hash.as_bytes()[..8].try_into().expect("eight bytes"));
        let ids = self.by_digest.entry(key).or_default();
        for &id in ids.iter() {
            let (start, length) = self.places[id as usize];
            if &self.literals[start as usize..(start + length) as usize] == literal {
                return id;
            }
        }
        let id = self.places.len() as u32;
        self.places.push((self.literals.len() as u32, literal.len() as u32));
        self.literals.extend_from_slice(literal);
        ids.push(id);
        id
    }

    fn literal_bytes(&self, id: u32) -> &[u8] {
        let (start, length) = self.places[id as usize];
        &self.literals[start as usize..(start + length) as usize]
    }

    fn clear(&mut self) {
        *self = Self::new();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Trying,
    Raw,
    Differences,
}

struct Column {
    list: bool,
    pending: Pending,
    inserts: Vec<u8>,
    insert_ids: Vec<u32>,
    choice: Choice,
}

struct Open {
    columns: Vec<Column>,
    touched: Pending,
    read: Pending,
    lifetimes: Vec<u8>,
    life_hour: u16,
    previous_key: u128,
    births: Vec<(u32, u16)>,
    strings: Vec<u32>,
    named: HashSet<u32, foldhash::fast::FixedState>,
    first: u16,
    used: bool,
    bytes: usize,
    previous: HashMap<u32, Arc<Vec<u8>>, foldhash::fast::FixedState>,
    status: Vec<(u16, Status, bool, u64)>,
    held_status: Option<(Status, bool, u64)>,
    entry: TableEntry,
    rule: Rule,
    restoring: Option<(bool, restore::RestoreEntry)>,
    log: bool,
    log_hour: Option<(u64, u64)>,
    log_keys: Vec<u8>,
    forgets: Vec<(u64, u64)>,
    part_appends: u64,
    block_appends: u64,
}

impl Open {
    fn new(kinds: &[Kind]) -> Self {
        Self {
            columns: kinds
                .iter()
                .map(|&kind| Column {
                    list: matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque),
                    pending: Pending::new(),
                    inserts: Vec::new(),
                    insert_ids: Vec::new(),
                    choice: Choice::Trying,
                })
                .collect(),
            touched: Pending::new(),
            read: Pending::new(),
            lifetimes: Vec::new(),
            life_hour: 0,
            previous_key: 0,
            births: Vec::new(),
            strings: Vec::new(),
            named: HashSet::default(),
            first: 0,
            used: false,
            bytes: 0,
            previous: HashMap::default(),
            status: Vec::new(),
            held_status: None,
            entry: TableEntry::default(),
            rule: Rule::default(),
            restoring: None,
            log: false,
            log_hour: None,
            log_keys: Vec::new(),
            forgets: Vec::new(),
            part_appends: 0,
            block_appends: 0,
        }
    }

    fn count(&mut self) -> usize {
        self.bytes = self
            .columns
            .iter()
            .map(|column| column.pending.bytes() + column.inserts.capacity() + column.insert_ids.capacity() * 4)
            .sum::<usize>()
            + self.touched.bytes()
            + self.read.bytes()
            + self.lifetimes.capacity()
            + self.births.capacity() * 8
            + self.strings.capacity() * 4
            + self.log_keys.capacity()
            + self.forgets.capacity() * 16;
        self.bytes
    }

    fn largest_column(&self) -> usize {
        if self.log {
            let values = self.columns.iter().map(|column| column.inserts.len() + column.pending.literals.len() + column.insert_ids.len()).max().unwrap_or(0);
            return values.max(self.lifetimes.len());
        }
        self.columns.iter().map(|column| column.pending.buffer.len() + column.pending.literals.len()).max().unwrap_or(0)
    }

    fn previous_bytes(&self) -> usize {
        self.previous.values().map(|stream| stream.capacity()).sum()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Written {
    pub parts: u64,
    pub frames: u64,
    pub small_frames: u64,
    pub numeric: (u64, u64),
    pub list: (u64, u64),
    pub lifetimes: (u64, u64),
    pub strings: (u64, u64),
    pub marks: (u64, u64),
    pub small: (u64, u64),
    pub directory_raw: u64,
    pub inserted_raw: u64,
    pub tracks: u64,
    pub events: u64,
    pub largest_stream: u64,
    pub models: [u64; 4],
    pub closes_cap: u64,
    pub closes_budget: u64,
    pub closes_block: u64,
    pub restores: (u64, u64),
    pub whole_restores: u64,
    pub based_restores: u64,
    pub repeated_restores: u64,
    pub restore_chunks: u64,
    pub named_chunks: u64,
    pub worst_replay: u64,
    pub largest_chunk: u64,
    pub packed: u64,
    pub read_back: u64,
    pub restore_parts: [[u64; 3]; 2],
}

impl Written {
    pub fn stored(&self) -> u64 {
        self.numeric.0 + self.list.0 + self.lifetimes.0 + self.strings.0 + self.marks.0 + self.small.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Held {
    pub pending: usize,
    pub previous: usize,
    pub pending_and_previous: usize,
    pub closing: usize,
    pub queued: usize,
    pub context: usize,
    pub restoring: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub appending: std::time::Duration,
    pub appends: u64,
    pub closing: std::time::Duration,
    pub restoring: std::time::Duration,
    pub reading_back: std::time::Duration,
    pub compressing_parts: std::time::Duration,
    pub compressing_restores: std::time::Duration,
    pub digesting: std::time::Duration,
    pub writing: std::time::Duration,
    pub waiting: std::time::Duration,
}

struct Job {
    table: usize,
    first: u16,
    ordinal: u64,
    streams: Vec<(u32, Arc<Vec<u8>>, Option<Arc<Vec<u8>>>)>,
    small: Vec<(u32, Vec<u8>)>,
    kinds: Vec<StreamKind>,
    restore: Option<RestoreJob>,
}

#[derive(Clone, Debug)]
struct RestoreJob {
    stream: u32,
    slot: u32,
    prefix: Option<FrameEntry>,
    named: Option<FrameEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamKind {
    Numeric,
    List,
    Lifetimes,
    Strings,
    Marks,
    Restore,
}

struct Done {
    table: usize,
    first: u16,
    ordinal: u64,
    restore: Option<RestoreJob>,
    spent: std::time::Duration,
    frames: Vec<(Vec<u32>, Vec<u8>, u32, bool, bool, Vec<StreamKind>)>,
}

static UNIT_ZSTD: Holder = Holder::new("unit compression");

fn label(name: &str) -> engine_allocation::labels::Scope {
    use engine_allocation::labels;
    labels::enter(labels::owner(name), labels::Kind::Operator)
}

enum Work {
    Part(Job),
    Frame(Vec<u8>, mpsc::Sender<io::Result<Vec<u8>>>),
    Stop,
}

#[derive(Clone)]
pub struct FrameCompressor {
    work: mpsc::Sender<Work>,
}

impl FrameCompressor {
    pub fn compress(&self, raw: &[u8]) -> io::Result<Vec<u8>> {
        let stopped = || invalid("The unit's compression thread stopped");
        let (reply, answer) = mpsc::channel();
        self.work.send(Work::Frame(raw.to_vec(), reply)).map_err(|_| stopped())?;
        answer.recv().map_err(|_| stopped())?
    }
}

struct Compression {
    jobs: Option<mpsc::Sender<Work>>,
    done: mpsc::Receiver<io::Result<Done>>,
    thread: Option<std::thread::JoinHandle<()>>,
    queued: VecDeque<usize>,
    context: Arc<std::sync::atomic::AtomicUsize>,
}

impl Compression {
    fn start(settings: Settings) -> io::Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Work>();
        let (outbox, done) = mpsc::channel();
        let context = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = context.clone();
        let thread = std::thread::Builder::new().name("unit compression".into()).spawn(move || {
            let _label = label("compressing");
            let mut compressor = match Compressor::new(&UNIT_ZSTD, settings.level).and_then(|mut compressor| {
                compressor.cap_window(settings.level, settings.window_log)?;
                Ok(compressor)
            }) {
                Ok(compressor) => compressor,
                Err(error) => {
                    let _ = outbox.send(Err(error));
                    return;
                }
            };
            for work in inbox {
                match work {
                    Work::Part(job) => {
                        let result = compress(&mut compressor, job, settings);
                        seen.fetch_max(compressor.bytes(), std::sync::atomic::Ordering::Relaxed);
                        if outbox.send(result).is_err() {
                            return;
                        }
                    }
                    Work::Frame(raw, reply) => {
                        let mut out = Vec::with_capacity(engine_allocation::zstd::bound(raw.len()));
                        let result = compressor.compress_append(&raw, &mut out).map(|_| out).map_err(|error| invalid(format!("zstd: {}", error.name())));
                        drop(raw);
                        seen.fetch_max(compressor.bytes(), std::sync::atomic::Ordering::Relaxed);
                        let _ = reply.send(result);
                    }
                    Work::Stop => return,
                }
            }
        })?;
        Ok(Self { jobs: Some(jobs), done, thread: Some(thread), queued: VecDeque::new(), context })
    }

    fn queued(&self) -> usize {
        self.queued.iter().sum()
    }
}

impl Drop for Compression {
    fn drop(&mut self) {
        if let Some(jobs) = self.jobs.take() {
            let _ = jobs.send(Work::Stop);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn compress(compressor: &mut Compressor, job: Job, settings: Settings) -> io::Result<Done> {
    let started = std::time::Instant::now();
    let mut frames = Vec::new();
    let zstd = |error: engine_allocation::zstd::Error| invalid(format!("zstd: {}", error.name()));
    for ((id, raw, prefix), kind) in job.streams.iter().zip(&job.kinds) {
        let mut out = Vec::with_capacity(engine_allocation::zstd::bound(raw.len()));
        match prefix {
            Some(prefix) => compressor.compress_append_prefixed(prefix, raw, &mut out).map_err(zstd)?,
            None => compressor.compress_append(raw, &mut out).map_err(zstd)?,
        };
        frames.push((vec![*id], out, raw.len() as u32, prefix.is_some(), false, vec![*kind]));
    }
    if !job.small.is_empty() {
        let mut joined = Vec::new();
        put_varint(&mut joined, job.small.len() as u64);
        for (id, raw) in &job.small {
            put_varint(&mut joined, u64::from(*id));
            put_varint(&mut joined, raw.len() as u64);
        }
        for (_, raw) in &job.small {
            joined.extend_from_slice(raw);
        }
        let mut out = Vec::with_capacity(engine_allocation::zstd::bound(joined.len()));
        compressor.compress_append(&joined, &mut out).map_err(zstd)?;
        let kinds = job.kinds[job.streams.len()..].to_vec();
        frames.push((job.small.iter().map(|(id, _)| *id).collect(), out, joined.len() as u32, false, true, kinds));
    }
    let _ = settings;
    Ok(Done { table: job.table, first: job.first, ordinal: job.ordinal, restore: job.restore, frames, spent: started.elapsed() })
}

#[derive(Clone, Copy)]
struct Found {
    slot: u32,
    hour: u16,
    at: u32,
    end: u32,
}

#[derive(Default)]
struct Scratch {
    found: Vec<Found>,
    directory: Vec<u8>,
    events: Vec<u8>,
    runs: Vec<u8>,
    run_directory: Vec<u8>,
    raw_events: Vec<u8>,
    raw_directory: Vec<u8>,
    second_events: Vec<u8>,
    second_directory: Vec<u8>,
    pool: Vec<u8>,
    pooled: Vec<u32>,
    uses: Vec<u32>,
    inserted: Vec<u8>,
    births: Vec<(u32, u16)>,
    counts: Vec<u32>,
    hours: Vec<u16>,
    peak: usize,
}

const SCRATCH_KEPT: usize = 4 << 20;

impl Scratch {
    fn room(&self) -> usize {
        self.found.capacity() * size_of::<Found>()
            + [&self.directory, &self.events, &self.runs, &self.run_directory, &self.raw_events, &self.raw_directory, &self.second_events, &self.second_directory, &self.pool, &self.inserted]
                .iter()
                .map(|held| held.capacity())
                .sum::<usize>()
            + (self.pooled.capacity() + self.uses.capacity() + self.counts.capacity()) * 4
            + self.hours.capacity() * 2
            + self.births.capacity() * size_of::<(u32, u16)>()
    }
}

pub struct Encoder {
    settings: Settings,
    tables: Vec<Open>,
    run_first: i32,
    hour: i32,
    on: bool,
    scratch: Scratch,
    payload: Vec<u8>,
    matcher: lists::Matcher,
    compression: Compression,
    changed: Vec<usize>,
    changed_marks: Vec<bool>,
    pending_total: usize,
    previous_total: usize,
    pub written: Written,
    pub held: Held,
    decompressor: engine_allocation::zstd::Decompressor,
    moves: Vec<(usize, Vec<Option<u32>>)>,
    timed: bool,
    pub timings: Timings,
}

impl Encoder {
    pub fn new(kinds: &[Vec<Kind>], settings: Settings) -> io::Result<Self> {
        let _label = label("pending");
        Ok(Self {
            settings,
            tables: kinds.iter().map(|kinds| Open::new(kinds)).collect(),
            run_first: 0,
            hour: 0,
            on: false,
            scratch: Scratch::default(),
            payload: Vec::new(),
            matcher: lists::Matcher::default(),
            compression: Compression::start(settings)?,
            changed: Vec::new(),
            changed_marks: vec![false; kinds.len()],
            pending_total: 0,
            previous_total: 0,
            written: Written::default(),
            held: Held::default(),
            decompressor: engine_allocation::zstd::Decompressor::new(&UNIT_ZSTD)?,
            moves: Vec::new(),
            timed: false,
            timings: Timings::default(),
        })
    }

    pub fn time(&mut self, timed: bool) {
        self.timed = timed;
    }

    fn started(&self) -> Option<std::time::Instant> {
        self.timed.then(std::time::Instant::now)
    }

    fn appended(&mut self, started: Option<std::time::Instant>) {
        if let Some(started) = started {
            self.timings.appending += started.elapsed();
            self.timings.appends += 1;
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn take_moves(&mut self) -> Vec<(usize, Vec<Option<u32>>)> {
        std::mem::take(&mut self.moves)
    }

    pub fn begin_run(&mut self, first_hour: i32) {
        self.on = false;
        for open in &mut self.tables {
            for column in &mut open.columns {
                column.choice = Choice::Trying;
            }
            open.named.clear();
            open.first = 1;
            open.status.clear();
            open.held_status = None;
            open.rule = Rule::default();
        }
        self.run_first = first_hour;
    }

    pub fn frames(&self) -> FrameCompressor {
        FrameCompressor { work: self.compression.jobs.clone().expect("a running thread") }
    }

    pub fn begin_hour(&mut self, hour: i32) {
        self.hour = hour;
        self.on = hour != self.run_first;
    }

    fn block_hour(&self) -> u16 {
        ((self.hour - self.run_first) as u32 % BLOCK) as u16
    }

    fn block(&self) -> u32 {
        (self.hour - self.run_first) as u32 / BLOCK
    }

    pub fn ends_block(&self) -> bool {
        u32::from(self.block_hour()) == BLOCK - 1
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    fn mark(&mut self, table: usize) {
        if !self.changed_marks[table] {
            self.changed_marks[table] = true;
            self.changed.push(table);
        }
    }

    pub fn status(&mut self, table: usize, held: HourStatus) {
        let started = self.started();
        let hour = self.block_hour();
        let run = (held.status, held.read, sample_code(held.sampled, self.hour));
        let open = &mut self.tables[table];
        if open.held_status != Some(run) || open.status.is_empty() {
            match open.status.last_mut() {
                Some(last) if last.0 == hour => *last = (hour, run.0, run.1, run.2),
                _ => open.status.push((hour, run.0, run.1, run.2)),
            }
            open.held_status = Some(run);
        }
        if let Status::Unavailable { reason } = held.status
            && self.on
            && !self.tables[table].named.contains(&reason)
        {
            let _label = label("pending");
            self.note_string(table, reason);
            self.tables[table].used = true;
            self.mark(table);
        }
        self.appended(started);
    }

    fn note_string(&mut self, table: usize, id: u32) {
        let open = &mut self.tables[table];
        if open.named.insert(id) {
            open.strings.push(id);
        }
    }

    pub fn number(&mut self, table: usize, column: usize, slot: u32, old: Value<'_>, new: Value<'_>) {
        if !self.on {
            return;
        }
        let started = self.started();
        let _label = label("pending");
        let hour = self.block_hour();
        if let Value::Text(id) = new {
            self.note_string(table, id);
        }
        let (old_bits, _) = bits(old);
        let (new_bits, held) = bits(new);
        let difference = match held {
            Value::Unrecorded => UNRECORDED,
            Value::Missing => MISSING,
            _ => u128::from(zig(new_bits.wrapping_sub(old_bits) as i64)) << 1,
        };
        let raw = match held {
            Value::Unrecorded => UNRECORDED,
            Value::Missing => MISSING,
            _ => u128::from(zig(new_bits as i64)) << 1,
        };
        let open = &mut self.tables[table];
        open.used = true;
        open.rule.events += 1;
        if let Some(chunks) = open.rule.chunks.get_mut(restore::column_stream(column) as usize) {
            chunks.mark(slot);
        }
        let pending_column = &mut open.columns[column];
        pending_column.pending.head(hour, slot);
        match pending_column.choice {
            Choice::Raw => put_wide(&mut pending_column.pending.buffer, raw),
            Choice::Differences => put_wide(&mut pending_column.pending.buffer, difference),
            Choice::Trying => {
                put_wide(&mut pending_column.pending.buffer, difference);
                put_wide(&mut pending_column.pending.buffer, raw);
            }
        }
        self.mark(table);
        self.appended(started);
    }

    pub fn list(&mut self, table: usize, column: usize, slot: u32, old: Value<'_>, new: Value<'_>) {
        if !self.on {
            return;
        }
        let started = self.started();
        let _label = label("pending");
        let hour = self.block_hour();
        let open = &mut self.tables[table];
        open.used = true;
        open.rule.events += 1;
        if let Some(chunks) = open.rule.chunks.get_mut(restore::column_stream(column) as usize) {
            chunks.mark(slot);
        }
        let pending = &mut open.columns[column].pending;
        pending.head(hour, slot);
        match (old, new) {
            (_, Value::Unrecorded) | (_, Value::Missing) => pending.buffer.push(lists::GONE),
            (Value::List(old), Value::List(new)) if old == new => pending.buffer.push(lists::SAME),
            (Value::List(old), Value::List(new)) if lists::against(new, old, &mut self.payload, &mut self.matcher) => {
                pending.buffer.extend_from_slice(&self.payload);
            }
            (_, Value::List(new)) => {
                let id = pending.literal(new);
                pending.buffer.push(lists::LITERAL);
                put_varint(&mut pending.buffer, u64::from(id));
            }
            _ => pending.buffer.push(lists::GONE),
        }
        self.mark(table);
        self.appended(started);
    }

    pub fn declare_logs(&mut self, logs: &[bool]) {
        for (open, &log) in self.tables.iter_mut().zip(logs) {
            open.log = log;
        }
    }

    pub fn append_row(&mut self, table: usize, ordinal: u64, key: u128) -> io::Result<()> {
        let started = self.started();
        let _label = label("pending");
        let first_hour = self.hour == self.run_first;
        let open = &mut self.tables[table];
        if !open.log {
            return Err(invalid(format!("Table {table} holds state: its rows are inserted, never appended")));
        }
        match &mut open.log_hour {
            Some((first, count)) if first.checked_add(*count) == Some(ordinal) => *count += 1,
            Some(_) => return Err(invalid(format!("Log table {table} appends row {ordinal} out of order"))),
            None => open.log_hour = Some((ordinal, 1)),
        }
        if first_hour && !open.used {
            open.first = 0;
        }
        put_wide(&mut open.log_keys, zig_wide(key.wrapping_sub(open.previous_key) as i128));
        open.previous_key = key;
        open.used = true;
        open.rule.events += 1;
        open.block_appends += 1;
        self.mark(table);
        self.appended(started);
        Ok(())
    }

    pub fn forget_rows(&mut self, table: usize, first: u64, count: u64) {
        let _label = label("pending");
        let first_hour = self.hour == self.run_first;
        let open = &mut self.tables[table];
        if first_hour && !open.used {
            open.first = 0;
        }
        open.forgets.push((first, count));
        open.used = true;
        open.rule.events += count;
        self.mark(table);
    }

    fn log_lives(&mut self, table: usize) {
        let hour = self.block_hour();
        let open = &mut self.tables[table];
        if open.log_hour.is_none() && open.forgets.is_empty() {
            return;
        }
        let (first, count) = open.log_hour.take().unwrap_or((0, 0));
        let since = if open.lifetimes.is_empty() { open.first } else { open.life_hour };
        put_varint(&mut open.lifetimes, u64::from(hour - since));
        put_varint(&mut open.lifetimes, open.forgets.iter().map(|&(_, run)| run).sum());
        put_varint(&mut open.lifetimes, count);
        open.forgets.sort_unstable();
        let mut previous = 0_u64;
        for &(from, run) in &open.forgets {
            put_varint(&mut open.lifetimes, zig((from - previous) as i64));
            for _ in 1..run {
                put_varint(&mut open.lifetimes, zig(1));
            }
            previous = from + run - 1;
        }
        if count > 0 {
            put_varint(&mut open.lifetimes, zig(first as i64));
            for _ in 1..count {
                put_varint(&mut open.lifetimes, zig(1));
            }
        }
        open.lifetimes.extend_from_slice(&open.log_keys);
        open.log_keys.clear();
        open.forgets.clear();
        open.life_hour = hour;
    }

    pub fn touch(&mut self, table: usize, slot: u32) {
        if !self.on {
            return;
        }
        let hour = self.block_hour();
        let open = &mut self.tables[table];
        open.used = true;
        open.touched.head(hour, slot);
        self.mark(table);
    }

    pub fn read_rows(&mut self, table: usize, slots: &[u32]) {
        if !self.on {
            return;
        }
        let hour = self.block_hour();
        let open = &mut self.tables[table];
        open.used = true;
        for &slot in slots {
            open.read.head(hour, slot);
        }
        self.mark(table);
    }

    pub fn lives(&mut self, table: usize, deleted: &[(u32, u128)], inserted: &[u32], key: impl Fn(u32) -> u128) {
        if !self.on || deleted.is_empty() && inserted.is_empty() {
            return;
        }
        let started = self.started();
        let _label = label("pending");
        let hour = self.block_hour();
        let open = &mut self.tables[table];
        open.used = true;
        open.rule.events += (deleted.len() + inserted.len()) as u64;
        for (stream, chunks) in open.rule.chunks.iter_mut().enumerate() {
            if stream as u32 != restore::META {
                for slot in deleted.iter().map(|&(slot, _)| slot).chain(inserted.iter().copied()) {
                    chunks.mark(slot);
                }
            }
        }
        let since = if open.lifetimes.is_empty() { open.first } else { open.life_hour };
        put_varint(&mut open.lifetimes, u64::from(hour - since));
        put_varint(&mut open.lifetimes, deleted.len() as u64);
        put_varint(&mut open.lifetimes, inserted.len() as u64);
        let mut previous = 0_i64;
        for &(slot, _) in deleted {
            put_varint(&mut open.lifetimes, zig(i64::from(slot) - previous));
            previous = i64::from(slot);
        }
        let mut previous = 0_i64;
        for &slot in inserted {
            put_varint(&mut open.lifetimes, zig(i64::from(slot) - previous));
            previous = i64::from(slot);
        }
        for &slot in inserted {
            let key = key(slot);
            put_wide(&mut open.lifetimes, zig_wide(key.wrapping_sub(open.previous_key) as i128));
            open.previous_key = key;
            open.births.push((slot, hour));
        }
        open.life_hour = hour;
        self.mark(table);
        self.appended(started);
    }

    pub fn inserted(&mut self, table: usize, column: usize, value: Value<'_>) {
        if !self.on && !self.tables[table].log {
            return;
        }
        let started = self.started();
        let _label = label("pending");
        if let Value::Text(id) = value {
            self.note_string(table, id);
        }
        let held = &mut self.tables[table].columns[column];
        if held.list {
            let (bytes, unrecorded) = match value {
                Value::List(bytes) => (bytes, false),
                _ => (&[][..], true),
            };
            let id = held.pending.literal(bytes);
            held.insert_ids.push(id << 1 | u32::from(unrecorded));
        } else {
            let (bits, value) = bits(value);
            let code = match value {
                Value::Unrecorded => UNRECORDED,
                Value::Missing => MISSING,
                _ => u128::from(zig(bits as i64)) << 1,
            };
            put_wide(&mut held.inserts, code);
        }
        self.appended(started);
    }

    pub fn end_hour(&mut self, tables: &mut [crate::store::TableState], strings: &Strings, sink: &mut dyn PartSink) -> io::Result<()> {
        for at in 0..self.changed.len() {
            let table = self.changed[at];
            if self.tables[table].log {
                self.log_lives(table);
            }
        }
        if self.hour == self.run_first {
            for table in 0..self.tables.len() {
                self.write_restore(table, &tables[table], strings, true, true, sink)?;
            }
            self.settle_restores(sink)?;
        }
        let hour = self.block_hour();
        let block_ends = u32::from(hour) == BLOCK - 1;
        let changed = std::mem::take(&mut self.changed);
        for &table in &changed {
            self.changed_marks[table] = false;
            let before = self.tables[table].bytes;
            let now = self.tables[table].count();
            self.pending_total = self.pending_total + now - before;
        }
        self.held.pending = self.held.pending.max(self.pending_total);
        self.held.pending_and_previous = self.held.pending_and_previous.max(self.pending_total + self.previous_total);
        if block_ends {
            for table in 0..self.tables.len() {
                if self.tables[table].used {
                    self.written.closes_block += 1;
                }
                self.close(table, hour, false, strings, sink)?;
            }
            self.restores(tables, strings, sink)?;
            self.end_block(hour, tables, sink)?;
        } else {
            for &table in &changed {
                if self.tables[table].largest_column() >= self.settings.cap {
                    self.written.closes_cap += 1;
                    self.close(table, hour, true, strings, sink)?;
                }
            }
            while self.pending_total + self.previous_total > self.settings.budget {
                let largest = (0..self.tables.len()).max_by_key(|&table| self.tables[table].bytes).expect("a table");
                if self.tables[largest].bytes == 0 {
                    break;
                }
                self.written.closes_budget += 1;
                self.close(largest, hour, true, strings, sink)?;
            }
        }
        self.drain(sink, false)?;
        Ok(())
    }

    pub fn end_run(&mut self, tables: &mut [crate::store::TableState], strings: &Strings, sink: &mut dyn PartSink) -> io::Result<()> {
        let hour = self.block_hour();
        if u32::from(hour) != BLOCK - 1 {
            for table in 0..self.tables.len() {
                self.close(table, hour, false, strings, sink)?;
            }
            self.restores(tables, strings, sink)?;
            self.end_block(hour, tables, sink)?;
        }
        self.drain(sink, true)
    }

    fn restores(&mut self, tables: &mut [crate::store::TableState], strings: &Strings, sink: &mut dyn PartSink) -> io::Result<()> {
        let settings = self.settings;
        let since: u64 = self.tables.iter().map(|open| open.rule.events).sum();
        self.written.worst_replay = self.written.worst_replay.max(since);
        for table in 0..self.tables.len() {
            let log = self.tables[table].log;
            let rule = &mut self.tables[table].rule;
            rule.blocks += 1;
            if !log && rule.restored && rule.events < settings.restore_events && rule.blocks < settings.restore_blocks {
                continue;
            }
            let state = &mut tables[table];
            let meta = (state.status(), state.sampled_at());
            if rule.restored && rule.events == 0 && rule.status == Some(meta) {
                rule.blocks = 0;
                self.tables[table].entry.restore = rule.last.clone();
                self.written.repeated_restores += 1;
                continue;
            }
            let whole = log || !rule.restored || rule.since_whole + 1 >= settings.whole_every;
            if !log && whole && state.slots() as u64 > 2 * state.live().count() as u64 + u64::from(settings.pack_margin) {
                let moved = state.pack()?;
                self.moves.push((table, moved));
                self.tables[table].entry.packed = true;
                self.written.packed += 1;
            }
            self.write_restore(table, state, strings, whole, false, sink)?;
        }
        self.settle_restores(sink)
    }

    fn settle_restores(&mut self, sink: &mut dyn PartSink) -> io::Result<()> {
        self.drain(sink, true)?;
        for open in &mut self.tables {
            let Some((first, entry)) = open.restoring.take() else { continue };
            if entry.whole {
                for (stream, chunks) in &entry.streams {
                    let held = open.rule.chunks.get_mut(*stream as usize).ok_or_else(|| invalid("A whole restore's stream past its table"))?;
                    held.whole = chunks.iter().map(|chunk| chunk.frame.clone()).collect();
                }
            }
            open.rule.last = Some(entry.clone());
            if first {
                open.entry.first = Some(entry);
            } else {
                open.entry.restore = Some(entry);
            }
        }
        Ok(())
    }

    fn write_restore(&mut self, table: usize, state: &crate::store::TableState, strings: &Strings, whole: bool, first: bool, sink: &mut dyn PartSink) -> io::Result<()> {
        let _label = label("restoring");
        let started = std::time::Instant::now();
        let before = self.timings;
        let open = &mut self.tables[table];
        let log = open.log;
        open.restoring = Some((first, restore::RestoreEntry { whole, streams: Vec::new(), log }));
        let rule = &mut open.rule;
        rule.restored = true;
        rule.events = 0;
        rule.blocks = 0;
        rule.since_whole = if whole { 0 } else { rule.since_whole + 1 };
        rule.status = Some((state.status(), state.sampled_at()));
        let slots = state.slots();
        if whole {
            rule.chunks = vec![Chunks::default(); state.columns().len() + 2];
            rule.whole_slots = slots;
            self.written.whole_restores += 1;
        } else {
            self.written.based_restores += 1;
        }
        let mut raw = Vec::new();
        let rule = &mut self.tables[table].rule;
        let listed = restore::meta(state, strings, (!whole).then_some(&rule.whole_strings), &mut raw)?;
        if whole {
            rule.whole_strings = listed.iter().copied().collect();
            rule.chunks[restore::META as usize].cut(vec![0]);
        }
        let open = &mut self.tables[table];
        open.named.clear();
        open.named.extend(open.rule.whole_strings.iter().copied());
        open.named.extend(listed);
        open.named.extend(open.strings.iter().copied());
        self.restore_chunk(table, restore::META, 0, 0, raw, whole, sink)?;
        if log {
            let mut raw = Vec::new();
            restore::listed_chunk(state, &mut raw);
            self.send_chunk(table, restore::ROWS, 0, raw, None, sink)?;
            self.timings.restoring += started.elapsed();
            return Ok(());
        }
        let rows_stream = restore::ROWS as usize;
        let whole_rows = self.tables[table].rule.chunks[rows_stream].cuts.clone();
        let mut rows: Vec<u32> = if whole { (0..slots.div_ceil(restore::ROWS_CHUNK).max(1)).map(|chunk| chunk * restore::ROWS_CHUNK).collect() } else { whole_rows.clone() };
        if rows.is_empty() {
            rows.push(0);
        }
        while let Some(&last) = rows.last()
            && last + restore::ROWS_CHUNK < slots
        {
            rows.push(last + restore::ROWS_CHUNK);
        }
        if whole {
            self.tables[table].rule.chunks[rows_stream].cut(rows.clone());
        }
        for (index, &from) in rows.iter().enumerate() {
            let next = rows.get(index + 1).copied();
            let to = next.unwrap_or(slots).max(from);
            if from >= slots && index > 0 {
                break;
            }
            let mut raw = Vec::new();
            if whole || index >= whole_rows.len() {
                restore::rows_chunk(state, from, to, &mut raw);
                self.send_chunk(table, restore::ROWS, from, raw, None, sink)?;
                continue;
            }
            if self.name_again(table, restore::ROWS, index, from, next, slots, sink)? {
                continue;
            }
            let prefix = self.read_whole(table, restore::ROWS, index, sink)?.ok_or_else(|| invalid("A restore's rows chunk has no whole one to be against"))?;
            let (_, whole_to) = self.tables[table].rule.chunks[rows_stream].whole_range(index, self.tables[table].rule.whole_slots);
            restore::changed_rows_chunk(state, from, to, &prefix.1, whole_to, &mut raw)?;
            self.send_chunk(table, restore::ROWS, from, raw, Some(prefix), sink)?;
        }
        for column in 0..state.columns().len() {
            let stream = restore::column_stream(column);
            if whole {
                let mut cuts = vec![0_u32];
                let mut raw = Vec::new();
                let mut from = 0;
                for slot in 0..slots {
                    restore::column_chunk(state, column, slot, slot + 1, &mut raw);
                    if raw.len() >= restore::CHUNK && slot + 1 < slots {
                        let index = cuts.len() - 1;
                        self.restore_chunk(table, stream, index, from, std::mem::take(&mut raw), whole, sink)?;
                        from = slot + 1;
                        cuts.push(from);
                    }
                }
                let index = cuts.len() - 1;
                self.restore_chunk(table, stream, index, from, raw, whole, sink)?;
                self.tables[table].rule.chunks[stream as usize].cut(cuts);
                continue;
            }
            let mut cuts = self.tables[table].rule.chunks[stream as usize].cuts.clone();
            if cuts.is_empty() {
                cuts.push(0);
            }
            for (index, &from) in cuts.iter().enumerate() {
                if from >= slots && index > 0 {
                    break;
                }
                let next = cuts.get(index + 1).copied().filter(|&next| next < slots);
                if self.name_again(table, stream, index, from, next, slots, sink)? {
                    continue;
                }
                if let Some(&to) = cuts.get(index + 1) {
                    let mut raw = Vec::new();
                    restore::column_chunk(state, column, from, to.max(from), &mut raw);
                    self.restore_chunk(table, stream, index, from, raw, whole, sink)?;
                    continue;
                }
                let (mut index, mut start, mut raw) = (index, from, Vec::new());
                for slot in from..slots {
                    restore::column_chunk(state, column, slot, slot + 1, &mut raw);
                    if raw.len() >= restore::CHUNK && slot + 1 < slots {
                        self.restore_chunk(table, stream, index, start, std::mem::take(&mut raw), whole, sink)?;
                        index += 1;
                        start = slot + 1;
                    }
                }
                self.restore_chunk(table, stream, index, start, raw, whole, sink)?;
            }
        }
        let others = (self.timings.reading_back - before.reading_back)
            + (self.timings.waiting - before.waiting)
            + (self.timings.digesting - before.digesting)
            + (self.timings.writing - before.writing);
        self.timings.restoring += started.elapsed().saturating_sub(others);
        Ok(())
    }

    fn name_again(&mut self, table: usize, stream: u32, index: usize, from: u32, next: Option<u32>, slots: u32, sink: &mut dyn PartSink) -> io::Result<bool> {
        let rule = &self.tables[table].rule;
        let Some(chunks) = rule.chunks.get(stream as usize) else { return Ok(false) };
        let Some(frame) = chunks.whole.get(index) else { return Ok(false) };
        if !chunks.clean(index) || chunks.whole_range(index, rule.whole_slots) != (from, next.unwrap_or(slots).min(slots).max(from)) {
            return Ok(false);
        }
        let job = Job {
            table,
            first: 0,
            ordinal: 0,
            streams: Vec::new(),
            small: Vec::new(),
            kinds: Vec::new(),
            restore: Some(RestoreJob { stream, slot: from, prefix: None, named: Some(frame.clone()) }),
        };
        self.written.named_chunks += 1;
        self.compression.queued.push_back(0);
        self.compression.jobs.as_ref().expect("a running thread").send(Work::Part(job)).map_err(|_| invalid("The unit's compression thread stopped"))?;
        self.drain(sink, false)?;
        Ok(true)
    }

    fn read_whole(&mut self, table: usize, stream: u32, index: usize, sink: &mut dyn PartSink) -> io::Result<Option<(FrameEntry, Arc<Vec<u8>>)>> {
        let Some(frame) = self.tables[table].rule.chunks.get(stream as usize).and_then(|chunks| chunks.whole.get(index)).cloned() else {
            return Ok(None);
        };
        let _label = label("restoring");
        let started = std::time::Instant::now();
        let mut stored = Vec::new();
        sink.read_back(frame.offset, frame.stored, &mut stored)?;
        if digest(&stored) != frame.digest {
            return Err(invalid("A whole restore's chunk read back differs from its digest"));
        }
        let mut bytes = Vec::with_capacity(frame.raw as usize);
        self.decompressor
            .decompress_append(&stored, &mut bytes)
            .map_err(|error| invalid(format!("A whole restore's chunk does not decompress: {}", error.name())))?;
        self.written.read_back += stored.len() as u64;
        self.timings.reading_back += started.elapsed();
        Ok(Some((frame, Arc::new(bytes))))
    }

    fn restore_chunk(&mut self, table: usize, stream: u32, index: usize, slot: u32, raw: Vec<u8>, whole: bool, sink: &mut dyn PartSink) -> io::Result<()> {
        let prefix = if whole { None } else { self.read_whole(table, stream, index, sink)? };
        self.send_chunk(table, stream, slot, raw, prefix, sink)
    }

    fn send_chunk(&mut self, table: usize, stream: u32, slot: u32, raw: Vec<u8>, prefix: Option<(FrameEntry, Arc<Vec<u8>>)>, sink: &mut dyn PartSink) -> io::Result<()> {
        let (prefix_frame, prefix) = match prefix {
            Some((frame, bytes)) => (Some(frame), Some(bytes)),
            None => (None, None),
        };
        let held = raw.len() + prefix.as_ref().map_or(0, |prefix| prefix.len());
        self.held.restoring = self.held.restoring.max(held);
        self.written.restores.1 += raw.len() as u64;
        self.written.largest_chunk = self.written.largest_chunk.max(raw.len() as u64);
        self.written.restore_chunks += 1;
        let job = Job {
            table,
            first: 0,
            ordinal: 0,
            streams: vec![(stream, Arc::new(raw), prefix)],
            small: Vec::new(),
            kinds: vec![StreamKind::Restore],
            restore: Some(RestoreJob { stream, slot, prefix: prefix_frame, named: None }),
        };
        self.compression.queued.push_back(held);
        self.held.queued = self.held.queued.max(self.compression.queued());
        self.compression.jobs.as_ref().expect("a running thread").send(Work::Part(job)).map_err(|_| invalid("The unit's compression thread stopped"))?;
        self.drain(sink, false)
    }

    fn end_block(&mut self, last: u16, tables: &[crate::store::TableState], sink: &mut dyn PartSink) -> io::Result<()> {
        self.drain(sink, true)?;
        let mut page = BlockPage { block: self.block(), hours: last + 1, tables: Vec::with_capacity(self.tables.len()) };
        for (open, state) in self.tables.iter_mut().zip(tables) {
            let mut entry = std::mem::take(&mut open.entry);
            if open.log {
                entry.log = true;
                entry.count = state.count().unwrap_or(0) - open.block_appends;
                for part in &mut entry.parts {
                    part.ordinal += entry.count;
                }
                open.block_appends = 0;
                open.part_appends = 0;
            }
            entry.status = status_runs(&open.status);
            open.status.clear();
            if let Some((status, read, sample)) = open.held_status {
                open.status.push((0, status, read, sample));
            }
            page.tables.push(entry);
            open.first = 0;
        }
        sink.block(page)
    }

    fn drain(&mut self, sink: &mut dyn PartSink, all: bool) -> io::Result<()> {
        loop {
            let must = all && !self.compression.queued.is_empty() || self.compression.queued() > self.settings.queue;
            let done = if must {
                let started = std::time::Instant::now();
                let done = match self.compression.done.recv() {
                    Ok(done) => done,
                    Err(_) => return Err(invalid("The unit's compression thread stopped")),
                };
                self.timings.waiting += started.elapsed();
                done
            } else {
                match self.compression.done.try_recv() {
                    Ok(done) => done,
                    Err(_) => return Ok(()),
                }
            };
            self.compression.queued.pop_front();
            let done = done?;
            if let Some(job) = done.restore {
                self.timings.compressing_restores += done.spent;
                let chunk = match job.named {
                    Some(frame) => restore::Chunk { first: job.slot, frame, prefix: None },
                    None => {
                        let (streams, stored, raw, chained, small, _) = done.frames.into_iter().next().ok_or_else(|| invalid("A restore chunk came back without its frame"))?;
                        let writing = std::time::Instant::now();
                        let offset = sink.frame(&stored)?;
                        self.timings.writing += writing.elapsed();
                        self.written.restores.0 += stored.len() as u64;
                        let part = match job.stream {
                            restore::ROWS => 0,
                            restore::META => 1,
                            _ => 2,
                        };
                        self.written.restore_parts[usize::from(job.prefix.is_some())][part] += stored.len() as u64;
                        self.written.frames += 1;
                        let digesting = std::time::Instant::now();
                        let digest = digest(&stored);
                        self.timings.digesting += digesting.elapsed();
                        restore::Chunk { first: job.slot, frame: FrameEntry { streams, offset, stored: stored.len() as u32, raw, digest, chained, small }, prefix: job.prefix }
                    }
                };
                let (_, entry) = self.tables[done.table].restoring.as_mut().ok_or_else(|| invalid("A restore chunk came back for no restore"))?;
                match entry.streams.last_mut() {
                    Some((stream, chunks)) if *stream == job.stream => chunks.push(chunk),
                    _ => entry.streams.push((job.stream, vec![chunk])),
                }
                continue;
            }
            self.timings.compressing_parts += done.spent;
            let mut part = PartEntry { first: done.first, frames: Vec::with_capacity(done.frames.len()), ordinal: done.ordinal, log: self.tables[done.table].log };
            for (streams, stored, raw, chained, small, kinds) in done.frames {
                let writing = std::time::Instant::now();
                let offset = sink.frame(&stored)?;
                self.timings.writing += writing.elapsed();
                let counted = stored.len() as u64;
                if small {
                    self.written.small.0 += counted;
                    self.written.small.1 += u64::from(raw);
                    self.written.small_frames += 1;
                } else {
                    let slot = match kinds[0] {
                        StreamKind::Numeric => &mut self.written.numeric,
                        StreamKind::List => &mut self.written.list,
                        StreamKind::Lifetimes => &mut self.written.lifetimes,
                        StreamKind::Strings => &mut self.written.strings,
                        StreamKind::Marks | StreamKind::Restore => &mut self.written.marks,
                    };
                    slot.0 += counted;
                }
                self.written.frames += 1;
                let digesting = std::time::Instant::now();
                let digest = digest(&stored);
                self.timings.digesting += digesting.elapsed();
                part.frames.push(FrameEntry { streams, offset, stored: stored.len() as u32, raw, digest, chained, small });
            }
            self.tables[done.table].entry.parts.push(part);
            self.held.context = self.held.context.max(self.compression.context.load(std::sync::atomic::Ordering::Relaxed));
        }
    }

    fn close(&mut self, table: usize, last: u16, chained: bool, strings: &Strings, sink: &mut dyn PartSink) -> io::Result<()> {
        let open = &mut self.tables[table];
        if !open.used {
            open.first = last + 1;
            if !chained && !open.previous.is_empty() {
                open.previous = HashMap::default();
                self.previous_total = self.tables.iter().map(Open::previous_bytes).sum();
            }
            return Ok(());
        }
        let _label = label("closing");
        let started = std::time::Instant::now();
        let settings = self.settings;
        let scratch = &mut self.scratch;
        scratch.births.clear();
        scratch.births.extend_from_slice(&open.births);
        scratch.births.sort_unstable();
        let births = std::mem::take(&mut scratch.births);
        let mut streams: Vec<(u32, Arc<Vec<u8>>, Option<Arc<Vec<u8>>>)> = Vec::new();
        let mut small: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut large_kinds = Vec::new();
        let mut small_kinds = Vec::new();
        let mut kept: HashMap<u32, Arc<Vec<u8>>, foldhash::fast::FixedState> = HashMap::default();
        let mut place = |id: u32, bytes: Vec<u8>, kind: StreamKind, open: &mut Open, written: &mut Written| {
            written.largest_stream = written.largest_stream.max(bytes.len() as u64);
            let tally = match kind {
                StreamKind::Numeric => &mut written.numeric,
                StreamKind::List => &mut written.list,
                StreamKind::Lifetimes => &mut written.lifetimes,
                StreamKind::Strings => &mut written.strings,
                StreamKind::Marks | StreamKind::Restore => &mut written.marks,
            };
            tally.1 += bytes.len() as u64;
            let previous = open.previous.get(&id).cloned();
            let keep = chained && kind != StreamKind::Strings;
            if bytes.len() < settings.small {
                if keep {
                    let _label = label("previous parts");
                    kept.insert(id, Arc::new(bytes.clone()));
                }
                small.push((id, bytes));
                small_kinds.push(kind);
            } else {
                let bytes = Arc::new(bytes);
                if keep {
                    kept.insert(id, bytes.clone());
                }
                streams.push((id, bytes, previous.filter(|_| kind != StreamKind::Strings)));
                large_kinds.push(kind);
            }
        };
        let first = open.first;
        for column in 0..open.columns.len() {
            let held = &open.columns[column];
            if held.pending.events == 0 && held.inserts.is_empty() && held.insert_ids.is_empty() {
                continue;
            }
            let (bytes, model, raw_won) = stream(held, &births, first, settings, scratch)?;
            self.written.events += held.pending.events;
            if !held.list && held.pending.events > 0 {
                self.written.models[model as usize] += 1;
                if held.choice == Choice::Trying {
                    open.columns[column].choice = if raw_won { Choice::Raw } else { Choice::Differences };
                }
            }
            let kind = if open.columns[column].list { StreamKind::List } else { StreamKind::Numeric };
            place(column_stream(column), bytes, kind, open, &mut self.written);
        }
        for (id, pending) in [(TOUCHED, 0), (READ, 1)] {
            let held = if pending == 0 { &open.touched } else { &open.read };
            if held.events == 0 {
                continue;
            }
            let bytes = marks(held, &births, first, scratch)?;
            place(id, bytes, StreamKind::Marks, open, &mut self.written);
        }
        if !open.lifetimes.is_empty() {
            let bytes = std::mem::take(&mut open.lifetimes);
            place(LIFETIMES, bytes, StreamKind::Lifetimes, open, &mut self.written);
        }
        if !open.strings.is_empty() {
            let mut ids = std::mem::take(&mut open.strings);
            ids.sort_unstable();
            let mut bytes = Vec::new();
            put_varint(&mut bytes, ids.len() as u64);
            let mut previous = 0_u32;
            for id in ids {
                let text = strings.get(id).ok_or_else(|| invalid(format!("String {id} a part names is not held")))?;
                put_varint(&mut bytes, u64::from(id - previous));
                previous = id;
                put_varint(&mut bytes, text.len() as u64);
                bytes.extend_from_slice(text);
            }
            place(STRINGS, bytes, StreamKind::Strings, open, &mut self.written);
        }
        self.scratch.births = births;
        self.tables[table].previous = kept;
        let raw: usize = streams.iter().map(|(_, bytes, _)| bytes.len()).sum::<usize>() + small.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        self.held.closing = self.held.closing.max(self.scratch.peak);
        if self.scratch.room() > SCRATCH_KEPT {
            self.scratch = Scratch { peak: self.scratch.peak, ..Scratch::default() };
        }
        self.written.parts += 1;
        large_kinds.extend(small_kinds);
        let job = Job { table, first, ordinal: self.tables[table].part_appends, streams, small, kinds: large_kinds, restore: None };
        {
            let _label = label("frames queued");
            self.compression.queued.push_back(raw);
            self.held.queued = self.held.queued.max(self.compression.queued());
            self.compression.jobs.as_ref().expect("a running thread").send(Work::Part(job)).map_err(|_| invalid("The unit's compression thread stopped"))?;
        }
        let open = &mut self.tables[table];
        for column in &mut open.columns {
            column.pending.clear();
            column.inserts = Vec::new();
            column.insert_ids = Vec::new();
        }
        open.touched.clear();
        open.read.clear();
        open.lifetimes = Vec::new();
        open.log_keys = Vec::new();
        open.forgets = Vec::new();
        open.previous_key = 0;
        open.births = Vec::new();
        open.first = last + 1;
        open.used = false;
        open.part_appends = open.block_appends;
        self.pending_total -= open.bytes;
        open.bytes = 0;
        self.previous_total = self.tables.iter().map(Open::previous_bytes).sum();
        self.held.previous = self.held.previous.max(self.previous_total);
        self.timings.closing += started.elapsed();
        self.drain(sink, false)
    }
}

fn bits(value: Value<'_>) -> (u64, Value<'_>) {
    match value {
        Value::Number(bits) => (bits, value),
        Value::Text(id) => (u64::from(id), value),
        Value::Missing | Value::Unrecorded => (0, value),
        Value::List(_) => (0, value),
    }
}

fn find(pending: &Pending, found: &mut Vec<Found>, payload: impl Fn(&[u8], &mut usize) -> io::Result<()>) -> io::Result<()> {
    found.clear();
    found.reserve_exact(pending.events as usize);
    let bytes = &pending.buffer;
    let (mut at, mut hour, mut slot) = (0_usize, 0_u16, 0_u32);
    while at < bytes.len() {
        let gap = get_varint(bytes, &mut at)?;
        if gap == 0 {
            hour = get_varint(bytes, &mut at)? as u16;
            slot = 0;
            continue;
        }
        slot += (gap - 1) as u32;
        let start = at;
        payload(bytes, &mut at)?;
        found.push(Found { slot, hour, at: start as u32, end: at as u32 });
    }
    found.sort_unstable_by_key(|event| (event.slot, event.at));
    Ok(())
}

fn each_mark(bytes: &[u8], mut each: impl FnMut(u32, u16)) -> io::Result<()> {
    let (mut at, mut hour, mut slot) = (0_usize, 0_u16, 0_u32);
    while at < bytes.len() {
        let gap = get_varint(bytes, &mut at)?;
        if gap == 0 {
            hour = get_varint(bytes, &mut at)? as u16;
            slot = 0;
            continue;
        }
        slot += (gap - 1) as u32;
        each(slot, hour);
    }
    Ok(())
}

fn mark_tracks(ends: &[u32], hours: &[u16], births: &[(u32, u16)], mut each: impl FnMut(u32, &[u16], Option<u16>)) {
    let mut start = 0;
    for (slot, &end) in ends.iter().enumerate() {
        let held = &hours[start..end as usize];
        start = end as usize;
        if held.is_empty() {
            continue;
        }
        let slot = slot as u32;
        let from = births.partition_point(|&(born, _)| born < slot);
        let to = births.partition_point(|&(born, _)| born <= slot);
        let (mut at, mut life) = (0, from);
        loop {
            let next_birth = births.get(life).filter(|_| life < to).map(|&(_, hour)| hour);
            let mut stop = at;
            while stop < held.len() && next_birth.is_none_or(|birth| held[stop] < birth) {
                stop += 1;
            }
            if stop > at {
                each(slot, &held[at..stop], (life > from).then(|| births[life - 1].1));
            }
            if stop == held.len() {
                break;
            }
            at = stop;
            life += 1;
        }
    }
}

fn varint_len(value: u64) -> usize {
    (64 - value.leading_zeros() as usize).div_ceil(7).max(1)
}

fn estimate(directory: &[u8], events: &[u8]) -> usize {
    let mut joined = Vec::with_capacity(directory.len() + events.len());
    joined.extend_from_slice(directory);
    joined.extend_from_slice(events);
    zstd::bulk::compress(&joined, 1).map_or(usize::MAX, |stored| stored.len())
}

fn tracks_of(found: &[Found], at: usize, end: usize, births: &[(u32, u16)], mut each: impl FnMut(usize, usize, Option<u16>) -> io::Result<()>) -> io::Result<()> {
    let slot = found[at].slot;
    let from = births.partition_point(|&(held, _)| held < slot);
    let to = births.partition_point(|&(held, _)| held <= slot);
    let mut life = from;
    let mut start = at;
    loop {
        let next_birth = births.get(life).filter(|_| life < to).map(|&(_, hour)| hour);
        let mut stop = start;
        while stop < end && next_birth.is_none_or(|birth| found[stop].hour < birth) {
            stop += 1;
        }
        if stop > start {
            let born = (life > from).then(|| births[life - 1].1);
            each(start, stop, born)?;
        }
        if stop == end {
            return Ok(());
        }
        start = stop;
        life += 1;
    }
}

fn put_track(directory: &mut Vec<u8>, slot_gap: u32, count: u64, born: Option<u16>, first: u16, bytes: usize) {
    put_varint(directory, u64::from(slot_gap));
    put_varint(directory, count << 1 | u64::from(born.is_some()));
    if let Some(born) = born {
        put_varint(directory, u64::from(born - first));
    }
    put_varint(directory, bytes as u64);
}

fn stream(column: &Column, births: &[(u32, u16)], first: u16, settings: Settings, scratch: &mut Scratch) -> io::Result<(Vec<u8>, u8, bool)> {
    let pending = &column.pending;
    let dual = !column.list && column.choice == Choice::Trying;
    let mut found = std::mem::take(&mut scratch.found);
    if column.list {
        find(pending, &mut found, |bytes, at| lists::skip(bytes, at, true))?;
    } else {
        find(pending, &mut found, |bytes, at| {
            get_wide(bytes, at)?;
            if dual {
                get_wide(bytes, at)?;
            }
            Ok(())
        })?;
    }
    let buffer = &pending.buffer;
    let Scratch { directory, events, runs, run_directory, raw_events, raw_directory, second_events, second_directory, pool, pooled, uses, inserted, .. } = scratch;
    for held in [&mut *directory, &mut *events, &mut *runs, &mut *run_directory, &mut *raw_events, &mut *raw_directory, &mut *second_events, &mut *second_directory, &mut *pool, &mut *inserted] {
        held.clear();
    }
    if column.list {
        uses.clear();
        uses.resize(pending.places.len(), 0);
        for event in &found {
            if buffer[event.at as usize] == lists::LITERAL {
                let mut at = event.at as usize + 1;
                uses[get_varint(buffer, &mut at)? as usize] += 1;
            }
        }
        for &id in &column.insert_ids {
            uses[(id >> 1) as usize] += 1;
        }
        pooled.clear();
        pooled.resize(pending.places.len(), u32::MAX);
        let mut next = 0;
        for (id, &count) in uses.iter().enumerate() {
            if count > 1 {
                pooled[id] = next;
                next += 1;
                let literal = pending.literal_bytes(id as u32);
                put_varint(pool, literal.len() as u64);
                pool.extend_from_slice(literal);
            }
        }
        for &id in &column.insert_ids {
            let (literal, unrecorded) = (id >> 1, id & 1);
            if pooled[literal as usize] != u32::MAX {
                put_varint(inserted, u64::from(pooled[literal as usize]) << 2 | 2 | u64::from(unrecorded));
            } else {
                let bytes = pending.literal_bytes(literal);
                put_varint(inserted, (bytes.len() as u64) << 2 | u64::from(unrecorded));
                inserted.extend_from_slice(bytes);
            }
        }
    } else {
        inserted.extend_from_slice(&column.inserts);
    }
    let (mut tracks, mut previous_slot) = (0_u64, 0_u32);
    let (mut total_events, mut total_runs) = (0_u64, 0_u64);
    let mut at = 0;
    while at < found.len() {
        let slot = found[at].slot;
        let mut end = at;
        while end < found.len() && found[end].slot == slot {
            end += 1;
        }
        tracks_of(&found, at, end, births, |start, stop, born| {
            let origin = born.unwrap_or(first);
            let (events_start, runs_start, raw_start, second_start) = (events.len(), runs.len(), raw_events.len(), second_events.len());
            let mut previous = origin;
            let mut open: Option<(u16, u32, u32, u64)> = None;
            let mut run_count = 0_u64;
            let mut last_difference = 0_i64;
            for event in &found[start..stop] {
                let gap = event.hour - previous;
                previous = event.hour;
                put_varint(events, u64::from(gap));
                let payload = &buffer[event.at as usize..event.end as usize];
                if column.list {
                    let form = payload[0];
                    if form == lists::LITERAL {
                        let mut cursor = 1;
                        let id = get_varint(payload, &mut cursor)? as usize;
                        if pooled[id] != u32::MAX {
                            events.push(lists::POOLED);
                            put_varint(events, u64::from(pooled[id]));
                        } else {
                            let literal = pending.literal_bytes(id as u32);
                            events.push(lists::LITERAL);
                            put_varint(events, literal.len() as u64);
                            events.extend_from_slice(literal);
                        }
                    } else {
                        events.extend_from_slice(payload);
                    }
                    continue;
                }
                let mut cut = 0;
                let code = get_wide(payload, &mut cut)?;
                let primary = &payload[..cut];
                events.extend_from_slice(primary);
                if dual {
                    put_varint(raw_events, u64::from(gap));
                    raw_events.extend_from_slice(&payload[cut..]);
                }
                if column.choice != Choice::Raw {
                    put_varint(second_events, u64::from(gap));
                    let second = if code & 1 == 0 {
                        let now = unzig((code >> 1) as u64);
                        let second = now.wrapping_sub(last_difference);
                        last_difference = now;
                        u128::from(zig(second)) << 1
                    } else {
                        last_difference = 0;
                        code
                    };
                    put_wide(second_events, second);
                }
                match &mut open {
                    Some((held_gap, held_at, held_end, times)) if *held_gap == gap && buffer[*held_at as usize..*held_end as usize] == *primary => {
                        *times += 1;
                    }
                    _ => {
                        if let Some((gap, at, end, times)) = open.replace((gap, event.at, event.at + cut as u32, 0)) {
                            put_varint(runs, u64::from(gap));
                            runs.extend_from_slice(&buffer[at as usize..end as usize]);
                            put_varint(runs, times);
                            run_count += 1;
                        }
                    }
                }
            }
            if let Some((gap, at, end, times)) = open {
                put_varint(runs, u64::from(gap));
                runs.extend_from_slice(&buffer[at as usize..end as usize]);
                put_varint(runs, times);
                run_count += 1;
            }
            let count = (stop - start) as u64;
            let gap = slot - previous_slot;
            put_track(directory, gap, count, born, first, events.len() - events_start);
            put_track(run_directory, gap, run_count, born, first, runs.len() - runs_start);
            put_track(raw_directory, gap, count, born, first, raw_events.len() - raw_start);
            put_track(second_directory, gap, count, born, first, second_events.len() - second_start);
            previous_slot = slot;
            tracks += 1;
            total_events += count;
            total_runs += run_count;
            Ok(())
        })?;
        at = end;
    }
    scratch.found = found;
    let Scratch { directory, events, runs, run_directory, raw_events, raw_directory, second_events, second_directory, pool, inserted, .. } = scratch;
    let (mut model, mut raw_won) = (DIFFERENCES, false);
    let (mut chosen_directory, mut chosen_events): (&[u8], &[u8]) = (directory, events);
    if column.list {
    } else if column.choice == Choice::Raw {
        model = RAW;
    } else {
        if total_runs * settings.runs <= total_events && total_events > 0 {
            model = RUNS;
            chosen_directory = run_directory;
            chosen_events = runs;
        }
        if chosen_events.len() >= 256 {
            let mut best = estimate(chosen_directory, chosen_events);
            let mut candidates: Vec<(u8, &[u8], &[u8])> = vec![(SECOND, second_directory, second_events)];
            if dual {
                candidates.push((RAW, raw_directory, raw_events));
            }
            for (candidate, other_directory, other_events) in candidates {
                let size = estimate(other_directory, other_events);
                if size < best {
                    best = size;
                    model = candidate;
                    chosen_directory = other_directory;
                    chosen_events = other_events;
                }
            }
        }
        raw_won = model == RAW;
    }
    let mut bytes = {
        let _label = label("previous parts");
        Vec::with_capacity(chosen_directory.len() + chosen_events.len() + pool.len() + inserted.len() + 24)
    };
    bytes.push(model);
    put_varint(&mut bytes, tracks);
    put_varint(&mut bytes, chosen_directory.len() as u64);
    put_varint(&mut bytes, pool.len() as u64);
    put_varint(&mut bytes, inserted.len() as u64);
    bytes.extend_from_slice(chosen_directory);
    bytes.extend_from_slice(pool);
    bytes.extend_from_slice(inserted);
    bytes.extend_from_slice(chosen_events);
    let held = scratch.found.capacity() * size_of::<Found>()
        + [&scratch.directory, &scratch.events, &scratch.runs, &scratch.run_directory, &scratch.raw_events, &scratch.raw_directory, &scratch.second_events, &scratch.second_directory]
            .iter()
            .map(|held| held.capacity())
            .sum::<usize>()
        + bytes.capacity();
    scratch.peak = scratch.peak.max(held);
    Ok((bytes, model, raw_won))
}

fn marks(pending: &Pending, births: &[(u32, u16)], first: u16, scratch: &mut Scratch) -> io::Result<Vec<u8>> {
    if pending.events > u64::from(u32::MAX) {
        return Err(invalid("A part holds more marks than a marks stream counts"));
    }
    let Scratch { counts, hours, directory, .. } = scratch;
    counts.clear();
    each_mark(&pending.buffer, |slot, _| {
        let slot = slot as usize;
        if slot >= counts.len() {
            counts.resize(slot + 1, 0);
        }
        counts[slot] += 1;
    })?;
    let mut total = 0_u32;
    for count in counts.iter_mut() {
        let held = *count;
        *count = total;
        total += held;
    }
    hours.clear();
    hours.resize(total as usize, 0);
    each_mark(&pending.buffer, |slot, hour| {
        let at = &mut counts[slot as usize];
        hours[*at as usize] = hour;
        *at += 1;
    })?;
    directory.clear();
    let (mut tracks, mut previous_slot, mut length) = (0_u64, 0_u32, 0_usize);
    mark_tracks(counts, hours, births, |slot, held, born| {
        let mut previous = born.unwrap_or(first);
        let bytes: usize = held
            .iter()
            .map(|&hour| {
                let gap = hour - previous;
                previous = hour;
                varint_len(u64::from(gap))
            })
            .sum();
        put_track(directory, slot - previous_slot, held.len() as u64, born, first, bytes);
        previous_slot = slot;
        tracks += 1;
        length += bytes;
    });
    let mut bytes = {
        let _label = label("previous parts");
        Vec::with_capacity(directory.len() + length + 24)
    };
    bytes.push(DIFFERENCES);
    put_varint(&mut bytes, tracks);
    put_varint(&mut bytes, directory.len() as u64);
    put_varint(&mut bytes, 0);
    put_varint(&mut bytes, 0);
    bytes.extend_from_slice(directory);
    mark_tracks(counts, hours, births, |_, held, born| {
        let mut previous = born.unwrap_or(first);
        for &hour in held {
            put_varint(&mut bytes, u64::from(hour - previous));
            previous = hour;
        }
    });
    scratch.peak = scratch.peak.max(scratch.room() + bytes.capacity());
    Ok(bytes)
}

fn put_option_status(out: &mut Vec<u8>, held: Option<(Status, Option<i32>)>) {
    match held {
        None => out.push(0),
        Some((status, sampled)) => {
            out.push(1);
            put_status(out, status);
            put_varint(out, sampled.map_or(0, |hour| u64::from(((hour << 1) ^ (hour >> 31)) as u32) + 1));
        }
    }
}

fn get_option_status(bytes: &[u8], at: &mut usize) -> io::Result<Option<(Status, Option<i32>)>> {
    let flag = *bytes.get(*at).ok_or_else(|| invalid("Resume state ends early"))?;
    *at += 1;
    if flag == 0 {
        return Ok(None);
    }
    let status = get_status(bytes, at)?;
    let sampled = match get_varint(bytes, at)? {
        0 => None,
        value => {
            let value = (value - 1) as u32;
            Some(((value >> 1) as i32) ^ -((value & 1) as i32))
        }
    };
    Ok(Some((status, sampled)))
}

impl Encoder {
    pub fn save(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, u64::from(((self.run_first << 1) ^ (self.run_first >> 31)) as u32));
        put_varint(&mut out, self.tables.len() as u64);
        for open in &self.tables {
            let rule = &open.rule;
            put_varint(&mut out, rule.events);
            put_varint(&mut out, u64::from(rule.blocks));
            put_varint(&mut out, u64::from(rule.since_whole));
            out.push(u8::from(rule.restored));
            put_varint(&mut out, u64::from(rule.whole_slots));
            put_varint(&mut out, rule.chunks.len() as u64);
            let mut previous = 0_u64;
            for chunks in &rule.chunks {
                put_varint(&mut out, chunks.cuts.len() as u64);
                chunks.cuts.iter().for_each(|&slot| put_varint(&mut out, u64::from(slot)));
                put_varint(&mut out, chunks.whole.len() as u64);
                chunks.whole.iter().for_each(|frame| put_frame(&mut out, frame, &mut previous));
                put_varint(&mut out, chunks.written.len() as u64);
                chunks.written.iter().for_each(|&word| put_varint(&mut out, word));
            }
            match &rule.last {
                None => out.push(0),
                Some(entry) => {
                    out.push(1);
                    put_restore(&mut out, entry, &mut previous);
                }
            }
            put_option_status(&mut out, rule.status);
            let mut listed: Vec<u32> = rule.whole_strings.iter().copied().collect();
            listed.sort_unstable();
            put_varint(&mut out, listed.len() as u64);
            let mut last_listed = 0_u32;
            for id in listed {
                put_varint(&mut out, u64::from(id - last_listed));
                last_listed = id;
            }
            for column in &open.columns {
                out.push(match column.choice {
                    Choice::Trying => 0,
                    Choice::Raw => 1,
                    Choice::Differences => 2,
                });
            }
            let mut named: Vec<u32> = open.named.iter().copied().collect();
            named.sort_unstable();
            put_varint(&mut out, named.len() as u64);
            let mut last = 0_u32;
            for id in named {
                put_varint(&mut out, u64::from(id - last));
                last = id;
            }
            match open.held_status {
                None => out.push(0),
                Some((status, read, sample)) => {
                    out.push(1);
                    put_status(&mut out, status);
                    out.push(u8::from(read));
                    put_varint(&mut out, sample);
                }
            }
        }
        out
    }

    pub fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut at = 0;
        let run_first = get_varint(bytes, &mut at)? as u32;
        self.run_first = ((run_first >> 1) as i32) ^ -((run_first & 1) as i32);
        if get_varint(bytes, &mut at)? as usize != self.tables.len() {
            return Err(invalid("Resume state of another schema"));
        }
        for open in &mut self.tables {
            let mut rule = Rule {
                events: get_varint(bytes, &mut at)?,
                blocks: get_varint(bytes, &mut at)? as u32,
                since_whole: get_varint(bytes, &mut at)? as u32,
                ..Rule::default()
            };
            rule.restored = *bytes.get(at).ok_or_else(|| invalid("Resume state ends early"))? != 0;
            at += 1;
            rule.whole_slots = get_varint(bytes, &mut at)? as u32;
            let mut previous = 0_u64;
            let streams = get_varint(bytes, &mut at)? as usize;
            if streams > bytes.len() {
                return Err(invalid("Resume state counts more restore streams than it holds"));
            }
            for _ in 0..streams {
                let mut chunks = Chunks::default();
                let count = |at: &mut usize| -> io::Result<u64> {
                    let count = get_varint(bytes, at)?;
                    if count > bytes.len() as u64 {
                        return Err(invalid("Resume state counts more chunks than it holds"));
                    }
                    Ok(count)
                };
                chunks.cuts = (0..count(&mut at)?).map(|_| get_varint(bytes, &mut at).map(|slot| slot as u32)).collect::<io::Result<_>>()?;
                chunks.whole = (0..count(&mut at)?).map(|_| get_frame(bytes, &mut at, &mut previous)).collect::<io::Result<_>>()?;
                chunks.written = (0..count(&mut at)?).map(|_| get_varint(bytes, &mut at)).collect::<io::Result<_>>()?;
                rule.chunks.push(chunks);
            }
            let flag = *bytes.get(at).ok_or_else(|| invalid("Resume state ends early"))?;
            at += 1;
            if flag != 0 {
                rule.last = Some(get_restore(bytes, &mut at, &mut previous)?);
            }
            rule.status = get_option_status(bytes, &mut at)?;
            let mut id = 0_u32;
            for _ in 0..get_varint(bytes, &mut at)? {
                id += get_varint(bytes, &mut at)? as u32;
                rule.whole_strings.insert(id);
            }
            if let Some(last) = &mut rule.last {
                last.log = open.log;
            }
            open.rule = rule;
            for column in &mut open.columns {
                column.choice = match *bytes.get(at).ok_or_else(|| invalid("Resume state ends early"))? {
                    0 => Choice::Trying,
                    1 => Choice::Raw,
                    _ => Choice::Differences,
                };
                at += 1;
            }
            open.named.clear();
            let mut id = 0_u32;
            for _ in 0..get_varint(bytes, &mut at)? {
                id += get_varint(bytes, &mut at)? as u32;
                open.named.insert(id);
            }
            let flag = *bytes.get(at).ok_or_else(|| invalid("Resume state ends early"))?;
            at += 1;
            open.held_status = if flag == 0 {
                None
            } else {
                let status = get_status(bytes, &mut at)?;
                let read = *bytes.get(at).ok_or_else(|| invalid("Resume state ends early"))? != 0;
                at += 1;
                Some((status, read, get_varint(bytes, &mut at)?))
            };
            open.status.clear();
            if let Some((status, read, sample)) = open.held_status {
                open.status.push((0, status, read, sample));
            }
            open.first = 0;
        }
        if at != bytes.len() {
            return Err(invalid("Resume state holds bytes past its end"));
        }
        Ok(())
    }
}
