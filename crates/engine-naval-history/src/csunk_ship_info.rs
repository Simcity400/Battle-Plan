
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029d_4788);
pub const SIZE: usize = 0xa8;
pub const SHIP_NAME: usize = 0x08;
pub const ATTRIBUTION_SOURCE_NAME: usize = 0x28;
pub const VICTIM_COUNTRY: usize = 0x48;
pub const ATTACKER_COUNTRY: usize = 0x4c;
pub const SINK_DATE: usize = 0x50;
pub const SINK_HOUR: usize = 0x58;
pub const SINK_DATE_PERSISTENT: usize = 0x60;
pub const SHIP_CLASS: usize = 0x68;
pub const ATTRIBUTION_CLASS: usize = 0x70;
pub const MODEL_LEVEL: usize = 0x78;
pub const VICTIM_EQUIPMENT_VARIANT: usize = 0x7c;
pub const DAMAGE_SOURCE_REFERENCE: usize = 0x84;
pub const PROVINCE: usize = 0x90;
pub const BATTLE: usize = 0x98;
pub const CAUSE_RECORD: usize = 0xa0;
pub const ASSISTED_KILL: usize = 0xa1;
#[cfg(test)]
pub const SAVE_RVA: usize = 0x00c8_4ef0;

use engine_native_memory::{
    Memory,
    cgame_date,
    cpdx_array::Array,
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::{cgame_state, cprovince};
use engine_native_parts::csub_unit_definition;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub ship_name: TextRange,
    pub attribution_name: TextRange,
    pub class_name: TextRange,
    pub attribution_class_name: TextRange,
    pub victim_country: u32,
    pub attacker_country: u32,
    pub sink_hour: i32,
    pub class_token: u32,
    pub attribution_class_token: u32,
    pub attribution_class_present: u8,
    pub model_level: i32,
    pub victim_equipment_variant: u64,
    pub damage_source_reference: u64,
    pub province: u32,
    pub province_present: u8,
    pub battle: u64,
    pub cause_record: u8,
    pub assisted_kill: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    current_hour: i32,
    output: &mut Vec<Record>,
    bytes: &mut Vec<u8>,
) -> ListWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        current_hour,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(bytes),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    current_hour: i32,
    mut output: engine_native_memory::walk::Buffer<'_, Record>,
    mut bytes: engine_native_memory::walk::Buffer<'_, u8>,
) -> ListWalk {
    let result = (|| {
        if game_state == 0 || current_hour <= 0 {
            return Err("sunk-ship history context invalid");
        }
        let header = game_state
            .checked_add(cgame_state::SUNK_SHIP_HISTORY)
            .ok_or("sunk-ship history address overflow")?;
        let array = Array::read(memory, header, 8).ok_or("sunk-ship history array invalid")?;
        output.ensure(array.count as usize)?;
        let mut bytes_len = 0_usize;
        for index in 0..array.count {
            let len = index as usize;
            if len == output.len() {
                return Ok(ListWalk {
                    outcome: Outcome::Truncated {
                        len,
                        unread_rows: array.count - index,
                    },
                    bytes_len,
                    required_bytes: bytes_len,
                });
            }
            let slot = array.element(index, 8).ok_or("sunk-ship slot overflow")?;
            let pointer = read::<u64>(memory, slot)
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("sunk-ship history pointer unreadable or null")?;
            match read_record(
                memory,
                base,
                pointer,
                Some(current_hour),
                &mut bytes,
                &mut bytes_len,
            )? {
                RecordRead::Ready(record) => output[len] = record,
                RecordRead::NeedsBytes(required_bytes) => {
                    return Ok(ListWalk {
                        outcome: Outcome::Truncated {
                            len,
                            unread_rows: array.count - index,
                        },
                        bytes_len,
                        required_bytes,
                    });
                }
            }
        }
        Ok(ListWalk {
            outcome: Outcome::Rows {
                len: array.count as usize,
            },
            bytes_len,
            required_bytes: bytes_len,
        })
    })();
    result.unwrap_or_else(|reason| ListWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

enum RecordRead {
    Ready(Record),
    NeedsBytes(usize),
}

pub fn read_one(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    bytes: &mut Vec<u8>,
) -> Result<(Record, usize), &'static str> {
    let mut used = 0;
    match read_record(
        memory,
        base,
        pointer,
        None,
        &mut engine_native_memory::walk::Buffer::Growing(bytes),
        &mut used,
    )? {
        RecordRead::Ready(record) => Ok((record, used)),
        RecordRead::NeedsBytes(_) => Err("embedded sunk-ship text capacity exhausted"),
    }
}

fn read_record(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    current_hour: Option<i32>,
    bytes: &mut engine_native_memory::walk::Buffer<'_, u8>,
    bytes_len: &mut usize,
) -> Result<RecordRead, &'static str> {
    let mut used = *bytes_len;
    let raw = read::<[u8; SIZE]>(memory, pointer).ok_or("sunk-ship history record unreadable")?;
    if u64_at(&raw, 0)
        != engine_sites::at(base, VTABLE_RVA)
            .ok_or("module address overflow")? as u64
    {
        return Err("sunk-ship history vtable mismatch");
    }
    if u64_at(&raw, SINK_DATE)
        != engine_sites::at(base, cgame_date::VTABLE_RVA)
            .ok_or("module address overflow")? as u64
        || u64_at(&raw, SINK_DATE_PERSISTENT)
            != engine_sites::at(base, cgame_date::PERSISTENT_VTABLE_RVA)
                .ok_or("module address overflow")? as u64
    {
        return Err("sunk-ship history date vtable mismatch");
    }
    let sink_hour = u32_at(&raw, SINK_HOUR) as i32;
    if current_hour.is_some_and(|hour| sink_hour <= 0 || sink_hour > hour) {
        return Err("sunk-ship history date outside recorded hour");
    }
    if raw[CAUSE_RECORD] > 1 || raw[ASSISTED_KILL] > 1 {
        return Err("sunk-ship history boolean invalid");
    }
    let (class_token, class_text) = class(memory, base, u64_at(&raw, SHIP_CLASS))?;
    let attribution = u64_at(&raw, ATTRIBUTION_CLASS);
    let (attribution_class_token, attribution_text) = if attribution == 0 {
        (0, Source::default())
    } else {
        class(memory, base, attribution)?
    };
    let sources = [
        string(memory, pointer + SHIP_NAME, &raw[SHIP_NAME..VICTIM_COUNTRY])?,
        string(
            memory,
            pointer + ATTRIBUTION_SOURCE_NAME,
            &raw[ATTRIBUTION_SOURCE_NAME..VICTIM_COUNTRY],
        )?,
        class_text,
        attribution_text,
    ];
    let required_bytes = sources.iter().try_fold(used, |total, source| {
        total
            .checked_add(source.len)
            .ok_or("sunk-ship text size overflow")
    })?;
    bytes.ensure(required_bytes)?;
    if required_bytes > bytes.len() {
        return Ok(RecordRead::NeedsBytes(required_bytes));
    }
    let mut ranges = [TextRange::default(); 4];
    for (source, range) in sources.into_iter().zip(&mut ranges) {
        if !read_slice(memory, source.pointer, &mut bytes[used..used + source.len]) {
            return Err("sunk-ship text bytes unreadable");
        }
        *range = TextRange {
            start: used,
            len: source.len,
        };
        used += source.len;
    }
    let province_pointer = u64_at(&raw, PROVINCE);
    let province = if province_pointer == 0 {
        0
    } else {
        let province_pointer =
            usize::try_from(province_pointer).map_err(|_| "sunk-ship province pointer overflow")?;
        if read::<u64>(memory, province_pointer)
            != Some(
                engine_sites::at(base, cprovince::VTABLE_RVA)
                    .ok_or("module address overflow")? as u64,
            )
        {
            return Err("sunk-ship province vtable mismatch");
        }
        province_pointer
            .checked_add(cprovince::ID)
            .and_then(|p| read::<u32>(memory, p))
            .ok_or("sunk-ship province id unreadable")?
    };
    let record = Record {
        ship_name: ranges[0],
        attribution_name: ranges[1],
        class_name: ranges[2],
        attribution_class_name: ranges[3],
        victim_country: u32_at(&raw, VICTIM_COUNTRY),
        attacker_country: u32_at(&raw, ATTACKER_COUNTRY),
        sink_hour,
        class_token,
        attribution_class_token,
        attribution_class_present: u8::from(attribution != 0),
        model_level: u32_at(&raw, MODEL_LEVEL) as i32,
        victim_equipment_variant: u64_at(&raw, VICTIM_EQUIPMENT_VARIANT),
        damage_source_reference: u64_at(&raw, DAMAGE_SOURCE_REFERENCE),
        province,
        province_present: u8::from(province_pointer != 0),
        battle: u64_at(&raw, BATTLE),
        cause_record: raw[CAUSE_RECORD],
        assisted_kill: raw[ASSISTED_KILL],
    };
    *bytes_len = used;
    Ok(RecordRead::Ready(record))
}

pub fn storage_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<(usize, usize), &'static str> {
    let header = game_state
        .checked_add(cgame_state::SUNK_SHIP_HISTORY)
        .ok_or("sunk-ship history address overflow")?;
    let array = Array::read(memory, header, 8).ok_or("sunk-ship history array invalid")?;
    let mut bytes = 0_usize;
    engine_native_memory::cpdx_array::pointers(
        memory,
        array,
        "sunk-ship history pointer unreadable",
        |_, pointer| {
            if pointer == 0 {
                return Err("sunk-ship history pointer unreadable or null");
            }
            let mut identity = [0_u8; 8];
            let mut strings = [0_u8; 64];
            let mut classes = [0_u8; 16];
            if memory.copy_scattered(
                [
                    pointer,
                    pointer
                        .checked_add(SHIP_NAME)
                        .ok_or("sunk-ship history address overflow")?,
                    pointer
                        .checked_add(SHIP_CLASS)
                        .ok_or("sunk-ship history address overflow")?,
                ],
                &mut [&mut identity, &mut strings, &mut classes],
            ) != 3
            {
                return Err("sunk-ship history sizing fields unreadable");
            }
            if u64::from_le_bytes(identity)
                != engine_sites::at(base, VTABLE_RVA)
                    .ok_or("module address overflow")? as u64
            {
                return Err("sunk-ship history vtable mismatch");
            }
            let mut sources = [
                string(memory, pointer + SHIP_NAME, &strings[..32])?,
                string(memory, pointer + ATTRIBUTION_SOURCE_NAME, &strings[32..])?,
                class(memory, base, u64_at(&classes, 0))?.1,
                Source::default(),
            ];
            if u64_at(&classes, 8) != 0 {
                sources[3] = class(memory, base, u64_at(&classes, 8))?.1;
            }
            for source in sources {
                bytes = bytes
                    .checked_add(source.len)
                    .ok_or("sunk-ship text size overflow")?;
            }
            Ok(())
        },
    )?;
    Ok((array.count as usize, bytes))
}

#[derive(Default, Clone, Copy)]
struct Source {
    pointer: usize,
    len: usize,
}

fn class(memory: &impl Memory, base: usize, pointer: u64) -> Result<(u32, Source), &'static str> {
    let pointer = usize::try_from(pointer).map_err(|_| "sunk-ship class address overflow")?;
    let header = read::<[u8; csub_unit_definition::NAME + msvc_string::STRIDE]>(memory, pointer)
        .ok_or("sunk-ship class unreadable")?;
    let vtable = u64_at(&header, 0);
    if vtable
        != engine_sites::at(base, csub_unit_definition::VTABLE_RVA)
            .ok_or("module address overflow")? as u64
        && vtable
            != engine_sites::at(base, csub_unit_definition::NULL_VTABLE_RVA)
                .ok_or("module address overflow")? as u64
    {
        return Err("sunk-ship class vtable mismatch");
    }
    Ok((
        u32_at(&header, csub_unit_definition::TOKEN),
        string(
            memory,
            pointer + csub_unit_definition::NAME,
            &header[csub_unit_definition::NAME..],
        )?,
    ))
}

fn string(memory: &impl Memory, address: usize, header: &[u8]) -> Result<Source, &'static str> {
    let len = u64_at(header, msvc_string::LENGTH);
    let capacity = u64_at(header, msvc_string::CAPACITY);
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("sunk-ship text header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "sunk-ship text length overflow")?;
    let pointer = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        usize::try_from(u64_at(header, msvc_string::DATA))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("sunk-ship text pointer invalid")?
    };
    let terminator = pointer
        .checked_add(len)
        .ok_or("sunk-ship text address overflow")?;
    if read::<u8>(memory, terminator) != Some(0) {
        return Err("sunk-ship text terminator invalid");
    }
    Ok(Source { pointer, len })
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed native field"),
    )
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed native field"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const GAME: usize = 0x100;
    const VECTOR: usize = 0x800;
    const FIRST: usize = 0x900;
    const SECOND: usize = 0xb00;
    const CLASS: usize = 0xd00;
    const NULL_CLASS: usize = 0xe00;
    const MAP_PROVINCE: usize = 0xf00;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn short(bytes: &mut [u8], at: usize, value: &[u8]) {
        put(bytes, at, value);
        put(
            bytes,
            at + msvc_string::LENGTH,
            &(value.len() as u64).to_le_bytes(),
        );
        put(bytes, at + msvc_string::CAPACITY, &15_u64.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x2000];
        put(
            &mut bytes,
            GAME + cgame_state::SUNK_SHIP_HISTORY,
            &(VECTOR as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            GAME + cgame_state::SUNK_SHIP_HISTORY_CAPACITY,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            GAME + cgame_state::SUNK_SHIP_HISTORY_COUNT,
            &2_u32.to_le_bytes(),
        );
        put(&mut bytes, VECTOR, &(FIRST as u64).to_le_bytes());
        put(&mut bytes, VECTOR + 8, &(SECOND as u64).to_le_bytes());
        put(
            &mut bytes,
            CLASS,
            &(csub_unit_definition::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            CLASS + csub_unit_definition::TOKEN,
            &123_u32.to_le_bytes(),
        );
        short(&mut bytes, CLASS + csub_unit_definition::NAME, b"destroyer");
        put(
            &mut bytes,
            NULL_CLASS,
            &(csub_unit_definition::NULL_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        short(&mut bytes, NULL_CLASS + csub_unit_definition::NAME, b"");
        put(
            &mut bytes,
            MAP_PROVINCE,
            &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MAP_PROVINCE + cprovince::ID,
            &0x8000_1234_u32.to_le_bytes(),
        );
        for (pointer, name) in [(FIRST, b"First".as_slice()), (SECOND, b"Second".as_slice())] {
            put(&mut bytes, pointer, &(VTABLE_RVA.0 as u64).to_le_bytes());
            short(&mut bytes, pointer + SHIP_NAME, name);
            short(&mut bytes, pointer + ATTRIBUTION_SOURCE_NAME, b"");
            put(
                &mut bytes,
                pointer + VICTIM_COUNTRY,
                &0xf123_4567_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + SINK_DATE,
                &(cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + SINK_DATE_PERSISTENT,
                &(cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + SINK_HOUR, &123_i32.to_le_bytes());
            put(
                &mut bytes,
                pointer + SHIP_CLASS,
                &(CLASS as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + ATTRIBUTION_CLASS,
                &(NULL_CLASS as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + MODEL_LEVEL, &i32::MIN.to_le_bytes());
            put(
                &mut bytes,
                pointer + VICTIM_EQUIPMENT_VARIANT,
                &0xf123_4567_0000_0046_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + DAMAGE_SOURCE_REFERENCE,
                &u64::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + PROVINCE,
                &(MAP_PROVINCE as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + BATTLE,
                &0x8765_4321_fedc_ba98_u64.to_le_bytes(),
            );
            bytes[pointer + CAUSE_RECORD] = 1;
        }
        bytes
    }

    fn capture(memory: &[u8], rows: &mut Vec<Record>, arena: &mut Vec<u8>) -> ListWalk {
        walk(
            &SliceMemory {
                base: 0,
                bytes: memory,
            },
            0,
            GAME,
            200,
            rows,
            arena,
        )
    }

    #[test]
    fn experience_embedded_record_keeps_its_own_date_without_global_list_admission() {
        let mut memory = fixture();
        put(&mut memory, FIRST + SINK_HOUR, &0_i32.to_le_bytes());
        assert!(matches!(
            capture(&memory, &mut vec![Record::default(); 2], &mut vec![0; 128]).outcome,
            Outcome::Unavailable {
                reason: "sunk-ship history date outside recorded hour"
            }
        ));
        let source = SliceMemory {
            base: 0,
            bytes: &memory,
        };
        let mut text = Vec::new();
        let (record, len) = read_one(&source, 0, FIRST, &mut text).unwrap();
        assert_eq!(record.sink_hour, 0);
        assert_eq!(
            &text[record.ship_name.start..record.ship_name.start + record.ship_name.len],
            b"First"
        );
        assert!(len <= text.len());
        put(&mut memory, FIRST, &0_u64.to_le_bytes());
        assert_eq!(
            read_one(
                &SliceMemory {
                    base: 0,
                    bytes: &memory
                },
                0,
                FIRST,
                &mut text
            ),
            Err("sunk-ship history vtable mismatch")
        );
    }

    #[test]
    fn ordered_list_keeps_duplicate_variant_handles_and_native_values() {
        let mut memory = fixture();
        let mut rows = vec![Record::default(); 2];
        let mut arena = vec![0; 128];
        assert_eq!(
            capture(&memory, &mut rows, &mut arena).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            &arena[rows[0].ship_name.start..rows[0].ship_name.start + rows[0].ship_name.len],
            b"First"
        );
        assert_eq!(
            rows[0].victim_equipment_variant,
            rows[1].victim_equipment_variant
        );
        assert_eq!(rows[0].victim_equipment_variant, 0xf123_4567_0000_0046);
        assert_eq!(rows[0].damage_source_reference, u64::MAX);
        assert_eq!(rows[0].model_level, i32::MIN);
        assert_eq!(rows[0].province, 0x8000_1234);
        assert_eq!(rows[0].battle, 0x8765_4321_fedc_ba98);
        put(&mut memory, VECTOR, &(SECOND as u64).to_le_bytes());
        put(
            &mut memory,
            GAME + cgame_state::SUNK_SHIP_HISTORY_COUNT,
            &1_u32.to_le_bytes(),
        );
        assert_eq!(
            capture(&memory, &mut rows, &mut arena).outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(&arena[..rows[0].ship_name.len], b"Second");
    }

    #[test]
    fn long_engine_text_retains_every_byte_without_utf8_conversion() {
        let mut memory = fixture();
        put(&mut memory, FIRST + SHIP_NAME, &0x1800_u64.to_le_bytes());
        put(
            &mut memory,
            FIRST + SHIP_NAME + msvc_string::LENGTH,
            &300_u64.to_le_bytes(),
        );
        put(
            &mut memory,
            FIRST + SHIP_NAME + msvc_string::CAPACITY,
            &300_u64.to_le_bytes(),
        );
        memory[0x1800..0x1800 + 300].fill(0xff);
        let mut rows = vec![Record::default(); 2];
        let mut arena = vec![0; 512];
        assert_eq!(
            capture(&memory, &mut rows, &mut arena).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].ship_name.len, 300);
        assert!(arena[..300].iter().all(|&byte| byte == 0xff));
    }

    #[test]
    fn bad_later_records_invalidate_the_list_instead_of_certifying_a_prefix() {
        for (offset, value) in [
            (ASSISTED_KILL, 2),
            (CAUSE_RECORD, 2),
            (0, 0),
            (SINK_DATE, 0),
        ] {
            let mut memory = fixture();
            memory[SECOND + offset] = value;
            let result = capture(&memory, &mut vec![Record::default(); 2], &mut vec![0; 128]);
            assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
            assert_eq!(result.bytes_len, 0);
        }
        let mut memory = fixture();
        put(&mut memory, SECOND + SINK_HOUR, &201_i32.to_le_bytes());
        assert!(matches!(
            capture(&memory, &mut vec![Record::default(); 2], &mut vec![0; 128]).outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn empty_history_and_optional_nulls_are_explicit() {
        let mut memory = fixture();
        put(&mut memory, FIRST + ATTRIBUTION_CLASS, &0_u64.to_le_bytes());
        put(&mut memory, FIRST + PROVINCE, &0_u64.to_le_bytes());
        let mut rows = vec![Record::default(); 2];
        assert_eq!(
            capture(&memory, &mut rows, &mut vec![0; 128]).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            (rows[0].province_present, rows[0].attribution_class_present),
            (0, 0)
        );
        put(
            &mut memory,
            GAME + cgame_state::SUNK_SHIP_HISTORY_COUNT,
            &0_u32.to_le_bytes(),
        );
        assert_eq!(
            capture(&memory, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn growing_sunk_history_retains_every_native_occurrence_and_full_text() {
        let mut bytes = fixture();
        let data = 0x10000;
        let mut rows = Vec::new();
        let mut names = Vec::new();
        for count in [5_000_u32, 12_000] {
            bytes.resize(data + count as usize * 8, 0);
            put(
                &mut bytes,
                GAME + cgame_state::SUNK_SHIP_HISTORY,
                &(data as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                GAME + cgame_state::SUNK_SHIP_HISTORY + 8,
                &count.to_le_bytes(),
            );
            put(
                &mut bytes,
                GAME + cgame_state::SUNK_SHIP_HISTORY + 12,
                &count.to_le_bytes(),
            );
            for index in 0..count as usize {
                put(&mut bytes, data + index * 8, &(FIRST as u64).to_le_bytes());
            }
            let walked = walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GAME,
                200,
                &mut rows,
                &mut names,
            );
            assert_eq!(
                walked.outcome,
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(walked.bytes_len, count as usize * 14);
            assert_eq!(
                rows[count as usize - 1].ship_name.start,
                (count as usize - 1) * 14
            );
        }
    }
}
