use crate::{
    event_ledger::{Event, EventLedger},
    hook::{Hook, Registers, Tap},
    watch_registry::{Notification, WatchRegistry},
};
use engine_native_memory::{Memory, read};
use engine_production::market_mutations as native;
use engine_resources::clend_lease_exchange as lease;
use replay_data::rows::MarketEventRow;
use std::{
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

pub mod held;
pub const AMENDMENT: u8 = 0;
pub const SCHEDULE: u8 = 2;
pub const DELIVERED: u8 = 3;
pub const LOST: u8 = 4;
pub const FUEL: u8 = 5;
pub const REMOVED: u8 = 6;
pub const INVENTORY: u8 = 8;
pub const PRICES: usize = 0;
pub const SUBSIDIES: usize = 1;
pub const AUTOMATION: usize = 2;
const COMMAND_COUNTRY: usize = 0x28;
pub const ONCE_APPLIED: u8 = 9;
pub const REQUEST_CREATED: u8 = 10;
pub const CONTRACT_CREATED: u8 = 11;
pub const CONTRACT_REMOVED: u8 = 12;
pub const AMENDMENT_COST: u8 = 19;
pub const CONTRACT_DELIVERED: u8 = 20;
pub const CONTRACT_PAYMENT: u8 = 21;
pub const POOL_VALUE: u8 = 22;
pub const POOL_END: u8 = 23;
pub const INVENTORY_MODE: u8 = 0;
pub const ROUTE_MODE: u8 = 4;
pub const ROUTE_BEGIN: u8 = 24;
pub const CONTRACT_STATE: u8 = 28;
pub const ROUTE_OPERAND: u8 = 30;
pub const FINAL_BEGIN: u8 = 31;
pub const FINAL_ITEM: u8 = 32;
pub const FINAL_END: u8 = 33;
pub const DELIVERY_UNAVAILABLE: u8 = 34;
const COUNTRY_RESET: u8 = 5;
const ADMISSION: u8 = 2;
const HOLDER_MARK: u8 = 3;
const FINAL_CONTRACT: u8 = 4;
const AGREEMENT_MARK: u8 = 2;
const CONTRACT_MARK: u8 = 1;
const INVENTORY_MARK: u8 = 0;
const READ_DELIVERED: u8 = 1;
const READ_SUNK: u8 = 2;
const READ_ONCE: u8 = 4;
const READ_ROUTE: u8 = 1;
const READ_RUNTIME: u8 = 2;
const READ_PURCHASE_TOTAL: u8 = 4;
const DERIVED_TOTAL: u8 = 0x80;
pub const OBSERVED_SITES: [usize; 25] = [
    0, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 23, 24, 25, 29, 30,
];
static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
std::thread_local! {
    static DEPTH: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
}
const VARIABLES_BLOOM_WORDS: usize = 64;
const VARIABLES_BLOOM_MASK: usize = VARIABLES_BLOOM_WORDS * 64 - 1;
static VARIABLES_WATCH: WatchRegistry = WatchRegistry::marks_only();
static VARIABLES_BLOOM: [AtomicU64; VARIABLES_BLOOM_WORDS] =
    [const { AtomicU64::new(0) }; VARIABLES_BLOOM_WORDS];
static VARIABLES_FORWARDED: AtomicBool = AtomicBool::new(false);
static VARIABLES_UNOBSERVED: AtomicBool = AtomicBool::new(false);
#[inline(always)]
fn variables_bloom_bits(at: usize) -> (usize, u64, usize, u64) {
    let mut hash = (at as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    hash ^= hash >> 32;
    let first = (hash as usize) & VARIABLES_BLOOM_MASK;
    let second = ((hash >> 12) as usize) & VARIABLES_BLOOM_MASK;
    (
        first >> 6,
        1_u64 << (first & 63),
        second >> 6,
        1_u64 << (second & 63),
    )
}
#[inline(always)]
fn variables_bloom_contains(at: usize) -> bool {
    let (w0, m0, w1, m1) = variables_bloom_bits(at);
    (VARIABLES_BLOOM[w0].load(Ordering::Relaxed) & m0 != 0)
        && (VARIABLES_BLOOM[w1].load(Ordering::Relaxed) & m1 != 0)
}
fn register_variable_watch(at: usize) -> Option<((u64, u64), bool)> {
    let (w0, m0, w1, m1) = variables_bloom_bits(at);
    VARIABLES_BLOOM[w0].fetch_or(m0, Ordering::SeqCst);
    VARIABLES_BLOOM[w1].fetch_or(m1, Ordering::SeqCst);
    VARIABLES_WATCH.register(at)
}
pub fn variable_write(at: usize) -> Notification<'static> {
    VARIABLES_WATCH.notify_if(at, || variables_bloom_contains(at))
}
pub fn variables_uncertified() {
    VARIABLES_UNOBSERVED.store(true, Ordering::SeqCst);
}
fn take_unobserved_variables() -> bool {
    VARIABLES_UNOBSERVED.swap(false, Ordering::SeqCst)
}
pub fn observe_variables(enabled: bool) {
    VARIABLES_FORWARDED.store(enabled, Ordering::SeqCst);
    refresh_coverage();
}
std::thread_local! { static CONTRACT_DELIVERY: std::cell::Cell<Option<MarketEventRow>> = const { std::cell::Cell::new(None) }; }
const STOCKPILE_POOL: usize = 0x38;
const STOCKPILE_COUNTRY: usize = 0x20;

impl Event for MarketEventRow {
    fn set_id(&mut self, id: u64) {
        self.id = id;
    }
}
static LEDGER: EventLedger<MarketEventRow> = EventLedger::new();
fn country_receiver(owner: u32) -> usize {
    (owner as usize + 1) << 3
}
static RESET_WATCH: WatchRegistry = WatchRegistry::marks_only();
static WATCHES: [WatchRegistry; 3] = [const { WatchRegistry::marks_only() }; 3];
static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; native::SITES.len()] =
    [const { AtomicUsize::new(0) }; native::SITES.len()];
static ENABLED: AtomicBool = AtomicBool::new(false);
static RESET_FORWARDED: AtomicBool = AtomicBool::new(false);
static UNHOOKED: OnceLock<String> = OnceLock::new();
static PUBLICATION_FAILURE: OnceLock<String> = OnceLock::new();
static FLIGHTS: AtomicUsize = AtomicUsize::new(0);
static OCCURRENCE: AtomicU64 = AtomicU64::new(0);
static AMENDMENTS: AtomicU64 = AtomicU64::new(0);
static AMENDMENT_ITEMS: AtomicU64 = AtomicU64::new(0);
static FINAL_SNAPSHOTS: AtomicU64 = AtomicU64::new(0);
static FINAL_ITEMS: AtomicU64 = AtomicU64::new(0);
static DELIVERY_AMOUNT_REFUSALS: AtomicU64 = AtomicU64::new(0);
static AMENDMENT_TIMES: std::sync::Mutex<
    Option<collector_extended_support::distribution::Distribution>,
> = std::sync::Mutex::new(None);
static COUNTRY_WATCH_DEMAND: AtomicUsize = AtomicUsize::new(0);
static REPLACEMENT_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SCHEDULE_DEMAND: AtomicUsize = AtomicUsize::new(0);
static CONTRACT_DEMAND: AtomicUsize = AtomicUsize::new(0);

pub fn active() -> usize {
    FLIGHTS.load(Ordering::SeqCst)
        + VARIABLES_WATCH.active()
        + RESET_WATCH.active()
        + WATCHES.iter().map(WatchRegistry::active).sum::<usize>()
}
pub fn capacity() -> usize {
    LEDGER
        .demand()
        .max(REPLACEMENT_DEMAND.load(Ordering::Acquire))
}
pub fn restore_sequence(previous: Option<u64>) -> Result<(), &'static str> {
    LEDGER.restore_sequence(previous)?;
    let next = previous
        .map_or(Some(0), |id| id.checked_add(1))
        .ok_or("market occurrence identity exhausted")?;
    OUTPUT_SEQUENCE.fetch_max(next, Ordering::AcqRel);
    OCCURRENCE.fetch_max(next, Ordering::AcqRel);
    Ok(())
}
pub fn schedule_storage(
    memory: &impl Memory,
    base: usize,
    resources: usize,
) -> Result<usize, &'static str> {
    let list = engine_native_memory::cpdx_array::Array::read(memory, resources + lease::OUTGOING, 8)
        .ok_or("market sizing exchange list unreadable")?;
    let mut demand = 0_usize;
    for index in 0..list.count {
        let pointer = read::<u64>(
            memory,
            list.element(index, 8)
                .ok_or("market sizing exchange address overflow")?,
        )
        .ok_or("market sizing exchange unreadable")? as usize;
        lease::identity(memory, base, pointer)?;
        demand = demand
            .checked_add(12)
            .ok_or("market sizing demand overflow")?;
        for offset in [lease::DELIVERED, lease::SUNK] {
            let entries =
                engine_native::cequipment_variant::pool::entries(memory, base, pointer + offset)?;
            demand = demand
                .checked_add(entries.count as usize)
                .ok_or("market sizing demand overflow")?;
        }
        for offset in [
            lease::MONTHLY_NEED,
            lease::PERCENTAGE_NEED,
            lease::ONCE_NEED,
        ] {
            let entries =
                engine_native::cequipment_variant::pool::entries(memory, base, pointer + offset)?;
            demand = demand
                .checked_add(entries.count as usize)
                .ok_or("market sizing demand overflow")?;
        }
    }
    Ok(demand)
}
/// # Safety
/// Run only during attach or the active session's serial sizing pass (or an isolated test), while
/// market observer storage remains live and teardown is excluded.
pub unsafe fn presize(demand: usize, countries: usize) {
    SCHEDULE_DEMAND.fetch_max(demand, Ordering::Release);
    prepare_events();
    VARIABLES_WATCH.offer(countries);
    VARIABLES_WATCH.collect();
    RESET_WATCH.offer(countries);
    RESET_WATCH.collect();
    for watch in &WATCHES {
        watch.offer(countries);
        watch.collect();
    }
    COUNTRY_WATCH_DEMAND.fetch_max(countries, Ordering::Release);
}
fn prepare_events() {
    match SCHEDULE_DEMAND
        .load(Ordering::Acquire)
        .checked_add(CONTRACT_DEMAND.load(Ordering::Acquire))
    {
        Some(demand) => LEDGER.prepare(demand.max(capacity())),
        None => failure("market event storage demand overflow"),
    }
}
pub fn contract_storage(_items: usize, _in_transit: usize) -> Option<usize> {
    Some(10)
}
/// # Safety
/// Run only during admitted economy capture or the active session's sizing pass (or an isolated
/// test), while market observer storage remains live and teardown is excluded.
pub unsafe fn prepare_contract_events(demand: usize) {
    CONTRACT_DEMAND.fetch_max(demand, Ordering::Release);
    prepare_events();
}
pub fn mark_stranded() {
    LEDGER.mark_stranded();
}
/// # Safety
/// Caller must own the active session's recovery or publication failure path (or an isolated test
/// session) when invalidating market-event drains.
pub unsafe fn lost_drain() {
    // SAFETY: Caller owns the market recovery or publication failure path.
    unsafe { LEDGER.lost_drain() };
}
/// # Safety
/// Caller must own the active session's projection or publication failure path (or an isolated
/// test session) when recording lost market-event output.
pub unsafe fn lost_output(count: usize) {
    // SAFETY: Caller owns the market projection or publication failure path.
    unsafe { LEDGER.lost_output(count) };
}
/// # Safety
/// Caller must run on the admitted hourly observer boundary as the sole consumer advancing market
/// observation and watch tables (or an isolated test session).
pub unsafe fn boundary(clock: Option<engine_native::cgame_state::Clock<u64>>, invalidate: bool) {
    // SAFETY: Caller runs on the admitted hourly observer boundary for market observation.
    unsafe { LEDGER.boundary(clock) };
    let invalidate = invalidate || LEDGER.interval_incomplete() || held::recovery_requested();
    let renew = if invalidate {
        [None; 4]
    } else {
        held::begin_hour()
    };
    let mut renewed = [None; 4];
    for (domain, watch) in WATCHES.iter().enumerate() {
        if watch
            .boundary(invalidate || renew[domain].is_some())
            .renewed()
        {
            renewed[domain] = renew[domain];
        }
    }
    if VARIABLES_WATCH.boundary(invalidate).renewed() && VARIABLES_WATCH.active() == 0 {
        for word in &VARIABLES_BLOOM {
            word.store(0, Ordering::SeqCst);
        }
    }
    if RESET_WATCH
        .boundary(invalidate || renew[3].is_some())
        .renewed()
    {
        renewed[3] = renew[3];
    }
    if invalidate {
        held::invalidate();
    } else {
        held::finish_renewals(renewed);
        if RESET_WATCH.take(|_| {}).is_none() {
            held::refuse("market reset certificate unavailable");
        }
    }
}
fn refresh_coverage() {
    let enabled = ENABLED.load(Ordering::SeqCst)
        && RESET_FORWARDED.load(Ordering::SeqCst)
        && VARIABLES_FORWARDED.load(Ordering::SeqCst);
    VARIABLES_WATCH.set_enabled(enabled);
    RESET_WATCH.set_enabled(enabled);
    for watch in &WATCHES {
        watch.set_enabled(enabled);
    }
}
pub fn observe_resets(enabled: bool) {
    RESET_FORWARDED.store(enabled, Ordering::SeqCst);
    refresh_coverage();
}
pub struct CountryReset {
    _marks: [Option<Notification<'static>>; 4],
    owner: Option<u32>,
}
impl Drop for CountryReset {
    fn drop(&mut self) {
        if let Some(owner) = self.owner {
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let result = country_reset_completed(&memory, BASE.load(Ordering::Relaxed), owner);
            if let Err(reason) = result {
                failure(reason);
            }
        }
    }
}
fn country_reset_completed(
    memory: &impl Memory,
    base: usize,
    owner: u32,
) -> Result<(), &'static str> {
    let event = root(
        memory,
        base,
        lease::Identity {
            id: 0,
            giver: owner,
            receiver: 0,
        },
        AMENDMENT_COST,
    )?;
    append(MarketEventRow {
        mode: COUNTRY_RESET,
        ..event
    })
}
pub fn country_reset(country: usize) -> CountryReset {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let owner = read::<u32>(&memory, country.wrapping_add(engine_native::ccountry::ID));
    if ENABLED.load(Ordering::SeqCst) && owner.is_none() {
        failure("market country reset identity unreadable");
    }
    CountryReset {
        _marks: std::array::from_fn(|domain| {
            owner.map(|id| {
                if domain == 3 {
                    RESET_WATCH.notify(country_receiver(id))
                } else {
                    WATCHES[domain].notify(country_receiver(id))
                }
            })
        }),
        owner: owner.filter(|_| ENABLED.load(Ordering::SeqCst)),
    }
}
/// # Safety
/// Caller must be the admitted hourly capture consumer draining market events while observer
/// storage stays live and teardown is excluded.
pub unsafe fn drain(output: &mut Vec<MarketEventRow>) -> Result<usize, &'static str> {
    if !ENABLED.load(Ordering::SeqCst)
        || !RESET_FORWARDED.load(Ordering::SeqCst)
        || !VARIABLES_FORWARDED.load(Ordering::SeqCst)
        || UNHOOKED.get().is_some()
    {
        return Err("market writer coverage incomplete (see market_changes status)");
    }
    // SAFETY: Caller holds hourly capture admission as the sole market-events consumer.
    let count = unsafe { LEDGER.drain(output)? };
    let mut times = AMENDMENT_TIMES
        .lock()
        .map_err(|_| "market amendment timing storage poisoned")?;
    for row in output
        .iter()
        .filter(|row| row.kind == AMENDMENT_COST && row.mode == 0)
    {
        if let Some(ns) =
            crate::hook_timing::ticks_to_ns(row.amount_raw as u64, crate::hook_timing::tsc_hz())
        {
            times.get_or_insert_with(Default::default).record_ns(ns);
        }
    }
    output.retain(|row| {
        row.kind != AMENDMENT_COST
            || matches!(
                row.mode,
                HOLDER_MARK | FINAL_CONTRACT | COUNTRY_RESET | ADMISSION
            )
    });
    Ok(count)
}
fn failure(reason: &'static str) {
    LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
    LEDGER.refuse(reason);
}
fn append(event: MarketEventRow) -> Result<(), &'static str> {
    LEDGER.attempt();
    let token = LEDGER.claim((event.occurrence as usize, event.kind as usize))?;
    LEDGER.complete(token, Ok(Some(event)))
}
fn root(
    memory: &impl Memory,
    base: usize,
    identity: lease::Identity,
    kind: u8,
) -> Result<MarketEventRow, &'static str> {
    let state = read::<u64>(
        memory,
        engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
            .ok_or("market game address overflow")?,
    )
    .filter(|p| *p != 0)
    .ok_or("market game state unreadable")? as usize;
    let hour = read(
        memory,
        state
            .checked_add(engine_native::cgame_state::CURRENT_HOUR)
            .ok_or("market clock address overflow")?,
    )
    .ok_or("market clock unreadable")?;
    let occurrence = OCCURRENCE
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
        .map_err(|_| "market occurrence identity exhausted")?;
    Ok(MarketEventRow {
        occurrence,
        hour,
        agreement: identity.id,
        country: identity.giver,
        receiver: identity.receiver,
        kind,
        ..Default::default()
    })
}
fn scalar<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    at: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(
        memory,
        at.checked_add(offset)
            .ok_or("market operand address overflow")?,
    )
    .ok_or("market writer operand unreadable")
}
fn global<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: engine_sites::Rva,
) -> Result<T, &'static str> {
    read(
        memory,
        engine_sites::at(base, key).ok_or("market global withheld")?,
    )
    .ok_or("market writer operand unreadable")
}
fn replacement(memory: &impl Memory, base: usize, at: usize) -> Result<(), &'static str> {
    let started = unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    };
    let identity = lease::identity(memory, base, at)?;
    let mut event = root(memory, base, identity, AMENDMENT)?;
    event.date = scalar(memory, at, lease::ACTIVE_SINCE)?;
    event.amount_raw = scalar(memory, at, lease::FUEL_DAILY)?;
    event.auxiliary_raw = scalar(memory, at, lease::FUEL_PERCENTAGE)?;
    let offsets = [
        lease::MONTHLY_NEED,
        lease::PERCENTAGE_NEED,
        lease::ONCE_NEED,
    ];
    let sources = offsets.map(|offset| {
        at.checked_add(offset)
            .ok_or("market schedule address overflow")
    });
    let sources = [sources[0]?, sources[1]?, sources[2]?];
    let count = lease::schedules(
        memory,
        base,
        sources,
        |header| {
            let count = header
                .counts
                .into_iter()
                .try_fold(0_usize, |a, n| a.checked_add(n as usize))
                .ok_or("market schedule size overflow")?;
            REPLACEMENT_DEMAND.fetch_max(count.saturating_add(2), Ordering::Release);
            let mut first = event;
            first.flags = header
                .allow_zero
                .into_iter()
                .enumerate()
                .fold(0, |flags, (i, zero)| flags | (u8::from(zero) << i));
            first.ordinal = u32::try_from(count)
                .map_err(|_| "market schedule size exceeds occurrence framing")?;
            append(first)
        },
        |item| {
            append(MarketEventRow {
                kind: SCHEDULE,
                mode: item.mode,
                ordinal: item.ordinal,
                variant: item.variant,
                amount_raw: item.amount_raw,
                auxiliary_raw: 0,
                ..event
            })
        },
    )?;
    let ticks = unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
    .saturating_sub(started);
    append(MarketEventRow {
        kind: AMENDMENT_COST,
        mode: 0,
        amount_raw: ticks as i64,
        auxiliary_raw: count as i64,
        ..event
    })?;
    AMENDMENTS.fetch_add(1, Ordering::Relaxed);
    AMENDMENT_ITEMS.fetch_add(count as u64, Ordering::Relaxed);
    Ok(())
}
fn mark_holder(
    event: MarketEventRow,
    pointer: usize,
    domain: u8,
    demand: u8,
) -> Result<(), &'static str> {
    append(MarketEventRow {
        kind: AMENDMENT_COST,
        mode: HOLDER_MARK,
        flags: domain,
        ordinal: u32::from(demand),
        total_raw: pointer as i64,
        ..event
    })
}
fn equipment(
    memory: &impl Memory,
    base: usize,
    delivered: bool,
    destination: usize,
    _source: usize,
) -> Result<(), &'static str> {
    let pointer = destination
        .checked_sub(if delivered {
            lease::DELIVERED
        } else {
            lease::SUNK
        })
        .ok_or("market delivery destination invalid")?;
    let event = root(
        memory,
        base,
        lease::identity(memory, base, pointer)?,
        if delivered { DELIVERED } else { LOST },
    )?;
    mark_holder(
        event,
        pointer,
        AGREEMENT_MARK,
        if delivered { READ_DELIVERED } else { READ_SUNK },
    )
}
fn inventory(
    memory: &impl Memory,
    base: usize,
    _site: usize,
    destination: usize,
    _source: usize,
    _amount: usize,
) -> Result<(), &'static str> {
    let stockpile = destination
        .checked_sub(STOCKPILE_POOL)
        .ok_or("market stockpile operand invalid")?;
    let owner = scalar(memory, stockpile, STOCKPILE_COUNTRY)?;
    let event = root(
        memory,
        base,
        lease::Identity {
            giver: owner,
            ..Default::default()
        },
        INVENTORY,
    )?;
    mark_holder(event, stockpile, INVENTORY_MARK, 0)
}
fn final_pools(memory: &impl Memory, base: usize, pointer: usize) -> Result<(), &'static str> {
    let event = root(
        memory,
        base,
        lease::identity(memory, base, pointer)?,
        FINAL_BEGIN,
    )?;
    let offsets = [lease::DELIVERED, lease::SUNK];
    let headers = offsets.map(|offset| {
        engine_native::cequipment_variant::pool::entries(memory, base, pointer + offset)
    });
    let headers = [headers[0]?, headers[1]?];
    let mut flags = 0;
    let mut count = 0_usize;
    for (mode, header) in headers.iter().enumerate() {
        flags |= u8::from(
            scalar::<u8>(
                memory,
                pointer + offsets[mode],
                engine_native::cequipment_variant::pool::ALLOW_ZERO,
            )? != 0,
        ) << mode;
        count = count
            .checked_add(header.count as usize)
            .ok_or("market final pool demand overflow")?;
    }
    REPLACEMENT_DEMAND.fetch_max(count.saturating_add(2), Ordering::Release);
    append(MarketEventRow { flags, ..event })?;
    for (mode, header) in headers.into_iter().enumerate() {
        for ordinal in 0..header.count {
            let item = engine_native_parts::equipment_entry::read(
                memory,
                base,
                header
                    .element(ordinal, engine_native_parts::equipment_entry::STRIDE)
                    .ok_or("market final entry address overflow")?,
                ordinal,
            )?;
            append(MarketEventRow {
                kind: FINAL_ITEM,
                mode: mode as u8,
                ordinal,
                variant: item.variant_handle,
                amount_raw: item.amount_raw,
                ..event
            })?;
        }
    }
    append(MarketEventRow {
        kind: FINAL_END,
        ..event
    })?;
    FINAL_SNAPSHOTS.fetch_add(1, Ordering::Relaxed);
    FINAL_ITEMS.fetch_add(count as u64, Ordering::Relaxed);
    Ok(())
}
fn contract_final(
    memory: &impl Memory,
    pointer: usize,
    event: MarketEventRow,
) -> Result<(), &'static str> {
    append(MarketEventRow {
        mode: FINAL_CONTRACT,
        flags: u8::from(
            scalar::<u8>(
                memory,
                pointer + engine_production::market_ledger::DRAFT,
                engine_production::market_ledger::DEFINITION_COST_VALID,
            )? != 0,
        ),
        ordinal: scalar::<i32>(
            memory,
            pointer,
            engine_production::cpurchase_contract::COMPLETED,
        )? as u32,
        amount_raw: scalar(
            memory,
            pointer,
            engine_production::cpurchase_contract::EQUIPMENT_PROGRESS,
        )?,
        auxiliary_raw: scalar(
            memory,
            pointer + engine_production::market_ledger::DRAFT,
            engine_production::market_ledger::DEFINITION_TOTAL_COST,
        )?,
        ..event
    })
}
fn contract_removing(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    destructor: bool,
) -> Result<(), &'static str> {
    let id: u64 = scalar(
        memory,
        pointer,
        engine_production::cpurchase_contract::ID_TYPE,
    )?;
    if id >> 32 == u64::from(native::RETIRED_REFERENCE_ID)
        && scalar::<u8>(
            memory,
            pointer,
            engine_production::cpurchase_contract::REGISTERED,
        )? == 0
    {
        return if destructor {
            Ok(())
        } else {
            Err("market removal contract identity invalid")
        };
    }
    if id as u32 != engine_production::cpurchase_contract::TYPE || id >> 32 == 0 {
        return Err("market removal contract identity invalid");
    }
    let event = root(
        memory,
        base,
        lease::Identity {
            id,
            giver: scalar(
                memory,
                pointer,
                engine_production::cpurchase_contract::SELLER,
            )?,
            receiver: scalar(
                memory,
                pointer,
                engine_production::cpurchase_contract::BUYER,
            )?,
        },
        AMENDMENT_COST,
    )?;
    contract_final(memory, pointer, event)?;
    append(MarketEventRow {
        kind: CONTRACT_REMOVED,
        flags: 1,
        ..event
    })
}
pub fn publish(events: &mut Vec<MarketEventRow>) -> Result<(), &'static str> {
    events.retain(|row| {
        row.kind != AMENDMENT_COST && (row.kind < FINAL_BEGIN || row.kind == DELIVERY_UNAVAILABLE)
    });
    for event in events {
        OUTPUT_SEQUENCE.fetch_max(event.occurrence, Ordering::AcqRel);
        event.id = OUTPUT_SEQUENCE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| "market output identity exhausted")?;
    }
    Ok(())
}
fn contract_root(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    kind: u8,
) -> Result<MarketEventRow, &'static str> {
    let identity = lease::Identity {
        id: scalar(
            memory,
            pointer,
            engine_production::cpurchase_contract::ID_TYPE,
        )?,
        giver: scalar(
            memory,
            pointer,
            engine_production::cpurchase_contract::SELLER,
        )?,
        receiver: scalar(
            memory,
            pointer,
            engine_production::cpurchase_contract::BUYER,
        )?,
    };
    root(memory, base, identity, kind)
}
fn contract_created(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    request: bool,
) -> Result<(), &'static str> {
    let event = contract_root(
        memory,
        base,
        pointer,
        if request {
            REQUEST_CREATED
        } else {
            CONTRACT_CREATED
        },
    )?;
    append(event)?;
    append(MarketEventRow {
        kind: AMENDMENT_COST,
        mode: ADMISSION,
        flags: u8::from(!request),
        total_raw: pointer as i64,
        ..event
    })
}
struct DeliveryScope(Option<MarketEventRow>);
impl Drop for DeliveryScope {
    fn drop(&mut self) {
        CONTRACT_DELIVERY.with(|slot| slot.set(self.0));
    }
}
fn owner_of_status(memory: &impl Memory, status: usize) -> Result<u32, &'static str> {
    let country: u64 = scalar(
        memory,
        status,
        engine_production::market_ledger::COUNTRY_BACKLINK,
    )?;
    scalar(memory, country as usize, engine_native::ccountry::ID)
}

type Native = unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize, usize) -> usize;
fn run(site: usize, args: [usize; 6]) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::MarketChanges);
    FLIGHTS.fetch_add(1, Ordering::SeqCst);
    let original: Native = unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    if !ENABLED.load(Ordering::SeqCst) {
        let result = timing
            .original(|| unsafe { original(args[0], args[1], args[2], args[3], args[4], args[5]) });
        FLIGHTS.fetch_sub(1, Ordering::SeqCst);
        return result;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let domain = match site {
        11 => Some(PRICES),
        12..=14 => Some(SUBSIDIES),
        15 => Some(AUTOMATION),
        _ => None,
    };
    let notification = domain.and_then(|domain| {
        let owner = if site == 15 {
            scalar(&memory, args[0], COMMAND_COUNTRY)
        } else {
            owner_of_status(&memory, args[0])
        };
        match owner {
            Ok(owner) => Some(WATCHES[domain].notify(country_receiver(owner))),
            Err(reason) => {
                WATCHES[domain].poison();
                failure(reason);
                None
            }
        }
    });
    let observing_event = domain.is_none() && site != 22 && site != 23;
    let depth = if observing_event {
        DEPTH.with(|slot| {
            let previous = slot.get();
            slot.set(previous + 1);
            previous
        })
    } else {
        0
    };
    if observing_event && depth == 0 {
        LEDGER.enter();
    }
    let registered = if site == 17 {
        read::<u64>(&memory, args[1]).map(|p| p as usize)
    } else {
        None
    };
    let _delivery_scope = if site == 21 {
        match contract_root(&memory, base, args[0], CONTRACT_DELIVERED) {
            Ok(event) => Some(DeliveryScope(CONTRACT_DELIVERY.with(|slot| {
                let previous = slot.get();
                slot.set(Some(event));
                previous
            }))),
            Err(reason) => {
                failure(reason);
                None
            }
        }
    } else {
        None
    };
    if matches!(site, 21 | 24 | 25)
        && let Err(reason) =
            contract_root(&memory, base, args[0], CONTRACT_STATE).and_then(|event| {
                mark_holder(
                    event,
                    args[0],
                    CONTRACT_MARK,
                    READ_RUNTIME
                        | if site == 21 {
                            READ_PURCHASE_TOTAL
                        } else if site == 25 {
                            READ_ROUTE
                        } else {
                            0
                        },
                )
            })
    {
        failure(reason);
    }
    if (site == 0 || site == 3)
        && let Err(reason) = final_pools(&memory, base, args[0])
    {
        failure(reason);
    }
    if site == 20 || site == native::CONTRACT_REMOVAL_SITE {
        let pointer = if site == 20 { args[0] } else { args[1] };
        if let Err(reason) = contract_removing(&memory, base, pointer, site == 20) {
            failure(reason);
        }
    }
    if site == 19 {
        let pointer = args[1];
        let witness =
            contract_root(&memory, base, pointer, CONTRACT_REMOVED).and_then(|mut event| {
                event.flags = u8::from(site == 20);
                append(event)
            });
        if let Err(reason) = witness {
            failure(reason);
        }
    }
    if site == 3 {
        let witness = lease::identity(&memory, base, args[0])
            .and_then(|identity| root(&memory, base, identity, REMOVED))
            .and_then(append);
        if let Err(reason) = witness {
            failure(reason);
        }
    }
    let result = timing
        .original(|| unsafe { original(args[0], args[1], args[2], args[3], args[4], args[5]) });
    let captured = match site {
        0 => replacement(&memory, base, args[0]),
        5 | 6 => equipment(&memory, base, site == 5, args[0], args[1]),
        7..=10 => inventory(&memory, base, site, args[0], args[1], args[2]),
        16 => (|| {
            let pointer = args[0]
                .checked_sub(lease::ONCE_NEED)
                .ok_or("lend-lease once operand invalid")?;
            let event = root(
                &memory,
                base,
                lease::identity(&memory, base, pointer)?,
                ONCE_APPLIED,
            )?;
            mark_holder(event, pointer, AGREEMENT_MARK, READ_ONCE)
        })(),
        17 => registered
            .ok_or("market registered contract operand unreadable")
            .and_then(|pointer| contract_created(&memory, base, pointer, false)),
        18 => contract_created(&memory, base, result, true),
        23 => CONTRACT_DELIVERY
            .with(|slot| slot.get())
            .ok_or("market payment has no native contract scope")
            .and_then(|event| {
                append(MarketEventRow {
                    kind: CONTRACT_PAYMENT,
                    amount_raw: args[1] as i64,
                    ..event
                })
            }),
        _ => Ok(()),
    };
    if let Err(reason) = captured {
        failure(reason);
    }
    drop(notification);
    if observing_event && depth == 0 {
        LEDGER.leave();
    }
    if observing_event {
        DEPTH.with(|slot| slot.set(depth));
    }
    FLIGHTS.fetch_sub(1, Ordering::SeqCst);
    result
}
fn fuel_store(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    amount: i64,
    lost: bool,
) -> Result<(), &'static str> {
    if amount == 0 {
        return Ok(());
    }
    let event = root(memory, base, lease::identity(memory, base, pointer)?, FUEL)?;
    append(MarketEventRow {
        mode: u8::from(lost),
        amount_raw: if lost { 0 } else { amount },
        auxiliary_raw: if lost { amount } else { 0 },
        ..event
    })
}
unsafe extern "system-unwind" fn fuel_write(registers: &Registers, tag: usize) {
    let amount = registers.rax as i64;
    if amount == 0 || !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::MarketChanges);
    FLIGHTS.fetch_add(1, Ordering::SeqCst);
    let outer = DEPTH.with(|slot| slot.get() == 0);
    if outer {
        LEDGER.enter();
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    if let Err(reason) = fuel_store(
        &memory,
        BASE.load(Ordering::Acquire),
        registers.rsi,
        amount,
        tag == native::FUEL_LOST_SITE,
    ) {
        failure(reason);
    }
    if outer {
        LEDGER.leave();
    }
    FLIGHTS.fetch_sub(1, Ordering::SeqCst);
}
macro_rules! callback {
    ($name:ident, $index:expr) => {
        unsafe extern "system-unwind" fn $name(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
            f: usize,
        ) -> usize {
            run($index, [a, b, c, d, e, f])
        }
    };
}
callback!(amendment, 0);
callback!(removal, 3);

callback!(delivered, 5);
callback!(lost, 6);
callback!(reserve, 7);
callback!(release, 8);
callback!(returned, 9);
callback!(committed, 10);
callback!(once_applied, 16);
callback!(contract_registered, 17);
callback!(contract_removed, 30);
callback!(request_created, 18);
callback!(request_removed, 19);
callback!(contract_destroyed, 20);
callback!(contract_delivery, 21);
callback!(contract_payment, 23);
callback!(contract_daily, 24);
callback!(route_rebuild, 25);
callback!(prices, 11);
callback!(grant, 12);
callback!(subsidy_use, 13);
callback!(subsidy_return, 14);
callback!(automation, 15);

fn site_failure(index: usize, phase: &str, reason: impl std::fmt::Display) -> String {
    format!(
        "market_changes {}: {phase}: {reason}",
        native::SITES[index].name
    )
}

fn located(base: usize, index: usize) -> Result<(usize, [u8; 16]), String> {
    engine_sites::hook(base, native::SITES[index].rva)
        .and_then(|code| Ok((code.address, code.sentinel()?)))
        .map_err(|reason| site_failure(index, "sentinel", reason))
}

fn verify_sites(base: usize) -> Result<(), String> {
    for &index in &OBSERVED_SITES {
        located(base, index)?;
    }
    Ok(())
}

fn publication_failure(phase: &str, reason: impl std::fmt::Display) -> String {
    let sites = OBSERVED_SITES
        .map(|index| native::SITES[index].name)
        .join(", ");
    format!("market_changes [{sites}]: {phase}: {reason}")
}

fn remember_publication_failure(phase: &str, reason: impl std::fmt::Display) -> &'static str {
    PUBLICATION_FAILURE
        .get_or_init(|| publication_failure(phase, reason))
        .as_str()
}

pub struct Hooks {
    hooks: Vec<Hook>,
    taps: Vec<Tap>,
}
impl Hooks {
    /// # Safety
    /// The host verified the executable and exclusively owns these dedicated sites.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Release);
        let callbacks = [
            amendment as *const () as usize,
            0,
            0,
            removal as *const () as usize,
            0,
            delivered as *const () as usize,
            lost as *const () as usize,
            reserve as *const () as usize,
            release as *const () as usize,
            returned as *const () as usize,
            committed as *const () as usize,
            prices as *const () as usize,
            grant as *const () as usize,
            subsidy_use as *const () as usize,
            subsidy_return as *const () as usize,
            automation as *const () as usize,
            once_applied as *const () as usize,
            contract_registered as *const () as usize,
            request_created as *const () as usize,
            request_removed as *const () as usize,
            contract_destroyed as *const () as usize,
            contract_delivery as *const () as usize,
            0,
            contract_payment as *const () as usize,
            contract_daily as *const () as usize,
            route_rebuild as *const () as usize,
            0,
            0,
            0,
            0,
            contract_removed as *const () as usize,
        ];
        let install = || -> Result<(Vec<Hook>, Vec<Tap>), String> {
            verify_sites(base)?;
            let mut hooks = Vec::with_capacity(OBSERVED_SITES.len() - 2);
            let mut taps = Vec::with_capacity(2);
            for &index in &OBSERVED_SITES {
                let site = &native::SITES[index];
                let (at, original) = located(base, index)?;
                if index == native::FUEL_SENT_SITE || index == native::FUEL_LOST_SITE {
                    let field = if index == native::FUEL_SENT_SITE {
                        lease::FUEL_SENT
                    } else {
                        lease::FUEL_SUNK
                    };
                    if original[..3] != [0x48, 0x01, 0x86]
                        || u32::from_le_bytes([original[3], original[4], original[5], original[6]])
                            as usize
                            != field
                    {
                        return Err(site_failure(index, "sentinel", "fuel store differs"));
                    }
                    taps.push(
                        unsafe { Tap::new(at, fuel_write, index, original) }
                            .map_err(|reason| site_failure(index, "tap construction", reason))?,
                    );
                    continue;
                }
                if callbacks[index] == 0 {
                    return Err(site_failure(
                        index,
                        "observer selection",
                        "no game-state callback",
                    ));
                }
                let target = site
                    .call_target
                    .map(|key| engine_sites::hook(base, key).map(|code| code.address))
                    .transpose()
                    .map_err(|reason| site_failure(index, "call target", reason))?;
                let hook = if target.is_some() {
                    unsafe { Hook::new_call(at, callbacks[index], original) }
                } else {
                    unsafe { Hook::new(at, callbacks[index], original) }
                }
                .map_err(|reason| site_failure(index, "hook construction", reason))?;
                ORIGINAL[index].store(
                    target.unwrap_or_else(|| hook.trampoline()),
                    Ordering::Release,
                );
                hooks.push(hook);
            }
            Ok((hooks, taps))
        };
        match install() {
            Ok((hooks, taps)) => Self { hooks, taps },
            Err(reason) => {
                let _ = UNHOOKED.set(reason);
                Self {
                    hooks: Vec::new(),
                    taps: Vec::new(),
                }
            }
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if enabled && let Some(reason) = UNHOOKED.get() {
            return Err(reason.as_str());
        }
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            refresh_coverage();
        }
        let ready = match crate::hook::shared::attach_outcome() {
            Some(outcome) => {
                outcome.map_err(|reason| remember_publication_failure("publication", reason))?
            }
            None => Hook::set_enabled_iter(
                self.hooks
                    .iter_mut()
                    .chain(self.taps.iter_mut().map(Tap::publication_hook)),
                enabled,
            )
            .map_err(|reason| remember_publication_failure("publication", reason))?,
        };
        if !ready {
            return Ok(false);
        }
        ENABLED.store(
            enabled && self.hooks.len() + self.taps.len() == OBSERVED_SITES.len(),
            Ordering::SeqCst,
        );
        refresh_coverage();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.taps.iter().flat_map(Tap::executable_ranges))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    pub(super) static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    pub struct Session(std::sync::MutexGuard<'static, ()>);
    impl Session {
        /// # Safety
        /// Caller must run in a detached, isolated test process with no live collector session
        /// for the lifetime of the returned guard; creating a `Session` resets and enables the
        /// process-wide market observer state.
        pub unsafe fn new() -> Self {
            let guard = SERIAL.lock().unwrap();
            LEDGER.reset();
            VARIABLES_UNOBSERVED.store(false, Ordering::SeqCst);
            ENABLED.store(true, Ordering::SeqCst);
            observe_resets(true);
            observe_variables(true);
            // SAFETY: Test session holds `SERIAL`, owning the global market observer state.
            unsafe { presize(16, 16) };
            held::invalidate();
            Self(guard)
        }
    }
    impl Session {
        pub fn seed(&self, rows: [replay_data::rows::TableRows; 3]) {
            held::test_census(rows);
        }
        pub fn lose_interval(&self) {
            // SAFETY: Test session holds `SERIAL`, owning the global market observer state.
            unsafe { LEDGER.lost_output(1) };
        }
        pub fn emit(&self, row: MarketEventRow) {
            append(row).unwrap();
        }
        pub fn create_contract(&self, memory: &impl Memory, pointer: usize) {
            contract_created(memory, 0, pointer, false).unwrap();
        }
        pub fn remove_contract(&self, memory: &impl Memory, pointer: usize) {
            assert!(OBSERVED_SITES.contains(&native::CONTRACT_REMOVAL_SITE));
            contract_removing(memory, 0, pointer, false).unwrap();
        }
        pub fn destroy_contract(
            &self,
            memory: &impl Memory,
            pointer: usize,
        ) -> Result<(), &'static str> {
            contract_removing(memory, 0, pointer, true)
        }
        pub fn reset_country(&self, memory: &impl Memory, owner: u32) {
            let _reset = RESET_WATCH.notify(country_receiver(owner));
            let _domains: [_; 3] =
                std::array::from_fn(|i| WATCHES[i].notify(country_receiver(owner)));
            country_reset_completed(memory, 0, owner).unwrap();
        }
        pub fn automation(&self, owner: u32) {
            drop(WATCHES[AUTOMATION].notify(country_receiver(owner)));
        }
        pub fn country_domain(&self, owner: u32, domain: usize) {
            drop(WATCHES[domain].notify(country_receiver(owner)));
        }
        pub fn country_domain_running(
            &self,
            domain: usize,
        ) -> crate::watch_registry::Running<'static> {
            WATCHES[domain].running()
        }
        pub fn fill_country_domain(&self, domain: usize) -> u32 {
            (10_000..=u32::MAX)
                .find(|&owner| WATCHES[domain].register(country_receiver(owner)).is_none())
                .unwrap()
        }
        pub fn offer_country_domain_growth(&self, domain: usize, countries: usize) {
            WATCHES.get(domain).unwrap_or(&RESET_WATCH).offer(countries);
        }
        pub fn defer_country_domain_retirement(&self, domain: usize) {
            WATCHES[domain].defer_retirement();
        }
        pub fn collect_country_domain(&self, domain: usize) {
            WATCHES.get(domain).unwrap_or(&RESET_WATCH).collect();
        }
        pub fn country_domain_capacity(&self, domain: usize) -> usize {
            WATCHES[domain].capacity()
        }
        pub fn inventory(&self, memory: &impl Memory, owner: u32) {
            let event = root(
                memory,
                0,
                lease::Identity {
                    giver: owner,
                    ..Default::default()
                },
                INVENTORY,
            )
            .unwrap();
            mark_holder(event, 0, INVENTORY_MARK, 0).unwrap();
        }
        pub fn contract_delivery(&self, memory: &impl Memory, pointer: usize) {
            let event = contract_root(memory, 0, pointer, CONTRACT_STATE).unwrap();
            mark_holder(event, pointer, CONTRACT_MARK, READ_PURCHASE_TOTAL).unwrap();
        }
    }
    impl Drop for Session {
        fn drop(&mut self) {
            let _ = &self.0;
            ENABLED.store(false, Ordering::SeqCst);
            observe_resets(false);
            observe_variables(false);
            held::invalidate();
            LEDGER.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn country_receiver_keys_cover_zero_and_the_full_native_id_domain() {
        let _session = unsafe { test_support::Session::new() };
        unsafe { boundary(
            Some(engine_native::cgame_state::Clock {
                hour: 1,
                ..Default::default()
            }),
            false,
        ) };
        for owner in [0, 1, 5000, u32::MAX] {
            let key = country_receiver(owner);
            assert_ne!(key, 0);
            assert_eq!(((key >> 3) - 1) as u32, owner);
            RESET_WATCH.register(key).unwrap();
            drop(RESET_WATCH.notify(key));
        }
        let mut owners = Vec::new();
        RESET_WATCH
            .take(|key| owners.push(((key >> 3) - 1) as u32))
            .unwrap();
        owners.sort_unstable();
        assert_eq!(owners, [0, 1, 5000, u32::MAX]);
    }
    #[test]
    fn contract_removal_copies_only_fixed_scalars_without_allocation() {
        let _session = unsafe { test_support::Session::new() };
        unsafe { LEDGER.boundary(Some(engine_native::cgame_state::Clock {
            hour: 1,
            ..Default::default()
        })) };
        assert!(
            unsafe { LEDGER.drain(&mut Vec::new()) }
                .unwrap_err()
                .contains("partial")
        );
        unsafe { LEDGER.boundary(Some(engine_native::cgame_state::Clock {
            hour: 2,
            ..Default::default()
        })) };
        const POINTER: usize = 0x100;
        struct FixedOperands {
            identity: Option<u64>,
            registered: Option<u8>,
            reads: std::cell::Cell<usize>,
        }
        impl Memory for FixedOperands {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                let value: u64 = if at == POINTER + engine_production::cpurchase_contract::ID_TYPE {
                    let Some(identity) = self.identity else {
                        return false;
                    };
                    identity
                } else if at == POINTER + engine_production::cpurchase_contract::REGISTERED {
                    let Some(registered) = self.registered else {
                        return false;
                    };
                    u64::from(registered)
                } else if at == POINTER + engine_production::cpurchase_contract::SELLER {
                    5000
                } else if at == POINTER + engine_production::cpurchase_contract::BUYER {
                    0
                } else if at == engine_native::cgame_state::SINGLETON_RVA.0 {
                    0x1000
                } else if at == 0x1000 + engine_native::cgame_state::CURRENT_HOUR {
                    2
                } else if at
                    == POINTER
                        + engine_production::market_ledger::DRAFT
                        + engine_production::market_ledger::DEFINITION_COST_VALID
                {
                    1
                } else if at == POINTER + engine_production::cpurchase_contract::COMPLETED {
                    3
                } else if at == POINTER + engine_production::cpurchase_contract::EQUIPMENT_PROGRESS
                {
                    50_000
                } else if at
                    == POINTER
                        + engine_production::market_ledger::DRAFT
                        + engine_production::market_ledger::DEFINITION_TOTAL_COST
                {
                    100_000
                } else {
                    panic!("removal callback walked a pool or definition");
                };
                out.copy_from_slice(&value.to_le_bytes()[..out.len()]);
                true
            }
        }
        let operands = |identity, registered| FixedOperands {
            identity,
            registered,
            reads: std::cell::Cell::new(0),
        };
        let identity = (17_u64 << 32) | u64::from(engine_production::cpurchase_contract::TYPE);
        let memory = operands(Some(identity), Some(1));
        crate::test_support::with_allocations_denied(|| {
            contract_removing(&memory, 0, POINTER, false)
        })
        .unwrap();
        assert_eq!(memory.reads.get(), 9);
        let retired = (u64::from(native::RETIRED_REFERENCE_ID) << 32) | 17;
        let memory = operands(Some(retired), Some(0));
        crate::test_support::with_allocations_denied(|| {
            contract_removing(&memory, 0, POINTER, true)
        })
        .unwrap();
        assert_eq!(memory.reads.get(), 2);

        let marker_identity = (u64::from(native::RETIRED_REFERENCE_ID) << 32)
            | u64::from(engine_production::cpurchase_contract::TYPE);
        let memory = operands(Some(marker_identity), Some(1));
        crate::test_support::with_allocations_denied(|| {
            contract_removing(&memory, 0, POINTER, true)
        })
        .unwrap();
        assert_eq!(memory.reads.get(), 10);
        let memory = operands(Some(marker_identity), Some(0));
        crate::test_support::with_allocations_denied(|| {
            contract_removing(&memory, 0, POINTER, true)
        })
        .unwrap();
        assert_eq!(memory.reads.get(), 2);

        for identity in [
            0,
            (17_u64 << 32) | u64::from(native::RETIRED_REFERENCE_ID),
            (17_u64 << 32) | u64::from(engine_production::cpurchase_contract::TYPE + 1),
            retired,
            marker_identity,
        ] {
            let memory = operands(Some(identity), Some(0));
            assert_eq!(
                crate::test_support::with_allocations_denied(|| {
                    contract_removing(&memory, 0, POINTER, false)
                }),
                Err("market removal contract identity invalid")
            );
        }
        for identity in [
            0,
            (17_u64 << 32) | u64::from(native::RETIRED_REFERENCE_ID),
            (17_u64 << 32) | u64::from(engine_production::cpurchase_contract::TYPE + 1),
        ] {
            let memory = operands(Some(identity), Some(0));
            assert_eq!(
                crate::test_support::with_allocations_denied(|| {
                    contract_removing(&memory, 0, POINTER, true)
                }),
                Err("market removal contract identity invalid")
            );
        }
        for destructor in [false, true] {
            let memory = operands(Some(retired), Some(1));
            assert_eq!(
                crate::test_support::with_allocations_denied(|| {
                    contract_removing(&memory, 0, POINTER, destructor)
                }),
                Err("market removal contract identity invalid")
            );
            for identity in [None, Some(retired)] {
                let memory = operands(identity, None);
                assert_eq!(
                    crate::test_support::with_allocations_denied(|| {
                        contract_removing(&memory, 0, POINTER, destructor)
                    }),
                    Err("market writer operand unreadable")
                );
            }
        }
        let mut events = Vec::new();
        assert_eq!(unsafe { LEDGER.drain(&mut events) }, Ok(4));
        assert_eq!(
            (events[0].kind, events[0].mode),
            (AMENDMENT_COST, FINAL_CONTRACT)
        );
        assert_eq!(events[1].kind, CONTRACT_REMOVED);
        assert_eq!(events[0].agreement, events[1].agreement);
        assert_eq!(events[0].agreement, identity);
        assert_eq!(events[2].agreement, marker_identity);
        assert_eq!(events[3].agreement, marker_identity);
        assert_eq!(
            (events[2].kind, events[2].mode),
            (AMENDMENT_COST, FINAL_CONTRACT)
        );
        assert_eq!(events[3].kind, CONTRACT_REMOVED);
        assert_eq!(
            (
                events[0].amount_raw,
                events[0].auxiliary_raw,
                events[0].ordinal
            ),
            (50_000, 100_000, 3)
        );
    }

    #[test]
    fn quiet_fuel_stores_do_no_guarded_reads_or_allocation() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("quiet fuel store read native memory");
            }
        }
        crate::test_support::with_allocations_denied(|| {
            for lost in [false, true] {
                fuel_store(&NoReads, usize::MAX, usize::MAX, 0, lost).unwrap();
            }
        });
    }
    #[test]
    fn market_amendment_stream_and_removal_witness_use_only_prepared_storage() {
        let _serial = test_support::SERIAL.lock().unwrap();
        use engine_native::{
            ccountry::synthetic::World,
            cequipment_variant::{self as variant, pool as pool},
        };
        struct Memory<'a>(&'a World);
        impl engine_native_memory::Memory for Memory<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == engine_native::cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                    output.copy_from_slice(&(self.0.game_state() as u64).to_le_bytes());
                    true
                } else {
                    self.0.memory().copy(address, output)
                }
            }
        }
        let mut world = World::new(0, 5000, 0x20);
        world.u32(
            world.game_state() + engine_native::cgame_state::CURRENT_HOUR,
            77,
        );
        let resources = world.alloc(0x30);
        world.u64(
            resources,
            engine_resources::ccountry_resources::VTABLE_RVA.0 as u64,
        );
        world.u32(
            resources + engine_resources::ccountry_resources::OWNER,
            5000,
        );
        let exchange = world.alloc(lease::SIZE);
        world.u64(exchange, lease::VTABLE_RVA.0 as u64);
        world.u64(exchange + lease::HANDLE, (17_u64 << 32) | 9001);
        world.u64(exchange + lease::GIVER_RESOURCES, resources as u64);
        world.u32(exchange + lease::RECEIVER, 4999);
        let model = world.alloc(16);
        world.u64(model, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        world.u64(model + 8, (31_u64 << 32) | variant::REFERENCE_TYPE as u64);
        for offset in [
            lease::MONTHLY_NEED,
            lease::PERCENTAGE_NEED,
            lease::ONCE_NEED,
        ] {
            let items = world.alloc(5000 * 16);
            for index in 0..5000 {
                world.u64(items + index * 16, model as u64);
                world.u64(items + index * 16 + 8, index as u64);
            }
            world.u64(exchange + offset, pool::VTABLE_RVA.0 as u64);
            world.array(exchange + offset + pool::ITEMS, items, 5000);
        }
        LEDGER.reset();
        LEDGER.prepare(30030);
        unsafe { LEDGER.boundary(Some(engine_native::cgame_state::Clock {
            hour: 76,
            ..Default::default()
        })) };
        assert_eq!(
            unsafe { LEDGER.drain(&mut Vec::new()) },
            Err("event ledger first/branch/recovery interval is partial")
        );
        unsafe { LEDGER.boundary(Some(engine_native::cgame_state::Clock {
            hour: 77,
            ..Default::default()
        })) };
        let memory = Memory(&world);
        crate::test_support::with_allocations_denied(|| {
            replacement(&memory, 0, exchange).unwrap();
            append(
                root(
                    &memory,
                    0,
                    lease::identity(&memory, 0, exchange).unwrap(),
                    REMOVED,
                )
                .unwrap(),
            )
            .unwrap();
        });
        assert_eq!(LEDGER.events.load(Ordering::Relaxed), 15003);
        assert_eq!(LEDGER.overflow.load(Ordering::Relaxed), 0);
        assert_eq!(LEDGER.lost_events_unknown.load(Ordering::Relaxed), 0);
        const CONTRACT: usize = 0x100;
        struct StackBirth;
        impl engine_native_memory::Memory for StackBirth {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                let value = if at == CONTRACT + engine_production::cpurchase_contract::ID_TYPE {
                    17_u64
                } else if at == CONTRACT + engine_production::cpurchase_contract::SELLER {
                    5000
                } else if at == CONTRACT + engine_production::cpurchase_contract::BUYER {
                    4999
                } else if at == engine_native::cgame_state::SINGLETON_RVA.0 {
                    0x1000
                } else if at == 0x1000 + engine_native::cgame_state::CURRENT_HOUR {
                    77
                } else {
                    panic!("birth callback read a definition");
                };
                output.copy_from_slice(&value.to_le_bytes()[..output.len()]);
                true
            }
        }
        crate::test_support::with_allocations_denied(|| {
            contract_created(&StackBirth, 0, CONTRACT, true).unwrap();
            contract_created(&StackBirth, 0, CONTRACT, false).unwrap();
        });
        let mut events = Vec::new();
        unsafe { LEDGER.drain(&mut events) }.unwrap();
        let births: Vec<_> = events
            .iter()
            .filter(|row| matches!(row.kind, REQUEST_CREATED | CONTRACT_CREATED))
            .collect();
        assert_eq!(births.len(), 2);
        assert!(
            births
                .iter()
                .all(|row| row.country == 5000 && row.receiver == 4999 && row.agreement == 17)
        );
        assert!(events.iter().all(|row| !(13..=18).contains(&row.kind)));
        struct Counted<'a> {
            memory: Memory<'a>,
            reads: std::cell::Cell<usize>,
        }
        impl engine_native_memory::Memory for Counted<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                self.memory.copy(at, output)
            }
        }
        let counted = Counted {
            memory: Memory(&world),
            reads: std::cell::Cell::new(0),
        };
        for destination in [lease::DELIVERED, lease::SUNK] {
            counted.reads.set(0);
            crate::test_support::with_allocations_denied(|| {
                equipment(
                    &counted,
                    0,
                    destination == lease::DELIVERED,
                    exchange + destination,
                    usize::MAX,
                )
                .unwrap()
            });
            assert!(
                counted.reads.get() <= 8,
                "transfer callback walked an operand pool"
            );
        }
        let stockpile = world.alloc(0x78);
        world.u32(stockpile + STOCKPILE_COUNTRY, 5000);
        let counted = Counted {
            memory: Memory(&world),
            reads: std::cell::Cell::new(0),
        };
        for site in 7..=10 {
            counted.reads.set(0);
            crate::test_support::with_allocations_denied(|| {
                inventory(&counted, 0, site, stockpile + STOCKPILE_POOL, usize::MAX, 0).unwrap()
            });
            assert_eq!(
                counted.reads.get(),
                3,
                "inventory callback read source/destination items"
            );
        }
        for offset in [lease::DELIVERED, lease::SUNK] {
            let items = world.alloc(5000 * 16);
            for index in 0..5000 {
                world.u64(items + index * 16, model as u64);
                world.u64(items + index * 16 + 8, index as u64);
            }
            world.u64(exchange + offset, pool::VTABLE_RVA.0 as u64);
            world.array(exchange + offset + pool::ITEMS, items, 5000);
        }
        crate::test_support::with_allocations_denied(|| {
            final_pools(&Memory(&world), 0, exchange).unwrap();
            append(
                root(
                    &Memory(&world),
                    0,
                    lease::identity(&Memory(&world), 0, exchange).unwrap(),
                    REMOVED,
                )
                .unwrap(),
            )
            .unwrap();
        });
        unsafe { LEDGER.drain(&mut events) }.unwrap();
        assert_eq!(
            events.iter().filter(|row| row.kind == FINAL_ITEM).count(),
            10_000
        );
        assert_eq!(
            events
                .iter()
                .filter(|row| row.kind == AMENDMENT_COST && row.mode == HOLDER_MARK)
                .count(),
            6
        );
        publish(&mut events).unwrap();
        assert!(
            events
                .iter()
                .all(|row| row.kind != AMENDMENT_COST && row.kind < FINAL_BEGIN)
        );
        assert!(events.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, REMOVED);
        let previous = events.last().unwrap().id;
        restore_sequence(Some(previous)).unwrap();
        events.clear();
        events.push(root(&StackBirth, 0, lease::Identity::default(), REQUEST_CREATED).unwrap());
        publish(&mut events).unwrap();
        assert!(events[0].id > previous);
        assert!(events[0].occurrence > previous);
    }
}
