use super::*;
use replay::{
    identity::{SampleBoundary, Verification},
    spool::Record,
};

#[test]
fn compatibility_old_campaign_is_refused_before_recording_history_is_seeded() {
    let id = "01234567-89ab-cdef-0123-456789abcdef";
    let root =
        std::env::temp_dir().join(format!("hoi4_history_compatibility_{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 1,
        sample_boundary: SampleBoundary::EndOfHour,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: replay::identity::COVERAGE_POLICY_VERSION,
            language: "l_english".into(),
            recorder_version: "synthetic".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        },
    };
    let replay =
        Replay::from_records(id.into(), &[vec![Record::SessionStart(session.clone())]]).unwrap();
    // A new process session can attach without needing an old session's hour to resume.
    let session = SessionStart { id: 2, ..session };
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&94_u16.to_le_bytes());
    assert_eq!(Replay::open(bytes.clone()).unwrap().format.version(), 94);
    let path = root.join(format!("{id}.battleplan"));
    fs::write(&path, &bytes).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    let error = prepared.take(id, &session).err().unwrap().to_string();
    assert_eq!(
        error,
        format!(
            "existing campaign replay is format 94; recording appends require format {}; move it aside or use another replay directory",
            FormatVersion::CURRENT.version()
        )
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    // Moving the old file aside starts without its parentage or lease.
    drop(prepared);
    fs::remove_file(&path).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert!(prepared.take(id, &session).unwrap().1.is_none());
    fs::write(&path, replay.bytes()).unwrap();
    let mut prepared = PreparedCampaigns::load(&root, &session, |_| Ok(())).unwrap();
    assert!(prepared.take(id, &session).unwrap().1.is_some());
    drop(prepared);
    fs::remove_dir_all(&root).unwrap();
}
