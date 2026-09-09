//! CVariables container shared by country, global and state script scopes.
//!
//! The old D51 collector verifies the RTTI name and caches the observed vtable;
//! its sources do not pin a vtable RVA. The container's hash-node start and
//! occupancy rules still need derivation before a new hourly walker is written.

/// CVariables RTTI type name; old country_variables_extractor::read_country_variables and D51 domain51_variable_scope require this exact name.
pub const RTTI_NAME: &str = ".?AVCVariables@@";
/// CVariables primary vtable pointer; old stable_header_pair_with_vtable reads it at the beginning of the object.
pub const VTABLE: usize = 0;
/// CVariables hash-storage base pointer; old country_variables_extractor::stable_header_pair_windowless reads eight bytes at +0x18.
pub const STORAGE: usize = 0x18;
/// CVariables declared entry count u32; old stable_header extracts the low half of the word at +0x20.
pub const ENTRY_COUNT: usize = 0x20;
/// CVariables bucket mask u32; old stable_header extracts the high half of the word at +0x20, with bucket count mask+1.
pub const BUCKET_MASK: usize = 0x24;
