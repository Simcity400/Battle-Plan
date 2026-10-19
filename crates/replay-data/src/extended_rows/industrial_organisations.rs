//! Military-industrial organisation operands. Each column changes independently:
//! daily funds do not rewrite trait trees, tasks, variables or policies.
use super::*;
pub use engine_native::cequipment_variant::OrganisationBonus as IndustrialOrganisationBonus;
pub use engine_production::nindustrial_organisation_corganisation::details::{
    Policy as IndustrialOrganisationPolicy, State as IndustrialOrganisationState,
    Task as IndustrialOrganisationTask, Technology as IndustrialOrganisationTechnology,
    Variable as IndustrialOrganisationVariable, Variables as IndustrialOrganisationVariables,
};

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
value_struct!(engine_scripts::cscript_flag::Row {
    token: u32,
    set_hour: i32,
    value: i16,
    remaining_days: i16
});
opaque_value!(Option<[u32; 2]>);
opaque_value!(Vec<IndustrialOrganisationBonus>);
opaque_value!(Vec<IndustrialOrganisationTask>);
opaque_value!(IndustrialOrganisationVariables);
opaque_value!(Vec<engine_scripts::cscript_flag::Row>);

/// Key: full organisation reference. `country` is its holding-country link.
/// State is copied verbatim, including signed fixed5 values, the queue's cursor,
/// optional absolute cooldown hour and source-location pair. The unlocked set is
/// the game's completed-trait state; the queue preserves order and repeats.
/// Trait definitions and coordinates come from the recorded content stack.
/// Tasks preserve every native slot and
/// distinguish research template occurrences from production-line references.
/// Variables retain literal keys, exact fixed5 values and the scoped seed;
/// flags retain set dates, signed values and expiry days.
///
/// A failed details read withholds this table alone. IndustrialOrganisations
/// and IndustrialOrganisationHistory retain their independent verdicts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IndustrialOrganisationDetailsRow {
    pub organisation: u64,
    pub country: u32,
    pub state: IndustrialOrganisationState,
    pub unlocked: Vec<u32>,
    pub queued: Vec<u32>,
    pub allowed_policies: Vec<u32>,
    pub tasks: Vec<IndustrialOrganisationTask>,
    pub variables: IndustrialOrganisationVariables,
    pub flags: Vec<engine_scripts::cscript_flag::Row>,
}
typed_codec!(IndustrialOrganisationDetailsRow, 64, |row| row.organisation as u128,
    |key| Self { organisation: key as u64, ..Default::default() }, {
        0 => (country): U32, 1 => (state.funds_raw): I64,
        2 => (state.size): I32, 3 => (state.points): I32,
        4 => (state.research_bonus_raw): I64, 5 => (state.task_capacity): I32,
        6 => (state.research_assign_cost_raw): I64, 7 => (state.production_assign_cost_raw): I64,
        8 => (state.design_team_change_cost): I32, 9 => (state.funds_gain_factor_raw): I64,
        10 => (state.size_cost_factor_raw): I64, 11 => (state.upgrades): U8,
        12 => (state.queue_index): I32, 13 => (state.policy): U32,
        14 => (state.cooldown_hour): OptionalI32, 15 => (state.source_location): Opaque,
        16 => (unlocked): U32List, 17 => (queued): U32List, 18 => (allowed_policies): U32List,
        19 => (tasks): Opaque, 20 => (variables): Opaque, 21 => (flags): Opaque
    }, |_row| Ok(()));

/// Every country's complete policy roster, in native order, including empty
/// rosters. Costs and cooldown days are stored per-country policy values;
/// the selected policy and absolute cooldown belong to each organisation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IndustrialOrganisationPoliciesRow {
    pub country: u32,
    pub policies: Vec<IndustrialOrganisationPolicy>,
}
typed_codec!(IndustrialOrganisationPoliciesRow, 32, |row| row.country as u128,
    |key| Self { country: key as u32, ..Default::default() }, { 0 => (policies): Opaque }, |_row| Ok(()));
