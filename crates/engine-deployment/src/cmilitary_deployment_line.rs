
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::cmilitary_deployment;
use engine_native::cname_group_member;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29da9b0);
pub const REFERENCE_TYPE: u32 = 63;
pub const TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const NAME: usize = 0x20;
pub const AMOUNT: usize = 0x28;
pub const DEPLOYMENT: usize = 0x30;
pub const PARENT: usize = 0x38;
pub const SIZE: usize = 0x40;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub handle: u64,
    pub conveyor: u64,
    pub country: u32,
    pub ordinal: u32,
    pub name_order: i32,
    pub amount: i32,
    pub training_raw: i64,
    pub max_training_raw: i64,
    pub max_manpower: i32,
    pub allow_zero_entries: u8,
    pub equipment_allocations: i32,
    pub distributable_priority: i32,
    pub halting_reason_present: u8,
    pub halting_reason: i32,
    pub custom_name_start: usize,
    pub custom_name_len: usize,
    pub custom_flag_a: u8,
    pub custom_flag_b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Children {
    pub equipment: Array,
    pub manpower: [Array; 2],
}

pub(crate) struct Read {
    pub row: Option<Row>,
    pub children: Children,
    pub used: usize,
    pub wanted: usize,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the conveyor, country, ordinal and row pointer identify one row, and the custom name buffer is caller-owned"
)]
pub(crate) fn read_row_buffer(
    memory: &impl Memory,
    base: usize,
    conveyor: u64,
    conveyor_pointer: usize,
    country: u32,
    ordinal: u32,
    pointer: usize,
    custom_names: &mut engine_native_memory::walk::Buffer<'_, u8>,
    custom_used: usize,
) -> Result<Read, &'static str> {
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("deployment line module address overflow")?;
    let head = read::<[u8; 0x40]>(memory, pointer).ok_or("deployment line fields unreadable")?;
    let quad = |offset: usize| u64::from_le_bytes(head[offset..offset + 8].try_into().unwrap());
    if quad(0) != expected as u64 {
        return Err("deployment line vtable mismatch");
    }
    let handle = quad(TYPE);
    if handle as u32 != REFERENCE_TYPE {
        return Err("deployment line reference type mismatch");
    }
    if quad(PARENT) != conveyor_pointer as u64 {
        return Err("deployment line parent conveyor mismatch");
    }
    let name = usize::try_from(quad(NAME))
        .ok()
        .filter(|pointer| *pointer != 0)
        .ok_or("deployment line name pointer unreadable or null")?;
    let name_order = cname_group_member::order(memory, base, name)?;
    let (source, custom_len, custom_flag_a, custom_flag_b) =
        cname_group_member::custom(memory, base, name)?;
    let deployment = usize::try_from(quad(DEPLOYMENT))
        .ok()
        .filter(|pointer| *pointer != 0)
        .ok_or("deployment line deployment pointer unreadable or null")?;
    let fields = cmilitary_deployment::read_fields(memory, base, deployment, pointer)?;
    let children = Children {
        equipment: fields.equipment,
        manpower: fields.manpower,
    };
    custom_names.ensure(
        custom_used
            .checked_add(custom_len)
            .ok_or("deployment name byte count overflow")?,
    )?;
    if custom_len > custom_names.len().saturating_sub(custom_used) {
        return Ok(Read {
            row: None,
            children,
            used: custom_used,
            wanted: custom_len,
        });
    }
    if custom_len != 0 {
        cname_group_member::copy_custom(memory, source, custom_len, custom_names, custom_used)
            .map_err(|_| "deployment line custom name bytes unreadable")?;
    }
    let row = Row {
        handle,
        conveyor,
        country,
        ordinal,
        name_order,
        amount: i32::from_le_bytes(head[AMOUNT..AMOUNT + 4].try_into().unwrap()),
        training_raw: fields.training_raw,
        max_training_raw: fields.max_training_raw,
        max_manpower: fields.max_manpower,
        allow_zero_entries: fields.allow_zero_entries,
        equipment_allocations: fields.equipment_allocations,
        distributable_priority: fields.distributable_priority,
        halting_reason_present: fields.halting_reason_present,
        halting_reason: fields.halting_reason,
        custom_name_start: custom_used,
        custom_name_len: custom_len,
        custom_flag_a,
        custom_flag_b,
    };
    Ok(Read {
        row: Some(row),
        children,
        used: custom_used + custom_len,
        wanted: custom_len,
    })
}
