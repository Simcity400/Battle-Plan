//! The in-process recording API the tray recorder drives. The command line
//! runs the same capture through [`crate::recording`].
use crate::{
    recording::{self, COLLECTOR_LOADED, CaptureOptions, TWO_GAMES, UNSUPPORTED_BUILD},
    windows::{self, Process, ProcessMemory},
};
use engine::{PINNED_BUILD, build::hash_executable};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

pub use crate::recording::{CaptureOutcome, Control, Omission, Refusal, Stage};

/// What one capture records and where.
pub struct Recording<'a> {
    /// Capture root: session folders and the active record.
    pub output: &'a Path,
    /// Folder whose recordings this session continues and publishes into:
    /// the capture root, or a campaign's own folder after New file.
    pub target: &'a Path,
    pub user: &'a Path,
    pub game_score: bool,
    pub operation_outcomes: bool,
    pub politics_capture: bool,
    /// Whether a load may be recorded in the session, and if not why, asked
    /// once per load: each load the probe names while recording, and at save
    /// each load the spool identifies that was never asked about. The flag
    /// says the session's spool already holds the campaign's hours from an
    /// earlier load, which a pending New file counts as an existing recording.
    /// Each decision is written to the session's admission record as it is
    /// made, and only admitted loads are ever published; others stay in the
    /// session folder and are reported in the outcome. A refusal while
    /// recording ends the session.
    pub admit: &'a dyn Fn(&str, bool) -> Result<(), Refusal>,
    /// The campaign the game has loaded right now and its load signal, read
    /// without the collector.
    pub loaded: &'a dyn Fn() -> Option<(String, i64)>,
    /// The campaign and load signal this session attaches for, already chosen:
    /// admitted before the collector starts.
    pub opening: Option<(&'a str, i64)>,
}

/// Attach to `pid`, record until `control` stops it or the game exits, then
/// finalize (or discard, when the user chose Discard). A session that reached
/// no spool ends with no recordings rather than an error.
pub fn record(
    pid: u32,
    recording: &Recording<'_>,
    control: &Control,
    stage: &dyn Fn(Stage<'_>),
) -> Result<CaptureOutcome, String> {
    let options = CaptureOptions {
        output: recording.output,
        target: recording.target,
        user: recording.user,
        game_score: recording.game_score,
        operation_outcomes: recording.operation_outcomes,
        politics_capture: recording.politics_capture,
        nothing_recorded_is_ok: true,
        live_status: true,
        admit: Some(recording.admit),
        loaded: Some(recording.loaded),
        opening: recording.opening,
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault: None,
    };
    recording::capture(pid, &options, control, stage)
}

/// The campaign a running game has loaded, its current hour and the load
/// signal that changes with each load of a save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedCampaign {
    pub campaign: String,
    pub hour: i32,
    /// The game's session start stamp, the collector's own load signal: a new
    /// load of the same campaign changes it.
    pub load: i64,
}

/// Reads which campaign a supported game has loaded, without the collector.
/// The game is opened with read-only rights; nothing is written or injected,
/// and nothing read here is recorded.
pub struct Probe {
    process: Process,
    base: usize,
}

impl Probe {
    /// Verify the game build, then find its image base.
    pub fn open(pid: u32) -> Result<Self, String> {
        let process = Process::open(pid, false)?;
        let exe = hash_executable(File::open(&process.path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if !PINNED_BUILD.matches(&exe) {
            return Err(format!("{UNSUPPORTED_BUILD}. Nothing recorded."));
        }
        let name = process
            .path
            .file_name()
            .ok_or("game executable has no name")?
            .to_owned();
        let base = windows::modules(pid)?
            .into_iter()
            .find(|module| module.path.file_name() == Some(name.as_os_str()))
            .ok_or("game module not found")?
            .base;
        Ok(Self { process, base })
    }

    pub fn start_time(&self) -> u64 {
        self.process.start_time
    }

    /// The installation the running game was started from.
    pub fn game_dir(&self) -> Option<PathBuf> {
        self.process.path.parent().map(Path::to_path_buf)
    }

    /// The loaded campaign when two reads agree on a campaign identity; none
    /// at the menu, while loading or when unreadable.
    pub fn loaded(&self) -> Option<LoadedCampaign> {
        if self.process.handle.signaled().unwrap_or(true) {
            return None;
        }
        let memory = ProcessMemory(&self.process.handle);
        let read =
            || engine::allocation::advisory(|| engine::cgame_state::read_clock(&memory, self.base));
        let first = read()?;
        let second = read()?;
        if first.play_session_started_at != second.play_session_started_at {
            return None;
        }
        agreed_campaign(first.campaign_id.as_bytes(), second.campaign_id.as_bytes()).map(
            |campaign| LoadedCampaign {
                campaign,
                hour: second.hour,
                load: second.play_session_started_at,
            },
        )
    }
}

/// The content identity a session records with (installation, DLC and mods),
/// from the collector configuration the recorder wrote for it.
pub fn session_identity(session: &Path) -> Result<replay::identity::Identity, String> {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(session.join("bin/collector.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let bytes: Vec<u8> =
        serde_json::from_value(config["session_start"].clone()).map_err(|e| e.to_string())?;
    let mut decoder = replay::codec::Decoder::new(&bytes);
    let start = replay::identity::SessionStart::decode(&mut decoder).map_err(|e| e.to_string())?;
    decoder.finish().map_err(|e| e.to_string())?;
    Ok(start.identity)
}

/// The campaign a session loaded first, as its spool shows by clock witness
/// time, so one that recorded and ended before any probe still counts. Reads
/// only the first records of each campaign's spool; safe while recording.
pub fn first_loaded_campaign(session: &Path) -> Option<String> {
    crate::replay_io::first_spooled_campaign(session)
}

/// A campaign identity read twice the same, in the recorded UUID form.
fn agreed_campaign(first: &[u8], second: &[u8]) -> Option<String> {
    (first == second && replay::identity::is_campaign_uuid(first))
        .then(|| String::from_utf8(first.to_vec()).ok())
        .flatten()
}

/// The one running game, none, or the refusal for more than one.
pub fn single_game() -> Result<Option<u32>, String> {
    recording::checked_single_game()
}

/// The game process's start time, which identifies it across PID reuse.
pub fn game_start_time(pid: u32) -> Result<u64, String> {
    Ok(Process::open(pid, false)?.start_time)
}

pub fn default_capture_directory() -> PathBuf {
    recording::default_capture_directory()
}

pub fn resolve_user_directory() -> Result<PathBuf, String> {
    recording::resolve_user_directory()
}

/// A capture error in plain words; the original text stays the detail.
/// `started` says whether the collector had begun recording.
pub fn problem_text(error: &str, started: bool) -> &'static str {
    if error.starts_with(UNSUPPORTED_BUILD) {
        "This the game version is not supported. Nothing recorded."
    } else if error == TWO_GAMES {
        "More than one game is running. Close the extra game to record."
    } else if error == COLLECTOR_LOADED {
        "Another recorder is attached to this game."
    } else if error.starts_with("recording completion is partial")
        || error.contains("spool retained")
    {
        "Recording could not be finished. The captured data was kept."
    } else if error.contains("Paradox user folder") || error.contains("Paradox user directory") {
        "The Paradox user folder was not found. Choose it in Settings."
    } else if started {
        "Recording stopped because of an error."
    } else {
        "Recording could not start."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sessions_content_identity_comes_from_its_collector_configuration() {
        use replay::identity::{Identity, SampleBoundary, SessionStart, Verification};
        let session = std::env::temp_dir().join(format!(
            "hoi4-session-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(session.join("bin")).unwrap();
        let start = SessionStart {
            id: 7,
            process_id: 1,
            process_start_time: 7,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "synthetic".into(),
                recorder_version: "synthetic".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        };
        let mut bytes = Vec::new();
        start.encode(&mut bytes).unwrap();
        std::fs::write(
            session.join("bin/collector.json"),
            serde_json::to_vec(&serde_json::json!({"session_start": bytes})).unwrap(),
        )
        .unwrap();
        assert_eq!(session_identity(&session).unwrap(), start.identity);
        assert!(session_identity(&session.join("missing")).is_err());
        std::fs::remove_dir_all(session).unwrap();
    }

    #[test]
    fn a_loaded_campaign_needs_two_agreeing_uuid_reads() {
        let id = b"00000000-0000-4000-8000-000000000000";
        assert_eq!(
            agreed_campaign(id, id).as_deref(),
            Some("00000000-0000-4000-8000-000000000000")
        );
        // A read that changed between the two copies, as during a load.
        assert_eq!(
            agreed_campaign(id, b"00000000-0000-4000-8000-000000000001"),
            None
        );
        // Not a campaign identity: an empty menu value or a save name.
        assert_eq!(agreed_campaign(b"", b""), None);
        assert_eq!(agreed_campaign(b"my save", b"my save"), None);
    }

    #[test]
    fn capture_refusals_read_as_plain_problems() {
        assert_eq!(
            problem_text(
                &format!(
                    "{UNSUPPORTED_BUILD}; wanted 1 bytes / 00; found 2 bytes / 01. Nothing recorded."
                ),
                false
            ),
            "This the game version is not supported. Nothing recorded."
        );
        assert_eq!(
            problem_text(TWO_GAMES, false),
            "More than one game is running. Close the extra game to record."
        );
        assert_eq!(
            problem_text(COLLECTOR_LOADED, false),
            "Another recorder is attached to this game."
        );
        assert_eq!(
            problem_text(
                "recording completion is partial: replay finalization failed: synthetic; recoverable spool retained at x",
                true
            ),
            "Recording could not be finished. The captured data was kept."
        );
        assert_eq!(
            problem_text(
                "detach timed out; collector may still be loaded; spool retained and not finalized",
                true
            ),
            "Recording could not be finished. The captured data was kept."
        );
        assert_eq!(
            problem_text("game exited before collector became ready", false),
            "Recording could not start."
        );
        assert_eq!(
            problem_text("synthetic failure", true),
            "Recording stopped because of an error."
        );
    }
}
