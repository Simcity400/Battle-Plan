//! Compressed recording containers, spool I/O and durable publication.

pub mod container;
#[cfg(windows)]
pub mod publication;
pub mod recorder_settings;
#[cfg(windows)]
pub mod recording_files;
pub mod spool;

use replay_data::{BranchId, FORMAT_VERSION, Hour, SessionId, codec, format, identity};
#[cfg(test)]
use replay_data::{cadence, event_feed_rows, value};
use replay_frame::{KEYFRAME_INTERVAL, branches, cpu, frame, rows};

#[cfg(test)]
mod acceptance;
