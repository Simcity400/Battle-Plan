
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a2_31d0);
pub const CHARACTER: usize = 0x10;
pub const IDEA_TOKEN: usize = 0x30;
pub const SLOT: usize = 0x80;
pub const TRAITS: usize = 0xc8;
pub const MODIFIER_ENTRIES: usize = 0x1b0;
pub const MODIFIER_CAPACITY_REASON: &str = "advisor modifier cache exceeds admitted capacity";

pub fn modifier_entries(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
) -> Result<engine_native_memory::cpdx_array::Array, &'static str> {
    let header = engine_native_memory::read::<[u8; 16]>(memory, crate::ccharacter::at(pointer, MODIFIER_ENTRIES)?)
        .ok_or("advisor modifier cache header unreadable")?;
    engine_native_memory::cpdx_array::Array::from_header(header, engine_native_parts::modifier_entry::STRIDE)
        .ok_or("advisor modifier cache header invalid")
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierDemand {
    pub rows: usize,
    pub bytes: usize,
    pub tooltip_bytes: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionDemand {
    pub characters: usize,
    pub rows: usize,
    pub bytes: usize,
}

pub fn definitions_demand(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
) -> Result<[DefinitionDemand; 2], &'static str> {
    let mut demand = [DefinitionDemand::default(); 2];
    crate::ccharacter::manager::visit(memory, base, game_state, |entry| {
        let total = &mut demand[entry.roster as usize];
        total.characters = total
            .characters
            .checked_add(1)
            .ok_or("advisor character count overflow")?;
        let mut tree = engine_native_memory::msvc_tree_node::Cursor::read(
            memory,
            entry.pointer,
            crate::ccharacter::ADVISOR_MAP_HEAD,
        )?;
        while let Some(node) = tree.next(memory)? {
            let role = engine_native_memory::read::<u64>(
                memory,
                crate::ccharacter::at(node, engine_native_parts::character_advisor_node::ADVISOR)?,
            )
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("advisor role pointer unreadable or null")?;
            total.rows = total
                .rows
                .checked_add(1)
                .ok_or("advisor row count overflow")?;
            for address in [
                crate::ccharacter::at(node, engine_native_parts::character_advisor_node::KEY)?,
                crate::ccharacter::at(role, IDEA_TOKEN)?,
                crate::ccharacter::at(role, SLOT)?,
            ] {
                let value = engine_native_memory::msvc_string::Descriptor::read(memory, address)?;
                total.bytes = total
                    .bytes
                    .checked_add(value.len)
                    .ok_or("advisor definition byte count overflow")?;
            }
        }
        Ok(())
    })?;
    Ok(demand)
}

pub fn modifier_demand(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
) -> Result<ModifierDemand, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    let mut demand = ModifierDemand::default();
    crate::ccharacter::manager::visit(memory, base, game_state, |entry| {
        let mut tree = engine_native_memory::msvc_tree_node::Cursor::read(
            memory,
            entry.pointer,
            crate::ccharacter::ADVISOR_MAP_HEAD,
        )?;
        while let Some(node) = tree.next(memory)? {
            let pointer = engine_native_memory::read::<u64>(
                memory,
                crate::ccharacter::at(node, engine_native_parts::character_advisor_node::ADVISOR)?,
            )
            .ok_or("advisor role pointer unreadable")?;
            let pointer = usize::try_from(pointer)
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("advisor role pointer null or invalid")?;
            let vtable =
                engine_native_memory::read::<u64>(memory, pointer).ok_or("advisor role vtable unreadable")?;
            if vtable != expected {
                return Err("advisor role vtable mismatch");
            }
            let parent = engine_native_memory::read::<u64>(memory, crate::ccharacter::at(pointer, CHARACTER)?)
                .ok_or("advisor role character parent unreadable")?;
            if parent != entry.pointer as u64 {
                return Err("advisor role character parent mismatch");
            }
            let array = modifier_entries(memory, pointer)?;
            let tooltips = count_tooltip_keys(memory, pointer)?;
            demand.tooltip_bytes = demand
                .tooltip_bytes
                .checked_add(tooltips.required)
                .ok_or("advisor tooltip byte count overflow")?;
            demand.rows = demand
                .rows
                .checked_add(1)
                .ok_or("advisor row count overflow")?;
            let bytes = (array.count as usize)
                .checked_mul(engine_native_parts::modifier_entry::STRIDE)
                .ok_or("advisor modifier cache size overflow")?;
            demand.bytes = demand
                .bytes
                .checked_add(bytes)
                .ok_or("advisor modifier byte count overflow")?;
        }
        Ok(())
    })?;
    Ok(demand)
}

pub const TOOLTIP_KEYS: usize = 0x218;
pub const TOOLTIP_CAPACITY_REASON: &str = "advisor tooltip keys exceed admitted capacity";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TooltipRead {
    pub len: usize,
    pub required: usize,
    pub recorded: bool,
}

pub fn visit_tooltip_keys(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    mut visit: impl FnMut(usize, [u8; 32]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let header = engine_native_memory::read::<[u8; 17]>(memory, crate::ccharacter::at(pointer, TOOLTIP_KEYS + 8)?)
        .ok_or("advisor tooltip table header unreadable")?;
    let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .map_err(|_| "advisor tooltip table pointer overflow")?;
    let count = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let overflow = usize::from(header[16]);
    if !mask
        .checked_add(1)
        .is_some_and(|capacity| capacity.is_power_of_two())
    {
        return Err("advisor tooltip table bucket mask invalid");
    }
    let span = (mask as usize)
        .checked_add(overflow)
        .and_then(|n| n.checked_add(1))
        .ok_or("advisor tooltip table span overflow")?;
    if count as usize > span || header[16] == 255 {
        return Err("advisor tooltip table header invalid");
    }
    if data == 0 {
        return if count == 0 {
            Ok(())
        } else {
            Err("advisor tooltip table has null storage")
        };
    }
    data.checked_add(
        span.checked_mul(0x28)
            .ok_or("advisor tooltip table span overflow")?,
    )
    .ok_or("advisor tooltip table address overflow")?;
    let mut observed = 0_u32;
    for index in 0..span {
        let slot = data + index * 0x28;
        let value =
            engine_native_memory::read::<[u8; 0x28]>(memory, slot).ok_or("advisor tooltip slot unreadable")?;
        match value[4] {
            0 => continue,
            255 => return Err("advisor tooltip sentinel inside table"),
            _ => {}
        }
        observed = observed
            .checked_add(1)
            .ok_or("advisor tooltip count overflow")?;
        if observed > count {
            return Err("advisor tooltip slot count differs from header");
        }
        visit(slot + 8, value[8..].try_into().unwrap())?;
    }
    if observed != count {
        return Err("advisor tooltip slot count differs from header");
    }
    Ok(())
}

pub fn count_tooltip_keys(memory: &impl engine_native_memory::Memory, pointer: usize) -> Result<TooltipRead, &'static str> {
    copy_tooltip_keys_buffer(memory, pointer, engine_native_memory::walk::Buffer::Count)
}


pub fn copy_tooltip_keys_buffer(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    mut bytes: engine_native_memory::walk::Buffer<'_, u8>,
) -> Result<TooltipRead, &'static str> {
    use engine_native_memory::msvc_string::Literal;
    let mut read = TooltipRead {
        recorded: true,
        ..Default::default()
    };
    visit_tooltip_keys(memory, pointer, |address, header| {
        let literal = Literal::from_header(address, header)?;
        Literal::validate(memory, [&literal])?;
        let needed = literal
            .len
            .checked_add(8)
            .ok_or("advisor tooltip size overflow")?;
        read.required = read
            .required
            .checked_add(needed)
            .ok_or("advisor tooltip byte count overflow")?;
        bytes.ensure(
            read.len
                .checked_add(needed)
                .ok_or("advisor tooltip byte count overflow")?,
        )?;
        if !read.recorded || needed > bytes.len() - read.len {
            read.recorded = false;
            return Ok(());
        }
        bytes[read.len..read.len + 8].copy_from_slice(&(literal.len as u64).to_le_bytes());
        Literal::copy_all(
            memory,
            [&literal],
            &mut bytes[read.len + 8..read.len + needed],
        )?;
        read.len += needed;
        Ok(())
    })?;
    if !read.recorded {
        read.len = 0;
    }
    Ok(read)
}

pub fn copy_modifier_entries(
    memory: &impl engine_native_memory::Memory,
    array: engine_native_memory::cpdx_array::Array,
    bytes: &mut [u8],
) -> Result<usize, &'static str> {
    let len = (array.count as usize)
        .checked_mul(engine_native_parts::modifier_entry::STRIDE)
        .ok_or("advisor modifier cache size overflow")?;
    if len > bytes.len() {
        return Err("advisor modifier cache exceeds byte capacity");
    }
    let mut first = 0;
    while first < array.count {
        let count = (array.count - first).min(64) as usize;
        let start = first as usize * engine_native_parts::modifier_entry::STRIDE;
        let end = start + count * engine_native_parts::modifier_entry::STRIDE;
        let address = array
            .element(first, engine_native_parts::modifier_entry::STRIDE)
            .ok_or("advisor modifier cache address overflow")?;
        if !engine_native_memory::read_slice(memory, address, &mut bytes[start..end]) {
            return Err("advisor modifier cache entries unreadable");
        }
        first += count as u32;
    }
    Ok(len)
}

pub struct MapView<'a, M> {
    memory: &'a M,
    address: usize,
    head: usize,
    pub header: [u8; 16],
    pub sentinel: [u8; 26],
}
impl<'a, M: engine_native_memory::Memory> MapView<'a, M> {
    pub fn read(memory: &'a M, character: usize) -> Result<Self, &'static str> {
        Self::with_sentinel(memory, character, |head| {
            engine_native_memory::read::<[u8; 26]>(memory, head)
        })
    }

    pub fn with_sentinel(
        memory: &'a M,
        character: usize,
        sentinel: impl FnOnce(usize) -> Option<[u8; 26]>,
    ) -> Result<Self, &'static str> {
        let address = crate::ccharacter::at(character, crate::ccharacter::ADVISOR_MAP_HEAD)?;
        let header = engine_native_memory::read::<[u8; 16]>(memory, address)
            .ok_or("character role map header unreadable")?;
        let head = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("character role map head invalid")?;
        u32::try_from(u64::from_le_bytes(header[8..].try_into().unwrap()))
            .map_err(|_| "character role map count overflow")?;
        let sentinel = sentinel(head).ok_or("character role map sentinel unreadable")?;
        let view = Self {
            memory,
            address,
            head,
            header,
            sentinel,
        };
        engine_native_memory::msvc_tree_node::Cursor::read(&view, character, crate::ccharacter::ADVISOR_MAP_HEAD)?;
        Ok(view)
    }
    fn owned(&self, address: usize, len: usize) -> Option<&[u8]> {
        if address == self.address && len == 16 {
            Some(&self.header)
        } else if address == self.head && len == 26 {
            Some(&self.sentinel)
        } else {
            None
        }
    }
}
impl<M: engine_native_memory::Memory> engine_native_memory::Memory for MapView<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some(bytes) = self.owned(address, output.len()) {
            output.copy_from_slice(bytes);
            true
        } else {
            self.memory.copy(address, output)
        }
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        if addresses
            .iter()
            .zip(outputs.iter())
            .all(|(&address, output)| self.owned(address, output.len()).is_none())
        {
            self.memory.copy_scattered(addresses, outputs)
        } else {
            for (index, (&address, output)) in addresses.iter().zip(outputs.iter_mut()).enumerate()
            {
                if !self.copy(address, output) {
                    return index;
                }
            }
            N
        }
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        self.copy_scattered(addresses, outputs)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub pointer: usize,
    pub character: u32,
    pub country: u32,
    pub ordinal: u32,
    pub key_start: usize,
    pub key_len: usize,
    pub idea_start: usize,
    pub idea_len: usize,
    pub slot_start: usize,
    pub slot_len: usize,
    pub modifier_start: usize,
    pub modifier_len: usize,
    pub modifier_recorded: bool,
    pub modifier_failure: Option<&'static str>,
    pub tooltip_start: usize,
    pub tooltip_len: usize,
    pub tooltip_recorded: bool,
    pub tooltip_failure: Option<&'static str>,
}

pub fn walk_prefix_entry(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    entry: crate::ccharacter::manager::Entry,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    bytes: engine_native_memory::walk::Buffer<'_, u8>,
) -> crate::ccharacter::NameWalk {
    walk_entry_inner_buffer(memory, base, entry, output, bytes, true)
}

pub fn walk_definitions_entry_buffer(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    entry: crate::ccharacter::manager::Entry,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    bytes: engine_native_memory::walk::Buffer<'_, u8>,
) -> crate::ccharacter::NameWalk {
    walk_entry_inner_buffer(memory, base, entry, output, bytes, false)
}

fn walk_entry_inner_buffer(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    entry: crate::ccharacter::manager::Entry,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut bytes: engine_native_memory::walk::Buffer<'_, u8>,
    capture_cache: bool,
) -> crate::ccharacter::NameWalk {
    use crate::ccharacter::{self, NameWalk, at};
    use engine_native_memory::{
        msvc_string::Literal,
        msvc_tree_node::Cursor as RoleTree,
        walk::Outcome,
    };
    use engine_native_parts::character_advisor_node;
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        let mut len = 0;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        {
            let (character_pointer, character, country) = (entry.pointer, entry.id, entry.country);
            let mut tree = RoleTree::read(memory, character_pointer, ccharacter::ADVISOR_MAP_HEAD)?;
            output.ensure(tree.count() as usize)?;
            let mut ordinal = 0;
            let mut previous_hash = None;
            if len == output.len() {
                return Ok(NameWalk {
                    outcome: if tree.count() == 0 {
                        Outcome::Rows { len: 0 }
                    } else {
                        Outcome::Truncated {
                            len: 0,
                            unread_rows: tree.count(),
                        }
                    },
                    bytes_len: 0,
                    required_bytes: 0,
                });
            }
            while let Some(node) = tree.next(memory)? {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(tree.count() - ordinal)
                        .ok_or("advisor row count overflow")?;
                    break;
                }
                let mut role_pointer = [0; 8];
                let mut key_header = [0; 32];
                let mut hash = [0; 4];
                if memory.copy_scattered(
                    [
                        at(node, character_advisor_node::ADVISOR)?,
                        at(node, character_advisor_node::KEY)?,
                        at(node, character_advisor_node::KEY_HASH)?,
                    ],
                    &mut [&mut role_pointer, &mut key_header, &mut hash],
                ) != 3
                {
                    return Err("advisor role node unreadable");
                }
                let role = usize::try_from(u64::from_le_bytes(role_pointer))
                    .ok()
                    .filter(|&p| p != 0)
                    .ok_or("advisor role pointer null or invalid")?;
                let mut vtable = [0; 8];
                let mut parent = [0; 8];
                let mut idea_header = [0; 32];
                let mut slot_header = [0; 32];
                if memory.copy_scattered(
                    [
                        role,
                        at(role, CHARACTER)?,
                        at(role, IDEA_TOKEN)?,
                        at(role, SLOT)?,
                    ],
                    &mut [&mut vtable, &mut parent, &mut idea_header, &mut slot_header],
                ) != 4
                {
                    return Err("advisor role fields unreadable");
                }
                if u64::from_le_bytes(vtable) != expected as u64 {
                    return Err("advisor role vtable mismatch");
                }
                if u64::from_le_bytes(parent) != character_pointer as u64 {
                    return Err("advisor role character parent mismatch");
                }
                let key = Literal::from_header(at(node, character_advisor_node::KEY)?, key_header)?;
                let hash = u32::from_le_bytes(hash);
                if previous_hash.is_some_and(|previous| previous > hash) {
                    return Err("advisor role map hash order invalid");
                }
                if previous_hash == Some(hash) {
                    return Err("advisor role hash collision needs native locale comparison");
                }
                previous_hash = Some(hash);
                let idea = Literal::from_header(at(role, IDEA_TOKEN)?, idea_header)?;
                let slot = Literal::from_header(at(role, SLOT)?, slot_header)?;
                Literal::validate(memory, [&key, &idea, &slot])?;
                let needed = key
                    .len
                    .checked_add(idea.len)
                    .and_then(|n| n.checked_add(slot.len))
                    .ok_or("advisor string size overflow")?;
                required_bytes = required_bytes
                    .checked_add(needed)
                    .ok_or("advisor string size overflow")?;
                bytes.ensure(
                    bytes_len
                        .checked_add(needed)
                        .ok_or("advisor byte count overflow")?,
                )?;
                if needed > bytes.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(tree.count() - ordinal)
                        .ok_or("advisor row count overflow")?;
                    break;
                }
                let key_start = bytes_len;
                let idea_start = key_start + key.len;
                let slot_start = idea_start + idea.len;
                Literal::copy_all(
                    memory,
                    [&key, &idea, &slot],
                    &mut bytes[key_start..slot_start + slot.len],
                )?;
                let modifier_start = slot_start + slot.len;
                let mut modifier_recorded = capture_cache;
                let mut modifier_failure = None;
                let modifier_len = if capture_cache {
                    let array = modifier_entries(memory, role)?;
                    let needed = (array.count as usize)
                        .checked_mul(engine_native_parts::modifier_entry::STRIDE)
                        .ok_or("advisor modifier cache size overflow")?;
                    required_bytes = required_bytes
                        .checked_add(needed)
                        .ok_or("advisor byte count overflow")?;
                    bytes.ensure(
                        modifier_start
                            .checked_add(needed)
                            .ok_or("advisor byte count overflow")?,
                    )?;
                    if needed > bytes.len() - modifier_start {
                        modifier_recorded = false;
                        modifier_failure = Some(MODIFIER_CAPACITY_REASON);
                        0
                    } else {
                        copy_modifier_entries(memory, array, &mut bytes[modifier_start..])?
                    }
                } else {
                    0
                };
                output[len] = Row {
                    pointer: role,
                    character,
                    country,
                    ordinal,
                    key_start,
                    key_len: key.len,
                    idea_start,
                    idea_len: idea.len,
                    slot_start,
                    slot_len: slot.len,
                    modifier_start,
                    modifier_len,
                    modifier_recorded,
                    modifier_failure,
                    ..Default::default()
                };
                len += 1;
                bytes_len += needed + modifier_len;
                ordinal += 1;
            }
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

pub fn append_tooltips(
    memory: &impl engine_native_memory::Memory,
    rows: &mut [Row],
    mut bytes: engine_native_memory::walk::Buffer<'_, u8>,
    mut captured: crate::ccharacter::NameWalk,
) -> crate::ccharacter::NameWalk {
    for row in &mut rows[..captured.outcome.len()] {
        row.tooltip_start = captured.bytes_len;
        match copy_tooltip_keys_buffer(memory, row.pointer, bytes.suffix(captured.bytes_len)) {
            Ok(read) => {
                let Some(required) = captured.required_bytes.checked_add(read.required) else {
                    row.tooltip_failure = Some("advisor tooltip byte count overflow");
                    continue;
                };
                captured.required_bytes = required;
                row.tooltip_recorded = read.recorded;
                row.tooltip_len = read.len;
                if read.recorded {
                    captured.bytes_len += read.len;
                } else {
                    row.tooltip_failure = Some(TOOLTIP_CAPACITY_REASON);
                }
            }
            Err(reason) => row.tooltip_failure = Some(reason),
        }
    }
    captured
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn tooltip_keys_are_exact_strings_in_occupied_slot_order_and_size_from_native_lengths() {
        let mut bytes = vec![0; 0x300];
        let table = TOOLTIP_KEYS + 8;
        bytes[table..table + 8].copy_from_slice(&0x40_u64.to_le_bytes());
        bytes[table + 8..table + 12].copy_from_slice(&2_u32.to_le_bytes());
        bytes[table + 12..table + 16].copy_from_slice(&3_u32.to_le_bytes());
        let first = [b'k', 0, 0xff];
        let last = [b'x'; 17];
        bytes[0x44] = 1;
        bytes[0x48..0x4b].copy_from_slice(&first);
        bytes[0x58..0x60].copy_from_slice(&3_u64.to_le_bytes());
        bytes[0x60..0x68].copy_from_slice(&15_u64.to_le_bytes());
        bytes[0x70..0x90].fill(0xff);
        let slot = 0x40 + 3 * 0x28;
        bytes[slot + 4] = 1;
        bytes[slot + 8..slot + 16].copy_from_slice(&0x180_u64.to_le_bytes());
        bytes[slot + 24..slot + 32].copy_from_slice(&17_u64.to_le_bytes());
        bytes[slot + 32..slot + 40].copy_from_slice(&17_u64.to_le_bytes());
        bytes[0x180..0x191].copy_from_slice(&last);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut output = vec![0; 36];
        let read = copy_tooltip_keys_buffer(&memory, 0, engine_native_memory::walk::Buffer::Growing(&mut output)).unwrap();
        assert_eq!(
            read,
            TooltipRead {
                len: 36,
                required: 36,
                recorded: true
            }
        );
        assert_eq!(&output[..8], &3_u64.to_le_bytes());
        assert_eq!(&output[8..11], &first);
        assert_eq!(&output[11..19], &17_u64.to_le_bytes());
        assert_eq!(&output[19..], &last);
        let read = count_tooltip_keys(&memory, 0).unwrap();
        assert_eq!(
            read,
            TooltipRead {
                len: 0,
                required: 36,
                recorded: false
            }
        );
        bytes[table + 8..table + 12].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            copy_tooltip_keys_buffer(&SliceMemory {
                    base: 0,
                    bytes: &bytes
                }, 0, engine_native_memory::walk::Buffer::Growing(&mut output)),
            Err("advisor tooltip slot count differs from header")
        );
    }

    #[test]
    fn modifier_demand_sums_live_headers_and_propagates_faults() {
        use crate::{ccharacter::{self, manager as ccharacter_manager}, cgame_state};
        const STATE: usize = 0x100;
        const MANAGER: usize = 0x1000;
        const ROSTER: usize = 0x2000;
        const OBJECTS: usize = 0x1_0000;
        const STRIDE: usize = 0x800;
        const COUNT: usize = 9;
        let mut bytes = vec![0; OBJECTS + COUNT * STRIDE];
        let word = |bytes: &mut [u8], address: usize, value: u64| {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        };
        let integer = |bytes: &mut [u8], address: usize, value: u32| {
            bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
        };
        word(
            &mut bytes,
            STATE + cgame_state::CHARACTER_MANAGER,
            MANAGER as u64,
        );
        word(&mut bytes, MANAGER, ccharacter_manager::VTABLE_RVA.0 as u64);
        let header = MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS;
        word(&mut bytes, header, ROSTER as u64);
        integer(&mut bytes, header + 8, COUNT as u32);
        integer(&mut bytes, header + 12, COUNT as u32);
        for index in 0..COUNT {
            let character = OBJECTS + index * STRIDE;
            let head = character + 0x300;
            let node = character + 0x340;
            let role = character + 0x400;
            word(&mut bytes, ROSTER + index * 8, character as u64);
            word(&mut bytes, character, ccharacter::VTABLE_RVA.0 as u64);
            integer(
                &mut bytes,
                character + ccharacter::TYPE,
                ccharacter::REFERENCE_TYPE,
            );
            integer(&mut bytes, character + ccharacter::ID, index as u32 + 1);
            integer(
                &mut bytes,
                character + ccharacter::COUNTRY,
                index as u32 + 1,
            );
            word(
                &mut bytes,
                character + ccharacter::ADVISOR_MAP_HEAD,
                head as u64,
            );
            word(&mut bytes, character + ccharacter::ADVISOR_MAP_SIZE, 1);
            for offset in [0, 8, 16] {
                word(&mut bytes, head + offset, node as u64);
                word(&mut bytes, node + offset, head as u64);
            }
            bytes[head + engine_native_memory::msvc_tree_node::NIL] = 1;
            word(
                &mut bytes,
                node + engine_native_parts::character_advisor_node::ADVISOR,
                role as u64,
            );
            word(&mut bytes, role, VTABLE_RVA.0 as u64);
            word(&mut bytes, role + CHARACTER, character as u64);
            let header = role + MODIFIER_ENTRIES;
            word(&mut bytes, header, (role + 0x200) as u64);
            integer(&mut bytes, header + 8, 2);
            integer(&mut bytes, header + 12, 2);
        }
        assert_eq!(
            modifier_demand(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                STATE,
            ),
            Ok(ModifierDemand {
                rows: COUNT,
                bytes: COUNT * 32,
                tooltip_bytes: 0,
            })
        );
        integer(&mut bytes, OBJECTS + 0x400 + MODIFIER_ENTRIES + 12, 3);
        assert_eq!(
            modifier_demand(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                STATE,
            ),
            Err("advisor modifier cache header invalid")
        );
    }

    #[test]
    fn modifier_cache_keeps_ordered_keys_and_signed_values() {
        const POINTER: usize = 0x100;
        const DATA: usize = 0x400;
        const COUNT: usize = 9;
        let mut bytes = vec![0; DATA + COUNT * engine_native_parts::modifier_entry::STRIDE];
        let header = POINTER + MODIFIER_ENTRIES;
        bytes[header..header + 8].copy_from_slice(&(DATA as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&(COUNT as u32).to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&(COUNT as u32).to_le_bytes());
        for index in 0..COUNT {
            let start = DATA + index * engine_native_parts::modifier_entry::STRIDE;
            bytes[start..start + 4].copy_from_slice(&(index as u32 % 7).to_le_bytes());
            bytes[start + 8..start + 16].copy_from_slice(&(-(index as i64)).to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let array = modifier_entries(&memory, POINTER).unwrap();
        let mut output = vec![0; COUNT * engine_native_parts::modifier_entry::STRIDE];
        assert_eq!(
            copy_modifier_entries(&memory, array, &mut output),
            Ok(output.len())
        );
        assert_eq!(output, bytes[DATA..]);
        let short = output.len() - 1;
        assert_eq!(
            copy_modifier_entries(&memory, array, &mut output[..short]),
            Err("advisor modifier cache exceeds byte capacity")
        );
        assert_eq!(
            copy_modifier_entries(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..bytes.len() - 1],
                },
                array,
                &mut output,
            ),
            Err("advisor modifier cache entries unreadable")
        );
    }
}
