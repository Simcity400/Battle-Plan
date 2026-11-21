
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod map {
    pub mod air_orders;
    pub mod air_selection;
    pub mod box_selection;
    pub mod counters;
    pub mod naval_combat_grid;
    pub mod naval_combat_occurrences;
    pub mod naval_selection;
    pub mod selection_sounds;

    pub mod counter_groups;
    pub mod counter_portraits;
    pub mod unit_selection;
}
