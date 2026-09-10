//! CEquipmentUpgradesInstance, pinned a729; embedded at CEquipmentVariant+68.

use crate::{
    Memory, cequipment_upgrade,
    cequipment_variant::{self, Literal, TextOutcome, TextSpan, complete, text_failure},
    cpdx_array::Array,
    equipment_upgrade_entry,
    memory::RecordField,
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
pub fn walk(
    memory: &impl Memory,
    base: usize,
    variants: &[cequipment_variant::RegistryRow],
    output: &mut [Row],
    bytes: &mut [u8],
) -> TextOutcome {
    let result = (|| {
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("upgrade module address overflow")?;
        let mut len = 0;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        let definition_vtable = base
            .checked_add(cequipment_upgrade::VTABLE_RVA)
            .ok_or("upgrade definition module address overflow")?;
        const BATCH: usize = 64;
        const HEADERS: [RecordField; 1] = [RecordField {
            source_offset: cequipment_variant::UPGRADES,
            destination_offset: 0,
            len: 24,
        }];
        const ENTRIES_LAYOUT: [RecordField; 1] = [RecordField {
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
            let copied =
                memory.copy_records(&pointers[..sources.len()], &HEADERS, 24, &mut headers);
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
                        &entry_pointers[..count],
                        &ENTRIES_LAYOUT,
                        9,
                        &mut entries,
                    );
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
                        &definitions[..entry_copied],
                        &DEFINITIONS,
                        44,
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
                        if len == output.len() || size > bytes.len() - bytes_len {
                            name.validate_terminator(memory)?;
                            unread_rows = unread_rows
                                .checked_add(1)
                                .ok_or("upgrade omitted count overflow")?;
                            continue;
                        }
                        name.copy(memory, &mut bytes[bytes_len..bytes_len + size])?;
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
            outcome: complete(len, unread_rows),
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
    fn upgrade_batches_skip_padding_and_validate_omitted_entries_at_the_boundary() {
        const COUNT: usize = 33;
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
        let mut rows = [Row::default(); COUNT];
        let mut names = [0; COUNT];
        let result = walk(&NoPadding(&bytes), 0, &variants, &mut rows, &mut names);
        assert_eq!(result.outcome, Outcome::Rows { len: COUNT });
        assert_eq!((rows[32].source_ordinal, rows[32].level), (32, 32));
        assert_eq!(names, [b'X'; COUNT]);
        let result = walk(
            &NoPadding(&bytes),
            0,
            &variants,
            &mut rows[..32],
            &mut names[..32],
        );
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 32,
                unread_rows: 1
            }
        );
        assert_eq!((result.bytes_len, result.required_bytes), (32, 33));
        bytes[0x400 + 32 * equipment_upgrade_entry::STRIDE
            ..0x408 + 32 * equipment_upgrade_entry::STRIDE]
            .fill(0);
        assert_eq!(
            walk(
                &NoPadding(&bytes),
                0,
                &variants,
                &mut rows[..32],
                &mut names[..32]
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
        let mut rows = [Row::default(); 1];
        let mut names = [0; 1];
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
    fn upgrade_capacity_reports_omission_and_invalid_identity_does_not_become_empty() {
        let mut bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let result = walk(&memory, 0, &[source(&bytes)], &mut [], &mut []);
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
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
                &mut [Row::default(); 1],
                &mut [0; 1]
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
                &mut [],
                &mut []
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
                &mut [],
                &mut []
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
