use super::*;
use crate::rows::{TransportAirEnvironment, TransportRadarCounts};

fn packet() -> TransportSpottingContext {
    TransportSpottingContext {
        sample_hour: 7,
        owners_complete: true,
        regions_complete: true,
        owners: vec![
            TransportSpottingOwner {
                owner_slot: 1,
                owner_raw_id: 10,
                target_owned_states: Some(1),
                radar: Some(TransportRadarCounts {
                    signed_count: 2,
                    counts: Some(vec![-11, 37]),
                }),
                navy_intel: Some(TransportNavyIntel {
                    owner_raw_id: Some(10),
                    owner_slot: Some(1),
                    signed_count: Some(3),
                    lanes: Some(vec![0, 0, i64::MIN]),
                }),
            },
            TransportSpottingOwner {
                owner_slot: 2,
                owner_raw_id: 20,
                target_owned_states: Some(1),
                radar: None,
                navy_intel: Some(TransportNavyIntel {
                    owner_raw_id: Some(20),
                    owner_slot: Some(2),
                    signed_count: Some(3),
                    lanes: Some(vec![0, 1, 0]),
                }),
            },
        ],
        reserved: Some(TransportSpottingOwner {
            owner_slot: 0,
            owner_raw_id: 0,
            target_owned_states: Some(0),
            radar: Some(TransportRadarCounts {
                signed_count: 0,
                counts: None,
            }),
            navy_intel: Some(TransportNavyIntel {
                owner_raw_id: Some(0),
                owner_slot: None,
                signed_count: Some(3),
                lanes: Some(vec![4, 5, 6]),
            }),
        }),
        regions: vec![TransportSpottingRegion {
            region_id: 3,
            runtime_slot: Some(1),
            signed_province_count: Some(-17),
            sampled_province: Some(99),
            weather_detection: Some(None),
            air_environment: Some(TransportAirEnvironment {
                weather: [Some(0), None],
                night_factor: Some(0),
                night: [None, None],
                target_region: Some(3),
            }),
        }],
    }
}
fn resolve(raw: u32) -> Fact<u32> {
    match raw {
        10 | 101 => Ok(1),
        20 | 202 => Ok(2),
        90 => Ok(0),
        _ => Err("original link unavailable"),
    }
}

#[test]
fn same_hour_aliases_raw_zero_fallback_and_wrapping_reciprocal_lanes() {
    let packet = packet();
    let view = View::new(Some(&packet), 7).unwrap();
    assert_eq!(view.radar_counts(101, 3, &mut resolve), Ok((37, -17)));
    assert_eq!(
        view.navy_intel_difference(101, 202, &mut resolve),
        Ok(i64::MAX)
    );
    assert_eq!(view.navy_intel_difference(0, 20, &mut resolve), Ok(6));
    assert_eq!(view.navy_intel_difference(90, 20, &mut resolve), Ok(6));
    assert_eq!(view.navy_intel_difference(999, 90, &mut resolve), Ok(0));
    assert!(View::new(Some(&packet), 8).is_err());
    assert!(View::new(None, 7).is_err());
}

#[test]
fn signed_nonpositive_target_and_owned_state_gates_skip_unreached_sources() {
    let mut absent = packet();
    absent.owners_complete = false;
    absent.owners.clear();
    absent.reserved = None;
    let view = View::new(Some(&absent), 7).unwrap();
    for target in [0, u32::MAX, 0x8000_0000] {
        assert_eq!(
            view.navy_intel_difference(999, target, &mut |_| panic!("unreached target link")),
            Ok(0)
        );
    }
    let mut packet = packet();
    packet.owners[1].target_owned_states = Some(-5);
    packet.owners[1].navy_intel = None;
    let view = View::new(Some(&packet), 7).unwrap();
    assert_eq!(
        view.navy_intel_difference(999, 20, &mut |raw| {
            assert_eq!(raw, 20);
            Ok(2)
        }),
        Ok(0)
    );
}

#[test]
fn intel_owner_zero_and_signed_gates_are_lazy_but_nonzero_translation_fault_is_not_healed() {
    let refused = TransportNavyIntel::default();
    let zero = TransportNavyIntel {
        owner_raw_id: Some(0),
        ..Default::default()
    };
    assert_eq!(lane(&refused, &zero), Ok(0));
    let mut negative = TransportNavyIntel {
        owner_raw_id: Some(u32::MAX),
        owner_slot: None,
        ..Default::default()
    };
    assert!(lane(&refused, &negative).is_err());
    negative.owner_slot = Some(u32::MAX);
    assert_eq!(lane(&refused, &negative), Ok(0));
    let owner = TransportNavyIntel {
        owner_raw_id: Some(10),
        owner_slot: Some(2),
        ..Default::default()
    };
    for count in [i32::MIN, -1, 0, 1, 2] {
        assert_eq!(
            lane(
                &TransportNavyIntel {
                    signed_count: Some(count),
                    ..Default::default()
                },
                &owner
            ),
            Ok(0)
        );
    }
    assert!(
        lane(
            &TransportNavyIntel {
                signed_count: Some(3),
                ..Default::default()
            },
            &owner
        )
        .is_err()
    );
}

#[test]
fn native_radar_zero_does_not_demand_runtime_slot_or_failed_body() {
    let mut packet = packet();
    packet.regions[0].runtime_slot = None;
    let view = View::new(Some(&packet), 7).unwrap();
    assert_eq!(
        view.radar_counts(0, 3, &mut |_| panic!("raw0 translation")),
        Ok((0, -17))
    );
    assert!(view.radar_counts(10, 3, &mut resolve).is_err());
    packet
        .reserved
        .as_mut()
        .unwrap()
        .radar
        .as_mut()
        .unwrap()
        .signed_count = -1;
    assert!(
        View::new(Some(&packet), 7)
            .unwrap()
            .radar_counts(0, 3, &mut resolve)
            .is_err()
    );
}

#[test]
fn weather_null_refusal_and_air_environment_faults_remain_independent() {
    let mut packet = packet();
    assert_eq!(
        View::new(Some(&packet), 7).unwrap().weather_detection(3),
        Ok(None)
    );
    packet.regions[0].weather_detection = None;
    let view = View::new(Some(&packet), 7).unwrap();
    assert!(view.weather_detection(3).is_err());
    assert_eq!(view.sampled_province(3), Ok(99));
    assert_eq!(view.air_environment(3).unwrap().weather, [Some(0), None]);
    packet.regions[0].weather_detection = Some(Some(i64::MIN));
    packet.regions[0].sampled_province = None;
    let view = View::new(Some(&packet), 7).unwrap();
    assert_eq!(view.weather_detection(3), Ok(Some(i64::MIN)));
    assert!(view.sampled_province(3).is_err());
}

#[test]
fn incomplete_positive_domain_does_not_replace_independent_genuine_reserved_object() {
    let mut packet = packet();
    packet.owners_complete = false;
    let view = View::new(Some(&packet), 7).unwrap();
    assert!(view.owner(10, &mut resolve).is_err());
    assert_eq!(view.owner(0, &mut resolve).unwrap().owner_slot, 0);
    packet.reserved = None;
    assert!(
        View::new(Some(&packet), 7)
            .unwrap()
            .owner(90, &mut resolve)
            .is_err()
    );
}
