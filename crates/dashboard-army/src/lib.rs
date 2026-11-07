//! Army widget products: division templates, training queues, division
//! careers and division equipment, each one country or division at a reached
//! hour.
//!
//! One product crate above `dashboard-base`: the
//! modules keep their dashboard paths (`products::<name>`); the dashboard
//! re-exports and registers them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod army_content;
    pub mod designer_stats;
    pub mod division_careers;
    pub mod division_equipment;
    pub mod division_needs;
    pub mod division_templates;
    pub mod training_queue;
}
