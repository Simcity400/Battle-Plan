
use engine_native_memory::{Memory, read};
use engine_native::ccountry;
use crate::ccountry_experience_status as status;
use engine_sites::Rva;

pub struct Site {
    pub rva: Rva,
    pub branch: usize,
    pub air_argument: bool,
}

pub const SITES: [Site; 6] = [
    Site {
        rva: Rva(0x13363bf),
        branch: 0,
        air_argument: false,
    },
    Site {
        rva: Rva(0x133664a),
        branch: 1,
        air_argument: false,
    },
    Site {
        rva: Rva(0x1336246),
        branch: 2,
        air_argument: true,
    },
    Site {
        rva: Rva(0x1337f60),
        branch: 0,
        air_argument: false,
    },
    Site {
        rva: Rva(0x133805f),
        branch: 1,
        air_argument: false,
    },
    Site {
        rva: Rva(0x1338152),
        branch: 2,
        air_argument: false,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Owner {
    pub pointer: usize,
    pub country: u32,
}

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
        != engine_sites::at(base, status::VTABLE_RVA)
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
        != engine_sites::at(base, ccountry::VTABLE_RVA)
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
            country: id,
        },
        id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    const STATUS: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x5000];
        bytes[STATUS..STATUS + 8].copy_from_slice(&(status::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[STATUS + 8..STATUS + 16].copy_from_slice(&(COUNTRY as u64).to_le_bytes());
        bytes[COUNTRY..COUNTRY + 8].copy_from_slice(&(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[COUNTRY + 8..COUNTRY + 12].copy_from_slice(&5001_u32.to_le_bytes());
        bytes[COUNTRY + 12..COUNTRY + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[COUNTRY + ccountry::EXPERIENCE_STATUS..COUNTRY + ccountry::EXPERIENCE_STATUS + 8]
            .copy_from_slice(&(STATUS as u64).to_le_bytes());
        bytes
    }
    #[test]
    fn native_owner_preserves_current_link_and_requires_its_backlink() {
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
                    country: 5001
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
