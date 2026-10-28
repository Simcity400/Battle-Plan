//! Inline project-history context. Optional values are serialized only when
//! their presence byte is set. Absent payload words may remain indeterminate and
//! must never be read as zero.

/// Project-context primary vtable.
pub const VTABLE_RVA: usize = 0x029f_f1c0;
/// Context signed i32 province.
pub const PROVINCE: usize = 0x08;
/// Context signed i32 progress.
pub const PROGRESS: usize = 0x0c;
/// Context character full reference, u32 type and ID.
pub const CHARACTER_REFERENCE: usize = 0x10;
/// Optional u32 country reference, read only when present.
pub const COUNTRY_REFERENCE: usize = 0x18;
/// Country-presence u8 flag.
pub const COUNTRY_PRESENT: usize = 0x1c;
/// Optional u32 scientist reference.
pub const SCIENTIST_REFERENCE: usize = 0x20;
/// Scientist-presence u8 flag.
pub const SCIENTIST_PRESENT: usize = 0x24;
/// Optional u32 state reference.
pub const STATE_REFERENCE: usize = 0x28;
/// State-presence u8 flag.
pub const STATE_PRESENT: usize = 0x2c;
/// Context constructor clearing presence bytes without initializing absent payloads.
pub const CONSTRUCT_RVA: usize = 0x01ac_ab70;

use crate::{Memory, read};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub province: i32,
    /// Native bits, with no scaling or rounding.
    pub progress: i32,
    pub character_reference: u64,
    pub country_present: u8,
    pub country_reference: Option<u32>,
    pub scientist_present: u8,
    pub scientist_reference: Option<u32>,
    pub state_present: u8,
    pub state_reference: Option<u32>,
}

/// Reads no optional payload unless the corresponding native presence byte is
/// nonzero, matching the serializer. Absent storage may be indeterminate.
pub fn read_row(memory: &impl Memory, base: usize, address: usize) -> Result<Row, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("project context vtable overflow")?;
    if read::<u64>(memory, address) != Some(expected as u64) {
        return Err("project context vtable unreadable or mismatched");
    }
    let field = |offset| {
        address
            .checked_add(offset)
            .ok_or("project context address overflow")
    };
    let mut fixed = [0; 16];
    let mut country = [0];
    let mut scientist = [0];
    let mut state = [0];
    if memory.copy_scattered(
        [
            field(PROVINCE)?,
            field(COUNTRY_PRESENT)?,
            field(SCIENTIST_PRESENT)?,
            field(STATE_PRESENT)?,
        ],
        &mut [&mut fixed, &mut country, &mut scientist, &mut state],
    ) != 4
    {
        return Err("project context fields unreadable");
    }
    let optional = |presence, offset| {
        if presence == 0 {
            Ok(None)
        } else {
            read::<u32>(memory, field(offset)?)
                .map(Some)
                .ok_or("project context present payload unreadable")
        }
    };
    Ok(Row {
        province: i32::from_le_bytes(fixed[..4].try_into().unwrap()),
        progress: i32::from_le_bytes(fixed[4..8].try_into().unwrap()),
        character_reference: u64::from_le_bytes(fixed[8..16].try_into().unwrap()),
        country_present: country[0],
        country_reference: optional(country[0], COUNTRY_REFERENCE)?,
        scientist_present: scientist[0],
        scientist_reference: optional(scientist[0], SCIENTIST_REFERENCE)?,
        state_present: state[0],
        state_reference: optional(state[0], STATE_REFERENCE)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    struct AbsentPayloads<'a>(SliceMemory<'a>);

    impl Memory for AbsentPayloads<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for offset in [COUNTRY_REFERENCE, SCIENTIST_REFERENCE, STATE_REFERENCE] {
                let start = self.0.base + offset;
                assert!(
                    address + destination.len() <= start || address >= start + 4,
                    "absent optional payload was accessed"
                );
            }
            self.0.copy(address, destination)
        }
    }

    #[test]
    fn absent_payloads_are_never_read_and_native_bits_survive() {
        let mut bytes = [0xff; 0x30];
        bytes[..8].copy_from_slice(&(0x1000_u64 + VTABLE_RVA as u64).to_le_bytes());
        bytes[PROVINCE..PROVINCE + 4].copy_from_slice(&(-7_i32).to_le_bytes());
        for offset in [COUNTRY_PRESENT, SCIENTIST_PRESENT, STATE_PRESENT] {
            bytes[offset] = 0;
        }
        let memory = AbsentPayloads(SliceMemory {
            base: 0x2000,
            bytes: &bytes,
        });
        let row = read_row(&memory, 0x1000, 0x2000).unwrap();
        assert_eq!(row.province, -7);
        assert_eq!(row.progress, -1);
        assert_eq!(row.character_reference, u64::MAX);
        assert_eq!(row.country_reference, None);
        assert_eq!(row.scientist_reference, None);
        assert_eq!(row.state_reference, None);
    }

    #[test]
    fn present_payloads_are_exact_and_faults_are_not_absence() {
        let mut bytes = [0; 0x30];
        bytes[..8].copy_from_slice(&(0x1000_u64 + VTABLE_RVA as u64).to_le_bytes());
        bytes[COUNTRY_PRESENT] = 1;
        bytes[COUNTRY_REFERENCE..COUNTRY_REFERENCE + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let memory = SliceMemory {
            base: 0x2000,
            bytes: &bytes,
        };
        assert_eq!(
            read_row(&memory, 0x1000, 0x2000).unwrap().country_reference,
            Some(u32::MAX)
        );
        struct MissingCountry<'a>(SliceMemory<'a>);
        impl Memory for MissingCountry<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                address != self.0.base + COUNTRY_REFERENCE && self.0.copy(address, destination)
            }
        }
        assert_eq!(
            read_row(&MissingCountry(memory), 0x1000, 0x2000),
            Err("project context present payload unreadable")
        );
    }
}
