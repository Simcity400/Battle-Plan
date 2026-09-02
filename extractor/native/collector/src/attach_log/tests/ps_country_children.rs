// Lane PS (2026-09-02): the eight country children are gated workers under
// their own domain ids, joined in table order by `CountryChildHandles::join`.
//
// These pin what the one-parent bundle used to give for free and the helper's
// structural tests cannot see (Codex round 1, findings 1-3): the tail's
// publication order, the join's sealing of a child that died before its guard
// or whose thread the OS refused, and the join's refusal to double-bracket a
// child whose own guard already spoke.

use super::state_fixture::{
    clean_status_fixture, compact_control_scheduling, drive_compact_frame, fixture_game_hour_raw,
};
use super::*;
use std::collections::BTreeSet;

/// A country read with nothing in it, for driving the gated child helper
/// without an engine.
fn empty_bundle_read() -> CompactBundleRead {
    CompactBundleRead {
        worker_panic_reason: None,
        specs: Vec::new(),
        rows: Vec::new(),
        read_time_us: 0,
        row_count: 0,
        status_flags: 0,
        technology_ids: Arc::new(TechnologyIdDictionary::default()),
        published_records: None,
    }
}

fn spec_of(id: u16) -> CompactCountryDomainSpec {
    *compact_country_domain_specs()
        .iter()
        .find(|spec| spec.id == id)
        .expect("a country domain spec")
}

/// Eight `None`s with one child in the slot its tag names.
fn one_child(child: CountryChildHandle<'_>) -> CountryChildHandles<'_> {
    let mut children: [Option<CountryChildHandle<'_>>; 8] = Default::default();
    let slot = COUNTRY_CHILD_BRACKET_TAGS
        .iter()
        .position(|tag| *tag == child.tag)
        .expect("a country child tag");
    children[slot] = Some(child);
    CountryChildHandles { children }
}

/// The child table and the spec table agree on order, so "table order" and
/// "the read order the one-parent bundle returned" are the same sequence.
///
/// kills: reordering `for_each_country_scoped_tracker` or
/// `compact_country_domain_specs` against each other.
#[test]
fn the_child_table_is_in_spec_order() {
    assert_eq!(
        COUNTRY_CHILD_BRACKET_TAGS.to_vec(),
        compact_country_domain_specs()
            .iter()
            .map(|spec| spec.id)
            .collect::<Vec<_>>(),
    );
}

/// A whole frame with all eight country domains scheduled publishes their
/// records in spec order, each domain's PERF record ahead of its DATA (or
/// gap) record -- the sequence the one-parent bundle sealed, which the fold
/// and every byte-identity claim rest on.
///
/// kills: swapping any two children in `CountryChildHandles.children`
/// (Codex round 1, finding 2); a join that returns reads in completion order.
#[test]
fn a_frame_publishes_the_country_children_in_spec_order() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    const GAME_STATE_AT: usize = 0x1000;
    let mut fixture = clean_status_fixture(0x2000);
    fixture.stage_scheduler_clock_at(GAME_STATE_AT);
    let state = fixture.collector_state();
    let names: Vec<&str> = compact_country_domain_specs()
        .iter()
        .map(|spec| spec.name)
        .collect();
    let control = compact_control_scheduling(&names);
    let outcome = drive_compact_frame(Some(&state), &control, fixture_game_hour_raw(), 7);

    let country: Vec<(u16, u16)> = outcome
        .records
        .iter()
        .filter(|record| (COMPACT_DOMAIN_FUEL..=COMPACT_DOMAIN_DIPLOMACY).contains(&record.domain_id))
        .map(|record| (record.domain_id, record.record_type))
        .collect();
    let mut order: Vec<u16> = Vec::new();
    for (domain_id, _) in &country {
        if order.last() != Some(domain_id) {
            order.push(*domain_id);
        }
    }
    assert_eq!(
        order,
        COUNTRY_CHILD_BRACKET_TAGS.to_vec(),
        "the country records must appear in spec order, each domain contiguous; got {country:?}",
    );
    for tag in COUNTRY_CHILD_BRACKET_TAGS {
        let first = country
            .iter()
            .find(|(domain_id, _)| domain_id == tag)
            .map(|(_, record_type)| *record_type);
        assert_eq!(
            first,
            Some(COMPACT_RECORD_TYPE_DOMAIN_PERF),
            "domain {tag}'s PERF record precedes its data record",
        );
    }
}

/// The join keeps table order whatever the roster scheduled and whatever
/// order the workers finished in.
///
/// kills: a join that pushes reads as they complete.
#[test]
fn the_join_returns_reads_in_table_order_not_completion_order() {
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let ids = thread::scope(|scope| {
        let mut children: [Option<CountryChildHandle<'_>>; 8] = Default::default();
        // Fuel is slow and diplomacy is fast, so completion order is the
        // reverse of table order.
        for (slot, tag, hold_ms) in [
            (0usize, COMPACT_DOMAIN_FUEL, 30u64),
            (3, COMPACT_DOMAIN_TECH, 10),
            (7, COMPACT_DOMAIN_DIPLOMACY, 0),
        ] {
            let spec = spec_of(tag);
            let handle = spawn_gated_country_child(
                scope,
                None,
                &lanes,
                tag,
                thread::Builder::new(),
                || {},
                move || {
                    thread::sleep(Duration::from_millis(hold_ms));
                    CompactBundleRead {
                        specs: vec![spec],
                        ..empty_bundle_read()
                    }
                },
                |_read| Vec::new(),
            )
            .expect("the OS spawned the child");
            children[slot] = Some(CountryChildHandle {
                tag,
                specs_for_panic: vec![spec],
                outcome: CountryChildOutcome::Spawned(handle),
            });
        }
        CountryChildHandles { children }
            .join(&lanes, 0)
            .iter()
            .map(|read| read.specs[0].id)
            .collect::<Vec<_>>()
    });
    assert_eq!(
        ids,
        vec![COMPACT_DOMAIN_FUEL, COMPACT_DOMAIN_TECH, COMPACT_DOMAIN_DIPLOMACY],
    );
}

/// A child that dies BEFORE its guard opens is sealed by the join as a badged
/// substitute with a synthetic `WorkerPanic` bracket, so reconciliation names
/// the panic rather than an unreadable clock.
///
/// kills: dropping `unbracketed_tags` and the
/// `record_worker_panicked_country_brackets` call from the join (Codex round
/// 1, finding 3); a substitute that loses the panic payload.
#[test]
fn a_child_that_dies_before_its_guard_is_sealed_as_a_worker_panic_by_the_join() {
    use compact_contract::domain_gap::DomainGapReason;
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let spec = spec_of(COMPACT_DOMAIN_FUEL);
    let reads = thread::scope(|scope| {
        let handle = spawn_gated_country_child(
            scope,
            None,
            &lanes,
            COMPACT_DOMAIN_FUEL,
            thread::Builder::new(),
            || panic!("died in the pre-bracket window"),
            empty_bundle_read,
            |_read| Vec::new(),
        )
        .expect("the OS spawned the child");
        one_child(CountryChildHandle {
            tag: COMPACT_DOMAIN_FUEL,
            specs_for_panic: vec![spec],
            outcome: CountryChildOutcome::Spawned(handle),
        })
        .join(&lanes, 0)
    });
    assert_eq!(reads.len(), 1);
    assert_ne!(reads[0].status_flags & COMPACT_STATUS_WORKER_PANIC, 0);
    assert_eq!(reads[0].specs.iter().map(|spec| spec.id).collect::<Vec<_>>(), vec![COMPACT_DOMAIN_FUEL]);
    assert!(
        reads[0]
            .worker_panic_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("died in the pre-bracket window")),
        "the substitute carries the payload the join caught: {:?}",
        reads[0].worker_panic_reason,
    );
    assert!(reads[0].published_records.is_none(), "the tail encodes the substitute inline");

    let brackets = lanes.take_clock_brackets();
    assert_eq!(brackets.len(), 1, "exactly one synthetic bracket: {brackets:?}");
    assert_eq!(brackets[0].tag, COMPACT_DOMAIN_FUEL);
    assert!(brackets[0].worker_panicked);
    assert!(
        brackets[0].opening.is_none() && brackets[0].closing.is_none(),
        "no clock is fabricated for a child that never read one",
    );
    let rejected = rejected_bracket_domains(&brackets, fixture_game_hour_raw(), HOUR_LABEL_MISMATCH_POLICY);
    assert_eq!(
        rejected
            .iter()
            .map(|(domain, (reason, _, _))| (*domain, *reason))
            .collect::<Vec<_>>(),
        vec![(COMPACT_DOMAIN_FUEL, DomainGapReason::WorkerPanic)],
    );
}

/// A child that dies INSIDE its guard already has a bracket from the guard's
/// own drop; the join must not add a second one over the same domain.
///
/// kills: a join that synthesizes a bracket for every panicked child rather
/// than for the uncovered ones.
#[test]
fn a_child_that_dies_inside_its_guard_keeps_the_guards_bracket_alone() {
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let spec = spec_of(COMPACT_DOMAIN_DECISIONS);
    let reads = thread::scope(|scope| {
        let handle = spawn_gated_country_child(
            scope,
            None,
            &lanes,
            COMPACT_DOMAIN_DECISIONS,
            thread::Builder::new(),
            || {},
            || -> CompactBundleRead { panic!("died inside the bracket") },
            |_read| Vec::new(),
        )
        .expect("the OS spawned the child");
        one_child(CountryChildHandle {
            tag: COMPACT_DOMAIN_DECISIONS,
            specs_for_panic: vec![spec],
            outcome: CountryChildOutcome::Spawned(handle),
        })
        .join(&lanes, 0)
    });
    assert_eq!(reads.len(), 1);
    assert_ne!(reads[0].status_flags & COMPACT_STATUS_WORKER_PANIC, 0);

    let brackets = lanes.take_clock_brackets();
    let ours: Vec<&DomainClockBracket> = brackets
        .iter()
        .filter(|bracket| bracket.tag == COMPACT_DOMAIN_DECISIONS)
        .collect();
    assert_eq!(ours.len(), 1, "the guard's own bracket, and no synthetic twin: {brackets:?}");
    assert!(ours[0].worker_panicked);
}

/// The spawn result is mapped, never unwrapped: a refusal becomes
/// `SpawnRefused` and a worker becomes `Spawned`.
///
/// kills: `from_spawn` panicking or aborting on `Err` (Codex round 2); the
/// helper's `Result` being unwrapped at the eight sites.
#[test]
fn from_spawn_maps_a_refusal_to_spawn_refused_and_a_worker_to_spawned() {
    let refused = CountryChildOutcome::from_spawn(Err(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        "Resource temporarily unavailable (os error 11)",
    )));
    assert!(
        matches!(&refused, CountryChildOutcome::SpawnRefused(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "a refused thread must be carried, not unwrapped",
    );
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    thread::scope(|scope| {
        let spawned = CountryChildOutcome::from_spawn(spawn_gated_country_child(
            scope,
            None,
            &lanes,
            COMPACT_DOMAIN_FUEL,
            thread::Builder::new(),
            || {},
            empty_bundle_read,
            |_read| Vec::new(),
        ));
        match spawned {
            CountryChildOutcome::Spawned(handle) => {
                handle.join().expect("a clean child joins cleanly");
            }
            CountryChildOutcome::SpawnRefused(error) => {
                panic!("a thread the OS created must be `Spawned`: {error}")
            }
        }
    });
}

/// A thread the OS refused is sealed the same way as a pre-guard panic: a
/// badged substitute naming the refusal, and a synthetic `WorkerPanic`
/// bracket. Before lane PS a refused child thread reached the parent's join
/// as a panic and sealed every country domain; without `from_spawn` and the
/// join's `SpawnRefused` arm it would unwind the frame.
///
/// The outcome goes through `from_spawn` so the same mapping the eight
/// production sites use is what this exercises (Codex round 2).
///
/// kills: a join that treats `SpawnRefused` as a clean read or drops its
/// synthetic bracket; `from_spawn` losing the error.
#[test]
fn a_refused_child_thread_is_sealed_as_a_worker_panic_naming_the_refusal() {
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let spec = spec_of(COMPACT_DOMAIN_POLITICS);
    let reads = one_child(CountryChildHandle {
        tag: COMPACT_DOMAIN_POLITICS,
        specs_for_panic: vec![spec],
        outcome: CountryChildOutcome::from_spawn(Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "Resource temporarily unavailable (os error 11)",
        ))),
    })
    .join(&lanes, 0);
    assert_eq!(reads.len(), 1);
    assert_ne!(reads[0].status_flags & COMPACT_STATUS_WORKER_PANIC, 0);
    assert!(
        reads[0]
            .worker_panic_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("refused by the OS") && reason.contains("os error 11")),
        "the substitute names the refusal: {:?}",
        reads[0].worker_panic_reason,
    );
    let brackets = lanes.take_clock_brackets();
    assert_eq!(brackets.len(), 1);
    assert_eq!(brackets[0].tag, COMPACT_DOMAIN_POLITICS);
    assert!(brackets[0].worker_panicked);
    let rejected: BTreeSet<u16> =
        rejected_bracket_domains(&brackets, fixture_game_hour_raw(), HOUR_LABEL_MISMATCH_POLICY)
            .keys()
            .copied()
            .collect();
    assert_eq!(rejected, BTreeSet::from([COMPACT_DOMAIN_POLITICS]));
}

/// THE REFUSAL, END TO END THROUGH THE PRODUCTION FAN-OUT, FOR EACH CHILD IN
/// TURN. One child is handed a thread the OS cannot create (a stack
/// reservation of `usize::MAX` bytes); the seven others get ordinary threads.
/// The join must seal the refused child as a badged substitute naming the
/// refusal with a synthetic `WorkerPanic` bracket, and the siblings must
/// publish untouched. Each of the eight sites is the refused one once.
///
/// kills: an `unwrap` on the helper's spawn result anywhere between the
/// builder and the join -- inside the helper (`Ok(builder.spawn_scoped(..)
/// .unwrap())`) or at ANY of the eight fan-out sites
/// (`Spawned(spawn_gated_country_child(..).unwrap())`) -- which turns the
/// refusal back into a panic on the collector thread (Codex rounds 3-4); a
/// fan-out that hands every child the same builder regardless of tag.
#[test]
fn a_thread_the_os_refuses_seals_that_child_through_the_production_fan_out() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let _rc_counters = lock_rc_counters();
    // THE CONTROL: this builder really is refused, or the assertions below
    // would be testing a thread that ran.
    assert!(
        thread::scope(|scope| {
            thread::Builder::new()
                .stack_size(usize::MAX)
                .spawn_scoped(scope, || {})
                .is_err()
        }),
        "a usize::MAX stack reservation must be refused by the OS for this test to mean anything",
    );
    const GAME_STATE_AT: usize = 0x1000;
    let mut fixture = clean_status_fixture(0x2000);
    fixture.stage_scheduler_clock_at(GAME_STATE_AT);
    let state = fixture.collector_state();
    let names: Vec<&str> = compact_country_domain_specs()
        .iter()
        .map(|spec| spec.name)
        .collect();
    let control = compact_control_scheduling(&names);
    for refused in COUNTRY_CHILD_BRACKET_TAGS {
        let refused = *refused;
        let mut trackers = CompactPerfGateTrackers::default();
        let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
        let gate = ReaderGate::new(12);
        let reads = thread::scope(|scope| {
            let country_trackers = CompactCountryGateTrackers {
                fuel: &mut trackers.fuel,
                production: &mut trackers.production,
                focus: &mut trackers.focus,
                tech: &mut trackers.tech,
                politics: &mut trackers.politics,
                politics_residuals: &mut trackers.politics_residuals,
                decisions: &mut trackers.decisions,
                diplomacy: &mut trackers.diplomacy,
            };
            spawn_compact_country_children(
                scope,
                Some(&gate),
                &lanes,
                &state,
                &control,
                compact_country_domain_specs().to_vec(),
                0,
                country_trackers,
                7,
                &mut trackers.tech_country_delta,
                |tag| {
                    if tag == refused {
                        thread::Builder::new().stack_size(usize::MAX)
                    } else {
                        thread::Builder::new()
                    }
                },
            )
            .join(&lanes, 0)
        });
        assert_eq!(
            reads.iter().map(|read| read.specs[0].id).collect::<Vec<_>>(),
            COUNTRY_CHILD_BRACKET_TAGS.to_vec(),
            "refusing {refused}: every child is joined in table order, the refused one included",
        );
        for read in &reads {
            let id = read.specs[0].id;
            if id == refused {
                assert_ne!(
                    read.status_flags & COMPACT_STATUS_WORKER_PANIC,
                    0,
                    "refusing {refused}: the refused child is badged",
                );
                assert!(
                    read.worker_panic_reason
                        .as_deref()
                        .is_some_and(|reason| reason.contains("refused by the OS")),
                    "refusing {refused}: the substitute names the refusal: {:?}",
                    read.worker_panic_reason,
                );
                assert!(
                    read.published_records.is_none(),
                    "refusing {refused}: the tail encodes the substitute inline",
                );
            } else {
                assert_eq!(
                    read.status_flags & COMPACT_STATUS_WORKER_PANIC,
                    0,
                    "refusing {refused}: sibling {id} must publish untouched",
                );
                assert!(
                    read.published_records.is_some(),
                    "refusing {refused}: sibling {id} encoded its own publication",
                );
            }
        }
        let brackets = lanes.take_clock_brackets();
        let ours: Vec<&DomainClockBracket> = brackets
            .iter()
            .filter(|bracket| bracket.tag == refused)
            .collect();
        assert_eq!(
            ours.len(),
            1,
            "refusing {refused}: one synthetic bracket for the refused child: {brackets:?}",
        );
        assert!(ours[0].worker_panicked);
        assert!(
            ours[0].opening.is_none() && ours[0].closing.is_none(),
            "refusing {refused}: no clock is fabricated for a thread that never ran",
        );
        assert_eq!(
            brackets.len(),
            8,
            "refusing {refused}: one bracket per child, seven real and one synthetic: {brackets:?}",
        );
    }
}
