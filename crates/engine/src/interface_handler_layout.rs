//! Interface-handler selection operands reached from the in-game idler.

/// Selected adjacency-rule-area pointer.
pub const SELECTED_ADJACENCY_RULE_AREA: usize = 0x418;
/// Signed i32 building-selection discriminator.
pub const BUILDING_SELECTION_KIND: usize = 0x458;
/// Shared province/state selection pointer; its kind selects exactly one interpretation.
pub const BUILDING_SELECTION: usize = 0x460;
/// Province-selection discriminator; zero selects province.
pub const BUILDING_SELECTION_PROVINCE: i32 = 0;
/// State-selection discriminator; one selects state.
pub const BUILDING_SELECTION_STATE: i32 = 1;
/// Selected adjacency-rule-area getter.
pub const GET_SELECTED_ADJACENCY_RULE_AREA_RVA: usize = 0x00b5_75a0;
/// Province-selection getter; requires kind zero.
pub const GET_BUILDING_PROVINCE_RVA: usize = 0x00b5_7850;
/// State-selection getter; requires kind one.
pub const GET_BUILDING_STATE_RVA: usize = 0x00b5_8340;
/// Selected province-pair set builder.
pub const BUILD_SELECTED_PROVINCE_PAIRS_RVA: usize = 0x00b5_57d0;
/// Supply-icon eligibility word, 32 bits; values one and three admit display. Other enum meanings are unspecified.
pub const SUPPLY_ICON_ELIGIBILITY_WORD: usize = 0x408;
