use super::*;
use crate::test_support::{
    ALLOCATION_CAMPAIGN, allocation_frame, allocation_range, allocation_records,
    allocation_test_directory, allocation_witness, recorded_frames, two_load_records,
};
use replay_frame::branches::BranchReason;

fn encode(records: &[Record]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| replay::spool::encode_record(record).unwrap())
        .collect()
}

fn write_spool(session: &Path, campaign: &[u8; 36], records: &[Record]) -> PathBuf {
    let directory = session.join(std::str::from_utf8(campaign).unwrap());
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("spool.0000.h4s");
    fs::write(&path, encode(records)).unwrap();
    path
}

fn allocation_session(name: &str) -> (PathBuf, PathBuf, TerminalWitness) {
    let root = allocation_test_directory(name);
    let session = root.join("session");
    let path = write_spool(&session, ALLOCATION_CAMPAIGN, &allocation_records());
    let witness = allocation_witness(fs::metadata(path).unwrap().len());
    (session, root.join("replays"), witness)
}

fn extension(
    primary: TerminalWitness,
    range: UncommittedRange,
    prefix: Option<(u32, u64)>,
) -> TerminalWitness {
    TerminalWitness {
        campaign_id: Some(range.campaign_id),
        branch: None,
        load_signal: None,
        failed_hour: None,
        committed_campaign_id: prefix.map(|_| range.campaign_id),
        committed_segment: prefix.map(|(segment, _)| segment),
        committed_bytes: prefix.map_or(0, |(_, bytes)| bytes),
        pending_campaign_id: None,
        pending_segment: None,
        pending_bytes: 0,
        uncommitted: Some(range),
        ..primary
    }
}

fn write_bundle(session: &Path, records: &[TerminalWitness]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let mut encoded = [0; WITNESS_BYTES];
        record.encode_bundle_record(&mut encoded, index as u32, records.len() as u32);
        bytes.extend_from_slice(&encoded);
    }
    fs::write(session.join(WITNESS_FILE_NAME), &bytes).unwrap();
    assert_eq!(read_failure_witnesses(session).unwrap(), records);
    bytes
}

fn finalized(session: &Path, output: &Path, count: usize) -> Vec<PathBuf> {
    let outcome = finalize_session_outcome(session, output).unwrap();
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert!(outcome.unsaved.is_empty());
    assert!(outcome.omitted.is_empty());
    assert!(outcome.unrenamed.is_empty());
    assert_eq!(outcome.replays.len(), count);
    outcome.replays
}

fn assert_repeated(session: &Path, output: &Path, paths: &[PathBuf]) {
    let bytes: Vec<_> = paths.iter().map(|path| fs::read(path).unwrap()).collect();
    assert_eq!(finalized(session, output, paths.len()), paths);
    for (path, before) in paths.iter().zip(bytes) {
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

fn assert_contents(
    path: &Path,
    campaign: &[u8; 36],
    frames: &[(BranchId, Hour)],
    gaps: &[(BranchId, Hour, Hour)],
    failed_hour: Option<Hour>,
) {
    let replay = open(path).unwrap();
    assert_eq!(replay.campaign_id.as_slice(), campaign);
    assert_eq!(recorded_frames(path), frames);
    let expected: Vec<_> = gaps
        .iter()
        .map(|&(branch, first_hour, last_hour)| Gap {
            branch,
            first_hour,
            last_hour,
            reason: ALLOCATION_GAP_REASON.into(),
        })
        .collect();
    assert_eq!(replay.gaps, expected);
    let partials: Vec<_> = replay
        .controls
        .iter()
        .filter(|control| matches!(control, ControlRecord::Partial { .. }))
        .cloned()
        .collect();
    assert_eq!(
        partials,
        [ControlRecord::Partial {
            session: 7,
            reason: ALLOCATION_GAP_REASON.into(),
            failed_hour,
        }]
    );
}

#[test]
fn bundled_ranges_keep_each_branch_and_derive_an_unwritten_branch_start() {
    let (session, output, mut primary) = allocation_session("bundle-branches");
    let mut records = two_load_records();
    records.pop();
    primary.committed_bytes = encode(&records).len() as u64;
    primary.branch = Some(11);
    primary.load_signal = Some(29);
    primary.failed_hour = Some(23);
    primary.uncommitted = Some(allocation_range(11, Some(29), 21..=22));
    let mut late = records;
    late.push(Record::BranchStart(BranchStart {
        id: 11,
        session: 7,
        loaded_hour: Some(20),
        first_hour: 21,
        parent: Parentage::Unknown,
        reason: BranchReason::LoadSignal,
    }));
    late.push(Record::Frame(allocation_frame(11, 21, 29)));
    let spool = write_spool(&session, ALLOCATION_CAMPAIGN, &late);
    let source = fs::read(&spool).unwrap();
    assert_eq!(
        replay::spool::recover_prefix(&source).valid_bytes,
        source.len()
    );
    let prefix = Some((0, primary.committed_bytes));
    let bundle = write_bundle(
        &session,
        &[
            primary,
            extension(primary, allocation_range(10, Some(23), 5..=7), prefix),
            extension(primary, allocation_range(9, Some(19), 5..=6), prefix),
        ],
    );

    let paths = finalized(&session, &output, 1);
    assert_contents(
        &paths[0],
        ALLOCATION_CAMPAIGN,
        &[(9, 2), (9, 3), (9, 4), (10, 3), (10, 4)],
        &[(9, 5, 6), (10, 5, 7), (11, 21, 23)],
        Some(23),
    );
    let replay = open(&paths[0]).unwrap();
    assert_eq!(replay.branches.len(), 3);
    assert_eq!(
        replay.branches[2],
        BranchStart {
            id: 11,
            session: 7,
            loaded_hour: None,
            first_hour: 21,
            parent: Parentage::Unknown,
            reason: BranchReason::LoadSignal,
        }
    );
    assert_repeated(&session, &output, &paths);
    assert_eq!(fs::read(spool).unwrap(), source);
    assert_eq!(fs::read(session.join(WITNESS_FILE_NAME)).unwrap(), bundle);
}

#[test]
fn bundled_campaigns_use_independent_caps_and_recover_a_never_started_campaign() {
    let (session, output, mut primary) = allocation_session("bundle-campaigns");
    let second = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    let never_started = *b"23b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    let first_spool = session
        .join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap())
        .join("spool.0000.h4s");
    let mut first = allocation_records();
    first.pop();
    primary.committed_bytes = encode(&first).len() as u64;
    first.push(Record::Frame(allocation_frame(9, 5, 19)));
    fs::write(&first_spool, encode(&first)).unwrap();
    let mut records = allocation_records();
    for record in &mut records {
        if let Record::Frame(frame) = record {
            frame.tables[TableId::Campaign.index()] =
                TableUpdate::Snapshot(TableRows::Campaign(vec![replay_data::rows::CampaignRow {
                    id: second.to_vec(),
                }]));
        }
    }
    let second_cap = encode(&records[..4]).len() as u64;
    assert_ne!(second_cap, primary.committed_bytes);
    let second_spool = write_spool(&session, &second, &records);
    let sources: Vec<_> = [first_spool, second_spool]
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            assert_eq!(
                replay::spool::recover_prefix(&bytes).valid_bytes,
                bytes.len()
            );
            (path, bytes)
        })
        .collect();
    primary.failed_hour = Some(7);
    primary.uncommitted = Some(allocation_range(9, Some(19), 5..=6));
    let second_range = UncommittedRange {
        campaign_id: second,
        ..allocation_range(9, Some(19), 3..=4)
    };
    let new_range = UncommittedRange {
        campaign_id: never_started,
        ..allocation_range(1, Some(31), 51..=52)
    };
    let bundle = write_bundle(
        &session,
        &[
            primary,
            extension(primary, second_range, Some((0, second_cap))),
            extension(primary, new_range, None),
            extension(
                primary,
                UncommittedRange {
                    first_hour: 53,
                    last_hour: 54,
                    branch: 2,
                    ..new_range
                },
                None,
            ),
        ],
    );
    let new_campaign = std::str::from_utf8(&never_started).unwrap();
    let new_directory = session.join(new_campaign);
    assert!(!new_directory.exists());
    assert!(!output.exists());
    assert!(
        spooled_campaigns(&session)
            .unwrap()
            .iter()
            .any(|campaign| campaign == new_campaign)
    );

    let paths = finalized(&session, &output, 3);
    assert!(segment_paths(&new_directory).unwrap().is_empty());
    for (campaign, frames, gaps, failed_hour) in [
        (
            *ALLOCATION_CAMPAIGN,
            vec![(9, 2), (9, 3), (9, 4)],
            vec![(9, 5, 7)],
            Some(7),
        ),
        (second, vec![(9, 2)], vec![(9, 3, 4)], None),
        (never_started, vec![], vec![(1, 51, 52), (2, 53, 54)], None),
    ] {
        let path = paths
            .iter()
            .find(|path| open(path).unwrap().campaign_id == campaign)
            .unwrap();
        assert_contents(path, &campaign, &frames, &gaps, failed_hour);
        if campaign == never_started {
            assert_eq!(
                open(path).unwrap().branches,
                [
                    BranchStart {
                        id: 1,
                        session: 7,
                        loaded_hour: None,
                        first_hour: 51,
                        parent: Parentage::Unknown,
                        reason: BranchReason::SessionStart,
                    },
                    BranchStart {
                        id: 2,
                        session: 7,
                        loaded_hour: None,
                        first_hour: 53,
                        parent: Parentage::Unknown,
                        reason: BranchReason::LoadSignal
                    }
                ]
            );
        }
    }
    assert!(segment_paths(&new_directory).unwrap().is_empty());
    assert_repeated(&session, &output, &paths);
    for (path, bytes) in sources {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert_eq!(fs::read(session.join(WITNESS_FILE_NAME)).unwrap(), bundle);
}

#[test]
fn bundled_ranges_append_once_after_published_and_unsaved_checkpoints() {
    for auto_save in [true, false] {
        let (session, output, mut primary) =
            allocation_session(&format!("bundle-checkpoint-{auto_save}"));
        let campaign_name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(campaign_name);
        let context = encode(&allocation_records()[..1]);
        fs::write(campaign.join("spool.0001.h4s"), &context).unwrap();
        let settings = session.join("settings.json");
        let naming = Naming {
            localisation: None,
            settings: &settings,
            report: None,
        };
        let checkpoint = checkpoint_campaign(
            &session,
            &output,
            campaign_name,
            0,
            auto_save,
            &mut Ahead::default(),
            naming,
            &|| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!((checkpoint.hours, checkpoint.deleted), (3, 1));
        let before = fs::read(&checkpoint.recording).unwrap();
        assert!(!campaign.join("spool.0000.h4s").exists());
        primary.committed_segment = Some(1);
        primary.committed_bytes = context.len() as u64;
        primary.branch = Some(10);
        primary.load_signal = Some(20);
        primary.failed_hour = Some(23);
        primary.uncommitted = Some(allocation_range(10, Some(20), 21..=22));
        let prefix = Some((1, primary.committed_bytes));
        write_bundle(
            &session,
            &[
                primary,
                extension(primary, allocation_range(9, Some(19), 5..=7), prefix),
            ],
        );
        let save = || {
            let (path, omitted) = save_campaign(
                &session,
                &output,
                campaign_name,
                !auto_save,
                &|| Ok(()),
                &mut Ahead::default(),
                naming,
            )
            .unwrap();
            assert!(omitted.is_empty());
            path.unwrap()
        };
        let path = save();
        let after = fs::read(&path).unwrap();
        let slot = Replay::open(before.clone()).unwrap().index[0].offset as usize - 32;
        assert!(after.len() > before.len());
        assert_eq!(after[..slot], before[..slot]);
        assert_eq!(after[slot + 32..before.len()], before[slot + 32..]);
        assert_contents(
            &path,
            ALLOCATION_CAMPAIGN,
            &[(9, 2), (9, 3), (9, 4)],
            &[(9, 5, 7), (10, 21, 23)],
            Some(23),
        );
        assert_eq!(
            recording_files::recording_of(&output, ALLOCATION_CAMPAIGN).unwrap(),
            auto_save.then(|| path.clone())
        );
        assert_eq!(save(), path);
        assert_eq!(fs::read(&path).unwrap(), after);
        let paths = finalized(&session, &output, 1);
        assert_eq!(fs::read(&paths[0]).unwrap(), after);
        assert_repeated(&session, &output, &paths);
        assert_eq!(fs::read(campaign.join("spool.0001.h4s")).unwrap(), context);
    }
}

#[test]
fn truncated_bundles_refuse_full_record_and_partial_record_loss() {
    let (session, output, mut primary) = allocation_session("bundle-truncation");
    primary.failed_hour = Some(8);
    primary.uncommitted = Some(allocation_range(9, Some(19), 5..=7));
    let prefix = Some((0, primary.committed_bytes));
    let bytes = write_bundle(
        &session,
        &[
            primary,
            extension(primary, allocation_range(10, Some(20), 21..=22), prefix),
            extension(primary, allocation_range(11, Some(21), 31..=32), prefix),
        ],
    );
    let spool = session
        .join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap())
        .join("spool.0000.h4s");
    let source = fs::read(&spool).unwrap();
    for length in [bytes.len() - WITNESS_BYTES, bytes.len() - 1, WITNESS_BYTES] {
        fs::write(session.join(WITNESS_FILE_NAME), &bytes[..length]).unwrap();
        assert_eq!(
            TerminalWitness::decode(&bytes[..WITNESS_BYTES]).unwrap(),
            primary
        );
        assert!(read_failure_witnesses(&session).is_err());
        let error = finalize_session_outcome(&session, &output).unwrap_err();
        assert_eq!(
            finalize_session_outcome(&session, &output).unwrap_err(),
            error
        );
        assert!(!output.exists() || recording_files::listing(&output).unwrap().is_empty());
        assert_eq!(fs::read(&spool).unwrap(), source);
        assert_eq!(
            fs::read(session.join(WITNESS_FILE_NAME)).unwrap(),
            bytes[..length]
        );
    }
}

#[test]
fn a_checkpointed_branch_merges_its_committed_spool_tail_before_deriving_a_lost_branch() {
    for auto_save in [true, false] {
        for code in [
            replay_data::witness::FailureCode::Reserve,
            replay_data::witness::FailureCode::Thread,
        ] {
            let (session, output, mut primary) =
                allocation_session(&format!("checkpoint-committed-tail-{auto_save}-{code:?}"));
            let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
            let start = BranchStart {
                id: 1,
                session: 7,
                loaded_hour: Some(7),
                first_hour: 8,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            };
            let context = vec![allocation_records().remove(0), Record::BranchStart(start)];
            let mut first = context.clone();
            first.extend((8..=10).map(|hour| Record::Frame(allocation_frame(1, hour, 19))));
            write_spool(&session, ALLOCATION_CAMPAIGN, &first);
            let tail = campaign.join("spool.0001.h4s");
            fs::write(&tail, encode(&context)).unwrap();
            let settings = session.join("settings.json");
            let naming = Naming {
                localisation: None,
                settings: &settings,
                report: None,
            };
            let checkpoint = checkpoint_campaign(
                &session,
                &output,
                std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap(),
                0,
                auto_save,
                &mut Ahead::default(),
                naming,
                &|| Ok(()),
            )
            .unwrap()
            .unwrap();
            assert_eq!((checkpoint.hours, checkpoint.deleted), (3, 1));
            let mut committed = context;
            committed.push(Record::Frame(allocation_frame(1, 11, 19)));
            let bytes = encode(&committed);
            fs::write(&tail, &bytes).unwrap();
            primary.committed_segment = Some(1);
            primary.committed_bytes = bytes.len() as u64;
            primary.branch = Some(2);
            primary.load_signal = Some(20);
            primary.failed_hour = Some(23);
            primary.uncommitted = Some(allocation_range(2, Some(20), 21..=22));
            primary.code = code as u8;
            write_bundle(&session, &[primary]);
            let paths = finalized(&session, &output, 1);
            assert_contents(
                &paths[0],
                ALLOCATION_CAMPAIGN,
                &[(1, 8), (1, 9), (1, 10), (1, 11)],
                &[(2, 21, 23)],
                Some(23),
            );
            assert_repeated(&session, &output, &paths);
            assert_eq!(fs::read(&tail).unwrap(), bytes);
        }
    }
}

#[test]
fn witness_only_campaigns_require_a_surviving_matching_process_session() {
    let (session, output, mut primary) = allocation_session("bundle-unverified-session");
    let never_started = *b"23b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    primary.process_start_time += 1;
    let range = UncommittedRange {
        campaign_id: never_started,
        ..allocation_range(1, Some(31), 51..=52)
    };
    let bundle = write_bundle(&session, &[primary, extension(primary, range, None)]);
    let name = std::str::from_utf8(&never_started).unwrap();
    let outcome = finalize_session_outcome(&session, &output).unwrap();
    assert!(outcome.replays.is_empty());
    assert!(outcome.failed_campaigns.contains(name));
    assert!(outcome.errors.iter().any(|error| {
        error.contains("terminal ranges have no surviving process session identity")
    }));
    assert!(segment_paths(&session.join(name)).unwrap().is_empty());
    assert_eq!(fs::read(session.join(WITNESS_FILE_NAME)).unwrap(), bundle);
}

#[test]
fn terminal_ranges_do_not_bypass_unverifiable_saved_only_bookkeeping() {
    let (session, output, primary) = allocation_session("bundle-unverified-saved-only");
    let never_started = *b"23b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    let name = std::str::from_utf8(&never_started).unwrap();
    let campaign = session.join(name);
    fs::create_dir_all(&campaign).unwrap();
    let saved = b"{}";
    fs::write(campaign.join(SAVED_FILE), saved).unwrap();
    let range = UncommittedRange {
        campaign_id: never_started,
        ..allocation_range(1, Some(31), 51..=52)
    };
    let bundle = write_bundle(&session, &[primary, extension(primary, range, None)]);
    let outcome = finalize_session_outcome(&session, &output).unwrap();
    assert!(outcome.failed_campaigns.contains(name));
    assert!(outcome.errors.iter().any(|error| {
        error.contains(name) && error.contains("saved revision cannot be verified; spool retained")
    }));
    assert!(
        outcome
            .replays
            .iter()
            .all(|path| open(path).unwrap().campaign_id != never_started)
    );
    assert!(segment_paths(&campaign).unwrap().is_empty());
    assert_eq!(fs::read(campaign.join(SAVED_FILE)).unwrap(), saved);
    assert_eq!(fs::read(session.join(WITNESS_FILE_NAME)).unwrap(), bundle);
}
