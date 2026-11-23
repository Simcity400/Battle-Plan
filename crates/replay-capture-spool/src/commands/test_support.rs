use replay_recording::{
    command_packet::{Batch, Packet},
    control::ControlRecord,
};
use replay_data::identity::{
    CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification,
};

pub fn session() -> SessionStart {
    SessionStart {
        id: 7,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 1,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "synthetic".into(),
            recorder_version: "synthetic".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

pub fn control(batch: &Batch, lost: u64) -> ControlRecord {
    ControlRecord::PlayerCommands {
        session: 7,
        packet: Packet::pack(4, 1, 2, 0, 100, 220, lost, batch).unwrap(),
    }
}
