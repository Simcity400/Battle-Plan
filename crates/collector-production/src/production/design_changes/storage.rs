use super::*;

static EVENTS: AtomicUsize = AtomicUsize::new(0);
static SIZING: Mutex<()> = Mutex::new(());

pub(super) fn census(events: usize) {
    EVENTS.fetch_max(events, Ordering::Relaxed);
}
pub(super) fn completed(batch: &Batch) {
    EVENTS.fetch_max(batch.required_events.saturating_mul(2), Ordering::Relaxed);
}
fn adequate(batch: &Batch, events: usize) -> bool {
    batch.events.capacity() >= events
}
fn prepared(events: usize) -> Batch {
    let mut batch = Batch::empty();
    batch.prepare(events);
    batch
}
/// The event demand, and whether the active and spare batches need larger storage.
fn demand(store: &Store) -> (usize, bool, bool) {
    let events = EVENTS
        .load(Ordering::Relaxed)
        .max(store.batch.required_events)
        .max(store.spare.required_events);
    let short = |batch: &Batch| {
        batch.events.is_empty() && batch.failure.is_none() && !adequate(batch, events)
    };
    (events, short(&store.batch), short(&store.spare))
}
/// Swaps in each prepared replacement its batch still needs.
fn install(store: &Mutex<Store>, events: usize, replacements: &mut [Option<Batch>; 2]) {
    let mut store = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Store { batch, spare, .. } = &mut *store;
    for (index, batch) in [batch, spare].into_iter().enumerate() {
        if batch.events.is_empty()
            && batch.failure.is_none()
            && !adequate(batch, events)
            && let Some(replacement) = &mut replacements[index]
            && adequate(replacement, events)
        {
            std::mem::swap(batch, replacement);
        }
    }
}
pub fn presize() {
    engine_allocation::advisory(presize_inner);
}
fn presize_inner() {
    let Ok(_sizing) = SIZING.try_lock() else {
        return;
    };
    let Some(store) = STORE.get() else {
        return;
    };
    let (events, active, spare) = demand(
        &store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    let mut replacements = [active.then(|| prepared(events)), spare.then(|| prepared(events))];
    install(store, events, &mut replacements);
    // Retired storage is released outside STORE.
    drop(replacements);
}

pub struct SetupPart {
    ordinal: usize,
    events: usize,
    output: Option<Batch>,
}
impl SetupPart {
    pub fn capture(&mut self) {
        engine_allocation::advisory(|| self.output = Some(prepared(self.events)));
    }
}
pub(in crate::production) fn presize_partitioned(
    acquire: &mut impl FnMut(Vec<SetupPart>) -> Result<Vec<SetupPart>, &'static str>,
) {
    engine_allocation::advisory(|| presize_with(acquire));
}
fn presize_with(
    acquire: &mut impl FnMut(Vec<SetupPart>) -> Result<Vec<SetupPart>, &'static str>,
) {
    let Ok(_sizing) = SIZING.try_lock() else {
        return;
    };
    let Some(store) = STORE.get() else {
        return;
    };
    let (events, active, spare) = demand(
        &store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    let Ok(mut parts) = engine_allocation::vec(2) else {
        return;
    };
    for (ordinal, needed) in [active, spare].into_iter().enumerate() {
        if needed {
            parts.push(SetupPart {
                ordinal,
                events,
                output: None,
            });
        }
    }
    let Ok(parts) = acquire(parts) else { return };
    let mut replacements = [None, None];
    for part in parts {
        replacements[part.ordinal] = part.output;
    }
    install(store, events, &mut replacements);
    // Retired storage is released outside STORE.
    drop(replacements);
}

pub(super) fn recycle(mut batch: Batch) {
    let Some(store) = STORE.get() else {
        return;
    };
    {
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if store.spare.events.capacity() <= batch.events.capacity()
            && store.spare.log.spans.capacity() <= batch.log.spans.capacity()
            && store.spare.log.bytes.capacity() <= batch.log.bytes.capacity()
        {
            std::mem::swap(&mut store.spare, &mut batch);
        }
    }
    drop(batch);
}
pub(super) fn release() {
    EVENTS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod cold_setup_tests {
    use super::*;
    fn parts(countries: usize) -> Vec<SetupPart> {
        vec![SetupPart {
            ordinal: 0,
            events: countries * 2,
            output: None,
        }]
    }
    fn demands(parts: &[SetupPart]) -> Vec<(usize, usize)> {
        parts
            .iter()
            .map(|part| {
                (
                    part.ordinal,
                    part.output.as_ref().map_or(0, |batch| batch.events.capacity()),
                )
            })
            .collect()
    }
    #[test]
    fn cold_storage_parts_preserve_serial_capacity_for_uneven_and_large_censuses() {
        for countries in [67, 1500] {
            let mut expected = parts(countries);
            for part in &mut expected {
                part.capture();
            }
            let mut actual = std::thread::scope(|scope| {
                let jobs: Vec<_> = parts(countries)
                    .into_iter()
                    .map(|mut part| {
                        scope.spawn(move || {
                            part.capture();
                            part
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .rev()
                    .map(|job| job.join().unwrap())
                    .collect::<Vec<_>>()
            });
            actual.sort_unstable_by_key(|part| part.ordinal);
            assert_eq!(demands(&actual), demands(&expected));
        }
    }
    #[test]
    fn optional_cold_storage_denial_never_calls_the_terminal_hook() {
        const NAME: &str = "optional_cold_storage_denial_never_calls_the_terminal_hook";
        const ENV: &str = "BATTLE_PLAN_COLD_STORAGE_TEST";
        if std::env::var(ENV).ok().as_deref() != Some(NAME) {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        &format!("production::design_changes::storage::cold_setup_tests::{NAME}")
                    ])
                    .env(ENV, NAME)
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
        fn failed() {
            panic!("optional cold storage allocation became terminal")
        }
        let mut parts = parts(67);
        engine_allocation::set_hooks(None, Some(failed));
        collector_test_support::with_allocations_denied(|| {
            for part in &mut parts {
                part.capture();
            }
        });
        engine_allocation::set_hooks(None, None);
        assert_eq!(demands(&parts)[0].1, 0);
    }
}
