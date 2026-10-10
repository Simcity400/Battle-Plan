//! CIsLeadingArmyGroup constants; this is the exact trigger class name from its registry entry.

/// CIsLeadingArmyGroup boolean operand byte.
pub const OPERAND: usize = 0x58;
/// Leading-army-group trigger evaluation.
pub const EVALUATE_RVA: usize = 0x0044_1cc0;
/// CIsLeadingArmyGroup interned `is_leading_army_group` keyword.
pub const KEYWORD_ID: u32 = 0x39bb;
/// CIsLeadingArmyGroup keyword registration instruction, not function entry.
pub const KEYWORD_REGISTRATION_SITE_RVA: usize = 0x000f_6588;
/// CIsLeadingArmyGroup unit-leader scope mask.
pub const ACCEPTED_SCOPE_MASK: u32 = 0x08;

/// CIsLeadingArmyGroup allocating factory.
pub const FACTORY_RVA: usize = 0x0044_3510;
/// CIsLeadingArmyGroup allocation size.
pub const SIZE: usize = 0x60;
/// CIsLeadingArmyGroup accepted-scope implementation.
pub const ACCEPTED_SCOPE_RVA: usize = 0x002e_efe0;
