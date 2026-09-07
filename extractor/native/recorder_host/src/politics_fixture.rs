pub(crate) fn append_test_politics_capital_slots(payload: &mut Vec<u8>, hour: i32, ordinal: u64) {
    use compact_contract::country_name_inputs::POLITICS_COUNTRY_ROW_BYTES;
    use compact_contract::country_capital_slot::{CapitalSlots, CountryGroup, Operands, Outcome, Failure};
    payload.splice(48..48, ordinal.to_le_bytes());
    let count = u32_at(payload, 8) as usize;
    let mut cursor = 56;
    let mut groups = Vec::new();
    for _ in 0..count {
        let tag_db_id = u32_at(payload, cursor);
        let flags = u32_at(payload, cursor + 4);
        let base_raw = (flags & (1 << 10) != 0).then(|| i32::from_le_bytes(payload[cursor+28..cursor+32].try_into().unwrap()));
        let override_raw = (flags & (1 << 11) != 0).then(|| i32::from_le_bytes(payload[cursor+32..cursor+36].try_into().unwrap()));
        cursor += POLITICS_COUNTRY_ROW_BYTES;
        for _ in 0..4 {
            let len = u16::from_le_bytes(payload[cursor..cursor+2].try_into().unwrap()) as usize;
            cursor += 2;
            payload.get(cursor..cursor+len).expect("complete fixture country string");
            cursor += len;
        }
        compact_contract::country_name_inputs::decode_token(payload, &mut cursor).unwrap();
        groups.push(CountryGroup { tag_db_id, operands: Operands { base_raw, override_raw },
            outcome: Outcome::Failed(Failure::StateUnreadable) });
    }
    let bytes = CapitalSlots { game_hour_raw: hour, observation_ordinal: ordinal, groups }.encode().unwrap();
    payload.extend_from_slice(&(bytes.len() as u32).to_le_bytes()); payload.extend_from_slice(&bytes);
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at+4].try_into().unwrap())
}
