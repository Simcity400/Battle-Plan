//! CFEXMember optional CLastTargetInfo. Writer141960770 copies the target
//! group's member count, not its identity; serializer141962D40 emits2E93/2F.
use crate::{Memory, cnaval_combatant::field};
/// LEA141960892; primary table142A0A138.
pub const VTABLE_RVA: usize = 0x02a0_a138;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub present: u8,
    pub is_convoy: u8,
    pub target_group_member_count: i32,
}
pub(crate) fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    let present = field::<u8>(memory, pointer, 0x10)?;
    if present == 0 {
        return Ok(Row::default());
    }
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(VTABLE_RVA)
            .ok_or("last-target module overflow")? as u64
    {
        return Err("last-target identity mismatch");
    }
    let row = Row {
        present,
        is_convoy: field(memory, pointer, 8)?,
        target_group_member_count: field(memory, pointer, 0xc)?,
    };
    if field::<u8>(memory, pointer, 0x10)? != present {
        return Err("last-target optional changed");
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_optional_does_not_read_inactive_storage_and_signed_count_survives() {
        let mut bytes = [0u8; 24];
        assert_eq!(
            read(
                &crate::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Ok(Row::default())
        );
        bytes[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
        bytes[8] = 0xfe;
        bytes[0xc..0x10].copy_from_slice(&i32::MIN.to_le_bytes());
        bytes[0x10] = 1;
        assert_eq!(
            read(
                &crate::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Ok(Row {
                present: 1,
                is_convoy: 0xfe,
                target_group_member_count: i32::MIN
            })
        );
    }
}
