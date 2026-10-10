use super::*;
use crate::value::Value;

fn packet() -> TransportMissionInputs {
    TransportMissionInputs {
        spotting_context: None,
        risk_ships: Some(TransportRiskShips {
            sample_hour: i32::MIN,
            receivers: Some(vec![TransportReceiverShips {
                task_force: u64::MAX,
                count: Some(2),
                ships: Some(vec![
                    TransportRiskShip {
                        ship: Some(u64::MAX),
                        class: Some(u32::MAX),
                        formation_flags: None,
                        positive_equipment_flags: Some(u64::MAX),
                        carrier_air_base: Some(None),
                    },
                    TransportRiskShip {
                        ship: None,
                        class: None,
                        formation_flags: Some(0),
                        positive_equipment_flags: None,
                        carrier_air_base: Some(Some(u64::MAX)),
                    },
                ]),
            }]),
        }),
        air: Some(TransportAirInputs {
            sample_hour: i32::MIN,
            countries: Some(vec![TransportAirCountry {
                country: Some(u32::MAX),
                slot: 7,
                present: true,
                factors: Some(vec![None, Some(i64::MIN), Some(0)]),
                regions: Some(vec![TransportAirRegion {
                    region: 0,
                    count: Some(3),
                    missions: Some(vec![
                        None,
                        Some(TransportAirMission {
                            wing: u64::MAX,
                            current_type: Some(u32::MAX),
                            aircraft: Some(i32::MIN),
                            disruption: Some(i64::MAX),
                            detection_13: None,
                            detection_14: Some(i64::MIN),
                            normalization: Some(TransportAirNormalization {
                                attack: Some(-1),
                                defence: Some(0),
                                speed: None,
                                ace: Some(None),
                                environment: Some(None),
                                ..Default::default()
                            }),
                        }),
                        None,
                    ]),
                }]),
            }]),
        }),
        naval_search: Some(TransportNavalSearch {
            width: Some(i32::MAX),
            regions: Some(vec![
                None,
                Some(TransportNavalRegion {
                    position: [i32::MIN, i32::MAX],
                    edges: vec![TransportNavalEdge {
                        to: u32::MAX,
                        flags: 255,
                        unrestricted: 17,
                        rules: vec![None, Some(0), Some(0)],
                    }],
                }),
            ]),
            rules: vec![vec![0, 255, 9]],
        }),
        sample_hour: i32::MIN,
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
                region_slots: Some(7),
                ledgers: Some(vec![
                    TransportLedger {
                        region: 0,
                        occurrences: Some(vec![0, u64::MAX, u64::MAX, 0]),
                    },
                    TransportLedger {
                        region: 6,
                        occurrences: None,
                    },
                ]),
            },
        ]),
        receivers: Some(vec![TransportMissionReceiver {
            task_force: u64::MAX,
            mission_owner: Some(u32::MAX - 1),
            logical_owner: Some(u32::MAX),
            in_regions: Some(u8::MAX),
            combat_references: Some(i32::MIN),
            combined_detection: Some(i64::MAX),
            detection_cache: Some([i64::MIN, -1]),
            received_supply_ratio: Some(0),
            risk: Some(TransportReceiverRisk {
                coordination: Some(i64::MIN),
                spotting_chance: None,
                night_spotting_chance: Some(i64::MAX),
                visibility: Some(-1),
                speed_base: Some(0),
                speed_modifier: None,
                terrain_class: Some(i32::MIN),
                stats: Some(TransportTaskForceStats {
                    detection_138: Some(i64::MIN),
                    visibility_148: None,
                    visibility_150: Some(i64::MAX),
                    carrier_detection_160: Some(-1),
                }),
                roster_owner: Some(u32::MAX),
                location: Some(0),
                region: None,
                location_controller: Some(1),
                detached_activity: Some(i32::MAX),
                ship_count: Some(i32::MIN),
                targeted_spotting_modifier: Some(i64::MIN),
                actual_terrain_speed: Some(i64::MAX),
            }),
        }]),
    }
}

fn encoded(value: &TransportMissionInputs) -> Vec<u8> {
    let mut bytes = Vec::new();
    Value::encode(value, &mut bytes).unwrap();
    assert_eq!(Value::encoded_bytes(value).unwrap(), bytes.len());
    assert_eq!(Value::try_clone(value).unwrap(), *value);
    let mut input = Decoder::new(&bytes);
    assert_eq!(TransportMissionInputs::decode(&mut input).unwrap(), *value);
    input.finish().unwrap();
    bytes
}

#[test]
fn raw_widths_order_nulls_repeats_and_independent_refusals_round_trip() {
    let bytes = encoded(&packet());
    assert_eq!(&bytes[..4], &i32::MIN.to_le_bytes());
    assert_eq!(bytes[4], 1);
    assert_eq!(&bytes[5..9], &2_u32.to_le_bytes());
    let mut independent = packet();
    independent.navies = None;
    encoded(&independent);
    independent = packet();
    independent.receivers = None;
    encoded(&independent);
    independent = packet();
    independent.navies.as_mut().unwrap()[1].region_slots = None;
    independent.navies.as_mut().unwrap()[1].ledgers = None;
    encoded(&independent);
    independent = packet();
    independent.receivers.as_mut().unwrap()[0].detection_cache = None;
    encoded(&independent);
}

#[test]
fn every_truncated_packet_and_invalid_collection_framing_is_refused() {
    let bytes = encoded(&packet());
    for end in 0..bytes.len() {
        assert!(TransportMissionInputs::decode(&mut Decoder::new(&bytes[..end])).is_err());
    }
    let mut invalid_option = bytes.clone();
    invalid_option[4] = 2;
    assert!(TransportMissionInputs::decode(&mut Decoder::new(&invalid_option)).is_err());
    let mut impossible_count = bytes;
    impossible_count[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(TransportMissionInputs::decode(&mut Decoder::new(&impossible_count)).is_err());
}

#[test]
fn current_refused_value_is_encoded_and_historical_absence_cannot_be_encoded() {
    let absent: Recorded<Option<TransportMissionInputs>> = Recorded::NotRecorded;
    let refused: Recorded<Option<TransportMissionInputs>> = Recorded::Value(None);
    let present: Recorded<Option<TransportMissionInputs>> = Recorded::Value(Some(packet()));
    let mut bytes = Vec::new();
    assert!(Value::encode(&absent, &mut bytes).is_err());
    assert!(bytes.is_empty());
    Value::encode(&refused, &mut bytes).unwrap();
    assert_eq!(bytes, [0]);
    bytes.clear();
    Value::encode(&present, &mut bytes).unwrap();
    assert_eq!(bytes[0], 1);
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        Recorded::<Option<TransportMissionInputs>>::decode(&mut input).unwrap(),
        present
    );
    input.finish().unwrap();
}

#[test]
fn singleton_suffix_delta_and_published_history_preserve_naming_prefix() {
    let row = ReservedCountryNamingRow {
        tag: b"SYNTHETIC".to_vec(),
        built: 7,
        transport_mission_inputs: Recorded::Value(Some(packet())),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        ReservedCountryNamingRow::decode_fields(0, &mut input).unwrap(),
        row
    );
    input.finish().unwrap();

    let refused = ReservedCountryNamingRow {
        transport_mission_inputs: Recorded::Value(None),
        ..row.clone()
    };
    let delta = refused.delta(Some(&row)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 22);
    assert_eq!(delta.apply(Some(row.clone())).unwrap(), refused);

    let version = compatibility::EFFECT_INPUTS_FORMAT;
    let format = crate::format::FormatVersion::read(version).unwrap();
    let fields = format.fields(TableId::ReservedCountryNaming);
    assert_eq!(fields, 13);
    let mut scan = Decoder::new(&bytes);
    for kind in &ReservedCountryNamingRow::SCHEMA[..fields] {
        kind.decode(&mut scan).unwrap();
    }
    let prefix = &bytes[..bytes.len() - scan.remaining()];
    let mut input = Decoder::new(prefix);
    input.format = version;
    input.recorded_fields = fields;
    let historical = ReservedCountryNamingRow::decode_fields(0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(historical.tag, row.tag);
    assert_eq!(historical.built, row.built);
    assert_eq!(historical.transport_mission_inputs, Recorded::NotRecorded);
    assert_eq!(historical.recorded_fields_bytes().unwrap(), prefix.len());
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
}

#[test]
fn physical_domains_round_trip_at_1500_and_5000_countries_without_a_viewer_cap() {
    for count in [1500_u32, 5000] {
        let mut navies = vec![TransportNavy {
            access_modes: Some(vec![]),
            slot: 0,
            present: false,
            region_slots: Some(0),
            ledgers: Some(Vec::new()),
        }];
        let mut receivers = Vec::new();
        for slot in 1..count {
            let handle = (u64::from(slot) << 32) | 61;
            navies.push(TransportNavy {
                access_modes: Some(vec![]),
                slot,
                present: true,
                region_slots: Some(2),
                ledgers: Some(vec![TransportLedger {
                    region: 1,
                    occurrences: Some(vec![0, handle, handle]),
                }]),
            });
            receivers.push(TransportMissionReceiver {
                task_force: handle,
                in_regions: Some(0),
                ..Default::default()
            });
        }
        let original = TransportMissionInputs {
            sample_hour: -7,
            navies: Some(navies),
            receivers: Some(receivers),
            naval_search: None,
            ..Default::default()
        };
        encoded(&original);
        let input = Recorded::Value(Some(original));
        let view = crate::transport_mission_view::MissionView::new(&input, -7).unwrap();
        let last = (u64::from(count - 1) << 32) | 61;
        assert_eq!(view.ledger(count - 1, 1), Ok(&[0, last, last][..]));
        assert_eq!(view.receiver(last).unwrap().task_force, last);
    }
}
