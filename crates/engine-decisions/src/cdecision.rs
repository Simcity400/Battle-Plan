//! CDecision definition identity. Completion/effect functions are retained sites. Authored
//! definitions store the token's folded FNV-1a hash. The default placeholder stores the 0xFFFFFFFF
//! sentinel and is outside content reach. Effect ownership follows the authored-definition argument
//! and its token. The untyped hook ABI does not establish the concrete receiver class.

use crate::{Memory, msvc_string, read};

/// CDecision MSVC string token.
pub const TOKEN: usize = 0x120;
/// CDecision primary vtable.
pub const VTABLE_RVA: usize = 0x027d_4f00;
/// Native fixed_random_seed content byte.
pub const FIXED_RANDOM_SEED: usize = 0x962;

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
pub const CANCEL_EFFECT_RVA: usize = 0x0071_b350;
/// Authored-decision removal-effect entry; observes invocation only.
pub const REMOVE_EFFECT_RVA: usize = 0x0071_b590;
/// Authored-decision timeout-effect entry; observes invocation only.
pub const TIMEOUT_EFFECT_RVA: usize = 0x0071_c490;
/// Three ordinary terminal-effect entry prefixes.
pub const TERMINAL_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x81, 0xec, 0xf0, 0x00,
];
/// Authored targeted-decision cancel-effect entry.
pub const TARGETED_CANCEL_EFFECT_RVA: usize = 0x0071_b7d0;
/// Authored targeted-decision removal-effect entry.
pub const TARGETED_REMOVE_EFFECT_RVA: usize = 0x0071_be10;
/// Targeted cancel/removal entry prefixes.
pub const TARGETED_TERMINAL_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Authored targeted-decision timeout-effect entry.
pub const TARGETED_TIMEOUT_EFFECT_RVA: usize = 0x0071_c130;
/// Exact targeted timeout-effect entry bytes.
pub const TARGETED_TIMEOUT_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x55, 0x41, 0x56,
];
