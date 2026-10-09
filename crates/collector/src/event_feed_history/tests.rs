use super::*;
use engine::{
    ccountry, cdiplomacy_status, cgame_state, crelation, crelation_status, cwar_relation,
};
use std::collections::BTreeMap;

struct NoReads;
impl Memory for NoReads {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("owned annex settlement must not reread roles after authored dispatch")
    }
}

fn operands(hour: i32, actor: u32) -> native::Operands {
    native::Operands {
        source: native::Source::Annex,
        hour,
        actor: native::CountryIdentity {
            reference: actor + 1,
            id: actor,
            instance: 44,
        },
        target: native::CountryIdentity {
            reference: 0,
            id: 0,
            instance: 11,
        },
    }
}

fn boundary(generation: u64, hour: i32) -> OwnerBoundary {
    OwnerBoundary {
        generation,
        read_hour: hour,
        coverage: EventFeedCoverage::Complete,
        failure: None,
    }
}

fn pack(store: &mut Store, boundary: OwnerBoundary) -> Prepared {
    let mut snapshot = Snapshot::new(store.capacity());
    store.snapshot(&mut snapshot, boundary).unwrap();
    snapshot.prepare_pack().unwrap()
}

fn ready(capacity: usize) -> Store {
    let mut store = Store::new(None).unwrap();
    store.prepare_capacity(capacity).unwrap();
    let first = pack(&mut store, boundary(7, 0));
    assert_eq!(first.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(first.publication());
    store
}

fn acquire(store: &mut Store, hour: i32, actor: u32) -> Ticket {
    store
        .acquire(7, operands(hour, actor), Proof::Annex)
        .unwrap()
}

fn settle(store: &mut Store, ticket: Ticket) {
    unsafe { store.normal_returned(ticket, 7, &NoReads, 0).unwrap() };
}

#[test]
fn repeated_same_hour_pair_has_distinct_acquisitions_and_later_visibility() {
    let mut store = ready(2);
    let first = acquire(&mut store, -17, 2);
    let second = acquire(&mut store, -17, 2);
    // Reentrant completion can reverse; history remains acquisition ordered.
    settle(&mut store, second);
    settle(&mut store, first);
    let packed = pack(&mut store, boundary(7, 8));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(packed.row().read_hour, 8);
    let events = &packed.row().events;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, 0);
    assert_eq!(events[1].sequence, 1);
    assert_eq!(events[0].hour, -17);
    assert_eq!(events[0].actor.reference, 3);
    assert_eq!(events[0].actor.id, 2);
    assert_eq!(events[0].actor.instance, 44);
    assert_eq!(events[0].target.id, 0);
    store.acknowledged(packed.publication());
    store.acknowledged(packed.publication());
    assert_eq!(store.unacknowledged(), 0);
    let empty = pack(&mut store, boundary(7, 9));
    assert!(empty.row().events.is_empty());
    assert_eq!(empty.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(empty.row().counts.unwrap().settled, 2);
}

#[test]
fn dropped_callback_ticket_never_releases_immutable_pending_storage() {
    let mut store = ready(1);
    {
        let _ticket = acquire(&mut store, 1, 5);
        // Deliberately no terminal call, as on the preserved RustDrop failure.
    }
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entries[0].event.actor.id, 5);
    assert!(store.entries[0].proof.is_some());
    let packed = pack(&mut store, boundary(7, 2));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().outstanding, 1);
    store.acknowledged(packed.publication());
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entries.len(), 1);
    let mut snapshot = Snapshot::new(1);
    assert!(store.snapshot(&mut snapshot, boundary(8, 10)).is_err());
}

#[test]
fn uncertain_normal_return_cannot_recertify_or_clear_a_claim() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 2, 4);
    store.uncertain(&ticket, "native dispatcher escaped");
    let retained = unsafe { store.normal_returned(ticket, 7, &NoReads, 0) }.unwrap_err();
    assert_eq!(
        retained.reason,
        "Event feed native completion remains uncertain"
    );
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.counts.settled, 0);
    assert_eq!(store.entry(&retained.ticket).unwrap().event.actor.id, 4);
    let packed = pack(&mut store, boundary(7, 3));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().events.len(), 1);
    assert_eq!(packed.row().events[0].source, EventFeedSource::Annex);
    assert_eq!(packed.row().counts.unwrap().settled, 0);
}

#[test]
fn same_hour_replacement_invalidates_generation_without_resetting_ownership() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 5, 6);
    store.invalidate("world replacement began");
    let mut snapshot = Snapshot::new(1);
    assert!(store.snapshot(&mut snapshot, boundary(8, 10)).is_err());
    let retained = unsafe { store.normal_returned(ticket, 8, &NoReads, 0) }.unwrap_err();
    assert_eq!(
        retained.reason,
        "Event feed world invalidated before settlement"
    );
    let packed = pack(&mut store, boundary(7, 5));
    assert_eq!(packed.row().generation, 7);
    assert_eq!(packed.row().read_hour, 5);
    assert_eq!(packed.row().outstanding, 1);
    assert_eq!(packed.row().counts.unwrap().acquired, 1);
    store.acknowledged(packed.publication());
    assert_eq!(store.outstanding(), 1);
    assert!(store.entries[0].proof.is_some());
}

#[test]
fn pressure_refuses_coverage_but_keeps_known_fact_and_requires_partial_recovery() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    let capacity = store.entries.capacity();
    while store.entries.len() < capacity {
        let other = acquire(&mut store, 1, 3);
        settle(&mut store, other);
    }
    assert_eq!(
        store.acquire(7, operands(1, 3), Proof::Annex).err(),
        Some("Event feed callback storage needs growth")
    );
    assert_eq!(store.entries.capacity(), capacity);
    settle(&mut store, ticket);
    let packed = pack(&mut store, boundary(7, 2));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().events.len(), capacity);
    assert_eq!(packed.row().counts.unwrap().rejected, 1);
    store.acknowledged(packed.publication());
    let recovery = pack(&mut store, boundary(7, 3));
    assert_eq!(recovery.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(recovery.publication());
    let complete = pack(&mut store, boundary(7, 4));
    assert_eq!(complete.row().coverage, EventFeedCoverage::Complete);
    assert!(complete.row().events.is_empty());
}

#[test]
fn stale_prepared_pack_never_removes_a_later_acquisition() {
    let mut store = ready(2);
    let first = acquire(&mut store, 1, 3);
    settle(&mut store, first);
    let prepared = pack(&mut store, boundary(7, 2));
    let second = acquire(&mut store, 2, 4);
    store.acknowledged(prepared.publication());
    assert_eq!(prepared.row().events.len(), 1);
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entries[0].state, State::Published);
    assert_eq!(store.entry(&second).unwrap().event.actor.id, 4);
}

#[test]
fn missing_owner_observation_never_reads_native_identity_or_certifies_empty() {
    let mut store = ready(1);
    store.invalidate("country provider association unavailable");
    assert_eq!(
        store
            .annex_before(
                &NoReads,
                0,
                7,
                native::AnnexFrame {
                    rbp: 0,
                    r8: 0,
                    r9: 0
                }
            )
            .err(),
        Some("Event feed world observation unavailable")
    );
    let packed = pack(&mut store, boundary(7, 1));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert!(packed.row().events.is_empty());
    assert_eq!(packed.row().counts.unwrap().rejected, 1);
}

#[test]
fn sequence_seed_and_cumulative_accounting_survive_a_genuine_generation_change() {
    let mut store = Store::new(Some(40)).unwrap();
    store.prepare_capacity(1).unwrap();
    let initial = pack(&mut store, boundary(7, 0));
    store.acknowledged(initial.publication());
    let first = acquire(&mut store, 6, 4);
    settle(&mut store, first);
    let packed = pack(&mut store, boundary(7, 6));
    assert_eq!(packed.row().events[0].sequence, 41);
    store.acknowledged(packed.publication());
    store.invalidate("world writer entered");
    let refused = pack(&mut store, boundary(7, 6));
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    store.acknowledged(refused.publication());
    let changed = pack(&mut store, boundary(8, 6));
    assert_eq!(changed.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(changed.publication());
    let second = store.acquire(8, operands(6, 4), Proof::Annex).unwrap();
    unsafe { store.normal_returned(second, 8, &NoReads, 0).unwrap() };
    let packed = pack(&mut store, boundary(8, 6));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(packed.row().events[0].sequence, 42);
    assert_eq!(packed.row().counts.unwrap().acquired, 2);
}

#[test]
fn count_and_sequence_exhaustion_are_unavailable_without_wrap() {
    let mut store = ready(1);
    store.counts.acquired = u64::MAX;
    assert_eq!(
        store.acquire(7, operands(1, 3), Proof::Annex).err(),
        Some("Event feed acquired count exhausted")
    );
    let packed = pack(&mut store, boundary(7, 2));
    assert_eq!(packed.row().counts, None);
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(store.counts.acquired, u64::MAX);
    assert!(store.entries.is_empty());

    let mut store = ready(1);
    store.next_sequence = u64::MAX;
    assert_eq!(
        store.acquire(7, operands(1, 3), Proof::Annex).err(),
        Some("Event feed sequence exhausted")
    );
    assert_eq!(store.next_sequence, u64::MAX);
    assert!(store.entries.is_empty());
    assert!(Store::new(Some(u64::MAX)).is_err());
}

#[test]
fn accepted_unsettled_interval_requires_partial_after_healthy_later_return() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 2);
    let unavailable = pack(&mut store, boundary(7, 2));
    assert_eq!(unavailable.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(unavailable.row().outstanding, 1);
    assert_eq!(unavailable.row().events.len(), 1);
    store.acknowledged(unavailable.publication());
    settle(&mut store, ticket);
    let recovery = pack(&mut store, boundary(7, 3));
    assert_eq!(recovery.row().coverage, EventFeedCoverage::Partial);
    assert!(recovery.row().events.is_empty());
    store.acknowledged(recovery.publication());
    let complete = pack(&mut store, boundary(7, 4));
    assert_eq!(complete.row().coverage, EventFeedCoverage::Complete);
}

#[test]
fn escaping_annex_fact_is_owned_once_without_settling_or_losing_its_ticket() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, -17, 4);
    store.uncertain(&ticket, "native dispatcher escaped");
    let prepared = pack(&mut store, boundary(7, 9));
    assert_eq!(prepared.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(prepared.row().events[0].hour, -17);
    assert_eq!(prepared.row().counts.unwrap().acquired, 1);
    assert_eq!(prepared.row().counts.unwrap().settled, 0);
    assert_eq!(prepared.row().outstanding, 1);
    store.acknowledged(prepared.publication());
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entry(&ticket).unwrap().state, State::PublishedPending);
    assert!(store.entry(&ticket).unwrap().proof.is_some());
    let next = pack(&mut store, boundary(7, 10));
    assert!(next.row().events.is_empty());
    assert_eq!(next.row().outstanding, 1);
    let mut snapshot = Snapshot::new(1);
    assert!(store.snapshot(&mut snapshot, boundary(8, 10)).is_err());
}

#[test]
fn pending_war_is_never_a_fact_before_same_invocation_normal_postproof() {
    let image = WarImage::new();
    let mut store = ready(1);
    let ticket = store
        .war_before(&image, BASE, 7, WarImage::frame())
        .unwrap();
    let prepared = pack(&mut store, boundary(7, 9));
    assert!(prepared.row().events.is_empty());
    assert_eq!(prepared.row().outstanding, 1);
    assert_eq!(prepared.row().counts.unwrap().settled, 0);
    store.acknowledged(prepared.publication());
    assert_eq!(store.entry(&ticket).unwrap().state, State::Pending);
    assert!(store.entry(&ticket).unwrap().proof.is_some());
}

#[test]
fn rejected_empty_interval_cannot_lose_old_refusal_on_generation_observation() {
    let mut store = ready(1);
    store.reject("observed identity read failed");
    store.invalidate("world writer entered");
    assert!(store.entries.is_empty());
    let diagnostic = pack(&mut store, boundary(8, 1));
    assert_eq!(diagnostic.row().generation, 8);
    assert_eq!(diagnostic.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        diagnostic.row().failure.as_deref(),
        Some(b"observed identity read failed".as_slice())
    );
    assert_eq!(diagnostic.row().counts.unwrap().rejected, 1);
    store.acknowledged(diagnostic.publication());
    let next = pack(&mut store, boundary(8, 1));
    assert_eq!(next.row().coverage, EventFeedCoverage::Partial);
    assert_eq!(next.row().counts.unwrap().rejected, 1);
}

#[test]
fn hourly_snapshot_requires_exact_receipts_and_keeps_later_callbacks() {
    let mut store = ready(4);
    let first = acquire(&mut store, 1, 3);
    settle(&mut store, first);
    let mut first = Snapshot::new(4);
    store.snapshot(&mut first, boundary(7, 2)).unwrap();
    let first_pack = first.prepare_pack().unwrap();
    let later = acquire(&mut store, 2, 4);
    settle(&mut store, later);
    let mut later = Snapshot::new(4);
    store.snapshot(&mut later, boundary(7, 3)).unwrap();
    let later_pack = later.prepare_pack().unwrap();
    assert_eq!(first_pack.row().events[0].sequence, 0);
    assert_eq!(later_pack.row().events[0].sequence, 1);
    store.acknowledged(later_pack.publication());
    assert_eq!(store.unacknowledged(), 1);
    assert_eq!(
        store.snapshot(&mut later, boundary(8, 3)),
        Err("Event feed prior world awaits output acceptance")
    );
    store.acknowledged(first_pack.publication());
    store.snapshot(&mut later, boundary(8, 3)).unwrap();
    let next = later.prepare_pack().unwrap();
    assert_eq!(next.row().coverage, EventFeedCoverage::Partial);
    assert!(next.row().events.is_empty());
    assert_eq!(next.row().counts.unwrap().settled, 2);
}

#[test]
fn invalidated_normal_return_settles_without_reading_the_old_war_proof() {
    let image = WarImage::new();
    let mut store = ready(1);
    let ticket = store
        .war_before(&image, BASE, 7, WarImage::frame())
        .unwrap();
    store.invalidate("native reset reached");
    store.world_returned(ticket).unwrap();
    assert_eq!(store.outstanding(), 0);
    assert_eq!((store.counts.settled, store.counts.failed), (1, 1));
    assert!(store.entries[0].proof.is_none());
    assert!(store.retained.is_none());
    let mut snapshot = Snapshot::new(1);
    store.snapshot(&mut snapshot, boundary(8, 9)).unwrap();
    let packed = snapshot.prepare_pack().unwrap();
    assert!(packed.row().events.is_empty());
    assert_eq!(packed.row().generation, 8);
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().counts.unwrap().failed, 1);
}

#[test]
fn reattachment_restores_counts_without_guessing_unsettled_history() {
    let counts = EventFeedCounts {
        acquired: 20,
        settled: 20,
        failed: 2,
        rejected: 3,
    };
    let mut store = Store::new(Some(40)).unwrap();
    store.restore_counts(counts);
    store.prepare_capacity(1).unwrap();
    let first = pack(&mut store, boundary(7, 0));
    assert_eq!(first.row().counts, Some(counts));
    assert_eq!(first.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(first.publication());
    let ticket = acquire(&mut store, 1, 2);
    settle(&mut store, ticket);
    let next = pack(&mut store, boundary(7, 2));
    assert_eq!(next.row().events[0].sequence, 41);
    assert_eq!(next.row().counts.unwrap().settled, 21);

    let mut store = Store::new(None).unwrap();
    store.restore_counts(EventFeedCounts {
        acquired: 1,
        ..Default::default()
    });
    let refused = pack(&mut store, boundary(7, 0));
    assert_eq!(refused.row().counts, None);
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    assert!(
        store
            .annex_before(
                &NoReads,
                0,
                7,
                native::AnnexFrame {
                    rbp: 0,
                    r8: 0,
                    r9: 0
                }
            )
            .is_err()
    );
}

#[test]
fn old_snapshot_receipt_cannot_acknowledge_a_new_refusal() {
    let mut store = ready(1);
    let mut snapshot = Snapshot::new(1);
    store.snapshot(&mut snapshot, boundary(7, 1)).unwrap();
    let old = snapshot.prepare_pack().unwrap();
    store.reject("new callback refused");
    assert!(store.unaccepted_output());
    store.acknowledged(old.publication());
    assert!(store.unaccepted_output());
    store.snapshot(&mut snapshot, boundary(7, 2)).unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        refused.row().failure.as_deref(),
        Some(b"new callback refused".as_slice())
    );
    store.acknowledged(refused.publication());
    assert!(!store.unaccepted_output());
    store.snapshot(&mut snapshot, boundary(7, 3)).unwrap();
    assert_eq!(
        snapshot.prepare_pack().unwrap().row().coverage,
        EventFeedCoverage::Partial
    );
}

#[test]
fn final_facts_remain_unaccepted_until_a_successful_output_receipt() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    assert!(store.unaccepted_output());
    let prepared = pack(&mut store, boundary(7, 2));
    assert!(store.unaccepted_output());
    store.acknowledged(prepared.publication());
    assert!(!store.unaccepted_output());
}

#[test]
fn settlement_of_an_accepted_annex_fact_requires_its_own_output_receipt() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    let first = pack(&mut store, boundary(7, 2));
    store.acknowledged(first.publication());
    settle(&mut store, ticket);
    assert_eq!(store.outstanding(), 0);
    assert!(store.unaccepted_output());
    let settled = pack(&mut store, boundary(7, 3));
    assert!(settled.row().events.is_empty());
    assert_eq!(settled.row().counts.unwrap().settled, 1);
    store.acknowledged(settled.publication());
    assert!(!store.unaccepted_output());
}

#[test]
fn between_tick_event_detach_releases_with_a_named_tail_count() {
    let mut store = ready(1);
    let accepted = store.accepted_revision;
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    let losses = store.detach_losses().unwrap();
    assert_eq!(losses.unrecorded_tail_events, 1);
    assert_eq!(losses.unrecorded_tail_revisions, 2);
    assert_eq!(losses.failed_batches, 0);
    assert_eq!(store.accepted_revision, accepted);
    assert_eq!(store.entries[0].publication, None);
}

#[test]
fn known_failed_final_batch_detach_releases_without_acknowledging_it() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    let failed = pack(&mut store, boundary(7, 2));
    let accepted = store.accepted_revision;
    assert_eq!(
        store.detach_losses(),
        Err("Event feed output receipt remains uncertain")
    );
    store.discarded(failed.publication());
    let losses = store.detach_losses().unwrap();
    assert_eq!(losses.unrecorded_tail_events, 1);
    assert_eq!(losses.failed_batches, 1);
    assert_eq!(store.accepted_revision, accepted);
    assert_eq!(store.entries[0].state, State::Known);
    store.discarded(failed.publication());
    assert_eq!(store.detach_losses().unwrap(), losses);
}

#[test]
fn late_settlement_and_rejection_detach_count_unaccepted_revisions() {
    for settlement in [false, true] {
        let mut store = ready(1);
        if settlement {
            let ticket = acquire(&mut store, 1, 3);
            let accepted = pack(&mut store, boundary(7, 2));
            store.acknowledged(accepted.publication());
            assert!(store.detach_losses().is_err());
            settle(&mut store, ticket);
        } else {
            store.reject("final input refused");
        }
        let losses = store.detach_losses().unwrap();
        assert_eq!(losses.unrecorded_tail_events, 0);
        assert_eq!(losses.unrecorded_tail_revisions, 1);
        assert!(store.unaccepted_output());
    }
}

#[test]
fn accounting_only_batch_without_a_receipt_retains_detach() {
    let mut store = ready(1);
    let uncertain = pack(&mut store, boundary(7, 1));
    let later = pack(&mut store, boundary(7, 2));
    store.acknowledged(later.publication());
    assert_eq!(store.unacknowledged(), 0);
    assert_eq!(
        store.detach_losses(),
        Err("Event feed output receipt remains uncertain")
    );
    store.discarded(uncertain.publication());
    assert_eq!(store.detach_losses().unwrap().failed_batches, 1);
}

#[test]
fn receipt_storage_covers_the_frame_pool_and_early_recycled_spool_write() {
    let mut store = ready(0);
    let mut receipts = Vec::new();
    for hour in 1..=crate::sampling::BUFFERS + 1 {
        receipts.push(pack(&mut store, boundary(7, hour as i32)).publication());
    }
    assert!(store.detach_losses().is_err());
    for receipt in receipts {
        store.acknowledged(receipt);
    }
    assert_eq!(store.detach_losses(), Ok(DetachLosses::default()));
}

#[test]
fn discarded_snapshot_can_be_retried_or_retired_at_a_world_change() {
    for world_changed in [false, true] {
        let mut store = ready(1);
        let ticket = acquire(&mut store, 1, 3);
        settle(&mut store, ticket);
        let failed = pack(&mut store, boundary(7, 2));
        store.discarded(failed.publication());
        let next = pack(&mut store, boundary(if world_changed { 8 } else { 7 }, 3));
        assert_eq!(next.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(next.row().events.len(), usize::from(!world_changed));
        assert_eq!(
            next.row().counts.unwrap().rejected,
            u64::from(world_changed)
        );
        store.acknowledged(next.publication());
        assert_eq!(store.unacknowledged(), 0);
        let recovered = pack(&mut store, boundary(if world_changed { 8 } else { 7 }, 4));
        assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
    }
}

#[test]
fn short_hourly_snapshot_preserves_facts_for_off_thread_growth() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    let mut snapshot = Snapshot::new(0);
    assert_eq!(
        store.snapshot(&mut snapshot, boundary(7, 2)),
        Err("Event feed hourly storage needs growth")
    );
    assert_eq!(snapshot.required, 1);
    assert_eq!(store.entries[0].publication, None);
    snapshot.grow(snapshot.required).unwrap();
    store.snapshot(&mut snapshot, boundary(7, 3)).unwrap();
    assert_eq!(snapshot.prepare_pack().unwrap().row().events.len(), 1);
}

#[test]
fn hourly_snapshot_scales_at_1500_and_5000_countries_without_callback_growth() {
    for countries in [1500, 5000] {
        let mut store = ready(countries);
        let mut snapshot = Snapshot::new(countries);
        let capacity = store.capacity();
        for actor in 0..countries {
            let ticket = acquire(&mut store, 10, actor as u32);
            settle(&mut store, ticket);
        }
        store.snapshot(&mut snapshot, boundary(7, 11)).unwrap();
        assert_eq!(store.capacity(), capacity);
        let pack = snapshot.prepare_pack().unwrap();
        assert_eq!(pack.row().events.len(), countries);
        assert_eq!(
            pack.row().events[countries - 1].actor.id,
            countries as u32 - 1
        );
        assert_eq!(pack.row().counts.unwrap().settled, countries as u64);
        assert_eq!(pack.row().coverage, EventFeedCoverage::Complete);
        store.acknowledged(pack.publication());
        assert_eq!(store.unacknowledged(), 0);
    }
}

const BASE: usize = 0x140000000;
const GAME: usize = 0x1000;
const ACTOR: usize = 0x20000;
const TARGET: usize = 0x30000;
const SELECTED: usize = 0x23000;
const RECIPROCAL: usize = 0x33000;
const CANDIDATE: usize = 0x50000;
const STACK: usize = 0x70000;

#[derive(Default)]
struct WarImage(BTreeMap<usize, u8>);
impl WarImage {
    fn bytes(&mut self, address: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.0.insert(address + offset, *byte);
        }
    }
    fn q(&mut self, address: usize, value: usize) {
        self.bytes(address, &(value as u64).to_le_bytes());
    }
    fn d(&mut self, address: usize, value: u32) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.q(address, data);
        self.d(address + 8, count);
        self.d(address + 12, count);
    }
    fn country(&mut self, pointer: usize, id: u32, instance: u32, diplomacy: usize, matrix: usize) {
        self.q(pointer, BASE + ccountry::VTABLE_RVA);
        self.d(pointer + ccountry::ID, id);
        self.d(pointer + ccountry::ID_INSTANCE, instance);
        self.q(pointer + ccountry::DIPLOMACY_STATUS, diplomacy);
        self.q(diplomacy, BASE + cdiplomacy_status::VTABLE_RVA);
        self.q(diplomacy + cdiplomacy_status::COUNTRY, pointer);
        self.array(diplomacy + cdiplomacy_status::RELATIONS, matrix, 4);
    }
    fn status(&mut self, pointer: usize, diplomacy: usize, target: u32) {
        self.q(pointer, BASE + crelation_status::VTABLE_RVA);
        self.q(pointer + crelation_status::DIPLOMACY_STATUS, diplomacy);
        self.d(pointer + crelation_status::TARGET_COUNTRY, target);
        self.q(pointer + crelation_status::WAR_RELATION, 0);
    }
    fn new() -> Self {
        let mut image = Self::default();
        image.q(BASE + cgame_state::SINGLETON_RVA, GAME);
        image.d(GAME + cgame_state::CURRENT_HOUR, (-17_i32) as u32);
        image.array(GAME + cgame_state::COUNTRIES, 0x4000, 4);
        image.array(GAME + cgame_state::COUNTRY_LINKS, 0x5000, 4);
        for (slot, country) in [0x10000, ACTOR, TARGET, TARGET].into_iter().enumerate() {
            image.q(0x4000 + slot * 8, country);
        }
        for (raw, canonical) in [0, 1, 2, 2].into_iter().enumerate() {
            image.d(0x5000 + raw * 4, canonical);
        }
        image.country(0x10000, 0, 11, 0x11000, 0x12000);
        image.country(ACTOR, 1, 44, 0x21000, 0x22000);
        image.country(TARGET, 2, 55, 0x31000, 0x32000);
        image.q(0x22000 + 2 * 8, SELECTED);
        image.q(0x32000 + 8, RECIPROCAL);
        image.status(SELECTED, 0x21000, 3);
        image.status(RECIPROCAL, 0x31000, 1);
        image.q(CANDIDATE, BASE + cwar_relation::VTABLE_RVA);
        image.d(CANDIDATE + crelation::TYPE_ID, cwar_relation::TYPE);
        image.d(CANDIDATE + crelation::FIRST_COUNTRY, 1);
        image.d(CANDIDATE + crelation::SECOND_COUNTRY, 2);
        image.bytes(CANDIDATE + crelation::CANCEL, &[0, 0]);
        image.q(STACK + 0x1d8, BASE + 0x00d0_baff);
        image.q(STACK + 0x448, BASE + 0x00d0_b57d);
        image.q(STACK + 0x438, SELECTED);
        image.q(STACK + 0x4a8, BASE + 0x00d2_080e);
        image.bytes(BASE + 0x00d0_baf9, &[0xff, 0x90, 0xa8, 0, 0, 0]);
        image.bytes(BASE + 0x00d0_b578, &[0xe8, 0x03, 0x05, 0, 0]);
        image.bytes(BASE + 0x00d2_0809, &[0xe8, 0xc2, 0xab, 0xfe, 0xff]);
        image
    }
    fn attach(&mut self) {
        self.q(SELECTED + crelation_status::WAR_RELATION, CANDIDATE);
        self.q(RECIPROCAL + crelation_status::WAR_RELATION, CANDIDATE);
    }
    fn frame() -> native::WarFrame {
        native::WarFrame {
            rsp_before_call: STACK,
            rcx: SELECTED,
            rdx: CANDIDATE,
            rdi: CANDIDATE,
        }
    }
}
impl Memory for WarImage {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        for (offset, destination) in output.iter_mut().enumerate() {
            let Some(byte) = address
                .checked_add(offset)
                .and_then(|address| self.0.get(&address))
            else {
                return false;
            };
            *destination = *byte;
        }
        true
    }
}

#[test]
fn guarded_war_acquisition_owns_opaque_proof_and_needs_fresh_reciprocal_return() {
    let mut image = WarImage::new();
    let mut store = ready(1);
    let ticket = store
        .war_before(&image, BASE, 7, WarImage::frame())
        .unwrap();
    assert!(matches!(
        store.entry(&ticket).unwrap().proof.as_ref(),
        Some(Proof::War(_))
    ));
    assert_eq!(store.outstanding(), 1);
    image.attach();
    image.d(GAME + cgame_state::CURRENT_HOUR, 9);
    unsafe { store.normal_returned(ticket, 7, &image, BASE).unwrap() };
    assert_eq!(store.outstanding(), 0);
    assert!(store.entries[0].proof.is_none());
    let packed = pack(&mut store, boundary(7, 10));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
    let event = &packed.row().events[0];
    assert_eq!(event.source, EventFeedSource::Declaration);
    assert_eq!(event.hour, -17);
    assert_eq!(event.target.reference, 3);
    assert_eq!(event.target.id, 2);
    assert_eq!(event.target.instance, 55);
    assert_eq!(packed.row().counts.unwrap().settled, 1);
}

#[test]
fn guarded_war_failed_postproof_consumes_once_without_recording_a_fact() {
    for invalidate_identity in [false, true] {
        let mut image = WarImage::new();
        let mut store = ready(1);
        let ticket = store
            .war_before(&image, BASE, 7, WarImage::frame())
            .unwrap();
        if invalidate_identity {
            image.attach();
            image.d(TARGET + ccountry::ID_INSTANCE, 56);
        }
        unsafe { store.normal_returned(ticket, 7, &image, BASE).unwrap() };
        assert_eq!(store.outstanding(), 0);
        assert_eq!(store.entries[0].state, State::Failed);
        assert!(store.entries[0].proof.is_none());
        assert_eq!(store.counts.settled, 1);
        assert_eq!(store.counts.failed, 1);
        let packed = pack(&mut store, boundary(7, 2));
        assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
        assert!(packed.row().events.is_empty());
        let expected = if invalidate_identity {
            "event feed country identity changed during native operation"
        } else {
            "event feed candidate reciprocal attachment not proved"
        };
        assert_eq!(packed.row().failure.as_deref(), Some(expected.as_bytes()));
        store.acknowledged(packed.publication());
        let recovery = pack(&mut store, boundary(7, 3));
        assert_eq!(recovery.row().coverage, EventFeedCoverage::Partial);
        assert_eq!(recovery.row().counts.unwrap().settled, 1);
    }
}
