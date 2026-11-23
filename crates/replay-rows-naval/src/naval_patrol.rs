use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalPatrolMissionInputRow {
    pub task_force: u64,
    pub target: Option<u64>,
    pub target_live: Option<u32>,
    pub progress: Option<i64>,
    pub chance: Option<i64>,
    pub surface_detection: Option<i64>,
    pub carrier_detection: Option<i64>,
    pub local_detection_known: bool,
    pub local_detection: Option<i64>,
    pub weather_detection_known: bool,
    pub weather_detection: Option<i64>,
    pub leader: u64,
    pub headquarters_known: bool,
    pub headquarters: std::sync::Arc<Vec<u64>>,
    pub surface_visibility: Option<i64>,
    pub submarine_visibility: Option<i64>,
    pub formation_class: Option<i32>,
    pub radar_cache: Option<i64>,
    pub air_cache: Option<i64>,
    pub submarine_detection: Option<i64>,
    pub carrier_submarine_detection: Option<i64>,
    pub mission_owner: Option<u32>,
    pub targeted_spotting_known: bool,
    pub targeted_spotting: Option<i64>,
}
typed_codec!(NavalPatrolMissionInputRow, 64, |row|row.task_force as u128,
 |key| Self { task_force:key as u64, ..Default::default() },
 {0 => (target): OptionalU64, 1 => (target_live): OptionalU32,
  2 => (progress): OptionalI64, 3 => (chance): OptionalI64,
  4 => (surface_detection): OptionalI64, 5 => (carrier_detection): OptionalI64,
  6 => (local_detection_known): Bool, 7 => (local_detection): OptionalI64,
  8 => (weather_detection_known): Bool, 9 => (weather_detection): OptionalI64,
  10 => (leader): U64, 11 => (headquarters_known): Bool, 12 => (headquarters): Opaque,
  13 => (surface_visibility): OptionalI64, 14 => (submarine_visibility): OptionalI64,
  15 => (formation_class): OptionalI32, 16 => (radar_cache): OptionalI64, 17 => (air_cache): OptionalI64, 18 => (submarine_detection): OptionalI64, 19 => (carrier_submarine_detection): OptionalI64, 20 => (mission_owner): OptionalU32, 21 => (targeted_spotting_known): Bool, 22 => (targeted_spotting): OptionalI64},
 |row| if row.target_live.is_some_and(|value|value>1) {Err(invalid("invalid patrol target liveness"))} else {Ok(())});


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_progress_keeps_stable_target_and_statistics() {
        let before = NavalPatrolMissionInputRow {
            task_force: 7,
            target: Some(99),
            target_live: Some(1),
            progress: Some(-1),
            chance: Some(1),
            surface_detection: Some(2),
            carrier_detection: Some(3),
            ..Default::default()
        };
        let after = NavalPatrolMissionInputRow {
            progress: Some(i64::MAX),
            ..before.clone()
        };
        let delta = after.delta(Some(&before)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 2);
        assert_eq!(
            NavalPatrolMissionInputRow::apply_delta(&delta, Some(before)).unwrap(),
            after
        );
        assert!(after.delta(Some(&after)).unwrap().is_none());
    }
}
