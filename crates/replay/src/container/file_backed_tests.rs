//! Held-file and preparation behavior through the current recording writer.

use super::*;
use std::sync::atomic::AtomicU64;

struct Fixture(PathBuf);

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "replay-held-{}-{}.battleplan",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn held_grouped_selected_reads_and_seeks_match_resident_readers() {
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    assert!(held.is_file_backed());
    assert!(held.bytes.is_empty());
    assert_eq!(held.index, memory.index);
    assert_eq!(held.ordinals, memory.ordinals);
    let selected = [TableId::Clock, TableId::WorldTension];
    let block = &held.index[0];
    assert_eq!(
        held.read_block(block).unwrap(),
        memory.read_block(block).unwrap()
    );
    assert_eq!(
        held.read_block_selected(block, &selected).unwrap(),
        memory.read_block_selected(block, &selected).unwrap()
    );
    assert_eq!(
        held.block_sections(block).unwrap(),
        memory.block_sections(block).unwrap()
    );
    for hour in 1..=3 {
        let SeekResult::Frame(left) = memory.seek_selected(1, hour, Some(&selected)).unwrap()
        else {
            panic!("resident hour is missing")
        };
        let SeekResult::Frame(right) = held.seek_selected(1, hour, Some(&selected)).unwrap() else {
            panic!("held hour is missing")
        };
        assert_eq!(left, right);
        for table in selected {
            assert_eq!(
                left.tables[table.index()].sampled_at(),
                right.tables[table.index()].sampled_at()
            );
        }
    }
}

#[test]
fn selected_seek_decodes_only_its_prefix_and_preserves_cancellation_errors() {
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let selected = [TableId::Clock];
    for replay in [&memory, &held] {
        let SeekResult::Frame(first) = replay.seek_selected(1, 1, Some(&selected)).unwrap() else {
            panic!("first hour is missing")
        };
        assert_eq!(first.hour, 1);
        assert!(
            !replay.is_verified(&replay.index[0]),
            "early seek leaves later column grammar unverified"
        );
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::OutOfMemory] {
            let mut checks = 0;
            let error = replay
                .seek_selected_with(1, 2, Some(&selected), || {
                    checks += 1;
                    if checks == 2 {
                        Err(io::Error::from(kind))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(checks, 2);
            assert!(!replay.is_verified(&replay.index[0]));
        }
        let SeekResult::Frame(last) = replay.seek_selected(1, 3, Some(&selected)).unwrap() else {
            panic!("last hour is missing")
        };
        assert_eq!(last.hour, 3);
        assert!(replay.is_verified(&replay.index[0]));
    }
}

#[test]
fn selected_through_parallel_reads_keep_selection_order_and_worker_setup() {
    use std::sync::atomic::AtomicUsize;
    let memory = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, &(1..=65).collect::<Vec<_>>())],
    )
    .unwrap();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let selected = [TableId::Clock, TableId::WorldTension];
    let reference = Replay::open(memory.bytes().to_vec()).unwrap();
    let expected = reference
        .read_block_selected(&memory.index[0], &selected)
        .unwrap();
    for replay in [&memory, &held] {
        let setup = AtomicUsize::new(0);
        let on_worker = || {
            setup.fetch_add(1, Ordering::Relaxed);
        };
        let parallel = Parallel {
            workers: 4,
            on_worker: &on_worker,
        };
        let actual = replay
            .read_block_selected_through(&replay.index[0], Some(&selected), 33, Some(&parallel))
            .unwrap();
        assert_eq!(actual, expected[..33]);
        assert!(!replay.is_verified(&replay.index[0]));
        assert!(setup.load(Ordering::Relaxed) > 0);
        assert!(setup.load(Ordering::Relaxed) <= parallel.workers);
        assert!(matches!(
            actual[0].tables[TableId::Campaign.index()],
            crate::frame::TableUpdate::Unavailable { .. }
        ));
    }
}

#[test]
fn selected_through_parallel_whole_read_rejects_corrupt_unselected_section() {
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let mut bytes = memory.bytes().to_vec();
    let block = &memory.index[0];
    bytes[(block.offset + block.length - 1) as usize] ^= 1;
    let fixture = Fixture::new(&bytes);
    let selected = [TableId::Clock];
    let sequential = Replay::open_path(&fixture.0).unwrap();
    let expected = sequential
        .read_block_selected(&sequential.index[0], &selected)
        .unwrap_err();
    let parallel = Replay::open_path(&fixture.0).unwrap();
    let actual = parallel
        .read_block_selected_through(
            &parallel.index[0],
            Some(&selected),
            parallel.index[0].last_hour,
            Some(&Parallel {
                workers: 4,
                on_worker: &|| {},
            }),
        )
        .unwrap_err();
    assert_eq!(actual.kind(), expected.kind());
    assert_eq!(actual.to_string(), expected.to_string());
    assert!(!parallel.is_verified(&parallel.index[0]));
}

#[test]
fn held_current_grouped_anchors_rebuild_the_same_selected_rows() {
    use crate::rows::CountryRow;
    let hours = [1, 2, 721, 722, 1441, 1442];
    let mut encoder = FrameEncoder::new();
    let mut input = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for hour in hours {
        let mut frame = source(1, hour);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=5_000)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("synthetic-country-{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        input.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let memory = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    assert_eq!(memory.index.len(), 3);
    let chain = grouped::keyframe_chain(&memory, 2, TableId::Countries).unwrap();
    assert!(
        chain.len() > 1,
        "the fixture must exercise a real grouped anchor"
    );
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    assert_eq!(
        grouped::keyframe_chain(&held, 2, TableId::Countries).unwrap(),
        chain
    );
    let selected = [TableId::Clock, TableId::Countries];
    let mut requests = 0;
    for index in &held.index {
        let parallel = held
            .read_block_selected_through(
                index,
                Some(&selected),
                index.last_hour,
                Some(&Parallel {
                    workers: 4,
                    on_worker: &|| {},
                }),
            )
            .unwrap();
        requests += 1;
        assert_eq!(
            parallel,
            memory.read_block_selected(index, &selected).unwrap()
        );
    }
    assert_eq!(
        requests, 3,
        "the 5,000-country anchor case reads one range per source block"
    );
    for hour in hours {
        let SeekResult::Frame(left) = memory.seek_selected(1, hour, Some(&selected)).unwrap()
        else {
            panic!("resident hour is missing")
        };
        let SeekResult::Frame(right) = held.seek_selected(1, hour, Some(&selected)).unwrap() else {
            panic!("held hour is missing")
        };
        assert_eq!(left, right);
    }
}

#[test]
fn held_metadata_open_uses_only_header_and_trailer_bytes() {
    struct Observed {
        bytes: io::Cursor<Vec<u8>>,
        reads: Vec<(u64, usize)>,
    }
    impl Read for Observed {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let start = self.bytes.position();
            let count = self.bytes.read(output)?;
            self.reads.push((start, count));
            Ok(count)
        }
    }
    impl Seek for Observed {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.bytes.seek(position)
        }
    }
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let mut observed = Observed {
        bytes: io::Cursor::new(memory.bytes().to_vec()),
        reads: Vec::new(),
    };
    let admitted = Replay::open_file_metadata(&mut observed).unwrap();
    assert_eq!(admitted.index, memory.index);
    assert_eq!(admitted.ordinals, memory.ordinals);
    for (offset, count) in observed.reads {
        assert!(
            offset + count as u64 <= memory.header_end as u64
                || offset >= memory.trailer_start as u64
        );
    }
}

#[test]
fn held_recordings_refuse_append_and_publication_before_any_output() {
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let appended = held.append(&[]).err().unwrap();
    assert_eq!(appended.kind(), io::ErrorKind::Unsupported);
    let copy = Fixture(fixture.0.with_extension("copy"));
    let written = held.write_atomic(&copy.0).unwrap_err();
    assert_eq!(written.kind(), io::ErrorKind::Unsupported);
    assert!(!copy.0.exists() && !temporary_path(&copy.0).exists());
}

#[test]
fn grouped_stream_resumes_at_accepted_blocks_and_rolls_back_cancellation() {
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let appended = memory.append(&[records(2, &[8, 9])]).unwrap();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let selected = [TableId::Clock];
    let mut stream = ReplayStream::new(&selected).unwrap();
    let mut updates = Vec::new();
    let mut oracle = FrameDecoder::for_tables(&[TableId::Campaign, TableId::Clock]);
    stream
        .next_block_with(
            &held,
            |frame, decoded| {
                let expected = oracle.advance(&frame)?;
                assert_eq!(decoded, expected);
                assert_eq!(
                    decoded.tables[TableId::Clock.index()].sampled_at(),
                    expected.tables[TableId::Clock.index()].sampled_at()
                );
                updates.push(frame);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        updates,
        held.read_block_selected(&held.index[0], &[TableId::Campaign, TableId::Clock])
            .unwrap()
    );
    let cursor = stream.cursor().clone();
    let accumulator = stream.accumulator().cloned();
    let fixture2 = Fixture::new(appended.bytes());
    let next = Replay::open_path(&fixture2.0).unwrap();
    let mut resumed = ReplayStream::resume(&selected, cursor.clone(), accumulator).unwrap();
    let error = resumed
        .next_block(&next, |_| Err(io::Error::from(io::ErrorKind::Interrupted)))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(resumed.cursor(), &cursor);
    assert_eq!(resumed.accumulator().unwrap().hour, 2);
    let mut hours = Vec::new();
    resumed
        .next_block(&next, |frame| {
            hours.push((frame.branch, frame.hour));
            Ok(())
        })
        .unwrap();
    assert_eq!(hours, [(2, 8), (2, 9)]);
    assert!(!resumed.next_block(&next, |_| panic!()).unwrap());
}

#[test]
fn malformed_grouped_admission_leaves_stream_at_previous_boundary() {
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let mut bytes = memory.bytes().to_vec();
    let block = &memory.index[0];
    bytes[(block.offset + block.length - 1) as usize] ^= 1;
    let fixture = Fixture::new(&bytes);
    let held = Replay::open_path(&fixture.0).unwrap();
    let mut stream = ReplayStream::new(&[TableId::Clock]).unwrap();
    assert!(
        stream
            .next_block(&held, |_| panic!("unaccepted frame escaped"))
            .is_err()
    );
    assert_eq!(stream.cursor(), &ReplayCursor::default());
    assert!(stream.accumulator().is_none());
}

fn interleaved_recording() -> Replay {
    let current = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let replay = edited_block(&current, |_| {});
    assert!(!grouped::is_grouped(
        &replay.block_bytes(&replay.index[0]).unwrap()
    ));
    replay
}

#[test]
fn positional_reads_finish_while_a_block_visitor_is_parked() {
    use std::sync::mpsc;
    let memory = interleaved_recording();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    let expected = memory.block_bytes(&memory.index[0]).unwrap();
    std::thread::scope(|scope| {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let replay = &held;
        let visiting = scope.spawn(move || {
            replay.visit_file_block(index, None, &mut |_| {
                assert!(replay.is_verified(index));
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(false)
            })
        });
        entered_rx.recv().unwrap();
        let mut magic = [0; 4];
        held.read_at(index.offset, &mut magic).unwrap();
        assert_eq!(magic, expected[..4]);
        assert_eq!(held.block_bytes(index).unwrap(), expected);
        assert_eq!(
            held.block_range(index, 1, 3).unwrap().as_ref(),
            &expected[1..4]
        );
        assert!(!visiting.is_finished());
        release_tx.send(()).unwrap();
        visiting.join().unwrap().unwrap();
    });
}

#[test]
fn held_visits_validate_once_before_the_first_callback() {
    let memory = interleaved_recording();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    assert!(!held.is_verified(index));
    for expected in [vec![true, false], vec![false]] {
        let mut visited = 0;
        let (result, passes) = file_backed::measure_held_passes(|| {
            held.visit_file_block(index, None, &mut |frame| {
                assert!(held.is_verified(index));
                assert_eq!(frame.hour, 1);
                visited += 1;
                Ok(false)
            })
        });
        result.unwrap();
        assert_eq!(visited, 1);
        assert_eq!(passes, expected);
    }
}

#[test]
fn held_admission_refuses_a_later_bad_payload_before_any_callback() {
    let memory = interleaved_recording();
    let malformed = edited_block(&memory, |raw| {
        raw.pop().unwrap();
    });
    let fixture = Fixture::new(malformed.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    let (result, passes) = file_backed::measure_held_passes(|| {
        held.visit_file_block(index, None, &mut |_| panic!("unadmitted frame escaped"))
    });
    assert_eq!(result.unwrap_err().to_string(), "truncated field");
    assert_eq!(passes, [true]);
    assert!(!held.is_verified(index));
}

#[test]
fn held_admission_refuses_buffered_and_unread_concatenated_frames() {
    let memory = interleaved_recording();
    let mut random = 1_u64;
    let noise: Vec<u8> = (0..96 * 1024)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random as u8
        })
        .collect();
    let small = zstd::encode_all(&[][..], 3).unwrap();
    let large = zstd::encode_all(&noise[..], 3).unwrap();
    assert!(memory.index[0].length + (small.len() as u64) < 64 * 1024);
    assert!(large.len() > 64 * 1024);
    for trailing in [small, large] {
        let mut prefix = memory.bytes()[..memory.trailer_start].to_vec();
        prefix.extend_from_slice(&trailing);
        let mut index = memory.index.clone();
        index[0].length += trailing.len() as u64;
        let bytes = rebuild(&memory, prefix, &index);
        let fixture = Fixture::new(&bytes);
        let held = Replay::open_path(&fixture.0).unwrap();
        let error = held
            .visit_file_block(&held.index[0], None, &mut |_| {
                panic!("trailing frame escaped")
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "block contains trailing or concatenated zstd frames"
        );
        assert!(!held.is_verified(&held.index[0]));
    }
}

#[test]
fn short_held_reads_keep_read_exact_eof_errors() {
    let memory = interleaved_recording();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    OpenOptions::new()
        .write(true)
        .open(&fixture.0)
        .unwrap()
        .set_len(index.offset + 2)
        .unwrap();
    let mut reference = File::open(&fixture.0).unwrap();
    reference.seek(SeekFrom::Start(index.offset)).unwrap();
    let expected = reference.read_exact(&mut [0; 4]).unwrap_err();
    let mut output = [0; 4];
    let errors = [
        held.read_at(index.offset, &mut output).unwrap_err(),
        held.block_bytes(index).unwrap_err(),
        held.visit_file_block(index, None, &mut |_| panic!("short block escaped"))
            .unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(error.kind(), expected.kind());
        assert_eq!(error.to_string(), expected.to_string());
    }
    assert_eq!(
        &output[..2],
        &memory.bytes()[index.offset as usize..index.offset as usize + 2]
    );
    held.read_at(index.offset + 100, &mut []).unwrap();
    assert!(!held.is_verified(index));
}
