//! CAgencyUpgrade definition identity. Branch reader@140618C40 constructs this
//! one native definition form for authored upgrades. Acquired levels are stored
//! separately on CIntelligenceAgency; reading a definition's level cap must not
//! clamp the stored acquisition value.

/// Constructor@140615D50, confirmed pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x027c_8c70;
/// Constructor@140615D50 stores the lexer token; Save@140FCA0F0 writes it.
pub const TOKEN: usize = 0x08;
/// Constructor@140615D50 sets this nonzero; Load@140FC8750 accepts nonzero.
pub const VALID: usize = 0x10;
/// Constructor@140615D50 stores the owning branch supplied by reader@140618C40.
pub const BRANCH: usize = 0x260;
