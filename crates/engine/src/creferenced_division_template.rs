//! CReferencedDivisionTemplate identity and retained model preferences.

/// CReferencedDivisionTemplate vtable; old offsets.rs:639, pinned RTTI COL at RVA2CB8218 confirms the actual class name.
pub const VTABLE_RVA: usize = 0x0293_8f70;
/// CReferencedDivisionTemplate reference type; old offsets.rs:640 and template reader.
pub const REFERENCE_TYPE: u32 = 52;
/// CReferencedDivisionTemplate u32 type word; old offsets.rs:641 and template reader.
pub const TYPE: usize = 0x08;
/// CReferencedDivisionTemplate u32 id; old offsets.rs:642 and template reader.
pub const ID: usize = 0x0c;
/// CReferencedDivisionTemplate contained composition storage; old UZ func_140c6f080.decompile.c returns template+18, no RTTI base there.
pub const COMPOSITION: usize = 0x18;
/// CReferencedDivisionTemplate name MSVC string; old recorder_host/native_design_catalog.rs:492 receives registry primary pointer.
pub const NAME: usize = 0x20;
/// CReferencedDivisionTemplate grid dimensions, columns low16/rows high16; old native_design_catalog.rs:493-495.
pub const GRID_DIMENSIONS: usize = 0x60;
/// CReferencedDivisionTemplate regiment CSubUnitDefinition pointer-vector data; old native_design_catalog.rs:500.
pub const REGIMENT_DATA: usize = 0x68;
/// CReferencedDivisionTemplate regiment u32 count; old native_design_catalog.rs:500, row stride8.
pub const REGIMENT_COUNT: usize = 0x74;
/// CReferencedDivisionTemplate support CSubUnitDefinition pointer-vector data; old native_design_catalog.rs:503.
pub const SUPPORT_DATA: usize = 0x88;
/// CReferencedDivisionTemplate support u32 count; old native_design_catalog.rs:501-503, row stride8.
pub const SUPPORT_COUNT: usize = 0x94;
/// CReferencedDivisionTemplate regimental-support definition pointer-vector data; old native_design_catalog.rs:505, primary pointer.
pub const REGIMENTAL_SUPPORT_DATA: usize = 0xa8;
/// CReferencedDivisionTemplate regimental-support u32 count; old native_design_catalog.rs:505, row stride8 not contained composition pairs.
pub const REGIMENTAL_SUPPORT_COUNT: usize = 0xb4;
/// CReferencedDivisionTemplate excluded CEquipmentVariant pointer-vector data; division_presentation_inputs.rs:105-132.
pub const EXCLUDED_VARIANT_DATA: usize = 0x140;
/// CReferencedDivisionTemplate exclusion u32 capacity; division_presentation_inputs.rs:107.
pub const EXCLUDED_VARIANT_CAPACITY: usize = 0x148;
/// CReferencedDivisionTemplate exclusion u32 count; division_presentation_inputs.rs:108.
pub const EXCLUDED_VARIANT_COUNT: usize = 0x14c;
/// CReferencedDivisionTemplate u32 priority; old native_design_catalog.rs:507.
pub const PRIORITY: usize = 0x1a4;
/// CReferencedDivisionTemplate lock-regime snapshot byte; old native_design_catalog.rs:507, no boolean collapse.
pub const LOCK_REGIME: usize = 0x1cc;
/// CReferencedDivisionTemplate token289A country operand i32; old native_design_catalog/native_inputs.rs:37 and template_country_inputs.
pub const COUNTRY_OPERAND_289A: usize = 0x1ec;
/// CReferencedDivisionTemplate token3ABE country operand i32; old native_design_catalog/native_inputs.rs:38 and template_country_inputs.
pub const COUNTRY_OPERAND_3ABE: usize = 0x1f4;
/// CReferencedDivisionTemplate raw model-category u64; division_presentation_inputs.rs:151, meaningful only when presence byte is set.
pub const MODEL_CATEGORY: usize = 0x200;
/// CReferencedDivisionTemplate category-presence byte; division_presentation_inputs.rs:147.
pub const MODEL_CATEGORY_PRESENT: usize = 0x208;
/// CReferencedDivisionTemplate MSVC entity-name string; division_presentation_inputs.rs:144, engine-owned bytes preserved.
pub const MODEL_ENTITY: usize = 0x210;
/// CReferencedDivisionTemplate lock byte; old native_design_catalog.rs:507 preserves its nonzero test.
pub const LOCKED: usize = 0x235;
/// CReferencedDivisionTemplate obsolete flag byte, bit0; old native_design_catalog.rs:507.
pub const OBSOLETE_FLAGS: usize = 0x236;

use crate::{
    Memory, ccountry, cpdx_array::Array, csub_unit_definition as subunit, msvc_string::Descriptor,
    read, walk::Outcome,
};

/// Contained composition +190, consumed by 140B90370 for the counter sprite.
pub const PRIORITY_UNIT: usize = COMPOSITION + 0x190;
/// Contained composition +198, preferred by category getter 140B8F7F0.
pub const CATEGORY_UNIT: usize = COMPOSITION + 0x198;
/// Contained composition +1A8, saved by 140B977F0 token34ED and consumed by 1418C4B50.
pub const CUSTOM_ICON: usize = COMPOSITION + 0x1a8;
/// Contained composition +1AC, token3525; controls custom-icon use under NATO settings.
pub const CUSTOM_ICON_NATO: usize = COMPOSITION + 0x1ac;
/// Contained composition +218, first nonzero override in category getter 140B8F7F0.
pub const UNIT_TYPE_OVERRIDE: usize = COMPOSITION + 0x218;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub template: u64,
    pub owner: u32,
    pub name: Span,
    pub priority_unit: u32,
    pub priority_unit_name: Span,
    /// Engine category, NOT a verified frame in the legacy twelve-frame strip.
    pub unit_type: i32,
    pub custom_icon: i32,
    pub custom_icon_nato: u8,
    /// Packed columns-low16/rows-high16, for each of the three native grids.
    pub dimensions: [u32; 3],
    pub composition: [Span; 3],
}

pub struct Walk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub units_len: usize,
    pub required_units: usize,
}

fn pointer(memory: &impl Memory, at: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, at)
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("template pointer unreadable")
}
fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    at: usize,
    offset: usize,
) -> Result<T, &'static str> {
    at.checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("template field unreadable")
}
fn definition(memory: &impl Memory, base: usize, at: usize) -> Result<Option<u32>, &'static str> {
    if at == 0 {
        return Ok(None);
    }
    let vtable = field::<u64>(memory, at, 0)?;
    let real = field::<u8>(memory, at, subunit::IS_REAL)?;
    let expected = if real == 0 {
        subunit::NULL_VTABLE_RVA
    } else {
        subunit::VTABLE_RVA
    };
    if base.checked_add(expected).map(|p| p as u64) != Some(vtable) {
        return Err("template subunit identity mismatch");
    }
    if real == 0 {
        Ok(None)
    } else {
        let token = field(memory, at, subunit::TOKEN)?;
        // Zero is the replay's empty-cell marker. A real zero-token definition
        // is unsupported, not silently converted into an empty grid cell.
        if token == 0 {
            return Err("real template subunit has unsupported zero token");
        }
        Ok(Some(token))
    }
}

/// Complete country template rosters, including unused templates. Names and
/// ordered grid slots retain exact bytes/tokens; zero marks an empty grid cell.
/// Capacity omits a WHOLE template and grows caller arenas off-thread. No engine
/// getters are invoked: the sprite/category selection is read-only arithmetic.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut [Row],
    bytes: &mut [u8],
    units: &mut [u32],
) -> Walk {
    let mut result = Walk {
        outcome: Outcome::Rows { len: 0 },
        bytes_len: 0,
        required_bytes: 0,
        units_len: 0,
        required_units: 0,
    };
    let captured = (|| {
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut found_country = false;
        let mut len = 0;
        let mut missing = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let root = country
                .pointer
                .checked_add(ccountry::DIVISION_TEMPLATES)
                .ok_or("template roster overflow")?;
            let roster = Array::read(memory, root, 8).ok_or("template roster invalid")?;
            for index in 0..roster.count {
                let at = pointer(
                    memory,
                    roster
                        .element(index, 8)
                        .ok_or("template roster index invalid")?,
                )?;
                let template = identity(memory, base, at)?;
                let mut row = Row {
                    template,
                    owner: country.id,
                    ..Row::default()
                };
                let mut arrays = [Array { data: 0, count: 0 }; 3];
                let mut required_units = 0_usize;
                for (which, offset) in [REGIMENT_DATA, SUPPORT_DATA, REGIMENTAL_SUPPORT_DATA]
                    .into_iter()
                    .enumerate()
                {
                    let header = at
                        .checked_add(offset)
                        .ok_or("template grid address overflow")?;
                    arrays[which] =
                        Array::read(memory, header, 8).ok_or("template grid invalid")?;
                    row.dimensions[which] = field(memory, at, offset - 8)?;
                    required_units = required_units
                        .checked_add(arrays[which].count as usize)
                        .ok_or("template grid length overflow")?;
                }
                let name = Descriptor::read(
                    memory,
                    at.checked_add(NAME)
                        .ok_or("template name address overflow")?,
                )?;
                let mut selected = field::<u64>(memory, at, PRIORITY_UNIT)? as usize;
                if selected == 0 {
                    // 140B90370 uses 140FAA3F0's low-16-bit real-unit count:
                    // nonzero selects [0,0], zero the null singleton. Retain
                    // the native wrap instead of assuming a smaller grid.
                    let mut real_count = 0_u16;
                    for cell in 0..arrays[0].count {
                        let p = pointer(
                            memory,
                            arrays[0]
                                .element(cell, 8)
                                .ok_or("template grid cell invalid")?,
                        )?;
                        real_count = real_count
                            .wrapping_add(u16::from(definition(memory, base, p)?.is_some()));
                    }
                    selected = if real_count != 0 {
                        pointer(
                            memory,
                            arrays[0]
                                .element(0, 8)
                                .ok_or("template first cell missing")?,
                        )?
                    } else {
                        pointer(
                            memory,
                            base.checked_add(subunit::NULL_OBJECT_RVA)
                                .ok_or("template null singleton overflow")?,
                        )?
                    };
                }
                if selected == 0 {
                    return Err("template selected subunit is null");
                }
                let selected_token = definition(memory, base, selected)?;
                row.priority_unit = selected_token.unwrap_or(0);
                let selected_name = if selected_token.is_some() {
                    Some(Descriptor::read(
                        memory,
                        selected
                            .checked_add(subunit::NAME)
                            .ok_or("subunit name overflow")?,
                    )?)
                } else {
                    None
                };
                row.custom_icon = field(memory, at, CUSTOM_ICON)?;
                row.custom_icon_nato = field(memory, at, CUSTOM_ICON_NATO)?;
                row.unit_type = field(memory, at, UNIT_TYPE_OVERRIDE)?;
                if row.unit_type == 0 {
                    let category = field::<u64>(memory, at, CATEGORY_UNIT)? as usize;
                    let category = if category == 0 { selected } else { category };
                    definition(memory, base, category)?;
                    row.unit_type = subunit::template_category(memory, category)?;
                }
                let needed_bytes = name
                    .len
                    .checked_add(selected_name.as_ref().map_or(0, |n| n.len))
                    .ok_or("template name length overflow")?;
                result.required_bytes = result
                    .required_bytes
                    .checked_add(needed_bytes)
                    .ok_or("template byte arena overflow")?;
                result.required_units = result
                    .required_units
                    .checked_add(required_units)
                    .ok_or("template unit arena overflow")?;
                if len == rows.len()
                    || needed_bytes > bytes.len() - result.bytes_len
                    || required_units > units.len() - result.units_len
                {
                    missing = missing
                        .checked_add(1)
                        .ok_or("template row count overflow")?;
                    continue;
                }
                row.name = Span {
                    start: result.bytes_len,
                    len: name.len,
                };
                name.copy(memory, &mut bytes[result.bytes_len..])?;
                result.bytes_len += name.len;
                row.priority_unit_name = Span {
                    start: result.bytes_len,
                    len: selected_name.as_ref().map_or(0, |n| n.len),
                };
                if let Some(name) = selected_name {
                    name.copy(memory, &mut bytes[result.bytes_len..])?;
                    result.bytes_len += name.len;
                }
                for (which, array) in arrays.into_iter().enumerate() {
                    row.composition[which] = Span {
                        start: result.units_len,
                        len: array.count as usize,
                    };
                    for cell in 0..array.count {
                        let p = pointer(
                            memory,
                            array
                                .element(cell, 8)
                                .ok_or("template grid index invalid")?,
                        )?;
                        units[result.units_len] = definition(memory, base, p)?.unwrap_or(0);
                        result.units_len += 1;
                    }
                }
                rows[len] = row;
                len += 1;
            }
        }
        if !found_country {
            return Err("template census contains no countries");
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    result.outcome = captured.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    result
}

/// Authenticates a template pointer and returns its full native handle: the
/// reference type in the low word and the id in the high word. Callers that
/// accept a null pointer must test for it before calling.
pub(crate) fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u64, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("division template module address overflow")?;
    let bytes = read::<[u8; 16]>(memory, pointer).ok_or("division template identity unreadable")?;
    let handle = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64
        || handle as u32 != REFERENCE_TYPE
    {
        return Err("division template vtable or reference type mismatch");
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    struct Fixture(Vec<u8>, std::cell::RefCell<Vec<(usize, usize)>>);
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.1.borrow_mut().push((address, output.len()));
            if address == subunit::NULL_OBJECT_RVA && output.len() == 8 {
                output.copy_from_slice(&0x9000_u64.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: &self.0,
            }
            .copy(address, output)
        }
    }
    impl Fixture {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0[at..at + bytes.len()].copy_from_slice(bytes);
        }
        fn word(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: u64, count: u32) {
            self.word(at, data);
            self.put(at + 8, &count.to_le_bytes());
            self.put(at + 12, &count.to_le_bytes());
        }
        fn text(&mut self, at: usize, bytes: &[u8]) {
            self.put(at, bytes);
            self.word(at + 16, bytes.len() as u64);
            self.word(at + 24, 15);
        }
    }
    fn fixture() -> Fixture {
        let mut memory = Fixture(vec![0; 0xa000], std::cell::RefCell::default());
        memory.array(0x1000 + crate::cgame_state::COUNTRIES, 0x2000, 2);
        memory.array(0x1000 + crate::cgame_state::COUNTRY_LINKS, 0x2100, 2);
        memory.word(0x2008, 0x3000);
        memory.put(0x2104, &1_u32.to_le_bytes());
        memory.word(0x3000, ccountry::VTABLE_RVA as u64);
        memory.put(0x3008, &1_u32.to_le_bytes());
        memory.array(0x3000 + ccountry::DIVISION_TEMPLATES, 0x4000, 1);
        memory.word(0x4000, 0x5000);
        memory.word(0x5000, VTABLE_RVA as u64);
        memory.word(0x5008, 0xffff_ffff_0000_0034);
        memory.text(0x5000 + NAME, &[b'A', 0, 255]);
        memory.array(0x5000 + REGIMENT_DATA, 0x6000, 2);
        memory.put(0x5000 + GRID_DIMENSIONS, &0x0002_0001_u32.to_le_bytes());
        memory.word(0x6000, 0x7000);
        memory.word(0x6008, 0);
        memory.put(0x5000 + CUSTOM_ICON, &(-1_i32).to_le_bytes());
        memory.word(0x7000, subunit::VTABLE_RVA as u64);
        memory.put(0x7008, &777_u32.to_le_bytes());
        memory.0[0x7010] = 1;
        memory.text(0x7018, b"infantry");
        memory.put(0x7000 + subunit::CATEGORY, &4_i32.to_le_bytes());
        memory.word(0x9000, subunit::NULL_VTABLE_RVA as u64);
        memory
    }
    #[test]
    fn template_rosters_keep_full_handles_exact_names_and_empty_grid_slots() {
        let memory = fixture();
        let mut rows = [Row::default(); 2];
        let mut bytes = [0; 64];
        let mut units = [0; 8];
        let result = walk(&memory, 0, 0x1000, &mut rows, &mut bytes, &mut units);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].template, 0xffff_ffff_0000_0034);
        assert_eq!(rows[0].owner, 1);
        assert_eq!(rows[0].priority_unit, 777);
        assert_eq!(
            &bytes[..result.bytes_len],
            &[b'A', 0, 255, b'i', b'n', b'f', b'a', b'n', b't', b'r', b'y']
        );
        assert_eq!(&units[..result.units_len], &[777, 0]);
        assert_eq!(rows[0].unit_type, 1);
        assert_eq!(rows[0].custom_icon, -1);
        assert_eq!(rows[0].composition[0], Span { start: 0, len: 2 });
        for address in [
            0x3000 + ccountry::DIVISION_TEMPLATES,
            0x5000 + REGIMENT_DATA,
            0x5000 + SUPPORT_DATA,
            0x5000 + REGIMENTAL_SUPPORT_DATA,
        ] {
            assert_eq!(
                memory
                    .1
                    .borrow()
                    .iter()
                    .filter(|p| **p == (address, 16))
                    .count(),
                1
            );
        }
    }
    #[test]
    fn template_capacity_omits_whole_rows_and_bad_identity_never_becomes_empty() {
        let mut memory = fixture();
        for (row_capacity, text_capacity, unit_capacity) in [(0, 64, 8), (2, 2, 8), (2, 64, 1)] {
            let result = walk(
                &memory,
                0,
                0x1000,
                &mut vec![Row::default(); row_capacity],
                &mut vec![0; text_capacity],
                &mut vec![0; unit_capacity],
            );
            assert_eq!(
                result.outcome,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                }
            );
            assert_eq!((result.bytes_len, result.units_len), (0, 0));
            assert_eq!((result.required_bytes, result.required_units), (11, 2));
        }
        memory.word(0x5000, 0);
        assert!(matches!(
            walk(
                &memory,
                0,
                0x1000,
                &mut [Row::default(); 2],
                &mut [0; 64],
                &mut [0; 8]
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
        let mut memory = fixture();
        memory.word(0x5000 + PRIORITY_UNIT, 0x7000);
        memory.put(0x5000 + UNIT_TYPE_OVERRIDE, &123_i32.to_le_bytes());
        let mut rows = [Row::default(); 1];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows, &mut [0; 64], &mut [0; 8]).outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].unit_type, 123); // Never clamp a native override to strip size.
    }

    #[test]
    fn fallback_preserves_native_real_count_wrap_and_rejects_zero_tokens() {
        for count in [65_535_u32, 65_536, 65_537] {
            let mut memory = fixture();
            memory.0.resize(0x10000 + count as usize * 8, 0);
            memory.array(0x5000 + REGIMENT_DATA, 0x10000, count);
            for index in 0..count as usize {
                memory.word(0x10000 + index * 8, 0x7000);
            }
            let mut rows = [Row::default(); 1];
            let result = walk(
                &memory,
                0,
                0x1000,
                &mut rows,
                &mut [0; 64],
                &mut vec![0; count as usize],
            );
            assert_eq!(result.outcome, Outcome::Rows { len: 1 });
            assert_eq!(rows[0].priority_unit, if count == 65_536 { 0 } else { 777 });
        }
        let mut memory = fixture();
        memory.put(0x7000 + subunit::TOKEN, &0_u32.to_le_bytes());
        assert_eq!(
            walk(
                &memory,
                0,
                0x1000,
                &mut [Row::default(); 1],
                &mut [0; 64],
                &mut [0; 8]
            )
            .outcome,
            Outcome::Unavailable {
                reason: "real template subunit has unsupported zero token"
            }
        );
    }

    #[test]
    fn full_handle_survives_and_a_wrong_type_word_fails() {
        let mut bytes = vec![0; 16];
        bytes[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
        bytes[8..].copy_from_slice(&0xffff_ffff_0000_0034_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(identity(&memory, 0, 0x1000), Ok(0xffff_ffff_0000_0034));
        bytes[8] = 0x33;
        assert_eq!(
            identity(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Err("division template vtable or reference type mismatch")
        );
        assert_eq!(
            identity(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..8]
                },
                0,
                0x1000
            ),
            Err("division template identity unreadable")
        );
    }
}
