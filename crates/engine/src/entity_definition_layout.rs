//! Entity-definition record created or reused by FUN14227d8c0; actual C++ class name is unproven.
//! Harvested from portable_replay/native/engine_evidence/src/wd_audio_open_gates.rs
//! at archive/pre-rewrite-2026-09-09. Presentation knowledge; no live table is enabled.

/// Definition allocation size; wd_audio_open_gates.rs:329 records the create/reuse path.
pub const SIZE: usize = 0x1c0;
/// Animation rate f32; wd_audio_open_gates.rs:244,327-336 traces copies from description +0x454.
pub const ANIMATION_RATE: usize = 0x1b4;
/// Field compared with description +0x444; width is unproved here, wd_audio_open_gates.rs:331.
pub const VERSION_COMPARE_FIELD: usize = 0x1a4;
/// Definition update copies the rate only when description +0x450 is set; wd_audio_open_gates.rs:269,327.
pub const UPDATE_RVA: usize = 0x0227_cef0;
/// Definition create/reuse copies rate without that gate after freshness checks; wd_audio_open_gates.rs:270,328-333.
pub const CREATE_OR_REUSE_RVA: usize = 0x0227_d8c0;
