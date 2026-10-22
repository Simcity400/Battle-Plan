//! Event-manager namespace rows and registry roots.
//!
//! Namespace slots depend on engine registration order. Numeric suffixes can
//! overflow their slot and alias another definition, so combined ids alone do
//! not prove a unique script event. The exact namespace-record class is unknown.

/// Namespace row array pointer.
pub const TABLE_RVA: usize = 0x0331_6f98;
/// Namespace record stride.
pub const STRIDE: usize = 0x28;
/// Namespace MSVC name string.
pub const NAME: usize = 0x00;
/// Namespace case-folded FNV-1a hash.
pub const NAME_HASH: usize = 0x20;
/// Event namespace numeric span.
pub const ID_NAMESPACE_SPAN: u32 = 100_000;
/// Event namespace reserved slots.
pub const ID_FIRST_NAMESPACE_SLOT: u32 = 10;
