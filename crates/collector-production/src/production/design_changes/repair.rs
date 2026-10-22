//! Receiver-local evidence of genuine overlapping native writers. Prepared marks
//! exclude uncertain snapshots; terminal reads recover current state, never the
//! unobservable occurrences. Ordinary STORE contention loses no callback.
use super::*;
use collector_certificates::watch_registry::WatchRegistry;

static MARKS: WatchRegistry = WatchRegistry::marking();
static PENDING: AtomicBool = AtomicBool::new(false);
static REPAIRING: AtomicBool = AtomicBool::new(false);
static ADDED: AtomicBool = AtomicBool::new(false);

pub(super) fn pending() -> bool {
    PENDING.load(Ordering::Acquire) || REPAIRING.load(Ordering::Acquire)
}
pub(super) fn added() {
    ADDED.store(true, Ordering::Release);
}
pub(super) fn prepare(watches: &HashMap<usize, Watch>) -> Result<(), &'static str> {
    MARKS.collect();
    MARKS.offer(watches.capacity());
    MARKS.set_enabled(true);
    MARKS.boundary(true);
    for pointer in watches.keys() {
        MARKS
            .register(*pointer)
            .ok_or("variant repair watch allocation failed")?;
    }
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
pub(super) struct Repairs {
    admitted: bool,
    pub watches: Vec<Watch>,
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
        lost: 0,
    };
    // Consuming the marks does not yet publish their replacement values. Keep
    // carried reads closed until the boundary has applied and checked them.
    REPAIRING.store(true, Ordering::Release);
    let pending = PENDING.swap(false, Ordering::AcqRel);
    let added = ADDED.swap(false, Ordering::AcqRel);
    if !pending && !added {
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
        if pending && fresh {
            pointers.insert(*pointer);
        }
    }
    MARKS
        .take(|pointer| {
            pointers.insert(pointer);
        })
        .ok_or("variant repair marks have a running writer or changed epoch")?;
    repairs.lost = pointers.len();

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
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_marks_cover_receivers_without_callback_growth() {
        let watches = (1..=5)
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
            for pointer in 1..=5 {
                mark(pointer);
                mark(pointer);
            }
        });
        let repairs = take(&mut Batch::empty()).unwrap();
        assert_eq!(repairs.lost, 5);
        assert_eq!(repairs.watches.len(), 5);
        assert!(pending());
        drop(repairs);
        assert!(!pending());
    }
}
