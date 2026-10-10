use super::*;
use crate::value::Value;

#[test]
fn original_risk_ship_repeats_and_independent_null_refusals_round_trip() {
    let ship = TransportRiskShip {
        ship: Some(u64::MAX),
        class: None,
        formation_flags: Some(u64::MAX),
        positive_equipment_flags: None,
        carrier_air_base: Some(None),
    };
    let packet = TransportRiskShips {
        sample_hour: i32::MIN,
        receivers: Some(vec![
            TransportReceiverShips {
                task_force: u64::MAX,
                count: Some(3),
                ships: Some(vec![
                    ship,
                    ship,
                    TransportRiskShip {
                        carrier_air_base: Some(Some(u64::MAX)),
                        ..ship
                    },
                ]),
            },
            TransportReceiverShips {
                task_force: 1,
                count: Some(i32::MIN),
                ships: None,
            },
        ]),
    };
    let mut bytes = Vec::new();
    packet.encode(&mut bytes).unwrap();
    assert_eq!(Value::encoded_bytes(&packet).unwrap(), bytes.len());
    assert_eq!(packet.try_clone().unwrap(), packet);
    let mut decoder = Decoder::new(&bytes);
    assert_eq!(TransportRiskShips::decode(&mut decoder).unwrap(), packet);
    decoder.finish().unwrap();
    for end in 0..bytes.len() {
        assert!(TransportRiskShips::decode(&mut Decoder::new(&bytes[..end])).is_err());
    }
}
