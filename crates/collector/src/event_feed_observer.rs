//! Native Event feed CALL observers, with hourly owned copies and output receipts.

pub(crate) use crate::event_feed_history::Fact;
use crate::{
    event_feed_history::{DetachLosses, Snapshot, Store, Ticket},
    hook::{Hook, TapSafety},
};
use engine::{Memory, event_feed_sites as native};
use std::{
    cell::UnsafeCell,
    mem::{ManuallyDrop, MaybeUninit, size_of},
    ops::Range,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering},
    },
};

const RETAIN: usize = 1 << 63;
const BEFORE_FAULT: u32 = 1;
const RETURNED_FAULT: u32 = 2;
const NATIVE_ESCAPE: u32 = 4;
const OWNERSHIP_FAULT: u32 = 8;

// Permanent aligned qword: low63 native flights; bit63 irreversible retention.
// Only entry ASM acquires and normal exit ASM releases. No reset or Rust Drop.
static FLIGHT: AtomicUsize = AtomicUsize::new(0);
static FAULT: AtomicU32 = AtomicU32::new(0);
static SHARED: AtomicPtr<Owner> = AtomicPtr::new(ptr::null_mut());
static TARGETS: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static RECEIPTS: LazyLock<crossbeam_queue::ArrayQueue<(u64, bool)>> = LazyLock::new(|| {
    crossbeam_queue::ArrayQueue::new(crate::event_feed_history::PUBLICATION_CAPACITY)
});
static DEMAND: AtomicUsize = AtomicUsize::new(0);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static SPARE: Mutex<Option<Store>> = Mutex::new(None);
static RETIRED: Mutex<Option<Store>> = Mutex::new(None);

/// Snapshot offsets are duplicated only in the matching ASM/header.
/// The ticket bytes are opaque to C++; only Rust creates/consumes the ticket.
#[repr(C)]
struct Frame {
    rsp_before_call: usize,
    rcx: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
    rdi: usize,
    rbp: usize,
    rax: usize,
    r10: usize,
    r11: usize,
    xmm: [[u8; 16]; 6],
    site: u32,
    owned_flight: u32,
    terminal: u32,
    reserved: u32,
    ticket: MaybeUninit<Ticket>,
    generation: u64,
    target: usize,
}
const _: () = {
    assert!(size_of::<usize>() == 8);
    assert!(size_of::<Ticket>() == 16);
    assert!(size_of::<Frame>() == 224);
    assert!(std::mem::offset_of!(Frame, xmm) == 80);
    assert!(std::mem::offset_of!(Frame, ticket) == 192);
    assert!(std::mem::offset_of!(Frame, target) == 216);
};

struct Owner {
    base: usize,
    armed: AtomicBool,
    busy: AtomicBool,
    store: UnsafeCell<Store>,
}
// Each short store phase has exclusive checked access. No lease crosses native.
unsafe impl Send for Owner {}
unsafe impl Sync for Owner {}

impl Owner {
    /// Off-thread setup with live-sized storage and the retained sequence seed.
    fn new(base: usize, store: Store) -> Arc<Self> {
        Arc::new(Self {
            base,
            armed: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            store: UnsafeCell::new(store),
        })
    }

    fn access<T>(&self, apply: impl FnOnce(&mut Store) -> T) -> Result<T, &'static str> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            fault(OWNERSHIP_FAULT);
            return Err("Event feed store overlap; owner retained");
        }
        // A Rust panic is contained. A foreign escape is contained by the C++
        // observer-only guard; it may leave busy set, which retains the owner.
        let result = catch_unwind(AssertUnwindSafe(|| {
            apply(unsafe { &mut *self.store.get() })
        }));
        match result {
            Ok(value) => {
                self.busy.store(false, Ordering::SeqCst);
                Ok(value)
            }
            Err(payload) => {
                fault(OWNERSHIP_FAULT);
                // No payload destructor or guessed lock/ticket cleanup after fault.
                std::mem::forget(payload);
                Err("Event feed callback panic; owner retained")
            }
        }
    }

    /// Revoke before any unobserved mutation. This atomic operation does not
    /// settle tickets/flights or establish that a world replacement succeeded.
    fn revoke_world(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

fn fault(code: u32) {
    // Static pointer-free, allocation-free sink. Even an unacquired escape
    // prevents later certification/free; no logical or physical count settles.
    FAULT.fetch_or(code, Ordering::SeqCst);
    FLIGHT.fetch_or(RETAIN, Ordering::SeqCst);
}

unsafe fn before(frame: &mut Frame) -> Result<bool, &'static str> {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        fault(BEFORE_FAULT);
        return Err("Event feed owner unavailable");
    };
    owner.access(|store| {
        if frame.owned_flight != 1 || FAULT.load(Ordering::SeqCst) != 0 {
            return Err(store.reject("Event feed native flight unavailable"));
        }
        let generation = crate::world_lifetime::token();
        if !owner.armed.load(Ordering::SeqCst) || generation.is_none() {
            store.invalidate("Event feed world observation unavailable");
            return Err(store.reject("Event feed world observation unavailable"));
        }
        let generation = generation.unwrap();
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let ticket = match frame.site {
            0 => store.war_before(
                &memory,
                owner.base,
                generation,
                native::WarFrame {
                    rsp_before_call: frame.rsp_before_call,
                    rcx: frame.rcx,
                    rdx: frame.rdx,
                    rdi: frame.rdi,
                },
                crate::war_provenance::inside(),
            ),
            1 => store.annex_before(
                &memory,
                owner.base,
                generation,
                native::AnnexFrame {
                    rbp: frame.rbp,
                    r8: frame.r8,
                    r9: frame.r9 as u64,
                },
                crate::war_provenance::inside(),
            ),
            _ => Err(store.reject("Event feed CALL owner differs")),
        };
        DEMAND.fetch_max(store.capacity_demand(), Ordering::Relaxed);
        let ticket = ticket?;
        frame.ticket.write(ticket);
        frame.generation = generation;
        // Only this final normal write grants the single terminal callback.
        frame.terminal = 1;
        Ok(true)
    })?
}

/// Record an event a provenance hook proved ([`crate::war_provenance`]):
/// `observe` reads it from the current world, `Ok(None)` when the operation
/// changed nothing. Refused, counted and named like a CALL observation when
/// the world is not observed, an operand does not read or storage is full.
pub(crate) fn record(
    observe: impl FnOnce(
        &native::provenance::World<'_, engine::memory::LiveMemory>,
        &engine::memory::LiveMemory,
        usize,
    ) -> Result<Option<Fact>, &'static str>,
) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let _ = owner.access(|store| {
        let Some(generation) = observed(owner, store) else {
            return;
        };
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let fact = native::provenance::World::new(&memory, owner.base).and_then(|world| {
            let hour = world.hour()?;
            Ok(observe(&world, &memory, owner.base)?.map(|fact| (hour, fact)))
        });
        // A refusal is counted and named by the store.
        match fact {
            Ok(Some((hour, fact))) => {
                let _ = store.known(generation, hour, fact);
            }
            Ok(None) => {}
            Err(reason) => {
                store.reject(reason);
            }
        }
        DEMAND.fetch_max(store.capacity_demand(), Ordering::Relaxed);
    });
}

/// Refuse a provenance event whose operands did not read.
pub(crate) fn refuse(reason: &'static str) {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        let _ = owner.access(|store| store.reject(reason));
    }
}

/// The observed world's generation, or `None` after refusing, as a CALL
/// observation does.
fn observed(owner: &Owner, store: &mut Store) -> Option<u64> {
    if FAULT.load(Ordering::SeqCst) != 0 {
        store.reject("Event feed native flight unavailable");
        return None;
    }
    match crate::world_lifetime::token() {
        Some(generation) if owner.armed.load(Ordering::SeqCst) => Some(generation),
        _ => {
            store.invalidate("Event feed world observation unavailable");
            store.reject("Event feed world observation unavailable");
            None
        }
    }
}

/// C++ invokes only on this exact original's normal return. It never repeats
/// this callback after Err/panic/SEH, and never guesses an incomplete settlement.
unsafe fn returned(frame: &mut Frame) -> Result<(), &'static str> {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        fault(RETURNED_FAULT);
        return Err("Event feed terminal owner unavailable");
    };
    if frame.terminal != 1 {
        fault(RETURNED_FAULT);
        return Err("Event feed terminal ownership differs");
    }
    // The store entry/proof was durable before original; ticket consumption is
    // once-only even if this callback fails after taking the opaque token.
    frame.terminal = 2;
    let ticket = unsafe { frame.ticket.assume_init_read() };
    let result = owner.access(|store| {
        if FAULT.load(Ordering::SeqCst) != 0 {
            store.uncertain(&ticket, "Event feed world or native lifetime invalidated");
            return Err(crate::event_feed_history::Retained {
                ticket,
                reason: "Event feed world or native lifetime invalidated",
            });
        }
        if !crate::world_lifetime::current(Some(frame.generation)) {
            return store.world_returned(ticket);
        }
        let memory = unsafe { engine::memory::LiveMemory::new() };
        unsafe { store.normal_returned(ticket, frame.generation, &memory, owner.base) }
    })?;
    match result {
        Ok(()) => {
            frame.terminal = 3;
            Ok(())
        }
        Err(retained) => {
            frame.ticket.write(retained.ticket);
            frame.terminal = 4;
            fault(RETURNED_FAULT);
            Err(retained.reason)
        }
    }
}

// C++/ASM own these stack frames. No game pointer is dereferenced directly.
#[unsafe(no_mangle)]
unsafe extern "system-unwind" fn hoi4_event_before(frame: *mut Frame) -> u32 {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeed);
    u32::from(unsafe { before(&mut *frame) }.unwrap_or(false))
}
#[unsafe(no_mangle)]
unsafe extern "system-unwind" fn hoi4_event_returned(frame: *mut Frame) {
    let _timing = crate::hook_timing::enter_return(crate::hook_timing::Family::EventFeed);
    let _ = unsafe { returned(&mut *frame) };
}
#[unsafe(no_mangle)]
extern "system" fn hoi4_event_observer_fault() {
    fault(BEFORE_FAULT | RETURNED_FAULT);
}
#[unsafe(no_mangle)]
extern "system" fn hoi4_event_native_escape() {
    fault(NATIVE_ESCAPE);
}

core::arch::global_asm!(
    include_str!("event_feed_entry.asm"),
    flight = sym FLIGHT,
    targets = sym TARGETS,
    guard = sym hoi4_event_native_guard,
);
unsafe extern "system-unwind" {
    fn hoi4_event_native_guard(frame: *mut Frame);
    fn event_war_entry();
    fn event_annex_entry();
    static event_stub_start: u8;
    static event_stub_end: u8;
}

struct Held {
    /// The two CALL hooks, then the provenance entry detours.
    hooks: Vec<Hook>,
    owner: Arc<Owner>,
    restored: bool,
}
/// Hooks and history remain owned until restoration and the final quiet check.
pub(crate) struct Hooks {
    held: Option<ManuallyDrop<Held>>,
}

impl Hooks {
    /// Build verified, unpublished CALL hooks using the shared patch owner.
    pub(crate) unsafe fn new(
        memory: &impl Memory,
        base: usize,
        last_sequence: Option<u64>,
        counts: replay::event_feed_rows::EventFeedCounts,
        slots: usize,
    ) -> Result<Self, &'static str> {
        let mut store = Store::new(last_sequence)?;
        store.restore_counts(counts);
        LazyLock::force(&RECEIPTS);
        let capacity = slots.saturating_mul(2).max(1024);
        store
            .prepare_capacity(capacity)
            .map_err(|_| "Event feed staging allocation failed")?;
        let owner = Owner::new(base, store);
        let range = unsafe { &event_stub_start } as *const u8 as usize
            ..unsafe { &event_stub_end } as *const u8 as usize;
        let make = |site: native::CallSite, callback| {
            let address = native::verify(memory, base, site)?;
            unsafe {
                Hook::new_call_with_safety(
                    address,
                    callback,
                    site.sentinel,
                    TapSafety {
                        in_flight: &FLIGHT,
                        stub: range.clone(),
                    },
                )
            }
            .map_err(|_| "Event feed CALL construction failed")
        };
        let mut hooks = vec![
            make(native::WAR, event_war_entry as *const () as usize)?,
            make(native::ANNEX, event_annex_entry as *const () as usize)?,
        ];
        hooks.extend(unsafe { crate::war_provenance::hooks(memory, base) }?);
        if FLIGHT.load(Ordering::SeqCst) != 0
            || FAULT.load(Ordering::SeqCst) != 0
            || !SHARED.load(Ordering::SeqCst).is_null()
        {
            return Err("Event feed previous owner retained");
        }
        TARGETS[0].store(base + native::WAR.target_rva, Ordering::SeqCst);
        TARGETS[1].store(base + native::ANNEX.target_rva, Ordering::SeqCst);
        SHARED.store(Arc::as_ptr(&owner).cast_mut(), Ordering::SeqCst);
        CAPACITY.store(capacity, Ordering::Release);
        Ok(Self {
            held: Some(ManuallyDrop::new(Held {
                hooks,
                owner,
                restored: true,
            })),
        })
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        let Some(held) = self.held.as_mut() else {
            return Ok(true);
        };
        if !enabled {
            held.owner.revoke_world();
        }
        let ready = Hook::set_family_enabled_attach(&mut held.hooks, enabled)
            .map_err(|_| "Event feed CALL state change failed")?;
        if ready {
            held.restored = !enabled;
            held.owner.armed.store(enabled, Ordering::SeqCst);
        }
        Ok(ready)
    }

    pub(crate) fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.held
            .as_ref()
            .into_iter()
            .flat_map(|held| held.hooks.iter().flat_map(Hook::executable_ranges))
            .chain(std::iter::once(
                unsafe { &event_stub_start } as *const u8 as usize
                    ..unsafe { &event_stub_end } as *const u8 as usize,
            ))
            .collect()
    }

    pub(crate) fn release_after_quiet(&mut self) -> Result<DetachLosses, &'static str> {
        let Some(held) = self.held.as_ref() else {
            return Ok(DetachLosses::default());
        };
        let losses = held.detach_losses(active())?;
        *SPARE
            .lock()
            .map_err(|_| "Event feed spare storage retained")? = None;
        *RETIRED
            .lock()
            .map_err(|_| "Event feed retired storage retained")? = None;
        SHARED.store(ptr::null_mut(), Ordering::SeqCst);
        TARGETS[0].store(0, Ordering::SeqCst);
        TARGETS[1].store(0, Ordering::SeqCst);
        let mut held = self.held.take().unwrap();
        drop(unsafe { ManuallyDrop::take(&mut held) });
        Ok(losses)
    }
}

impl Held {
    fn detach_losses(&self, native_flights: usize) -> Result<DetachLosses, &'static str> {
        if !self.restored
            || self.hooks.iter().any(Hook::is_enabled)
            || native_flights != 0
            || self.owner.busy.load(Ordering::SeqCst)
        {
            return Err("Event feed owner has unsettled native ownership");
        }
        self.owner.access(|store| {
            take_receipts(store);
            store.detach_losses()
        })?
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // Also releases unpublished bootstrap storage on cancellation.
        let _ = self.release_after_quiet();
    }
}

fn take_receipts(store: &mut Store) {
    while let Some((publication, accepted)) = RECEIPTS.pop() {
        if accepted {
            store.acknowledged(publication);
        } else {
            store.discarded(publication);
        }
    }
}

/// Native flights of the CALL hooks (bit 63 retains the owner) and the
/// provenance entry detours in flight.
pub(crate) fn active() -> usize {
    FLIGHT.load(Ordering::SeqCst) + crate::war_provenance::active()
}

/// A family whose patch forwards provenance to the event feed. While one is
/// not enabled its events or causes go unobserved, so no interval is
/// complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Forwarder {
    /// [`crate::politics_changes`]: the diplomatic dispatch and civil war
    /// start that declarations happen in.
    Politics,
    /// [`crate::territory_changes`]: set owner, every state
    /// transfer.
    Territory,
}

static FORWARDERS: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

pub(crate) fn forwarder(forwarder: Forwarder, enabled: bool) {
    FORWARDERS[forwarder as usize].store(enabled, Ordering::SeqCst);
}

/// An observed interval's coverage: complete only when the capture is and
/// every forwarder is enabled.
fn coverage(partial: bool) -> replay::event_feed_rows::EventFeedCoverage {
    use replay::event_feed_rows::EventFeedCoverage;
    if partial
        || !FORWARDERS
            .iter()
            .all(|enabled| enabled.load(Ordering::SeqCst))
    {
        EventFeedCoverage::Partial
    } else {
        EventFeedCoverage::Complete
    }
}

/// Live war relations the latest Wars walk counted, for sizing: one
/// conference can end every one of them in an hour.
static WARS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn note_wars(count: usize) {
    WARS.store(count, Ordering::Relaxed);
}

pub(crate) fn lost_drain() {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        owner.revoke_world();
    }
}

pub(crate) fn capture_growing(output: &mut Snapshot, hour: i32, partial: bool) {
    let generation = crate::world_lifetime::generation();
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        output.refuse(generation, hour, "Event feed hooks unavailable");
        return;
    };
    let result = owner
        .access(|store| {
            take_receipts(store);
            if active() == 0
                && let (Ok(mut spare), Ok(mut retired)) = (SPARE.try_lock(), RETIRED.try_lock())
                && retired.is_none()
                && let Some(mut offered) = spare.take()
            {
                store.install_capacity(&mut offered);
                CAPACITY.store(store.capacity(), Ordering::Release);
                *retired = Some(offered);
            }
            let failure = if FAULT.load(Ordering::SeqCst) != 0 || active() & RETAIN != 0 {
                Some("Event feed native or callback ownership retained")
            } else if !owner.armed.load(Ordering::SeqCst) {
                Some("Event feed publication lost; interval unavailable")
            } else {
                crate::world_lifetime::unavailable_reason()
            };
            let boundary = crate::event_feed_history::OwnerBoundary {
                generation,
                read_hour: hour,
                coverage: coverage(partial),
                failure,
            };
            let result = store.snapshot_growing(output, boundary);
            if let Err(reason) = result {
                store.invalidate(reason);
            }
            owner.armed.store(true, Ordering::SeqCst);
            result
        })
        .and_then(|result| result);
    if let Err(reason) = result {
        output.refuse(generation, hour, reason);
    }
}

pub(crate) fn accepted(pack: &crate::event_feed_history::Prepared) -> std::io::Result<()> {
    if pack.publication() != 0 {
        RECEIPTS
            .push((pack.publication(), true))
            .map_err(|_| std::io::Error::other("Event feed output receipt queue full"))?;
    }
    Ok(())
}

pub(crate) fn discarded(publication: u64) {
    if publication != 0 && RECEIPTS.push((publication, false)).is_err() {
        // The frame pool plus the prepared spool write bound receipts.
        fault(OWNERSHIP_FAULT);
    }
}

/// Spool-only growth and retirement; native callbacks never allocate or free.
pub(crate) fn presize(slots: usize) {
    engine::allocation::advisory(|| presize_storage(slots));
}

fn presize_storage(slots: usize) {
    if active() & RETAIN != 0 {
        return;
    }
    if let Ok(mut retired) = RETIRED.lock() {
        drop(retired.take());
    }
    let mut capacity = CAPACITY.load(Ordering::Acquire);
    crate::table_capture::grow_capacity(
        &mut capacity,
        // Two war links per country, and one hour's state owner changes and
        // ended war relations at most every state and war at once.
        slots
            .saturating_mul(2)
            .saturating_add(crate::territory_changes::state_slots())
            .saturating_add(WARS.load(Ordering::Relaxed))
            .max(1024)
            .max(DEMAND.load(Ordering::Relaxed)),
    );
    if capacity <= CAPACITY.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut spare) = SPARE.lock() {
        if spare
            .as_ref()
            .is_some_and(|store| store.capacity() >= capacity)
        {
            return;
        }
        if let Ok(mut store) = Store::new(None)
            && store.prepare_capacity(capacity).is_ok()
        {
            *spare = Some(store);
        }
    }
}

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "enabled": unsafe { SHARED.load(Ordering::SeqCst).as_ref() }.is_some_and(|owner| owner.armed.load(Ordering::SeqCst)),
        "unhooked": crate::world_lifetime::unavailable_reason(),
        "native_flights": active(), "faults": FAULT.load(Ordering::SeqCst),
        "provenance": crate::war_provenance::status(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        CONTEXT, RtlLookupFunctionEntry, RtlVirtualUnwind,
    };

    #[test]
    fn escaped_native_flight_retains_the_event_feed_owner_without_a_ticket() {
        extern "system" fn original() {}
        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let code = crate::hook::TestCode::new();
        let target = original as *const () as usize;
        let (first, first_bytes) = code.stub(0, target);
        let (second, second_bytes) = code.stub(64, target);
        let held = Held {
            hooks: unsafe {
                vec![
                    Hook::new(first, target, first_bytes).unwrap(),
                    Hook::new(second, target, second_bytes).unwrap(),
                ]
            },
            owner: Owner::new(0, Store::new(None).unwrap()),
            restored: true,
        };
        assert_eq!(
            held.detach_losses(RETAIN | 1),
            Err("Event feed owner has unsettled native ownership")
        );
        assert_eq!(held.detach_losses(0), Ok(DetachLosses::default()));
    }

    /// An interval is complete only while both provenance forwarders are
    /// enabled: without them its state transfers or declaration causes went
    /// unobserved.
    #[test]
    fn a_disabled_forwarder_leaves_the_interval_partial() {
        use replay::event_feed_rows::EventFeedCoverage;
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        forwarder(Forwarder::Politics, true);
        forwarder(Forwarder::Territory, true);
        assert_eq!(coverage(false), EventFeedCoverage::Complete);
        assert_eq!(coverage(true), EventFeedCoverage::Partial);
        for disabled in [Forwarder::Politics, Forwarder::Territory] {
            forwarder(disabled, false);
            assert_eq!(coverage(false), EventFeedCoverage::Partial);
            forwarder(disabled, true);
        }
        forwarder(Forwarder::Politics, false);
        forwarder(Forwarder::Territory, false);
    }

    #[test]
    fn call_entries_unwind_to_the_native_return_with_the_original_stack() {
        for (entry, site) in [
            (event_war_entry as *const () as usize, native::WAR),
            (event_annex_entry as *const () as usize, native::ANNEX),
        ] {
            let first = unsafe { std::slice::from_raw_parts(entry as *const u8, 7) };
            assert_eq!(first, &[0x48, 0x81, 0xec, 8, 1, 0, 0]);
            let pc = entry as u64 + 7;
            let mut image_base = 0;
            let function = unsafe { RtlLookupFunctionEntry(pc, &mut image_base, ptr::null_mut()) };
            assert!(!function.is_null());
            assert_eq!(
                image_base + unsafe { (*function).BeginAddress } as u64,
                entry as u64
            );
            #[repr(align(16))]
            struct Stack([u64; 40]);
            let mut stack = Stack([0; 40]);
            let return_pc = 0x140000000 + site.return_rva as u64;
            stack.0[33] = return_pc;
            let mut context: CONTEXT = unsafe { std::mem::zeroed() };
            context.Rip = pc;
            context.Rsp = stack.0.as_ptr() as u64;
            context.Rdi = 77;
            context.Rbp = 88;
            let mut handler_data = ptr::null_mut();
            let mut establisher = 0;
            unsafe {
                RtlVirtualUnwind(
                    0,
                    image_base,
                    pc,
                    function,
                    &mut context,
                    &mut handler_data,
                    &mut establisher,
                    ptr::null_mut(),
                );
            }
            assert_eq!(context.Rip, return_pc);
            assert_eq!(context.Rsp, stack.0.as_ptr() as u64 + 272);
            assert_eq!((context.Rdi, context.Rbp), (77, 88));
        }
    }
}
