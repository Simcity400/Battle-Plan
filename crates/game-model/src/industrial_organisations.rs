#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct State {
    pub research_bonus_raw: i64,
    pub task_capacity: i32,
    pub research_assign_cost_raw: i64,
    pub production_assign_cost_raw: i64,
    pub design_team_change_cost: i32,
    pub funds_gain_factor_raw: i64,
    pub size_cost_factor_raw: i64,
    pub funds_raw: i64,
    pub size: i32,
    pub points: i32,
    pub upgrades: u8,
    pub queue_index: i32,
    pub policy: u32,
    pub cooldown_hour: Option<i32>,
    /// Optional native source-file index and line; inactive storage is not read.
    pub source_location: Option<[u32; 2]>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[derive(replay_spool::Spooled)]
pub struct Policy {
    pub token: u32,
    pub cost: i32,
    pub days: i32,
    pub country: u32,
    pub name: Vec<u8>,
    pub cached_score_raw: i64,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryPolicies {
    pub country: u32,
    pub policies: Vec<Policy>,
    /// The country's effective policy-cost modifier in signed fixed5, read in
    /// the same visit; `None` when its accumulator could not be read.
    pub cost_modifier_raw: Option<i64>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Technology {
    pub country: u32,
    pub handle: u32,
    pub index: i32,
    pub token: u32,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Task {
    /// Every native task slot is retained, including empty slots.
    pub technology: Option<Technology>,
    pub line: Option<u64>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Variable {
    pub name: Vec<u8>,
    pub value_raw: i64,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Variables {
    pub seed: Option<u32>,
    pub entries: Vec<Variable>,
}
