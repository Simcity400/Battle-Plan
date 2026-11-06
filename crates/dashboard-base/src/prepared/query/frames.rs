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
//! An entry lives while a request uses or reads it; besides, each branch keeps
//! the hour it reconstructed last and the block it read last, which the
//! widgets showing that hour read next. A reader publishes its read before
//! other requests may read the same block or hour.
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

/// How often a request waiting on another's read checks its cancellation.
const WAIT_CHECK: Duration = Duration::from_millis(50);

/// A source's cached reads: see the module documentation.
pub struct FrameCache {
    entries: BTreeMap<Key, Entry>,
    /// Each branch's latest hour (false) and block (true).
    latest: BTreeMap<(u64, bool), Key>,
    /// Whether the latest are kept; a test turns it off to see what requests alone hold.
    keep: bool,
    /// Block reads and hour seeks made through the cache.
    reads: (usize, usize),
}

impl Default for FrameCache {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            latest: BTreeMap::new(),
            keep: true,
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

/// The entries `source`'s cache holds a value of.
#[cfg(any(test, feature = "test-support"))]
pub fn held(source: &Source) -> usize {
    let cache = source.state::<FrameCache>();
    let cache = lock(&cache);
    cache
        .entries
        .values()
        .filter(|entry| lock(&entry.slot.state).value.is_some())
        .count()
}

#[cfg(test)]
thread_local! {
    /// Run on this thread each time one of its reads is released.
    pub(crate) static ON_RELEASE: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
    /// Run on this thread each time it starts waiting on another request's read.
    pub(crate) static ON_WAIT: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
    /// Tables whose block decodes on this thread fail as damaged, the first selected one named.
    pub(crate) static DAMAGED: std::cell::RefCell<Vec<TableId>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The hour whose next block decoder advance on this thread fails once, its frame consumed.
    pub(crate) static FAIL_ADVANCE: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
    /// Tables whose block reads on this thread cannot be kept, as an unreadable block's.
    pub(crate) static UNCACHEABLE: std::cell::RefCell<Vec<TableId>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Run on this thread each time it has leased a block's entry, before using it.
    pub(crate) static ON_LEASED: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
    /// Run on this thread each time it starts reading a block itself.
    pub(crate) static ON_READ: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Marks `hour`'s entry as read by another request until dropped.
#[cfg(test)]
pub(crate) fn hold_hour(source: &Source, branch: u64, hour: i32) -> impl Drop + '_ {
    hold(source, Key::Hour { branch, hour })
}

/// Marks `block`'s entry as read by another request until dropped.
#[cfg(test)]
pub(crate) fn hold_block<'a>(source: &'a Source, block: &BlockIndex) -> impl Drop + 'a {
    hold(
        source,
        Key::Block {
            branch: block.branch,
            offset: block.offset,
        },
    )
}

#[cfg(test)]
fn hold(source: &Source, key: Key) -> Reading<'_> {
    let (slot, _lease) = leased_slot(source, key).expect("this thread may decode");
    lock(&slot.state).reading = true;
    Reading { source, key, slot }
}

/// The number of entries `source`'s cache holds, filled or not.
#[cfg(test)]
pub(crate) fn entries(source: &Source) -> usize {
    lock(&source.state::<FrameCache>()).entries.len()
}

/// Whether `source` keeps each branch's latest hour and block from now on.
#[cfg(any(test, feature = "test-support"))]
pub fn keep_latest(source: &Source, keep: bool) {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    cache.keep = keep;
    let latest: Vec<Key> = cache.latest.values().copied().collect();
    for key in latest {
        release(&mut cache, key);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Block { branch: u64, offset: u64 },
    Hour { branch: u64, hour: i32 },
}

impl Key {
    /// Which of its branch's latest it may be.
    fn latest(self) -> (u64, bool) {
        match self {
            Self::Block { branch, .. } => (branch, true),
            Self::Hour { branch, .. } => (branch, false),
        }
    }
}

struct Entry {
    slot: Arc<Slot>,
    /// Requests using its value meanwhile ([`Lease`]); a leased entry stays.
    leases: usize,
}

/// A request's use of an entry's value: the entry stays held until it ends.
struct Lease<'a> {
    source: &'a Source,
    key: Key,
    slot: Arc<Slot>,
}

impl<'a> Lease<'a> {
    /// A lease on `key`'s entry as it is now, if there is one.
    fn of(source: &'a Source, key: Key) -> Option<Self> {
        let cache = source.state::<FrameCache>();
        let mut cache = lock(&cache);
        let entry = cache.entries.get_mut(&key)?;
        entry.leases += 1;
        Some(Self {
            source,
            key,
            slot: Arc::clone(&entry.slot),
        })
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let cache = self.source.state::<FrameCache>();
        let mut cache = lock(&cache);
        if let Some(entry) = cache
            .entries
            .get_mut(&self.key)
            .filter(|entry| Arc::ptr_eq(&entry.slot, &self.slot))
        {
            entry.leases -= 1;
            if entry.leases == 0 {
                release(&mut cache, self.key);
            }
        }
    }
}

/// Remove `key`'s entry once nothing uses or reads it, unless it is its branch's latest kept.
fn release(cache: &mut FrameCache, key: Key) {
    if cache.keep && cache.latest.get(&key.latest()) == Some(&key) {
        return;
    }
    let idle = cache
        .entries
        .get(&key)
        .is_some_and(|entry| entry.leases == 0 && !lock(&entry.slot.state).reading);
    if idle {
        cache.entries.remove(&key);
    }
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
                seek_with(
                    source,
                    branch,
                    hour,
                    tables,
                    cancelled,
                    &mut Decoders::new(source, block, hour, &mut decoders, cancelled),
                )?
            }
            None => seek(source, branch, hour, tables, cancelled)?,
        };
        visit(hour, result)?;
    }
    Ok(())
}

/// [`hours`] sought one at a time: each block's selections walk forward
/// while one branch's hours are sought in rising order. An hour before the
/// last one sought walks its block again.
pub struct Walk<'a> {
    source: &'a Source,
    branch: u64,
    active: Option<u64>,
    last: Option<i32>,
    decoders: BTreeMap<Vec<TableId>, BlockDecoder<'a>>,
}

impl<'a> Walk<'a> {
    pub fn new(source: &'a Source, branch: u64) -> Self {
        Self {
            source,
            branch,
            active: None,
            last: None,
            decoders: BTreeMap::new(),
        }
    }

    pub fn source(&self) -> &'a Source {
        self.source
    }

    pub fn branch(&self) -> u64 {
        self.branch
    }

    /// [`seek`]'s frame of `tables` at `hour`.
    pub fn seek(
        &mut self,
        hour: i32,
        tables: &[TableId],
        cancelled: &AtomicBool,
    ) -> io::Result<SeekResult> {
        check_cancel(cancelled)?;
        let (source, branch) = (self.source, self.branch);
        let Some(block) = source.replay_ref().index.iter().find(|block| {
            block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
        }) else {
            return seek(source, branch, hour, tables, cancelled);
        };
        if self.active != Some(block.offset) || self.last.is_some_and(|last| hour < last) {
            self.decoders.clear();
            self.active = Some(block.offset);
        }
        self.last = Some(hour);
        let found = seek_with(
            source,
            branch,
            hour,
            tables,
            cancelled,
            &mut Decoders::new(source, block, hour, &mut self.decoders, cancelled),
        );
        // A failed advance can leave a decoder without its baseline: walk the block afresh.
        if found.is_err() {
            self.decoders.clear();
            self.active = None;
            self.last = None;
        }
        found
    }

    /// [`window`]'s frames, through this walk.
    pub fn window(
        &mut self,
        first: i32,
        last: i32,
        tables: &[TableId],
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<DecodedFrame>> {
        let (source, branch) = (self.source, self.branch);
        let mut result = Vec::new();
        for block in source.replay_ref().index.iter().filter(|block| {
            block.branch == branch && block.last_hour >= first && block.first_hour <= last
        }) {
            for hour in block.first_hour.max(first)..=block.last_hour.min(last) {
                check_cancel(cancelled)?;
                if source.replay_ref().gaps.iter().any(|gap| {
                    gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour
                }) {
                    continue;
                }
                if let SeekResult::Frame(frame) = self.seek(hour, tables, cancelled)? {
                    result.push(*frame);
                }
            }
        }
        Ok(result)
    }
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
            let frame = seek_with(
                source,
                branch,
                hour,
                tables,
                cancelled,
                &mut Decoders::new(source, block, hour, &mut decoders, cancelled),
            )?;
            if let SeekResult::Frame(frame) = frame {
                result.push(*frame);
            }
        }
    }
    Ok(result)
}

/// How one hour's entry reads the tables it lacks.
trait HourRead {
    /// Whether reading `tables` waits on no other entry.
    fn ready(&self, tables: &[TableId]) -> bool;
    /// Called holding no entry, so its waits never keep others from the hour.
    fn prepare(&mut self, tables: &[TableId]) -> io::Result<()>;
    fn read(&mut self, tables: &[TableId]) -> io::Result<SeekResult>;
}

/// An hour sought from its block's keyframe: it waits on no other entry.
struct Direct<'a> {
    source: &'a Source,
    branch: u64,
    hour: i32,
    cancelled: &'a AtomicBool,
}

impl HourRead for Direct<'_> {
    fn ready(&self, _: &[TableId]) -> bool {
        true
    }

    fn prepare(&mut self, _: &[TableId]) -> io::Result<()> {
        Ok(())
    }

    fn read(&mut self, tables: &[TableId]) -> io::Result<SeekResult> {
        count(self.source, 0, 1);
        let cancelled = self.cancelled;
        self.source
            .replay_ref()
            .seek_selected_with(self.branch, self.hour, Some(tables), || {
                check_cancel(cancelled)
            })
    }
}

/// A walk's decoders of one block, one per selection; making one waits on the block's entry.
struct Decoders<'a, 'd, 'c> {
    source: &'a Source,
    block: &'a BlockIndex,
    hour: i32,
    decoders: &'d mut BTreeMap<Vec<TableId>, BlockDecoder<'a>>,
    /// Preparations that failed, each answered where its read would have been made.
    failed: BTreeMap<Vec<TableId>, io::Error>,
    cancelled: &'c AtomicBool,
}

impl<'a, 'd, 'c> Decoders<'a, 'd, 'c> {
    fn new(
        source: &'a Source,
        block: &'a BlockIndex,
        hour: i32,
        decoders: &'d mut BTreeMap<Vec<TableId>, BlockDecoder<'a>>,
        cancelled: &'c AtomicBool,
    ) -> Self {
        Self {
            source,
            block,
            hour,
            decoders,
            failed: BTreeMap::new(),
            cancelled,
        }
    }
}

impl HourRead for Decoders<'_, '_, '_> {
    fn ready(&self, tables: &[TableId]) -> bool {
        let tables = sorted(&[tables, &[TableId::Campaign]].concat());
        self.decoders.contains_key(&tables) || self.failed.contains_key(&tables)
    }

    fn prepare(&mut self, tables: &[TableId]) -> io::Result<()> {
        let tables = sorted(&[tables, &[TableId::Campaign]].concat());
        if !self.decoders.contains_key(&tables) && !self.failed.contains_key(&tables) {
            match BlockDecoder::new(self.source, self.block, &tables, self.cancelled) {
                Ok(decoder) => {
                    self.decoders.insert(tables, decoder);
                }
                Err(error) => {
                    self.failed.insert(tables, error);
                }
            }
        }
        Ok(())
    }

    fn read(&mut self, tables: &[TableId]) -> io::Result<SeekResult> {
        let tables = sorted(&[tables, &[TableId::Campaign]].concat());
        if let Some(error) = self.failed.remove(&tables) {
            return Err(error);
        }
        if !self.decoders.contains_key(&tables) {
            let decoder = BlockDecoder::new(self.source, self.block, &tables, self.cancelled)?;
            self.decoders.insert(tables.clone(), decoder);
        }
        let found = self
            .decoders
            .get_mut(&tables)
            .expect("selected decoder")
            .at(self.source, self.hour, self.cancelled);
        // A failed advance can leave the decoder without its baseline: a retry starts afresh.
        if found.is_err() {
            self.decoders.remove(&tables);
        }
        found
    }
}

/// Request-local decoder state; reconstructed hours are retained only by the
/// shared hour cache. Different windows never lock each other's block walk.
struct BlockDecoder<'a> {
    /// Keeps the block's entry held while this decoder reads it.
    _lease: Option<Lease<'a>>,
    branch: u64,
    reads: Vec<BlockRead>,
    decoders: Vec<(FrameDecoder, Vec<TableId>)>,
    position: usize,
    frame: Option<DecodedFrame>,
    /// Once a frame's campaign differs, every later hour of the block is that gap, as a seek finds.
    mismatch: Option<String>,
}

impl<'a> BlockDecoder<'a> {
    fn new(
        source: &'a Source,
        index: &BlockIndex,
        tables: &[TableId],
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        #[cfg(test)]
        if let Some(table) = DAMAGED
            .with_borrow(|damaged| tables.iter().find(|table| damaged.contains(table)).copied())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("damaged {table:?}"),
            ));
        }
        let (reads, lease) = match leased_block(source, index, tables, cancelled)? {
            Some((reads, lease)) => (reads, Some(lease)),
            None => (
                vec![BlockRead {
                    tables: tables.to_vec(),
                    frames: Arc::new(source.replay_ref().read_block_recorded_selected_with(
                        index,
                        tables,
                        || check_cancel(cancelled),
                    )?),
                }],
                None,
            ),
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
            _lease: lease,
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
            #[cfg(test)]
            if FAIL_ADVANCE.get() == Some(self.reads[0].frames[self.position].hour) {
                FAIL_ADVANCE.set(None);
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "injected advance failure",
                ));
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

/// `key`'s entry, made when there is none.
fn entry(cache: &mut FrameCache, key: Key) -> io::Result<&mut Entry> {
    if !cache.entries.contains_key(&key) {
        super::progressive::decodes_allowed()?;
    }
    Ok(cache.entries.entry(key).or_insert_with(|| Entry {
        slot: Arc::default(),
        leases: 0,
    }))
}

/// The entry of `key`, leased under the same lock, so no other request's
/// release can remove it first. A request that may not decode
/// ([`super::progressive::without_block_decodes`]) and finds none is refused
/// without one being made: it could only wait for a read or read itself.
fn leased_slot(source: &Source, key: Key) -> io::Result<(Arc<Slot>, Lease<'_>)> {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    let entry = entry(&mut cache, key)?;
    entry.leases += 1;
    let slot = Arc::clone(&entry.slot);
    let lease = Lease {
        source,
        key,
        slot: Arc::clone(&slot),
    };
    Ok((slot, lease))
}

/// Remove `key`'s entry when it is `slot` and nothing uses or reads it, unless it holds its
/// branch's latest kept value, so no entry outlives the requests using it otherwise.
fn discard(source: &Source, key: Key, slot: &Arc<Slot>) {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    if cache
        .entries
        .get(&key)
        .is_some_and(|entry| Arc::ptr_eq(&entry.slot, slot))
    {
        release(&mut cache, key);
    }
}

/// `key`'s read ended: one holding a value is its branch's latest, the one it replaces let go;
/// one holding nothing goes once unleased.
fn settle(source: &Source, key: Key, slot: &Arc<Slot>, kept: bool) {
    let cache = source.state::<FrameCache>();
    let mut cache = lock(&cache);
    if !cache
        .entries
        .get(&key)
        .is_some_and(|entry| Arc::ptr_eq(&entry.slot, slot))
    {
        return;
    }
    if kept {
        if let Some(previous) = cache.latest.insert(key.latest(), key)
            && previous != key
        {
            release(&mut cache, previous);
        }
    } else if cache.entries[&key].leases == 0 {
        cache.entries.remove(&key);
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

/// End `reading`, `key`'s read: publish `value` (none: the read keeps nothing, and an entry
/// holding nothing goes) before any other request may read the entry.
fn publish(
    source: &Source,
    key: Key,
    slot: &Arc<Slot>,
    reading: Reading<'_>,
    value: Option<Value>,
) {
    let kept = {
        let mut state = lock(&slot.state);
        if let Some(value) = value {
            state.value = Some(value);
        }
        state.value.is_some()
    };
    settle(source, key, slot, kept);
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
            #[cfg(test)]
            ON_WAIT.with_borrow(|hook| {
                if let Some(hook) = hook {
                    hook();
                }
            });
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
        TableUpdate::Unchanged { .. } | TableUpdate::NotRecorded { .. } => 0,
        TableUpdate::Unavailable { reason } | TableUpdate::Incompatible { reason, .. } => {
            reason.capacity()
        }
    }
}

/// What one table's view keeps alive: its shared allocation and its rows or reason.
pub(crate) fn view_bytes(view: &TableView) -> usize {
    let held = match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows_bytes(rows),
        TableView::Unavailable { reason } => reason.capacity(),
        TableView::NotRecorded { .. } => 0,
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
                    | TableUpdate::NotRecorded { .. }
                    | TableUpdate::Incompatible { .. }
            )
        })
}

/// `block`'s reads covering `tables`: each read's tables (sorted) and frames,
/// as `Replay::read_block_selected(block, read tables)` returned them. Every
/// read holds the block's frames in order; together the reads hold `tables`.
/// `None` when the cache cannot answer exactly and the caller reads directly.
/// The answer keeps the block's entry held until it is dropped.
pub fn block<'a>(
    source: &'a Source,
    block: &BlockIndex,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Option<LeasedReads<'a>>> {
    Ok(
        leased_block(source, block, tables, cancelled)?.map(|(reads, lease)| LeasedReads {
            reads,
            _lease: Some(lease),
        }),
    )
}

/// A block's reads, and for the cache's answer the lease that keeps its entry held while
/// they are used.
pub struct LeasedReads<'a> {
    reads: Vec<BlockRead>,
    _lease: Option<Lease<'a>>,
}

impl LeasedReads<'_> {
    /// Reads made outside the cache, which keeps nothing for them.
    pub fn uncached(reads: Vec<BlockRead>) -> Self {
        Self {
            reads,
            _lease: None,
        }
    }
}

impl std::ops::Deref for LeasedReads<'_> {
    type Target = [BlockRead];

    fn deref(&self) -> &[BlockRead] {
        &self.reads
    }
}

/// [`block`]'s reads with the lease keeping its entry, for a test to hold or release.
#[cfg(test)]
pub(crate) fn leased<'a>(
    source: &'a Source,
    block: &BlockIndex,
    tables: &[TableId],
) -> io::Result<Option<impl Drop + Send + use<'a>>> {
    Ok(leased_block(source, block, tables, &AtomicBool::new(false))?.map(|(_, lease)| lease))
}

/// [`block`], its entry leased to the caller so it stays held while used.
fn leased_block<'a>(
    source: &'a Source,
    block: &BlockIndex,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Option<(Vec<BlockRead>, Lease<'a>)>> {
    let wanted = sorted(tables);
    let key = Key::Block {
        branch: block.branch,
        offset: block.offset,
    };
    // Held from the start, so a block being read or answered stays found.
    let (slot, lease) = leased_slot(source, key)?;
    #[cfg(test)]
    ON_LEASED.with_borrow(|hook| {
        if let Some(hook) = hook {
            hook();
        }
    });
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
        Ok(reads) => return Ok(Some((reads, lease))),
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
    #[cfg(test)]
    ON_READ.with_borrow(|hook| {
        if let Some(hook) = hook {
            hook();
        }
    });
    count(source, 1, 0);
    let frames = match source
        .replay_ref()
        .read_block_recorded_selected_with(block, &added, || check_cancel(cancelled))
    {
        Err(error)
            if error.kind() == io::ErrorKind::Interrupted
                && cancelled.load(std::sync::atomic::Ordering::Acquire) =>
        {
            publish(source, key, &slot, reading, None);
            return Err(error);
        }
        result => result,
    };
    #[cfg(test)]
    let frames = frames.and_then(|frames| {
        match UNCACHEABLE
            .with_borrow(|tables| added.iter().find(|table| tables.contains(table)).copied())
        {
            Some(table) => Err(io::Error::other(format!("unreadable {table:?}"))),
            None => Ok(frames),
        }
    });
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
    if let Err(error) = check_cancel(cancelled) {
        publish(source, key, &slot, reading, None);
        return Err(error);
    }
    let mut reads = (*held).clone();
    reads.push(BlockRead {
        tables: added,
        frames: Arc::new(frames),
    });
    let answer = covering(&reads).expect("the reads hold every wanted table");
    publish(source, key, &slot, reading, Some(Value::Block(Arc::new(reads))));
    Ok(Some((answer, lease)))
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
    seek_with(
        source,
        branch,
        hour,
        tables,
        cancelled,
        &mut Direct {
            source,
            branch,
            hour,
            cancelled,
        },
    )
}

/// What a read at `key`'s entry, as it is now, would select (none: it reads no block), whether
/// another request is reading it, and a lease keeping its held frame for the join.
fn lacking<'a>(
    source: &'a Source,
    key: Key,
    tables: &[TableId],
    wanted: &[TableId],
) -> (Option<Vec<TableId>>, bool, Option<Lease<'a>>) {
    let Some(lease) = Lease::of(source, key) else {
        return (Some(tables.to_vec()), false, None);
    };
    let state = lock(&lease.slot.state);
    let (lacks, reading, held) = match &state.value {
        Some(Value::Hour(held)) => {
            let added = missing(&held.tables, wanted);
            ((!added.is_empty()).then_some(added), state.reading, true)
        }
        _ => (Some(tables.to_vec()), state.reading, false),
    };
    drop(state);
    (lacks, reading, held.then_some(lease))
}

/// Coalesce one hour's reconstruction, join only missing tables and publish
/// before releasing its slot. A read never waits on its block holding the hour.
fn seek_with(
    source: &Source,
    branch: u64,
    hour: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
    reader: &mut impl HourRead,
) -> io::Result<SeekResult> {
    // A seek reconstructs the campaign with every selection.
    let wanted = sorted(&[tables, &[TableId::Campaign]].concat());
    let key = Key::Hour { branch, hour };
    // Without block decodes a request takes what is held or is refused; it prepares nothing.
    let prepares = !super::progressive::block_decodes_disabled();
    // Keeps the hour's entry and held frame while this request prepares, waits and reads.
    let mut _lease = None;
    let (slot, reading, held) = loop {
        if prepares {
            let (lacks, reading, lease) = lacking(source, key, tables, &wanted);
            _lease = lease;
            // A request the hour's reader may answer waits for it before reading the block.
            if let Some(lacks) = lacks
                && !reading
                && !reader.ready(&lacks)
            {
                reader.prepare(&lacks)?;
            }
        }
        // Leased as found: another request's release would remove an entry not yet being read.
        let (slot, lease) = leased_slot(source, key)?;
        _lease = Some(lease);
        let covered = |value: &Value| match value {
            Value::Hour(held) if missing(&held.tables, &wanted).is_empty() => masked(held, &wanted),
            _ => None,
        };
        let (reading, held) =
            match wait_or_read(source, key, Arc::clone(&slot), covered, cancelled)? {
                Ok(frame) => return Ok(SeekResult::Frame(Box::new(frame))),
                Err(read) => read,
            };
        let lacks = match &held {
            Some(Value::Hour(held)) => {
                let added = missing(&held.tables, &wanted);
                (!added.is_empty()).then_some(added)
            }
            _ => Some(tables.to_vec()),
        };
        match lacks {
            // Another read changed what the hour lacks while this one waited.
            Some(lacks) if prepares && !reader.ready(&lacks) => drop(reading),
            _ => break (slot, reading, held),
        }
    };
    let mut read = |tables: &[TableId]| reader.read(tables);
    let Some(Value::Hour(held)) = held else {
        // Nothing held: this read is the request's own.
        let result = read(tables);
        let kept = match &result {
            Ok(SeekResult::Frame(frame)) => {
                Some(Value::Hour(Arc::new(HourFrame {
                    tables: wanted,
                    frame: (**frame).clone(),
                })))
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
    publish(source, key, &slot, reading, Some(Value::Hour(Arc::new(kept))));
    Ok(SeekResult::Frame(Box::new(answer)))
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

pub fn gap_error(reason: &str, text: impl Into<String>) -> io::Error {
    if reason == "frame campaign identity differs from replay header"
        || reason.starts_with("unreadable block")
        || reason.starts_with("invalid frame baseline at hour ")
    {
        crate::invalid(reason)
    } else {
        dashboard_core::refused(text)
    }
}

#[cfg(test)]
mod gap_error_tests {
    use super::*;

    #[test]
    fn corruption_and_campaign_mismatches_are_not_public_refusals() {
        for reason in [
            "unreadable block: grouped section is not a zstd frame",
            "invalid frame baseline at hour 17: synthetic damage",
            "frame campaign identity differs from replay header",
        ] {
            let error = gap_error(reason, "Synthetic hour was not recorded");
            assert!(dashboard_core::refusal(&error).is_none());
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), reason);
        }
        let error = gap_error(
            "collector refused synthetic capture",
            "Synthetic hour was not recorded",
        );
        assert_eq!(
            dashboard_core::refusal(&error),
            Some("Synthetic hour was not recorded")
        );
    }
}
