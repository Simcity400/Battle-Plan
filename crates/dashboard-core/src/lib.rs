//! The dashboard's shared contract: host/viewer protocol, data transport,
//! reply DTOs, configuration, map settings, content findings and the block
//! scanner every product reads recordings through. Nothing here names a
//! product.

pub mod blocks;
pub mod config;
pub mod dto;
pub mod findings;
pub mod map_display;
pub mod map_pairs;
pub mod map_settings;
pub mod protocol;
#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
pub mod transport;

pub use dto::*;

pub fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}
