#[doc(hidden)]
pub mod checkpoint;
pub mod codec;
pub mod display;
pub mod fixed_point;
#[doc(hidden)]
pub mod measurement;
pub mod rows;
#[doc(hidden)]
pub mod sliced {
    pub mod ranges;
}
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;
#[doc(hidden)]
pub mod value;

pub type Hour = i32;
pub type SessionId = u64;
pub type BranchId = u64;
pub const FORMAT_VERSION: u16 = rows::compatibility::EQUIPMENT_DESTRUCTION_STARTUP_FORMAT;

#[doc(hidden)]
pub use engine_allocation as __allocation;
