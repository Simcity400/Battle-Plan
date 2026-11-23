use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NamingOperands {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub override_name: cname_group_member::Span,
    pub programmatic: u8,
    pub equipment: Option<u64>,
}

pub fn naming(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
    bytes: &mut Vec<u8>,
    len: &mut usize,
    required: &mut usize,
) -> Result<Option<NamingOperands>, &'static str> {
    use cname_group_member as member;
    let pointer = name_member(memory, base, pointer, handle)?;
    let header = member::member(memory, pointer)?;
    let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    if engine_sites::at(base, member::VTABLE_RVA).map(|value| value as u64) != Some(word(0)) {
        return Err("ship naming member identity invalid");
    }
    let equipment = match word(member::NAMING_EQUIPMENT) {
        0 => None,
        pointer => {
            let pointer =
                usize::try_from(pointer).map_err(|_| "ship naming equipment pointer overflow")?;
            let identity = read::<[u8; 16]>(memory, pointer)
                .ok_or("ship naming equipment identity unreadable")?;
            let full = u64::from_le_bytes(identity[8..].try_into().unwrap());
            if engine_sites::at(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA).map(|value| value as u64)
                != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                || full as u32 != cequipment_variant::REFERENCE_TYPE
            {
                return Err("ship naming equipment identity invalid");
            }
            Some(full)
        }
    };
    let Some(override_name) =
        member::raw_override_from_member(memory, &header, bytes, len, required)?
    else {
        return Ok(None);
    };
    let signed = |at| i32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    Ok(Some(NamingOperands {
        kind: signed(member::TYPE),
        order: signed(member::ORDER),
        ordered: header[member::FLAG_A],
        programmatic: header[member::FLAG_B],
        override_name,
        equipment,
    }))
}
