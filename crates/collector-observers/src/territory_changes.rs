//! Allocation-free notifications from the native writers of every States and
//! Provinces row ([`engine_mutations::territory_mutations`]). A detour only marks the
//! ids its receiver names in the family's [`DirtySet`]s, counts its site and
//! calls the original; it never changes game behaviour.
//!
//! Three sets, each with at most one consumer:
//! - `STATES` (state ids), taken by the States job
//!   ([`crate::acquisition_states::StateStore`]): the owner, controller,
//!   reset and reader sites name a state through its guarded id
//!   `cstate::ID`; the state-name effects name none.
//! - `PROVINCES` (province ids): the province writers name a province
//!   through its guarded id `cprovince::ID`; the owner-area rebuild and
//!   `province initialization` name none.
//! - `PROVINCE_STATES` (state ids): the owner, reset, reader and
//!   border-conflict sites, whose writes reach the provinces linked to the
//!   state.
//!
//! - `MODIFIER_STATES` (state ids): the resource-gate refresh and state
//!   destruction, forwarded by the raid family that owns their patches
//!   ([`modifier_forwarder`]); they rebuild or drop a state's modifier
//!   accumulator and country hash, which a province's bunker term reads.
//!   It observes only while both families' hooks are enabled.
//!
//! [`Changes::take_provinces`] is the take of `PROVINCES` and `PROVINCE_STATES`; the
//! Provinces job drains it once an hour ([`crate::province_changes::take`])
//! for its row carry and the [area family](crate::area_changes), and sizes
//! the province set ([`walked_provinces`]). A receiver whose id does not
//! read marks no id and moves its set's epoch, so that consumer reads
//! everything. [`building_changed`] and
//! [`building_unnamed`] mark the locations of building writes, and [`poison`]
//! takes the map editor, whose patches the building family owns.
use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine_mutations::{territory_mutations::MUTATIONS, territory_mutations::Receiver};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static DOMAIN: Domain = Domain::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Per [`Table`]: reused and read rows, quiet hours (carried with no
/// marked id) and full reads.
static READS: [[AtomicU64; 4]; 2] = [const { [const { AtomicU64::new(0) }; 4] }; 2];
/// Slots of the state and province arrays the latest complete captures
/// walked, for off-thread sizing.
static SLOTS: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];

/// A table the family carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    States = 0,
    Provinces = 1,
}

/// One observation domain: the three dirty sets and each site's calls.
pub struct Domain {
    provinces: DirtySet,
    states: DirtySet,
    province_states: DirtySet,
    modifier_states: DirtySet,
    calls: [AtomicU64; SITES],
    /// Receivers whose id did not read.
    unnamed: AtomicU64,
    /// Forwarded building writes, and those naming no status.
    buildings: [AtomicU64; 2],
}

/// A running writer's marks; dropping them marks its return.
#[must_use]
pub struct Marks<'a>(#[allow(dead_code)] [Option<Dirty<'a>>; 2]);

impl Default for Domain {
    fn default() -> Self {
        Self::new()
    }
}

impl Domain {
    pub const fn new() -> Self {
        Self {
            provinces: DirtySet::new(),
            states: DirtySet::new(),
            province_states: DirtySet::new(),
            modifier_states: DirtySet::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            buildings: [const { AtomicU64::new(0) }; 2],
        }
    }

    fn sets(&self) -> [&DirtySet; 4] {
        [
            &self.provinces,
            &self.states,
            &self.province_states,
            &self.modifier_states,
        ]
    }

    /// Hook path, allocation-free: a forwarded writer of state `id`'s
    /// modifier accumulator and country hash is about to run.
    pub fn modifier_state(&self, id: Option<usize>) -> Dirty<'_> {
        self.mark(&self.modifier_states, id)
    }

    fn mark<'a>(&'a self, set: &'a DirtySet, id: Option<usize>) -> Dirty<'a> {
        match id {
            Some(id) => set.notify(id),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                set.notify_every()
            }
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// whose receiver `target` names is about to run.
    pub fn notify(&self, site: usize, target: Target) -> Marks<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        Marks(match target {
            Target::Province(id) => [Some(self.mark(&self.provinces, id)), None],
            Target::State {
                id,
                state,
                provinces,
            } => [
                state.then(|| self.mark(&self.states, id)),
                provinces.then(|| self.mark(&self.province_states, id)),
            ],
            Target::EveryProvince => [Some(self.provinces.notify_every()), None],
            Target::EveryState => [Some(self.states.notify_every()), None],
        })
    }

    /// A building writer of the status with `flags` and `location`
    /// ([`engine_native::cbuilding_status::FLAGS`], [`engine_native::cbuilding_status::ID`]).
    pub fn building(&self, flags: u32, location: u32) -> Marks<'_> {
        use engine_native::{cbuilding_status::PROVINCE_FLAG, cbuilding_status::STATE_FLAG};
        self.buildings[0].fetch_add(1, Ordering::Relaxed);
        let id = (location != 0).then_some(location as usize);
        Marks([
            (flags & PROVINCE_FLAG != 0).then(|| self.mark(&self.provinces, id)),
            (flags & STATE_FLAG != 0).then(|| self.mark(&self.states, id)),
        ])
    }

    /// A building writer naming no status: every province and state.
    pub fn building_unnamed(&self) -> Marks<'_> {
        self.buildings[1].fetch_add(1, Ordering::Relaxed);
        Marks([
            Some(self.provinces.notify_every()),
            Some(self.states.notify_every()),
        ])
    }

    fn poison(&self) {
        for set in self.sets() {
            set.poison();
        }
    }

    /// An enabled domain with sets for `states` and `provinces` ids
    /// installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(states: usize, provinces: usize) -> Self {
        let domain = Self::new();
        domain.provinces.offer(provinces);
        domain.states.offer(states);
        domain.province_states.offer(states);
        domain.modifier_states.offer(states);
        for set in domain.sets() {
            set.boundary(false);
            set.set_enabled(true);
        }
        domain
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        for set in self.sets() {
            set.boundary(invalidate);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.sets().iter().any(|set| set.poisoned())
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// The States job's take of the marked state ids ([`DirtySet::take`]).
    fn take_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// The takes of the marked province ids and of the state ids whose
    /// provinces are marked; both epochs, or `None` when either refuses.
    fn take_provinces(
        &self,
        provinces: &mut dyn FnMut(usize),
        states: &mut dyn FnMut(usize),
    ) -> Option<[u64; 2]>;
    /// The take of the state ids whose modifiers a forwarded writer rebuilt.
    fn take_modifier_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
}

impl Changes for Domain {
    fn take_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.states.take(visit)
    }
    fn take_provinces(
        &self,
        provinces: &mut dyn FnMut(usize),
        states: &mut dyn FnMut(usize),
    ) -> Option<[u64; 2]> {
        // Both take, so neither keeps bits a refused partner consumed.
        let provinces = self.provinces.take(provinces);
        let states = self.province_states.take(states);
        Some([provinces?, states?])
    }
    fn take_modifier_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.modifier_states.take(visit)
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn take_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take_states(visit)
    }
    fn take_provinces(
        &self,
        provinces: &mut dyn FnMut(usize),
        states: &mut dyn FnMut(usize),
    ) -> Option<[u64; 2]> {
        DOMAIN.take_provinces(provinces, states)
    }
    fn take_modifier_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take_modifier_states(visit)
    }
}

static SCRIPTED_VP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SCRIPTED_CONTROLLERS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Loader-only content demand: optional writer hooks are absent without consumers.
pub fn scripted_capture(victory_point: bool, controllers: bool) {
    SCRIPTED_VP.store(victory_point, Ordering::Relaxed);
    SCRIPTED_CONTROLLERS.store(controllers, Ordering::Relaxed);
}

/// What a site's first argument names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A province; `None` when its id does not read.
    Province(Option<usize>),
    /// A state and the rows its writes reach; `None` when its id does not
    /// read.
    State {
        id: Option<usize>,
        state: bool,
        provinces: bool,
    },
    EveryProvince,
    EveryState,
}

/// The guarded id at `object + offset`; zero, which no province or state
/// has, names none.
fn object_id(object: usize, offset: usize) -> Option<usize> {
    // SAFETY: a guarded read of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    object
        .checked_add(offset)
        .and_then(|address| engine_native::read::<u32>(&memory, address))
        .filter(|&id| id != 0)
        .map(|id| id as usize)
}

/// What `receiver` names given a site's first argument.
pub fn target(receiver: Receiver, first: usize) -> Target {
    match receiver {
        Receiver::Province => Target::Province(object_id(first, engine_native::cprovince::ID)),
        Receiver::State(rows) => Target::State {
            id: object_id(first, engine_native::cstate::ID),
            state: rows.state,
            provinces: rows.provinces,
        },
        Receiver::EveryProvince => Target::EveryProvince,
        Receiver::EveryState => Target::EveryState,
    }
}

/// Notifications running now.
pub fn active() -> usize {
    DOMAIN.sets().iter().map(|set| set.active()).sum()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    for set in DOMAIN.sets() {
        set.boundary(invalidate);
    }
}

/// Disable reuse of both tables for the attachment: a writer the family
/// does not observe ran (the map editor's state nudger, forwarded by the
/// family that owns its patch).
pub fn poison() {
    DOMAIN.poison();
}

/// Hook path, allocation-free: a building writer of the status with `flags` and `location`
/// (`cbuilding_status::FLAGS` and `ID`) is about to run. Marks the province when the flags'
/// province bit is set and the state when the state bit is, since a province's fort sum and a
/// state's infrastructure terms read its buildings. Hold the result over the native call.
pub fn building_changed(flags: u32, location: u32) -> Marks<'static> {
    DOMAIN.building(flags, location)
}

/// Hook path, allocation-free: a building writer whose status cannot be
/// named is about to run; every province and state is read afresh.
pub fn building_unnamed() -> Marks<'static> {
    DOMAIN.building_unnamed()
}

/// Hook path, allocation-free: a writer of state `id`'s modifier accumulator
/// and country hash, forwarded by its patch owner ([`crate::raid_changes`]).
/// Hold the result over the native call.
pub fn modifier_state(id: Option<usize>) -> Dirty<'static> {
    DOMAIN.modifier_state(id)
}

static HOOKS_ENABLED: AtomicBool = AtomicBool::new(false);
static MODIFIER_FORWARDER: AtomicBool = AtomicBool::new(false);

/// The raid family's forwarder of the modifier writers enabled or disabled.
pub fn modifier_forwarder(enabled: bool) {
    MODIFIER_FORWARDER.store(enabled, Ordering::SeqCst);
    observe_modifiers();
}

fn observe_modifiers() {
    DOMAIN.modifier_states.set_enabled(
        HOOKS_ENABLED.load(Ordering::SeqCst) && MODIFIER_FORWARDER.load(Ordering::SeqCst),
    );
}

/// Off the game thread: size the sets for the ids the latest captures
/// walked. Nothing is offered before a capture walked them: a later larger
/// bitmap waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    let [states, provinces] = SLOTS.each_ref().map(|slots| slots.load(Ordering::Relaxed));
    if states != 0 {
        DOMAIN.states.offer(states);
        DOMAIN.province_states.offer(states);
        DOMAIN.modifier_states.offer(states);
    }
    if provinces != 0 {
        DOMAIN.provinces.offer(provinces);
    }
}

/// Slots of the state array the latest complete States capture walked.
pub fn state_slots() -> usize {
    SLOTS[Table::States as usize].load(Ordering::Relaxed)
}

/// The Provinces job walked a complete province array of `slots` slots: the
/// next [`presize`] sizes the province set for the job's takes.
pub fn walked_provinces(slots: usize) {
    SLOTS[Table::Provinces as usize].store(slots, Ordering::Relaxed);
}

/// Off the game thread: free replaced bitmaps.
pub fn collect() {
    for set in DOMAIN.sets() {
        set.collect();
    }
}

/// One capture's rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    /// Carried with no marked id.
    pub quiet: bool,
    /// Every row read: no held capture certified any.
    pub full: bool,
}

/// A worker publishes one capture's counts and, when it walked the whole
/// table, its array's slots for sizing.
pub fn record(table: Table, counts: &Counts, slots: Option<usize>) {
    for (target, value) in READS[table as usize].iter().zip([
        counts.reused,
        counts.read,
        u64::from(counts.quiet),
        u64::from(counts.full),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(slots) = slots {
        SLOTS[table as usize].store(slots, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&DOMAIN.calls)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let table = |table: Table| {
        let reads = READS[table as usize]
            .each_ref()
            .map(|value| value.load(Ordering::Relaxed));
        crate::status::status!({
            "reused_rows": reads[0],
            "read_rows": reads[1],
            "quiet_hours": reads[2],
            "full_reads": reads[3],
        })
    };
    crate::status::status!({
        "configured": cfg!(feature = "territory_changes"),
        "poisoned": DOMAIN.sets().iter().any(|set| set.poisoned()),
        "states": table(Table::States),
        "provinces": table(Table::Provinces),
        "events": events,
        "unnamed_receivers": DOMAIN.unnamed.load(Ordering::Relaxed),
        "building_writes": DOMAIN.buildings[0].load(Ordering::Relaxed),
        "unnamed_building_writes": DOMAIN.buildings[1].load(Ordering::Relaxed),
        "province_set": DOMAIN.provinces.status(),
        "state_set": DOMAIN.states.status(),
        "province_state_set": DOMAIN.province_states.status(),
        "modifier_state_set": DOMAIN.modifier_states.status(),
        "scope": "cumulative since attach; a States row is carried when no hooked writer marked its state since the held complete capture in the same epoch and the state array header, slot pointers and shared infrastructure and continent operands are unchanged; every state's identity and manpower and StateFlags are read every hour; Provinces rows are carried by province_changes, whose status counts them"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; both tables then
    /// read every row afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "territory_changes")
            && MUTATIONS.iter().enumerate().all(|(index, (site, _))| {
                (index == 12 && !SCRIPTED_CONTROLLERS.load(Ordering::Relaxed))
                    || (index == 13 && !SCRIPTED_VP.load(Ordering::Relaxed))
                    || engine_native::read::<[u8; 16]>(memory, base + site.rva)
                        == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                province_controller as *const () as usize,
                victory_points as *const () as usize,
                read_province as *const () as usize,
                insert_border_conflict as *const () as usize,
                erase_border_conflict as *const () as usize,
                rebuild_owner_areas as *const () as usize,
                initialize_provinces as *const () as usize,
                state_owner_entry as *const () as usize,
                state_controller as *const () as usize,
                reset_state as *const () as usize,
                read_state as *const () as usize,
                state_name_effect as *const () as usize,
                recount_single_controller as *const () as usize,
                refresh_cached_victory_point as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                if (index == 12 && !SCRIPTED_CONTROLLERS.load(Ordering::Relaxed))
                    || (index == 13 && !SCRIPTED_VP.load(Ordering::Relaxed))
                {
                    continue;
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            crate::occupation_changes::forwarder(
                crate::occupation_changes::Forwarder::Territory,
                false,
            );
            HOOKS_ENABLED.store(false, Ordering::SeqCst);
            for set in DOMAIN.sets() {
                set.set_enabled(false);
            }
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Territory,
                false,
            );
            crate::event_feed_observer::forwarder(
                crate::event_feed_observer::Forwarder::Territory,
                false,
            );
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
        let [provinces, states, province_states, _] = DOMAIN.sets();
        for set in [provinces, states, province_states] {
            set.set_enabled(enabled && !self.hooks.is_empty());
        }
        HOOKS_ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe_modifiers();
        // The country counter inputs observe the state owner, controller
        // and reset sites here ([`crate::country_input_changes`]).
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Territory,
            enabled && !self.hooks.is_empty(),
        );
        // The event feed records each state's owner change through the
        // State owner setter detour  ([`state_owner`]).
        crate::event_feed_observer::forwarder(
            crate::event_feed_observer::Forwarder::Territory,
            enabled && !self.hooks.is_empty(),
        );
        crate::occupation_changes::forwarder(
            crate::occupation_changes::Forwarder::Territory,
            enabled && !self.hooks.is_empty(),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the bitmaps are dropped only once
        // the hooks were never enabled, or were removed and drained after
        // the last capture.
        for set in DOMAIN.sets() {
            unsafe { set.release() };
        }
    }
}

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the first names the receiver as
// its [`Receiver`] says. The marks cover the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::TerritoryChanges);
            let _occupation = (7..=11).contains(&$site).then(|| {
                (
                    crate::occupation_changes::changed(first),
                    crate::occupation_changes::changed(
                        first.wrapping_add(engine_native::cstate::COUNTRY_MODIFIERS),
                    ),
                )
            });
            let _marks = DOMAIN.notify($site, target(MUTATIONS[$site].1, first));
            // The state owner and controller setters and the state reset
            // also write the countries' state lists, which the country
            // counter inputs observe through this patch.
            let _inputs = crate::country_input_changes::forwarded_territory($site, first, second);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
// Province controller (province, tag, bool, bool).
callback!(province_controller, 0);
// Victory points (province, value).
callback!(victory_points, 1);
// Read province (province, reader, ...).
callback!(read_province, 2);
// Border-conflict insert (state, combat).
callback!(insert_border_conflict, 3);
// Border-conflict erase (state, combat).
callback!(erase_border_conflict, 4);
// Owner-area rebuild ().
callback!(rebuild_owner_areas, 5);
// Initialize provinces (state).
callback!(initialize_provinces, 6);
// State owner setter (state, tag, bool, bool), with its caller's
// return address for the event feed's state transfers
// ([`crate::war_provenance::state_owner_before`]).
#[unsafe(naked)]
unsafe extern "system-unwind" fn state_owner_entry() {
    core::arch::naked_asm!("mov rax, [rsp]", "jmp {shim}", shim = sym territory_state_owner_shim);
}
core::arch::global_asm!(
    ".text", ".globl territory_state_owner_shim", ".seh_proc territory_state_owner_shim",
    "territory_state_owner_shim:", "sub rsp, 0x38", ".seh_stackalloc 0x38", ".seh_endprologue",
    "mov [rsp + 0x20], rax", "call {body}", "add rsp, 0x38", "ret", ".seh_endproc",
    body = sym state_owner);
unsafe extern "system-unwind" {
    fn territory_state_owner_shim();
}
unsafe extern "system-unwind" fn state_owner(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
    caller: usize,
) -> usize {
    const SITE: usize = 7;
    let _occupation = (
        crate::occupation_changes::changed(first),
        crate::occupation_changes::changed(
            first.wrapping_add(engine_native::cstate::COUNTRY_MODIFIERS),
        ),
    );
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::TerritoryChanges);
    let _marks = DOMAIN.notify(SITE, target(MUTATIONS[SITE].1, first));
    let _inputs = crate::country_input_changes::forwarded_territory(SITE, first, second);
    let transfer = crate::war_provenance::state_owner_before(first, third, fourth, caller);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SITE].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    crate::war_provenance::state_owner_after(transfer);
    result
}
// State controller setter (state, tag, bool, bool).
callback!(state_controller, 8);
// State reset (state).
callback!(reset_state, 9);
// Read state (state, reader, token).
callback!(read_state, 10);
// State name set/reset effects(effect, scope).
callback!(state_name_effect, 11);
// Cached controller grouping publication (state, optional change result).
callback!(recount_single_controller, 12);
callback!(refresh_cached_victory_point, 13);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_mutations::territory_mutations::StateRows;

    fn take_states(domain: &Domain) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = domain.take_states(&mut |id| ids.push(id));
        (epoch, ids)
    }

    fn take_provinces(domain: &Domain) -> (Option<[u64; 2]>, Vec<usize>, Vec<usize>) {
        let (mut provinces, mut states) = (Vec::new(), Vec::new());
        let epochs = domain.take_provinces(&mut |id| provinces.push(id), &mut |id| states.push(id));
        (epochs, provinces, states)
    }

    /// Receivers name their ids through the live object: a province's id, a state's id; a zero or
    /// unreadable id names none.
    #[test]
    fn receivers_name_their_ids_through_the_live_object() {
        let mut province = vec![0_u8; 0x200];
        let mut state = vec![0_u8; 0x200];
        province[engine_native::cprovince::ID..][..4].copy_from_slice(&517_u32.to_le_bytes());
        state[engine_native::cstate::ID..][..4].copy_from_slice(&42_u32.to_le_bytes());
        let rows = StateRows {
            state: true,
            provinces: false,
        };
        assert_eq!(
            target(Receiver::Province, province.as_ptr() as usize),
            Target::Province(Some(517))
        );
        assert_eq!(
            target(Receiver::State(rows), state.as_ptr() as usize),
            Target::State {
                id: Some(42),
                state: true,
                provinces: false
            }
        );
        assert_eq!(
            target(Receiver::State(rows), province.as_ptr() as usize),
            Target::State {
                id: None,
                state: true,
                provinces: false
            }
        );
        assert_eq!(target(Receiver::Province, 0), Target::Province(None));
        assert_eq!(target(Receiver::EveryProvince, 0), Target::EveryProvince);
    }

    /// Each site marks exactly the sets its census entry names: province
    /// writers the province, state writers the state and/or its provinces,
    /// and receiverless writers every id of their table.
    #[test]
    fn each_site_marks_the_sets_its_writes_reach() {
        for (site, (mutation, receiver)) in MUTATIONS.iter().enumerate() {
            let name = mutation.name;
            let domain = Domain::enabled(64, 1024);
            let states = take_states(&domain).0.unwrap();
            let provinces = take_provinces(&domain).0.unwrap();
            let target = match receiver {
                Receiver::Province => Target::Province(Some(700)),
                Receiver::State(rows) => Target::State {
                    id: Some(9),
                    state: rows.state,
                    provinces: rows.provinces,
                },
                Receiver::EveryProvince => Target::EveryProvince,
                Receiver::EveryState => Target::EveryState,
            };
            drop(domain.notify(site, target));
            let (state_epoch, state_ids) = take_states(&domain);
            let (province_epochs, province_ids, province_states) = take_provinces(&domain);
            let province_epochs = province_epochs.unwrap();
            let (state_marked, province_marked, province_state_marked) = match receiver {
                Receiver::Province => (false, true, false),
                Receiver::State(rows) => (rows.state, false, rows.provinces),
                Receiver::EveryProvince => {
                    assert_eq!(state_epoch, Some(states), "{name}");
                    assert_ne!(province_epochs[0], provinces[0], "{name}");
                    assert_eq!(province_epochs[1], provinces[1], "{name}");
                    continue;
                }
                Receiver::EveryState => {
                    assert_ne!(state_epoch, Some(states), "{name}");
                    assert_eq!(province_epochs, provinces, "{name}");
                    continue;
                }
            };
            assert_eq!(state_epoch, Some(states), "{name}");
            assert_eq!(province_epochs, provinces, "{name}");
            assert_eq!(
                state_ids,
                if state_marked { vec![9] } else { vec![] },
                "{name}"
            );
            assert_eq!(
                province_ids,
                if province_marked { vec![700] } else { vec![] },
                "{name}"
            );
            assert_eq!(
                province_states,
                if province_state_marked {
                    vec![9]
                } else {
                    vec![]
                },
                "{name}"
            );
            assert_eq!(domain.calls[site].load(Ordering::Relaxed), 1, "{name}");
        }
    }

    /// An unreadable receiver id moves the epoch of every set its writes
    /// reach, so those consumers read everything.
    #[test]
    fn an_unnamed_receiver_revokes_its_sets() {
        let domain = Domain::enabled(64, 1024);
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        drop(domain.notify(
            7,
            Target::State {
                id: None,
                state: true,
                provinces: true,
            },
        ));
        assert_ne!(take_states(&domain).0, Some(states));
        let after = take_provinces(&domain).0.unwrap();
        assert_eq!(after[0], provinces[0]);
        assert_ne!(after[1], provinces[1]);
        assert_eq!(domain.unnamed.load(Ordering::Relaxed), 2);
    }

    /// A forwarded building write marks the province or state its status
    /// locates; one naming no status revokes both tables.
    #[test]
    fn building_writes_mark_their_location() {
        use engine_native::{cbuilding_status::PROVINCE_FLAG, cbuilding_status::STATE_FLAG};
        let domain = Domain::enabled(64, 1024);
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        drop(domain.building(PROVINCE_FLAG, 300));
        drop(domain.building(STATE_FLAG, 12));
        assert_eq!(take_states(&domain), (Some(states), vec![12]));
        assert_eq!(
            take_provinces(&domain),
            (Some(provinces), vec![300], vec![])
        );
        drop(domain.building_unnamed());
        assert_ne!(take_states(&domain).0, Some(states));
        assert_ne!(take_provinces(&domain).0.unwrap()[0], provinces[0]);
    }

    #[test]
    fn an_invalidating_boundary_revokes_every_consumer() {
        let domain = Domain::enabled(64, 1024);
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        domain.boundary(false);
        assert_eq!(take_states(&domain).0, Some(states));
        assert_eq!(take_provinces(&domain).0, Some(provinces));
        domain.boundary(true);
        assert_ne!(take_states(&domain).0, Some(states));
        let after = take_provinces(&domain).0.unwrap();
        assert!(after[0] != provinces[0] && after[1] != provinces[1]);
    }

    /// A forwarded modifier writer marks its state in the modifier set only;
    /// one whose state does not read moves that set's epoch.
    #[test]
    fn forwarded_modifier_writers_mark_only_the_modifier_set() {
        let domain = Domain::enabled(64, 1024);
        let take = |domain: &Domain| {
            let mut ids = Vec::new();
            let epoch = domain.take_modifier_states(&mut |id| ids.push(id));
            (epoch, ids)
        };
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        let modifiers = take(&domain).0.unwrap();
        drop(domain.modifier_state(Some(12)));
        assert_eq!(take(&domain), (Some(modifiers), vec![12]));
        assert_eq!(take_states(&domain), (Some(states), vec![]));
        assert_eq!(take_provinces(&domain), (Some(provinces), vec![], vec![]));
        drop(domain.modifier_state(None));
        assert_ne!(take(&domain).0, Some(modifiers));
        assert_eq!(take_states(&domain).0, Some(states));
    }

    #[test]
    fn poisoning_refuses_every_set() {
        let domain = Domain::enabled(64, 1024);
        domain.poison();
        assert!(domain.poisoned());
        assert_eq!(take_states(&domain).0, None);
        assert_eq!(take_provinces(&domain).0, None);
        assert_eq!(domain.take_modifier_states(&mut |_| ()), None);
    }
}
