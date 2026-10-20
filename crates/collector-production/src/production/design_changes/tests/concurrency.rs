//! Deterministic writer overlap: barriers order originals and terminal copies,
//! never wall-clock delays or scheduler assumptions.
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
    // These independent synthetic designs are their own version owners.
    for row in &registry {
        unsafe { std::ptr::write_unaligned((row.pointer() + native::ORIGIN_LINK) as *mut i32, 0) };
    }
    let variants = acquire(&space, &registry);
    BASE.store(space.base, Ordering::Release);
    ENABLED.store(true, Ordering::Release);
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
unsafe extern "system-unwind" fn edited(
    pointer: usize,
    pause: usize,
    value: usize,
    _: usize,
) -> usize {
    unsafe {
        std::ptr::write_unaligned((pointer + native::MAP_RANGE_STAT) as *mut i64, value as i64);
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
    let args = [pointer, pause as *const Barrier as usize, value, 0];
    std::thread::spawn(move || {
        collector_test_support::with_allocations_denied(|| unsafe { around::<SITE>(args, edited) })
    })
}
#[test]
fn different_saved_designs_overlap_and_publish_exact_terminal_snapshots() {
    let (space, registry, baseline) = setup(2);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<0>(registry[0].pointer(), &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<0>(registry[1].pointer(), &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(active(), 2);
    assert!(!MISSED.load(Ordering::Acquire));
    // Finish in the opposite order to the original stores. Snapshots and
    // sequence allocation share STORE, so publication follows terminal copies.
    pauses[1].wait();
    assert_eq!(second.join().unwrap(), 22);
    pauses[0].wait();
    assert_eq!(first.join().unwrap(), 11);
    let mut changes = Vec::new();
    drain(space.base, 7, &mut changes).unwrap();
    assert_eq!(
        changes.iter().map(|row| row.variant).collect::<Vec<_>>(),
        [registry[1].id, registry[0].id]
    );
    assert_eq!(
        changes
            .iter()
            .map(|row| row.snapshot.as_ref().unwrap().stats[0])
            .collect::<Vec<_>>(),
        [22, 11]
    );
    assert_eq!(status().competing_writer_rejections, 0);
    assert_eq!(status().lock_contentions, 0);
    assert!(!needs_full());
    assert!(held::certified().is_some());
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn same_saved_design_overlap_refuses_even_when_original_stores_are_ordered() {
    let (space, registry, _) = setup(1);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let first = spawn::<0>(registry[0].pointer(), &pauses[0], 11);
    pauses[0].wait();
    let second = spawn::<0>(registry[0].pointer(), &pauses[1], 22);
    pauses[1].wait();
    assert_eq!(active(), 2);
    pauses[0].wait();
    first.join().unwrap();
    // The rejected original still owns a claim after the initial owner exits.
    let third = collector_test_support::with_allocations_denied(|| unsafe {
        around::<0>([registry[0].pointer(), 0, 33, 0], edited)
    });
    assert_eq!(third, 33);
    pauses[1].wait();
    second.join().unwrap();
    assert_eq!(active(), 0);
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(status().competing_writer_rejections, 2);
    assert_eq!(status().lock_contentions, 0);
    assert_eq!(status().missed_refusals, 1);
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
    let (space, registry, baseline) = setup(3);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &pauses[0] as *const Barrier as usize,
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
    drain(space.base, 7, &mut changes).unwrap();
    assert_eq!(
        changes.iter().map(|row| row.variant).collect::<Vec<_>>(),
        [registry[1].id, registry[2].id, registry[0].id]
    );
    assert_eq!(status().competing_writer_rejections, 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn a_terminal_snapshot_does_not_wait_for_the_workers_store_lock() {
    let (space, registry, _) = setup(1);
    let pause = Barrier::new(2);
    let writer = spawn::<0>(registry[0].pointer(), &pause, 1);
    pause.wait();
    let store = STORE.get().unwrap().lock().unwrap();
    pause.wait();
    // Joining with STORE held proves callback try_lock never waits for drain.
    assert_eq!(writer.join().unwrap(), 1);
    drop(store);
    assert_eq!(active(), 0);
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(status().lock_contentions, 1);
    assert_eq!(status().competing_writer_rejections, 0);
}
#[test]
fn unwind_releases_receiver_claims_and_restores_same_thread_nesting() {
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
    assert_eq!(status().competing_writer_rejections, 0);
    assert!(MISSED.load(Ordering::Acquire));
}

#[test]
fn different_derived_refreshes_can_overlap_when_their_version_owners_are_independent() {
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
    assert_eq!(status().competing_writer_rejections, 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &registry));
}
#[test]
fn derived_refreshes_of_different_designs_still_refuse_a_shared_version_owner() {
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
    assert_eq!(status().competing_writer_rejections, 1);
    pauses[0].wait();
    first.join().unwrap();
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    // The first refresh's terminal parent-identity witness also rejects the
    // version owner's still-latched conflict, independently of writer entry.
    assert_eq!(status().competing_writer_rejections, 2);
    assert_eq!(status().lock_contentions, 0);
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
    let (space, registry, _) = setup(1);
    let pause = Barrier::new(2);
    let args = [0, &pause as *const Barrier as usize, 0, 0];
    let creating = std::thread::spawn(move || unsafe { around::<22>(args, wait_only) });
    pause.wait();
    assert_eq!(
        unsafe { around::<0>([registry[0].pointer(), 0, 42, 0], edited) },
        42
    );
    pause.wait();
    assert_eq!(creating.join().unwrap(), 19);
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(status().competing_writer_rejections, 1);
    assert_eq!(status().unknown_receivers, 1);
}
#[test]
fn foreign_destruction_conflicts_with_an_active_ancestry_traversal() {
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
    assert_eq!(status().competing_writer_rejections, 1);
    assert_eq!(status().lock_contentions, 0);
}
unsafe extern "system-unwind" fn moved(
    destination: usize,
    source: usize,
    pause: usize,
    _: usize,
) -> usize {
    let pause = unsafe { &*(pause as *const Barrier) };
    pause.wait();
    pause.wait();
    unsafe {
        std::ptr::copy_nonoverlapping(source as *const u8, destination as *mut u8, ROOT_BYTES)
    };
    19
}
#[test]
fn a_consuming_move_and_an_unrelated_saved_edit_keep_binding_events_and_exact_rows() {
    let (space, registry, baseline) = setup(3);
    let pauses = [Barrier::new(2), Barrier::new(2)];
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &pauses[0] as *const Barrier as usize,
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
    drain(space.base, 7, &mut changes).unwrap();
    assert_eq!(changes.len(), 4);
    assert!(changes[0].removed && changes[1].removed);
    assert_eq!(
        [changes[0].variant, changes[1].variant],
        [registry[0].id, registry[1].id]
    );
    assert_eq!(changes[3].variant, registry[1].id);
    {
        let store = STORE.get().unwrap().lock().unwrap();
        assert_eq!(store.watched[&registry[0].pointer()].id, registry[1].id);
        assert!(!store.watched.contains_key(&registry[1].pointer()));
    }
    let fresh_registry = [
        native::RegistryRow::read(&space.memory(), space.base, registry[0].pointer()).unwrap(),
        registry[2],
    ];
    assert_eq!(status().competing_writer_rejections, 0);
    assert_eq!(status().lock_contentions, 0);
    assert_published_equals_fresh(baseline, changes, acquire(&space, &fresh_registry));
}
#[test]
fn a_move_source_claim_survives_saved_binding_retirement() {
    let (space, registry, _) = setup(2);
    let pause = Barrier::new(2);
    let args = [
        registry[0].pointer(),
        registry[1].pointer(),
        &pause as *const Barrier as usize,
        0,
    ];
    let moving = std::thread::spawn(move || unsafe { around::<9>(args, moved) });
    pause.wait();
    // Both saved bindings are temporarily absent. The receiver claim, not
    // membership at this instant, must remember the competing source writer.
    assert_eq!(
        unsafe { around::<0>([registry[1].pointer(), 0, 33, 0], edited) },
        33
    );
    assert!(!MISSED.load(Ordering::Acquire));
    pause.wait();
    moving.join().unwrap();
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
    assert_eq!(status().competing_writer_rejections, 1);
    assert_eq!(status().lock_contentions, 0);
}
#[test]
fn a_saved_destruction_conflict_is_not_mistaken_for_two_unwatched_drafts() {
    let (space, registry, _) = setup(1);
    let pointer = registry[0].pointer();
    let pause = Barrier::new(2);
    let args = [pointer, &pause as *const Barrier as usize, 0, 0];
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
    assert_eq!(status().competing_writer_rejections, 1);
    assert_eq!(status().lock_contentions, 0);
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
        &pause as *const Barrier as usize,
        0,
    ];
    let moving = std::thread::spawn(move || unsafe { around::<9>(args, moved) });
    pause.wait();
    // Both primary receivers differ. The child's snapshot additionally reads
    // the still-consuming parent's identity, so that dependency must refuse.
    collector_test_support::with_allocations_denied(|| {
        observe(&space.memory(), space.base, child, false, false);
    });
    assert_eq!(status().competing_writer_rejections, 1);
    assert_eq!(status().lock_contentions, 0);
    pause.wait();
    moving.join().unwrap();
    assert!(drain(space.base, 7, &mut Vec::new()).is_err());
}

#[test]
fn a_full_snapshot_refuses_a_parent_binding_while_a_foreign_move_consumes_it() {
    parent_snapshot_refuses_a_consuming_move(0);
}
#[test]
fn a_full_snapshot_refuses_a_parent_binding_in_the_foreign_moves_source_operand() {
    parent_snapshot_refuses_a_consuming_move(1);
}
#[test]
fn different_saved_designs_with_a_parent_link_still_admit_concurrent_ordinary_edits() {
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
    // The parent's identity is immutable under its ordinary stats writer.
    // Complete the child while that writer is still inside its original.
    pauses[0].wait();
    first.join().unwrap();
    pauses[1].wait();
    second.join().unwrap();
    let mut changes = Vec::new();
    drain(space.base, 7, &mut changes).unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(status().competing_writer_rejections, 0);
    assert_eq!(status().lock_contentions, 0);
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
    assert_eq!(status().competing_writer_rejections, 1);
}
