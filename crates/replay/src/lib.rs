//! The shared, deliberately incompatible spool and replay format.

pub mod branches;
pub mod cadence;
pub mod codec;
pub mod container;
pub mod eta_derive;
pub mod fixed_layout;
pub mod fixed_point;
pub mod format;
pub mod frame;
pub mod identity;
pub mod rows;
pub mod sliced;
pub mod spool;
mod value;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
/// The only recording version this build reads and writes.
pub const FORMAT_VERSION: u16 = 62;
pub const KEYFRAME_INTERVAL: u32 = 720;

#[cfg(test)]
mod acceptance;
