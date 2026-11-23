use collector_hooks::hook::Hook;
use engine_characters::ccharacter_portraits::MUTATIONS;
use engine_native_memory::Memory;
use engine_native_politics::portrait_cache::{
    Changes,
    Counts,
    EntryImage,
    HolderImage,
    Readers,
    Reading,
};
use std::{
    cell::{Cell, UnsafeCell},
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{WaitOnAddress, WakeByAddressAll};

const WATCHES: usize = 65536;
static REVISIONS: Revisions<WATCHES> = Revisions::new();
static EPOCH: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(0);
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
    writing: bool,
}
impl Mutation {
    fn change(array: usize, site: usize) -> Self {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        PRE_IMAGES.before_change(array, &memory);
        Self::enter(array, site, true)
    }
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

static PRE_IMAGES: PreImages = PreImages::new(Duration::from_millis(5));
static WITHHELD: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

const OPEN: u32 = 0;
const CLOSING: u32 = 1;
const READING: u32 = 2;

const ADMITTED: u8 = 0;
const WRITER_RUNNING: u8 = 1;
const READER_HOLDING: u8 = 2;

thread_local! {
    static WRITING: Cell<usize> = const { Cell::new(0) };
}

struct PreImages {
    state: AtomicU32,
    in_flight: AtomicUsize,
    bound: Duration,
    withheld: AtomicU8,
    logging: AtomicUsize,
    readers: Readers,
    reserved: [AtomicUsize; 3],
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
// published, and only in a capture that began with no reader holding a pre-image; the storage
// is replaced only while no writer logs and no reader holds one. Every borrow `image` hands out
// is counted in `readers` until its `Reading` drops, and covers only a published record's
// entries and their text ranges.
unsafe impl Sync for PreImages {}

impl PreImages {
    const fn new(bound: Duration) -> Self {
        Self {
            state: AtomicU32::new(OPEN),
            in_flight: AtomicUsize::new(0),
            bound,
            withheld: AtomicU8::new(ADMITTED),
            logging: AtomicUsize::new(0),
            readers: Readers::new(),
            reserved: [const { AtomicUsize::new(0) }; 3],
            demand: [const { AtomicUsize::new(0) }; 3],
            storage: UnsafeCell::new(Storage {
                records: Vec::new(),
                entries: Vec::new(),
                text: Vec::new(),
            }),
        }
    }

    fn begin(&self) -> bool {
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
        let withheld = if !settled {
            WRITER_RUNNING
        } else if !self.readers.idle() {
            READER_HOLDING
        } else {
            ADMITTED
        };
        if withheld == ADMITTED {
            // SAFETY: closing, with no writer logging since the last end and no reader holding a
            // pre-image, and the capture gate admits one capture at a time.
            let storage = unsafe { &*self.storage.get() };
            let used = self.reserved[RECORDS].load(Ordering::SeqCst);
            for record in &storage.records[..used.min(storage.records.len())] {
                record.ready.store(false, Ordering::SeqCst);
            }
            for reserved in &self.reserved {
                reserved.store(0, Ordering::SeqCst);
            }
        }
        self.withheld.store(withheld, Ordering::SeqCst);
        self.state.store(READING, Ordering::SeqCst);
        self.wake();
        withheld == ADMITTED
    }

    fn end(&self, grow: bool) {
        self.state.store(OPEN, Ordering::SeqCst);
        self.wake();
        while self.logging.load(Ordering::SeqCst) != 0 {
            std::thread::yield_now();
        }
        if grow && self.readers.idle() {
            // SAFETY: open, with no writer logging and no reader holding a pre-image; a reader
            // entering now sees the store open and borrows nothing.
            unsafe { self.grow() };
        }
    }

    fn wake(&self) {
        unsafe { WakeByAddressAll(self.state.as_ptr().cast()) };
    }

    fn wait_while(&self, state: u32, deadline: Option<Instant>) {
        while self.state.load(Ordering::SeqCst) == state {
            let timeout = match deadline {
                None => 100,
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return;
                    }
                    u32::try_from(left.as_millis()).map_or(100, |ms| ms.clamp(1, 100))
                }
            };
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
    /// Open, with no writer logging and no reader holding a pre-image.
    unsafe fn grow(&self) {
        let storage = unsafe { &mut *self.storage.get() };
        let [records, entries, text] = self.demand.each_ref().map(|d| d.load(Ordering::SeqCst));
        if records <= storage.records.len()
            && entries <= storage.entries.len()
            && text <= storage.text.len()
        {
            return;
        }
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

    fn before_change(&self, array: usize, memory: &impl Memory) {
        let depth = WRITING.get();
        WRITING.set(depth + 1);
        let mut waited = depth != 0;
        loop {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            match self.state.load(Ordering::SeqCst) {
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
                        collector_timing::hook_timing::Family::PortraitChanges as usize,
                    );
                    self.wait_while(READING, None);
                }
                _ => return,
            }
        }
    }

    fn after_change(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        WRITING.set(WRITING.get() - 1);
    }

    fn log(&self, array: usize, memory: &impl Memory) -> Log {
        self.logging.fetch_add(1, Ordering::SeqCst);
        let log = if self.state.load(Ordering::SeqCst) != READING
            || self.withheld.load(Ordering::SeqCst) != ADMITTED
        {
            Log::Inactive
        } else if self.find(array).is_some() {
            Log::Logged
        } else {
            self.record(array, memory)
        };
        self.logging.fetch_sub(1, Ordering::SeqCst);
        log
    }

    fn find(&self, array: usize) -> Option<&Record> {
        // SAFETY: replaced only while no writer logs and no reader holds a pre-image.
        let storage = unsafe { &*self.storage.get() };
        let len = self.reserved[RECORDS].load(Ordering::SeqCst);
        storage.records[..len.min(storage.records.len())]
            .iter()
            .find(|record| {
                // SAFETY: a ready record's fields were written before it was published.
                record.ready.load(Ordering::Acquire) && unsafe { *record.array.get() } == array
            })
    }

    fn record(&self, array: usize, memory: &impl Memory) -> Log {
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

    fn reserve(&self, part: usize, count: usize) -> usize {
        let first = self.reserved[part].fetch_add(count, Ordering::SeqCst);
        self.demand[part].fetch_max(first.saturating_add(count), Ordering::SeqCst);
        first
    }

    fn copy(&self, array: usize, memory: &impl Memory, storage: &Storage) -> Option<Logged> {
        use engine_native_politics::character_portrait_entry as entry;
        use engine_native_memory::{msvc_string, read};
        let unreadable = Some(Logged::Unreadable);
        let Some(header) = read::<[u8; 16]>(memory, array) else {
            return unreadable;
        };
        let Some(holder) = engine_native_memory::cpdx_array::Array::from_header(header, entry::STRIDE)
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

impl engine_native_politics::portrait_cache::PreImages for PreImages {
    fn image(&self, array: usize) -> Option<Result<Reading<'_>, &'static str>> {
        let reader = self.readers.enter();
        if self.state.load(Ordering::SeqCst) != READING {
            return None;
        }
        match self.withheld.load(Ordering::SeqCst) {
            ADMITTED => {}
            WRITER_RUNNING => {
                return Some(Err(
                    "character portrait writer still running when the capture began",
                ));
            }
            _ => {
                return Some(Err(
                    "character portrait pre-image still read when the capture began",
                ));
            }
        }
        let record = self.find(array)?;
        // SAFETY: `reader` is counted before the state was seen reading, so no capture that
        // begins and no growth while it lives rewrites the published record or its slots.
        Some(match unsafe { *record.image.get() } {
            Logged::Unreadable => Err("character portrait pre-image unreadable"),
            Logged::Holder {
                header,
                first,
                count,
            } => {
                let storage = unsafe { &*self.storage.get() };
                let entries: *const [UnsafeCell<EntryImage>] =
                    &storage.entries[first..first + count];
                let entries = unsafe { &*(entries as *const [EntryImage]) };
                // SAFETY: the record's writer filled each of its entries' text ranges before
                // publishing it, and they stay unchanged until the next capture begins.
                Ok(Reading::new(reader, unsafe {
                    HolderImage::published(
                        header,
                        entries,
                        UnsafeCell::raw_get(storage.text.as_ptr()).cast_const(),
                        storage.text.len(),
                    )
                }))
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

pub struct PreImagesCapture;
impl PreImagesCapture {
    pub fn begin() -> Self {
        if !PRE_IMAGES.begin() {
            let reader = PRE_IMAGES.withheld.load(Ordering::SeqCst) == READER_HOLDING;
            WITHHELD[usize::from(reader)].fetch_add(1, Ordering::Relaxed);
        }
        Self
    }
}
impl Drop for PreImagesCapture {
    fn drop(&mut self) {
        PRE_IMAGES.end(true);
    }
}

pub fn withheld_captures() -> [u64; 2] {
    WITHHELD
        .each_ref()
        .map(|count| count.load(Ordering::Relaxed))
}

#[cfg(any(test, feature = "test-support"))]
pub fn pre_images_active() -> bool {
    PRE_IMAGES.state.load(Ordering::SeqCst) == READING
}

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
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
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
                let (at, original) = collector_hooks::located_hook(base, site.rva)?;
                let hook = unsafe { Hook::new(at, callbacks[index], original) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            engine_native_politics::portrait_cache::install(&PRE_IMAGES);
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        // SAFETY: PortraitChanges owns its hook family slot and coordinates its pre-image protocol.
        unsafe {
            collector_timing::capture_gate::exempt(
                collector_timing::hook_timing::Family::PortraitChanges as usize,
                enabled && !self.hooks.is_empty(),
            );
        }
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

unsafe extern "system" fn assign(array: usize, result: usize, key: usize, value: usize) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::PortraitChanges,
    );
    let _mutation = Mutation::change(array, 0);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(array, result, key, value) })
}
unsafe extern "system" fn copy(holder: usize, source: usize) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::PortraitChanges,
    );
    let _mutation = Mutation::change(holder.wrapping_add(8), 1);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder, source) })
}
unsafe extern "system" fn construct(holder: usize) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::PortraitChanges,
    );
    let _mutation = Mutation::create(holder.wrapping_add(8), 2);
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder) })
}
unsafe extern "system" fn copy_construct(holder: usize, source: usize) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::PortraitChanges,
    );
    let _mutation = Mutation::create(holder.wrapping_add(8), 3);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder, source) })
}
unsafe extern "system" fn destroy(holder: usize) {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::PortraitChanges,
    );
    let _mutation = Mutation::change(holder.wrapping_add(8), 4);
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[4].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder) })
}
unsafe extern "system" fn insert(array: usize, at: i32, source: usize, count: i32) {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::PortraitChanges,
    );
    let _mutation = Mutation::change(array, 5);
    let original: unsafe extern "system" fn(usize, i32, usize, i32) =
        unsafe { std::mem::transmute(ORIGINAL[5].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(array, at, source, count) })
}

#[cfg(test)]
mod revision_tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use engine_native_politics::portrait_cache::PreImages as _;

    const ENTRY: usize = 0x100;

    fn holder(base: usize, name: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 0x200];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        put(0, &((base + ENTRY) as u64).to_le_bytes());
        put(8, &1_u32.to_le_bytes());
        put(12, &1_u32.to_le_bytes());
        put(ENTRY, &4_u32.to_le_bytes());
        put(ENTRY + 8, &[1]);
        put(ENTRY + 0x10, name);
        put(ENTRY + 0x20, &(name.len() as u64).to_le_bytes());
        put(ENTRY + 0x28, &15_u64.to_le_bytes());
        bytes
    }

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

    #[test]
    fn the_first_change_in_a_capture_logs_the_holder_as_it_was() {
        const ARRAY: usize = 0x1000;
        let bytes = holder(ARRAY, b"x.dds");
        let mut changed = bytes.clone();
        changed[ENTRY..ENTRY + 4].copy_from_slice(&9_u32.to_le_bytes());
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
        assert!(store.begin());
        assert_eq!(store.log(ARRAY, &before), Log::Full);
        assert!(store.image(ARRAY).is_none());
        store.end(true);
        assert!(store.begin());
        assert_eq!(store.log(ARRAY, &before), Log::Logged);
        assert_eq!(store.log(ARRAY, &after), Log::Logged);
        let reading = store.image(ARRAY).unwrap().unwrap();
        let image = reading.image();
        assert_eq!(image.header[..], bytes[..16]);
        assert_eq!(image.entries().len(), 1);
        assert_eq!(image.entries()[0].head[..], bytes[ENTRY..ENTRY + 16]);
        assert_eq!(image.identity(0).unwrap(), b"x.dds");
        drop(reading);
        assert!(store.image(ARRAY + 8).is_none());
        store.end(true);
        assert!(store.image(ARRAY).is_none());
        assert!(store.begin());
        assert_eq!(store.log(ARRAY + 0x1000, &before), Log::Logged);
        assert!(store.image(ARRAY + 0x1000).unwrap().is_err());
        assert!(store.image(ARRAY).is_none());
        store.end(false);
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
            patient.in_flight.fetch_sub(1, Ordering::SeqCst);
            assert!(begin.join().unwrap());
            late.join().unwrap();
        });
        assert!(patient.image(ARRAY + 0x1000).is_none());
        assert_eq!(
            patient.image(ARRAY).unwrap().unwrap().image().header[..],
            bytes[..16]
        );
        patient.in_flight.fetch_sub(1, Ordering::SeqCst);
        patient.end(false);
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
    fn a_held_pre_image_keeps_its_bytes_until_its_reader_returns() {
        const FIRST: usize = 0x1000;
        const SECOND: usize = 0x3000;
        let (first, second) = (holder(FIRST, b"x.dds"), holder(SECOND, b"y.dds"));
        let first = SliceMemory {
            base: FIRST,
            bytes: &first,
        };
        let second = SliceMemory {
            base: SECOND,
            bytes: &second,
        };
        let text = |store: &PreImages| unsafe { (*store.storage.get()).text.as_ptr() };
        let store = PreImages::new(Duration::ZERO);
        assert!(store.begin());
        assert_eq!(store.log(FIRST, &first), Log::Full);
        store.end(true);
        assert!(store.begin());
        assert_eq!(store.log(FIRST, &first), Log::Logged);
        let reading = store.image(FIRST).unwrap().unwrap();
        store.demand[TEXT].fetch_add(1, Ordering::SeqCst);
        let held = text(&store);
        store.end(true);
        assert_eq!(text(&store), held);
        assert!(!store.begin());
        assert!(store.find(FIRST).is_some());
        assert_eq!(store.log(SECOND, &second), Log::Inactive);
        assert!(store.image(SECOND).unwrap().is_err());
        assert!(store.image(FIRST).unwrap().is_err());
        store.end(true);
        assert_eq!(text(&store), held);
        assert_eq!(reading.image().identity(0).unwrap(), b"x.dds");
        drop(reading);
        assert!(store.begin());
        assert_eq!(store.log(SECOND, &second), Log::Logged);
        assert_eq!(
            store
                .image(SECOND)
                .unwrap()
                .unwrap()
                .image()
                .identity(0)
                .unwrap(),
            b"y.dds"
        );
        store.end(true);
        assert_ne!(text(&store), held);
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
