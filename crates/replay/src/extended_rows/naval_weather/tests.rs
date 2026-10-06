use super::*;

#[test]
fn temperature_and_raw_mud_deltas_keep_extreme_native_words() {
    let before = WeatherProvinceRow {
        id: 13_413,
        temperature: crate::rows::Recorded::Value(i64::MIN),
        ..Default::default()
    };
    let after = WeatherProvinceRow {
        temperature: crate::rows::Recorded::Value(i64::MAX),
        raw_mud: crate::rows::Recorded::Value(255),
        ..before.clone()
    };
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 4) | (1 << 5));
    assert_eq!(
        WeatherProvinceRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        WeatherProvinceRow::decode_fields(after.key(), &mut Decoder::new(&bytes)).unwrap(),
        after
    );
}
