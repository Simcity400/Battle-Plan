use engine_native_memory::capture_text::Value;
use engine_native_memory::Memory;
use engine_native::cnaval_combatant::field;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a5_6f88);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Row {
    pub target_slot: u32,
    pub target_name: engine_native_memory::capture_text::Owned,
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

pub const RECORD: [engine_native_memory::memory::RecordField; 5] = {
    use engine_native_memory::memory::RecordField;
    [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 12,
        },
        RecordField {
            source_offset: 0x10,
            destination_offset: 12,
            len: engine_native_memory::msvc_string::STRIDE,
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
        target_name: engine_native_memory::capture_text::Owned::read(
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
