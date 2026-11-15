//! The product contract and the shared products other products build on. The
//! dashboard registers every product, these included.

mod product;
pub use product::*;

pub mod artwork;
pub mod battles;
pub mod built;
pub mod character_portraits;
pub mod equipment_icon;
pub mod fixed5_runs;
pub mod idea_format;
pub mod politics_content;
pub mod read_ahead;
pub mod resources;

