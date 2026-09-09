//! Entity registry passed to name lookup and named allocation.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Registry named factory FUN142278710; a729_hook.rs:200 and hook_entity_named_factory retain the call.
pub const ENTITY_NAMED_FACTORY_RVA: usize = 0x0227_8710;
/// Named factory entry bytes checked by a729_hook.rs:230.
pub const ENTITY_NAMED_FACTORY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x40, 0x41, 0x8b, 0xd9, 0x41, 0x8b, 0xf8,
];
/// Registry definition lookup FUN14227e5f0; a729_hook.rs:201 and hook_entity_definition_lookup retain the call.
pub const ENTITY_DEFINITION_LOOKUP_RVA: usize = 0x0227_E5F0;
/// Definition lookup entry bytes checked by a729_hook.rs:233.
pub const ENTITY_DEFINITION_LOOKUP_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x68, 0x10, 0x48, 0x89, 0x70, 0x18, 0x48,
];
