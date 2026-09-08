use super::*;
use super::division_combat_membership as membership;
use compact_contract::division_routes::MAX_ROUTE_PROVINCES;

const ORDINARY_ROUTE: usize = 0x1f0;
const ALTERNATE_ROUTE: usize = 0x210;

fn division_identity<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<[u64; 4]> {
    let identity = [
        mem.u64(body)?,
        mem.u32(body.checked_add(DIVISION_OFF_ID_TYPE)?)? as u64,
        mem.u32(body.checked_add(DIVISION_OFF_ID_ID)?)? as u64,
        mem.u32(body.checked_add(DIVISION_OFF_LOCATION)?)? as u64,
    ];
    if identity[0] != module.checked_add(DIVISION_VTABLE_RVA)? as u64
        || !is_compact_division_body_type(identity[1] as u32)
        || identity[2] != id as u64
    {
        return None;
    }
    Some(identity)
}

#[derive(PartialEq, Eq)]
struct TemplateRead {
    identity: [u64; 4],
    override_pointer: usize,
    selected_pointer: usize,
    template: Option<u32>,
}

fn template_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<TemplateRead> {
    let identity = division_identity(mem, module, body, id)?;
    // The collector body is CDivision + 0x10; the current template is CDivision + 0x3c8.
    let override_pointer = mem.u64(body.checked_add(0x3b8)?)? as usize;
    let selected_pointer = if override_pointer == 0 {
        mem.u64(body.checked_add(DIVISION_OFF_TEMPLATE)?)? as usize
    } else {
        override_pointer
    };
    let template = if selected_pointer == 0 {
        None
    } else {
        if !is_map_control_heap_pointer(selected_pointer)
            || mem.u64(selected_pointer)?
                != module.checked_add(DIVISION_TEMPLATE_VTABLE_RVA)? as u64
            || mem.u32(selected_pointer.checked_add(DIVISION_TEMPLATE_OFF_TYPE)?)?
                != DIVISION_TEMPLATE_TYPE
        {
            return None;
        }
        let template_id = mem.u32(selected_pointer.checked_add(DIVISION_TEMPLATE_OFF_ID)?)?;
        if template_id == 0 || template_id > MAX_DIVISION_TEMPLATE_ID {
            return None;
        }
        Some(template_id)
    };
    Some(TemplateRead {
        identity,
        override_pointer,
        selected_pointer,
        template,
    })
}

#[cfg(test)]
fn stable_template<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> Option<Option<u32>> {
    let first = template_once(mem, module, body, id)?;
    between();
    let second = template_once(mem, module, body, id)?;
    (first == second).then_some(first.template)
}

#[derive(PartialEq, Eq)]
struct PreferenceRead {
    string_custody: [usize; 3],
    exclusions: Option<ExclusionRead>,
    preference: compact_contract::division_routes::TemplateModelPreference,
}

#[derive(PartialEq, Eq)]
struct ExclusionRead {
    header: [u64; 3],
    pointers: Vec<u64>,
    ids: Vec<Option<u32>>,
}

fn exclusions_once<M: DecodeMemory + ?Sized>(mem: &M, module: usize, template: usize) -> Option<ExclusionRead> {
    let at = template.checked_add(0x140)?;
    let data = mem.u64(at)?;
    let capacity = mem.u32(at.checked_add(8)?)?;
    let count = mem.u32(at.checked_add(12)?)?;
    if count > capacity || count as usize > compact_contract::division_routes::MAX_TEMPLATE_EXCLUSIONS {
        return None;
    }
    let mut pointers = Vec::with_capacity(count as usize);
    let mut ids = Vec::with_capacity(count as usize);
    if count != 0 {
        if !is_map_control_heap_pointer(data as usize) { return None; }
        let bytes = mem.bytes(data as usize, (count as usize).checked_mul(8)?)?;
        for entry in bytes.chunks_exact(8) {
            let pointer = u64::from_le_bytes(entry.try_into().ok()?);
            pointers.push(pointer);
            if pointer == 0 { ids.push(None); continue; }
            let pointer = pointer as usize;
            if !is_map_control_heap_pointer(pointer)
                || mem.u64(pointer)? != module.checked_add(DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA)? as u64
                || mem.u32(pointer.checked_add(DIVISION_EQUIPMENT_VARIANT_OFF_TYPE)?)? != DIVISION_EQUIPMENT_VARIANT_TYPE {
                return None;
            }
            let id = mem.u32(pointer.checked_add(DIVISION_EQUIPMENT_VARIANT_OFF_ID)?)?;
            if id == 0 || id > MAX_DIVISION_EQUIPMENT_VARIANT_ID { return None; }
            ids.push(Some(id));
        }
    }
    Some(ExclusionRead { header: [data, capacity as u64, count as u64], pointers, ids })
}

fn preference_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    selected: &TemplateRead,
) -> Option<PreferenceRead> {
    let template_id = selected.template?;
    let pointer = selected.selected_pointer;
    let entity = read_msvc_string_snapshot_once_in(
        mem,
        pointer.checked_add(0x210)?,
        compact_contract::division_routes::MAX_MODEL_ENTITY_BYTES,
    )?;
    let present = mem.scalar::<1>(pointer.checked_add(0x208)?)?[0];
    let category = if present == 0 {
        None
    } else {
        Some(mem.u64(pointer.checked_add(0x200)?)?)
    };
    Some(PreferenceRead {
        string_custody: [entity.size, entity.cap, entity.chars_addr],
        exclusions: exclusions_once(mem, module, pointer),
        preference: compact_contract::division_routes::TemplateModelPreference {
            template_id,
            entity: String::from_utf8(entity.bytes).ok()?,
            category,
            excluded_variants: None,
        },
    })
}

fn stable_template_inputs<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> (
    Option<Option<u32>>,
    Option<compact_contract::division_routes::TemplateModelPreference>,
) {
    let Some(first) = template_once(mem, module, body, id) else {
        return (None, None);
    };
    let preference = preference_once(mem, module, &first);
    between();
    let Some(second) = template_once(mem, module, body, id) else {
        return (None, None);
    };
    if first != second {
        return (None, None);
    }
    let after = preference_once(mem, module, &second);
    let preference = match (preference, after) {
        (Some(mut before), Some(after)) if before.string_custody == after.string_custody
            && before.preference == after.preference => {
                if before.exclusions == after.exclusions {
                    before.preference.excluded_variants = before.exclusions.map(|value| value.ids);
                }
                Some(before.preference)
            }
        _ => None,
    };
    (Some(first.template), preference)
}

pub(super) fn current_template_inputs(
    module: usize,
    body: usize,
    id: u32,
) -> (
    Option<Option<u32>>,
    Option<compact_contract::division_routes::TemplateModelPreference>,
) {
    stable_template_inputs(&LIVE_MEMORY, module, body, id, || {})
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct MotionInputs {
    pub ordinary_route: Option<Vec<u32>>,
    pub alternate_route: Option<Vec<u32>>,
    pub route_head: Option<Option<u32>>,
    pub scalars: compact_contract::division_routes::MovementScalars,
}

struct CombatRead {
    pointer: u64,
    identity: Option<compact_contract::division_routes::CombatIdentity>,
}

/// Decode the first combat using the still-open B+1a4 count witness.
fn first_combat_open(
    module: usize,
    body: usize,
    count: u32,
    read: &mut impl FnMut(usize, usize, membership::Field) -> Result<[u8; 8], membership::Refusal>,
) -> Result<CombatRead, membership::Refusal> {
    use compact_contract::division_routes::{CombatClass, CombatIdentity};
    use membership::{Cause, Field, Stage};
    let data = u64::from_le_bytes(read(
        motion_at(body, 0x198, 8, Field::FirstCombatVector)?,
        8,
        Field::FirstCombatVector,
    )?);
    let capacity = motion_u32(read(
        motion_at(body, 0x1a0, 4, Field::FirstCombatVector)?,
        4,
        Field::FirstCombatVector,
    )?);
    if count > capacity || count > MAX_DIVISION_COMBAT_REF_COUNT {
        return Err(motion_refusal(
            Cause::InvalidIdentity,
            Field::FirstCombatVector,
            Stage::Open,
        ));
    }
    if count == 0 {
        return Ok(CombatRead {
            pointer: 0,
            identity: None,
        });
    }
    let data = usize::try_from(data).map_err(|_| {
        motion_refusal(
            Cause::InvalidAddress,
            Field::FirstCombatPointer,
            Stage::Admission,
        )
    })?;
    let pointer = u64::from_le_bytes(read(
        motion_at(data, 0, 8, Field::FirstCombatPointer)?,
        8,
        Field::FirstCombatPointer,
    )?);
    if pointer == 0 {
        return Err(motion_refusal(
            Cause::InvalidIdentity,
            Field::FirstCombatPointer,
            Stage::Open,
        ));
    }
    let primary = usize::try_from(pointer).map_err(|_| {
        motion_refusal(
            Cause::InvalidAddress,
            Field::CombatIdentity,
            Stage::Admission,
        )
    })?;
    let vtables = [
        u64::from_le_bytes(read(
            motion_at(primary, 0, 8, Field::CombatIdentity)?,
            8,
            Field::CombatIdentity,
        )?),
        u64::from_le_bytes(read(
            motion_at(
                primary,
                LAND_COMBAT_MANAGER_BODY_DELTA,
                8,
                Field::CombatIdentity,
            )?,
            8,
            Field::CombatIdentity,
        )?),
    ];
    let image = |rva| {
        module.checked_add(rva).map(|v| v as u64).ok_or_else(|| {
            motion_refusal(
                Cause::AddressOverflow,
                Field::CombatIdentity,
                Stage::Admission,
            )
        })
    };
    let class = if vtables
        == [
            image(LAND_COMBAT_PRIMARY_VTABLE_RVA)?,
            image(LAND_COMBAT_BODY_VTABLE_RVA)?,
        ] {
        CombatClass::Land
    } else if vtables
        == [
            image(LAND_BORDER_WAR_COMBAT_PRIMARY_VTABLE_RVA)?,
            image(LAND_BORDER_WAR_COMBAT_BODY_VTABLE_RVA)?,
        ]
    {
        CombatClass::BorderWar
    } else {
        return Err(motion_refusal(
            Cause::InvalidIdentity,
            Field::CombatIdentity,
            Stage::Open,
        ));
    };
    let kind = motion_u32(read(
        motion_at(
            primary,
            LAND_COMBAT_MANAGER_BODY_DELTA + LAND_COMBAT_OFF_TYPE,
            4,
            Field::CombatIdentity,
        )?,
        4,
        Field::CombatIdentity,
    )?);
    let id = motion_u32(read(
        motion_at(
            primary,
            LAND_COMBAT_MANAGER_BODY_DELTA + LAND_COMBAT_OFF_ID,
            4,
            Field::CombatIdentity,
        )?,
        4,
        Field::CombatIdentity,
    )?);
    if id == 0 || id > i32::MAX as u32 || (class == CombatClass::Land && kind != LAND_COMBAT_TYPE) {
        return Err(motion_refusal(
            Cause::InvalidIdentity,
            Field::CombatIdentity,
            Stage::Open,
        ));
    }
    Ok(CombatRead {
        pointer,
        identity: Some(CombatIdentity { class, kind, id }),
    })
}

/// Expected values from the admitted current division frame. These values do not
/// authenticate campaign/branch identity; close_frame_identity supplies its
/// already-open authority and must drain all its funded closes on every call.
#[derive(Clone, Copy)]
pub(super) struct MotionFrameInput {
    pub game_state: usize,
    pub raw_hour: i32,
}
#[derive(Clone, Copy)]
pub(super) struct MotionDivisionInput {
    pub body: usize,
    pub kind: u32,
    pub id: u32,
    pub location: u32,
    pub combat_count: u32,
}
#[derive(Debug)]
pub(super) struct MotionWithMembership {
    pub motion: MotionInputs,
    /// Primary+1e0 sampled inside this acquisition. The root sampler publishes
    /// this through TrainingScalars.country_operand after sample_without_country.
    pub country_operand: compact_contract::division_routes::TrainingObservation<i32>,
    pub first_combat_countries: membership::FirstCombatCountries,
}
fn motion_country_observation(
    result: Result<i32, membership::Refusal>,
) -> compact_contract::division_routes::TrainingObservation<i32> {
    use compact_contract::division_routes::{TrainingObservation as Obs, TrainingRefusal as R};
    use membership::Cause as C;
    match result {
        Ok(raw) => Obs::Measured(raw),
        Err(reason) => Obs::Unavailable(match reason.cause {
            C::NotObserved => R::NotObserved,
            C::Unreadable | C::HourUnreadable => R::Unreadable,
            C::Changed | C::RootChanged | C::HourChanged => R::Changed,
            C::VerificationBudgetExceeded
            | C::RetainedBudgetExceeded
            | C::AllocationFailed
            | C::AddressOverflow => R::BoundExceeded,
            C::InvalidIdentity | C::InvalidAddress | C::NullSide | C::Cycle => R::InvalidIdentity,
        }),
    }
}
fn motion_refusal(
    cause: membership::Cause,
    field: membership::Field,
    stage: membership::Stage,
) -> membership::Refusal {
    membership::Refusal::at(cause, field, stage, None)
}
fn motion_u32(bytes: [u8; 8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}
fn motion_at(
    base: usize,
    offset: usize,
    width: usize,
    field: membership::Field,
) -> Result<usize, membership::Refusal> {
    use membership::{Cause, Stage};
    let at = base
        .checked_add(offset)
        .ok_or_else(|| motion_refusal(Cause::AddressOverflow, field, Stage::Admission))?;
    let end = at
        .checked_add(width)
        .ok_or_else(|| motion_refusal(Cause::AddressOverflow, field, Stage::Admission))?;
    if !is_map_control_heap_pointer(base) || end > HEAP_HI {
        return Err(motion_refusal(
            Cause::InvalidAddress,
            field,
            Stage::Admission,
        ));
    }
    Ok(at)
}
fn open_motion_route(
    body: usize,
    offset: usize,
    field: membership::Field,
    budget: &country_roster_readset::Budget,
    trace: &mut membership::Trace,
    allocation: &mut membership::Allocation,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<Vec<u32>, membership::Refusal> {
    use membership::{Cause, Stage};
    let data = u64::from_le_bytes(trace.word(
        motion_at(body, offset, 8, field)?,
        8,
        field,
        budget,
        allocation,
        read,
    )?);
    let capacity = motion_u32(trace.word(
        motion_at(body, offset + 8, 4, field)?,
        4,
        field,
        budget,
        allocation,
        read,
    )?);
    let count = motion_u32(trace.word(
        motion_at(body, offset + 12, 4, field)?,
        4,
        field,
        budget,
        allocation,
        read,
    )?) as usize;
    if count > capacity as usize || count > MAX_ROUTE_PROVINCES {
        return Err(motion_refusal(Cause::InvalidIdentity, field, Stage::Open));
    }
    // Count is bounded by the existing route contract, never a new native cap.
    let mut provinces = membership::charged_u32_buffer(count, budget, allocation, field)?;
    if count != 0 {
        let data = usize::try_from(data)
            .map_err(|_| motion_refusal(Cause::InvalidAddress, field, Stage::Admission))?;
        for index in 0..count {
            let offset = index
                .checked_mul(4)
                .ok_or_else(|| motion_refusal(Cause::AddressOverflow, field, Stage::Admission))?;
            let province = motion_u32(trace.word(
                motion_at(data, offset, 4, field)?,
                4,
                field,
                budget,
                allocation,
                read,
            )?);
            if !valid_division_province_id(province) {
                return Err(motion_refusal(Cause::InvalidIdentity, field, Stage::Open));
            }
            // Exact capacity was fully charged before its fallible allocation.
            provinces.push(province);
        }
    }
    Ok(provinces)
}
fn open_motion_head(
    module: usize,
    body: usize,
    budget: &country_roster_readset::Budget,
    trace: &mut membership::Trace,
    allocation: &mut membership::Allocation,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<Option<u32>, membership::Refusal> {
    use membership::{Cause, Field, Stage};
    let field = Field::RouteHead;
    let pointer = u64::from_le_bytes(trace.word(
        motion_at(body, 0x1e0, 8, field)?,
        8,
        field,
        budget,
        allocation,
        read,
    )?);
    if pointer == 0 {
        return Ok(None);
    }
    let pointer = usize::try_from(pointer)
        .map_err(|_| motion_refusal(Cause::InvalidAddress, field, Stage::Admission))?;
    let vptr = u64::from_le_bytes(trace.word(
        motion_at(pointer, 0, 8, field)?,
        8,
        field,
        budget,
        allocation,
        read,
    )?);
    let expected = module
        .checked_add(CPROVINCE_VTABLE_RVA)
        .ok_or_else(|| motion_refusal(Cause::AddressOverflow, field, Stage::Admission))?;
    if vptr != expected as u64 {
        return Err(motion_refusal(Cause::InvalidIdentity, field, Stage::Open));
    }
    let province = motion_u32(trace.word(
        motion_at(pointer, CPROVINCE_OFF_ID, 4, field)?,
        4,
        field,
        budget,
        allocation,
        read,
    )?);
    if !valid_division_province_id(province) {
        return Err(motion_refusal(Cause::InvalidIdentity, field, Stage::Open));
    }
    Ok(Some(province))
}

/// Funded motion and first-combat acquisition used by
/// sample_division_movement_with_training.
/// Every value below comes from one opening traversal and original-address closes.
/// Current raw motion semantics are retained: signed numerator, raw pointer
/// presence, arbitrary retreat byte, route order/duplicates and nullable head.
///
/// Fresh first-combat envelope is 240+24N (112 for empty vector); additional
/// motion witnesses cost 64+8(A+B) for two routes, 16/40 for null/non-null head,
/// and 50 for numerator/transfer/target/retreat; paired country adds 8.
/// Each failed funded open also closes.
/// Caller owns row/encoding allocations and enclosing attachment/campaign/branch
/// custody beyond these root/hour witnesses. No old closed read receives a discount.
pub(super) fn current_motion_inputs_with_membership<R: FnMut(usize, &mut [u8]) -> bool>(
    module: usize,
    frame: MotionFrameInput,
    division: MotionDivisionInput,
    budget: &country_roster_readset::Budget,
    read: &mut R,
    close_frame_identity: impl FnOnce(&mut R) -> Result<(), membership::Refusal>,
) -> MotionWithMembership {
    use membership::{Cause, Field, Stage};
    let mut frame_trace = membership::Trace::default();
    let mut division_trace = membership::Trace::default();
    let mut combat_trace = membership::Trace::default();
    let mut fields: [membership::Trace; 8] = std::array::from_fn(|_| membership::Trace::default());
    let mut allocation = membership::Allocation::default();
    let mut motion = MotionInputs::default();
    let mut country_operand = motion_country_observation(Err(motion_refusal(
        Cause::NotObserved,
        Field::DivisionCountryOperand,
        Stage::Admission,
    )));
    let context = (|| {
        let root = module
            .checked_add(compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
            .ok_or_else(|| {
                motion_refusal(Cause::AddressOverflow, Field::FrameRoot, Stage::Admission)
            })?;
        let game_state = u64::from_le_bytes(frame_trace.word(
            root,
            8,
            Field::FrameRoot,
            budget,
            &mut allocation,
            read,
        )?);
        if game_state != frame.game_state as u64 {
            return Err(motion_refusal(
                Cause::RootChanged,
                Field::FrameRoot,
                Stage::Open,
            ));
        }
        let hour = frame_trace
            .word(
                motion_at(frame.game_state, GS_CURRENT_DATE_OFFSET, 4, Field::Hour)?,
                4,
                Field::Hour,
                budget,
                &mut allocation,
                read,
            )
            .map_err(|mut reason| {
                if reason.cause == Cause::Unreadable {
                    reason.cause = Cause::HourUnreadable;
                }
                reason
            })?;
        let hour = i32::from_le_bytes(hour[..4].try_into().unwrap());
        if hour < TECH_DATE_EPOCH_HOURS as i32 || hour != frame.raw_hour {
            return Err(motion_refusal(Cause::HourChanged, Field::Hour, Stage::Open));
        }
        let primary = division
            .body
            .checked_sub(DIVISION_WRAPPER_TO_BODY)
            .ok_or_else(|| {
                motion_refusal(
                    Cause::AddressOverflow,
                    Field::DivisionIdentity,
                    Stage::Admission,
                )
            })?;
        let mut word = |base: usize, offset: usize, width: usize| {
            division_trace.word(
                motion_at(base, offset, width, Field::DivisionIdentity)?,
                width,
                Field::DivisionIdentity,
                budget,
                &mut allocation,
                read,
            )
        };
        let p = u64::from_le_bytes(word(primary, 0, 8)?);
        let b = u64::from_le_bytes(word(division.body, 0, 8)?);
        let kind = motion_u32(word(division.body, DIVISION_OFF_ID_TYPE, 4)?);
        let id = motion_u32(word(division.body, DIVISION_OFF_ID_ID, 4)?);
        let location = motion_u32(word(division.body, DIVISION_OFF_LOCATION, 4)?);
        let p_vptr = module
            .checked_add(DIVISION_WRAPPER_VTABLE_RVA)
            .ok_or_else(|| {
                motion_refusal(
                    Cause::AddressOverflow,
                    Field::DivisionIdentity,
                    Stage::Admission,
                )
            })?;
        let b_vptr = module.checked_add(DIVISION_VTABLE_RVA).ok_or_else(|| {
            motion_refusal(
                Cause::AddressOverflow,
                Field::DivisionIdentity,
                Stage::Admission,
            )
        })?;
        if p != p_vptr as u64
            || b != b_vptr as u64
            || !is_compact_division_body_type(kind)
            || kind != division.kind
            || id != division.id
            || location != division.location
            || !valid_division_province_id(location)
        {
            return Err(motion_refusal(
                Cause::InvalidIdentity,
                Field::DivisionIdentity,
                Stage::Open,
            ));
        }
        let count = motion_u32(division_trace.word(
            motion_at(
                division.body,
                DIVISION_OFF_COMBAT_REF_COUNT,
                4,
                Field::FirstCombatVector,
            )?,
            4,
            Field::FirstCombatVector,
            budget,
            &mut allocation,
            read,
        )?);
        if count != division.combat_count || count > MAX_DIVISION_COMBAT_REF_COUNT {
            return Err(motion_refusal(
                Cause::InvalidIdentity,
                Field::FirstCombatVector,
                Stage::Open,
            ));
        }
        Ok(count)
    })();
    let first = match context {
        Err(reason) => Err(reason),
        Ok(count) => {
            let primary = division.body - DIVISION_WRAPPER_TO_BODY; // admitted above
            country_operand = motion_country_observation((|| {
                let bytes = fields[7].word(
                    motion_at(primary, 0x1e0, 4, Field::DivisionCountryOperand)?,
                    4,
                    Field::DivisionCountryOperand,
                    budget,
                    &mut allocation,
                    read,
                )?;
                Ok(i32::from_le_bytes(bytes[..4].try_into().unwrap()))
            })());

            motion.ordinary_route = open_motion_route(
                division.body,
                ORDINARY_ROUTE,
                Field::OrdinaryRoute,
                budget,
                &mut fields[0],
                &mut allocation,
                read,
            )
            .ok();
            motion.alternate_route = open_motion_route(
                division.body,
                ALTERNATE_ROUTE,
                Field::AlternateRoute,
                budget,
                &mut fields[1],
                &mut allocation,
                read,
            )
            .ok();
            motion.route_head = open_motion_head(
                module,
                division.body,
                budget,
                &mut fields[2],
                &mut allocation,
                read,
            )
            .ok();
            let mut scalar =
                |index: usize, offset: usize, width: usize, field: membership::Field| {
                    fields[index].word(
                        motion_at(division.body, offset, width, field)?,
                        width,
                        field,
                        budget,
                        &mut allocation,
                        read,
                    )
                };
            motion.scalars.numerator_raw = scalar(3, 0x230, 8, Field::Numerator)
                .map(i64::from_le_bytes)
                .ok();
            motion.scalars.naval_transfer_membership = scalar(4, 0x2e8, 8, Field::Transfer)
                .map(|v| u64::from_le_bytes(v) != 0)
                .ok();
            motion.scalars.movement_target_present = scalar(5, 0x1d8, 8, Field::Target)
                .map(|v| u64::from_le_bytes(v) != 0)
                .ok();
            motion.scalars.retreat_raw = scalar(6, 0x23c, 1, Field::Retreat).map(|v| v[0]).ok();
            first_combat_open(
                module,
                division.body,
                count,
                &mut |at, width, field| {
                    combat_trace.word(at, width, field, budget, &mut allocation, read)
                },
            )
            .map(|combat| {
                combat
                    .identity
                    .map(|identity| membership::FirstCombatInput {
                        primary: combat.pointer as usize,
                        identity,
                    })
            })
        }
    };
    let countries = membership::with_membership(module, first, budget, read, |read| {
        let combat_close = combat_trace.close(read);
        // All sibling obligations drain even if combat or an earlier scalar fails.
        let ordinary = fields[0].close(read);
        let alternate = fields[1].close(read);
        let head = fields[2].close(read);
        let numerator = fields[3].close(read);
        let transfer = fields[4].close(read);
        let target = fields[5].close(read);
        let retreat = fields[6].close(read);
        if ordinary.is_err() {
            motion.ordinary_route = None;
        }
        if alternate.is_err() {
            motion.alternate_route = None;
        }
        if head.is_err() {
            motion.route_head = None;
        }
        if numerator.is_err() {
            motion.scalars.numerator_raw = None;
        }
        if transfer.is_err() {
            motion.scalars.naval_transfer_membership = None;
        }
        if target.is_err() {
            motion.scalars.movement_target_present = None;
        }
        if retreat.is_err() {
            motion.scalars.retreat_raw = None;
        }
        if let Err(reason) = fields[7].close(read) {
            country_operand = motion_country_observation(Err(reason));
        }
        let division_close = division_trace.close(read);
        let frame_close = frame_trace.close(read).map_err(|mut reason| {
            reason.cause = match (reason.field, reason.cause) {
                (Field::FrameRoot, Cause::Changed) => Cause::RootChanged,
                (Field::Hour, Cause::Changed) => Cause::HourChanged,
                (Field::Hour, Cause::Unreadable) => Cause::HourUnreadable,
                _ => reason.cause,
            };
            reason
        });
        // Call even after any reader/allocation/shared-budget refusal.
        let authority_close = close_frame_identity(read);
        let outer = authority_close
            .and(frame_close)
            .and(division_close)
            .and(context.map(|_| ()));
        if let Err(reason) = outer {
            motion = MotionInputs::default();
            country_operand = motion_country_observation(Err(reason));
        }
        outer.and(combat_close)
    });
    motion.scalars.first_combat = match &countries {
        membership::FirstCombatCountries::NoFirstCombat => Some(None),
        membership::FirstCombatCountries::Present { combat, .. } => Some(Some(*combat)),
        membership::FirstCombatCountries::Unavailable(_) => None,
    };
    MotionWithMembership {
        motion,
        country_operand,
        first_combat_countries: countries,
    }
}

#[derive(PartialEq, Eq)]
struct AcclimatizationRead {
    identity: [u64; 4],
    custody: Vec<u64>,
    value: compact_contract::division_routes::Acclimatization,
}

fn acclimatization_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<AcclimatizationRead> {
    use compact_contract::division_routes::{
        Acclimatization, AcclimatizationCategory, MAX_ACCLIMATIZATION_CATEGORIES,
        MAX_MODEL_ENTITY_BYTES,
    };
    let identity = division_identity(mem, module, body, id)?;
    let at = body.checked_add(0x4a8)?;
    if mem.u64(at)? != module.checked_add(0x27c7b30)? as u64 {
        return None;
    }
    let registry = mem.u64(module.checked_add(0x3318080)?)? as usize;
    if !is_map_control_heap_pointer(registry) {
        return None;
    }
    let registered = mem.u64(registry)? as usize;
    let registered_capacity = mem.u32(registry.checked_add(8)?)?;
    let registered_count = mem.u32(registry.checked_add(12)?)?;
    let data = mem.u64(at.checked_add(8)?)? as usize;
    let capacity = mem.u32(at.checked_add(0x10)?)?;
    let count = mem.u32(at.checked_add(0x14)?)?;
    if count != registered_count
        || count > capacity
        || registered_count > registered_capacity
        || count as usize > MAX_ACCLIMATIZATION_CATEGORIES
    {
        return None;
    }
    let maximum_raw = mem.u64(at.checked_add(0x38)?)? as i64;
    let mut custody = vec![
        registry as u64,
        registered as u64,
        registered_capacity as u64,
        registered_count as u64,
        data as u64,
        capacity as u64,
        count as u64,
    ];
    let mut categories = Vec::with_capacity(count as usize);
    if count != 0 {
        if !is_map_control_heap_pointer(registered) || !is_map_control_heap_pointer(data) {
            return None;
        }
        let pointers = mem.bytes(registered, (count as usize).checked_mul(8)?)?;
        let entries = mem.bytes(data, (count as usize).checked_mul(16)?)?;
        let mut seen = BTreeSet::new();
        for (registered, entry) in pointers.chunks_exact(8).zip(entries.chunks_exact(16)) {
            let pointer = u64::from_le_bytes(entry[..8].try_into().ok()?) as usize;
            if pointer as u64 != u64::from_le_bytes(registered.try_into().ok()?)
                || !is_map_control_heap_pointer(pointer)
            {
                return None;
            }
            let name = read_msvc_string_snapshot_once_in(mem, pointer, MAX_MODEL_ENTITY_BYTES)?;
            let token = mem.u32(pointer.checked_add(0x20)?)?;
            let category = String::from_utf8(name.bytes).ok()?;
            if category.is_empty() || !seen.insert(category.clone()) {
                return None;
            }
            custody.extend([
                pointer as u64,
                name.size as u64,
                name.cap as u64,
                name.chars_addr as u64,
                token as u64,
            ]);
            categories.push(AcclimatizationCategory {
                category,
                weight_raw: i64::from_le_bytes(entry[8..].try_into().ok()?),
            });
        }
    }
    Some(AcclimatizationRead {
        identity,
        custody,
        value: Acclimatization {
            maximum_raw,
            categories,
        },
    })
}

fn stable_acclimatization<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> Option<compact_contract::division_routes::Acclimatization> {
    let first = acclimatization_once(mem, module, body, id)?;
    between();
    let second = acclimatization_once(mem, module, body, id)?;
    (first == second).then_some(first.value)
}

pub(super) fn acclimatization(
    module: usize,
    body: usize,
    id: u32,
) -> Option<compact_contract::division_routes::Acclimatization> {
    stable_acclimatization(&LIVE_MEMORY, module, body, id, || {})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Memory(RefCell<BTreeMap<usize, u8>>, std::cell::Cell<u64>);
    impl Memory {
        fn put(&self, at: usize, bytes: &[u8]) {
            self.0
                .borrow_mut()
                .extend(bytes.iter().enumerate().map(|(i, b)| (at + i, *b)));
        }
        fn put32(&self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn put64(&self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn vector(&self, body: usize, offset: usize, data: usize, provinces: &[u32]) {
            self.put64(body + offset, data as u64);
            self.put32(body + offset + 8, provinces.len() as u32);
            self.put32(body + offset + 12, provinces.len() as u32);
            for (index, province) in provinces.iter().enumerate() {
                self.put32(data + index * 4, *province);
            }
        }
    }
    impl DecodeMemory for Memory {
        fn scalar<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
            self.bytes(at, N)?.try_into().ok()
        }
        fn bytes_with_reason(&self, at: usize, len: usize) -> Result<Vec<u8>, GuardedReadRefusal> {
            (at..at + len)
                .map(|key| {
                    self.0
                        .borrow()
                        .get(&key)
                        .copied()
                        .ok_or(GuardedReadRefusal::OutsideConfirmedRanges)
                })
                .collect()
        }
        fn fill(&self, at: usize, buf: &mut [u8]) -> bool {
            let Some(bytes) = self.bytes(at, buf.len()) else {
                return false;
            };
            buf.copy_from_slice(&bytes);
            true
        }
    }
    fn fixture() -> (Memory, usize, usize) {
        let mem = Memory::default();
        let module = 0x140000000;
        let body = HEAP_LO + 0x10000;
        mem.put64(body - DIVISION_WRAPPER_TO_BODY, (module + DIVISION_WRAPPER_VTABLE_RVA) as u64);
        mem.put64(body, (module + DIVISION_VTABLE_RVA) as u64);
        let game_state = body + 0xd0000;
        mem.put64(module + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA, game_state as u64);
        mem.put32(game_state + GS_CURRENT_DATE_OFFSET, TECH_DATE_EPOCH_HOURS as u32 + 100);
        mem.put32(body - DIVISION_WRAPPER_TO_BODY + 0x1e0, i32::MIN as u32);
        mem.put32(body + DIVISION_OFF_ID_TYPE, DIVISION_TYPE);
        mem.put32(body + DIVISION_OFF_ID_ID, 7);
        mem.put32(body + DIVISION_OFF_LOCATION, 12);
        mem.vector(body, ORDINARY_ROUTE, body + 0x10000, &[19, 17, 19]);
        mem.vector(body, ALTERNATE_ROUTE, 0, &[]);
        mem.put64(body + 0x198, 0); mem.put32(body + 0x1a0, 0); mem.put32(body + 0x1a4, 0);
        mem.put64(body + 0x230, 0); mem.put64(body + 0x2e8, 0); mem.put64(body + 0x1d8, 0);
        mem.put(body + 0x23c, &[0]);
        (mem, module, body)
    }
    fn capture_motion(
        mem: &Memory,
        module: usize,
        body: usize,
        id: u32,
        location: u32,
        combat_count: u32,
        between: impl FnOnce(),
    ) -> MotionInputs {
        let mut visits = BTreeMap::new();
        let mut between = Some(between);
        let opened_generation = mem.1.get();
        let mut authority_closed = false;
        let result = current_motion_inputs_with_membership(
            module,
            MotionFrameInput {
                game_state: body + 0xd0000,
                raw_hour: TECH_DATE_EPOCH_HOURS as i32 + 100,
            },
            MotionDivisionInput { body, kind: DIVISION_TYPE, id, location, combat_count },
            &country_roster_readset::Budget::default(),
            &mut |at, out| {
                let visit = visits.entry(at).or_insert(0);
                *visit += 1;
                if *visit == 2 {
                    if let Some(between) = between.take() {
                        between();
                    }
                }
                mem.fill(at, out)
            },
            |_| {
                // The fixture's generation is outside the native address layout.
                authority_closed = true;
                if mem.1.get() == opened_generation {
                    Ok(())
                } else {
                    Err(motion_refusal(membership::Cause::Changed,
                        membership::Field::FrameRoot, membership::Stage::Close))
                }
            },
        );
        assert!(authority_closed);
        result.motion
    }

    #[test]
    fn route_head_uses_province_pointer_and_preserves_unknown_null_and_changed() {
        let (mem, module, body) = fixture();
        let province = body + 0x50000;
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 0, || {}).route_head, None);
        mem.put64(body + 0x1e0, 0);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 0, || {}).route_head, Some(None));
        mem.put64(body + 0x1e0, province as u64);
        mem.put64(province, (module + CPROVINCE_VTABLE_RVA) as u64);
        mem.put32(province + CPROVINCE_OFF_ID, 23);
        mem.put32(body + DIVISION_OFF_LOCATION, 12);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 0, || {}).route_head, Some(Some(23)));
        let input = capture_motion(&mem, module, body, 7, 12, 0, || {});
        assert_eq!(input.ordinary_route, Some(vec![19, 17, 19]));
        assert_eq!(input.route_head, Some(Some(23)));
        let changed = capture_motion(&mem, module, body, 7, 12, 0, || mem.put32(body + 0x10000, 18));
        assert_eq!(changed.ordinary_route, None);
        assert_eq!(changed.route_head, Some(Some(23)));
        mem.put32(body + 0x10000, 19);
        assert_eq!(
            capture_motion(&mem, module, body, 7, 12, 0, || mem
                .put32(province + CPROVINCE_OFF_ID, 24)).route_head,
            None
        );
        mem.put64(province, 0);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 0, || {}).route_head, None);
        mem.put64(province, (module + CPROVINCE_VTABLE_RVA) as u64);
        mem.put32(province + CPROVINCE_OFF_ID, 0);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 0, || {}).route_head, None);
        mem.put32(province + CPROVINCE_OFF_ID, 23);
        assert_eq!(
            capture_motion(&mem, module, body, 7, 12, 0, || mem
                .put32(body + DIVISION_OFF_ID_ID, 8)).route_head,
            None
        );
    }

    #[test]
    fn template_preferences_bind_current_identity_and_preserve_empty_optional_and_torn_values() {
        let (mem, module, body) = fixture();
        let template = body + 0x30000;
        mem.put64(body + 0x3b8, template as u64);
        mem.put64(template, (module + DIVISION_TEMPLATE_VTABLE_RVA) as u64);
        mem.put32(
            template + DIVISION_TEMPLATE_OFF_TYPE,
            DIVISION_TEMPLATE_TYPE,
        );
        mem.put32(template + DIVISION_TEMPLATE_OFF_ID, 73);
        mem.put(template + 0x210, b"tank_entity\0");
        mem.put64(template + 0x220, 11);
        mem.put64(template + 0x228, 15);
        mem.put(template + 0x208, &[0]);
        let (id, preference) = stable_template_inputs(&mem, module, body, 7, || {});
        assert_eq!(id, Some(Some(73)));
        let preference = preference.unwrap();
        assert_eq!(preference.template_id, 73);
        assert_eq!(preference.entity, "tank_entity");
        assert_eq!(preference.category, None);
        mem.put(template + 0x208, &[1]);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || {}),
            (Some(Some(73)), None)
        );
        mem.put64(template + 0x200, 0);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || {})
                .1
                .unwrap()
                .category,
            Some(0)
        );
        mem.put(template + 0x210, b"\0");
        mem.put64(template + 0x220, 0);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || {})
                .1
                .unwrap()
                .entity,
            ""
        );
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || mem.put64(template + 0x200, 9)),
            (Some(Some(73)), None)
        );
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || mem
                .put32(template + DIVISION_TEMPLATE_OFF_ID, 74)),
            (None, None)
        );
        mem.put32(template + DIVISION_TEMPLATE_OFF_ID, 73);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || mem.put64(body + 0x3b8, 0)),
            (None, None)
        );
    }

    #[test]
    fn template_exclusions_preserve_unknown_empty_null_order_and_torn_variant_identity() {
        let (mem, module, body) = fixture();
        let template = body + 0x30000;
        let list = body + 0x40000;
        let variant = body + 0x50000;
        mem.put64(body + 0x3b8, template as u64);
        mem.put64(template, (module + DIVISION_TEMPLATE_VTABLE_RVA) as u64);
        mem.put32(template + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
        mem.put32(template + DIVISION_TEMPLATE_OFF_ID, 73);
        mem.put(template + 0x210, &[0]);
        mem.put64(template + 0x220, 0);
        mem.put64(template + 0x228, 15);
        mem.put(template + 0x208, &[1]);
        mem.put64(template + 0x200, u64::MAX);
        let read = || stable_template_inputs(&mem, module, body, 7, || {}).1.unwrap();
        assert_eq!(read().excluded_variants, None);
        mem.put64(template + 0x140, 0);
        mem.put32(template + 0x148, 0);
        mem.put32(template + 0x14c, 0);
        assert_eq!(read().excluded_variants, Some(vec![]));
        mem.put64(template + 0x140, list as u64);
        mem.put32(template + 0x148, 3);
        mem.put32(template + 0x14c, 3);
        mem.put64(list, variant as u64);
        mem.put64(list + 8, 0);
        mem.put64(list + 16, variant as u64);
        mem.put64(variant, (module + DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA) as u64);
        mem.put32(variant + DIVISION_EQUIPMENT_VARIANT_OFF_TYPE, DIVISION_EQUIPMENT_VARIANT_TYPE);
        mem.put32(variant + DIVISION_EQUIPMENT_VARIANT_OFF_ID, 11);
        assert_eq!(read().excluded_variants, Some(vec![Some(11), None, Some(11)]));
        let torn = stable_template_inputs(&mem, module, body, 7, || mem.put32(variant + DIVISION_EQUIPMENT_VARIANT_OFF_ID, 12)).1.unwrap();
        assert_eq!(torn.excluded_variants, None);
        assert_eq!(torn.category, Some(u64::MAX));
        assert!(torn.entity.is_empty());
        mem.put32(template + 0x14c, 4);
        assert_eq!(read().excluded_variants, None);
        mem.put32(template + 0x14c, 3);
        mem.put64(variant, 0);
        assert_eq!(read().excluded_variants, None);
    }

    #[test]
    fn acclimatization_census_preserves_registered_order_raw_values_empty_and_torn_refusal() {
        let (mem, module, body) = fixture();
        let at = body + 0x4a8;
        let registry = body + 0x60000;
        let registered = body + 0x70000;
        let data = body + 0x80000;
        let cold = body + 0x90000;
        let hot = body + 0xa0000;
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
        mem.put64(at, (module + 0x27c7b30) as u64);
        mem.put64(module + 0x3318080, registry as u64);
        mem.put64(registry, 0);
        mem.put32(registry + 8, 0);
        mem.put32(registry + 12, 0);
        mem.put64(at + 8, 0);
        mem.put32(at + 0x10, 0);
        mem.put32(at + 0x14, 0);
        mem.put64(at + 0x38, 10_000_000);
        let empty = stable_acclimatization(&mem, module, body, 7, || {}).unwrap();
        assert_eq!(empty.maximum_raw, 10_000_000);
        assert!(empty.categories.is_empty());
        mem.put64(registry, registered as u64);
        mem.put32(registry + 8, 2);
        mem.put32(registry + 12, 2);
        mem.put64(at + 8, data as u64);
        mem.put32(at + 0x10, 2);
        mem.put32(at + 0x14, 2);
        for (index, (pointer, name, weight)) in [(cold, "cold", -1i64), (hot, "hot", 123_456i64)]
            .into_iter()
            .enumerate()
        {
            mem.put64(registered + index * 8, pointer as u64);
            mem.put64(data + index * 16, pointer as u64);
            mem.put64(data + index * 16 + 8, weight as u64);
            mem.put(pointer, format!("{name}\0").as_bytes());
            mem.put64(pointer + 0x10, name.len() as u64);
            mem.put64(pointer + 0x18, 15);
            mem.put32(pointer + 0x20, index as u32 + 100);
        }
        let value = stable_acclimatization(&mem, module, body, 7, || {}).unwrap();
        assert_eq!(
            value
                .categories
                .iter()
                .map(|v| (v.category.as_str(), v.weight_raw))
                .collect::<Vec<_>>(),
            vec![("cold", -1), ("hot", 123_456)]
        );
        assert_eq!(
            stable_acclimatization(&mem, module, body, 7, || mem.put64(data + 8, 0)),
            None
        );
        assert_eq!(
            stable_acclimatization(&mem, module, body, 7, || mem.put(cold, b"warm\0")),
            None
        );
        mem.put64(data, hot as u64);
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
        mem.put64(data, cold as u64);
        mem.put32(at + 0x14, 1);
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
        mem.put32(at + 0x14, 2);
        mem.put64(at, 0);
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
    }

    #[test]
    fn current_template_uses_only_witnessed_null_fallback_and_preserves_absence() {
        let (mem, module, body) = fixture();
        let standard = body + 0x30000;
        let current = body + 0x40000;
        for (at, id) in [(standard, 61), (current, 72)] {
            mem.put64(at, (module + DIVISION_TEMPLATE_VTABLE_RVA) as u64);
            mem.put32(at + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
            mem.put32(at + DIVISION_TEMPLATE_OFF_ID, id);
        }
        mem.put64(body + DIVISION_OFF_TEMPLATE, standard as u64);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), None);
        mem.put64(body + 0x3b8, 0);
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(61))
        );
        mem.put64(body + 0x3b8, current as u64);
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(72))
        );
        mem.0.borrow_mut().remove(&(body + DIVISION_OFF_TEMPLATE));
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(72))
        );
        mem.put32(current + DIVISION_TEMPLATE_OFF_TYPE, 70);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), None);
        mem.put32(current + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
        assert_eq!(
            stable_template(&mem, module, body, 7, || mem
                .put32(current + DIVISION_TEMPLATE_OFF_ID, 73)),
            None
        );
        mem.put64(body + 0x3b8, 0);
        mem.put64(body + DIVISION_OFF_TEMPLATE, 0);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), Some(None));
        assert_eq!(
            stable_template(&mem, module, body, 7, || mem
                .put32(body + DIVISION_OFF_ID_ID, 8)),
            None
        );
    }

    #[test]
    fn both_routes_preserve_native_order_and_independent_availability() {
        let (mem, module, body) = fixture();
        let read = || capture_motion(&mem, module, body, 7, 12, 0, || {});
        assert_eq!(read().ordinary_route, Some(vec![19, 17, 19]));
        assert_eq!(read().alternate_route, Some(vec![]));
        mem.vector(body, ALTERNATE_ROUTE, body + 0x20000, &[22, 21]);
        assert_eq!(read().ordinary_route, Some(vec![19, 17, 19]));
        assert_eq!(read().alternate_route, Some(vec![22, 21]));
        mem.0.borrow_mut().remove(&(body + ORDINARY_ROUTE));
        assert_eq!(read().ordinary_route, None);
        assert_eq!(read().alternate_route, Some(vec![22, 21]));
        mem.0.borrow_mut().remove(&(body + ALTERNATE_ROUTE + 12));
        assert_eq!(read().alternate_route, None);
    }

    #[test]
    fn grouped_motion_preserves_zero_signed_width_and_nullable_pointer_changes() {
        let (mem, module, body) = fixture();
        let read = || capture_motion(&mem, module, body, 7, 12, 0, || {});
        assert_eq!(read().scalars.numerator_raw, Some(0));
        assert_eq!(read().scalars.naval_transfer_membership, Some(false));
        assert_eq!(read().scalars.movement_target_present, Some(false));
        assert_eq!(read().scalars.retreat_raw, Some(0));
        assert_eq!(read().scalars.first_combat, Some(None));
        for raw in [i64::MIN, i64::MAX, -1, 0x1_0000_0000] {
            mem.put64(body + 0x230, raw as u64);
            assert_eq!(read().scalars.numerator_raw, Some(raw));
        }
        mem.put(body + 0x23c, &[255]);
        assert_eq!(read().scalars.retreat_raw, Some(255));
        for offset in [0x2e8, 0x1d8] {
            mem.put64(body + offset, (body + 0x90000) as u64);
        }
        assert_eq!(read().scalars.naval_transfer_membership, Some(true));
        assert_eq!(read().scalars.movement_target_present, Some(true));
        let torn = capture_motion(&mem, module, body, 7, 12, 0, || {
            mem.put64(body + 0x2e8, (body + 0xa0000) as u64);
            mem.put64(body + 0x1d8, (body + 0xb0000) as u64);
            mem.put64(body + 0x230, 123);
        });
        assert_eq!(torn.scalars.naval_transfer_membership, None);
        assert_eq!(torn.scalars.movement_target_present, None);
        assert_eq!(torn.scalars.numerator_raw, None);
        assert_eq!(torn.scalars.retreat_raw, Some(255));
        mem.0.borrow_mut().remove(&(body + 0x230 + 7));
        assert_eq!(read().scalars.numerator_raw, None);
        let changed = capture_motion(&mem, module, body, 7, 12, 0, || mem.put32(body + DIVISION_OFF_ID_ID, 8));
        assert_eq!(changed, MotionInputs::default());
    }

    #[test]
    fn first_combat_preserves_native_first_identity_and_refuses_reorder_or_reuse() {
        use compact_contract::division_routes::{CombatClass, CombatIdentity};
        let (mem, module, body) = fixture();
        let data = body + 0x60000;
        let first = body + 0x70000;
        let second = body + 0x80000;
        mem.put64(body + 0x198, data as u64); mem.put32(body + 0x1a0, 2); mem.put32(body + 0x1a4, 2);
        mem.put64(data, first as u64); mem.put64(data + 8, second as u64);
        for (pointer, id) in [(first, 91), (second, 92)] {
            mem.put64(pointer, (module + LAND_COMBAT_PRIMARY_VTABLE_RVA) as u64);
            mem.put64(pointer + 0x10, (module + LAND_COMBAT_BODY_VTABLE_RVA) as u64);
            mem.put32(pointer + 0x18, LAND_COMBAT_TYPE); mem.put32(pointer + 0x1c, id);
        }
        let expected = Some(Some(CombatIdentity { class: CombatClass::Land, kind: 62, id: 91 }));
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 2, || {}).scalars.first_combat, expected);
        let changed = capture_motion(&mem, module, body, 7, 12, 2, || {
            mem.put64(data, second as u64); mem.put64(data + 8, first as u64);
        });
        assert_eq!(changed.scalars.first_combat, None);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 2, || {}).scalars.first_combat.unwrap().unwrap().id, 92);
        let changed = capture_motion(&mem, module, body, 7, 12, 2, || mem.put32(second + 0x1c, 93));
        assert_eq!(changed.scalars.first_combat, None);
        mem.put64(second, 0);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 2, || {}).scalars.first_combat, None);
    }

    #[test]
    fn both_route_pools_reject_invalid_or_changed_bytes_without_dropping_other_scalars() {
        let (mem, module, body) = fixture();
        let changed = capture_motion(&mem, module, body, 7, 12, 0, || mem.put32(body + 0x10000, 20));
        assert_eq!(changed.ordinary_route, None);
        assert_eq!(changed.scalars.numerator_raw, Some(0));
        mem.vector(body, ORDINARY_ROUTE, body + 0x10000, &[0]);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 0, || {}).ordinary_route, None);
        mem.put32(body + ALTERNATE_ROUTE + 12, MAX_ROUTE_PROVINCES as u32 + 1);
        assert_eq!(capture_motion(&mem, module, body, 7, 12, 0, || {}).alternate_route, None);
    }
    mod membership_adapter_tests {
        use super::*;
        use compact_contract::division_routes::CombatClass;
        use membership::{Cause, Field, FirstCombatCountries};
        use std::cell::Cell;

        struct AdapterFixture {
            mem: Memory,
            module: usize,
            division: MotionDivisionInput,
            frame: MotionFrameInput,
            vector: usize,
            combat: usize,
            side: usize,
            nodes: usize,
            generation: Cell<u64>,
        }
        struct Reads {
            visits: BTreeMap<usize, usize>,
            events: Vec<(usize, usize)>,
            authority_closes: usize,
        }
        impl Reads {
            fn all_closed(&self) {
                for (at, count) in &self.visits {
                    assert_eq!(*count, 2, "expected one open/one original close at {at:x}");
                }
                assert_eq!(self.authority_closes, 1);
            }
            fn bytes(&self) -> usize {
                self.events.iter().map(|(_, width)| width).sum()
            }
        }
        impl AdapterFixture {
            fn new(class: CombatClass, countries: &[i32]) -> Self {
                // Reuse this module's actual presentation Memory/fixture.
                let (mem, module, body) = fixture();
                let game_state = body + 0xd0000;
                let vector = body + 0x90000;
                let combat = body + 0xa0000;
                let side = body + 0xb0000;
                let nodes = body + 0xc0000;
                mem.put64(
                    body - DIVISION_WRAPPER_TO_BODY,
                    (module + DIVISION_WRAPPER_VTABLE_RVA) as u64,
                );
                mem.put64(
                    module + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
                    game_state as u64,
                );
                let raw_hour = TECH_DATE_EPOCH_HOURS as i32 + 100;
                mem.put32(game_state + GS_CURRENT_DATE_OFFSET, raw_hour as u32);
                mem.put32(body - DIVISION_WRAPPER_TO_BODY + 0x1e0, i32::MIN as u32);
                mem.put64(body + 0x1e0, 0); // Nullable route head, not primary country+1e0.
                mem.put64(body + 0x198, vector as u64);
                mem.put32(body + 0x1a0, 2);
                mem.put32(body + DIVISION_OFF_COMBAT_REF_COUNT, 1);
                mem.put64(vector, combat as u64);
                let (primary_vptr, secondary_vptr, side_vptr, kind) = match class {
                    CombatClass::Land => (
                        LAND_COMBAT_PRIMARY_VTABLE_RVA,
                        LAND_COMBAT_BODY_VTABLE_RVA,
                        0x29913c0,
                        62,
                    ),
                    CombatClass::BorderWar => (
                        LAND_BORDER_WAR_COMBAT_PRIMARY_VTABLE_RVA,
                        LAND_BORDER_WAR_COMBAT_BODY_VTABLE_RVA,
                        0x29a5818,
                        999,
                    ),
                };
                mem.put64(combat, (module + primary_vptr) as u64);
                mem.put64(combat + 0x10, (module + secondary_vptr) as u64);
                mem.put32(combat + 0x18, kind);
                mem.put32(combat + 0x1c, 79);
                mem.put64(combat + 0x28, side as u64);
                mem.put64(side, (module + side_vptr) as u64);
                mem.put64(side + 0x18, combat as u64);
                mem.put64(
                    side + 0x38,
                    if countries.is_empty() {
                        0
                    } else {
                        nodes as u64
                    },
                );
                for (i, &country) in countries.iter().enumerate() {
                    mem.put32(nodes + i * 0x20, country as u32);
                    mem.put64(
                        nodes + i * 0x20 + 16,
                        if i + 1 == countries.len() {
                            0
                        } else {
                            (nodes + (i + 1) * 0x20) as u64
                        },
                    );
                }
                Self {
                    mem,
                    module,
                    division: MotionDivisionInput {
                        body,
                        kind: DIVISION_TYPE,
                        id: 7,
                        location: 12,
                        combat_count: 1,
                    },
                    frame: MotionFrameInput {
                        game_state,
                        raw_hour,
                    },
                    vector,
                    combat,
                    side,
                    nodes,
                    generation: Cell::new(1),
                }
            }
            fn run(
                &self,
                budget: &country_roster_readset::Budget,
                mut before: impl FnMut(&Self, usize, usize) -> bool,
            ) -> (MotionWithMembership, Reads) {
                let mut log = Reads {
                    visits: BTreeMap::new(),
                    events: Vec::new(),
                    authority_closes: 0,
                };
                // A fixture generation authority is deliberately outside native
                // address layout. The root sampler supplies its opened frame authority.
                let opened_generation = self.generation.get();
                let result = current_motion_inputs_with_membership(
                    self.module,
                    self.frame,
                    self.division,
                    budget,
                    &mut |at, out| {
                        let visit = log.visits.entry(at).or_default();
                        *visit += 1;
                        log.events.push((at, out.len()));
                        if !before(self, at, *visit) {
                            return false;
                        }
                        self.mem.fill(at, out)
                    },
                    |_| {
                        log.authority_closes += 1;
                        if self.generation.get() == opened_generation {
                            Ok(())
                        } else {
                            Err(motion_refusal(
                                Cause::Changed,
                                Field::FrameRoot,
                                membership::Stage::Close,
                            ))
                        }
                    },
                );
                (result, log)
            }
        }
        #[test]
        #[ignore = "exports hash-bound actual adapter/publisher/fold membership stages"]
        fn actual_sampler_publisher_fold_membership_stages() {
            use crate::attach_log::{division_membership_portable, province_occupants::tests::stage};
            use compact_contract::division_routes::{self as wire, TrainingObservation as Obs};
            use compact_contract::domain_gap::{
                DomainGapPart, DomainGapReason, DomainGapRecordV1, DomainGapWitnessProvenance,
            };
            use std::io::Write;

            const HOUR: i32 = DATE_EPOCH_CONST + 16_959_367;
            const COMBAT: wire::CombatIdentity = wire::CombatIdentity {
                class: CombatClass::Land,
                kind: 62,
                id: 79,
            };
            fn expected_membership(unavailable: bool) -> wire::FirstCombatCountriesObservation {
                wire::FirstCombatCountriesObservation::Present {
                    combat: COMBAT,
                    countries: if unavailable {
                        wire::CountryWordsObservation::Unavailable(wire::CombatMembershipRefusal {
                            cause: wire::CombatMembershipCause::Unreadable,
                            field: wire::CombatMembershipField::Next,
                            stage: wire::CombatMembershipStage::Open,
                            ordinal: Some(0),
                            original_ordinal: None,
                        })
                    } else {
                        wire::CountryWordsObservation::Measured(vec![i32::MAX, i32::MIN, 0, i32::MIN])
                    },
                }
            }
            fn emit(
                f: &AdapterFixture,
                tracker: &mut FrameJournalled<BTreeMap<u32, DivisionMovementSample>>,
                unavailable: bool,
                expected_rows: u32,
                delta: bool,
            ) -> Vec<u8> {
                // One budget owns acquisition, conversion, journal copies and encoding.
                let budget = country_roster_readset::Budget::default();
                let (result, reads) = f.run(&budget, |f, at, visit| {
                    !(unavailable && at == f.nodes + 16 && visit == 1)
                });
                assert_eq!(reads.bytes(), budget.verification_bytes());
                reads.all_closed();
                assert_eq!(result.country_operand, Obs::Measured(i32::MIN));
                assert_eq!(result.motion.scalars.first_combat, Some(Some(COMBAT)));
                let expected_scalars = result.motion.scalars;
                let expected = expected_membership(unavailable);
                let mut row = deleted_division_sample(f.division.id, f.division.kind);
                row.deleted = false;
                row.location = f.division.location;
                row.raw_max_organisation = Some(100_000);
                row.combat_ref_count = f.division.combat_count;
                row.in_combat = row.combat_ref_count != 0;
                row.moving = result
                    .motion
                    .scalars
                    .numerator_raw
                    .is_some_and(|raw| raw != 0);
                row.ordinary_route = result.motion.ordinary_route;
                row.alternate_route = result.motion.alternate_route;
                row.route_head = result.motion.route_head;
                row.movement_scalars = result.motion.scalars;
                row.training_scalars.country_operand = result.country_operand;
                row.first_combat_countries =
                    division_membership_portable::observation(result.first_combat_countries, &budget);
                assert_eq!(row.first_combat_countries, expected);
                let (rows, changed, flags) =
                    compact_division_emitted_rows(vec![row], 1, 0, tracker, &budget).unwrap();
                let expected_flags = if delta { COMPACT_STATUS_DELTA_DATA } else { 0 };
                assert_eq!(rows.len(), expected_rows as usize);
                assert_eq!(changed, expected_rows);
                assert_eq!(flags, expected_flags);
                let bytes =
                    compact_divisions_data_payload(&rows, 0, flags, changed, 1, None, None, &budget)
                        .unwrap();
                for (at, expected) in [
                    (8, expected_rows),
                    (12, expected_rows),
                    (16, expected_flags),
                    (24, 1),
                ] {
                    assert_eq!(
                        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                        expected
                    );
                }
                let parsed = wire::DivisionRoutes::parse(&bytes).expect("actual publisher payload");
                if expected_rows == 1 {
                    assert_eq!(parsed.movement_scalars(0), expected_scalars);
                    assert_eq!(
                        parsed.training_scalars(0).country_operand,
                        Obs::Measured(i32::MIN)
                    );
                    assert_eq!(parsed.first_combat_countries(0).unwrap(), expected);
                    assert_ne!(
                        u32::from_le_bytes(bytes[40..44].try_into().unwrap()) & (1 << 4),
                        0
                    );
                }
                bytes
            }
            fn append_stage(
                stream: &mut Vec<u8>,
                sequence: &mut u64,
                hour: i32,
                record_type: u16,
                payload: &[u8],
                name: &str,
            ) -> serde_json::Value {
                append_compact_record(stream, *sequence, hour, 20, record_type, 0, payload);
                *sequence += 1;
                let mut value = stage(stream, *sequence, hour);
                value
                    .as_object_mut()
                    .unwrap()
                    .insert("name".into(), name.into());
                value
            }
            fn close_hour(stream: &mut Vec<u8>, sequence: &mut u64, hour: i32) {
                // stage() seals only its copy. Persist the real boundary before advancing time.
                append_compact_record(
                    stream,
                    *sequence,
                    hour,
                    compact_contract::COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_END,
                    0,
                    &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN],
                );
                *sequence += 1;
            }

            let destination = std::path::PathBuf::from(
                std::env::var_os("BATTLE_PLAN_D2RA_MEMBERSHIP_JOINT_OUTPUT").or_else(|| std::env::var_os("HOI4_D2RA_MEMBERSHIP_JOINT_OUTPUT"))
                    .expect("exclusive membership output required"),
            );
            assert!(destination.is_absolute());
            let mut f = AdapterFixture::new(CombatClass::Land, &[i32::MAX, i32::MIN, 0, i32::MIN]);
            f.frame.raw_hour = HOUR;
            f.mem
                .put32(f.frame.game_state + GS_CURRENT_DATE_OFFSET, HOUR as u32);
            let mut tracker = FrameJournalled::default();
            let mut stream = Vec::new();
            let mut sequence = 1;
            let mut stages = Vec::new();
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &emit(&f, &mut tracker, false, 1, false),
                "measured",
            ));
            tracker.close_frame_transaction();
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &emit(&f, &mut tracker, true, 1, true),
                "unavailable_same_hour",
            ));
            tracker.close_frame_transaction();

            let gap = DomainGapRecordV1 {
                part: DomainGapPart::State,
                reason: DomainGapReason::SourceUnavailable,
                target_hour_raw: HOUR,
                range_start_hour_raw: HOUR,
                range_end_hour_raw: HOUR,
                tick_before: 0,
                tick_after: 0,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: sequence,
            }
            .encode_with_witness_provenance(DomainGapWitnessProvenance::Unobserved)
            .unwrap();
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
                &gap,
                "gap_same_hour",
            ));
            // A published source gap clears the producer's baseline and requires a full row.
            tracker.advance(BTreeMap::new());
            tracker.close_frame_transaction();
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &emit(&f, &mut tracker, false, 1, false),
                "full_same_hour_after_gap",
            ));
            tracker.close_frame_transaction();
            close_hour(&mut stream, &mut sequence, HOUR);

            f.frame.raw_hour = HOUR + 1;
            f.mem.put32(
                f.frame.game_state + GS_CURRENT_DATE_OFFSET,
                (HOUR + 1) as u32,
            );
            tracker = FrameJournalled::default();
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR + 1,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &emit(&f, &mut tracker, false, 1, false),
                "recovery_next_hour",
            ));
            tracker.close_frame_transaction();
            close_hour(&mut stream, &mut sequence, HOUR + 1);

            f.frame.raw_hour = HOUR + 2;
            f.mem.put32(
                f.frame.game_state + GS_CURRENT_DATE_OFFSET,
                (HOUR + 2) as u32,
            );
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR + 2,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &emit(&f, &mut tracker, false, 0, true),
                "empty_delta_after_recovery",
            ));
            tracker.close_frame_transaction();
            f.mem.put64(f.division.body + 0x230, 1);
            let mut hostile = emit(&f, &mut tracker, false, 1, true);
            // Remove the fixed-row identity, leaving the measured membership descriptor intact.
            // Changing only the id would be valid: D2RA stores the identity once in the row.
            hostile[32 + 204..32 + 216].fill(0);
            assert_eq!(
                wire::DivisionRoutes::parse(&hostile).err().as_deref(),
                Some("combat membership identity or ordered pool custody invalid")
            );
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR + 2,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &hostile,
                "malformed_pair",
            ));
            // Receiver rejection leaves the sender's baseline advanced; this is not sink rollback.
            tracker.close_frame_transaction();
            stages.push(append_stage(
                &mut stream,
                &mut sequence,
                HOUR + 2,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &emit(&f, &mut tracker, false, 0, true),
                "empty_after_malformed",
            ));

            for (index, value) in stages.iter().enumerate() {
                let snapshot = &value["snapshot"];
                let problem = match index {
                    2 | 3 => Some("domain_gap"),
                    6 | 7 => Some("malformed_payload"),
                    _ => None,
                };
                if let Some(problem) = problem {
                    assert!(snapshot["divisions"].is_null());
                    assert!(snapshot["provenance"]["collector"]["domain_problems"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|p| p["domain_id"] == 20 && p["problem"] == problem));
                } else {
                    let row = &snapshot["divisions"][0];
                    assert_eq!(row["div_id"], 7);
                    assert_eq!(row["div_type"], 4713);
                    assert_eq!(
                        row["first_combat"],
                        serde_json::json!({"status":"measured","class":"land","type":62,"id":79})
                    );
                    assert_eq!(
                        row["training_scalars"]["country_operand"],
                        serde_json::json!({"status":"measured","raw_i32":i32::MIN})
                    );
                    assert_eq!(
                        row["first_combat_countries"],
                        serde_json::from_str::<serde_json::Value>(&expected_membership(index == 1).json())
                            .unwrap()
                    );
                    assert!(!snapshot["provenance"]["collector"]["domain_problems"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|p| p["domain_id"] == 20));
                }
            }
            let output = serde_json::to_vec(&serde_json::json!({"schema":1,"stages":stages})).unwrap();
            assert!(output.len() <= 1024 * 1024);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)
                .unwrap();
            file.write_all(&output).unwrap();
            file.sync_all().unwrap();
        }

        fn countries(result: &MotionWithMembership) -> Vec<i32> {
            match &result.first_combat_countries {
                FirstCombatCountries::Present {
                    countries: Ok(words),
                    combat,
                } => {
                    assert_eq!(result.motion.scalars.first_combat, Some(Some(*combat)));
                    words.iter().collect()
                }
                other => panic!("expected complete membership, got {other:?}"),
            }
        }
        fn outer_failure(result: &MotionWithMembership) -> membership::Refusal {
            assert_eq!(result.motion.scalars.first_combat, None);
            match result.first_combat_countries {
                FirstCombatCountries::Unavailable(reason) => reason,
                ref other => panic!("expected outer unavailable, got {other:?}"),
            }
        }

        #[test]
        fn actual_adapter_preserves_motion_semantics_and_reads_every_identity_only_twice() {
            for class in [CombatClass::Land, CombatClass::BorderWar] {
                for numerator in [i64::MIN, -1, 0, i64::MAX] {
                    let f = AdapterFixture::new(class, &[0, -1, i32::MIN, i32::MAX, 0]);
                    f.mem.put64(f.division.body + 0x230, numerator as u64);
                    f.mem.put64(f.division.body + 0x2e8, 0x123);
                    f.mem.put64(f.division.body + 0x1d8, 0x456);
                    f.mem.put(f.division.body + 0x23c, &[251]);
                    let expected = MotionInputs {
                        ordinary_route: Some(vec![19, 17, 19]),
                        alternate_route: Some(vec![]),
                        route_head: Some(None),
                        scalars: compact_contract::division_routes::MovementScalars {
                            numerator_raw: Some(numerator),
                            naval_transfer_membership: Some(true),
                            movement_target_present: Some(true),
                            retreat_raw: Some(251),
                            first_combat: Some(Some(compact_contract::division_routes::CombatIdentity {
                                class,
                                kind: if class == CombatClass::Land { 62 } else { 999 },
                                id: 79,
                            })),
                        },
                    };
                    let budget = country_roster_readset::Budget::default();
                    let (result, log) = f.run(&budget, |_, _, _| true);
                    assert_eq!(result.motion, expected);
                    assert_eq!(countries(&result), [0, -1, i32::MIN, i32::MAX, 0]);
                    assert_eq!(log.bytes(), 378 + 8 * 3 + 24 * 5);
                    assert_eq!(log.bytes(), budget.verification_bytes());
                    assert_eq!(log.visits.get(&(f.division.body - 0x10 + 0x1e0)), Some(&2));
                    assert_eq!(
                        result.country_operand,
                        compact_contract::division_routes::TrainingObservation::Measured(i32::MIN)
                    );
                    assert!(!log.visits.contains_key(&(f.division.body - 0x10 + 0x1d8)));
                    log.all_closed();
                }
            }
        }

        #[test]
        fn actual_empty_vector_empty_census_null_side_and_unreadable_count_remain_distinct() {
            let mut f = AdapterFixture::new(CombatClass::Land, &[]);
            f.division.combat_count = 0;
            f.mem
                .put32(f.division.body + DIVISION_OFF_COMBAT_REF_COUNT, 0);
            let budget = country_roster_readset::Budget::test_limits(250 + 8 * 3, 16 * 1024 * 1024);
            let (result, log) = f.run(&budget, |_, _, _| true);
            assert!(matches!(
                result.first_combat_countries,
                FirstCombatCountries::NoFirstCombat
            ));
            assert_eq!(result.motion.scalars.first_combat, Some(None));
            assert!(!log.visits.contains_key(&f.vector));
            assert_eq!(log.bytes(), 250 + 8 * 3);
            log.all_closed();

            let (result, log) = f.run(
                &country_roster_readset::Budget::default(),
                |f, at, visit| !(at == f.division.body + DIVISION_OFF_COMBAT_REF_COUNT && visit == 1),
            );
            assert_eq!(outer_failure(&result).cause, Cause::Unreadable);
            assert_eq!(result.motion, MotionInputs::default());
            log.all_closed();

            let f = AdapterFixture::new(CombatClass::Land, &[]);
            let (result, log) = f.run(&country_roster_readset::Budget::default(), |_, _, _| true);
            assert!(countries(&result).is_empty());
            log.all_closed();
            f.mem.put64(f.combat + 0x28, 0);
            let (result, log) = f.run(&country_roster_readset::Budget::default(), |_, _, _| true);
            match result.first_combat_countries {
                FirstCombatCountries::Present {
                    countries: Err(reason),
                    combat,
                } => {
                    assert_eq!(reason.cause, Cause::NullSide);
                    assert_eq!(result.motion.scalars.first_combat, Some(Some(combat)));
                }
                other => panic!("{other:?}"),
            }
            log.all_closed();
        }

        #[test]
        fn positive_count_null_first_entry_is_invalid_identity_not_empty() {
            let f = AdapterFixture::new(CombatClass::Land, &[]);
            f.mem.put64(f.vector, 0);
            let (result, log) = f.run(&country_roster_readset::Budget::default(), |_, _, _| true);
            let reason = outer_failure(&result);
            assert_eq!(
                (reason.cause, reason.field),
                (Cause::InvalidIdentity, Field::FirstCombatPointer)
            );
            assert_eq!(result.motion.scalars.numerator_raw, Some(0));
            assert!(!log.visits.contains_key(&f.combat));
            log.all_closed();
        }

        #[test]
        fn first_pointer_reorder_and_complete_identity_reuse_fail_both_combat_projections() {
            for offset in [0usize, 0x10, 0x18, 0x1c, usize::MAX] {
                let f = AdapterFixture::new(CombatClass::Land, &[1]);
                let replacement = f.combat + 0x1000;
                // Same typed identity at another complete object is not custody.
                f.mem.put64(
                    replacement,
                    (f.module + LAND_COMBAT_PRIMARY_VTABLE_RVA) as u64,
                );
                f.mem.put64(
                    replacement + 0x10,
                    (f.module + LAND_COMBAT_BODY_VTABLE_RVA) as u64,
                );
                f.mem.put32(replacement + 0x18, 62);
                f.mem.put32(replacement + 0x1c, 79);
                let (result, log) = f.run(
                    &country_roster_readset::Budget::default(),
                    |f, at, visit| {
                        if at == f.nodes && visit == 1 {
                            if offset == usize::MAX {
                                f.mem.put64(f.vector, replacement as u64);
                            } else if offset < 0x18 {
                                f.mem.put64(f.combat + offset, 0);
                            } else {
                                f.mem.put32(f.combat + offset, 80);
                            }
                        }
                        true
                    },
                );
                assert_eq!(outer_failure(&result).cause, Cause::Changed);
                assert_eq!(result.motion.scalars.numerator_raw, Some(0));
                assert_eq!(result.motion.ordinary_route, Some(vec![19, 17, 19]));
                assert!(!log.visits.contains_key(&replacement));
                log.all_closed();
            }
        }

        #[test]
        fn every_parent_identity_mutation_invalidates_motion_and_membership_after_node_failure() {
            let prototype = AdapterFixture::new(CombatClass::Land, &[1]);
            let body = prototype.division.body;
            let root =
                prototype.module + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA;
            for (at, width) in [
                (root, 8),
                (prototype.frame.game_state + GS_CURRENT_DATE_OFFSET, 4),
                (body - 0x10, 8),
                (body, 8),
                (body + 8, 4),
                (body + 12, 4),
                (body + DIVISION_OFF_LOCATION, 4),
                (body + DIVISION_OFF_COMBAT_REF_COUNT, 4),
            ] {
                let f = AdapterFixture::new(CombatClass::Land, &[1]);
                let (result, log) = f.run(
                    &country_roster_readset::Budget::default(),
                    |f, current, visit| {
                        if current == at && visit == 2 {
                            let mut bytes = f.mem.bytes(at, width).unwrap();
                            bytes[0] ^= 1;
                            f.mem.put(at, &bytes);
                        }
                        !(current == f.nodes && visit == 1)
                    },
                );
                outer_failure(&result);
                assert_eq!(result.motion, MotionInputs::default());
                let last_root = log
                    .events
                    .iter()
                    .rposition(|(current, _)| *current == root)
                    .unwrap();
                let last_node = log
                    .events
                    .iter()
                    .rposition(|(current, _)| *current == f.nodes)
                    .unwrap();
                assert!(last_root > last_node);
                log.all_closed();
            }
        }

        #[test]
        fn callback_generation_failure_invalidates_empty_and_present_results_and_always_runs() {
            for empty in [false, true] {
                let mut f = AdapterFixture::new(CombatClass::BorderWar, &[1]);
                if empty {
                    f.division.combat_count = 0;
                    f.mem
                        .put32(f.division.body + DIVISION_OFF_COMBAT_REF_COUNT, 0);
                }
                let (result, log) = f.run(
                    &country_roster_readset::Budget::default(),
                    |f, at, visit| {
                        if at == f.division.body + 0x230 && visit == 1 {
                            f.generation.set(2);
                        }
                        true
                    },
                );
                assert_eq!(outer_failure(&result).cause, Cause::Changed);
                assert_eq!(result.motion, MotionInputs::default());
                log.all_closed();
            }
            let f = AdapterFixture::new(CombatClass::Land, &[1]);
            let budget = country_roster_readset::Budget::test_limits(0, 16 * 1024 * 1024);
            let (result, log) = f.run(&budget, |_, _, _| true);
            assert_eq!(
                outer_failure(&result).cause,
                Cause::VerificationBudgetExceeded
            );
            assert!(log.events.is_empty());
            log.all_closed();
        }

        #[test]
        fn raw_motion_mutations_refuse_only_the_affected_field_even_if_presence_is_unchanged() {
            for (offset, width) in [(0x230, 8), (0x2e8, 8), (0x1d8, 8), (0x23c, 1)] {
                let f = AdapterFixture::new(CombatClass::Land, &[1]);
                if width == 8 {
                    f.mem.put64(f.division.body + offset, 17);
                } else {
                    f.mem.put(f.division.body + offset, &[17]);
                }
                let (result, log) = f.run(
                    &country_roster_readset::Budget::default(),
                    |f, at, visit| {
                        if at == f.division.body + offset && visit == 2 {
                            if width == 8 {
                                f.mem.put64(at, 18);
                            } else {
                                f.mem.put(at, &[18]);
                            }
                        }
                        true
                    },
                );
                assert_eq!(countries(&result), [1]);
                assert_eq!(
                    result.motion.scalars.numerator_raw,
                    if offset == 0x230 { None } else { Some(0) }
                );
                assert_eq!(
                    result.motion.scalars.naval_transfer_membership,
                    if offset == 0x2e8 { None } else { Some(false) }
                );
                assert_eq!(
                    result.motion.scalars.movement_target_present,
                    if offset == 0x1d8 { None } else { Some(false) }
                );
                assert_eq!(
                    result.motion.scalars.retreat_raw,
                    if offset == 0x23c { None } else { Some(0) }
                );
                log.all_closed();
            }
        }

        #[test]
        fn route_order_and_head_closes_preserve_independent_motion_and_membership() {
            for change_head in [false, true] {
                let f = AdapterFixture::new(CombatClass::Land, &[-1, 0]);
                let province = f.division.body + 0x50000;
                f.mem.put64(f.division.body + 0x1e0, province as u64);
                f.mem
                    .put64(province, (f.module + CPROVINCE_VTABLE_RVA) as u64);
                f.mem.put32(province + CPROVINCE_OFF_ID, 31);
                let budget = country_roster_readset::Budget::default();
                let (result, log) = f.run(&budget, |f, at, visit| {
                    if visit == 2
                        && at
                            == if change_head {
                                province + CPROVINCE_OFF_ID
                            } else {
                                f.division.body + 0x10000 + 4
                            }
                    {
                        f.mem.put32(at, 32);
                    }
                    true
                });
                assert_eq!(countries(&result), [-1, 0]);
                assert_eq!(
                    result.motion.route_head,
                    if change_head { None } else { Some(Some(31)) }
                );
                assert_eq!(
                    result.motion.ordinary_route,
                    if change_head {
                        Some(vec![19, 17, 19])
                    } else {
                        None
                    }
                );
                assert_eq!(result.motion.alternate_route, Some(vec![]));
                assert_eq!(log.bytes(), 402 + 8 * 3 + 24 * 2);
                assert_eq!(log.bytes(), budget.verification_bytes());
                log.all_closed();
            }
        }

        #[test]
        fn exact_budget_and_one_byte_short_close_actual_adapter_without_prefix_or_rediscovery() {
            let f = AdapterFixture::new(CombatClass::Land, &[0, 7, 0]);
            let full = 378 + 8 * 3 + 24 * 3;
            for limit in [full, full - 1] {
                let budget = country_roster_readset::Budget::test_limits(limit, 16 * 1024 * 1024);
                let (result, log) = f.run(&budget, |_, _, _| true);
                if limit == full {
                    assert_eq!(countries(&result), [0, 7, 0]);
                    assert_eq!(log.bytes(), full);
                } else {
                    match &result.first_combat_countries {
                        FirstCombatCountries::Present {
                            countries: Err(reason),
                            combat,
                        } => {
                            assert_eq!(reason.cause, Cause::VerificationBudgetExceeded);
                            assert_eq!(reason.ordinal, Some(2));
                            assert_eq!(result.motion.scalars.first_combat, Some(Some(*combat)));
                        }
                        other => panic!("{other:?}"),
                    }
                    assert!(!log.visits.contains_key(&(f.nodes + 0x40)));
                    assert_eq!(log.bytes(), full - 24);
                }
                assert_eq!(budget.verification_bytes(), full);
                log.all_closed();
            }
        }

        #[test]
        fn retained_growth_failure_and_poisoned_quota_do_not_skip_adapter_closes() {
            let f = AdapterFixture::new(CombatClass::Land, &[1]);
            let roomy = country_roster_readset::Budget::default();
            let (_, first_log) = f.run(&roomy, |_, _, _| true);
            first_log.all_closed();
            let exact_retained = roomy.retained_bytes();
            let short =
                country_roster_readset::Budget::test_limits(10 * 1024 * 1024, exact_retained - 1);
            let (result, log) = f.run(&short, |_, _, _| true);
            match result.first_combat_countries {
                FirstCombatCountries::Present {
                    countries: Err(reason),
                    ..
                } => assert_eq!(reason.cause, Cause::RetainedBudgetExceeded),
                other => panic!("{other:?}"),
            }
            assert_eq!(short.retained_bytes(), exact_retained);
            log.all_closed();

            let exact = 378 + 8 * 3 + 24;
            let budget = country_roster_readset::Budget::test_limits(exact, 16 * 1024 * 1024);
            let (result, log) = f.run(&budget, |f, at, visit| {
                if at == f.nodes && visit == 1 {
                    assert!(budget.reserve_verification(1).is_err());
                }
                true
            });
            assert_eq!(countries(&result), [1]);
            assert_eq!(budget.verification_bytes(), exact + 1);
            assert_eq!(log.bytes(), exact);
            log.all_closed();
        }

        #[test]
        fn actual_adapter_class_correlation_side_failure_and_later_complete_recovery() {
            let f = AdapterFixture::new(CombatClass::BorderWar, &[7, -1]);
            f.mem.put64(f.side, (f.module + 0x29913c0) as u64);
            let (result, log) = f.run(&country_roster_readset::Budget::default(), |_, _, _| true);
            match result.first_combat_countries {
                FirstCombatCountries::Present {
                    countries: Err(reason),
                    ..
                } => {
                    assert_eq!(
                        (reason.cause, reason.field),
                        (Cause::InvalidIdentity, Field::SideClass)
                    );
                    assert!(result.motion.scalars.first_combat.unwrap().is_some());
                }
                other => panic!("{other:?}"),
            }
            log.all_closed();
            f.mem.put64(f.side, (f.module + 0x29a5818) as u64);
            let (result, log) = f.run(&country_roster_readset::Budget::default(), |_, _, _| true);
            assert_eq!(countries(&result), [7, -1]);
            log.all_closed();
            // Acquisition-only recovery; the joint exporter covers publication and fold.
        }
        #[test]
        fn every_actual_adapter_read_failure_drains_all_funded_original_closes() {
            let baseline = AdapterFixture::new(CombatClass::Land, &[1, -1]);
            let (_, complete) =
                baseline.run(&country_roster_readset::Budget::default(), |_, _, _| true);
            for at in complete.visits.keys().copied() {
                for failed_visit in [1, 2] {
                    let f = AdapterFixture::new(CombatClass::Land, &[1, -1]);
                    let budget = country_roster_readset::Budget::default();
                    let (result, log) = f.run(&budget, |_, current, visit| {
                        !(current == at && visit == failed_visit)
                    });
                    assert_eq!(log.bytes(), budget.verification_bytes());
                    let body = f.division.body;
                    let root =
                        f.module + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA;
                    if [
                        root,
                        f.frame.game_state + GS_CURRENT_DATE_OFFSET,
                        body - 0x10,
                        body,
                        body + 8,
                        body + 12,
                        body + DIVISION_OFF_LOCATION,
                        body + DIVISION_OFF_COMBAT_REF_COUNT,
                    ]
                    .contains(&at)
                    {
                        outer_failure(&result);
                        assert_eq!(result.motion, MotionInputs::default());
                    } else if [
                        body + 0x198,
                        body + 0x1a0,
                        f.vector,
                        f.combat,
                        f.combat + 0x10,
                        f.combat + 0x18,
                        f.combat + 0x1c,
                    ]
                    .contains(&at)
                    {
                        assert_eq!(outer_failure(&result).cause, Cause::Unreadable);
                        assert_eq!(result.motion.scalars.numerator_raw, Some(0));
                    } else if at == f.combat + 0x28 || (at >= f.side && at < f.nodes + 0x40) {
                        assert!(matches!(
                            &result.first_combat_countries,
                            FirstCombatCountries::Present {
                                countries: Err(_),
                                ..
                            }
                        ));
                    } else {
                        assert_eq!(countries(&result), [1, -1]);
                    }
                    log.all_closed();
                }
            }
        }

        #[test]
        fn route_output_is_funded_before_first_element_and_growth_uses_same_trace_budget() {
            let f = AdapterFixture::new(CombatClass::Land, &[1]);
            let expected: Vec<_> = (0..67)
                .map(|index| if index % 2 == 0 { 19 } else { 17 })
                .collect();
            f.mem.vector(
                f.division.body,
                ORDINARY_ROUTE,
                f.division.body + 0x10000,
                &expected,
            );
            let budget = country_roster_readset::Budget::default();
            let before_buffer = Cell::new(0usize);
            let (result, log) = f.run(&budget, |f, at, visit| {
                if visit == 1 && at == f.division.body + ORDINARY_ROUTE + 12 {
                    before_buffer.set(budget.retained_bytes());
                }
                if visit == 1 && at == f.division.body + 0x10000 {
                    assert_eq!(
                        budget.retained_bytes(),
                        before_buffer.get() + expected.len() * std::mem::size_of::<u32>()
                    );
                }
                true
            });
            assert_eq!(result.motion.ordinary_route, Some(expected.clone()));
            assert_eq!(countries(&result), [1]);
            assert_eq!(log.bytes(), 378 + 8 * expected.len() + 24);
            log.all_closed();

            let short = country_roster_readset::Budget::test_limits(
                10 * 1024 * 1024,
                before_buffer.get() + expected.len() * std::mem::size_of::<u32>() - 1,
            );
            let (result, log) = f.run(&short, |_, _, _| true);
            assert_eq!(result.motion.ordinary_route, None);
            assert!(!log.visits.contains_key(&(f.division.body + 0x10000)));
            log.all_closed();
        }

        #[test]
        fn paired_country_preserves_signed_extremes_and_closes_after_membership() {
            use compact_contract::division_routes::TrainingObservation as Obs;
            for raw in [i32::MIN, -1, 0, i32::MAX] {
                let f = AdapterFixture::new(CombatClass::Land, &[raw, 0, raw]);
                let at = f.division.body - DIVISION_WRAPPER_TO_BODY + 0x1e0;
                f.mem.put32(at, raw as u32);
                let budget = country_roster_readset::Budget::default();
                let (result, log) = f.run(&budget, |_, _, _| true);
                assert_eq!(result.country_operand, Obs::Measured(raw));
                assert_eq!(countries(&result), [raw, 0, raw]);
                assert_eq!(log.visits.get(&at), Some(&2));
                let country_close = log
                    .events
                    .iter()
                    .rposition(|(address, _)| *address == at)
                    .unwrap();
                let node_close = log
                    .events
                    .iter()
                    .rposition(|(address, _)| *address == f.nodes)
                    .unwrap();
                assert!(country_close > node_close);
                assert_eq!(log.bytes(), budget.verification_bytes());
                log.all_closed();
            }
        }

        #[test]
        fn paired_country_failure_is_independent_and_never_replaced_with_owner() {
            use compact_contract::division_routes::{TrainingObservation as Obs, TrainingRefusal as R};
            for unreadable in [true, false] {
                for failed_visit in [1, 2] {
                    let f = AdapterFixture::new(CombatClass::BorderWar, &[i32::MIN]);
                    let at = f.division.body - DIVISION_WRAPPER_TO_BODY + 0x1e0;
                    let owner = f.division.body - DIVISION_WRAPPER_TO_BODY + 0x1d8;
                    f.mem.put32(owner, i32::MIN as u32);
                    let (result, log) = f.run(
                        &country_roster_readset::Budget::default(),
                        |f, current, visit| {
                            if current == at && visit == failed_visit {
                                if unreadable {
                                    return false;
                                }
                                f.mem.put32(at, 0);
                            }
                            true
                        },
                    );
                    let expected = if unreadable {
                        Obs::Unavailable(R::Unreadable)
                    } else if failed_visit == 1 {
                        Obs::Measured(0)
                    } else {
                        Obs::Unavailable(R::Changed)
                    };
                    assert_eq!(result.country_operand, expected);
                    assert_eq!(countries(&result), [i32::MIN]);
                    assert!(!log.visits.contains_key(&owner));
                    log.all_closed();
                }
            }
        }

        #[test]
        fn matched_prefix_does_not_publish_census_and_frame_failure_clears_paired_country() {
            use compact_contract::division_routes::{TrainingObservation as Obs, TrainingRefusal as R};
            for frame_changed in [false, true] {
                let f = AdapterFixture::new(CombatClass::Land, &[i32::MIN, 7]);
                let root =
                    f.module + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA;
                let (result, log) = f.run(
                    &country_roster_readset::Budget::default(),
                    |f, at, visit| {
                        if frame_changed && at == root && visit == 2 {
                            f.mem.put64(at, (f.frame.game_state + 0x1000) as u64);
                        }
                        !(at == f.nodes + 0x20 && visit == 1)
                    },
                );
                if frame_changed {
                    assert_eq!(outer_failure(&result).cause, Cause::RootChanged);
                    assert_eq!(result.country_operand, Obs::Unavailable(R::Changed));
                } else {
                    assert_eq!(result.country_operand, Obs::Measured(i32::MIN));
                    assert!(matches!(
                        result.first_combat_countries,
                        FirstCombatCountries::Present {
                            countries: Err(_),
                            ..
                        }
                    ));
                }
                log.all_closed();
            }
        }
    }
}
