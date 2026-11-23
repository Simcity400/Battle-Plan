
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    walk::Outcome,
};
use engine_native_parts::cequipment_upgrade;
use engine_native::cequipment_variant;
use engine_native_politics::equipment_upgrade_entry;

pub const VTABLE_RVA: Rva = Rva(0x0283_f1b0);
pub const ENTRIES: usize = 0x08;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub variant: u64,
    pub source_ordinal: u32,
    pub definition_token: i32,
    pub definition_pointer: usize,
    pub level: u8,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    variants: &[cequipment_variant::RegistryRow],
    output: &mut Vec<Row>,
) -> Outcome {
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA)
            .ok_or("upgrade module address overflow")?;
        let mut len = 0_usize;
        let definition_vtable = engine_sites::at(base, cequipment_upgrade::VTABLE_RVA)
            .ok_or("upgrade definition module address overflow")?;
        const BATCH: usize = 64;
        const HEADERS: [RecordField; 1] = [RecordField {
            source_offset: cequipment_variant::UPGRADES,
            destination_offset: 0,
            len: 24,
        }];
        const ENTRIES_FIELDS: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 9,
        }];
        const DEFINITIONS: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 12,
        }];
        const HEADERS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&HEADERS, 24).expect("valid record layout");
        const ENTRIES_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&ENTRIES_FIELDS, 9).expect("valid record layout");
        const DEFINITIONS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&DEFINITIONS, 12).expect("valid record layout");
        let mut pointers = [0; BATCH];
        let mut headers = [0; BATCH * 24];
        let mut entries = [0; BATCH * 9];
        let mut entry_pointers = [0; BATCH];
        let mut definitions = [0; BATCH];
        let mut definition_fields = [0; BATCH * 12];
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
                    return Err("variant upgrade instance header unreadable");
                }
                let variant = source.id;
                let header = &headers[index * 24..(index + 1) * 24];
                if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
                    return Err("variant upgrade instance vtable mismatch");
                }
                let array = Array::from_header(
                    header[8..].try_into().unwrap(),
                    equipment_upgrade_entry::STRIDE,
                )
                .ok_or("variant upgrade array unreadable or invalid")?;
                for first in (0..array.count).step_by(BATCH) {
                    let count = ((array.count - first) as usize).min(BATCH);
                    for (slot, pointer) in entry_pointers.iter_mut().take(count).enumerate() {
                        *pointer = array
                            .element(first + slot as u32, equipment_upgrade_entry::STRIDE)
                            .ok_or("upgrade entry address overflow")?;
                    }
                    let entry_copied = memory.copy_records(
                        RecordSources::Pointers(&entry_pointers[..count]),
                        &ENTRIES_LAYOUT,
                        &mut entries,
                    );
                    let required = len
                        .checked_add(entry_copied)
                        .ok_or("upgrade row count overflow")?;
                    if required > output.len() {
                        crate::allocation::reserve(output, required - output.len())
                            .map_err(|_| "upgrade row allocation failed")?;
                        output.resize(required, Row::default());
                    }
                    for (pointer, entry) in definitions
                        .iter_mut()
                        .zip(entries.chunks_exact(9))
                        .take(entry_copied)
                    {
                        *pointer =
                            usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                                .unwrap_or(0);
                    }
                    let definition_copied = memory.copy_records(
                        RecordSources::Pointers(&definitions[..entry_copied]),
                        &DEFINITIONS_LAYOUT,
                        &mut definition_fields,
                    );
                    for slot in 0..count {
                        if slot >= entry_copied {
                            return Err("upgrade entry unreadable");
                        }
                        if definitions[slot] == 0 {
                            return Err("upgrade definition pointer null or invalid");
                        }
                        if definition_copied <= slot {
                            return Err("upgrade definition identity unreadable");
                        }
                        let field = &definition_fields[slot * 12..(slot + 1) * 12];
                        if u64::from_le_bytes(field[..8].try_into().unwrap())
                            != definition_vtable as u64
                        {
                            return Err("upgrade definition vtable mismatch");
                        }
                        output[len] = Row {
                            variant,
                            source_ordinal: first + slot as u32,
                            definition_token: i32::from_le_bytes(field[8..12].try_into().unwrap()),
                            definition_pointer: definitions[slot],
                            level: entries[slot * 9 + 8],
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
    use engine_native_memory::{memory::SliceMemory, read_census::ReadCensus};

    fn source(bytes: &[u8]) -> engine_native::cequipment_variant::RegistryRow {
        engine_native::cequipment_variant::RegistryRow::read(&memory(bytes), 0, 0x1000).unwrap()
    }

    fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory {
            base: 0x1000,
            bytes,
        }
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x300];
        bytes[..8].copy_from_slice(&(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&0x8000_0000_0000_0046_u64.to_le_bytes());
        bytes[0x68..0x70].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x70..0x78].copy_from_slice(&0x1180_u64.to_le_bytes());
        bytes[0x78..0x7c].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x7c..0x80].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x180..0x188].copy_from_slice(&0x1200_u64.to_le_bytes());
        bytes[0x188] = 255;
        bytes[0x189..0x190].fill(0xce);
        bytes[0x200..0x208].copy_from_slice(&(cequipment_upgrade::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x208..0x20c].copy_from_slice(&i32::MIN.to_le_bytes());
        bytes
    }

    #[test]
    fn upgrade_native_count_does_not_allocate_before_entry_reads() {
        let mut bytes = fixture();
        bytes[0x70..0x78].copy_from_slice(&0x9000_u64.to_le_bytes());
        for start in [0x78, 0x7c] {
            bytes[start..start + 4].copy_from_slice(&32_768_u32.to_le_bytes());
        }
        let mut rows = Vec::new();
        assert_eq!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut rows),
            Outcome::Unavailable {
                reason: "upgrade entry unreadable"
            }
        );
        assert_eq!(rows.capacity(), 0);
    }

    #[test]
    fn upgrade_growth_keeps_batches_padding_and_boundary_faults() {
        const COUNT: usize = 65;
        let mut bytes = fixture();
        bytes.resize(0x400 + COUNT * equipment_upgrade_entry::STRIDE, 0);
        bytes[0x70..0x78].copy_from_slice(&0x1400_u64.to_le_bytes());
        bytes[0x78..0x7c].copy_from_slice(&(COUNT as u32).to_le_bytes());
        bytes[0x7c..0x80].copy_from_slice(&(COUNT as u32).to_le_bytes());
        for index in 0..COUNT {
            let start = 0x400 + index * equipment_upgrade_entry::STRIDE;
            bytes[start..start + 8].copy_from_slice(&0x1200_u64.to_le_bytes());
            bytes[start + 8] = index as u8;
        }
        struct NoPadding<'a>(&'a [u8]);
        impl Memory for NoPadding<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                for index in 0..COUNT {
                    let padding = 0x1400 + index * equipment_upgrade_entry::STRIDE + 9;
                    if address < padding + 7 && address.saturating_add(output.len()) > padding {
                        return false;
                    }
                }
                memory(self.0).copy(address, output)
            }
        }
        let variants = [source(&bytes)];
        let mut rows = vec![Row::default(); COUNT];
        let memory = NoPadding(&bytes);
        let census = ReadCensus::new(&memory);
        assert_eq!(
            walk(&census, 0, &variants, &mut rows),
            Outcome::Rows { len: COUNT }
        );
        assert_eq!((rows[64].source_ordinal, rows[64].level), (64, 64));
        let counts = (census.calls(), census.spans(), census.bytes());
        assert_eq!(counts, (5, 1 + COUNT * 2, 24 + COUNT * 21));
        for initial in [0, 64] {
            let mut growing_rows = vec![Row::default(); initial];
            census.reset();
            assert_eq!(
                walk(&census, 0, &variants, &mut growing_rows),
                Outcome::Rows { len: COUNT }
            );
            assert_eq!(growing_rows, rows);
            assert_eq!((census.calls(), census.spans(), census.bytes()), counts);
        }
        bytes[0x400 + 64 * equipment_upgrade_entry::STRIDE
            ..0x408 + 64 * equipment_upgrade_entry::STRIDE]
            .fill(0);
        assert_eq!(
            walk(&NoPadding(&bytes), 0, &variants, &mut Vec::new()),
            Outcome::Unavailable {
                reason: "upgrade definition pointer null or invalid"
            }
        );
    }

    #[test]
    fn upgrade_resolves_definition_and_native_byte_without_padding_or_name() {
        let mut bytes = fixture();
        bytes[0x218..0x238].fill(0xff);
        let mut rows = vec![Row::default(); 1];
        assert_eq!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            (rows[0].variant, rows[0].definition_token, rows[0].level),
            (0x8000_0000_0000_0046, i32::MIN, 255)
        );
    }

    #[test]
    fn upgrade_growth_retains_rows_and_invalid_identity_does_not_become_empty() {
        let mut bytes = fixture();
        assert_eq!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut Vec::new()),
            Outcome::Rows { len: 1 }
        );
        bytes[0x200] ^= 1;
        assert!(matches!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut Vec::new()),
            Outcome::Unavailable { .. }
        ));
        bytes[0x7c..0x80].fill(0);
        assert_eq!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut Vec::new()),
            Outcome::Rows { len: 0 }
        );
        bytes[0x68] ^= 1;
        assert!(matches!(
            walk(&memory(&bytes), 0, &[source(&bytes)], &mut Vec::new()),
            Outcome::Unavailable { .. }
        ));
        bytes[0x68] ^= 1;
        bytes[0x7c..0x80].copy_from_slice(&1_u32.to_le_bytes());
        let blocked = Blocked(&bytes, 0x1200 + 11);
        assert_eq!(
            walk(&blocked, 0, &[source(&bytes)], &mut Vec::new()),
            Outcome::Unavailable {
                reason: "upgrade definition identity unreadable"
            }
        );
    }

    struct Blocked<'a>(&'a [u8], usize);

    impl Memory for Blocked<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            !(address..address + output.len()).contains(&self.1)
                && memory(self.0).copy(address, output)
        }
    }
}
