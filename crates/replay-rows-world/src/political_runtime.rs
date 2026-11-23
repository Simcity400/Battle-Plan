use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PoliticalRuntimeScalars {
    pub sample_hour: PacketHour,
    pub river_small: Option<i64>,
    pub river_large: Option<i64>,
    pub night: Option<i64>,
    pub fort: Option<i64>,
    pub move_strength_threshold: Option<i64>,
    pub move_organisation_threshold: Option<i64>,
}
value_struct!(PoliticalRuntimeScalars { sample_hour: PacketHour, river_small: Option<i64>,
    river_large: Option<i64>, night: Option<i64>, fort: Option<i64>,
    move_strength_threshold: Option<i64>, move_organisation_threshold: Option<i64> });
opaque_value!(PoliticalRuntimeScalars);

#[cfg(test)]
mod tests {
    use super::*;
    use replay_rows::{codec::Decoder, value::Value};
    #[test]
    fn runtime_scalars_exact_signed_values_refusal_and_freshness_round_trip() {
        let values = PoliticalRuntimeScalars {
            sample_hour: PacketHour::Host,
            river_small: Some(i64::MIN),
            river_large: None,
            night: Some(0),
            fort: Some(i64::MAX),
            move_strength_threshold: Some(-1),
            move_organisation_threshold: Some(1),
        };
        let mut bytes = Vec::new();
        Value::encode(&values, &mut bytes).unwrap();
        assert_eq!(Value::encoded_bytes(&values).unwrap(), bytes.len());
        let mut input = Decoder::new(&bytes);
        assert_eq!(PoliticalRuntimeScalars::decode(&mut input).unwrap(), values);
        input.finish().unwrap();
        let mut stamped = (-17_i32).to_le_bytes().to_vec();
        stamped.extend_from_slice(&bytes);
        let mut input = Decoder::new(&stamped);
        input.format = STAMPED_PACKET_FORMAT;
        let historical = PoliticalRuntimeScalars::decode(&mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(
            historical,
            PoliticalRuntimeScalars {
                sample_hour: PacketHour::Stamped(-17),
                ..values
            }
        );
        assert!(Value::encode(&historical, &mut Vec::new()).is_err());
        bytes[0] = 2;
        assert!(PoliticalRuntimeScalars::decode(&mut Decoder::new(&bytes)).is_err());
    }
}
