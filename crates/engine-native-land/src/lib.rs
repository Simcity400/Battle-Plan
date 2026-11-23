#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

pub mod battle_sounds;
pub mod combat_sources;
pub mod division_combat_inputs;
pub mod division_panel_operands;
pub mod invasion_preparations;
pub mod land_combat_details;
pub mod land_reinforcement_inputs;
pub mod land_tactic_inputs;
pub mod order_execution;
pub mod supply_node;
pub mod supply_parent_map;
pub mod supply_route;
pub mod template_hq_context;
pub mod unit_history;
