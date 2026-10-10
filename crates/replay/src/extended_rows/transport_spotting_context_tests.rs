use super::*;
use crate::value::Value;
fn packet() -> TransportSpottingContext {
    TransportSpottingContext {
        sample_hour: i32::MIN,
        owners_complete: true,
        regions_complete: true,
        owners: vec![TransportSpottingOwner {
            owner_slot: u32::MAX,
            owner_raw_id: u32::MAX - 1,
            target_owned_states: Some(i32::MIN),
            radar: Some(TransportRadarCounts {
                signed_count: i32::MIN,
                counts: None,
            }),
            navy_intel: Some(TransportNavyIntel {
                owner_raw_id: Some(u32::MAX),
                owner_slot: Some(u32::MAX),
                signed_count: Some(i32::MAX),
                lanes: Some(vec![i64::MIN, i64::MAX, -1, 0]),
            }),
        }],
        reserved: Some(TransportSpottingOwner {
            radar: Some(TransportRadarCounts {
                signed_count: 0,
                counts: Some(vec![]),
            }),
            ..Default::default()
        }),
        regions: vec![TransportSpottingRegion {
            region_id: u32::MAX,
            runtime_slot: Some(i32::MIN),
            signed_province_count: Some(i32::MAX),
            sampled_province: Some(0),
            weather_detection: Some(Some(i64::MIN)),
            air_environment: Some(TransportAirEnvironment {
                weather: [Some(i64::MIN), None],
                night_factor: Some(i64::MAX),
                night: [Some(-1), Some(0)],
                target_region: Some(i32::MIN),
            }),
        }],
    }
}
fn encoded(packet: &TransportSpottingContext) -> Vec<u8> {
    let mut bytes = vec![];
    Value::encode(packet, &mut bytes).unwrap();
    assert_eq!(Value::encoded_bytes(packet).unwrap(), bytes.len());
    assert_eq!(Value::try_clone(packet).unwrap(), *packet);
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        TransportSpottingContext::decode(&mut input).unwrap(),
        *packet
    );
    input.finish().unwrap();
    bytes
}
#[test]
fn raw_widths_and_independent_native_null_refusal_empty_states_round_trip() {
    let original = packet();
    encoded(&original);
    let mut previous = vec![];
    for state in [None, Some(None), Some(Some(0)), Some(Some(i64::MAX))] {
        let mut value = original.clone();
        value.regions[0].weather_detection = state;
        let bytes = encoded(&value);
        assert_ne!(bytes, previous);
        previous = bytes;
    }
    let mut value = original;
    value.owners_complete = false;
    value.regions[0].sampled_province = None;
    value.owners[0].navy_intel.as_mut().unwrap().owner_slot = None;
    value.owners[0].navy_intel.as_mut().unwrap().lanes = None;
    encoded(&value);
    value.owners.clear();
    value.regions.clear();
    value.reserved = None;
    encoded(&value);
}
#[test]
fn every_truncated_context_and_impossible_original_owner_array_is_refused() {
    let mut bytes = encoded(&packet());
    for end in 0..bytes.len() {
        assert!(TransportSpottingContext::decode(&mut Decoder::new(&bytes[..end])).is_err());
    }
    // sample-hour4, owner completeness1, region completeness1, then owner count4.
    bytes[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(TransportSpottingContext::decode(&mut Decoder::new(&bytes)).is_err());
}
