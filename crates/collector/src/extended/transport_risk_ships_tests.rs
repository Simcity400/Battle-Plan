use super::*;
use std::cell::RefCell;

struct Image {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Image {
    fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, bytes.len()));
        let Some(source) = at
            .checked_add(bytes.len())
            .and_then(|end| self.bytes.get(at..end))
        else {
            return false;
        };
        bytes.copy_from_slice(source);
        true
    }
}
fn image() -> Image {
    Image {
        bytes: vec![0; 0x2000],
        reads: RefCell::new(Vec::new()),
    }
}
fn put(image: &mut Image, at: usize, value: u64) {
    image.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn handle(id: u64) -> u64 {
    (id << 32) | u64::from(cship::HANDLE_TYPE)
}
fn observation(
    flags: Result<u64, &'static str>,
    positive: Result<u64, &'static str>,
) -> cship::TransportModelObservation {
    cship::TransportModelObservation {
        identity: Some(Ok(())),
        formation_flags: Some(flags),
        positive_equipment_flags: Some(positive),
        carrier_air_base: Some(Ok(None)),
    }
}

#[test]
fn live_demand_prepares_the_first_model_and_receiver_ship_graph() {
    let memory = image();
    let mut captured = Captured::new(0, 0);
    captured.prepare(5000, 5000);
    captured.begin_models(7);
    let ship = handle(9);
    for _ in 0..5000 {
        captured.model(ship, observation(Ok(0), Ok(0))).unwrap();
    }
    {
        let mut observer = captured.observer(&memory, 0, 7).unwrap();
        for receiver in 1..=5000 {
            observer.begin(receiver, Ok(1));
            observer.ship(receiver, 0, 0x1000, Ok(ship), Ok(1)).unwrap();
            observer.finish(receiver, Ok(()));
        }
    }
    assert_eq!(
        captured
            .packet(7)
            .unwrap()
            .unwrap()
            .receivers
            .unwrap()
            .len(),
        5000
    );
}

#[test]
fn unchanged_five_thousand_owner_passes_keep_demand_and_recycled_capacity_stable() {
    let mut memory = image();
    put(&mut memory, 0x1000 + cship::FORMATION_FLAGS, 1 << 22);
    let mut captured = Captured::new(0, 0);
    captured.prepare(5000, 15000);
    let bytes = captured.allocated_bytes();
    for hour in 1..=5 {
        captured.prepare(5000, 15000);
        captured.begin_models(hour);
        for ordinal in 1..=5000 {
            captured
                .model(handle(ordinal), observation(Ok(0), Ok(0)))
                .unwrap();
        }
        captured.begin(hour);
        {
            let mut observer = captured.observer(&memory, 0, hour).unwrap();
            for receiver in 1..=5000 {
                observer.begin(receiver, Ok(2));
                observer
                    .ship(receiver, 0, 0x1000, Ok(handle(receiver)), Ok(1))
                    .unwrap();
                observer
                    .ship(receiver, 1, 0x1000, Ok(handle(receiver + 5000)), Ok(1))
                    .unwrap();
                observer.finish(receiver, Ok(()));
            }
        }
        assert_eq!(
            (
                captured.receivers.required,
                captured.ships.required,
                captured.models.required
            ),
            (5000, 15000, 15000)
        );
        assert_eq!(captured.receivers.len, 5000);
        assert_eq!(captured.ships.len, 10000);
        assert_eq!(captured.models.len, 10000);
        assert_eq!(captured.graph_refusals, 0);
        assert!(
            captured.receivers.first[..captured.receivers.len]
                .iter()
                .all(|row| row.complete)
        );
        captured.reset();
        captured.grow();
        assert_eq!(captured.allocated_bytes(), bytes);
    }
}

#[test]
fn rejected_five_thousand_headers_retain_full_pass_pressure_without_accumulating_hours() {
    let memory = image();
    let mut captured = Captured::new(0, 5000);
    captured.begin_models(1);
    for ordinal in 1..=5000 {
        captured
            .model(handle(ordinal), observation(Ok(0), Ok(0)))
            .unwrap();
    }
    captured.begin(1);
    {
        let mut observer = captured.observer(&memory, 0, 1).unwrap();
        crate::allocation_failure_tests::with_allocations_denied(|| {
            for receiver in 1..=5000 {
                observer.begin(receiver, Ok(3));
                observer.finish(receiver, Err("original receiver admission failed"));
            }
        });
    }
    let required = (
        captured.receivers.required,
        captured.ships.required,
        captured.models.required,
    );
    assert_eq!(required, (5000, 15000, 20000));
    assert_eq!(captured.receivers.len, 0);
    assert_eq!(captured.ships.len, 0);
    assert!(captured.packet(1).unwrap().unwrap().receivers.is_none());
    captured.fail();
    captured.reset();
    captured.grow();
    let bytes = captured.allocated_bytes();
    for hour in 2..=5 {
        captured.begin_models(hour);
        for ordinal in 1..=5000 {
            captured
                .model(handle(ordinal), observation(Ok(0), Ok(0)))
                .unwrap();
        }
        captured.begin(hour);
        {
            let mut observer = captured.observer(&memory, 0, hour).unwrap();
            for receiver in 1..=5000 {
                observer.begin(receiver, Ok(3));
                for ordinal in 0..3 {
                    observer
                        .ship(receiver, ordinal, 0x1000, Ok(handle(receiver)), Ok(1))
                        .unwrap();
                }
                observer.finish(receiver, Ok(()));
            }
        }
        assert_eq!(
            (
                captured.receivers.required,
                captured.ships.required,
                captured.models.required
            ),
            required
        );
        assert_eq!(captured.graph_refusals, 0);
        assert_eq!(captured.ships.len, 15000);
        assert!(
            captured.receivers.first[..captured.receivers.len]
                .iter()
                .all(|row| row.complete)
        );
        captured.reset();
        captured.grow();
        assert_eq!(captured.allocated_bytes(), bytes);
    }
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn original_model_failure_is_borrowed_once_and_repeated_ship_ordinals_survive() {
    let memory = image();
    let mut captured = Captured::new(2, 4);
    captured.begin_models(9);
    let ship = handle(7);
    captured
        .model(ship, observation(Ok(0), Err("original pool failed")))
        .unwrap();
    // A later successful original occurrence cannot heal the first attempt.
    captured.model(ship, observation(Ok(0), Ok(0))).unwrap();
    {
        let mut observer = captured.observer(&memory, 0, 9).unwrap();
        observer.begin(11, Ok(2));
        observer
            .ship(11, 0, 0x1000, Ok(ship), Err("class unreached"))
            .unwrap();
        observer.ship(11, 1, 0x1000, Ok(ship), Ok(3)).unwrap();
        observer.finish(11, Ok(()));
    }
    assert!(memory.reads.borrow().is_empty());
    let packet = captured.packet(9).unwrap().unwrap();
    let ships = packet.receivers.unwrap()[0].ships.clone().unwrap();
    assert_eq!(ships.len(), 2);
    assert_eq!(ships[0].ship, ships[1].ship);
    assert_eq!(ships[0].positive_equipment_flags, None);
    assert_eq!(ships[1].positive_equipment_flags, None);
    assert_eq!(captured.model_refusals, 2);
}

#[test]
fn outside_receiver_positive_ships_reads_unattempted_carrier_once_without_pool() {
    let mut memory = image();
    let pointer = 0x1000;
    put(&mut memory, pointer + cship::FORMATION_FLAGS, 1 << 22);
    put(&mut memory, pointer + cship::CARRIER_AIR_BASE, 0);
    let mut captured = Captured::new(1, 2);
    let ship = handle(8);
    captured.begin_models(3);
    {
        let mut observer = captured.observer(&memory, 0, 3).unwrap();
        observer.begin(12, Ok(2));
        for ordinal in 0..2 {
            observer
                .ship(12, ordinal, pointer, Ok(ship), Ok(4))
                .unwrap();
        }
        observer.finish(12, Ok(()));
    }
    assert_eq!(
        *memory.reads.borrow(),
        vec![
            (pointer + cship::FORMATION_FLAGS, 8),
            (pointer + cship::CARRIER_AIR_BASE, 8)
        ]
    );
    let packet = captured.packet(3).unwrap().unwrap();
    assert_eq!(
        packet.receivers.unwrap()[0].ships.as_ref().unwrap()[0].carrier_air_base,
        Some(None)
    );
    assert_eq!(captured.model_refusals, 0);
}

#[test]
fn original_risk_ship_headers_grow_the_first_graph_and_terminal_failures_stay_named() {
    let mut memory = image();
    put(&mut memory, 0x1000 + cship::FORMATION_FLAGS, 1 << 22);
    let mut captured = Captured::new(1, 0);
    captured.begin_models(1);
    {
        let mut observer = captured.observer(&memory, 0, 1).unwrap();
        observer.begin(1, Ok(7));
        for ordinal in 0..7 {
            observer
                .ship(1, ordinal, 0x1000, Ok(handle(2)), Ok(1))
                .unwrap();
        }
        observer.finish(1, Ok(()));
    }
    assert_eq!(captured.ships.required, 7);
    assert!(captured.models.required >= 7);
    assert_eq!(
        captured.packet(1).unwrap().unwrap().receivers.unwrap()[0]
            .ships
            .as_ref()
            .unwrap()
            .len(),
        7
    );
    captured.receivers.required = 3;
    captured.fail();
    captured.reset();
    captured.grow();
    assert!(captured.models.first.len() >= 7 && captured.ships.first.len() >= 7);
    assert!(captured.receivers.first.len() >= 3);
    assert_eq!(captured.packet(1).unwrap(), None);
    let ship = handle(9);
    memory.reads.borrow_mut().clear();
    captured.begin_models(2);
    captured.model(ship, observation(Ok(0), Ok(0))).unwrap();
    {
        let mut observer = captured.observer(&memory, 0, 2).unwrap();
        observer.begin(13, Ok(1));
        observer.ship(13, 0, 0x1000, Ok(ship), Ok(1)).unwrap();
        observer.finish(13, Err("original SHIPS prefix failed"));
    }
    let receiver = captured
        .packet(2)
        .unwrap()
        .unwrap()
        .receivers
        .unwrap()
        .remove(0);
    assert_eq!(receiver.count, Some(1));
    assert_eq!(receiver.ships, None);
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn ownership_loan_and_disjoint_five_thousand_ship_graphs_grow_without_retrying_models() {
    let mut memory = image();
    put(&mut memory, 0x1000 + cship::FORMATION_FLAGS, 1 << 22);
    let mut original = Arena::new(5000);
    for ordinal in 0..5000 {
        original.first[ordinal] = Model {
            ship: handle(ordinal as u64 + 1),
            observation: observation(Ok(0), Err("first original pool failed")),
        };
    }
    original.len = 5000;
    let pointer = original.first.as_ptr();
    let mut captured = Captured::new(0, 0);
    captured.begin_models(7);
    captured.loan_model_part(0, &mut original);
    assert_eq!(captured.model_parts[0].first.as_ptr(), pointer);
    assert_eq!(original.len, 0);
    assert!(memory.reads.borrow().is_empty());
    {
        let mut observer = captured.observer(&memory, 0, 7).unwrap();
        for ordinal in 0..5000 {
            let receiver = ordinal as u64 + 1;
            observer.begin(receiver, Ok(2));
            observer
                .ship(receiver, 0, 0x1000, Ok(handle(receiver)), Ok(1))
                .unwrap();
            observer
                .ship(receiver, 1, 0x1000, Ok(handle(receiver + 5000)), Ok(1))
                .unwrap();
            observer.finish(receiver, Ok(()));
        }
    }
    let packet = captured.packet(7).unwrap().unwrap();
    let receivers = packet.receivers.unwrap();
    assert_eq!(receivers.len(), 5000);
    assert!(
        receivers
            .iter()
            .all(|receiver| receiver.ships.as_ref().unwrap().len() == 2)
    );
    assert_eq!(captured.models.len, 10000);
    assert_eq!(captured.graph_refusals, 0);
    assert_eq!(captured.model_refusals, 5000);
    assert_eq!(memory.reads.borrow().len(), 10000);
    assert!(memory.reads.borrow().iter().all(|(at, _)| {
        *at == 0x1000 + cship::FORMATION_FLAGS || *at == 0x1000 + cship::CARRIER_AIR_BASE
    }));
    assert!(receivers.iter().all(|receiver| {
        receiver.ships.as_ref().unwrap()[0]
            .positive_equipment_flags
            .is_none()
    }));
}

#[test]
fn previous_hour_or_reset_model_journal_never_authorizes_a_new_source_read() {
    let memory = image();
    let mut captured = Captured::new(1, 2);
    captured.begin_models(5);
    captured
        .model(handle(1), observation(Ok(0), Ok(0)))
        .unwrap();
    captured.begin(6);
    assert!(captured.observer(&memory, 0, 6).is_err());
    assert_eq!(captured.packet(6).unwrap().unwrap().receivers, None);
    captured.reset();
    captured.begin(5);
    assert!(captured.observer(&memory, 0, 5).is_err());
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn model_loan_and_index_allocation_failures_refuse_the_whole_original_graph() {
    let memory = image();
    for index_only in [false, true] {
        let mut captured = Captured::new(0, 0);
        captured.begin_models(7);
        let mut original = Arena::new(1);
        original.first[0] = Model {
            ship: handle(1),
            observation: observation(Ok(0), Ok(0)),
        };
        original.len = 1;
        if index_only {
            std::mem::swap(&mut captured.models, &mut original);
        } else {
            captured.loan_model_part(0, &mut original);
        }
        let reason = crate::allocation_failure_tests::with_allocations_denied(|| {
            captured.observer(&memory, 0, 7).err().unwrap()
        });
        assert_eq!(
            reason,
            if index_only {
                "capture buffer allocation failed"
            } else {
                "transport model loan allocation failed"
            }
        );
        assert_eq!(captured.graph_refusals, 1);
        assert_eq!(captured.packet(7).unwrap().unwrap().receivers, None);
        captured.begin(7);
        assert_eq!(
            captured.observer(&memory, 0, 7).err(),
            Some("transport original ship-model attempt journal unavailable for current hour")
        );
        assert_eq!(captured.packet(7).unwrap().unwrap().receivers, None);
        assert!(memory.reads.borrow().is_empty());
    }
}

#[test]
fn original_ship_graph_refusals_keep_header_and_walk_reasons() {
    let memory = image();
    let mut captured = Captured::new(2, 0);
    captured.begin_models(3);
    captured.begin(3);
    {
        let mut observer = captured.observer(&memory, 0, 3).unwrap();
        observer.begin(1, Err("fresh task-force ship roster invalid"));
        observer.finish(1, Ok(()));
        observer.begin(2, Ok(1));
        observer.finish(2, Err("risk original ship parent mismatch"));
    }
    let value = serde_json::to_value(status()).unwrap();
    for reason in [
        "fresh task-force ship roster invalid",
        "risk original ship parent mismatch",
    ] {
        assert!(value["graph_reasons"][reason].as_u64().unwrap() > 0);
    }
}
