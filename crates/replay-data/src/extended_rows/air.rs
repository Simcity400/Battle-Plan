use super::*;

/// Direct incoming registration of a relocating wing. A complete table omits
/// known nonmembers. An absent ordinal preserves a failed destination witness
/// or writer certificate independently of the wing census. Progress operands
/// are read only for incoming wings admitted by the native counter/limit gate.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// Ground Crews for demanded regional Air mission icons. Country is canonical
/// slot. A complete table omits observed inactive flags; None is an independent
/// unavailable certificate or read, never an observed false value.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirGroundCrewRow {
    pub supply: Option<AirSupplySummary>,
    pub country: u32,
    pub region: u32,
    pub enabled: Option<u32>,
}
typed_codec!(AirGroundCrewRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Self::default() },
 { 0 => (enabled): OptionalU32, 1 => (supply): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
opaque_value!(Option<AirSupplySummary>);

/// AirRegionalMissions: one registered mission in its country's regional
/// vector. `ordinal` is the wing's position there; its owner and region are
/// the AirWings row's. Aircraft scenarios visit each region's wings in this order.
/// `active_order` is an independently certified increasing label in the country's
/// all-active vector, used by map arrows across regions. Gaps have no meaning;
/// missing certification is unknown and does not invalidate regional ordinals.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirRegionalMissionRow {
    pub wing: u64,
    pub ordinal: u32,
    pub active_order: Option<u64>,
}
typed_codec!(AirRegionalMissionRow, 64, |row| row.wing as u128,
 |key| Self { wing: key as u64, ..Default::default() },
 { 0 => (ordinal): U32, 1 => (active_order): OptionalU64 }, |_row| Ok(()));

/// AirCountryRelations: one position of a country's vectors used to pick
/// whose wings fly for the viewing country: kind 0 the reciprocal
/// air-volunteer donors, kind 1 the wartime allies, kind 2 the enemies, each
/// in stored order. `country` is the owner's canonical slot, `other` the
/// raw reference as stored. A complete table holds every country's vectors;
/// a country without rows has three empty vectors. The table is sliced by
/// `country`: SliceReads holds the hour each owner's rows were last emitted,
/// and every change is emitted at the hour it is observed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirCountryRelationRow {
    pub country: u32,
    pub kind: u8,
    pub ordinal: u32,
    pub other: u32,
}
impl AirCountryRelationRow {
    pub const DONOR: u8 = engine_air_history::air_participant_vectors::DONOR;
    pub const ALLY: u8 = engine_air_history::air_participant_vectors::ALLY;
    pub const ENEMY: u8 = engine_air_history::air_participant_vectors::ENEMY;
}
typed_codec!(AirCountryRelationRow, 66, |row| (row.country as u128) << 34 | (row.kind as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 34) as u32, kind: ((key >> 32) & 3) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (other): U32 }, |_row| Ok(()));

/// AirReferenceWitnesses: the base handles the flying-aircraft map resolves
/// through the reference registry: every wing's pool base and transfer
/// base, including wings with selected missions but no regional registration.
/// A base's fields are present only when AirBases lacks it or
/// disagrees; a probed handle missing from a complete table did not resolve.
/// A state base's province is its cached one, else the first of the manager's
/// state/kind list. A stateless base's `host_owner` is its host ship's
/// task-force roster owner, None when the host does not resolve (the null
/// controller) or when its task force is null, which `host_without_task_force` marks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// Pool-definition AIR_FLAGS bits the flying-aircraft map reads: permit
/// zero-plane wings, send bombers to base equipment, and accept foreign scout targets.
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
/// Missions whose target scans test reached-province membership: tactical
/// bit 1, strategic 3 and 16, naval 4 and 7, port 8.
pub const AIR_MISSION_REACHED_MEMBERSHIP: u32 = 0x1019a;
/// Missions whose scenario province comes from the reached IDs:
/// fighters (bits 0 and 2; bit 7 is in the membership set) and scouts (14).
pub const AIR_MISSION_REACHED_DRAWS: u32 = (1 << 0) | (1 << 2) | (1 << 14);
/// Missions whose reached IDs are recorded.
pub const AIR_MISSION_REACHED_IDS: u32 = AIR_MISSION_REACHED_MEMBERSHIP | AIR_MISSION_REACHED_DRAWS;

/// A native requested-key lookup can prove an entry present while its disabled
/// feature skips the raw value. Playback admits that absence only under the
/// recorded definition's disabled gate.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirNormalizationEntry {
    pub present: bool,
    pub raw: Option<i64>,
}
value_struct!(AirNormalizationEntry { present: bool, raw: Option<i64> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirMissionNormalization {
    pub night_stat: Option<i64>,
    pub modifiers: [Option<AirNormalizationEntry>; 2],
}
value_struct!(AirMissionNormalization { night_stat: Option<i64>, modifiers: [Option<AirNormalizationEntry>; 2] });
opaque_value!(Option<AirMissionNormalization>);

/// AirMissionInputs: each registered mission's stored operands for
/// flying-aircraft admission and count: region coverage and reassignment
/// penalty for sortie efficiency, the last target and the reached-province
/// vector for scenarios. The IDs are kept only for missions in
/// [`AIR_MISSION_REACHED_IDS`], with repeats and in stored order: setting a
/// region appends its provinces in roster order, and scenario draws index
/// into that order. The vector can be stale after a transfer.
///
/// Flying-aircraft counts recompute efficiency from current state each redraw.
/// Regional statistics instead read the retained current execution and count.
/// The hourly air update
/// writes the stored values before combat only after coverage, wing status,
/// mission eligibility, day/night period, capacity and carrier-combat gates
/// pass; otherwise they retain an older value. That update also requires
/// a zero base status where the map's calculation does not.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirMissionInputRow {
    pub wing: u64,
    pub coverage_raw: i64,
    pub reassignment_penalty_raw: i64,
    pub last_target_province: i32,
    pub reached_count: i32,
    pub reached_provinces: Vec<u32>,
    /// The pool definition's AIR_FLAGS within [`AIR_MISSION_MAP_FLAGS`].
    pub definition_map_flags: u64,
    /// Raw CAS diversion probability operand.
    /// None is a guarded-read refusal, Some(0) an observed zero; older
    /// recordings retain NotRecorded.
    pub cas_ratio_raw: Recorded<Option<i64>>,
    /// Whether the mission's retained base-to-region trace has more than one entry (0/1), the
    /// regional interception-alert gate. None is an independent read refusal.
    pub interception_alert_gate: Recorded<Option<u32>>,
    /// Current simulation mask, independent of selected missions and the saved active byte.
    pub current_type: Recorded<Option<u32>>,
    /// Retained simulation count; present only for nonzero current execution.
    pub effective_aircraft: Recorded<Option<i32>>,
    /// Only executing nonzero-count missions demand these independent raw operands.
    pub normalization: Recorded<Option<AirMissionNormalization>>,
    /// Raw cached statistics selected by positive one-bit current execution,
    /// only when retained aircraft is nonzero. Independent numeric deltas.
    pub detection_stat_13: Recorded<Option<i64>>,
    pub detection_stat_14: Recorded<Option<i64>>,
    /// Independently delta-encoded raw disruption and its demanded combat leaves.
    pub patrol_disruption_raw: Recorded<Option<i64>>,
    pub patrol_attack_raw: Recorded<Option<i64>>,
    pub patrol_defence_raw: Recorded<Option<i64>>,
    pub patrol_speed_raw: Recorded<Option<i64>>,
    pub patrol_wing_attack_raw: Recorded<Option<i64>>,
    pub patrol_wing_defence_raw: Recorded<Option<i64>>,
    /// Held sampled clock accumulator marker; Updated clocks derive it.
    pub patrol_night_applied_raw: Recorded<Option<i64>>,
}
typed_codec!(AirMissionInputRow, 64, |row| row.wing as u128,
 |key| Self { wing: key as u64, ..Default::default() },
 { 0 => (coverage_raw): I64, 1 => (reassignment_penalty_raw): I64, 2 => (last_target_province): I32, 3 => (reached_count): I32, 4 => (reached_provinces): U32List, 5 => (definition_map_flags): U64, 6 => (cas_ratio_raw): OptionalI64, 7 => (interception_alert_gate): OptionalU32, 8 => (current_type): OptionalU32, 9 => (effective_aircraft): OptionalI32, 10 => (normalization): Opaque, 11 => (detection_stat_13): OptionalI64, 12 => (detection_stat_14): OptionalI64, 13 => (patrol_disruption_raw): OptionalI64, 14 => (patrol_attack_raw): OptionalI64, 15 => (patrol_defence_raw): OptionalI64, 16 => (patrol_speed_raw): OptionalI64, 17 => (patrol_wing_attack_raw): OptionalI64, 18 => (patrol_wing_defence_raw): OptionalI64, 19 => (patrol_night_applied_raw): OptionalI64 }, |_row| Ok(()));

/// AirSortieBaseCountries: a registered wing's base container for one
/// country, indexed by canonical slot: capacity and supply ratio for the
/// mission country, fuel demand and allocation for the effective country.
/// Rows cover the wing's effective country and its volunteer hosts; fuel is
/// present only for an effective country, the only country whose fuel is
/// queried. A null or all-zero container has no row (the law treats both as zero).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// AirSortieModifiers: raw key-0x11 accumulator entries the sortie law adds:
/// scope 0 a weather region (subject 0), 1 a country's own (region 0), 2 a
/// country-region object. Only nonzero entries of registered missions'
/// regions and effective countries are rows. Scope 3 is every console
/// efficiency override (region 0), zero included; it replaces the whole law.
/// Scope4 retains certified console detection overrides; scope5 is their
/// singleton availability witness (raw0 unavailable, raw1 complete).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirSortieModifierRow {
    pub scope: u8,
    /// Canonical country slot, or zero for weather.
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
    /// Singleton 0/1 certificate witness; a complete zero-row vector is distinct from refusal.
    pub const DETECTION_OVERRIDE_STATUS: u8 = 5;
    pub const MENU_MODIFIERS_STATUS: u8 = 6;
}
typed_codec!(AirSortieModifierRow, 67, |row| (row.scope as u128) << 64 | (row.subject as u128) << 32 | row.region as u128,
 |key| Self { scope: (key >> 64) as u8, subject: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (raw): I64 }, |_row| Ok(()));

/// AirSortieSpecialBases: a registered wing's base operands for applying
/// sortie efficiency to aircraft count: a state rocket base's aggregate
/// launcher level, an ordinary host base's task-force stance, and any host
/// base's fleet-leader key-0x11 entry; and for scenario eligibility: a host
/// base's task-force combat references and, while positive, the region of
/// the task force's location. A host base with a null task force or a stale
/// (null or unresolved) ship has no row. Kind-2 hosts are skipped: their count is always 1.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// AirSortieModifierMetadata: key 0x11's runtime definition, applied to every
/// sortie modifier: flags (bit 4 clamps to [0, 1]) and the feature selector,
/// gated by the DLC mask (SimulationFeatures) when `callback_kind` is 1. A singleton.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirSortieModifierMetadataRow {
    pub flags: u32,
    pub feature_selector: u32,
    pub callback_kind: u8,
}
typed_codec!(AirSortieModifierMetadataRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (flags): U32, 1 => (feature_selector): U32, 2 => (callback_kind): U8 }, |_row| Ok(()));

/// The air mission mask bit of naval mine laying.
pub const AIR_MISSION_LAY_MINES: u32 = 1 << 12;
/// The air mission mask bit of naval patrol, whose fighters fly its scenarios.
pub const AIR_MISSION_NAVAL_PATROL: u32 = 1 << 7;

/// AirNavalPatrolCombats: for each effective country of a registered
/// naval-patrol mission, its task forces whose first combat is naval and in
/// one of the country's patrolled regions, in fleet then task-force order.
/// The row holds the combat's province and that province's stored strategic
/// region, which the scenario compares. `ordinal` counts only these rows;
/// no other table records the fleet or task-force order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirNavalPatrolCombatRow {
    /// Canonical country slot.
    pub country: u32,
    pub ordinal: u32,
    pub province: u32,
    pub region: u32,
}
typed_codec!(AirNavalPatrolCombatRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (province): U32, 1 => (region): U32 }, |_row| Ok(()));

/// Availability metadata for a captured country's demanded window-zero
/// histories. A native ring's summation index cannot reach this sentinel.
pub const AIR_HISTORY_AVAILABILITY_REGION: u32 = u32::MAX;
pub const AIR_HISTORY_AVAILABILITY_INDEX: u32 = u32::MAX;

opaque_value!(Option<[i64; 7]>);

/// Sparse native ring slots; per-level markers certify omitted zero records and cursors.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirNavalPatrolHistoryRow {
    /// Canonical country slot.
    pub country: u32,
    pub region: u32,
    pub level: u8,
    pub index: u32,
    pub enemy_ships_sunk_raw: i64,
    pub our_fighters_lost_raw: i64,
    /// Raw channels 10/17/51/58/44/24/65, independent of the original naval pair.
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

pub const AIR_MENU_CHANNELS: usize = 89;
pub const AIR_MENU_EXTRA_CHANNELS: usize = AIR_MENU_CHANNELS - AIR_HISTORY_SCENARIO_CHANNELS.len();
pub const AIR_HISTORY_SCENARIO_CHANNELS: [usize; 9] = [43, 3, 10, 17, 51, 58, 44, 24, 65];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
opaque_value!(Option<AirHistoryWindow>);

impl AirHistoryWindow {
    pub fn validate(self) -> io::Result<()> {
        if self.capacity == 0 || self.capacity > i32::MAX as u32 || self.current >= self.capacity {
            return Err(invalid("air history ring has an invalid shape"));
        }
        Ok(())
    }
}
opaque_value!(Option<[i64; AIR_MENU_EXTRA_CHANNELS]>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirMenuWindow {
    pub current: u32,
    pub wrapped: bool,
    pub records: Vec<[i64; AIR_MENU_CHANNELS]>,
}

/// The air mission mask bit of training. The map draws the viewing country's
/// training wings in a separate pass by their pool base's region; only a
/// non-null mission region is registered, so an unregistered training wing
/// is found through its pool rather than the regional mission vector.
pub const AIR_MISSION_TRAINING: u32 = 1 << 11;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AceDeath {
    pub killer_ace: u64,
    pub killer_country: u32,
}
value_struct!(AceDeath {
    killer_ace: u64,
    killer_country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// WingEquipment: key wing:ordinal (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WingEquipmentRow {
    pub wing: u64,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
}
typed_codec!(WingEquipmentRow, 96, |row| (row.wing as u128) << 32 | row.ordinal as u128, |key| Self { wing: (key >> 32) as u64, ordinal: key as u32, variant: Default::default(), amount_raw: Default::default() },
 { 0 => (variant): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

/// AirBases: key id (64 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirBaseRow {
    pub id: u64,
    pub state: Option<u32>,
    pub province: Option<u32>,
    pub host: u64,
    pub capacity: i32,
    pub kind: i32,
    /// Fresh same-hour carrier facts. Historical absence stays NotRecorded.
    pub carrier: Recorded<Option<AirBaseCarrierInputs>>,
}
typed_codec!(AirBaseRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ..Default::default() },
 { 0 => (state): OptionalU32, 1 => (province): OptionalU32, 2 => (host): U64, 3 => (capacity): I32, 4 => (kind): I32, 5 => (carrier): Opaque }, |_row| Ok(()));

/// WingCumulative: key id (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WingCumulativeRow {
    pub id: u64,
    pub accidents: i32,
    pub experience: i64,
}
typed_codec!(WingCumulativeRow, 64, |row| row.id as u128, |key| Self { id: key as u64, accidents: Default::default(), experience: Default::default() },
 { 0 => (accidents): I32, 1 => (experience): I64 }, |_row| Ok(()));

/// Retired: only the save comparison read the regional air history and
/// damage. Kept for older recordings' framing.
///
/// AirRegionHistory: key country:region (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirRegionHistoryRow {
    pub country: u32,
    pub region: u32,
    pub entries: Vec<AirHistoryEntry>,
}
typed_codec!(AirRegionHistoryRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, entries: Default::default() },
 { 0 => (entries): Opaque }, |_row| Ok(()));

/// AirRegionDamage: key country:region (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirRegionDamageRow {
    pub country: u32,
    pub region: u32,
    pub entries: Vec<AirDamageEntry>,
}
typed_codec!(AirRegionDamageRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, entries: Default::default() },
 { 0 => (entries): Opaque }, |_row| Ok(()));

/// One damage entry of a retained record: the cause dispatched by the record updater
/// and its signed fixed5 amount, summed per cause by the damage-entry updater.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirCombatDamage {
    pub cause: u32,
    pub amount: i64,
}
value_struct!(AirCombatDamage {
    cause: u32,
    amount: i64
});

/// One wing combat record (stride 0x98): the wing id, the native hour the updater created it at,
/// the plane high-water mark, the ORed mission mask, the selection and classification bytes, the
/// copied equipment's allow-zero byte and the attribution raw country, with both damage vectors in
/// native order: `first_damage` (causes 0, 1, 2, 6 and 7) and `second_damage` (causes 3, 4, 5, 9
/// and 10), plus the retained equipment pool in native order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    pub equipment: Option<Vec<AirCombatEquipment>>,
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
    second_damage: Vec<AirCombatDamage>,
    equipment: Option<Vec<AirCombatEquipment>>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirCombatEquipment {
    pub variant: u64,
    pub amount_raw: i64,
}
value_struct!(AirCombatEquipment {
    variant: u64,
    amount_raw: i64
});

/// AirRegionCombat: one regional air-combat holder of a country's strategic air region-combat
/// table, key container country (the strategic air's raw country) and strategic region (the table
/// index). `owner` is the holder's raw owner. `own` is the own-records vector (records of wings
/// whose effective country is the owner's, selected when finding or creating a record), `foreign`
/// the foreign-records vector, each in native order with duplicates.
///
/// A row is the holder's native state at the hour it was last read, not necessarily the
/// frame's hour: the collector reads a holder only when the record updater touched it
/// (its records dated at or after the previous read, splicing the rest from the held row)
/// or its sampled check comes up, and carries the row otherwise. The daily pass prunes
/// records the row still holds; playback derives that pruning
/// ([`crate::air_region_derive`]). Allocated holders without records have no row.
/// The record vectors are shared: a carried row is the same allocation hour to hour,
/// encodes as the vector itself and compares by pointer first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
pub struct AirAcePortraitInputs {
    pub cached: Option<Vec<u8>>,
    pub culture: Option<Vec<u8>>,
    /// Empty or failed global selection; refuse only if culture has no sprites.
    pub fallback: Option<Vec<u8>>,
}
value_struct!(AirAcePortraitInputs { cached: Option<Vec<u8>>, culture: Option<Vec<u8>>, fallback: Option<Vec<u8>> });
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
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
opaque_value!(Option<AirAcePresentation>);

/// AirAces: key id (64 bits); presentation is an independent appended field.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// Raw first and last name indices, and the selected definition's name. None is a capture
    /// failure; a null definition is represented inside it.
    pub presentation: Recorded<Option<AirAcePresentation>>,
}
typed_codec!(AirAceRow, 64, |row| row.id as u128, |key| Self { id: key as u64, country: Default::default(), ordinal: Default::default(), aircraft_type_mask: Default::default(), assigned_wing: Default::default(), alive: Default::default(), handled: Default::default(), kill_type: Default::default(), death: Default::default(), name: Default::default(), surname: Default::default(), callsign: Default::default(), presentation: Default::default() },
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (aircraft_type_mask): U32, 3 => (assigned_wing): U64, 4 => (alive): U8, 5 => (handled): U8, 6 => (kill_type): U32, 7 => (death): Opaque, 8 => (name): Bytes, 9 => (surname): Bytes, 10 => (callsign): Bytes, 11 => (presentation): Opaque }, |_row| Ok(()));

/// AirStatistics: key country:region (64 bits), 26 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirStatisticsRow {
    pub country: u32,
    pub region: u32,
    pub year_slot: u32,
    pub year_window_wrapped: u8,
    pub channels: [i64; 24],
}
typed_codec!(AirStatisticsRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, year_slot: Default::default(), year_window_wrapped: Default::default(), channels: Default::default() },
 { 0 => (year_slot): U32, 1 => (year_window_wrapped): U8, 2 => (channels[0]): I64, 3 => (channels[1]): I64, 4 => (channels[2]): I64, 5 => (channels[3]): I64, 6 => (channels[4]): I64, 7 => (channels[5]): I64, 8 => (channels[6]): I64, 9 => (channels[7]): I64, 10 => (channels[8]): I64, 11 => (channels[9]): I64, 12 => (channels[10]): I64, 13 => (channels[11]): I64, 14 => (channels[12]): I64, 15 => (channels[13]): I64, 16 => (channels[14]): I64, 17 => (channels[15]): I64, 18 => (channels[16]): I64, 19 => (channels[17]): I64, 20 => (channels[18]): I64, 21 => (channels[19]): I64, 22 => (channels[20]): I64, 23 => (channels[21]): I64, 24 => (channels[22]): I64, 25 => (channels[23]): I64 }, |_row| Ok(()));

/// AirTheatres: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirTheatreRow {
    pub id: u64,
    pub owner: u32,
    pub ordinal: u32,
    pub name: Vec<u8>,
}
typed_codec!(AirTheatreRow, 64, |row| row.id as u128, |key| Self { id: key as u64, owner: Default::default(), ordinal: Default::default(), name: Default::default() },
 { 0 => (owner): U32, 1 => (ordinal): U32, 2 => (name): Bytes }, |_row| Ok(()));

/// AirTheatreGroups: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirTheatreGroupRow {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub name: Vec<u8>,
}
typed_codec!(AirTheatreGroupRow, 64, |row| row.id as u128, |key| Self { id: key as u64, theatre: Default::default(), ordinal: Default::default(), name: Default::default() },
 { 0 => (theatre): U64, 1 => (ordinal): U32, 2 => (name): Bytes }, |_row| Ok(()));

/// AirTheatreMembers: key group:wing (128 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirTheatreMemberRow {
    pub wing: u64,
    pub group: u64,
    pub ordinal: u32,
}
typed_codec!(AirTheatreMemberRow, 128, |row| (row.group as u128) << 64 | row.wing as u128, |key| Self { group: (key >> 64) as u64, wing: key as u64, ordinal: Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

/// AirBaseRegistrations: key base:slot (96 bits), 6 payload mask bits.
///
/// One country's registered pool membership at one air base. `slot` is the
/// container index, which the engine fills at the country's resolved ordinal.
/// Wings are flattened across the container's pools in pool order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirBaseRegistrationRow {
    /// Full native air base handle; join it to AirBases.
    pub base: u64,
    pub slot: u32,
    /// The container's capacity word; pool registration requires it positive.
    /// The indexed container's presence alone grants base access.
    pub capacity_raw: i64,
    /// Full native air wing handles in native order, repeats included.
    pub wings: Vec<u64>,
    /// Fresh same-hour container and pool facts, including empty pools.
    pub carrier: Recorded<Option<AirBaseRegistrationCarrierInputs>>,
    /// Retired per-pool plane sums of the selected transport carrier preview.
    pub transport_planes: Recorded<Retired>,
    /// Current state-container fuel words. Independent columns let numeric
    /// changes carry unchanged pool metadata. Host bases use their own packet.
    pub fuel_demand_raw: Recorded<Option<i64>>,
    pub fuel_allocated_raw: Recorded<Option<i64>>,
}
typed_codec!(AirBaseRegistrationRow, 96, |row| (row.base as u128) << 32 | row.slot as u128, |key| Self { base: (key >> 32) as u64, slot: key as u32, ..Default::default() },
 { 0 => (capacity_raw): I64, 1 => (wings): U64List, 2 => (carrier): Opaque, 3 => (transport_planes): Opaque, 4 => (fuel_demand_raw): OptionalI64, 5 => (fuel_allocated_raw): OptionalI64 }, |_row| Ok(()));
opaque_value!(Option<Vec<Option<i32>>>);
opaque_value!(Vec<AirHistoryEntry>);
opaque_value!(Vec<AirDamageEntry>);
opaque_value!(std::sync::Arc<Vec<AirCombatRecord>>);
opaque_value!(Option<AceDeath>);
value_struct!(AirWingRaidReference {
    raw: u64,
    resolves: bool
});
opaque_value!(Option<AirWingRaidReference>);
