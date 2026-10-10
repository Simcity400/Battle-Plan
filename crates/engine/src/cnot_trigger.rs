//! Negated trigger layout. No child may be true; this does not negate
//! a conjunction.

/// Negated trigger returns false when any child evaluates true.
pub const EVALUATE_RVA: usize = 0x0053_e070;
/// CNotTrigger interned `not` keyword.
pub const KEYWORD_ID: u32 = 0x296a;
/// Negated trigger accepts any scope.
pub const ACCEPTED_SCOPE_MASK: u32 = 0;

/// CNotTrigger allocating factory.
pub const FACTORY_RVA: usize = 0x0054_02c0;
/// CNotTrigger allocation size.
pub const SIZE: usize = 0x58;
/// Negated accepted-scope implementation returns zero.
pub const ACCEPTED_SCOPE_RVA: usize = 0x0012_0030;
