//! Military deployment line: identity, repetition count and owned deployment.
//!
//! The 0x40-byte line owns a name and an amount. A null deployment causes removal, so live lines
//! own a deployment. The walker authenticates its parent backlink.

use crate::{Memory, cmilitary_deployment, cname_group_member, cpdx_array::Array, read};

/// CMilitaryDeploymentLine primary vtable.
pub const VTABLE_RVA: usize = 0x29da9b0;
/// CMilitaryDeploymentLine reference kind.
pub const REFERENCE_TYPE: u32 = 63;
/// CMilitaryDeploymentLine reference type word, u32; the shared CReferenceObject qword.
pub const TYPE: usize = 0x08;
/// CMilitaryDeploymentLine reference id, u32; the high half of the same qword.
pub const ID: usize = 0x0c;
/// CMilitaryDeploymentLine division-name object pointer.
pub const NAME: usize = 0x20;
/// CMilitaryDeploymentLine amount, i32.
pub const AMOUNT: usize = 0x28;
/// Owned deployment pointer; null causes line removal.
pub const DEPLOYMENT: usize = 0x30;
/// CMilitaryDeploymentLine parent conveyor pointer.
pub const PARENT: usize = 0x38;
/// CMilitaryDeploymentLine allocation size.
pub const SIZE: usize = 0x40;

/// One line and the fields of the CMilitaryDeployment it owns.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native handle: reference type in the low word, id in the high word.
    pub handle: u64,
    /// Owning conveyor's full native handle.
    pub conveyor: u64,
    /// Raw country reference of the owning CDeploymentStatus.
    pub country: u32,
    /// Stored position inside the conveyor's line vector.
    pub ordinal: u32,
    /// Native signed order word of the division-name object; zero is a value.
    pub name_order: i32,
    pub amount: i32,
    /// Native i64 fixed5; zero is trained-from-scratch, not an absent value.
    pub training_raw: i64,
    pub max_training_raw: i64,
    pub max_manpower: i32,
    /// Raw pool byte of the deployment's in-transit equipment pool.
    pub allow_zero_entries: u8,
    /// Native count of the pool's allocation array; zero proves the engine
    /// cannot write the saved equipment wrapper.
    pub equipment_allocations: i32,
    /// CEquipmentDistributable priority; the save omits the default of one.
    pub distributable_priority: i32,
    /// Raw `_bIsSet` byte of the deployment's optional halting reason.
    pub halting_reason_present: u8,
    /// Raw halting-reason word; meaningful only when the flag byte is set.
    pub halting_reason: i32,
    /// Arena start of the literal custom division-name bytes from `cname_group_member::OVERRIDE_NAME`; empty names are valid.
    pub custom_name_start: usize,
    /// Custom-name byte length; zero omits the saved name.
    pub custom_name_len: usize,
    /// Raw flag byte (`cname_group_member::FLAG_A`); its save field is emitted only when clear.
    pub custom_flag_a: u8,
    /// Raw flag byte (`cname_group_member::FLAG_B`); its save field is emitted only when set.
    pub custom_flag_b: u8,
}

/// The child arrays a caller must visit after the row itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Children {
    pub equipment: Array,
    pub manpower: [Array; 2],
}

/// One line read. `children` is available even when the row itself is omitted,
/// so a caller that cannot store this line can still count exactly what it
/// left out.
pub(crate) struct Read {
    /// `None` when the literal custom name does not fit the caller's arena; the
    /// row is never published with a shortened name.
    pub row: Option<Row>,
    pub children: Children,
    /// Arena bytes used after this line, unchanged when the row is omitted.
    pub used: usize,
    /// The custom name's full length, reported whether or not it was copied.
    pub wanted: usize,
}

/// Authenticates the line, its parent backlink, its name object and its
/// deployment, including the deployment's own back pointer to this line, then
/// returns the merged row and the deployment's child arrays.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_row_buffer(
    memory: &impl Memory,
    base: usize,
    conveyor: u64,
    conveyor_pointer: usize,
    country: u32,
    ordinal: u32,
    pointer: usize,
    custom_names: &mut crate::walk::Buffer<'_, u8>,
    custom_used: usize,
) -> Result<Read, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("deployment line module address overflow")?;
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
