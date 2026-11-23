use super::*;
use replay_data::identity::{
    CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification,
};

fn session() -> SessionStart {
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

fn actor(id: u64, key: i32, since: u64) -> Actor {
    Actor {
        at_qpc: since,
        tag: b"GER".to_vec(),
        id,
        player: key,
        country: 3,
        since_qpc: since,
        user: b"same".to_vec(),
        name: b"same".to_vec(),
    }
}

fn event(actor: u64, player: i32, qpc: u64) -> Event {
    Event {
        paused: false,
        actor,
        player,
        qpc,
        hour: 100,
        kind: 1,
        command_country: Some(3),
        origin: Origin::PlayerOrder,
        sequence: qpc as u32,
    }
}

fn batch() -> Batch {
    Batch {
        kinds: vec![Kind {
            id: 1,
            token: 9,
            name: "synthetic movement".into(),
            category: Category::Army,
        }],
        actors: vec![
            actor(1, 10, 100),
            actor(2, 20, 130),
            actor(3, 10, 160),
            actor(4, 30, 100),
        ],
        departures: vec![(1, 160)],
        events: vec![event(1, 10, 110), event(2, 20, 140), event(3, 10, 170)],
        spans: vec![Span {
            paused: false,
            hour: 100,
            from_qpc: 100,
            to_qpc: 220,
            complete: true,
        }],
    }
}

fn control(batch: &Batch, lost: u64) -> ControlRecord {
    ControlRecord::PlayerCommands {
        session: 7,
        packet: Packet::pack(4, 1, 2, 0, 100, 220, lost, batch).unwrap(),
    }
}

#[test]
fn codec_retains_negative_deltas_unknowns_and_boundary_integers() {
    let mut batch = batch();
    batch.events[0].hour = i32::MAX;
    batch.events[0].sequence = u32::MAX;
    batch.events[1].hour = i32::MIN;
    batch.events[1].sequence = 0;
    batch.events[1].command_country = Some(u32::MAX);
    batch.events[2].actor = 0;
    batch.events[2].player = -2;
    batch.events[2].origin = Origin::Unresolved;
    batch.events[2].command_country = None;
    let packet = Packet::pack(4, 1, 2, 0, 100, 220, 2, &batch).unwrap();
    assert_eq!(packet.unpack().unwrap(), batch);
    let mut bytes = Vec::new();
    packet.encode(&mut bytes).unwrap();
    assert_eq!(bytes.len(), packet.encoded_bytes().unwrap());
    let decoded = Packet::decode(&mut Decoder::new(&bytes)).unwrap();
    assert_eq!(decoded, packet);
    for end in 0..bytes.len() {
        assert!(Packet::decode(&mut Decoder::new(&bytes[..end])).is_err());
    }
    let mut corrupt = packet;
    corrupt.raw_length += 1;
    assert!(corrupt.unpack().is_err());
}

#[test]
fn compact_columns_keep_full_width_actor_residuals_and_backward_span_gaps() {
    let mut batch = batch();
    batch.events = (0..600)
        .map(|i| Event {
            actor: if i % 2 == 0 { u64::MAX } else { 0 },
            player: if i % 2 == 0 { i32::MIN } else { i32::MAX },
            command_country: if i % 2 == 0 { None } else { Some(u32::MAX) },
            qpc: i,
            hour: if i % 2 == 0 { i32::MAX } else { i32::MIN },
            sequence: if i % 2 == 0 { u32::MAX } else { 0 },
            kind: u32::MAX,
            origin: Origin::Unresolved,
            paused: i % 3 == 0,
        })
        .collect();
    batch.spans = vec![
        Span {
            from_qpc: 0,
            to_qpc: u64::MAX,
            hour: i32::MIN,
            complete: true,
            paused: false,
        },
        Span {
            from_qpc: 0,
            to_qpc: 0,
            hour: i32::MAX,
            complete: false,
            paused: true,
        },
    ];
    let packet = Packet::pack(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        0,
        u64::MAX,
        u64::MAX,
        &batch,
    )
    .unwrap();
    assert!(packet.compressed);
    assert_eq!(packet.unpack().unwrap(), batch);
    let mut bytes = Vec::new();
    packet.encode(&mut bytes).unwrap();
    assert_eq!(packet.encoded_bytes().unwrap(), bytes.len());
    assert_eq!(Packet::decode(&mut Decoder::new(&bytes)).unwrap(), packet);
    for events in [Vec::new(), vec![event(1, 1, u64::MAX)]] {
        let batch = Batch {
            events,
            ..Default::default()
        };
        let packet = Packet::pack(1, 1, 1, 0, u64::MAX, u64::MAX, 0, &batch).unwrap();
        assert!(!packet.compressed);
        assert_eq!(packet.unpack().unwrap(), batch);
    }
}

#[test]
fn coop_duplicate_names_late_join_reused_key_and_idle_player_stay_separate() {
    let report = analyze(&[session()], &[control(&batch(), 0)], 4).unwrap();
    assert!(report.recorded);
    assert_eq!(report.players.len(), 4);
    let players = report.players;
    assert_eq!(players[0].apm, Some(1.0));
    assert_eq!(players[1].observed_minutes, Some(1.5));
    assert_eq!(players[2].actor.player, players[0].actor.player);
    assert_ne!(players[2].actor.id, players[0].actor.id);
    assert_eq!(players[3].apm, Some(0.0));
    assert!(
        players
            .iter()
            .all(|player| player.confirmed_orders == player.commands)
    );
}

#[test]
fn loss_unresolved_and_unclassified_never_become_valid_zero_rates() {
    let mut source = batch();
    source.events[0].origin = Origin::Unresolved;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert_eq!(report.players[0].unresolved_commands, 1);
    assert_eq!(report.players[0].apm, None);
    source.kinds[0].category = Category::Other;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert_eq!(report.players[0].unclassified_commands, 1);
    assert_eq!(report.players[0].apm, None);
    let report = analyze(&[session()], &[control(&batch(), 1)], 4).unwrap();
    assert!(
        report
            .players
            .iter()
            .all(|player| !player.coverage_complete && player.apm.is_none())
    );
    assert!(!analyze(&[session()], &[], 4).unwrap().recorded);
}

#[test]
fn retries_are_idempotent_and_conflicting_packet_identity_is_rejected() {
    let first = control(&batch(), 0);
    let repeated = analyze(&[session()], &[first.clone(), first.clone()], 4).unwrap();
    assert_eq!(repeated.players[0].commands, 1);
    assert!(analyze(&[session()], &[first, control(&batch(), 1)], 4).is_err());
}

#[test]
fn reattachment_to_one_process_world_and_branch_has_independent_identities() {
    let first = control(&batch(), 0);
    let mut second = first.clone();
    let ControlRecord::PlayerCommands { packet, .. } = &mut second else {
        unreachable!()
    };
    packet.capture += 1;
    let report = analyze(&[session()], &[first, second], 4).unwrap();
    assert_eq!(report.players.len(), 8);
    assert_ne!(report.players[0].capture, report.players[4].capture);
    assert!(report.players.iter().all(|player| player.coverage_complete));
}

#[test]
fn metadata_after_departure_cannot_extend_or_reverse_observed_time() {
    let mut batch = batch();
    batch.actors.truncate(1);
    let mut after = batch.actors[0].clone();
    after.at_qpc = 180;
    batch.actors.push(after);
    batch.events.truncate(1);
    let index = index(&[session()], &[control(&batch, 0)], 4).unwrap();
    assert_eq!(index.analysis.players[0].observed_minutes, Some(1.0));
    assert!(index.timelines[0].measure(100, 100, 180, 160).is_none());
}

#[test]
fn columnar_capture_metadata_has_the_same_player_report_as_rows() {
    use crate::{
        branches::{BranchReason, BranchStart, Parentage},
        identity::IdentityCodec,
        spool::{Record, encode_record},
    };
    let mut encoded_session = Vec::new();
    session().encode(&mut encoded_session).unwrap();
    let controls = vec![control(&batch(), 0)];
    let expected = analyze(&[session()], &controls, 4).unwrap();
    let capture = crate::columnar_file::Capture {
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
    let decoded = crate::columnar_file::Capture::read(&encoded).unwrap();
    let recording = Recording::from_columnar(decoded).unwrap();
    let actual = analyze(&recording.sessions, &recording.controls, 4).unwrap();
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    let branch = match crate::spool::decode_record(&capture.controls[0]).unwrap() {
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
    let schema = crate::columnar_frames::schema().unwrap();
    let identity = replay_columnar::envelope::Identity {
        schema: replay_columnar::envelope::schema_digest(&schema),
        campaign: capture.campaign_id.clone(),
        content: Vec::new(),
        capture_mode: capture.capture_mode,
    };
    let mut writer = crate::columnar_file::FileWriter::create(
        std::io::Cursor::new(Vec::new()),
        identity,
        schema.len(),
        crate::columnar_file::Settings::HIGH,
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
        let recording = Recording::open_path(&path).unwrap();
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
fn event_outside_coverage_and_unknown_issuer_withhold_rates() {
    let mut source = batch();
    source.spans[0].from_qpc = 120;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert!(
        report
            .players
            .iter()
            .all(|player| !player.coverage_complete)
    );
    source.events[0].actor = 0;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert_eq!(report.unattributed_commands, 1);
    assert!(report.players.iter().all(|player| player.apm.is_none()));
}

#[test]
fn phase_declares_a_campaign_branch_without_recording_an_hour() {
    use crate::{
        branches::{BranchReason, BranchStart, Parentage},
        spool::Record,
    };
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
    use crate::{
        branches::{BranchReason, BranchStart, Gap, Parentage},
        spool::{Record, encode_record},
    };
    use replay_data::identity::IdentityCodec;

    let first = control(&batch(), 0);
    let mut action = event(2, 20, 230);
    action.hour = 100;
    action.origin = Origin::Unresolved;
    let second = ControlRecord::PlayerCommands {
        session: 7,
        packet: Packet::pack(
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
        .unwrap(),
    };
    let mut renamed = actor(2, 20, 130);
    renamed.name = b"later name".to_vec();
    renamed.at_qpc = 260;
    let third = ControlRecord::PlayerCommands {
        session: 7,
        packet: Packet::pack(
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
        .unwrap(),
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
    let capture = crate::columnar_file::Capture {
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
        crate::columnar_file::Capture::read(&capture.write().unwrap()).unwrap(),
    )
    .unwrap();
    let expected = serde_json::to_value(expected).unwrap();
    for actual in [
        analyze(&rows.sessions, &rows.controls, 5).unwrap(),
        analyze(&columns.sessions, &columns.controls, 5).unwrap(),
    ] {
        assert_eq!(serde_json::to_value(actual).unwrap(), expected);
    }
    let mut missing_prefix = second.clone();
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
fn automated_execution_with_a_human_sender_does_not_inflate_player_apm() {
    let mut source = batch();
    source.actors = vec![actor(1, 1, 100)];
    source.departures.clear();
    source.events = vec![event(1, 1, 110)];
    source.kinds.push(Kind {
        id: 2,
        token: 10,
        name: "synthetic automatic plan maintenance".into(),
        category: Category::Automatic,
    });
    for time in 111..=210 {
        let mut automatic = event(1, 1, time);
        automatic.origin = Origin::AutonomousCountry;
        automatic.command_country = Some(9);
        source.events.push(automatic);
        let mut maintenance = event(1, 1, time);
        maintenance.kind = 2;
        source.events.push(maintenance);
    }
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    let player = &report.players[0];
    assert_eq!(player.commands, 1);
    assert_eq!(player.confirmed_orders, 1);
    assert_eq!(player.apm, Some(0.5));
    assert_eq!(player.confirmed_orders_per_minute, Some(0.5));
    assert_eq!(player.categories, BTreeMap::from([(Category::Army, 1)]));
    assert!(player.unresolved_kinds.is_empty());

    let mut ambiguous = event(1, 1, 215);
    ambiguous.origin = Origin::Unresolved;
    source.events.push(ambiguous);
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    let player = &report.players[0];
    assert_eq!(player.confirmed_orders, 1);
    assert_eq!(player.unresolved_commands, 1);
    assert_eq!(player.apm, None);
    assert_eq!(player.confirmed_orders_per_minute, Some(0.5));
    assert_eq!(player.categories, BTreeMap::from([(Category::Army, 1)]));
    assert_eq!(player.unresolved_kinds["synthetic movement"], 1);
}

#[test]
fn paused_commands_before_native_load_marker_resume_into_the_first_hour() {
    use crate::{
        branches::{BranchReason, BranchStart, Parentage},
        spool::Record,
    };
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
