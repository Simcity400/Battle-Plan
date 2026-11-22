#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub mod camouflage;
pub mod equipment_graphics;
pub mod model_labels;
pub mod model_multi_fallback;
pub mod model_pick;
pub mod model_selection;
pub mod trade_routes;
pub mod unit_models;
