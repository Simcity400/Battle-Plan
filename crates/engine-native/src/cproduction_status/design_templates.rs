use super::*;
use engine_sites::Rva;

pub const RECORDS: usize = 0x4e8;
pub const RECORD_STRIDE: usize = 40;
pub const USES: usize = 0;
pub const FACTOR: usize = 8;
pub const EQUIPMENT: usize = 16;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DesignTemplate {
    pub uses: i32,
    pub factor: i64,
    pub equipment: Vec<u32>,
}

pub fn selected<'a>(
    entries: &'a [DesignTemplate],
    equipment: &[u32],
) -> Option<(usize, &'a DesignTemplate)> {
    let mut selected: Option<(usize, &DesignTemplate)> = None;
    for (ordinal, entry) in entries.iter().enumerate() {
        if equipment
            .iter()
            .any(|token| entry.equipment.contains(token))
            && selected.is_none_or(|(_, prior)| entry.factor > prior.factor)
        {
            selected = Some((ordinal, entry));
        }
    }
    selected
}

pub fn read_templates(
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> Result<Vec<DesignTemplate>, &'static str> {
    let at = |offset: usize| {
        status
            .checked_add(offset)
            .ok_or("design template address overflow")
    };
    if read::<u64>(memory, status) != engine_sites::at(base, VTABLE_RVA).map(|value| value as u64) {
        return Err("design template production owner identity mismatch");
    }
    let entries = Array::read(memory, at(RECORDS)?, RECORD_STRIDE)
        .ok_or("design template roster unreadable or invalid")?;
    let mut result = Vec::new();
    for ordinal in 0..entries.count {
        let address = entries
            .element(ordinal, RECORD_STRIDE)
            .ok_or("design template address overflow")?;
        let bytes = read::<[u8; RECORD_STRIDE]>(memory, address)
            .ok_or("design template entry unreadable")?;
        let tokens = Array::from_header(bytes[EQUIPMENT..EQUIPMENT + 16].try_into().unwrap(), 4)
            .ok_or("design template equipment roster invalid")?;
        let mut equipment = Vec::new();
        const BATCH: usize = 64;
        let mut buffer = [0_u8; BATCH * 4];
        for first in (0..tokens.count).step_by(BATCH) {
            let count = (tokens.count - first).min(BATCH as u32) as usize;
            let address = tokens
                .element(first, 4)
                .ok_or("design template equipment address overflow")?;
            if !memory.copy(address, &mut buffer[..count * 4]) {
                return Err("design template equipment token unreadable");
            }
            engine_allocation::reserve(&mut equipment, count)
                .map_err(|_| "design template allocation failed")?;
            equipment.extend(
                buffer[..count * 4]
                    .chunks_exact(4)
                    .map(|word| u32::from_le_bytes(word.try_into().unwrap())),
            );
        }
        engine_allocation::reserve(&mut result, 1)
            .map_err(|_| "design template allocation failed")?;
        result.push(DesignTemplate {
            uses: i32::from_le_bytes(bytes[USES..USES + 4].try_into().unwrap()),
            factor: i64::from_le_bytes(bytes[FACTOR..FACTOR + 8].try_into().unwrap()),
            equipment,
        });
    }
    Ok(result)
}

#[derive(Clone, Copy)]
pub enum Receiver {
    FirstArray,
    SecondArray,
    Status,
}
pub struct Mutation {
    pub name: &'static str,
    pub rva: Rva,
    pub receiver: Receiver,
}
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        name: "AppendDesignTemplate",
        rva: Rva(0x004b_6bb0),
        receiver: Receiver::FirstArray,
    },
    Mutation {
        name: "ReadDesignTemplates",
        rva: Rva(0x00ec_9050),
        receiver: Receiver::SecondArray,
    },
    Mutation {
        name: "ConsumeDesignTemplate",
        rva: Rva(0x00ee_29d0),
        receiver: Receiver::Status,
    },
    Mutation {
        name: "ResetDesignTemplates",
        rva: Rva(0x00ed_69a0),
        receiver: Receiver::Status,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn design_templates_preserve_uses_ties_duplicates_and_unclamped_factors() {
        let entries = [
            DesignTemplate {
                uses: 0,
                factor: 120_000,
                equipment: vec![2, 2],
            },
            DesignTemplate {
                uses: 5,
                factor: 120_000,
                equipment: vec![3],
            },
        ];
        assert_eq!(selected(&entries, &[3, 2]).unwrap().0, 0);
        assert_eq!(selected(&entries, &[2]).unwrap().1.uses, 0);
        assert_eq!(selected(&entries, &[2]).unwrap().1.factor, 120_000);
        assert_eq!(selected(&entries, &[7]), None);
        let negative = [DesignTemplate {
            uses: -5,
            factor: -30,
            equipment: vec![2],
        }];
        assert_eq!(selected(&negative, &[2]).unwrap().0, 0);
    }
    #[test]
    fn design_templates_read_only_bounded_payload_and_preserve_known_empty() {
        let mut bytes = vec![0_u8; 0x800];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        fn memory(bytes: &[u8]) -> engine_native_memory::memory::SliceMemory<'_> {
            engine_native_memory::memory::SliceMemory {
                base: 0x1000,
                bytes,
            }
        }
        assert!(
            read_templates(&memory(&bytes), 0, 0x1000)
                .unwrap()
                .is_empty()
        );
        bytes[RECORDS..RECORDS + 8].copy_from_slice(&0x1600_u64.to_le_bytes());
        bytes[RECORDS + 8..RECORDS + 16]
            .copy_from_slice(&[1_u32.to_le_bytes(), 1_u32.to_le_bytes()].concat());
        bytes[0x600..0x604].copy_from_slice(&(-7_i32).to_le_bytes());
        bytes[0x608..0x610].copy_from_slice(&123_457_i64.to_le_bytes());
        bytes[0x610..0x618].copy_from_slice(&0x1700_u64.to_le_bytes());
        bytes[0x618..0x620].copy_from_slice(&[3_u32.to_le_bytes(), 3_u32.to_le_bytes()].concat());
        bytes[0x700..0x70c].copy_from_slice(
            &[
                2_u32.to_le_bytes(),
                2_u32.to_le_bytes(),
                u32::MAX.to_le_bytes(),
            ]
            .concat(),
        );
        assert_eq!(
            read_templates(&memory(&bytes), 0, 0x1000).unwrap(),
            [DesignTemplate {
                uses: -7,
                factor: 123_457,
                equipment: vec![2, 2, u32::MAX]
            }]
        );
        bytes[0x61c..0x620].copy_from_slice(&4_u32.to_le_bytes());
        assert_eq!(
            read_templates(&memory(&bytes), 0, 0x1000),
            Err("design template equipment roster invalid")
        );
    }
}
