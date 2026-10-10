//! Division object; all fields use its primary pointer. The hourly walker
//! retains full handles and i64 hit points and organisation. Candidate fields
//! with unknown widths remain identified below. Shared stat-block, ownership,
//! location, map-entity and modifier fields are in cunit.

use crate::modifier_keys as keys;
use crate::{
    Memory, carmy_group, ccountry, cgame_state, corders_group, cpdx_array::Array, cprovince,
    creferenced_division_template, cunit, read, walk::Outcome,
};

pub mod country_movement_inputs;

/// Primary division vtable.
pub const VTABLE_RVA: usize = 0x0294_3ad0;
/// Secondary division vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0294_3cb0;
/// Secondary subobject displacement.
pub const SECONDARY_BASE: usize = 0x10;
/// Low-supply counter input, i64 fixed5.
pub const COUNTER_SUPPLY_STATUS: usize = 0x40;
/// Byte gating the yellow invalid-orders glyph.
pub const INVALID_ORDERS: usize = 0x24e;
/// Fixed5 factor multiplying the plain strength ratio before clamping to
/// zero through 100000. Both counter and committed-combat strength bars use
/// this product.
pub const STRENGTH_FACTOR: usize = 0x468;
/// Disrupted-supply candidate; only i32 is characterized and full width is unknown.
pub const DISRUPTED_SUPPLY_CANDIDATE: usize = 0xb0;
/// Orders-group member backlink, authenticated reciprocally against the group.
pub const ORDERS_GROUP: usize = 0xc0;
/// Primary order-instance pointer.
pub const PRIMARY_ORDER: usize = 0xd8;
/// Maximum-organisation getter virtual byte offset.
pub const MAXIMUM_ORGANISATION_VTABLE_SLOT: usize = 0x130;
/// Movement target pointer, tested for presence.
pub const MOVEMENT_TARGET: usize = 0x1e8;
/// Source-province pointer requiring province authentication.
pub const MOVEMENT_SOURCE: usize = 0x1f8;
/// Movement numerator, i64.
pub const MOVEMENT_NUMERATOR: usize = 0x240;
/// Raw retreat byte.
pub const RETREAT: usize = 0x24c;
/// Strategic-redeployment byte.
pub const STRATEGIC_REDEPLOYMENT: usize = 0x2af;
/// Government-in-exile byte. Raid edges accept exile or their independent
/// second gate. Exile permits a different controller area when state owners agree.
pub const EXILED: usize = 0x2ae;
/// Naval-transfer membership pointer, also selecting the counter-supply override.
pub const NAVAL_TRANSFER_MEMBERSHIP: usize = 0x2f8;
/// Deployment handle type, u32.
pub const DEPLOYMENT_TYPE: usize = 0x32c;
/// Deployment handle id, u32.
pub const DEPLOYMENT_ID: usize = 0x330;
/// Embedded equipment-variant pool.
pub const EQUIPMENT_POOL: usize = 0x348;
/// Equipment-bank vector data.
pub const EQUIPMENT_DATA: usize = 0x368;
/// Equipment-bank capacity, u32.
pub const EQUIPMENT_CAPACITY: usize = 0x370;
/// Equipment-bank count, u32.
pub const EQUIPMENT_COUNT: usize = 0x374;
/// Standard referenced-template pointer used by serialization.
pub const STANDARD_TEMPLATE: usize = 0x3b8;
/// Separate fake-intelligence template pointer.
pub const FAKE_INTEL_TEMPLATE: usize = 0x3c8;
/// Embedded manpower-values object.
pub const MANPOWER_VALUES: usize = 0x3d8;
pub const MANPOWER_DATA: usize = MANPOWER_VALUES + crate::carmy_manpower_values::DATA;
/// Manpower-entry count, i32; reductions preserve wrapping arithmetic.
pub const MANPOWER_COUNT: usize = MANPOWER_VALUES + crate::carmy_manpower_values::COUNT;
/// Current hit points, i64 fixed5.
pub const STRENGTH: usize = 0x420;
/// Organisation, i64 fixed5.
pub const ORGANISATION: usize = 0x428;
/// Experience, i64 fixed5.
pub const EXPERIENCE: usize = 0x430;
/// Total attrition, i64 fixed5.
pub const ATTRITION_TOTAL: usize = 0x448;
/// Entrenchment, i64 fixed5.
pub const ENTRENCHMENT: usize = 0x458;
/// Entrenchment cap, i64 fixed5.
pub const ENTRENCHMENT_CAP: usize = 0x460;
/// Attrition-icon input, separate from total attrition.
pub const COUNTER_ATTRITION: usize = 0x438;
/// Icon threshold, i64, tested after applying the country modifier.
pub const COUNTER_ATTRITION_THRESHOLD_RVA: usize = 0x0331_c5d8;
/// Maximum exercise level define, i32.
pub const EXERCISE_MAXIMUM_LEVEL_RVA: usize = 0x0331_9b50;
/// Minimum effective strength fraction define, i64.
pub const EXERCISE_MINIMUM_STRENGTH_RVA: usize = 0x0331_9be0;
/// Ordered i64 experience-threshold array.
pub const EXPERIENCE_THRESHOLDS_RVA: usize = 0x0332_0590;
/// Requests-object pointer selecting the serialized upgrade group.
pub const REQUESTS: usize = 0x478;
/// Retained land model number, i32, selected from priority subunit and
/// equipment. Recalculation runs once per day in the division's staggered hour
/// and on template changes, deployment and creation. Between updates retain
/// the displayed value; model suffix selection uses its sign-extended low byte.
pub const RETAINED_VISUAL_VARIANT: usize = 0x4b0;
/// Embedded acclimatization object requiring vtable authentication.
pub const ACCLIMATIZATION: usize = 0x4b8;
/// Signed fixed5 maximum grace hours, serialized unconditionally.
pub const MAXIMUM_SUPPLY_GRACE: usize = 0x600;
/// Supply-gain candidate; only u32 is characterized and full width is unknown.
pub const SUPPLY_GAIN_CANDIDATE: usize = 0x608;
/// Signed fixed5 remaining grace hours, serialized unconditionally including zero.
pub const REMAINING_SUPPLY_GRACE: usize = 0x610;
/// Fuel candidate; only u32 is characterized and full width is unknown.
pub const FUEL_CANDIDATE: usize = 0x620;
/// Current fuel, i64.
pub const CURRENT_FUEL: usize = 0x628;
/// Raw counter-separation byte.
pub const SEPARATE_COUNTER: usize = 0x630;
/// Cumulative-loss candidate; only i32 is characterized and full width is unknown.
pub const CUMULATIVE_LOSSES_CANDIDATE: usize = 0x678;
/// Deployment-handle resolver; both-zero handles short-circuit.
pub const RESOLVE_DEPLOYMENT_RVA: usize = 0x00be_c560;

// Attrition event-site descriptors do not install hooks.
/// Attrition entry checkpoint.
pub const ATTRITION_ENTRY_RVA: usize = 0x00C6_5DC0;
/// Attrition entry instruction sentinel.
pub const ATTRITION_ENTRY_SENTINEL: [u8; 7] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08];
/// Attrition entry continuation; observe before the displaced instruction.
pub const ATTRITION_ENTRY_CONTINUATION_RVA: usize = 0x00C6_5DC7;
/// Attrition upkeep checkpoint.
pub const ATTRITION_UPKEEP_RVA: usize = 0x00C6_5E3C;
/// Attrition upkeep instruction sentinel.
pub const ATTRITION_UPKEEP_SENTINEL: [u8; 7] = [0x48, 0x8b, 0x83, 0xb8, 0x00, 0x00, 0x00];
/// Attrition upkeep continuation; observe before the displaced instruction.
pub const ATTRITION_UPKEEP_CONTINUATION_RVA: usize = 0x00C6_5E43;
/// Attrition terrain checkpoint.
pub const ATTRITION_TERRAIN_RVA: usize = 0x00C6_5ED7;
/// Attrition terrain instruction sentinel.
pub const ATTRITION_TERRAIN_SENTINEL: [u8; 8] = [0x48, 0x8b, 0xde, 0x48, 0x89, 0x5c, 0x24, 0x58];
/// Attrition terrain continuation; observe before the displaced instruction.
pub const ATTRITION_TERRAIN_CONTINUATION_RVA: usize = 0x00C6_5EDF;
/// Attrition equipment checkpoint.
pub const ATTRITION_EQUIPMENT_RVA: usize = 0x00C6_608D;
/// Attrition equipment instruction sentinel.
pub const ATTRITION_EQUIPMENT_SENTINEL: [u8; 5] = [0x48, 0x8b, 0x7c, 0x24, 0x60];
/// Attrition equipment continuation; observe before the displaced instruction.
pub const ATTRITION_EQUIPMENT_CONTINUATION_RVA: usize = 0x00C6_6092;
/// Attrition state checkpoint.
pub const ATTRITION_STATE_RVA: usize = 0x00C6_61D4;
/// Attrition state instruction sentinel.
pub const ATTRITION_STATE_SENTINEL: [u8; 6] = [0x48, 0x03, 0xd8, 0x4d, 0x85, 0xe4];
/// Attrition state continuation; observe after the displaced instruction.
pub const ATTRITION_STATE_CONTINUATION_RVA: usize = 0x00C6_61DA;
/// Attrition supply a checkpoint.
pub const ATTRITION_SUPPLY_A_RVA: usize = 0x00C6_625F;
/// Attrition supply a instruction sentinel.
pub const ATTRITION_SUPPLY_A_SENTINEL: [u8; 5] = [0x48, 0x03, 0xda, 0xeb, 0x4f];
/// Attrition supply a continuation; observe after the displaced instruction.
pub const ATTRITION_SUPPLY_A_CONTINUATION_RVA: usize = 0x00C6_62B3;
/// Attrition supply b checkpoint.
pub const ATTRITION_SUPPLY_B_RVA: usize = 0x00C6_62A0;
/// Attrition supply b instruction sentinel.
pub const ATTRITION_SUPPLY_B_SENTINEL: [u8; 6] = [0x48, 0x03, 0xda, 0x4d, 0x85, 0xe4];
/// Attrition supply b continuation; observe after the displaced instruction.
pub const ATTRITION_SUPPLY_B_CONTINUATION_RVA: usize = 0x00C6_62A6;
/// Attrition state modifiers checkpoint.
pub const ATTRITION_STATE_MODIFIERS_RVA: usize = 0x00C6_67FE;
/// Attrition state modifiers instruction sentinel.
pub const ATTRITION_STATE_MODIFIERS_SENTINEL: [u8; 8] =
    [0x48, 0x03, 0x5d, 0xd8, 0x48, 0x8b, 0x55, 0x08];
/// Attrition state modifiers continuation; observe after the displaced instruction.
pub const ATTRITION_STATE_MODIFIERS_CONTINUATION_RVA: usize = 0x00C6_6806;
/// Attrition weather modifiers checkpoint.
pub const ATTRITION_WEATHER_MODIFIERS_RVA: usize = 0x00C6_6AA7;
/// Attrition weather modifiers instruction sentinel.
pub const ATTRITION_WEATHER_MODIFIERS_SENTINEL: [u8; 11] = [
    0x48, 0x03, 0x5d, 0xe0, 0x49, 0x8b, 0x8d, 0xc0, 0x00, 0x00, 0x00,
];
/// Attrition weather modifiers continuation; observe after the displaced instruction.
pub const ATTRITION_WEATHER_MODIFIERS_CONTINUATION_RVA: usize = 0x00C6_6AB2;
/// Attrition derived terms checkpoint.
pub const ATTRITION_DERIVED_TERMS_RVA: usize = 0x00C6_6F43;
/// Attrition derived terms instruction sentinel.
pub const ATTRITION_DERIVED_TERMS_SENTINEL: [u8; 8] =
    [0x48, 0x03, 0xd8, 0x48, 0x8b, 0x74, 0x24, 0x60];
/// Attrition derived terms continuation; observe after the displaced instruction.
pub const ATTRITION_DERIVED_TERMS_CONTINUATION_RVA: usize = 0x00C6_6F4B;
/// Attrition training checkpoint.
pub const ATTRITION_TRAINING_RVA: usize = 0x00C6_6F70;
/// Attrition training instruction sentinel.
pub const ATTRITION_TRAINING_SENTINEL: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x58];
/// Attrition training continuation; observe before the displaced instruction.
pub const ATTRITION_TRAINING_CONTINUATION_RVA: usize = 0x00C6_6F75;
/// Attrition border war checkpoint.
pub const ATTRITION_BORDER_WAR_RVA: usize = 0x00C6_7106;
/// Attrition border war instruction sentinel.
pub const ATTRITION_BORDER_WAR_SENTINEL: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x58];
/// Attrition border war continuation; observe before the displaced instruction.
pub const ATTRITION_BORDER_WAR_CONTINUATION_RVA: usize = 0x00C6_710B;
/// Attrition final checkpoint.
pub const ATTRITION_FINAL_RVA: usize = 0x00C6_7307;
/// Attrition final instruction sentinel.
pub const ATTRITION_FINAL_SENTINEL: [u8; 5] = [0x48, 0x89, 0x54, 0x24, 0x58];
/// Attrition final continuation; observe after the displaced instruction.
pub const ATTRITION_FINAL_CONTINUATION_RVA: usize = 0x00C6_730C;
/// Attrition cached join checkpoint.
pub const ATTRITION_CACHED_JOIN_RVA: usize = 0x00C7_95A0;
/// Attrition cached join instruction sentinel.
pub const ATTRITION_CACHED_JOIN_SENTINEL: [u8; 7] = [0x48, 0x89, 0x8b, 0x38, 0x04, 0x00, 0x00];
/// Attrition cached join continuation; observe after the displaced instruction.
pub const ATTRITION_CACHED_JOIN_CONTINUATION_RVA: usize = 0x00C7_95A7;
/// Attrition refresh callsite.
pub const ATTRITION_REFRESH_CALL_RVA: usize = 0x00c7_9598;
/// Attrition refresh return address.
pub const ATTRITION_REFRESH_RETURN_RVA: usize = 0x00c7_959d;
/// Division-width leaf returning i64.
pub const WIDTH_LEAF_RVA: usize = 0x00c6_e270;
/// Width-leaf entry sentinel.
pub const WIDTH_LEAF_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
/// Width-leaf virtual entry index.
pub const WIDTH_LEAF_VTABLE_INDEX: usize = 46;
/// Primary vtable entry count.
pub const VTABLE_ENTRY_COUNT: usize = 59;
/// Primary vtable entry selecting the width leaf.
pub const WIDTH_LEAF_VTABLE_SLOT_RVA: usize = 0x0294_3c40;
/// Width-leaf unwind metadata.
pub const WIDTH_LEAF_UNWIND_RVA: usize = 0x0363_7d30;
/// Candidate active-damage function receiving the division first; its receiver type is unknown.
pub const ACTIVE_DAMAGE_RVA: usize = 0x00c7_bfc0;
/// Active-damage entry sentinel.
pub const ACTIVE_DAMAGE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// Self-returning virtual byte offset.
pub const SELF_VTABLE_SLOT: usize = 0x38;
/// Shared self-returning thunk; its address does not identify a unique owner.
pub const SELF_THUNK_RVA: usize = 0x001f_8640;
/// Organisation getter virtual byte offset.
pub const ORGANISATION_VTABLE_SLOT: usize = 0x120;
/// Organisation getter returning raw i64.
pub const ORGANISATION_GETTER_RVA: usize = 0x00c7_8810;
/// Strength fraction from current and maximum hit points, clamped to zero through 100000.
pub const STRENGTH_FRACTION_GETTER_RVA: usize = 0x00c7_81e0;
/// Current-strength getter returning i64.
pub const STRENGTH_GETTER_RVA: usize = 0x00c7_8820;
/// Maximum-strength getter with minimum one hundred.
pub const MAXIMUM_STRENGTH_GETTER_RVA: usize = 0x00c7_2eb0;
/// Standard-template composition getter, returning default composition when null.
pub const TEMPLATE_COMPOSITION_GETTER_RVA: usize = 0x00c6_f080;

/// Constructor followed by registration of the full handle.
pub const CONSTRUCTOR_RVA: usize = 0x00c5_dd30;
/// Secondary-receiver serializer retaining template and i64 hit points and organisation.
pub const SERIALIZE_RVA: usize = 0x00c8_0e00;
/// Secondary-receiver loader restoring the same template and i64 fields.
pub const DESERIALIZE_RVA: usize = 0x00c7_d090;
/// Synchronous hourly division update.
pub const HOURLY_UPDATE_RVA: usize = 0x00c7_8a90;
/// Pre-update run in joined parallel work before serial unit updates.
pub const PREPARE_HOURLY_UPDATE_RVA: usize = 0x00c7_9330;
/// Active-stat recalculation in staggered parallel work joined before returning.
pub const UPDATE_ACTIVE_STATS_RVA: usize = 0x00c7_deb0;
/// Hit-points setter clamping i64 after stat recalculation.
pub const SET_HIT_POINTS_RVA: usize = 0x00c7_fa80;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Native reference qword: type in the low u32, numeric id in the high u32.
    pub id: u64,
    /// Raw country link whose canonical country owns this roster entry.
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    /// Zero represents the native null location pointer.
    pub province: u32,
    /// Standard template's full native reference; zero represents a null pointer.
    pub template: u64,
    /// Current HP, not the derived strength fraction; native signed fixed5.
    pub hit_points: i64,
    /// Native signed fixed5, without display rounding.
    pub organisation: i64,
    /// Full COrdersGroup reference of the army this division belongs to, read
    /// through the embedded COrdersGroupMember back-pointer at primary +0xc0.
    /// Zero preserves the native null pointer of an unassigned division; it is
    /// an independent second source for the group's own member vector, never a
    /// substitute for it.
    pub orders_group: u64,
    /// Standard template's [`creferenced_division_template::REVEAL_SKIP`]
    /// byte; zero for a null template.
    pub reveal_skip: u8,
}

/// Hourly companion to a division's existing authenticated core row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IconInputsRow {
    pub division: u64,
    /// Native byte, including noncanonical true values.
    pub separate_counter: u8,
    /// Full preferred template handle; zero means the pointer is null.
    pub alternate_template: u64,
}

/// Bounded, allocation-free acquisition from the shared division census.
/// The same template authentication is used for the standard and alternate
/// pointers in the current source acquisition. A supplied seed and source must
/// belong to this same acquisition boundary; failed seed fields are not reread.
pub fn read_icon_inputs(
    memory: &impl Memory,
    base: usize,
    source: crate::cequipment_variant_pool::DivisionSource,
    seed: Option<&DivisionSeed>,
) -> Result<IconInputsRow, &'static str> {
    let at = |offset| {
        source
            .pointer
            .checked_add(offset)
            .ok_or("army icon address overflow")
    };
    let primary = base
        .checked_add(VTABLE_RVA)
        .ok_or("army icon module address overflow")?;
    let secondary = base
        .checked_add(SECONDARY_VTABLE_RVA)
        .ok_or("army icon module address overflow")?;
    let identity = || {
        let matches = if source.pointer == 0 || source.division == 0 {
            false
        } else if let Some(seed) = seed {
            seed.pointer == source.pointer
                && seed.division == source.division
                && seed.primary_vtable == Some(primary as u64)
                && seed.secondary_vtable == Some(secondary as u64)
                && seed.handle == Some(source.division)
                // This is the raw roster reference, not its canonical slot.
                && seed.roster_owner == Some(source.country)
        } else {
            read::<u64>(memory, source.pointer) == Some(primary as u64)
                && read::<u64>(memory, at(SECONDARY_BASE)?) == Some(secondary as u64)
                && read::<u64>(memory, at(cunit::HANDLE)?) == Some(source.division)
                && read::<u32>(memory, at(cunit::ROSTER_OWNER)?) == Some(source.country)
        };
        if !matches {
            return Err("army icon source identity changed");
        }
        Ok(())
    };
    identity()?;
    let separate_counter =
        read::<u8>(memory, at(SEPARATE_COUNTER)?).ok_or("army separate-counter byte unreadable")?;
    let pointer = read::<u64>(memory, at(FAKE_INTEL_TEMPLATE)?)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("army alternate template pointer unreadable")?;
    let alternate_template =
        if pointer != 0 && seed.is_some_and(|seed| seed.standard_template == Some(pointer)) {
            // The core projection authenticated this exact pointer and retained its
            // handle in source.template. A different alternate template stays live.
            if source.template == 0 {
                return Err("non-null division template has null handle");
            }
            source.template
        } else {
            template_handle(memory, base, pointer)?
        };
    if seed.is_none() {
        identity()?;
    }
    Ok(IconInputsRow {
        division: source.division,
        separate_counter,
        alternate_template,
    })
}

fn template_handle(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    let expected = base
        .checked_add(creferenced_division_template::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("division template vtable mismatch");
    }
    let handle = pointer
        .checked_add(creferenced_division_template::TYPE)
        .and_then(|address| read::<u64>(memory, address))
        .ok_or("division template handle unreadable")?;
    if handle == 0 {
        Err("non-null division template has null handle")
    } else {
        Ok(handle)
    }
}

/// Reads each canonical country's CArmy roster once after the full tick returns.
/// Country aliases are authenticated but do not duplicate a roster. Rows retain
/// native order; duplicate-handle checks and sorting belong to the spool thread.
/// Vtables and nonzero handles validate the roster object's native identity
/// fields, not independent membership in the engine's reference registry.
/// A full output buffer still counts every remaining non-null roster slot and
/// checks its pointer read, without reading that row's fields or allocating.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_with(memory, base, game_state, output, |_, _| {})
}

/// Shares this census with allocation-free state readers. Each invocation reads
/// independently and visits only authenticated rows that fit in `output`.
/// Pointers are transient sources, never replay identities. Visitors retain
/// their own capacity/failure outcomes; a truncated or unavailable parent walk
/// cannot establish a complete dependent census, even after earlier visits.
pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    mut visit: impl FnMut(usize, &Row),
) -> Outcome {
    let result = (|| {
        let array = |offset, stride, reason| {
            game_state
                .checked_add(offset)
                .and_then(|p| Array::read(memory, p, stride))
                .ok_or(reason)
        };
        let reader = Reader {
            memory,
            base,
            countries: array(
                cgame_state::COUNTRIES,
                8,
                "country array unreadable or invalid",
            )?,
            links: array(
                cgame_state::COUNTRY_LINKS,
                4,
                "country links unreadable or invalid",
            )?,
            provinces: array(
                cgame_state::PROVINCES,
                8,
                "province array unreadable or invalid",
            )?,
        };
        if reader.countries.count < 2 {
            return Err("country table has no country slots");
        }
        if reader.links.count == 0 {
            return Err("country links have no sentinel slot");
        }
        if reader.provinces.count < 2 {
            return Err("province table has no province slots");
        }
        let mut len = 0;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..reader.countries.count {
            let country = reader.country_pointer(slot)?;
            if country == 0 {
                continue;
            }
            reader.vtable(country, ccountry::VTABLE_RVA, "country vtable mismatch")?;
            let id = country
                .checked_add(ccountry::ID)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("country id unreadable")?;
            let canonical = reader.canonical(id)?;
            if reader.canonical(slot)? != canonical {
                return Err("country slot and object resolve to different countries");
            }
            if reader.country_pointer(canonical)? != country {
                return Err("canonical country slot points to a different object");
            }
            if slot != canonical {
                continue;
            }
            found_country = true;
            let roster = country
                .checked_add(ccountry::DIVISIONS)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("division roster unreadable or invalid")?;
            // Unlike the country/province tables, this vector has no sentinel.
            for index in 0..roster.count {
                let pointer = roster
                    .element(index, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .ok_or("division roster pointer unreadable")?;
                if pointer == 0 {
                    continue;
                }
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("division unread row count overflow")?;
                } else {
                    output[len] = reader.row(pointer, canonical)?;
                    visit(pointer, &output[len]);
                    len += 1;
                }
            }
        }
        if !found_country {
            Err("country table has no readable countries")
        } else if unread_rows != 0 {
            Ok(Outcome::Truncated { len, unread_rows })
        } else {
            Ok(Outcome::Rows { len })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Scale applied to a naval transport edge speed.
pub const TRANSPORT_SCALE_RVA: usize = 0x0331_efe0;
/// Fixed speed of a naval invasion edge.
pub const INVASION_SPEED_RVA: usize = 0x0331_ead8;
/// Outer scale of the normal and rail edge speeds.
pub const OUTER_SCALE_RVA: usize = 0x0331_d688;
/// River terms of crossing kinds two and three.
pub const RIVER_CROSSING_TWO_RVA: usize = 0x0331_d820;
pub const RIVER_CROSSING_THREE_RVA: usize = 0x0331_d8b0;

/// Fixed native fields captured once from the current authenticated division census.
/// An absent field records its original failed read; consumers never retry it.
/// Pointer-bearing fields admit child reads only after their consumer validates them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionSeed {
    pub pointer: usize,
    pub division: u64,
    /// Country-roster position; removals swap in the final entry.
    pub country_ordinal: Option<u32>,
    pub primary_vtable: Option<u64>,
    pub secondary_vtable: Option<u64>,
    pub handle: Option<u64>,
    pub roster_owner: Option<u32>,
    pub logical_owner: Option<u32>,
    pub expeditionary_owner: Option<u32>,
    pub location: Option<usize>,
    pub standard_template: Option<usize>,
    pub strength: Option<i64>,
    pub organisation: Option<i64>,
    pub orders_group: Option<usize>,
    pub class: Option<u32>,
    pub stat_block: Option<usize>,
    pub ordinary_route: Option<[u8; 16]>,
    pub alternate_route: Option<[u8; 16]>,
    pub combat: Option<[u8; 16]>,
    pub movement_numerator: Option<i64>,
    pub movement_target: Option<usize>,
    pub movement_source: Option<usize>,
    pub naval_transfer_membership: Option<usize>,
    pub retreat: Option<u8>,
    pub strategic_redeployment: Option<u8>,
    pub counter_attrition: Option<i64>,
    pub entrenchment: Option<i64>,
    pub entrenchment_cap: Option<i64>,
    pub counter_supply_status: Option<i64>,
    /// [`INVALID_ORDERS`]; the counter's yellow `!` gate.
    pub invalid_orders: Option<u8>,
    /// [`STRENGTH_FACTOR`]; multiplies the counter's strength ratio.
    pub strength_factor: Option<i64>,
    /// [`RETAINED_VISUAL_VARIANT`], the stored land model number.
    pub model_number: Option<i32>,
    /// The first 0x40 bytes of the embedded [`ACCLIMATIZATION`] object.
    pub acclimatization: Option<[u8; 0x40]>,
    /// [`cunit::MAP_ENTITY`]; only whether it is set is recorded.
    pub map_entity: Option<usize>,
    /// [`EXILED`], the raid army-reach exile byte.
    pub exiled: Option<u8>,
    /// Division-name member pointer; operands are copied separately.
    pub name_member: Option<usize>,
    pub supply_grace_maximum: Option<i64>,
    pub supply_grace_remaining: Option<i64>,
    /// Owned header acquired once for the hourly complete contributor list.
    pub manpower_values: Option<[u8; 24]>,
}

impl DivisionSeed {
    pub const PRIMARY_VTABLE: u64 = 1 << 0;
    pub const SECONDARY_VTABLE: u64 = 1 << 1;
    pub const HANDLE: u64 = 1 << 2;
    pub const ROSTER_OWNER: u64 = 1 << 3;
    pub const LOGICAL_OWNER: u64 = 1 << 4;
    pub const EXPEDITIONARY_OWNER: u64 = 1 << 5;
    pub const LOCATION: u64 = 1 << 6;
    pub const STANDARD_TEMPLATE: u64 = 1 << 7;
    pub const STRENGTH: u64 = 1 << 8;
    pub const ORGANISATION: u64 = 1 << 9;
    pub const ORDERS_GROUP: u64 = 1 << 10;
    pub const CLASS: u64 = 1 << 11;
    pub const STAT_BLOCK: u64 = 1 << 12;
    pub const ORDINARY_ROUTE: u64 = 1 << 13;
    pub const ALTERNATE_ROUTE: u64 = 1 << 14;
    pub const COMBAT: u64 = 1 << 15;
    pub const MOVEMENT_NUMERATOR: u64 = 1 << 16;
    pub const MOVEMENT_TARGET: u64 = 1 << 17;
    pub const MOVEMENT_SOURCE: u64 = 1 << 18;
    pub const NAVAL_TRANSFER_MEMBERSHIP: u64 = 1 << 19;
    pub const RETREAT: u64 = 1 << 20;
    pub const STRATEGIC_REDEPLOYMENT: u64 = 1 << 21;
    pub const COUNTER_ATTRITION: u64 = 1 << 22;
    pub const ENTRENCHMENT: u64 = 1 << 23;
    pub const ENTRENCHMENT_CAP: u64 = 1 << 24;
    pub const COUNTER_SUPPLY_STATUS: u64 = 1 << 25;
    pub const INVALID_ORDERS: u64 = 1 << 26;
    pub const STRENGTH_FACTOR: u64 = 1 << 27;
    pub const MODEL_NUMBER: u64 = 1 << 28;
    pub const ACCLIMATIZATION: u64 = 1 << 29;
    pub const MAP_ENTITY: u64 = 1 << 30;
    pub const EXILED: u64 = 1 << 31;
    pub const NAME_MEMBER: u64 = 1 << 32;
    pub const SUPPLY_GRACE_MAXIMUM: u64 = 1 << 33;
    pub const SUPPLY_GRACE_REMAINING: u64 = 1 << 34;
    pub const MANPOWER_VALUES: u64 = 1 << 35;
}

const DIVISION_SEED_SPANS: [(usize, usize); 36] = [
    (0, 8),
    (SECONDARY_BASE, 8),
    (cunit::HANDLE, 8),
    (cunit::ROSTER_OWNER, 4),
    (cunit::LOGICAL_OWNER, 4),
    (cunit::EXPEDITIONARY_OWNER, 4),
    (cunit::LOCATION, 8),
    (STANDARD_TEMPLATE, 8),
    (STRENGTH, 8),
    (ORGANISATION, 8),
    (ORDERS_GROUP, 8),
    (cunit::CLASS, 4),
    (cunit::STAT_BLOCK, 8),
    (cunit::ORDINARY_ROUTE_DATA, 16),
    (cunit::ALTERNATE_ROUTE_DATA, 16),
    (cunit::COMBAT_REFERENCE_DATA, 16),
    (MOVEMENT_NUMERATOR, 8),
    (MOVEMENT_TARGET, 8),
    (MOVEMENT_SOURCE, 8),
    (NAVAL_TRANSFER_MEMBERSHIP, 8),
    (RETREAT, 1),
    (STRATEGIC_REDEPLOYMENT, 1),
    (COUNTER_ATTRITION, 8),
    (ENTRENCHMENT, 8),
    (ENTRENCHMENT_CAP, 8),
    (COUNTER_SUPPLY_STATUS, 8),
    (INVALID_ORDERS, 1),
    (STRENGTH_FACTOR, 8),
    (RETAINED_VISUAL_VARIANT, 4),
    (ACCLIMATIZATION, 0x40),
    (cunit::MAP_ENTITY, 8),
    (EXILED, 1),
    (crate::carmy_combat_inputs::NAME_GROUP_MEMBER, 8),
    (MAXIMUM_SUPPLY_GRACE, 8),
    (REMAINING_SUPPLY_GRACE, 8),
    (MANPOWER_VALUES, 24),
];
const DIVISION_SEED_FIELDS: [crate::memory::RecordField; 36] =
    crate::memory::packed_fields(DIVISION_SEED_SPANS);
const DIVISION_SEED_STRIDE: usize = crate::memory::packed_stride(DIVISION_SEED_SPANS);

/// Gather exact fixed layouts in bounded row batches. A failed field stays failed;
/// later fields remain independently available to other tables. The caller sets
/// each native pointer and expected full handle before capture.
pub fn capture_division_seeds(memory: &impl Memory, seeds: &mut [DivisionSeed]) -> u64 {
    const BATCH: usize = 64;
    let mut bytes = [0u8; BATCH * DIVISION_SEED_STRIDE];
    let mut pointers = [0usize; BATCH];
    let mut total = 0u64;
    for chunk in seeds.chunks_mut(BATCH) {
        for (pointer, seed) in pointers.iter_mut().zip(chunk.iter()) {
            *pointer = seed.pointer;
        }
        const LAYOUT: crate::memory::RecordLayout<'static> =
            crate::memory::RecordLayout::new(&DIVISION_SEED_FIELDS, DIVISION_SEED_STRIDE)
                .expect("valid division seed layout");
        let copied = memory.copy_records(
            crate::memory::RecordSources::Pointers(&pointers[..chunk.len()]),
            &LAYOUT,
            &mut bytes,
        );
        for (row, seed) in chunk.iter_mut().enumerate() {
            let mut valid = 0u64;
            let record = &mut bytes[row * DIVISION_SEED_STRIDE..(row + 1) * DIVISION_SEED_STRIDE];
            for (field, layout) in DIVISION_SEED_FIELDS.iter().enumerate() {
                let ordinal = row * DIVISION_SEED_FIELDS.len() + field;
                let success = ordinal < copied
                    || (ordinal > copied
                        && seed
                            .pointer
                            .checked_add(layout.source_offset)
                            .is_some_and(|at| {
                                memory.copy(
                                    at,
                                    &mut record[layout.destination_offset
                                        ..layout.destination_offset + layout.len],
                                )
                            }));
                if success {
                    valid |= 1 << field;
                    total += layout.len as u64;
                }
            }
            seed.primary_vtable = (valid & DivisionSeed::PRIMARY_VTABLE != 0)
                .then(|| u64::from_le_bytes(record[0..8].try_into().unwrap()));
            seed.secondary_vtable = (valid & DivisionSeed::SECONDARY_VTABLE != 0)
                .then(|| u64::from_le_bytes(record[8..16].try_into().unwrap()));
            seed.handle = (valid & DivisionSeed::HANDLE != 0)
                .then(|| u64::from_le_bytes(record[16..24].try_into().unwrap()));
            seed.roster_owner = (valid & DivisionSeed::ROSTER_OWNER != 0)
                .then(|| u32::from_le_bytes(record[24..28].try_into().unwrap()));
            seed.logical_owner = (valid & DivisionSeed::LOGICAL_OWNER != 0)
                .then(|| u32::from_le_bytes(record[28..32].try_into().unwrap()));
            seed.expeditionary_owner = (valid & DivisionSeed::EXPEDITIONARY_OWNER != 0)
                .then(|| u32::from_le_bytes(record[32..36].try_into().unwrap()));
            seed.location = (valid & DivisionSeed::LOCATION != 0)
                .then(|| u64::from_le_bytes(record[36..44].try_into().unwrap()) as usize);
            seed.standard_template = (valid & DivisionSeed::STANDARD_TEMPLATE != 0)
                .then(|| u64::from_le_bytes(record[44..52].try_into().unwrap()) as usize);
            seed.strength = (valid & DivisionSeed::STRENGTH != 0)
                .then(|| i64::from_le_bytes(record[52..60].try_into().unwrap()));
            seed.organisation = (valid & DivisionSeed::ORGANISATION != 0)
                .then(|| i64::from_le_bytes(record[60..68].try_into().unwrap()));
            seed.orders_group = (valid & DivisionSeed::ORDERS_GROUP != 0)
                .then(|| u64::from_le_bytes(record[68..76].try_into().unwrap()) as usize);
            seed.class = (valid & DivisionSeed::CLASS != 0)
                .then(|| u32::from_le_bytes(record[76..80].try_into().unwrap()));
            seed.stat_block = (valid & DivisionSeed::STAT_BLOCK != 0)
                .then(|| u64::from_le_bytes(record[80..88].try_into().unwrap()) as usize);
            seed.ordinary_route = (valid & DivisionSeed::ORDINARY_ROUTE != 0)
                .then(|| record[88..104].try_into().unwrap());
            seed.alternate_route = (valid & DivisionSeed::ALTERNATE_ROUTE != 0)
                .then(|| record[104..120].try_into().unwrap());
            seed.combat =
                (valid & DivisionSeed::COMBAT != 0).then(|| record[120..136].try_into().unwrap());
            seed.movement_numerator = (valid & DivisionSeed::MOVEMENT_NUMERATOR != 0)
                .then(|| i64::from_le_bytes(record[136..144].try_into().unwrap()));
            seed.movement_target = (valid & DivisionSeed::MOVEMENT_TARGET != 0)
                .then(|| u64::from_le_bytes(record[144..152].try_into().unwrap()) as usize);
            seed.movement_source = (valid & DivisionSeed::MOVEMENT_SOURCE != 0)
                .then(|| u64::from_le_bytes(record[152..160].try_into().unwrap()) as usize);
            seed.naval_transfer_membership = (valid & DivisionSeed::NAVAL_TRANSFER_MEMBERSHIP != 0)
                .then(|| u64::from_le_bytes(record[160..168].try_into().unwrap()) as usize);
            seed.retreat = (valid & DivisionSeed::RETREAT != 0).then(|| record[168]);
            seed.strategic_redeployment =
                (valid & DivisionSeed::STRATEGIC_REDEPLOYMENT != 0).then(|| record[169]);
            seed.counter_attrition = (valid & DivisionSeed::COUNTER_ATTRITION != 0)
                .then(|| i64::from_le_bytes(record[170..178].try_into().unwrap()));
            seed.entrenchment = (valid & DivisionSeed::ENTRENCHMENT != 0)
                .then(|| i64::from_le_bytes(record[178..186].try_into().unwrap()));
            seed.entrenchment_cap = (valid & DivisionSeed::ENTRENCHMENT_CAP != 0)
                .then(|| i64::from_le_bytes(record[186..194].try_into().unwrap()));
            seed.counter_supply_status = (valid & DivisionSeed::COUNTER_SUPPLY_STATUS != 0)
                .then(|| i64::from_le_bytes(record[194..202].try_into().unwrap()));
            seed.invalid_orders = (valid & DivisionSeed::INVALID_ORDERS != 0).then(|| record[202]);
            seed.strength_factor = (valid & DivisionSeed::STRENGTH_FACTOR != 0)
                .then(|| i64::from_le_bytes(record[203..211].try_into().unwrap()));
            seed.model_number = (valid & DivisionSeed::MODEL_NUMBER != 0)
                .then(|| i32::from_le_bytes(record[211..215].try_into().unwrap()));
            seed.acclimatization = (valid & DivisionSeed::ACCLIMATIZATION != 0)
                .then(|| record[215..279].try_into().unwrap());
            seed.map_entity = (valid & DivisionSeed::MAP_ENTITY != 0)
                .then(|| u64::from_le_bytes(record[279..287].try_into().unwrap()) as usize);
            seed.exiled = (valid & DivisionSeed::EXILED != 0).then(|| record[287]);
            seed.name_member = (valid & DivisionSeed::NAME_MEMBER != 0)
                .then(|| u64::from_le_bytes(record[288..296].try_into().unwrap()) as usize);
            seed.supply_grace_maximum = (valid & DivisionSeed::SUPPLY_GRACE_MAXIMUM != 0)
                .then(|| i64::from_le_bytes(record[296..304].try_into().unwrap()));
            seed.supply_grace_remaining = (valid & DivisionSeed::SUPPLY_GRACE_REMAINING != 0)
                .then(|| i64::from_le_bytes(record[304..312].try_into().unwrap()));
            seed.manpower_values = (valid & DivisionSeed::MANPOWER_VALUES != 0)
                .then(|| record[312..336].try_into().unwrap());
        }
    }
    total
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MovementRow {
    pub division: u64,
    pub source: u32,
    pub target: u32,
    pub numerator: i64,
    pub retreat: u8,
    pub strategic_redeployment: u8,
    /// Current strategic-action operands, present only on the demanded branch.
    pub strategic_action: StrategicAction,
    pub naval_transfer: u8,
    pub combat_count: u32,
    /// 0 no combat, 1 first-combat attacker-country, 2 first-combat defender.
    pub combat_role: u8,
    /// Province of the first referenced combat, zero when absent. The viewer
    /// derives stack kind from this and effective content.
    pub combat_province: u32,
    /// One when the unit map entity is set. Combat icons use the first attacker
    /// with an entity. Stack kind alone cannot establish presence because combat
    /// roles take kinds nine and ten without one.
    pub map_entity: u8,
    pub path_start: usize,
    pub path_len: usize,
    pub path_total: u32,
    pub alternate_start: usize,
    pub alternate_len: usize,
    pub alternate_total: u32,
    /// Raw saved `withdraw` byte, acquired after the established movement
    /// pass. An independent refusal withholds only this arrow-style operand.
    pub withdrawal: Option<u8>,
    /// Raw signed movement priority. Only class-zero follow-orders arrows test
    /// values below one; unknown enum values are never clamped.
    pub movement_priority: Option<i32>,
    /// Complete ordered full identities in MovementPacket's owned arena.
    pub combat_references: Option<(usize, usize)>,
    pub combat_references_failure: Option<&'static str>,
    pub selected_exiled: Option<u8>,
    pub selected_commander: Option<bool>,
    pub selected_retreat: [Option<i64>; 2],
    pub selected_retreat_failures: [Option<&'static str>; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementWalk {
    pub outcome: Outcome,
    pub paths_len: usize,
    pub required_paths: usize,
}

#[derive(Clone, Copy, Default)]
struct PresentationInput {
    bytes: [u8; 16],
}

/// Exact dependencies retained only within one acquisition. Worker storage
/// grows at each successful reached dependency; a vacant callback uses direct reads.
pub(crate) struct PresentationInputs {
    entries: std::collections::HashMap<(usize, usize), PresentationInput>,
    worker_growth: bool,
}

impl PresentationInputs {
    pub(crate) fn new(capacity: usize) -> std::io::Result<Self> {
        let mut entries = std::collections::HashMap::new();
        crate::allocation::reserve_map(&mut entries, capacity)?;
        Ok(Self {
            entries,
            worker_growth: capacity != 0,
        })
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<((usize, usize), PresentationInput)>()
    }
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
    pub(crate) fn read<T: crate::memory::Raw>(
        &mut self,
        memory: &impl Memory,
        address: usize,
    ) -> Option<T> {
        let len = std::mem::size_of::<T>();
        // These operands are scalar/native record prefixes, at most 16 bytes.
        if !self.worker_growth || address == 0 || len > 16 || len == 0 {
            return read(memory, address);
        }
        if let Some(entry) = self.entries.get(&(address, len)) {
            // Raw is sealed to padding-free all-bit-pattern types.
            return Some(unsafe { entry.bytes.as_ptr().cast::<T>().read_unaligned() });
        }
        let mut entry = PresentationInput::default();
        if !crate::read_slice(memory, address, &mut entry.bytes[..len]) {
            return None;
        }
        let value = unsafe { entry.bytes.as_ptr().cast::<T>().read_unaligned() };
        crate::allocation::reserve_map(&mut self.entries, 1).ok()?;
        self.entries.insert((address, len), entry);
        Some(value)
    }
}

/// Owned province identities and consumed geography fields, shared only among
/// divisions in one movement pass. It never retains unit or render state.
#[cfg(test)]
pub struct MovementInputs {
    shared: std::cell::RefCell<PresentationInputs>,
}
#[cfg(test)]
impl MovementInputs {
    pub fn new(capacity: usize) -> Self {
        Self {
            shared: std::cell::RefCell::new(PresentationInputs::new(capacity).unwrap()),
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.shared.borrow().allocated_bytes()
    }
}

#[cfg(test)]
fn shared_presentation_read<T: crate::memory::Raw>(
    memory: &impl Memory,
    inputs: Option<&MovementInputs>,
    address: usize,
) -> Option<T> {
    match inputs {
        Some(inputs) => inputs.shared.borrow_mut().read(memory, address),
        None => read(memory, address),
    }
}

fn presentation_field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("division presentation field unreadable")
}

fn presentation_array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| Array::read(memory, p, stride))
        .ok_or("division presentation array invalid")
}

/// (object, offset, length) fields addressed as `presentation_field` does, in
/// one guarded call; an overflowing field fails there.
fn presentation_batch<const N: usize>(
    memory: &impl Memory,
    fields: [(usize, usize, usize); N],
) -> crate::memory::FieldBatch<N> {
    crate::memory::FieldBatch::gather(
        memory,
        fields.map(|(object, offset, len)| (object.checked_add(offset).unwrap_or(0), len)),
    )
}

/// A gathered field, consulted where `presentation_field` would read it.
fn presentation_batched<T: crate::memory::Raw, const N: usize>(
    memory: &impl Memory,
    fields: &crate::memory::FieldBatch<N>,
    index: usize,
) -> Result<T, &'static str> {
    fields
        .get(memory, index)
        .ok_or("division presentation field unreadable")
}

const MOVEMENT_ONE: i64 = 100_000;

fn movement_product(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / MOVEMENT_ONE
}

/// Every normal-speed multiplication divides by MOVEMENT_ONE before the final
/// clamp, bounding that positive input to i64::MAX / MOVEMENT_ONE. A nonzero
/// scale in this range therefore cannot wrap or truncate the final speed to
/// zero, regardless of the edge operands. Larger scales need exact evaluation.
fn normal_speed_zero_from_scale(scale: i64) -> Option<bool> {
    if scale == 0 {
        Some(true)
    } else if (-MOVEMENT_ONE..=MOVEMENT_ONE).contains(&scale) {
        Some(false)
    } else {
        None
    }
}

fn movement_fraction(numerator: i64, denominator: i64) -> Result<i64, &'static str> {
    if denominator == 0
        || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(MOVEMENT_ONE)
        .checked_div(denominator)
        .ok_or("native movement fraction traps")
}

fn movement_pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<usize, &'static str> {
    presentation_field::<u64>(memory, object, offset).map(|p| p as usize)
}

fn movement_address(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("movement address overflow")
}

fn movement_modifier(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    key: u32,
) -> Result<i64, &'static str> {
    if accumulator == 0 {
        return Ok(0);
    }
    crate::modifier_accumulator::lookup_effective_at(memory, base, accumulator, key)
        .map(|v| v.effective)
}

pub(crate) fn movement_definition(
    memory: &impl Memory,
    base: usize,
    province: usize,
) -> Result<usize, &'static str> {
    if province == 0
        || read::<u64>(memory, province)
            != base.checked_add(cprovince::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("movement province identity mismatch");
    }
    let definition = movement_pointer(memory, province, cprovince::DEFINITION)?;
    if definition == 0
        || presentation_field::<u32>(memory, definition, crate::province_definition::ID)?
            != presentation_field::<u32>(memory, province, cprovince::ID)?
    {
        return Err("movement province definition mismatch");
    }
    Ok(definition)
}

fn movement_flags(memory: &impl Memory, base: usize, province: usize) -> Result<u8, &'static str> {
    presentation_field(
        memory,
        movement_definition(memory, base, province)?,
        crate::province_definition::TERRAIN_FLAGS,
    )
}

pub(crate) fn movement_route_province(
    memory: &impl Memory,
    base: usize,
    state: usize,
    route: Array,
    index: u32,
) -> Result<usize, &'static str> {
    let id = read::<u32>(
        memory,
        route
            .element(index, 4)
            .ok_or("movement route index invalid")?,
    )
    .ok_or("movement route id unreadable")?;
    let provinces = presentation_array(memory, state, cgame_state::PROVINCES, 8)?;
    let province = read::<u64>(
        memory,
        provinces
            .element(id, 8)
            .ok_or("movement route province outside roster")?,
    )
    .ok_or("movement route province unreadable")? as usize;
    if province != 0 {
        movement_definition(memory, base, province)?;
        if presentation_field::<u32>(memory, province, cprovince::ID)? != id {
            return Err("movement route province identity mismatch");
        }
    }
    Ok(province)
}

/// The arrival calculation uses the actual route's final destination rather than a hypothetical edge.
pub(crate) fn movement_attacking(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
) -> Result<bool, &'static str> {
    let route = presentation_array(memory, army, cunit::ORDINARY_ROUTE_DATA, 4)?;
    if route.count == 0 {
        return Ok(false);
    }
    let destination = movement_route_province(memory, base, state, route, route.count - 1)?;
    if destination == 0 {
        return Ok(false);
    }
    let controller = presentation_field::<i32>(memory, destination, cprovince::CONTROLLER)?;
    if controller <= 0 {
        return Ok(false);
    }
    ccountry::movement_enemy(
        memory,
        base,
        state,
        controller as u32,
        presentation_field(memory, army, cunit::LOGICAL_OWNER)?,
    )
}

/// Owned native CArmy speed operands. No game address is needed to evaluate
/// them; the common normal inputs are acquired once for the current army.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Transient inline acquisition value; boxing would allocate on the callback.
// The retained ETA packet stores common inputs once per division.
#[allow(clippy::large_enum_variant)]
pub enum MovementSpeedInputs {
    Fixed(i64),
    Scaled {
        speed: i64,
        scale: i64,
    },
    Normal {
        common: NormalSpeedOperands,
        edge: NormalEdgeOperands,
    },
}
impl Default for MovementSpeedInputs {
    fn default() -> Self {
        Self::Fixed(0)
    }
}
impl From<i64> for MovementSpeedInputs {
    fn from(value: i64) -> Self {
        Self::Fixed(value)
    }
}
impl MovementSpeedInputs {
    pub fn evaluate(self) -> i64 {
        match self {
            Self::Fixed(value) => value,
            Self::Scaled { speed, scale } => movement_product(speed, scale),
            Self::Normal { common, edge } => common.prepare().evaluate(edge),
        }
    }
    /// Normal-range operands prove a positive scaled minimum without deriving
    /// the speed. Values outside the proof retain exact native evaluation for
    /// the lazy zero-speed route guard, including all wrapping edge cases.
    pub fn is_zero(self) -> bool {
        if let Self::Normal { common, edge } = self
            && common.bounded
            && edge.bounded()
        {
            return false;
        }
        self.evaluate() == 0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NormalSpeedOperands {
    modifier: NormalModifierOperands,
    base_speed: i64,
    organisation: Option<(i64, i64)>,
    fuel: Option<(i64, i64)>,
    outer_scale: i64,
    bounded: bool,
}
impl NormalSpeedOperands {
    /// Once-per-division worker preparation. All optional-read guards and
    /// fallible native divisions were resolved during dependency acquisition.
    pub fn prepare(self) -> PreparedNormalSpeed {
        PreparedNormalSpeed {
            modifier: self.modifier.evaluate(),
            base_speed: self.base_speed,
            organisation: self.organisation.map(|(ratio, penalty)| {
                MOVEMENT_ONE
                    .wrapping_add(movement_product(MOVEMENT_ONE.wrapping_sub(ratio), penalty))
            }),
            fuel: self.fuel.map(|(ratio, minimum)| {
                movement_product(MOVEMENT_ONE.wrapping_sub(minimum), ratio).wrapping_add(minimum)
            }),
            outer_scale: self.outer_scale,
        }
    }
    fn bounded(&self) -> bool {
        // Modifier and edge together are bounded by 1e9. Base speed <=1e6
        // gives <=1e10 before the non-increasing organisation/fuel factors.
        // Every multiply stays below 1e15, and positive scale preserves the
        // native minimum of 100000. These are sufficient, not assumed limits.
        movement_bounded(self.base_speed, 1_000_000)
            && self.modifier.bounded()
            && self.organisation.is_none_or(|(ratio, penalty)| {
                (0..=MOVEMENT_ONE).contains(&ratio) && (-MOVEMENT_ONE..=0).contains(&penalty)
            })
            && self.fuel.is_none_or(|(ratio, minimum)| {
                (0..=MOVEMENT_ONE).contains(&ratio) && (0..=MOVEMENT_ONE).contains(&minimum)
            })
            && (1..=MOVEMENT_ONE).contains(&self.outer_scale)
    }
}
/// Public because a replayer recomposes it from the recorded per-division
/// terms and evaluates the same edges without any game address.
#[derive(Clone, Copy)]
pub struct PreparedNormalSpeed {
    pub modifier: i64,
    pub base_speed: i64,
    pub organisation: Option<i64>,
    pub fuel: Option<i64>,
    pub outer_scale: i64,
}
impl PreparedNormalSpeed {
    pub fn evaluate(self, edge: NormalEdgeOperands) -> i64 {
        let mut speed =
            movement_product(self.base_speed, self.modifier.wrapping_add(edge.evaluate()));
        if let Some(factor) = self.organisation {
            speed = movement_product(factor, speed);
        }
        if let Some(factor) = self.fuel {
            speed = movement_product(factor, speed);
        }
        movement_product(speed.max(MOVEMENT_ONE), self.outer_scale)
    }
}
fn movement_bounded(value: i64, bound: i64) -> bool {
    (-bound..=bound).contains(&value)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct NormalModifierOperands {
    additive: i64,
    weather: [(i64, i64); 4],
    supply: (i64, i64),
    air: MovementAirOperands,
}
impl NormalModifierOperands {
    fn evaluate(self) -> i64 {
        let mut value = self.additive;
        for (acclimatization, modifier) in self.weather {
            value = value.wrapping_add(movement_product(acclimatization, modifier));
        }
        value
            .wrapping_add(movement_product(self.supply.0, self.supply.1))
            .wrapping_add(self.air.evaluate())
    }
    fn bounded(self) -> bool {
        movement_bounded(self.additive, 100_000_000)
            && self
                .weather
                .iter()
                .all(|&(a, b)| movement_bounded(a, MOVEMENT_ONE) && movement_bounded(b, 1_000_000))
            && movement_bounded(self.supply.0, 1_000_000)
            && movement_bounded(self.supply.1, 1_000_000)
            && self.air.bounded()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct MovementAirOperands {
    fraction: i64,
    impact: i64,
    scale: i64,
    defense: i64,
}
impl MovementAirOperands {
    fn evaluate(self) -> i64 {
        movement_product(movement_product(self.impact, self.fraction), self.scale)
            .wrapping_add(self.defense)
            .min(0)
    }
    fn bounded(self) -> bool {
        movement_bounded(self.fraction, MOVEMENT_ONE)
            && [self.impact, self.scale, self.defense]
                .into_iter()
                .all(|value| movement_bounded(value, 1_000_000))
    }
}

/// Public for the same reason as `PreparedNormalSpeed`: the recorded per-leg
/// operands are recomposed at replay time. `river` is (combined, adjustment,
/// crossing term) in the movement calculation's assembly order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NormalEdgeOperands {
    pub additive: i64,
    pub river: Option<(i64, i64, i64)>,
}
impl NormalEdgeOperands {
    fn evaluate(self) -> i64 {
        let mut value = self.additive;
        if let Some((combined, mut adjustment, river)) = self.river {
            if adjustment < 0 {
                adjustment = movement_product(combined, adjustment);
            }
            value = value
                .wrapping_add(adjustment)
                .wrapping_add(movement_product(
                    MOVEMENT_ONE.wrapping_add(combined).max(0),
                    river,
                ));
        }
        value
    }
    fn bounded(self) -> bool {
        movement_bounded(self.additive, 100_000_000)
            && self.river.is_none_or(|(a, b, c)| {
                [a, b, c]
                    .into_iter()
                    .all(|value| movement_bounded(value, 1_000_000))
            })
    }
}

/// Passive CArmy speed query with the null-tooltip path.
/// Every result uses fresh captured operands; no engine call or UI speed cache.
#[cfg(test)]
pub fn effective_movement_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    from: usize,
    to: usize,
) -> Result<i64, &'static str> {
    effective_movement_speed_with_transport(memory, base, state, army, from, to, |owner, region| {
        crate::cnaval_unit_transfer::movement_speed(memory, base, state, owner, region)
    })
}

/// The same movement branches with an army-independent transport callback.
/// Share its unscaled result only within an independently verified acquisition
/// pass.
#[cfg(test)]
pub fn effective_movement_speed_with_transport(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    from: usize,
    to: usize,
    transport_speed: impl FnMut(u32, usize) -> Result<i64, &'static str>,
) -> Result<i64, &'static str> {
    movement_speed_inputs_internal(
        memory,
        base,
        state,
        army,
        from,
        to,
        None,
        None,
        transport_speed,
    )
    .map(MovementSpeedInputs::evaluate)
}

/// Typed geometry values acquired by the caller from its original native source.
/// Geometry owns the authenticated dependencies used by this acquisition.
pub(crate) trait MovementGeometry {
    fn province_id(&mut self, pointer: usize) -> Result<u32, &'static str>;
    fn route_pointer(&mut self, id: u32) -> Result<usize, &'static str>;
    fn definition(&mut self, id: u32) -> Result<usize, &'static str>;
    fn flags(&mut self, id: u32) -> Result<u8, &'static str>;
    fn crossing(&mut self, from: u32, to: u32) -> Result<Option<u8>, &'static str>;
}

/// Typed per-division inputs shared with Movement and Status. All fields and
/// route bytes belong to the current acquisition; failed fields are never retried.
pub(crate) struct MovementSharedInputs<'a> {
    pub seed: &'a DivisionSeed,
    pub route_ids: fresh_counter_eta::MovementRouteIds<'a>,
    pub geometry: &'a mut dyn MovementGeometry,
    /// The current ETA range's existing directed-edge/infrastructure memo.
    /// Ordinary movement readers leave this absent and keep direct acquisition.
    pub rail_speeds: Option<&'a mut crate::crailway_manager::MovementSpeeds>,
    pub state_modifiers: Option<&'a std::cell::RefCell<MovementStateModifiers>>,
    pub category_contributions: Option<&'a std::cell::RefCell<MovementCategoryContributions>>,
    /// Complete template supply ratios acquired lazily in this ETA pass.
    pub supply_templates: Option<&'a std::cell::RefCell<std::collections::HashMap<usize, i64>>>,
    /// Complete diplomacy/region prefixes from this ETA range's current pass.
    pub air_prefixes: Option<&'a std::cell::RefCell<MovementAirMemo>>,
}

/// An air query retains every identity already authenticated before its prefix.
/// Distinct raw owners remain distinct even when country resolution aliases them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MovementAirKey {
    base: usize,
    state: usize,
    owner_raw: u32,
    owner_pointer: usize,
    canonical: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MovementAirPrefix {
    fraction: i64,
    impact: i64,
    scale: i64,
}

#[derive(Clone, Copy, Default)]
struct MovementAirActive {
    key: MovementAirKey,
    value: bool,
    generation: u64,
}

#[derive(Clone, Copy, Default)]
struct MovementAirRegion {
    key: MovementAirKey,
    region: usize,
    /// None is a completely read no-hostile result, not a failed query.
    value: Option<MovementAirPrefix>,
    generation: u64,
}

/// Two complete successful queries shared inside one ETA acquisition only.
/// Full keys decide hits; collisions remain cold and replace only on success.
/// Generation reset and all storage growth happen at the packet boundaries.
pub(crate) struct MovementAirMemo {
    active: Vec<MovementAirActive>,
    regions: Vec<MovementAirRegion>,
    generation: u64,
}

fn movement_memo_slots(rows: usize) -> std::io::Result<usize> {
    if rows == 0 {
        Ok(0)
    } else {
        rows.checked_next_power_of_two()
            .ok_or_else(crate::allocation::oom)
    }
}

impl MovementAirMemo {
    fn new(rows: usize) -> std::io::Result<Self> {
        let slots = movement_memo_slots(rows)?;
        Ok(Self {
            active: crate::allocation::filled(slots, MovementAirActive::default())?,
            regions: crate::allocation::filled(slots, MovementAirRegion::default())?,
            generation: 1,
        })
    }

    fn clear(&mut self) {
        self.generation = match self.generation.checked_add(1) {
            Some(generation) => generation,
            None => {
                for entry in &mut self.active {
                    entry.generation = 0;
                }
                for entry in &mut self.regions {
                    entry.generation = 0;
                }
                1
            }
        };
    }

    fn reserve(&mut self, rows: usize) -> std::io::Result<()> {
        if self.active.len() < rows {
            let slots = movement_memo_slots(rows)?;
            crate::allocation::resize(&mut self.active, slots, MovementAirActive::default())?;
            crate::allocation::resize(&mut self.regions, slots, MovementAirRegion::default())?;
            self.clear();
        }
        Ok(())
    }

    fn allocated_bytes(&self) -> usize {
        self.active.capacity() * size_of::<MovementAirActive>()
            + self.regions.capacity() * size_of::<MovementAirRegion>()
    }

    fn slot(&self, key: MovementAirKey, region: usize) -> Option<usize> {
        let mask = self.active.len().checked_sub(1)?;
        let hash = (key.owner_pointer as u64)
            ^ (region as u64).rotate_left(11)
            ^ u64::from(key.owner_raw).rotate_left(23)
            ^ u64::from(key.canonical).rotate_left(37)
            ^ (key.base as u64).rotate_left(49)
            ^ (key.state as u64).rotate_left(7);
        let mixed = (hash ^ (hash >> 32)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        Some((mixed ^ (mixed >> 32)) as usize & mask)
    }

    fn active(&self, key: MovementAirKey) -> Option<bool> {
        let entry = &self.active[self.slot(key, 0)?];
        (entry.generation == self.generation && entry.key == key).then_some(entry.value)
    }

    fn install_active(&mut self, key: MovementAirKey, value: bool) {
        if let Some(slot) = self.slot(key, 0) {
            self.active[slot] = MovementAirActive {
                key,
                value,
                generation: self.generation,
            };
        }
    }

    fn region(&self, key: MovementAirKey, region: usize) -> Option<Option<MovementAirPrefix>> {
        let entry = &self.regions[self.slot(key, region)?];
        (entry.generation == self.generation && entry.key == key && entry.region == region)
            .then_some(entry.value)
    }

    fn install_region(
        &mut self,
        key: MovementAirKey,
        region: usize,
        value: Option<MovementAirPrefix>,
    ) {
        if let Some(slot) = self.slot(key, region) {
            self.regions[slot] = MovementAirRegion {
                key,
                region,
                value,
                generation: self.generation,
            };
        }
    }
}

/// Complete inputs to the selected-unit category contribution. Other movement
/// stages authenticate these identities independently before this query.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MovementCategoryKey {
    base: usize,
    selected: usize,
    country_accumulator: usize,
    leader_accumulator: usize,
    leader: usize,
}

/// Successful category queries retained only inside one ETA acquisition.
/// A collision follows the original cold reader and replaces the slot only
/// after complete success. Slot storage grows only outside acquisition.
pub(crate) struct MovementCategoryContributions {
    entries: Vec<MovementCategoryContribution>,
    generation: u64,
}

#[derive(Clone, Copy, Default)]
struct MovementCategoryContribution {
    key: MovementCategoryKey,
    value: i64,
    generation: u64,
}

impl MovementCategoryContributions {
    fn new(rows: usize) -> std::io::Result<Self> {
        Ok(Self {
            entries: crate::allocation::filled(
                movement_memo_slots(rows)?,
                MovementCategoryContribution::default(),
            )?,
            generation: 1,
        })
    }

    fn clear(&mut self) {
        self.generation = match self.generation.checked_add(1) {
            Some(generation) => generation,
            None => {
                for entry in &mut self.entries {
                    entry.generation = 0;
                }
                1
            }
        };
    }

    fn reserve(&mut self, rows: usize) -> std::io::Result<()> {
        if self.entries.len() < rows {
            crate::allocation::resize(
                &mut self.entries,
                movement_memo_slots(rows)?,
                MovementCategoryContribution::default(),
            )?;
            self.clear();
        }
        Ok(())
    }

    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<MovementCategoryContribution>()
    }

    fn slot(&self, key: MovementCategoryKey) -> Option<usize> {
        let mask = self.entries.len().checked_sub(1)?;
        // Rotate each identity separately so aliased accumulators still mix.
        // Equality below, rather than this index, establishes a cache hit.
        let hash = key.selected as u64
            ^ (key.country_accumulator as u64).rotate_left(11)
            ^ (key.leader_accumulator as u64).rotate_left(23)
            ^ (key.leader as u64).rotate_left(37)
            ^ (key.base as u64).rotate_left(49);
        let mixed = (hash ^ (hash >> 32)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        Some((mixed ^ (mixed >> 32)) as usize & mask)
    }
}

#[derive(Clone, Copy)]
struct MovementStateModifier {
    base: usize,
    game_state: usize,
    state: usize,
    owner: u32,
    key: u32,
    value: i64,
}

/// Complete state queries shared only inside one ETA acquisition. Each range
/// has one slot per admitted-row capacity; collisions use the native reader
/// and replace that slot. No query or failure allocates or starts a second walk.
pub(crate) struct MovementStateModifiers {
    entries: Vec<Option<MovementStateModifier>>,
    requests: u64,
    hits: u64,
}

impl MovementStateModifiers {
    fn new(rows: usize) -> std::io::Result<Self> {
        Ok(Self {
            entries: crate::allocation::filled(movement_memo_slots(rows)?, None)?,
            requests: 0,
            hits: 0,
        })
    }

    fn clear(&mut self) {
        self.entries.fill(None);
        self.requests = 0;
        self.hits = 0;
    }

    fn reserve(&mut self, rows: usize) -> std::io::Result<()> {
        if self.entries.len() < rows {
            crate::allocation::resize(&mut self.entries, movement_memo_slots(rows)?, None)?;
            self.clear();
        }
        Ok(())
    }

    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Option<MovementStateModifier>>()
    }

    fn get(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        state: usize,
        owner: u32,
        key: u32,
    ) -> Result<i64, &'static str> {
        self.get_with(base, game_state, state, owner, key, || {
            crate::cstate::movement_modifier(memory, base, game_state, state, owner, key)
        })
    }

    fn get_with(
        &mut self,
        base: usize,
        game_state: usize,
        state: usize,
        owner: u32,
        key: u32,
        acquire: impl FnOnce() -> Result<i64, &'static str>,
    ) -> Result<i64, &'static str> {
        self.requests = self.requests.saturating_add(1);
        let hash = (state as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ u64::from(owner).wrapping_mul(0xbf58_476d_1ce4_e5b9)
            ^ u64::from(key).wrapping_mul(0x94d0_49bb_1331_11eb);
        let slot = self
            .entries
            .len()
            .checked_sub(1)
            .map(|mask| (hash ^ (hash >> 32)) as usize & mask);
        if let Some(entry) = slot.and_then(|slot| self.entries[slot])
            && (
                entry.base,
                entry.game_state,
                entry.state,
                entry.owner,
                entry.key,
            ) == (base, game_state, state, owner, key)
        {
            self.hits = self.hits.saturating_add(1);
            return Ok(entry.value);
        }
        let value = acquire()?;
        if let Some(slot) = slot {
            self.entries[slot] = Some(MovementStateModifier {
                base,
                game_state,
                state,
                owner,
                key,
                value,
            });
        }
        Ok(value)
    }
}

#[allow(clippy::too_many_arguments)]
fn movement_state_modifier(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: usize,
    owner: u32,
    key: u32,
    shared: Option<&MovementSharedInputs<'_>>,
) -> Result<i64, &'static str> {
    match shared.and_then(|shared| shared.state_modifiers) {
        Some(memo) => memo
            .borrow_mut()
            .get(memory, base, game_state, state, owner, key),
        None => crate::cstate::movement_modifier(memory, base, game_state, state, owner, key),
    }
}

fn movement_shared_seed_value<T>(
    shared: &mut Option<&mut MovementSharedInputs<'_>>,
    select: impl FnOnce(&DivisionSeed) -> Option<T>,
    acquire: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if let Some(shared) = shared {
        select(shared.seed).ok_or("division presentation field unreadable")
    } else {
        acquire()
    }
}

fn movement_shared_flags(
    memory: &impl Memory,
    base: usize,
    province: usize,
    shared: &mut Option<&mut MovementSharedInputs<'_>>,
) -> Result<u8, &'static str> {
    if province == 0 {
        return Err("movement province identity mismatch");
    }
    if let Some(shared) = shared {
        let id = shared.geometry.province_id(province)?;
        shared.geometry.flags(id)
    } else {
        movement_flags(memory, base, province)
    }
}

fn movement_shared_definition(
    memory: &impl Memory,
    base: usize,
    province: usize,
    shared: &mut Option<&mut MovementSharedInputs<'_>>,
) -> Result<usize, &'static str> {
    if province == 0 {
        return Err("movement province identity mismatch");
    }
    if let Some(shared) = shared {
        let id = shared.geometry.province_id(province)?;
        shared.geometry.definition(id)
    } else {
        movement_definition(memory, base, province)
    }
}

fn movement_shared_route(
    memory: &impl Memory,
    army: usize,
    shared: &mut Option<&mut MovementSharedInputs<'_>>,
) -> Result<[u8; 16], &'static str> {
    if let Some(shared) = shared {
        shared
            .seed
            .ordinary_route
            .ok_or("division presentation array invalid")
    } else {
        army.checked_add(cunit::ORDINARY_ROUTE_DATA)
            .and_then(|address| read(memory, address))
            .ok_or("division presentation array invalid")
    }
}

fn movement_shared_route_province(
    memory: &impl Memory,
    base: usize,
    state: usize,
    route: Array,
    index: u32,
    shared: &mut Option<&mut MovementSharedInputs<'_>>,
) -> Result<usize, &'static str> {
    if let Some(shared) = shared {
        route
            .element(index, 4)
            .ok_or("movement route index invalid")?;
        let id = shared
            .route_ids
            .get(memory, index as usize)?
            .ok_or("movement route id unreadable")?;
        let province = shared.geometry.route_pointer(id)?;
        if province != 0 {
            shared.geometry.definition(id)?;
        }
        Ok(province)
    } else {
        movement_route_province(memory, base, state, route, index)
    }
}

fn movement_shared_attacking(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
) -> Result<bool, &'static str> {
    if shared.is_none() {
        return movement_attacking(memory, base, state, army);
    }
    let route = Array::from_header(movement_shared_route(memory, army, &mut shared)?, 4)
        .ok_or("division presentation array invalid")?;
    if route.count == 0 {
        return Ok(false);
    }
    let destination =
        movement_shared_route_province(memory, base, state, route, route.count - 1, &mut shared)?;
    if destination == 0 {
        return Ok(false);
    }
    let controller = presentation_field::<i32>(memory, destination, cprovince::CONTROLLER)?;
    if controller <= 0 {
        return Ok(false);
    }
    let owner = movement_shared_seed_value(
        &mut shared,
        |seed| seed.logical_owner,
        || presentation_field(memory, army, cunit::LOGICAL_OWNER),
    )?;
    ccountry::movement_enemy(memory, base, state, controller as u32, owner)
}

/// Callback-local storage for one army's composed graph, so a branch the
/// current edge skipped is read once per division and never per leg. Never
/// retained in captured/spooled rows; a new instance per acquisition pass.
#[derive(Default, Clone, Copy)]
pub(crate) struct NormalMovementContext {
    army: usize,
    transfer: Option<Result<usize, &'static str>>,
    transport_owner: Option<Result<u32, &'static str>>,
    transport_scale: Option<Result<i64, &'static str>>,
    land: Option<Result<MovementLandInputs, &'static str>>,
    invasion_speed: Option<Result<i64, &'static str>>,
    roots: Option<Result<MovementOwnerInputs, &'static str>>,
    rail: Option<Result<MovementRailInputs, &'static str>>,
    normal: Option<Result<NormalMovementInputs, &'static str>>,
    /// Successful terrain contribution from this army's current normal edge.
    /// The route scan consumes it by exact terrain pointer in this same pass.
    terrain: Option<(usize, i64)>,
    edge_observations: fresh_counter_eta::NormalEdgeObservations,
    current: Option<fresh_counter_eta::CurrentSpeedObservation>,
    /// IDs from a successful legacy route copy in this same pass.
    route_endpoints: Option<(u32, u32)>,
    legacy_terrain: Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
struct MovementLandInputs {
    location: usize,
    next_flags: Option<u8>,
}
impl MovementLandInputs {
    fn invasion(self) -> bool {
        self.next_flags.is_some_and(|flags| flags & 1 != 0)
    }
}

fn movement_land_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
) -> Result<MovementLandInputs, &'static str> {
    let location = movement_shared_seed_value(
        &mut shared,
        |seed| seed.location,
        || movement_pointer(memory, army, cunit::LOCATION),
    )?;
    let route_header = movement_shared_route(memory, army, &mut shared)?;
    let route = Array::from_header(route_header, 4).ok_or("division presentation array invalid")?;
    let mut result = MovementLandInputs {
        location,
        next_flags: None,
    };
    if route.count != 0 {
        let class = movement_shared_seed_value(
            &mut shared,
            |seed| seed.class,
            || presentation_field::<u32>(memory, army, cunit::CLASS),
        )?;
        if class == 0 {
            let next = movement_shared_route_province(memory, base, state, route, 0, &mut shared)?;
            if next != 0 {
                let flags = movement_shared_flags(memory, base, location, &mut shared)?;
                if flags & 3 == 0 {
                    result.next_flags =
                        Some(movement_shared_flags(memory, base, next, &mut shared)?);
                }
            }
        }
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct MovementOwnerInputs {
    roster: u32,
    owner_raw: u32,
    owner: ccountry::Country,
    actual_state: usize,
    strategic: u8,
}
fn movement_owner_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    location: usize,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
) -> Result<MovementOwnerInputs, &'static str> {
    let expeditionary = movement_shared_seed_value(
        &mut shared,
        |seed| seed.expeditionary_owner.map(|value| value as i32),
        || presentation_field::<i32>(memory, army, cunit::EXPEDITIONARY_OWNER),
    )?;
    let roster = movement_shared_seed_value(
        &mut shared,
        |seed| seed.roster_owner,
        || presentation_field(memory, army, cunit::ROSTER_OWNER),
    )?;
    let owner_raw = if expeditionary > 0 {
        expeditionary as u32
    } else {
        roster
    };
    let owner = ccountry::Reader::new(memory, base, state)?.resolve(owner_raw)?;
    // Retain the original checked-address failure before actual-state reads.
    movement_address(owner.pointer, ccountry::MODIFIER_ACCUMULATOR)?;
    let actual_state = movement_pointer(memory, location, cprovince::STATE)?;
    let strategic = movement_shared_seed_value(
        &mut shared,
        |seed| seed.strategic_redeployment,
        || presentation_field(memory, army, STRATEGIC_REDEPLOYMENT),
    )?;
    Ok(MovementOwnerInputs {
        roster,
        owner_raw,
        owner,
        actual_state,
        strategic,
    })
}

#[derive(Clone, Copy)]
struct MovementRailInputs {
    country: i64,
    state: Option<i64>,
    friendly_state: Option<i64>,
}
impl MovementRailInputs {
    fn blocked(self) -> bool {
        self.country != 0
            || self.state.is_some_and(|value| value != 0)
            || self.friendly_state.is_some_and(|value| value != 0)
    }
}
fn movement_rail_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    roots: MovementOwnerInputs,
    shared: Option<&MovementSharedInputs<'_>>,
) -> Result<MovementRailInputs, &'static str> {
    let inputs = strategic_action::read_inputs(
        memory,
        base,
        state,
        roots.owner_raw,
        roots.owner.pointer,
        || Ok(roots.roster),
        || Ok(roots.actual_state),
        shared.and_then(|shared| shared.state_modifiers),
    )
    .map_err(StrategicActionFailure::reason)?;
    Ok(MovementRailInputs {
        country: inputs.country_modifier,
        state: inputs.state_modifier,
        friendly_state: inputs.friendly_state_modifier,
    })
}

fn movement_cached<T: Copy>(
    slot: Option<&mut Option<Result<T, &'static str>>>,
    acquire: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let Some(slot) = slot else {
        return acquire();
    };
    if let Some(result) = *slot {
        return result;
    }
    let result = acquire();
    *slot = Some(result);
    result
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn effective_movement_speed_in_pass(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    from: usize,
    to: usize,
    context: &mut NormalMovementContext,
    transport_speed: impl FnMut(u32, usize) -> Result<i64, &'static str>,
) -> Result<i64, &'static str> {
    movement_speed_inputs_internal(
        memory,
        base,
        state,
        army,
        from,
        to,
        None,
        Some(context),
        transport_speed,
    )
    .map(MovementSpeedInputs::evaluate)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn movement_speed_inputs_seeded_in_pass(
    memory: &impl Memory,
    base: usize,
    state: usize,
    shared: &mut MovementSharedInputs<'_>,
    from: usize,
    to: usize,
    context: &mut NormalMovementContext,
    transport_speed: impl FnMut(u32, usize) -> Result<i64, &'static str>,
) -> Result<MovementSpeedInputs, &'static str> {
    if context.army == shared.seed.pointer
        && let Some(current) = context.current
        && (current.from, current.to) == (from, to)
    {
        return current.result;
    }
    let result = movement_speed_inputs_internal(
        memory,
        base,
        state,
        shared.seed.pointer,
        from,
        to,
        Some(shared),
        Some(&mut *context),
        transport_speed,
    );
    context.current = Some(fresh_counter_eta::CurrentSpeedObservation { from, to, result });
    result
}

#[allow(clippy::too_many_arguments)]
fn movement_speed_inputs_internal(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    from: usize,
    to: usize,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
    mut context: Option<&mut NormalMovementContext>,
    mut transport_speed: impl FnMut(u32, usize) -> Result<i64, &'static str>,
) -> Result<MovementSpeedInputs, &'static str> {
    let same_army = army != 0
        && context
            .as_deref()
            .is_some_and(|context| context.army == army);
    // The cache belongs to one army; a new army starts from an empty one.
    if !same_army && let Some(context) = context.as_deref_mut() {
        *context = NormalMovementContext::default();
    }
    if !same_army {
        let vtable = if let Some(shared) = shared.as_deref_mut() {
            shared.seed.primary_vtable
        } else {
            read::<u64>(memory, army)
        };
        if army == 0 || vtable != base.checked_add(VTABLE_RVA).map(|p| p as u64) {
            return Err("movement army identity mismatch");
        }
    }
    if let Some(context) = context.as_deref_mut()
        && context.army != army
    {
        context.army = army;
    }
    // Clear bit zero selects transport. The polarity is
    // intentionally expressed in raw flags, independently of terrain naming.
    let transport_flags = (from != 0
        && movement_shared_flags(memory, base, from, &mut shared)? & 1 == 0)
        || (to != 0 && movement_shared_flags(memory, base, to, &mut shared)? & 1 == 0);
    let transfer = movement_cached(
        context.as_deref_mut().map(|context| &mut context.transfer),
        || {
            movement_shared_seed_value(
                &mut shared,
                |seed| seed.naval_transfer_membership,
                || movement_pointer(memory, army, NAVAL_TRANSFER_MEMBERSHIP),
            )
        },
    )?;
    let transport = transfer != 0 || transport_flags;
    if transport {
        if from == 0 {
            return Err("native transport source province is null");
        }
        let owner = movement_cached(
            context
                .as_deref_mut()
                .map(|context| &mut context.transport_owner),
            || {
                movement_shared_seed_value(
                    &mut shared,
                    |seed| seed.logical_owner,
                    || presentation_field(memory, army, cunit::LOGICAL_OWNER),
                )
            },
        )?;
        let region = movement_pointer(memory, from, cprovince::STRATEGIC_REGION)?;
        let speed = transport_speed(owner, region)?;
        let scale = movement_cached(
            context
                .as_deref_mut()
                .map(|context| &mut context.transport_scale),
            || presentation_field(memory, base, TRANSPORT_SCALE_RVA),
        )?;
        return Ok(MovementSpeedInputs::Scaled { speed, scale });
    }
    let land = movement_cached(
        context.as_deref_mut().map(|context| &mut context.land),
        || movement_land_inputs(memory, base, state, army, shared.as_deref_mut()),
    )?;
    if land.invasion() {
        return movement_cached(
            context
                .as_deref_mut()
                .map(|context| &mut context.invasion_speed),
            || presentation_field(memory, base, INVASION_SPEED_RVA),
        )
        .map(MovementSpeedInputs::Fixed);
    }
    let location = land.location;
    let roots = movement_cached(
        context.as_deref_mut().map(|context| &mut context.roots),
        || movement_owner_inputs(memory, base, state, army, location, shared.as_deref_mut()),
    )?;
    let owner_raw = roots.owner_raw;
    let owner = roots.owner;
    let actual_state = roots.actual_state;
    if roots.strategic != 0 && to != 0 {
        let rail = movement_cached(
            context.as_deref_mut().map(|context| &mut context.rail),
            || movement_rail_inputs(memory, base, state, roots, shared.as_deref()),
        )?;
        if !rail.blocked() {
            let speed = match shared
                .as_deref_mut()
                .filter(|_| from != to)
                .and_then(|shared| shared.rail_speeds.as_deref_mut())
            {
                Some(memo) => memo.get(memory, base, state, from, to)?,
                None => crate::crailway_manager::movement_speed(memory, base, state, from, to)?,
            };
            return Ok(MovementSpeedInputs::Scaled {
                speed,
                scale: presentation_field(memory, base, OUTER_SCALE_RVA)?,
            });
        }
    }
    // Do not prepare normal operands for transport, invasion, rail, or the
    // native no-destination path. Those branches do not consume this graph.
    if to != 0
        && let Some(context) = context.as_deref_mut()
    {
        let common = movement_cached(Some(&mut context.normal), || {
            normal_movement_inputs(
                memory,
                base,
                state,
                army,
                owner_raw,
                owner.pointer,
                actual_state,
                shared.as_deref_mut(),
            )
        })?;
        let edge = normal_movement_edge(
            memory,
            base,
            state,
            from,
            to,
            &common.modifier,
            shared.as_deref_mut(),
            Some(&mut context.terrain),
            Some(&mut context.edge_observations),
        )?;
        return Ok(MovementSpeedInputs::Normal {
            common: common.speed,
            edge,
        });
    }
    let thin = (|| {
        let stats = movement_shared_seed_value(
            &mut shared,
            |seed| seed.stat_block,
            || movement_pointer(memory, army, cunit::STAT_BLOCK),
        )?;
        let (_, max_org) = crate::csub_unit_definition::division_maxima(memory, base, stats)?;
        let mut speed = presentation_field::<i64>(memory, stats, 0x290)?;
        let attacking =
            movement_shared_attacking(memory, base, state, army, shared.as_deref_mut())?;
        if to != 0 {
            speed = movement_product(
                speed,
                normal_movement_modifier(
                    memory,
                    base,
                    state,
                    army,
                    from,
                    to,
                    stats,
                    owner_raw,
                    owner.pointer,
                    actual_state,
                    attacking,
                )?,
            );
        }
        let organisation = movement_shared_seed_value(
            &mut shared,
            |seed| seed.organisation,
            || presentation_field::<i64>(memory, army, ORGANISATION),
        )?;
        let org_fraction =
            if (organisation as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
                MOVEMENT_ONE
            } else {
                movement_fraction(organisation, max_org)?.clamp(0, MOVEMENT_ONE)
            };
        let org_threshold = presentation_field::<i64>(memory, base, 0x0331_cec0)?;
        if org_fraction < org_threshold && attacking {
            let ratio = movement_fraction(org_fraction, org_threshold)?;
            let penalty = movement_product(
                MOVEMENT_ONE.wrapping_sub(ratio),
                presentation_field(memory, base, 0x0331_aa18)?,
            );
            speed = movement_product(MOVEMENT_ONE.wrapping_add(penalty), speed);
        }
        let fuel_maximum = movement_product(
            presentation_field(memory, stats, 0x120)?,
            presentation_field(memory, base, 0x0331_97d0)?,
        );
        let fuel_ratio = if fuel_maximum > 0 {
            let fuel = presentation_field::<i64>(memory, army, FUEL_CANDIDATE)?;
            if (fuel as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
                movement_fraction(fuel, fuel_maximum)?.clamp(0, MOVEMENT_ONE)
            } else {
                MOVEMENT_ONE
            }
        } else {
            MOVEMENT_ONE
        };
        if fuel_ratio < MOVEMENT_ONE {
            let minimum = presentation_field::<i64>(memory, base, 0x0331_9f90)?;
            speed = movement_product(
                movement_product(MOVEMENT_ONE.wrapping_sub(minimum), fuel_ratio)
                    .wrapping_add(minimum),
                speed,
            );
        }
        Ok(MovementSpeedInputs::Scaled {
            speed: speed.max(MOVEMENT_ONE),
            scale: presentation_field(memory, base, OUTER_SCALE_RVA)?,
        })
    })();
    if to == 0
        && let Err(reason) = thin
        && let Some(context) = context
    {
        // No edge getter is reached here. Every thin-path operand is also
        // required by normal_movement_inputs with the same frozen controls.
        // Retain that shared failure without withholding rail/transport.
        context.normal = Some(Err(reason));
    }
    thin
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct NormalMovementInputs {
    modifier: NormalModifierCommon,
    speed: NormalSpeedOperands,
}

#[allow(clippy::too_many_arguments)]
fn normal_movement_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    owner_raw: u32,
    owner: usize,
    actual_state: usize,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
) -> Result<NormalMovementInputs, &'static str> {
    let stats = movement_shared_seed_value(
        &mut shared,
        |seed| seed.stat_block,
        || movement_pointer(memory, army, cunit::STAT_BLOCK),
    )?;
    // The stat block's identity, maxima and base speed are one guarded call.
    let stat_fields = presentation_batch(
        memory,
        [
            (stats, 0, 8),
            (stats, crate::csub_unit_definition::MAXIMUM_ORGANISATION, 16),
            (stats, 0x290, 8),
        ],
    );
    let (_, max_org) =
        crate::csub_unit_definition::division_maxima_gathered(memory, base, stats, &stat_fields)?;
    let base_speed = presentation_batched(memory, &stat_fields, 2)?;
    let attacking = movement_shared_attacking(memory, base, state, army, shared.as_deref_mut())?;
    let modifier = normal_movement_common(
        memory,
        base,
        state,
        army,
        stats,
        owner_raw,
        owner,
        actual_state,
        attacking,
        shared.as_deref_mut(),
    )?;
    let organisation = movement_shared_seed_value(
        &mut shared,
        |seed| seed.organisation,
        || presentation_field::<i64>(memory, army, ORGANISATION),
    )?;
    let org_fraction =
        if (organisation as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
            MOVEMENT_ONE
        } else {
            movement_fraction(organisation, max_org)?.clamp(0, MOVEMENT_ONE)
        };
    // The operands every division reads from here on, one guarded call; the
    // branch-selected ones below keep their own reads.
    let speed_fields = presentation_batch(
        memory,
        [
            (base, 0x0331_cec0, 8),
            (stats, 0x120, 8),
            (base, 0x0331_97d0, 8),
            (base, OUTER_SCALE_RVA, 8),
        ],
    );
    let org_threshold = presentation_batched::<i64, 4>(memory, &speed_fields, 0)?;
    let organisation_factor = if org_fraction < org_threshold && attacking {
        let ratio = movement_fraction(org_fraction, org_threshold)?;
        Some((ratio, presentation_field(memory, base, 0x0331_aa18)?))
    } else {
        None
    };
    let fuel_maximum = movement_product(
        presentation_batched(memory, &speed_fields, 1)?,
        presentation_batched(memory, &speed_fields, 2)?,
    );
    let fuel_ratio = if fuel_maximum > 0 {
        let fuel = presentation_field::<i64>(memory, army, FUEL_CANDIDATE)?;
        if (fuel as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
            movement_fraction(fuel, fuel_maximum)?.clamp(0, MOVEMENT_ONE)
        } else {
            MOVEMENT_ONE
        }
    } else {
        MOVEMENT_ONE
    };
    let fuel_factor = if fuel_ratio < MOVEMENT_ONE {
        let minimum = presentation_field::<i64>(memory, base, 0x0331_9f90)?;
        Some((fuel_ratio, minimum))
    } else {
        None
    };
    let mut speed = NormalSpeedOperands {
        modifier: modifier.value,
        base_speed,
        organisation: organisation_factor,
        fuel: fuel_factor,
        outer_scale: presentation_batched(memory, &speed_fields, 3)?,
        bounded: false,
    };
    speed.bounded = speed.bounded();
    Ok(NormalMovementInputs { modifier, speed })
}

#[allow(clippy::too_many_arguments)]
fn normal_movement_modifier(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    from: usize,
    to: usize,
    stats: usize,
    owner_raw: u32,
    owner: usize,
    actual_state: usize,
    attacking: bool,
) -> Result<i64, &'static str> {
    let common = normal_movement_common(
        memory,
        base,
        state,
        army,
        stats,
        owner_raw,
        owner,
        actual_state,
        attacking,
        None,
    )?;
    Ok(common.value.evaluate().wrapping_add(
        normal_movement_edge(memory, base, state, from, to, &common, None, None, None)?.evaluate(),
    ))
}

/// One complete category contribution in its original native demand order.
/// Only a successful result is retained for this frozen ETA capture. Keeping
/// each modifier query between its original pointer/token reads preserves cold
/// first failures and duplicate category occurrences.
fn movement_category_contribution(
    memory: &impl Memory,
    base: usize,
    selected: usize,
    country_accumulator: usize,
    leader_accumulator: usize,
    leader: usize,
    memo: Option<&std::cell::RefCell<MovementCategoryContributions>>,
) -> Result<i64, &'static str> {
    let key = MovementCategoryKey {
        base,
        selected,
        country_accumulator,
        leader_accumulator,
        leader,
    };
    let (slot, cached) = memo.map_or((None, None), |memo| {
        let memo = memo.borrow();
        let slot = memo.slot(key);
        let cached = slot.and_then(|slot| {
            let entry = &memo.entries[slot];
            (entry.generation == memo.generation && entry.key == key).then_some(entry.value)
        });
        (slot, cached)
    });
    if let Some(value) = cached {
        return Ok(value);
    }
    let mut result = 0_i64;
    if presentation_field::<u8>(memory, selected, 0x5a8)? & 4 != 0 && leader != 0 {
        result = result
            .wrapping_add(movement_modifier(
                memory,
                base,
                leader_accumulator,
                keys::ARMY_ARMOR_SPEED_FACTOR,
            )?)
            .wrapping_add(movement_modifier(
                memory,
                base,
                country_accumulator,
                keys::ARMY_ARMOR_SPEED_FACTOR,
            )?);
    }
    let token = presentation_field::<u32>(memory, selected, 0x5d8)?;
    if token != 0 {
        result = result.wrapping_add(movement_modifier(memory, base, country_accumulator, token)?);
    }
    let categories = presentation_array(memory, selected, 0x508, 8)?;
    for index in 0..categories.count {
        let category = movement_pointer(
            memory,
            categories
                .element(index, 8)
                .ok_or("movement category index invalid")?,
            0,
        )?;
        let token = presentation_field::<u32>(memory, category, 0x60)?;
        if token != 0 {
            result =
                result.wrapping_add(movement_modifier(memory, base, country_accumulator, token)?);
        }
    }
    if let (Some(memo), Some(slot)) = (memo, slot) {
        let mut memo = memo.borrow_mut();
        memo.entries[slot] = MovementCategoryContribution {
            key,
            value: result,
            generation: memo.generation,
        };
    }
    Ok(result)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct NormalModifierCommon {
    value: NormalModifierOperands,
    stats: usize,
    owner: usize,
    leader: usize,
    leader_accumulator: usize,
}

#[allow(clippy::too_many_arguments)]
fn normal_movement_common(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    stats: usize,
    owner_raw: u32,
    owner: usize,
    actual_state: usize,
    attacking: bool,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
) -> Result<NormalModifierCommon, &'static str> {
    let country_accumulator = movement_address(owner, ccountry::MODIFIER_ACCUMULATOR)?;
    let leader_accumulator = if let Some(shared) = shared.as_deref_mut() {
        crate::carmy_leader::movement_accumulator_seeded(memory, base, shared.seed)?
    } else {
        crate::carmy_leader::movement_accumulator(memory, base, army)?
    }
    .unwrap_or(0);
    let group = movement_shared_seed_value(
        &mut shared,
        |seed| seed.orders_group,
        || movement_pointer(memory, army, ORDERS_GROUP),
    )?;
    let leader = if group == 0 {
        0
    } else {
        movement_pointer(memory, group, 0x88)?
    };
    let location = movement_shared_seed_value(
        &mut shared,
        |seed| seed.location,
        || movement_pointer(memory, army, cunit::LOCATION),
    )?;
    let roster = movement_shared_seed_value(
        &mut shared,
        |seed| seed.roster_owner,
        || presentation_field::<u32>(memory, army, cunit::ROSTER_OWNER),
    )?;
    let mut result = MOVEMENT_ONE;
    let mut operands = NormalModifierOperands::default();
    let combats = if let Some(shared) = shared.as_deref_mut() {
        Array::from_header(
            shared
                .seed
                .combat
                .ok_or("division presentation array invalid")?,
            8,
        )
        .ok_or("division presentation array invalid")?
    } else {
        presentation_array(memory, army, cunit::COMBAT_REFERENCE_DATA, 8)?
    };
    if combats.count != 0 && presentation_field::<u8>(memory, army, 0x24d)? == 0 {
        result = 0;
        for index in 0..combats.count {
            let combat = movement_pointer(
                memory,
                combats
                    .element(index, 8)
                    .ok_or("movement combat index invalid")?,
                0,
            )?;
            let vtable =
                read::<u64>(memory, combat).ok_or("movement combat identity unreadable")?;
            let land = vtable == movement_address(base, crate::cland_combat::VTABLE_RVA)? as u64
                || vtable
                    == movement_address(base, crate::cland_border_war_combat::VTABLE_RVA)? as u64;
            if !land && vtable != movement_address(base, crate::cnaval_combat::VTABLE_RVA)? as u64 {
                return Err("movement combat type unrecognized");
            }
            let side = movement_pointer(memory, combat, 0x28)?;
            let members = presentation_array(memory, side, 0x20, 8)?;
            let mut found = false;
            for member in 0..members.count {
                if movement_pointer(
                    memory,
                    members
                        .element(member, 8)
                        .ok_or("movement combat member index invalid")?,
                    0,
                )? == army
                {
                    found = true;
                    break;
                }
            }
            if found {
                // Land primary dispatch returns one, naval returns two. Stop at the
                // first match.
                if land {
                    result = movement_product(
                        presentation_field(memory, base, 0x0331_d150)?,
                        MOVEMENT_ONE,
                    )
                    .wrapping_add(presentation_field::<i64>(memory, combat, 0xb8)?);
                }
                break;
            }
        }
    }
    result = result
        .wrapping_add(movement_modifier(
            memory,
            base,
            country_accumulator,
            keys::ARMY_SPEED_FACTOR,
        )?)
        .wrapping_add(movement_modifier(
            memory,
            base,
            leader_accumulator,
            keys::ARMY_SPEED_FACTOR,
        )?);
    if attacking {
        result = result.wrapping_add(movement_modifier(
            memory,
            base,
            country_accumulator,
            keys::ARMY_ATTACK_SPEED_FACTOR,
        )?);
    }
    if actual_state != 0 {
        result = result.wrapping_add(movement_state_modifier(
            memory,
            base,
            state,
            actual_state,
            roster,
            keys::ARMY_SPEED_FACTOR,
            shared.as_deref(),
        )?);
        if attacking {
            result = result.wrapping_add(movement_state_modifier(
                memory,
                base,
                state,
                actual_state,
                roster,
                keys::ARMY_ATTACK_SPEED_FACTOR,
                shared.as_deref(),
            )?);
        }
        let controller = presentation_field(memory, actual_state, 0xcc)?;
        let key = if ccountry::movement_friendly(memory, base, state, owner_raw, controller)? {
            keys::ARMY_SPEED_FACTOR_FOR_CONTROLLER
        } else if ccountry::movement_enemy(memory, base, state, owner_raw, controller)? {
            keys::ENEMY_ARMY_SPEED_FACTOR
        } else {
            0
        };
        if key != 0 {
            result = result.wrapping_add(movement_state_modifier(
                memory,
                base,
                state,
                actual_state,
                roster,
                key,
                shared.as_deref(),
            )?);
        }
    }
    let acclimatization = movement_acclimatization(memory, army)?;
    let (province_weather, region_weather) =
        crate::cweather_manager::movement_modifiers(memory, base, state, location)?;
    for (index, accumulator) in [province_weather, region_weather].into_iter().enumerate() {
        if accumulator != 0 {
            operands.weather[index * 2] = (
                acclimatization,
                movement_modifier(memory, base, accumulator, keys::ARMY_SPEED_FACTOR)?,
            );
            if attacking {
                operands.weather[index * 2 + 1] = (
                    acclimatization,
                    movement_modifier(memory, base, accumulator, keys::ARMY_ATTACK_SPEED_FACTOR)?,
                );
            }
        }
    }
    let selected = movement_priority_unit(
        memory,
        base,
        movement_shared_seed_value(
            &mut shared,
            |seed| seed.standard_template,
            || movement_pointer(memory, army, STANDARD_TEMPLATE),
        )?,
    )?;
    result = result.wrapping_add(movement_category_contribution(
        memory,
        base,
        selected,
        country_accumulator,
        leader_accumulator,
        leader,
        shared
            .as_deref()
            .and_then(|shared| shared.category_contributions),
    )?);
    operands.air = movement_air_inputs(
        memory,
        base,
        state,
        army,
        stats,
        location,
        shared.as_deref_mut(),
    )?;
    // The census proves whether the retreat branch below reads +0x24d, so a
    // division that is not retreating reads it with the supply gate.
    let not_retreating = shared
        .as_deref()
        .is_some_and(|shared| shared.seed.retreat == Some(0));
    let gates = crate::memory::FieldBatch::gather(
        memory,
        [
            (army.checked_add(0x2ae).unwrap_or(0), 1),
            (
                army.checked_add(0x24d)
                    .filter(|_| not_retreating)
                    .unwrap_or(0),
                1,
            ),
        ],
    );
    if presentation_batched::<u8, 2>(memory, &gates, 0)? == 0 {
        // The two operands this branch reads around the penalty.
        let supply_fields = presentation_batch(memory, [(base, 0x0331_94e0, 8), (army, 0x2e, 1)]);
        operands.supply = (
            presentation_batched(memory, &supply_fields, 0)?,
            movement_supply_penalty(
                memory,
                base,
                army,
                country_accumulator,
                leader_accumulator,
                shared.as_deref_mut(),
            )?,
        );
        if presentation_batched::<u8, 2>(memory, &supply_fields, 1)? == 0
            && !ccountry::has_core_supply_access(
                memory,
                base,
                state,
                owner_raw,
                movement_pointer(memory, location, cprovince::CONTROLLER_AREA)?,
            )?
        {
            let define = if presentation_field::<i64>(memory, army, 0x58)? > 0 {
                0x0331_9618
            } else {
                0x0331_9568
            };
            result = result.wrapping_add(presentation_field::<i64>(memory, base, define)?);
        }
    }
    if movement_shared_seed_value(
        &mut shared,
        |seed| seed.retreat,
        || presentation_field::<u8>(memory, army, RETREAT),
    )? != 0
    {
        result = result
            .wrapping_add(presentation_field::<i64>(memory, base, 0x0331_9758)?)
            .wrapping_add(movement_modifier(
                memory,
                base,
                country_accumulator,
                keys::ARMY_RETREAT_SPEED_FACTOR,
            )?);
    } else if (if not_retreating {
        presentation_batched::<u8, 2>(memory, &gates, 1)?
    } else {
        presentation_field::<u8>(memory, army, 0x24d)?
    }) != 0
    {
        result = result.wrapping_add(presentation_field::<i64>(memory, base, 0x0331_9808)?);
    }
    operands.additive = result;
    Ok(NormalModifierCommon {
        value: operands,
        stats,
        owner,
        leader,
        leader_accumulator,
    })
}

#[allow(clippy::too_many_arguments)]
fn normal_movement_edge(
    memory: &impl Memory,
    base: usize,
    state: usize,
    from: usize,
    to: usize,
    common: &NormalModifierCommon,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
    terrain_output: Option<&mut Option<(usize, i64)>>,
    mut observations: Option<&mut fresh_counter_eta::NormalEdgeObservations>,
) -> Result<NormalEdgeOperands, &'static str> {
    let stats = common.stats;
    let owner = common.owner;
    let leader = common.leader;
    let leader_accumulator = common.leader_accumulator;
    let mut result = 0_i64;
    let mut river_operands = None;
    let definition = movement_shared_definition(memory, base, to, &mut shared)?;
    let terrain = movement_pointer(memory, definition, 0xa8)?;
    let unit_terrain = match observations
        .as_deref()
        .and_then(|observations| observations.terrain)
        .filter(|seen| seen.pointer == terrain)
    {
        Some(seen) => seen.unit,
        None => {
            let result =
                crate::csub_unit_definition::movement_terrain(memory, base, stats, terrain);
            if let Some(observations) = observations.as_deref_mut() {
                observations.terrain = Some(fresh_counter_eta::TerrainObservation {
                    pointer: terrain,
                    unit: result,
                    leader: None,
                });
            }
            result
        }
    };
    let mut terrain_value = unit_terrain?;
    result = result.wrapping_add(terrain_value);
    // The destination's fort array and state are one guarded call.
    let destination = presentation_batch(memory, [(to, 0x1e0, 16), (to, cprovince::STATE, 8)]);
    let buildings = destination
        .get(memory, 0)
        .and_then(|header| Array::from_header(header, 8))
        .ok_or("division presentation array invalid")?;
    let mut fort_level = 0_i32;
    for index in 0..buildings.count {
        let building = movement_pointer(
            memory,
            buildings
                .element(index, 8)
                .ok_or("movement fort index invalid")?,
            0,
        )?;
        let kind = movement_pointer(memory, building, 0x1e0)?;
        fort_level = fort_level.wrapping_add(
            i32::from(presentation_field::<i16>(memory, building, 0x42)?)
                .wrapping_mul(presentation_field::<i32>(memory, kind, 0x308)?),
        );
    }
    if fort_level > 0 {
        result = result.wrapping_add(movement_cached(
            observations
                .as_deref_mut()
                .map(|observations| &mut observations.fort_penalty),
            || presentation_field::<i64>(memory, stats, 0x378),
        )?);
    }
    if crate::cweather_manager::movement_snow(memory, base, state, to)? {
        result = result.wrapping_add(movement_cached(
            observations
                .as_deref_mut()
                .map(|observations| &mut observations.snow_penalty),
            || presentation_field::<i64>(memory, stats, 0x3f0),
        )?);
    }
    if leader != 0 {
        let leader_value = movement_cached(
            observations
                .as_deref_mut()
                .and_then(|observations| observations.terrain.as_mut())
                .map(|terrain| &mut terrain.leader),
            || {
                crate::carmy_leader::movement_terrain(
                    memory,
                    base,
                    leader,
                    presentation_field(memory, terrain, 0x88)?,
                )
            },
        )?;
        terrain_value = terrain_value.wrapping_add(leader_value);
        result = result.wrapping_add(leader_value);
    }
    let crossing = if let Some(shared) = shared {
        let from_id = shared.geometry.province_id(from)?;
        let to_id = shared.geometry.province_id(to)?;
        shared.geometry.crossing(from_id, to_id)?
    } else {
        let from_definition = movement_definition(memory, base, from)?;
        let neighbors = presentation_array(
            memory,
            from_definition,
            crate::province_definition::NEIGHBOURS,
            crate::province_neighbour::STRIDE,
        )?;
        let destination_id = presentation_field::<u32>(memory, to, cprovince::ID)?;
        let mut crossing = None;
        for index in 0..neighbors.count {
            let neighbor = neighbors
                .element(index, crate::province_neighbour::STRIDE)
                .ok_or("movement neighbor index invalid")?;
            if presentation_field::<u32>(memory, neighbor, crate::province_neighbour::PROVINCE_ID)?
                == destination_id
            {
                crossing = Some(presentation_field::<u8>(
                    memory,
                    neighbor,
                    crate::province_neighbour::CROSSING_KIND,
                )?);
                break;
            }
        }
        crossing
    };
    if let Some(crossing) = crossing
        && (1..=3).contains(&crossing)
    {
        let diplomacy = movement_pointer(memory, owner, 0xf88)?;
        let controller = presentation_field::<u32>(memory, to, cprovince::CONTROLLER)?;
        let canonical = if controller == 0 {
            0
        } else {
            ccountry::Reader::new(memory, base, state)?
                .resolve(controller)?
                .canonical_slot
        };
        let relations = presentation_array(memory, diplomacy, 8, 8)?;
        let relation = movement_pointer(
            memory,
            relations
                .element(canonical, 8)
                .ok_or("movement river relation outside roster")?,
            0,
        )?;
        let combined = movement_modifier(
            memory,
            base,
            movement_address(relation, 0x140)?,
            keys::RIVER_CROSSING_FACTOR_AGAINST,
        )?
        .wrapping_add(movement_cached(
            observations
                .as_deref_mut()
                .map(|observations| &mut observations.river_leader),
            || {
                movement_modifier(
                    memory,
                    base,
                    leader_accumulator,
                    keys::RIVER_CROSSING_FACTOR,
                )
            },
        )?);
        let adjustment = movement_cached(
            observations.map(|observations| &mut observations.river_adjustment),
            || presentation_field::<i64>(memory, stats, 0x3a0),
        )?;
        let river = match crossing {
            2 => presentation_field(memory, base, RIVER_CROSSING_TWO_RVA)?,
            3 => presentation_field(memory, base, RIVER_CROSSING_THREE_RVA)?,
            _ => 0,
        };
        river_operands = Some((combined, adjustment, river));
    }
    let destination_state = presentation_batched::<u64, 2>(memory, &destination, 1)? as usize;
    if destination_state != 0 {
        result = result.wrapping_add(crate::cstate::movement_infrastructure(
            memory,
            base,
            destination_state,
        )?);
    }
    if let Some(output) = terrain_output {
        *output = Some((terrain, terrain_value));
    }
    Ok(NormalEdgeOperands {
        additive: result,
        river: river_operands,
    })
}

/// Owned ETA operands for one moving division. Per-terrain and per-leg values
/// live in the packet's arenas; a row addresses only its own span.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EtaInputRow {
    pub division: u64,
    pub speed_modifier: i64,
    pub speed_base: i64,
    pub speed_org: Option<i64>,
    pub speed_fuel: Option<i64>,
    pub current_speed: i64,
    /// Span of (terrain index, composed modifier) pairs in the packet arena.
    pub terrain_start: usize,
    pub terrain_len: usize,
    pub fort_penalty: i64,
    pub snow_penalty: i64,
    pub river_adjustment: i64,
    pub river_leader: i64,
    pub class: u32,
    /// Viewpoint-independent effective strategic-rail modifier operands; replay
    /// evaluates their nonzero test.
    pub rail_country_modifier: Option<i64>,
    pub rail_state_modifier: Option<i64>,
    pub rail_friendly_state_modifier: Option<i64>,
    pub leg_start: usize,
    pub leg_len: usize,
    /// A branch no leg took records an empty vector instead of zeros.
    pub any_river: bool,
    pub any_transport: bool,
    pub any_rail: bool,
}

/// Borrowed per-leg arenas in route order; a row addresses its own span. A
/// span covers the legs the native walk evaluates, so it ends at the first
/// zero-speed leg rather than at the end of the route.
#[derive(Clone, Copy)]
pub struct EtaLegArenas<'a> {
    pub river: &'a [i64],
    pub transport: &'a [i64],
    pub rail: &'a [i64],
}

/// Per-walk attribution for the ETA input acquisition. Diagnostics only: no
/// observation comparison and no row depends on these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EtaInputCounters {
    pub walks: u64,
    pub divisions: u64,
    pub legs: u64,
    /// The per-division composed speed graph: the native decision tree and
    /// the additional branches required for the owned ETA inputs.
    pub graph_ticks: u64,
    /// The per-leg route scan, including its terrain and river operands.
    pub scan_ticks: u64,
    /// The rest of a division: the route copy and the prepared speed tail.
    pub other_ticks: u64,
    pub geometry_hits: u64,
    pub geometry_misses: u64,
    /// The session-static map memo: hits served without reading, misses read
    /// live, and how often the memo was cleared.
    pub static_hits: u64,
    pub static_misses: u64,
    pub static_cleared: u64,
    pub state_modifier_requests: u64,
    pub state_modifier_hits: u64,
    pub air_defense_requests: u64,
    pub air_defense_hits: u64,
}

impl EtaInputCounters {
    pub fn add(&mut self, other: Self) {
        let sum = |left: &mut u64, right: u64| *left = left.saturating_add(right);
        sum(&mut self.walks, other.walks);
        sum(&mut self.divisions, other.divisions);
        sum(&mut self.legs, other.legs);
        sum(&mut self.graph_ticks, other.graph_ticks);
        sum(&mut self.scan_ticks, other.scan_ticks);
        sum(&mut self.other_ticks, other.other_ticks);
        sum(&mut self.geometry_hits, other.geometry_hits);
        sum(&mut self.geometry_misses, other.geometry_misses);
        sum(&mut self.static_hits, other.static_hits);
        sum(&mut self.static_misses, other.static_misses);
        sum(&mut self.static_cleared, other.static_cleared);
        sum(
            &mut self.state_modifier_requests,
            other.state_modifier_requests,
        );
        sum(&mut self.state_modifier_hits, other.state_modifier_hits);
        sum(&mut self.air_defense_requests, other.air_defense_requests);
        sum(&mut self.air_defense_hits, other.air_defense_hits);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EtaInputWalk {
    pub outcome: Outcome,
    pub terrain_len: usize,
    pub required_terrain: usize,
    pub legs_len: usize,
    pub required_legs: usize,
}

/// Pooled owned ETA inputs. Fixed callback packets retain explicit shortages.
/// Worker-owned packets reserve from original census and route controls before
/// publication; no native walk restarts after growth.
pub struct EtaInputPacket {
    units: Vec<EtaInputRow>,
    /// One-based signed terrain index and composed modifier per division span.
    terrain: Vec<(i32, i64)>,
    leg_count: usize,
    leg_river: Vec<i64>,
    leg_transport: Vec<i64>,
    leg_rail: Vec<i64>,
    route: Vec<u32>,
    /// Same-handle acquired graph results retained until fresh projection.
    fresh_observations: Vec<fresh_counter_eta::DivisionEtaObservation>,
    /// Frame-local (logical owner, strategic region) transport speeds.
    transport: crate::cnaval_unit_transfer::MovementSpeeds,
    rail: crate::crailway_manager::MovementSpeeds,
    state_modifiers: std::cell::RefCell<MovementStateModifiers>,
    category_contributions: std::cell::RefCell<MovementCategoryContributions>,
    supply_templates: std::cell::RefCell<std::collections::HashMap<usize, i64>>,
    air_prefixes: std::cell::RefCell<MovementAirMemo>,
    counters: EtaInputCounters,
    result: EtaInputWalk,
    growing: bool,
}

const ETA_INPUT_UNCAPTURED: EtaInputWalk = EtaInputWalk {
    outcome: Outcome::Unavailable {
        reason: "division ETA inputs not captured",
    },
    terrain_len: 0,
    required_terrain: 0,
    legs_len: 0,
    required_legs: 0,
};

impl EtaInputPacket {
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn new(
        unit_capacity: usize,
        terrain_capacity: usize,
        leg_capacity: usize,
    ) -> std::io::Result<Self> {
        let mut supply_templates = std::collections::HashMap::new();
        crate::allocation::reserve_map(&mut supply_templates, unit_capacity)?;
        Ok(Self {
            units: crate::allocation::vec(unit_capacity)?,
            terrain: crate::allocation::vec(terrain_capacity)?,
            leg_count: 0,
            leg_river: crate::allocation::vec(leg_capacity)?,
            leg_transport: crate::allocation::vec(leg_capacity)?,
            leg_rail: crate::allocation::vec(leg_capacity)?,
            // Every admitted route fits this leg capacity before its resize.
            route: crate::allocation::vec(leg_capacity)?,
            fresh_observations: crate::allocation::vec(unit_capacity)?,
            transport: crate::cnaval_unit_transfer::MovementSpeeds::try_with_capacity(
                unit_capacity,
            )?,
            rail: crate::crailway_manager::MovementSpeeds::try_new(leg_capacity)?,
            state_modifiers: std::cell::RefCell::new(MovementStateModifiers::new(unit_capacity)?),
            category_contributions: std::cell::RefCell::new(MovementCategoryContributions::new(
                unit_capacity,
            )?),
            supply_templates: std::cell::RefCell::new(supply_templates),
            air_prefixes: std::cell::RefCell::new(MovementAirMemo::new(unit_capacity)?),
            counters: EtaInputCounters::default(),
            result: ETA_INPUT_UNCAPTURED,
            growing: false,
        })
    }
    /// Allocation-free placeholder while a job owns the complete packet.
    pub fn vacant() -> Self {
        Self {
            units: Vec::new(),
            terrain: Vec::new(),
            leg_count: 0,
            leg_river: Vec::new(),
            leg_transport: Vec::new(),
            leg_rail: Vec::new(),
            route: Vec::new(),
            fresh_observations: Vec::new(),
            transport: crate::cnaval_unit_transfer::MovementSpeeds::vacant(),
            rail: crate::crailway_manager::MovementSpeeds::vacant(),
            state_modifiers: std::cell::RefCell::new(MovementStateModifiers {
                entries: Vec::new(),
                requests: 0,
                hits: 0,
            }),
            category_contributions: std::cell::RefCell::new(MovementCategoryContributions {
                entries: Vec::new(),
                generation: 1,
            }),
            supply_templates: std::cell::RefCell::new(std::collections::HashMap::new()),
            air_prefixes: std::cell::RefCell::new(MovementAirMemo {
                active: Vec::new(),
                regions: Vec::new(),
                generation: 1,
            }),
            counters: EtaInputCounters::default(),
            result: ETA_INPUT_UNCAPTURED,
            growing: false,
        }
    }

    pub fn clear(&mut self) {
        self.units.clear();
        self.terrain.clear();
        self.leg_count = 0;
        self.leg_river.clear();
        self.leg_transport.clear();
        self.leg_rail.clear();
        self.clear_fresh_observations();
        self.transport.clear();
        self.rail.clear();
        self.state_modifiers.get_mut().clear();
        self.category_contributions.get_mut().clear();
        self.supply_templates.get_mut().clear();
        self.air_prefixes.get_mut().clear();
        self.counters = EtaInputCounters::default();
        self.result = ETA_INPUT_UNCAPTURED;
    }
    /// A range's rows and variable-length inputs have independent capacities.
    pub fn reserve(
        &mut self,
        unit_capacity: usize,
        terrain_capacity: usize,
        leg_capacity: usize,
    ) -> std::io::Result<()> {
        eta_reserve(&mut self.units, unit_capacity)?;
        eta_reserve(&mut self.terrain, terrain_capacity)?;
        eta_reserve(&mut self.leg_river, leg_capacity)?;
        eta_reserve(&mut self.leg_transport, leg_capacity)?;
        eta_reserve(&mut self.leg_rail, leg_capacity)?;
        eta_reserve(&mut self.route, leg_capacity)?;
        eta_reserve(&mut self.fresh_observations, unit_capacity)?;
        self.transport.reserve(unit_capacity)?;
        self.state_modifiers.get_mut().reserve(unit_capacity)?;
        self.category_contributions
            .get_mut()
            .reserve(unit_capacity)?;
        self.air_prefixes.get_mut().reserve(unit_capacity)?;
        let templates = self.supply_templates.get_mut();
        let additional = unit_capacity.saturating_sub(templates.len());
        crate::allocation::reserve_map(templates, additional)?;
        Ok(())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.units.capacity() * size_of::<EtaInputRow>()
            + self.terrain.capacity() * size_of::<(i32, i64)>()
            + self.leg_river.capacity() * 8
            + (self.leg_transport.capacity() + self.leg_rail.capacity()) * 8
            + self.route.capacity() * 4
            + self.fresh_observations.capacity()
                * size_of::<fresh_counter_eta::DivisionEtaObservation>()
            + self.transport.allocated_bytes()
            + self.rail.allocated_bytes()
            + self.state_modifiers.borrow().allocated_bytes()
            + self.category_contributions.borrow().allocated_bytes()
            + self.supply_templates.borrow().capacity() * (size_of::<(usize, i64)>() + 1)
            + self.air_prefixes.borrow().allocated_bytes()
    }
    pub fn outcome(&self) -> EtaInputWalk {
        self.result
    }
    pub fn counters(&self) -> EtaInputCounters {
        self.counters
    }
    pub fn rows(&self) -> &[EtaInputRow] {
        &self.units
    }
    pub fn terrain(&self) -> &[(i32, i64)] {
        &self.terrain
    }
    pub fn leg_count(&self) -> usize {
        self.leg_count
    }
    pub fn legs(&self) -> EtaLegArenas<'_> {
        EtaLegArenas {
            river: &self.leg_river,
            transport: &self.leg_transport,
            rail: &self.leg_rail,
        }
    }

    /// End the handoff on success, reset, or a collector bypass/panic path.
    /// Storage remains prepared; no legacy rows or growth demand are cleared.
    pub fn clear_fresh_observations(&mut self) {
        self.fresh_observations.clear();
    }

    fn take_fresh_context(
        &mut self,
        division: u64,
        pointer: usize,
    ) -> Option<NormalMovementContext> {
        let index = self
            .fresh_observations
            .binary_search_by_key(&(division, pointer), |row| (row.division, row.pointer))
            .ok()?;
        self.fresh_observations[index].context.take()
    }
}

fn eta_reserve<T>(vector: &mut Vec<T>, capacity: usize) -> std::io::Result<()> {
    if vector.capacity() < capacity {
        crate::allocation::reserve_exact(vector, capacity - vector.len())?;
    }
    Ok(())
}

struct EtaGeometry<'a, M> {
    packet: &'a mut crate::cprovince::GeometryPacket,
    memory: &'a M,
}
impl<M: Memory> MovementGeometry for EtaGeometry<'_, M> {
    fn province_id(&mut self, pointer: usize) -> Result<u32, &'static str> {
        self.packet.acquire_province_id(self.memory, pointer)
    }
    fn route_pointer(&mut self, id: u32) -> Result<usize, &'static str> {
        self.packet.acquire_route_pointer(self.memory, id)
    }
    fn definition(&mut self, id: u32) -> Result<usize, &'static str> {
        self.packet.acquire_definition(self.memory, id)
    }
    fn flags(&mut self, id: u32) -> Result<u8, &'static str> {
        self.packet.acquire_flags(self.memory, id)
    }
    fn crossing(&mut self, from: u32, to: u32) -> Result<Option<u8>, &'static str> {
        self.packet.acquire_crossing(self.memory, from, to)
    }
}

/// The scales a leg's speed can be proved zero from, read once per walk. A
/// missing one only costs that branch its stop proof, never correctness.
#[derive(Clone, Copy, Default)]
struct EtaStopScales {
    transport: Option<i64>,
    invasion: Option<i64>,
    outer: Option<i64>,
}
impl EtaStopScales {
    fn read(memory: &impl Memory, base: usize) -> Self {
        let define = |rva| presentation_field::<i64>(memory, base, rva).ok();
        Self {
            transport: define(TRANSPORT_SCALE_RVA),
            invasion: define(INVASION_SPEED_RVA),
            outer: define(OUTER_SCALE_RVA),
        }
    }
    fn scaled_is_zero(speed: i64, scale: Option<i64>) -> bool {
        scale.is_some_and(|scale| movement_product(speed, scale) == 0)
    }
}

/// Acquire everything a replayer needs to recompute the native ETA of every
/// moving division in one contiguous `span` of the census: the composed
/// per-division speed graph, the per-terrain and per-edge constants, and one
/// thin route scan for the per-leg operands. Only divisions with a nonempty
/// route produce a row. The scan ends where the native walk ends, at the first
/// leg proved to have zero speed. Ranges retain their original census order.
#[allow(clippy::too_many_arguments)]
pub fn capture_eta_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    seeds: &[DivisionSeed],
    span: std::ops::Range<usize>,
    source_outcome: Outcome,
    row_capacity: usize,
    terrain_capacity: usize,
    leg_capacity: usize,
    geometry: &mut crate::cprovince::GeometryPacket,
    statics: &mut crate::cprovince::StaticGeometry,
    packet: &mut EtaInputPacket,
) -> EtaInputWalk {
    packet.clear();
    let scales = EtaStopScales::read(memory, base);
    // Re-key the session memo before any early return, so its counters always
    // describe this walk.
    let statics_ready = statics.begin(memory, base, state);
    let mut required_legs = 0_usize;
    let result = (|| {
        if let Outcome::Unavailable { reason } = source_outcome {
            return Err(reason);
        }
        if source_outcome.len() != seeds.len() {
            return Err("division ETA input source length invalid");
        }
        if span.start > span.end || span.end > seeds.len() {
            return Err("division ETA input range outside census");
        }
        geometry
            .ready()
            .map_err(|_| "division ETA input root invalid")?;
        if !statics_ready {
            return Err("division ETA input static root invalid");
        }
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if packet.growing {
            packet
                .reserve(span.len(), packet.terrain.len(), packet.route.len())
                .map_err(|_| "division ETA allocation failed")?;
        }
        let mut len = 0_usize;
        // The census's own unread rows belong to the first range only, so
        // concatenated ranges report them exactly once.
        let mut missing = match source_outcome {
            Outcome::Truncated { unread_rows, .. } if span.start == 0 => unread_rows,
            _ => 0,
        };
        for seed in seeds.iter().take(span.end).skip(span.start) {
            let admitted = (|| -> Result<Option<EtaInputRow>, &'static str> {
                if seed.pointer == 0
                    || seed.division == 0
                    || seed.primary_vtable != Some(expected)
                    || seed.handle != Some(seed.division)
                {
                    return Err("division ETA input identity changed");
                }

                let route = seed
                    .ordinary_route
                    .and_then(|header| Array::from_header(header, 4))
                    .ok_or("division ETA input route invalid")?;
                if route.count == 0 {
                    return Ok(None);
                }
                required_legs = required_legs
                    .checked_add(route.count as usize)
                    .ok_or("division ETA input leg count overflow")?;
                if packet.growing {
                    // Movement and transport consume this original route header.
                    packet
                        .reserve(span.len(), packet.terrain.len(), required_legs)
                        .map_err(|_| "division ETA allocation failed")?;
                }
                let leg_storage = leg_capacity
                    .min(packet.leg_river.capacity())
                    .min(packet.leg_transport.capacity())
                    .min(packet.leg_rail.capacity())
                    .min(packet.route.capacity());
                if len == row_capacity.min(packet.units.capacity())
                    || leg_storage.saturating_sub(packet.leg_count) < route.count as usize
                {
                    missing = missing.checked_add(1).ok_or("ETA missing count overflow")?;
                    return Ok(None);
                }
                if packet.fresh_observations.len() == packet.fresh_observations.capacity() {
                    return Err("division ETA observation arena exhausted");
                }
                let mut context = NormalMovementContext::default();
                let result = eta_input_row(
                    memory,
                    base,
                    state,
                    seed,
                    route,
                    terrain_capacity,
                    scales,
                    geometry,
                    statics,
                    packet,
                    &mut context,
                );
                packet
                    .fresh_observations
                    .push(fresh_counter_eta::DivisionEtaObservation {
                        division: seed.division,
                        pointer: seed.pointer,
                        context: Some(context),
                    });
                result.map(Some)
            })();
            if let Some(row) = admitted? {
                packet.units.push(row);
                len += 1;
            }
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    let walk = EtaInputWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        terrain_len: packet.terrain.len(),
        // One spare pair, so an exhausted arena reports growth and a healthy
        // one does not.
        required_terrain: packet.terrain.len().saturating_add(1),
        legs_len: packet.leg_count,
        required_legs,
    };
    let geometry_counters = geometry.counters();
    let static_counters = statics.counters();
    packet.counters.static_hits = static_counters.hits;
    packet.counters.static_misses = static_counters.misses;
    packet.counters.static_cleared = static_counters.cleared;
    packet.counters.walks = 1;
    packet.counters.divisions = packet.units.len() as u64;
    packet.counters.legs = packet.leg_count as u64;
    packet.counters.geometry_hits = geometry_counters.hits;
    packet.counters.geometry_misses = geometry_counters.misses;
    packet.counters.state_modifier_requests = packet.state_modifiers.borrow().requests;
    packet.counters.state_modifier_hits = packet.state_modifiers.borrow().hits;
    packet.result = walk;
    walk
}

/// One division's operands. The context caches this army's common graph, so a
/// branch the current edge skipped is read once here and never per leg.
#[allow(clippy::too_many_arguments)]
fn eta_input_row(
    memory: &impl Memory,
    base: usize,
    state: usize,
    seed: &DivisionSeed,
    route: Array,
    terrain_capacity: usize,
    scales: EtaStopScales,
    geometry: &mut crate::cprovince::GeometryPacket,
    statics: &mut crate::cprovince::StaticGeometry,
    packet: &mut EtaInputPacket,
    context: &mut NormalMovementContext,
) -> Result<EtaInputRow, &'static str> {
    // The route ids leave the pool for this division, so a seeded acquisition
    // can borrow them while the arenas keep taking legs.
    let started = crate::clock::qpc();
    let measured = packet
        .counters
        .graph_ticks
        .saturating_add(packet.counters.scan_ticks);
    let mut route_ids = std::mem::take(&mut packet.route);
    route_ids.clear();
    // capture_eta_inputs admitted this route against packet.route.capacity().
    debug_assert!(route.count as usize <= route_ids.capacity());
    route_ids.resize(route.count as usize, 0);
    let result = crate::cpdx_array::copy_u32s(
        memory,
        route,
        &mut route_ids,
        "division ETA input route unreadable",
    )
    .and_then(|()| {
        let terrain_start = packet.terrain.len();
        let result = eta_input_route(
            memory,
            base,
            state,
            seed,
            terrain_capacity,
            scales,
            &route_ids,
            geometry,
            statics,
            packet,
            context,
        );
        context.route_endpoints = Some((route_ids[0], *route_ids.last().unwrap()));
        context.legacy_terrain = Some((terrain_start, packet.terrain.len() - terrain_start));
        result
    });
    packet.route = route_ids;
    // Everything this division spent outside the two measured spans.
    let spans = packet
        .counters
        .graph_ticks
        .saturating_add(packet.counters.scan_ticks)
        .saturating_sub(measured);
    packet.counters.other_ticks = packet
        .counters
        .other_ticks
        .saturating_add(crate::clock::since(started).saturating_sub(spans));
    result
}

#[allow(clippy::too_many_arguments)]
fn eta_input_route(
    memory: &impl Memory,
    base: usize,
    state: usize,
    seed: &DivisionSeed,
    terrain_capacity: usize,
    scales: EtaStopScales,
    route_ids: &[u32],
    geometry: &mut crate::cprovince::GeometryPacket,
    statics: &mut crate::cprovince::StaticGeometry,
    packet: &mut EtaInputPacket,
    context: &mut NormalMovementContext,
) -> Result<EtaInputRow, &'static str> {
    let class = seed.class.ok_or("division ETA input field unreadable")?;
    let location = seed.location.ok_or("division ETA input field unreadable")?;
    if location == 0 {
        return Err("division ETA input moving division has null location");
    }
    let location_id = geometry.acquire_province_id(memory, location)?;
    let next = geometry.acquire_pointer(memory, route_ids[0])?;
    let mut row = EtaInputRow {
        division: seed.division,
        class,
        leg_start: packet.leg_count,
        terrain_start: packet.terrain.len(),
        ..EtaInputRow::default()
    };
    // Prime the context through the native decision tree, which also yields the
    // one (location -> next) speed the duration subtracts as progress. A branch
    // it skipped is acquired only when a later route leg actually needs it.
    let graph_started = crate::clock::qpc();
    let current = {
        let mut reader = EtaGeometry {
            packet: geometry,
            memory,
        };
        let mut shared = MovementSharedInputs {
            seed,
            route_ids: fresh_counter_eta::MovementRouteIds::Full(route_ids),
            geometry: &mut reader,
            rail_speeds: Some(&mut packet.rail),
            state_modifiers: Some(&packet.state_modifiers),
            category_contributions: Some(&packet.category_contributions),
            supply_templates: Some(&packet.supply_templates),
            air_prefixes: Some(&packet.air_prefixes),
        };
        let transport = &mut packet.transport;
        movement_speed_inputs_seeded_in_pass(
            memory,
            base,
            state,
            &mut shared,
            location,
            next,
            context,
            |owner, region| eta_transport_speed(memory, base, state, transport, owner, region),
        )?
    };
    let scan_started = crate::clock::qpc();
    packet.counters.graph_ticks = packet
        .counters
        .graph_ticks
        .saturating_add(scan_started.saturating_sub(graph_started).max(0) as u64);
    let prepared = match current {
        MovementSpeedInputs::Normal { common, edge } => {
            let prepared = common.prepare();
            row.current_speed = prepared.evaluate(edge);
            Some(prepared)
        }
        _ => {
            row.current_speed = current.evaluate();
            None
        }
    };

    let transfer = seed
        .naval_transfer_membership
        .ok_or("division ETA input field unreadable")?;
    let mut normal: Option<NormalMovementInputs> = context.normal.transpose()?;
    let mut river = None;
    let mut from = location;
    let mut from_id = location_id;
    let mut source_flags = None;
    for &id in route_ids {
        if id == 0 {
            return Err("division ETA input province identity mismatch");
        }
        // The session memo supplies static map data after begin() validates
        // the current session and province-census key.
        let destination = statics.province(memory, base, id)?;
        let to = destination.pointer;
        if to == 0 {
            return Err("division ETA input province identity mismatch");
        }
        if from != to {
            let from_flags = match source_flags {
                Some(flags) => flags,
                None => statics.province(memory, base, from_id)?.flags,
            };
            let to_flags = destination.flags;
            // Leg distances and crossing kinds are static map content; the
            // replay derives them from the matching content stack.
            let leg = packet.leg_count;
            packet.leg_count += 1;
            packet.leg_river.push(0);
            packet.leg_transport.push(0);
            packet.leg_rail.push(0);
            let transport = transfer != 0 || from_flags & 1 == 0 || to_flags & 1 == 0;
            let mut strategic = 0;
            let mut rail_blocked = false;
            let mut invasion = false;
            if !transport {
                // Transport never consumes the army's land-owner or rail
                // graph. In a mixed route, acquire it at the first land leg.
                let mut reader = EtaGeometry {
                    packet: geometry,
                    memory,
                };
                let mut shared = MovementSharedInputs {
                    seed,
                    route_ids: fresh_counter_eta::MovementRouteIds::Full(route_ids),
                    geometry: &mut reader,
                    rail_speeds: None,
                    state_modifiers: Some(&packet.state_modifiers),
                    category_contributions: Some(&packet.category_contributions),
                    supply_templates: Some(&packet.supply_templates),
                    air_prefixes: Some(&packet.air_prefixes),
                };
                let land = movement_cached(Some(&mut context.land), || {
                    movement_land_inputs(memory, base, state, seed.pointer, Some(&mut shared))
                })?;
                invasion = land.invasion();
                if !invasion {
                    let roots = movement_cached(Some(&mut context.roots), || {
                        movement_owner_inputs(
                            memory,
                            base,
                            state,
                            seed.pointer,
                            land.location,
                            Some(&mut shared),
                        )
                    })?;
                    strategic = roots.strategic;
                    if strategic != 0 {
                        let rail = movement_cached(Some(&mut context.rail), || {
                            movement_rail_inputs(memory, base, state, roots, Some(&shared))
                        })?;
                        row.rail_country_modifier = Some(rail.country);
                        row.rail_state_modifier = rail.state;
                        row.rail_friendly_state_modifier = rail.friendly_state;
                        rail_blocked = rail.blocked();
                    }
                }
            }
            // The route walk stops at the first zero-speed leg
            // and never reads the tail. Record that leg, then stop: reading on
            // would make an unreadable later operand deny the whole table.
            let stopped = if transport {
                let owner_raw = seed
                    .logical_owner
                    .ok_or("division ETA input field unreadable")?;
                let region = movement_pointer(memory, from, cprovince::STRATEGIC_REGION)?;
                packet.leg_transport[leg] = eta_transport_speed(
                    memory,
                    base,
                    state,
                    &mut packet.transport,
                    owner_raw,
                    region,
                )?;
                row.any_transport = true;
                EtaStopScales::scaled_is_zero(packet.leg_transport[leg], scales.transport)
            } else if invasion {
                // An authored define the replay reads from matching content.
                scales.invasion == Some(0)
            } else if strategic != 0 && !rail_blocked {
                packet.leg_rail[leg] = packet.rail.get(memory, base, state, from, to)?;
                row.any_rail = true;
                EtaStopScales::scaled_is_zero(packet.leg_rail[leg], scales.outer)
            } else {
                let common = match normal {
                    Some(common) => common,
                    None => {
                        let roots = context
                            .roots
                            .ok_or("division ETA input normal roots missing")??;
                        let acquired = {
                            let mut reader = EtaGeometry {
                                packet: geometry,
                                memory,
                            };
                            let mut shared = MovementSharedInputs {
                                seed,
                                route_ids: fresh_counter_eta::MovementRouteIds::Full(route_ids),
                                geometry: &mut reader,
                                rail_speeds: None,
                                state_modifiers: Some(&packet.state_modifiers),
                                category_contributions: Some(&packet.category_contributions),
                                supply_templates: Some(&packet.supply_templates),
                                air_prefixes: Some(&packet.air_prefixes),
                            };
                            normal_movement_inputs(
                                memory,
                                base,
                                state,
                                seed.pointer,
                                roots.owner_raw,
                                roots.owner.pointer,
                                roots.actual_state,
                                Some(&mut shared),
                            )
                        };
                        context.normal = Some(acquired);
                        let common = acquired?;
                        normal = Some(common);
                        common
                    }
                };
                eta_terrain_modifier(
                    memory,
                    base,
                    destination,
                    &common.modifier,
                    context.terrain,
                    row.terrain_start,
                    terrain_capacity,
                    packet,
                )?;
                let crossing = geometry.acquire_crossing(memory, from_id, id)?;
                if crossing.is_some_and(|crossing| (1..=3).contains(&crossing)) {
                    let roster = match &river {
                        Some(roster) => roster,
                        None => river.insert({
                            let reader = ccountry::Reader::new(memory, base, state)?;
                            let owner = reader.of_pointer(common.modifier.owner)?;
                            let relations = crate::crelation_status::MovementRelations::new(
                                memory, base, owner,
                            )?;
                            EtaRiverRoster {
                                reader,
                                relations,
                                definition: std::cell::Cell::new(None),
                            }
                        }),
                    };
                    packet.leg_river[leg] = eta_river_combined(memory, base, roster, to)?;
                    row.any_river = true;
                }
                if from == location && to == next {
                    // This edge was already evaluated for current progress.
                    row.current_speed == 0
                } else if let Some(zero) = normal_speed_zero_from_scale(common.speed.outer_scale) {
                    zero
                } else {
                    // An exceptional scale can wrap a positive normal speed
                    // to zero. Keep the thin scan for the proved range, but
                    // evaluate this edge exactly before touching its tail.
                    let mut reader = EtaGeometry {
                        packet: geometry,
                        memory,
                    };
                    let mut shared = MovementSharedInputs {
                        seed,
                        route_ids: fresh_counter_eta::MovementRouteIds::Full(route_ids),
                        geometry: &mut reader,
                        rail_speeds: None,
                        state_modifiers: Some(&packet.state_modifiers),
                        category_contributions: Some(&packet.category_contributions),
                        supply_templates: Some(&packet.supply_templates),
                        air_prefixes: Some(&packet.air_prefixes),
                    };
                    let edge = normal_movement_edge(
                        memory,
                        base,
                        state,
                        from,
                        to,
                        &common.modifier,
                        Some(&mut shared),
                        None,
                        None,
                    )?;
                    common.speed.prepare().evaluate(edge) == 0
                }
            };
            if stopped {
                break;
            }
        }
        from = to;
        from_id = id;
        source_flags = Some(destination.flags);
    }
    packet.counters.scan_ticks = packet
        .counters
        .scan_ticks
        .saturating_add(crate::clock::since(scan_started));
    row.leg_len = packet.leg_count - row.leg_start;
    if let Some(common) = normal {
        let prepared = prepared.unwrap_or_else(|| common.speed.prepare());
        row.speed_modifier = prepared.modifier;
        row.speed_base = prepared.base_speed;
        row.speed_org = prepared.organisation;
        row.speed_fuel = prepared.fuel;
        let stats = common.modifier.stats;
        (row.fort_penalty, row.snow_penalty, row.river_adjustment) =
            normal_eta_penalties(memory, stats, &mut context.edge_observations)?;
        row.river_leader =
            movement_cached(Some(&mut context.edge_observations.river_leader), || {
                movement_modifier(
                    memory,
                    base,
                    common.modifier.leader_accumulator,
                    keys::RIVER_CROSSING_FACTOR,
                )
            })?;
    }
    row.terrain_len = packet.terrain.len() - row.terrain_start;
    Ok(row)
}

/// One guarded batch for only the common penalty fields not observed yet.
/// A failed prefix retains the first failed Result and leaves its tail absent.
fn normal_eta_penalties(
    memory: &impl Memory,
    stats: usize,
    observations: &mut fresh_counter_eta::NormalEdgeObservations,
) -> Result<(i64, i64, i64), &'static str> {
    for result in [
        observations.fort_penalty,
        observations.snow_penalty,
        observations.river_adjustment,
    ] {
        if let Some(Err(reason)) = result {
            return Err(reason);
        }
    }
    let fields = presentation_batch(
        memory,
        [
            (
                stats,
                0x378,
                if observations.fort_penalty.is_none() {
                    8
                } else {
                    0
                },
            ),
            (
                stats,
                0x3f0,
                if observations.snow_penalty.is_none() {
                    8
                } else {
                    0
                },
            ),
            (
                stats,
                0x3a0,
                if observations.river_adjustment.is_none() {
                    8
                } else {
                    0
                },
            ),
        ],
    );
    let fort = movement_cached(Some(&mut observations.fort_penalty), || {
        presentation_batched(memory, &fields, 0)
    })?;
    let snow = movement_cached(Some(&mut observations.snow_penalty), || {
        presentation_batched(memory, &fields, 1)
    })?;
    let river = movement_cached(Some(&mut observations.river_adjustment), || {
        presentation_batched(memory, &fields, 2)
    })?;
    Ok((fort, snow, river))
}

/// Frame-local memo: the composed transport speed depends only on the owner and
/// the source strategic region, so repeated water legs cost nothing.
fn eta_transport_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    memo: &mut crate::cnaval_unit_transfer::MovementSpeeds,
    owner: u32,
    region: usize,
) -> Result<i64, &'static str> {
    memo.get(memory, base, state, owner, region)
}

/// The composed sub-unit plus leader terrain modifier for one destination,
/// recorded once per distinct native terrain index on this division's route.
/// The destination's terrain is map data the caller already holds.
#[allow(clippy::too_many_arguments)]
fn eta_terrain_modifier(
    memory: &impl Memory,
    base: usize,
    destination: crate::cprovince::StaticProvince,
    common: &NormalModifierCommon,
    acquired: Option<(usize, i64)>,
    start: usize,
    terrain_capacity: usize,
    packet: &mut EtaInputPacket,
) -> Result<(), &'static str> {
    let (terrain, index) = (destination.terrain, destination.terrain_index);
    if index <= 0 {
        return Err("division ETA input terrain index is the null slot or negative");
    }
    if packet.terrain[start..]
        .iter()
        .any(|(seen, _)| *seen == index)
    {
        return Ok(());
    }
    let value = match acquired.filter(|(pointer, _)| *pointer == terrain) {
        Some((_, value)) => value,
        None => {
            let mut value =
                crate::csub_unit_definition::movement_terrain(memory, base, common.stats, terrain)?;
            if common.leader != 0 {
                value = value.wrapping_add(crate::carmy_leader::movement_terrain(
                    memory,
                    base,
                    common.leader,
                    index,
                )?);
            }
            value
        }
    };
    if packet.growing {
        let required = packet
            .terrain
            .len()
            .checked_add(1)
            .ok_or("division ETA terrain demand overflow")?;
        eta_reserve(&mut packet.terrain, required).map_err(|_| "division ETA allocation failed")?;
    }
    if packet.terrain.len() == terrain_capacity.min(packet.terrain.capacity()) {
        return Err("division ETA input terrain capacity exhausted");
    }
    packet.terrain.push((index, value));
    Ok(())
}

/// The owner's relation roster and the country reader are the same on every
/// crossing leg of one route, so a division reads them at most once.
struct EtaRiverRoster<'a, M: Memory> {
    reader: ccountry::Reader<'a, M>,
    relations: crate::crelation_status::MovementRelations,
    definition: std::cell::Cell<Option<Result<crate::modifier_definition::Operands, &'static str>>>,
}

/// River relation term for one crossing leg. The leader's share is
/// recorded separately in `river_leader` and combined during replay.
fn eta_river_combined<M: Memory>(
    memory: &M,
    base: usize,
    roster: &EtaRiverRoster<'_, M>,
    to: usize,
) -> Result<i64, &'static str> {
    let controller = presentation_field::<u32>(memory, to, cprovince::CONTROLLER)?;
    let canonical = if controller == 0 {
        0
    } else {
        roster.reader.resolve(controller)?.canonical_slot
    };
    let relation = roster
        .relations
        .status(memory, base, &roster.reader, canonical)?;
    let mut definition = roster.definition.get();
    let value = country_movement_inputs::river_modifier(memory, base, relation, &mut definition);
    roster.definition.set(definition);
    value
}

fn movement_air_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    stats: usize,
    location: usize,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
) -> Result<MovementAirOperands, &'static str> {
    let owner_raw = movement_shared_seed_value(
        &mut shared,
        |seed| seed.logical_owner,
        || presentation_field(memory, army, cunit::LOGICAL_OWNER),
    )?;
    // Every division still resolves its current raw owner before either memo.
    let owner = ccountry::Reader::new(memory, base, state)?.resolve(owner_raw)?;
    let key = MovementAirKey {
        base,
        state,
        owner_raw,
        owner_pointer: owner.pointer,
        canonical: owner.canonical_slot,
    };
    let memo = shared.as_deref().and_then(|shared| shared.air_prefixes);
    let active = movement_air_active(memory, key, memo)?;
    // This pointer is an independent observation even when active is false.
    let region = movement_pointer(memory, location, cprovince::STRATEGIC_REGION)?;
    if !active || region == 0 {
        return Ok(MovementAirOperands::default());
    }
    let Some(prefix) = movement_air_region(memory, key, region, memo)? else {
        return Ok(MovementAirOperands::default());
    };
    Ok(MovementAirOperands {
        fraction: prefix.fraction,
        impact: prefix.impact,
        scale: prefix.scale,
        // The regional result never supplies this division's stats or defines.
        defense: crate::csub_unit_definition::movement_air_defense(memory, base, stats)?,
    })
}

fn movement_air_active(
    memory: &impl Memory,
    key: MovementAirKey,
    memo: Option<&std::cell::RefCell<MovementAirMemo>>,
) -> Result<bool, &'static str> {
    if let Some(active) = memo.and_then(|memo| memo.borrow().active(key)) {
        return Ok(active);
    }
    let diplomacy = movement_pointer(memory, key.owner_pointer, 0xf88)?;
    let mut active = presentation_field::<i32>(memory, diplomacy, 0xa4)? != 0;
    if !active {
        let relations = presentation_array(memory, diplomacy, 0x20, 8)?;
        for index in 0..relations.count {
            let relation = movement_pointer(
                memory,
                relations
                    .element(index, 8)
                    .ok_or("movement air diplomacy index invalid")?,
                0,
            )?;
            if presentation_field::<u32>(memory, relation, 8)? == 0x380a {
                active = true;
                break;
            }
        }
    }
    // Only the successful lazy prefix is retained, never a failed relation.
    if let Some(memo) = memo {
        memo.borrow_mut().install_active(key, active);
    }
    Ok(active)
}

fn movement_air_region(
    memory: &impl Memory,
    key: MovementAirKey,
    region: usize,
    memo: Option<&std::cell::RefCell<MovementAirMemo>>,
) -> Result<Option<MovementAirPrefix>, &'static str> {
    if let Some(prefix) = memo.and_then(|memo| memo.borrow().region(key, region)) {
        return Ok(prefix);
    }
    let manager = movement_pointer(memory, key.state, cgame_state::STRATEGIC_AIR_MANAGER)?;
    let countries = presentation_array(memory, manager, 0x30, 8)?;
    let country = movement_pointer(
        memory,
        countries
            .element(key.canonical, 8)
            .ok_or("movement air country outside roster")?,
        0,
    )?;
    let index = presentation_field::<i32>(memory, region, 0x60)?;
    if index < 0 {
        return Err("movement air region index negative");
    }
    let regions = presentation_array(memory, country, 0xe0, 0xa0)?;
    let entry = regions
        .element(index as u32, 0xa0)
        .ok_or("movement air region outside cache")?;
    let friendly = presentation_field::<i64>(memory, entry, 0)?;
    let hostile = presentation_field::<i64>(memory, entry, 8)?;
    let prefix = if hostile <= 0 {
        None
    } else {
        let total = friendly.wrapping_add(hostile);
        let fraction = if total > 0 {
            movement_fraction(hostile, total)?
        } else {
            0
        };
        Some(MovementAirPrefix {
            fraction,
            impact: presentation_field(memory, entry, 0x18)?,
            scale: presentation_field(memory, key.base, 0x0331_ddd8)?,
        })
    };
    // The complete common prefix can survive a later division-specific defense
    // failure; that independent defense query remains fresh on every hit.
    if let Some(memo) = memo {
        memo.borrow_mut().install_region(key, region, prefix);
    }
    Ok(prefix)
}

fn movement_acclimatization(memory: &impl Memory, army: usize) -> Result<i64, &'static str> {
    let object = movement_address(army, ACCLIMATIZATION)?;
    let active = movement_pointer(memory, object, 0x20)?;
    if active == 0 {
        return Ok(MOVEMENT_ONE);
    }
    let entries = presentation_array(memory, object, 8, 0x10)?;
    let mut chosen = 0;
    let mut weight = 0_i64;
    for index in 0..entries.count {
        let entry = entries
            .element(index, 0x10)
            .ok_or("movement acclimatization index invalid")?;
        let value = presentation_field::<i64>(memory, entry, 8)?;
        if value > weight {
            weight = value;
            chosen = movement_pointer(memory, entry, 0)?;
        }
    }
    if chosen != active {
        return Ok(MOVEMENT_ONE);
    }
    let fraction = movement_fraction(weight, presentation_field(memory, object, 0x38)?)?;
    let scaled = movement_product(fraction, presentation_field(memory, active, 0x30)?);
    Ok(if scaled < 0 {
        MOVEMENT_ONE
    } else {
        MOVEMENT_ONE - scaled.min(MOVEMENT_ONE)
    })
}

pub(crate) fn movement_priority_unit(
    memory: &impl Memory,
    base: usize,
    template: usize,
) -> Result<usize, &'static str> {
    // The template's identity and priority unit are one guarded call.
    let fields = presentation_batch(
        memory,
        [
            (template, 0, 8),
            (template, creferenced_division_template::PRIORITY_UNIT, 8),
        ],
    );
    if template == 0
        || fields.get::<u64>(memory, 0)
            != base
                .checked_add(creferenced_division_template::VTABLE_RVA)
                .map(|p| p as u64)
    {
        return Err("movement template identity mismatch");
    }
    let mut selected = presentation_batched::<u64, 2>(memory, &fields, 1)? as usize;
    if selected == 0 {
        let grid = presentation_array(
            memory,
            template,
            creferenced_division_template::REGIMENT_DATA,
            8,
        )?;
        let mut count = 0_u16;
        for i in 0..grid.count {
            let p = movement_pointer(
                memory,
                grid.element(i, 8)
                    .ok_or("movement regiment index invalid")?,
                0,
            )?;
            if p != 0 {
                count = count.wrapping_add(u16::from(
                    presentation_field::<u8>(memory, p, crate::csub_unit_definition::IS_REAL)? != 0,
                ));
            }
        }
        selected = if count != 0 {
            movement_pointer(
                memory,
                grid.element(0, 8).ok_or("movement first regiment absent")?,
                0,
            )?
        } else {
            movement_pointer(memory, base, crate::csub_unit_definition::NULL_OBJECT_RVA)?
        };
    }
    let vtable = read::<u64>(memory, selected);
    if selected == 0
        || (vtable
            != base
                .checked_add(crate::csub_unit_definition::VTABLE_RVA)
                .map(|p| p as u64)
            && vtable
                != base
                    .checked_add(crate::csub_unit_definition::NULL_VTABLE_RVA)
                    .map(|p| p as u64))
    {
        return Err("movement selected unit identity mismatch");
    }
    Ok(selected)
}

fn movement_supply_penalty(
    memory: &impl Memory,
    base: usize,
    army: usize,
    country_accumulator: usize,
    leader_accumulator: usize,
    mut shared: Option<&mut MovementSharedInputs<'_>>,
) -> Result<i64, &'static str> {
    if movement_shared_seed_value(
        &mut shared,
        |seed| seed.naval_transfer_membership,
        || movement_pointer(memory, army, NAVAL_TRANSFER_MEMBERSHIP),
    )? != 0
    {
        return Ok(0);
    }
    // Every untransferred division reads these three; the supply gain terms
    // are read, as their own guarded call, only by a division with supply.
    let fields = presentation_batch(
        memory,
        [(army, 0x600, 8), (army, 0x498, 4), (base, 0x0331_9394, 4)],
    );
    let available = if presentation_batched::<i64, 3>(memory, &fields, 0)? < 1 {
        0
    } else {
        let gain = presentation_batch(
            memory,
            [(base, 0x0331_91c0, 4), (army, 0x610, 8), (army, 0x40, 8)],
        );
        let hours =
            i64::from(presentation_batched::<i32, 3>(memory, &gain, 0)?).wrapping_mul(MOVEMENT_ONE);
        movement_fraction(presentation_batched(memory, &gain, 1)?, hours)?
            .max(presentation_batched(memory, &gain, 2)?)
            .clamp(0, MOVEMENT_ONE)
    };
    let numerator =
        i64::from(presentation_batched::<i32, 3>(memory, &fields, 1)?).wrapping_mul(MOVEMENT_ONE);
    let denominator =
        i64::from(presentation_batched::<i32, 3>(memory, &fields, 2)?).wrapping_mul(MOVEMENT_ONE);
    let penalty = movement_product(
        MOVEMENT_ONE.wrapping_sub(available).clamp(0, MOVEMENT_ONE),
        movement_fraction(numerator, denominator)?,
    );
    if penalty == 0 {
        return Ok(0);
    }
    let template = movement_shared_seed_value(
        &mut shared,
        |seed| seed.standard_template,
        || movement_pointer(memory, army, STANDARD_TEMPLATE),
    )?;
    let ratio = movement_supply_template_ratio(
        memory,
        template,
        shared.as_deref().and_then(|shared| shared.supply_templates),
    )?;
    let mut multiplier = MOVEMENT_ONE.wrapping_add(movement_modifier(
        memory,
        base,
        country_accumulator,
        keys::OUT_OF_SUPPLY_FACTOR,
    )?);
    if ratio > 0 {
        multiplier = multiplier.wrapping_add(movement_product(
            ratio,
            movement_modifier(
                memory,
                base,
                country_accumulator,
                keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
            )?,
        ));
    }
    if leader_accumulator != 0 {
        multiplier = multiplier.wrapping_add(movement_modifier(
            memory,
            base,
            leader_accumulator,
            keys::OUT_OF_SUPPLY_FACTOR,
        )?);
        if ratio > 0 {
            multiplier = multiplier.wrapping_add(movement_product(
                ratio,
                movement_modifier(
                    memory,
                    base,
                    leader_accumulator,
                    keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
                )?,
            ));
        }
    }
    Ok(movement_product(multiplier.max(0), penalty))
}

/// The complete ordered composition depends only on this template during the
/// frozen hourly acquisition. Keep every division's supply gates before this
/// lookup and its country/leader modifiers after it. Distinct template pointers
/// retain distinct observations even when their arrays or unit pointers alias.
fn movement_supply_template_ratio(
    memory: &impl Memory,
    template: usize,
    memo: Option<&std::cell::RefCell<std::collections::HashMap<usize, i64>>>,
) -> Result<i64, &'static str> {
    if template == 0 {
        return Err("movement supply template is null");
    }
    if let Some(ratio) = memo.and_then(|memo| memo.borrow().get(&template).copied()) {
        return Ok(ratio);
    }
    let units = presentation_array(memory, template, 0xc0, 0x10)?;
    let mut total = 0_i32;
    let mut special = 0_i32;
    for index in 0..units.count {
        let entry = units
            .element(index, 0x10)
            .ok_or("movement supply composition index invalid")?;
        let unit = movement_pointer(memory, entry, 0)?;
        let count = presentation_field::<i32>(memory, entry, 8)?;
        total = total.wrapping_add(count);
        if presentation_field::<u8>(memory, unit, 0x5b7)? != 0 {
            special = special.wrapping_add(count);
        }
    }
    let ratio = if total == 0 {
        0
    } else {
        movement_fraction(
            i64::from(special).wrapping_mul(MOVEMENT_ONE),
            i64::from(total).wrapping_mul(MOVEMENT_ONE),
        )?
    };
    if let Some(memo) = memo {
        let mut memo = memo.borrow_mut();
        // Storage is reserved from admitted-row capacity off capture. A full
        // or vacant packet uses the original reader without growing or retrying.
        if memo.len() < memo.capacity() {
            memo.insert(template, ratio);
        }
    }
    Ok(ratio)
}

mod strategic_action;
pub use strategic_action::{StrategicAction, StrategicActionFailure, StrategicRedeploymentInputs};

mod movement;
pub use movement::{
    MovementPacket, capture_movement_inputs, capture_movement_inputs_with_combat_references,
    project_movement,
};

#[cfg(test)]
use movement::movement_route_prefixes;
#[cfg(test)]
pub use movement::{walk_movement, walk_movement_with_inputs};

#[cfg(test)]
pub(crate) use cunit::FirstCombatKey as MovementCombatKey;
/// The capture's shared first-combat memo; see [`cunit::FirstCombatMemo`].
pub(crate) use cunit::FirstCombatMemo as MovementCombats;

fn movement_seed_value<T>(value: Option<T>) -> Result<T, &'static str> {
    value.ok_or("division presentation field unreadable")
}

#[cfg(test)]
fn movement_scalar_tail(memory: &impl Memory, army: usize) -> Result<(u8, u8), &'static str> {
    use crate::memory::RecordField;
    // Movement priority is an independent optional style suffix, acquired only
    // after the complete movement prefix in both the owned and reference walks.
    const FIELDS: [RecordField; 2] = [
        RecordField {
            source_offset: RETREAT,
            destination_offset: 0,
            len: 1,
        },
        RecordField {
            source_offset: STRATEGIC_REDEPLOYMENT,
            destination_offset: 1,
            len: 1,
        },
    ];
    let mut bytes = [0; 2];
    const LAYOUT: crate::memory::RecordLayout<'static> =
        crate::memory::RecordLayout::new(&FIELDS, 2).expect("valid movement scalar layout");
    if memory.copy_records(
        crate::memory::RecordSources::Pointers(&[army]),
        &LAYOUT,
        &mut bytes,
    ) != FIELDS.len()
    {
        return Err("division presentation field unreadable");
    }
    Ok((bytes[0], bytes[1]))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatusRow {
    pub division: u64,
    pub maximum_hit_points: i64,
    pub maximum_organisation: i64,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub supply_ratio: i64,
    pub supply_grace_maximum: i64,
    pub supply_grace_remaining: i64,
    pub attrition_value: i64,
    pub attrition_threshold: i64,
    pub attrition: u8,
    pub exercising: u8,
    /// The no-orders predicate is false for an ungrouped unit, so its glyph needs
    /// the unit's group.
    pub has_order: u8,
    /// Invalid-orders byte gating the yellow glyph.
    pub invalid_orders: u8,
    /// Raw [`cunit::INVALID_ORDERS_REASON`] when the invalid-order gate is
    /// nonzero; None otherwise or on this field's isolated guarded failure.
    pub invalid_orders_reason: Option<i32>,
    /// Raw [`STRENGTH_FACTOR`], fixed5.
    pub strength_factor: i64,
    /// [`RETAINED_VISUAL_VARIANT`], the stored land model number.
    pub model_number: i32,
    /// Acclimatization maximum, raw i64; `None` when this division's
    /// acclimatization was unreadable or outgrew the packet this hour. Only
    /// this field and its span fail, never the row.
    pub acclimatization_maximum: Option<i64>,
    /// Span of this division's (category token, weight) entries in the packet;
    /// empty when the maximum is `None`.
    pub acclimatization_start: usize,
    pub acclimatization_len: usize,
    /// Raid army-reach inputs, `MOVE_*` bits. The commander's retreat bans
    /// are [`RetreatBanRow`]'s, read daily.
    pub move_flags: u8,
    pub border_combat_stats: Option<[i64; 3]>,
    /// Operand spans in the collector's hour-local name arena, never cached display text.
    pub naming: Option<crate::cname_group_member::Inputs>,
    pub country_ordinal: Option<u32>,
    /// Exact stored supply-footer scalars; only this pack fails on a scalar fault.
    pub supply: Option<crate::cunit_supply_inputs::Inputs>,
    pub supply_origins_start: usize,
    pub supply_failure: Option<crate::cunit_supply_inputs::Failure>,
}

impl StatusRow {
    /// [`EXILED`] is nonzero.
    pub const MOVE_EXILED: u8 = 1;
    /// The stat block's [`crate::csub_unit_definition::CAN_EXFILTRATE_FROM_COAST`] is nonzero.
    pub const MOVE_CAN_EXFILTRATE_FROM_COAST: u8 = 2;
}

/// A division whose commander bans retreat: retain the bits of its selected
/// unit-modifier block independently of combat membership. Divisions without
/// a ban emit no row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetreatBanRow {
    pub division: u64,
    /// The owner the caller keyed the division's read by.
    pub owner: u32,
    /// [`crate::carmy_leader::RETREAT_BANNED_DEFENDING`] and
    /// [`crate::carmy_leader::RETREAT_BANNED_ATTACKING`].
    pub bans: u8,
}

use crate::corders_group_member::{MembershipCapacity, MembershipPacket, MembershipQuery};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatusCapacities {
    pub units: usize,
    /// Ordered distributed-supply entries across the authenticated divisions.
    pub supply_origins: usize,
    pub manpower: usize,
    /// Acclimatization (category, weight) entries across all divisions.
    pub acclimatization: usize,
    /// Registered acclimatization categories.
    pub acclimatization_categories: usize,
    pub thresholds: usize,
    pub modifiers: crate::modifier_accumulator::SnapshotCapacity,
    pub membership: MembershipCapacity,
}

impl StatusCapacities {
    /// Only the acquired division census is known before its nested headers.
    pub fn new(units: usize) -> Self {
        Self {
            units,
            supply_origins: 0,
            manpower: 0,
            acclimatization: 0,
            acclimatization_categories: 0,
            thresholds: 0,
            modifiers: crate::modifier_accumulator::SnapshotCapacity::default(),
            membership: MembershipCapacity {
                queries: units,
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StatusLevelOperands {
    manpower_start: usize,
    manpower_len: usize,
    experience: Option<i64>,
    thresholds_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StatusExerciseOperands {
    maximum_level: i32,
    autonomous: bool,
    level: Option<StatusLevelOperands>,
    deployment_present: Option<bool>,
    strength: Option<i64>,
    minimum_strength: Option<i64>,
    stop_at_maximum: Option<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StatusUnitInputs {
    division: u64,
    maxima: usize,
    threshold: usize,
    group: usize,
    membership: usize,
    attrition_value: i64,
    entrenchment: i64,
    entrenchment_cap: i64,
    supply_ratio: i64,
    supply_grace_maximum: i64,
    supply_grace_remaining: i64,
    training: u8,
    invalid_orders: u8,
    invalid_orders_reason: Option<i32>,
    strength_factor: i64,
    exiled: u8,
    model_number: i32,
    acclimatization_maximum: Option<i64>,
    acclimatization_start: usize,
    acclimatization_len: usize,
    exercise: Option<StatusExerciseOperands>,
    failure: Option<&'static str>,
    border_combat_stats: Option<[i64; 3]>,
    supply: Option<crate::cunit_supply_inputs::Inputs>,
    supply_origins_start: usize,
    supply_failure: Option<crate::cunit_supply_inputs::Failure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusCountryInputs {
    pointer: usize,
    modifier: crate::modifier_accumulator::SnapshotObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusGroupInputs {
    pointer: usize,
    training: Result<u8, &'static str>,
    stop_at_maximum: Option<Result<u8, &'static str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusAttritionMetadata {
    feature_selector: u32,
    flags: Option<u32>,
}

/// Owned Status inputs, indexed by the authenticated division prefix. No live
/// addresses are interpreted during projection. Native child admission and
/// branch pruning finish here before the game resumes.
pub struct StatusPacket {
    growing: bool,
    capacities: StatusCapacities,
    required: StatusCapacities,
    units: Vec<StatusUnitInputs>,
    supply_origins: Vec<crate::cunit_supply_inputs::Origin>,
    supply_origins_len: usize,
    border_index: Vec<(u64, usize)>,
    border_stats: Vec<Option<[i64; 3]>>,
    maxima: Vec<crate::csub_unit_definition::DivisionMaxima>,
    maxima_index: Vec<usize>,
    countries: Vec<StatusCountryInputs>,
    country_index: Vec<usize>,
    groups: Vec<StatusGroupInputs>,
    group_index: Vec<usize>,
    /// Commander retreat bans by orders group, for divisions in a combat.
    /// Orders groups of divisions in combat, in first-seen order, and their bans.
    commander_groups: Vec<usize>,
    commander_bans: Vec<Result<Option<crate::carmy_leader::RetreatBans>, &'static str>>,
    commander_index: Vec<usize>,
    /// Per-walk modifier metadata for the ban keys.
    commander_memo: crate::modifier_definition::DefinitionMemo,
    modifiers: crate::modifier_accumulator::SnapshotArena,
    membership: MembershipPacket,
    manpower: Vec<i32>,
    acclimatization: Vec<crate::acclimatization_entry::Entry>,
    /// The category registry, loaded on the hour's first nonempty entry list.
    categories: crate::acclimatization_category::Categories,
    categories_loaded: Option<Result<(), &'static str>>,
    thresholds_header: Option<Result<[u8; 16], &'static str>>,
    thresholds: Vec<Result<i64, &'static str>>,
    attrition_metadata: Option<Result<StatusAttritionMetadata, &'static str>>,
    attrition_define: Option<Result<i64, &'static str>>,
    maximum_level: Option<Result<i32, &'static str>>,
    minimum_strength: Option<Result<i64, &'static str>>,
    outcome: Outcome,
}

fn status_pointer_slot(slots: &[usize], pointer: usize, key: impl Fn(usize) -> usize) -> usize {
    let hash = (pointer as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut at = (hash ^ (hash >> 32)) as usize % slots.len();
    while slots[at] != 0 && key(slots[at] - 1) != pointer {
        at = (at + 1) % slots.len();
    }
    at
}

impl StatusPacket {
    pub fn new(units: usize) -> std::io::Result<Self> {
        let mut packet = Self::with_capacities(StatusCapacities::new(units))?;
        packet.enable_growth();
        Ok(packet)
    }

    /// Allocation-free placeholder while the complete packet is owned by a job.
    pub fn vacant() -> Self {
        let capacities = StatusCapacities {
            units: 0,
            supply_origins: 0,
            manpower: 0,
            acclimatization: 0,
            acclimatization_categories: 0,
            thresholds: 0,
            modifiers: crate::modifier_accumulator::SnapshotCapacity::default(),
            membership: MembershipCapacity::default(),
        };
        Self {
            growing: false,
            capacities,
            required: capacities,
            units: Vec::new(),
            supply_origins: Vec::new(),
            supply_origins_len: 0,
            border_index: Vec::new(),
            border_stats: Vec::new(),
            maxima: Vec::new(),
            maxima_index: Vec::new(),
            countries: Vec::new(),
            country_index: Vec::new(),
            groups: Vec::new(),
            group_index: Vec::new(),
            commander_groups: Vec::new(),
            commander_bans: Vec::new(),
            commander_index: Vec::new(),
            commander_memo: crate::modifier_definition::DefinitionMemo::default(),
            modifiers: crate::modifier_accumulator::SnapshotArena::vacant(),
            membership: MembershipPacket::vacant(),
            manpower: Vec::new(),
            acclimatization: Vec::new(),
            categories: crate::acclimatization_category::Categories::default(),
            categories_loaded: None,
            thresholds_header: None,
            thresholds: Vec::new(),
            attrition_metadata: None,
            attrition_define: None,
            maximum_level: None,
            minimum_strength: None,
            outcome: Outcome::Rows { len: 0 },
        }
    }

    pub fn with_capacities(capacities: StatusCapacities) -> std::io::Result<Self> {
        let index_capacity = capacities
            .units
            .checked_mul(2)
            .ok_or_else(crate::allocation::oom)?;
        Ok(Self {
            growing: false,
            capacities,
            required: capacities,
            units: crate::allocation::vec(capacities.units)?,
            supply_origins: crate::allocation::filled(
                capacities.supply_origins,
                crate::cunit_supply_inputs::Origin::default(),
            )?,
            supply_origins_len: 0,
            border_index: crate::allocation::vec(capacities.units)?,
            border_stats: crate::allocation::filled(capacities.units, None)?,
            maxima: crate::allocation::vec(capacities.units)?,
            maxima_index: crate::allocation::filled(index_capacity, 0)?,
            countries: crate::allocation::vec(capacities.units)?,
            country_index: crate::allocation::filled(index_capacity, 0)?,
            groups: crate::allocation::vec(capacities.units)?,
            group_index: crate::allocation::filled(index_capacity, 0)?,
            commander_groups: crate::allocation::vec(capacities.units)?,
            commander_bans: crate::allocation::vec(capacities.units)?,
            commander_index: crate::allocation::filled(index_capacity, 0)?,
            // The two retreat-ban keys; a vacant packet stays allocation-free.
            commander_memo: crate::modifier_definition::DefinitionMemo::with_capacity(
                if capacities.units == 0 { 0 } else { 2 },
            ),
            modifiers: crate::modifier_accumulator::SnapshotArena::try_new(capacities.modifiers)?,
            membership: MembershipPacket::try_with_capacity(
                capacities.membership.queries,
                capacities.membership.groups,
                capacities.membership.roots,
                capacities.membership.chunks,
            )?,
            manpower: crate::allocation::vec(capacities.manpower)?,
            acclimatization: crate::allocation::vec(capacities.acclimatization)?,
            categories: crate::acclimatization_category::Categories::try_with_capacity(
                capacities.acclimatization_categories,
            )?,
            categories_loaded: None,
            thresholds_header: None,
            thresholds: crate::allocation::vec(capacities.thresholds)?,
            attrition_metadata: None,
            attrition_define: None,
            maximum_level: None,
            minimum_strength: None,
            outcome: Outcome::Rows { len: 0 },
        })
    }

    /// Worker ownership allows original native controls to grow storage.
    pub fn enable_growth(&mut self) {
        self.growing = true;
        self.modifiers.enable_worker_growth();
    }

    pub fn clear(&mut self) {
        self.units.clear();
        self.supply_origins_len = 0;
        self.border_index.clear();
        self.border_stats.fill(None);
        self.maxima.clear();
        self.maxima_index.fill(0);
        self.countries.clear();
        self.country_index.fill(0);
        self.groups.clear();
        self.group_index.fill(0);
        self.commander_groups.clear();
        self.commander_bans.clear();
        self.commander_index.fill(0);
        // Keep the memo's storage: a fresh memo would allocate on every walk.
        self.commander_memo.clear();
        self.modifiers.clear();
        self.membership.clear();
        self.manpower.clear();
        self.acclimatization.clear();
        self.categories.clear();
        self.categories_loaded = None;
        self.thresholds_header = None;
        self.thresholds.clear();
        self.attrition_metadata = None;
        self.attrition_define = None;
        self.maximum_level = None;
        self.minimum_strength = None;
        self.required = self.capacities;
        self.outcome = Outcome::Rows { len: 0 };
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn project(&self, output: &mut [StatusRow]) -> Outcome {
        project_status(self, output)
    }

    /// The complete ordered origin vector; a missing node fragment cannot be
    /// mistaken for an empty vector or invalidate the row's scalar supply lines.
    pub fn supply_origins(&self, row: &StatusRow) -> Option<&[crate::cunit_supply_inputs::Origin]> {
        let len = row.supply?.origins_len.ok()?;
        let end = row.supply_origins_start.checked_add(len)?;
        self.supply_origins
            .get(..self.supply_origins_len)?
            .get(row.supply_origins_start..end)
    }

    /// Rebuild only during buffer recycling. Retain private arena demand and
    /// membership storage grown during acquisition even when a source fails
    /// before row projection. `membership` is the shared pooled storage floor.
    pub fn grow(&mut self, units: usize, membership: MembershipCapacity) -> std::io::Result<()> {
        self.capacities.membership = self.membership.capacity();
        let mut next = self.required_capacities();
        next.units = next.units.max(units).max(self.capacities.units);
        next.supply_origins = next.supply_origins.max(self.capacities.supply_origins);
        next.manpower = next.manpower.max(self.capacities.manpower);
        next.acclimatization = next.acclimatization.max(self.capacities.acclimatization);
        next.acclimatization_categories = next
            .acclimatization_categories
            .max(self.capacities.acclimatization_categories);
        next.thresholds = next.thresholds.max(self.capacities.thresholds);
        next.modifiers.nodes = next
            .modifiers
            .nodes
            .max(self.capacities.modifiers.nodes)
            .max(next.units);
        next.modifiers.entries = next
            .modifiers
            .entries
            .max(self.capacities.modifiers.entries);
        next.membership = self.capacities.membership.grown(next.membership);
        next.membership.queries = next
            .membership
            .queries
            .max(membership.queries)
            .max(next.units);
        next.membership.groups = next.membership.groups.max(membership.groups);
        next.membership.roots = next.membership.roots.max(membership.roots);
        next.membership.chunks = next.membership.chunks.max(membership.chunks);
        if next != self.capacities {
            let growing = self.growing;
            *self = Self::with_capacities(next)?;
            if growing {
                self.enable_growth();
            }
        }
        Ok(())
    }

    pub fn required_capacities(&self) -> StatusCapacities {
        let mut required = self.required;
        required.modifiers = self.modifiers.required_capacity();
        required.membership = self.membership.required_capacity();
        required
    }

    pub fn membership_capacity(&self) -> MembershipCapacity {
        self.membership.capacity()
    }
    /// Off-hour demand only; the existing grow/recycle path provisions it.
    pub fn raise_supply_origins(&mut self, required: usize) {
        self.required.supply_origins = self.required.supply_origins.max(required);
    }

    pub fn allocated_bytes(&self) -> usize {
        use std::mem::size_of;
        self.units.capacity() * size_of::<StatusUnitInputs>()
            + self.supply_origins.capacity() * size_of::<crate::cunit_supply_inputs::Origin>()
            + self.border_index.capacity() * size_of::<(u64, usize)>()
            + self.border_stats.capacity() * size_of::<Option<[i64; 3]>>()
            + self.maxima.capacity() * size_of::<crate::csub_unit_definition::DivisionMaxima>()
            + self.countries.capacity() * size_of::<StatusCountryInputs>()
            + self.groups.capacity() * size_of::<StatusGroupInputs>()
            + self.commander_groups.capacity() * size_of::<usize>()
            + self.commander_bans.capacity()
                * size_of::<Result<Option<crate::carmy_leader::RetreatBans>, &'static str>>()
            + self.commander_memo.allocated_bytes()
            + (self.maxima_index.capacity()
                + self.country_index.capacity()
                + self.group_index.capacity()
                + self.commander_index.capacity())
                * size_of::<usize>()
            + self.modifiers.allocated_bytes()
            + self.membership.allocated_bytes()
            + self.manpower.capacity() * size_of::<i32>()
            + self.acclimatization.capacity() * size_of::<crate::acclimatization_entry::Entry>()
            + self.categories.allocated_bytes()
            + self.thresholds.capacity() * size_of::<Result<i64, &'static str>>()
    }

    fn maxima(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
    ) -> Result<usize, &'static str> {
        let slot = status_pointer_slot(&self.maxima_index, pointer, |i| self.maxima[i].pointer);
        if self.maxima_index[slot] != 0 {
            return Ok(self.maxima_index[slot] - 1);
        }
        let value = crate::csub_unit_definition::capture_division_maxima(memory, base, pointer)?;
        let index = self.maxima.len();
        self.maxima.push(value);
        self.maxima_index[slot] = index + 1;
        Ok(index)
    }

    fn country_threshold(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
    ) -> Result<usize, &'static str> {
        let slot = status_pointer_slot(&self.country_index, pointer, |i| self.countries[i].pointer);
        if self.country_index[slot] != 0 {
            return Ok(self.country_index[slot] - 1);
        }
        let modifier =
            self.modifiers
                .capture(memory, pointer, keys::UNIT_UPKEEP_ATTRITION_FACTOR)?;
        if self.modifiers.value(modifier)?.present {
            if self.attrition_metadata.is_none() {
                self.attrition_metadata = Some((|| {
                    use crate::modifier_definition as metadata;
                    let table =
                        presentation_field::<u64>(memory, base, metadata::TABLE_RVA)? as usize;
                    let entry = table
                        .checked_add(keys::UNIT_UPKEEP_ATTRITION_FACTOR as usize * metadata::STRIDE)
                        .filter(|_| table != 0)
                        .ok_or("attrition metadata null or overflow")?;
                    let feature_selector =
                        presentation_field::<u32>(memory, entry, metadata::FEATURE_SELECTOR)?;
                    // Retain the successful selector even when it excludes the
                    // unproved callback branch. FLAGS is not then demanded.
                    let flags = if feature_selector == 0 {
                        Some(presentation_field::<u32>(memory, entry, metadata::FLAGS)?)
                    } else {
                        None
                    };
                    Ok(StatusAttritionMetadata {
                        feature_selector,
                        flags,
                    })
                })());
            }
            if self.attrition_metadata.unwrap()?.feature_selector != 0 {
                return Err("attrition modifier needs unproved feature callback");
            }
        }
        if self.attrition_define.is_none() {
            self.attrition_define = Some(presentation_field::<i64>(
                memory,
                base,
                COUNTER_ATTRITION_THRESHOLD_RVA,
            ));
        }
        self.attrition_define.unwrap()?;
        let index = self.countries.len();
        self.countries
            .push(StatusCountryInputs { pointer, modifier });
        self.country_index[slot] = index + 1;
        Ok(index)
    }

    fn training(&mut self, memory: &impl Memory, pointer: usize) -> Result<u8, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let slot = status_pointer_slot(&self.group_index, pointer, |i| self.groups[i].pointer);
        if self.group_index[slot] != 0 {
            return self.groups[self.group_index[slot] - 1].training;
        }
        let training = presentation_field(memory, pointer, corders_group::TRAINING);
        self.groups.push(StatusGroupInputs {
            pointer,
            training,
            stop_at_maximum: None,
        });
        self.group_index[slot] = self.groups.len();
        training
    }

    /// Retreat-ban bits for one division. Read both blocks once per group;
    /// select the unit's block only when they differ.
    fn commander(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
    ) -> Result<u8, &'static str> {
        let group = seed
            .orders_group
            .ok_or("division presentation field unreadable")?;
        if group == 0 {
            return Ok(0);
        }
        let slot = status_pointer_slot(&self.commander_index, group, |i| self.commander_groups[i]);
        let bans = if self.commander_index[slot] != 0 {
            self.commander_bans[self.commander_index[slot] - 1]?
        } else {
            if self.commander_groups.len() == self.commander_groups.capacity() {
                return Err("division status commander packet capacity exceeded");
            }
            let bans = crate::carmy_leader::retreat_bans(memory, base, group);
            self.commander_groups.push(group);
            self.commander_bans.push(bans);
            self.commander_index[slot] = self.commander_groups.len();
            bans?
        };
        let Some(bans) = bans else {
            return Ok(0);
        };
        if bans.ordinary == bans.alternate {
            return Ok(bans.ordinary);
        }
        let accumulator = crate::carmy_leader::movement_accumulator_seeded(memory, base, seed)?
            .ok_or("retreat ban leader vanished")?;
        bans.select(accumulator)
    }

    /// Reads the bans of every group of a wanted division at once
    /// (`retreat_bans_many`), keeping each group's own result for
    /// [`Self::commander`] to report at that division, in source order.
    fn prepare_commanders(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        wanted: &dyn Fn(&DivisionSeed) -> bool,
    ) {
        let first = self.commander_groups.len();
        for seed in seeds {
            let Some(group) = seed
                .orders_group
                .filter(|&group| group != 0 && wanted(seed))
            else {
                continue;
            };
            let slot =
                status_pointer_slot(&self.commander_index, group, |i| self.commander_groups[i]);
            if self.commander_index[slot] != 0 {
                continue;
            }
            if self.commander_groups.len() == self.commander_groups.capacity() {
                break;
            }
            self.commander_groups.push(group);
            self.commander_bans.push(Ok(None));
            self.commander_index[slot] = self.commander_groups.len();
        }
        crate::carmy_leader::retreat_bans_many(
            memory,
            base,
            &self.commander_groups[first..],
            &mut self.commander_bans[first..],
            &self.commander_memo,
        );
    }

    /// The retreat bans of every division whose raw roster owner `owner`
    /// maps to a read owner, one row per division with a ban, in source
    /// order. The daily read of `DivisionRetreatBans`: each wanted group's
    /// commander is read once, both blocks, and a division's block is
    /// selected only when they differ. A failed read fails the table.
    ///
    /// `owner` maps a raw roster owner to `Ok(Some(owner))` for a read owner
    /// and `Ok(None)` for one outside the read; `Err` is an owner that could
    /// not be resolved. A division whose owner is unreadable (its roster
    /// owner field or `owner` failed) might belong to any read owner, and a
    /// returned owner's rows replace all of its earlier ones, so the read
    /// returns no rows at all: it is truncated, with those divisions as its
    /// unread rows, and every earlier ban stands until a read resolves them.
    pub fn retreat_bans(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        source: Outcome,
        owner: &dyn Fn(u32) -> Result<Option<u32>, &'static str>,
        output: &mut [RetreatBanRow],
    ) -> Outcome {
        let result = (|| {
            if matches!(source, Outcome::Unavailable { .. }) {
                return Err("division retreat ban source unavailable");
            }
            let seeds = seeds
                .get(..source.len())
                .ok_or("division source count invalid")?;
            let owned = |seed: &DivisionSeed| {
                seed.roster_owner
                    .ok_or("division roster owner unreadable")
                    .and_then(owner)
            };
            let unresolved = seeds.iter().filter(|seed| owned(seed).is_err()).count();
            if unresolved != 0 {
                return Ok(Outcome::Truncated {
                    len: 0,
                    unread_rows: u32::try_from(unresolved).unwrap_or(u32::MAX),
                });
            }
            self.commander_groups.clear();
            self.commander_bans.clear();
            self.commander_index.fill(0);
            self.commander_memo.clear();
            self.prepare_commanders(memory, base, seeds, &|seed| {
                matches!(owned(seed), Ok(Some(_)))
            });
            let (mut len, mut unread) = (0, 0_u32);
            for seed in seeds {
                let Ok(Some(owner)) = owned(seed) else {
                    continue;
                };
                let bans = self.commander(memory, base, seed)?;
                if bans == 0 {
                    continue;
                }
                match output.get_mut(len) {
                    Some(row) => {
                        *row = RetreatBanRow {
                            division: seed.division,
                            owner,
                            bans,
                        };
                        len += 1;
                    }
                    None => unread += 1,
                }
            }
            Ok(if unread == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: unread,
                }
            })
        })();
        result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
    }

    fn stop_at_maximum(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
    ) -> Result<u8, &'static str> {
        let slot = status_pointer_slot(&self.group_index, pointer, |i| self.groups[i].pointer);
        let group = self
            .groups
            .get_mut(self.group_index[slot].wrapping_sub(1))
            .ok_or("division training group inputs missing")?;
        *group.stop_at_maximum.get_or_insert_with(|| {
            presentation_field(memory, pointer, corders_group::STOP_AT_MAXIMUM)
        })
    }

    fn capture_level(
        &mut self,
        memory: &impl Memory,
        base: usize,
        army: usize,
    ) -> Result<StatusLevelOperands, &'static str> {
        let address = army
            .checked_add(MANPOWER_DATA)
            .ok_or("division presentation array invalid")?;
        let header =
            read::<[u8; 16]>(memory, address).ok_or("division presentation array invalid")?;
        let values = Array::from_header(header, 8).ok_or("division presentation array invalid")?;
        let start = self.manpower.len();
        let required = start
            .checked_add(values.count as usize)
            .ok_or("exercise manpower count overflow")?;
        self.required.manpower = self.required.manpower.max(required);
        if self.growing {
            crate::allocation::reserve(&mut self.manpower, required - start)
                .map_err(|_| "status manpower allocation failed")?;
            self.capacities.manpower = self.capacities.manpower.max(required);
        }
        if required > self.capacities.manpower {
            return Err("division status manpower packet capacity exceeded");
        }
        const BATCH: usize = 64;
        const FIELDS: [crate::memory::RecordField; 1] = [crate::memory::RecordField {
            source_offset: 4,
            destination_offset: 0,
            len: 4,
        }];
        const LAYOUT: crate::memory::RecordLayout<'static> =
            crate::memory::RecordLayout::new(&FIELDS, 4).expect("valid status manpower layout");
        let mut bytes = [0; BATCH * 4];
        let mut manpower = 0_i32;
        for first in (0..values.count).step_by(BATCH) {
            let count = ((values.count - first) as usize).min(BATCH);
            let entry = values
                .element(first, 8)
                .ok_or("exercise manpower entry overflow")?;
            let copied = memory.copy_records(
                crate::memory::RecordSources::Strided {
                    base: entry,
                    stride: 8,
                    count,
                },
                &LAYOUT,
                &mut bytes[..count * 4],
            );
            for word in bytes[..copied * 4].chunks_exact(4) {
                let value = i32::from_le_bytes(word.try_into().unwrap());
                // The admitted array header checked the entire stride-8 range.
                // Keep only complete offset-4 words, including on a later fault.
                self.manpower.push(value);
                manpower = manpower.wrapping_add(value);
            }
            if copied != count {
                return Err("division presentation field unreadable");
            }
        }
        let experience = if manpower < 1 {
            None
        } else {
            Some(presentation_field::<i64>(memory, army, EXPERIENCE)?)
        };
        let fraction = status_experience_fraction(manpower, experience.unwrap_or(0));
        if self.thresholds_header.is_none() {
            self.thresholds_header = Some(
                base.checked_add(EXPERIENCE_THRESHOLDS_RVA)
                    .and_then(|p| read::<[u8; 16]>(memory, p))
                    .ok_or("exercise thresholds invalid"),
            );
        }
        let thresholds = Array::from_header(self.thresholds_header.unwrap()?, 8)
            .ok_or("exercise thresholds invalid")?;
        if self.growing {
            let additional = (thresholds.count as usize).saturating_sub(self.thresholds.len());
            crate::allocation::reserve(&mut self.thresholds, additional)
                .map_err(|_| "status threshold allocation failed")?;
            self.capacities.thresholds = self.capacities.thresholds.max(thresholds.count as usize);
        }
        if thresholds.count == 0 {
            return Err("exercise thresholds empty");
        }
        let mut thresholds_len = 0;
        for index in 0..thresholds.count as usize {
            if index == self.thresholds.len() {
                self.required.thresholds = self.required.thresholds.max(index + 1);
                if index == self.capacities.thresholds {
                    return Err("division status threshold packet capacity exceeded");
                }
                let value = thresholds
                    .element(index as u32, 8)
                    .and_then(|p| read::<i64>(memory, p))
                    .ok_or("exercise threshold unreadable");
                self.thresholds.push(value);
            }
            let value = self.thresholds[index]?;
            thresholds_len = index + 1;
            if value > fraction {
                break;
            }
        }
        Ok(StatusLevelOperands {
            manpower_start: start,
            manpower_len: values.count as usize,
            experience,
            thresholds_len,
        })
    }

    fn capture_exercise(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seed: &DivisionSeed,
        unit: usize,
    ) -> Result<StatusExerciseOperands, &'static str> {
        let mut inputs = StatusExerciseOperands::default();

        let owner = seed
            .roster_owner
            .ok_or("division presentation field unreadable")?;
        inputs.maximum_level = (*self
            .maximum_level
            .get_or_insert_with(|| presentation_field(memory, base, EXERCISE_MAXIMUM_LEVEL_RVA)))?;
        inputs.autonomous = cgame_state::autonomous_country(memory, base, state, owner)?;
        if inputs.autonomous {
            let level = self.capture_level(memory, base, seed.pointer)?;
            inputs.level = Some(level);
            if self.level(level)? >= inputs.maximum_level {
                return Ok(inputs);
            }
        }
        let deployment = presentation_field::<u64>(memory, seed.pointer, DEPLOYMENT_TYPE)?;
        inputs.deployment_present =
            Some(crate::creference_registry::resolve(memory, base, deployment)?.is_some());
        if inputs.deployment_present == Some(true) {
            return Ok(inputs);
        }

        let strength = seed
            .strength
            .ok_or("division presentation field unreadable")?;
        inputs.strength = Some(strength);
        let maximum_hp = self.maxima[self.units[unit].maxima].effective().0;
        let fraction = status_strength_fraction(strength, maximum_hp);
        let minimum = (*self.minimum_strength.get_or_insert_with(|| {
            presentation_field(memory, base, EXERCISE_MINIMUM_STRENGTH_RVA)
        }))?;
        inputs.minimum_strength = Some(minimum);
        if fraction <= minimum {
            return Ok(inputs);
        }
        let stop = self.stop_at_maximum(memory, self.units[unit].group)?;
        inputs.stop_at_maximum = Some(stop);
        if stop != 0 && inputs.level.is_none() {
            inputs.level = Some(self.capture_level(memory, base, seed.pointer)?);
        }
        Ok(inputs)
    }

    fn level(&self, inputs: StatusLevelOperands) -> Result<i32, &'static str> {
        let words = self
            .manpower
            .get(inputs.manpower_start..inputs.manpower_start + inputs.manpower_len)
            .ok_or("division exercise manpower inputs invalid")?;
        let manpower = words
            .iter()
            .fold(0_i32, |sum, word| sum.wrapping_add(*word));
        let fraction = status_experience_fraction(manpower, inputs.experience.unwrap_or(0));
        let thresholds = self
            .thresholds
            .get(..inputs.thresholds_len)
            .ok_or("division exercise threshold inputs invalid")?;
        let mut level = 0;
        for &threshold in thresholds {
            if threshold? > fraction {
                break;
            }
            level += 1;
        }
        Ok(level)
    }

    fn exercise(
        &self,
        inputs: StatusExerciseOperands,
        maximum_hp: i64,
    ) -> Result<bool, &'static str> {
        if inputs.autonomous
            && self.level(inputs.level.ok_or("division exercise level missing")?)?
                >= inputs.maximum_level
        {
            return Ok(false);
        }
        if inputs
            .deployment_present
            .ok_or("division exercise deployment input missing")?
        {
            return Ok(false);
        }
        let fraction = status_strength_fraction(
            inputs
                .strength
                .ok_or("division exercise strength missing")?,
            maximum_hp,
        );
        if fraction
            <= inputs
                .minimum_strength
                .ok_or("division exercise strength define missing")?
        {
            return Ok(false);
        }
        Ok(inputs
            .stop_at_maximum
            .ok_or("division exercise stop flag missing")?
            == 0
            || self.level(inputs.level.ok_or("division exercise level missing")?)?
                < inputs.maximum_level)
    }

    /// The same fresh exercise predicate projected into DivisionStatus, over
    /// already acquired operands. Attrition's on-demand getter consumes this
    /// predicate without revisiting game memory or using the training byte.
    pub fn exercising_at(&self, index: usize) -> Result<u8, &'static str> {
        let unit = self
            .units
            .get(index)
            .ok_or("division exercise index invalid")?;
        let Some(inputs) = unit.exercise else {
            return Ok(0);
        };
        let maximum_hp = self
            .maxima
            .get(unit.maxima)
            .ok_or("division exercise maxima missing")?
            .effective()
            .0;
        self.exercise(inputs, maximum_hp).map(u8::from)
    }

    /// The existing authenticated full-handle index, without another census.
    pub fn status_index(&self, division: u64) -> Option<usize> {
        let position = self
            .border_index
            .binary_search_by_key(&division, |entry| entry.0)
            .ok()?;
        Some(self.border_index[position].1)
    }

    /// A division's (category token, weight) acclimatization entries, in
    /// native order with repeats, as projected into its row.
    pub fn acclimatization(
        &self,
        row: &StatusRow,
    ) -> Option<&[crate::acclimatization_entry::Entry]> {
        self.acclimatization.get(
            row.acclimatization_start
                ..row
                    .acclimatization_start
                    .checked_add(row.acclimatization_len)?,
        )
    }

    /// Embedded acclimatization data: its maximum and every entry's
    /// category token and weight. The winning category is the viewer's to pick.
    /// A failure, including a full packet, leaves only this division's field
    /// `None`; the packet still asks for the room it lacked.
    fn capture_acclimatization(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        unit: usize,
    ) {
        let start = self.acclimatization.len();
        let maximum = self.read_acclimatization(memory, base, seed);
        let inputs = &mut self.units[unit];
        inputs.acclimatization_maximum = maximum.ok();
        inputs.acclimatization_start = start;
        inputs.acclimatization_len = match maximum {
            Ok(_) => self.acclimatization.len() - start,
            Err(_) => {
                self.acclimatization.truncate(start);
                0
            }
        };
    }

    /// Appends the entries, one span read per division, and returns the maximum.
    fn read_acclimatization(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
    ) -> Result<i64, &'static str> {
        use crate::{acclimatization_data as data, acclimatization_entry as entry};
        let object = seed
            .acclimatization
            .ok_or("division presentation field unreadable")?;
        if Some(u64::from_le_bytes(object[..8].try_into().unwrap()))
            != base.checked_add(data::VTABLE_RVA).map(|p| p as u64)
        {
            return Err("division acclimatization identity mismatch");
        }
        let entries = Array::from_header(
            object[data::DATA..data::DATA + 16].try_into().unwrap(),
            entry::STRIDE,
        )
        .ok_or("division acclimatization entries invalid")?;
        let count = entries.count as usize;
        let required = self
            .acclimatization
            .len()
            .checked_add(count)
            .ok_or("division acclimatization count overflow")?;
        self.required.acclimatization = self.required.acclimatization.max(required);
        if self.growing {
            crate::allocation::reserve(&mut self.acclimatization, count)
                .map_err(|_| "status acclimatization allocation failed")?;
            self.capacities.acclimatization = self.capacities.acclimatization.max(required);
        }
        if required > self.capacities.acclimatization {
            return Err("division status acclimatization packet capacity exceeded");
        }
        if count != 0 {
            let categories = *self.categories_loaded.get_or_insert_with(|| {
                if self.growing {
                    self.categories.load_growing(
                        memory,
                        base,
                        &mut self.required.acclimatization_categories,
                    )
                } else {
                    self.categories.load(
                        memory,
                        base,
                        &mut self.required.acclimatization_categories,
                    )
                }
            });
            categories?;
        }
        const SPAN: usize = 32;
        let mut bytes = [0_u8; SPAN * entry::STRIDE];
        for first in (0..count).step_by(SPAN) {
            let len = (count - first).min(SPAN);
            let at = entries
                .element(first as u32, entry::STRIDE)
                .ok_or("division acclimatization entries invalid")?;
            if !crate::read_slice(memory, at, &mut bytes[..len * entry::STRIDE]) {
                return Err("division acclimatization entries unreadable");
            }
            for raw in bytes[..len * entry::STRIDE].chunks_exact(entry::STRIDE) {
                let category = usize::try_from(u64::from_le_bytes(
                    raw[entry::CATEGORY..entry::CATEGORY + 8]
                        .try_into()
                        .unwrap(),
                ))
                .ok()
                .and_then(|pointer| self.categories.token(pointer))
                .ok_or("division acclimatization category is not registered")?;
                self.acclimatization.push(entry::Entry {
                    category,
                    weight: i64::from_le_bytes(
                        raw[entry::WEIGHT..entry::WEIGHT + 8].try_into().unwrap(),
                    ),
                });
            }
        }
        Ok(i64::from_le_bytes(
            object[data::MAXIMUM..data::MAXIMUM + 8].try_into().unwrap(),
        ))
    }

    /// Acquisition truncates at this table's own source prefix. Movement's route
    /// arena admission never changes which Status units are demanded.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        row_capacity: usize,
    ) -> Outcome {
        self.clear();
        let prepared = (|| -> Result<usize, &'static str> {
            if let Outcome::Unavailable { reason } = source_outcome {
                return Err(reason);
            }
            if source_outcome.len() != seeds.len() {
                return Err("division status source length invalid");
            }
            if self.growing {
                // Serialization retains every division; child-header growth follows the
                // same acquisition without retrying the walk.
                self.grow(seeds.len(), self.membership.capacity())
                    .map_err(|_| "status seed allocation failed")?;
            }
            let countries = ccountry::Reader::new(memory, base, state)?;
            self.required.units = self.required.units.max(seeds.len());
            if seeds.len() > self.capacities.units {
                return Err("division status seed packet capacity exceeded");
            }
            let count = seeds.len().min(if self.growing {
                seeds.len()
            } else {
                row_capacity
            });
            for seed in &seeds[..count] {
                let unit = self.units.len();
                self.units.push(StatusUnitInputs {
                    division: seed.division,
                    ..StatusUnitInputs::default()
                });

                let result = (|| -> Result<(), &'static str> {
                    if seed.primary_vtable != base.checked_add(VTABLE_RVA).map(|p| p as u64)
                        || seed
                            .handle
                            .ok_or("division presentation field unreadable")?
                            != seed.division
                        || seed.division == 0
                    {
                        return Err("division status source identity changed");
                    }

                    let stats = seed
                        .stat_block
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].maxima = self.maxima(memory, base, stats)?;

                    let expeditionary = seed
                        .expeditionary_owner
                        .ok_or("division presentation field unreadable")?
                        as i32;
                    let raw_country = if expeditionary > 0 {
                        expeditionary as u32
                    } else {
                        seed.roster_owner
                            .ok_or("division presentation field unreadable")?
                    };
                    let country = countries.resolve(raw_country)?;
                    self.units[unit].threshold =
                        self.country_threshold(memory, base, country.pointer)?;

                    let group = seed
                        .orders_group
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].group = group;
                    let member = seed
                        .pointer
                        .checked_add(cunit::SCHEDULED_MEMBER)
                        .ok_or("division membership address overflow")?;
                    self.units[unit].membership = self.membership.push_query(MembershipQuery {
                        member,
                        own_group: group,
                    })?;
                    Ok(())
                })();
                if let Err(reason) = result {
                    self.units[unit].failure = Some(reason);
                    break;
                }
            }
            Ok(count)
        })();
        // Query outcomes are independent and retain their first failing root.
        // Finishing in source order preserves a prior unit's scalar/exercise
        // error ahead of a later unit's membership or preparation failure.
        self.membership.acquire(memory, base);
        let result = prepared.and_then(|count| {
            for (unit, seed) in seeds.iter().take(self.units.len()).enumerate() {
                if let Some(reason) = self.units[unit].failure {
                    return Err(reason);
                }
                self.membership.outcome(self.units[unit].membership)?;

                self.units[unit].attrition_value = seed
                    .counter_attrition
                    .ok_or("division presentation field unreadable")?;

                self.units[unit].entrenchment = seed
                    .entrenchment
                    .ok_or("division presentation field unreadable")?;

                self.units[unit].entrenchment_cap = seed
                    .entrenchment_cap
                    .ok_or("division presentation field unreadable")?;

                self.units[unit].supply_ratio = seed
                    .counter_supply_status
                    .ok_or("division presentation field unreadable")?;
                self.units[unit].supply_grace_maximum = seed
                    .supply_grace_maximum
                    .ok_or("division maximum supply grace unreadable")?;
                self.units[unit].supply_grace_remaining = seed
                    .supply_grace_remaining
                    .ok_or("division remaining supply grace unreadable")?;
                self.units[unit].invalid_orders = seed
                    .invalid_orders
                    .ok_or("division presentation field unreadable")?;
                self.units[unit].strength_factor = seed
                    .strength_factor
                    .ok_or("division presentation field unreadable")?;
                self.units[unit].exiled = seed
                    .exiled
                    .ok_or("division presentation field unreadable")?;
                self.units[unit].model_number = seed
                    .model_number
                    .ok_or("division presentation field unreadable")?;
                self.capture_acclimatization(memory, base, seed, unit);
                self.units[unit].training = self.training(memory, self.units[unit].group)?;
                if self.units[unit].training != 0 {
                    self.units[unit].exercise =
                        Some(self.capture_exercise(memory, base, state, seed, unit)?);
                }
            }
            let source_missing = match source_outcome {
                Outcome::Truncated { unread_rows, .. } => unread_rows,
                _ => 0,
            };
            let omitted =
                u32::try_from(seeds.len() - count).map_err(|_| "division status count overflow")?;
            let missing = source_missing
                .checked_add(omitted)
                .ok_or("division status count overflow")?;
            Ok(if missing == 0 {
                Outcome::Rows { len: count }
            } else {
                Outcome::Truncated {
                    len: count,
                    unread_rows: missing,
                }
            })
        });
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        if self.units.len() > self.border_index.capacity() {
            if matches!(self.outcome, Outcome::Rows { .. }) {
                self.outcome = Outcome::Unavailable {
                    reason: "border division index packet capacity exceeded",
                };
            }
            return self.outcome;
        }
        self.border_index.extend(
            self.units
                .iter()
                .enumerate()
                .map(|(index, unit)| (unit.division, index)),
        );
        self.border_index.sort_unstable_by_key(|entry| entry.0);
        // Complete the established scalar/exercise prefix before this optional
        // tooltip operand. A failed read must neither withhold that prefix nor
        // prevent the collector's later naming and border-stat acquisition.
        let mut origins_required = 0_usize;
        for (unit, seed) in self.units.iter_mut().zip(seeds).take(self.outcome.len()) {
            if unit.invalid_orders != 0 {
                unit.invalid_orders_reason = seed
                    .pointer
                    .checked_add(cunit::INVALID_ORDERS_REASON)
                    .and_then(|address| read(memory, address));
            }
            unit.supply_origins_start = self.supply_origins_len;
            let origins = if self.growing {
                crate::walk::Buffer::GrowingFrom(&mut self.supply_origins, self.supply_origins_len)
            } else {
                crate::walk::Buffer::Fixed(&mut self.supply_origins[self.supply_origins_len..])
            };
            match crate::cunit_supply_inputs::capture_buffer(memory, seed.pointer, origins) {
                Ok(inputs) => {
                    let demand = match inputs.origins_len {
                        Ok(len) => {
                            self.supply_origins_len += len;
                            len
                        }
                        Err(crate::cunit_supply_inputs::Failure::OriginsCapacity { required }) => {
                            required
                        }
                        Err(_) => 0,
                    };
                    origins_required = origins_required.saturating_add(demand);
                    unit.supply_failure = inputs.origins_len.err();
                    unit.supply = Some(inputs);
                }
                Err(reason) => unit.supply_failure = Some(reason),
            }
        }
        self.required.supply_origins = self.required.supply_origins.max(origins_required);
        self.outcome
    }

    /// Augment the same census lineage after state and province membership
    /// joins. Storage is provisioned with the packet, so this allocates nothing.
    /// A selected failure withholds Status; None is exclusively outside demand.
    pub fn capture_border_stats(
        &mut self,
        memory: &impl Memory,
        base: usize,
        demanded: impl IntoIterator<Item = u64>,
    ) -> Outcome {
        let result = (|| {
            if !matches!(self.outcome, Outcome::Rows { .. }) {
                return Ok(self.outcome);
            }
            if self
                .border_index
                .windows(2)
                .any(|entries| entries[0].0 == entries[1].0)
            {
                return Err("border division status census duplicates a handle");
            }
            for handle in demanded {
                let position = self
                    .border_index
                    .binary_search_by_key(&handle, |entry| entry.0)
                    .map_err(|_| "demanded border army absent from division census")?;
                let unit = self.border_index[position].1;
                let cache = self.units[unit].maxima;
                let stats = self
                    .border_stats
                    .get_mut(cache)
                    .ok_or("border combat stat memo outside packet")?;
                let value = match *stats {
                    Some(value) => value,
                    None => {
                        let pointer = self
                            .maxima
                            .get(cache)
                            .ok_or("border combat stat source outside packet")?
                            .pointer;
                        let value = crate::csub_unit_definition::capture_division_combat_stats(
                            memory, base, pointer,
                        )?;
                        *stats = Some(value);
                        value
                    }
                };
                self.units[unit].border_combat_stats = Some(value);
            }
            Ok(self.outcome)
        })();
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        self.outcome
    }

    pub fn fail_border_stats(&mut self, reason: &'static str) {
        if matches!(self.outcome, Outcome::Rows { .. }) {
            self.outcome = Outcome::Unavailable { reason };
        }
    }
}

fn status_experience_fraction(manpower: i32, experience: i64) -> i64 {
    if manpower < 1 {
        0
    } else if (experience as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        0xffff_ffff
    } else {
        experience.wrapping_mul(100_000) / (i64::from(manpower) * 100_000)
    }
}

fn status_strength_fraction(strength: i64, maximum_hp: i64) -> i64 {
    if (strength as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        100_000
    } else {
        (strength.wrapping_mul(100_000) / maximum_hp).clamp(0, 100_000)
    }
}

/// Pure row projection over directly indexed owned operands. It cannot read
/// live game memory or resolve a native object.
pub fn project_status(packet: &StatusPacket, output: &mut [StatusRow]) -> Outcome {
    let result = (|| -> Result<Outcome, &'static str> {
        if let Outcome::Unavailable { reason } = packet.outcome {
            return Err(reason);
        }
        if packet.outcome.len() > packet.units.len() || packet.outcome.len() > output.len() {
            return Err("division status projection capacity invalid");
        }
        for (unit, row) in packet
            .units
            .iter()
            .zip(output.iter_mut())
            .take(packet.outcome.len())
        {
            let maxima = packet
                .maxima
                .get(unit.maxima)
                .ok_or("division status maxima index invalid")?;
            let (maximum_hit_points, maximum_organisation) = maxima.effective();
            let move_flags = (u8::from(unit.exiled != 0) * StatusRow::MOVE_EXILED)
                | (u8::from(maxima.can_exfiltrate_from_coast != 0)
                    * StatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST);
            let country = packet
                .countries
                .get(unit.threshold)
                .ok_or("division status country index invalid")?;
            let modifier = packet.modifiers.value(country.modifier)?;
            let mut value = modifier.raw;
            if modifier.present {
                let metadata = packet
                    .attrition_metadata
                    .ok_or("division status metadata missing")??;
                if metadata
                    .flags
                    .ok_or("division status metadata flags missing")?
                    & crate::modifier_definition::CLAMP_MAXIMUM
                    != 0
                {
                    value = value.clamp(0, 100_000);
                }
            }
            let attrition_threshold = value.wrapping_add(100_000).max(0).wrapping_mul(
                packet
                    .attrition_define
                    .ok_or("division status attrition define missing")??,
            ) / 100_000;
            let has_order = packet.membership.outcome(unit.membership)?;
            *row = StatusRow {
                division: unit.division,
                maximum_hit_points,
                maximum_organisation,
                entrenchment: unit.entrenchment,
                entrenchment_cap: unit.entrenchment_cap,
                supply_ratio: unit.supply_ratio,
                supply_grace_maximum: unit.supply_grace_maximum,
                supply_grace_remaining: unit.supply_grace_remaining,
                attrition_value: unit.attrition_value,
                attrition_threshold,
                attrition: u8::from(attrition_threshold < unit.attrition_value),
                exercising: u8::from(match unit.exercise {
                    Some(inputs) => packet.exercise(inputs, maximum_hit_points)?,
                    None => false,
                }),
                has_order: u8::from(has_order),
                invalid_orders: unit.invalid_orders,
                invalid_orders_reason: unit.invalid_orders_reason,
                strength_factor: unit.strength_factor,
                move_flags,
                model_number: unit.model_number,
                acclimatization_maximum: unit.acclimatization_maximum,
                acclimatization_start: unit.acclimatization_start,
                acclimatization_len: unit.acclimatization_len,
                border_combat_stats: unit.border_combat_stats,
                naming: None,
                country_ordinal: None,
                supply: unit.supply,
                supply_origins_start: unit.supply_origins_start,
                supply_failure: unit.supply_failure,
            };
        }
        Ok(packet.outcome)
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Counter dig-in duration selects the commander's unit-modifier block,
/// independently of its weather block.
pub fn counter_dig_in_factor(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
) -> Result<i64, &'static str> {
    let Some(root) = crate::carmy_leader::movement_accumulator_seeded(memory, base, seed)? else {
        return Ok(100_000);
    };
    crate::modifier_accumulator::requested_effective_at(
        memory,
        base,
        root,
        crate::modifier_keys::COMBAT_ENTRENCHMENT,
    )
    .map(|value| 100_000_i64.wrapping_add(value))
}

#[cfg(test)]
fn status_counter_scalars(
    memory: &impl Memory,
    army: usize,
) -> Result<(i64, i64, i64, i64), &'static str> {
    use crate::memory::RecordField;
    // This batch stays after has_order and before any training/exercise reads.
    // The discontiguous source gaps are deliberately excluded.
    const FIELDS: [RecordField; 4] = [
        RecordField {
            source_offset: COUNTER_ATTRITION,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: ENTRENCHMENT,
            destination_offset: 8,
            len: 8,
        },
        RecordField {
            source_offset: ENTRENCHMENT_CAP,
            destination_offset: 16,
            len: 8,
        },
        RecordField {
            source_offset: COUNTER_SUPPLY_STATUS,
            destination_offset: 24,
            len: 8,
        },
    ];
    let mut bytes = [0; 32];
    if memory.copy_records(
        crate::memory::RecordSources::Pointers(&[army]),
        &crate::memory::RecordLayout::new(&FIELDS, bytes.len()).unwrap(),
        &mut bytes,
    ) != FIELDS.len()
    {
        return Err("division presentation field unreadable");
    }
    Ok((
        i64::from_le_bytes(bytes[..8].try_into().unwrap()),
        i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        i64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        i64::from_le_bytes(bytes[24..].try_into().unwrap()),
    ))
}

#[cfg(test)]
fn exercise_level(memory: &impl Memory, base: usize, army: usize) -> Result<i32, &'static str> {
    let mut packet = StatusPacket::new(1).unwrap();
    let inputs = packet.capture_level(memory, base, army)?;
    packet.level(inputs)
}

#[cfg(test)]
fn exercising(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    group: usize,
    maximum_hp: i64,
) -> Result<bool, &'static str> {
    let mut packet = StatusPacket::new(1).unwrap();
    if packet.training(memory, group)? == 0 {
        return Ok(false);
    }
    let mut seeds = [DivisionSeed {
        pointer: army,
        ..DivisionSeed::default()
    }];
    capture_division_seeds(memory, &mut seeds);
    packet.units.push(StatusUnitInputs {
        group,
        ..StatusUnitInputs::default()
    });
    packet
        .maxima
        .push(crate::csub_unit_definition::DivisionMaxima {
            strength: maximum_hp,
            ..crate::csub_unit_definition::DivisionMaxima::default()
        });
    let inputs = packet.capture_exercise(memory, base, state, &seeds[0], 0)?;
    packet.exercise(inputs, maximum_hp)
}

#[cfg(test)]
pub fn walk_status(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[crate::cequipment_variant_pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [StatusRow],
) -> Outcome {
    let mut packet = StatusPacket::new(sources.len()).unwrap();
    status_test_capture(
        memory,
        base,
        state,
        sources,
        source_outcome,
        output,
        &mut packet,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn status_test_capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[crate::cequipment_variant_pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [StatusRow],
    packet: &mut StatusPacket,
) -> Outcome {
    let mut seeds: Vec<_> = sources
        .iter()
        .map(|source| DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            ..DivisionSeed::default()
        })
        .collect();
    // The collector's shared census performs this once for all consumers.
    // This test convenience captures only the independently demanded prefix.
    let count = seeds.len().min(output.len());
    if !matches!(source_outcome, Outcome::Unavailable { .. }) {
        capture_division_seeds(memory, &mut seeds[..count]);
    }
    let source_outcome = match source_outcome {
        Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
        Outcome::Rows { len } if len == count => Outcome::Rows { len },
        Outcome::Rows { len } => Outcome::Truncated {
            len: count,
            unread_rows: (len - count) as u32,
        },
        Outcome::Truncated { len, unread_rows } => Outcome::Truncated {
            len: count,
            unread_rows: unread_rows + (len - count) as u32,
        },
    };
    packet.capture(
        memory,
        base,
        state,
        &seeds[..count],
        source_outcome,
        output.len(),
    );
    packet.project(output)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn walk_status_with_membership(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[crate::cequipment_variant_pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [StatusRow],
    membership: &mut MembershipPacket,
) -> Outcome {
    let mut packet = StatusPacket::new(sources.len()).unwrap();
    std::mem::swap(&mut packet.membership, membership);
    let outcome = status_test_capture(
        memory,
        base,
        state,
        sources,
        source_outcome,
        output,
        &mut packet,
    );
    std::mem::swap(&mut packet.membership, membership);
    outcome
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    countries: Array,
    links: Array,
    provinces: Array,
}

impl<M: Memory> Reader<'_, M> {
    fn country_pointer(&self, slot: u32) -> Result<usize, &'static str> {
        self.countries
            .element(slot, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("country pointer unreadable")
    }

    fn canonical(&self, id: u32) -> Result<u32, &'static str> {
        if id == 0 {
            return Err("country id is zero");
        }
        self.links
            .element(id, 4)
            .and_then(|p| read::<u32>(self.memory, p))
            .filter(|&slot| slot > 0 && slot < self.countries.count)
            .ok_or("country link unreadable or outside country table")
    }

    fn vtable(&self, pointer: usize, rva: usize, reason: &'static str) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(rva)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }

    fn row(&self, pointer: usize, canonical: u32) -> Result<Row, &'static str> {
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("division address overflow")
        };
        self.vtable(pointer, VTABLE_RVA, "division primary vtable mismatch")?;
        self.vtable(
            at(SECONDARY_BASE)?,
            SECONDARY_VTABLE_RVA,
            "division secondary vtable mismatch",
        )?;
        let id =
            read::<u64>(self.memory, at(cunit::HANDLE)?).ok_or("division handle unreadable")?;
        if id == 0 {
            return Err("non-null division has null handle");
        }
        let roster_owner = read::<u32>(self.memory, at(cunit::ROSTER_OWNER)?)
            .ok_or("division roster owner unreadable")?;
        if self.canonical(roster_owner)? != canonical {
            return Err("division owner differs from roster country");
        }
        let logical_owner = read::<u32>(self.memory, at(cunit::LOGICAL_OWNER)?)
            .ok_or("division logical owner unreadable")?;
        let expeditionary_owner = read::<u32>(self.memory, at(cunit::EXPEDITIONARY_OWNER)?)
            .ok_or("division expeditionary owner unreadable")?;
        let location = read::<u64>(self.memory, at(cunit::LOCATION)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("division location pointer unreadable")?;
        let province = self.province(location)?;
        let template_pointer = read::<u64>(self.memory, at(STANDARD_TEMPLATE)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("division template pointer unreadable")?;
        let template = self.template(template_pointer)?;
        let reveal_skip = if template_pointer == 0 {
            0
        } else {
            template_pointer
                .checked_add(creferenced_division_template::REVEAL_SKIP)
                .and_then(|address| read::<u8>(self.memory, address))
                .ok_or("division template reveal skip unreadable")?
        };
        let hit_points =
            read::<i64>(self.memory, at(STRENGTH)?).ok_or("division hit points unreadable")?;
        let organisation = read::<i64>(self.memory, at(ORGANISATION)?)
            .ok_or("division organisation unreadable")?;
        let group = read::<u64>(self.memory, at(ORDERS_GROUP)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("division orders group pointer unreadable")?;
        let orders_group = self.orders_group(group)?;
        Ok(Row {
            id,
            roster_owner,
            logical_owner,
            expeditionary_owner,
            province,
            template,
            hit_points,
            organisation,
            orders_group,
            reveal_skip,
        })
    }

    /// The back-pointer is authenticated as either a plain COrdersGroup or a
    /// CArmyGroup before its native reference replaces it. A null pointer is
    /// the engine's own "no army", not a failed read.
    fn orders_group(&self, pointer: usize) -> Result<u64, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let identity = read::<[u8; 16]>(self.memory, pointer)
            .ok_or("division orders group identity unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let normal = self
            .base
            .checked_add(corders_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let group = self
            .base
            .checked_add(carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if vtable != normal && vtable != group {
            return Err("division orders group vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != corders_group::REFERENCE_TYPE {
            return Err("division orders group reference type mismatch");
        }
        Ok(handle)
    }

    fn province(&self, pointer: usize) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let expected = self
            .base
            .checked_add(cprovince::VTABLE_RVA)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) != Some(expected as u64) {
            return Err("division province vtable mismatch");
        }
        let id = pointer
            .checked_add(cprovince::ID)
            .and_then(|p| read::<u32>(self.memory, p))
            .ok_or("division province id unreadable")?;
        if id == 0 {
            return Err("non-null division province has zero id");
        }
        let registered = self
            .provinces
            .element(id, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .ok_or("division province slot unreadable or out of range")?;
        if registered != pointer as u64 {
            return Err("division province differs from registered slot");
        }
        Ok(id)
    }

    #[cfg(test)]
    fn province_cached(
        &self,
        pointer: usize,
        inputs: Option<&MovementInputs>,
    ) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let expected = self
            .base
            .checked_add(cprovince::VTABLE_RVA)
            .ok_or("module address overflow")?;
        if shared_presentation_read::<u64>(self.memory, inputs, pointer) != Some(expected as u64) {
            return Err("division province vtable mismatch");
        }
        let id = pointer
            .checked_add(cprovince::ID)
            .and_then(|p| shared_presentation_read::<u32>(self.memory, inputs, p))
            .ok_or("division province id unreadable")?;
        if id == 0 {
            return Err("non-null division province has zero id");
        }
        let registered = self
            .provinces
            .element(id, 8)
            .and_then(|p| shared_presentation_read::<u64>(self.memory, inputs, p))
            .ok_or("division province slot unreadable or out of range")?;
        if registered != pointer as u64 {
            return Err("division province differs from registered slot");
        }
        Ok(id)
    }

    fn template(&self, pointer: usize) -> Result<u64, &'static str> {
        template_handle(self.memory, self.base, pointer)
    }
}

mod fresh_counter_eta;
pub use fresh_counter_eta::{
    FreshCounterEtaPacket, FreshCounterEtaRow, FreshCounterEtaWalk, FreshNormalEtaRow,
    capture_counter_land_terrains, capture_fresh_counter_eta, counter_land_terrain_capacity,
    fresh_counter_eta_map_demand, fresh_counter_eta_map_demand_keyed,
};

#[cfg(test)]
mod eta_tests;

#[cfg(test)]
#[path = "carmy/border/tests.rs"]
mod border_tests;
#[cfg(test)]
mod tests;

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod bench;
