use super::*;

fn capacity() -> Capacities {
    Capacities {
        navies: 3,
        ledgers: 2,
        occurrences: 4,
        receivers: 2,
        access_modes: 0,
        ..Default::default()
    }
}

#[test]
fn original_access_bytes_are_current_independent_and_keep_full_growth_after_refusal() {
    use engine::{cpdx_array::Array, memory::SliceMemory};
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[0, 2, 1, 255],
    };
    let mut shared = Shared::new(Capacities {
        access_modes: 4,
        ..capacity()
    });
    shared.begin(7, Ok(3));
    shared.navy(0, false, Ok(0));
    shared.access_modes(&memory, Ok(Array::default()));
    shared.navy(1, true, Err("original ledger header refused"));
    shared.access_modes(
        &memory,
        Ok(Array {
            data: 0x1000,
            count: 4,
        }),
    );
    shared.navy(2, true, Ok(3));
    shared.access_modes(&memory, Err("original access header refused"));
    shared.finish();
    let navies = shared.projection().unwrap().unwrap().navies.unwrap();
    assert_eq!(navies[0].access_modes, Some(vec![]));
    assert_eq!(navies[1].access_modes, Some(vec![0, 2, 1, 255]));
    assert!(navies[1].ledgers.is_none());
    assert!(navies[2].access_modes.is_none());
    assert_eq!(navies[2].ledgers, Some(vec![]));
    shared.begin(8, Ok(2));
    shared.navy(0, false, Ok(0));
    shared.access_modes(&memory, Ok(Array::default()));
    shared.navy(1, true, Ok(0));
    shared.access_modes(
        &memory,
        Ok(Array {
            data: 0x1000,
            count: 5000,
        }),
    );
    assert_eq!(shared.required().access_modes, 5000);
    shared.retain_access_demand(5000);
    assert_eq!(
        shared.required().access_modes,
        5000,
        "held floor must not double copied demand"
    );
    shared.retain_access_demand(7000);
    assert_eq!(shared.required().access_modes, 7000);
    shared.fail("terminal");
    let mut capacity = capacity();
    shared.grow(&mut capacity);
    assert!(capacity.access_modes >= 7000);
}

#[test]
fn original_physical_nulls_nullable_repeats_and_receiver_dictionary_survive() {
    let mut shared = Shared::new(capacity());
    let handle = (71_u64 << 32) | 4;
    shared.begin(-19, Ok(3));
    shared.navy(0, false, Ok(0));
    shared.navy(1, true, Ok(9));
    shared.ledger(7, 3, Ok(&[0, handle, handle]));
    shared.navy(2, false, Ok(0));
    shared.receiver(TransportMissionReceiver {
        task_force: handle,
        in_regions: Some(0),
        ..Default::default()
    });
    assert!(shared.projection().unwrap().is_none());
    shared.finish();
    let packet = shared.projection().unwrap().unwrap();
    assert_eq!(packet.sample_hour, -19);
    let navies = packet.navies.unwrap();
    assert_eq!(
        navies
            .iter()
            .map(|navy| (navy.slot, navy.present))
            .collect::<Vec<_>>(),
        [(0, false), (1, true), (2, false)]
    );
    assert_eq!(
        navies[1].ledgers.as_ref().unwrap()[0].occurrences,
        Some(vec![0, handle, handle])
    );
    assert_eq!(packet.receivers.unwrap()[0].task_force, handle);
}

#[test]
fn partial_ledger_never_becomes_native_zero_and_domains_refuse_independently() {
    let mut shared = Shared::new(capacity());
    let handle = (71_u64 << 32) | 4;
    shared.begin(20, Ok(3));
    shared.navy(0, false, Ok(0));
    shared.navy(1, true, Ok(9));
    shared.ledger(3, 2, Err("original nonnull identity refused"));
    shared.navy(2, true, Err("original outer header refused"));
    shared.receiver(TransportMissionReceiver {
        task_force: handle,
        ..Default::default()
    });
    shared.finish();
    let packet = shared.projection().unwrap().unwrap();
    let navies = packet.navies.unwrap();
    assert_eq!(navies[1].ledgers.as_ref().unwrap()[0].occurrences, None);
    assert_eq!(navies[2].region_slots, None);
    assert_eq!(navies[2].ledgers, None);
    assert_eq!(packet.receivers.unwrap().len(), 1);
}

#[test]
fn admission_failure_preserves_growth_and_reset_retires_old_packet() {
    let mut shared = Shared::new(Capacities {
        navies: 1,
        ledgers: 1,
        occurrences: 1,
        receivers: 1,
        access_modes: 0,
        ..Default::default()
    });
    shared.begin(7, Ok(5000));
    shared.navy(0, false, Ok(0));
    shared.navy(1, true, Ok(6));
    shared.ledger(2, 5000, Err("original capacity refused"));
    shared.fail("ordinary terminal failure");
    let pressure = shared.required();
    assert_eq!(pressure.navies, 5000);
    assert_eq!(pressure.occurrences, 5000);
    let mut capacity = Capacities::default();
    shared.grow(&mut capacity);
    assert!(capacity.navies >= 5000 && capacity.occurrences >= 5000 && capacity.receivers >= 5000);
    assert!(shared.projection().unwrap().unwrap().navies.is_none());
    shared.reset();
    assert!(shared.projection().unwrap().is_none());
    shared.begin(8, Ok(0));
    shared.finish();
    assert_eq!(
        shared.projection().unwrap().unwrap().navies,
        Some(Vec::new())
    );
}

#[test]
fn nullable_ledger_aggregate_retains_each_original_refusal_reason() {
    let mut shared = Shared::new(capacity());
    shared.begin(1, Ok(1));
    shared.navy(0, true, Ok(4));
    shared.ledger(1, 1, Err("transport original mission identity refused"));
    shared.ledger(
        2,
        1,
        Err("transport mission receiver is not original class1"),
    );
    let value = serde_json::to_value(status()).unwrap();
    for reason in [
        "transport original mission identity refused",
        "transport mission receiver is not original class1",
    ] {
        assert!(
            value["nullable_ledger_or_receiver_certificate_reasons"][reason]
                .as_u64()
                .unwrap()
                > 0
        );
    }
}

#[test]
fn worker_shared_original_counts_grow_every_arena_before_first_publication() {
    let mut shared = Shared::new(Capacities::default());
    for (hour, slots, occurrences) in [(7, 5001, 17), (8, 5001, 33)] {
        let access = vec![255; occurrences];
        let memory = engine::memory::SliceMemory {
            base: 0x1000,
            bytes: &access,
        };
        shared.begin_growing(hour, Ok(slots));
        shared.navy(0, false, Ok(0));
        for slot in 1..slots {
            let handle = ((slot as u64) << 32) | 4;
            shared.navy(slot as u32, true, Ok(3));
            shared.access_modes(
                &memory,
                Ok(engine::cpdx_array::Array {
                    data: 0x1000,
                    count: occurrences as u32,
                }),
            );
            let values = vec![handle; occurrences];
            shared.ledger(1, occurrences, Ok(&values));
            shared.receiver(TransportMissionReceiver {
                task_force: handle,
                in_regions: Some(0),
                ..Default::default()
            });
        }
        shared.finish();
        let packet = shared.projection().unwrap().unwrap();
        let navies = packet.navies.unwrap();
        assert_eq!(navies.len(), slots);
        assert_eq!(packet.receivers.unwrap().len(), slots - 1);
        for navy in &navies[1..] {
            assert_eq!(navy.access_modes.as_ref().unwrap(), &access);
            assert_eq!(
                navy.ledgers.as_ref().unwrap()[0]
                    .occurrences
                    .as_ref()
                    .unwrap()
                    .len(),
                occurrences
            );
        }
        assert_eq!(shared.navy_reason, None);
        assert_eq!(shared.receiver_reason, None);
    }
}
