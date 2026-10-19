//! Owned snapshots taken at native design mutation boundaries. Native callbacks
//! only fill prepared raw storage; typed decoding and growth run on a worker.
use super::*;
mod forward;
pub(super) mod held;
pub(super) mod liveness;
mod scalar;
mod source_hooks;
mod sources;
use engine_native::cequipment_variant as native;
pub use forward::{Availability, Forwarder, Phase, Writer, availability_is_current, forwarder};
use std::collections::HashMap;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const ROOT_BYTES: usize = native::DESIGN_TEAM_TRAITS + 4;
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
static MISSED: AtomicBool = AtomicBool::new(false);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(1);
static SEED_REVISION: AtomicU64 = AtomicU64::new(0);
static SEED_EPOCH: AtomicU64 = AtomicU64::new(0);
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
fn publish_availability() {
    let epoch = EPOCH.load(Ordering::Acquire);
    let token = forward::token();
    let saved = ENABLED.load(Ordering::Acquire) && SAVED_BASELINE.load(Ordering::Acquire);
    // Shared move forwarding must be live before a dependent capability can
    // become current. Reset wins any raced world-input publication below.
    collector_observers::world_input_changes::observe_forwarder(saved);
    forward::available_at(
        token,
        saved
            && SOURCE_BASELINE.load(Ordering::Acquire)
            && LIVENESS_BASELINE.load(Ordering::Acquire)
            && !RECOVER.load(Ordering::Acquire)
            && !MISSED.load(Ordering::Acquire),
    );
    // A reset can run while this worker publishes either forwarding gate.
    // Its epoch wins even when the old publisher resumes after the reset.
    if EPOCH.load(Ordering::Acquire) != epoch || !SAVED_BASELINE.load(Ordering::Acquire) {
        collector_observers::world_input_changes::observe_forwarder(false);
    }
}

fn withdraw_baselines() {
    SAVED_BASELINE.store(false, Ordering::Release);
    SOURCE_BASELINE.store(false, Ordering::Release);
    LIVENESS_BASELINE.store(false, Ordering::Release);
    forward::invalidate();
    collector_observers::world_input_changes::observe_forwarder(false);
}

// Descriptor order stays shared with the receiver decoder. These sites are
// patched exclusively by world-input, and never enter this family's hook list.
fn update_enabled() {
    let enabled = OWN_ENABLED.load(Ordering::Acquire)
        && WORLD_ENABLED.load(Ordering::Acquire)
        && source_hooks::enabled()
        && liveness::enabled()
        && super::design_template_changes::enabled();
    let previous = ENABLED.swap(enabled, Ordering::AcqRel);
    if !enabled {
        if previous {
            // A dependency gap invalidates address bindings even when it is
            // closed again before the next worker drain.
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
            MISSED.store(true, Ordering::Release);
        }
    }
}
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
        MISSED.store(true, Ordering::Release);
    }
    update_enabled();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Design,
    Scalar(scalar::Field),
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
    flags: Option<[u8; 3]>,
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
    fn prepare(&mut self, events: usize, spans: usize, bytes: usize) {
        self.log.grow();
        self.log.presize(spans, bytes);
        if events > self.events.capacity() {
            let additional = events - self.events.len();
            let _ = engine_allocation::reserve(&mut self.events, additional);
        }
    }
}
#[derive(Clone, Copy)]
struct Watch {
    id: u64,
    pointer: usize,
    flags: [u8; 3],
}
struct Store {
    batch: Batch,
    spare: Batch,
    watched: HashMap<usize, Watch>,
    interests: sources::Interests,
    epoch: u64,
}
static RECOVER: AtomicBool = AtomicBool::new(true);
static RECOVERY_READ: AtomicBool = AtomicBool::new(true);
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
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
/// The fresh census completed all dependencies at this parked boundary.
/// It establishes a new interval; no events from an earlier gap are invented.
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
        collector_observers::world_input_changes::observe_forwarder(false);
        EPOCH.fetch_add(1, Ordering::AcqRel);
        withdraw_baselines();
        super::design_context::invalidate();
        RECOVER.store(true, Ordering::Release);
        RECOVERY_READ.store(true, Ordering::Release);
        MISSED.store(true, Ordering::Release);
        if let Some(store) = STORE.get() {
            // A worker may be preparing its next buffers. The missed verdict
            // already invalidates this batch; never wait for that worker here.
            if let Ok(mut store) = store.try_lock() {
                store.batch.clear();
            }
        }
    }
}

fn u64_at(log: &RawLog<1>, offset: usize) -> u64 {
    u64::from_le_bytes(log.bytes[offset..offset + 8].try_into().unwrap())
}
fn u32_at(log: &RawLog<1>, offset: usize) -> u32 {
    u32::from_le_bytes(log.bytes[offset..offset + 4].try_into().unwrap())
}
fn take(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    pointer: usize,
    size: usize,
) -> Result<usize, &'static str> {
    log.take_bulk(memory, pointer, size, 1)
        .ok_or("variant change raw storage exhausted or unreadable")
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

/// Record the resolver's original requests, so decode never substitutes a
/// later reference-registry state for this design's serialization witness.
struct Staging<'a, M> {
    log: std::cell::RefCell<&'a mut RawLog<1>>,
    source: &'a M,
}
impl<M: Memory> Memory for Staging<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let mut log = self.log.borrow_mut();
        let Some(start) = log.take_bulk(self.source, address, output.len(), 1) else {
            return false;
        };
        output.copy_from_slice(&log.bytes[start..start + output.len()]);
        true
    }
}
fn reference(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    base: usize,
    handle: u64,
) -> Result<Option<usize>, &'static str> {
    let pointer = engine_native::creference_registry::resolve(
        &Staging {
            log: std::cell::RefCell::new(&mut *log),
            source: memory,
        },
        base,
        handle,
    )?;
    if let Some(pointer) = pointer {
        take(log, memory, pointer, 16)?;
        take(
            log,
            memory,
            pointer
                .checked_add(engine_production::nindustrial_organisation_corganisation::OWNER)
                .ok_or("design organisation owner address overflow")?,
            4,
        )?;
    }
    Ok(pointer)
}

/// No allocation or native calls. Every pointer remains private to this raw
/// observation and is followed only while the enclosing writer is quiescent.
fn stage_interests(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    mut interests: Option<&mut sources::Interests>,
) -> Result<(), &'static str> {
    let root = take(log, memory, pointer, native::STATS)?;
    let tail = take(
        log,
        memory,
        pointer
            .checked_add(native::MISSION_GROUP_DATA)
            .ok_or("variant change address overflow")?,
        ROOT_BYTES - native::MISSION_GROUP_DATA,
    )?;
    let field = |offset: usize| {
        if offset < native::STATS {
            root + offset
        } else {
            tail + offset - native::MISSION_GROUP_DATA
        }
    };
    let design_team = u64_at(log, field(native::DESIGN_TEAM));
    if design_team != 0 {
        let organisation = reference(log, memory, base, design_team)?;
        if let Some(interests) = interests.as_mut() {
            interests.organisation(design_team, organisation)?;
        }
    }
    if u32_at(log, field(native::NAMED_BONUSES) + 12) != 0 {
        take(
            log,
            memory,
            pointer
                .checked_add(native::STATS)
                .ok_or("variant change address overflow")?,
            native::STAT_COUNT * 8,
        )?;
        let staged = Staging {
            log: std::cell::RefCell::new(&mut *log),
            source: memory,
        };
        let provider = native::visit_named_sources(&staged, base, pointer, |pointer, _, _| {
            let mut log = staged.log.borrow_mut();
            let source = take(&mut log, memory, pointer, native::NAMED_BONUS_STRIDE)?;
            string(&mut log, memory, source + native::NAMED_BONUS_NAME)?;
            string(&mut log, memory, source + native::NAMED_BONUS_DESCRIPTION)?;
            array(&mut log, memory, source + 8 + native::BONUS_STATS, 16)?;
            array(&mut log, memory, source + 8 + native::BONUS_TERRAIN, 8)?;
            Ok(())
        })?;
        if let Some((owner, status)) = provider
            && let Some(interests) = interests.as_mut()
        {
            interests.insert(sources::Key::Provider(status), owner)?;
        }
    }
    for (offset, size) in [
        (native::MAP_RANGE_STAT, 32),
        (native::MAXIMUM_SPEED_CACHE_CANDIDATE, 8),
        (native::RELIABILITY_CACHE_CANDIDATE, 8),
        (native::AIR_DEFENSE_CACHE_CANDIDATE, 24),
        (native::BUILD_COST_CACHE_CANDIDATE, 8),
    ] {
        take(
            log,
            memory,
            pointer
                .checked_add(offset)
                .ok_or("variant change address overflow")?,
            size,
        )?;
    }
    if u64_at(log, root)
        != base
            .checked_add(native::VTABLE_RVA)
            .ok_or("variant change module address overflow")? as u64
        || u32_at(log, field(native::TYPE)) != native::REFERENCE_TYPE
    {
        return Err("variant change identity mismatch");
    }
    for offset in [
        native::DISPLAY_NAME,
        native::VARIANT_NAME,
        native::EXPLICIT_ENTITY,
        native::ICON_OVERRIDE_TOKEN,
    ] {
        string(log, memory, field(offset))?;
    }
    let definition = u64_at(log, field(native::DEFINITION)) as usize;
    take(log, memory, definition, 12)?;
    let archetype = take(
        log,
        memory,
        definition
            .checked_add(engine_native::cequipment_type::ARCHETYPE)
            .ok_or("variant change address overflow")?,
        8,
    )?;
    let archetype = u64_at(log, archetype) as usize;
    take(log, memory, archetype, 12)?;
    take(
        log,
        memory,
        archetype
            .checked_add(engine_native::cequipment_type::DESIGN_COST_MODIFIER)
            .ok_or("variant change type context address overflow")?,
        4,
    )?;
    let parent = u64_at(log, field(native::PARENT)) as usize;
    if parent != 0 {
        take(log, memory, parent, 16)?;
    }
    let group = u64_at(log, field(native::DIVISION_NAMES_GROUP)) as usize;
    if group != 0 {
        let group = take(log, memory, group, 40)?;
        string(log, memory, group + 8)?;
    }
    let (modules, count) = array(log, memory, field(native::MODULES) + 8, 16)?;
    for ordinal in 0..count as usize {
        let module = u64_at(log, modules + ordinal * 16 + 8) as usize;
        take(log, memory, module, 12)?;
        take(
            log,
            memory,
            module
                .checked_add(engine_native::cequipment_module::DESIGN_COST_MODIFIER)
                .ok_or("variant change module context address overflow")?,
            4,
        )?;
        take(
            log,
            memory,
            module
                .checked_add(engine_native::cequipment_module::ACTIVE)
                .ok_or("variant change address overflow")?,
            1,
        )?;
    }
    let (upgrades, count) = array(log, memory, field(native::UPGRADES) + 8, 16)?;
    for ordinal in 0..count as usize {
        let upgrade = u64_at(log, upgrades + ordinal * 16) as usize;
        take(log, memory, upgrade, 12)?;
        if ordinal == 0 {
            take(
                log,
                memory,
                upgrade
                    .checked_add(engine_native::cequipment_upgrade::COST_CURRENCY)
                    .ok_or("upgrade currency address overflow")?,
                4,
            )?;
        }
    }
    let (missions, count) = array(log, memory, field(native::MISSION_GROUP_DATA), 40)?;
    for ordinal in 0..count as usize {
        array(log, memory, missions + ordinal * 40 + 16, 16)?;
    }
    let (excluded, count) = array(log, memory, field(native::MODULE_MISSION_EXCLUSIONS), 16)?;
    for ordinal in 0..count as usize {
        take(
            log,
            memory,
            u64_at(log, excluded + ordinal * 16) as usize,
            12,
        )?;
    }
    for (offset, stride) in [
        (native::NAMED_BONUSES, 4),
        (native::TERRAIN_ATTRITION, 8),
        (native::NICHES, 4),
        (native::BUILD_RESOURCES, 16),
        (native::DESIGN_TEAM_BONUS, 16),
    ] {
        array(log, memory, field(offset), stride)?;
    }
    let (sources, count) = array(log, memory, field(native::BONUS_SOURCES), 8)?;
    for ordinal in 0..count as usize {
        let source = u64_at(log, sources + ordinal * 8) as usize;
        if let Some(interests) = interests.as_mut() {
            interests.insert(sources::Key::Direct(source), 0)?;
        }
        let size = native::bonus_source_size(memory, base, source)?;
        let source = take(log, memory, source, size)?;
        array(log, memory, source + native::BONUS_STATS, 16)?;
        array(log, memory, source + native::BONUS_TERRAIN, 8)?;
        let idea = if size > native::BONUS_IDEA {
            u64_at(log, source + native::BONUS_IDEA) as usize
        } else {
            0
        };
        if idea != 0 {
            let idea = take(log, memory, idea, engine_native::cidea::TOKEN + 32)?;
            string(log, memory, idea + engine_native::cidea::TOKEN)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn stage(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(), &'static str> {
    stage_interests(log, memory, base, pointer, None)
}

/// Called after a complete census on its acquisition worker. Capacity derives
/// from that census's owned values, including every nested native occurrence.
pub(super) fn prepare(registry: &[native::RegistryRow], variants: &Variants) {
    let mut bytes = registry.len().saturating_mul(ROOT_BYTES + 36);
    let mut spans = registry.len().saturating_mul(21);
    for row in variants
        .definitions
        .rows
        .iter()
        .take(variants.definitions.outcome.len())
    {
        bytes = bytes.saturating_add(
            row.display_name.len
                + row.variant_name.len
                + row.explicit_entity.len
                + row.icon.len
                + 4,
        );
        let state = &row.design;
        if state.design_team != 0 {
            spans = spans.saturating_add(16);
            bytes = bytes.saturating_add(512);
            if row.organisation_pointer.is_some() {
                spans = spans.saturating_add(2);
                bytes = bytes.saturating_add(20);
            }
        }
        bytes = bytes.saturating_add(
            usize::from(!state.named_bonuses.is_empty()) * native::STAT_COUNT * 8
                + state.design_team_bonus.len() * 16
                + state.named_bonuses.len() * 4
                + state.module_mission_exclusions.len() * 16
                + state.terrain_attrition.len() * 8
                + state.niches.len() * 4
                + state.build_resources.len() * 16,
        );
        let Ok((source_bytes, source_spans)) =
            super::bonus_snapshots::raw_demand(&state.bonus_sources)
        else {
            RECOVER.store(true, Ordering::Release);
            MISSED.store(true, Ordering::Release);
            return;
        };
        bytes = bytes.saturating_add(source_bytes);
        spans = spans.saturating_add(source_spans);
        let Ok((source_bytes, source_spans)) =
            super::bonus_snapshots::raw_demand(&state.named_sources)
        else {
            RECOVER.store(true, Ordering::Release);
            MISSED.store(true, Ordering::Release);
            return;
        };
        bytes = bytes.saturating_add(source_bytes);
        spans = spans.saturating_add(source_spans);
        for mission in &state.mission_stats {
            spans = spans.saturating_add(1);
            bytes = bytes.saturating_add(40 + mission.stats.len() * 16);
        }
        bytes = bytes.saturating_add(
            state
                .division_names_group
                .as_ref()
                .map_or(0, |s| 41 + s.len())
                + usize::from(state.parent.is_some()) * 16,
        );
    }
    let modules = variants.modules.outcome.len();
    let upgrades = variants.upgrades.outcome.len();
    spans = spans.saturating_add(modules.saturating_mul(3) + upgrades);
    bytes = bytes.saturating_add(modules.saturating_mul(33) + upgrades.saturating_mul(28));
    let store = STORE.get_or_init(|| {
        Mutex::new(Store {
            batch: Batch::empty(),
            spare: Batch::empty(),
            watched: HashMap::new(),
            interests: sources::Interests::default(),
            epoch: EPOCH.load(Ordering::Acquire),
        })
    });
    let mut store = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let epoch = EPOCH.load(Ordering::Acquire);
    let revision = SEED_REVISION.load(Ordering::Acquire);
    if !RECOVER.load(Ordering::Acquire) && store.epoch == epoch && !store.batch.events.is_empty() {
        return;
    }
    let mut attempt = BaselineAttempt { accepted: false };
    if SEED_EPOCH.load(Ordering::Acquire) != epoch
        || REVISION.load(Ordering::Acquire) != revision
        || active() != 0
    {
        MISSED.store(true, Ordering::Release);
        RECOVER.store(true, Ordering::Release);
        return;
    }
    if store.epoch != epoch {
        store.watched.clear();
        store.interests = sources::Interests::default();
        store.batch.clear();
        sources::release();
        liveness::release();
        held::release();
        store.epoch = epoch;
    }
    store
        .batch
        .prepare(registry.len().saturating_mul(2).max(1), spans, bytes);
    store
        .spare
        .prepare(registry.len().saturating_mul(2).max(1), spans, bytes);
    store.watched.clear();
    if engine_allocation::reserve_map(&mut store.watched, registry.len().saturating_mul(2).max(1))
        .is_err()
    {
        RECOVER.store(true, Ordering::Release);
        MISSED.store(true, Ordering::Release);
        return;
    }
    let mut ids = std::collections::HashSet::new();
    for (source, definition) in registry.iter().zip(&variants.definitions.rows) {
        if !ids.insert(source.id)
            || store
                .watched
                .insert(
                    source.pointer(),
                    Watch {
                        id: source.id,
                        pointer: source.pointer(),
                        flags: [
                            definition.design.can_upgrade_type,
                            definition.design.can_upgrade_variant,
                            definition.design.can_upgrade_modules,
                        ],
                    },
                )
                .is_some()
        {
            store.batch.failure =
                Some("variant change census contains duplicate identity or address");
            return;
        }
    }
    if let Err(reason) = held::seed(registry, variants) {
        store.batch.failure = Some(reason);
        RECOVER.store(true, Ordering::Release);
        return;
    }
    if let Err(reason) = liveness::seed(variants, &mut store.interests) {
        store.batch.failure = Some(reason);
        RECOVER.store(true, Ordering::Release);
        return;
    }
    if let Err(reason) = sources::seed(variants, &mut store.interests) {
        store.batch.failure = Some(reason);
        RECOVER.store(true, Ordering::Release);
        return;
    }
    if active() != 0
        || store.epoch != EPOCH.load(Ordering::Acquire)
        || REVISION.load(Ordering::Acquire) != revision
    {
        MISSED.store(true, Ordering::Release);
        return;
    }
    SAVED_BASELINE.store(true, Ordering::Release);
    SOURCE_BASELINE.store(true, Ordering::Release);
    LIVENESS_BASELINE.store(true, Ordering::Release);
    RECOVER.store(false, Ordering::Release);
    MISSED.store(false, Ordering::Release);
    drop(store);
    if REVISION.load(Ordering::Acquire) != revision
        || active() != 0
        || EPOCH.load(Ordering::Acquire) != epoch
    {
        MISSED.store(true, Ordering::Release);
        RECOVER.store(true, Ordering::Release);
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
            // A consumed recovery read is not retried every hour. The existing
            // daily census can admit a later stable baseline.
            RECOVER.store(true, Ordering::Release);
            MISSED.store(true, Ordering::Release);
        }
    }
}

fn observe(memory: &impl Memory, base: usize, pointer: usize, removed: bool, published: bool) {
    let Some(store) = STORE.get() else {
        MISSED.store(true, Ordering::Release);
        return;
    };
    let Ok(mut store) = store.try_lock() else {
        MISSED.store(true, Ordering::Release);
        return;
    };
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    let watched = store.watched.get(&pointer).copied();
    if !published && watched.is_none() {
        return;
    }
    let identity = match native::identity(memory, base, pointer) {
        Ok(id) => id,
        Err(reason) => {
            store.batch.failure = Some(reason);
            return;
        }
    };
    if let Some(watch) = watched {
        if watch.id != identity {
            store.batch.failure = Some("variant identity changed without observed replacement");
            return;
        }
    } else if published {
        if store.watched.len() == store.watched.capacity() {
            store.batch.failure = Some("variant change watch capacity exhausted");
            return;
        }
        store.watched.insert(
            pointer,
            Watch {
                id: identity,
                pointer,
                flags: [0; 3],
            },
        );
    }
    store.batch.required_events = store.batch.required_events.saturating_add(1);
    if store.batch.events.len() == store.batch.events.capacity() {
        store.batch.failure = Some("variant change event capacity exhausted");
        return;
    }
    let Some(state) = base
        .checked_add(engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|p| engine_native::read::<u64>(memory, p))
    else {
        store.batch.failure = Some("variant change clock unreadable");
        return;
    };
    let Some(hour) = (state as usize)
        .checked_add(engine_native::cgame_state::CURRENT_HOUR)
        .and_then(|p| engine_native::read(memory, p))
    else {
        store.batch.failure = Some("variant change hour unreadable");
        return;
    };
    if let Some(flags) = pointer
        .checked_add(native::CAN_UPGRADE_TYPE)
        .and_then(|p| engine_native::read::<[u8; 3]>(memory, p))
    {
        if let Some(watch) = store.watched.get_mut(&pointer) {
            watch.flags = flags;
        }
    } else {
        store.batch.failure = Some("variant change flags unreadable");
        return;
    }
    let start = store.batch.log.spans.len();
    let Store {
        batch, interests, ..
    } = &mut *store;
    match stage_interests(&mut batch.log, memory, base, pointer, Some(interests)) {
        Ok(()) => {
            let end = store.batch.log.spans.len();
            store.batch.events.push(Event {
                kind: Kind::Design,
                sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
                hour,
                pointer,
                variant: identity,
                flags: None,
                removed,
                start,
                end,
            });
        }
        Err(reason) => store.batch.failure = Some(reason),
    }
}

/// Destruction retires the prepared binding before any native fields die.
/// Drafts need one map lookup and no native reads or event payload.
fn retire(memory: &impl Memory, base: usize, pointer: usize) {
    let Some(store) = STORE.get() else {
        return;
    };
    let Ok(mut store) = store.try_lock() else {
        MISSED.store(true, Ordering::Release);
        return;
    };
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    retire_from(&mut store, memory, base, pointer);
}
fn retire_from(store: &mut Store, memory: &impl Memory, base: usize, pointer: usize) {
    let Some(watch) = store.watched.remove(&pointer) else {
        return;
    };
    store.batch.required_events = store.batch.required_events.saturating_add(1);
    if store.batch.events.len() == store.batch.events.capacity() {
        store.batch.failure = Some("variant removal event capacity exhausted");
        return;
    }
    let hour = base
        .checked_add(engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|address| engine_native::read::<u64>(memory, address))
        .and_then(|state| (state as usize).checked_add(engine_native::cgame_state::CURRENT_HOUR))
        .and_then(|address| engine_native::read::<i32>(memory, address));
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
        flags: None,
        removed: true,
        start: 0,
        end: 0,
    });
}

/// A complete interval is meaningful only with every mutation site installed
/// and no overlapping or lost callback. A full refresh cannot invent its past.
pub fn pending() -> bool {
    needs_full()
        || !enabled()
        || MISSED.load(Ordering::Acquire)
        || STORE.get().is_some_and(|s| {
            s.try_lock().map_or(true, |s| {
                !s.batch.events.is_empty() || s.batch.failure.is_some()
            })
        })
}
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst) + super::design_template_changes::active()
}

fn flag_change(memory: &impl Memory, base: usize, pointer: usize) {
    let Some(store) = STORE.get() else {
        MISSED.store(true, Ordering::Release);
        return;
    };
    let Ok(mut store) = store.try_lock() else {
        MISSED.store(true, Ordering::Release);
        return;
    };
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    let Some(watch) = store.watched.get(&pointer).copied() else {
        return;
    };
    let Some(flags) = pointer
        .checked_add(native::CAN_UPGRADE_TYPE)
        .and_then(|p| engine_native::read::<[u8; 3]>(memory, p))
    else {
        store.batch.failure = Some("variant eligibility flags unreadable");
        return;
    };
    if watch.flags == flags {
        return;
    }
    store.batch.required_events = store.batch.required_events.saturating_add(1);
    if store.batch.events.len() == store.batch.events.capacity() {
        store.batch.failure = Some("variant change event capacity exhausted");
        return;
    }
    let Some(state) = base
        .checked_add(engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|p| engine_native::read::<u64>(memory, p))
    else {
        store.batch.failure = Some("variant change clock unreadable");
        return;
    };
    let Some(hour) = (state as usize)
        .checked_add(engine_native::cgame_state::CURRENT_HOUR)
        .and_then(|p| engine_native::read(memory, p))
    else {
        store.batch.failure = Some("variant change hour unreadable");
        return;
    };
    let variant = watch.id;
    store.watched.get_mut(&pointer).unwrap().flags = flags;
    store.batch.events.push(Event {
        kind: Kind::Design,
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        hour,
        pointer,
        variant,
        flags: Some(flags),
        removed: false,
        start: 0,
        end: 0,
    });
}

use collector_hooks::hook::Hook;
use native::mutations::{FORWARDED_SITES, MUTATIONS, Receiver};
use std::{cell::Cell, ops::Range, sync::atomic::AtomicU32};
static BASE: AtomicUsize = AtomicUsize::new(0);
static OWNER: AtomicU32 = AtomicU32::new(0);
static ORIGINAL: [AtomicUsize; MUTATIONS.len()] = [const { AtomicUsize::new(0) }; MUTATIONS.len()];
thread_local! {
    static TARGET: Cell<usize> = const { Cell::new(0) };
    static CREATING: Cell<bool> = const { Cell::new(false) };
    static CREATED: Cell<usize> = const { Cell::new(0) };
}

/// Restore observation ownership even when the original unwinds through us.
/// An unwound writer has no completed snapshot and cannot certify carry.
struct CallbackScope {
    outer: bool,
    previous_target: usize,
    creation: Option<(bool, usize)>,
    completed: bool,
}
impl Drop for CallbackScope {
    fn drop(&mut self) {
        REVISION.fetch_add(1, Ordering::AcqRel);
        if !self.completed {
            MISSED.store(true, Ordering::Release);
        }
        if let Some((creating, created)) = self.creation {
            CREATING.set(creating);
            CREATED.set(created);
        }
        TARGET.set(self.previous_target);
        if self.outer {
            OWNER.store(0, Ordering::Release);
        }
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
    /// owns these sites, and must quiesce every callback before unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        BASE.store(base, Ordering::Release);
        let mut hooks = Vec::new();
        if native::mutations::WRITER_AUDIT_REFUSAL.is_none()
            && MUTATIONS.iter().all(|site| {
                base.checked_add(site.rva)
                    .and_then(|p| engine_native::read::<[u8; 16]>(memory, p))
                    == Some(site.sentinel)
            })
        {
            for (index, site) in MUTATIONS.iter().enumerate() {
                if FORWARDED_SITES.contains(&index) {
                    continue;
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks()[index], site.sentinel) }
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
        // Clone destinations are unpublished temporary storage. The wrapper
        // keeps both original operands intact; publication supplies its state.
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
                .and_then(|p| engine_native::read::<u64>(memory, p))
            else {
                MISSED.store(true, Ordering::Release);
                return 0;
            };
            match engine_native::creference_registry::resolve(memory, base, handle) {
                Ok(pointer) => pointer.unwrap_or(0),
                Err(_) => {
                    MISSED.store(true, Ordering::Release);
                    0
                }
            }
        }
        Receiver::Roster | Receiver::CreateCommand => 0,
    }
}

/// Derived refresh updates one ancestry owner's version ceiling. Intermediate
/// ancestors are only traversed by the game; they need no design snapshots.
fn version_owner(
    memory: &impl Memory,
    mut pointer: usize,
    population: usize,
) -> Result<usize, &'static str> {
    let origin = |pointer: usize| {
        pointer
            .checked_add(native::ORIGIN_LINK)
            .and_then(|p| engine_native::read::<i32>(memory, p))
            .ok_or("variant version origin unreadable")
    };
    let licensed = origin(pointer)? > 0;
    for _ in 0..=population {
        let parent = pointer
            .checked_add(native::PARENT)
            .and_then(|p| engine_native::read::<u64>(memory, p))
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
                MISSED.store(true, Ordering::Release);
            }
            CREATED.set(pointer);
        }
        Receiver::Roster => {
            let Some(array) = Array::read(memory, first, 8) else {
                MISSED.store(true, Ordering::Release);
                return;
            };
            for ordinal in 0..array.count {
                let Some(pointer) = array
                    .element(ordinal, 8)
                    .and_then(|p| engine_native::read::<u64>(memory, p))
                else {
                    MISSED.store(true, Ordering::Release);
                    return;
                };
                flag_change(memory, base, pointer as usize);
            }
        }
        Receiver::Removed => {}
        _ if pointer != 0 => {
            if let Some(field) = scalar::field(site) {
                scalar::observe(memory, base, pointer, field);
            } else if mutation.witness.is_some_and(|w| {
                (native::CAN_UPGRADE_TYPE..=native::CAN_UPGRADE_MODULES).contains(&w)
            }) {
                flag_change(memory, base, pointer);
            } else {
                observe(
                    memory,
                    base,
                    pointer,
                    false,
                    matches!(mutation.receiver, Receiver::Published),
                );
            }
        }
        _ => {}
    }
}

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

unsafe fn around<const SITE: usize>(
    args: [usize; 4],
    original: collector_observers::world_input_changes::Original,
) -> usize {
    unsafe { around_with_identities::<SITE>(args, original, None) }
}
unsafe fn around_with_identities<const SITE: usize>(
    [first, second, third, fourth]: [usize; 4],
    original: collector_observers::world_input_changes::Original,
    identities: Option<[Option<u64>; 2]>,
) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::VariantChanges);
    REVISION.fetch_add(1, Ordering::AcqRel);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    let outer = OWNER
        .compare_exchange(0, thread, Ordering::AcqRel, Ordering::Acquire)
        .is_ok();
    let admitted = outer || OWNER.load(Ordering::Acquire) == thread;
    if !admitted {
        MISSED.store(true, Ordering::Release);
    }
    let base = BASE.load(Ordering::Acquire);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let pointer = if admitted {
        target(&memory, base, MUTATIONS[SITE].receiver, first, second)
    } else {
        0
    };
    let previous = TARGET.replace(pointer);
    let create_scope = admitted && matches!(MUTATIONS[SITE].receiver, Receiver::CreateCommand);
    let previous_creating = if create_scope {
        CREATING.replace(true)
    } else {
        false
    };
    let previous_created = if create_scope { CREATED.replace(0) } else { 0 };
    let mut scope = CallbackScope {
        outer,
        previous_target: previous,
        creation: create_scope.then_some((previous_creating, previous_created)),
        completed: false,
    };
    let selected = admitted && (pointer == 0 || pointer != previous);
    if selected && matches!(MUTATIONS[SITE].receiver, Receiver::Removed) && pointer != 0 {
        retire(&memory, base, pointer);
    }
    let moving = MUTATIONS[SITE].name == "MoveAssign";
    let move_bindings = if admitted && moving {
        STORE.get().and_then(|store| {
            store.try_lock().ok().map(|store| {
                [
                    store.watched.get(&first).copied(),
                    store.watched.get(&second).copied(),
                ]
            })
        })
    } else {
        None
    };
    if admitted && moving && move_bindings.is_none() {
        MISSED.store(true, Ordering::Release);
    }
    if let (Some(bindings), Some(identities)) = (move_bindings, identities) {
        for (binding, identity) in bindings.into_iter().zip(identities) {
            if binding.is_some_and(|binding| Some(binding.id) != identity) {
                MISSED.store(true, Ordering::Release);
            }
        }
    }
    // Both old identities are retained before the consuming original runs.
    // The source is never read after that call, even if it is moved-from.
    if let Some(bindings) = move_bindings {
        for binding in bindings.into_iter().flatten() {
            retire(&memory, base, binding.pointer);
        }
    }
    let before = MUTATIONS[SITE]
        .witness
        .and_then(|offset| pointer.checked_add(offset))
        .and_then(|p| engine_native::read::<u16>(&memory, p));
    if MUTATIONS[SITE].witness.is_some() && before.is_none() {
        MISSED.store(true, Ordering::Release);
    }
    let forwarded = unsafe { forward::begin(SITE, pointer) };
    let moved_source = if admitted && moving && first != second {
        unsafe { forward::begin(SITE, second) }
    } else {
        None
    };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
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
                .and_then(|p| engine_native::read::<u16>(&memory, p));
            if after.is_none() {
                MISSED.store(true, Ordering::Release);
            }
            after != before
        });
    if let Some(bindings) = move_bindings {
        if bindings.iter().any(Option::is_some) {
            observe(&memory, base, first, false, true);
        }
    } else if selected && changed {
        returned(&memory, base, SITE, pointer, first);
    }
    // This writer can change a different saved design even when an enclosing
    // edit owns the current design's final snapshot, or the current object is
    // an excluded draft. Never suppress that shared ancestry write.
    if admitted && SITE == 1 && pointer != 0 {
        let count = STORE
            .get()
            .and_then(|s| s.try_lock().ok().map(|s| s.watched.len()))
            .unwrap_or(0);
        match version_owner(&memory, pointer, count) {
            Ok(owner) if owner != pointer => {
                scalar::observe(&memory, base, owner, scalar::Field::MaxVersion)
            }
            Ok(_) => {}
            Err(_) => MISSED.store(true, Ordering::Release),
        }
    }
    if create_scope {
        let created = CREATED.get();
        if created != 0 {
            observe(&memory, base, created, false, true);
        }
    }
    scope.completed = true;
    result
}

fn copy_rows<S: rows::TableSchema>(source: &[S::Row]) -> std::io::Result<Vec<S::Row>> {
    let mut result = engine_allocation::vec(source.len())?;
    for row in source {
        result.push(rows::RowCodec::try_clone(row)?);
    }
    Ok(result)
}

/// Only changed rows own storage. Membership changes and keyframes use the
/// complete path below; ordinary edits never clone unrelated design payloads.
fn replace<S: rows::TableSchema>(
    projection: Option<&Projection<'_>>,
    edits: Vec<(usize, S::Row)>,
) -> Result<Option<TableInput>, &'static str> {
    let Some((token, baseline)) =
        projection.and_then(Projection::observed_comparison_baseline::<S>)
    else {
        return Ok(None);
    };
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
    Ok(Some(if indices.is_empty() {
        TableInput::ComparedEqual(token)
    } else {
        TableInput::ComparedChanges {
            token,
            indices,
            rows: S::wrap(output),
        }
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
    history: &[rows::VariantChangeRow],
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
    for event in history {
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
    history: &[rows::VariantChangeRow],
    projection: Option<&Projection<'_>>,
    owner: impl Fn(&S::Row) -> u64,
    make: impl Fn(&rows::VariantSnapshot) -> std::io::Result<Vec<S::Row>>,
) -> TableInput {
    let result = (|| {
        if history.iter().all(|event| {
            event.organisation.is_some()
                || event.flags.is_some()
                || event.scalar.is_some()
                || event.sources.is_some()
        }) {
            return Ok(unavailable(replay_data::cadence::SKIPPED_REASON).into());
        }
        if let Some(input) = sparse::<S>(history, projection, &owner, &make)? {
            return Ok(input);
        }
        let baseline = projection
            .and_then(Projection::observed_baseline::<S>)
            .ok_or("variant changes have no complete preceding design state")?;
        let mut result =
            copy_rows::<S>(baseline).map_err(|_| "variant change projection allocation failed")?;
        for event in history {
            if event.organisation.is_some()
                || event.flags.is_some()
                || event.scalar.is_some()
                || event.sources.is_some()
            {
                continue;
            }
            result.retain(|row| owner(row) != event.variant);
            if !event.removed {
                let snapshot = event
                    .snapshot
                    .as_ref()
                    .ok_or("variant changes have an incomplete observation")?;
                let values =
                    make(snapshot).map_err(|_| "variant change projection allocation failed")?;
                engine_allocation::reserve(&mut result, values.len())
                    .map_err(|_| "variant change projection allocation failed")?;
                result.extend(values);
            }
        }
        result.sort_unstable_by_key(rows::RowCodec::key);
        Ok::<_, &'static str>(TableView::Complete(S::wrap(result)).into())
    })();
    result.unwrap_or_else(|reason| unavailable(reason).into())
}

pub(super) fn project(variants: &Captured, projection: Option<&Projection<'_>>) -> [TableInput; 4] {
    if let Outcome::Unavailable { reason } = variants.outcome {
        return std::array::from_fn(|_| unavailable(reason).into());
    }
    if variants.rows.is_empty() {
        return std::array::from_fn(|_| unavailable(replay_data::cadence::SKIPPED_REASON).into());
    }
    let history = &variants.rows;
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

pub(super) fn drain(
    base: usize,
    hour: i32,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<(), &'static str> {
    output.clear();
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
    let result = (|| {
        if !ENABLED.load(Ordering::Acquire) {
            return Err("variant change hooks unavailable");
        }
        if MISSED.swap(false, Ordering::AcqRel) || active() != 0 {
            return Err("variant change interval has an unobserved or running writer");
        }
        if let Some(reason) = batch.failure {
            return Err(reason);
        }
        decode_batch(&batch, base, hour, output)?;
        held::apply(&batch.events, output)?;
        sources::prune_interests()?;
        Ok(())
    })();
    if result.is_err() {
        withdraw_baselines();
        if !RECOVER.swap(true, Ordering::AcqRel) {
            RECOVERY_READ.store(true, Ordering::Release);
        }
    }
    let required_events = batch.required_events;
    batch.prepare(required_events, 0, 0);
    batch.clear();
    batch.log.grow();
    store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .spare = batch;
    result
}

fn project_definitions(
    history: &[rows::VariantChangeRow],
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
            for event in history {
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
                    let mut row = if let Ok(position) =
                        edits.binary_search_by_key(&index, |(index, _)| *index)
                    {
                        rows::RowCodec::try_clone(&edits[position].1)
                    } else {
                        rows::RowCodec::try_clone(&baseline[index])
                    }
                    .map_err(|_| "variant scalar projection allocation failed")?;
                    scalar
                        .apply_definition(&mut row)
                        .map_err(|_| "variant scalar projection failed")?;
                    edit(&mut edits, index, row)?;
                } else if let Some(sources) = &event.sources {
                    let mut row = rows::RowCodec::try_clone(&baseline[index])
                        .map_err(|_| "variant source change allocation failed")?;
                    if let Ok(position) = edits.binary_search_by_key(&index, |(index, _)| *index) {
                        row = rows::RowCodec::try_clone(&edits[position].1)
                            .map_err(|_| "variant source change allocation failed")?;
                    }
                    let state = row
                        .design
                        .require_mut("VariantDefinitions.design")
                        .map_err(|_| "variant source change lacks design state")?;
                    state.bonus_sources = engine_allocation::copy_vec(&sources.direct)
                        .map_err(|_| "variant source change allocation failed")?;
                    state.named_sources = engine_allocation::copy_vec(&sources.named)
                        .map_err(|_| "variant source change allocation failed")?;
                    edit(&mut edits, index, row)?;
                } else if let Some(flags) = event.flags {
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
        let mut result = copy_rows::<tables::VariantDefinitions>(baseline)
            .map_err(|_| "variant change allocation failed")?;
        for event in history {
            if event.organisation.is_some() {
                continue;
            }
            if let Some(scalar) = &event.scalar {
                let row = result
                    .iter_mut()
                    .find(|row| row.id == event.variant)
                    .ok_or("variant scalar names an unknown design")?;
                scalar
                    .apply_definition(row)
                    .map_err(|_| "variant scalar projection failed")?;
                continue;
            }
            if let Some(sources) = &event.sources {
                let row = result
                    .iter_mut()
                    .find(|r| r.id == event.variant)
                    .ok_or("variant sources name an unknown design")?;
                let state = row
                    .design
                    .require_mut("VariantDefinitions.design")
                    .map_err(|_| "variant source change lacks design state")?;
                state.bonus_sources = engine_allocation::copy_vec(&sources.direct)
                    .map_err(|_| "variant source change allocation failed")?;
                state.named_sources = engine_allocation::copy_vec(&sources.named)
                    .map_err(|_| "variant source change allocation failed")?;
                continue;
            }
            if let Some(flags) = event.flags {
                let row = result
                    .iter_mut()
                    .find(|r| r.id == event.variant)
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
            result.retain(|r| r.id != event.variant);
            if !event.removed {
                let snapshot = event
                    .snapshot
                    .as_ref()
                    .ok_or("variant changes have an incomplete observation")?;
                let definition = snapshot
                    .try_definition()
                    .map_err(|_| "variant change allocation failed")?;
                engine_allocation::reserve(&mut result, 1)
                    .map_err(|_| "variant change allocation failed")?;
                result.push(definition);
            }
        }
        result.sort_unstable_by_key(|r| r.id);
        Ok::<_, &'static str>(
            TableView::Complete(rows::TableRows::VariantDefinitions(result)).into(),
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

/// Owned worker output, loaned with the existing observer-boundary job.
pub struct Captured {
    rows: Vec<rows::VariantChangeRow>,
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
    /// Withdraw this frame's verdict without revoking the native certificate.
    /// Keep owned payloads until worker acquisition replaces them, so a reset
    /// never destroys a preceding hour's nested allocations on the game thread.
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
        self.rows.capacity() * std::mem::size_of::<rows::VariantChangeRow>() + self.heap_bytes
    }
    pub fn capture(&mut self, base: usize, hour: i32) {
        // Country accumulator admission is independent of the saved-design
        // observer. Its worker consumes only demanded, dirtied numeric owners.
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        self.fog = super::design_fog::capture(&memory, base);
        self.liveness = liveness::Observation::refused();
        let result = drain(base, hour, &mut self.rows).and_then(|_| {
            self.liveness = liveness::observation()?;
            self.dictionary = super::bonus_snapshots::snapshot()?;
            Ok(())
        });
        (self.context, self.templates) =
            super::design_context::capture(&memory, base).unwrap_or_default();
        self.context_acquired = true;
        self.outcome = match result {
            Ok(()) => Outcome::Rows {
                len: self.rows.len(),
            },
            Err(reason) => {
                self.rows.clear();
                Outcome::Unavailable { reason }
            }
        };
        self.heap_bytes = self.rows.iter().map(rows::RowCodec::heap_bytes).sum();
        publish_availability();
    }
    pub fn fail(&mut self, reason: &'static str) {
        // Failure can be posted by the callback when a worker loan is absent.
        // Retain nested payloads; only the worker replaces/drops their storage.
        self.outcome = Outcome::Unavailable { reason };
        self.liveness.refuse();
        self.context_acquired = false;
        boundary(true);
    }
    pub(super) fn table_input(&self, projection: Option<&Projection<'_>>) -> TableInput {
        if let Outcome::Unavailable { reason } = self.outcome {
            return unavailable(reason).into();
        }
        if self.rows.is_empty() {
            return TableView::Complete(rows::TableRows::VariantChanges(Vec::new())).into();
        }
        let result = (|| {
            let mut output = engine_allocation::vec(self.rows.len())?;
            let mut previous = std::collections::HashMap::new();
            engine_allocation::reserve_map(&mut previous, self.rows.len())?;
            for (index, row) in self.rows.iter().enumerate() {
                let preceding = previous
                    .get(&row.variant)
                    .map_or(&[][..], |&last| &self.rows[last..last + 1]);
                if !unchanged(row, preceding, projection) {
                    output.push(rows::RowCodec::try_clone(row)?);
                }
                previous.insert(row.variant, index);
            }
            Ok::<_, std::io::Error>(output)
        })();
        match result {
            Ok(rows) => TableView::Complete(rows::TableRows::VariantChanges(rows)).into(),
            Err(_) => unavailable("variant change projection allocation failed").into(),
        }
    }
}

fn unchanged(
    event: &rows::VariantChangeRow,
    preceding: &[rows::VariantChangeRow],
    projection: Option<&Projection<'_>>,
) -> bool {
    if event.removed {
        return false;
    }
    let Some(snapshot) = &event.snapshot else {
        return false;
    };
    if let Some(previous) = preceding
        .iter()
        .rev()
        .find(|row| row.variant == event.variant)
    {
        return !previous.removed && previous.snapshot.as_ref() == Some(snapshot);
    }
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
        // Runtime retains this owner whenever restoration or callback quiescence
        // is unproved. Otherwise no native entry can still name the buffers.
        if let Some(store) = STORE.get() {
            let mut store = store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *store = Store {
                batch: Batch::empty(),
                spare: Batch::empty(),
                watched: HashMap::new(),
                interests: sources::Interests::default(),
                epoch: EPOCH.load(Ordering::Acquire),
            };
        }
        super::bonus_snapshots::release();
        sources::release();
        liveness::release();
        held::release();
        super::design_context::invalidate();
    }
}

#[derive(serde::Serialize)]
pub struct Status {
    enabled: bool,
    writer_audit_refusal: Option<&'static str>,
    active: usize,
    needs_recovery: bool,
    missed: bool,
    next_sequence: u64,
    bonus_snapshots: super::bonus_snapshots::Status,
    country_context: super::design_context::Status,
    templates: super::design_template_changes::Status,
}
pub fn status() -> Status {
    Status {
        enabled: enabled(),
        writer_audit_refusal: native::mutations::WRITER_AUDIT_REFUSAL,
        active: active(),
        needs_recovery: RECOVER.load(Ordering::Acquire),
        missed: MISSED.load(Ordering::Acquire),
        next_sequence: SEQUENCE.load(Ordering::Acquire),
        bonus_snapshots: super::bonus_snapshots::status(),
        country_context: super::design_context::status(),
        templates: super::design_template_changes::status(),
    }
}

fn decode_batch(
    batch: &Batch,
    base: usize,
    hour: i32,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<(), &'static str> {
    let mut preceding_sequence = 0;
    for event in &batch.events {
        if event.sequence == 0 || event.sequence <= preceding_sequence {
            return Err("variant occurrences are not in native observation order");
        }
        preceding_sequence = event.sequence;
        if let Kind::Organisation(live) = event.kind {
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
            continue;
        }
        if let Kind::Scalar(field) = event.kind {
            let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end)?;
            let scalar = scalar::decode(&memory, event.pointer, field)?;
            engine_allocation::reserve(output, 1)
                .map_err(|_| "variant scalar allocation failed")?;
            output.push(rows::VariantChangeRow {
                sequence: OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                hour: event.hour,
                variant: event.variant,
                scalar: Some(scalar),
                ..Default::default()
            });
            continue;
        }
        if event.kind != Kind::Design {
            if matches!(event.kind, Kind::SourceRemoved | Kind::ProviderReset(_)) {
                sources::decode(
                    event,
                    &engine_native::memory::SliceMemory {
                        base: 0,
                        bytes: &[],
                    },
                    base,
                    output,
                )?;
            } else {
                let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end)?;
                sources::decode(event, &memory, base, output)?;
            }
            continue;
        }
        if let Some(flags) = event.flags {
            engine_allocation::reserve(output, 1)
                .map_err(|_| "variant change allocation failed")?;
            output.push(rows::VariantChangeRow {
                sequence: OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                hour: event.hour,
                variant: event.variant,
                flags: Some(flags),
                ..Default::default()
            });
            continue;
        }
        if event.removed {
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
            continue;
        }
        let memory = crate::raw::Recorded::build_range(&batch.log, event.start..event.end)?;
        let source = [native::RegistryRow::read(&memory, base, event.pointer)?];
        let mut captured = Variants::vacant();
        captured.capture_state(
            &memory,
            base,
            Sources {
                rows: &source,
                outcome: Outcome::Rows { len: 1 },
            },
            hour,
            true,
        );
        let definition = definitions(&captured.definitions, &captured.definition_bytes);
        let definition = match definition {
            TableView::Complete(rows::TableRows::VariantDefinitions(mut rows))
                if rows.len() == 1 =>
            {
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
        sources::track(&captured.definitions.rows[0])?;
        liveness::sample(&captured.definitions.rows[0])?;
        super::design_context::event(&memory, base, &captured, event.removed)?;
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
        engine_allocation::reserve(output, 1).map_err(|_| "variant change allocation failed")?;
        output.push(rows::VariantChangeRow {
            sequence: OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            hour: event.hour,
            variant: event.variant,
            flags: None,
            scalar: None,
            sources: None,
            organisation: None,
            removed: event.removed,
            snapshot: Some(snapshot),
            failure: Vec::new(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
