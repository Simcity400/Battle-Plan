//! CDecision definition identity. Completion/effect functions are retained sites. Authored
//! definitions store the token's folded FNV-1a hash. The default placeholder stores the 0xFFFFFFFF
//! sentinel and is outside content reach. Effect ownership follows the authored-definition argument
//! and its token. The untyped hook ABI does not establish the concrete receiver class.

use crate::{Memory, msvc_string, read};

/// CDecision MSVC string token.
pub const TOKEN: usize = 0x120;
/// CDecision primary vtable.
pub const VTABLE_RVA: usize = 0x0286_8550;
/// Native fixed_random_seed content byte.
pub const FIXED_RANDOM_SEED: usize = 0x962;
/// Activation selects the timed targeted container when this byte is nonzero.
pub const TARGETED_TIMED: usize = 0xb00;

/// Exact token location. The caller copies bytes into its preallocated arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token {
    pub address: usize,
    pub len: usize,
}

pub(crate) fn token(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<Token, &'static str> {
    if definition == 0 {
        return Err("decision definition is null");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, definition) != Some(expected as u64) {
        return Err("decision definition vtable mismatch");
    }
    let address = definition
        .checked_add(TOKEN)
        .ok_or("decision token address overflow")?;
    let header = read::<[u8; 32]>(memory, address).ok_or("decision token header unreadable")?;
    let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("decision token header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "decision token length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "decision token pointer overflow")?
    };
    if source == 0 {
        return Err("decision token pointer is null");
    }
    let terminator = source
        .checked_add(len)
        .ok_or("decision token extent overflow")?;
    if read::<u8>(memory, terminator) != Some(0) {
        return Err("decision token terminator unreadable or invalid");
    }
    Ok(Token {
        address: source,
        len,
    })
}

/// Authored-decision cancel-effect entry; observes invocation only.
pub const CANCEL_EFFECT_RVA: usize = 0x0077_00b0;
/// Authored-decision removal-effect entry; observes invocation only.
pub const REMOVE_EFFECT_RVA: usize = 0x0077_02f0;
/// Three ordinary terminal-effect entry prefixes.
pub const TERMINAL_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x81, 0xec, 0xf0, 0x00,
];
/// Authored targeted-decision cancel-effect entry.
pub const TARGETED_CANCEL_EFFECT_RVA: usize = 0x0077_0530;
/// Authored targeted-decision removal-effect entry.
pub const TARGETED_REMOVE_EFFECT_RVA: usize = 0x0077_0b70;
/// Targeted cancel/removal entry prefixes.
pub const TARGETED_TERMINAL_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Targeted decision's execute-effect entry.
pub const TARGETED_EXECUTE_EFFECT_RVA: usize = 0x0077_0850;
/// The ordinary timed-mission success caller of execution.
pub const MISSION_COMPLETION_RETURN_RVA: usize = 0x0077_f389;

/// Event-feed-owned decision entries. Targeted timeout is forwarded by the
/// politics owner and therefore has no second patch here.
pub const EVENT_FEED_ENTRIES: [(&str, usize, [u8; 16]); 5] = [
    (
        "DecisionCancellationEffects",
        CANCEL_EFFECT_RVA,
        TERMINAL_EFFECT_SENTINEL,
    ),
    (
        "DecisionRemovalEffects",
        REMOVE_EFFECT_RVA,
        TERMINAL_EFFECT_SENTINEL,
    ),
    (
        "TargetedDecisionCancellationEffects",
        TARGETED_CANCEL_EFFECT_RVA,
        TARGETED_TERMINAL_EFFECT_SENTINEL,
    ),
    (
        "TargetedDecisionRemovalEffects",
        TARGETED_REMOVE_EFFECT_RVA,
        TARGETED_TERMINAL_EFFECT_SENTINEL,
    ),
    (
        "TargetedDecisionExecuted",
        TARGETED_EXECUTE_EFFECT_RVA,
        TARGETED_TERMINAL_EFFECT_SENTINEL,
    ),
];
