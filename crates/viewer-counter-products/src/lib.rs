
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod map {
    pub mod counter_content;
    pub mod counter_eta;
    pub mod counter_tooltips;
    pub mod division_eta;
    pub mod faction_theatre_tooltips;
    pub mod movement_routes;
    pub mod occupation_modifier_tooltips;
    pub mod orders;
    pub mod province_tooltips;
}
