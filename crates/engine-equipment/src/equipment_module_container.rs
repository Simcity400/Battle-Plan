
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::Outcome,
};
use engine_native_parts::cequipment_module;
use engine_native::cequipment_variant;
use engine_native_politics::equipment_module_entry;

pub const IDENTITY_WORD_RVA_CANDIDATE: Rva = Rva(0x030f_d000);
pub const ENTRIES_CANDIDATE: usize = 0x08;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub variant: u64,
    pub source_ordinal: u32,
    pub slot: u32,
    pub definition_token: u32,
    pub definition_active: u8,
    pub definition_pointer: usize,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    variants: &[cequipment_variant::RegistryRow],
    output: &mut Vec<Row>,
) -> Outcome {
    let result = (|| {
        let expected = engine_sites::at(base, cequipment_module::VTABLE_RVA)
            .ok_or("module definition address overflow")?;
        let mut len = 0_usize;
        const BATCH: usize = 32;
        const HEADERS: [RecordField; 1] = [RecordField {
            source_offset: cequipment_variant::MODULES + ENTRIES_CANDIDATE,
            destination_offset: 0,
            len: 16,
        }];
        const DEFINITIONS: [RecordField; 2] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 12,
            },
            RecordField {
                source_offset: cequipment_module::ACTIVE,
                destination_offset: 12,
                len: 1,
            },
        ];
        const DEFINITION_SIZE: usize = 13;
        const HEADERS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&HEADERS, 16).expect("valid record layout");
        const DEFINITIONS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&DEFINITIONS, DEFINITION_SIZE).expect("valid record layout");
        let mut pointers = [0; BATCH];
        let mut headers = [0; BATCH * 16];
        let mut entries = [0; BATCH * equipment_module_entry::STRIDE];
        let mut definitions = [0; BATCH];
        let mut definition_fields = [0; BATCH * DEFINITION_SIZE];
        for sources in variants.chunks(BATCH) {
            for (pointer, source) in pointers.iter_mut().zip(sources) {
                *pointer = source.pointer();
            }
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..sources.len()]),
                &HEADERS_LAYOUT,
                &mut headers,
            );
            for (index, source) in sources.iter().enumerate() {
                if index >= copied {
                    return Err("variant module array unreadable or invalid");
                }
                let variant = source.id;
                let array = Array::from_header(
                    headers[index * 16..(index + 1) * 16].try_into().unwrap(),
                    equipment_module_entry::STRIDE,
                )
                .ok_or("variant module array unreadable or invalid")?;
                for first in (0..array.count).step_by(BATCH) {
                    let count = ((array.count - first) as usize).min(BATCH);
                    let address = array
                        .element(first, equipment_module_entry::STRIDE)
                        .ok_or("module entry address overflow")?;
                    if !read_slice(
                        memory,
                        address,
                        &mut entries[..count * equipment_module_entry::STRIDE],
                    ) {
                        return Err("module entry unreadable");
                    }
                    let required = len.checked_add(count).ok_or("module row count overflow")?;
                    if required > output.len() {
                        let additional = required - output.len();
                        crate::allocation::reserve(output, additional)
                            .map_err(|_| "module row allocation failed")?;
                        output.resize(required, Row::default());
                    }
                    for (pointer, entry) in definitions
                        .iter_mut()
                        .zip(entries.chunks_exact(equipment_module_entry::STRIDE))
                        .take(count)
                    {
                        *pointer =
                            usize::try_from(u64::from_le_bytes(entry[8..].try_into().unwrap()))
                                .unwrap_or(0);
                    }
                    let definition_copied = memory.copy_records(
                        RecordSources::Pointers(&definitions[..count]),
                        &DEFINITIONS_LAYOUT,
                        &mut definition_fields,
                    );
                    for (slot, entry) in entries[..count * equipment_module_entry::STRIDE]
                        .chunks_exact(equipment_module_entry::STRIDE)
                        .enumerate()
                    {
                        if definitions[slot] == 0 {
                            return Err("module definition pointer null or invalid");
                        }
                        if definition_copied < (slot + 1) * DEFINITIONS.len() {
                            return Err("module definition identity or active byte unreadable");
                        }
                        let field = &definition_fields
                            [slot * DEFINITION_SIZE..(slot + 1) * DEFINITION_SIZE];
                        if u64::from_le_bytes(field[..8].try_into().unwrap()) != expected as u64 {
                            return Err("module definition vtable mismatch");
                        }
                        output[len] = Row {
                            variant,
                            source_ordinal: first + slot as u32,
                            slot: u32::from_le_bytes(entry[..4].try_into().unwrap()),
                            definition_token: u32::from_le_bytes(field[8..12].try_into().unwrap()),
                            definition_active: field[12],
                            definition_pointer: definitions[slot],
                        };
                        len += 1;
                    }
                }
            }
        }
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn source(bytes: &[u8]) -> engine_native::cequipment_variant::RegistryRow {
        engine_native::cequipment_variant::RegistryRow::read(
            &SliceMemory {
                base: 0x1000,
                bytes,
            },
            0,
            0x1000,
        )
        .unwrap()
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x300];
        bytes[..8].copy_from_slice(&(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&0x1234_5678_0000_0046_u64.to_le_bytes());
        bytes[0xb0..0xb8].fill(0xee);
        bytes[0xb8..0xc0].copy_from_slice(&0x1180_u64.to_le_bytes());
        bytes[0xc0..0xc4].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0xc4..0xc8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x180..0x184].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[0x188..0x190].copy_from_slice(&0x1200_u64.to_le_bytes());
        bytes[0x200..0x208].copy_from_slice(&(cequipment_module::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x208..0x20c].copy_from_slice(&0x8765_4321_u32.to_le_bytes());
        bytes[0x210] = 0xff;
        bytes
    }

    fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory {
            base: 0x1000,
            bytes,
        }
    }

    #[test]
    fn module_native_count_does_not_allocate_before_entry_reads() {
        let mut bytes = fixture();
        bytes[0xb8..0xc0].copy_from_slice(&0x9000_u64.to_le_bytes());
        for start in [0xc0, 0xc4] {
            bytes[start..start + 4].copy_from_slice(&32_768_u32.to_le_bytes());
        }
        let mut rows = Vec::new();
        assert_eq!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut rows),
            Outcome::Unavailable {
                reason: "module entry unreadable"
            }
        );
        assert_eq!(rows.capacity(), 0);
    }

    #[test]
    fn module_uses_real_variant_and_definition_authentication() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 1];
        assert_eq!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            (rows[0].slot, rows[0].definition_token),
            (u32::MAX, 0x8765_4321)
        );
        assert_eq!(rows[0].definition_active, 0xff);
    }

    #[test]
    fn module_names_are_never_read_and_null_definitions_are_unavailable() {
        let mut bytes = fixture();
        bytes[0x218..0x238].fill(0xff);
        assert_eq!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut Vec::new()),
            Outcome::Rows { len: 1 }
        );
        bytes[0x188..0x190].fill(0);
        assert!(matches!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut Vec::new()),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn module_batch_boundary_grows_without_losing_native_order() {
        struct Trace<'a> {
            memory: SliceMemory<'a>,
            reads: std::cell::RefCell<Vec<(usize, usize)>>,
            blocked: Option<usize>,
        }
        impl Memory for Trace<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                !self
                    .blocked
                    .is_some_and(|blocked| address <= blocked && blocked - address < output.len())
                    && self.memory.copy(address, output)
            }
        }
        const COUNT: usize = 33;
        let mut bytes = fixture();
        bytes.resize(0x400 + COUNT * equipment_module_entry::STRIDE, 0);
        bytes[0xb8..0xc0].copy_from_slice(&0x1400_u64.to_le_bytes());
        bytes[0xc0..0xc4].copy_from_slice(&(COUNT as u32).to_le_bytes());
        bytes[0xc4..0xc8].copy_from_slice(&(COUNT as u32).to_le_bytes());
        for index in 0..COUNT {
            let start = 0x400 + index * equipment_module_entry::STRIDE;
            bytes[start..start + 4].copy_from_slice(&(100 + index as u32).to_le_bytes());
            bytes[start + 8..start + 16].copy_from_slice(&0x1200_u64.to_le_bytes());
        }
        let mut memory = Trace {
            memory: memory(&bytes),
            reads: Default::default(),
            blocked: None,
        };
        let variants = [source(&bytes)];
        let mut rows = Vec::new();
        assert_eq!(
            walk(&memory, 0, &variants, &mut rows),
            Outcome::Rows { len: COUNT }
        );
        assert_eq!((rows[32].source_ordinal, rows[32].slot), (32, 132));
        let reads = memory.reads.take();
        rows.truncate(32);
        assert_eq!(
            walk(&memory, 0, &variants, &mut rows),
            Outcome::Rows { len: COUNT }
        );
        assert_eq!(memory.reads.take(), reads);
        memory.blocked = Some(0x1400 + 32 * equipment_module_entry::STRIDE);
        let mut failed_rows = Vec::new();
        assert_eq!(
            walk(&memory, 0, &variants, &mut failed_rows),
            Outcome::Unavailable {
                reason: "module entry unreadable"
            }
        );
        assert_eq!(&failed_rows[..32], &rows[..32]);
    }
}
