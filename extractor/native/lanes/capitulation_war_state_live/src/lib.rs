//! Compact domain 60 `capitulation_war_state_live`: per-country capitulation /
//! surrender state (CCountry diplomacy sub-object) + the global war pool
//! (tag pair, start date, casualty accumulators, and the dormant exact H-B5
//! threat/instigator/hostility residual carrier).
//!
//! Offsets were validated 15/15 against the melt across all 437
//! live countries; war pool 958/958 melt pairs) plus the E1 wiring probe:
//! war start_date_raw i32 at row+0x10, 958/958 exact.
//!
//! Deliberately INDEPENDENT of the domain 53 record: the d53 base extract is
//! fail-closed on build 870ebf (relations root SOURCE_UNAVAILABLE), and its
//! extraction is all-or-nothing. This domain has its own country-rooted read
//! and its own war-pool discovery cache so the political map emits today.

pub mod compact;
pub mod extractor;

pub use compact::DOMAIN_ID;
