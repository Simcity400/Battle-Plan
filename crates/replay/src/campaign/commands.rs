use super::Replay;
use replay_capture_spool::commands::{PlayerSummary, Recording, analyze};
use std::{io, path::Path};

pub fn open_path(path: &Path) -> io::Result<Recording> {
    let recording = replay_recording::columnar_frames::Recording::open_path(path)?;
    let capture = recording.capture()?;
    drop(recording);
    match capture {
        Some(capture) => Recording::from_columnar(capture),
        None => {
            let replay = Replay::open_path(path)?;
            Ok(Recording {
                sessions: replay.sessions,
                branches: replay.branches,
                controls: replay.controls,
            })
        }
    }
}


pub fn summarize(replay: &Replay, branch: u64) -> io::Result<Vec<PlayerSummary>> {
    Ok(analyze(&replay.sessions, &replay.controls, branch)?.players)
}

#[cfg(test)]
mod tests;
