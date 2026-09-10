//! CDecision definition identity. Completion/effect functions are retained knowledge only.
//! Effect ownership follows the authored-definition argument and its +0x120 token read;
//! the old untyped hook ABI does not independently establish the methods' RTTI owner.

use crate::{Memory, msvc_string, read};

/// CDecision MSVC string token; offsets.rs:292 and direct_frd_transitions_live/a729_hook.rs:41,1374 agree on the read.
pub const TOKEN: usize = 0x120;
pub const VTABLE_RVA: usize = 0x027d_4f00;

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

/// Authored-decision cancel-effect entry FUN14071b350; direct_frd_transitions_live/lib.rs:62 and hook_decision_cancel_effect observe invocation only.
pub const CANCEL_EFFECT_RVA: usize = 0x0071_b350;
/// Authored-decision removal-effect entry FUN14071b590; direct_frd_transitions_live/lib.rs:63 and hook_decision_remove_effect observe invocation only.
pub const REMOVE_EFFECT_RVA: usize = 0x0071_b590;
/// Authored-decision timeout-effect entry FUN14071c490; direct_frd_transitions_live/lib.rs:64 and hook_decision_timeout_effect observe invocation only.
pub const TIMEOUT_EFFECT_RVA: usize = 0x0071_c490;
/// Three ordinary terminal-effect entry prefixes; direct_frd_transitions_live/a729_hook.rs:72-74,150-160 bind these same bytes to all three RVAs.
pub const TERMINAL_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x81, 0xec, 0xf0, 0x00,
];
/// Authored targeted-decision cancel effect FUN14071b7d0; direct_frd_transitions_live/lib.rs:65 and prepare_terminal_row read definition token +0x120.
pub const TARGETED_CANCEL_EFFECT_RVA: usize = 0x0071_b7d0;
/// Authored targeted-decision removal effect FUN14071be10; direct_frd_transitions_live/lib.rs:66 and prepare_terminal_row read definition token +0x120.
pub const TARGETED_REMOVE_EFFECT_RVA: usize = 0x0071_be10;
/// Targeted cancel/removal entry prefixes; direct_frd_transitions_live/a729_hook.rs:75-77,165-170 bind these bytes to both excluded hooks.
pub const TARGETED_TERMINAL_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Authored targeted-decision timeout effect FUN14071c130; direct_frd_transitions_live/lib.rs:67 and prepare_terminal_row read definition token +0x120.
pub const TARGETED_TIMEOUT_EFFECT_RVA: usize = 0x0071_c130;
/// FUN14071c130 entry prefix; direct_frd_transitions_live/a729_hook.rs:78-80,175 bound and checked these bytes before its excluded hook.
pub const TARGETED_TIMEOUT_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x55, 0x41, 0x56,
];
