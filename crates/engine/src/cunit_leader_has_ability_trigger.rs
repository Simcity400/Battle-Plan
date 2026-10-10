//! Unit-leader ability-trigger layout and registration operands.

/// Ability predicate; resolves the caster before checking unlocked abilities.
pub const EVALUATE_RVA: usize = 0x0044_2ad0;
/// Interned ability-trigger keyword.
pub const KEYWORD_ID: u32 = 0x38ef;
/// CUnitLeaderHasAbilityTrigger keyword registration instruction, not function entry.
pub const KEYWORD_REGISTRATION_SITE_RVA: usize = 0x000f_4176;
/// CUnitLeaderHasAbilityTrigger unit-leader scope mask.
pub const ACCEPTED_SCOPE_MASK: u32 = 0x08;

/// CUnitLeaderHasAbilityTrigger ability-token MSVC string.
pub const ABILITY_TOKEN: usize = 0x58;
/// CUnitLeaderHasAbilityTrigger allocating factory.
pub const FACTORY_RVA: usize = 0x0044_39a0;
/// CUnitLeaderHasAbilityTrigger allocation size.
pub const SIZE: usize = 0x78;
/// Accepted-scope implementation; returns the unit-leader scope mask.
pub const ACCEPTED_SCOPE_RVA: usize = 0x002e_efe0;
