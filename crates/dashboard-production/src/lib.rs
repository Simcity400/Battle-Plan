#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products;
pub mod registrations;

pub fn forget(source: &dashboard_base::Source) {
    products::production::forget(source);
    products::construction::forget(source);
    products::equipment_produced::forget(source);
}
