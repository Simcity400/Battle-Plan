use super::*;

#[test]
fn weather_suspension_none_is_distinct_from_zero_and_opaque_tags_are_strict() {
    let before = SimulationFeatureRow {
        dlc_mask: 0x1234,
        human_ai: 7,
        weather_suspend: crate::rows::Recorded::Value(None),
        ..Default::default()
    };
    let after = SimulationFeatureRow {
        weather_suspend: crate::rows::Recorded::Value(Some([0, 0])),
        ..before.clone()
    };
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    assert_eq!(
        SimulationFeatureRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        SimulationFeatureRow::decode_fields(0, &mut Decoder::new(&bytes)).unwrap(),
        after
    );
    let tag = bytes.len() - 4; // instant_decisions follows the opaque weather field.
    bytes[tag] = 2;
    assert!(SimulationFeatureRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
    bytes[tag] = 1;
    bytes.pop();
    assert!(SimulationFeatureRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
}
