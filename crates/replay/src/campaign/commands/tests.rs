use replay_capture_spool::commands::{Recording, analyze};
use super::Replay;
use replay_capture_spool::commands::test_support::{control, session};
use replay_recording::{command_packet::{Batch, Origin, Packet, Phase, Span, test_support::{actor, batch, event}}, control::ControlRecord};
use replay_data::identity::CaptureMode;

#[test]
fn columnar_capture_metadata_has_the_same_player_report_as_rows() {
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use replay_data::identity::IdentityCodec;
    use replay_capture_spool::spool::{Record, encode_record};
    let mut encoded_session = Vec::new();
    session().encode(&mut encoded_session).unwrap();
    let controls = vec![control(&batch(), 0)];
    let expected = analyze(&[session()], &controls, 4).unwrap();
    let capture = replay_recording::capture::Capture {
        campaign_id: b"synthetic".to_vec(),
        content: Vec::new(),
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![encoded_session],
        run_sessions: Vec::new(),
        stop: None,
        format: replay_data::FORMAT_VERSION,
        journaled: 0,
        controls: vec![
            encode_record(&Record::BranchStart(BranchStart {
                id: 4,
                session: 7,
                loaded_hour: Some(100),
                first_hour: 100,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }))
            .unwrap(),
            encode_record(&Record::Control(controls[0].clone())).unwrap(),
        ],
    };
    let encoded = capture.write().unwrap();
    let decoded = replay_recording::capture::Capture::read(&encoded).unwrap();
    let recording = Recording::from_columnar(decoded).unwrap();
    let actual = analyze(&recording.sessions, &recording.controls, 4).unwrap();
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    let branch = match replay_capture_spool::spool::decode_record(&capture.controls[0]).unwrap() {
        Record::BranchStart(branch) => branch,
        _ => unreachable!(),
    };
    let row = crate::campaign::fixtures::encode_records(
        recorder_derive::writers(),
        capture.campaign_id.clone(),
        &[vec![
            Record::SessionStart(session()),
            Record::BranchStart(branch),
            Record::Control(controls[0].clone()),
        ]],
    )
    .unwrap();
    let schema = replay_recording::schema::schema().unwrap();
    let identity = replay_columnar::envelope::Identity {
        schema: replay_columnar::envelope::schema_digest(&schema),
        campaign: capture.campaign_id.clone(),
        content: Vec::new(),
        capture_mode: capture.capture_mode,
    };
    let mut writer = replay_journal::columnar_file::FileWriter::create(
        std::io::Cursor::new(Vec::new()),
        identity,
        schema.len(),
        replay_settings::compression::Settings::HIGH,
        None,
    )
    .unwrap();
    let container = writer.container();
    let record = container
        .put_zstd(replay_columnar::envelope::Kind::Controls, replay_columnar::container::Tags::default(), &capture.write().unwrap(), 3)
        .unwrap();
    container.set_open(replay_columnar::container::SectionKey::RECORDS, &[record]).unwrap();
    let columnar = writer.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "hoi4-command-columnar-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    for (name, bytes) in [
        ("rows.battleplan", &row[..]),
        ("columns.h4col", &columnar[..]),
    ] {
        let path = directory.join(name);
        std::fs::write(&path, bytes).unwrap();
        let recording = crate::campaign::commands::open_path(&path).unwrap();
        let actual = analyze(&recording.sessions, &recording.controls, 4).unwrap();
        assert_eq!(actual.players.len(), 4);
        assert_eq!(actual.players[0].apm, Some(1.0));
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(directory).unwrap();
    let mut corrupt = capture;
    corrupt.controls[1].pop();
    assert!(Recording::from_columnar(corrupt).is_err());
}

#[test]
fn phase_declares_a_campaign_branch_without_recording_an_hour() {
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use replay_capture_spool::spool::Record;
    let campaign = *b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    let mut entry = control(&batch(), 0);
    let ControlRecord::PlayerCommands { packet, .. } = &mut entry else {
        unreachable!()
    };
    packet.phase = Some(Phase {
        start: BranchStart {
            id: 4,
            session: 7,
            loaded_hour: None,
            first_hour: 100,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        },
        campaign,
        qpc: 221,
        hour: 100,
        load_signal: 19,
    });
    let records = vec![
        Record::SessionStart(session()),
        Record::Control(entry.clone()),
    ];
    let replay = Replay::from_records(recorder_derive::writers(), campaign.to_vec(), &[records]).unwrap();
    assert_eq!(replay.branches.len(), 1);
    assert!(replay.index.is_empty());
    let history = replay.into_history().unwrap();
    let tail = history.session_tail(7).unwrap();
    assert_eq!(tail.hour, 100);
    assert!(!tail.recorded);
    for hour in [100, 101] {
        let mut tracker = history.tracker_for(&session()).unwrap();
        assert!(tracker.record_hour(4, 100).is_err());
        assert!(
            tracker
                .observe(Some(hour), hour, Some(19), &[])
                .unwrap()
                .start
                .is_none()
        );
    }
    let ControlRecord::PlayerCommands { packet, .. } = &mut entry else {
        unreachable!()
    };
    packet.phase.as_mut().unwrap().campaign[0] = b'f';
    let conflicting = Replay::from_records(recorder_derive::writers(), campaign.to_vec(), &[vec![Record::SessionStart(session()), Record::Control(entry)]]).unwrap();
    assert!(conflicting.controls.is_empty());
    assert_eq!(conflicting.damaged_parts, 1);
    assert!(conflicting.warnings.iter().any(|warning| warning.contains("player command phase conflicts")), "{:?}", conflicting.warnings);
}

#[test]
fn branches_share_capture_metadata_without_importing_future_updates_or_other_lifetimes() {
    use replay_frame::branches::{BranchReason, BranchStart, Gap, Parentage};
    use replay_capture_spool::spool::{Record, encode_record};
    use replay_data::identity::IdentityCodec;

    let first = control(&batch(), 0);
    let mut action = event(2, 20, 230);
    action.hour = 100;
    action.origin = Origin::Unresolved;
    let second = ControlRecord::PlayerCommands {
        session: 7,
        packet: Box::new(Packet::pack(
            5,
            1,
            2,
            1,
            220,
            260,
            0,
            &Batch {
                events: vec![action],
                spans: vec![Span {
                    paused: false,
                    hour: 100,
                    from_qpc: 220,
                    to_qpc: 260,
                    complete: true,
                }],
                ..Default::default()
            },
        )
        .unwrap()),
    };
    let mut renamed = actor(2, 20, 130);
    renamed.name = b"later name".to_vec();
    renamed.at_qpc = 260;
    let third = ControlRecord::PlayerCommands {
        session: 7,
        packet: Box::new(Packet::pack(
            6,
            1,
            2,
            2,
            260,
            280,
            0,
            &Batch {
                actors: vec![renamed],
                spans: vec![Span {
                    paused: false,
                    hour: 100,
                    from_qpc: 260,
                    to_qpc: 280,
                    complete: true,
                }],
                ..Default::default()
            },
        )
        .unwrap()),
    };
    let controls = vec![first.clone(), second.clone(), third.clone()];
    let expected = analyze(&[session()], &controls, 5).unwrap();
    assert_eq!(expected.unattributed_commands, 0);
    assert_eq!(expected.players.len(), 3);
    assert_eq!(expected.players[0].actor.id, 2);
    assert_eq!(expected.players[0].actor.name, b"same");
    assert_eq!(expected.players[0].commands, 1);
    assert_eq!(expected.players[0].unresolved_commands, 1);
    assert_eq!(expected.players[0].observed_minutes, Some(40.0 / 60.0));
    assert!(expected.players[0].apm.is_none());
    assert!(
        expected
            .players
            .iter()
            .all(|player| player.coverage_complete)
    );
    assert_eq!(
        analyze(&[session()], &controls, 6).unwrap().players[0]
            .actor
            .name,
        b"later name"
    );

    let mut records = vec![Record::SessionStart(session())];
    for (id, hour, control) in [(4, 100, first), (5, 220, second.clone()), (6, 260, third)] {
        records.push(Record::BranchStart(BranchStart {
            id,
            session: 7,
            loaded_hour: None,
            first_hour: hour,
            parent: Parentage::Unknown,
            reason: BranchReason::HourDiscontinuity,
        }));
        records.push(Record::Gap(Gap {
            branch: id,
            first_hour: hour,
            last_hour: hour,
            reason: "frames omitted in this test".into(),
        }));
        records.push(Record::Control(control));
    }
    let rows = Replay::from_records(recorder_derive::writers(), b"synthetic".to_vec(), &[records.clone()]).unwrap();
    let mut encoded_session = Vec::new();
    session().encode(&mut encoded_session).unwrap();
    let mut metadata = Vec::new();
    for record in &records[1..] {
        metadata.push(encode_record(record).unwrap());
    }
    let capture = replay_recording::capture::Capture {
        campaign_id: b"synthetic".to_vec(),
        content: Vec::new(),
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![encoded_session],
        run_sessions: Vec::new(),
        stop: None,
        format: replay_data::FORMAT_VERSION,
        journaled: 0,
        controls: metadata,
    };
    let columns = Recording::from_columnar(
        replay_recording::capture::Capture::read(&capture.write().unwrap()).unwrap(),
    )
    .unwrap();
    let expected = serde_json::to_value(expected).unwrap();
    for actual in [
        analyze(&rows.sessions, &rows.controls, 5).unwrap(),
        analyze(&columns.sessions, &columns.controls, 5).unwrap(),
    ] {
        assert_eq!(serde_json::to_value(actual).unwrap(), expected);
    }
    let mut missing_prefix = second;
    let ControlRecord::PlayerCommands { packet, .. } = &mut missing_prefix else {
        unreachable!()
    };
    packet.capture = 9;
    let missing = analyze(&[session()], &[controls[0].clone(), missing_prefix], 5).unwrap();
    assert_eq!(missing.unattributed_commands, 1);
    assert!(missing.players.is_empty());
    let mut lost_prefix = controls;
    let ControlRecord::PlayerCommands { packet, .. } = &mut lost_prefix[0] else {
        unreachable!()
    };
    packet.lost = 1;
    let lost = analyze(&[session()], &lost_prefix, 5).unwrap();
    assert!(
        lost.players
            .iter()
            .all(|player| !player.coverage_complete && player.apm.is_none())
    );
}

#[test]
fn paused_commands_before_native_load_marker_resume_into_the_first_hour() {
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use replay_capture_spool::spool::Record;
    let campaign = *b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    let mut entry = control(&batch(), 0);
    let ControlRecord::PlayerCommands { packet, .. } = &mut entry else {
        unreachable!()
    };
    packet.phase = Some(Phase {
        start: BranchStart {
            id: 4,
            session: 7,
            loaded_hour: None,
            first_hour: 100,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        },
        campaign,
        qpc: 221,
        hour: 100,
        load_signal: 0,
    });
    let replay = Replay::from_records(
        recorder_derive::writers(),
        campaign.to_vec(),
        &[vec![
            Record::SessionStart(session()),
            Record::Control(entry),
        ]],
    )
    .unwrap();
    let history = replay.into_history().unwrap();
    assert_eq!(history.session_tail(7).unwrap().load_signal, None);
    let mut tracker = history.tracker_for(&session()).unwrap();
    let first = tracker.observe(Some(100), 100, Some(19), &[]).unwrap();
    assert_eq!(first.branch, 4);
    assert!(first.start.is_none());
    tracker.record_hour(4, 100).unwrap();
    let loaded = tracker.observe(Some(100), 101, Some(20), &[]).unwrap();
    assert_eq!(loaded.start.unwrap().reason, BranchReason::LoadSignal);
}
