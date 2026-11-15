//! Publication lanes for map events.
//!
//! A generation gives each registered native thread its own single-producer
//! byte ring and shares one [`SharedLane`] among threads that register after
//! every ring is taken. A callback copies its event into a frame in bounded
//! synchronous steps, without allocating, locking or calling the game, then
//! takes the next number of the one global sequence and publishes the frame
//! with a release store. An event that does not fit goes whole to storage of
//! its own on its generation's lock-free spill, so no callback waits for the
//! spool, and its demand sizes the next generation; nothing is overwritten, and
//! only a refused allocation refuses an event.
//!
//! The hourly cut ([`seal`]) reads two counters: when every sequence number
//! taken has been published, the interval is exactly the events numbered up
//! to the last one taken; admission skips copied events, but only writing frees their space.
//! The sizing thread offers a larger generation ([`grow`]); callbacks move to
//! it, the older one is drained until empty and kept until [`release`].
use crate::shared_lane::SharedLane;
use collector_primitives::spill_stack::SpillStack;
use std::{
    cell::{Cell, UnsafeCell},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

/// Frame header: payload length, sequence, hour, family and the native world
/// occurrence the event happened in.
pub const HEADER: usize = 4 + 8 + 4 + 1 + 8;
/// The smallest ring a dedicated lane gets.
pub const MINIMUM_LANE_BYTES: usize = 64 * 1024;

/// Why a publication was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No generation is installed.
    Staging,
    /// The allocator refused the spill an event that did not fit needed.
    Allocation,
    /// The writer could not read the event's operands.
    Read(&'static str),
}

struct Lane {
    bytes: Box<[UnsafeCell<u8>]>,
    /// Producer-owned write position.
    write: UnsafeCell<u64>,
    published: AtomicU64,
    consumed: AtomicU64,
    /// Spool-owned: where the next interval's copy starts, at least `consumed`.
    assembled: AtomicU64,
    /// The largest pending byte count a refused frame needed.
    demand: AtomicUsize,
}
// One producer writes a lane's bytes ahead of `published`; the consumer reads
// only behind it and frees only through `consumed`.
unsafe impl Sync for Lane {}

impl Lane {
    fn new(bytes: usize) -> Result<Self, &'static str> {
        let bytes = bytes
            .max(MINIMUM_LANE_BYTES)
            .checked_next_power_of_two()
            .ok_or("map event lane size overflow")?;
        let mut storage = Vec::new();
        engine_allocation::reserve_exact(&mut storage, bytes)
            .map_err(|_| "map event lane allocation failed")?;
        storage.resize_with(bytes, || UnsafeCell::new(0));
        Ok(Self {
            bytes: engine_allocation::boxed_slice(storage)
                .map_err(|_| "map event lane allocation failed")?,
            write: UnsafeCell::new(0),
            published: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            assembled: AtomicU64::new(0),
            demand: AtomicUsize::new(0),
        })
    }
    fn capacity(&self) -> usize {
        self.bytes.len()
    }
    fn at(&self, position: u64) -> *mut u8 {
        let index = position as usize & (self.bytes.len() - 1);
        self.bytes[index].get()
    }
    fn copy_out(&self, position: u64, output: &mut [u8]) {
        let start = position as usize & (self.bytes.len() - 1);
        let first = output.len().min(self.bytes.len() - start);
        unsafe {
            std::ptr::copy_nonoverlapping(self.at(position), output.as_mut_ptr(), first);
            std::ptr::copy_nonoverlapping(
                self.at(0),
                output.as_mut_ptr().add(first),
                output.len() - first,
            );
        }
    }
}

/// A shared-lane slot's header; its payload is in the slot's byte span.
#[derive(Clone, Copy, Default)]
struct SharedRecord {
    sequence: u64,
    hour: i32,
    family: u8,
    world: u64,
    len: u32,
}

/// An event no lane or shared slot had room for, with its payload.
struct Spilled {
    sequence: u64,
    hour: i32,
    family: u8,
    world: u64,
    payload: Vec<u8>,
    /// A queued interval holds it.
    admitted: bool,
}

struct Log {
    serial: u64,
    lanes: Box<[Lane]>,
    shared: SharedLane<SharedRecord>,
    /// Spilled events callbacks pushed and the spool has not taken.
    pushed: SpillStack<Spilled>,
    /// Taken from `pushed`, sorted by sequence and held until the interval that
    /// copied it is written; callbacks never lock it.
    spill: Mutex<Vec<Spilled>>,
    /// Per shared slot: its index + 1 while a queued interval holds it.
    admitted: Box<[AtomicU64]>,
    registered: AtomicUsize,
    publishers: AtomicUsize,
    /// The generation this one replaced, while it still holds events.
    previous: AtomicPtr<Log>,
}

impl Log {
    fn new(lanes: usize, lane_bytes: usize, shared: (usize, usize)) -> Result<Self, &'static str> {
        let mut storage = Vec::new();
        engine_allocation::reserve_exact(&mut storage, lanes.max(1))
            .map_err(|_| "map event lane allocation failed")?;
        for _ in 0..lanes.max(1) {
            storage.push(Lane::new(lane_bytes)?);
        }
        let shared = SharedLane::new(shared.0.max(1), shared.1.max(HEADER))
            .map_err(|_| "map event lane allocation failed")?;
        let mut admitted = Vec::new();
        engine_allocation::reserve_exact(&mut admitted, shared.len())
            .map_err(|_| "map event lane allocation failed")?;
        admitted.resize_with(shared.len(), || AtomicU64::new(0));
        Ok(Self {
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            lanes: engine_allocation::boxed_slice(storage)
                .map_err(|_| "map event lane allocation failed")?,
            shared,
            pushed: SpillStack::new(),
            spill: Mutex::new(Vec::new()),
            admitted: engine_allocation::boxed_slice(admitted)
                .map_err(|_| "map event lane allocation failed")?,
            registered: AtomicUsize::new(0),
            publishers: AtomicUsize::new(0),
            previous: AtomicPtr::new(std::ptr::null_mut()),
        })
    }
    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .lanes
                .iter()
                .map(|lane| size_of::<Lane>() + lane.capacity())
                .sum::<usize>()
            + self.shared.allocated_bytes()
            + self.admitted.len() * size_of::<AtomicU64>()
            + {
                let spill = self.spill();
                spill.capacity() * size_of::<Spilled>()
                    + spill
                        .iter()
                        .map(|spilled| spilled.payload.capacity())
                        .sum::<usize>()
            }
    }
    fn admission(&self, slot: u64) -> &AtomicU64 {
        &self.admitted[slot as usize & (self.admitted.len() - 1)]
    }
    /// Spool side: every spilled event, sorted by sequence, the pushed ones taken
    /// in. A refused allocation loses the taken events, and the next interval
    /// reports lost output.
    fn spill(&self) -> std::sync::MutexGuard<'_, Vec<Spilled>> {
        let mut spill = self
            .spill
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.pushed.is_empty() {
            match self.pushed.take_into(&mut spill) {
                Ok(()) => spill.sort_unstable_by_key(|spilled| spilled.sequence),
                Err(_) => crate::interval::lost_output(),
            }
        }
        spill
    }
}

static SERIAL: AtomicU64 = AtomicU64::new(1);
static CURRENT: AtomicPtr<Log> = AtomicPtr::new(std::ptr::null_mut());
#[allow(clippy::vec_box)] // Published pointers stay stable through vector growth.
static LOGS: Mutex<Vec<Box<Log>>> = Mutex::new(Vec::new());
/// The last sequence number taken; the first event takes 1.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
/// How many events have been published.
static PUBLISHED: AtomicU64 = AtomicU64::new(0);
/// The longest payload the shared lane refused.
static SHARED_DEMAND: AtomicUsize = AtomicUsize::new(0);
/// Threads that registered after a generation's lanes were all taken.
static REQUIRED_LANES: AtomicUsize = AtomicUsize::new(0);
/// Intervals admitted to the writer queue and not yet written.
static ADMITTED: AtomicUsize = AtomicUsize::new(0);
/// Events that went to a spill since attach.
static SPILLED: AtomicU64 = AtomicU64::new(0);

std::thread_local! {
    /// (generation address, its serial, this thread's lane index).
    static LANE: Cell<(usize, u64, usize)> = const { Cell::new((0, 0, 0)) };
}

/// Where a frame's bytes go. `put` copies the payload in order.
pub struct Writer<'a> {
    target: Target<'a>,
    written: usize,
    len: usize,
}

enum Target<'a> {
    /// A lane and the position of the writer's first byte.
    Lane(&'a Lane, u64),
    Slot(*mut u8),
}

impl Writer<'_> {
    pub fn put(&mut self, bytes: &[u8]) {
        let end = self.written + bytes.len();
        assert!(
            end <= self.len,
            "map event writer exceeded its declared length"
        );
        match self.target {
            Target::Lane(lane, start) => {
                let position = start + self.written as u64;
                let index = position as usize & (lane.bytes.len() - 1);
                let first = bytes.len().min(lane.bytes.len() - index);
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), lane.at(position), first);
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr().add(first),
                        lane.at(0),
                        bytes.len() - first,
                    );
                }
            }
            Target::Slot(slot) => unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), slot.add(self.written), bytes.len());
            },
        }
        self.written = end;
    }
    pub fn u8(&mut self, value: u8) {
        self.put(&[value]);
    }
    pub fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }
}

struct Lease(&'static Log);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.publishers.fetch_sub(1, Ordering::SeqCst);
    }
}

fn lease() -> Option<Lease> {
    loop {
        let log = CURRENT.load(Ordering::SeqCst);
        if log.is_null() {
            return None;
        }
        let log = unsafe { &*log };
        log.publishers.fetch_add(1, Ordering::SeqCst);
        // A generation replaced after this check is drained only once no
        // publisher holds it, so this frame is never lost with it.
        if std::ptr::eq(CURRENT.load(Ordering::SeqCst), log) {
            return Some(Lease(log));
        }
        drop(Lease(log));
    }
}

fn lane_index(log: &Log) -> usize {
    LANE.with(|cache| {
        let address = log as *const Log as usize;
        let (cached, serial, index) = cache.get();
        if cached == address && serial == log.serial {
            return index;
        }
        let index = log.registered.fetch_add(1, Ordering::Relaxed);
        cache.set((address, log.serial, index));
        if index >= log.lanes.len() {
            REQUIRED_LANES.fetch_max(index + 1, Ordering::Relaxed);
        }
        index
    })
}

/// Publish one event of `len` payload bytes that `write` copies, in the
/// native world occurrence `world`. A writer that returns an error publishes
/// nothing.
pub fn publish(
    family: u8,
    hour: i32,
    world: u64,
    len: usize,
    write: impl FnOnce(&mut Writer<'_>) -> Result<(), &'static str>,
) -> Result<u64, Refusal> {
    let lease = lease().ok_or(Refusal::Staging)?;
    let log = lease.0;
    let total = HEADER + len;
    let header = |sequence: u64| {
        let mut bytes = [0_u8; HEADER];
        bytes[..4].copy_from_slice(&(len as u32).to_le_bytes());
        bytes[4..12].copy_from_slice(&sequence.to_le_bytes());
        bytes[12..16].copy_from_slice(&hour.to_le_bytes());
        bytes[16] = family;
        bytes[17..25].copy_from_slice(&world.to_le_bytes());
        bytes
    };
    let Some(lane) = log.lanes.get(lane_index(log)) else {
        if len > log.shared.stride() {
            SHARED_DEMAND.fetch_max(len, Ordering::Relaxed);
            return spill(log, family, hour, world, len, write);
        }
        let Some(mut slot) = log.shared.try_reserve() else {
            return spill(log, family, hour, world, len, write);
        };
        let mut writer = Writer {
            target: Target::Slot(slot.bytes()),
            written: 0,
            len,
        };
        write(&mut writer).map_err(Refusal::Read)?;
        if writer.written != len {
            return Err(Refusal::Read("map event writer left its frame short"));
        }
        let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
        slot.publish(SharedRecord {
            sequence,
            hour,
            family,
            world,
            len: len as u32,
        });
        PUBLISHED.fetch_add(1, Ordering::SeqCst);
        return Ok(sequence);
    };
    let start = unsafe { *lane.write.get() };
    let pending = start.wrapping_sub(lane.consumed.load(Ordering::Acquire)) as usize;
    if pending + total > lane.capacity() {
        lane.demand.fetch_max(pending + total, Ordering::Relaxed);
        return spill(log, family, hour, world, len, write);
    }
    let mut writer = Writer {
        target: Target::Lane(lane, start + HEADER as u64),
        written: 0,
        len,
    };
    write(&mut writer).map_err(Refusal::Read)?;
    if writer.written != len {
        return Err(Refusal::Read("map event writer left its frame short"));
    }
    let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
    let header = header(sequence);
    let mut header_writer = Writer {
        target: Target::Lane(lane, start),
        written: 0,
        len: HEADER,
    };
    header_writer.put(&header);
    let end = start + total as u64;
    unsafe { *lane.write.get() = end };
    lane.published.store(end, Ordering::Release);
    PUBLISHED.fetch_add(1, Ordering::SeqCst);
    Ok(sequence)
}

/// An event its lane or the shared lane has no room for: its payload is copied into storage
/// of its own, numbered only once that storage is held, and pushed before it counts as
/// published, so a sealed interval's spilled events are all on the spill.
#[cold]
fn spill(
    log: &Log,
    family: u8,
    hour: i32,
    world: u64,
    len: usize,
    write: impl FnOnce(&mut Writer<'_>) -> Result<(), &'static str>,
) -> Result<u64, Refusal> {
    let mut payload = Vec::new();
    engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut payload, len))
        .map_err(|_| Refusal::Allocation)?;
    payload.resize(len, 0);
    let mut writer = Writer {
        target: Target::Slot(payload.as_mut_ptr()),
        written: 0,
        len,
    };
    write(&mut writer).map_err(Refusal::Read)?;
    if writer.written != len {
        return Err(Refusal::Read("map event writer left its frame short"));
    }
    let mut spilled = log
        .pushed
        .reserve(Spilled {
            sequence: 0,
            hour,
            family,
            world,
            payload,
            admitted: false,
        })
        .map_err(|_| Refusal::Allocation)?;
    let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
    spilled.sequence = sequence;
    spilled.push();
    SPILLED.fetch_add(1, Ordering::Relaxed);
    PUBLISHED.fetch_add(1, Ordering::SeqCst);
    Ok(sequence)
}

/// The hourly cut: the last sequence number of the interval, and whether
/// every number up to it had been published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seal {
    pub last: u64,
    pub complete: bool,
}

/// Game thread, constant time.
pub fn seal() -> Seal {
    for _ in 0..64 {
        // Published first: every publication it counts took a number no
        // later than the sequence read after it.
        let published = PUBLISHED.load(Ordering::SeqCst);
        let last = SEQUENCE.load(Ordering::SeqCst);
        if published == last {
            return Seal {
                last,
                complete: true,
            };
        }
        std::hint::spin_loop();
    }
    Seal {
        last: SEQUENCE.load(Ordering::SeqCst),
        complete: false,
    }
}

/// The next sequence number, for a recording that continues a history.
pub fn restore_sequence(previous: u64) -> Result<(), &'static str> {
    if !CURRENT.load(Ordering::SeqCst).is_null() {
        return Err("map event sequence restoration requires no staging");
    }
    if previous == u64::MAX {
        return Err("map event sequence exhausted");
    }
    SEQUENCE.fetch_max(previous, Ordering::SeqCst);
    PUBLISHED.fetch_max(previous, Ordering::SeqCst);
    Ok(())
}

/// One copied event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub sequence: u64,
    pub hour: i32,
    pub family: u8,
    pub world: u64,
    pub start: usize,
    pub len: usize,
}

/// An interval's copied events and what committing it frees.
#[derive(Default)]
pub struct Assembled {
    pub frames: Vec<Frame>,
    pub bytes: Vec<u8>,
    lanes: Vec<(usize, usize, u64)>,
    shared: Vec<(usize, u64)>,
    rejected: Vec<(usize, u64)>,
    /// (generation, sequence) of each spilled event copied, ascending per generation.
    spilled: Vec<(usize, u64)>,
    admitted: AtomicBool,
}

fn generations() -> impl Iterator<Item = &'static Log> {
    let mut next = CURRENT.load(Ordering::SeqCst);
    std::iter::from_fn(move || {
        if next.is_null() {
            return None;
        }
        let log = unsafe { &*next };
        next = log.previous.load(Ordering::SeqCst);
        Some(log)
    })
}

/// Spool thread: copy every published event numbered up to `seal.last`
/// that no earlier interval has committed or queued, in sequence order.
pub fn assemble(seal: Seal, output: &mut Assembled) -> Result<(), &'static str> {
    output.frames.clear();
    output.bytes.clear();
    output.lanes.clear();
    output.shared.clear();
    output.rejected.clear();
    output.spilled.clear();
    output.admitted.store(false, Ordering::Relaxed);
    let mut header = [0_u8; HEADER];
    for log in generations() {
        let generation = log as *const Log as usize;
        for (index, lane) in log.lanes.iter().enumerate() {
            let published = lane.published.load(Ordering::Acquire);
            let mut position = lane.assembled.load(Ordering::Acquire);
            while position < published {
                lane.copy_out(position, &mut header);
                let len = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
                let sequence = u64::from_le_bytes(header[4..12].try_into().unwrap());
                if sequence > seal.last {
                    break;
                }
                let start = output.bytes.len();
                engine_allocation::reserve(&mut output.bytes, len)
                    .map_err(|_| "map event assembly allocation failed")?;
                output.bytes.resize(start + len, 0);
                lane.copy_out(position + HEADER as u64, &mut output.bytes[start..]);
                engine_allocation::reserve(&mut output.frames, 1)
                    .map_err(|_| "map event assembly allocation failed")?;
                output.frames.push(Frame {
                    sequence,
                    hour: i32::from_le_bytes(header[12..16].try_into().unwrap()),
                    family: header[16],
                    world: u64::from_le_bytes(header[17..25].try_into().unwrap()),
                    start,
                    len,
                });
                position += (HEADER + len) as u64;
            }
            engine_allocation::reserve(&mut output.lanes, 1)
                .map_err(|_| "map event assembly allocation failed")?;
            output.lanes.push((generation, index, position));
        }
        let rejected = log.shared.rejected();
        for (sequence, slot) in log
            .shared
            .published(|record| record.sequence)
            .map_err(|_| "map event assembly allocation failed")?
        {
            if sequence > seal.last || log.admission(slot).load(Ordering::Acquire) == slot + 1 {
                continue;
            }
            let (record, bytes) = log.shared.record(slot);
            let start = output.bytes.len();
            let len = record.len as usize;
            engine_allocation::reserve(&mut output.bytes, len)
                .map_err(|_| "map event assembly allocation failed")?;
            output.bytes.extend_from_slice(&bytes[..len]);
            engine_allocation::reserve(&mut output.frames, 1)
                .map_err(|_| "map event assembly allocation failed")?;
            output.frames.push(Frame {
                sequence,
                hour: record.hour,
                family: record.family,
                world: record.world,
                start,
                len,
            });
            engine_allocation::reserve(&mut output.shared, 1)
                .map_err(|_| "map event assembly allocation failed")?;
            output.shared.push((generation, slot));
        }
        engine_allocation::reserve(&mut output.rejected, 1)
            .map_err(|_| "map event assembly allocation failed")?;
        output.rejected.push((generation, rejected));
        copy_spilled(seal, generation, &log.spill(), output)?;
    }
    output.frames.sort_unstable_by_key(|frame| frame.sequence);
    Ok(())
}

fn copy_spilled(
    seal: Seal,
    generation: usize,
    spilled: &[Spilled],
    output: &mut Assembled,
) -> Result<(), &'static str> {
    for spilled in spilled
        .iter()
        .take_while(|spilled| spilled.sequence <= seal.last)
        .filter(|spilled| !spilled.admitted)
    {
        let start = output.bytes.len();
        engine_allocation::reserve(&mut output.bytes, spilled.payload.len())
            .map_err(|_| "map event assembly allocation failed")?;
        output.bytes.extend_from_slice(&spilled.payload);
        engine_allocation::reserve(&mut output.frames, 1)
            .map_err(|_| "map event assembly allocation failed")?;
        output.frames.push(Frame {
            sequence: spilled.sequence,
            hour: spilled.hour,
            family: spilled.family,
            world: spilled.world,
            start,
            len: spilled.payload.len(),
        });
        engine_allocation::reserve(&mut output.spilled, 1)
            .map_err(|_| "map event assembly allocation failed")?;
        output.spilled.push((generation, spilled.sequence));
    }
    Ok(())
}

/// The positions in a generation's spill of the events `copied` lists for it.
fn spilled_in<'a>(
    spill: &'a mut [Spilled],
    generation: usize,
    copied: &'a [(usize, u64)],
) -> impl Iterator<Item = &'a mut Spilled> {
    let mut spill = spill.iter_mut().peekable();
    copied
        .iter()
        .filter(move |&&(address, _)| address == generation)
        .filter_map(move |&(_, sequence)| {
            while spill.next_if(|spilled| spilled.sequence < sequence).is_some() {}
            spill.next_if(|spilled| spilled.sequence == sequence)
        })
}

/// Generations stay allocated until `release`, including after replacement.
fn generation(address: usize) -> &'static Log {
    unsafe { &*(address as *const Log) }
}

impl Assembled {
    pub fn allocated_bytes(&self) -> usize {
        self.frames.capacity() * size_of::<Frame>()
            + self.bytes.capacity()
            + self.lanes.capacity() * size_of::<(usize, usize, u64)>()
            + (self.shared.capacity() + self.rejected.capacity() + self.spilled.capacity())
                * size_of::<(usize, u64)>()
    }

    /// The generations this interval copied spilled events from.
    fn spilled_generations(&self) -> impl Iterator<Item = usize> + '_ {
        self.spilled
            .iter()
            .enumerate()
            .filter(|&(index, &(address, _))| {
                index == 0 || self.spilled[index - 1].0 != address
            })
            .map(|(_, &(address, _))| address)
    }

    /// Allocation-free admission skips these events without freeing their space.
    pub fn admitted(&self) {
        if self.admitted.swap(true, Ordering::AcqRel) {
            return;
        }
        ADMITTED.fetch_add(1, Ordering::AcqRel);
        for &(address, index, position) in &self.lanes {
            if let Some(lane) = generation(address).lanes.get(index) {
                lane.assembled.fetch_max(position, Ordering::Release);
            }
        }
        for &(address, slot) in &self.shared {
            generation(address)
                .admission(slot)
                .store(slot + 1, Ordering::Release);
        }
        for address in self.spilled_generations() {
            let mut spill = generation(address).spill();
            for spilled in spilled_in(&mut spill, address, &self.spilled) {
                spilled.admitted = true;
            }
        }
    }

    /// After the interval's frame is written: free what it copied, and
    /// unlink older generations that are empty and that no callback holds.
    /// A frame written without admission counts as admitted too.
    pub fn commit(&self) {
        if self.admitted.swap(false, Ordering::AcqRel) {
            let _ = ADMITTED.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            });
        }
        for &(address, index, position) in &self.lanes {
            if let Some(lane) = generation(address).lanes.get(index) {
                lane.consumed.fetch_max(position, Ordering::Release);
                lane.assembled.fetch_max(position, Ordering::Release);
            }
        }
        for &(address, slot) in &self.shared {
            generation(address).shared.release(slot);
        }
        for &(address, rejected) in &self.rejected {
            generation(address).shared.advance(rejected);
        }
        for address in self.spilled_generations() {
            let mut spill = generation(address).spill();
            let mut written = self
                .spilled
                .iter()
                .filter(|&&(log, _)| log == address)
                .map(|&(_, sequence)| sequence)
                .peekable();
            spill.retain(|spilled| {
                while written.next_if(|&sequence| sequence < spilled.sequence).is_some() {}
                written.next_if_eq(&spilled.sequence).is_none()
            });
        }
        let mut newer: Option<&Log> = None;
        for log in generations() {
            let empty = log.publishers.load(Ordering::SeqCst) == 0
                && log.lanes.iter().all(|lane| {
                    lane.consumed.load(Ordering::Acquire) == lane.published.load(Ordering::Acquire)
                })
                && log
                    .shared
                    .published(|record| record.sequence)
                    .is_ok_and(|published| published.is_empty())
                && log.spill().is_empty();
            match newer {
                Some(newer) if empty => newer
                    .previous
                    .store(log.previous.load(Ordering::SeqCst), Ordering::SeqCst),
                _ => newer = Some(log),
            }
        }
    }
}

/// After draining lost frames, copy from the last write and report their lost output.
pub fn reset_admitted() {
    for log in generations() {
        for lane in log.lanes.iter() {
            lane.assembled
                .store(lane.consumed.load(Ordering::Acquire), Ordering::Release);
        }
        for admission in log.admitted.iter() {
            admission.store(0, Ordering::Release);
        }
        for spilled in log.spill().iter_mut() {
            spilled.admitted = false;
        }
    }
    if ADMITTED.swap(0, Ordering::AcqRel) != 0 {
        // The row and the faults its seal took were never written.
        crate::interval::lost_output();
    }
}

/// Sizing thread: install a generation with at least `lanes` thread lanes
/// of `lane_bytes`, raised by every refusal's recorded demand. Returns
/// whether a generation was installed.
pub fn grow(lanes: usize, lane_bytes: usize) -> Result<bool, &'static str> {
    let current = CURRENT.load(Ordering::SeqCst);
    let lanes = lanes.max(REQUIRED_LANES.load(Ordering::Relaxed));
    let (lane_bytes, shared) = match unsafe { current.as_ref() } {
        Some(log) => {
            let demand = log
                .lanes
                .iter()
                .map(|lane| lane.demand.load(Ordering::Relaxed).saturating_mul(2))
                .max()
                .unwrap_or(0);
            let largest = log.lanes.iter().map(Lane::capacity).max().unwrap_or(0);
            let slots = log.shared.required_entries().max(log.shared.len());
            let stride = SHARED_DEMAND
                .load(Ordering::Relaxed)
                .max(log.shared.stride())
                .max(lane_bytes / 8);
            let bytes = lane_bytes.max(demand).max(largest);
            if log.lanes.len() >= lanes
                && bytes <= largest
                && slots <= log.shared.len()
                && stride <= log.shared.stride()
            {
                return Ok(false);
            }
            (bytes, (slots, stride))
        }
        // Room for every thread past the rings to publish once per interval
        // before growth sizes the shared lane from what it refused.
        None => (
            lane_bytes,
            (lanes.saturating_mul(4).max(16), lane_bytes / 8),
        ),
    };
    let fresh = engine_allocation::boxed(Log::new(lanes, lane_bytes, shared)?)
        .map_err(|_| "map event lane allocation failed")?;
    let pointer = &*fresh as *const Log as *mut Log;
    let mut retained = LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    engine_allocation::reserve(&mut retained, 1).map_err(|_| "map event lane allocation failed")?;
    retained.push(fresh);
    unsafe { (*pointer).previous.store(current, Ordering::SeqCst) };
    CURRENT.store(pointer, Ordering::SeqCst);
    Ok(true)
}

/// Bytes every retained generation holds.
pub fn allocated_bytes() -> usize {
    LOGS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|log| log.allocated_bytes())
        .sum()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Capacity {
    pub generations: usize,
    pub lanes: usize,
    pub lane_bytes: usize,
    pub registered: usize,
    pub required_lanes: usize,
    pub shared_entries: usize,
    pub shared_stride: usize,
    pub shared_rejected: u64,
    pub sequence: u64,
    pub published: u64,
    /// Events that went to a spill since attach, and those every generation's spill holds.
    pub spilled: u64,
    pub spill_pending: usize,
}

pub fn capacity() -> Capacity {
    let mut capacity = Capacity {
        generations: generations().count(),
        required_lanes: REQUIRED_LANES.load(Ordering::Relaxed),
        sequence: SEQUENCE.load(Ordering::Relaxed),
        published: PUBLISHED.load(Ordering::Relaxed),
        spilled: SPILLED.load(Ordering::Relaxed),
        spill_pending: generations().map(|log| log.spill().len()).sum(),
        ..Default::default()
    };
    if let Some(log) = unsafe { CURRENT.load(Ordering::SeqCst).as_ref() } {
        capacity.lanes = log.lanes.len();
        capacity.lane_bytes = log.lanes.iter().map(Lane::capacity).max().unwrap_or(0);
        capacity.registered = log.registered.load(Ordering::Relaxed);
        capacity.shared_entries = log.shared.len();
        capacity.shared_stride = log.shared.stride();
        capacity.shared_rejected = log.shared.rejected();
    }
    capacity
}

/// Free every generation.
///
/// # Safety
/// No callback can publish: the hooks are restored, no native call holds
/// one, and every reader of an assembled interval has finished.
pub unsafe fn release() {
    CURRENT.store(std::ptr::null_mut(), Ordering::SeqCst);
    ADMITTED.store(0, Ordering::SeqCst);
    LOGS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    unsafe { release() };
    SEQUENCE.store(0, Ordering::SeqCst);
    PUBLISHED.store(0, Ordering::SeqCst);
    SHARED_DEMAND.store(0, Ordering::SeqCst);
    REQUIRED_LANES.store(0, Ordering::SeqCst);
    SPILLED.store(0, Ordering::SeqCst);
}

#[cfg(test)]
#[path = "lanes/tests.rs"]
mod tests;
