//! Each country's strategic operative as the Operatives map reads it: the
//! mission and operation activity the agency's operatives raise against the
//! country, and the control-trade influence and diplomatic pressure other
//! countries hold over it. Both maps live on the target country's object and
//! are keyed by the influencing country.
//!
//! The manager's pointer vector is indexed by country slot, as the network
//! walk ([`crate::ccountry_intel_network`]) reads it; the save keys each
//! object by that slot. A null entry has no object and no row.

use crate::{
    Memory, ccountry, cgame_state, cpdx_array::Array, cstrategic_operative as strategic,
    cstrategic_operative_manager as manager, memory::Raw, operation_token_map, read, read_slice,
    walk::Outcome,
};

/// Map entry's stored Robin Hood hash, u32.
pub const ENTRY_HASH: usize = 0x00;
/// Map entry's probe-distance byte: zero is empty; the end sentinel is all ones.
pub const ENTRY_PROBE: usize = 0x04;
/// Map entry's raw country key, u32: the influencing country.
pub const ENTRY_COUNTRY: usize = 0x08;
/// Control-trade entry stride.
pub const TRADE_STRIDE: usize = 0x18;
/// Control-trade influence, signed i64 fixed5.
pub const TRADE_VALUE: usize = 0x10;
/// Diplomatic-pressure entry stride.
pub const PRESSURE_STRIDE: usize = 0x28;
/// Diplomatic-pressure acceptance, signed i64 Q15 (saved as `opinion`).
pub const PRESSURE_ACCEPTANCE: usize = 0x10;
/// Diplomatic-pressure tension, signed i64 Q15.
pub const PRESSURE_TENSION: usize = 0x18;
// Both entry kinds hold their first value at the same offset.
const _: () = assert!(TRADE_VALUE == PRESSURE_ACCEPTANCE);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Acquisition identity for certified auxiliary readers; never recorded.
    pub native_pointer: usize,
    /// The object's raw owner country reference.
    pub country: u32,
    /// Its index in the manager's vector, the country slot the save keys by.
    pub slot: u32,
    pub mission_activity: i64,
    pub operation_activity: i64,
    /// The row's entries in the caller's arena: `trade_len` control-trade
    /// entries from `entries_start`, then `pressure_len` diplomatic-pressure
    /// entries, each map in native table order.
    pub entries_start: usize,
    pub trade_len: usize,
    pub pressure_len: usize,
}

/// One map entry. A control-trade entry holds its influence in `value` and a
/// zero `tension`; a diplomatic-pressure entry holds its acceptance in
/// `value` and its tension.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The influencing country's raw reference.
    pub owner: u32,
    pub value: i64,
    pub tension: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    /// The already-acquired manager membership for auxiliary bootstrap reads.
    /// None when the manager is disabled or acquisition failed.
    pub objects: Option<Array>,
    pub entries_len: usize,
    /// Entries every object needs, including omitted rows.
    pub required_entries: usize,
}

/// Every country slot's strategic operative. A row whose entries do not fit
/// is omitted and counted; sizing walks validate every entry.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    entries: &mut [Entry],
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(entries),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    entries: &mut Vec<Entry>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(entries),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut entries: crate::walk::Buffer<'_, Entry>,
) -> Walk {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let manager_pointer =
            pointer(memory, game_state, cgame_state::STRATEGIC_OPERATIVE_MANAGER)?;
        // The constructor leaves this root null when La Resistance is disabled.
        if manager_pointer == 0 {
            return Ok(Walk {
                outcome: Outcome::Rows { len: 0 },
                objects: None,
                entries_len: 0,
                required_entries: 0,
            });
        }
        let objects = at(manager_pointer, manager::OPERATIVES_DATA).and_then(|address| {
            Array::read(memory, address, manager::OPERATIVE_STRIDE)
                .ok_or("strategic operative vector unreadable or invalid")
        })?;
        let expected = base
            .checked_add(strategic::VTABLE_RVA)
            .ok_or("strategic operative vtable address overflow")?;
        let (mut len, mut unread_rows) = (0_usize, 0_u32);
        let (mut entries_len, mut required_entries) = (0_usize, 0_usize);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let object = objects
                .element(slot, manager::OPERATIVE_STRIDE)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|object| usize::try_from(object).ok())
                .ok_or("strategic operative slot unreadable or outside the vector")?;
            if object == 0 {
                continue;
            }
            if read::<u64>(memory, object) != Some(expected as u64) {
                return Err("strategic operative vtable mismatch");
            }
            let owner = field::<u32>(memory, object, strategic::OWNER)?;
            if countries.resolve(owner)?.pointer != country.pointer {
                return Err("strategic operative owner does not match its country slot");
            }
            let mission_activity = field(memory, object, strategic::MISSION_ACTIVITY)?;
            let operation_activity = field(memory, object, strategic::OPERATION_ACTIVITY)?;
            let trade = Map::read(
                memory,
                at(object, strategic::TRADE_INFLUENCE)?,
                TRADE_STRIDE,
            )?;
            let pressure = Map::read(
                memory,
                at(object, strategic::DIPLOMATIC_PRESSURE)?,
                PRESSURE_STRIDE,
            )?;
            let count = (trade.count as usize)
                .checked_add(pressure.count as usize)
                .ok_or("strategic operative entry count overflow")?;
            required_entries = required_entries
                .checked_add(count)
                .ok_or("strategic operative entry demand overflow")?;
            entries.ensure(required_entries)?;
            output.ensure(
                len.checked_add(1)
                    .ok_or("strategic operative row demand overflow")?,
            )?;
            let fits = len < output.len() && count <= entries.len() - entries_len;
            let mut written = 0;
            for (map, tension) in [(trade, None), (pressure, Some(PRESSURE_TENSION))] {
                map.visit(memory, &countries, |owner, slot| {
                    if fits {
                        entries[entries_len + written] = Entry {
                            owner,
                            value: word(slot, TRADE_VALUE),
                            tension: tension.map_or(0, |at| word(slot, at)),
                        };
                        written += 1;
                    }
                })?;
            }
            if !fits {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("strategic operative unread row count overflow")?;
                continue;
            }
            output[len] = Row {
                native_pointer: object,
                country: owner,
                slot,
                mission_activity,
                operation_activity,
                entries_start: entries_len,
                trade_len: trade.count as usize,
                pressure_len: pressure.count as usize,
            };
            len += 1;
            entries_len += count;
        }
        Ok(Walk {
            objects: Some(objects),
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            entries_len,
            required_entries,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        objects: None,
        outcome: Outcome::Unavailable { reason },
        entries_len: 0,
        required_entries: 0,
    })
}

/// A Robin Hood map header with the operation-token maps' layout
/// ([`operation_token_map`]): a table of `mask + 1 + overflow` slots
/// followed by an end sentinel. An empty map before its first insertion has
/// a zero mask and overflow and points at shared static storage, which is
/// not read.
#[derive(Debug, Clone, Copy)]
struct Map {
    data: usize,
    count: u32,
    slots: usize,
    mask: u32,
    overflow: u8,
    stride: usize,
}

/// A signed i64 at `at` in one copied map slot.
fn word(slot: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(slot[at..at + 8].try_into().unwrap())
}

impl Map {
    fn read(memory: &impl Memory, map: usize, stride: usize) -> Result<Self, &'static str> {
        let header = at(map, operation_token_map::TABLE).and_then(|address| {
            read::<[u8; 17]>(memory, address).ok_or("strategic operative map header unreadable")
        })?;
        let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "strategic operative map pointer overflow")?;
        let count = i32::from_le_bytes(header[8..12].try_into().unwrap());
        let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
        let overflow = header[16];
        if count < 0 {
            return Err("strategic operative map count negative");
        }
        if mask == 0 && overflow == 0 {
            return if count == 0 {
                Ok(Self {
                    data,
                    count: 0,
                    slots: 0,
                    mask,
                    overflow,
                    stride,
                })
            } else {
                Err("strategic operative static map has live entries")
            };
        }
        let buckets = mask
            .checked_add(1)
            .ok_or("strategic operative map mask overflow")?;
        if !buckets.is_power_of_two()
            || !(2..=1 << 30).contains(&buckets)
            || u32::from(overflow) != buckets.trailing_zeros() + 2
            || count as u32 > buckets
        {
            return Err("strategic operative map geometry invalid");
        }
        let slots = buckets as usize + usize::from(overflow);
        if data == 0 {
            return Err("strategic operative map storage null");
        }
        let sentinel = slots
            .checked_mul(stride)
            .and_then(|extent| data.checked_add(extent))
            .and_then(|end| end.checked_add(ENTRY_PROBE))
            .ok_or("strategic operative map extent overflow")?;
        if read::<u8>(memory, sentinel) != Some(u8::MAX) {
            return Err("strategic operative map end sentinel invalid or unreadable");
        }
        Ok(Self {
            data,
            count: count as u32,
            slots,
            mask,
            overflow,
            stride,
        })
    }

    /// Every occupied entry's key and copied slot in table order, after
    /// checking its probe distance and its key, and that the census equals
    /// the count.
    fn visit<M: Memory>(
        &self,
        memory: &M,
        countries: &ccountry::Reader<'_, M>,
        mut each: impl FnMut(u32, &[u8]),
    ) -> Result<(), &'static str> {
        const BATCH: usize = 64;
        let mut bytes = [0_u8; BATCH * PRESSURE_STRIDE];
        let mut observed = 0_u32;
        let mut first = 0;
        while first < self.slots {
            let batch = (self.slots - first).min(BATCH);
            let address = first
                .checked_mul(self.stride)
                .and_then(|offset| self.data.checked_add(offset))
                .ok_or("strategic operative map address overflow")?;
            let chunk = &mut bytes[..batch * self.stride];
            if !read_slice(memory, address, chunk) {
                return Err("strategic operative map slot unreadable");
            }
            for (offset, slot) in chunk.chunks_exact(self.stride).enumerate() {
                let index = first + offset;
                let distance = slot[ENTRY_PROBE];
                match distance {
                    0 => continue,
                    u8::MAX => return Err("strategic operative map sentinel inside census"),
                    _ => {}
                }
                let hash = u32::from_le_bytes(slot[ENTRY_HASH..ENTRY_HASH + 4].try_into().unwrap());
                let home = (hash & self.mask) as usize;
                if distance > self.overflow || home + usize::from(distance) - 1 != index {
                    return Err("strategic operative map probe distance invalid");
                }
                observed = observed
                    .checked_add(1)
                    .filter(|&observed| observed <= self.count)
                    .ok_or("strategic operative map census differs from count")?;
                let country =
                    u32::from_le_bytes(slot[ENTRY_COUNTRY..ENTRY_COUNTRY + 4].try_into().unwrap());
                if country != 0 {
                    countries.resolve(country)?;
                }
                each(country, slot);
            }
            first += batch;
        }
        if observed != self.count {
            return Err("strategic operative map census differs from count");
        }
        Ok(())
    }
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("strategic operative address overflow")
}

fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or("strategic operative field unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "strategic operative pointer overflow")
}

#[cfg(test)]
mod tests;
