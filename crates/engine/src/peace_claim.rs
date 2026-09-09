//! Peace claim object field from the old D97 claim/contest observation callbacks.
//!
//! No RTTI class name or persistent ownership was established by this reader.
//! The presentation hooks that supplied the pointer are excluded from the new
//! collector. An hourly source and lifetime proof are still required.
//!
//! Source: archive tag `archive/pre-rewrite-2026-09-09`,
//! `extractor/native/lanes/design_catalog_context_live/src/a40_peace_evidence.rs`,
//! lines 131-132 and 930-945. Its 0x40 evidence capture was not a struct-size proof.

/// Peace claim +0x08, process-local action pointer; old capture_claim read usize and rejected a null pointer.
pub const ACTION: usize = 0x08;
