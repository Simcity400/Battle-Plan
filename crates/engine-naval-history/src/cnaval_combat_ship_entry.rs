
use engine_sites::Rva;
use engine_native_memory::capture_text::Value;
use engine_native_memory::{Memory, read};

pub struct HitBuffers<'a> {
    pub air_hits: &'a mut Vec<engine_native_naval::naval_combat_air_hit::Row>,
    pub naval_hits: &'a mut Vec<engine_native_naval::naval_combat_hit::Row>,
    pub lengths: [usize; 2],
    pub required: [usize; 2],
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    pub member: u64,
    pub owner: u32,
    pub subunit_index: i32,
    pub formation_type: i32,
    pub build_cost_ic: i64,
    pub strength_ratio: i64,
    pub pride: u8,
    pub is_convoy: u8,
    pub name: engine_native_memory::capture_text::Owned,
    pub type_name: engine_native_memory::capture_text::Owned,
    pub sprite: engine_native_memory::capture_text::Owned,
    pub sunk_by: engine_native_memory::capture_text::Owned,
    pub variant: u64,
    pub convoy_client: u64,
    pub convoy_index: i32,
    pub naval_hits: [usize; 2],
    pub air_hits: [[usize; 2]; 2],
}
impl DetailsRow {
    pub fn allocated_bytes(&self) -> usize {
        self.name.allocated_bytes()
            + self.type_name.allocated_bytes()
            + self.sprite.allocated_bytes()
            + self.sunk_by.allocated_bytes()
    }
}

pub fn capture_details(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected_handle: u64,
    buffers: &mut HitBuffers<'_>,
) -> Result<DetailsRow, &'static str> {
    use engine_native::cnaval_combatant::field;
    if values(memory, base, pointer)?.0 != expected_handle {
        return Err("naval result copied ship handle mismatch");
    }
    let text = |offset| {
        engine_native_memory::capture_text::Owned::read(
            memory,
            pointer
                .checked_add(offset)
                .ok_or("naval result copied text overflow")?,
            false,
        )
        .map_err(|reason| {
            if reason == "string allocation failed" {
                reason
            } else {
                "naval result copied text invalid or unreadable"
            }
        })
    };
    let (naval_hits, air_hits) = capture_hits(memory, base, pointer, [0xd8, 0xf0, 0x108], buffers)?;
    Ok(DetailsRow {
        member: expected_handle,
        owner: field(memory, pointer, 0x18)?,
        subunit_index: field(memory, pointer, 0x1c)?,
        formation_type: field(memory, pointer, 0x20)?,
        build_cost_ic: field(memory, pointer, 0x28)?,
        strength_ratio: field(memory, pointer, 0x30)?,
        pride: field(memory, pointer, 0x38)?,
        is_convoy: field(memory, pointer, 0x39)?,
        name: text(0x40)?,
        type_name: text(0x60)?,
        sprite: text(0x80)?,
        sunk_by: text(0xa0)?,
        variant: field(memory, pointer, 0xc0)?,
        convoy_client: field(memory, pointer, 0xc8)?,
        convoy_index: field(memory, pointer, 0xd0)?,
        naval_hits,
        air_hits,
    })
}

type HitSpans = ([usize; 2], [[usize; 2]; 2]);

pub(crate) fn capture_hits(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    offsets: [usize; 3],
    buffers: &mut HitBuffers<'_>,
) -> Result<HitSpans, &'static str> {
    use engine_native::cfex_member::{EntryOutput, append_entries};
    use engine_native_memory::cpdx_array::Array;
    let mut counts = [0usize; 2];
    let mut arrays = [Array::default(); 3];
    for (index, (header, offset)) in arrays.iter_mut().zip(offsets).enumerate() {
        let address = pointer
            .checked_add(offset)
            .ok_or("naval result hit header overflow")?;
        let array = Array::read(memory, address, 8).ok_or("naval result hit array invalid")?;
        counts[usize::from(index == 0)] += array.count as usize;
        *header = array;
    }
    for (lane, count) in counts.into_iter().enumerate() {
        buffers.required[lane] = buffers.required[lane]
            .checked_add(count)
            .ok_or("naval result hit required count overflow")?;
    }
    let naval = append_entries(
        memory,
        arrays[0],
        8,
        EntryOutput::Owned(buffers.naval_hits),
        &mut buffers.lengths[1],
        |memory, p| engine_native_naval::naval_combat_hit::read(memory, base, p),
        Some(&engine_native_naval::naval_combat_hit::RECORD),
    )?;
    let mut air = [[0; 2]; 2];
    for (index, array) in arrays[1..].iter().copied().enumerate() {
        air[index] = append_entries(
            memory,
            array,
            8,
            EntryOutput::Owned(buffers.air_hits),
            &mut buffers.lengths[0],
            |memory, p| engine_native_naval::naval_combat_air_hit::read(memory, base, p),
            Some(&engine_native_naval::naval_combat_air_hit::RECORD),
        )?;
    }
    Ok((naval, air))
}

pub const VTABLE_RVA: Rva = Rva(0x029d_a620);
pub const SIZE: usize = 0x120;
pub const HANDLE: usize = 8;

pub const PRESENCE: usize = 0xb0;

pub(crate) fn values(
    memory: &impl Memory,
    base: usize,
    entry: usize,
) -> Result<(u64, u64), &'static str> {
    let header =
        read::<[u8; 16]>(memory, entry).ok_or("naval result member identity unreadable")?;
    let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
    if Some(vtable) != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64) {
        return Err("naval result member vtable mismatch");
    }
    let handle = u64::from_le_bytes(header[8..].try_into().unwrap());
    if handle == 0 {
        return Err("naval result member handle is null");
    }
    let value = entry
        .checked_add(PRESENCE)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("naval result member value unreadable")?;
    Ok((handle, value))
}

#[cfg(test)]
mod detail_tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use std::cell::{Cell, RefCell};
    const BASE: usize = 0x1_4000_0000;
    const ENTRY: usize = 0x100;
    const ID: u64 = 0xfeed_beef_1234_5678;
    struct Counted<'a> {
        memory: SliceMemory<'a>,
        calls: Cell<usize>,
        bytes: Cell<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl<'a> Counted<'a> {
        fn new(bytes: &'a [u8], fault: Option<usize>) -> Self {
            Self {
                memory: SliceMemory { base: 0, bytes },
                calls: Cell::new(0),
                bytes: Cell::new(0),
                reads: RefCell::default(),
                fault,
            }
        }
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let ordinal = self.calls.get();
            self.calls.set(self.calls.get() + 1);
            self.bytes.set(self.bytes.get() + destination.len());
            self.reads.borrow_mut().push((address, destination.len()));
            if self.fault == Some(ordinal) {
                destination.fill(0xed);
                return false;
            }
            self.memory.copy(address, destination)
        }
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        for (offset, value) in [
            (0, (BASE + VTABLE_RVA.0) as u64),
            (8, ID),
            (0x28, (-321i64) as u64),
            (0x30, 99999),
            (0xc0, 0x1122_3344_5566_7788),
            (0xc8, 0xaabb_ccdd_eeff_0011),
        ] {
            bytes[ENTRY + offset..ENTRY + offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        for offset in [0x40, 0x60, 0x80, 0xa0] {
            bytes[ENTRY + offset] = 0xff;
            bytes[ENTRY + offset + 16..ENTRY + offset + 24].copy_from_slice(&1u64.to_le_bytes());
            bytes[ENTRY + offset + 24..ENTRY + offset + 32].copy_from_slice(&15u64.to_le_bytes());
        }
        bytes[ENTRY + 0xd0..ENTRY + 0xd4].copy_from_slice(&(-1i32).to_le_bytes());
        bytes
    }
    fn add_hits(bytes: &mut [u8], offsets: [usize; 3]) {
        for (offset, (data, count)) in
            offsets
                .into_iter()
                .zip([(0x800u64, 2u32), (0x820, 1), (0x828, 1)])
        {
            bytes[ENTRY + offset..ENTRY + offset + 8].copy_from_slice(&data.to_le_bytes());
            bytes[ENTRY + offset + 8..ENTRY + offset + 12].copy_from_slice(&count.to_le_bytes());
            bytes[ENTRY + offset + 12..ENTRY + offset + 16].copy_from_slice(&count.to_le_bytes());
        }
        for (at, value) in [
            (0x800, 0x900u64),
            (0x808, 0x900),
            (0x820, 0x980),
            (0x828, 0x980),
            (0x900, (BASE + engine_native_naval::naval_combat_hit::VTABLE_RVA.0) as u64),
            (0x928, 15),
            (0x938, i64::MIN as u64),
            (0x940, i64::MAX as u64),
            (
                0x980,
                (BASE + engine_native_naval::naval_combat_air_hit::VTABLE_RVA.0) as u64,
            ),
            (0x988, 0xb00),
            (0xb00, (BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64),
            (0xb08, ID),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x990..0x994].copy_from_slice(&(-3i32).to_le_bytes());
        bytes[0x994..0x998].copy_from_slice(&u32::MAX.to_le_bytes());
    }
    #[test]
    fn hit_arenas_exceed_16384() {
        const COUNT: usize = 16_385;
        const DATA: usize = 0x2000;
        let mut bytes = fixture();
        add_hits(&mut bytes, [0xd8, 0xf0, 0x108]);
        bytes.resize(DATA + COUNT * 8, 0);
        bytes[ENTRY + 0xd8..ENTRY + 0xe0].copy_from_slice(&(DATA as u64).to_le_bytes());
        for offset in [0xe0, 0xe4] {
            bytes[ENTRY + offset..ENTRY + offset + 4]
                .copy_from_slice(&(COUNT as u32).to_le_bytes());
        }
        for entry in bytes[DATA..].chunks_exact_mut(8) {
            entry.copy_from_slice(&0x900u64.to_le_bytes());
        }
        let mut buffers = HitBuffers {
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 2],
            required: [0; 2],
        };
        capture_hits(
            &Counted::new(&bytes, None),
            BASE,
            ENTRY,
            [0xd8, 0xf0, 0x108],
            &mut buffers,
        )
        .unwrap();
        assert_eq!(buffers.naval_hits.len(), COUNT);
        assert_eq!(buffers.naval_hits[COUNT - 1].damage, i64::MIN);
    }

    #[test]
    fn copied_result_survives_missing_live_ship_and_preserves_bytes_and_full_refs() {
        let bytes = fixture();
        let memory = Counted::new(&bytes, None);
        let mut buffers = HitBuffers {
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 2],
            required: [0; 2],
        };
        let row = capture_details(&memory, BASE, ENTRY, ID, &mut buffers).unwrap();
        assert_eq!(row.member, ID);
        assert_eq!(row.build_cost_ic, -321);
        assert_eq!(row.strength_ratio, 99999);
        assert_eq!(row.name.as_bytes(), [0xff]);
        assert_eq!(row.sunk_by.as_bytes(), [0xff]);
        assert_eq!(row.variant, 0x1122_3344_5566_7788);
        assert_eq!(row.convoy_client, 0xaabb_ccdd_eeff_0011);
        assert_eq!(row.convoy_index, -1);
        assert_eq!(buffers.lengths, [0, 0]);
        let trace = memory.reads.borrow().clone();
        assert_eq!(trace.iter().filter(|p| **p == (ENTRY, 16)).count(), 1);
        assert!(!trace.contains(&(ENTRY, 0xd8)));
        for offset in [0xd8, 0xf0, 0x108] {
            assert_eq!(
                trace.iter().filter(|p| **p == (ENTRY + offset, 16)).count(),
                1
            );
        }
        for ordinal in 0..trace.len() {
            let memory = Counted::new(&bytes, Some(ordinal));
            let mut buffers = HitBuffers {
                air_hits: &mut Vec::new(),
                naval_hits: &mut Vec::new(),
                lengths: [0; 2],
                required: [0; 2],
            };
            assert!(capture_details(&memory, BASE, ENTRY, ID, &mut buffers).is_err());
            assert_eq!(*memory.reads.borrow(), trace[..=ordinal]);
            assert_eq!(buffers.lengths, [0; 2]);
        }
        eprintln!(
            "retained result ship, four one-byte copied strings/no hits: {} copies, {} bytes",
            memory.calls.get(),
            memory.bytes.get()
        );
    }
    #[test]
    fn hit_arena_growth_preserves_duplicate_occurrences_and_fault_order() {
        let mut bytes = fixture();
        add_hits(&mut bytes, [0xd8, 0xf0, 0x108]);
        let memory = Counted::new(&bytes, None);
        let mut buffers = HitBuffers {
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 2],
            required: [0; 2],
        };
        let row = capture_details(&memory, BASE, ENTRY, ID, &mut buffers).unwrap();
        assert_eq!(buffers.required, [2, 2]);
        assert_eq!(buffers.lengths, [2, 2]);
        assert_eq!(row.naval_hits, [0, 2]);
        assert_eq!(row.air_hits, [[0, 1], [1, 1]]);
        assert_eq!(buffers.naval_hits[0], buffers.naval_hits[1]);
        assert_eq!(buffers.air_hits[0], buffers.air_hits[1]);
        assert_eq!(buffers.naval_hits[0].damage, i64::MIN);
        assert_eq!(buffers.air_hits[0].count, -3);
        let trace = memory.reads.borrow().clone();
        assert_eq!(trace.iter().filter(|p| **p == (0x800, 16)).count(), 1);
        assert!(!trace.contains(&(0x800, 8)) && !trace.contains(&(0x808, 8)));
        for capacity in [0, 1, 8] {
            for fault in (0..trace.len()).map(Some).chain([None]) {
                let reference = Counted::new(&bytes, fault);
                let mut reference_buffers = HitBuffers {
                    air_hits: &mut vec![engine_native_naval::naval_combat_air_hit::Row::default(); 8],
                    naval_hits: &mut vec![engine_native_naval::naval_combat_hit::Row::default(); 8],
                    lengths: [0; 2],
                    required: [0; 2],
                };
                let expected = capture_details(&reference, BASE, ENTRY, ID, &mut reference_buffers);
                let observed = Counted::new(&bytes, fault);
                let mut actual = HitBuffers {
                    air_hits: &mut vec![engine_native_naval::naval_combat_air_hit::Row::default(); capacity],
                    naval_hits: &mut vec![engine_native_naval::naval_combat_hit::Row::default(); capacity],
                    lengths: [0; 2],
                    required: [0; 2],
                };
                assert_eq!(
                    capture_details(&observed, BASE, ENTRY, ID, &mut actual),
                    expected
                );
                assert_eq!(*observed.reads.borrow(), *reference.reads.borrow());
                assert_eq!(actual.lengths, reference_buffers.lengths);
                assert_eq!(actual.required, reference_buffers.required);
                assert_eq!(
                    &actual.air_hits[..actual.lengths[0]],
                    &reference_buffers.air_hits[..reference_buffers.lengths[0]]
                );
                assert_eq!(
                    &actual.naval_hits[..actual.lengths[1]],
                    &reference_buffers.naval_hits[..reference_buffers.lengths[1]]
                );
                if let Some(ordinal) = fault {
                    if expected.is_err() {
                        assert_eq!(*observed.reads.borrow(), trace[..=ordinal]);
                    } else {
                        assert_eq!(expected, Ok(row.clone()));
                    }
                } else {
                    assert_eq!(expected, Ok(row.clone()));
                }
            }
        }
        assert_eq!(
            capture_details(&memory, BASE, ENTRY, ID + 1, &mut buffers),
            Err("naval result copied ship handle mismatch")
        );
        let mut overflow = HitBuffers {
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 2],
            required: [usize::MAX, 0],
        };
        assert_eq!(
            capture_details(&memory, BASE, ENTRY, ID, &mut overflow),
            Err("naval result hit required count overflow")
        );
        assert!(overflow.air_hits.is_empty() && overflow.naval_hits.is_empty());
    }

    #[test]
    fn copied_air_preserves_variant_operands_and_first_fault_prefixes() {
                use crate::cnaval_combat_air_entry as air;
        const VARIANT: usize = 0x500;
        let mut bytes = vec![0; 0x1000];
        for (at, value) in [
            (ENTRY, (BASE + air::VTABLE_RVA.0) as u64),
            (ENTRY + 8, VARIANT as u64),
            (ENTRY + 0x30, 3),
            (ENTRY + 0x38, 15),
            (VARIANT, (BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64),
            (VARIANT + 8, ID),
            (VARIANT + 0x2a0, i64::MIN as u64),
            (VARIANT + 0x2a8, i64::MAX as u64),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (0x10, i32::MIN as u32),
            (0x14, i32::MAX as u32),
            (0x18, u32::MAX),
            (0x1c, u32::MAX),
        ] {
            bytes[ENTRY + offset..ENTRY + offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[ENTRY + 0x20..ENTRY + 0x23].copy_from_slice(&[b'A', 0, 0xff]);
        add_hits(&mut bytes, [0x40, 0x58, 0x70]);
        let memory = Counted::new(&bytes, None);
        let mut buffers = HitBuffers {
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 2],
            required: [0; 2],
        };
        let row = air::capture_details(&memory, BASE, ENTRY, &mut buffers).unwrap();
        assert_eq!(row.variant, ID);
        assert_eq!(row.variant_summary_inputs, [i64::MIN, i64::MAX]);
        assert_eq!(
            (row.initial_planes, row.current_planes, row.lost_planes),
            (i32::MIN, i32::MAX, -1)
        );
        assert_eq!(row.country, u32::MAX);
        assert_eq!(row.name.as_bytes(), &[b'A', 0, 0xff]);
        assert_eq!((row.naval_hits, row.air_hits), ([0, 2], [[0, 1], [1, 1]]));
        assert_eq!(buffers.required, [2, 2]);
        assert_eq!(buffers.lengths, [2, 2]);
        let trace = memory.reads.borrow().clone();
        assert_eq!(trace.iter().filter(|p| **p == (VARIANT + 8, 8)).count(), 1);
        assert_eq!(trace.iter().filter(|p| **p == (ENTRY, 8)).count(), 1);
        assert!(!trace.contains(&(ENTRY, 0x40)));
        for ordinal in 0..trace.len() {
            let memory = Counted::new(&bytes, Some(ordinal));
            let mut buffers = HitBuffers {
                air_hits: &mut Vec::new(),
                naval_hits: &mut Vec::new(),
                lengths: [0; 2],
                required: [0; 2],
            };
            match air::capture_details(&memory, BASE, ENTRY, &mut buffers) {
                Err(_) => assert_eq!(*memory.reads.borrow(), trace[..=ordinal]),
                Ok(observed) => assert_eq!(
                    (observed.variant, observed.naval_hits, observed.air_hits),
                    (row.variant, row.naval_hits, row.air_hits)
                ),
            }
        }
    }
    #[test]
    fn copied_result_strings_keep_every_mod_sized_byte_without_the_old_256_bound() {
        const DATA: usize = 0x2000;
        for length in [257usize, 4097, 5000] {
            let mut bytes = fixture();
            bytes.resize(DATA + length + 1, 0);
            bytes[DATA..DATA + length].fill(0xff);
            for offset in [0x40, 0x60, 0x80, 0xa0] {
                bytes[ENTRY + offset..ENTRY + offset + 8]
                    .copy_from_slice(&(DATA as u64).to_le_bytes());
                bytes[ENTRY + offset + 16..ENTRY + offset + 24]
                    .copy_from_slice(&(length as u64).to_le_bytes());
                bytes[ENTRY + offset + 24..ENTRY + offset + 32]
                    .copy_from_slice(&(length as u64).to_le_bytes());
            }
            let memory = Counted::new(&bytes, None);
            let row = capture_details(
                &memory,
                BASE,
                ENTRY,
                ID,
                &mut HitBuffers {
                    air_hits: &mut Vec::new(),
                    naval_hits: &mut Vec::new(),
                    lengths: [0; 2],
                    required: [0; 2],
                },
            )
            .unwrap();
            for text in [&row.name, &row.type_name, &row.sprite, &row.sunk_by] {
                assert_eq!(text.as_bytes(), vec![0xff; length]);
            }
            assert!(row.allocated_bytes() >= 4 * length);
            assert!(engine_native_memory::msvc_string::read_text::<256>(&memory, ENTRY + 0x40).is_none());
        }
    }
}
