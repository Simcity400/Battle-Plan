use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::{
    cdecision,
    cdecision_cooldown as cooldown,
    cdecision_status as status,
    ctargeted_decision as targeted,
    ctimed_decision as timed,
};
pub const PERMANENT: usize = 0x960;
pub const REMOVE_REQUESTED: usize = 0x2d;
pub const CANDIDATE_HEADERS: [usize; 4] = [0x160, 0x178, 0x190, 0x1a8];
pub const HEADERS: [usize; 6] = [
    status::DECISIONS_TAKEN,
    status::RE_ENABLE,
    status::REMOVE,
    status::ACTIVE_TIMED,
    status::TARGETED,
    status::TARGETED_TIMED,
];
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub pointer: usize,
    pub definition: usize,
    pub days: i32,
    pub lifecycle: i32,
    pub flags: u8,
    pub country_target: Option<u32>,
    pub numeric_target: Option<i32>,
}
fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    p: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(
        memory,
        p.checked_add(offset)
            .ok_or("decision runtime field overflow")?,
    )
    .ok_or("decision runtime field unreadable")
}
pub fn record(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    container: u8,
) -> Result<Record, &'static str> {
    let (vtable, definition, days, state) = match container {
        0 => (cdecision::VTABLE_RVA, 0, None, None),
        1 | 2 => (
            cooldown::VTABLE_RVA,
            cooldown::DEFINITION,
            Some(cooldown::DAYS),
            None,
        ),
        3 => (
            timed::VTABLE_RVA,
            timed::DEFINITION,
            Some(timed::DAYS),
            Some(timed::STATE),
        ),
        4 | 5 => (
            targeted::VTABLE_RVA,
            targeted::DEFINITION,
            Some(targeted::DAYS),
            Some(targeted::STATE),
        ),
        _ => return Err("decision runtime container differs"),
    };
    let vtable =
        engine_sites::at(base, vtable).ok_or("decision runtime instance identity differs")?;
    if read::<u64>(memory, pointer) != Some(vtable as u64) {
        return Err("decision runtime instance identity differs");
    }
    let definition = if container == 0 {
        pointer
    } else {
        field::<u64>(memory, pointer, definition)? as usize
    };
    let expected = engine_sites::at(base, cdecision::VTABLE_RVA)
        .ok_or("decision runtime definition identity differs")?;
    if read::<u64>(memory, definition) != Some(expected as u64) {
        return Err("decision runtime definition identity differs");
    }
    let mut value = Record {
        pointer,
        definition,
        days: days
            .map(|offset| field(memory, pointer, offset))
            .transpose()?
            .unwrap_or(0),
        lifecycle: state
            .map(|offset| field(memory, pointer, offset))
            .transpose()?
            .unwrap_or(0),
        flags: u8::from(field::<u8>(memory, definition, PERMANENT)? != 0),
        ..Default::default()
    };
    if container >= 4 {
        value.country_target = Some(field(memory, pointer, targeted::COUNTRY_TARGET)?);
        value.numeric_target = Some(field(memory, pointer, targeted::NUMERIC_TARGET)?);
        value.flags |= u8::from(field::<u8>(memory, pointer, targeted::IGNORE)? != 0) << 1;
        value.flags |= u8::from(field::<u8>(memory, pointer, REMOVE_REQUESTED)? != 0) << 2;
    }
    Ok(value)
}
pub fn array(memory: &impl Memory, status: usize, container: u8) -> Result<Array, &'static str> {
    let offset = *HEADERS
        .get(container as usize)
        .ok_or("decision runtime container differs")?;
    Array::read(
        memory,
        status
            .checked_add(offset)
            .ok_or("decision runtime header overflow")?,
        8,
    )
    .ok_or("decision runtime header unreadable")
}
pub fn at(
    memory: &impl Memory,
    base: usize,
    array: Array,
    ordinal: u32,
    container: u8,
) -> Result<Record, &'static str> {
    let p = array
        .element(ordinal, 8)
        .ok_or("decision runtime ordinal differs")?;
    record(
        memory,
        base,
        field::<u64>(memory, p, 0)? as usize,
        container,
    )
}

pub const CANDIDATE_STRIDE: usize = 0x18;
pub const CANDIDATE_DEFINITION: usize = 8;
pub const CANDIDATE_ROOT: usize = 0x10;
pub const IGNORED_TOKENS: usize = 0x14f8;
pub const REMOVAL_PASS_ACTIVE: usize = 0x1c1;
pub const EXPLICIT_TARGETS: usize = 0x21;

pub const ADMISSION_OVERRIDE_RVA: Rva = Rva(0x033ddaa2);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_records_retain_signed_words_definition_gate_and_target_flags() {
        let mut bytes = vec![0; 0xc00];
        let pointer = 0x1000;
        let definition = 0x1100;
        bytes[..8].copy_from_slice(&(targeted::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x10..0x18].copy_from_slice(&(definition as u64).to_le_bytes());
        bytes[0x20..0x24].copy_from_slice(&17_u32.to_le_bytes());
        bytes[0x24..0x28].copy_from_slice(&(-19_i32).to_le_bytes());
        bytes[0x28..0x2c].copy_from_slice(&3_i32.to_le_bytes());
        bytes[0x2c] = 1;
        bytes[0x2d] = 1;
        bytes[0x30..0x34].copy_from_slice(&i32::MIN.to_le_bytes());
        bytes[0x100..0x108].copy_from_slice(&(cdecision::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x100 + PERMANENT] = 1;
        let memory = engine_native_memory::memory::SliceMemory {
            base: pointer,
            bytes: &bytes,
        };
        let value = record(&memory, 0, pointer, 5).unwrap();
        assert_eq!((value.days, value.lifecycle, value.flags), (i32::MIN, 3, 7));
        assert_eq!(
            (value.country_target, value.numeric_target),
            (Some(17), Some(-19))
        );
        assert!(record(&memory, 0, pointer, 3).is_err());
    }
}
