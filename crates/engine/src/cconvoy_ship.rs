//! CConvoyShip raw bar operands, separate from transport-order objects.
//! Constructor141AC31F0 stores vtable142A23ED8 (instruction141AC3220).
use crate::{Memory, cnaval_combatant::field};
pub const VTABLE_RVA: usize = 0x02a2_3ed8;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub strength: i64,
    pub organisation: i64,
    pub maximum_strength: i64,
    pub maximum_organisation: i64,
}
///141AC37A0/141AC3510 divide these operands, with native minimum100. Keep
/// the unmodified variant max fields rather than those calculated quotients.
pub(crate) fn read(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(VTABLE_RVA)
            .ok_or("convoy ship module overflow")? as u64
        || field::<u64>(memory, pointer, 8)? != handle
    {
        return Err("convoy ship identity mismatch");
    }
    let variant = field::<u64>(memory, pointer, 0x18)? as usize;
    if field::<u64>(memory, variant, 0)?
        != base
            .checked_add(crate::cequipment_variant::VTABLE_RVA)
            .ok_or("convoy variant module overflow")? as u64
    {
        return Err("convoy ship variant identity mismatch");
    }
    let row = Row {
        strength: field(memory, pointer, 0x20)?,
        organisation: field(memory, pointer, 0x28)?,
        maximum_strength: field(memory, variant, 0x2e8)?,
        maximum_organisation: field(memory, variant, 0x2e0)?,
    };
    Ok(row)
}
