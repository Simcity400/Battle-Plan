//! Counter and province hover text, fresh route queries and order products.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod map {
    pub mod counter_content;
    pub mod counter_eta;
    pub mod counter_tooltips;
    pub mod division_eta;
    pub mod movement_routes;
    pub mod occupation_modifier_tooltips;
    pub mod orders;
    pub mod province_tooltips;
}
