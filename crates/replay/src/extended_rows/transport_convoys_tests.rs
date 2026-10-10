use super::*;
use crate::value::Value;

fn packet() -> TransportConvoys {
    TransportConvoys {
        sample_hour: i32::MIN,
        priority: Some(i32::MIN),
        unreserved: Some(i32::MAX),
        cache: Some(TransportConvoyCache {
            valid: u8::MAX,
            raw: Some(i64::MIN),
        }),
        pool: Some(vec![
            TransportConvoyEntry {
                variant: None,
                flags: Some(u64::MAX),
                quantity: 0,
            },
            TransportConvoyEntry {
                variant: Some(u64::MAX),
                flags: None,
                quantity: -1,
            },
            TransportConvoyEntry {
                variant: Some(u64::MAX),
                flags: Some(1),
                quantity: i64::MAX,
            },
        ]),
        reservations: Some(vec![TransportConvoyReservation {
            priority: -1,
            quantity: i32::MIN,
        }]),
        first_variant: Some(Some(TransportConvoyVariant {
            variant: u64::MAX,
            surface_detection: Some(i64::MIN),
            submarine_detection: Some(i64::MAX),
            visibility: Some(-1),
            speed: Some(0),
        })),
    }
}

#[test]
fn original_convoy_packet_keeps_order_extremes_and_independent_refusals() {
    let original = packet();
    let mut bytes = Vec::new();
    Value::encode(&original, &mut bytes).unwrap();
    assert_eq!(Value::encoded_bytes(&original).unwrap(), bytes.len());
    assert_eq!(Value::try_clone(&original).unwrap(), original);
    let mut input = Decoder::new(&bytes);
    assert_eq!(TransportConvoys::decode(&mut input).unwrap(), original);
    input.finish().unwrap();
    for end in 0..bytes.len() {
        assert!(TransportConvoys::decode(&mut Decoder::new(&bytes[..end])).is_err());
    }
    let mut refused = original;
    refused.pool = None;
    refused.first_variant = Some(None);
    refused.cache = None;
    refused.reservations = None;
    bytes.clear();
    Value::encode(&refused, &mut bytes).unwrap();
    assert_eq!(
        TransportConvoys::decode(&mut Decoder::new(&bytes)).unwrap(),
        refused
    );

    let mut partial = packet();
    let first = partial.first_variant.as_mut().unwrap().as_mut().unwrap();
    first.surface_detection = None;
    first.submarine_detection = None;
    first.speed = None;
    bytes.clear();
    Value::encode(&partial, &mut bytes).unwrap();
    let decoded = TransportConvoys::decode(&mut Decoder::new(&bytes)).unwrap();
    assert_eq!(decoded, partial);
    let first = crate::transport_convoys::first(&decoded).unwrap().unwrap();
    assert_eq!(first.variant, u64::MAX);
    assert_eq!(first.visibility, Some(-1));
    assert_eq!(first.speed, None);
}

#[test]
fn original_convoy_suffix_delta_and_historical_prefix_are_distinct() {
    let row = CountryAppearanceRow {
        country: 7,
        transport_convoys: Recorded::Value(Some(packet())),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        CountryAppearanceRow::decode_fields(7, &mut input).unwrap(),
        row
    );
    input.finish().unwrap();
    let refused = CountryAppearanceRow {
        transport_convoys: Recorded::Value(None),
        ..row.clone()
    };
    let delta = refused.delta(Some(&row)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 21);
    assert_eq!(delta.apply(Some(row.clone())).unwrap(), refused);

    let version = compatibility::EFFECT_INPUTS_FORMAT;
    let fields = crate::format::FormatVersion::read(version)
        .unwrap()
        .fields(TableId::CountryAppearance);
    assert_eq!(fields, 21);
    let mut scan = Decoder::new(&bytes);
    for kind in &CountryAppearanceRow::SCHEMA[..fields] {
        kind.decode(&mut scan).unwrap();
    }
    let prefix = &bytes[..bytes.len() - scan.remaining()];
    let mut input = Decoder::new(prefix);
    input.format = version;
    input.recorded_fields = fields;
    let historical = CountryAppearanceRow::decode_fields(7, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(historical.transport_convoys, Recorded::NotRecorded);
    assert_eq!(historical.recorded_fields_bytes().unwrap(), prefix.len());
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
}
