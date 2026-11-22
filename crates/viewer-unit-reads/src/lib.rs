#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod air_detection;
pub mod air_participants;
pub mod air_range;
pub mod air_reachability;
pub mod air_region_reach;
pub mod air_registry_sources;
pub mod air_sortie_defines;
pub mod air_transport_content;
pub mod aircraft_model;
pub mod counter_action;
pub mod counter_anchors;
pub mod counter_attrition;
pub mod counter_inputs;
pub mod counter_move;
pub mod counter_order;
pub mod counter_status;
pub mod model_admission;
pub mod movement_reads;
pub mod movement_routes;
pub mod naval_count_approx;
pub mod naval_dominance_sources;
pub mod port_mission_defines;
pub mod railway_gun_counter;
pub mod task_force_visual;
