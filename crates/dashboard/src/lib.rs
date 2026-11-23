
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod answer;
pub mod content_check;
pub mod products;
pub mod reply_check;
pub mod panel;
pub mod service;

pub use service::Service;

#[cfg(any(test, feature = "export-types"))]
mod contract;
#[cfg(any(test, feature = "export-types"))]
pub use contract::{request_fields, request_paths, series_walk, typescript_contract};

#[cfg(test)]
mod contract_tests;
