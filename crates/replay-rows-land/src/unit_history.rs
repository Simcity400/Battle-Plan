use super::*;
use std::sync::Arc;

pub type SharedDivisionHistoryRow = Arc<DivisionHistoryRow>;

impl SharedRow for DivisionHistoryRow {
    const KEY_ERROR: &'static str = "division history key exceeds table width";
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionHistoryRow {
    pub division: u64,
    pub list: u8,
    pub ordinal: u32,
    pub available: u8,
    pub reason: Vec<u8>,
    pub amount: i32,
    pub name: Vec<u8>,
    pub custom_name: Vec<u8>,
    pub kind: i32,
    pub target_country: u32,
    pub hour: i32,
    pub medal_token: Option<i32>,
    pub medal_definition: Option<i32>,
    pub medal_enabled: u8,
    pub medal_count: u8,
    pub inherit: u8,
    pub multiplier: i64,
    pub orders: i32,
    pub provinces: Vec<u32>,
    pub sunk_ship: Option<SunkShipEntry>,
}
impl DivisionHistoryRow {
    pub const SUMMARY: u8 = 0;
    pub const CAREER: u8 = 1;
    pub const AWARDS: u8 = 2;
    pub const OPERATION: i32 = 12;
    pub const OPERATION_EXCELLED: i32 = 13;
    pub const NAVAL_COMBAT: i32 = 23;
    pub const NAVAL_COMBAT_EXCELLED: i32 = 24;
}
typed_codec!(DivisionHistoryRow, 98, |row| ((row.division as u128) << 34) | ((row.list as u128) << 32) | row.ordinal as u128,
    |key| Self { division: (key >> 34) as u64, list: ((key >> 32) & 3) as u8, ordinal: key as u32, ..Default::default() }, {
        0 => (available): U8, 1 => (reason): Bytes, 2 => (amount): I32, 3 => (name): Bytes,
        4 => (custom_name): Bytes, 5 => (kind): I32, 6 => (target_country): U32, 7 => (hour): I32,
        8 => (medal_token): OptionalI32, 9 => (medal_definition): OptionalI32, 10 => (medal_count): U8,
        11 => (inherit): U8, 12 => (multiplier): I64, 13 => (orders): I32,
        14 => (provinces): U32List, 15 => (sunk_ship): Opaque, 16 => (medal_enabled): U8
    }, |_row| Ok(()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experience_history_shared_rows_round_trip_and_delta_without_changing_held_entries() {
        let first = Arc::new(DivisionHistoryRow {
            division: u64::MAX,
            list: DivisionHistoryRow::AWARDS,
            ordinal: u32::MAX,
            available: 1,
            name: b"stored subject".to_vec(),
            medal_token: Some(17),
            medal_enabled: 1,
            multiplier: -123_456,
            provinces: vec![7, 3, 7],
            ..Default::default()
        });
        let held = RowCodec::try_clone(&first).unwrap();
        assert!(Arc::ptr_eq(&first, &held));
        let mut bytes = Vec::new();
        first.encode_fields_reserved(&mut bytes).unwrap();
        let decoded =
            SharedDivisionHistoryRow::decode_fields(first.key(), &mut Decoder::new(&bytes))
                .unwrap();
        assert_eq!(decoded, first);
        let changed = Arc::new(DivisionHistoryRow {
            medal_count: 2,
            ..first.as_ref().clone()
        });
        let delta = changed.delta(Some(&first)).unwrap().unwrap();
        assert_eq!(
            SharedDivisionHistoryRow::apply_delta(&delta, Some(first)).unwrap(),
            changed
        );
        assert_eq!(held.medal_count, 0);
        assert_eq!(held.provinces, [7, 3, 7]);
        bytes.clear();
        replay_rows::checkpoint::rows::Row::encode_checkpoint_fields(&held, &mut bytes).unwrap();
        let decoded =
            <SharedDivisionHistoryRow as replay_rows::checkpoint::rows::Row>::decode_checkpoint_fields(
                held.key(),
                &mut Decoder::new(&bytes),
            )
            .unwrap();
        assert_eq!(decoded, held);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionPanelSourceRow {
    pub kind: u8,
    pub owner: u64,
    pub ordinal: u32,
    pub node: Option<DivisionPanelSourceNode>,
    pub reason: Vec<u8>,
}
pub type SharedDivisionPanelSourceRow = Arc<DivisionPanelSourceRow>;
typed_codec!(DivisionPanelSourceRow,104,|row|((row.kind as u128)<<96)|((row.owner as u128)<<32)|row.ordinal as u128,
    |key|Self {kind:(key>>96) as u8,owner:(key>>32) as u64,ordinal:key as u32,..Default::default()}, {
        0=>(node):Opaque,1=>(reason):Bytes
    },|_row|Ok(()));

impl SharedRow for DivisionPanelSourceRow {
    const KEY_ERROR: &'static str = "division panel source key exceeds table width";
}
