//! Shader cache presentation layout; no recorded table or hook is enabled.

/// Inline shader-program array.
pub const PROGRAMS: usize = 0x20;
/// Shader/effect lookup using this cache.
pub const ENGINE_SHADER_EFFECT_LOOKUP_RVA: u32 = 0x023F_4EC0;
/// Exact lookup-entry sentinel.
pub const ENGINE_SHADER_EFFECT_LOOKUP_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x4c, 0x89, 0x40, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55,
];
