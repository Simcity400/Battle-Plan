//! Division object; all fields use its primary pointer. The hourly walker
//! retains full handles and i64 hit points and organisation. Candidate fields
//! with unknown widths remain identified below. Shared stat-block, ownership,
//! location, map-entity and modifier fields are in cunit.

use crate::modifier_keys as keys;
use crate::{
    Memory, carmy_group, ccountry, cgame_state, corders_group, cpdx_array::Array, cprovince,
    creferenced_division_template, cunit, read, walk::Outcome,
};

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
/// Byte that, nonzero, spares the movement speed's out-of-supply step its
/// noncore-supply term.
pub const NONCORE_SUPPLY_BYPASS: usize = 0x2e;
/// Signed air-supply word, i64: positive, the movement speed's noncore-supply
/// term takes the air noncore-supply define instead of the ordinary one.
pub const AIR_SUPPLY: usize = 0x58;
/// Signed days out of supply, i32: the movement speed's and attrition's
/// out-of-supply penalty scale it by the supply-days define.
pub const OUT_OF_SUPPLY_DAYS: usize = 0x498;
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
/// Previous template retained while a deployed division completes conversion.
pub const PREVIOUS_TEMPLATE: usize = 0x3c0;
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
/// The division's fuel, a signed fixed5 word: the movement speed's fuel ratio divides it
/// by the stat block's capacity, and the army writer saves it as `fuel`.
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
    /// Full COrdersGroup reference of the army this division belongs to, read through the embedded
    /// COrdersGroupMember back-pointer (`ORDERS_GROUP`). Zero preserves the native null pointer of
    /// an unassigned division; it is an independent second source for the group's own member
    /// vector, never a substitute for it.
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
    // A seed carries both fields from the census's gather at this boundary
    // ([`capture_division_seeds`]); a field that gather could not read failed
    // there and is not read again.
    let separate_counter = match seed {
        Some(seed) => seed.separate_counter,
        None => read::<u8>(memory, at(SEPARATE_COUNTER)?),
    }
    .ok_or("army separate-counter byte unreadable")?;
    let pointer = match seed {
        Some(seed) => seed.alternate_template,
        None => read::<u64>(memory, at(FAKE_INTEL_TEMPLATE)?)
            .and_then(|pointer| usize::try_from(pointer).ok()),
    }
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

/// Reads only a transition-dirty division, using its authenticated census identity.
pub fn pending_template(
    memory: &impl Memory,
    base: usize,
    army: usize,
) -> Result<u64, &'static str> {
    let pointer = army
        .checked_add(PREVIOUS_TEMPLATE)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("division previous template pointer unreadable")?;
    template_handle(memory, base, pointer)
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
    /// [`cunit::MOVEMENT_PRIORITY`], the raw signed movement priority of the
    /// movement arrow style, beside the retreat byte.
    pub movement_priority: Option<i32>,
    /// [`cunit::WITHDRAWAL`], the raw saved `withdraw` byte of the movement
    /// arrow style, beside the retreat byte.
    pub withdrawal: Option<u8>,
    /// [`FAKE_INTEL_TEMPLATE`], the counter's alternate template pointer,
    /// beside the standard template's line ([`read_icon_inputs`]).
    pub alternate_template: Option<usize>,
    /// [`SEPARATE_COUNTER`], on the supply grace scalars' line.
    pub separate_counter: Option<u8>,
    /// [`NONCORE_SUPPLY_BYPASS`], on the received supply ratio's line.
    pub noncore_supply_bypass: Option<u8>,
    /// [`OUT_OF_SUPPLY_DAYS`].
    pub out_of_supply_days: Option<i32>,
    /// [`FUEL_CANDIDATE`], the fuel the movement speed's fuel ratio reads, on the
    /// supply grace scalars' line.
    pub fuel: Option<i64>,
    /// [`AIR_SUPPLY`], which selects the noncore-supply term's air define.
    pub air_supply: Option<i64>,
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
    pub const MOVEMENT_PRIORITY: u64 = 1 << 36;
    pub const WITHDRAWAL: u64 = 1 << 37;
    pub const ALTERNATE_TEMPLATE: u64 = 1 << 38;
    pub const SEPARATE_COUNTER: u64 = 1 << 39;
    pub const NONCORE_SUPPLY_BYPASS: u64 = 1 << 40;
    pub const OUT_OF_SUPPLY_DAYS: u64 = 1 << 41;
    pub const FUEL: u64 = 1 << 42;
    pub const AIR_SUPPLY: u64 = 1 << 43;
}

/// The two arrow-style fields (beside `RETREAT`) share the retreat byte's line, so the census
/// gathers them there instead of the movement pass reading each division twice more; the counter
/// icon's alternate template and separate-counter byte share the standard template's and the supply
/// grace scalars' lines.
const DIVISION_SEED_SPANS: [(usize, usize); 44] = [
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
    (cunit::MOVEMENT_PRIORITY, 4),
    (cunit::WITHDRAWAL, 1),
    (FAKE_INTEL_TEMPLATE, 8),
    (SEPARATE_COUNTER, 1),
    (NONCORE_SUPPLY_BYPASS, 1),
    (OUT_OF_SUPPLY_DAYS, 4),
    (FUEL_CANDIDATE, 8),
    (AIR_SUPPLY, 8),
];
const DIVISION_SEED_FIELDS: [crate::memory::RecordField; 44] =
    crate::memory::packed_fields(DIVISION_SEED_SPANS);
const DIVISION_SEED_STRIDE: usize = crate::memory::packed_stride(DIVISION_SEED_SPANS);

/// Gather exact fixed layouts in bounded row batches. A failed field stays failed;
/// later fields remain independently available to other tables. The caller sets
/// each native pointer and expected full handle before capture.
///
/// The armies are scattered through the heap, so the gather is bound by their
/// cache misses; the record copy hints each field of the rows ahead of it.
/// Hinting whole batches ahead as well made the cold gather slower (the
/// `bench_division_seed_gather_cold` bench).
pub fn capture_division_seeds(memory: &impl Memory, seeds: &mut [DivisionSeed]) -> u64 {
    const BATCH: usize = 64;
    let mut bytes = [0u8; BATCH * DIVISION_SEED_STRIDE];
    let mut pointers = [0usize; BATCH];
    let mut total = 0u64;
    for first in (0..seeds.len()).step_by(BATCH) {
        let next = (first + BATCH).min(seeds.len());
        let chunk = &mut seeds[first..next];
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
            seed.movement_priority = (valid & DivisionSeed::MOVEMENT_PRIORITY != 0)
                .then(|| i32::from_le_bytes(record[336..340].try_into().unwrap()));
            seed.withdrawal = (valid & DivisionSeed::WITHDRAWAL != 0).then(|| record[340]);
            seed.alternate_template = (valid & DivisionSeed::ALTERNATE_TEMPLATE != 0)
                .then(|| u64::from_le_bytes(record[341..349].try_into().unwrap()) as usize);
            seed.separate_counter =
                (valid & DivisionSeed::SEPARATE_COUNTER != 0).then(|| record[349]);
            seed.noncore_supply_bypass =
                (valid & DivisionSeed::NONCORE_SUPPLY_BYPASS != 0).then(|| record[350]);
            seed.out_of_supply_days = (valid & DivisionSeed::OUT_OF_SUPPLY_DAYS != 0)
                .then(|| i32::from_le_bytes(record[351..355].try_into().unwrap()));
            seed.fuel = (valid & DivisionSeed::FUEL != 0)
                .then(|| i64::from_le_bytes(record[355..363].try_into().unwrap()));
            seed.air_supply = (valid & DivisionSeed::AIR_SUPPLY != 0)
                .then(|| i64::from_le_bytes(record[363..371].try_into().unwrap()));
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
    /// Raw saved `withdraw` byte, gathered with the division's seed and
    /// published after the established movement pass. An independent refusal
    /// withholds only this arrow-style operand.
    pub withdrawal: Option<u8>,
    /// Raw signed movement priority (saved `move_priority`); the arrow renderer
    /// tests <1 only for class-zero follow-orders arrows. Never clamp an
    /// unknown enum. Gathered with the division's seed, as `withdrawal`.
    pub movement_priority: Option<i32>,
    /// Complete ordered full identities in MovementPacket's owned arena.
    pub combat_references: Option<(usize, usize)>,
    pub combat_references_failure: Option<&'static str>,
    /// Raw signed fixed5 maximum speed of the authenticated stat block (the stat block's maximum
    /// speed), read only when the ordinary route count is positive. None is outside demand or a
    /// guarded refusal.
    pub base_speed: Option<i64>,
    /// [`FUEL_CANDIDATE`] already held by the census seed, including stationary
    /// divisions whose panel speed takes the fuel factor. None is a field refusal.
    pub fuel: Option<i64>,
    /// Regional transport speed acquired only for an embarked unit by the caller's companion.
    pub transport_speed: Option<i64>,
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
    /// Keeps nothing: every operand is read. Allocates nothing.
    pub(crate) fn vacant() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            worker_growth: false,
        }
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

pub fn movement_definition(
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
pub fn movement_attacking(
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

fn movement_bounded(value: i64, bound: i64) -> bool {
    (-bound..=bound).contains(&value)
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

/// The modifier keys the land movement speed reads for `template`'s movement
/// priority sub-unit ([`movement_priority_unit`]), appended to `keys` in native
/// order with repeats kept: the sub-unit's own key
/// ([`crate::csub_unit_definition::SPEED_MODIFIER_KEY`]) when nonzero, then each
/// nonzero key of its categories
/// ([`crate::csub_unit_definition::CATEGORY_MODIFIER_KEY`]). On a refusal `keys`
/// keeps only what it held before.
pub fn movement_speed_keys(
    memory: &impl Memory,
    base: usize,
    template: usize,
    keys: &mut Vec<u32>,
) -> Result<(), &'static str> {
    let start = keys.len();
    let result = movement_speed_key_demands(memory, base, template, keys);
    if result.is_err() {
        keys.truncate(start);
    }
    result
}

/// Acquisition demand keeps already-readable keys if a later category refuses.
/// The complete recorded template list still uses `movement_speed_keys`.
pub fn movement_speed_key_demands(
    memory: &impl Memory,
    base: usize,
    template: usize,
    keys: &mut Vec<u32>,
) -> Result<(), &'static str> {
    use crate::csub_unit_definition as unit;

    let selected = movement_priority_unit(memory, base, template)?;
    let token = presentation_field::<u32>(memory, selected, unit::SPEED_MODIFIER_KEY)?;
    if token != 0 {
        crate::allocation::reserve(keys, 1).map_err(|_| "movement speed key allocation failed")?;
        keys.push(token);
    }
    let categories = presentation_array(memory, selected, unit::CATEGORIES, 8)?;
    crate::allocation::reserve(keys, categories.count as usize)
        .map_err(|_| "movement speed key allocation failed")?;
    for index in 0..categories.count {
        let category = movement_pointer(
            memory,
            categories
                .element(index, 8)
                .ok_or("movement category index invalid")?,
            0,
        )?;
        let token = presentation_field::<u32>(memory, category, unit::CATEGORY_MODIFIER_KEY)?;
        if token != 0 {
            keys.push(token);
        }
    }
    Ok(())
}

mod strategic_action;
pub use strategic_action::read_inputs as strategic_redeployment_inputs;
pub use strategic_action::{StrategicAction, StrategicActionFailure, StrategicRedeploymentInputs};

mod movement;
#[cfg(feature = "movement_witness")]
pub mod movement_witness;
pub mod organisation_movement;
pub use movement::{
    MovementPacket, capture_movement_inputs, capture_movement_inputs_with_combat_references,
    capture_movement_inputs_with_extra, project_movement,
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
    /// Exact movement organisation pair: outer None refuses; inner None certifies no penalty.
    pub movement_organisation: Option<Option<[i64; 2]>>,
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
    /// The division's acclimatization camouflage category; `None` when it was
    /// unreadable or its category registry outgrew the packet this hour. Only
    /// this field fails, never the row.
    pub camouflage: Option<Camouflage>,
    /// Movement availability from the same acclimatization read, which the
    /// selected army's attack list scales its region weather term by; `None`
    /// when it was unreadable this hour.
    pub weather_factor: Option<i64>,
    /// Raid army-reach and movement-speed supply inputs, `MOVE_*` bits. The
    /// commander's retreat bans are [`RetreatBanRow`]'s.
    pub move_flags: u8,
    /// [`OUT_OF_SUPPLY_DAYS`]; `None` when that field alone was unreadable.
    pub out_of_supply_days: Option<i32>,
    pub border_combat_stats: Option<[i64; 3]>,
    /// Operand spans in the collector's hour-local name arena, never cached display text.
    pub naming: Option<crate::cname_group_member::Inputs>,
    pub country_ordinal: Option<u32>,
    /// Exact stored supply-footer scalars ([`crate::cunit_supply_inputs`]);
    /// only this pack fails on a scalar fault.
    pub supply: Option<crate::cunit_supply_inputs::Inputs>,
    /// The first of the footer's origin rows in [`StatusPacket::supply_origins`].
    pub supply_origins_start: usize,
    pub supply_failure: Option<crate::cunit_supply_inputs::Failure>,
}

/// The division's land-model camouflage: the token of the registered
/// acclimatization category whose `<entity>_<condition>` variants it may show
/// (the strongest entry, when its weight reaches the show threshold), or `None`
/// when no category does. The province and state conditions belong to the
/// consumer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Camouflage {
    pub category: Option<u32>,
}

impl StatusRow {
    /// [`EXILED`] is nonzero.
    pub const MOVE_EXILED: u8 = 1;
    /// The stat block's [`crate::csub_unit_definition::CAN_EXFILTRATE_FROM_COAST`] is nonzero.
    pub const MOVE_CAN_EXFILTRATE_FROM_COAST: u8 = 2;
    /// [`NONCORE_SUPPLY_BYPASS`] is nonzero: the movement speed's out-of-supply step
    /// adds no noncore-supply term.
    pub const MOVE_NONCORE_SUPPLY_BYPASS: u8 = 4;
    /// [`AIR_SUPPLY`] is positive: the noncore-supply term takes the air
    /// noncore-supply speed define instead of the ordinary one.
    pub const MOVE_NONCORE_AIR: u8 = 8;
    /// Air-supply sign unavailable; only the reached noncore speed branch refuses.
    pub const MOVE_NONCORE_AIR_REFUSED: u8 = 64;
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

mod status_parts;
pub use status_parts::StatusPart;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatusCapacities {
    pub units: usize,
    /// Ordered distributed-supply rows across the authenticated divisions.
    pub supply_origins: usize,
    pub manpower: usize,
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
    movement_organisation: Option<Option<[i64; 2]>>,
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
    noncore_supply_bypass: u8,
    air_supply: Option<i64>,
    out_of_supply_days: Option<i32>,
    model_number: i32,
    camouflage: Option<Camouflage>,
    weather_factor: Option<i64>,
    exercise: Option<StatusExerciseOperands>,
    /// Range-local decisions over its membership and exercise arenas.
    has_order: Option<Result<bool, &'static str>>,
    exercising: Option<Result<u8, &'static str>>,
    failure: Option<&'static str>,
    border_combat_stats: Option<[i64; 3]>,
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
    /// Certified experience lent to the scalar/exercise ranges.
    prepared_experience: Vec<Option<i64>>,
    /// Empty selects every footer; otherwise this capture's per-unit demand.
    supply_requested: Vec<bool>,
    organisation_memo: organisation_movement::OrganisationMemo,
    /// Each unit's supply footer, by unit index; the first `supply_len` are
    /// this hour's.
    supply: Vec<crate::cunit_supply_inputs::UnitCapture>,
    supply_len: usize,
    supply_origins: Vec<crate::cunit_supply_inputs::Origin>,
    supply_origins_len: usize,
    border_index: Vec<(u64, usize)>,
    border_stats: Vec<Option<[i64; 3]>>,
    maxima: Vec<crate::csub_unit_definition::DivisionMaxima>,
    maxima_index: Vec<usize>,
    /// Every maxima-index lookup ([`Self::maxima_slot`]), selection and
    /// admission alike.
    #[cfg(test)]
    maxima_probes: std::sync::atomic::AtomicUsize,
    /// Initialized once with packet storage, reused by every hourly pass.
    acclimatization: Option<Box<AcclimatizationBatch>>,
    countries: Vec<StatusCountryInputs>,
    country_index: Vec<usize>,
    groups: Vec<StatusGroupInputs>,
    group_index: Vec<usize>,
    /// Commander retreat bans by orders group, for divisions in a combat.
    /// Orders groups of divisions in combat, in first-seen order, and their bans.
    commander_groups: Vec<usize>,
    commander_bans: Vec<Result<Option<crate::carmy_leader::RetreatBans>, &'static str>>,
    commander_index: Vec<usize>,
    /// Each division the latest [`Self::retreat_bans`] read, by id, with the index of
    /// its group in `commander_bans` (`usize::MAX` for an ungrouped division).
    commander_divisions: Vec<(u64, usize)>,
    /// Per-walk ban metadata. Only the exclusive bans pass uses it; the
    /// mutex permits immutable status preparation to be shared with parts.
    commander_memo: std::sync::Mutex<crate::modifier_definition::DefinitionMemo>,
    modifiers: crate::modifier_accumulator::SnapshotArena,
    membership: MembershipPacket,
    manpower: Vec<i32>,
    exercise_control: Option<cgame_state::AiControl>,
    exercise_autonomy: std::collections::HashMap<u32, bool>,
    /// The category registry, loaded on the hour's first shown category.
    categories: crate::acclimatization_category::Categories,
    categories_loaded: Option<Result<(), &'static str>>,
    /// [`crate::acclimatization_data::CAMO_SHOW_AT_RVA`], read on the hour's
    /// first winning entry.
    camouflage_show_at: Option<Result<i64, &'static str>>,
    category_factors: CategoryFactors,
    thresholds_header: Option<Result<[u8; 16], &'static str>>,
    thresholds: Vec<Result<i64, &'static str>>,
    attrition_metadata: Option<Result<StatusAttritionMetadata, &'static str>>,
    attrition_define: Option<Result<i64, &'static str>>,
    maximum_level: Option<Result<i32, &'static str>>,
    minimum_strength: Option<Result<i64, &'static str>>,
    outcome: Outcome,
}

#[derive(Clone, Copy)]
struct SelectedMaxima {
    slot: usize,
    /// The gathered row, or no request for a memo hit or carried block.
    row: usize,
}

impl Default for SelectedMaxima {
    fn default() -> Self {
        Self {
            slot: usize::MAX,
            row: usize::MAX,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct StagedMaxima {
    pointer: usize,
    slot: usize,
    row: usize,
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
            prepared_experience: Vec::new(),
            supply_requested: Vec::new(),
            organisation_memo: organisation_movement::OrganisationMemo::vacant(),
            supply: Vec::new(),
            supply_len: 0,
            supply_origins: Vec::new(),
            supply_origins_len: 0,
            border_index: Vec::new(),
            border_stats: Vec::new(),
            maxima: Vec::new(),
            maxima_index: Vec::new(),
            #[cfg(test)]
            maxima_probes: std::sync::atomic::AtomicUsize::new(0),
            acclimatization: None,
            countries: Vec::new(),
            country_index: Vec::new(),
            groups: Vec::new(),
            group_index: Vec::new(),
            commander_groups: Vec::new(),
            commander_bans: Vec::new(),
            commander_index: Vec::new(),
            commander_divisions: Vec::new(),
            commander_memo: std::sync::Mutex::new(
                crate::modifier_definition::DefinitionMemo::default(),
            ),
            modifiers: crate::modifier_accumulator::SnapshotArena::vacant(),
            membership: MembershipPacket::vacant(),
            manpower: Vec::new(),
            exercise_control: None,
            exercise_autonomy: std::collections::HashMap::new(),
            categories: crate::acclimatization_category::Categories::default(),
            categories_loaded: None,
            camouflage_show_at: None,
            category_factors: CategoryFactors::default(),
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
        let mut exercise_autonomy = std::collections::HashMap::new();
        crate::allocation::reserve_map(&mut exercise_autonomy, capacities.units)?;
        Ok(Self {
            growing: false,
            capacities,
            required: capacities,
            units: crate::allocation::vec(capacities.units)?,
            prepared_experience: Vec::new(),
            supply_requested: Vec::new(),
            organisation_memo: organisation_movement::OrganisationMemo::new(capacities.units)
                .unwrap_or_else(|_| organisation_movement::OrganisationMemo::vacant()),
            supply: crate::allocation::filled(
                capacities.units,
                crate::cunit_supply_inputs::UnitCapture::default(),
            )?,
            supply_len: 0,
            supply_origins: crate::allocation::filled(
                capacities.supply_origins,
                crate::cunit_supply_inputs::Origin::default(),
            )?,
            supply_origins_len: 0,
            border_index: crate::allocation::vec(capacities.units)?,
            border_stats: crate::allocation::filled(capacities.units, None)?,
            maxima: crate::allocation::vec(capacities.units)?,
            maxima_index: crate::allocation::filled(index_capacity, 0)?,
            #[cfg(test)]
            maxima_probes: std::sync::atomic::AtomicUsize::new(0),
            acclimatization: if capacities.units == 0 {
                None
            } else {
                Some(crate::allocation::boxed(AcclimatizationBatch::new())?)
            },
            countries: crate::allocation::vec(capacities.units)?,
            country_index: crate::allocation::filled(index_capacity, 0)?,
            groups: crate::allocation::vec(capacities.units)?,
            group_index: crate::allocation::filled(index_capacity, 0)?,
            commander_groups: crate::allocation::vec(capacities.units)?,
            commander_bans: crate::allocation::vec(capacities.units)?,
            commander_index: crate::allocation::filled(index_capacity, 0)?,
            commander_divisions: crate::allocation::vec(capacities.units)?,
            // The two retreat-ban keys; a vacant packet stays allocation-free.
            commander_memo: std::sync::Mutex::new(
                crate::modifier_definition::DefinitionMemo::with_capacity(
                    if capacities.units == 0 { 0 } else { 2 },
                ),
            ),
            modifiers: crate::modifier_accumulator::SnapshotArena::try_new(capacities.modifiers)?,
            membership: MembershipPacket::try_with_capacity(
                capacities.membership.queries,
                capacities.membership.groups,
                capacities.membership.roots,
                capacities.membership.chunks,
            )?,
            manpower: crate::allocation::vec(capacities.manpower)?,
            exercise_control: None,
            exercise_autonomy,
            categories: crate::acclimatization_category::Categories::try_with_capacity(
                capacities.acclimatization_categories,
            )?,
            categories_loaded: None,
            camouflage_show_at: None,
            category_factors: CategoryFactors::default(),
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
        self.prepared_experience.clear();
        self.supply_len = 0;
        self.supply_origins_len = 0;
        self.border_index.clear();
        self.border_stats.fill(None);
        self.maxima.clear();
        self.maxima_index.fill(0);
        #[cfg(test)]
        self.maxima_probes
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.countries.clear();
        self.country_index.fill(0);
        self.groups.clear();
        self.group_index.fill(0);
        self.commander_groups.clear();
        self.commander_bans.clear();
        self.commander_index.fill(0);
        self.commander_divisions.clear();
        // Keep the memo's storage: a fresh memo would allocate on every walk.
        self.commander_memo
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.modifiers.clear();
        self.membership.clear();
        self.manpower.clear();
        self.exercise_control = None;
        self.exercise_autonomy.clear();
        self.categories.clear();
        self.categories_loaded = None;
        self.camouflage_show_at = None;
        self.category_factors = CategoryFactors::default();
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

    /// Caller-owned daily footer selection. This configuration survives packet
    /// clearing, like growth mode; set it for every acquisition. Failure selects
    /// all footers, leaving the optional carry unable to suppress a native read.
    pub fn select_supply(&mut self, requested: &[bool]) -> Result<(), &'static str> {
        self.supply_requested.clear();
        crate::allocation::reserve(&mut self.supply_requested, requested.len())
            .map_err(|_| "division supply selection allocation failed")?;
        self.supply_requested.extend_from_slice(requested);
        Ok(())
    }
    /// Inspect captured footer bytes only; skipped units have no native sample.
    pub fn supply_at(&self, index: usize) -> (
        Option<crate::cunit_supply_inputs::Inputs>,
        Option<crate::cunit_supply_inputs::Failure>,
        Option<&[crate::cunit_supply_inputs::Origin]>,
    ) {
        let unit = self.supply.get(index).copied().filter(|_| index < self.supply_len)
            .unwrap_or_default();
        let inputs = unit.inputs.ok();
        let failure = unit.inputs.err().or_else(|| inputs.and_then(|v| v.origins_len.err()));
        let origins = inputs.and_then(|v| v.origins_len.ok()).and_then(|len| {
            self.supply_origins.get(..self.supply_origins_len)?
                .get(unit.origins_start..unit.origins_start.checked_add(len)?)
        });
        (inputs, failure, origins)
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
            let mut packet = Self::with_capacities(next)?;
            std::mem::swap(&mut packet.supply_requested, &mut self.supply_requested);
            *self = packet;
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
            + self.prepared_experience.capacity() * size_of::<Option<i64>>()
            + self.supply_requested.capacity() * size_of::<bool>()
            + self.organisation_memo.allocated_bytes()
            + self.supply.capacity() * size_of::<crate::cunit_supply_inputs::UnitCapture>()
            + self.supply_origins.capacity() * size_of::<crate::cunit_supply_inputs::Origin>()
            + self.border_index.capacity() * size_of::<(u64, usize)>()
            + self.border_stats.capacity() * size_of::<Option<[i64; 3]>>()
            + self.maxima.capacity() * size_of::<crate::csub_unit_definition::DivisionMaxima>()
            + self.countries.capacity() * size_of::<StatusCountryInputs>()
            + self.groups.capacity() * size_of::<StatusGroupInputs>()
            + self.commander_groups.capacity() * size_of::<usize>()
            + self.commander_bans.capacity()
                * size_of::<Result<Option<crate::carmy_leader::RetreatBans>, &'static str>>()
            + self.commander_divisions.capacity() * size_of::<(u64, usize)>()
            + self
                .commander_memo
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .allocated_bytes()
            + (self.maxima_index.capacity()
                + self.country_index.capacity()
                + self.group_index.capacity()
                + self.commander_index.capacity())
                * size_of::<usize>()
            + self.modifiers.allocated_bytes()
            + self.membership.allocated_bytes()
            + self.manpower.capacity() * size_of::<i32>()
            + self.exercise_autonomy.capacity() * (size_of::<(u32, bool)>() + 1)
            + self.categories.allocated_bytes()
            + self.thresholds.capacity() * size_of::<Result<i64, &'static str>>()
            + self
                .acclimatization
                .as_ref()
                .map_or(0, |_| size_of::<AcclimatizationBatch>())
    }

    /// The only lookup of the maxima index.
    fn maxima_slot(&self, pointer: usize, key: impl Fn(usize) -> usize) -> usize {
        #[cfg(test)]
        self.maxima_probes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        status_pointer_slot(&self.maxima_index, pointer, key)
    }

    fn stage_maxima(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        batch: &mut crate::csub_unit_definition::MaximaBatch,
    ) -> [SelectedMaxima; 64] {
        let mut selected = [SelectedMaxima::default(); 64];
        let mut staged = [StagedMaxima::default(); 64];
        let mut staged_len = 0;
        let mut pointers = [0_usize; 64];
        let mut len = 0;
        let identity = base.checked_add(VTABLE_RVA).map(|at| at as u64);
        for (index, seed) in seeds.iter().enumerate() {
            if seed.primary_vtable != identity
                || seed.handle != Some(seed.division)
                || seed.division == 0
            {
                continue;
            }
            let Some(pointer) = seed.stat_block else {
                continue;
            };
            let slot = self.maxima_slot(pointer, |i| {
                if i < self.maxima.len() {
                    self.maxima[i].pointer
                } else {
                    staged[usize::MAX - 1 - i].pointer
                }
            });
            let resident = self.maxima_index[slot];
            let row = if resident == 0 {
                // Reserve only a key, never a value. An earlier carried seed
                // reserves its block without a request, so later aliases also
                // supply no speculative read.
                let carried = held
                    .get(index)
                    .copied()
                    .flatten()
                    .is_some_and(|value| value.pointer == pointer);
                let row = if carried || pointer == 0 {
                    usize::MAX
                } else {
                    pointers[len] = pointer;
                    len += 1;
                    len - 1
                };
                self.maxima_index[slot] = usize::MAX - staged_len;
                staged[staged_len] = StagedMaxima { pointer, slot, row };
                staged_len += 1;
                row
            } else if resident - 1 < self.maxima.len() {
                usize::MAX
            } else {
                staged[usize::MAX - resident].row
            };
            selected[index] = SelectedMaxima { slot, row };
        }
        // Undo reservations before native acquisition or serial admission.
        // The selected slots retain the same collision order as that admission.
        for pending in &staged[..staged_len] {
            self.maxima_index[pending.slot] = 0;
        }
        batch.capture(memory, &pointers[..len]);
        selected
    }

    fn maxima(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        selected: SelectedMaxima,
        held: Option<crate::csub_unit_definition::DivisionMaxima>,
        batch: &crate::csub_unit_definition::MaximaBatch,
    ) -> Result<usize, &'static str> {
        let slot = selected.slot;
        debug_assert_ne!(slot, usize::MAX);
        if self.maxima_index[slot] != 0 {
            return Ok(self.maxima_index[slot] - 1);
        }
        let value = match held.filter(|value| value.pointer == pointer) {
            Some(value) => value,
            None => batch.read(memory, base, pointer, selected.row)?,
        };
        let index = self.maxima.len();
        self.maxima.push(value);
        self.maxima_index[slot] = index + 1;
        Ok(index)
    }

    /// The stat-block maxima this pass used for seed `index`: read or carried,
    /// `None` for a seed the pass did not establish.
    pub fn unit_maxima(&self, index: usize) -> Option<crate::csub_unit_definition::DivisionMaxima> {
        let unit = self
            .units
            .get(index)
            .filter(|unit| unit.failure.is_none())?;
        self.maxima.get(unit.maxima).copied()
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
    /// (`retreat_bans_many`, which skips the commanders `held` returns),
    /// keeping each group's own result for [`Self::commander`] to report at
    /// that division, in source order.
    fn prepare_commanders(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        wanted: &dyn Fn(&DivisionSeed) -> bool,
        held: &mut dyn FnMut(usize) -> Option<crate::carmy_leader::RetreatBans>,
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
            self.commander_memo
                .get_mut()
                .unwrap_or_else(|error| error.into_inner()),
            held,
        );
    }

    /// The movement entries of `division`'s commander block `block`
    /// ([`COMMANDER_BLOCK_NONE`], [`COMMANDER_BLOCK_ORDINARY`] or
    /// [`COMMANDER_BLOCK_SEPARATE_HQ`]) from the latest [`Self::retreat_bans`],
    /// which reads them with the bans: all absent without a commander, None where
    /// that read did not reach the division or its commander, or disagrees with
    /// `block`.
    pub fn commander_movement(
        &self,
        division: u64,
        block: u8,
    ) -> Option<crate::carmy_leader::MovementEntries> {
        let absent = [None; crate::carmy_leader::MOVEMENT_MODIFIER_KEYS.len()];
        let at = self
            .commander_divisions
            .binary_search_by_key(&division, |entry| entry.0)
            .ok()?;
        let bans = match self.commander_divisions[at].1 {
            usize::MAX => None,
            index => (*self.commander_bans.get(index)?).ok()?,
        };
        match (bans, block) {
            (_, COMMANDER_BLOCK_NONE) => Some(absent),
            (Some(bans), COMMANDER_BLOCK_ORDINARY) => Some(bans.movement[0]),
            (Some(bans), COMMANDER_BLOCK_SEPARATE_HQ) => Some(bans.movement[1]),
            _ => None,
        }
    }

    /// Each commander the latest [`Self::retreat_bans`] reached, with its
    /// group's result, in read order.
    pub fn commander_bans(
        &self,
    ) -> impl Iterator<Item = Result<Option<crate::carmy_leader::RetreatBans>, &'static str>> + '_
    {
        self.commander_bans.iter().copied()
    }

    /// The retreat bans of every division whose raw roster owner `owner`
    /// maps to a read owner, one row per division with a ban, in source
    /// order. The hourly read of `DivisionRetreatBans`: each wanted group's
    /// commander is read once, both blocks, unless `held` returns its bans,
    /// and a division's block is selected only when they differ. A failed
    /// read fails the table.
    ///
    /// `owner` maps a raw roster owner to `Ok(Some(owner))` for a read owner
    /// and `Ok(None)` for one outside the read; `Err` is an owner that could
    /// not be resolved. A division whose owner is unreadable (its roster
    /// owner field or `owner` failed) has no row owner, so the read returns
    /// no rows at all: it is truncated, with those divisions as its unread
    /// rows.
    #[allow(clippy::too_many_arguments)] // The census, its outcome, the owner map, output and held bans.
    pub fn retreat_bans(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        source: Outcome,
        owner: &dyn Fn(u32) -> Result<Option<u32>, &'static str>,
        output: &mut [RetreatBanRow],
        held: &mut dyn FnMut(usize) -> Option<crate::carmy_leader::RetreatBans>,
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
            self.commander_divisions.clear();
            self.commander_memo
                .get_mut()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
            self.prepare_commanders(
                memory,
                base,
                seeds,
                &|seed| matches!(owned(seed), Ok(Some(_))),
                held,
            );
            let (mut len, mut unread) = (0, 0_u32);
            for seed in seeds {
                let Ok(Some(owner)) = owned(seed) else {
                    continue;
                };
                let bans = self.commander(memory, base, seed)?;
                if self.commander_divisions.len() < self.commander_divisions.capacity() {
                    let index = match seed.orders_group {
                        Some(group) if group != 0 => {
                            let slot = status_pointer_slot(&self.commander_index, group, |i| {
                                self.commander_groups[i]
                            });
                            self.commander_index[slot].wrapping_sub(1)
                        }
                        _ => usize::MAX,
                    };
                    self.commander_divisions.push((seed.division, index));
                }
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
            self.commander_divisions
                .sort_unstable_by_key(|entry| entry.0);
            Ok(if unread == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: unread,
                }
            })
        })();
        result.unwrap_or_else(|reason| {
            // A failed read keeps no partly filled, unsorted division list.
            self.commander_divisions.clear();
            Outcome::Unavailable { reason }
        })
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
        held_experience: Option<i64>,
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
                // The admitted array header checked the entire stride-8 range. Keep only complete
                // manpower words, including on a later fault.
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
            Some(match held_experience {
                Some(value) => value,
                None => presentation_field::<i64>(memory, army, EXPERIENCE)?,
            })
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

    /// AI control from the first observation of each raw owner in this Status
    /// pass. The existing country reader and AiControl headers/counts remain
    /// valid while the hourly callback parks the game; human-control changes
    /// and loads cannot run before its jobs join. Acquire only at the exercise
    /// gate, never ahead of a prior unit's work. The first error aborts Status,
    /// so no failed observation is retried.
    fn exercise_autonomous<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        countries: &ccountry::Reader<'_, M>,
        raw: u32,
    ) -> Result<bool, &'static str> {
        if let Some(&value) = self.exercise_autonomy.get(&raw) {
            return Ok(value);
        }
        let canonical = if raw == 0 {
            0
        } else {
            countries.resolve(raw)?.canonical_slot
        };
        if self.exercise_control.is_none() {
            self.exercise_control = Some(cgame_state::AiControl::read(memory, state)?);
        }
        let value = self
            .exercise_control
            .as_mut()
            .unwrap()
            .autonomous(memory, base, countries, raw, canonical)?;
        if self.exercise_autonomy.len() < self.exercise_autonomy.capacity() {
            self.exercise_autonomy.insert(raw, value);
        }
        Ok(value)
    }

    fn capture_exercise(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        unit: usize,
        held_experience: Option<i64>,
        autonomous: impl FnOnce(&mut Self, u32) -> Result<bool, &'static str>,
    ) -> Result<StatusExerciseOperands, &'static str> {
        let mut inputs = StatusExerciseOperands::default();

        let owner = seed
            .roster_owner
            .ok_or("division presentation field unreadable")?;
        inputs.maximum_level = (*self
            .maximum_level
            .get_or_insert_with(|| presentation_field(memory, base, EXERCISE_MAXIMUM_LEVEL_RVA)))?;
        inputs.autonomous = autonomous(self, owner)?;
        if inputs.autonomous {
            let level = self.capture_level(memory, base, seed.pointer, held_experience)?;
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
            inputs.level = Some(self.capture_level(memory, base, seed.pointer, held_experience)?);
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
        if let Some(decision) = unit.exercising {
            return decision;
        }
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

    /// Camouflage from the division's embedded acclimatization data. A failure,
    /// including a full category registry, leaves only this division's field
    /// `None`; the packet still asks for the room it lacked.
    fn capture_camouflage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        unit: usize,
        first: Option<&[u8]>,
    ) {
        let (camouflage, weather_factor) = match acclimatization_winner(memory, base, seed, first) {
            Ok((object, winner)) => (
                self.read_camouflage(memory, base, &object, winner),
                weather_factor(memory, &object, winner, &mut self.category_factors),
            ),
            Err(reason) => (Err(reason), Err(reason)),
        };
        self.units[unit].camouflage = camouflage.ok();
        self.units[unit].weather_factor = weather_factor.ok();
    }

    /// Winner selection walks every entry for the first strictly greatest weight above zero; one
    /// span read per 32 entries copies them here. The camouflage decision shows nothing for no
    /// winner or a null category, and otherwise compares the winner's weight with the maximum times
    /// the live `ACCLIMATIZATION_CAMO_SHOW_AT`. Only a shown category's pointer is resolved to its
    /// token through the registry, as only it is used; the define and the registry are read once
    /// per pass, when first needed.
    fn read_camouflage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        object: &[u8; 0x40],
        winner: Option<(u64, i64)>,
    ) -> Result<Camouflage, &'static str> {
        use crate::acclimatization_data as data;
        let Some((category, weight)) = winner.filter(|(category, _)| *category != 0) else {
            return Ok(Camouflage { category: None });
        };
        let show_at = *self
            .camouflage_show_at
            .get_or_insert_with(|| presentation_field::<i64>(memory, base, data::CAMO_SHOW_AT_RVA));
        let maximum =
            i64::from_le_bytes(object[data::MAXIMUM..data::MAXIMUM + 8].try_into().unwrap());
        if !data::shows(weight, maximum, show_at?) {
            return Ok(Camouflage { category: None });
        }
        let categories = *self.categories_loaded.get_or_insert_with(|| {
            if self.growing {
                self.categories.load_growing(
                    memory,
                    base,
                    &mut self.required.acclimatization_categories,
                )
            } else {
                self.categories
                    .load(memory, base, &mut self.required.acclimatization_categories)
            }
        });
        categories?;
        let token = usize::try_from(category)
            .ok()
            .and_then(|pointer| self.categories.token(pointer))
            .ok_or("division acclimatization category is not registered")?;
        Ok(Camouflage {
            category: Some(token),
        })
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
        self.capture_holding(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            row_capacity,
            &[],
            &[],
        )
    }

    /// [`Self::capture`] with the caller's carried maxima: `held[i]` is seed
    /// `i`'s stat-block maxima when no writer touched its block since the
    /// read they came from, and is used in place of reading that block when
    /// it names the seed's block. Any other seed's block is read.
    #[allow(clippy::too_many_arguments)]
    pub fn capture_holding(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        row_capacity: usize,
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        experience: &[Option<i64>],
    ) -> Outcome {
        let prepared = self.prepare_holding_inner(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            row_capacity,
            held,
            false,
        );
        self.finish_holding(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            experience,
            prepared,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_holding_inner<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        row_capacity: usize,
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        defer_membership: bool,
    ) -> Result<(usize, ccountry::Reader<'a, M>), &'static str> {
        self.clear();
        let prepared = (|| {
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
            let mut batch = crate::csub_unit_definition::MaximaBatch::new();
            let mut selected = [SelectedMaxima::default(); 64];
            let identity = base.checked_add(VTABLE_RVA).map(|at| at as u64);
            for (index, seed) in seeds[..count].iter().enumerate() {
                if index.is_multiple_of(64) {
                    let end = (index + 64).min(count);
                    selected = self.stage_maxima(
                        memory,
                        base,
                        &seeds[index..end],
                        held.get(index..held.len().min(end)).unwrap_or(&[]),
                        &mut batch,
                    );
                }
                let unit = self.units.len();
                self.units.push(StatusUnitInputs {
                    division: seed.division,
                    ..StatusUnitInputs::default()
                });

                let result = (|| -> Result<(), &'static str> {
                    if seed.primary_vtable != identity
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
                    self.units[unit].maxima = self.maxima(
                        memory,
                        base,
                        stats,
                        selected[index % 64],
                        held.get(index).copied().flatten(),
                        &batch,
                    )?;

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
            Ok((count, countries))
        })();
        // Query outcomes are independent and retain their first failing root.
        // Finishing in source order preserves a prior unit's scalar/exercise
        // error ahead of a later unit's membership or preparation failure.
        if !defer_membership {
            self.membership.acquire(memory, base);
        }
        prepared
    }

    /// Ordered shared maxima/country admission only. Independent membership,
    /// scalar and exercise acquisition belongs to each final-status range.
    /// Maxima stay ordered because aliases must retain the first seed's held
    /// observation, even when a later alias offers contradictory operands.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_holding(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        experience: &[Option<i64>],
    ) -> Outcome {
        let prepared = self.prepare_holding_inner(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            seeds.len(),
            held,
            true,
        );
        self.outcome = match prepared {
            Ok((count, _)) => {
                if crate::allocation::reserve(&mut self.prepared_experience, experience.len())
                    .is_err()
                {
                    Outcome::Unavailable {
                        reason: "status experience allocation failed",
                    }
                } else {
                    self.prepared_experience.extend_from_slice(experience);
                    match source_outcome {
                        Outcome::Truncated { unread_rows, .. } => Outcome::Truncated {
                            len: count,
                            unread_rows,
                        },
                        _ => Outcome::Rows { len: count },
                    }
                }
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        self.outcome
    }

    /// Number of prepared units, including the first unit whose preparation
    /// failed. Later ranges must never manufacture maxima for its suffix.
    pub fn prepared_units(&self) -> usize {
        self.units.len()
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_holding<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        experience: &[Option<i64>],
        prepared: Result<(usize, ccountry::Reader<'_, M>), &'static str>,
        defer_children: bool,
    ) -> Outcome {
        let mut entries = self.acclimatization.take();
        let result = prepared.and_then(|(count, countries)| {
            let admitted = &seeds[..self.units.len()];
            if !admitted.is_empty() {
                let entries = entries
                    .as_deref_mut()
                    .expect("admitted packet has entry scratch");
                for (unit, seed) in admitted.iter().enumerate() {
                    if !defer_children && unit.is_multiple_of(64) {
                        entries.capture(
                            memory,
                            base,
                            &admitted[unit..(unit + 64).min(admitted.len())],
                        );
                    }
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
                    self.units[unit].noncore_supply_bypass = seed
                        .noncore_supply_bypass
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].air_supply = seed.air_supply;
                    self.units[unit].out_of_supply_days = seed.out_of_supply_days;
                    self.units[unit].model_number = seed
                        .model_number
                        .ok_or("division presentation field unreadable")?;
                    if !defer_children {
                        self.capture_camouflage(memory, base, seed, unit, entries.first(unit % 64));
                    }
                    self.units[unit].training = self.training(memory, self.units[unit].group)?;
                    if self.units[unit].training != 0 {
                        self.units[unit].exercise = Some(self.capture_exercise(
                            memory,
                            base,
                            seed,
                            unit,
                            experience.get(unit).copied().flatten(),
                            |packet, raw| {
                                packet.exercise_autonomous(memory, base, state, &countries, raw)
                            },
                        )?);
                    }
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
        self.acclimatization = entries;
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        if defer_children {
            return self.outcome;
        }
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
        self.capture_status_children(memory, seeds);
        self.outcome
    }

    fn capture_status_children(&mut self, memory: &impl Memory, seeds: &[DivisionSeed]) {
        // Complete the established scalar/exercise prefix before these optional
        // operands. A failed read must neither withhold that prefix nor
        // prevent the collector's later naming and border-stat acquisition.
        for (unit, seed) in self.units.iter_mut().zip(seeds).take(self.outcome.len()) {
            if unit.invalid_orders != 0 {
                unit.invalid_orders_reason = seed
                    .pointer
                    .checked_add(cunit::INVALID_ORDERS_REASON)
                    .and_then(|address| read(memory, address));
            }
        }
        // The supply child's footer for every admitted division, in record
        // batches; each division's pack and origin rows fail alone.
        self.supply_len = self.outcome.len().min(self.supply.len());
        let used = crate::cunit_supply_inputs::capture_selected_units(
            memory,
            |index| seeds[index].pointer,
            |index| self.supply_requested.get(index).copied().unwrap_or(true),
            &mut self.supply[..self.supply_len],
            &mut self.supply_origins,
            self.growing,
        );
        self.supply_origins_len = used.len;
        self.required.supply_origins = self.required.supply_origins.max(used.required);
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
        for (index, (unit, row)) in packet
            .units
            .iter()
            .zip(output.iter_mut())
            .take(packet.outcome.len())
            .enumerate()
        {
            let maxima = packet
                .maxima
                .get(unit.maxima)
                .ok_or("division status maxima index invalid")?;
            let supply = packet
                .supply
                .get(..packet.supply_len)
                .and_then(|supply| supply.get(index))
                .copied()
                .unwrap_or_default();
            let (maximum_hit_points, maximum_organisation) = maxima.effective();
            let move_flags = (u8::from(unit.exiled != 0) * StatusRow::MOVE_EXILED)
                | (u8::from(maxima.can_exfiltrate_from_coast != 0)
                    * StatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST)
                | (u8::from(unit.noncore_supply_bypass != 0)
                    * StatusRow::MOVE_NONCORE_SUPPLY_BYPASS)
                | (u8::from(unit.air_supply.is_some_and(|value| value > 0))
                    * StatusRow::MOVE_NONCORE_AIR)
                | (u8::from(unit.air_supply.is_none()) * StatusRow::MOVE_NONCORE_AIR_REFUSED);
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
            let has_order = unit
                .has_order
                .unwrap_or_else(|| packet.membership.outcome(unit.membership))?;
            *row = StatusRow {
                division: unit.division,
                maximum_hit_points,
                maximum_organisation,
                movement_organisation: unit.movement_organisation,
                entrenchment: unit.entrenchment,
                entrenchment_cap: unit.entrenchment_cap,
                supply_ratio: unit.supply_ratio,
                supply_grace_maximum: unit.supply_grace_maximum,
                supply_grace_remaining: unit.supply_grace_remaining,
                attrition_value: unit.attrition_value,
                attrition_threshold,
                attrition: u8::from(attrition_threshold < unit.attrition_value),
                exercising: match unit.exercising {
                    Some(decision) => decision?,
                    None => u8::from(match unit.exercise {
                        Some(inputs) => packet.exercise(inputs, maximum_hit_points)?,
                        None => false,
                    }),
                },
                has_order: u8::from(has_order),
                invalid_orders: unit.invalid_orders,
                invalid_orders_reason: unit.invalid_orders_reason,
                strength_factor: unit.strength_factor,
                move_flags,
                out_of_supply_days: unit.out_of_supply_days,
                model_number: unit.model_number,
                camouflage: unit.camouflage,
                weather_factor: unit.weather_factor,
                border_combat_stats: unit.border_combat_stats,
                naming: None,
                country_ordinal: None,
                supply: supply.inputs.ok(),
                supply_origins_start: supply.origins_start,
                supply_failure: match supply.inputs {
                    Ok(inputs) => inputs.origins_len.err(),
                    Err(reason) => Some(reason),
                },
            };
        }
        Ok(packet.outcome)
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// A division's acclimatization bytes and the selected entry's key and
/// weight ([`acclimatization_winner`]).
type AcclimatizationWinner = ([u8; 0x40], Option<(u64, i64)>);

/// Winner selection walks entries in their native order. Each row retains its
/// first 32 entries; longer arrays continue with the same serial span reads,
/// with no content cap. Owned CData identities and array arithmetic validate
/// sources.
struct AcclimatizationBatch {
    bytes: [[u8; 32 * crate::acclimatization_entry::STRIDE]; 64],
    lengths: [usize; 64],
    copied: usize,
}

impl AcclimatizationBatch {
    fn new() -> Self {
        Self {
            bytes: [[0; 32 * crate::acclimatization_entry::STRIDE]; 64],
            lengths: [0; 64],
            copied: 0,
        }
    }

    fn capture(&mut self, memory: &impl Memory, base: usize, seeds: &[DivisionSeed]) {
        use crate::{acclimatization_data as data, acclimatization_entry as entry};
        self.lengths.fill(0);
        let mut addresses = [0_usize; 64];
        for (index, seed) in seeds.iter().enumerate() {
            let Some(object) = seed.acclimatization else {
                continue;
            };
            if Some(u64::from_le_bytes(object[..8].try_into().unwrap()))
                != base.checked_add(data::VTABLE_RVA).map(|at| at as u64)
            {
                continue;
            }
            if let Some(array) = Array::from_header(
                object[data::DATA..data::DATA + 16].try_into().unwrap(),
                entry::STRIDE,
            ) && array.count != 0
            {
                addresses[index] = array.data;
                self.lengths[index] = (array.count as usize).min(32) * entry::STRIDE;
            }
        }
        if self.lengths.iter().all(|&len| len == 0) {
            self.copied = 0;
            return;
        }
        let mut rows = self.bytes.iter_mut();
        let mut destinations: [&mut [u8]; 64] =
            std::array::from_fn(|index| &mut rows.next().unwrap()[..self.lengths[index]]);
        self.copied = memory.copy_scattered_prefetched(addresses, &mut destinations);
        if self.copied < 64 {
            crate::batch_fallback::record(crate::batch_fallback::Batch::DivisionAcclimatization);
        }
    }

    fn first(&self, index: usize) -> Option<&[u8]> {
        (index < self.copied && self.lengths[index] != 0)
            .then(|| &self.bytes[index][..self.lengths[index]])
    }
}

/// The division's embedded acclimatization data (the seed's first `0x40` bytes)
/// and its selected entry: the first strictly greatest weight above zero, read
/// 32 entries per span.
fn acclimatization_winner(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    gathered_first: Option<&[u8]>,
) -> Result<AcclimatizationWinner, &'static str> {
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
    const SPAN: usize = 32;
    let mut bytes = [std::mem::MaybeUninit::uninit(); SPAN * entry::STRIDE];
    let mut strongest = data::Strongest::new();
    for first in (0..count).step_by(SPAN) {
        let len = (count - first).min(SPAN);
        let at = entries
            .element(first as u32, entry::STRIDE)
            .ok_or("division acclimatization entries invalid")?;
        let raw_entries = match gathered_first.filter(|_| first == 0) {
            Some(gathered) => gathered,
            None => crate::memory::read_slice_uninit(memory, at, &mut bytes[..len * entry::STRIDE])
                .ok_or("division acclimatization entries unreadable")?,
        };
        for raw in raw_entries.chunks_exact(entry::STRIDE) {
            strongest.offer(
                u64::from_le_bytes(
                    raw[entry::CATEGORY..entry::CATEGORY + 8]
                        .try_into()
                        .unwrap(),
                ),
                i64::from_le_bytes(raw[entry::WEIGHT..entry::WEIGHT + 8].try_into().unwrap()),
            );
        }
    }
    Ok((object, strongest.winner()))
}

/// The acclimatization categories' movement factors read this pass. A category is content every
/// division in its climate shares, so each is read once a pass; a full memo only costs a fresh
/// read.
#[derive(Default)]
struct CategoryFactors {
    entries: [(u64, i64); 8],
    len: usize,
}

impl CategoryFactors {
    fn factor(&mut self, memory: &impl Memory, category: u64) -> Result<i64, &'static str> {
        if let Some(&(_, factor)) = self.entries[..self.len]
            .iter()
            .find(|(held, _)| *held == category)
        {
            return Ok(factor);
        }
        let factor = usize::try_from(category)
            .ok()
            .and_then(|at| at.checked_add(crate::acclimatization_data::CATEGORY_FACTOR))
            .and_then(|address| crate::read::<i64>(memory, address))
            .ok_or("division acclimatization category factor unreadable")?;
        if let Some(entry) = self.entries.get_mut(self.len) {
            *entry = (category, factor);
            self.len += 1;
        }
        Ok(factor)
    }
}

/// Movement availability: one without an active climate or when the winner's category is not that
/// one, otherwise [`crate::acclimatization_data::movement_factor`] with the active category's
/// factor, read once a pass ([`CategoryFactors`]).
fn weather_factor(
    memory: &impl Memory,
    object: &[u8; 0x40],
    winner: Option<(u64, i64)>,
    factors: &mut CategoryFactors,
) -> Result<i64, &'static str> {
    use crate::acclimatization_data as data;
    let active = u64::from_le_bytes(object[data::ACTIVE..data::ACTIVE + 8].try_into().unwrap());
    let Some((_, weight)) = winner.filter(|(category, _)| active != 0 && *category == active)
    else {
        return Ok(100_000);
    };
    let maximum = i64::from_le_bytes(object[data::MAXIMUM..data::MAXIMUM + 8].try_into().unwrap());
    Ok(data::movement_factor(
        weight,
        maximum,
        factors.factor(memory, active)?,
    ))
}

/// Counter's dig-in duration reads the selected commander's unit modifier
/// block, not the leader's own weather block.
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

/// One pass's [`counter_dig_in_factor`] reads that the divisions of a group,
/// or of a commander, share: the leader selection's group reads
/// ([`crate::carmy_leader::LeaderMemo`]) and each selected modifier block's
/// entrenchment term. Within one quiescent acquisition neither can change, so
/// every division under the same commander block reads it once.
pub struct DigInMemo {
    leaders: crate::carmy_leader::LeaderMemo,
    values: crate::carmy_leader::PassTable<Result<i64, &'static str>>,
}

impl DigInMemo {
    /// Room for `divisions` divisions, each in its own group under its own
    /// commander; zero allocates nothing.
    pub fn try_with_capacity(divisions: usize) -> std::io::Result<Self> {
        Ok(Self {
            leaders: crate::carmy_leader::LeaderMemo::try_with_capacity(divisions)?,
            values: crate::carmy_leader::PassTable::try_with_capacity(divisions)?,
        })
    }

    /// Keeps nothing: every division reads afresh. Allocates nothing.
    pub fn vacant() -> Self {
        Self {
            leaders: crate::carmy_leader::LeaderMemo::vacant(),
            values: crate::carmy_leader::PassTable::vacant(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.leaders.capacity().min(self.values.capacity())
    }

    pub fn allocated_bytes(&self) -> usize {
        self.leaders.allocated_bytes() + self.values.allocated_bytes()
    }

    /// Starts a pass: nothing a previous pass read is used again.
    pub fn clear(&mut self) {
        self.leaders.clear();
        self.values.clear();
    }
}

/// [`counter_dig_in_factor`] over a pass's [`DigInMemo`]: the same factor and
/// the same first failure, with each group's selection reads and each
/// commander block's term read once per pass.
pub fn counter_dig_in_factor_memoized(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    memo: &mut DigInMemo,
) -> Result<i64, &'static str> {
    counter_dig_in_selection_memoized(memory, base, seed, memo).map(|(factor, _)| factor)
}

/// The commander unit modifier block a division's movement speed, out-of-supply
/// penalty and dig-in factor read: none without a commander, else the ordinary or
/// the separate-HQ block its HQ selects.
pub const COMMANDER_BLOCK_NONE: u8 = 0;
pub const COMMANDER_BLOCK_ORDINARY: u8 = 1;
pub const COMMANDER_BLOCK_SEPARATE_HQ: u8 = 2;

/// [`counter_dig_in_factor_memoized`] with the commander block the selection chose
/// (`COMMANDER_BLOCK_*`), from the same reads.
pub fn counter_dig_in_selection_memoized(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    memo: &mut DigInMemo,
) -> Result<(i64, u8), &'static str> {
    let Some((root, separate)) =
        crate::carmy_leader::movement_block_memoized(memory, base, seed, &mut memo.leaders)?
    else {
        return Ok((100_000, COMMANDER_BLOCK_NONE));
    };
    let block = if separate {
        COMMANDER_BLOCK_SEPARATE_HQ
    } else {
        COMMANDER_BLOCK_ORDINARY
    };
    memo.values
        .get_or(root, || {
            crate::modifier_accumulator::requested_effective_at(
                memory,
                base,
                root,
                crate::modifier_keys::COMBAT_ENTRENCHMENT,
            )
        })
        .map(|value| (100_000_i64.wrapping_add(value), block))
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
    let inputs = packet.capture_level(memory, base, army, None)?;
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
    let inputs = packet.capture_exercise(memory, base, &seeds[0], 0, None, |_, raw| {
        cgame_state::autonomous_country(memory, base, state, raw)
    })?;
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

mod presentation_demand;
pub use presentation_demand::{PresentationDemand, presentation_demand};

#[cfg(test)]
#[path = "carmy/border/tests.rs"]
mod border_tests;
#[cfg(test)]
mod tests;

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod bench;

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod status_bench;
