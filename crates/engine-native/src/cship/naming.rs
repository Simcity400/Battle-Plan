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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShipNamesAndCrew {
    pub name: Option<cname_group_member::Span>,
    pub naming: Option<NamingOperands>,
    pub crew: Option<[i32; 2]>,
}

pub struct EquipmentMemo {
    entries: [(usize, u64); 256],
}

impl Default for EquipmentMemo {
    fn default() -> Self {
        Self {
            entries: [(0, 0); 256],
        }
    }
}

impl EquipmentMemo {
    fn slot(pointer: usize) -> usize {
        ((pointer >> 4).wrapping_mul(0x9e37_79b9_7f4a_7c15)) & 255
    }

    fn get(&self, pointer: usize) -> Option<u64> {
        let first = Self::slot(pointer);
        for probe in 0..8 {
            let (key, value) = self.entries[(first + probe) & 255];
            if key == pointer {
                return Some(value);
            }
            if key == 0 {
                return None;
            }
        }
        None
    }

    fn insert(&mut self, pointer: usize, value: u64) {
        let first = Self::slot(pointer);
        for probe in 0..8 {
            let slot = &mut self.entries[(first + probe) & 255];
            if slot.0 == 0 || slot.0 == pointer {
                *slot = (pointer, value);
                return;
            }
        }
    }
}

fn naming_from_member(
    memory: &impl Memory,
    base: usize,
    header: &[u8; cname_group_member::INPUT_SPAN],
    bytes: &mut Vec<u8>,
    len: &mut usize,
    required: &mut usize,
    memo: Option<&mut EquipmentMemo>,
) -> Result<Option<NamingOperands>, &'static str> {
    use cname_group_member as member;
    let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    if engine_sites::at(base, member::VTABLE_RVA).map(|value| value as u64) != Some(word(0)) {
        return Err("ship naming member identity invalid");
    }
    let equipment = match word(member::NAMING_EQUIPMENT) {
        0 => None,
        pointer => {
            let pointer =
                usize::try_from(pointer).map_err(|_| "ship naming equipment pointer overflow")?;
            if let Some(full) = memo.as_deref().and_then(|memo| memo.get(pointer)) {
                Some(full)
            } else {
                let identity = read::<[u8; 16]>(memory, pointer)
                    .ok_or("ship naming equipment identity unreadable")?;
                let full = u64::from_le_bytes(identity[8..].try_into().unwrap());
                if engine_sites::at(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA)
                    .map(|value| value as u64)
                    != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                    || full as u32 != cequipment_variant::REFERENCE_TYPE
                {
                    return Err("ship naming equipment identity invalid");
                }
                if let Some(memo) = memo {
                    memo.insert(pointer, full);
                }
                Some(full)
            }
        }
    };
    let Some(override_name) =
        member::raw_override_from_member(memory, header, bytes, len, required)?
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

#[allow(
    clippy::too_many_arguments,
    reason = "Decodes one pre-fetched ship naming member header with per-walk equipment memo."
)]
pub fn decode_batched_names_and_crew(
    memory: &impl Memory,
    base: usize,
    member_ptr: usize,
    header: Result<[u8; cname_group_member::INPUT_SPAN], &'static str>,
    crew: Option<[i32; 2]>,
    name_bytes: &mut Vec<u8>,
    name_len: &mut usize,
    name_required: &mut usize,
    override_bytes: &mut Vec<u8>,
    override_len: &mut usize,
    override_required: &mut usize,
    memo: &mut EquipmentMemo,
) -> ShipNamesAndCrew {
    if member_ptr == 0 {
        return ShipNamesAndCrew {
            name: None,
            naming: None,
            crew,
        };
    }
    let Ok(header) = header else {
        let name = cname_group_member::cached_name_growing(
            memory,
            base,
            member_ptr,
            name_bytes,
            name_len,
            name_required,
        )
        .ok()
        .flatten();
        return ShipNamesAndCrew {
            name,
            naming: None,
            crew,
        };
    };
    let name = cname_group_member::cached_name_from_member(
        memory,
        base,
        &header,
        name_bytes,
        name_len,
        name_required,
    )
    .ok()
    .flatten();
    let naming = naming_from_member(
        memory,
        base,
        &header,
        override_bytes,
        override_len,
        override_required,
        Some(memo),
    )
    .ok()
    .flatten();
    ShipNamesAndCrew { name, naming, crew }
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
    let pointer = name_member(memory, base, pointer, handle)?;
    let header = cname_group_member::member(memory, pointer)?;
    naming_from_member(memory, base, &header, bytes, len, required, None)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Combines ship cached name, naming operands, and crew reads into one scatter and member fetch."
)]
pub fn capture_names_and_crew(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
    name_bytes: &mut Vec<u8>,
    name_len: &mut usize,
    name_required: &mut usize,
    override_bytes: &mut Vec<u8>,
    override_len: &mut usize,
    override_required: &mut usize,
    quiet: impl Fn() -> Option<(u64, u64)>,
) -> ShipNamesAndCrew {
    let crew_token = quiet();
    let mut identity = [0u8; 16];
    let mut member_bytes = [0u8; 8];
    let mut crew_bytes = [0u8; 8];
    let member_address = pointer.checked_add(NAME_MEMBER);
    let crew_address = crew_token.and_then(|_| pointer.checked_add(ACQUIRED_MANPOWER));
    let (completed, crew) = match (member_address, crew_address) {
        (Some(member_at), Some(crew_at)) => {
            let done = memory.copy_scattered_prefetched(
                [pointer, member_at, crew_at],
                &mut [&mut identity, &mut member_bytes, &mut crew_bytes],
            );
            let crew = if done == 3 {
                (quiet() == crew_token).then(|| {
                    let [a, b, c, d, e, f, g, h] = crew_bytes;
                    [
                        i32::from_le_bytes([a, b, c, d]),
                        i32::from_le_bytes([e, f, g, h]),
                    ]
                })
            } else if done < 2 {
                read::<[u8; 8]>(memory, crew_at).and_then(|bytes| {
                    (quiet() == crew_token).then(|| {
                        let [a, b, c, d, e, f, g, h] = bytes;
                        [
                            i32::from_le_bytes([a, b, c, d]),
                            i32::from_le_bytes([e, f, g, h]),
                        ]
                    })
                })
            } else {
                None
            };
            (done.min(2), crew)
        }
        (Some(member_at), None) => {
            let done = memory.copy_scattered_prefetched(
                [pointer, member_at],
                &mut [&mut identity, &mut member_bytes],
            );
            (done, None)
        }
        (None, Some(crew_at)) => {
            let crew = read::<[u8; 8]>(memory, crew_at).and_then(|bytes| {
                (quiet() == crew_token).then(|| {
                    let [a, b, c, d, e, f, g, h] = bytes;
                    [
                        i32::from_le_bytes([a, b, c, d]),
                        i32::from_le_bytes([e, f, g, h]),
                    ]
                })
            });
            (0, crew)
        }
        (None, None) => (0, None),
    };
    let expected = match engine_sites::at(base, VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            return ShipNamesAndCrew {
                name: None,
                naming: None,
                crew,
            };
        }
    };
    if completed < 2
        || handle == 0
        || handle as u32 != HANDLE_TYPE
        || u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected
        || u64::from_le_bytes(identity[8..].try_into().unwrap()) != handle
    {
        return ShipNamesAndCrew {
            name: None,
            naming: None,
            crew,
        };
    }
    let Some(member_ptr) = usize::try_from(u64::from_le_bytes(member_bytes))
        .ok()
        .filter(|address| *address != 0)
    else {
        return ShipNamesAndCrew {
            name: None,
            naming: None,
            crew,
        };
    };
    let Ok(header) = cname_group_member::member(memory, member_ptr) else {
        let name = cname_group_member::cached_name_growing(
            memory,
            base,
            member_ptr,
            name_bytes,
            name_len,
            name_required,
        )
        .ok()
        .flatten();
        return ShipNamesAndCrew {
            name,
            naming: None,
            crew,
        };
    };
    let name = cname_group_member::cached_name_from_member(
        memory,
        base,
        &header,
        name_bytes,
        name_len,
        name_required,
    )
    .ok()
    .flatten();
    let naming = naming_from_member(
        memory,
        base,
        &header,
        override_bytes,
        override_len,
        override_required,
        None,
    )
    .ok()
    .flatten();
    ShipNamesAndCrew { name, naming, crew }
}
