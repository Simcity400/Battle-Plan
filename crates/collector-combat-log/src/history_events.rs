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
static BASE: AtomicUsize = AtomicUsize::new(0);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Witness {
    pub revision: u64,
    pub lost: u64,
}
#[derive(Clone, Copy)]
#[repr(usize)]
enum Loss {
    WriterLifetime = 1,
    Unwind = 2,
    EventLifetime = 3,
    Handoff = 4,
    Storage = 5,
    Session = 6,
    Allocation = 7,
    Operand = 8,
    EquipmentFeature = 9,
    Growth = 10,
    DrainOverlap = 11,
    Reset = 12,
    Load = 13,
}
const LOSS_REASONS: [&str; 13] = [
    "combat log writer lifetime counter exhausted",
    "combat log native writer escaped unwind",
    "combat log event lifetime counter exhausted",
    "combat log event storage handoff overlapped writer",
    "combat log event storage not prepared",
    "combat log event session unreadable",
    "combat log event allocation failed",
    "combat log entries could not be followed in the game",
    "combat log equipment feature unreadable",
    "combat log event growth allocation failed",
    "combat log writer overlapped drain",
    "combat log pool reset invalidated history baseline",
    "combat log load invalidated history baseline",
];
struct Writers {
    active: AtomicUsize,
    revision: AtomicU64,
    enabled: AtomicBool,
    escaped: AtomicBool,
    exhausted: AtomicBool,
    lost: AtomicU64,
    first_loss: AtomicUsize,
    unhooked: OnceLock<String>,
}
impl Writers {
    const fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
            enabled: AtomicBool::new(false),
            escaped: AtomicBool::new(false),
            exhausted: AtomicBool::new(false),
            lost: AtomicU64::new(0),
            first_loss: AtomicUsize::new(0),
            unhooked: OnceLock::new(),
        }
    }
    fn fail(&self, reason: Loss) {
        let _ = self
            .lost
            .fetch_update(SeqCst, SeqCst, |n| Some(n.saturating_add(1)));
        let _ = self
            .first_loss
            .compare_exchange(0, reason as usize, SeqCst, SeqCst);
    }
    fn first_loss(&self) -> Option<&'static str> {
        LOSS_REASONS
            .get(self.first_loss.load(SeqCst).checked_sub(1)?)
            .copied()
    }
    fn changed(&self) {
        if self
            .revision
            .fetch_update(SeqCst, SeqCst, |n| n.checked_add(1))
            .is_err()
        {
            self.exhausted.store(true, SeqCst);
            self.fail(Loss::WriterLifetime);
        }
    }
    fn enter(&self) -> WriterCall<'_> {
        self.active.fetch_add(1, SeqCst);
        self.changed();
        WriterCall {
            writers: self,
            finished: false,
        }
    }
    fn witness(&self) -> Witness {
        Witness {
            revision: self.revision.load(SeqCst),
            lost: self.lost.load(SeqCst),
        }
    }
    fn quiet(&self) -> Option<Witness> {
        self.quiet_since(self.witness())
    }
    fn quiet_since(&self, witness: Witness) -> Option<Witness> {
        (self.enabled.load(SeqCst)
            && !self.exhausted.load(SeqCst)
            && self.unhooked.get().is_none()
            && self.active.load(SeqCst) == 0
            && self.witness() == witness)
            .then_some(witness)
    }
}
struct WriterCall<'a> {
    writers: &'a Writers,
    finished: bool,
}
impl WriterCall<'_> {
    #[cfg(test)]
    fn finish(mut self) {
        self.finished = true;
    }
}
impl Drop for WriterCall<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.writers.escaped.store(true, SeqCst);
            self.writers.fail(Loss::Unwind);
        }
        self.writers.active.fetch_sub(1, SeqCst);
    }
}

static WRITERS: Writers = Writers::new();
static ORDER: AtomicU64 = AtomicU64::new(1);
static REQUIRED: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_DEMAND: AtomicUsize = AtomicUsize::new(0);
static NONCOMBAT_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SEED_DEMAND: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 13] = [const { AtomicUsize::new(0) }; 13];

fn fail(reason: Loss) {
    WRITERS.fail(reason);
}
pub fn active() -> usize {
    WRITERS.active.load(SeqCst)
}
pub(crate) fn quiet() -> Option<Witness> {
    WRITERS.quiet()
}
/// # Safety
/// Caller must own session teardown/detach (or isolated test cleanup) when disabling combat
/// history observation.
pub unsafe fn stop() {
    WRITERS.enabled.store(false, SeqCst);
    WRITERS.changed();
}
pub fn retention_required() -> bool {
    WRITERS.escaped.load(SeqCst)
}
/// # Safety
/// Caller must own session teardown/detach after combat history hooks are stopped and quiesced
/// (or isolated test cleanup with no live hooks or concurrent consumers).
pub unsafe fn release_storage() -> Result<(), &'static str> {
    if WRITERS.enabled.load(SeqCst) || WRITERS.active.load(SeqCst) != 0 || retention_required() {
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
    WRITERS.enabled.load(SeqCst) && !WRITERS.exhausted.load(SeqCst)
}
fn advance(counter: &AtomicU64) -> u64 {
    match counter.fetch_update(SeqCst, SeqCst, |n| {
        n.checked_add(1).filter(|next| *next < (1 << 61))
    }) {
        Ok(n) => n,
        Err(_) => {
            WRITERS.exhausted.store(true, SeqCst);
            fail(Loss::EventLifetime);
            u64::MAX
        }
    }
}

thread_local! { static SIGNAL: Cell<Option<i64>> = const { Cell::new(None) }; }
struct Call {
    writer: WriterCall<'static>,
    previous_signal: Option<i64>,
    _hook: Option<collector_memory::terminal::Scope>,
}
impl Call {
    fn enter() -> Self {
        let hook = crate::hook_timing::label();
        let writer = WRITERS.enter();
        let previous_signal = SIGNAL.get();
        SIGNAL.set(if observing() {
            previous_signal.or_else(signal)
        } else {
            None
        });
        Self {
            writer,
            previous_signal,
            _hook: Some(hook),
        }
    }
    fn finish(mut self) {
        self.writer.finished = true;
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        SIGNAL.set(self.previous_signal);
    }
}
fn memory() -> engine_native::memory::LiveMemory {
    unsafe { engine_native::memory::LiveMemory::new() }
}
fn signal() -> Option<i64> {
    engine_native::read(
        &memory(),
        engine_sites::at(
            BASE.load(SeqCst),
            engine_native::cgame_state::PLAY_SESSION_STARTED_AT_RVA,
        )?,
    )
}
fn emit(fact: native::Fact) {
    if HANDOFF.load(SeqCst) {
        fail(Loss::Handoff);
        return;
    }
    let Some(lane) = (unsafe { LANE.load(SeqCst).as_ref() }) else {
        REQUIRED.fetch_add(1, SeqCst);
        fail(Loss::Storage);
        return;
    };
    let Some(signal) = SIGNAL.get() else {
        fail(Loss::Session);
        return;
    };
    let order = advance(&ORDER);
    if WRITERS.exhausted.load(SeqCst) {
        return;
    }
    let published = publish(
        lane,
        Logged {
            order,
            signal,
            fact,
        },
    );
    let required = lane.required_entries();
    if required > lane.len() {
        REQUIRED.fetch_max(required, SeqCst);
    }
    if !published {
        fail(Loss::Allocation);
    }
}
pub(crate) fn publish(lane: &Lane, entry: Logged) -> bool {
    let Some(reservation) = lane.reserve() else {
        return false;
    };
    reservation.publish(entry);
    true
}
fn observe(result: Result<(), &'static str>) {
    if result.is_err() {
        fail(Loss::Operand);
    }
}
fn equipment_enabled() -> bool {
    match engine_sites::at(BASE.load(SeqCst), engine_dlc::dlc_feature_state::MASK_RVA)
        .and_then(|p| engine_native::read::<u32>(&memory(), p))
    {
        Some(mask) => mask & engine_dlc::dlc_feature_state::EQUIPMENT_LOSSES_MASK != 0,
        None => {
            fail(Loss::EquipmentFeature);
            false
        }
    }
}

/// # Safety
/// Run only from the admitted combat capture or serial sizing pass (or an isolated test), while
/// history-event storage remains live and teardown is excluded.
pub unsafe fn prepare_active(demand: usize) {
    ACTIVE_DEMAND.fetch_max(demand, SeqCst);
    // SAFETY: Caller guarantees admitted combat capture or sizing pass while storage is live.
    unsafe { prepare(0) };
}
pub(crate) fn prepare_noncombat(demand: usize) {
    NONCOMBAT_DEMAND.fetch_max(demand, SeqCst);
    // SAFETY: Crate-internal sizing helper called from `combat_log::Capacities::presize`.
    unsafe { prepare(0) };
}
pub(crate) fn prepare_seed(demand: usize) {
    SEED_DEMAND.fetch_max(demand.max(1), SeqCst);
    // SAFETY: Crate-internal sizing helper called from `history_capture` while storage is live.
    unsafe { prepare(0) };
}
fn demand(seed: usize, active: usize, noncombat: usize, attempted: usize) -> usize {
    seed.saturating_add(active)
        .saturating_add(noncombat)
        .max(attempted)
}
/// # Safety
/// Run only during attach, the active session's serial sizing pass, or admitted combat-log capture
/// (or an isolated test), while history-event storage remains live and teardown is excluded.
pub unsafe fn prepare(attempted: usize) {
    REQUIRED.fetch_max(
        demand(
            SEED_DEMAND.load(SeqCst),
            ACTIVE_DEMAND.load(SeqCst),
            NONCOMBAT_DEMAND.load(SeqCst),
            attempted,
        ),
        SeqCst,
    );
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
    if WRITERS.active.load(SeqCst) != 0 || storage.as_ref().is_some_and(|s| !s.is_drained()) {
        return;
    }
    let Ok(next) =
        engine_allocation::advisory(|| Lane::new(required, 0).and_then(engine_allocation::boxed))
    else {
        fail(Loss::Growth);
        return;
    };
    HANDOFF.store(true, SeqCst);
    if WRITERS.active.load(SeqCst) == 0 && storage.as_ref().is_none_or(|s| s.is_drained()) {
        let mut next = next;
        LANE.store(&mut *next, SeqCst);
        CAPACITY.store(next.len(), SeqCst);
        *storage = Some(next);
    }
    HANDOFF.store(false, SeqCst);
}

pub(crate) fn drain(output: &mut Vec<Logged>) -> Result<Witness, &'static str> {
    output.clear();
    let storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let before = quiet().ok_or("combat log writer active at drain")?;
    if WRITERS.active.load(SeqCst) != 0 {
        return Err("combat log writer active at drain");
    }
    let Some(lane) = storage.as_ref() else {
        return Err("combat log event storage not prepared");
    };
    let mut entries = DRAIN_SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    copy_published(lane, output, &mut entries)?;
    if quiet() != Some(before) {
        fail(Loss::DrainOverlap);
    }
    if !observing() || WRITERS.unhooked.get().is_some() {
        return Err("combat log writers not observed throughout");
    }
    quiet()
        .filter(|w| *w == before)
        .ok_or("combat log writer overlapped drain")
}
pub(crate) fn copy_published(
    lane: &Lane,
    output: &mut Vec<Logged>,
    entries: &mut Vec<(u64, u64)>,
) -> Result<(), &'static str> {
    engine_allocation::advisory(|| {
        output.clear();
        let rejected = lane.rejected();
        lane.published_into(entries, |r| r.order)?;
        engine_allocation::reserve(output, entries.len())
            .map_err(|_| "combat log event drain allocation failed")?;
        for &(_, index) in entries.iter() {
            output.push(lane.record(index).0);
            lane.release(index);
        }
        lane.advance(rejected);
        Ok(())
    })
}
pub fn refusals() -> impl serde::Serialize {
    crate::status::status!({"lost": WRITERS.lost.load(SeqCst), "first_loss": WRITERS.first_loss()})
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "lost": WRITERS.lost.load(SeqCst), "first_loss": WRITERS.first_loss(),
        "unhooked": WRITERS.unhooked.get(), "revision": WRITERS.revision.load(SeqCst),
        "required_entries": REQUIRED.load(SeqCst), "capacity": CAPACITY.load(SeqCst),
        "active_demand": ACTIVE_DEMAND.load(SeqCst), "noncombat_demand": NONCOMBAT_DEMAND.load(SeqCst),
        "seed_demand": SEED_DEMAND.load(SeqCst), "escaped_unwind": WRITERS.escaped.load(SeqCst),
        "recovery": crate::history_capture::recovery_status(),
        "scope": "incremental native writer occurrences; loss requires exact retained-state recovery at the first quiet acquisition"
    })
}

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
            let identity = "combat log creation identity differs";
            let manager_vtable =
                engine_sites::at(base, engine_land_inputs::ncombat_log_cmanager::VTABLE_RVA)
                    .ok_or(identity)?;
            let logs_vtable = engine_sites::at(
                base,
                engine_land_inputs::ncombat_log_corders_group_logs::VTABLE_RVA,
            )
            .ok_or(identity)?;
            if engine_native::read::<u64>(&m, manager) != Some(manager_vtable as u64)
                || engine_native::read::<u64>(&m, result) != Some(logs_vtable as u64)
            {
                return Err(identity);
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
            kind: native::RELEASE,
            root: id as u64,
            ..Default::default()
        });
    }
    call.finish();
}
unsafe extern "system-unwind" fn reset() {
    let call = Call::enter();
    if observing() {
        fail(Loss::Reset);
    }
    let original: unsafe extern "system-unwind" fn() =
        unsafe { std::mem::transmute(ORIGINAL[11].load(SeqCst)) };
    unsafe { original() };
    call.finish();
}
unsafe extern "system-unwind" fn load(reader: usize) {
    let call = Call::enter();
    if observing() {
        fail(Loss::Load);
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
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, SeqCst);
        match unsafe { Self::install(base) } {
            Ok(hooks) => hooks,
            Err(reason) => {
                let _ = WRITERS.unhooked.set(reason);
                Self {
                    hooks: Vec::new(),
                    tap: None,
                }
            }
        }
    }
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    unsafe fn install(base: usize) -> Result<Self, String> {
        let sites = [
            (
                sites::GROUP_CREATE_CALL_RVA,
                group_create as *const () as usize,
            ),
            (
                sites::OWN_EQUIPMENT_RVA,
                own_equipment as *const () as usize,
            ),
            (sites::OWN_VARIANT_RVA, own_variant as *const () as usize),
            (sites::ENEMY_RVA, enemy as *const () as usize),
            (sites::RECOVERED_RVA, recovered as *const () as usize),
            (sites::MANPOWER_RVA, manpower as *const () as usize),
            (sites::RESULT_RVA, result as *const () as usize),
            (sites::GROUP_PRUNE_RVA, prune as *const () as usize),
            (sites::GROUP_DESTROY_RVA, destroy as *const () as usize),
            (sites::BATTLE_ALLOCATE_RVA, allocate as *const () as usize),
            (sites::BATTLE_RELEASE_RVA, release as *const () as usize),
            (sites::BATTLE_RESET_RVA, reset as *const () as usize),
            (sites::BATTLE_LOAD_RVA, load as *const () as usize),
        ];
        let mut hooks = Vec::new();
        let append =
            engine_sites::resolve(base, sites::GROUP_APPEND_RVA).map_err(|w| w.to_string())?;
        for (index, (rva, callback)) in sites.into_iter().enumerate() {
            let code = engine_sites::code(base, rva).map_err(|w| w.to_string())?;
            let bytes = code.original::<16>().map_err(|w| w.to_string())?;
            let hook = if index == 0 {
                unsafe { Hook::new_call(code.address, callback, bytes) }?
            } else {
                unsafe { Hook::new(code.address, callback, bytes) }?
            };
            ORIGINAL[index].store(
                if index == 0 {
                    append
                } else {
                    hook.trampoline()
                },
                SeqCst,
            );
            hooks.push(hook);
        }
        let counter =
            engine_sites::code(base, sites::TEMPLATE_COUNTER_RVA).map_err(|w| w.to_string())?;
        let tap = unsafe {
            Tap::new(
                counter.address,
                template,
                0,
                counter.original::<16>().map_err(|w| w.to_string())?,
            )
        }?;
        Ok(Self {
            hooks,
            tap: Some(tap),
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        WRITERS.enabled.store(false, SeqCst);
        if enabled && (WRITERS.unhooked.get().is_some() || retention_required()) {
            return Err("combat log writer family unavailable".into());
        }
        let complete = Hook::set_enabled_iter(
            self.hooks
                .iter_mut()
                .chain(self.tap.iter_mut().map(Tap::publication_hook)),
            enabled,
        )?;
        if complete {
            WRITERS.enabled.store(enabled, SeqCst);
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
pub(crate) mod test_support {
    use super::*;

    pub struct Producer;
    impl Producer {
        pub fn new() -> Self {
            assert!(LANE.load(SeqCst).is_null());
            assert_eq!(active(), 0);
            assert!(WRITERS.unhooked.get().is_none());
            WRITERS.revision.store(0, SeqCst);
            WRITERS.lost.store(0, SeqCst);
            WRITERS.first_loss.store(0, SeqCst);
            WRITERS.escaped.store(false, SeqCst);
            WRITERS.exhausted.store(false, SeqCst);
            ORDER.store(1, SeqCst);
            REQUIRED.store(0, SeqCst);
            ACTIVE_DEMAND.store(0, SeqCst);
            NONCOMBAT_DEMAND.store(0, SeqCst);
            SEED_DEMAND.store(0, SeqCst);
            WRITERS.enabled.store(true, SeqCst);
            Self
        }
        pub fn write(&self, signal: i64, read: impl FnOnce(&mut fn(native::Fact))) {
            let call = Call {
                writer: WRITERS.enter(),
                previous_signal: SIGNAL.replace(Some(signal)),
                _hook: None,
            };
            read(&mut (emit as fn(native::Fact)));
            call.finish();
        }
        pub fn capacity(&self) -> usize {
            CAPACITY.load(SeqCst)
        }
        pub fn required(&self) -> usize {
            REQUIRED.load(SeqCst)
        }
        pub fn losses(&self) -> u64 {
            WRITERS.lost.load(SeqCst)
        }
        pub fn rejected(&self) -> u64 {
            STORAGE
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, |lane| lane.rejected())
        }
    }
    impl Drop for Producer {
        fn drop(&mut self) {
            // SAFETY: Unit test owns combat-history teardown with no live hooks.
            unsafe {
                stop();
                release_storage().unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_population_and_seed_size_the_burst_without_loss() {
        let required = demand(1, 0, 11_136, 0);
        let lane = Lane::new(required, 0).unwrap();
        collector_test_support::with_allocations_denied(|| {
            for order in 1..=11_136 {
                assert!(publish(
                    &lane,
                    Logged {
                        order,
                        signal: 123,
                        ..Default::default()
                    }
                ));
            }
        });
        assert_eq!(lane.rejected(), 0);
        assert_eq!(lane.published(|r| r.order).unwrap().len(), 11_136);
        assert_eq!(demand(13, 21, 34, 0), 68);
        assert_eq!(demand(13, 21, 34, 1000), 1000);
    }

    #[test]
    fn a_burst_past_the_lane_keeps_every_occurrence_in_order_and_sizes_growth() {
        let lane = Lane::new(1, 0).unwrap();
        for order in 1..=11_136 {
            assert!(publish(
                &lane,
                Logged {
                    order,
                    ..Default::default()
                }
            ));
        }
        assert_eq!(lane.rejected(), 0);
        assert!(lane.required_entries() >= 11_136);
        let mut output = Vec::new();
        copy_published(&lane, &mut output, &mut Vec::new()).unwrap();
        assert_eq!(
            output.iter().map(|logged| logged.order).collect::<Vec<_>>(),
            (1..=11_136).collect::<Vec<_>>()
        );
        assert!(lane.is_drained());
    }

    #[test]
    fn refused_overflow_allocations_count_all_attempted_occurrences_for_growth() {
        let lane = Lane::new(1, 0).unwrap();
        collector_test_support::with_allocations_denied(|| {
            for order in 1..=11_136 {
                publish(
                    &lane,
                    Logged {
                        order,
                        ..Default::default()
                    },
                );
            }
        });
        assert_eq!(lane.rejected(), 11_135);
        assert_eq!(lane.required_entries(), 11_136);
    }

    #[test]
    fn writer_witness_survives_empty_attach_and_load_bursts_without_allocating() {
        let writers = Writers::new();
        writers.enabled.store(true, SeqCst);
        let initial = writers.quiet().unwrap();
        collector_test_support::with_allocations_denied(|| {
            for _ in 0..11_136 {
                writers.enter().finish();
            }
        });
        let settled = writers.quiet().unwrap();
        assert_eq!(settled.revision - initial.revision, 11_136);
        assert_eq!(settled.lost, 0);
        assert_ne!(initial, settled);
        collector_test_support::with_allocations_denied(|| {
            for _ in 0..11_136 {
                writers.enter().finish();
            }
        });
        assert_eq!(writers.quiet().unwrap().revision, 22_272);
        assert_eq!(writers.quiet().unwrap().lost, 0);
    }

    #[test]
    fn overlapping_writers_never_supply_a_quiet_witness() {
        let writers = Writers::new();
        writers.enabled.store(true, SeqCst);
        let first = writers.enter();
        let second = writers.enter();
        assert_eq!(writers.quiet(), None);
        second.finish();
        assert_eq!(writers.quiet(), None);
        first.finish();
        assert_eq!(writers.quiet().unwrap().revision, 2);
    }

    #[test]
    fn a_writer_finishing_during_the_quiet_sample_invalidates_the_old_witness() {
        let writers = Writers::new();
        writers.enabled.store(true, SeqCst);
        let before = writers.quiet().unwrap();
        writers.enter().finish();
        assert_eq!(writers.quiet_since(before), None);
        assert_ne!(writers.quiet().unwrap(), before);
    }

    #[test]
    fn unwound_writer_retains_hook_code_but_can_establish_a_new_baseline() {
        let writers = Writers::new();
        writers.enabled.store(true, SeqCst);
        let before = writers.quiet().unwrap();
        drop(writers.enter());
        assert!(writers.escaped.load(SeqCst));
        let after = writers.quiet().unwrap();
        assert_ne!(before, after);
        assert_eq!(after.lost, 1);
        writers.enter().finish();
        assert_ne!(writers.quiet(), Some(after));
    }

    #[test]
    fn simultaneous_first_losses_publish_one_fixed_diagnostic_without_allocation() {
        let first = Writers::new();
        assert_eq!(first.first_loss(), None);
        collector_test_support::with_allocations_denied(|| first.fail(Loss::Allocation));
        assert_eq!(
            first.first_loss(),
            Some("combat log event allocation failed")
        );
        let writers = Writers::new();
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|threads| {
            for index in 0..8 {
                let writers = &writers;
                let start = &start;
                threads.spawn(move || {
                    start.wait();
                    for _ in 0..1_500 {
                        writers.fail(if index % 2 == 0 {
                            Loss::Allocation
                        } else {
                            Loss::Unwind
                        });
                    }
                });
            }
        });
        assert_eq!(writers.lost.load(SeqCst), 12_000);
        let first = writers.first_loss().unwrap();
        assert!(matches!(
            first,
            "combat log event allocation failed" | "combat log native writer escaped unwind"
        ));
        collector_test_support::with_allocations_denied(|| writers.fail(Loss::Load));
        assert_eq!(writers.first_loss(), Some(first));
        assert_eq!(writers.lost.load(SeqCst), 12_001);
    }

    #[test]
    fn unavailable_hooks_and_exhausted_witnesses_never_certify_reuse() {
        let writers = Writers::new();
        assert_eq!(writers.quiet(), None);
        writers.enabled.store(true, SeqCst);
        writers.revision.store(u64::MAX, SeqCst);
        writers.enter().finish();
        assert_eq!(writers.quiet(), None);
        assert_eq!(writers.lost.load(SeqCst), 1);
        let writers = Writers::new();
        writers.enabled.store(true, SeqCst);
        writers
            .unhooked
            .set("synthetic missing hook".into())
            .unwrap();
        assert_eq!(writers.quiet(), None);
    }
}
