//! Resource calculation receiver obtained through country resources.
//! Its country word at +0x18 is distinct from country-object +8.

/// Adjusted resource amount: applies modifier 0x39, then folds on the same receiver.
pub const MODIFIER_AMOUNT_RVA: usize = 0x00ca_5340;
/// Own-resource-country predicate using country-tag equivalence with receiver +0x18.
pub const IS_OWN_RESOURCE_COUNTRY_RVA: usize = 0x00ca_5510;
/// Resource-country resolver: matching table value, else address of receiver +0x18.
pub const RESOURCE_COUNTRY_RVA: usize = 0x00ca_4fc0;
/// Amount fold receiving the fixed5 modifier product on the same receiver.
pub const FOLD_AMOUNT_CANDIDATE_RVA: usize = 0x00ca_5200;
/// Raw country link, i32, used for fallback and comparison.
pub const COUNTRY: usize = 0x18;
/// State pointer used for state modifiers and cross-country lookup.
pub const STATE_CANDIDATE: usize = 0x80;
/// Sorted resource-country row pointer, searched by binary lookup.
pub const RESOURCE_COUNTRIES: usize = 0x3e0;
/// Sorted resource-country count, i32, bounding the search.
pub const RESOURCE_COUNTRY_COUNT: usize = 0x3ec;
/// Resource amount modifier index.
pub const RESOURCE_AMOUNT_MODIFIER: u32 = 0x39;
