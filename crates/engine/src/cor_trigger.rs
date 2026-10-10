//! Disjunction-trigger layout.

/// Evaluation stops at the first true child.
pub const EVALUATE_RVA: usize = 0x0053_e0f0;
/// Interned disjunction keyword.
pub const KEYWORD_ID: u32 = 0x2969;
/// Accepted-scope mask: zero admits any scope.
pub const ACCEPTED_SCOPE_MASK: u32 = 0;

/// Disjunction-trigger constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0053_d430;
/// Allocating factory for a disjunction trigger.
pub const FACTORY_RVA: usize = 0x0054_0310;
/// Disjunction-trigger allocation size.
pub const SIZE: usize = 0x58;
/// Accepted-scope implementation returning zero.
pub const ACCEPTED_SCOPE_RVA: usize = 0x0012_0030;
