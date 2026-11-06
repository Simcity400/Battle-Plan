//! Writer-selected receiver sets. Hooks only mark already registered native objects. The existing
//! worker censuses admit new objects; quiet hours take the marked receivers without a world walk.
use collector_certificates::watch_registry::WatchRegistry;
use collector_hooks::hook::{Hook, Registers, Tap};
use collector_timing::hook_timing::{self, Family};
use engine_native::Memory;
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

static FORCES: WatchRegistry = WatchRegistry::marking();
static BASES: WatchRegistry = WatchRegistry::marking();
static ENABLED: AtomicBool = AtomicBool::new(false);
static AUTHENTICATION_REFUSALS: [AtomicU64; SITES.len() + METHODS.len()] =
    [const { AtomicU64::new(0) }; SITES.len() + METHODS.len()];
static CALLS: AtomicU64 = AtomicU64::new(0);
static PUBLICATION_ERROR: Mutex<Option<String>> = Mutex::new(None);
static REFUSED: AtomicU64 = AtomicU64::new(0);
static READ_REFUSALS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
pub(crate) fn read_refused(table: usize) {
    READ_REFUSALS[table].fetch_add(1, Ordering::Relaxed);
}

pub const UNAVAILABLE: &str = "naval repair writer observation unavailable";

/// Native patch inventory shared with the collector's cross-family ownership audit.
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
) -> &'static str {
    // Attach/detach failure only. Batch failures may apply to the whole publication,
    // so retain every candidate's name instead of assigning a guessed culprit.
    let names = names.collect::<Vec<_>>().join(", ");
    let message = format!("naval_management_changes {action} [{names}]: {reason}");
    *PUBLICATION_ERROR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.clone());
    Box::leak(message.into_boxed_str())
}

pub fn boundary(invalidate: bool) {
    FORCES.boundary(invalidate);
    BASES.boundary(invalidate);
}
pub fn active() -> usize {
    FORCES.active().saturating_add(BASES.active())
}
pub fn collect() {
    FORCES.collect();
    BASES.collect();
}
pub fn force_demand(entries: usize) {
    FORCES.offer(entries.saturating_mul(2));
}
pub fn base_demand(entries: usize) {
    BASES.offer(entries.saturating_mul(2));
}

static CENSUS_PENDING: AtomicBool = AtomicBool::new(false);

/// Live watch demand, or `None` before a load has made its game state and navy manager.
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
        .and_then(|at| engine_native::read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("naval repair base map unreadable")?;
    if manager == 0 {
        return Ok(None);
    }
    let countries = engine_native::ccountry::Reader::new(memory, base, state)?.count() as usize;
    let fleets = engine_native::creference_registry::entry_count(
        memory,
        base,
        engine_native::cfleet::HANDLE_TYPE,
    )?;
    let ships = engine_native::creference_registry::entry_count(
        memory,
        base,
        engine_native::cship::HANDLE_TYPE,
    )?;
    let refits = engine_native::creference_registry::entry_count(
        memory,
        base,
        engine_native::cship_refit_production_line::REFERENCE_TYPE,
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
        .checked_add(engine_native::rail_anchor_selector::MASK)
        .and_then(|at| engine_native::read::<u32>(memory, at))
        .and_then(|mask| (mask as usize).checked_add(1))
        .ok_or("naval repair base demand unreadable or overflow")?;
    Ok(Some((receivers, buckets)))
}

fn offer_census(receivers: usize, buckets: usize) {
    FORCES.offer(receivers);
    base_demand(buckets);
    CENSUS_PENDING.store(false, Ordering::Release);
}

/// Off the game thread: the first census for a collector attached before any load. A watch
/// table installed later only grows at an invalidating boundary, so none is offered until then.
pub fn presize(memory: &impl Memory, base: usize) {
    if !CENSUS_PENDING.load(Ordering::Acquire) {
        return;
    }
    let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native::read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok());
    if let Some(state) = state
        && let Ok(Some((receivers, buckets))) = census(memory, base, state)
    {
        offer_census(receivers, buckets);
    }
}

pub fn register_force(pointer: usize) -> bool {
    let available = FORCES.sequence(pointer).is_some() && FORCES.sequence(pointer | 1).is_some();
    if !available {
        REFUSED.fetch_add(1, Ordering::Relaxed);
    }
    available
}
pub fn register_base(pointer: usize) -> bool {
    let available = BASES.sequence(pointer).is_some() && BASES.sequence(pointer | 1).is_some();
    if !available {
        REFUSED.fetch_add(1, Ordering::Relaxed);
    }
    available
}
pub fn take_forces(visit: impl FnMut(usize)) -> Option<u64> {
    FORCES.take(visit)
}
pub fn take_bases(visit: impl FnMut(usize)) -> Option<u64> {
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
        // This selected store names both operands. Ignore an assignment that changes no bit;
        // in particular, empty ports' repeated zero allocations never reread their repair work.
        let value = write
            .value
            .map_or(write.literal, |value| register(registers, value) as u64);
        let Some(address) = receiver.checked_add(write.offset) else {
            return;
        };
        // SAFETY: the pinned native store is about to access this live object. Copies remain
        // guarded; an unreadable comparison invalidates the hold rather than hiding the write.
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let same = match write.bytes {
            1 => engine_native::read::<u8>(&memory, address).map(|old| old == value as u8),
            4 => engine_native::read::<u32>(&memory, address).map(|old| old == value as u32),
            8 => engine_native::read::<u64>(&memory, address).map(|old| old == value),
            _ => None,
        };
        if same == Some(true) {
            return;
        }
    }
    CALLS.fetch_add(1, Ordering::Relaxed);
    let registry = if site.base { &BASES } else { &FORCES };
    drop(registry.notify(receiver | usize::from(site.retire)));
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
    // Recalculation's initialization flag is the only branch that stores refit progress.
    let observes = site.rva != sites::REFIT_RECALCULATE.rva || second as u8 != 0;
    let notification = receiver
        .filter(|_| observes)
        .map(|pointer| registry.notify(pointer | usize::from(site.retire)));
    CALLS.fetch_add(1, Ordering::Relaxed);
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
    ) -> Result<Self, &'static str> {
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
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            self.observe(false);
        }
        let ready = Hook::set_enabled_iter_attach(
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
        // The runtime's normal restoration and range drain precede dropping this owner.
        unsafe {
            FORCES.release();
            BASES.release();
        }
    }
}

pub fn status() -> impl serde::Serialize {
    collector_status::status::status!({
        "enabled": enabled(),
        "publication_error": PUBLICATION_ERROR.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone(),
        "calls": CALLS.load(Ordering::Relaxed),
        "refused_registrations": REFUSED.load(Ordering::Relaxed),
        "force_takes": FORCES.takes(),
        "registered_force_receivers": FORCES.registered(),
        "registered_base_receivers": BASES.registered(),
        "base_takes": BASES.takes(),
        "sites": SITES.len() + METHODS.len(),
        "authentication_refusals": SITES.iter().map(|site| site.name).chain(METHODS.iter().map(|site| site.name))
            .zip(AUTHENTICATION_REFUSALS.iter().map(|count| count.load(Ordering::Relaxed)))
            .filter(|(_, count)| *count != 0).collect::<Vec<_>>(),
        "read_refusals": {
            "countries": READ_REFUSALS[0].load(Ordering::Relaxed),
            "fleets": READ_REFUSALS[1].load(Ordering::Relaxed),
            "task_forces": READ_REFUSALS[2].load(Ordering::Relaxed),
            "composition": READ_REFUSALS[3].load(Ordering::Relaxed),
            "refit_ships": READ_REFUSALS[4].load(Ordering::Relaxed),
            "refit_progress": READ_REFUSALS[5].load(Ordering::Relaxed),
            "repair_children": READ_REFUSALS[6].load(Ordering::Relaxed),
            "bases": READ_REFUSALS[7].load(Ordering::Relaxed),
            "queues": READ_REFUSALS[8].load(Ordering::Relaxed),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn naval_management_integration_failure_keeps_family_site_and_reason() {
        let _serial = fixture::SERIAL.lock().unwrap();
        let message = hook_failure(
            "publication failed",
            ["repair split command", "task destroy"].into_iter(),
            "synthetic verification refusal".into(),
        );
        assert!(message.contains("naval_management_changes"));
        assert!(message.contains("repair split command"));
        assert!(message.contains("task destroy"));
        assert!(message.contains("synthetic verification refusal"));
        assert_eq!(PUBLICATION_ERROR.lock().unwrap().as_deref(), Some(message));
        *PUBLICATION_ERROR.lock().unwrap() = None;
    }

    #[test]
    fn naval_management_integration_site_inventory_includes_independent_split() {
        assert_eq!(SITES.len(), 48);
        assert_eq!(METHODS.len(), 19);
        assert_eq!(SITES.len() + METHODS.len(), 67);
        assert_eq!(AUTHENTICATION_REFUSALS.len(), 67);
        assert!(
            SITES
                .iter()
                .any(|site| site.rva == sites::REPAIR_SPLIT_COMMAND.rva)
        );
        assert_eq!(
            sites::TASK_DESTROY.rva,
            engine_native::ctask_force::DESTRUCTOR_RVA
        );
        assert_eq!(
            METHODS
                .iter()
                .filter(|site| site.rva == engine_native::ctask_force::DESTRUCTOR_RVA)
                .count(),
            1
        );
    }

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
        // SAFETY: Registers contains only integer register values.
        let mut registers: Registers = unsafe { std::mem::zeroed() };
        registers.rdi = selected;
        for split in [1, 0] {
            registers.rax = split;
            // SAFETY: this command descriptor only marks its register receiver.
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
    fn naval_management_method_timing_excludes_its_native_original() {
        let _serial = fixture::SERIAL.lock().unwrap();
        let _sample = hook_timing::sample_all();
        unsafe extern "system-unwind" fn original(
            first: usize,
            _: usize,
            _: usize,
            _: usize,
        ) -> usize {
            std::hint::black_box(first) + 1
        }
        let before = hook_timing::counts(Family::NavalManagementChanges);
        let previous = ORIGINALS[0].swap(original as *const () as usize, Ordering::SeqCst);
        // SAFETY: the test owns this slot and its original has the method ABI.
        let result = unsafe { method::<0>(0x1000, 0, 0, 0) };
        ORIGINALS[0].store(previous, Ordering::SeqCst);
        assert_eq!(result, 0x1001);
        let after = hook_timing::counts(Family::NavalManagementChanges);
        assert_eq!(after.calls - before.calls, 1);
        assert_eq!(after.sampled - before.sampled, 1);
        assert!(after.original > before.original);
        assert_eq!(
            Family::NavalManagementChanges.name(),
            "naval_management_changes"
        );
        assert_eq!(
            Family::NavalManagementChanges.kind(),
            hook_timing::Kind::Mixed
        );
    }

    #[test]
    fn naval_management_writer_sites_are_distinct() {
        // Every patch replaces at least five bytes; tap spans are decoded from the located code.
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
        boundary(true);
    }
    pub fn mark(pointer: usize, base: bool, retire: bool) {
        drop(if base { &BASES } else { &FORCES }.notify(pointer | usize::from(retire)));
    }
    pub fn unavailable() {
        FORCES.set_enabled(false);
        BASES.set_enabled(false);
    }
}
