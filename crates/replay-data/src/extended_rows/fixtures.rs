use super::*;
use std::fmt::Debug;
impl Fixture for DoctrineCountryRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            slot: key as u32,
            country: u32::sample(tick),
            expiry_events: u64::sample(tick + 1),
        }
    }
}
impl Fixture for DoctrineFolderRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            slot: (key >> 32) as u32,
            ordinal: key as u32,
            folder: <u32>::sample(tick),
            grand_doctrine: <Option<u32>>::sample(tick + 1),
            completed_subdoctrines: <Vec<u32>>::sample(tick + 2),
        }
    }
}
impl Fixture for DoctrineTrackRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            slot: (key >> 64) as u32,
            folder: (key >> 32) as u32,
            ordinal: key as u32,
            sub_doctrine: <Option<u32>>::sample(tick),
            rewards: <i32>::sample(tick + 1),
            mastery: <i64>::sample(tick + 2),
            mastery_bank: <i64>::sample(tick + 3),
            daily_mastery: <i64>::sample(tick + 4),
            effective_daily_mastery: <i64>::sample(tick + 5),
        }
    }
}
impl Fixture for DoctrineLeaderMasteryRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            slot: (key >> 96) as u32,
            folder: (key >> 64) as u32,
            track: (key >> 32) as u32,
            ordinal: key as u32,
            leader: <u64>::sample(tick),
            mastery: <i64>::sample(tick + 1),
        }
    }
}
impl Fixture for DoctrineCostReductionRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            slot: (key >> 32) as u32,
            ordinal: key as u32,
            uses: <i32>::sample(tick),
            name: <Vec<u8>>::sample(tick + 1),
            cost_factor: <i64>::sample(tick + 2),
            folder: <Option<u32>>::sample(tick + 3),
        }
    }
}
impl Fixture for DoctrineMasteryGrantRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            slot: (key >> 32) as u32,
            ordinal: key as u32,
            name: <Option<Vec<u8>>>::sample(tick),
            daily_mastery: <i64>::sample(tick + 1),
            bonus: <i64>::sample(tick + 2),
            days_at_anchor: <i32>::sample(tick + 3),
            expiry_events_at_anchor: <u64>::sample(tick + 4),
            relevant_tracks: DoctrineTrackFilter {
                track_index: Some(tick as i32),
                ..Default::default()
            },
        }
    }
}
impl Fixture for DoctrineEquipmentBonusRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            slot: (key >> 32) as u32,
            ordinal: key as u32,
            id: <u32>::sample(tick),
            index: <i32>::sample(tick + 1),
            equipment_bonus: <i32>::sample(tick + 2),
        }
    }
}
impl Fixture for NavalRegionalMineRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let kind = ((key >> 32) & 1) as u8;
        Self {
            country: (key >> 33) as u32,
            kind,
            region: key as u32,
            amount: if kind == 0 { 0 } else { tick as i64 + 1 },
        }
    }
}
impl Fixture for RocketProductionOutputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let country = (key >> 64) as u32;
        let variant = key as u64;
        let design = variant != 0;
        let unknown_owner = country == 0 && design;
        let inexact = bool::sample(tick) || unknown_owner;
        let ticks = (tick % 3 + 1) as u64;
        let deliveries = (tick % 5 + 1) as u64;
        Self {
            country,
            variant,
            spent_raw: if design { i64::sample(tick) } else { 0 },
            instant_raw: if design { i64::sample(tick + 1) } else { 0 },
            since_hour: i32::sample(tick),
            exact: !inexact,
            ticks,
            inexact_ticks: if inexact { ticks } else { 0 },
            unknown_design_ticks: if !design && inexact { ticks } else { 0 },
            unknown_country_ticks: if country == 0 && inexact { ticks } else { 0 },
            units: if design { i64::sample(tick + 2) } else { 0 },
            units_exact: !inexact,
            deliveries,
            inexact_deliveries: if inexact { deliveries } else { 0 },
        }
    }
}
impl Fixture for EventFeedHistoryRow {
    fn fixture(_key: u128, tick: usize) -> Self {
        use crate::event_feed_rows::{
            EventFeedCause, EventFeedCountry, EventFeedCounts, EventFeedCoverage, EventFeedEvent,
            EventFeedRoute, EventFeedSource,
        };
        let generation = u64::sample(tick);
        let read_hour = tick as i32;
        let outstanding = u64::from(tick % 3 == 1);
        let unavailable = tick % 3 == 2;
        let event_count = u64::from(!tick.is_multiple_of(4));
        Self {
            generation,
            script_refusals: 0,
            script_tokens: Vec::new(),
            script_scopes: Vec::new(),
            windows: crate::rows::Recorded::Value(crate::event_feed_rows::EventWindows {
                verdicts: Ok(Vec::new()),
                presentations: Vec::new(),
                presentations_unrecorded: unavailable.then(|| b"synthetic presentation loss".to_vec()),
            }),
            read_hour,
            coverage: match tick % 3 {
                0 => EventFeedCoverage::Complete,
                1 => EventFeedCoverage::Partial,
                _ => EventFeedCoverage::Unavailable,
            },
            counts: (!unavailable).then_some(EventFeedCounts {
                acquired: event_count + outstanding,
                settled: event_count,
                ..Default::default()
            }),
            outstanding,
            failure: unavailable.then(|| b"synthetic event accounting failure".to_vec()),
            events: if tick.is_multiple_of(4) {
                Vec::new()
            } else {
                vec![EventFeedEvent {
                    script: None,
                    sequence: tick as u64 + 1,
                    generation,
                    hour: read_hour - 1,
                    source: match (tick / 4) % 8 {
                        0 => EventFeedSource::Declaration,
                        1 => EventFeedSource::Join,
                        2 => EventFeedSource::CivilWarWarTransfer,
                        3 => EventFeedSource::Annex,
                        4 => EventFeedSource::WarEnded,
                        5 => EventFeedSource::Release,
                        6 => EventFeedSource::StateTransfer,
                        _ => EventFeedSource::PeaceConference,
                    },
                    actor: EventFeedCountry {
                        reference: u32::sample(tick),
                        id: u32::sample(tick + 1),
                        instance: u32::sample(tick + 2),
                    },
                    target: EventFeedCountry {
                        reference: u32::sample(tick + 1),
                        id: u32::sample(tick + 2),
                        instance: u32::sample(tick),
                    },
                    cause: crate::rows::Recorded::Value(EventFeedCause {
                        route: EventFeedRoute::ALL[tick % EventFeedRoute::ALL.len()],
                        within: EventFeedRoute::ALL[(tick + 1) % EventFeedRoute::ALL.len()],
                        conference: tick.is_multiple_of(3).then(|| u64::sample(tick)),
                        state: tick.is_multiple_of(2).then(|| u32::sample(tick + 3)),
                        ally: None,
                        reason: tick.is_multiple_of(5).then_some(4),
                        flags: tick as u8 & 3,
                    }),
                }]
            },
        }
    }
}
impl<T: Sample> Sample for Recorded<T> {
    fn sample(tick: usize) -> Self {
        Self::Value(T::sample(tick))
    }
}
trait Sample {
    fn sample(tick: usize) -> Self;
}
impl Fixture for EquipmentDestructionRow {
    fn smallest_key() -> u128 {
        1
    }

    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 72) as u32,
            cause: EquipmentDestructionCause::try_from((key >> 64) as u8).unwrap(),
            variant: key as u64,
            amount_raw: if tick.is_multiple_of(2) { 1 } else { -1 }
                * (tick as i64).saturating_add(1),
            ..Default::default()
        }
    }
    fn largest_key() -> u128 {
        (u32::MAX as u128) << 72
            | (EquipmentDestructionCause::ScriptedStockpileFraction as u128) << 64
            | u64::MAX as u128
    }
}
impl Fixture for DivisionEquipmentDeletionRow {
    fn smallest_key() -> u128 {
        1_u128 << 32
    }

    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            division_handle: key as u64,
            country: u32::sample(tick),
            expeditionary_owner: u32::sample(tick),
            disposition: match tick % 13 {
                0..=7 => DivisionDeletionDisposition::Loss,
                8 => DivisionDeletionDisposition::Returned,
                9 => DivisionDeletionDisposition::Merged,
                10 => DivisionDeletionDisposition::HqTransferred,
                11 => DivisionDeletionDisposition::VolunteerTransferred,
                _ => DivisionDeletionDisposition::ExileTransferred,
            },
            cause: match tick % 13 {
                0 => Some(EquipmentDestructionCause::Unknown),
                1 => Some(EquipmentDestructionCause::DivisionDestroyed),
                2 => Some(EquipmentDestructionCause::DivisionOverrun),
                3 => Some(EquipmentDestructionCause::DivisionDeleted),
                4 => Some(EquipmentDestructionCause::DivisionDisbanded),
                5 => Some(EquipmentDestructionCause::DivisionCapitulation),
                6 => Some(EquipmentDestructionCause::DivisionScripted),
                7 => Some(EquipmentDestructionCause::DivisionTemplateRemoved),
                _ => None,
            },
        }
    }
}
impl Fixture for AirLossEventRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            id: key as u64,
            game_hour: i32::sample(tick),
            country: u32::sample(tick),
            region: u32::sample(tick + 1),
            native_cause: (tick % 7) as u32,
            family_base: engine_reference_values::native_values::air_loss::FAMILY_BASES
                [tick % engine_reference_values::native_values::air_loss::FAMILY_BASES.len()],
            credited_enemy: bool::sample(tick),
            amount_raw: tick as i64 + 1,
            opponent: Some(u32::sample(tick)),
            daily_slot: (tick % engine_reference_values::native_values::air_history::DAILY_WINDOW_CAPACITY as usize)
                as u32,
        }
    }
}
impl Fixture for BuildingBombingDamageRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            id: key as u64,
            game_hour: i32::sample(tick),
            attacker: Option::<u32>::sample(tick),
            victim: u32::sample(tick + 1),
            state: u32::sample(tick + 2),
            province: Option::<u32>::sample(tick + 1),
            building_type: u32::sample(tick + 3),
            native_cause: u32::sample(tick + 4),
            cause: BuildingBombingDamageCause::try_from((tick % 8) as u8).unwrap(),
            amount_raw: (tick as i64).saturating_add(1),
            credited_raw: i64::sample(tick),
        }
    }
}
macro_rules! signed_samples {
    ($($ty:ty),+) => { $(impl Sample for $ty {
        fn sample(tick: usize) -> Self { match tick % 3 { 0 => <$ty>::MIN, 1 => -1, _ => <$ty>::MAX } }
    })+ };
}
macro_rules! unsigned_samples {
    ($($ty:ty),+) => { $(impl Sample for $ty {
        fn sample(tick: usize) -> Self { match tick % 3 { 0 => 0, 1 => 1, _ => <$ty>::MAX } }
    })+ };
}
signed_samples!(i8, i16, i32, i64);
unsigned_samples!(u8, u16, u32, u64);
impl Sample for PacketHour {
    fn sample(_tick: usize) -> Self {
        Self::Host
    }
}
impl Sample for bool {
    fn sample(tick: usize) -> Self {
        tick.is_multiple_of(2)
    }
}
impl<T: Sample> Sample for Option<T> {
    fn sample(tick: usize) -> Self {
        if tick.is_multiple_of(4) {
            None
        } else {
            Some(T::sample(tick))
        }
    }
}
impl<T: Sample> Sample for std::sync::Arc<T> {
    fn sample(tick: usize) -> Self {
        Self::new(T::sample(tick))
    }
}
impl<T: Sample> Sample for Vec<T> {
    fn sample(tick: usize) -> Self {
        (0..tick % 3).map(|index| T::sample(tick + index)).collect()
    }
}
impl<T: Sample, const N: usize> Sample for [T; N] {
    fn sample(tick: usize) -> Self {
        std::array::from_fn(|index| T::sample(tick + index))
    }
}
macro_rules! sample_struct {
    ($ty:ty { $($field:ident : $field_ty:ty),* $(,)? }) => {
        impl Sample for $ty { fn sample(tick: usize) -> Self { Self { $($field: <$field_ty as Sample>::sample(tick),)* } } }
    };
}
trait Fixture: RowCodec + Debug {
    fn fixture(key: u128, tick: usize) -> Self;
    fn smallest_key() -> u128 {
        0
    }
    fn largest_key() -> u128 {
        maximum_key::<Self>()
    }
}
macro_rules! fixture {
    ($ty:ident, |$key:ident| { $($key_field:ident: $key_value:expr),* $(,)? },
     { $($field:ident: $field_ty:ty),+ $(,)? }) => {
        impl Fixture for $ty {
            fn fixture($key: u128, tick: usize) -> Self {
                let mut row = Self { $($key_field: $key_value,)* $($field: <$field_ty as Sample>::sample(tick),)+ };
                RowCodec::round_to_display(&mut row);
                row
            }
        }
    };
}
impl Sample for Retired {
    fn sample(_tick: usize) -> Self {
        Self
    }
}
macro_rules! retired_samples {
    ($($ty:ty),+) => {
        $(impl Sample for $ty {
            fn sample(_tick: usize) -> Self {
                Self::default()
            }
        })+
    };
}
impl<T> Sample for RetiredNested<T> {
    fn sample(_tick: usize) -> Self {
        Self::default()
    }
}
impl<T> Sample for RetiredKept<T> {
    fn sample(_tick: usize) -> Self {
        Self::default()
    }
}
retired_samples!(
    RetiredBool,
    RetiredI64,
    RetiredI32,
    RetiredU8,
    RetiredU32,
    RetiredU64,
    RetiredOptionalI64,
    RetiredU32List
);
sample_struct!(AirNormalizationEntry { present: bool, raw: Option<i64> });
sample_struct!(AirMissionNormalization { night_stat: Option<i64>, modifiers: [Option<AirNormalizationEntry>; 2] });
sample_struct!(WeatherDisplay { width: u32, height: u32, refresh_interval: i64,
    refresh_identity: u64, edges: Vec<u8> });
impl<T: Sample> Sample for Witnessed<T> {
    fn sample(tick: usize) -> Self {
        Self((tick % 3 == 1).then(|| T::sample(tick)))
    }
}
sample_struct!(AttackStats { river: [Option<i64>; 2], fort: [Option<i64>; 2], snow: Option<i64>, night: Option<i64> });
sample_struct!(AttackTerrain { terrain: i32, attack: Option<i64> });
sample_struct!(DivisionAttackInputs { sample_hour: PacketHour, template: Option<u64>, stats: AttackStats, terrain: Option<Vec<AttackTerrain>>, acclimatization: Option<i64> });
sample_struct!(CommanderAttackInputs { leader: u64, fort: Option<i64>, river: Option<i64>, snow: Option<i64>, night: Option<i64>, terrain: Option<Vec<AttackTerrain>> });
sample_struct!(OrdersGroupAttackInputs { sample_hour: PacketHour, commander: Option<Option<CommanderAttackInputs>> });
sample_struct!(WeatherRegionAttackInputs { sample_hour: PacketHour, attack: Option<i64>, breakthrough: Option<i64> });
sample_struct!(CountryAttackInputs { sample_hour: PacketHour, land_night_attack: Option<i64>, land_bunker_effectiveness_factor: Option<i64> });
sample_struct!(ProvinceAttackInputs { sample_hour: PacketHour, fort_sum: Option<i32>, land_bunker_effectiveness_factor: Option<i64> });
sample_struct!(DivisionAttackStats { terrain: Vec<i64>, river: [i64; 2], fort: [i64; 2], snow: i64, night: i64 });
sample_struct!(CommanderAttackStats { leader: u64, terrain: Vec<i64>, fort: i64, river: i64, snow: i64, night: i64 });
sample_struct!(DivisionMovementStats { terrain: Vec<i64>, scalars: Option<[i64; 3]>, fuel_rate: i64, air_defense: i64 });
sample_struct!(CommanderMovementStats { terrain: Vec<i64> });
sample_struct!(CountryAttackModifiers { land_night_attack: Option<i64>, land_bunker_effectiveness_factor: Option<i64> });
sample_struct!(CountryMovementModifiers { army_speed_factor: Option<i64>, army_armor_speed_factor: Option<i64>, attrition: Option<i64>, winter_attrition_factor: Option<i64>, heat_attrition_factor: Option<i64>, out_of_supply_factor: Option<i64>, special_forces_out_of_supply_factor: Option<i64>, army_attack_speed_factor: Option<i64>, army_retreat_speed_factor: Option<i64> });
sample_struct!(RegionCombatWeather { army_attack_factor: Option<i64>, breakthrough_factor: Option<i64>, army_speed_factor: Option<i64>, army_attack_speed_factor: Option<i64> });
sample_struct!(ProvinceModifierEntry {
    key: u32,
    value: i64
});
impl Sample for ProvinceWeatherModifiers {
    fn sample(tick: usize) -> Self {
        let mut entries = Vec::<ProvinceModifierEntry>::sample(tick);
        entries.retain(|entry| entry.key != 0);
        entries.sort_by_key(|entry| entry.key);
        entries.dedup_by_key(|entry| entry.key);
        Self { entries }
    }
}
sample_struct!(ProvinceStaticModifier { name: Vec<u8>, days: i32 });
sample_struct!(ProvinceStaticModifiers { entries: Vec<ProvinceStaticModifier> });
impl Sample for PoliticalRuntimeScalars {
    fn sample(tick: usize) -> Self {
        Self {
            sample_hour: PacketHour::sample(tick),
            river_small: Option::<i64>::sample(tick),
            river_large: Option::<i64>::sample(tick + 1),
            night: Option::<i64>::sample(tick + 2),
            fort: Option::<i64>::sample(tick + 3),
            move_strength_threshold: Option::<i64>::sample(tick + 4),
            move_organisation_threshold: Option::<i64>::sample(tick + 5),
        }
    }
}
impl Sample for UnitCombatReference {
    fn sample(tick: usize) -> Self {
        Self {
            combat: u64::sample(tick + 1).max(1),
            kind: ((tick % 5) % 3) as u8,
        }
    }
}
fn unit_combat_references_fixture(
    tick: usize,
    count: u32,
) -> Recorded<Option<UnitCombatReferences>> {
    Recorded::Value(if tick.is_multiple_of(4) || count > 2 {
        None
    } else {
        Some(UnitCombatReferences {
            sample_hour: PacketHour::sample(tick),
            entries: (0..count as usize)
                .map(|index| UnitCombatReference::sample(tick + index))
                .collect(),
        })
    })
}
impl Sample for DirectedWars {
    fn sample(tick: usize) -> Self {
        let matrix_count = [1, 17, 65_536][tick % 3];
        let receiver_slot = [0, 1, 65_535][tick % 3];
        let mut cells = match tick % 5 {
            0 => Vec::new(),
            1 => vec![WarCell {
                target_slot: 0,
                active: true,
            }],
            _ => vec![
                WarCell {
                    target_slot: 0,
                    active: false,
                },
                WarCell {
                    target_slot: matrix_count - 1,
                    active: true,
                },
            ],
        };
        cells.dedup_by_key(|cell| cell.target_slot);
        Self {
            sample_hour: PacketHour::sample(tick),
            receiver_raw_id: i32::sample(tick),
            receiver_slot,
            matrix_count,
            complete_active_domain: tick.is_multiple_of(2),
            cells,
            refused_slots: if matrix_count > 1 && !tick.is_multiple_of(4) {
                vec![matrix_count / 2]
            } else {
                Vec::new()
            },
        }
    }
}
impl Sample for TemplateNameValue {
    fn sample(tick: usize) -> Self {
        match tick % 5 {
            0 => Self::SameAsDivisionTemplates,
            1 => Self::Changed {
                literal: vec![0, 255],
                fallback_key: None,
            },
            2 => Self::Changed {
                literal: Vec::new(),
                fallback_key: Some(Vec::new()),
            },
            _ => Self::Changed {
                literal: Vec::new(),
                fallback_key: Some(vec![255, 0]),
            },
        }
    }
}
sample_struct!(TemplateDisplayName {
    sample_hour: PacketHour,
    value: TemplateNameValue
});
sample_struct!(SettledPeaceAction {
    country: u32, turn: u32, ordinal: u32, token: u32, taker: u32,
    giver: u32, negotiator: u32, status: i32, cost: i32, form: u8,
    state: Option<i32>, flags: Option<[u8; 4]>, country_operand: Option<u32>,
    puppet_auxiliary: Option<u64>, ship: Option<u64>, ratio: Option<i64>,
    name_key: Option<Vec<u8>>, cached_name: Option<Vec<u8>>
});
sample_struct!(SettledPeaceTransfer {
    state: i32,
    taker: u32,
    automatic: bool
});
sample_struct!(SettledPeaceSubject {
    overlord: u32,
    subject: u32
});
sample_struct!(NuclearWarSupport {
    country: u32,
    before_raw: i64,
    after_raw: i64
});
sample_struct!(NuclearBuildingDamage { state: u32, province: Option<u32>, controller: u32, building_type: u32, amount_raw: i64 });
sample_struct!(NuclearEquipmentLoss {
    country: u32,
    wing: bool,
    variant: u64,
    amount_raw: i64
});
sample_struct!(NuclearWingLoss {
    country: u32,
    manpower_before: i64,
    manpower_after: i64
});
sample_struct!(crate::event_feed_rows::EventFeedCountry {
    reference: u32,
    id: u32,
    instance: u32
});
impl Sample for NuclearStrikePart {
    fn sample(tick: usize) -> Self {
        [
            Self::WarSupport,
            Self::Buildings,
            Self::Units,
            Self::Equipment,
        ][tick % 4]
    }
}
impl Sample for NuclearUnitDamage {
    fn sample(tick: usize) -> Self {
        let organisation = Option::<i64>::sample(tick);
        Self {
            handle: u64::sample(tick),
            class: u32::sample(tick),
            owner: u32::sample(tick + 1),
            strength_before: i64::sample(tick),
            strength_after: i64::sample(tick + 1),
            organisation_before: organisation,
            organisation_after: organisation.map(|value| value / 2),
            manpower_before: i64::sample(tick + 2),
            manpower_after: i64::sample(tick + 3),
        }
    }
}
impl Sample for NuclearStrikeRefusal {
    fn sample(tick: usize) -> Self {
        Self {
            part: NuclearStrikePart::sample(tick),
            reason: Vec::<u8>::sample(tick),
            count: u32::sample(tick).max(1),
        }
    }
}
fixture!(NavalAccidentReportRow, |key| {country: (key >> 33) as u32, kind: ((key >> 32) & 1) as u8, ordinal: key as u32}, {region: u32, equipment: u64, damage: i64, sunk: u8, ship: u64, owner: u32, expire_hour: i32, mine_owner: u32});
fixture!(NavalRepairCountryRow, |key| {country: key as u32}, {max_factories_for_repair: i32, max_allowed_dockyards_to_repair: i32, num_used_dockyards: i32});
fixture!(NavalRepairFleetRow, |key| {fleet: key as u64}, {automated_homebase: u8});
fixture!(NavalRepairTaskForceRow, |key| {task_force: key as u64}, {repair_mode: i32, repair_split: u8, repair_target: u32, auto_reinforcement: u8, refit_variant: u64, refit_after_repair: u64, composition_token: u32, merge_after_repair: u64});
fixture!(NavalTaskForceCompositionRow, |key| {task_force: (key >> 40) as u64, kind: (key >> 32) as u8, ordinal: key as u32}, {sub_unit: i32, role: i32, definition: i32, amount: i32});
fixture!(NavalRepairBaseRow, |key| {province: key as u32}, {priority: i32, allocated_dockyards: i32});
fixture!(NavalRepairQueueRow, |key| {province: (key >> 32) as u32, ordinal: key as u32}, {ship: u64});
fixture!(NavalRefitShipRow, |key| {ship: key as u64}, {line: u64});
fixture!(NavalRepairChildRow, |key| {task_force: (key >> 32) as u64, ordinal: key as u32}, {child: u64});
fixture!(NavalRefitProgressRow, |key| {line: key as u64}, {progress_raw: i64});
fixture!(NuclearStrikeRow, |key| {id: key as u64}, {hour: i32, launcher: crate::event_feed_rows::EventFeedCountry, nuke_type: u32, province: u32, state: u32, owner: crate::event_feed_rows::EventFeedCountry, controller: u32, war_support: Vec<NuclearWarSupport>, buildings: Vec<NuclearBuildingDamage>, units: Vec<NuclearUnitDamage>, equipment: Vec<NuclearEquipmentLoss>, refusals: Vec<NuclearStrikeRefusal>, wings: Vec<NuclearWingLoss>});
impl Fixture for PeaceSettlementRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let conference = Option::<u64>::sample(tick);
        let hour = Option::<i32>::sample(tick);
        let failure = if conference.is_none() || hour.is_none() {
            Some(b"synthetic guarded fault".to_vec())
        } else {
            Option::<Vec<u8>>::sample(tick)
        };
        Self {
            sequence: key as u64,
            conference,
            hour,
            winners: Vec::<u32>::sample(tick),
            losers: Vec::<u32>::sample(tick + 1),
            actions: Vec::<SettledPeaceAction>::sample(tick),
            transfers: Vec::<SettledPeaceTransfer>::sample(tick),
            subjects: Vec::<SettledPeaceSubject>::sample(tick),
            failure,
            omitted: u64::sample(tick),
        }
    }
}
sample_struct!(NameOperands { kind: i32, order: i32, ordered: u8, programmatic: u8, custom: Vec<u8>, group: Vec<u8> });
sample_struct!(FuelAwardRecipient { kind: u32, reference: u64, owner: u32, host: u64, amount_raw: i64, name: Option<Vec<u8>>, name_reason: Vec<u8> });
sample_struct!(FuelAwardHour { hour: i32, gains_raw: [i64;4], own_gain_raw: i64, transfers_raw: [i64;4], consumed_raw: i64, received_raw: [i64;3], recipients: Vec<FuelAwardRecipient>, history_index: i32, remaining_hours: i32, pending_raw: [i64;3], current_raw: i64 });
sample_struct!(FuelAwardWindow { country_instance: u32, continuity_start_hour: Option<i32>, hours: Vec<FuelAwardHour>, reason: Vec<u8> });
sample_struct!(FuelAwardDayRecipient { kind: u32, reference: u64, owner: u32, host: u64, name: Option<Vec<u8>>, name_reason: Vec<u8>, first_hour: i32, last_hour: i32, amount_raw: i64 });
sample_struct!(FuelAwardDay { country_instance: u32, continuity_start_hour: Option<i32>, last_hour: FuelAwardDayHour, gains_raw: [i64;4], own_gain_raw: i64, own_gain_quantity_raw: i64, transfers_raw: [i64;4], consumed_raw: i64, received_raw: [i64;3], recipients: Vec<FuelAwardDayRecipient>, reason: Vec<u8> });
sample_struct!(CachedModifier {
    definition: u32,
    value: i64
});
sample_struct!(AdvisorModifierCache { entries: Vec<CachedModifier> });

sample_struct!(DivisionManpowerContributor {
    country: u32,
    amount: i32
});
sample_struct!(LandEquipmentLoss {
    variant: u64,
    amount_raw: i64
});
sample_struct!(LandSideLosses { manpower:i32, equipment:Vec<LandEquipmentLoss> });
sample_struct!(LandTerminalObserver {
    attacker: bool,
    kind: u8,
    item: u64,
    values: [u64; 5]
});
sample_struct!(LandResultSide { countries:Vec<u32>, secondary_countries:Vec<u32>, commander:u64, outcome_flags:u8, progress:i64 });
fixture!(DivisionLandInputsRow, |key| {division:key as u64}, {manpower:Vec<DivisionManpowerContributor>, received_supply:RetiredI64, maximum_grace:RetiredI64, remaining_grace:RetiredI64, naval_transfer:bool});
fixture!(DestroyedDivisionsRow, |key| {id:key as u64}, {division:u64, owner:u32, killer_country:u32, killer_division:Option<u64>, province:u32, destination:Option<u32>, combats:Vec<u64>, hour:i32, triggering_combat:Option<u64>, combat_province:Option<u32>});
fixture!(LandEventCoverageRow, |key| {id:key as u32}, {epoch:u64, from_hour:i32, to_hour:i32, complete:bool, lost_events:u64});
impl Fixture for LandCombatEdgesRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            id: key as u64,
            kind: (tick % 2) as u8,
            hour: i32::sample(tick),
            combat: (key >> 64) as u64,
            division: Option::<u64>::sample(tick),
            side: [0, 1, Self::NO_SIDE][tick % 3],
            manpower: Option::<i32>::sample(tick),
            casualties_before: Option::<i32>::sample(tick),
            casualties_after: Option::<i32>::sample(tick),
            withheld: (tick % 5) as u8,
        }
    }
}

impl Fixture for LandCombatResultsRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            id: key as u64,
            combat: u64::sample(tick),
            province: u32::sample(tick),
            day: i32::sample(tick),
            duration: i32::sample(tick),
            end_hour: i32::sample(tick),
            sides: <[LandResultSide; 2]>::sample(tick),
            winners: (tick % 4) as u8,
            border_war: bool::sample(tick),
            border_end_flags: <[u8; 3]>::sample(tick),
            border_terminating: <[u8; 2]>::sample(tick),
            on_action_winners: Option::<u8>::sample(tick),
            history_countries: <[u32; 2]>::sample(tick),
            history_tags: <[Vec<u8>; 2]>::sample(tick),
            losses: <[LandSideLosses; 2]>::sample(tick),
            observers: Vec::<LandTerminalObserver>::sample(tick),
        }
    }
}

sample_struct!(ProjectState {
    current_offset: u32,
    state_enums: [u32; 4],
    not_completed: u8,
    phase_progress: i64,
    phase_target: i32,
    prototype_progress: i32,
    iterations: i32
});
sample_struct!(ProjectContext { province: i32, progress: i32, character_reference: u64, country_present: u8, country_reference: Option<u32>, scientist_present: u8, scientist_reference: Option<u32>, state_present: u8, state_reference: Option<u32> });
sample_struct!(RaidSource { kind: Option<u32>, kind_present: u8, province: u32, ship_reference_type: u32, ship_reference_id: u32, country: i32, building_type: u32, building_location: i32, building_country: i32 });
sample_struct!(RaidScope { province: u32, state: u32, leader_reference: u64, leader_province: u32, building_type: u32, building_location: u32, building_flags: u32, building_country: i32, leader_country: Option<i32> });
sample_struct!(ScriptVariableEntry { name: Vec<u8>, value_raw: i64 });
sample_struct!(AceDeath {
    killer_ace: u64,
    killer_country: u32
});
sample_struct!(AirHistoryEntry {
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
sample_struct!(AirDamageEntry {
    side: u8,
    date: i32,
    wing_id: u32,
    classification: u8,
    channel: u8,
    cause: u32,
    amount: i64
});
sample_struct!(AirCombatDamage {
    cause: u32,
    amount: i64
});
sample_struct!(AirCombatRecord {
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
    equipment: Recorded<Option<Vec<AirCombatEquipment>>>
});
sample_struct!(AirCombatEquipment {
    variant: u64,
    amount_raw: i64
});
sample_struct!(SunkShipEntry { ship_name: Vec<u8>, attribution_name: Vec<u8>, class_name: Vec<u8>, attribution_class_name: Vec<u8>, victim_country: u32, attacker_country: u32, sink_hour: i32, class_token: u32, attribution_class_token: u32, attribution_class_present: u8, model_level: i32, victim_equipment_variant: u64, damage_source_reference: u64, province: u32, province_present: u8, battle: u64, cause_record: u8, assisted_kill: u8 });
fixture!(WarOwnedRelationRow, |key| {owner: (key >> 32) as u32, ordinal: key as u32}, {first_country: u32, second_country: u32, retired: u8, first_goal_count: u32, second_goal_count: u32});
fixture!(WarRelationGoalRow, |key| {owner: (key >> 96) as u32, relation_ordinal: (key >> 64) as u32, side: (key >> 32) as u32, ordinal: key as u32}, {handle: u64, resolved: u8, type_token: u32, actor: u32, recipient: u32, state_count: u32, first_state: u32, country_count: u32, first_country: u32});
impl Fixture for WarParticipationRow {
    fn largest_key() -> u128 {
        (u64::MAX as u128) << 32 | 1
    }
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            war_first: (key >> 64) as u32,
            war_second: (key >> 32) as u32,
            direction: key as u32,
            first_country: u32::sample(tick),
            second_country: u32::sample(tick + 1),
            equipment_damage: i64::sample(tick),
            province_capture: i64::sample(tick + 1),
            air_damage: i64::sample(tick + 2),
            strategic_bombing: i64::sample(tick + 3),
            sunk_ships: i64::sample(tick + 4),
            sunk_convoys: i64::sample(tick + 5),
            casualties: i64::sample(tick + 6),
            unknown_casualties: i64::sample(tick + 7),
            lend_lease_sent: i64::sample(tick + 8),
            lend_lease_received: i64::sample(tick + 9),
        }
    }
}
impl Fixture for WarCapturedProvinceRow {
    fn largest_key() -> u128 {
        (u64::MAX as u128) << 64 | 1_u128 << 32 | u32::MAX as u128
    }
    fn fixture(key: u128, _tick: usize) -> Self {
        Self {
            war_first: (key >> 96) as u32,
            war_second: (key >> 64) as u32,
            direction: (key >> 32) as u32,
            province: key as u32 as i32,
            credited: 1,
        }
    }
}
impl Fixture for WarLendLeaseHistoryRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 32) as u32,
            target: key as u32,
            industry_sent: i64::sample(tick),
            fuel_sent: i64::sample(tick + 1),
            industry_received: i64::sample(tick + 2),
            fuel_received: i64::sample(tick + 3),
        }
    }
}
fixture!(WarOverviewCountryRow, |key| {country: key as u32}, {major: u8, divisions: i32, factories: i32, view_subject: u8, score_available: u8});
fixture!(WarNameRuleRow, |key| {index: key as u32}, {prefix: Vec<u8>, viewpoint: u32});
fixture!(FocusRow, |key| {country: key as u32}, {tree: Option<Vec<u8>>, current: Option<Vec<u8>>, continuous: Option<Vec<u8>>, progress: i64, paused: u8, completed_count: i32});
fixture!(CompletedFocusRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>});
fixture!(ResearchTechnologyRow, |key| {country: (key >> 32) as u32, database_index: key as u32 as i32}, {technology_handle: u32, applied_bonus_count: i32, classifier: u32, level: i32, researched: u8, progress_raw: i64, ahead_reduction_years_raw: i64});
impl Fixture for ResearchModifierRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 64) as u32,
            index: (key >> 32) as u32,
            ordinal: key as u32,
            technology_handle: u32::sample(tick),
            kind: (tick % (Self::SHARING_ROSTER as usize + 1)) as u8,
            raw: i64::sample(tick),
            auxiliary: i64::sample(tick + 1),
            count: u32::sample(tick),
            source_id: u32::sample(tick),
            name: Vec::<u8>::sample(tick),
        }
    }
}
impl Fixture for CountryStabilityInputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let totals = |offset: usize| {
            let tracked = !(tick + offset).is_multiple_of(3);
            (
                tracked.then(|| i64::sample(tick + offset)),
                if tracked { offset as u32 * 7 } else { u32::MAX },
            )
        };
        let parts: [(Option<i64>, u32); 10] = std::array::from_fn(totals);
        let mut row = Self {
            country: key as u32,
            recruitment_ratio: Recorded::Value(Some(tick as i64)),
            exile_host: Recorded::Value(Some(tick as i32)),
            exile_legitimacy: Recorded::Value(Some(tick as i64)),
            exile_leaders: Recorded::Value(Some(tick as i32)),
            exile_manpower: Recorded::Value(Some(tick as i32)),
            exile_manpower_cap: Recorded::Value(Some(tick as i32)),
            stability_raw: i64::sample(tick),
            war_support_raw: i64::sample(tick + 1),
            at_war: (tick % 2) as u8,
            offensive_war: (tick / 2 % 2) as u8,
            defensive_war: (tick / 4 % 2) as u8,
            ruling_party_popularity_raw: i64::sample(tick + 2),
            propaganda_stability_q15: i64::sample(tick + 3),
            coastal_protection_stability_raw: i64::sample(tick + 4),
            bombing_war_support_q15: i64::sample(tick + 5),
            heroes_war_support_q15: i64::sample(tick + 6),
            convoys_war_support_q15: i64::sample(tick + 7),
            propaganda_war_support_q15: i64::sample(tick + 8),
            interception_sum_raw: i64::sample(tick + 9),
            world_tension_raw: i64::sample(tick + 10),
            base_stability_war_factor: i64::sample(tick + 11),
            base_stability_party_popularity_factor: i64::sample(tick + 12),
            min_stability: i64::sample(tick + 13),
            max_stability: i64::sample(tick + 14),
            war_support_offensive_war: i64::sample(tick + 15),
            war_support_defensive_war: i64::sample(tick + 16),
            war_support_tension_impact: i64::sample(tick + 17),
            min_war_support: i64::sample(tick + 18),
            max_war_support: i64::sample(tick + 19),
            interception_war_support_scale_q15: i64::sample(tick + 20),
            interception_bombing_war_support_impact: i64::sample(tick + 21),
            base_max_command_power: i64::sample(tick + 22),
            keys: parts.iter().map(|(_, key)| *key).collect(),
            children: u32::sample(tick),
            language: Vec::<u8>::sample(tick),
            ..Default::default()
        };
        [
            row.stability_factor,
            row.party_popularity_stability_factor,
            row.war_stability_factor,
            row.offensive_war_stability_factor,
            row.defensive_war_stability_factor,
            row.war_support_factor,
            row.stability_weekly,
            row.war_support_weekly,
        ] = std::array::from_fn(|index| parts[index].0);
        row
    }
}
impl Fixture for CountryStabilityLineRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let kind = (tick % (Self::DYNAMIC_MODIFIER_VALUES as usize + 1)) as u8;
        Self {
            country: (key >> 64) as u32,
            line: (key >> 32) as u32,
            key: key as u32,
            kind,
            identity: Vec::<u8>::sample(tick),
            identity_id: u32::sample(tick),
            secondary: i64::sample(tick + 1),
            value: i64::sample(tick + 2),
            text: if kind == Self::UNPAIRED {
                Vec::<u8>::sample(tick + 1)
            } else {
                Vec::new()
            },
        }
    }
}
fixture!(ResearchInputRow, |key| {country: (key >> 32) as u32, index: key as u32}, {technology_handle: u32, technology_present: u8, token: Vec<u8>, classifier: u32, level: i32, progress_raw: i64, cost_raw: i64, cost_scale_raw: i64, cost_debug: u8, research_speed_raw: i64, banked_days_raw: i64, applied_days_raw: i64, ahead_reduction_raw: i64, speed_bonus_raw: i64, bonus_count: u32, database_index: Recorded<i32>});
fixture!(ResearchAppliedBonusRow, |key| {country: (key >> 64) as u32, index: (key >> 32) as u32, ordinal: key as u32}, {technology_handle: u32, id: i32, resolved: u8, name: Vec<u8>, ahead_reduction_raw: i64, speed_bonus_raw: i64, cost_reduction_raw: Option<i64>});
fixture!(ResearchSlotRow, |key| {country: (key >> 32) as u32, index: key as u32}, {technology_handle: u32, technology_present: u8, research_speed_raw: i64, banked_days_raw: i64, applied_days_raw: i64, database_index: Recorded<i32>});
fixture!(TechnologyDefinitionRow, |key| {database_index: key as u32 as i32}, {technology_handle: u32, year: i32, max_level: i32, research_cost_raw: i64, id: Vec<u8>});
fixture!(PoliticsRow, |key| {country: key as u32}, {last_election: i32, election_frequency: RetiredI32, elections_allowed: RetiredU8, capital: i32, ruling_party: Option<u32>, original_capital: i32, has_elections: Recorded<bool>, election_interval: Recorded<i32>, next_election_cache: Recorded<Option<i32>>});
fixture!(PoliticalPartyRow, |key| {country: (key >> 32) as u32, id: key as u32}, {popularity: i64, is_ruling: u8, token: Vec<u8>, tag: u32, name_default: u8, name: Vec<u8>, long_name: Vec<u8>});
fixture!(ActiveIdeaRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>});
fixture!(DecisionCountsRow, |key| {country: key as u32}, {taken: u32, re_enable: u32, remove: u32, timed: u32, targeted_timed: u32, targeted: u32});
impl Fixture for DecisionRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let container = (key >> 32) as u8;
        Self {
            country: (key >> 40) as u32,
            container,
            ordinal: key as u32,
            token: Vec::<u8>::sample(tick),
            days: i32::sample(tick),
            lifecycle: i32::sample(tick),
            target_kind: u8::sample(tick),
            country_target: u32::sample(tick),
            numeric_target: u32::sample(tick),
            ignore: u8::sample(tick),
            description_seed: if container >= 3 {
                crate::rows::Recorded::Value(Option::<Option<i32>>::sample(tick))
            } else {
                crate::rows::Recorded::Value(None)
            },
        }
    }
}
fixture!(DynamicModifierRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {scope_country: u32, scope_state: i32, days_remaining: i32, enabled: u8, token: Vec<u8>, values: Vec<i64>});
fixture!(PreferredTacticRow, |key| {country: key as u32}, {tactic_id: i32, token: Vec<u8>});
fixture!(CountryTacticUnlockRow, |key| {country: key as u32}, {unlocked_tactic_ids: Vec<u32>});
fixture!(TechSharingGroupRow, |key| {id: key as u32}, {base_bonus: i64, member_count: u32});
fixture!(TechSharingMemberRow, |key| {group: (key >> 32) as u32, ordinal: key as u32}, {country: u32, bonus: i64});
fixture!(PowerBalanceRow, |key| {ordinal: key as u32}, {value_raw: i64, balance: Vec<u8>, left: Vec<u8>, right: Vec<u8>, trending: Vec<u8>, active_range: Vec<u8>});
fixture!(PowerBalanceMemberRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {country: u32, daily_raw: i64, weekly_raw: i64, daily_present: bool, weekly_present: bool, daily_flags: u32, weekly_flags: u32});
fixture!(PowerBalanceRangeRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {identity: Vec<u8>});
fixture!(PowerBalanceOverrideRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {side: Vec<u8>, gfx: Vec<u8>});
fixture!(ProductionLineRow, |key| {handle: key as u64}, {country: u32, kind: u8, variant: u64, mio: u64, active_factories: i32, requested_factories: i32, cost_raw: i64, speed_raw: i64, produced_raw: RetiredI64, priority: i32, amount: i32, max_factories: i32, efficiency_raw: i64});
impl Fixture for CountryProductionModifiersRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: key as u32,
            power_raw: tick as i64,
            base_start: 10 + tick as i32,
            base_cap: 50 + tick as i32,
            gain_raw: tick as i64,
            balance_raw: i64::sample(tick),
            factory_speed: [tick as i64; 4],
            retention: [tick as i64; 4],
            conversion_raw: tick as i64,
            trees: engine_reference_values::native_values::production_modifiers::KEYS
                .into_iter()
                .map(|key| ProductionModifierTree {
                    key,
                    nodes: vec![ModifierBreakdownNode {
                        children: 0,
                        value: tick as i64,
                        name: b"synthetic source".to_vec(),
                    }],
                    failure: Vec::new(),
                })
                .collect(),
        }
    }
}
impl Fixture for ProductionLineModifiersRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            line: key as u64,
            country: tick as u32,
            variant: tick as u64,
            organisation: u64::sample(tick),
            kind: (tick % 4) as u8,
            active_factories: tick as i32,
            resource_penalty_raw: -(tick as i64),
            organisation_dirty: !tick.is_multiple_of(2),
            conversion: RetiredBool::default(),
            licensed: bool::sample(tick + 1),
            naval_efficiency: bool::sample(tick),
            naval_output: bool::sample(tick + 1),
            conversion_output: bool::sample(tick),
            refit_output: bool::sample(tick + 1),
            converting: Recorded::Value(bool::sample(tick + 1)),
            bonuses: [tick as i64; 7],
            factory_efficiencies: vec![tick as i64, -(tick as i64)],
        }
    }
}
fixture!(StockpileRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {variant_handle: u64, amount_raw: i64});
fixture!(DivisionEquipmentRow, |key| {division: (key >> 32) as u64, ordinal: key as u32}, {template: u64, country: u32, variant_handle: u64, amount_raw: i64});
sample_struct!(IndustrialOrganisationBonus {
    key: u32,
    value_raw: i64
});
fixture!(VariantDefinitionRow, |key| {id: key as u64}, {equipment_token: u32, creator_link: u32, origin_link: u32, append_variant_name: u8, obsolete: u8, classifier_flags: u64, mission_mask: u32, display_name: Vec<u8>, variant_name: Vec<u8>, explicit_entity: Vec<u8>, archetype_token: u32, icon_active: u8, icon_kind: u32, icon: Vec<u8>, version: Recorded<u8>, design: Recorded<VariantDesignState>});
sample_struct!(LocalSubunitRequirement {
    archetype_token: u32,
    amount_raw: i64
});
fixture!(CountryLocalSubunitRow, |key| {country: (key >> 32) as u32, token: key as u32}, {requirements: Vec<LocalSubunitRequirement>});
impl Fixture for AvailableEquipmentVariantRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 34) as u32,
            roster_kind: ((key >> 32) & 3) as u8,
            ordinal: key as u32,
            variant: u64::sample(tick),
        }
    }
    fn largest_key() -> u128 {
        (u128::from(u32::MAX) << 34) | (2 << 32) | u128::from(u32::MAX)
    }
}
sample_struct!(UnitBonusStat {
    selector: u32,
    value_raw: i64
});
sample_struct!(UnitBonusGrant {
    category_token: u32,
    add: u8,
    display_as_percentage: u8,
    stats: Vec<UnitBonusStat>
});
sample_struct!(UnitBonusTarget {
    category: bool,
    token: u32,
    combat_width_raw: i64,
    stats: Vec<UnitBonusStat>,
    grants: Vec<UnitBonusGrant>
});
fixture!(CountryUnitBonusRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {source_kind: u8, source_id: u32, source_number: i32, label: Vec<u8>, targets: Vec<UnitBonusTarget>});
fixture!(VariantModuleRow, |key| {variant: (key >> 32) as u64, source_ordinal: key as u32}, {slot: u32, definition_token: u32, definition_active: u8});
impl Sample for VariantDesignState {
    fn sample(tick: usize) -> Self {
        Self {
            parent: Some(tick as u64),
            role_icon_index: tick as i32,
            ..Default::default()
        }
    }
}
fixture!(VariantStatsRow, |key| {variant: key as u64}, {values: [Option<i64>;10], organisation: u64, organisation_trait_count: i32, organisation_bonuses: Vec<IndustrialOrganisationBonus>});
fixture!(VariantUpgradeRow, |key| {variant: (key >> 32) as u64, source_ordinal: key as u32}, {definition_token: i32, level: u8});
fixture!(ArmyUpgradeGroupRow, |key| {army: key as u64}, {country: u32, priority: u32});
fixture!(ArmyUpgradeRequestRow, |key| {army: (key >> 32) as u64, ordinal: key as u32}, {country: u32, equipment: u64, request_raw: i64, total_raw: i64});
fixture!(ArmyUpgradeDeliveryRow, |key| {army: (key >> 32) as u64, ordinal: key as u32}, {country: u32, status: u32, progress_raw: Option<i64>, total_progress_raw: i64, allow_zero_entries: u8});
fixture!(ArmyUpgradeProducedRow, |key| {army: (key >> 64) as u64, delivery_ordinal: (key >> 32) as u32, ordinal: key as u32}, {country: u32, equipment: u64, amount_raw: i64});
fixture!(OrdersGroupRow, |key| {handle: key as u64}, {country: u32, is_army_group: u8, motorization_level: u8, theatre: u64, theatre_ordinal: Option<u32>, theater_group: u64, parent: u64, leader: u64, hq_provider: u64, name: Recorded<Option<Vec<u8>>>});
fixture!(OrdersGroupMemberRow, |key| {group: (key >> 32) as u64, ordinal: key as u32}, {unit: u64});
fixture!(ArmyGroupMemberRow, |key| {group: (key >> 32) as u64, ordinal: key as u32}, {child: u64});
fixture!(IncomingDiplomaticActionRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {handle: u64, status: u8, action_type: i32, actor: u32, original_actor: u32, recipient: u32, original_recipient: u32, hour: i32, last_command_hour: i32});
sample_struct!(ScientistTemplateOrigin { file: Vec<u8>, line: u32 });
fixture!(ScientistRow, |key| {character: key as u32}, {country: u32, ordinal: u32, trait_count: i32, skill_count: i32, active: Recorded<Option<u8>>});
fixture!(ScientistDailyRow, |key| {character: key as u32}, {country: u32, injury_days: Recorded<Option<i32>>, template_visibility: Recorded<Option<u8>>, template_origin: Recorded<Option<ScientistTemplateOrigin>>});
fixture!(ScientistSkillRow, |key| {character: (key >> 32) as u32, ordinal: key as u32}, {token: u32, level: i32, experience_raw: i64});
fixture!(CountryFuelRow, |key| {country: key as u32}, {current_raw: i64, maximum_raw: i64, gain_raw: RetiredI64, cost_raw: RetiredI64, gain_from_states_raw: RetiredI64, lend_lease_gain_raw: i64, lend_lease_consumption_raw: RetiredI64, gain_per_oil_raw: RetiredI64, history_index: RetiredI32, remaining_hours: RetiredI32, ring_gain_raw: i64, ring_consumed_raw: i64, ring_other_raw: i64, ring_requested_0_raw: RetiredI64, ring_requested_1_raw: RetiredI64, ring_requested_2_raw: RetiredI64, ring_received_0_raw: i64, ring_received_1_raw: i64, ring_received_2_raw: i64, country_instance: Recorded<Option<u32>>, awards: Recorded<Retired>, demand: Recorded<FuelDemandRead>, award_day: Recorded<FuelAwardDay>});
impl Sample for FuelDemandHour {
    fn sample(tick: usize) -> Self {
        if tick.is_multiple_of(2) {
            Self::Host
        } else {
            Self::Stamped(i32::sample(tick))
        }
    }
}
impl Sample for FuelAwardDayHour {
    fn sample(tick: usize) -> Self {
        if tick.is_multiple_of(2) {
            Self::Host
        } else {
            Self::Stamped(i32::sample(tick))
        }
    }
}
impl Sample for FuelDemandPart {
    fn sample(tick: usize) -> Self {
        part_of_kind((tick % usize::from(FuelDemandPart::KINDS)) as u8, tick)
    }
}
fn part_of_kind(kind: u8, tick: usize) -> FuelDemandPart {
    let unit = || FuelDemandUnit {
        reference: u64::sample(tick),
        rate_raw: Some(i64::sample(tick)),
        name: Some(Vec::<u8>::sample(tick)),
        ..Default::default()
    };
    match kind {
        0 => FuelDemandPart::Division(unit()),
        1 => FuelDemandPart::Group(FuelDemandGroup {
            reference: u64::sample(tick),
            members: Vec::<u64>::sample(tick),
            ..Default::default()
        }),
        2 => FuelDemandPart::Fleet(FuelDemandFleet {
            reference: u64::sample(tick),
            name_reason: Vec::<u8>::sample(tick),
            ..Default::default()
        }),
        3 => FuelDemandPart::TaskForce(unit()),
        4 => FuelDemandPart::Site(FuelDemandSite {
            list: (tick % 2) as u8,
            ordinal: u32::sample(tick),
            base: u64::sample(tick),
            wings: vec![FuelDemandWing {
                reference: u64::sample(tick),
                context: Option::<u32>::sample(tick),
                ..Default::default()
            }],
            ..Default::default()
        }),
        _ => FuelDemandPart::Context(FuelDemandContext {
            region: u32::sample(tick),
            sums: [i64::sample(tick); 24],
        }),
    }
}
impl Fixture for MapEventsRow {
    fn fixture(_key: u128, tick: usize) -> Self {
        let health = std::array::from_fn(|index| {
            let complete = !(tick + index).is_multiple_of(3);
            let mut faults = MapEventFaults::default();
            if !complete {
                faults.insert(MapEventFault::ALL[(tick + index) % MapEventFault::ALL.len()]);
            }
            MapEventFamilyHealth {
                installed: true,
                enabled: true,
                complete,
                faults,
                missed: (tick + index) as u64,
            }
        });
        let first = tick as u64 + 1;
        let payloads = [
            MapEventPayload::FakeDivisionRevealed(MapEventUnitIntel {
                owner: u32::sample(tick),
                anchor_province: u32::sample(tick + 1),
                visibility_province: u32::sample(tick + 2),
                fog: tick.is_multiple_of(2),
                intel: vec![
                    MapEventIntelEntry {
                        country: 0,
                        level: i8::MIN,
                    },
                    MapEventIntelEntry {
                        country: u32::sample(tick),
                        level: i8::MAX,
                    },
                ],
            }),
            MapEventPayload::ResistanceActivity(MapEventResistance {
                province: u32::sample(tick),
                controller: u32::sample(tick + 1),
                overlord: 0,
                key: Vec::<u8>::sample(tick),
            }),
            MapEventPayload::BuildingDamage(MapEventBuildingDamage {
                building_type: u32::sample(tick),
                province: u32::sample(tick + 1),
                controller: u32::sample(tick + 2),
                reason: MapEventBuildingReason::ALL[tick % 5],
            }),
            MapEventPayload::ScriptedMapRefresh(u32::sample(tick)),
        ];
        let events = payloads
            .into_iter()
            .take(tick % 5)
            .enumerate()
            .map(|(index, payload)| MapEvent {
                sequence: first + index as u64 * 2,
                hour: tick as i32,
                payload,
            })
            .collect::<Vec<_>>();
        Self {
            epoch: (tick / 5) as u64,
            from_hour: tick as i32 - 1,
            to_hour: tick as i32,
            first_sequence: first,
            next_sequence: first + 8,
            health,
            events,
        }
    }
}
impl Fixture for FuelDemandPartRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let kind = ((key >> 80) & 7) as u8;
        let mut part = part_of_kind(kind, tick);
        let ident = (key >> 16) as u64;
        match &mut part {
            FuelDemandPart::Division(unit) | FuelDemandPart::TaskForce(unit) => {
                unit.reference = ident
            }
            FuelDemandPart::Group(group) => group.reference = ident,
            FuelDemandPart::Fleet(fleet) => fleet.reference = ident,
            FuelDemandPart::Site(site) => {
                site.list = (ident >> 32) as u8;
                site.ordinal = ident as u32;
            }
            FuelDemandPart::Context(context) => context.region = ident as u32,
        }
        Self {
            country: (key >> 83) as u32,
            kind,
            ident: part.ident(),
            occurrence: key as u16,
            position: u32::sample(tick),
            part,
        }
    }
    fn largest_key() -> u128 {
        (u32::MAX as u128) << 83
            | ((FuelDemandPart::KINDS - 1) as u128) << 80
            | (u32::MAX as u128) << 16
            | u16::MAX as u128
    }
}
impl Fixture for FuelAwardRecipientRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let name_present = tick.is_multiple_of(2);
        let first_offset = ((tick % 24) as u8).max(1);
        let last_offset = (tick % usize::from(first_offset)) as u8;
        Self {
            country: (key >> 83) as u32,
            kind: (((key >> 80) & 7) as u8).min(2),
            reference: (key >> 16) as u64,
            occurrence: key as u16,
            position: u32::sample(tick),
            owner: u32::sample(tick + 1),
            host: u64::sample(tick + 2),
            name_present,
            name: if name_present { Vec::<u8>::sample(tick + 3) } else { Vec::new() },
            name_reason: Vec::<u8>::sample(tick + 4),
            first_offset,
            last_offset,
            amount_raw: i64::sample(tick + 5),
        }
    }
    fn largest_key() -> u128 {
        (u32::MAX as u128) << 83
            | (2_u128 << 80)
            | (u64::MAX as u128) << 16
            | u16::MAX as u128
    }
}
impl Sample for FuelDemandRead {
    fn sample(tick: usize) -> Self {
        Self {
            hour: FuelDemandHour::sample(tick),
            country_instance: Some(tick as u32),
            divisions: vec![FuelDemandUnit {
                reference: u64::sample(tick),
                rate_raw: Some(i64::sample(tick)),
                name: if tick.is_multiple_of(2) {
                    Some(Vec::new())
                } else {
                    None
                },
                name_reason: if tick.is_multiple_of(2) {
                    Vec::new()
                } else {
                    b"synthetic name refusal".to_vec()
                },
                ..Default::default()
            }],
            ..Default::default()
        }
    }
}
fixture!(CountryFuelConsumerRow, |key| {country: (key >> 32) as u32, index: key as u32}, {priority: i32, requested_raw: i64, received_raw: i64});
fixture!(CountryFactoriesRow, |key| {country: key as u32}, {military_levels: i32, military_total_raw: i64, military_controlled_raw: i64, military_owned_raw: i64, military_damaged_raw: i64, military_used: i32, military_unavailable: i32, military_from_subjects: i32, military_to_overlord: i32, military_from_others: i32, military_to_others: i32, naval_levels: i32, naval_total_raw: i64, naval_controlled_raw: i64, naval_owned_raw: i64, naval_damaged_raw: i64, naval_used: i32, naval_unavailable: i32, naval_from_subjects: i32, naval_to_overlord: i32, naval_from_others: i32, naval_to_others: i32, civilian_levels: i32, civilian_total_raw: i64, civilian_controlled_raw: i64, civilian_owned_raw: i64, civilian_damaged_raw: i64, civilian_used: i32, civilian_unavailable: i32, civilian_from_subjects: i32, civilian_to_overlord: i32, civilian_from_others: i32, civilian_to_others: i32, civilian_license_exports: i32, civilian_trade_exports: i32, consumer_goods: i32, special_projects: i32, license_imports: i32, trade_imports: i32, contract_payments: i32});
fixture!(ResourceDefinitionRow, |key| {id: key as u32}, {token: u32, active: u8, factory_cost: i64});
fixture!(CountryResourceRow, |key| {country: (key >> 32) as u32, resource: key as u32}, {produced: i64, adjustment: i64, imported: i64, used: i64, special_total: i64, production_demand: i64, to_export: i64, base_export: i64, exported: i64});
fixture!(ResourceExchangeRow, |key| {id: key as u64}, {exporter: u32, importer: u32, resource: u32, import_ordinal: u32, export_ordinal: u32, convoys_assigned: i32, convoys_required: i32, efficiency_a: i64, efficiency_b: i64, delivered: i64, required_factories: i32, assigned_factories: i32, no_trade_influence: u8, no_resources: u8});
fixture!(PurchaseContractRow, |key| {id: key as u64}, {seller: u32, buyer: u32, speed: i32, convoys_assigned: i32, convoys_total: i32, route_efficiency_sum_raw: i64, route_days: i32, factory_cic_progress_raw: i64, equipment_transfer_progress_raw: i64, collected_raw: i64, completed_deliveries: i32});
fixture!(ProvinceSupplyRow, |key| {province: key as u32}, {controller_index: u16, capital_country_index: u16, supply_node_level: u8, naval_base_level: u8, flags: u8});
fixture!(HubMotorizationRow, |key| {owner: (key >> 96) as u32, province: (key >> 64) as u32 as i32, node_type: (key >> 32) as u32 as i32, actor: key as u32}, {node_index: u32, map_entry_index: u32, level: u8});
fixture!(DisruptedSupplyRow, |key| {country: (key >> 64) as u32, province: (key >> 32) as u32, node_type: key as u32}, {value: i64});
fixture!(ForeignHomebaseRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {province: u32, supply: i64, start: i64, penalty: i64, added_penalty: i64, duration: i64, hours: i64, decay: i64, base: u8, source: Vec<u8>});
fixture!(CountryExperienceRow, |key| {country: key as u32}, {army_raw: i64, army_daily_raw: i64, army_daily_training_raw: i64, navy_raw: i64, navy_daily_raw: i64, navy_daily_training_raw: i64, air_raw: i64, air_daily_raw: i64, armies_for_training_raw: i64});
impl Fixture for CountryExperienceGainRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 32) as u32,
            instance: key as u32,
            complete: (tick % 2) as u8,
            army_raw: if tick.is_multiple_of(2) { 0 } else { i64::MAX },
            navy_raw: if tick.is_multiple_of(2) { i64::MAX } else { 0 },
            air_raw: tick as i64,
        }
    }
}
fixture!(BuildingRow, |key| {location: (key >> 32) as u32, building_type: key as u32}, {ordinal: u32, status_flags: u32, country: u32, level: i16, healthy_level: i16, partial_health: i64, active: bool});
fixture!(StateBaseResourceRow, |key| {state: (key >> 32) as u32, resource: key as u32}, {ordinal: u32, amount: i64});
fixture!(BuildingConstructionRow, |key| {id: key as u64}, {country: u32, queue_ordinal: u32, active_factories: i32, cost: i64, speed: i64, produced: i64, priority: i32, amount: i32, created_hour: i32, building_type: u32, target_flags: u32, target_location: u32, target_state: u32, target_country: u32, repair_levels: i32});
fixture!(RailwayConstructionRow, |key| {id: key as u64}, {country: u32, queue_ordinal: u32, active_factories: i32, cost: i64, speed: i64, produced: i64, priority: i32, amount: i32, created_hour: i32, base_level: i32, target_level: i32, cancelled: u8, current_segment: i32, remaining_cost: i64, route: Vec<u32>});
impl Sample for OccupationGroupLaw {
    fn sample(tick: usize) -> Self {
        Self {
            occupied: u32::sample(tick),
            scope_country: Option::<u32>::sample(tick),
            law: Option::<Vec<u8>>::sample(tick),
        }
    }
}
fixture!(OccupationSettingsRow, |key| {country: key as u32}, {garrison_template: Option<u64>, default_law_present: u8, law: Vec<u8>, occupied: Recorded<Option<Vec<u32>>>, group_laws: Recorded<Option<Vec<OccupationGroupLaw>>>, fallback_law: Recorded<Option<Vec<u8>>>});
fixture!(StateOccupationLawRow, |key| {controller: (key >> 64) as u32, occupied: (key >> 32) as u32, state: key as u32}, {name: Vec<u8>});
fixture!(StateGarrisonRow, |key| {controller: (key >> 32) as u32, state: key as u32}, {occupied: u32, required_manpower: i32, strength_ratio: i64, garrison_required: i64, serialized_flag: u8, equipment: Recorded<GarrisonEquipmentInputs>});
fixture!(StateAssignedManpowerRow, |key| {controller: (key >> 64) as u32, state: (key >> 32) as u32, ordinal: key as u32}, {occupied: u32, source_country: u32, amount: i32});
fixture!(CountryOccupationReserveManpowerRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {source_country: u32, amount: i32});
sample_struct!(OccupationAddedTarget {value: i64, controller: i32, occupied: i32, localisation: Vec<u8>});
sample_struct!(OccupationEffect {
    key: u32,
    value: i64
});
sample_struct!(OccupationEffectExtra {
    child_sources: u32,
    custom_tooltips: u32
});
sample_struct!(OccupationHover {exile_host: i32, legitimacy: i64, flow_from: u32, flow: i64, gain_ratio: i64, added: Vec<OccupationAddedTarget>, resistance_effects: Vec<OccupationEffect>, compliance_effects: Vec<OccupationEffect>,resistance_extra:OccupationEffectExtra,compliance_extra:OccupationEffectExtra});
impl Sample for OccupationModifierSource {
    fn sample(tick: usize) -> Self {
        Self {
            section: (tick % 6) as u8,
            source: (tick % 4) as u8,
            key: u32::sample(tick),
            total: Some(i64::sample(tick)),
            nodes: vec![ModifierBreakdownNode {
                children: 0,
                value: i64::sample(tick),
                name: b"fixture source".to_vec(),
            }],
            failure: Vec::new(),
        }
    }
}
sample_struct!(OccupationMap {stored_target: Option<i64>, core_country: Option<u32>, resistance: Vec<Vec<u8>>, compliance: Vec<Vec<u8>>});
fixture!(StateResistanceRow, |key| {state: key as u32}, {country: u32, level: i64, speed: i64, compliance: i64, compliance_speed: i64, active: u8, hover: Recorded<Option<OccupationHover>>, modifier_sources: Recorded<Vec<OccupationModifierSource>>, map: Recorded<Option<OccupationMap>>});
fixture!(CountryCollaborationRow, |key| {source: (key >> 32) as u32, target: key as u32}, {progress: i64});
fixture!(CharacterRow, |key| {id: key as u32}, {country: u32, name_token_id: i32, roster: u8, ordinal: u32, name: Vec<u8>, token: Vec<u8>, template_present: u8, template_tag: u32, template_file: Vec<u8>, template_line: i32, template_generated: u8, gender: u32});
fixture!(CountryLeaderRoleRow, |key| {character: (key >> 32) as u32, ideology: key as u32 as i32}, {country: u32, current_ideology: Recorded<Option<Option<u32>>>});
fixture!(AdvisorRoleRow, |key| {character: (key >> 32) as u32, ordinal: key as u32}, {country: u32, key: Vec<u8>, idea: Vec<u8>, slot: Vec<u8>, modifier_cache: Recorded<Option<AdvisorModifierCache>>, tooltip_keys: Recorded<Option<Vec<Vec<u8>>>>});
impl Fixture for UnitLeaderRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let codename = Option::<NameOperands>::sample(tick);
        Self {
            id: (key >> 32) as u32,
            reference_type: key as u32,
            character_type: u32::sample(tick),
            character_id: u32::sample(tick),
            category: i32::sample(tick),
            experience: i64::sample(tick),
            kind: if codename.is_some() {
                3
            } else {
                u8::sample(tick)
            },
            skill_status: u8::sample(tick),
            skill_valid: u8::sample(tick),
            skill_level: i32::sample(tick),
            skill_required_experience: i32::sample(tick),
            skill_category: i32::sample(tick),
            components: <[i32; 4]>::sample(tick),
            inherited_text_a: Vec::<u8>::sample(tick),
            inherited_text_b: Vec::<u8>::sample(tick),
            owner: u32::sample(tick),
            captured: u8::sample(tick),
            visibility_choice: u8::sample(tick),
            template_visibility: u8::sample(tick),
            exile_country: Recorded::Value(i32::sample(tick)),
            codename: Recorded::Value(codename),
            formation_reference: Recorded::Value(Option::<u64>::sample(tick)),
        }
    }
}
fixture!(UnitLeaderTraitRow, |key| {leader_reference: (key >> 32) as u64, ordinal: key as u32}, {token_id: i32, definition_ordinal: i32, token: Vec<u8>});
fixture!(UnitLeaderTraitExperienceRow, |key| {leader_reference: (key >> 33) as u64, list: ((key >> 32) & 1) as u8, ordinal: key as u32}, {token_id: i32, definition_ordinal: i32, experience: i64});
fixture!(ActiveAbilityRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {scope_country: u32, token: Vec<u8>, caster: Option<u64>, strategic_region: Option<u32>, division_count: i32, activation_hour: i32, phase: u8, allocate_cost: u8, allocated_command_power: i64, duration_hours: i32, cooldown_hours: i32, target_kind: i32});
fixture!(CapitulationRow, |key| {country: key as u32}, {capitulated: u8, capitulated_hour: i32, last_surrender_hour: RetiredI32, faction_join_hour: RetiredI32, legitimacy_raw: RetiredI64, exile_army_leaders: RetiredI32, war_goal_recipients: Recorded<Option<Vec<u32>>>});
fixture!(SubjectRow, |key| {overlord: (key >> 32) as u32, subject: key as u32}, {start_hour: i32, end_hour: i32, cancel: u8, autonomy_ratio: i64, state: Vec<u8>});
fixture!(AutonomyRow, |key| {subject: key as u32}, {score: i64, lower_score: i64, upper_score: i64, last_modified_bits: u32, state: Vec<u8>, previous: Option<Vec<u8>>, next: Option<Vec<u8>>});
fixture!(IntelligenceAgencyRow, |key| {country: key as u32}, {upgrade_progress_raw: i64, defense_raw: i64, active_count: i32, admission_ceiling: i32, unknown_f0: u32, name: Vec<u8>, created: u8});
fixture!(OperativeMembershipRow, |key| {id: (key >> 40) as u64, roster_country: (key >> 8) as u32, roster_kind: key as u8}, {owner_country: u32, legacy_description_length: u8, assignment: u32, experience_raw: i64, skill: Option<i32>, capturer_country: u32, captured_hour: i32, action_variant: Option<u32>, ordinal: Recorded<u32>, target_country: Recorded<Option<u32>>, operation: Recorded<Option<u64>>, name: Recorded<Vec<u8>>, cooldown_kind: Recorded<Option<u32>>, cooldown_start_hour: Recorded<Option<i32>>, cooldown_end_hour: Recorded<Option<i32>>});
sample_struct!(OperativeEffectModifier { key: u32, value: i64, sources: Option<[std::sync::Arc<Vec<ModifierBreakdownNode>>; 3]> });
sample_struct!(OperativeEffect { owned_states: i32, network_gain_scale: i32, trade_opinion: Option<i64>, modifiers: Vec<OperativeEffectModifier> });
fixture!(OperativeMissionRow, |key| {roster_country: (key >> 64) as u32, id: key as u64}, {ordinal: u32, owner_country: u32, assignment: u32, cooldown_kind: Option<u32>, cooldown_start_hour: Option<i32>, cooldown_end_hour: Option<i32>, variant: Option<u32>, target_country: Option<u32>, target_state: Option<i32>, ideology: Vec<u8>, pending_variant: Option<u32>, pending_country: Option<u32>, pending_state: Option<i32>, pending_ideology: Vec<u8>, portrait: Vec<u8>, root_out_country_factor: Option<i64>, root_out_operative_factor: Option<i64>, root_out_nationality: Option<u8>, map_selected: Recorded<Option<u8>>, map_warning: Recorded<Option<u8>>, active_effect: Recorded<Option<std::sync::Arc<OperativeEffect>>>, pending_effect: Recorded<Option<std::sync::Arc<OperativeEffect>>>});
sample_struct!(TradeInfluence {
    owner: u32,
    value: i64
});
sample_struct!(DiplomaticPressure {
    owner: u32,
    acceptance: i64,
    tension: i64
});
sample_struct!(AgencyBufferedOperation { operation: u64, state: Option<u32> });
sample_struct!(AgencyBufferedOffer { definition: u32, country: u32, state: Option<u32>, duration: Option<i32> });
fixture!(AgencyOperationBuffersRow, |key| {snapshot: key as u8}, {completed: Vec<AgencyBufferedOperation>, running: Vec<AgencyBufferedOperation>, ready: Vec<AgencyBufferedOperation>, unready: Vec<AgencyBufferedOperation>, offers: Recorded<Vec<AgencyBufferedOffer>>});
fixture!(StrategicOperativeRow, |key| {country: key as u32}, {slot: u32, mission_activity: i64, operation_activity: i64, trade_influence: Vec<TradeInfluence>, diplomatic_pressure: Vec<DiplomaticPressure>, mission_rosters: Recorded<Option<std::sync::Arc<[Vec<u64>; 5]>>>});
fixture!(CryptologyRow, |key| {owner_country: (key >> 32) as u32, target_country: key as u32}, {amount_raw: i64, bonus_days: i32, bonus_active: u8, decrypting: u8, ordinal: Recorded<u32>});
fixture!(IntelNetworkRow, |key| {owner_country: (key >> 64) as u32, target_country: (key >> 32) as u32, state_id: key as u32 as i32}, {strength_raw: Option<i64>, subnetwork_selector: i32, local_strength_raw: i64, gain_raw: i64, gain_from_operatives_raw: i64, gain_from_adjacency_raw: i64, local_quiet: u8, gain_factor_raw: Option<i64>, strength_target_raw: Option<i64>, target_from_operatives_raw: Option<i64>, target_from_counterintelligence_raw: Option<i64>, subnetwork_quiet: u8});
fixture!(IntelNetworkSummaryRow, |key| { owner_country: (key >> 32) as u32, target_country: key as u32 }, {
    ordinal: u32,
    covered_core_states: i32, covered_controlled_states: i32, covered_worth: i32,
    controlled_strength_sum: i64, core_strength_sum: i64, max_state_strength: i64
});
fixture!(IntelNetworkAuxiliaryRow, |key| {owner:(key>>96) as u32,target:(key>>64) as u32,id:key as u64}, {kind:u8,subnetwork:u32,ordinal:u64,country:u32,count_a:i32,count_b:i32,count_c:i32,first:i64,second:i64,third:i64});
fixture!(IntelContributionRow, |key| { owner: (key >> 72) as u32, kind: (key >> 64) as u8, pool: (key >> 32) as u32, ordinal: key as u32 }, { target: i32, civilian:i64, army:i64, navy:i64, air:i64 });
fixture!(IntelSourceRow, |key| { owner:(key>>96) as u32, pool:(key>>64) as u32, source:(key>>32) as u32, ordinal:key as u32 }, { target:i32,civilian:i64,army:i64,navy:i64,air:i64,id:i32,removed:u8,next_id:i32,discriminant:u32,count:u32,other_count:u32,reference_country:u32,reference_pool:i32 });
fixture!(PropagandaEffortRow, |key| { slot: (key >> 64) as u32, sequence: key as u64, derived_drift: 0 }, {stability: i64, war_support: i64, country: u32});
fixture!(ScriptFlagRow, |key| {scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, token: key as u32}, {set_hour: i32, value: i16, remaining_days: i16});
impl Fixture for ReserveModifierBreakdownRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            kind: (key >> 64) as u8,
            owner: key as u64,
            nodes: vec![
                ModifierBreakdownNode {
                    children: 2,
                    value: tick as i64,
                    name: Vec::new(),
                },
                ModifierBreakdownNode {
                    children: 0,
                    value: i64::MIN + tick as i64,
                    name: vec![0, 255, tick as u8],
                },
                ModifierBreakdownNode {
                    children: 0,
                    value: i64::MAX - tick as i64,
                    name: b"source".to_vec(),
                },
            ],
        }
    }
    fn largest_key() -> u128 {
        (2_u128 << 64) | u64::MAX as u128
    }
}
impl Fixture for ReserveModifierTreeRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let kind = (key >> 64) as u8;
        Self {
            kind,
            owner: key as u64,
            name: if kind == 0 || tick.is_multiple_of(3) {
                Vec::new()
            } else {
                vec![b'L', 0xff, tick as u8]
            },
            nodes: (!tick.is_multiple_of(3))
                .then(|| ReserveModifierBreakdownRow::fixture(1, tick).nodes),
        }
    }
    fn largest_key() -> u128 {
        (2_u128 << 64) | u64::MAX as u128
    }
}
impl Fixture for PowerBalanceModifierBreakdownRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let nodes = ReserveModifierBreakdownRow::fixture((1_u128 << 64) | 1, tick).nodes;
        let source = format!("source_{tick}").into_bytes();
        Self {
            record: key as u32,
            sources: vec![source.clone(), source],
            daily: nodes.clone(),
            weekly: nodes,
        }
    }
    fn largest_key() -> u128 {
        u32::MAX as u128
    }
}
impl Fixture for NavalModifierBreakdownRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            accumulator: (key >> 32) as u64,
            key: key as u32,
            uses: vec![NavalModifierUse {
                combat: tick as u64 + 1,
                side: (tick % 2) as u8,
                battle_slot: if tick.is_multiple_of(2) {
                    u32::MAX
                } else {
                    tick as u32
                },
                source_scope: tick as u8,
                source_index: tick as u32,
                order: tick as u32,
            }],
            nodes: ReserveModifierBreakdownRow::fixture(1, tick).nodes,
        }
    }
    fn largest_key() -> u128 {
        (u64::MAX as u128) << 32 | u32::MAX as u128
    }
}
fn naval_key_trees(tick: usize) -> Vec<NavalModifierKeyTree> {
    let nodes = ReserveModifierBreakdownRow::fixture(1, tick).nodes;
    let mut trees = vec![NavalModifierKeyTree {
        key: 0x24,
        nodes: nodes.clone(),
    }];
    if !tick.is_multiple_of(3) {
        trees.push(NavalModifierKeyTree { key: 0x1b2, nodes });
    }
    trees
}
impl Fixture for NavalModifierTreeRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            kind: ((key >> 96) as u8) % 18,
            owner: (key >> 32) as u64,
            sub: key as u32,
            trees: naval_key_trees(tick),
        }
    }
    fn largest_key() -> u128 {
        (17_u128 << 96) | (u64::MAX as u128) << 32 | u32::MAX as u128
    }
}
impl Fixture for NavalModifierTreeSampleRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            combat: (key >> 56) as u64,
            side: ((key >> 48) as u8) & 1,
            battle_slot: (key >> 16) as u32,
            ordinal: key as u16,
            scope: tick as u8,
            kind: (tick % 18) as u8,
            owner: tick as u64,
            sub: u32::MAX - tick as u32,
            trees: naval_key_trees(tick),
        }
    }
    fn largest_key() -> u128 {
        (u64::MAX as u128) << 56 | (1_u128 << 48) | (u32::MAX as u128) << 16 | u16::MAX as u128
    }
}
impl Fixture for AttritionModifierBreakdownRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            accumulator: (key >> 32) as u64,
            key: key as u32,
            nodes: (!tick.is_multiple_of(4))
                .then(|| ReserveModifierBreakdownRow::fixture(1, tick).nodes),
        }
    }
    fn largest_key() -> u128 {
        (u64::MAX as u128) << 32 | u32::MAX as u128
    }
}
impl Fixture for GlobalSavedEventTargetRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            ordinal: key as u32,
            name: format!("target_{tick}").into_bytes(),
            state: tick as i32,
            country: -(tick as i32),
            character: tick as u32,
            live: Self::UNSAVED_NAME | (tick % 2048) as u16,
            character_leader: tick.is_multiple_of(2).then_some(tick as u64),
        }
    }
    fn largest_key() -> u128 {
        u128::from(u32::MAX)
    }
}
fixture!(ScriptVariablesRow, |key| {scope_kind: (key >> 32) as u8, scope_id: key as u32}, {entries: Vec<ScriptVariableEntry>, seed: Recorded<Option<u32>>});
fixture!(ScriptVariableNameRow, |key| {id: key as u32}, {name: Vec<u8>});
fixture!(ScriptVariableValueRow, |key| {scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, var_id: key as u32}, {value_raw: i64});
sample_struct!(OperationRequirement {
    amount: i64,
    days: i32,
    archetype: u32,
    equipment: u8,
    uses_time: u8
});
sample_struct!(OperationEquipment {
    variant: u64,
    archetype: u32,
    amount: i64
});
sample_struct!(OperationEquipmentTotal {
    archetype: u32,
    amount: i64
});
fixture!(OperationRow, |key| {id: key as u64}, {owner: u32, target: u32, target_province: Option<u32>, definition_token: u32, preparation_current: i64, preparation_required: i64, flag_40: u8, auto_repeat: u8, terminal: u8, start_hour: i32, duration_days: i32, phase_count: u32, assigned_slots: u32, bonus: Recorded<u8>, detected: Recorded<u8>, effect_description: Recorded<Vec<u8>>, phases: Vec<u32>, requirements: Vec<OperationRequirement>, equipment: Vec<OperationEquipment>, equipment_totals: Vec<OperationEquipmentTotal>, equipment_allow_zero: u8});
fixture!(OperationAssignmentRow, |key| {operation: (key >> 32) as u64, ordinal: key as u32}, {operative: u64, selection: u32});
fixture!(WingEquipmentRow, |key| {wing: (key >> 32) as u64, ordinal: key as u32}, {variant: u64, amount_raw: i64});
sample_struct!(AirBaseCountryOccurrence {
    country: u32,
    slot: u32
});
sample_struct!(AirBaseCarrierInputs { registered_countries: Option<Vec<AirBaseCountryOccurrence>>, host_name: Option<Vec<u8>> });
sample_struct!(CarrierAirPool { type_index: i32, subunit_index: u32, wing_start: u32, wing_count: u32, current_fuel_raw: Option<i64>, maximum_fuel_raw: Option<i64> });
sample_struct!(AirBaseRegistrationCarrierInputs { country: u32, fuel_demand_raw: Option<i64>, fuel_allocated_raw: Option<i64>, pools: Option<Vec<CarrierAirPool>> });
fixture!(AirBaseRow, |key| {id: key as u64}, {state: Option<u32>, province: Option<u32>, host: u64, capacity: i32, kind: i32, carrier: Recorded<Option<AirBaseCarrierInputs>>});
fixture!(WingCumulativeRow, |key| {id: key as u64}, {accidents: i32, experience: i64});
fixture!(AirRegionHistoryRow, |key| {country: (key >> 32) as u32, region: key as u32}, {entries: Vec<AirHistoryEntry>});
fixture!(AirRegionDamageRow, |key| {country: (key >> 32) as u32, region: key as u32}, {entries: Vec<AirDamageEntry>});
fixture!(AirRegionCombatRow, |key| {country: (key >> 32) as u32, region: key as u32}, {owner: u32, own: std::sync::Arc<Vec<AirCombatRecord>>, foreign: std::sync::Arc<Vec<AirCombatRecord>>});
sample_struct!(AirAcePortraitInputs {cached: Option<Vec<u8>>, culture: Option<Vec<u8>>, fallback: Option<Vec<u8>>});
sample_struct!(AirAcePresentation {gender: u8, pool_index: i32, definition: Option<Vec<u8>>, portrait: Option<AirAcePortraitInputs>});
fixture!(AirAceRow, |key| {id: key as u64}, {country: u32, ordinal: u32, aircraft_type_mask: u32, assigned_wing: u64, alive: u8, handled: u8, kill_type: u32, death: Option<AceDeath>, name: Vec<u8>, surname: Vec<u8>, callsign: Vec<u8>, presentation: Recorded<Option<AirAcePresentation>>});
fixture!(AirStatisticsRow, |key| {country: (key >> 32) as u32, region: key as u32}, {year_slot: u32, year_window_wrapped: u8, channels: [i64;24]});
fixture!(SunkConvoyRow, |key| {month: (key >> 64) as u32 as i32, killer_country: (key >> 32) as u32, owner_country: key as u32}, {convoys: i32, ordinal: u32});
fixture!(OwnerTransportSpeedRow, |key| {owner: (key >> 32) as u32, region: key as u32}, {speed: Option<i64>});
sample_struct!(StateMovementTerms { army_speed: Option<i64>, attack_speed: Option<i64>, relation_speed: Option<i64>, branch: u8 });
fixture!(MovementStateTermRow, |key| {state: (key >> 64) as u32, roster: (key >> 32) as u32, owner: key as u32}, {terms: Option<StateMovementTerms>});
fixture!(CoreSupplyAccessRow, |key| {owner: (key >> 32) as u32, province: key as u32}, {access: Option<bool>});
fixture!(CountryUnitSpeedEntryRow, |key| {country: (key >> 32) as u32, key: key as u32}, {entry: Option<Option<i64>>});
fixture!(RiverRelationRow, |key| {owner: (key >> 32) as u32, controller: key as u32}, {entry: Option<Option<i64>>});
fixture!(RailCooldownRow, |key| {province: key as u32}, {cooldown_ticks: i32, building: Recorded<Option<RailwayBuildingState>>, selection: Recorded<Option<RailwaySelectionInputs>>, selection_hour: Recorded<Option<i32>>, building_bound: Recorded<Option<bool>>});
fixture!(RailEdgeRow, |key| {from: (key >> 32) as u32, to: key as u32}, {level: i32});
fixture!(LandTheatreRow, |key| {id: key as u64}, {owner: u32, ordinal: u32, volunteers_theatre: i32});
fixture!(LandTheatreGroupRow, |key| {id: key as u64}, {theatre: u64, ordinal: u32, priority: i32, name: Vec<u8>});
fixture!(LandTheatreMemberRow, |key| {theater_group: (key >> 64) as u64, orders_group: key as u64}, {ordinal: u32});
fixture!(AirTheatreRow, |key| {id: key as u64}, {owner: u32, ordinal: u32, name: Vec<u8>});
fixture!(AirTheatreGroupRow, |key| {id: key as u64}, {theatre: u64, ordinal: u32, name: Vec<u8>});
fixture!(AirTheatreMemberRow, |key| {group: (key >> 64) as u64, wing: key as u64}, {ordinal: u32});
fixture!(NavalResultRow, |key| {id: key as u64}, {province: u32, result_hour: i32, retire_hour: i32, category: u32, flag_a: u8, flag_b: u8, subtype: u8, readiness: u8, winning_side: u8, losing_side: u8, province_ordinal: u32});
impl Fixture for NavalResultSourceRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let game_hour = i32::sample(tick);
        let transient = bool::sample(tick);
        Self {
            id: key as u64,
            game_hour,
            result: u64::sample(tick + 1),
            combat: u64::sample(tick + 2),
            merged: bool::sample(tick + 1),
            transient,
            combat_created_hour: if transient {
                Some(game_hour)
            } else {
                Option::<i32>::sample(tick)
            },
        }
    }
}

fixture!(NavalResultMemberRow, |key| {result: (key >> 64) as u64, member: key as u64}, {presence_value: u64, ordinal: u32, side: u8});
fixture!(NavalResultCountryRow, |key| {result: (key >> 40) as u64, list: (key >> 32) as u8, country: key as u32}, {ordinal: u32});
fixture!(SunkShipsRow, |_key| {}, {entries: Vec<SunkShipEntry>});
sample_struct!(RaidCounterOperative {
    handle: u64,
    defense: i64,
    context: i64
});
fixture!(RaidIntelInputRow, |key| {country: key as u32}, {strategic_present: u8, country_defense: i64, crypto_strength: Option<i64>, operatives: Vec<RaidCounterOperative>, defense_tree: Recorded<Option<std::sync::Arc<Vec<ModifierBreakdownNode>>>>});
sample_struct!(AirPowerContribution {
    country: u32,
    amount: i64
});
sample_struct!(AirPowerMenu { coverage: i64, reductions: [i64; 4], contributions: [Vec<AirPowerContribution>; 2] });
fixture!(RaidAirScoreRow, |key| {country: (key >> 32) as u32, region: key as u32}, {friendly: i64, enemy: i64, impact: Recorded<i64>, friendly_missions: u16, enemy_missions: u16, air_spotting_scale: Option<i64>, menu: Recorded<Option<std::sync::Arc<AirPowerMenu>>>});

#[test]
fn air_map_mission_counters_change_independently_of_the_superiority_weights() {
    let previous = RaidAirScoreRow {
        country: 5000,
        region: 9000,
        friendly: 0,
        enemy: 0,
        impact: Recorded::Value(0),
        friendly_missions: 0,
        enemy_missions: 0,
        air_spotting_scale: Some(0),
        menu: Recorded::Value(None),
    };
    let next = RaidAirScoreRow {
        friendly_missions: u16::MAX,
        enemy_missions: 1,
        ..previous.clone()
    };
    round_trip(&next);
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 3) | (1 << 4));
    let mut bytes = Vec::new();
    encode_delta(TableId::RaidAirScores, &delta, 0, &mut bytes).unwrap();
    let decoded = decode_delta(TableId::RaidAirScores, 0, &mut Decoder::new(&bytes)).unwrap();
    assert_eq!(decoded.apply(Some(previous)).unwrap(), next);
}

#[test]
fn regional_air_spotting_scale_has_an_independent_numeric_delta_and_unknown_state() {
    let previous = RaidAirScoreRow {
        country: 5000,
        region: 9000,
        air_spotting_scale: Some(0),
        ..Default::default()
    };
    let next = RaidAirScoreRow {
        air_spotting_scale: Some(-7),
        ..previous.clone()
    };
    round_trip(&next);
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 5);
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
    let unknown = RaidAirScoreRow {
        air_spotting_scale: None,
        ..next.clone()
    };
    round_trip(&unknown);
    let delta = unknown.delta(Some(&next)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 5);
    assert_eq!(delta.apply(Some(next)).unwrap(), unknown);
}
fixture!(RaidTooltipOperandRow, |key| {raid: (key >> 32) as u64, ordinal: key as u32}, {kind: u32, key: u32, raw: i64});

impl Fixture for CharacterRoleTraitRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            character: (key >> 40) as u32,
            kind: (key >> 32) as u8,
            role: key as u32,
            tokens: vec![u32::sample(tick), 7],
            definitions: if (key >> 32) as u8 == Self::SCIENTIST {
                Vec::new()
            } else {
                vec![u32::sample(tick + 1), 3]
            },
        }
    }
    fn largest_key() -> u128 {
        (u128::from(u32::MAX) << 40) | (u128::from(Self::SCIENTIST) << 32)
    }
}
impl Fixture for CharacterOperativeRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let resolved = tick % 2 == 1;
        Self {
            character: key as u32,
            reference: u64::sample(tick).max(1),
            resolved,
            category: if resolved { i32::sample(tick) } else { 0 },
            owner: if resolved { u32::sample(tick) } else { 0 },
            skill_level: if resolved {
                Option::<i32>::sample(tick)
            } else {
                None
            },
            codename: Recorded::Value(if resolved {
                Option::<NameOperands>::sample(tick)
            } else {
                None
            }),
            name: Recorded::Value(if resolved {
                Some(Vec::<u8>::sample(tick))
            } else {
                None
            }),
            trait_definitions: if resolved {
                Vec::<u32>::sample(tick)
            } else {
                Vec::new()
            },
        }
    }
}
fixture!(RaidFacilityRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {id: u32, owner: u32, province: u32, active_project: Option<u32>, project_definition: Option<u32>});
sample_struct!(ProjectFacilityOperand {
    kind: u32,
    scientist: u64,
    token: u32,
    key: u32,
    ordinal: u32,
    assignment: u32,
    raw: i64
});
sample_struct!(ProjectScientistAssignment {
    scientist: u64,
    target: i32,
    current: i32,
    timed: u8
});
sample_struct!(ProjectVariable { name: Vec<u8>, value_raw: i64 });
sample_struct!(ProjectVariableStore { seed: u32, entries: Vec<ProjectVariable> });
impl Sample for ProjectRuntime {
    fn sample(tick: usize) -> Self {
        Self {
            pending_known: true,
            pending_reward: Option::<u32>::sample(tick),
            pending_ordinal: Option::<u32>::sample(tick),
            variables: Option::<ProjectVariableStore>::sample(tick).map(Into::into),
        }
    }
}
fixture!(ProjectProgramRow, |key| {id: key as u32}, {owner: u32, province: u32, active_project: Option<u32>, scientist: u64, unread_rewards: i32, building: Vec<u8>, assignments: Recorded<Option<Vec<ProjectScientistAssignment>>>, facility: Recorded<Option<Vec<ProjectFacilityOperand>>>});
fixture!(ProjectRow, |key| {owner: (key >> 32) as u32, local_id: key as u32}, {definition_id: u32, program_reference: u64, started: u8, completed: u8, state: ProjectState, runtime: Recorded<ProjectRuntime>});
fixture!(ProjectHistoryRow, |key| {owner: (key >> 64) as u32, project: (key >> 32) as u32, ordinal: key as u32}, {prototype_reward_handle: u32, option_handle: u32, date: i32, context: ProjectContext});
fixture!(ProjectBreakthroughRow, |key| {owner: (key >> 32) as u32, specialization: key as u32}, {points: i32});
fixture!(NukeRow, |key| {country: (key >> 32) as u32, nuke_type: key as u32}, {amount_raw: i64, ready: i32});
fixture!(RaidCountryRow, |key| {canonical_slot: key as u32}, {country: i32, eligible_types: Vec<u32>, used_types: Vec<u32>});
fixture!(RaidRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {parent_slot: u32, ordinal: u32, phase: i32, outcome: i32, source: RaidSource, show_for: Vec<u32>, unit: u64, air_wing: u64, travelled: Option<i64>, preparation_hours: i32, end_hour: Option<i32>, completion_country: Option<u32>, completion_recorded: u8});
fixture!(RaidTargetRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {scope: RaidScope, raid_type: u32, detected: u8});
fixture!(NavalBaseRow, |key| {province: key as u32}, {country: u32, level: i32, healthy_level: i32, countries: Vec<u32>, countries_unreadable: bool, country_tags: Recorded<Option<Vec<Vec<u8>>>>});
fixture!(BattleSoundRow, |key| {ordinal: key as u32}, {center_x: u32, center_z: u32, radius: u32, provinces: Vec<u32>, sounding: bool, object: u64});
fixture!(PeaceConferenceRow, |key| {id: key as u64}, {ordinal: u32, current_turn: i32, finalized: u8});
fixture!(WeatherRegionRow, |key| {id: key as u32}, {max_snow: i64, light_rain: u8, heavy_rain: u8, snow: u8, blizzard: u8, sandstorm: u8, arctic_water: u8, attack: Recorded<Witnessed<WeatherRegionAttackInputs>>, combat: Recorded<Option<RegionCombatWeather>>, air_detection: Recorded<Option<Option<i64>>>, map_modifiers_known: Recorded<bool>, naval_detection: Recorded<Option<i64>>, air_attack: Recorded<Option<i64>>, air_defence: Recorded<Option<i64>>, air_efficiency: Recorded<Option<std::sync::Arc<Vec<ModifierBreakdownNode>>>>, custom_sea: Recorded<Option<Vec<i16>>>});
impl Fixture for WeatherProvinceRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let id = key as u32;
        Self {
            id,
            snow: i64::sample(tick),
            permanent_snow: i64::sample(tick),
            display_snow: u8::sample(tick),
            display_mud: u8::sample(tick),
            temperature: ProvinceTemperature::sample(tick),
            raw_mud: Recorded::<u8>::sample(tick),
            clock: if id == 0 {
                DayNightClock::held(crate::day_night::Clock {
                    local_hour: i32::sample(tick),
                    night_factor: i64::sample(tick),
                })
            } else {
                DayNightClock::sample(tick)
            },
            display: Recorded::<Option<WeatherDisplay>>::sample(tick),
            temperature_pass: Recorded::Value((id == 0).then(|| TemperaturePass::sample(tick))),
            modifiers: Recorded::<Option<ProvinceWeatherModifiers>>::sample(tick),
            daylight: Recorded::Value((id == 0).then(|| WeatherDaylight::sample(tick))),
            static_modifiers: Recorded::<Option<ProvinceStaticModifiers>>::sample(tick),
        }
    }
}
impl Sample for ProvinceTemperature {
    fn sample(tick: usize) -> Self {
        Self::anchored(crate::temperature::State {
            temperature: i64::sample(tick),
            drift: i64::sample(tick + 1),
            staged: i64::sample(tick + 2),
            working: i64::sample(tick + 3),
        })
    }
}
sample_struct!(TemperaturePass { anchor: i32, cursor: i32, anchor_seed: Option<u32>, hours: Vec<Option<u32>>, witness: TemperatureWitness });
impl Sample for TemperatureWitness {
    fn sample(tick: usize) -> Self {
        Self::new(u64::sample(tick))
    }
}
sample_struct!(WeatherDaylight { updated: bool, held: Option<[i64; 3]>, held_feather: Option<[i64; 2]> });
fixture!(OperationTokensRow, |key| {owner: (key >> 32) as u32, target: key as u32}, {tokens: Vec<u32>});
fixture!(LexicalTokenRow, |key| {token: key as u32}, {literal: Vec<u8>});
fixture!(OwnedWargoalRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {recipient: u32, goal_type: i32, states: Vec<u32>});
fixture!(AvailableAdvisorRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {character: u32, idea: Vec<u8>});

fixture!(TimedIdeaRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>, days: i32});
fixture!(DecisionRandomItemRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>, count: i32, target: u32});
fixture!(CapturedOperativeReferenceRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {operative: u64, owner: Option<u32>});
sample_struct!(TransportTaskForceStats { detection_138: Option<i64>, visibility_148: Option<i64>, visibility_150: Option<i64>, carrier_detection_160: Option<i64> });
sample_struct!(TransportRadarCounts { signed_count: i32, counts: Option<Vec<i32>> });
impl Sample for SpottingSample {
    fn sample(_tick: usize) -> Self {
        Self::Derived
    }
}
impl Sample for DayNightClock {
    fn sample(tick: usize) -> Self {
        if tick.is_multiple_of(3) {
            Self::held(crate::day_night::Clock {
                local_hour: i32::sample(tick),
                night_factor: i64::sample(tick + 1),
            })
        } else {
            Self::UPDATED
        }
    }
}
sample_struct!(TransportSpottingRegion {
    region_id: u32, runtime_slot: Option<i32>, signed_province_count: Option<i32>,
    sample: SpottingSample, weather_detection: Option<Option<i64>>,
    air_environment: Option<TransportAirEnvironment>
});
sample_struct!(TransportAirMission {
    wing: u64, current_type: Option<u32>, aircraft: Option<i32>, disruption: Option<i64>,
    detection_13: Option<i64>, detection_14: Option<i64>, normalization: Option<TransportAirNormalization>
});
sample_struct!(TransportAirModifiers { direct: [Option<i64>; 2], positive: [Option<u32>; 2], effective: [Option<i64>; 16] });
sample_struct!(TransportAirCombatModifier { lane: u8, raw: Option<i64> });
sample_struct!(TransportAirAce {
    assigned_aircraft: Option<Option<i32>>, modifiers: Option<TransportAirModifiers>,
    type_modifiers: Option<TransportAirModifiers>, owner_effectiveness: Option<i64>,
    combat: Option<Vec<TransportAirCombatModifier>>
});
sample_struct!(TransportAirEnvironment {
    weather: [Option<i64>; 2], night_factor: Option<i64>, night: [Option<i64>; 2], target_region: Option<i32>
});
sample_struct!(TransportAirNormalization {
    attack: Option<i64>, defence: Option<i64>, speed: Option<i64>, enabled_missions: u32,
    wing: Option<TransportAirModifiers>, country: Option<TransportAirModifiers>,
    ace: Option<Option<TransportAirAce>>, environment: Option<Option<TransportAirEnvironment>>,
    night_stat: Option<i64>, base_region: Option<i32>, carrier_mission: Option<Option<i32>>
});
sample_struct!(TransportRiskShip {
    ship: Option<u64>, class: Option<u32>, formation_flags: Option<u64>,
    positive_equipment_flags: Option<u64>, carrier_air_base: Option<Option<u64>>
});
sample_struct!(TransportReceiverShips { task_force: u64, count: Option<i32>, ships: Option<Vec<TransportRiskShip>> });
sample_struct!(TransportAirRegion { region: u32, count: Option<i32>, missions: Option<Vec<Option<TransportAirMission>>> });
sample_struct!(TransportReceiverRisk {
    coordination: Option<i64>, spotting_chance: Option<i64>, night_spotting_chance: Option<i64>, visibility: Option<i64>,
    speed_base: Option<i64>, speed_modifier: Option<i64>, terrain_class: Option<i32>, stats: Option<TransportTaskForceStats>,
    roster_owner: Option<u32>, location: Option<u32>, region: Option<u32>, location_controller: Option<u32>,
    detached_activity: Option<i32>, ship_count: Option<i32>,
    targeted_spotting_modifier: Option<i64>, actual_terrain_speed: Option<i64>
});
impl Fixture for ReservedCountryNamingRow {
    fn fixture(_key: u128, tick: usize) -> Self {
        Self {
            tag: Vec::<u8>::sample(tick),
            name_tag: Vec::<u8>::sample(tick + 1),
            cosmetic_tag: Vec::<u8>::sample(tick + 2),
            leader_ideology: Vec::<u8>::sample(tick + 3),
            group: Option::<Vec<u8>>::sample(tick + 4),
            overlord: i32::sample(tick),
            autonomy: Option::<Vec<u8>>::sample(tick + 5),
            built: (tick & 7) as u8,
            original_tag: Recorded::<Option<Vec<u8>>>::sample(tick + 6),
            original_tag_reference: Recorded::<Option<i32>>::sample(tick + 7),
            original_tag_buckets: Recorded::<Option<Vec<Vec<u32>>>>::sample(tick + 8),
            country_array: Recorded::<Option<Vec<u32>>>::sample(tick + 9),
            owned_state_count: Recorded::<Option<i32>>::sample(tick + 10),
            dynamic_tag_boundary: Recorded::<Option<i32>>::sample(tick + 11),
            faction: Recorded::<Option<u64>>::sample(tick + 11),
            movement: Recorded::Value(Retired),
            land_theatre_count: Recorded::<Option<u32>>::sample(tick + 13),
            launch_sources: Recorded::<Option<ReservedLaunchSources>>::sample(tick + 14),
            reverse_visibility: Recorded::<Option<ReservedReverseVisibilityInputs>>::sample(
                tick + 15,
            ),
            country_intel_sample: Recorded::<Option<ReservedCountryIntelSample>>::sample(tick + 16),
            attack: Recorded::<Witnessed<Option<CountryAttackInputs>>>::sample(tick + 17),
            raw_country_observations: Recorded::Value(Retired),
            transport_mission_inputs: Recorded::Value(Retired),
        }
    }
}

fixture!(ReservedNearestNavyRow, |key| {slot: key as u32}, {present: bool, provinces: Option<u32>});
fixture!(ReservedNearestBaseRow, |key| {slot: (key >> 32) as u32, province: key as u32}, {base: u32});
fixture!(TransportNavyRow, |key| {ordinal: key as u32}, {slot: u32, present: bool, region_slots: Option<u32>, ledgers: bool, access_modes: Option<Vec<u8>>});
fixture!(TransportLedgerRow, |key| {navy: (key >> 32) as u32, region: key as u32}, {occurrences: Option<Vec<u64>>});
impl Fixture for TransportReceiverRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            receiver: TransportMissionReceiver {
                task_force: key as u64,
                mission_owner: Option::<u32>::sample(tick + 6),
                logical_owner: Option::<u32>::sample(tick),
                in_regions: Option::<u8>::sample(tick + 1),
                combat_references: Option::<i32>::sample(tick + 2),
                combined_detection: Option::<i64>::sample(tick + 3),
                detection_cache: Option::<[i64; 2]>::sample(tick + 4),
                received_supply_ratio: Option::<i64>::sample(tick + 5),
                risk: Option::<TransportReceiverRisk>::sample(tick + 7),
            },
        }
    }
}
impl Fixture for TransportSearchRegionRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            ordinal: key as u32,
            region: (!tick.is_multiple_of(3)).then(|| TransportNavalRegion {
                position: [i32::sample(tick), i32::sample(tick + 1)],
                edges: vec![TransportNavalEdge {
                    to: u32::sample(tick),
                    flags: u8::sample(tick),
                    unrestricted: u8::sample(tick + 1),
                    rules: vec![None, Some(0), Some(u32::sample(tick))],
                }],
            }),
        }
    }
}
impl Sample for SpottingLanes {
    fn sample(tick: usize) -> Self {
        if tick.is_multiple_of(3) {
            Self::CountryIntel
        } else {
            Self::Copied(Vec::<i64>::sample(tick))
        }
    }
}
sample_struct!(SpottingNavyIntel { owner_raw_id: Option<u32>, owner_slot: Option<u32>, signed_count: Option<i32>, lanes: Option<SpottingLanes> });
fixture!(TransportSpottingOwnerRow, |key| {owner_slot: (key >> 32) as u32, occurrence: key as u32}, {owner_raw_id: u32, target_owned_states: Option<i32>, radar: Option<TransportRadarCounts>, navy_intel: Option<SpottingNavyIntel>});
fixture!(TransportSpottingRegionRow, |key| {ordinal: key as u32}, {region: TransportSpottingRegion});
fixture!(TransportAirRootRow, |key| {ordinal: key as u32}, {country: Option<u32>, slot: u32, present: bool, regions: Option<u32>, factors: Option<Vec<Option<i64>>>});
fixture!(TransportAirRegionRow, |key| {root: (key >> 32) as u32, ordinal: key as u32}, {region: TransportAirRegion});
impl Fixture for TransportRiskShipsRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            receiver: TransportReceiverShips {
                task_force: key as u64,
                count: Option::<i32>::sample(tick),
                ships: Option::<Vec<TransportRiskShip>>::sample(tick + 1),
            },
        }
    }
}

fixture!(HiredAdvisorRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {character: u32, idea: Vec<u8>, slot: Vec<u8>});
fixture!(NavalSupremacyRow, |key| {region: (key >> 32) as u32, country: key as u32}, {region_slot: u32, current: i64, base_target: i64, target: i64, individual_ratio: i64, previous: i64, decline_from: i64});
fixture!(RadarCoverageRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {state: Option<u32>, provinces: Vec<u32>, radius: Option<i64>});

#[test]
fn radar_range_changes_do_not_rewrite_the_coverage_membership() {
    let previous = RadarCoverageRow {
        country: 5000,
        ordinal: 0,
        state: Some(7000),
        radius: Some(200_000),
        provinces: vec![11, 19],
    };
    let next = RadarCoverageRow {
        radius: Some(i64::MIN),
        ..previous.clone()
    };
    round_trip(&next);
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    let mut bytes = Vec::new();
    encode_delta(TableId::RadarCoverage, &delta, 0, &mut bytes).unwrap();
    let decoded = decode_delta(TableId::RadarCoverage, 0, &mut Decoder::new(&bytes)).unwrap();
    assert_eq!(decoded.apply(Some(previous)).unwrap(), next);
}
fixture!(SentVolunteerRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {division: u64});
impl Fixture for LogisticsEquipmentRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 32) as u32,
            archetype: key as u32,
            token: u32::sample(tick),
            balance_raw: i64::sample(tick),
            stock_raw: i64::sample(tick),
            available_raw: Recorded::Value(Option::<i64>::sample(tick).map(|value| value.max(0))),
        }
    }
}
fixture!(IndustrialOrganisationRow, |key| {organisation: key as u64}, {country: u32, ordinal: u32, owner: u32, token: u32, name: Vec<u8>, icon: Vec<u8>});
fixture!(IndustrialOrganisationHistoryRow, |key| {organisation: (key >> 64) as u64, variant: key as u64}, {units: i32, design_hour: Recorded<Option<i32>>});
impl Fixture for IndustrialOrganisationPoliciesRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: key as u32,
            policies: vec![IndustrialOrganisationPolicy {
                token: u32::sample(tick),
                cost: i32::sample(tick),
                days: i32::sample(tick),
                country: u32::sample(tick + 1),
                name: Vec::<u8>::sample(tick + 2),
                cached_score_raw: i64::sample(tick + 3),
            }],
            cost_modifier_raw: Recorded::Value(Option::<i64>::sample(tick + 4)),
        }
    }
}
impl Fixture for IndustrialOrganisationDetailsRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            organisation: key as u64,
            country: u32::sample(tick),
            state: IndustrialOrganisationState {
                funds_raw: i64::sample(tick),
                size: i32::sample(tick),
                cooldown_hour: Some(i32::sample(tick)),
                ..Default::default()
            },
            unlocked: vec![u32::sample(tick)],
            queued: vec![u32::sample(tick + 1), u32::sample(tick + 1)],
            allowed_policies: vec![u32::sample(tick + 2)],
            tasks: vec![IndustrialOrganisationTask {
                technology: Some(IndustrialOrganisationTechnology {
                    country: u32::sample(tick),
                    handle: u32::sample(tick),
                    index: i32::sample(tick),
                    token: u32::sample(tick),
                }),
                line: Some(u64::sample(tick)),
            }],
            variables: IndustrialOrganisationVariables {
                seed: Some(u32::sample(tick)),
                entries: vec![IndustrialOrganisationVariable {
                    name: vec![0xff, 0, tick as u8],
                    value_raw: i64::sample(tick),
                }],
            },
            flags: vec![game_model::script_flag::Row {
                token: u32::sample(tick),
                set_hour: i32::sample(tick),
                value: i16::sample(tick),
                remaining_days: i16::sample(tick + 1),
            }],
        }
    }
}
impl Fixture for ConstructionLineOutputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let railway = bool::sample(tick);
        let ended = bool::sample(tick);
        Self {
            line: key as u64,
            railway: <bool as Sample>::sample(tick),
            construction_raw: <i64 as Sample>::sample(tick),
            repair_raw: <i64 as Sample>::sample(tick),
            bank_raw: <i64 as Sample>::sample(tick),
            instant_raw: <i64 as Sample>::sample(tick),
            carry_raw: <i64 as Sample>::sample(tick),
            ticks: <i32 as Sample>::sample(tick),
            since_hour: <i32 as Sample>::sample(tick),
            observed_since_creation: <bool as Sample>::sample(tick),
            linked: <bool as Sample>::sample(tick),
            ended: <bool as Sample>::sample(tick),
            exact: <bool as Sample>::sample(tick),
            repair_ticks: <i32 as Sample>::sample(tick),
            completion: Recorded::Value(ended.then_some(ConstructionCompletion {
                hour: i32::sample(tick),
                cancelled: railway,
            })),
            count_reset: Recorded::Value(if railway { 0 } else { i64::sample(tick) }),
            country: Recorded::Value(Some(u32::sample(tick))),
            building_type: Recorded::Value((!railway).then(|| u32::sample(tick + 1))),
            target_flags: Recorded::Value((!railway).then(|| u32::sample(tick + 2))),
            target_location: Recorded::Value((!railway).then(|| u32::sample(tick + 3))),
            target_state: Recorded::Value((!railway).then(|| u32::sample(tick + 4))),
            target_country: Recorded::Value((!railway).then(|| u32::sample(tick + 5))),
            route_first: Recorded::Value(railway.then(|| u32::sample(tick + 6))),
            route_last: Recorded::Value(railway.then(|| u32::sample(tick + 7))),
        }
    }
}
impl Fixture for ProductionLineOutputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            line: (key >> 32) as u64,
            ordinal: key as u32,
            country: u32::sample(tick),
            variant: u64::sample(tick),
            mio: u64::sample(tick),
            units: i64::sample(tick),
            mio_credit: RetiredI64::sample(tick),
            since_hour: i32::sample(tick),
            observed_since_creation: bool::sample(tick),
            ended: bool::sample(tick + 1),
            exact: bool::sample(tick),
            spent_raw: Recorded::<Option<i64>>::sample(tick),
            instant_raw: Recorded::<RetiredOptionalI64>::sample(tick),
            purpose: Recorded::Value((tick % 4) as u8),
        }
    }
}
fixture!(CombatLogEquipmentRow, |key| {country: (key >> 88) as u32, group: (key >> 56) as u32, kind: (key >> 48) as u8, month: (key >> 32) as u16, variant: key as u32}, {date: i32, amount_raw: i64});
fixture!(DivisionCensusRow, |key| {country: key as u32}, {grouped: u32, ungrouped: u32});
fixture!(CombatLogStateRow, |_key| {}, {viewing_country_raw: i32, prune_observed: u8});
fixture!(LendLeaseEquipmentRow, |key| {giver: (key >> 72) as u32, exchange: (key >> 40) as u32, kind: (key >> 32) as u8, variant: key as u32}, {receiver: u32, amount_raw: i64});
fixture!(VolunteerHostRow, |key| {country: (key >> 32) as u32, position: key as u32}, {host: u32, deployed_divisions: i32, air_capacity_modifier: Recorded<Option<i64>>});
fixture!(TheatreAreaRow, |key| {theatre: (key >> 32) as u64, ordinal: key as u32}, {representative: Option<u32>, provinces: Vec<u32>});
fixture!(StationedUnitRow, |key| {theatre: (key >> 32) as u64, ordinal: key as u32}, {unit: u64});
fixture!(UnitIntelRow, |key| {unit: (key >> 32) as u64, ordinal: key as u32}, {viewer: u32, level: i8});
fixture!(AirBaseRegistrationRow, |key| {base: (key >> 32) as u64, slot: key as u32}, {capacity_raw: i64, wings: Vec<u64>, carrier: Recorded<Option<AirBaseRegistrationCarrierInputs>>, transport_planes: Recorded<Retired>, fuel_demand_raw: Recorded<Option<i64>>, fuel_allocated_raw: Recorded<Option<i64>>});
fixture!(ProductionLicenseRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {variant: u64});
fixture!(ProductionLicenseRecordRow, |key| {owner: (key >> 32) as u32, ordinal: key as u32}, {giver: u32, lended_cic: i32, required_cic: i32, start_date: i32, parent: u64, operand_raw: i64, equipment: Vec<u64>});
fixture!(DeploymentConveyorRow, |key| {handle: key as u64}, {country: u32, ordinal: u32, template: u64, amount: i32, location: u32, priority: u32, role_token: u32, closed: u8, name: Vec<u8>, exile_country: Option<u32>});
fixture!(DeploymentLineRow, |key| {handle: key as u64}, {conveyor: u64, country: u32, ordinal: u32, name_order: i32, amount: i32, training_raw: i64, max_training_raw: i64, max_manpower: i32, allow_zero_entries: u8, equipment_allocations: i32, distributable_priority: i32, halting_reason_present: u8, halting_reason: i32, custom_name: Vec<u8>, custom_flag_a: u8, custom_flag_b: u8});
fixture!(DeploymentManpowerRow, |key| {line: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32}, {source_country: u32, amount: i32});
fixture!(DeploymentEquipmentRow, |key| {line: (key >> 32) as u64, ordinal: key as u32}, {variant_handle: u64, amount_raw: i64});
fixture!(WorldTensionRow, |_key| {}, {total_raw: i64, source_count: i32, country_aggregate_count: i32});
fixture!(ProvinceNameRow, |key| {province: key as u32}, {name: Vec<u8>});
fixture!(StateFlagRow, |key| {state: key as u32}, {demilitarized: u8, border_conflict_active: Recorded<u8>, border_conflict_countries: Recorded<Vec<[i32; 2]>>});
sample_struct!(ParadropPreparationTerms { target: u32, country_term: i64, relation_term: Option<i64>, leader_term: Option<i64>, days_term: i64 });
sample_struct!(ParadropPreparationInputs { progress: i64, terms: Option<ParadropPreparationTerms> });
sample_struct!(OrderExecutionInputs { route_is_ok: Option<u8>, paradrop: Option<ParadropPreparationInputs> });
impl Sample for OrderChildFront {
    fn sample(tick: usize) -> Self {
        Self {
            group: (u64::from(u32::sample(tick)) << 32)
                | u64::from(engine_reference_values::native_values::orders_group::REFERENCE_TYPE),
            segment_start: i32::sample(tick),
            segment_end: i32::sample(tick + 1),
        }
    }
}
impl Sample for OrderChildFrontInputs {
    fn sample(tick: usize) -> Self {
        match tick % 7 {
            0 => Self {
                area_province_count: 2,
                ..Default::default()
            },
            1 => Self {
                area_province_count: i32::MAX,
                ..Default::default()
            },
            2 => Self {
                area_province_count: 3,
                count: Some(0),
                group_child_count: None,
                sections: Some(Vec::new()),
            },
            3 => Self {
                area_province_count: i32::MAX,
                count: Some(2),
                group_child_count: None,
                sections: None,
            },
            4 => Self {
                area_province_count: 3,
                count: Some(2),
                group_child_count: Some(-1),
                sections: None,
            },
            5 => Self {
                area_province_count: 3,
                count: Some(2),
                group_child_count: Some(1),
                sections: Some(vec![OrderChildFront::sample(tick)]),
            },
            _ => {
                let first = OrderChildFront::sample(tick);
                let second = OrderChildFront {
                    group: first.group,
                    ..OrderChildFront::sample(tick + 1)
                };
                Self {
                    area_province_count: i32::MAX,
                    count: Some(2),
                    group_child_count: Some(3),
                    sections: Some(vec![first, second]),
                }
            }
        }
    }
}
fixture!(ArmyOrderPlanRow, |key| {owner_group: (key >> 32) as u64, instance: key as u32}, {front: u64, front_valid: u8, section: u32, segment_start: i32, segment_end: i32, blitz: u8, withdraw: u8, fallback: u8, virtual_order: u8, has_child: u8, owner_override: u64, assigned_divisions: Vec<u64>, predecessors: Vec<u32>, successors: Vec<u32>, name_prefix: Vec<u8>, name: Vec<u8>, name_first: Vec<u8>, name_second: Vec<u8>, name_unique: Vec<u8>, name_postfix: Vec<u8>, child_fronts: Recorded<Option<OrderChildFrontInputs>>, invasion_transports: Recorded<Option<i32>>, execution: Recorded<Option<OrderExecutionInputs>>});
fixture!(OrdersGroupPlanRow, |key| {group: key as u64}, {red: u32, green: u32, blue: u32, alpha: u32, primary_orders: Vec<u32>, fallback_orders: Vec<u32>, name: Vec<u8>});
impl Fixture for FrontRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            front: key as u64,
            section_ids: vec![7 + tick as u32, 3],
            section_ends: vec![if tick.is_multiple_of(2) { 0 } else { 2 }, 2],
            province_pairs: vec![tick as u32, tick as u32 + 1],
        }
    }
}
sample_struct!(TrainAllocation {
    variant: u64,
    count: i32
});
impl Sample for SparseBytes {
    fn sample(tick: usize) -> Self {
        let dense: Vec<u8> = (0..(tick % 3) * 150)
            .map(|index| {
                if index % 5 < 2 && !(index + tick).is_multiple_of(3) {
                    (index % 255 + 1) as u8
                } else {
                    0
                }
            })
            .collect();
        Self::from_dense(&dense).unwrap()
    }
}
fixture!(SupplyNodeRow, |key| {country: (key >> 32) as u32, index: key as u32}, {province: u32, kind: u32, flag: u8, available: i64, consumed: i64, parent: i32, route_from: u16, route_to: u16, break_after: u8, path: Vec<u32>});
sample_struct!(CombatModifierValue {
    id: u32,
    value_a: i64,
    value_b: i64
});
sample_struct!(LandCombatCommanderTrait { token_id: i32, definition_ordinal: i32, kind: u32, relevant: u8, token: Vec<u8> });
sample_struct!(LandCombatCommanderLearningTrait { token_id: i32, definition_ordinal: i32, required_experience: i32, experience: i64, token: Vec<u8> });
sample_struct!(LandCombatCommanderPortraitModifier {
    source: u8,
    id: u32,
    value: i64,
    metadata_flags: u32,
    category_mask: u32,
    feature_selector: u32
});
sample_struct!(LandCombatCommanderPortraitTooltip { source: u8, bucket: u32, key: u32, text: Vec<u8> });
sample_struct!(CombatCommanderSkillDefinition {
    present: u8,
    valid: u8,
    level: i32,
    required_experience: i32,
    category: i32
});
sample_struct!(CombatCommanderPortraitEntry { role: u32, size: u32, is_set: u8, kind: u32, text: Vec<u8> });
sample_struct!(CombatCommanderPresentation { character: u64, character_state: u8, character_country: u32, character_gender: u32, character_role_leader: u64, character_fallback_leader: u64, character_role_source: u8, fallback_text: [Vec<u8>;3], portraits: Vec<CombatCommanderPortraitEntry>, role_traits: Vec<LandCombatCommanderTrait> });
sample_struct!(NavalCommanderInputs { commander: u64, country: u32, category: i32, experience: i64, skills: [i32; 5], skill_definitions: [CombatCommanderSkillDefinition;5], next_skill: CombatCommanderSkillDefinition, name: Vec<u8>, description: Vec<u8>, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait>, learning_threshold: i64, modifiers: Vec<LandCombatCommanderPortraitModifier>, modifier_feature_flags: u32, modifier_callback_kind: u8, tooltips: Vec<LandCombatCommanderPortraitTooltip>, timed_traits: Recorded<Vec<LandCombatCommanderTimedTrait>> });
sample_struct!(LandCombatSideDetails { committed: Vec<u64>, reserves: Vec<u64>, tactic_id: i32, tactic_kind: u8, tactic_counter_id: u32, tactic_token: Vec<u8>, air_counts: Vec<i32>, anti_air_attack: RetiredKept<i64>, air_kills: u32, damage: [i64; 2], ground_damage: RetiredNested<[i64; 2]>, prevented_damage: RetiredKept<[i64; 2]>, organisation_loss_sum: i64, organisation_loss_count: u32, commander: u64, commander_skill: i32, commander_category: i32, commander_country: u32, commander_experience: RetiredNested<i64>, commander_name_token: Vec<u8>, commander_description_token: RetiredNested<Vec<u8>>, commander_formation: u64, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, timed_traits: RetiredNested<Vec<LandCombatCommanderTimedTrait>>, learning_traits: RetiredNested<Vec<LandCombatCommanderLearningTrait>> });
sample_struct!(LandCombatExileInputs { origin_type: i32, country: i32, country_fallback: i32, location_cores: Vec<u32>, occupiers: Vec<u32> });
sample_struct!(LandCombatCommanderTimedTrait {
    token_id: i32,
    definition_ordinal: i32,
    remaining_days: i32
});
sample_struct!(LandCombatLearningProgress { token_id: i32, definition_ordinal: i32, progress: i64, token: Vec<u8> });
sample_struct!(LandCombatSideInputs { anti_air_attack: i64, prevented_damage: [i64; 2], skill_progress: i64, skill_modifiers: Option<[i64; 2]>, description_token: Vec<u8>, timed_traits: Vec<LandCombatCommanderTimedTrait>, learning_traits: Option<Vec<LandCombatLearningProgress>> });
sample_struct!(LandCombatTacticCountryWeight {
    country: u32,
    tactic_id: i32,
    modifier_key: u32,
    metadata_flags: u32,
    metadata_gate: i32,
    country_factor_raw: i64,
    accumulator_raw: i64,
    accumulator_entry_present: u8
});
sample_struct!(LandCombatTacticInputs {
    preferred_tactic_ids: [[i32; 3]; 2],
    country_weights: [LandCombatTacticCountryWeight; 2],
    country_weight_present: [u8; 2]
});
sample_struct!(LandDivisionModifier {
    raw: i64,
    present: u8
});
sample_struct!(LandDivisionWidthTerm {
    subunit: u32,
    count: i32,
    width: i64
});
sample_struct!(LandDivisionContext {
    template: u64,
    leader_modifier_kind: u8,
    country_modifiers: [LandDivisionModifier; 3],
    leader_modifiers: [LandDivisionModifier; 3],
    modifier_metadata: [[u32; 2]; 3],
    feature_flags: u32,
    eligibility_callback: u8
});
sample_struct!(LandCombatMemberValues {
    naming: NameOperands,
    strength_combat_factor: i64,
    fuel_inputs: [i64; 2],
    stat_values: [i64; 5],
    armour: i64,
    piercing: i64,
    air_attack: i64,
    reconnaissance: i64,
    initiative: i64,
    base_speed: i64
});
sample_struct!(NavalCombatSummaryModifier {
    raw: i64,
    present: u8
});
sample_struct!(NavalCombatSummaryInputs { recent_join_penalty: i64, escort_bonus: i64, countries: Vec<u32>, secondary_countries: Vec<u32>, modifiers: [[NavalCombatSummaryModifier; 5]; 5], modifier_metadata: [[u32; 2]; 5], feature_flags: u32, eligibility_callback: u8 });
sample_struct!(NavalCombatAirEfficiencyModifier {
    raw: i64,
    present: u8
});
sample_struct!(NavalCombatAirEfficiency {
    source: u8,
    pool_type_flags: u64,
    mission: i32,
    container_present: u8,
    base_crowding: i64,
    night: i64,
    modifiers: [NavalCombatAirEfficiencyModifier; 3],
    modifier_metadata: [[u32; 2]; 4],
    feature_flags: u32,
    eligibility_callback: u8
});
sample_struct!(NavalCombatAirWingKill {
    wing: u64,
    index: u32,
    category: u64,
    count: i32
});
sample_struct!(NavalCombatPlane { wing_types: Vec<u32>, source: u8, group_index: u32, index: u32, carrier_slot: i32, carrier_member_present: u8, carrier_state: u32, air_base: u64, wings: [Vec<u64>;2], initial_planes: i32, current_planes: i32, casualties: i32, time_duration: i32, country: u32, date: i32, flags: [u8; 2], variant: u64, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2], kills: Vec<NavalCombatAirWingKill>, efficiency: NavalCombatAirEfficiency });
sample_struct!(NavalCombatSideDetails { damage_history: [[i64;6];6], original_health: i64, total_damage: i64, damage_by_type: Vec<NavalDamageSource>, commander: u64, engaged: u8, retreating: u8, capital_screening: i64, carrier_screening: i64, positioning: i64, anti_air: i64, planes: Vec<NavalCombatPlane>, summary: NavalCombatSummaryInputs, commander_inputs: Option<NavalCommanderInputs>, units: Vec<u64> });
sample_struct!(NavalDamageSource {
    token: u32,
    damage: i64
});
sample_struct!(NavalCombatDamage {
    source: u64,
    country: u32,
    damage: i64
});
sample_struct!(NavalCombatAirHit {
    variant: u64,
    count: i32,
    country: u32
});
sample_struct!(NavalCombatHit { target_slot: u32, target_name: Vec<u8>, convoy: u8, damage: i64, denominator: i64, last_hit: u8 });
sample_struct!(NavalCombatLastTarget {
    present: u8,
    is_convoy: u8,
    target_group_member_count: i32
});
sample_struct!(NavalShipModifierTerm {
    key: u32,
    raw: i64,
    metadata_flags: u32,
    feature_selector: u32
});
sample_struct!(NavalShipModifierSource { scope: u8, index: u32, selector: u32, reference: u64, weights: [i64; 2], movement: i64, dynamic_keys: [u32; 2], terms: Vec<NavalShipModifierTerm> });
sample_struct!(NavalShipWeaponContext { task_force: u64, country: u32, commander: u64, ship_class: u32, subunit: u32, weapon_bases: [i64; 5], weather_factor: i64, fuel: [i64; 2], night: i64, formation_flags: u64, detection_base: i64, air_base: u64, air_base_maximum: i32, air_detection_base: i64, terrain_name: Vec<u8>, sources: Vec<NavalShipModifierSource> });
sample_struct!(NavalResultShip { member: u64, owner: u32, subunit_index: i32, formation_type: i32, build_cost_ic: i64, strength_ratio: i64, pride: u8, is_convoy: u8, name: Vec<u8>, type_name: Vec<u8>, sprite: Vec<u8>, sunk_by: Vec<u8>, variant: u64, convoy_client: u64, convoy_index: i32, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });
sample_struct!(NavalResultPlane { variant: u64, variant_summary_inputs: [i64; 2], initial_planes: i32, current_planes: i32, lost_planes: i32, country: u32, name: Vec<u8>, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });
sample_struct!(NavalResultSideDetails { leader: u64, hq_leader: u64, hq_influences: Vec<u64>, ships: Vec<NavalResultShip>, planes: Vec<NavalResultPlane>, hq_modifiers: Vec<NavalShipModifierSource>, commander_context: Option<NavalCommanderInputs>, hq_commander_context: Option<NavalCommanderInputs> });
sample_struct!(LandBorderWarSide { country: i32, state: i32, win_event: Vec<u8>, lose_event: Vec<u8> });
sample_struct!(LandBorderWarInputs {
    transfer_state: u8,
    sides: [LandBorderWarSide; 2],
    combat_width: Recorded<i64>
});
fixture!(LandCombatDetailsRow, |key| { combat: key as u64 }, { province: u32, phase: i32, width_factor: i64, attacker_bonus: i64, defender_bonus: i64, stored_terrain_valid: u8, stored_terrain_name: Vec<u8>, sides: [LandCombatSideDetails;2], tactic_inputs: LandCombatTacticInputs, border_war: Recorded<Option<LandBorderWarInputs>>, side_inputs: Recorded<[LandCombatSideInputs;2]> });
impl Fixture for LandCombatDivisionRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let mut row = Self {
            combat: (key >> 33) as u64,
            side: ((key >> 32) & 1) as u8,
            ordinal: key as u32,
            role: u8::sample(tick),
            province: u32::sample(tick),
            division: u64::sample(tick),
            commander: u64::sample(tick),
            scales: <[i64; 4]>::sample(tick),
            context: LandDivisionContext::sample(tick),
            width_terms: Vec::<LandDivisionWidthTerm>::sample(tick),
            direction_group: i32::sample(tick),
            exile: Option::<LandCombatExileInputs>::sample(tick),
            categories: u64::sample(tick),
            reinforcement_speed: Option::<i64>::sample(tick)
                .map(|share| share.clamp(0, LandCombatDivisionRow::SPEED_SHARE_MAXIMUM)),
            own: Option::<LandCombatMemberValues>::sample(tick),
        };
        RowCodec::round_to_display(&mut row);
        row
    }
}
fixture!(LandCombatModifierRow, |key| { combat: (key >> 1) as u64, side: (key & 1) as u8 }, { border_values: Option<[i64;3]>, divisions: Retired });
impl Fixture for LandCombatDivisionModifierRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let mut row = Self {
            combat: (key >> 33) as u64,
            side: ((key >> 32) & 1) as u8,
            ordinal: key as u32,
            entries: Vec::<CombatModifierValue>::sample(tick),
            division: Option::<u64>::sample(tick),
        };
        RowCodec::round_to_display(&mut row);
        row
    }
}
impl Fixture for CombatEngagementRefreshRow {
    fn smallest_key() -> u128 {
        1_u128 << 33
    }
    fn largest_key() -> u128 {
        (u128::from(Self::LOCAL_ALL_SIDES) << 97) | (u128::from(i32::MAX as u64) << 33)
    }
    fn fixture(key: u128, tick: usize) -> Self {
        let kind = ((key >> 97) & 3) as u8;
        let target = (key >> 33) as u64;
        let side = ((key >> 32) & 1) as u8;
        let viewer = key as u32;
        let triggers = if kind == Self::LOCAL_ALL_SIDES {
            if tick.is_multiple_of(2) {
                Self::TRIGGER_RECORDED
            } else {
                Self::TRIGGER_RECORDED | Self::TRIGGER_CACHE_WRITE
            }
        } else if kind == Self::CONTROLLER_TARGET {
            Self::TRIGGER_RECORDED
        } else {
            ((tick % 3) + 1) as u8
        };
        Self {
            kind,
            target,
            side,
            viewer,
            triggers,
        }
    }
}
impl Fixture for LandCombatSideRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let positive = |offset: usize| {
            (0..(tick + offset) % 3)
                .map(|index| (tick + offset + index) as u32 + 1)
                .collect()
        };
        Self {
            combat: key as u64,
            indicator: u8::sample(tick),
            flags: <[u32; 2]>::sample(tick),
            attacker_primary: Vec::<u32>::sample(tick),
            attacker_secondary: positive(1),
            defender_primary: Vec::<u32>::sample(tick + 1),
            defender_secondary: positive(2),
        }
    }
}
impl Fixture for NavalCombatDetailsRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            combat: key as u64,
            province: u32::sample(tick),
            day: i32::sample(tick),
            duration: i32::sample(tick),
            progress: i64::sample(tick),
            sides: <[NavalCombatSideDetails; 2]>::sample(tick),
            custom_sea: (!tick.is_multiple_of(4)).then_some(vec![1, 0, 0, 0, 254, 255]),
            custom_sea_error: u8::from(tick.is_multiple_of(4)),
        }
    }
}
fixture!(NavalCombatShipRow, |key| { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32 }, { ship: u64, owner: u32, state: u32, member_source: u8, group_index: u32, member_index: u32, convoy: u64, subunit_index: i32, formation_type: i32, build_cost_ic: i64, cached_strength_ratio: i64, pride: u8, is_convoy: u8, name: Vec<u8>, type_name: Vec<u8>, sprite: Vec<u8>, sunk_by: Vec<u8>, variant: u64, arrival_hours: i32, received_criticals: i32, evacuation_state: i32, detection_state: i32, escape_progress: i64, escape_speed_base: i64, last_target: NavalCombatLastTarget, received_damage: [i64; 6], live_ship: u8, live_convoy: u8, strength: i64, organisation: i64, base_organisation: i64, maximum_hit_points: i64, formation_flags: u64, weapon_context: NavalShipWeaponContext, damage_entries: Vec<NavalCombatDamage>, air_hits: [Vec<NavalCombatAirHit>;2], naval_hits: Vec<NavalCombatHit> });
fixture!(NavalCombatResultDetailsRow, |key| { result: key as u64 }, { sides: [NavalResultSideDetails;2] });
fixture!(TrainInventoryRow, |key| {country: key as u32}, {node_count: u32, allocations: Vec<TrainAllocation>, controller_access: SparseBytes, allocated_trucks: Recorded<Option<i32>>, required_trucks_raw: Recorded<Option<i64>>, required_trains: Recorded<Option<i32>>});
fixture!(CountryAppearanceRow, |key| {country: key as u32}, {red: u32, green: u32, blue: u32, alpha: u32, cosmetic_tag: Vec<u8>, name_tag: Vec<u8>, leader_ideology: Vec<u8>, overlord: i32, graphical_culture: Vec<u8>, leader_character: Option<u32>, old_name: Option<Vec<u8>>, old_adjective: Option<Vec<u8>>, old_definite_name: Option<Vec<u8>>, ui_red: u32, ui_green: u32, ui_blue: u32, ui_alpha: u32, graphical_culture_2d: Vec<u8>, subjects: Recorded<Vec<i32>>, original_tag: Recorded<Option<Vec<u8>>>, original_tag_reference: Recorded<Option<i32>>, transport_convoys: Recorded<Retired>});
fixture!(ArmyIconInputRow, |key| {division: key as u64}, {separate_counter: u8, alternate_template: u64});
fixture!(ProvinceUnitOrderRow,|key| {province:key as u32},{mixed:Vec<u64>,naval:Vec<u64>});
sample_struct!(CharacterPortrait {category:u32,size:u32,is_set:u8,kind:u32,identity:Vec<u8>});
fixture!(CharacterPortraitRow,|key| {character:key as u32},{portraits:Vec<CharacterPortrait>});
fixture!(LeaderPortraitRow,|key| {leader:key as u64},{character:u64,portrait_c0:Vec<u8>,portrait_e0:Vec<u8>,portrait_100:Vec<u8>});
sample_struct!(NavalSearcher { reference: Option<u64>, resolves: Option<bool>, roster_owner: Option<u32> });
sample_struct!(NavalEngagementTarget { reference: u64, resolves: Option<bool> });
sample_struct!(NavalPortMissionInputs { sample_hour: PacketHour, searching: Option<Vec<NavalSearcher>>, engagement_target: Option<NavalEngagementTarget>, avoided_ship_types: Option<Vec<Option<Vec<u8>>>> });
sample_struct!(HullVariantInputs { definition_index: Option<i32>, module_count: Option<i32>, upgrade_level_sum: Option<i32>, module_priority_sum: Option<i32>, version: Option<u8>, organisation_marker: Option<i32> });
fixture!(TaskForceCounterInputRow, |key| {task_force: key as u64}, {is_in_regions: u8, running_away: u8, engagement_rule: i32, mission_region_count: i32, bombardment_region: u32, repair_parent: u64, detached_activity: i32, repair_last_mission: i32, received_supply_ratio: i64, attached_headquarters: i32, name: Recorded<Option<Vec<u8>>>, supply: Recorded<Option<UnitSupplyInputs>>, port_mission: Recorded<Option<NavalPortMissionInputs>>});
sample_struct!(FreshTaskForceCounterEta { sample_hour: PacketHour, base_speed: Option<i64>, speed_modifier: Option<i64>, terrain_class: Option<i32>, modified_range: Option<i64>, exiled: Option<u8>, strategic_redeployment: Option<u8> });
fixture!(TaskForceMapMotionRow, |key| {task_force: key as u64}, {target: u32, combat_role: u8, path: Vec<u32>, alternate_path: Vec<u32>, movement_numerator: Recorded<Option<i64>>, retreat: Recorded<Option<u8>>, withdrawal: Recorded<Option<u8>>, naval_transfer: Recorded<Option<u8>>, combat_links: Recorded<Option<[u64; 2]>>, fresh: Recorded<Option<FreshTaskForceCounterEta>>});
fixture!(FloatingHarborDeploymentRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {province: i32, status: u8});
fixture!(FacilityMapFlagRow, |key| {province: key as u32}, {flag: u8});
impl Fixture for NavalProvinceCombatRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let combats = Vec::<u64>::sample(tick);
        let kinds = (0..combats.len())
            .map(|index| ((tick + index) % (Self::NAVAL as usize + 1)) as u8)
            .collect();
        Self {
            province: key as u32,
            combats,
            kinds,
        }
    }
}
fixture!(NavalCombatTransferRow, |key| {combat: key as u64}, {transfers: Vec<u64>});
fixture!(NavalTransferUnitRow, |key| {transfer: key as u64}, {units: Vec<u64>});
fixture!(TaskForceMissionRow, |key| {task_force: key as u64}, {owner: u32, current_region: u32, regions: Vec<u32>});
fixture!(NavalCombatAirBaseWingHandlesRow, |key| { base: key as u64 }, { wings: Vec<u64> });
sample_struct!(RailwayGunEtaOperands {
    numerator: i64,
    current_speed: i64
});
sample_struct!(FreshRailwayCounterEta {
    sample_hour: PacketHour,
    maximum_speed: Option<i64>,
    land_supply_factor: Option<i64>,
    orders_group: Option<u64>
});
impl Fixture for RailwayGunCounterInputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let combat_count = u32::sample(tick);
        let mut row = Self {
            gun: key as u64,
            roster_owner: u32::sample(tick),
            expeditionary_owner: u32::sample(tick),
            logical_owner: u32::sample(tick),
            province: u32::sample(tick),
            naval_transfer: u8::sample(tick),
            transfer_action_status: u8::sample(tick),
            exiled: u8::sample(tick),
            retreat: u8::sample(tick),
            strategic_redeployment: u8::sample(tick),
            reference_entry_count: i32::sample(tick),
            assignee_group: u64::sample(tick),
            subunit_token: u32::sample(tick),
            current_strength: i64::sample(tick),
            maximum_strength: i64::sample(tick),
            received_supply_ratio: i64::sample(tick),
            railway_building_level: Option::<i16>::sample(tick),
            target: u32::sample(tick),
            combat_role: u8::sample(tick),
            combat_province: u32::sample(tick),
            combat_count,
            attacking: u8::sample(tick),
            ordinary_path: Vec::<u32>::sample(tick),
            alternate_path: Vec::<u32>::sample(tick),
            name: Recorded::<Option<Vec<u8>>>::sample(tick),
            eta: Recorded::<Option<RailwayGunEtaOperands>>::sample(tick),
            supply: Recorded::<Option<UnitSupplyInputs>>::sample(tick),
            withdrawal: Recorded::<Option<u8>>::sample(tick),
            fresh: Recorded::<Option<FreshRailwayCounterEta>>::sample(tick),
            combat_references: unit_combat_references_fixture(tick, combat_count),
            transport: Recorded::Value(Retired),
            theatre: Recorded::<Option<u64>>::sample(tick + 1),
            queued_actions: Recorded::<Option<i32>>::sample(tick + 2),
        };
        RowCodec::round_to_display(&mut row);
        row
    }
}
fixture!(AirDailyRosterRow, |key| {country: key as u32}, {eligible: bool});
fixture!(FleetCounterInputRow, |key| {fleet: key as u64}, {red: u32, green: u32, blue: u32, alpha: u32, mission_region_count: i32, task_forces: Vec<u64>, mission_regions: Recorded<Option<std::sync::Arc<Vec<u32>>>>});
impl Fixture for CountryCounterInputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let country = key as u32;
        let mut directed_wars = Option::<DirectedWars>::sample(tick);
        if let Some(packet) = &mut directed_wars {
            packet.receiver_raw_id = country as i32;
        }
        Self {
            country,
            pride_of_the_fleet: u64::sample(tick),
            owned_state_count: i32::sample(tick),
            controlled_state_count: i32::sample(tick),
            first_division: u64::sample(tick),
            owned_states: Recorded::<Option<Vec<u32>>>::sample(tick),
            controlled_states: Recorded::<Option<Vec<u32>>>::sample(tick),
            core_states: Recorded::<Option<Vec<u32>>>::sample(tick),
            army_leaders: Recorded::<Option<Vec<u64>>>::sample(tick),
            navy_leaders: Recorded::<Option<Vec<u64>>>::sample(tick),
            mio_policies: Recorded::<Option<Vec<u32>>>::sample(tick),
            neighbors: Recorded::<Option<Vec<u32>>>::sample(tick),
            co_belligerents: Recorded::<Option<Vec<u32>>>::sample(tick),
            zero_war: Recorded::<Option<[u8; 2]>>::sample(tick),
            movement: Recorded::Value(Retired),
            zero_subject: Recorded::<Option<ZeroSubjectInputs>>::sample(tick),
            pride_name: Recorded::<Option<Vec<u8>>>::sample(tick),
            forward_visibility: Recorded::<Option<ReverseZeroVisibilityInputs>>::sample(tick),
            attack: Recorded::<Witnessed<CountryAttackInputs>>::sample(tick),
            directed_wars: Recorded::Value(directed_wars),
            attack_modifiers: Recorded::<Option<CountryAttackModifiers>>::sample(tick + 1),
            movement_modifiers: Recorded::<Option<CountryMovementModifiers>>::sample(tick + 2),
            combat_width_factor: Recorded::<Option<i64>>::sample(tick + 3),
            sent_expeditionaries: Recorded::Value(Some(vec![0, 7, 7, u64::MAX].into())),
            recruitment_modifiers: Recorded::Value(Some([Some(17), None, Some(-2), Some(5)])),
            air_modifiers: Recorded::<Option<[Option<i64>; 10]>>::sample(tick + 3),
            air_combat_modifiers_known: true.into(),
            air_superiority_efficiency: (Option::<i64>::sample(tick)).into(),
            air_intercept_efficiency: (Option::<i64>::sample(tick + 1)).into(),
            air_cas_efficiency: (Option::<i64>::sample(tick + 2)).into(),
            air_nav_efficiency: (Option::<i64>::sample(tick + 3)).into(),
            air_attack_factor: (Option::<i64>::sample(tick + 4)).into(),
            air_defence_factor: (Option::<i64>::sample(tick + 5)).into(),
            air_strategic_bomber_defence_factor: (Option::<i64>::sample(tick + 6)).into(),
            mines_effect_reduction: Recorded::Value(Some(Some(tick as i64))),
            naval_detection: Recorded::Value(Some(Some(-(tick as i64)))),
        }
    }
}
fixture!(OrdersGroupCounterInputRow, |key| {group: key as u64}, {withdrawing: u8, hq_distance: Recorded<Option<i32>>, leader_enroute_days: Recorded<Option<i32>>, border_war_presence: Recorded<Option<u8>>, invasion_source_distance: Recorded<Option<i32>>, hq_nearest_front: Recorded<Option<i32>>, template_context: Recorded<Option<std::sync::Arc<TemplateHqContext>>>, attack: Recorded<Witnessed<OrdersGroupAttackInputs>>, commander_attack: Recorded<Option<Option<CommanderAttackStats>>>, commander_movement: Recorded<Option<Option<CommanderMovementStats>>>});
fixture!(LeaderPortraitPathRow, |key| {leader: key as u64}, {country: u32, tag_reference: u32, tag: Vec<u8>});
impl Fixture for ShipModelRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            ship: key as u64,
            variant: u64::sample(tick),
            hull_sprite: Vec::<u8>::sample(tick),
            fallback_sprite: Vec::<u8>::sample(tick + 1),
            model_number: i32::sample(tick),
            formation_flags: u64::sample(tick),
            subunit_index: Recorded::<Option<i32>>::sample(tick),
            strike: Recorded::Value(Retired),
            hull_variant: Recorded::<Option<HullVariantInputs>>::sample(tick),
            has_criticals: Some((tick % 2) as u32).into(),
            air_capacity_kind: Some((tick % 3 + 1) as u32).into(),
            naval_power: Recorded::<Option<[i64; 5]>>::sample(tick),
            counter: Some(ShipCounterInputs {
                roster_ordinal: tick as u32,
                maximum_strength: i64::sample(tick),
                anti_air_base: i64::sample(tick + 1),
                ..Default::default()
            }),
        }
    }
}
fixture!(LocalPlayerRow, |_key| {}, {country: u32, observer_country: u32});
fixture!(LandUnitOrderRow, |_key| {}, {units: Recorded<Option<Vec<u64>>>});
fixture!(SimulationFeatureRow, |_key| {}, {dlc_mask: u32, human_ai: u8, state_array_len: Recorded<Option<u32>>, weather_suspend: Recorded<Option<[u8; 2]>>, instant_decisions: Recorded<Option<u8>>, runtime_scalars: Recorded<PoliticalRuntimeScalars>, movement_defines: Recorded<MovementDefines>});
sample_struct!(MovementDefines { out_of_supply_speed: Option<i64>, retreat_speed: Option<i64>, withdrawal_speed: Option<i64>, out_of_fuel_speed: Option<i64>, fuel_capacity_scale: Option<i64>, supply_days: Option<i64>, supply_grace: Option<i64>, air_speed: Option<i64>, air_defense_denominator: Option<i64>, air_defense_reduction: Option<i64>, out_of_supply_attrition: Option<i64> });
fixture!(RaidPlayerAllowedTypeRow, |key| {country: key as u32}, {allowed: Vec<u32>});
impl Fixture for ModifierDefinitionRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            index: key as u32,
            token: u32::sample(tick),
            name_key: Vec::<u8>::sample(tick + 1),
            postfix_key: Vec::<u8>::sample(tick + 2),
            argument: Vec::<u8>::sample(tick + 3),
            flags: u32::sample(tick + 4),
            category_mask: u32::sample(tick + 5),
            feature_selector: u32::sample(tick + 6),
            precision: i32::sample(tick + 7),
            eligibility: (tick % 2) as u8,
        }
    }
}
fixture!(CountryVisibilityRow, |key| {country: key as u32}, {unit_modifier: i64});
fixture!(StateVisibilityRow, |key| {state: key as u32}, {country: u32, network_modifier: i64});
sample_struct!(TransportSubjectAccess { sample_hour: PacketHour, complete_owner: bool,
    target_raw: Option<u32>, subject: Option<ZeroSubjectInputs> });
fixture!(RelationLinkRow, |key| {country: (key >> 32) as u32, target_index: key as u32},
    {links: u8, transport_subject: Recorded<Option<TransportSubjectAccess>>});
fixture!(CountryWarRow, |key| {country: key as u32}, {can_occupy_non_war: u8, enemies_read: u8, enemies: Vec<u32>, ordered: Recorded<Option<OrderedEnemyInputs>>});
fixture!(NavalRegionAccessRow, |key| {country: (key >> 32) as u32, region: key as u32}, {mode: u8});
fixture!(BorderConflictRow, |key| {province: key as u32}, {groups: Vec<u64>});
fixture!(RaidTargetStatusRow, |key| {ordinal: key as u32}, {valid: u8, scope: RaidScope, detected: Vec<u8>});
impl Fixture for RaidPathDistanceRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            dispatcher: if key >> 64 == 0 {
                RaidPathDispatcher::Land
            } else {
                RaidPathDispatcher::Naval
            },
            source: (key >> 32) as u32,
            target: key as u32,
            distance: i64::sample(tick),
            ok: (tick % 2) as u8,
        }
    }
}
fixture!(RaidCooldownRow, |key| {owner_slot: (key >> 32) as u32, ordinal: key as u32}, {raid_type: u32, scope: RaidScope, days: i32});
fixture!(StateBuildingInputRow, |key| {state: key as u32}, {owner: u32, controller: u32, extra_shared_slots: i32, local_building_slots: Option<i64>, local_building_slots_factor: Option<i64>, global_building_slots: Option<i64>, global_building_slots_factor: Option<i64>, local_factories: Option<i64>, local_factory_energy_consumption: Option<i64>, local_factory_energy_consumption_per_infrastructure: Option<i64>, factory_energy_consumption: Option<i64>, factory_energy_factor: Option<i64>});
fixture!(StateResourceInputRow, |key| {state: key as u32}, {gate: u8, global_factor: i64, origin: u8, infrastructure_level: Option<i32>, infrastructure_modifier: i64, supply_route: u8, local_manpower: Recorded<Option<i64>>, local_noncore_manpower: Recorded<Option<i64>>, recruitment_add: Recorded<Option<i64>>, recruitment_factor: Recorded<Option<i64>>});
fixture!(StateResourceRow, |key| {state: (key >> 32) as u32, resource: key as u32}, {pool: i64, additive: i64, factor: i64, cost: i64});
fixture!(StateDynamicModifierRow, |key| {state: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>, scope_country: u32, scope_state: i32});
fixture!(CountryBuildingResourceRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {entry: u32, building: u32, count: u32, resource: u32, amount: i64});
impl Fixture for ResourceOriginRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            state: (key >> 32) as u32,
            country: key as u32,
            ordinal: u32::sample(tick),
            list: (tick % 2) as u8,
            efficiency: i64::sample(tick),
            route_factor: i64::sample(tick),
            local_resources_factor: i64::sample(tick),
            bad: bool::sample(tick),
            convoys_assigned: i32::sample(tick),
            convoys_required: i32::sample(tick),
            convoys_base: Recorded::<i32>::sample(tick + 1),
        }
    }
}
fixture!(ResourceRightRow, |key| {state: (key >> 32) as u32, resource: key as u32}, {holder: u32, ordinal: u32});
fixture!(CountryOilFactorRow, |key| {country: key as u32}, {factor: i64});
fixture!(StateTemporaryResourceRow, |key| {state: (key >> 32) as u32, resource: key as u32}, {amount: i64});
fixture!(ConvoyClientRow, |key| {handle: key as u32}, {owner_ordinal: Option<u32>, other_ordinal: Option<u32>, category: u8, owner: u32, other: u32, route_class: u8, route_country: u32, route_other: u32, bad: bool, convoys_assigned: i32, convoys_required: i32, efficiency_a: Recorded<i64>, efficiency_b: Recorded<i64>, convoys_base: Recorded<i32>});
fixture!(CountryWargoalRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {handle: u64, actor: u32, recipient: u32, goal_type: u32, expires: i32, name_token: u32});
fixture!(WargoalJustificationRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {actor: u32, recipient: u32, goal_type: u32, total_cost_raw: i64, daily_cost_raw: i64, progress_raw: i64, start: i32, end: i32, pay_upfront: u8, threat_raw: i64});
impl Fixture for WargoalTargetRow {
    fn largest_key() -> u128 {
        (u32::MAX as u128) << 72 | 4u128 << 64 | u64::MAX as u128
    }
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 72) as u32,
            kind: (key >> 64) as u8,
            goal: (key >> 32) as u32,
            ordinal: key as u32,
            value: u32::sample(tick) as u64,
        }
    }
}
fixture!(RegionalConvoyEfficiencyRow, |key| {country: (key >> 32) as u32, region: key as u32}, {efficiency: i64});
fixture!(StrategicRegionWeightRow, |key| {region: key as u32}, {weight: i32});
fixture!(FactionRuleRow, |key| {faction: (key >> 32) as u64, ordinal: key as u32}, {token: u32});
fixture!(FactionProgramRow, |key| {faction: (key >> 32) as u64, ordinal: key as u32}, {reference: u64});
fixture!(FactionUnlockRow, |key| {faction: key as u64}, {research: u8, manpower: u8});
fixture!(FactionManpowerRow, |key| {faction: (key >> 32) as u64, ordinal: key as u32}, {country: u32, value: i32});
fixture!(FactionGoalStatusRow, |key| {faction: key as u64}, {manifest: Option<u32>});
fixture!(FactionMinimumUpgradeRow,|key|{faction:(key>>32) as u64,ordinal:key as u32},{group:u32,token:u32});
fixture!(FactionDoctrineRow,|key|{faction:(key>>32) as u64,ordinal:key as u32},{token:u32});
fixture!(FactionAdvisorModifierRow,|key|{faction:(key>>64) as u64,slot:(key>>32) as u32,ordinal:key as u32},{kind:u32,country:u32,character:Option<u64>,date:i32,name:Vec<u8>,modifier:u32,raw:i64,date_present:u8});
fixture!(FactionProjectionRow,|key|{faction:key as u64},{projection_raw:i64,influence_raw:i64,effects_raw:i64,influence_phase:u64,influence_members:u32,influence_awards_raw:i64,rank_phase:u64,rank_members:u32,projection_phase:u64,influence_share_phase:u64,influence_accounts:Vec<u32>});
fixture!(FactionResourceRow,|key|{faction:(key>>32) as u64,ordinal:key as u32},{token:u32,raw:i64});
fixture!(FactionResearchRow, |key| {faction:(key>>32) as u64,ordinal:key as u32}, {scientist_base_raw:i64, countries:Vec<u32>, bonus_raw:i64});
impl Fixture for FactionAccountRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            contribution_subject_gain: Option::<i64>::sample(tick),
            population_growth: Option::<i64>::sample(tick + 1),
            account: (key >> 64) as u32,
            kind: (key >> 32) as u32,
            ordinal: key as u32,
            statistic_raw: i64::sample(tick),
            country: u32::sample(tick),
            influence_numerator_raw: i64::sample(tick),
            influence_denominator_raw: i64::sample(tick),
            influence_refresh: u32::sample(tick),
            influence_faction: u64::sample(tick),
            influence_supplied: u8::sample(tick),
            influence_total_raw: i64::sample(tick),
            rank_derived: u8::sample(tick),
            rank_faction: u64::sample(tick),
            rank_input_raw: i64::sample(tick),
            rank_input_order: u32::sample(tick),
            rank_members: u32::sample(tick),
            projection_numerator_raw: i64::sample(tick),
            projection_denominator_raw: i64::sample(tick),
            projection_refresh: u32::sample(tick),
            influence_share_raw: i64::sample(tick),
            projection_share_raw: i64::sample(tick),
            projection_rank: i32::sample(tick),
            projection_derived: u8::sample(tick),
            initiative_raw: i64::sample(tick),
            initiative_product_raw: i64::sample(tick),
            initiative_steps: u32::sample(tick),
            contribution_raw: i64::sample(tick),
            contribution_gain_a_raw: i64::sample(tick),
            contribution_gain_b_raw: i64::sample(tick),
            previous_war_score_raw: i64::sample(tick),
            upgrade_groups: vec![u32::sample(tick)],
            upgrade_tokens: vec![u32::sample(tick + 1)],
            completed_goals: vec![u64::MAX, u32::sample(tick) as u64],
        }
    }
}
impl Fixture for FactionVariableRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let ordinal = key as u32;
        Self {
            faction: (key >> 32) as u64,
            ordinal,
            name: if ordinal == 0 {
                Vec::new()
            } else {
                Vec::<u8>::sample(tick)
            },
            value_raw: if ordinal == 0 { 0 } else { i64::sample(tick) },
            seed: (ordinal == 0).then(|| u32::sample(tick)),
        }
    }
}
impl Fixture for FactionGoalEntryRow {
    fn largest_key() -> u128 {
        (u64::MAX as u128) << 40 | 4_u128 << 32 | u32::MAX as u128
    }
    fn fixture(key: u128, tick: usize) -> Self {
        let kind = (key >> 32) as u8;
        Self {
            faction: (key >> 40) as u64,
            kind,
            ordinal: key as u32,
            token: if kind == 0 {
                Some(u32::sample(tick))
            } else {
                Option::<u32>::sample(tick)
            },
            status: i32::sample(tick),
            progress_raw: i64::sample(tick),
            active_sections: Vec::<u32>::sample(tick),
            date: i32::sample(tick),
            slots: i32::sample(tick),
        }
    }
}
fixture!(TradeRouteRow, |key| {country: (key >> 40) as u32, class: (key >> 32) as u8, other: key as u32}, {kind: u8, from: u32, to: u32, path: Vec<u32>, source_state: Option<u32>, destination_state: Option<u32>});
fixture!(RaidCountryInputRow, |key| {canonical_slot: key as u32}, {air_range_factor: Option<i64>, naval_commando_range_factor: Option<i64>, autonomous: Option<u32>, convoy_reservations: Vec<u32>, volunteer_stockpile_planes: Recorded<Option<i32>>});
fixture!(CountryCommandPowerRow, |key| {canonical_slot: key as u32}, {allocated_command_power_raw: i64, max_command_power: Option<i64>, max_command_power_factor: Option<i64>, ace_effectiveness_factor: Option<i64>, air_ace_bonuses_factor: Option<i64>});
fixture!(DivisionRetreatBanRow, |key| {division: key as u64}, {owner: u32, bans: u8});
fixture!(StateCoreRow, |key| {state: key as u32}, {cores: Vec<u32>, claims: Recorded<Vec<u32>>, strategic_locations: Recorded<Vec<u64>>, province_strategic_locations: Recorded<Vec<u64>>});
fixture!(InvasionPreparationRow, |key| {owner_group: (key >> 32) as u64, instance: key as u32}, {progress: i64, target: u32, country_term: i64, relation_term: Option<i64>, relation_bonus: Option<i64>, leader_term: Option<i64>, leader_applies: u8, days_term: i64, unit_owner: u32, zero_war_present: RetiredU8, zero_war_retired: RetiredU8, zero_co_belligerent: RetiredU8, co_belligerents: RetiredU32List});
fixture!(CountryCharacterRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {character: Option<u32>});
fixture!(PartyLeaderRow, |key| {country: (key >> 64) as u32, party: (key >> 32) as u32, ordinal: key as u32}, {character: Option<u32>, leader_id: i32, role: Option<i32>});
fixture!(CountryCivilWarRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {target: u32});
fixture!(AdvisorTokenRow, |key| {ordinal: key as u32}, {token: Vec<u8>});
fixture!(FactionAdvisorRow, |key| {faction: (key >> 32) as u64, ordinal: key as u32}, {token: Vec<u8>});
fixture!(RaidEquipmentRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {phase: i32, end_hour: Option<i32>, variants: Vec<u64>, amounts: Vec<i64>, nukes: i32, archetype_tokens: Vec<u32>, archetype_amounts: Vec<i64>});
fixture!(RaidSpeedInputRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {phase: i32, outcome: i32, end_hour: Option<i32>, wing_max_speed: Option<i64>, army_max_speed: Option<i64>, transport_speeds: Vec<i64>, transport_from_country: u8, transport_fallback: u8, ace_present: u8, ace_alive: u8, ace_speed: i64, ace_type_speed: i64, ace_wing_planes: Option<i32>, ace_owner: Option<u32>, wing_country: Option<u32>});
fixture!(StockpileArchetypeRow, |key| {canonical_slot: (key >> 32) as u32, archetype: key as u32}, {units: i32});
fixture!(RaidAirSiteRow, |key| {canonical_slot: (key >> 32) as u32, ordinal: key as u32}, {list: u8, base: u64, container_capacity_raw: Option<i64>});
fixture!(AceRangeInputRow, |key| {canonical_slot: (key >> 64) as u32, ace: key as u64}, {wing: u64, alive: u8, range: i64, type_range: i64, normalization: Recorded<Option<[Option<i64>; 2]>>, fuel_modifier: Recorded<Option<[Option<i64>; 1]>>, combat_modifiers_known: Recorded<bool>, speed_modifier_raw: Recorded<Option<i64>>, attack_modifier_raw: Recorded<Option<i64>>, defence_modifier_raw: Recorded<Option<i64>>, type_speed_modifier_raw: Recorded<Option<i64>>, type_combat_flags: Recorded<u8>});
fixture!(SliceReadRow, |key| {table: (key >> 32) as u32, owner: key as u32}, {sample_hour: i32});
impl Fixture for NavalPatrolMissionInputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            task_force: key as u64,
            target: Option::<u64>::sample(tick),
            target_live: Some((tick % 2) as u32),
            progress: Option::<i64>::sample(tick + 1),
            chance: Option::<i64>::sample(tick + 2),
            surface_detection: Option::<i64>::sample(tick + 3),
            carrier_detection: Option::<i64>::sample(tick + 4),
            local_detection_known: bool::sample(tick),
            local_detection: Option::<i64>::sample(tick + 5),
            weather_detection_known: bool::sample(tick + 1),
            weather_detection: Option::<i64>::sample(tick + 6),
            leader: u64::sample(tick),
            headquarters_known: bool::sample(tick),
            headquarters: std::sync::Arc::new(Vec::<u64>::sample(tick)),
            surface_visibility: Option::<i64>::sample(tick + 7),
            submarine_visibility: Option::<i64>::sample(tick + 8),
            formation_class: Option::<i32>::sample(tick),
            radar_cache: Option::<i64>::sample(tick + 9),
            air_cache: Option::<i64>::sample(tick + 10),
            submarine_detection: Option::<i64>::sample(tick + 11),
            carrier_submarine_detection: Option::<i64>::sample(tick + 12),
            mission_owner: Option::<u32>::sample(tick),
            targeted_spotting_known: bool::sample(tick),
            targeted_spotting: Option::<i64>::sample(tick + 13),
        }
    }
}
fixture!(NavalMissionTaskForceRow, |key| {task_force: key as u64}, {raiding_factor: i64, local_coordination: Option<i64>, local_raiding: Option<i64>, local_detection: Option<i64>, local_speed: Option<i64>, local_spotting: Option<i64>, local_night_spotting: Option<i64>, local_visibility: Option<i64>, weather_coordination: Option<i64>, weather_raiding: Option<i64>, weather_detection: Option<i64>, weather_speed: Option<i64>, weather_spotting: Option<i64>, weather_night_spotting: Option<i64>, weather_visibility: Option<i64>, leader: u64, headquarters: std::sync::Arc<Vec<u64>>, range_fleet: u64, range_begin: i32, range_end: i32, convoys: i32, range_base: i64, fuel: i64, fuel_requested: i64, underway: u8, replenishment: i32, replenishment_limit: i32, local_range: Option<i64>, local_range_factor: Option<i64>, weather_range: Option<i64>, weather_range_factor: Option<i64>, mission_owner: u32, raiding_spotting: Option<i64>, raiding_region: Option<u32>, raiding_targets: Option<[Option<bool>; 2]>, raiding_target_country: Option<u32>, raiding_target_convoys: Option<i32>, raiding_transport_invasion: Option<u8>, raiding_convoy_variant: Option<u64>, raiding_convoy_surface_detection: Option<i64>, raiding_convoy_sub_detection: Option<i64>, raiding_convoy_visibility: Option<i64>, raiding_convoy_maximum_speed: Option<i64>, raiding_source_speed: Option<i64>, raiding_source_terrain_class: Option<i32>});
impl Fixture for NavalMissionLedgerRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let ring = !tick.is_multiple_of(2);
        let mut task_forces = Vec::<u64>::sample(tick);
        if task_forces.is_empty() {
            task_forces.push(u64::sample(tick));
        }
        Self {
            owner: (key >> 32) as u32,
            region: key as u32,
            task_forces,
            ring,
            positive: if ring { i32::sample(tick) } else { 0 },
            total: if ring { i32::sample(tick + 1) } else { 0 },
        }
    }
}
impl Fixture for NavalMissionCountryRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: key as u32,
            coordination: Option::<i64>::sample(tick),
            detection: Option::<i64>::sample(tick + 1).into(),
            speed: Option::<i64>::sample(tick + 11).into(),
            spotting: Option::<i64>::sample(tick + 2).into(),
            night_spotting: Option::<i64>::sample(tick + 3).into(),
            visibility: Option::<i64>::sample(tick + 4).into(),
            raiding: Option::<i64>::sample(tick + 5),
            escort: Option::<i64>::sample(tick + 6),
            weather_reduction: Option::<i64>::sample(tick + 7),
            range: Option::<i64>::sample(tick + 8),
            range_factor: Option::<i64>::sample(tick + 9),
            underway: Option::<i64>::sample(tick + 10),
            hubs: Vec::<u32>::sample(tick),
            escort_tree: Recorded::Value(match tick % 3 {
                0 => None,
                1 => Some(Vec::new()),
                _ => Some(ReserveModifierBreakdownRow::fixture(1, tick).nodes),
            }),
        }
    }
}
impl Fixture for NavalMissionCommanderRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 72) as u32,
            reference: (key >> 8) as u64,
            source: key as u8,
            coordination: Option::sample(tick),
            detection: Option::sample(tick + 4).into(),
            speed: Option::sample(tick + 8).into(),
            spotting: Option::sample(tick + 5).into(),
            night_spotting: Option::sample(tick + 6).into(),
            visibility: Option::sample(tick + 7).into(),
            raiding: Option::sample(tick + 1),
            range: Option::sample(tick + 2),
            range_factor: Option::sample(tick + 3),
        }
    }
    fn largest_key() -> u128 {
        (u128::from(u32::MAX) << 72) | (u128::from(u64::MAX) << 8) | 4
    }
}
fixture!(FactionTheatreRow, |key| {faction: (key >> 32) as u64, ordinal: key as u32}, {icon: Vec<u8>, regions: Vec<u32>, country_count: u32, hidden_for: Vec<u32>, name: Recorded<Vec<u8>>, owner: Recorded<u32>, commander: Recorded<Option<u64>>, countries: Recorded<Vec<u32>>});
fixture!(AirRegionalMissionRow, |key| {wing: key as u64}, {ordinal: u32, active_order: Option<u64>});
sample_struct!(AirSupplySummary {
    severity: i32,
    needed: i64,
    supplied: i64,
    deficit: i64
});
fixture!(AirGroundCrewRow, |key| {country: (key >> 32) as u32, region: key as u32}, {enabled: Option<u32>, supply: Recorded<Option<AirSupplySummary>>, supply_severity: Recorded<Option<i32>>, supply_needed: Recorded<Option<i64>>, supply_supplied: Recorded<Option<i64>>, supply_deficit_extra: Recorded<Option<i64>>});
fixture!(AirTransferInputRow, |key| {wing: key as u64}, {base: u64, ordinal: Option<u32>, progress_raw: Option<i64>, distance_raw: Option<i64>});
fixture!(AirCountryRelationRow, |key| {country: (key >> 34) as u32, kind: ((key >> 32) & 3) as u8, ordinal: key as u32}, {other: u32});
fixture!(AirReferenceWitnessRow, |key| {handle: key as u64}, {base_kind: Option<i32>, state: Option<u32>, province: Option<u32>, host: Option<u64>, host_owner: Option<u32>, host_without_task_force: bool});
fixture!(AirMissionInputRow, |key| {wing: key as u64}, {coverage_raw: i64, reassignment_penalty_raw: i64, last_target_province: i32, reached_count: i32, reached_provinces: Vec<u32>, definition_map_flags: u64, cas_ratio_raw: Recorded<Option<i64>>, interception_alert_gate: Recorded<Option<u32>>, current_type: Recorded<Option<u32>>, effective_aircraft: Recorded<Option<i32>>, normalization: Recorded<Option<AirMissionNormalization>>, detection_stat_13: Recorded<Option<i64>>, detection_stat_14: Recorded<Option<i64>>, patrol_disruption_raw: Recorded<Option<i64>>, patrol_attack_raw: Recorded<Option<i64>>, patrol_defence_raw: Recorded<Option<i64>>, patrol_speed_raw: Recorded<Option<i64>>, patrol_wing_attack_raw: Recorded<Option<i64>>, patrol_wing_defence_raw: Recorded<Option<i64>>, patrol_night_applied_raw: Recorded<Option<i64>>});
fixture!(AirSortieBaseCountryRow, |key| {base: (key >> 32) as u64, country: key as u32}, {capacity_raw: i64, supply_raw: i64, fuel_demand_raw: Option<i64>, fuel_allocated_raw: Option<i64>});
fixture!(AirSortieModifierRow, |key| {scope: (key >> 64) as u8, subject: (key >> 32) as u32, region: key as u32}, {raw: i64});
fixture!(AirSortieSpecialBaseRow, |key| {base: key as u64}, {rocket_slots: Option<i32>, carrier_stance: Option<i32>, leader_modifier_raw: i64, task_force_combats: Option<i32>, task_force_region: Option<u32>});
fixture!(AirSortieModifierMetadataRow, |_key| {}, {flags: u32, feature_selector: u32, callback_kind: u8});
fixture!(NavalMineLevelRow, |key| {country: (key >> 32) as u32, region: key as u32}, {raw_mines: i64});
fixture!(AirNavalPatrolCombatRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {province: u32, region: u32});
impl Fixture for AirNavalPatrolHistoryRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 72) as u32,
            region: (key >> 40) as u32,
            level: (key >> 32) as u8,
            index: key as u32,
            enemy_ships_sunk_raw: i64::sample(tick),
            our_fighters_lost_raw: i64::sample(tick),
            scenario_channels: Recorded::<Option<[i64; 7]>>::sample(tick),
            menu: Recorded::<Option<[i64; AIR_MENU_EXTRA_CHANNELS]>>::sample(tick),
            window: Option::<AirHistoryWindow>::sample(tick),
        }
    }
    fn largest_key() -> u128 {
        (maximum_key::<Self>() & !(0xff_u128 << 32)) | (2_u128 << 32)
    }
}
fn strategic_action_fixture(tick: usize) -> StrategicRedeploymentInputs {
    match tick % 7 {
        0 => StrategicRedeploymentInputs {
            country_modifier: i64::MIN,
            ..Default::default()
        },
        1 => StrategicRedeploymentInputs::default(),
        2 => StrategicRedeploymentInputs {
            state_modifier: Some(i64::MAX),
            ..Default::default()
        },
        3 => StrategicRedeploymentInputs {
            state_modifier: Some(0),
            ..Default::default()
        },
        _ => StrategicRedeploymentInputs {
            country_modifier: 0,
            state_modifier: Some(0),
            friendly_state_modifier: Some(match tick % 7 {
                4 => 0,
                5 => i64::MIN,
                _ => i64::MAX,
            }),
        },
    }
}

impl Sample for TerrainSpeed {
    fn sample(tick: usize) -> Self {
        Self {
            terrain: [1, 256, i32::MAX][tick % 3],
            modifier: i64::sample(tick),
        }
    }
}
impl Sample for StrategicRedeploymentInputs {
    fn sample(tick: usize) -> Self {
        strategic_action_fixture(tick)
    }
}
impl Sample for ZeroSubjectInputs {
    fn sample(tick: usize) -> Self {
        match tick % 5 {
            0 => Self::default(),
            1 => Self {
                relation_type: Some(0x380a),
                parties: None,
            },
            2 => Self {
                relation_type: Some(0x30d1),
                parties: Some([Some(0), None]),
            },
            3 => Self {
                relation_type: Some(0x30d1),
                parties: Some([None, Some(0)]),
            },
            _ => Self {
                relation_type: Some(0x30d1),
                parties: Some([Some(u32::sample(tick).max(1)), Some(u32::sample(tick + 1))]),
            },
        }
    }
}
sample_struct!(LaunchNavalBase {
    base_province: u32,
    country: u32
});
impl Sample for LaunchNavalBases {
    fn sample(tick: usize) -> Self {
        match tick % 3 {
            0 => Self::Unavailable,
            1 => Self::NativeNull,
            _ => Self::Bases(Vec::<LaunchNavalBase>::sample(tick)),
        }
    }
}
sample_struct!(ReservedLaunchSources { sample_hour: PacketHour, naval_bases: LaunchNavalBases, owned_states: Option<Vec<u32>> });
sample_struct!(ReverseZeroVisibilityInputs { target: Option<u32>, war: Option<[u8; 2]>, subject: Option<ZeroSubjectInputs>, military_access: Option<u8> });
sample_struct!(ReservedReverseVisibilityInputs { sample_hour: PacketHour, cells: Vec<Option<ReverseZeroVisibilityInputs>> });
impl Sample for ReservedCountryIntelSample {
    fn sample(tick: usize) -> Self {
        Self {
            sample_hour: i32::sample(tick),
            vector_count: match tick % 3 {
                0 => 0,
                1 => 1,
                _ => i32::MAX as u32,
            },
        }
    }
}
sample_struct!(RailwayBuildingHealth { level: i16, healthy: i16, partial: Option<i64> });
sample_struct!(RailwayBuildingState { building: Option<RailwayBuildingHealth> });
sample_struct!(OrderedEnemyInputs { sample_hour: PacketHour, entries: Vec<u32> });

sample_struct!(RailwaySelectionInputs { levels: Option<Vec<i32>>, pair_count: Option<i32>, building_level: Option<Option<i16>> });

sample_struct!(ParadropTransportModifiers {
    country_term: i64,
    commander_term: Option<i64>
});
sample_struct!(DivisionOrderTransportInputs {
    naval_weight: Option<i64>,
    paradrop_eligible: Option<u8>,
    paradrop_modifiers: Option<ParadropTransportModifiers>
});
impl Fixture for DivisionMovementRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let reached = !tick.is_multiple_of(8);
        Self {
            transport_speed: Recorded::<Option<i64>>::sample(tick),
            division: key as u64,
            source: u32::sample(tick),
            target: if reached { 0 } else { u32::sample(tick) },
            numerator: i64::sample(tick),
            retreat: if reached { 0 } else { u8::sample(tick) },
            strategic_redeployment: if reached { u8::MAX } else { 0 },
            naval_transfer: u8::sample(tick),
            combat_count: u32::sample(tick),
            path_total: if reached { 2 } else { u32::sample(tick) },
            path: vec![u32::sample(tick), u32::sample(tick + 1)],
            alternate_total: u32::sample(tick),
            alternate_path: Vec::<u32>::sample(tick),
            combat_role: u8::sample(tick),
            combat_province: u32::sample(tick),
            map_entity: u8::sample(tick),
            withdrawal: Recorded::sample(tick),
            movement_priority: Recorded::sample(tick),
            order_transport: Recorded::sample(tick),
            combat_references: unit_combat_references_fixture(tick, u32::sample(tick)),
            fuel: Recorded::<Option<i64>>::sample(tick + 1),
            own_base_speed: Option::<Option<i64>>::sample(tick),
            queued_actions: Recorded::<Option<i32>>::sample(tick + 2),
            strategic_action: if reached {
                crate::rows::Recorded::Value(StrategicAction::Available(strategic_action_fixture(
                    tick,
                )))
            } else {
                crate::rows::Recorded::Value(StrategicAction::Unreached)
            },
        }
    }
}
sample_struct!(DivisionCamouflage { category: Option<u32> });
sample_struct!(DivisionAcclimatization {
    category: u32,
    weight: i64
});
sample_struct!(UnitSupplyDemand {
    count: i32,
    value: i64
});
sample_struct!(UnitSupplyOrigin {
    province: i32,
    requested: i64,
    received: i64
});
sample_struct!(UnitSupplyInputs { asked: i64, received: i64, from_state: i64,
    air_drops: i64, disruption: i64, division_demand: UnitSupplyDemand,
    airplane_demand: UnitSupplyDemand, ship_demand: UnitSupplyDemand,
    origins: Option<Vec<UnitSupplyOrigin>> });
sample_struct!(DivisionSupplyFooter { asked: i64, received: i64, from_state: i64,
    air_drops: i64, disruption: i64, division_demand: UnitSupplyDemand,
    airplane_demand: UnitSupplyDemand, ship_demand: UnitSupplyDemand,
    origins: Option<Vec<UnitSupplyOrigin>> });
fixture!(DivisionPanelStatsRow, |key| {division: key as u64}, {panel_stats: [Option<i64>; 22], suppression: Recorded<Option<i64>>, status_maxima: [Option<i64>; 2]});
fixture!(DivisionStatusRow, |key| {division: key as u64}, {entrenchment: i64, entrenchment_cap: i64, supply_ratio: Option<i64>, attrition_value: i64, attrition_threshold: i64, exercising: u8, has_order: u8, invalid_orders: u8, strength_factor: i64, model_number: i32, acclimatization_maximum: Option<i64>, acclimatization: Vec<DivisionAcclimatization>, move_flags: u8, border_combat_stats: Recorded<Option<[i64; 3]>>, naming: Recorded<Option<NameOperands>>, supply_grace_maximum: Recorded<i64>, supply_grace_remaining: Recorded<i64>, country_ordinal: Recorded<Option<u32>>, invalid_orders_reason: Recorded<Option<i32>>, supply: Recorded<Option<DivisionSupplyFooter>>, dig_in_factor: Recorded<Option<i64>>, name: Recorded<Option<Vec<u8>>>, attack: Recorded<Witnessed<DivisionAttackInputs>>, camouflage: Recorded<Option<DivisionCamouflage>>, attack_stats: Recorded<Option<DivisionAttackStats>>, weather_factor: Recorded<Option<i64>>, name_template: Recorded<Witnessed<u64>>, movement_stats: Recorded<Option<DivisionMovementStats>>, out_of_supply_days: Recorded<Option<i32>>, commander_modifiers: Recorded<Option<[Option<i64>; 5]>>, supply_ratio_exact: Recorded<Option<i64>>, supply_grace_exact: Recorded<Option<i64>>, experience: Recorded<Option<i64>>, pending_template: Recorded<Option<u64>>, officer_experience: Recorded<Option<i64>>, career_counter: Recorded<Option<i32>>, panel_source:Recorded<Option<[u64;3]>>, combat_movement: Recorded<Option<Option<i64>>>, organisation_exact: Recorded<Option<i64>>, maximum_organisation_exact: Recorded<Option<i64>>, commander_width_factor: Recorded<Option<i64>>});
fixture!(DivisionModelInputRow, |key| {division: key as u64}, {model_number: Option<i32>, camouflage: Option<DivisionCamouflage>, exercising: Option<u8>});
impl Fixture for DivisionEtaInputsRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let legs = |offset: usize| {
            (0..tick % 3)
                .map(|i| i64::sample(tick + offset + i))
                .collect()
        };
        Self {
            division: key as u64,
            sample_hour: i32::sample(tick),
            speed_modifier: i64::sample(tick),
            speed_base: i64::sample(tick + 1),
            speed_org: Option::<i64>::sample(tick),
            speed_fuel: Option::<i64>::sample(tick + 1),
            current_speed: i64::sample(tick + 1),
            terrain_speed: (0..tick % 3)
                .map(|i| TerrainSpeed {
                    terrain: [1, 256, 1581][i],
                    modifier: i64::sample(tick + i),
                })
                .collect(),
            fort_penalty: i64::sample(tick),
            snow_penalty: i64::sample(tick + 1),
            river_adjustment: i64::sample(tick + 2),
            river_leader: i64::sample(tick),
            class: u32::sample(tick),
            rail_country_modifier: Option::<i64>::sample(tick),
            rail_state_modifier: Option::<i64>::sample(tick + 1),
            rail_friendly_state_modifier: Option::<i64>::sample(tick + 2),
            leg_river_combined: legs(1),
            leg_transport_speed: if tick.is_multiple_of(2) {
                Vec::new()
            } else {
                legs(2)
            },
            leg_rail_speed: if tick.is_multiple_of(3) {
                Vec::new()
            } else {
                legs(3)
            },
        }
    }
}
sample_struct!(TemplateSubunit {
    token: u32,
    local: u8,
    count: i32
});
sample_struct!(TemplateGrids {
    regiments: Vec<TemplateSubunit>,
    support: Vec<TemplateSubunit>,
    regimental_support: Vec<TemplateSubunit>
});
sample_struct!(TemplateCell {
    token: u32,
    local: u8,
    occupied: bool
});
sample_struct!(TemplateGrid { columns: u32, rows: u32, cells: Vec<TemplateCell> });
sample_struct!(TemplateLayout {
    regiments: TemplateGrid,
    support: TemplateGrid,
    regimental_support: TemplateGrid
});
sample_struct!(TemplateSettings {
    roster_ordinal: u32, names_group: Option<Vec<u8>>,
    priority: i32, is_army_hq: u8, locked: u8, obsolete: u8,
    obsolete_change_date: i32, force_allow_recruiting: u8,
    allow_new_equipment: u8, allow_foreign_equipment: u8,
    parent: u64, creator_country: i32, origin_type: i32, fake_intel: u8,
    division_cap: Option<i32>, equipment_niche: u8,
    role: Option<u32>, ai_role: Option<u32>, ai_role_fallback: u32,
    total_manpower: i32, command_power_cost: i64, training_time: i32,
    forbidden_equipment: Option<Vec<u32>>
});
sample_struct!(GarrisonTemplateSelection { occupied: Option<u32>, state: Option<u32>, template: Option<u64> });
fixture!(CountryTemplateStateRow, |key| {country: key as u32}, {garrison_priority: Recorded<Option<i32>>,selected_staff: u64, next_hq_order: i32, lock_all: u8, garrisons: Recorded<Option<std::sync::Arc<Vec<GarrisonTemplateSelection>>>>});
fixture!(DivisionTemplateRow, |key| {template: key as u64}, {owner: u32, name: Vec<u8>, unit_type: i32, name_key: Recorded<Vec<u8>>});
fixture!(DivisionTemplateIconRow, |key| {template: key as u64}, {manpower_exile: Recorded<bool>, priority_unit: u32, priority_unit_name: Vec<u8>, custom_icon: i32, custom_icon_nato: u8, representative: u32, representative_local: u8, subunits: Vec<TemplateSubunit>, country: i32, foreign_country: i32, model_category: Option<u64>, model_entity: Vec<u8>, excluded_variants: Vec<u64>, model_unreadable: u8, formation_division_count: Recorded<Option<i32>>, display_name: Recorded<Option<TemplateDisplayName>>, grids: Recorded<TemplateGrids>, base_width: Recorded<Option<i64>>, speed_keys: Recorded<Option<Vec<u32>>>, layout: Recorded<Option<std::sync::Arc<TemplateLayout>>>, settings: Recorded<Option<std::sync::Arc<TemplateSettings>>>});
fixture!(LandCombatProgressRow, |key| {combat: key as u64}, {attacker_progress: RetiredI64, defender_progress: RetiredI64, direction_attacker: RetiredU64, direction_province: RetiredU32, attacker_manpower_loss:Recorded<i32>, defender_manpower_loss:Recorded<i32>, attacker_equipment_losses:Recorded<Vec<LandEquipmentLoss>>, defender_equipment_losses:Recorded<Vec<LandEquipmentLoss>>});
fixture!(ThreatSourceRow, |key| {ordinal: key as u32}, {country: u32, second_country: u32, current_raw: i64, final_raw: i64, daily_raw: i64, date: i32, reason: Vec<u8>});
fixture!(ThreatCountryAggregateRow, |key| {country_slot: key as u32}, {total_raw: i64});
fixture!(RelationOpinionRow, |key| {owner_country: (key >> 32) as u32, target_country: key as u32}, {opinion: i32});
fixture!(RelationTupleRow, |key| {owner_country: (key >> 80) as u32, target_country: (key >> 48) as u32, type_id: (key >> 16) as u32, ordinal: key as u16}, {first_country: u32, second_country: u32, start_hour: i32, end_hour: i32, cancel: u8, retired: u8});
fixture!(CountryIntelRow, |key| {owner_country: (key >> 32) as u32, target_index: key as u32}, {army: i64, air: i64, civilian: i64, navy: i64});

fn maximum_key<T: RowCodec>() -> u128 {
    if T::KEY_BITS == 128 {
        u128::MAX
    } else {
        (1_u128 << T::KEY_BITS) - 1
    }
}
fn game_score_error(known: bool) -> Vec<u8> {
    if known {
        Vec::new()
    } else {
        b"synthetic game score read failure".to_vec()
    }
}
impl Fixture for CountryGameScoreInputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let army_count = Option::<i32>::sample(tick);
        let land_count = Option::<i32>::sample(tick + 2);
        let fleets: Vec<_> = (0..tick % 3)
            .map(|_| GameScoreFleetCount {
                fleet: u64::sample(tick),
                task_forces: Some(i32::sample(tick)),
                error: Vec::new(),
            })
            .collect();
        let navy_count = (!tick.is_multiple_of(4)).then(|| {
            fleets.iter().fold(0_i32, |sum, fleet| {
                sum.wrapping_add(fleet.task_forces.unwrap())
            })
        });
        let air_state = (tick % 3) as u8;
        Self {
            country: key as u32,
            army_count,
            army_error: game_score_error(army_count.is_some()),
            navy_count,
            navy_error: game_score_error(navy_count.is_some()),
            land_count,
            land_error: game_score_error(land_count.is_some()),
            air_state,
            air_error: game_score_error(air_state != Self::AIR_UNKNOWN),
            first_sites: if air_state == Self::AIR_PRESENT {
                vec![GameScoreAirSite {
                    base: u64::sample(tick),
                    access: (tick % 3) as u8,
                    error: Vec::new(),
                }]
            } else {
                Vec::new()
            },
            fleets,
        }
    }
}
impl Fixture for AirWingGameScoreStatsRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let planes = Option::<i32>::sample(tick);
        let stat_raw = Option::<i64>::sample(tick + 1);
        let mission_index = Option::<i32>::sample(tick + 2).map(|value| i32::from(value as i8));
        Self {
            id: key as u64,
            planes,
            planes_error: game_score_error(planes.is_some()),
            stat_raw,
            stat_error: game_score_error(stat_raw.is_some()),
            mission_index,
            mission_error: game_score_error(mission_index.is_some()),
        }
    }
}
impl Fixture for GameScoreMultiplierRow {
    fn fixture(_key: u128, tick: usize) -> Self {
        let values: [Option<i64>; 6] =
            std::array::from_fn(|index| Option::<i64>::sample(tick + index));
        Self {
            id: 0,
            total_raw: values[0],
            total_error: game_score_error(values[0].is_some()),
            army_raw: values[1],
            army_error: game_score_error(values[1].is_some()),
            navy_raw: values[2],
            navy_error: game_score_error(values[2].is_some()),
            air_raw: values[3],
            air_error: game_score_error(values[3].is_some()),
            industry_raw: values[4],
            industry_error: game_score_error(values[4].is_some()),
            land_raw: values[5],
            land_error: game_score_error(values[5].is_some()),
        }
    }
}
fn rows<T: Fixture>(tick: usize) -> Vec<T> {
    let mut rows = vec![T::fixture(T::smallest_key(), tick)];
    if T::KEY_BITS != 0 {
        rows.push(T::fixture(T::smallest_key() + 1, tick + 1));
        if tick % 17 < 5 {
            rows.push(T::fixture(T::largest_key(), tick + 2));
        }
    }
    rows
}
pub fn sample_extended_table(table: TableId, tick: usize) -> Option<TableRows> {
    if let Some(rows) = super::sample_politics_table(table, tick) {
        return Some(rows);
    }
    Some(match table {
        TableId::WarOwnedRelations => {
            TableRows::WarOwnedRelations(rows::<WarOwnedRelationRow>(tick))
        }
        TableId::WarRelationGoals => TableRows::WarRelationGoals(rows::<WarRelationGoalRow>(tick)),
        TableId::WarOverviewCountries => {
            TableRows::WarOverviewCountries(rows::<WarOverviewCountryRow>(tick))
        }
        TableId::WarParticipation => TableRows::WarParticipation(rows::<WarParticipationRow>(tick)),
        TableId::WarCapturedProvinces => {
            TableRows::WarCapturedProvinces(rows::<WarCapturedProvinceRow>(tick))
        }
        TableId::WarLendLeaseHistories => {
            TableRows::WarLendLeaseHistories(rows::<WarLendLeaseHistoryRow>(tick))
        }
        TableId::WarNameRules => TableRows::WarNameRules(rows::<WarNameRuleRow>(tick)),
        TableId::Focus => TableRows::Focus(rows::<FocusRow>(tick)),
        TableId::CompletedFocuses => TableRows::CompletedFocuses(rows::<CompletedFocusRow>(tick)),
        TableId::ResearchTechnologies => {
            TableRows::ResearchTechnologies(rows::<ResearchTechnologyRow>(tick))
        }
        TableId::ResearchSlots => TableRows::ResearchSlots(rows::<ResearchSlotRow>(tick)),
        TableId::TechnologyDefinitions => {
            TableRows::TechnologyDefinitions(rows::<TechnologyDefinitionRow>(tick))
        }
        TableId::Politics => TableRows::Politics(rows::<PoliticsRow>(tick)),
        TableId::PoliticalParties => TableRows::PoliticalParties(rows::<PoliticalPartyRow>(tick)),
        TableId::ActiveIdeas => TableRows::ActiveIdeas(rows::<ActiveIdeaRow>(tick)),
        TableId::DecisionCounts => TableRows::DecisionCounts(rows::<DecisionCountsRow>(tick)),
        TableId::Decisions => TableRows::Decisions(rows::<DecisionRow>(tick)),
        TableId::DynamicModifiers => TableRows::DynamicModifiers(rows::<DynamicModifierRow>(tick)),
        TableId::PreferredTactics => TableRows::PreferredTactics(rows::<PreferredTacticRow>(tick)),
        TableId::CountryTacticUnlocks => {
            TableRows::CountryTacticUnlocks(rows::<CountryTacticUnlockRow>(tick))
        }
        TableId::DoctrineCountries => {
            TableRows::DoctrineCountries(rows::<DoctrineCountryRow>(tick))
        }
        TableId::DoctrineFolders => TableRows::DoctrineFolders(rows::<DoctrineFolderRow>(tick)),
        TableId::DoctrineTracks => TableRows::DoctrineTracks(rows::<DoctrineTrackRow>(tick)),
        TableId::DoctrineLeaderMastery => {
            TableRows::DoctrineLeaderMastery(rows::<DoctrineLeaderMasteryRow>(tick))
        }
        TableId::DoctrineCostReductions => {
            TableRows::DoctrineCostReductions(rows::<DoctrineCostReductionRow>(tick))
        }
        TableId::DoctrineMasteryGrants => {
            TableRows::DoctrineMasteryGrants(rows::<DoctrineMasteryGrantRow>(tick))
        }
        TableId::DoctrineEquipmentBonuses => {
            TableRows::DoctrineEquipmentBonuses(rows::<DoctrineEquipmentBonusRow>(tick))
        }
        TableId::TechSharingGroups => {
            TableRows::TechSharingGroups(rows::<TechSharingGroupRow>(tick))
        }
        TableId::TechSharingMembers => {
            TableRows::TechSharingMembers(rows::<TechSharingMemberRow>(tick))
        }
        TableId::PowerBalances => TableRows::PowerBalances(rows::<PowerBalanceRow>(tick)),
        TableId::PowerBalanceMembers => {
            TableRows::PowerBalanceMembers(rows::<PowerBalanceMemberRow>(tick))
        }
        TableId::PowerBalanceRanges => {
            TableRows::PowerBalanceRanges(rows::<PowerBalanceRangeRow>(tick))
        }
        TableId::PowerBalanceOverrides => {
            TableRows::PowerBalanceOverrides(rows::<PowerBalanceOverrideRow>(tick))
        }
        TableId::ProductionLines => TableRows::ProductionLines(rows::<ProductionLineRow>(tick)),
        TableId::CountryProductionModifiers => {
            TableRows::CountryProductionModifiers(rows::<CountryProductionModifiersRow>(tick))
        }
        TableId::ProductionLineModifiers => {
            TableRows::ProductionLineModifiers(rows::<ProductionLineModifiersRow>(tick))
        }
        TableId::Stockpiles => TableRows::Stockpiles(rows::<StockpileRow>(tick)),
        TableId::DivisionEquipment => {
            TableRows::DivisionEquipment(rows::<DivisionEquipmentRow>(tick))
        }
        TableId::VariantDefinitions => {
            TableRows::VariantDefinitions(rows::<VariantDefinitionRow>(tick))
        }
        TableId::VariantModules => TableRows::VariantModules(rows::<VariantModuleRow>(tick)),
        TableId::VariantStats => TableRows::VariantStats(rows::<VariantStatsRow>(tick)),
        TableId::VariantChanges => TableRows::VariantChanges(rows::<VariantChangeRow>(tick)),
        TableId::VariantBonusSnapshots => {
            TableRows::VariantBonusSnapshots(rows::<VariantBonusSnapshotRow>(tick))
        }
        TableId::DesignCountryModifiers => {
            TableRows::DesignCountryModifiers(rows::<DesignCountryModifierRow>(tick))
        }
        TableId::DesignOrganisationLiveness => {
            TableRows::DesignOrganisationLiveness(rows::<DesignOrganisationLivenessRow>(tick))
        }
        TableId::DesignObserverContext => {
            TableRows::DesignObserverContext(rows::<DesignObserverContextRow>(tick))
        }
        TableId::DesignCountryTemplates => {
            TableRows::DesignCountryTemplates(rows::<DesignCountryTemplatesRow>(tick))
        }
        TableId::VariantUpgrades => TableRows::VariantUpgrades(rows::<VariantUpgradeRow>(tick)),
        TableId::ArmyUpgradeGroups => {
            TableRows::ArmyUpgradeGroups(rows::<ArmyUpgradeGroupRow>(tick))
        }
        TableId::ArmyUpgradeRequests => {
            TableRows::ArmyUpgradeRequests(rows::<ArmyUpgradeRequestRow>(tick))
        }
        TableId::ArmyUpgradeDeliveries => {
            TableRows::ArmyUpgradeDeliveries(rows::<ArmyUpgradeDeliveryRow>(tick))
        }
        TableId::ArmyUpgradeProduced => {
            TableRows::ArmyUpgradeProduced(rows::<ArmyUpgradeProducedRow>(tick))
        }
        TableId::CountryFuel => TableRows::CountryFuel(rows::<CountryFuelRow>(tick)),
        TableId::CountryFuelConsumers => {
            TableRows::CountryFuelConsumers(rows::<CountryFuelConsumerRow>(tick))
        }
        TableId::CountryFactories => TableRows::CountryFactories(rows::<CountryFactoriesRow>(tick)),
        TableId::ResourceDefinitions => {
            TableRows::ResourceDefinitions(rows::<ResourceDefinitionRow>(tick))
        }
        TableId::CountryResources => TableRows::CountryResources(rows::<CountryResourceRow>(tick)),
        TableId::ResourceExchanges => {
            TableRows::ResourceExchanges(rows::<ResourceExchangeRow>(tick))
        }
        TableId::PurchaseContracts => {
            TableRows::PurchaseContracts(rows::<PurchaseContractRow>(tick))
        }
        TableId::ProvinceSupply => TableRows::ProvinceSupply(rows::<ProvinceSupplyRow>(tick)),
        TableId::HubMotorization => TableRows::HubMotorization(rows::<HubMotorizationRow>(tick)),
        TableId::DisruptedSupply => TableRows::DisruptedSupply(rows::<DisruptedSupplyRow>(tick)),
        TableId::ForeignHomebases => TableRows::ForeignHomebases(rows::<ForeignHomebaseRow>(tick)),
        TableId::CountryExperience => {
            TableRows::CountryExperience(rows::<CountryExperienceRow>(tick))
        }
        TableId::EventFeedHistory => TableRows::EventFeedHistory(rows::<EventFeedHistoryRow>(tick)),
        TableId::CountryExperienceGains => {
            TableRows::CountryExperienceGains(rows::<CountryExperienceGainRow>(tick))
        }
        TableId::StateBuildings => TableRows::StateBuildings(rows::<BuildingRow>(tick)),
        TableId::ProvinceBuildings => TableRows::ProvinceBuildings(rows::<BuildingRow>(tick)),
        TableId::StateBaseResources => {
            TableRows::StateBaseResources(rows::<StateBaseResourceRow>(tick))
        }
        TableId::BuildingConstruction => {
            TableRows::BuildingConstruction(rows::<BuildingConstructionRow>(tick))
        }
        TableId::RailwayConstruction => {
            TableRows::RailwayConstruction(rows::<RailwayConstructionRow>(tick))
        }
        TableId::OccupationSettings => {
            TableRows::OccupationSettings(rows::<OccupationSettingsRow>(tick))
        }
        TableId::StateOccupationLaws => {
            TableRows::StateOccupationLaws(rows::<StateOccupationLawRow>(tick))
        }
        TableId::StateGarrisons => TableRows::StateGarrisons(rows::<StateGarrisonRow>(tick)),
        TableId::StateAssignedManpower => {
            TableRows::StateAssignedManpower(rows::<StateAssignedManpowerRow>(tick))
        }
        TableId::CountryOccupationReserveManpower => TableRows::CountryOccupationReserveManpower(
            rows::<CountryOccupationReserveManpowerRow>(tick),
        ),
        TableId::StateResistance => TableRows::StateResistance(rows::<StateResistanceRow>(tick)),
        TableId::CountryCollaboration => {
            TableRows::CountryCollaboration(rows::<CountryCollaborationRow>(tick))
        }
        TableId::Characters => TableRows::Characters(rows::<CharacterRow>(tick)),
        TableId::CountryLeaderRoles => {
            TableRows::CountryLeaderRoles(rows::<CountryLeaderRoleRow>(tick))
        }
        TableId::AdvisorRoles => TableRows::AdvisorRoles(rows::<AdvisorRoleRow>(tick)),
        TableId::UnitLeaders => TableRows::UnitLeaders(rows::<UnitLeaderRow>(tick)),
        TableId::UnitLeaderTraits => TableRows::UnitLeaderTraits(rows::<UnitLeaderTraitRow>(tick)),
        TableId::UnitLeaderTraitExperience => {
            TableRows::UnitLeaderTraitExperience(rows::<UnitLeaderTraitExperienceRow>(tick))
        }
        TableId::ActiveAbilities => TableRows::ActiveAbilities(rows::<ActiveAbilityRow>(tick)),
        TableId::Capitulation => TableRows::Capitulation(rows::<CapitulationRow>(tick)),
        TableId::Subjects => TableRows::Subjects(rows::<SubjectRow>(tick)),
        TableId::Autonomy => TableRows::Autonomy(rows::<AutonomyRow>(tick)),
        TableId::IntelligenceAgencies => {
            TableRows::IntelligenceAgencies(rows::<IntelligenceAgencyRow>(tick))
        }
        TableId::OperativeMemberships => {
            TableRows::OperativeMemberships(rows::<OperativeMembershipRow>(tick))
        }
        TableId::Cryptology => TableRows::Cryptology(rows::<CryptologyRow>(tick)),
        TableId::IntelNetworks => TableRows::IntelNetworks(rows::<IntelNetworkRow>(tick)),
        TableId::IntelContributions => {
            TableRows::IntelContributions(rows::<IntelContributionRow>(tick))
        }
        TableId::IntelNetworkAuxiliaries => {
            TableRows::IntelNetworkAuxiliaries(rows::<IntelNetworkAuxiliaryRow>(tick))
        }
        TableId::IntelSources => TableRows::IntelSources(rows::<IntelSourceRow>(tick)),
        TableId::PropagandaEfforts => {
            TableRows::PropagandaEfforts(rows::<PropagandaEffortRow>(tick))
        }
        TableId::IntelNetworkSummaries => {
            TableRows::IntelNetworkSummaries(rows::<IntelNetworkSummaryRow>(tick))
        }
        TableId::ScriptFlags => TableRows::ScriptFlags(rows::<ScriptFlagRow>(tick)),
        TableId::GlobalSavedEventTargets => {
            TableRows::GlobalSavedEventTargets(rows::<GlobalSavedEventTargetRow>(tick))
        }
        TableId::EquipmentDestruction => {
            let mut rows = rows::<EquipmentDestructionRow>(tick);
            if (tick / 6).is_multiple_of(2) {
                rows.retain(|row| (row.cause as u8) <= EquipmentDestructionCause::AirNavalAa as u8);
                rows.insert(
                    0,
                    EquipmentDestructionRow {
                        coverage: EquipmentDestructionCoverage::SupplementalUnavailable.into(),
                        unavailable_reason: format!("synthetic optional hook failure {}", tick % 3)
                            .into(),
                        ..Default::default()
                    },
                );
            }
            TableRows::EquipmentDestruction(rows)
        }
        TableId::DivisionEquipmentDeletions => {
            TableRows::DivisionEquipmentDeletions(rows::<DivisionEquipmentDeletionRow>(tick))
        }
        TableId::AirLossEvents => TableRows::AirLossEvents(rows::<AirLossEventRow>(tick)),
        TableId::AirDailyRoster => TableRows::AirDailyRoster(rows::<AirDailyRosterRow>(tick)),
        TableId::RocketProductionOutput => {
            TableRows::RocketProductionOutput(rows::<RocketProductionOutputRow>(tick))
        }
        TableId::BuildingBombingDamage => {
            TableRows::BuildingBombingDamage(rows::<BuildingBombingDamageRow>(tick))
        }
        TableId::ScriptVariables => TableRows::ScriptVariables(rows::<ScriptVariablesRow>(tick)),
        TableId::Operations => TableRows::Operations(rows::<OperationRow>(tick)),
        TableId::AgencyUpgradeLevels => {
            TableRows::AgencyUpgradeLevels(rows::<AgencyUpgradeLevelRow>(tick))
        }
        TableId::OperationCompletions => {
            TableRows::OperationCompletions(rows::<OperationCompletionRow>(tick))
        }
        TableId::OperationAssignments => {
            TableRows::OperationAssignments(rows::<OperationAssignmentRow>(tick))
        }
        TableId::WingEquipment => TableRows::WingEquipment(rows::<WingEquipmentRow>(tick)),
        TableId::AirBases => TableRows::AirBases(rows::<AirBaseRow>(tick)),
        TableId::WingCumulative => TableRows::WingCumulative(rows::<WingCumulativeRow>(tick)),
        TableId::AirRegionHistory => TableRows::AirRegionHistory(rows::<AirRegionHistoryRow>(tick)),
        TableId::AirRegionDamage => TableRows::AirRegionDamage(rows::<AirRegionDamageRow>(tick)),
        TableId::AirRegionCombat => TableRows::AirRegionCombat(rows::<AirRegionCombatRow>(tick)),
        TableId::AirAces => TableRows::AirAces(rows::<AirAceRow>(tick)),
        TableId::AirStatistics => TableRows::AirStatistics(rows::<AirStatisticsRow>(tick)),
        TableId::SunkConvoys => TableRows::SunkConvoys(rows::<SunkConvoyRow>(tick)),
        TableId::RailCooldowns => TableRows::RailCooldowns(rows::<RailCooldownRow>(tick)),
        TableId::RailEdges => TableRows::RailEdges(rows::<RailEdgeRow>(tick)),
        TableId::LandTheatres => TableRows::LandTheatres(rows::<LandTheatreRow>(tick)),
        TableId::LandTheatreGroups => {
            TableRows::LandTheatreGroups(rows::<LandTheatreGroupRow>(tick))
        }
        TableId::LandTheatreMembers => {
            TableRows::LandTheatreMembers(rows::<LandTheatreMemberRow>(tick))
        }
        TableId::AirTheatres => TableRows::AirTheatres(rows::<AirTheatreRow>(tick)),
        TableId::AirTheatreGroups => TableRows::AirTheatreGroups(rows::<AirTheatreGroupRow>(tick)),
        TableId::AirTheatreMembers => {
            TableRows::AirTheatreMembers(rows::<AirTheatreMemberRow>(tick))
        }
        TableId::NavalResults => TableRows::NavalResults(rows::<NavalResultRow>(tick)),
        TableId::NavalResultSources => {
            TableRows::NavalResultSources(rows::<NavalResultSourceRow>(tick))
        }
        TableId::NavalResultMembers => {
            TableRows::NavalResultMembers(rows::<NavalResultMemberRow>(tick))
        }
        TableId::NavalResultCountries => {
            TableRows::NavalResultCountries(rows::<NavalResultCountryRow>(tick))
        }
        TableId::SunkShips => TableRows::SunkShips(rows::<SunkShipsRow>(tick)),
        TableId::RaidTooltipOperands => {
            TableRows::RaidTooltipOperands(rows::<RaidTooltipOperandRow>(tick))
        }
        TableId::CountryRetiredCharacters => {
            TableRows::CountryRetiredCharacters(rows::<CountryCharacterRow>(tick))
        }
        TableId::CharacterRoleTraits => {
            TableRows::CharacterRoleTraits(rows::<CharacterRoleTraitRow>(tick))
        }
        TableId::CharacterOperatives => {
            TableRows::CharacterOperatives(rows::<CharacterOperativeRow>(tick))
        }
        TableId::RaidIntelInputs => TableRows::RaidIntelInputs(rows::<RaidIntelInputRow>(tick)),
        TableId::RaidAirScores => TableRows::RaidAirScores(rows::<RaidAirScoreRow>(tick)),
        TableId::RaidFacilities => TableRows::RaidFacilities(rows::<RaidFacilityRow>(tick)),
        TableId::ProjectPrograms => TableRows::ProjectPrograms(rows::<ProjectProgramRow>(tick)),
        TableId::Projects => TableRows::Projects(rows::<ProjectRow>(tick)),
        TableId::ProjectHistory => TableRows::ProjectHistory(rows::<ProjectHistoryRow>(tick)),
        TableId::ProjectBreakthroughs => {
            TableRows::ProjectBreakthroughs(rows::<ProjectBreakthroughRow>(tick))
        }
        TableId::Nukes => TableRows::Nukes(rows::<NukeRow>(tick)),
        TableId::RaidCountries => TableRows::RaidCountries(rows::<RaidCountryRow>(tick)),
        TableId::Raids => TableRows::Raids(rows::<RaidRow>(tick)),
        TableId::RaidTargets => TableRows::RaidTargets(rows::<RaidTargetRow>(tick)),
        TableId::ResearchModifiers => {
            TableRows::ResearchModifiers(rows::<ResearchModifierRow>(tick))
        }
        TableId::ResearchInputs => TableRows::ResearchInputs(rows::<ResearchInputRow>(tick)),
        TableId::ResearchAppliedBonuses => {
            TableRows::ResearchAppliedBonuses(rows::<ResearchAppliedBonusRow>(tick))
        }
        TableId::NavalBases => TableRows::NavalBases(rows::<NavalBaseRow>(tick)),
        TableId::BattleSounds => TableRows::BattleSounds(rows::<BattleSoundRow>(tick)),
        TableId::PeaceConferences => TableRows::PeaceConferences(rows::<PeaceConferenceRow>(tick)),
        TableId::WeatherRegions => TableRows::WeatherRegions(rows::<WeatherRegionRow>(tick)),
        TableId::WeatherProvinces => TableRows::WeatherProvinces(rows::<WeatherProvinceRow>(tick)),
        TableId::OperationTokens => TableRows::OperationTokens(rows::<OperationTokensRow>(tick)),
        TableId::LexicalTokens => TableRows::LexicalTokens(rows::<LexicalTokenRow>(tick)),
        TableId::NavalSupremacy => TableRows::NavalSupremacy(rows::<NavalSupremacyRow>(tick)),
        TableId::RadarCoverage => TableRows::RadarCoverage(rows::<RadarCoverageRow>(tick)),
        TableId::SentVolunteers => TableRows::SentVolunteers(rows::<SentVolunteerRow>(tick)),
        TableId::LogisticsEquipment => {
            TableRows::LogisticsEquipment(rows::<LogisticsEquipmentRow>(tick))
        }
        TableId::IndustrialOrganisations => {
            TableRows::IndustrialOrganisations(rows::<IndustrialOrganisationRow>(tick))
        }
        TableId::IndustrialOrganisationHistory => {
            TableRows::IndustrialOrganisationHistory(rows::<IndustrialOrganisationHistoryRow>(tick))
        }
        TableId::IndustrialOrganisationPolicies => TableRows::IndustrialOrganisationPolicies(
            rows::<IndustrialOrganisationPoliciesRow>(tick),
        ),
        TableId::IndustrialOrganisationDetails => {
            TableRows::IndustrialOrganisationDetails(rows::<IndustrialOrganisationDetailsRow>(tick))
        }
        TableId::ProductionLineOutput => {
            TableRows::ProductionLineOutput(rows::<ProductionLineOutputRow>(tick))
        }
        TableId::CombatLogEquipment => {
            TableRows::CombatLogEquipment(rows::<CombatLogEquipmentRow>(tick))
        }
        TableId::CombatLogState => TableRows::CombatLogState(rows::<CombatLogStateRow>(tick)),
        TableId::CombatLogObservers => {
            TableRows::CombatLogObservers(rows::<CombatLogObserverRow>(tick))
        }
        TableId::CombatLogBattleSides => {
            TableRows::CombatLogBattleSides(rows::<CombatLogBattleSideRow>(tick))
        }
        TableId::CombatLogBattleEquipment => {
            TableRows::CombatLogBattleEquipment(rows::<CombatLogBattleEquipmentRow>(tick))
        }
        TableId::CombatLogBattleModifiers => {
            TableRows::CombatLogBattleModifiers(rows::<CombatLogBattleModifierRow>(tick))
        }
        TableId::CombatLogBattleCountries => {
            TableRows::CombatLogBattleCountries(rows::<CombatLogBattleCountryRow>(tick))
        }

        TableId::CombatLogGroups => TableRows::CombatLogGroups(rows::<CombatLogGroupRow>(tick)),
        TableId::CombatLogMonths => TableRows::CombatLogMonths(rows::<CombatLogMonthRow>(tick)),
        TableId::CombatLogManpower => {
            TableRows::CombatLogManpower(rows::<CombatLogManpowerRow>(tick))
        }
        TableId::CombatLogTemplates => {
            TableRows::CombatLogTemplates(rows::<CombatLogTemplateRow>(tick))
        }
        TableId::CombatLogReferences => {
            TableRows::CombatLogReferences(rows::<CombatLogReferenceRow>(tick))
        }
        TableId::CombatLogBattles => TableRows::CombatLogBattles(rows::<CombatLogBattleRow>(tick)),
        TableId::CombatLogEnemyEquipment => {
            TableRows::CombatLogEnemyEquipment(rows::<CombatLogEnemyEquipmentRow>(tick))
        }

        TableId::LendLeaseEquipment => {
            TableRows::LendLeaseEquipment(rows::<LendLeaseEquipmentRow>(tick))
        }
        TableId::DivisionCensus => TableRows::DivisionCensus(rows::<DivisionCensusRow>(tick)),
        TableId::ConstructionLineOutput => {
            TableRows::ConstructionLineOutput(rows::<ConstructionLineOutputRow>(tick))
        }
        TableId::PeaceBids => TableRows::PeaceBids(rows::<PeaceSettlementRow>(tick)),
        TableId::PeaceSettlements => TableRows::PeaceSettlements(rows::<PeaceSettlementRow>(tick)),
        TableId::NuclearStrikes => TableRows::NuclearStrikes(rows::<NuclearStrikeRow>(tick)),
        TableId::LandCombatDivisionModifiers => {
            TableRows::LandCombatDivisionModifiers(rows::<LandCombatDivisionModifierRow>(tick))
        }
        TableId::FuelDemandParts => TableRows::FuelDemandParts(rows::<FuelDemandPartRow>(tick)),
        TableId::MapEvents => TableRows::MapEvents(rows::<MapEventsRow>(tick)),
        TableId::OwnerTransportSpeeds => {
            TableRows::OwnerTransportSpeeds(rows::<OwnerTransportSpeedRow>(tick))
        }
        TableId::ReserveModifierTrees => {
            TableRows::ReserveModifierTrees(rows::<ReserveModifierTreeRow>(tick))
        }
        TableId::OperativeMissions => {
            TableRows::OperativeMissions(rows::<OperativeMissionRow>(tick))
        }
        TableId::AgencyOperationBuffers => {
            TableRows::AgencyOperationBuffers(rows::<AgencyOperationBuffersRow>(tick))
        }
        TableId::StrategicOperatives => {
            TableRows::StrategicOperatives(rows::<StrategicOperativeRow>(tick))
        }
        TableId::ResourceOrigins => TableRows::ResourceOrigins(rows::<ResourceOriginRow>(tick)),
        TableId::ResourceRights => TableRows::ResourceRights(rows::<ResourceRightRow>(tick)),
        TableId::CountryOilFactors => {
            TableRows::CountryOilFactors(rows::<CountryOilFactorRow>(tick))
        }
        TableId::StateTemporaryResources => {
            TableRows::StateTemporaryResources(rows::<StateTemporaryResourceRow>(tick))
        }
        TableId::ConvoyClients => TableRows::ConvoyClients(rows::<ConvoyClientRow>(tick)),
        TableId::TradeRoutes => TableRows::TradeRoutes(rows::<TradeRouteRow>(tick)),
        TableId::CountryWargoals => TableRows::CountryWargoals(rows::<CountryWargoalRow>(tick)),
        TableId::WargoalTargets => TableRows::WargoalTargets(rows::<WargoalTargetRow>(tick)),
        TableId::WargoalJustifications => {
            TableRows::WargoalJustifications(rows::<WargoalJustificationRow>(tick))
        }
        TableId::DivisionPanelSources => {
            TableRows::DivisionPanelSources(rows::<SharedDivisionPanelSourceRow>(tick))
        }
        TableId::DivisionHistory => {
            TableRows::DivisionHistory(rows::<SharedDivisionHistoryRow>(tick))
        }
        TableId::CountryMarket => {
            let row = CountryMarketRow {
                inventory: MarketPool {
                    allow_zero: bool::sample(tick),
                    items: vec![MarketEquipment {
                        variant: u64::sample(tick),
                        amount_raw: i64::sample(tick + 1),
                    }],
                }
                .into(),
                prices: vec![MarketPriceLevel {
                    variant: u64::sample(tick + 1),
                    level: i32::sample(tick),
                }]
                .into(),
                subsidies: vec![MarketSubsidy {
                    cic_raw: i64::sample(tick),
                    archetype: u32::sample(tick + 1),
                    kind: (tick % 3) as u8,
                    targets: Vec::<u32>::sample(tick),
                    trigger: Vec::<u8>::sample(tick + 1),
                }]
                .into(),
                subsidy_total_raw: i64::sample(tick),
                auto_send_market_access: bool::sample(tick + 1),
                ..Default::default()
            };
            TableRows::CountryMarket(
                [0, 1, u32::MAX]
                    .into_iter()
                    .take(if tick % 17 < 5 { 3 } else { 2 })
                    .map(|country| CountryMarketRow {
                        country,
                        ..row.clone()
                    })
                    .collect(),
            )
        }
        TableId::MarketContractTerms => {
            let pool = |sample| {
                SharedMarketPool::from(MarketPool {
                    allow_zero: bool::sample(sample),
                    items: vec![MarketEquipment {
                        variant: u64::sample(sample),
                        amount_raw: i64::sample(sample + 1),
                    }],
                })
            };
            let row = MarketContractTermsRow {
                buyer: u32::sample(tick),
                speed: i32::sample(tick),
                equipment: pool(tick),
                prices: pool(tick + 1),
                price_levels: vec![MarketPriceLevel {
                    variant: u64::sample(tick),
                    level: i32::sample(tick + 1),
                }]
                .into(),
                subsidies: vec![MarketSubsidy {
                    cic_raw: i64::sample(tick + 1),
                    archetype: u32::sample(tick),
                    kind: (tick % 3) as u8,
                    targets: Vec::<u32>::sample(tick + 1),
                    trigger: Vec::<u8>::sample(tick),
                }]
                .into(),
                runtime: MarketContractRuntime {
                    route_equipment: pool(tick + 2),
                    route_length: i32::sample(tick),
                    route_type: u8::sample(tick),
                    route_request: i32::sample(tick + 1),
                    route_sunk_convoys_raw: i64::sample(tick),
                    period_days: i32::sample(tick + 2),
                    meta_cic_raw: i64::sample(tick + 1),
                    meta_convoys_raw: i64::sample(tick + 2),
                    meta_efficiency_raw: i64::sample(tick),
                    history_factory_raw: i64::sample(tick + 1),
                    history_equipment_raw: i64::sample(tick + 2),
                    history_collected_raw: i64::sample(tick),
                },
                variables: vec![MarketVariable {
                    name: Vec::<u8>::sample(tick),
                    amount_raw: i64::sample(tick + 1),
                }]
                .into(),
                variable_state: u64::sample(tick),
                ..Default::default()
            };
            let last = (u32::MAX as u128) << 72 | (u64::MAX as u128) << 8 | 1;
            TableRows::MarketContractTerms(
                [0_u128, 1, last]
                    .into_iter()
                    .take(if tick % 17 < 5 { 3 } else { 2 })
                    .map(|key| MarketContractTermsRow {
                        country: (key >> 72) as u32,
                        id: (key >> 8) as u64,
                        kind: key as u8,
                        ..row.clone()
                    })
                    .collect(),
            )
        }
        TableId::LendLeaseAgreements => {
            let pool = |sample| {
                SharedMarketPool::from(MarketPool {
                    allow_zero: bool::sample(sample),
                    items: vec![MarketEquipment {
                        variant: u64::sample(sample),
                        amount_raw: i64::sample(sample + 1),
                    }],
                })
            };
            let row = LendLeaseAgreementRow {
                receiver: u32::sample(tick),
                revision: u64::sample(tick),
                active_since: i32::sample(tick),
                last_delivery: i32::sample(tick + 1),
                monthly: pool(tick),
                percentage: pool(tick + 1),
                once: pool(tick + 2),
                delivered: pool(tick + 3),
                sunk: pool(tick + 4),
                fuel_daily_raw: i64::sample(tick),
                fuel_percentage_raw: i64::sample(tick + 1),
                fuel_sent_raw: i64::sample(tick + 2),
                fuel_sunk_raw: i64::sample(tick + 3),
                last_fuel_delivered_raw: i64::sample(tick + 4),
                ..Default::default()
            };
            TableRows::LendLeaseAgreements(
                [0_u128, 1, maximum_key::<LendLeaseAgreementRow>()]
                    .into_iter()
                    .take(if tick % 17 < 5 { 3 } else { 2 })
                    .map(|key| LendLeaseAgreementRow {
                        country: (key >> 64) as u32,
                        id: key as u64,
                        ..row.clone()
                    })
                    .collect(),
            )
        }
        TableId::MarketEvents => {
            let kinds = [0, 2, 3, 4, 5, 6, 10, 11, 12, 20, 21];
            let row = MarketEventRow {
                occurrence: u64::sample(tick),
                hour: i32::sample(tick),
                agreement: u64::sample(tick + 1),
                country: u32::sample(tick),
                receiver: u32::sample(tick + 1),
                kind: kinds[tick % kinds.len()],
                mode: (tick % 3) as u8,
                ordinal: u32::sample(tick + 2),
                variant: u64::sample(tick + 2),
                amount_raw: i64::sample(tick),
                auxiliary_raw: i64::sample(tick + 1),
                total_raw: i64::sample(tick + 2),
                lost_total_raw: i64::sample(tick + 3),
                date: i32::sample(tick + 2),
                flags: u8::sample(tick),
                ..Default::default()
            };
            TableRows::MarketEvents(
                [0, 1, u64::MAX]
                    .into_iter()
                    .take(if tick % 17 < 5 { 3 } else { 2 })
                    .map(|id| MarketEventRow { id, ..row })
                    .collect(),
            )
        }
        TableId::RegionalConvoyEfficiency => {
            TableRows::RegionalConvoyEfficiency(rows::<RegionalConvoyEfficiencyRow>(tick))
        }
        TableId::FactionRules => TableRows::FactionRules(rows::<FactionRuleRow>(tick)),
        TableId::FactionPrograms => TableRows::FactionPrograms(rows::<FactionProgramRow>(tick)),
        TableId::FactionMinimumUpgrades => {
            TableRows::FactionMinimumUpgrades(rows::<FactionMinimumUpgradeRow>(tick))
        }
        TableId::FactionDoctrine => TableRows::FactionDoctrine(rows::<FactionDoctrineRow>(tick)),
        TableId::FactionProjection => {
            TableRows::FactionProjection(rows::<FactionProjectionRow>(tick))
        }
        TableId::FactionAdvisorModifiers => {
            TableRows::FactionAdvisorModifiers(rows::<FactionAdvisorModifierRow>(tick))
        }
        TableId::FactionResources => TableRows::FactionResources(rows::<FactionResourceRow>(tick)),
        TableId::FactionResearch => TableRows::FactionResearch(rows::<FactionResearchRow>(tick)),
        TableId::FactionAccounts => TableRows::FactionAccounts(rows::<FactionAccountRow>(tick)),
        TableId::FactionVariables => TableRows::FactionVariables(rows::<FactionVariableRow>(tick)),
        TableId::FactionUnlocks => TableRows::FactionUnlocks(rows::<FactionUnlockRow>(tick)),
        TableId::FactionManpower => TableRows::FactionManpower(rows::<FactionManpowerRow>(tick)),
        TableId::FactionGoalStatus => {
            TableRows::FactionGoalStatus(rows::<FactionGoalStatusRow>(tick))
        }
        TableId::FactionGoalEntries => {
            TableRows::FactionGoalEntries(rows::<FactionGoalEntryRow>(tick))
        }
        TableId::NavalAccidentReports => {
            TableRows::NavalAccidentReports(rows::<NavalAccidentReportRow>(tick))
        }
        TableId::NavalDominanceTaskForceRanges => {
            TableRows::NavalDominanceTaskForceRanges(rows::<NavalMissionTaskForceRow>(tick))
        }
        TableId::NavalPatrolMissionInputs => {
            TableRows::NavalPatrolMissionInputs(rows::<NavalPatrolMissionInputRow>(tick))
        }
        TableId::NavalRegionalMines => {
            TableRows::NavalRegionalMines(rows::<NavalRegionalMineRow>(tick))
        }
        TableId::NavalRepairCountries => {
            TableRows::NavalRepairCountries(rows::<NavalRepairCountryRow>(tick))
        }
        TableId::NavalRepairFleets => {
            TableRows::NavalRepairFleets(rows::<NavalRepairFleetRow>(tick))
        }
        TableId::NavalRepairTaskForces => {
            TableRows::NavalRepairTaskForces(rows::<NavalRepairTaskForceRow>(tick))
        }
        TableId::NavalTaskForceComposition => {
            TableRows::NavalTaskForceComposition(rows::<NavalTaskForceCompositionRow>(tick))
        }
        TableId::NavalRepairBases => TableRows::NavalRepairBases(rows::<NavalRepairBaseRow>(tick)),
        TableId::NavalRepairQueue => TableRows::NavalRepairQueue(rows::<NavalRepairQueueRow>(tick)),
        TableId::NavalRefitShips => TableRows::NavalRefitShips(rows::<NavalRefitShipRow>(tick)),
        TableId::NavalRepairChildren => {
            TableRows::NavalRepairChildren(rows::<NavalRepairChildRow>(tick))
        }
        TableId::NavalRefitProgress => {
            TableRows::NavalRefitProgress(rows::<NavalRefitProgressRow>(tick))
        }
        TableId::MovementStateTerms => {
            TableRows::MovementStateTerms(rows::<MovementStateTermRow>(tick))
        }
        TableId::CoreSupplyAccess => TableRows::CoreSupplyAccess(rows::<CoreSupplyAccessRow>(tick)),
        TableId::CountryUnitSpeedEntries => {
            TableRows::CountryUnitSpeedEntries(rows::<CountryUnitSpeedEntryRow>(tick))
        }
        TableId::RiverRelations => TableRows::RiverRelations(rows::<RiverRelationRow>(tick)),
        TableId::DivisionTransportSpeeds => {
            TableRows::DivisionTransportSpeeds(rows::<OwnerTransportSpeedRow>(tick))
        }
        TableId::AirGroundCrews => TableRows::AirGroundCrews(rows::<AirGroundCrewRow>(tick)),
        TableId::AirTransferInputs => {
            TableRows::AirTransferInputs(rows::<AirTransferInputRow>(tick))
        }
        TableId::DivisionLandInputs => {
            TableRows::DivisionLandInputs(rows::<DivisionLandInputsRow>(tick))
        }
        TableId::LandCombatResults => {
            TableRows::LandCombatResults(rows::<LandCombatResultsRow>(tick))
        }
        TableId::DestroyedDivisions => {
            TableRows::DestroyedDivisions(rows::<DestroyedDivisionsRow>(tick))
        }
        TableId::LandEventCoverage => {
            TableRows::LandEventCoverage(rows::<LandEventCoverageRow>(tick))
        }
        TableId::VolunteerHosts => TableRows::VolunteerHosts(rows::<VolunteerHostRow>(tick)),
        TableId::TheatreAreas => TableRows::TheatreAreas(rows::<TheatreAreaRow>(tick)),
        TableId::StationedUnits => TableRows::StationedUnits(rows::<StationedUnitRow>(tick)),
        TableId::UnitIntel => TableRows::UnitIntel(rows::<UnitIntelRow>(tick)),
        TableId::AirBaseRegistrations => {
            TableRows::AirBaseRegistrations(rows::<AirBaseRegistrationRow>(tick))
        }
        TableId::ProductionLicenses => {
            TableRows::ProductionLicenses(rows::<ProductionLicenseRow>(tick))
        }
        TableId::ProductionLicenseRecords => {
            TableRows::ProductionLicenseRecords(rows::<ProductionLicenseRecordRow>(tick))
        }
        TableId::DeploymentConveyors => {
            TableRows::DeploymentConveyors(rows::<DeploymentConveyorRow>(tick))
        }
        TableId::DeploymentLines => TableRows::DeploymentLines(rows::<DeploymentLineRow>(tick)),
        TableId::DeploymentManpower => {
            TableRows::DeploymentManpower(rows::<DeploymentManpowerRow>(tick))
        }
        TableId::DeploymentEquipment => {
            TableRows::DeploymentEquipment(rows::<DeploymentEquipmentRow>(tick))
        }
        TableId::WorldTension => TableRows::WorldTension(rows::<WorldTensionRow>(tick)),
        TableId::ProvinceNames => TableRows::ProvinceNames(rows::<ProvinceNameRow>(tick)),
        TableId::StateFlags => TableRows::StateFlags(rows::<StateFlagRow>(tick)),
        TableId::ArmyOrderPlans => TableRows::ArmyOrderPlans(rows::<ArmyOrderPlanRow>(tick)),
        TableId::OrdersGroupPlans => TableRows::OrdersGroupPlans(rows::<OrdersGroupPlanRow>(tick)),
        TableId::Fronts => TableRows::Fronts(rows::<FrontRow>(tick)),
        TableId::CountryAppearance => {
            TableRows::CountryAppearance(rows::<CountryAppearanceRow>(tick))
        }
        TableId::ArmyIconInputs => TableRows::ArmyIconInputs(rows::<ArmyIconInputRow>(tick)),
        TableId::ProvinceUnitOrder => {
            TableRows::ProvinceUnitOrder(rows::<ProvinceUnitOrderRow>(tick))
        }
        TableId::CharacterPortraits => {
            TableRows::CharacterPortraits(rows::<CharacterPortraitRow>(tick))
        }
        TableId::LeaderPortraits => TableRows::LeaderPortraits(rows::<LeaderPortraitRow>(tick)),
        TableId::TaskForceCounterInputs => {
            TableRows::TaskForceCounterInputs(rows::<TaskForceCounterInputRow>(tick))
        }
        TableId::RailwayGunCounterInputs => {
            TableRows::RailwayGunCounterInputs(rows::<RailwayGunCounterInputRow>(tick))
        }
        TableId::TaskForceMapMotion => {
            TableRows::TaskForceMapMotion(rows::<TaskForceMapMotionRow>(tick))
        }
        TableId::FloatingHarborDeployments => {
            TableRows::FloatingHarborDeployments(rows::<FloatingHarborDeploymentRow>(tick))
        }
        TableId::FacilityMapFlags => TableRows::FacilityMapFlags(rows::<FacilityMapFlagRow>(tick)),
        TableId::NavalProvinceCombats => {
            TableRows::NavalProvinceCombats(rows::<NavalProvinceCombatRow>(tick))
        }
        TableId::NavalCombatTransfers => {
            TableRows::NavalCombatTransfers(rows::<NavalCombatTransferRow>(tick))
        }
        TableId::NavalTransferUnits => {
            TableRows::NavalTransferUnits(rows::<NavalTransferUnitRow>(tick))
        }
        TableId::TaskForceMissions => {
            TableRows::TaskForceMissions(rows::<TaskForceMissionRow>(tick))
        }
        TableId::NavalCombatAirBaseWingHandles => {
            TableRows::NavalCombatAirBaseWingHandles(rows::<NavalCombatAirBaseWingHandlesRow>(tick))
        }
        TableId::FleetCounterInputs => {
            TableRows::FleetCounterInputs(rows::<FleetCounterInputRow>(tick))
        }
        TableId::CountryCounterInputs => {
            TableRows::CountryCounterInputs(rows::<CountryCounterInputRow>(tick))
        }
        TableId::OrdersGroupCounterInputs => {
            TableRows::OrdersGroupCounterInputs(rows::<OrdersGroupCounterInputRow>(tick))
        }
        TableId::LeaderPortraitPaths => {
            TableRows::LeaderPortraitPaths(rows::<LeaderPortraitPathRow>(tick))
        }
        TableId::ShipModels => TableRows::ShipModels(rows::<ShipModelRow>(tick)),
        TableId::SupplyNodes => TableRows::SupplyNodes(rows::<SupplyNodeRow>(tick)),
        TableId::LandCombatDetails => {
            TableRows::LandCombatDetails(rows::<LandCombatDetailsRow>(tick))
        }
        TableId::LandCombatDivisions => {
            TableRows::LandCombatDivisions(rows::<LandCombatDivisionRow>(tick))
        }
        TableId::LandCombatModifiers => {
            TableRows::LandCombatModifiers(rows::<LandCombatModifierRow>(tick))
        }
        TableId::NavalCombatDetails => {
            TableRows::NavalCombatDetails(rows::<NavalCombatDetailsRow>(tick))
        }
        TableId::NavalCombatShips => TableRows::NavalCombatShips(rows::<NavalCombatShipRow>(tick)),
        TableId::NavalCombatResultDetails => {
            TableRows::NavalCombatResultDetails(rows::<NavalCombatResultDetailsRow>(tick))
        }
        TableId::TrainInventory => TableRows::TrainInventory(rows::<TrainInventoryRow>(tick)),
        TableId::LocalPlayer => TableRows::LocalPlayer(rows::<LocalPlayerRow>(tick)),
        TableId::LandUnitOrder => TableRows::LandUnitOrder(rows::<LandUnitOrderRow>(tick)),
        TableId::SimulationFeatures => {
            TableRows::SimulationFeatures(rows::<SimulationFeatureRow>(tick))
        }
        TableId::CountryVisibility => {
            TableRows::CountryVisibility(rows::<CountryVisibilityRow>(tick))
        }
        TableId::StateVisibility => TableRows::StateVisibility(rows::<StateVisibilityRow>(tick)),
        TableId::RelationLinks => TableRows::RelationLinks(rows::<RelationLinkRow>(tick)),
        TableId::SliceReads => TableRows::SliceReads(rows::<SliceReadRow>(tick)),
        TableId::NavalMissionTaskForces => {
            TableRows::NavalMissionTaskForces(rows::<NavalMissionTaskForceRow>(tick))
        }
        TableId::NavalMissionLedgers => {
            TableRows::NavalMissionLedgers(rows::<NavalMissionLedgerRow>(tick))
        }
        TableId::NavalMissionCountries => {
            TableRows::NavalMissionCountries(rows::<NavalMissionCountryRow>(tick))
        }
        TableId::NavalMissionCommanders => {
            TableRows::NavalMissionCommanders(rows::<NavalMissionCommanderRow>(tick))
        }
        TableId::FactionTheatres => TableRows::FactionTheatres(rows::<FactionTheatreRow>(tick)),
        TableId::AirRegionalMissions => {
            TableRows::AirRegionalMissions(rows::<AirRegionalMissionRow>(tick))
        }
        TableId::AirCountryRelations => {
            TableRows::AirCountryRelations(rows::<AirCountryRelationRow>(tick))
        }
        TableId::AirReferenceWitnesses => {
            TableRows::AirReferenceWitnesses(rows::<AirReferenceWitnessRow>(tick))
        }
        TableId::NavalMineLevels => TableRows::NavalMineLevels(rows::<NavalMineLevelRow>(tick)),
        TableId::AirNavalPatrolCombats => {
            TableRows::AirNavalPatrolCombats(rows::<AirNavalPatrolCombatRow>(tick))
        }
        TableId::AirNavalPatrolHistories => {
            TableRows::AirNavalPatrolHistories(rows::<AirNavalPatrolHistoryRow>(tick))
        }
        TableId::AirMissionInputs => TableRows::AirMissionInputs(rows::<AirMissionInputRow>(tick)),
        TableId::AirSortieBaseCountries => {
            TableRows::AirSortieBaseCountries(rows::<AirSortieBaseCountryRow>(tick))
        }
        TableId::AirSortieModifiers => {
            TableRows::AirSortieModifiers(rows::<AirSortieModifierRow>(tick))
        }
        TableId::AirSortieSpecialBases => {
            TableRows::AirSortieSpecialBases(rows::<AirSortieSpecialBaseRow>(tick))
        }
        TableId::AirSortieModifierMetadata => {
            TableRows::AirSortieModifierMetadata(rows::<AirSortieModifierMetadataRow>(tick))
        }
        TableId::RaidCooldowns => TableRows::RaidCooldowns(rows::<RaidCooldownRow>(tick)),
        TableId::RaidPathDistances => {
            TableRows::RaidPathDistances(rows::<RaidPathDistanceRow>(tick))
        }
        TableId::StateResourceInputs => {
            TableRows::StateResourceInputs(rows::<StateResourceInputRow>(tick))
        }
        TableId::StateBuildingInputs => {
            TableRows::StateBuildingInputs(rows::<StateBuildingInputRow>(tick))
        }
        TableId::StateResources => TableRows::StateResources(rows::<StateResourceRow>(tick)),
        TableId::StateDynamicModifiers => {
            TableRows::StateDynamicModifiers(rows::<StateDynamicModifierRow>(tick))
        }
        TableId::CountryBuildingResources => {
            TableRows::CountryBuildingResources(rows::<CountryBuildingResourceRow>(tick))
        }
        TableId::RaidCountryInputs => {
            TableRows::RaidCountryInputs(rows::<RaidCountryInputRow>(tick))
        }
        TableId::CountryCommandPower => {
            TableRows::CountryCommandPower(rows::<CountryCommandPowerRow>(tick))
        }
        TableId::DivisionRetreatBans => {
            TableRows::DivisionRetreatBans(rows::<DivisionRetreatBanRow>(tick))
        }
        TableId::StateCores => TableRows::StateCores(rows::<StateCoreRow>(tick)),
        TableId::RaidEquipment => TableRows::RaidEquipment(rows::<RaidEquipmentRow>(tick)),
        TableId::InvasionPreparations => {
            TableRows::InvasionPreparations(rows::<InvasionPreparationRow>(tick))
        }
        TableId::CountryCharacters => {
            TableRows::CountryCharacters(rows::<CountryCharacterRow>(tick))
        }
        TableId::PartyLeaders => TableRows::PartyLeaders(rows::<PartyLeaderRow>(tick)),
        TableId::CountryCivilWars => TableRows::CountryCivilWars(rows::<CountryCivilWarRow>(tick)),
        TableId::AdvisorTokens => TableRows::AdvisorTokens(rows::<AdvisorTokenRow>(tick)),
        TableId::FactionAdvisors => TableRows::FactionAdvisors(rows::<FactionAdvisorRow>(tick)),
        TableId::RaidSpeedInputs => TableRows::RaidSpeedInputs(rows::<RaidSpeedInputRow>(tick)),
        TableId::StockpileArchetypes => {
            TableRows::StockpileArchetypes(rows::<StockpileArchetypeRow>(tick))
        }
        TableId::RaidAirSites => TableRows::RaidAirSites(rows::<RaidAirSiteRow>(tick)),
        TableId::AceRangeInputs => TableRows::AceRangeInputs(rows::<AceRangeInputRow>(tick)),
        TableId::CountryWars => TableRows::CountryWars(rows::<CountryWarRow>(tick)),
        TableId::NavalRegionAccess => {
            TableRows::NavalRegionAccess(rows::<NavalRegionAccessRow>(tick))
        }
        TableId::BorderConflicts => TableRows::BorderConflicts(rows::<BorderConflictRow>(tick)),
        TableId::RaidPlayerAllowedTypes => {
            TableRows::RaidPlayerAllowedTypes(rows::<RaidPlayerAllowedTypeRow>(tick))
        }
        TableId::RaidTargetStatuses => {
            TableRows::RaidTargetStatuses(rows::<RaidTargetStatusRow>(tick))
        }
        TableId::CountryLocalSubunits => {
            TableRows::CountryLocalSubunits(rows::<CountryLocalSubunitRow>(tick))
        }
        TableId::AvailableEquipmentVariants => {
            TableRows::AvailableEquipmentVariants(rows::<AvailableEquipmentVariantRow>(tick))
        }
        TableId::CountryUnitBonuses => {
            TableRows::CountryUnitBonuses(rows::<CountryUnitBonusRow>(tick))
        }
        TableId::NewDivisionEquipment => {
            TableRows::NewDivisionEquipment(rows::<DivisionEquipmentRow>(tick))
        }
        TableId::ModifierDefinitions => {
            TableRows::ModifierDefinitions(rows::<ModifierDefinitionRow>(tick))
        }
        TableId::OwnedWargoals => TableRows::OwnedWargoals(rows::<OwnedWargoalRow>(tick)),
        TableId::TimedIdeas => TableRows::TimedIdeas(rows::<TimedIdeaRow>(tick)),
        TableId::DecisionRandomItems => {
            TableRows::DecisionRandomItems(rows::<DecisionRandomItemRow>(tick))
        }
        TableId::CapturedOperativeReferences => {
            TableRows::CapturedOperativeReferences(rows::<CapturedOperativeReferenceRow>(tick))
        }
        TableId::ReservedNearestNavies => {
            TableRows::ReservedNearestNavies(rows::<ReservedNearestNavyRow>(tick))
        }
        TableId::ReservedNearestBases => {
            TableRows::ReservedNearestBases(rows::<ReservedNearestBaseRow>(tick))
        }
        TableId::TransportNavies => TableRows::TransportNavies(rows::<TransportNavyRow>(tick)),
        TableId::TransportLedgers => TableRows::TransportLedgers(rows::<TransportLedgerRow>(tick)),
        TableId::TransportReceivers => {
            TableRows::TransportReceivers(rows::<TransportReceiverRow>(tick))
        }
        TableId::TransportSearchRegions => {
            TableRows::TransportSearchRegions(rows::<TransportSearchRegionRow>(tick))
        }
        TableId::TransportSpottingOwners => {
            TableRows::TransportSpottingOwners(rows::<TransportSpottingOwnerRow>(tick))
        }
        TableId::TransportSpottingRegions => {
            TableRows::TransportSpottingRegions(rows::<TransportSpottingRegionRow>(tick))
        }
        TableId::TransportAirRoots => {
            TableRows::TransportAirRoots(rows::<TransportAirRootRow>(tick))
        }
        TableId::TransportAirRegions => {
            TableRows::TransportAirRegions(rows::<TransportAirRegionRow>(tick))
        }
        TableId::TransportRiskShips => {
            TableRows::TransportRiskShips(rows::<TransportRiskShipsRow>(tick))
        }
        TableId::ReservedCountryNaming => {
            TableRows::ReservedCountryNaming(rows::<ReservedCountryNamingRow>(tick))
        }
        TableId::AvailableAdvisors => {
            TableRows::AvailableAdvisors(rows::<AvailableAdvisorRow>(tick))
        }
        TableId::HiredAdvisors => TableRows::HiredAdvisors(rows::<HiredAdvisorRow>(tick)),
        TableId::CountryStabilityInputs => {
            TableRows::CountryStabilityInputs(rows::<CountryStabilityInputRow>(tick))
        }
        TableId::CountryStabilityLines => {
            TableRows::CountryStabilityLines(rows::<CountryStabilityLineRow>(tick))
        }
        TableId::DivisionMovement => TableRows::DivisionMovement(rows::<DivisionMovementRow>(tick)),
        TableId::DivisionPanelStats => {
            TableRows::DivisionPanelStats(rows::<DivisionPanelStatsRow>(tick))
        }
        TableId::DivisionStatus => TableRows::DivisionStatus(rows::<DivisionStatusRow>(tick)),
        TableId::DivisionModelInputs => {
            TableRows::DivisionModelInputs(rows::<DivisionModelInputRow>(tick))
        }
        TableId::StrategicRegionWeights => {
            TableRows::StrategicRegionWeights(rows::<StrategicRegionWeightRow>(tick))
        }
        TableId::DivisionEtaInputs => {
            TableRows::DivisionEtaInputs(rows::<DivisionEtaInputsRow>(tick))
        }
        TableId::CountryTemplateState => {
            TableRows::CountryTemplateState(rows::<CountryTemplateStateRow>(tick))
        }
        TableId::DivisionTemplates => {
            TableRows::DivisionTemplates(rows::<DivisionTemplateRow>(tick))
        }
        TableId::DivisionTemplateIcons => {
            TableRows::DivisionTemplateIcons(rows::<DivisionTemplateIconRow>(tick))
        }
        TableId::LandCombatProgress => {
            TableRows::LandCombatProgress(rows::<LandCombatProgressRow>(tick))
        }
        TableId::LandCombatSides => TableRows::LandCombatSides(rows::<LandCombatSideRow>(tick)),
        TableId::LandCombatEdges => {
            TableRows::LandCombatEdges(rows::<LandCombatEdgesRow>(tick))
        }
        TableId::FuelAwardRecipients => {
            TableRows::FuelAwardRecipients(rows::<FuelAwardRecipientRow>(tick))
        }
        TableId::ScriptVariableNames => {
            TableRows::ScriptVariableNames(rows::<ScriptVariableNameRow>(tick))
        }
        TableId::ScriptVariableValues => {
            TableRows::ScriptVariableValues(rows::<ScriptVariableValueRow>(tick))
        }
        TableId::LandCombatResultObservers => {
            TableRows::LandCombatResultObservers(rows::<CombatLogObserverRow>(tick))
        }
        TableId::CombatEngagementRefreshes => {
            TableRows::CombatEngagementRefreshes(rows::<CombatEngagementRefreshRow>(tick))
        }
        TableId::ThreatSources => TableRows::ThreatSources(rows::<ThreatSourceRow>(tick)),
        TableId::ThreatCountryAggregates => {
            TableRows::ThreatCountryAggregates(rows::<ThreatCountryAggregateRow>(tick))
        }
        TableId::OrdersGroups => TableRows::OrdersGroups(rows::<OrdersGroupRow>(tick)),
        TableId::OrdersGroupMembers => {
            TableRows::OrdersGroupMembers(rows::<OrdersGroupMemberRow>(tick))
        }
        TableId::ArmyGroupMembers => TableRows::ArmyGroupMembers(rows::<ArmyGroupMemberRow>(tick)),
        TableId::IncomingDiplomaticActions => {
            TableRows::IncomingDiplomaticActions(rows::<IncomingDiplomaticActionRow>(tick))
        }
        TableId::Scientists => TableRows::Scientists(rows::<ScientistRow>(tick)),
        TableId::ScientistDaily => TableRows::ScientistDaily(rows::<ScientistDailyRow>(tick)),
        TableId::ScientistSkills => TableRows::ScientistSkills(rows::<ScientistSkillRow>(tick)),
        TableId::RelationOpinions => TableRows::RelationOpinions(rows::<RelationOpinionRow>(tick)),
        TableId::RelationTuples => TableRows::RelationTuples(rows::<RelationTupleRow>(tick)),
        TableId::CountryIntel => TableRows::CountryIntel(rows::<CountryIntelRow>(tick)),
        TableId::ReserveModifierBreakdowns => {
            TableRows::ReserveModifierBreakdowns(rows::<ReserveModifierBreakdownRow>(tick))
        }
        TableId::PowerBalanceModifierBreakdowns => TableRows::PowerBalanceModifierBreakdowns(
            rows::<PowerBalanceModifierBreakdownRow>(tick),
        ),
        TableId::NavalModifierBreakdowns => {
            TableRows::NavalModifierBreakdowns(rows::<NavalModifierBreakdownRow>(tick))
        }
        TableId::NavalModifierTrees => {
            TableRows::NavalModifierTrees(rows::<NavalModifierTreeRow>(tick))
        }
        TableId::NavalModifierTreeSamples => {
            TableRows::NavalModifierTreeSamples(rows::<NavalModifierTreeSampleRow>(tick))
        }
        TableId::AttritionModifierBreakdowns => {
            TableRows::AttritionModifierBreakdowns(rows::<AttritionModifierBreakdownRow>(tick))
        }
        TableId::CountryGameScoreInputs => {
            TableRows::CountryGameScoreInputs(rows::<CountryGameScoreInputRow>(tick))
        }
        TableId::AirWingGameScoreStats => {
            TableRows::AirWingGameScoreStats(rows::<AirWingGameScoreStatsRow>(tick))
        }
        TableId::GameScoreMultipliers => {
            TableRows::GameScoreMultipliers(rows::<GameScoreMultiplierRow>(tick))
        }
        _ => return None,
    })
}
#[cfg(test)]
fn check_table<T: Fixture>(table: TableId) {
    assert_eq!(table.key_bits(), T::KEY_BITS);
    let mut checked_encodings = std::collections::BTreeSet::new();
    for key in [T::smallest_key(), T::largest_key()] {
        let mut previous = T::fixture(key, 0);
        for tick in 1..=12 {
            let next = T::fixture(key, tick);
            assert_eq!(next.key(), key, "{table:?} key");
            let mut bytes = Vec::new();
            encode_row(&next, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<T>(0, &mut input).unwrap(), next, "{table:?}");
            input.finish().unwrap();
            if checked_encodings.insert(bytes.clone()) {
                for prefix in 0..bytes.len() {
                    assert!(
                        decode_row::<T>(0, &mut Decoder::new(&bytes[..prefix])).is_err(),
                        "{table:?} prefix {prefix}"
                    );
                }
            }
            let full = next.delta(None).unwrap().unwrap();
            assert_eq!(full.mask, table.full_mask(), "{table:?}");
            assert_eq!(full.apply::<T>(None).unwrap(), next, "{table:?} insert");
            if let Some(delta) = next.delta(Some(&previous)).unwrap() {
                let mut bytes = Vec::new();
                encode_delta(table, &delta, 0, &mut bytes).unwrap();
                let mut input = Decoder::new(&bytes);
                let decoded = decode_delta(table, 0, &mut input).unwrap();
                input.finish().unwrap();
                assert_eq!(
                    decoded
                        .apply(Some(previous.clone()))
                        .unwrap_or_else(|error| panic!("{table:?} tick {tick}: {error}")),
                    next,
                    "{table:?} delta"
                );
                let mut trailing = decoded;
                trailing.values.push(0);
                assert!(trailing.apply(Some(previous.clone())).is_err());
                assert_eq!(
                    previous
                        .delta(Some(&next))
                        .unwrap()
                        .unwrap()
                        .apply(Some(next.clone()))
                        .unwrap(),
                    previous
                );
            } else {
                assert_eq!(previous, next);
            }
            assert!(next.delta(Some(&next)).unwrap().is_none());
            previous = next;
        }
    }
}

#[test]
fn industrial_organisation_tables_preserve_queue_dates_and_source_values() {
    fn prepared<T: Fixture + crate::checkpoint::rows::Row>() {
        for tick in [0, 5, 12] {
            let expected = vec![T::fixture(T::largest_key(), tick)];
            let mut bytes = Vec::new();
            crate::checkpoint::rows::encode_rows(&expected, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                crate::checkpoint::rows::decode_rows::<T>(&mut input).unwrap(),
                expected
            );
            input.finish().unwrap();
            for end in 0..bytes.len() {
                assert!(
                    crate::checkpoint::rows::decode_rows::<T>(&mut Decoder::new(&bytes[..end]))
                        .is_err()
                );
            }
        }
    }
    prepared::<IndustrialOrganisationDetailsRow>();
    prepared::<IndustrialOrganisationPoliciesRow>();
    prepared::<IndustrialOrganisationHistoryRow>();
    prepared::<VariantStatsRow>();
    check_table::<IndustrialOrganisationDetailsRow>(TableId::IndustrialOrganisationDetails);
    check_table::<IndustrialOrganisationPoliciesRow>(TableId::IndustrialOrganisationPolicies);
    check_table::<IndustrialOrganisationHistoryRow>(TableId::IndustrialOrganisationHistory);
    check_table::<VariantStatsRow>(TableId::VariantStats);
    let previous = IndustrialOrganisationDetailsRow::fixture((5_u128 << 64) | 79, 3);
    let mut next = previous.clone();
    next.state.points += 1;
    assert_eq!(next.delta(Some(&previous)).unwrap().unwrap().mask, 1 << 3);
    next = previous.clone();
    next.queued.reverse();
    next.queued.push(7);
    assert_eq!(next.delta(Some(&previous)).unwrap().unwrap().mask, 1 << 17);
    next = previous.clone();
    next.tasks.push(IndustrialOrganisationTask::default());
    assert_eq!(next.delta(Some(&previous)).unwrap().unwrap().mask, 1 << 19);
}

#[test]
fn game_score_tables_round_trip_optional_values_ordered_witnesses_and_deltas() {
    check_table::<CountryGameScoreInputRow>(TableId::CountryGameScoreInputs);
    check_table::<AirWingGameScoreStatsRow>(TableId::AirWingGameScoreStats);
    check_table::<GameScoreMultiplierRow>(TableId::GameScoreMultipliers);
}

#[test]
fn character_input_rows_refuse_contradictions() {
    let role = CharacterRoleTraitRow {
        character: 1,
        kind: CharacterRoleTraitRow::ADVISOR,
        role: 4,
        tokens: vec![9],
        definitions: vec![2],
    };
    let operative = CharacterOperativeRow {
        character: 1,
        reference: 5,
        resolved: true,
        owner: 9,
        ..Default::default()
    };
    assert!(
        TableRows::CharacterRoleTraits(vec![role.clone()])
            .validate_keys()
            .is_ok()
    );
    assert!(
        TableRows::CharacterOperatives(vec![operative.clone()])
            .validate_keys()
            .is_ok()
    );
    for bad in [
        CharacterRoleTraitRow {
            kind: 3,
            ..role.clone()
        },
        CharacterRoleTraitRow {
            kind: CharacterRoleTraitRow::SCIENTIST,
            ..role.clone()
        },
        CharacterRoleTraitRow {
            tokens: Vec::new(),
            definitions: Vec::new(),
            ..role.clone()
        },
        CharacterRoleTraitRow {
            definitions: Vec::new(),
            ..role
        },
    ] {
        assert!(encode_row(&bad, 0, &mut Vec::new()).is_err(), "{bad:?}");
        let mut bytes = Vec::new();
        bad.encode_fields(&mut bytes).unwrap();
        let decoded =
            CharacterRoleTraitRow::decode_fields(bad.key(), &mut Decoder::new(&bytes)).unwrap();
        assert!(
            TableRows::CharacterRoleTraits(vec![decoded])
                .validate_keys()
                .is_err()
        );
    }
    for bad in [
        CharacterOperativeRow {
            reference: 0,
            ..operative.clone()
        },
        CharacterOperativeRow {
            resolved: false,
            ..operative.clone()
        },
        CharacterOperativeRow {
            resolved: false,
            owner: 0,
            skill_level: Some(1),
            ..operative.clone()
        },
    ] {
        assert!(encode_row(&bad, 0, &mut Vec::new()).is_err(), "{bad:?}");
        let mut bytes = Vec::new();
        bad.encode_fields(&mut bytes).unwrap();
        let decoded =
            CharacterOperativeRow::decode_fields(bad.key(), &mut Decoder::new(&bytes)).unwrap();
        assert!(
            TableRows::CharacterOperatives(vec![decoded])
                .validate_keys()
                .is_err()
        );
    }
    let unresolved = CharacterOperativeRow {
        character: 1,
        reference: 5,
        ..Default::default()
    };
    let delta = operative
        .delta(Some(&CharacterOperativeRow {
            owner: 0,
            ..operative.clone()
        }))
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    encode_delta(TableId::CharacterOperatives, &delta, 0, &mut bytes).unwrap();
    let decoded = decode_delta(TableId::CharacterOperatives, 0, &mut Decoder::new(&bytes)).unwrap();
    let applied = decoded.apply(Some(unresolved)).unwrap();
    assert_eq!(applied.owner, 9);
    assert!(
        TableRows::CharacterOperatives(vec![applied])
            .validate_keys()
            .is_err()
    );
    let bad = CharacterOperativeRow {
        reference: 0,
        ..operative
    };
    assert!(decoded.apply(Some(bad)).is_err());
}

#[test]
fn eta_terrain_entries_are_distinct_database_slots_of_any_width() {
    let entry = |terrain| TerrainSpeed {
        terrain,
        modifier: -10_000,
    };
    let row = |terrain_speed| DivisionEtaInputsRow {
        division: 5,
        terrain_speed,
        ..Default::default()
    };
    let wide = row(vec![entry(1), entry(256), entry(1581)]);
    let mut bytes = Vec::new();
    wide.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        DivisionEtaInputsRow::decode_fields(wide.key(), &mut Decoder::new(&bytes)).unwrap(),
        wide
    );
    for bad in [
        row(vec![entry(0)]),
        row(vec![entry(-1)]),
        row(vec![entry(256), entry(3), entry(256)]),
    ] {
        assert!(encode_row(&bad, 0, &mut Vec::new()).is_err(), "{bad:?}");
    }
}

#[test]
fn supply_vertex_rows_round_trip_beside_nodes_and_delta_only_their_edge() {
    let node = SupplyNodeRow {
        country: 7,
        index: 3,
        province: 12,
        kind: 2,
        available: 500000,
        parent: 40,
        route_from: 40,
        route_to: 12,
        path: vec![12, 11, 40],
        ..Default::default()
    };
    let vertex = SupplyNodeRow {
        country: 7,
        index: SupplyNodeRow::VERTEX | 40,
        province: 40,
        parent: 9,
        ..Default::default()
    };
    assert!(vertex.is_vertex() && !node.is_vertex());
    assert!(node.key() < vertex.key());
    for row in [&node, &vertex] {
        let mut bytes = Vec::new();
        encode_row(row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(&decode_row::<SupplyNodeRow>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
    }
    let moved = SupplyNodeRow {
        parent: 41,
        route_from: 41,
        route_to: 40,
        break_after: 1,
        path: vec![40, 41],
        ..vertex
    };
    let delta = moved.delta(Some(&vertex)).unwrap().unwrap();
    let mut bytes = Vec::new();
    encode_delta(TableId::SupplyNodes, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::SupplyNodes, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(vertex)).unwrap(), moved);
}

#[test]
fn equipment_spending_keeps_old_delivery_prefix_and_independent_optional_suffixes() {
    let table = crate::rows::TableId::ProductionLineOutput;
    assert_eq!(
        table
            .last_change()
            .fields(table, crate::rows::compatibility::WIDGET_CAPTURE_FORMAT - 1),
        9
    );
    assert!(
        crate::format::FormatVersion::CURRENT
            .field_gaps(table)
            .next()
            .is_none()
    );
    let row = ProductionLineOutputRow {
        line: 7,
        country: 5001,
        variant: 9,
        mio: 11,
        units: 3,
        mio_credit: RetiredI64::holding(5),
        since_hour: 100,
        ended: true,
        exact: true,
        spent_raw: Recorded::Value(Some(i64::MIN)),
        instant_raw: Recorded::Value(RetiredOptionalI64::holding(Some(i64::MAX))),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ProductionLineOutputRow::SCHEMA[..9] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = bytes.len() - input.remaining();
    let mut input = Decoder::new(&bytes[..prefix]);
    input.recorded_fields = 9;
    let old = ProductionLineOutputRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        (old.units, old.mio_credit, old.exact, old.ended),
        (3, RetiredI64::holding(5), true, true)
    );
    assert_eq!(old.spent_raw, Recorded::NotRecorded);
    assert_eq!(old.instant_raw, Recorded::NotRecorded);
    assert_eq!(old.purpose, Recorded::NotRecorded);
    assert_eq!(old.purpose(), OutputPurpose::Unknown);
    for (spent, instant) in [
        (Some(i64::MIN), Some(i64::MAX)),
        (None, Some(0)),
        (Some(0), None),
    ] {
        round_trip(&ProductionLineOutputRow {
            spent_raw: Recorded::Value(spent),
            instant_raw: Recorded::Value(RetiredOptionalI64::holding(instant)),
            ..row.clone()
        });
    }
}

#[test]
fn output_purpose_appends_after_spending_and_unrecorded_reads_unknown() {
    let table = crate::rows::TableId::ProductionLineOutput;
    let format = crate::rows::compatibility::CONVERSION_PURPOSE_FORMAT;
    assert_eq!(table.last_change().fields(table, format - 1), 11);
    assert_eq!(table.last_change().fields(table, format), 12);
    assert!(table.last_change().table_gap(table, format - 1).is_none());
    let row = ProductionLineOutputRow {
        line: 7,
        units: 3,
        spent_raw: Recorded::Value(Some(-5)),
        purpose: Recorded::Value(OutputPurpose::Converted.raw()),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ProductionLineOutputRow::SCHEMA[..11] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = bytes.len() - input.remaining();
    let mut input = Decoder::new(&bytes[..prefix]);
    input.recorded_fields = 11;
    let old = ProductionLineOutputRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!((old.units, old.spent_raw), (3, Recorded::Value(Some(-5))));
    assert_eq!(old.purpose(), OutputPurpose::Unknown);
    for purpose in [
        OutputPurpose::Unknown,
        OutputPurpose::New,
        OutputPurpose::Converted,
        OutputPurpose::Refit,
    ] {
        assert_eq!(OutputPurpose::from_raw(purpose.raw()), Some(purpose));
        let row = ProductionLineOutputRow {
            purpose: Recorded::Value(purpose.raw()),
            ..row.clone()
        };
        round_trip(&row);
        assert_eq!(row.purpose(), purpose);
    }
    assert_eq!(
        ProductionLineOutputRow::default().purpose(),
        OutputPurpose::Unknown
    );
    let invalid = ProductionLineOutputRow {
        purpose: Recorded::Value(4),
        ..row
    };
    assert!(invalid.try_clone().is_err());
}

#[test]
fn line_conversion_control_appends_and_older_lines_leave_it_unrecorded() {
    let table = crate::rows::TableId::ProductionLineModifiers;
    let format = crate::rows::compatibility::CONVERSION_PURPOSE_FORMAT;
    assert_eq!(table.last_change().fields(table, format - 1), 15);
    assert_eq!(table.last_change().fields(table, format), 16);
    assert!(table.last_change().table_gap(table, format - 1).is_none());
    let row = ProductionLineModifiersRow {
        line: 7,
        converting: Recorded::Value(true),
        ..Default::default()
    };
    round_trip(&row);
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ProductionLineModifiersRow::SCHEMA[..15] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = bytes.len() - input.remaining();
    let mut input = Decoder::new(&bytes[..prefix]);
    input.recorded_fields = 15;
    let old = ProductionLineModifiersRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(old.converting, Recorded::NotRecorded);
}

#[test]
fn construction_completion_appends_without_changing_old_output_prefixes() {
    let mut row = ConstructionLineOutputRow {
        line: 7,
        construction_raw: -1,
        repair_raw: i64::MIN,
        since_hour: 100,
        ended: true,
        exact: true,
        ..Default::default()
    };
    let mut prefix = Vec::new();
    row.encode_fields(&mut prefix).unwrap();
    let mut input = Decoder::new(&prefix);
    for kind in &ConstructionLineOutputRow::SCHEMA[..13] {
        kind.decode(&mut input).unwrap();
    }
    let length = prefix.len() - input.remaining();
    for format in [98, 99] {
        let mut input = Decoder::new(&prefix[..length]);
        input.format = format;
        input.recorded_fields = FormatVersion::read(format)
            .unwrap()
            .fields(TableId::ConstructionLineOutput);
        let old = ConstructionLineOutputRow::decode_fields(7, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(old.completion, Recorded::NotRecorded);
        assert_eq!((old.construction_raw, old.repair_raw), (-1, i64::MIN));
    }
    for (railway, cancelled) in [(false, false), (true, false), (true, true)] {
        row.railway = railway;
        let next = ConstructionLineOutputRow {
            completion: Recorded::Value(Some(ConstructionCompletion {
                hour: 120,
                cancelled,
            })),
            ..row.clone()
        };
        let delta = next.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 13);
        assert_eq!(delta.apply(Some(row.clone())).unwrap(), next);
        round_trip(&next);
    }
}

#[test]
fn construction_output_owner_and_targets_round_trip_and_preserve_older_absence() {
    let format = crate::rows::compatibility::CONSTRUCTION_OUTPUT_OWNER_FORMAT;
    for railway in [false, true] {
        let row = ConstructionLineOutputRow {
            line: 59,
            railway,
            country: Recorded::Value(Some(1507)),
            building_type: Recorded::Value((!railway).then_some(17001)),
            target_flags: Recorded::Value((!railway).then_some(5)),
            target_location: Recorded::Value((!railway).then_some(120003)),
            target_state: Recorded::Value((!railway).then_some(9001)),
            target_country: Recorded::Value((!railway).then_some(0)),
            route_first: Recorded::Value(railway.then_some(120003)),
            route_last: Recorded::Value(railway.then_some(120009)),
            ..Default::default()
        };
        round_trip(&row);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        for kind in &ConstructionLineOutputRow::SCHEMA[..15] {
            kind.decode(&mut input).unwrap();
        }
        let length = bytes.len() - input.remaining();
        let mut input = Decoder::new(&bytes[..length]);
        input.format = format - 1;
        input.recorded_fields = FormatVersion::read(input.format)
            .unwrap()
            .fields(TableId::ConstructionLineOutput);
        let historical = ConstructionLineOutputRow::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        for field in [
            historical.country,
            historical.building_type,
            historical.target_flags,
            historical.target_location,
            historical.target_state,
            historical.target_country,
            historical.route_first,
            historical.route_last,
        ] {
            assert_eq!(field, Recorded::NotRecorded);
        }
    }
    let previous = FormatVersion::read(format - 1).unwrap();
    assert_eq!(previous.fields(TableId::ConstructionLineOutput), 15);
    assert_eq!(
        previous.field_gaps(TableId::ConstructionLineOutput).count(),
        8
    );
    assert_eq!(
        FormatVersion::CURRENT.fields(TableId::ConstructionLineOutput),
        23
    );
}

#[test]
fn oracle_witness_suffixes_preserve_the_readable_historical_prefixes() {
    let construction = ConstructionLineOutputRow {
        line: 7,
        count_reset: Recorded::Value(2),
        ..Default::default()
    };
    let naval = NavalBaseRow {
        province: 7,
        countries: vec![5, 8, 5],
        country_tags: Recorded::Value(Some(vec![b"A".to_vec(), b"B".to_vec(), b"A".to_vec()])),
        ..Default::default()
    };
    round_trip(&construction);
    round_trip(&naval);
    let mut bytes = Vec::new();
    construction.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ConstructionLineOutputRow::SCHEMA[..14] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = bytes.len() - input.remaining();
    let mut input = Decoder::new(&bytes[..prefix]);
    input.format = crate::rows::compatibility::POLITICAL_FORMAT - 1;
    input.recorded_fields = FormatVersion::read(input.format)
        .unwrap()
        .fields(TableId::ConstructionLineOutput);
    let historical = ConstructionLineOutputRow::decode_fields(7, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(historical.completion, construction.completion);
    assert_eq!(historical.count_reset, Recorded::NotRecorded);
    bytes.clear();
    naval.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &NavalBaseRow::SCHEMA[..5] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = bytes.len() - input.remaining();
    let mut input = Decoder::new(&bytes[..prefix]);
    input.format = crate::rows::compatibility::POLITICAL_FORMAT - 1;
    input.recorded_fields = FormatVersion::read(input.format)
        .unwrap()
        .fields(TableId::NavalBases);
    let historical = NavalBaseRow::decode_fields(7, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(historical.countries, naval.countries);
    assert_eq!(historical.country_tags, Recorded::NotRecorded);
}

#[test]
fn naval_base_country_lists_keep_every_entry_in_native_order() {
    let previous = NavalBaseRow {
        province: 158,
        country: 102,
        level: 10,
        healthy_level: 9,
        countries: (0..20_u32).rev().map(|index| index * 3 + 1).collect(),
        countries_unreadable: false,
        country_tags: Recorded::Value(None),
    };
    round_trip(&previous);
    let mut next = previous.clone();
    next.countries[19] = u32::MAX;
    next.countries.push(7);
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 3);
    let mut bytes = Vec::new();
    encode_delta(TableId::NavalBases, &delta, 0, &mut bytes).unwrap();
    let decoded = decode_delta(TableId::NavalBases, 0, &mut Decoder::new(&bytes)).unwrap();
    assert_eq!(decoded.apply(Some(previous)).unwrap(), next);
}

#[test]
fn sparse_bytes_keep_native_length_nonzero_runs_and_refuse_noncanonical_forms() {
    use crate::value::Value;
    let mut dense = vec![0_u8; 5000];
    for (index, byte) in dense.iter_mut().enumerate() {
        if index % 97 == 0 || (1000..1004).contains(&index) {
            *byte = (index % 251 + 1) as u8;
        }
    }
    dense[4999] = 255;
    let sparse = SparseBytes::from_dense(&dense).unwrap();
    assert_eq!(sparse.len, 5000);
    let mut restored = vec![0; sparse.len as usize];
    for run in &sparse.runs {
        let start = run.start as usize;
        restored[start..start + run.bytes.len()].copy_from_slice(&run.bytes);
    }
    assert_eq!(restored, dense);
    for index in [0, 1, 96, 97, 999, 1000, 1003, 1004, 4998, 4999] {
        assert_eq!(sparse.get(index), Some(dense[index as usize]), "{index}");
    }
    assert_eq!(sparse.get(5000), None);
    assert_eq!(SparseBytes::default().get(0), None);
    let mut bytes = Vec::new();
    sparse.encode(&mut bytes).unwrap();
    assert!(bytes.len() < dense.len() / 10, "{} bytes", bytes.len());
    let mut input = Decoder::new(&bytes);
    assert_eq!(SparseBytes::decode(&mut input).unwrap(), sparse);
    input.finish().unwrap();
    let full = SparseBytes::from_dense(&[9; 300]).unwrap();
    let mut bytes = Vec::new();
    full.encode(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 8 + 1 + 2 + 300);

    let run = |start, bytes: &[u8]| ByteRun {
        start,
        bytes: bytes.to_vec(),
    };
    for runs in [
        vec![run(0, &[])],
        vec![run(0, &[1, 0])],
        vec![run(0, &[1]), run(1, &[2])],
        vec![run(2, &[1]), run(0, &[2])],
        vec![run(9, &[1, 2])],
    ] {
        let value = SparseBytes { len: 10, runs };
        assert!(value.encode(&mut Vec::new()).is_err(), "{value:?}");
    }
    for wire in [
        [
            &10_u32.to_le_bytes()[..],
            &2_u32.to_le_bytes(),
            &[0, 1, 5, 0, 1, 6],
        ]
        .concat(),
        [
            &10_u32.to_le_bytes()[..],
            &1_u32.to_le_bytes(),
            &[0, 2, 5, 0],
        ]
        .concat(),
        [
            &2_u32.to_le_bytes()[..],
            &1_u32.to_le_bytes(),
            &[1, 2, 5, 6],
        ]
        .concat(),
        [
            &10_u32.to_le_bytes()[..],
            &1_u32.to_le_bytes(),
            &[0x80, 0, 1, 5],
        ]
        .concat(),
    ] {
        assert!(SparseBytes::decode(&mut Decoder::new(&wire)).is_err());
    }
}

#[test]
fn sparse_country_intel_keeps_counts_and_zero_gaps() {
    const SLOTS: u32 = 20;
    let mut rows = Vec::new();
    for owner in 1..SLOTS {
        let count = if owner == 7 { 10 } else { SLOTS };
        let hit = owner % count;
        rows.push(CountryIntelRow {
            owner_country: owner,
            target_index: hit,
            army: -(owner as i64),
            ..Default::default()
        });
        if hit != count - 1 {
            rows.push(CountryIntelRow {
                owner_country: owner,
                target_index: count - 1,
                ..Default::default()
            });
        }
    }
    rows.sort_by_key(CountryIntelRow::key);
    let mut bytes = Vec::new();
    let mut previous_key = 0;
    for row in &rows {
        encode_row(row, previous_key, &mut bytes).unwrap();
        previous_key = row.key();
    }
    let mut input = Decoder::new(&bytes);
    let mut previous_key = 0;
    let decoded: Vec<CountryIntelRow> = rows
        .iter()
        .map(|_| {
            let row = decode_row::<CountryIntelRow>(previous_key, &mut input).unwrap();
            previous_key = row.key();
            row
        })
        .collect();
    input.finish().unwrap();
    assert_eq!(decoded, rows);
    assert!(bytes.len() < 80 * SLOTS as usize, "{} bytes", bytes.len());

    let counts = CountryIntelRow::native_counts(&decoded);
    assert_eq!(counts.len(), SLOTS as usize - 1);
    assert_eq!(counts[&1], SLOTS);
    assert_eq!(counts[&7], 10);
    assert_eq!(counts.get(&SLOTS), None);
    let record = |owner: u32, index: u32| {
        decoded
            .iter()
            .find(|row| row.owner_country == owner && row.target_index == index)
    };
    assert!(record(1, 2).is_none() && 2 < counts[&1]);
    assert_eq!(record(1, 1).unwrap().army, -1);
    assert!(record(7, 20).is_none() && 20 >= counts[&7]);
}

#[test]
fn train_inventory_access_round_trips_sparsely() {
    let mut dense = vec![0_u8; 1000];
    for slot in [1, 2, 3, 700, 999] {
        dense[slot] = 1;
    }
    let previous = TrainInventoryRow {
        country: 42,
        node_count: 3,
        allocations: vec![TrainAllocation {
            variant: u64::MAX,
            count: i32::MIN,
        }],
        controller_access: SparseBytes::from_dense(&dense).unwrap(),
        ..Default::default()
    };
    assert!(round_trip(&previous) < 64);
    dense[701] = 128;
    let next = TrainInventoryRow {
        controller_access: SparseBytes::from_dense(&dense).unwrap(),
        ..previous.clone()
    };
    assert_eq!(next.controller_access.runs.len(), 3);
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
}

#[test]
fn party_leader_role_keys_round_trip_and_delta() {
    let previous = PartyLeaderRow {
        character: Some(0),
        role: Some(i32::MIN),
        leader_id: -1,
        ..Default::default()
    };
    for role in [i32::MIN, 0, i32::MAX] {
        let row = PartyLeaderRow {
            role: Some(role),
            ..previous.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        assert_eq!(
            decode_row::<PartyLeaderRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            row
        );
        if role != i32::MIN {
            let delta = row.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 2);
            assert_eq!(delta.apply(Some(previous.clone())).unwrap(), row);
        }
    }
    let none = PartyLeaderRow {
        character: None,
        role: None,
        ..previous
    };
    let delta = none.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.apply(Some(previous.clone())).unwrap(), none);
    for row in [
        PartyLeaderRow {
            role: None,
            ..previous.clone()
        },
        PartyLeaderRow {
            character: None,
            ..previous
        },
    ] {
        assert!(encode_row(&row, 0, &mut Vec::new()).is_err());
    }
}

#[test]
fn country_appearance_preserves_float_bits_and_literal_tag_transitions() {
    let previous = CountryAppearanceRow {
        country: u32::MAX,
        red: 0x8000_0000,
        green: 0x7fc1_2345,
        blue: 0x7f80_0000,
        alpha: 0xff80_0000,
        cosmetic_tag: b"BRM_british".to_vec(),
        name_tag: b"GER".to_vec(),
        leader_ideology: b"nazism".to_vec(),
        overlord: -1,
        graphical_culture: b"western_european_gfx".to_vec(),
        leader_character: Some(u32::MAX),
        old_name: Some(b"Deutsches Reich".to_vec()),
        old_adjective: Some(Vec::new()),
        old_definite_name: None,
        ui_red: 0x3f80_0000,
        ui_green: 0x0000_0001,
        ui_blue: 0x7fc0_0001,
        ui_alpha: 0xbf80_0000,
        graphical_culture_2d: b"western_european_2d".to_vec(),
        subjects: Recorded::Value(vec![7, -1]),
        original_tag: Recorded::Value(Some(b"GER".to_vec())),
        original_tag_reference: Recorded::Value(Some(i32::MAX)),
        transport_convoys: Recorded::Value(Retired),
    };
    let mut bytes = Vec::new();
    encode_row(&previous, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<CountryAppearanceRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        previous
    );
    let next = CountryAppearanceRow {
        cosmetic_tag: Vec::new(),
        ..previous.clone()
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 4);
    assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    for leader_character in [Some(0), None] {
        let next = CountryAppearanceRow {
            leader_character,
            ..previous.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        assert_eq!(
            decode_row::<CountryAppearanceRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            next
        );
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 9);
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    for old_name in [Some(Vec::new()), None, Some(b"Reich".to_vec())] {
        let next = CountryAppearanceRow {
            old_name,
            ..previous.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        assert_eq!(
            decode_row::<CountryAppearanceRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            next
        );
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 10);
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    let recoloured = CountryAppearanceRow {
        ui_green: 0x3f00_0000,
        ..previous.clone()
    };
    let delta = recoloured.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 14);
    assert_eq!(delta.apply(Some(previous.clone())).unwrap(), recoloured);
    let next = CountryAppearanceRow {
        graphical_culture_2d: Vec::new(),
        ..previous.clone()
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 17);
    assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
}

#[test]
fn ship_models_preserve_full_flags_and_counter_inputs() {
    let row = ShipModelRow {
        formation_flags: u64::MAX,
        subunit_index: Recorded::Value(None),
        strike: Recorded::Value(Retired),
        hull_variant: Recorded::Value(None),
        counter: Some(ShipCounterInputs {
            roster_ordinal: 7,
            definition_valid: 255,
            carrier_air_base: u64::MAX,
            maximum_strength: i64::MIN,
            anti_air_base: i64::MIN + 1,
            maximum_organisation: i64::MAX,
            positive_equipment_flags: u64::MAX,
        }),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    encode_row(&row, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<ShipModelRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        row
    );
}

#[test]
fn ship_subunit_index_preserves_zero_refusal_and_signed_suffix_deltas() {
    let baseline = ShipModelRow {
        subunit_index: Recorded::Value(None),
        ..ShipModelRow::fixture(u64::MAX as u128, 4)
    };
    for value in [None, Some(i32::MIN), Some(-1), Some(0), Some(i32::MAX)] {
        let next = ShipModelRow {
            subunit_index: Recorded::Value(value),
            ..baseline.clone()
        };
        round_trip(&next);
        if value.is_none() {
            assert!(next.delta(Some(&baseline)).unwrap().is_none());
            continue;
        }
        let delta = next.delta(Some(&baseline)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 6);
        let mut bytes = Vec::new();
        encode_delta(TableId::ShipModels, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let decoded = decode_delta(TableId::ShipModels, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded.apply(Some(baseline.clone())).unwrap(), next);
        let cleared = baseline.delta(Some(&next)).unwrap().unwrap();
        assert_eq!(cleared.mask, 1 << 6);
        assert_eq!(cleared.apply(Some(next)).unwrap(), baseline);
    }
    let historical = ShipModelRow {
        subunit_index: Recorded::NotRecorded,
        ..baseline
    };
    assert_eq!(historical.not_recorded_fields(), 1 << 6);
    assert!(encode_row(&historical, 0, &mut Vec::new()).is_err());
}

#[test]
fn naval_port_packet_preserves_ordered_unknowns_and_raw_bytes_in_suffix_delta() {
    let baseline = TaskForceCounterInputRow {
        port_mission: Recorded::Value(None),
        ..TaskForceCounterInputRow::fixture(u64::MAX as u128, 4)
    };
    let packet = NavalPortMissionInputs {
        sample_hour: PacketHour::Host,
        searching: Some(vec![
            NavalSearcher {
                reference: Some(u64::MAX),
                resolves: Some(true),
                roster_owner: Some(0),
            },
            NavalSearcher::default(),
            NavalSearcher {
                reference: Some(0),
                resolves: Some(false),
                roster_owner: None,
            },
        ]),
        engagement_target: Some(NavalEngagementTarget {
            reference: 1 << 63,
            resolves: None,
        }),
        avoided_ship_types: Some(vec![Some(vec![0xff, 0, b'A']), None, Some(Vec::new())]),
    };
    let next = TaskForceCounterInputRow {
        port_mission: Recorded::Value(Some(packet)),
        ..baseline.clone()
    };
    round_trip(&next);
    let delta = next.delta(Some(&baseline)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 12);
    let mut bytes = Vec::new();
    encode_delta(TableId::TaskForceCounterInputs, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::TaskForceCounterInputs, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(baseline.clone())).unwrap(), next);
    let cleared = baseline.delta(Some(&next)).unwrap().unwrap();
    assert_eq!(cleared.mask, 1 << 12);
    assert_eq!(cleared.apply(Some(next)).unwrap(), baseline);
}

#[test]
fn current_ship_suffixes_keep_independent_missing_scalars_and_signed_bits() {
    let baseline = ShipModelRow {
        hull_variant: Recorded::Value(None),
        ..ShipModelRow::fixture(u64::MAX as u128, 4)
    };
    let variant = HullVariantInputs {
        definition_index: Some(-1),
        module_count: Some(i32::MAX),
        upgrade_level_sum: Some(i32::MIN),
        module_priority_sum: None,
        version: Some(u8::MAX),
        organisation_marker: Some(-1),
    };
    {
        let next = ShipModelRow {
            hull_variant: Recorded::Value(Some(variant)),
            ..baseline.clone()
        };
        round_trip(&next);
        let delta = next.delta(Some(&baseline)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 8);
        let mut bytes = Vec::new();
        encode_delta(TableId::ShipModels, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let decoded = decode_delta(TableId::ShipModels, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded.apply(Some(baseline.clone())).unwrap(), next);
        let cleared = baseline.delta(Some(&next)).unwrap().unwrap();
        assert_eq!(cleared.apply(Some(next)).unwrap(), baseline);
    }
}

#[test]
fn naval_window_stored_operands_and_breakdowns_round_trip() {
    check_table::<NavalModifierTreeRow>(TableId::NavalModifierTrees);
    check_table::<NavalModifierTreeSampleRow>(TableId::NavalModifierTreeSamples);
}

#[test]
fn combat_window_operands_round_trip() {
    let sides = LandCombatSideDetails {
        commander_formation: u64::MAX,
        commander: 0x8000_0041_0000_0035,
        ..LandCombatSideDetails::default()
    };
    let details = LandCombatDetailsRow {
        combat: 9,
        sides: [sides, LandCombatSideDetails::default()],
        ..LandCombatDetailsRow::default()
    };
    let mut bytes = Vec::new();
    encode_row(&details, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<LandCombatDetailsRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        details
    );
    let division = LandCombatDivisionRow {
        combat: 9,
        side: 1,
        ordinal: u32::MAX,
        direction_group: i32::MIN,
        categories: u64::MAX,
        exile: Some(LandCombatExileInputs {
            origin_type: 2,
            country: -1,
            country_fallback: 7,
            location_cores: vec![7, 0],
            occupiers: vec![u32::MAX],
        }),
        ..LandCombatDivisionRow::default()
    };
    bytes.clear();
    encode_row(&division, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<LandCombatDivisionRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        division
    );
    assert_eq!(TableId::LandCombatDivisions.full_mask(), (1 << 15) - 1);
    assert_eq!(TableId::LandCombatDivisions.mask_width(), 2);
}

#[test]
fn persisted_upgrade_groups_keep_army_identity_and_native_priority() {
    let mut previous = ArmyUpgradeGroupRow {
        country: u32::MAX,
        army: u64::MAX,
        priority: 1,
    };
    assert_eq!(TableId::ArmyUpgradeGroups.full_mask(), 3);
    for priority in [0, u32::MAX, 1] {
        let next = ArmyUpgradeGroupRow {
            priority,
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 2);
        assert_eq!(delta.key, u128::from(u64::MAX));
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(input.varint().unwrap(), u128::from(u64::MAX));
        assert_eq!(input.remaining(), 8);
        assert_eq!(input.u32().unwrap(), u32::MAX);
        assert_eq!(input.u32().unwrap(), priority);
        input.finish().unwrap();
        assert_eq!(
            decode_row::<ArmyUpgradeGroupRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            next
        );
        previous = next;
    }
}

#[test]
fn operations_record_native_definition_and_independent_progress_fields() {
    let previous = OperationRow {
        id: 7,
        owner: 1,
        target: 2,
        definition_token: 0x1234_5678,
        preparation_current: i64::MIN,
        preparation_required: i64::MAX,
        ..OperationRow::default()
    };
    assert_eq!(TableId::Operations.field_count(), 21);
    let mut bytes = Vec::new();
    encode_row(&previous, 0, &mut bytes).unwrap();
    assert_eq!(bytes.len(), 84);
    assert_eq!(&bytes[49..59], &[0; 10]);
    for list in bytes[59..83].chunks(8) {
        assert_eq!(list, &[4, 0, 0, 0, 0, 0, 0, 0]);
    }
    assert_eq!(bytes[83], 0);
    assert_eq!(
        &bytes[..14],
        &[7, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]
    );
    assert_eq!(&bytes[14..22], &i64::MIN.to_le_bytes());
    assert_eq!(&bytes[22..30], &i64::MAX.to_le_bytes());
    let next = OperationRow {
        preparation_current: i64::MAX,
        assigned_slots: u32::MAX,
        ..previous.clone()
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 4) | (1 << 12));
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
}

#[test]
fn intel_networks_preserve_unassigned_selectors_and_optional_native_strength() {
    let mut previous = IntelNetworkRow {
        owner_country: u32::MAX,
        target_country: 7,
        state_id: i32::MIN,
        strength_raw: None,
        subnetwork_selector: -1,
        local_strength_raw: i64::MIN,
        ..Default::default()
    };
    for (subnetwork_selector, strength_raw, mask) in [
        (i32::MIN, None, 2),
        (-2, None, 2),
        (0, Some(0), 3),
        (0, Some(i64::MIN), 1),
        (0, Some(i64::MAX), 1),
        (i32::MAX, Some(i64::MAX), 2),
        (-1, None, 3),
    ] {
        let next = IntelNetworkRow {
            strength_raw,
            subnetwork_selector,
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, mask);
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<IntelNetworkRow>(0, &mut input).unwrap(), next);
        input.finish().unwrap();
        previous = next;
    }
}

#[test]
fn propaganda_efforts_keep_loaded_drift_then_derive_only_after_the_native_writer() {
    let mut rows = vec![
        PropagandaEffortRow {
            slot: 5000,
            country: u32::MAX,
            sequence: 91,
            stability: i64::MAX,
            war_support: -13,
            derived_drift: 0,
        },
        PropagandaEffortRow {
            slot: 5000,
            country: u32::MAX,
            sequence: 92,
            stability: 1,
            war_support: 19,
            derived_drift: 0,
        },
        PropagandaEffortRow {
            slot: 5000,
            country: u32::MAX,
            sequence: u64::MAX,
            stability: 17,
            war_support: -19,
            derived_drift: 0,
        },
    ];
    assert_eq!(
        PropagandaEffortRow::weekly_drift(&rows, 5000),
        Some((17, -19))
    );
    assert!(rows[0].delta(Some(&rows[0])).unwrap().is_none());
    let previous = rows[2];
    rows[2].stability = 0;
    rows[2].war_support = 0;
    rows[2].derived_drift = 1;
    assert_eq!(
        rows[2]
            .delta(Some(&previous))
            .unwrap()
            .unwrap()
            .apply(Some(previous))
            .unwrap(),
        rows[2]
    );
    assert_eq!(
        PropagandaEffortRow::weekly_drift(&rows, 5000),
        Some((i64::MIN, 6))
    );
    assert_eq!(PropagandaEffortRow::weekly_drift(&rows, 4999), None);
    rows.pop();
    assert_eq!(PropagandaEffortRow::weekly_drift(&rows, 5000), None);
}

#[test]
fn intel_network_summaries_keep_independent_cache_words_and_change_only_deltas() {
    let previous = IntelNetworkSummaryRow {
        owner_country: u32::MAX,
        target_country: 5_000,
        ordinal: 4_998,
        covered_core_states: 11,
        covered_controlled_states: 7,
        covered_worth: i32::MAX,
        controlled_strength_sum: i64::MIN,
        core_strength_sum: -13,
        max_state_strength: i64::MAX,
    };
    assert!(previous.delta(Some(&previous)).unwrap().is_none());
    let next = IntelNetworkSummaryRow {
        core_strength_sum: 0,
        ..previous
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 4);
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
    let mut bytes = Vec::new();
    encode_row(&next, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<IntelNetworkSummaryRow>(0, &mut input).unwrap(),
        next
    );
    input.finish().unwrap();
}

#[test]
fn intel_sources_keep_full_width_occurrence_keys_and_independent_change_only_columns() {
    let contribution = IntelContributionRow {
        owner: 5000,
        kind: 6,
        pool: u32::MAX,
        ordinal: u32::MAX,
        target: -17,
        civilian: i64::MIN,
        air: i64::MAX,
        ..Default::default()
    };
    round_trip(&contribution);
    assert!(contribution.delta(Some(&contribution)).unwrap().is_none());
    let next = IntelContributionRow {
        army: 19,
        ..contribution
    };
    let delta = next.delta(Some(&contribution)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    assert_eq!(delta.apply(Some(contribution)).unwrap(), next);
    let source = IntelSourceRow {
        owner: u32::MAX,
        pool: u32::MAX,
        source: u32::MAX,
        ordinal: u32::MAX,
        id: i32::MIN,
        discriminant: u32::MAX,
        reference_country: 5000,
        reference_pool: -17,
        ..Default::default()
    };
    assert_eq!(source.key(), u128::MAX);
    round_trip(&source);
    assert!(source.delta(Some(&source)).unwrap().is_none());
    let next = IntelSourceRow {
        removed: 1,
        ..source
    };
    let delta = next.delta(Some(&source)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 6);
    assert_eq!(delta.apply(Some(source)).unwrap(), next);
}

#[test]
fn intel_networks_carry_gains_quiet_bytes_and_the_selected_subnetworks_targets() {
    let previous = IntelNetworkRow {
        owner_country: 1,
        target_country: 2,
        state_id: 219,
        strength_raw: Some(7_250_000),
        subnetwork_selector: 0,
        local_strength_raw: 7_250_000,
        ..Default::default()
    };
    let next = IntelNetworkRow {
        gain_raw: -175_000,
        gain_from_operatives_raw: i64::MIN,
        gain_from_adjacency_raw: i64::MAX,
        local_quiet: u8::MAX,
        gain_factor_raw: Some(100_000),
        strength_target_raw: Some(7_250_000),
        target_from_operatives_raw: Some(3_000_000),
        target_from_counterintelligence_raw: Some(-4_750_000),
        subnetwork_quiet: 1,
        ..previous
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 0b1111_1111_1000);
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
    let mut bytes = Vec::new();
    encode_row(&next, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(decode_row::<IntelNetworkRow>(0, &mut input).unwrap(), next);
    input.finish().unwrap();
}

#[test]
fn theatre_volunteer_tag_preserves_signed_bits_independently_of_owner() {
    let mut previous = LandTheatreRow {
        id: u64::MAX,
        owner: u32::MAX,
        ordinal: 7,
        volunteers_theatre: i32::MIN,
    };
    assert_eq!(TableId::LandTheatres.full_mask(), 7);
    for volunteers_theatre in [i32::MAX, 0, -1, i32::MIN] {
        let next = LandTheatreRow {
            volunteers_theatre,
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 2);
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        let mut wire = Vec::new();
        encode_row(&next, 0, &mut wire).unwrap();
        let mut input = Decoder::new(&wire);
        assert_eq!(input.varint().unwrap(), u128::from(u64::MAX));
        assert_eq!(input.u32().unwrap(), u32::MAX);
        assert_eq!(input.u32().unwrap(), 7);
        assert_eq!(input.i32().unwrap(), volunteers_theatre);
        input.finish().unwrap();
        previous = next;
    }
}

#[test]
fn wide_masks_change_one_native_channel_at_a_time() {
    assert_eq!(TableId::AirStatistics.field_count(), 26);
    assert_eq!(TableId::AirStatistics.mask_width(), 4);
    let previous = AirStatisticsRow::default();
    for index in 0..24 {
        let mut next = previous.clone();
        next.channels[index] = i64::MIN;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1_u64 << (index + 2));
        let mut bytes = Vec::new();
        encode_delta(TableId::AirStatistics, &delta, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[1..5], &(1_u32 << (index + 2)).to_le_bytes());
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    let previous = VariantStatsRow::default();
    for index in 0..10 {
        let mut next = previous.clone();
        next.values[index] = Some(i64::MAX);
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1_u64 << index);
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    let previous = ProjectRow::default();
    let mut next = previous.clone();
    next.state.state_enums[3] = u32::MAX;
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask.count_ones(), 1);
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
}

#[test]
fn structured_fields_reject_hostile_inner_counts_tags_and_suffixes() {
    fn reject<T: NativeValue>(inner: &[u8]) {
        let mut bytes = Vec::new();
        put_bytes(&mut bytes, inner).unwrap();
        assert!(read_value::<T>(Kind::Opaque, &mut Decoder::new(&bytes)).is_err());
    }
    reject::<Option<Vec<u8>>>(&[2]);
    reject::<Option<Vec<u8>>>(&[1, 255, 255, 255, 255]);
    reject::<Option<Vec<u8>>>(&[0, 0]);
    reject::<Option<AceDeath>>(&[1, 0]);
    reject::<Option<AceDeath>>(&[2]);
    reject::<Vec<i64>>(&u32::MAX.to_le_bytes());
    reject::<Vec<ScriptVariableEntry>>(&u32::MAX.to_le_bytes());
    reject::<Vec<AirHistoryEntry>>(&u32::MAX.to_le_bytes());
    reject::<Vec<AirDamageEntry>>(&u32::MAX.to_le_bytes());
    reject::<std::sync::Arc<Vec<AirCombatRecord>>>(&u32::MAX.to_le_bytes());
    let records = vec![AirCombatRecord {
        wing_id: 3,
        first_damage: vec![AirCombatDamage { cause: 1, amount: -2 }],
        equipment: Recorded::Value(Some(vec![AirCombatEquipment { variant: 70, amount_raw: 5 }])),
        ..Default::default()
    }];
    let (mut plain, mut shared) = (Vec::new(), Vec::new());
    crate::value::Value::encode(&records, &mut plain).unwrap();
    crate::value::Value::encode(&std::sync::Arc::new(records), &mut shared).unwrap();
    assert_eq!(plain, shared);
    reject::<Vec<SunkShipEntry>>(&u32::MAX.to_le_bytes());
    let mut nested = 1_u32.to_le_bytes().to_vec();
    nested.extend_from_slice(&u32::MAX.to_le_bytes());
    nested.extend_from_slice(&0_i64.to_le_bytes());
    reject::<Vec<ScriptVariableEntry>>(&nested);
    for kind in [Kind::OptionalI32, Kind::OptionalU64, Kind::Bool] {
        assert!(kind.decode(&mut Decoder::new(&[2])).is_err());
    }
    for (kind, old, difference) in [
        (Kind::I16, Field::I16(i16::MAX), 1),
        (Kind::I16, Field::I16(i16::MIN), -1),
        (Kind::U16, Field::U16(u16::MAX), 1),
        (Kind::U16, Field::U16(0), -1),
        (Kind::Bool, Field::Bool(true), 1),
        (Kind::OptionalI32, Field::OptionalI32(Some(i32::MIN)), -1),
        (Kind::OptionalU64, Field::OptionalU64(Some(u64::MAX)), 1),
    ] {
        assert!(Change::Number(difference).apply(kind, old).is_err());
    }
}

#[test]
fn ace_presentation_suffix_keeps_previous_roster_bytes_and_explicit_absence() {
    let row = AirAceRow {
        id: u64::MAX,
        name: vec![0xff, 0, b'N'],
        surname: b"S".to_vec(),
        presentation: Recorded::Value(Some(AirAcePresentation {
            gender: 7,
            pool_index: -3,
            definition: Some(b"mod_type".to_vec()),
            portrait: Some(AirAcePortraitInputs {
                cached: Some(b"mod_sprite".to_vec()),
                ..Default::default()
            }),
        })),
        ..Default::default()
    };
    let table = TableId::AirAces;
    let previous =
        crate::format::FormatVersion::read(crate::rows::compatibility::GAME_SCORE_FORMAT - 1)
            .unwrap();
    assert!(previous.table_refusal(table).is_none());
    assert_eq!(previous.fields(table), 11);
    let bytes = TableRows::AirAces(vec![row.clone()]).fixture_prefix(previous.fields(table));
    let mut input = Decoder::new(&bytes);
    input.format = previous.version();
    input.recorded_fields = previous.fields(table);
    let decoded = decode_row::<AirAceRow>(0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.name, row.name);
    assert_eq!(decoded.surname, row.surname);
    assert_eq!(decoded.id, row.id);
    assert_eq!(decoded.presentation, Recorded::NotRecorded);
    assert_ne!(decoded.presentation, Recorded::Value(None));
    assert_eq!(
        TableRows::AirAces(vec![decoded])
            .field_gaps()
            .collect::<Vec<_>>(),
        [("presentation", 1)]
    );
}

#[test]
fn division_status_and_panel_stats_keep_separate_masks_with_movement_headroom() {
    let row = DivisionStatusRow {
        division: 7,
        experience: Recorded::Value(Some(i64::MAX)),
        pending_template: Recorded::Value(Some(u64::MAX)),
        officer_experience: Some(i64::MIN).into(),
        career_counter: Some(i32::MAX).into(),
        ..Default::default()
    };
    round_trip(&row);
    let mut changed = row.clone();
    changed.officer_experience = Some(-17).into();
    assert_eq!(changed.delta(Some(&row)).unwrap().unwrap().mask, 1 << 34);
    round_trip(&changed);
    assert!(TableId::DivisionStatus.field_count() + 3 <= u64::BITS as usize);
    let mut row = DivisionPanelStatsRow {
        division: 7,
        ..Default::default()
    };
    for (index, value) in row.panel_stats.iter_mut().enumerate() {
        *value = Some(i64::MIN + index as i64);
    }
    row.suppression = Recorded::Value(Some(i64::MIN + 22));
    round_trip(&row);
    assert_eq!(TableId::DivisionPanelStats.field_count(), 25);
    assert_eq!(row.word(0x3d), Some(Recorded::Value(Some(i64::MIN))));
    assert_eq!(row.word(0x3e), Some(Recorded::Value(Some(i64::MIN + 21))));
    assert_eq!(row.word(0xc), Some(Recorded::Value(Some(i64::MIN + 22))));
    assert_eq!(row.word(0x30), None);
    for index in 0..23 {
        let mut changed = row.clone();
        if index < 22 {
            changed.panel_stats[index] = None;
        } else {
            changed.suppression = Recorded::Value(None);
        }
        let delta = changed.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << index);
        assert_eq!(delta.apply(Some(row.clone())).unwrap(), changed);
    }
    let mut bytes = Vec::new();
    crate::checkpoint::rows::encode_rows(&[row.clone()], &mut bytes).unwrap();
    let restored =
        crate::checkpoint::rows::decode_rows::<DivisionPanelStatsRow>(&mut Decoder::new(&bytes))
            .unwrap();
    assert_eq!(restored, [row]);
}

#[test]
fn land_side_inputs_append_at_display_precision_and_older_recordings_lack_them() {
    let side = LandCombatSideInputs {
        anti_air_attack: 123_456_789,
        prevented_damage: [1_234_567, 99_999],
        skill_progress: 456_789,
        skill_modifiers: Some([-12_345, 67_891]),
        description_token: b"GENERAL_DESC".to_vec(),
        timed_traits: vec![LandCombatCommanderTimedTrait {
            token_id: 7,
            definition_ordinal: 3,
            remaining_days: 0,
        }],
        learning_traits: Some(vec![LandCombatLearningProgress {
            token_id: 9,
            definition_ordinal: 4,
            progress: 5_999,
            token: b"trait_engineer".to_vec(),
        }]),
    };
    let mut row = LandCombatDetailsRow {
        combat: 9,
        side_inputs: Recorded::Value([side, LandCombatSideInputs::default()]),
        ..Default::default()
    };
    RowCodec::round_to_display(&mut row);
    let Recorded::Value([stepped, empty]) = &row.side_inputs else {
        unreachable!();
    };
    assert_eq!(stepped.anti_air_attack, 123_456_789);
    assert_eq!(stepped.prevented_damage, [1_200_000, 0]);
    assert_eq!(stepped.skill_progress, 456_000);
    assert_eq!(stepped.skill_modifiers, Some([-12_345, 67_891]));
    assert_eq!(stepped.learning_traits.as_ref().unwrap()[0].progress, 5_000);
    assert_eq!(empty.skill_progress, LandCombatSideInputs::NO_PROGRESS);
    round_trip(&row);
    let mut unread = row.clone();
    if let Recorded::Value([side, _]) = &mut unread.side_inputs {
        side.skill_progress = LandCombatSideInputs::UNREAD_PROGRESS;
        side.skill_modifiers = None;
        side.learning_traits = None;
    }
    RowCodec::round_to_display(&mut unread);
    let Recorded::Value([side, _]) = &unread.side_inputs else {
        unreachable!();
    };
    assert_eq!(side.skill_progress, LandCombatSideInputs::UNREAD_PROGRESS);
    assert_ne!(unread, row);
    round_trip(&unread);
    let mut drifted = row.clone();
    if let Recorded::Value([side, _]) = &mut drifted.side_inputs {
        side.prevented_damage[0] += 99_999;
        side.skill_progress += 999;
    }
    RowCodec::round_to_display(&mut drifted);
    assert_eq!(drifted, row);
    assert_eq!(row.cas_inputs(0), Some((123_456_789, [1_200_000, 0])));
    assert_eq!(row.cas_inputs(1), Some((0, [0, 0])));
    assert_eq!(
        LandCombatDetailsRow::default().cas_inputs(0),
        Some((0, [0, 0]))
    );
    let neither = LandCombatDetailsRow {
        side_inputs: Recorded::NotRecorded,
        ..Default::default()
    };
    assert_eq!(neither.cas_inputs(0), None);
    let table = TableId::LandCombatDetails;
    let previous =
        crate::format::FormatVersion::read(crate::rows::compatibility::POLITICAL_FORMAT).unwrap();
    assert!(previous.table_refusal(table).is_none());
    assert_eq!(previous.fields(table), 10);
    let mut older = row.clone();
    older.sides[1].anti_air_attack = RetiredKept::older_value(-5);
    older.sides[1].prevented_damage = RetiredKept::older_value([1_234_567, 1]);
    let bytes = crate::rows::with_historical_stamp(0, || {
        TableRows::LandCombatDetails(vec![older]).fixture_prefix(previous.fields(table))
    });
    let mut input = Decoder::new(&bytes);
    input.format = previous.version();
    input.recorded_fields = previous.fields(table);
    let decoded = decode_row::<LandCombatDetailsRow>(0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.side_inputs, Recorded::NotRecorded);
    assert_eq!(decoded.cas_inputs(0), Some((0, [0, 0])));
    assert_eq!(decoded.cas_inputs(1), Some((-5, [1_234_567, 1])));
    assert_eq!(
        TableRows::LandCombatDetails(vec![decoded])
            .field_gaps()
            .collect::<Vec<_>>(),
        [("side_inputs", 1)]
    );
    assert_eq!(crate::format::FormatVersion::CURRENT.fields(table), 11);
}

#[test]
fn land_combat_members_store_their_division_values_once() {
    let table = TableId::LandCombatDivisions;
    assert_eq!(table.field_count(), 15);
    assert_eq!(table.field_name(14).as_deref(), Some("own"));
    let mut panel = DivisionPanelStatsRow {
        division: 7,
        ..Default::default()
    };
    for (at, word) in panel.panel_stats.iter_mut().enumerate() {
        *word = Some(at as i64 * 1_000);
    }
    let status = DivisionStatusRow {
        division: 7,
        strength_factor: 90_000,
        naming: Recorded::Value(Some(NameOperands {
            order: 3,
            ordered: 1,
            group: b"G".to_vec(),
            ..Default::default()
        })),
        movement_stats: Recorded::Value(Some(DivisionMovementStats {
            fuel_rate: 2_000,
            air_defense: 3_000,
            ..Default::default()
        })),
        ..Default::default()
    };
    let movement = DivisionMovementRow {
        division: 7,
        fuel: Recorded::Value(Some(500)),
        ..Default::default()
    };
    let member = LandCombatDivisionRow {
        combat: 9,
        side: 1,
        ordinal: 2,
        division: 7,
        ..Default::default()
    };
    let values = member
        .values(Some(&panel), Some(&status), Some(&movement))
        .unwrap();
    assert_eq!(values.stat_values, [11_000, 12_000, 20_000, 9_000, 10_000]);
    assert_eq!(
        (
            values.armour,
            values.piercing,
            values.reconnaissance,
            values.initiative,
            values.base_speed
        ),
        (13_000, 14_000, 3_000, 15_000, 21_000)
    );
    assert_eq!(
        (values.fuel_inputs, values.air_attack, values.strength_combat_factor),
        ([500, 2_000], 3_000, 90_000)
    );
    let own = LandCombatMemberValues {
        armour: 1,
        ..values
    };
    let orphan = LandCombatDivisionRow {
        own: Some(own.clone()),
        ..member.clone()
    };
    assert_eq!(orphan.values(None, None, None), Some(own));
    assert_eq!(member.values(Some(&panel), None, Some(&movement)), None);
    for row in [member, orphan] {
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<LandCombatDivisionRow>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
    }
}

#[test]
fn nested_allocations_move_unchanged_and_absence_differs_from_empty() {
    let previous = AirAceRow {
        name: vec![0, 255, b'N'],
        surname: vec![254],
        callsign: vec![0],
        death: Some(AceDeath {
            killer_ace: u64::MAX,
            killer_country: u32::MAX,
        }),
        ..AirAceRow::default()
    };
    let name = previous.name.as_ptr();
    let mut next = previous.clone();
    next.handled = 255;
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    let restored = delta.apply(Some(previous)).unwrap();
    assert_eq!(restored, next);
    assert_eq!(restored.name.as_ptr(), name);
    let mut previous = FocusRow::default();
    for tree in [Some(vec![]), None, Some(vec![0, 255]), Some(vec![])] {
        let next = FocusRow {
            tree,
            ..FocusRow::default()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1);
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        previous = next;
    }
}

#[test]
fn state_flag_payload_keeps_lossless_bytes_and_unchanged_emits_no_delta() {
    let mut previous = StateFlagRow {
        border_conflict_active: crate::rows::Recorded::Value(0),
        border_conflict_countries: crate::rows::Recorded::Value(Vec::new()),
        state: 978,
        demilitarized: 0,
    };
    for byte in [0, 1, 255, 2, 0] {
        let row = StateFlagRow {
            border_conflict_active: crate::rows::Recorded::Value(0),
            border_conflict_countries: crate::rows::Recorded::Value(Vec::new()),
            state: 978,
            demilitarized: byte,
        };
        let mut payload = Vec::new();
        row.encode_fields(&mut payload).unwrap();
        assert_eq!(payload, [byte, 0, 4, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            StateFlagRow::decode_fields(978, &mut Decoder::new(&payload)).unwrap(),
            row
        );
        if row == previous {
            assert!(row.delta(Some(&previous)).unwrap().is_none());
        } else {
            let delta = row.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1);
            assert_eq!(delta.apply(Some(previous)).unwrap(), row);
        }
        previous = row;
    }
}

#[test]
fn every_extended_table_round_trips_extremes_nested_records_and_deltas() {
    check_table::<FocusRow>(TableId::Focus);
    check_table::<CompletedFocusRow>(TableId::CompletedFocuses);
    check_table::<ResearchTechnologyRow>(TableId::ResearchTechnologies);
    check_table::<ResearchSlotRow>(TableId::ResearchSlots);
    check_table::<TechnologyDefinitionRow>(TableId::TechnologyDefinitions);
    check_table::<PoliticsRow>(TableId::Politics);
    check_table::<PoliticalPartyRow>(TableId::PoliticalParties);
    check_table::<ActiveIdeaRow>(TableId::ActiveIdeas);
    check_table::<DecisionCountsRow>(TableId::DecisionCounts);
    check_table::<DecisionRow>(TableId::Decisions);
    check_table::<DynamicModifierRow>(TableId::DynamicModifiers);
    check_table::<PreferredTacticRow>(TableId::PreferredTactics);
    check_table::<CountryTacticUnlockRow>(TableId::CountryTacticUnlocks);
    check_table::<TechSharingGroupRow>(TableId::TechSharingGroups);
    check_table::<TechSharingMemberRow>(TableId::TechSharingMembers);
    check_table::<PowerBalanceRow>(TableId::PowerBalances);
    check_table::<PowerBalanceMemberRow>(TableId::PowerBalanceMembers);
    check_table::<PowerBalanceRangeRow>(TableId::PowerBalanceRanges);
    check_table::<PowerBalanceOverrideRow>(TableId::PowerBalanceOverrides);
    check_table::<ProductionLineRow>(TableId::ProductionLines);
    check_table::<CountryProductionModifiersRow>(TableId::CountryProductionModifiers);
    check_table::<ProductionLineModifiersRow>(TableId::ProductionLineModifiers);
    check_table::<StockpileRow>(TableId::Stockpiles);
    check_table::<DivisionEquipmentRow>(TableId::DivisionEquipment);
    check_table::<VariantDefinitionRow>(TableId::VariantDefinitions);
    check_table::<VariantModuleRow>(TableId::VariantModules);
    check_table::<VariantStatsRow>(TableId::VariantStats);
    check_table::<VariantChangeRow>(TableId::VariantChanges);
    check_table::<VariantBonusSnapshotRow>(TableId::VariantBonusSnapshots);
    check_table::<DesignCountryModifierRow>(TableId::DesignCountryModifiers);
    check_table::<DesignCountryTemplatesRow>(TableId::DesignCountryTemplates);
    check_table::<DesignObserverContextRow>(TableId::DesignObserverContext);
    check_table::<DesignOrganisationLivenessRow>(TableId::DesignOrganisationLiveness);
    check_table::<VariantUpgradeRow>(TableId::VariantUpgrades);
    check_table::<ArmyUpgradeGroupRow>(TableId::ArmyUpgradeGroups);
    check_table::<ArmyUpgradeRequestRow>(TableId::ArmyUpgradeRequests);
    check_table::<ArmyUpgradeDeliveryRow>(TableId::ArmyUpgradeDeliveries);
    check_table::<ArmyUpgradeProducedRow>(TableId::ArmyUpgradeProduced);
    check_table::<CountryFuelRow>(TableId::CountryFuel);
    check_table::<CountryFuelConsumerRow>(TableId::CountryFuelConsumers);
    check_table::<CountryFactoriesRow>(TableId::CountryFactories);
    check_table::<ResourceDefinitionRow>(TableId::ResourceDefinitions);
    check_table::<CountryResourceRow>(TableId::CountryResources);
    check_table::<ResourceExchangeRow>(TableId::ResourceExchanges);
    check_table::<PurchaseContractRow>(TableId::PurchaseContracts);
    check_table::<ProvinceSupplyRow>(TableId::ProvinceSupply);
    check_table::<HubMotorizationRow>(TableId::HubMotorization);
    check_table::<DisruptedSupplyRow>(TableId::DisruptedSupply);
    check_table::<ForeignHomebaseRow>(TableId::ForeignHomebases);
    check_table::<CountryExperienceRow>(TableId::CountryExperience);
    check_table::<CountryExperienceGainRow>(TableId::CountryExperienceGains);
    check_table::<BuildingRow>(TableId::StateBuildings);
    check_table::<BuildingRow>(TableId::ProvinceBuildings);
    check_table::<StateBaseResourceRow>(TableId::StateBaseResources);
    check_table::<BuildingConstructionRow>(TableId::BuildingConstruction);
    check_table::<RailwayConstructionRow>(TableId::RailwayConstruction);
    check_table::<OccupationSettingsRow>(TableId::OccupationSettings);
    check_table::<StateOccupationLawRow>(TableId::StateOccupationLaws);
    check_table::<StateGarrisonRow>(TableId::StateGarrisons);
    check_table::<StateAssignedManpowerRow>(TableId::StateAssignedManpower);
    check_table::<CountryOccupationReserveManpowerRow>(TableId::CountryOccupationReserveManpower);
    check_table::<StateResistanceRow>(TableId::StateResistance);
    check_table::<CountryCollaborationRow>(TableId::CountryCollaboration);
    check_table::<CharacterRow>(TableId::Characters);
    check_table::<CountryLeaderRoleRow>(TableId::CountryLeaderRoles);
    check_table::<AdvisorRoleRow>(TableId::AdvisorRoles);
    check_table::<UnitLeaderRow>(TableId::UnitLeaders);
    check_table::<UnitLeaderTraitRow>(TableId::UnitLeaderTraits);
    check_table::<UnitLeaderTraitExperienceRow>(TableId::UnitLeaderTraitExperience);
    check_table::<ActiveAbilityRow>(TableId::ActiveAbilities);
    check_table::<CapitulationRow>(TableId::Capitulation);
    check_table::<SubjectRow>(TableId::Subjects);
    check_table::<AutonomyRow>(TableId::Autonomy);
    check_table::<IntelligenceAgencyRow>(TableId::IntelligenceAgencies);
    check_table::<OperativeMembershipRow>(TableId::OperativeMemberships);
    check_table::<CryptologyRow>(TableId::Cryptology);
    check_table::<IntelNetworkRow>(TableId::IntelNetworks);
    check_table::<IntelNetworkSummaryRow>(TableId::IntelNetworkSummaries);
    check_table::<IntelContributionRow>(TableId::IntelContributions);
    check_table::<IntelNetworkAuxiliaryRow>(TableId::IntelNetworkAuxiliaries);
    check_table::<IntelSourceRow>(TableId::IntelSources);
    check_table::<PropagandaEffortRow>(TableId::PropagandaEfforts);
    check_table::<ScriptFlagRow>(TableId::ScriptFlags);
    check_table::<ScriptVariablesRow>(TableId::ScriptVariables);
    check_table::<GlobalSavedEventTargetRow>(TableId::GlobalSavedEventTargets);
    check_table::<EquipmentDestructionRow>(TableId::EquipmentDestruction);
    check_table::<DivisionEquipmentDeletionRow>(TableId::DivisionEquipmentDeletions);
    check_table::<AirLossEventRow>(TableId::AirLossEvents);
    check_table::<AirDailyRosterRow>(TableId::AirDailyRoster);
    check_table::<RocketProductionOutputRow>(TableId::RocketProductionOutput);
    check_table::<BuildingBombingDamageRow>(TableId::BuildingBombingDamage);
    check_table::<ReserveModifierBreakdownRow>(TableId::ReserveModifierBreakdowns);
    check_table::<PowerBalanceModifierBreakdownRow>(TableId::PowerBalanceModifierBreakdowns);
    check_table::<NavalModifierBreakdownRow>(TableId::NavalModifierBreakdowns);
    check_table::<OperationRow>(TableId::Operations);
    check_table::<AgencyUpgradeLevelRow>(TableId::AgencyUpgradeLevels);
    check_table::<OperationCompletionRow>(TableId::OperationCompletions);
    check_table::<OperationAssignmentRow>(TableId::OperationAssignments);
    check_table::<WingEquipmentRow>(TableId::WingEquipment);
    check_table::<AirBaseRow>(TableId::AirBases);
    check_table::<WingCumulativeRow>(TableId::WingCumulative);
    check_table::<AirRegionHistoryRow>(TableId::AirRegionHistory);
    check_table::<AirRegionDamageRow>(TableId::AirRegionDamage);
    check_table::<AirRegionCombatRow>(TableId::AirRegionCombat);
    check_table::<AirAceRow>(TableId::AirAces);
    check_table::<AirStatisticsRow>(TableId::AirStatistics);
    check_table::<SunkConvoyRow>(TableId::SunkConvoys);
    check_table::<RailCooldownRow>(TableId::RailCooldowns);
    check_table::<RailEdgeRow>(TableId::RailEdges);
    check_table::<LandTheatreRow>(TableId::LandTheatres);
    check_table::<LandTheatreGroupRow>(TableId::LandTheatreGroups);
    check_table::<LandTheatreMemberRow>(TableId::LandTheatreMembers);
    check_table::<AirTheatreRow>(TableId::AirTheatres);
    check_table::<AirTheatreGroupRow>(TableId::AirTheatreGroups);
    check_table::<AirTheatreMemberRow>(TableId::AirTheatreMembers);
    check_table::<NavalResultRow>(TableId::NavalResults);
    check_table::<NavalResultSourceRow>(TableId::NavalResultSources);
    check_table::<NavalResultMemberRow>(TableId::NavalResultMembers);
    check_table::<NavalResultCountryRow>(TableId::NavalResultCountries);
    check_table::<SunkShipsRow>(TableId::SunkShips);
    check_table::<RaidTooltipOperandRow>(TableId::RaidTooltipOperands);
    check_table::<CountryCharacterRow>(TableId::CountryRetiredCharacters);
    check_table::<CharacterRoleTraitRow>(TableId::CharacterRoleTraits);
    check_table::<CharacterOperativeRow>(TableId::CharacterOperatives);
    check_table::<RaidIntelInputRow>(TableId::RaidIntelInputs);
    check_table::<RaidAirScoreRow>(TableId::RaidAirScores);
    check_table::<RaidFacilityRow>(TableId::RaidFacilities);
    check_table::<ProjectProgramRow>(TableId::ProjectPrograms);
    check_table::<ProjectRow>(TableId::Projects);
    check_table::<ProjectHistoryRow>(TableId::ProjectHistory);
    check_table::<ProjectBreakthroughRow>(TableId::ProjectBreakthroughs);
    check_table::<NukeRow>(TableId::Nukes);
    check_table::<RaidCountryRow>(TableId::RaidCountries);
    check_table::<RaidRow>(TableId::Raids);
    check_table::<RaidTargetRow>(TableId::RaidTargets);
    check_table::<ResearchModifierRow>(TableId::ResearchModifiers);
    check_table::<ResearchInputRow>(TableId::ResearchInputs);
    check_table::<ResearchAppliedBonusRow>(TableId::ResearchAppliedBonuses);
    check_table::<NavalBaseRow>(TableId::NavalBases);
    check_table::<BattleSoundRow>(TableId::BattleSounds);
    check_table::<PeaceConferenceRow>(TableId::PeaceConferences);
    check_table::<PeaceSettlementRow>(TableId::PeaceSettlements);
    check_table::<NuclearStrikeRow>(TableId::NuclearStrikes);
    check_table::<PeaceSettlementRow>(TableId::PeaceBids);
    check_table::<WeatherRegionRow>(TableId::WeatherRegions);
    check_table::<WeatherProvinceRow>(TableId::WeatherProvinces);
    check_table::<OperationTokensRow>(TableId::OperationTokens);
    check_table::<LexicalTokenRow>(TableId::LexicalTokens);
    check_table::<NavalSupremacyRow>(TableId::NavalSupremacy);
    check_table::<RadarCoverageRow>(TableId::RadarCoverage);
    check_table::<SentVolunteerRow>(TableId::SentVolunteers);
    check_table::<LogisticsEquipmentRow>(TableId::LogisticsEquipment);
    check_table::<IndustrialOrganisationRow>(TableId::IndustrialOrganisations);
    check_table::<IndustrialOrganisationHistoryRow>(TableId::IndustrialOrganisationHistory);
    check_table::<IndustrialOrganisationPoliciesRow>(TableId::IndustrialOrganisationPolicies);
    check_table::<IndustrialOrganisationDetailsRow>(TableId::IndustrialOrganisationDetails);
    check_table::<ProductionLineOutputRow>(TableId::ProductionLineOutput);
    check_table::<ConstructionLineOutputRow>(TableId::ConstructionLineOutput);
    check_table::<CombatLogEquipmentRow>(TableId::CombatLogEquipment);
    check_table::<CombatLogStateRow>(TableId::CombatLogState);
    check_table::<CombatLogObserverRow>(TableId::CombatLogObservers);
    check_table::<CombatLogBattleSideRow>(TableId::CombatLogBattleSides);
    check_table::<CombatLogBattleEquipmentRow>(TableId::CombatLogBattleEquipment);
    check_table::<CombatLogBattleModifierRow>(TableId::CombatLogBattleModifiers);
    check_table::<CombatLogBattleCountryRow>(TableId::CombatLogBattleCountries);

    check_table::<CombatLogGroupRow>(TableId::CombatLogGroups);
    check_table::<CombatLogMonthRow>(TableId::CombatLogMonths);
    check_table::<CombatLogManpowerRow>(TableId::CombatLogManpower);
    check_table::<CombatLogTemplateRow>(TableId::CombatLogTemplates);
    check_table::<CombatLogReferenceRow>(TableId::CombatLogReferences);
    check_table::<CombatLogBattleRow>(TableId::CombatLogBattles);
    check_table::<CombatLogEnemyEquipmentRow>(TableId::CombatLogEnemyEquipment);

    check_table::<LendLeaseEquipmentRow>(TableId::LendLeaseEquipment);
    check_table::<DivisionCensusRow>(TableId::DivisionCensus);
    check_table::<DivisionLandInputsRow>(TableId::DivisionLandInputs);
    check_table::<LandCombatResultsRow>(TableId::LandCombatResults);
    check_table::<DestroyedDivisionsRow>(TableId::DestroyedDivisions);
    check_table::<LandEventCoverageRow>(TableId::LandEventCoverage);
    check_table::<VolunteerHostRow>(TableId::VolunteerHosts);
    check_table::<TheatreAreaRow>(TableId::TheatreAreas);
    check_table::<StationedUnitRow>(TableId::StationedUnits);
    check_table::<UnitIntelRow>(TableId::UnitIntel);
    check_table::<AirBaseRegistrationRow>(TableId::AirBaseRegistrations);
    check_table::<ProductionLicenseRow>(TableId::ProductionLicenses);
    check_table::<ProductionLicenseRecordRow>(TableId::ProductionLicenseRecords);
    check_table::<DeploymentConveyorRow>(TableId::DeploymentConveyors);
    check_table::<DeploymentLineRow>(TableId::DeploymentLines);
    check_table::<DeploymentManpowerRow>(TableId::DeploymentManpower);
    check_table::<DeploymentEquipmentRow>(TableId::DeploymentEquipment);
    check_table::<WorldTensionRow>(TableId::WorldTension);
    check_table::<ProvinceNameRow>(TableId::ProvinceNames);
    check_table::<StateFlagRow>(TableId::StateFlags);
    check_table::<ArmyOrderPlanRow>(TableId::ArmyOrderPlans);
    check_table::<OrdersGroupPlanRow>(TableId::OrdersGroupPlans);
    check_table::<FrontRow>(TableId::Fronts);
    check_table::<CountryAppearanceRow>(TableId::CountryAppearance);
    check_table::<ArmyIconInputRow>(TableId::ArmyIconInputs);
    check_table::<ShipModelRow>(TableId::ShipModels);
    check_table::<ProvinceUnitOrderRow>(TableId::ProvinceUnitOrder);
    check_table::<CharacterPortraitRow>(TableId::CharacterPortraits);
    check_table::<LeaderPortraitRow>(TableId::LeaderPortraits);
    check_table::<TaskForceCounterInputRow>(TableId::TaskForceCounterInputs);
    check_table::<RailwayGunCounterInputRow>(TableId::RailwayGunCounterInputs);
    check_table::<TaskForceMapMotionRow>(TableId::TaskForceMapMotion);
    check_table::<FloatingHarborDeploymentRow>(TableId::FloatingHarborDeployments);
    check_table::<FacilityMapFlagRow>(TableId::FacilityMapFlags);
    check_table::<NavalProvinceCombatRow>(TableId::NavalProvinceCombats);
    check_table::<NavalCombatTransferRow>(TableId::NavalCombatTransfers);
    check_table::<NavalTransferUnitRow>(TableId::NavalTransferUnits);
    check_table::<TaskForceMissionRow>(TableId::TaskForceMissions);
    check_table::<NavalCombatAirBaseWingHandlesRow>(TableId::NavalCombatAirBaseWingHandles);
    check_table::<FleetCounterInputRow>(TableId::FleetCounterInputs);
    check_table::<CountryCounterInputRow>(TableId::CountryCounterInputs);
    check_table::<OrdersGroupCounterInputRow>(TableId::OrdersGroupCounterInputs);
    check_table::<LeaderPortraitPathRow>(TableId::LeaderPortraitPaths);
    check_table::<SupplyNodeRow>(TableId::SupplyNodes);
    check_table::<LandCombatDetailsRow>(TableId::LandCombatDetails);
    check_table::<LandCombatDivisionRow>(TableId::LandCombatDivisions);
    check_table::<LandCombatModifierRow>(TableId::LandCombatModifiers);
    check_table::<LandCombatDivisionModifierRow>(TableId::LandCombatDivisionModifiers);
    check_table::<FuelDemandPartRow>(TableId::FuelDemandParts);
    check_table::<MapEventsRow>(TableId::MapEvents);
    check_table::<OwnerTransportSpeedRow>(TableId::OwnerTransportSpeeds);
    check_table::<ReserveModifierTreeRow>(TableId::ReserveModifierTrees);
    check_table::<OperativeMissionRow>(TableId::OperativeMissions);
    check_table::<StrategicOperativeRow>(TableId::StrategicOperatives);
    check_table::<AgencyOperationBuffersRow>(TableId::AgencyOperationBuffers);
    check_table::<ResourceOriginRow>(TableId::ResourceOrigins);
    check_table::<ResourceRightRow>(TableId::ResourceRights);
    check_table::<CountryOilFactorRow>(TableId::CountryOilFactors);
    check_table::<StateTemporaryResourceRow>(TableId::StateTemporaryResources);
    check_table::<ConvoyClientRow>(TableId::ConvoyClients);
    check_table::<TradeRouteRow>(TableId::TradeRoutes);
    check_table::<CountryWargoalRow>(TableId::CountryWargoals);
    check_table::<WargoalTargetRow>(TableId::WargoalTargets);
    check_table::<WargoalJustificationRow>(TableId::WargoalJustifications);
    check_table::<SharedDivisionHistoryRow>(TableId::DivisionHistory);
    check_table::<SharedDivisionPanelSourceRow>(TableId::DivisionPanelSources);
    check_table::<RegionalConvoyEfficiencyRow>(TableId::RegionalConvoyEfficiency);
    check_table::<FactionRuleRow>(TableId::FactionRules);
    check_table::<FactionProgramRow>(TableId::FactionPrograms);
    check_table::<FactionMinimumUpgradeRow>(TableId::FactionMinimumUpgrades);
    check_table::<FactionDoctrineRow>(TableId::FactionDoctrine);
    check_table::<FactionProjectionRow>(TableId::FactionProjection);
    check_table::<FactionResourceRow>(TableId::FactionResources);
    check_table::<FactionAdvisorModifierRow>(TableId::FactionAdvisorModifiers);
    check_table::<FactionResearchRow>(TableId::FactionResearch);
    check_table::<FactionAccountRow>(TableId::FactionAccounts);
    check_table::<FactionVariableRow>(TableId::FactionVariables);
    check_table::<FactionUnlockRow>(TableId::FactionUnlocks);
    check_table::<FactionManpowerRow>(TableId::FactionManpower);
    check_table::<FactionGoalStatusRow>(TableId::FactionGoalStatus);
    check_table::<FactionGoalEntryRow>(TableId::FactionGoalEntries);
    check_table::<MovementStateTermRow>(TableId::MovementStateTerms);
    check_table::<CoreSupplyAccessRow>(TableId::CoreSupplyAccess);
    check_table::<CountryUnitSpeedEntryRow>(TableId::CountryUnitSpeedEntries);
    check_table::<RiverRelationRow>(TableId::RiverRelations);
    check_table::<OwnerTransportSpeedRow>(TableId::DivisionTransportSpeeds);
    check_table::<NavalCombatDetailsRow>(TableId::NavalCombatDetails);
    check_table::<NavalCombatShipRow>(TableId::NavalCombatShips);
    check_table::<NavalCombatResultDetailsRow>(TableId::NavalCombatResultDetails);
    check_table::<TrainInventoryRow>(TableId::TrainInventory);
    check_table::<LocalPlayerRow>(TableId::LocalPlayer);
    check_table::<LandUnitOrderRow>(TableId::LandUnitOrder);
    check_table::<SimulationFeatureRow>(TableId::SimulationFeatures);
    check_table::<CountryVisibilityRow>(TableId::CountryVisibility);
    check_table::<StateVisibilityRow>(TableId::StateVisibility);
    check_table::<RaidTargetStatusRow>(TableId::RaidTargetStatuses);
    check_table::<RaidCooldownRow>(TableId::RaidCooldowns);
    check_table::<RaidPathDistanceRow>(TableId::RaidPathDistances);
    check_table::<StateResourceInputRow>(TableId::StateResourceInputs);
    check_table::<StateBuildingInputRow>(TableId::StateBuildingInputs);
    check_table::<StateResourceRow>(TableId::StateResources);
    check_table::<StateDynamicModifierRow>(TableId::StateDynamicModifiers);
    check_table::<CountryBuildingResourceRow>(TableId::CountryBuildingResources);
    check_table::<RaidCountryInputRow>(TableId::RaidCountryInputs);
    check_table::<CountryCommandPowerRow>(TableId::CountryCommandPower);
    check_table::<DivisionRetreatBanRow>(TableId::DivisionRetreatBans);
    check_table::<StateCoreRow>(TableId::StateCores);
    check_table::<RaidEquipmentRow>(TableId::RaidEquipment);
    check_table::<InvasionPreparationRow>(TableId::InvasionPreparations);
    check_table::<CountryCharacterRow>(TableId::CountryCharacters);
    check_table::<PartyLeaderRow>(TableId::PartyLeaders);
    check_table::<CountryCivilWarRow>(TableId::CountryCivilWars);
    check_table::<AdvisorTokenRow>(TableId::AdvisorTokens);
    check_table::<FactionAdvisorRow>(TableId::FactionAdvisors);
    check_table::<RaidSpeedInputRow>(TableId::RaidSpeedInputs);
    check_table::<StockpileArchetypeRow>(TableId::StockpileArchetypes);
    check_table::<RaidAirSiteRow>(TableId::RaidAirSites);
    check_table::<AceRangeInputRow>(TableId::AceRangeInputs);
    check_table::<CountryWarRow>(TableId::CountryWars);
    check_table::<NavalRegionAccessRow>(TableId::NavalRegionAccess);
    check_table::<BorderConflictRow>(TableId::BorderConflicts);
    check_table::<RaidPlayerAllowedTypeRow>(TableId::RaidPlayerAllowedTypes);
    check_table::<RelationLinkRow>(TableId::RelationLinks);
    check_table::<SliceReadRow>(TableId::SliceReads);
    check_table::<FactionTheatreRow>(TableId::FactionTheatres);
    check_table::<CountryLocalSubunitRow>(TableId::CountryLocalSubunits);
    check_table::<AvailableEquipmentVariantRow>(TableId::AvailableEquipmentVariants);
    check_table::<CountryUnitBonusRow>(TableId::CountryUnitBonuses);
    check_table::<DivisionEquipmentRow>(TableId::NewDivisionEquipment);
    check_table::<AirRegionalMissionRow>(TableId::AirRegionalMissions);
    check_table::<AirGroundCrewRow>(TableId::AirGroundCrews);
    check_table::<AirTransferInputRow>(TableId::AirTransferInputs);
    check_table::<NavalAccidentReportRow>(TableId::NavalAccidentReports);
    check_table::<NavalRegionalMineRow>(TableId::NavalRegionalMines);
    check_table::<NavalMissionTaskForceRow>(TableId::NavalDominanceTaskForceRanges);
    check_table::<NavalPatrolMissionInputRow>(TableId::NavalPatrolMissionInputs);
    check_table::<AirCountryRelationRow>(TableId::AirCountryRelations);
    check_table::<AirReferenceWitnessRow>(TableId::AirReferenceWitnesses);
    check_table::<NavalMineLevelRow>(TableId::NavalMineLevels);
    check_table::<AirNavalPatrolCombatRow>(TableId::AirNavalPatrolCombats);
    check_table::<AirNavalPatrolHistoryRow>(TableId::AirNavalPatrolHistories);
    check_table::<AirMissionInputRow>(TableId::AirMissionInputs);
    check_table::<AirSortieBaseCountryRow>(TableId::AirSortieBaseCountries);
    check_table::<AirSortieModifierRow>(TableId::AirSortieModifiers);
    check_table::<AirSortieSpecialBaseRow>(TableId::AirSortieSpecialBases);
    check_table::<AirSortieModifierMetadataRow>(TableId::AirSortieModifierMetadata);
    check_table::<ModifierDefinitionRow>(TableId::ModifierDefinitions);
    check_table::<OwnedWargoalRow>(TableId::OwnedWargoals);
    check_table::<AvailableAdvisorRow>(TableId::AvailableAdvisors);
    check_table::<TimedIdeaRow>(TableId::TimedIdeas);
    check_table::<DecisionRandomItemRow>(TableId::DecisionRandomItems);
    check_table::<CapturedOperativeReferenceRow>(TableId::CapturedOperativeReferences);
    check_table::<ReservedCountryNamingRow>(TableId::ReservedCountryNaming);
    check_table::<ReservedNearestNavyRow>(TableId::ReservedNearestNavies);
    check_table::<ReservedNearestBaseRow>(TableId::ReservedNearestBases);
    check_table::<TransportNavyRow>(TableId::TransportNavies);
    check_table::<TransportLedgerRow>(TableId::TransportLedgers);
    check_table::<TransportReceiverRow>(TableId::TransportReceivers);
    check_table::<TransportSearchRegionRow>(TableId::TransportSearchRegions);
    check_table::<TransportSpottingOwnerRow>(TableId::TransportSpottingOwners);
    check_table::<TransportSpottingRegionRow>(TableId::TransportSpottingRegions);
    check_table::<TransportAirRootRow>(TableId::TransportAirRoots);
    check_table::<TransportAirRegionRow>(TableId::TransportAirRegions);
    check_table::<TransportRiskShipsRow>(TableId::TransportRiskShips);
    check_table::<AttritionModifierBreakdownRow>(TableId::AttritionModifierBreakdowns);
    check_table::<HiredAdvisorRow>(TableId::HiredAdvisors);
    check_table::<CountryStabilityInputRow>(TableId::CountryStabilityInputs);
    check_table::<CountryStabilityLineRow>(TableId::CountryStabilityLines);
    check_table::<NavalMissionTaskForceRow>(TableId::NavalMissionTaskForces);
    check_table::<NavalMissionLedgerRow>(TableId::NavalMissionLedgers);
    check_table::<NavalMissionCountryRow>(TableId::NavalMissionCountries);
    check_table::<NavalMissionCommanderRow>(TableId::NavalMissionCommanders);
    check_table::<CountryTemplateStateRow>(TableId::CountryTemplateState);
    check_table::<DivisionTemplateRow>(TableId::DivisionTemplates);
    check_table::<DivisionTemplateIconRow>(TableId::DivisionTemplateIcons);
    check_table::<DivisionMovementRow>(TableId::DivisionMovement);
    check_table::<DivisionPanelStatsRow>(TableId::DivisionPanelStats);
    check_table::<DivisionStatusRow>(TableId::DivisionStatus);
    check_table::<DivisionModelInputRow>(TableId::DivisionModelInputs);
    check_table::<StrategicRegionWeightRow>(TableId::StrategicRegionWeights);
    check_table::<DivisionEtaInputsRow>(TableId::DivisionEtaInputs);
    check_table::<LandCombatProgressRow>(TableId::LandCombatProgress);
    check_table::<LandCombatSideRow>(TableId::LandCombatSides);
    check_table::<CombatEngagementRefreshRow>(TableId::CombatEngagementRefreshes);
    check_table::<LandCombatEdgesRow>(TableId::LandCombatEdges);
    check_table::<FuelAwardRecipientRow>(TableId::FuelAwardRecipients);
    check_table::<ScriptVariableNameRow>(TableId::ScriptVariableNames);
    check_table::<ScriptVariableValueRow>(TableId::ScriptVariableValues);
    check_table::<CombatLogObserverRow>(TableId::LandCombatResultObservers);
    check_table::<ThreatSourceRow>(TableId::ThreatSources);
    check_table::<ThreatCountryAggregateRow>(TableId::ThreatCountryAggregates);
    check_table::<RelationOpinionRow>(TableId::RelationOpinions);
    check_table::<RelationTupleRow>(TableId::RelationTuples);
    check_table::<CountryIntelRow>(TableId::CountryIntel);
}

#[test]
fn movement_operand_rows_keep_refusal_absence_and_zero_distinct() {
    let mut encodings = std::collections::BTreeSet::new();
    for entry in [None, Some(None), Some(Some(0)), Some(Some(i64::MIN))] {
        let row = CountryUnitSpeedEntryRow {
            country: u32::MAX,
            key: 0x16d,
            entry,
        };
        round_trip(&row);
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        assert!(encodings.insert(bytes), "{entry:?}");
        round_trip(&RiverRelationRow {
            owner: 1,
            controller: u32::MAX,
            entry,
        });
    }
    let mut encodings = std::collections::BTreeSet::new();
    for access in [None, Some(false), Some(true)] {
        let row = CoreSupplyAccessRow {
            owner: 7,
            province: u32::MAX,
            access,
        };
        round_trip(&row);
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        assert!(encodings.insert(bytes), "{access:?}");
    }
    let terms = StateMovementTerms {
        army_speed: Some(i64::MIN),
        attack_speed: None,
        relation_speed: Some(i64::MAX),
        branch: StateMovementTerms::ENEMY,
    };
    for terms in [None, Some(StateMovementTerms::default()), Some(terms)] {
        let row = MovementStateTermRow {
            state: u32::MAX,
            roster: 2,
            owner: u32::MAX,
            terms,
        };
        assert_eq!(row.key() >> 64, u32::MAX as u128);
        round_trip(&row);
    }
}

#[test]
fn political_runtime_fixture_suffix_deltas_leave_prefixes_unchanged() {
    let old = DivisionStatusRow::fixture(u64::MAX as u128, 1);
    let next = DivisionStatusRow {
        dig_in_factor: Recorded::Value(Some(i64::MIN)),
        ..old.clone()
    };
    selected_route_suffix(TableId::DivisionStatus, 20, &old, &next);
    let next = DivisionStatusRow {
        attack_stats: Recorded::Value(Some(DivisionAttackStats {
            terrain: vec![i64::MIN, 0, -15_000],
            river: [-30_000, -60_000],
            fort: [i64::MAX, 0],
            snow: -1,
            night: 7,
        })),
        ..old.clone()
    };
    selected_route_suffix(TableId::DivisionStatus, 24, &old, &next);
    let next = DivisionStatusRow {
        weather_factor: Recorded::Value(Some(i64::MIN)),
        ..old.clone()
    };
    selected_route_suffix(TableId::DivisionStatus, 25, &old, &next);
    let next = DivisionStatusRow {
        name_template: Recorded::Value(Witnessed(Some(3005 << 32 | 52))),
        ..old.clone()
    };
    selected_route_suffix(TableId::DivisionStatus, 26, &old, &next);
    for combat_movement in [Some(None), Some(Some(i64::MIN)), None] {
        let next = DivisionStatusRow {
            combat_movement: Recorded::Value(combat_movement),
            ..old.clone()
        };
        selected_route_suffix(TableId::DivisionStatus, 37, &old, &next);
    }
    for value in [None, Some(924_554)] {
        let next = DivisionStatusRow {
            organisation_exact: Recorded::Value(value),
            ..old.clone()
        };
        selected_route_suffix(TableId::DivisionStatus, 38, &old, &next);
        let next = DivisionStatusRow {
            maximum_organisation_exact: Recorded::Value(value),
            ..old.clone()
        };
        selected_route_suffix(TableId::DivisionStatus, 39, &old, &next);
    }
    let next = DivisionStatusRow {
        move_flags: old.move_flags ^ DivisionStatusRow::MOVE_NONCORE_AIR,
        ..old.clone()
    };
    selected_route_suffix(TableId::DivisionStatus, 12, &old, &next);

    let old = DivisionMovementRow::fixture(u64::MAX as u128, 1);
    let mut next = old.clone();
    next.combat_references
        .require_mut("DivisionMovement.combat_references")
        .unwrap()
        .as_mut()
        .unwrap()
        .entries[0]
        .kind = 2;
    selected_route_suffix(TableId::DivisionMovement, 18, &old, &next);
    let old = RailwayGunCounterInputRow::fixture(u64::MAX as u128, 1);
    let mut next = old.clone();
    next.combat_references
        .require_mut("RailwayGunCounterInputs.combat_references")
        .unwrap()
        .as_mut()
        .unwrap()
        .entries[0]
        .kind = 2;
    selected_route_suffix(TableId::RailwayGunCounterInputs, 28, &old, &next);

    let old = CountryCounterInputRow::fixture(0, 0);
    let mut directed_wars = DirectedWars::sample(0);
    directed_wars.receiver_raw_id = old.country as i32;
    let next = CountryCounterInputRow {
        directed_wars: Recorded::Value(Some(directed_wars)),
        ..old.clone()
    };
    selected_route_suffix(TableId::CountryCounterInputs, 18, &old, &next);
    let next = CountryCounterInputRow {
        attack_modifiers: Recorded::Value(Some(CountryAttackModifiers {
            land_night_attack: Some(i64::MIN),
            land_bunker_effectiveness_factor: None,
        })),
        ..old.clone()
    };
    selected_route_suffix(TableId::CountryCounterInputs, 19, &old, &next);

    let old = DivisionTemplateIconRow::fixture(u64::MAX as u128, 1);
    let mut next = old.clone();
    next.display_name
        .require_mut("DivisionTemplateIcons.display_name")
        .unwrap()
        .as_mut()
        .unwrap()
        .value = TemplateNameValue::SameAsDivisionTemplates;
    selected_route_suffix(TableId::DivisionTemplateIcons, 14, &old, &next);
    let mut next = old.clone();
    let grids = next
        .grids
        .require_mut("DivisionTemplateIcons.grids")
        .unwrap();
    let moved = TemplateSubunit {
        token: u32::MAX,
        local: 1,
        count: i32::MAX,
    };
    grids.support.push(moved.clone());
    let mut other = next.clone();
    let other_grids = other
        .grids
        .require_mut("DivisionTemplateIcons.grids")
        .unwrap();
    other_grids.support.pop();
    other_grids.regimental_support.push(moved);
    selected_route_suffix(TableId::DivisionTemplateIcons, 15, &old, &next);
    selected_route_suffix(TableId::DivisionTemplateIcons, 15, &next, &other);
    for speed_keys in [Some(vec![u32::MAX, 7, 7]), Some(Vec::new()), None] {
        let next = DivisionTemplateIconRow {
            speed_keys: Recorded::Value(speed_keys),
            ..old.clone()
        };
        selected_route_suffix(TableId::DivisionTemplateIcons, 20, &old, &next);
    }

    let mut next = old.clone();
    next.layout = Recorded::Value(Some(
        TemplateLayout {
            regiments: TemplateGrid {
                columns: 2,
                rows: 3,
                cells: vec![
                    TemplateCell {
                        token: 17,
                        local: 1,
                        occupied: true,
                    },
                    TemplateCell::default(),
                ],
            },
            ..Default::default()
        }
        .into(),
    ));
    selected_route_suffix(TableId::DivisionTemplateIcons, 18, &old, &next);
    let mut other = next.clone();
    std::sync::Arc::make_mut(
        other
            .layout
            .require_mut("layout")
            .unwrap()
            .as_mut()
            .unwrap(),
    )
    .regiments
    .cells
    .swap(0, 1);
    selected_route_suffix(TableId::DivisionTemplateIcons, 18, &next, &other);
    let mut next = old.clone();
    std::sync::Arc::make_mut(
        next.settings
            .require_mut("settings")
            .unwrap()
            .as_mut()
            .unwrap(),
    )
    .priority = -99;
    selected_route_suffix(TableId::DivisionTemplateIcons, 19, &old, &next);
    let mut next = old.clone();
    let settings = std::sync::Arc::make_mut(
        next.settings
            .require_mut("settings")
            .unwrap()
            .as_mut()
            .unwrap(),
    );
    settings.names_group = Some(b"MOD_RENAMED_GROUP".to_vec());
    settings.roster_ordinal = settings.roster_ordinal.wrapping_add(1);
    selected_route_suffix(TableId::DivisionTemplateIcons, 19, &old, &next);

    let old = SimulationFeatureRow::fixture(0, 0);
    let mut next = old.clone();
    next.runtime_scalars
        .require_mut("SimulationFeatures.runtime_scalars")
        .unwrap()
        .move_strength_threshold = Some(0);
    selected_route_suffix(TableId::SimulationFeatures, 4, &old, &next);
}

#[test]
fn division_movement_style_numeric_schema_round_trips_extremes_and_refusals() {
    let baseline = DivisionMovementRow {
        withdrawal: Recorded::Value(None),
        movement_priority: Recorded::Value(None),
        ..DivisionMovementRow::fixture(u64::MAX as u128, 4)
    };
    for priority in [None, Some(0), Some(-1), Some(i32::MIN), Some(i32::MAX)] {
        for withdrawal in [None, Some(0xfe)] {
            let row = DivisionMovementRow {
                withdrawal: Recorded::Value(withdrawal),
                movement_priority: Recorded::Value(priority),
                ..baseline.clone()
            };
            round_trip(&row);
        }
    }
    let next = DivisionMovementRow {
        movement_priority: Recorded::Value(Some(i32::MIN)),
        ..baseline.clone()
    };
    let delta = next.delta(Some(&baseline)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 16);
    let mut bytes = Vec::new();
    encode_delta(TableId::DivisionMovement, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::DivisionMovement, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(baseline.clone())).unwrap(), next);

    let legacy = DivisionMovementRow {
        withdrawal: Recorded::NotRecorded,
        movement_priority: Recorded::NotRecorded,
        ..baseline
    };
    assert_eq!(legacy.not_recorded_fields(), (1 << 15) | (1 << 16));
    assert!(encode_row(&legacy, 0, &mut Vec::new()).is_err());
}

#[test]
fn task_force_motion_numeric_schema_round_trips_extremes_and_independent_refusals() {
    let baseline = TaskForceMapMotionRow {
        movement_numerator: Recorded::Value(None),
        retreat: Recorded::Value(None),
        withdrawal: Recorded::Value(None),
        naval_transfer: Recorded::Value(None),
        combat_links: Recorded::Value(Some([0; 2])),
        ..TaskForceMapMotionRow::fixture(u64::MAX as u128, 4)
    };
    for (numerator, retreat, withdrawal, transfer) in [
        (None, None, Some(0xfe), Some(1)),
        (Some(0), Some(0), None, Some(1)),
        (Some(-1), Some(0xfe), Some(0), None),
        (Some(i64::MIN), None, None, None),
        (Some(i64::MAX), Some(u8::MAX), Some(u8::MAX), Some(0)),
    ] {
        round_trip(&TaskForceMapMotionRow {
            movement_numerator: Recorded::Value(numerator),
            retreat: Recorded::Value(retreat),
            withdrawal: Recorded::Value(withdrawal),
            naval_transfer: Recorded::Value(transfer),
            ..baseline.clone()
        });
    }
    let next = TaskForceMapMotionRow {
        movement_numerator: Recorded::Value(Some(i64::MIN)),
        ..baseline.clone()
    };
    let delta = next.delta(Some(&baseline)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 4);
    let mut bytes = Vec::new();
    encode_delta(TableId::TaskForceMapMotion, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::TaskForceMapMotion, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(baseline.clone())).unwrap(), next);

    let legacy = TaskForceMapMotionRow {
        movement_numerator: Recorded::NotRecorded,
        retreat: Recorded::NotRecorded,
        withdrawal: Recorded::NotRecorded,
        naval_transfer: Recorded::NotRecorded,
        combat_links: Recorded::NotRecorded,
        ..baseline
    };
    assert_eq!(
        legacy.not_recorded_fields(),
        (1 << 4) | (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8)
    );
    assert!(encode_row(&legacy, 0, &mut Vec::new()).is_err());
}

#[test]
fn task_force_combat_links_round_trip_raw_pairs_refusals_and_independent_delta() {
    let baseline = TaskForceMapMotionRow {
        combat_links: Recorded::Value(Some([0; 2])),
        ..TaskForceMapMotionRow::fixture(u64::MAX as u128, 4)
    };
    for value in [
        None,
        Some([0; 2]),
        Some([u64::MAX, 0]),
        Some([u64::MAX, u64::MAX]),
        Some([1, 1 << 63]),
    ] {
        round_trip(&TaskForceMapMotionRow {
            combat_links: Recorded::Value(value),
            ..baseline.clone()
        });
    }
    let next = TaskForceMapMotionRow {
        combat_links: Recorded::Value(Some([u64::MAX, 1 << 63])),
        ..baseline.clone()
    };
    let delta = next.delta(Some(&baseline)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 8);
    let mut bytes = Vec::new();
    encode_delta(TableId::TaskForceMapMotion, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::TaskForceMapMotion, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(baseline.clone())).unwrap(), next);
    let legacy = TaskForceMapMotionRow {
        combat_links: Recorded::NotRecorded,
        ..baseline
    };
    assert_eq!(legacy.not_recorded_fields(), 1 << 8);
    assert!(encode_row(&legacy, 0, &mut Vec::new()).is_err());
}

#[test]
fn task_force_fresh_eta_round_trips_components_and_changes_only_suffix() {
    let baseline = TaskForceMapMotionRow {
        fresh: Recorded::Value(None),
        ..TaskForceMapMotionRow::fixture(u64::MAX as u128, 4)
    };
    let full = FreshTaskForceCounterEta {
        sample_hour: PacketHour::Host,
        base_speed: Some(i64::MIN),
        speed_modifier: Some(i64::MAX),
        terrain_class: Some(i32::MIN),
        modified_range: Some(-1),
        exiled: Some(0xfe),
        strategic_redeployment: Some(u8::MAX),
    };
    let missing = FreshTaskForceCounterEta::default();
    for packet in [
        None,
        Some(full),
        Some(missing),
        Some(FreshTaskForceCounterEta {
            base_speed: None,
            speed_modifier: Some(0),
            modified_range: Some(i64::MAX),
            ..full
        }),
    ] {
        round_trip(&TaskForceMapMotionRow {
            fresh: Recorded::Value(packet),
            ..baseline.clone()
        });
    }
    let next = TaskForceMapMotionRow {
        fresh: Recorded::Value(Some(full)),
        ..baseline.clone()
    };
    let delta = next.delta(Some(&baseline)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 9);
    let mut bytes = Vec::new();
    encode_delta(TableId::TaskForceMapMotion, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::TaskForceMapMotion, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(baseline.clone())).unwrap(), next);
    let historical = TaskForceMapMotionRow {
        fresh: Recorded::NotRecorded,
        ..baseline
    };
    assert_eq!(historical.not_recorded_fields(), 1 << 9);
    assert!(encode_row(&historical, 0, &mut Vec::new()).is_err());
}

#[test]
fn land_death_without_a_selected_killer_differs_from_native_null() {
    let null = DestroyedDivisionsRow {
        id: 7,
        killer_division: Some(0),
        ..Default::default()
    };
    let unknown = DestroyedDivisionsRow {
        killer_division: None,
        ..null.clone()
    };
    round_trip(&null);
    round_trip(&unknown);
    let delta = unknown.delta(Some(&null)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 3);
    assert_eq!(
        delta.apply::<DestroyedDivisionsRow>(Some(null)).unwrap(),
        unknown
    );
}

#[test]
fn strategic_action_codec_round_trips_snapshots_and_modifier_only_delta() {
    let previous = DivisionMovementRow::fixture(u64::MAX as u128, 4);
    let mut next = previous.clone();
    next.strategic_action =
        crate::rows::Recorded::Value(StrategicAction::Available(StrategicRedeploymentInputs {
            friendly_state_modifier: Some(i64::MIN),
            ..strategic_action_fixture(4)
        }));
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 14);
    let mut bytes = Vec::new();
    encode_delta(TableId::DivisionMovement, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::DivisionMovement, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(previous)).unwrap(), next);
    next.strategic_action = crate::rows::Recorded::Value(StrategicAction::Withheld(
        StrategicActionFailure::CurrentState,
    ));
    round_trip(&next);
    assert!(next.delta(Some(&next)).unwrap().is_none());
}

#[test]
fn strategic_action_codec_preserves_skips_and_per_row_refusals() {
    let row = DivisionMovementRow::fixture(9, 4);
    for tick in 0..7 {
        let inputs = strategic_action_fixture(tick);
        assert_eq!(inputs.blocked(), matches!(tick, 0 | 2 | 5 | 6));
        round_trip(&DivisionMovementRow {
            strategic_action: crate::rows::Recorded::Value(StrategicAction::Available(inputs)),
            ..row.clone()
        });
    }
    for reason in [
        StrategicActionFailure::Owner,
        StrategicActionFailure::CountryModifier,
        StrategicActionFailure::CurrentState,
        StrategicActionFailure::StateModifier,
        StrategicActionFailure::Friendly,
        StrategicActionFailure::FriendlyStateModifier,
    ] {
        round_trip(&DivisionMovementRow {
            strategic_action: crate::rows::Recorded::Value(StrategicAction::Withheld(reason)),
            ..row.clone()
        });
    }
    assert!(
        encode_row(
            &DivisionMovementRow {
                strategic_action: crate::rows::Recorded::Value(StrategicAction::Unreached),
                ..row.clone()
            },
            0,
            &mut Vec::new()
        )
        .is_err()
    );
    for skipped in [
        DivisionMovementRow {
            path_total: u32::MAX,
            ..row.clone()
        },
        DivisionMovementRow {
            target: 3,
            ..row.clone()
        },
        DivisionMovementRow {
            retreat: 1,
            ..row.clone()
        },
        DivisionMovementRow {
            strategic_redeployment: 0,
            ..row.clone()
        },
    ] {
        assert!(encode_row(&skipped, 0, &mut Vec::new()).is_err());
        round_trip(&DivisionMovementRow {
            strategic_action: crate::rows::Recorded::Value(StrategicAction::Unreached),
            ..skipped
        });
    }
    let mut bytes = Vec::new();
    crate::value::Value::encode(
        row.strategic_action
            .require("DivisionMovement.strategic_action")
            .unwrap(),
        &mut bytes,
    )
    .unwrap();
    for (offset, bad_tag) in [(0, 3), (9, 2), (18, 2)] {
        let mut malformed = bytes.clone();
        malformed[offset] = bad_tag;
        assert!(
            <StrategicAction as crate::value::Value>::decode(&mut Decoder::new(&malformed))
                .is_err()
        );
    }
    assert!(<StrategicAction as crate::value::Value>::decode(&mut Decoder::new(&[2, 6])).is_err());
}

#[test]
fn strategic_action_codec_measures_incremental_payload_bytes() {
    let absent = StrategicAction::Unreached;
    assert_eq!(absent.field().encoded_bytes().unwrap(), 5);
    assert_eq!(
        StrategicAction::Withheld(StrategicActionFailure::Owner)
            .field()
            .encoded_bytes()
            .unwrap(),
        6
    );
    let base = DivisionMovementRow::fixture(9, 4);
    let baseline = DivisionMovementRow {
        strategic_redeployment: 0,
        strategic_action: crate::rows::Recorded::Value(absent),
        ..base.clone()
    };
    let baseline_bytes = round_trip(&baseline);
    assert_eq!(baseline_bytes, 69 + 5 + 5 + 5 + 1 + 9 + 5);
    assert!(baseline.delta(Some(&baseline)).unwrap().is_none());
    for (tick, expected) in [
        (0, 15),
        (1, 15),
        (2, 23),
        (3, 23),
        (4, 31),
        (5, 31),
        (6, 31),
    ] {
        let status = StrategicAction::Available(strategic_action_fixture(tick));
        assert_eq!(status.field().encoded_bytes().unwrap(), expected);
        let row = DivisionMovementRow {
            strategic_action: crate::rows::Recorded::Value(status),
            ..base.clone()
        };
        assert_eq!(round_trip(&row) - baseline_bytes, expected - 5);
    }
}

#[cfg(test)]
fn selected_route_suffix<T: RowCodec + Debug>(table: TableId, index: usize, old: &T, next: &T) {
    round_trip(old);
    round_trip(next);
    for (previous, row) in [(old, next), (next, old)] {
        let delta = row.delta(Some(previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1_u64 << index, "{table:?}");
        let mut bytes = Vec::new();
        encode_delta(table, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let decoded = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded.apply(Some(previous.clone())).unwrap(), *row);
    }
}

#[test]
fn selected_routes_enemy_codec_preserves_occurrence_order_separately_from_legacy_sets() {
    let base = CountryWarRow {
        country: 777,
        can_occupy_non_war: 0xfe,
        enemies_read: 1,
        enemies: vec![11, 29],
        ordered: Some(OrderedEnemyInputs {
            sample_hour: PacketHour::Host,
            entries: vec![29, 0, 11, 29, u32::MAX],
        })
        .into(),
    };
    for entries in [Vec::new(), vec![11, 29], vec![29, 11, 0, 29, u32::MAX]] {
        let next = CountryWarRow {
            ordered: Some(OrderedEnemyInputs {
                sample_hour: PacketHour::Host,
                entries,
            })
            .into(),
            ..base.clone()
        };
        selected_route_suffix(TableId::CountryWars, 3, &base, &next);
        assert_eq!(next.enemies, base.enemies);
    }
    let refused = CountryWarRow {
        ordered: None.into(),
        ..base.clone()
    };
    selected_route_suffix(TableId::CountryWars, 3, &base, &refused);
}

fixture!(AgencyUpgradeLevelRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {definition_pointer: u64, branch_pointer: u64, upgrade_token: Option<u32>, branch_token: Option<u32>, level: i32, identity_status: u8});

impl Fixture for OperationCompletionRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let description_error = (tick % 3) as u8;
        let effect_description = if description_error == 0 {
            Vec::<u8>::sample(tick + 1)
        } else {
            Vec::new()
        };
        Self {
            owner: (key >> 64) as u32,
            sequence: key as u64,
            id: u64::sample(tick),
            target: u32::sample(tick),
            definition_token: u32::sample(tick),
            completion_hour: i32::sample(tick),
            terminal: u8::sample(tick),
            bonus: u8::sample(tick),
            detected: u8::sample(tick),
            description_required: if description_error == 0 {
                effect_description.len() as u64
            } else {
                u64::sample(tick)
            },
            effect_description,
            description_error,
        }
    }
}
sample_struct!(GarrisonEquipment {
    equipment_token: u32,
    archetype_token: u32,
    archetype_active: u8,
    required_raw: i64,
    required_lookup_raw: i64,
    allocated_raw: i64,
    weight_raw: i64
});
sample_struct!(GarrisonEquipmentInputs { rows: Vec<GarrisonEquipment>, failure: Option<Vec<u8>> });

#[test]
fn research_occurrence_suffix_keeps_historical_slot_and_input_prefixes() {
    let input = ResearchInputRow {
        country: 7,
        index: 2,
        technology_handle: 0xffff_fffe,
        progress_raw: 123,
        database_index: Recorded::Value(9),
        ..Default::default()
    };
    let slot = ResearchSlotRow {
        country: 7,
        index: 2,
        technology_handle: 0xffff_fffe,
        research_speed_raw: 456,
        database_index: Recorded::Value(9),
        ..Default::default()
    };
    let prefix = |schema: &[Kind], count: usize, bytes: &[u8]| {
        let mut decoder = Decoder::new(bytes);
        for field in &schema[..count] {
            field.decode(&mut decoder).unwrap();
        }
        bytes.len() - decoder.remaining()
    };
    let mut bytes = Vec::new();
    input.encode_fields(&mut bytes).unwrap();
    let end = prefix(ResearchInputRow::SCHEMA, 15, &bytes);
    let mut decoder = Decoder::new(&bytes[..end]);
    decoder.recorded_fields = 15;
    let historical = ResearchInputRow::decode_fields(input.key(), &mut decoder).unwrap();
    decoder.finish().unwrap();
    assert_eq!(historical.progress_raw, 123);
    assert_eq!(historical.database_index, Recorded::NotRecorded);
    bytes.clear();
    slot.encode_fields(&mut bytes).unwrap();
    let end = prefix(ResearchSlotRow::SCHEMA, 5, &bytes);
    let mut decoder = Decoder::new(&bytes[..end]);
    decoder.recorded_fields = 5;
    let historical = ResearchSlotRow::decode_fields(slot.key(), &mut decoder).unwrap();
    decoder.finish().unwrap();
    assert_eq!(historical.research_speed_raw, 456);
    assert_eq!(historical.database_index, Recorded::NotRecorded);
}

#[test]
fn gap_research_grants_research_grants_project_runtime_and_assignments_round_trip_exact_operands() {
    round_trip(&ProjectRow {
        owner: u32::MAX,
        local_id: 7,
        runtime: Recorded::Value(ProjectRuntime {
            pending_known: true,
            pending_reward: Some(u32::MAX),
            pending_ordinal: Some(0),
            variables: Some(
                ProjectVariableStore {
                    seed: u32::MAX,
                    entries: vec![
                        ProjectVariable {
                            name: b"mod_array^0".to_vec(),
                            value_raw: i64::MIN,
                        },
                        ProjectVariable {
                            name: b"mod_array^0".to_vec(),
                            value_raw: i64::MAX,
                        },
                    ],
                }
                .into(),
            ),
        }),
        ..Default::default()
    });
    round_trip(&ProjectProgramRow {
        id: 3,
        assignments: Recorded::Value(Some(vec![
            ProjectScientistAssignment {
                scientist: u64::MAX,
                target: i32::MIN,
                current: i32::MAX,
                timed: 253,
            },
            ProjectScientistAssignment::default(),
        ])),
        ..Default::default()
    });
    round_trip(&ScientistDailyRow {
        character: 7,
        injury_days: Recorded::Value(Some(i32::MIN)),
        template_visibility: Recorded::Value(Some(1)),
        template_origin: Recorded::Value(Some(ScientistTemplateOrigin {
            file: b"events/mod.txt".to_vec(),
            line: u32::MAX,
        })),
        ..Default::default()
    });
    for kind in ResearchModifierRow::GRANT_COUNTER..=ResearchModifierRow::GRANT_CATEGORY {
        round_trip(&ResearchModifierRow {
            country: u32::MAX,
            index: u32::MAX - 1,
            kind,
            raw: i64::MIN,
            auxiliary: i64::MAX,
            source_id: u32::MAX,
            name: b"mod_grant".to_vec(),
            ..Default::default()
        });
    }
}

#[test]
fn manpower_operand_tables_preserve_independent_fields_and_ordered_references() {
    check_table::<CountryCounterInputRow>(TableId::CountryCounterInputs);
    check_table::<CountryStabilityInputRow>(TableId::CountryStabilityInputs);
    check_table::<StateResourceInputRow>(TableId::StateResourceInputs);
    check_table::<PoliticsRow>(TableId::Politics);
    fn prepared<T: Fixture + crate::checkpoint::rows::Row>() {
        for tick in 0..4 {
            let row = T::fixture(T::smallest_key(), tick);
            let mut bytes = Vec::new();
            crate::checkpoint::rows::encode_rows(std::slice::from_ref(&row), &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                crate::checkpoint::rows::decode_rows::<T>(&mut input).unwrap(),
                [row]
            );
            input.finish().unwrap();
        }
    }
    prepared::<CountryCounterInputRow>();
    prepared::<CountryStabilityInputRow>();
    prepared::<StateResourceInputRow>();
    prepared::<PoliticsRow>();
}

#[test]
fn occupation_root_total_absence_round_trips_distinctly_from_zero() {
    let mut zero = StateResistanceRow::fixture(7, 0);
    zero.modifier_sources = Recorded::Value(vec![OccupationModifierSource {
        section: 2,
        source: 0,
        key: engine_occupation::cresistance::hover::DECAY,
        total: Some(0),
        ..Default::default()
    }]);
    round_trip(&zero);
    let mut unknown = zero.clone();
    let Recorded::Value(sources) = &mut unknown.modifier_sources else {
        unreachable!()
    };
    sources[0].total = None;
    sources[0].failure = b"fixture root value unreadable".to_vec();
    round_trip(&unknown);
    let delta = unknown.delta(Some(&zero)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 7);
    assert_eq!(
        delta.apply::<StateResistanceRow>(Some(zero)).unwrap(),
        unknown
    );
}

impl Fixture for VariantChangeRow {
    fn smallest_key() -> u128 {
        1
    }
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            sequence: key as u64,
            hour: tick as i32,
            variant: key as u64,
            flags: None,
            scalar: None,
            sources: None,
            organisation: None,
            removed: tick % 2 == 1,
            snapshot: Some(VariantSnapshot {
                definition: VariantDefinitionRow::fixture(key, tick).into(),
                stats: (0..10)
                    .map(|index| ((tick + index) % 3 != 0).then_some(tick as i64))
                    .collect(),
                ..Default::default()
            }),
            failure: Vec::new(),
        }
    }
}

impl Fixture for VariantBonusSnapshotRow {
    fn smallest_key() -> u128 {
        1
    }
    fn fixture(key: u128, tick: usize) -> Self {
        let source = VariantBonusSource {
            terrain_attrition: vec![tick as i64],
            ..Default::default()
        };
        Self {
            id: key as u64,
            source,
        }
    }
}

impl Fixture for DesignCountryModifierRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 32) as u32,
            modifier: key as u32,
            value: Some(i64::sample(tick)),
            failure: 0,
        }
    }
}

impl Fixture for DesignOrganisationLivenessRow {
    fn smallest_key() -> u128 {
        1
    }
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            organisation: key as u64,
            live: tick % 2 == 1,
        }
    }
}

fixture!(DesignObserverContextRow, |_key| {}, {fog: u8});
impl Fixture for DesignCountryTemplatesRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: key as u32,
            uses: vec![i32::sample(tick)],
            factors: vec![i64::sample(tick)],
            equipment: vec![vec![u32::sample(tick), u32::sample(tick)]],
            failure: 0,
        }
    }
}

fixture!(DivisionHistoryRow, |key| { division: (key >> 34) as u64, list: ((key >> 32) & 3) as u8, ordinal: key as u32 }, {
    available: u8, reason: Vec<u8>, amount: i32, name: Vec<u8>, custom_name: Vec<u8>, kind: i32,
    target_country: u32, hour: i32, medal_token: Option<i32>, medal_definition: Option<i32>, medal_count: u8, medal_enabled: u8,
    inherit: u8, multiplier: i64, orders: i32, provinces: Vec<u32>, sunk_ship: Option<SunkShipEntry>
});

impl Fixture for SharedDivisionHistoryRow {
    fn fixture(key: u128, tick: usize) -> Self {
        std::sync::Arc::new(DivisionHistoryRow::fixture(key, tick))
    }
}

fixture!(DivisionPanelSourceRow, |key| { kind:(key>>96) as u8,owner:(key>>32) as u64,ordinal:key as u32 }, {node:Option<DivisionPanelSourceNode>, reason:Vec<u8>});
sample_struct!(DivisionPanelSourceValue { key: u32, raw: i64 });
sample_struct!(DivisionPanelSourceNode {children:u32,name:Vec<u8>,values:Vec<DivisionPanelSourceValue>});
impl Fixture for SharedDivisionPanelSourceRow {
    fn fixture(key: u128, tick: usize) -> Self {
        std::sync::Arc::new(DivisionPanelSourceRow::fixture(key, tick))
    }
}

fixture!(CombatLogGroupRow, |key| {country: (key >> 64) as u32, group: key as u64}, {ordinal: u32});
fixture!(CombatLogMonthRow, |key| {country: (key >> 72) as u32, group: (key >> 40) as u32, kind: (key >> 32) as u8, ordinal: key as u32}, {reason: i32, date: i32, allow_zero: bool});
fixture!(CombatLogManpowerRow, |key| {country: (key >> 64) as u32, group: (key >> 32) as u32, ordinal: key as u32}, {reason: i32, date: i32, losses_0: i32, losses_1: i32, losses_2: i32});
fixture!(CombatLogTemplateRow, |key| {country: (key >> 64) as u32, group: (key >> 32) as u32, ordinal: key as u32}, {template: u64, date: i32, wins: i32, total: i32});
fixture!(CombatLogReferenceRow, |key| {country: (key >> 64) as u32, group: (key >> 32) as u32, ordinal: key as u32}, {battle: i32, attacker: bool});
fixture!(CombatLogEnemyEquipmentRow, |key| {country: (key >> 96) as u32, group: (key >> 64) as u32, month: (key >> 32) as u32, ordinal: key as u32}, {variant: u64, amount_raw: i64});

fixture!(CombatLogBattleRow, |key| {id: key as u32}, {reference_count: i32, date: i32, province: u32, flags: [u8; 4]});

fixture!(CombatLogBattleSideRow, |key| {battle: (key >> 8) as u32, side: key as u8}, {manpower: i32, air_factor_raw: i64, leader: u64, pool_flags: u8});
fixture!(CombatLogBattleEquipmentRow, |key| {battle: (key >> 48) as u32, side: (key >> 40) as u8, kind: (key >> 32) as u8, ordinal: key as u32}, {variant: u64, amount_raw: i64});
fixture!(CombatLogBattleModifierRow, |key| {battle: (key >> 40) as u32, side: (key >> 32) as u8, ordinal: key as u32}, {modifier: i32, hours: i32});
fixture!(CombatLogBattleCountryRow, |key| {battle: (key >> 40) as u32, side: (key >> 32) as u8, ordinal: key as u32}, {country: u32});

fixture!(CombatLogObserverRow, |key| {combat:(key >> 64) as u64, attacker:key & 1 != 0, ordinal:(key as u64) >> 1}, {kind:u8, item:u64, index:u64, identity:u64, identity_2:u64, identity_3:u64, a:i64, b:i64, c:i64, d:i64, e:i64});

sample_struct!(HqEquipment {
    variant: u64,
    amount: i64
});
sample_struct!(HqRequirement {
    token: u32,
    amount: i64
});
sample_struct!(HqDeployment {
    country: u32,
    priority: i32,
    allow_zero: u8,
    assembled_manpower: i32,
    requested_manpower: i32,
    order: i32,
    requisitioned: u8
});
sample_struct!(TemplateHqContext {target:u64,deployed:u8,queued:u8,deployment:Option<HqDeployment>,equipment:Vec<HqEquipment>,required:Vec<HqRequirement>});

#[test]
fn template_garrison_priority_schema_matches_its_optional_integer_wire_values() {
    assert!(matches!(
        CountryTemplateStateRow::SCHEMA[4],
        Kind::OptionalI32
    ));
    for priority in [
        None,
        Some(i32::MIN),
        Some(-3),
        Some(0),
        Some(1),
        Some(i32::MAX),
    ] {
        let row = CountryTemplateStateRow {
            country: 11,
            selected_staff: 52,
            garrison_priority: Recorded::Value(priority),
            garrisons: Recorded::Value(Some(
                vec![GarrisonTemplateSelection {
                    state: Some(0),
                    template: Some(52),
                    ..Default::default()
                }]
                .into(),
            )),
            ..Default::default()
        };
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        for (index, kind) in CountryTemplateStateRow::SCHEMA.iter().enumerate() {
            let value = kind.decode(&mut input).unwrap();
            if index == 4 {
                let Field::OptionalI32(value) = value else {
                    panic!("priority uses optional integer framing");
                };
                assert_eq!(value, priority);
            }
        }
        input.finish().unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            CountryTemplateStateRow::decode_fields(row.key(), &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
    }
}

#[test]
fn participation_rows_round_trip_exact_components_membership_and_column_deltas() {
    check_table::<WarParticipationRow>(TableId::WarParticipation);
    check_table::<WarCapturedProvinceRow>(TableId::WarCapturedProvinces);
    check_table::<WarLendLeaseHistoryRow>(TableId::WarLendLeaseHistories);
    assert!(encode_row(&WarLendLeaseHistoryRow::default(), 0, &mut Vec::new()).is_err());
    let baseline = WarParticipationRow {
        war_first: 5000,
        war_second: 1500,
        direction: 1,
        equipment_damage: i64::MIN,
        ..Default::default()
    };
    let changed = WarParticipationRow {
        equipment_damage: i64::MAX,
        ..baseline
    };
    let delta = changed.delta(Some(&baseline)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    assert_eq!(delta.apply(Some(baseline)).unwrap(), changed);
    let invalid = WarParticipationRow {
        direction: 2,
        ..Default::default()
    };
    assert!(encode_row(&invalid, 0, &mut Vec::new()).is_err());
}

#[test]
fn air_map_interception_trace_gate_changes_without_rewriting_other_mission_operands() {
    use crate::rows::{AirMissionInputRow, RowCodec};
    let mut before = AirMissionInputRow {
        wing: 7,
        interception_alert_gate: (Some(0)).into(),
        ..Default::default()
    };
    let mut after = before.clone();
    after.interception_alert_gate = Some(1).into();
    assert_eq!(after.delta(Some(&before)).unwrap().unwrap().mask, 1 << 7);
    check_table::<AirMissionInputRow>(TableId::AirMissionInputs);
    before.interception_alert_gate = None.into();
    assert_eq!(after.delta(Some(&before)).unwrap().unwrap().mask, 1 << 7);
}

#[test]
fn current_air_execution_and_count_have_independent_deltas() {
    use crate::rows::{AirMissionInputRow, RowCodec};
    let before = AirMissionInputRow {
        wing: 9,
        current_type: (Some(0)).into(),
        ..Default::default()
    };
    let mut after = before.clone();
    after.current_type = Some(0x8000_0000).into();
    assert_eq!(after.delta(Some(&before)).unwrap().unwrap().mask, 1 << 8);
    let before = after.clone();
    after.effective_aircraft = Some(i32::MIN).into();
    assert_eq!(after.delta(Some(&before)).unwrap().unwrap().mask, 1 << 9);
    check_table::<AirMissionInputRow>(TableId::AirMissionInputs);
}

#[test]
fn country_air_modifiers_preserve_absence_signed_words_and_an_independent_delta() {
    use crate::rows::{CountryCounterInputRow, RowCodec};
    let before = CountryCounterInputRow {
        country: 4,
        air_modifiers: Recorded::Value(Some([None; 10])),
        ..Default::default()
    };
    let mut after = before.clone();
    let mut values = [None; 10];
    values[0] = Some(i64::MIN);
    values[9] = Some(i64::MAX);
    after.air_modifiers = Recorded::Value(Some(values));
    assert_eq!(after.delta(Some(&before)).unwrap().unwrap().mask, 1 << 23);
    let mut refused = after.clone();
    refused.air_modifiers = Recorded::Value(None);
    assert_eq!(refused.delta(Some(&after)).unwrap().unwrap().mask, 1 << 23);
    check_table::<CountryCounterInputRow>(TableId::CountryCounterInputs);
}

#[test]
fn country_air_combat_columns_delta_separately_and_reuse_existing_home_terms() {
    use engine_reference_values::modifier_keys as keys;
    let mut row = CountryCounterInputRow {
        country: 7,
        ..Default::default()
    };
    assert!(row.air_combat_modifier(keys::AIR_ATTACK_FACTOR).is_err());
    row.air_combat_modifiers_known = true.into();
    assert_eq!(row.air_combat_modifier(keys::AIR_ATTACK_FACTOR), Ok(None));
    let old = row.clone();
    row.air_attack_factor = Some(i64::MIN).into();
    assert_eq!(
        row.air_combat_modifier(keys::AIR_ATTACK_FACTOR),
        Ok(Some(i64::MIN))
    );
    assert_eq!(row.delta(Some(&old)).unwrap().unwrap().mask, 1_u64 << 30);
    let mut home = [None; 10];
    home[2] = Some(-123);
    home[3] = Some(i64::MAX);
    row.air_modifiers = Recorded::Value(Some(home));
    assert_eq!(
        row.air_combat_modifier(keys::AIR_HOME_DEFENCE_FACTOR),
        Ok(Some(-123))
    );
    assert_eq!(
        row.air_combat_modifier(keys::AIR_POWER_PROJECTION_FACTOR),
        Ok(Some(i64::MAX))
    );
    let old = row.clone();
    row.air_combat_modifiers_known = false.into();
    assert_eq!(row.delta(Some(&old)).unwrap().unwrap().mask, 1_u64 << 33);
    assert!(row.air_combat_modifier(keys::AIR_ATTACK_FACTOR).is_err());
    assert_eq!(
        row.air_combat_modifier(keys::AIR_HOME_DEFENCE_FACTOR),
        Ok(Some(-123))
    );
    check_table::<CountryCounterInputRow>(TableId::CountryCounterInputs);
}

#[test]
fn ace_combat_columns_preserve_signed_raw_words_and_independent_type_predicates() {
    let mut row = AceRangeInputRow {
        canonical_slot: 7,
        ace: 19,
        alive: 1,
        combat_modifiers_known: true.into(),
        type_combat_flags: 1.into(),
        ..Default::default()
    };
    assert_eq!(row.type_speed_modifier(), Ok(None));
    assert_eq!(row.type_attack_positive(), Ok(false));
    assert_eq!(row.type_defence_positive(), Ok(false));
    let old = row.clone();
    row.attack_modifier_raw = Some(i64::MIN).into();
    assert_eq!(row.delta(Some(&old)).unwrap().unwrap().mask, 1 << 8);
    let old = row.clone();
    row.type_speed_modifier_raw = Some(i64::MAX).into();
    assert_eq!(row.delta(Some(&old)).unwrap().unwrap().mask, 1 << 10);
    let old = row.clone();
    row.type_combat_flags = 5.into();
    assert_eq!(row.delta(Some(&old)).unwrap().unwrap().mask, 1 << 11);
    assert_eq!(row.type_speed_modifier(), Ok(Some(i64::MAX)));
    assert_eq!(row.type_attack_positive(), Ok(false));
    assert_eq!(row.type_defence_positive(), Ok(true));
    row.type_combat_flags = 0.into();
    assert!(row.type_speed_modifier().is_err());
    assert!(row.type_attack_positive().is_err());
    assert!(row.type_defence_positive().is_err());
    check_table::<AceRangeInputRow>(TableId::AceRangeInputs);
}

#[test]
fn map_operand_suffixes_distinguish_unrecorded_from_current_named_refusals() {
    for (current, prefix) in [
        (
            TableRows::WeatherRegions(vec![WeatherRegionRow {
                id: 7,
                ..Default::default()
            }]),
            9,
        ),
        (
            TableRows::AceRangeInputs(vec![AceRangeInputRow {
                canonical_slot: 7,
                ace: 9,
                ..Default::default()
            }]),
            4,
        ),
        (
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 7,
                ..Default::default()
            }]),
            21,
        ),
        (
            TableRows::AirMissionInputs(vec![AirMissionInputRow {
                wing: 7,
                ..Default::default()
            }]),
            6,
        ),
        (
            TableRows::VolunteerHosts(vec![VolunteerHostRow {
                country: 7,
                ..Default::default()
            }]),
            2,
        ),
        (
            TableRows::RaidCountryInputs(vec![RaidCountryInputRow {
                canonical_slot: 7,
                ..Default::default()
            }]),
            4,
        ),
        (
            TableRows::FleetCounterInputs(vec![FleetCounterInputRow {
                fleet: 7,
                ..Default::default()
            }]),
            6,
        ),
        (
            TableRows::NavalMissionCountries(vec![NavalMissionCountryRow {
                country: 7,
                ..Default::default()
            }]),
            8,
        ),
        (
            TableRows::NavalMissionCommanders(vec![NavalMissionCommanderRow {
                country: 7,
                ..Default::default()
            }]),
            4,
        ),
    ] {
        let table = current.table();
        assert_eq!(current.iter().next().unwrap().not_recorded_fields(), 0);
        current.encode_rows(&mut Vec::new()).unwrap();
        let bytes = current.fixture_prefix(prefix);
        let mut input = Decoder::new(&bytes);
        input.recorded_fields = prefix;
        let older = table.decode_rows(1, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(
            older.iter().next().unwrap().not_recorded_fields(),
            full_mask(table.field_count()) & !full_mask(prefix)
        );
        assert_eq!(older.field_gaps().count(), table.field_count() - prefix);
        assert!(older.encode_rows(&mut Vec::new()).is_err());
    }
}

#[test]
fn detection_override_scopes_round_trip_above_the_original_two_bit_domain() {
    use crate::rows::{AirSortieModifierRow, RowCodec};
    let before = AirSortieModifierRow {
        scope: AirSortieModifierRow::DETECTION_OVERRIDE,
        subject: u32::MAX,
        region: 0,
        raw: i64::MIN,
    };
    let mut bytes = Vec::new();
    before.encode_fields(&mut bytes).unwrap();
    let mut decoder = Decoder::new(&bytes);
    let decoded = AirSortieModifierRow::decode_fields(before.key(), &mut decoder).unwrap();
    assert_eq!(decoded, before);
    decoder.finish().unwrap();
    let witness = AirSortieModifierRow {
        scope: AirSortieModifierRow::DETECTION_OVERRIDE_STATUS,
        subject: 0,
        region: 0,
        raw: 1,
    };
    assert!(witness.key() > before.key());
    check_table::<AirSortieModifierRow>(TableId::AirSortieModifiers);
}

#[test]
fn country_mine_reduction_keeps_refusal_absent_key_and_signed_value_distinct() {
    use crate::rows::{CountryCounterInputRow, RowCodec};
    let before = CountryCounterInputRow {
        country: 19,
        mines_effect_reduction: Recorded::Value(None),
        ..Default::default()
    };
    let absent = CountryCounterInputRow {
        mines_effect_reduction: Recorded::Value(Some(None)),
        ..before.clone()
    };
    assert_eq!(absent.delta(Some(&before)).unwrap().unwrap().mask, 1 << 24);
    let present = CountryCounterInputRow {
        mines_effect_reduction: Recorded::Value(Some(Some(i64::MIN))),
        ..before
    };
    assert_eq!(present.delta(Some(&absent)).unwrap().unwrap().mask, 1 << 24);
    check_table::<CountryCounterInputRow>(TableId::CountryCounterInputs);
}

#[test]
fn ship_naval_power_is_an_independent_raw_optional_delta() {
    let old = ShipModelRow {
        ship: 1,
        naval_power: Recorded::Value(None),
        ..Default::default()
    };
    let next = ShipModelRow {
        naval_power: Recorded::Value(Some([i64::MIN, i64::MAX, -1, 0, 123_456_789])),
        ..old.clone()
    };
    let delta = next.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 11);
    assert_eq!(delta.apply(Some(old)).unwrap(), next);
}

#[test]
fn experience_suffixes_keep_retired_bearing_prefixes_readable() {
    fn check<T: RowCodec + Fields + std::fmt::Debug + PartialEq>(table: TableId, row: T) {
        use crate::{codec::Decoder, format::FormatVersion};
        for version in [91, 97, 99, 102, 103] {
            let format = FormatVersion::read(version).unwrap();
            assert!(format.table_refusal(table).is_none());
            let fields = format.fields(table);
            let mut encoded = Vec::new();
            row.encode_fields(&mut encoded).unwrap();
            let mut scan = Decoder::new(&encoded);
            for kind in &T::SCHEMA[..fields] {
                kind.decode(&mut scan).unwrap();
            }
            let prefix = &encoded[..encoded.len() - scan.remaining()];
            let mut input = Decoder::new(prefix);
            input.format = version;
            input.recorded_fields = fields;
            let decoded = T::decode_fields(row.key(), &mut input).unwrap();
            input.finish().unwrap();
            let missing = table
                .last_change()
                .appended
                .iter()
                .filter(|field| field.introduced > version)
                .fold(0, |mask, field| mask | (1 << field.index));
            assert_eq!(
                decoded.not_recorded_fields(),
                missing,
                "{table:?} at {version}"
            );
            let mut delta = row.delta(None).unwrap().unwrap();
            let mut scan = Decoder::new(&delta.values);
            for kind in &T::SCHEMA[..fields] {
                kind.decode_change(&mut scan).unwrap();
            }
            delta.values.truncate(delta.values.len() - scan.remaining());
            delta.format = version;
            delta.recorded_fields = fields;
            delta.mask = (1 << fields) - 1;
            assert_eq!(
                delta.apply::<T>(None).unwrap(),
                decoded,
                "{table:?} at {version}"
            );
            assert!(decoded.encode_fields(&mut Vec::new()).is_err());
        }
    }
    for version in [91, 97, 99, 102, 103] {
        let format = crate::format::FormatVersion::read(version).unwrap();
        assert_eq!(
            format.table_refusal(TableId::DivisionStatus).unwrap(),
            format!(
                "DivisionStatus layout or meaning changed at format {}; this recording is format {version}",
                crate::rows::compatibility::STORED_ONCE_FORMAT
            )
        );
    }
    check(
        TableId::CountryCounterInputs,
        CountryCounterInputRow {
            country: 7,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers {
                army_speed_factor: Some(-13),
                ..Default::default()
            })),
            combat_width_factor: Some(29).into(),
            ..Default::default()
        },
    );
}

#[test]
fn appended_operands_read_as_not_recorded_in_older_recordings() {
    fn older<T: RowCodec + Fields + Debug + PartialEq>(table: TableId, row: T, version: u16) -> T {
        use crate::{codec::Decoder, format::FormatVersion};
        let fields = FormatVersion::read(version).unwrap().fields(table);
        assert!(fields < T::SCHEMA.len(), "{table:?} at {version}");
        let mut encoded = Vec::new();
        row.encode_fields(&mut encoded).unwrap();
        let mut scan = Decoder::new(&encoded);
        for kind in &T::SCHEMA[..fields] {
            kind.decode(&mut scan).unwrap();
        }
        let mut input = Decoder::new(&encoded[..encoded.len() - scan.remaining()]);
        input.format = version;
        input.recorded_fields = fields;
        let decoded = T::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        let mut delta = row.delta(None).unwrap().unwrap();
        let mut scan = Decoder::new(&delta.values);
        for kind in &T::SCHEMA[..fields] {
            kind.decode_change(&mut scan).unwrap();
        }
        delta.values.truncate(delta.values.len() - scan.remaining());
        delta.format = version;
        delta.recorded_fields = fields;
        delta.mask = (1 << fields) - 1;
        assert_eq!(
            delta.apply::<T>(None).unwrap(),
            decoded,
            "{table:?} at {version}"
        );
        decoded
    }
    let policies = older(
        TableId::IndustrialOrganisationPolicies,
        IndustrialOrganisationPoliciesRow {
            country: 4,
            policies: Vec::new(),
            cost_modifier_raw: Some(-20_000).into(),
        },
        crate::rows::compatibility::MIO_POLICY_COST_FORMAT - 1,
    );
    assert_eq!(policies.cost_modifier_raw, Recorded::NotRecorded);
    let before = crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT - 1;
    let origin = older(
        TableId::ResourceOrigins,
        ResourceOriginRow {
            state: 7,
            country: 3,
            convoys_required: 4,
            convoys_base: 5.into(),
            ..Default::default()
        },
        before,
    );
    assert_eq!(origin.convoys_required, 4);
    assert_eq!(origin.convoys_base, Recorded::NotRecorded);
    let client = older(
        TableId::ConvoyClients,
        ConvoyClientRow {
            handle: 9,
            convoys_required: 2,
            efficiency_a: 80_000.into(),
            efficiency_b: 95_000.into(),
            convoys_base: 3.into(),
            ..Default::default()
        },
        before,
    );
    assert_eq!(client.convoys_required, 2);
    assert_eq!(
        [client.efficiency_a, client.efficiency_b],
        [Recorded::NotRecorded, Recorded::NotRecorded]
    );
    assert_eq!(client.convoys_base, Recorded::NotRecorded);
    let history = older(
        TableId::IndustrialOrganisationHistory,
        IndustrialOrganisationHistoryRow {
            organisation: 3,
            variant: 9,
            units: 11,
            design_hour: Some(-123).into(),
        },
        crate::rows::compatibility::INDUSTRIAL_ORGANISATION_DETAILS_FORMAT - 1,
    );
    assert_eq!(history.units, 11);
    assert_eq!(history.design_hour, Recorded::NotRecorded);
    let theatre = older(
        TableId::FactionTheatres,
        FactionTheatreRow {
            faction: (2 << 32) | 88,
            ordinal: 1,
            icon: b"GFX_allies".to_vec(),
            regions: vec![29, u32::MAX],
            country_count: 2,
            hidden_for: vec![7],
            name: b"Western Front".to_vec().into(),
            owner: Recorded::Value(11),
            commander: Some((844 << 32) | 4713).into(),
            countries: vec![11, 12].into(),
        },
        crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT - 1,
    );
    assert_eq!(
        (theatre.icon.as_slice(), theatre.country_count, theatre.hidden_for.as_slice()),
        (&b"GFX_allies"[..], 2, &[7][..])
    );
    assert_eq!(theatre.name, Recorded::NotRecorded);
    assert_eq!(theatre.owner, Recorded::NotRecorded);
    assert_eq!(theatre.commander, Recorded::NotRecorded);
    assert_eq!(theatre.countries, Recorded::NotRecorded);
    let before = crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT - 1;
    let division = older(
        TableId::Divisions,
        DivisionRow {
            id: 0x0000_0005_0000_0029,
            province: 12,
            theatre: Some(0x0000_0011_0000_0043).into(),
            ..Default::default()
        },
        before,
    );
    assert_eq!(division.province, 12);
    assert_eq!(division.theatre, Recorded::NotRecorded);
    let movement = older(
        TableId::DivisionMovement,
        DivisionMovementRow {
            queued_actions: Some(2).into(),
            ..DivisionMovementRow::fixture(41, 1)
        },
        before,
    );
    assert_eq!(movement.queued_actions, Recorded::NotRecorded);
    let gun = older(
        TableId::RailwayGunCounterInputs,
        RailwayGunCounterInputRow {
            theatre: Some(0).into(),
            queued_actions: Some(1).into(),
            ..RailwayGunCounterInputRow::fixture(43, 2)
        },
        before,
    );
    assert_eq!(gun.theatre, Recorded::NotRecorded);
    assert_eq!(gun.queued_actions, Recorded::NotRecorded);
    let state = older(
        TableId::States,
        StateRow {
            id: 9,
            owner: 7,
            marked_owners: Some(vec![3, 5]).into(),
            ..Default::default()
        },
        before,
    );
    assert_eq!(state.owner, 7);
    assert_eq!(state.marked_owners, Recorded::NotRecorded);
    let weather = older(
        TableId::WeatherProvinces,
        WeatherProvinceRow {
            static_modifiers: Recorded::Value(Some(ProvinceStaticModifiers {
                entries: vec![ProvinceStaticModifier {
                    name: b"the_great_wall".to_vec(),
                    days: -1,
                }],
            })),
            ..WeatherProvinceRow::fixture(5, 3)
        },
        crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT - 1,
    );
    assert_eq!(weather.static_modifiers, Recorded::NotRecorded);
    assert_eq!(
        weather.modifiers,
        WeatherProvinceRow::fixture(5, 3).modifiers
    );
    let region = older(
        TableId::WeatherRegions,
        WeatherRegionRow {
            id: 41,
            snow: 1,
            custom_sea: Some(vec![1, 0, -1]).into(),
            ..Default::default()
        },
        crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT - 1,
    );
    assert_eq!((region.id, region.snow), (41, 1));
    assert_eq!(region.custom_sea, Recorded::NotRecorded);
    let rail = older(
        TableId::RailCooldowns,
        RailCooldownRow {
            province: 812,
            cooldown_ticks: 24,
            building_bound: Some(true).into(),
            ..Default::default()
        },
        crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT - 1,
    );
    assert_eq!(rail.cooldown_ticks, 24);
    assert_eq!(rail.building_bound, Recorded::NotRecorded);
}

#[test]
fn combat_log_observer_numeric_columns_round_trip_rows_deltas_and_prepared_checkpoints() {
    check_table::<CombatLogObserverRow>(TableId::CombatLogObservers);
    for tick in [0, 5, 12] {
        let expected = vec![CombatLogObserverRow::fixture(
            CombatLogObserverRow::largest_key(),
            tick,
        )];
        let mut bytes = Vec::new();
        crate::checkpoint::rows::encode_rows(&expected, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            crate::checkpoint::rows::decode_rows::<CombatLogObserverRow>(&mut input).unwrap(),
            expected
        );
        input.finish().unwrap();
        for end in 0..bytes.len() {
            assert!(
                crate::checkpoint::rows::decode_rows::<CombatLogObserverRow>(&mut Decoder::new(
                    &bytes[..end]
                ))
                .is_err()
            );
        }
    }
}

#[test]
fn raid_dispatcher_keys_round_trip_independently_at_province_boundaries() {
    let mut rows = Vec::new();
    for dispatcher in [RaidPathDispatcher::Land, RaidPathDispatcher::Naval] {
        for (source, target) in [(0, 0), (7, 9), (u32::MAX, u32::MAX)] {
            rows.push(RaidPathDistanceRow {
                dispatcher,
                source,
                target,
                distance: if dispatcher == RaidPathDispatcher::Land {
                    900_000
                } else {
                    500_000
                },
                ok: 1,
            });
        }
    }
    let table = TableRows::RaidPathDistances(rows.clone());
    let mut bytes = Vec::new();
    table.encode_rows(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        TableId::RaidPathDistances
            .decode_rows(rows.len(), &mut input)
            .unwrap(),
        table
    );
    input.finish().unwrap();
    let mut changed = rows[4].clone();
    changed.ok = 0;
    changed.distance = i64::MIN;
    let delta = changed.delta(Some(&rows[4])).unwrap().unwrap();
    assert_eq!(delta.apply(Some(rows[4].clone())).unwrap(), changed);
    assert_eq!(rows[1].dispatcher, RaidPathDispatcher::Land);
    changed.ok = 2;
    assert!(changed.check_key_fields().is_err());
}

impl Sample for AirHistoryWindow {
    fn sample(tick: usize) -> Self {
        Self {
            complete: tick % 2 == 0,
            capacity: 4,
            current: (tick % 4) as u32,
            wrapped: tick > 3,
        }
    }
}

impl Sample for std::sync::Arc<Vec<ModifierBreakdownNode>> {
    fn sample(tick: usize) -> Self {
        let leaves = tick % 3;
        let root = ModifierBreakdownNode {
            children: leaves as u32,
            value: tick as i64,
            name: Vec::new(),
        };
        let leaves = (0..leaves).map(|index| ModifierBreakdownNode {
            children: 0,
            value: i64::MIN + (tick + index) as i64,
            name: vec![b'k', index as u8, tick as u8],
        });
        Self::new(std::iter::once(root).chain(leaves).collect())
    }
}
