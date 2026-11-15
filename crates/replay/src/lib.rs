//! Campaign recordings, spool I/O and durable publication.

/// The features this build of the crate has, for a binary to state.
pub const FEATURES: &[&str] = &[
    #[cfg(feature = "keep-journal")]
    "keep-journal",
    #[cfg(feature = "memory-labels")]
    "memory-labels",
    #[cfg(feature = "multiplied")]
    "multiplied",
];

pub mod at;
pub mod campaign;
pub mod cells;
pub mod columnar_file;
pub mod columnar_frames;
pub mod columnar_journal;
pub mod columnar_natives;
pub mod commands;
pub mod compression;
pub mod encode_report;
pub mod lanes;
pub mod native_columns;
pub mod native_frame;
pub mod packing;
pub mod points;
pub mod progress;
#[cfg(windows)]
pub mod publication;
#[cfg(windows)]
pub mod recompress;
pub mod recorder_settings;
pub mod multiplied;
#[cfg(windows)]
pub mod recording_files;
pub mod spool;
pub mod summaries;

use replay_data::{BranchId, Hour, SessionId, codec, format, identity, scope};
use replay_frame::{branches, frame, rows};

