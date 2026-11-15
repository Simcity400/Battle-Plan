use super::*;

#[test]
fn scripted_refresh_payload_round_trips_every_ordinal_as_its_own_family() {
    for ordinal in [0, 1, 5_000, u32::MAX] {
        let payload = MapEventPayload::ScriptedMapRefresh(ordinal);
        assert_eq!(payload.family(), MapEventFamily::ScriptedMapRefresh);
        let mut bytes = Vec::new();
        payload.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), payload.encoded_bytes().unwrap());
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(MapEventPayload::decode(&mut decoder).unwrap(), payload);
        decoder.finish().unwrap();
    }
}
