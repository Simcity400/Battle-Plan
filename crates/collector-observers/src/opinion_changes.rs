use crate::hook::Hook;
use engine_mutations::{opinion_mutations::MUTATIONS, opinion_mutations::RELATIONS};
use engine_native_memory::Memory;
use engine_native::crelation::status::Mark;
use std::{
    ops::Range,
    ptr,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};

struct Log {
    statuses: Box<[AtomicUsize]>,
    owners: Box<[AtomicUsize]>,
}

impl Log {
    fn new(len: usize) -> Option<Box<Self>> {
        let slots = || {
            let mut slots = engine_allocation::vec(len).ok()?;
            slots.resize_with(len, || AtomicUsize::new(0));
            Some(slots.into_boxed_slice())
        };
        let (statuses, owners) = (slots()?, slots()?);
        Some(Box::new(Self { statuses, owners }))
    }

    fn len(&self) -> u64 {
        self.statuses.len() as u64
    }
}

static LOG: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
static OFFERED: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
static DRAINING: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
static RETIRED: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
static INSTALLS: AtomicU64 = AtomicU64::new(0);
static INSTALLED_LEN: AtomicUsize = AtomicUsize::new(0);
static OFFERED_LEN: AtomicUsize = AtomicUsize::new(0);

fn free(log: *mut Log) {
    if !log.is_null() {
        // SAFETY: each pointer came from Box::into_raw and is owned here.
        drop(unsafe { Box::from_raw(log) });
    }
}

fn installed() -> Option<&'static Log> {
    // SAFETY: a replaced log is freed only after a drain saw no writer
    // running, and the consumer is the only drain. Teardown's `release` frees
    // the installed log only once the hooks are drained and acquisition has
    // stopped, so no borrow from here outlives either.
    unsafe { LOG.load(Ordering::SeqCst).as_ref() }
}
#[repr(align(64))]
pub struct Padded<T>(pub T);
impl<T> std::ops::Deref for Padded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

static CURSOR: Padded<AtomicU64> = Padded(AtomicU64::new(0));
static EPOCH: AtomicU64 = AtomicU64::new(1);
static ENABLED: AtomicBool = AtomicBool::new(false);
static WAR_FORWARD: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
const SITES: usize = MUTATIONS.len() + RELATIONS.len();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static EVENTS: [AtomicU64; SITES + 1] = [const { AtomicU64::new(0) }; SITES + 1];
static UNHOOKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static COUNTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static WALKS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}

#[doc = " The opinion a live status holds. The receiver is the status the native"]
#[doc = " writer is about to read and write."]
fn opinion(status: usize) -> i32 {
    let address = status.wrapping_add(engine_native::crelation::status::OPINION);
    unsafe { std::ptr::read_volatile(address as *const i32) }
}

#[doc = " The owning diplomacy of a live status the native writer is writing. Only"]
#[doc = " the status constructor writes it (`engine_native::crelation::status::DIPLOMACY_STATUS`)."]
fn diplomacy(status: usize) -> usize {
    let address = status.wrapping_add(engine_native::crelation::status::DIPLOMACY_STATUS);
    unsafe { std::ptr::read_volatile(address as *const usize) }
}

struct Mutation;
impl Mutation {
    fn enter(site: usize) -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        if site != 0 {
            EVENTS[site].fetch_add(1, Ordering::Relaxed);
        }
        Self
    }

    fn mark(&self, status: usize, before: i32) {
        if opinion(status) != before {
            self.mark_always(status);
        }
    }

    fn mark_always(&self, status: usize) {
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        publish(index, status, diplomacy(status));
    }
}

pub struct Replaced {
    mutation: Mutation,
    status: usize,
}

impl Drop for Replaced {
    fn drop(&mut self) {
        self.mutation.mark_always(self.status);
    }
}

pub fn relation_replaced(status: usize) -> Option<Replaced> {
    if !ENABLED.load(Ordering::SeqCst) {
        return None;
    }
    EVENTS[SITES].fetch_add(1, Ordering::Relaxed);
    Some(Replaced {
        mutation: Mutation::enter(0),
        status,
    })
}

pub fn set_war_forward(enabled: bool) {
    WAR_FORWARD.store(enabled, Ordering::SeqCst);
    invalidate();
}

fn publish(index: u64, status: usize, diplomacy: usize) -> bool {
    let Some(log) = installed() else {
        invalidate();
        return true;
    };
    let slot = (index % log.len()) as usize;
    log.owners[slot].store(diplomacy, Ordering::SeqCst);
    log.statuses[slot].store(status, Ordering::SeqCst);
    let lapped = CURSOR.load(Ordering::SeqCst).wrapping_sub(index) > log.len();
    if lapped {
        invalidate();
    }
    lapped
}
impl Drop for Mutation {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn drain(
    consumed: &mut u64,
    output: &mut [Mark],
    required: &mut usize,
) -> Option<(u64, Option<usize>)> {
    let cursor = CURSOR.load(Ordering::SeqCst);
    let epoch = EPOCH.load(Ordering::SeqCst);
    let observing = ENABLED.load(Ordering::SeqCst) && WAR_FORWARD.load(Ordering::SeqCst);
    #[cfg(any(test, feature = "test-support"))]
    BETWEEN_READS.with_borrow_mut(|between| {
        if let Some(between) = between.take() {
            between();
        }
    });
    let settled = ACTIVE.load(Ordering::SeqCst) == 0;
    let first = std::mem::replace(consumed, cursor);
    let pending = cursor.wrapping_sub(first);
    *required = usize::try_from(pending).unwrap_or(usize::MAX);
    let log = installed();
    if settled {
        install_offered();
    }
    if !observing {
        COUNTS[1].fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let fits = log.map_or(pending == 0, |log| pending <= log.len());
    let marks = if fits {
        if !settled || pending as usize > output.len() {
            None
        } else if !log.is_none_or(|log| {
            copy_marks(log, first, &mut output[..pending as usize], || {
                CURSOR.load(Ordering::SeqCst)
            })
        }) {
            COUNTS[2].fetch_add(1, Ordering::Relaxed);
            None
        } else if EPOCH.load(Ordering::SeqCst) != epoch {
            None
        } else {
            COUNTS[4].fetch_add(pending, Ordering::Relaxed);
            Some(pending as usize)
        }
    } else {
        COUNTS[2].fetch_add(1, Ordering::Relaxed);
        None
    };
    COUNTS[if marks.is_some() { 0 } else { 1 }].fetch_add(1, Ordering::Relaxed);
    Some((epoch, marks))
}

fn install_offered() {
    let previous = DRAINING.swap(ptr::null_mut(), Ordering::SeqCst);
    if !previous.is_null()
        && RETIRED
            .compare_exchange(
                ptr::null_mut(),
                previous,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
    {
        DRAINING.store(previous, Ordering::SeqCst);
        return;
    }
    let offered = OFFERED.swap(ptr::null_mut(), Ordering::SeqCst);
    if offered.is_null() {
        return;
    }
    INSTALLED_LEN.store(unsafe { &*offered }.statuses.len(), Ordering::SeqCst);
    let replaced = LOG.swap(offered, Ordering::SeqCst);
    DRAINING.store(replaced, Ordering::SeqCst);
    INSTALLS.fetch_add(1, Ordering::Relaxed);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        invalidate();
    }
}

pub fn offer(marks: usize) {
    let Some(len) = marks
        .checked_add(marks / 4)
        .and_then(|len| len.max(1).checked_next_power_of_two())
    else {
        return;
    };
    if len <= OFFERED_LEN.load(Ordering::SeqCst) {
        return;
    }
    let Some(log) = Log::new(len) else {
        return;
    };
    OFFERED_LEN.store(len, Ordering::SeqCst);
    #[cfg(any(test, feature = "test-support"))]
    if let Some(between) = OFFERING.with_borrow_mut(Option::take) {
        between();
    }
    free(OFFERED.swap(Box::into_raw(log), Ordering::SeqCst));
}

pub fn collect() {
    free(RETIRED.swap(ptr::null_mut(), Ordering::SeqCst));
}

#[doc = " Free every log. No writer or drain may run: the hooks were removed and"]
#[doc = " drained and the acquisition pool has stopped."]
fn release() {
    for log in [&LOG, &OFFERED, &DRAINING, &RETIRED] {
        free(log.swap(ptr::null_mut(), Ordering::SeqCst));
    }
    INSTALLED_LEN.store(0, Ordering::SeqCst);
    OFFERED_LEN.store(0, Ordering::SeqCst);
    invalidate();
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static BETWEEN_READS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static OFFERING: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

fn copy_marks(log: &Log, first: u64, output: &mut [Mark], cursor: impl Fn() -> u64) -> bool {
    for (offset, mark) in output.iter_mut().enumerate() {
        let slot = ((first + offset as u64) % log.len()) as usize;
        *mark = Mark {
            status: log.statuses[slot].load(Ordering::SeqCst),
            diplomacy: log.owners[slot].load(Ordering::SeqCst),
        };
    }
    cursor().wrapping_sub(first) <= log.len()
}

pub fn cursor() -> u64 {
    CURSOR.load(Ordering::SeqCst)
}

pub fn store_lent() {
    COUNTS[3].fetch_add(1, Ordering::Relaxed);
}

pub fn record(
    counts: engine_native::crelation::status::BaselineCounts,
    previous: &mut engine_native::crelation::status::BaselineCounts,
) {
    let now = [
        counts.full_walks,
        counts.incremental_walks,
        counts.reused_owners,
        counts.marks,
        counts.refused_observations,
        counts.read_owners,
        counts.invalidated_owners,
    ];
    let before = [
        previous.full_walks,
        previous.incremental_walks,
        previous.reused_owners,
        previous.marks,
        previous.refused_observations,
        previous.read_owners,
        previous.invalidated_owners,
    ];
    for ((total, now), before) in WALKS.iter().zip(now).zip(before) {
        total.fetch_add(now.saturating_sub(before), Ordering::Relaxed);
    }
    *previous = counts;
}

pub fn status() -> impl serde::Serialize {
    let counts = COUNTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let walks = WALKS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let event_counts = EVENTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .chain(&RELATIONS)
            .map(|site| site.name)
            .chain(["replace_relation_forwarded"])
            .zip(event_counts)
            .skip(1),
    );
    crate::status::status!({
        "configured": cfg!(feature = "opinion_changes"),
        "unhooked": UNHOOKED.get(),
        "war_forward": WAR_FORWARD.load(Ordering::SeqCst),
        "observed_refreshes": counts[0],
        "unobserved_refreshes": counts[1],
        "overflowed_logs": counts[2],
        "store_lent": counts[3],
        "marks": counts[4],
        "log_capacity": INSTALLED_LEN.load(Ordering::Relaxed),
        "log_installs": INSTALLS.load(Ordering::Relaxed),
        "full_walks": walks[0],
        "incremental_walks": walks[1],
        "reused_owners": walks[2],
        "read_marks": walks[3],
        "refused_observations": walks[4],
        "read_owners": walks[5],
        "invalidated_owners": walks[6],
        "events": events,
        "scope": "cumulative since attach; each hour reads one owner slice and every owner a drained mark names (every owner when the marks are not handed over: overflowed_logs counts a log too small for the hour's marks, which offers a larger one); an incremental walk replays each selected owner whose country, diplomacy and matrix are unchanged and reads only the statuses a writer changed since its last read, and reads an invalid owner in full (read_owners); events count calls except recompute_opinion, whose changes are the marks"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// Never fails: without every hook, refreshes read every status.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let hooks = if cfg!(feature = "opinion_changes") {
            unsafe { Self::install(base) }.unwrap_or_else(|reason| {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    reason,
                );
                let _ = UNHOOKED.set(reason);
                Vec::new()
            })
        } else {
            Vec::new()
        };
        Ok(Self { hooks })
    }

    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    unsafe fn install(base: usize) -> Result<Vec<Hook>, &'static str> {
        let callbacks: [usize; SITES] = [
            recompute_opinion as *const () as usize,
            reset as *const () as usize,
            load_token as *const () as usize,
            construct as *const () as usize,
            air_relation as *const () as usize,
            attache_relation as *const () as usize,
            boost_party_relation as *const () as usize,
            subject_relation as *const () as usize,
            docking_relation as *const () as usize,
            embargo_relation as *const () as usize,
            equipment_purchase_relation as *const () as usize,
            guarantee_relation as *const () as usize,
            improve_relation as *const () as usize,
            lend_lease_relation as *const () as usize,
            licence_relation as *const () as usize,
            market_access_relation as *const () as usize,
            military_access_relation as *const () as usize,
            blockade_relation as *const () as usize,
            non_aggression_relation as *const () as usize,
        ];
        let sites = || MUTATIONS.iter().chain(&RELATIONS);
        for site in sites() {
            crate::located(base, site.rva)?;
        }
        let mut hooks = Vec::new();
        for (index, site) in sites().enumerate() {
            let code = engine_sites::hook(base, site.rva)?;
            let hook = unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                .map_err(|_| "native hook construction failed")?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(hooks)
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            crate::occupation_changes::forwarder(
                crate::occupation_changes::Forwarder::Relations,
                false,
            );
            ENABLED.store(false, Ordering::SeqCst);
            crate::war_changes::forwarder(crate::war_changes::Forwarder::Opinions, false);
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Opinions,
                false,
            );
        }
        if !Hook::set_family_enabled_shared_prepared(&mut self.hooks, enabled, invalidate)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        crate::war_changes::forwarder(
            crate::war_changes::Forwarder::Opinions,
            enabled && !self.hooks.is_empty(),
        );
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Opinions,
            enabled && !self.hooks.is_empty(),
        );
        crate::occupation_changes::forwarder(
            crate::occupation_changes::Forwarder::Relations,
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
        release();
    }
}

fn original(site: usize) -> usize {
    ORIGINAL[site].load(Ordering::Acquire)
}

unsafe extern "system" fn recompute_opinion(status: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let mutation = Mutation::enter(0);
    let before = opinion(status);
    let original: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(original(0)) };
    timing.original(|| unsafe { original(status) });
    mutation.mark(status, before);
}

unsafe extern "system" fn reset(status: usize, first: usize, second: usize, value: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let _occupation = crate::occupation_changes::changed(status);
    let mutation = Mutation::enter(1);
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::StatusReset);
    let _participation =
        crate::war_changes::participation_change(crate::war_changes::Site::StatusReset, status, 0);
    let _inputs = crate::country_input_changes::forwarded(
        crate::country_input_changes::Forward::RelationReset,
        status,
        0,
    );
    let original: unsafe extern "system" fn(usize, usize, usize, usize) =
        unsafe { std::mem::transmute(original(1)) };
    timing.original(|| unsafe { original(status, first, second, value) });
    mutation.mark_always(status);
}

macro_rules! relation {
    ($name:ident, $index:expr) => {
        unsafe extern "system" fn $name(status: usize, relation: usize, slot: usize) {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
            let site = MUTATIONS.len() + $index;
            let mutation = Mutation::enter(site);
            let _wars = crate::war_changes::forwarded(crate::war_changes::Site::RelationSlot);
            let _participation = crate::war_changes::participation_change(
                crate::war_changes::Site::RelationSlot,
                status,
                0,
            );
            let _inputs = crate::country_input_changes::forwarded(
                crate::country_input_changes::Forward::RelationSlot,
                status,
                0,
            );
            let original: unsafe extern "system" fn(usize, usize, usize) =
                unsafe { std::mem::transmute(original(site)) };
            timing.original(|| unsafe { original(status, relation, slot) });
            mutation.mark_always(status);
        }
    };
}
relation!(air_relation, 0);
relation!(attache_relation, 1);
relation!(boost_party_relation, 2);
relation!(subject_relation, 3);
relation!(docking_relation, 4);
relation!(embargo_relation, 5);
relation!(equipment_purchase_relation, 6);
relation!(guarantee_relation, 7);
relation!(improve_relation, 8);
relation!(lend_lease_relation, 9);
relation!(licence_relation, 10);
relation!(market_access_relation, 11);
relation!(military_access_relation, 12);
relation!(blockade_relation, 13);
relation!(non_aggression_relation, 14);

unsafe extern "system" fn load_token(status: usize, reader: usize, token: i32) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let _mutation = Mutation::enter(2);
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::StatusLoad);
    let _participation =
        crate::war_changes::participation_change(crate::war_changes::Site::StatusLoad, status, 0);
    let original: unsafe extern "system" fn(usize, usize, i32) =
        unsafe { std::mem::transmute(original(2)) };
    timing.original(|| unsafe { original(status, reader, token) });
    invalidate();
}

unsafe extern "system" fn construct(storage: usize, diplomacy: usize, target: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let _mutation = Mutation::enter(3);
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::StatusConstruct);
    let _participation = crate::war_changes::participation_change(
        crate::war_changes::Site::StatusConstruct,
        storage,
        diplomacy,
    );
    invalidate();
    let original: unsafe extern "system" fn(usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(original(3)) };
    let result = timing.original(|| unsafe { original(storage, diplomacy, target) });
    invalidate();
    result
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(any(test, feature = "test-support"))]
pub fn observe_for_test(enabled: bool) {
    WAR_FORWARD.store(enabled, Ordering::SeqCst);
    ENABLED.store(enabled, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEN: usize = 8;

    fn install_for_test() -> &'static Log {
        release();
        let log = Box::into_raw(Log::new(LEN).unwrap());
        free(OFFERED.swap(log, Ordering::SeqCst));
        OFFERED_LEN.store(LEN, Ordering::SeqCst);
        install_offered();
        installed().unwrap()
    }

    #[test]
    fn a_lapped_log_refuses_its_copy() {
        let log = Log::new(LEN).unwrap();
        let mut output = [Mark::default(); 2];
        assert!(copy_marks(&log, 5, &mut output, || 5 + LEN as u64));
        assert!(!copy_marks(&log, 5, &mut output, || 6 + LEN as u64));
    }

    #[test]
    fn a_writer_lapped_while_stalled_ends_the_observation() {
        let _lock = test_lock();
        observe_for_test(true);
        let log = install_for_test();
        let mut consumed = cursor();
        let (mut output, mut required) = ([Mark::default(); 4], 0);
        let stalled = Mutation::enter(0);
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        for offset in 0..LEN {
            let writer = Mutation::enter(0);
            let at = CURSOR.fetch_add(1, Ordering::SeqCst);
            assert!(!publish(at, 0x1000 + offset, 0x2000));
            drop(writer);
        }
        assert_eq!(
            log.statuses[(index % LEN as u64) as usize].load(Ordering::SeqCst),
            0x1000 + LEN - 1
        );
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(None)
        );
        assert!(publish(index, 0xdead, 0xbeef));
        drop(stalled);
        let writer = Mutation::enter(0);
        let at = CURSOR.fetch_add(1, Ordering::SeqCst);
        assert!(!publish(at, 0x3000, 0x2000));
        drop(writer);
        observe_for_test(false);
    }

    #[test]
    fn a_lapped_writer_leaving_mid_drain_hands_over_no_marks() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test();
        let mut consumed = cursor();
        let mut output = vec![Mark::default(); LEN];
        let mut required = 0;
        let stalled = Mutation::enter(0);
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(None)
        );
        assert_eq!(consumed, index + 1);
        for offset in 0..LEN {
            let writer = Mutation::enter(0);
            let at = CURSOR.fetch_add(1, Ordering::SeqCst);
            publish(at, 0x1000 + offset, 0x2000);
            drop(writer);
        }
        let before = EPOCH.load(Ordering::SeqCst);
        BETWEEN_READS.with_borrow_mut(|between| {
            *between = Some(Box::new(move || {
                assert!(publish(index, 0xdead, 0xbeef));
                drop(stalled);
            }))
        });
        let drained = drain(&mut consumed, &mut output, &mut required);
        assert_eq!(required, LEN);
        let (epoch, marks) = drained.unwrap();
        assert_eq!(epoch, before, "the epoch read before the writer left");
        assert_eq!(marks, None, "no marks handed over under that epoch");
        let (next, _) = drain(&mut consumed, &mut output, &mut required).unwrap();
        assert!(next > before);
        observe_for_test(false);
    }

    #[test]
    fn an_overflowing_hour_offers_a_log_the_next_drain_installs() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test();
        let mut consumed = cursor();
        let mut output = vec![Mark::default(); 4 * LEN];
        let mut required = 0;
        let burst = |count: usize| {
            for offset in 0..count {
                let writer = Mutation::enter(0);
                let at = CURSOR.fetch_add(1, Ordering::SeqCst);
                publish(at, 0x1000 + offset, 0x2000);
                drop(writer);
            }
        };
        burst(3 * LEN);
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(None)
        );
        assert_eq!(required, 3 * LEN);
        offer(required);
        assert_eq!(INSTALLED_LEN.load(Ordering::SeqCst), LEN);
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(Some(0))
        );
        assert_eq!(INSTALLED_LEN.load(Ordering::SeqCst), 32);
        burst(3 * LEN);
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(Some(3 * LEN))
        );
        assert_eq!(output[3 * LEN - 1].status, 0x1000 + 3 * LEN - 1);
        collect();
        release();
        let before = EPOCH.load(Ordering::SeqCst);
        let writer = Mutation::enter(0);
        assert!(publish(
            CURSOR.fetch_add(1, Ordering::SeqCst),
            0x1000,
            0x2000
        ));
        drop(writer);
        assert!(EPOCH.load(Ordering::SeqCst) > before);
        observe_for_test(false);
    }

    #[test]
    fn unrecognised_prologues_degrade_to_full_reads() {
        let _lock = test_lock();
        let bytes = vec![0_u8; 0x10];
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut hooks = unsafe { Hooks::new(&memory, 0) }.unwrap();
        assert!(hooks.executable_ranges().next().is_none());
        assert_eq!(hooks.set_enabled(true), Ok(true));
        let (mut consumed, mut output, mut required) = (cursor(), [Mark::default(); 4], 0);
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
        assert!(
            serde_json::to_value(status()).unwrap()["unhooked"]
                .as_str()
                .unwrap()
                .contains("withheld")
        );
    }

    #[test]
    fn drained_marks_are_exact_and_refuse_what_they_cannot_account_for() {
        let _lock = test_lock();
        let mut status = [0_i32; 0x400];
        let address = status.as_mut_ptr() as usize;
        let at = engine_native::crelation::status::OPINION / 4;
        observe_for_test(true);
        install_for_test();
        let mut consumed = cursor();
        let (mut output, mut required) = ([Mark::default(); 4], 0);
        status[engine_native::crelation::status::DIPLOMACY_STATUS / 4..][..2]
            .copy_from_slice(&[0x5678, 0x1234]);
        let drained = |consumed: &mut u64, output: &mut [Mark], required: &mut usize| {
            drain(consumed, output, required).map(|(_, marks)| marks)
        };
        let mutation = Mutation::enter(0);
        mutation.mark(address, 0);
        status[at] = 7;
        mutation.mark(address, 0);
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(None)
        );
        drop(mutation);
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(Some(0))
        );
        let mutation = Mutation::enter(1);
        status[at] = 8;
        mutation.mark(address, 7);
        drop(mutation);
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(Some(1))
        );
        assert_eq!(
            output[0],
            Mark {
                status: address,
                diplomacy: 0x1234_0000_5678,
            }
        );
        for _ in 0..5 {
            let mutation = Mutation::enter(0);
            status[at] += 1;
            mutation.mark(address, status[at] - 1);
        }
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(None)
        );
        assert_eq!(required, 5);
        let (epoch, _) = drain(&mut consumed, &mut output, &mut required).unwrap();
        invalidate();
        let (next, marks) = drain(&mut consumed, &mut output, &mut required).unwrap();
        assert!(next > epoch);
        assert_eq!(marks, Some(0));
        observe_for_test(false);
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
    }

    #[test]
    fn a_drain_taking_an_older_offer_while_a_larger_one_is_published_reports_each_ring() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test();
        collect();
        let drain_once = || {
            let (mut consumed, mut output, mut required) = (cursor(), [Mark::default(); 1], 0);
            drain(&mut consumed, &mut output, &mut required);
        };
        offer(2 * LEN);
        OFFERING.with_borrow_mut(|between| *between = Some(Box::new(drain_once)));
        offer(8 * LEN);
        assert_eq!(installed().unwrap().statuses.len(), 32);
        assert_eq!(INSTALLED_LEN.load(Ordering::SeqCst), 32, "the drain reports the ring it took");
        drain_once();
        assert_eq!(installed().unwrap().statuses.len(), 128);
        assert_eq!(INSTALLED_LEN.load(Ordering::SeqCst), 128, "the larger ring is reported once installed");
        let installs = INSTALLS.load(Ordering::SeqCst);
        offer(8 * LEN);
        assert!(OFFERED.load(Ordering::SeqCst).is_null(), "the installed ring already serves that demand");
        drain_once();
        assert_eq!(INSTALLS.load(Ordering::SeqCst), installs);
        collect();
        release();
        observe_for_test(false);
    }
}
