//! CHasDLCTrigger constants; its mask is the runtime feature mask, not the launcher roster.

/// Required DLC feature mask, u32; zero always returns false.
pub const REQUIRED_MASK: usize = 0x58;
/// DLC trigger evaluation requires a nonzero mask and runtime feature eligibility.
pub const EVALUATE_RVA: usize = 0x003d_40b0;
/// CHasDLCTrigger interned `has_dlc` keyword.
pub const KEYWORD_ID: u32 = 0x2d1b;
/// CHasDLCTrigger keyword registration instruction, not function entry.
pub const KEYWORD_REGISTRATION_SITE_RVA: usize = 0x000e_94ae;
/// DLC trigger accepts any scope.
pub const ACCEPTED_SCOPE_MASK: u32 = 0;

/// CHasDLCTrigger allocating factory.
pub const FACTORY_RVA: usize = 0x003e_40b0;
/// CHasDLCTrigger allocation size.
pub const SIZE: usize = 0x60;
/// DLC accepted-scope implementation returns zero.
pub const ACCEPTED_SCOPE_RVA: usize = 0x0012_0030;
