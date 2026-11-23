#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod flags;
pub mod gui;
pub mod pdxshader;
pub mod train_gfx;

#[cfg(test)]
mod layer_tests;
