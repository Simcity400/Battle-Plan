//! Pinned game build and guarded memory reads. Live sampling proof is pending.

pub mod build;
pub mod ccountry;
pub mod cdlc_manager;
pub mod cgame_state;
pub mod cpolitical_status;
pub mod cprovince;
pub mod cstate;
pub mod memory;
pub mod pdx;
pub mod walk;

pub use build::{Build, PINNED_BUILD};
pub use memory::{Memory, read, read_slice};
