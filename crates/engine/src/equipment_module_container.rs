//! Equipment-module collection embedded at CEquipmentVariant+B0.
//! BCFF10/BD0D50 establish the array at +8 and its signed count at +14.
//! The old identity word at +0 is not a proven vtable and is not used to authenticate the collection.

use crate::{
    Memory, cequipment_module,
    cequipment_variant::{self, Literal, TextOutcome, TextSpan, text_failure},
    cpdx_array::Array,
    equipment_module_entry,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::Outcome,
};

/// Candidate module-container identity word; read_compact_air_variant_modules compares it, but RTTI validation fails.
pub const IDENTITY_WORD_RVA_CANDIDATE: usize = 0x0306_d170;
/// Candidate module-container data pointer; old air reader uses variant+B8, eight bytes after its assumed header.
pub const ENTRIES_CANDIDATE: usize = 0x08;
/// Candidate module-container u32 capacity; old air reader uses variant+C0, relative header+10.
pub const ENTRIES_CAPACITY_CANDIDATE: usize = 0x10;
/// Candidate module-container u32 count; old air reader uses variant+C4, relative header+14.
pub const ENTRIES_COUNT_CANDIDATE: usize = 0x14;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub variant: u64,
    pub source_ordinal: u32,
    pub slot: u32,
    pub definition_token: u32,
    pub definition_active: u8,
    pub definition_name: TextSpan,
}

/// The containing variant authenticates this header. Native BCFF10/BD0D50
/// traverse variant+B8/C4; the old word at B0 is not used as a fake vtable.
/// `variants` must be the same complete registry census used for variant
/// definitions. Missing source tables cannot be represented by a shorter slice.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    variants: &[cequipment_variant::RegistryRow],
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
) -> TextOutcome {
    let result = (|| {
        let expected = base
            .checked_add(cequipment_module::VTABLE_RVA)
            .ok_or("module definition address overflow")?;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        // One chunk's module entries are read with a single contiguous copy
        // whose failure is reported for the whole chunk, so this size is part
        // of the walker's error precedence rather than a free tuning knob.
        const BATCH: usize = 32;
        const HEADERS: [RecordField; 1] = [RecordField {
            source_offset: cequipment_variant::MODULES + ENTRIES_CANDIDATE,
            destination_offset: 0,
            len: 16,
        }];
        const DEFINITIONS: [RecordField; 3] = [
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
            RecordField {
                source_offset: cequipment_module::NAME,
                destination_offset: 13,
                len: 32,
            },
        ];
        const HEADERS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&HEADERS, 16).expect("valid record layout");
        const DEFINITIONS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&DEFINITIONS, 45).expect("valid record layout");
        let mut pointers = [0; BATCH];
        let mut headers = [0; BATCH * 16];
        let mut entries = [0; BATCH * equipment_module_entry::STRIDE];
        let mut definitions = [0; BATCH];
        let mut definition_fields = [0; BATCH * 45];
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
                        output
                            .try_reserve(required - output.len())
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
                        let ordinal = first + slot as u32;
                        if definitions[slot] == 0 {
                            return Err("module definition pointer null or invalid");
                        }
                        if definition_copied < (slot + 1) * DEFINITIONS.len() {
                            return Err("module definition identity or active byte unreadable");
                        }
                        let field = &definition_fields[slot * 45..(slot + 1) * 45];
                        if u64::from_le_bytes(field[..8].try_into().unwrap()) != expected as u64 {
                            return Err("module definition vtable mismatch");
                        }
                        let name = Literal::from_header(field[13..].try_into().unwrap())?;
                        let size = name.len;
                        required_bytes = required_bytes
                            .checked_add(size)
                            .ok_or("module name size overflow")?;
                        name.copy_growing(
                            memory,
                            bytes,
                            bytes_len,
                            "module name allocation failed",
                        )?;
                        output[len] = Row {
                            variant,
                            source_ordinal: ordinal,
                            slot: u32::from_le_bytes(entry[..4].try_into().unwrap()),
                            definition_token: u32::from_le_bytes(field[8..12].try_into().unwrap()),
                            definition_active: field[12],
                            definition_name: TextSpan {
                                start: bytes_len,
                                len: size,
                            },
                        };
                        bytes_len += size;
                        len += 1;
                    }
                }
            }
        }
        Ok(TextOutcome {
            outcome: Outcome::Rows { len },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(text_failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{memory::SliceMemory, walk::Outcome};

    fn source(bytes: &[u8]) -> crate::cequipment_variant::RegistryRow {
        crate::cequipment_variant::RegistryRow::read(
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
        bytes[..8].copy_from_slice(&(cequipment_variant::VTABLE_RVA as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&0x1234_5678_0000_0046_u64.to_le_bytes());
        bytes[0xb0..0xb8].fill(0xee);
        bytes[0xb8..0xc0].copy_from_slice(&0x1180_u64.to_le_bytes());
        bytes[0xc0..0xc4].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0xc4..0xc8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x180..0x184].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[0x188..0x190].copy_from_slice(&0x1200_u64.to_le_bytes());
        bytes[0x200..0x208].copy_from_slice(&(cequipment_module::VTABLE_RVA as u64).to_le_bytes());
        bytes[0x208..0x20c].copy_from_slice(&0x8765_4321_u32.to_le_bytes());
        bytes[0x210] = 0xff;
        bytes[0x218..0x21a].copy_from_slice(&[0xff, b'M']);
        bytes[0x228..0x230].copy_from_slice(&2_u64.to_le_bytes());
        bytes[0x230..0x238].copy_from_slice(&15_u64.to_le_bytes());
        bytes
    }

    #[test]
    fn module_native_count_does_not_allocate_before_entry_reads() {
        let mut bytes = fixture();
        bytes[0xb8..0xc0].copy_from_slice(&0x9000_u64.to_le_bytes());
        for start in [0xc0, 0xc4] {
            bytes[start..start + 4].copy_from_slice(&32_768_u32.to_le_bytes());
        }
        let mut rows = Vec::new();
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            &[source(&bytes)],
            &mut rows,
            &mut Vec::new(),
        );
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "module entry unreadable"
            }
        );
        assert_eq!(rows.capacity(), 0);
        assert_eq!(result.required_bytes, 0);
    }

    #[test]
    fn module_uses_real_variant_and_definition_authentication() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 1];
        let mut names = vec![0; 2];
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            &[source(&bytes)],
            &mut rows,
            &mut names,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            (rows[0].slot, rows[0].definition_token),
            (u32::MAX, 0x8765_4321)
        );
        assert_eq!(names, [0xff, b'M']);
        assert_eq!(rows[0].definition_active, 0xff);
    }

    #[test]
    fn module_names_never_clip_and_null_definitions_are_unavailable() {
        let mut bytes = fixture();
        let result = walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            &[source(&bytes)],
            &mut vec![Row::default(); 1],
            &mut vec![0; 1],
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.required_bytes, 2);
        bytes[0x188..0x190].fill(0);
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                &[source(&bytes)],
                &mut vec![Row::default(); 1],
                &mut vec![0; 8]
            )
            .outcome,
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
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            reads: Default::default(),
            blocked: None,
        };
        let variants = [source(&bytes)];
        let mut rows = Vec::new();
        let mut names = Vec::new();
        let result = walk(&memory, 0, &variants, &mut rows, &mut names);
        assert_eq!(result.outcome, Outcome::Rows { len: COUNT });
        assert_eq!((rows[32].source_ordinal, rows[32].slot), (32, 132));
        assert_eq!(&names[64..], &[0xff, b'M']);
        let reads = memory.reads.take();
        rows.truncate(32);
        names.truncate(64);
        let result = walk(&memory, 0, &variants, &mut rows, &mut names);
        assert_eq!(result.outcome, Outcome::Rows { len: COUNT });
        assert_eq!((result.bytes_len, result.required_bytes), (66, 66));
        assert_eq!(memory.reads.take(), reads);
        memory.blocked = Some(0x1400 + 32 * equipment_module_entry::STRIDE);
        let mut failed_rows = Vec::new();
        assert_eq!(
            walk(&memory, 0, &variants, &mut failed_rows, &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "module entry unreadable"
            }
        );
        assert_eq!(&failed_rows[..32], &rows[..32]);
    }
}
