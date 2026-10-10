//! Country-flag trigger. Comparison operands belong to its flag-trigger base.

/// Evaluator resolving a country flag before applying its comparison.
pub const EVALUATE_RVA: usize = 0x003d_01f0;
/// Country-flag keyword token id.
pub const KEYWORD_ID: u32 = 0x29c8;
/// Keyword registration instruction site.
pub const KEYWORD_REGISTRATION_SITE_RVA: usize = 0x000a_63b3;
/// Country-scope mask.
pub const ACCEPTED_SCOPE_MASK: u32 = 0x04;

/// Allocating factory installing the vtable.
pub const FACTORY_RVA: usize = 0x003e_2cc0;
/// Allocation size.
pub const SIZE: usize = 0xb0;
/// Accepted-scope implementation.
pub const ACCEPTED_SCOPE_RVA: usize = 0x002e_2c90;
