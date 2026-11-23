pub const FEATURES: &[&str] = if engine_allocation::labels::ENABLED {
    &[
        #[cfg(feature = "keep-journal")]
        "keep-journal",
        "memory-labels",
        #[cfg(feature = "multiplied")]
        "multiplied",
    ]
} else {
    &[
        #[cfg(feature = "keep-journal")]
        "keep-journal",
        #[cfg(feature = "multiplied")]
        "multiplied",
    ]
};

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
