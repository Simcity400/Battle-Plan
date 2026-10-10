//! Entity registry passed to name lookup and named allocation.
//!
//! Presentation layout only; this module enables no recorded table or hook.

/// Registry named factory.
pub const ENTITY_NAMED_FACTORY_RVA: usize = 0x0227_8710;
/// Named-factory entry bytes.
pub const ENTITY_NAMED_FACTORY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x40, 0x41, 0x8b, 0xd9, 0x41, 0x8b, 0xf8,
];
/// Registry definition lookup.
pub const ENTITY_DEFINITION_LOOKUP_RVA: usize = 0x0227_E5F0;
/// Definition-lookup entry bytes.
pub const ENTITY_DEFINITION_LOOKUP_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x68, 0x10, 0x48, 0x89, 0x70, 0x18, 0x48,
];
