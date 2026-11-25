use collector_certificates::watch_registry::WatchRegistry;
use collector_hooks::hook::{Hook, Registers, Tap};
use collector_timing::hook_timing::{self, Family};
use engine_native_memory::Memory;
use engine_sites::Rva;
use std::{
    ops::Range,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

mod sites;
use sites::{METHODS, Register, SITES};
static ORIGINALS: [AtomicUsize; METHODS.len()] = [const { AtomicUsize::new(0) }; METHODS.len()];

static FORCES: WatchRegistry = WatchRegistry::marks_only();
static BASES: WatchRegistry = WatchRegistry::marks_only();
static FORCES_REFUSED: AtomicBool = AtomicBool::new(false);
static BASES_REFUSED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static AUTHENTICATION_REFUSALS: [AtomicU64; SITES.len() + METHODS.len()] =
    [const { AtomicU64::new(0) }; SITES.len() + METHODS.len()];
static CALLS: hook_timing::CallCounters<1> = hook_timing::CallCounters::new();
static PUBLICATION_ERROR: Mutex<Option<String>> = Mutex::new(None);
static REFUSED: AtomicU64 = AtomicU64::new(0);
static READ_REFUSALS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
pub(crate) fn read_refused(table: usize) {
    READ_REFUSALS[table].fetch_add(1, Ordering::Relaxed);
}

pub const UNAVAILABLE: &str = "naval repairs could not be followed in the game";

pub fn patch_sites() -> impl Iterator<Item = (&'static str, Rva)> {
    SITES
        .iter()
        .map(|site| (site.name, site.rva))
        .chain(METHODS.iter().map(|site| (site.name, site.rva)))
}

fn hook_failure(
    action: &str,
    names: impl Iterator<Item = &'static str>,
    reason: String,
) -> String {
    let names = names.collect::<Vec<_>>().join(", ");
    let message = format!("naval_management_changes {action} [{names}]: {reason}");
    *PUBLICATION_ERROR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.clone());
    message
}

/// # Safety
/// Caller must run on the admitted hourly observer boundary while naval-management watch tables
/// stay live and teardown is excluded.
pub unsafe fn boundary(invalidate: bool) {
    renew(&FORCES, &FORCES_REFUSED, invalidate);
    renew(&BASES, &BASES_REFUSED, invalidate);
}
fn renew(registry: &WatchRegistry, refused: &AtomicBool, invalidate: bool) {
    let grow = refused.load(Ordering::Relaxed) && registry.growth_pending();
    if registry.boundary(invalidate || grow).renewed() {
        refused.store(false, Ordering::Relaxed);
    }
}
pub fn active() -> usize {
    FORCES.active().saturating_add(BASES.active())
}
pub fn collect() {
    FORCES.collect();
    BASES.collect();
}
pub(crate) fn force_demand(entries: usize) {
    FORCES.offer(entries.saturating_mul(2));
}
pub(crate) fn base_demand(entries: usize) {
    BASES.offer(entries.saturating_mul(2));
}

static CENSUS_PENDING: AtomicBool = AtomicBool::new(false);

fn census(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<Option<(usize, usize)>, &'static str> {
    if state == 0 {
        return Ok(None);
    }
    let manager = state
        .checked_add(engine_native::cgame_state::STRATEGIC_NAVY_MANAGER)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("naval repair base map unreadable")?;
    if manager == 0 {
        return Ok(None);
    }
    let countries = engine_native::ccountry::Reader::new(memory, base, state)?.count() as usize;
    let fleets = engine_native_parts::creference_registry::entry_count(
        memory,
        base,
        engine_native::cfleet::HANDLE_TYPE,
    )?;
    let ships = engine_native_parts::creference_registry::entry_count(
        memory,
        base,
        engine_native::cship::HANDLE_TYPE,
    )?;
    let refits = engine_native_parts::creference_registry::entry_count(
        memory,
        base,
        engine_native_parts::cship_refit_production_line::REFERENCE_TYPE,
    )?;
    let receivers = countries
        .checked_mul(3)
        .and_then(|value| {
            fleets
                .checked_mul(4)
                .and_then(|fleets| value.checked_add(fleets))
        })
        .and_then(|value| value.checked_add(ships))
        .and_then(|value| value.checked_add(refits))
        .and_then(|value| value.checked_mul(2))
        .ok_or("naval repair watch demand overflow")?;
    let buckets = manager
        .checked_add(engine_native_parts::rail_anchor_selector::MASK)
        .and_then(|at| engine_native_memory::read::<u32>(memory, at))
        .and_then(|mask| (mask as usize).checked_add(1))
        .ok_or("naval repair base demand unreadable or overflow")?;
    Ok(Some((receivers, buckets)))
}

fn offer_census(receivers: usize, buckets: usize) {
    FORCES.offer(receivers);
    base_demand(buckets);
    CENSUS_PENDING.store(false, Ordering::Release);
}

/// # Safety
/// Caller must run on the active session's sizing pass with a valid game `base` while
/// naval-management watch storage stays live and teardown is excluded.
pub unsafe fn presize(memory: &impl Memory, base: usize) {
    if !CENSUS_PENDING.load(Ordering::Acquire) {
        return;
    }
    let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok());
    if let Some(state) = state
        && let Ok(Some((receivers, buckets))) = census(memory, base, state)
    {
        offer_census(receivers, buckets);
    }
}

fn admit(registry: &WatchRegistry, refused: &AtomicBool, pointer: usize) -> bool {
    let available =
        registry.register(pointer).is_some() && registry.register(pointer | 1).is_some();
    if !available {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        refused.store(true, Ordering::Relaxed);
    }
    available
}
pub(crate) fn register_force(pointer: usize) -> bool {
    admit(&FORCES, &FORCES_REFUSED, pointer)
}
pub(crate) fn register_base(pointer: usize) -> bool {
    admit(&BASES, &BASES_REFUSED, pointer)
}
pub(crate) fn take_forces(visit: impl FnMut(usize)) -> Option<u64> {
    FORCES.take(visit)
}
pub(crate) fn take_bases(visit: impl FnMut(usize)) -> Option<u64> {
    BASES.take(visit)
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

fn register(registers: &Registers, register: Register) -> usize {
    match register {
        Register::Rax => registers.rax,
        Register::Rbx => registers.rbx,
        Register::Rcx => registers.rcx,
        Register::Rdx => registers.rdx,
        Register::Rbp => registers.rbp,
        Register::Rsi => registers.rsi,
        Register::Rdi => registers.rdi,
        Register::R11 => registers.r11,
        Register::R12 => registers.r12,
        Register::R13 => registers.r13,
        Register::R14 => registers.r14,
        Register::R15 => registers.r15,
    }
}
unsafe extern "system-unwind" fn changed(registers: &Registers, tag: usize) {
    let _timing = hook_timing::enter(Family::NavalManagementChanges);
    let site = &SITES[tag];
    let Some(receiver) = register(registers, site.register).checked_add_signed(site.adjustment)
    else {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if let Some(write) = &site.write {
        let value = write
            .value
            .map_or(write.literal, |value| register(registers, value) as u64);
        let Some(address) = receiver.checked_add(write.offset) else {
            return;
        };
        // SAFETY: the pinned native store is about to access this live object. Copies remain
        // guarded; an unreadable comparison invalidates the hold rather than hiding the write.
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let same = match write.bytes {
            1 => engine_native_memory::read::<u8>(&memory, address).map(|old| old == value as u8),
            4 => engine_native_memory::read::<u32>(&memory, address).map(|old| old == value as u32),
            8 => engine_native_memory::read::<u64>(&memory, address).map(|old| old == value),
            _ => None,
        };
        if same == Some(true) {
            return;
        }
    }
    CALLS.increment(0);
    let registry = if site.base { &BASES } else { &FORCES };
    registry.touch(receiver | usize::from(site.retire));
}

unsafe extern "system-unwind" fn method<const INDEX: usize>(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = hook_timing::enter(Family::NavalManagementChanges);
    let site = &METHODS[INDEX];
    let receiver =
        [first, second, third, fourth][site.argument].checked_add_signed(site.adjustment);
    let registry = if site.base { &BASES } else { &FORCES };
    let observes = site.rva != sites::REFIT_RECALCULATE.rva || second as u8 != 0;
    let notification = receiver
        .filter(|_| observes)
        .map(|pointer| registry.notify(pointer | usize::from(site.retire)));
    CALLS.increment(0);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINALS[INDEX].load(Ordering::Acquire)) };
    let value = timing.original(|| unsafe { original(first, second, third, fourth) });
    drop(notification);
    value
}

pub struct Hooks {
    taps: Vec<Tap>,
    methods: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The caller owns these sites in the authenticated executable and restores and quiesces
    /// every executable range before releasing them. Each descriptor identifies a complete
    /// stack-independent instruction span with no incoming interior branch or native return PC.
    pub unsafe fn new(
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<Self, String> {
        let mut taps = Vec::new();
        let mut methods = Vec::new();
        let mut located = [(0, [0; 16]); SITES.len() + METHODS.len()];
        let mut authenticated = true;
        for (index, (slot, rva)) in located
            .iter_mut()
            .zip(
                SITES
                    .iter()
                    .map(|site| site.rva)
                    .chain(METHODS.iter().map(|site| site.rva)),
            )
            .enumerate()
        {
            match engine_sites::hook(base, rva)
                .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
            {
                Ok(code) => *slot = code,
                Err(_) => {
                    AUTHENTICATION_REFUSALS[index].fetch_add(1, Ordering::Relaxed);
                    authenticated = false;
                }
            }
        }
        if !authenticated {
            return Ok(Self { taps, methods });
        }
        match census(memory, base, state)? {
            Some((receivers, buckets)) => offer_census(receivers, buckets),
            None => CENSUS_PENDING.store(true, Ordering::Release),
        }
        engine_allocation::reserve(&mut taps, SITES.len())
            .map_err(|_| "naval repair hook allocation failed")?;
        for (tag, (site, &(at, sentinel))) in SITES.iter().zip(&located).enumerate() {
            let tap =
                unsafe { Tap::new_owned_span(at, changed, tag, sentinel) }.map_err(|reason| {
                    hook_failure(
                        "tap construction failed",
                        std::iter::once(site.name),
                        reason,
                    )
                })?;
            taps.push(tap);
        }
        let callbacks: [usize; METHODS.len()] = [
            method::<0> as *const () as usize,
            method::<1> as *const () as usize,
            method::<2> as *const () as usize,
            method::<3> as *const () as usize,
            method::<4> as *const () as usize,
            method::<5> as *const () as usize,
            method::<6> as *const () as usize,
            method::<7> as *const () as usize,
            method::<8> as *const () as usize,
            method::<9> as *const () as usize,
            method::<10> as *const () as usize,
            method::<11> as *const () as usize,
            method::<12> as *const () as usize,
            method::<13> as *const () as usize,
            method::<14> as *const () as usize,
            method::<15> as *const () as usize,
            method::<16> as *const () as usize,
            method::<17> as *const () as usize,
            method::<18> as *const () as usize,
            method::<19> as *const () as usize,
        ];
        engine_allocation::reserve(&mut methods, METHODS.len())
            .map_err(|_| "naval repair method allocation failed")?;
        for (index, (site, &(at, sentinel))) in
            METHODS.iter().zip(&located[SITES.len()..]).enumerate()
        {
            let hook = unsafe { Hook::new(at, callbacks[index], sentinel) }.map_err(|reason| {
                hook_failure(
                    "method construction failed",
                    std::iter::once(site.name),
                    reason,
                )
            })?;
            ORIGINALS[index].store(hook.trampoline(), Ordering::Release);
            methods.push(hook);
        }
        Ok(Self { taps, methods })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            self.observe(false);
        }
        let ready = Hook::set_enabled_iter(
            self.taps
                .iter_mut()
                .map(Tap::publication_hook)
                .chain(self.methods.iter_mut()),
            enabled,
        )
        .map_err(|reason| {
            hook_failure(
                "publication failed",
                SITES
                    .iter()
                    .map(|site| site.name)
                    .chain(METHODS.iter().map(|site| site.name)),
                reason,
            )
        })?;
        if ready {
            self.observe(enabled && !self.taps.is_empty());
        }
        Ok(ready)
    }
    fn observe(&self, enabled: bool) {
        ENABLED.store(enabled, Ordering::SeqCst);
        FORCES.set_enabled(enabled);
        BASES.set_enabled(enabled);
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps
            .iter()
            .flat_map(Tap::executable_ranges)
            .chain(self.methods.iter().flat_map(Hook::executable_ranges))
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        CENSUS_PENDING.store(false, Ordering::Release);
        unsafe {
            FORCES.release();
            BASES.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_only_command_marks_its_task_force_without_a_mode_write() {
        let _serial = fixture::SERIAL.lock().unwrap();
        let _sample = hook_timing::sample_all();
        let before = hook_timing::counts(Family::NavalManagementChanges);
        fixture::prepare();
        let selected = 0x1000;
        let other = 0x2000;
        assert!(register_force(selected));
        assert!(register_force(other));
        assert!(register_base(selected));
        take_forces(|_| {}).unwrap();
        take_bases(|_| {}).unwrap();
        let tag = SITES
            .iter()
            .position(|site| site.rva == sites::REPAIR_SPLIT_COMMAND.rva)
            .expect("independent split writer must be installed");
        let mut registers: Registers = unsafe { std::mem::zeroed() };
        registers.rdi = selected;
        for split in [1, 0] {
            registers.rax = split;
            unsafe { changed(&registers, tag) };
            let mut marked = Vec::new();
            take_forces(|pointer| marked.push(pointer)).unwrap();
            assert_eq!(marked, [selected]);
            take_bases(|_| panic!("split must not dirty ports")).unwrap();
            take_forces(|_| panic!("quiet task must carry")).unwrap();
        }
        let after = hook_timing::counts(Family::NavalManagementChanges);
        assert_eq!(after.calls - before.calls, 2);
        assert_eq!(after.sampled - before.sampled, 2);
    }

    #[test]
    fn one_ship_erased_from_a_base_repair_queue_marks_that_base() {
        let _serial = fixture::SERIAL.lock().unwrap();
        fixture::prepare();
        let base = 0x3000;
        let other = 0x4000;
        assert!(register_base(base));
        assert!(register_base(other));
        take_bases(|_| {}).unwrap();
        take_forces(|_| {}).unwrap();
        let index = METHODS
            .iter()
            .position(|site| site.name == "base queue erase")
            .expect("one ship's erase from a base's repair queue is observed");
        assert_eq!(index, 19, "method::<19> is the erase's callback");
        assert!(METHODS[index].base && METHODS[index].argument == 0);
        unsafe extern "system-unwind" fn erase(base: usize, _: usize, _: usize, _: usize) -> usize {
            base
        }
        let previous = ORIGINALS[index].swap(erase as *const () as usize, Ordering::SeqCst);
        let result = unsafe { method::<19>(base, 0x5000, 0, 0) };
        ORIGINALS[index].store(previous, Ordering::SeqCst);
        assert_eq!(result, base);
        let mut marked = Vec::new();
        take_bases(|pointer| marked.push(pointer)).unwrap();
        assert_eq!(marked, [base]);
        take_forces(|_| panic!("a queue erase marks no task force")).unwrap();
    }

    #[test]
    fn a_refused_registration_takes_the_grown_table_at_the_next_hour() {
        let _serial = fixture::SERIAL.lock().unwrap();
        let receivers: Vec<usize> = (1..=4096).map(|index| index * 0x40).collect();
        for base in [false, true] {
            fixture::undersized();
            let (register, demand): (fn(usize) -> bool, fn(usize)) = if base {
                (register_base, base_demand)
            } else {
                (register_force, force_demand)
            };
            let refused = receivers.iter().filter(|&&pointer| !register(pointer)).count();
            assert!(refused > 0, "the smallest table refuses some of {} receivers", receivers.len());
            demand(receivers.len());
            // SAFETY: Unit test holds `fixture::SERIAL` as the sole naval-management owner.
            unsafe { boundary(false) };
            let refused = receivers.iter().filter(|&&pointer| !register(pointer)).count();
            assert_eq!(refused, 0, "base {base}: the grown table takes every receiver");
        }
        fixture::prepare();
    }

    #[test]
    fn naval_management_writer_sites_are_distinct() {
        let mut sites: Vec<_> = patch_sites().collect();
        sites.sort_unstable_by_key(|&(_, rva)| rva);
        for pair in sites.windows(2) {
            assert!(
                pair[0].1.0 + 5 <= pair[1].1.0,
                "{} overlaps {}",
                pair[0].0,
                pair[1].0
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    pub static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    pub fn prepare() {
        FORCES.offer(40_000);
        BASES.offer(10_000);
        FORCES.set_enabled(true);
        BASES.set_enabled(true);
        // SAFETY: Test fixture holds `fixture::SERIAL` as the sole naval-management owner.
        unsafe { boundary(true) };
    }
    pub fn undersized() {
        unsafe {
            FORCES.release();
            BASES.release();
        }
        FORCES.offer(1);
        BASES.offer(1);
        FORCES.set_enabled(true);
        BASES.set_enabled(true);
        // SAFETY: Test fixture holds `fixture::SERIAL` as the sole naval-management owner.
        unsafe { boundary(true) };
    }
    pub fn mark(pointer: usize, base: bool, retire: bool) {
        drop(if base { &BASES } else { &FORCES }.notify(pointer | usize::from(retire)));
    }
    pub fn unavailable() {
        FORCES.set_enabled(false);
        BASES.set_enabled(false);
    }
}
