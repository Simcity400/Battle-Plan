//! Accepted experience adjustments in the pinned a729 executable. Each tap is
//! immediately before the bank addition, after gain modifiers and any daily
//! rejection. Positive adjustments are income before the bank cap; negatives
//! are spending or corrections and must not be inferred from a balance fall.
//!
//! AddArmyExperience@141291410, AddNavyExperience@1412916A0 and
//! AddAirExperience@1412912D0 have one accepted-add path each. DailyUpdate
//! @141292FC0 adds the three passive modifiers directly, then clears its daily
//! counters. The counters themselves clamp army/navy gain, so reading them
//! cannot recover an accepted overshoot. Constructor141291090 and token reader
//! 141293AD0 restore state, and Reset@141292ED0 clears it; none is an income writer.

use crate::{Memory, ccountry, ccountry_experience_status as status, read};

pub struct Site {
    pub rva: usize,
    pub branch: usize,
    pub air_argument: bool,
    pub sentinel: [u8; 16],
}

/// `hoi4re asm`: RBX is the status and RDI the adjusted-amount pointer, except
/// AddAirExperience uses RSI. The relocated prefixes are a bank load and add
/// (seven bytes). No return PC or branch target lies inside those prefixes;
/// incoming branches target their first instruction. All native registers,
/// flags and XMM values must survive observation before the original addition.
/// Authentication uses the first 16 native bytes. At the DailyUpdate sites
/// (`asm 141292FC0`) this ends before the later floor-store displacement;
/// only the complete seven-byte load/add prefix is relocated.
pub const SITES: [Site; 6] = [
    Site {
        rva: 0x129151f,
        branch: 0,
        air_argument: false,
        sentinel: [
            0x48, 0x8b, 0x53, 0x10, 0x48, 0x03, 0x17, 0x48, 0x89, 0x53, 0x10, 0x79, 0x0a, 0x48,
            0xc7, 0x43,
        ],
    },
    Site {
        rva: 0x12917aa,
        branch: 1,
        air_argument: false,
        sentinel: [
            0x48, 0x8b, 0x53, 0x28, 0x48, 0x03, 0x17, 0x48, 0x89, 0x53, 0x28, 0x79, 0x0a, 0x48,
            0xc7, 0x43,
        ],
    },
    Site {
        rva: 0x12913a6,
        branch: 2,
        air_argument: true,
        sentinel: [
            0x48, 0x8b, 0x16, 0x48, 0x03, 0x53, 0x40, 0x48, 0x89, 0x53, 0x40, 0x79, 0x0a, 0x48,
            0xc7, 0x43,
        ],
    },
    Site {
        rva: 0x12930c0,
        branch: 0,
        air_argument: false,
        sentinel: [
            0x48, 0x8b, 0x53, 0x10, 0x48, 0x03, 0x17, 0x48, 0x89, 0x53, 0x10, 0x79, 0x06, 0x48,
            0x89, 0x6b,
        ],
    },
    Site {
        rva: 0x12931bf,
        branch: 1,
        air_argument: false,
        sentinel: [
            0x48, 0x8b, 0x53, 0x28, 0x48, 0x03, 0x17, 0x48, 0x89, 0x53, 0x28, 0x79, 0x06, 0x48,
            0x89, 0x6b,
        ],
    },
    Site {
        rva: 0x12932b2,
        branch: 2,
        air_argument: false,
        sentinel: [
            0x48, 0x8b, 0x17, 0x48, 0x03, 0x53, 0x40, 0x48, 0x89, 0x53, 0x40, 0x79, 0x06, 0x48,
            0x89, 0x6b,
        ],
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Owner {
    pub pointer: usize,
    pub instance: u32,
}

/// Authenticate the native status's owner/backlink, with no country selector.
/// CCountry +08/+0C is the raw country reference and native instance identity.
pub fn owner(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(Owner, u32), &'static str> {
    let mut header = [0; 16];
    if pointer == 0 || !memory.copy(pointer, &mut header) {
        return Err("experience gain status header unreadable");
    }
    if Some(u64::from_le_bytes(header[..8].try_into().unwrap()) as usize)
        != base.checked_add(status::VTABLE_RVA)
    {
        return Err("experience gain status vtable mismatch");
    }
    let country = usize::try_from(u64::from_le_bytes(header[8..].try_into().unwrap()))
        .map_err(|_| "experience gain owner pointer overflow")?;
    let mut identity = [0; 16];
    if country == 0 || !memory.copy(country, &mut identity) {
        return Err("experience gain owner identity unreadable");
    }
    if Some(u64::from_le_bytes(identity[..8].try_into().unwrap()) as usize)
        != base.checked_add(ccountry::VTABLE_RVA)
    {
        return Err("experience gain owner vtable mismatch");
    }
    let id = u32::from_le_bytes(identity[8..12].try_into().unwrap());
    if id == 0
        || country
            .checked_add(ccountry::EXPERIENCE_STATUS)
            .and_then(|at| read::<u64>(memory, at))
            != Some(pointer as u64)
    {
        return Err("experience gain owner backlink mismatch");
    }
    Ok((
        Owner {
            pointer: country,
            instance: u32::from_le_bytes(identity[12..].try_into().unwrap()),
        },
        id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    const STATUS: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x5000];
        bytes[STATUS..STATUS + 8].copy_from_slice(&(status::VTABLE_RVA as u64).to_le_bytes());
        bytes[STATUS + 8..STATUS + 16].copy_from_slice(&(COUNTRY as u64).to_le_bytes());
        bytes[COUNTRY..COUNTRY + 8].copy_from_slice(&(ccountry::VTABLE_RVA as u64).to_le_bytes());
        bytes[COUNTRY + 8..COUNTRY + 12].copy_from_slice(&5001_u32.to_le_bytes());
        bytes[COUNTRY + 12..COUNTRY + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[COUNTRY + ccountry::EXPERIENCE_STATUS..COUNTRY + ccountry::EXPERIENCE_STATUS + 8]
            .copy_from_slice(&(STATUS as u64).to_le_bytes());
        bytes
    }
    #[test]
    fn native_owner_preserves_alias_and_instance_and_requires_its_backlink() {
        let mut bytes = fixture();
        assert_eq!(
            owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                STATUS
            )
            .unwrap(),
            (
                Owner {
                    pointer: COUNTRY,
                    instance: u32::MAX
                },
                5001
            )
        );
        bytes[COUNTRY + ccountry::EXPERIENCE_STATUS] ^= 1;
        assert_eq!(
            owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                STATUS
            ),
            Err("experience gain owner backlink mismatch")
        );
    }
    #[test]
    fn short_status_country_and_backlink_copies_refuse_instead_of_zero() {
        let bytes = fixture();
        for end in [
            STATUS + 15,
            COUNTRY + 15,
            COUNTRY + ccountry::EXPERIENCE_STATUS + 7,
        ] {
            assert!(
                owner(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes[..end]
                    },
                    0,
                    STATUS
                )
                .is_err()
            );
        }
        assert!(
            owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                usize::MAX,
                STATUS
            )
            .is_err()
        );
    }
}
