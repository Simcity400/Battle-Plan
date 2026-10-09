//! Operation completion observation, before the manager can destroy an instance.
//!
//! P: Completion@1413EE390 has one indexed caller, CALL@14118B5BD in
//! 14118B110. Its successful AL=1 return@1413EF440 follows bonus store
//! 1413EE9D5, description assignment/trim@1413EEB8B/1413EEBA0, terminal
//! store@1413EEBE6 and detection store@1413EEF2D. It does not destroy this.
//! The caller subsequently handles auto-repeat@14118B91A/14118B940 and
//! AI cleanup@14118BC8E/14118BE6C. The observer copies every successful
//! call before returning to that caller, without a controller predicate.
//! P: Invalidation@141188FC0 -> predicate1413EE2C0 does not complete or
//! store outcomes; its cleanup/deletion is cancellation, not a completion.

use crate::{
    Memory, ccharacter::Literal, ccountry, cgame_state, coperation,
    coperation_instance as operation, read,
};

/// P: 1413EE390 entry. The five-byte patch/relocation is only MOV [RSP+18],RBX.
/// It contains no CALL return PC, RIP-relative operand or interior branch
/// target. The remainder is an authenticated sentinel, not a patched span.
pub const COMPLETION_RVA: usize = 0x013e_e390;
pub const COMPLETION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub id: u64,
    pub target: u32,
    pub definition_token: u32,
    pub completion_hour: i32,
    pub terminal: u8,
    pub bonus: u8,
    pub detected: u8,
}

/// Callback-local validated description. Its game pointer cannot be exposed
/// to an off-thread consumer; only the copied bytes and Row leave the callback.
pub struct Observation {
    pub row: Row,
    description: Literal,
}
impl Observation {
    pub fn description_len(&self) -> usize {
        self.description.len
    }
    pub fn copy_description(
        &self,
        memory: &impl Memory,
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        Literal::copy_all(memory, [&self.description], output)
            .map_err(|_| "operation completion description bytes unreadable")
    }
    /// Copy the stored +0xA0 string (assignment@1413EEB8B) across a byte-ring
    /// boundary. Both destinations belong to this callback's lane.
    pub fn copy_description_parts(
        &self,
        memory: &impl Memory,
        first: &mut [u8],
        second: &mut [u8],
    ) -> Result<(), &'static str> {
        const UNREADABLE: &str = "operation completion description bytes unreadable";
        if first.len().checked_add(second.len()) != Some(self.description.len) {
            return Err(UNREADABLE);
        }
        if second.is_empty() {
            return self.copy_description(memory, first);
        }
        if self.description.len <= 15 {
            let mut inline = [0; 15];
            self.copy_description(memory, &mut inline[..self.description.len])?;
            first.copy_from_slice(&inline[..first.len()]);
            second.copy_from_slice(&inline[first.len()..self.description.len]);
            return Ok(());
        }
        let source = self.description.source();
        let tail = source.checked_add(first.len()).ok_or(UNREADABLE)?;
        if memory.copy_scattered([source, tail], &mut [first, second]) != 2 {
            return Err(UNREADABLE);
        }
        Ok(())
    }
}

/// Only the successful native return is admitted. No pre-completion read is
/// needed: terminal calls return AL=0 and never produce a duplicate event.
pub fn observe(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Observation, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("operation completion address overflow")
    };
    let expected = base
        .checked_add(operation::VTABLE_RVA)
        .ok_or("operation completion module overflow")?;
    let identity =
        read::<[u8; 16]>(memory, pointer).ok_or("operation completion identity unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
        return Err("operation completion instance vtable mismatch");
    }
    let id = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if id == 0 {
        return Err("operation completion null handle");
    }
    let definition_pointer = read::<u64>(memory, at(operation::DEFINITION)?)
        .ok_or("operation completion definition unreadable")? as usize;
    let definition_token = coperation::definition(memory, base, definition_pointer)?;
    let owner_pointer = read::<u64>(memory, at(operation::OWNER_COUNTRY)?)
        .ok_or("operation completion owner unreadable")? as usize;
    let owner = read::<[u8; 16]>(memory, owner_pointer)
        .ok_or("operation completion owner identity unreadable")?;
    if u64::from_le_bytes(owner[..8].try_into().unwrap())
        != base
            .checked_add(ccountry::VTABLE_RVA)
            .ok_or("operation completion module overflow")? as u64
    {
        return Err("operation completion owner vtable mismatch");
    }
    let owner = u32::from_le_bytes(owner[ccountry::ID..ccountry::ID + 4].try_into().unwrap());
    if owner == 0 {
        return Err("operation completion null owner");
    }
    let target = read::<u32>(memory, at(operation::TARGET_COUNTRY_ID)?)
        .ok_or("operation completion target unreadable")?;
    let terminal = read::<u8>(memory, at(operation::TERMINAL)?)
        .ok_or("operation completion terminal unreadable")?;
    let outcome = read::<[u8; 34]>(memory, at(operation::EFFECT_DESCRIPTION)?)
        .ok_or("operation completion outcomes unreadable")?;
    if terminal != 1 || outcome[32..].iter().any(|&flag| flag > 1) {
        return Err("operation completion stored flags invalid");
    }
    let header = outcome[..32].try_into().unwrap();
    if u64::from_le_bytes(outcome[16..24].try_into().unwrap()) > operation::DESCRIPTION_MAX_LENGTH
        || u64::from_le_bytes(outcome[24..32].try_into().unwrap())
            > operation::DESCRIPTION_MAX_LENGTH
    {
        return Err("operation completion description exceeds native string maximum");
    }
    let description = Literal::from_header(at(operation::EFFECT_DESCRIPTION)?, header)
        .map_err(|_| "operation completion description header invalid")?;
    Literal::validate(memory, [&description])
        .map_err(|_| "operation completion description terminator unreadable or invalid")?;
    let state = read::<u64>(
        memory,
        base.checked_add(cgame_state::SINGLETON_RVA)
            .ok_or("operation completion clock address overflow")?,
    )
    .ok_or("operation completion clock unreadable")? as usize;
    let completion_hour = read::<i32>(
        memory,
        state
            .checked_add(cgame_state::CURRENT_HOUR)
            .ok_or("operation completion hour address overflow")?,
    )
    .ok_or("operation completion hour unreadable")?;
    Ok(Observation {
        row: Row {
            owner,
            id,
            target,
            definition_token,
            completion_hour,
            terminal,
            bonus: outcome[32],
            detected: outcome[33],
        },
        description,
    })
}
