use crate::codec::{Decoder, invalid_static as invalid, put_bytes, put_varint};
use crate::display::{Rounding, Step};
pub use crate::event_feed_rows::EventFeedHistoryRow;
pub use crate::map_event_rows::*;
use crate::measurement::{self, Counter};
pub use crate::nuclear_rows::*;
pub use crate::peace_rows::*;
use crate::value::{EncodeValue, Value};
pub use combat_rows::*;
use engine_allocation as allocation;
pub use extended_rows::*;
pub use land_rows::*;
pub use modifier_breakdown_rows::*;
pub use naval_mission_rows::*;
pub use politics_rows::*;
pub use production_modifier_rows::*;
use std::io;
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ClockRow {
    pub hour: i32,
    pub play_time: u64,
    pub play_session_started_at: i64,
    pub seed: i32,
    pub speed: i32,
    pub debug_current_ref_id: Option<u32>,
    pub unit: Option<i32>,
    pub order_index: Option<i32>,
    pub front_index: Option<i32>,
    pub theatre_index: Option<i32>,
    pub theater_group_index: Option<i32>,
    pub military_deployment_line_index: Option<i32>,
    pub military_deployment_conveyor_index: Option<i32>,
    pub equipment_variant_index: Option<i32>,
    pub country_leader_index: Option<i32>,
    pub navy_id: Option<i32>,
    pub land_combat_id: Option<i32>,
    pub multiplayer_random_count: Option<i32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CampaignRow {
    pub id: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryRow {
    pub id: u32,
    pub canonical_slot: u32,
    pub tag: Vec<u8>,
    pub political_power_raw: Option<i64>,
    pub command_power_raw: i64,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub convoys_raw: Option<i64>,
    pub peace_conference_participant: u8,
    pub manpower_residual: i32,
    pub border_land_doctrine_level: Recorded<Option<i32>>,
}


#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static VALIDATED_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryLinkRow {
    pub id: u32,
    pub canonical_slot: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionRise {
    pub hour: i32,
    pub count: u32,
}
crate::value::value_struct!(ConstructionRise { hour: i32, count: u32 });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceRow {
    pub id: u32,
    pub state: u32,
    pub owner: u32,
    pub controller: u32,
    pub victory_points: i32,
    pub fort_positive: u8,
    pub move_gate: u8,
    pub port_sea: i32,
    pub attack: Recorded<Witnessed<ProvinceAttackInputs>>,
    pub selected_areas: Recorded<Retired>,
    pub fort_bunker: Recorded<Option<i64>>,
    pub controller_area: Recorded<Option<u32>>,
    pub owner_area: Recorded<Option<u32>>,
    pub area_node: Recorded<Option<ControllerAreaNode>>,
}

impl ProvinceRow {
    pub const MOVE_NULL_OWNER_AREA: u8 = 1;
    pub const MOVE_BORDER_CONFLICT: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateRow {
    pub infrastructure: Recorded<Option<StateInfrastructure>>,
    pub id: u32,
    pub owner: u32,
    pub controller: u32,
    pub movement_infrastructure: i64,
    pub manpower_available: i32,
    pub manpower_locked: Recorded<i32>,
    pub custom_name: Vec<u8>,
    pub owner_name: Vec<u8>,
    pub continent: Recorded<Option<u32>>,
    pub railway_infrastructure: Recorded<Option<i64>>,
    pub population: Recorded<i32>,
    pub best_vp_index: Recorded<Option<i32>>,
    pub single_controller: Recorded<Option<u32>>,
    pub category: Recorded<Option<u32>>,
    pub marked_owners: Recorded<Option<Vec<u32>>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateInfrastructure {
    pub building_type: u32,
    pub level: i32,
    pub maximum: i32,
}
crate::value::value_struct!(StateInfrastructure { building_type: u32, level: i32, maximum: i32 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionRow {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub template: u64,
    pub hit_points: i64,
    pub organisation: i64,
    pub orders_group: u64,
    pub reveal_skip: u8,
    pub theatre: Recorded<Option<u64>>,
}

impl DivisionRow {
    pub const HIT_POINTS_STEP: Step = Step::fixed5(10_000, Rounding::TowardZeroNonzero);
    pub const ORGANISATION_STEP: Step = Step::fixed5(10_000, Rounding::TowardZero);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarRelationRow {
    pub first_country: u32,
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub cancel: u8,
    pub first_casualties: i64,
    pub second_casualties: i64,
    pub first_unknown_casualties: i64,
    pub second_unknown_casualties: i64,
    pub first_was_instigator: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionRow {
    pub id: u64,
    pub name: Vec<u8>,
    pub template_token: Option<u32>,
    pub color_red: u32,
    pub color_green: u32,
    pub color_blue: u32,
    pub color_alpha: u32,
    pub color_overridden: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionMemberRow {
    pub country: u32,
    pub faction: u64,
    pub ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PlayerRow {
    pub ordinal: Recorded<u32>,
    pub id: i32,
    pub country: u32,
    pub flags: u8,
    pub user: Vec<u8>,
    pub name: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct HumanControllerRow {
    pub country: u32,
    pub count: i8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatRow {
    pub id: u64,
    pub province: u32,
    pub duration: i32,
    pub day: i32,
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
    pub attacker_classes: Vec<u32>,
    pub defender_classes: Vec<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FleetRow {
    pub id: u64,
    pub owner: u32,
    pub home_base: u32,
    pub name: Recorded<Option<Vec<u8>>>,
    pub leader: Recorded<Option<u64>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TaskForceRow {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub mission: u32,
    pub route_count: i32,
    pub combat_references: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipRow {
    pub name: Recorded<Option<Vec<u8>>>,
    pub naming: Recorded<Option<ShipNamingOperands>>,
    pub required_manpower: Recorded<Option<i32>>,
    pub id: u64,
    pub task_force: u64,
    pub strength: i64,
    pub organisation: i64,
    pub experience: i64,
    pub category: u32,
    pub sunk_convoys: i32,
    pub class_id: i32,
    pub acquired_manpower: Recorded<Option<i32>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipNamingOperands {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub override_name: Vec<u8>,
    pub programmatic: u8,
    pub equipment: Option<u64>,
}
crate::value::value_struct!(ShipNamingOperands {
    kind: i32, order: i32, ordered: u8, override_name: Vec<u8>,
    programmatic: u8, equipment: Option<u64>,
});

impl ShipRow {
    pub const EXPERIENCE_STEP: Step = Step::fixed5(1_000, Rounding::TowardZero);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatRow {
    pub id: u64,
    pub province: u32,
    pub attacker_positioning: i64,
    pub defender_positioning: i64,
    pub attacker_anti_air: i64,
    pub defender_anti_air: i64,
    pub registry_ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatParticipantRow {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: u64,
    pub owner: u32,
    pub state: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirWingRow {
    pub id: u64,
    pub owner: u32,
    pub air_base: u64,
    pub base_override: u64,
    pub country_override: i32,
    pub planes: i32,
    pub acquired_manpower: Recorded<Option<i32>>,
    pub exile_country: Recorded<Option<i32>>,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
    pub base_range_raw: i64,
    pub ace: u64,
    pub mission_active: u8,
    pub eligibility_veto: u8,
    pub eligibility_counter: i32,
    pub eligibility_limit: i32,
    pub attached_orders_group: Option<u64>,
    pub pool_ordinal: u32,
    pub name: Recorded<Option<Vec<u8>>>,
    pub pool_subunit: Recorded<Option<u32>>,
    pub raid_reference: Recorded<Option<AirWingRaidReference>>,
    pub strategic_bombing_raw: Recorded<Option<i64>>,
    pub replacement_setting: Recorded<Option<u8>>,
    pub map_flags: Recorded<Option<u32>>,
    pub fuel_base_raw: Recorded<Option<i64>>,
    pub fuel_current_type: Recorded<Option<i32>>,
    pub fuel_effective_aircraft: Recorded<Option<i32>>,
    pub fuel_context_mode: Recorded<Option<u32>>,
    pub fuel_capacity_class: Recorded<Option<i32>>,
    pub fuel_override_resolved: Recorded<Option<u32>>,
}

impl AirWingRow {
    pub fn fuel_inputs(&self) -> Result<AirWingFuelInputs, String> {
        Ok(AirWingFuelInputs {
            base_fuel: *self.fuel_base_raw.require("AirWings.fuel_base_raw")?,
            current_type: *self
                .fuel_current_type
                .require("AirWings.fuel_current_type")?,
            effective_aircraft: *self
                .fuel_effective_aircraft
                .require("AirWings.fuel_effective_aircraft")?,
            context_mode: self
                .fuel_context_mode
                .require("AirWings.fuel_context_mode")?
                .map(u8::try_from)
                .transpose()
                .map_err(|_| "Air fuel context is outside its stored byte".to_owned())?,
            capacity_class: *self
                .fuel_capacity_class
                .require("AirWings.fuel_capacity_class")?,
            override_resolved: match self
                .fuel_override_resolved
                .require("AirWings.fuel_override_resolved")?
            {
                Some(0) => Some(false),
                Some(1) => Some(true),
                None => None,
                _ => {
                    return Err(
                        "Air fuel override liveness is outside its native boolean domain".into(),
                    );
                }
            },
        })
    }
    pub fn training_stop(&self) -> Option<bool> {
        self.map_flag(0)
    }
    pub fn attachment_coverage(&self) -> Option<bool> {
        self.map_flag(2)
    }
    pub fn assigned_to_raid(&self) -> Option<bool> {
        self.map_flag(4)
    }
    fn map_flag(&self, bit: u32) -> Option<bool> {
        match self.map_flags {
            Recorded::Value(Some(flags)) if flags & (1 << bit) != 0 => {
                Some(flags & (2 << bit) != 0)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirWingRaidReference {
    pub raw: u64,
    pub resolves: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirWingFuelInputs {
    pub base_fuel: Option<i64>,
    pub current_type: Option<i32>,
    pub effective_aircraft: Option<i32>,
    pub context_mode: Option<u8>,
    pub capacity_class: Option<i32>,
    pub override_resolved: Option<bool>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyOrderRow {
    pub owner_group: u64,
    pub instance: u32,
    pub country: u32,
    pub order_type: i32,
    pub active: u8,
    pub path: Vec<u32>,
    pub target_states: Vec<u32>,
    pub paradrop_target: u32,
}

pub trait RowsVisit {
    type Output;
    fn rows<R: RowCodec>(self, rows: &[R]) -> Self::Output;
}

pub trait RowsPairVisit {
    type Output;
    fn rows<R: RowCodec>(self, rows: &[R], other: &[R]) -> Self::Output;
}

pub trait RowCodec: Default + Clone + Eq {
    fn try_clone(&self) -> io::Result<Self>;
    fn try_clone_validated(&self) -> io::Result<Self>;
    fn heap_bytes(&self) -> usize;
    fn encoded_bound(&self) -> io::Result<usize>;
    fn snapshot_bytes(&self) -> io::Result<usize>;
    const KEY_BITS: u32;
    fn key(&self) -> u128;
    fn check_key_fields(&self) -> io::Result<()>;
    fn check_selected_fields(&self, mask: u64) -> io::Result<()> {
        let _ = mask;
        self.check_key_fields()
    }
    fn not_recorded_fields(&self) -> u64;
    fn differing_fields(&self, previous: &Self) -> u64 {
        let _ = previous;
        u64::MAX
    }
    fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str));
    fn visit_wire(
        &self,
        visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
    ) -> io::Result<()>;
    fn from_wire<'a>(
        key: u128,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Self>;
    fn patch_from_wire<'a>(
        &mut self,
        mask: u64,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<bool> {
        let _ = (mask, wire);
        Ok(false)
    }
    fn from_wire_fields<'a>(
        key: u128,
        mask: u64,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Self> {
        let _ = mask;
        Self::from_wire(key, wire)
    }
    fn recorded_fields_bytes(&self) -> io::Result<usize>;
    fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>>;
    fn encode_change(
        &self,
        previous: Option<&Self>,
        previous_key: u128,
        output: &mut Vec<u8>,
    ) -> io::Result<bool> {
        allocation::reserve(output, self.encoded_bound()?)?;
        self.encode_change_reserved(previous, previous_key, output)
    }
    fn encode_change_reserved(
        &self,
        previous: Option<&Self>,
        previous_key: u128,
        output: &mut Vec<u8>,
    ) -> io::Result<bool>;
    fn encode_fields(&self, output: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(output, self.snapshot_bytes()?)?;
        self.encode_fields_reserved(output)
    }
    fn encode_fields_reserved(&self, output: &mut Vec<u8>) -> io::Result<()>;
    fn encode_recorded_fields(&self, output: &mut Vec<u8>) -> io::Result<u64> {
        self.encode_fields(output)?;
        Ok(0)
    }
    fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self>;
    fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self>;
    fn has_display_steps(&self) -> bool;
    fn round_to_display(&mut self);
}

#[doc(hidden)]
pub trait Fields {
    const SCHEMA: &'static [Kind];
    const STEPS: &'static [Option<Step>];
    const NAMES: &'static [&'static str];
}


pub mod compatibility;

#[path = "rows/checkpoint.rs"]
pub mod checkpoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Recorded<T> {
    NotRecorded,
    Value(T),
}
impl<T: Default> Default for Recorded<T> {
    fn default() -> Self {
        Self::Value(T::default())
    }
}
impl<T> From<T> for Recorded<T> {
    fn from(value: T) -> Self {
        Self::Value(value)
    }
}
impl<T> Recorded<T> {
    pub fn require_mut(&mut self, field: &str) -> Result<&mut T, String> {
        match self {
            Self::Value(value) => Ok(value),
            Self::NotRecorded => Err(format!("{field} not recorded")),
        }
    }
    pub fn require(&self, field: &str) -> Result<&T, String> {
        match self {
            Self::Value(value) => Ok(value),
            Self::NotRecorded => Err(format!("{field} not recorded")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeltaRow {
    pub format: u16,
    pub key: u128,
    pub mask: u64,
    pub recorded_fields: usize,
    pub values: Vec<u8>,
}
impl DeltaRow {
    pub fn apply<T: RowCodec>(&self, previous: Option<T>) -> io::Result<T> {
        T::apply_delta(self, previous)
    }
}
pub fn varint_bytes(value: u128) -> usize {
    (128 - value.leading_zeros() as usize).div_ceil(7).max(1)
}

pub const fn full_mask(count: usize) -> u64 {
    if count == 64 {
        u64::MAX
    } else {
        (1_u64 << count) - 1
    }
}
pub fn check_native_key(key: u128, bits: u32) -> io::Result<()> {
    if bits == 0 && key != 0 {
        return Err(invalid("singleton row key must be zero"));
    }
    if bits < 128 && key >> bits != 0 {
        return Err(invalid("row key exceeds native width"));
    }
    Ok(())
}
#[cfg(any(test, feature = "test-support"))]
pub fn encode_row<T: RowCodec>(row: &T, previous: u128, output: &mut Vec<u8>) -> io::Result<()> {
    allocation::reserve(output, row.snapshot_bytes()?)?;
    encode_row_reserved(row, previous, output)
}
pub fn encode_row_reserved<T: RowCodec>(
    row: &T,
    previous: u128,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    row.check_key_fields()?;
    check_native_key(row.key(), T::KEY_BITS)?;
    let difference = row
        .key()
        .checked_sub(previous)
        .ok_or_else(|| invalid("row keys decreased"))?;
    put_varint(output, difference);
    row.encode_fields_reserved(output)
}
pub fn decode_row<T: RowCodec>(previous: u128, input: &mut Decoder<'_>) -> io::Result<T> {
    let key = previous
        .checked_add(input.varint()?)
        .ok_or_else(|| invalid("row key addition overflow"))?;
    check_native_key(key, T::KEY_BITS)?;
    T::decode_fields(key, input)
}
pub fn validate_typed<T: RowCodec>(rows: &[T]) -> io::Result<()> {
    let mut previous = None;
    for row in rows {
        #[cfg(any(test, feature = "test-support"))]
        VALIDATED_ROWS.with(|count| count.set(count.get() + 1));
        measurement::count(Counter::ValidatedRows, 1);
        let key = row.key();
        check_native_key(key, T::KEY_BITS)?;
        row.check_key_fields()?;
        if previous.is_some_and(|old| old >= key) {
            return Err(invalid(
                "table rows must have matching types and unique ascending keys",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}
pub fn encode_typed<T: RowCodec>(rows: &[T], output: &mut Vec<u8>) -> io::Result<()> {
    let bytes = rows
        .iter()
        .try_fold(0, |size, row| allocation::add(size, row.snapshot_bytes()?))?;
    allocation::reserve(output, bytes)?;
    let mut previous = 0;
    for row in rows {
        encode_row_reserved(row, previous, output)?;
        previous = row.key();
    }
    Ok(())
}
pub fn patch_typed<'a, T: RowCodec>(
    rows: &mut [T],
    patches: &[(u128, u64)],
    wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
) -> io::Result<()> {
    let mut from = 0;
    for (at, &(key, mask)) in patches.iter().enumerate() {
        let position = (from < rows.len())
            .then(|| locate(rows, from, key).ok())
            .flatten()
            .ok_or_else(|| invalid("a patched row is not in its table"))?;
        let row = &mut rows[position];
        if !row.patch_from_wire(mask, &mut |field| wire(at, field))? {
            *row = T::from_wire(key, &mut |field| wire(at, field))?;
        }
        from = position + 1;
    }
    Ok(())
}
pub fn rows_from_wire_typed<'a, T: RowCodec>(
    keys: &[u128],
    wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
) -> io::Result<Vec<T>> {
    let mut rows = allocation::vec(keys.len())?;
    for (at, &key) in keys.iter().enumerate() {
        rows.push(T::from_wire(key, &mut |field| wire(at, field))?);
    }
    Ok(rows)
}
pub fn rows_from_wire_fields_typed<'a, T: RowCodec>(
    keys: &[u128],
    mask: u64,
    wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
) -> io::Result<Vec<T>> {
    let mut rows = allocation::vec(keys.len())?;
    for (at, &key) in keys.iter().enumerate() {
        rows.push(T::from_wire_fields(key, mask, &mut |field| wire(at, field))?);
    }
    Ok(rows)
}
pub fn decode_typed<T: RowCodec>(count: usize, input: &mut Decoder<'_>) -> io::Result<Vec<T>> {
    let mut rows = allocation::vec(count)?;
    let mut previous = 0;
    for _ in 0..count {
        let row = decode_row::<T>(previous, input)?;
        previous = row.key();
        rows.push(row);
    }
    Ok(rows)
}
#[derive(Debug, Default)]
pub struct ChangedRows {
    pub pairs: Vec<(u32, Option<u32>)>,
    pub removed: Vec<u128>,
    pub lost: bool,
}

impl ChangedRows {
    pub fn pair(&mut self, row: u32, baseline: Option<u32>) {
        if !self.lost && allocation::advisory(|| allocation::reserve(&mut self.pairs, 1)).is_ok() {
            self.pairs.push((row, baseline));
        } else {
            self.lose();
        }
    }

    pub fn remove(&mut self, key: u128) {
        if !self.lost && allocation::advisory(|| allocation::reserve(&mut self.removed, 1)).is_ok() {
            self.removed.push(key);
        } else {
            self.lose();
        }
    }

    pub fn lose(&mut self) {
        self.lost = true;
        self.pairs = Vec::new();
        self.removed = Vec::new();
    }
}

pub fn count_header(output: &mut Vec<u8>) -> io::Result<usize> {
    allocation::reserve(output, 8)?;
    let at = output.len();
    output.extend_from_slice(&0_u32.to_le_bytes());
    Ok(at)
}

pub fn encode_changes_typed<T: RowCodec>(
    rows: &[T],
    previous: &[T],
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
    mut changed: Option<&mut ChangedRows>,
) -> io::Result<bool> {
    u32::try_from(rows.len()).map_err(|_| invalid("too many rows"))?;
    u32::try_from(previous.len()).map_err(|_| invalid("too many rows"))?;
    let start = output.len();
    let mut count_at = None;
    if changed.is_none() {
        count_at = Some(count_header(output)?);
    }
    let mut changed_count = 0_u32;
    let mut changed_key = 0_u128;
    deleted.clear();
    let mut deleted_count = 0_u32;
    let mut deleted_key = 0_u128;
    let mut delete = |key, changed: &mut Option<&mut ChangedRows>| -> io::Result<()> {
        allocation::reserve(deleted, (T::KEY_BITS as usize).div_ceil(7).max(1))?;
        put_varint(deleted, key - deleted_key);
        deleted_key = key;
        deleted_count += 1;
        if let Some(changed) = changed {
            changed.remove(key);
        }
        Ok(())
    };
    let mut old = previous.iter().enumerate().peekable();
    for (at, row) in rows.iter().enumerate() {
        let key = row.key();
        while old.peek().is_some_and(|(_, previous)| previous.key() < key) {
            delete(old.next().unwrap().1.key(), &mut changed)?;
        }
        let baseline = if old.peek().is_some_and(|(_, previous)| previous.key() == key) {
            old.next()
        } else {
            None
        };
        if baseline.is_some_and(|(_, baseline)| baseline == row) {
            continue;
        }
        if count_at.is_none() {
            count_at = Some(count_header(output)?);
        }
        if row.encode_change(baseline.map(|(_, baseline)| baseline), changed_key, output)? {
            changed_count += 1;
            changed_key = key;
            if let Some(changed) = &mut changed {
                changed.pair(at as u32, baseline.map(|(before, _)| before as u32));
            }
        }
    }
    for (_, row) in old {
        delete(row.key(), &mut changed)?;
    }
    if changed.is_some() && changed_count == 0 && deleted_count == 0 {
        output.truncate(start);
        return Ok(false);
    }
    let count_at = match count_at {
        Some(at) => at,
        None => count_header(output)?,
    };
    output[count_at..count_at + 4].copy_from_slice(&changed_count.to_le_bytes());
    allocation::reserve(output, allocation::add(4, deleted.len())?)?;
    output.extend_from_slice(&deleted_count.to_le_bytes());
    output.extend_from_slice(deleted);
    Ok(changed_count != 0 || deleted_count != 0)
}
pub fn encode_replacements_typed<T: RowCodec>(
    rows: &[T],
    previous: &[T],
    indices: &[usize],
    output: &mut Vec<u8>,
) -> io::Result<bool> {
    if rows.len() != indices.len()
        || indices.windows(2).any(|pair| pair[0] >= pair[1])
        || rows
            .iter()
            .zip(indices)
            .any(|(row, &index)| previous.get(index).is_none_or(|old| row.key() != old.key()))
    {
        return Err(invalid(
            "replacement rows must name unique ascending baseline positions with unchanged keys",
        ));
    }
    u32::try_from(rows.len()).map_err(|_| invalid("too many replacement rows"))?;
    allocation::reserve(output, 8)?;
    let count_at = output.len();
    output.extend_from_slice(&0_u32.to_le_bytes());
    let mut changed_count = 0_u32;
    let mut changed_key = 0_u128;
    for (row, &index) in rows.iter().zip(indices) {
        let old = &previous[index];
        if row != old && row.encode_change(Some(old), changed_key, output)? {
            changed_count += 1;
            changed_key = row.key();
        }
    }
    output[count_at..count_at + 4].copy_from_slice(&changed_count.to_le_bytes());
    allocation::reserve(output, 4)?;
    output.extend_from_slice(&0_u32.to_le_bytes());
    Ok(changed_count != 0)
}

pub fn round_rows_to_display<T: RowCodec>(rows: &mut [T]) {
    if rows.first().is_some_and(RowCodec::has_display_steps) {
        for row in rows {
            row.round_to_display();
        }
    }
}

pub fn apply_typed<T: RowCodec>(
    previous: Vec<T>,
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    apply_typed_at(previous, changed, deleted).map(|(rows, _)| rows)
}

pub fn apply_typed_at<T: RowCodec>(
    mut previous: Vec<T>,
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<(Vec<T>, Option<Vec<usize>>)> {
    let mut at = allocation::vec(changed.len())?;
    let mut from = 0;
    for delta in changed {
        let found = locate(&previous, from, delta.key);
        from = found.unwrap_or_else(|position| position);
        at.push(found);
    }
    let mut gone = allocation::vec(deleted.len())?;
    let mut from = 0;
    for &key in deleted {
        let Ok(position) = locate(&previous, from, key) else {
            let len = previous.len();
            return merge_typed(previous, len, changed, deleted, Ok, None).map(|rows| (rows, None));
        };
        from = position + 1;
        gone.push(position);
    }
    let inserted = at.iter().filter(|found| found.is_err()).count();
    if inserted == 0 && gone.is_empty() {
        let mut positions = allocation::vec(at.len())?;
        for (delta, found) in changed.iter().zip(at) {
            let position = found.unwrap_or_else(|_| unreachable!("no row is inserted"));
            let row = std::mem::take(&mut previous[position]);
            previous[position] = delta.apply(Some(row))?;
            positions.push(position);
        }
        return Ok((previous, Some(positions)));
    }
    if inserted + gone.len() > IN_PLACE_EDITS {
        return merge_runs(previous, changed, &at, &gone)
            .map(|(rows, positions)| (rows, Some(positions)));
    }
    let mut rows_in = allocation::vec(inserted)?;
    for (delta, found) in changed.iter().zip(&at) {
        match *found {
            Ok(position) => {
                let row = std::mem::take(&mut previous[position]);
                previous[position] = delta.apply(Some(row))?;
            }
            Err(position) => rows_in.push((position, delta.apply::<T>(None)?)),
        }
    }
    for &position in gone.iter().rev() {
        measurement::count(Counter::ShiftedRows, previous.len() - position - 1);
        previous.remove(position);
    }
    allocation::reserve(&mut previous, rows_in.len())?;
    let mut removed = gone.iter().peekable();
    let mut removed_before = 0;
    for (earlier, (position, row)) in rows_in.into_iter().enumerate() {
        while removed.next_if(|&&gone| gone < position).is_some() {
            removed_before += 1;
        }
        let position = position - removed_before + earlier;
        measurement::count(Counter::ShiftedRows, previous.len() - position);
        previous.insert(position, row);
    }
    let mut positions = allocation::vec(changed.len())?;
    let mut from = 0;
    for delta in changed {
        let position = locate(&previous, from, delta.key).unwrap_or_else(|position| position);
        from = position;
        positions.push(position);
    }
    Ok((previous, Some(positions)))
}

const IN_PLACE_EDITS: usize = 2;

fn locate<T: RowCodec>(rows: &[T], from: usize, key: u128) -> Result<usize, usize> {
    let rest = &rows[from..];
    let mut bound = 1;
    while bound < rest.len() && rest[bound].key() < key {
        bound *= 2;
    }
    let low = bound / 2;
    let high = (bound + 1).min(rest.len());
    match rest[low..high].binary_search_by_key(&key, T::key) {
        Ok(position) => Ok(from + low + position),
        Err(position) => Err(from + low + position),
    }
}

fn merge_runs<T: RowCodec>(
    previous: Vec<T>,
    changed: &[DeltaRow],
    at: &[Result<usize, usize>],
    gone: &[usize],
) -> io::Result<(Vec<T>, Vec<usize>)> {
    let mut rows = allocation::vec(allocation::add(previous.len(), changed.len())?)?;
    let mut positions = allocation::vec(changed.len())?;
    measurement::count(Counter::MergedRows, previous.len());
    let mut source = Runs {
        rows: previous.into_iter(),
        next: 0,
    };
    let mut gone = gone.iter().copied().peekable();
    for (delta, found) in changed.iter().zip(at) {
        let place = found.unwrap_or_else(|position| position);
        while let Some(position) = gone.next_if(|&position| position < place) {
            source.move_to(&mut rows, position);
            source.take();
        }
        source.move_to(&mut rows, place);
        positions.push(rows.len());
        match found {
            Ok(_) => rows.push(delta.apply(Some(source.take()))?),
            Err(_) => rows.push(delta.apply::<T>(None)?),
        }
    }
    for position in gone {
        source.move_to(&mut rows, position);
        source.take();
    }
    rows.extend(source.rows);
    Ok((rows, positions))
}

struct Runs<T> {
    rows: std::vec::IntoIter<T>,
    next: usize,
}

impl<T> Runs<T> {
    fn move_to(&mut self, out: &mut Vec<T>, end: usize) {
        out.extend(self.rows.by_ref().take(end - self.next));
        self.next = end;
    }
    fn take(&mut self) -> T {
        self.next += 1;
        self.rows.next().expect("a located row is present")
    }
}

pub fn apply_typed_validated<T: RowCodec>(
    previous: Vec<T>,
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    let (rows, at) = apply_typed_at(previous, changed, deleted)?;
    match at {
        Some(at) => validate_typed_at(&rows, &at)?,
        None => validate_typed(&rows)?,
    }
    Ok(rows)
}

pub fn validate_typed_at<T: RowCodec>(rows: &[T], at: &[usize]) -> io::Result<()> {
    let unordered = || invalid("table rows must have matching types and unique ascending keys");
    for (index, &position) in at.iter().enumerate() {
        #[cfg(any(test, feature = "test-support"))]
        VALIDATED_ROWS.with(|count| count.set(count.get() + 1));
        measurement::count(Counter::ValidatedRows, 1);
        let key = rows[position].key();
        check_native_key(key, T::KEY_BITS)?;
        rows[position].check_key_fields()?;
        if position > 0 && rows[position - 1].key() >= key {
            return Err(unordered());
        }
        let next = position + 1;
        if at.get(index + 1) != Some(&next) && rows.get(next).is_some_and(|row| key >= row.key()) {
            return Err(unordered());
        }
    }
    Ok(())
}

pub fn apply_typed_shared<T: RowCodec>(
    previous: &[T],
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    merge_typed(
        previous,
        previous.len(),
        changed,
        deleted,
        RowCodec::try_clone,
        None,
    )
}

pub fn apply_typed_shared_validated<T: RowCodec>(
    previous: &[T],
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    let mut at = allocation::vec(changed.len())?;
    let rows = merge_typed(
        previous,
        previous.len(),
        changed,
        deleted,
        RowCodec::try_clone_validated,
        Some(&mut at),
    )?;
    validate_typed_at(&rows, &at)?;
    Ok(rows)
}

pub fn merge_typed<T: RowCodec, R: std::borrow::Borrow<T>>(
    previous: impl IntoIterator<Item = R>,
    len: usize,
    changed: &[DeltaRow],
    deleted: &[u128],
    own: impl Fn(R) -> io::Result<T>,
    mut at: Option<&mut Vec<usize>>,
) -> io::Result<Vec<T>> {
    let mut rows = allocation::vec(allocation::add(len, changed.len())?)?;
    measurement::count(Counter::MergedRows, len);
    let mut changed = changed.iter().peekable();
    let mut deleted = deleted.iter().peekable();
    let mut note = |rows: &Vec<T>| {
        if let Some(at) = at.as_deref_mut() {
            at.push(rows.len());
        }
    };
    for previous in previous {
        let key = previous.borrow().key();
        if deleted.peek().is_some_and(|deleted| **deleted < key) {
            return Err(invalid("delta deletes an absent key"));
        }
        while changed.peek().is_some_and(|delta| delta.key < key) {
            note(&rows);
            rows.push(changed.next().unwrap().apply::<T>(None)?);
        }
        if deleted.peek().is_some_and(|deleted| **deleted == key) {
            deleted.next();
        } else if changed.peek().is_some_and(|delta| delta.key == key) {
            note(&rows);
            rows.push(changed.next().unwrap().apply(Some(own(previous)?))?);
        } else {
            rows.push(own(previous)?);
        }
    }
    if deleted.next().is_some() {
        return Err(invalid("delta deletes an absent key"));
    }
    for delta in changed {
        note(&rows);
        rows.push(delta.apply::<T>(None)?);
    }
    Ok(rows)
}

#[inline(never)]
pub fn rows_at_typed<T: RowCodec>(rows: &[T], keys: &[u128]) -> io::Result<Vec<T>> {
    let mut found = Vec::new();
    allocation::reserve(&mut found, keys.len())?;
    let mut from = 0;
    for &key in keys {
        let at = from
            + rows[from..]
                .binary_search_by_key(&key, |row| row.key())
                .map_err(|_| invalid("a written key is not held"))?;
        found.push(rows[at].try_clone_validated()?);
        from = at + 1;
    }
    Ok(found)
}

#[inline(never)]
pub fn changes_typed<T: RowCodec>(rows: &[T], previous: &[T]) -> io::Result<(Vec<T>, Vec<u128>)> {
    let (mut changed, mut deleted) = (Vec::new(), Vec::new());
    let (mut at, mut was) = (0, 0);
    loop {
        let order = match (rows.get(at), previous.get(was)) {
            (None, None) => break,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(row), Some(old)) => row.key().cmp(&old.key()),
        };
        match order {
            std::cmp::Ordering::Greater => {
                allocation::reserve(&mut deleted, 1)?;
                deleted.push(previous[was].key());
                was += 1;
            }
            order => {
                let row = &rows[at];
                if order == std::cmp::Ordering::Less || *row != previous[was] {
                    allocation::reserve(&mut changed, 1)?;
                    changed.push(row.try_clone_validated()?);
                }
                at += 1;
                was += usize::from(order == std::cmp::Ordering::Equal);
            }
        }
    }
    Ok((changed, deleted))
}

#[inline(never)]
pub fn apply_changes_typed<T: RowCodec>(
    mut rows: Vec<T>,
    changed: Vec<T>,
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    let ascending = |mut keys: std::slice::Iter<'_, u128>| {
        let mut last = keys.next();
        keys.all(|key| last.replace(key).is_some_and(|last| last < key))
    };
    let keys = allocation::collect_vec(changed.iter().map(RowCodec::key))?;
    if !ascending(keys.iter()) || !ascending(deleted.iter()) {
        return merge_changes(rows, changed, deleted);
    }
    let mut places = allocation::vec(keys.len())?;
    let mut from = 0;
    for &key in &keys {
        let at = locate(&rows, from, key);
        from = match at {
            Ok(position) => position + 1,
            Err(position) => position,
        };
        places.push(at);
    }
    let mut gone = allocation::vec(deleted.len())?;
    from = 0;
    for &key in deleted {
        let position =
            locate(&rows, from, key).map_err(|_| invalid("row changes delete an absent key"))?;
        gone.push(position);
        from = position + 1;
    }
    let mut set = places.iter().filter_map(|at| at.ok()).peekable();
    gone.retain(|&position| {
        while set.next_if(|&at| at < position).is_some() {}
        set.peek() != Some(&position)
    });
    let mut inserted = allocation::vec(places.iter().filter(|at| at.is_err()).count())?;
    let mut removed = gone.iter().peekable();
    let mut before = 0;
    for (at, row) in places.into_iter().zip(changed) {
        match at {
            Ok(position) => rows[position] = row,
            Err(position) => {
                while removed.next_if(|&&gone| gone < position).is_some() {
                    before += 1;
                }
                inserted.push((position - before, row));
            }
        }
    }
    if !gone.is_empty() {
        let (mut index, mut removed) = (0, gone.iter().peekable());
        rows.retain(|_| {
            let kept = removed.next_if(|&&gone| gone == index).is_none();
            index += 1;
            kept
        });
    }
    if !inserted.is_empty() {
        let (mut read, holes) = (rows.len(), inserted.len());
        allocation::resize_default(&mut rows, allocation::add(read, holes)?)?;
        let mut write = rows.len();
        for (position, row) in inserted.into_iter().rev() {
            while read > position {
                read -= 1;
                write -= 1;
                rows.swap(read, write);
            }
            write -= 1;
            rows[write] = row;
        }
    }
    Ok(rows)
}

pub fn merge_changes<T: RowCodec>(
    rows: Vec<T>,
    changed: Vec<T>,
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    let mut out = allocation::vec(allocation::add(rows.len(), changed.len())?)?;
    let mut changed = changed.into_iter().peekable();
    let mut deleted = deleted.iter().peekable();
    for row in rows {
        let key = row.key();
        while let Some(change) = changed.next_if(|change| change.key() < key) {
            out.push(change);
        }
        if deleted.peek().is_some_and(|deleted| **deleted < key) {
            return Err(invalid("row changes delete an absent key"));
        }
        if deleted.next_if(|deleted| **deleted == key).is_some() {
            continue;
        }
        out.push(changed.next_if(|change| change.key() == key).unwrap_or(row));
    }
    out.extend(changed);
    if deleted.next().is_some() {
        return Err(invalid("row changes delete an absent key"));
    }
    Ok(out)
}

#[doc(hidden)]
pub trait NativeValue: Sized {
    const HAS_STEPS: bool = false;
    const RESTORED: bool = false;
    fn restored_tombstone(&self) -> bool {
        false
    }
    fn try_clone(&self) -> io::Result<Self>;
    fn heap_bytes(&self) -> usize;
    fn field(&self) -> Field<'_>;
    fn from_field(field: Field<'_>) -> io::Result<Self>;
    fn not_recorded() -> io::Result<Self> {
        Err(invalid("uncatalogued absent row field"))
    }
    fn visit_field_gaps(&self, _visit: &mut dyn FnMut(&'static str)) {}
    fn round_display(&mut self) {}
    fn round_to(&mut self, _step: Step) {}
    fn to_units(&self, _step: Step) -> io::Result<Self> {
        Err(invalid("row field takes no display step"))
    }
    #[allow(clippy::wrong_self_convention, reason = "the decoded value is consumed into native units")]
    fn from_units(self, _step: Step) -> io::Result<Self> {
        Err(invalid("row field takes no display step"))
    }
}
impl<T: NativeValue> NativeValue for Recorded<T> {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Value(value) => NativeValue::heap_bytes(value),
            Self::NotRecorded => 0,
        }
    }
    const HAS_STEPS: bool = T::HAS_STEPS;
    fn restored_tombstone(&self) -> bool {
        T::RESTORED && matches!(self, Self::NotRecorded)
    }
    fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        if let Self::Value(value) = self {
            value.visit_field_gaps(visit);
        }
    }
    fn round_display(&mut self) {
        if let Self::Value(value) = self {
            value.round_display();
        }
    }
    fn round_to(&mut self, step: Step) {
        if let Self::Value(value) = self {
            value.round_to(step);
        }
    }
    fn to_units(&self, step: Step) -> io::Result<Self> {
        match self {
            Self::NotRecorded => Ok(Self::NotRecorded),
            Self::Value(value) => value.to_units(step).map(Self::Value),
        }
    }
    fn from_units(self, step: Step) -> io::Result<Self> {
        match self {
            Self::NotRecorded => Ok(Self::NotRecorded),
            Self::Value(value) => value.from_units(step).map(Self::Value),
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        match self {
            Self::NotRecorded => Ok(Self::NotRecorded),
            Self::Value(v) => T::try_clone(v).map(Self::Value),
        }
    }
    fn field(&self) -> Field<'_> {
        match self {
            Self::NotRecorded => Field::NotRecorded,
            Self::Value(v) => v.field(),
        }
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        if T::RESTORED
            && let Field::Opaque(Opaque::Encoded(bytes, format)) = &field
            && bytes.is_empty()
            && *format >= compatibility::RETIRED_INPUTS_FORMAT
        {
            return Ok(Self::NotRecorded);
        }
        T::from_field(field).map(Self::Value)
    }
    fn not_recorded() -> io::Result<Self> {
        Ok(Self::NotRecorded)
    }
}
pub fn read_field<T: NativeValue>(index: usize, kind: Kind, input: &mut Decoder<'_>) -> io::Result<T> {
    if index >= input.recorded_fields {
        T::not_recorded()
    } else {
        read_value(kind, input)
    }
}
pub fn read_stepped_field<T: NativeValue>(
    index: usize,
    kind: Kind,
    step: Option<Step>,
    input: &mut Decoder<'_>,
) -> io::Result<T> {
    let value: T = read_field(index, kind, input)?;
    match step {
        Some(step) if crate::display::stepped_format(input.format) => value.from_units(step),
        _ => Ok(value),
    }
}
pub fn round_native_field<T: NativeValue>(value: &mut T, step: Option<Step>) {
    match step {
        Some(step) => value.round_to(step),
        None if T::HAS_STEPS => value.round_display(),
        None => {}
    }
}
pub fn native_has_steps<T: NativeValue>(_: &T, step: Option<Step>) -> bool {
    step.is_some() || T::HAS_STEPS
}
pub fn native_wire_field<T: NativeValue>(value: &T) -> Field<'_> {
    if value.restored_tombstone() {
        Field::Opaque(Opaque::Encoded(&[], crate::FORMAT_VERSION))
    } else {
        value.field()
    }
}
pub fn visit_wire_field<T: NativeValue>(
    value: &T,
    index: usize,
    step: Option<Step>,
    visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
) -> io::Result<()> {
    match step {
        None => visit(index, native_wire_field(value)),
        Some(step) => visit(index, value.to_units(step)?.field()),
    }
}

pub const fn column<R: Fields>(name: &str) -> usize {
    let mut at = 0;
    while at < R::NAMES.len() {
        if same_name(R::NAMES[at], name) {
            return at;
        }
        at += 1;
    }
    panic!("no such schema field")
}

pub const fn mask<R: Fields>(names: &[&str]) -> u64 {
    let (mut mask, mut at) = (0_u64, 0);
    while at < names.len() {
        mask |= 1 << column::<R>(names[at]);
        at += 1;
    }
    mask
}

pub const fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut at = 0;
    while at < a.len() {
        if a[at] != b[at] {
            return false;
        }
        at += 1;
    }
    true
}

pub fn from_wire_field<T: NativeValue>(field: Field<'_>, step: Option<Step>) -> io::Result<T> {
    if matches!(field, Field::NotRecorded) {
        return T::not_recorded();
    }
    let value = T::from_field(field)?;
    match step {
        Some(step) => value.from_units(step),
        None => Ok(value),
    }
}
pub fn from_wire_encoded<'a, T: RowCodec>(
    key: u128,
    fields: usize,
    wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
) -> io::Result<T> {
    let mut bytes = Vec::new();
    for index in 0..fields {
        wire(index)?.encode(&mut bytes)?;
    }
    let mut input = Decoder::new(&bytes);
    let row = T::decode_fields(key, &mut input)?;
    input.finish()?;
    Ok(row)
}
pub fn encode_native_field<T: NativeValue>(
    value: &T,
    step: Option<Step>,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    match step {
        None => native_wire_field(value).encode_reserved(output),
        Some(step) => value.to_units(step)?.field().encode_reserved(output),
    }
}
pub fn encode_native_change<T: NativeValue>(
    value: &T,
    previous: Option<&T>,
    step: Option<Step>,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    match step {
        None => native_wire_field(value)
            .change(previous.map_or(Field::Empty, native_wire_field))?
            .encode(output),
        Some(step) => {
            let units = value.to_units(step)?;
            let previous = previous.map(|old| old.to_units(step)).transpose()?;
            units
                .field()
                .change(previous.as_ref().map_or(Field::Empty, NativeValue::field))?
                .encode(output)
        }
    }
}
pub fn apply_stepped_value<T: NativeValue>(
    target: &mut T,
    kind: Kind,
    step: Option<Step>,
    input: &mut Decoder<'_>,
) -> io::Result<()> {
    match step {
        Some(step) if crate::display::stepped_format(input.format) => {
            let mut units = target.to_units(step)?;
            apply_value(&mut units, kind, input)?;
            *target = units.from_units(step)?;
            Ok(())
        }
        _ => apply_value(target, kind, input),
    }
}
macro_rules! native_scalar {
    ($ty:ty, $kind:ident) => {
        impl NativeValue for $ty {
            fn heap_bytes(&self) -> usize {
                0
            }
            fn try_clone(&self) -> io::Result<Self> {
                Ok(*self)
            }
            fn round_to(&mut self, step: Step) {
                <$ty as Value>::round_to(self, step);
            }
            fn to_units(&self, step: Step) -> io::Result<Self> {
                <$ty as Value>::to_units(self, step)
            }
            fn from_units(self, step: Step) -> io::Result<Self> {
                <$ty as Value>::from_units(self, step)
            }
            fn field(&self) -> Field<'_> {
                Field::$kind(*self)
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                match field {
                    Field::$kind(value) => Ok(value),
                    _ => Err(invalid("field type differs from schema")),
                }
            }
        }
    };
}
native_scalar!(u8, U8);
native_scalar!(i8, I8);
native_scalar!(u32, U32);
native_scalar!(i32, I32);
native_scalar!(u64, U64);
native_scalar!(i64, I64);
native_scalar!(Option<i64>, OptionalI64);
native_scalar!(Option<u32>, OptionalU32);
native_scalar!(u16, U16);
native_scalar!(i16, I16);
native_scalar!(bool, Bool);
native_scalar!(Option<i32>, OptionalI32);
native_scalar!(Option<u64>, OptionalU64);
impl NativeValue for Vec<u8> {
    fn heap_bytes(&self) -> usize {
        self.capacity()
    }
    fn try_clone(&self) -> io::Result<Self> {
        allocation::copy_vec(self)
    }
    fn field(&self) -> Field<'_> {
        Field::Bytes(self)
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::Bytes(value) => allocation::copy_vec(value),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}
impl NativeValue for String {
    fn heap_bytes(&self) -> usize {
        self.capacity()
    }
    fn try_clone(&self) -> io::Result<Self> {
        allocation::string(self)
    }
    fn field(&self) -> Field<'_> {
        Field::Bytes(self.as_bytes())
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::Bytes(value) => allocation::string(
                std::str::from_utf8(value).map_err(|_| invalid("invalid UTF-8 string"))?,
            ),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}
macro_rules! native_vector {
    ($ty:ty, $kind:ident) => {
        impl NativeValue for Vec<$ty> {
            fn heap_bytes(&self) -> usize {
                self.capacity() * size_of::<$ty>()
            }
            fn try_clone(&self) -> io::Result<Self> {
                allocation::copy_vec(self)
            }
            fn field(&self) -> Field<'_> {
                Field::$kind(List::Native(self))
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                match field {
                    Field::$kind(value) => value.to_vec(),
                    _ => Err(invalid("field type differs from schema")),
                }
            }
        }
    };
}
native_vector!(u32, U32List);
native_vector!(u64, U64List);
pub fn read_value<T: NativeValue>(kind: Kind, input: &mut Decoder<'_>) -> io::Result<T> {
    T::from_field(kind.decode(input)?)
}
pub fn apply_value<T: NativeValue>(
    target: &mut T,
    kind: Kind,
    input: &mut Decoder<'_>,
) -> io::Result<()> {
    let value = T::from_field(kind.decode_change(input)?.apply(kind, target.field())?)?;
    *target = value;
    Ok(())
}
#[doc(hidden)]
pub fn column_error(key: u128, column: &'static str) -> impl FnOnce(io::Error) -> io::Error {
    move |error| column_context(error, key, column)
}
#[cold]
#[inline(never)]
fn column_context(error: io::Error, key: u128, column: &str) -> io::Error {
    crate::codec::error_context(error, format_args!("row {key} column {column}"))
}
#[macro_export]
#[doc(hidden)]
macro_rules! typed_codec {
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| $initial:expr,
     { $($index:literal => ($($field:tt)+) : $kind:ident $(@ $step:path)?),+ $(,)? }, |$checked:ident| $check:expr) => {
        $crate::typed_codec!(
            $ty,
            $bits,
            |$row| $key,
            |$key_name| $initial,
            { $($index => ($($field)+): $kind $(@ $step)?),+ },
            |$checked| $check,
            |_withheld_field, _withheld_format| false
        );
    };
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| $initial:expr,
     { $($index:literal => ($($field:tt)+) : $kind:ident $(@ $step:path)?),+ $(,)? }, |$checked:ident| $check:expr, |$wf:ident, $wfmt:ident| $withhold:expr) => {
        $crate::typed_codec!(
            $ty,
            $bits,
            |$row| $key,
            |$key_name| $initial,
            { $($index => ($($field)+): $kind $(@ $step)?),+ },
            |$checked, mask| {
                let all = $crate::rows::full_mask(Self::SCHEMA.len());
                if mask & all == all {
                    $check
                } else {
                    Ok(())
                }
            },
            |$wf, $wfmt| $withhold
        );
    };
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| $initial:expr,
     { $($index:literal => ($($field:tt)+) : $kind:ident $(@ $step:path)?),+ $(,)? }, |$checked:ident, $mask:ident| $check:expr) => {
        $crate::typed_codec!(
            $ty,
            $bits,
            |$row| $key,
            |$key_name| $initial,
            { $($index => ($($field)+): $kind $(@ $step)?),+ },
            |$checked, $mask| $check,
            |_withheld_field, _withheld_format| false
        );
    };
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| $initial:expr,
     { $($index:literal => ($($field:tt)+) : $kind:ident $(@ $step:path)?),+ $(,)? }, |$checked:ident, $mask:ident| $check:expr, |$wf:ident, $wfmt:ident| $withhold:expr) => {
        $crate::row!($ty, |$key_name| $initial, { $(($($field)+): $kind),+ });
        #[cfg(any(test, feature = "test-support"))]
        impl $crate::checkpoint::endpoint::sparse::SparseReference for $ty {
            fn sparse_fixture(&self, field: usize) -> $crate::checkpoint::endpoint::sparse::SparseFixture<Self> {
                use $crate::checkpoint::endpoint::sparse::{SparseFixture, different, native_bytes, native_copy};
                let baseline = RowCodec::try_clone_validated(self).unwrap();
                let mut source = RowCodec::try_clone_validated(self).unwrap();
                $(source.$($field)+ = different(&baseline.$($field)+, Kind::$kind, $index);)+
                let mut expected = RowCodec::try_clone_validated(&baseline).unwrap();
                let payload = match field {
                    $($index => {
                        expected.$($field)+ = native_copy(&source.$($field)+, Kind::$kind);
                        native_bytes(&source.$($field)+)
                    },)+
                    _ => panic!("unknown schema field {field}"),
                };
                SparseFixture { baseline, source, expected, payload }
            }
        }
        impl Fields for $ty {
            const SCHEMA: &'static [Kind] = &[$(Kind::$kind),+];
            const STEPS: &'static [Option<$crate::display::Step>] = &[$($crate::display_step!($($step)?)),+];
            const NAMES: &'static [&'static str] = &[$(stringify!($($field)+)),+];
        }
        impl $ty {
            pub const fn key(&self) -> u128 { let $row = self; $key }
            fn restored_tombstones(&self) -> u64 {
                let mut mask = 0;
                $(if NativeValue::restored_tombstone(&self.$($field)+) { mask |= 1 << $index; })+
                mask
            }
        }
        impl RowCodec for $ty {
            const KEY_BITS: u32 = $bits;
            fn heap_bytes(&self) -> usize {
                0 $(+ NativeValue::heap_bytes(&self.$($field)+))+
            }
            fn key(&self) -> u128 { self.key() }
            fn has_display_steps(&self) -> bool {
                [$(native_has_steps(&self.$($field)+, $crate::display_step!($($step)?))),+].contains(&true)
            }
            fn round_to_display(&mut self) {
                $(round_native_field(&mut self.$($field)+, $crate::display_step!($($step)?));)+
            }
            fn check_key_fields(&self) -> io::Result<()> { self.check_selected_fields(full_mask(Self::SCHEMA.len())) }
            fn check_selected_fields(&self, $mask: u64) -> io::Result<()> { let $checked = self; $check }
            fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                $(NativeValue::visit_field_gaps(&self.$($field)+, visit);)+
            }
            fn visit_wire(
                &self,
                visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
            ) -> io::Result<()> {
                $(visit_wire_field(&self.$($field)+, $index, $crate::display_step!($($step)?), visit).map_err($crate::rows::column_error(self.key(), stringify!($($field)+)))?;)+
                Ok(())
            }
            fn not_recorded_fields(&self) -> u64 {
                let mut mask = 0;
                $(if matches!(self.$($field)+.field(), Field::NotRecorded) { mask |= 1 << $index; })+
                mask
            }
            fn differing_fields(&self, previous: &Self) -> u64 {
                let mut mask = 0;
                $(if self.$($field)+ != previous.$($field)+ { mask |= 1 << $index; })+
                mask
            }
            #[allow(clippy::field_reassign_with_default, reason = "a key initializer also serves rows with nested fields")]
            fn try_clone(&self) -> io::Result<Self> {
                self.check_key_fields()?;
                check_native_key(self.key(), Self::KEY_BITS)?;
                self.try_clone_validated()
            }
            #[allow(clippy::field_reassign_with_default, reason = "a key initializer also serves rows with nested fields")]
            fn try_clone_validated(&self) -> io::Result<Self> {
                let $key_name = self.key();
                let mut row = $initial;
                $(row.$($field)+ = NativeValue::try_clone(&self.$($field)+)?;)+
                Ok(row)
            }
            fn recorded_fields_bytes(&self) -> io::Result<usize> {
                let mut bytes = 0;
                $(if !matches!(self.$($field)+.field(), Field::NotRecorded) { bytes = allocation::add(bytes, self.$($field)+.field().recorded_bytes()?)?; })+
                Ok(bytes)
            }
            fn snapshot_bytes(&self) -> io::Result<usize> {
                let size = ($bits as usize).div_ceil(7).max(1);
                $(let size = allocation::add(size, native_wire_field(&self.$($field)+).encoded_bytes().map_err($crate::rows::column_error(self.key(), stringify!($($field)+)))?)?;)+
                Ok(size)
            }
            fn encoded_bound(&self) -> io::Result<usize> {
                let size = ($bits as usize).div_ceil(7).max(1) + Self::SCHEMA.len().div_ceil(8);
                $(let size = allocation::add(size, native_wire_field(&self.$($field)+).encoded_bound().map_err($crate::rows::column_error(self.key(), stringify!($($field)+)))?)?;)+
                Ok(size)
            }
            fn encode_change_reserved(&self, previous: Option<&Self>, previous_key: u128, output: &mut Vec<u8>) -> io::Result<bool> {
                let missing = self.not_recorded_fields() & !self.restored_tombstones();
                if missing != 0 { return Err($crate::codec::error_context(invalid("row fields not recorded"), format_args!("row {} column {}", self.key(), Self::NAMES[missing.trailing_zeros() as usize]))); }
                self.check_key_fields()?;
                check_native_key(self.key(), Self::KEY_BITS)?;
                if let Some(previous) = previous {
                    previous.check_key_fields()?;
                    if previous.key() != self.key() { return Err(invalid("delta row and baseline have different keys")); }
                    if previous == self { return Ok(false); }
                }
                let mask = match previous { Some(old) => self.differing_fields(old), None => 0 $(| 1_u64 << $index)+ };
                if mask == 0 { return Ok(false); }
                put_varint(output, self.key().checked_sub(previous_key).ok_or_else(|| invalid("row keys decreased"))?);
                output.extend_from_slice(&mask.to_le_bytes()[..Self::SCHEMA.len().div_ceil(8)]);
                $(if mask & (1_u64 << $index) != 0 {
                    encode_native_change(&self.$($field)+, match previous { Some(old) => Some(&old.$($field)+), None => None }, $crate::display_step!($($step)?), output).map_err($crate::rows::column_error(self.key(), stringify!($($field)+)))?;
                })+
                Ok(true)
            }
            fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> {
                let missing = self.not_recorded_fields() & !self.restored_tombstones();
                if missing != 0 { return Err($crate::codec::error_context(invalid("row fields not recorded"), format_args!("row {} column {}", self.key(), Self::NAMES[missing.trailing_zeros() as usize]))); }
                self.check_key_fields()?;
                check_native_key(self.key(), Self::KEY_BITS)?;
                if let Some(previous) = previous {
                    previous.check_key_fields()?;
                    if previous.key() != self.key() { return Err(invalid("delta row and baseline have different keys")); }
                    if self == previous { return Ok(None); }
                }
                let mask = match previous { Some(old) => self.differing_fields(old), None => 0 $(| 1_u64 << $index)+ };
                let mut delta = DeltaRow { format: $crate::FORMAT_VERSION, key: self.key(), mask, values: Vec::new(), recorded_fields: Self::SCHEMA.len() };
                $(
                    if mask & (1_u64 << $index) != 0 {
                        encode_native_change(&self.$($field)+, match previous { Some(old) => Some(&old.$($field)+), None => None }, $crate::display_step!($($step)?), &mut delta.values).map_err($crate::rows::column_error(self.key(), stringify!($($field)+)))?;
                    }
                )+
                Ok(Some(delta))
            }
            fn encode_recorded_fields(&self, output: &mut Vec<u8>) -> io::Result<u64> {
                let mut size = 0;
                let mut skipped = 0;
                $(match native_wire_field(&self.$($field)+) {
                    Field::NotRecorded => skipped |= 1 << $index,
                    field => size = allocation::add(size, field.encoded_bytes()?)?,
                })+
                allocation::reserve(output, size)?;
                $(if skipped & 1 << $index == 0 {
                    encode_native_field(&self.$($field)+, $crate::display_step!($($step)?), output).map_err($crate::rows::column_error(self.key(), stringify!($($field)+)))?;
                })+
                Ok(skipped)
            }
            fn encode_fields_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
                $(encode_native_field(&self.$($field)+, $crate::display_step!($($step)?), output).map_err($crate::rows::column_error(self.key(), stringify!($($field)+)))?;)+
                Ok(())
            }
            #[allow(clippy::field_reassign_with_default, reason = "a key initializer also serves rows with nested fields")]
            fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
                let $key_name = key;
                let mut row = $initial;
                $(row.$($field)+ = if {
                    let ($wf, $wfmt) = ($index, input.format);
                    $withhold
                } {
                    if $index < input.recorded_fields {
                        Kind::$kind.decode(input)?;
                    }
                    NativeValue::not_recorded()?
                } else {
                    read_stepped_field($index, Kind::$kind, $crate::display_step!($($step)?), input)?
                };)+
                Ok(row)
            }
            fn patch_from_wire<'a>(&mut self, mask: u64, wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>) -> io::Result<bool> {
                let key = self.key();
                $(if mask & (1_u64 << $index) != 0 {
                    self.$($field)+ = from_wire_field(wire($index)?, $crate::display_step!($($step)?)).map_err($crate::rows::column_error(key, stringify!($($field)+)))?;
                })+
                self.check_key_fields()?;
                Ok(true)
            }
            #[allow(clippy::field_reassign_with_default, reason = "a key initializer also serves rows with nested fields")]
            fn from_wire_fields<'a>(key: u128, mask: u64, wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>) -> io::Result<Self> {
                let all = full_mask(Self::SCHEMA.len());
                if mask & all == all { return Self::from_wire(key, wire); }
                check_native_key(key, Self::KEY_BITS)?;
                let $key_name = key;
                let mut row = $initial;
                $(if mask & (1_u64 << $index) != 0 {
                    row.$($field)+ = from_wire_field(wire($index)?, $crate::display_step!($($step)?)).map_err($crate::rows::column_error(key, stringify!($($field)+)))?;
                })+
                row.check_selected_fields(mask)?;
                Ok(row)
            }
            #[allow(clippy::field_reassign_with_default, reason = "a key initializer also serves rows with nested fields")]
            fn from_wire<'a>(key: u128, wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>) -> io::Result<Self> {
                check_native_key(key, Self::KEY_BITS)?;
                let $key_name = key;
                let mut row = $initial;
                $(row.$($field)+ = from_wire_field(wire($index)?, $crate::display_step!($($step)?)).map_err($crate::rows::column_error(key, stringify!($($field)+)))?;)+
                row.check_key_fields()?;
                Ok(row)
            }
            fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self> {
                check_native_key(delta.key, Self::KEY_BITS)?;
                let fields = delta.recorded_fields;
                if fields > Self::SCHEMA.len() { return Err(invalid("unknown recorded field count")); }
                let mask = full_mask(fields);
                if delta.mask == 0 || delta.mask & !mask != 0 { return Err(invalid("delta field mask is empty or has unknown bits")); }
                let mut row = if let Some(previous) = previous {
                    previous.check_key_fields()?;
                    if previous.key() != delta.key { return Err(invalid("delta row and baseline have different keys")); }
                    previous
                } else {
                    if delta.mask != mask { return Err(invalid("inserted row requires every field")); }
                    let $key_name = delta.key;
                    $initial
                };
                let mut input = Decoder::new(&delta.values);
                input.format = delta.format;
                $(if $index >= fields || {
                    let ($wf, $wfmt) = ($index, delta.format);
                    $withhold
                } {
                    row.$($field)+ = NativeValue::not_recorded()?;
                })+

                $(if delta.mask & (1_u64 << $index) != 0 {
                    if {
                        let ($wf, $wfmt) = ($index, delta.format);
                        $withhold
                    } {
                        Kind::$kind.decode_change(&mut input)?;
                        row.$($field)+ = NativeValue::not_recorded()?;
                    } else {
                        apply_stepped_value(&mut row.$($field)+, Kind::$kind, $crate::display_step!($($step)?), &mut input)?;
                    }
                })+
                input.finish()?;
                Ok(row)
            }
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! row_codec {
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| { $($key_field:ident : $key_value:expr),* $(,)? },
     { $($index:literal => $field:ident : $kind:ident $(@ $step:path)?),+ $(,)? }, |$checked:ident| $check:expr) => {
        typed_codec!($ty, $bits, |$row| $key,
            |$key_name| Self { $($key_field: $key_value,)* $($field: Default::default(),)+ },
            { $($index => ($field): $kind $(@ $step)?),+ }, |$checked| $check);
    };
}
row_codec!(ClockRow, 0, |_row| 0, |_key| {  }, {0 => hour: I32, 1 => play_time: U64, 2 => play_session_started_at: I64, 3 => seed: I32, 4 => speed: I32, 5 => debug_current_ref_id: OptionalU32, 6 => unit: OptionalI32, 7 => order_index: OptionalI32, 8 => front_index: OptionalI32, 9 => theatre_index: OptionalI32, 10 => theater_group_index: OptionalI32, 11 => military_deployment_line_index: OptionalI32, 12 => military_deployment_conveyor_index: OptionalI32, 13 => equipment_variant_index: OptionalI32, 14 => country_leader_index: OptionalI32, 15 => navy_id: OptionalI32, 16 => land_combat_id: OptionalI32, 17 => multiplayer_random_count: OptionalI32}, |_row| Ok(()));
row_codec!(CampaignRow, 0, |_row| 0, |_key| {  }, {0 => id: Bytes}, |_row| Ok(()));
mod event_feed;
row_codec!(CountryRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => canonical_slot: U32, 1 => tag: Bytes, 2 => political_power_raw: OptionalI64, 3 => command_power_raw: I64, 4 => stability_raw: I64, 5 => war_support_raw: I64, 6 => convoys_raw: OptionalI64, 7 => peace_conference_participant: U8, 8 => manpower_residual: I32, 9 => border_land_doctrine_level: OptionalI32}, |_row| Ok(()));
row_codec!(CountryLinkRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => canonical_slot: U32}, |_row| Ok(()));
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryExperienceUnsayableRow {
    pub country: u32,
    pub army_raw: Recorded<i64>,
    pub army_daily_raw: Recorded<i64>,
    pub army_daily_training_raw: Recorded<i64>,
    pub navy_raw: Recorded<i64>,
    pub navy_daily_raw: Recorded<i64>,
    pub navy_daily_training_raw: Recorded<i64>,
    pub air_raw: Recorded<i64>,
    pub air_daily_raw: Recorded<i64>,
    pub armies_for_training_raw: Recorded<i64>,
}
#[cfg(any(test, feature = "test-support"))]
row_codec!(CountryExperienceUnsayableRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => army_raw: I64, 1 => army_daily_raw: I64, 2 => army_daily_training_raw: I64, 3 => navy_raw: I64, 4 => navy_daily_raw: I64, 5 => navy_daily_training_raw: I64, 6 => air_raw: I64, 7 => air_daily_raw: I64, 8 => armies_for_training_raw: I64}, |_row| Ok(()));
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionRowAt141 {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub template: u64,
    pub hit_points: i64,
    pub organisation: i64,
    pub orders_group: u64,
    pub reveal_skip: u8,
}
#[cfg(any(test, feature = "test-support"))]
row_codec!(DivisionRowAt141, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => roster_owner: U32, 1 => logical_owner: U32, 2 => expeditionary_owner: U32, 3 => province: U32, 4 => template: U64, 5 => hit_points: I64 @ DivisionRow::HIT_POINTS_STEP, 6 => organisation: I64 @ DivisionRow::ORGANISATION_STEP, 7 => orders_group: U64, 8 => reveal_skip: U8}, |_row| Ok(()));
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateRowAt141 {
    pub id: u32,
    pub owner: u32,
    pub controller: u32,
    pub movement_infrastructure: i64,
    pub custom_name: Vec<u8>,
    pub owner_name: Vec<u8>,
    pub manpower_available: i32,
    pub continent: Recorded<Option<u32>>,
    pub railway_infrastructure: Recorded<Option<i64>>,
    pub population: Recorded<i32>,
    pub category: Recorded<Option<u32>>,
    pub manpower_locked: Recorded<i32>,
    pub best_vp_index: Recorded<Option<i32>>,
    pub single_controller: Recorded<Option<u32>>,
    pub infrastructure: Recorded<Option<StateInfrastructure>>,
}
#[cfg(any(test, feature = "test-support"))]
row_codec!(StateRowAt141, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => owner: U32, 1 => controller: U32, 2 => movement_infrastructure: I64, 3 => custom_name: Bytes, 4 => owner_name: Bytes, 5 => manpower_available: I32, 6 => continent: OptionalU32, 7 => railway_infrastructure: OptionalI64, 8 => population: I32, 9 => category: OptionalU32, 10 => manpower_locked: I32, 11 => best_vp_index: OptionalI32, 12 => single_controller: OptionalU32, 13 => infrastructure: Opaque}, |_row| Ok(()));
row_codec!(ProvinceRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => state: U32, 1 => owner: U32, 2 => controller: U32, 3 => victory_points: I32, 4 => fort_positive: U8, 5 => move_gate: U8, 6 => port_sea: I32, 7 => attack: Opaque, 8 => selected_areas: Opaque, 9 => fort_bunker: OptionalI64, 10 => controller_area: OptionalU32, 11 => owner_area: OptionalU32, 12 => area_node: Opaque}, |_row| Ok(()));
row_codec!(StateRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => owner: U32, 1 => controller: U32, 2 => movement_infrastructure: I64, 3 => custom_name: Bytes, 4 => owner_name: Bytes, 5 => manpower_available: I32, 6 => continent: OptionalU32, 7 => railway_infrastructure: OptionalI64, 8 => population: I32, 9 => category: OptionalU32, 10 => manpower_locked: I32, 11 => best_vp_index: OptionalI32, 12 => single_controller: OptionalU32, 13 => infrastructure: Opaque, 14 => marked_owners: Opaque}, |_row| Ok(()));
row_codec!(DivisionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => roster_owner: U32, 1 => logical_owner: U32, 2 => expeditionary_owner: U32, 3 => province: U32, 4 => template: U64, 5 => hit_points: I64 @ Self::HIT_POINTS_STEP, 6 => organisation: I64 @ Self::ORGANISATION_STEP, 7 => orders_group: U64, 8 => reveal_skip: U8, 9 => theatre: OptionalU64}, |_row| Ok(()));
row_codec!(WarRelationRow, 64, |row| (row.first_country as u128) << 32 | row.second_country as u128, |key| { first_country: (key >> 32) as u32, second_country: key as u32 }, {0 => start_hour: I32, 1 => end_hour: I32, 2 => cancel: U8, 3 => first_casualties: I64, 4 => second_casualties: I64, 5 => first_unknown_casualties: I64, 6 => second_unknown_casualties: I64, 7 => first_was_instigator: U8}, |_row| Ok(()));
row_codec!(FactionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => name: Bytes, 1 => template_token: OptionalU32, 2 => color_red: U32, 3 => color_green: U32, 4 => color_blue: U32, 5 => color_alpha: U32, 6 => color_overridden: U8}, |_row| Ok(()));
row_codec!(FactionMemberRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => faction: U64, 1 => ordinal: U32}, |_row| Ok(()));
row_codec!(PlayerRow, 32, |row| row.id as u32 as u128, |key| { id: key as u32 as i32 }, {0 => country: U32, 1 => flags: U8, 2 => user: Bytes, 3 => name: Bytes, 4 => ordinal: U32}, |_row| Ok(()));
row_codec!(HumanControllerRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => count: I8}, |_row| Ok(()));
row_codec!(LandCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => duration: I32, 2 => day: I32, 3 => attackers: U64List, 4 => defenders: U64List, 5 => attacker_classes: U32List, 6 => defender_classes: U32List}, |_row| Ok(()));
row_codec!(FleetRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => home_base: U32, 2 => name: Opaque, 3 => leader: OptionalU64}, |_row| Ok(()));
row_codec!(TaskForceRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => fleet: U64, 1 => roster_owner: U32, 2 => logical_owner: U32, 3 => expeditionary_owner: U32, 4 => province: U32, 5 => mission: U32, 6 => route_count: I32, 7 => combat_references: I32}, |_row| Ok(()));
row_codec!(ShipRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => task_force: U64, 1 => strength: I64, 2 => organisation: I64, 3 => experience: I64 @ Self::EXPERIENCE_STEP, 4 => category: U32, 5 => sunk_convoys: I32, 6 => class_id: I32, 7 => acquired_manpower: OptionalI32, 8 => required_manpower: OptionalI32, 9 => name: Opaque, 10 => naming: Opaque}, |_row| Ok(()));
row_codec!(NavalCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => attacker_positioning: I64, 2 => defender_positioning: I64, 3 => attacker_anti_air: I64, 4 => defender_anti_air: I64, 5 => registry_ordinal: U32}, |_row| Ok(()));
row_codec!(NavalCombatParticipantRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.battle_slot as u128, |key| { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32 }, {0 => ship: U64, 1 => owner: U32, 2 => state: U32}, |row| if row.side > 1 { Err(invalid("naval combat participant side exceeds one")) } else { Ok(()) });
row_codec!(AirWingRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => air_base: U64, 2 => base_override: U64, 3 => country_override: I32, 4 => planes: I32, 5 => mission_mask: U32, 6 => mission_region: OptionalU32, 7 => base_range_raw: I64, 8 => ace: U64, 9 => mission_active: U8, 10 => eligibility_veto: U8, 11 => eligibility_counter: I32, 12 => eligibility_limit: I32, 13 => attached_orders_group: OptionalU64, 14 => pool_ordinal: U32, 15 => name: Opaque, 16 => pool_subunit: OptionalU32, 17 => raid_reference: Opaque, 18 => strategic_bombing_raw: OptionalI64, 19 => acquired_manpower: OptionalI32, 20 => exile_country: OptionalI32, 21 => replacement_setting: Opaque, 22 => map_flags: OptionalU32, 23 => fuel_base_raw: OptionalI64, 24 => fuel_current_type: OptionalI32, 25 => fuel_effective_aircraft: OptionalI32, 26 => fuel_context_mode: OptionalU32, 27 => fuel_capacity_class: OptionalI32, 28 => fuel_override_resolved: OptionalU32}, |_row| Ok(()));
row_codec!(ArmyOrderRow, 96, |row| (row.owner_group as u128) << 32 | row.instance as u128, |key| { owner_group: (key >> 32) as u64, instance: key as u32 }, {0 => country: U32, 1 => order_type: I32, 2 => active: U8, 3 => paradrop_target: U32, 4 => path: U32List, 5 => target_states: U32List}, |_row| Ok(()));

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    Bool,
    OptionalI32,
    OptionalU32,
    OptionalI64,
    OptionalU64,
    Bytes,
    U32List,
    U64List,
    Opaque,
}
impl Kind {
    pub fn minimum_row_bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::I8 => 1,
            Self::U16 => 2,
            Self::I16 => 2,
            Self::U32 => 4,
            Self::I32 => 4,
            Self::U64 => 8,
            Self::I64 => 8,
            Self::Bool => 1,
            Self::OptionalI32 | Self::OptionalU32 | Self::OptionalI64 | Self::OptionalU64 => 1,
            Self::Bytes | Self::U32List | Self::U64List | Self::Opaque => 4,
        }
    }
    pub fn minimum_change_bytes(self) -> usize {
        match self {
            Self::Bytes | Self::U32List | Self::U64List | Self::Opaque => 4,
            _ => 1,
        }
    }
    pub fn decode_change<'a>(self, decoder: &mut Decoder<'a>) -> io::Result<Change<'a>> {
        if matches!(
            self,
            Self::Bytes | Self::U32List | Self::U64List | Self::Opaque
        ) {
            return Ok(match self.decode(decoder)? {
                Field::Bytes(value) => Change::Bytes(value),
                Field::U32List(value) => Change::U32List(value),
                Field::U64List(value) => Change::U64List(value),
                Field::Opaque(value) => Change::Opaque(value),
                _ => unreachable!(),
            });
        }
        if matches!(
            self,
            Self::OptionalI32 | Self::OptionalU32 | Self::OptionalI64 | Self::OptionalU64
        ) {
            match decoder.u8()? {
                0 => return Ok(Change::Optional(None)),
                1 => {}
                _ => return Err(invalid("invalid optional integer tag")),
            }
        }
        let encoded = decoder.varint()?;
        let difference = ((encoded >> 1) as i128) ^ -((encoded & 1) as i128);
        let maximum = match self {
            Self::U8 => u128::from(u8::MAX),
            Self::I8 => u128::from(u8::MAX),
            Self::U16 => u128::from(u16::MAX),
            Self::I16 => u128::from(u16::MAX),
            Self::U32 => u128::from(u32::MAX),
            Self::I32 => u128::from(u32::MAX),
            Self::U64 => u128::from(u64::MAX),
            Self::I64 => u128::from(u64::MAX),
            Self::Bool => 1,
            Self::OptionalI32 => u128::from(u32::MAX),
            Self::OptionalU32 => u128::from(u32::MAX),
            Self::OptionalI64 => u128::from(u64::MAX),
            Self::OptionalU64 => u128::from(u64::MAX),
            _ => unreachable!(),
        };
        if difference.unsigned_abs() > maximum {
            return Err(invalid("numeric difference exceeds native field range"));
        }
        Ok(
            if matches!(
                self,
                Self::OptionalI32 | Self::OptionalU32 | Self::OptionalI64 | Self::OptionalU64
            ) {
                Change::Optional(Some(difference))
            } else {
                Change::Number(difference)
            },
        )
    }
    fn number(self, value: i128) -> io::Result<Field<'static>> {
        let range = |_| invalid("numeric delta exceeds native field range");
        Ok(match self {
            Self::U8 => Field::U8(u8::try_from(value).map_err(range)?),
            Self::I8 => Field::I8(i8::try_from(value).map_err(range)?),
            Self::U16 => Field::U16(u16::try_from(value).map_err(range)?),
            Self::I16 => Field::I16(i16::try_from(value).map_err(range)?),
            Self::U32 => Field::U32(u32::try_from(value).map_err(range)?),
            Self::I32 => Field::I32(i32::try_from(value).map_err(range)?),
            Self::U64 => Field::U64(u64::try_from(value).map_err(range)?),
            Self::I64 => Field::I64(i64::try_from(value).map_err(range)?),
            Self::Bool => Field::Bool(match value {
                0 => false,
                1 => true,
                _ => return Err(invalid("numeric delta exceeds native field range")),
            }),
            Self::OptionalI32 => Field::OptionalI32(Some(i32::try_from(value).map_err(range)?)),
            Self::OptionalU32 => Field::OptionalU32(Some(u32::try_from(value).map_err(range)?)),
            Self::OptionalI64 => Field::OptionalI64(Some(i64::try_from(value).map_err(range)?)),
            Self::OptionalU64 => Field::OptionalU64(Some(u64::try_from(value).map_err(range)?)),
            _ => return Err(invalid("numeric change applied to list field")),
        })
    }
    pub fn decode<'a>(self, decoder: &mut Decoder<'a>) -> io::Result<Field<'a>> {
        Ok(match self {
            Self::U8 => Field::U8(<u8 as Value>::decode(decoder)?),
            Self::I8 => Field::I8(<i8 as Value>::decode(decoder)?),
            Self::U16 => Field::U16(<u16 as Value>::decode(decoder)?),
            Self::I16 => Field::I16(<i16 as Value>::decode(decoder)?),
            Self::U32 => Field::U32(<u32 as Value>::decode(decoder)?),
            Self::I32 => Field::I32(<i32 as Value>::decode(decoder)?),
            Self::U64 => Field::U64(<u64 as Value>::decode(decoder)?),
            Self::I64 => Field::I64(<i64 as Value>::decode(decoder)?),
            Self::Bool => Field::Bool(<bool as Value>::decode(decoder)?),
            Self::OptionalI32 => Field::OptionalI32(match decoder.u8()? {
                0 => None,
                1 => Some(<i32 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::OptionalU32 => Field::OptionalU32(match decoder.u8()? {
                0 => None,
                1 => Some(<u32 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::OptionalI64 => Field::OptionalI64(match decoder.u8()? {
                0 => None,
                1 => Some(<i64 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::OptionalU64 => Field::OptionalU64(match decoder.u8()? {
                0 => None,
                1 => Some(<u64 as Value>::decode(decoder)?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
            Self::Bytes | Self::Opaque => {
                let length = usize::try_from(decoder.u32()?)
                    .map_err(|_| invalid("row byte length exceeds address space"))?;
                let bytes = decoder.take(length)?;
                if matches!(self, Self::Bytes) {
                    Field::Bytes(bytes)
                } else {
                    Field::Opaque(Opaque::Encoded(bytes, decoder.format))
                }
            }
            Self::U32List => {
                let count = decoder.count(4)?;
                Field::U32List(List::Encoded(decoder.take(count * 4)?))
            }
            Self::U64List => {
                let count = decoder.count(8)?;
                Field::U64List(List::Encoded(decoder.take(count * 8)?))
            }
        })
    }
}
#[derive(Clone, Copy)]
pub enum Opaque<'a> {
    Native(&'a dyn EncodeValue),
    Encoded(&'a [u8], u16),
}
impl Opaque<'_> {
    pub fn encoded_bytes(self) -> io::Result<usize> {
        allocation::add(
            4,
            match self {
                Self::Encoded(bytes, _) => bytes.len(),
                Self::Native(value) => value.encoded_bytes()?,
            },
        )
    }
    pub fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(output, self.encoded_bytes()?)?;
        self.encode_reserved(output)
    }
    pub fn encode_reserved(self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Encoded(bytes, _) => put_bytes(output, bytes),
            Self::Native(value) => {
                let prefix = output.len();
                output.extend_from_slice(&0_u32.to_le_bytes());
                value.encode_value_reserved(output)?;
                let length = u32::try_from(output.len() - prefix - 4)
                    .map_err(|_| invalid("structured field exceeds u32"))?;
                output[prefix..prefix + 4].copy_from_slice(&length.to_le_bytes());
                Ok(())
            }
        }
    }
}
pub enum Change<'a> {
    Number(i128),
    Optional(Option<i128>),
    Bytes(&'a [u8]),
    U32List(List<'a, u32>),
    U64List(List<'a, u64>),
    Opaque(Opaque<'a>),
}
impl<'a> Change<'a> {
    pub fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let bytes = match self {
            Self::Bytes(value) => allocation::add(4, value.len())?,
            Self::U32List(value) => value.encoded_bytes()?,
            Self::U64List(value) => value.encoded_bytes()?,
            Self::Opaque(value) => value.encoded_bytes()?,
            Self::Number(value) | Self::Optional(Some(value)) => {
                let zigzag = ((*value as u128) << 1) ^ ((*value >> 127) as u128);
                (128 - zigzag.leading_zeros() as usize).div_ceil(7).max(1)
                    + usize::from(matches!(self, Self::Optional(_)))
            }
            Self::Optional(None) => 1,
        };
        allocation::reserve(output, bytes)?;
        let difference = match self {
            Self::Number(value) => *value,
            Self::Optional(Some(value)) => {
                output.push(1);
                *value
            }
            Self::Optional(None) => {
                output.push(0);
                return Ok(());
            }
            Self::Bytes(value) => return put_bytes(output, value),
            Self::U32List(value) => return value.encode(output),
            Self::U64List(value) => return value.encode(output),
            Self::Opaque(value) => return value.encode_reserved(output),
        };
        let zigzag = ((difference as u128) << 1) ^ ((difference >> 127) as u128);
        put_varint(output, zigzag);
        Ok(())
    }
    pub fn apply(self, kind: Kind, previous: Field<'a>) -> io::Result<Field<'a>> {
        match self {
            Self::Bytes(value) => Ok(Field::Bytes(value)),
            Self::U32List(value) => Ok(Field::U32List(value)),
            Self::U64List(value) => Ok(Field::U64List(value)),
            Self::Opaque(value) => Ok(Field::Opaque(value)),
            Self::Optional(None) => match kind {
                Kind::OptionalI32 => Ok(Field::OptionalI32(None)),
                Kind::OptionalU32 => Ok(Field::OptionalU32(None)),
                Kind::OptionalI64 => Ok(Field::OptionalI64(None)),
                Kind::OptionalU64 => Ok(Field::OptionalU64(None)),
                _ => Err(invalid("optional change applied to nonoptional field")),
            },
            Self::Number(difference) | Self::Optional(Some(difference)) => {
                let value = previous
                    .number()?
                    .checked_add(difference)
                    .ok_or_else(|| invalid("numeric delta addition overflow"))?;
                kind.number(value)
            }
        }
    }
}
#[derive(Clone, Copy)]
#[doc(hidden)]
pub enum Field<'a> {
    Empty,
    NotRecorded,
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bool(bool),
    OptionalI32(Option<i32>),
    OptionalU32(Option<u32>),
    OptionalI64(Option<i64>),
    OptionalU64(Option<u64>),
    Bytes(&'a [u8]),
    U32List(List<'a, u32>),
    U64List(List<'a, u64>),
    Opaque(Opaque<'a>),
}
impl<'a> Field<'a> {
    pub fn recorded_bytes(self) -> io::Result<usize> {
        match self {
            Self::Opaque(Opaque::Native(value)) => allocation::add(4, value.recorded_bytes()?),
            _ => self.encoded_bytes(),
        }
    }
    pub fn encoded_bytes(self) -> io::Result<usize> {
        match self {
            Self::U8(_) | Self::I8(_) | Self::Bool(_) => Ok(1),
            Self::U16(_) | Self::I16(_) => Ok(2),
            Self::U32(_) | Self::I32(_) => Ok(4),
            Self::U64(_) | Self::I64(_) => Ok(8),
            Self::OptionalI32(value) => Ok(1 + usize::from(value.is_some()) * 4),
            Self::OptionalU32(value) => Ok(1 + usize::from(value.is_some()) * 4),
            Self::OptionalI64(value) => Ok(1 + usize::from(value.is_some()) * 8),
            Self::OptionalU64(value) => Ok(1 + usize::from(value.is_some()) * 8),
            _ => self.encoded_bound(),
        }
    }
    pub fn encoded_bound(self) -> io::Result<usize> {
        match self {
            Self::Bytes(bytes) => allocation::add(4, bytes.len()),
            Self::U32List(values) => values.encoded_bytes(),
            Self::U64List(values) => values.encoded_bytes(),
            Self::Opaque(value) => value.encoded_bytes(),
            Self::U8(_) | Self::I8(_) => Ok(2),
            Self::U16(_) | Self::I16(_) => Ok(3),
            Self::U32(_) | Self::I32(_) => Ok(5),
            Self::U64(_) | Self::I64(_) => Ok(10),
            Self::Bool(_) => Ok(1),
            Self::OptionalI32(_) | Self::OptionalU32(_) => Ok(6),
            Self::OptionalI64(_) | Self::OptionalU64(_) => Ok(11),
            Self::Empty | Self::NotRecorded => Err(invalid("missing or not recorded row field")),
        }
    }
    pub fn number(self) -> io::Result<i128> {
        Ok(match self {
            Self::Empty
            | Self::OptionalI32(None)
            | Self::OptionalU32(None)
            | Self::OptionalI64(None)
            | Self::OptionalU64(None) => 0,
            Self::U8(value) => i128::from(value),
            Self::I8(value) => i128::from(value),
            Self::U16(value) => i128::from(value),
            Self::I16(value) => i128::from(value),
            Self::U32(value) => i128::from(value),
            Self::I32(value) => i128::from(value),
            Self::U64(value) => i128::from(value),
            Self::I64(value) => i128::from(value),
            Self::Bool(value) => i128::from(u8::from(value)),
            Self::OptionalI32(Some(value)) => i128::from(value),
            Self::OptionalU32(Some(value)) => i128::from(value),
            Self::OptionalI64(Some(value)) => i128::from(value),
            Self::OptionalU64(Some(value)) => i128::from(value),
            _ => return Err(invalid("list field used as numeric baseline")),
        })
    }
    pub fn change(self, previous: Self) -> io::Result<Change<'a>> {
        match self {
            Self::Bytes(value) => return Ok(Change::Bytes(value)),
            Self::U32List(value) => return Ok(Change::U32List(value)),
            Self::U64List(value) => return Ok(Change::U64List(value)),
            Self::Opaque(value) => return Ok(Change::Opaque(value)),
            Self::OptionalI32(None)
            | Self::OptionalU32(None)
            | Self::OptionalI64(None)
            | Self::OptionalU64(None) => return Ok(Change::Optional(None)),
            _ => {}
        }
        let difference = self
            .number()?
            .checked_sub(previous.number()?)
            .ok_or_else(|| invalid("numeric delta subtraction overflow"))?;
        Ok(
            if matches!(
                self,
                Self::OptionalI32(_)
                    | Self::OptionalU32(_)
                    | Self::OptionalI64(_)
                    | Self::OptionalU64(_)
            ) {
                Change::Optional(Some(difference))
            } else {
                Change::Number(difference)
            },
        )
    }
    pub fn encode_reserved(self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Opaque(value) => value.encode_reserved(output),
            value => value.encode(output),
        }
    }
    pub fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Empty | Self::NotRecorded => {
                return Err(invalid("missing or not recorded row field"));
            }
            Self::U8(value) => <u8 as Value>::encode_reserved(&value, output)?,
            Self::I8(value) => <i8 as Value>::encode_reserved(&value, output)?,
            Self::U16(value) => <u16 as Value>::encode_reserved(&value, output)?,
            Self::I16(value) => <i16 as Value>::encode_reserved(&value, output)?,
            Self::U32(value) => <u32 as Value>::encode_reserved(&value, output)?,
            Self::I32(value) => <i32 as Value>::encode_reserved(&value, output)?,
            Self::U64(value) => <u64 as Value>::encode_reserved(&value, output)?,
            Self::I64(value) => <i64 as Value>::encode_reserved(&value, output)?,
            Self::Bool(value) => <bool as Value>::encode_reserved(&value, output)?,
            Self::OptionalI32(value) => <Option<i32> as Value>::encode_reserved(&value, output)?,
            Self::OptionalU32(value) => <Option<u32> as Value>::encode_reserved(&value, output)?,
            Self::OptionalI64(value) => <Option<i64> as Value>::encode_reserved(&value, output)?,
            Self::OptionalU64(value) => <Option<u64> as Value>::encode_reserved(&value, output)?,
            Self::Bytes(value) => put_bytes(output, value)?,
            Self::U32List(value) => value.encode(output)?,
            Self::U64List(value) => value.encode(output)?,
            Self::Opaque(value) => value.encode(output)?,
        }
        Ok(())
    }
}
pub trait OpaqueValue: Value + crate::checkpoint::values::CheckpointValue {
    const RESTORED: bool = false;
}

#[macro_export]
#[doc(hidden)]
macro_rules! opaque_value {
    ($ty:ty, restored) => {
        $crate::opaque_value!(@impl [] $ty, true);
    };
    ($ty:ty) => {
        $crate::opaque_value!(@impl [] $ty, false);
    };
    (@impl [$($generic:tt)*] $ty:ty, $restored:expr) => {
        $crate::rows::checkpoint::opaque!(@impl [$($generic)*] $ty);
        impl $($generic)* $crate::rows::NativeValue for $ty {
            const HAS_STEPS: bool = <$ty as $crate::value::Value>::HAS_STEPS;
            const RESTORED: bool = $restored;
            fn heap_bytes(&self) -> usize {
                <$ty as $crate::value::Value>::heap_bytes(self)
            }
            fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                $crate::value::Value::visit_field_gaps(self, visit);
            }
            fn round_display(&mut self) {
                $crate::value::Value::round_display(self);
            }
            fn round_to(&mut self, step: $crate::display::Step) {
                $crate::value::Value::round_to(self, step);
            }
            fn to_units(&self, step: $crate::display::Step) -> std::io::Result<Self> {
                $crate::value::Value::to_units(self, step)
            }
            fn from_units(self, step: $crate::display::Step) -> std::io::Result<Self> {
                $crate::value::Value::from_units(self, step)
            }
            fn try_clone(&self) -> std::io::Result<Self> {
                $crate::value::Value::try_clone(self)
            }
            fn field(&self) -> $crate::rows::Field<'_> {
                $crate::rows::Field::Opaque($crate::rows::Opaque::Native(self))
            }
            fn from_field(field: $crate::rows::Field<'_>) -> std::io::Result<Self> {
                let $crate::rows::Field::Opaque($crate::rows::Opaque::Encoded(bytes, format)) = field else {
                    return Err($crate::codec::invalid_static("structured field requires encoded native bytes"));
                };
                $crate::measurement::count($crate::measurement::Counter::NativeOpaqueBytes, bytes.len());
                let mut input = $crate::codec::Decoder::new(bytes);
                input.format = format;
                let value = <$ty as $crate::value::Value>::decode(&mut input)?;
                input.finish()?;
                Ok(value)
            }
        }
    };
}

macro_rules! opaque_wrapper {
    ($element:ident [$($generic:tt)+] $wrapper:ty) => {
        impl <$($generic)+> OpaqueValue for $wrapper {
            const RESTORED: bool = $element::RESTORED;
        }
        opaque_value!(@impl [<$($generic)+>] $wrapper, <Self as OpaqueValue>::RESTORED);
    };
}

opaque_wrapper!(T [T: OpaqueValue] Option<T>);
opaque_wrapper!(T [T: OpaqueValue] Vec<T>);
opaque_wrapper!(T [T: OpaqueValue, const N: usize] [T; N]);
opaque_wrapper!(T [T: OpaqueValue] Witnessed<T>);
opaque_wrapper!(T [T: OpaqueValue] std::sync::Arc<T>);

opaque_value!(crate::event_feed_rows::EventFeedCoverage);
impl OpaqueValue for crate::event_feed_rows::EventFeedCounts {}
impl OpaqueValue for ShipNamingOperands {}
impl OpaqueValue for StateInfrastructure {}
impl OpaqueValue for ConstructionRise {}
impl OpaqueValue for crate::event_feed_rows::EventFeedEvent {}
impl OpaqueValue for crate::event_feed_rows::EventFeedScope {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum List<'a, T> {
    Native(&'a [T]),
    Encoded(&'a [u8]),
}

macro_rules! native_list {
    ($ty:ty, $width:expr) => {
        impl List<'_, $ty> {
            pub fn to_vec(self) -> io::Result<Vec<$ty>> {
                match self {
                    Self::Native(values) => allocation::copy_vec(values),
                    Self::Encoded(bytes) => {
                        let mut values = allocation::vec(bytes.len() / $width)?;
                        for value in bytes.chunks_exact($width) {
                            values.push(<$ty>::from_le_bytes(value.try_into().unwrap()));
                        }
                        Ok(values)
                    }
                }
            }
            pub fn encoded_bytes(self) -> io::Result<usize> {
                allocation::add(
                    4,
                    match self {
                        Self::Native(values) => allocation::mul(values.len(), $width)?,
                        Self::Encoded(bytes) => bytes.len(),
                    },
                )
            }

            pub fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
                let length = match self {
                    Self::Native(values) => values.len(),
                    Self::Encoded(bytes) => bytes.len() / $width,
                };
                let count = u32::try_from(length).map_err(|_| invalid("list field exceeds u32"))?;
                output.extend_from_slice(&count.to_le_bytes());
                match self {
                    Self::Native(values) => {
                        for value in values {
                            output.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    Self::Encoded(bytes) => output.extend_from_slice(bytes),
                }
                Ok(())
            }
        }
    };
}

native_list!(u32, 4);
native_list!(u64, 8);

#[path = "rows/retired.rs"]
mod retired;
pub use retired::{
    Retired, RetiredBool, RetiredI32, RetiredI64, RetiredKept, RetiredNested, RetiredOptionalI64,
    RetiredU8, RetiredU32, RetiredU32List, RetiredU64,
};
#[path = "combat_rows.rs"]
mod combat_rows;
#[path = "extended_rows.rs"]
mod extended_rows;
#[path = "land_rows.rs"]
mod land_rows;
#[path = "naval_mission_rows.rs"]
mod naval_mission_rows;
#[path = "politics_rows.rs"]
mod politics_rows;

#[path = "modifier_breakdown_rows.rs"]
mod modifier_breakdown_rows;
#[path = "production_modifier_rows.rs"]
mod production_modifier_rows;

pub mod ops;
pub use ops::{RowOps, TableRow};

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_rows;
