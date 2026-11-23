use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalMissionTaskForceRow {
    pub task_force: u64,
    pub mission_owner: u32,
    pub raiding_factor: i64,
    pub local_coordination: Option<i64>,
    pub local_raiding: Option<i64>,
    pub local_detection: Option<i64>,
    pub local_speed: Option<i64>,
    pub local_spotting: Option<i64>,
    pub local_night_spotting: Option<i64>,
    pub local_visibility: Option<i64>,
    pub weather_coordination: Option<i64>,
    pub weather_raiding: Option<i64>,
    pub weather_detection: Option<i64>,
    pub weather_speed: Option<i64>,
    pub weather_spotting: Option<i64>,
    pub weather_night_spotting: Option<i64>,
    pub weather_visibility: Option<i64>,
    pub leader: u64,
    pub headquarters: std::sync::Arc<Vec<u64>>,
    pub range_fleet: u64,
    pub range_begin: i32,
    pub range_end: i32,
    pub convoys: i32,
    pub raiding_spotting: Option<i64>,
    pub raiding_region: Option<u32>,
    pub raiding_targets: Option<[Option<bool>; 2]>,
    pub raiding_target_country: Option<u32>,
    pub raiding_target_convoys: Option<i32>,
    pub raiding_transport_invasion: Option<u8>,
    pub raiding_convoy_variant: Option<u64>,
    pub raiding_convoy_surface_detection: Option<i64>,
    pub raiding_convoy_sub_detection: Option<i64>,
    pub raiding_convoy_visibility: Option<i64>,
    pub raiding_convoy_maximum_speed: Option<i64>,
    pub raiding_source_speed: Option<i64>,
    pub raiding_source_terrain_class: Option<i32>,

    pub range_base: i64,
    pub fuel: i64,
    pub fuel_requested: i64,
    pub underway: u8,
    pub replenishment: i32,
    pub replenishment_limit: i32,
    pub local_range: Option<i64>,
    pub local_range_factor: Option<i64>,
    pub weather_range: Option<i64>,
    pub weather_range_factor: Option<i64>,
}
impl NavalMissionTaskForceRow {
    pub fn value(&self, key: u32) -> (Option<i64>, Option<i64>) {
        use engine_reference_values::modifier_keys as keys;
        match key {
            keys::NAVAL_COORDINATION => (self.local_coordination, self.weather_coordination),
            keys::NAVAL_DETECTION => (self.local_detection, self.weather_detection),
            keys::NAVAL_SPEED_FACTOR => (self.local_speed, self.weather_speed),
            keys::SPOTTING_CHANCE => (self.local_spotting, self.weather_spotting),
            keys::NIGHT_SPOTTING_CHANCE => (self.local_night_spotting, self.weather_night_spotting),
            keys::NAVY_VISIBILITY => (self.local_visibility, self.weather_visibility),
            keys::CONVOY_RAIDING_EFFICIENCY_FACTOR => (self.local_raiding, self.weather_raiding),
            keys::NAVAL_RANGE => (self.local_range, self.weather_range),
            keys::NAVAL_RANGE_FACTOR => (self.local_range_factor, self.weather_range_factor),
            _ => unreachable!("unexpected naval mission task-force modifier key"),
        }
    }
}
typed_codec!(NavalMissionTaskForceRow, 64, |row| row.task_force as u128, |key| Self { task_force: key as u64, ..Default::default() },
 { 0 => (raiding_factor): I64, 1 => (local_coordination): OptionalI64, 2 => (local_raiding): OptionalI64, 3 => (weather_coordination): OptionalI64, 4 => (weather_raiding): OptionalI64, 5 => (leader): U64, 6 => (headquarters): Opaque, 7 => (range_fleet): U64, 8 => (range_begin): I32, 9 => (range_end): I32, 10 => (convoys): I32, 11 => (range_base): I64, 12 => (fuel): I64, 13 => (fuel_requested): I64, 14 => (underway): U8, 15 => (replenishment): I32, 16 => (replenishment_limit): I32, 17 => (local_range): OptionalI64, 18 => (local_range_factor): OptionalI64, 19 => (weather_range): OptionalI64, 20 => (weather_range_factor): OptionalI64, 21 => (mission_owner): U32, 22 => (raiding_spotting): OptionalI64, 23 => (raiding_region): OptionalU32, 24 => (raiding_targets): Opaque, 25 => (local_detection): OptionalI64, 26 => (weather_detection): OptionalI64,
 27 => (local_spotting): OptionalI64, 28 => (weather_spotting): OptionalI64,
 29 => (local_night_spotting): OptionalI64, 30 => (weather_night_spotting): OptionalI64,
 31 => (local_visibility): OptionalI64, 32 => (weather_visibility): OptionalI64,
33 => (raiding_target_country): OptionalU32, 34 => (raiding_target_convoys): OptionalI32, 35 => (raiding_transport_invasion): Opaque, 36 => (raiding_convoy_variant): OptionalU64, 37 => (raiding_convoy_surface_detection): OptionalI64, 38 => (raiding_convoy_sub_detection): OptionalI64, 39 => (raiding_convoy_visibility): OptionalI64, 40 => (raiding_convoy_maximum_speed): OptionalI64, 41 => (raiding_source_speed): OptionalI64, 42 => (raiding_source_terrain_class): OptionalI32, 43 => (local_speed): OptionalI64, 44 => (weather_speed): OptionalI64}, |_row| Ok(()));


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalMissionLedgerRow {
    pub owner: u32,
    pub region: u32,
    pub task_forces: Vec<u64>,
    pub ring: bool,
    pub positive: i32,
    pub total: i32,
}
typed_codec!(NavalMissionLedgerRow, 64, |row| (row.owner as u128) << 32 | row.region as u128, |key| Self { owner: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (task_forces): U64List, 1 => (ring): Bool, 2 => (positive): I32, 3 => (total): I32 },
 |row| if row.task_forces.is_empty() || (!row.ring && (row.positive, row.total) != (0, 0)) { Err(invalid("invalid naval mission ledger")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalMissionCountryRow {
    pub country: u32,
    pub coordination: Option<i64>,
    pub detection: Recorded<Option<i64>>,
    pub speed: Recorded<Option<i64>>,
    pub spotting: Recorded<Option<i64>>,
    pub night_spotting: Recorded<Option<i64>>,
    pub visibility: Recorded<Option<i64>>,
    pub raiding: Option<i64>,
    pub escort: Option<i64>,
    pub weather_reduction: Option<i64>,
    pub range: Option<i64>,
    pub range_factor: Option<i64>,
    pub underway: Option<i64>,
    pub hubs: Vec<u32>,
    pub escort_tree: Recorded<Option<Vec<ModifierBreakdownNode>>>,
}
impl NavalMissionCountryRow {
    pub fn value(&self, key: u32) -> Result<Option<i64>, &'static str> {
        use engine_reference_values::modifier_keys as keys;
        match key {
            keys::NAVAL_COORDINATION => Ok(self.coordination),
            keys::NAVAL_DETECTION => recorded(&self.detection, "naval mission detection not recorded"),
            keys::NAVAL_SPEED_FACTOR => recorded(&self.speed, "naval mission speed not recorded"),
            keys::SPOTTING_CHANCE => recorded(&self.spotting, "naval mission spotting not recorded"),
            keys::NIGHT_SPOTTING_CHANCE => recorded(&self.night_spotting, "naval mission night_spotting not recorded"),
            keys::NAVY_VISIBILITY => recorded(&self.visibility, "naval mission visibility not recorded"),
            keys::CONVOY_RAIDING_EFFICIENCY_FACTOR => Ok(self.raiding),
            keys::CONVOY_ESCORT_EFFICIENCY => Ok(self.escort),
            keys::NAVY_WEATHER_PENALTY => Ok(self.weather_reduction),
            keys::NAVAL_RANGE => Ok(self.range),
            keys::NAVAL_RANGE_FACTOR => Ok(self.range_factor),
            keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR => Ok(self.underway),
            _ => unreachable!("unexpected naval mission country modifier key"),
        }
    }
}
fn recorded(
    value: &Recorded<Option<i64>>,
    missing: &'static str,
) -> Result<Option<i64>, &'static str> {
    match value {
        Recorded::Value(value) => Ok(*value),
        Recorded::NotRecorded => Err(missing),
    }
}
typed_codec!(NavalMissionCountryRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (coordination): OptionalI64, 1 => (raiding): OptionalI64, 2 => (escort): OptionalI64, 3 => (weather_reduction): OptionalI64, 4 => (range): OptionalI64, 5 => (range_factor): OptionalI64, 6 => (underway): OptionalI64, 7 => (hubs): U32List, 8 => (detection): OptionalI64, 9 => (spotting): OptionalI64, 10 => (night_spotting): OptionalI64, 11 => (visibility): OptionalI64, 12 => (escort_tree): Opaque, 13 => (speed): OptionalI64 }, |row| match &row.escort_tree { Recorded::Value(Some(nodes)) if !nodes.is_empty() => check_tree(nodes), _ => Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalMissionCommanderRow {
    pub country: u32,
    pub reference: u64,
    pub source: u8,
    pub coordination: Option<i64>,
    pub detection: Recorded<Option<i64>>,
    pub speed: Recorded<Option<i64>>,
    pub spotting: Recorded<Option<i64>>,
    pub night_spotting: Recorded<Option<i64>>,
    pub visibility: Recorded<Option<i64>>,
    pub raiding: Option<i64>,
    pub range: Option<i64>,
    pub range_factor: Option<i64>,
}
impl NavalMissionCommanderRow {
    pub const SOURCES: u8 = 5;
}
impl NavalMissionCommanderRow {
    pub fn value(&self, key: u32) -> Result<Option<i64>, &'static str> {
        use engine_reference_values::modifier_keys as keys;
        match key {
            keys::NAVAL_COORDINATION => Ok(self.coordination),
            keys::NAVAL_DETECTION => recorded(&self.detection, "naval mission detection not recorded"),
            keys::NAVAL_SPEED_FACTOR => recorded(&self.speed, "naval mission speed not recorded"),
            keys::SPOTTING_CHANCE => recorded(&self.spotting, "naval mission spotting not recorded"),
            keys::NIGHT_SPOTTING_CHANCE => recorded(&self.night_spotting, "naval mission night_spotting not recorded"),
            keys::NAVY_VISIBILITY => recorded(&self.visibility, "naval mission visibility not recorded"),
            keys::CONVOY_RAIDING_EFFICIENCY_FACTOR => Ok(self.raiding),
            keys::NAVAL_RANGE => Ok(self.range),
            keys::NAVAL_RANGE_FACTOR => Ok(self.range_factor),
            _ => unreachable!("unexpected naval mission commander modifier key"),
        }
    }
}
typed_codec!(NavalMissionCommanderRow, 104, |row| (row.country as u128) << 72 | (row.reference as u128) << 8 | row.source as u128, |key| Self { country: (key >> 72) as u32, reference: (key >> 8) as u64, source: key as u8, ..Default::default() },
 { 0 => (coordination): OptionalI64, 1 => (raiding): OptionalI64, 2 => (range): OptionalI64, 3 => (range_factor): OptionalI64, 4 => (detection): OptionalI64, 5 => (spotting): OptionalI64, 6 => (night_spotting): OptionalI64, 7 => (visibility): OptionalI64, 8 => (speed): OptionalI64 }, |row| if row.source >= NavalMissionCommanderRow::SOURCES { Err(invalid("invalid naval mission commander source")) } else { Ok(()) });

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escort_tree_delta_is_independent_and_preserves_proven_empty() {
        let previous = NavalMissionCountryRow {
            country: 7,
            escort: Some(25_000),
            escort_tree: Recorded::Value(Some(vec![ModifierBreakdownNode {
                value: 25_000,
                name: b"Idea".to_vec(),
                children: 0,
            }])),
            ..Default::default()
        };
        let next = NavalMissionCountryRow {
            escort_tree: Recorded::Value(Some(Vec::new())),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 12);
        assert_eq!(
            NavalMissionCountryRow::apply_delta(&delta, Some(previous)).unwrap(),
            next
        );
        let refused = NavalMissionCountryRow {
            escort_tree: Recorded::Value(None),
            ..next.clone()
        };
        assert_ne!(refused, next);
    }

    #[test]
    fn spotting_delta_never_repeats_stable_region_or_target_liveness() {
        let previous = NavalMissionTaskForceRow {
            task_force: 7,
            raiding_spotting: Some(-1),
            raiding_region: Some(42),
            raiding_targets: Some([Some(true), Some(false)]),
            ..Default::default()
        };
        let next = NavalMissionTaskForceRow {
            raiding_spotting: Some(i64::MAX),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 22);
        assert_eq!(
            NavalMissionTaskForceRow::apply_delta(&delta, Some(previous)).unwrap(),
            next
        );
        assert!(next.delta(Some(&next)).unwrap().is_none());
    }

    #[test]
    fn convoy_count_delta_carries_design_and_independent_source_stats() {
        let previous = NavalMissionTaskForceRow {
            task_force: 7,
            raiding_target_country: Some(2),
            raiding_target_convoys: Some(3),
            raiding_transport_invasion: Some(0),
            raiding_convoy_variant: Some(9),
            raiding_convoy_surface_detection: Some(-1),
            raiding_source_speed: Some(1234),
            raiding_source_terrain_class: Some(-1),
            ..Default::default()
        };
        let next = NavalMissionTaskForceRow {
            raiding_target_convoys: Some(-4),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1_u64 << 34);
        assert_eq!(
            NavalMissionTaskForceRow::apply_delta(&delta, Some(previous)).unwrap(),
            next
        );
    }
}
