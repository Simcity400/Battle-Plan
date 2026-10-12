//! Appendix C columnar replies, immutable dictionary shards and registered shapes.
pub mod dictionary;
pub mod errors;
pub mod fixtures;
pub mod model;
pub mod reader;
pub mod shapes;
pub mod writer;
pub use errors::{Result, WireError};
pub use model::*;
pub use reader::read;
pub use writer::write;
