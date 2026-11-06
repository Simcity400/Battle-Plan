//! Retained naval-hit entry: target slot/name, convoy flag, accumulated damage,
//! denominator and final-hit flag. The denominator is distinct from a hit count.
use crate::capture_text::Value;
use crate::{Memory, cnaval_combatant::field};
use engine_sites::Rva;

/// Naval-hit entry primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a5_6f88);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Row {
    pub target_slot: u32,
    /// Stored MSVC string copied using its full length.
    pub target_name: crate::capture_text::Owned,
    pub convoy: u8,
    pub damage: i64,
    pub denominator: i64,
    pub last_hit: u8,
}
impl Row {
    pub fn allocated_bytes(&self) -> usize {
        self.target_name.allocated_bytes()
    }
}

/// The hit's fields [`read`] reads (vtable, target slot, target name
/// header, convoy, damage and denominator, last hit), one record per hit for
/// a run of hits (`cfex_member::append_entries`).
pub const RECORD: [crate::memory::RecordField; 5] = {
    use crate::memory::RecordField;
    [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 12,
        },
        RecordField {
            source_offset: 0x10,
            destination_offset: 12,
            len: crate::msvc_string::STRIDE,
        },
        RecordField {
            source_offset: 0x30,
            destination_offset: 44,
            len: 1,
        },
        RecordField {
            source_offset: 0x38,
            destination_offset: 45,
            len: 16,
        },
        RecordField {
            source_offset: 0x48,
            destination_offset: 61,
            len: 1,
        },
    ]
};

pub fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, VTABLE_RVA).ok_or("naval hit module withheld")? as u64
    {
        return Err("naval hit vtable mismatch");
    }
    Ok(Row {
        target_slot: field(memory, pointer, 8)?,
        target_name: crate::capture_text::Owned::read(
            memory,
            pointer.checked_add(0x10).ok_or("naval hit text overflow")?,
            false,
        )
        .map_err(|reason| {
            if reason == "string allocation failed" {
                reason
            } else {
                "naval hit target name invalid or unreadable"
            }
        })?,
        convoy: field(memory, pointer, 0x30)?,
        damage: field(memory, pointer, 0x38)?,
        denominator: field(memory, pointer, 0x40)?,
        last_hit: field(memory, pointer, 0x48)?,
    })
}
