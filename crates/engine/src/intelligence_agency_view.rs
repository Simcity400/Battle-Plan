//! Agency presentation view; no RTTI class name was established by the archived reader.
//!
//! Source: `extractor/native/lanes/diplomacy_intel_operatives_live/src/a30_visibility.rs`.
//! The view owns widget pointers, not omniscient simulation state. Its acquisition is excluded.

/// Agency view name/logo widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:43,295).
pub const NAME_LOGO_WIDGET: usize = 0x630;
/// Agency view upgrade-progress widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:44,296).
pub const UPGRADE_PROGRESS_WIDGET: usize = 0x638;
/// Agency view operative-progress widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:45,297).
pub const OPERATIVE_PROGRESS_WIDGET: usize = 0x640;
/// Agency view before-creation widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:46,298).
pub const BEFORE_CREATION_WIDGET: usize = 0x648;
/// Agency view upgrades-before-creation widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:47,299).
pub const UPGRADES_BEFORE_WIDGET: usize = 0x650;
/// Agency view operatives-before-creation widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:48,300).
pub const OPERATIVES_BEFORE_WIDGET: usize = 0x658;
/// Agency view operations widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:49,307).
pub const OPERATIONS_WIDGET: usize = 0x660;
/// Agency view ciphers widget pointer; confirmed by the old exact-build visibility presenter export and reader (source:50,308).
pub const CIPHERS_WIDGET: usize = 0x668;
