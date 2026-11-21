
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod map {
    pub mod intel_icons;
    pub mod raid_arrows;
    pub mod raids;
}
