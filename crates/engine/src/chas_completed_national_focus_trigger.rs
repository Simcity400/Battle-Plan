//! Completed-national-focus trigger layout.

/// CHasCompletedNationalFocusTrigger focus-id string.
pub const FOCUS_ID: usize = 0x58;
/// Completed-national-focus trigger evaluation.
pub const EVALUATE_RVA: usize = 0x003d_3a60;
/// CHasCompletedNationalFocusTrigger interned `has_completed_focus` keyword.
pub const KEYWORD_ID: u32 = 0x340a;
/// CHasCompletedNationalFocusTrigger keyword registration instruction, not function entry.
pub const KEYWORD_REGISTRATION_SITE_RVA: usize = 0x0011_2e6c;
/// CHasCompletedNationalFocusTrigger entry registration.
pub const REGISTER_ENTRY_RVA: usize = 0x0002_1720;
/// CHasCompletedNationalFocusTrigger country-scope mask.
pub const ACCEPTED_SCOPE_MASK: u32 = 0x04;

/// CHasCompletedNationalFocusTrigger allocating factory.
pub const FACTORY_RVA: usize = 0x003e_3db0;
/// CHasCompletedNationalFocusTrigger allocation size.
pub const SIZE: usize = 0x78;
/// CHasCompletedNationalFocusTrigger accepted-scope implementation.
pub const ACCEPTED_SCOPE_RVA: usize = 0x002e_2c90;
