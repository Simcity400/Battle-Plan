
use engine_sites::Rva;
use engine_native_memory::{Memory, msvc_string, read};

pub const TOKEN: usize = 0x120;
pub const VTABLE_RVA: Rva = Rva(0x0286_8570);
pub const FIXED_RANDOM_SEED: usize = 0x962;
pub const TARGETED_TIMED: usize = 0xb00;

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
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
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

pub const CANCEL_EFFECT_RVA: Rva = Rva(0x0077_0120);
pub const REMOVE_EFFECT_RVA: Rva = Rva(0x0077_0360);
pub const TARGETED_CANCEL_EFFECT_RVA: Rva = Rva(0x0077_05a0);
pub const TARGETED_REMOVE_EFFECT_RVA: Rva = Rva(0x0077_0be0);
pub const TARGETED_EXECUTE_EFFECT_RVA: Rva = Rva(0x0077_08c0);
pub const MISSION_COMPLETION_RETURN_RVA: Rva = Rva(0x0077_f3f9);

pub const EVENT_FEED_ENTRIES: [(&str, Rva); 5] = [
    ("DecisionCancellationEffects", CANCEL_EFFECT_RVA),
    ("DecisionRemovalEffects", REMOVE_EFFECT_RVA),
    ("TargetedDecisionCancellationEffects", TARGETED_CANCEL_EFFECT_RVA),
    ("TargetedDecisionRemovalEffects", TARGETED_REMOVE_EFFECT_RVA),
    ("TargetedDecisionExecuted", TARGETED_EXECUTE_EFFECT_RVA),
];
