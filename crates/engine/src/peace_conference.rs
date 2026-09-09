//! Peace conference object fields retained from the old D97 observation path.
//!
//! The descriptive module name is not an RTTI class-name claim. The old reader
//! resolved the GUI's reference through the shared reference resolver and checked
//! both reference words against this object. It did not prove an hourly walk of
//! all conferences. Completion semantics and the object's writers need reassessment
//! before a state walker is added; the old transport reduced the raw word to bool.
//!
//! Source: archive tag `archive/pre-rewrite-2026-09-09`,
//! `extractor/native/lanes/design_catalog_context_live/src/a40_peace_evidence.rs`,
//! lines 127-130 and 818-863. Evidence capture lengths were not object sizes.

/// Peace conference +0x08, type/id u32 reference pair; old capture_conference_refresh back-joined it to the GUI handle.
pub const REFERENCE: usize = 0x08;
/// Peace conference +0xD8, raw i32 turn; read_i32 in old capture_conference_refresh confirms the reader width.
pub const CURRENT_TURN: usize = 0xd8;
/// Peace conference +0x170, raw 64-bit completion-related storage; old capture_conference_refresh read usize then tested nonzero, not a proven bool layout.
pub const COMPLETION_STORAGE: usize = 0x170;
