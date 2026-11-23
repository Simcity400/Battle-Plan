use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineCountryRow {
    pub slot: u32,
    pub country: u32,
    pub expiry_events: u64,
}
typed_codec!(DoctrineCountryRow, 32, |row| row.slot as u128,
 |key| Self { slot: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (expiry_events): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineFolderRow {
    pub slot: u32,
    pub ordinal: u32,
    pub folder: u32,
    pub grand_doctrine: Option<u32>,
    pub completed_subdoctrines: Vec<u32>,
}
typed_codec!(DoctrineFolderRow, 64, |row| ((row.slot as u128) << 32) | (row.ordinal as u128),
 |key| Self { slot: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (folder): U32, 1 => (grand_doctrine): OptionalU32, 2 => (completed_subdoctrines): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineTrackRow {
    pub slot: u32,
    pub folder: u32,
    pub ordinal: u32,
    pub sub_doctrine: Option<u32>,
    pub rewards: i32,
    pub mastery: i64,
    pub mastery_bank: i64,
    pub daily_mastery: i64,
    pub effective_daily_mastery: i64,
}
typed_codec!(DoctrineTrackRow, 96,
 |row| ((row.slot as u128) << 64) | ((row.folder as u128) << 32) | (row.ordinal as u128),
 |key| Self { slot: (key >> 64) as u32, folder: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (sub_doctrine): OptionalU32, 1 => (rewards): I32, 2 => (mastery): I64, 3 => (mastery_bank): I64, 4 => (daily_mastery): I64, 5 => (effective_daily_mastery): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineLeaderMasteryRow {
    pub slot: u32,
    pub folder: u32,
    pub track: u32,
    pub ordinal: u32,
    pub leader: u64,
    pub mastery: i64,
}
typed_codec!(DoctrineLeaderMasteryRow, 128,
 |row| ((row.slot as u128) << 96) | ((row.folder as u128) << 64) | ((row.track as u128) << 32) | (row.ordinal as u128),
 |key| Self { slot: (key >> 96) as u32, folder: (key >> 64) as u32, track: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (leader): U64, 1 => (mastery): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineCostReductionRow {
    pub slot: u32,
    pub ordinal: u32,
    pub uses: i32,
    pub name: Vec<u8>,
    pub cost_factor: i64,
    pub folder: Option<u32>,
}
typed_codec!(DoctrineCostReductionRow, 64, |row| ((row.slot as u128) << 32) | (row.ordinal as u128),
 |key| Self { slot: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (uses): I32, 1 => (name): Bytes, 2 => (cost_factor): I64, 3 => (folder): OptionalU32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineTrackFilter {
    pub track: Option<u32>,
    pub folder: Option<u32>,
    pub grand_doctrine: Option<u32>,
    pub sub_doctrine: Option<u32>,
    pub track_index: Option<i32>,
}
value_struct!(DoctrineTrackFilter { track: Option<u32>, folder: Option<u32>, grand_doctrine: Option<u32>, sub_doctrine: Option<u32>, track_index: Option<i32> });
opaque_value!(DoctrineTrackFilter);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineMasteryGrantRow {
    pub slot: u32,
    pub ordinal: u32,
    pub name: Option<Vec<u8>>,
    pub relevant_tracks: DoctrineTrackFilter,
    pub daily_mastery: i64,
    pub bonus: i64,
    pub days_at_anchor: i32,
    pub expiry_events_at_anchor: u64,
}
impl DoctrineMasteryGrantRow {
    pub fn days_at(&self, expiry_events: u64) -> i32 {
        self.days_at_anchor
            .wrapping_sub(expiry_events.wrapping_sub(self.expiry_events_at_anchor) as i32)
    }
}

typed_codec!(DoctrineMasteryGrantRow, 64, |row| ((row.slot as u128) << 32) | (row.ordinal as u128),
 |key| Self { slot: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (name): Opaque, 1 => (relevant_tracks): Opaque, 2 => (daily_mastery): I64, 3 => (bonus): I64, 4 => (days_at_anchor): I32, 5 => (expiry_events_at_anchor): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DoctrineEquipmentBonusRow {
    pub slot: u32,
    pub ordinal: u32,
    pub id: u32,
    pub index: i32,
    pub equipment_bonus: i32,
}
typed_codec!(DoctrineEquipmentBonusRow, 64, |row| ((row.slot as u128) << 32) | (row.ordinal as u128),
 |key| Self { slot: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (id): U32, 1 => (index): I32, 2 => (equipment_bonus): I32 }, |_row| Ok(()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctrine_expiry_anchor_wraps_without_rewriting_grant_payloads() {
        let grant = DoctrineMasteryGrantRow {
            days_at_anchor: i32::MIN,
            expiry_events_at_anchor: u64::MAX,
            name: Some(b"mod grant".to_vec()),
            ..Default::default()
        };
        assert_eq!(grant.days_at(u64::MAX), i32::MIN);
        assert_eq!(grant.days_at(0), i32::MAX);
        let decoded = grant.delta(None).unwrap().unwrap().apply(None).unwrap();
        assert_eq!(decoded, grant);
        let previous = DoctrineCountryRow {
            expiry_events: u64::MAX,
            ..Default::default()
        };
        let next = DoctrineCountryRow {
            expiry_events: 0,
            ..previous.clone()
        };
        assert_eq!(next.delta(Some(&previous)).unwrap().unwrap().mask, 1 << 1);
        assert!(grant.delta(Some(&decoded)).unwrap().is_none());
    }

    #[test]
    fn doctrine_mastery_changes_only_its_numeric_column() {
        let previous = DoctrineTrackRow {
            slot: 5_000,
            folder: 257,
            ordinal: 71,
            sub_doctrine: Some(95_001),
            rewards: 9,
            mastery: 123_456_789,
            mastery_bank: -1,
            daily_mastery: 54_321,
            effective_daily_mastery: 98_765,
        };
        let mut next = previous.clone();
        next.mastery += 1;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 2);
        let token = previous.sub_doctrine;
        let decoded = delta.apply(Some(previous)).unwrap();
        assert_eq!(decoded, next);
        assert_eq!(decoded.sub_doctrine, token);
        assert!(decoded.delta(Some(&next)).unwrap().is_none());
    }
}
