pub mod adapter;
pub mod dll_collector;
pub mod extractor;
pub mod scheduled;
pub mod static_semantics;

#[allow(dead_code)]
#[path = "../domain47_compact.rs"]
pub mod domain47_characters_leaders_cabinet_live;

pub use domain47_characters_leaders_cabinet_live::{
    decode_payload, encode_compact_data_record, encode_payload, ActiveRulerRow, AdvisorRow,
    CharacterLaneRow, CharacterPortraitRow, CharacterRow, CountryLeaderRoleRow,
    Domain44GateSummary, UnitLeaderRow, UnitLeaderTraitRow, DOMAIN_ID, DOMAIN_NAME,
    SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS, SCHEMA_ROOTS_VALIDATED, SCHEMA_STATIC_NAME_CLOSURE,
    STATUS_BRIDGE_UNAVAILABLE, STATUS_PARTIAL_DATA, STATUS_SOURCE_UNAVAILABLE, STATUS_WORKER_PANIC,
    UNIT_LEADER_ARMY, UNIT_LEADER_NAVY,
};
