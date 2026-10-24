//! Persistent country-indexed raid graph. Country handles resolve through
//! the country-link table; layout alone does not establish hourly consistency.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0297_1f98;
/// Country-raid-status array pointer.
pub const COUNTRY_STATUSES: usize = 0xd8;
/// Country-status array capacity, u32.
#[cfg(test)]
pub const COUNTRY_STATUSES_CAPACITY: usize = 0xe0;

use crate::{
    Memory, ccountry, ccountry_raid_status, cgame_state, cpdx_array::Array, craid_instance,
    craid_target, read, walk::Outcome,
};

/// One target cooldown of one country, in native order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CooldownRow {
    pub owner_slot: u32,
    pub ordinal: u32,
    pub raid_type: u32,
    pub scope: craid_target::Row,
    pub days: i32,
}

/// Caller-owned storage for [`walk`]. Rows keep ranges into the arenas.
pub struct Buffers<'a> {
    pub parents: &'a mut [ccountry_raid_status::Row],
    /// Eligible and used raid type tokens of every parent.
    pub type_tokens: &'a mut [u32],
    pub instances: &'a mut [craid_instance::Row],
    /// Raw `show_for` country handles of every instance.
    pub show_for: &'a mut [u32],
    pub cooldowns: &'a mut [CooldownRow],
    /// The caller-owned raid type token memo, sized from the raid database.
    pub type_memo: &'a mut ccountry_raid_status::TypeTokens,
}

pub struct GrowingBuffers<'a> {
    pub parents: &'a mut Vec<ccountry_raid_status::Row>,
    /// Eligible and used raid type tokens of every parent.
    pub type_tokens: &'a mut Vec<u32>,
    pub instances: &'a mut Vec<craid_instance::Row>,
    /// Raw `show_for` country handles of every instance.
    pub show_for: &'a mut Vec<u32>,
    pub cooldowns: &'a mut Vec<CooldownRow>,
    /// The caller-owned raid type token memo, sized from the raid database.
    pub type_memo: &'a mut ccountry_raid_status::TypeTokens,
}

pub struct BuffersBuffer<'a> {
    pub parents: crate::walk::Buffer<'a, ccountry_raid_status::Row>,
    /// Eligible and used raid type tokens of every parent.
    pub type_tokens: crate::walk::Buffer<'a, u32>,
    pub instances: crate::walk::Buffer<'a, craid_instance::Row>,
    /// Raw `show_for` country handles of every instance.
    pub show_for: crate::walk::Buffer<'a, u32>,
    pub cooldowns: crate::walk::Buffer<'a, CooldownRow>,
    /// The caller-owned raid type token memo, sized from the raid database.
    pub type_memo: &'a mut ccountry_raid_status::TypeTokens,
}

impl<'a> From<Buffers<'a>> for BuffersBuffer<'a> {
    fn from(value: Buffers<'a>) -> Self {
        Self {
            parents: crate::walk::Buffer::Fixed(value.parents),
            type_tokens: crate::walk::Buffer::Fixed(value.type_tokens),
            instances: crate::walk::Buffer::Fixed(value.instances),
            show_for: crate::walk::Buffer::Fixed(value.show_for),
            cooldowns: crate::walk::Buffer::Fixed(value.cooldowns),
            type_memo: value.type_memo,
        }
    }
}

impl<'a> From<GrowingBuffers<'a>> for BuffersBuffer<'a> {
    fn from(value: GrowingBuffers<'a>) -> Self {
        Self {
            parents: crate::walk::Buffer::Growing(value.parents),
            type_tokens: crate::walk::Buffer::Growing(value.type_tokens),
            instances: crate::walk::Buffer::Growing(value.instances),
            show_for: crate::walk::Buffer::Growing(value.show_for),
            cooldowns: crate::walk::Buffer::Growing(value.cooldowns),
            type_memo: value.type_memo,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub parents: Outcome,
    pub instances: Outcome,
    pub cooldowns: Outcome,
    pub type_tokens_len: usize,
    pub show_for_len: usize,
    /// Arena demand, including rows that did not fit.
    pub required_type_tokens: usize,
    pub required_show_for: usize,
    /// Instance and cooldown rows refused because their target's leader
    /// captor is unproven; counted in the truncation, never in the demand.
    pub refused_leaders: u32,
}

/// Fills `arena[*len..][..count]` through `copy`, or only counts the demand
/// when it does not fit. Returns the start of the filled range.
fn append<T: Default>(
    arena: &mut crate::walk::Buffer<'_, T>,
    len: &mut usize,
    required: &mut usize,
    count: usize,
    copy: impl FnOnce(&mut [T]) -> Result<(), &'static str>,
) -> Result<Option<usize>, &'static str> {
    *required = required
        .checked_add(count)
        .ok_or("raid arena demand overflow")?;
    arena.ensure(*required)?;
    let start = *len;
    let Some(slot) = start
        .checked_add(count)
        .and_then(|end| arena.get_mut(start..end))
    else {
        return Ok(None);
    };
    copy(slot)?;
    *len += count;
    Ok(Some(start))
}

/// The `show_for` handles, 64 per guarded call.
fn copy_handles(memory: &impl Memory, source: Array, slot: &mut [u32]) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    const STRIDE: usize = craid_instance::SHOW_FOR_COUNTRY_STRIDE;
    let mut bytes = [0_u8; BATCH * STRIDE];
    for (chunk, handles) in slot.chunks_mut(BATCH).enumerate() {
        let bytes = &mut bytes[..handles.len() * STRIDE];
        let copied = source
            .element((chunk * BATCH) as u32, STRIDE)
            .is_some_and(|at| crate::memory::read_slice(memory, at, bytes));
        if !copied {
            return Err("raid show-for handle unreadable");
        }
        for (handle, word) in handles.iter_mut().zip(bytes.chunks_exact(STRIDE)) {
            *handle = u32::from_le_bytes(word.try_into().unwrap());
        }
    }
    Ok(())
}

/// The type-token arena demand of every raid parent's eligible and used
/// lists: [`walk`] with no row storage, which copies the parents in their
/// batches and counts each list without reading it. For presizing the arena
/// before a full hour (the collector's live sizing observation).
pub fn required_type_tokens(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let walk = walk(
        memory,
        base,
        game_state,
        Buffers {
            parents: &mut [],
            type_tokens: &mut [],
            instances: &mut [],
            show_for: &mut [],
            cooldowns: &mut [],
            type_memo: &mut ccountry_raid_status::TypeTokens::empty(),
        },
    );
    match walk.parents {
        Outcome::Unavailable { reason } => Err(reason),
        _ => Ok(walk.required_type_tokens),
    }
}

/// Reads the persistent country-indexed raid graph once. Parent and instance
/// capacities are independent; a row whose arena range does not fit is left
/// out and counted, so a prefix never implies a complete census. Omitted
/// instances are counted from authenticated vector headers. No allocation.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, buffers: Buffers<'_>) -> Walk {
    walk_buffer(memory, base, game_state, buffers.into())
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: GrowingBuffers<'_>,
) -> Walk {
    walk_buffer(memory, base, game_state, buffers.into())
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: BuffersBuffer<'_>,
) -> Walk {
    let BuffersBuffer {
        mut parents,
        mut type_tokens,
        mut instances,
        mut show_for,
        mut cooldowns,
        type_memo: known,
    } = buffers;
    let mut walk = Walk {
        parents: Outcome::Rows { len: 0 },
        instances: Outcome::Rows { len: 0 },
        cooldowns: Outcome::Rows { len: 0 },
        type_tokens_len: 0,
        show_for_len: 0,
        required_type_tokens: 0,
        required_show_for: 0,
        refused_leaders: 0,
    };
    let result = (|| {
        let root = game_state
            .checked_add(cgame_state::RAID_SYSTEM)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("raid system unreadable or null")?;
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        if read::<u64>(memory, root) != Some(expected as u64) {
            return Err("raid system vtable mismatch");
        }
        let roster = root
            .checked_add(COUNTRY_STATUSES)
            .and_then(|p| Array::read(memory, p, ccountry_raid_status::STRIDE))
            .ok_or("raid parent array unreadable or invalid")?;
        if roster.count == 0 {
            let empty = Outcome::Rows { len: 0 };
            return Ok((empty, empty, empty));
        }
        parents.ensure(roster.count as usize)?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let (mut parent_len, mut unread_parents) = (0_usize, 0_u32);
        let (mut instance_len, mut unread_instances) = (0_usize, 0_u32);
        let mut instance_error = None;
        let (mut cooldown_len, mut unread_cooldowns) = (0_usize, 0_u32);
        known.begin(memory, base);
        if parents.is_growing() {
            known.prepare_worker()?;
        }
        const BATCH: usize = 64;
        let mut parent_records = [0_u8; BATCH * ccountry_raid_status::PARENT_BYTES];
        let mut cooldown_records = [0_u8; BATCH * ccountry_raid_status::COOLDOWN_BYTES];
        for ordinal in 0..roster.count {
            // Parents are copied 64 at a time, each batch when it starts.
            let in_batch = ordinal as usize % BATCH;
            if in_batch == 0 {
                let count = (roster.count - ordinal).min(BATCH as u32) as usize;
                ccountry_raid_status::copy_parents(
                    memory,
                    roster,
                    ordinal,
                    count,
                    &mut parent_records,
                )?;
            }
            let record = &parent_records[in_batch * ccountry_raid_status::PARENT_BYTES..]
                [..ccountry_raid_status::PARENT_BYTES];
            let (mut row, children) =
                ccountry_raid_status::parse_row(record, base, ordinal, &countries)?;
            let mut tokens = |array: Array| {
                append(
                    &mut type_tokens,
                    &mut walk.type_tokens_len,
                    &mut walk.required_type_tokens,
                    array.count as usize,
                    |slot| ccountry_raid_status::copy_type_tokens(memory, base, array, slot, known),
                )
            };
            let eligible = tokens(children.eligible)?;
            let used = tokens(children.used)?;
            match (eligible, used, parents.get_mut(parent_len)) {
                (Some(eligible), Some(used), Some(slot)) => {
                    row.eligible_start = eligible;
                    row.eligible_len = children.eligible.count as usize;
                    row.used_start = used;
                    row.used_len = children.used.count as usize;
                    *slot = row;
                    parent_len += 1;
                }
                _ => unread_parents += 1,
            }
            cooldowns.ensure(
                cooldown_len
                    .checked_add(children.cooldowns.count as usize)
                    .ok_or("raid cooldown count overflow")?,
            )?;
            for index in 0..children.cooldowns.count {
                let Some(slot) = cooldowns.get_mut(cooldown_len) else {
                    unread_cooldowns += children.cooldowns.count - index;
                    break;
                };
                // A country's cooldowns are copied 64 at a time.
                let in_batch = index as usize % BATCH;
                if in_batch == 0 {
                    let count = (children.cooldowns.count - index).min(BATCH as u32) as usize;
                    ccountry_raid_status::copy_cooldowns(
                        memory,
                        children.cooldowns,
                        index,
                        count,
                        &mut cooldown_records,
                    )?;
                }
                let record = &cooldown_records[in_batch * ccountry_raid_status::COOLDOWN_BYTES..]
                    [..ccountry_raid_status::COOLDOWN_BYTES];
                let (raid_type, scope, days) =
                    ccountry_raid_status::parse_cooldown(memory, base, record, known)?;
                // An unproven leader captor refuses just this row.
                if scope.leader_refused {
                    unread_cooldowns += 1;
                    walk.refused_leaders += 1;
                    continue;
                }
                *slot = CooldownRow {
                    owner_slot: ordinal,
                    ordinal: index,
                    raid_type,
                    scope,
                    days,
                };
                cooldown_len += 1;
            }
            if instance_error.is_some() {
                continue;
            }
            instances.ensure(
                instance_len
                    .checked_add(children.instances.count as usize)
                    .ok_or("raid instance count overflow")?,
            )?;
            for child_ordinal in 0..children.instances.count {
                if instance_len == instances.len() {
                    unread_instances += children.instances.count - child_ordinal;
                    break;
                }
                let row = children
                    .instances
                    .element(child_ordinal, ccountry_raid_status::POINTER_STRIDE)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("raid instance pointer unreadable or null")
                    .and_then(|address| {
                        craid_instance::read_row(
                            memory,
                            base,
                            address,
                            ordinal,
                            child_ordinal,
                            &countries,
                        )
                    })
                    .and_then(|(mut row, handles)| {
                        let start = append(
                            &mut show_for,
                            &mut walk.show_for_len,
                            &mut walk.required_show_for,
                            handles.count as usize,
                            |slot| copy_handles(memory, handles, slot),
                        )?;
                        Ok(start.map(|start| {
                            row.show_for_start = start;
                            row
                        }))
                    });
                match row {
                    // An unproven leader captor refuses just this row.
                    Ok(Some(row)) if row.target.leader_refused => {
                        unread_instances += 1;
                        walk.refused_leaders += 1;
                    }
                    Ok(Some(row)) => {
                        instances[instance_len] = row;
                        instance_len += 1;
                    }
                    Ok(None) => unread_instances += 1,
                    Err(reason) => {
                        instance_error = Some(reason);
                        break;
                    }
                }
            }
        }
        let parent_outcome = if unread_parents == 0 {
            Outcome::Rows { len: parent_len }
        } else {
            Outcome::Truncated {
                len: parent_len,
                unread_rows: unread_parents,
            }
        };
        let instance_outcome = if let Some(reason) = instance_error {
            Outcome::Unavailable { reason }
        } else {
            // Native references are the row keys; vector positions are retained
            // in each row. Sorting in place detects duplicates without a set.
            instances[..instance_len].sort_unstable_by_key(|row| row.key());
            if instances[..instance_len]
                .windows(2)
                .any(|pair| pair[0].key() == pair[1].key())
            {
                Outcome::Unavailable {
                    reason: "duplicate raid instance reference",
                }
            } else if unread_instances == 0 {
                Outcome::Rows { len: instance_len }
            } else {
                Outcome::Truncated {
                    len: instance_len,
                    unread_rows: unread_instances,
                }
            }
        };
        let cooldown_outcome = if unread_cooldowns == 0 {
            Outcome::Rows { len: cooldown_len }
        } else {
            Outcome::Truncated {
                len: cooldown_len,
                unread_rows: unread_cooldowns,
            }
        };
        Ok((parent_outcome, instance_outcome, cooldown_outcome))
    })();
    (walk.parents, walk.instances, walk.cooldowns) = result.unwrap_or_else(|reason| {
        let failed = Outcome::Unavailable { reason };
        (failed, failed, failed)
    });
    walk
}

#[cfg(test)]
mod tests;
