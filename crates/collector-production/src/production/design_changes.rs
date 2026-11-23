use super::*;
#[macro_use]
mod diagnostics;
mod forward;
pub(super) mod held;
pub(super) mod liveness;
mod repair;
mod witness;
mod snapshots;
mod storage;
pub use snapshots::speculative::Part as SnapshotPart;
pub use storage::SetupPart as ColdSetupPart;
pub use storage::presize;
pub struct Speculation<'a> {
    pub lanes: usize,
    pub run: &'a mut dyn FnMut(Vec<SnapshotPart>) -> Result<Vec<SnapshotPart>, &'static str>,
}
mod source_hooks;
mod sources;
mod writers;
use engine_native::cequipment_variant as native;
pub use forward::{Availability, Forwarder, Phase, Writer, availability_is_current, forwarder};
use std::collections::HashMap;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const ROOT_BYTES: usize = native::DESIGN_TEAM_TRAITS + 4;
const STORAGE_FAILED: &str = "variant change storage allocation failed";
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
fn callback_store(store: &Mutex<Store>) -> std::sync::MutexGuard<'_, Store> {
    let guard = match store.try_lock() {
        Ok(store) => store,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
            #[cfg(test)]
            BEFORE_STORE_WAIT.with(|hook| {
                if let Some(hook) = hook.take() {
                    hook();
                }
            });
            let begin = std::time::Instant::now();
            let store = store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let ns = u64::try_from(begin.elapsed().as_nanos()).unwrap_or(u64::MAX);
            STORE_WAIT_NS.fetch_add(ns, Ordering::Relaxed);
            STORE_MAX_WAIT_NS.fetch_max(ns, Ordering::Relaxed);
            store
        }
    };
    STAGING_REVISION.fetch_add(1, Ordering::Relaxed);
    guard
}
static STAGING_REVISION: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
thread_local! {
    static BEFORE_STORE_WAIT: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
}
static MISSED: AtomicBool = AtomicBool::new(false);
static MISSED_AT: std::sync::atomic::AtomicPtr<std::panic::Location<'static>> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static LAST_MISSED_AT: std::sync::atomic::AtomicPtr<std::panic::Location<'static>> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static RUNNING_REFUSALS: AtomicU64 = AtomicU64::new(0);
static MISSED_REFUSALS: AtomicU64 = AtomicU64::new(0);
static COMPETING_WRITER_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static LOCK_CONTENTIONS: AtomicU64 = AtomicU64::new(0);
static STORE_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static STORE_MAX_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_RECEIVERS: AtomicU64 = AtomicU64::new(0);
static REPAIRED_INTERVALS: AtomicU64 = AtomicU64::new(0);
static REPAIRED_RECEIVERS: AtomicU64 = AtomicU64::new(0);
static CLAIM_CAPACITY_REFUSALS: AtomicU64 = AtomicU64::new(0);
#[track_caller]
fn missed_at() {
    MISSED_AT.store(
        std::ptr::from_ref(std::panic::Location::caller()).cast_mut(),
        Ordering::Release,
    );
    MISSED.store(true, Ordering::Release);
}
static CENSUS_REVISION: AtomicU64 = AtomicU64::new(1);
static CENSUS_BASELINE: AtomicU64 = AtomicU64::new(0);
static REFUSED_INTERVALS: AtomicU64 = AtomicU64::new(0);
static LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);
pub(super) fn refuse_census() {
    CENSUS_REVISION.fetch_add(1, Ordering::AcqRel);
}
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(1);
static SEED_REVISION: AtomicU64 = AtomicU64::new(0);
static SEED_EPOCH: AtomicU64 = AtomicU64::new(0);
static SAVED_POPULATION: AtomicUsize = AtomicUsize::new(0);
pub(super) fn begin_read() {
    SEED_REVISION.store(REVISION.load(Ordering::Acquire), Ordering::Release);
    SEED_EPOCH.store(EPOCH.load(Ordering::Acquire), Ordering::Release);
}

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static OWN_ENABLED: AtomicBool = AtomicBool::new(false);
static WORLD_ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(1);
static SAVED_BASELINE: AtomicBool = AtomicBool::new(false);
static SOURCE_BASELINE: AtomicBool = AtomicBool::new(false);
static LIVENESS_BASELINE: AtomicBool = AtomicBool::new(false);
fn observe_world_forwarder(enabled: bool) {
    if !enabled {
        refuse_census();
    }
    collector_observers::world_input_changes::observe_forwarder(enabled);
}
fn publish_availability() {
    let epoch = EPOCH.load(Ordering::Acquire);
    let token = forward::token();
    let observing = ENABLED.load(Ordering::Acquire);
    let saved = observing && SAVED_BASELINE.load(Ordering::Acquire);
    observe_world_forwarder(observing);
    forward::available_at(
        token,
        saved
            && SOURCE_BASELINE.load(Ordering::Acquire)
            && LIVENESS_BASELINE.load(Ordering::Acquire)
            && !RECOVER.load(Ordering::Acquire)
            && !MISSED.load(Ordering::Acquire),
    );
    if EPOCH.load(Ordering::Acquire) != epoch || !ENABLED.load(Ordering::Acquire) {
        observe_world_forwarder(false);
    }
}

fn withdraw_baselines() {
    SAVED_BASELINE.store(false, Ordering::Release);
    SOURCE_BASELINE.store(false, Ordering::Release);
    LIVENESS_BASELINE.store(false, Ordering::Release);
    forward::invalidate();
}

fn update_enabled() {
    collector_observers::variant_stat_witness::install(stat_timing_quiet);
    let enabled = OWN_ENABLED.load(Ordering::Acquire)
        && WORLD_ENABLED.load(Ordering::Acquire)
        && source_hooks::enabled()
        && liveness::enabled()
        && super::design_template_changes::enabled();
    let previous = ENABLED.swap(enabled, Ordering::AcqRel);
    if !enabled {
        if previous {
            boundary(true);
        }
        withdraw_baselines();
    }
    publish_availability();
}
pub(super) struct ProviderReset {
    invocation: Option<forward::Invocation<'static>>,
    complete: bool,
}
impl ProviderReset {
    /// # Safety
    /// The hooked original returned normally, on the native thread that began this reset.
    pub(super) unsafe fn finish(mut self) {
        self.complete = true;
        if let Some(invocation) = self.invocation.take() {
            unsafe { invocation.finish() };
        }
    }
}
impl Drop for ProviderReset {
    fn drop(&mut self) {
        REVISION.fetch_add(1, Ordering::AcqRel);
        if !self.complete {
            unobserved!();
        }
    }
}
/// # Safety
/// Only the provider reset detour calls this, on the native thread running it, with `pointer` that
/// call's live provider.
pub(super) unsafe fn provider_reset(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> ProviderReset {
    REVISION.fetch_add(1, Ordering::AcqRel);
    source_hooks::reset_provider(memory, base, pointer);
    ProviderReset {
        invocation: unsafe { forward::invalidation(true, pointer) },
        complete: false,
    }
}
fn world_enabled(enabled: bool) {
    WORLD_ENABLED.store(enabled, Ordering::Release);
    if !enabled {
        unobserved!();
    }
    update_enabled();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Design,
    Source,
    SourceRemoved,
    Provider(u32),
    ProviderReset(u32),
    Organisation(bool),
}
#[derive(Clone, Copy)]
struct Event {
    kind: Kind,
    sequence: u64,
    hour: i32,
    pointer: usize,
    variant: u64,
    removed: bool,
    start: usize,
    end: usize,
}
struct Batch {
    log: RawLog<1>,
    events: Vec<Event>,
    required_events: usize,
    failure: Option<&'static str>,
}
impl Batch {
    fn empty() -> Self {
        Self {
            log: RawLog::new(0, 0),
            events: Vec::new(),
            required_events: 0,
            failure: None,
        }
    }
    fn clear(&mut self) {
        self.log.clear();
        self.events.clear();
        self.failure = None;
        self.required_events = 0;
    }
    fn prepare(&mut self, events: usize) {
        if events > self.events.capacity() {
            let additional = events - self.events.len();
            let _ = engine_allocation::reserve(&mut self.events, additional);
        }
    }
    fn room(&mut self) -> Result<(), &'static str> {
        self.required_events = self.required_events.saturating_add(1);
        if self.events.len() == self.events.capacity() {
            engine_allocation::advisory(|| engine_allocation::reserve(&mut self.events, 1))
                .map_err(|_| STORAGE_FAILED)?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Watch {
    id: u64,
    pointer: usize,
    flags: [u8; 3],
    dirty: bool,
}
#[derive(Default)]
struct Published {
    pointers: Vec<usize>,
    overflowed: bool,
}
struct Store {
    batch: Batch,
    spare: Batch,
    watched: HashMap<usize, Watch>,
    published: Published,
    interests: sources::Interests,
    epoch: u64,
}
static RECOVER: AtomicBool = AtomicBool::new(true);
static RECOVERY_READ: AtomicBool = AtomicBool::new(true);
static STAT_WITHHELD_BY_LANE: [AtomicU64; native::STAT_LANE_COUNT] =
    [const { AtomicU64::new(0) }; native::STAT_LANE_COUNT];
static LAST_STAT_WITHHELD: Mutex<Option<(&'static str, &'static str)>> = Mutex::new(None);

pub(super) fn stat_timing_quiet() -> Result<native::QuietToken, &'static str> {
    if native::mutations::WRITER_AUDIT_REFUSAL.is_some() {
        return Err("variant stat writer audit failed");
    }
    if !enabled() {
        return Err("variant stat writer hooks disabled");
    }
    if active() != 0 {
        return Err("variant stat writer active");
    }
    let Some((world_epoch, world_revision)) =
        collector_observers::world_input_changes::design_quiet()
    else {
        return Err(
            if collector_observers::world_input_changes::design_forwarder_active() != 0 {
                "variant stat writer active"
            } else {
                "variant stat writer hooks disabled"
            },
        );
    };
    let epoch = EPOCH.load(Ordering::SeqCst);
    let revision = REVISION.load(Ordering::SeqCst);
    if active() != 0 {
        return Err("variant stat writer active");
    }
    if !enabled() {
        return Err("variant stat writer hooks disabled");
    }
    Ok(native::QuietToken {
        epoch,
        revision,
        world_epoch,
        world_revision,
    })
}

#[derive(Clone, Copy)]
struct StatWitnessSnapshot {
    total_withheld: u64,
    by_lane: [u64; native::STAT_LANE_COUNT],
    last_lane: Option<&'static str>,
    last_reason: Option<&'static str>,
}

fn stat_witness_snapshot() -> StatWitnessSnapshot {
    let mut total_withheld = 0_u64;
    let mut by_lane = [0_u64; native::STAT_LANE_COUNT];
    for (slot, counter) in by_lane.iter_mut().zip(&STAT_WITHHELD_BY_LANE) {
        let count = counter.load(Ordering::Relaxed);
        *slot = count;
        total_withheld = total_withheld.saturating_add(count);
    }
    let last = *LAST_STAT_WITHHELD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    StatWitnessSnapshot {
        total_withheld,
        by_lane,
        last_lane: last.map(|(lane, _)| lane),
        last_reason: last.map(|(_, reason)| reason),
    }
}

pub fn stat_witness_status() -> impl serde::Serialize {
    struct LaneCounts([u64; native::STAT_LANE_COUNT]);
    impl serde::Serialize for LaneCounts {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(native::STAT_LANE_COUNT))?;
            for (lane, count) in native::STAT_LANES.into_iter().zip(self.0) {
                map.serialize_entry(lane.name, &count)?;
            }
            map.end()
        }
    }
    #[derive(serde::Serialize)]
    struct StatWitnessStatus {
        total_withheld: u64,
        by_lane: LaneCounts,
        last_lane: Option<&'static str>,
        last_reason: Option<&'static str>,
    }
    let snapshot = stat_witness_snapshot();
    StatWitnessStatus {
        total_withheld: snapshot.total_withheld,
        by_lane: LaneCounts(snapshot.by_lane),
        last_lane: snapshot.last_lane,
        last_reason: snapshot.last_reason,
    }
}

pub(super) struct StatLaneWitness;

impl native::StatWitness for StatLaneWitness {
    fn quiet(&self) -> Result<native::QuietToken, &'static str> {
        stat_timing_quiet()
    }
    fn record_withheld(&mut self, lane: usize, reason: &'static str) {
        if let Some(counter) = STAT_WITHHELD_BY_LANE.get(lane) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(stat_lane) = native::STAT_LANES.get(lane) {
            *LAST_STAT_WITHHELD
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some((stat_lane.name, reason));
        }
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DrainProfile {
    pub events: u32,
    pub designs: u32,
    pub source_writes: u32,
    pub source_notes: u32,
    pub events_ns: u64,
    pub reads_ns: u64,
    pub unchanged: u32,
    pub lane_walks: u32,
    pub complete_ns: [u64; 2],
    pub stages_ns: [u64; 3],
    pub boundary_ns: [u64; 2],
    pub lanes: u32,
    pub lane_ns: [u64; 2],
}
static DRAIN_PROFILE: Mutex<DrainProfile> = Mutex::new(DrainProfile {
    events: 0,
    designs: 0,
    source_writes: 0,
    source_notes: 0,
    events_ns: 0,
    reads_ns: 0,
    unchanged: 0,
    lane_walks: 0,
    complete_ns: [0; 2],
    stages_ns: [0; 3],
    boundary_ns: [0; 2],
    lanes: 0,
    lane_ns: [0; 2],
});
fn drain_profile_mut() -> std::sync::MutexGuard<'static, DrainProfile> {
    DRAIN_PROFILE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub fn drain_profile() -> DrainProfile {
    std::mem::take(&mut *drain_profile_mut())
}
fn elapsed_ns(begin: std::time::Instant) -> u64 {
    u64::try_from(begin.elapsed().as_nanos()).unwrap_or(u64::MAX)
}
pub fn needs_full() -> bool {
    enabled() && RECOVERY_READ.load(Ordering::Acquire)
}
pub(super) fn needs_baseline() -> bool {
    !SAVED_BASELINE.load(Ordering::Acquire)
        || !SOURCE_BASELINE.load(Ordering::Acquire)
        || !LIVENESS_BASELINE.load(Ordering::Acquire)
        || RECOVER.load(Ordering::Acquire)
}
pub(super) fn begin_full() {
    RECOVERY_READ.store(false, Ordering::Release);
}
pub(super) fn baseline_admitted() -> bool {
    enabled()
        && !needs_baseline()
        && !MISSED.load(Ordering::Acquire)
        && active() == 0
        && SEED_REVISION.load(Ordering::Acquire) == REVISION.load(Ordering::Acquire)
        && SEED_EPOCH.load(Ordering::Acquire) == EPOCH.load(Ordering::Acquire)
}
pub fn boundary(invalidate: bool) {
    if invalidate {
        observe_world_forwarder(false);
        EPOCH.fetch_add(1, Ordering::AcqRel);
        withdraw_baselines();
        super::design_context::invalidate();
        RECOVER.store(true, Ordering::Release);
        RECOVERY_READ.store(true, Ordering::Release);
        unobserved!();
        if let Some(store) = STORE.get() {
            if let Ok(mut store) = store.try_lock() {
                store.batch.clear();
            }
        }
    }
}

fn u64_at(log: &RawLog<1>, offset: usize) -> u64 {
    u64::from_le_bytes(log.bytes[offset..offset + 8].try_into().unwrap())
}
fn take(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    pointer: usize,
    size: usize,
) -> Result<usize, &'static str> {
    log.take_bulk(memory, pointer, size, 1)
        .ok_or(if log.overflowed {
            "variant change raw storage exhausted"
        } else {
            "variant change raw memory unreadable"
        })
}
fn string(log: &mut RawLog<1>, memory: &impl Memory, header: usize) -> Result<(), &'static str> {
    let length = usize::try_from(u64_at(log, header + 16))
        .map_err(|_| "variant change string length overflow")?;
    let capacity = u64_at(log, header + 24);
    if length as u64 > capacity || capacity < 15 {
        return Err("variant change string header invalid");
    }
    if capacity > 15 {
        let pointer = u64_at(log, header) as usize;
        take(
            log,
            memory,
            pointer,
            length
                .checked_add(1)
                .ok_or("variant change string length overflow")?,
        )?;
    }
    Ok(())
}
fn array(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    header: usize,
    stride: usize,
) -> Result<(usize, u32), &'static str> {
    let source = Array::from_header(log.bytes[header..header + 16].try_into().unwrap(), stride)
        .ok_or("variant change array unreadable or invalid")?;
    let size = (source.count as usize)
        .checked_mul(stride)
        .ok_or("variant change array size overflow")?;
    let start = take(log, memory, source.data, size)?;
    Ok((start, source.count))
}

#[cfg(test)]
pub(super) fn prepare(registry: &[native::RegistryRow], variants: &Variants) {
    prepare_with(registry, variants, presize);
}
pub(super) fn prepare_partitioned(
    registry: &[native::RegistryRow],
    variants: &Variants,
    acquire: &mut impl FnMut(Vec<storage::SetupPart>) -> Result<Vec<storage::SetupPart>, &'static str>,
) {
    prepare_with(registry, variants, || {
        storage::presize_partitioned(acquire)
    });
}
struct ColdDemand {
    events: usize,
}
impl ColdDemand {
    fn record(&self) {
        storage::census(self.events);
    }
}
fn cold_demand(registry: &[native::RegistryRow]) -> ColdDemand {
    ColdDemand {
        events: registry.len().saturating_mul(2).max(1),
    }
}
fn cold_store() -> &'static Mutex<Store> {
    STORE.get_or_init(|| {
        Mutex::new(Store {
            batch: Batch::empty(),
            spare: Batch::empty(),
            watched: HashMap::new(),
            published: Published::default(),
            interests: sources::Interests::default(),
            epoch: EPOCH.load(Ordering::Acquire),
        })
    })
}

pub struct ColdStorage(ColdDemand);
impl ColdStorage {
    pub fn capture(
        &mut self,
        acquire: &mut impl FnMut(
            Vec<storage::SetupPart>,
        ) -> Result<Vec<storage::SetupPart>, &'static str>,
    ) {
        cold_store();
        self.0.record();
        storage::presize_partitioned(acquire);
    }
}
pub(super) fn cold_storage(registry: &[native::RegistryRow]) -> Option<ColdStorage> {
    let cold = STORE.get().is_none_or(|store| {
        store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .batch
            .events
            .capacity()
            == 0
    });
    cold.then(|| ColdStorage(cold_demand(registry)))
}

fn prepare_with(registry: &[native::RegistryRow], variants: &Variants, sizing: impl FnOnce()) {
    let demand = cold_demand(registry);
    let store = cold_store();
    demand.record();
    let epoch = EPOCH.load(Ordering::Acquire);
    let revision = SEED_REVISION.load(Ordering::Acquire);
    let (old_epoch, organisations, pointers, cold) = {
        let store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !RECOVER.load(Ordering::Acquire)
            && store.epoch == epoch
            && !store.batch.events.is_empty()
        {
            return;
        }
        (
            store.epoch,
            store.interests.organisations.len(),
            store.interests.organisation_pointers.len(),
            store.batch.events.capacity() == 0,
        )
    };
    let mut attempt = BaselineAttempt { accepted: false };
    if SEED_EPOCH.load(Ordering::Acquire) != epoch
        || REVISION.load(Ordering::Acquire) != revision
        || active() != 0
    {
        unobserved!();
        return;
    }
    if cold {
        sizing();
    }
    let prepared = (|| {
        if RECOVER.load(Ordering::Acquire) {
            witness::release();
        }
        writers::prepare(registry.len().saturating_mul(2).max(1))?;
        let mut identities = HashMap::new();
        let mut watched = HashMap::new();
        engine_allocation::reserve_map(&mut identities, registry.len())
            .map_err(|_| "variant change identity allocation failed")?;
        engine_allocation::reserve_map(&mut watched, registry.len().saturating_mul(2).max(1))
            .map_err(|_| "variant change watch allocation failed")?;
        let mut published = Published::default();
        engine_allocation::reserve(&mut published.pointers, registry.len().max(1))
            .map_err(|_| "variant change watch allocation failed")?;
        for (source, definition) in registry.iter().zip(&variants.definitions.rows) {
            if identities.insert(source.id, source.pointer()).is_some()
                || watched
                    .insert(
                        source.pointer(),
                        Watch {
                            id: source.id,
                            pointer: source.pointer(),
                            dirty: false,
                            flags: [
                                definition.design.can_upgrade_type,
                                definition.design.can_upgrade_variant,
                                definition.design.can_upgrade_modules,
                            ],
                        },
                    )
                    .is_some()
            {
                return Err("variant change census contains duplicate identity or address");
            }
        }
        let mut interests = sources::Interests::default();
        if old_epoch == epoch {
            interests.prepare_organisations(organisations.max(pointers))?;
            let store = store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if store.interests.organisations.len() > interests.organisations.capacity()
                || store.interests.organisation_pointers.len()
                    > interests.organisation_pointers.capacity()
            {
                return Err("variant organisation demand changed during census");
            }
            interests.organisations.extend(
                store
                    .interests
                    .organisations
                    .iter()
                    .map(|(key, value)| (*key, *value)),
            );
            interests.organisation_pointers.extend(
                store
                    .interests
                    .organisation_pointers
                    .iter()
                    .map(|(key, value)| (*key, *value)),
            );
        } else {
            sources::release();
            liveness::release();
            held::release();
            witness::release();
        }
        repair::prepare(&watched)?;
        held::seed(registry, variants, &watched)?;
        liveness::seed(variants, &mut interests)?;
        sources::seed(variants, &mut interests)?;
        Ok((watched, published, interests))
    })();
    let (mut watched, mut published, mut interests) = match prepared {
        Ok(prepared) => prepared,
        Err(reason) => {
            store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .batch
                .failure = Some(reason);
            return;
        }
    };
    {
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        STAGING_REVISION.fetch_add(1, Ordering::Relaxed);
        if active() != 0
            || epoch != EPOCH.load(Ordering::Acquire)
            || REVISION.load(Ordering::Acquire) != revision
        {
            unobserved!();
            return;
        }
        if store.epoch != epoch {
            store.batch.clear();
        }
        std::mem::swap(&mut store.watched, &mut watched);
        std::mem::swap(&mut store.published, &mut published);
        std::mem::swap(&mut store.interests, &mut interests);
        store.epoch = epoch;
        SAVED_POPULATION.store(store.watched.len(), Ordering::Release);
        SAVED_BASELINE.store(true, Ordering::Release);
        SOURCE_BASELINE.store(true, Ordering::Release);
        LIVENESS_BASELINE.store(true, Ordering::Release);
        RECOVER.store(false, Ordering::Release);
        MISSED.store(false, Ordering::Release);
        CENSUS_BASELINE.store(CENSUS_REVISION.load(Ordering::Acquire), Ordering::Release);
        diagnostics::clear();
    }
    if REVISION.load(Ordering::Acquire) != revision
        || active() != 0
        || EPOCH.load(Ordering::Acquire) != epoch
    {
        unobserved!();
        withdraw_baselines();
    } else {
        attempt.accepted = true;
    }
    publish_availability();
}

struct BaselineAttempt {
    accepted: bool,
}
impl Drop for BaselineAttempt {
    fn drop(&mut self) {
        if !self.accepted {
            withdraw_baselines();
            RECOVER.store(true, Ordering::Release);
            unobserved!();
        }
    }
}

fn mark_design(memory: &impl Memory, base: usize, pointer: usize, published: bool) {
    let Some(mutex) = STORE.get() else {
        unobserved!();
        return;
    };
    let mut store = callback_store(mutex);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    if !store.watched.contains_key(&pointer) {
        if !published {
            return;
        }
        let identity = match engine_native_parts::equipment_variant_identity::identity(memory, base, pointer) {
            Ok(identity) => identity,
            Err(reason) => {
                store.batch.failure = Some(reason);
                return;
            }
        };
        if store.watched.len() == store.watched.capacity()
            && engine_allocation::advisory(|| engine_allocation::reserve_map(&mut store.watched, 1))
                .is_err()
        {
            store.batch.failure = Some(STORAGE_FAILED);
            return;
        }
        repair::added();
        let published = &mut store.published;
        if published.pointers.len() < published.pointers.capacity()
            || engine_allocation::advisory(|| engine_allocation::reserve(&mut published.pointers, 1))
                .is_ok()
        {
            published.pointers.push(pointer);
        } else {
            published.overflowed = true;
        }
        store.watched.insert(
            pointer,
            Watch {
                id: identity,
                pointer,
                flags: [0; 3],
                dirty: true,
            },
        );
        SAVED_POPULATION.fetch_max(store.watched.len(), Ordering::AcqRel);
    }
    drop(store);
    repair::mark(pointer);
}

fn unrecorded_writer(receiver: Receiver, pointer: usize, moved_source: Option<usize>) {
    if pointer == 0 || matches!(receiver, Receiver::Roster | Receiver::CreateCommand) {
        unknown_receiver!();
        return;
    }
    if matches!(receiver, Receiver::Published) {
        unknown_receiver!();
    } else if moved_source.is_some() || matches!(receiver, Receiver::Removed) {
        let Some(store) = STORE.get() else {
            unknown_receiver!();
            return;
        };
        let store = callback_store(store);
        if store.watched.contains_key(&pointer)
            || moved_source.is_some_and(|source| store.watched.contains_key(&source))
        {
            missed!();
        }
    }
    dirty_receiver(pointer);
    if let Some(source) = moved_source {
        dirty_receiver(source);
    }
}
fn dirty_receiver(pointer: usize) {
    repair::mark(pointer);
}

fn retire(memory: &impl Memory, base: usize, pointer: usize) -> bool {
    let Some(store) = STORE.get() else {
        return false;
    };
    let mut store = callback_store(store);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return false;
    }
    let watched = store.watched.contains_key(&pointer);
    if watched {
        retire_from(&mut store, memory, base, pointer);
    }
    watched
}
fn retire_from(store: &mut Store, memory: &impl Memory, base: usize, pointer: usize) {
    let Some(watch) = store.watched.remove(&pointer) else {
        return;
    };
    if let Err(reason) = store.batch.room() {
        store.batch.failure = Some(reason);
        return;
    }
    let hour = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|address| engine_native_memory::read::<u64>(memory, address))
        .and_then(|state| (state as usize).checked_add(engine_native::cgame_state::CURRENT_HOUR))
        .and_then(|address| engine_native_memory::read::<i32>(memory, address));
    let Some(hour) = hour else {
        store.batch.failure = Some("variant removal hour unreadable");
        return;
    };
    store.batch.events.push(Event {
        kind: Kind::Design,
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        hour,
        pointer,
        variant: watch.id,
        removed: true,
        start: 0,
        end: 0,
    });
}

pub fn pending() -> bool {
    needs_full()
        || !enabled()
        || MISSED.load(Ordering::Acquire)
        || repair::pending()
        || STORE.get().is_some_and(|s| {
            s.try_lock().map_or(true, |s| {
                !s.batch.events.is_empty() || s.batch.failure.is_some()
            })
        })
}
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
        + super::design_template_changes::active()
        + collector_observers::world_input_changes::design_forwarder_active()
}

use collector_hooks::hook::Hook;
use native::mutations::{FORWARDED_SITES, MUTATIONS, Receiver};
use std::{cell::Cell, ops::Range};
static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; MUTATIONS.len()] = [const { AtomicUsize::new(0) }; MUTATIONS.len()];
thread_local! {
    static TARGET: Cell<usize> = const { Cell::new(0) };
    static CREATING: Cell<bool> = const { Cell::new(false) };
    static CREATED: Cell<usize> = const { Cell::new(0) };
}

struct CallbackScope {
    claims: Option<writers::Flight>,
    previous_target: usize,
    creation: Option<(bool, usize)>,
    completed: bool,
}
impl Drop for CallbackScope {
    fn drop(&mut self) {
        REVISION.fetch_add(1, Ordering::AcqRel);
        if !self.completed {
            unobserved!();
        }
        if let Some((creating, created)) = self.creation {
            CREATING.set(creating);
            CREATED.set(created);
        }
        TARGET.set(self.previous_target);
        drop(self.claims.take());
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

const HOOK_ERRORS: [&str; 23] = [
    "saved-design RebuildStats hook construction failed",
    "saved-design RefreshDerivedState hook construction failed",
    "saved-design RefreshOrganisation hook construction failed",
    "saved-design RefreshOrganisationBonus hook construction failed",
    "saved-design SetObsolete hook construction failed",
    "saved-design PropagateObsolete hook construction failed",
    "saved-design SetHighlight hook construction failed",
    "saved-design EnableUpgradeVariant hook construction failed",
    "saved-design RefreshNiches hook construction failed",
    "saved-design MoveAssign hook construction failed",
    "saved-design Publish hook construction failed",
    "saved-design Destroy hook construction failed",
    "saved-design Rename hook construction failed",
    "saved-design SetAutoUpgraded hook construction failed",
    "saved-design SetNamesGroup hook construction failed",
    "saved-design SetRoleIcon hook construction failed",
    "saved-design SetModel hook construction failed",
    "saved-design SetIcon hook construction failed",
    "saved-design Update hook construction failed",
    "saved-design BulkUpgradeModules hook construction failed",
    "saved-design BulkUpgradeTypes hook construction failed",
    "saved-design BulkUpgradeVariants hook construction failed",
    "saved-design CreateCommand hook construction failed",
];
const PUBLICATION_ERROR: &str = "saved-design hook publication failed (sites: RebuildStats, RefreshDerivedState, RefreshOrganisation, RefreshOrganisationBonus, SetObsolete, PropagateObsolete, SetHighlight, EnableUpgradeVariant, RefreshNiches, MoveAssign, Publish, Destroy, Rename, SetAutoUpgraded, SetNamesGroup, SetRoleIcon, SetModel, SetIcon, Update, BulkUpgradeModules, BulkUpgradeTypes, BulkUpgradeVariants, CreateCommand)";
pub struct Hooks {
    hooks: Vec<Hook>,
    templates: super::design_template_changes::Hooks,
    sources: source_hooks::Hooks,
    liveness: liveness::Hooks,
}
impl Hooks {
    /// # Safety
    /// The caller verified this process's executable identity and exclusively
    /// owns these sites, and must quiesce every callback before unloading. The owner stays live,
    /// and drops these hooks, only once patch restoration and callback quiescence are proved.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        BASE.store(base, Ordering::Release);
        let mut hooks = Vec::new();
        if native::mutations::WRITER_AUDIT_REFUSAL.is_none()
            && MUTATIONS.iter().all(|site| {
                engine_sites::hook(base, site.rva)
                    .and_then(|code| code.sentinel::<16>())
                    .is_ok()
            })
        {
            for (index, site) in MUTATIONS.iter().enumerate() {
                if FORWARDED_SITES.contains(&index) {
                    continue;
                }
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks()[index], code.sentinel::<16>()?) }
                        .map_err(|_| HOOK_ERRORS[index])?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        let templates = unsafe { super::design_template_changes::Hooks::new(memory, base) }?;
        collector_observers::world_input_changes::forwarder(Some(
            collector_observers::world_input_changes::Forwarder {
                call: forwarded,
                enabled: world_enabled,
            },
        ));
        let sources = unsafe { source_hooks::Hooks::new(memory, base) }?;
        let liveness = unsafe { liveness::Hooks::new(memory, base) }?;
        Ok(Self {
            hooks,
            templates,
            sources,
            liveness,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            OWN_ENABLED.store(false, Ordering::Release);
            update_enabled();
        }
        if !self.liveness.set_enabled(enabled)? {
            return Ok(false);
        }
        if !self.sources.set_enabled(enabled)? {
            return Ok(false);
        }
        if !self.templates.set_enabled(enabled)? {
            return Ok(false);
        }
        let changed = Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| PUBLICATION_ERROR)?;
        if changed {
            OWN_ENABLED.store(
                enabled && self.hooks.len() == MUTATIONS.len() - FORWARDED_SITES.len(),
                Ordering::Release,
            );
            update_enabled();
        }
        Ok(changed)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.templates.executable_ranges())
            .chain(self.sources.executable_ranges())
            .chain(self.liveness.executable_ranges())
    }
}

/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn forwarded(
    site: collector_observers::world_input_changes::Forward,
    call: collector_observers::world_input_changes::ForwardCall,
    original: collector_observers::world_input_changes::Original,
) -> usize {
    let args = call.args;
    use collector_observers::world_input_changes::Forward;
    if !enabled() {
        return unsafe { original(args[0], args[1], args[2], args[3]) };
    }
    match site {
        Forward::DesignOrganisation => unsafe { around::<2>(args, original) },
        Forward::DesignBonuses => unsafe { around::<3>(args, original) },
        Forward::DesignMove => unsafe {
            around_with_identities::<9>(args, original, Some(call.before))
        },
        Forward::DesignClone => unsafe { original(args[0], args[1], args[2], args[3]) },
    }
}

fn target(
    memory: &impl Memory,
    base: usize,
    receiver: Receiver,
    first: usize,
    second: usize,
) -> usize {
    match receiver {
        Receiver::Variant | Receiver::Removed => first,
        Receiver::Published => second,
        Receiver::Command => {
            let Some(handle) = first
                .checked_add(native::mutations::COMMAND_VARIANT)
                .and_then(|p| engine_native_memory::read::<u64>(memory, p))
            else {
                unknown_receiver!();
                return 0;
            };
            match engine_native_parts::creference_registry::resolve(memory, base, handle) {
                Ok(pointer) => pointer.unwrap_or(0),
                Err(_) => {
                    unknown_receiver!();
                    0
                }
            }
        }
        Receiver::Roster | Receiver::CreateCommand => 0,
    }
}

fn version_owner(
    memory: &impl Memory,
    mut pointer: usize,
    population: usize,
) -> Result<usize, &'static str> {
    let origin = |pointer: usize| {
        pointer
            .checked_add(native::ORIGIN_LINK)
            .and_then(|p| engine_native_memory::read::<i32>(memory, p))
            .ok_or("variant version origin unreadable")
    };
    let licensed = origin(pointer)? > 0;
    for _ in 0..=population {
        let parent = pointer
            .checked_add(native::PARENT)
            .and_then(|p| engine_native_memory::read::<u64>(memory, p))
            .ok_or("variant version ancestry unreadable")? as usize;
        if parent == 0 {
            return if licensed {
                Err("licensed variant has no version ancestry")
            } else {
                Ok(pointer)
            };
        }
        if licensed && origin(parent)? <= 0 {
            return Ok(pointer);
        }
        pointer = parent;
    }
    Err("variant version ancestry exceeds the saved population")
}

fn returned(memory: &impl Memory, base: usize, site: usize, pointer: usize, first: usize) {
    let mutation = &MUTATIONS[site];
    match mutation.receiver {
        Receiver::Published if CREATING.get() => {
            if CREATED.get() != 0 && CREATED.get() != pointer {
                unobserved!();
            }
            CREATED.set(pointer);
        }
        Receiver::Roster => {
            let Some(array) = Array::read(memory, first, 8) else {
                unobserved!();
                return;
            };
            for ordinal in 0..array.count {
                let Some(pointer) = array
                    .element(ordinal, 8)
                    .and_then(|p| engine_native_memory::read::<u64>(memory, p))
                else {
                    unobserved!();
                    return;
                };
                mark_design(memory, base, pointer as usize, false);
            }
        }
        Receiver::Removed => {}
        _ if pointer != 0 => mark_design(
            memory,
            base,
            pointer,
            matches!(mutation.receiver, Receiver::Published),
        ),
        _ => {}
    }
}

/// # Safety
/// The arguments are ones `SITE`'s published original accepts with its ABI, and every object they
/// reach stays valid for the reads its site makes; one the original destroys is an identity only
/// once it returns.
unsafe fn callback<const SITE: usize>(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let original = unsafe {
        std::mem::transmute::<usize, collector_observers::world_input_changes::Original>(
            ORIGINAL[SITE].load(Ordering::Acquire),
        )
    };
    unsafe { around::<SITE>([first, second, third, fourth], original) }
}

/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn around<const SITE: usize>(
    args: [usize; 4],
    original: collector_observers::world_input_changes::Original,
) -> usize {
    unsafe { around_with_identities::<SITE>(args, original, None) }
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn around_with_identities<const SITE: usize>(
    [first, second, third, fourth]: [usize; 4],
    original: collector_observers::world_input_changes::Original,
    identities: Option<[Option<u64>; 2]>,
) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::VariantChanges);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    REVISION.fetch_add(1, Ordering::AcqRel);
    let base = BASE.load(Ordering::Acquire);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let pointer = target(&memory, base, MUTATIONS[SITE].receiver, first, second);
    if pointer == 0
        && !matches!(
            MUTATIONS[SITE].receiver,
            Receiver::Roster | Receiver::CreateCommand
        )
    {
        unknown_receiver!();
    }
    let moving = MUTATIONS[SITE].name == "MoveAssign";
    let footprint = if SITE == 1 {
        writers::Footprint::Ancestry
    } else if moving || matches!(MUTATIONS[SITE].receiver, Receiver::Removed) {
        writers::Footprint::Lifetime
    } else {
        writers::Footprint::Design
    };
    let mut claims = writers::enter(pointer, moving.then_some(second), footprint);
    let version = if SITE == 1 && pointer != 0 {
        match version_owner(&memory, pointer, SAVED_POPULATION.load(Ordering::Acquire)) {
            Ok(owner) => {
                if owner != pointer
                    && let Some(claims) = claims.as_mut()
                {
                    claims.extend(owner);
                }
                Some(owner)
            }
            Err(_) => {
                unknown_receiver!();
                None
            }
        }
    } else {
        None
    };
    let admitted = claims.as_ref().is_some_and(writers::Flight::admitted);
    if !admitted {
        unrecorded_writer(
            MUTATIONS[SITE].receiver,
            pointer,
            (MUTATIONS[SITE].name == "MoveAssign").then_some(second),
        );
    }
    let previous = TARGET.replace(pointer);
    let create_scope = admitted && matches!(MUTATIONS[SITE].receiver, Receiver::CreateCommand);
    let previous_creating = if create_scope {
        CREATING.replace(true)
    } else {
        false
    };
    let previous_created = if create_scope { CREATED.replace(0) } else { 0 };
    let mut scope = CallbackScope {
        claims,
        previous_target: previous,
        creation: create_scope.then_some((previous_creating, previous_created)),
        completed: false,
    };
    let selected = admitted && (pointer == 0 || pointer != previous);
    let retired = selected
        && matches!(MUTATIONS[SITE].receiver, Receiver::Removed)
        && pointer != 0
        && retire(&memory, base, pointer);
    let move_bindings = if admitted && moving {
        STORE.get().map(|store| {
            let store = callback_store(store);
            [
                store.watched.get(&first).copied(),
                store.watched.get(&second).copied(),
            ]
        })
    } else {
        None
    };
    if admitted && moving && move_bindings.is_none() {
        unobserved!();
    }
    if let (Some(bindings), Some(identities)) = (move_bindings, identities) {
        for (binding, identity) in bindings.into_iter().zip(identities) {
            if binding.is_some_and(|binding| Some(binding.id) != identity) {
                unobserved!();
            }
        }
    }
    if let Some(bindings) = move_bindings {
        for binding in bindings.into_iter().flatten() {
            retire(&memory, base, binding.pointer);
        }
    }
    let before = MUTATIONS[SITE]
        .witness
        .and_then(|offset| pointer.checked_add(offset))
        .and_then(|p| engine_native_memory::read::<u16>(&memory, p));
    if MUTATIONS[SITE].witness.is_some() && before.is_none() {
        unobserved!();
    }
    let forwarded = unsafe { forward::begin(SITE, pointer) };
    let moved_source = if moving && first != second {
        unsafe { forward::begin(SITE, second) }
    } else {
        None
    };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    if !admitted {
        unrecorded_writer(MUTATIONS[SITE].receiver, pointer, moving.then_some(second));
    }
    if let Some(moved_source) = moved_source {
        unsafe { moved_source.finish() };
    }
    if let Some(forwarded) = forwarded {
        unsafe { forwarded.finish() };
    }
    let changed = !matches!(MUTATIONS[SITE].receiver, Receiver::Removed)
        && MUTATIONS[SITE].witness.is_none_or(|offset| {
            let after = pointer
                .checked_add(offset)
                .and_then(|p| engine_native_memory::read::<u16>(&memory, p));
            if after.is_none() {
                unobserved!();
            }
            after != before
        });
    if let Some(bindings) = move_bindings {
        if bindings.iter().any(Option::is_some) {
            mark_design(&memory, base, first, true);
        }
    } else if selected && changed {
        returned(&memory, base, SITE, pointer, first);
    }
    if SITE == 1 && pointer != 0 {
        match version_owner(&memory, pointer, SAVED_POPULATION.load(Ordering::Acquire)) {
            Ok(owner) => {
                if Some(owner) != version {
                    unknown_receiver!();
                } else if owner != pointer {
                    if admitted {
                        mark_design(&memory, base, owner, false);
                    } else {
                        dirty_receiver(owner);
                    }
                }
            }
            Err(_) => unknown_receiver!(),
        }
    }
    if create_scope {
        let created = CREATED.get();
        if created != 0 {
            mark_design(&memory, base, created, true);
        }
    }
    let (conflicted, dependency) = scope
        .claims
        .take()
        .map_or((false, false), writers::Flight::finish_with_dependency);
    if SITE == 1
        && (!admitted || conflicted)
        && (dependency || version.is_none_or(|owner| owner != pointer))
    {
        unobserved!();
    }
    if admitted && conflicted {
        if retired {
            missed!();
        }
        unrecorded_writer(MUTATIONS[SITE].receiver, pointer, moving.then_some(second));
        if let Some(owner) = version {
            dirty_receiver(owner);
        }
    }
    scope.completed = true;
    result
}

fn replace<S: rows::TableSchema>(
    projection: Option<&Projection<'_>>,
    edits: Vec<(usize, S::Row)>,
) -> Result<Option<TableInput>, &'static str> {
    let Some((token, baseline)) =
        projection.and_then(Projection::observed_comparison_baseline::<S>)
    else {
        return Ok(None);
    };
    if edits
        .iter()
        .all(|(index, row)| baseline.get(*index).is_some_and(|old| old == row))
    {
        return Ok(Some(
            match projection.and_then(Projection::observed_shared_baseline::<S>) {
                Some(rows) => TableInput::SharedComplete(std::sync::Arc::clone(rows)),
                None => TableInput::ComparedEqual(token),
            },
        ));
    }
    if !token.allows_row_updates() {
        return Ok(None);
    }
    let mut indices =
        engine_allocation::vec(edits.len()).map_err(|_| "variant change allocation failed")?;
    let mut output =
        engine_allocation::vec(edits.len()).map_err(|_| "variant change allocation failed")?;
    for (index, row) in edits {
        if baseline.get(index).is_some_and(|old| old == &row) {
            continue;
        }
        indices.push(index);
        output.push(row);
    }
    Ok(Some(TableInput::ComparedChanges {
        token,
        indices,
        rows: S::wrap(output),
    }))
}

fn edit<R>(edits: &mut Vec<(usize, R)>, index: usize, row: R) -> Result<(), &'static str> {
    match edits.binary_search_by_key(&index, |(index, _)| *index) {
        Ok(position) => edits[position].1 = row,
        Err(position) => {
            engine_allocation::reserve(edits, 1).map_err(|_| "variant change allocation failed")?;
            edits.insert(position, (index, row));
        }
    }
    Ok(())
}

fn sparse<S: rows::TableSchema>(
    history: snapshots::History<'_>,
    projection: Option<&Projection<'_>>,
    owner: impl Fn(&S::Row) -> u64,
    make: impl Fn(&rows::VariantSnapshot) -> std::io::Result<Vec<S::Row>>,
) -> Result<Option<TableInput>, &'static str> {
    let Some(baseline) = projection.and_then(Projection::observed_baseline::<S>) else {
        return Ok(None);
    };
    if projection
        .and_then(Projection::observed_comparison_baseline::<S>)
        .is_none()
    {
        return Ok(None);
    }
    let mut edits = Vec::new();
    for change in history.iter() {
        let event = change?.view();
        if event.organisation.is_some()
            || event.flags.is_some()
            || event.scalar.is_some()
            || event.sources.is_some()
        {
            continue;
        }
        if event.removed {
            return Ok(None);
        }
        let snapshot = event
            .snapshot
            .as_ref()
            .ok_or("variant changes have an incomplete observation")?;
        let start = baseline.partition_point(|row| owner(row) < event.variant);
        let end = start + baseline[start..].partition_point(|row| owner(row) == event.variant);
        let fresh = make(snapshot).map_err(|_| "variant change allocation failed")?;
        if fresh.len() != end - start
            || !fresh
                .iter()
                .zip(&baseline[start..end])
                .all(|(a, b)| rows::RowCodec::key(a) == rows::RowCodec::key(b))
        {
            return Ok(None);
        }
        for (index, row) in fresh.into_iter().enumerate() {
            edit(&mut edits, start + index, row)?;
        }
    }
    replace::<S>(projection, edits)
}

fn apply<S: rows::TableSchema>(
    history: snapshots::History<'_>,
    projection: Option<&Projection<'_>>,
    owner: impl Fn(&S::Row) -> u64,
    make: impl Fn(&rows::VariantSnapshot) -> std::io::Result<Vec<S::Row>>,
) -> TableInput {
    let result = (|| {
        let mut only_partial = true;
        for change in history.iter() {
            let event = change?.view();
            if event.organisation.is_none()
                && event.flags.is_none()
                && event.scalar.is_none()
                && event.sources.is_none()
            {
                only_partial = false;
                break;
            }
        }
        if only_partial {
            return Ok(unavailable(replay_data::cadence::SKIPPED_REASON).into());
        }
        if let Some(input) = sparse::<S>(history, projection, &owner, &make)? {
            return Ok(input);
        }
        let baseline = projection
            .and_then(Projection::observed_baseline::<S>)
            .ok_or("variant changes have no complete preceding design state")?;
        let mut changed: Vec<(u64, Vec<S::Row>)> = Vec::new();
        for change in history.iter() {
            let event = change?.view();
            if event.organisation.is_some()
                || event.flags.is_some()
                || event.scalar.is_some()
                || event.sources.is_some()
            {
                continue;
            }
            let values = if event.removed {
                Vec::new()
            } else {
                let snapshot = event
                    .snapshot
                    .as_ref()
                    .ok_or("variant changes have an incomplete observation")?;
                make(snapshot).map_err(|_| "variant change projection allocation failed")?
            };
            match changed.binary_search_by_key(&event.variant, |(id, _)| *id) {
                Ok(index) => changed[index].1 = values,
                Err(index) => {
                    engine_allocation::reserve(&mut changed, 1)
                        .map_err(|_| "variant change projection allocation failed")?;
                    changed.insert(index, (event.variant, values));
                }
            }
        }
        let mut result = engine_allocation::vec(baseline.len())
            .map_err(|_| "variant change projection allocation failed")?;
        for row in baseline {
            if changed
                .binary_search_by_key(&owner(row), |(id, _)| *id)
                .is_err()
            {
                result.push(
                    rows::RowCodec::try_clone(row)
                        .map_err(|_| "variant change projection allocation failed")?,
                );
            }
        }
        for (_, values) in changed {
            engine_allocation::reserve(&mut result, values.len())
                .map_err(|_| "variant change projection allocation failed")?;
            result.extend(values);
        }
        result.sort_unstable_by_key(rows::RowCodec::key);
        Ok::<_, &'static str>(TableView::Complete(S::wrap(result)).into())
    })();
    result.unwrap_or_else(|reason| unavailable(reason).into())
}

#[cfg(test)]
pub(super) fn project(variants: &Captured, projection: Option<&Projection<'_>>) -> [TableInput; 4] {
    let history = variants.history();
    project_history(variants, history, projection)
}
pub(super) fn project_history(
    variants: &Captured,
    history: snapshots::History<'_>,
    projection: Option<&Projection<'_>>,
) -> [TableInput; 4] {
    if let Outcome::Unavailable { reason } = variants.outcome {
        return std::array::from_fn(|_| unavailable(reason).into());
    }
    if history.is_empty() {
        return std::array::from_fn(|_| unavailable(replay_data::cadence::SKIPPED_REASON).into());
    }
    [
        project_definitions(history, projection),
        apply::<tables::VariantModules>(
            history,
            projection,
            |r| r.variant,
            |s| {
                let mut result = engine_allocation::vec(s.modules.len())?;
                for r in &s.modules {
                    result.push(rows::VariantModuleRow {
                        variant: s.definition.id,
                        source_ordinal: r.ordinal,
                        slot: r.slot,
                        definition_token: r.definition_token,
                        definition_active: r.active,
                    });
                }
                Ok(result)
            },
        ),
        apply::<tables::VariantStats>(
            history,
            projection,
            |r| r.variant,
            |s| {
                let mut result = engine_allocation::vec(1)?;
                let design = s
                    .definition
                    .design
                    .require("VariantChanges.design")
                    .map_err(std::io::Error::other)?;
                result.push(rows::VariantStatsRow {
                    organisation: design.design_team,
                    organisation_trait_count: design.design_team_traits,
                    organisation_bonuses: engine_allocation::collect_vec(
                        design.design_team_bonus.iter().map(|entry| {
                            rows::IndustrialOrganisationBonus {
                                key: entry.index,
                                value_raw: entry.value,
                            }
                        }),
                    )?,
                    variant: s.definition.id,
                    values: s
                        .stats
                        .as_slice()
                        .try_into()
                        .map_err(|_| std::io::Error::other("variant residual length invalid"))?,
                });
                Ok(result)
            },
        ),
        apply::<tables::VariantUpgrades>(
            history,
            projection,
            |r| r.variant,
            |s| {
                let mut result = engine_allocation::vec(s.upgrades.len())?;
                for r in &s.upgrades {
                    result.push(rows::VariantUpgradeRow {
                        variant: s.definition.id,
                        source_ordinal: r.ordinal,
                        definition_token: r.definition_token,
                        level: r.level,
                    });
                }
                Ok(result)
            },
        ),
    ]
}

#[cfg(test)]
pub(super) fn drain(
    base: usize,
    hour: i32,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<usize, &'static str> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    drain_with(&memory, base, hour, output)
}
#[cfg(test)]
fn drain_with(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<usize, &'static str> {
    let mut snapshots = snapshots::Shared::default();
    let result = drain_shared(memory, base, hour, output, &mut snapshots);
    snapshots.materialize(output)?;
    result
}
#[cfg(test)]
fn drain_shared(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    output: &mut Vec<rows::VariantChangeRow>,
    snapshots: &mut snapshots::Shared,
) -> Result<usize, &'static str> {
    drain_speculating(memory, base, hour, output, snapshots, None)
}
fn drain_speculating(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    output: &mut Vec<rows::VariantChangeRow>,
    snapshots: &mut snapshots::Shared,
    speculation: Option<&mut Speculation<'_>>,
) -> Result<usize, &'static str> {
    let start = std::time::Instant::now();
    output.clear();
    snapshots.clear();
    let revision = REVISION.load(Ordering::Acquire);
    let epoch = EPOCH.load(Ordering::Acquire);
    let dependencies = CENSUS_REVISION.load(Ordering::Acquire);
    let Some(store) = STORE.get() else {
        return Err("variant change observer not prepared");
    };
    let mut batch = {
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Store { batch, spare, .. } = &mut *store;
        std::mem::swap(batch, spare);
        std::mem::replace(spare, Batch::empty())
    };
    drain_profile_mut().events = batch.events.len() as u32;
    let result = (|| {
        if !ENABLED.load(Ordering::Acquire) {
            return Err("variant change hooks unavailable");
        }
        let missed = MISSED.swap(false, Ordering::AcqRel);
        if active() != 0 {
            RUNNING_REFUSALS.fetch_add(1, Ordering::Relaxed);
            refuse_census();
            return Err("variant change interval has an unobserved or running writer");
        }
        if missed {
            MISSED_REFUSALS.fetch_add(1, Ordering::Relaxed);
            LAST_MISSED_AT.store(MISSED_AT.load(Ordering::Acquire), Ordering::Release);
            return Err("variant change interval has an unobserved or running writer");
        }
        if let Some(reason) = batch.failure {
            return Err(reason);
        }
        source_hooks::materialize(&mut batch, memory, base)?;
        let repairs = repair::take(&mut batch)?;
        held::check_lifetimes(&batch.events)?;
        let tail = std::time::Instant::now();
        let mut removed = engine_allocation::vec(batch.events.len())
            .map_err(|_| "variant repair allocation failed")?;
        removed.extend(
            batch
                .events
                .iter()
                .filter(|event| event.removed)
                .map(|event| event.pointer),
        );
        {
            let mut profile = drain_profile_mut();
            profile.designs = repairs.watches.len() as u32;
            profile.source_notes = source_hooks::NOTES.swap(0, Ordering::Relaxed) as u32;
            profile.source_writes = source_hooks::WRITES.swap(0, Ordering::Relaxed) as u32;
        }
        let stage = std::time::Instant::now();
        drain_profile_mut().boundary_ns[0] = elapsed_ns(start);
        let witnesses = witness::take(&repairs.watches, &removed);
        let mut reads = snapshots::speculative::read_marked(
            speculation,
            base,
            hour,
            &repairs.watches,
            witnesses,
            &removed,
        )?;
        let walked = repairs.watches.len()
            - reads
                .values()
                .filter(|read| matches!(read, snapshots::speculative::Read::Unchanged(_)))
                .count();
        if walked != 0 {
            collector_observers::world_input_changes::prepare_design_mios(walked, true);
        }
        drain_profile_mut().stages_ns[0] = elapsed_ns(stage);
        for watch in &repairs.watches {
            sources::suspend(watch.id)?;
        }
        let begin = std::time::Instant::now();
        decode_events(&batch, base, output)?;
        drain_profile_mut().events_ns = elapsed_ns(begin);
        for watch in &repairs.watches {
            if matches!(
                reads.get(&watch.pointer),
                Some(snapshots::speculative::Read::Unchanged(_))
            ) {
                sources::resume(watch.id)?;
            }
        }
        liveness::refresh_context(memory, base, &batch.events)?;
        let stage = std::time::Instant::now();
        engine_allocation::reserve(output, repairs.watches.len())
            .map_err(|_| "variant repair allocation failed")?;
        snapshots.reserve(walked)?;
        engine_allocation::reserve(&mut batch.events, repairs.watches.len())
            .map_err(|_| "variant repair allocation failed")?;
        let mut kept =
            engine_allocation::advisory(|| engine_allocation::vec(repairs.watches.len())).ok();
        let (mut unchanged, mut lane_walks) = (0, 0);
        for watch in &repairs.watches {
            use snapshots::speculative::Read;
            let (walk, recorded) = match reads.remove(&watch.pointer) {
                Some(Read::Unchanged(witness)) => {
                    liveness::change(witness.team.0, witness.team.1)?;
                    if let Some(kept) = kept.as_mut() {
                        kept.push((watch.pointer, witness));
                    }
                    unchanged += 1;
                    batch.events.push(Event {
                        kind: Kind::Design,
                        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
                        hour,
                        pointer: watch.pointer,
                        variant: watch.id,
                        removed: false,
                        start: 0,
                        end: 0,
                    });
                    output.push(rows::VariantChangeRow {
                        sequence: OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                        hour,
                        variant: watch.id,
                        flags: Some(watch.flags),
                        ..Default::default()
                    });
                    continue;
                }
                Some(Read::Walked(walk, decoded, recorded)) => (Some((walk, decoded)), recorded),
                _ => (None, None),
            };
            lane_walks += u32::from(walk.is_some());
            let decoded = repair::snapshot(memory, base, hour, watch.pointer, &removed, walk)?;
            if decoded.value.definition.id != watch.id {
                return Err("variant repair identity changed without observed replacement");
            }
            repair::complete(watch, &decoded.value)?;
            if let Some(mut recorded) = recorded {
                let state = decoded
                    .value
                    .definition
                    .design
                    .require("VariantChanges.design")
                    .map_err(|_| "variant repair lacks design state")?;
                recorded.team = (state.design_team, state.design_team_serialized);
                if let Some(kept) = kept.as_mut() {
                    kept.push((watch.pointer, recorded));
                }
            }
            batch.events.push(Event {
                kind: Kind::Design,
                sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
                hour,
                pointer: watch.pointer,
                variant: watch.id,
                removed: false,
                start: 0,
                end: 0,
            });
            snapshots.push(
                output.len() + snapshots.len(),
                OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                hour,
                watch.id,
                std::sync::Arc::new(decoded),
            )?;
        }
        {
            let mut profile = drain_profile_mut();
            profile.unchanged = unchanged;
            profile.lane_walks = lane_walks;
            profile.stages_ns[1] = elapsed_ns(stage);
        }
        let stage = std::time::Instant::now();
        held::apply_shared(&batch.events, output, snapshots)?;
        held::check_lifetimes(&batch.events)?;
        sources::drop_suspended();
        sources::prune_interests()?;
        drain_profile_mut().stages_ns[2] = elapsed_ns(stage);
        drain_profile_mut().reads_ns = elapsed_ns(tail);
        if active() != 0
            || revision != REVISION.load(Ordering::Acquire)
            || epoch != EPOCH.load(Ordering::Acquire)
            || dependencies != CENSUS_REVISION.load(Ordering::Acquire)
            || MISSED.load(Ordering::Acquire)
        {
            unobserved!();
            return Err("variant change proof changed during boundary acquisition");
        }
        if let Some(kept) = kept {
            witness::keep_all(kept);
        }
        repair::settle(&repairs.watches);
        if repairs.lost != 0 {
            REPAIRED_INTERVALS.fetch_add(1, Ordering::Relaxed);
            REPAIRED_RECEIVERS.fetch_add(repairs.watches.len() as u64, Ordering::Relaxed);
        }
        Ok(repairs.lost)
    })();
    if let Err(reason) = result {
        witness::release();
        sources::drop_suspended();
        diagnostics::refused();
        diagnostics::refusal(reason);
        REFUSED_INTERVALS.fetch_add(1, Ordering::Relaxed);
        *LAST_REFUSAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
        withdraw_baselines();
        if !RECOVER.swap(true, Ordering::AcqRel) {
            RECOVERY_READ.store(true, Ordering::Release);
        }
    }
    storage::completed(&batch);
    batch.clear();
    storage::recycle(batch);
    result
}

fn replace_sources(
    state: &mut rows::VariantDesignState,
    sources: &rows::VariantSourceReferences,
) -> Result<(), &'static str> {
    for (target, source) in [
        (&mut state.bonus_sources, &sources.direct),
        (&mut state.named_sources, &sources.named),
    ] {
        let additional = source.len().saturating_sub(target.len());
        engine_allocation::reserve(target, additional)
            .map_err(|_| "variant source change allocation failed")?;
        target.clear();
        target.extend_from_slice(source);
    }
    Ok(())
}

struct DefinitionState {
    rows: Vec<Option<rows::VariantDefinitionRow>>,
    positions: HashMap<u64, usize>,
}
impl DefinitionState {
    fn new(baseline: &[rows::VariantDefinitionRow]) -> Result<Self, &'static str> {
        let mut rows = engine_allocation::vec(baseline.len())
            .map_err(|_| "variant change allocation failed")?;
        let mut positions = HashMap::new();
        engine_allocation::reserve_map(&mut positions, baseline.len())
            .map_err(|_| "variant change allocation failed")?;
        for row in baseline {
            let row =
                rows::RowCodec::try_clone(row).map_err(|_| "variant change allocation failed")?;
            positions.insert(row.id, rows.len());
            rows.push(Some(row));
        }
        Ok(Self { rows, positions })
    }
    fn get(&self, variant: u64) -> Option<&rows::VariantDefinitionRow> {
        self.rows.get(*self.positions.get(&variant)?)?.as_ref()
    }
    fn get_mut(&mut self, variant: u64) -> Option<&mut rows::VariantDefinitionRow> {
        self.rows.get_mut(*self.positions.get(&variant)?)?.as_mut()
    }
    fn remove(&mut self, variant: u64) {
        if let Some(&index) = self.positions.get(&variant) {
            self.rows[index] = None;
        }
    }
    fn insert(&mut self, row: rows::VariantDefinitionRow) -> Result<(), &'static str> {
        if let Some(&index) = self.positions.get(&row.id) {
            if self.rows[index].is_some() {
                return Err("variant change produced duplicate engine ids");
            }
            self.rows[index] = Some(row);
        } else {
            engine_allocation::reserve_map(&mut self.positions, 1)
                .map_err(|_| "variant change allocation failed")?;
            engine_allocation::reserve(&mut self.rows, 1)
                .map_err(|_| "variant change allocation failed")?;
            self.positions.insert(row.id, self.rows.len());
            self.rows.push(Some(row));
        }
        Ok(())
    }
    fn finish(self) -> Result<Vec<rows::VariantDefinitionRow>, &'static str> {
        let mut rows = engine_allocation::vec(self.rows.len())
            .map_err(|_| "variant change allocation failed")?;
        rows.extend(self.rows.into_iter().flatten());
        rows.sort_unstable_by_key(rows::RowCodec::key);
        Ok(rows)
    }
}

fn upgrade_flags(row: &rows::VariantDefinitionRow) -> Option<[u8; 3]> {
    let state = row.design.require("VariantDefinitions.design").ok()?;
    Some([
        state.can_upgrade_type,
        state.can_upgrade_variant,
        state.can_upgrade_modules,
    ])
}
fn project_definitions(
    history: snapshots::History<'_>,
    projection: Option<&Projection<'_>>,
) -> TableInput {
    let result = (|| {
        let baseline = projection
            .and_then(Projection::observed_baseline::<tables::VariantDefinitions>)
            .ok_or("variant changes have no complete preceding design definitions")?;
        if projection
            .and_then(Projection::observed_comparison_baseline::<tables::VariantDefinitions>)
            .is_some()
        {
            let mut edits: Vec<(usize, rows::VariantDefinitionRow)> = Vec::new();
            let mut same_members = true;
            for change in history.iter() {
                let event = change?.view();
                if event.organisation.is_some() {
                    continue;
                }
                let Ok(index) = baseline.binary_search_by_key(&event.variant, |row| row.id) else {
                    same_members = false;
                    break;
                };
                if event.removed {
                    same_members = false;
                    break;
                }
                if let Some(scalar) = &event.scalar {
                    if edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .is_err()
                    {
                        edit(
                            &mut edits,
                            index,
                            rows::RowCodec::try_clone(&baseline[index])
                                .map_err(|_| "variant scalar projection allocation failed")?,
                        )?;
                    }
                    let position = edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .unwrap();
                    rows::RowCodec::check_key_fields(&edits[position].1)
                        .map_err(|_| "variant scalar projection allocation failed")?;
                    scalar
                        .apply_definition(&mut edits[position].1)
                        .map_err(|_| "variant scalar projection failed")?;
                } else if let Some(sources) = &event.sources {
                    if edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .is_err()
                    {
                        edit(
                            &mut edits,
                            index,
                            rows::RowCodec::try_clone(&baseline[index])
                                .map_err(|_| "variant source change allocation failed")?,
                        )?;
                    }
                    let position = edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .unwrap();
                    let row = &mut edits[position].1;
                    rows::RowCodec::check_key_fields(row)
                        .map_err(|_| "variant source change allocation failed")?;
                    let state = row
                        .design
                        .require_mut("VariantDefinitions.design")
                        .map_err(|_| "variant source change lacks design state")?;
                    replace_sources(state, sources)?;
                } else if let Some(flags) = event.flags {
                    let previous = edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .map_or(&baseline[index], |position| &edits[position].1);
                    if upgrade_flags(previous) == Some(flags) {
                        continue;
                    }
                    if edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .is_err()
                    {
                        edit(
                            &mut edits,
                            index,
                            rows::RowCodec::try_clone(&baseline[index])
                                .map_err(|_| "variant change allocation failed")?,
                        )?;
                    }
                    let position = edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .unwrap();
                    let row = &mut edits[position].1;
                    let state = row
                        .design
                        .require_mut("VariantDefinitions.design")
                        .map_err(|_| "variant flags lack their design state")?;
                    [
                        state.can_upgrade_type,
                        state.can_upgrade_variant,
                        state.can_upgrade_modules,
                    ] = flags;
                } else {
                    let snapshot = event
                        .snapshot
                        .as_ref()
                        .ok_or("variant changes have an incomplete observation")?;
                    let previous = edits
                        .binary_search_by_key(&index, |(index, _)| *index)
                        .map_or(&baseline[index], |position| &edits[position].1);
                    if snapshot.definition.matches(previous) {
                        continue;
                    }
                    edit(
                        &mut edits,
                        index,
                        snapshot
                            .try_definition()
                            .map_err(|_| "variant change allocation failed")?,
                    )?;
                }
            }
            if same_members
                && let Some(input) = replace::<tables::VariantDefinitions>(projection, edits)?
            {
                return Ok(input);
            }
        }
        let mut result = DefinitionState::new(baseline)?;
        for change in history.iter() {
            let event = change?.view();
            if event.organisation.is_some() {
                continue;
            }
            if let Some(scalar) = &event.scalar {
                let row = result
                    .get_mut(event.variant)
                    .ok_or("variant scalar names an unknown design")?;
                scalar
                    .apply_definition(row)
                    .map_err(|_| "variant scalar projection failed")?;
                continue;
            }
            if let Some(sources) = &event.sources {
                let row = result
                    .get_mut(event.variant)
                    .ok_or("variant sources name an unknown design")?;
                let state = row
                    .design
                    .require_mut("VariantDefinitions.design")
                    .map_err(|_| "variant source change lacks design state")?;
                replace_sources(state, sources)?;
                continue;
            }
            if let Some(flags) = event.flags {
                if result.get(event.variant).and_then(upgrade_flags) == Some(flags) {
                    continue;
                }
                let row = result
                    .get_mut(event.variant)
                    .ok_or("variant flags name an unknown design")?;
                let state = row
                    .design
                    .require_mut("VariantDefinitions.design")
                    .map_err(|_| "variant flags lack their design state")?;
                [
                    state.can_upgrade_type,
                    state.can_upgrade_variant,
                    state.can_upgrade_modules,
                ] = flags;
                continue;
            }
            if !event.removed
                && result.get(event.variant).is_some_and(|row| {
                    event
                        .snapshot
                        .is_some_and(|snapshot| snapshot.definition.matches(row))
                })
            {
                continue;
            }
            result.remove(event.variant);
            if !event.removed {
                let snapshot = event
                    .snapshot
                    .as_ref()
                    .ok_or("variant changes have an incomplete observation")?;
                let definition = snapshot
                    .try_definition()
                    .map_err(|_| "variant change allocation failed")?;
                result.insert(definition)?;
            }
        }
        Ok::<_, &'static str>(
            TableView::Complete(rows::TableRows::VariantDefinitions(result.finish()?)).into(),
        )
    })();
    result.unwrap_or_else(|reason| unavailable(reason).into())
}

fn callbacks() -> [usize; 23] {
    [
        entry::<0> as *const () as usize,
        entry::<1> as *const () as usize,
        entry::<2> as *const () as usize,
        entry::<3> as *const () as usize,
        entry::<4> as *const () as usize,
        entry::<5> as *const () as usize,
        entry::<6> as *const () as usize,
        entry::<7> as *const () as usize,
        entry::<8> as *const () as usize,
        entry::<9> as *const () as usize,
        entry::<10> as *const () as usize,
        entry::<11> as *const () as usize,
        entry::<12> as *const () as usize,
        entry::<13> as *const () as usize,
        entry::<14> as *const () as usize,
        entry::<15> as *const () as usize,
        entry::<16> as *const () as usize,
        entry::<17> as *const () as usize,
        entry::<18> as *const () as usize,
        entry::<19> as *const () as usize,
        entry::<20> as *const () as usize,
        entry::<21> as *const () as usize,
        entry::<22> as *const () as usize,
    ]
}
unsafe extern "system-unwind" fn entry<const SITE: usize>(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    unsafe { callback::<SITE>(a, b, c, d) }
}

pub struct Captured {
    rows: Vec<rows::VariantChangeRow>,
    snapshots: snapshots::Shared,
    pub(super) outcome: Outcome,
    heap_bytes: usize,
    dictionary: super::bonus_snapshots::Snapshot,
    context: super::design_context::Snapshot,
    templates: super::design_context::Snapshot,
    fog: super::design_fog::Observation,
    pub(super) liveness: liveness::Observation,
    context_acquired: bool,
}
impl Captured {
    pub fn vacant() -> Self {
        Self {
            rows: Vec::new(),
            snapshots: snapshots::Shared::default(),
            outcome: Outcome::Unavailable {
                reason: "variant changes not acquired",
            },
            heap_bytes: 0,
            dictionary: None,
            context: None,
            templates: None,
            fog: super::design_fog::Observation::default(),
            liveness: liveness::Observation::default(),
            context_acquired: false,
        }
    }
    pub(super) fn reset(&mut self) {
        self.context_acquired = false;
        self.outcome = Outcome::Unavailable {
            reason: "variant changes not acquired",
        };
    }
    pub(super) fn dictionary(&self) -> &super::bonus_snapshots::Snapshot {
        if matches!(self.outcome, Outcome::Rows { .. }) {
            &self.dictionary
        } else {
            &None
        }
    }
    pub(super) fn fog(&self) -> Option<&super::design_fog::Observation> {
        self.context_acquired.then_some(&self.fog)
    }
    pub(super) fn templates(&self) -> &super::design_context::Snapshot {
        if self.context_acquired {
            &self.templates
        } else {
            &None
        }
    }
    pub(super) fn context(&self) -> &super::design_context::Snapshot {
        if self.context_acquired {
            &self.context
        } else {
            &None
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<rows::VariantChangeRow>()
            + self.snapshots.allocated_bytes()
            + self.heap_bytes
    }
    pub fn capture(&mut self, base: usize, hour: i32) {
        self.capture_with(
            base,
            hour,
            |memory| super::design_context::capture(memory, base),
            None,
        );
    }
    /// `capture`, with the dirty country context parts acquired by `acquire`.
    ///
    /// # Safety
    /// Snapshot parts borrow this drain's batch and retained cache. `speculate` must return,
    /// on success, failure or unwind, only once it has dropped or returned every part it was
    /// given and no other thread still uses one.
    pub unsafe fn capture_partitioned(
        &mut self,
        base: usize,
        hour: i32,
        lanes: usize,
        mut acquire: impl FnMut(
            Vec<super::design_context::CountryPart>,
        )
            -> Result<Vec<super::design_context::CountryPart>, &'static str>,
        mut speculate: impl FnMut(Vec<SnapshotPart>) -> Result<Vec<SnapshotPart>, &'static str>,
    ) {
        let mut speculation = Speculation {
            lanes,
            run: &mut speculate,
        };
        self.capture_with(
            base,
            hour,
            |memory| super::design_context::capture_partitioned(memory, base, lanes, &mut acquire),
            Some(&mut speculation),
        );
    }
    fn capture_with(
        &mut self,
        base: usize,
        hour: i32,
        context: impl FnOnce(
            &engine_native_memory::memory::LiveMemory,
        ) -> Result<
            (
                super::design_context::Snapshot,
                super::design_context::Snapshot,
            ),
            &'static str,
        >,
        speculation: Option<&mut Speculation<'_>>,
    ) {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        self.fog = super::design_fog::capture(&memory, base);
        self.liveness = liveness::Observation::refused();
        let result = drain_speculating(
            &memory,
            base,
            hour,
            &mut self.rows,
            &mut self.snapshots,
            speculation,
        )
        .and_then(|_| {
            self.liveness = liveness::observation()?;
            self.dictionary = super::bonus_snapshots::snapshot()?;
            Ok(())
        });
        let begin = std::time::Instant::now();
        (self.context, self.templates) = context(&memory).unwrap_or_default();
        drain_profile_mut().boundary_ns[1] = elapsed_ns(begin);
        self.context_acquired = true;
        self.outcome = match result {
            Ok(()) => Outcome::Rows {
                len: self.rows.len() + self.snapshots.len(),
            },
            Err(reason) => {
                self.rows.clear();
                self.snapshots.clear();
                Outcome::Unavailable { reason }
            }
        };
        self.heap_bytes = self.rows.iter().map(rows::RowCodec::heap_bytes).sum();
        publish_availability();
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.outcome = Outcome::Unavailable { reason };
        self.liveness.refuse();
        self.context_acquired = false;
        boundary(true);
    }
    pub(super) fn history(&self) -> snapshots::History<'_> {
        snapshots::History::new(&self.rows, &self.snapshots)
    }
    #[cfg(test)]
    pub(super) fn table_input(&self, projection: Option<&Projection<'_>>) -> TableInput {
        let history = self.history();
        self.table_input_history(history, projection)
    }
    pub(super) fn table_input_history(
        &self,
        history: snapshots::History<'_>,
        projection: Option<&Projection<'_>>,
    ) -> TableInput {
        if let Outcome::Unavailable { reason } = self.outcome {
            return unavailable(reason).into();
        }
        if history.is_empty() {
            return TableView::Complete(rows::TableRows::VariantChanges(Vec::new())).into();
        }
        let result = (|| {
            let mut output = engine_allocation::vec(history.len())?;
            let mut previous = std::collections::HashMap::new();
            engine_allocation::reserve_map(&mut previous, history.len())?;
            append_history(history.iter(), &mut output, &mut previous, projection)?;
            Ok::<_, std::io::Error>(output)
        })();
        match result {
            Ok(rows) => TableView::Complete(rows::TableRows::VariantChanges(rows)).into(),
            Err(_) => unavailable("variant change projection allocation failed").into(),
        }
    }
}

fn append_history<'a>(
    history: impl Iterator<Item = Result<snapshots::Change<'a>, &'static str>>,
    output: &mut Vec<rows::VariantChangeRow>,
    previous: &mut HashMap<u64, snapshots::Change<'a>>,
    projection: Option<&Projection<'_>>,
) -> std::io::Result<()> {
    for change in history {
        let change = change.map_err(std::io::Error::other)?;
        let variant = change.view().variant;
        if !unchanged(change, previous.get(&variant).copied(), projection) {
            engine_allocation::reserve(output, 1)?;
            output.push(change.try_clone()?);
        }
        previous.insert(variant, change);
    }
    Ok(())
}

fn holds_flags(
    variant: u64,
    flags: [u8; 3],
    preceding: Option<snapshots::Change<'_>>,
    projection: Option<&Projection<'_>>,
) -> bool {
    let held = match preceding {
        Some(previous) => {
            let previous = previous.view();
            if previous.removed {
                return false;
            }
            previous.flags.or_else(|| {
                let state = previous
                    .snapshot?
                    .definition
                    .design
                    .require("VariantChanges.design")
                    .ok()?;
                Some([
                    state.can_upgrade_type,
                    state.can_upgrade_variant,
                    state.can_upgrade_modules,
                ])
            })
        }
        None => projection
            .and_then(Projection::observed_baseline::<tables::VariantDefinitions>)
            .and_then(|definitions| {
                let index = definitions
                    .binary_search_by_key(&variant, |row| row.id)
                    .ok()?;
                upgrade_flags(&definitions[index])
            }),
    };
    held == Some(flags)
}

fn unchanged(
    event: snapshots::Change<'_>,
    preceding: Option<snapshots::Change<'_>>,
    projection: Option<&Projection<'_>>,
) -> bool {
    let view = event.view();
    if view.removed {
        return false;
    }
    if let Some(flags) = view.flags {
        return holds_flags(view.variant, flags, preceding, projection);
    }
    let Some(snapshot) = view.snapshot else {
        return false;
    };
    if let Some(previous) = preceding {
        if let (snapshots::Change::Snapshot(current), snapshots::Change::Snapshot(previous)) =
            (event, previous)
            && std::sync::Arc::ptr_eq(&current.value, &previous.value)
        {
            return true;
        }
        let previous = previous.view();
        return !previous.removed && previous.snapshot == Some(snapshot);
    }
    let event = view;
    let Some(projection) = projection else {
        return false;
    };
    let Some(definitions) = projection.observed_baseline::<tables::VariantDefinitions>() else {
        return false;
    };
    let Ok(index) = definitions.binary_search_by_key(&event.variant, |row| row.id) else {
        return false;
    };
    if !snapshot.definition.matches(&definitions[index]) {
        return false;
    }
    let (Some(modules), Some(stats), Some(upgrades)) = (
        projection.observed_baseline::<tables::VariantModules>(),
        projection.observed_baseline::<tables::VariantStats>(),
        projection.observed_baseline::<tables::VariantUpgrades>(),
    ) else {
        return false;
    };
    let module_start = modules.partition_point(|row| row.variant < event.variant);
    let upgrade_start = upgrades.partition_point(|row| row.variant < event.variant);
    let Ok(stat) = stats.binary_search_by_key(&event.variant, |row| row.variant) else {
        return false;
    };
    stats[stat].values.as_slice() == snapshot.stats.as_slice()
        && modules[module_start..]
            .iter()
            .take_while(|row| row.variant == event.variant)
            .map(|r| {
                (
                    r.source_ordinal,
                    r.slot,
                    r.definition_token,
                    r.definition_active,
                )
            })
            .eq(snapshot
                .modules
                .iter()
                .map(|r| (r.ordinal, r.slot, r.definition_token, r.active)))
        && upgrades[upgrade_start..]
            .iter()
            .take_while(|row| row.variant == event.variant)
            .map(|r| (r.source_ordinal, r.definition_token, r.level))
            .eq(snapshot
                .upgrades
                .iter()
                .map(|r| (r.ordinal, r.definition_token, r.level)))
}

impl Drop for Hooks {
    fn drop(&mut self) {
        OWN_ENABLED.store(false, Ordering::Release);
        collector_observers::world_input_changes::forwarder(None);
        update_enabled();
        if let Some(store) = STORE.get() {
            let mut store = store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *store = Store {
                batch: Batch::empty(),
                spare: Batch::empty(),
                watched: HashMap::new(),
                published: Published::default(),
                interests: sources::Interests::default(),
                epoch: EPOCH.load(Ordering::Acquire),
            };
        }
        storage::release();
        writers::release();
        // SAFETY: the owner drops these hooks only after patch restoration and callback
        // quiescence, with acquisition stopped.
        unsafe { repair::release() };
        super::bonus_snapshots::release();
        sources::release();
        liveness::release();
        held::release();
        witness::release();
        super::design_context::invalidate();
    }
}

#[cfg(test)]
fn decode_batch(
    batch: &Batch,
    base: usize,
    _hour: i32,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<(), &'static str> {
    decode_events(batch, base, output)
}
fn decode_events(
    batch: &Batch,
    base: usize,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<(), &'static str> {
    let mut preceding_sequence = 0;
    for event in &batch.events {
        if event.sequence == 0 || event.sequence <= preceding_sequence {
            return Err("variant occurrences are not in native observation order");
        }
        preceding_sequence = event.sequence;
        match event.kind {
            Kind::Organisation(live) => {
                liveness::change(event.variant, live)?;
                engine_allocation::reserve(output, 1)
                    .map_err(|_| "design liveness event allocation failed")?;
                output.push(rows::VariantChangeRow {
                    sequence: OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                    hour: event.hour,
                    organisation: Some(rows::VariantOrganisationChange {
                        organisation: event.variant,
                        live,
                    }),
                    ..Default::default()
                });
            }
            Kind::SourceRemoved | Kind::ProviderReset(_) => sources::decode(
                event,
                &engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: &[],
                },
                base,
                output,
            )?,
            Kind::Source | Kind::Provider(_) => {
                let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end)?;
                sources::decode(event, &memory, base, output)?;
            }
            Kind::Design if event.removed => {
                witness::remove(event.pointer);
                super::design_context::remove(event.variant)?;
                sources::remove(event.variant);
                engine_allocation::reserve(output, 1)
                    .map_err(|_| "variant removal allocation failed")?;
                output.push(rows::VariantChangeRow {
                    sequence: OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                    hour: event.hour,
                    variant: event.variant,
                    removed: true,
                    ..Default::default()
                });
            }
            Kind::Design => return Err("variant design occurrence outside a boundary read"),
        }
    }
    Ok(())
}

#[cfg(test)]
fn snapshot(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    pointer: usize,
    fresh: bool,
) -> Result<rows::VariantSnapshot, &'static str> {
    let decoded = decode_snapshot(memory, base, hour, pointer, fresh)?;
    Ok(decoded.value)
}
thread_local! {
    static DECODE_SCRATCH: std::cell::RefCell<Option<Box<Variants>>> =
        const { std::cell::RefCell::new(None) };
    static WALK_SCRATCH: std::cell::RefCell<Option<Box<RawWalk>>> =
        const { std::cell::RefCell::new(None) };
}
struct DecodeScratch(Option<Box<Variants>>);
impl DecodeScratch {
    fn take() -> Self {
        let mut scratch = DECODE_SCRATCH
            .with(|scratch| scratch.borrow_mut().take())
            .unwrap_or_else(|| Box::new(Variants::vacant()));
        scratch.reset();
        Self(Some(scratch))
    }
}
impl std::ops::Deref for DecodeScratch {
    type Target = Variants;
    fn deref(&self) -> &Variants {
        self.0.as_deref().unwrap()
    }
}
impl std::ops::DerefMut for DecodeScratch {
    fn deref_mut(&mut self) -> &mut Variants {
        self.0.as_deref_mut().unwrap()
    }
}
impl Drop for DecodeScratch {
    fn drop(&mut self) {
        if let Some(scratch) = self.0.take() {
            DECODE_SCRATCH.with(|slot| *slot.borrow_mut() = Some(scratch));
        }
    }
}
struct WalkScratch(Option<Box<RawWalk>>);
impl WalkScratch {
    fn take() -> Self {
        Self(Some(
            WALK_SCRATCH
                .with(|scratch| scratch.borrow_mut().take())
                .unwrap_or_else(|| Box::new(RawWalk::vacant())),
        ))
    }
}
impl std::ops::Deref for WalkScratch {
    type Target = RawWalk;
    fn deref(&self) -> &RawWalk {
        self.0.as_deref().unwrap()
    }
}
impl std::ops::DerefMut for WalkScratch {
    fn deref_mut(&mut self) -> &mut RawWalk {
        self.0.as_deref_mut().unwrap()
    }
}
impl Drop for WalkScratch {
    fn drop(&mut self) {
        if let Some(scratch) = self.0.take() {
            WALK_SCRATCH.with(|slot| *slot.borrow_mut() = Some(scratch));
        }
    }
}
type RawDefinition = native::DefinitionRow<Option<native::VariantDesignState>>;
struct RawWalk {
    sources: usize,
    details: bool,
    definitions: CaptureRows<RawDefinition>,
    definition_bytes: Arena<u8>,
    modules: CaptureRows<engine_equipment::equipment_module_container::Row>,
    stats: CaptureRows<native::StatsRow>,
    upgrades: CaptureRows<engine_equipment::cequipment_upgrades_instance::Row>,
}
impl RawWalk {
    fn vacant() -> Self {
        Self {
            sources: 0,
            details: false,
            definitions: CaptureRows::new(0),
            definition_bytes: Arena::new(0),
            modules: CaptureRows::new(0),
            stats: CaptureRows::new(0),
            upgrades: CaptureRows::new(0),
        }
    }
    fn reset(&mut self) {
        self.sources = 0;
        self.details = false;
        self.definitions.reset();
        self.definition_bytes.reset();
        self.modules.reset();
        self.stats.reset();
        self.upgrades.reset();
    }
    fn complete(&self) -> bool {
        self.sources == 1
            && self.details
            && [
                self.definitions.outcome,
                self.modules.outcome,
                self.stats.outcome,
                self.upgrades.outcome,
            ]
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    }
}
fn walk_snapshot(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    pointer: usize,
    walk: &mut RawWalk,
) -> Result<(), &'static str> {
    walk.reset();
    let source = [native::RegistryRow::read(memory, base, pointer)?];
    walk.sources = source.len();
    let mut reads = crate::read_accounting::table(super::TABLES[3]);
    super::capture_text(
        &mut walk.definitions,
        &mut walk.definition_bytes,
        Ok(&source[..]),
        |source, rows, arena| {
            native::walk_definitions_carried(
                memory,
                base,
                source,
                rows,
                arena,
                |_| Ok(None),
                |state| Ok(Some(state)),
            )
        },
    );
    if !replay_data::cadence::should_walk(replay_data::rows::TableId::VariantModules, hour, true) {
        let skipped = Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        };
        walk.modules.outcome = skipped;
        walk.stats.outcome = skipped;
        walk.upgrades.outcome = skipped;
        return Ok(());
    }
    walk.details = true;
    reads.select(super::TABLES[4]);
    super::capture_source(&mut walk.modules, Ok(&source[..]), |source, rows| {
        engine_equipment::equipment_module_container::walk(memory, base, source, rows)
    });
    super::capture_source(&mut walk.stats, Ok(&source[..]), |source, rows| {
        let mut witness = StatLaneWitness;
        native::walk_stats_carried(memory, base, source, rows, &mut (), &mut witness)
    });
    super::capture_source(&mut walk.upgrades, Ok(&source[..]), |source, rows| {
        engine_equipment::cequipment_upgrades_instance::walk(memory, base, source, rows)
    });
    Ok(())
}
fn intern_definitions(
    target: &mut CaptureRows<native::DefinitionRow<std::sync::Arc<rows::VariantDesignState>>>,
    raw: &mut CaptureRows<RawDefinition>,
) -> Result<(), &'static str> {
    target.outcome = raw.outcome;
    let len = raw.outcome.len();
    let result = (|| {
        if target.rows.len() < len {
            engine_allocation::resize_default(&mut target.rows, len)
                .map_err(|_| "variant definition allocation failed")?;
        }
        for (target, raw) in target.rows.iter_mut().zip(raw.rows.iter_mut()).take(len) {
            let row = std::mem::take(raw);
            let design = super::bonus_snapshots::intern(
                row.design.ok_or("variant definition design missing")?,
            )?;
            *target = native::DefinitionRow {
                identity: row.identity,
                archetype_pointer: row.archetype_pointer,
                organisation_pointer: row.organisation_pointer,
                source_links: row.source_links,
                display_name: row.display_name,
                variant_name: row.variant_name,
                explicit_entity: row.explicit_entity,
                icon_active: row.icon_active,
                icon_kind: row.icon_kind,
                icon: row.icon,
                version: row.version,
                design: std::sync::Arc::new(design),
            };
        }
        Ok(())
    })();
    if let Err(reason) = result {
        target.outcome = Outcome::Unavailable { reason };
    }
    result
}
fn decode_walk(walk: &mut RawWalk) -> Result<snapshots::Decoded, &'static str> {
    let mut captured = DecodeScratch::take();
    let interned = intern_definitions(&mut captured.definitions, &mut walk.definitions);
    std::mem::swap(&mut captured.definition_bytes, &mut walk.definition_bytes);
    if interned.is_err() {
        captured.definition_bytes.len = 0;
        captured.definition_bytes.required = 0;
    }
    std::mem::swap(&mut captured.modules, &mut walk.modules);
    std::mem::swap(&mut captured.stats, &mut walk.stats);
    std::mem::swap(&mut captured.upgrades, &mut walk.upgrades);
    let result = assemble_snapshot(&mut captured);
    std::mem::swap(&mut captured.definition_bytes, &mut walk.definition_bytes);
    std::mem::swap(&mut captured.modules, &mut walk.modules);
    std::mem::swap(&mut captured.stats, &mut walk.stats);
    std::mem::swap(&mut captured.upgrades, &mut walk.upgrades);
    result
}
fn assemble_snapshot(captured: &mut Variants) -> Result<snapshots::Decoded, &'static str> {
    let definition = match definitions(&captured.definitions, &captured.definition_bytes) {
        TableView::Complete(rows::TableRows::VariantDefinitions(mut rows)) if rows.len() == 1 => {
            rows.remove(0)
        }
        _ => return Err("variant change definition decode failed"),
    };
    if !matches!(captured.modules.outcome, Outcome::Rows { .. })
        || !matches!(captured.stats.outcome, Outcome::Rows { len: 1 })
        || !matches!(captured.upgrades.outcome, Outcome::Rows { .. })
    {
        return Err("variant change children decode failed");
    }
    let snapshot = rows::VariantSnapshot {
        definition: definition.into(),
        modules: captured.modules.rows[..captured.modules.outcome.len()]
            .iter()
            .map(|r| rows::VariantSnapshotModule {
                ordinal: r.source_ordinal,
                slot: r.slot,
                definition_token: r.definition_token,
                active: r.definition_active,
            })
            .collect(),
        stats: engine_allocation::copy_vec(&captured.stats.rows[0].values)
            .map_err(|_| "variant change allocation failed")?,
        upgrades: captured.upgrades.rows[..captured.upgrades.outcome.len()]
            .iter()
            .map(|r| rows::VariantSnapshotUpgrade {
                ordinal: r.source_ordinal,
                definition_token: r.definition_token,
                level: r.level,
            })
            .collect(),
    };
    Ok(snapshots::Decoded {
        value: snapshot,
        definition: std::mem::take(&mut captured.definitions.rows[0]),
    })
}
fn complete_snapshot(
    memory: &impl Memory,
    base: usize,
    fresh: bool,
    walk: &mut RawWalk,
) -> Result<snapshots::Decoded, &'static str> {
    let begin = std::time::Instant::now();
    let decoded = decode_walk(walk);
    drain_profile_mut().complete_ns[0] += elapsed_ns(begin);
    complete_decoded(memory, base, fresh, walk, decoded?)
}
fn complete_decoded(
    memory: &impl Memory,
    base: usize,
    fresh: bool,
    walk: &RawWalk,
    decoded: snapshots::Decoded,
) -> Result<snapshots::Decoded, &'static str> {
    let begin = std::time::Instant::now();
    let result = observe_walk(memory, base, fresh, walk, &decoded.definition);
    drain_profile_mut().complete_ns[1] += elapsed_ns(begin);
    result.map(|()| decoded)
}
fn decode_snapshot(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    pointer: usize,
    fresh: bool,
) -> Result<snapshots::Decoded, &'static str> {
    let mut walk = WalkScratch::take();
    walk_snapshot(memory, base, hour, pointer, &mut walk)?;
    complete_snapshot(memory, base, fresh, &mut walk)
}
fn observe_walk(
    memory: &impl Memory,
    base: usize,
    fresh: bool,
    walk: &RawWalk,
    definition: &native::DefinitionRow<std::sync::Arc<rows::VariantDesignState>>,
) -> Result<(), &'static str> {
    if walk.details {
        for _ in 0..walk.stats.outcome.len() {
            collector_observers::world_input_changes::record_daily(
                collector_observers::world_input_changes::Domain::DesignIndustrialOrganisation,
                false,
                true,
            );
        }
    }
    if fresh {
        let mut store = STORE
            .get()
            .ok_or("variant change observer not prepared")?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sources::repair(definition, &mut store.interests)?;
        liveness::seed_one(definition, &mut store.interests)?;
    } else {
        sources::track(definition)?;
    }
    liveness::sample(definition)?;
    super::design_context::event(
        memory,
        base,
        definition,
        &walk.modules.rows[..walk.modules.outcome.len()],
        walk.upgrades.rows[..walk.upgrades.outcome.len()].first(),
    )
}

#[cfg(test)]
fn isolated() -> bool {
    const CHILD: &str = "BATTLEPLAN_DESIGN_CHANGES_ISOLATED_TEST";
    let thread = std::thread::current();
    let name = thread.name().unwrap();
    if std::env::var_os(CHILD).as_deref() == Some(std::ffi::OsStr::new(name)) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated test failed: {}\n--- child stderr ---\n{}\n--- child stdout ---\n{stdout}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

#[cfg(test)]
mod tests;
