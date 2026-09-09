//! Peace action object field read through a peace claim by the old D97 producer.
//!
//! The descriptive name does not assert a resolved RTTI class. The old producer
//! read the token without establishing a complete action-state walk or proving
//! that the object remains valid at the hourly boundary.
//!
//! Source: archive tag `archive/pre-rewrite-2026-09-09`,
//! `extractor/native/lanes/design_catalog_context_live/src/a40_peace_evidence.rs`,
//! lines 133-134 and 930-945. Its 0x200 evidence capture was not a struct-size proof.

/// Peace action +0x08, raw u32 token; old capture_claim read it from the action pointer obtained at claim+0x08.
pub const TOKEN: usize = 0x08;
