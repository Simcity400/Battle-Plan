//! Rows and values the registry's tests declare through the row codecs, which only this crate
//! can expand.
use super::*;
use crate::display::Rounding;
use crate::value::value_struct;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const TENTH: Step = Step::fixed5(10_000, Rounding::TowardZero);
pub const WHOLE: Step = Step::fixed5(100_000, Rounding::HalfAwayFromZero);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Gauge {
    pub id: u32,
    pub level: i64,
}
value_struct!(Gauge { id: u32, level @ TENTH: i64 });
opaque_value!(Vec<Gauge>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SteppedRow {
    pub id: u64,
    pub exact: i64,
    pub level: i64,
    pub optional: Option<i64>,
    pub recorded: Recorded<i64>,
    pub pair: [i64; 2],
    pub gauges: Vec<Gauge>,
}
typed_codec!(SteppedRow, 64, |row| row.id as u128,
 |key| Self { id: key as u64, ..Default::default() },
 { 0 => (exact): I64, 1 => (level): I64 @ TENTH, 2 => (optional): OptionalI64 @ TENTH, 3 => (recorded): I64 @ WHOLE, 4 => (pair): Opaque @ TENTH, 5 => (gauges): Opaque }, |_row| Ok(()));

/// [`CountedRow`]'s key-field checks so far.
pub static COUNTED_ROW_CHECKS: AtomicUsize = AtomicUsize::new(0);
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CountedRow {
    pub key: u32,
    pub text: Vec<u8>,
}
typed_codec!(CountedRow, 32, |row| row.key as u128,
|key| Self { key: key as u32, ..Default::default() },
{ 0 => (text): Bytes }, |row| {
    COUNTED_ROW_CHECKS.fetch_add(1, Ordering::Relaxed);
    if row.text.is_empty() { return Err(invalid("synthetic missing text")); }
    Ok(())
});

/// The format-101 layout of a land combat side, before its hover-only
/// and oracle-only parts retired.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoricalSide {
    pub committed: Vec<u64>,
    pub reserves: Vec<u64>,
    pub tactic_id: i32,
    pub tactic_kind: u8,
    pub tactic_counter_id: u32,
    pub tactic_token: Vec<u8>,
    pub air_counts: Vec<i32>,
    pub anti_air_attack: i64,
    pub air_kills: u32,
    pub damage: [i64; 6],
    pub organisation_loss: [i64; 24],
    pub organisation_loss_count: u32,
    pub commander: u64,
    pub commander_skill: i32,
    pub commander_category: i32,
    pub commander_country: u32,
    pub commander_experience: i64,
    pub commander_name_token: Vec<u8>,
    pub commander_description_token: Vec<u8>,
    pub commander_formation: u64,
    pub presentation: CombatCommanderPresentation,
    pub traits: Vec<LandCombatCommanderTrait>,
    pub timed_traits: Vec<LandCombatCommanderTimedTrait>,
    pub learning_traits: Vec<LandCombatCommanderLearningTrait>,
}
value_struct!(HistoricalSide { committed: Vec<u64>, reserves: Vec<u64>, tactic_id: i32, tactic_kind: u8, tactic_counter_id: u32, tactic_token: Vec<u8>, air_counts: Vec<i32>, anti_air_attack: i64, air_kills: u32, damage: [i64; 6], organisation_loss: [i64; 24], organisation_loss_count: u32, commander: u64, commander_skill: i32, commander_category: i32, commander_country: u32, commander_experience: i64, commander_name_token: Vec<u8>, commander_description_token: Vec<u8>, commander_formation: u64, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, timed_traits: Vec<LandCombatCommanderTimedTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait> });
