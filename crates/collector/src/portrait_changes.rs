//! Bounded, allocation-free portrait mutation notifications. Only authenticated
//! reader addresses are registered; notifications never infer an object owner.
//!
//! These writers run outside the command batch, on the game thread and possibly others, and
//! need not wait for any capture: the first to change a holder during one logs the holder's
//! array as it was, and every portrait reader takes that image
//! ([`engine_native::portrait_cache::settled`]). A writer the store cannot hold waits for the
//! capture instead.
use crate::hook::Hook;
use engine_characters::ccharacter_portraits::MUTATIONS;
use engine_native::{
    Memory,
    portrait_cache::{Changes, Counts, EntryImage, HolderImage},
};
use std::{
    cell::{Cell, UnsafeCell},
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{WaitOnAddress, WakeByAddressAll};

const WATCHES: usize = 65536;
static REVISIONS: Revisions<WATCHES> = Revisions::new();
static EPOCH: AtomicU64 = AtomicU64::new(1);
/// Moved by every writer: guards a walk's fresh reads of unwatched holders.
static REVISION: AtomicU64 = AtomicU64::new(0);
/// Moved only by writers of a watched holder. The holder constructor and
/// two destructors also run every hour for holders no
/// Capture watches  (division construction,
/// ship construction, leader UI items such as
/// leader UI item construction). A quiet certificate covers only
/// holders watched when it was taken, and a watch is never removed during an
/// attachment, so no other writer can change what it certifies. A writer
/// that looked its holder up before the capture registered it is still
/// active when the capture checks `ACTIVE`, which then certifies nothing.
static WATCHED_REVISION: AtomicU64 = AtomicU64::new(0);
static PLANNED_PARTS: AtomicU64 = AtomicU64::new(0);
static QUIET_PARTS: AtomicU64 = AtomicU64::new(0);
static WORK: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];
static EVENTS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static COUNTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

struct Slot {
    address: AtomicUsize,
    sequence: AtomicU64,
}

/// Slots are never removed or reused during an attachment. Bounded probing
/// can refuse a new watch; that address then uses ordinary fresh reads. A
/// collision cannot dirty another address or certify an unobserved writer.
struct Revisions<const N: usize> {
    slots: [Slot; N],
}
impl<const N: usize> Revisions<N> {
    const fn new() -> Self {
        assert!(N.is_power_of_two());
        Self {
            slots: [const {
                Slot {
                    address: AtomicUsize::new(0),
                    sequence: AtomicU64::new(0),
                }
            }; N],
        }
    }

    fn sequence(&self, array: usize, register: bool) -> Option<&AtomicU64> {
        if array == 0 {
            return None;
        }
        let first = ((array >> 3).wrapping_mul(0x9e3779b97f4a7c15) >> 32) & (N - 1);
        for probe in 0..N.min(16) {
            let slot = &self.slots[(first + probe) & (N - 1)];
            let mut address = slot.address.load(Ordering::SeqCst);
            if address == 0 && register {
                address = match slot.address.compare_exchange(
                    0,
                    array,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => array,
                    Err(address) => address,
                };
            }
            if address == array {
                return Some(&slot.sequence);
            }
            if address == 0 {
                return None;
            }
        }
        None
    }
}

fn increment(value: &AtomicU64) {
    if value.fetch_add(1, Ordering::SeqCst) == u64::MAX {
        POISONED.store(true, Ordering::SeqCst);
    }
}

pub fn invalidate() {
    increment(&EPOCH);
}

struct Mutation {
    sequence: Option<&'static AtomicU64>,
    /// In flight in the pre-image store until the original returns.
    writing: bool,
}
impl Mutation {
    /// A writer of the existing holder whose array is at `array`.
    fn change(array: usize, site: usize) -> Self {
        // SAFETY: the game's memory, only read.
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        PRE_IMAGES.before_change(array, &memory);
        Self::enter(array, site, true)
    }
    /// A writer that brings the holder at `array` into being: no capture began reading it.
    fn create(array: usize, site: usize) -> Self {
        Self::enter(array, site, false)
    }
    fn enter(array: usize, site: usize, writing: bool) -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        let sequence = REVISIONS.sequence(array, false);
        note(sequence);
        EVENTS[site].fetch_add(1, Ordering::Relaxed);
        Self { sequence, writing }
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        note(self.sequence);
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
        if self.writing {
            PRE_IMAGES.after_change();
        }
    }
}

/// How long a capture's begin waits for writers already in flight; writers in flight take
/// microseconds.
static PRE_IMAGES: PreImages = PreImages::new(Duration::from_millis(5));
/// Captures whose portrait reads were withheld for a writer still in flight at their begin.
static WITHHELD: AtomicU64 = AtomicU64::new(0);

const OPEN: u32 = 0;
const CLOSING: u32 = 1;
const READING: u32 = 2;

thread_local! {
    /// Writers of an existing holder this thread is inside.
    static WRITING: Cell<usize> = const { Cell::new(0) };
}

/// Holders' arrays as they were at the running capture's instant, each logged by the first writer
/// to change it during that capture. The storage grows between captures to what earlier ones
/// needed; a writer it cannot hold waits for the capture to end instead.
///
/// A writer counts itself in flight, then reads the state; a begin sets `CLOSING`, waits until
/// none is in flight, then sets `READING`. That instant splits the writes of every thread: one
/// done before it is read live, and one after it logs first; a writer seeing `CLOSING` waits.
struct PreImages {
    state: AtomicU32,
    /// Writers of an existing holder from their look at `state` to their original's return.
    in_flight: AtomicUsize,
    /// How long a begin waits for writers in flight, and a writer for a begin.
    bound: Duration,
    /// The running capture could not wait out the writers: its portrait reads are withheld.
    withheld: AtomicBool,
    /// Writers inside a log: the storage is replaced only while none is.
    logging: AtomicUsize,
    /// Records, entries and text bytes reserved in the running capture.
    reserved: [AtomicUsize; 3],
    /// The most of each any capture since attach asked for.
    demand: [AtomicUsize; 3],
    storage: UnsafeCell<Storage>,
}

const RECORDS: usize = 0;
const ENTRIES: usize = 1;
const TEXT: usize = 2;

struct Storage {
    records: Vec<Record>,
    entries: Vec<UnsafeCell<EntryImage>>,
    text: Vec<UnsafeCell<u8>>,
}

#[derive(Default)]
struct Record {
    ready: AtomicBool,
    array: UnsafeCell<usize>,
    image: UnsafeCell<Logged>,
}

#[derive(Clone, Copy, Default)]
enum Logged {
    Holder {
        header: [u8; 16],
        first: usize,
        count: usize,
    },
    #[default]
    Unreadable,
}

#[derive(Debug, PartialEq, Eq)]
enum Log {
    Inactive,
    Logged,
    Full,
}

// SAFETY: a slot is written only by the writer that reserved it, before its record is
// published; published slots are only read until the next capture begins, after every reader
// of this one returned; the storage is replaced only while no writer logs and no capture reads.
unsafe impl Sync for PreImages {}

impl PreImages {
    const fn new(bound: Duration) -> Self {
        Self {
            state: AtomicU32::new(OPEN),
            in_flight: AtomicUsize::new(0),
            bound,
            withheld: AtomicBool::new(false),
            logging: AtomicUsize::new(0),
            reserved: [const { AtomicUsize::new(0) }; 3],
            demand: [const { AtomicUsize::new(0) }; 3],
            storage: UnsafeCell::new(Storage {
                records: Vec::new(),
                entries: Vec::new(),
                text: Vec::new(),
            }),
        }
    }

    /// On the thread that reads, before the capture's first read: the capture's instant. False,
    /// withholding its portrait reads, when a writer is still in flight after the bound or is
    /// this thread's own, which cannot finish while it reads.
    fn begin(&self) -> bool {
        // SAFETY: open with no writer logging since the last end, and the capture gate admits
        // one capture at a time.
        let storage = unsafe { &*self.storage.get() };
        let used = self.reserved[RECORDS].load(Ordering::SeqCst);
        for record in &storage.records[..used.min(storage.records.len())] {
            record.ready.store(false, Ordering::SeqCst);
        }
        for reserved in &self.reserved {
            reserved.store(0, Ordering::SeqCst);
        }
        self.state.store(CLOSING, Ordering::SeqCst);
        let deadline = Instant::now() + self.bound;
        let settled = WRITING.get() == 0
            && loop {
                if self.in_flight.load(Ordering::SeqCst) == 0 {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::yield_now();
            };
        self.withheld.store(!settled, Ordering::SeqCst);
        self.state.store(READING, Ordering::SeqCst);
        self.wake();
        settled
    }

    /// After the capture's last read: no writer logs any more and a writer waiting for the
    /// capture goes on; with `grow`, the storage then grows to what the writers asked for.
    fn end(&self, grow: bool) {
        self.state.store(OPEN, Ordering::SeqCst);
        self.wake();
        while self.logging.load(Ordering::SeqCst) != 0 {
            std::thread::yield_now();
        }
        if grow {
            // SAFETY: open with no writer logging, and the capture's readers returned.
            unsafe { self.grow() };
        }
    }

    fn wake(&self) {
        // SAFETY: an aligned u32 that lives as long as the store.
        unsafe { WakeByAddressAll(self.state.as_ptr().cast()) };
    }

    /// Waits while the state is `state`, until `deadline` if any.
    fn wait_while(&self, state: u32, deadline: Option<Instant>) {
        while self.state.load(Ordering::SeqCst) == state {
            let timeout = match deadline {
                // Bounds a missed wake.
                None => 100,
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return;
                    }
                    u32::try_from(left.as_millis()).map_or(100, |ms| ms.clamp(1, 100))
                }
            };
            // SAFETY: an aligned u32 that lives as long as the store, compared with a local of
            // its size.
            unsafe {
                WaitOnAddress(
                    self.state.as_ptr().cast(),
                    (&state as *const u32).cast(),
                    size_of::<u32>(),
                    timeout,
                )
            };
        }
    }

    /// # Safety
    /// Open, with no writer logging and no capture reading.
    unsafe fn grow(&self) {
        // SAFETY: the caller's.
        let storage = unsafe { &mut *self.storage.get() };
        let [records, entries, text] = self.demand.each_ref().map(|d| d.load(Ordering::SeqCst));
        if records <= storage.records.len()
            && entries <= storage.entries.len()
            && text <= storage.text.len()
        {
            return;
        }
        // Refused, the store stays as it is and writers it cannot hold keep waiting.
        if let (Some(records), Some(entries), Some(text)) = (
            boxed(records.max(storage.records.len())),
            boxed(entries.max(storage.entries.len())),
            boxed(text.max(storage.text.len())),
        ) {
            *storage = Storage {
                records,
                entries,
                text,
            };
        }
    }

    /// Before a writer changes the existing holder at `array`, in flight until
    /// [`Self::after_change`]: waits out a begin, then logs the holder for the running capture,
    /// or waits for that capture to end when the store cannot hold it.
    fn before_change(&self, array: usize, memory: &impl Memory) {
        let depth = WRITING.get();
        WRITING.set(depth + 1);
        // A writer inside another on its thread would wait for a begin that waits for it.
        let mut waited = depth != 0;
        loop {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            match self.state.load(Ordering::SeqCst) {
                // The begin waits for this writer, or withholds its capture's portraits.
                CLOSING if waited => return,
                CLOSING => {
                    self.in_flight.fetch_sub(1, Ordering::SeqCst);
                    self.wait_while(CLOSING, Some(Instant::now() + self.bound));
                    waited = true;
                }
                READING => {
                    if self.log(array, memory) != Log::Full {
                        return;
                    }
                    self.in_flight.fetch_sub(1, Ordering::SeqCst);
                    collector_timing::capture_gate::hold(
                        crate::hook_timing::Family::PortraitChanges as usize,
                    );
                    self.wait_while(READING, None);
                }
                _ => return,
            }
        }
    }

    /// The writer's original returned.
    fn after_change(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        WRITING.set(WRITING.get() - 1);
    }

    fn log(&self, array: usize, memory: &impl Memory) -> Log {
        self.logging.fetch_add(1, Ordering::SeqCst);
        let log = if self.state.load(Ordering::SeqCst) != READING {
            Log::Inactive
        } else if self.find(array).is_some() {
            Log::Logged
        } else {
            self.record(array, memory)
        };
        self.logging.fetch_sub(1, Ordering::SeqCst);
        log
    }

    /// The published record of the array at `array`, if any.
    fn find(&self, array: usize) -> Option<&Record> {
        // SAFETY: replaced only while no writer logs and no capture reads.
        let storage = unsafe { &*self.storage.get() };
        let len = self.reserved[RECORDS].load(Ordering::SeqCst);
        storage.records[..len.min(storage.records.len())]
            .iter()
            .find(|record| {
                // SAFETY: a ready record's fields were written before it was published.
                record.ready.load(Ordering::Acquire) && unsafe { *record.array.get() } == array
            })
    }

    /// Copies the array at `array` into room it reserves and publishes its record. Full when a
    /// part did not fit; every part is reserved all the same, so its demand is known.
    fn record(&self, array: usize, memory: &impl Memory) -> Log {
        // SAFETY: as in `find`.
        let storage = unsafe { &*self.storage.get() };
        let slot = self.reserve(RECORDS, 1);
        let image = self.copy(array, memory, storage);
        let (Some(record), Some(image)) = (storage.records.get(slot), image) else {
            return Log::Full;
        };
        // SAFETY: this writer reserved the slot; nothing reads it before it is published.
        unsafe {
            *record.array.get() = array;
            *record.image.get() = image;
        }
        record.ready.store(true, Ordering::Release);
        Log::Logged
    }

    /// The first of `count` reserved in `part`.
    fn reserve(&self, part: usize, count: usize) -> usize {
        let first = self.reserved[part].fetch_add(count, Ordering::SeqCst);
        self.demand[part].fetch_max(first.saturating_add(count), Ordering::SeqCst);
        first
    }

    /// The array at `array` in reserved room; `None` when a part did not fit.
    fn copy(&self, array: usize, memory: &impl Memory, storage: &Storage) -> Option<Logged> {
        use engine_native::{character_portrait_entry as entry, msvc_string, read};
        let unreadable = Some(Logged::Unreadable);
        let Some(header) = read::<[u8; 16]>(memory, array) else {
            return unreadable;
        };
        let Some(holder) = engine_native::cpdx_array::Array::from_header(header, entry::STRIDE)
        else {
            return unreadable;
        };
        let count = holder.count as usize;
        let first = self.reserve(ENTRIES, count);
        let mut fits = first.saturating_add(count) <= storage.entries.len();
        for index in 0..holder.count {
            let Some(at) = holder.element(index, entry::STRIDE) else {
                return unreadable;
            };
            let Some(identity) = at.checked_add(entry::IDENTITY) else {
                return unreadable;
            };
            let (Some(head), Some(layout)) = (
                read::<[u8; 16]>(memory, at),
                read::<[u8; msvc_string::STRIDE]>(memory, identity),
            ) else {
                return unreadable;
            };
            let word = |at: usize| u64::from_le_bytes(layout[at..at + 8].try_into().unwrap());
            let (len, capacity) = (word(msvc_string::LENGTH), word(msvc_string::CAPACITY));
            let source = if capacity == msvc_string::INLINE_CAPACITY {
                identity
            } else {
                word(0) as usize
            };
            let terminated = || {
                usize::try_from(len)
                    .ok()
                    .and_then(|len| source.checked_add(len))
                    .and_then(|end| read::<u8>(memory, end))
                    == Some(0)
            };
            if capacity < msvc_string::INLINE_CAPACITY || len > capacity || !terminated() {
                return unreadable;
            }
            let len = len as usize;
            let start = self.reserve(TEXT, len);
            fits &= start.saturating_add(len) <= storage.text.len();
            if !fits {
                continue;
            }
            // SAFETY: cells this writer reserved, contiguous in one allocation.
            let text = unsafe {
                std::slice::from_raw_parts_mut(
                    UnsafeCell::raw_get(storage.text.as_ptr().add(start)),
                    len,
                )
            };
            if len != 0 && !memory.copy(source, text) {
                return unreadable;
            }
            // SAFETY: an entry slot this writer reserved.
            unsafe {
                *storage.entries[first + index as usize].get() = EntryImage {
                    head,
                    text: [start, len],
                };
            }
        }
        fits.then_some(Logged::Holder {
            header,
            first,
            count,
        })
    }
}

impl engine_native::portrait_cache::PreImages for PreImages {
    fn image(&self, array: usize) -> Option<Result<HolderImage<'_>, &'static str>> {
        if self.state.load(Ordering::SeqCst) != READING {
            return None;
        }
        if self.withheld.load(Ordering::SeqCst) {
            return Some(Err(
                "character portrait writer still running when the capture began",
            ));
        }
        let record = self.find(array)?;
        // SAFETY: a published record and the slots it names are not written again until the
        // next capture begins, after every reader of this one has returned.
        Some(match unsafe { *record.image.get() } {
            Logged::Unreadable => Err("character portrait pre-image unreadable"),
            Logged::Holder {
                header,
                first,
                count,
            } => {
                // SAFETY: as above.
                let storage = unsafe { &*self.storage.get() };
                let entries: *const [UnsafeCell<EntryImage>] =
                    &storage.entries[first..first + count];
                let text: *const [UnsafeCell<u8>] = &storage.text[..];
                // SAFETY: as above; an UnsafeCell has the layout of what it holds.
                let (entries, text) =
                    unsafe { (&*(entries as *const [EntryImage]), &*(text as *const [u8])) };
                Ok(HolderImage {
                    header,
                    entries,
                    text,
                })
            }
        })
    }
}

fn boxed<T: Default>(len: usize) -> Option<Vec<T>> {
    let mut items = Vec::new();
    items.try_reserve_exact(len).ok()?;
    items.resize_with(len, T::default);
    Some(items)
}

/// A capture's portrait instant, held or beside the game: begun on the thread that reads before
/// the capture's first read, ended after its last, an unwind included. When a writer outlasts
/// the begin, every portrait read of the capture is withheld by name.
pub(crate) struct PreImagesCapture;
impl PreImagesCapture {
    pub(crate) fn begin() -> Self {
        if !PRE_IMAGES.begin() {
            WITHHELD.fetch_add(1, Ordering::Relaxed);
        }
        Self
    }
}
impl Drop for PreImagesCapture {
    fn drop(&mut self) {
        PRE_IMAGES.end(true);
    }
}

/// Captures whose portrait reads were withheld for a writer still in flight at their begin.
pub(crate) fn withheld_captures() -> u64 {
    WITHHELD.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) fn pre_images_active() -> bool {
    PRE_IMAGES.state.load(Ordering::SeqCst) == READING
}

/// One writer boundary of a holder: `sequence` is its watch, `None` for a
/// holder no capture watches.
fn note(sequence: Option<&AtomicU64>) {
    increment(&REVISION);
    if let Some(sequence) = sequence {
        increment(&WATCHED_REVISION);
        increment(sequence);
    }
}

pub struct Observer;
impl Observer {
    fn domain(revision: &AtomicU64) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let epoch = EPOCH.load(Ordering::SeqCst);
        let revision = revision.load(Ordering::SeqCst);
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some((epoch, revision))
    }
}

pub fn record_planned_part() {
    PLANNED_PARTS.fetch_add(1, Ordering::Relaxed);
}

/// A share carried whole from a quiet domain and an unchanged census,
/// without its prepared program's reads.
pub fn record_quiet_part() {
    QUIET_PARTS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_work(planned: bool, ticks: u64) {
    let slot = if planned { 0 } else { 2 };
    WORK[slot].fetch_add(1, Ordering::Relaxed);
    WORK[slot + 1].fetch_add(ticks, Ordering::Relaxed);
}
impl Changes for Observer {
    fn quiet_revision(&self) -> Option<(u64, u64)> {
        Observer::domain(&WATCHED_REVISION)
    }
    fn any_revision(&self) -> Option<(u64, u64)> {
        Observer::domain(&REVISION)
    }
    fn revision(&self, array: usize) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let epoch = EPOCH.load(Ordering::SeqCst);
        let sequence = REVISIONS.sequence(array, true)?.load(Ordering::SeqCst);
        if ACTIVE.load(Ordering::SeqCst) != 0 {
            return None;
        }
        Some((epoch, sequence))
    }
}

pub fn record(counts: Counts) {
    for (target, value) in COUNTS.iter().zip([
        counts.reused_characters,
        counts.reused_entries,
        counts.read_characters,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "portrait_changes") {
            let callbacks = [
                assign as *const () as usize,
                copy as *const () as usize,
                construct as *const () as usize,
                copy_construct as *const () as usize,
                destroy as *const () as usize,
                insert as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                let (at, original) = crate::located_hook(base, site.rva)?;
                let hook = unsafe { Hook::new(at, callbacks[index], original) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            engine_native::portrait_cache::install(&PRE_IMAGES);
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        // Every portrait reader takes what these writers change as it was when the capture
        // began, so they need not wait for it; that needs no revision, so poison changes nothing.
        collector_timing::capture_gate::exempt(
            crate::hook_timing::Family::PortraitChanges as usize,
            enabled && !self.hooks.is_empty(),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

unsafe extern "system" fn assign(array: usize, result: usize, key: usize, value: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::change(array, 0);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(array, result, key, value) })
}
unsafe extern "system" fn copy(holder: usize, source: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::change(holder.wrapping_add(8), 1);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder, source) })
}
unsafe extern "system" fn construct(holder: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::create(holder.wrapping_add(8), 2);
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder) })
}
unsafe extern "system" fn copy_construct(holder: usize, source: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::create(holder.wrapping_add(8), 3);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder, source) })
}
unsafe extern "system" fn destroy(holder: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::change(holder.wrapping_add(8), 4);
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[4].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder) })
}
unsafe extern "system" fn insert(array: usize, at: i32, source: usize, count: i32) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::change(array, 5);
    let original: unsafe extern "system" fn(usize, i32, usize, i32) =
        unsafe { std::mem::transmute(ORIGINAL[5].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(array, at, source, count) })
}

#[cfg(test)]
mod revision_tests {
    use super::*;

    /// Army, ship and UI holders, which no capture watches, move only the
    /// revision a walk guards its fresh reads with; a watched holder's writer
    /// moves the quiet revision and its own sequence too.
    #[test]
    fn unwatched_holder_writers_leave_the_quiet_revision_still() {
        let (watched, unwatched) = (0x7ff0_1234_5680, 0x7ff0_6543_2180);
        let sequence = REVISIONS.sequence(watched, true).unwrap();
        let revisions =
            || [&REVISION, &WATCHED_REVISION, sequence].map(|value| value.load(Ordering::SeqCst));
        let [any, quiet, own] = revisions();
        note(REVISIONS.sequence(unwatched, false));
        assert_eq!(revisions(), [any + 1, quiet, own]);
        note(REVISIONS.sequence(watched, false));
        assert_eq!(revisions(), [any + 2, quiet + 1, own + 1]);
    }

    /// A holder changed during a capture is logged by its first writer as it was; a writer
    /// the store has no room for waits, and the store grows for the next capture; a capture's
    /// begin waits out writers in flight, or withholds its portraits by name.
    #[test]
    fn the_first_change_in_a_capture_logs_the_holder_as_it_was() {
        use engine_native::{memory::SliceMemory, portrait_cache::PreImages as _};
        const ARRAY: usize = 0x1000;
        const ENTRY: usize = 0x100;
        let put = |bytes: &mut [u8], at: usize, value: &[u8]| {
            bytes[at..at + value.len()].copy_from_slice(value)
        };
        let mut bytes = vec![0; 0x200];
        put(&mut bytes, 0, &((ARRAY + ENTRY) as u64).to_le_bytes());
        put(&mut bytes, 8, &1_u32.to_le_bytes());
        put(&mut bytes, 12, &1_u32.to_le_bytes());
        put(&mut bytes, ENTRY, &4_u32.to_le_bytes());
        bytes[ENTRY + 8] = 1;
        put(&mut bytes, ENTRY + 0x10, b"x.dds");
        put(&mut bytes, ENTRY + 0x20, &5_u64.to_le_bytes());
        put(&mut bytes, ENTRY + 0x28, &15_u64.to_le_bytes());
        let mut changed = bytes.clone();
        put(&mut changed, ENTRY, &9_u32.to_le_bytes());
        let before = SliceMemory {
            base: ARRAY,
            bytes: &bytes,
        };
        let after = SliceMemory {
            base: ARRAY,
            bytes: &changed,
        };
        let store = PreImages::new(Duration::ZERO);
        assert_eq!(store.log(ARRAY, &before), Log::Inactive);
        // No room yet: the writer must wait, and the store then grows to what it asked for.
        assert!(store.begin());
        assert_eq!(store.log(ARRAY, &before), Log::Full);
        assert!(store.image(ARRAY).is_none());
        store.end(true);
        assert!(store.begin());
        assert_eq!(store.log(ARRAY, &before), Log::Logged);
        // A later change in the same capture keeps the first image.
        assert_eq!(store.log(ARRAY, &after), Log::Logged);
        let image = store.image(ARRAY).unwrap().unwrap();
        assert_eq!(image.header[..], bytes[..16]);
        assert_eq!(image.entries.len(), 1);
        assert_eq!(image.entries[0].head[..], bytes[ENTRY..ENTRY + 16]);
        assert_eq!(image.identity(&image.entries[0]).unwrap(), b"x.dds");
        assert!(store.image(ARRAY + 8).is_none());
        store.end(true);
        assert!(store.image(ARRAY).is_none());
        // A holder that cannot be read is logged as such, never as empty.
        assert!(store.begin());
        assert_eq!(store.log(ARRAY + 0x1000, &before), Log::Logged);
        assert!(store.image(ARRAY + 0x1000).unwrap().is_err());
        assert!(store.image(ARRAY).is_none());
        store.end(false);
        // A writer that saw no capture is still in flight when one begins: the begin waits until
        // its change is done, so the capture reads that change live. A writer arriving meanwhile
        // waits for the begin, then logs its holder as it was.
        let patient = PreImages::new(Duration::from_secs(600));
        assert!(patient.begin());
        assert_eq!(patient.log(ARRAY, &before), Log::Full);
        patient.end(true);
        std::thread::scope(|threads| {
            threads
                .spawn(|| patient.before_change(ARRAY + 0x1000, &before))
                .join()
                .unwrap();
            let begin = threads.spawn(|| patient.begin());
            while patient.state.load(Ordering::SeqCst) != CLOSING {
                std::thread::yield_now();
            }
            let late = threads.spawn(|| patient.before_change(ARRAY, &before));
            assert!(!begin.is_finished());
            assert!(!late.is_finished());
            // The first writer's original returns.
            patient.in_flight.fetch_sub(1, Ordering::SeqCst);
            assert!(begin.join().unwrap());
            late.join().unwrap();
        });
        assert!(patient.image(ARRAY + 0x1000).is_none());
        assert_eq!(
            patient.image(ARRAY).unwrap().unwrap().header[..],
            bytes[..16]
        );
        patient.in_flight.fetch_sub(1, Ordering::SeqCst);
        patient.end(false);
        // One still in flight when the bound runs out withholds the capture's portraits by name,
        // and the next capture's while it stays in flight; so does one on the capturing thread.
        store.in_flight.fetch_add(1, Ordering::SeqCst);
        assert!(!store.begin());
        assert!(store.image(ARRAY).unwrap().is_err());
        assert!(store.image(ARRAY + 8).unwrap().is_err());
        store.end(false);
        assert!(store.image(ARRAY).is_none());
        assert!(!store.begin());
        store.end(false);
        store.in_flight.fetch_sub(1, Ordering::SeqCst);
        assert!(store.begin());
        assert!(store.image(ARRAY).is_none());
        store.end(false);
        WRITING.set(1);
        assert!(!store.begin());
        store.end(false);
        WRITING.set(0);
    }

    #[test]
    fn exact_watches_survive_collisions_and_refuse_overflow_without_aliasing() {
        let revisions = Revisions::<4>::new();
        assert!(revisions.sequence(0, true).is_none());
        for address in [8, 16, 24, 32] {
            assert!(revisions.sequence(address, false).is_none());
            revisions
                .sequence(address, true)
                .unwrap()
                .store(address as u64, Ordering::SeqCst);
        }
        assert!(revisions.sequence(40, true).is_none());
        assert!(revisions.sequence(40, false).is_none());
        for address in [8, 16, 24, 32] {
            assert_eq!(
                revisions
                    .sequence(address, true)
                    .unwrap()
                    .load(Ordering::SeqCst),
                address as u64
            );
        }
        revisions
            .sequence(8, false)
            .unwrap()
            .fetch_add(2, Ordering::SeqCst);
        assert_eq!(
            revisions.sequence(8, true).unwrap().load(Ordering::SeqCst),
            10
        );
        assert_eq!(
            revisions.sequence(16, true).unwrap().load(Ordering::SeqCst),
            16
        );
    }
}
