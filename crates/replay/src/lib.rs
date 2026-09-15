//! The shared, deliberately incompatible spool and replay format.

pub mod branches;
pub mod cadence;
pub mod codec;
pub mod container;
pub mod format;
pub mod frame;
pub mod identity;
pub mod rows;
pub mod spool;
mod value;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
/// The only version the writer emits. [`format::FormatVersion::READABLE`]
/// lists every version the reader still accepts.
pub const FORMAT_VERSION: u16 = 19;
pub const KEYFRAME_INTERVAL: u32 = 720;

#[cfg(test)]
mod acceptance;
