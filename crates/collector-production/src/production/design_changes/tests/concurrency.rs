use super::*;
use crate::production::variant_scale_tests::{Space, space};
use std::sync::Barrier;

fn acquire(space: &Space, registry: &[native::RegistryRow]) -> Variants {
    let mut variants = Variants::vacant();
    variants.capture_state_carried(
        &space.memory(),
        space.base,
        Sources {
            rows: registry,
            outcome: Outcome::Rows {
                len: registry.len(),
            },
        },
        7,
        true,
        false,
    );
    assert_eq!(
        variants.definitions.outcome,
        Outcome::Rows {
            len: registry.len()
        }
    );
    variants
}
fn setup(population: usize) -> (Space, Vec<native::RegistryRow>, Variants) {
    let space = space(1, population);
    let mut registry = Vec::new();
    assert_eq!(
        native::walk_registry(&space.memory(), space.base, space.game_state, &mut registry),
        Outcome::Rows { len: population }
    );
    for row in &registry {
        unsafe { std::ptr::write_unaligned((row.pointer() + native::ORIGIN_LINK) as *mut i32, 0) };
    }
    BASE.store(space.base, Ordering::Release);
    ENABLED.store(true, Ordering::Release);
    collector_observers::world_input_changes::set_design_watch_enabled_for_test(true);
    let variants = acquire(&space, &registry);
    begin_full();
    begin_read();
    prepare(&registry, &variants);
    assert!(baseline_admitted());
    (space, registry, variants)
}
fn views(variants: Variants) -> [TableView; 4] {
    let mut captured = crate::production::Captured::with_capacities(&Capacities::default());
    captured.restore_variants(variants);
    let [_, _, _, definitions, modules, stats, upgrades, ..] = captured.tables();
    [definitions, modules, stats, upgrades]
}
fn table_ids() -> [TableId; 4] {
    [
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantStats,
        TableId::VariantUpgrades,
    ]
}
fn assert_published_equals_fresh(
    baseline: Variants,
    changes: Vec<rows::VariantChangeRow>,
    fresh: Variants,
) {
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut decoder = replay_frame::frame::FrameDecoder::new();
    let mut first = input(6);
    for (table, view) in table_ids().into_iter().zip(views(baseline)) {
        first.tables[table.index()] = view.into();
    }
    decoder
        .apply(&encoder.encode_input(first, true).unwrap())
        .unwrap();
    let scratch = crate::projection::ProjectionScratch::default();
    let projection = Projection::new(&encoder, 1, 7, false, &scratch);
    let mut captured = Captured::vacant();
    captured.outcome = Outcome::Rows { len: changes.len() };
    captured.rows = changes;
    let mut next = input(7);
    for (table, value) in table_ids()
        .into_iter()
        .zip(project(&captured, Some(&projection)))
    {
        next.tables[table.index()] = value;
    }
    let wire = encoder.encode_input(next, false).unwrap();
    let published = decoder.apply(&wire).unwrap();
    for (table, expected) in table_ids().into_iter().zip(views(fresh)) {
        assert_eq!(&*published.tables[table.index()], &expected, "{table:?}");
    }
}
struct OnlyDesign<'a> {
    space: &'a Space,
    forbidden: usize,
    reads: std::cell::Cell<usize>,
}
impl Memory for OnlyDesign<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        assert!(
            address + output.len() <= self.forbidden || address >= self.forbidden + ROOT_BYTES,
            "repair must not read an untouched saved design"
        );
        self.reads.set(self.reads.get() + 1);
        self.space.memory().copy(address, output)
    }
}
unsafe extern "system-unwind" fn edited(
    pointer: usize,
    pause: usize,
    value: usize,
    _: usize,
) -> usize {
    unsafe {
        let definition =
            std::ptr::read_unaligned((pointer + native::DEFINITION) as *const usize);
        std::ptr::write_unaligned(
            (definition + engine_native_parts::cequipment_type::STATS_BASE + 46 * 8) as *mut i64,
            value as i64,
        );
        std::ptr::write_unaligned((pointer + native::AIR_RANGE_STAT) as *mut i64, value as i64);
        std::ptr::write((pointer + native::HIGHLIGHT) as *mut u8, value as u8);
    }
    if pause != 0 {
        let pause = unsafe { &*(pause as *const Barrier) };
        pause.wait();
        pause.wait();
    }
    value
}
fn spawn<const SITE: usize>(
    pointer: usize,
    pause: &Barrier,
    value: usize,
) -> std::thread::JoinHandle<usize> {
    let args = [pointer, std::ptr::from_ref::<Barrier>(pause) as usize, value, 0];
    std::thread::spawn(move || {
        collector_test_support::with_allocations_denied(|| unsafe { around::<SITE>(args, edited) })
    })
}
#[test]
fn different_saved_designs_overlap_and_publish_exact_terminal_snapshots() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<0>(registry[0].pointer(), &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<0>(registry[1].pointer(), &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(active(), 2);
    assert!(!MISSED.load(Ordering::Acquire));
    pauses[1].wait();
    assert_eq!(second.join().unwrap(), 22);
    pauses[0].wait();
    assert_eq!(first.join().unwrap(), 11);
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(2));
    assert_eq!(
        changes.iter().map(|row| row.variant).collect::<Vec<_>>(),
        [registry[0].id, registry[1].id]
    );
    assert_eq!(
        changes
            .iter()
            .map(|row| row.snapshot.as_ref().unwrap().stats[0])
            .collect::<Vec<_>>(),
        [Some(11), Some(22)]
    );
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
    assert!(!needs_full());
    assert!(held::certified().is_some());
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
fn same_saved_design_overlap<const SITE: usize>() {
    let (space, registry, baseline) = setup(2);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<SITE>(registry[0].pointer(), &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<SITE>(registry[0].pointer(), &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(active(), 2);
    pauses[0].wait();
    first.join().unwrap();
    let third = collector_test_support::with_allocations_denied(|| unsafe {
        around::<SITE>([registry[0].pointer(), 0, 33, 0], edited)
    });
    assert_eq!(third, 33);
    pauses[1].wait();
    second.join().unwrap();
    assert_eq!(active(), 0);
    let mut changes = Vec::new();
    let meter = OnlyDesign {
        space: &space,
        forbidden: registry[1].pointer(),
        reads: std::cell::Cell::new(0),
    };
    assert_eq!(drain_with(&meter, space.base, 7, &mut changes), Ok(1));
    assert!(meter.reads.get() > 0);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].snapshot.as_ref().unwrap().stats[0], Some(33));
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 2);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
    assert_eq!(MISSED_REFUSALS.load(Ordering::Relaxed), 0);
    assert_eq!(REPAIRED_RECEIVERS.load(Ordering::Relaxed), 1);
    assert!(!needs_full());
    assert_published_equals_fresh(baseline, changes.clone(), acquire(&space, &registry));
    let mut captured = Captured::vacant();
    captured.outcome = Outcome::Rows { len: changes.len() };
    captured.rows = changes;
    assert!(matches!(captured.table_input(None), TableInput::Sampled(
        TableView::Complete(rows::TableRows::VariantChanges(rows))) if rows.len() == 1));
}
#[test]
fn same_saved_design_overlap_repairs_only_that_receiver_even_when_original_stores_are_ordered() {
    if !isolated() {
        return;
    }
    same_saved_design_overlap::<0>();
}
#[test]
fn overlapping_self_owned_derived_writers_repair_only_that_design() {
    if !isolated() {
        return;
    }
    same_saved_design_overlap::<1>();
}
unsafe extern "system-unwind" fn nested(
    pointer: usize,
    other: usize,
    pause: usize,
    _: usize,
) -> usize {
    assert_eq!(TARGET.get(), pointer);
    unsafe {
        around::<6>([pointer, 0, 1, 0], edited);
        around::<0>([other, 0, 2, 0], edited);
    }
    assert_eq!(TARGET.get(), pointer);
    let pause = unsafe { &*(pause as *const Barrier) };
    pause.wait();
    pause.wait();
    77
}
#[test]
fn same_thread_nesting_keeps_outer_snapshot_and_independent_nested_receiver() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(3);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &raw const pauses[0] as usize,
        0,
    ];
    let first = std::thread::spawn(move || {
        collector_test_support::with_allocations_denied(|| unsafe { around::<0>(args, nested) })
    });
    pauses[0].wait();
    let second = spawn::<0>(registry[2].pointer(), &pauses[1], 3);
    pauses[1].wait();
    assert_eq!(active(), 2);
    pauses[1].wait();
    second.join().unwrap();
    pauses[0].wait();
    assert_eq!(first.join().unwrap(), 77);
    assert_eq!(TARGET.get(), 0);
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(3));
    assert_eq!(
        changes.iter().map(|row| row.variant).collect::<Vec<_>>(),
        [registry[0].id, registry[1].id, registry[2].id]
    );
    assert_eq!(
        changes
            .iter()
            .map(|row| row.snapshot.as_ref().unwrap().stats[0])
            .collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3)]
    );
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn contended_marks_keep_every_receiver_for_one_terminal_snapshot_each() {
    if !isolated() {
        return;
    }
    static WAIT: OnceLock<Barrier> = OnceLock::new();
    fn waiting() {
        WAIT.get().unwrap().wait();
    }
    let (space, registry, baseline) = setup(2);
    let pause = WAIT.get_or_init(|| Barrier::new(registry.len() + 1));
    let store = STORE.get().unwrap().lock().unwrap();
    std::thread::scope(|threads| {
        let writers: Vec<_> = registry
            .iter()
            .enumerate()
            .map(|(index, source)| {
                threads.spawn(move || {
                    BEFORE_STORE_WAIT.set(Some(waiting));
                    collector_test_support::with_allocations_denied(|| {
                        for value in [index * 10 + 1, index * 10 + 2] {
                            assert_eq!(
                                unsafe { around::<0>([source.pointer(), 0, value, 0], edited) },
                                value
                            );
                        }
                    });
                })
            })
            .collect();
        pause.wait();
        drop(store);
        for writer in writers {
            writer.join().unwrap();
        }
    });
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(2));
    assert_eq!(
        changes.iter().map(|row| row.variant).collect::<Vec<_>>(),
        [registry[0].id, registry[1].id]
    );
    assert_eq!(
        changes
            .iter()
            .map(|row| row.snapshot.as_ref().unwrap().stats[0])
            .collect::<Vec<_>>(),
        [Some(2), Some(12)]
    );
    assert!(
        changes
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert!(LOCK_CONTENTIONS.load(Ordering::Relaxed) >= 2);
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
    assert_eq!(REPAIRED_INTERVALS.load(Ordering::Relaxed), 1);
    assert!(!needs_full());
    assert_published_equals_fresh(baseline, changes.clone(), acquire(&space, &registry));
    let mut captured = Captured::vacant();
    captured.outcome = Outcome::Rows { len: changes.len() };
    captured.rows = changes;
    assert!(matches!(captured.table_input(None), TableInput::Sampled(
        TableView::Complete(rows::TableRows::VariantChanges(rows))) if rows.len() == 2));
}
#[test]
fn a_contended_self_owned_derived_writer_marks_only_its_receiver() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    assert_eq!(
        with_store_contended(|| unsafe { around::<1>([registry[0].pointer(), 0, 17, 0], edited) }),
        17
    );
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].variant, registry[0].id);
    assert_eq!(changes[0].snapshot.as_ref().unwrap().stats[0], Some(17));
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn a_contended_scalar_and_flag_mark_reads_the_design_once_and_refreshes_cached_flags() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    let pointer = registry[0].pointer();
    unsafe {
        std::ptr::write((pointer + native::AUTO_UPGRADED) as *mut u8, 1);
        std::ptr::write((pointer + native::CAN_UPGRADE_VARIANT) as *mut u8, 1);
    }
    with_store_contended(|| {
        mark_design(&space.memory(), space.base, pointer, false);
        mark_design(&space.memory(), space.base, pointer, false);
    });
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 1);
    let snapshot = changes[0].snapshot.as_ref().unwrap();
    let state = snapshot.definition.design.require("design").unwrap();
    assert_eq!((state.auto_upgraded, state.can_upgrade_variant), (1, 1));
    assert_eq!(
        STORE.get().unwrap().lock().unwrap().watched[&pointer].flags[1],
        1
    );
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn a_parent_content_conflict_does_not_poison_an_independent_child_identity_snapshot() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(3);
    let parent = registry[0].pointer();
    let child = registry[2].pointer();
    unsafe { std::ptr::write_unaligned((child + native::PARENT) as *mut usize, parent) };
    let baseline = acquire(&space, &registry);
    begin_read();
    prepare(&registry, &baseline);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<0>(parent, &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<0>(parent, &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(unsafe { around::<0>([child, 0, 7, 0], edited) }, 7);
    pauses[0].wait();
    first.join().unwrap();
    pauses[1].wait();
    second.join().unwrap();
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(2));
    assert_eq!(
        changes.iter().map(|row| row.variant).collect::<Vec<_>>(),
        [registry[0].id, registry[2].id]
    );
    assert_eq!(
        changes
            .iter()
            .map(|row| row.snapshot.as_ref().unwrap().stats[0])
            .collect::<Vec<_>>(),
        [Some(22), Some(7)]
    );
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn unknown_receiver_loss_still_requests_a_full_census() {
    if !isolated() {
        return;
    }
    let (space, _, _) = setup(2);
    unsafe extern "system-unwind" fn unknown(_: usize, _: usize, _: usize, _: usize) -> usize {
        19
    }
    assert_eq!(
        collector_test_support::with_allocations_denied(|| unsafe { around::<0>([0; 4], unknown) }),
        19
    );
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert!(needs_full());
    assert_eq!(UNKNOWN_RECEIVERS.load(Ordering::Relaxed), 1);
    assert_eq!(REPAIRED_RECEIVERS.load(Ordering::Relaxed), 0);
}
#[test]
fn a_repair_rechecks_the_revision_after_its_live_reads() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    struct Changed<'a>(&'a Space, std::cell::Cell<bool>);
    impl Memory for Changed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !self.1.replace(true) {
                DESIGN_GATE.mark_dirty();
            }
            self.0.memory().copy(address, output)
        }
    }
    dirty_receiver(registry[0].pointer());
    let memory = Changed(&space, std::cell::Cell::new(false));
    assert!(drain_with(&memory, space.base, 7, &mut Vec::new()).is_err());
    assert!(needs_full());
    assert!(held::certified().is_none());
}

#[test]
fn a_new_saved_binding_keeps_its_publication_and_contended_edit() {
    if !isolated() {
        return;
    }
    let space = space(1, 3);
    let mut registry = Vec::new();
    native::walk_registry(&space.memory(), space.base, space.game_state, &mut registry);
    BASE.store(space.base, Ordering::Release);
    ENABLED.store(true, Ordering::Release);
    collector_observers::world_input_changes::set_design_watch_enabled_for_test(true);
    let baseline = acquire(&space, &registry[..2]);
    begin_full();
    begin_read();
    prepare(&registry[..2], &baseline);
    assert!(baseline_admitted());
    let pointer = registry[2].pointer();
    mark_design(&space.memory(), space.base, pointer, true);
    with_store_contended(|| unsafe { around::<0>([pointer, 0, 17, 0], edited) });
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].variant, registry[2].id);
    assert_eq!(changes[0].snapshot.as_ref().unwrap().stats[0], Some(17));
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn a_dirty_receiver_retired_before_capture_is_never_reread() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    dirty_receiver(registry[0].pointer());
    assert!(retire(&space.memory(), space.base, registry[0].pointer()));
    let meter = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&meter, space.base, 7, &mut changes), Ok(1));
    assert_eq!(meter.reads.get(), 0);
    assert_eq!(changes.len(), 1);
    assert!(changes[0].removed);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry[1..]));
}
#[test]
fn consuming_a_saved_parent_forces_its_carried_child_to_rederive_identity() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(3);
    unsafe {
        std::ptr::write_unaligned(
            (registry[2].pointer() + native::PARENT) as *mut usize,
            registry[0].pointer(),
        )
    };
    let baseline = acquire(&space, &registry);
    begin_read();
    prepare(&registry, &baseline);
    let pause = Barrier::new(2);
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &raw const pause as usize,
        0,
    ];
    let moving = std::thread::spawn(move || unsafe { around::<9>(args, moved) });
    pause.wait();
    pause.wait();
    moving.join().unwrap();
    assert_eq!(
        drain(space.base, 7, &mut Vec::new()),
        Err("variant parent identity has a consumed saved binding")
    );
    assert!(needs_full());
    assert!(held::census().is_none());
}

#[test]
fn unwind_releases_receiver_claims_and_restores_same_thread_nesting() {
    if !isolated() {
        return;
    }
    unsafe extern "system-unwind" fn unwind(_: usize, _: usize, _: usize, _: usize) -> usize {
        panic!("synthetic design original unwinds");
    }
    let (_space, registry, _) = setup(1);
    let pointer = registry[0].pointer();
    assert!(
        std::panic::catch_unwind(|| unsafe { around::<0>([pointer, 0, 0, 0], unwind) }).is_err()
    );
    assert_eq!(active(), 0);
    assert_eq!(TARGET.get(), 0);
    assert!(!CREATING.get());
    assert_eq!(unsafe { around::<0>([pointer, 0, 42, 0], edited) }, 42);
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert!(MISSED.load(Ordering::Acquire));
}

#[test]
fn different_derived_refreshes_can_overlap_when_their_version_owners_are_independent() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<1>(registry[0].pointer(), &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<1>(registry[1].pointer(), &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(active(), 2);
    pauses[1].wait();
    second.join().unwrap();
    pauses[0].wait();
    first.join().unwrap();
    let mut changes = Vec::new();
    drain(space.base, 7, &mut changes).unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn derived_refreshes_of_different_designs_still_refuse_a_shared_version_owner() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(3);
    let owner = registry[2].pointer();
    for row in &registry[..2] {
        unsafe { std::ptr::write_unaligned((row.pointer() + native::PARENT) as *mut usize, owner) };
    }
    let baseline = acquire(&space, &registry);
    begin_read();
    prepare(&registry, &baseline);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<1>(registry[0].pointer(), &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<1>(registry[1].pointer(), &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(active(), 2);
    pauses[1].wait();
    second.join().unwrap();
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
    pauses[0].wait();
    first.join().unwrap();
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
    assert!(repair::pending());
    let repairs = repair::take(&mut Batch::empty()).unwrap();
    assert!(
        repairs
            .watches
            .iter()
            .any(|watch| watch.pointer == owner && watch.dirty)
    );
    assert!(STORE.get().unwrap().lock().unwrap().watched[&owner].dirty);
}
unsafe extern "system-unwind" fn wait_only(_: usize, pause: usize, _: usize, _: usize) -> usize {
    let pause = unsafe { &*(pause as *const Barrier) };
    pause.wait();
    pause.wait();
    19
}
#[test]
fn unknown_create_scope_conflicts_with_a_foreign_saved_writer() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(1);
    let pause = Barrier::new(2);
    let args = [0, &raw const pause as usize, 0, 0];
    let creating = std::thread::spawn(move || unsafe { around::<22>(args, wait_only) });
    pause.wait();
    assert_eq!(
        unsafe { around::<0>([registry[0].pointer(), 0, 42, 0], edited) },
        42
    );
    pause.wait();
    assert_eq!(creating.join().unwrap(), 19);
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(UNKNOWN_RECEIVERS.load(Ordering::Relaxed), 1);
}
#[test]
fn foreign_destruction_conflicts_with_an_active_ancestry_traversal() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    let pause = Barrier::new(2);
    let refresh = spawn::<1>(registry[0].pointer(), &pause, 11);
    pause.wait();
    assert_eq!(
        unsafe { around::<11>([registry[1].pointer(), 0, 42, 0], edited) },
        42
    );
    pause.wait();
    refresh.join().unwrap();
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
}
unsafe extern "system-unwind" fn moved(
    destination: usize,
    source: usize,
    pause: usize,
    _: usize,
) -> usize {
    if pause != 0 {
        let pause = unsafe { &*(pause as *const Barrier) };
        pause.wait();
        pause.wait();
    }
    unsafe {
        std::ptr::copy_nonoverlapping(source as *const u8, destination as *mut u8, ROOT_BYTES)
    };
    19
}
#[test]
fn a_contended_move_of_two_unwatched_drafts_preserves_saved_rows_without_native_repair() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(3);
    let baseline = acquire(&space, &registry[2..]);
    begin_read();
    prepare(&registry[2..], &baseline);
    assert_eq!(
        with_store_contended(|| unsafe {
            around::<9>([registry[0].pointer(), registry[1].pointer(), 0, 0], moved)
        }),
        19
    );
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[2].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(0));
    assert_eq!(memory.reads.get(), 0);
    assert!(changes.is_empty());
    assert!(!needs_full());
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry[2..]));
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
}
#[test]
fn a_contended_move_with_a_saved_operand_retains_the_consumed_bindings() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    with_store_contended(|| unsafe {
        around::<9>([registry[0].pointer(), registry[1].pointer(), 0, 0], moved)
    });
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[1].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 3);
    assert!(changes[0].removed && changes[1].removed);
    assert_eq!(
        [changes[0].variant, changes[1].variant],
        [registry[0].id, registry[1].id]
    );
    assert!(changes[2].snapshot.is_some() && changes[2].variant == registry[1].id);
    assert!(!needs_full());
    let rebound =
        native::RegistryRow::read(&space.memory(), space.base, registry[0].pointer()).unwrap();
    let mut survivors = vec![rebound];
    survivors.extend_from_slice(&registry[2..]);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &survivors));
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
}
#[test]
fn a_contended_move_of_a_new_unprepared_saved_operand_retains_every_binding_occurrence() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(3);
    let baseline = acquire(&space, &registry[2..]);
    begin_read();
    prepare(&registry[2..], &baseline);
    mark_design(&space.memory(), space.base, registry[1].pointer(), true);
    with_store_contended(|| unsafe {
        around::<9>([registry[0].pointer(), registry[1].pointer(), 0, 0], moved)
    });
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[1].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 2);
    assert!(changes[0].removed && changes[0].variant == registry[1].id);
    assert!(changes[1].snapshot.is_some() && changes[1].variant == registry[1].id);
    assert!(!needs_full());
    let rebound =
        native::RegistryRow::read(&space.memory(), space.base, registry[0].pointer()).unwrap();
    let mut survivors = vec![rebound];
    survivors.extend_from_slice(&registry[2..]);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &survivors));
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
}
#[test]
fn a_consuming_move_and_an_unrelated_saved_edit_keep_binding_events_and_exact_rows() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(3);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &raw const pauses[0] as usize,
        0,
    ];
    let moving = std::thread::spawn(move || {
        collector_test_support::with_allocations_denied(|| unsafe { around::<9>(args, moved) })
    });
    pauses[0].wait();
    let other = spawn::<0>(registry[2].pointer(), &pauses[1], 33);
    pauses[1].wait();
    assert_eq!(active(), 2);
    pauses[1].wait();
    other.join().unwrap();
    pauses[0].wait();
    assert_eq!(moving.join().unwrap(), 19);
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(2));
    assert_eq!(changes.len(), 4);
    assert!(changes[0].removed && changes[1].removed);
    assert_eq!(
        [changes[0].variant, changes[1].variant],
        [registry[0].id, registry[1].id]
    );
    assert_eq!(
        [changes[2].variant, changes[3].variant],
        [registry[1].id, registry[2].id]
    );
    assert_eq!(changes[3].snapshot.as_ref().unwrap().stats[0], Some(33));
    {
        let store = STORE.get().unwrap().lock().unwrap();
        assert_eq!(store.watched[&registry[0].pointer()].id, registry[1].id);
        assert!(!store.watched.contains_key(&registry[1].pointer()));
    }
    let fresh_registry = [
        native::RegistryRow::read(&space.memory(), space.base, registry[0].pointer()).unwrap(),
        registry[2],
    ];
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &fresh_registry));
}
#[test]
fn a_move_source_claim_survives_saved_binding_retirement() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    let pause = Barrier::new(2);
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &raw const pause as usize,
        0,
    ];
    let moving = std::thread::spawn(move || unsafe { around::<9>(args, moved) });
    pause.wait();
    assert_eq!(
        unsafe { around::<0>([registry[1].pointer(), 0, 33, 0], edited) },
        33
    );
    assert!(!MISSED.load(Ordering::Acquire));
    pause.wait();
    moving.join().unwrap();
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
}
#[test]
fn a_saved_destruction_conflict_is_not_mistaken_for_two_unwatched_drafts() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(1);
    let pointer = registry[0].pointer();
    let pause = Barrier::new(2);
    let args = [pointer, &raw const pause as usize, 0, 0];
    let destroying = std::thread::spawn(move || unsafe { around::<11>(args, wait_only) });
    pause.wait();
    assert!(
        !STORE
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .watched
            .contains_key(&pointer)
    );
    assert_eq!(unsafe { around::<0>([pointer, 0, 42, 0], edited) }, 42);
    assert!(!MISSED.load(Ordering::Acquire));
    pause.wait();
    destroying.join().unwrap();
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
}
fn parent_snapshot_refuses_a_consuming_move(operand: usize) {
    let (space, registry, _) = setup(3);
    let child = registry[2].pointer();
    unsafe {
        std::ptr::write_unaligned(
            (child + native::PARENT) as *mut usize,
            registry[operand].pointer(),
        )
    };
    let baseline = acquire(&space, &registry);
    begin_read();
    prepare(&registry, &baseline);
    let pause = Barrier::new(2);
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &raw const pause as usize,
        0,
    ];
    let moving = std::thread::spawn(move || unsafe { around::<9>(args, moved) });
    pause.wait();
    collector_test_support::with_allocations_denied(|| {
        mark_design(&space.memory(), space.base, child, false);
    });
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
    pause.wait();
    moving.join().unwrap();
    assert_eq!(
        drain(space.base, 7, &mut Vec::new()),
        Err("variant parent identity has a consumed saved binding")
    );
}

#[test]
fn a_full_snapshot_refuses_a_parent_binding_while_a_foreign_move_consumes_it() {
    if !isolated() {
        return;
    }
    parent_snapshot_refuses_a_consuming_move(0);
}
#[test]
fn a_full_snapshot_refuses_a_parent_binding_in_the_foreign_moves_source_operand() {
    if !isolated() {
        return;
    }
    parent_snapshot_refuses_a_consuming_move(1);
}
#[test]
fn different_saved_designs_with_a_parent_link_still_admit_concurrent_ordinary_edits() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    unsafe {
        std::ptr::write_unaligned(
            (registry[0].pointer() + native::PARENT) as *mut usize,
            registry[1].pointer(),
        );
    }
    let baseline = acquire(&space, &registry);
    begin_read();
    prepare(&registry, &baseline);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<0>(registry[0].pointer(), &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<0>(registry[1].pointer(), &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(active(), 2);
    pauses[0].wait();
    first.join().unwrap();
    pauses[1].wait();
    second.join().unwrap();
    let mut changes = Vec::new();
    drain(space.base, 7, &mut changes).unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 0);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}

static FORWARDED_MOVE: AtomicU64 = AtomicU64::new(0);
static FORWARDED_SOURCE: AtomicUsize = AtomicUsize::new(0);
static FORWARD_AVAILABLE: AtomicBool = AtomicBool::new(false);
fn forward_available(availability: Availability) {
    FORWARD_AVAILABLE.store(availability.is_current(), Ordering::SeqCst);
}
unsafe fn forwarded_move(writer: Writer, pointer: usize, phase: Phase) {
    if writer != Writer::MoveAssign {
        return;
    }
    let source = pointer == FORWARDED_SOURCE.load(Ordering::Relaxed);
    let event = match (source, phase) {
        (false, Phase::Before) => 1,
        (true, Phase::Before) => 2,
        (true, Phase::After) => 3,
        (false, Phase::After) => 4,
        (_, Phase::Unwound) => 5,
    };
    FORWARDED_MOVE
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |old| {
            Some(old * 10 + event)
        })
        .unwrap();
}
static MOVE_FORWARDER: Forwarder = Forwarder {
    call: forwarded_move,
    enabled: forward_available,
};
#[test]
fn a_rejected_move_still_forwards_both_original_receivers_through_terminal_phases() {
    if !isolated() {
        return;
    }
    unsafe extern "system-unwind" fn move_now(
        destination: usize,
        source: usize,
        _: usize,
        _: usize,
    ) -> usize {
        unsafe {
            std::ptr::copy_nonoverlapping(source as *const u8, destination as *mut u8, ROOT_BYTES)
        };
        19
    }
    let (_space, registry, _) = setup(2);
    let source = registry[1].pointer();
    FORWARDED_SOURCE.store(source, Ordering::Relaxed);
    forwarder(Some(&MOVE_FORWARDER));
    assert!(FORWARD_AVAILABLE.load(Ordering::SeqCst));
    let pause = Barrier::new(2);
    let writer = spawn::<0>(source, &pause, 33);
    pause.wait();
    assert_eq!(
        collector_test_support::with_allocations_denied(|| unsafe {
            around::<9>([registry[0].pointer(), source, 0, 0], move_now)
        }),
        19
    );
    pause.wait();
    writer.join().unwrap();
    forwarder(None);
    assert_eq!(active(), 0);
    assert_eq!(FORWARDED_MOVE.load(Ordering::SeqCst), 1234);
    assert_eq!(COMPETING_WRITER_REJECTIONS.load(Ordering::Relaxed), 1);
}

#[test]
fn a_contended_saved_destruction_logs_its_tombstone_before_the_original() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    let pointer = registry[0].pointer();
    with_store_contended(|| assert!(retire(&space.memory(), space.base, pointer)));
    let memory = OnlyDesign {
        space: &space,
        forbidden: pointer,
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(0));
    assert_eq!(memory.reads.get(), 0);
    assert_eq!(changes.len(), 1);
    assert!(changes[0].removed);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry[1..]));
}
#[test]
fn contended_draft_destruction_needs_neither_a_native_read_nor_a_full_census() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(1);
    with_store_contended(|| {
        assert!(!retire(&space.memory(), space.base, usize::MAX));
    });
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(0));
    assert_eq!(memory.reads.get(), 0);
    assert!(!needs_full());
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn publication_after_a_contended_retirement_retains_both_occurrences() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(1);
    with_store_contended(|| retire(&space.memory(), space.base, registry[0].pointer()));
    mark_design(&space.memory(), space.base, registry[0].pointer(), true);
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 2);
    assert!(changes[0].removed && changes[1].snapshot.is_some());
    assert!(!needs_full());
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn a_new_saved_binding_logs_retirement_after_its_first_boundary() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    let baseline = acquire(&space, &registry[..1]);
    begin_read();
    prepare(&registry[..1], &baseline);
    mark_design(&space.memory(), space.base, registry[1].pointer(), true);
    assert_eq!(drain(space.base, 7, &mut Vec::new()), Ok(1));
    with_store_contended(|| retire(&space.memory(), space.base, registry[1].pointer()));
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[1].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 8, &mut changes), Ok(0));
    assert_eq!(changes.len(), 1);
    assert!(changes[0].removed);
    assert_eq!(changes[0].variant, registry[1].id);
}
fn saved_source() -> (Space, Vec<native::RegistryRow>, Variants, usize) {
    let (space, registry, _) = setup(3);
    let source = space.bytes.as_ptr() as usize + space.bytes.len() - 0x100;
    unsafe {
        std::ptr::write_bytes(source as *mut u8, 0, 0x100);
        std::ptr::write_unaligned(
            source as *mut u64,
            (space.base + native::SPECIFIC_BONUS_VTABLE_RVA.0) as u64,
        );
        std::ptr::write_unaligned((source + native::BONUS_STATS) as *mut usize, source + 0x80);
        std::ptr::write_unaligned(
            (source + native::BONUS_STATS + 8) as *mut u64,
            1 | (1u64 << 32),
        );
        std::ptr::write_unaligned((source + 0x88) as *mut i64, 11);
        std::ptr::write_unaligned((source + 0xc0) as *mut usize, source);
        for row in &registry[..2] {
            std::ptr::write_unaligned(
                (row.pointer() + native::BONUS_SOURCES) as *mut usize,
                source + 0xc0,
            );
            std::ptr::write_unaligned(
                (row.pointer() + native::BONUS_SOURCES + 8) as *mut u64,
                1 | (1u64 << 32),
            );
        }
    }
    let baseline = acquire(&space, &registry);
    begin_read();
    prepare(&registry, &baseline);
    assert!(baseline_admitted());
    (space, registry, baseline, source)
}
#[test]
fn a_contended_source_keeps_every_dependant_occurrence_without_terminal_rereads() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline, source) = saved_source();
    unsafe { std::ptr::write_unaligned((source + 0x88) as *mut i64, 23) };
    with_store_contended(|| {
        source_hooks::observe(
            &space.memory(),
            space.base,
            sources::Key::Direct(source),
            false,
        )
    });
    struct SourceOnly<'a> {
        space: &'a Space,
        registry: &'a [native::RegistryRow],
        source: usize,
        reads: std::cell::Cell<usize>,
    }
    impl Memory for SourceOnly<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert!(
                self.registry
                    .iter()
                    .all(|row| address + output.len() <= row.pointer()
                        || address >= row.pointer() + ROOT_BYTES),
                "the boundary must not reread a dependant design"
            );
            if (self.source..self.source + 0x100).contains(&address) {
                self.reads.set(self.reads.get() + 1);
            }
            self.space.memory().copy(address, output)
        }
    }
    let memory = SourceOnly {
        space: &space,
        registry: &registry,
        source,
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(0));
    assert!(memory.reads.get() > 0);
    assert_eq!(changes.len(), 2);
    assert_eq!(REPAIRED_RECEIVERS.load(Ordering::Relaxed), 0);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn a_contended_demanded_source_retirement_keeps_its_observation_without_reading_dead_storage() {
    if !isolated() {
        return;
    }
    let (space, _, _, source) = saved_source();
    with_store_contended(|| {
        source_hooks::observe(
            &space.memory(),
            space.base,
            sources::Key::Direct(source),
            true,
        )
    });
    struct Dead;
    impl Memory for Dead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("retired source must not be dereferenced");
        }
    }
    let mut changes = Vec::new();
    assert_eq!(
        drain_with(&Dead, space.base, 7, &mut changes),
        Err("applied design source destroyed while still referenced")
    );
    assert!(needs_full());
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
}
#[test]
fn a_contended_author_retirement_keeps_liveness_without_rewriting_frozen_designs() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    let pointer = usize::MAX;
    STORE
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .interests
        .organisation(1, Some(pointer))
        .unwrap();
    liveness::change(1, true).unwrap();
    with_store_contended(|| liveness::retired(&space.memory(), space.base, pointer));
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(0));
    assert_eq!(changes.len(), 1);
    let author = changes[0].organisation.as_ref().unwrap();
    assert_eq!(author.organisation, 1);
    assert!(!author.live);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
    assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
    let held = held::certified().unwrap();
    for (source, row) in registry.iter().zip(&baseline.definitions.rows) {
        assert!(std::sync::Arc::ptr_eq(
            &held.get(source).unwrap(),
            &row.design
        ));
    }
    drop(held);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}

#[test]
fn contended_author_roster_keeps_its_occurrence_and_frozen_designs() {
    if !isolated() {
        return;
    }
    use engine_production::nindustrial_organisation_corganisation as organisation;
    let (space, registry, baseline) = setup(1);
    let mut bytes = [0u8; organisation::REFERENCE + 8];
    bytes[..8].copy_from_slice(&((space.base + organisation::VTABLE_RVA.0) as u64).to_le_bytes());
    bytes[organisation::REFERENCE..organisation::REFERENCE + 8]
        .copy_from_slice(&1u64.to_le_bytes());
    let pointer = bytes.as_ptr() as usize;
    let pointers = [pointer];
    let header = [pointers.as_ptr() as usize as u64, 1 | (1u64 << 32)];
    liveness::change(1, true).unwrap();
    with_store_contended(|| {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        liveness::roster(&memory, space.base, header.as_ptr() as usize);
    });
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(0));
    assert_eq!(changes.len(), 1);
    assert!(!changes[0].organisation.as_ref().unwrap().live);
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}

#[test]
fn contended_draft_destruction_does_not_poison_an_unrelated_saved_publication() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    let baseline = acquire(&space, &registry[..1]);
    begin_read();
    prepare(&registry[..1], &baseline);
    with_store_contended(|| {
        retire(&space.memory(), space.base, usize::MAX);
    });
    assert!(!repair::pending());
    mark_design(&space.memory(), space.base, registry[1].pointer(), true);
    let mut changes = Vec::new();
    assert_eq!(drain(space.base, 7, &mut changes), Ok(1));
    assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}

#[test]
fn a_rejected_saved_destruction_still_refuses_unobservable_lifetime_history() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    with_store_contended(|| unrecorded_writer(Receiver::Removed, registry[0].pointer(), None));
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    assert!(drain_with(&memory, space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(memory.reads.get(), 0);
    assert!(needs_full());
    assert_eq!(LOCK_CONTENTIONS.load(Ordering::Relaxed), 1);
}
#[test]
fn a_source_without_a_receiver_still_refuses_the_whole_interval() {
    if !isolated() {
        return;
    }
    let (space, _, _) = setup(1);
    source_hooks::observe(&space.memory(), space.base, sources::Key::Direct(0), false);
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert!(needs_full());
    assert_eq!(UNKNOWN_RECEIVERS.load(Ordering::Relaxed), 1);
}

#[test]
fn a_contended_new_saved_retirement_logs_without_reading_dead_storage() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    let baseline = acquire(&space, &registry[..1]);
    begin_read();
    prepare(&registry[..1], &baseline);
    mark_design(&space.memory(), space.base, registry[1].pointer(), true);
    with_store_contended(|| retire(&space.memory(), space.base, registry[1].pointer()));
    struct Dead;
    impl Memory for Dead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("retirement cannot authorize any native read");
        }
    }
    let mut changes = Vec::new();
    assert_eq!(drain_with(&Dead, space.base, 7, &mut changes), Ok(0));
    assert_eq!(changes.len(), 1);
    assert!(changes[0].removed && changes[0].variant == registry[1].id);
    assert!(!needs_full());
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry[..1]));
}
#[test]
fn a_contended_saved_parent_retirement_still_revokes_carried_child_identities() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    unsafe {
        std::ptr::write_unaligned(
            (registry[1].pointer() + native::PARENT) as *mut usize,
            registry[0].pointer(),
        );
    }
    let baseline = acquire(&space, &registry);
    begin_read();
    prepare(&registry, &baseline);
    repair::mark(registry[1].pointer());
    with_store_contended(|| retire(&space.memory(), space.base, registry[0].pointer()));
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    assert_eq!(
        drain_with(&memory, space.base, 7, &mut Vec::new()),
        Err("variant parent identity has a consumed saved binding")
    );
    assert_eq!(memory.reads.get(), 0);
    assert!(needs_full());
}

#[test]
fn a_repaired_child_never_follows_a_new_parent_with_a_consumed_receiver() {
    if !isolated() {
        return;
    }
    let (space, registry, _) = setup(2);
    unsafe {
        std::ptr::write_unaligned(
            (registry[1].pointer() + native::PARENT) as *mut usize,
            registry[0].pointer(),
        );
    }
    repair::mark(registry[1].pointer());
    with_store_contended(|| retire(&space.memory(), space.base, registry[0].pointer()));
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    assert_eq!(
        drain_with(&memory, space.base, 7, &mut Vec::new()),
        Err("variant parent identity has a consumed saved binding")
    );
    assert!(memory.reads.get() > 0);
    assert!(needs_full());
}

#[test]
fn a_native_overlap_repair_and_an_unrelated_retirement_preserve_exact_surviving_rows() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(3);
    unsafe {
        around::<0>([registry[1].pointer(), 0, 77, 0], edited);
    }
    repair::mark(registry[1].pointer());
    with_store_contended(|| retire(&space.memory(), space.base, registry[0].pointer()));
    let memory = OnlyDesign {
        space: &space,
        forbidden: registry[0].pointer(),
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(1));
    assert!(memory.reads.get() > 0);
    assert_eq!(changes.len(), 2);
    assert!(changes[0].removed);
    assert_eq!(REFUSED_INTERVALS.load(Ordering::Relaxed), 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry[1..]));
}

#[test]
fn a_rebound_move_destination_can_be_repaired_without_reading_the_consumed_source() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    let destination = registry[0].pointer();
    let source = registry[1].pointer();
    collector_test_support::with_allocations_denied(|| unsafe {
        around::<9>([destination, source, 0, 0], moved)
    });
    collector_test_support::with_allocations_denied(|| unsafe {
        around::<0>([destination, 0, 77, 0], edited)
    });
    repair::mark(destination);
    let memory = OnlyDesign {
        space: &space,
        forbidden: source,
        reads: std::cell::Cell::new(0),
    };
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(1));
    assert_eq!(changes.len(), 3);
    assert!(changes[..2].iter().all(|row| row.removed));
    let rebound = native::RegistryRow::read(&space.memory(), space.base, destination).unwrap();
    assert_eq!(rebound.id, registry[1].id);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &[rebound]));
}

#[test]
fn a_delayed_variant_job_keeps_carry_closed_until_its_terminal_repair_finishes() {
    if !isolated() {
        return;
    }
    let (space, registry, baseline) = setup(2);
    let pause = Barrier::new(2);
    let writer = spawn::<0>(registry[0].pointer(), &pause, 77);
    pause.wait();
    boundary(false);
    pause.wait();
    writer.join().unwrap();
    repair::mark(registry[0].pointer());
    assert!(repair::pending());
    assert!(held::certified().is_none());
    assert!(!needs_full());
    struct DuringRepair<'a>(OnlyDesign<'a>);
    impl Memory for DuringRepair<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert!(repair::pending());
            assert!(held::certified().is_none());
            self.0.copy(address, output)
        }
    }
    let memory = DuringRepair(OnlyDesign {
        space: &space,
        forbidden: registry[1].pointer(),
        reads: std::cell::Cell::new(0),
    });
    let mut changes = Vec::new();
    assert_eq!(drain_with(&memory, space.base, 7, &mut changes), Ok(1));
    assert!(memory.0.reads.get() > 0);
    assert!(!repair::pending());
    assert!(held::certified().is_some());
    assert!(!needs_full());
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}

#[test]
fn a_delayed_variant_capture_settles_its_refusal_before_the_census_decision() {
    if !isolated() {
        return;
    }
    let (space, _, _) = setup(1);
    boundary(false);
    unknown_receiver!();
    assert!(!needs_full());
    let mut changes = Captured::vacant();
    changes.capture(space.base, 7);
    assert!(needs_full());
    assert!(matches!(changes.outcome, Outcome::Unavailable { .. }));
    assert!(held::certified().is_none());
    for input in project(&changes, None) {
        assert!(matches!(
            input,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }
}
