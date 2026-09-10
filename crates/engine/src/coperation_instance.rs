//! COperationInstance, the persistent espionage-operation object.
//!
//! The pinned executable's RTTI identifies the primary vtable. Fields come from
//! `compact_contract/src/build_profile.rs:515-540` and the old D53
//! `extractor.rs::read_operations`; they still need hourly consistency and save
//! validation. The old reader derived phase/progress/status values from these
//! fields, so its fixed-five progress output is not a native stored field.

use crate::{Memory, coperation, cpdx_array::Array, cprovince, read};

/// COperationInstance primary vtable; build_profile.rs:515 and pinned a729 RTTI confirm the class, subobject offset zero.
pub const VTABLE_RVA: usize = 0x029a_76c0;
/// COperationInstance packed u64 reference at +8; read_operations reads low u32 type and high u32 instance id.
pub const HANDLE: usize = 0x08;
/// Historical COperationInstance reference type 4713 from build_profile.rs:518; 13E9210 registers a dynamic full handle, so this is not an admission whitelist.
pub const REFERENCE_TYPE: u32 = 4_713;
/// COperationInstance signed i64 fixed-point preparation; 13ED380 parses it and 13EC480 compares signed qwords.
pub const PREPARATION_CURRENT: usize = 0x30;
/// COperationInstance signed i64 fixed-point preparation requirement; 13ED380 parses it and 13EC480 compares signed qwords.
pub const PREPARATION_REQUIRED: usize = 0x38;
/// COperationInstance boolean flag; 1189390 stores its fifth argument and 13ED380 parses it as bool.
pub const FLAG_40: usize = 0x40;
/// COperationInstance auto-repeat flag; 118B110 tests it after completion and 13ED380 parses it as bool.
pub const AUTO_REPEAT: usize = 0x41;
/// COperationInstance terminal byte; read_operations at extractor.rs:7898 accepts zero or one at +42.
pub const TERMINAL: usize = 0x42;
/// COperationInstance COperation definition pointer; read_operations authenticates the +48 pointee's vtable.
pub const DEFINITION: usize = 0x48;
/// COperationInstance owning CCountry pointer; read_operations joins +50 through CCountry's raw tag id.
pub const OWNER_COUNTRY: usize = 0x50;
/// COperationInstance target-country raw u32 tag id; read_operations reads +58 and joins the country table.
pub const TARGET_COUNTRY_ID: usize = 0x58;
/// COperationInstance optional target CProvince pointer; read_operation_target_state authenticates +60 before following CProvince::STATE.
pub const TARGET_PROVINCE: usize = 0x60;
/// COperationInstance CGameDate signed i32 hour; 13EAD80 copies CGameState+468 here on start, and 13EF710 saves it only when duration is nonzero.
pub const START_HOUR: usize = 0x70;
/// COperationInstance signed i32 duration days; 13ED380 uses signed parser 24AB970, with zero tested by 118B110.
pub const DURATION_DAYS: usize = 0x80;
/// COperationInstance assigned-operative record-array pointer; read_operation_assigned_operatives reads +E0 with 0x38-byte elements.
pub const ASSIGNED_OPERATIVE_DATA: usize = 0xe0;
/// COperationInstance assigned-slot signed capacity; 13E9210 grows +E0 storage by 0x38-byte records using this word.
pub const ASSIGNED_OPERATIVE_CAPACITY: usize = 0xe8;
/// COperationInstance assigned-slot signed count; 13E9210 initializes all slots, including unassigned handles.
pub const ASSIGNED_OPERATIVE_COUNT: usize = 0xec;
/// COperationInstance selected-phase array data; 13E9210 constructs this CPdxArray and copies selected phases into it.
pub const PHASE_DATA: usize = 0xf8;
/// COperationInstance signed phase-array count; 13E9210 initializes +F8/+100/+104 and 13EF710 serializes that many definition pointers in order.
pub const PHASE_COUNT: usize = 0x104;
/// COperationInstance inline resource map; operation_is_prepared at extractor.rs:8889 uses a header rooted at +110.
pub const RESOURCE_MAP: usize = 0x110;
/// COperationInstance requirement-record array pointer; operation_is_prepared reads +158 with 0x20-byte elements.
pub const REQUIREMENT_DATA: usize = 0x158;
/// COperationInstance requirement-record u32 count; operation_is_prepared reads +164 before walking optional and required resources.
pub const REQUIREMENT_COUNT: usize = 0x164;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub owner: u32,
    pub target: u32,
    pub target_province: Option<u32>,
    pub definition_token: u32,
    pub preparation_current: i64,
    pub preparation_required: i64,
    pub flag_40: u8,
    pub auto_repeat: u8,
    pub terminal: u8,
    pub start_hour: i32,
    pub duration_days: i32,
    pub phase_count: u32,
    pub assigned_slots: u32,
}

pub(crate) struct Instance {
    pub row: Row,
    pub owner_pointer: usize,
    pub assigned: Array,
}

pub(crate) fn instance(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Instance, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("operation address overflow")
    };
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("operation module address overflow")?;
    let identity = read::<[u8; 16]>(memory, pointer).ok_or("operation identity unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
        return Err("operation instance vtable mismatch");
    }
    let id = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if id == 0 {
        return Err("operation instance has null handle");
    }
    let mut preparation = [0; 19];
    let mut definition_pointer = [0; 8];
    let mut owner_pointer = [0; 8];
    let mut target = [0; 4];
    let mut province_pointer = [0; 8];
    let mut start_hour = [0; 4];
    let mut duration_days = [0; 4];
    let mut phase_count = [0; 4];
    if memory.copy_scattered(
        [
            at(PREPARATION_CURRENT)?,
            at(DEFINITION)?,
            at(OWNER_COUNTRY)?,
            at(TARGET_COUNTRY_ID)?,
            at(TARGET_PROVINCE)?,
            at(START_HOUR)?,
            at(DURATION_DAYS)?,
            at(PHASE_COUNT)?,
        ],
        &mut [
            &mut preparation,
            &mut definition_pointer,
            &mut owner_pointer,
            &mut target,
            &mut province_pointer,
            &mut start_hour,
            &mut duration_days,
            &mut phase_count,
        ],
    ) != 8
    {
        return Err("operation instance fields unreadable");
    }
    if preparation[16..].iter().any(|&flag| flag > 1) {
        return Err("operation boolean flag invalid");
    }
    let phase_count = i32::from_le_bytes(phase_count);
    if phase_count < 0 {
        return Err("operation phase count negative");
    }
    let owner_pointer = usize::try_from(u64::from_le_bytes(owner_pointer))
        .map_err(|_| "operation owner pointer overflow")?;
    let definition_pointer = usize::try_from(u64::from_le_bytes(definition_pointer))
        .map_err(|_| "operation definition pointer overflow")?;
    let definition = coperation::definition(memory, base, definition_pointer)?;
    let province_pointer = usize::try_from(u64::from_le_bytes(province_pointer))
        .map_err(|_| "operation target province pointer overflow")?;
    let target_province = if province_pointer == 0 {
        None
    } else {
        let expected = base
            .checked_add(cprovince::VTABLE_RVA)
            .ok_or("province module address overflow")?;
        if read::<u64>(memory, province_pointer) != Some(expected as u64) {
            return Err("operation target province vtable mismatch");
        }
        Some(
            province_pointer
                .checked_add(cprovince::ID)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("operation target province id unreadable")?,
        )
    };
    let assigned = Array::read(
        memory,
        at(ASSIGNED_OPERATIVE_DATA)?,
        crate::operation_assigned_operative::STRIDE,
    )
    .ok_or("operation assigned slots unreadable or invalid")?;
    Ok(Instance {
        row: Row {
            id,
            target: u32::from_le_bytes(target),
            target_province,
            definition_token: definition,
            preparation_current: i64::from_le_bytes(preparation[..8].try_into().unwrap()),
            preparation_required: i64::from_le_bytes(preparation[8..16].try_into().unwrap()),
            flag_40: preparation[16],
            auto_repeat: preparation[17],
            terminal: preparation[18],
            start_hour: i32::from_le_bytes(start_hour),
            duration_days: i32::from_le_bytes(duration_days),
            phase_count: phase_count as u32,
            assigned_slots: assigned.count,
            ..Row::default()
        },
        owner_pointer,
        assigned,
    })
}
