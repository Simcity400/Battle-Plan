
use engine_sites::Rva;
use engine_native_memory::{Memory, read};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    pub leader: u64,
    pub hq_leader: u64,
    pub hq_influences: Vec<u64>,
}

pub fn capture_details(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<DetailsRow, &'static str> {
    authenticate(memory, base, pointer)?;
    let address = pointer
        .checked_add(0x50)
        .ok_or("naval result side fields overflow")?;
    let header = read::<[u8; 32]>(memory, address).ok_or("naval result side fields unreadable")?;
    let word =
        |index: usize| u64::from_le_bytes(header[index * 8..index * 8 + 8].try_into().unwrap());
    let [leader, hq_leader, begin, end] = [word(0), word(1), word(2), word(3)];
    let (hq_leader, begin, end) = hq_range(memory, base, hq_leader, begin, end)?;
    let bytes = end
        .checked_sub(begin)
        .ok_or("naval result HQ range reversed")?;
    if bytes % 8 != 0 || (bytes != 0 && begin == 0) {
        return Err("naval result HQ range invalid");
    }
    let mut row = DetailsRow {
        leader,
        hq_leader,
        hq_influences: Vec::new(),
    };
    crate::allocation::reserve_push_batch(&mut row.hq_influences, (bytes / 8) as usize)
        .map_err(|_| "collector allocation failed")?;
    for index in 0..(bytes / 8) as usize {
        let wrapper = read::<u64>(
            memory,
            (begin as usize)
                .checked_add(index * 8)
                .ok_or("naval result HQ element overflow")?,
        )
        .ok_or("naval result HQ element unreadable")? as usize;
        if wrapper == 0 {
            return Err("naval result HQ wrapper is null");
        }
        let at = wrapper
            .checked_add(0x40)
            .ok_or("naval result HQ reference wrapper overflow")?;
        let reference =
            read::<u64>(memory, at).ok_or("naval result HQ reference wrapper unreadable")? as usize;
        let value = if reference == 0 {
            0
        } else {
            read(
                memory,
                reference
                    .checked_add(8)
                    .ok_or("naval result HQ reference overflow")?,
            )
            .ok_or("naval result HQ reference unreadable")?
        };
        row.hq_influences.push(value);
    }
    Ok(row)
}

pub fn capture_influences(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    buffers: &mut engine_native_naval::naval_ship_modifier_inputs::InfluenceBuffers<'_>,
) -> Result<[usize; 2], &'static str> {
    authenticate(memory, base, pointer)?;
    let at = pointer
        .checked_add(0x58)
        .ok_or("naval result HQ range overflow")?;
    let header = read::<[u8; 24]>(memory, at).ok_or("naval result HQ range unreadable")?;
    let (_, begin, end) = hq_range(
        memory,
        base,
        u64::from_le_bytes(header[..8].try_into().unwrap()),
        u64::from_le_bytes(header[8..16].try_into().unwrap()),
        u64::from_le_bytes(header[16..].try_into().unwrap()),
    )?;
    let begin = begin as usize;
    let end = end as usize;
    let bytes = end
        .checked_sub(begin)
        .ok_or("naval result HQ range reversed")?;
    if bytes % 8 != 0 || (bytes != 0 && begin == 0) {
        return Err("naval result HQ range invalid");
    }
    let start = buffers.lengths[0];
    for index in 0..bytes / 8 {
        let wrapper = read::<u64>(
            memory,
            begin
                .checked_add(index * 8)
                .ok_or("naval result HQ wrapper overflow")?,
        )
        .ok_or("naval result HQ wrapper unreadable")? as usize;
        let destination = engine_native_naval::naval_ship_modifier_inputs::InfluenceBuffers {
            sources: &mut *buffers.sources,
            terms: &mut *buffers.terms,
            lengths: &mut *buffers.lengths,
            required: &mut *buffers.required,
        };
        engine_native_naval::naval_ship_modifier_inputs::capture_influence(
            memory,
            base,
            wrapper,
            u32::try_from(index).map_err(|_| "naval result HQ source index overflow")?,
            destination,
        )?;
    }
    Ok([start, buffers.lengths[0] - start])
}

fn hq_range(
    memory: &impl Memory,
    base: usize,
    leader: u64,
    begin: u64,
    end: u64,
) -> Result<(u64, u64, u64), &'static str> {
    if engine_native_parts::creference_registry::resolve(memory, base, leader)
        .map_err(|_| "naval result HQ commander lookup unreadable")?
        .is_none()
    {
        return Ok((0, 0, 0));
    }
    Ok((leader, begin, end))
}

pub fn visit_entries(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    air: bool,
    mut visit: impl FnMut(u32, usize) -> Result<(), &'static str>,
) -> Result<usize, &'static str> {
    use engine_native_memory::cpdx_array::Array;
    authenticate(memory, base, pointer)?;
    let address = pointer
        .checked_add(if air { 8 } else { MEMBERS })
        .ok_or("naval result entry list overflow")?;
    let array = Array::read(memory, address, 8).ok_or("naval result entry list invalid")?;
    for index in 0..array.count {
        let entry = read::<u64>(
            memory,
            array
                .element(index, 8)
                .ok_or("naval result entry address overflow")?,
        )
        .ok_or("naval result entry pointer unreadable")? as usize;
        if entry == 0 {
            return Err("naval result entry pointer is null");
        }
        visit(index, entry)?;
    }
    Ok(array.count as usize)
}

pub const VTABLE_RVA: Rva = Rva(0x029d_a670);

pub const SIZE: usize = 0x70;
pub const MEMBERS: usize = 0x20;
pub const COUNTRIES: usize = 0x38;
#[cfg(test)]
pub const COUNTRIES_CAPACITY: usize = 0x40;
pub const COUNTRIES_COUNT: usize = 0x44;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00d4_1340);

pub(crate) fn authenticate(
    memory: &impl Memory,
    base: usize,
    side: usize,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, side) != Some(expected as u64) {
        return Err("naval result side vtable mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod detail_tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use std::cell::RefCell;

    const HQ: u64 = 0x1_0000_0001;

    struct HqMemory<'a> {
        bytes: &'a [u8],
        base: usize,
    }
    impl Memory for HqMemory<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if at == self.base + engine_native_parts::creference_registry::LOW_ROOTS_RVA.0 + 8 {
                out.copy_from_slice(&0x1000u64.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(at, out)
        }
    }
    fn live_hq(bytes: &mut Vec<u8>) {
        use engine_native_parts::{
            creference_registry as registry,
            reference_bucket as bucket,
        };
        bytes.resize(bytes.len().max(0x1800), 0);
        let hash = HQ.rotate_left(32).wrapping_mul(registry::HASH_MULTIPLIER);
        let slot = ((hash >> 32) as u32 ^ hash as u32) & 7;
        let at = 0x1100 + slot as usize * bucket::STRIDE;
        for (at, value) in [
            (0x1008, 0x1100u64),
            (at + bucket::TYPE, HQ),
            (at + bucket::OBJECT, 0x1700),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x1010..0x1014].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x1014..0x1018].copy_from_slice(&7u32.to_le_bytes());
        bytes[0x1018] = 5;
        bytes[at + bucket::DISTANCE] = 1;
        bytes[0x1100 + 13 * bucket::STRIDE + bucket::DISTANCE] = 255;
    }

    struct Traced<M> {
        memory: M,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }

    impl<M: Memory> Memory for Traced<M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            if self.fault == Some(ordinal) {
                output.fill(0xed);
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    #[test]
    fn hq_native_wrapper_pointers_become_full_influence_references() {
        let mut bytes = vec![0; 0x800];
        let base = 0x1_4000_0000usize;
        for (at, value) in [
            (0x100, (base + VTABLE_RVA.0) as u64),
            (0x150, 0xaabb_ccdd_1234_5678),
            (0x158, HQ),
            (0x160, 0x300),
            (0x168, 0x310),
            (0x300, 0x400),
            (0x308, 0x500),
            (0x440, 0x600),
            (0x540, 0),
            (0x608, 0xffee_ddcc_aabb_0099),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        live_hq(&mut bytes);
        let memory = Traced {
            memory: HqMemory {
                base,
                bytes: &bytes,
            },
            reads: RefCell::default(),
            fault: None,
        };
        let row = capture_details(&memory, base, 0x100).unwrap();
        assert_eq!(row.leader, 0xaabb_ccdd_1234_5678);
        assert_eq!(row.hq_leader, HQ);
        assert_eq!(row.hq_influences.len(), 2);
        assert_eq!(&row.hq_influences[..2], &[0xffee_ddcc_aabb_0099, 0]);
        let expected = memory.reads.borrow().clone();
        assert!(expected.ends_with(&[(0x300, 8), (0x440, 8), (0x608, 8), (0x308, 8), (0x540, 8)]));
        for ordinal in 0..expected.len() {
            let memory = Traced {
                memory: HqMemory {
                    base,
                    bytes: &bytes,
                },
                reads: RefCell::default(),
                fault: Some(ordinal),
            };
            assert!(capture_details(&memory, base, 0x100).is_err());
            assert_eq!(*memory.reads.borrow(), expected[..=ordinal]);
        }
    }

    #[test]
    fn entry_lists_and_empty_influences_keep_initial_headers_and_native_order() {
        let base = 0x1_4000_0000usize;
        let mut bytes = vec![0; 0x400];
        bytes[0x100..0x108].copy_from_slice(&((base + VTABLE_RVA.0) as u64).to_le_bytes());
        for offset in [8, MEMBERS] {
            let at = 0x100 + offset;
            bytes[at..at + 8].copy_from_slice(&0x300u64.to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&2u32.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&2u32.to_le_bytes());
        }
        bytes[0x300..0x308].copy_from_slice(&0x900u64.to_le_bytes());
        bytes[0x308..0x310].copy_from_slice(&0x800u64.to_le_bytes());
        for air in [false, true] {
            let expected = [
                (0x100, 8),
                (0x100 + if air { 8 } else { MEMBERS }, 16),
                (0x300, 8),
                (0x308, 8),
            ];
            for fault in [None, Some(0), Some(1), Some(2), Some(3)] {
                let memory = Traced {
                    memory: SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    },
                    reads: RefCell::default(),
                    fault,
                };
                let mut entries = Vec::new();
                let result = visit_entries(&memory, base, 0x100, air, |ordinal, pointer| {
                    entries.push((ordinal, pointer));
                    Ok(())
                });
                if let Some(ordinal) = fault {
                    assert!(result.is_err());
                    assert_eq!(*memory.reads.borrow(), expected[..=ordinal]);
                    assert_eq!(entries.len(), ordinal.saturating_sub(2));
                } else {
                    assert_eq!(result, Ok(2));
                    assert_eq!(entries, [(0, 0x900), (1, 0x800)]);
                    assert_eq!(*memory.reads.borrow(), expected);
                }
            }
        }
        let memory = Traced {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            reads: RefCell::default(),
            fault: None,
        };
        let mut lengths = [0; 2];
        let mut required = [0; 2];
        let mut sources = Vec::new();
        let mut terms = Vec::new();
        let mut buffers = engine_native_naval::naval_ship_modifier_inputs::InfluenceBuffers {
            sources: &mut sources,
            terms: &mut terms,
            lengths: &mut lengths,
            required: &mut required,
        };
        assert_eq!(
            capture_influences(&memory, base, 0x100, &mut buffers),
            Ok([0; 2])
        );
        assert_eq!(*memory.reads.borrow(), [(0x100, 8), (0x158, 24)]);
        assert_eq!(lengths, [0; 2]);
        assert_eq!(required, [0; 2]);
    }

    #[test]
    fn retained_influences_grow_and_preserve_duplicate_wrappers_and_fault_prefixes() {
        use engine_native_naval::naval_ship_modifier_inputs::{
            InfluenceBuffers,
            ModifierTerm,
            SourceRow,
        };
        use engine_native_parts::{
            creference_registry as registry,
            modifier_definition as md,
            reference_bucket,
        };
        const BASE: usize = 0x1000_0000;
        const REFERENCE: u64 = 0x1_0000_0001;
        struct InfluenceMemory<'a>(&'a [u8]);
        impl Memory for InfluenceMemory<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let special = if address == BASE + registry::LOW_ROOTS_RVA.0 + 8 {
                    Some(0x1000u64)
                } else if address == BASE + md::TABLE_RVA.0 {
                    assert_eq!(output.len(), 16);
                    output[..8].copy_from_slice(&0x6000u64.to_le_bytes());
                    output[8..12].copy_from_slice(&64u32.to_le_bytes());
                    output[12..].copy_from_slice(&64u32.to_le_bytes());
                    return true;
                } else {
                    None
                };
                if let Some(value) = special {
                    assert_eq!(output.len(), 8);
                    output.copy_from_slice(&value.to_le_bytes());
                    return true;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
        let mut bytes = vec![0; 0x8000];
        let hash = REFERENCE
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let slot = ((hash >> 32) as u32 ^ hash as u32) & 7;
        let bucket = 0x1100 + slot as usize * reference_bucket::STRIDE;
        for (at, value) in [
            (0x100, (BASE + VTABLE_RVA.0) as u64),
            (0x158, REFERENCE),
            (0x160, 0x300),
            (0x168, 0x310),
            (0x300, 0x400),
            (0x308, 0x400),
            (0x440, 0x500),
            (0x508, REFERENCE),
            (0x1008, 0x1100),
            (bucket + reference_bucket::TYPE, REFERENCE),
            (bucket + reference_bucket::OBJECT, 0x1800),
            (0x18a8, 0x2000),
            (0x2f70, 0x4000),
            (0x2f80, 0x4000),
            (0x2fa0, 0x4000),
            (0x2f90, 0x4000),
            (0x40e0, 0x5000),
            (0x5008, i64::MIN as u64),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (at, value) in [
            (0x1010, 1u32),
            (0x1014, 7),
            (0x2e7c, 2),
            (0x40e8, 1),
            (0x40ec, 1),
            (0x5000, 0x2a),
            (0x6000 + 0x2a * md::STRIDE + md::FLAGS, 0x81234567),
        ] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x1018] = 5;
        bytes[bucket + reference_bucket::DISTANCE] = 1;
        bytes[0x1100 + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE] = 255;
        let run = |size, fault| {
            let memory = Traced {
                memory: InfluenceMemory(&bytes),
                reads: RefCell::default(),
                fault,
            };
            let mut sources = vec![SourceRow::default(); size];
            let mut terms = vec![ModifierTerm::default(); size];
            let mut lengths = [0; 2];
            let mut required = [0; 2];
            let result = capture_influences(
                &memory,
                BASE,
                0x100,
                &mut InfluenceBuffers {
                    sources: &mut sources,
                    terms: &mut terms,
                    lengths: &mut lengths,
                    required: &mut required,
                },
            );
            (
                result,
                lengths,
                required,
                sources[..lengths[0]].to_vec(),
                terms[..lengths[1]].to_vec(),
                memory.reads.into_inner(),
            )
        };
        let complete = run(8, None);
        assert_eq!(complete.0, Ok([0, 8]));
        assert_eq!(complete.1, [8, 1]);
        assert_eq!(complete.2, [8, 1]);
        for (index, source) in complete.3.iter().enumerate() {
            assert_eq!(source.scope, 12 + (index % 4) as u8);
            assert_eq!(source.index, (index / 4) as u32);
            assert_eq!(source.reference, REFERENCE);
            assert_eq!(source.terms, [0, 1]);
        }
        assert!(complete.4.iter().all(|term| {
            term.key == 0x2a && term.raw == i64::MIN && term.metadata_flags == 0x81234567
        }));
        for fault in std::iter::once(None).chain((0..complete.5.len()).map(Some)) {
            let expected = run(8, fault);
            if let Some(ordinal) = fault {
                assert!(expected.0.is_err());
                assert_eq!(expected.5, complete.5[..=ordinal]);
            }
            for size in [0, 1] {
                assert_eq!(run(size, fault), expected);
            }
        }
    }

    #[test]
    fn absent_hq_commander_never_dereferences_a_freed_borrowed_range() {
        let base = 0x1_4000_0000usize;
        let mut bytes = vec![0; 0x200];
        bytes[0x100..0x108].copy_from_slice(&((base + VTABLE_RVA.0) as u64).to_le_bytes());
        bytes[0x160..0x168].copy_from_slice(&0xffff_ffff_ffff_fff0u64.to_le_bytes());
        bytes[0x168..0x170].copy_from_slice(&1u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let row = capture_details(&memory, base, 0x100).unwrap();
        assert_eq!(row.hq_leader, 0);
        assert!(row.hq_influences.is_empty());
        assert_eq!(
            capture_influences(
                &memory,
                base,
                0x100,
                &mut engine_native_naval::naval_ship_modifier_inputs::InfluenceBuffers {
                    sources: &mut Vec::new(),
                    terms: &mut Vec::new(),
                    lengths: &mut [0; 2],
                    required: &mut [0; 2],
                }
            ),
            Ok([0; 2])
        );

        live_hq(&mut bytes);
        bytes[0x158..0x160].copy_from_slice(&HQ.to_le_bytes());
        bytes[0x1010..0x1014].copy_from_slice(&0u32.to_le_bytes());
        let hash = HQ
            .rotate_left(32)
            .wrapping_mul(engine_native_parts::creference_registry::HASH_MULTIPLIER);
        let slot = ((hash >> 32) as u32 ^ hash as u32) & 7;
        bytes[0x1100
            + slot as usize * engine_native_parts::reference_bucket::STRIDE
            + engine_native_parts::reference_bucket::DISTANCE] = 0;
        let memory = HqMemory {
            base,
            bytes: &bytes,
        };
        assert_eq!(capture_details(&memory, base, 0x100).unwrap().hq_leader, 0);
        assert_eq!(
            capture_influences(
                &memory,
                base,
                0x100,
                &mut engine_native_naval::naval_ship_modifier_inputs::InfluenceBuffers {
                    sources: &mut Vec::new(),
                    terms: &mut Vec::new(),
                    lengths: &mut [0; 2],
                    required: &mut [0; 2],
                }
            ),
            Ok([0; 2])
        );
    }
}
