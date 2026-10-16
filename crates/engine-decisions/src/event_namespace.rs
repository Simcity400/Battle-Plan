//! Event-manager namespace rows and registry roots.
//!
//! Namespace slots depend on engine registration order. Numeric suffixes can
//! overflow their slot and alias another definition, so combined ids alone do
//! not prove a unique script event. The exact namespace-record class is unknown.

/// Namespace row array pointer.
pub const TABLE_RVA: usize = 0x0331_6f98;
/// Namespace array capacity, u32.
pub const CAPACITY_RVA: usize = 0x0331_6fa0;
/// Namespace array count, u32.
pub const COUNT_RVA: usize = 0x0331_6fa4;
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
/// Bare event-id exclusive limit.
pub const BARE_EVENT_ID_LIMIT: u32 = 1_000_000;
/// Namespace and numeric-ID resolver.
pub const RESOLVE_EVENT_ID_RVA: usize = 0x001f_03a0;
/// Namespace and numeric-ID arithmetic site.
pub const EVENT_ID_FORMULA_RVA: usize = 0x001f_055f;
/// Namespace-registration handler.
pub const REGISTER_RVA: usize = 0x00a0_1920;
/// Case-folded namespace hash.
pub const HASH_NAME_RVA: usize = 0x00bb_a1b0;
/// Case-insensitive namespace comparison.
pub const COMPARE_NAME_RVA: usize = 0x0253_d9f8;
/// Numeric suffix reducer; does not validate every digit.
pub const PARSE_NUMERIC_SUFFIX_RVA: usize = 0x024b_8700;
/// Bare event-ID bound validator.
pub const VALIDATE_BARE_EVENT_ID_RVA: usize = 0x0116_b780;
