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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Before,
    After,
}

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
static FORWARDING: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
static OPEN: AtomicU32 = AtomicU32::new(0);
static GAME: AtomicUsize = AtomicUsize::new(0);
static HOUR: AtomicI32 = AtomicI32::new(0);
static KEY: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicU32 = AtomicU32::new(0);
static EQUIPMENT: AtomicU8 = AtomicU8::new(IDLE);
const IDLE: u8 = 0;
const RUNNING: u8 = 1;
const OBSERVED: u8 = 2;
static STRIKES: AtomicU64 = AtomicU64::new(0);
static REFUSED: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
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
pub fn presize(demand: usize) {
    if !foreign_test() {
        LEDGER.prepare(demand);
    }
}
/// # Safety
/// Caller must own the active session's recovery or publication failure path (or an isolated test
/// owning nuclear-strike observation) when invalidating nuclear-strike drains.
pub unsafe fn lost_drain() {
    if !foreign_test() {
        // SAFETY: Caller owns the nuclear-strike recovery or publication failure path.
        unsafe { LEDGER.lost_drain() };
    }
}
/// # Safety
/// Caller must own the active session's projection or publication failure path (or an isolated
/// test owning nuclear-strike observation) when recording lost nuclear-strike output.
pub unsafe fn lost_output(entries: usize) {
    if !foreign_test() {
        // SAFETY: Caller owns the nuclear-strike projection or publication failure path.
        unsafe { LEDGER.lost_output(entries) };
    }
}
/// # Safety
/// Caller must run on the admitted hourly capture boundary as the sole consumer advancing
/// nuclear-strike observation (or an isolated test owning nuclear-strike observation).
pub unsafe fn boundary(clock: Option<Clock>) {
    if !foreign_test() {
        // SAFETY: Caller runs on the admitted hourly capture boundary for nuclear-strike observation.
        unsafe { LEDGER.boundary(clock) };
    }
}
/// # Safety
/// Caller must be the admitted hourly capture consumer draining nuclear-strike events while
/// observer storage stays live and teardown is excluded.
pub unsafe fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    if foreign_test() || UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("nuclear strike hooks unavailable (see status unhooked)");
    }
    // SAFETY: Caller holds hourly capture admission as the sole nuclear-strikes consumer.
    unsafe { LEDGER.drain(output) }
}

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

#[must_use]
pub struct BuildingWatch {
    before: building_damage_sites::Snapshot,
}
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
    wing.checked_add(engine_native_air::cair_wing::ACQUIRED_MANPOWER)
        .and_then(|at| engine_native::read(memory, at))
        .ok_or("nuclear strike wing manpower unreadable")
}
fn crew(wing: usize) -> Result<Crew, &'static str> {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let vtable = engine_sites::at(
        BASE.load(Ordering::Relaxed),
        engine_native_air::cair_wing::VTABLE_RVA,
    )
    .ok_or("nuclear strike wing unauthenticated")?;
    if engine_native::read::<u64>(&memory, wing) != Some(vtable as u64) {
        return Err("nuclear strike wing unauthenticated");
    }
    Ok(Crew {
        wing,
        country: engine_events::equipment_destruction::air_owner(&memory, wing)?,
        before: crew_manpower(&memory, wing)?,
    })
}
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
            Err(reason) if phase == Phase::Before => refuse(Part::Units, reason),
            Err(_) => {}
        },
    );
    if let Err(reason) = walked {
        refuse(Part::Units, reason);
    }
}

type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;

#[doc = " Runs a site's original. The detours and originals use the unwinding ABI,"]
#[doc = " as operation_outcomes' do: a native exception crosses the Rust frame,"]
#[doc = " whose drops run, back into the native caller. The relocated prologue"]
#[doc = " executes no call, so no exception is raised while the trampoline is the"]
#[doc = " current frame; the original's own body unwinds by its function table."]
#[doc = " The flight count retires only on a normal return."]
fn run(site: usize, nuke: usize, province: usize, r8: usize, r9: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::NuclearStrikes);
    FLIGHTS.fetch_add(1, Ordering::SeqCst);
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(nuke, province, r8, r9) });
    FLIGHTS.fetch_sub(1, Ordering::SeqCst);
    result
}

struct Opened;
impl Drop for Opened {
    fn drop(&mut self) {
        close();
    }
}

unsafe extern "system-unwind" fn impact(
    nuke: usize,
    province: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    LEDGER.enter();
    open(nuke, province);
    let _opened = Opened;
    let result = run(0, nuke, province, r8, r9);
    LEDGER.leave();
    result
}
unsafe extern "system-unwind" fn notify(
    nuke: usize,
    province: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    close();
    run(1, nuke, province, r8, r9)
}
unsafe extern "system-unwind" fn unit_damage(
    nuke: usize,
    province: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
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


#[cfg(test)]
mod tests;
