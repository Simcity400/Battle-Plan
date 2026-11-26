use crate::{
    hook::{Hook, Registers, Tap},
    output_log::{Call, Drained, Log, raise},
};
use collector_primitives::spill_stack::SpillStack;
use engine_fuel::fuel_awards as native;
use engine_native_memory::{Memory, memory::LiveMemory};
use engine_sites::Rva;
use std::{
    cell::{RefCell, UnsafeCell},
    ops::Range,
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static LOG: Log = Log::new(0);
static SPILL: Spill = Spill::new();
const SITES: usize = 10;
static BASE: AtomicUsize = AtomicUsize::new(0);
static ROOT: AtomicU64 = AtomicU64::new(1);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static REFUSAL: collector_primitives::first_reason::FirstReason =
    collector_primitives::first_reason::FirstReason::new();
static CENSUS_LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);
static COUNTERS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
static CURRENT: Mutex<Domains> = Mutex::new(Domains {
    current: None,
    retired: None,
});
static LIVE: AtomicPtr<Buffer> = AtomicPtr::new(std::ptr::null_mut());
static NAME_STRIDE: AtomicUsize = AtomicUsize::new(0);
static CURRENT_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static CURRENT_NAMES: AtomicUsize = AtomicUsize::new(0);
static ENTRY_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static ENTRY_CONSUMED: AtomicU64 = AtomicU64::new(0);
static NAME_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static NAME_CONSUMED: AtomicU64 = AtomicU64::new(0);
static ENTRY_DEMAND: AtomicUsize = AtomicUsize::new(0);
static NAME_DEMAND: AtomicUsize = AtomicUsize::new(0);
static APPEND_REFUSALS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static APPEND_CAUSES: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
const SETTLED_NAMES: [&str; 8] = [
    "fuel root observation epoch changed",
    "fuel root reset during award",
    "fuel root recipient read refused",
    "fuel root recipient receipt lost",
    "fuel receipts do not conserve native consumer totals",
    "fuel nested award replaced an unfinished root",
    "fuel settled native read refused",
    "fuel callback tag unsupported",
];
static SETTLED_CAUSES: [AtomicU64; SETTLED_NAMES.len()] =
    [const { AtomicU64::new(0) }; SETTLED_NAMES.len()];
static FIRST_APPEND_REFUSAL: AtomicUsize = AtomicUsize::new(0);
static LOST_DRAINS: AtomicU64 = AtomicU64::new(0);
static DRAIN_REFUSALS: AtomicU64 = AtomicU64::new(0);
static FIRST_DRAIN_REFUSAL: OnceLock<&'static str> = OnceLock::new();
static SIZING: Mutex<()> = Mutex::new(());
static OFFERED: AtomicPtr<Buffer> = AtomicPtr::new(std::ptr::null_mut());
static OFFERED_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static OFFERED_NAMES: AtomicUsize = AtomicUsize::new(0);
static OFFERED_STRIDE: AtomicUsize = AtomicUsize::new(0);
static CURRENT_STRIDE: AtomicUsize = AtomicUsize::new(0);
static PACKED_NAMES: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicPtr<Buffer> = AtomicPtr::new(std::ptr::null_mut());

#[derive(Clone, Copy, Debug, Default)]
pub struct Entry {
    pub root: u64,
    pub country: u32,
    pub recipient: Option<native::Recipient>,
    pub amount: i64,
    pub started: native::Started,
    pub settled: native::Settled,
    pub refused: bool,
    pub reset: bool,
    pub name: Option<(usize, usize)>,
    pub name_reason: Option<&'static str>,
}

pub(crate) const NO_NAME: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RootData {
    pub started: native::Started,
    pub settled: native::Settled,
    pub amount: i64,
    pub name: Option<(usize, usize)>,
    pub name_reason: Option<&'static str>,
    pub refused: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CompactPayload {
    Recipient {
        reference: u64,
        host: u64,
        amount: i64,
        name_reason: Option<&'static str>,
        country: u32,
        owner: u32,
        kind: u32,
        name_start: u32,
        name_len: u32,
    },
    Root {
        country: u32,
        index: u32,
    },
    Reset {
        country: u32,
        status: usize,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CompactEntry {
    pub root: u64,
    pub payload: CompactPayload,
}

impl Default for CompactEntry {
    fn default() -> Self {
        Self {
            root: 0,
            payload: CompactPayload::Reset {
                country: 0,
                status: 0,
            },
        }
    }
}

impl CompactEntry {
    pub(crate) fn from_entry(entry: Entry, roots: &mut Vec<RootData>) -> std::io::Result<Self> {
        if entry.reset {
            return Ok(Self {
                root: entry.root,
                payload: CompactPayload::Reset {
                    country: entry.country,
                    status: entry.started.status,
                },
            });
        }
        if let Some(recipient) = entry.recipient {
            let (name_start, name_len) = match entry.name {
                Some((start, len)) => (
                    u32::try_from(start).unwrap_or(NO_NAME),
                    u32::try_from(len).unwrap_or(0),
                ),
                None => (NO_NAME, 0),
            };
            return Ok(Self {
                root: entry.root,
                payload: CompactPayload::Recipient {
                    reference: recipient.reference,
                    amount: entry.amount,
                    name_reason: entry.name_reason,
                    country: entry.country,
                    owner: recipient.owner,
                    host: recipient.host,
                    name_start,
                    name_len,
                    kind: recipient.kind,
                },
            });
        }
        let index = u32::try_from(roots.len()).map_err(|_| engine_allocation::oom())?;
        engine_allocation::reserve(roots, 1)?;
        roots.push(RootData {
            started: entry.started,
            settled: entry.settled,
            amount: entry.amount,
            name: entry.name,
            name_reason: entry.name_reason,
            refused: entry.refused,
        });
        Ok(Self {
            root: entry.root,
            payload: CompactPayload::Root {
                country: entry.country,
                index,
            },
        })
    }

    fn name_span(&self, root_data: Option<&RootData>) -> Option<(usize, usize)> {
        match self.payload {
            CompactPayload::Recipient {
                name_start,
                name_len,
                ..
            } => (name_start != NO_NAME).then_some((name_start as usize, name_len as usize)),
            CompactPayload::Root { .. } => root_data.and_then(|root| root.name),
            CompactPayload::Reset { .. } => None,
        }
    }

    fn into_entry(self, root_data: Option<RootData>) -> Entry {
        match self.payload {
            CompactPayload::Reset { country, status } => Entry {
                root: self.root,
                country,
                reset: true,
                started: native::Started {
                    status,
                    ..Default::default()
                },
                ..Default::default()
            },
            CompactPayload::Recipient {
                reference,
                amount,
                name_reason,
                country,
                owner,
                host,
                name_start,
                name_len,
                kind,
            } => Entry {
                root: self.root,
                country,
                recipient: Some(native::Recipient {
                    kind,
                    reference,
                    owner,
                    host,
                }),
                amount,
                name: (name_start != NO_NAME).then_some((name_start as usize, name_len as usize)),
                name_reason,
                ..Default::default()
            },
            CompactPayload::Root { country, .. } => {
                let data = root_data.unwrap_or_default();
                Entry {
                    root: self.root,
                    country,
                    amount: data.amount,
                    started: data.started,
                    settled: data.settled,
                    refused: data.refused,
                    name: data.name,
                    name_reason: data.name_reason,
                    ..Default::default()
                }
            }
        }
    }
}
struct Spill {
    pushed: SpillStack<Pushed>,
    held: Mutex<Spilled>,
    lost: AtomicU64,
}
struct Pushed {
    index: u64,
    entry: Entry,
    name_required: usize,
    name: Vec<u8>,
}
struct Spilled {
    entries: Vec<(u64, Entry, usize)>,
    names: Vec<u8>,
}
impl Spilled {
    fn discard_before(&mut self, index: u64) {
        self.entries.retain(|&(at, ..)| at >= index);
        if self.entries.is_empty() {
            self.names.clear();
        }
    }
}
impl Spill {
    const fn new() -> Self {
        Self {
            pushed: SpillStack::new(),
            held: Mutex::new(Spilled {
                entries: Vec::new(),
                names: Vec::new(),
            }),
            lost: AtomicU64::new(u64::MAX),
        }
    }
    fn held(&self) -> MutexGuard<'_, Spilled> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let Spilled { entries, names } = &mut *held;
        for pushed in self.pushed.take() {
            if engine_allocation::reserve(entries, 1)
                .and_then(|()| engine_allocation::reserve(names, pushed.name.len()))
                .is_err()
            {
                self.lost.fetch_min(pushed.index, Ordering::AcqRel);
                continue;
            }
            let mut entry = pushed.entry;
            if let Some((_, len)) = entry.name {
                entry.name = Some((names.len(), len));
                names.extend_from_slice(&pushed.name);
            }
            entries.push((pushed.index, entry, pushed.name_required));
        }
        held
    }
    fn push(
        &self,
        index: u64,
        mut entry: Entry,
        name: Option<native::StoredName>,
        memory: &impl Memory,
    ) -> bool {
        let name_required = name.as_ref().map_or(0, native::StoredName::byte_len);
        let mut bytes = Vec::new();
        if engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut bytes, name_required))
            .is_err()
        {
            self.lost.fetch_min(index, Ordering::AcqRel);
            return false;
        }
        if let Some(name) = name {
            bytes.resize(name_required, 0);
            match name.copy(memory, &mut bytes) {
                Ok(()) => entry.name = Some((0, name_required)),
                Err(reason) => {
                    bytes.clear();
                    entry.name_reason = Some(reason);
                }
            }
        }
        let refusal = entry.name_reason;
        let pushed = Pushed {
            index,
            entry,
            name_required,
            name: bytes,
        };
        if self.pushed.push(pushed).is_err() {
            self.lost.fetch_min(index, Ordering::AcqRel);
            return false;
        }
        if let Some(reason) = refusal {
            REFUSAL.record(reason);
            COUNTERS[8].fetch_add(1, Ordering::Relaxed);
        }
        true
    }
    fn push_bytes(
        &self,
        index: u64,
        mut entry: Entry,
        name_required: usize,
        name_bytes: Option<&[u8]>,
    ) -> bool {
        let mut bytes = Vec::new();
        let copy_len = name_bytes.map_or(0, <[u8]>::len);
        if engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut bytes, copy_len))
            .is_err()
        {
            self.lost.fetch_min(index, Ordering::AcqRel);
            return false;
        }
        if let Some(src) = name_bytes {
            bytes.extend_from_slice(src);
            entry.name = Some((0, src.len()));
        }
        let refusal = entry.name_reason;
        let pushed = Pushed {
            index,
            entry,
            name_required,
            name: bytes,
        };
        if self.pushed.push(pushed).is_err() {
            self.lost.fetch_min(index, Ordering::AcqRel);
            return false;
        }
        if let Some(reason) = refusal {
            REFUSAL.record(reason);
            COUNTERS[8].fetch_add(1, Ordering::Relaxed);
        }
        true
    }
    fn release(&self) {
        drop(self.pushed.take());
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = Spilled {
            entries: Vec::new(),
            names: Vec::new(),
        };
        self.lost.store(u64::MAX, Ordering::Release);
    }
}
const STORAGE_FAILED: &str = "fuel receipt storage allocation failed";

struct Slot {
    entry: UnsafeCell<CompactEntry>,
    name_required: UnsafeCell<u32>,
    published: AtomicU64,
}
unsafe impl Sync for Slot {}
struct RootSlot {
    data: UnsafeCell<RootData>,
    owner: AtomicU64,
}
unsafe impl Sync for RootSlot {}
struct Buffer {
    entries: Vec<Slot>,
    roots: Vec<RootSlot>,
    names: Vec<UnsafeCell<u8>>,
    stride: usize,
    publishers: AtomicUsize,
    next: AtomicPtr<Buffer>,
    disposed: AtomicPtr<Buffer>,
    base: AtomicU64,
    cursor: AtomicU64,
    root_cursor: AtomicUsize,
    names_demand: AtomicUsize,
    entries_required: AtomicUsize,
    _charge: crate::raw_memory::Charge,
}
unsafe impl Sync for Buffer {}
impl Buffer {
    #[cfg(test)]
    fn new(
        entries: usize,
        names: usize,
        charge: crate::raw_memory::Charge,
    ) -> std::io::Result<Self> {
        Self::with_stride(entries, names, names, charge)
    }
    fn with_stride(
        entries: usize,
        names: usize,
        stride: usize,
        mut charge: crate::raw_memory::Charge,
    ) -> std::io::Result<Self> {
        let mut slots = engine_allocation::vec(entries)?;
        slots.resize_with(entries, || Slot {
            entry: UnsafeCell::new(CompactEntry::default()),
            name_required: UnsafeCell::new(0),
            published: AtomicU64::new(0),
        });
        let root_count = entries.div_ceil(8).max(entries.min(64)).max(1);
        let mut roots = engine_allocation::vec(root_count)?;
        roots.resize_with(root_count, || RootSlot {
            data: UnsafeCell::new(RootData::default()),
            owner: AtomicU64::new(0),
        });
        let stride = stride.max(names.div_ceil(entries.max(1)));
        let count = entries
            .checked_mul(stride)
            .ok_or_else(engine_allocation::oom)?;
        let mut names = engine_allocation::vec(count)?;
        names.resize_with(count, || UnsafeCell::new(0));
        let bytes = crate::raw_memory::vector_bytes(&slots)
            .checked_add(crate::raw_memory::vector_bytes(&roots))
            .and_then(|n| n.checked_add(crate::raw_memory::vector_bytes(&names)))
            .ok_or_else(engine_allocation::oom)?;
        charge.set(crate::raw_memory::Capacity {
            payload_vectors: bytes,
            ..Default::default()
        });
        Ok(Self {
            entries: slots,
            roots,
            names,
            stride,
            publishers: AtomicUsize::new(0),
            next: AtomicPtr::new(std::ptr::null_mut()),
            disposed: AtomicPtr::new(std::ptr::null_mut()),
            base: AtomicU64::new(0),
            cursor: AtomicU64::new(0),
            root_cursor: AtomicUsize::new(0),
            names_demand: AtomicUsize::new(0),
            entries_required: AtomicUsize::new(0),
            _charge: charge,
        })
    }
    fn release_roots_up_to(&self, cursor: u64) {
        for slot in &self.roots {
            let owner = slot.owner.load(Ordering::Acquire);
            if owner != 0 && owner <= cursor {
                let _ =
                    slot.owner
                        .compare_exchange(owner, 0, Ordering::AcqRel, Ordering::Relaxed);
            }
        }
    }
}
struct Publishing<'a>(&'a Buffer);
impl Drop for Publishing<'_> {
    fn drop(&mut self) {
        self.0.publishers.fetch_sub(1, Ordering::Release);
    }
}
fn publishing(mut buffer: &Buffer) -> Option<Publishing<'_>> {
    loop {
        match buffer
            .publishers
            .fetch_update(Ordering::Acquire, Ordering::Relaxed, |n| {
                n.checked_add(1).filter(|&n| n != usize::MAX)
            }) {
            Ok(_) => return Some(Publishing(buffer)),
            Err(usize::MAX) => {
                buffer = successor(buffer)?;
            }
            Err(_) => return None,
        }
    }
}
fn successor(buffer: &Buffer) -> Option<&Buffer> {
    let next = unsafe { buffer.next.load(Ordering::Acquire).as_ref() }?;
    let frontier = buffer.cursor.load(Ordering::Acquire);
    next.base.fetch_max(frontier, Ordering::Release);
    next.cursor.fetch_max(frontier, Ordering::Release);
    Some(next)
}
struct Domains {
    current: Option<Box<Buffer>>,
    retired: Option<Box<Buffer>>,
}
#[derive(Clone, Copy)]
struct Pending {
    root: u64,
    epoch: u64,
    started: native::Started,
    located: native::Located,
    stamp: Stamp,
    received: [i64; 3],
    refused: bool,
    cause: Option<usize>,
}
#[derive(Clone, Copy)]
struct BatchedRecipient {
    kind: u32,
    pointer: usize,
    host_ptr: usize,
    recipient: native::Recipient,
    amount: i64,
    name: Option<(usize, usize)>,
    name_required: usize,
    name_reason: Option<&'static str>,
}

#[derive(Default)]
struct PendingBatch {
    items: Vec<BatchedRecipient>,
    names: Vec<u8>,
    total_name_bytes: usize,
    max_name_len: usize,
    table: Vec<u32>,
    touched: Vec<usize>,
}

impl PendingBatch {
    #[inline(always)]
    fn hash(kind: u32, pointer: usize, host_ptr: usize) -> usize {
        let mut h = (pointer as u64).wrapping_mul(0x517c_c1b7_2722_0a95)
            ^ (host_ptr as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ u64::from(kind).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h ^= h >> 32;
        h as usize
    }

    fn clear(&mut self) {
        self.items.clear();
        self.names.clear();
        self.total_name_bytes = 0;
        self.max_name_len = 0;
        if self.touched.len() * 4 < self.table.len() {
            for &slot in &self.touched {
                self.table[slot] = 0;
            }
        } else {
            self.table.fill(0);
        }
        self.touched.clear();
    }

    #[inline(always)]
    fn try_coalesce(
        &mut self,
        kind: u32,
        pointer: usize,
        host_ptr: usize,
        amount: i64,
    ) -> Option<bool> {
        let len = self.table.len();
        if len == 0 {
            return None;
        }
        let mask = len - 1;
        let mut slot = Self::hash(kind, pointer, host_ptr) & mask;
        loop {
            let entry = self.table[slot];
            if entry == 0 {
                return None;
            }
            let item = &mut self.items[(entry - 1) as usize];
            if item.kind == kind && item.pointer == pointer && item.host_ptr == host_ptr {
                if let Some(sum) = item.amount.checked_add(amount) {
                    item.amount = sum;
                    return Some(true);
                }
                return Some(false);
            }
            slot = (slot + 1) & mask;
        }
    }

    fn rehash(&mut self, new_len: usize) {
        let mut table = Vec::new();
        if engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut table, new_len))
            .is_err()
        {
            return;
        }
        table.resize(new_len, 0);
        let mask = new_len - 1;
        self.touched.clear();
        let _ = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.touched, self.items.len())
        });
        for (idx, item) in self.items.iter().enumerate() {
            let mut slot = Self::hash(item.kind, item.pointer, item.host_ptr) & mask;
            while table[slot] != 0 {
                slot = (slot + 1) & mask;
            }
            table[slot] = (idx + 1) as u32;
            if self.touched.len() < self.touched.capacity() {
                self.touched.push(slot);
            }
        }
        self.table = table;
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "batched recipient carries key fields, decoded recipient, amount, name and live memory"
    )]
    fn push(
        &mut self,
        kind: u32,
        pointer: usize,
        host_ptr: usize,
        recipient: native::Recipient,
        amount: i64,
        stored_name: Option<&native::StoredName>,
        mut name_reason: Option<&'static str>,
        insert_in_table: bool,
        memory: &impl Memory,
    ) -> bool {
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.items, 1)).is_err() {
            return false;
        }
        let name_required = stored_name.map_or(0, native::StoredName::byte_len);
        let mut name = None;
        if let Some(stored) = stored_name {
            let start = self.names.len();
            if engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.names, name_required)
            })
            .is_err()
            {
                return false;
            }
            self.names.resize(start + name_required, 0);
            match stored.copy(memory, &mut self.names[start..start + name_required]) {
                Ok(()) => name = Some((start, name_required)),
                Err(reason) => {
                    self.names.truncate(start);
                    name_reason = Some(reason);
                }
            }
        }
        let idx = self.items.len();
        self.items.push(BatchedRecipient {
            kind,
            pointer,
            host_ptr,
            recipient,
            amount,
            name,
            name_required,
            name_reason,
        });
        self.total_name_bytes = self.total_name_bytes.saturating_add(name_required);
        self.max_name_len = self.max_name_len.max(name_required);
        if insert_in_table && idx < u32::MAX as usize {
            if self.table.is_empty() {
                self.rehash(64);
            } else if self.items.len() * 2 >= self.table.len() {
                self.rehash(self.table.len().saturating_mul(2));
            }
            if !self.table.is_empty() && self.items.len() * 2 < self.table.len() {
                let mask = self.table.len() - 1;
                let mut slot = Self::hash(kind, pointer, host_ptr) & mask;
                while self.table[slot] != 0 {
                    slot = (slot + 1) & mask;
                }
                self.table[slot] = (idx + 1) as u32;
                if self.touched.len() == self.touched.capacity() {
                    let _ = engine_allocation::advisory(|| {
                        engine_allocation::reserve(&mut self.touched, 1)
                    });
                }
                if self.touched.len() < self.touched.capacity() {
                    self.touched.push(slot);
                } else {
                    self.touched.clear();
                    self.touched.resize(self.table.len(), 0);
                }
            }
        }
        true
    }
}

#[derive(Default)]
struct ThreadState {
    pending: Option<Pending>,
    batch: PendingBatch,
    registry: native::Registry,
}

thread_local! {
    static THREAD_STATE: RefCell<ThreadState> = RefCell::new(ThreadState::default());
}

fn stop_admission_and_drained() -> bool {
    LOG.enabled.store(false, Ordering::SeqCst);
    LOG.active() == 0
}

type Stamp = (u64, u64);
static HOURS: AtomicU64 = AtomicU64::new(0);
static HELD: crate::hook_timing::CallCounters<3> = crate::hook_timing::CallCounters::new();

pub fn boundary() {
    HOURS.fetch_add(1, Ordering::Relaxed);
}

fn stamp() -> Stamp {
    (
        crate::world_lifetime::generation(),
        HOURS.load(Ordering::Relaxed),
    )
}

fn held_for(pending: &Pending, epoch_current: bool) -> Option<native::Located> {
    (!pending.refused
        && epoch_current
        && pending.stamp == stamp()
        && !crate::world_lifetime::invalidating())
    .then_some(pending.located)
}

#[cfg(test)]
fn held(pending: &Pending) -> Option<native::Located> {
    held_for(pending, pending.epoch == LOG.epoch.load(Ordering::Acquire))
}

pub fn presize(memory: &impl Memory, base: usize) {
    engine_allocation::advisory(|| presize_storage(memory, base));
}

fn presize_storage(memory: &impl Memory, base: usize) {
    offer(crate::country_scale::census_rows(0).max(1), 0);
    let observed = collector_extended_support::demand::demand_air_game_state(memory, base)
        .and_then(|state| {
            state
                .map(|state| native::demand(memory, base, state))
                .transpose()
        });
    let demand = match observed {
        Ok(None) => return,
        Ok(Some(demand)) => demand,
        Err(reason) => {
            refuse_census(reason);
            offer(0, 0);
            return;
        }
    };
    NAME_STRIDE.fetch_max(demand.name_stride, Ordering::Release);
    let Some(entries) = demand
        .entries
        .checked_add(demand.entries / 4)
        .map(|n| n.max(1))
    else {
        refuse_census("fuel census entry size overflow");
        return;
    };
    let Some(names) = demand.names.checked_add(demand.names / 4) else {
        refuse_census("fuel census name size overflow");
        return;
    };
    offer(entries, names);
}

fn refuse_census(reason: &'static str) {
    REFUSAL.record(reason);
    *CENSUS_LAST_REFUSAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
}

fn offer(entries: usize, names: usize) {
    let Ok(sizing) = SIZING.lock() else { return };
    let entries = entries.max(ENTRY_DEMAND.load(Ordering::Acquire));
    let names = names.max(NAME_DEMAND.load(Ordering::Acquire));
    if entries == 0 && names == 0 {
        return;
    }
    let stride = NAME_STRIDE.load(Ordering::Acquire);
    let fits = |slots: &AtomicUsize, bytes: &AtomicUsize, width: &AtomicUsize| {
        slots.load(Ordering::Acquire) >= entries
            && bytes.load(Ordering::Acquire) >= names
            && width.load(Ordering::Acquire) >= stride
    };
    if fits(&CURRENT_ENTRIES, &CURRENT_NAMES, &CURRENT_STRIDE)
        || fits(&OFFERED_ENTRIES, &OFFERED_NAMES, &OFFERED_STRIDE)
    {
        return;
    }
    let mut entry_capacity = CURRENT_ENTRIES
        .load(Ordering::Acquire)
        .max(OFFERED_ENTRIES.load(Ordering::Acquire));
    let mut name_capacity = CURRENT_NAMES
        .load(Ordering::Acquire)
        .max(OFFERED_NAMES.load(Ordering::Acquire));
    let mut packed_names = PACKED_NAMES.load(Ordering::Acquire);
    crate::table_capture::grow_capacity(&mut entry_capacity, entries);
    crate::table_capture::grow_capacity(&mut name_capacity, names);
    crate::table_capture::grow_capacity(&mut packed_names, names);
    PACKED_NAMES.fetch_max(packed_names, Ordering::Release);
    let Ok(buffer) = Buffer::with_stride(entry_capacity, name_capacity, stride, Default::default())
        .and_then(engine_allocation::boxed)
    else {
        drop(sizing);
        refuse_census("fuel observation allocation failed");
        return;
    };
    let shape = (buffer.entries.len(), buffer.names.len(), buffer.stride);
    let displaced = OFFERED.swap(Box::into_raw(buffer), Ordering::AcqRel);
    OFFERED_ENTRIES.store(shape.0, Ordering::Release);
    OFFERED_NAMES.store(shape.1, Ordering::Release);
    OFFERED_STRIDE.store(shape.2, Ordering::Release);
    drop(sizing);
    if !displaced.is_null() {
        drop(unsafe { Box::from_raw(displaced) });
    }
}
fn dispose(buffer: Box<Buffer>) {
    let node = Box::into_raw(buffer);
    let mut head = DISPOSED.load(Ordering::Acquire);
    loop {
        unsafe { (*node).disposed.store(head, Ordering::Relaxed) };
        match DISPOSED.compare_exchange_weak(head, node, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(current) => head = current,
        }
    }
}
fn release_disposed() {
    let mut node = DISPOSED.swap(std::ptr::null_mut(), Ordering::AcqRel);
    while !node.is_null() {
        let buffer = unsafe { Box::from_raw(node) };
        node = buffer.disposed.load(Ordering::Relaxed);
        drop(buffer);
    }
}
pub fn install_offered() {
    let Ok(mut domains) = CURRENT.try_lock() else {
        return;
    };
    retire_consumed(&mut domains);
    if OFFERED.load(Ordering::Acquire).is_null()
        || domains.retired.is_some()
        || (domains.current.is_some()
            && LOG.cursor.load(Ordering::SeqCst) != LOG.consumed.load(Ordering::Relaxed))
        || LOG.active() != 0
    {
        return;
    }
    install_prepared(&mut domains);
}
fn retire_consumed(domains: &mut Domains) {
    if domains.retired.as_ref().is_some_and(|buffer| {
        LOG.consumed.load(Ordering::Acquire) >= buffer.cursor.load(Ordering::Acquire)
    }) && LOG.active() == 0
        && let Some(retired) = domains.retired.take()
    {
        dispose(retired);
    }
}
fn install_prepared(domains: &mut Domains) {
    let pointer = OFFERED.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if pointer.is_null() {
        return;
    }
    let offered = unsafe { Box::from_raw(pointer) };
    if let Some(old) = domains.current.as_ref() {
        old.next.store(pointer, Ordering::Release);
        if old
            .publishers
            .compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return_offer(offered);
            return;
        }
        let _ = successor(old);
    } else {
        let frontier = LOG.cursor.load(Ordering::SeqCst);
        offered.base.store(frontier, Ordering::Release);
        offered.cursor.store(frontier, Ordering::Release);
    }
    let capacity = offered.entries.len();
    CURRENT_ENTRIES.store(capacity, Ordering::Release);
    CURRENT_NAMES.store(offered.names.len(), Ordering::Release);
    CURRENT_STRIDE.store(offered.stride, Ordering::Release);
    LIVE.store(pointer, Ordering::SeqCst);
    domains.retired = domains.current.replace(offered);
    LOG.capacity.store(capacity, Ordering::Release);
}
fn return_offer(offered: Box<Buffer>) {
    let pointer = Box::into_raw(offered);
    if OFFERED
        .compare_exchange(
            std::ptr::null_mut(),
            pointer,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        dispose(unsafe { Box::from_raw(pointer) });
    }
}
pub fn collect() {
    release_disposed();
    offer(0, 0);
}
pub fn active() -> usize {
    LOG.active()
}

pub fn release() -> Result<(), &'static str> {
    if LOG.enabled.load(Ordering::SeqCst) || active() != 0 {
        return Err("fuel storage release before callback quiescence");
    }
    let (current, retired, offered) = {
        let mut domains = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        let sizing = SIZING.lock().unwrap_or_else(|error| error.into_inner());
        LIVE.store(std::ptr::null_mut(), Ordering::SeqCst);
        LOG.capacity.store(0, Ordering::Release);
        CURRENT_ENTRIES.store(0, Ordering::Release);
        CURRENT_NAMES.store(0, Ordering::Release);
        CURRENT_STRIDE.store(0, Ordering::Release);
        OFFERED_ENTRIES.store(0, Ordering::Release);
        OFFERED_NAMES.store(0, Ordering::Release);
        OFFERED_STRIDE.store(0, Ordering::Release);
        ENTRY_ATTEMPTS.store(0, Ordering::SeqCst);
        ENTRY_CONSUMED.store(0, Ordering::Relaxed);
        NAME_ATTEMPTS.store(0, Ordering::SeqCst);
        NAME_CONSUMED.store(0, Ordering::Relaxed);
        ENTRY_DEMAND.store(0, Ordering::Release);
        NAME_DEMAND.store(0, Ordering::Release);
        PACKED_NAMES.store(0, Ordering::Release);
        NAME_STRIDE.store(0, Ordering::Release);
        LOG.consumed
            .store(LOG.cursor.load(Ordering::SeqCst), Ordering::Relaxed);
        let offered = OFFERED.swap(std::ptr::null_mut(), Ordering::AcqRel);
        drop(sizing);
        (
            domains.current.take(),
            domains.retired.take(),
            (!offered.is_null()).then(|| unsafe { Box::from_raw(offered) }),
        )
    };
    drop(current);
    drop(retired);
    drop(offered);
    SPILL.release();
    release_disposed();
    Ok(())
}

fn attempted_since(cursor: u64, consumed: &AtomicU64) -> usize {
    usize::try_from(cursor.wrapping_sub(consumed.load(Ordering::Relaxed))).unwrap_or(usize::MAX)
}
fn refuse_append(entry: Entry, absent: bool) {
    let lane = if entry.reset {
        2
    } else if entry.recipient.is_some() {
        0
    } else {
        1
    };
    let code = lane + if absent { 4 } else { 1 };
    let _ = FIRST_APPEND_REFUSAL.compare_exchange(0, code, Ordering::Release, Ordering::Relaxed);
    APPEND_REFUSALS[lane].fetch_add(1, Ordering::Relaxed);
    APPEND_CAUSES[code - 1].fetch_add(1, Ordering::Relaxed);
    if absent {
        COUNTERS[11].fetch_add(1, Ordering::Relaxed);
    }
    LOG.invalidate();
}
fn append(
    _call: &Call<'_>,
    entry: Entry,
    name: Option<native::StoredName>,
    memory: &impl Memory,
) -> bool {
    let attempted = ENTRY_ATTEMPTS
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    raise(
        &ENTRY_DEMAND,
        attempted_since(attempted, &ENTRY_CONSUMED),
        Ordering::Release,
    );
    if let Some(name) = &name {
        let len = name.byte_len();
        let bytes = len as u64;
        let attempted = NAME_ATTEMPTS
            .fetch_add(bytes, Ordering::Relaxed)
            .wrapping_add(bytes);
        raise(
            &NAME_DEMAND,
            attempted_since(attempted, &NAME_CONSUMED),
            Ordering::Release,
        );
        raise(&NAME_STRIDE, len, Ordering::Release);
    }
    let Some(buffer) = (unsafe { LIVE.load(Ordering::Acquire).as_ref() }).and_then(publishing)
    else {
        let index = LOG.cursor.fetch_add(1, Ordering::SeqCst);
        if SPILL.push(index, entry, name, memory) {
            return true;
        }
        refuse_append(entry, true);
        return false;
    };
    let retained = append_buffer(&LOG, &SPILL, buffer.0, entry, name, memory);
    if !retained {
        refuse_append(entry, false);
    }
    retained
}
fn flush_batch_orphans(_call: &Call<'_>, batch: &mut PendingBatch, root: u64, country: u32) {
    let count = batch.items.len();
    if count == 0 {
        batch.clear();
        return;
    }
    let attempted = ENTRY_ATTEMPTS
        .fetch_add(count as u64, Ordering::Relaxed)
        .wrapping_add(count as u64);
    raise(
        &ENTRY_DEMAND,
        attempted_since(attempted, &ENTRY_CONSUMED),
        Ordering::Release,
    );
    if batch.total_name_bytes != 0 {
        let bytes = batch.total_name_bytes as u64;
        let attempted = NAME_ATTEMPTS
            .fetch_add(bytes, Ordering::Relaxed)
            .wrapping_add(bytes);
        raise(
            &NAME_DEMAND,
            attempted_since(attempted, &NAME_CONSUMED),
            Ordering::Release,
        );
        raise(&NAME_STRIDE, batch.max_name_len, Ordering::Release);
    }
    let buffer = (unsafe { LIVE.load(Ordering::Acquire).as_ref() }).and_then(publishing);
    let base_index = LOG.cursor.fetch_add(count as u64, Ordering::AcqRel);
    if let Some(ref buffer) = buffer {
        let buf = buffer.0;
        buf.cursor
            .fetch_max(base_index + count as u64, Ordering::Relaxed);
        let pending_entries = (base_index + count as u64 - 1)
            .wrapping_sub(
                LOG.consumed
                    .load(Ordering::Acquire)
                    .max(buf.base.load(Ordering::Acquire)),
            )
            .saturating_add(1);
        raise(
            &buf.entries_required,
            usize::try_from(pending_entries).unwrap_or(usize::MAX),
            Ordering::Relaxed,
        );
        let entries_len = buf.entries.len();
        let max_start = entries_len
            .checked_sub(1)
            .and_then(|last| last.checked_mul(buf.stride));
        let fast = pending_entries <= entries_len as u64
            && batch.max_name_len <= buf.stride
            && u32::try_from(batch.max_name_len).is_ok()
            && max_start.is_some_and(|m| u32::try_from(m).is_ok() && m < NO_NAME as usize);
        if fast {
            let pow2 = entries_len.is_power_of_two();
            for (offset, item) in batch.items.iter().enumerate() {
                let index = base_index + offset as u64;
                let slot = if pow2 {
                    index as usize & (entries_len - 1)
                } else {
                    index as usize % entries_len
                };
                let mut name_start = NO_NAME;
                let mut name_len = 0_u32;
                if let Some((start, len)) = item.name {
                    let dst_start = slot * buf.stride;
                    // SAFETY: `index` exclusively owns `slot` and `len <= buf.stride`.
                    let dst = unsafe {
                        std::slice::from_raw_parts_mut(
                            buf.names.as_ptr().add(dst_start).cast::<u8>().cast_mut(),
                            len,
                        )
                    };
                    dst.copy_from_slice(&batch.names[start..start + len]);
                    name_start = dst_start as u32;
                    name_len = len as u32;
                }
                if let Some(reason) = item.name_reason {
                    REFUSAL.record(reason);
                    COUNTERS[8].fetch_add(1, Ordering::Relaxed);
                }
                let compact = CompactEntry {
                    root,
                    payload: CompactPayload::Recipient {
                        reference: item.recipient.reference,
                        amount: item.amount,
                        name_reason: item.name_reason,
                        country,
                        owner: item.recipient.owner,
                        host: item.recipient.host,
                        name_start,
                        name_len,
                        kind: item.recipient.kind,
                    },
                };
                // SAFETY: `index` exclusively owns `buf.entries[slot]`.
                unsafe {
                    *buf.entries[slot].entry.get() = compact;
                    *buf.entries[slot].name_required.get() = item.name_required as u32;
                }
                buf.entries[slot]
                    .published
                    .store(index + 1, Ordering::Release);
            }
            batch.clear();
            return;
        }
    }
    let absent = buffer.is_none();
    for (offset, item) in batch.items.iter().enumerate() {
        let entry = Entry {
            root,
            country,
            recipient: Some(item.recipient),
            amount: item.amount,
            name_reason: item.name_reason,
            ..Default::default()
        };
        let bytes = item.name.map(|(s, l)| &batch.names[s..s + l]);
        if !SPILL.push_bytes(base_index + offset as u64, entry, item.name_required, bytes) {
            refuse_append(entry, absent);
        }
    }
    batch.clear();
}

fn append_batch(
    _call: &Call<'_>,
    batch: &mut PendingBatch,
    pending: Pending,
    settled: native::Settled,
    memory: &impl Memory,
) {
    let items_len = batch.items.len();
    let count = items_len + 1;
    let attempted = ENTRY_ATTEMPTS
        .fetch_add(count as u64, Ordering::Relaxed)
        .wrapping_add(count as u64);
    raise(
        &ENTRY_DEMAND,
        attempted_since(attempted, &ENTRY_CONSUMED),
        Ordering::Release,
    );
    if batch.total_name_bytes != 0 {
        let bytes = batch.total_name_bytes as u64;
        let attempted = NAME_ATTEMPTS
            .fetch_add(bytes, Ordering::Relaxed)
            .wrapping_add(bytes);
        raise(
            &NAME_DEMAND,
            attempted_since(attempted, &NAME_CONSUMED),
            Ordering::Release,
        );
        raise(&NAME_STRIDE, batch.max_name_len, Ordering::Release);
    }
    let root_entry = Entry {
        root: pending.root,
        country: pending.started.country,
        started: pending.started,
        settled,
        refused: pending.refused,
        ..Default::default()
    };
    let buffer = (unsafe { LIVE.load(Ordering::Acquire).as_ref() }).and_then(publishing);
    let base_index = LOG.cursor.fetch_add(count as u64, Ordering::AcqRel);
    let root_index = base_index + items_len as u64;
    let mut retained = true;
    let mut published_fast = false;
    if let Some(buffer) = &buffer {
        let buf = buffer.0;
        buf.cursor
            .fetch_max(base_index + count as u64, Ordering::Relaxed);
        let pending_entries = root_index
            .wrapping_sub(
                LOG.consumed
                    .load(Ordering::Acquire)
                    .max(buf.base.load(Ordering::Acquire)),
            )
            .saturating_add(1);
        raise(
            &buf.entries_required,
            usize::try_from(pending_entries).unwrap_or(usize::MAX),
            Ordering::Relaxed,
        );
        let entries_len = buf.entries.len();
        let max_start = entries_len
            .checked_sub(1)
            .and_then(|last| last.checked_mul(buf.stride));
        let fast = pending_entries <= entries_len as u64
            && batch.max_name_len <= buf.stride
            && u32::try_from(batch.max_name_len).is_ok()
            && max_start.is_some_and(|m| u32::try_from(m).is_ok() && m < NO_NAME as usize);
        if fast {
            let root_count = buf.roots.len();
            let start_pos = buf.root_cursor.fetch_add(1, Ordering::Relaxed) % root_count;
            let owner = root_index + 1;
            let mut claimed = None;
            for step in 0..root_count {
                let pos = (start_pos + step) % root_count;
                let root_slot = &buf.roots[pos];
                if root_slot.owner.load(Ordering::Relaxed) == 0
                    && root_slot
                        .owner
                        .compare_exchange(0, owner, Ordering::Acquire, Ordering::Relaxed)
                        .is_ok()
                {
                    claimed = Some(pos);
                    break;
                }
            }
            if let Some(root_pos) = claimed {
                let pow2 = entries_len.is_power_of_two();
                for (offset, item) in batch.items.iter().enumerate() {
                    let index = base_index + offset as u64;
                    let slot = if pow2 {
                        index as usize & (entries_len - 1)
                    } else {
                        index as usize % entries_len
                    };
                    let mut name_start = NO_NAME;
                    let mut name_len = 0_u32;
                    if let Some((start, len)) = item.name {
                        let dst_start = slot * buf.stride;
                        // SAFETY: `index` exclusively owns `slot` and `len <= buf.stride`.
                        let dst = unsafe {
                            std::slice::from_raw_parts_mut(
                                buf.names.as_ptr().add(dst_start).cast::<u8>().cast_mut(),
                                len,
                            )
                        };
                        dst.copy_from_slice(&batch.names[start..start + len]);
                        name_start = dst_start as u32;
                        name_len = len as u32;
                    }
                    if let Some(reason) = item.name_reason {
                        REFUSAL.record(reason);
                        COUNTERS[8].fetch_add(1, Ordering::Relaxed);
                    }
                    let compact = CompactEntry {
                        root: pending.root,
                        payload: CompactPayload::Recipient {
                            reference: item.recipient.reference,
                            amount: item.amount,
                            name_reason: item.name_reason,
                            country: pending.started.country,
                            owner: item.recipient.owner,
                            host: item.recipient.host,
                            name_start,
                            name_len,
                            kind: item.recipient.kind,
                        },
                    };
                    // SAFETY: `index` exclusively owns `buf.entries[slot]`.
                    unsafe {
                        *buf.entries[slot].entry.get() = compact;
                        *buf.entries[slot].name_required.get() = item.name_required as u32;
                    }
                    buf.entries[slot]
                        .published
                        .store(index + 1, Ordering::Release);
                }
                // SAFETY: `buf.roots[root_pos].owner` was exclusively claimed from 0 to `root_index + 1`.
                unsafe {
                    *buf.roots[root_pos].data.get() = RootData {
                        started: root_entry.started,
                        settled: root_entry.settled,
                        amount: root_entry.amount,
                        name: root_entry.name,
                        name_reason: root_entry.name_reason,
                        refused: root_entry.refused,
                    };
                }
                let root_slot = if pow2 {
                    root_index as usize & (entries_len - 1)
                } else {
                    root_index as usize % entries_len
                };
                let compact = CompactEntry {
                    root: root_entry.root,
                    payload: CompactPayload::Root {
                        country: root_entry.country,
                        index: root_pos as u32,
                    },
                };
                // SAFETY: `root_index` exclusively owns `buf.entries[root_slot]`.
                unsafe {
                    *buf.entries[root_slot].entry.get() = compact;
                    *buf.entries[root_slot].name_required.get() = 0;
                }
                buf.entries[root_slot]
                    .published
                    .store(root_index + 1, Ordering::Release);
                published_fast = true;
            }
        }
    }
    if !published_fast {
        let absent = buffer.is_none();
        for (offset, item) in batch.items.iter().enumerate() {
            let entry = Entry {
                root: pending.root,
                country: pending.started.country,
                recipient: Some(item.recipient),
                amount: item.amount,
                name_reason: item.name_reason,
                ..Default::default()
            };
            let bytes = item.name.map(|(s, l)| &batch.names[s..s + l]);
            if !SPILL.push_bytes(base_index + offset as u64, entry, item.name_required, bytes) {
                refuse_append(entry, absent);
                retained = false;
            }
        }
        if !SPILL.push(root_index, root_entry, None, memory) {
            refuse_append(root_entry, absent);
            retained = false;
        }
    }
    batch.clear();
    if pending.refused || !retained {
        if let Some(cause) = pending.cause {
            SETTLED_CAUSES[cause].fetch_add(1, Ordering::Relaxed);
        }
        COUNTERS[2].fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
fn append_root(call: &Call<'_>, pending: Pending, settled: native::Settled, memory: &impl Memory) {
    let retained = append(
        call,
        Entry {
            root: pending.root,
            country: pending.started.country,
            started: pending.started,
            settled,
            refused: pending.refused,
            ..Default::default()
        },
        None,
        memory,
    );
    if pending.refused || !retained {
        if let Some(cause) = pending.cause {
            SETTLED_CAUSES[cause].fetch_add(1, Ordering::Relaxed);
        }
        COUNTERS[2].fetch_add(1, Ordering::Relaxed);
    }
}
fn append_buffer(
    log: &Log,
    spill: &Spill,
    buffer: &Buffer,
    mut entry: Entry,
    name: Option<native::StoredName>,
    memory: &impl Memory,
) -> bool {
    let index = log.cursor.fetch_add(1, Ordering::AcqRel);
    buffer.cursor.fetch_max(index + 1, Ordering::Relaxed);
    let pending = index
        .wrapping_sub(
            log.consumed
                .load(Ordering::Acquire)
                .max(buffer.base.load(Ordering::Acquire)),
        )
        .saturating_add(1);
    raise(
        &buffer.entries_required,
        usize::try_from(pending).unwrap_or(usize::MAX),
        Ordering::Relaxed,
    );
    if pending > buffer.entries.len() as u64
        || name
            .as_ref()
            .is_some_and(|name| name.byte_len() > buffer.stride)
    {
        return spill.push(index, entry, name, memory);
    }
    let entries_len = buffer.entries.len();
    let slot = if entries_len.is_power_of_two() {
        index as usize & (entries_len - 1)
    } else {
        index as usize % entries_len
    };
    let name_required = name.as_ref().map_or(0, native::StoredName::byte_len);
    let Ok(name_required_u32) = u32::try_from(name_required) else {
        return spill.push(index, entry, name, memory);
    };
    let start = slot * buffer.stride;
    if name.is_some()
        && (u32::try_from(start).is_err() || start == NO_NAME as usize)
    {
        return spill.push(index, entry, name, memory);
    }
    let compact = if entry.reset {
        CompactEntry {
            root: entry.root,
            payload: CompactPayload::Reset {
                country: entry.country,
                status: entry.started.status,
            },
        }
    } else if let Some(recipient) = entry.recipient {
        let mut name_start = NO_NAME;
        let mut name_len = 0_u32;
        if let Some(name) = name {
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(
                    buffer.names.as_ptr().add(start).cast::<u8>().cast_mut(),
                    name.byte_len(),
                )
            };
            match name.copy(memory, bytes) {
                Ok(()) => {
                    name_start = start as u32;
                    name_len = name_required_u32;
                }
                Err(reason) => entry.name_reason = Some(reason),
            }
        }
        CompactEntry {
            root: entry.root,
            payload: CompactPayload::Recipient {
                reference: recipient.reference,
                amount: entry.amount,
                name_reason: entry.name_reason,
                country: entry.country,
                owner: recipient.owner,
                host: recipient.host,
                name_start,
                name_len,
                kind: recipient.kind,
            },
        }
    } else {
        let root_count = buffer.roots.len();
        let start_pos = buffer.root_cursor.fetch_add(1, Ordering::Relaxed) % root_count;
        let owner = index + 1;
        let mut claimed = None;
        for step in 0..root_count {
            let pos = (start_pos + step) % root_count;
            let root_slot = &buffer.roots[pos];
            if root_slot.owner.load(Ordering::Relaxed) == 0
                && root_slot
                    .owner
                    .compare_exchange(0, owner, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
            {
                claimed = Some(pos);
                break;
            }
        }
        let Some(root_pos) = claimed else {
            return spill.push(index, entry, name, memory);
        };
        if let Some(name) = name {
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(
                    buffer.names.as_ptr().add(start).cast::<u8>().cast_mut(),
                    name.byte_len(),
                )
            };
            match name.copy(memory, bytes) {
                Ok(()) => entry.name = Some((start, name.byte_len())),
                Err(reason) => entry.name_reason = Some(reason),
            }
        }
        // SAFETY: `buffer.roots[root_pos].owner` was exclusively claimed from 0 to `index + 1`.
        unsafe {
            *buffer.roots[root_pos].data.get() = RootData {
                started: entry.started,
                settled: entry.settled,
                amount: entry.amount,
                name: entry.name,
                name_reason: entry.name_reason,
                refused: entry.refused,
            };
        }
        CompactEntry {
            root: entry.root,
            payload: CompactPayload::Root {
                country: entry.country,
                index: root_pos as u32,
            },
        }
    };
    if let Some(reason) = entry.name_reason {
        REFUSAL.record(reason);
        COUNTERS[8].fetch_add(1, Ordering::Relaxed);
    }
    unsafe {
        *buffer.entries[slot].entry.get() = compact;
        *buffer.entries[slot].name_required.get() = name_required_u32;
    }
    buffer.entries[slot]
        .published
        .store(index + 1, Ordering::Release);
    true
}

#[cfg(feature = "callback_profile")]
const TAG_NAMES: [&str; 10] = [
    "begin", "end", "land_0", "land_1", "land_2", "land_3", "land_4", "naval", "air", "reset",
];
#[cfg(feature = "callback_profile")]
static TAG_COST: [[AtomicU64; 2]; TAG_NAMES.len()] =
    [const { [const { AtomicU64::new(0) }; 2] }; TAG_NAMES.len()];
#[cfg(feature = "callback_profile")]
struct TagTimer {
    tag: usize,
    start: u64,
}
#[cfg(feature = "callback_profile")]
impl TagTimer {
    fn new(tag: usize) -> Self {
        let start = unsafe { core::arch::x86_64::_rdtsc() };
        Self { tag, start }
    }
}
#[cfg(feature = "callback_profile")]
impl Drop for TagTimer {
    fn drop(&mut self) {
        let ticks = unsafe { core::arch::x86_64::_rdtsc() }.saturating_sub(self.start);
        if let Some([calls, total]) = TAG_COST.get(self.tag) {
            calls.fetch_add(1, Ordering::Relaxed);
            total.fetch_add(ticks, Ordering::Relaxed);
        }
    }
}
#[cfg(feature = "callback_profile")]
pub fn tag_profile() -> impl serde::Serialize {
    let hz = crate::hook_timing::tsc_hz().unwrap_or(0);
    crate::status::map(
        TAG_NAMES
            .into_iter()
            .zip(&TAG_COST)
            .map(move |(name, [calls, ticks])| {
                let ticks = u128::from(ticks.load(Ordering::Relaxed));
                let ns = ticks
                    .saturating_mul(1_000_000_000)
                    .checked_div(u128::from(hz))
                    .map_or(u64::MAX, |ns| u64::try_from(ns).unwrap_or(u64::MAX));
                (name, [calls.load(Ordering::Relaxed), ns])
            }),
    )
}

unsafe extern "system-unwind" fn observe(registers: &Registers, tag: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::FuelAwards);
    #[cfg(feature = "callback_profile")]
    let _tag = TagTimer::new(tag);
    let call = LOG.enter();
    if !LOG.enabled.load(Ordering::Acquire) {
        return;
    }
    let memory = unsafe { LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    if tag == 9 {
        let status = registers.rsi;
        THREAD_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if let Some(pending) = state.pending.as_mut()
                && pending.started.status == status
            {
                pending.refused = true;
                pending.cause.get_or_insert(1);
            }
        });
        if status == 0
            || !append(
                &call,
                Entry {
                    reset: true,
                    started: native::Started {
                        status,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                None,
                &memory,
            )
        {
            LOG.invalidate();
            REFUSAL.record(if status == 0 {
                "fuel reset status is null"
            } else {
                STORAGE_FAILED
            });
            COUNTERS[9].fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    if tag == 0 {
        match native::start_held(&memory, base, registers.rsi, None) {
            Ok((started, located, _)) => {
                let next = Pending {
                    root: ROOT.fetch_add(1, Ordering::Relaxed),
                    epoch: LOG.epoch.load(Ordering::Acquire),
                    started,
                    located,
                    stamp: stamp(),
                    received: [0; 3],
                    refused: false,
                    cause: None,
                };
                THREAD_STATE.with(|state| {
                    let state = &mut *state.borrow_mut();
                    state.registry = native::Registry::default();
                    let old = state.pending.replace(next);
                    if let Some(prev) = old {
                        flush_batch_orphans(
                            &call,
                            &mut state.batch,
                            prev.root,
                            prev.started.country,
                        );
                        COUNTERS[0].fetch_add(1, Ordering::Relaxed);
                        if let Some(pending) = state.pending.as_mut() {
                            pending.refused = true;
                            pending.cause.get_or_insert(5);
                        }
                    } else {
                        state.batch.clear();
                    }
                });
            }
            Err(reason) => {
                REFUSAL.record(reason);
                THREAD_STATE.with(|state| {
                    let state = &mut *state.borrow_mut();
                    if let Some(prev) = state.pending.take() {
                        flush_batch_orphans(
                            &call,
                            &mut state.batch,
                            prev.root,
                            prev.started.country,
                        );
                    } else {
                        state.batch.clear();
                    }
                });
                COUNTERS[0].fetch_add(1, Ordering::Relaxed);
            }
        }
        return;
    }
    THREAD_STATE.with(|state| {
        let state = &mut *state.borrow_mut();
        let Some(pending) = state.pending.as_mut() else {
            COUNTERS[1].fetch_add(1, Ordering::Relaxed);
            return;
        };
        let epoch_current = pending.epoch == LOG.epoch.load(Ordering::Acquire);
        if !epoch_current {
            pending.refused = true;
            pending.cause.get_or_insert(0);
        }
        let located = held_for(pending, epoch_current);
        if tag == 1 {
            let mut pending = state.pending.take().unwrap();
            let settled = native::settle_held(
                &memory,
                base,
                pending.started,
                registers.rbx,
                located.as_ref(),
            )
            .map(|(settled, taken)| {
                if taken {
                    HELD.increment(0);
                }
                settled
            });
            if let Err(reason) = &settled {
                pending.cause.get_or_insert(6);
                REFUSAL.record(reason);
            }
            if !settled
                .as_ref()
                .is_ok_and(|s| s.received == pending.received)
            {
                pending.refused = true;
                pending.cause.get_or_insert(4);
            }
            if pending.refused {
                REFUSAL.record(SETTLED_NAMES[pending.cause.unwrap_or(4)]);
            }
            append_batch(
                &call,
                &mut state.batch,
                pending,
                settled.unwrap_or_default(),
                &memory,
            );
            return;
        }
        let (kind, pointer, host_ptr, amount) = match tag {
            2..=6 => (0, registers.rax, 0, registers.rdx as i64),
            7 => (2, registers.rcx, 0, registers.rdx as i64),
            8 => (1, registers.rdi, registers.rdx, registers.rbx as i64),
            _ => {
                pending.refused = true;
                pending.cause.get_or_insert(7);
                return;
            }
        };
        pending.received[kind as usize] = pending.received[kind as usize].wrapping_add(amount);
        let coalesced = state.batch.try_coalesce(kind, pointer, host_ptr, amount);
        if coalesced == Some(true) {
            if located.is_some() {
                HELD.increment(if kind == 1 { 1 } else { 2 });
            }
            return;
        }
        let insert_in_table = coalesced.is_none();
        let recipient_and_name = if kind == 1 {
            if let Some(located) = located.as_ref() {
                native::air_held_with(
                    &memory,
                    base,
                    pointer,
                    registers.rdx,
                    pending.started.country,
                    Some(&located.country),
                    Some(&mut state.registry),
                )
            } else {
                native::air_held(
                    &memory,
                    base,
                    pointer,
                    registers.rdx,
                    pending.started.country,
                    None,
                )
            }
            .map(|(recipient, taken)| (recipient, None, taken))
        } else if located.is_some() {
            native::unit_with_name(&memory, base, pointer, kind, Some(&mut state.registry))
                .map(|(recipient, name, taken)| (recipient, Some(name), taken))
        } else {
            native::unit_with_name(&memory, base, pointer, kind, None)
                .map(|(recipient, name, taken)| (recipient, Some(name), taken))
        };
        match recipient_and_name {
            Ok((recipient, name, taken)) => {
                if taken {
                    HELD.increment(if kind == 1 { 1 } else { 2 });
                }
                let (name, name_reason) = match name {
                    Some(Ok(n)) => (Some(n), None),
                    Some(Err(reason)) => (None, Some(reason)),
                    None => (None, None),
                };
                let buffered = state.batch.push(
                    kind,
                    pointer,
                    host_ptr,
                    recipient,
                    amount,
                    name.as_ref(),
                    name_reason,
                    insert_in_table,
                    &memory,
                );
                if !buffered
                    && !append(
                        &call,
                        Entry {
                            root: pending.root,
                            country: pending.started.country,
                            recipient: Some(recipient),
                            amount,
                            name_reason,
                            ..Default::default()
                        },
                        name,
                        &memory,
                    )
                {
                    pending.refused = true;
                    pending.cause.get_or_insert(3);
                }
            }
            Err(reason) => {
                REFUSAL.record(reason);
                pending.refused = true;
                pending.cause.get_or_insert(2);
                COUNTERS[3].fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

pub fn drain(
    entries: &mut Vec<Entry>,
    names: &mut Vec<u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    drain_buffers(
        engine_native_memory::walk::Buffer::Growing(entries),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}
pub(crate) fn drain_compact(
    entries: &mut Vec<CompactEntry>,
    roots: &mut Vec<RootData>,
    names: &mut Vec<u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let result = drain_domains_compact(
        engine_native_memory::walk::Buffer::Growing(entries),
        roots,
        engine_native_memory::walk::Buffer::Growing(names),
    );
    if let Err(reason) = result {
        let _ = FIRST_DRAIN_REFUSAL.set(reason);
        DRAIN_REFUSALS.fetch_add(1, Ordering::Relaxed);
    }
    result
}
pub fn lost_drain() {
    LOST_DRAINS.fetch_add(1, Ordering::Relaxed);
    LOG.invalidate();
}
fn drain_buffers(
    entries: engine_native_memory::walk::Buffer<'_, Entry>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let result = drain_domains(entries, names);
    if let Err(reason) = result {
        let _ = FIRST_DRAIN_REFUSAL.set(reason);
        DRAIN_REFUSALS.fetch_add(1, Ordering::Relaxed);
    }
    result
}
fn drain_domains_compact(
    entries: engine_native_memory::walk::Buffer<'_, CompactEntry>,
    roots: &mut Vec<RootData>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let Ok(domains) = CURRENT.try_lock() else {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
        return Err("fuel observation storage busy");
    };
    let Some(buffer) = domains.current.as_deref() else {
        return Err("fuel observation storage absent");
    };
    let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
    let names_required = attempted_since(name_cursor, &NAME_CONSUMED);
    let result = handover_compact(
        &LOG,
        &SPILL,
        buffer,
        domains.retired.as_deref(),
        entries,
        roots,
        names,
        UNHOOKED.get().copied().or_else(|| {
            (!LOG.enabled.load(Ordering::SeqCst)).then_some("fuel award hooks disabled")
        }),
    );
    ENTRY_DEMAND.fetch_max(
        buffer.entries_required.load(Ordering::Relaxed),
        Ordering::Release,
    );
    NAME_DEMAND.fetch_max(
        buffer.names_demand.load(Ordering::Relaxed),
        Ordering::Release,
    );
    result.map(|(drained, len, required)| {
        let entry_cursor = ENTRY_ATTEMPTS.load(Ordering::SeqCst);
        let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
        if LOG.active() == 0
            && LOG.cursor.load(Ordering::SeqCst) == drained.cursor
            && LOG.epoch.load(Ordering::SeqCst) == drained.epoch
            && drained.first + drained.copied as u64 == drained.cursor
        {
            ENTRY_CONSUMED.store(entry_cursor, Ordering::Relaxed);
            NAME_CONSUMED.store(name_cursor, Ordering::Relaxed);
        } else {
            ENTRY_CONSUMED.fetch_add(drained.copied as u64, Ordering::Relaxed);
            NAME_CONSUMED.fetch_add(len as u64, Ordering::Relaxed);
        }
        (drained, len, required.max(names_required))
    })
}
fn drain_domains(
    entries: engine_native_memory::walk::Buffer<'_, Entry>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let Ok(domains) = CURRENT.try_lock() else {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
        return Err("fuel observation storage busy");
    };
    let Some(buffer) = domains.current.as_deref() else {
        return Err("fuel observation storage absent");
    };
    let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
    let names_required = attempted_since(name_cursor, &NAME_CONSUMED);
    let result = handover_buffers(
        &LOG,
        &SPILL,
        buffer,
        domains.retired.as_deref(),
        entries,
        names,
        UNHOOKED.get().copied().or_else(|| {
            (!LOG.enabled.load(Ordering::SeqCst)).then_some("fuel award hooks disabled")
        }),
    );
    ENTRY_DEMAND.fetch_max(
        buffer.entries_required.load(Ordering::Relaxed),
        Ordering::Release,
    );
    NAME_DEMAND.fetch_max(
        buffer.names_demand.load(Ordering::Relaxed),
        Ordering::Release,
    );
    result.map(|(drained, len, required)| {
        let entry_cursor = ENTRY_ATTEMPTS.load(Ordering::SeqCst);
        let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
        if LOG.active() == 0
            && LOG.cursor.load(Ordering::SeqCst) == drained.cursor
            && LOG.epoch.load(Ordering::SeqCst) == drained.epoch
            && drained.first + drained.copied as u64 == drained.cursor
        {
            ENTRY_CONSUMED.store(entry_cursor, Ordering::Relaxed);
            NAME_CONSUMED.store(name_cursor, Ordering::Relaxed);
        } else {
            ENTRY_CONSUMED.fetch_add(drained.copied as u64, Ordering::Relaxed);
            NAME_CONSUMED.fetch_add(len as u64, Ordering::Relaxed);
        }
        (drained, len, required.max(names_required))
    })
}
fn handover_buffers(
    log: &Log,
    spill: &Spill,
    buffer: &Buffer,
    older: Option<&Buffer>,
    mut entries: engine_native_memory::walk::Buffer<'_, Entry>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    unavailable: Option<&'static str>,
) -> Result<(Drained, usize, usize), &'static str> {
    COUNTERS[4].fetch_add(1, Ordering::Relaxed);
    let first = log.consumed.load(Ordering::Acquire);
    let cursor = log.cursor.load(Ordering::SeqCst);
    let epoch = log.epoch.load(Ordering::SeqCst);
    if let Some(reason) = unavailable {
        log.consumed.store(cursor, Ordering::Release);
        if log.active() == 0 {
            buffer.release_roots_up_to(cursor);
            if let Some(old) = older {
                old.release_roots_up_to(cursor);
            }
        }
        spill.held().discard_before(cursor);
        return Err(reason);
    }
    let pending = usize::try_from(cursor.wrapping_sub(first))
        .map_err(|_| "fuel observation interval size exceeds address space")?;
    let mut drained = Drained {
        epoch,
        first,
        cursor,
        copied: 0,
        pending,
    };
    if log.active() != 0 {
        return Ok((drained, 0, 0));
    }
    let mut held = spill.held();
    if spill.lost.load(Ordering::Acquire) < cursor {
        let _ = spill
            .lost
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |lost| {
                (lost < cursor).then_some(u64::MAX)
            });
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
        buffer.release_roots_up_to(cursor);
        if let Some(old) = older {
            old.release_roots_up_to(cursor);
        }
        log.consumed.store(cursor, Ordering::Release);
        held.discard_before(cursor);
        drained.first = cursor;
        return Ok((drained, 0, 0));
    }
    let boundary = older.map_or(first, |old| old.cursor.load(Ordering::Acquire));
    let source = |index: u64| older.filter(|_| index < boundary).unwrap_or(buffer);
    let slot = |index: u64| {
        let domain = source(index);
        let slot = (index as usize)
            .checked_rem(domain.entries.len())
            .map(|slot| &domain.entries[slot])
            .filter(|slot| slot.published.load(Ordering::Acquire) == index + 1)
            .ok_or("fuel receipt publication incomplete")?;
        Ok::<_, &'static str>((domain, slot))
    };
    entries.ensure(pending)?;
    let copied = pending.min(entries.len());
    held.entries.sort_unstable_by_key(|&(index, ..)| index);
    let stale = held.entries.partition_point(|&(index, ..)| index < first);
    let spilled = &held.entries[stale..];
    let spilled_at = |at: &mut usize, index: u64| match spilled.get(*at) {
        Some(&(held, entry, required)) if held == index => {
            *at += 1;
            Some((entry, required))
        }
        _ => None,
    };
    let mut name_bytes = 0usize;
    let mut required = 0usize;
    let mut at = 0;
    for index in first..cursor {
        let (entry_name, name_required) = match spilled_at(&mut at, index) {
            Some((entry, name_required)) => (entry.name, name_required),
            None => {
                let (domain, slot) = slot(index)?;
                // SAFETY: `slot.published == index + 1` establishes visibility of the written slot and root.
                let (compact, name_required) =
                    unsafe { (*slot.entry.get(), *slot.name_required.get() as usize) };
                let root_data = match compact.payload {
                    CompactPayload::Root { index, .. } => {
                        Some(unsafe { *domain.roots[index as usize].data.get() })
                    }
                    _ => None,
                };
                (compact.name_span(root_data.as_ref()), name_required)
            }
        };
        required = required.saturating_add(name_required);
        if index - first < copied as u64 {
            name_bytes = name_bytes
                .checked_add(entry_name.map_or(0, |(_, len)| len))
                .ok_or("fuel name handover size overflow")?;
        }
    }
    buffer.names_demand.fetch_max(required, Ordering::Relaxed);
    if !names.ensure(if names.is_growing() {
        required
    } else {
        name_bytes
    })? {
        return Err("fuel name handover does not fit");
    }
    let mut len = 0usize;
    let mut at = 0;
    let mut has_roots = false;
    for (offset, target) in entries[..copied].iter_mut().enumerate() {
        let index = first + offset as u64;
        let entry = match spilled_at(&mut at, index) {
            Some((mut entry, _)) => {
                if let Some((start, name_len)) = entry.name {
                    names
                        .get_mut(len..len.saturating_add(name_len))
                        .ok_or("fuel name handover does not fit")?
                        .copy_from_slice(&held.names[start..start + name_len]);
                    entry.name = Some((len, name_len));
                    len += name_len;
                }
                entry
            }
            None => {
                let (domain, slot) = slot(index)?;
                // SAFETY: `slot.published == index + 1` establishes visibility of the written slot and root.
                let compact = unsafe { *slot.entry.get() };
                let root_data = match compact.payload {
                    CompactPayload::Root { index, .. } => {
                        has_roots = true;
                        Some(unsafe { *domain.roots[index as usize].data.get() })
                    }
                    _ => None,
                };
                let mut entry = compact.into_entry(root_data);
                if let Some((start, name_len)) = entry.name {
                    let bytes = names
                        .get_mut(len..len.saturating_add(name_len))
                        .ok_or("fuel name handover does not fit")?;
                    for (out, byte) in bytes.iter_mut().zip(&domain.names[start..start + name_len])
                    {
                        *out = unsafe { *byte.get() };
                    }
                    entry.name = Some((len, name_len));
                    len += name_len;
                }
                entry
            }
        };
        *target = entry;
    }
    if log.epoch.load(Ordering::SeqCst) != epoch {
        return Err("fuel award observation restarted");
    }
    if has_roots {
        let mut release_at = 0;
        for offset in 0..copied {
            let index = first + offset as u64;
            if spilled_at(&mut release_at, index).is_some() {
                continue;
            }
            if let Ok((domain, slot)) = slot(index) {
                // SAFETY: `log.consumed` still holds `first`, so `slot` stays immutable until released.
                let compact = unsafe { *slot.entry.get() };
                if let CompactPayload::Root {
                    index: root_pos, ..
                } = compact.payload
                {
                    domain.roots[root_pos as usize]
                        .owner
                        .store(0, Ordering::Release);
                }
            }
        }
    }
    log.consumed.store(first + copied as u64, Ordering::Release);
    held.discard_before(first + copied as u64);
    drained.copied = copied;
    COUNTERS[7].fetch_add(copied as u64, Ordering::Relaxed);
    Ok((drained, len, required))
}

#[expect(clippy::too_many_arguments, reason = "mirrors handover_buffers with separate root arena")]
fn handover_compact(
    log: &Log,
    spill: &Spill,
    buffer: &Buffer,
    older: Option<&Buffer>,
    mut entries: engine_native_memory::walk::Buffer<'_, CompactEntry>,
    roots: &mut Vec<RootData>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    unavailable: Option<&'static str>,
) -> Result<(Drained, usize, usize), &'static str> {
    roots.clear();
    COUNTERS[4].fetch_add(1, Ordering::Relaxed);
    let first = log.consumed.load(Ordering::Acquire);
    let cursor = log.cursor.load(Ordering::SeqCst);
    let epoch = log.epoch.load(Ordering::SeqCst);
    if let Some(reason) = unavailable {
        log.consumed.store(cursor, Ordering::Release);
        if log.active() == 0 {
            buffer.release_roots_up_to(cursor);
            if let Some(old) = older {
                old.release_roots_up_to(cursor);
            }
        }
        spill.held().discard_before(cursor);
        return Err(reason);
    }
    let pending = usize::try_from(cursor.wrapping_sub(first))
        .map_err(|_| "fuel observation interval size exceeds address space")?;
    let mut drained = Drained {
        epoch,
        first,
        cursor,
        copied: 0,
        pending,
    };
    if log.active() != 0 {
        return Ok((drained, 0, 0));
    }
    let mut held = spill.held();
    if spill.lost.load(Ordering::Acquire) < cursor {
        let _ = spill
            .lost
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |lost| {
                (lost < cursor).then_some(u64::MAX)
            });
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
        buffer.release_roots_up_to(cursor);
        if let Some(old) = older {
            old.release_roots_up_to(cursor);
        }
        log.consumed.store(cursor, Ordering::Release);
        held.discard_before(cursor);
        drained.first = cursor;
        return Ok((drained, 0, 0));
    }
    let boundary = older.map_or(first, |old| old.cursor.load(Ordering::Acquire));
    let source = |index: u64| older.filter(|_| index < boundary).unwrap_or(buffer);
    let slot = |index: u64| {
        let domain = source(index);
        let slot = (index as usize)
            .checked_rem(domain.entries.len())
            .map(|slot| &domain.entries[slot])
            .filter(|slot| slot.published.load(Ordering::Acquire) == index + 1)
            .ok_or("fuel receipt publication incomplete")?;
        Ok::<_, &'static str>((domain, slot))
    };
    entries.ensure(pending)?;
    let copied = pending.min(entries.len());
    held.entries.sort_unstable_by_key(|&(index, ..)| index);
    let stale = held.entries.partition_point(|&(index, ..)| index < first);
    let spilled = &held.entries[stale..];
    let spilled_at = |at: &mut usize, index: u64| match spilled.get(*at) {
        Some(&(held, entry, required)) if held == index => {
            *at += 1;
            Some((entry, required))
        }
        _ => None,
    };
    let mut name_bytes = 0usize;
    let mut required = 0usize;
    let mut root_count = 0usize;
    let mut at = 0;
    for index in first..cursor {
        let (entry_name, name_required, is_root) = match spilled_at(&mut at, index) {
            Some((entry, name_required)) => {
                (entry.name, name_required, !entry.reset && entry.recipient.is_none())
            }
            None => {
                let (domain, slot) = slot(index)?;
                // SAFETY: `slot.published == index + 1` establishes visibility of the written slot and root.
                let (compact, name_required) =
                    unsafe { (*slot.entry.get(), *slot.name_required.get() as usize) };
                let (root_data, is_root) = match compact.payload {
                    CompactPayload::Root { index, .. } => {
                        (Some(unsafe { *domain.roots[index as usize].data.get() }), true)
                    }
                    _ => (None, false),
                };
                (compact.name_span(root_data.as_ref()), name_required, is_root)
            }
        };
        required = required.saturating_add(name_required);
        if index - first < copied as u64 {
            root_count += usize::from(is_root);
            name_bytes = name_bytes
                .checked_add(entry_name.map_or(0, |(_, len)| len))
                .ok_or("fuel name handover size overflow")?;
        }
    }
    buffer.names_demand.fetch_max(required, Ordering::Relaxed);
    if !names.ensure(if names.is_growing() {
        required
    } else {
        name_bytes
    })? {
        return Err("fuel name handover does not fit");
    }
    engine_allocation::reserve(roots, root_count)
        .map_err(|_| "fuel root handover allocation failed")?;
    let mut len = 0usize;
    let mut at = 0;
    let mut has_roots = false;
    for (offset, target) in entries[..copied].iter_mut().enumerate() {
        let index = first + offset as u64;
        let compact = match spilled_at(&mut at, index) {
            Some((mut entry, _)) => {
                if let Some((start, name_len)) = entry.name {
                    names
                        .get_mut(len..len.saturating_add(name_len))
                        .ok_or("fuel name handover does not fit")?
                        .copy_from_slice(&held.names[start..start + name_len]);
                    entry.name = Some((len, name_len));
                    len += name_len;
                }
                CompactEntry::from_entry(entry, roots)
                    .map_err(|_| "fuel root handover allocation failed")?
            }
            None => {
                let (domain, slot) = slot(index)?;
                // SAFETY: `slot.published == index + 1` establishes visibility of the written slot and root.
                let mut compact = unsafe { *slot.entry.get() };
                match &mut compact.payload {
                    CompactPayload::Recipient {
                        name_start,
                        name_len,
                        ..
                    } => {
                        if *name_start != NO_NAME {
                            let start = *name_start as usize;
                            let nlen = *name_len as usize;
                            let bytes = names
                                .get_mut(len..len.saturating_add(nlen))
                                .ok_or("fuel name handover does not fit")?;
                            for (out, byte) in
                                bytes.iter_mut().zip(&domain.names[start..start + nlen])
                            {
                                *out = unsafe { *byte.get() };
                            }
                            *name_start = u32::try_from(len)
                                .map_err(|_| "fuel name handover size overflow")?;
                            len += nlen;
                        }
                    }
                    CompactPayload::Root { index, .. } => {
                        has_roots = true;
                        let mut data = unsafe { *domain.roots[*index as usize].data.get() };
                        if let Some((start, nlen)) = data.name {
                            let bytes = names
                                .get_mut(len..len.saturating_add(nlen))
                                .ok_or("fuel name handover does not fit")?;
                            for (out, byte) in
                                bytes.iter_mut().zip(&domain.names[start..start + nlen])
                            {
                                *out = unsafe { *byte.get() };
                            }
                            data.name = Some((len, nlen));
                            len += nlen;
                        }
                        let out_index = u32::try_from(roots.len())
                            .map_err(|_| "fuel root handover allocation failed")?;
                        roots.push(data);
                        *index = out_index;
                    }
                    CompactPayload::Reset { .. } => {}
                }
                compact
            }
        };
        *target = compact;
    }
    if log.epoch.load(Ordering::SeqCst) != epoch {
        return Err("fuel award observation restarted");
    }
    if has_roots {
        let mut release_at = 0;
        for offset in 0..copied {
            let index = first + offset as u64;
            if spilled_at(&mut release_at, index).is_some() {
                continue;
            }
            if let Ok((domain, slot)) = slot(index) {
                // SAFETY: `log.consumed` still holds `first`, so `slot` stays immutable until released.
                let compact = unsafe { *slot.entry.get() };
                if let CompactPayload::Root {
                    index: root_pos, ..
                } = compact.payload
                {
                    domain.roots[root_pos as usize]
                        .owner
                        .store(0, Ordering::Release);
                }
            }
        }
    }
    log.consumed.store(first + copied as u64, Ordering::Release);
    held.discard_before(first + copied as u64);
    drained.copied = copied;
    COUNTERS[7].fetch_add(copied as u64, Ordering::Relaxed);
    Ok((drained, len, required))
}

#[cfg(test)]
#[path = "fuel_awards/tests.rs"]
mod tests;
pub fn capacities() -> (usize, usize) {
    (
        CURRENT_ENTRIES
            .load(Ordering::Acquire)
            .max(OFFERED_ENTRIES.load(Ordering::Acquire)),
        CURRENT_NAMES
            .load(Ordering::Acquire)
            .max(OFFERED_NAMES.load(Ordering::Acquire)),
    )
}

pub fn drained_capacities() -> (usize, usize) {
    (
        CURRENT_ENTRIES
            .load(Ordering::Acquire)
            .max(OFFERED_ENTRIES.load(Ordering::Acquire)),
        PACKED_NAMES
            .load(Ordering::Acquire)
            .max(NAME_DEMAND.load(Ordering::Acquire)),
    )
}

pub struct Hooks {
    taps: Vec<Tap>,
    publication_attempted: bool,
}
impl Hooks {
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        let sites: [Rva; SITES] = [
            native::BEGIN,
            native::END,
            native::LAND[0],
            native::LAND[1],
            native::LAND[2],
            native::LAND[3],
            native::LAND[4],
            native::NAVAL,
            native::AIR,
            native::RESET,
        ];
        let mut taps = Vec::new();
        let result = (|| -> Result<(), &'static str> {
            let mut located = [(0, [0; 16]); SITES];
            for (slot, &key) in located.iter_mut().zip(&sites) {
                *slot = engine_sites::hook(base, key)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                    .inspect_err(|cause| {
                        crate::hook::installation::refused(
                            crate::hook::installation::Kind::Sentinel,
                            None,
                            cause,
                        )
                    })?;
            }
            for (tag, (address, original)) in located.into_iter().enumerate() {
                taps.push(
                    unsafe { Tap::new(address, observe, tag, original) }
                        .map_err(|_| "fuel award tap construction failed")?,
                );
            }
            Ok(())
        })();
        if let Err(reason) = result {
            let _ = UNHOOKED.set(reason);
            taps.clear();
        }
        BASE.store(base, Ordering::Relaxed);
        Self {
            taps,
            publication_attempted: false,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if enabled {
            self.publication_attempted = true;
        }
        if !enabled && !stop_admission_and_drained() {
            return Ok(false);
        }
        let ready = match crate::hook::shared::attach_outcome() {
            Some(outcome) => outcome.map_err(|_| "fuel award tap publication failed")?,
            None => Hook::set_enabled_iter(
                self.taps.iter_mut().map(Tap::publication_hook),
                enabled,
            )
            .map_err(|_| "fuel award tap publication failed")?,
        };
        if ready {
            LOG.invalidate();
            LOG.enabled
                .store(enabled && !self.taps.is_empty(), Ordering::SeqCst);
        }
        Ok(ready)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps.iter().flat_map(Tap::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        if !self.publication_attempted {
            let _ = release();
        }
    }
}
