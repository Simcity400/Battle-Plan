use super::*;
use std::{
    collections::HashMap,
    sync::{Arc, MutexGuard},
};

type Entry = (usize, Arc<rows::VariantDesignState>);
static HELD: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();
fn cache() -> MutexGuard<'static, HashMap<u64, Entry>> {
    HELD.get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(in super::super) struct Certified(MutexGuard<'static, HashMap<u64, Entry>>);
impl Certified {
    pub(in super::super) fn get(
        &self,
        source: &native::RegistryRow,
    ) -> Option<Arc<rows::VariantDesignState>> {
        self.0
            .get(&source.id)
            .filter(|(pointer, _)| *pointer == source.pointer())
            .map(|(_, state)| Arc::clone(state))
    }
}
pub(in super::super) fn certified() -> Option<Certified> {
    (native::mutations::WRITER_AUDIT_REFUSAL.is_none()
        && SAVED_BASELINE.load(Ordering::Acquire)
        && SOURCE_BASELINE.load(Ordering::Acquire)
        && LIVENESS_BASELINE.load(Ordering::Acquire)
        && !RECOVER.load(Ordering::Acquire)
        && !pending()
        && active() == 0)
        .then(|| Certified(cache()))
}
pub(in super::super) struct Census {
    proof: CensusProof,
}
pub(in super::super) struct CensusProof {
    revision: u64,
    epoch: u64,
    dependencies: u64,
}
impl CensusProof {
    pub(in super::super) fn is_current(&self) -> bool {
        enabled()
            && active() == 0
            && self.epoch == EPOCH.load(Ordering::Acquire)
            && self.revision == DESIGN_GATE.revision()
            && self.dependencies == CENSUS_REVISION.load(Ordering::Acquire)
            && !repair::pending()
    }
}
impl Census {
    pub(in super::super) fn get(
        &self,
        source: &native::RegistryRow,
    ) -> Option<Arc<rows::VariantDesignState>> {
        {
            let store = STORE
                .get()?
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let watch = store.watched.get(&source.pointer())?;
            if store.epoch != self.proof.epoch
                || watch.id != source.id
                || watch.dirty
                || !sources::certified_bindings().unchanged(source.id, &store.interests)
            {
                return None;
            }
        }
        cache()
            .get(&source.id)
            .filter(|(pointer, _)| *pointer == source.pointer())
            .map(|(_, state)| Arc::clone(state))
    }
    pub(in super::super) fn is_current(&self) -> bool {
        self.proof.is_current()
    }
    pub(in super::super) fn release(self) -> CensusProof {
        self.proof
    }
}
#[cfg(test)]
thread_local! {
    static AFTER_CENSUS_ADMISSION: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
}
pub(in super::super) fn census() -> Option<Census> {
    let store = STORE
        .get()?
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let revision = DESIGN_GATE.revision();
    let dependencies = CENSUS_REVISION.load(Ordering::Acquire);
    if native::mutations::WRITER_AUDIT_REFUSAL.is_some()
        || !enabled()
        || active() != 0
        || repair::pending()
        || store.epoch != EPOCH.load(Ordering::Acquire)
        || dependencies != CENSUS_BASELINE.load(Ordering::Acquire)
    {
        return None;
    }
    #[cfg(test)]
    AFTER_CENSUS_ADMISSION.with(|hook| {
        if let Some(hook) = hook.take() {
            hook();
        }
    });
    Some(Census {
        proof: CensusProof {
            revision,
            epoch: store.epoch,
            dependencies,
        },
    })
}
pub(super) fn seed(
    registry: &[native::RegistryRow],
    variants: &Variants,
    watches: &HashMap<usize, Watch>,
) -> Result<(), &'static str> {
    let mut held = cache();
    held.retain(|id, (pointer, _)| watches.get(pointer).is_some_and(|watch| watch.id == *id));
    let additional = registry.len().saturating_sub(held.len());
    engine_allocation::reserve_map(&mut held, additional)
        .map_err(|_| "variant held state allocation failed")?;
    for (source, row) in registry.iter().zip(&variants.definitions.rows) {
        if row.identity.id != source.id {
            return Err("variant held state identity mismatch");
        }
        if held.get(&source.id).is_some_and(|(pointer, state)| {
            *pointer == source.pointer() && Arc::ptr_eq(state, &row.design)
        }) {
            continue;
        }
        super::witness::remove(source.pointer());
        held.insert(source.id, (source.pointer(), Arc::clone(&row.design)));
    }
    Ok(())
}
pub(super) fn apply_shared(
    events: &[Event],
    rows: &[rows::VariantChangeRow],
    snapshots: &snapshots::Shared,
) -> Result<(), &'static str> {
    if events.is_empty() {
        return Ok(());
    }
    if events
        .iter()
        .filter(|event| matches!(event.kind, Kind::Design))
        .count()
        != rows
            .iter()
            .filter(|row| row.sources.is_none() && row.organisation.is_none())
            .count()
            + snapshots.len()
    {
        return Err("variant held state event count mismatch");
    }
    let mut held = cache();
    apply_to_shared(&mut held, events, rows, snapshots)
}
#[cfg(test)]
fn apply_to(
    held: &mut HashMap<u64, Entry>,
    events: &[Event],
    rows: &[rows::VariantChangeRow],
) -> Result<(), &'static str> {
    apply_to_shared(held, events, rows, &snapshots::Shared::default())
}
fn apply_to_shared(
    held: &mut HashMap<u64, Entry>,
    events: &[Event],
    rows: &[rows::VariantChangeRow],
    snapshots: &snapshots::Shared,
) -> Result<(), &'static str> {
    let mut events = events
        .iter()
        .filter(|event| matches!(event.kind, Kind::Design));
    for change in snapshots.iter(rows) {
        let row = match change? {
            snapshots::Change::Snapshot(snapshot) => {
                let event = events
                    .next()
                    .ok_or("variant held state event count mismatch")?;
                engine_allocation::reserve_map(held, 1)
                    .map_err(|_| "variant held state allocation failed")?;
                held.insert(
                    snapshot.variant,
                    (event.pointer, Arc::clone(&snapshot.value.definition.design)),
                );
                continue;
            }
            snapshots::Change::Row(row) => row,
        };
        if row.organisation.is_some() {
            continue;
        }
        if let Some(sources) = &row.sources {
            let (pointer, previous) = held
                .get(&row.variant)
                .ok_or("variant source change lacks preceding state")?;
            let mut state = rows::NativeValue::try_clone(previous.as_ref())
                .map_err(|_| "variant source change allocation failed")?;
            state.bonus_sources = engine_allocation::copy_vec(&sources.direct)
                .map_err(|_| "variant source change allocation failed")?;
            state.named_sources = engine_allocation::copy_vec(&sources.named)
                .map_err(|_| "variant source change allocation failed")?;
            let pointer = *pointer;
            super::witness::remove(pointer);
            held.insert(row.variant, (pointer, Arc::new(state)));
            continue;
        }
        let event = events
            .next()
            .ok_or("variant held state event count mismatch")?;
        if row.removed {
            held.remove(&row.variant);
            continue;
        }
        if let Some(scalar) = &row.scalar {
            let (pointer, previous) = held
                .get(&row.variant)
                .ok_or("variant scalar has no preceding state")?;
            if *pointer != event.pointer {
                return Err("variant scalar address mismatch");
            }
            if scalar.changes_design() {
                let mut state = rows::NativeValue::try_clone(previous.as_ref())
                    .map_err(|_| "variant scalar allocation failed")?;
                scalar
                    .apply_design(&mut state)
                    .map_err(|_| "variant scalar apply failed")?;
                let pointer = *pointer;
                held.insert(row.variant, (pointer, Arc::new(state)));
            }
            continue;
        }
        let state = if let Some(flags) = row.flags {
            let (pointer, previous) = held
                .get(&row.variant)
                .ok_or("variant held eligibility has no preceding state")?;
            if *pointer != event.pointer {
                return Err("variant held eligibility address mismatch");
            }
            let current = [
                previous.can_upgrade_type,
                previous.can_upgrade_variant,
                previous.can_upgrade_modules,
            ];
            if current == flags {
                continue;
            }
            let mut state = rows::NativeValue::try_clone(previous.as_ref())
                .map_err(|_| "variant held state allocation failed")?;
            state.can_upgrade_type = flags[0];
            state.can_upgrade_variant = flags[1];
            state.can_upgrade_modules = flags[2];
            state
        } else {
            let state = row
                .snapshot
                .as_ref()
                .ok_or("variant held event lacks its snapshot")?
                .definition
                .design
                .require("VariantChanges.design")
                .map_err(|_| "variant held event lacks its design state")?;
            rows::NativeValue::try_clone(state)
                .map_err(|_| "variant held state allocation failed")?
        };
        engine_allocation::reserve_map(held, 1)
            .map_err(|_| "variant held state allocation failed")?;
        held.insert(row.variant, (event.pointer, Arc::new(state)));
    }
    Ok(())
}
pub(super) fn check_lifetimes(events: &[Event]) -> Result<(), &'static str> {
    if !events.iter().any(|event| event.removed) {
        return Ok(());
    }
    let mut retired = std::collections::HashSet::new();
    engine_allocation::reserve_set(&mut retired, events.len())
        .map_err(|_| "variant lifetime dependency allocation failed")?;
    retired.extend(
        events
            .iter()
            .filter(|event| event.removed)
            .map(|event| event.variant),
    );
    if cache()
        .values()
        .any(|(_, state)| state.parent.is_some_and(|id| retired.contains(&id)))
    {
        unobserved!();
        return Err("variant parent identity has a consumed saved binding");
    }
    Ok(())
}
pub(super) fn release() {
    cache().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_events_keep_untouched_designs_and_name_only_edits_share_deep_storage() {
        let previous = Arc::new(rows::VariantDesignState {
            manpower: 99,
            ..Default::default()
        });
        let mut held = HashMap::from([
            (1, (100, Arc::clone(&previous))),
            (2, (200, Arc::clone(&previous))),
        ]);
        let event = Event {
            kind: Kind::Design,
            sequence: 1,
            hour: 7,
            pointer: 100,
            variant: 1,
            removed: false,
            start: 0,
            end: 0,
        };
        let row = rows::VariantChangeRow {
            sequence: 1,
            hour: 7,
            variant: 1,
            scalar: Some(rows::VariantScalarChange {
                display_name: Some(b"saved".to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let rows = [row];
        collector_test_support::with_allocations_denied(|| apply_to(&mut held, &[event], &rows))
            .unwrap();
        assert!(Arc::ptr_eq(&held[&1].1, &previous));
        let row = rows::VariantChangeRow {
            sequence: 2,
            hour: 7,
            variant: 1,
            scalar: Some(rows::VariantScalarChange {
                auto_upgraded: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        };
        apply_to(&mut held, &[event], &[row]).unwrap();
        assert_eq!(held[&1].1.auto_upgraded, 1);
        assert_eq!(held[&1].1.manpower, 99);
        assert!(Arc::ptr_eq(&held[&2].1, &previous));
    }
    #[test]
    fn variant_held_values_replace_only_touched_designs_and_refuse_unknown_flags() {
        let original = Arc::new(rows::VariantDesignState {
            manpower: 9,
            ..Default::default()
        });
        let quiet = Arc::clone(&original);
        let mut held = HashMap::from([(1, (100, original)), (2, (200, Arc::clone(&quiet)))]);
        collector_test_support::with_allocations_denied(|| apply_to(&mut held, &[], &[])).unwrap();
        assert!(Arc::ptr_eq(&held[&1].1, &quiet));
        let event = Event {
            kind: Kind::Design,
            sequence: 1,
            hour: 7,
            pointer: 100,
            variant: 1,
            removed: false,
            start: 0,
            end: 0,
        };
        let row = rows::VariantChangeRow {
            sequence: 1,
            hour: 7,
            variant: 1,
            flags: Some([1, 2, 3]),
            ..Default::default()
        };
        apply_to(&mut held, &[event], std::slice::from_ref(&row)).unwrap();
        assert_eq!(held[&1].1.can_upgrade_modules, 3);
        assert_eq!(held[&1].1.manpower, 9);
        assert!(Arc::ptr_eq(&held[&2].1, &quiet));
        assert!(!Arc::ptr_eq(&held[&1].1, &quiet));
        assert!(
            apply_to(
                &mut held,
                &[Event {
                    pointer: 101,
                    ..event
                }],
                std::slice::from_ref(&row)
            )
            .is_err()
        );
        assert!(
            apply_to(
                &mut held,
                &[Event {
                    variant: 3,
                    ..event
                }],
                &[rows::VariantChangeRow {
                    variant: 3,
                    ..row.clone()
                }]
            )
            .is_err()
        );
        apply_to(
            &mut held,
            &[Event {
                removed: true,
                ..event
            }],
            &[rows::VariantChangeRow {
                removed: true,
                ..row
            }],
        )
        .unwrap();
        assert!(!held.contains_key(&1));
        assert!(Arc::ptr_eq(&held[&2].1, &quiet));
    }
}

#[cfg(test)]
mod census_tests {
    use super::super::super::variant_scale_tests::{Space, space};
    use super::*;
    use collector_observers::world_input_changes;
    use std::{cell::Cell, sync::Barrier};

    fn acquire(
        memory: &impl Memory,
        base: usize,
        sources: &[native::RegistryRow],
        carry: bool,
    ) -> Variants {
        let mut variants = Variants::vacant();
        variants.capture_state_carried(
            memory,
            base,
            Sources {
                rows: sources,
                outcome: Outcome::Rows { len: sources.len() },
            },
            7,
            true,
            carry,
        );
        variants
    }
    fn views(variants: Variants) -> [TableView; 4] {
        let mut captured = super::super::super::Captured::with_capacities(&Capacities::default());
        captured.restore_variants(variants);
        let [_, _, _, definitions, modules, stats, upgrades, ..] = captured.tables();
        [definitions, modules, stats, upgrades]
    }
    fn setup() -> (Space, Vec<native::RegistryRow>, Variants) {
        let space = space(2, 4);
        writers::prepare(8).unwrap();
        let mut sources = Vec::new();
        assert_eq!(
            native::walk_registry(&space.memory(), space.base, space.game_state, &mut sources),
            Outcome::Rows { len: 8 }
        );
        let baseline = acquire(&space.memory(), space.base, &sources[..7], false);
        assert_eq!(baseline.definitions.outcome, Outcome::Rows { len: 7 });
        let watches = sources[..7]
            .iter()
            .map(|source| {
                (
                    source.pointer(),
                    Watch {
                        id: source.id,
                        pointer: source.pointer(),
                        flags: [0; 3],
                        dirty: false,
                    },
                )
            })
            .collect();
        repair::prepare(&watches).unwrap();
        seed(&sources[..7], &baseline, &watches).unwrap();
        let mut interests = sources::Interests::default();
        liveness::seed(&baseline, &mut interests).unwrap();
        sources::seed(&baseline, &mut interests).unwrap();
        assert!(
            STORE
                .set(Mutex::new(Store {
                    batch: Batch::empty(),
                    spare: Batch::empty(),
                    watched: watches,
                    published: Published::default(),
                    interests,
                    epoch: EPOCH.load(Ordering::Acquire),
                }))
                .is_ok()
        );
        ENABLED.store(true, Ordering::Release);
        collector_observers::world_input_changes::set_design_watch_enabled_for_test(true);
        SAVED_BASELINE.store(true, Ordering::Release);
        SOURCE_BASELINE.store(true, Ordering::Release);
        LIVENESS_BASELINE.store(true, Ordering::Release);
        RECOVER.store(false, Ordering::Release);
        RECOVERY_READ.store(false, Ordering::Release);
        CENSUS_BASELINE.store(CENSUS_REVISION.load(Ordering::Acquire), Ordering::Release);
        (space, sources, baseline)
    }
    struct Meter<'a> {
        space: &'a Space,
        bytes: Cell<usize>,
        unreadable: Option<usize>,
    }
    impl Memory for Meter<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .unreadable
                .is_some_and(|at| (address..address + output.len()).contains(&at))
            {
                return false;
            }
            self.bytes.set(self.bytes.get() + output.len());
            self.space.memory().copy(address, output)
        }
    }
    #[test]
    fn variant_recovery_rechecks_a_writer_waiting_after_census_admission() {
        if !isolated() {
            return;
        }
        static PAUSE: OnceLock<Barrier> = OnceLock::new();
        fn admitted() {
            let pause = PAUSE.get().unwrap();
            pause.wait();
            pause.wait();
        }
        let (space, sources, baseline) = setup();
        BASE.store(space.base, Ordering::Release);
        let pointer = sources[0].pointer();
        let pause = PAUSE.get_or_init(|| Barrier::new(2));
        fn waiting() {
            PAUSE.get().unwrap().wait();
        }
        let writer = std::thread::spawn(move || {
            pause.wait();
            BEFORE_STORE_WAIT.set(Some(waiting));
            collector_test_support::with_allocations_denied(|| unsafe {
                around::<0>([pointer, 0, 23, 0], refresh_bonus)
            });
        });
        AFTER_CENSUS_ADMISSION.with(|hook| hook.set(Some(admitted)));
        let recovered = acquire(&space.memory(), space.base, &sources[..7], true);
        writer.join().unwrap();
        assert!(repair::pending());
        assert!(pending());
        assert!(LOCK_CONTENTIONS.load(Ordering::Relaxed) >= 1);
        assert_eq!(recovered.definitions.rows[0].design.design_team_traits, 23);
        assert!(!Arc::ptr_eq(
            &recovered.definitions.rows[0].design,
            &baseline.definitions.rows[0].design
        ));
        assert_eq!(
            views(recovered),
            views(acquire(&space.memory(), space.base, &sources[..7], false))
        );
    }
    #[test]
    fn partitioned_recovery_never_holds_store_during_native_reads_and_rechecks_its_proof() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        struct Invalidate<'a> {
            space: &'a Space,
            trigger: usize,
            changed: Cell<bool>,
        }
        impl Memory for Invalidate<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert!(STORE.get().unwrap().try_lock().is_ok());
                if address == self.trigger && !self.changed.replace(true) {
                    unobserved!();
                }
                self.space.memory().copy(address, output)
            }
        }
        let memory = Invalidate {
            space: &space,
            trigger: sources[0].pointer() + native::EQUIPMENT_TOKEN,
            changed: Cell::new(false),
        };
        let mut recovered = Variants::vacant();
        recovered.capture_partitioned(
            space.base,
            Sources {
                rows: &sources,
                outcome: Outcome::Rows { len: sources.len() },
            },
            7,
            true,
            true,
            3,
            &mut |mut parts| {
                for part in &mut parts {
                    part.capture(&memory, space.base);
                }
                Ok(parts)
            },
        );
        assert!(memory.changed.get());
        assert!(
            recovered
                .definitions
                .rows
                .iter()
                .all(|row| row.source_links.is_some())
        );
        assert_eq!(
            views(recovered),
            views(acquire(&space.memory(), space.base, &sources, false))
        );
    }
    #[test]
    fn variant_census_final_proof_rechecks_pending_receiver_marks() {
        if !isolated() {
            return;
        }
        let (_space, sources, _) = setup();
        let proof = census().unwrap();
        repair::mark(sources[0].pointer());
        assert_eq!(proof.proof.revision, DESIGN_GATE.revision());
        assert_eq!(
            proof.proof.dependencies,
            CENSUS_REVISION.load(Ordering::Acquire)
        );
        assert!(!proof.is_current());
    }
    #[test]
    fn variant_recovery_reads_only_marked_and_new_deep_values_and_matches_a_full_census() {
        if !isolated() {
            return;
        }
        let (mut space, sources, baseline) = setup();
        let changed = sources[1];
        let removed = sources[2];
        let at = changed.pointer() + native::AUTO_UPGRADED - space.bytes.as_ptr() as usize;
        space.bytes[at] = 1;
        mark_design(&space.memory(), space.base, changed.pointer(), false);
        let mut changes = Vec::new();
        assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
        assert_eq!(changes.len(), 1);
        let snapshot = changes[0].snapshot.as_ref().unwrap();
        assert_eq!(snapshot.definition.design.require("design").unwrap().auto_upgraded, 1);
        assert!(!needs_full());
        let sources: Vec<_> = sources
            .into_iter()
            .filter(|source| source.id != removed.id)
            .collect();
        let meter = Meter {
            space: &space,
            bytes: Cell::new(0),
            unreadable: None,
        };
        let reduced = acquire(&meter, space.base, &sources, true);
        let reduced_bytes = meter.bytes.replace(0);
        let full = acquire(&meter, space.base, &sources, false);
        let full_bytes = meter.bytes.get();
        assert!(
            reduced_bytes < full_bytes,
            "{reduced_bytes} must be less than {full_bytes}"
        );
        assert!(Arc::ptr_eq(
            &reduced.definitions.rows[0].design,
            &baseline.definitions.rows[0].design
        ));
        assert!(!Arc::ptr_eq(
            &reduced.definitions.rows[1].design,
            &baseline.definitions.rows[1].design
        ));
        assert_eq!(reduced.definitions.rows[1].design.auto_upgraded, 1);
        assert!(
            reduced
                .definitions
                .rows
                .last()
                .unwrap()
                .source_links
                .is_some()
        );
        assert!(
            !reduced
                .definitions
                .rows
                .iter()
                .any(|row| row.identity.id == removed.id)
        );
        assert_eq!(views(reduced), views(full));

        let new = sources.last().unwrap().pointer();
        let failing = Meter {
            space: &space,
            bytes: Cell::new(0),
            unreadable: Some(new + native::AUTO_UPGRADED),
        };
        let reduced = acquire(&failing, space.base, &sources, true);
        let full = acquire(&failing, space.base, &sources, false);
        assert!(matches!(
            reduced.definitions.outcome,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(views(reduced), views(full));

        let fresh = acquire(&space.memory(), space.base, &sources, true);
        let watches = sources
            .iter()
            .map(|source| {
                (
                    source.pointer(),
                    Watch {
                        id: source.id,
                        pointer: source.pointer(),
                        flags: [0; 3],
                        dirty: false,
                    },
                )
            })
            .collect();
        seed(&sources, &fresh, &watches).unwrap();
        assert!(!cache().contains_key(&removed.id));
        assert_eq!(cache().len(), sources.len());
    }
    fn shared_bonus_refresh() -> world_input_changes::Original {
        world_input_changes::forwarder(Some(world_input_changes::Forwarder {
            call: forwarded,
            enabled: |_| {},
        }));
        world_input_changes::design_bonus_refresh_for_test(refresh_bonus)
    }
    unsafe extern "system-unwind" fn rebuilt(_: usize, _: usize, _: usize, _: usize) -> usize {
        19
    }
    unsafe extern "system-unwind" fn refresh_bonus(
        variant: usize,
        pause: usize,
        traits: usize,
        rebuild: usize,
    ) -> usize {
        unsafe {
            std::ptr::write_unaligned(
                (variant + native::DESIGN_TEAM_TRAITS) as *mut i32,
                traits as i32,
            );
        }
        if pause != 0 {
            let pause = unsafe { &*(pause as *const Barrier) };
            pause.wait();
            pause.wait();
        }
        if rebuild != 0 {
            unsafe { around::<0>([variant, 0, 0, 0], rebuilt) }
        } else {
            19
        }
    }
    #[test]
    fn an_unchanged_marked_design_keeps_its_binding_and_carry_and_enters_the_ledger() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        let design = sources[1];
        mark_design(&space.memory(), space.base, design.pointer(), false);
        let mut changes = Vec::new();
        assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
        assert!(changes[0].snapshot.is_some());
        let certified = {
            let store = STORE.get().unwrap().lock().unwrap();
            sources::certified_bindings().unchanged(design.id, &store.interests)
        };
        mark_design(&space.memory(), space.base, design.pointer(), false);
        assert_eq!(drain(space.base, 8, &mut changes), Ok(1));
        assert_eq!(changes.len(), 1);
        assert!(changes[0].snapshot.is_none());
        let store = STORE.get().unwrap().lock().unwrap();
        let watch = store.watched[&design.pointer()];
        assert_eq!(changes[0].flags, Some(watch.flags));
        assert!(!watch.dirty);
        assert_eq!(
            sources::certified_bindings().unchanged(design.id, &store.interests),
            certified
        );
    }
    #[test]
    fn variant_census_rejects_unforwarded_shared_refresh_before_inner_rebuild() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        BASE.store(space.base, Ordering::Release);
        let refresh = shared_bonus_refresh();
        let proof = census().unwrap();
        let revision = DESIGN_GATE.revision();
        let pause = Barrier::new(2);
        let args = [
            sources[0].pointer(),
            &pause as *const Barrier as usize,
            23,
            1,
        ];
        let writer =
            std::thread::spawn(move || unsafe { refresh(args[0], args[1], args[2], args[3]) });
        pause.wait();
        let shared_active = world_input_changes::design_forwarder_active();
        let private_active = DESIGN_GATE.active();
        let private_revision = DESIGN_GATE.revision();
        let watch_dirty = STORE.get().unwrap().lock().unwrap().watched[&sources[0].pointer()].dirty;
        let traits = engine_native_memory::read::<i32>(
            &space.memory(),
            sources[0].pointer() + native::DESIGN_TEAM_TRAITS,
        );
        let current = proof.is_current();
        drop(proof);
        let admitted = census().is_some();
        let interval = drain(space.base, 7, &mut Vec::new());
        pause.wait();
        let result = writer.join().unwrap();
        assert_eq!(shared_active, 1);
        assert_eq!(private_active, 0);
        assert_eq!(private_revision, revision);
        assert!(!watch_dirty);
        assert_eq!(traits, Some(23));
        assert!(
            !current,
            "the shared original invalidates an acquired census"
        );
        assert!(!admitted, "the shared original prevents census admission");
        assert_eq!(world_input_changes::design_forwarder_active(), 0);
        assert!(
            DESIGN_GATE.revision() > revision,
            "the inner rebuild arrives later"
        );
        assert_eq!(result, 19);
        assert_eq!(
            interval,
            Err("variant change interval has an unobserved or running writer")
        );
        assert_eq!(RUNNING_REFUSALS.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn variant_recovery_reacquires_deep_values_after_a_completed_unforwarded_bonus_refresh() {
        if !isolated() {
            return;
        }
        let (space, sources, baseline) = setup();
        let refresh = shared_bonus_refresh();
        observe_world_forwarder(true);
        let proof = census().unwrap();
        observe_world_forwarder(false);
        assert!(
            !proof.is_current(),
            "closing dispatch revokes acquired proofs too"
        );
        drop(proof);
        let revision = DESIGN_GATE.revision();
        assert_eq!(unsafe { refresh(sources[0].pointer(), 0, 23, 0) }, 19);
        assert_eq!(world_input_changes::design_forwarder_active(), 0);
        assert_eq!(DESIGN_GATE.revision(), revision);
        assert!(!STORE.get().unwrap().lock().unwrap().watched[&sources[0].pointer()].dirty);
        assert!(
            census().is_none(),
            "closing shared dispatch revokes the old census"
        );
        observe_world_forwarder(true);
        assert!(
            census().is_none(),
            "reopening dispatch cannot restore that proof"
        );
        let recovered = acquire(&space.memory(), space.base, &sources, true);
        assert_eq!(recovered.definitions.rows[0].design.design_team_traits, 23);
        assert!(!Arc::ptr_eq(
            &recovered.definitions.rows[0].design,
            &baseline.definitions.rows[0].design,
        ));
        assert_eq!(
            views(recovered),
            views(acquire(&space.memory(), space.base, &sources, false))
        );
    }
    #[test]
    fn variant_recovery_keeps_shared_observation_and_reuses_untouched_deep_values() {
        if !isolated() {
            return;
        }
        let (space, sources, baseline) = setup();
        BASE.store(space.base, Ordering::Release);
        let refresh = shared_bonus_refresh();
        observe_world_forwarder(true);
        assert!(collector_test_support::with_allocations_denied(|| {
            retire(&space.memory(), space.base, sources[1].pointer())
        }));
        assert_eq!(
            drain(space.base, 7, &mut Vec::new()),
            Err(STORAGE_FAILED)
        );
        assert!(!SAVED_BASELINE.load(Ordering::Acquire));
        assert!(
            census().is_some(),
            "a local refusal preserves independent proofs"
        );
        let revision = DESIGN_GATE.revision();
        assert_eq!(unsafe { refresh(sources[0].pointer(), 0, 23, 0) }, 19);
        assert!(DESIGN_GATE.revision() > revision);
        assert!(census().is_none());
        assert_eq!(drain(space.base, 7, &mut Vec::new()), Ok(1));
        let recovered = acquire(&space.memory(), space.base, &sources, true);
        assert_eq!(recovered.definitions.rows[0].design.design_team_traits, 23);
        assert!(Arc::ptr_eq(
            &recovered.definitions.rows[2].design,
            &baseline.definitions.rows[2].design,
        ));
        assert_eq!(
            views(recovered),
            views(acquire(&space.memory(), space.base, &sources, false))
        );
    }
    #[test]
    fn variant_recovery_never_carries_uncertified_dependencies_lifetimes_or_overlapping_writers() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        assert!(census().is_some());
        CENSUS_REVISION.fetch_add(1, Ordering::AcqRel);
        assert!(
            census().is_none(),
            "a dependency write must revoke recovery carry"
        );
        let failing = Meter {
            space: &space,
            bytes: Cell::new(0),
            unreadable: Some(sources[0].pointer() + native::AUTO_UPGRADED),
        };
        assert_eq!(
            views(acquire(&failing, space.base, &sources, true)),
            views(acquire(&failing, space.base, &sources, false))
        );
        CENSUS_BASELINE.store(CENSUS_REVISION.load(Ordering::Acquire), Ordering::Release);
        let held_flight = DESIGN_GATE.enter(false);
        assert!(census().is_none());
        drop(held_flight);
        let proof = census().unwrap();
        unobserved!();
        assert!(!proof.is_current());
        drop(proof);
        assert!(
            census().is_none(),
            "draining MISSED must not restore the lost proof"
        );
        MISSED.store(false, Ordering::Release);
        assert!(census().is_none());
        CENSUS_BASELINE.store(CENSUS_REVISION.load(Ordering::Acquire), Ordering::Release);
        EPOCH.fetch_add(1, Ordering::AcqRel);
        assert!(census().is_none());
    }
    #[test]
    fn variant_recovery_reacquires_all_deep_values_when_its_certificate_changes_during_the_read() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        struct Invalidate<'a> {
            space: &'a Space,
            trigger: usize,
            changed: Cell<bool>,
        }
        impl Memory for Invalidate<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == self.trigger && !self.changed.replace(true) {
                    unobserved!();
                }
                self.space.memory().copy(address, output)
            }
        }
        let memory = Invalidate {
            space: &space,
            trigger: sources[0].pointer() + native::EQUIPMENT_TOKEN,
            changed: Cell::new(false),
        };
        let reduced = acquire(&memory, space.base, &sources, true);
        assert!(memory.changed.get());
        assert!(
            reduced
                .definitions
                .rows
                .iter()
                .all(|row| row.source_links.is_some()),
            "an invalidated read must discard every carried value"
        );
        assert_eq!(
            views(reduced),
            views(acquire(&space.memory(), space.base, &sources, false))
        );
    }
    #[test]
    fn partitioned_recovery_retries_every_definition_after_census_invalidation() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        struct Invalidate<'a> {
            space: &'a Space,
            trigger: usize,
            changed: Cell<bool>,
        }
        impl Memory for Invalidate<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == self.trigger && !self.changed.replace(true) {
                    unobserved!();
                }
                self.space.memory().copy(address, output)
            }
        }
        let memory = Invalidate {
            space: &space,
            trigger: sources[0].pointer() + native::EQUIPMENT_TOKEN,
            changed: Cell::new(false),
        };
        let mut reduced = Variants::vacant();
        let mut passes = 0;
        reduced.capture_partitioned(
            space.base,
            Sources {
                rows: &sources,
                outcome: Outcome::Rows { len: sources.len() },
            },
            7,
            true,
            true,
            2,
            &mut |mut parts| {
                passes += 1;
                for part in &mut parts {
                    part.capture(&memory, space.base);
                }
                Ok(parts)
            },
        );
        assert!(memory.changed.get());
        assert_eq!(passes, 2);
        assert!(
            reduced
                .definitions
                .rows
                .iter()
                .all(|row| row.source_links.is_some())
        );
        assert_eq!(
            views(reduced),
            views(acquire(&space.memory(), space.base, &sources, false))
        );
    }
    #[test]
    fn failed_variant_recovery_revokes_proofs_before_the_next_native_interval() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        assert!(collector_test_support::with_allocations_denied(|| {
            retire(&space.memory(), space.base, sources[0].pointer())
        }));
        assert_eq!(
            drain(space.base, 7, &mut Vec::new()),
            Err(STORAGE_FAILED)
        );
        let failing = Meter {
            space: &space,
            bytes: Cell::new(0),
            unreadable: Some(sources[0].pointer() + native::AUTO_UPGRADED),
        };
        let mut variants = Variants::vacant();
        variants.capture(
            &failing,
            space.base,
            Sources {
                rows: &sources,
                outcome: Outcome::Rows { len: sources.len() },
            },
            7,
            false,
        );
        assert!(matches!(
            variants.definitions.outcome,
            Outcome::Unavailable { .. }
        ));
        assert!(
            !needs_full(),
            "a failed recovery must not create hourly retries"
        );
        assert!(
            census().is_none(),
            "forwarding may be withdrawn until the daily retry"
        );
    }
    #[test]
    fn variant_store_collision_keeps_the_design_mark() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        super::super::tests::with_store_contended(|| {
            mark_design(&space.memory(), space.base, sources[0].pointer(), false);
        });
        let mut changes = Vec::new();
        assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].variant, sources[0].id);
        assert!(changes[0].snapshot.is_some());
        assert_eq!(MISSED_REFUSALS.load(Ordering::Relaxed), 0);
        assert_eq!(RUNNING_REFUSALS.load(Ordering::Relaxed), 0);
        assert_eq!(REPAIRED_RECEIVERS.load(Ordering::Relaxed), 1);
        assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
        assert!(!needs_full());
    }
    #[test]
    fn variant_unwatched_draft_collisions_do_not_refuse_the_saved_interval() {
        if !isolated() {
            return;
        }
        let (space, sources, _) = setup();
        BASE.store(space.base, Ordering::Release);
        let draft = sources[7].pointer();
        let pause = Barrier::new(2);
        let pause_pointer = &pause as *const Barrier as usize;
        let writer = std::thread::spawn(move || unsafe {
            around::<0>([draft, pause_pointer, 23, 0], refresh_bonus)
        });
        pause.wait();
        let result = collector_test_support::with_allocations_denied(|| unsafe {
            around::<0>([draft, 0, 0, 0], rebuilt)
        });
        pause.wait();
        assert_eq!(writer.join().unwrap(), 19);
        assert_eq!(result, 19);
        collector_test_support::with_allocations_denied(|| {
            unrecorded_writer(Receiver::Removed, draft, None);
        });
        assert!(!MISSED.load(Ordering::Acquire));
        assert_eq!(drain(space.base, 7, &mut Vec::new()), Ok(0));
        assert!(!needs_full());
        assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
        unrecorded_writer(Receiver::Published, sources[7].pointer(), None);
        assert!(drain(space.base, 7, &mut Vec::new()).is_err());
        assert_eq!(RUNNING_REFUSALS.load(Ordering::Relaxed), 0);
        assert_eq!(MISSED_REFUSALS.load(Ordering::Relaxed), 1);
        assert!(!LAST_MISSED_AT.load(Ordering::Acquire).is_null());
    }
    #[test]
    fn variant_log_owner_collision_marks_known_receivers_without_revoking_unrelated_designs() {
        if !isolated() {
            return;
        }
        let (space, sources, baseline) = setup();
        collector_test_support::with_allocations_denied(|| {
            unrecorded_writer(Receiver::Variant, sources[1].pointer(), None);
            unrecorded_writer(Receiver::Variant, sources[2].pointer(), None);
        });
        assert!(
            census().is_none(),
            "pending atomic marks cannot certify deep reuse"
        );
        assert_eq!(drain(space.base, 7, &mut Vec::new()), Ok(2));
        let proof = census().unwrap();
        for at in [1, 2] {
            assert_eq!(
                proof.get(&sources[at]).as_deref(),
                Some(&*baseline.definitions.rows[at].design)
            );
        }
        assert!(Arc::ptr_eq(
            &proof.get(&sources[0]).unwrap(),
            &baseline.definitions.rows[0].design
        ));
        drop(proof);
        collector_test_support::with_allocations_denied(|| {
            unrecorded_writer(Receiver::Roster, 0, None)
        });
        assert!(
            census().is_none(),
            "an unknown receiver still revokes every value"
        );
    }
}
