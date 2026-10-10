//! Conditional trigger layout.

/// CIfTrigger embedded CAndTrigger for `limit`.
pub const LIMIT: usize = 0x58;
/// Conditional alternative branch, reached only when the limit fails.
pub const OTHERWISE: usize = 0xb0;
/// Conditional trigger evaluation.
pub const EVALUATE_RVA: usize = 0x0138_cc00;
/// CIfTrigger per-key loader.
pub const LOAD_KEY_RVA: usize = 0x0139_5b90;
/// CTriggerEntry<CIfTrigger> registration.
pub const REGISTER_ENTRY_RVA: usize = 0x0007_9840;
/// CIfTrigger interned `if` keyword.
pub const KEYWORD_ID: u32 = 0x27c3;
/// CIfTrigger `limit` keyword.
pub const LIMIT_KEYWORD_ID: u32 = 0x2a0a;
/// CIfTrigger `else_if` keyword.
pub const ELSE_IF_KEYWORD_ID: u32 = 0x38ed;
/// CIfTrigger `else` keyword.
pub const ELSE_KEYWORD_ID: u32 = 0x36d3;
/// CIfTrigger `else_if` keyword registration instruction, not function entry.
pub const ELSE_IF_REGISTRATION_SITE_RVA: usize = 0x0009_a035;
/// CIfTrigger `else` keyword registration instruction, not function entry.
pub const ELSE_REGISTRATION_SITE_RVA: usize = 0x0009_9ffc;
/// Conditional trigger accepts any scope.
pub const ACCEPTED_SCOPE_MASK: u32 = 0;

/// CIfTrigger scalar-deleting destructor.
pub const SCALAR_DELETING_DESTRUCTOR_RVA: usize = 0x0053_df20;
/// CIfTrigger allocation size.
pub const SIZE: usize = 0xb8;
/// Conditional accepted-scope implementation returns zero.
pub const ACCEPTED_SCOPE_RVA: usize = 0x0012_0030;
