use super::super::grouped::{
    self,
    reader_tests::{forced_keyframe, many_blocks_fixture, rewrite, rewrite_fixture},
};
use super::*;
use std::sync::atomic::AtomicUsize;

struct Fixture {
    path: PathBuf,
    file: Option<File>,
    replay: Option<Arc<Replay>>,
}

impl Fixture {
    fn new(replay: &Replay) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = Path::new(r"captures\reader-source-fixtures");
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join(format!(
            "source-{}-{}-{}.replay",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(replay.bytes()).unwrap();
        file.flush().unwrap();
        let held = Arc::new(Replay::open_path(&path).unwrap());
        Self {
            path,
            file: Some(file),
            replay: Some(held),
        }
    }
    fn held(&self) -> Arc<Replay> {
        Arc::clone(self.replay.as_ref().unwrap())
    }
    fn write_block(&mut self, replay: &Replay, position: usize) {
        let index = &replay.index[position];
        let file = self.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(index.offset)).unwrap();
        file.write_all(
            &replay.bytes[index.offset as usize..(index.offset + index.length) as usize],
        )
        .unwrap();
        file.flush().unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.replay.take();
        self.file.take();
        std::fs::remove_file(&self.path).unwrap();
    }
}

fn execution(ahead: bool) -> ReaderExecution {
    ReaderExecution {
        workers: 1,
        read_ahead: ahead,
    }
}

fn assert_poisoned(reader: &SequentialReader) {
    assert_eq!(reader.cursor().next_block, 0);
    assert_eq!(reader.census().anchor_states, 0);
    assert_eq!(reader.census().raw_blocks, 0);
    assert!(reader.current().is_none());
}

#[test]
fn transient_rewrite_during_the_consumed_read_cannot_admit_a_stale_anchor() {
    let original = rewrite_fixture(b'a');
    let changed = rewrite_fixture(b'b');
    assert_eq!(original.index, changed.index);
    for local in [false, true] {
        let mut fixture = Fixture::new(&original);
        let held = fixture.held();
        let selection = ReaderSelection::new(&[TableId::LexicalTokens]).unwrap();
        let mut reader = if local {
            SequentialReader::for_block(selection, 0, execution(false)).unwrap()
        } else {
            SequentialReader::new(selection, execution(false)).unwrap()
        };
        let mut stage = 0;
        let ReadStep::Hour { frame, .. } = reader
            .advance(&held, || {
                if super::super::check::step()
                    == Some(super::super::check::Step::Source(
                        grouped::keyframe_section(TableId::LexicalTokens),
                    ))
                {
                    match stage {
                        0 => {
                            fixture.write_block(&changed, 0);
                            stage = 1;
                        }
                        1 => {
                            fixture.write_block(&original, 0);
                            stage = 2;
                        }
                        _ => {}
                    }
                }
                Ok(())
            })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(stage, 2);
        assert_eq!(
            *frame.tables[TableId::LexicalTokens.index()],
            TableView::Complete(TableRows::LexicalTokens(vec![
                crate::rows::LexicalTokenRow {
                    token: 0,
                    literal: vec![b'b'; 64]
                }
            ]))
        );
        let error = reader.advance(&held, || Ok(())).err().unwrap();
        assert_eq!(
            error.to_string(),
            "recording differs from reader source prefix"
        );
        assert_poisoned(&reader);
        assert!(!held.is_verified(&held.index[0]));
    }
}

#[test]
fn rewriting_after_decode_before_the_boundary_invalidates_consumed_bytes() {
    let original = rewrite_fixture(b'a');
    let changed = rewrite_fixture(b'b');
    let mut fixture = Fixture::new(&original);
    let held = fixture.held();
    let mut reader = SequentialReader::new(
        ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
        execution(false),
    )
    .unwrap();
    assert!(matches!(
        reader.advance(&held, || Ok(())).unwrap(),
        ReadStep::Hour { .. }
    ));
    fixture.write_block(&changed, 0);
    let error = reader.advance(&held, || Ok(())).err().unwrap();
    assert_eq!(
        error.to_string(),
        "recording differs from reader source prefix"
    );
    assert_poisoned(&reader);
}

#[test]
fn boundary_and_accepted_history_are_immutable_but_tail_appends_remain_writable() {
    let original = rewrite_fixture(b'a');
    let changed = rewrite_fixture(b'b');
    let mut fixture = Fixture::new(&original);
    let held = fixture.held();
    let mut reader = SequentialReader::new(
        ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
        execution(false),
    )
    .unwrap();
    assert!(matches!(
        reader.advance(&held, || Ok(())).unwrap(),
        ReadStep::Hour { .. }
    ));
    let ReadStep::BlockEnd { receipt } = reader.advance(&held, || Ok(())).unwrap() else {
        panic!()
    };
    let receipt = receipt.clone();
    let index = &changed.index[0];
    for accepted in [false, true] {
        let file = fixture.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(index.offset)).unwrap();
        assert_eq!(
            file.write_all(
                &changed.bytes[index.offset as usize..(index.offset + index.length) as usize]
            )
            .unwrap_err()
            .raw_os_error(),
            Some(33)
        );
        if !accepted {
            reader.accept_block(&receipt).unwrap();
        }
    }
    let file = fixture.file.as_mut().unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(b"uncommitted append").unwrap();
    file.flush().unwrap();
    assert_eq!(reader.cursor().next_block, 1);
}

#[test]
fn a_pending_receipt_is_verified_and_leased_when_attached_to_another_file() {
    let original = rewrite_fixture(b'a');
    let changed = rewrite_fixture(b'b');
    assert_eq!(original.index, changed.index);
    for same in [false, true] {
        let fixture = Fixture::new(&original);
        let mut replacement = Fixture::new(if same { &original } else { &changed });
        let held = fixture.held();
        let next = replacement.held();
        let mut reader = SequentialReader::new(
            ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
            execution(false),
        )
        .unwrap();
        assert!(matches!(
            reader.advance(&held, || Ok(())).unwrap(),
            ReadStep::Hour { .. }
        ));
        let ReadStep::BlockEnd { receipt } = reader.advance(&held, || Ok(())).unwrap() else {
            panic!()
        };
        let receipt = receipt.clone();
        if !same {
            let error = reader.advance(&next, || Ok(())).err().unwrap();
            assert_eq!(
                error.to_string(),
                "recording differs from reader source prefix"
            );
            assert_poisoned(&reader);
            assert!(reader.accept_block(&receipt).is_err());
            continue;
        }
        let ReadStep::BlockEnd { receipt: attached } = reader.advance(&next, || Ok(())).unwrap()
        else {
            panic!()
        };
        assert_eq!(attached, &receipt);
        let bytes = reader.census().source_check_bytes;
        let index = &changed.index[0];
        let file = replacement.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(index.offset)).unwrap();
        assert_eq!(
            file.write_all(
                &changed.bytes[index.offset as usize..(index.offset + index.length) as usize]
            )
            .unwrap_err()
            .raw_os_error(),
            Some(33)
        );
        assert!(matches!(
            reader.advance(&next, || Ok(())).unwrap(),
            ReadStep::BlockEnd { .. }
        ));
        assert_eq!(reader.census().source_check_bytes, bytes);
        reader.accept_block(&receipt).unwrap();
        assert_eq!(reader.cursor().next_block, 1);
    }
}

#[test]
fn ahead_work_keeps_the_hash_of_its_actual_inflation_buffers() {
    let make = |byte| {
        rewrite(rewrite_fixture(b'a'), |position, id, raw| {
            if position == 1 && id == grouped::keyframe_section(TableId::LexicalTokens) {
                let rows = TableRows::LexicalTokens(vec![crate::rows::LexicalTokenRow {
                    token: 0,
                    literal: vec![byte; 64],
                }]);
                *raw = forced_keyframe(&rows, None);
            }
        })
    };
    let original = make(b'a');
    let changed = make(b'b');
    assert_eq!(original.index, changed.index);
    let mut fixture = Fixture::new(&original);
    let held = fixture.held();
    let mut reader = SequentialReader::new(
        ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
        execution(true),
    )
    .unwrap();
    assert!(matches!(
        reader.advance(&held, || Ok(())).unwrap(),
        ReadStep::Hour { .. }
    ));
    drop(
        reader
            .ahead
            .as_mut()
            .unwrap()
            .take(&mut Check::disabled())
            .unwrap(),
    );
    fixture.write_block(&changed, 1);
    reader
        .ahead
        .as_mut()
        .unwrap()
        .request(
            Arc::clone(&held),
            1,
            reader.core.decoded,
            reader.core.credits.acquire(2).unwrap(),
        )
        .unwrap();
    let ahead = reader
        .ahead
        .as_mut()
        .unwrap()
        .take(&mut Check::disabled())
        .unwrap();
    fixture.write_block(&original, 1);
    let ReadStep::BlockEnd { receipt } = reader.advance(&held, || Ok(())).unwrap() else {
        panic!()
    };
    let receipt = receipt.clone();
    reader.accept_block(&receipt).unwrap();
    reader
        .core
        .open(&held, 1, true, Some(ahead), &mut Check::disabled())
        .unwrap();
    reader.core.advance(&held, &mut Check::disabled()).unwrap();
    assert_eq!(
        *reader.current().unwrap().tables[TableId::LexicalTokens.index()],
        TableView::Complete(TableRows::LexicalTokens(vec![
            crate::rows::LexicalTokenRow {
                token: 0,
                literal: vec![b'b'; 64]
            }
        ]))
    );
    let error = reader.advance(&held, || Ok(())).err().unwrap();
    assert_eq!(
        error.to_string(),
        "recording differs from reader source prefix"
    );
    assert_eq!(reader.cursor().next_block, 1);
    assert_eq!(reader.census().raw_blocks, 0);
    assert!(reader.current().is_none());
}

#[test]
fn verification_hashes_each_payload_twice_not_the_history_at_every_boundary() {
    for blocks in [8, 64] {
        let original = many_blocks_fixture(blocks);
        assert_eq!(original.index.len(), blocks as usize);
        let total: u64 = original.index.iter().map(|index| index.length).sum();
        for ahead in [false, true] {
            let fixture = Fixture::new(&original);
            let mut held = fixture.held();
            let mut reader = SequentialReader::new(
                ReaderSelection::new(&[TableId::Clock]).unwrap(),
                execution(ahead),
            )
            .unwrap();
            for position in 0..blocks as usize {
                assert!(matches!(
                    reader.advance(&held, || Ok(())).unwrap(),
                    ReadStep::Hour { .. }
                ));
                let ReadStep::BlockEnd { receipt } = reader.advance(&held, || Ok(())).unwrap()
                else {
                    panic!()
                };
                let receipt = receipt.clone();
                let bytes = reader.census().source_check_bytes;
                for _ in 0..3 {
                    assert!(matches!(
                        reader.advance(&held, || Ok(())).unwrap(),
                        ReadStep::BlockEnd { .. }
                    ));
                }
                if !ahead {
                    assert_eq!(reader.census().source_check_bytes, bytes);
                }
                reader.accept_block(&receipt).unwrap();
                assert_eq!(reader.cursor().next_block, position + 1);
                held = Arc::new(Replay::open_path(&fixture.path).unwrap());
            }
            assert_eq!(reader.census().source_check_bytes, 2 * total);
            for _ in 0..5 {
                assert!(matches!(
                    reader.advance(&held, || Ok(())).unwrap(),
                    ReadStep::Tail
                ));
            }
            assert_eq!(reader.census().source_check_bytes, 2 * total);
            let local_source = fixture.held();
            let mut local = SequentialReader::for_block(
                ReaderSelection::new(&[TableId::Clock]).unwrap(),
                blocks as usize - 1,
                execution(true),
            )
            .unwrap();
            assert!(matches!(
                local.advance(&local_source, || Ok(())).unwrap(),
                ReadStep::Hour { .. }
            ));
            assert!(matches!(
                local.advance(&local_source, || Ok(())).unwrap(),
                ReadStep::BlockEnd { .. }
            ));
            assert_eq!(
                local.census().source_check_bytes,
                2 * original.index.last().unwrap().length
            );
        }
    }
}

#[test]
fn linked_save_append_and_detached_checkpoint_keep_committed_payload_leases() {
    let first = super::super::tests::records(1, &[1, 2]);
    let original = Replay::from_records(super::super::tests::CAMPAIGN.to_vec(), &[first]).unwrap();
    let mut fixture = Fixture::new(&original);
    let held = fixture.held();
    let selection = ReaderSelection::new(&[TableId::Clock]).unwrap();
    let mut reader = SequentialReader::new(selection.clone(), execution(false)).unwrap();
    for _ in 0..2 {
        assert!(matches!(
            reader.advance(&held, || Ok(())).unwrap(),
            ReadStep::Hour { .. }
        ));
    }
    let ReadStep::BlockEnd { receipt } = reader.advance(&held, || Ok(())).unwrap() else {
        panic!()
    };
    let receipt = receipt.clone();
    reader.accept_block(&receipt).unwrap();
    let appended = original
        .append(&[super::super::tests::records(2, &[3, 4])])
        .unwrap();
    assert!(appended.previous.is_some());
    let file = fixture.file.as_mut().unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(&appended.bytes()[original.bytes().len()..])
        .unwrap();
    let slot = appended.slot.unwrap();
    file.seek(SeekFrom::Start(slot as u64)).unwrap();
    file.write_all(&appended.bytes()[slot..slot + super::super::commit::SLOT_SIZE])
        .unwrap();
    file.flush().unwrap();
    let reopened = Arc::new(Replay::open_path(&fixture.path).unwrap());
    assert_eq!(held.revision(), original.revision());
    assert_eq!(reopened.revision(), appended.revision());
    let (checkpoint, frame) = reader.into_checkpoint().unwrap();
    let mut resumed =
        SequentialReader::resume(selection, checkpoint, frame, execution(false)).unwrap();
    for hour in [3, 4] {
        let ReadStep::Hour { frame, .. } = resumed.advance(&reopened, || Ok(())).unwrap() else {
            panic!()
        };
        assert_eq!((frame.branch, frame.hour), (2, hour));
    }
    let ReadStep::BlockEnd { receipt } = resumed.advance(&reopened, || Ok(())).unwrap() else {
        panic!()
    };
    let receipt = receipt.clone();
    resumed.accept_block(&receipt).unwrap();
    assert_eq!(resumed.cursor().next_block, 2);
    assert!(matches!(
        resumed.advance(&reopened, || Ok(())).unwrap(),
        ReadStep::Tail
    ));
}

#[test]
fn multichunk_receipt_digest_reproduces_on_a_detached_revision() {
    let original = grouped::reader_tests::multi_chunk_fixture();
    let expected = source::block_digest(
        &original,
        &original.index[0],
        &Arc::new(std::sync::atomic::AtomicU64::new(0)),
        &mut Check::disabled(),
    )
    .unwrap();
    let fixture = Fixture::new(&original);
    let held = fixture.held();
    let selection = ReaderSelection::new(&[TableId::LexicalTokens]).unwrap();
    let mut reader = SequentialReader::new(selection.clone(), execution(false)).unwrap();
    loop {
        match reader.advance(&held, || Ok(())).unwrap() {
            ReadStep::Hour { .. } => {}
            ReadStep::BlockEnd { receipt } => {
                assert_eq!(receipt.source_digest, expected);
                let receipt = receipt.clone();
                reader.accept_block(&receipt).unwrap();
                break;
            }
            ReadStep::Tail => panic!(),
        }
    }
    assert_eq!(reader.census().source_check_bytes, 2 * held.index[0].length);
    let (checkpoint, frame) = reader.into_checkpoint().unwrap();
    let mut restored =
        SequentialReader::resume(selection, checkpoint, frame, execution(false)).unwrap();
    let reopened = Arc::new(Replay::open_path(&fixture.path).unwrap());
    let ReadStep::Hour { frame, ordinal, .. } = restored.advance(&reopened, || Ok(())).unwrap()
    else {
        panic!()
    };
    assert_eq!(ordinal, 1);
    assert_eq!(frame.hour, reopened.index[1].first_hour);
}
