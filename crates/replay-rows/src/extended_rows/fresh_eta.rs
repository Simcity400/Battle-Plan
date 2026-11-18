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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwayBuildingState {
    pub building: Option<RailwayBuildingHealth>,
}
value_struct!(RailwayBuildingState {
    building: Option<RailwayBuildingHealth>,
});

/// Native signed building values for directed departure health. Partial health
/// is required only on the reached damaged branch; preserve its raw sentinels.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

