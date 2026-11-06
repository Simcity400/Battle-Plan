//! Owned observation at `peace_finalizer_site`, distinct from hourly walks.
//! History uses in-order country nodes, turn order and action order; statuses
//! three and four are excluded. Reduced transfers remain separate because a
//! split demand need not transfer a state to its taker.

use crate::{
    Memory, cpdx_array::Array, peace_action, peace_conference as conference,
    peace_history_node as history, read,
};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Action {
    pub country: u32,
    pub turn: u32,
    pub ordinal: u32,
    pub operands: peace_action::Row,
    pub name_len: usize,
    pub name_start: usize,
    /// The native SSO length is storage, not a text cap; longer names use Output.
    pub name_inline: [u8; crate::msvc_string::INLINE_CAPACITY as usize],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Transfer {
    pub state: i32,
    pub taker: u32,
    /// Native automatic civil-war list, with no invented action token/giver.
    pub automatic: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Subject {
    pub overlord: u32,
    pub subject: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub id: Option<u64>,
    pub winners: usize,
    pub losers: usize,
    pub actions: usize,
    pub transfers: usize,
    pub subjects: usize,
    pub name_bytes: usize,
    /// First unavailable name's exact required byte count, for off-thread growth.
    pub required_name_bytes: usize,
    /// Lower bound on unread operands; stop at the first capacity limit.
    pub omitted: usize,
    /// First read/authentication failure; partial operands cannot be exact.
    pub failure: Option<&'static str>,
}

pub struct Output<'a> {
    pub winners: &'a mut [u32],
    pub losers: &'a mut [u32],
    pub actions: &'a mut [Action],
    pub transfers: &'a mut [Transfer],
    pub subjects: &'a mut [Subject],
    pub name_bytes: &'a mut [u8],
}

/// Worker resident-bid buffers sized from original country/action counts and retained name lengths.
pub struct GrowingOutput<'a> {
    pub winners: crate::walk::Buffer<'a, u32>,
    pub losers: crate::walk::Buffer<'a, u32>,
    pub actions: crate::walk::Buffer<'a, Action>,
    pub transfers: crate::walk::Buffer<'a, Transfer>,
    pub subjects: crate::walk::Buffer<'a, Subject>,
    pub name_bytes: crate::walk::Buffer<'a, u8>,
}
impl<'a> From<Output<'a>> for GrowingOutput<'a> {
    fn from(output: Output<'a>) -> Self {
        use crate::walk::Buffer::Fixed;
        Self {
            winners: Fixed(output.winners),
            losers: Fixed(output.losers),
            actions: Fixed(output.actions),
            transfers: Fixed(output.transfers),
            subjects: Fixed(output.subjects),
            name_bytes: Fixed(output.name_bytes),
        }
    }
}

fn array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    width: usize,
) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, width))
        .ok_or("peace settlement array unreadable or invalid")
}

fn element<T: crate::memory::Raw>(
    memory: &impl Memory,
    array: Array,
    index: usize,
) -> Result<T, &'static str> {
    array
        .element(index as u32, size_of::<T>())
        .and_then(|at| read(memory, at))
        .ok_or("peace settlement array element unreadable")
}

fn validate_last<T: crate::memory::Raw>(
    memory: &impl Memory,
    array: Array,
) -> Result<(), &'static str> {
    if let Some(last) = array.count.checked_sub(1) {
        let _: T = element(memory, array, last as usize)?;
    }
    Ok(())
}

fn remember_node(seen: &mut HashSet<usize>, node: usize) -> Result<(), &'static str> {
    if seen.contains(&node) {
        return Err("peace history cycle");
    }
    crate::allocation::reserve_set(seen, 1).map_err(|_| "peace demand allocation failed")?;
    seen.insert(node);
    Ok(())
}

/// One attempt, no allocation, retries, game calls or live pointers in output.
pub fn copy(memory: &impl Memory, base: usize, pointer: usize, output: Output<'_>) -> Snapshot {
    copy_internal(memory, base, pointer, output.into(), true, false)
}

/// Resident pre-finalization bids, including statuses excluded from settlement.
/// Used only for the independent fresh-save comparison, at their sampled hour.
pub fn copy_bids(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    output: Output<'_>,
) -> Snapshot {
    copy_internal(memory, base, pointer, output.into(), false, false)
}

/// Worker-only continuation from the first acquired headers, without retries.
pub fn copy_bids_growing(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    output: GrowingOutput<'_>,
) -> Snapshot {
    copy_internal(memory, base, pointer, output, false, false)
}

/// Off-thread sizing from backed native arrays, including rejected bids and empty turns.
/// No payload is recorded; cached-name headers supply their variable byte demand.
pub fn demand(memory: &impl Memory, base: usize, pointer: usize) -> Snapshot {
    crate::allocation::advisory(|| demand_advisory(memory, base, pointer))
}

fn demand_advisory(memory: &impl Memory, base: usize, pointer: usize) -> Snapshot {
    copy_internal(
        memory,
        base,
        pointer,
        Output {
            winners: &mut [],
            losers: &mut [],
            actions: &mut [],
            transfers: &mut [],
            subjects: &mut [],
            name_bytes: &mut [],
        }
        .into(),
        false,
        true,
    )
}

fn add_count(count: &mut usize, additional: usize) -> Result<(), &'static str> {
    *count = count
        .checked_add(additional)
        .ok_or("peace demand size overflow")?;
    Ok(())
}

fn copy_internal(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    mut output: GrowingOutput<'_>,
    settled: bool,
    measuring: bool,
) -> Snapshot {
    let mut result = Snapshot::default();
    let attempt = (|| {
        let identity =
            read::<[u8; 16]>(memory, pointer).ok_or("peace settlement identity unreadable")?;
        if engine_sites::at(base, conference::VTABLE_RVA).map(|at| at as u64)
            != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
        {
            return Err("peace settlement vtable mismatch");
        }
        let id = u64::from_le_bytes(identity[conference::REFERENCE..].try_into().unwrap());
        if id == 0 {
            return Err("peace settlement reference is null");
        }
        result.id = Some(id);
        let finalized = pointer
            .checked_add(conference::FINALIZED)
            .and_then(|at| read::<u8>(memory, at))
            .ok_or("peace finalized byte unreadable")?;
        if finalized > 1 || (settled && finalized != 1) {
            return Err("peace settlement finalized byte differs");
        }
        for (offset, destination, count) in [
            (
                conference::WINNERS,
                &mut output.winners,
                &mut result.winners,
            ),
            (conference::LOSERS, &mut output.losers, &mut result.losers),
        ] {
            let countries = array(memory, pointer, offset, 4)?;
            if measuring {
                validate_last::<u32>(memory, countries)?;
                *count = countries.count as usize;
                continue;
            }
            destination.ensure(countries.count as usize)?;
            for index in 0..countries.count as usize {
                if *count == destination.len() {
                    result.omitted = countries.count as usize - index;
                    return Ok(());
                }
                destination[*count] = element::<u32>(memory, countries, index)?;
                *count += 1;
            }
        }
        if !settled && !measuring && finalized == 1 {
            return Ok(());
        }
        let tree = pointer
            .checked_add(conference::HISTORY)
            .and_then(|at| read::<[u8; 16]>(memory, at))
            .ok_or("peace history header unreadable")?;
        let head = u64::from_le_bytes(tree[..8].try_into().unwrap()) as usize;
        let expected = u64::from_le_bytes(tree[8..].try_into().unwrap());
        if measuring {
            let state = engine_sites::at(base, crate::cgame_state::SINGLETON_RVA)
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("peace demand country registry unreadable")?
                as usize;
            let countries = array(memory, state, crate::cgame_state::COUNTRIES, 8)?;
            validate_last::<u64>(memory, countries)?;
            // History keys select countries through the live game-state country roster.
            if expected > u64::from(countries.count) {
                return Err("peace history exceeds country registry");
            }
        }
        // Only off-thread sizing allocates cycle-detection storage, as nodes are read.
        let mut seen = measuring.then(HashSet::new);
        let mut links = measuring.then(HashSet::new);
        // A valid tree traverses at most three links per native node.
        let step_limit = usize::try_from(expected)
            .ok()
            .and_then(|count| count.checked_mul(3))
            .ok_or("peace history size overflow")?;
        let sentinel = history::copy(memory, head)?;
        if !sentinel.nil {
            return Err("peace history sentinel is not nil");
        }
        let mut node = sentinel.left;
        let mut visited = 0_u64;
        let mut steps = 0_usize;
        while node != head {
            if visited >= expected {
                return Err("peace history size or cycle mismatch");
            }
            let current = history::copy(memory, node)?;
            if current.nil {
                return Err("peace history contains a foreign sentinel");
            }
            if measuring {
                validate_last::<u64>(memory, current.turns)?;
            }
            if let Some(seen) = seen.as_mut() {
                remember_node(seen, node)?;
            }
            visited += 1;
            for turn in 0..current.turns.count as usize {
                let turn_pointer = element::<u64>(memory, current.turns, turn)? as usize;
                let (country, actions) =
                    crate::peace_bidding_turn::copy(memory, base, turn_pointer)?;
                if country != current.country {
                    return Err("peace bidding turn country mismatch");
                }
                if measuring {
                    validate_last::<u64>(memory, actions)?;
                    add_count(&mut result.actions, actions.count as usize)?;
                    for ordinal in 0..actions.count as usize {
                        let pointer = element::<u64>(memory, actions, ordinal)? as usize;
                        let name = peace_action::cached_name(memory, pointer)?;
                        if name.len > crate::msvc_string::INLINE_CAPACITY as usize {
                            add_count(&mut result.name_bytes, name.len)?;
                        }
                    }
                    continue;
                }
                output.actions.ensure(
                    result
                        .actions
                        .checked_add(actions.count as usize)
                        .ok_or("peace action size overflow")?,
                )?;
                for ordinal in 0..actions.count as usize {
                    let action_pointer =
                        || element::<u64>(memory, actions, ordinal).map(|at| at as usize);
                    // An excluded bid needs no storage, so it can neither fill nor truncate.
                    if settled
                        && !peace_action::included_in_settlement(memory, base, action_pointer()?)?
                    {
                        continue;
                    }
                    if result.actions == output.actions.len() {
                        result.omitted = actions.count as usize - ordinal;
                        return Ok(());
                    }
                    let action_pointer = action_pointer()?;
                    let operands = peace_action::copy(memory, base, action_pointer)?;
                    if !settled || operands.included_in_settlement() {
                        let mut action = Action {
                            country,
                            turn: turn as u32,
                            ordinal: ordinal as u32,
                            operands,
                            ..Default::default()
                        };
                        let name = peace_action::cached_name(memory, action_pointer)?;
                        action.name_len = name.len;
                        if name.len <= action.name_inline.len() {
                            name.copy(memory, &mut action.name_inline)?;
                        } else {
                            let end = result
                                .name_bytes
                                .checked_add(name.len)
                                .ok_or("peace name size overflow")?;
                            output.name_bytes.ensure(end)?;
                            let Some(bytes) = output.name_bytes.get_mut(result.name_bytes..end)
                            else {
                                result.required_name_bytes = name.len;
                                result.omitted = actions.count as usize - ordinal;
                                return Ok(());
                            };
                            name.copy(memory, bytes)?;
                            action.name_start = result.name_bytes;
                            result.name_bytes = end;
                        }
                        output.actions[result.actions] = action;
                        result.actions += 1;
                    }
                }
            }
            if let Some(links) = links.as_mut() {
                links.clear();
                remember_node(links, node)?;
            }
            // Native successor traversal; a bounded step count refuses corrupt links.
            if current.right != head {
                node = current.right;
                loop {
                    steps = steps.checked_add(1).ok_or("peace history size overflow")?;
                    if steps > step_limit {
                        return Err("peace history cycle");
                    }
                    if let Some(links) = links.as_mut() {
                        remember_node(links, node)?;
                    }
                    let next = history::copy(memory, node)?;
                    if next.nil {
                        return Err("peace history contains a foreign sentinel");
                    }
                    if next.left == head {
                        break;
                    }
                    node = next.left;
                }
            } else {
                let mut child = node;
                node = current.parent;
                while node != head {
                    steps = steps.checked_add(1).ok_or("peace history size overflow")?;
                    if steps > step_limit {
                        return Err("peace history cycle");
                    }
                    if let Some(links) = links.as_mut() {
                        remember_node(links, node)?;
                    }
                    let parent = history::copy(memory, node)?;
                    if parent.right != child {
                        break;
                    }
                    child = node;
                    node = parent.parent;
                }
            }
        }
        if visited != expected {
            return Err("peace history size mismatch");
        }
        if !settled && !measuring {
            return Ok(());
        }
        for (offset, vtable, automatic) in [
            (
                conference::WINNER_PARTICIPANTS,
                crate::conference_winner_participant::VTABLE_RVA,
                true,
            ),
            (
                conference::LIBERATED_PARTICIPANTS,
                crate::conference_liberated_participant::VTABLE_RVA,
                false,
            ),
            (
                conference::SUBJECT_PARTICIPANTS,
                crate::conference_subject_participant::VTABLE_RVA,
                false,
            ),
            (
                conference::GOVERNMENT_PARTICIPANTS,
                crate::conference_force_government_participant::VTABLE_RVA,
                false,
            ),
        ] {
            let participants = array(memory, pointer, offset, 8)?;
            if measuring {
                validate_last::<u64>(memory, participants)?;
            }
            if !measuring && offset == conference::SUBJECT_PARTICIPANTS {
                output.subjects.ensure(
                    result
                        .subjects
                        .checked_add(participants.count as usize)
                        .ok_or("peace subject size overflow")?,
                )?;
            }
            for index in 0..participants.count as usize {
                if !measuring
                    && offset == conference::SUBJECT_PARTICIPANTS
                    && result.subjects == output.subjects.len()
                {
                    result.omitted = participants.count as usize - index;
                    return Ok(());
                }
                let participant = element::<u64>(memory, participants, index)? as usize;
                let expected =
                    engine_sites::at(base, vtable).ok_or("peace beneficiary vtable mismatch")?;
                if read::<u64>(memory, participant) != Some(expected as u64) {
                    return Err("peace beneficiary vtable mismatch");
                }
                let (mut taker, states) =
                    crate::conference_beneficiary_participant::copy(memory, participant)?;
                let result_offset = if offset == conference::SUBJECT_PARTICIPANTS {
                    Some(crate::conference_subject_participant::RESULT_COUNTRY)
                } else if offset == conference::GOVERNMENT_PARTICIPANTS {
                    Some(crate::conference_force_government_participant::RESULT_COUNTRY)
                } else {
                    None
                };
                if let Some(result_offset) = result_offset {
                    taker = participant
                        .checked_add(result_offset)
                        .and_then(|at| read(memory, at))
                        .ok_or("peace result country unreadable")?;
                }
                if offset == conference::SUBJECT_PARTICIPANTS {
                    let overlord = participant
                        .checked_add(crate::conference_subject_participant::OVERLORD)
                        .and_then(|at| read(memory, at))
                        .ok_or("peace subject overlord unreadable")?;
                    if !measuring {
                        output.subjects[result.subjects] = Subject {
                            overlord,
                            subject: taker,
                        };
                    }
                    result.subjects += 1;
                }
                if measuring {
                    validate_last::<u64>(memory, states)?;
                    add_count(&mut result.transfers, states.count as usize)?;
                } else {
                    output.transfers.ensure(
                        result
                            .transfers
                            .checked_add(states.count as usize)
                            .ok_or("peace transfer size overflow")?,
                    )?;
                }
                for state_index in 0..if measuring { 0 } else { states.count as usize } {
                    if result.transfers == output.transfers.len() {
                        result.omitted = states.count as usize - state_index;
                        return Ok(());
                    }
                    let state = element::<u64>(memory, states, state_index)? as usize;
                    let vtable = engine_sites::at(base, crate::cstate::VTABLE_RVA)
                        .ok_or("peace transfer state vtable mismatch")?;
                    if read::<u64>(memory, state) != Some(vtable as u64) {
                        return Err("peace transfer state vtable mismatch");
                    }
                    let id = state
                        .checked_add(crate::cstate::ID)
                        .and_then(|at| read::<i32>(memory, at))
                        .ok_or("peace transfer state id unreadable")?;
                    output.transfers[result.transfers] = Transfer {
                        state: id,
                        taker,
                        automatic: false,
                    };
                    result.transfers += 1;
                }
                if automatic {
                    let (owner, states) =
                        crate::conference_winner_participant::copy(memory, base, participant)?;
                    if owner != taker {
                        return Err("peace automatic winner mismatch");
                    }
                    if measuring {
                        validate_last::<i32>(memory, states)?;
                        add_count(&mut result.transfers, states.count as usize)?;
                    } else {
                        output.transfers.ensure(
                            result
                                .transfers
                                .checked_add(states.count as usize)
                                .ok_or("peace transfer size overflow")?,
                        )?;
                    }
                    for state_index in 0..if measuring { 0 } else { states.count as usize } {
                        if result.transfers == output.transfers.len() {
                            result.omitted = states.count as usize - state_index;
                            return Ok(());
                        }
                        output.transfers[result.transfers] = Transfer {
                            state: element(memory, states, state_index)?,
                            taker,
                            automatic: true,
                        };
                        result.transfers += 1;
                    }
                }
            }
        }
        Ok(())
    })();
    result.failure = attempt.err();
    result
}
