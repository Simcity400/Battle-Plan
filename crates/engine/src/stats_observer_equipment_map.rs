//! Equipment-loss map embedded in NCombatLog::CStatsObserver at +0x70.
//!
//! The engine's exact C++ map specialization is unproved; this descriptive
//! module retains the vector location established by the old reader.

/// Observer equipment-map variant/amount vector; old land_combat_detail_events_live/x2_carriers.rs:174,5328-5335 reads its CPdxArray-like header.
pub const VARIANTS: usize = 0x20;
