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
fn held_containment_streams_the_same_proof_as_resident_containment() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let appended = first.append(&[records(2, &[8, 9])]).unwrap();
    let earlier = Fixture::new(first.bytes());
    let later = Fixture::new(appended.bytes());
    let held = Replay::open_path(&later.0).unwrap();
    assert!(held.bytes.is_empty());
    let (holds, read) = file_backed::measure_held_reads(|| held.holds(&earlier.0).unwrap());
    assert!(holds);
    assert_eq!(read, 2 * first.saved_length());
    assert_eq!(holds, appended.holds(&earlier.0).unwrap());
    assert!(
        !Replay::open_path(&earlier.0)
            .unwrap()
            .holds(&later.0)
            .unwrap()
    );
    let mut changed = first.bytes().to_vec();
    changed[first.index[0].offset as usize + 5] ^= 1;
    std::fs::write(&earlier.0, changed).unwrap();
    assert!(!held.holds(&earlier.0).unwrap());
    assert!(!appended.holds(&earlier.0).unwrap());
    assert!(held.bytes.is_empty());
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
fn held_readers_refuse_historical_block_magics_without_visiting() {
    let memory = current_recording();
    let offset = memory.index[0].offset as usize;
    for magic in [*b"H4GC", *b"H4GB", [0x28, 0xb5, 0x2f, 0xfd]] {
        let mut bytes = memory.bytes().to_vec();
        bytes[offset..offset + 4].copy_from_slice(&magic);
        let fixture = Fixture::new(&bytes);
        let held = Replay::open_path(&fixture.0).unwrap();
        let index = &held.index[0];
        assert!(held.read_block(index).is_err());
        assert!(held.read_block_selected(index, &[TableId::Clock]).is_err());
        assert!(
            held.visit_block(index, None, |_| panic!("legacy frame escaped"))
                .is_err()
        );
        assert!(held.block_sections(index).is_err());
        assert!(
            held.read_block_kept(index, &[TableId::Clock], || Ok(()))
                .is_err()
        );
        let mut watches = [BlockWatch::new(Watching::Every(&[TableId::Clock]), |_| {
            panic!("legacy watch escaped")
        })];
        assert!(
            held.read_block_watched(index, &mut watches, || Ok(()))
                .is_err()
        );
        assert!(matches!(held.seek(1, 1).unwrap(), SeekResult::Gap { .. }));
        assert!(!held.is_verified(index));
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

fn current_recording() -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap()
}

#[test]
fn positional_reads_finish_while_a_block_visitor_is_parked() {
    use std::sync::mpsc;
    let memory = current_recording();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    let expected = memory.block_bytes(&memory.index[0]).unwrap();
    held.read_block(index).unwrap();
    std::thread::scope(|scope| {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let replay = &held;
        let visiting = scope.spawn(move || {
            replay.visit_block(index, None, &mut |_| {
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
fn held_visits_validate_before_callbacks_and_keep_admission() {
    let memory = current_recording();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    assert!(!held.is_verified(index));
    for _ in 0..2 {
        let mut visited = 0;
        held.visit_block(index, None, |frame| {
            assert_eq!(frame.hour, 1);
            visited += 1;
            Ok(false)
        })
        .unwrap();
        assert_eq!(visited, 1);
        assert!(held.is_verified(index));
    }
}

#[test]
fn held_admission_refuses_a_later_bad_payload_before_any_callback() {
    let memory = current_recording();
    let fixture = Fixture::new(&damage_later_entries(&memory, 0));
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    let error = held
        .visit_block(index, None, |_| panic!("unadmitted frame escaped"))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!held.is_verified(index));
}

#[test]
fn held_admission_refuses_small_and_large_concatenated_frames() {
    let memory = current_recording();
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
            .visit_block(&held.index[0], None, &mut |_| {
                panic!("trailing frame escaped")
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "grouped block has trailing or concatenated bytes"
        );
        assert!(!held.is_verified(&held.index[0]));
    }
}

#[test]
fn short_held_reads_keep_read_exact_eof_errors() {
    let memory = current_recording();
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
        held.visit_block(index, None, &mut |_| panic!("short block escaped"))
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

#[test]
fn held_byte_visit_keeps_its_revision_across_rename_and_atomic_replacement() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let second = first.append(&[records(2, &[8, 9])]).unwrap();
    let fixture = Fixture::new(first.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let moved = fixture.0.with_extension("moved");
    std::fs::rename(&fixture.0, &moved).unwrap();
    std::fs::write(&fixture.0, second.bytes()).unwrap();
    let mut visited = Vec::new();
    let (_, bytes) = file_backed::measure_held_reads(|| {
        held.visit_held_bytes(|chunk| {
            visited.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap()
    });
    assert_eq!(visited, first.bytes());
    assert_eq!(bytes, first.bytes().len() as u64);
    let current = Replay::open_path(&fixture.0).unwrap();
    let candidate = fixture.0.with_extension("pending");
    std::fs::write(&candidate, first.bytes()).unwrap();
    crate::publication::replace_file(&candidate, &fixture.0).unwrap();
    let mut visited = Vec::new();
    current
        .visit_held_bytes(|chunk| {
            visited.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
    assert_eq!(visited, second.bytes());
    std::fs::remove_file(moved).unwrap();
}

#[test]
fn held_byte_visit_has_a_captured_length_and_propagates_cancellation() {
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&fixture.0)
        .unwrap()
        .write_all(b"later in-place bytes")
        .unwrap();
    let mut visited = Vec::new();
    held.visit_held_bytes(|chunk| {
        visited.extend_from_slice(chunk);
        Ok(())
    })
    .unwrap();
    assert_eq!(visited, memory.bytes(), "never chase a changing EOF");
    let error = held
        .visit_held_bytes(|_| Err(io::Error::from(io::ErrorKind::Interrupted)))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn append_identity_bytes_ignore_only_the_commit_slot_and_keep_the_held_revision() {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let fixture = Fixture::new(first.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let appended = first.append(&[records(2, &[8, 9])]).unwrap();
    std::fs::write(&fixture.0, appended.bytes()).unwrap();
    let witness = |replay: &Replay| {
        let mut bytes = Vec::new();
        replay
            .visit_append_identity_bytes(|chunk| {
                bytes.extend_from_slice(chunk);
                Ok(())
            })
            .unwrap();
        let mut expected = replay.bytes().to_vec();
        if !replay.is_file_backed() {
            let slot = replay.slot.unwrap();
            expected[slot..slot + super::commit::SLOT_SIZE].fill(0);
            assert_eq!(bytes, expected);
        }
        bytes
    };
    let original = witness(&first);
    assert_eq!(witness(&held), original);
    let next = witness(&appended);
    let prefix = first.index.last().unwrap();
    let prefix = (prefix.offset + prefix.length) as usize;
    assert_eq!(original[..prefix], next[..prefix]);
    assert_ne!(original, next);
    assert_eq!(
        held.visit_append_identity_bytes(|_| Err(io::ErrorKind::Interrupted.into()))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn held_chunk_api_reads_only_the_requested_frame_and_matches_resident_slices() {
    let hours: Vec<_> = (1..=400).collect();
    let memory = Replay::from_records(CAMPAIGN.into(), &[records(1, &hours)]).unwrap();
    let fixture = Fixture::new(memory.bytes());
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &held.index[0];
    let section = Section::Later(TableId::Clock);
    let held_chunks = held.section_chunks(index, section).unwrap();
    let memory_chunks = memory.section_chunks(&memory.index[0], section).unwrap();
    assert_eq!(held_chunks.len(), 3);
    let mut resident = Vec::new();
    let mut buffer = Vec::new();
    for (held, memory) in held_chunks.zip(memory_chunks) {
        assert_eq!(
            (held.first_hour, held.last_hour),
            (memory.first_hour, memory.last_hour)
        );
        assert_eq!(
            (held.stored_length, held.raw_length),
            (memory.stored_length, memory.raw_length)
        );
        let (_, bytes) =
            file_backed::measure_held_reads(|| held.inflate_into(&mut buffer).unwrap());
        assert_eq!(bytes, held.stored_length);
        memory.inflate_into(&mut resident).unwrap();
        assert_eq!(buffer, resident);
    }
}
