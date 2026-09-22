//! CEquipmentUpgradesInstance, pinned a729; embedded at CEquipmentVariant+68.

use crate::{
    Memory, cequipment_upgrade,
    cequipment_variant::{self, Literal, TextOutcome, TextSpan, text_failure},
    cpdx_array::Array,
    equipment_upgrade_entry,
    memory::{RecordField, RecordLayout, RecordSources},
    walk::Outcome,
};

/// CEquipmentUpgradesInstance primary vtable; executable RTTI confirms old AIR_VARIANT_UPGRADE_CONTAINER identity.
pub const VTABLE_RVA: usize = 0x027a_b628;
/// CEquipmentUpgradesInstance entry vector data; read_compact_air_variant_upgrade_entries reads variant+70, instance+8.
pub const ENTRIES: usize = 0x08;
/// CEquipmentUpgradesInstance entry-vector u32 capacity; legacy variant+78 normalizes to instance+10.
pub const ENTRIES_CAPACITY: usize = 0x10;
/// CEquipmentUpgradesInstance entry-vector u32 count; legacy variant+7C normalizes to instance+14.
pub const ENTRIES_COUNT: usize = 0x14;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub variant: u64,
    pub source_ordinal: u32,
    pub definition_token: i32,
    pub level: u8,
    pub definition_name: TextSpan,
}

/// Reads the full native upgrade identity and byte level. Pointer values and
/// entry padding never escape into replay fields. Names use the caller's arena.
/// `variants` must be the same complete registry census used for variant
/// definitions. Missing source tables cannot be represented by a shorter slice.
/// Destinations grow after bounded payload reads; callers must permit allocation.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    variants: &[cequipment_variant::RegistryRow],
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
) -> TextOutcome {
    let result = (|| {
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("upgrade module address overflow")?;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let definition_vtable = base
            .checked_add(cequipment_upgrade::VTABLE_RVA)
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
        const DEFINITIONS: [RecordField; 2] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 12,
            },
            RecordField {
                source_offset: cequipment_upgrade::NAME,
                destination_offset: 12,
                len: 32,
            },
        ];
        const HEADERS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&HEADERS, 24).expect("valid record layout");
        const ENTRIES_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&ENTRIES_FIELDS, 9).expect("valid record layout");
        const DEFINITIONS_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&DEFINITIONS, 44).expect("valid record layout");
        let mut pointers = [0; BATCH];
        let mut headers = [0; BATCH * 24];
        let mut entries = [0; BATCH * 9];
        let mut entry_pointers = [0; BATCH];
        let mut definitions = [0; BATCH];
        let mut definition_fields = [0; BATCH * 44];
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
                        output
                            .try_reserve(required - output.len())
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
                        if definition_copied < (slot + 1) * DEFINITIONS.len() {
                            return Err("upgrade definition identity or name unreadable");
                        }
                        let entry = &entries[slot * 9..(slot + 1) * 9];
                        let field = &definition_fields[slot * 44..(slot + 1) * 44];
                        if u64::from_le_bytes(field[..8].try_into().unwrap())
                            != definition_vtable as u64
                        {
                            return Err("upgrade definition vtable mismatch");
                        }
                        let definition_token = i32::from_le_bytes(field[8..12].try_into().unwrap());
                        let name = Literal::from_header(field[12..].try_into().unwrap())?;
                        let size = name.len;
                        required_bytes = required_bytes
                            .checked_add(size)
                            .ok_or("upgrade name size overflow")?;
                        name.copy_growing(
                            memory,
                            bytes,
                            bytes_len,
                            "upgrade name allocation failed",
                        )?;
                        output[len] = Row {
                            variant,
                            source_ordinal: first + slot as u32,
                            definition_token,
                            level: entry[8],
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
    use crate::{memory::SliceMemory, read_census::ReadCensus};

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
        bytes[8..16].copy_from_slice(&0x8000_0000_0000_0046_u64.to_le_bytes());
        bytes[0x68..0x70].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
        bytes[0x70..0x78].copy_from_slice(&0x1180_u64.to_le_bytes());
        bytes[0x78..0x7c].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x7c..0x80].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x180..0x188].copy_from_slice(&0x1200_u64.to_le_bytes());
        bytes[0x188] = 255;
        bytes[0x189..0x190].fill(0xce);
        bytes[0x200..0x208].copy_from_slice(&(cequipment_upgrade::VTABLE_RVA as u64).to_le_bytes());
        bytes[0x208..0x20c].copy_from_slice(&i32::MIN.to_le_bytes());
        bytes[0x218] = b'X';
        bytes[0x228..0x230].copy_from_slice(&1_u64.to_le_bytes());
        bytes[0x230..0x238].copy_from_slice(&15_u64.to_le_bytes());
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
                reason: "upgrade entry unreadable"
            }
        );
        assert_eq!(rows.capacity(), 0);
        assert_eq!(result.required_bytes, 0);
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
                SliceMemory {
                    base: 0x1000,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
        let variants = [source(&bytes)];
        let mut rows = vec![Row::default(); COUNT];
        let mut names = vec![0; COUNT];
        let memory = NoPadding(&bytes);
        let census = ReadCensus::new(&memory);
        let result = walk(&census, 0, &variants, &mut rows, &mut names);
        assert_eq!(result.outcome, Outcome::Rows { len: COUNT });
        assert_eq!((rows[64].source_ordinal, rows[64].level), (64, 64));
        assert_eq!(names, [b'X'; COUNT]);
        let counts = (census.calls(), census.spans(), census.bytes());
        assert_eq!(counts, (5, 1 + COUNT * 3, 24 + COUNT * 53));
        for initial in [0, 64] {
            let mut growing_rows = vec![Row::default(); initial];
            let mut growing_names = vec![0; initial];
            census.reset();
            let result = walk(&census, 0, &variants, &mut growing_rows, &mut growing_names);
            assert_eq!(result.outcome, Outcome::Rows { len: COUNT });
            assert_eq!((result.bytes_len, result.required_bytes), (COUNT, COUNT));
            assert_eq!(growing_rows, rows);
            assert_eq!(growing_names, names);
            assert_eq!((census.calls(), census.spans(), census.bytes()), counts);
        }
        bytes[0x400 + 64 * equipment_upgrade_entry::STRIDE
            ..0x408 + 64 * equipment_upgrade_entry::STRIDE]
            .fill(0);
        assert_eq!(
            walk(
                &NoPadding(&bytes),
                0,
                &variants,
                &mut Vec::new(),
                &mut Vec::new()
            )
            .outcome,
            Outcome::Unavailable {
                reason: "upgrade definition pointer null or invalid"
            }
        );
    }

    #[test]
    fn upgrade_resolves_definition_and_native_byte_without_padding() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 1];
        let mut names = vec![0; 1];
        assert_eq!(
            walk(&memory, 0, &[source(&bytes)], &mut rows, &mut names).outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            (rows[0].variant, rows[0].definition_token, rows[0].level),
            (0x8000_0000_0000_0046, i32::MIN, 255)
        );
        assert_eq!(names, [b'X']);
    }

    #[test]
    fn heap_name_growth_keeps_occurrences_costs_and_first_faults() {
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            blocked: Option<(usize, usize, usize)>,
            seen: std::cell::Cell<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if let Some((blocked, len, occurrence)) = self.blocked
                    && address == blocked
                    && output.len() == len
                {
                    self.seen.set(self.seen.get() + 1);
                    if self.seen.get() == occurrence {
                        return false;
                    }
                }
                self.memory.copy(address, output)
            }
        }
        let mut bytes = fixture();
        bytes.resize(0x440, 0);
        bytes[0x78..0x7c].copy_from_slice(&2_u32.to_le_bytes());
        bytes[0x7c..0x80].copy_from_slice(&2_u32.to_le_bytes());
        bytes[0x190..0x198].copy_from_slice(&0x1200_u64.to_le_bytes());
        bytes[0x198] = 7;
        bytes[0x218..0x220].copy_from_slice(&0x1400_u64.to_le_bytes());
        bytes[0x228..0x230].copy_from_slice(&20_u64.to_le_bytes());
        bytes[0x230..0x238].copy_from_slice(&31_u64.to_le_bytes());
        bytes[0x400..0x414].copy_from_slice(b"abcdefghijklmnopqrst");
        let variants = [source(&bytes)];
        for (blocked, expected) in [
            (None, Outcome::Rows { len: 2 }),
            (
                Some((0x1200, 12, 1)),
                Outcome::Unavailable {
                    reason: "upgrade definition identity or name unreadable",
                },
            ),
            (
                Some((0x1400, 20, 1)),
                Outcome::Unavailable {
                    reason: "equipment string bytes or terminator unreadable",
                },
            ),
            (
                Some((0x1414, 1, 1)),
                Outcome::Unavailable {
                    reason: "equipment string bytes or terminator unreadable",
                },
            ),
            (
                Some((0x1400, 20, 2)),
                Outcome::Unavailable {
                    reason: "equipment string bytes or terminator unreadable",
                },
            ),
        ] {
            let mut expected_counts = None;
            for initial in [40, 0, 20] {
                let memory = Fault {
                    memory: SliceMemory {
                        base: 0x1000,
                        bytes: &bytes,
                    },
                    blocked,
                    seen: std::cell::Cell::new(0),
                };
                let census = ReadCensus::new(&memory);
                let mut rows = vec![Row::default(); usize::from(initial == 40) * 2];
                let mut names = vec![0; initial];
                let result = walk(&census, 0, &variants, &mut rows, &mut names);
                assert_eq!(result.outcome, expected);
                let counts = (census.calls(), census.spans(), census.bytes());
                assert_eq!(counts, *expected_counts.get_or_insert(counts));
                assert_eq!(rows.len(), 2);
                if blocked.is_none() {
                    assert_eq!(counts, (5, 11, 172));
                    assert_eq!((result.bytes_len, result.required_bytes), (40, 40));
                    assert_eq!(
                        names.as_slice(),
                        b"abcdefghijklmnopqrstabcdefghijklmnopqrst"
                    );
                    assert_eq!((rows[0].source_ordinal, rows[1].source_ordinal), (0, 1));
                    assert_eq!((rows[0].level, rows[1].level), (255, 7));
                } else {
                    assert_eq!((result.bytes_len, result.required_bytes), (0, 0));
                }
            }
        }
        // The batched definition gather observes this later invalid pointer,
        // but the earlier entry's payload failure still has precedence.
        bytes[0x190..0x198].fill(0);
        let memory = Fault {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            blocked: Some((0x1400, 20, 1)),
            seen: std::cell::Cell::new(0),
        };
        assert_eq!(
            walk(&memory, 0, &variants, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "equipment string bytes or terminator unreadable"
            }
        );
    }

    #[test]
    fn upgrade_growth_retains_rows_and_invalid_identity_does_not_become_empty() {
        let mut bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let result = walk(
            &memory,
            0,
            &[source(&bytes)],
            &mut Vec::new(),
            &mut Vec::new(),
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.required_bytes, 1);
        bytes[0x200] ^= 1;
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                &[source(&bytes)],
                &mut Vec::new(),
                &mut Vec::new()
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
        bytes[0x7c..0x80].fill(0);
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                &[source(&bytes)],
                &mut Vec::new(),
                &mut Vec::new()
            )
            .outcome,
            Outcome::Rows { len: 0 }
        );
        bytes[0x68] ^= 1;
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                &[source(&bytes)],
                &mut Vec::new(),
                &mut Vec::new()
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
