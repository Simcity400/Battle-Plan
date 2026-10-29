//! Engine-native typed tables and lossless field-mask codecs.
/// Sparse absolute typed rows in a prepared endpoint.
pub use crate::checkpoint::endpoint::EndpointRows;

use crate::codec::{Decoder, invalid_static as invalid, put_bytes, put_varint};
use crate::display::{Rounding, Step};
pub use crate::event_feed_rows::EventFeedHistoryRow;
#[cfg(test)]
use crate::format::FormatVersion;
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
/// Number of registered recording tables, shared by rows and frame consumers.
pub const TABLE_COUNT: usize = TableId::ALL.len();
/// The sampled hour plus the engine's own monotonic allocation counters.
///
/// Every counter is the value the save-header serializer prints for its
/// named key at the sampled tick. Comparing with a save header establishes
/// whether it was written at that tick or after further engine work.
/// `debug_current_ref_id` keeps its printed unsigned width; the rest keep signed i32.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClockRow {
    pub hour: i32,
    pub play_time: u64,
    pub play_session_started_at: i64,
    pub seed: i32,
    /// Stored speed index, clamped to 0..=4 by the setter and saved
    /// unconditionally under token 0x6e. Pause lives on the application object
    /// behind a virtual call and is not recorded.
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
pub struct CampaignRow {
    pub id: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryRow {
    /// Raw country link id, which need not equal its canonical array slot.
    pub id: u32,
    pub canonical_slot: u32,
    pub tag: Vec<u8>,
    pub political_power_raw: Option<i64>,
    pub command_power_raw: i64,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub convoys_raw: Option<i64>,
    /// Unit counters hide while the viewing country's stored hide flag is set.
    pub peace_conference_participant: u8,
    /// Stored signed residual manpower.
    pub manpower_residual: i32,
    /// Stored signed land-doctrine level. Hourly and present only for province
    /// controllers reached by active border wars; None means outside that
    /// hour's demand, never a failed read.
    pub border_land_doctrine_level: Recorded<Option<i32>>,
}

#[cfg(test)]
#[path = "rows/border/tests.rs"]
mod border_tests;
#[cfg(test)]
#[path = "rows/validated_apply_tests.rs"]
mod validated_apply_tests;

#[cfg(test)]
thread_local! {
    /// Rows whose keys this thread validated.
    static VALIDATED_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryLinkRow {
    /// Raw country reference used by provinces and other engine tables.
    pub id: u32,
    pub canonical_slot: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProvinceRow {
    pub id: u32,
    pub state: u32,
    /// Raw country link id; zero is retained as read from the engine.
    pub owner: u32,
    /// Raw country link id; resolve through CountryLinks to a canonical slot.
    pub controller: u32,
    /// Stored signed, unscaled province value.
    pub victory_points: i32,
    /// Hourly: nonzero when the native fort sum over this province's buildings
    /// is positive, which is the only thing the movement graph asks of it.
    pub fort_positive: u8,
    /// Hourly province-movement access gates, the `MOVE_*` bits.
    pub move_gate: u8,
    /// The id of the sea province this land province's port opens onto, used
    /// by path-edge port classification; zero without a port. Static: one delta
    /// on the first hour.
    pub port_sea: i32,
    /// The selected-attack fort sum and state factor recordings through
    /// format 101 captured ([`ProvinceAttackInputs`]); current writers leave
    /// it empty ([`Witnessed`]).
    pub attack: Recorded<Witnessed<ProvinceAttackInputs>>,
    /// Retired controller-area graph operands of the selected-route preview.
    pub selected_areas: Recorded<Retired>,
    /// For a province whose healthy land fort level (the walk's fort sum) is positive and
    /// whose controller is positive, its state's effective
    /// `land_bunker_effectiveness_factor` for that controller added to the controller's
    /// own factor by the fort-base calculation. None for every other province, which no
    /// attack list asks for it, and for a fortified one's read refusal. Older formats use
    /// NotRecorded.
    pub fort_bunker: Recorded<Option<i64>>,
    /// The province's controller area (compared and searched during route finding), as the id of
    /// its smallest member province, whose row holds its [`Self::area_node`]; zero for a null area.
    /// None is a capture refusal. The collector reads an area again only when a controller change
    /// can have touched it (`engine_native::controller_areas`).
    pub controller_area: Recorded<Option<u32>>,
    /// The province's owner area labelled the same way; the route search reads only equality and
    /// null.
    pub owner_area: Recorded<Option<u32>>,
    /// On the province whose id labels its controller area, that area's node;
    /// None on every other province. On the label province None refuses the
    /// node.
    pub area_node: Recorded<Option<ControllerAreaNode>>,
}

impl ProvinceRow {
    /// A land, non-lake province (terrain flags bit 0 set, bit 1 clear) whose
    /// A null owner area refuses province movement access.
    pub const MOVE_NULL_OWNER_AREA: u8 = 1;
    /// The size of the province's border-conflict set is nonzero. Province movement
    /// access then refuses entry unless the army's own location has this bit or the
    /// province's BorderConflicts row lists the army's orders group.
    pub const MOVE_BORDER_CONFLICT: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateRow {
    pub id: u32,
    /// Raw country link id; zero is retained as read from the engine.
    pub owner: u32,
    /// Raw country link id; resolve through CountryLinks to a canonical slot.
    pub controller: u32,
    /// Hourly native `cstate::movement_infrastructure` result, fixed5. Not
    /// derivable from the staggered StateBuildings table.
    pub movement_infrastructure: i64,
    /// Stored signed available manpower.
    pub manpower_available: i32,
    /// Stored state pool population unavailable for recruitment.
    pub manpower_locked: Recorded<i32>,
    /// Exact localised bytes set by `set_state_name`, empty when absent;
    /// the display name prefers this rename.
    pub custom_name: Vec<u8>,
    /// Exact owner-qualified name bytes set on every owner change, empty
    /// when the owner has no such key.
    pub owner_name: Vec<u8>,
    /// Retained continent lexer token, zero for a null continent; None is an
    /// independent capture failure.
    pub continent: Recorded<Option<u32>>,
    /// Exact hourly healthy/MAX infrastructure fraction used by railway ETA,
    /// independent of the additive normal-movement term. None is a counted
    /// read refusal; Some(0) retains native disabled or absent infrastructure.
    pub railway_infrastructure: Recorded<Option<i64>>,
    /// Stored state population, read every hour beside `manpower_available`
    /// and saved as the manpower pool's `total`: the value the
    /// `state_population` triggers compare and the manpower map mode colours
    /// each state by. Available manpower is not population.
    pub population: Recorded<i32>,
    /// Cached index into the native state province list, in recorded-content order.
    pub best_vp_index: Recorded<Option<i32>>,
    /// Whether the cached controller grouping takes the whole-state branch.
    /// None is a capture refusal; false groups positive province controllers.
    pub single_controller: Recorded<Option<u32>>,
    /// The token of the state's live category, read every hour beside
    /// `population` and saved as `state_category`: what the states map mode
    /// colours each state by and whose `buildings_max_level` limits building
    /// levels. None is a counted read refusal.
    pub category: Recorded<Option<u32>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionRow {
    /// Full native handle: type in the low 32 bits, id in the high 32 bits.
    pub id: u64,
    /// Raw country reference of the roster containing this division.
    pub roster_owner: u32,
    /// Raw unit owner reference.
    pub logical_owner: u32,
    /// Raw expeditionary-owner reference, including the engine's null value.
    pub expeditionary_owner: u32,
    pub province: u32,
    /// Full native standard-template handle, with the same type/id packing as id.
    pub template: u64,
    /// Native signed fixed-point hit points, with denominator 100,000.
    pub hit_points: i64,
    /// Native signed fixed-point organisation, with denominator 100,000.
    pub organisation: i64,
    /// Full orders group handle of the army holding this division, or zero for the
    /// engine's own null parent. It is an independent second source for
    /// OrdersGroupMembers, never a substitute for that ordered vector.
    pub orders_group: u64,
    /// The standard template's native fog reveal-skip byte (zero for none).
    pub reveal_skip: u8,
}

impl DivisionRow {
    /// `hit_points` at the game's display precision ([`crate::display`]). The game shows
    /// a division's strength only as a share of its maximum (whole percentages in combat,
    /// army-average and current-strength text) and its maximum at one highlighted
    /// decimal, so hit points keep that tenth, truncated as the fixed5 formatter does. A
    /// nonzero value below a tenth keeps one tenth with its sign, so the zero and sign
    /// tests consumers apply (`hp >= 0`, the retreat sum's `<= 0`) read the same. By
    /// owner decision 2026-10-11 the derived displays (strength percentages and bars,
    /// army totals, border war power, the retreat-admission sum and the move-strength
    /// warning) read the stepped value and can differ from the game in their last digit
    /// at boundaries.
    pub const HIT_POINTS_STEP: Step = Step::fixed5(10_000, Rounding::TowardZeroNonzero);
    /// `organisation` at the game's finest display of it: the combat window prints
    /// current organisation with one decimal truncated toward zero by the fixed5
    /// formatter; the counter hover's whole units in a fixed colour and every installed
    /// mod's override are coarser. Damage application clamps it at zero. By owner
    /// decision 2026-10-11 the derived displays (land combat progress and the marker's
    /// whole-unit progress text, the combat heading frame, average organisation, bars,
    /// border war power and the move-organisation warning) read the stepped value and can
    /// differ from the game in their last digit at boundaries.
    pub const ORGANISATION_STEP: Step = Step::fixed5(10_000, Rounding::TowardZero);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WarRelationRow {
    /// Ordered country identities form the row key; this is not a full war id.
    pub first_country: u32,
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    /// Raw engine byte, retained without boolean normalization.
    pub cancel: u8,
    pub first_casualties: i64,
    pub second_casualties: i64,
    pub first_unknown_casualties: i64,
    pub second_unknown_casualties: i64,
    /// Raw engine byte, retained without boolean normalization.
    pub first_was_instigator: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionRow {
    pub id: u64,
    /// Literal engine name bytes; an empty name is valid.
    pub name: Vec<u8>,
    /// The faction template's lexer token (its spelling is in LexicalTokens), the
    /// save's `template`; None when the faction has no template. The game shows the
    /// template's localised name when the literal name is empty.
    /// That localised name is not recorded; resolve it through the content's
    /// faction templates.
    pub template_token: Option<u32>,
    /// The faction colour, the save's `color`: the native `colour` components as raw f32
    /// bits. The save writes each of red, green and blue as `(int)(c * 255.0f)` and alpha
    /// only when it is not 1.0.
    pub color_red: u32,
    pub color_green: u32,
    pub color_blue: u32,
    pub color_alpha: u32,
    /// Raw engine byte, the save's `is_color_overridden`: zero when the colour was
    /// taken from the leader's country colour, retained without boolean normalization.
    pub color_overridden: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionMemberRow {
    /// Raw country identity; a country has at most one faction membership.
    pub country: u32,
    pub faction: u64,
    /// Recorded member order; ordinal zero identifies the first member.
    pub ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PlayerRow {
    /// Native signed session id; its raw u32 bits form the row key.
    pub id: i32,
    pub country: u32,
    /// Raw byte, including bits other than the save's country-leader bit.
    pub flags: u8,
    pub user: Vec<u8>,
    pub name: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HumanControllerRow {
    /// Raw country link id, independently authenticated from the player roster.
    pub country: u32,
    /// Native signed byte; neither zero nor negative values are normalized.
    pub count: i8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatRow {
    pub id: u64,
    pub province: u32,
    pub duration: i32,
    pub day: i32,
    /// Ordered native unit handles, including the complete type word.
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
    /// Each attacker's native unit class, in `attackers` order: 0 is a
    /// division, 1 a task force, 13 a railway gun.
    pub attacker_classes: Vec<u32>,
    /// Each defender's native unit class, in `defenders` order.
    pub defender_classes: Vec<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FleetRow {
    pub id: u64,
    pub owner: u32,
    pub home_base: u32,
    /// Stored fleet-name bytes; ordinary serialization writes even empty.
    pub name: Recorded<Option<Vec<u8>>>,
    /// Full fleet-leader reference, distinct from a mission commander.
    pub leader: Recorded<Option<u64>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaskForceRow {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub mission: u32,
    /// Signed task-force route length. An enemy fleet in a strait's sea blocks
    /// only while this is below 1; saved as the length of the task force's `path`.
    pub route_count: i32,
    /// Stored combat-reference count; combat presence tests it as positive.
    pub combat_references: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShipRow {
    /// Exact retained display cache; None is an independent acquisition refusal.
    pub name: Recorded<Option<Vec<u8>>>,
    /// Raw saved naming state, independent of the unsaved display cache.
    pub naming: Recorded<Option<ShipNamingOperands>>,
    pub required_manpower: Recorded<Option<i32>>,
    pub id: u64,
    pub task_force: u64,
    pub strength: i64,
    pub organisation: i64,
    /// The ship's experience, signed fixed5, at [`Self::EXPERIENCE_STEP`]. Its run-time writers are
    /// experience awards (accepted fixed5 addition, then the ship's experience cap), assignment
    /// during transfer and captain replacement, and damage scaling. Construction zeroes it and
    /// loading floors a negative saved value at zero. Save serialization writes all eight bytes.
    pub experience: i64,
    pub category: u32,
    pub sunk_convoys: i32,
    pub class_id: i32,
    pub acquired_manpower: Recorded<Option<i32>>,
}

/// Ship name-member serialization operands, not a generated display name.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShipNamingOperands {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub override_name: Vec<u8>,
    pub programmatic: u8,
    /// Full retained naming-variant identity; None is a known null pointer.
    pub equipment: Option<u64>,
}
crate::value::value_struct!(ShipNamingOperands {
    kind: i32, order: i32, ordered: u8, override_name: Vec<u8>,
    programmatic: u8, equipment: Option<u64>,
});

impl ShipRow {
    /// `experience` at the game's finest display of it ([`crate::display`]). The game
    /// shows a ship's experience only through its ratio to the maximum, flooring
    /// experience at zero and truncating the fixed5 ratio: the level counts the
    /// `NMilitary.UNIT_EXP_LEVELS` thresholds the ratio reaches (`>=`), and the tooltip
    /// prints progress through the current level as a whole percentage in a fixed colour,
    /// truncated by the fixed5 formatter. One percent of the narrowest level of any
    /// installed mod (UNIT_EXP_LEVELS steps of 0.02 at NNavy.NAVY_MAX_XP 100) is 0.02,
    /// and of vanilla's 0.1, so a hundredth truncated toward zero is never coarser than
    /// the game. Every installed threshold times NAVY_MAX_XP is a whole multiple of the
    /// step, so the level read from the stepped value is the game's; by owner decision
    /// 2026-10-11 the progress percent can differ from the game's in its last digit at
    /// boundaries.
    pub const EXPERIENCE_STEP: Step = Step::fixed5(1_000, Rounding::TowardZero);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatRow {
    pub id: u64,
    pub province: u32,
    pub attacker_positioning: i64,
    pub defender_positioning: i64,
    pub attacker_anti_air: i64,
    pub defender_anti_air: i64,
    /// Index in the mixed native active-combat registry, before naval filtering; it
    /// orders the spillover pass of the naval combat map icons.
    pub registry_ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatParticipantRow {
    /// All 64 handle bits, the side bit, and all 32 slot bits form the 97-bit key.
    pub combat: u64,
    /// Attacker zero, defender one; this selects the native participant vector.
    pub side: u8,
    pub battle_slot: u32,
    /// Full native ship handle; zero is retained for convoy entries.
    pub ship: u64,
    pub owner: u32,
    pub state: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// Stored raid range-limit operand.
    pub base_range_raw: i64,
    /// The assigned ace's full reference.
    pub ace: u64,
    /// Scenario-eligibility operands: saved mission active byte, veto byte,
    /// and the counter and its limit.
    pub mission_active: u8,
    pub eligibility_veto: u8,
    pub eligibility_counter: i32,
    pub eligibility_limit: i32,
    /// Attached orders group's authenticated full handle, `Some(0)` when
    /// null; `None` when the same-pass group identity is unavailable.
    pub attached_orders_group: Option<u64>,
    /// The wing's position in its owner's pools as stored: pools in order,
    /// each pool's wings in order, from zero for each owner. The training pass
    /// admits its first 50 candidates in this order, and the save lists wings in it.
    pub pool_ordinal: u32,
    /// Stored name bytes. No generator operands survive; display only in the
    /// recording language. `None` is a guarded-read/arena refusal, `Some(empty)`
    /// is an observed empty name. Older recordings are absent. Ordinary text
    /// saves write `name` even when empty; only checksum mode omits it.
    pub name: Recorded<Option<Vec<u8>>>,
    /// Containing pool's authenticated subunit definition database index, joined to the
    /// recorded content stack. Some(0) is the null subtype; None is an optional
    /// acquisition refusal, older formats NotRecorded.
    pub pool_subunit: Recorded<Option<u32>>,
    /// Stored full paired raid reference and certified liveness for pool-backed
    /// wings. Never derive liveness from the raid roster.
    pub raid_reference: Recorded<Option<AirWingRaidReference>>,
    /// Selected fixed5 stat 0x32, mission 0x8; no graphical choice. None means
    /// another mission or an optional read refusal; older formats use
    /// NotRecorded, while Some(0) is an observed zero.
    pub strategic_bombing_raw: Recorded<Option<i64>>,
    /// Independent writer-certified saved reinforcement preference byte.
    /// None is unavailable admission/read; every raw byte remains representable.
    pub replacement_setting: Recorded<Option<u8>>,
    /// Bit 0/1: training-stop known/value; bit 2/3: attachment coverage
    /// known/value; bit 4/5: raid-assignment liveness known/value.
    /// Unreached fields and missing certificates remain unknown.
    pub map_flags: Recorded<Option<u32>>,
    /// Independent numeric columns keep a changing execution count from
    /// repeating the quiet cached statistic, context and training operands.
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
    /// Raw registry liveness for the reached training override gate.
    pub override_resolved: Option<bool>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyOrderRow {
    /// All 64 owner handle bits and all 32 instance bits form the 96-bit row key.
    pub owner_group: u64,
    pub instance: u32,
    pub country: u32,
    pub order_type: i32,
    pub active: u8,
    /// Native ordered geometry; no derived path replaces these entries.
    pub path: Vec<u32>,
    pub target_states: Vec<u32>,
    pub paradrop_target: u32,
}

pub trait RowCodec: Default + Clone + Eq {
    fn try_clone(&self) -> io::Result<Self>;
    /// Copy admitted rows without repeating their key/nested validation.
    fn try_clone_validated(&self) -> io::Result<Self>;
    /// Bytes this row's strings, lists and nested values hold beyond the row.
    fn heap_bytes(&self) -> usize;
    fn encoded_bound(&self) -> io::Result<usize>;
    fn snapshot_bytes(&self) -> io::Result<usize>;
    const KEY_BITS: u32;
    fn key(&self) -> u128;
    fn check_key_fields(&self) -> io::Result<()>;
    fn not_recorded_fields(&self) -> u64;
    fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str));
    /// Each schema field in order as the wire holds it: stepped fields in their step's
    /// units, a field this row did not record as [`Field::NotRecorded`], a restored
    /// field's tombstone as its empty payload.
    fn visit_wire(
        &self,
        visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
    ) -> io::Result<()>;
    /// The row keyed `key` whose schema fields `wire` hands by index, as [`Self::visit_wire`]
    /// visits them: step units become native units again, as [`Self::decode_fields`] reads them.
    fn from_wire<'a>(
        key: u128,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Self>;
    /// Sets the schema fields `mask` names from `wire`, as [`Self::from_wire`] reads them.
    /// False leaves the row as it was, for its caller to build it whole instead.
    fn patch_from_wire<'a>(
        &mut self,
        mask: u64,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<bool> {
        let _ = (mask, wire);
        Ok(false)
    }
    fn recorded_fields_bytes(&self) -> io::Result<usize>;
    fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>>;
    /// Encode one changed row directly, including its key difference and mask.
    /// Returns false without writing when no field changed.
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
    /// The fields [`Self::encode_fields`] writes, but those this row did not record, which
    /// it skips; and a bit per skipped field.
    fn encode_recorded_fields(&self, output: &mut Vec<u8>) -> io::Result<u64> {
        self.encode_fields(output)?;
        Ok(0)
    }
    fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self>;
    /// Takes ownership so an unchanged string or list keeps its existing allocation.
    fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self>;
    /// Whether a field of this row, or a part nested in one, has a display
    /// step ([`crate::display`]).
    fn has_display_steps(&self) -> bool;
    /// Round every stepped field to its display step, in place.
    fn round_to_display(&mut self);
}

#[doc(hidden)]
pub trait Fields {
    const SCHEMA: &'static [Kind];
    /// Each field's display step ([`crate::display`]), in schema order.
    const STEPS: &'static [Option<Step>];
    /// Each field's row member, in schema order.
    const NAMES: &'static [&'static str];
}

pub trait TableSchema {
    type Row: RowCodec;
    const ID: TableId;
    fn borrow(rows: &TableRows) -> Option<&[Self::Row]>;
    fn borrow_mut(rows: &mut TableRows) -> Option<&mut Vec<Self::Row>>;
    fn take(rows: TableRows) -> Result<Vec<Self::Row>, TableRows>;
    fn wrap(rows: Vec<Self::Row>) -> TableRows;
}

pub mod compatibility;
pub mod restricted_changes;

#[path = "rows/checkpoint.rs"]
pub(crate) mod checkpoint;

/// Playback absence is distinct from every native value, including None/empty.
/// Current producers default to a recorded native default; old decoders set
/// NotRecorded explicitly. No absence tag is written to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

macro_rules! define_tables {
    ($($table:ident = $id:literal => $variant:ident($row:ident)),+ $(,)?) => {
        $crate::checkpoint::rows::tables! { $($table => $row),+ }
        $crate::checkpoint::endpoint::tables! { $($table => $row),+ }
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum TableId { $($table = $id),+ }
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum TableRows { $($table(Vec<$row>)),+ }
        /// Inspection borrows concrete rows; decoded storage never owns this enum.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum RowRef<'a> { $($variant(&'a $row)),+ }
        pub enum RowIter<'a> { $($table(std::slice::Iter<'a, $row>)),+ }
        pub mod tables { $(pub struct $table;)+ }
        $(impl TableSchema for tables::$table {
            type Row = $row;
            const ID: TableId = TableId::$table;
            fn borrow(rows: &TableRows) -> Option<&[Self::Row]> {
                match rows { TableRows::$table(rows) => Some(rows), _ => None }
            }
            fn borrow_mut(rows: &mut TableRows) -> Option<&mut Vec<Self::Row>> {
                match rows { TableRows::$table(rows) => Some(rows), _ => None }
            }
            fn take(rows: TableRows) -> Result<Vec<Self::Row>, TableRows> {
                match rows { TableRows::$table(rows) => Ok(rows), rows => Err(rows) }
            }
            fn wrap(rows: Vec<Self::Row>) -> TableRows { TableRows::$table(rows) }
        })+
        impl TableId {
            pub const ALL: [Self; [$(stringify!($table)),+].len()] = [$(Self::$table),+];
            /// Dense storage position, independent of wire IDs.
            pub const fn index(self) -> usize {
                enum Index { $($table),+ }
                match self { $(Self::$table => Index::$table as usize),+ }
            }
            pub const fn schema(self) -> &'static [Kind] {
                match self { $(Self::$table => <$row as Fields>::SCHEMA),+ }
            }
            pub const fn key_bits(self) -> u32 {
                match self { $(Self::$table => <$row as RowCodec>::KEY_BITS),+ }
            }
            /// The display step ([`crate::display`]) of top-level field
            /// `field`; nested steps belong to their value declarations.
            pub fn display_step(self, field: usize) -> Option<Step> {
                let steps: &[Option<Step>] = match self { $(Self::$table => <$row as Fields>::STEPS),+ };
                steps.get(field).copied().flatten()
            }
            /// The row member field `field` is stored from, as its codec names it.
            pub fn field_name(self, field: usize) -> Option<&'static str> {
                let names: &[&str] = match self { $(Self::$table => <$row as Fields>::NAMES),+ };
                names.get(field).copied()
            }
            pub fn decode_rows(self, count: usize, input: &mut Decoder<'_>) -> io::Result<TableRows> {
                Ok(match self { $(Self::$table => TableRows::$table(decode_typed::<$row>(count, input)?)),+ })
            }
            /// This table's rows at `keys`, in their order, each field as `wire(row, field)` hands
            /// it in its wire form ([`RowCodec::from_wire`]).
            pub fn rows_from_wire<'a>(
                self,
                keys: &[u128],
                wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
            ) -> io::Result<TableRows> {
                Ok(match self { $(Self::$table => TableRows::$table(rows_from_wire_typed::<$row>(keys, wire)?)),+ })
            }
        }
        impl TableRows {
            pub(crate) fn range_update_fits(&self, previous: &TableRows, ranges: &[std::ops::Range<u128>]) -> bool {
                match (self, previous) { $((Self::$table(fresh), Self::$table(old)) => crate::sliced::ranges::fits(old, fresh, ranges)),+, _ => false }
            }
            pub(crate) fn encode_range_update(&self, previous: &TableRows, ranges: &[std::ops::Range<u128>], output: &mut Vec<u8>, deleted: &mut Vec<u8>) -> io::Result<bool> {
                match (self, previous) { $((Self::$table(fresh), Self::$table(old)) => crate::sliced::ranges::encode(old, fresh, ranges, output, deleted)),+, _ => Err(invalid("key range table type mismatch")) }
            }
            pub(crate) fn apply_range_update(self, previous: &mut TableRows, ranges: &[std::ops::Range<u128>]) -> io::Result<TableRows> {
                match (self, previous) { $((Self::$table(fresh), Self::$table(old)) => crate::sliced::ranges::apply(old, fresh, ranges).map(Self::$table)),+, _ => Err(invalid("key range table type mismatch")) }
            }
            pub fn inline_bytes(&self) -> usize {
                match self { $(Self::$table(rows) => rows.len() * std::mem::size_of::<$row>()),+ }
            }
            /// Bytes of the row allocation: its capacity at this table's own
            /// row size. Strings and lists a row owns on the heap are not counted.
            pub fn allocated_bytes(&self) -> usize {
                match self { $(Self::$table(rows) => rows.capacity() * std::mem::size_of::<$row>()),+ }
            }
            /// Bytes the rows keep alive: their allocation and what each
            /// row's strings, lists and nested values hold in turn.
            pub fn heap_bytes(&self) -> usize {
                match self { $(Self::$table(rows) => rows.capacity() * std::mem::size_of::<$row>()
                    + rows.iter().map($crate::rows::RowCodec::heap_bytes).sum::<usize>()),+ }
            }
            pub fn truncate(&mut self, len: usize) {
                match self { $(Self::$table(rows) => rows.truncate(len)),+ }
            }
            /// Round every row's stepped fields to their display steps
            /// ([`crate::display`]), as capture stores them; a table without
            /// steps is left untouched.
            pub fn round_to_display(&mut self) {
                match self { $(Self::$table(rows) => round_rows_to_display(rows)),+ }
            }
            pub fn from_rows<T: TableSchema>(rows: Vec<T::Row>) -> Self { T::wrap(rows) }
            pub fn empty(table: TableId) -> Self {
                match table { $(TableId::$table => Self::$table(Vec::new())),+ }
            }
            /// One default row of `table`: the shape a complete singleton table
            /// (`key_bits() == 0`) must have, for fixtures that complete every
            /// singleton from the registry rather than by name.
            #[cfg(any(test, feature = "test-support"))]
            pub fn one_default(table: TableId) -> Self {
                match table { $(TableId::$table => Self::$table(vec![Default::default()])),+ }
            }
            pub const fn table(&self) -> TableId {
                match self { $(Self::$table(_) => TableId::$table),+ }
            }
            pub fn len(&self) -> usize {
                match self { $(Self::$table(rows) => rows.len()),+ }
            }
            pub fn is_empty(&self) -> bool { self.len() == 0 }
            pub fn rows<T: TableSchema>(&self) -> Option<&[T::Row]> { T::borrow(self) }
            pub fn rows_mut<T: TableSchema>(&mut self) -> Option<&mut Vec<T::Row>> { T::borrow_mut(self) }
            pub fn iter(&self) -> RowIter<'_> {
                match self { $(Self::$table(rows) => RowIter::$table(rows.iter())),+ }
            }
            pub fn get(&self, index: usize) -> Option<RowRef<'_>> { self.iter().nth(index) }
            pub fn contains_key(&self, key: u128) -> bool {
                match self { $(Self::$table(rows) => rows.binary_search_by_key(&key, |row| row.key()).is_ok()),+ }
            }

            pub fn first(&self) -> Option<RowRef<'_>> { self.iter().next() }
            pub fn validate_keys(&self) -> io::Result<()> {
                match self { $(Self::$table(rows) => validate_typed(rows)),+ }
            }
            pub fn try_clone(&self) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => {
                    let mut owned = allocation::vec(rows.len())?;
                    for row in rows { owned.push(RowCodec::try_clone(row)?); }
                    Self::$table(owned)
                }),+ })
            }
            pub fn try_clone_validated(&self) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => {
                    let mut owned = allocation::vec(rows.len())?;
                    for row in rows { owned.push(RowCodec::try_clone_validated(row)?); }
                    Self::$table(owned)
                }),+ })
            }
            pub fn recorded_rows_bytes(&self) -> io::Result<usize> {
                self.validate_keys()?;
                match self { $(Self::$table(rows) => {
                    let mut bytes = 4;
                    let mut previous = 0;
                    for row in rows {
                        bytes = allocation::add(bytes, varint_bytes(row.key() - previous))?;
                        bytes = allocation::add(bytes, row.recorded_fields_bytes()?)?;
                        previous = row.key();
                    }
                    Ok(bytes)
                }),+ }
            }
            pub fn encode_rows(&self, output: &mut Vec<u8>) -> io::Result<()> {
                match self { $(Self::$table(rows) => encode_typed(rows, output)),+ }
            }
            /// One row's fields in schema order, without its key.
            pub fn encode_row_fields(&self, index: usize, output: &mut Vec<u8>) -> io::Result<()> {
                match self { $(Self::$table(rows) => rows.get(index).ok_or_else(|| invalid("row index out of range"))?.encode_fields(output)),+ }
            }
            /// One row's recorded fields in schema order, without its key, and a bit per
            /// field it did not record (skipped).
            pub fn encode_row_recorded_fields(&self, index: usize, output: &mut Vec<u8>) -> io::Result<u64> {
                match self { $(Self::$table(rows) => rows.get(index).ok_or_else(|| invalid("row index out of range"))?.encode_recorded_fields(output)),+ }
            }
            pub fn encode_changes(&self, previous: &Self, output: &mut Vec<u8>, deleted: &mut Vec<u8>) -> io::Result<bool> {
                match (self, previous) {
                    $((Self::$table(rows), Self::$table(previous)) => encode_changes_typed(rows, previous, output, deleted)),+,
                    _ => Err(invalid("table rows have different types")),
                }
            }
            pub fn encode_replacements(&self, previous: &Self, indices: &[usize], output: &mut Vec<u8>) -> io::Result<bool> {
                match (self, previous) {
                    $((Self::$table(rows), Self::$table(previous)) => encode_replacements_typed(rows, previous, indices, output)),+,
                    _ => Err(invalid("replacement rows have different types")),
                }
            }
            /// Indices, types and unchanged keys were checked while encoding,
            /// before any table in the frame could change its baseline.
            /// Returns the replaced rows, in the replacement's storage.
            pub fn install_replacements(&mut self, mut replacement: Self, indices: &[usize]) -> Self {
                match (self, &mut replacement) {
                    $((Self::$table(previous), Self::$table(rows)) => {
                        for (&index, row) in indices.iter().zip(rows.iter_mut()) {
                            std::mem::swap(&mut previous[index], row);
                        }
                    }),+,
                    _ => unreachable!("validated replacement type"),
                }
                replacement
            }
            pub fn apply_delta(self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table(apply_typed(rows, changed, deleted)?)),+ })
            }
            /// [`Self::apply_delta`] to rows whose keys are valid, then
            /// [`Self::validate_keys`] of the result.
            pub fn apply_delta_validated(self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table(apply_typed_validated(rows, changed, deleted)?)),+ })
            }
            /// [`Self::apply_delta`] for a baseline that stays shared.
            pub fn apply_delta_shared(&self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table(apply_typed_shared(rows, changed, deleted)?)),+ })
            }
            /// [`Self::apply_delta_validated`] for a baseline that stays shared.
            pub fn apply_delta_shared_validated(&self, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<Self> {
                Ok(match self { $(Self::$table(rows) => Self::$table(apply_typed_shared_validated(rows, changed, deleted)?)),+ })
            }
            /// What `self` changed since `previous`, both valid rows of one
            /// table: each row it holds that `previous` does not hold equal at
            /// its key, copied, and each key of `previous` it lacks. None when
            /// they hold different tables.
            pub fn changes_since(&self, previous: &Self) -> io::Result<Option<RowChanges>> {
                // One arm per table, not per pair of tables: pair matches are costly to compile.
                Ok(match self { $(Self::$table(rows) => match <tables::$table as TableSchema>::borrow(previous) {
                    Some(previous) => {
                        let (changed, deleted) = changes_typed(rows, previous)?;
                        Some(RowChanges { rows: Self::$table(changed), deleted })
                    }
                    None => None,
                }),+ })
            }
            /// The rows [`Self::changes_since`] found `changes` from: each of
            /// its rows set at its key and each of its deleted keys removed.
            pub fn apply_changes(self, changes: RowChanges) -> io::Result<Self> {
                let RowChanges { rows: changed, deleted } = changes;
                match self { $(Self::$table(rows) => {
                    let changed = <tables::$table as TableSchema>::take(changed)
                        .map_err(|_| invalid("row changes of another table"))?;
                    Ok(Self::$table(apply_changes_typed(rows, changed, &deleted)?))
                }),+ }
            }
            /// Each row keyed as `patches` names, in key order, with the fields its mask names
            /// set as `wire(patch, field)` hands them ([`RowCodec::patch_from_wire`]); a row
            /// that cannot be patched is built whole from every field.
            pub fn patch_from_wire<'a>(
                &mut self,
                patches: &[(u128, u64)],
                wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
            ) -> io::Result<()> {
                match self { $(Self::$table(rows) => patch_typed(rows, patches, wire)),+ }
            }
        }
        impl RowRef<'_> {
            pub fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                match self { $(Self::$variant(row) => row.visit_nested_field_gaps(visit)),+ }
            }
            pub fn visit_wire(
                &self,
                visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
            ) -> io::Result<()> {
                match self { $(Self::$variant(row) => row.visit_wire(visit)),+ }
            }
            pub fn not_recorded_fields(&self) -> u64 {
                match self { $(Self::$variant(row) => row.not_recorded_fields()),+ }
            }
            pub const fn table(&self) -> TableId {
                match self { $(Self::$variant(_) => TableId::$table),+ }
            }
            pub fn key(&self) -> u128 {
                match self { $(Self::$variant(row) => row.key()),+ }
            }
        }
        impl<'a> Iterator for RowIter<'a> {
            type Item = RowRef<'a>;
            fn next(&mut self) -> Option<Self::Item> {
                match self { $(Self::$table(rows) => rows.next().map(RowRef::$variant)),+ }
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = self.len(); (len, Some(len))
            }
            fn nth(&mut self, index: usize) -> Option<Self::Item> {
                match self { $(Self::$table(rows) => rows.nth(index).map(RowRef::$variant)),+ }
            }
        }
        impl ExactSizeIterator for RowIter<'_> {
            fn len(&self) -> usize { match self { $(Self::$table(rows) => rows.len()),+ } }
        }
        impl<'a> IntoIterator for &'a TableRows {
            type Item = RowRef<'a>;
            type IntoIter = RowIter<'a>;
            fn into_iter(self) -> Self::IntoIter { self.iter() }
        }
    };
}
define_tables! {
Clock = 0 => Clock(ClockRow),
Campaign = 1 => Campaign(CampaignRow),
Countries = 2 => Country(CountryRow),
CountryLinks = 3 => CountryLink(CountryLinkRow),
Provinces = 4 => Province(ProvinceRow),
States = 5 => State(StateRow),
Divisions = 6 => Division(DivisionRow),
WarRelations = 7 => WarRelation(WarRelationRow),
Factions = 8 => Faction(FactionRow),
FactionMembers = 9 => FactionMember(FactionMemberRow),
Players = 10 => Player(PlayerRow),
HumanControllers = 11 => HumanController(HumanControllerRow),
LandCombats = 12 => LandCombat(LandCombatRow),
Fleets = 13 => Fleet(FleetRow),
TaskForces = 14 => TaskForce(TaskForceRow),
Ships = 15 => Ship(ShipRow),
NavalCombats = 16 => NavalCombat(NavalCombatRow),
NavalCombatParticipants = 17 => NavalCombatParticipant(NavalCombatParticipantRow),
AirWings = 18 => AirWing(AirWingRow),
ArmyOrders = 19 => ArmyOrder(ArmyOrderRow),
Focus = 20 => Focus(FocusRow),
CompletedFocuses = 21 => CompletedFocus(CompletedFocusRow),
ResearchTechnologies = 22 => ResearchTechnology(ResearchTechnologyRow),
ResearchSlots = 23 => ResearchSlot(ResearchSlotRow),
TechnologyDefinitions = 24 => TechnologyDefinition(TechnologyDefinitionRow),
Politics = 25 => Politics(PoliticsRow),
PoliticalParties = 26 => PoliticalParty(PoliticalPartyRow),
ActiveIdeas = 27 => ActiveIdea(ActiveIdeaRow),
DecisionCounts = 28 => DecisionCounts(DecisionCountsRow),
Decisions = 29 => Decision(DecisionRow),
DynamicModifiers = 30 => DynamicModifier(DynamicModifierRow),
PreferredTactics = 31 => PreferredTactic(PreferredTacticRow),
CountryTacticUnlocks = 32 => CountryTacticUnlock(CountryTacticUnlockRow),
TechSharingGroups = 33 => TechSharingGroup(TechSharingGroupRow),
TechSharingMembers = 34 => TechSharingMember(TechSharingMemberRow),
PowerBalances = 35 => PowerBalance(PowerBalanceRow),
PowerBalanceMembers = 36 => PowerBalanceMember(PowerBalanceMemberRow),
PowerBalanceRanges = 37 => PowerBalanceRange(PowerBalanceRangeRow),
PowerBalanceOverrides = 38 => PowerBalanceOverride(PowerBalanceOverrideRow),
ProductionLines = 39 => ProductionLine(ProductionLineRow),
Stockpiles = 40 => Stockpile(StockpileRow),
DivisionEquipment = 41 => DivisionEquipment(DivisionEquipmentRow),
VariantDefinitions = 42 => VariantDefinition(VariantDefinitionRow),
VariantModules = 43 => VariantModule(VariantModuleRow),
VariantStats = 44 => VariantStats(VariantStatsRow),
VariantUpgrades = 45 => VariantUpgrade(VariantUpgradeRow),
ArmyUpgradeGroups = 46 => ArmyUpgradeGroup(ArmyUpgradeGroupRow),
ArmyUpgradeRequests = 47 => ArmyUpgradeRequest(ArmyUpgradeRequestRow),
ArmyUpgradeDeliveries = 48 => ArmyUpgradeDelivery(ArmyUpgradeDeliveryRow),
ArmyUpgradeProduced = 49 => ArmyUpgradeProduced(ArmyUpgradeProducedRow),
CountryFuel = 50 => CountryFuel(CountryFuelRow),
ResourceDefinitions = 51 => ResourceDefinition(ResourceDefinitionRow),
CountryResources = 52 => CountryResource(CountryResourceRow),
ResourceExchanges = 53 => ResourceExchange(ResourceExchangeRow),
PurchaseContracts = 54 => PurchaseContract(PurchaseContractRow),
ProvinceSupply = 55 => ProvinceSupply(ProvinceSupplyRow),
HubMotorization = 56 => HubMotorization(HubMotorizationRow),
DisruptedSupply = 57 => DisruptedSupply(DisruptedSupplyRow),
ForeignHomebases = 58 => ForeignHomebase(ForeignHomebaseRow),
CountryExperience = 59 => CountryExperience(CountryExperienceRow),
StateBuildings = 60 => StateBuilding(BuildingRow),
ProvinceBuildings = 61 => ProvinceBuilding(BuildingRow),
StateBaseResources = 62 => StateBaseResource(StateBaseResourceRow),
BuildingConstruction = 63 => BuildingConstruction(BuildingConstructionRow),
RailwayConstruction = 64 => RailwayConstruction(RailwayConstructionRow),
OccupationSettings = 65 => OccupationSettings(OccupationSettingsRow),
StateOccupationLaws = 66 => StateOccupationLaw(StateOccupationLawRow),
StateGarrisons = 67 => StateGarrison(StateGarrisonRow),
StateAssignedManpower = 68 => StateAssignedManpower(StateAssignedManpowerRow),
CountryOccupationReserveManpower = 69 => CountryOccupationReserveManpower(CountryOccupationReserveManpowerRow),
StateResistance = 70 => StateResistance(StateResistanceRow),
CountryCollaboration = 71 => CountryCollaboration(CountryCollaborationRow),
Characters = 72 => Character(CharacterRow),
CountryLeaderRoles = 73 => CountryLeaderRole(CountryLeaderRoleRow),
AdvisorRoles = 74 => AdvisorRole(AdvisorRoleRow),
UnitLeaders = 75 => UnitLeader(UnitLeaderRow),
UnitLeaderTraits = 76 => UnitLeaderTrait(UnitLeaderTraitRow),
UnitLeaderTraitExperience = 77 => UnitLeaderTraitExperience(UnitLeaderTraitExperienceRow),
ActiveAbilities = 78 => ActiveAbility(ActiveAbilityRow),
Capitulation = 79 => Capitulation(CapitulationRow),
Subjects = 80 => Subject(SubjectRow),
Autonomy = 81 => Autonomy(AutonomyRow),
IntelligenceAgencies = 82 => IntelligenceAgency(IntelligenceAgencyRow),
OperativeMemberships = 83 => OperativeMembership(OperativeMembershipRow),
Cryptology = 84 => Cryptology(CryptologyRow),
IntelNetworks = 85 => IntelNetwork(IntelNetworkRow),
ScriptFlags = 86 => ScriptFlag(ScriptFlagRow),
ScriptVariables = 87 => ScriptVariables(ScriptVariablesRow),
Operations = 88 => Operation(OperationRow),
OperationAssignments = 89 => OperationAssignment(OperationAssignmentRow),
WingEquipment = 90 => WingEquipment(WingEquipmentRow),
AirBases = 91 => AirBase(AirBaseRow),
WingCumulative = 92 => WingCumulative(WingCumulativeRow),
AirRegionHistory = 93 => AirRegionHistory(AirRegionHistoryRow),
AirRegionDamage = 94 => AirRegionDamage(AirRegionDamageRow),
AirAces = 95 => AirAce(AirAceRow),
AirStatistics = 96 => AirStatistics(AirStatisticsRow),
SunkConvoys = 97 => SunkConvoy(SunkConvoyRow),
RailCooldowns = 98 => RailCooldown(RailCooldownRow),
RailEdges = 99 => RailEdge(RailEdgeRow),
LandTheatres = 100 => LandTheatre(LandTheatreRow),
LandTheatreGroups = 101 => LandTheatreGroup(LandTheatreGroupRow),
LandTheatreMembers = 102 => LandTheatreMember(LandTheatreMemberRow),
AirTheatres = 103 => AirTheatre(AirTheatreRow),
AirTheatreGroups = 104 => AirTheatreGroup(AirTheatreGroupRow),
AirTheatreMembers = 105 => AirTheatreMember(AirTheatreMemberRow),
NavalResults = 106 => NavalResult(NavalResultRow),
NavalResultMembers = 107 => NavalResultMember(NavalResultMemberRow),
NavalResultCountries = 108 => NavalResultCountry(NavalResultCountryRow),
SunkShips = 109 => SunkShips(SunkShipsRow),
ProjectPrograms = 110 => ProjectProgram(ProjectProgramRow),
Projects = 111 => Project(ProjectRow),
ProjectHistory = 112 => ProjectHistory(ProjectHistoryRow),
ProjectBreakthroughs = 113 => ProjectBreakthrough(ProjectBreakthroughRow),
Nukes = 114 => Nuke(NukeRow),
RaidCountries = 115 => RaidCountry(RaidCountryRow),
Raids = 116 => Raid(RaidRow),
PeaceConferences = 117 => PeaceConference(PeaceConferenceRow),
WeatherRegions = 118 => WeatherRegion(WeatherRegionRow),
WeatherProvinces = 119 => WeatherProvince(WeatherProvinceRow),
OperationTokens = 120 => OperationTokens(OperationTokensRow),
LexicalTokens = 121 => LexicalToken(LexicalTokenRow),
NavalSupremacy = 122 => NavalSupremacy(NavalSupremacyRow),
RadarCoverage = 123 => RadarCoverage(RadarCoverageRow),
VolunteerHosts = 124 => VolunteerHost(VolunteerHostRow),
TheatreAreas = 125 => TheatreArea(TheatreAreaRow),
StationedUnits = 126 => StationedUnit(StationedUnitRow),
UnitIntel = 127 => UnitIntel(UnitIntelRow),
AirBaseRegistrations = 128 => AirBaseRegistration(AirBaseRegistrationRow),
ProductionLicenses = 129 => ProductionLicense(ProductionLicenseRow),
DeploymentConveyors = 130 => DeploymentConveyor(DeploymentConveyorRow),
DeploymentLines = 131 => DeploymentLine(DeploymentLineRow),
DeploymentManpower = 132 => DeploymentManpower(DeploymentManpowerRow),
DeploymentEquipment = 133 => DeploymentEquipment(DeploymentEquipmentRow),
WorldTension = 134 => WorldTension(WorldTensionRow),
ThreatSources = 135 => ThreatSource(ThreatSourceRow),
ThreatCountryAggregates = 136 => ThreatCountryAggregate(ThreatCountryAggregateRow),
ProductionLicenseRecords = 137 => ProductionLicenseRecord(ProductionLicenseRecordRow),
OrdersGroups = 138 => OrdersGroup(OrdersGroupRow),
OrdersGroupMembers = 139 => OrdersGroupMember(OrdersGroupMemberRow),
ArmyGroupMembers = 140 => ArmyGroupMember(ArmyGroupMemberRow),
IncomingDiplomaticActions = 141 => IncomingDiplomaticAction(IncomingDiplomaticActionRow),
Scientists = 142 => Scientist(ScientistRow),
ScientistSkills = 143 => ScientistSkill(ScientistSkillRow),
RelationOpinions = 144 => RelationOpinion(RelationOpinionRow),
RelationTuples = 145 => RelationTuple(RelationTupleRow),
CountryIntel = 146 => CountryIntel(CountryIntelRow),
ProvinceNames = 147 => ProvinceNames(ProvinceNameRow),
LandCombatProgress = 148 => LandCombatProgress(LandCombatProgressRow),
DivisionTemplates = 149 => DivisionTemplates(DivisionTemplateRow),
DivisionMovement = 150 => DivisionMovement(DivisionMovementRow),
DivisionStatus = 151 => DivisionStatus(DivisionStatusRow),
StateFlags = 152 => StateFlag(StateFlagRow),
ArmyOrderPlans = 153 => ArmyOrderPlan(ArmyOrderPlanRow),
OrdersGroupPlans = 154 => OrdersGroupPlan(OrdersGroupPlanRow),
Fronts = 155 => Front(FrontRow),
CountryAppearance = 156 => CountryAppearance(CountryAppearanceRow),
ArmyIconInputs = 157 => ArmyIconInput(ArmyIconInputRow),
ShipModels = 158 => ShipModel(ShipModelRow),
SupplyNodes = 159 => SupplyNode(SupplyNodeRow),
TrainInventory = 160 => TrainInventory(TrainInventoryRow),
LandCombatDetails = 161 => LandCombatDetails(LandCombatDetailsRow),
LandCombatDivisions = 162 => LandCombatDivisions(LandCombatDivisionRow),
LandCombatModifiers = 163 => LandCombatModifiers(LandCombatModifierRow),
NavalCombatDetails = 164 => NavalCombatDetails(NavalCombatDetailsRow),
NavalCombatShips = 165 => NavalCombatShips(NavalCombatShipRow),
NavalCombatResultDetails = 166 => NavalCombatResultDetails(NavalCombatResultDetailsRow),
ProvinceUnitOrder = 167 => ProvinceUnitOrder(ProvinceUnitOrderRow),
CharacterPortraits = 168 => CharacterPortrait(CharacterPortraitRow),
LeaderPortraits = 169 => LeaderPortrait(LeaderPortraitRow),
TaskForceCounterInputs = 170 => TaskForceCounterInput(TaskForceCounterInputRow),
FleetCounterInputs = 171 => FleetCounterInput(FleetCounterInputRow),
CountryCounterInputs = 172 => CountryCounterInput(CountryCounterInputRow),
OrdersGroupCounterInputs = 173 => OrdersGroupCounterInput(OrdersGroupCounterInputRow),
LeaderPortraitPaths = 174 => LeaderPortraitPath(LeaderPortraitPathRow),
DivisionEtaInputs = 175 => DivisionEtaInputs(DivisionEtaInputsRow),
RaidTargets = 176 => RaidTarget(RaidTargetRow),
NavalBases = 177 => NavalBases(NavalBaseRow),
ResearchInputs = 178 => ResearchInputs(ResearchInputRow),
ResearchAppliedBonuses = 179 => ResearchAppliedBonuses(ResearchAppliedBonusRow),
ResearchModifiers = 180 => ResearchModifiers(ResearchModifierRow),
SimulationFeatures = 181 => SimulationFeatures(SimulationFeatureRow),
CountryVisibility = 182 => CountryVisibility(CountryVisibilityRow),
StateVisibility = 183 => StateVisibility(StateVisibilityRow),
RelationLinks = 184 => RelationLinks(RelationLinkRow),
SliceReads = 185 => SliceReads(SliceReadRow),
DivisionTemplateIcons = 186 => DivisionTemplateIcons(DivisionTemplateIconRow),
FactionTheatres = 187 => FactionTheatres(FactionTheatreRow),
RailwayGunCounterInputs = 188 => RailwayGunCounterInput(RailwayGunCounterInputRow),
CountryLocalSubunits = 189 => CountryLocalSubunits(CountryLocalSubunitRow),
AvailableEquipmentVariants = 190 => AvailableEquipmentVariants(AvailableEquipmentVariantRow),
NewDivisionEquipment = 191 => NewDivisionEquipment(DivisionEquipmentRow),
ModifierDefinitions = 192 => ModifierDefinition(ModifierDefinitionRow),
HiredAdvisors = 193 => HiredAdvisor(HiredAdvisorRow),
CountryStabilityInputs = 194 => CountryStabilityInput(CountryStabilityInputRow),
CountryStabilityLines = 195 => CountryStabilityLine(CountryStabilityLineRow),
CountryFuelConsumers = 196 => CountryFuelConsumer(CountryFuelConsumerRow),
SentVolunteers = 197 => SentVolunteer(SentVolunteerRow),
RaidTargetStatuses = 198 => RaidTargetStatus(RaidTargetStatusRow),
RaidCooldowns = 199 => RaidCooldowns(RaidCooldownRow),
RaidPathDistances = 200 => RaidPathDistances(RaidPathDistanceRow),
RaidCountryInputs = 201 => RaidCountryInput(RaidCountryInputRow),
StockpileArchetypes = 202 => StockpileArchetype(StockpileArchetypeRow),
RaidAirSites = 203 => RaidAirSite(RaidAirSiteRow),
AceRangeInputs = 204 => AceRangeInput(AceRangeInputRow),
StateResourceInputs = 205 => StateResourceInput(StateResourceInputRow),
StateResources = 206 => StateResource(StateResourceRow),
StateDynamicModifiers = 207 => StateDynamicModifier(StateDynamicModifierRow),
CountryBuildingResources = 208 => CountryBuildingResource(CountryBuildingResourceRow),
CountryWars = 209 => CountryWars(CountryWarRow),
NavalRegionAccess = 210 => NavalRegionAccess(NavalRegionAccessRow),
BorderConflicts = 211 => BorderConflicts(BorderConflictRow),
RaidPlayerAllowedTypes = 212 => RaidPlayerAllowedTypes(RaidPlayerAllowedTypeRow),
CountryCommandPower = 213 => CountryCommandPower(CountryCommandPowerRow),
DivisionRetreatBans = 214 => DivisionRetreatBans(DivisionRetreatBanRow),
StateCores = 215 => StateCores(StateCoreRow),
RaidSpeedInputs = 216 => RaidSpeedInput(RaidSpeedInputRow),
RaidEquipment = 217 => RaidEquipment(RaidEquipmentRow),
InvasionPreparations = 218 => InvasionPreparation(InvasionPreparationRow),
CountryCharacters = 219 => CountryCharacter(CountryCharacterRow),
PartyLeaders = 220 => PartyLeader(PartyLeaderRow),
CountryCivilWars = 221 => CountryCivilWar(CountryCivilWarRow),
AdvisorTokens = 222 => AdvisorToken(AdvisorTokenRow),
FactionAdvisors = 223 => FactionAdvisor(FactionAdvisorRow),
LandCombatSides = 224 => LandCombatSide(LandCombatSideRow),
AirRegionalMissions = 225 => AirRegionalMissions(AirRegionalMissionRow),
AirCountryRelations = 226 => AirCountryRelations(AirCountryRelationRow),
AirReferenceWitnesses = 227 => AirReferenceWitnesses(AirReferenceWitnessRow),
NavalMineLevels = 228 => NavalMineLevels(NavalMineLevelRow),
AirMissionInputs = 229 => AirMissionInputs(AirMissionInputRow),
AirSortieBaseCountries = 230 => AirSortieBaseCountries(AirSortieBaseCountryRow),
AirSortieModifiers = 231 => AirSortieModifiers(AirSortieModifierRow),
AirSortieSpecialBases = 232 => AirSortieSpecialBases(AirSortieSpecialBaseRow),
AirSortieModifierMetadata = 233 => AirSortieModifierMetadata(AirSortieModifierMetadataRow),
LocalPlayer = 234 => LocalPlayer(LocalPlayerRow),
AirNavalPatrolCombats = 235 => AirNavalPatrolCombats(AirNavalPatrolCombatRow),
AirNavalPatrolHistories = 236 => AirNavalPatrolHistories(AirNavalPatrolHistoryRow),
RaidFacilities = 237 => RaidFacility(RaidFacilityRow),
RaidAirScores = 238 => RaidAirScore(RaidAirScoreRow),
RaidIntelInputs = 239 => RaidIntelInput(RaidIntelInputRow),
RaidTooltipOperands = 240 => RaidTooltipOperand(RaidTooltipOperandRow),
TaskForceMapMotion = 241 => TaskForceMapMotion(TaskForceMapMotionRow),
FloatingHarborDeployments = 242 => FloatingHarborDeployments(FloatingHarborDeploymentRow),
FacilityMapFlags = 243 => FacilityMapFlags(FacilityMapFlagRow),
CountryRetiredCharacters = 244 => CountryRetiredCharacter(CountryCharacterRow),
CharacterRoleTraits = 245 => CharacterRoleTrait(CharacterRoleTraitRow),
CharacterOperatives = 246 => CharacterOperative(CharacterOperativeRow),
NavalProvinceCombats = 247 => NavalProvinceCombats(NavalProvinceCombatRow),
NavalCombatTransfers = 248 => NavalCombatTransfers(NavalCombatTransferRow),
NavalTransferUnits = 249 => NavalTransferUnits(NavalTransferUnitRow),
TaskForceMissions = 250 => TaskForceMissions(TaskForceMissionRow),
NavalCombatAirBaseWingHandles = 251 => NavalCombatAirBaseWingHandles(NavalCombatAirBaseWingHandlesRow),
CountryFactories = 252 => CountryFactories(CountryFactoriesRow),
NavalMissionTaskForces = 253 => NavalMissionTaskForce(NavalMissionTaskForceRow),
NavalMissionLedgers = 254 => NavalMissionLedger(NavalMissionLedgerRow),
NavalMissionCountries = 255 => NavalMissionCountry(NavalMissionCountryRow),
NavalMissionCommanders = 256 => NavalMissionCommander(NavalMissionCommanderRow),
GlobalSavedEventTargets = 257 => GlobalSavedEventTargets(GlobalSavedEventTargetRow),
ReserveModifierBreakdowns = 258 => ReserveModifierBreakdowns(ReserveModifierBreakdownRow),
PowerBalanceModifierBreakdowns = 259 => PowerBalanceModifierBreakdowns(PowerBalanceModifierBreakdownRow),
LogisticsEquipment = 260 => LogisticsEquipment(LogisticsEquipmentRow),
IndustrialOrganisations = 261 => IndustrialOrganisation(IndustrialOrganisationRow),
IndustrialOrganisationHistory = 262 => IndustrialOrganisationHistoryEntry(IndustrialOrganisationHistoryRow),
ProductionLineOutput = 263 => ProductionLineOutput(ProductionLineOutputRow),
ConstructionLineOutput = 264 => ConstructionLineOutput(ConstructionLineOutputRow),
CombatLogEquipment = 265 => CombatLogEquipment(CombatLogEquipmentRow),
CombatLogState = 266 => CombatLogState(CombatLogStateRow),
LendLeaseEquipment = 267 => LendLeaseEquipment(LendLeaseEquipmentRow),
DivisionCensus = 268 => DivisionCensus(DivisionCensusRow),
EquipmentDestruction = 269 => EquipmentDestruction(EquipmentDestructionRow),
NavalModifierBreakdowns = 270 => NavalModifierBreakdowns(NavalModifierBreakdownRow),
AvailableAdvisors = 271 => AvailableAdvisor(AvailableAdvisorRow),
OwnedWargoals = 272 => OwnedWargoal(OwnedWargoalRow),
TimedIdeas = 273 => TimedIdea(TimedIdeaRow),
DecisionRandomItems = 274 => DecisionRandomItem(DecisionRandomItemRow),
CapturedOperativeReferences = 275 => CapturedOperativeReference(CapturedOperativeReferenceRow),
ReservedCountryNaming = 276 => ReservedCountryNaming(ReservedCountryNamingRow),
AttritionModifierBreakdowns = 277 => AttritionModifierBreakdowns(AttritionModifierBreakdownRow),
CountryProductionModifiers = 278 => CountryProductionModifiers(CountryProductionModifiersRow),
ProductionLineModifiers = 279 => ProductionLineModifiers(ProductionLineModifiersRow),
CountryExperienceGains = 280 => CountryExperienceGains(CountryExperienceGainRow),
CountryGameScoreInputs = 281 => CountryGameScoreInput(CountryGameScoreInputRow),
AirWingGameScoreStats = 282 => AirWingGameScoreStats(AirWingGameScoreStatsRow),
GameScoreMultipliers = 283 => GameScoreMultiplier(GameScoreMultiplierRow),
AgencyUpgradeLevels = 284 => AgencyUpgradeLevel(AgencyUpgradeLevelRow),
OperationCompletions = 285 => OperationCompletion(OperationCompletionRow),
CountryPowerDaily = 286 => CountryPowerDaily(CountryPowerDailyRow),
CountryPowerChanges = 287 => CountryPowerChange(CountryPowerChangeRow),
CountryPowerDailyCosts = 288 => CountryPowerDailyCost(CountryPowerDailyCostRow),
GovernmentChanges = 289 => GovernmentChange(GovernmentChangeRow),
PoliticsCaptureCoverage = 290 => PoliticsCaptureCoverage(PoliticsCaptureCoverageRow),
WarOwnedRelations = 291 => WarOwnedRelations(WarOwnedRelationRow),
WarRelationGoals = 292 => WarRelationGoals(WarRelationGoalRow),
WarOverviewCountries = 293 => WarOverviewCountries(WarOverviewCountryRow),
WarNameRules = 294 => WarNameRules(WarNameRuleRow),
PeaceSettlements = 295 => PeaceSettlements(PeaceSettlementRow),
PeaceBids = 296 => PeaceBids(PeaceSettlementRow),
DivisionEquipmentDeletions = 297 => DivisionEquipmentDeletion(DivisionEquipmentDeletionRow),
DivisionLandInputs = 298 => DivisionLandInputs(DivisionLandInputsRow),
LandCombatResults = 299 => LandCombatResults(LandCombatResultsRow),
DestroyedDivisions = 300 => DestroyedDivision(DestroyedDivisionsRow),
LandEventCoverage = 301 => LandEventCoverage(LandEventCoverageRow),
NavalResultSources = 302 => NavalResultSources(NavalResultSourceRow),
AirLossEvents = 303 => AirLossEvent(AirLossEventRow),
BuildingBombingDamage = 304 => BuildingBombingDamage(BuildingBombingDamageRow),
AirDailyRoster = 305 => AirDailyRoster(AirDailyRosterRow),
EventFeedHistory = 306 => EventFeedHistory(EventFeedHistoryRow),
RocketProductionOutput = 307 => RocketProductionOutput(RocketProductionOutputRow),
ReservedNearestNavies = 308 => ReservedNearestNavy(ReservedNearestNavyRow),
ReservedNearestBases = 309 => ReservedNearestBase(ReservedNearestBaseRow),
TransportNavies = 310 => TransportNavy(TransportNavyRow),
TransportLedgers = 311 => TransportLedger(TransportLedgerRow),
TransportReceivers = 312 => TransportReceiver(TransportReceiverRow),
TransportSearchRegions = 313 => TransportSearchRegion(TransportSearchRegionRow),
TransportSpottingOwners = 314 => TransportSpottingOwner(TransportSpottingOwnerRow),
TransportSpottingRegions = 315 => TransportSpottingRegion(TransportSpottingRegionRow),
TransportAirRoots = 316 => TransportAirRoot(TransportAirRootRow),
TransportAirRegions = 317 => TransportAirRegion(TransportAirRegionRow),
TransportRiskShips = 318 => TransportRiskShips(TransportRiskShipsRow),
NavalModifierTrees = 319 => NavalModifierTrees(NavalModifierTreeRow),
NavalModifierTreeSamples = 320 => NavalModifierTreeSamples(NavalModifierTreeSampleRow),
AirRegionCombat = 321 => AirRegionCombat(AirRegionCombatRow),
NuclearStrikes = 322 => NuclearStrike(NuclearStrikeRow),
LandCombatDivisionModifiers = 323 => LandCombatDivisionModifier(LandCombatDivisionModifierRow),
FuelDemandParts = 324 => FuelDemandPart(FuelDemandPartRow),
MapEvents = 325 => MapEvents(MapEventsRow),
OwnerTransportSpeeds = 326 => OwnerTransportSpeed(OwnerTransportSpeedRow),
ReserveModifierTrees = 327 => ReserveModifierTree(ReserveModifierTreeRow),
BattleSounds = 328 => BattleSound(BattleSoundRow),
OperativeMissions = 329 => OperativeMission(OperativeMissionRow),
StrategicOperatives = 330 => StrategicOperative(StrategicOperativeRow),
AgencyOperationBuffers = 331 => AgencyOperationBuffers(AgencyOperationBuffersRow),
ResourceOrigins = 332 => ResourceOrigin(ResourceOriginRow),
ResourceRights = 333 => ResourceRight(ResourceRightRow),
CountryOilFactors = 334 => CountryOilFactor(CountryOilFactorRow),
StateTemporaryResources = 335 => StateTemporaryResource(StateTemporaryResourceRow),
ConvoyClients = 336 => ConvoyClient(ConvoyClientRow),
TradeRoutes = 337 => TradeRoute(TradeRouteRow),
RegionalConvoyEfficiency = 338 => RegionalConvoyEfficiency(RegionalConvoyEfficiencyRow),
CountryWargoals = 339 => CountryWargoal(CountryWargoalRow),
WargoalTargets = 340 => WargoalTarget(WargoalTargetRow),
WargoalJustifications = 341 => WargoalJustification(WargoalJustificationRow),
IndustrialOrganisationDetails = 342 => IndustrialOrganisationDetails(IndustrialOrganisationDetailsRow),
IndustrialOrganisationPolicies = 343 => IndustrialOrganisationPolicies(IndustrialOrganisationPoliciesRow),
VariantChanges = 344 => VariantChange(VariantChangeRow),
VariantBonusSnapshots = 345 => VariantBonusSnapshot(VariantBonusSnapshotRow),
DesignCountryModifiers = 346 => DesignCountryModifier(DesignCountryModifierRow),
DesignCountryTemplates = 347 => DesignCountryTemplates(DesignCountryTemplatesRow),
DesignObserverContext = 348 => DesignObserverContext(DesignObserverContextRow),
DesignOrganisationLiveness = 349 => DesignOrganisationLiveness(DesignOrganisationLivenessRow),

DivisionHistory = 350 => DivisionHistory(SharedDivisionHistoryRow),
DivisionPanelSources = 351 => DivisionPanelSources(SharedDivisionPanelSourceRow),
DivisionPanelStats = 352 => DivisionPanelStats(DivisionPanelStatsRow),
DoctrineFolders = 353 => DoctrineFolder(DoctrineFolderRow),
DoctrineTracks = 354 => DoctrineTrack(DoctrineTrackRow),
DoctrineLeaderMastery = 355 => DoctrineLeaderMastery(DoctrineLeaderMasteryRow),
DoctrineCostReductions = 356 => DoctrineCostReduction(DoctrineCostReductionRow),
DoctrineMasteryGrants = 357 => DoctrineMasteryGrant(DoctrineMasteryGrantRow),
DoctrineEquipmentBonuses = 358 => DoctrineEquipmentBonus(DoctrineEquipmentBonusRow),
DoctrineCountries = 359 => DoctrineCountry(DoctrineCountryRow),
ScientistDaily = 360 => ScientistDaily(ScientistDailyRow),
FactionAccounts = 361 => FactionAccount(FactionAccountRow),
FactionResearch = 362 => FactionResearch(FactionResearchRow),
FactionMinimumUpgrades = 363 => FactionMinimumUpgrade(FactionMinimumUpgradeRow),
FactionDoctrine = 364 => FactionDoctrine(FactionDoctrineRow),
FactionProjection = 365 => FactionProjection(FactionProjectionRow),
FactionResources = 366 => FactionResource(FactionResourceRow),
FactionGoalStatus = 367 => FactionGoalStatus(FactionGoalStatusRow),
FactionGoalEntries = 368 => FactionGoalEntry(FactionGoalEntryRow),
FactionRules = 369 => FactionRule(FactionRuleRow),
FactionPrograms = 370 => FactionProgram(FactionProgramRow),
FactionManpower = 371 => FactionManpower(FactionManpowerRow),
FactionUnlocks = 372 => FactionUnlock(FactionUnlockRow),
FactionVariables = 373 => FactionVariable(FactionVariableRow),
FactionAdvisorModifiers = 374 => FactionAdvisorModifier(FactionAdvisorModifierRow),
CombatLogGroups = 375 => CombatLogGroups(CombatLogGroupRow),
CombatLogMonths = 376 => CombatLogMonths(CombatLogMonthRow),
CombatLogManpower = 377 => CombatLogManpower(CombatLogManpowerRow),
CombatLogTemplates = 378 => CombatLogTemplates(CombatLogTemplateRow),
CombatLogReferences = 379 => CombatLogReferences(CombatLogReferenceRow),
CombatLogBattles = 380 => CombatLogBattles(CombatLogBattleRow),
CombatLogEnemyEquipment = 381 => CombatLogEnemyEquipment(CombatLogEnemyEquipmentRow),
CombatLogBattleSides = 382 => CombatLogBattleSides(CombatLogBattleSideRow),
CombatLogBattleEquipment = 383 => CombatLogBattleEquipment(CombatLogBattleEquipmentRow),
CombatLogBattleModifiers = 384 => CombatLogBattleModifiers(CombatLogBattleModifierRow),
CombatLogBattleCountries = 385 => CombatLogBattleCountries(CombatLogBattleCountryRow),
CombatLogObservers = 386 => CombatLogObservers(CombatLogObserverRow),
CountryTemplateState = 387 => CountryTemplateState(CountryTemplateStateRow),
NavalRepairCountries = 388 => NavalRepairCountries(NavalRepairCountryRow),
NavalRepairFleets = 389 => NavalRepairFleets(NavalRepairFleetRow),
NavalRepairTaskForces = 390 => NavalRepairTaskForces(NavalRepairTaskForceRow),
NavalTaskForceComposition = 391 => NavalTaskForceComposition(NavalTaskForceCompositionRow),
NavalRepairBases = 392 => NavalRepairBases(NavalRepairBaseRow),
NavalRepairQueue = 393 => NavalRepairQueue(NavalRepairQueueRow),
NavalRefitShips = 394 => NavalRefitShips(NavalRefitShipRow),
NavalRefitProgress = 395 => NavalRefitProgress(NavalRefitProgressRow),
NavalRepairChildren = 396 => NavalRepairChildren(NavalRepairChildRow),
CountryMarket = 397 => CountryMarket(CountryMarketRow),
MarketContractTerms = 398 => MarketContractTerms(MarketContractTermsRow),
LendLeaseAgreements = 399 => LendLeaseAgreement(LendLeaseAgreementRow),
MarketEvents = 400 => MarketEvent(MarketEventRow),
IntelNetworkSummaries = 401 => IntelNetworkSummary(IntelNetworkSummaryRow),
PropagandaEfforts = 402 => PropagandaEffort(PropagandaEffortRow),
IntelContributions = 403 => IntelContribution(IntelContributionRow),
IntelSources = 404 => IntelSource(IntelSourceRow),
IntelNetworkAuxiliaries = 405 => IntelNetworkAuxiliary(IntelNetworkAuxiliaryRow),
MovementStateTerms = 406 => MovementStateTerm(MovementStateTermRow),
CoreSupplyAccess = 407 => CoreSupplyAccess(CoreSupplyAccessRow),
CountryUnitSpeedEntries = 408 => CountryUnitSpeedEntry(CountryUnitSpeedEntryRow),
RiverRelations = 409 => RiverRelation(RiverRelationRow),
DivisionTransportSpeeds = 410 => DivisionTransportSpeed(OwnerTransportSpeedRow),
WarParticipation = 411 => WarParticipation(WarParticipationRow),
WarCapturedProvinces = 412 => WarCapturedProvinces(WarCapturedProvinceRow),
WarLendLeaseHistories = 413 => WarLendLeaseHistories(WarLendLeaseHistoryRow),
AirGroundCrews = 414 => AirGroundCrews(AirGroundCrewRow),
AirTransferInputs = 415 => AirTransferInputs(AirTransferInputRow),
NavalRegionalMines = 416 => NavalRegionalMines(NavalRegionalMineRow),
NavalAccidentReports = 417 => NavalAccidentReports(NavalAccidentReportRow),
NavalDominanceTaskForceRanges = 418 => NavalDominanceTaskForceRanges(NavalMissionTaskForceRow),
NavalPatrolMissionInputs = 419 => NavalPatrolMissionInputs(NavalPatrolMissionInputRow),
CountryUnitBonuses = 420 => CountryUnitBonus(CountryUnitBonusRow),
DivisionModelInputs = 421 => DivisionModelInput(DivisionModelInputRow),
}
impl TableId {
    /// Discard only proven field framing. No old row is decoded, validated as
    /// today's type, or translated; Opaque payloads are skipped by byte length.
    pub fn skip_snapshot(&self, fields: usize, input: &mut Decoder<'_>) -> io::Result<()> {
        let schema = &self.schema()[..fields];
        let minimum = 1 + schema
            .iter()
            .map(|kind| kind.minimum_row_bytes())
            .sum::<usize>();
        let count = input.count(minimum)?;
        if count > 1 && self.key_bits() == 0 {
            return Err(invalid("singleton table cannot contain multiple rows"));
        }
        let mut previous = 0;
        for index in 0..count {
            let key = self.decode_key(previous, input)?;
            if index != 0 && key <= previous {
                return Err(invalid("row keys did not advance"));
            }
            previous = key;
            for kind in schema {
                kind.decode(input)?;
            }
        }
        Ok(())
    }

    pub fn skip_delta(&self, fields: usize, input: &mut Decoder<'_>) -> io::Result<()> {
        let schema = &self.schema()[..fields];
        let width = fields.div_ceil(8);
        let count = input.count(1 + width)?;
        let mut previous = 0;
        for index in 0..count {
            let key = self.decode_key(previous, input)?;
            if index != 0 && key <= previous {
                return Err(invalid("delta keys did not advance"));
            }
            previous = key;
            let mut bytes = [0; 8];
            bytes[..width].copy_from_slice(input.take(width)?);
            let mask = u64::from_le_bytes(bytes);
            if mask == 0 || mask & !full_mask(fields) != 0 {
                return Err(invalid("invalid discarded delta mask"));
            }
            for (i, kind) in schema.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    kind.decode_change(input)?;
                }
            }
        }
        let count = input.count(1)?;
        previous = 0;
        for index in 0..count {
            let key = self.decode_key(previous, input)?;
            if index != 0 && key <= previous {
                return Err(invalid("deleted keys did not advance"));
            }
            previous = key;
        }
        Ok(())
    }

    /// Whether a field's changes are whole byte strings (strings, lists and
    /// nested records) rather than numeric differences.
    pub fn byte_field(self, field: usize) -> bool {
        matches!(
            self.schema().get(field),
            Some(Kind::Bytes | Kind::U32List | Kind::U64List | Kind::Opaque)
        )
    }

    /// The exact bytes of one field's plain encoding, read from `input`.
    pub fn field_span<'a>(self, field: usize, input: &mut Decoder<'a>) -> io::Result<&'a [u8]> {
        let before = input.rest();
        self.schema()[field].decode(input)?;
        Ok(&before[..before.len() - input.remaining()])
    }

    /// The exact bytes of one field's plain delta change, read from `input`.
    pub fn change_span<'a>(self, field: usize, input: &mut Decoder<'a>) -> io::Result<&'a [u8]> {
        let before = input.rest();
        self.schema()[field].decode_change(input)?;
        Ok(&before[..before.len() - input.remaining()])
    }

    /// Walk one plain table update in `format`, from its status byte, and
    /// report every token's exact bytes in wire order. Counts, masks and field
    /// framing get the same checks as decoding; row values are not decoded.
    pub fn walk_update<'a>(
        self,
        bytes: &'a [u8],
        format: u16,
        visit: &mut dyn FnMut(Token, &'a [u8]),
    ) -> io::Result<()> {
        let mut input = Decoder::new(bytes);
        input.format = format;
        let fields = self.last_change().fields(self, format);
        let (status, token) = span(&mut input, |input| input.u8())?;
        visit(Token::Status, token);
        match status {
            0 | 3 => {
                if status == 3 {
                    let (missing, token) = span(&mut input, |input| input.u32())?;
                    if missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    visit(Token::Missing, token);
                }
                let minimum = self.minimum_row_bytes_at(format);
                self.walk_rows(fields, WalkRows::Snapshot, minimum, &mut input, visit)?;
            }
            1 => {
                let (_, token) = span(&mut input, |input| input.i32())?;
                visit(Token::Baseline, token);
                let minimum = self.minimum_delta_bytes_at(format);
                self.walk_rows(fields, WalkRows::Changed, minimum, &mut input, visit)?;
                self.walk_rows(fields, WalkRows::Deleted, 1, &mut input, visit)?;
            }
            2 => {
                let (_, token) = span(&mut input, |input| input.i32())?;
                visit(Token::Baseline, token);
            }
            4 => {
                let (reason, token) = span(&mut input, |input| input.string())?;
                if reason.is_empty() {
                    return Err(invalid("unavailable table needs a reason"));
                }
                visit(Token::Reason, token);
            }
            _ => return Err(invalid("unknown table status")),
        }
        input.finish()
    }

    /// A counted run of snapshot rows, changed rows or deleted keys.
    fn walk_rows<'a>(
        self,
        fields: usize,
        rows: WalkRows,
        minimum: usize,
        input: &mut Decoder<'a>,
        visit: &mut dyn FnMut(Token, &'a [u8]),
    ) -> io::Result<()> {
        let schema = &self.schema()[..fields];
        let width = fields.div_ceil(8);
        let (count, token) = span(input, |input| input.count(minimum))?;
        visit(Token::Count, token);
        let mut previous = 0;
        for index in 0..count {
            let (key, token) = span(input, |input| self.decode_key(previous, input))?;
            if index != 0 && key <= previous {
                return Err(invalid("row keys did not advance"));
            }
            previous = key;
            visit(Token::Key(key), token);
            match rows {
                WalkRows::Snapshot => {
                    for (field, kind) in schema.iter().enumerate() {
                        let (_, token) = span(input, |input| kind.decode(input).map(|_| ()))?;
                        visit(Token::Field(field), token);
                    }
                }
                WalkRows::Changed => {
                    let (mask, token) = span(input, |input| {
                        let mut bytes = [0; 8];
                        bytes[..width].copy_from_slice(input.take(width)?);
                        Ok(u64::from_le_bytes(bytes))
                    })?;
                    if mask == 0 || mask & !full_mask(fields) != 0 {
                        return Err(invalid("invalid recorded delta mask"));
                    }
                    visit(Token::Mask, token);
                    for (field, kind) in schema.iter().enumerate() {
                        if mask & (1 << field) != 0 {
                            let (_, token) =
                                span(input, |input| kind.decode_change(input).map(|_| ()))?;
                            visit(Token::Field(field), token);
                        }
                    }
                }
                WalkRows::Deleted => {}
            }
        }
        Ok(())
    }

    pub const fn field_count(self) -> usize {
        self.schema().len()
    }

    pub const fn mask_width(self) -> usize {
        self.field_count().div_ceil(8)
    }

    pub const fn full_mask(self) -> u64 {
        full_mask(self.field_count())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn minimum_row_bytes(self) -> usize {
        self.minimum_row_bytes_at(crate::FORMAT_VERSION)
    }
    pub fn minimum_row_bytes_at(self, format: u16) -> usize {
        1 + self.schema()[..self.last_change().fields(self, format)]
            .iter()
            .map(|kind| kind.minimum_row_bytes())
            .sum::<usize>()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn minimum_delta_bytes(self) -> usize {
        self.minimum_delta_bytes_at(crate::FORMAT_VERSION)
    }
    pub fn minimum_delta_bytes_at(self, format: u16) -> usize {
        1 + self.last_change().fields(self, format).div_ceil(8)
            + self.schema()[..self.last_change().fields(self, format)]
                .iter()
                .map(|kind| kind.minimum_change_bytes())
                .min()
                .unwrap_or(0)
    }

    pub fn check_key(self, key: u128) -> io::Result<()> {
        if self.key_bits() < 128 && key >> self.key_bits() != 0 {
            return Err(invalid("row key exceeds native width"));
        }
        if matches!(
            self,
            Self::Clock | Self::Campaign | Self::GameScoreMultipliers
        ) && key != 0
        {
            return Err(invalid("singleton row key must be zero"));
        }
        Ok(())
    }

    pub fn encode_key(self, key: u128, previous: u128, output: &mut Vec<u8>) -> io::Result<()> {
        self.check_key(key)?;
        let difference = key
            .checked_sub(previous)
            .ok_or_else(|| invalid("row keys decreased"))?;
        allocation::reserve(output, 19)?;
        put_varint(output, difference);
        Ok(())
    }

    pub fn decode_key(self, previous: u128, decoder: &mut Decoder<'_>) -> io::Result<u128> {
        let difference = decoder.varint()?;
        let key = previous
            .checked_add(difference)
            .ok_or_else(|| invalid("row key addition overflow"))?;
        self.check_key(key)?;
        Ok(key)
    }

    fn check_mask(self, mask: u64) -> io::Result<()> {
        if mask == 0 || mask & !self.full_mask() != 0 {
            return Err(invalid("delta field mask is empty or has unknown bits"));
        }
        Ok(())
    }
}

impl TableRows {
    /// Counts field gaps independently of a table's row completeness.
    pub fn field_gaps(&self) -> impl Iterator<Item = (&'static str, usize)> + '_ {
        let fields = self
            .table()
            .last_change()
            .appended
            .iter()
            .filter_map(|field| {
                let missing = self
                    .iter()
                    .filter(|row| row.not_recorded_fields() & (1 << field.index) != 0)
                    .count();
                (missing != 0).then_some((field.name, missing))
            });
        let mut nested = std::collections::BTreeMap::<&'static str, usize>::new();
        for row in self {
            row.visit_nested_field_gaps(&mut |name| {
                *nested.entry(name).or_default() += 1;
            });
        }
        fields.chain(nested)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TableRows {
    /// Synthetic historical fixtures: copy just the established field prefix.
    pub fn fixture_prefix(&self, fields: usize) -> Vec<u8> {
        let table = self.table();
        let mut current = Vec::new();
        self.encode_rows(&mut current).unwrap();
        let mut input = Decoder::new(&current);
        let mut prefix = Vec::new();
        let mut previous = 0;
        for _ in 0..self.len() {
            let start = current.len() - input.remaining();
            previous = table.decode_key(previous, &mut input).unwrap();
            for kind in &table.schema()[..fields] {
                kind.decode(&mut input).unwrap();
            }
            let end = current.len() - input.remaining();
            prefix.extend_from_slice(&current[start..end]);
            for kind in &table.schema()[fields..] {
                kind.decode(&mut input).unwrap();
            }
        }
        input.finish().unwrap();
        prefix
    }
}

impl TryFrom<u16> for TableId {
    type Error = io::Error;

    fn try_from(value: u16) -> io::Result<Self> {
        Self::ALL
            .iter()
            .find(|id| **id as u16 == value)
            .copied()
            .ok_or_else(|| invalid("unknown table id"))
    }
}

#[derive(Clone, Copy)]
enum WalkRows {
    Snapshot,
    Changed,
    Deleted,
}

/// Read one value and return it with the exact bytes it occupied.
fn span<'a, T>(
    input: &mut Decoder<'a>,
    read: impl FnOnce(&mut Decoder<'a>) -> io::Result<T>,
) -> io::Result<(T, &'a [u8])> {
    let before = input.rest();
    let value = read(input)?;
    Ok((value, &before[..before.len() - input.remaining()]))
}

/// One token of a plain table update ([`TableId::walk_update`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Status,
    Baseline,
    Count,
    Missing,
    Reason,
    Key(u128),
    Mask,
    Field(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaRow {
    /// Layout of values; retained for nested fields and old insert masks.
    pub format: u16,
    pub key: u128,
    pub mask: u64,
    /// Decoder-only field presence; never changes current wire bytes.
    pub recorded_fields: usize,
    /// Selected native fields in schema order; exact numeric differences use zigzag varints.
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
fn check_native_key(key: u128, bits: u32) -> io::Result<()> {
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
fn encode_row_reserved<T: RowCodec>(
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
fn validate_typed<T: RowCodec>(rows: &[T]) -> io::Result<()> {
    let mut previous = None;
    for row in rows {
        #[cfg(test)]
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
fn encode_typed<T: RowCodec>(rows: &[T], output: &mut Vec<u8>) -> io::Result<()> {
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
fn patch_typed<'a, T: RowCodec>(
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
fn rows_from_wire_typed<'a, T: RowCodec>(
    keys: &[u128],
    wire: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
) -> io::Result<Vec<T>> {
    let mut rows = allocation::vec(keys.len())?;
    for (at, &key) in keys.iter().enumerate() {
        rows.push(T::from_wire(key, &mut |field| wire(at, field))?);
    }
    Ok(rows)
}
fn decode_typed<T: RowCodec>(count: usize, input: &mut Decoder<'_>) -> io::Result<Vec<T>> {
    let mut rows = allocation::vec(count)?;
    let mut previous = 0;
    for _ in 0..count {
        let row = decode_row::<T>(previous, input)?;
        previous = row.key();
        rows.push(row);
    }
    Ok(rows)
}
fn encode_changes_typed<T: RowCodec>(
    rows: &[T],
    previous: &[T],
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
) -> io::Result<bool> {
    u32::try_from(rows.len()).map_err(|_| invalid("too many rows"))?;
    u32::try_from(previous.len()).map_err(|_| invalid("too many rows"))?;
    // Size a row only after the merge establishes that it changed. Walking
    // every nested payload here repeats work for the often-quiet baseline.
    allocation::reserve(output, 8)?;
    let count_at = output.len();
    output.extend_from_slice(&0_u32.to_le_bytes());
    let mut changed_count = 0_u32;
    let mut changed_key = 0_u128;
    deleted.clear();
    let mut deleted_count = 0_u32;
    let mut deleted_key = 0_u128;
    let mut delete = |key| -> io::Result<()> {
        allocation::reserve(deleted, (T::KEY_BITS as usize).div_ceil(7).max(1))?;
        put_varint(deleted, key - deleted_key);
        deleted_key = key;
        deleted_count += 1;
        Ok(())
    };
    let mut old = previous.iter().peekable();
    for row in rows {
        let key = row.key();
        while old.peek().is_some_and(|previous| previous.key() < key) {
            delete(old.next().unwrap().key())?;
        }
        let baseline = if old.peek().is_some_and(|previous| previous.key() == key) {
            old.next()
        } else {
            None
        };
        // Both tables were validated before this merge. Preserve the cheap
        // unchanged-row exit instead of entering the larger field encoder.
        if baseline == Some(row) {
            continue;
        }
        if row.encode_change(baseline, changed_key, output)? {
            changed_count += 1;
            changed_key = key;
        }
    }
    for row in old {
        delete(row.key())?;
    }
    output[count_at..count_at + 4].copy_from_slice(&changed_count.to_le_bytes());
    allocation::reserve(output, allocation::add(4, deleted.len())?)?;
    output.extend_from_slice(&deleted_count.to_le_bytes());
    output.extend_from_slice(deleted);
    Ok(changed_count != 0 || deleted_count != 0)
}
fn encode_replacements_typed<T: RowCodec>(
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
    output.extend_from_slice(&0_u32.to_le_bytes()); // unchanged membership: no deletions
    Ok(changed_count != 0)
}

fn round_rows_to_display<T: RowCodec>(rows: &mut [T]) {
    if rows.first().is_some_and(RowCodec::has_display_steps) {
        for row in rows {
            row.round_to_display();
        }
    }
}

fn apply_typed<T: RowCodec>(
    previous: Vec<T>,
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    apply_typed_at(previous, changed, deleted).map(|(rows, _)| rows)
}

/// [`apply_typed`], and the changed rows' final positions when deletions were located.
fn apply_typed_at<T: RowCodec>(
    mut previous: Vec<T>,
    changed: &[DeltaRow],
    deleted: &[u128],
) -> io::Result<(Vec<T>, Option<Vec<usize>>)> {
    // Keys ascend on both sides, so each is found searching on from the last.
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
            // The merge reports an absent key where a row-by-row walk meets it.
            let len = previous.len();
            return merge_typed(previous, len, changed, deleted, Ok, None).map(|rows| (rows, None));
        };
        from = position + 1;
        gone.push(position);
    }
    let inserted = at.iter().filter(|found| found.is_err()).count();
    // Changes to rows already present keep the table's length and order: they apply
    // in place, rather than moving every row of the table into a new one.
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
    // A merge moves every row once; an insertion or deletion in place moves the
    // rows after it, so only a few edits are cheaper in place.
    if inserted + gone.len() > IN_PLACE_EDITS {
        return merge_runs(previous, changed, &at, &gone)
            .map(|(rows, positions)| (rows, Some(positions)));
    }
    // Rows change in key order, as a merge changes them, then the edits move rows.
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

/// Insertions plus deletions applied in place rather than by a merge.
const IN_PLACE_EDITS: usize = 2;

/// Where `key` is in ascending `rows`, searching on from `from`: Ok for a
/// present key, Err for where it would go.
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

/// [`merge_typed`] of an owned baseline whose deleted keys are all present,
/// `at` and `gone` the changed and deleted keys' places in it: unchanged rows
/// move in runs between them. Changes apply in key order, as the merge applies them.
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

/// A baseline's rows moved out in order, `next` the position of the first left.
struct Runs<T> {
    rows: std::vec::IntoIter<T>,
    next: usize,
}

impl<T> Runs<T> {
    /// Move the rows before position `end` to `out`.
    fn move_to(&mut self, out: &mut Vec<T>, end: usize) {
        out.extend(self.rows.by_ref().take(end - self.next));
        self.next = end;
    }
    /// The row at the next position.
    fn take(&mut self) -> T {
        self.next += 1;
        self.rows.next().expect("a located row is present")
    }
}

/// Kept baseline rows stay valid; check changed rows in whole-validation error order.
fn apply_typed_validated<T: RowCodec>(
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

/// [`validate_typed`] of rows whose keys were valid before the rows at the
/// ascending positions `at` changed.
fn validate_typed_at<T: RowCodec>(rows: &[T], at: &[usize]) -> io::Result<()> {
    let unordered = || invalid("table rows must have matching types and unique ascending keys");
    for (index, &position) in at.iter().enumerate() {
        #[cfg(test)]
        VALIDATED_ROWS.with(|count| count.set(count.get() + 1));
        measurement::count(Counter::ValidatedRows, 1);
        let key = rows[position].key();
        check_native_key(key, T::KEY_BITS)?;
        rows[position].check_key_fields()?;
        if position > 0 && rows[position - 1].key() >= key {
            return Err(unordered());
        }
        // The next row's own checks hold unless it changed too, when its
        // turn checks this order.
        let next = position + 1;
        if at.get(index + 1) != Some(&next) && rows.get(next).is_some_and(|row| key >= row.key()) {
            return Err(unordered());
        }
    }
    Ok(())
}

/// One pass over a baseline another hour still holds: a row is cloned only
/// once the merge keeps or changes it, never a deleted one, rather than cloning
/// the whole table and merging again.
fn apply_typed_shared<T: RowCodec>(
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

/// [`apply_typed_shared`] of a validated baseline: kept rows are copied
/// without checking them again, and only the changed rows are checked, in
/// whole-validation error order.
fn apply_typed_shared_validated<T: RowCodec>(
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

/// `own` turns a baseline row into an output row; it runs only for rows the
/// merge keeps or changes. `at` receives the changed rows' positions.
fn merge_typed<T: RowCodec, R: std::borrow::Borrow<T>>(
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

/// The rows a later version of a table changed or added since an earlier
/// one, and the keys it dropped ([`TableRows::changes_since`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowChanges {
    rows: TableRows,
    deleted: Vec<u128>,
}

impl RowChanges {
    /// `rows` to set at their keys and `deleted` keys to remove, both in key order.
    pub fn new(rows: TableRows, deleted: Vec<u128>) -> Self {
        Self { rows, deleted }
    }

    /// Bytes its rows and deleted keys hold.
    pub fn heap_bytes(&self) -> usize {
        self.rows.heap_bytes() + self.deleted.capacity() * size_of::<u128>()
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            rows: self.rows.try_clone_validated()?,
            deleted: allocation::copy_vec(&self.deleted)?,
        })
    }
}

#[inline(never)]
fn changes_typed<T: RowCodec>(rows: &[T], previous: &[T]) -> io::Result<(Vec<T>, Vec<u128>)> {
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
fn apply_changes_typed<T: RowCodec>(
    mut rows: Vec<T>,
    changed: Vec<T>,
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    // Rows changed in place keep the table's order; only an insertion or a
    // deletion moves the rows after it.
    if deleted.is_empty() {
        let mut at = allocation::vec(changed.len())?;
        let mut from = 0;
        for row in &changed {
            match locate(&rows, from, row.key()) {
                Ok(position) => {
                    at.push(position);
                    from = position + 1;
                }
                Err(_) => break,
            }
        }
        if at.len() == changed.len() {
            for (position, row) in at.into_iter().zip(changed) {
                rows[position] = row;
            }
            return Ok(rows);
        }
    }
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
    /// Whether this field holds nested parts with display steps
    /// ([`crate::display`]).
    const HAS_STEPS: bool = false;
    /// A whole Opaque row field restored after a provisional retirement
    /// (`opaque_value!(T, restored)`): the tombstone's empty payload, which
    /// format-102 writers stored meanwhile and no recorded value encodes,
    /// reads as [`Recorded::NotRecorded`].
    const RESTORED: bool = false;
    /// This field holds a restored field's tombstone ([`Self::RESTORED`]),
    /// which writes the provisional writer's empty payload again.
    fn restored_tombstone(&self) -> bool {
        false
    }
    fn try_clone(&self) -> io::Result<Self>;
    /// Bytes this field's own allocations hold: its string or list at its
    /// capacity, and what a nested value holds in turn ([`Value::heap_bytes`]).
    fn heap_bytes(&self) -> usize;
    fn field(&self) -> Field<'_>;
    fn from_field(field: Field<'_>) -> io::Result<Self>;
    fn not_recorded() -> io::Result<Self> {
        Err(invalid("uncatalogued absent row field"))
    }
    fn visit_field_gaps(&self, _visit: &mut dyn FnMut(&'static str)) {}
    /// Round every nested stepped part to its step, in place.
    fn round_display(&mut self) {}
    /// Round this field's numbers to its own `step`, in place.
    fn round_to(&mut self, _step: Step) {}
    /// This field's numbers rounded and divided by `step`, as the wire holds them.
    fn to_units(&self, _step: Step) -> io::Result<Self> {
        Err(invalid("row field takes no display step"))
    }
    /// A decoded field's numbers multiplied back by `step`.
    #[allow(clippy::wrong_self_convention)]
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
fn read_field<T: NativeValue>(index: usize, kind: Kind, input: &mut Decoder<'_>) -> io::Result<T> {
    if index >= input.recorded_fields {
        T::not_recorded()
    } else {
        read_value(kind, input)
    }
}
/// [`read_field`] for a declared field: step units become native units.
fn read_stepped_field<T: NativeValue>(
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
/// Round a declared field: to its own step, or each nested stepped part.
fn round_native_field<T: NativeValue>(value: &mut T, step: Option<Step>) {
    match step {
        Some(step) => value.round_to(step),
        None if T::HAS_STEPS => value.round_display(),
        None => {}
    }
}
fn native_has_steps<T: NativeValue>(_: &T, step: Option<Step>) -> bool {
    step.is_some() || T::HAS_STEPS
}
/// The field a declared value writes: its own, or for a restored field's
/// tombstone ([`NativeValue::RESTORED`]) the empty payload a provisional
/// writer stored, so a recording holding one is written again byte for byte.
fn native_wire_field<T: NativeValue>(value: &T) -> Field<'_> {
    if value.restored_tombstone() {
        Field::Opaque(Opaque::Encoded(&[], crate::FORMAT_VERSION))
    } else {
        value.field()
    }
}
/// One declared field as [`encode_native_field`] would write it, handed to `visit`.
fn visit_wire_field<T: NativeValue>(
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
/// A declared field from the wire form [`visit_wire_field`] hands: step units become native.
fn from_wire_field<T: NativeValue>(field: Field<'_>, step: Option<Step>) -> io::Result<T> {
    if matches!(field, Field::NotRecorded) {
        return T::not_recorded();
    }
    let value = T::from_field(field)?;
    match step {
        Some(step) => value.from_units(step),
        None => Ok(value),
    }
}
/// [`RowCodec::from_wire`] through a row's own field decoding: its `fields` wire fields
/// encoded as the wire holds them, then decoded.
pub(crate) fn from_wire_encoded<'a, T: RowCodec>(
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
/// Encode a declared field, in step units when it has a step.
fn encode_native_field<T: NativeValue>(
    value: &T,
    step: Option<Step>,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    match step {
        None => native_wire_field(value).encode_reserved(output),
        Some(step) => value.to_units(step)?.field().encode_reserved(output),
    }
}
/// Encode a declared field's change from `previous`, in step units when it has
/// a step: both sides are rounded, so the difference is a whole number of steps.
fn encode_native_change<T: NativeValue>(
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
/// Apply a recorded change to a declared field: a stepped field in a stepped
/// format changes by whole steps.
fn apply_stepped_value<T: NativeValue>(
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
fn read_value<T: NativeValue>(kind: Kind, input: &mut Decoder<'_>) -> io::Result<T> {
    T::from_field(kind.decode(input)?)
}
fn apply_value<T: NativeValue>(
    target: &mut T,
    kind: Kind,
    input: &mut Decoder<'_>,
) -> io::Result<()> {
    let value = T::from_field(kind.decode_change(input)?.apply(kind, target.field())?)?;
    *target = value;
    Ok(())
}
/// A field written `5 => (hit_points): I64 @ STEP` is stored at a display
/// step ([`crate::display`]).
macro_rules! typed_codec {
    ($ty:ident, $bits:literal, |$row:ident| $key:expr, |$key_name:ident| $initial:expr,
     { $($index:literal => ($($field:tt)+) : $kind:ident $(@ $step:path)?),+ $(,)? }, |$checked:ident| $check:expr) => {
        $crate::checkpoint::rows::row!($ty, |$key_name| $initial, { $(($($field)+): $kind),+ });
        #[cfg(test)]
        impl $crate::checkpoint::endpoint::tests::SparseReference for $ty {
            fn sparse_fixture(&self, field: usize) -> $crate::checkpoint::endpoint::tests::SparseFixture<Self> {
                use $crate::checkpoint::endpoint::tests::{SparseFixture, different, native_bytes, native_copy};
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
            const STEPS: &'static [Option<$crate::display::Step>] = &[$($crate::value::display_step!($($step)?)),+];
            const NAMES: &'static [&'static str] = &[$(stringify!($($field)+)),+];
        }
        impl $ty {
            pub const fn key(&self) -> u128 { let $row = self; $key }
            /// Restored fields holding their tombstone, which encode again.
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
                [$(native_has_steps(&self.$($field)+, $crate::value::display_step!($($step)?))),+].contains(&true)
            }
            fn round_to_display(&mut self) {
                $(round_native_field(&mut self.$($field)+, $crate::value::display_step!($($step)?));)+
            }
            fn check_key_fields(&self) -> io::Result<()> { let $checked = self; $check }
            fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                $(NativeValue::visit_field_gaps(&self.$($field)+, visit);)+
            }
            fn visit_wire(
                &self,
                visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
            ) -> io::Result<()> {
                $(visit_wire_field(&self.$($field)+, $index, $crate::value::display_step!($($step)?), visit).map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", self.key(), stringify!($($field)+))))?;)+
                Ok(())
            }
            fn not_recorded_fields(&self) -> u64 {
                let mut mask = 0;
                $(if matches!(self.$($field)+.field(), Field::NotRecorded) { mask |= 1 << $index; })+
                mask
            }
            // Key initializers also serve rows with nested fields.
            #[allow(clippy::field_reassign_with_default)]
            fn try_clone(&self) -> io::Result<Self> {
                self.check_key_fields()?;
                check_native_key(self.key(), Self::KEY_BITS)?;
                self.try_clone_validated()
            }
            #[allow(clippy::field_reassign_with_default)]
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
                $(let size = allocation::add(size, native_wire_field(&self.$($field)+).encoded_bytes().map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", self.key(), stringify!($($field)+))))?)?;)+
                Ok(size)
            }
            fn encoded_bound(&self) -> io::Result<usize> {
                let size = ($bits as usize).div_ceil(7).max(1) + Self::SCHEMA.len().div_ceil(8);
                $(let size = allocation::add(size, native_wire_field(&self.$($field)+).encoded_bound().map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", self.key(), stringify!($($field)+))))?)?;)+
                Ok(size)
            }
            fn encode_change_reserved(&self, previous: Option<&Self>, previous_key: u128, output: &mut Vec<u8>) -> io::Result<bool> {
                let missing = self.not_recorded_fields() & !self.restored_tombstones();
                if missing != 0 { return Err(crate::codec::error_context(invalid("row fields not recorded"), format_args!("row {} column {}", self.key(), Self::NAMES[missing.trailing_zeros() as usize]))); }
                self.check_key_fields()?;
                check_native_key(self.key(), Self::KEY_BITS)?;
                if let Some(previous) = previous {
                    previous.check_key_fields()?;
                    if previous.key() != self.key() { return Err(invalid("delta row and baseline have different keys")); }
                    if previous == self { return Ok(false); }
                }
                let mut mask = 0_u64;
                $(if previous.is_none_or(|old| old.$($field)+ != self.$($field)+) { mask |= 1_u64 << $index; })+
                if mask == 0 { return Ok(false); }
                put_varint(output, self.key().checked_sub(previous_key).ok_or_else(|| invalid("row keys decreased"))?);
                output.extend_from_slice(&mask.to_le_bytes()[..Self::SCHEMA.len().div_ceil(8)]);
                $(if mask & (1_u64 << $index) != 0 {
                    encode_native_change(&self.$($field)+, previous.map(|old| &old.$($field)+), $crate::value::display_step!($($step)?), output).map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", self.key(), stringify!($($field)+))))?;
                })+
                Ok(true)
            }
            fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> {
                let missing = self.not_recorded_fields() & !self.restored_tombstones();
                if missing != 0 { return Err(crate::codec::error_context(invalid("row fields not recorded"), format_args!("row {} column {}", self.key(), Self::NAMES[missing.trailing_zeros() as usize]))); }
                self.check_key_fields()?;
                check_native_key(self.key(), Self::KEY_BITS)?;
                if let Some(previous) = previous {
                    previous.check_key_fields()?;
                    if previous.key() != self.key() { return Err(invalid("delta row and baseline have different keys")); }
                    if self == previous { return Ok(None); }
                }
                let mut delta = DeltaRow { format: crate::FORMAT_VERSION, key: self.key(), mask: 0, values: Vec::new(), recorded_fields: Self::SCHEMA.len() };
                $(
                    if previous.is_none_or(|old| old.$($field)+ != self.$($field)+) {
                        delta.mask |= 1_u64 << $index;
                        encode_native_change(&self.$($field)+, previous.map(|old| &old.$($field)+), $crate::value::display_step!($($step)?), &mut delta.values).map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", self.key(), stringify!($($field)+))))?;
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
                    encode_native_field(&self.$($field)+, $crate::value::display_step!($($step)?), output).map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", self.key(), stringify!($($field)+))))?;
                })+
                Ok(skipped)
            }
            fn encode_fields_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
                $(encode_native_field(&self.$($field)+, $crate::value::display_step!($($step)?), output).map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", self.key(), stringify!($($field)+))))?;)+
                Ok(())
            }
            #[allow(clippy::field_reassign_with_default)]
            fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
                let $key_name = key;
                let mut row = $initial;
                $(row.$($field)+ = read_stepped_field($index, Kind::$kind, $crate::value::display_step!($($step)?), input)?;)+
                Ok(row)
            }
            fn patch_from_wire<'a>(&mut self, mask: u64, wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>) -> io::Result<bool> {
                let key = self.key();
                $(if mask & (1_u64 << $index) != 0 {
                    self.$($field)+ = from_wire_field(wire($index)?, $crate::value::display_step!($($step)?)).map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", key, stringify!($($field)+))))?;
                })+
                self.check_key_fields()?;
                Ok(true)
            }
            #[allow(clippy::field_reassign_with_default)]
            fn from_wire<'a>(key: u128, wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>) -> io::Result<Self> {
                check_native_key(key, Self::KEY_BITS)?;
                let $key_name = key;
                let mut row = $initial;
                $(row.$($field)+ = from_wire_field(wire($index)?, $crate::value::display_step!($($step)?)).map_err(|error| crate::codec::error_context(error, format_args!("row {} column {}", key, stringify!($($field)+))))?;)+
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
                $(if $index >= fields { row.$($field)+ = NativeValue::not_recorded()?; })+

                $(if delta.mask & (1_u64 << $index) != 0 { apply_stepped_value(&mut row.$($field)+, Kind::$kind, $crate::value::display_step!($($step)?), &mut input)?; })+
                input.finish()?;
                Ok(row)
            }
        }
    };
}

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
row_codec!(ProvinceRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => state: U32, 1 => owner: U32, 2 => controller: U32, 3 => victory_points: I32, 4 => fort_positive: U8, 5 => move_gate: U8, 6 => port_sea: I32, 7 => attack: Opaque, 8 => selected_areas: Opaque, 9 => fort_bunker: OptionalI64, 10 => controller_area: OptionalU32, 11 => owner_area: OptionalU32, 12 => area_node: Opaque}, |_row| Ok(()));
row_codec!(StateRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => owner: U32, 1 => controller: U32, 2 => movement_infrastructure: I64, 3 => custom_name: Bytes, 4 => owner_name: Bytes, 5 => manpower_available: I32, 6 => continent: OptionalU32, 7 => railway_infrastructure: OptionalI64, 8 => population: I32, 9 => category: OptionalU32, 10 => manpower_locked: I32, 11 => best_vp_index: OptionalI32, 12 => single_controller: OptionalU32}, |_row| Ok(()));
row_codec!(DivisionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => roster_owner: U32, 1 => logical_owner: U32, 2 => expeditionary_owner: U32, 3 => province: U32, 4 => template: U64, 5 => hit_points: I64 @ Self::HIT_POINTS_STEP, 6 => organisation: I64 @ Self::ORGANISATION_STEP, 7 => orders_group: U64, 8 => reveal_skip: U8}, |_row| Ok(()));
row_codec!(WarRelationRow, 64, |row| (row.first_country as u128) << 32 | row.second_country as u128, |key| { first_country: (key >> 32) as u32, second_country: key as u32 }, {0 => start_hour: I32, 1 => end_hour: I32, 2 => cancel: U8, 3 => first_casualties: I64, 4 => second_casualties: I64, 5 => first_unknown_casualties: I64, 6 => second_unknown_casualties: I64, 7 => first_was_instigator: U8}, |_row| Ok(()));
row_codec!(FactionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => name: Bytes, 1 => template_token: OptionalU32, 2 => color_red: U32, 3 => color_green: U32, 4 => color_blue: U32, 5 => color_alpha: U32, 6 => color_overridden: U8}, |_row| Ok(()));
row_codec!(FactionMemberRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => faction: U64, 1 => ordinal: U32}, |_row| Ok(()));
row_codec!(PlayerRow, 32, |row| row.id as u32 as u128, |key| { id: key as u32 as i32 }, {0 => country: U32, 1 => flags: U8, 2 => user: Bytes, 3 => name: Bytes}, |_row| Ok(()));
row_codec!(HumanControllerRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => count: I8}, |_row| Ok(()));
row_codec!(LandCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => duration: I32, 2 => day: I32, 3 => attackers: U64List, 4 => defenders: U64List, 5 => attacker_classes: U32List, 6 => defender_classes: U32List}, |_row| Ok(()));
row_codec!(FleetRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => home_base: U32, 2 => name: Opaque, 3 => leader: OptionalU64}, |_row| Ok(()));
row_codec!(TaskForceRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => fleet: U64, 1 => roster_owner: U32, 2 => logical_owner: U32, 3 => expeditionary_owner: U32, 4 => province: U32, 5 => mission: U32, 6 => route_count: I32, 7 => combat_references: I32}, |_row| Ok(()));
row_codec!(ShipRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => task_force: U64, 1 => strength: I64, 2 => organisation: I64, 3 => experience: I64 @ Self::EXPERIENCE_STEP, 4 => category: U32, 5 => sunk_convoys: I32, 6 => class_id: I32, 7 => acquired_manpower: OptionalI32, 8 => required_manpower: OptionalI32, 9 => name: Opaque, 10 => naming: Opaque}, |_row| Ok(()));
row_codec!(NavalCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => attacker_positioning: I64, 2 => defender_positioning: I64, 3 => attacker_anti_air: I64, 4 => defender_anti_air: I64, 5 => registry_ordinal: U32}, |_row| Ok(()));
row_codec!(NavalCombatParticipantRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.battle_slot as u128, |key| { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32 }, {0 => ship: U64, 1 => owner: U32, 2 => state: U32}, |row| if row.side > 1 { Err(invalid("naval combat participant side exceeds one")) } else { Ok(()) });
row_codec!(AirWingRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => air_base: U64, 2 => base_override: U64, 3 => country_override: I32, 4 => planes: I32, 5 => mission_mask: U32, 6 => mission_region: OptionalU32, 7 => base_range_raw: I64, 8 => ace: U64, 9 => mission_active: U8, 10 => eligibility_veto: U8, 11 => eligibility_counter: I32, 12 => eligibility_limit: I32, 13 => attached_orders_group: OptionalU64, 14 => pool_ordinal: U32, 15 => name: Opaque, 16 => pool_subunit: OptionalU32, 17 => raid_reference: Opaque, 18 => strategic_bombing_raw: OptionalI64, 19 => acquired_manpower: OptionalI32, 20 => exile_country: OptionalI32, 21 => replacement_setting: Opaque, 22 => map_flags: OptionalU32, 23 => fuel_base_raw: OptionalI64, 24 => fuel_current_type: OptionalI32, 25 => fuel_effective_aircraft: OptionalI32, 26 => fuel_context_mode: OptionalU32, 27 => fuel_capacity_class: OptionalI32, 28 => fuel_override_resolved: OptionalU32}, |_row| Ok(()));
row_codec!(ArmyOrderRow, 96, |row| (row.owner_group as u128) << 32 | row.instance as u128, |key| { owner_group: (key >> 32) as u64, instance: key as u32 }, {0 => country: U32, 1 => order_type: I32, 2 => active: U8, 3 => paradrop_target: U32, 4 => path: U32List, 5 => target_states: U32List}, |_row| Ok(()));

pub fn encode_delta(
    table: TableId,
    delta: &DeltaRow,
    previous_key: u128,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    if delta.recorded_fields != table.field_count() || delta.format != crate::FORMAT_VERSION {
        return Err(invalid("historical fields cannot be re-encoded"));
    }
    table.check_key(delta.key)?;
    table.check_mask(delta.mask)?;
    let mut decoder = Decoder::new(&delta.values);
    for (index, kind) in table.schema().iter().enumerate() {
        if delta.mask & (1_u64 << index) != 0 {
            kind.decode_change(&mut decoder)?;
        }
    }
    decoder.finish()?;
    table.encode_key(delta.key, previous_key, output)?;
    allocation::reserve(
        output,
        allocation::add(table.mask_width(), delta.values.len())?,
    )?;
    output.extend_from_slice(&delta.mask.to_le_bytes()[..table.mask_width()]);
    output.extend_from_slice(&delta.values);
    Ok(())
}

pub fn decode_delta(
    table: TableId,
    previous_key: u128,
    decoder: &mut Decoder<'_>,
) -> io::Result<DeltaRow> {
    // A decoded change admits only its canonical encoding, so the recorded
    // bytes are the values encoding them again would write.
    let (key, mask, fields, values) = decode_delta_with(table, previous_key, decoder)?;
    let values = allocation::copy_vec(values)?;
    #[cfg(any(test, feature = "test-support"))]
    delta_decode_ownership::record();
    Ok(DeltaRow {
        format: decoder.format,
        key,
        mask,
        values,
        recorded_fields: fields,
    })
}

/// Validate the recorded delta fields without copying their borrowed values.
/// Typed nested validation and baseline arithmetic remain reconstruction work.
pub fn validate_delta_row(
    table: TableId,
    previous_key: u128,
    decoder: &mut Decoder<'_>,
) -> io::Result<u128> {
    decode_delta_with(table, previous_key, decoder).map(|(key, ..)| key)
}

/// One recorded delta row's key, mask and field count, every field change
/// validated, and the bytes of those changes.
fn decode_delta_with<'a>(
    table: TableId,
    previous_key: u128,
    decoder: &mut Decoder<'a>,
) -> io::Result<(u128, u64, usize, &'a [u8])> {
    let key = table.decode_key(previous_key, decoder)?;
    let mut mask_bytes = [0; 8];
    let fields = table.last_change().fields(table, decoder.format);
    let width = fields.div_ceil(8);
    mask_bytes[..width].copy_from_slice(decoder.take(width)?);
    let mask = u64::from_le_bytes(mask_bytes);
    if mask == 0 || mask & !full_mask(fields) != 0 {
        return Err(invalid("invalid recorded delta mask"));
    }
    let values = decoder.rest();
    for (index, kind) in table.schema().iter().enumerate() {
        if mask & (1_u64 << index) != 0 {
            kind.decode_change(decoder)?;
        }
    }
    Ok((
        key,
        mask,
        fields,
        &values[..values.len() - decoder.remaining()],
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub mod delta_decode_ownership {
    use std::cell::Cell;

    thread_local! {
        static OWNED_ROWS: Cell<Option<usize>> = const { Cell::new(None) };
    }

    pub(super) fn record() {
        OWNED_ROWS.with(|count| {
            if let Some(value) = count.get() {
                count.set(Some(value + 1));
            }
        });
    }

    /// Observe just this thread and restore the previous observation on panic.
    pub fn count<T>(read: impl FnOnce() -> T) -> (T, usize) {
        struct Reset(Option<usize>);
        impl Drop for Reset {
            fn drop(&mut self) {
                OWNED_ROWS.with(|count| count.set(self.0));
            }
        }
        let _reset = Reset(OWNED_ROWS.with(|count| count.replace(Some(0))));
        let result = read();
        let count = OWNED_ROWS.with(|count| count.get().unwrap());
        (result, count)
    }
}

#[derive(Clone, Copy)]
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
    fn minimum_row_bytes(self) -> usize {
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
    fn minimum_change_bytes(self) -> usize {
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
    fn encoded_bytes(self) -> io::Result<usize> {
        allocation::add(
            4,
            match self {
                Self::Encoded(bytes, _) => bytes.len(),
                Self::Native(value) => value.encoded_bytes()?,
            },
        )
    }
    fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(output, self.encoded_bytes()?)?;
        self.encode_reserved(output)
    }
    fn encode_reserved(self, output: &mut Vec<u8>) -> io::Result<()> {
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
    pub(crate) fn apply(self, kind: Kind, previous: Field<'a>) -> io::Result<Field<'a>> {
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
    fn recorded_bytes(self) -> io::Result<usize> {
        match self {
            Self::Opaque(Opaque::Native(value)) => allocation::add(4, value.recorded_bytes()?),
            _ => self.encoded_bytes(),
        }
    }
    fn encoded_bytes(self) -> io::Result<usize> {
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
    fn encoded_bound(self) -> io::Result<usize> {
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
    fn number(self) -> io::Result<i128> {
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
    /// The enclosing row reserved every field's snapshot bound already.
    fn encode_reserved(self, output: &mut Vec<u8>) -> io::Result<()> {
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
// `opaque_value!(T, restored)` marks a whole field restored after a
// provisional retirement (`NativeValue::RESTORED`).
macro_rules! opaque_value {
    ($ty:ty, restored) => {
        opaque_value!($ty, true);
    };
    ($ty:ty) => {
        opaque_value!($ty, false);
    };
    ($ty:ty, $restored:literal) => {
        $crate::rows::checkpoint::opaque!($ty);
        impl NativeValue for $ty {
            const HAS_STEPS: bool = <$ty as Value>::HAS_STEPS;
            const RESTORED: bool = $restored;
            fn heap_bytes(&self) -> usize {
                <$ty as Value>::heap_bytes(self)
            }
            fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                Value::visit_field_gaps(self, visit);
            }
            fn round_display(&mut self) {
                Value::round_display(self);
            }
            fn round_to(&mut self, step: $crate::display::Step) {
                Value::round_to(self, step);
            }
            fn to_units(&self, step: $crate::display::Step) -> io::Result<Self> {
                Value::to_units(self, step)
            }
            fn from_units(self, step: $crate::display::Step) -> io::Result<Self> {
                Value::from_units(self, step)
            }
            fn try_clone(&self) -> io::Result<Self> {
                Value::try_clone(self)
            }
            fn field(&self) -> Field<'_> {
                Field::Opaque(Opaque::Native(self))
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                let Field::Opaque(Opaque::Encoded(bytes, format)) = field else {
                    return Err(invalid("structured field requires encoded native bytes"));
                };
                measurement::count(Counter::NativeOpaqueBytes, bytes.len());
                let mut input = Decoder::new(bytes);
                input.format = format;
                let value = <$ty as Value>::decode(&mut input)?;
                input.finish()?;
                Ok(value)
            }
        }
    };
}

opaque_value!(crate::event_feed_rows::EventFeedCoverage);
opaque_value!(Option<crate::event_feed_rows::EventFeedCounts>);
opaque_value!(Option<ShipNamingOperands>);
opaque_value!(Vec<crate::event_feed_rows::EventFeedEvent>);
opaque_value!(Vec<crate::event_feed_rows::EventFeedScope>);

/// A field borrows either decoded native values or checked wire bytes. Decoding a
/// list allocates only when constructing its owning row, never a temporary list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum List<'a, T> {
    Native(&'a [T]),
    Encoded(&'a [u8]),
}

macro_rules! native_list {
    ($ty:ty, $width:expr) => {
        impl List<'_, $ty> {
            fn to_vec(self) -> io::Result<Vec<$ty>> {
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
            fn encoded_bytes(self) -> io::Result<usize> {
                allocation::add(
                    4,
                    match self {
                        Self::Native(values) => allocation::mul(values.len(), $width)?,
                        Self::Encoded(bytes) => bytes.len(),
                    },
                )
            }

            fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
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

#[cfg(test)]
#[path = "rows/display_tests.rs"]
mod display_tests;

#[cfg(test)]
#[path = "rows/supply_footer_tests.rs"]
mod supply_footer_tests;

#[cfg(test)]
#[path = "rows/loss_sum_tests.rs"]
mod loss_sum_tests;

#[cfg(test)]
#[path = "rows/land_modifier_tests.rs"]
mod land_modifier_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patrol_disruption_delta_does_not_repeat_quiet_combat_inputs() {
        let previous = AirMissionInputRow {
            wing: 7,
            patrol_disruption_raw: (Some(17)).into(),
            patrol_attack_raw: (Some(i64::MAX)).into(),
            patrol_defence_raw: (Some(i64::MIN)).into(),
            patrol_speed_raw: (Some(-13)).into(),
            patrol_wing_attack_raw: (Some(-19)).into(),
            patrol_wing_defence_raw: (Some(23)).into(),
            ..Default::default()
        };
        let next = AirMissionInputRow {
            patrol_disruption_raw: (Some(-29)).into(),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 13);
        assert_eq!(
            delta.apply::<AirMissionInputRow>(Some(previous)).unwrap(),
            next
        );
        round_trip(TableId::AirMissionInputs, next);
    }

    #[test]
    fn patrol_held_marker_delta_is_independent_of_disruption_and_combat() {
        let previous = AirMissionInputRow {
            wing: 7,
            patrol_disruption_raw: (Some(11)).into(),
            patrol_attack_raw: (Some(13)).into(),
            patrol_defence_raw: (Some(17)).into(),
            patrol_night_applied_raw: (Some(-100_000)).into(),
            ..Default::default()
        };
        let next = AirMissionInputRow {
            patrol_night_applied_raw: (Some(50_001)).into(),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 19);
        assert_eq!(
            delta.apply::<AirMissionInputRow>(Some(previous)).unwrap(),
            next
        );
        round_trip(TableId::AirMissionInputs, next);
    }

    #[test]
    fn patrol_air_detection_numeric_columns_delta_independently_of_execution_and_normalization() {
        let previous = AirMissionInputRow {
            wing: 7,
            current_type: (Some(8)).into(),
            effective_aircraft: (Some(13)).into(),
            detection_stat_13: (Some(i64::MIN)).into(),
            detection_stat_14: (Some(i64::MAX)).into(),
            ..Default::default()
        };
        let next = AirMissionInputRow {
            detection_stat_13: (Some(-123_456)).into(),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 11);
        assert_eq!(
            delta.apply::<AirMissionInputRow>(Some(previous)).unwrap(),
            next
        );
        round_trip(TableId::AirMissionInputs, next);
    }

    #[test]
    fn wing_fuel_count_delta_does_not_repeat_quiet_statistic_or_training_fields() {
        let previous = AirWingRow {
            id: 7,
            fuel_base_raw: Some(i64::MIN).into(),
            fuel_current_type: Some(4).into(),
            fuel_effective_aircraft: Some(81).into(),
            fuel_context_mode: Some(2).into(),
            fuel_capacity_class: None.into(),
            fuel_override_resolved: None.into(),
            ..Default::default()
        };
        let next = AirWingRow {
            fuel_effective_aircraft: Some(80).into(),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 25);
        assert_eq!(delta.apply::<AirWingRow>(Some(previous)).unwrap(), next);
        assert_eq!(next.fuel_inputs().unwrap().base_fuel, Some(i64::MIN));
        assert_eq!(next.fuel_inputs().unwrap().effective_aircraft, Some(80));
        round_trip(TableId::AirWings, next);
    }

    #[test]
    fn wing_fuel_accessor_preserves_independent_absence_and_rejects_invalid_domains() {
        let mut row = AirWingRow {
            fuel_base_raw: None.into(),
            fuel_current_type: Some(0).into(),
            fuel_effective_aircraft: None.into(),
            fuel_context_mode: None.into(),
            fuel_capacity_class: Some(-1).into(),
            fuel_override_resolved: Some(0).into(),
            ..Default::default()
        };
        let input = row.fuel_inputs().unwrap();
        assert_eq!(input.base_fuel, None);
        assert_eq!(input.current_type, Some(0));
        assert_eq!(input.capacity_class, Some(-1));
        assert_eq!(input.override_resolved, Some(false));
        row.fuel_override_resolved = Some(2).into();
        assert!(row.fuel_inputs().is_err());
        row.fuel_override_resolved = Some(1).into();
        row.fuel_context_mode = Some(256).into();
        assert!(row.fuel_inputs().is_err());
    }

    #[test]
    fn air_grid_flag_pairs_keep_raid_training_and_coverage_independently_unknown() {
        for packed in 0..64 {
            let row = AirWingRow {
                map_flags: Some(packed).into(),
                ..Default::default()
            };
            for (shift, value) in [
                (0, row.training_stop()),
                (2, row.attachment_coverage()),
                (4, row.assigned_to_raid()),
            ] {
                assert_eq!(
                    value,
                    (packed & (1 << shift) != 0).then_some(packed & (2 << shift) != 0)
                );
            }
        }
        for map_flags in [Recorded::NotRecorded, Recorded::Value(None)] {
            let row = AirWingRow {
                map_flags,
                ..Default::default()
            };
            assert_eq!(row.assigned_to_raid(), None);
        }
    }

    #[test]
    fn validated_copy_does_not_repeat_admission_validation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CHECKS: AtomicUsize = AtomicUsize::new(0);
        #[derive(Clone, Default, PartialEq, Eq)]
        struct CountedRow {
            key: u32,
            text: Vec<u8>,
        }
        typed_codec!(CountedRow, 32, |row| row.key as u128,
        |key| Self { key: key as u32, ..Default::default() },
        { 0 => (text): Bytes }, |row| {
            CHECKS.fetch_add(1, Ordering::Relaxed);
            if row.text.is_empty() { return Err(invalid("synthetic missing text")); }
            Ok(())
        });
        let row = CountedRow {
            key: 1,
            text: b"synthetic".to_vec(),
        };
        CHECKS.store(0, Ordering::Relaxed);
        row.check_key_fields().unwrap();
        let copy = row.try_clone_validated().unwrap();
        assert_eq!(copy.text, row.text);
        assert_eq!(CHECKS.load(Ordering::Relaxed), 1);
        assert!(CountedRow::default().try_clone().is_err());
    }

    #[test]
    fn table_ids_are_dense_unique_and_round_trip() {
        for (position, table) in TableId::ALL.into_iter().enumerate() {
            assert_eq!(table as usize, position);
            assert_eq!(table.index(), position);
            assert_eq!(TableId::try_from(table as u16).unwrap(), table);
        }
        assert!(TableId::try_from(TableId::ALL.len() as u16).is_err());
    }

    #[test]
    fn peace_operands_failures_and_omissions_round_trip() {
        let row = PeaceSettlementRow {
            sequence: u64::MAX,
            conference: Some(0x700000009),
            hour: Some(91),
            winners: vec![0xf0000021],
            losers: vec![0xa0000023],
            actions: vec![SettledPeaceAction {
                token: u32::MAX,
                taker: 0xf0000021,
                giver: 0xa0000023,
                negotiator: 0xf0000030,
                status: 2,
                cost: -7,
                state: Some(9234),
                flags: Some([1, 2, 3, 4]),
                ..Default::default()
            }],
            transfers: vec![SettledPeaceTransfer {
                state: 9234,
                taker: 0xf0000021,
                automatic: true,
            }],
            subjects: vec![SettledPeaceSubject {
                overlord: 0xf0000021,
                subject: 0xa0000023,
            }],
            failure: Some(b"first guarded fault".to_vec()),
            omitted: 8,
        };
        round_trip(TableId::PeaceSettlements, row.clone());
        round_trip(TableId::PeaceBids, row);
    }

    #[test]
    fn peace_success_and_present_empty_failure_are_distinct_on_the_wire() {
        let success = PeaceSettlementRow {
            conference: Some(7),
            hour: Some(91),
            ..Default::default()
        };
        let failure = PeaceSettlementRow {
            failure: Some(Vec::new()),
            ..success.clone()
        };
        round_trip(TableId::PeaceSettlements, success.clone());
        round_trip(TableId::PeaceSettlements, failure.clone());
        pair(TableId::PeaceSettlements, success, failure);
    }

    /// A battle keeps its coordinates' exact bits (negative zero and a NaN
    /// payload included), its members in order with repeats, and its
    /// sounding flag; a moved battle stores only its changed fields.
    #[test]
    fn battle_sounds_round_trip_exact_bits_and_member_order() {
        let row = BattleSoundRow {
            ordinal: u32::MAX,
            center_x: (-0.0_f32).to_bits(),
            center_z: 0x7fc0_1234,
            radius: f32::MAX.to_bits(),
            provinces: vec![13_000, 7, 13_000, u32::MAX],
            sounding: true,
            object: 0x0000_7ff6_1234_5678,
        };
        round_trip(TableId::BattleSounds, row.clone());
        round_trip(TableId::BattleSounds, BattleSoundRow::default());
        let moved = BattleSoundRow {
            center_x: 1.5_f32.to_bits(),
            ..row.clone()
        };
        pair(TableId::BattleSounds, row.clone(), moved.clone());
        let delta = moved.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, 1);
        assert_eq!(delta.apply(Some(row.clone())).unwrap(), moved);
        assert_eq!(BattleSoundRow::default().key(), 0);
        assert_eq!(row.key(), u32::MAX as u128);
    }

    #[test]
    fn nuclear_strike_outcomes_and_refusals_round_trip_and_validate() {
        let row = NuclearStrikeRow {
            id: 41,
            hour: -7,
            launcher: crate::event_feed_rows::EventFeedCountry {
                reference: 4_999,
                id: 4_998,
                instance: u32::MAX,
            },
            nuke_type: 1,
            province: 13_000,
            state: 0,
            owner: crate::event_feed_rows::EventFeedCountry {
                reference: 0,
                id: 0,
                instance: 7,
            },
            controller: 77,
            war_support: vec![NuclearWarSupport {
                country: 0,
                before_raw: i64::MIN,
                after_raw: i64::MAX,
            }],
            buildings: vec![NuclearBuildingDamage {
                state: 0,
                province: Some(13_000),
                controller: 77,
                building_type: 0xf004,
                amount_raw: -1,
            }],
            units: vec![NuclearUnitDamage {
                handle: (8 << 32) | 0x42,
                class: 13,
                owner: 5,
                strength_before: 100_000,
                strength_after: 0,
                organisation_before: None,
                organisation_after: None,
                manpower_before: 250,
                manpower_after: 0,
            }],
            equipment: vec![NuclearEquipmentLoss {
                country: 3,
                wing: true,
                variant: u64::MAX,
                amount_raw: 300_000,
            }],
            refusals: vec![NuclearStrikeRefusal {
                part: NuclearStrikePart::Units,
                reason: b"nuclear strike unit class unsupported".to_vec(),
                count: 2,
            }],
            wings: vec![NuclearWingLoss {
                country: 3,
                manpower_before: 400,
                manpower_after: 0,
            }],
        };
        assert!(row.complete(NuclearStrikePart::Buildings));
        assert!(!row.complete(NuclearStrikePart::Units));
        round_trip(TableId::NuclearStrikes, row.clone());
        round_trip(TableId::NuclearStrikes, NuclearStrikeRow::default());
        let mut uncounted = row.clone();
        uncounted.refusals[0].count = 0;
        assert!(uncounted.check_key_fields().is_err());
        let mut half = row;
        half.units[0].organisation_before = Some(1);
        assert!(half.check_key_fields().is_err());
    }

    fn round_trip<T: RowCodec + std::fmt::Debug>(table: TableId, row: T) {
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<T>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
        for cut in 0..bytes.len() {
            assert!(decode_row::<T>(0, &mut Decoder::new(&bytes[..cut])).is_err());
        }
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.mask, table.full_mask());
        assert_eq!(delta.apply::<T>(None).unwrap(), row);
        bytes.clear();
        encode_delta(table, &delta, 0, &mut bytes).unwrap();
        let mut streamed = Vec::new();
        assert!(row.encode_change(None, 0, &mut streamed).unwrap());
        assert_eq!(streamed, bytes);
        assert!(
            !row.encode_change(Some(&row), row.key(), &mut streamed)
                .unwrap()
        );
        assert_eq!(streamed, bytes, "unchanged rows must write nothing");
        let mut input = Decoder::new(&bytes);
        let restored = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(restored, delta);
        assert_eq!(restored.apply::<T>(None).unwrap(), row);
    }
    fn pair<T: RowCodec + std::fmt::Debug>(table: TableId, low: T, high: T) {
        for (old, new) in [(&low, &high), (&high, &low)] {
            let delta = new.delta(Some(old)).unwrap().unwrap();
            let mut bytes = Vec::new();
            encode_delta(table, &delta, 0, &mut bytes).unwrap();
            let mut streamed = Vec::new();
            assert!(new.encode_change(Some(old), 0, &mut streamed).unwrap());
            assert_eq!(
                streamed, bytes,
                "direct output differs from the independent delta codec"
            );
            let mut input = Decoder::new(&bytes);
            let restored = decode_delta(table, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(restored.apply(Some(old.clone())).unwrap(), *new);
            for cut in 0..bytes.len() {
                assert!(decode_delta(table, 0, &mut Decoder::new(&bytes[..cut])).is_err());
            }
        }
    }
    fn country() -> CountryRow {
        CountryRow {
            border_land_doctrine_level: crate::rows::Recorded::Value(None),
            id: 7,
            canonical_slot: 1,
            tag: vec![b'G', 0xff, b'r'],
            political_power_raw: Some(i64::MIN),
            command_power_raw: -9_007_199_254_740_993,
            stability_raw: 100_001,
            war_support_raw: 99_999,
            convoys_raw: Some(i64::MAX),
            peace_conference_participant: u8::MAX,
            manpower_residual: i32::MIN,
        }
    }

    #[test]
    fn streamed_merge_matches_independent_changes_and_deletions() {
        let row = |id, owner| StateRow {
            id,
            owner,
            ..StateRow::default()
        };
        let cases = [
            (vec![], vec![]),
            (vec![], vec![row(2, 9), row(u32::MAX, 7)]),
            (vec![row(1, 9), row(4, 7)], vec![]),
            (
                vec![row(1, 9), row(3, 1), row(4, 7)],
                vec![row(2, 9), row(3, 2), row(4, 7), row(8, 6)],
            ),
            (vec![row(3, 2)], vec![row(3, 2)]),
        ];
        let mut scratch = Vec::new();
        for (old, new) in cases {
            let changed = new
                .iter()
                .filter_map(|row| {
                    row.delta(old.iter().find(|before| before.id == row.id))
                        .unwrap()
                })
                .collect::<Vec<_>>();
            let deleted = old
                .iter()
                .filter(|before| !new.iter().any(|row| row.id == before.id))
                .map(RowCodec::key)
                .collect::<Vec<_>>();
            let mut expected = (changed.len() as u32).to_le_bytes().to_vec();
            let mut previous_key = 0;
            for row in &changed {
                encode_delta(TableId::States, row, previous_key, &mut expected).unwrap();
                previous_key = row.key;
            }
            expected.extend_from_slice(&(deleted.len() as u32).to_le_bytes());
            previous_key = 0;
            for key in &deleted {
                TableId::States
                    .encode_key(*key, previous_key, &mut expected)
                    .unwrap();
                previous_key = *key;
            }
            let mut actual = Vec::new();
            assert_eq!(
                encode_changes_typed(&new, &old, &mut actual, &mut scratch).unwrap(),
                !changed.is_empty() || !deleted.is_empty()
            );
            assert_eq!(actual, expected);
        }
    }
    fn division() -> DivisionRow {
        DivisionRow {
            id: 0x8000_0001_0000_0029,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 123,
            template: 0x8000_0002_0000_002a,
            // Whole display steps (crate::display), as capture stores them.
            hit_points: -9_007_199_254_740_000,
            organisation: 100_000,
            orders_group: 0x0000_0039_0000_0035,
            reveal_skip: 0,
        }
    }
    fn war_relation() -> WarRelationRow {
        WarRelationRow {
            first_country: 7,
            second_country: 19,
            start_hour: i32::MIN,
            end_hour: i32::MAX,
            cancel: 0,
            first_casualties: i64::MIN,
            second_casualties: i64::MAX,
            first_unknown_casualties: i64::MIN,
            second_unknown_casualties: i64::MAX,
            first_was_instigator: 0,
        }
    }
    #[test]
    fn new_division_operands_apply_through_current_masks() {
        let previous = LandCombatDivisionRow {
            combat: 7,
            side: 0,
            ordinal: 1,
            ..LandCombatDivisionRow::default()
        };
        let next = LandCombatDivisionRow {
            direction_group: i32::MIN,
            exile: Some(crate::rows::LandCombatExileInputs {
                origin_type: 2,
                location_cores: vec![u32::MAX],
                ..Default::default()
            }),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 27) | (1 << 28));
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
        let mut bytes = Vec::new();
        encode_delta(TableId::LandCombatDivisions, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let restored = decode_delta(TableId::LandCombatDivisions, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(restored.apply(Some(previous)).unwrap(), next);
    }

    #[test]
    fn full_rows_and_insertions_preserve_native_values_and_raw_aliases() {
        round_trip(
            TableId::Clock,
            ClockRow {
                hour: i32::MIN,
                play_time: u64::MAX,
                play_session_started_at: i64::MIN,
                seed: i32::MAX,
                speed: i32::MIN,
                debug_current_ref_id: Some(u32::MAX),
                unit: Some(i32::MIN),
                order_index: Some(i32::MAX),
                front_index: Some(-1),
                theatre_index: Some(0),
                theater_group_index: None,
                military_deployment_line_index: Some(7841),
                military_deployment_conveyor_index: Some(744),
                equipment_variant_index: Some(6295),
                country_leader_index: Some(1),
                navy_id: Some(8263),
                land_combat_id: Some(15_359),
                multiplayer_random_count: Some(i32::MIN),
            },
        );
        round_trip(
            TableId::Campaign,
            CampaignRow {
                id: vec![0xff, 0, b'a'],
            },
        );
        round_trip(TableId::Countries, country());
        round_trip(
            TableId::CountryLinks,
            CountryLinkRow {
                id: 2,
                canonical_slot: 1,
            },
        );
        round_trip(
            TableId::Provinces,
            ProvinceRow {
                id: 123,
                state: 456,
                owner: 7,
                controller: 2,
                victory_points: i32::MIN,
                fort_positive: 1,
                move_gate: 0,
                port_sea: 0,
                attack: Default::default(),
                selected_areas: Default::default(),
                fort_bunker: Recorded::Value(Some(-25_000)),
                controller_area: Recorded::Value(Some(u32::MAX)),
                owner_area: Recorded::Value(None),
                area_node: Recorded::Value(Some(ControllerAreaNode {
                    first_land: None,
                    neighbours: Some(vec![u32::MAX, 0, u32::MAX]),
                })),
            },
        );
        round_trip(
            TableId::States,
            StateRow {
                manpower_locked: Default::default(),

                id: u32::MAX,
                owner: 0,
                controller: u32::MAX,
                movement_infrastructure: i64::MIN,
                railway_infrastructure: Some(i64::MAX).into(),
                manpower_available: i32::MIN,
                population: i32::MAX.into(),
                category: Recorded::Value(Some(u32::MAX)),
                custom_name: vec![0xff, 0, b'a'],
                owner_name: "Département".into(),
                continent: Recorded::Value(Some(u32::MAX)),

                best_vp_index: Default::default(),

                single_controller: Default::default(),
            },
        );
        round_trip(
            TableId::Divisions,
            DivisionRow {
                id: u64::MAX,
                roster_owner: u32::MAX,
                logical_owner: 0,
                expeditionary_owner: u32::MAX,
                province: u32::MAX,
                template: u64::MAX,
                hit_points: i64::MIN,
                organisation: i64::MAX,
                orders_group: u64::MAX,
                reveal_skip: 0,
            },
        );
        round_trip(
            TableId::WarRelations,
            WarRelationRow {
                first_country: u32::MAX,
                second_country: u32::MAX,
                cancel: u8::MAX,
                first_was_instigator: 2,
                ..war_relation()
            },
        );
        round_trip(
            TableId::Factions,
            FactionRow {
                id: 0,
                name: Vec::new(),
                ..FactionRow::default()
            },
        );
        round_trip(
            TableId::Factions,
            FactionRow {
                id: u64::MAX,
                name: vec![0, 0xff, b'A'],
                template_token: Some(u32::MAX),
                color_red: f32::NAN.to_bits(),
                color_green: 0,
                color_blue: u32::MAX,
                color_alpha: 1.0_f32.to_bits(),
                color_overridden: u8::MAX,
            },
        );
        round_trip(
            TableId::Factions,
            FactionRow {
                id: 7,
                name: Vec::new(),
                template_token: Some(0),
                ..FactionRow::default()
            },
        );
        round_trip(
            TableId::FactionMembers,
            FactionMemberRow {
                country: u32::MAX,
                faction: u64::MAX,
                ordinal: u32::MAX,
            },
        );
    }

    #[test]
    fn province_masks_keep_exact_victory_points_independent_of_control() {
        let previous = ProvinceRow {
            id: u32::MAX,
            state: 7,
            owner: 19,
            controller: 0,
            victory_points: i32::MIN,
            fort_positive: 0,
            move_gate: 0,
            port_sea: 0,
            attack: Default::default(),
            selected_areas: Default::default(),
            fort_bunker: Recorded::Value(None),
            controller_area: Default::default(),
            owner_area: Default::default(),
            area_node: Default::default(),
        };
        assert_eq!(TableId::Provinces.full_mask(), 8191);
        // Keep the independently covered legacy seven-field mask domain.
        for mask in 1..=127 {
            let next = ProvinceRow {
                id: previous.id,
                state: if mask & 1 != 0 {
                    u32::MAX
                } else {
                    previous.state
                },
                owner: if mask & 2 != 0 { 0 } else { previous.owner },
                controller: if mask & 4 != 0 {
                    u32::MAX
                } else {
                    previous.controller
                },
                victory_points: if mask & 8 != 0 {
                    i32::MAX
                } else {
                    previous.victory_points
                },
                fort_positive: if mask & 16 != 0 {
                    1
                } else {
                    previous.fort_positive
                },
                move_gate: if mask & 32 != 0 {
                    u8::MAX
                } else {
                    previous.move_gate
                },
                port_sea: if mask & 64 != 0 {
                    i32::MIN
                } else {
                    previous.port_sea
                },
                attack: previous.attack.clone(),
                selected_areas: previous.selected_areas,
                fort_bunker: previous.fort_bunker,
                controller_area: previous.controller_area,
                owner_area: previous.owner_area,
                area_node: previous.area_node.clone(),
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut wire = Vec::new();
            encode_delta(TableId::Provinces, &delta, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            let restored = decode_delta(TableId::Provinces, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(restored.apply(Some(previous.clone())).unwrap(), next);
            assert_eq!(
                previous
                    .delta(Some(&next))
                    .unwrap()
                    .unwrap()
                    .apply(Some(next.clone()))
                    .unwrap(),
                previous
            );
            round_trip(TableId::Provinces, next);
        }
        for victory_points in [i32::MIN, -1, 0, 50, i32::MAX] {
            let row = ProvinceRow {
                victory_points,
                ..previous.clone()
            };
            let mut wire = Vec::new();
            encode_row(&row, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            assert_eq!(input.varint().unwrap(), row.key());
            let fields = wire.len() - input.remaining();
            // Victory points remain after the three legacy country/state words,
            // independently of the newly appended attack operand envelope.
            assert_eq!(
                &wire[fields + 12..fields + 16],
                &victory_points.to_le_bytes()
            );
            round_trip(TableId::Provinces, row);
        }
    }

    /// Labels, a null area, a refused label, a refused node operand and a
    /// known empty contact list are each their own value, field by field.
    #[test]
    fn province_areas_keep_labels_null_and_node_refusals_distinct() {
        let base = ProvinceRow {
            id: 17,
            controller: 701,
            owner: 702,
            controller_area: Recorded::Value(Some(17)),
            owner_area: Recorded::Value(Some(0)),
            area_node: Recorded::Value(Some(ControllerAreaNode {
                first_land: Some(u32::MAX),
                neighbours: Some(vec![29, 29, 0, u32::MAX]),
            })),
            ..Default::default()
        };
        round_trip(TableId::Provinces, base.clone());
        for change in 0..7 {
            let mut next = base.clone();
            fn node(row: &mut ProvinceRow) -> &mut ControllerAreaNode {
                match &mut row.area_node {
                    Recorded::Value(Some(node)) => node,
                    _ => unreachable!(),
                }
            }
            match change {
                0 => next.controller_area = Recorded::Value(None),
                1 => next.controller_area = Recorded::Value(Some(0)),
                2 => next.owner_area = Recorded::Value(None),
                3 => next.area_node = Recorded::Value(None),
                4 => node(&mut next).first_land = None,
                5 => node(&mut next).neighbours = None,
                _ => node(&mut next).neighbours = Some(Vec::new()),
            }
            assert_ne!(next, base, "{change}");
            let delta = next.delta(Some(&base)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << [10, 10, 11, 12, 12, 12, 12][change]);
            let mut wire = Vec::new();
            encode_delta(TableId::Provinces, &delta, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            let restored = decode_delta(TableId::Provinces, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(restored.apply(Some(base.clone())).unwrap(), next);
            round_trip(TableId::Provinces, next);
        }
    }

    #[test]
    fn map_rows_preserve_full_native_handles_signed_values_and_ordered_lists() {
        round_trip(
            TableId::Players,
            PlayerRow {
                id: i32::MIN,
                country: u32::MAX,
                flags: u8::MAX,
                user: vec![0xff, 0],
                name: vec![0, 0xfe],
            },
        );
        round_trip(
            TableId::HumanControllers,
            HumanControllerRow {
                country: u32::MAX,
                count: i8::MIN,
            },
        );
        round_trip(
            TableId::LandCombats,
            LandCombatRow {
                id: u64::MAX,
                province: u32::MAX,
                duration: i32::MIN,
                day: i32::MAX,
                attackers: vec![u64::MAX, 0, 1_u64 << 63],
                defenders: vec![1, u64::MAX],
                attacker_classes: vec![0, 13, u32::MAX],
                defender_classes: vec![1, 0],
            },
        );
        round_trip(
            TableId::Fleets,
            FleetRow {
                id: u64::MAX,
                owner: u32::MAX,
                home_base: u32::MAX,
                name: Recorded::Value(Some(b"synthetic fleet".to_vec())),
                leader: Recorded::Value(Some(u64::MAX)),
            },
        );
        round_trip(
            TableId::TaskForces,
            TaskForceRow {
                id: u64::MAX,
                fleet: u64::MAX,
                roster_owner: u32::MAX,
                logical_owner: 0,
                expeditionary_owner: u32::MAX,
                province: u32::MAX,
                mission: u32::MAX,
                route_count: 0,
                combat_references: i32::MIN,
            },
        );
        round_trip(
            TableId::Ships,
            ShipRow {
                name: Default::default(),
                naming: Default::default(),
                acquired_manpower: Default::default(),
                required_manpower: Default::default(),

                id: u64::MAX,
                task_force: u64::MAX,
                strength: i64::MIN,
                organisation: i64::MAX,
                experience: -9_007_199_254_741_000,
                category: u32::MAX,
                sunk_convoys: i32::MIN,
                class_id: i32::MIN,
            },
        );
        round_trip(
            TableId::NavalCombats,
            NavalCombatRow {
                id: u64::MAX,
                province: u32::MAX,
                attacker_positioning: i64::MIN,
                defender_positioning: i64::MAX,
                attacker_anti_air: -9_007_199_254_740_993,
                defender_anti_air: 9_007_199_254_740_993,
                registry_ordinal: u32::MAX,
            },
        );
        round_trip(
            TableId::NavalCombatParticipants,
            NavalCombatParticipantRow {
                combat: u64::MAX,
                side: 1,
                battle_slot: u32::MAX,
                ship: u64::MAX,
                owner: u32::MAX,
                state: u32::MAX,
            },
        );
        round_trip(
            TableId::AirWings,
            AirWingRow {
                acquired_manpower: Default::default(),
                exile_country: Default::default(),

                id: u64::MAX,
                owner: u32::MAX,
                air_base: u64::MAX,
                base_override: 1_u64 << 63,
                country_override: i32::MIN,
                planes: i32::MIN,
                mission_mask: u32::MAX,
                mission_region: Some(u32::MAX),
                base_range_raw: i64::MIN,
                ace: u64::MAX,
                mission_active: u8::MAX,
                eligibility_veto: 1,
                eligibility_counter: i32::MIN,
                eligibility_limit: i32::MAX,
                attached_orders_group: Some(u64::MAX),
                pool_ordinal: u32::MAX,
                name: Recorded::Value(Some(vec![0xff, 0, b'x'])),
                strategic_bombing_raw: Recorded::Value(Some(i64::MIN)),
                replacement_setting: Recorded::Value(Some(u8::MAX)),
                map_flags: Recorded::Value(Some(15)),
                fuel_base_raw: Some(i64::MIN).into(),
                fuel_current_type: Some(i32::MIN).into(),
                fuel_effective_aircraft: Some(i32::MAX).into(),
                fuel_context_mode: Some(u32::from(u8::MAX)).into(),
                fuel_capacity_class: Some(i32::MIN).into(),
                fuel_override_resolved: Some(0).into(),
                pool_subunit: Recorded::Value(Some(u32::MAX)),
                raid_reference: Recorded::Value(Some(AirWingRaidReference {
                    raw: u64::MAX,
                    resolves: false,
                })),
            },
        );
        round_trip(
            TableId::ArmyOrders,
            ArmyOrderRow {
                owner_group: u64::MAX,
                instance: u32::MAX,
                country: u32::MAX,
                order_type: i32::MIN,
                active: u8::MAX,
                paradrop_target: u32::MAX,
                path: vec![u32::MAX, 0, 1_u32 << 31],
                target_states: vec![0, u32::MAX],
            },
        );
    }

    #[test]
    fn one_native_scalar_tick_costs_four_bytes() {
        let old = country();
        let mut new = old.clone();
        new.stability_raw += 1;
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        let delta = new.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 4);
        assert_eq!(delta.values, [2]);
        let mut bytes = Vec::new();
        encode_delta(TableId::Countries, &delta, 0, &mut bytes).unwrap();
        assert_eq!(bytes, [7, 0x10, 0, 2]);
        assert_eq!(delta.apply::<CountryRow>(Some(old.clone())).unwrap(), new);
    }

    #[test]
    fn state_masks_keep_owner_and_controller_as_independent_raw_references() {
        let old = StateRow {
            id: 0x1234_5678,
            owner: 7,
            controller: 2,
            movement_infrastructure: 100_000,
            ..StateRow::default()
        };
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        for mask in 1..=TableId::States.full_mask() {
            let new = StateRow {
                manpower_locked: Recorded::Value(if mask & 1024 != 0 { -1 } else { 0 }),

                id: old.key().try_into().unwrap(),
                manpower_available: if mask & 32 != 0 { i32::MAX } else { 0 },
                continent: Recorded::Value(if mask & 64 != 0 { Some(123) } else { None }),
                owner: if mask & 1 != 0 { 0 } else { 7 },
                controller: if mask & 2 != 0 { u32::MAX } else { 2 },
                movement_infrastructure: if mask & 4 != 0 { 100_001 } else { 100_000 },
                railway_infrastructure: (if mask & 128 != 0 { Some(0) } else { None }).into(),
                population: (if mask & 256 != 0 { -1 } else { 0 }).into(),
                category: Recorded::Value(if mask & 512 != 0 { Some(123) } else { None }),
                custom_name: if mask & 8 != 0 {
                    b"Rename".to_vec()
                } else {
                    Vec::new()
                },
                owner_name: if mask & 16 != 0 {
                    b"Owner".to_vec()
                } else {
                    Vec::new()
                },

                best_vp_index: (if mask & 2048 != 0 { Some(-1) } else { None }).into(),
                single_controller: (if mask & 4096 != 0 { Some(1) } else { None }).into(),
            };
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::States, &delta, 0, &mut bytes).unwrap();
            // Thirteen fields take a two-byte mask.
            let scalars = 5
                + 2
                + usize::from(mask & 1 != 0)
                + 5 * usize::from(mask & 2 != 0)
                + usize::from(mask & 4 != 0)
                + 5 * usize::from(mask & 32 != 0)
                + 3 * usize::from(mask & 64 != 0)
                + 2 * usize::from(mask & 128 != 0)
                + usize::from(mask & 256 != 0)
                + 3 * usize::from(mask & 512 != 0)
                + usize::from(mask & 1024 != 0)
                + 2 * usize::from(mask & 2048 != 0)
                + 2 * usize::from(mask & 4096 != 0);
            // The name fields add their own length-prefixed bytes.
            let names = 6 * usize::from(mask & 8 != 0) + 5 * usize::from(mask & 16 != 0);
            if names == 0 {
                assert_eq!(bytes.len(), scalars);
            } else {
                assert!(bytes.len() > scalars + names);
            }
            assert_eq!(&bytes[..5], &[0xf8, 0xac, 0xd1, 0x91, 0x01]);
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::States, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(decoded.apply::<StateRow>(Some(old.clone())).unwrap(), new);
            if mask != TableId::States.full_mask() {
                assert!(decoded.apply::<StateRow>(None).is_err());
            }
        }
    }

    #[test]
    fn division_rows_keep_full_handles_and_store_health_in_display_steps() {
        for id in [0, 1_u64 << 63, u64::MAX] {
            let row = DivisionRow { id, ..division() };
            let mut bytes = Vec::new();
            encode_row(&row, 0, &mut bytes).unwrap();
            let key_len = if id == 0 { 1 } else { 10 };
            assert_eq!(bytes.len(), key_len + 49);
            assert_eq!(
                &bytes[key_len + 16..key_len + 24],
                &division().template.to_le_bytes()
            );
            // Hit points and organisation are stored in tenths.
            assert_eq!(
                &bytes[key_len + 24..key_len + 32],
                &(division().hit_points / 10_000).to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 32..key_len + 40],
                &(division().organisation / 10_000).to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 40..key_len + 48],
                &division().orders_group.to_le_bytes()
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<DivisionRow>(0, &mut input).unwrap(), row);
            input.finish().unwrap();
            for cut in 0..bytes.len() {
                assert!(decode_row::<DivisionRow>(0, &mut Decoder::new(&bytes[..cut])).is_err());
            }
            let delta = row.delta(None).unwrap().unwrap();
            bytes.clear();
            encode_delta(TableId::Divisions, &delta, 0, &mut bytes).unwrap();
            assert_eq!(bytes.len(), key_len + 2 + delta.values.len());
            assert_eq!(&bytes[key_len..key_len + 2], &[0xff, 1]);
            assert_eq!(delta.apply::<DivisionRow>(None).unwrap(), row);
        }
    }

    #[test]
    fn division_masks_preserve_every_field_subset() {
        let original = division();
        let old = original.clone();
        for mask in 1..=TableId::Divisions.full_mask() {
            let mut updated = original.clone();
            if mask & 1 != 0 {
                updated.roster_owner = u32::MAX;
            }
            if mask & 2 != 0 {
                updated.logical_owner = 0;
            }
            if mask & 4 != 0 {
                updated.expeditionary_owner = 7;
            }
            if mask & 8 != 0 {
                updated.province = u32::MAX;
            }
            if mask & 16 != 0 {
                updated.template = u64::MAX;
            }
            if mask & 32 != 0 {
                updated.hit_points = i64::MIN;
            }
            if mask & 64 != 0 {
                updated.organisation = i64::MAX;
            }
            if mask & 128 != 0 {
                updated.orders_group = u64::MAX;
            }
            if mask & 256 != 0 {
                updated.reveal_skip = 255;
            }
            let new = updated;
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::Divisions, &delta, 0, &mut bytes).unwrap();
            assert_eq!(bytes.len(), 10 + 2 + delta.values.len());
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::Divisions, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(
                decoded.apply::<DivisionRow>(Some(old.clone())).unwrap(),
                new
            );
            assert_eq!(
                old.delta(Some(&new))
                    .unwrap()
                    .unwrap()
                    .apply::<DivisionRow>(Some(new.clone()))
                    .unwrap(),
                old
            );
            if mask != TableId::Divisions.full_mask() {
                assert!(decoded.apply::<DivisionRow>(None).is_err());
            }
        }
    }

    #[test]
    fn war_relation_keys_keep_ordered_endpoints_out_of_the_payload() {
        let mut keys = Vec::new();
        for (first_country, second_country) in [
            (0, 0),
            (0, u32::MAX),
            (7, 19),
            (19, 7),
            (u32::MAX, 0),
            (u32::MAX, u32::MAX),
        ] {
            let expected = WarRelationRow {
                first_country,
                second_country,
                cancel: 2,
                first_was_instigator: 255,
                ..war_relation()
            };
            let row = expected.clone();
            assert_eq!(
                row.key(),
                (u128::from(first_country) << 32) | u128::from(second_country)
            );
            keys.push(row.key());
            let mut bytes = Vec::new();
            encode_row(&row, 0, &mut bytes).unwrap();
            let mut fields = Decoder::new(&bytes);
            assert_eq!(fields.varint().unwrap(), row.key());
            assert_eq!(fields.remaining(), 42);
            assert_eq!(fields.i32().unwrap(), expected.start_hour);
            assert_eq!(fields.i32().unwrap(), expected.end_hour);
            assert_eq!(fields.u8().unwrap(), 2);
            assert_eq!(fields.i64().unwrap(), expected.first_casualties);
            assert_eq!(fields.i64().unwrap(), expected.second_casualties);
            assert_eq!(fields.i64().unwrap(), expected.first_unknown_casualties);
            assert_eq!(fields.i64().unwrap(), expected.second_unknown_casualties);
            assert_eq!(fields.u8().unwrap(), 255);
            fields.finish().unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<WarRelationRow>(0, &mut input).unwrap(), row);
            input.finish().unwrap();
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn war_relation_masks_preserve_each_native_field_and_raw_byte_value() {
        let original = war_relation();
        let old = original.clone();
        for mask in 1..=TableId::WarRelations.full_mask() {
            let mut updated = original.clone();
            if mask & 1 != 0 {
                updated.start_hour = i32::MAX;
            }
            if mask & 2 != 0 {
                updated.end_hour = i32::MIN;
            }
            if mask & 4 != 0 {
                updated.cancel = 255;
            }
            if mask & 8 != 0 {
                updated.first_casualties = i64::MAX;
            }
            if mask & 16 != 0 {
                updated.second_casualties = i64::MIN;
            }
            if mask & 32 != 0 {
                updated.first_unknown_casualties = i64::MAX;
            }
            if mask & 64 != 0 {
                updated.second_unknown_casualties = i64::MIN;
            }
            if mask & 128 != 0 {
                updated.first_was_instigator = 2;
            }
            let new = updated;
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut wire = Vec::new();
            encode_delta(TableId::WarRelations, &delta, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            let decoded = decode_delta(TableId::WarRelations, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(
                decoded.apply::<WarRelationRow>(Some(old.clone())).unwrap(),
                new
            );
            assert_eq!(
                old.delta(Some(&new))
                    .unwrap()
                    .unwrap()
                    .apply::<WarRelationRow>(Some(new.clone()))
                    .unwrap(),
                old
            );
        }
        for value in 0..=u8::MAX {
            let new = WarRelationRow {
                cancel: value,
                first_was_instigator: u8::MAX - value,
                ..original.clone()
            };
            let mut wire = Vec::new();
            encode_row(&new, 0, &mut wire).unwrap();
            assert_eq!(
                decode_row::<WarRelationRow>(0, &mut Decoder::new(&wire)).unwrap(),
                new
            );
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(
                delta.apply::<WarRelationRow>(Some(old.clone())).unwrap(),
                new
            );
        }
    }

    #[test]
    fn field_masks_handle_every_subset_and_option_transitions() {
        let old = country();
        for mask in 1_u64..=TableId::Countries.full_mask() {
            let mut new = old.clone();
            if mask & 1 != 0 {
                new.canonical_slot = 3;
            }
            if mask & 2 != 0 {
                new.tag = vec![0xfe];
            }
            if mask & 4 != 0 {
                new.political_power_raw = None;
            }
            if mask & 8 != 0 {
                new.command_power_raw = i64::MAX;
            }
            if mask & 16 != 0 {
                new.stability_raw = i64::MIN;
            }
            if mask & 32 != 0 {
                new.war_support_raw = -1;
            }
            if mask & 64 != 0 {
                new.convoys_raw = None;
            }
            if mask & 128 != 0 {
                new.peace_conference_participant = 0;
            }
            if mask & 256 != 0 {
                new.manpower_residual = i32::MAX;
            }
            if mask & 512 != 0 {
                new.border_land_doctrine_level = crate::rows::Recorded::Value(Some(i32::MIN));
            }
            let old = old.clone();
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            assert_eq!(delta.apply::<CountryRow>(Some(old.clone())).unwrap(), new);
            let reverse = old.delta(Some(&new)).unwrap().unwrap();
            assert_eq!(reverse.apply::<CountryRow>(Some(new.clone())).unwrap(), old);
        }
    }

    #[test]
    fn signed_byte_and_optional_u32_changes_keep_every_bit_and_presence() {
        let mut previous = HumanControllerRow {
            country: 7,
            count: i8::MIN,
        };
        for count in (i8::MIN..=i8::MAX).rev() {
            let next = HumanControllerRow { country: 7, count };
            if let Some(delta) = next.delta(Some(&previous)).unwrap() {
                assert_eq!(
                    delta
                        .apply::<HumanControllerRow>(Some(previous.clone()))
                        .unwrap(),
                    next
                );
            }
            previous = next;
        }
        let mut previous = AirWingRow::default();
        for region in [Some(u32::MAX), Some(0), None, Some(0), Some(u32::MAX), None] {
            let next = AirWingRow {
                mission_region: region,
                ..AirWingRow::default()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 6);
            assert_eq!(
                delta.apply::<AirWingRow>(Some(previous.clone())).unwrap(),
                next
            );
            previous = next;
        }
        for (kind, previous, difference) in [
            (Kind::I8, Field::I8(i8::MAX), 1),
            (Kind::I8, Field::I8(i8::MIN), -1),
            (Kind::OptionalU32, Field::OptionalU32(Some(u32::MAX)), 1),
            (Kind::OptionalU32, Field::OptionalU32(Some(0)), -1),
        ] {
            assert!(Change::Number(difference).apply(kind, previous).is_err());
        }
    }

    #[test]
    fn stored_air_wing_name_presence_and_rename_delta_preserve_bytes() {
        let mut previous = AirWingRow {
            id: 0x123400000045,
            ..Default::default()
        };
        for name in [
            Recorded::Value(Some(Vec::new())),
            Recorded::Value(Some(vec![0xff, 0, b'X'])),
            Recorded::Value(None),
        ] {
            let next = AirWingRow {
                name,
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 15);
            assert_eq!(delta.apply::<AirWingRow>(Some(previous)).unwrap(), next);
            round_trip(TableId::AirWings, next.clone());
            previous = next;
        }
        let history = TableId::AirWings.last_change();
        assert_eq!(history.fields(TableId::AirWings, 98), 15);
        assert_eq!(history.fields(TableId::AirWings, 99), 16);
    }

    #[test]
    fn transport_operand_deltas_preserve_null_fault_full_handle_and_resolution() {
        let mut previous = AirWingRow {
            id: 0x1234_0000_0045,
            pool_subunit: Recorded::Value(None),
            raid_reference: Recorded::Value(None),
            ..Default::default()
        };
        for pool_subunit in [Some(0), Some(u32::MAX), None] {
            let next = AirWingRow {
                pool_subunit: Recorded::Value(pool_subunit),
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 16);
            assert_eq!(delta.apply::<AirWingRow>(Some(previous)).unwrap(), next);
            round_trip(TableId::AirWings, next.clone());
            previous = next;
        }
        for raid_reference in [
            Some(AirWingRaidReference {
                raw: 0,
                resolves: false,
            }),
            Some(AirWingRaidReference {
                raw: u64::MAX,
                resolves: false,
            }),
            Some(AirWingRaidReference {
                raw: u64::MAX,
                resolves: true,
            }),
            None,
        ] {
            let next = AirWingRow {
                raid_reference: Recorded::Value(raid_reference),
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 17);
            let mut wire = Vec::new();
            encode_delta(TableId::AirWings, &delta, 0, &mut wire).unwrap();
            let decoded = decode_delta(TableId::AirWings, 0, &mut Decoder::new(&wire)).unwrap();
            assert_eq!(decoded.apply::<AirWingRow>(Some(previous)).unwrap(), next);
            previous = next;
        }
    }

    #[test]
    fn air_scenario_operand_snapshots_and_deltas_keep_zero_fault_and_signed_words() {
        let mut wing = AirWingRow {
            id: 7,
            ..Default::default()
        };
        let mut mission = AirMissionInputRow {
            wing: 7,
            ..Default::default()
        };
        for value in [Some(0), Some(i64::MIN), Some(i64::MAX), None] {
            let next = AirWingRow {
                strategic_bombing_raw: value.into(),
                ..wing.clone()
            };
            let delta = next.delta(Some(&wing)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 18);
            assert_eq!(delta.apply::<AirWingRow>(Some(wing)).unwrap(), next);
            round_trip(TableId::AirWings, next.clone());
            wing = next;
            let next = AirMissionInputRow {
                cas_ratio_raw: value.into(),
                ..mission.clone()
            };
            let delta = next.delta(Some(&mission)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 6);
            assert_eq!(
                delta.apply::<AirMissionInputRow>(Some(mission)).unwrap(),
                next
            );
            round_trip(TableId::AirMissionInputs, next.clone());
            mission = next;
        }
        let mut history = AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index: 7,
            enemy_ships_sunk_raw: 1,
            ..Default::default()
        };
        for values in [
            Some([i64::MIN, i64::MAX, -1, 0, 3, 44, 65]),
            Some([0; 7]),
            None,
        ] {
            let next = AirNavalPatrolHistoryRow {
                scenario_channels: values.into(),
                ..history.clone()
            };
            let delta = next.delta(Some(&history)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 2);
            assert_eq!(
                delta
                    .apply::<AirNavalPatrolHistoryRow>(Some(history))
                    .unwrap(),
                next
            );
            round_trip(TableId::AirNavalPatrolHistories, next.clone());
            history = next;
        }
        let witness = AirNavalPatrolHistoryRow::availability_witness(5000);
        assert!(witness.is_availability_witness());
        assert!(!witness.has_values());
        round_trip(TableId::AirNavalPatrolHistories, witness);
        let refused = AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index: 7,
            scenario_channels: Recorded::Value(None),
            ..Default::default()
        };
        assert!(refused.has_values());
        round_trip(TableId::AirNavalPatrolHistories, refused.clone());
        let zero = AirNavalPatrolHistoryRow {
            scenario_channels: Recorded::Value(Some([0; 7])),
            ..refused.clone()
        };
        assert!(!zero.has_values());
        let mut bytes = Vec::new();
        zero.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        for kind in &AirNavalPatrolHistoryRow::SCHEMA[..2] {
            kind.decode(&mut input).unwrap();
        }
        let prefix = &bytes[..bytes.len() - input.remaining()];
        let mut input = Decoder::new(prefix);
        input.format = compatibility::WIDGET_CAPTURE_FORMAT;
        input.recorded_fields = 2;
        let older = AirNavalPatrolHistoryRow::decode_fields(zero.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(older.scenario_channels, Recorded::NotRecorded);
        assert!(!older.has_values());
        assert_ne!(older, refused);
        assert_ne!(older, zero);
    }

    #[test]
    fn list_changes_replace_exact_order_and_refuse_truncated_or_hostile_lengths() {
        let mut previous = LandCombatRow::default();
        for attackers in [vec![u64::MAX, 1], vec![1, u64::MAX], vec![], vec![0, 0]] {
            let next = LandCombatRow {
                attackers,
                ..LandCombatRow::default()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 3);
            assert_eq!(
                delta
                    .apply::<LandCombatRow>(Some(previous.clone()))
                    .unwrap(),
                next
            );
            previous = next;
        }
        let mut previous = ArmyOrderRow::default();
        for path in [vec![u32::MAX, 1], vec![1, u32::MAX], vec![], vec![0, 0]] {
            let next = ArmyOrderRow {
                path,
                ..ArmyOrderRow::default()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 4);
            assert_eq!(
                delta.apply::<ArmyOrderRow>(Some(previous.clone())).unwrap(),
                next
            );
            previous = next;
        }
        for (kind, width) in [(Kind::U32List, 4), (Kind::U64List, 8)] {
            assert!(
                kind.decode(&mut Decoder::new(&u32::MAX.to_le_bytes()))
                    .is_err()
            );
            let mut bytes = 2_u32.to_le_bytes().to_vec();
            bytes.resize(4 + 2 * width - 1, 0);
            assert!(kind.decode_change(&mut Decoder::new(&bytes)).is_err());
        }
    }

    #[test]
    fn numeric_differences_preserve_the_full_signed_and_unsigned_spans() {
        pair(
            TableId::Clock,
            ClockRow {
                hour: i32::MIN,
                play_time: 0,
                play_session_started_at: i64::MIN,
                seed: i32::MIN,
                speed: i32::MIN,
                debug_current_ref_id: Some(0),
                unit: Some(i32::MIN),
                order_index: None,
                front_index: Some(i32::MAX),
                theatre_index: Some(0),
                theater_group_index: Some(-1),
                military_deployment_line_index: None,
                military_deployment_conveyor_index: Some(0),
                equipment_variant_index: Some(0),
                country_leader_index: Some(0),
                navy_id: Some(0),
                land_combat_id: Some(0),
                multiplayer_random_count: Some(i32::MAX),
            },
            ClockRow {
                hour: i32::MAX,
                play_time: u64::MAX,
                play_session_started_at: i64::MAX,
                seed: i32::MAX,
                speed: i32::MAX,
                debug_current_ref_id: Some(u32::MAX),
                unit: Some(i32::MAX),
                order_index: Some(i32::MIN),
                front_index: None,
                theatre_index: Some(-1),
                theater_group_index: None,
                military_deployment_line_index: Some(i32::MIN),
                military_deployment_conveyor_index: Some(1),
                equipment_variant_index: Some(-1),
                country_leader_index: Some(1),
                navy_id: Some(1),
                land_combat_id: Some(1),
                multiplayer_random_count: Some(i32::MIN),
            },
        );
        pair(
            TableId::Divisions,
            DivisionRow {
                template: 0,
                hit_points: i64::MIN,
                organisation: i64::MAX,
                ..division()
            },
            DivisionRow {
                template: u64::MAX,
                hit_points: i64::MAX,
                organisation: i64::MIN,
                ..division()
            },
        );
    }

    #[test]
    fn numeric_changes_refuse_out_of_range_differences_and_reconstructed_values() {
        fn reject<T: RowCodec>(table: TableId, previous: T, mask: u64, values: Vec<u8>) {
            let change = DeltaRow {
                format: crate::FORMAT_VERSION,
                recorded_fields: table.field_count(),
                key: previous.key(),
                mask,
                values,
            };
            let mut bytes = Vec::new();
            encode_delta(table, &change, 0, &mut bytes).unwrap();
            assert!(change.apply(Some(previous.clone())).is_err());
            let restored = decode_delta(table, 0, &mut Decoder::new(&bytes)).unwrap();
            assert!(restored.apply(Some(previous)).is_err());
        }
        reject(
            TableId::Clock,
            ClockRow {
                play_time: u64::MAX,
                ..ClockRow::default()
            },
            2,
            vec![2],
        );
        reject(TableId::Clock, ClockRow::default(), 2, vec![1]);
        reject(
            TableId::Clock,
            ClockRow {
                seed: i32::MAX,
                ..ClockRow::default()
            },
            8,
            vec![2],
        );
        reject(
            TableId::Clock,
            ClockRow {
                play_session_started_at: i64::MIN,
                ..ClockRow::default()
            },
            4,
            vec![1],
        );
        reject(
            TableId::States,
            StateRow {
                owner: u32::MAX,
                ..StateRow::default()
            },
            1,
            vec![2],
        );
        reject(
            TableId::Countries,
            CountryRow {
                political_power_raw: Some(i64::MAX),
                ..country()
            },
            4,
            vec![1, 2],
        );
        reject(
            TableId::WarRelations,
            WarRelationRow {
                cancel: u8::MAX,
                ..war_relation()
            },
            4,
            vec![2],
        );
        reject(TableId::WarRelations, war_relation(), 128, vec![1]);
        for (table, mask, difference) in [
            (TableId::WarRelations, 4, 1_u128 << 8),
            (TableId::States, 1, 1_u128 << 32),
            (TableId::Clock, 2, 1_u128 << 64),
        ] {
            let mut values = Vec::new();
            put_varint(&mut values, difference * 2);
            let delta = DeltaRow {
                format: crate::FORMAT_VERSION,
                recorded_fields: table.field_count(),
                key: 0,
                mask,
                values: values.clone(),
            };
            assert!(encode_delta(table, &delta, 0, &mut Vec::new()).is_err());
            let mut wire = vec![0, mask as u8];
            wire.extend(values);
            assert!(decode_delta(table, 0, &mut Decoder::new(&wire)).is_err());
        }
    }

    #[test]
    fn native_composite_keys_preserve_full_handles_side_and_slot() {
        let order = ArmyOrderRow {
            owner_group: u64::MAX,
            instance: u32::MAX,
            ..ArmyOrderRow::default()
        };
        let participant = NavalCombatParticipantRow {
            combat: u64::MAX,
            side: 1,
            battle_slot: u32::MAX,
            ..NavalCombatParticipantRow::default()
        };
        assert_eq!(order.key(), (1_u128 << 96) - 1);
        assert_eq!(participant.key(), (1_u128 << 97) - 1);
        round_trip(TableId::ArmyOrders, order);
        round_trip(TableId::NavalCombatParticipants, participant);
        let invalid = NavalCombatParticipantRow {
            side: 2,
            ..NavalCombatParticipantRow::default()
        };
        assert!(invalid.delta(None).is_err());
        assert!(encode_row(&invalid, 0, &mut Vec::new()).is_err());
        let mut unchanged = vec![91, 92];
        assert!(invalid.encode_change(None, 0, &mut unchanged).is_err());
        let valid = NavalCombatParticipantRow::default();
        assert_eq!(
            valid
                .encode_change(Some(&invalid), 0, &mut unchanged)
                .unwrap_err()
                .to_string(),
            valid.delta(Some(&invalid)).unwrap_err().to_string(),
        );
        assert_eq!(unchanged, [91, 92]);
        let attacker = NavalCombatParticipantRow {
            combat: 0xffff_ffff_0000_0001,
            battle_slot: u32::MAX,
            ..NavalCombatParticipantRow::default()
        };
        let defender = NavalCombatParticipantRow {
            side: 1,
            ..attacker.clone()
        };
        let other_type = NavalCombatParticipantRow {
            combat: 0xffff_ffff_0000_0002,
            ..attacker.clone()
        };
        assert_ne!(attacker.key(), defender.key());
        assert_ne!(attacker.key(), other_type.key());
    }

    #[test]
    fn native_key_widths_full_width_guard_and_masks_refuse_overflow() {
        assert!(check_native_key(u128::MAX, 128).is_ok());
        assert_eq!(full_mask(64), u64::MAX);
        for table in TableId::ALL {
            let key = if table.key_bits() == 128 {
                u128::MAX
            } else {
                (1_u128 << table.key_bits()) - 1
            };
            let mut bytes = Vec::new();
            table.encode_key(key, 0, &mut bytes).unwrap();
            assert_eq!(
                bytes.len(),
                if key == 0 {
                    1
                } else {
                    table.key_bits().div_ceil(7) as usize
                }
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(table.decode_key(0, &mut input).unwrap(), key);
            input.finish().unwrap();
            if table.key_bits() < 128 {
                for oversized in [1_u128 << table.key_bits(), u128::MAX] {
                    let before = bytes.clone();
                    assert!(table.encode_key(oversized, 0, &mut bytes).is_err());
                    assert_eq!(bytes, before);
                }
            }
            let mut one = Vec::new();
            put_varint(&mut one, 1);
            assert!(
                table
                    .decode_key(u128::MAX, &mut Decoder::new(&one))
                    .is_err()
            );
        }
    }

    #[test]
    fn malformed_deltas_cannot_invent_baselines_or_ignore_suffixes() {
        let old = country();
        let mut new = old.clone();
        new.stability_raw += 1;
        let valid = new.delta(Some(&old)).unwrap().unwrap();
        assert!(valid.apply::<CountryRow>(None).is_err());
        let other = CountryRow {
            id: 99,
            ..country()
        };
        assert!(valid.apply(Some(other)).is_err());
        for mask in [0, TableId::Countries.full_mask() + 1] {
            let bad = DeltaRow {
                format: crate::FORMAT_VERSION,
                recorded_fields: TableId::Countries.field_count(),
                mask,
                ..valid.clone()
            };
            assert!(bad.apply(Some(old.clone())).is_err());
            assert!(encode_delta(TableId::Countries, &bad, 0, &mut Vec::new()).is_err());
        }
        let mut suffix = valid.clone();
        suffix.values.push(0);
        assert!(suffix.apply(Some(old.clone())).is_err());
        assert!(encode_delta(TableId::Countries, &suffix, 0, &mut Vec::new()).is_err());
        let mut missing = valid;
        missing.values.clear();
        assert!(missing.apply(Some(old)).is_err());
        assert!(decode_row::<ClockRow>(0, &mut Decoder::new(&[1; 25])).is_err());
        assert!(decode_row::<CampaignRow>(0, &mut Decoder::new(&[1, 0, 0, 0, 0])).is_err());
    }

    #[test]
    fn scalar_deltas_move_unchanged_strings_and_ordered_lists() {
        let old = country();
        let tag = old.tag.as_ptr();
        let mut next = old.clone();
        next.command_power_raw += 1;
        let delta = next.delta(Some(&old)).unwrap().unwrap();
        let restored = delta.apply(Some(old)).unwrap();
        assert_eq!(restored, next);
        assert_eq!(restored.tag.as_ptr(), tag);
        let old = LandCombatRow {
            attackers: vec![u64::MAX, 0, 7],
            defenders: vec![9, 1],
            ..LandCombatRow::default()
        };
        let attackers = old.attackers.as_ptr();
        let defenders = old.defenders.as_ptr();
        let mut next = old.clone();
        next.duration = 1;
        let delta = next.delta(Some(&old)).unwrap().unwrap();
        let restored = delta.apply(Some(old)).unwrap();
        assert_eq!(restored, next);
        assert_eq!(restored.attackers.as_ptr(), attackers);
        assert_eq!(restored.defenders.as_ptr(), defenders);
    }
}

#[cfg(test)]
#[path = "rows/format99/tests.rs"]
pub(crate) mod format99_tests;

#[cfg(test)]
#[path = "rows/political_runtime/tests.rs"]
mod political_runtime_tests;

#[cfg(test)]
#[path = "rows/intel_inputs/tests.rs"]
pub(crate) mod intel_inputs_tests;

#[test]
fn raid_memo_table_metadata_uses_the_dispatcher_key() {
    assert_eq!(TableId::RaidPathDistances.key_bits(), 65);
    let land = RaidPathDistanceRow {
        source: u32::MAX,
        target: u32::MAX,
        ..Default::default()
    };
    let naval = RaidPathDistanceRow {
        dispatcher: RaidPathDispatcher::Naval,
        ..land.clone()
    };
    assert_eq!(land.key(), u64::MAX as u128);
    assert_eq!(naval.key(), (1u128 << 65) - 1);
}

#[cfg(test)]
#[path = "rows/ship_names_tests.rs"]
mod ship_names_tests;
#[cfg(test)]
#[path = "rows/visit_wire_tests.rs"]
mod visit_wire_tests;
