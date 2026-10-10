//! Conjunction-trigger layout and callable metadata.

/// Evaluator stopping at the first false child.
pub const EVALUATE_RVA: usize = 0x0053_dff0;
/// Conjunction keyword token id.
pub const KEYWORD_ID: u32 = 0x2968;
/// Accepted-scope mask; zero accepts any scope.
pub const ACCEPTED_SCOPE_MASK: u32 = 0;

/// Constructor installing the vtable and keyword token.
pub const CONSTRUCTOR_RVA: usize = 0x0053_d400;
/// Accepted-scope implementation returning zero.
pub const ACCEPTED_SCOPE_RVA: usize = 0x0012_0030;

/// Trigger-check virtual byte offset.
pub const CHECK_VTABLE_OFFSET: usize = 0x18;
