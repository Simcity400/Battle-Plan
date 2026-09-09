//! Shared modifier/define metadata table used by D43, D57, D74 and D96.
//! The old code does not establish the concrete C++ row class name.
//! A nonzero feature selector requires engine callback semantics; the old passive readers refuse it.

/// Modifier definition table data-pointer global; old build_profile.rs:103 and attach_log.rs:21069.
pub const TABLE_RVA: usize = 0x0331_69c0;
/// Modifier definition metadata row stride; old build_profile.rs:104 and D74 resolve_attach_profile.
pub const STRIDE: usize = 0x78;
/// Modifier definition flags u32; old build_profile.rs:105 and D43/D57/D74/D96 metadata reads.
pub const FLAGS: usize = 0x60;
pub const CLAMP_MAXIMUM: u32 = 1 << 4;
/// Modifier definition feature selector 32-bit word; D96 reads i32, D57/D74 read u32 and require zero.
pub const FEATURE_SELECTOR: usize = 0x68;
/// Modifier eligibility callback pointer global; old build_profile.rs:108 and attach_log.rs:21070.
pub const ELIGIBILITY_CALLBACK_RVA: usize = 0x0331_7cd0;
/// Modifier key for generating-war-goal tension; old build_profile.rs:109 and D57 threshold reader.
pub const GENERATE_WARGOAL_TENSION: u32 = 0xe9;
/// Modifier key for joining-faction tension; old build_profile.rs:110 and D57 threshold reader.
pub const JOIN_FACTION_TENSION: u32 = 0xeb;
/// Modifier key for lend-lease tension; old build_profile.rs:111 and D57 threshold reader.
pub const LEND_LEASE_TENSION: u32 = 0xec;
/// Modifier key for volunteer-sending tension; old build_profile.rs:112 and D57 threshold reader.
pub const SEND_VOLUNTEERS_TENSION: u32 = 0xef;
/// Modifier key for guarantee tension; old build_profile.rs:113 and D57 threshold reader.
pub const GUARANTEE_TENSION: u32 = 0xf0;
/// Modifier key for daily power-balance change; old d74_power_balance.rs:7 and get_modifiers.
pub const POWER_BALANCE_DAILY: u32 = 0x98;
/// Modifier key for weekly power-balance change; old d74_power_balance.rs:8 and get_modifiers.
pub const POWER_BALANCE_WEEKLY: u32 = 0x99;
/// Country preferred-tactic weighting factor global i64 fixed5; old compact_contract/tactic_weight.rs:30.
pub const COUNTRY_PREFERRED_TACTIC_FACTOR_RVA: usize = 0x0331_d370;
/// Subject export-share modifier key; old compact_market_trade_available_export reads key 0x112.
pub const SUBJECT_EXPORT_SHARE: u32 = 0x112;
/// Subject factory-cost modifier key; old compact_market_trade_factory_cost reads key 0x113.
pub const SUBJECT_FACTORY_COST: u32 = 0x113;
/// Relation export-share modifier key; old compact_market_trade_available_export reads key 0x178.
pub const RELATION_EXPORT_SHARE: u32 = 0x178;
/// Relation factory-cost modifier key; old compact_market_trade_factory_cost reads key 0x179.
pub const RELATION_FACTORY_COST: u32 = 0x179;
/// Counterpart additive factory-cost modifier key; old compact_market_trade_factory_cost reads key 0x17a.
pub const COUNTERPART_FACTORY_COST: u32 = 0x17a;
/// Trade-feature flags global u32; old attach_log.rs:21067 and compact_market_trade_feature_5_enabled.
pub const TRADE_FEATURE_FLAGS_RVA: usize = 0x0331_6e78;
/// Trade-feature-5 bit mask; old attach_log.rs:21068 authenticates this engine gate before trade derivation.
pub const TRADE_FEATURE_MASK: u32 = 0x0e;
