
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod blocks;
pub mod config;
pub mod dto;
pub mod findings;
pub mod map_display;
pub mod map_pairs;
pub mod map_settings;
pub mod presentation;
pub mod protocol;
mod query_error;
pub use query_error::{QueryError, duplicate, refusal, refused};
#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
pub mod transport;

pub use dto::*;

pub fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}
