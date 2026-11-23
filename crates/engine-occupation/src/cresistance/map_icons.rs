use super::*;
use engine_native::ccountry;
use crate::{ccountry_occupation_data, ccountry_occupation_status, pdx_robin_hood_table};
use engine_native_memory::{msvc_string, read};
use engine_allocation as allocation;
use engine_sites::Rva;

pub const STORED_TARGET: usize = 0x28;
pub const STATE_RESISTANCE_DEFINITIONS: usize = 0x210;
pub const STATE_COMPLIANCE_DEFINITIONS: usize = 0x228;
pub const CORE_RESISTANCE_DEFINITIONS: usize = 0x58;
pub const CORE_COMPLIANCE_DEFINITIONS: usize = 0x70;
pub const DEFINITION_VTABLE_RVA: Rva = Rva(0x02a3_5550);
pub const DEFINITION_NAME: usize = 0x10;

pub use game_model::occupation_map::Markers;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Inputs {
    pub markers: Option<Markers>,
    pub controller: u32,
    pub receivers: [usize; 4],
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("occupation map pointer invalid or overflow")
}

fn append_definitions(
    memory: &impl Memory,
    base: usize,
    object: usize,
    offset: usize,
    output: &mut Vec<Vec<u8>>,
) -> Result<(), &'static str> {
    let list = Array::read(memory, at(object, offset)?, 8)
        .ok_or("occupation map active-definition list unreadable or invalid")?;
    allocation::reserve(output, list.count as usize)
        .map_err(|_| "occupation map definitions allocation failed")?;
    for index in 0..list.count {
        let definition = list
            .element(index, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("occupation map definition pointer unreadable")?;
        let expected = engine_sites::at(base, DEFINITION_VTABLE_RVA)
            .ok_or("occupation map definition identity mismatch")?;
        if definition == 0 || read::<u64>(memory, definition) != Some(expected as u64) {
            return Err("occupation map definition identity mismatch");
        }
        let mut token = Vec::new();
        let mut len = 0;
        msvc_string::append(
            memory,
            at(definition, DEFINITION_NAME)?,
            &mut token,
            &mut len,
        )?;
        token.truncate(len);
        output.push(token);
    }
    Ok(())
}

fn target(memory: &impl Memory, object: usize, row: Row) -> Result<Option<i64>, &'static str> {
    if row.level == 0 && row.compliance <= 0 {
        Ok(Some(
            read(memory, at(object, STORED_TARGET)?)
                .ok_or("occupation map stored target unreadable")?,
        ))
    } else {
        Ok(None)
    }
}

pub fn read_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    source: cstate::Source,
    row: Row,
) -> Result<Inputs, &'static str> {
    if row.country as i32 <= 0 {
        return Ok(Inputs::default());
    }
    let object = at(source.pointer, cstate::RESISTANCE)?;
    let controller = read::<u32>(memory, at(source.pointer, cstate::CONTROLLER)?)
        .ok_or("occupation map controller unreadable")?;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let resolve = |raw| {
        if raw == 0 || countries.link(raw)?.is_none() {
            countries.reserved()
        } else {
            countries.resolve(raw)
        }
    };
    let controlling = resolve(controller)?;
    let occupied = resolve(row.country)?;
    let holder = ccountry_occupation_status::status(memory, base, controlling.pointer)?;
    let entry = pdx_robin_hood_table::find(
        memory,
        at(holder, ccountry_occupation_status::OCCUPATION_DATA_TABLE)?,
        countries.link(row.country)?.unwrap_or(0),
    )?;
    let data = if let Some(entry) = entry {
        ccountry_occupation_data::identity(memory, base, holder, entry, &countries)?;
        entry.value
    } else {
        0
    };
    let mut markers = Markers {
        stored_target: target(memory, object, row)?,
        ..Default::default()
    };
    for (list, state_offset, core_offset) in [
        (
            &mut markers.resistance,
            STATE_RESISTANCE_DEFINITIONS,
            CORE_RESISTANCE_DEFINITIONS,
        ),
        (
            &mut markers.compliance,
            STATE_COMPLIANCE_DEFINITIONS,
            CORE_COMPLIANCE_DEFINITIONS,
        ),
    ] {
        append_definitions(memory, base, object, state_offset, list)?;
        if data != 0 {
            append_definitions(memory, base, data, core_offset, list)?;
        }
    }
    let retained_country = if data != 0 {
        let pointer = read::<u64>(
            memory,
            at(data, ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER)?,
        )
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("occupation map core scope pointer unreadable")?;
        let expected = engine_sites::at(base, ccountry::VTABLE_RVA)
            .ok_or("occupation map core scope country identity mismatch")?;
        if pointer == 0 || read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("occupation map core scope country identity mismatch");
        }
        markers.core_country = Some(
            read(memory, at(pointer, ccountry::ID)?)
                .ok_or("occupation map core scope country id unreadable")?,
        );
        pointer
    } else {
        0
    };
    Ok(Inputs {
        markers: Some(markers),
        controller,
        receivers: [
            controlling.pointer,
            occupied.pointer,
            holder,
            retained_country,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn occupation_map_captures_inactive_lists_and_core_scope_without_hover_active_gate() {
        let mut bytes = super::super::hover::tests::fixture();
        let source = cstate::Source {
            pointer: 0x6000,
            id: 7,
        };
        let object = source.pointer + cstate::RESISTANCE;
        let mut put =
            |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        put(0x2000 + ccountry::OCCUPATION_STATUS, 0xc000);
        put(0xc000, ccountry_occupation_status::VTABLE_RVA.0 as u64);
        put(
            0xc000 + ccountry_occupation_status::SECONDARY_BASE,
            ccountry_occupation_status::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put(
            0xc000 + ccountry_occupation_status::OCCUPATION_DATA_TABLE,
            0x11300,
        );
        put(
            0xc000 + ccountry_occupation_status::OCCUPATION_DATA_TABLE + 8,
            1 | (1u64 << 32),
        );
        put(0x11300, 1u64 << 32);
        put(0x11300 + 8, 2);
        put(0x11300 + 16, 0xd000);
        put(0xd000, ccountry_occupation_data::VTABLE_RVA.0 as u64);
        put(0xd000 + ccountry_occupation_data::PARENT_STATUS, 0xc000);
        put(0xd000 + ccountry_occupation_data::OCCUPIED_COUNTRY, 3);
        put(
            0xd000 + ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER,
            0x4000,
        );
        for (header, storage, definition) in [
            (object + STATE_RESISTANCE_DEFINITIONS, 0x13600, 0x13400),
            (0xd000 + CORE_RESISTANCE_DEFINITIONS, 0x13620, 0x13500),
        ] {
            put(header, storage);
            put(header + 8, 1 | (1u64 << 32));
            put(storage as usize, definition);
            put(definition as usize, DEFINITION_VTABLE_RVA.0 as u64);
            put(definition as usize + DEFINITION_NAME + 16, 1);
            put(definition as usize + DEFINITION_NAME + 24, 15);
        }
        bytes[0x13400 + DEFINITION_NAME] = b'S';
        bytes[0x13500 + DEFINITION_NAME] = b'C';
        bytes[object + ACTIVE] = 0;
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let row = super::super::read_state(&memory, 0, source.pointer, source.id).unwrap();
        let input = read_inputs(&memory, 0, 0x1000, source, row).unwrap();
        assert_eq!(input.receivers, [0x2000, 0x4000, 0xc000, 0x4000]);
        let map = input.markers.unwrap();
        assert_eq!(map.resistance, [b"S".to_vec(), b"C".to_vec()]);
        assert_eq!(map.core_country, Some(2));
        assert_eq!(map.stored_target, None);
        bytes[0xd000 + ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER..][..8]
            .copy_from_slice(&0x2000u64.to_le_bytes());
        bytes[0xd000 + CORE_RESISTANCE_DEFINITIONS + 12..][..4].fill(0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let input = read_inputs(&memory, 0, 0x1000, source, row).unwrap();
        assert_eq!(input.receivers, [0x2000, 0x4000, 0xc000, 0x2000]);
        let map = input.markers.unwrap();
        assert_eq!(map.core_country, Some(1));
        assert_eq!(map.resistance, [b"S".to_vec()]);
        bytes[0xd000 + ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER..][..8].fill(0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert!(read_inputs(&memory, 0, 0x1000, source, row).is_err());
        bytes[0x11300 + 4] = 0;
        bytes[0xc000 + ccountry_occupation_status::OCCUPATION_DATA_TABLE + 8..][..4].fill(0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let input = read_inputs(&memory, 0, 0x1000, source, row).unwrap();
        assert_eq!(input.receivers, [0x2000, 0x4000, 0xc000, 0]);
        assert_eq!(input.markers.unwrap().core_country, None);
    }

    #[test]
    fn occupation_map_keeps_active_identity_occurrences_in_state_then_core_order() {
        let mut bytes = vec![0; 0x600];
        let mut put =
            |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        put(STATE_RESISTANCE_DEFINITIONS, 0x400);
        put(STATE_RESISTANCE_DEFINITIONS + 8, 2 | (2u64 << 32));
        put(0x300 + CORE_RESISTANCE_DEFINITIONS, 0x410);
        put(0x300 + CORE_RESISTANCE_DEFINITIONS + 8, 1 | (1u64 << 32));
        put(0x400, 0x480);
        put(0x408, 0x480);
        put(0x410, 0x500);
        for (definition, token) in [(0x480, b"state".as_slice()), (0x500, b"core".as_slice())] {
            put(definition, DEFINITION_VTABLE_RVA.0 as u64);
            put(definition + DEFINITION_NAME + 16, token.len() as u64);
            put(definition + DEFINITION_NAME + 24, 15);
        }
        bytes[0x480 + DEFINITION_NAME..][..5].copy_from_slice(b"state");
        bytes[0x500 + DEFINITION_NAME..][..4].copy_from_slice(b"core");
        let mut output = Vec::new();
        for at in [
            STATE_RESISTANCE_DEFINITIONS,
            0x300 + CORE_RESISTANCE_DEFINITIONS,
            0x400,
            0x408,
            0x410,
        ] {
            let value = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) + 0x1000;
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        append_definitions(
            &memory,
            0,
            0x1000,
            STATE_RESISTANCE_DEFINITIONS,
            &mut output,
        )
        .unwrap();
        append_definitions(&memory, 0, 0x1300, CORE_RESISTANCE_DEFINITIONS, &mut output).unwrap();
        assert_eq!(
            output,
            [b"state".to_vec(), b"state".to_vec(), b"core".to_vec()]
        );
    }

    #[test]
    fn occupation_map_target_is_read_only_for_the_zero_level_suppression_edge() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        for row in [
            Row {
                level: 1,
                ..Default::default()
            },
            Row {
                compliance: 1,
                ..Default::default()
            },
        ] {
            assert_eq!(target(&Unreadable, 0x1000, row), Ok(None));
        }
        assert!(target(&Unreadable, 0x1000, Row::default()).is_err());
        let bytes = 234_567i64.to_le_bytes();
        assert_eq!(
            target(
                &SliceMemory {
                    base: 0x1000 + STORED_TARGET,
                    bytes: &bytes
                },
                0x1000,
                Row::default()
            ),
            Ok(Some(234_567))
        );
        let input =
            read_inputs(&Unreadable, 0, 0, cstate::Source::default(), Row::default()).unwrap();
        assert_eq!(input.markers, None);
        assert_eq!(input.receivers, [0; 4]);
    }
}
