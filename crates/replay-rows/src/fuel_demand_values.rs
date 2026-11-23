use crate::value::value_struct;
use game_model::fuel_demand::{AirInputs, Modifier};

value_struct!(Modifier { raw: Option<i64>, present: bool, effective: i64 });
value_struct!(AirInputs { reached: bool, mission_first: u32, mission_selected: u32,
    mask: u32, veto: u8, counter: i32, limit: i32, override_reference: u64,
    override_resolved: bool, planes: i32, capacity_threshold: i64, capacity_divisor: i32,
    active_count: i32, base_fuel: i64, country_modifier: Modifier, ace: u64,
    ace_alive: u8, ace_assigned_wing: u64, ace_assigned_planes: Option<i32>,
    ace_count: i64, ace_cap: i64, ace_modifier: Modifier, ace_country_modifier: Modifier,
    air_weight: i64, mission_cost: i64, period: u8, region: Option<u32>,
    context_counts: [i32; 24], context_thresholds: [i64; 24] });
