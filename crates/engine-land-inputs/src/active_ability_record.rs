
pub const STRIDE: usize = 0x58;
pub const DEFINITION: usize = 0x00;
pub const COUNTRY: usize = 0x08;
pub const CASTER: usize = 0x10;
#[cfg(test)]
pub const DIVISION_COUNT: usize = 0x18;
pub const STRATEGIC_REGION: usize = 0x20;
pub const ACTIVATION_HOUR: usize = 0x30;
pub const PHASE: usize = 0x48;
pub const ACTIVE: u8 = 0;
pub const COOLDOWN: u8 = 1;
pub const ALLOCATED_COMMAND_POWER: usize = 0x50;

use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};
use crate::cability;
use engine_native::{carmy_leader, cunit_leader};
use engine_native_parts::{cnavy_leader, cstrategic_region};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub scope_country: u32,
    pub token_start: usize,
    pub token_len: usize,
    pub caster: Option<u64>,
    pub strategic_region: Option<u32>,
    pub division_count: i32,
    pub activation_hour: i32,
    pub phase: u8,
    pub allocate_cost: u8,
    pub allocated_command_power: i64,
    pub duration_hours: i32,
    pub cooldown_hours: i32,
    pub target_kind: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    country: u32,
    array_address: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut tokens: engine_native_memory::walk::Buffer<'_, u8>,
) -> TokenWalk {
    let result = (|| {
        let array = Array::read(memory, array_address, STRIDE)
            .ok_or("active ability array unreadable or invalid")?;
        output.ensure(array.count as usize)?;
        let mut len = 0;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let truncated = |len, ordinal: u32, counted: u32, bytes_len, required_bytes| {
            Ok(TokenWalk {
                outcome: Outcome::Truncated {
                    len,
                    unread_rows: array.count - ordinal,
                },
                bytes_len,
                required_bytes: count_tokens(memory, base, array, counted, required_bytes),
            })
        };
        for ordinal in 0..array.count {
            if len == output.len() {
                return truncated(len, ordinal, ordinal, bytes_len, required_bytes);
            }
            let pointer = array
                .element(ordinal, STRIDE)
                .ok_or("active ability record address overflow")?;
            let record = record(memory, pointer)?;
            let definition = definition(memory, base, record.definition)?;
            required_bytes = required_bytes
                .checked_add(definition.token_len)
                .ok_or("active ability token byte count overflow")?;
            tokens.ensure(
                bytes_len
                    .checked_add(definition.token_len)
                    .ok_or("active ability token byte count overflow")?,
            )?;
            if definition.token_len > tokens.len() - bytes_len {
                return truncated(len, ordinal, ordinal + 1, bytes_len, required_bytes);
            }
            let caster = caster(memory, base, record.caster)?;
            let strategic_region = strategic_region(memory, base, record.strategic_region)?;
            let end = bytes_len + definition.token_len;
            match definition.token {
                Token::Inline(bytes) => {
                    tokens[bytes_len..end].copy_from_slice(&bytes[..definition.token_len])
                }
                Token::Heap(source) => {
                    if !read_slice(memory, source, &mut tokens[bytes_len..end]) {
                        return Err("active ability token bytes unreadable");
                    }
                }
            }
            output[len] = Row {
                country,
                ordinal,
                scope_country: record.scope_country,
                token_start: bytes_len,
                token_len: definition.token_len,
                caster,
                strategic_region,
                division_count: record.division_count,
                activation_hour: record.activation_hour,
                phase: record.phase,
                allocate_cost: definition.allocate_cost,
                allocated_command_power: record.allocated_command_power,
                duration_hours: definition.duration_hours,
                cooldown_hours: definition.cooldown_hours,
                target_kind: definition.target_kind,
            };
            bytes_len = end;
            len += 1;
        }
        Ok(TokenWalk {
            outcome: Outcome::Rows { len },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| TokenWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

pub fn demand(
    memory: &impl Memory,
    base: usize,
    array_address: usize,
) -> Result<(usize, usize), &'static str> {
    let array = Array::read(memory, array_address, STRIDE)
        .ok_or("active ability array unreadable or invalid")?;
    let expected = engine_sites::at(base, cability::VTABLE_RVA)
        .ok_or("active ability module address overflow")? as u64;
    let mut bytes = 0_usize;
    for ordinal in 0..array.count {
        let definition = array
            .element(ordinal, STRIDE)
            .and_then(|pointer| pointer.checked_add(DEFINITION))
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("active ability definition pointer unreadable or null")?;
        if read::<u64>(memory, definition) != Some(expected) {
            return Err("active ability definition vtable mismatch");
        }
        let token = definition
            .checked_add(cability::TOKEN)
            .ok_or("active ability token address overflow")?;
        let descriptor = msvc_string::Descriptor::read(memory, token)?;
        bytes = bytes
            .checked_add(descriptor.len)
            .ok_or("active ability token byte count overflow")?;
    }
    Ok((array.count as usize, bytes))
}

fn count_tokens(
    memory: &impl Memory,
    base: usize,
    array: Array,
    first: u32,
    mut required: usize,
) -> usize {
    let Some(expected) = engine_sites::at(base, cability::VTABLE_RVA) else {
        return required;
    };
    let mut last: Option<(usize, usize)> = None;
    for ordinal in first..array.count {
        let Some(definition) = array
            .element(ordinal, STRIDE)
            .and_then(|record| record.checked_add(DEFINITION))
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
        else {
            return required;
        };
        let token_len = match last {
            Some((pointer, token_len)) if pointer == definition => token_len,
            _ => {
                let mut vtable = [0; 8];
                let mut header = [0; msvc_string::STRIDE];
                let Some(token) = definition.checked_add(cability::TOKEN) else {
                    return required;
                };
                if memory.copy_scattered([definition, token], &mut [&mut vtable, &mut header]) != 2
                    || u64::from_le_bytes(vtable) != expected as u64
                {
                    return required;
                }
                let length = u64::from_le_bytes(
                    header[msvc_string::LENGTH..msvc_string::CAPACITY]
                        .try_into()
                        .unwrap(),
                );
                let capacity =
                    u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
                let Some(token_len) = usize::try_from(length)
                    .ok()
                    .filter(|_| length <= capacity && capacity >= msvc_string::INLINE_CAPACITY)
                else {
                    return required;
                };
                last = Some((definition, token_len));
                token_len
            }
        };
        let Some(total) = required.checked_add(token_len) else {
            return required;
        };
        required = total;
    }
    required
}

struct Record {
    definition: usize,
    scope_country: u32,
    caster: usize,
    strategic_region: usize,
    division_count: i32,
    activation_hour: i32,
    phase: u8,
    allocated_command_power: i64,
}

fn record(memory: &impl Memory, pointer: usize) -> Result<Record, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("active ability record address overflow")
    };
    let mut definition_country = [0; 12];
    let mut caster_count = [0; 12];
    let mut target = [0; 8];
    let mut hour = [0; 4];
    let mut phase = [0; 1];
    let mut cost = [0; 8];
    if memory.copy_scattered(
        [
            at(DEFINITION)?,
            at(CASTER)?,
            at(STRATEGIC_REGION)?,
            at(ACTIVATION_HOUR)?,
            at(PHASE)?,
            at(ALLOCATED_COMMAND_POWER)?,
        ],
        &mut [
            &mut definition_country,
            &mut caster_count,
            &mut target,
            &mut hour,
            &mut phase,
            &mut cost,
        ],
    ) != 6
    {
        return Err("active ability record fields unreadable");
    }
    let pointer = |bytes: &[u8]| {
        usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
            .map_err(|_| "active ability reference address overflow")
    };
    Ok(Record {
        definition: pointer(&definition_country[..8])?,
        scope_country: u32::from_le_bytes(definition_country[8..].try_into().unwrap()),
        caster: pointer(&caster_count[..8])?,
        strategic_region: pointer(&target)?,
        division_count: i32::from_le_bytes(caster_count[8..].try_into().unwrap()),
        activation_hour: i32::from_le_bytes(hour),
        phase: phase[0],
        allocated_command_power: i64::from_le_bytes(cost),
    })
}

struct Definition {
    token: Token,
    token_len: usize,
    target_kind: i32,
    allocate_cost: u8,
    duration_hours: i32,
    cooldown_hours: i32,
}

enum Token {
    Inline([u8; 16]),
    Heap(usize),
}

fn definition(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Definition, &'static str> {
    let expected = engine_sites::at(base, cability::VTABLE_RVA)
        .ok_or("active ability module address overflow")? as u64;
    if read::<u64>(memory, pointer) != Some(expected) {
        return Err("active ability definition vtable mismatch");
    }
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("active ability definition address overflow")
    };
    let mut token = [0; msvc_string::STRIDE];
    let mut target_and_cost_flag = [0; 5];
    let mut duration_cooldown = [0; 8];
    if memory.copy_scattered(
        [
            at(cability::TOKEN)?,
            at(cability::TARGET_KIND)?,
            at(cability::DURATION_HOURS)?,
        ],
        &mut [
            &mut token,
            &mut target_and_cost_flag,
            &mut duration_cooldown,
        ],
    ) != 3
    {
        return Err("active ability definition fields unreadable");
    }
    let length = u64::from_le_bytes(
        token[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(token[msvc_string::CAPACITY..].try_into().unwrap());
    if length > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("active ability token header invalid");
    }
    let token_len = usize::try_from(length).map_err(|_| "active ability token length overflow")?;
    let token = if capacity == msvc_string::INLINE_CAPACITY {
        if token[token_len] != 0 {
            return Err("active ability token terminator unreadable or invalid");
        }
        Token::Inline(token[..16].try_into().unwrap())
    } else {
        let source = usize::try_from(u64::from_le_bytes(token[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("active ability token pointer invalid")?;
        let terminator = source
            .checked_add(token_len)
            .ok_or("active ability token address overflow")?;
        if read::<u8>(memory, terminator) != Some(0) {
            return Err("active ability token terminator unreadable or invalid");
        }
        Token::Heap(source)
    };
    Ok(Definition {
        token,
        token_len,
        target_kind: i32::from_le_bytes(target_and_cost_flag[..4].try_into().unwrap()),
        allocate_cost: target_and_cost_flag[cability::ALLOCATE_COST - cability::TARGET_KIND],
        duration_hours: i32::from_le_bytes(duration_cooldown[..4].try_into().unwrap()),
        cooldown_hours: i32::from_le_bytes(duration_cooldown[4..].try_into().unwrap()),
    })
}

fn caster(memory: &impl Memory, base: usize, pointer: usize) -> Result<Option<u64>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let identity =
        read::<[u8; 16]>(memory, pointer).ok_or("active ability caster identity unreadable")?;
    let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
    let known = [
        cunit_leader::VTABLE_RVA,
        carmy_leader::VTABLE_RVA,
        cnavy_leader::VTABLE_RVA,
        engine_native_parts::coperative_leader_layout::VTABLE_RVA,
    ];
    if engine_sites::which(base, vtable as usize, &known).is_none() {
        return Err("active ability caster vtable mismatch");
    }
    let handle = u64::from_le_bytes(identity[cunit_leader::TYPE..].try_into().unwrap());
    if handle as u32 != cunit_leader::REFERENCE_TYPE {
        return Err("active ability caster handle type mismatch");
    }
    Ok(Some(handle))
}

fn strategic_region(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Option<u32>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let identity = read::<[u8; 16]>(memory, pointer)
        .ok_or("active ability strategic region identity unreadable")?;
    let primary = engine_sites::at(base, cstrategic_region::VTABLE_RVA)
        .ok_or("active ability module address overflow")? as u64;
    let secondary = engine_sites::at(base, cstrategic_region::SECONDARY_VTABLE_RVA)
        .ok_or("active ability module address overflow")? as u64;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != primary
        || u64::from_le_bytes(identity[8..].try_into().unwrap()) != secondary
    {
        return Err("active ability strategic region vtable mismatch");
    }
    cstrategic_region::id(memory, base, pointer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const MEMORY_BASE: usize = 0x1000;
    const ARRAY: usize = 0x1100;
    const RECORDS: usize = 0x2000;
    const DEFINITION_A: usize = 0x3000;
    const DEFINITION_B: usize = 0x3200;
    const LEADER: usize = 0x4000;
    const REGION: usize = 0x4100;
    const HEAP_TOKEN: usize = 0x5000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        let start = address - MEMORY_BASE;
        bytes[start..start + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x5000];
        put(&mut bytes, ARRAY, &(RECORDS as u64).to_le_bytes());
        put(&mut bytes, ARRAY + 8, &2_u32.to_le_bytes());
        put(&mut bytes, ARRAY + 12, &2_u32.to_le_bytes());
        for (index, definition) in [DEFINITION_A, DEFINITION_B].into_iter().enumerate() {
            let record = RECORDS + index * STRIDE;
            put(
                &mut bytes,
                record + DEFINITION,
                &(definition as u64).to_le_bytes(),
            );
            put(&mut bytes, record + COUNTRY, &99_u32.to_le_bytes());
            put(
                &mut bytes,
                record + DIVISION_COUNT,
                &(-17_i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                record + ACTIVATION_HOUR,
                &60_789_123_i32.to_le_bytes(),
            );
            put(&mut bytes, record + PHASE, &[index as u8]);
            put(
                &mut bytes,
                record + ALLOCATED_COMMAND_POWER,
                &(-9_007_199_254_740_991_i64).to_le_bytes(),
            );
            put(
                &mut bytes,
                definition,
                &(cability::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, definition + cability::TOKEN, b"ability");
            put(
                &mut bytes,
                definition + cability::TOKEN_LENGTH,
                &7_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + cability::TOKEN_CAPACITY,
                &15_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + cability::TARGET_KIND,
                &2_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + cability::ALLOCATE_COST,
                &[u8::from(index == 0)],
            );
            put(
                &mut bytes,
                definition + cability::DURATION_HOURS,
                &(-1_i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                definition + cability::COOLDOWN_HOURS,
                &i32::MAX.to_le_bytes(),
            );
        }
        put(&mut bytes, RECORDS + CASTER, &(LEADER as u64).to_le_bytes());
        put(
            &mut bytes,
            RECORDS + STRATEGIC_REGION,
            &(REGION as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LEADER,
            &(carmy_leader::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LEADER + cunit_leader::TYPE,
            &((u64::from(u32::MAX) << 32) | u64::from(cunit_leader::REFERENCE_TYPE)).to_le_bytes(),
        );
        put(
            &mut bytes,
            REGION,
            &(cstrategic_region::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            REGION + cstrategic_region::SECONDARY,
            &(cstrategic_region::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            REGION + cstrategic_region::ID,
            &u32::MAX.to_le_bytes(),
        );
        bytes
    }

    fn run(bytes: &[u8], output: &mut Vec<Row>, tokens: &mut Vec<u8>) -> TokenWalk {
        walk_buffer(&SliceMemory {
                base: MEMORY_BASE,
                bytes,
            }, 0, 42, ARRAY, engine_native_memory::walk::Buffer::Growing(output), engine_native_memory::walk::Buffer::Growing(tokens))
    }

    #[test]
    fn preserves_native_values_ordinals_and_absent_references() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 2];
        let mut tokens = vec![0; 14];
        let result = run(&bytes, &mut rows, &mut tokens);
        assert_eq!(
            result,
            TokenWalk {
                outcome: Outcome::Rows { len: 2 },
                bytes_len: 14,
                required_bytes: 14
            }
        );
        assert_eq!(&tokens, b"abilityability");
        assert_eq!(
            rows[0],
            Row {
                country: 42,
                ordinal: 0,
                scope_country: 99,
                token_start: 0,
                token_len: 7,
                caster: Some((u64::from(u32::MAX) << 32) | u64::from(cunit_leader::REFERENCE_TYPE)),
                strategic_region: Some(u32::MAX),
                division_count: -17,
                activation_hour: 60_789_123,
                phase: ACTIVE,
                allocate_cost: 1,
                allocated_command_power: -9_007_199_254_740_991,
                duration_hours: -1,
                cooldown_hours: i32::MAX,
                target_kind: 2,
            }
        );
        assert_eq!(rows[1].ordinal, 1);
        assert_eq!(rows[1].phase, COOLDOWN);
        assert_eq!(rows[1].caster, None);
        assert_eq!(rows[1].strategic_region, None);
        assert_eq!(rows[1].division_count, -17);
        assert_eq!(rows[1].allocate_cost, 0);
        assert_eq!(rows[1].allocated_command_power, -9_007_199_254_740_991);
        assert_eq!(rows[1].token_start, 7);
    }

    #[test]
    fn the_count_probe_counts_every_row_and_token() {
        let bytes = fixture();
        assert_eq!(
            walk_buffer(
                &SliceMemory {
                    base: MEMORY_BASE,
                    bytes: &bytes,
                },
                0,
                42,
                ARRAY,
                engine_native_memory::walk::Buffer::Count,
                engine_native_memory::walk::Buffer::Count,
            ),
            TokenWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 2
                },
                bytes_len: 0,
                required_bytes: 14
            }
        );
        struct Observed<'a>(SliceMemory<'a>, std::cell::RefCell<Vec<usize>>);
        impl Memory for Observed<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.1.borrow_mut().push(address);
                self.0.copy(address, output)
            }
        }
        let observed = Observed(
            SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            },
            Default::default(),
        );
        assert_eq!(
            walk_buffer(&observed, 0, 42, ARRAY, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count).required_bytes,
            14
        );
        let mut expected = vec![ARRAY];
        for (index, definition) in [DEFINITION_A, DEFINITION_B].into_iter().enumerate() {
            expected.extend([
                RECORDS + index * STRIDE + DEFINITION,
                definition,
                definition + cability::TOKEN,
            ]);
        }
        assert_eq!(*observed.1.borrow(), expected);
    }

    #[test]
    fn initializer_values_remain_recorded_without_caster_or_allocation() {
        let mut bytes = fixture();
        for index in 0..2 {
            let record = RECORDS + index * STRIDE;
            put(&mut bytes, record + CASTER, &0_u64.to_le_bytes());
            put(&mut bytes, record + DIVISION_COUNT, &1_i32.to_le_bytes());
            put(
                &mut bytes,
                record + ALLOCATED_COMMAND_POWER,
                &0_i64.to_le_bytes(),
            );
        }
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            run(&bytes, &mut rows, &mut vec![0; 14]).outcome,
            Outcome::Rows { len: 2 }
        );
        assert!(rows.iter().all(|row| row.caster.is_none()
            && row.division_count == 1
            && row.allocated_command_power == 0));
        assert_eq!((rows[0].allocate_cost, rows[1].allocate_cost), (1, 0));
    }

    #[test]
    fn heap_tokens_and_unknown_phase_bytes_remain_exact() {
        let mut bytes = fixture();
        let token = vec![0xff; 300];
        put(
            &mut bytes,
            DEFINITION_A + cability::TOKEN,
            &(HEAP_TOKEN as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION_A + cability::TOKEN_LENGTH,
            &300_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION_A + cability::TOKEN_CAPACITY,
            &511_u64.to_le_bytes(),
        );
        put(&mut bytes, HEAP_TOKEN, &token);
        put(&mut bytes, RECORDS + PHASE, &[255]);
        let mut rows = vec![Row::default(); 2];
        let mut tokens = vec![0; 307];
        assert_eq!(
            run(&bytes, &mut rows, &mut tokens).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].phase, 255);
        assert_eq!(rows[0].token_len, 300);
        assert_eq!(&tokens[..300], token.as_slice());
    }

    #[test]
    fn invalid_definitions_and_foreign_objects_refuse_the_whole_country() {
        for address in [DEFINITION_B, LEADER, REGION] {
            let mut bytes = fixture();
            put(&mut bytes, address, &0_u64.to_le_bytes());
            let result = run(&bytes, &mut vec![Row::default(); 2], &mut vec![0; 14]);
            assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
            assert_eq!(result.bytes_len, 0);
        }
        for address in [
            LEADER + cunit_leader::TYPE,
            REGION + cstrategic_region::SECONDARY,
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &999_u32.to_le_bytes());
            assert!(matches!(
                run(&bytes, &mut vec![Row::default(); 2], &mut vec![0; 14]).outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn empty_arrays_are_rows_but_corrupt_arrays_and_strings_are_unavailable() {
        let mut bytes = fixture();
        put(&mut bytes, ARRAY, &[0; 16]);
        assert_eq!(
            run(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Rows { len: 0 }
        );
        for (address, value) in [
            (ARRAY + 12, &(-1_i32).to_le_bytes()[..]),
            (
                DEFINITION_A + cability::TOKEN_LENGTH,
                &16_u64.to_le_bytes()[..],
            ),
            (DEFINITION_B + cability::TOKEN + 7, &[1][..]),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, value);
            assert!(matches!(
                run(&bytes, &mut vec![Row::default(); 2], &mut vec![0; 14]).outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn unreadable_later_record_and_token_faults_never_publish_a_prefix() {
        struct FaultMemory<'a> {
            memory: SliceMemory<'a>,
            hole: std::ops::Range<usize>,
        }
        impl Memory for FaultMemory<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(destination.len()) else {
                    return false;
                };
                if address < self.hole.end && end > self.hole.start {
                    return false;
                }
                self.memory.copy(address, destination)
            }
        }
        let bytes = fixture();
        let phase = RECORDS + STRIDE + PHASE;
        let fault = FaultMemory {
            memory: SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            },
            hole: phase..phase + 1,
        };
        let result = walk_buffer(&fault, 0, 42, ARRAY, engine_native_memory::walk::Buffer::Growing(&mut vec![Row::default(); 2]), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 14]));
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "active ability record fields unreadable"
            }
        );
        assert_eq!(result.bytes_len, 0);
        for (offset, length) in [(DIVISION_COUNT, 4), (ALLOCATED_COMMAND_POWER, 8)] {
            for index in 0..2 {
                let address = RECORDS + index * STRIDE + offset;
                let fault = FaultMemory {
                    memory: SliceMemory {
                        base: MEMORY_BASE,
                        bytes: &bytes,
                    },
                    hole: address..address + length,
                };
                let result = walk_buffer(&fault, 0, 42, ARRAY, engine_native_memory::walk::Buffer::Growing(&mut vec![Row::default(); 2]), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 14]));
                assert_eq!(
                    result.outcome,
                    Outcome::Unavailable {
                        reason: "active ability record fields unreadable",
                    }
                );
            }
        }
        let mut bytes = fixture();
        put(
            &mut bytes,
            DEFINITION_B + cability::TOKEN_CAPACITY,
            &31_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION_B + cability::TOKEN,
            &(HEAP_TOKEN as u64).to_le_bytes(),
        );
        let fault = FaultMemory {
            memory: SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            },
            hole: HEAP_TOKEN..HEAP_TOKEN + 7,
        };
        let result = walk_buffer(&fault, 0, 42, ARRAY, engine_native_memory::walk::Buffer::Growing(&mut vec![Row::default(); 2]), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 14]));
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "active ability token bytes unreadable"
            }
        );
        assert_eq!(result.bytes_len, 0);
    }
}
