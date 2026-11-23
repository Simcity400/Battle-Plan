#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod adjacencies;
pub mod adjacency_rules;
pub mod ambient;
pub mod cities;
pub mod map;
pub mod movement;
pub mod positions;
pub mod province_geometry;
pub mod province_graph;
pub mod railways;
pub mod scripted_map_modes;
pub mod strategic_locations;
pub mod terrain_images;
