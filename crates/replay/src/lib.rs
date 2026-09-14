//! The shared, deliberately incompatible spool and replay format.

pub mod branches;
pub mod cadence;
pub mod codec;
pub mod container;
pub mod frame;
pub mod identity;
pub mod rows;
pub mod spool;
mod value;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
pub const FORMAT_VERSION: u16 = 17;
pub const KEYFRAME_INTERVAL: u32 = 720;

#[cfg(test)]
mod acceptance;
