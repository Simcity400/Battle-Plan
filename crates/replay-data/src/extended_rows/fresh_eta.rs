//! Current-hour railway route admission inputs recorded beside the counters.
//!
//! The fresh counter-route speed graph, the owner movement packet (its river,
//! naval, theatre and transport-speed domains) and the reserved nearest-base
//! caches retired at [`crate::rows::compatibility::RETIRED_INPUTS_FORMAT`];
//! playback estimates arrival times from routes and each division's base
//! speed instead. [`OwnerTransportSpeedRow`] keeps the transport speeds a
//! railway gun's sea-crossing and transfer legs move at, and a division's
//! transport and sea legs (DivisionTransportSpeeds). The land movement speed's
//! location state terms ([`MovementStateTermRow`]), noncore-supply gate
//! ([`CoreSupplyAccessRow`]), priority sub-unit and category entries
//! ([`CountryUnitSpeedEntryRow`]) and relation river crossing factor
//! ([`RiverRelationRow`]) are recorded beside them.
use super::*;

/// The transport speed a railway-gun owner's sea-crossing and naval-transfer
/// legs move at in one strategic region, before the content's naval speed
/// scale: raw fixed5, read hourly for every logical owner of a recorded
/// railway gun and every strategic region holding a province that owner's
/// guns stand on or route through (their recorded ordinary and alternate
/// paths). `owner` is the gun row's `logical_owner` reference. `None` is that
/// query's counted refusal; the other regions keep theirs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct OwnerTransportSpeedRow {
    pub owner: u32,
    pub region: u32,
    pub speed: Option<i64>,
}
typed_codec!(OwnerTransportSpeedRow, 64, |row| (row.owner as u128) << 32 | row.region as u128,
|key| Self { owner: (key >> 32) as u32, region: key as u32, ..Default::default() },
{ 0 => (speed): OptionalI64 }, |_row| Ok(()));

/// DivisionTransportSpeeds reuses [`OwnerTransportSpeedRow`]: the transport speed a
/// division's naval-transfer legs and its legs to or from a sea province move at,
/// before the content's transport scale, for each logical owner (raw, the
/// DivisionRow reference) of a division with a naval transfer or a sea province
/// on its location or route, in the strategic region of every such leg's source.
/// `None` is that query's counted refusal.
///
/// MovementStateTerms: the location state's modifier entries a division's land
/// movement speed adds, read once per (state, roster owner, owner). `state` is
/// the location province's state (a province without one has no term and no row),
/// `roster` the division's roster owner, whose modifier block of the state is
/// read, and `owner` the expeditionary owner if positive, else the roster owner,
/// as DivisionRow holds them (raw). `terms: None` is a counted read refusal.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct MovementStateTermRow {
    pub state: u32,
    pub roster: u32,
    pub owner: u32,
    pub terms: Option<StateMovementTerms>,
}
typed_codec!(MovementStateTermRow, 96, |row| (row.state as u128) << 64 | (row.roster as u128) << 32 | row.owner as u128,
|key| Self { state: (key >> 64) as u32, roster: (key >> 32) as u32, owner: key as u32, ..Default::default() },
{ 0 => (terms): Opaque }, |_row| Ok(()));

/// The raw entries of the state modifier block selected for the roster owner
/// ([`MovementStateTermRow`]), made effective on playback with the recorded
/// definitions; `None` is an absent entry. `army_speed` is always added,
/// `attack_speed` only for a hostile move, and by `branch` (the owner's movement
/// relation to the state's controller: [`Self::NEITHER`], [`Self::FRIENDLY`] or
/// [`Self::ENEMY`]) `relation_speed`, the selected controller or enemy entry. No unused
/// controller entry is recorded. Attack speed is queried only when a mover sharing
/// this key has a hostile final destination. Its refusal is independent of common
/// terms, in [`StateMovementTerms::ATTACK_REFUSED`]; peaceful consumers ignore it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct StateMovementTerms {
    pub army_speed: Option<i64>,
    pub attack_speed: Option<i64>,
    pub relation_speed: Option<i64>,
    pub branch: u8,
}
value_struct!(StateMovementTerms {
    army_speed: Option<i64>,
    attack_speed: Option<i64>,
    relation_speed: Option<i64>,
    branch: u8,
});
opaque_value!(Option<StateMovementTerms>);
impl StateMovementTerms {
    /// The controller is neither movement-friendly nor movement-enemy: no
    /// controller term.
    pub const NEITHER: u8 = 0;
    /// The controller is movement-friendly: the controller speed applies.
    pub const FRIENDLY: u8 = 1;
    /// The controller is a movement enemy: the enemy speed applies.
    pub const ENEMY: u8 = 2;
    /// Controller relation bits, independent of conditional entry availability.
    pub const RELATION_MASK: u8 = 3;
    /// Only the hostile-destination attack entry refused; common terms remain available.
    pub const ATTACK_REFUSED: u8 = 4;
}

/// CoreSupplyAccess: whether the owner (expeditionary owner if positive, else
/// roster owner, raw) has core supply access in the controller area of
/// `province`, the division's location province. Read once per (owner,
/// province), only for divisions whose noncore-supply term can apply (not
/// exiled and without [`DivisionStatusRow::MOVE_NONCORE_SUPPLY_BYPASS`]). Without
/// access the movement speed adds the noncore-supply speed define
/// ([`DivisionStatusRow::MOVE_NONCORE_AIR`] selects its air form). `None` is a
/// counted read refusal.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CoreSupplyAccessRow {
    pub owner: u32,
    pub province: u32,
    pub access: Option<bool>,
}
opaque_value!(Option<bool>);
typed_codec!(CoreSupplyAccessRow, 64, |row| (row.owner as u128) << 32 | row.province as u128,
|key| Self { owner: (key >> 32) as u32, province: key as u32, ..Default::default() },
{ 0 => (access): Opaque }, |_row| Ok(()));

/// CountryUnitSpeedEntries: one raw entry of a country's modifier accumulator,
/// keyed by modifier index, for every key of the movement priority sub-unit's
/// speed keys ([`DivisionTemplateIconRow::speed_keys`]) of the templates the
/// country's divisions use; `country` is the owner reference as DivisionRow holds
/// it (expeditionary owner if positive, else roster owner, raw). The entry is made
/// effective on playback with the recorded definitions. The outer `None` is a
/// counted read refusal, the inner `None` an absent entry.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CountryUnitSpeedEntryRow {
    pub country: u32,
    pub key: u32,
    pub entry: Option<Option<i64>>,
}
typed_codec!(CountryUnitSpeedEntryRow, 64, |row| (row.country as u128) << 32 | row.key as u128,
|key| Self { country: (key >> 32) as u32, key: key as u32, ..Default::default() },
{ 0 => (entry): Opaque }, |_row| Ok(()));

/// RiverRelations: the raw `river_crossing_factor_against` entry of the owner's
/// diplomatic relation towards `controller`, which a river crossing into a
/// province that country controls adds to the commander block's river crossing
/// factor. `owner` is the expeditionary owner if positive, else the roster owner,
/// and `controller` the raw Provinces controller of a route province, both raw.
/// The outer `None` is a counted read refusal; the inner `None` is no relation or
/// no entry.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct RiverRelationRow {
    pub owner: u32,
    pub controller: u32,
    pub entry: Option<Option<i64>>,
}
typed_codec!(RiverRelationRow, 64, |row| (row.owner as u128) << 32 | row.controller as u128,
|key| Self { owner: (key >> 32) as u32, controller: key as u32, ..Default::default() },
{ 0 => (entry): Opaque }, |_row| Ok(()));

/// Selected source rail-info building. `building: None` is an observed null
/// building; the enclosing row's absent optional state is unreached or
/// unavailable. Recorded branch gates distinguish when it is demanded.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct RailwayBuildingState {
    pub building: Option<RailwayBuildingHealth>,
}
value_struct!(RailwayBuildingState {
    building: Option<RailwayBuildingHealth>,
});

/// Native signed building values for directed departure health. Partial health
/// is required only on the reached damaged branch; preserve its raw sentinels.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct RailwayBuildingHealth {
    pub level: i16,
    pub healthy: i16,
    pub partial: Option<i64>,
}
value_struct!(RailwayBuildingHealth {
    level: i16,
    healthy: i16,
    partial: Option<i64>,
});

/// Current-hour railway-selection inputs, independent of sorted RailEdges.
/// Levels retain every signed entry in definition-neighbour order. None
/// in a domain is a counted capture refusal. Building-level None can also
/// be outside both gates; Some(None) observes the null pointer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct RailwaySelectionInputs {
    pub levels: Option<Vec<i32>>,
    pub pair_count: Option<i32>,
    pub building_level: Option<Option<i16>>,
}
value_struct!(RailwaySelectionInputs {
    levels: Option<Vec<i32>>,
    pair_count: Option<i32>,
    building_level: Option<Option<i16>>,
});

opaque_value!(Option<RailwayBuildingState>);
opaque_value!(Option<RailwaySelectionInputs>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    fn round_trip<T: Value + std::fmt::Debug + PartialEq>(value: &T) -> Vec<u8> {
        let mut bytes = Vec::new();
        Value::encode(value, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(T::decode(&mut input).unwrap(), *value);
        input.finish().unwrap();
        bytes
    }

    #[test]
    fn historical_gap_refusal_null_building_and_zero_health_stay_distinct() {
        type Field = Recorded<Option<RailwayBuildingState>>;
        let historical = <Field as Value>::not_recorded().unwrap();
        assert_eq!(historical, Recorded::NotRecorded);
        assert_eq!(Value::recorded_bytes(&historical).unwrap(), 0);
        assert!(Value::encode(&historical, &mut Vec::new()).is_err());

        let refusal = Recorded::Value(None);
        let null = Recorded::Value(Some(RailwayBuildingState { building: None }));
        let zero = Recorded::Value(Some(RailwayBuildingState {
            building: Some(RailwayBuildingHealth {
                level: 0,
                healthy: 0,
                partial: None,
            }),
        }));
        let encoded = [
            round_trip::<Field>(&refusal),
            round_trip::<Field>(&null),
            round_trip::<Field>(&zero),
        ];
        assert_eq!(encoded[0], [0]);
        assert_eq!(encoded[1], [1, 0]);
        assert_eq!(encoded[2], [1, 1, 0, 0, 0, 0, 0]);
    }

    fn check_suffix<T: RowCodec + Fields + std::fmt::Debug>(
        table: TableId,
        index: usize,
        previous: &T,
        row: &T,
    ) {
        assert_eq!(table.field_count(), T::SCHEMA.len());
        assert_eq!(table.mask_width(), T::SCHEMA.len().div_ceil(8));
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(&T::decode_fields(row.key(), &mut input).unwrap(), row);
        input.finish().unwrap();
        bytes.clear();
        let delta = row.delta(Some(previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << index);
        encode_delta(table, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let delta = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(&delta.apply(Some(previous.clone())).unwrap(), row);
    }

    #[test]
    fn fresh_suffixes_round_trip_and_change_without_legacy_fields() {
        let previous = StateRow {
            id: u32::MAX,
            movement_infrastructure: 918,
            ..Default::default()
        };
        for value in [0, i64::MIN, i64::MAX] {
            check_suffix(
                TableId::States,
                7,
                &previous,
                &StateRow {
                    railway_infrastructure: Some(value).into(),
                    ..previous.clone()
                },
            );
        }
        let previous = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: -1,
            ..Default::default()
        };
        for building in [
            None,
            Some(RailwayBuildingHealth {
                level: 1,
                healthy: 0,
                partial: Some(i64::MIN),
            }),
        ] {
            check_suffix(
                TableId::RailCooldowns,
                1,
                &previous,
                &RailCooldownRow {
                    building: Some(RailwayBuildingState { building }).into(),
                    ..previous.clone()
                },
            );
        }
    }

    fn historical_prefix<T: RowCodec + Fields + std::fmt::Debug>(
        table: TableId,
        index: usize,
        row: &T,
    ) -> T {
        let introduced = table
            .last_change()
            .appended
            .iter()
            .find(|field| field.index == index)
            .expect("fresh suffix has a compatibility entry")
            .introduced;
        let format = crate::format::FormatVersion::read(introduced - 1).unwrap();
        let fields = format.fields(table);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        for kind in &T::SCHEMA[..fields] {
            kind.decode(&mut input).unwrap();
        }
        let prefix = &bytes[..bytes.len() - input.remaining()];
        let mut input = Decoder::new(prefix);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = T::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(
            decoded.not_recorded_fields(),
            full_mask(T::SCHEMA.len()) ^ full_mask(fields)
        );
        assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
        assert!(decoded.encode_fields(&mut Vec::new()).is_err());

        let delta = row.delta(None).unwrap().unwrap();
        let mut input = Decoder::new(&delta.values);
        for kind in &T::SCHEMA[..fields] {
            kind.decode_change(&mut input).unwrap();
        }
        bytes.clear();
        table.encode_key(row.key(), 0, &mut bytes).unwrap();
        bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
        bytes.extend_from_slice(&delta.values[..delta.values.len() - input.remaining()]);
        let mut input = Decoder::new(&bytes);
        input.format = format.version();
        input.recorded_fields = fields;
        let delta = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(delta.apply::<T>(None).unwrap(), decoded);
        decoded
    }

    #[test]
    fn selected_railway_suffix_preserves_signed_native_domains_and_legacy_absence() {
        let previous = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: -1,
            building: Some(RailwayBuildingState { building: None }).into(),
            ..Default::default()
        };
        let input = RailwaySelectionInputs {
            levels: Some(vec![i32::MIN, 0, i32::MAX]),
            pair_count: Some(i32::MIN),
            building_level: Some(Some(i16::MIN)),
        };
        let row = RailCooldownRow {
            selection: Some(input.clone()).into(),
            ..previous.clone()
        };
        check_suffix(TableId::RailCooldowns, 2, &previous, &row);
        check_suffix(
            TableId::RailCooldowns,
            3,
            &row,
            &RailCooldownRow {
                selection_hour: Some(i32::MIN).into(),
                ..row.clone()
            },
        );
        let current = RailCooldownRow {
            selection_hour: Some(12).into(),
            ..row.clone()
        };
        check_suffix(
            TableId::RailCooldowns,
            3,
            &current,
            &RailCooldownRow {
                selection_hour: Some(13).into(),
                ..current.clone()
            },
        );
        let old = historical_prefix(TableId::RailCooldowns, 1, &row);
        assert_eq!(old.selection, Recorded::NotRecorded);
        assert_eq!(old.selection_hour, Recorded::NotRecorded);
        assert_eq!(old.building, Recorded::NotRecorded);
        assert_eq!(old.cooldown_ticks, -1);
        // All three suffixes share their introduction, so this selects the
        // same historical prefix rather than one containing `building`.
        let old = historical_prefix(TableId::RailCooldowns, 2, &row);
        assert_eq!(old.building, Recorded::NotRecorded);
        assert_eq!(old.selection, Recorded::NotRecorded);
        assert_eq!(old.selection_hour, Recorded::NotRecorded);
        let missing = round_trip(&Recorded::<Option<RailwaySelectionInputs>>::Value(None));
        let empty = round_trip(&Recorded::Value(Some(RailwaySelectionInputs {
            levels: Some(Vec::new()),
            pair_count: Some(0),
            building_level: Some(None),
        })));
        assert_ne!(missing, empty);
        let unqueried = round_trip(&RailwaySelectionInputs {
            building_level: None,
            ..input.clone()
        });
        let null = round_trip(&RailwaySelectionInputs {
            building_level: Some(None),
            ..input.clone()
        });
        let zero = round_trip(&RailwaySelectionInputs {
            building_level: Some(Some(0)),
            ..input
        });
        assert_ne!(unqueried, null);
        assert_ne!(null, zero);
    }

    #[test]
    fn movement_state_rail_prefixes_and_current_country_deltas_preserve_values() {
        let row = CountryCounterInputRow {
            forward_visibility: crate::rows::Recorded::Value(None),
            country: u32::MAX,
            first_division: u64::MAX,
            movement: Recorded::Value(Retired),
            ..Default::default()
        };
        let next = CountryCounterInputRow {
            owned_state_count: i32::MIN,
            air_combat_modifiers_known: true.into(),
            air_nav_efficiency: Some(i64::MAX).into(),
            ..row.clone()
        };
        let delta = next.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 1) | (1 << 29) | (1 << 33));
        assert_eq!(delta.apply(Some(row)).unwrap(), next);
        assert_eq!(next.movement, Recorded::Value(Retired));
        let row = StateRow {
            id: u32::MAX,
            controller: 71,
            movement_infrastructure: i64::MIN,
            railway_infrastructure: Some(0).into(),
            ..Default::default()
        };
        let decoded = historical_prefix(TableId::States, 7, &row);
        assert_eq!(decoded.railway_infrastructure, Recorded::NotRecorded);
        assert_eq!(decoded.controller, row.controller);
        assert_eq!(decoded.movement_infrastructure, row.movement_infrastructure);
        let row = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: i32::MIN,
            building: Some(RailwayBuildingState { building: None }).into(),
            selection: Default::default(),
            selection_hour: Default::default(),
        };
        let decoded = historical_prefix(TableId::RailCooldowns, 1, &row);
        assert_eq!(decoded.building, Recorded::NotRecorded);
        assert_eq!(decoded.cooldown_ticks, row.cooldown_ticks);
    }

    /// The retired table keeps its published framing, which older recordings
    /// need to discard it.
    #[test]
    fn published_eta_snapshot_and_delta_keep_all_eighteen_fields_and_bytes() {
        let row = DivisionEtaInputsRow {
            division: u64::MAX,
            sample_hour: i32::MIN,
            speed_modifier: i64::MIN,
            speed_base: i64::MAX,
            speed_org: Some(-1),
            speed_fuel: None,
            current_speed: -7,
            terrain_speed: vec![TerrainSpeed {
                terrain: 17,
                modifier: i64::MIN,
            }],
            fort_penalty: -2,
            snow_penalty: -3,
            river_adjustment: -4,
            river_leader: -5,
            class: u32::MAX,
            rail_country_modifier: Some(i64::MIN),
            rail_state_modifier: None,
            rail_friendly_state_modifier: Some(i64::MAX),
            leg_river_combined: vec![i64::MIN, 0, i64::MAX],
            leg_transport_speed: vec![-1, 0, 1],
            leg_rail_speed: vec![7, -7, 0],
        };
        // Accepted99 fields 5/14/15/16 are Opaque: each u32 byte-length
        // envelope precedes the vector's separate u32 element count. Keep
        // this golden independent of the row, Field and Value encoders.
        let mut expected = Vec::new();
        expected.extend_from_slice(&i64::MIN.to_le_bytes()); // 0: speed_modifier
        expected.extend_from_slice(&i64::MAX.to_le_bytes()); // 1: speed_base
        expected.push(1); // 2: speed_org = Some(-1)
        expected.extend_from_slice(&(-1_i64).to_le_bytes());
        expected.push(0); // 3: speed_fuel = None
        expected.extend_from_slice(&(-7_i64).to_le_bytes()); // 4: current_speed
        expected.extend_from_slice(&16_u32.to_le_bytes()); // 5: opaque payload length
        expected.extend_from_slice(&1_u32.to_le_bytes()); // terrain entry count
        expected.extend_from_slice(&17_i32.to_le_bytes());
        expected.extend_from_slice(&i64::MIN.to_le_bytes());
        for value in [-2_i64, -3, -4, -5] {
            // 6..9: fort, snow, river, leader
            expected.extend_from_slice(&value.to_le_bytes());
        }
        expected.extend_from_slice(&u32::MAX.to_le_bytes()); // 10: class
        expected.push(1); // 11: rail_country_modifier = Some(MIN)
        expected.extend_from_slice(&i64::MIN.to_le_bytes());
        expected.push(0); // 12: rail_state_modifier = None
        expected.push(1); // 13: rail_friendly_state_modifier = Some(MAX)
        expected.extend_from_slice(&i64::MAX.to_le_bytes());
        for values in [[i64::MIN, 0, i64::MAX], [-1, 0, 1], [7, -7, 0]] {
            expected.extend_from_slice(&28_u32.to_le_bytes()); // 14..16: opaque length
            expected.extend_from_slice(&3_u32.to_le_bytes()); // vector element count
            for value in values {
                expected.extend_from_slice(&value.to_le_bytes());
            }
        }
        expected.extend_from_slice(&i32::MIN.to_le_bytes()); // 17: sample_hour
        let mut actual = Vec::new();
        row.encode_fields(&mut actual).unwrap();
        assert_eq!(actual.len(), 209);
        for (offset, length, count) in [
            (34, 16_u32, 1_u32),
            (109, 28, 3),
            (141, 28, 3),
            (173, 28, 3),
        ] {
            assert_eq!(&actual[offset..offset + 4], &length.to_le_bytes());
            assert_eq!(&actual[offset + 4..offset + 8], &count.to_le_bytes());
        }
        assert_eq!(actual, expected);
        let format = crate::format::FormatVersion::read(99).unwrap();
        let fields = format.fields(TableId::DivisionEtaInputs);
        assert_eq!(fields, 18);
        assert_eq!(DivisionEtaInputsRow::SCHEMA.len(), fields);
        assert_eq!(TableId::DivisionEtaInputs.mask_width(), 3);
        let mut input = Decoder::new(&actual);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = DivisionEtaInputsRow::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded, row);
        assert_eq!(decoded.not_recorded_fields(), 0);
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.mask, full_mask(18));
        let mut wire = Vec::new();
        encode_delta(TableId::DivisionEtaInputs, &delta, 0, &mut wire).unwrap();
        assert_eq!(
            &wire[..13],
            &[
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0xff, 0xff, 0x03,
            ]
        );
        let mut input = Decoder::new(&wire);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = decode_delta(TableId::DivisionEtaInputs, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded.mask, full_mask(18));
        assert_eq!(decoded.apply::<DivisionEtaInputsRow>(None).unwrap(), row);
    }
}
