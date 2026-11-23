pub mod division_equipment;
pub mod equipment_losses;
pub mod stock;

#[cfg(all(test, feature = "memory-labels"))]
#[global_allocator]
static ALLOCATOR: engine_allocation::labels::Counting<std::alloc::System> =
    engine_allocation::labels::Counting(std::alloc::System);
