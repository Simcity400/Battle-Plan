use super::*;
use engine_native_memory::memory::LiveMemory;

fn boundary(hour: i32) -> OwnerBoundary {
    OwnerBoundary {
        generation: 7,
        read_hour: hour,
        coverage: EventFeedCoverage::Complete,
        failure: None,
    }
}

fn fact(actor: u32) -> Fact {
    let country = |id| native::CountryIdentity {
        reference: id + 1,
        id,
        instance: 44,
    };
    Fact {
        source: EventFeedSource::Annex,
        actor: country(actor),
        target: country(0),
        cause: Default::default(),
    }
}

fn ready() -> Store {
    let mut store = Store::new(None).unwrap();
    store.prepare_capacity(128).unwrap();
    let mut output = Snapshot::new(0);
    store.snapshot(&mut output, boundary(0)).unwrap();
    store.acknowledged(output.publication());
    store
}

fn seal_hour(store: &mut Store, hour: i32) -> Prepared {
    store
        .prepare_boundary::<LiveMemory>(boundary(hour), None)
        .unwrap();
    store.known(7, hour, fact(hour as u32)).unwrap();
    let mut spare = store
        .receipt_room()
        .map_or_else(Vec::new, Vec::with_capacity);
    let mut output = Snapshot::new(0);
    output.reserve(store.seal_demand().unwrap()).unwrap();
    crate::test_support::with_allocations_denied(|| {
        store.install_receipts(&mut spare);
        store.seal(&mut output, boundary(hour))
    })
    .unwrap();
    drop(spare);
    output.prepare_pack().unwrap()
}

fn backlog(batched: bool) -> (Store, Vec<Prepared>) {
    let mut store = ready();
    let packs: Vec<_> = (1..=40).map(|hour| seal_hour(&mut store, hour)).collect();
    assert!(packs.iter().all(|pack| pack.row().events.len() == 1));
    assert_eq!(store.unacknowledged(), 40);
    let receipts: Vec<Receipt> = packs
        .iter()
        .enumerate()
        .map(|(index, pack)| {
            let outcome = match index {
                7 => Some(false),
                30 => None,
                _ => Some(true),
            };
            (pack.publication(), outcome)
        })
        .collect();
    if batched {
        store.receipts(&receipts);
    } else {
        for (publication, outcome) in receipts {
            match outcome {
                Some(true) => store.acknowledged(publication),
                Some(false) => store.discarded(publication),
                None => {}
            }
        }
    }
    (store, packs)
}

#[test]
fn a_write_behind_backlog_seals_every_hour_without_a_receipt_cap() {
    let _globals = crate::event_feed_observer::test_globals();
    let (mut store, packs) = backlog(true);
    assert_eq!(store.unacknowledged(), 1);
    assert_eq!(
        store.detach_losses(),
        Err("Event feed output receipt remains uncertain")
    );
    let next = seal_hour(&mut store, 41);
    assert_eq!(next.row().coverage, EventFeedCoverage::Unavailable);
    let hours: Vec<_> = next.row().events.iter().map(|event| event.hour).collect();
    assert_eq!(hours, [8, 41]);
    store.acknowledged(packs[30].publication());
    store.acknowledged(next.publication());
    let losses = store.detach_losses().unwrap();
    assert_eq!(losses.failed_batches, 1);
    assert_eq!(losses.unrecorded_tail_events, 0);
}

#[test]
fn batched_receipts_match_receipts_applied_one_at_a_time() {
    let _globals = crate::event_feed_observer::test_globals();
    let (mut batched, _) = backlog(true);
    let (mut single, _) = backlog(false);
    assert_eq!(batched.unacknowledged(), single.unacknowledged());
    assert_eq!(batched.publications, single.publications);
    assert_eq!(batched.accepted_revision, single.accepted_revision);
    assert_eq!(batched.failure_for_test(), single.failure_for_test());
    let [batched, single] = [&mut batched, &mut single].map(|store| seal_hour(store, 41));
    assert_eq!(batched.row(), single.row());
}

#[test]
fn repeated_unknown_and_in_flight_receipts_change_nothing() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready();
    let pack = seal_hour(&mut store, 1);
    let publication = pack.publication();
    store.receipts(&[(publication, None)]);
    assert_eq!(store.unacknowledged(), 1);
    assert_eq!(store.publications, [publication]);
    store.acknowledged(publication);
    assert_eq!(store.unacknowledged(), 0);
    store.discarded(publication);
    store.receipts(&[(publication + 1000, Some(false)), (u64::MAX, Some(true))]);
    let losses = store.detach_losses().unwrap();
    assert_eq!(losses.failed_batches, 0);
    assert_eq!(losses.unrecorded_tail_events, 0);
    assert_eq!(store.failure_for_test(), None);
}

#[test]
fn a_seal_whose_receipt_room_cannot_grow_refuses_by_name() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready();
    while store.receipt_room().is_none() {
        seal_hour(&mut store, 1);
    }
    store
        .prepare_boundary::<LiveMemory>(boundary(2), None)
        .unwrap();
    let mut output = Snapshot::new(0);
    assert_eq!(
        crate::test_support::with_allocations_denied(|| store.seal(&mut output, boundary(2))),
        Err(RECEIPT_STORAGE)
    );
}
