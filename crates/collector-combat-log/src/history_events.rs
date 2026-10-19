//! Native retained-history event producer. Hooks never allocate, wait, or
//! follow unrelated histories. The single worker drains into reusable storage.
use collector_hooks::hook::{Hook, Registers, Tap};
use collector_primitives::shared_lane::SharedLane;
use engine_land_inputs::{combat_log_events as native, combat_log_sites as sites};
use engine_native::Memory;
use std::{
    cell::Cell,
    ops::Range,
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering::SeqCst},
    },
};

#[derive(Clone, Copy, Default)]
pub struct Logged {
    pub order: u64,
    pub signal: i64,
    pub fact: native::Fact,
}
type Lane = SharedLane<Logged>;
static LANE: AtomicPtr<Lane> = AtomicPtr::new(ptr::null_mut());
static STORAGE: Mutex<Option<Box<Lane>>> = Mutex::new(None);
static DRAIN_SCRATCH: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static HANDOFF: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static BASE: AtomicUsize = AtomicUsize::new(0);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static REVISION: AtomicU64 = AtomicU64::new(0);
static ORDER: AtomicU64 = AtomicU64::new(1);
static LOST: AtomicU64 = AtomicU64::new(0);
static REQUIRED: AtomicUsize = AtomicUsize::new(0);
static ESCAPED: AtomicBool = AtomicBool::new(false);
static EXHAUSTED: AtomicBool = AtomicBool::new(false);
static FIRST_LOSS: OnceLock<&'static str> = OnceLock::new();
static ORIGINAL: [AtomicUsize; 13] = [const { AtomicUsize::new(0) }; 13];
static UNHOOKED: OnceLock<String> = OnceLock::new();

pub fn fail(reason: &'static str) {
    let _ = LOST.fetch_update(SeqCst, SeqCst, |n| Some(n.saturating_add(1)));
    let _ = FIRST_LOSS.set(reason);
}
pub fn losses() -> u64 {
    LOST.load(SeqCst)
}
pub fn quiet() -> Option<u64> {
    let revision = REVISION.load(SeqCst);
    (observing() && ACTIVE.load(SeqCst) == 0).then_some(revision)
}
pub fn stop() {
    ENABLED.store(false, SeqCst);
}
pub fn retention_required() -> bool {
    ESCAPED.load(SeqCst)
}
pub fn release_storage() -> Result<(), &'static str> {
    if ENABLED.load(SeqCst) || ACTIVE.load(SeqCst) != 0 || retention_required() {
        return Err("combat log event storage release before quiescence");
    }
    let mut storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    LANE.store(ptr::null_mut(), SeqCst);
    drop(storage.take());
    *DRAIN_SCRATCH.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
    CAPACITY.store(0, SeqCst);
    crate::combat_log::release_retained_history();
    Ok(())
}
fn observing() -> bool {
    ENABLED.load(SeqCst) && !ESCAPED.load(SeqCst) && !EXHAUSTED.load(SeqCst)
}
fn advance(counter: &AtomicU64) -> u64 {
    // Stay below the publication lane's mark bits as well as integer wrap.
    match counter.fetch_update(SeqCst, SeqCst, |n| {
        n.checked_add(1).filter(|next| *next < (1 << 61))
    }) {
        Ok(n) => n,
        Err(_) => {
            EXHAUSTED.store(true, SeqCst);
            fail("combat log event lifetime counter exhausted");
            u64::MAX
        }
    }
}

thread_local! { static SIGNAL: Cell<Option<i64>> = const { Cell::new(None) }; }
struct Call {
    finished: bool,
    previous_signal: Option<i64>,
}
impl Call {
    fn enter() -> Self {
        ACTIVE.fetch_add(1, SeqCst);
        advance(&REVISION);
        let previous_signal = SIGNAL.get();
        SIGNAL.set(if observing() {
            previous_signal.or_else(signal)
        } else {
            None
        });
        Self {
            finished: false,
            previous_signal,
        }
    }
    fn finish(mut self) {
        self.finished = true;
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if !self.finished {
            ESCAPED.store(true, SeqCst);
            fail("combat log native writer escaped unwind");
        }
        SIGNAL.set(self.previous_signal);
        ACTIVE.fetch_sub(1, SeqCst);
    }
}
fn memory() -> engine_native::memory::LiveMemory {
    unsafe { engine_native::memory::LiveMemory::new() }
}
fn signal() -> Option<i64> {
    engine_native::read(
        &memory(),
        BASE.load(SeqCst)
            .checked_add(engine_native::cgame_state::PLAY_SESSION_STARTED_AT_RVA)?,
    )
}
fn emit(fact: native::Fact) {
    if HANDOFF.load(SeqCst) {
        fail("combat log event storage handoff overlapped writer");
        return;
    }
    // ACTIVE excludes reclamation; HANDOFF is tested after entering the call.
    let Some(lane) = (unsafe { LANE.load(SeqCst).as_ref() }) else {
        REQUIRED.fetch_add(1, SeqCst);
        fail("combat log event storage not prepared");
        return;
    };
    let Some(signal) = SIGNAL.get() else {
        fail("combat log event session unreadable");
        return;
    };
    let order = advance(&ORDER);
    if EXHAUSTED.load(SeqCst) {
        return;
    }
    if !publish(
        lane,
        Logged {
            order,
            signal,
            fact,
        },
    ) {
        REQUIRED.fetch_max(lane.required_entries(), SeqCst);
        fail("combat log event capacity exceeded");
    }
}
fn publish(lane: &Lane, entry: Logged) -> bool {
    let Some(reservation) = lane.reserve() else {
        return false;
    };
    reservation.publish(entry);
    true
}
fn observe(result: Result<(), &'static str>) {
    if let Err(reason) = result {
        fail(reason);
    }
}
fn equipment_enabled() -> bool {
    match BASE
        .load(SeqCst)
        .checked_add(engine_dlc::dlc_feature_state::MASK_RVA)
        .and_then(|p| engine_native::read::<u32>(&memory(), p))
    {
        Some(mask) => mask & engine_dlc::dlc_feature_state::EQUIPMENT_LOSSES_MASK != 0,
        None => {
            fail("combat log equipment feature unreadable");
            false
        }
    }
}

/// Worker only. Demand comes from the initial existing group walk and attempted
/// events, including every refused element. No game census is performed here.
pub fn prepare(demand: usize) {
    REQUIRED.fetch_max(demand, SeqCst);
    if REQUIRED.load(SeqCst) <= CAPACITY.load(SeqCst) {
        return;
    }
    let mut storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let required = REQUIRED
        .load(SeqCst)
        .max(storage.as_ref().map_or(0, |s| s.required_entries()));
    if required == 0 || storage.as_ref().is_some_and(|s| s.len() >= required) {
        return;
    }
    let Ok(next) = Lane::new(required, 0).and_then(engine_allocation::boxed) else {
        return;
    };
    HANDOFF.store(true, SeqCst);
    if ACTIVE.load(SeqCst) == 0 && storage.as_ref().is_none_or(|s| s.is_drained()) {
        let mut next = next;
        LANE.store(&mut *next, SeqCst);
        CAPACITY.store(next.len(), SeqCst);
        *storage = Some(next);
    }
    HANDOFF.store(false, SeqCst);
}

/// A copy never follows a game pointer. Refused intervals remain unavailable
/// until an explicit full baseline; callers must not turn loss into polling.
pub fn drain(output: &mut Vec<Logged>) -> Result<(u64, u64), &'static str> {
    output.clear();
    let storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let before = REVISION.load(SeqCst);
    if ACTIVE.load(SeqCst) != 0 {
        return Err("combat log writer active at drain");
    }
    let Some(lane) = storage.as_ref() else {
        return Err("combat log event storage not prepared");
    };
    let rejected = lane.rejected();
    let mut entries = DRAIN_SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    lane.published_into(&mut entries, |r| r.order)?;
    engine_allocation::reserve(output, entries.len())
        .map_err(|_| "combat log event drain allocation failed")?;
    for &(_, index) in entries.iter() {
        output.push(lane.record(index).0);
        lane.release(index);
    }
    lane.advance(rejected);
    if ACTIVE.load(SeqCst) != 0 || before != REVISION.load(SeqCst) {
        fail("combat log writer overlapped drain");
    }
    if !observing() || UNHOOKED.get().is_some() {
        return Err("combat log writers not observed throughout");
    }
    Ok((LOST.load(SeqCst), before))
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({"lost": LOST.load(SeqCst), "first_loss": FIRST_LOSS.get(), "unhooked": UNHOOKED.get(), "required_entries": REQUIRED.load(SeqCst), "escaped_unwind": ESCAPED.load(SeqCst), "scope": "retained combat log writer occurrences since attach; any loss revokes the retained-history certificate"})
}

// Only the group's creation append reaches this call site. Existing lookup
// hits incur no callback, country read, count read or event reservation.
unsafe extern "system-unwind" fn group_create(array: usize, element: usize) {
    let call = Call::enter();
    let original: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[0].load(SeqCst)) };
    unsafe { original(array, element) };
    if observing() {
        let m = memory();
        let base = BASE.load(SeqCst);
        observe((|| -> Result<(), &'static str> {
            let manager = array
                .checked_sub(engine_land_inputs::ncombat_log_cmanager::GROUPS)
                .ok_or("combat log manager address underflow")?;
            let result = engine_native::read::<u64>(&m, element)
                .ok_or("combat log new group pointer unreadable")?
                as usize;
            if engine_native::read::<u64>(&m, manager)
                != base
                    .checked_add(engine_land_inputs::ncombat_log_cmanager::VTABLE_RVA)
                    .map(|p| p as u64)
                || engine_native::read::<u64>(&m, result)
                    != base
                        .checked_add(engine_land_inputs::ncombat_log_corders_group_logs::VTABLE_RVA)
                        .map(|p| p as u64)
            {
                return Err("combat log creation identity differs");
            }
            let country = engine_native::read::<u32>(
                &m,
                manager + engine_land_inputs::ncombat_log_cmanager::COUNTRY,
            )
            .ok_or("combat log manager country unreadable")?;
            let reference = engine_native::read::<u64>(
                &m,
                result + engine_land_inputs::ncombat_log_corders_group_logs::GROUP,
            )
            .ok_or("combat log group reference unreadable")?;
            let count = engine_native::read::<u32>(&m, array + 12)
                .and_then(|n| n.checked_sub(1))
                .ok_or("combat log appended group count invalid")?;
            emit(native::Fact {
                kind: native::GROUP,
                root: result as u64,
                values: [country as u64, reference, count as u64, 0, 0],
                ..Default::default()
            });
            Ok(())
        })());
    }
    call.finish();
}

macro_rules! equipment_writer {
    ($name:ident, $slot:expr, ($($arg:ident:$ty:ty),*), $kind:expr) => {
        unsafe extern "system-unwind" fn $name(owner: usize, $($arg:$ty),*) {
            let call = Call::enter();
            let original: unsafe extern "system-unwind" fn(usize, $($ty),*) = unsafe { std::mem::transmute(ORIGINAL[$slot].load(SeqCst)) };
            unsafe { original(owner, $($arg),*) };
            if observing() && ($kind == 4 || equipment_enabled()) { observe(native::last_month(&memory(), BASE.load(SeqCst), owner, $kind, &mut emit)); }
            call.finish();
        }
    }
}
equipment_writer!(own_equipment, 1, (reason:i32, pool:usize), reason as u8);
equipment_writer!(own_variant, 2, (reason:i32, variant:usize, amount:i32), reason as u8);
equipment_writer!(enemy, 3, (pool:usize), 5);
equipment_writer!(recovered, 4, (pool:usize), 4);
unsafe extern "system-unwind" fn manpower(owner: usize, amount: i32, reason: i32) {
    let call = Call::enter();
    let original: unsafe extern "system-unwind" fn(usize, i32, i32) =
        unsafe { std::mem::transmute(ORIGINAL[5].load(SeqCst)) };
    unsafe { original(owner, amount, reason) };
    if observing() {
        observe(native::last_manpower(
            &memory(),
            BASE.load(SeqCst),
            owner,
            &mut emit,
        ));
    }
    call.finish();
}
unsafe extern "system-unwind" fn result(owner: usize, activity: usize, id: i32, victory: u8) {
    let call = Call::enter();
    let original: unsafe extern "system-unwind" fn(usize, usize, i32, u8) =
        unsafe { std::mem::transmute(ORIGINAL[6].load(SeqCst)) };
    unsafe { original(owner, activity, id, victory) };
    if observing() {
        let m = memory();
        let base = BASE.load(SeqCst);
        observe(native::last_reference(&m, owner, &mut emit));
        emit(native::Fact {
            kind: native::COUNT,
            root: id as u32 as u64,
            ..Default::default()
        });
        if activity != 0 && equipment_enabled() {
            observe(native::last_month(&m, base, owner, 5, &mut emit));
        }
        // The normal result route writes its own combat month inline before
        // this call; metadata is reached here without walking other months.
        if activity == 0 && equipment_enabled() {
            observe(native::last_month(&m, base, owner, 0, &mut emit));
        }
    }
    call.finish();
}
unsafe extern "system-unwind" fn template(registers: &Registers, _: usize) {
    let call = Call::enter();
    if observing() {
        observe(native::template(
            &memory(),
            BASE.load(SeqCst),
            registers.r13,
            registers.rdi,
            true,
            &mut emit,
        ));
    }
    call.finish();
}
unsafe extern "system-unwind" fn prune(owner: usize) {
    let call = Call::enter();
    let before = observing().then(|| native::counts(&memory(), BASE.load(SeqCst), owner));
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[7].load(SeqCst)) };
    unsafe { original(owner) };
    if let Some(before) = before {
        observe(before.and_then(|before| {
            let after = native::counts(&memory(), BASE.load(SeqCst), owner)?;
            for (kind, (before, after)) in before.into_iter().zip(after).enumerate() {
                let removed = before
                    .checked_sub(after)
                    .ok_or("combat log prune grew array")?;
                if removed != 0 {
                    emit(native::Fact {
                        kind: native::PREFIX_REMOVED,
                        root: owner as u64,
                        item: kind as u64,
                        values: [removed as u64, 0, 0, 0, 0],
                    });
                }
            }
            Ok(())
        }));
    }
    call.finish();
}
unsafe extern "system-unwind" fn destroy(owner: usize) {
    let call = Call::enter();
    if observing() {
        emit(native::Fact {
            kind: native::GROUP_REMOVED,
            root: owner as u64,
            ..Default::default()
        });
    }
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[8].load(SeqCst)) };
    unsafe { original(owner) };
    call.finish();
}
unsafe extern "system-unwind" fn allocate(source: usize) -> u32 {
    let call = Call::enter();
    let token = advance(&ORDER);
    if observing() {
        observe(native::battle(
            &memory(),
            BASE.load(SeqCst),
            source,
            token,
            &mut emit,
        ));
    }
    let original: unsafe extern "system-unwind" fn(usize) -> u32 =
        unsafe { std::mem::transmute(ORIGINAL[9].load(SeqCst)) };
    let id = unsafe { original(source) };
    if observing() {
        emit(native::Fact {
            kind: native::SLOT,
            root: token,
            item: id as u64,
            ..Default::default()
        });
    }
    call.finish();
    id
}
unsafe extern "system-unwind" fn release(id: u32) {
    let call = Call::enter();
    let original: unsafe extern "system-unwind" fn(u32) =
        unsafe { std::mem::transmute(ORIGINAL[10].load(SeqCst)) };
    unsafe { original(id) };
    if observing() {
        emit(native::Fact {
            kind: native::COUNT,
            root: id as u64,
            ..Default::default()
        });
    }
    call.finish();
}
unsafe extern "system-unwind" fn reset() {
    let call = Call::enter();
    if observing() {
        fail("combat log pool reset invalidated history baseline");
    }
    let original: unsafe extern "system-unwind" fn() =
        unsafe { std::mem::transmute(ORIGINAL[11].load(SeqCst)) };
    unsafe { original() };
    call.finish();
}
unsafe extern "system-unwind" fn load(reader: usize) {
    let call = Call::enter();
    if observing() {
        fail("combat log load invalidated history baseline");
    }
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[12].load(SeqCst)) };
    unsafe { original(reader) };
    call.finish();
}

pub struct Hooks {
    hooks: Vec<Hook>,
    tap: Option<Tap>,
}
impl Hooks {
    /// # Safety
    /// The caller owns hook publication and quiescence for the verified image.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, SeqCst);
        match unsafe { Self::install(memory, base) } {
            Ok(hooks) => hooks,
            Err(reason) => {
                let _ = UNHOOKED.set(reason);
                Self {
                    hooks: Vec::new(),
                    tap: None,
                }
            }
        }
    }
    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Self, String> {
        let sites = [
            (
                sites::GROUP_CREATE_CALL_RVA,
                sites::GROUP_CREATE_CALL_SENTINEL,
                group_create as *const () as usize,
            ),
            (
                sites::OWN_EQUIPMENT_RVA,
                sites::OWN_EQUIPMENT_SENTINEL,
                own_equipment as *const () as usize,
            ),
            (
                sites::OWN_VARIANT_RVA,
                sites::OWN_VARIANT_SENTINEL,
                own_variant as *const () as usize,
            ),
            (
                sites::ENEMY_RVA,
                sites::ENEMY_SENTINEL,
                enemy as *const () as usize,
            ),
            (
                sites::RECOVERED_RVA,
                sites::RECOVERED_SENTINEL,
                recovered as *const () as usize,
            ),
            (
                sites::MANPOWER_RVA,
                sites::MANPOWER_SENTINEL,
                manpower as *const () as usize,
            ),
            (
                sites::RESULT_RVA,
                sites::RESULT_SENTINEL,
                result as *const () as usize,
            ),
            (
                sites::GROUP_PRUNE_RVA,
                sites::GROUP_PRUNE_SENTINEL,
                prune as *const () as usize,
            ),
            (
                sites::GROUP_DESTROY_RVA,
                sites::GROUP_DESTROY_SENTINEL,
                destroy as *const () as usize,
            ),
            (
                sites::BATTLE_ALLOCATE_RVA,
                sites::BATTLE_ALLOCATE_SENTINEL,
                allocate as *const () as usize,
            ),
            (
                sites::BATTLE_RELEASE_RVA,
                sites::BATTLE_RELEASE_SENTINEL,
                release as *const () as usize,
            ),
            (
                sites::BATTLE_RESET_RVA,
                sites::BATTLE_RESET_SENTINEL,
                reset as *const () as usize,
            ),
            (
                sites::BATTLE_LOAD_RVA,
                sites::BATTLE_LOAD_SENTINEL,
                load as *const () as usize,
            ),
        ];
        let mut hooks = Vec::new();
        for (index, (rva, bytes, callback)) in sites.into_iter().enumerate() {
            if engine_native::read::<[u8; 16]>(memory, base + rva) != Some(bytes) {
                return Err("combat log writer sentinel differs".into());
            }
            let hook = if index == 0 {
                unsafe { Hook::new_call(base + rva, callback, bytes) }?
            } else {
                unsafe { Hook::new(base + rva, callback, bytes) }?
            };
            ORIGINAL[index].store(
                if index == 0 {
                    base + sites::GROUP_APPEND_RVA
                } else {
                    hook.trampoline()
                },
                SeqCst,
            );
            hooks.push(hook);
        }
        if engine_native::read::<[u8; 16]>(memory, base + sites::TEMPLATE_COUNTER_RVA)
            != Some(sites::TEMPLATE_COUNTER_SENTINEL)
        {
            return Err("combat log template sentinel differs".into());
        }
        let tap = unsafe {
            Tap::new(
                base + sites::TEMPLATE_COUNTER_RVA,
                template,
                0,
                sites::TEMPLATE_COUNTER_SENTINEL,
            )
        }?;
        Ok(Self {
            hooks,
            tap: Some(tap),
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        ENABLED.store(false, SeqCst);
        if enabled && (UNHOOKED.get().is_some() || retention_required()) {
            return Err("combat log writer family unavailable".into());
        }
        // Publish as one family so a partial enable cannot certify history.
        let mut refs: Vec<&mut Hook> = self.hooks.iter_mut().collect();
        if let Some(tap) = self.tap.as_mut() {
            refs.push(tap.publication_hook());
        }
        let complete = Hook::set_enabled_batch(&mut refs, enabled)?;
        if complete {
            ENABLED.store(enabled, SeqCst);
        }
        Ok(complete)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.tap.iter().flat_map(Tap::executable_ranges))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combat_log_history_producer_allocates_nothing_and_counts_complete_refused_demand() {
        let lane = Lane::new(2, 0).unwrap();
        let row = |order| Logged {
            order,
            signal: 123,
            fact: native::Fact {
                kind: native::MANPOWER,
                root: 99,
                item: 100,
                values: [0, 60_000_000, i32::MIN as u32 as u64, 0, i32::MAX as u64],
            },
        };
        collector_test_support::with_allocations_denied(|| {
            assert!(publish(&lane, row(1)));
            assert!(publish(&lane, row(2)));
            assert!(!publish(&lane, row(3)));
            assert!(!publish(&lane, row(4)));
        });
        assert!(lane.required_entries() >= 4);
        assert_eq!(lane.rejected(), 2);
        let published = lane.published(|r| r.order).unwrap();
        assert_eq!(published.len(), 2);
        for (order, index) in published {
            let (actual, _) = lane.record(index);
            assert_eq!(actual.order, order);
            assert_eq!(actual.fact, row(order).fact);
            lane.release(index);
        }
        lane.advance(lane.rejected());
        assert!(lane.is_drained());
    }
}
