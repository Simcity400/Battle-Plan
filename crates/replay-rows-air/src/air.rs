use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirTransferInputRow {
    pub wing: u64,
    pub base: u64,
    pub ordinal: Option<u32>,
    pub progress_raw: Option<i64>,
    pub distance_raw: Option<i64>,
}
typed_codec!(AirTransferInputRow, 64, |row| row.wing as u128,
 |key| Self { wing: key as u64, ..Default::default() },
 { 0 => (base): U64, 1 => (ordinal): OptionalU32, 2 => (progress_raw): OptionalI64, 3 => (distance_raw): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirGroundCrewRow {
    pub supply: Recorded<Option<AirSupplySummary>>,
    pub country: u32,
    pub region: u32,
    pub enabled: Option<u32>,
}
typed_codec!(AirGroundCrewRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Self::default() },
 { 0 => (enabled): OptionalU32, 1 => (supply): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirSupplySummary {
    pub severity: i32,
    pub needed: i64,
    pub supplied: i64,
    pub deficit: i64,
}
value_struct!(AirSupplySummary {
    severity: i32,
    needed: i64,
    supplied: i64,
    deficit: i64
});
impl OpaqueValue for AirSupplySummary {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionalMissionRow {
    pub wing: u64,
    pub ordinal: u32,
    pub active_order: Option<u64>,
}
typed_codec!(AirRegionalMissionRow, 64, |row| row.wing as u128,
 |key| Self { wing: key as u64, ..Default::default() },
 { 0 => (ordinal): U32, 1 => (active_order): OptionalU64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirCountryRelationRow {
    pub country: u32,
    pub kind: u8,
    pub ordinal: u32,
    pub other: u32,
}
impl AirCountryRelationRow {
    pub const DONOR: u8 = game_model::air_participants::DONOR;
    pub const ALLY: u8 = game_model::air_participants::ALLY;
    pub const ENEMY: u8 = game_model::air_participants::ENEMY;
}
typed_codec!(AirCountryRelationRow, 66, |row| (row.country as u128) << 34 | (row.kind as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 34) as u32, kind: ((key >> 32) & 3) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (other): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirReferenceWitnessRow {
    pub handle: u64,
    pub base_kind: Option<i32>,
    pub state: Option<u32>,
    pub province: Option<u32>,
    pub host: Option<u64>,
    pub host_owner: Option<u32>,
    pub host_without_task_force: bool,
}
typed_codec!(AirReferenceWitnessRow, 64, |row| row.handle as u128,
 |key| Self { handle: key as u64, ..Default::default() },
 { 0 => (base_kind): OptionalI32, 1 => (state): OptionalU32, 2 => (province): OptionalU32, 3 => (host): OptionalU64, 4 => (host_owner): OptionalU32, 5 => (host_without_task_force): Bool }, |_row| Ok(()));

pub const AIR_MISSION_ZERO_PLANE_FLAGS: u64 = 0x1c_0001_0000;
pub const AIR_MISSION_BASE_EQUIPMENT_FLAGS: u64 = 0x6_0001_0000;
pub const AIR_MISSION_FOREIGN_TARGET_FLAG: u64 = 1 << 18;
pub const AIR_MISSION_MAP_FLAGS: u64 = AIR_MISSION_ZERO_PLANE_FLAGS
    | AIR_MISSION_BASE_EQUIPMENT_FLAGS
    | AIR_MISSION_FOREIGN_TARGET_FLAG
    | AIR_MENU_FIGHTER_FLAGS
    | AIR_MENU_BOMBER_FLAGS
    | AIR_MENU_TRANSPORT_FLAGS;
pub const AIR_MENU_FIGHTER_FLAGS: u64 = 0x1_0000_0c00;
pub const AIR_MENU_BOMBER_FLAGS: u64 = 0x20_b000 | AIR_MISSION_ZERO_PLANE_FLAGS;
pub const AIR_MENU_TRANSPORT_FLAGS: u64 = 1 << 14;
pub const AIR_MENU_FIGHTER_MISSIONS: u32 = 7;
pub const AIR_MENU_NAVAL_MISSIONS: u32 = 0x8190;
pub const AIR_MENU_STRATEGIC_MISSIONS: u32 = 8;
pub const AIR_MENU_RECON_MISSIONS: u32 = 1 << 14;
pub const AIR_MENU_MINE_MISSIONS: u32 = (1 << 12) | (1 << 13);
pub const AIR_MENU_LOGISTICS_MISSIONS: u32 = 1 << 9;
pub const AIR_MENU_SUPPLY_MISSIONS: u32 = 1 << 10;
pub const AIR_MENU_RECON_FEATURE: u32 = 1 << 5;
pub const AIR_MENU_MINES_FEATURE: u32 = 1 << 4;
pub const AIR_MENU_LOGISTICS_FEATURE: u32 = 1 << 7;
pub const AIR_MENU_SUPPLY_FEATURE: u32 = 1 << 3;
pub const AIR_MISSION_REACHED_MEMBERSHIP: u32 = 0x1019a;
pub const AIR_MISSION_REACHED_DRAWS: u32 = (1 << 0) | (1 << 2) | (1 << 14);
pub const AIR_MISSION_REACHED_IDS: u32 = AIR_MISSION_REACHED_MEMBERSHIP | AIR_MISSION_REACHED_DRAWS;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirNormalizationEntry {
    pub present: bool,
    pub raw: Option<i64>,
}
value_struct!(AirNormalizationEntry { present: bool, raw: Option<i64> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirMissionNormalization {
    pub night_stat: Option<i64>,
    pub modifiers: [Option<AirNormalizationEntry>; 2],
}
value_struct!(AirMissionNormalization { night_stat: Option<i64>, modifiers: [Option<AirNormalizationEntry>; 2] });
impl OpaqueValue for AirMissionNormalization {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirMissionInputRow {
    pub wing: u64,
    pub coverage_raw: i64,
    pub reassignment_penalty_raw: i64,
    pub last_target_province: i32,
    pub reached_count: i32,
    pub reached_provinces: Vec<u32>,
    pub definition_map_flags: u64,
    pub cas_ratio_raw: Recorded<Option<i64>>,
    pub interception_alert_gate: Recorded<Option<u32>>,
    pub current_type: Recorded<Option<u32>>,
    pub effective_aircraft: Recorded<Option<i32>>,
    pub normalization: Recorded<Option<AirMissionNormalization>>,
    pub detection_stat_13: Recorded<Option<i64>>,
    pub detection_stat_14: Recorded<Option<i64>>,
    pub patrol_disruption_raw: Recorded<Option<i64>>,
    pub patrol_attack_raw: Recorded<Option<i64>>,
    pub patrol_defence_raw: Recorded<Option<i64>>,
    pub patrol_speed_raw: Recorded<Option<i64>>,
    pub patrol_wing_attack_raw: Recorded<Option<i64>>,
    pub patrol_wing_defence_raw: Recorded<Option<i64>>,
    pub patrol_night_applied_raw: Recorded<Option<i64>>,
}
typed_codec!(AirMissionInputRow, 64, |row| row.wing as u128,
 |key| Self { wing: key as u64, ..Default::default() },
 { 0 => (coverage_raw): I64, 1 => (reassignment_penalty_raw): I64, 2 => (last_target_province): I32, 3 => (reached_count): I32, 4 => (reached_provinces): U32List, 5 => (definition_map_flags): U64, 6 => (cas_ratio_raw): OptionalI64, 7 => (interception_alert_gate): OptionalU32, 8 => (current_type): OptionalU32, 9 => (effective_aircraft): OptionalI32, 10 => (normalization): Opaque, 11 => (detection_stat_13): OptionalI64, 12 => (detection_stat_14): OptionalI64, 13 => (patrol_disruption_raw): OptionalI64, 14 => (patrol_attack_raw): OptionalI64, 15 => (patrol_defence_raw): OptionalI64, 16 => (patrol_speed_raw): OptionalI64, 17 => (patrol_wing_attack_raw): OptionalI64, 18 => (patrol_wing_defence_raw): OptionalI64, 19 => (patrol_night_applied_raw): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirSortieBaseCountryRow {
    pub base: u64,
    pub country: u32,
    pub capacity_raw: i64,
    pub supply_raw: i64,
    pub fuel_demand_raw: Option<i64>,
    pub fuel_allocated_raw: Option<i64>,
}
typed_codec!(AirSortieBaseCountryRow, 96, |row| (row.base as u128) << 32 | row.country as u128,
 |key| Self { base: (key >> 32) as u64, country: key as u32, ..Default::default() },
 { 0 => (capacity_raw): I64, 1 => (supply_raw): I64, 2 => (fuel_demand_raw): OptionalI64, 3 => (fuel_allocated_raw): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirSortieModifierRow {
    pub scope: u8,
    pub subject: u32,
    pub region: u32,
    pub raw: i64,
}
impl AirSortieModifierRow {
    pub const WEATHER: u8 = 0;
    pub const COUNTRY: u8 = 1;
    pub const COUNTRY_REGION: u8 = 2;
    pub const EFFICIENCY_OVERRIDE: u8 = 3;
    pub const DETECTION_OVERRIDE: u8 = 4;
    pub const DETECTION_OVERRIDE_STATUS: u8 = 5;
    pub const MENU_MODIFIERS_STATUS: u8 = 6;
}
typed_codec!(AirSortieModifierRow, 67, |row| (row.scope as u128) << 64 | (row.subject as u128) << 32 | row.region as u128,
 |key| Self { scope: (key >> 64) as u8, subject: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirSortieSpecialBaseRow {
    pub base: u64,
    pub rocket_slots: Option<i32>,
    pub carrier_stance: Option<i32>,
    pub leader_modifier_raw: i64,
    pub task_force_combats: Option<i32>,
    pub task_force_region: Option<u32>,
}
typed_codec!(AirSortieSpecialBaseRow, 64, |row| row.base as u128,
 |key| Self { base: key as u64, ..Default::default() },
 { 0 => (rocket_slots): OptionalI32, 1 => (carrier_stance): OptionalI32, 2 => (leader_modifier_raw): I64, 3 => (task_force_combats): OptionalI32, 4 => (task_force_region): OptionalU32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirSortieModifierMetadataRow {
    pub flags: u32,
    pub feature_selector: u32,
    pub callback_kind: u8,
}
typed_codec!(AirSortieModifierMetadataRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (flags): U32, 1 => (feature_selector): U32, 2 => (callback_kind): U8 }, |_row| Ok(()));

pub const AIR_MISSION_LAY_MINES: u32 = 1 << 12;
pub const AIR_MISSION_NAVAL_PATROL: u32 = 1 << 7;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirNavalPatrolCombatRow {
    pub country: u32,
    pub ordinal: u32,
    pub province: u32,
    pub region: u32,
}
typed_codec!(AirNavalPatrolCombatRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (province): U32, 1 => (region): U32 }, |_row| Ok(()));

pub const AIR_HISTORY_AVAILABILITY_REGION: u32 = u32::MAX;
pub const AIR_HISTORY_AVAILABILITY_INDEX: u32 = u32::MAX;


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirNavalPatrolHistoryRow {
    pub country: u32,
    pub region: u32,
    pub level: u8,
    pub index: u32,
    pub enemy_ships_sunk_raw: i64,
    pub our_fighters_lost_raw: i64,
    pub scenario_channels: Recorded<Option<[i64; 7]>>,
    pub menu: Recorded<Option<[i64; AIR_MENU_EXTRA_CHANNELS]>>,
    pub window: Option<AirHistoryWindow>,
}
impl AirNavalPatrolHistoryRow {
    pub fn window_marker(
        country: u32,
        region: u32,
        level: u8,
        mut window: AirHistoryWindow,
        complete: bool,
    ) -> Self {
        window.complete = complete;
        Self {
            country,
            region,
            level,
            index: u32::MAX,
            scenario_channels: Recorded::Value(None),
            menu: Recorded::Value(None),
            window: Some(window),
            ..Default::default()
        }
    }

    pub fn availability_witness(country: u32) -> Self {
        Self {
            country,
            region: AIR_HISTORY_AVAILABILITY_REGION,
            index: AIR_HISTORY_AVAILABILITY_INDEX,
            scenario_channels: Recorded::Value(Some([0; 7])),
            ..Default::default()
        }
    }

    pub fn is_availability_witness(&self) -> bool {
        self.region == AIR_HISTORY_AVAILABILITY_REGION
            && self.index == AIR_HISTORY_AVAILABILITY_INDEX
    }

    pub fn is_menu(&self) -> bool {
        self.level != 0 || self.is_window()
    }

    pub fn is_window(&self) -> bool {
        self.region != AIR_HISTORY_AVAILABILITY_REGION && self.index == u32::MAX
    }

    pub fn channels(&self) -> Result<[i64; AIR_MENU_CHANNELS], &'static str> {
        let Recorded::Value(Some(extra)) = &self.menu else {
            return Err("air menu history channels unavailable");
        };
        let Recorded::Value(Some(scenario)) = self.scenario_channels else {
            return Err("air menu scenario channels unavailable");
        };
        let known = [
            self.enemy_ships_sunk_raw,
            self.our_fighters_lost_raw,
            scenario[0],
            scenario[1],
            scenario[2],
            scenario[3],
            scenario[4],
            scenario[5],
            scenario[6],
        ];
        let mut rest = extra.iter();
        Ok(std::array::from_fn(
            |channel| match AIR_HISTORY_SCENARIO_CHANNELS
                .iter()
                .position(|&known| known == channel)
            {
                Some(at) => known[at],
                None => *rest.next().unwrap(),
            },
        ))
    }

    pub fn set_channels(&mut self, values: &[i64; AIR_MENU_CHANNELS]) {
        self.enemy_ships_sunk_raw = values[AIR_HISTORY_SCENARIO_CHANNELS[0]];
        self.our_fighters_lost_raw = values[AIR_HISTORY_SCENARIO_CHANNELS[1]];
        self.scenario_channels = Recorded::Value(Some(std::array::from_fn(|at| {
            values[AIR_HISTORY_SCENARIO_CHANNELS[at + 2]]
        })));
        let mut extra = values.iter().enumerate().filter_map(|(at, &value)| {
            (!AIR_HISTORY_SCENARIO_CHANNELS.contains(&at)).then_some(value)
        });
        self.menu = Recorded::Value(Some(std::array::from_fn(|_| extra.next().unwrap())));
    }

    pub fn summation_index(&self, rows: &[Self]) -> Result<Option<u32>, &'static str> {
        let at = rows
            .binary_search_by_key(&(self.country, self.region, self.level, u32::MAX), |row| {
                (row.country, row.region, row.level, row.index)
            })
            .map_err(|_| "air history window metadata missing")?;
        let state = rows[at]
            .window
            .ok_or("air history window metadata unavailable")?;
        state
            .validate()
            .map_err(|_| "air history window metadata invalid")?;
        if self.index >= state.capacity {
            return Err("air history native slot outside capacity");
        }
        Ok(if state.wrapped {
            Some(
                ((u64::from(self.index) + u64::from(state.capacity) - u64::from(state.current))
                    % u64::from(state.capacity)) as u32,
            )
        } else {
            (self.index < state.current).then_some(self.index)
        })
    }

    pub fn has_values(&self) -> bool {
        self.enemy_ships_sunk_raw != 0
            || self.our_fighters_lost_raw != 0
            || matches!(&self.menu, Recorded::Value(Some(values)) if values.iter().any(|&value| value != 0))
            || matches!(&self.scenario_channels, Recorded::Value(values) if values.is_none_or(|values| values != [0; 7]))
    }
}
typed_codec!(AirNavalPatrolHistoryRow, 104, |row| (row.country as u128) << 72 | (row.region as u128) << 40 | (row.level as u128) << 32 | row.index as u128,
|key| Self { country: (key >> 72) as u32, region: (key >> 40) as u32, level: (key >> 32) as u8, index: key as u32, ..Default::default() },
{ 0 => (enemy_ships_sunk_raw): I64, 1 => (our_fighters_lost_raw): I64, 2 => (scenario_channels): Opaque, 3 => (menu): Opaque, 4 => (window): Opaque }, |row| {
   if row.level > 2 { return Err(invalid("air history level is invalid")); }
   if let Some(window) = row.window { window.validate()?; }
   Ok(())
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirHistoryWindow {
    pub complete: bool,
    pub capacity: u32,
    pub current: u32,
    pub wrapped: bool,
}
value_struct!(AirHistoryWindow {
    complete: bool,
    capacity: u32,
    current: u32,
    wrapped: bool
});
impl OpaqueValue for AirHistoryWindow {}

impl AirHistoryWindow {
    pub fn validate(self) -> io::Result<()> {
        if self.capacity == 0 || self.capacity > i32::MAX as u32 || self.current >= self.capacity {
            return Err(invalid("air history ring has an invalid shape"));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirMenuWindow {
    pub current: u32,
    pub wrapped: bool,
    pub records: Vec<[i64; AIR_MENU_CHANNELS]>,
}

pub const AIR_MISSION_TRAINING: u32 = 1 << 11;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AceDeath {
    pub killer_ace: u64,
    pub killer_country: u32,
}
value_struct!(AceDeath {
    killer_ace: u64,
    killer_country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirHistoryEntry {
    pub side: u8,
    pub date: i32,
    pub wing_id: u32,
    pub classification: u8,
    pub region_owner: u32,
    pub planes: i32,
    pub mission_mask: u32,
    pub selection: u8,
    pub copied: u8,
    pub attributed_country: u32,
}
value_struct!(AirHistoryEntry {
    side: u8,
    date: i32,
    wing_id: u32,
    classification: u8,
    region_owner: u32,
    planes: i32,
    mission_mask: u32,
    selection: u8,
    copied: u8,
    attributed_country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirDamageEntry {
    pub side: u8,
    pub date: i32,
    pub wing_id: u32,
    pub classification: u8,
    pub channel: u8,
    pub cause: u32,
    pub amount: i64,
}
value_struct!(AirDamageEntry {
    side: u8,
    date: i32,
    wing_id: u32,
    classification: u8,
    channel: u8,
    cause: u32,
    amount: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WingEquipmentRow {
    pub wing: u64,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
}
typed_codec!(WingEquipmentRow, 96, |row| (row.wing as u128) << 32 | row.ordinal as u128, |key| Self { wing: (key >> 32) as u64, ordinal: key as u32, variant: Default::default(), amount_raw: Default::default() },
 { 0 => (variant): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirBaseRow {
    pub id: u64,
    pub state: Option<u32>,
    pub province: Option<u32>,
    pub host: u64,
    pub capacity: i32,
    pub kind: i32,
    pub carrier: Recorded<Option<AirBaseCarrierInputs>>,
}
typed_codec!(AirBaseRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ..Default::default() },
 { 0 => (state): OptionalU32, 1 => (province): OptionalU32, 2 => (host): U64, 3 => (capacity): I32, 4 => (kind): I32, 5 => (carrier): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WingCumulativeRow {
    pub id: u64,
    pub accidents: i32,
    pub experience: i64,
}
typed_codec!(WingCumulativeRow, 64, |row| row.id as u128, |key| Self { id: key as u64, accidents: Default::default(), experience: Default::default() },
 { 0 => (accidents): I32, 1 => (experience): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionHistoryRow {
    pub country: u32,
    pub region: u32,
    pub entries: Vec<AirHistoryEntry>,
}
typed_codec!(AirRegionHistoryRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, entries: Default::default() },
 { 0 => (entries): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionDamageRow {
    pub country: u32,
    pub region: u32,
    pub entries: Vec<AirDamageEntry>,
}
typed_codec!(AirRegionDamageRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, entries: Default::default() },
 { 0 => (entries): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirCombatDamage {
    pub cause: u32,
    pub amount: i64,
}
value_struct!(AirCombatDamage {
    cause: u32,
    amount: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirCombatRecord {
    pub wing_id: u32,
    pub date: i32,
    pub planes: i32,
    pub mission_mask: u32,
    pub selection: u8,
    pub classification: u8,
    pub copied: u8,
    pub attributed_country: u32,
    pub first_damage: Vec<AirCombatDamage>,
    pub second_damage: Vec<AirCombatDamage>,
    pub equipment: Recorded<Option<Vec<AirCombatEquipment>>>,
}
value_struct!(AirCombatRecord {
    wing_id: u32,
    date: i32,
    planes: i32,
    mission_mask: u32,
    selection: u8,
    classification: u8,
    copied: u8,
    attributed_country: u32,
    first_damage: Vec<AirCombatDamage>,
    second_damage: Vec<AirCombatDamage>
} appended AirCombatRecord {
    equipment: Recorded<Option<Vec<AirCombatEquipment>>>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirCombatEquipment {
    pub variant: u64,
    pub amount_raw: i64,
}
value_struct!(AirCombatEquipment {
    variant: u64,
    amount_raw: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionCombatRow {
    pub country: u32,
    pub region: u32,
    pub owner: u32,
    pub own: std::sync::Arc<Vec<AirCombatRecord>>,
    pub foreign: std::sync::Arc<Vec<AirCombatRecord>>,
}
typed_codec!(AirRegionCombatRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (owner): U32, 1 => (own): Opaque, 2 => (foreign): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirAcePortraitInputs {
    pub cached: Option<Vec<u8>>,
    pub culture: Option<Vec<u8>>,
    pub fallback: Option<Vec<u8>>,
}
value_struct!(AirAcePortraitInputs { cached: Option<Vec<u8>>, culture: Option<Vec<u8>>, fallback: Option<Vec<u8>> });
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirAcePresentation {
    pub gender: u8,
    pub pool_index: i32,
    pub definition: Option<Vec<u8>>,
    pub portrait: Option<AirAcePortraitInputs>,
}
value_struct!(AirAcePresentation {
    gender: u8,
    pool_index: i32,
    definition: Option<Vec<u8>>,
    portrait: Option<AirAcePortraitInputs>,
});
impl OpaqueValue for AirAcePresentation {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirAceRow {
    pub id: u64,
    pub country: u32,
    pub ordinal: u32,
    pub aircraft_type_mask: u32,
    pub assigned_wing: u64,
    pub alive: u8,
    pub handled: u8,
    pub kill_type: u32,
    pub death: Option<AceDeath>,
    pub name: Vec<u8>,
    pub surname: Vec<u8>,
    pub callsign: Vec<u8>,
    pub presentation: Recorded<Option<AirAcePresentation>>,
}
typed_codec!(AirAceRow, 64, |row| row.id as u128, |key| Self { id: key as u64, country: Default::default(), ordinal: Default::default(), aircraft_type_mask: Default::default(), assigned_wing: Default::default(), alive: Default::default(), handled: Default::default(), kill_type: Default::default(), death: Default::default(), name: Default::default(), surname: Default::default(), callsign: Default::default(), presentation: Default::default() },
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (aircraft_type_mask): U32, 3 => (assigned_wing): U64, 4 => (alive): U8, 5 => (handled): U8, 6 => (kill_type): U32, 7 => (death): Opaque, 8 => (name): Bytes, 9 => (surname): Bytes, 10 => (callsign): Bytes, 11 => (presentation): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirStatisticsRow {
    pub country: u32,
    pub region: u32,
    pub year_slot: u32,
    pub year_window_wrapped: u8,
    pub channels: [i64; 24],
}
typed_codec!(AirStatisticsRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, year_slot: Default::default(), year_window_wrapped: Default::default(), channels: Default::default() },
 { 0 => (year_slot): U32, 1 => (year_window_wrapped): U8, 2 => (channels[0]): I64, 3 => (channels[1]): I64, 4 => (channels[2]): I64, 5 => (channels[3]): I64, 6 => (channels[4]): I64, 7 => (channels[5]): I64, 8 => (channels[6]): I64, 9 => (channels[7]): I64, 10 => (channels[8]): I64, 11 => (channels[9]): I64, 12 => (channels[10]): I64, 13 => (channels[11]): I64, 14 => (channels[12]): I64, 15 => (channels[13]): I64, 16 => (channels[14]): I64, 17 => (channels[15]): I64, 18 => (channels[16]): I64, 19 => (channels[17]): I64, 20 => (channels[18]): I64, 21 => (channels[19]): I64, 22 => (channels[20]): I64, 23 => (channels[21]): I64, 24 => (channels[22]): I64, 25 => (channels[23]): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirTheatreRow {
    pub id: u64,
    pub owner: u32,
    pub ordinal: u32,
    pub name: Vec<u8>,
}
typed_codec!(AirTheatreRow, 64, |row| row.id as u128, |key| Self { id: key as u64, owner: Default::default(), ordinal: Default::default(), name: Default::default() },
 { 0 => (owner): U32, 1 => (ordinal): U32, 2 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirTheatreGroupRow {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub name: Vec<u8>,
}
typed_codec!(AirTheatreGroupRow, 64, |row| row.id as u128, |key| Self { id: key as u64, theatre: Default::default(), ordinal: Default::default(), name: Default::default() },
 { 0 => (theatre): U64, 1 => (ordinal): U32, 2 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirTheatreMemberRow {
    pub wing: u64,
    pub group: u64,
    pub ordinal: u32,
}
typed_codec!(AirTheatreMemberRow, 128, |row| (row.group as u128) << 64 | row.wing as u128, |key| Self { group: (key >> 64) as u64, wing: key as u64, ordinal: Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirBaseRegistrationRow {
    pub base: u64,
    pub slot: u32,
    pub capacity_raw: i64,
    pub wings: Vec<u64>,
    pub carrier: Recorded<Option<AirBaseRegistrationCarrierInputs>>,
    pub transport_planes: Recorded<Retired>,
    pub fuel_demand_raw: Recorded<Option<i64>>,
    pub fuel_allocated_raw: Recorded<Option<i64>>,
}
typed_codec!(AirBaseRegistrationRow, 96, |row| (row.base as u128) << 32 | row.slot as u128, |key| Self { base: (key >> 32) as u64, slot: key as u32, ..Default::default() },
 { 0 => (capacity_raw): I64, 1 => (wings): U64List, 2 => (carrier): Opaque, 3 => (transport_planes): Opaque, 4 => (fuel_demand_raw): OptionalI64, 5 => (fuel_allocated_raw): OptionalI64 }, |_row| Ok(()));
impl OpaqueValue for AirHistoryEntry {}
impl OpaqueValue for AirDamageEntry {}
impl OpaqueValue for AirCombatRecord {}
impl OpaqueValue for AceDeath {}
value_struct!(AirWingRaidReference {
    raw: u64,
    resolves: bool
});
impl OpaqueValue for AirWingRaidReference {}
