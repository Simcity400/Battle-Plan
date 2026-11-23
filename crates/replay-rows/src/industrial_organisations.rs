use game_model::equipment::OrganisationBonus as IndustrialOrganisationBonus;
use game_model::industrial_organisations::{
    Policy as IndustrialOrganisationPolicy, State as IndustrialOrganisationState,
    Task as IndustrialOrganisationTask, Technology as IndustrialOrganisationTechnology,
    Variable as IndustrialOrganisationVariable, Variables as IndustrialOrganisationVariables,
};

use crate::value::value_struct;

value_struct!(IndustrialOrganisationState {
    research_bonus_raw: i64, task_capacity: i32, research_assign_cost_raw: i64,
    production_assign_cost_raw: i64, design_team_change_cost: i32,
    funds_gain_factor_raw: i64, size_cost_factor_raw: i64, funds_raw: i64,
    size: i32, points: i32, upgrades: u8, queue_index: i32, policy: u32,
    cooldown_hour: Option<i32>, source_location: Option<[u32; 2]>,
});
value_struct!(IndustrialOrganisationBonus {
    key: u32,
    value_raw: i64
});
value_struct!(IndustrialOrganisationPolicy {
    token: u32,
    cost: i32,
    days: i32,
    country: u32,
    name: Vec<u8>,
    cached_score_raw: i64
});
opaque_value!(Vec<IndustrialOrganisationPolicy>);
value_struct!(IndustrialOrganisationTechnology {
    country: u32,
    handle: u32,
    index: i32,
    token: u32
});
value_struct!(IndustrialOrganisationTask { technology: Option<IndustrialOrganisationTechnology>, line: Option<u64> });
value_struct!(IndustrialOrganisationVariable { name: Vec<u8>, value_raw: i64 });
value_struct!(IndustrialOrganisationVariables { seed: Option<u32>, entries: Vec<IndustrialOrganisationVariable> });
value_struct!(game_model::script_flag::Row {
    token: u32,
    set_hour: i32,
    value: i16,
    remaining_days: i16
});
opaque_value!(Option<[u32; 2]>);
opaque_value!(Vec<IndustrialOrganisationBonus>);
opaque_value!(Vec<IndustrialOrganisationTask>);
opaque_value!(IndustrialOrganisationVariables);
opaque_value!(Vec<game_model::script_flag::Row>);
