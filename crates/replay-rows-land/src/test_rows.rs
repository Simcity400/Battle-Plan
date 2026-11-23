use super::*;

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
