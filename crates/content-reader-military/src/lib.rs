#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod combat_tactics;
pub mod doctrines;
pub mod industrial_organisations;
pub mod intelligence;
pub mod leader_traits;
pub mod mio_traits;
pub mod scientist_traits;
pub mod special_projects;
pub mod technology;
pub mod technology_sharing;
pub mod unit_medals;
