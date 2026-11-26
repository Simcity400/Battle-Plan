use crate::{
    recording::{self, COLLECTOR_LOADED, CaptureOptions, GAME_CLAIMED},
    windows::{Process, ProcessMemory},
};
use replay_data::{identity::IdentityCodec, scope::CaptureMode};
use std::path::{Path, PathBuf};

pub use recorder_platform::games::{
    Discovery, GameProcess, ProcessState, SeveralGames, choose_game,
};

pub use crate::recording::{
    CaptureOutcome, Control, Omission, Refusal, SavedWaiting, SetAside, Stage, Stop, Undiscarded,
    UnloadObservation, Waiting, WaitingCampaign, WaitingLock, WaitingName, WaitingNames,
    end_abandoned_sessions, lock_waiting, session_folders, waiting_campaigns, waiting_folder,
};

pub struct Recording<'a> {
    pub output: &'a Path,
    pub target: &'a Path,
    pub user: &'a Path,
    pub capture_mode: CaptureMode,
    pub admit: &'a dyn Fn(&str, bool) -> Result<(), Refusal>,
    pub loaded: &'a dyn Fn() -> Option<(String, i64)>,
    pub opening: Option<(&'a str, i64)>,
    pub auto_save: &'a dyn Fn() -> bool,
    pub names_wanted: &'a dyn Fn() -> bool,
}

pub struct OpenedGame(Process);

pub fn open_game(game: &GameProcess) -> Result<Option<OpenedGame>, String> {
    Ok(Process::open_selected(game, true)?.map(OpenedGame))
}

pub fn record(
    game: OpenedGame,
    recording: &Recording<'_>,
    control: &Control,
    stage: &dyn Fn(Stage<'_>),
) -> Result<CaptureOutcome, String> {
    let options = CaptureOptions {
        output: recording.output,
        cache: &crate::directories::cache_directory(),
        target: recording.target,
        user: recording.user,
        game_score: true,
        operation_outcomes: true,
        politics_capture: true,
        capture_mode: recording.capture_mode,
        nothing_recorded_is_ok: true,
        live_status: true,
        admit: Some(recording.admit),
        loaded: Some(recording.loaded),
        opening: recording.opening,
        auto_save: Some(recording.auto_save),
        names_wanted: Some(recording.names_wanted),
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault: None,
    };
    recording::capture(game.0, &options, control, stage)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedCampaign {
    pub campaign: String,
    pub hour: i32,
    pub load: i64,
}

pub struct Probe {
    process: Process,
    base: usize,
}

impl Probe {
    pub fn open(game: &GameProcess) -> Result<Self, String> {
        Self::read(
            Process::open_selected(game, false)?
                .ok_or("the selected game exited; its PID names another process")?,
        )
    }

    pub fn open_pid(pid: u32) -> Result<Self, String> {
        Self::read(Process::open(pid, false)?)
    }

    fn read(process: Process) -> Result<Self, String> {
        let base = crate::game_sites::module_base(&process, process.pid)?;
        crate::game_sites::locate(&process.path, base)?
            .layouts()
            .map_err(|withheld| format!("{PROBE_WITHHELD}: {withheld}"))?;
        engine_native::cgame_state::clock_sites(base)
            .map_err(|withheld| format!("{PROBE_WITHHELD}: {withheld}"))?;
        Ok(Self { process, base })
    }

    pub fn start_time(&self) -> u64 {
        self.process.start_time
    }

    pub fn game_dir(&self) -> Option<PathBuf> {
        self.process.path.parent().map(Path::to_path_buf)
    }

    pub fn presentation_sites(&self) -> Result<(), &'static str> {
        engine_native_air::command_inputs::idler_sites(self.base)
    }

    pub(crate) fn clock(&self, presentation: bool) -> Option<crate::clock::Sample> {
        if self.process.handle.signaled().unwrap_or(true) {
            return None;
        }
        crate::clock::sample(
            &ProcessMemory(&self.process.handle),
            self.base,
            presentation,
        )
    }

    pub fn loaded(&self) -> Option<LoadedCampaign> {
        if self.process.handle.signaled().unwrap_or(true) {
            return None;
        }
        let memory = ProcessMemory(&self.process.handle);
        let read = || {
            engine_allocation::advisory(|| {
                engine_native::cgame_state::read_clock(&memory, self.base)
            })
        };
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

pub fn session_identity(session: &Path) -> Result<replay_data::identity::Identity, String> {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(session.join("bin/collector.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let bytes: Vec<u8> =
        serde_json::from_value(config["session_start"].clone()).map_err(|e| e.to_string())?;
    let mut decoder = replay_data::codec::Decoder::new(&bytes);
    let start =
        replay_data::identity::SessionStart::decode(&mut decoder).map_err(|e| e.to_string())?;
    decoder.finish().map_err(|e| e.to_string())?;
    Ok(start.identity)
}

pub fn first_loaded_campaign(session: &Path) -> Option<String> {
    crate::replay_io::first_loaded_campaign(session)
}

pub fn unsaved_bytes(session: &Path, target: &Path, campaign: &str) -> Option<u64> {
    crate::replay_io::unsaved_bytes(session, target, campaign)
}

fn agreed_campaign(first: &[u8], second: &[u8]) -> Option<String> {
    (first == second && replay_data::identity::is_campaign_uuid(first))
        .then(|| String::from_utf8(first.to_vec()).ok())
        .flatten()
}

pub fn game_processes() -> Result<Vec<GameProcess>, String> {
    crate::windows::game_processes()
}

pub fn default_capture_directory() -> PathBuf {
    crate::directories::default_capture_directory()
}

pub fn resolve_user_directory() -> Result<PathBuf, String> {
    crate::directories::resolve_user_directory()
}

const PROBE_WITHHELD: &str = "the loaded campaign is withheld on this unrecognised game build";

pub fn probe_withheld(error: &str) -> bool {
    error.starts_with(PROBE_WITHHELD)
}

pub fn problem_text(error: &str, started: bool) -> &'static str {
    if error.contains("withheld: the game build is unrecognised") {
        "This game build isn’t recognised. Nothing it holds could be checked, so nothing was recorded."
    } else if error == COLLECTOR_LOADED || error == GAME_CLAIMED {
        "Another recorder is connected to this game. Close the other recorder to record here."
    } else if error.starts_with("recording completion is partial")
        || error.contains("spool retained")
    {
        "Recording couldn’t be finished. The captured data was kept."
    } else if error.contains("Paradox user folder") || error.contains("Paradox user directory") {
        "The Paradox user folder wasn’t found. Choose it in Settings."
    } else if started {
        "Recording stopped because of an error."
    } else {
        "Recording couldn’t start."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sessions_content_identity_comes_from_its_collector_configuration() {
        use replay_data::{
            identity::CaptureMode, identity::Identity, identity::SampleBoundary,
            identity::SessionStart, identity::Verification,
        };
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
            capture_mode: CaptureMode::Full,
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
        assert_eq!(
            agreed_campaign(id, b"00000000-0000-4000-8000-000000000001"),
            None
        );
        assert_eq!(agreed_campaign(b"", b""), None);
        assert_eq!(agreed_campaign(b"my save", b"my save"), None);
    }

}
