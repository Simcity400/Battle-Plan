use crate::{
    hook::{Hook, Registers, Tap},
    hook_timing::{self, Family as Timing},
    lanes::{self, Refusal, Writer},
};
use engine_events::map_event_sites as site;
use engine_native_memory::{Memory, memory::LiveMemory};
use replay_data::{rows::MapEventFamily as Family, rows::MapEventFault as Fault};
use std::{
    cell::Cell,
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering},
    },
};

const N: usize = Family::COUNT;

static BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: [AtomicBool; N] = [const { AtomicBool::new(false) }; N];
static ENABLED: [AtomicBool; N] = [const { AtomicBool::new(false) }; N];
static FAULTS: [AtomicU16; N] = [const { AtomicU16::new(0) }; N];
static MISSED: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static EVENTS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static REJECTED: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static STRANDED: AtomicUsize = AtomicUsize::new(0);
static UNHOOKED: [OnceLock<&'static str>; N] = [const { OnceLock::new() }; N];
static FAKE_TARGET: AtomicUsize = AtomicUsize::new(0);
static GENERAL_TARGET: AtomicUsize = AtomicUsize::new(0);
static PARADROP_TARGET: AtomicUsize = AtomicUsize::new(0);
static SCRIPTED_REFRESH_TARGET: AtomicUsize = AtomicUsize::new(0);
static RAILWAY_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static RESISTANCE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BUILDING_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static LAST_REFUSAL: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

fn pack(reason: &'static str) -> u64 {
    let (address, len) = (reason.as_ptr() as u64, reason.len() as u64);
    if address >> 48 != 0 || len >> 16 != 0 {
        return 0;
    }
    address << 16 | len
}

fn unpack(word: u64) -> Option<&'static str> {
    (word != 0).then(|| unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(
            (word >> 16) as *const u8,
            (word & 0xffff) as usize,
        ))
    })
}

std::thread_local! {
    static UNWOUND: Cell<bool> = const { Cell::new(false) };
}

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

pub(crate) fn installed(family: Family) -> bool {
    INSTALLED[family.index()].load(Ordering::SeqCst)
}
pub(crate) fn enabled(family: Family) -> bool {
    ENABLED[family.index()].load(Ordering::SeqCst)
}
pub(crate) fn take_faults(family: Family) -> u16 {
    FAULTS[family.index()].swap(0, Ordering::SeqCst)
}
pub(crate) fn missed(family: Family) -> u64 {
    MISSED[family.index()].load(Ordering::SeqCst)
}
pub(crate) fn reset_missed() {
    for missed in &MISSED {
        missed.store(0, Ordering::SeqCst);
    }
}
pub(crate) fn fault(family: Family, fault: Fault) {
    FAULTS[family.index()].fetch_or(1 << fault as u8, Ordering::SeqCst);
}

fn refused(family: Family, refusal: Refusal) {
    let index = family.index();
    MISSED[index].fetch_add(1, Ordering::SeqCst);
    match refusal {
        Refusal::Staging => fault(family, Fault::Capacity),
        Refusal::Allocation => {
            LAST_REFUSAL[index].store(pack("map event spill allocation failed"), Ordering::Relaxed);
            fault(family, Fault::Capacity);
        }
        Refusal::Read(reason) => {
            LAST_REFUSAL[index].store(pack(reason), Ordering::Relaxed);
            fault(family, Fault::Read);
        }
    }
}

struct Call {
    family: Family,
    returned: bool,
}
impl Call {
    fn enter(family: Family) -> Self {
        if !UNWOUND.get() {
            ACTIVE.fetch_add(1, Ordering::SeqCst);
        }
        Self {
            family,
            returned: false,
        }
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if UNWOUND.get() {
            return;
        }
        if self.returned {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        } else {
            UNWOUND.set(true);
            STRANDED.fetch_add(1, Ordering::SeqCst);
            fault(self.family, Fault::Unwind);
        }
    }
}

fn hour(memory: &impl Memory, base: usize) -> Result<(usize, i32), &'static str> {
    let state = site::game_state(memory, base)?;
    Ok((state, site::hour(memory, state)?))
}

fn record(
    family: Family,
    read: impl FnOnce(&LiveMemory, usize, usize) -> Result<Event, &'static str>,
) {
    let world = crate::world_lifetime::generation();
    let memory = unsafe { LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let result =
        hour(&memory, base).and_then(|(state, hour)| Ok((hour, read(&memory, base, state)?)));
    let (hour, event) = match result {
        Ok(read) => read,
        Err(reason) => return refused(family, Refusal::Read(reason)),
    };
    let len = event.len();
    match publish_in_world(family, hour, world, len, |writer| {
        event.write(&memory, writer)
    }) {
        Ok(_) => {
            EVENTS[family.index()].fetch_add(1, Ordering::Relaxed);
        }
        Err(refusal) => refused(family, refusal),
    }
}

pub(crate) fn publish_in_world(
    family: Family,
    hour: i32,
    world: u64,
    len: usize,
    write: impl FnOnce(&mut Writer<'_>) -> Result<(), &'static str>,
) -> Result<u64, Refusal> {
    lanes::publish(family as u8, hour, world, len, |writer| {
        write(writer)?;
        if crate::world_lifetime::generation() != world {
            return Err("map event world changed while its operands were read");
        }
        Ok(())
    })
}

enum Event {
    Unit {
        owner: u32,
        anchor: u32,
        visibility: u32,
        fog: bool,
        intel: engine_native_memory::cpdx_array::Array,
    },
    Province(u32),
    ScriptedRefresh(u32),
    General {
        owner: u32,
        captor: u32,
        province: u32,
    },
    Paradrop {
        province: u32,
        country: u32,
        lost: bool,
        fog: bool,
    },
    Resistance {
        state: site::Resistance,
        key: (usize, usize),
    },
    Building {
        building: site::Building,
        reason: u8,
    },
}

impl Event {
    fn len(&self) -> usize {
        match self {
            Self::Unit { intel, .. } => 17 + intel.count as usize * 5,
            Self::Province(_) | Self::ScriptedRefresh(_) => 4,
            Self::General { .. } => 12,
            Self::Paradrop { .. } => 10,
            Self::Resistance { key, .. } => 16 + key.1,
            Self::Building { .. } => 13,
        }
    }
    fn write(&self, memory: &impl Memory, out: &mut Writer<'_>) -> Result<(), &'static str> {
        match *self {
            Self::Unit {
                owner,
                anchor,
                visibility,
                fog,
                intel,
            } => {
                out.u32(owner);
                out.u32(anchor);
                out.u32(visibility);
                out.u8(u8::from(fog));
                out.u32(intel.count);
                let mut entries = [(0_u32, 0_i8); 32];
                let mut start = 0;
                while start < intel.count as usize {
                    let batch = (intel.count as usize - start).min(entries.len());
                    site::intel_entries(memory, intel, start, &mut entries[..batch])?;
                    for (country, level) in &entries[..batch] {
                        out.u32(*country);
                        out.u8(*level as u8);
                    }
                    start += batch;
                }
            }
            Self::Province(province) | Self::ScriptedRefresh(province) => out.u32(province),
            Self::General {
                owner,
                captor,
                province,
            } => {
                out.u32(owner);
                out.u32(captor);
                out.u32(province);
            }
            Self::Paradrop {
                province,
                country,
                lost,
                fog,
            } => {
                out.u32(province);
                out.u32(country);
                out.u8(u8::from(lost));
                out.u8(u8::from(fog));
            }
            Self::Resistance { state, key } => {
                out.u32(state.province);
                out.u32(state.controller);
                out.u32(state.overlord);
                out.u32(key.1 as u32);
                let mut chunk = [0_u8; 64];
                let mut copied = 0;
                while copied < key.1 {
                    let len = (key.1 - copied).min(chunk.len());
                    if !engine_native_memory::read_slice(memory, key.0 + copied, &mut chunk[..len]) {
                        return Err("map event activity key unreadable");
                    }
                    out.put(&chunk[..len]);
                    copied += len;
                }
            }
            Self::Building { building, reason } => {
                out.u32(building.building_type);
                out.u32(building.province);
                out.u32(building.controller);
                out.u8(reason);
            }
        }
        Ok(())
    }
}

fn unit_event(
    memory: &LiveMemory,
    base: usize,
    state: usize,
    unit: usize,
    anchor: Option<usize>,
) -> Result<Event, &'static str> {
    let visibility = site::unit_province(memory, base, state, unit)?;
    Ok(Event::Unit {
        owner: site::unit_owner(memory, unit)?,
        anchor: match anchor {
            Some(province) => site::province_id(memory, province)?,
            None => visibility,
        },
        visibility,
        fog: site::fog(memory, base)?,
        intel: site::intel(memory, unit)?,
    })
}

type Native1 = unsafe extern "system-unwind" fn(usize) -> usize;
type NativeRefresh = unsafe extern "system-unwind" fn(usize, u8);
type Native4 = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;
type Native6 = unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize, usize) -> usize;

unsafe extern "system-unwind" fn fake_reveal(unit: usize) -> usize {
    let timing = hook_timing::enter(Timing::MapEvents);
    let mut call = Call::enter(Family::FakeDivisionRevealed);
    if enabled(Family::FakeDivisionRevealed) {
        record(Family::FakeDivisionRevealed, |memory, base, state| {
            unit_event(memory, base, state, unit, None)
        });
    }
    let target: Native1 = unsafe { std::mem::transmute(FAKE_TARGET.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { target(unit) });
    call.returned = true;
    result
}

unsafe extern "system-unwind" fn railway_overrun(
    gun: usize,
    from: usize,
    to: usize,
    r9: usize,
) -> usize {
    let timing = hook_timing::enter(Timing::MapEvents);
    let mut call = Call::enter(Family::RailwayGunOverrun);
    if enabled(Family::RailwayGunOverrun) {
        record(Family::RailwayGunOverrun, |memory, base, state| {
            unit_event(memory, base, state, gun, Some(to))
        });
    }
    let original: Native4 =
        unsafe { std::mem::transmute(RAILWAY_ORIGINAL.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(gun, from, to, r9) });
    call.returned = true;
    result
}

unsafe extern "system-unwind" fn critical_bombardment(registers: &Registers, _tag: usize) {
    let _timing = hook_timing::enter(Timing::MapEvents);
    if enabled(Family::CriticalShoreBombardment) {
        let province = registers.r13;
        record(Family::CriticalShoreBombardment, |memory, _, _| {
            site::province_id(memory, province).map(Event::Province)
        });
    }
}

unsafe extern "system-unwind" fn scripted_refresh(manager: usize, refresh_all: u8) {
    let timing = hook_timing::enter(Timing::MapEvents);
    let mut call = Call::enter(Family::ScriptedMapRefresh);
    if enabled(Family::ScriptedMapRefresh) {
        record(Family::ScriptedMapRefresh, |memory, base, _| {
            site::scripted_map_mode(memory, base).map(Event::ScriptedRefresh)
        });
    }
    let target: NativeRefresh =
        unsafe { std::mem::transmute(SCRIPTED_REFRESH_TARGET.load(Ordering::Acquire)) };
    timing.original(|| unsafe { target(manager, refresh_all) });
    call.returned = true;
}

unsafe extern "system-unwind" fn general_captured(
    leader: usize,
    captor: usize,
    province: usize,
    r9: usize,
) -> usize {
    let timing = hook_timing::enter(Timing::MapEvents);
    let mut call = Call::enter(Family::GeneralCaptured);
    if enabled(Family::GeneralCaptured) {
        if province == 0 {
            REJECTED[Family::GeneralCaptured.index()].fetch_add(1, Ordering::Relaxed);
        } else {
            record(Family::GeneralCaptured, |memory, _, _| {
                Ok(Event::General {
                    owner: site::general_owner(memory, leader)?,
                    captor: site::reference(memory, captor)?,
                    province: site::province_id(memory, province)?,
                })
            });
        }
    }
    let target: Native4 = unsafe { std::mem::transmute(GENERAL_TARGET.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { target(leader, captor, province, r9) });
    call.returned = true;
    result
}

unsafe extern "system-unwind" fn paradrop_landing(
    province: usize,
    country: usize,
    lost: usize,
    r9: usize,
) -> usize {
    let timing = hook_timing::enter(Timing::MapEvents);
    let mut call = Call::enter(Family::ParadropLanding);
    if enabled(Family::ParadropLanding) {
        record(Family::ParadropLanding, |memory, base, _| {
            Ok(Event::Paradrop {
                province: site::province_id(memory, province)?,
                country: site::reference(memory, country)?,
                lost: lost as u8 != 0,
                fog: site::fog(memory, base)?,
            })
        });
    }
    let target: Native4 = unsafe { std::mem::transmute(PARADROP_TARGET.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { target(province, country, lost, r9) });
    call.returned = true;
    result
}

unsafe extern "system-unwind" fn resistance_activity(
    resistance: usize,
    activity: usize,
    scope: usize,
    r9: usize,
) -> usize {
    let timing = hook_timing::enter(Timing::MapEvents);
    let mut call = Call::enter(Family::ResistanceActivity);
    if enabled(Family::ResistanceActivity) {
        record(Family::ResistanceActivity, |memory, base, state| {
            Ok(Event::Resistance {
                state: site::resistance(memory, base, state, resistance)?,
                key: site::activity_key(memory, activity)?,
            })
        });
    }
    let original: Native4 =
        unsafe { std::mem::transmute(RESISTANCE_ORIGINAL.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(resistance, activity, scope, r9) });
    call.returned = true;
    result
}

pub fn building_shows(client: u8, text: u8, reason: u32, before: i64, after: i64) -> bool {
    client != 0 && text != 0 && reason != 0 && after < before
}

unsafe extern "system-unwind" fn building_notification(
    building: usize,
    source: usize,
    level: usize,
    before: usize,
    client: usize,
    text: usize,
) -> usize {
    let timing = hook_timing::enter(Timing::MapEvents);
    let mut call = Call::enter(Family::BuildingDamage);
    if enabled(Family::BuildingDamage) {
        let memory = unsafe { LiveMemory::new() };
        let gate = (|| -> Result<Option<u8>, &'static str> {
            if client as u8 == 0 || text as u8 == 0 || source == 0 {
                return Ok(None);
            }
            let reason = engine_native_memory::read::<u32>(&memory, source)
                .ok_or("map event building reason unreadable")?;
            let after = site::level_health(&memory, building, level as u16 as i16)?;
            if !building_shows(client as u8, text as u8, reason, before as i64, after) {
                return Ok(None);
            }
            u8::try_from(reason)
                .ok()
                .filter(|reason| (1..=5).contains(reason))
                .map(Some)
                .ok_or("map event building reason unknown")
        })();
        match gate {
            Ok(Some(reason)) => record(Family::BuildingDamage, |memory, _, state| {
                Ok(Event::Building {
                    building: site::building(memory, state, building)?,
                    reason,
                })
            }),
            Ok(None) => {
                REJECTED[Family::BuildingDamage.index()].fetch_add(1, Ordering::Relaxed);
            }
            Err(reason) => refused(Family::BuildingDamage, Refusal::Read(reason)),
        }
    }
    let original: Native6 =
        unsafe { std::mem::transmute(BUILDING_ORIGINAL.load(Ordering::Acquire)) };
    let result =
        timing.original(|| unsafe { original(building, source, level, before, client, text) });
    call.returned = true;
    result
}

pub struct Hooks {
    hooks: Vec<(Family, Hook)>,
    taps: Vec<(Family, Tap)>,
}

impl Hooks {
    /// Construct every family whose site authenticates; a family whose
    /// site is withheld stays unavailable and the others install.
    ///
    /// # Safety
    /// The memory source and base identify the authenticated executable in
    /// this process. The caller owns these sites exclusively and must restore
    /// them, wait for [`active`] to drain, and confirm that no thread is still
    /// executing in the hooks' code ranges before releasing code: a thread can
    /// be in a relay before it counts itself in [`active`].
    pub unsafe fn new(
        _memory: &impl Memory,
        base: usize,
        selected: impl Fn(Family) -> bool,
    ) -> Self {
        BASE.store(base, Ordering::Relaxed);
        let mut hooks = Vec::new();
        let mut taps = Vec::new();
        for family in Family::ALL.into_iter().filter(|family| !selected(*family)) {
            let _ = UNHOOKED[family.index()].set("map event family disabled by attach config");
        }
        let located = |family: Family, key| match engine_sites::hook(base, key)
            .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
        {
            Ok(located) => Some(located),
            Err(cause) => {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Sentinel,
                    None,
                    cause,
                );
                let _ = UNHOOKED[family.index()].set(cause);
                None
            }
        };
        let call_target = |site: usize, original: [u8; 16]| {
            let displacement = i32::from_le_bytes(original[1..5].try_into().unwrap());
            (site + 5).wrapping_add_signed(displacement as isize)
        };
        let mut add = |family: Family, built: Result<Hook, String>| match built {
            Ok(hook) => {
                INSTALLED[family.index()].store(true, Ordering::SeqCst);
                hooks.push((family, hook));
            }
            Err(_) => {
                let _ = UNHOOKED[family.index()].set("map event hook construction failed");
            }
        };
        for (family, key, callback, target) in [
            (
                Family::FakeDivisionRevealed,
                site::FAKE_REVEAL_VISIBILITY_CALL_RVA,
                fake_reveal as *const () as usize,
                &FAKE_TARGET,
            ),
            (
                Family::GeneralCaptured,
                site::GENERAL_CAPTURE_CALL_RVA,
                general_captured as *const () as usize,
                &GENERAL_TARGET,
            ),
            (
                Family::ParadropLanding,
                site::PARADROP_LANDING_CALL_RVA,
                paradrop_landing as *const () as usize,
                &PARADROP_TARGET,
            ),
        ] {
            if !selected(family) {
                continue;
            }
            let Some((at, original)) = located(family, key) else {
                continue;
            };
            target.store(call_target(at, original), Ordering::Release);
            add(family, unsafe { Hook::new_call(at, callback, original) });
        }
        for (family, key, callback, trampoline) in [
            (
                Family::RailwayGunOverrun,
                site::RAILWAY_OVERRUN_RVA,
                railway_overrun as *const () as usize,
                &RAILWAY_ORIGINAL,
            ),
            (
                Family::ResistanceActivity,
                site::RESISTANCE_ACTIVITY_RVA,
                resistance_activity as *const () as usize,
                &RESISTANCE_ORIGINAL,
            ),
            (
                Family::BuildingDamage,
                site::BUILDING_NOTIFICATION_RVA,
                building_notification as *const () as usize,
                &BUILDING_ORIGINAL,
            ),
        ] {
            if !selected(family) {
                continue;
            }
            let Some((at, original)) = located(family, key) else {
                continue;
            };
            let built = unsafe { Hook::new(at, callback, original) };
            if let Ok(hook) = &built {
                trampoline.store(hook.trampoline(), Ordering::Release);
            }
            add(family, built);
        }
        let family = Family::CriticalShoreBombardment;
        if selected(family)
            && let Some((at, original)) = located(family, site::CRITICAL_BOMBARDMENT_TAP_RVA)
        {
            match unsafe { Tap::new(at, critical_bombardment, 0, original) } {
                Ok(tap) => {
                    INSTALLED[family.index()].store(true, Ordering::SeqCst);
                    taps.push((family, tap));
                }
                Err(_) => {
                    let _ = UNHOOKED[family.index()].set("map event tap construction failed");
                }
            }
        }
        let family = Family::ScriptedMapRefresh;
        if selected(family)
            && let Some((at, original)) = located(family, site::SCRIPTED_REFRESH_CALL_RVA)
        {
            SCRIPTED_REFRESH_TARGET.store(call_target(at, original), Ordering::Release);
            match unsafe { Hook::new_call(at, scripted_refresh as *const () as usize, original) } {
                Ok(hook) => {
                    INSTALLED[family.index()].store(true, Ordering::SeqCst);
                    hooks.push((family, hook));
                }
                Err(_) => {
                    let _ = UNHOOKED[family.index()]
                        .set("scripted map refresh call hook construction failed");
                }
            }
        }
        Self { hooks, taps }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            for flag in &ENABLED {
                flag.store(false, Ordering::SeqCst);
            }
            for family in Family::ALL {
                fault(family, Fault::Disabled);
            }
        }
        let ready = Hook::set_enabled_iter(
            self.hooks
                .iter_mut()
                .map(|(_, hook)| hook)
                .chain(self.taps.iter_mut().map(|(_, tap)| tap.publication_hook())),
            enabled,
        )
        .map_err(|_| "map event hook state change failed")?;
        if ready && enabled {
            for family in self
                .hooks
                .iter()
                .map(|(family, _)| family)
                .chain(self.taps.iter().map(|(family, _)| family))
            {
                ENABLED[family.index()].store(true, Ordering::SeqCst);
            }
        }
        Ok(ready)
    }

    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(|(_, hook)| hook.executable_ranges())
            .chain(
                self.taps
                    .iter()
                    .flat_map(|(_, tap)| tap.executable_ranges()),
            )
            .collect()
    }
}

pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "active": active(),
        "stranded": STRANDED.load(Ordering::SeqCst),
        "families": crate::status::map(Family::ALL.into_iter().map(|family| {
            let index = family.index();
            (family.name(), crate::status::status!({
                "installed": INSTALLED[index].load(Ordering::SeqCst),
                "enabled": ENABLED[index].load(Ordering::SeqCst),
                "unhooked": UNHOOKED[index].get(),
                "events": EVENTS[index].load(Ordering::Relaxed),
                "rejected": REJECTED[index].load(Ordering::Relaxed),
                "missed": MISSED[index].load(Ordering::Relaxed),
                "last_refusal": unpack(LAST_REFUSAL[index].load(Ordering::Relaxed)),
            }))
        })),
        "lanes": lanes::capacity(),
        "allocated_bytes": lanes::allocated_bytes(),
        "scope": "since attach: events published, rejected by the game's own gates, and refused (missed, which marks the interval incomplete)",
    })
}

#[cfg(test)]
pub(crate) fn set_for_tests(family: Family, installed: bool, enabled: bool) {
    INSTALLED[family.index()].store(installed, Ordering::SeqCst);
    ENABLED[family.index()].store(enabled, Ordering::SeqCst);
}
#[cfg(test)]
pub(crate) fn set_base_for_tests(base: usize) {
    BASE.store(base, Ordering::SeqCst);
}
#[cfg(test)]
pub(crate) fn set_original_for_tests(family: Family, original: usize) {
    let slot = match family {
        Family::FakeDivisionRevealed => &FAKE_TARGET,
        Family::RailwayGunOverrun => &RAILWAY_ORIGINAL,
        Family::GeneralCaptured => &GENERAL_TARGET,
        Family::ParadropLanding => &PARADROP_TARGET,
        Family::ResistanceActivity => &RESISTANCE_ORIGINAL,
        Family::BuildingDamage => &BUILDING_ORIGINAL,
        Family::ScriptedMapRefresh => &SCRIPTED_REFRESH_TARGET,
        Family::CriticalShoreBombardment => return,
    };
    slot.store(original, Ordering::SeqCst);
}
#[cfg(test)]
pub(crate) unsafe fn building_notification_for_tests(
    building: usize,
    source: usize,
    level: usize,
    before: usize,
    client: usize,
    text: usize,
) -> usize {
    unsafe { building_notification(building, source, level, before, client, text) }
}
#[cfg(test)]
pub(crate) fn refuse_for_tests(family: Family, refusal: Refusal) {
    refused(family, refusal);
}
#[cfg(test)]
pub(crate) unsafe fn fake_reveal_for_tests(unit: usize) -> usize {
    unsafe { fake_reveal(unit) }
}
#[cfg(test)]
pub(crate) unsafe fn scripted_refresh_for_tests(manager: usize, refresh_all: u8) {
    unsafe { scripted_refresh(manager, refresh_all) };
}
