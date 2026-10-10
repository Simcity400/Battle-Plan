//! Graphic-pool selection metadata, separate from object layouts.

/// Matching-pool collector; visits pools in reverse order, appends through the callback and returns zero.
pub const COLLECT_MATCHING_POOLS_RVA: usize = 0x009f_1540;
/// Country-context traversal; zero-return collectors continue through country, faction, capital-region and default pools.
pub const COLLECT_CONTEXT_POOLS_RVA: usize = 0x009f_0bb0;
/// Definition/archetype traversal; collects from the archetype even when both identities coincide.
pub const COLLECT_DEFINITION_AND_ARCHETYPE_POOLS_RVA: usize = 0x009f_13f0;
/// Graphic-item lookup; compares complete keys after hashing, including collisions.
pub const FIND_COMPLETE_KEY_RVA: usize = 0x009e_fbd0;
