#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products;
pub mod registrations;

pub fn forget(source: &dashboard_base::Source) {
    products::feed::scope::forget(source);
}
