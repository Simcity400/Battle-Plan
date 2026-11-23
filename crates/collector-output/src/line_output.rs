#[path = "line_output/rocket_observer.rs"]
mod rocket_observer;
pub use crate::output_log::Drained;
use crate::{
    opinion_changes::Padded,
    output_hooks,
    output_log::{DrainCounters, Log, Reserved, Spill},
};
use engine_native_memory::Memory;
use replay_data::rows::OutputPurpose;
use engine_production::{
    line_output_sites::CREDIT, line_output_sites::DELIVERY,
    line_output_sites::LINE_HANDLE, line_output_sites::LINE_ORGANISATION,
    line_output_sites::LINE_VARIANT, line_output_sites::UNIT_SCALE,
    line_output_sites::VARIANT_HANDLE,
};
use std::{
    cell::Cell,
    mem::ManuallyDrop,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};

fn capacity_for_lines(events: usize) -> usize {
    events
        .saturating_mul(2)
        .max(OBSERVATION.required.load(Ordering::Relaxed))
}

#[derive(Default)]
struct Slot {
    line: AtomicU64,
    country: AtomicU32,
    variant: AtomicU64,
    organisation: AtomicU64,
    counts: AtomicU64,
    exact: AtomicU8,
    spent: AtomicU64,
    instant: AtomicU64,
    spending_known: AtomicU8,
    kind: AtomicU8,
    purpose: AtomicU8,
}

struct Storage {
    slots: Vec<Slot>,
}
impl Storage {
    fn new(capacity: usize) -> Option<Box<Self>> {
        let mut slots = engine_allocation::vec(capacity).ok()?;
        slots.resize_with(capacity, Slot::default);
        engine_allocation::boxed(Self { slots }).ok()
    }
}

#[derive(Default)]
struct LogSwap {
    current: Option<Box<Storage>>,
    next: Option<Box<Storage>>,
    retired: Option<Box<Storage>>,
}
static LOG_SWAP: Mutex<LogSwap> = Mutex::new(LogSwap {
    current: None,
    next: None,
    retired: None,
});
static LOG: AtomicPtr<Storage> = AtomicPtr::new(ptr::null_mut());
static CAPACITY: &AtomicUsize = &OBSERVATION.capacity;
static SWAPS: AtomicU64 = AtomicU64::new(0);
static FAILED_OFFERS: AtomicU64 = AtomicU64::new(0);

pub fn presize(events: usize) {
    engine_allocation::advisory(|| presize_storage(events));
}

fn presize_storage(events: usize) {
    let Ok(mut swap) = LOG_SWAP.lock() else {
        return;
    };
    let capacity = capacity_for_lines(events);
    if (!LOG.load(Ordering::SeqCst).is_null() && capacity <= CAPACITY.load(Ordering::Acquire))
        || swap
            .next
            .as_ref()
            .is_some_and(|log| capacity <= log.slots.len())
    {
        return;
    }
    if let Some(next) = Storage::new(capacity) {
        swap.next = Some(next);
    } else {
        FAILED_OFFERS.fetch_add(1, Ordering::Relaxed);
    }
}

/// # Safety
/// Caller must hold exclusive pre-publication attach ownership (before enabling hooks), run on
/// the admitted post-drain observer boundary under the capture gate, or hold `test_lock()` in an
/// isolated test with hooks quiesced as the sole log-swap consumer.
pub unsafe fn install_offered() {
    let Ok(mut swap) = LOG_SWAP.try_lock() else {
        return;
    };
    if swap.retired.is_some()
        || native_retention_required()
        || active() != 0
        || CURSOR.load(Ordering::SeqCst) != CONSUMED.load(Ordering::Relaxed)
    {
        return;
    }
    let Some(mut next) = swap.next.take() else {
        return;
    };
    let capacity = next.slots.len();
    let pointer = &mut *next as *mut Storage;
    swap.retired = swap.current.replace(next);
    LOG.store(pointer, Ordering::SeqCst);
    CAPACITY.store(capacity, Ordering::Release);
    if swap.retired.is_some() {
        SWAPS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn collect() {
    if native_retention_required() {
        return;
    }
    if let Ok(mut swap) = LOG_SWAP.lock() {
        drop(swap.retired.take());
    }
}

fn log() -> Option<&'static Storage> {
    // SAFETY: a native writer counts itself in `ACTIVE` from the hook's entry until after its
    // append, and an unsettled call holds the log through `native_retention_required`; tests write
    // under `test_lock`. The observer boundary's job is the sole drain, and it installs a
    // replacement only with neither held; the capture gate keeps command-batch writers out through
    // that sequence, and detach drains every callback before releasing storage.
    unsafe { LOG.load(Ordering::SeqCst).as_ref() }
}

static OBSERVATION: Log = Log::new(0);
static SPILL: Spill<Entry> = Spill::new();
static CURSOR: &Padded<AtomicU64> = &OBSERVATION.cursor;
static ENABLED: &AtomicBool = &OBSERVATION.enabled;
pub(super) static ACTIVE: &Padded<AtomicUsize> = &OBSERVATION.active;
pub fn active() -> usize {
    OBSERVATION.active()
}
static ORIGINAL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static BASE: AtomicUsize = AtomicUsize::new(0);
static CREDIT_CALLERS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static DELIVERY_CALLERS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static ROCKET_DELIVERY: AtomicUsize = AtomicUsize::new(0);

fn bind_callers(base: usize) -> Result<(), &'static str> {
    let bind = |slot: &AtomicUsize, key| {
        let address = engine_sites::resolve(base, key)
            .map_err(|_| "production line caller withheld")?;
        slot.store(address, Ordering::Relaxed);
        Ok(())
    };
    for (slot, &key) in CREDIT_CALLERS.iter().zip(&CREDIT.callers) {
        bind(slot, key)?;
    }
    for (slot, &key) in DELIVERY_CALLERS.iter().zip(&DELIVERY.callers) {
        bind(slot, key)?;
    }
    bind(
        &ROCKET_DELIVERY,
        engine_production::line_output_sites::ROCKET_DELIVERY_RETURN,
    )
}
static CONSUMED: &AtomicU64 = &OBSERVATION.consumed;
static UNHOOKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static COUNTERS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
static SIZING_REFUSALS: AtomicU64 = AtomicU64::new(0);
static FIRST_SIZING_REFUSAL: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static DELIVERY_REFUSALS: AtomicU64 = AtomicU64::new(0);
static FIRST_DELIVERY_REFUSAL: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static SPENDING_REFUSALS: AtomicU64 = AtomicU64::new(0);
static FIRST_SPENDING_REFUSAL: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static PURPOSE_REFUSALS: AtomicU64 = AtomicU64::new(0);
static FIRST_PURPOSE_REFUSAL: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

pub fn refuse_spending(reason: &'static str) {
    SPENDING_REFUSALS.fetch_add(1, Ordering::Relaxed);
    let _ = FIRST_SPENDING_REFUSAL.set(reason);
}

fn refuse_purpose(reason: &'static str) {
    PURPOSE_REFUSALS.fetch_add(1, Ordering::Relaxed);
    let _ = FIRST_PURPOSE_REFUSAL.set(reason);
}

fn refuse_delivery(reason: &'static str) {
    COUNTERS[2].fetch_add(1, Ordering::Relaxed);
    DELIVERY_REFUSALS.fetch_add(1, Ordering::Relaxed);
    let _ = FIRST_DELIVERY_REFUSAL.set(reason);
}
pub fn refuse_sizing(reason: &'static str) {
    SIZING_REFUSALS.fetch_add(1, Ordering::Relaxed);
    let _ = FIRST_SIZING_REFUSAL.set(reason);
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub kind: OutputKind,
    pub line: u64,
    pub country: u32,
    pub variant: u64,
    pub mio: u64,
    pub units: i32,
    pub credit: i32,
    pub spending: Option<(i64, i64)>,
    pub exact: bool,
    pub purpose: OutputPurpose,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputKind {
    #[default]
    Equipment,
    Spending(engine_production::line_output_sites::RocketFailure),
    Delivery {
        units_known: bool,
    },
    UnknownDelivery,
}
impl OutputKind {
    fn raw(self) -> u8 {
        match self {
            Self::Equipment => 0,
            Self::Spending(failure) => 1 + failure as u8,
            Self::Delivery { units_known: true } => 7,
            Self::Delivery { units_known: false } => 8,
            Self::UnknownDelivery => 9,
        }
    }
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Equipment),
            1..=6 => Some(Self::Spending(
                engine_production::line_output_sites::RocketFailure::from_raw(raw - 1),
            )),
            7 => Some(Self::Delivery { units_known: true }),
            8 => Some(Self::Delivery { units_known: false }),
            9 => Some(Self::UnknownDelivery),
            _ => None,
        }
    }
}

impl Entry {
    pub fn rocket_event(&self) -> crate::rocket_output::Entry {
        use crate::rocket_output::Entry as Rocket;
        match self.kind {
            OutputKind::Equipment => Rocket::Equipment,
            OutputKind::Spending(mut failure) => {
                if failure == engine_production::line_output_sites::RocketFailure::None
                    && self.spending.is_none()
                {
                    failure = engine_production::line_output_sites::RocketFailure::Amount;
                }
                let (spent_raw, instant_raw) = self.spending.unwrap_or_default();
                Rocket::Spending(engine_production::line_output_sites::RocketTick {
                    country: self.country,
                    variant: self.variant,
                    spent_raw,
                    instant_raw,
                    failure,
                })
            }
            OutputKind::Delivery { units_known } => Rocket::Delivery {
                country: self.country,
                variant: self.variant,
                units: units_known.then_some(self.units),
                coherent: self.exact,
            },
            OutputKind::UnknownDelivery => Rocket::Delivery {
                country: 0,
                variant: 0,
                units: None,
                coherent: false,
            },
        }
    }
}
pub fn native_retention_required() -> bool {
    rocket_observer::retention_required()
}

pub fn rocket_unavailable_reason() -> Option<&'static str> {
    rocket_observer::unavailable_reason()
}

#[derive(Clone, Copy)]
struct Pending {
    generation: Option<u64>,
    producer: Option<usize>,
    variant: usize,
    amount: i64,
    doubled: bool,
}
const NOTHING_PENDING: Pending = Pending {
    generation: None,
    producer: None,
    variant: 0,
    amount: 0,
    doubled: false,
};

thread_local! {
    static PENDING: Cell<Pending> = const { Cell::new(NOTHING_PENDING) };
}

/// # Safety
/// Caller must own the active session's capture or publication failure path (or hold `test_lock()`
/// in an isolated test) when invalidating line-output observation.
pub unsafe fn invalidate() {
    OBSERVATION.invalidate();
}

#[cfg(test)]
struct Call;
#[cfg(test)]
impl Call {
    fn enter() -> crate::output_log::Call<'static> {
        OBSERVATION.enter()
    }
}

fn producer(callers: &[AtomicUsize; 4], caller: usize) -> Option<usize> {
    callers
        .iter()
        .position(|address| address.load(Ordering::Relaxed) == caller)
}

#[doc = " A live native value the game is using; the detour's receivers are the line"]
#[doc = " the game just produced for and its variant."]
fn live_u64(address: usize) -> u64 {
    unsafe { std::ptr::read_volatile(address as *const u64) }
}

fn observe_delivery(caller: usize, variant: usize, amount: i64) {
    let Some(index) = producer(&DELIVERY_CALLERS, caller) else {
        COUNTERS[2].fetch_add(1, Ordering::Relaxed);
        return;
    };
    COUNTERS[1].fetch_add(1, Ordering::Relaxed);
    let doubled = PENDING.get().producer.is_some();
    PENDING.set(Pending {
        generation: crate::world_lifetime::token(),
        producer: Some(index),
        variant,
        amount,
        doubled,
    });
}

fn credit_entry(line: usize, speed: i64, count: i32, caller: usize, pending: Pending) -> Option<Entry> {
    let variant = live_u64(line.wrapping_add(LINE_VARIANT)) as usize;
    let producer = producer(&CREDIT_CALLERS, caller);
    let (units, exact) = match pending.producer {
        None => (0, producer.is_some()),
        Some(index) => {
            let units = (pending.amount >= 0 && pending.amount % UNIT_SCALE == 0)
                .then(|| i32::try_from(pending.amount / UNIT_SCALE).ok())
                .flatten();
            match units {
                Some(units)
                    if producer == Some(index)
                        && pending.variant == variant
                        && units == count
                        && !pending.doubled =>
                {
                    (units, true)
                }
                _ => (0, false),
            }
        }
    };
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let country = engine_production::line_output_sites::producing_country(&memory, base, line).ok()?;
    let spending = producer
        .ok_or("production output producer unsupported")
        .and_then(|producer| {
            engine_production::line_output_sites::applied_spending(&memory, base, country, producer, speed)
        })
        .map_err(refuse_spending)
        .ok();
    let purpose = match producer {
        Some(0) => match engine_production::line_output_sites::converting(&memory, line) {
            Ok(true) => OutputPurpose::Converted,
            Ok(false) => OutputPurpose::New,
            Err(reason) => {
                refuse_purpose(reason);
                OutputPurpose::Unknown
            }
        },
        Some(1 | 2) => OutputPurpose::New,
        Some(3) => OutputPurpose::Refit,
        _ => {
            refuse_purpose("production output producer unsupported");
            OutputPurpose::Unknown
        }
    };
    Some(Entry {
        kind: OutputKind::Equipment,
        country,
        line: live_u64(line.wrapping_add(LINE_HANDLE)),
        variant: if variant == 0 {
            0
        } else {
            live_u64(variant.wrapping_add(VARIANT_HANDLE))
        },
        mio: live_u64(line.wrapping_add(LINE_ORGANISATION)),
        units,
        credit: count,
        spending,
        exact,
        purpose,
    })
}

fn observe_credit(frozen: Option<Entry>) {
    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
    match frozen {
        Some(entry) => append(entry),
        None => {
            COUNTERS[9].fetch_add(1, Ordering::Relaxed);
            OBSERVATION.invalidate();
        }
    }
}

fn append(entry: Entry) {
    COUNTERS[3].fetch_add(1, Ordering::Relaxed);
    if !entry.exact {
        COUNTERS[4].fetch_add(1, Ordering::Relaxed);
    }
    let (index, reserved) = OBSERVATION.reserve();
    let slot = match reserved {
        Reserved::Slot(slot) => log().and_then(|log| log.slots.get(slot)),
        Reserved::Spill => None,
    };
    let Some(slot) = slot else {
        OBSERVATION.spill(&SPILL, index, entry);
        return;
    };
    slot.line.store(entry.line, Ordering::SeqCst);
    slot.country.store(entry.country, Ordering::SeqCst);
    slot.variant.store(entry.variant, Ordering::SeqCst);
    slot.organisation.store(entry.mio, Ordering::SeqCst);
    slot.counts.store(
        u64::from(entry.units as u32) | u64::from(entry.credit as u32) << 32,
        Ordering::SeqCst,
    );
    slot.exact.store(u8::from(entry.exact), Ordering::SeqCst);
    let (spent, instant) = entry.spending.unwrap_or_default();
    slot.spent.store(spent as u64, Ordering::SeqCst);
    slot.instant.store(instant as u64, Ordering::SeqCst);
    slot.spending_known
        .store(u8::from(entry.spending.is_some()), Ordering::SeqCst);
    slot.kind.store(entry.kind.raw(), Ordering::SeqCst);
    slot.purpose.store(entry.purpose.raw(), Ordering::SeqCst);
    OBSERVATION.finish(index);
}

fn entry(slot: &Slot) -> Entry {
    let counts = slot.counts.load(Ordering::SeqCst);
    Entry {
        kind: OutputKind::from_raw(slot.kind.load(Ordering::SeqCst)).unwrap_or_else(|| {
            OBSERVATION.invalidate();
            OutputKind::Equipment
        }),
        line: slot.line.load(Ordering::SeqCst),
        country: slot.country.load(Ordering::SeqCst),
        variant: slot.variant.load(Ordering::SeqCst),
        mio: slot.organisation.load(Ordering::SeqCst),
        units: counts as u32 as i32,
        credit: (counts >> 32) as u32 as i32,
        spending: (slot.spending_known.load(Ordering::SeqCst) != 0).then(|| {
            (
                slot.spent.load(Ordering::SeqCst) as i64,
                slot.instant.load(Ordering::SeqCst) as i64,
            )
        }),
        exact: slot.exact.load(Ordering::SeqCst) != 0,
        purpose: OutputPurpose::from_raw(slot.purpose.load(Ordering::SeqCst)).unwrap_or_default(),
    }
}

pub(crate) fn drain(output: &mut Vec<Entry>) -> Result<Drained, &'static str> {
    drain_buffer(engine_native_memory::walk::Buffer::Growing(output))
}
fn drain_buffer(output: engine_native_memory::walk::Buffer<'_, Entry>) -> Result<Drained, &'static str> {
    let log = log();
    let unavailable = || {
        if native_retention_required() {
            Some("production native flight or acquired event remains unsettled")
        } else if UNHOOKED.get().is_some() {
            Some("production line hooks are not installed")
        } else if !ENABLED.load(Ordering::SeqCst) {
            Some("production line hooks are not enabled")
        } else {
            crate::world_lifetime::unavailable_reason()
        }
    };
    OBSERVATION.drain_buffer(
        output,
        &SPILL,
        |slot| Some(entry(log?.slots.get(slot)?)),
        unavailable,
        "production line observation restarted during the drain",
        DrainCounters {
            drains: &COUNTERS[5],
            copied: &COUNTERS[6],
            overflow: &COUNTERS[7],
            unobserved: &COUNTERS[8],
        },
    )
}

pub fn status() -> impl serde::Serialize {
    let counters = COUNTERS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let base = output_hooks::status::<0>(&OBSERVATION, &UNHOOKED, None);
    base.join(crate::status::status!({
        "capacity": CAPACITY.load(Ordering::Acquire),
        "swaps": SWAPS.load(Ordering::Relaxed),
        "failed_offers": FAILED_OFFERS.load(Ordering::Relaxed),
        "credits": counters[0],
        "producer_deliveries": counters[1],
        "other_deliveries": counters[2],
        "entries": counters[3],
        "inexact_entries": counters[4],
        "drains": counters[5],
        "handed_over": counters[6],
        "overflowed_logs": counters[7],
        "unobserved_drains": counters[8],
        "failed_producer_reads": counters[9],
        "spending_refusals": SPENDING_REFUSALS.load(Ordering::Relaxed),
        "first_spending_refusal": FIRST_SPENDING_REFUSAL.get().copied(),
        "purpose_refusals": PURPOSE_REFUSALS.load(Ordering::Relaxed),
        "first_purpose_refusal": FIRST_PURPOSE_REFUSAL.get().copied(),
        "native_retention_required": native_retention_required(),
        "rocket_unavailable": rocket_unavailable_reason(),
        "rocket_capture_refusals": rocket_observer::refusals(),
        "sizing_refusals": SIZING_REFUSALS.load(Ordering::Relaxed),
        "first_sizing_refusal": FIRST_SIZING_REFUSAL.get().copied(),
        "delivery_refusals": DELIVERY_REFUSALS.load(Ordering::Relaxed),
        "first_delivery_refusal": FIRST_DELIVERY_REFUSAL.get().copied(),
        "scope": "cumulative since attach; credits and entries include every ProduceEquipment, including zero-unit ticks; rocket coverage and refusals are reported independently; unknown delivery refusal preserves known spending; spending refusal does not change delivery/MIO exactness"
    }))
}

pub struct Hooks {
    shared: ManuallyDrop<output_hooks::Hooks<3>>,
}

impl Hooks {
    /// The runtime's build gate admitted this family. Installation failure leaves
    /// the output table unavailable; site order preserves paired observation.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize, events: usize) -> Self {
        presize(events);
        // SAFETY: Caller exclusively owns this hook family during attach before enabling hooks.
        unsafe { install_offered() };
        let domain = output_hooks::Domain {
            failure: output_hooks::Failure::Propagate,
            observation: &OBSERVATION,
            base: &BASE,
            originals: &ORIGINAL,
            installed: None,
            unhooked: &UNHOOKED,
        };
        let [credit, delivery, rocket] = rocket_observer::descriptors();
        Self {
            shared: ManuallyDrop::new(unsafe {
                output_hooks::Hooks::new_with_safety(
                    base,
                    [credit.0, delivery.0, rocket.0],
                    [credit.1, delivery.1, rocket.1],
                    domain,
                    if LOG.load(Ordering::SeqCst).is_null() {
                        Err("production line log allocation failed")
                    } else {
                        bind_callers(base)
                    },
                    [credit.2, delivery.2, rocket.2],
                )
            }),
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        self.shared
            .set_enabled_prepared(enabled, || OBSERVATION.invalidate())
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.shared
            .executable_ranges()
            .chain(rocket_observer::stub_ranges())
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        if native_retention_required() {
            return;
        }
        unsafe { ManuallyDrop::drop(&mut self.shared) };
        LOG.store(ptr::null_mut(), Ordering::SeqCst);
        CAPACITY.store(0, Ordering::Release);
        let mut swap = LOG_SWAP
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *swap = LogSwap::default();
        SPILL.release();
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) fn observe_for_test(enabled: bool) {
    crate::world_lifetime::observe_for_test(enabled);
    ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
    bind_callers(0).unwrap();
    if enabled {
        // SAFETY: In-crate tests hold `test_lock()` with `ENABLED` false before enabling.
        unsafe { reset_log_for_test(4096) };
    }
    CONSUMED.store(CURSOR.load(Ordering::SeqCst), Ordering::Relaxed);
    ENABLED.store(enabled, Ordering::SeqCst);
    PENDING.set(NOTHING_PENDING);
}

/// # Safety
/// Caller holds [`test_lock()`] (or exclusive fixture ownership) with line-output hooks disabled
/// and quiescent.
#[cfg(any(test, feature = "test-support"))]
pub unsafe fn reset_log_for_test(capacity: usize) {
    OBSERVATION.required.store(0, Ordering::Relaxed);
    SPILL.clear();
    let mut current = Storage::new(capacity).unwrap();
    LOG.store(&mut *current, Ordering::SeqCst);
    CAPACITY.store(capacity, Ordering::Release);
    *LOG_SWAP.lock().unwrap() = LogSwap {
        current: Some(current),
        ..Default::default()
    };
}

#[cfg(test)]
fn produce_for_test(line: usize, producer: usize, count: i32, delivered: Option<i64>) {
    if let Some(amount) = delivered {
        observe_delivery(
            DELIVERY.callers[producer].0,
            live_u64(line + LINE_VARIANT) as usize,
            amount,
        );
    }
    let pending = PENDING.replace(NOTHING_PENDING);
    observe_credit(credit_entry(line, 0, count, CREDIT.callers[producer].0, pending));
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const MILITARY: usize = 0;
    const NAVAL: usize = 1;

    #[test]
    fn common_projection_keeps_missing_spending_and_signed_delivery_independent() {
        use crate::rocket_output::Entry as Rocket;
        use engine_production::{line_output_sites::RocketFailure, line_output_sites::RocketTick};
        let missing = Entry {
            kind: OutputKind::Spending(RocketFailure::None),
            country: 5001,
            variant: 42,
            ..Entry::default()
        };
        assert_eq!(
            missing.rocket_event(),
            Rocket::Spending(RocketTick {
                country: 5001,
                variant: 42,
                spent_raw: 0,
                instant_raw: 0,
                failure: RocketFailure::Amount,
            })
        );
        let signed = Entry {
            kind: OutputKind::Delivery { units_known: true },
            country: 5001,
            variant: 42,
            units: -2,
            exact: false,
            ..Entry::default()
        };
        assert_eq!(
            signed.rocket_event(),
            Rocket::Delivery {
                country: 5001,
                variant: 42,
                units: Some(-2),
                coherent: false,
            }
        );
    }

    pub(super) struct Line {
        line: Vec<u64>,
        variant: Vec<u64>,
        _definition: Vec<u64>,
        _status: Vec<u64>,
        country: Vec<u64>,
    }
    impl Line {
        pub(super) fn new(handle: u64, variant: u64, mio: u64) -> Self {
            let definition = vec![0_u64; 0xc0];
            let mut owned_variant = vec![0_u64; 0x80];
            owned_variant[VARIANT_HANDLE / 8] = variant;
            owned_variant[engine_native::cequipment_variant::DEFINITION / 8] =
                definition.as_ptr() as u64;
            let mut line = vec![0_u64; 0x20];
            let mut status = vec![0_u64; (engine_native::cproduction_status::COUNTRY + 8) / 8];
            let mut country = vec![0_u64; (engine_native::ccountry::PRODUCTION_STATUS + 8) / 8];
            status[0] = engine_native::cproduction_status::VTABLE_RVA.0 as u64;
            country[0] = engine_native::ccountry::VTABLE_RVA.0 as u64;
            country[engine_native::ccountry::ID / 8] = 5001;
            country[engine_native::ccountry::PRODUCTION_STATUS / 8] = status.as_ptr() as u64;
            status[engine_native::cproduction_status::COUNTRY / 8] = country.as_ptr() as u64;
            line[0] = engine_native_parts::cmilitary_production_line::VTABLE_RVA.0 as u64;
            line[engine_native_parts::cproduction_line::PRODUCTION_STATUS / 8] = status.as_ptr() as u64;
            line[LINE_HANDLE / 8] = handle;
            line[LINE_VARIANT / 8] = owned_variant.as_ptr() as u64;
            line[LINE_ORGANISATION / 8] = mio;
            Self {
                line,
                variant: owned_variant,
                _definition: definition,
                _status: status,
                country,
            }
        }
        pub(super) fn address(&self) -> usize {
            self.line.as_ptr() as usize
        }
        pub(super) fn set_converting(&mut self, converting: bool) {
            let bit = 1_u64 << (8 * (engine_production::line_output_sites::LINE_CONVERSION % 8));
            let word = &mut self.line[engine_production::line_output_sites::LINE_CONVERSION / 8];
            *word = if converting { *word | bit } else { *word & !bit };
            self.variant[engine_native::cequipment_variant::PARENT / 8] = 0x1_0000_0046;
        }
        pub(super) fn replace(&mut self, variant: u64, mio: u64) {
            self.variant[VARIANT_HANDLE / 8] = variant;
            self.line[LINE_ORGANISATION / 8] = mio;
        }
        fn variant_pointer(&self) -> usize {
            self.line[LINE_VARIANT / 8] as usize
        }
    }

    fn drained(capacity: usize) -> (Result<Drained, &'static str>, Vec<Entry>) {
        let mut output = vec![Entry::default(); capacity];
        let result = drain(&mut output);
        let copied = result.map_or(0, |drained| drained.copied);
        output.truncate(copied);
        (result, output)
    }

    fn tick(line: u64) -> Entry {
        Entry {
            kind: OutputKind::Equipment,
            line,
            country: 5001,
            variant: line + 100,
            mio: line + 200,
            units: 1,
            credit: 1,
            spending: Some((line as i64, 0)),
            exact: true,
            purpose: OutputPurpose::New,
        }
    }

    #[test]
    fn a_live_population_larger_than_the_log_presizes_a_full_serial_pass() {
        let _lock = test_lock();
        observe_for_test(true);
        // SAFETY: Test holds `test_lock()` with line-output hooks quiescent.
        unsafe { reset_log_for_test(8) };
        let population = 70_000;
        let before = drained(0).0.unwrap();
        let swaps = serde_json::to_value(status()).unwrap()["swaps"]
            .as_u64()
            .unwrap();
        let overflows = serde_json::to_value(status()).unwrap()["overflowed_logs"]
            .as_u64()
            .unwrap();
        presize(population);
        let offered = LOG_SWAP.lock().unwrap().next.as_deref().unwrap() as *const Storage;
        presize(population);
        assert_eq!(
            LOG_SWAP.lock().unwrap().next.as_deref().unwrap() as *const Storage,
            offered
        );
        assert_eq!(serde_json::to_value(status()).unwrap()["capacity"], 8);
        unsafe { install_offered() };
        assert_eq!(
            serde_json::to_value(status()).unwrap()["capacity"],
            population * 2
        );
        assert_eq!(serde_json::to_value(status()).unwrap()["swaps"], swaps + 1);
        assert!(LOG_SWAP.lock().unwrap().retired.is_some());
        collect();
        assert!(LOG_SWAP.lock().unwrap().retired.is_none());
        let expected: Vec<_> = (1..=population as u64).map(tick).collect();
        for &entry in &expected {
            let _writer = Call::enter();
            append(entry);
        }
        let (result, entries) = drained(population);
        let after = result.unwrap();
        assert_eq!(entries, expected);
        assert!(after.complete());
        assert_eq!((after.epoch, after.first), (before.epoch, before.cursor));
        assert_eq!(
            serde_json::to_value(status()).unwrap()["overflowed_logs"],
            overflows
        );
        assert!(drained(1).1.is_empty());
        observe_for_test(false);
    }
    #[test]
    fn a_burst_into_an_empty_log_is_handed_over_whole_and_grows_the_next_log() {
        let _lock = test_lock();
        observe_for_test(true);
        // SAFETY: Test holds `test_lock()` with line-output hooks quiescent.
        unsafe { reset_log_for_test(0) };
        assert_eq!(capacity_for_lines(0), 0);
        let overflows = serde_json::to_value(status()).unwrap()["overflowed_logs"]
            .as_u64()
            .unwrap();
        let mut frame = Vec::new();
        for line in 1..=5000 {
            let _writer = Call::enter();
            append(tick(line));
        }
        let spilled = drain(&mut frame).unwrap();
        assert_eq!((spilled.pending, spilled.copied), (5000, 5000));
        assert_eq!(frame[..5000], (1..=5000).map(tick).collect::<Vec<_>>());
        assert_eq!(OBSERVATION.required.load(Ordering::Relaxed), 5000);
        assert!(SPILL.is_empty());
        presize(0);
        unsafe { install_offered() };
        collect();
        assert_eq!(CAPACITY.load(Ordering::Acquire), 5000);
        for line in 5001..=10_000 {
            let _writer = Call::enter();
            append(tick(line));
        }
        assert!(SPILL.is_empty());
        let drained = drain(&mut frame).unwrap();
        assert_eq!((drained.pending, drained.copied), (5000, 5000));
        assert_eq!(frame[..5000], (5001..=10_000).map(tick).collect::<Vec<_>>());
        assert_eq!(
            (drained.epoch, drained.first),
            (spilled.epoch, spilled.cursor)
        );
        assert_eq!(
            serde_json::to_value(status()).unwrap()["overflowed_logs"],
            overflows
        );
        observe_for_test(false);
    }

    #[test]
    fn swaps_wait_for_pending_entries_and_writers_without_losing_or_duplicating_ticks() {
        let _lock = test_lock();
        observe_for_test(true);
        // SAFETY: Test holds `test_lock()` with line-output hooks quiescent.
        unsafe { reset_log_for_test(8) };
        for line in 1..=3 {
            let _writer = Call::enter();
            append(tick(line));
        }
        presize(20);
        unsafe { install_offered() };
        assert_eq!(serde_json::to_value(status()).unwrap()["capacity"], 8);
        assert!(LOG_SWAP.lock().unwrap().next.is_some());
        let (first, mut entries) = drained(2);
        let first = first.unwrap();
        assert!(first.complete());
        let writer = Call::enter();
        unsafe { install_offered() };
        assert_eq!(serde_json::to_value(status()).unwrap()["capacity"], 8);
        assert!(LOG_SWAP.lock().unwrap().next.is_some());
        drop(writer);
        unsafe { install_offered() };
        assert_eq!(
            serde_json::to_value(status()).unwrap()["capacity"],
            capacity_for_lines(20)
        );
        assert!(LOG_SWAP.lock().unwrap().retired.is_some());
        presize(2000);
        unsafe { install_offered() };
        assert_eq!(
            serde_json::to_value(status()).unwrap()["capacity"],
            capacity_for_lines(20)
        );
        collect();
        unsafe { install_offered() };
        collect();
        for line in 4..=6 {
            let _writer = Call::enter();
            append(tick(line));
        }
        let (after, rest) = drained(8);
        let after = after.unwrap();
        entries.extend(rest);
        assert_eq!(entries, (1..=6).map(tick).collect::<Vec<_>>());
        assert_eq!(after.first, first.cursor);
        assert_eq!(after.epoch, first.epoch);
        assert!(after.complete());
        assert!(drained(8).1.is_empty());
        observe_for_test(false);
    }

    #[test]
    fn deliveries_pair_with_their_credit_and_blocked_ships_credit_alone() {
        let _lock = test_lock();
        observe_for_test(true);
        let a = Line::new(0x11_0000_0021, 0x5_0000_0046, 0x9_0000_004f);
        let b = Line::new(0x12_0000_0021, 0x6_0000_0046, 0);
        produce_for_test(a.address(), MILITARY, 3, Some(3 * UNIT_SCALE));
        produce_for_test(a.address(), MILITARY, 0, None);
        produce_for_test(b.address(), NAVAL, 2, None);
        observe_delivery(0x00ff_5a7a, b.variant_pointer(), 5 * UNIT_SCALE);
        produce_for_test(b.address(), NAVAL, 0, None);
        produce_for_test(a.address(), MILITARY, -1, None);
        let (result, entries) = drained(8);
        let drained = result.unwrap();
        assert!(drained.complete());
        assert_eq!(
            entries,
            [
                Entry {
                    kind: OutputKind::Equipment,
                    country: 5001,
                    line: 0x11_0000_0021,
                    variant: 0x5_0000_0046,
                    mio: 0x9_0000_004f,
                    units: 3,
                    credit: 3,
                    spending: None,
                    exact: true,
                    purpose: OutputPurpose::New,
                },
                Entry {
                    kind: OutputKind::Equipment,
                    country: 5001,
                    line: 0x11_0000_0021,
                    variant: 0x5_0000_0046,
                    mio: 0x9_0000_004f,
                    units: 0,
                    credit: 0,
                    spending: None,
                    exact: true,
                    purpose: OutputPurpose::New,
                },
                Entry {
                    kind: OutputKind::Equipment,
                    country: 5001,
                    line: 0x12_0000_0021,
                    variant: 0x6_0000_0046,
                    mio: 0,
                    units: 0,
                    credit: 2,
                    spending: None,
                    exact: true,
                    purpose: OutputPurpose::New,
                },
                Entry {
                    kind: OutputKind::Equipment,
                    country: 5001,
                    line: 0x12_0000_0021,
                    variant: 0x6_0000_0046,
                    mio: 0,
                    units: 0,
                    credit: 0,
                    spending: None,
                    exact: true,
                    purpose: OutputPurpose::New,
                },
                Entry {
                    kind: OutputKind::Equipment,
                    country: 5001,
                    line: 0x11_0000_0021,
                    variant: 0x5_0000_0046,
                    mio: 0x9_0000_004f,
                    units: 0,
                    credit: -1,
                    spending: None,
                    exact: true,
                    purpose: OutputPurpose::New,
                },
            ]
        );
        observe_for_test(false);
    }

    #[test]
    fn zero_conversion_is_exact_only_when_delivery_and_credit_match() {
        let _lock = test_lock();
        observe_for_test(true);
        let a = Line::new(0x11_0000_0021, 0x5_0000_0046, 0);
        let other = Line::new(0x12_0000_0021, 0x6_0000_0046, 0);
        produce_for_test(a.address(), MILITARY, 0, Some(0));
        produce_for_test(a.address(), MILITARY, 1, Some(UNIT_SCALE));
        let (_, entries) = drained(8);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.exact));
        assert_eq!((entries[0].units, entries[0].credit), (0, 0));
        assert_eq!((entries[1].units, entries[1].credit), (1, 1));
        assert!(entries.iter().all(|entry| entry.spending.is_none()));

        observe_delivery(DELIVERY.callers[NAVAL].0, a.variant_pointer(), 0);
        observe_credit(credit_entry(
            a.address(),
            0,
            0,
            CREDIT.callers[MILITARY].0,
            PENDING.replace(NOTHING_PENDING),
        ));
        observe_delivery(DELIVERY.callers[MILITARY].0, other.variant_pointer(), 0);
        observe_credit(credit_entry(
            a.address(),
            0,
            0,
            CREDIT.callers[MILITARY].0,
            PENDING.replace(NOTHING_PENDING),
        ));
        observe_delivery(DELIVERY.callers[MILITARY].0, a.variant_pointer(), 0);
        produce_for_test(a.address(), MILITARY, 0, Some(0));
        observe_delivery(DELIVERY.callers[MILITARY].0, a.variant_pointer(), 0);
        observe_credit(credit_entry(a.address(), 0, 0, 0x1234, PENDING.replace(NOTHING_PENDING)));
        produce_for_test(a.address(), MILITARY, 1, Some(0));
        produce_for_test(a.address(), MILITARY, 0, Some(UNIT_SCALE));
        produce_for_test(
            a.address(),
            MILITARY,
            1,
            Some((i64::from(i32::MAX) + 1) * UNIT_SCALE),
        );
        let (_, entries) = drained(8);
        assert_eq!(entries.len(), 7);
        assert!(entries.iter().all(|entry| !entry.exact && entry.units == 0));
        observe_for_test(false);
    }

    #[test]
    fn purposes_follow_the_producer_and_the_conversion_control() {
        let _lock = test_lock();
        observe_for_test(true);
        let mut a = Line::new(0x11_0000_0021, 0x5_0000_0046, 0);
        a.set_converting(true);
        produce_for_test(a.address(), MILITARY, 0, None);
        produce_for_test(a.address(), MILITARY, 1, Some(UNIT_SCALE));
        produce_for_test(a.address(), 1, 1, Some(UNIT_SCALE));
        produce_for_test(a.address(), 2, 0, None);
        produce_for_test(a.address(), 3, 1, Some(UNIT_SCALE));
        a.set_converting(false);
        produce_for_test(a.address(), MILITARY, 1, Some(UNIT_SCALE));
        let (_, entries) = drained(8);
        assert_eq!(
            entries.iter().map(|entry| entry.purpose).collect::<Vec<_>>(),
            [
                OutputPurpose::Converted,
                OutputPurpose::Converted,
                OutputPurpose::New,
                OutputPurpose::New,
                OutputPurpose::Refit,
                OutputPurpose::New,
            ]
        );
        assert!(entries.iter().all(|entry| entry.exact));
        observe_for_test(false);
    }

    #[test]
    fn unauthenticated_producer_ends_the_observation_instead_of_recording_zero() {
        let _lock = test_lock();
        observe_for_test(true);
        let mut a = Line::new(0x11_0000_0021, 0x5_0000_0046, 0);
        let before = drained(8).0.unwrap();
        a.country[engine_native::ccountry::ID / 8] = 0;
        produce_for_test(a.address(), MILITARY, 1, Some(UNIT_SCALE));
        let (after, entries) = drained(8);
        assert!(entries.is_empty());
        assert_ne!(after.unwrap().epoch, before.epoch);
        observe_for_test(false);
    }

    #[test]
    fn deliveries_that_do_not_pair_exactly_make_the_tick_inexact() {
        let _lock = test_lock();
        observe_for_test(true);
        let a = Line::new(0x21_0000_0021, 0x7_0000_0046, 0);
        let other = Line::new(0x22_0000_0021, 0x8_0000_0046, 0);
        let inexact = |credit, purpose| Entry {
            kind: OutputKind::Equipment,
            country: 5001,
            line: 0x21_0000_0021,
            variant: 0x7_0000_0046,
            mio: 0,
            units: 0,
            credit,
            spending: None,
            exact: false,
            purpose,
        };
        observe_delivery(DELIVERY.callers[NAVAL].0, a.variant_pointer(), UNIT_SCALE);
        let pending = PENDING.replace(NOTHING_PENDING);
        observe_credit(credit_entry(a.address(), 0, 1, CREDIT.callers[MILITARY].0, pending));
        observe_delivery(
            DELIVERY.callers[MILITARY].0,
            other.variant_pointer(),
            UNIT_SCALE,
        );
        let pending = PENDING.replace(NOTHING_PENDING);
        observe_credit(credit_entry(a.address(), 0, 1, CREDIT.callers[MILITARY].0, pending));
        observe_delivery(DELIVERY.callers[MILITARY].0, a.variant_pointer(), UNIT_SCALE);
        produce_for_test(a.address(), MILITARY, 1, Some(UNIT_SCALE));
        produce_for_test(a.address(), MILITARY, 1, Some(UNIT_SCALE / 2));
        produce_for_test(a.address(), NAVAL, -2, Some(-2 * UNIT_SCALE));
        observe_credit(credit_entry(a.address(), 0, 0, 0x1234, NOTHING_PENDING));
        let (_, entries) = drained(8);
        assert_eq!(
            entries,
            [
                inexact(1, OutputPurpose::New),
                inexact(1, OutputPurpose::New),
                inexact(1, OutputPurpose::New),
                inexact(1, OutputPurpose::New),
                inexact(-2, OutputPurpose::New),
                inexact(0, OutputPurpose::Unknown)
            ]
        );
        observe_for_test(false);
    }

    #[test]
    fn active_writers_defer_entries_and_short_storage_grows_without_losing_them() {
        let _lock = test_lock();
        observe_for_test(true);
        let a = Line::new(0x31_0000_0021, 0x9_0000_0046, 0);
        for count in 1..=5 {
            produce_for_test(
                a.address(),
                MILITARY,
                count,
                Some(i64::from(count) * UNIT_SCALE),
            );
        }
        let writer = Call::enter();
        let (result, _) = drained(8);
        let during = result.unwrap();
        assert_eq!(during.copied, 0);
        assert!(!during.complete());
        drop(writer);
        let (result, entries) = drained(2);
        let last = result.unwrap();
        assert_eq!(last.first, during.first);
        assert!(last.complete());
        assert_eq!((last.copied, last.pending), (5, 5));
        assert_eq!(
            entries.iter().map(|e| e.units).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        observe_for_test(false);
    }

    #[test]
    fn bursts_past_the_log_disabled_hooks_and_epochs_are_handed_over_as_they_are() {
        let _lock = test_lock();
        observe_for_test(true);
        // SAFETY: Test holds `test_lock()` with line-output hooks quiescent.
        unsafe { reset_log_for_test(8) };
        let a = Line::new(0x41_0000_0021, 0xa_0000_0046, 0);
        let (before, _) = drained(1);
        let before = before.unwrap();
        let overflows = serde_json::to_value(status()).unwrap()["overflowed_logs"]
            .as_u64()
            .unwrap();
        for count in 1..=20 {
            produce_for_test(a.address(), NAVAL, count, None);
        }
        presize(20);
        unsafe { install_offered() };
        assert_eq!(serde_json::to_value(status()).unwrap()["capacity"], 8);
        let (result, entries) = drained(4);
        let whole = result.unwrap();
        assert_eq!(whole.first, before.first + before.copied as u64);
        assert!(whole.complete());
        assert_eq!(
            entries.iter().map(|e| e.credit).collect::<Vec<_>>(),
            (1..=20).collect::<Vec<_>>()
        );
        assert_eq!(
            serde_json::to_value(status()).unwrap()["overflowed_logs"],
            overflows
        );
        unsafe { install_offered() };
        collect();
        assert_eq!(
            serde_json::to_value(status()).unwrap()["capacity"],
            capacity_for_lines(20)
        );
        let grown = drained(4).0.unwrap();
        assert_eq!((grown.epoch, grown.first), (whole.epoch, whole.cursor));
        unsafe { invalidate() };
        let (result, _) = drained(4);
        assert!(result.unwrap().epoch > whole.epoch);
        ENABLED.store(false, Ordering::SeqCst);
        produce_for_test(a.address(), NAVAL, 1, None);
        let (result, _) = drained(4);
        assert_eq!(result, Err("production line hooks are not enabled"));
        observe_for_test(false);
    }

    #[test]
    fn concurrent_producers_keep_every_entry() {
        let _lock = test_lock();
        observe_for_test(true);
        let lines: Vec<Line> = (0..4)
            .map(|index| Line::new(0x51_0000_0021 + (index << 32), 0xb_0000_0046, 0))
            .collect();
        let addresses: Vec<usize> = lines.iter().map(Line::address).collect();
        std::thread::scope(|scope| {
            for &address in &addresses {
                scope.spawn(move || {
                    for count in 1..=500 {
                        produce_for_test(
                            address,
                            MILITARY,
                            count,
                            Some(i64::from(count) * UNIT_SCALE),
                        );
                    }
                });
            }
        });
        let (result, entries) = drained(4096);
        assert!(result.unwrap().complete());
        assert_eq!(entries.len(), 2000);
        for line in &lines {
            let handle = line.line[LINE_HANDLE / 8];
            let units: i64 = entries
                .iter()
                .filter(|entry| entry.line == handle && entry.exact)
                .map(|entry| i64::from(entry.units))
                .sum();
            assert_eq!(units, 500 * 501 / 2);
        }
        observe_for_test(false);
    }
}
