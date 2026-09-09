//! Entity-description record initialized by FUN1422813b0; actual C++ class name is unproven.
//! Harvested from portable_replay/native/engine_evidence/src/wd_audio_open_gates.rs
//! at archive/pre-rewrite-2026-09-09. Presentation knowledge; no live table is enabled.

/// Entity-description initializer; wd_audio_open_gates.rs:237,320-325 names and follows the allocation.
pub const INITIALIZER_RVA: usize = 0x0228_13b0;
/// Description is a 0x460-byte local initialized by SEntityReader; wd_audio_open_gates.rs:238,325.
pub const SIZE: usize = 0x460;
/// Unidentified default-true byte initialized by FUN1422813b0; wd_audio_open_gates.rs:22.
pub const DEFAULT_TRUE_FLAG: usize = 0x40c;
/// Authored gate byte; wd_audio_open_gates.rs:242,320-335 traces gated update versus ungated creation.
pub const ANIMATION_RATE_AUTHORED: usize = 0x450;
/// Animation rate f32, default 1.0; wd_audio_open_gates.rs:243,320-335 traces both consumers.
pub const ANIMATION_RATE: usize = 0x454;
/// Unidentified field assigned native f32 bits for 1.0 by the initializer; wd_audio_open_gates.rs:324.
pub const DEFAULT_FLOAT_440: usize = 0x440;
/// Unidentified field assigned native f32 bits for 1.0 by the initializer; wd_audio_open_gates.rs:324.
pub const DEFAULT_FLOAT_44C: usize = 0x44c;
/// Field compared with definition +0x1a4 for freshness; width is unproved here, wd_audio_open_gates.rs:331.
pub const VERSION_COMPARE_FIELD: usize = 0x444;
