//! Receiver-local lost-log evidence. Slots are prepared from the saved census;
//! hooks only mark atomics, even while STORE is unavailable. The acquisition worker
//! reads terminal state, not a replacement history of the lost occurrences.
use super::*;
use collector_certificates::watch_registry::WatchRegistry;

static MARKS: WatchRegistry = WatchRegistry::marking();
static RETIRED: WatchRegistry = WatchRegistry::marking();
static RETIRING: AtomicBool = AtomicBool::new(false);
pub(super) static DEFERRED_RETIREMENTS: AtomicU64 = AtomicU64::new(0);
pub(super) static DRAFT_RETIREMENTS: AtomicU64 = AtomicU64::new(0);
pub(super) static DRAFT_MOVES: AtomicU64 = AtomicU64::new(0);
static PENDING: AtomicBool = AtomicBool::new(false);
static REPAIRING: AtomicBool = AtomicBool::new(false);
static ADDED: AtomicBool = AtomicBool::new(false);

pub(super) fn pending() -> bool {
    PENDING.load(Ordering::Acquire)
        || REPAIRING.load(Ordering::Acquire)
        || dependency_repair::pending()
}
pub(super) fn added() {
    ADDED.store(true, Ordering::Release);
}
pub(super) fn prepare(watches: &HashMap<usize, Watch>) -> Result<(), &'static str> {
    for marks in [&MARKS, &RETIRED] {
        marks.collect();
        marks.offer(watches.capacity());
        marks.set_enabled(true);
        marks.boundary(true);
        for pointer in watches.keys() {
            marks
                .register(*pointer)
                .ok_or("variant repair watch allocation failed")?;
        }
    }
    RETIRING.store(false, Ordering::Release);
    PENDING.store(false, Ordering::Release);
    ADDED.store(false, Ordering::Release);
    Ok(())
}
pub(super) fn mark(pointer: usize) {
    if pointer == 0 || STORE.get().is_none() {
        unknown_receiver!();
        return;
    }
    drop(MARKS.notify(pointer));
    PENDING.store(true, Ordering::Release);
}
pub(super) fn contended(pointer: usize) {
    LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    RECEIVER_LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    mark(pointer);
}
/// Destruction supplies its receiver before the original. Only prepared saved
/// slots produce tombstones; excluded drafts require no native read or event.
/// Publication while a retirement is deferred must use a complete roster,
/// because the same pointer may now name a replacement binding.
pub(super) fn retire(pointer: usize) {
    if pointer == 0 {
        unknown_receiver!();
        return;
    }
    LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    RECEIVER_LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    let notification = RETIRED.notify(pointer);
    if !notification.has_mark() && !ADDED.load(Ordering::Acquire) {
        // No publication has added an unprepared binding. This receiver was
        // never saved in this interval, so its destruction loses no row.
        DRAFT_RETIREMENTS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    DEFERRED_RETIREMENTS.fetch_add(1, Ordering::Relaxed);
    RETIRING.store(true, Ordering::Release);
    drop(notification);
    PENDING.store(true, Ordering::Release);
}
/// A consuming move needs no saved bindings only when both operands lack
/// prepared slots and no publication has added an unprepared binding. Boundary
/// registration clears ADDED, so its in-progress window cannot prove absence.
/// The caller's active flight also prevents a concurrent census from admission.
pub(super) fn excluded_move(first: usize, second: usize) -> bool {
    if first == 0
        || second == 0
        || !SAVED_BASELINE.load(Ordering::Acquire)
        || RECOVER.load(Ordering::Acquire)
        || ADDED.load(Ordering::Acquire)
        || REPAIRING.load(Ordering::Acquire)
    {
        return false;
    }
    let first = MARKS.notify(first);
    let second = MARKS.notify(second);
    if first.has_mark()
        || second.has_mark()
        || ADDED.load(Ordering::Acquire)
        || REPAIRING.load(Ordering::Acquire)
    {
        return false;
    }
    LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    RECEIVER_LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    DRAFT_MOVES.fetch_add(1, Ordering::Relaxed);
    true
}
pub(super) fn publication() {
    if RETIRING.load(Ordering::Acquire) {
        unobserved!();
    }
}
pub(super) struct Repairs {
    admitted: bool,
    pub watches: Vec<Watch>,
    pub retired: Vec<Watch>,
    pub authors: std::collections::HashSet<u64>,
    pub lost: usize,
}
impl Drop for Repairs {
    fn drop(&mut self) {
        if !self.admitted {
            // Failed mark consumption cannot reopen a clean census between
            // dropping this guard and the caller posting its refusal.
            unobserved!();
        }
        REPAIRING.store(false, Ordering::Release);
    }
}
pub(super) fn take(batch: &mut Batch) -> Result<Repairs, &'static str> {
    let mut repairs = Repairs {
        admitted: false,
        watches: Vec::new(),
        retired: Vec::new(),
        authors: std::collections::HashSet::new(),
        lost: 0,
    };
    // Consuming the marks does not yet publish their replacement values. Keep
    // carried reads closed until the boundary has applied and checked them.
    REPAIRING.store(true, Ordering::Release);
    let pending = PENDING.swap(false, Ordering::AcqRel);
    let added = ADDED.swap(false, Ordering::AcqRel);
    let retiring = RETIRING.swap(false, Ordering::AcqRel);
    if !pending && !added && !dependency_repair::pending() {
        repairs.admitted = true;
        return Ok(repairs);
    }
    let mut store = STORE
        .get()
        .ok_or("variant change observer not prepared")?
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return Err("variant repair epoch changed");
    }
    let mut pointers = std::collections::HashSet::new();
    engine_allocation::reserve_set(&mut pointers, MARKS.registered() + store.watched.len())
        .map_err(|_| "variant repair allocation failed")?;
    // Publication can add a saved design since the last prepared slots. Until
    // this boundary registers it, a lost callback cannot have marked its slot.
    // Repair those new bindings too, but never dereference retired pointers.
    for pointer in store.watched.keys() {
        let (_, fresh) = MARKS
            .register(*pointer)
            .ok_or("variant repair receiver registration failed")?;
        let (_, retirement_fresh) = RETIRED
            .register(*pointer)
            .ok_or("variant retirement receiver registration failed")?;
        if retiring && retirement_fresh {
            return Err("variant unprepared binding may have an unobserved retirement");
        }
        if pending && fresh {
            pointers.insert(*pointer);
        }
    }
    MARKS
        .take(|pointer| {
            pointers.insert(pointer);
        })
        .ok_or("variant repair marks have a running writer or changed epoch")?;
    let mut retired = std::collections::HashSet::new();
    engine_allocation::reserve_set(&mut retired, RETIRED.registered())
        .map_err(|_| "variant retirement repair allocation failed")?;
    RETIRED
        .take(|pointer| {
            retired.insert(pointer);
        })
        .ok_or("variant retirement marks have a running writer or changed epoch")?;
    let dependencies = dependency_repair::take(&mut store, batch, &mut pointers)?;
    repairs.authors = dependencies.authors;
    repairs.lost = pointers.len() + retired.len() + dependencies.lost;
    repairs.retired = engine_allocation::vec(retired.len())
        .map_err(|_| "variant retirement repair allocation failed")?;
    for pointer in &retired {
        if let Some(watch) = store.watched.remove(pointer) {
            repairs.retired.push(watch);
        }
    }
    pointers.extend(retired);

    repairs.watches =
        engine_allocation::vec(pointers.len()).map_err(|_| "variant repair allocation failed")?;
    for pointer in &pointers {
        if let Some(watch) = store.watched.get_mut(pointer) {
            watch.dirty = true;
            repairs.watches.push(*watch);
        }
    }
    // No recorded snapshot of this receiver is trustworthy after an overlap.
    // Keep binding tombstones, shared-source and author-liveness occurrences;
    // terminal repair is applied after those independent ordered events.
    batch.events.retain(|event| {
        !matches!(event.kind, Kind::Design | Kind::Scalar(_))
            || event.removed
            || !pointers.contains(&event.pointer)
    });
    repairs.watches.sort_unstable_by_key(|watch| watch.id);
    repairs.admitted = true;
    Ok(repairs)
}
/// A lost edit can reach a parent absent from the preceding held state.
/// Guard the reader itself, not a separate earlier sample of PARENT: native
/// receiver changes must never make repair authenticate consumed storage.
pub(super) fn snapshot(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    pointer: usize,
    events: &[Event],
) -> Result<rows::VariantSnapshot, &'static str> {
    // A consuming move can retire then rebind this repair's own receiver.
    // Its current Watch certifies the root, not any other consumed parent.
    if !events
        .iter()
        .any(|event| event.removed && event.pointer != pointer)
    {
        return super::snapshot(memory, base, hour, pointer, true);
    }
    struct Surviving<'a, M> {
        source: &'a M,
        events: &'a [Event],
        receiver: usize,
        consumed: std::cell::Cell<bool>,
    }
    impl<M: Memory> Memory for Surviving<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !output.is_empty()
                && self.events.iter().any(|event| {
                    event.removed
                        && event.pointer != self.receiver
                        && (address
                            .checked_sub(event.pointer)
                            .is_some_and(|offset| offset < ROOT_BYTES)
                            || event
                                .pointer
                                .checked_sub(address)
                                .is_some_and(|offset| offset < output.len()))
                })
            {
                self.consumed.set(true);
                return false;
            }
            self.source.copy(address, output)
        }
    }
    let surviving = Surviving {
        source: memory,
        events,
        receiver: pointer,
        consumed: std::cell::Cell::new(false),
    };
    let result = super::snapshot(&surviving, base, hour, pointer, true);
    if surviving.consumed.get() {
        unobserved!();
        return Err("variant parent identity has a consumed saved binding");
    }
    result
}
pub(super) fn complete(
    watch: &Watch,
    snapshot: &rows::VariantSnapshot,
) -> Result<(), &'static str> {
    let state = snapshot
        .definition
        .design
        .require("VariantChanges.design")
        .map_err(|_| "variant repair lacks design state")?;
    let mut store = STORE
        .get()
        .unwrap()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current = store
        .watched
        .get_mut(&watch.pointer)
        .filter(|current| current.id == watch.id)
        .ok_or("variant repair binding changed during acquisition")?;
    current.flags = [
        state.can_upgrade_type,
        state.can_upgrade_variant,
        state.can_upgrade_modules,
    ];
    Ok(())
}
/// Hooks and acquisition have stopped before their owner releases storage.
pub(super) unsafe fn release() {
    unsafe {
        MARKS.release();
        RETIRED.release();
        dependency_repair::release();
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_marks_cover_five_thousand_receivers_without_callback_growth() {
        let watches = (1..=5_000)
            .map(|pointer| {
                (
                    pointer,
                    Watch {
                        id: pointer as u64,
                        pointer,
                        flags: [0; 3],
                        dirty: false,
                    },
                )
            })
            .collect();
        prepare(&watches).unwrap();
        assert!(
            STORE
                .set(Mutex::new(Store {
                    batch: Batch::empty(),
                    spare: Batch::empty(),
                    watched: watches,
                    interests: sources::Interests::default(),
                    epoch: EPOCH.load(Ordering::Acquire),
                }))
                .is_ok()
        );
        collector_test_support::with_allocations_denied(|| {
            for pointer in 1..=5_000 {
                mark(pointer);
                mark(pointer);
            }
        });
        let repairs = take(&mut Batch::empty()).unwrap();
        assert_eq!(repairs.lost, 5_000);
        assert_eq!(repairs.watches.len(), 5_000);
        assert!(pending());
        drop(repairs);
        assert!(!pending());
    }
}
