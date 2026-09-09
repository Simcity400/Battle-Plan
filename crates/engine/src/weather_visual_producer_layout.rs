//! Presentation producer pairing primary/fading entities for a strategic region, distinct from simulation weather.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Region pointer; a729_hook.rs:10792 follows it to region id +0x58.
pub const REGION: usize = 0x30;
/// Primary/fading entity-pair array pointer; a729_hook.rs:10727 and :11050.
pub const ENTITY_PAIRS: usize = 0x48;
/// Entity-pair capacity u32; a729_hook.rs:11040 reads it.
pub const PAIR_CAPACITY: usize = 0x50;
/// Entity-pair count u32; a729_hook.rs:10723 reads it.
pub const PAIR_COUNT: usize = 0x54;
/// Native weather code u32; a729_hook.rs:10793 narrows to u8 in old capture, which is not the source width.
pub const WEATHER_CODE: usize = 0x60;
/// Visual producer FUN14166ec50; lib.rs:93 and hook_weather retain its producer argument.
pub const WEATHER_VISUAL_PRODUCER_RVA: u32 = 0x0166_EC50;
/// Visual producer entry bytes checked by a729_hook.rs:254.
pub const WEATHER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x50, 0x10, 0x55, 0x53, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57,
];
