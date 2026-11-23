use super::{design_context::Snapshot, *};
use collector_observers::world_input_changes::{Changes, Domain, Observer, Token};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
struct Held {
    token: Option<Token>,
    snapshot: Snapshot,
}
static HELD: OnceLock<Mutex<Held>> = OnceLock::new();
#[derive(Default)]
pub(super) struct Observation {
    pub order: u64,
    pub snapshot: Snapshot,
}
static ORDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
impl Held {
    fn capture(&mut self, memory: &impl Memory, base: usize, changes: &impl Changes) -> Snapshot {
        let before = changes.token(Domain::GameFog);
        if before.is_none() {
            self.token = None;
            return None;
        }
        if before == self.token {
            return self.snapshot.clone();
        }
        let value = engine_sites::at(base, engine_events::map_event_sites::FOG_OF_WAR_RVA)
            .and_then(|address| engine_native_memory::read::<u8>(memory, address));
        self.token = None;
        let value = value.filter(|_| changes.token(Domain::GameFog) == before)?;
        let equal = self.snapshot.as_ref().is_some_and(|snapshot| {
            matches!(snapshot.as_ref(), rows::TableRows::DesignObserverContext(rows) if rows.len() == 1 && rows[0].fog == value)
        });
        if !equal {
            let mut values = engine_allocation::vec(1).ok()?;
            values.push(rows::DesignObserverContextRow { fog: value });
            self.snapshot = Some(Arc::new(rows::TableRows::DesignObserverContext(values)));
        }
        self.token = before;
        self.snapshot.clone()
    }
}
pub(super) fn capture(memory: &impl Memory, base: usize) -> Observation {
    let _reads = read_accounting::table(TableId::DesignObserverContext);
    let mut held = HELD
        .get_or_init(|| Mutex::new(Held::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let snapshot = held.capture(memory, base, &Observer);
    Observation {
        order: ORDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        snapshot,
    }
}
pub(super) fn input(snapshot: &Snapshot) -> TableInput {
    snapshot.as_ref().map_or_else(
        || unavailable("game fog writer certificate or singleton read unavailable").into(),
        |rows| TableInput::SharedComplete(Arc::clone(rows)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct MemoryByte {
        reads: Cell<usize>,
        value: Cell<Option<u8>>,
    }
    impl Memory for MemoryByte {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert_eq!(address, engine_events::map_event_sites::FOG_OF_WAR_RVA.0);
            assert_eq!(output.len(), 1);
            self.reads.set(self.reads.get() + 1);
            if let Some(value) = self.value.get() {
                output[0] = value;
                true
            } else {
                false
            }
        }
    }
    struct ChangesToken(Mutex<Option<Token>>);
    impl Changes for ChangesToken {
        fn token(&self, domain: Domain) -> Option<Token> {
            assert_eq!(domain, Domain::GameFog);
            *self.0.lock().unwrap()
        }
        fn country(&self, _: usize) -> Option<[Token; 2]> {
            unreachable!()
        }
    }
    #[test]
    fn fog_reads_only_initial_or_changed_tokens_and_refuses_unobserved_or_failed_values() {
        let memory = MemoryByte {
            reads: Cell::new(0),
            value: Cell::new(Some(255)),
        };
        let changes = ChangesToken(Mutex::new(Some((1, 1))));
        let mut held = Held::default();
        let initial = held.capture(&memory, 0, &changes).unwrap();
        let quiet =
            collector_test_support::with_allocations_denied(|| held.capture(&memory, 0, &changes))
                .unwrap();
        assert!(Arc::ptr_eq(&initial, &quiet));
        assert_eq!(memory.reads.get(), 1);
        *changes.0.lock().unwrap() = None;
        assert!(held.capture(&memory, 0, &changes).is_none());
        assert_eq!(memory.reads.get(), 1);
        *changes.0.lock().unwrap() = Some((1, 2));
        memory.value.set(None);
        assert!(held.capture(&memory, 0, &changes).is_none());
        memory.value.set(Some(0));
        let next = held.capture(&memory, 0, &changes).unwrap();
        assert!(!Arc::ptr_eq(&initial, &next));
        assert!(
            matches!(next.as_ref(),rows::TableRows::DesignObserverContext(rows) if rows[0].fog == 0)
        );
    }
}
