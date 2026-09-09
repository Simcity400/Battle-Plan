//! High-resolution seed helper; function metadata without an engine-struct owner.
//! Source: extractor/native/lanes/production_render_submit_live/src/{lib.rs,a729_hook.rs}
//! at archive/pre-rewrite-2026-09-09. No event capture is enabled.

/// Seed helper FUN1424dbe60 identified by lib.rs:99; writes two native u32 words (a729_hook.rs:10647-10651).
pub const HIGH_RESOLUTION_SEED_RVA: u32 = 0x024D_BE60;
/// Seed helper entry bytes checked by a729_hook.rs:270; writes two native u32 words (a729_hook.rs:10647-10651).
pub const SEED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0xe8, 0x9e, 0xc7,
];
