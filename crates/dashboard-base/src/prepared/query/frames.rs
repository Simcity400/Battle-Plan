//! What sampled reads decode, shared between the queries of one source: a
//! block's frames as read for the tables asked of it, and one hour's
//! reconstructed frame with the union of the tables asked of it. A request
//! missing tables reads only those (a block keeps each read beside the earlier
//! ones; an hour joins them to its frame); concurrent requests for one block or
//! hour wait for a single read. Each table is read and reconstructed on its own,
//! so what a request receives is, table by table, exactly its own narrower read,
//! and it sees only the tables it asked for. What a join cannot reproduce
//! exactly (an error, a gap, a keyframe whose extra table would fail the
//! decoder) is read directly as before and not kept.
//!
//! The cache keeps at most [`CACHE_BYTES`] per source, counting everything an
//! entry keeps alive: each read's frames whole (every table's update, read or
//! not, and the hour's metrics), the rows they hold with every string, list and
//! nested value each row owns at its capacity, and the reasons of unavailable
//! tables. It evicts
//! the least recently used entry first; an entry larger than the budget serves
//! only the requests waiting on its read. A reader publishes its read and
//! accounts it before other requests may read the same block or hour.
//!
//! A request made without block decodes ([`super::progressive::without_block_decodes`])
//! takes what the cache holds and never waits on another request's read: it is
//! refused at once, to be answered where decodes are allowed.
use crate::{Source, metadata::check_cancel};
use replay::{container::BlockIndex, container::SeekResult};
use replay_data::rows::TABLE_COUNT;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{
    frame::DecodedFrame, frame::FrameDecoder, frame::RecordedFrame as Frame, frame::SharedView,
    frame::TableUpdate, frame::TableView,
};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard, atomic::AtomicBool},
    time::Duration,
};

/// Bytes kept per source: the newest hours' frames and blocks for the tables
/// several widgets read, not a recording's history.
pub const CACHE_BYTES: usize = 512 << 20;

/// How often a request waiting on another's read checks its cancellation.
const WAIT_CHECK: Duration = Duration::from_millis(50);

/// A source's cached reads: see the module documentation.
pub struct FrameCache {
    entries: BTreeMap<Key, Entry>,
    bytes: usize,
    /// Bytes kept at most: [`CACHE_BYTES`].
    budget: usize,
    clock: u64,
    /// Block reads and hour seeks made through the cache.
    reads: (usize, usize),
}

impl Default for FrameCache {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            budget: CACHE_BYTES,
            clock: 0,
            reads: (0, 0),
        }
    }
}

fn count(source: &Source, blocks: usize, hours: usize) {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    cache.reads.0 += blocks;
    cache.reads.1 += hours;
}

/// Block reads and hour seeks made through `source`'s cache so far.
#[cfg(any(test, feature = "test-support"))]
pub fn reads(source: &Source) -> (usize, usize) {
    lock(&source.state::<FrameCache>()).reads
}

/// Bytes `source`'s cache holds.
#[cfg(any(test, feature = "test-support"))]
pub fn held_bytes(source: &Source) -> usize {
    lock(&source.state::<FrameCache>()).bytes
}

/// Whether `source`'s cache accounts exactly what its entries hold: each
/// entry's bytes are its published value's, and the total is their sum.
#[cfg(any(test, feature = "test-support"))]
pub fn accounted(source: &Source) -> bool {
    let cache = source.state::<FrameCache>();
    let cache = lock(&cache);
    cache.bytes
        == cache
            .entries
            .values()
            .map(|entry| entry.bytes)
            .sum::<usize>()
        && cache.entries.values().all(|entry| {
            let state = lock(&entry.slot.state);
            entry.bytes == state.value.as_ref().map_or(0, |_| state.bytes)
        })
}

#[cfg(test)]
thread_local! {
    /// Run on this thread each time one of its reads is released.
    pub(crate) static ON_RELEASE: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Marks `hour`'s entry as read by another request until dropped.
#[cfg(test)]
pub(crate) fn hold_hour(source: &Source, branch: u64, hour: i32) -> impl Drop + '_ {
    let key = Key::Hour { branch, hour };
    let slot = slot(source, key).expect("this thread may decode");
    lock(&slot.state).reading = true;
    Reading { source, key, slot }
}

/// The number of entries `source`'s cache holds, filled or not.
#[cfg(test)]
pub(crate) fn entries(source: &Source) -> usize {
    lock(&source.state::<FrameCache>()).entries.len()
}

/// Keep at most `budget` bytes for `source` from now on.
#[cfg(any(test, feature = "test-support"))]
pub fn set_budget(source: &Source, budget: usize) {
    lock(&source.state::<FrameCache>()).budget = budget;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Block { branch: u64, offset: u64 },
    Hour { branch: u64, hour: i32 },
}

struct Entry {
    slot: Arc<Slot>,
    used: u64,
    bytes: usize,
}

#[derive(Default)]
struct Slot {
    state: Mutex<SlotState>,
    filled: Condvar,
}

#[derive(Default)]
struct SlotState {
    reading: bool,
    value: Option<Value>,
    /// What `value` keeps alive.
    bytes: usize,
}

#[derive(Clone)]
enum Value {
    Block(Arc<Vec<BlockRead>>),
    Hour(Arc<HourFrame>),
}

/// One read of a block: its frames with `tables` (sorted) selected.
#[derive(Clone)]
pub struct BlockRead {
    pub tables: Vec<TableId>,
    pub frames: Arc<Vec<Frame>>,
}

/// One hour's frame reconstructed with `tables` (sorted, the campaign included).
struct HourFrame {
    tables: Vec<TableId>,
    frame: DecodedFrame,
}

/// Exact, ascending hours, sharing block decodes and walking each selection
/// once per block. Unlike a window this reconstructs no unrequested output.
pub fn hours(
    source: &Source,
    branch: u64,
    hours: &[i32],
    tables: &[TableId],
    cancelled: &AtomicBool,
    mut visit: impl FnMut(i32, SeekResult) -> io::Result<()>,
) -> io::Result<()> {
    if hours.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(crate::invalid(
            "Requested hours must be distinct and ascending",
        ));
    }
    let mut active = None;
    let mut decoders = BTreeMap::new();
    for &hour in hours {
        check_cancel(cancelled)?;
        let block = source.replay_ref().index.iter().find(|block| {
            block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
        });
        let result = match block {
            Some(block) => {
                if active != Some(block.offset) {
                    decoders.clear();
                    active = Some(block.offset);
                }
                seek_with(source, branch, hour, tables, cancelled, |tables| {
                    let tables = sorted(&[tables, &[TableId::Campaign]].concat());
                    if !decoders.contains_key(&tables) {
                        decoders.insert(
                            tables.clone(),
                            BlockDecoder::new(source, block, &tables, cancelled)?,
                        );
                    }
                    decoders
                        .get_mut(&tables)
                        .expect("selected decoder")
                        .at(source, hour, cancelled)
                })?
            }
            None => seek(source, branch, hour, tables, cancelled)?,
        };
        visit(hour, result)?;
    }
    Ok(())
}

/// A bounded chronological read, sharing hour slots and table joins with `seek`.
/// Each selection's decoder advances once through the intersecting block.
pub fn window(
    source: &Source,
    branch: u64,
    first: i32,
    last: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Vec<DecodedFrame>> {
    let mut result = Vec::new();
    for block in source.replay_ref().index.iter().filter(|block| {
        block.branch == branch && block.last_hour >= first && block.first_hour <= last
    }) {
        let mut decoders = BTreeMap::new();
        for hour in block.first_hour.max(first)..=block.last_hour.min(last) {
            check_cancel(cancelled)?;
            if source
                .replay_ref()
                .gaps
                .iter()
                .any(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
            {
                continue;
            }
            let frame = seek_with(source, branch, hour, tables, cancelled, |tables| {
                let tables = sorted(&[tables, &[TableId::Campaign]].concat());
                if !decoders.contains_key(&tables) {
                    let decoder = BlockDecoder::new(source, block, &tables, cancelled)?;
                    decoders.insert(tables.clone(), decoder);
                }
                decoders
                    .get_mut(&tables)
                    .expect("selected decoder")
                    .at(source, hour, cancelled)
            })?;
            if let SeekResult::Frame(frame) = frame {
                result.push(*frame);
            }
        }
    }
    Ok(result)
}

/// Request-local decoder state; reconstructed hours are retained only by the
/// shared hour cache. Different windows never lock each other's block walk.
struct BlockDecoder {
    branch: u64,
    reads: Vec<BlockRead>,
    decoders: Vec<(FrameDecoder, Vec<TableId>)>,
    position: usize,
    frame: Option<DecodedFrame>,
    /// Once a frame's campaign differs, every later hour of the block is that gap, as a seek finds.
    mismatch: Option<String>,
}

impl BlockDecoder {
    fn new(
        source: &Source,
        index: &BlockIndex,
        tables: &[TableId],
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let reads = match block(source, index, tables, cancelled)? {
            Some(reads) => reads,
            None => vec![BlockRead {
                tables: tables.to_vec(),
                frames: Arc::new(
                    source
                        .replay_ref()
                        .read_block_recorded_selected(index, tables)?,
                ),
            }],
        };
        let decoders = reads
            .iter()
            .map(|read| {
                let selected: Vec<_> = read
                    .tables
                    .iter()
                    .copied()
                    .filter(|table| tables.binary_search(table).is_ok())
                    .collect();
                (FrameDecoder::for_tables(&selected), selected)
            })
            .collect();
        Ok(Self {
            branch: index.branch,
            reads,
            decoders,
            position: 0,
            frame: None,
            mismatch: None,
        })
    }

    fn at(&mut self, source: &Source, hour: i32, cancelled: &AtomicBool) -> io::Result<SeekResult> {
        if let Some(reason) = &self.mismatch {
            return Ok(SeekResult::Gap {
                branch: self.branch,
                hour,
                reason: reason.clone(),
            });
        }
        if self.frame.as_ref().is_some_and(|frame| frame.hour != hour) {
            self.frame = None;
        }
        while self.position < self.reads[0].frames.len()
            && self.reads[0].frames[self.position].hour <= hour
        {
            check_cancel(cancelled)?;
            let mut frame: Option<DecodedFrame> = None;
            for ((decoder, selected), read) in self.decoders.iter_mut().zip(&self.reads) {
                let decoded = decoder.advance(&read.frames[self.position])?;
                if let Err(error) = crate::metadata::validate_campaign(decoded, source.replay_ref())
                {
                    self.position = self.reads[0].frames.len();
                    self.frame = None;
                    let reason = error.to_string();
                    self.mismatch = Some(reason.clone());
                    return Ok(SeekResult::Gap {
                        branch: self.branch,
                        hour,
                        reason,
                    });
                }
                if decoded.hour != hour {
                    continue;
                }
                if let Some(frame) = &mut frame {
                    for table in selected {
                        frame.tables[table.index()] = decoded.tables[table.index()].clone();
                    }
                } else {
                    frame = Some(decoded.clone());
                }
            }
            self.frame = frame;
            self.position += 1;
        }
        match &self.frame {
            Some(frame) if frame.hour == hour => Ok(SeekResult::Frame(Box::new(frame.clone()))),
            // An hour the block holds no frame of: the gap a seek names.
            _ => Ok(SeekResult::Gap {
                branch: self.branch,
                hour,
                reason: source.replay_ref().gap_reason(self.branch, hour)?,
            }),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn sorted(tables: &[TableId]) -> Vec<TableId> {
    let mut tables = tables.to_vec();
    tables.sort_unstable();
    tables.dedup();
    tables
}

fn missing(held: &[TableId], wanted: &[TableId]) -> Vec<TableId> {
    wanted
        .iter()
        .copied()
        .filter(|table| held.binary_search(table).is_err())
        .collect()
}

fn joined(held: &[TableId], added: &[TableId]) -> Vec<TableId> {
    let mut tables = [held, added].concat();
    tables.sort_unstable();
    tables.dedup();
    tables
}

/// The entry of `key`, marked used now. A request that may not decode
/// ([`super::progressive::without_block_decodes`]) and finds none is refused
/// without one being made: it could only wait for a read or read itself.
fn slot(source: &Source, key: Key) -> io::Result<Arc<Slot>> {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    cache.clock += 1;
    let clock = cache.clock;
    if !cache.entries.contains_key(&key) {
        super::progressive::decodes_allowed()?;
    }
    let entry = cache.entries.entry(key).or_insert_with(|| Entry {
        slot: Arc::default(),
        used: 0,
        bytes: 0,
    });
    entry.used = clock;
    Ok(Arc::clone(&entry.slot))
}

/// Remove `key`'s entry when it is `slot` and neither holds a value nor is
/// being read, so no empty entry outlives the request that made it.
fn discard(source: &Source, key: Key, slot: &Arc<Slot>) {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    let empty = cache.entries.get(&key).is_some_and(|entry| {
        Arc::ptr_eq(&entry.slot, slot) && {
            let state = lock(&slot.state);
            state.value.is_none() && !state.reading
        }
    });
    if empty {
        cache.entries.remove(&key);
    }
}

/// Account `key`'s filled `bytes` (none: nothing kept, so the entry goes) and
/// evict least recently used entries beyond the budget, this one included.
fn settle(source: &Source, key: Key, slot: &Arc<Slot>, bytes: Option<usize>) {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    let cache = &mut *cache;
    let Some(entry) = cache
        .entries
        .get_mut(&key)
        .filter(|entry| Arc::ptr_eq(&entry.slot, slot))
    else {
        return;
    };
    cache.bytes -= entry.bytes;
    match bytes {
        Some(bytes) => {
            entry.bytes = bytes;
            cache.bytes += bytes;
        }
        None => {
            cache.entries.remove(&key);
        }
    }
    while cache.bytes > cache.budget {
        let Some((&oldest, _)) = cache
            .entries
            .iter()
            .filter(|(_, entry)| entry.bytes > 0)
            .min_by_key(|(_, entry)| entry.used)
        else {
            break;
        };
        let entry = cache.entries.remove(&oldest).expect("listed entry");
        cache.bytes -= entry.bytes;
    }
}

/// `key`'s read under way: clears `reading`, wakes waiters and removes the
/// entry if it was left empty, however the read ends.
struct Reading<'a> {
    source: &'a Source,
    key: Key,
    slot: Arc<Slot>,
}

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        lock(&self.slot.state).reading = false;
        self.slot.filled.notify_all();
        discard(self.source, self.key, &self.slot);
        #[cfg(test)]
        ON_RELEASE.with_borrow(|hook| {
            if let Some(hook) = hook {
                hook();
            }
        });
    }
}

/// End `reading`, `key`'s read: publish `value` with the bytes it keeps alive
/// (none: the read keeps nothing, and an entry holding nothing goes) and
/// account it, before any other request may read the entry.
/// A whole map node for one entry of `T`: a node holds at least one entry and
/// reserves room for eleven, its metadata and an internal node's twelve edges.
const fn map_node_bytes<T>() -> usize {
    11 * std::mem::size_of::<T>() + 16 * std::mem::size_of::<usize>()
}

/// What an entry holds beyond its value: its shared slot (lock, condition
/// and state) and a whole node of the cache's map, a bound on its place there.
const ENTRY_BYTES: usize = 2 * std::mem::size_of::<usize>()
    + std::mem::size_of::<Slot>()
    + map_node_bytes::<(Key, Entry)>();

fn publish(
    source: &Source,
    key: Key,
    slot: &Arc<Slot>,
    reading: Reading<'_>,
    value: Option<(Value, usize)>,
) {
    let bytes = {
        let mut state = lock(&slot.state);
        match value {
            Some((value, bytes)) => {
                let bytes = bytes + ENTRY_BYTES;
                state.value = Some(value);
                state.bytes = bytes;
                Some(bytes)
            }
            None => state.value.is_some().then_some(state.bytes),
        }
    };
    settle(source, key, slot, bytes);
    drop(reading);
}

/// The held value when it covers the request; otherwise this request reads
/// (`reading` set, the held value to join) once no other read is under way.
/// Without block decodes, an uncovered request is refused rather than waiting
/// or reading. A refused or cancelled request leaves no empty entry.
fn wait_or_read<'a, T>(
    source: &'a Source,
    key: Key,
    slot: Arc<Slot>,
    covered: impl Fn(&Value) -> Option<T>,
    cancelled: &AtomicBool,
) -> io::Result<Result<T, (Reading<'a>, Option<Value>)>> {
    let waited = (|| {
        let mut state = lock(&slot.state);
        loop {
            if let Some(found) = state.value.as_ref().and_then(&covered) {
                return Ok(Ok(found));
            }
            super::progressive::decodes_allowed()?;
            if !state.reading {
                break;
            }
            state = slot
                .filled
                .wait_timeout(state, WAIT_CHECK)
                .unwrap_or_else(|error| error.into_inner())
                .0;
            check_cancel(cancelled)?;
        }
        state.reading = true;
        Ok(Err(state.value.clone()))
    })();
    match waited {
        Ok(Ok(found)) => Ok(Ok(found)),
        Ok(Err(held)) => Ok(Err((Reading { source, key, slot }, held))),
        Err(error) => {
            discard(source, key, &slot);
            Err(error)
        }
    }
}

/// What a table's rows keep alive: their allocation and, row by row, every
/// string, list and nested value they own at its capacity
/// ([`TableRows::heap_bytes`]).
fn rows_bytes(rows: &TableRows) -> usize {
    rows.heap_bytes()
}

/// What an update keeps alive beyond its place in its frame.
pub(crate) fn update_bytes(update: &TableUpdate) -> usize {
    match update {
        TableUpdate::Snapshot(rows) | TableUpdate::Truncated { rows, .. } => rows_bytes(rows),
        TableUpdate::Delta {
            changed, deleted, ..
        } => {
            changed.capacity() * std::mem::size_of::<replay_data::rows::DeltaRow>()
                + changed
                    .iter()
                    .map(|row| row.values.capacity())
                    .sum::<usize>()
                + deleted.capacity() * std::mem::size_of::<u128>()
        }
        TableUpdate::Unchanged { .. } => 0,
        TableUpdate::Unavailable { reason } | TableUpdate::Incompatible { reason, .. } => {
            reason.capacity()
        }
    }
}

/// What a read's frames keep alive: each frame whole (every table's update
/// and the hour's metrics) and what each update holds.
fn frames_bytes(frames: &Vec<Frame>) -> usize {
    let mut bytes = crate::prepared::prepass::HeldBytes::default();
    bytes.add(frames.capacity() * size_of::<Frame>());
    for frame in frames {
        bytes.heap(frame);
    }
    bytes.total() as usize
}

/// What one table's view keeps alive: its shared allocation and its rows or reason.
pub(crate) fn view_bytes(view: &TableView) -> usize {
    let held = match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows_bytes(rows),
        TableView::Unavailable { reason } => reason.capacity(),
    };
    std::mem::size_of::<TableView>() + 2 * std::mem::size_of::<usize>() + held
}

/// A keyframe the decoder accepts whatever tables it reconstructs.
fn independent_keyframe(frame: &Frame) -> bool {
    !frame.keyframe
        || frame.tables.iter().all(|update| {
            matches!(
                update,
                TableUpdate::Snapshot(_)
                    | TableUpdate::Unavailable { .. }
                    | TableUpdate::Incompatible { .. }
            )
        })
}

/// `block`'s reads covering `tables`: each read's tables (sorted) and frames,
/// as `Replay::read_block_selected(block, read tables)` returned them. Every
/// read holds the block's frames in order; together the reads hold `tables`.
/// `None` when the cache cannot answer exactly and the caller reads directly.
pub fn block(
    source: &Source,
    block: &BlockIndex,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<BlockRead>>> {
    let wanted = sorted(tables);
    let key = Key::Block {
        branch: block.branch,
        offset: block.offset,
    };
    let slot = slot(source, key)?;
    let covering = |reads: &[BlockRead]| {
        let held = reads
            .iter()
            .flat_map(|read| read.tables.iter().copied())
            .collect::<std::collections::BTreeSet<_>>();
        wanted.iter().all(|table| held.contains(table)).then(|| {
            reads
                .iter()
                .filter(|read| {
                    read.tables
                        .iter()
                        .any(|table| wanted.binary_search(table).is_ok())
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let (reading, held) = match wait_or_read(
        source,
        key,
        Arc::clone(&slot),
        |value| match value {
            Value::Block(reads) => covering(reads),
            Value::Hour(_) => None,
        },
        cancelled,
    )? {
        Ok(reads) => return Ok(Some(reads)),
        Err(read) => read,
    };
    let held = match held {
        Some(Value::Block(reads)) => reads,
        _ => Arc::new(Vec::new()),
    };
    let have = held
        .iter()
        .flat_map(|read| read.tables.iter().copied())
        .collect::<Vec<_>>();
    let added = missing(&sorted(&have), &wanted);
    count(source, 1, 0);
    let frames = source
        .replay_ref()
        .read_block_recorded_selected(block, &added);
    // Every read lists the same frames; a keyframe the decoder would refuse
    // with these tables selected is read with the request's tables alone.
    let usable = frames.ok().filter(|frames| {
        frames.iter().all(independent_keyframe)
            && held.first().is_none_or(|first| {
                first.frames.len() == frames.len()
                    && first.frames.iter().zip(frames).all(|(held, read)| {
                        (held.branch, held.hour, held.keyframe, &held.metrics)
                            == (read.branch, read.hour, read.keyframe, &read.metrics)
                    })
            })
    });
    let Some(frames) = usable else {
        publish(source, key, &slot, reading, None);
        return Ok(None);
    };
    let bytes = frames_bytes(&frames);
    let mut reads = (*held).clone();
    reads.push(BlockRead {
        tables: added,
        frames: Arc::new(frames),
    });
    let answer = covering(&reads).expect("the reads hold every wanted table");
    // The held reads' frames stay accounted as they were; the containers
    // holding every read are accounted again as they now are.
    let held_bytes = lock(&slot.state).bytes;
    // The held charge includes its entry's bytes, which publishing adds again.
    let held_containers = if held.is_empty() {
        0
    } else {
        block_containers(&held) + ENTRY_BYTES
    };
    let value_bytes = held_bytes.saturating_sub(held_containers) + bytes + block_containers(&reads);
    let value = (Value::Block(Arc::new(reads)), value_bytes);
    publish(source, key, &slot, reading, Some(value));
    Ok(Some(answer))
}

/// One hour's frame reconstructed with `tables`, exactly as
/// `Replay::seek_selected_with(branch, hour, Some(tables), ..)` answers.
pub fn seek(
    source: &Source,
    branch: u64,
    hour: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<SeekResult> {
    seek_with(source, branch, hour, tables, cancelled, |tables| {
        count(source, 0, 1);
        source
            .replay_ref()
            .seek_selected_with(branch, hour, Some(tables), || check_cancel(cancelled))
    })
}

/// Coalesce one hour's reconstruction, join only missing tables and publish
/// before releasing its slot. The supplied reader may advance a window decoder.
fn seek_with(
    source: &Source,
    branch: u64,
    hour: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
    mut read: impl FnMut(&[TableId]) -> io::Result<SeekResult>,
) -> io::Result<SeekResult> {
    // A seek reconstructs the campaign with every selection.
    let wanted = sorted(&[tables, &[TableId::Campaign]].concat());
    let key = Key::Hour { branch, hour };
    let slot = slot(source, key)?;
    let covered = |value: &Value| match value {
        Value::Hour(held) if missing(&held.tables, &wanted).is_empty() => masked(held, &wanted),
        _ => None,
    };
    let (reading, held) = match wait_or_read(source, key, Arc::clone(&slot), covered, cancelled)? {
        Ok(frame) => return Ok(SeekResult::Frame(Box::new(frame))),
        Err(read) => read,
    };
    let Some(Value::Hour(held)) = held else {
        // Nothing held: this read is the request's own.
        let result = read(tables);
        let kept = match &result {
            Ok(SeekResult::Frame(frame)) => {
                let kept = HourFrame {
                    tables: wanted,
                    frame: (**frame).clone(),
                };
                let bytes = hour_bytes(&kept);
                Some((Value::Hour(Arc::new(kept)), bytes))
            }
            _ => None,
        };
        publish(source, key, &slot, reading, kept);
        return result;
    };
    let added = missing(&held.tables, &wanted);
    let joined_frame = match (added.is_empty(), &held) {
        (false, held) => match read(&added) {
            Ok(SeekResult::Frame(frame))
                if (frame.branch, frame.hour, &frame.metrics)
                    == (held.frame.branch, held.frame.hour, &held.frame.metrics) =>
            {
                let mut joined_frame = held.frame.clone();
                for table in &added {
                    joined_frame.tables[table.index()] = frame.tables[table.index()].clone();
                }
                Some(HourFrame {
                    tables: joined(&held.tables, &added),
                    frame: joined_frame,
                })
            }
            _ => None,
        },
        (true, _) => None,
    };
    let answer = joined_frame.as_ref().and_then(|kept| masked(kept, &wanted));
    let (Some(kept), Some(answer)) = (joined_frame, answer) else {
        // A gap, an error or a join that cannot be answered: as before, and the
        // held frame stays as it was.
        publish(source, key, &slot, reading, None);
        return read(tables);
    };
    let bytes = hour_bytes(&kept);
    publish(
        source,
        key,
        &slot,
        reading,
        Some((Value::Hour(Arc::new(kept)), bytes)),
    );
    Ok(SeekResult::Frame(Box::new(answer)))
}

/// What holds a block's reads beyond their frames: the shared list, each
/// read's table selection and the allocation holding its frames.
fn block_containers(reads: &Vec<BlockRead>) -> usize {
    const SHARED: usize = 2 * std::mem::size_of::<usize>();
    SHARED
        + std::mem::size_of::<Vec<BlockRead>>()
        + reads.capacity() * std::mem::size_of::<BlockRead>()
        + reads
            .iter()
            .map(|read| {
                read.tables.capacity() * std::mem::size_of::<TableId>()
                    + SHARED
                    + std::mem::size_of::<Vec<Frame>>()
            })
            .sum::<usize>()
}

/// What a held hour keeps alive: its frame, its table selection, the
/// allocation holding them, and the views of the tables it reconstructed
/// (every other table shares the one unselected view).
fn hour_bytes(held: &HourFrame) -> usize {
    2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<HourFrame>()
        + held.tables.capacity() * std::mem::size_of::<TableId>()
        + held
            .tables
            .iter()
            .map(|table| view_bytes(&held.frame.tables[table.index()]))
            .sum::<usize>()
}

/// `held`'s frame with only `wanted` reconstructed, the rest unselected exactly as
/// a narrower seek leaves them; none when no unselected view is at hand to copy.
fn masked(held: &HourFrame, wanted: &[TableId]) -> Option<DecodedFrame> {
    let unselected: Option<&SharedView> = TableId::ALL
        .iter()
        .copied()
        .find(|table| held.tables.binary_search(table).is_err())
        .map(|table| &held.frame.tables[table.index()]);
    let mut frame = held.frame.clone();
    for table in TableId::ALL {
        if wanted.binary_search(&table).is_err() && held.tables.binary_search(&table).is_ok() {
            frame.tables[table.index()] = unselected?.clone();
        }
    }
    debug_assert_eq!(frame.tables.len(), TABLE_COUNT);
    Some(frame)
}
