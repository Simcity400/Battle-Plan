//! Border-war trigger layout; its full scope includes unit leaders and combat.

/// CIsBorderWar boolean operand byte.
pub const OPERAND: usize = 0x58;
/// Border-war trigger evaluation.
pub const EVALUATE_RVA: usize = 0x003d_8ec0;
/// CIsBorderWar interned `is_border_war` keyword.
pub const KEYWORD_ID: u32 = 0x399a;
/// CIsBorderWar keyword registration instruction, not function entry.
pub const KEYWORD_REGISTRATION_SITE_RVA: usize = 0x000f_5f4c;
/// Full unit-leader/combat scope mask.
pub const ACCEPTED_SCOPE_MASK: u32 = 0x18;

/// CIsBorderWar allocating factory.
pub const FACTORY_RVA: usize = 0x003e_5ae0;
/// CIsBorderWar allocation size.
pub const SIZE: usize = 0x60;
/// Border-war accepted-scope implementation.
pub const ACCEPTED_SCOPE_RVA: usize = 0x003e_e7f0;
