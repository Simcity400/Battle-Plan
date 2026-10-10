//! CAdvisor role object. A role's existence does not assert appointment: the
//! appointed advisors are the CCountryCharacters +0x58 list (`ccountry_characters.rs`).

/// Advisor-role vtable.
pub const VTABLE_RVA: usize = 0x0299_0e60;
/// Parent CCharacter pointer requiring authentication.
pub const CHARACTER: usize = 0x10;
/// Idea-token string.
pub const IDEA_TOKEN: usize = 0x30;
/// Slot string.
pub const SLOT: usize = 0x80;
/// Trait-pointer array with signed count. Trait queries search it; loading
/// fills it and serialization writes names in occurrence order.
pub const TRAITS: usize = 0xc8;
/// Cached modifier-accumulator array. Rebuilds clear it and add trait blocks;
/// factories, trait changes, upgrades and loading rebuild it. Independent own
/// blocks are added separately. Tooltip keys use a separate acquisition.
pub const MODIFIER_ENTRIES: usize = 0x1b0;
pub const MODIFIER_CAPACITY_REASON: &str = "advisor modifier cache exceeds admitted capacity";

/// Header of the stored modifier entries; values are read fresh each hour,
/// independently of any mutation-certified role-definition reuse.
pub fn modifier_entries(
    memory: &impl crate::Memory,
    pointer: usize,
) -> Result<crate::cpdx_array::Array, &'static str> {
    let header = crate::read::<[u8; 16]>(memory, crate::ccharacter::at(pointer, MODIFIER_ENTRIES)?)
        .ok_or("advisor modifier cache header unreadable")?;
    crate::cpdx_array::Array::from_header(header, crate::modifier_entry::STRIDE)
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

/// Serialization visits every advisor-map occurrence. Role key, idea and
/// slot descriptors size storage without copying their payloads.
pub fn definitions_demand(
    memory: &impl crate::Memory,
    base: usize,
    game_state: usize,
) -> Result<[DefinitionDemand; 2], &'static str> {
    let mut demand = [DefinitionDemand::default(); 2];
    crate::ccharacter_manager::visit(memory, base, game_state, |entry| {
        let total = &mut demand[entry.roster as usize];
        total.characters = total
            .characters
            .checked_add(1)
            .ok_or("advisor character count overflow")?;
        let mut tree = crate::msvc_tree_node::Cursor::read(
            memory,
            entry.pointer,
            crate::ccharacter::ADVISOR_MAP_HEAD,
        )?;
        while let Some(node) = tree.next(memory)? {
            let role = crate::read::<u64>(
                memory,
                crate::ccharacter::at(node, crate::character_advisor_node::ADVISOR)?,
            )
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("advisor role pointer unreadable or null")?;
            total.rows = total
                .rows
                .checked_add(1)
                .ok_or("advisor row count overflow")?;
            for address in [
                crate::ccharacter::at(node, crate::character_advisor_node::KEY)?,
                crate::ccharacter::at(role, IDEA_TOKEN)?,
                crate::ccharacter::at(role, SLOT)?,
            ] {
                let value = crate::msvc_string::Descriptor::read(memory, address)?;
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

/// Live role and accumulator headers only. Rebuilds update the array; the
/// next hourly capture acquires its entries independently.
pub fn modifier_demand(
    memory: &impl crate::Memory,
    base: usize,
    game_state: usize,
) -> Result<ModifierDemand, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut demand = ModifierDemand::default();
    crate::ccharacter_manager::visit(memory, base, game_state, |entry| {
        let mut tree = crate::msvc_tree_node::Cursor::read(
            memory,
            entry.pointer,
            crate::ccharacter::ADVISOR_MAP_HEAD,
        )?;
        while let Some(node) = tree.next(memory)? {
            let pointer = crate::read::<u64>(
                memory,
                crate::ccharacter::at(node, crate::character_advisor_node::ADVISOR)?,
            )
            .ok_or("advisor role pointer unreadable")?;
            let pointer = usize::try_from(pointer)
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("advisor role pointer null or invalid")?;
            let vtable =
                crate::read::<u64>(memory, pointer).ok_or("advisor role vtable unreadable")?;
            if vtable != expected {
                return Err("advisor role vtable mismatch");
            }
            let parent = crate::read::<u64>(memory, crate::ccharacter::at(pointer, CHARACTER)?)
                .ok_or("advisor role character parent unreadable")?;
            if parent != entry.pointer as u64 {
                return Err("advisor role character parent mismatch");
            }
            let array = modifier_entries(memory, pointer)?;
            let tooltips = copy_tooltip_keys(memory, pointer, &mut [])?;
            demand.tooltip_bytes = demand
                .tooltip_bytes
                .checked_add(tooltips.required)
                .ok_or("advisor tooltip byte count overflow")?;
            demand.rows = demand
                .rows
                .checked_add(1)
                .ok_or("advisor row count overflow")?;
            let bytes = (array.count as usize)
                .checked_mul(crate::modifier_entry::STRIDE)
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

/// Cached tooltip string set: bucket header, occupied byte and key string.
/// Keys retain exact bytes in occupied-slot order, independently of tokens.
pub const TOOLTIP_KEYS: usize = 0x218;
pub const TOOLTIP_CAPACITY_REASON: &str = "advisor tooltip keys exceed admitted capacity";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TooltipRead {
    pub len: usize,
    pub required: usize,
    pub recorded: bool,
}

/// Visits occupied slot headers in source order; no payload is read here.
pub fn visit_tooltip_keys(
    memory: &impl crate::Memory,
    pointer: usize,
    mut visit: impl FnMut(usize, [u8; 32]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let header = crate::read::<[u8; 17]>(memory, crate::ccharacter::at(pointer, TOOLTIP_KEYS + 8)?)
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
            crate::read::<[u8; 0x28]>(memory, slot).ok_or("advisor tooltip slot unreadable")?;
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

/// Packs each occupied key as its u64 byte length followed by its bytes.
/// A short arena still visits every slot and counts its exact byte demand.
pub fn copy_tooltip_keys(
    memory: &impl crate::Memory,
    pointer: usize,
    bytes: &mut [u8],
) -> Result<TooltipRead, &'static str> {
    copy_tooltip_keys_buffer(memory, pointer, crate::walk::Buffer::Fixed(bytes))
}

pub fn copy_tooltip_keys_buffer(
    memory: &impl crate::Memory,
    pointer: usize,
    mut bytes: crate::walk::Buffer<'_, u8>,
) -> Result<TooltipRead, &'static str> {
    use crate::ccharacter::Literal;
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

/// Copies the whole ordered accumulator into a caller-owned byte slice.
pub fn copy_modifier_entries(
    memory: &impl crate::Memory,
    array: crate::cpdx_array::Array,
    bytes: &mut [u8],
) -> Result<usize, &'static str> {
    let len = (array.count as usize)
        .checked_mul(crate::modifier_entry::STRIDE)
        .ok_or("advisor modifier cache size overflow")?;
    if len > bytes.len() {
        return Err("advisor modifier cache exceeds byte capacity");
    }
    let mut first = 0;
    while first < array.count {
        let count = (array.count - first).min(64) as usize;
        let start = first as usize * crate::modifier_entry::STRIDE;
        let end = start + count * crate::modifier_entry::STRIDE;
        let address = array
            .element(first, crate::modifier_entry::STRIDE)
            .ok_or("advisor modifier cache address overflow")?;
        if !crate::read_slice(memory, address, &mut bytes[start..end]) {
            return Err("advisor modifier cache entries unreadable");
        }
        first += count as u32;
    }
    Ok(len)
}

/// Current map controls, owned for exactly one character observation. Reuse
/// still reads and validates the map header and sentinel; a miss walks through
/// these same observations rather than retrying either native read.
pub struct MapView<'a, M> {
    memory: &'a M,
    address: usize,
    head: usize,
    pub header: [u8; 16],
    pub sentinel: [u8; 26],
}
impl<'a, M: crate::Memory> MapView<'a, M> {
    pub fn read(memory: &'a M, character: usize) -> Result<Self, &'static str> {
        Self::with_sentinel(memory, character, |head| {
            crate::read::<[u8; 26]>(memory, head)
        })
    }

    /// `read` with the sentinel acquisition supplied by the caller, which may
    /// have gathered it in a batch. It is consulted only after the header
    /// checks pass, so failure order and reasons are unchanged.
    pub fn with_sentinel(
        memory: &'a M,
        character: usize,
        sentinel: impl FnOnce(usize) -> Option<[u8; 26]>,
    ) -> Result<Self, &'static str> {
        let address = crate::ccharacter::at(character, crate::ccharacter::ADVISOR_MAP_HEAD)?;
        let header = crate::read::<[u8; 16]>(memory, address)
            .ok_or("character role map header unreadable")?;
        let head = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("character role map head invalid")?;
        // Preserve the ordinary reader's count-before-sentinel failure order.
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
        crate::msvc_tree_node::Cursor::read(&view, character, crate::ccharacter::ADVISOR_MAP_HEAD)?;
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
impl<M: crate::Memory> crate::Memory for MapView<'_, M> {
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

// Factories allocate a new advisor and assign its idea and slot strings,
// so replacement changes the role pointer. Parsing can instead modify
// those members in an existing object. Pointer identity alone cannot
// certify unchanged definition bytes. A change-driven cache must cover
// parsing, construction, replacement, destruction and address reuse before
// suppressing current reads.
//
// These excluded mutator sites affect advisor roles; their receiver types
// are not established by the gate.
/// Advisor erase-effect function; its gate hook is excluded.
pub const ADVISOR_ERASE_EFFECT_RVA: usize = 0x0035_C330;
/// Advisor erase-effect entry sentinel for the excluded gate hook.
pub const ADVISOR_ERASE_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];

/// Advisor replacement-effect function; its gate hook is excluded.
pub const ADVISOR_REPLACE_EFFECT_RVA: usize = 0x0034_7DF0;
/// Advisor replacement-effect entry sentinel for the excluded gate hook.
pub const ADVISOR_REPLACE_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];

/// Advisor bulk-copy function; its gate hook is excluded.
pub const ADVISOR_BULK_COPY_RVA: usize = 0x00F8_B8A0;
/// Advisor bulk-copy entry sentinel for the excluded gate hook.
pub const ADVISOR_BULK_COPY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];

/// Advisor insertion function; its gate hook is excluded.
pub const ADVISOR_INSERT_RVA: usize = 0x00F8_D380;
/// Advisor insertion entry sentinel for the excluded gate hook.
pub const ADVISOR_INSERT_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x44, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Authenticated current CAdvisor; used only within the stopped callback.
    pub pointer: usize,
    pub character: u32,
    pub country: u32,
    /// Position in this character's native string-keyed role map.
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

/// Captures persistent advisor roles, with exact map key, idea token and slot.
/// A role's existence does not assert appointment; the appointed list is
/// CCountryCharacters +0x58 (`ccountry_characters::walk`, the HiredAdvisors table).
/// Native order is retained; sorting and row-key validation belong off-thread.
pub fn walk(
    memory: &impl crate::Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    bytes: &mut [u8],
) -> crate::ccharacter::NameWalk {
    use crate::{ccharacter::NameWalk, ccharacter_manager, walk::Outcome};
    let result = (|| {
        let mut len = 0;
        let mut bytes_len = 0;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        ccharacter_manager::visit(memory, base, game_state, |entry| {
            let end = if unread_rows == 0 { output.len() } else { len };
            let result = walk_entry_inner(
                memory,
                base,
                entry,
                &mut output[len..end],
                &mut bytes[bytes_len..],
                true,
            );
            let (added, unread) = match result.outcome {
                Outcome::Rows { len } => (len, 0),
                Outcome::Truncated { len, unread_rows } => (len, unread_rows),
                Outcome::Unavailable { reason } => return Err(reason),
            };
            for row in &mut output[len..len + added] {
                row.key_start += bytes_len;
                row.idea_start += bytes_len;
                row.slot_start += bytes_len;
                row.modifier_start += bytes_len;
            }
            len += added;
            bytes_len += result.bytes_len;
            required_bytes = required_bytes
                .checked_add(result.required_bytes)
                .ok_or("advisor string size overflow")?;
            unread_rows = unread_rows
                .checked_add(unread)
                .ok_or("advisor row count overflow")?;
            Ok(())
        })?;
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
    let result = result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    });
    append_tooltips(memory, output, bytes, result)
}

/// Walks one authenticated character. Native keys use unsigned hash order,
/// with a locale-sensitive comparator only on hash collisions.
pub fn walk_entry(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    output: &mut [Row],
    bytes: &mut [u8],
) -> crate::ccharacter::NameWalk {
    let prefix = walk_entry_inner(memory, base, entry, output, bytes, true);
    append_tooltips(memory, output, bytes, prefix)
}

/// Copies existing role fields and numeric caches before a multi-character
/// caller appends tooltip keys with `append_tooltips`.
pub fn walk_prefix_entry(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    output: &mut [Row],
    bytes: &mut [u8],
) -> crate::ccharacter::NameWalk {
    walk_entry_inner(memory, base, entry, output, bytes, true)
}

/// Mutation-certified definitions omit the independently fresh accumulator.
pub fn walk_definitions_entry(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    output: &mut [Row],
    bytes: &mut [u8],
) -> crate::ccharacter::NameWalk {
    walk_entry_inner(memory, base, entry, output, bytes, false)
}

/// Worker-only storage from each acquired role-map and string header.
pub fn walk_definitions_entry_buffer(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    output: crate::walk::Buffer<'_, Row>,
    bytes: crate::walk::Buffer<'_, u8>,
) -> crate::ccharacter::NameWalk {
    walk_entry_inner_buffer(memory, base, entry, output, bytes, false)
}

fn walk_entry_inner(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    output: &mut [Row],
    bytes: &mut [u8],
    capture_cache: bool,
) -> crate::ccharacter::NameWalk {
    walk_entry_inner_buffer(
        memory,
        base,
        entry,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(bytes),
        capture_cache,
    )
}

fn walk_entry_inner_buffer(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    mut output: crate::walk::Buffer<'_, Row>,
    mut bytes: crate::walk::Buffer<'_, u8>,
    capture_cache: bool,
) -> crate::ccharacter::NameWalk {
    use crate::{
        ccharacter::{self, Literal, NameWalk, at},
        character_advisor_node,
        msvc_tree_node::Cursor as RoleTree,
        walk::Outcome,
    };
    let result = (|| {
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
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
                        .checked_mul(crate::modifier_entry::STRIDE)
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

/// Append keys only after the complete pre-existing role/cache prefix fits.
pub fn append_tooltips(
    memory: &impl crate::Memory,
    rows: &mut [Row],
    bytes: &mut [u8],
    mut captured: crate::ccharacter::NameWalk,
) -> crate::ccharacter::NameWalk {
    for row in &mut rows[..captured.outcome.len()] {
        row.tooltip_start = captured.bytes_len;
        match copy_tooltip_keys(memory, row.pointer, &mut bytes[captured.bytes_len..]) {
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
    use crate::memory::SliceMemory;

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
        // An empty slot retains bytes that must never become a key.
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
        let mut output = [0; 36];
        let read = copy_tooltip_keys(&memory, 0, &mut output).unwrap();
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
        let read = copy_tooltip_keys(&memory, 0, &mut output[..35]).unwrap();
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
            copy_tooltip_keys(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                &mut output
            ),
            Err("advisor tooltip slot count differs from header")
        );
    }

    #[test]
    fn modifier_demand_sums_live_headers_for_5000_countries_and_propagates_faults() {
        use crate::{ccharacter, ccharacter_manager, cgame_state};
        const STATE: usize = 0x100;
        const MANAGER: usize = 0x1000;
        const ROSTER: usize = 0x2000;
        const OBJECTS: usize = 0x1_0000;
        const STRIDE: usize = 0x800;
        const COUNT: usize = 5_000;
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
        word(&mut bytes, MANAGER, ccharacter_manager::VTABLE_RVA as u64);
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
            word(&mut bytes, character, ccharacter::VTABLE_RVA as u64);
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
            bytes[head + crate::msvc_tree_node::NIL] = 1;
            word(
                &mut bytes,
                node + crate::character_advisor_node::ADVISOR,
                role as u64,
            );
            word(&mut bytes, role, VTABLE_RVA as u64);
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
    fn modifier_cache_keeps_5000_ordered_keys_and_signed_values() {
        const POINTER: usize = 0x100;
        const DATA: usize = 0x400;
        const COUNT: usize = 5_000;
        let mut bytes = vec![0; DATA + COUNT * crate::modifier_entry::STRIDE];
        let header = POINTER + MODIFIER_ENTRIES;
        bytes[header..header + 8].copy_from_slice(&(DATA as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&(COUNT as u32).to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&(COUNT as u32).to_le_bytes());
        for index in 0..COUNT {
            let start = DATA + index * crate::modifier_entry::STRIDE;
            bytes[start..start + 4].copy_from_slice(&(index as u32 % 7).to_le_bytes());
            bytes[start + 8..start + 16].copy_from_slice(&(-(index as i64)).to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let array = modifier_entries(&memory, POINTER).unwrap();
        let mut output = vec![0; COUNT * crate::modifier_entry::STRIDE];
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
