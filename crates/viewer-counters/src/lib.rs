//! Native counters modules with no host or sibling feature dependency.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod map {
    pub mod air_selection;
    pub mod counters;
    pub mod naval_combat_grid;
    pub mod naval_combat_occurrences;
    pub mod naval_selection;

    pub mod counter_groups;
    pub mod counter_portraits;
    pub mod unit_selection;
}
