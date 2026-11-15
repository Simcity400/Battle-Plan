//! A campaign recording's hours read in place: [`Natives`] places its runs, [`Cursor`] reads them.

use crate::{campaign::Replay, columnar_frames::Recording, rows::TableId};
use std::io;

mod cursor;
#[cfg(test)]
mod tests;

pub use cursor::Cursor;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Where the recording holds an hour: its run, and the hour's ordinal in the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub run: u32,
    pub ordinal: u32,
}

/// A campaign's recording, each readable run placed by its branch and hours.
pub struct Natives {
    recording: Recording,
    /// Each run (branch, first hour, ordinals, run), by branch and first hour.
    runs: Vec<(u64, i32, u32, u32)>,
}

impl Natives {
    /// The recording `replay` holds.
    pub fn new(replay: &Replay) -> io::Result<Self> {
        Self::of(replay.recording().selecting(&TableId::ALL)?)
    }

    /// `recording`'s runs; one whose directory cannot be read is the recording's damage, and
    /// holds no hour.
    pub fn of(recording: Recording) -> io::Result<Self> {
        let mut runs = Vec::new();
        for run in 0..recording.run_count() {
            let Ok(held) = recording.run(run) else {
                continue;
            };
            if held.ordinals != 0 {
                engine_allocation::reserve(&mut runs, 1)?;
                runs.push((held.branch, held.first_hour, held.ordinals, run));
            }
        }
        runs.sort_unstable_by_key(|&(branch, first, ..)| (branch, first));
        Ok(Self { recording, runs })
    }

    /// Where `hour` of `branch` is held; none when no run holds it.
    pub fn find(&self, branch: u64, hour: i32) -> Option<Place> {
        let at = self
            .runs
            .partition_point(|&(held, first, ..)| (held, first) <= (branch, hour));
        let &(held, first, ordinals, run) = self.runs.get(at.checked_sub(1)?)?;
        let ordinal = u32::try_from(i64::from(hour) - i64::from(first)).ok()?;
        (held == branch && ordinal < ordinals).then_some(Place { run, ordinal })
    }

    pub fn recording(&self) -> &Recording {
        &self.recording
    }
}
