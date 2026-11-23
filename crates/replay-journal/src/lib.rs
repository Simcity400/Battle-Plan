pub const FEATURES: &[&str] = &[
    #[cfg(feature = "keep-journal")]
    "keep-journal",
    #[cfg(feature = "memory-labels")]
    "memory-labels",
    #[cfg(feature = "multiplied")]
    "multiplied",
];

pub mod columnar_file;
#[cfg(any(test, feature = "journal"))]
pub mod columnar_journal;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
#[cfg(any(test, feature = "recording"))]
pub mod multiplied;
#[cfg(any(test, feature = "journal"))]
pub mod native_columns;
#[cfg(any(test, feature = "journal"))]
pub mod native_frame;
#[cfg(any(test, feature = "recording"))]
pub mod summaries;
#[cfg(any(test, feature = "test-support"))]
pub mod synthetic;
