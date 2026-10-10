use super::*;
use crate::rows::{TransportLedger, TransportNavy};

#[test]
fn contribution_refusals_are_independent_of_own_lazy_risk_gate() {
    let mut receiver = TransportMissionReceiver {
        in_regions: Some(0),
        combined_detection: Some(0),
        risk: Some(crate::rows::TransportReceiverRisk {
            stats: Some(crate::rows::TransportTaskForceStats {
                detection_138: Some(-1),
                carrier_detection_160: Some(0),
                ..Default::default()
            }),
            ship_count: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(risk_refusals(&receiver), [false; RISK_REFUSAL_NAMES.len()]);
    receiver
        .risk
        .as_mut()
        .unwrap()
        .stats
        .as_mut()
        .unwrap()
        .detection_138 = None;
    let missing = risk_refusals(&receiver);
    assert!(missing[2]);
    assert_eq!(missing.into_iter().filter(|missing| *missing).count(), 1);
    receiver.combined_detection = None;
    assert!(risk_refusals(&receiver)[21]);
    receiver.combined_detection = Some(0);
    receiver.in_regions = Some(1);
    receiver.combat_references = Some(0);
    let missing = risk_refusals(&receiver);
    assert!(missing[0] && missing[5] && missing[12] && missing[19] && missing[20]);
    receiver.risk = None;
    assert!(risk_refusals(&receiver)[1]);
}

const HANDLE: u64 = 0x1234_0000_000d;

#[test]
fn current_physical_access_arrays_never_use_carried_daily_modes_or_ledger_fallback() {
    let mut native_packet = packet();
    let navy = &mut native_packet.navies.as_mut().unwrap()[1];
    navy.access_modes = Some(vec![0, 2, 1, 255]);
    navy.ledgers = None;
    let input = Recorded::Value(Some(native_packet));
    let view = MissionView::new(&input, 23).unwrap();
    assert_eq!(view.access_modes(0), Ok(&[][..]));
    assert_eq!(view.access_modes(1), Ok(&[0, 2, 1, 255][..]));
    assert!(view.ledger(1, 0).is_err());
    let mut native_packet = packet();
    native_packet.navies.as_mut().unwrap()[1].access_modes = None;
    let input = Recorded::Value(Some(native_packet));
    let view = MissionView::new(&input, 23).unwrap();
    assert!(view.access_modes(1).is_err());
    assert!(view.ledger(1, 0).is_ok());
    let mut native_packet = packet();
    native_packet.navies.as_mut().unwrap()[0].access_modes = Some(vec![1]);
    let input = Recorded::Value(Some(native_packet));
    assert!(
        MissionView::new(&input, 23)
            .unwrap()
            .access_modes(0)
            .is_err()
    );
}

fn packet() -> TransportMissionInputs {
    TransportMissionInputs {
        naval_search: None,
        spotting_context: None,
        air: None,
        risk_ships: None,
        sample_hour: 23,
        navies: Some(vec![
            TransportNavy {
                access_modes: Some(vec![]),
                slot: 0,
                present: false,
                region_slots: Some(0),
                ledgers: Some(vec![]),
            },
            TransportNavy {
                access_modes: Some(vec![]),
                slot: 1,
                present: true,
                region_slots: Some(6),
                ledgers: Some(vec![
                    TransportLedger {
                        region: 0,
                        occurrences: Some(vec![0, HANDLE, HANDLE, 0]),
                    },
                    TransportLedger {
                        region: 4,
                        occurrences: None,
                    },
                ]),
            },
        ]),
        receivers: Some(vec![TransportMissionReceiver {
            task_force: HANDLE,
            in_regions: Some(255),
            combat_references: Some(-1),
            combined_detection: Some(-123),
            detection_cache: Some([0, 0]),
            received_supply_ratio: Some(50_000),
            ..TransportMissionReceiver::default()
        }]),
    }
}

#[test]
fn historical_current_absence_and_wrong_hour_are_distinct() {
    assert!(matches!(
        MissionView::new(&Recorded::NotRecorded, 23),
        Err("transport mission packet not recorded")
    ));
    assert!(matches!(
        MissionView::new(&Recorded::Value(None), 23),
        Err("transport mission packet refused")
    ));
    assert!(matches!(
        MissionView::new(&Recorded::Value(Some(packet())), 24),
        Err("transport mission packet has wrong sample hour")
    ));
}

#[test]
fn null_empty_repeated_refused_and_outside_domain_ledgers_remain_distinct() {
    let value = Recorded::Value(Some(packet()));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(view.ledger(0, u32::MAX), Ok(&[][..]));
    assert_eq!(view.ledger(1, 0), Ok(&[0, HANDLE, HANDLE, 0][..]));
    assert_eq!(view.ledger(1, 1), Ok(&[][..]));
    assert_eq!(view.ledger(1, 4), Err("transport region ledger refused"));
    assert_eq!(
        view.ledger(1, 6),
        Err("transport region outside original navy domain")
    );
    assert_eq!(
        view.ledger(2, 0),
        Err("transport physical navy slot outside original domain")
    );
    assert_eq!(
        view.receiver(HANDLE).unwrap().combined_detection,
        Some(-123)
    );
    assert_eq!(
        view.receiver(0),
        Err("transport null occurrence has no receiver")
    );
    assert_eq!(
        view.receiver(HANDLE + 1),
        Err("transport mission receiver missing")
    );
}

#[test]
fn receiver_and_navy_domain_refusals_do_not_erase_each_other() {
    let mut original = packet();
    original.navies = None;
    let value = Recorded::Value(Some(original));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(
        view.ledger(1, 0),
        Err("transport physical navy domain refused")
    );
    assert!(view.receiver(HANDLE).is_ok());
    let mut original = packet();
    original.receivers = None;
    let value = Recorded::Value(Some(original));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(view.ledger(1, 0), Ok(&[0, HANDLE, HANDLE, 0][..]));
    assert_eq!(
        view.receiver(HANDLE),
        Err("transport mission receiver dictionary refused")
    );
}

#[test]
fn malformed_keys_are_refused_once_without_poisoning_other_domain() {
    let mut original = packet();
    original.navies.as_mut().unwrap()[1].slot = 3;
    let value = Recorded::Value(Some(original));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(
        view.ledger(1, 0),
        Err("transport physical navy domain is incomplete")
    );
    assert!(view.receiver(HANDLE).is_ok());
    let mut original = packet();
    let duplicate = original.receivers.as_ref().unwrap()[0];
    original.receivers.as_mut().unwrap().push(duplicate);
    let value = Recorded::Value(Some(original));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(
        view.receiver(HANDLE),
        Err("transport mission receiver dictionary keys invalid")
    );
    assert!(view.ledger(1, 0).is_ok());
    let mut original = packet();
    original.navies.as_mut().unwrap()[1]
        .ledgers
        .as_mut()
        .unwrap()
        .swap(0, 1);
    let value = Recorded::Value(Some(original));
    assert_eq!(
        MissionView::new(&value, 23).unwrap().ledger(1, 0),
        Err("transport navy ledger keys invalid")
    );
}

#[test]
fn native_null_slot_zero_empty_manager_and_full_handle_high_word_remain_distinct() {
    let mut original = packet();
    original.navies.as_mut().unwrap()[0] = TransportNavy {
        access_modes: Some(vec![]),
        slot: 0,
        present: true,
        region_slots: Some(6),
        ledgers: Some(vec![]),
    };
    let value = Recorded::Value(Some(original));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(
        view.ledger(0, 0),
        Err("transport physical navy domain lacks native null slot zero")
    );
    assert!(view.receiver(HANDLE).is_ok());
    let mut original = packet();
    original.navies = Some(vec![]);
    let value = Recorded::Value(Some(original));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(view.ledger(0, 0), Ok(&[][..]));
    assert_eq!(view.ledger(u32::MAX, u32::MAX), Ok(&[][..]));
    assert!(view.receiver(HANDLE).is_ok());
    let mut original = packet();
    original.receivers.as_mut().unwrap()[0].task_force = 51;
    let value = Recorded::Value(Some(original));
    let view = MissionView::new(&value, 23).unwrap();
    assert_eq!(
        view.receiver(51),
        Err("transport mission receiver dictionary keys invalid")
    );
    assert!(view.ledger(1, 0).is_ok());
}

#[test]
fn native_gate_short_circuits_original_failures_and_keeps_signed_count() {
    let mut receiver = packet().receivers.unwrap()[0];
    assert_eq!(demanded(&receiver), Ok(true));
    receiver.in_regions = Some(0);
    receiver.combat_references = None;
    assert_eq!(demanded(&receiver), Ok(false));
    receiver.in_regions = Some(7);
    assert_eq!(
        demanded(&receiver),
        Err("transport mission combat header refused")
    );
    receiver.combat_references = Some(1);
    assert_eq!(demanded(&receiver), Ok(false));
    receiver.combat_references = Some(i32::MIN);
    assert_eq!(demanded(&receiver), Ok(true));
}

#[test]
fn numerical_detection_borrows_same_packet_raw_cache_and_supply() {
    let mut receiver = packet().receivers.unwrap()[0];
    let defines = Defines {
        base: 100_000,
        radar_bonus: 300_000,
        air_superiority_bonus: -200_000,
    };
    assert_eq!(
        detection(&receiver, defines),
        Ok(DetectionOperands {
            combined_detection: -123,
            multiplier: 50_000_000,
        })
    );
    receiver.received_supply_ratio = Some(-50_000);
    assert_eq!(
        detection(&receiver, defines).unwrap().multiplier,
        -50_000_000
    );
    receiver.received_supply_ratio = Some(200_000);
    assert_eq!(
        detection(&receiver, defines).unwrap().multiplier,
        100_000_000
    );
    receiver.detection_cache = None;
    assert_eq!(
        detection(&receiver, defines),
        Err("transport detection cache refused")
    );
    receiver.in_regions = Some(0);
    assert_eq!(
        detection(&receiver, defines),
        Err("transport detection branch not reached")
    );
}
