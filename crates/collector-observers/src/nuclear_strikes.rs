//! Nuclear strikes as an hourly event ledger ([`engine_events::nuclear_strike_sites`]).
//!
//! The impact's entry opens a strike on its thread and records its launcher,
//! type and target; the notification's entry closes it, before the authored
//! `on_nuke_drop` effects run. While it is open, its outcome is recorded as
//! the native writers apply it:
//!
//! - the unit damage entry (this family's own site) reads each unit it
//!   reaches before and after the call, both under the call's number, so a
//!   failed read leaves only its own call's units unpaired;
//! - the families owning the other writers forward them on the open strike's
//!   thread: war support from [`crate::country_input_changes`] (add war
//!   support), building damage from [`crate::building_changes`] (destroy
//!   healthy levels, measured as [`engine_events::building_damage_sites`] measures
//!   bombing) and pool differences from [`crate::equipment_destruction`]
//!   (the land damage and aircraft loss writers).
//!
//! A part whose owner does not forward, whose read fails or whose call goes
//! unobserved is recorded as a refusal of that part. A strike whose own
//! operands cannot be read, a full ledger or overlapping calls make the
//! interval unavailable ([`EventLedger`]). Callbacks never allocate.
use crate::{
    event_ledger::{Event, EventLedger},
    hook::Hook,
};
use engine_events::{building_damage_sites, nuclear_strike_sites as native};
use engine_native::Memory;
use replay_data::rows::NuclearStrikePart as Part;
use std::{
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

type Clock = engine_native::cgame_state::Clock<u64>;

/// When a unit was read: before or after the damage call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Before,
    After,
}

/// One ledger entry: a strike, then what happened to it until the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Strike {
        hour: i32,
        strike: native::Strike,
    },
    WarSupport {
        country: u32,
        before: i64,
        after: i64,
    },
    Building {
        state: u32,
        province: Option<u32>,
        controller: u32,
        building_type: u32,
        amount_raw: i64,
    },
    Unit {
        phase: Phase,
        /// The open strike's unit damage call, counted from zero.
        call: u32,
        unit: native::Unit,
    },
    Equipment {
        wing: bool,
        country: u32,
        variant: u64,
        amount_raw: i64,
    },
    Crew {
        country: u32,
        before: i32,
        after: i32,
    },
    Refused {
        part: Part,
        reason: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub item: Item,
}
impl Default for Entry {
    fn default() -> Self {
        Self {
            id: 0,
            item: Item::Strike {
                hour: 0,
                strike: native::Strike::default(),
            },
        }
    }
}
impl Event for Entry {
    fn set_id(&mut self, id: u64) {
        self.id = id;
    }
}

static LEDGER: EventLedger<Entry> = EventLedger::new();
static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; native::SITES.len()] =
    [const { AtomicUsize::new(0) }; native::SITES.len()];
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
/// Whether the owner of each part's writer forwards it (indexed by part).
static FORWARDING: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
/// The thread whose strike is open; 0 when none is.
static OPEN: AtomicU32 = AtomicU32::new(0);
/// The open strike's game state, clock read and claim key.
static GAME: AtomicUsize = AtomicUsize::new(0);
static HOUR: AtomicI32 = AtomicI32::new(0);
static KEY: AtomicUsize = AtomicUsize::new(0);
/// The open strike's unit damage calls so far.
static CALLS: AtomicU32 = AtomicU32::new(0);
/// The forwarded equipment call: idle, running or observed.
static EQUIPMENT: AtomicU8 = AtomicU8::new(IDLE);
const IDLE: u8 = 0;
const RUNNING: u8 = 1;
const OBSERVED: u8 = 2;
static STRIKES: AtomicU64 = AtomicU64::new(0);
static REFUSED: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// Detour calls whose native original has not returned normally; one that a
/// foreign exception unwound stays counted, so detach keeps the collector.
static FLIGHTS: AtomicUsize = AtomicUsize::new(0);

pub fn active() -> usize {
    FLIGHTS.load(Ordering::SeqCst)
}
pub fn restore_sequence(previous: Option<u64>) -> Result<(), &'static str> {
    if ENABLED.load(Ordering::Acquire) {
        return Err("nuclear strike sequence restoration requires disabled hooks");
    }
    LEDGER.restore_sequence(previous)
}
/// Unit tests share these statics with every other collector test: only the
/// test that owns the observation drives the ledger.
fn foreign_test() -> bool {
    #[cfg(test)]
    if !tests::owns_observation() {
        return true;
    }
    false
}
pub fn mark_stranded() {
    if !foreign_test() {
        LEDGER.mark_stranded();
    }
}
pub fn capacity() -> usize {
    LEDGER.demand()
}
/// Off-thread: size the ledger for `demand` entries.
pub fn presize(demand: usize) {
    if !foreign_test() {
        LEDGER.prepare(demand);
    }
}
pub fn lost_drain() {
    if !foreign_test() {
        LEDGER.lost_drain();
    }
}
pub fn lost_output(entries: usize) {
    if !foreign_test() {
        LEDGER.lost_output(entries);
    }
}
pub fn boundary(clock: Option<Clock>) {
    if !foreign_test() {
        LEDGER.boundary(clock);
    }
}
pub fn drain_growing(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    if foreign_test() || UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("nuclear strike hooks unavailable (see status unhooked)");
    }
    LEDGER.drain_growing(output)
}

/// An owning family says whether it forwards `part`'s writer.
pub fn forwarder(part: Part, forwarding: bool) {
    FORWARDING[part as usize].store(forwarding, Ordering::SeqCst);
}

fn current() -> bool {
    let open = OPEN.load(Ordering::Acquire);
    open != 0 && open == unsafe { GetCurrentThreadId() }
}

fn record(item: Item) {
    LEDGER.attempt();
    if LEDGER.poisoned.load(Ordering::SeqCst) {
        return;
    }
    let result = LEDGER
        .claim((KEY.load(Ordering::Relaxed), 0))
        .and_then(|token| LEDGER.complete(token, Ok(Some(Entry { id: 0, item }))));
    if let Err(reason) = result {
        LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        LEDGER.refuse(reason);
    }
}

fn refuse(part: Part, reason: &'static str) {
    REFUSED[part as usize].fetch_add(1, Ordering::Relaxed);
    record(Item::Refused { part, reason });
}

fn game_operands(memory: &impl Memory, base: usize) -> Result<(usize, i32), &'static str> {
    let game = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native::read::<u64>(memory, at))
        .and_then(|game| usize::try_from(game).ok())
        .filter(|game| *game != 0)
        .ok_or("nuclear strike game state unreadable")?;
    let hour = game
        .checked_add(engine_native::cgame_state::CURRENT_HOUR)
        .and_then(|at| engine_native::read::<i32>(memory, at))
        .ok_or("nuclear strike clock unreadable")?;
    Ok((game, hour))
}

/// The impact's entry: record the strike and open it on this thread.
fn open(nuke: usize, province: usize) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    if LEDGER.poisoned.load(Ordering::SeqCst) || crate::allocation::failed() {
        LEDGER.attempt();
        LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let read = (|| {
        if crate::world_lifetime::invalidating() {
            return Err("nuclear strike native world awaits hourly boundary");
        }
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let (game, hour) = game_operands(&memory, base)?;
        Ok((
            game,
            hour,
            native::strike(&memory, base, game, nuke, province)?,
        ))
    })();
    let (game, hour, strike) = match read {
        Ok(read) => read,
        Err(reason) => {
            LEDGER.attempt();
            LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
            LEDGER.refuse(reason);
            return;
        }
    };
    GAME.store(game, Ordering::Relaxed);
    HOUR.store(hour, Ordering::Relaxed);
    KEY.store(nuke, Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    STRIKES.fetch_add(1, Ordering::Relaxed);
    record(Item::Strike { hour, strike });
    for (part, reason) in [
        (Part::WarSupport, "war support writer not forwarded"),
        (Part::Buildings, "building damage writer not forwarded"),
        (
            Part::Equipment,
            "equipment destruction writers not forwarded",
        ),
    ] {
        if !FORWARDING[part as usize].load(Ordering::SeqCst) {
            refuse(part, reason);
        }
    }
    OPEN.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
}

fn close() {
    if current() {
        OPEN.store(0, Ordering::Release);
    }
}

/// A forwarded `add_war_support` call on the open strike: the country's war
/// support before, recorded with its value after when dropped.
#[must_use]
pub struct WarSupportWatch {
    country: usize,
    tag: u32,
    before: i64,
}
fn war_support_field(memory: &impl Memory, country: usize) -> Result<i64, &'static str> {
    country
        .checked_add(engine_native::ccountry::WAR_SUPPORT)
        .and_then(|at| engine_native::read(memory, at))
        .ok_or("nuclear strike war support unreadable")
}
/// Hook path, allocation-free: add war support is about
/// to run on `country`.
pub fn war_support(country: usize) -> Option<WarSupportWatch> {
    if !current() {
        return None;
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let read = (|| {
        let vtable = engine_sites::at(
            BASE.load(Ordering::Relaxed),
            engine_native::ccountry::VTABLE_RVA,
        )
        .ok_or("nuclear strike war support country unauthenticated")?;
        if engine_native::read::<u64>(&memory, country) != Some(vtable as u64) {
            return Err("nuclear strike war support country unauthenticated");
        }
        let tag = country
            .checked_add(engine_native::ccountry::ID)
            .and_then(|at| engine_native::read(&memory, at))
            .ok_or("nuclear strike war support country unreadable")?;
        Ok((tag, war_support_field(&memory, country)?))
    })();
    match read {
        Ok((tag, before)) => Some(WarSupportWatch {
            country,
            tag,
            before,
        }),
        Err(reason) => {
            refuse(Part::WarSupport, reason);
            None
        }
    }
}
impl Drop for WarSupportWatch {
    fn drop(&mut self) {
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        match war_support_field(&memory, self.country) {
            Ok(after) => record(Item::WarSupport {
                country: self.tag,
                before: self.before,
                after,
            }),
            Err(reason) => refuse(Part::WarSupport, reason),
        }
    }
}

/// A forwarded `DestroyHealthyLevels` call on the open strike.
#[must_use]
pub struct BuildingWatch {
    before: building_damage_sites::Snapshot,
}
/// Hook path, allocation-free: destroy healthy levels
/// is about to run on `building` with damage `source`.
pub fn building(building: usize, source: usize) -> Option<BuildingWatch> {
    if !current() {
        return None;
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    match building_damage_sites::snapshot(
        &memory,
        BASE.load(Ordering::Relaxed),
        GAME.load(Ordering::Relaxed),
        building,
        source,
        0,
        HOUR.load(Ordering::Relaxed),
    ) {
        Ok(before) => Some(BuildingWatch { before }),
        Err(reason) => {
            refuse(Part::Buildings, reason);
            None
        }
    }
}
impl Drop for BuildingWatch {
    fn drop(&mut self) {
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let measured =
            building_damage_sites::applied_damage(&memory, base, &self.before).and_then(|amount| {
                if amount == 0 {
                    return Ok(None);
                }
                let at = building_damage_sites::attribution(&memory, base, &self.before)?;
                Ok(Some(Item::Building {
                    state: at.state,
                    province: at.province,
                    controller: at.victim,
                    building_type: at.building_type,
                    amount_raw: amount,
                }))
            });
        match measured {
            Ok(Some(item)) => record(item),
            Ok(None) => {}
            Err(reason) => refuse(Part::Buildings, reason),
        }
    }
}

/// A forwarded pool writer call on the open strike; dropped unobserved, it
/// refuses the strike's equipment. A wing's call also records the crews it
/// took.
#[must_use]
pub struct EquipmentCall {
    crew: Option<Crew>,
}
struct Crew {
    wing: usize,
    country: u32,
    before: i32,
}
fn crew_manpower(memory: &impl Memory, wing: usize) -> Result<i32, &'static str> {
    wing.checked_add(engine_native::cair_wing::ACQUIRED_MANPOWER)
        .and_then(|at| engine_native::read(memory, at))
        .ok_or("nuclear strike wing manpower unreadable")
}
fn crew(wing: usize) -> Result<Crew, &'static str> {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let vtable = engine_sites::at(
        BASE.load(Ordering::Relaxed),
        engine_native::cair_wing::VTABLE_RVA,
    )
    .ok_or("nuclear strike wing unauthenticated")?;
    if engine_native::read::<u64>(&memory, wing) != Some(vtable as u64) {
        return Err("nuclear strike wing unauthenticated");
    }
    Ok(Crew {
        wing,
        // Aircraft loss writer credits the casualties to this country.
        country: engine_events::equipment_destruction::air_owner(&memory, wing)?,
        before: crew_manpower(&memory, wing)?,
    })
}
/// Hook path: a pool writer that changes equipment is about to run: a
/// division's `active_damage`, or `aircraft loss writer` on `wing`.
pub fn equipment_call(wing: Option<usize>) -> Option<EquipmentCall> {
    if !current() {
        return None;
    }
    EQUIPMENT.store(RUNNING, Ordering::Relaxed);
    let crew = wing.and_then(|wing| {
        crew(wing)
            .inspect_err(|reason| refuse(Part::Equipment, reason))
            .ok()
    });
    Some(EquipmentCall { crew })
}
impl Drop for EquipmentCall {
    fn drop(&mut self) {
        if EQUIPMENT.swap(IDLE, Ordering::Relaxed) != OBSERVED {
            refuse(
                Part::Equipment,
                "equipment destruction did not observe a strike's pool writer",
            );
        }
        if let Some(crew) = &self.crew {
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            match crew_manpower(&memory, crew.wing) {
                Ok(after) => record(Item::Crew {
                    country: crew.country,
                    before: crew.before,
                    after,
                }),
                Err(reason) => refuse(Part::Equipment, reason),
            }
        }
    }
}
/// Hook path: one variant's signed pool difference of the running call.
pub fn equipment(wing: bool, country: u32, variant: u64, amount_raw: i64) {
    if current() && EQUIPMENT.load(Ordering::Relaxed) == RUNNING {
        record(Item::Equipment {
            wing,
            country,
            variant,
            amount_raw,
        });
    }
}
/// Hook path: the running call's pool differences were all forwarded.
pub fn equipment_observed() {
    if current() {
        let _ = EQUIPMENT.compare_exchange(RUNNING, OBSERVED, Ordering::Relaxed, Ordering::Relaxed);
    }
}

fn units(province: usize, phase: Phase, call: u32) {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let walked = native::units(
        &memory,
        BASE.load(Ordering::Relaxed),
        province,
        |unit| match unit {
            Ok(unit) => record(Item::Unit { phase, call, unit }),
            // The after read meets the same unit again; count it once.
            Err(reason) if phase == Phase::Before => refuse(Part::Units, reason),
            Err(_) => {}
        },
    );
    if let Err(reason) = walked {
        refuse(Part::Units, reason);
    }
}

type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;

/// Runs a site's original. The detours and originals use the unwinding ABI,
/// as operation_outcomes' do: a native exception crosses the Rust frame,
/// whose drops run, back into the native caller. The relocated prologue
/// executes no call, so no exception is raised while the trampoline is the
/// current frame; the original's own body unwinds by its function table.
/// The flight count retires only on a normal return.
fn run(site: usize, nuke: usize, province: usize, r8: usize, r9: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::NuclearStrikes);
    FLIGHTS.fetch_add(1, Ordering::SeqCst);
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(nuke, province, r8, r9) });
    FLIGHTS.fetch_sub(1, Ordering::SeqCst);
    result
}

/// Closes the strike when the impact returns, normally or unwinding, so no
/// later writer on its thread is taken for its outcome.
struct Opened;
impl Drop for Opened {
    fn drop(&mut self) {
        close();
    }
}

// Each site takes the nuke object and province in RCX and RDX and reads no
// other argument (`asm` at each entry); R8 and R9 are forwarded unchanged.
unsafe extern "system-unwind" fn impact(
    nuke: usize,
    province: usize,
    r8: usize,
    r9: usize,
) -> usize {
    LEDGER.enter();
    open(nuke, province);
    let _opened = Opened;
    let result = run(0, nuke, province, r8, r9);
    // The notification closed it already; an unwound call leaves the ledger
    // active, which strands its interval.
    LEDGER.leave();
    result
}
unsafe extern "system-unwind" fn notify(
    nuke: usize,
    province: usize,
    r8: usize,
    r9: usize,
) -> usize {
    close();
    run(1, nuke, province, r8, r9)
}
unsafe extern "system-unwind" fn unit_damage(
    nuke: usize,
    province: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let call = current().then(|| CALLS.fetch_add(1, Ordering::Relaxed));
    if let Some(call) = call {
        units(province, Phase::Before, call);
    }
    let result = run(2, nuke, province, r8, r9);
    if let Some(call) = call {
        units(province, Phase::After, call);
    }
    result
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction. A
    /// differing sentinel leaves the family uninstalled and its table
    /// unavailable.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Relaxed);
        let callbacks = [
            impact as *const () as usize,
            notify as *const () as usize,
            unit_damage as *const () as usize,
        ];
        let install = || -> Result<Vec<Hook>, &'static str> {
            for site in &native::SITES {
                crate::located(base, site.rva)?;
            }
            let mut hooks = Vec::with_capacity(native::SITES.len());
            for (index, site) in native::SITES.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| "nuclear strike hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            Ok(hooks)
        };
        match install() {
            Ok(hooks) => {
                INSTALLED.store(true, Ordering::Release);
                Self { hooks }
            }
            Err(reason) => {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    reason,
                );
                let _ = UNHOOKED.set(reason);
                Self { hooks: Vec::new() }
            }
        }
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
            .map_err(|_| "nuclear strike hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

pub fn status() -> impl serde::Serialize {
    let forwarding = FORWARDING
        .each_ref()
        .map(|flag| flag.load(Ordering::SeqCst));
    let refused = REFUSED
        .each_ref()
        .map(|count| count.load(Ordering::Relaxed));
    crate::status::status!({
        "installed": INSTALLED.load(Ordering::Acquire), "enabled": ENABLED.load(Ordering::SeqCst),
        "unhooked": UNHOOKED.get(), "active": active(), "ledger_active": LEDGER.active.load(Ordering::SeqCst),
        "stranded": LEDGER.stranded.load(Ordering::SeqCst),
        "branch_unavailable": LEDGER.poisoned.load(Ordering::SeqCst),
        "strikes": STRIKES.load(Ordering::Relaxed), "entries": LEDGER.events.load(Ordering::Relaxed),
        "forwarding_war_support": forwarding[Part::WarSupport as usize],
        "forwarding_buildings": forwarding[Part::Buildings as usize],
        "forwarding_equipment": forwarding[Part::Equipment as usize],
        "refused_war_support": refused[Part::WarSupport as usize],
        "refused_buildings": refused[Part::Buildings as usize],
        "refused_units": refused[Part::Units as usize],
        "refused_equipment": refused[Part::Equipment as usize],
        "overflow": LEDGER.overflow.load(Ordering::Relaxed), "refusals": LEDGER.refusals.load(Ordering::Relaxed),
        "lost_events": LEDGER.lost_events.load(Ordering::Relaxed), "lost_drains": LEDGER.lost_drains.load(Ordering::Relaxed),
        "lost_events_unknown": LEDGER.lost_events_unknown.load(Ordering::Relaxed),
        "overlaps": LEDGER.overlaps.load(Ordering::Relaxed), "drains": LEDGER.drains.load(Ordering::Relaxed),
        "handed_over": LEDGER.handed_over.load(Ordering::Relaxed),
        "entry_capacity": LEDGER.capacity.load(Ordering::Acquire), "entry_required": capacity(),
        "partial_intervals": LEDGER.partial_intervals.load(Ordering::Relaxed),
        "prepare_failures": LEDGER.prepare_failures.load(Ordering::Relaxed),
        "scope": "since attach; a strike's parts refused while open are named in its row; first/recovery partial intervals withheld"
    })
}

#[cfg(test)]
mod tests;
